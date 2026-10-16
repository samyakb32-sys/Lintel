// Island extras — a METEOR V1 feature. The contract is docs/ARCHITECTURE.md §2: nothing
// runs before `start`, everything is gone after `stop`, and every command of
// the feature arrives through `call`.
// Rust owns: capture exclusion of the island window, the pointer-display watcher, opening quick-access targets.
// The page owns: the mirror (camera) and the quick-access section. The wheel and key switches are the
// island's own settings (Settings > Island).

pub mod disp;

use notch_core::{Ctx, Feature, Group, Manifest, Use};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use svc_sysevents::Sub;
use tauri::Manager;

static MANIFEST: Manifest = Manifest {
    id: "island-extras",
    name: "Island extras",
    group: Group::Island,
    // Off until the owner has reviewed it: the mirror asks for the camera.
    default_on: true,
    uses: &[Use::Camera],
    shortcuts: &[],
    tray: &[],
    defaults: r#"{ "showInScreenshots": true, "followPointer": false,
      "mirrorShape": "square", "prevScreen": "", "sites": [],
      "quick": [
        { "label": "Settings", "target": "ms-settings:" },
        { "label": "Calculator", "target": "calc.exe" },
        { "label": "Notepad", "target": "notepad.exe" },
        { "label": "Snipping", "target": "ms-screenclip:" },
        { "label": "Downloads", "target": "shell:Downloads" }
      ] }"#,
};

/// A website address for a quick-access tile: only http and https, a host, no user name or password.
/// Returns the address as the browser will get it, or an error.
pub fn web_url(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    let u = url::Url::parse(raw).map_err(|_| "not an address".to_string())?;
    if !matches!(u.scheme(), "http" | "https") || u.host_str().is_none_or(str::is_empty) || !u.username().is_empty() || u.password().is_some() || raw.len() > 2000 {
        return Err("not an address".into());
    }
    Ok(u.into())
}

#[derive(Default)]
pub struct IslandExtras {
    watch: Mutex<Option<Watch>>,
}

/// The pointer-display watcher: one thread that sleeps on a channel, and the sysevents it listens to.
struct Watch {
    tx: Sender<bool>,
    _subs: Vec<Sub>,
}

fn island_hwnd(cx: &Ctx) -> Option<isize> {
    cx.app().get_webview_window("island")?.hwnd().ok().map(|h| h.0 as isize)
}

impl IslandExtras {
    fn apply(&self, cx: &Ctx, s: &Value) {
        if let Some(h) = island_hwnd(cx) {
            let hide = s["showInScreenshots"].as_bool() == Some(false);
            if let Err(e) = disp::set_capture_excluded(h, hide) {
                notch_core::log::line(format!("island-extras: capture flag: {e}"));
            }
        }
        let mut w = self.watch.lock().unwrap();
        let want = s["followPointer"].as_bool() == Some(true);
        if want && w.is_none() {
            *w = Some(spawn_watch(cx.clone()));
        } else if !want {
            if let Some(old) = w.take() {
                let _ = old.tx.send(false);
            }
        }
    }
}

fn spawn_watch(cx: Ctx) -> Watch {
    let (tx, rx) = channel::<bool>();
    let t1 = tx.clone();
    let t2 = tx.clone();
    // With one display there is nowhere to follow the pointer to: the thread sleeps until a display comes.
    let many = Arc::new(AtomicBool::new(false));
    let m2 = many.clone();
    let subs = vec![
        svc_sysevents::on_display_change(move || {
            let _ = t1.send(true);
        }),
        svc_sysevents::on_foreground(move |_| {
            if m2.load(Ordering::Relaxed) {
                let _ = t2.send(true);
            }
        }),
    ];
    std::thread::spawn(move || loop {
        let several = disp::displays().len() > 1;
        many.store(several, Ordering::Relaxed);
        let woke = if several { rx.recv_timeout(Duration::from_millis(500)) } else { rx.recv().map_err(|_| RecvTimeoutError::Disconnected) };
        match woke {
            Ok(false) | Err(RecvTimeoutError::Disconnected) => break,
            _ => {}
        }
        if !several || notch_core::paused() {
            continue;
        }
        let (Some(h), Some((x, y))) = (island_hwnd(&cx), disp::pointer()) else { continue };
        // The page moves the island (Bridge.reposition) when it is not open.
        if disp::display_at(x, y) != disp::display_of_window(h) {
            cx.emit("island-extras", "off-display", json!({}));
        }
    });
    Watch { tx, _subs: subs }
}

impl Feature for IslandExtras {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn start(&self, cx: &Ctx) -> Result<(), String> {
        self.apply(cx, &cx.settings("island-extras"));
        Ok(())
    }

    fn stop(&self, cx: &Ctx) {
        if let Some(w) = self.watch.lock().unwrap().take() {
            let _ = w.tx.send(false);
        }
        // the island is shown in captures again when the feature is off
        if let Some(h) = island_hwnd(cx) {
            let _ = disp::set_capture_excluded(h, false);
        }
    }

    fn settings_changed(&self, cx: &Ctx, s: &Value) {
        self.apply(cx, s);
    }

    fn call(&self, cx: &Ctx, method: &str, args: Value) -> Result<Value, String> {
        match method {
            "open" => {
                let t = args["target"].as_str().unwrap_or("").trim();
                if t.is_empty() {
                    return Err("nothing to open".into());
                }
                disp::open(t)?;
                Ok(json!({}))
            }
            "web_url" => web_url(args["url"].as_str().unwrap_or("")).map(Value::String),
            "state" => Ok(json!({
                "excluded": island_hwnd(cx).and_then(disp::capture_excluded),
                "displays": disp::displays().len(),
            })),
            _ => Err("unknown method".into()),
        }
    }

    fn strings(&self) -> &'static str {
        include_str!("../../../src/features/island-extras/strings.json")
    }
}

pub fn feature() -> std::sync::Arc<dyn Feature> {
    std::sync::Arc::new(IslandExtras::default())
}
