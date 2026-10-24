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
use super::{complete, Bus, Task};
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

/// An approved task goes into the agent's own page, and its reply is read back when it has finished writing.
/// An agent without an adapter keeps the task as sent: nothing reads it, and it waits for `finish`.
/// The thread lives for one task (two seconds between looks, ten minutes at most) and ends with `stop`.
pub(crate) fn deliver(cx: &Ctx, bus: &Arc<Mutex<Bus>>, epoch: &Arc<AtomicU64>, task: Task) {
    if adapter(&task.to).is_none() {
        return;
    }
    let (cx, bus, epoch) = (cx.clone(), bus.clone(), epoch.clone());
    let mine = epoch.load(Ordering::Relaxed);
    std::thread::spawn(move || {
        let fail = |why: &str| complete(&cx, &bus, task.id, format!("Not delivered: {why}"));
        let before = match run(&cx, &task.to, "__hubAdapter.state()") {
            Ok(s) if s["ready"] == true => s["last"].as_str().unwrap_or_default().to_string(),
            Ok(_) => return fail("the page is not ready"),
            Err(why) => return fail(&why),
        };
        let message = Value::String(format!("Task from {} via the hub:\n{}", task.from, task.text));
        match run(&cx, &task.to, &format!("__hubAdapter.send({message})")) {
            Ok(s) if s["ok"] == true => {}
            Ok(s) => return fail(s["error"].as_str().unwrap_or("it could not be sent")),
            Err(why) => return fail(&why),
        }
        let (mut seen_busy, mut stable) = (false, 0);
        for _ in 0..300 {
            std::thread::sleep(Duration::from_secs(2));
            if epoch.load(Ordering::Relaxed) != mine {
                return;
            }
            let Ok(s) = run(&cx, &task.to, "__hubAdapter.state()") else { continue };
            if s["busy"] == true {
                (seen_busy, stable) = (true, 0);
                continue;
            }
            let last = s["last"].as_str().unwrap_or_default();
            if !last.is_empty() && (seen_busy || last != before) {
                stable += 1;
                if stable >= 2 {
                    return complete(&cx, &bus, task.id, last.to_string());
                }
            }
        }
        fail("no reply was read within ten minutes");
    });
}
