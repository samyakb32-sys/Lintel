// Files dropped on the island, taken by WebView2 itself.
//
// Tauri's drag-drop goes through drop targets wry registers on the webview's
// child windows. The innermost windows, though, belong to msedgewebview2.exe,
// and drags from the classic Explorer folder view (Downloads, Documents…) never
// walked up to wry's target: "no drop" cursor, nothing reached the app. Only the
// newer WinUI views (Home, Recent) got through.
//
// So `dragDropEnabled` is off for the island and WebView2 takes drops the way
// Edge does. The page accepts the HTML5 drag (src/core/bridge.ts) and hands the
// dropped File objects back with `chrome.webview.postMessageWithAdditionalObjects`;
// here they turn back into real paths and reach the island as a `file-drag` drop.
//
// The message (`dropMessage` in src/core/bridge.ts) is a string shaped like one
// of Tauri's own IPC requests, with our name as its command and no key:
// `{"cmd":"<slug>-file-drop","callback":0,"error":0,"payload":{"feature":""},…}`.
// Every string message also reaches Tauri's IPC. One it cannot read (the bare
// word this used to be) made it write a JSON error to the page's console on
// every drop; one without its key it drops without a word. It cannot be an
// object instead: wry's own handler fails on a message that is not a string,
// and WebView2 then calls no handler after it, so the drop never got here.
// A feature with a drop zone of its own (the shelf) names itself in
// `payload.feature`: the paths then go to that feature's `dropped` call and
// nowhere else.

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{AppHandle, Emitter};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2File, ICoreWebView2WebMessageReceivedEventArgs2,
};
use webview2_com::{take_pwstr, WebMessageReceivedEventHandler};
use wv2_core::{Interface, PWSTR};

use crate::island::{self, WINDOW_LABEL};

#[derive(serde::Serialize, Clone)]
struct FileDrop {
    #[serde(rename = "type")]
    kind: &'static str,
    paths: Vec<String>,
}

/// Whose drop a page's message announces: "" for the island's own, a feature's
/// id for a drop zone of its own. None for any other message.
fn drop_owner(message: &str) -> Option<String> {
    let message: serde_json::Value = serde_json::from_str(message).ok()?;
    if message.get("cmd")?.as_str()? != format!("{}-file-drop", notch_brand::SLUG) {
        return None;
    }
    Some(message.pointer("/payload/feature")?.as_str()?.to_string())
}

/// On while the drop feature runs: a drop that arrives while it is off is ignored.
static ENABLED: AtomicBool = AtomicBool::new(false);
static INSTALLED: AtomicBool = AtomicBool::new(false);

/// The drop feature's switch: an untagged drop is ignored while it is off.
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Puts the handler on the island's webview, once, at launch. It does nothing
/// until a page hands it files: whose they are decides who must be running.
pub fn install(app: &AppHandle) {
    if INSTALLED.swap(true, Ordering::Relaxed) {
        return;
    }
    let Some(win) = island::window(app) else { return };
    let handle = app.clone();
    let result = win.with_webview(move |webview| unsafe {
        let core = match webview.controller().CoreWebView2() {
            Ok(core) => core,
            Err(err) => {
                crate::log::line(format!("file drop: no webview: {err}"));
                return;
            }
        };
        let handler = WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            // Tauri's own IPC messages come through here too; they carry no objects.
            let Ok(with_objects) = args.cast::<ICoreWebView2WebMessageReceivedEventArgs2>() else { return Ok(()) };
            let Ok(objects) = with_objects.AdditionalObjects() else { return Ok(()) };
            let mut count = 0u32;
            objects.Count(&mut count)?;
            if count == 0 {
                return Ok(());
            }
            // Whose drop: the base's (""), or the feature the message names.
            let mut text = PWSTR::null();
            let message = if args.TryGetWebMessageAsString(&mut text).is_ok() { take_pwstr(text) } else { String::new() };
            let Some(owner) = drop_owner(&message) else { return Ok(()) };
            let feature = (!owner.is_empty()).then_some(owner);
            if feature.is_none() && !enabled() {
                return Ok(());
            }
            let mut paths = Vec::new();
            for i in 0..count {
                let Ok(file) = objects.GetValueAtIndex(i)?.cast::<ICoreWebView2File>() else { continue };
                let mut path = PWSTR::null();
                file.Path(&mut path)?;
                paths.push(take_pwstr(path));
            }
            if let Some(feature) = feature {
                // Off this thread: a feature's call never runs inside a webview callback.
                let app = handle.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    if let Err(err) = crate::registry::call(&app, &feature, "dropped", serde_json::json!({ "paths": paths })) {
                        crate::log::line(format!("file drop for {feature}: {err}"));
                    }
                });
                return Ok(());
            }
            crate::files::allow_dropped(paths.iter().cloned());
            let _ = handle.emit_to(WINDOW_LABEL, "file-drag", FileDrop { kind: "drop", paths });
            Ok(())
        }));
        let mut token = 0i64;
        if let Err(err) = core.add_WebMessageReceived(&handler, &mut token) {
            crate::log::line(format!("file drop: cannot listen: {err}"));
        }
    });
    if let Err(err) = result {
        crate::log::line(format!("file drop: {err}"));
    }
}

#[cfg(test)]
mod tests {
    use super::drop_owner;

    #[test]
    fn a_drop_message_names_its_owner_and_nothing_else_is_one() {
        let message = |feature: &str| {
            format!(r#"{{"cmd":"meteor-file-drop","callback":0,"error":0,"payload":{{"feature":"{feature}"}},"__TAURI_INVOKE_KEY__":""}}"#)
        };
        assert_eq!(drop_owner(&message("")).as_deref(), Some(""));
        assert_eq!(drop_owner(&message("shelf")).as_deref(), Some("shelf"));
        // The old bare word, one of Tauri's own requests, a broken message.
        for other in ["meteor-file-drop", r#"{"cmd":"plugin:event|emit","payload":{"feature":"shelf"}}"#, r#"{"cmd":"meteor-file-drop","payload":{}}"#, "", "{"] {
            assert_eq!(drop_owner(other), None, "{other}");
        }
    }
}
