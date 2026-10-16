// Breaks — eye and movement break reminders, the fade that can go with them, and screen time.
// The contract is docs/ARCHITECTURE.md §2: nothing runs before `start`, everything is gone after
// `stop`, and every command of the feature arrives through `call`.
//
// Everything inside is off until the person turns it on, and with everything off the feature runs
// nothing at all. With a reminder or screen time on there is one thread that wakes once a second
// (it sleeps through a lock or a sleep, woken by the session event), and two subscriptions of
// svc-sysevents: the session, and the window in front (its WinEvent hook, never a poll).
//
// Events: `state` { break: { kind, left, total } | null, eye, move, held, fade, ev? }, where
// `eye` and `move` are the seconds at the PC until the next break of that kind (null = off) and
// `ev` { phase: "soon" | "held" | "start" | "end", kind, why } is what just happened.
// Commands: `state`, `skip`, `snooze`, `take` { kind }, `usage`, `clear`.

pub mod fade;
pub mod rule;
pub mod sys;
pub mod usage;

use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use fade::Fade;
use notch_core::{Ctx, Feature, Group, Manifest, Use};
use rule::{Config, Ev, Kind, Plan, Rule};
use serde_json::{json, Value};
use svc_sysevents::{Foreground, Session};
use usage::Usage;

const ID: &str = "breaks";
/// No key and no pointer for this long, with nothing full screen: the person is away.
pub const IDLE_AFTER: u64 = 120;
/// With the fade on, the island says so this many seconds before the screens go dark.
const WARN: u64 = 10;
/// A tick this late was a sleep nobody announced: those seconds were spent away.
const GAP: u64 = 5;

static MANIFEST: Manifest = Manifest {
    id: ID,
    name: "Breaks and screen time",
    group: Group::Tools,
    default_on: true,
    // The fade reads Esc and clicks while it is on screen.
    uses: &[Use::InputHook],
    shortcuts: &[],
    tray: &[],
    // eyeEvery, moveEvery, moveMins, snooze: minutes. eyeSecs: seconds.
    defaults: r#"{ "eye": false, "eyeEvery": 20, "eyeSecs": 20, "move": false, "moveEvery": 50, "moveMins": 2, "snooze": 5, "holdCall": true, "holdFullscreen": true, "fade": false, "sound": true, "screenTime": false }"#,
};

#[derive(Clone, Copy, Default)]
struct Cfg {
    rule: Config,
    fade: bool,
    hold_call: bool,
    hold_fullscreen: bool,
    screen_time: bool,
}

impl Cfg {
    fn read(s: &Value) -> Cfg {
        let on = |key: &str| s[key].as_bool().unwrap_or(false);
        let num = |key: &str, min: u64, max: u64, default: u64| s[key].as_f64().map_or(default, |v| (v as u64).clamp(min, max));
        let fade = on("fade");
        Cfg {
            rule: Config {
                eye: Plan { on: on("eye"), every: num("eyeEvery", 1, 180, 20) * 60, secs: num("eyeSecs", 5, 600, 20) },
                mov: Plan { on: on("move"), every: num("moveEvery", 1, 240, 50) * 60, secs: num("moveMins", 1, 30, 2) * 60 },
                snooze: num("snooze", 1, 60, 5) * 60,
                warn: if fade { WARN } else { 0 },
            },
            fade,
            hold_call: on("holdCall"),
            hold_fullscreen: on("holdFullscreen"),
            screen_time: on("screenTime"),
        }
    }

    fn wanted(&self) -> bool {
        self.rule.eye.on || self.rule.mov.on || self.screen_time
    }
}

#[derive(Default)]
struct St {
    cfg: Cfg,
    rule: Rule,
    /// There while screen time is on.
    usage: Option<Usage>,
    fade: Option<Fade>,
    /// Why the person cannot be there: 1 locked, 2 asleep, 4 the display is off.
    gone: u8,
    fullscreen: bool,
    /// The loop of this generation is the one that runs.
    gen: u64,
    live: bool,
    /// A break was started by hand: its seconds are counted from that moment, not from the last tick.
    rebase: bool,
}

/// The `state` event, to every window.
pub type Emit = Arc<dyn Fn(Value) + Send + Sync>;

/// Everything the feature runs. It knows the app only as `emit` and a file, so the probe runs it too.
pub struct Engine {
    emit: Emit,
    /// The screen-time file.
    file: PathBuf,
    st: Mutex<St>,
    wake: Condvar,
    /// What the loop listens to, and the generation it belongs to. Never touched with `st` held:
    /// dropping a subscription waits for the service's thread, which may be waiting for `st`.
    subs: Mutex<(u64, Vec<svc_sysevents::Sub>)>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Engine {
    pub fn new(emit: Emit, file: PathBuf) -> Arc<Engine> {
        Arc::new(Engine {
            emit,
            file,
            st: Mutex::default(),
            wake: Condvar::new(),
            subs: Mutex::default(),
            thread: Mutex::default(),
        })
    }

    fn status(&self, g: &St) -> Value {
        let mut v = g.rule.status();
        v["fade"] = json!(g.cfg.fade);
        v
    }

    pub fn status_now(&self) -> Value {
        self.status(&self.st.lock().unwrap())
    }

    /// What the rule decided goes to the page and to the fade. Called with `st` held.
    fn tell(self: &Arc<Self>, g: &mut St, evs: impl IntoIterator<Item = Ev>) {
        for ev in evs {
            let (phase, kind, why) = match ev {
                Ev::Soon(kind) => ("soon", kind, ""),
                Ev::Held(kind) => ("held", kind, ""),
                Ev::Start(kind, secs) => {
                    if let Some(old) = g.fade.take() {
                        old.close();
                    }
                    if g.cfg.fade {
                        let me = Arc::downgrade(self);
                        let title = match kind {
                            Kind::Eye => "Look at something far away",
                            Kind::Move => "Stand up and stretch",
                        };
                        g.fade = Some(Fade::show(
                            secs,
                            notch_core::i18n::t(title),
                            notch_core::i18n::t("Esc or click to skip"),
                            move || {
                                if let Some(me) = me.upgrade() {
                                    me.end("skip");
                                }
                            },
                        ));
                    }
                    ("start", kind, "")
                }
                Ev::End(kind, why) => {
                    if let Some(fade) = g.fade.take() {
                        fade.close();
                    }
                    ("end", kind, why)
                }
            };
            let mut v = self.status(g);
            v["ev"] = json!({ "phase": phase, "kind": kind.name(), "why": why });
            (self.emit)(v);
        }
    }

    /// Skip or snooze, from the island or from the fade.
    pub fn end(self: &Arc<Self>, why: &'static str) {
        let mut g = self.st.lock().unwrap();
        let ev = if why == "skip" { g.rule.skip() } else { g.rule.stop(why) };
        self.tell(&mut g, ev);
    }

    /// A break now, asked for by hand: it runs even with every reminder off.
    pub fn take(self: &Arc<Self>, kind: Kind) {
        let launch = {
            let mut g = self.st.lock().unwrap();
            let ev = g.rule.take(kind);
            g.rebase |= ev.is_some();
            self.tell(&mut g, ev);
            self.wake.notify_all();
            self.ensure(&mut g)
        };
        if let Some(gen) = launch {
            self.launch(gen);
        }
    }

    /// The settings, as the app keeps them (the manifest's defaults with the person's on top).
    pub fn apply(self: &Arc<Self>, settings: &Value) {
        let cfg = Cfg::read(settings);
        let (launch, fresh) = {
            let mut g = self.st.lock().unwrap();
            let fresh = cfg.screen_time && g.usage.is_none();
            if fresh {
                g.usage = Some(Usage::load(Some(self.file.clone())));
            } else if !cfg.screen_time {
                if let Some(mut usage) = g.usage.take() {
                    usage.save();
                }
            }
            if !cfg.fade {
                if let Some(fade) = g.fade.take() {
                    fade.close();
                }
            }
            g.rule.cfg = cfg.rule;
            g.cfg = cfg;
            (self.emit)(self.status(&g));
            self.wake.notify_all();
            (self.ensure(&mut g), fresh)
        };
        if let Some(gen) = launch {
            self.launch(gen);
        } else if fresh {
            self.front(&svc_sysevents::foreground());
        }
    }

    /// The loop is needed (something is on, or a break is on screen) and does not run: its generation.
    fn ensure(&self, g: &mut St) -> Option<u64> {
        if g.live || !(g.cfg.wanted() || g.rule.on_break().is_some()) {
            return None;
        }
        g.live = true;
        g.gen += 1;
        Some(g.gen)
    }

    fn launch(self: &Arc<Self>, gen: u64) {
        let (a, b) = (Arc::downgrade(self), Arc::downgrade(self));
        let subs = vec![
            svc_sysevents::on_session(move |e| {
                if let Some(me) = a.upgrade() {
                    me.session(e);
                }
            }),
            svc_sysevents::on_foreground(move |fg| {
                if let Some(me) = b.upgrade() {
                    me.front(fg);
                }
            }),
        ];
        *self.subs.lock().unwrap() = (gen, subs);
        self.front(&svc_sysevents::foreground());
        let me = self.clone();
        *self.thread.lock().unwrap() = Some(std::thread::spawn(move || me.run(gen)));
    }

    fn session(self: &Arc<Self>, e: Session) {
        let (bit, on) = match e {
            Session::Lock => (1, true),
            Session::Unlock => (1, false),
            Session::Sleep => (2, true),
            Session::Resume => (2, false),
            Session::DisplayOff => (4, true),
            Session::DisplayOn => (4, false),
            _ => return,
        };
        let mut g = self.st.lock().unwrap();
        if on {
            g.gone |= bit;
            // The person left: that ends a break on screen, and the PC may not come back.
            let ev = g.rule.stop("away");
            self.tell(&mut g, ev);
            if let Some(usage) = g.usage.as_mut() {
                usage.save();
            }
        } else {
            g.gone &= !bit;
        }
        self.wake.notify_all();
    }

    fn front(&self, fg: &Foreground) {
        let counting = {
            let mut g = self.st.lock().unwrap();
            g.fullscreen = fg.fullscreen;
            g.usage.is_some()
        };
        if !counting {
            return;
        }
        // This app's own windows are nobody's screen time. The name is read outside the lock.
        let app = (fg.pid != std::process::id()).then(|| sys::app_of(fg.hwnd, fg.pid, &fg.exe)).flatten();
        if let Some(usage) = self.st.lock().unwrap().usage.as_mut() {
            usage.front(&usage::today(), app);
        }
    }

    fn run(self: Arc<Self>, gen: u64) {
        let mut last = Instant::now();
        let mut unsaved = 0u64;
        let mut g = self.st.lock().unwrap();
        while g.gen == gen {
            // Everything was turned off and no break is on screen: the loop ends itself.
            if !g.cfg.wanted() && g.rule.on_break().is_none() {
                g.live = false;
                break;
            }
            // Locked or asleep there is nothing to count: no tick, the session event wakes it.
            g = if g.gone != 0 && g.rule.on_break().is_none() {
                self.wake.wait(g).unwrap()
            } else {
                let next = (last + Duration::from_secs(1)).saturating_duration_since(Instant::now());
                self.wake.wait_timeout(g, next).unwrap().0
            };
            if g.gen != gen {
                break;
            }
            if std::mem::take(&mut g.rebase) {
                last = Instant::now();
                continue;
            }
            let dt = last.elapsed().as_secs();
            if dt == 0 {
                continue;
            }
            last += Duration::from_secs(dt);
            let away = g.gone != 0 || dt > GAP || (sys::idle_secs() >= IDLE_AFTER && !g.fullscreen);
            if let Some(usage) = g.usage.as_mut() {
                if !away {
                    usage.add(&usage::today(), dt);
                    unsaved += dt;
                }
                // Once a minute while it counts, and when the person leaves.
                if unsaved >= 60 || (away && unsaved > 0) {
                    usage.save();
                    unsaved = 0;
                }
            }
            let (cfg, fullscreen) = (g.cfg, g.fullscreen);
            // The tray's Pause keeps everything off the screen: a break waits for it like for a call.
            let evs = g.rule.tick(dt, away, || {
                notch_core::paused()
                    || (cfg.hold_fullscreen && fullscreen)
                    || (cfg.hold_call && !sys::mic_in_use().is_empty())
            });
            self.tell(&mut g, evs);
        }
        drop(g);
        let mut subs = self.subs.lock().unwrap();
        if subs.0 == gen {
            subs.1.clear();
        }
    }

    /// Today and the days kept, for the page; with `clear`, after forgetting all of it.
    pub fn usage(&self, clear: bool) -> Value {
        let mut g = self.st.lock().unwrap();
        if clear {
            match g.usage.as_mut() {
                Some(usage) => usage.clear(),
                // Off: what was kept is on disk only.
                None => _ = std::fs::remove_file(&self.file),
            }
        }
        g.usage.as_ref().map_or(json!({ "on": false }), |u| u.snapshot(&usage::today()))
    }

    /// Leaves nothing behind: no thread, no subscription, no window.
    pub fn stop(self: &Arc<Self>) {
        let fade = {
            let mut g = self.st.lock().unwrap();
            g.gen += 1;
            g.live = false;
            let fade = g.fade.take();
            let ev = g.rule.stop("off");
            self.tell(&mut g, ev);
            if let Some(mut usage) = g.usage.take() {
                usage.save();
            }
            self.wake.notify_all();
            fade
        };
        // The screens are given back before `stop` returns.
        if let Some(fade) = fade {
            fade.close_wait();
        }
        *self.subs.lock().unwrap() = (0, Vec::new());
        if let Some(thread) = self.thread.lock().unwrap().take() {
            let _ = thread.join();
        }
    }
}

#[derive(Default)]
pub struct Breaks {
    inner: Mutex<Option<Arc<Engine>>>,
}

impl Feature for Breaks {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn start(&self, cx: &Ctx) -> Result<(), String> {
        let inner = self
            .inner
            .lock()
            .unwrap()
            .get_or_insert_with(|| {
                let cx = cx.clone();
                let file = cx.data_dir(ID).join(usage::FILE);
                Engine::new(Arc::new(move |v| cx.emit(ID, "state", v)), file)
            })
            .clone();
        inner.apply(&cx.settings(ID));
        Ok(())
    }

    fn stop(&self, _cx: &Ctx) {
        let inner = self.inner.lock().unwrap().take();
        if let Some(inner) = inner {
            inner.stop();
        }
    }

    fn settings_changed(&self, _cx: &Ctx, settings: &Value) {
        let inner = self.inner.lock().unwrap().clone();
        if let Some(inner) = inner {
            inner.apply(settings);
        }
    }

    fn call(&self, _cx: &Ctx, method: &str, args: Value) -> Result<Value, String> {
        let inner = self.inner.lock().unwrap().clone().ok_or("turned off")?;
        match method {
            "state" => {}
            "skip" => inner.end("skip"),
            "snooze" => inner.end("snooze"),
            "take" => inner.take(args["kind"].as_str().and_then(Kind::parse).ok_or("unknown break")?),
            "usage" | "clear" => return Ok(inner.usage(method == "clear")),
            _ => return Err("unknown method".into()),
        }
        Ok(inner.status_now())
    }

    fn strings(&self) -> &'static str {
        include_str!("../../../src/features/breaks/strings.json")
    }
}

pub fn feature() -> std::sync::Arc<dyn Feature> {
    std::sync::Arc::new(Breaks::default())
}
