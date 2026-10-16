// Voice assistant — a METEOR V1 feature (docs/VOICE.md). The contract is docs/ARCHITECTURE.md §2: nothing
// runs before `start`, everything is gone after `stop`, and every command arrives through `call`.
//
// This file is the seam between the voice packages (VOICE §10): the shared types, the events, the state,
// the generation number and the dispatch by method prefix. A package fills its own module files and
// never edits this one; what it needs changed here goes into its report as a shared-file request.
//
// How the modules fit:
// - Every module with a method prefix has the same four functions: `start`, `stop`, `settings`, `call`.
//   `start` and `stop` are idempotent and keep what they need in statics of their own file.
// - `ears` is the only module that opens the microphone; it sets idle, waiting, hearing, listening,
//   asking and (at a final transcript) thinking. `mouth` sets speaking. `wake` never opens the
//   microphone: it gets segments from `ears::on_segment`, and owns the guards that call `pause`/`resume`.
// - The three voices (`sapi`, `speech`, `eleven`) have the same three functions: `ready`, `voices`, `synth`.
// - A child process is started through `proc::spawn` only.
// - No words in the log: states, counts and timings only (VOICE §8).

pub mod proc;
pub mod speak_proto;
pub mod timing;

pub mod ears;
pub mod stt;
pub mod turn;
pub mod vad;

pub mod mouth;
pub mod player;
pub mod sapi;

pub mod wake;
pub mod wakephrase;

pub mod caption;
pub mod eleven;
pub mod files;
pub mod generator;
pub mod needle;
pub mod speech;
pub mod weather;

use notch_core::{Ctx, Feature, Group, Manifest, ShortcutDef, Use};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The feature's id: settings, events (`voice:<event>`), data folder, secrets (`voice/<name>`).
pub const ID: &str = "voice";

// ── Events to the page (VOICE §3.5), each `voice:<name>` ──────────────────────

pub mod event {
    /// `StateEvent`: on every change of state.
    pub const STATE: &str = "state";
    /// `Level`: `timing::LEVEL_HZ` times a second, during a turn only.
    pub const LEVEL: &str = "level";
    /// `Transcript`: a partial about once a second, then the final one.
    pub const HEARD: &str = "heard";
    /// `Spoke`: as a chunk starts playing.
    pub const SPOKE: &str = "spoke";
    /// `Download`: progress of a model download (action model, natural voice, text model).
    pub const DOWNLOAD: &str = "download";
    /// The talk key and the turn rules: a turn began, was held, ended (voice-ears).
    pub const TURN: &str = "turn";
    /// A message the user should be told (a deaf engine, a lost device).
    pub const ERROR: &str = "error";
    /// `{wanted, woke}`: the microphone is to be open for the wake phrase or not; the phrase was just heard.
    pub const WAKE: &str = "wake";
}

/// Where events go: `emitter(cx)` in the app, a closure in a probe (an example cannot make a `Ctx`).
pub type Emit = Arc<dyn Fn(&str, Value) + Send + Sync>;

pub fn emitter(cx: &Ctx) -> Emit {
    let cx = cx.clone();
    Arc::new(move |event, payload| cx.emit(ID, event, payload))
}

// ── Shared types ──────────────────────────────────────────────────────────────

pub type Result<T> = std::result::Result<T, VoiceError>;

/// The one error of the crate. `call` hands its text to the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceError {
    /// The package that fills this module has not landed (its name).
    Unavailable(&'static str),
    /// A model or engine is not downloaded, or fails its hash (what).
    Missing(String),
    /// Something did not answer within its `timing` row (what).
    Timeout(&'static str),
    /// A newer generation took over (§7.4); not shown to the user.
    Cancelled,
    /// A switch is off or a rule of §8 forbids it.
    Refused(String),
    Failed(String),
}

impl std::fmt::Display for VoiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VoiceError::Unavailable(package) => write!(f, "not built yet ({package})"),
            VoiceError::Missing(what) => write!(f, "{what} is not downloaded"),
            VoiceError::Timeout(what) => write!(f, "{what} did not answer in time"),
            VoiceError::Cancelled => f.write_str("cancelled"),
            VoiceError::Refused(why) | VoiceError::Failed(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for VoiceError {}

impl From<VoiceError> for String {
    fn from(e: VoiceError) -> String {
        e.to_string()
    }
}

/// Mono float audio, -1 to 1, with its sample rate (16 kHz from the microphone, 24 kHz from Kokoro,
/// whatever a Windows voice gives).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AudioChunk {
    pub rate: u32,
    pub samples: Vec<f32>,
}

impl AudioChunk {
    pub fn duration(&self) -> Duration {
        Duration::from_secs_f64(self.samples.len() as f64 / self.rate.max(1) as f64)
    }
}

/// A voice segment the detection cut, or a sliding window of long speech (`window`), as `wake` gets it.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub audio: AudioChunk,
    /// How much of it was voiced (the energy gate of §6).
    pub voiced: Duration,
    /// Mean power of the voiced frames, and the noise floor at the time.
    pub power: f32,
    pub noise: f32,
    pub window: bool,
}

/// `voice:heard {text, final}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transcript {
    pub text: String,
    #[serde(rename = "final")]
    pub is_final: bool,
}

/// `voice:level {mic, out}`, each 0 to 1.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Level {
    pub mic: f32,
    pub out: f32,
}

/// `voice:spoke {gen, seq}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spoke {
    pub gen: u64,
    pub seq: u32,
}

/// `voice:download {what, received, total, done, error?}`; `what` is `needle`, `speech` or `generator`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Download {
    pub what: String,
    pub received: u64,
    pub total: u64,
    pub done: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    Kokoro,
    Windows,
    Eleven,
}

/// One voice of the picker. `id` is `<engine>:<the engine's own id>` (`kokoro:af_heart`,
/// `windows:Microsoft David`, `eleven:<voice id>`); it is what the setting `voice` and `mouth.say` carry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceDesc {
    pub id: String,
    pub name: String,
    pub engine: Engine,
    /// BCP 47 (`en-US`, `en-GB`).
    pub lang: String,
}

impl VoiceDesc {
    /// `kokoro:af_heart` → (Kokoro, "af_heart").
    pub fn split(id: &str) -> Option<(Engine, &str)> {
        let (engine, own) = id.split_once(':')?;
        let engine = match engine {
            "kokoro" => Engine::Kokoro,
            "windows" => Engine::Windows,
            "eleven" => Engine::Eleven,
            _ => return None,
        };
        Some((engine, own))
    }
}

/// One chunk for a voice to synthesise. `voice` is the engine's own id (after the colon).
#[derive(Debug, Clone, PartialEq)]
pub struct SynthRequest {
    pub text: String,
    pub voice: String,
    /// 1.0 is the voice's own pace.
    pub rate: f32,
    /// 1.0 is the voice's own pitch; the Windows voices only.
    pub pitch: f32,
}

/// What a voice hands back: one sentence may come in several pieces. `false` from the sink means
/// "no longer wanted" (a newer generation): the voice stops and returns `Err(Cancelled)`.
pub type AudioSink<'a> = &'a mut dyn FnMut(AudioChunk) -> bool;

/// What a turn waits for: the first-word and end-of-turn rows of §3.4 (`ears.listen {kind}`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ListenKind {
    /// After a wake or a tap: first word 5 s, silence 1.6 s (2.6 s after an extending word).
    #[default]
    Command,
    /// A reply to a question: first word 8 s.
    Reply,
    /// Yes or no: first word 5 s.
    YesNo,
    /// Awaiting a dropped file: 20 s.
    File,
    /// A free-text answer (a message to dictate): silence 3.2 s.
    FreeText,
}

// ── State (§3.4) and generation (§7.4) ────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PauseReason {
    Tray,
    Lock,
    Sleep,
    DisplayOff,
    EnergySaver,
    Fullscreen,
    /// Dictation or a recording has the microphone.
    MicBusy,
    /// Every microphone is muted.
    MicMuted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Off,
    /// Shortcut only, microphone closed.
    Idle,
    /// Wake on, microphone open.
    Waiting,
    /// A voice is heard while waiting, not yet known to be the wake phrase.
    Hearing,
    /// A command is being taken.
    Listening,
    Thinking,
    Speaking,
    /// A question was put; the microphone waits for the reply.
    Asking,
    /// The turn is over; captions stay `timing::CAPTIONS_STAY`.
    Closing,
    Paused(PauseReason),
}

impl State {
    pub fn name(self) -> &'static str {
        match self {
            State::Off => "off",
            State::Idle => "idle",
            State::Waiting => "waiting",
            State::Hearing => "hearing",
            State::Listening => "listening",
            State::Thinking => "thinking",
            State::Speaking => "speaking",
            State::Asking => "asking",
            State::Closing => "closing",
            State::Paused(_) => "paused",
        }
    }

    /// The states the page may set (`state.set`): not off, not paused.
    pub fn from_name(name: &str) -> Option<State> {
        [State::Idle, State::Waiting, State::Hearing, State::Listening, State::Thinking, State::Speaking, State::Asking, State::Closing]
            .into_iter()
            .find(|s| s.name() == name)
    }
}

/// `voice:state {state, reason?, gen}`. `gen` is the current generation: the page puts it in
/// `mouth.say`, and anything it still holds for an older one is stale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct StateEvent {
    pub state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<PauseReason>,
    pub gen: u64,
}

impl StateEvent {
    pub fn now() -> StateEvent {
        let state = state();
        let reason = match state {
            State::Paused(reason) => Some(reason),
            _ => None,
        };
        StateEvent { state: state.name(), reason, gen: generation() }
    }
}

static STATE: Mutex<State> = Mutex::new(State::Off);
static GENERATION: AtomicU64 = AtomicU64::new(0);

pub fn state() -> State {
    *STATE.lock().unwrap()
}

/// The one place the state changes; `voice:state` goes out when it did.
pub fn set_state(emit: &Emit, next: State) {
    {
        let mut now = STATE.lock().unwrap();
        if *now == next {
            return;
        }
        *now = next;
    }
    notch_core::log::line(format!("voice: state {}", next.name()));
    emit(event::STATE, json!(StateEvent::now()));
}

pub fn generation() -> u64 {
    GENERATION.load(Ordering::SeqCst)
}

/// An interruption (§7.4): raises the generation and returns the new one. Whatever was queued,
/// synthesised late or streamed for an older one is dropped by whoever holds it. Callers that must also
/// silence the player use `mouth::halt`, which calls this.
pub fn interrupt() -> u64 {
    GENERATION.fetch_add(1, Ordering::SeqCst) + 1
}

/// Everything goes quiet at once (§3.3): the microphone closes, speech stops, every child ends.
/// Called by the guards in `wake` (tray Pause, lock, sleep, display off, energy saver…).
pub fn pause(cx: &Ctx, reason: PauseReason) {
    ears::stop(cx);
    mouth::stop(cx);
    proc::kill_all();
    set_state(&emitter(cx), State::Paused(reason));
}

/// The reason is gone: back to idle, and `ears::start` reopens the microphone if the wake is wanted.
pub fn resume(cx: &Ctx) {
    if !matches!(state(), State::Paused(_)) {
        return;
    }
    set_state(&emitter(cx), State::Idle);
    let _ = mouth::start(cx);
    let _ = ears::start(cx);
}

// ── The feature ───────────────────────────────────────────────────────────────

static MANIFEST: Manifest = Manifest {
    id: ID,
    name: "Voice",
    group: Group::Tools,
    default_on: true,
    uses: &[Use::Microphone, Use::Network, Use::InputHook],
    // A chord the user may add; the talk key itself (Right Ctrl alone) is the setting `key`, read by `ears`.
    shortcuts: &[ShortcutDef { id: "talk", label: "Talk to the assistant", default: None }],
    tray: &[("listen", "Listen for the wake phrase")],
    // VOICE §9 with the owner's decisions. `key`: a key name or "" (cleared). `wake`: "off", "on" or
    // "plugged" (only on mains power). `voice`: a `VoiceDesc::id`, "" = Kokoro when downloaded, else Windows.
    // `brain`: "auto", a fixed choice or "off". `keepVoice`, `keepModel`: minutes.
    defaults: r#"{"key":"RCtrl","hold":true,"wake":"off","speak":true,"shortReplies":"spoken","captions":true,"voice":"","kokoroVoice":"af_heart","rate":1.0,"winRate":1.1,"winPitch":1.0,"keepVoice":5,"bargeIn":false,"esc":true,"duck":true,"needle":true,"brain":"auto","threads":2,"keepModel":10,"localServer":false,"freeModel":false,"claude":false,"claudeSearch":false,"eleven":false,"elevenVoice":"","weather":false,"city":"","lookup":false,"chatAloud":false}"#,
};

struct Module {
    prefix: &'static str,
    start: fn(&Ctx) -> Result<()>,
    stop: fn(&Ctx),
    settings: fn(&Ctx, &Value),
    call: fn(&Ctx, &str, Value) -> Result<Value>,
}

macro_rules! module {
    ($m:ident) => {
        Module { prefix: stringify!($m), start: $m::start, stop: $m::stop, settings: $m::settings, call: $m::call }
    };
}

/// Started in this order, stopped in reverse; `call("<prefix>.<method>")` goes to its `call` as `<method>`.
static MODULES: [Module; 10] = [
    module!(caption),
    module!(mouth),
    module!(speech),
    module!(eleven),
    module!(needle),
    module!(generator),
    module!(weather),
    module!(files),
    module!(ears),
    module!(wake),
];

#[derive(Default)]
pub struct Voice;

impl Feature for Voice {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn start(&self, cx: &Ctx) -> std::result::Result<(), String> {
        set_state(&emitter(cx), State::Idle);
        for m in &MODULES {
            if let Err(e) = (m.start)(cx) {
                self.stop(cx);
                return Err(e.into());
            }
        }
        // The host hands settings over only when they change: every module gets them once it runs.
        self.settings_changed(cx, &cx.settings(ID));
        Ok(())
    }

    fn stop(&self, cx: &Ctx) {
        for m in MODULES.iter().rev() {
            (m.stop)(cx);
        }
        proc::kill_all();
        interrupt();
        set_state(&emitter(cx), State::Off);
    }

    fn settings_changed(&self, cx: &Ctx, settings: &Value) {
        for m in &MODULES {
            (m.settings)(cx, settings);
        }
    }

    fn call(&self, cx: &Ctx, method: &str, args: Value) -> std::result::Result<Value, String> {
        let (prefix, rest) = method.split_once('.').unwrap_or((method, ""));
        let answer = match (prefix, rest) {
            // The host's own two: the `talk` chord and the tray item `listen`.
            ("shortcut", "") => ears::call(cx, "shortcut", args),
            ("tray", "") => wake::call(cx, "tray", args),
            ("state", "") => Ok(json!(StateEvent::now())),
            // The page's phases (thinking, asking, closing) when Rust cannot know them.
            ("state", "set") => match args["state"].as_str().and_then(State::from_name) {
                Some(next) => {
                    set_state(&emitter(cx), next);
                    Ok(json!(StateEvent::now()))
                }
                None => Err(VoiceError::Failed("unknown state".into())),
            },
            _ => match MODULES.iter().find(|m| m.prefix == prefix) {
                Some(m) => (m.call)(cx, rest, args),
                None => Err(VoiceError::Failed("unknown method".into())),
            },
        };
        answer.map_err(String::from)
    }

    fn strings(&self) -> &'static str {
        include_str!("../../../src/features/voice/strings.json")
    }
}

pub fn feature() -> Arc<dyn Feature> {
    Arc::new(Voice)
}
