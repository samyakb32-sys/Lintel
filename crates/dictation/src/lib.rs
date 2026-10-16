// Dictation — a METEOR V1 feature. The contract is docs/ARCHITECTURE.md §2: nothing
// runs before `start`, everything is gone after `stop`, and every command of
// the feature arrives through `call`.

pub mod capture;
pub mod engine;
pub mod hold;
pub mod whistle;

use engine::Session;
use notch_core::{Ctx, Feature, Group, Manifest, Use};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

static MANIFEST: Manifest = Manifest {
    id: "dictation",
    name: "Dictation",
    group: Group::Tools,
    // Off until the package works; flip it then if the feature belongs to the core product.
    default_on: false,
    uses: &[Use::Microphone],
    shortcuts: &[],
    tray: &[],
    defaults: r#"{"engine":"whistle","hold":""}"#,
};

enum Running {
    Windows(Session),
    Whistle(capture::Capture),
}

pub struct Dictation {
    hold: Mutex<Option<hold::Hold>>,
    session: Mutex<Option<Running>>,
    downloading: Arc<AtomicBool>,
}

impl Running {
    fn stop(self) {
        match self {
            Running::Windows(s) => s.stop(),
            Running::Whistle(c) => c.stop(),
        }
    }
}

/// What the hold shortcut listens through: the microphone and the engine the user chose, text collected until release.
struct Listening {
    run: Running,
    texts: Arc<Mutex<Vec<String>>>,
    ended: mpsc::Receiver<()>,
}

impl hold::Take for Listening {
    fn finish(self: Box<Self>) -> String {
        self.run.stop();
        // The last phrase is still in the engine; "ended" comes when it is done.
        let _ = self.ended.recv_timeout(Duration::from_secs(45));
        let t = self.texts.lock().unwrap().join(" ");
        t
    }
    fn cancel(self: Box<Self>) {
        self.run.stop();
    }
}

fn open_engine(cx: &Ctx) -> Result<Box<dyn hold::Take>, String> {
    let (texts, (done, ended)) = (Arc::new(Mutex::new(Vec::new())), mpsc::channel::<()>());
    let (cx2, t2, done) = (cx.clone(), texts.clone(), Mutex::new(done));
    let emit: engine::Emit = Arc::new(move |ev, v| match ev {
        "text" => t2.lock().unwrap().push(v["text"].as_str().unwrap_or("").to_string()),
        "ended" => {
            let _ = done.lock().unwrap().send(());
        }
        "level" => cx2.emit("dictation", "level", v),
        "error" => cx2.emit("dictation", "hold-error", v),
        _ => {}
    });
    let run = if whistle_on(cx) {
        let dir = cx.data_dir("dictation");
        if !whistle::ready(&dir) {
            return Err("speech model not downloaded".into());
        }
        Running::Whistle(capture::Capture::start(emit, dir))
    } else {
        Running::Windows(Session::start(emit))
    };
    Ok(Box::new(Listening { run, texts, ended }))
}

impl Dictation {
    /// The hold shortcut: on while one is set in Settings, gone otherwise.
    fn rehook(&self, cx: &Ctx) {
        *self.hold.lock().unwrap() = None;
        let Some(combo) = cx.settings("dictation")["hold"].as_str().and_then(hold::parse) else { return };
        let (c1, c2) = (cx.clone(), cx.clone());
        let hooks = hold::Hooks {
            open: Box::new(move || open_engine(&c1)),
            emit: Arc::new(move |ev, v| c2.emit("dictation", ev, v)),
        };
        *self.hold.lock().unwrap() = Some(hold::Hold::new(combo, Arc::new(hooks)));
    }

    fn halt(&self) {
        if let Some(r) = self.session.lock().unwrap().take() {
            r.stop();
        }
    }
}

impl Feature for Dictation {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn start(&self, cx: &Ctx) -> Result<(), String> {
        self.rehook(cx);
        Ok(())
    }

    fn stop(&self, _cx: &Ctx) {
        *self.hold.lock().unwrap() = None;
        self.halt();
    }

    fn settings_changed(&self, cx: &Ctx, _settings: &Value) {
        self.rehook(cx);
    }

    fn call(&self, cx: &Ctx, method: &str, _args: Value) -> Result<Value, String> {
        match method {
            // Whistle needs no check; the Windows recognizer is created (not started) to see that it works.
            "available" if whistle_on(cx) => Ok(json!({ "ok": true, "engine": "whistle" })),
            "available" => std::thread::spawn(engine::probe).join().map_err(|_| "panic".to_string())?
                .map(|v| json!({ "ok": true, "language": v["language"] }))
                .or_else(|e| Ok(json!({ "ok": false, "error": e.json() }))),
            "status" => Ok(json!({ "installed": whistle::ready(&cx.data_dir("dictation")), "downloading": self.downloading.load(Ordering::SeqCst) })),
            "download" => {
                if self.downloading.swap(true, Ordering::SeqCst) {
                    return Ok(json!({}));
                }
                let (cx, busy) = (cx.clone(), self.downloading.clone());
                std::thread::spawn(move || {
                    let dir = cx.data_dir("dictation");
                    let r = tauri::async_runtime::block_on(whistle::download(&dir, |_| {}));
                    busy.store(false, Ordering::SeqCst);
                    match r {
                        Ok(()) => cx.emit("dictation", "downloaded", json!({})),
                        Err(m) => cx.emit("dictation", "download-error", json!({ "message": m })),
                    }
                });
                Ok(json!({}))
            }
            "start" => {
                self.halt();
                let cx2 = cx.clone();
                let emit: engine::Emit = Arc::new(move |ev, v| cx2.emit("dictation", ev, v));
                let run = if whistle_on(cx) {
                    let dir = cx.data_dir("dictation");
                    if !whistle::ready(&dir) {
                        emit("error", json!({ "code": "nomodel", "privacy": false, "message": "speech model not downloaded" }));
                        return Ok(json!({}));
                    }
                    Running::Whistle(capture::Capture::start(emit, dir))
                } else {
                    Running::Windows(Session::start(emit))
                };
                *self.session.lock().unwrap() = Some(run);
                Ok(json!({}))
            }
            "stop" => {
                self.halt();
                Ok(json!({}))
            }
            "settings" => {
                std::process::Command::new("explorer").arg("ms-settings:privacy-speech").spawn().map_err(|e| e.to_string())?;
                Ok(json!({}))
            }
            _ => Err("unknown method".into()),
        }
    }

    fn strings(&self) -> &'static str {
        include_str!("../../../src/features/dictation/strings.json")
    }
}

fn whistle_on(cx: &Ctx) -> bool {
    cx.settings("dictation")["engine"].as_str() != Some("windows")
}

pub fn feature() -> Arc<dyn Feature> {
    Arc::new(Dictation { hold: Mutex::new(None), session: Mutex::new(None), downloading: Arc::new(AtomicBool::new(false)) })
}
