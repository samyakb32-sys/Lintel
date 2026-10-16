// Audio extras — a METEOR V1 feature. The contract is docs/ARCHITECTURE.md §2: nothing
// runs before `start`, everything is gone after `stop`, and every command of
// the feature arrives through `call`.
//
// Three things, each with its own switch:
// - the playing bars follow the music, and the mascot bounces to the song's
//   beat (tap.rs): the loopback is open only while the page says one of them is
//   on screen (`watch { bars, mascot }`) and, for the mascot, while music plays;
// - finer volume steps and replace mode (keys.rs): a stage on the shared
//   keyboard hook, there only while one of the two is on.
//
// Events: `levels` [7 heights, 0…1] thirty times a second, or null when the bars
// go back to their own motion; `beat` { playing, bpm, at } (music plays and the
// bounce is on; the tempo to dance at, 0 while none is known; the Unix time in
// milliseconds of one beat); `volume` { level, mute } after a key applied here.

use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{mpsc, Arc, Mutex, MutexGuard};

use notch_core::{Ctx, Feature, Group, Manifest, Use};
use serde_json::{json, Value};

pub mod bands;
pub mod keys;
pub mod tap;

use keys::Mode;
use tap::{Tap, Told};

const ID: &str = "audio-extras";

static MANIFEST: Manifest = Manifest {
    id: ID,
    name: "Audio extras",
    group: Group::Media,
    // Listens to what the computer plays and can take the volume keys: the user asks for it.
    default_on: false,
    uses: &[Use::InputHook],
    shortcuts: &[],
    tray: &[],
    defaults: r#"{ "liveBars": true, "bounce": true, "fineSteps": false, "replaceHud": false }"#,
};

#[derive(Clone, Copy, Default, PartialEq)]
struct Cfg {
    live_bars: bool,
    bounce: bool,
    keys: Mode,
}

impl Cfg {
    fn of(v: &Value) -> Self {
        let on = |key: &str, default: bool| v[key].as_bool().unwrap_or(default);
        Cfg {
            live_bars: on("liveBars", true),
            bounce: on("bounce", true),
            keys: Mode { fine: on("fineSteps", false), replace: on("replaceHud", false) },
        }
    }
}

/// What the tap's thread and the key thread reach the page through.
struct Out {
    cx: Ctx,
    /// The media service says a player plays.
    playing: AtomicBool,
    bounce: AtomicBool,
    beat: Mutex<Option<(f64, u64)>>,
}

impl Out {
    fn told(&self, told: Told) {
        match told {
            Told::Levels(levels) => {
                let levels = levels.map(|l| l.map(|v| (v * 1000.0).round() / 1000.0));
                self.cx.emit(ID, "levels", levels);
            }
            Told::Beat(beat) => {
                *self.beat.lock().unwrap() = beat;
                self.tell_beat();
            }
            Told::Quiet(why) => notch_core::log::line(format!("{ID}: {why}")),
        }
    }

    fn tell_beat(&self) {
        let playing = self.bounce.load(Relaxed) && self.playing.load(Relaxed);
        let (bpm, at) = self.beat.lock().unwrap().filter(|_| playing).unwrap_or((0.0, 0));
        self.cx.emit(ID, "beat", json!({ "playing": playing, "bpm": bpm, "at": at }));
    }
}

#[derive(Default)]
struct St {
    cfg: Cfg,
    /// What the page says is on screen.
    bars: bool,
    mascot: bool,
    /// Who plays what, to know a new song from a pause.
    song: String,
    media: Option<svc_media::Sub>,
    keys: Option<svc_input::Sub>,
}

struct Core {
    out: Arc<Out>,
    tap: Tap,
    st: Mutex<St>,
}

impl Core {
    fn st(&self) -> MutexGuard<'_, St> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The tap is told who looks, and whether there is a beat to follow.
    fn refresh(&self) {
        let (bars, mascot, follow) = {
            let st = self.st();
            (st.cfg.live_bars && st.bars, st.mascot, st.cfg.bounce && self.out.playing.load(Relaxed))
        };
        self.tap.watch(bars, mascot, follow);
    }

    /// The media service has news: playing or not, and which song.
    fn heard(&self, now: Option<&svc_media::Playback>) {
        let playing = now.is_some_and(|p| p.playing);
        let song = now.map(|p| format!("{}\n{}\n{}", p.source_aumid, p.title, p.artist)).unwrap_or_default();
        let was = self.out.playing.swap(playing, Relaxed);
        let new_song = std::mem::replace(&mut self.st().song, song.clone()) != song;
        if new_song {
            self.tap.new_song();
            *self.out.beat.lock().unwrap() = None;
        } else if playing && !was {
            self.tap.moved();
        }
        if new_song || playing != was {
            self.refresh();
            self.out.tell_beat();
        }
    }

    /// The settings as they are now: the two subscriptions follow them. The
    /// media service is listened to only for the bounce; the bars are on screen
    /// only while something plays anyway.
    fn configure(self: &Arc<Self>, cfg: Cfg) {
        let was = std::mem::replace(&mut self.st().cfg, cfg);
        self.out.bounce.store(cfg.bounce, Relaxed);
        // Subscribed and dropped outside the lock: both services call back on their own threads.
        if cfg.bounce && self.st().media.is_none() {
            let weak = Arc::downgrade(self);
            let sub = svc_media::subscribe(move |now| {
                if let Some(core) = weak.upgrade() {
                    core.heard(now);
                }
            });
            self.st().media = Some(sub);
            // A subscriber that joins a running service is told nothing until the next change.
            self.heard(svc_media::state().as_ref());
        } else if !cfg.bounce {
            let sub = self.st().media.take();
            drop(sub);
            self.out.playing.store(false, Relaxed);
        }
        if cfg.keys != was.keys || (cfg.keys.any() && self.st().keys.is_none()) {
            let old = self.st().keys.take();
            drop(old);
            if cfg.keys.any() {
                let sub = key_stage(&self.out, cfg.keys);
                self.st().keys = Some(sub);
            }
        }
        self.refresh();
        self.out.tell_beat();
    }
}

/// The volume keys, while fine steps or replace mode is on. The hook only hands
/// each press to a thread of its own, which writes the level and tells the
/// page; that thread ends with the hook, whose closure holds the channel.
fn key_stage(out: &Arc<Out>, mode: Mode) -> svc_input::Sub {
    let (tx, rx) = mpsc::channel();
    let out = out.clone();
    let spawned = std::thread::Builder::new().name("audio-extras-keys".into()).spawn(move || {
        let mut asked = None;
        for act in rx {
            match keys::apply(act, &mut asked) {
                Ok((level, mute)) => out.cx.emit(ID, "volume", json!({ "level": level, "mute": mute })),
                Err(err) => notch_core::log::line(format!("{ID}: volume key: {err}")),
            }
        }
    });
    if let Err(err) = spawned {
        notch_core::log::line(format!("{ID}: no thread for the volume keys: {err}"));
    }
    keys::hook(mode, move |act| {
        let _ = tx.send(act);
    })
}

#[derive(Default)]
pub struct AudioExtras {
    running: Mutex<Option<Arc<Core>>>,
}

impl Feature for AudioExtras {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn start(&self, cx: &Ctx) -> Result<(), String> {
        let mut running = self.running.lock().unwrap();
        if running.is_some() {
            return Ok(());
        }
        let out = Arc::new(Out {
            cx: cx.clone(),
            playing: AtomicBool::new(false),
            bounce: AtomicBool::new(false),
            beat: Mutex::new(None),
        });
        let tap = {
            let out = out.clone();
            Tap::new(move |told| out.told(told))
        };
        let core = Arc::new(Core { out, tap, st: Mutex::default() });
        core.configure(Cfg::of(&cx.settings(ID)));
        *running = Some(core);
        Ok(())
    }

    fn stop(&self, cx: &Ctx) {
        let Some(core) = self.running.lock().unwrap().take() else { return };
        let (media, keys) = {
            let mut st = core.st();
            (st.media.take(), st.keys.take())
        };
        drop((media, keys));
        // The tap's thread ends with the last holder of the core: this one, unless a callback is in flight.
        drop(core);
        cx.emit(ID, "levels", Value::Null);
        cx.emit(ID, "beat", json!({ "playing": false, "bpm": 0, "at": 0 }));
    }

    fn settings_changed(&self, _cx: &Ctx, settings: &Value) {
        if let Some(core) = self.running.lock().unwrap().as_ref() {
            core.configure(Cfg::of(settings));
        }
    }

    fn call(&self, _cx: &Ctx, method: &str, args: Value) -> Result<Value, String> {
        let core = self.running.lock().unwrap().clone().ok_or("not running")?;
        match method {
            // The page says what is on screen: the playing bars, the mascot.
            "watch" => {
                {
                    let mut st = core.st();
                    st.bars = args["bars"].as_bool().unwrap_or(false);
                    st.mascot = args["mascot"].as_bool().unwrap_or(false);
                }
                core.refresh();
                core.out.tell_beat();
                Ok(Value::Null)
            }
            _ => Err("unknown method".into()),
        }
    }

    fn strings(&self) -> &'static str {
        include_str!("../../../src/features/audio-extras/strings.json")
    }
}

pub fn feature() -> std::sync::Arc<dyn Feature> {
    std::sync::Arc::new(AudioExtras::default())
}
