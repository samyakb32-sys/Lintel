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

use notch_core::Ctx;
use serde_json::{json, Value};
use tauri::{LogicalPosition, LogicalSize, Manager, Url, WebviewBuilder, WebviewUrl};

const HUB: &str = "hub";
const PREFIX: &str = "hub-agent-";
// Must equal BROWSER_ARGS in src-tauri/src/winfactory.rs: WebView2 fixes them for the whole app.
const BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --autoplay-policy=no-user-gesture-required";

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
            let builder = WebviewBuilder::new(&mine, WebviewUrl::External(url))
                .additional_browser_args(BROWSER_ARGS)
                .on_navigation(|to| to.scheme() == "https");
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
