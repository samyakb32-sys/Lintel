// The agents' own pages: one child webview per agent inside the hub window,
// laid over the stage the hub page leaves empty. The page tells where the stage
// is (logical pixels from the window's top-left); this only places and shows.
//
// Notes the design rests on:
// - The pages are remote (https). Capabilities grant IPC to local content only
//   (`src-tauri/capabilities`), so an agent's page cannot call a Tauri command.
// - WebView2 has one browser environment per app, so every agent shares this
//   app's cookie store: signing in to an agent signs it in once, for all.
// - Navigation stays on https; nothing else loads.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use notch_core::Ctx;
use serde_json::{json, Value};
use super::{complete, folder, Bus, Decision, Request, Task};
use tauri::{LogicalPosition, LogicalSize, Manager, Url, WebviewBuilder, WebviewUrl};

const HUB: &str = "hub";
const PREFIX: &str = "hub-agent-";
// Must equal BROWSER_ARGS in src-tauri/src/winfactory.rs: WebView2 fixes them for the whole app.
const BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --autoplay-policy=no-user-gesture-required";

/// The adapter script of an agent whose page the hub can read and write.
fn adapter(id: &str) -> Option<&'static str> {
    match id {
        "claude" => Some(include_str!("../adapters/claude.js")),
        "chatgpt" => Some(include_str!("../adapters/chatgpt.js")),
        "gemini" => Some(include_str!("../adapters/gemini.js")),
        "grok" => Some(include_str!("../adapters/grok.js")),
        _ => None,
    }
}

fn num(args: &Value, key: &str) -> Result<f64, String> {
    args.get(key).and_then(Value::as_f64).filter(|n| n.is_finite() && *n >= 0.0).ok_or_else(|| format!("missing {key}"))
}

/// Shows the page of one agent in the stage and hides the others. Created on first use.
pub fn show(cx: &Ctx, args: &Value) -> Result<Value, String> {
    let id = args["id"].as_str().filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')).ok_or("bad id")?;
    let url: Url = args["url"].as_str().ok_or("missing url")?.parse().map_err(|_| "bad url")?;
    if url.scheme() != "https" {
        return Err("an agent page must be https".into());
    }
    let (x, y, w, h) = (num(args, "x")?, num(args, "y")?, num(args, "w")?, num(args, "h")?);
    let app = cx.app();
    let host = app.get_webview_window(HUB).ok_or("the hub window is not open")?;
    let mine = format!("{PREFIX}{id}");
    for (label, view) in app.webviews() {
        if label.starts_with(PREFIX) && label != mine {
            let _ = view.hide();
        }
    }
    match app.get_webview(&mine) {
        Some(view) => {
            view.set_position(LogicalPosition::new(x, y)).map_err(|e| e.to_string())?;
            view.set_size(LogicalSize::new(w, h)).map_err(|e| e.to_string())?;
            view.show().map_err(|e| e.to_string())?;
        }
        None => {
            let mut builder = WebviewBuilder::new(&mine, WebviewUrl::External(url))
                .additional_browser_args(BROWSER_ARGS)
                .on_navigation(|to| to.scheme() == "https");
            if let Some(script) = adapter(id) {
                builder = builder.initialization_script(script);
            }
            host.as_ref().window().add_child(builder, LogicalPosition::new(x, y), LogicalSize::new(w, h)).map_err(|e| e.to_string())?;
        }
    }
    Ok(json!({}))
}

/// Hides every agent page (the stage shows a message instead).
pub fn hide_all(cx: &Ctx) {
    for (label, view) in cx.app().webviews() {
        if label.starts_with(PREFIX) {
            let _ = view.hide();
        }
    }
}

/// Runs `js` in the agent's page and returns what it answered. The adapter's functions are synchronous.
fn run(cx: &Ctx, id: &str, js: &str) -> Result<Value, String> {
    let view = cx.app().get_webview(&format!("{PREFIX}{id}")).ok_or("open that agent's tab first")?;
    let (tx, rx) = mpsc::channel();
    view.eval_with_callback(js.to_string(), move |answer| {
        let _ = tx.send(answer);
    })
    .map_err(|e| e.to_string())?;
    let answer = rx.recv_timeout(Duration::from_secs(5)).map_err(|_| "the page did not answer")?;
    serde_json::from_str(&answer).map_err(|e| e.to_string())
}

/// Reads the tools the agent has switched on (its connectors) and records them as its capabilities.
pub(crate) fn read_caps(cx: &Ctx, bus: &Mutex<Bus>, id: &str) -> Result<Value, String> {
    adapter(id).ok_or("no adapter for this agent")?;
    if run(cx, id, "__hubAdapter.state()")?["ready"] != true {
        return Err("the page is not ready: signed out, or the adapter no longer matches the page".into());
    }
    if run(cx, id, "__hubAdapter.toolsOpen()")?["ok"] != true {
        return Err("the tools menu was not found".into());
    }
    std::thread::sleep(Duration::from_millis(600));
    let read = run(cx, id, "__hubAdapter.toolsRead()")?;
    let caps: Vec<String> = read["tools"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
    if let Some(agent) = bus.lock().unwrap().participants.get_mut(id) {
        agent.caps = caps.clone();
    }
    cx.emit("hub", "caps", json!({ "id": id, "caps": caps }));
    Ok(json!({ "caps": caps }))
}

/// What an agent is told when a folder is shared: how to ask for a file.
const FILE_NOTE: &str = "\n\n(The user shares a folder with you through the hub. If you need a file from it, end your reply with [[hub:read relative/path]], or [[hub:list relative/path]] to see a folder. To save a file there, end your reply with [[hub:write relative/path]], the whole new text of the file, then [[hub:end]]. The hub asks the user each time and answers in this chat. Ask for nothing you do not need.)";

/// What an agent asked for at the end of its reply.
#[derive(Debug, PartialEq)]
enum Ask {
    Read(String),
    List(String),
    Write(String, String),
}

/// The text between `[[hub:write …]]` and `[[hub:end]]`, without the code fence an agent often wraps it in.
fn unfenced(text: &str) -> String {
    let text = text.trim_matches(|c| c == '\n' || c == '\r');
    match (text.strip_prefix("```"), text.ends_with("```")) {
        (Some(rest), true) => rest.split_once('\n').map_or("", |(_, body)| body).trim_end_matches("```").trim_end_matches('\n').to_string(),
        _ => text.to_string(),
    }
}

/// The last request an agent made in its reply.
fn requested(reply: &str) -> Option<Ask> {
    let at = |marker: &str| reply.rfind(marker);
    let last = [("[[hub:read ", 0), ("[[hub:list ", 1), ("[[hub:write ", 2)].into_iter().filter_map(|(m, k)| at(m).map(|i| (i, m, k))).max()?;
    let (i, marker, kind) = last;
    let rest = &reply[i + marker.len()..];
    let end = rest.find("]]")?;
    let path = rest[..end].trim().trim_matches('/').to_string();
    // The note above names `relative/path`: an agent that only quotes it asks for nothing.
    if path.is_empty() || path == "relative/path" {
        return None;
    }
    Some(match kind {
        0 => Ask::Read(path),
        1 => Ask::List(path),
        _ => {
            let body = &rest[end + 2..];
            Ask::Write(path, unfenced(&body[..body.find("[[hub:end]]")?]))
        }
    })
}

enum Wait {
    Reply(String),
    Stopped,
    Failed(String),
}

/// Waits for the page to start and finish writing its reply (two looks a few seconds apart say it is done).
fn wait_for_reply(cx: &Ctx, id: &str, before: &str, epoch: &AtomicU64, mine: u64) -> Wait {
    let (mut seen_busy, mut stable) = (false, 0);
    for _ in 0..300 {
        std::thread::sleep(Duration::from_secs(2));
        if epoch.load(Ordering::Relaxed) != mine {
            return Wait::Stopped;
        }
        let Ok(s) = run(cx, id, "__hubAdapter.state()") else { continue };
        if s["busy"] == true {
            (seen_busy, stable) = (true, 0);
            continue;
        }
        let last = s["last"].as_str().unwrap_or_default();
        if !last.is_empty() && (seen_busy || last != before) {
            stable += 1;
            if stable >= 2 {
                return Wait::Reply(last.to_string());
            }
        }
    }
    Wait::Failed("no reply was read within ten minutes".into())
}

/// Puts a request to the user (or, for a read or a list in auto mode, grants it) and carries it out in the
/// shared folder. `None` when the hub stopped meanwhile.
fn answer(cx: &Ctx, bus: &Mutex<Bus>, epoch: &AtomicU64, mine: u64, task: &Task, ask: Ask) -> Option<String> {
    let (kind, path, content) = match &ask {
        Ask::Read(p) => ("read", p, None),
        Ask::List(p) => ("list", p, None),
        Ask::Write(p, text) => ("write", p, Some(text.clone())),
    };
    // A write that cannot happen is not put to the user.
    let replaces = match &content {
        Some(text) => match folder::check_write(cx, path, text.len()) {
            Ok(replaces) => replaces,
            Err(why) => return Some(format!("Hub: {kind} {path} failed: {why}.")),
        },
        None => false,
    };
    let auto = content.is_none() && cx.settings("hub")["folderAuto"] == true;
    let id = {
        let mut bus = bus.lock().unwrap();
        bus.next += 1;
        let id = bus.next;
        let status = if auto { Decision::Allowed } else { Decision::Pending };
        let request = Request { id, task: task.id, agent: task.to.clone(), kind: kind.into(), path: path.clone(), status, auto, content: content.clone(), replaces };
        bus.requests.insert(id, request.clone());
        cx.emit("hub", "request", request);
        id
    };
    // Ten minutes at most, a look a second, only while a request waits.
    let mut decision = Decision::Pending;
    for _ in 0..600 {
        decision = bus.lock().unwrap().requests.get(&id).map_or(Decision::Denied, |r| r.status);
        if decision != Decision::Pending {
            break;
        }
        std::thread::sleep(Duration::from_secs(1));
        if epoch.load(Ordering::Relaxed) != mine {
            return None;
        }
    }
    if decision != Decision::Allowed {
        return Some(format!("Hub: the user did not allow {kind} {path}."));
    }
    let got = match &ask {
        Ask::Read(_) => folder::read(cx, path).map(|v| v["text"].as_str().unwrap_or_default().chars().take(50_000).collect::<String>()),
        Ask::List(_) => folder::list(cx, path).map(|v| {
            let names = v["entries"].as_array().map(|a| a.iter().filter(|e| e["blocked"] != true).map(|e| format!("{}{}", e["path"].as_str().unwrap_or_default(), if e["dir"] == true { "/" } else { "" })).collect::<Vec<_>>());
            names.unwrap_or_default().join("\n")
        }),
        Ask::Write(_, text) => folder::write(cx, path, text).map(|v| format!("saved, {} bytes{}", v["bytes"], if v["replaced"] == true { ", the old file was kept as a backup" } else { "" })),
    };
    Some(match got {
        Ok(text) if kind == "write" => format!("Hub: {kind} {path}: {text}."),
        Ok(text) => format!("Hub: {kind} {path}\n\n{text}"),
        Err(why) => format!("Hub: {kind} {path} failed: {why}."),
    })
}

/// An approved task goes into the agent's own page, and its reply is read back when it has finished writing.
/// A reply that asks for a file of the shared folder is put to the user, answered in the same chat, and
/// the next reply is read (five requests at most per task). An agent without an adapter keeps the task as
/// sent: nothing reads it, and it waits for `finish`.
/// The thread lives for one task and ends with `stop`.
pub(crate) fn deliver(cx: &Ctx, bus: &Arc<Mutex<Bus>>, epoch: &Arc<AtomicU64>, task: Task) {
    if adapter(&task.to).is_none() {
        return;
    }
    let (cx, bus, epoch) = (cx.clone(), bus.clone(), epoch.clone());
    let mine = epoch.load(Ordering::Relaxed);
    std::thread::spawn(move || {
        let fail = |why: &str| complete(&cx, &bus, task.id, format!("Not delivered: {why}"));
        let sharing = cx.settings("hub")["folder"].as_str().is_some_and(|f| !f.is_empty());
        let mut message = format!("Task from {} via the hub:\n{}{}", task.from, task.text, if sharing { FILE_NOTE } else { "" });
        for asked in 0..=5 {
            let before = match run(&cx, &task.to, "__hubAdapter.state()") {
                Ok(s) if s["ready"] == true => s["last"].as_str().unwrap_or_default().to_string(),
                Ok(_) => return fail("the page is not ready"),
                Err(why) => return fail(&why),
            };
            match run(&cx, &task.to, &format!("__hubAdapter.send({})", Value::String(message.clone()))) {
                Ok(s) if s["ok"] == true => {}
                Ok(s) => return fail(s["error"].as_str().unwrap_or("it could not be sent")),
                Err(why) => return fail(&why),
            }
            let reply = match wait_for_reply(&cx, &task.to, &before, &epoch, mine) {
                Wait::Reply(reply) => reply,
                Wait::Stopped => return,
                Wait::Failed(why) => return fail(&why),
            };
            match requested(&reply).filter(|_| sharing && asked < 5) {
                Some(ask) => match answer(&cx, &bus, &epoch, mine, &task, ask) {
                    Some(text) => message = text,
                    None => return,
                },
                None => return complete(&cx, &bus, task.id, reply),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{requested, Ask};

    #[test]
    fn the_last_request_of_a_reply_is_read() {
        assert_eq!(requested("Sure.\n[[hub:read docs/a.md]]"), Some(Ask::Read("docs/a.md".into())));
        assert_eq!(requested("[[hub:read a.md]] no, rather [[hub:list /src/]]"), Some(Ask::List("src".into())));
        assert_eq!(requested("no request here"), None);
        assert_eq!(requested("[[hub:read relative/path]]"), None);
        assert_eq!(requested("[[hub:delete a.md]]"), None);
        assert_eq!(requested("[[hub:read unterminated"), None);
    }

    #[test]
    fn a_write_carries_its_text() {
        assert_eq!(requested("Here.\n[[hub:write notes/a.md]]\n# Title\n\nbody\n[[hub:end]]"), Some(Ask::Write("notes/a.md".into(), "# Title\n\nbody".into())));
        assert_eq!(requested("[[hub:write a.rs]]\n```rust\nfn main() {}\n```\n[[hub:end]]"), Some(Ask::Write("a.rs".into(), "fn main() {}".into())));
        // Cut off before its end: nothing is written.
        assert_eq!(requested("[[hub:write a.md]]\nhalf a fi"), None);
        // A later read wins over an earlier write.
        assert_eq!(requested("[[hub:write a.md]]\nx\n[[hub:end]]\n[[hub:read b.md]]"), Some(Ask::Read("b.md".into())));
    }
}
