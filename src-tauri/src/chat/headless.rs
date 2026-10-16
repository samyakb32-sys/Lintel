// The headless completion the voice assistant asks for (docs/VOICE.md §4.3):
// one request, its answer streamed back as events. It stands beside the chat's
// own `send` and shares none of its state: no chat window, no word in the
// chat's history, nothing of the conversation kept here (the caller sends it
// whole each time, and its time limits and rounds are its own: it cancels).
//
// Local first is the owner's rule, and this module holds its half of it: a
// request goes only to the provider the caller named, a missing `provider` is
// refused (the chat's own default, Claude, is no default here), and a key is
// read only for the provider that was named.
//
//   feature_call("chat", "ask", {
//     id?          the caller's own id, so it can listen before it asks
//     provider     "builtin" + `url`: the local text model's server, this computer only
//                  "local": the model server from chat that runs on this computer
//                  "ollama" | "lmstudio" | "custom": that one, wherever the user pointed it
//                  "anthropic" · "openai" | "google" | "openrouter" | "openrouter-free": the user's key
//     model?, system?, max_tokens?
//     messages     `{role, content}` turns, or the `messages` an earlier ask gave back
//     tools?       [{name, description, parameters}], `parameters` a JSON schema
//     results?     [{id, content, error?}]: what the calls of that earlier ask answered
//     web_search?  Claude only
//   }) → {id}
//   chat:ask-delta {id, text}    the new words
//   chat:ask-done  {id, stop, text, tools, messages}
//                  stop: "end_turn" | "max_tokens" | "tool_use" (run `tools: [{id, name, input}]`,
//                  then ask again with these `messages` and the `results`);
//                  or {id, stop: "error", error}, or {id, stop: "cancel"}
//   feature_call("chat", "cancel", {id}) → {cancelled}
//   feature_call("chat", "reach") → {ok}: the server "local" means answers

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reqwest::Url;
use serde_json::{json, Value};

use crate::i18n::{t, tf};
use crate::settings::Settings;
use crate::{claude, local_chat, net, openai_compat};

/// Where the events go: `(event, payload)`, `chat:<event>` in the app.
pub type Emit = Arc<dyn Fn(&str, Value) + Send + Sync>;

/// Claude answers aloud with its smallest model unless the caller names one.
const CLAUDE_MODEL: &str = "claude-haiku-5-5";
const MAX_TOKENS: u64 = 1024;
/// A turn the server paused (its own tools ran long) is taken up this often.
const MAX_CONTINUES: usize = 5;
/// The whole of one request. A local model may still be reading its prompt;
/// the caller's own limits (VOICE §3.4) cancel long before these.
const LOCAL_TIMEOUT: Duration = Duration::from_secs(300);
const ONLINE_TIMEOUT: Duration = Duration::from_secs(90);
/// VOICE §4.3: the user's own server counts when it answers this fast.
const REACH_TIMEOUT: Duration = Duration::from_millis(300);
const UNREADABLE: &str = "The model's tool call could not be read.";

// ── Who answers ───────────────────────────────────────────────────────────────

#[derive(Debug, Default, Clone, Copy, PartialEq)]
enum Wire {
    /// `/chat/completions`: llama.cpp, Ollama, LM Studio, OpenAI, Google AI, OpenRouter.
    #[default]
    OpenAi,
    /// Anthropic's Messages API.
    Anthropic,
}

struct Target {
    wire: Wire,
    /// The chat completions or Messages endpoint.
    url: Url,
    key: Option<String>,
    model: String,
    /// OpenAI's own API has another name for `max_tokens` (openai_compat.rs).
    max_tokens_field: &'static str,
    /// What this server wants in the request besides.
    extra: Value,
    /// OpenRouter's free models: asked only while the model still costs nothing.
    free: Option<&'static openai_compat::Provider>,
}

fn target(wire: Wire, endpoint: &str, key: Option<String>, model: String) -> Result<Target, String> {
    let url = Url::parse(endpoint).map_err(|e| e.to_string())?;
    Ok(Target { wire, url, key, model, max_tokens_field: "max_tokens", extra: json!({}), free: None })
}

/// The server "local" means: the one picked in the chat when that is a model
/// server, else the first that is connected. Only one on this computer, with a
/// model chosen: an address elsewhere is used when it is named, never implied.
fn local_server(settings: &Settings) -> Option<(local_chat::Server, Url, String)> {
    [settings.chat_provider.as_str(), "ollama", "lmstudio", "custom"].into_iter().find_map(|id| {
        let server = local_chat::server(settings, id)?;
        let base = net::normalise_server_url(&server.url).ok().filter(net::is_loopback_url)?;
        let model = super::model_for(settings, id);
        (!model.is_empty()).then_some((server, base, model))
    })
}

/// Where a request goes, from the provider the caller named. `key_of` is the
/// credential store: asked for the named provider's key and for nothing else.
fn resolve(args: &Value, settings: &Settings, key_of: &dyn Fn(&str) -> Option<String>) -> Result<Target, String> {
    let asked = args["model"].as_str().map(str::trim).filter(|m| !m.is_empty()).map(str::to_string);
    let completions = |base: &Url| net::join(base, "v1/chat/completions");
    let target = match args["provider"].as_str().unwrap_or("") {
        "" => return Err("chat.ask: no provider was named".into()),
        "builtin" => {
            let base = net::normalise_server_url(args["url"].as_str().unwrap_or(""))?;
            if !net::is_loopback_url(&base) {
                return Err("chat.ask: the built-in model runs on this computer only".into());
            }
            // llama.cpp: keep the prompt it has already read, and no hidden
            // chain of thought.
            let extra = json!({ "cache_prompt": true, "chat_template_kwargs": { "enable_thinking": false } });
            return Ok(Target { extra, ..target(Wire::OpenAi, &completions(&base), None, asked.unwrap_or_default())? });
        }
        "local" => {
            let (server, base, model) =
                local_server(settings).ok_or("chat.ask: no model server from chat runs on this computer")?;
            target(Wire::OpenAi, &completions(&base), server.key, asked.unwrap_or(model))?
        }
        super::ANTHROPIC => {
            let key = key_of(claude::KEY).ok_or_else(|| t("API key missing. Open settings."))?;
            let model = asked.unwrap_or_else(|| CLAUDE_MODEL.into());
            target(Wire::Anthropic, claude::endpoint()?.as_str(), Some(key), model)?
        }
        id => {
            let model = asked.unwrap_or_else(|| super::model_for(settings, id));
            if let Some(p) = openai_compat::provider(id) {
                let key = key_of(p.key).ok_or_else(|| tf("{name} API key missing. Add it in Settings.", &[("name", p.name)]))?;
                let mut target = target(Wire::OpenAi, &format!("{}/chat/completions", p.base_url), Some(key), model)?;
                target.max_tokens_field = p.max_tokens_field;
                if p.free_only {
                    // As the chat does: zero price ceilings, held by OpenRouter
                    // when it routes the request.
                    target.extra = json!({ "provider": { "max_price": { "prompt": 0, "completion": 0, "request": 0 } } });
                    target.free = Some(p);
                }
                target
            } else if let Some(server) = local_chat::server(settings, id) {
                let base = net::normalise_server_url(&server.url)?;
                target(Wire::OpenAi, &completions(&base), server.key, model)?
            } else {
                return Err(format!("Unknown chat provider: {id}"));
            }
        }
    };
    if target.model.is_empty() {
        return Err(t("Pick a model above the chat box first."));
    }
    Ok(target)
}

// ── The asks on their way ─────────────────────────────────────────────────────

struct Running {
    task: tauri::async_runtime::JoinHandle<()>,
    emit: Emit,
}

/// Every event goes out with this held, so nothing follows an ask's `ask-done`.
static RUNNING: Mutex<BTreeMap<String, Running>> = Mutex::new(BTreeMap::new());
static NEXT: AtomicU64 = AtomicU64::new(1);

/// Who may be asked at all, whatever the Chat switch says. The built-in model, on this computer
/// (`resolve` holds it there): always, so the voice assistant answers with Chat off. Anyone else: only
/// with voice's own switch for it on (`voice`: its settings, null while it is off). A key saved for the
/// chat never implies it.
pub fn permit(args: &Value, voice: &Value) -> Result<(), String> {
    let switch = match args["provider"].as_str().unwrap_or("") {
        "builtin" => return Ok(()),
        "local" | "ollama" | "lmstudio" | "custom" => "localServer",
        super::ANTHROPIC => "claude",
        _ => "freeModel",
    };
    match voice[switch] == true {
        true => Ok(()),
        false => Err(format!("chat.ask: the voice assistant's switch for this provider is off ({switch})")),
    }
}

/// `ask`: starts the request and answers with its id at once.
pub fn ask(args: Value, settings: &Settings, emit: Emit) -> Result<Value, String> {
    let target = resolve(&args, settings, &crate::secrets::get)?;
    begin(target, args, emit)
}

fn begin(target: Target, args: Value, emit: Emit) -> Result<Value, String> {
    let id = match args["id"].as_str().filter(|id| !id.is_empty()) {
        Some(id) => id.to_string(),
        None => format!("ask-{}", NEXT.fetch_add(1, Ordering::Relaxed)),
    };
    // Held until the task is on the list: it cannot end before it is there.
    let mut running = RUNNING.lock().unwrap();
    if running.contains_key(&id) {
        return Err(format!("chat.ask: {id} is still running"));
    }
    let task = tauri::async_runtime::spawn({
        let id = id.clone();
        async move {
            let done = run(&id, &target, &args).await.unwrap_or_else(|error| json!({ "stop": "error", "error": error }));
            finish(&id, done);
        }
    });
    running.insert(id.clone(), Running { task, emit });
    Ok(json!({ "id": id }))
}

/// New words to the caller, unless the ask was cancelled meanwhile.
fn say(id: &str, text: &str) {
    if let Some(run) = RUNNING.lock().unwrap().get(id) {
        (run.emit)("ask-delta", json!({ "id": id, "text": text }));
    }
}

/// Whoever takes the ask off the list says how it ended, once.
fn finish(id: &str, mut done: Value) -> bool {
    let mut running = RUNNING.lock().unwrap();
    let Some(run) = running.remove(id) else { return false };
    // Drops the request where it is. (The task itself calls this at its end.)
    run.task.abort();
    done["id"] = json!(id);
    (run.emit)("ask-done", done);
    true
}

/// `cancel`: drops the request where it is. False when the ask had ended.
pub fn cancel(id: &str) -> bool {
    finish(id, json!({ "stop": "cancel" }))
}

/// The chat was switched off: every answer on its way ends here.
pub fn stop_all() {
    let ids: Vec<String> = RUNNING.lock().unwrap().keys().cloned().collect();
    for id in ids {
        cancel(&id);
    }
}

/// `reach`: does the server "local" means answer, in the time a turn can wait?
pub fn reach(settings: &Settings) -> bool {
    let Some((server, base, _)) = local_server(settings) else { return false };
    tauri::async_runtime::block_on(answers(&base, server.key.as_deref()))
}

async fn answers(base: &Url, key: Option<&str>) -> bool {
    let Ok(client) = local_chat::http(base, key, REACH_TIMEOUT) else { return false };
    let request = client.get(net::join(base, "v1/models")).header("Authorization", local_chat::bearer(key));
    request.send().await.is_ok_and(|reply| reply.status().is_success())
}

// ── One request ───────────────────────────────────────────────────────────────

/// The answers to the model's tool calls, as each format wants them told.
fn with_results(wire: Wire, messages: &mut Vec<Value>, results: &[Value]) {
    let said = |r: &Value| r["content"].as_str().map_or_else(|| r["content"].to_string(), str::to_string);
    match wire {
        Wire::OpenAi => {
            messages.extend(results.iter().map(|r| json!({ "role": "tool", "tool_call_id": r["id"], "content": said(r) })));
        }
        // All of them in one message: Claude reads split answers as "one call at a time".
        Wire::Anthropic if !results.is_empty() => {
            let blocks: Vec<Value> = results
                .iter()
                .map(|r| json!({ "type": "tool_result", "tool_use_id": r["id"], "content": said(r), "is_error": r["error"] == true }))
                .collect();
            messages.push(json!({ "role": "user", "content": blocks }));
        }
        Wire::Anthropic => {}
    }
}

fn tools_for(wire: Wire, args: &Value) -> Vec<Value> {
    let mut tools: Vec<Value> = args["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|tool| {
            let (name, about) = (&tool["name"], tool["description"].as_str().unwrap_or(""));
            let schema = if tool["parameters"].is_object() { tool["parameters"].clone() } else { json!({ "type": "object", "properties": {} }) };
            match wire {
                Wire::OpenAi => json!({ "type": "function", "function": { "name": name, "description": about, "parameters": schema } }),
                // Not `eager_input_streaming`: a voice action's input is a few
                // words, and left as it is the API checks it against the schema.
                Wire::Anthropic => json!({ "name": name, "description": about, "input_schema": schema }),
            }
        })
        .collect();
    if wire == Wire::Anthropic && args["web_search"] == true {
        tools.push(json!({ "type": "web_search_20260209", "name": "web_search", "max_uses": 3 }));
    }
    tools
}

fn done(stop: &str, text: String, tools: Vec<Value>, messages: Vec<Value>) -> Value {
    json!({ "stop": stop, "text": text, "tools": tools, "messages": messages })
}

async fn run(id: &str, target: &Target, args: &Value) -> Result<Value, String> {
    let (wire, key) = (target.wire, target.key.as_deref());
    if let Some(p) = target.free {
        openai_compat::still_free(p, key.unwrap_or_default(), &target.model).await?;
    }
    let mut messages = args["messages"].as_array().cloned().unwrap_or_default();
    with_results(wire, &mut messages, args["results"].as_array().map(Vec::as_slice).unwrap_or_default());
    let system = args["system"].as_str().unwrap_or("");
    let max_tokens = args["max_tokens"].as_u64().unwrap_or(MAX_TOKENS);
    let tools = tools_for(wire, args);
    let timeout = if net::is_loopback_url(&target.url) { LOCAL_TIMEOUT } else { ONLINE_TIMEOUT };
    // With a key: https or this computer only, and no redirect followed (net.rs).
    let client = local_chat::http(&target.url, key, timeout)?;
    let mut answer = Answer { wire, ..Default::default() };

    if wire == Wire::OpenAi {
        let mut body = target.extra.clone();
        body["stream"] = json!(true);
        body[target.max_tokens_field] = json!(max_tokens);
        if !target.model.is_empty() {
            body["model"] = json!(target.model);
        }
        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }
        let lead = (!system.is_empty()).then(|| json!({ "role": "system", "content": system }));
        body["messages"] = lead.iter().chain(&messages).cloned().collect();
        let request = client.post(target.url.clone()).header("Authorization", local_chat::bearer(key));
        read(id, request.json(&body), &mut answer).await?;
        return answer.open_ai_done(id, messages);
    }

    let mut body = json!({
        "model": target.model,
        "max_tokens": max_tokens,
        "stream": true,
        // Thinking counts toward `max_tokens`: little of it, for a spoken answer.
        // debt: ponytail: sent to every model; one too old for `effort` gets the API's own error (retry without it then).
        "output_config": { "effort": "low" },
        // The prompt up to here is kept for the next round of the same turn.
        "cache_control": { "type": "ephemeral" },
    });
    // debt: ponytail: no `fallbacks` (the default model has none): a refusal is an error and the caller's next brain answers.
    if !system.is_empty() {
        body["system"] = json!(system);
    }
    if !tools.is_empty() {
        body["tools"] = json!(tools);
    }
    // What Claude wrote before the server paused its turn: sent back as it
    // came (thinking and its signature included), the turn goes on from there.
    let mut written: Vec<Value> = Vec::new();
    for _ in 0..=MAX_CONTINUES {
        let mut sent = messages.clone();
        if !written.is_empty() {
            sent.push(json!({ "role": "assistant", "content": written }));
        }
        body["messages"] = json!(sent);
        let request = client
            .post(target.url.clone())
            .header("x-api-key", key.unwrap_or_default())
            .header("anthropic-version", claude::ANTHROPIC_VERSION);
        read(id, request.json(&body), &mut answer).await?;
        written.append(&mut answer.blocks);
        answer.inputs.clear();
        let stop = match answer.stop.take().as_deref() {
            Some("pause_turn") => continue,
            Some("refusal") => return Err(answer.refusal.take().unwrap_or_else(|| t("Claude declined this one."))),
            Some("tool_use") => "tool_use",
            Some("max_tokens") => "max_tokens",
            Some(_) => "end_turn",
            None => return Err("The answer stopped before its end.".into()),
        };
        // A call that `max_tokens` cut short is never handed over to be run.
        let calls = written.iter().filter(|block| stop == "tool_use" && block["type"] == "tool_use");
        let tools = calls.map(|b| json!({ "id": b["id"], "name": b["name"], "input": b["input"] })).collect();
        messages.push(json!({ "role": "assistant", "content": written }));
        return Ok(done(stop, answer.tell(id, true), tools, messages));
    }
    Err("The answer was paused too many times.".into())
}

/// Sends the request and reads the answer's event stream into `answer`; the
/// caller hears the new words after every piece that arrives.
async fn read(id: &str, request: reqwest::RequestBuilder, answer: &mut Answer) -> Result<(), String> {
    let network = |e: reqwest::Error| tf("Network error: {error}", &[("error", &e.without_url().to_string())]);
    let mut reply = request.send().await.map_err(network)?;
    let status = reply.status();
    if !status.is_success() {
        let body = net::read_capped(reply, net::MAX_ERROR_BODY).await.unwrap_or_default();
        return Err(format!("HTTP {}: {}", status.as_u16(), super::brief(&net::error_detail(&body))));
    }
    let (mut pending, mut received) = (Vec::<u8>::new(), 0usize);
    while let Some(piece) = reply.chunk().await.map_err(network)? {
        received += piece.len();
        if received > net::MAX_BODY {
            return Err(t("The server's answer is too large."));
        }
        pending.extend_from_slice(&piece);
        // Whole lines only: a piece may end in the middle of one, or of a character.
        while let Some(end) = pending.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = pending.drain(..=end).collect();
            answer.line(&String::from_utf8_lossy(&line))?;
        }
        answer.tell(id, false);
    }
    // A last line without its newline.
    answer.line(&String::from_utf8_lossy(&pending))
}

// ── The answer as it arrives ──────────────────────────────────────────────────

#[derive(Default, Clone)]
struct Call {
    id: String,
    name: String,
    input: String,
}

#[derive(Default)]
struct Answer {
    wire: Wire,
    /// Every word so far, and how much of it the caller has heard.
    text: String,
    told: usize,
    stop: Option<String>,
    refusal: Option<String>,
    /// Claude: the content blocks, and each tool call's input as it is written.
    blocks: Vec<Value>,
    inputs: Vec<String>,
    /// OpenAI: the tool calls, by their index.
    calls: Vec<Call>,
}

fn append(block: &mut Value, field: &str, piece: &str) {
    match &mut block[field] {
        Value::String(whole) => whole.push_str(piece),
        other => *other = json!(piece),
    }
}

impl Answer {
    fn line(&mut self, line: &str) -> Result<(), String> {
        let Some(data) = line.trim_end().strip_prefix("data:") else { return Ok(()) };
        // "[DONE]", a keep-alive: nothing to take.
        let Ok(event) = serde_json::from_str::<Value>(data.trim_start()) else { return Ok(()) };
        match self.wire {
            Wire::OpenAi => self.open_ai(&event),
            Wire::Anthropic => self.anthropic(&event),
        }
    }

    fn open_ai(&mut self, event: &Value) -> Result<(), String> {
        if let Some(message) = event.pointer("/error/message").and_then(Value::as_str) {
            return Err(super::brief(message));
        }
        let choice = &event["choices"][0];
        self.text.push_str(choice["delta"]["content"].as_str().unwrap_or(""));
        for part in choice["delta"]["tool_calls"].as_array().into_iter().flatten() {
            // A model calls a handful of tools; an index past that is not one.
            let at = part["index"].as_u64().unwrap_or(0) as usize;
            if at >= 32 {
                continue;
            }
            if self.calls.len() <= at {
                self.calls.resize(at + 1, Call::default());
            }
            let call = &mut self.calls[at];
            // The id and the name come whole, the arguments in pieces.
            for (field, value) in [(&mut call.id, &part["id"]), (&mut call.name, &part["function"]["name"])] {
                if field.is_empty() {
                    field.push_str(value.as_str().unwrap_or(""));
                }
            }
            call.input.push_str(part["function"]["arguments"].as_str().unwrap_or(""));
        }
        if let Some(stop) = choice["finish_reason"].as_str() {
            self.stop = Some(stop.into());
        }
        Ok(())
    }

    fn anthropic(&mut self, event: &Value) -> Result<(), String> {
        let at = event["index"].as_u64().unwrap_or(0) as usize;
        match event["type"].as_str().unwrap_or("") {
            "content_block_start" => {
                let block = &event["content_block"];
                self.blocks.push(if block.is_object() { block.clone() } else { json!({}) });
                self.inputs.push(String::new());
            }
            "content_block_delta" => {
                let (Some(block), Some(input)) = (self.blocks.get_mut(at), self.inputs.get_mut(at)) else { return Ok(()) };
                let delta = &event["delta"];
                match delta["type"].as_str().unwrap_or("") {
                    "text_delta" => {
                        let piece = delta["text"].as_str().unwrap_or("");
                        self.text.push_str(piece);
                        append(block, "text", piece);
                    }
                    // Kept whole, to be sent back unchanged when the turn goes on.
                    "thinking_delta" => append(block, "thinking", delta["thinking"].as_str().unwrap_or("")),
                    "signature_delta" => append(block, "signature", delta["signature"].as_str().unwrap_or("")),
                    "input_json_delta" => input.push_str(delta["partial_json"].as_str().unwrap_or("")),
                    "citations_delta" => match &mut block["citations"] {
                        Value::Array(list) => list.push(delta["citation"].clone()),
                        other => *other = json!([delta["citation"]]),
                    },
                    _ => {}
                }
            }
            "content_block_stop" => {
                if let (Some(block), Some(input)) = (self.blocks.get_mut(at), self.inputs.get(at)) {
                    if !input.is_empty() {
                        block["input"] = serde_json::from_str(input).map_err(|_| UNREADABLE.to_string())?;
                    }
                }
            }
            "message_delta" => {
                if let Some(stop) = event["delta"]["stop_reason"].as_str() {
                    self.stop = Some(stop.into());
                }
                if let Some(why) = event.pointer("/delta/stop_details/explanation").and_then(Value::as_str) {
                    self.refusal = Some(why.into());
                }
            }
            "error" => return Err(super::brief(event["error"]["message"].as_str().unwrap_or("The provider reported an error."))),
            _ => {}
        }
        Ok(())
    }

    /// The words so far. A local model's thinking stays unsaid, as in the chat
    /// (local_chat.rs), and so does what may be the start of its tag until the
    /// stream says which it is; `whole`: nothing more is coming.
    fn words(&self, whole: bool) -> String {
        if self.wire == Wire::Anthropic {
            return self.text.clone();
        }
        let mut words = local_chat::progressive_filter(&self.text);
        if let Some(at) = words.rfind('<').filter(|at| !whole && "<think>".starts_with(&words[*at..])) {
            words.truncate(at);
        }
        words
    }

    /// Tells the caller the words it has not heard yet; answers all of them.
    fn tell(&mut self, id: &str, whole: bool) -> String {
        let words = self.words(whole);
        if let Some(fresh) = words.get(self.told..).filter(|fresh| !fresh.is_empty()) {
            say(id, fresh);
            self.told = words.len();
        }
        words
    }

    fn open_ai_done(mut self, id: &str, mut messages: Vec<Value>) -> Result<Value, String> {
        let text = self.tell(id, true);
        let cut = self.stop.as_deref() == Some("length");
        if self.stop.as_deref() == Some("content_filter") {
            return Err("The model declined this one.".into());
        }
        // A call that the token limit cut short is never handed over to be run.
        let (mut calls, mut tools) = (Vec::new(), Vec::new());
        for (i, call) in self.calls.iter().enumerate().filter(|(_, call)| !cut && !call.name.is_empty()) {
            let id = if call.id.is_empty() { format!("call_{i}") } else { call.id.clone() };
            let input: Value = match call.input.trim() {
                "" => json!({}),
                written => serde_json::from_str(written).map_err(|_| UNREADABLE.to_string())?,
            };
            calls.push(json!({ "id": id, "type": "function", "function": { "name": call.name, "arguments": input.to_string() } }));
            tools.push(json!({ "id": id, "name": call.name, "input": input }));
        }
        if text.is_empty() && tools.is_empty() {
            return Err(t("No response text."));
        }
        let mut message = json!({ "role": "assistant", "content": text });
        if !calls.is_empty() {
            message["tool_calls"] = json!(calls);
        }
        messages.push(message);
        let stop = if !tools.is_empty() { "tool_use" } else if cut { "max_tokens" } else { "end_turn" };
        Ok(done(stop, text, tools, messages))
    }
}

#[cfg(test)]
mod tests {
    // Two stand-in servers of our own on loopback, one in each format. No real
    // provider is called and the credential store is never opened: a test that
    // read a key would panic (`no_key`).
    //
    //   cargo test -p meteor --lib chat::headless -- --nocapture

    use super::*;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::mpsc::{self, Receiver};
    use std::time::Instant;

    const WAIT: Duration = Duration::from_secs(10);
    const HEAD: &str = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n";

    // ── The stand-in ──────────────────────────────────────────────────────────

    /// What a stand-in was sent: the request's head in lower case, and its JSON.
    struct Asked {
        head: String,
        body: Value,
    }

    struct StandIn {
        url: String,
        asked: Receiver<Asked>,
        /// Told when the client was gone before a reply was written out.
        hung_up: Receiver<()>,
    }

    /// Pieces written one after the other, each after its pause in
    /// milliseconds; then the connection closes.
    type Reply = Vec<(u64, Vec<u8>)>;

    /// A server on a free loopback port that answers its nth request with the nth reply.
    fn stand_in(replies: Vec<Reply>) -> StandIn {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (tell, asked) = mpsc::channel();
        let (gone, hung_up) = mpsc::channel();
        std::thread::spawn(move || {
            for reply in replies {
                let Ok((mut conn, _)) = listener.accept() else { return };
                let _ = tell.send(request(&mut conn));
                for (pause, piece) in reply {
                    std::thread::sleep(Duration::from_millis(pause));
                    if conn.write_all(&piece).and_then(|_| conn.flush()).is_err() {
                        let _ = gone.send(());
                        return;
                    }
                }
            }
        });
        StandIn { url, asked, hung_up }
    }

    fn request(conn: &mut TcpStream) -> Asked {
        let (mut bytes, mut buf) = (Vec::new(), [0u8; 8192]);
        loop {
            let n = conn.read(&mut buf).unwrap_or(0);
            bytes.extend_from_slice(&buf[..n]);
            if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                let length = head.lines().find_map(|l| l.strip_prefix("content-length:")).map_or(0, |v| v.trim().parse().unwrap());
                if bytes.len() >= end + 4 + length || n == 0 {
                    return Asked { head, body: serde_json::from_slice(&bytes[end + 4..]).unwrap_or(Value::Null) };
                }
            } else if n == 0 {
                return Asked { head: String::new(), body: Value::Null };
            }
        }
    }

    /// An event stream: each event a `data:` line, with the `event:` line
    /// Anthropic writes before it and the `[DONE]` an OpenAI stream ends on.
    fn stream(wire: Wire, events: &[Value]) -> Vec<u8> {
        let mut out = String::from(HEAD);
        for event in events {
            if wire == Wire::Anthropic {
                out.push_str(&format!("event: {}\n", event["type"].as_str().unwrap()));
            }
            out.push_str(&format!("data: {event}\n\n"));
        }
        if wire == Wire::OpenAi {
            out.push_str("data: [DONE]\n\n");
        }
        out.into_bytes()
    }

    fn at_once(wire: Wire, events: &[Value]) -> Reply {
        vec![(0, stream(wire, events))]
    }

    // OpenAI's events.
    fn word(text: &str) -> Value {
        json!({ "choices": [{ "index": 0, "delta": { "content": text } }] })
    }
    fn call_part(part: Value) -> Value {
        json!({ "choices": [{ "index": 0, "delta": { "tool_calls": [part] } }] })
    }
    fn finished(why: &str) -> Value {
        json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": why }] })
    }

    // Anthropic's events.
    fn message_start() -> Value {
        json!({ "type": "message_start", "message": { "id": "msg_1", "type": "message", "role": "assistant", "model": CLAUDE_MODEL, "content": [], "stop_reason": null, "usage": { "input_tokens": 20, "output_tokens": 1 } } })
    }
    fn block(index: usize, block: Value) -> Value {
        json!({ "type": "content_block_start", "index": index, "content_block": block })
    }
    fn text_block(index: usize) -> Value {
        block(index, json!({ "type": "text", "text": "" }))
    }
    fn delta(index: usize, delta: Value) -> Value {
        json!({ "type": "content_block_delta", "index": index, "delta": delta })
    }
    fn text(index: usize, text: &str) -> Value {
        delta(index, json!({ "type": "text_delta", "text": text }))
    }
    fn input(index: usize, json: &str) -> Value {
        delta(index, json!({ "type": "input_json_delta", "partial_json": json }))
    }
    fn block_stop(index: usize) -> Value {
        json!({ "type": "content_block_stop", "index": index })
    }
    fn stopped(why: &str) -> Value {
        json!({ "type": "message_delta", "delta": { "stop_reason": why, "stop_sequence": null }, "usage": { "output_tokens": 12 } })
    }
    fn message_stop() -> Value {
        json!({ "type": "message_stop" })
    }

    // ── The caller ────────────────────────────────────────────────────────────

    struct Heard(Receiver<(String, Value)>);

    impl Heard {
        /// The words of this ask's `ask-delta` events, and its `ask-done`.
        fn out(&self, id: &str) -> (String, Value) {
            let (mut words, mut deltas) = (String::new(), 0);
            loop {
                let (event, payload) = self.0.recv_timeout(WAIT).expect("the ask ends");
                assert_eq!(payload["id"], id);
                match event.as_str() {
                    "ask-delta" => {
                        deltas += 1;
                        words.push_str(payload["text"].as_str().unwrap());
                    }
                    "ask-done" => {
                        let how = json!({ "stop": payload["stop"], "tools": payload["tools"], "error": payload["error"] });
                        println!("{id}: {deltas} deltas {words:?}, done {how}");
                        return (words, payload);
                    }
                    other => panic!("unexpected event {other}"),
                }
            }
        }
    }

    fn start(target: Target, args: Value) -> (String, Heard) {
        let (tell, heard) = mpsc::channel();
        let tell = Mutex::new(tell);
        let emit: Emit = Arc::new(move |event: &str, payload: Value| {
            let _ = tell.lock().unwrap().send((event.to_string(), payload));
        });
        let id = begin(target, args, emit).unwrap()["id"].as_str().unwrap().to_string();
        (id, Heard(heard))
    }

    fn no_key(key: &str) -> Option<String> {
        panic!("a test opened the credential store for {key}")
    }

    /// As `ask` does, for a provider that needs no key.
    fn start_local(args: Value) -> (String, Heard) {
        let target = resolve(&args, &Settings::default(), &no_key).unwrap();
        start(target, args)
    }

    /// Claude's format at the stand-in, with a key that is nobody's.
    fn claude_at(server: &StandIn) -> Target {
        target(Wire::Anthropic, &format!("{}/v1/messages", server.url), Some("test-key".into()), CLAUDE_MODEL.into()).unwrap()
    }

    fn timer_tool() -> Value {
        json!([{ "name": "start_timer", "description": "Starts a timer.", "parameters": { "type": "object", "properties": { "minutes": { "type": "number" }, "label": { "type": "string" } }, "required": ["minutes"] } }])
    }

    // ── A loopback OpenAI-compatible server ───────────────────────────────────

    #[test]
    fn a_local_server_streams_its_words_and_its_thinking_stays_unsaid() {
        let events = [
            json!({ "choices": [{ "index": 0, "delta": { "role": "assistant" } }] }),
            word("<think>\nThe user asks about a caf"),
            word("é.\n</thi"),
            word("nk>"),
            json!({ "choices": [{ "index": 0, "delta": { "content": null, "reasoning_content": "hidden" } }] }),
            word("Le café "),
            word("ouvre à 8 h"),
            word(". 2 <"),
            word(" 3."),
            finished("stop"),
        ];
        // Written seven bytes at a time: pieces end mid-line and mid-character.
        let reply: Reply = stream(Wire::OpenAi, &events).chunks(7).map(|piece| (1, piece.to_vec())).collect();
        let server = stand_in(vec![reply]);
        let question = json!({ "role": "user", "content": "Quand ouvre le café ?" });
        let (id, heard) = start_local(json!({
            "id": "t-words", "provider": "builtin", "url": server.url, "model": "Qwen3.5-4B", "system": "Answer aloud.",
            "messages": [question], "max_tokens": 96, "stream": true,
        }));
        assert_eq!(id, "t-words");
        let (words, done) = heard.out(&id);
        assert_eq!(words, "Le café ouvre à 8 h. 2 < 3.");
        assert_eq!((&done["stop"], &done["text"], &done["tools"]), (&json!("end_turn"), &json!(words), &json!([])));
        assert_eq!(done["messages"], json!([question, { "role": "assistant", "content": words }]));

        let asked = server.asked.recv_timeout(WAIT).unwrap();
        assert!(asked.head.starts_with("post /v1/chat/completions "), "{}", asked.head);
        // The placeholder every local server accepts: no key went anywhere.
        assert!(asked.head.contains("authorization: bearer ollama"), "{}", asked.head);
        assert!(!asked.head.contains("x-api-key"));
        assert_eq!(
            asked.body,
            json!({
                "cache_prompt": true, "chat_template_kwargs": { "enable_thinking": false },
                "stream": true, "max_tokens": 96, "model": "Qwen3.5-4B",
                "messages": [{ "role": "system", "content": "Answer aloud." }, question],
            })
        );
    }

    #[test]
    fn a_local_model_calls_a_tool_and_is_told_what_it_answered() {
        let first = [
            call_part(json!({ "index": 0, "id": "call_7", "type": "function", "function": { "name": "start_timer", "arguments": "" } })),
            call_part(json!({ "index": 0, "function": { "arguments": "{\"minu" } })),
            call_part(json!({ "index": 0, "function": { "arguments": "tes\": 10}" } })),
            finished("tool_calls"),
        ];
        let second = [word("Ten minutes, "), word("starting now."), finished("stop")];
        let server = stand_in(vec![at_once(Wire::OpenAi, &first), at_once(Wire::OpenAi, &second)]);
        let question = json!({ "role": "user", "content": "Set a timer for ten minutes." });

        let (id, heard) = start_local(json!({ "provider": "builtin", "url": server.url, "tools": timer_tool(), "messages": [question] }));
        let (words, done) = heard.out(&id);
        assert_eq!(words, "");
        assert_eq!(done["stop"], "tool_use");
        assert_eq!(done["tools"], json!([{ "id": "call_7", "name": "start_timer", "input": { "minutes": 10 } }]));
        let asked = server.asked.recv_timeout(WAIT).unwrap();
        assert_eq!(asked.body["max_tokens"], MAX_TOKENS);
        let function = json!({ "name": "start_timer", "description": "Starts a timer.", "parameters": timer_tool()[0]["parameters"] });
        assert_eq!(asked.body["tools"], json!([{ "type": "function", "function": function }]));

        // The caller ran the timer: the conversation goes back with its answer.
        let (id, heard) = start_local(json!({
            "provider": "builtin", "url": server.url, "tools": timer_tool(),
            "messages": done["messages"], "results": [{ "id": "call_7", "content": "Timer set." }],
        }));
        let (words, done) = heard.out(&id);
        assert_eq!(words, "Ten minutes, starting now.");
        assert_eq!(done["stop"], "end_turn");
        let call = json!({ "id": "call_7", "type": "function", "function": { "name": "start_timer", "arguments": "{\"minutes\":10}" } });
        assert_eq!(
            server.asked.recv_timeout(WAIT).unwrap().body["messages"],
            json!([question, { "role": "assistant", "content": "", "tool_calls": [call] }, { "role": "tool", "tool_call_id": "call_7", "content": "Timer set." }])
        );
    }

    // ── Anthropic ─────────────────────────────────────────────────────────────

    #[test]
    fn claude_streams_calls_a_tool_and_gets_its_own_thinking_back() {
        let first = [
            message_start(),
            block(0, json!({ "type": "thinking", "thinking": "", "signature": "" })),
            delta(0, json!({ "type": "signature_delta", "signature": "sig-abc" })),
            block_stop(0),
            json!({ "type": "ping" }),
            text_block(1),
            text(1, "Setting "),
            text(1, "it."),
            block_stop(1),
            block(2, json!({ "type": "tool_use", "id": "toolu_1", "name": "start_timer", "input": {} })),
            input(2, "{\"minutes\": 1"),
            input(2, "0, \"label\": \"pasta\"}"),
            block_stop(2),
            stopped("tool_use"),
            message_stop(),
        ];
        let second = [message_start(), text_block(0), text(0, "Ten minutes for the pasta."), block_stop(0), stopped("end_turn"), message_stop()];
        let in_pieces: Reply = stream(Wire::Anthropic, &first).chunks(11).map(|piece| (1, piece.to_vec())).collect();
        let server = stand_in(vec![in_pieces, at_once(Wire::Anthropic, &second)]);
        let question = json!({ "role": "user", "content": "Ten minutes for the pasta." });

        let (id, heard) = start(
            claude_at(&server),
            json!({ "provider": "anthropic", "system": "Answer aloud.", "tools": timer_tool(), "web_search": true, "messages": [question] }),
        );
        let (words, done) = heard.out(&id);
        assert_eq!(words, "Setting it.");
        assert_eq!((&done["stop"], &done["text"]), (&json!("tool_use"), &json!("Setting it.")));
        let call = json!({ "id": "toolu_1", "name": "start_timer", "input": { "minutes": 10, "label": "pasta" } });
        assert_eq!(done["tools"], json!([call]));
        // The whole turn as Claude wrote it, the thinking block and its signature included.
        let wrote = json!({ "role": "assistant", "content": [
            { "type": "thinking", "thinking": "", "signature": "sig-abc" },
            { "type": "text", "text": "Setting it." },
            { "type": "tool_use", "id": "toolu_1", "name": "start_timer", "input": call["input"] },
        ] });
        assert_eq!(done["messages"], json!([question, wrote]));

        let asked = server.asked.recv_timeout(WAIT).unwrap();
        assert!(asked.head.starts_with("post /v1/messages "), "{}", asked.head);
        assert!(asked.head.contains("x-api-key: test-key"), "{}", asked.head);
        assert!(asked.head.contains("anthropic-version: 2023-06-01"));
        assert!(!asked.head.contains("authorization:"));
        assert_eq!(
            asked.body,
            json!({
                "model": "claude-haiku-5-5", "max_tokens": 1024, "stream": true, "system": "Answer aloud.",
                "output_config": { "effort": "low" }, "cache_control": { "type": "ephemeral" },
                "tools": [
                    { "name": "start_timer", "description": "Starts a timer.", "input_schema": timer_tool()[0]["parameters"] },
                    { "type": "web_search_20260209", "name": "web_search", "max_uses": 3 },
                ],
                "messages": [question],
            })
        );

        // The caller ran the timer: its answer goes back after Claude's own turn, unchanged.
        let (id, heard) = start(
            claude_at(&server),
            json!({ "provider": "anthropic", "tools": timer_tool(), "messages": done["messages"], "results": [{ "id": "toolu_1", "content": "Timer set." }] }),
        );
        let (words, done) = heard.out(&id);
        assert_eq!(words, "Ten minutes for the pasta.");
        assert_eq!(done["stop"], "end_turn");
        let told = json!({ "role": "user", "content": [{ "type": "tool_result", "tool_use_id": "toolu_1", "content": "Timer set.", "is_error": false }] });
        assert_eq!(server.asked.recv_timeout(WAIT).unwrap().body["messages"], json!([question, wrote, told]));
    }

    #[test]
    fn a_turn_the_server_paused_goes_on_under_the_same_id() {
        let found = json!({ "type": "web_search_tool_result", "tool_use_id": "srvtoolu_1", "content": [
            { "type": "web_search_result", "title": "Paris", "url": "https://example.com/paris", "encrypted_content": "abc" },
        ] });
        let first = [
            message_start(),
            text_block(0),
            text(0, "Let me look. "),
            block_stop(0),
            block(1, json!({ "type": "server_tool_use", "id": "srvtoolu_1", "name": "web_search", "input": {} })),
            input(1, "{\"query\": \"weather Paris\"}"),
            block_stop(1),
            stopped("pause_turn"),
            message_stop(),
        ];
        let second = [message_start(), block(0, found.clone()), block_stop(0), text_block(1), text(1, "It is sunny in Paris."), block_stop(1), stopped("end_turn"), message_stop()];
        let server = stand_in(vec![at_once(Wire::Anthropic, &first), at_once(Wire::Anthropic, &second)]);
        let question = json!({ "role": "user", "content": "What is the weather in Paris?" });

        let (id, heard) = start(claude_at(&server), json!({ "provider": "anthropic", "web_search": true, "messages": [question] }));
        let (words, done) = heard.out(&id);
        assert_eq!(words, "Let me look. It is sunny in Paris.");
        assert_eq!((&done["stop"], &done["text"], &done["tools"]), (&json!("end_turn"), &json!(words), &json!([])));
        // One end for the two requests, and nothing after it.
        assert_eq!(heard.0.recv_timeout(WAIT), Err(mpsc::RecvTimeoutError::Disconnected));

        let before = json!([
            { "type": "text", "text": "Let me look. " },
            { "type": "server_tool_use", "id": "srvtoolu_1", "name": "web_search", "input": { "query": "weather Paris" } },
        ]);
        assert_eq!(server.asked.recv_timeout(WAIT).unwrap().body["messages"], json!([question]));
        // The second request: what Claude had written, sent back as it came, and no word of ours added.
        assert_eq!(server.asked.recv_timeout(WAIT).unwrap().body["messages"], json!([question, { "role": "assistant", "content": before }]));
        let whole = json!([before[0], before[1], found, { "type": "text", "text": "It is sunny in Paris." }]);
        assert_eq!(done["messages"], json!([question, { "role": "assistant", "content": whole }]));
    }

    // ── Cancel, failures ──────────────────────────────────────────────────────

    #[test]
    fn a_slow_answer_is_cancelled_by_its_id() {
        for wire in [Wire::OpenAi, Wire::Anthropic] {
            // A word every 30 ms for nine seconds, unless the reader goes away.
            let (lead, one) = match wire {
                Wire::OpenAi => (String::new(), word("word ")),
                Wire::Anthropic => (format!("data: {}\n\ndata: {}\n\n", message_start(), text_block(0)), text(0, "word ")),
            };
            let mut reply: Reply = vec![(0, format!("{HEAD}{lead}").into_bytes())];
            reply.extend((0..300).map(|_| (30, format!("data: {one}\n\n").into_bytes())));
            let server = stand_in(vec![reply]);
            let args = json!({ "provider": "builtin", "url": server.url, "messages": [{ "role": "user", "content": "Tell me a long story." }] });
            let (id, heard) = match wire {
                Wire::OpenAi => start_local(args),
                Wire::Anthropic => start(claude_at(&server), args),
            };

            let (event, first) = heard.0.recv_timeout(WAIT).unwrap();
            assert_eq!((event.as_str(), &first["id"]), ("ask-delta", &json!(id)));
            assert!(first["text"].as_str().unwrap().starts_with("word"), "{first}");
            let at = Instant::now();
            assert!(cancel(&id));
            let (_, done) = heard.out(&id);
            assert_eq!(done, json!({ "id": id, "stop": "cancel" }));
            // The request itself is gone: the server finds nobody to write to.
            server.hung_up.recv_timeout(Duration::from_secs(3)).expect("the request was dropped");
            println!("cancel ({wire:?}): the server lost its reader {} ms after the cancel", at.elapsed().as_millis());
            // And nothing follows the end: the task and what it wrote to are gone.
            assert_eq!(heard.0.recv_timeout(WAIT), Err(mpsc::RecvTimeoutError::Disconnected));
            assert!(!cancel(&id));
        }
    }

    #[test]
    fn an_ask_that_fails_says_why_and_hands_over_no_call() {
        let ends = |wire: Wire, reply: Vec<u8>| -> Value {
            let server = stand_in(vec![vec![(0, reply)]]);
            let args = json!({ "provider": "builtin", "url": server.url, "tools": timer_tool(), "messages": [{ "role": "user", "content": "hi" }] });
            let (id, heard) = match wire {
                Wire::OpenAi => start_local(args),
                Wire::Anthropic => start(claude_at(&server), args),
            };
            let mut done = heard.out(&id).1;
            done.as_object_mut().unwrap().remove("id");
            done
        };
        let error = |why: &str| json!({ "stop": "error", "error": why });
        let claude = |events: &[Value]| ends(Wire::Anthropic, stream(Wire::Anthropic, events));
        let local = |events: &[Value]| ends(Wire::OpenAi, stream(Wire::OpenAi, events));

        // The provider refuses the key.
        let body = r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#;
        let refused = format!("HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        assert_eq!(ends(Wire::Anthropic, refused.into_bytes()), error("HTTP 401: invalid x-api-key"));
        // It gives up halfway, declines, or the line drops.
        let overloaded = json!({ "type": "error", "error": { "type": "overloaded_error", "message": "Overloaded" } });
        assert_eq!(claude(&[message_start(), text_block(0), text(0, "Half"), overloaded]), error("Overloaded"));
        let declined = json!({ "type": "message_delta", "delta": { "stop_reason": "refusal", "stop_details": { "type": "refusal", "category": "cyber", "explanation": "Not this one." } } });
        assert_eq!(claude(&[message_start(), declined, message_stop()]), error("Not this one."));
        assert_eq!(claude(&[message_start(), text_block(0), text(0, "Half")]), error("The answer stopped before its end."));
        assert_eq!(local(&[finished("stop")]), error("No response text."));

        // A tool call nobody can read is an error, never a call.
        let broken = call_part(json!({ "index": 0, "id": "call_1", "function": { "name": "start_timer", "arguments": "{\"minutes\":" } }));
        assert_eq!(local(&[broken.clone(), finished("tool_calls")]), error(UNREADABLE));
        let call = block(0, json!({ "type": "tool_use", "id": "toolu_1", "name": "start_timer", "input": {} }));
        assert_eq!(claude(&[message_start(), call.clone(), input(0, "{\"minutes\":"), block_stop(0), stopped("tool_use"), message_stop()]), error(UNREADABLE));
        // And one the token limit cut short is not handed over to be run.
        let cut = local(&[word("Half"), broken, finished("length")]);
        assert_eq!((&cut["stop"], &cut["text"], &cut["tools"]), (&json!("max_tokens"), &json!("Half"), &json!([])));
        let cut = claude(&[message_start(), call, input(0, "{\"minutes\": 1"), stopped("max_tokens"), message_stop()]);
        assert_eq!((&cut["stop"], &cut["tools"]), (&json!("max_tokens"), &json!([])));
    }

    // ── Who is asked ──────────────────────────────────────────────────────────

    #[test]
    fn the_built_in_model_answers_with_chat_off_and_anyone_else_needs_the_voice_switch() {
        let ask = |provider: &str, voice: Value| permit(&json!({ "provider": provider, "url": "http://127.0.0.1:5199" }), &voice).is_ok();
        // Voice off (null), or on with every switch at its default: only the model on this computer.
        for voice in [Value::Null, json!({ "localServer": false, "freeModel": false, "claude": false })] {
            assert!(ask("builtin", voice.clone()));
            for other in ["", "local", "ollama", "lmstudio", "custom", "anthropic", "openai", "google", "openrouter", "openrouter-free", "nobody"] {
                assert!(!ask(other, voice.clone()), "{other}");
            }
        }
        // Each switch opens its own providers and no others.
        assert!(ask("anthropic", json!({ "claude": true })));
        assert!(!ask("anthropic", json!({ "freeModel": true, "localServer": true })));
        assert!(ask("openrouter-free", json!({ "freeModel": true })) && ask("google", json!({ "freeModel": true })));
        assert!(!ask("google", json!({ "claude": true, "localServer": true })));
        assert!(ask("local", json!({ "localServer": true })) && ask("ollama", json!({ "localServer": true })));
        assert!(!ask("local", json!({ "claude": true, "freeModel": true })));
    }

    #[test]
    fn nobody_is_asked_and_no_key_is_read_unless_the_provider_was_named() {
        let read = Mutex::new(Vec::<String>::new());
        let keys = |key: &str| -> Option<String> {
            read.lock().unwrap().push(key.to_string());
            Some(format!("key-of-{key}"))
        };
        let to = |args: Value, settings: &Settings| resolve(&args, settings, &keys).map(|t| (t.wire, t.url.to_string(), t.key, t.model));
        let none = Settings::default();

        // No provider, or one nobody knows: refused. The chat's own default (Claude) is none here.
        assert!(to(json!({ "messages": [] }), &none).is_err());
        assert!(to(json!({ "provider": "" }), &none).is_err());
        assert!(to(json!({ "provider": "nobody" }), &none).is_err());
        // The built-in model: the address it was given, on this computer only.
        assert!(to(json!({ "provider": "builtin" }), &none).is_err());
        assert!(to(json!({ "provider": "builtin", "url": "https://api.openai.com" }), &none).is_err());
        assert!(to(json!({ "provider": "builtin", "url": "http://192.168.1.20:8080" }), &none).is_err());
        assert_eq!(
            to(json!({ "provider": "builtin", "url": "http://localhost:5199" }), &none).unwrap(),
            (Wire::OpenAi, "http://127.0.0.1:5199/v1/chat/completions".into(), None, String::new())
        );
        // "local": a server from chat, connected, with a model, on this computer.
        assert!(to(json!({ "provider": "local" }), &none).is_err());
        let mut mine = Settings::default();
        mine.ollama_url = "http://192.168.1.20:11434".into();
        mine.chat_models.insert("ollama".into(), "llama3.2".into());
        mine.lmstudio_url = "http://127.0.0.1:1234".into();
        // One on another machine is used when it is named, never implied; one without a model is not ready.
        assert!(to(json!({ "provider": "local" }), &mine).is_err());
        assert_eq!(to(json!({ "provider": "ollama" }), &mine).unwrap().1, "http://192.168.1.20:11434/v1/chat/completions");
        mine.chat_models.insert("lmstudio".into(), "qwen3".into());
        assert_eq!(
            to(json!({ "provider": "local" }), &mine).unwrap(),
            (Wire::OpenAi, "http://127.0.0.1:1234/v1/chat/completions".into(), None, "qwen3".into())
        );
        assert!(read.lock().unwrap().is_empty(), "nothing so far needed a key");

        // Claude and the online ones: each reads its own key and no other.
        let (wire, url, key, model) = to(json!({ "provider": "anthropic" }), &none).unwrap();
        assert_eq!((wire, key.as_deref(), model.as_str()), (Wire::Anthropic, Some("key-of-anthropic-api-key"), "claude-haiku-5-5"));
        assert!(url.ends_with("/v1/messages"), "{url}");
        let free = resolve(&json!({ "provider": "openrouter-free", "model": "some/model:free" }), &none, &keys).unwrap();
        assert_eq!(free.url.as_str(), "https://openrouter.ai/api/v1/chat/completions");
        assert_eq!(free.extra["provider"]["max_price"], json!({ "prompt": 0, "completion": 0, "request": 0 }));
        assert!(free.free.is_some());
        let paid = resolve(&json!({ "provider": "openai" }), &none, &keys).unwrap();
        assert_eq!((paid.url.as_str(), paid.max_tokens_field), ("https://api.openai.com/v1/chat/completions", "max_completion_tokens"));
        assert_eq!(*read.lock().unwrap(), ["anthropic-api-key", "openrouter-api-key", "openai-api-key"]);
        // Without its key a provider is not asked at all.
        assert!(resolve(&json!({ "provider": "anthropic" }), &none, &|_| None).is_err());
    }

    #[test]
    fn reach_says_whether_the_local_server_answers_in_time() {
        let ok = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".to_vec();
        let answers_at = |server: &StandIn| tauri::async_runtime::block_on(answers(&Url::parse(&server.url).unwrap(), None));
        let server = stand_in(vec![vec![(0, ok.clone())]]);
        assert!(answers_at(&server));
        assert!(server.asked.recv_timeout(WAIT).unwrap().head.starts_with("get /v1/models "));
        // One that takes its time is not waited for.
        let slow = stand_in(vec![vec![(2_000, ok)]]);
        let at = Instant::now();
        assert!(!answers_at(&slow));
        println!("reach: gave up on a silent server after {} ms", at.elapsed().as_millis());
        assert!(at.elapsed() < Duration::from_millis(1_500));
        // And with no server connected in the chat, nothing is asked.
        assert!(!reach(&Settings::default()));
    }
}
