// Proves crates/voice/src/{wake,wakephrase}.rs without a microphone and without a sound:
//
//   CARGO_BUILD_JOBS=4 cargo run -p notch-feat-voice --example voice-wake-probe [-- <part of a clip's name>]
//
// 1. The phrase in a transcript: a table of sentences through `wakephrase`.
// 2. The guards as they decide: `wake::Guards`, every reason by itself.
// 3. The spike's clips (target/voice-spike/audio) through the capture path's entry, `ears::feed`, with
//    `wake::Listener` as the wake's sink: the accept counts for the phrase alone and the phrase with a
//    command against the spike's own, and the phrase stripped from the command.
// 4. The confusable clips, silence, noise and hum (target/voice-spike/audio_fa): accepts, each counted.
// 5. One line from start to end: every guard put on and lifted, a turn by the key on battery, its own
//    speech. At every poll the page's rule for the indicator is held against whether the path listens.
// 6. "Stop" while it talks: a Windows voice written to target/voice-dev/voice-wake/stop.wav, read back.
//
// A file stands in for the microphone, which is never opened; nothing is played. What stands in for the
// app here: the probe tells the path what `wake::wanted` answered, as `ears::settings` does, and sets
// the paused state as `crate::pause` does (both need the app's `Ctx`).

use notch_feat_voice::ears::{self, Capture};
use notch_feat_voice::timing::*;
use notch_feat_voice::vad::VadEvent;
use notch_feat_voice::wake::{Guards, Heard, Listener, Why, WAKE};
use notch_feat_voice::{sapi, set_state, state, wakephrase, Emit, ListenKind, PauseReason, Segment, State, SynthRequest};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering::SeqCst};
use std::sync::{Arc, Mutex};
use std::time::Instant;

static FAILED: AtomicU32 = AtomicU32::new(0);

fn check(ok: bool) -> &'static str {
    if ok {
        "ok"
    } else {
        FAILED.fetch_add(1, SeqCst);
        "FAIL"
    }
}

// ── 1. The phrase ─────────────────────────────────────────────────────────────

fn phrases() {
    println!("── the phrase \"{}\" in a transcript (keywords {:?}) ──", wakephrase::phrase(), wakephrase::keywords());
    let cases: &[(&str, Option<&str>)] = &[
        ("OK METEOR.", Some("")),
        ("Okay, METEOR V1.", Some("")),
        ("Oaky METEOR V1.", Some("")),
        ("Okie Noche", Some("")),
        ("OK NOTH", Some("")),
        ("Okay, Knocch", Some("")),
        ("Okay, Nash.", Some("")),
        ("OK METEOR, set a timer for twelve minutes.", Some("set a timer for twelve minutes.")),
        ("\" OK METEOR,\" said a timer for twelve minutes.", Some("said a timer for twelve minutes.")),
        ("Okay, METEOR V1. Set a timer for 12 minutes.", Some("Set a timer for 12 minutes.")),
        ("uh okay lintel pause the music", Some("pause the music")),
        ("OK METEOR search OK METEOR play music", Some("play music")),
        ("the song says ok meteor", Some("")),
        ("Okay, not.", None),
        ("Okay, not that one, the other one.", None),
        ("Okay, not sure.", None),
        ("Okay, nice.", None),
        ("lintel", None),
        ("METEOR V1, how are you", None),
        ("OK", None),
        ("OK Google", None),
        ("Opening METEOR V1.", None),
        ("a METEOR V1 above the rest", None),
        ("\"O King METEOR V1,\" said a timer", None),
        ("OK notchly, set a timer", None),
        ("okkkk METEOR V1", None),
        ("Thank you.", None),
        ("", None),
    ];
    for (text, want) in cases {
        let found = wakephrase::find(text);
        let rest = found.as_ref().map(|wake| wake.rest.as_str());
        println!("  {:<52} → {:<40} {}", format!("{text:?}"), format!("{rest:?}"), check(rest == *want));
    }
    let at = wakephrase::find("uh okay lintel pause").map(|wake| wake.at);
    println!("  the phrase's place in \"uh okay lintel pause\": {at:?} {}", check(at == Some(3)));
    let ok: &[(&str, bool)] = &[("Okay, not sure.", true), ("OK", true), ("Opening METEOR V1.", false), ("Thank you.", false)];
    for (text, want) in ok {
        println!("  an OK word in {text:?}: {} {}", wakephrase::ok_word(text), check(wakephrase::ok_word(text) == *want));
    }
    let stock: &[(&str, bool)] = &[("Thank you.", true), ("Thanks for watching!", true), ("you", true), ("Bye.", true), ("Thank you for the timer", false), ("OK METEOR", false), ("", false)];
    for (text, want) in stock {
        println!("  only a stock phrase, {text:?}: {} {}", wakephrase::stock(text), check(wakephrase::stock(text) == *want));
    }
}

// ── 2. The guards ─────────────────────────────────────────────────────────────

/// Everything in order: on the charger, the page there.
const CLEAR: Guards = Guards { tray: false, locked: false, asleep: false, saver: false, display_off: false, fullscreen: false, muted: false, mains: true, page: true };

fn guards() {
    use PauseReason::*;
    println!("── the guards: is the microphone wanted for the wake phrase, and does everything stop ──");
    let on = json!({ "wake": "on", "bargeIn": false });
    let plugged = json!({ "wake": "plugged", "bargeIn": false });
    let barge = json!({ "wake": "on", "bargeIn": true });
    let off = json!({ "wake": "off" });
    let battery = Guards { mains: false, ..CLEAR };
    let cases: Vec<(&str, Guards, &Value, State, bool, Option<PauseReason>)> = vec![
        ("the switch as it ships (wake: off)", CLEAR, &off, State::Idle, false, None),
        ("no setting at all", CLEAR, &Value::Null, State::Idle, false, None),
        ("wake on, on the charger", CLEAR, &on, State::Idle, true, None),
        ("wake on, on battery", battery, &on, State::Idle, true, None),
        ("only while plugged in, on the charger", CLEAR, &plugged, State::Waiting, true, None),
        ("only while plugged in, on battery", battery, &plugged, State::Waiting, false, None),
        ("the tray's Pause", Guards { tray: true, ..CLEAR }, &on, State::Waiting, false, Some(Tray)),
        ("locked", Guards { locked: true, ..CLEAR }, &on, State::Waiting, false, Some(Lock)),
        ("asleep", Guards { asleep: true, ..CLEAR }, &on, State::Waiting, false, Some(Sleep)),
        ("energy saver", Guards { saver: true, ..CLEAR }, &on, State::Waiting, false, Some(EnergySaver)),
        ("Pause and locked: the first reason", Guards { tray: true, locked: true, ..CLEAR }, &on, State::Waiting, false, Some(Tray)),
        ("the display is off", Guards { display_off: true, ..CLEAR }, &on, State::Waiting, false, None),
        ("a full-screen app in front", Guards { fullscreen: true, ..CLEAR }, &on, State::Waiting, false, None),
        ("the microphone is muted", Guards { muted: true, ..CLEAR }, &on, State::Waiting, false, None),
        ("no page to show the indicator", Guards { page: false, ..CLEAR }, &on, State::Idle, false, None),
        ("it is talking", CLEAR, &on, State::Speaking, false, None),
        ("it is talking and may be interrupted by voice", CLEAR, &barge, State::Speaking, true, None),
        ("thinking, wake on (the microphone stays open)", CLEAR, &on, State::Thinking, true, None),
        ("voice is paused", CLEAR, &on, State::Paused(Lock), false, None),
        ("voice is off", CLEAR, &on, State::Off, false, None),
    ];
    for (name, guards, settings, state, wanted, stops) in cases {
        let (w, s) = (guards.wanted(settings, state), guards.stops());
        println!("  {name:<48} → wanted {w:<5} stops {:<18} {}", format!("{s:?}"), check(w == wanted && s == stops));
    }
}

// ── The capture path with a file for a microphone ─────────────────────────────

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target")
}

/// 16-bit mono PCM at the capture rate, as the spike wrote its clips.
fn read_wav(path: &Path) -> Vec<f32> {
    let bytes = std::fs::read(path).unwrap_or_default();
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let size = u32::from_le_bytes([bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]]) as usize;
        let body = &bytes[at + 8..(at + 8 + size).min(bytes.len())];
        if &bytes[at..at + 4] == b"fmt " {
            let (channels, rate, bits) = (body[2], u32::from_le_bytes([body[4], body[5], body[6], body[7]]), body[14]);
            assert!(channels == 1 && rate == CAPTURE_RATE && bits == 16, "{}: not 16 kHz mono 16-bit", path.display());
        }
        if &bytes[at..at + 4] == b"data" {
            return body.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0).collect();
        }
        at += 8 + size + size % 2;
    }
    Vec::new()
}

fn write_wav(path: &Path, samples: &[f32]) {
    let data: Vec<u8> = samples.iter().flat_map(|s| ((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes()).collect();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    for field in [16u32, 1 | 1 << 16, CAPTURE_RATE, CAPTURE_RATE * 2, 2 | 16 << 16] {
        bytes.extend_from_slice(&field.to_le_bytes());
    }
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&data);
    let _ = std::fs::write(path, bytes);
}

/// One run of the wake's sink.
struct Run {
    window: bool,
    secs: f32,
    voiced: f32,
    heard: Heard,
}

impl Run {
    fn line(&self) -> String {
        let tight = self.heard.tight.as_ref().map_or(String::new(), |tight| format!(", on the tighter cut {tight:?}"));
        let again = self.heard.again.as_ref().map_or(String::new(), |again| format!(", without keywords {again:?}"));
        format!("{} of {:.2} s, {:.2} s voiced: {:?} {:?}{tight}{again}", if self.window { "window" } else { "segment" }, self.secs, self.voiced, self.heard.why, self.heard.text)
    }

    /// The engine was run for it.
    fn ran(&self) -> bool {
        !matches!(self.heard.why, Why::Guard | Why::Spaced | Why::Gate)
    }
}

type Events = Arc<Mutex<Vec<(String, Value)>>>;

/// What the probe shares with the sink: the listener, its runs, and everything sent to the page.
struct Bench {
    dir: PathBuf,
    emit: Emit,
    events: Events,
    listener: Arc<Mutex<Listener>>,
    runs: Arc<Mutex<Vec<Run>>>,
}

impl Bench {
    fn new(dir: &Path) -> Bench {
        let events: Events = Arc::default();
        let sent = events.clone();
        let emit: Emit = Arc::new(move |event, payload| sent.lock().unwrap().push((event.to_string(), payload)));
        let mut listener = Listener::new(dir.to_path_buf(), emit.clone());
        listener.wanted = Some(true);
        let listener = Arc::new(Mutex::new(listener));
        let runs: Arc<Mutex<Vec<Run>>> = Arc::default();
        let (l, r) = (listener.clone(), runs.clone());
        ears::on_segment(Some(Arc::new(move |segment: &Segment| {
            let heard = l.lock().unwrap().hear(segment, Instant::now());
            let woke = heard.woke();
            r.lock().unwrap().push(Run { window: segment.window, secs: segment.audio.duration().as_secs_f32(), voiced: segment.voiced.as_secs_f32(), heard });
            woke
        })));
        Bench { dir: dir.to_path_buf(), emit, events, listener, runs }
    }

    /// A listener that has heard nothing yet. `playing`: something plays, so every wake gets its second pass.
    fn fresh(&self, playing: bool, barge_in: bool) {
        let mut listener = Listener::new(self.dir.clone(), self.emit.clone());
        (listener.playing, listener.barge_in, listener.wanted) = (Some(playing), barge_in, Some(true));
        *self.listener.lock().unwrap() = listener;
        self.runs.lock().unwrap().clear();
    }

    fn take_runs(&self) -> Vec<Run> {
        std::mem::take(&mut *self.runs.lock().unwrap())
    }
}

/// The page's rule for the indicator (`earFor` in src/features/voice/wake.ts), on what Rust sent.
fn ear(state: &str, wanted: bool) -> bool {
    !matches!(state, "off" | "paused") && (wanted || matches!(state, "listening" | "asking" | "waiting" | "hearing"))
}

/// The capture path fed from files, and what the page would know of it.
struct Line<'a> {
    bench: &'a Bench,
    core: Arc<Mutex<Capture>>,
    /// What the path was last told `wake::wanted` answers, and what the page was last sent of it.
    told: bool,
    wanted: bool,
    state: String,
    last: Option<String>,
    /// Said while it talked, as sent on to the page.
    partials: Vec<String>,
    // Since `mark`: polls fed, polls in which the path listened, in which the indicator was on, in which
    // the two differed; and segments the detection began.
    polls: u32,
    mic: u32,
    lit: u32,
    apart: u32,
    starts: u32,
}

impl<'a> Line<'a> {
    fn new(bench: &'a Bench) -> Line<'a> {
        set_state(&bench.emit, State::Idle);
        bench.events.lock().unwrap().clear();
        Line { bench, core: Arc::new(Mutex::new(Capture::new(bench.emit.clone()))), told: false, wanted: false, state: "idle".into(), last: None, partials: Vec::new(), polls: 0, mic: 0, lit: 0, apart: 0, starts: 0 }
    }

    /// What Rust sent since the last look. Answers whether a turn's final transcript was among it.
    fn drain(&mut self) -> bool {
        let mut ended = false;
        for (event, payload) in self.bench.events.lock().unwrap().drain(..) {
            match event.as_str() {
                "state" => self.state = payload["state"].as_str().unwrap_or("").to_string(),
                WAKE => self.wanted = payload["wanted"] == true,
                "heard" if payload["final"] == true => {
                    self.last = payload["text"].as_str().map(str::to_string);
                    ended = true;
                }
                "heard" => self.partials.push(payload["text"].as_str().unwrap_or("").to_string()),
                _ => {}
            }
        }
        ended
    }

    /// What the app does when the guards or the settings changed: everything stops (`crate::pause`), or
    /// `ears` is told what `wake::wanted` answers, the page first.
    fn tell(&mut self, guards: &Guards, settings: &Value) {
        if let Some(reason) = guards.stops() {
            self.core.lock().unwrap().waiting(false);
            self.told = false;
            set_state(&self.bench.emit, State::Paused(reason));
        } else {
            if matches!(state(), State::Paused(_)) {
                set_state(&self.bench.emit, State::Idle);
            }
            let on = guards.wanted(settings, state());
            if on != self.wanted {
                (self.bench.emit)(WAKE, json!({ "wanted": on, "woke": false }));
            }
            let mut core = self.core.lock().unwrap();
            core.waiting(on);
            // A device opened anew learns the room again.
            if on && !self.told {
                core.opened();
            }
            self.told = on;
        }
        self.drain();
    }

    fn mark(&mut self) {
        (self.polls, self.mic, self.lit, self.apart, self.starts) = (0, 0, 0, 0, 0);
        self.last = None;
        self.partials.clear();
    }

    /// One poll through `ears::feed`, the engine run at once; then the indicator against the path.
    fn poll(&mut self, frame: &[f32]) {
        let step = ears::feed(&self.core, frame);
        self.starts += (step.vad == VadEvent::Start) as u32;
        let recording = step.turn.is_some();
        if let Some(job) = step.job {
            ears::transcribe(&self.core, &self.bench.dir, job);
        }
        let ended = self.drain();
        // The capture thread keeps the device while the wake is wanted or a turn is being recorded.
        let mic = self.told || recording && !ended;
        let lit = ear(&self.state, self.wanted);
        self.polls += 1;
        self.mic += mic as u32;
        self.lit += lit as u32;
        self.apart += (mic != lit) as u32;
    }

    /// Audio in polls of 30 ms, then the room until a turn has ended or `limit` seconds of room went by
    /// (at once, if no turn is on).
    fn play(&mut self, audio: &[f32], room: &[f32], limit: f32) {
        let poll = (CAPTURE_POLL.as_secs_f32() * CAPTURE_RATE as f32) as usize;
        let mut source = audio.to_vec();
        let (mut at, mut extra) = (0, 0.0);
        loop {
            let turn = matches!(self.state.as_str(), "listening" | "asking" | "hearing");
            if at + poll > audio.len() && (self.last.is_some() || extra >= limit || !turn && extra >= 0.6) {
                break;
            }
            while source.len() < at + poll {
                if room.is_empty() {
                    source.resize(at + poll, 0.0);
                } else {
                    source.extend_from_slice(room);
                }
            }
            self.poll(&source[at..at + poll]);
            at += poll;
            if at > audio.len() {
                extra += CAPTURE_POLL.as_secs_f32();
            }
        }
    }

    /// "210 polls: the path listened in 210, the indicator was on in 210, apart in 0"
    fn tracked(&self) -> String {
        format!("{} polls: the path listened in {}, the indicator was on in {}, apart in {}", self.polls, self.mic, self.lit, self.apart)
    }
}

/// The room around a clip: its own noise for the white-noise clips (their first 0.3 s holds nothing
/// else), silence for the others.
fn room(name: &str, clip: &[f32]) -> Vec<f32> {
    if name.contains("white") {
        clip[..clip.len().min(4800)].to_vec()
    } else {
        Vec::new()
    }
}

fn names(dir: &Path, filter: &str) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir).map(|d| d.flatten().filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".wav")).map(str::to_string)).collect()).unwrap_or_default();
    names.sort();
    names.retain(|n| n.contains(filter));
    names
}

/// A clip heard while waiting for the wake phrase: a second of the room, the clip, the room again.
fn waiting(bench: &Bench, name: &str, clip: &[f32], limit: f32) -> (Vec<Run>, Option<String>) {
    let on = json!({ "wake": "on" });
    let room = room(name, clip);
    let mut line = Line::new(bench);
    line.tell(&CLEAR, &on);
    let mut source: Vec<f32> = if room.is_empty() { vec![0.0; CAPTURE_RATE as usize] } else { room.iter().cycle().take(CAPTURE_RATE as usize).copied().collect() };
    source.extend_from_slice(clip);
    line.play(&source, &room, limit);
    line.tell(&CLEAR, &json!({ "wake": "off" }));
    (bench.take_runs(), line.last.clone())
}

// ── 3. The accept counts ──────────────────────────────────────────────────────

const SETS: [(&str, &str); 2] = [("w_oknotch", "the phrase alone"), ("wc_oknotch_timer", "the phrase and a command")];
const CONDITIONS: [&str; 3] = ["clean", "white10", "babble5"];

fn tally(counts: &BTreeMap<(String, String), (u32, u32)>, set: &str) -> (String, u32, u32) {
    let per: Vec<(u32, u32)> = CONDITIONS.iter().map(|c| counts.get(&(set.to_string(), c.to_string())).copied().unwrap_or_default()).collect();
    let text = per.iter().zip(CONDITIONS).map(|((yes, of), c)| format!("{c} {yes}/{of}")).collect::<Vec<_>>().join(", ");
    (text, per.iter().map(|p| p.0).sum(), per.iter().map(|p| p.1).sum())
}

fn accepts(bench: &Bench, filter: &str) {
    let audio = root().join("voice-spike/audio");
    println!("── the spike's clips of \"{}\" through the capture path, the wake's sink behind it ──", wakephrase::phrase());
    // What the spike reached: its transcripts of the whole files with the keywords, through the matcher.
    let results: Value = std::fs::read(root().join("voice-spike/whistle_results.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null);
    let mut spike = BTreeMap::<(String, String), (u32, u32)>::new();
    for r in results.as_array().into_iter().flatten().filter(|r| r["mode"] == "kw") {
        let (set, cond) = (r["key"].as_str().unwrap_or(""), r["cond"].as_str().unwrap_or(""));
        if SETS.iter().any(|s| s.0 == set) {
            let count = spike.entry((set.to_string(), cond.to_string())).or_default();
            *count = (count.0 + wakephrase::find(r["text"].as_str().unwrap_or("")).is_some() as u32, count.1 + 1);
        }
    }
    let clips: Vec<String> = names(&audio, filter).into_iter().filter(|n| SETS.iter().any(|s| n.starts_with(&format!("{}__", s.0)))).collect();
    if clips.is_empty() {
        return println!("  SKIPPED: no clips");
    }
    let (mut ours, mut second, mut strict) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
    let (mut stripped, mut commands) = (0, 0);
    for name in &clips {
        let clip = read_wav(&audio.join(format!("{name}.wav")));
        let parts: Vec<&str> = name.split("__").collect();
        let key = (parts[0].to_string(), parts[3].to_string());
        bench.fresh(false, false);
        let (runs, last) = waiting(bench, name, &clip, 9.0);
        let woke = runs.iter().any(|run| run.heard.woke());
        let count: &mut (u32, u32) = ours.entry(key.clone()).or_default();
        *count = (count.0 + woke as u32, count.1 + 1);
        println!("  {name}{}", if woke { "" } else { "   (no wake)" });
        for run in &runs {
            println!("      {}", run.line());
        }
        if woke && parts[0] == "wc_oknotch_timer" {
            let text = last.unwrap_or_default();
            // Stripped: the command is there and neither the phrase nor its OK word leads it.
            let clean = !text.is_empty() && wakephrase::find(&text).is_none() && !text.split_whitespace().next().is_some_and(wakephrase::ok_word);
            commands += 1;
            stripped += clean as u32;
            println!("      the command, as the turn hands it on: {text:?}{}", if clean { "" } else { "   (the phrase is still in it)" });
        }
        // Something plays: the second pass, without keywords, must still hear the OK word.
        bench.fresh(true, false);
        let woke = waiting(bench, name, &clip, 0.6).0.iter().any(|run| run.heard.woke());
        let count: &mut (u32, u32) = second.entry(key.clone()).or_default();
        *count = (count.0 + woke as u32, count.1 + 1);
        // Three wakes that no command followed: the gate is doubled.
        bench.fresh(false, false);
        for _ in 0..3 {
            bench.listener.lock().unwrap().empty_wake(Instant::now());
        }
        let woke = waiting(bench, name, &clip, 0.6).0.iter().any(|run| run.heard.woke());
        let count: &mut (u32, u32) = strict.entry(key).or_default();
        *count = (count.0 + woke as u32, count.1 + 1);
    }
    for (set, what) in SETS {
        let (here, yes, of) = tally(&ours, set);
        let (theirs, spike_yes, spike_of) = tally(&spike, set);
        if of == 0 {
            continue;
        }
        let reached = of < spike_of || yes >= spike_yes;
        println!("  {what}: woke {yes} of {of} ({here}) | the spike, whole files: {spike_yes} of {spike_of} ({theirs}) {}", check(reached));
        println!("      with something playing (second pass): {} of {of} ({}) | under the doubled gate: {} of {of} ({})", tally(&second, set).1, tally(&second, set).0, tally(&strict, set).1, tally(&strict, set).0);
    }
    if commands > 0 {
        println!("  the phrase stripped from the command: {stripped} of {commands} turns that began with it {}", check(stripped * 10 >= commands * 8));
    }
}

// ── 4. What must never wake ───────────────────────────────────────────────────

fn rejects(bench: &Bench, filter: &str) {
    let audio = root().join("voice-spike/audio_fa");
    println!("── what must never wake (target/voice-spike/audio_fa), with and without something playing ──");
    let groups = [("f_", "confusable sentences"), ("z_silence", "silence"), ("z_white", "white noise"), ("z_hum_clicks", "hum with clicks")];
    for (prefix, what) in groups {
        let clips: Vec<String> = names(&audio, filter).into_iter().filter(|n| n.starts_with(prefix)).collect();
        if clips.is_empty() {
            continue;
        }
        let (mut woke, mut ran, mut whys, mut seconds) = (0, 0, BTreeMap::<String, u32>::new(), 0.0);
        for name in &clips {
            let mut clip = read_wav(&audio.join(format!("{name}.wav")));
            if prefix.starts_with("z_") {
                // Twelve times the three seconds: past the forced end at 30 s.
                clip = clip.iter().cycle().take(clip.len() * 12).copied().collect();
            }
            seconds += clip.len() as f32 / CAPTURE_RATE as f32;
            for playing in [false, true] {
                bench.fresh(playing, false);
                let (runs, _) = waiting(bench, "", &clip, 2.0);
                woke += runs.iter().filter(|run| run.heard.woke()).count();
                ran += runs.iter().filter(|run| run.ran()).count();
                for run in &runs {
                    *whys.entry(format!("{:?}", run.heard.why)).or_default() += 1;
                }
                if prefix.starts_with("z_") && !playing {
                    let texts: Vec<String> = runs.iter().filter(|run| run.ran()).map(|run| format!("{:?}", run.heard.text)).collect();
                    let order: Vec<&str> = runs.iter().map(|run| if run.ran() { "run" } else if run.heard.why == Why::Spaced { "spaced" } else { "gate" }).collect();
                    println!("      {name}: {} · the engine wrote {}", if order.is_empty() { "no segment".to_string() } else { order.join(" ") }, if texts.is_empty() { "nothing".to_string() } else { texts.join(" ") });
                }
            }
        }
        println!("  {what}: {woke} accepts in {} clips, {seconds:.0} s, twice ({ran} engine runs; {whys:?}) {}", clips.len(), check(woke == 0));
    }
}

// ── 5. The guards on the path, and the indicator against the microphone ───────

/// A guard is gone: the path is told, and the room for a moment.
fn lifted(line: &mut Line, guards: &Guards, settings: &Value, what: &str) {
    line.mark();
    line.tell(guards, settings);
    line.play(&[0.0; 9600], &[], 0.0);
    println!("      {what}: {} · state {} {}", line.tracked(), line.state, check(line.apart == 0 && line.mic == line.polls && line.state == "waiting"));
}

fn session(bench: &Bench) {
    let audio = root().join("voice-spike/audio");
    let phrase = read_wav(&audio.join("w_oknotch__zira__r10__clean.wav"));
    let command = read_wav(&audio.join("wc_oknotch_timer__zira__r10__clean.wav"));
    let pause = read_wav(&audio.join("c_pause__zira__r10__clean.wav"));
    println!("── one line, every guard: does the path still listen, and is the indicator on exactly then ──");
    if phrase.is_empty() || command.is_empty() || pause.is_empty() {
        return println!("  SKIPPED: no clips");
    }
    let quiet = vec![0.0; CAPTURE_RATE as usize];
    let on = json!({ "wake": "on", "bargeIn": false });
    let plugged = json!({ "wake": "plugged", "bargeIn": false });
    let off = json!({ "wake": "off" });
    bench.fresh(false, false);
    let mut line = Line::new(bench);

    // Before the page has said it can show the indicator, nothing listens.
    line.mark();
    line.tell(&Guards { page: false, ..CLEAR }, &on);
    line.play(&phrase, &[], 0.6);
    println!("  wake on, no page yet, \"{}\" said: {} · segments begun {} · state {} {}", wakephrase::phrase(), line.tracked(), line.starts, line.state, check(line.mic == 0 && line.lit == 0 && line.starts == 0 && bench.take_runs().is_empty()));

    line.mark();
    line.tell(&CLEAR, &on);
    line.play(&quiet, &[], 0.0);
    println!("  the page is there, the room for a second: {} · state {} {}", line.tracked(), line.state, check(line.apart == 0 && line.mic == line.polls && line.state == "waiting"));

    line.mark();
    line.play(&command, &[], 9.0);
    let woke = bench.take_runs().iter().any(|run| run.heard.woke());
    println!("  \"{}\" and a command: woke {woke}, the command {:?} · {} · state {} {}", wakephrase::phrase(), line.last, line.tracked(), line.state, check(woke && line.apart == 0 && line.mic == line.polls && line.state == "thinking"));
    // The page takes the turn on; then the captions have had their time.
    set_state(&bench.emit, State::Closing);
    line.core.lock().unwrap().rest();
    line.drain();

    // Each guard: put on, the phrase said into it, lifted, the room.
    let cases: Vec<(&str, Guards, &Value, &str)> = vec![
        ("the tray's Pause", Guards { tray: true, ..CLEAR }, &on, "paused"),
        ("locked", Guards { locked: true, ..CLEAR }, &on, "paused"),
        ("asleep", Guards { asleep: true, ..CLEAR }, &on, "paused"),
        ("energy saver", Guards { saver: true, ..CLEAR }, &on, "paused"),
        ("the display is off", Guards { display_off: true, ..CLEAR }, &on, "idle"),
        ("a full-screen app in front", Guards { fullscreen: true, ..CLEAR }, &on, "idle"),
        ("the microphone is muted", Guards { muted: true, ..CLEAR }, &on, "idle"),
        ("only while plugged in, the charger pulled", Guards { mains: false, ..CLEAR }, &plugged, "idle"),
        ("the wake's switch turned off", CLEAR, &off, "idle"),
    ];
    for (name, guards, settings, state) in cases {
        line.mark();
        notch_core::set_paused(guards.tray);
        line.tell(&guards, settings);
        line.play(&phrase, &[], 0.6);
        let deaf = line.mic == 0 && line.lit == 0 && line.starts == 0 && bench.take_runs().is_empty();
        println!("  {name}, \"{}\" said: {} · segments begun {} · engine runs 0 · state {} {}", wakephrase::phrase(), line.tracked(), line.starts, line.state, check(deaf && line.state == state));
        notch_core::set_paused(false);
        lifted(&mut line, &CLEAR, if settings["wake"] == "plugged" { &plugged } else { &on }, "lifted");
    }

    // On battery under "only while plugged in" the key still starts a turn: the indicator is on for it, and only for it.
    let battery = Guards { mains: false, ..CLEAR };
    line.tell(&battery, &plugged);
    line.mark();
    line.core.lock().unwrap().listen(ListenKind::Command, false, 1);
    line.core.lock().unwrap().opened();
    line.drain();
    line.play(&pause, &[], 9.0);
    let turn = (line.mic, line.lit, line.apart, line.last.clone());
    line.play(&quiet, &[], 0.0);
    println!("  on battery, a turn by the key: heard {:?} · {} · state {} {}", turn.3, line.tracked(), line.state, check(turn.0 > 0 && turn.2 == 0 && line.apart == 0 && line.mic == turn.0 && line.lit == turn.1 && turn.3.as_ref().is_some_and(|text| !text.is_empty())));
    set_state(&bench.emit, State::Closing);
    line.core.lock().unwrap().rest();
    lifted(&mut line, &CLEAR, &plugged, "the charger is back");

    // Its own speech: the wake's microphone closes for it, unless it may be interrupted by voice.
    line.mark();
    set_state(&bench.emit, State::Speaking);
    line.tell(&CLEAR, &on);
    line.play(&phrase, &[], 0.6);
    println!("  it talks, \"{}\" said: {} · segments begun {} · state {} {}", wakephrase::phrase(), line.tracked(), line.starts, line.state, check(line.mic == 0 && line.lit == 0 && line.starts == 0 && bench.take_runs().is_empty()));
    set_state(&bench.emit, State::Closing);
    line.tell(&CLEAR, &on);
    line.core.lock().unwrap().rest();
    lifted(&mut line, &CLEAR, &on, "it has finished");

    // Had the path been left open (the page gone silent), the listener itself refuses: no engine run.
    for (what, speaking) in [("it talks", true), ("the tray's Pause", false)] {
        line.mark();
        if speaking {
            set_state(&bench.emit, State::Speaking);
        }
        notch_core::set_paused(!speaking);
        line.drain();
        line.play(&phrase, &[], 0.6);
        let runs = bench.take_runs();
        println!("  {what}, the path left open: {} segments reached the listener, {:?}, engine runs {} {}", runs.len(), runs.iter().map(|run| run.heard.why).collect::<Vec<_>>(), runs.iter().filter(|run| run.ran()).count(), check(!runs.is_empty() && runs.iter().all(|run| run.heard.why == Why::Guard)));
        notch_core::set_paused(false);
        set_state(&bench.emit, State::Closing);
        line.core.lock().unwrap().rest();
        line.drain();
    }

    // After all of it, it still wakes.
    line.mark();
    line.play(&phrase, &[], 9.0);
    let woke = bench.take_runs().iter().any(|run| run.heard.woke());
    println!("  and after all of it, \"{}\": woke {woke} · {} {}", wakephrase::phrase(), line.tracked(), check(woke && line.apart == 0));
}

// ── 6. "Stop" while it talks ──────────────────────────────────────────────────

/// A Windows voice saying `text`, to a file at the capture rate, padded and levelled as the spike's clips.
fn say(text: &str, path: &Path) -> Vec<f32> {
    let mut pieces = Vec::new();
    let said = sapi::synth(&SynthRequest { text: text.into(), voice: String::new(), rate: 1.0, pitch: 1.0 }, &mut |piece| {
        pieces.push(piece);
        true
    });
    if said.is_err() || pieces.is_empty() {
        return Vec::new();
    }
    let rate = pieces[0].rate as f32;
    let voice: Vec<f32> = pieces.into_iter().flat_map(|piece| piece.samples).collect();
    let peak = voice.iter().fold(0.0f32, |peak, s| peak.max(s.abs())).max(1e-6);
    let mut clip = vec![0.0; 4800];
    clip.extend((0..(voice.len() as f32 * CAPTURE_RATE as f32 / rate) as usize).map(|i| {
        let at = i as f32 * rate / CAPTURE_RATE as f32;
        let (a, b) = (voice[at as usize], voice[(at as usize + 1).min(voice.len() - 1)]);
        (a + (b - a) * at.fract()) / peak * 0.5
    }));
    clip.extend(vec![0.0; 4800]);
    write_wav(path, &clip);
    read_wav(path)
}

fn stop(bench: &Bench) {
    println!("── while it talks and may be interrupted by voice (the setting bargeIn) ──");
    let out = root().join("voice-dev/voice-wake");
    let _ = std::fs::create_dir_all(&out);
    let clip = say("Stop.", &out.join("stop.wav"));
    let phrase = read_wav(&root().join("voice-spike/audio/w_oknotch__zira__r10__clean.wav"));
    if clip.is_empty() || phrase.is_empty() {
        return println!("  SKIPPED: no Windows voice, or no clips");
    }
    let level = (clip.iter().map(|s| s * s).sum::<f32>() / clip.len() as f32).sqrt();
    println!("  \"Stop.\" by a Windows voice, read back from {}: {:.2} s at {} Hz, level {level:.3} {}", out.join("stop.wav").display(), clip.len() as f32 / CAPTURE_RATE as f32, CAPTURE_RATE, check(clip.len() > 9600 && level > 0.01));
    let barge = json!({ "wake": "on", "bargeIn": true });
    bench.fresh(false, true);
    let mut line = Line::new(bench);
    line.tell(&CLEAR, &barge);
    line.play(&vec![0.0; CAPTURE_RATE as usize], &[], 0.0);
    set_state(&bench.emit, State::Speaking);
    line.tell(&CLEAR, &barge);
    line.mark();
    line.play(&clip, &[], 0.6);
    let runs = bench.take_runs();
    for run in &runs {
        println!("      {}", run.line());
    }
    let said: Vec<String> = line.partials.iter().map(|text| text.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase()).collect();
    println!("  \"Stop.\" said: sent to the page as heard {:?} (its turn loop ends the turn on \"stop\") · {} · state {} {}", line.partials, line.tracked(), line.state, check(said.iter().any(|text| text == "stop") && line.state == "speaking" && line.apart == 0));
    line.mark();
    line.play(&phrase, &[], 9.0);
    let woke = bench.take_runs().iter().any(|run| run.heard.woke());
    println!("  \"{}\" said: woke {woke}, it stops talking and listens · {} · state after the turn {} {}", wakephrase::phrase(), line.tracked(), line.state, check(woke && line.apart == 0));
}

fn main() {
    // `notch_core::log` writes under %LOCALAPPDATA%: this run's lines stay out of the app's own log.
    let out = root().join("voice-dev/voice-wake");
    let _ = std::fs::create_dir_all(&out);
    std::env::set_var("LOCALAPPDATA", &out);
    let filter = std::env::args().nth(1).unwrap_or_default();
    let from = Instant::now();
    phrases();
    guards();
    let dir = root().join("dictation-probe");
    if notch_feat_voice::stt::ready(&dir) {
        let bench = Bench::new(&dir);
        accepts(&bench, &filter);
        rejects(&bench, &filter);
        if filter.is_empty() {
            session(&bench);
            stop(&bench);
        }
        ears::on_segment(None);
    } else {
        println!("SKIPPED: no engine in {}", dir.display());
        FAILED.fetch_add(1, SeqCst);
    }
    let failed = FAILED.load(SeqCst);
    println!("{} in {:.0} s", if failed == 0 { "ok".to_string() } else { format!("{failed} FAILED") }, from.elapsed().as_secs_f32());
    std::process::exit(failed.min(1) as i32);
}
