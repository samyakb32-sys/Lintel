// Ears: the capture thread, the talk key and the turn they drive. Package voice-ears (VOICE §3.2 to §3.4).
//
// The only module that opens the microphone (WASAPI shared, 16 kHz mono float, as dictation/capture.rs),
// and the one that sets idle, waiting, hearing, listening, asking, thinking (at a final transcript) and
// closing through `crate::set_state`. Emits `voice:level {mic}` and `voice:heard`, and two events of its
// own: `voice:turn` (how a turn ended, in numbers) and `voice:error`. Every number is in `crate::timing`.
// Audio stays in memory; no transcript is logged.
//
// Four parts:
// 1. `TalkKey`: the talk key pressed on its own, as pure logic, and the hook and thread that feed it.
// 2. `Capture`: the capture path without a device or a clock. Frames go in (`feed`), states and events
//    come out, and so do the transcriptions it wants run (`Job`, `transcribe`). The microphone feeds it,
//    and so does the probe, from a file.
// 3. The microphone: a thread that lives only while the device is open.
// 4. The module's functions, as the other packages call them.

use crate::stt::{self, SttOptions};
use crate::timing::*;
use crate::turn::{Turn, TurnEnd, TurnStep};
use crate::vad::{Vad, VadEvent};
use crate::{event, mouth, set_state, state, wake, wakephrase};
use crate::{AudioChunk, Download, Emit, ListenKind, Result, Segment, State, Transcript, VoiceError};
use notch_core::Ctx;
use notch_feat_dictation::whistle;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::SeqCst};
use std::sync::{mpsc, Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::*;
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

const ZERO: Duration = Duration::ZERO;

fn samples(d: Duration) -> usize {
    (d.as_secs_f64() * CAPTURE_RATE as f64) as usize
}

fn span(samples: usize) -> Duration {
    Duration::from_secs_f64(samples as f64 / CAPTURE_RATE as f64)
}

/// A panic in someone's callback must not leave voice deaf for the rest of the session.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

// ── 1. The talk key ───────────────────────────────────────────────────────────

/// What the talk key did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Talk {
    Nothing,
    /// Pressed and let go on its own: a turn starts (or, during one, it is over).
    Tap,
    /// Held on its own for `HOLD_AFTER`: it talks until `Release`.
    Hold,
    Release,
}

/// The talk key pressed on its own. Pure: it is told what the keyboard hook saw, with the hook's clock
/// in microseconds (`svc_input::Key::t`). The key is never taken: with another key it is the ordinary
/// key it always was, and the window in front got every press.
pub struct TalkKey {
    hold: bool,
    /// When it went down.
    down: Option<u64>,
    /// Another key, a click or the wheel came with it: a chord, not ours.
    chord: bool,
    holding: bool,
}

impl TalkKey {
    /// `hold`: holding it talks until release (the setting); without it any press on its own is a tap.
    pub fn new(hold: bool) -> TalkKey {
        TalkKey { hold, down: None, chord: false, holding: false }
    }

    /// It went down at `t`. `alone`: no modifier and no mouse button was already down.
    pub fn press(&mut self, t: u64, alone: bool) -> Talk {
        if self.down.is_none() {
            (self.down, self.chord, self.holding) = (Some(t), !alone, false);
        }
        Talk::Nothing
    }

    /// Another key went down, a mouse button, or the wheel turned.
    pub fn other(&mut self) -> Talk {
        if self.down.is_some() && !self.holding {
            self.chord = true;
        }
        Talk::Nothing
    }

    pub fn release(&mut self) -> Talk {
        match (self.down.take(), self.holding, self.chord) {
            (None, ..) | (_, false, true) => Talk::Nothing,
            (_, true, _) => {
                self.holding = false;
                Talk::Release
            }
            _ => Talk::Tap,
        }
    }

    /// Time went by: a key still down on its own at `due` is a hold.
    pub fn tick(&mut self, t: u64) -> Talk {
        if self.due().is_some_and(|due| t >= due) {
            self.holding = true;
            return Talk::Hold;
        }
        Talk::Nothing
    }

    /// When `tick` is to be called, if a press may still become a hold.
    pub fn due(&self) -> Option<u64> {
        self.down.filter(|_| self.hold && self.undecided()).map(|at| at + HOLD_AFTER.as_micros() as u64)
    }

    /// Down, and neither a chord nor a hold yet: what comes next decides.
    pub fn undecided(&self) -> bool {
        self.down.is_some() && !self.chord && !self.holding
    }
}

/// The setting `key`: the names the Settings page may store. "" or a name not known here: no talk key.
fn vk_of(name: &str) -> Option<u16> {
    Some(match name {
        "RCtrl" => 0xA3,
        "RAlt" => 0xA5,
        "RShift" => 0xA1,
        "RWin" => 0x5C,
        "Menu" => 0x5D,
        "ScrollLock" => 0x91,
        "Pause" => 0x13,
        "Insert" => 0x2D,
        f if f.starts_with('F') => 0x6F + f[1..].parse::<u16>().ok().filter(|n| (1..=24).contains(n))?,
        _ => return None,
    })
}

/// No modifier and no mouse button is down besides the key itself (Shift then Right Ctrl, or a drag
/// with Ctrl, is a chord from its first moment).
fn alone(vk: u16) -> bool {
    const HELD: [u16; 13] = [0x01, 0x02, 0x04, 0x05, 0x06, 0x5B, 0x5C, 0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5];
    HELD.iter().filter(|k| **k != vk).all(|k| unsafe { GetAsyncKeyState(*k as i32) } >= 0)
}

enum KeyEvent {
    Press(u64, bool),
    Release,
    Other,
    Esc,
    Quit,
}

/// The keyboard hook of `svc-input`, there only while voice is on, and the thread that times a hold.
/// The hook only looks: every key goes on to the window it was for.
fn hook(vk: Option<u16>, hold: bool, esc: bool) -> (svc_input::Sub, mpsc::Sender<KeyEvent>) {
    let (tx, rx) = mpsc::channel();
    let (keys, down) = (tx.clone(), AtomicBool::new(false));
    // Order 27: behind the debounce (10), the blocked shortcuts (20) and dictation's hold key (25), before the remaps (30).
    let sub = svc_input::keyboard(27, false, move |k| {
        if Some(k.vk) == vk {
            // A key held down repeats its press: only the change counts.
            if down.swap(k.down, SeqCst) != k.down {
                let _ = keys.send(if k.down { KeyEvent::Press(k.t, alone(k.vk)) } else { KeyEvent::Release });
            }
        } else if k.down {
            if down.load(SeqCst) {
                let _ = keys.send(KeyEvent::Other);
            }
            if esc && k.vk == 0x1B {
                let _ = keys.send(KeyEvent::Esc);
            }
        }
        svc_input::Verdict::Pass
    });
    let mice = tx.clone();
    std::thread::spawn(move || timekeeper(rx, mice, hold));
    (sub, tx)
}

/// Asleep until a key event comes; awake on a clock only for the 0.4 s in which a press may become a hold.
fn timekeeper(rx: mpsc::Receiver<KeyEvent>, tx: mpsc::Sender<KeyEvent>, hold: bool) {
    let mut key = TalkKey::new(hold);
    let mut mouse: Option<svc_input::Sub> = None;
    loop {
        let event = match key.due() {
            Some(due) => match rx.recv_timeout(Duration::from_micros(due.saturating_sub(svc_input::now()))) {
                Ok(event) => Some(event),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(_) => return,
            },
            None => match rx.recv() {
                Ok(event) => Some(event),
                Err(_) => return,
            },
        };
        talk(match event {
            Some(KeyEvent::Press(t, alone)) => key.press(t, alone),
            Some(KeyEvent::Release) => key.release(),
            Some(KeyEvent::Other) => key.other(),
            Some(KeyEvent::Esc) => {
                stop_turn();
                Talk::Nothing
            }
            Some(KeyEvent::Quit) => return,
            None => key.tick(svc_input::now()),
        });
        // Ctrl with a click or the wheel is a chord too: the mouse is watched while a press is undecided, and only then.
        if key.undecided() && mouse.is_none() {
            let tx = tx.clone();
            mouse = Some(svc_input::mouse(12, false, move |m| {
                if matches!(m.kind, svc_input::MouseKind::Down(_) | svc_input::MouseKind::Wheel | svc_input::MouseKind::HWheel) {
                    let _ = tx.send(KeyEvent::Other);
                }
                svc_input::Verdict::Pass
            }));
        } else if !key.undecided() {
            mouse = None;
        }
    }
}

/// What a tap, a hold and its release do. The host's chord `talk` is a tap.
fn talk(what: Talk) {
    let begun = match what {
        Talk::Nothing => return,
        Talk::Release => return release(),
        // A tap during a turn says "that was all": it ends as a released hold does.
        Talk::Tap if recording() => return release(),
        Talk::Tap => begin(ListenKind::Command, false),
        Talk::Hold => begin(ListenKind::Command, true),
    };
    if let Err(e) = begun {
        notch_core::log::line(format!("voice: the talk key did not start a turn: {e}"));
        if let Some(emit) = lock(&EARS).as_ref().map(|ears| ears.emit.clone()) {
            emit(ERROR, json!({ "message": e.to_string() }));
        }
    }
}

// ── 2. The capture path ───────────────────────────────────────────────────────

/// `voice:turn {gen, end, rule, audio, voiced, stt, runs, voice: [from, to]}`: how a turn ended, in
/// milliseconds: the row of §3.4 it ended by, what was recorded, how much of it was voiced and where,
/// and the engine's time over its runs. `voice:error {message}`: a turn that could not start or lost its microphone.
pub use crate::event::{ERROR, TURN};

/// A transcription the path wants run. Whoever feeds it runs it (`transcribe`): the microphone on a
/// thread of its own, the probe at once.
pub struct Job {
    id: u64,
    pub audio: AudioChunk,
    pub options: SttOptions,
}

/// What one frame did, for whoever feeds the path.
#[derive(Default)]
pub struct Step {
    /// While waiting: what the detection made of it.
    pub vad: VadEvent,
    /// During a turn: what the turn made of it.
    pub turn: Option<TurnStep>,
    pub job: Option<Job>,
    /// The wake's sink was asked, and its answer.
    pub woke: Option<bool>,
    segment: Option<Segment>,
}

/// A voice segment while waiting for the wake phrase.
struct Seg {
    audio: Vec<f32>,
    /// Frame by frame: its samples, and its power if it was voiced (else 0).
    frames: Vec<(usize, f32)>,
    age: Duration,
    /// When the next sliding window is due.
    window: Duration,
}

impl Seg {
    /// The last `most` of it, as the wake gets it.
    fn cut(&self, most: Duration, noise: f32, window: bool) -> Segment {
        let from = self.audio.len().saturating_sub(samples(most));
        let (mut at, mut voiced, mut power, mut count) = (0, 0, 0.0, 0);
        for &(len, p) in &self.frames {
            if at >= from && p > 0.0 {
                (voiced, power, count) = (voiced + len, power + p, count + 1);
            }
            at += len;
        }
        Segment {
            audio: AudioChunk { rate: CAPTURE_RATE, samples: self.audio[from..].to_vec() },
            voiced: span(voiced),
            power: power / count.max(1) as f32,
            noise,
            window,
        }
    }
}

/// A turn being recorded.
struct Take {
    turn: Turn,
    held: bool,
    gen: u64,
    /// It began with the wake phrase: the engine is given the phrase as keywords, and the text loses
    /// everything up to it.
    woke: bool,
    audio: Vec<f32>,
    /// Still to drop: the first moment of a reply (`FOLLOW_UP_MIC`).
    skip: usize,
    /// The voiced part of `audio`, in samples.
    first: Option<usize>,
    last: usize,
    voiced: Duration,
    /// The transcription that is out; where the audio of the last one asked for ended, and of the last
    /// one answered.
    job: Option<u64>,
    asked: usize,
    seen: Option<usize>,
    /// The turn asked for its provisional transcript while the engine was busy with an older one.
    want: bool,
    /// Since a transcript was last asked for.
    since: Duration,
    text: String,
    /// The engine's time and runs, for the log.
    stt: Duration,
    runs: u32,
    /// Decided: only the final transcript is awaited.
    end: Option<TurnEnd>,
    failed: Option<VoiceError>,
}

impl Take {
    fn new(turn: Turn, held: bool, gen: u64, audio: Vec<f32>) -> Take {
        Take {
            turn,
            held,
            gen,
            woke: false,
            audio,
            skip: 0,
            first: None,
            last: 0,
            voiced: ZERO,
            job: None,
            asked: 0,
            seen: None,
            want: false,
            since: ZERO,
            text: String::new(),
            stt: ZERO,
            runs: 0,
            end: None,
            failed: None,
        }
    }

    /// What the engine gets: the voice with `CAPTURE_RING` of quiet on each side, so that the wait
    /// before the first word and the silence that ended the turn cost it nothing (`STT_QUICK`).
    fn clip(&self) -> (usize, usize) {
        let ring = samples(CAPTURE_RING);
        match self.first {
            Some(first) => (first.saturating_sub(ring), (self.last + ring).min(self.audio.len())),
            None => (0, self.audio.len()),
        }
    }

    fn worth(&self) -> bool {
        self.voiced >= WORTH_TRANSCRIBING
    }

    /// The last transcript is of all the voice there is.
    fn fresh(&self) -> bool {
        self.seen == Some(self.clip().1)
    }

    /// A transcript of what there is, unless one is out.
    fn ask(&mut self, ids: &mut u64) -> Option<Job> {
        if self.job.is_some() {
            return None;
        }
        let (from, to) = self.clip();
        *ids += 1;
        (self.job, self.asked, self.since) = (Some(*ids), to, ZERO);
        Some(Job {
            id: *ids,
            audio: AudioChunk { rate: CAPTURE_RATE, samples: self.audio[from..to].to_vec() },
            options: SttOptions { language: Some("en".into()), keywords: if self.woke { wakephrase::keywords() } else { Vec::new() } },
        })
    }
}

/// The capture path: detection while waiting for the wake phrase, the recording of a turn, its end
/// and its transcripts. It has no device and no clock: time is the audio it is fed.
pub struct Capture {
    emit: Emit,
    vad: Vad,
    /// The microphone stays open for the wake phrase.
    wake: bool,
    /// Which capture thread feeds it; 0: none, the microphone is closed.
    mic: u64,
    /// The last `CAPTURE_RING`, while no voice is heard.
    ring: Vec<f32>,
    seg: Option<Seg>,
    take: Option<Take>,
    jobs: u64,
    /// Since the last `voice:level`.
    level: Duration,
    /// Turns in a row with voice and no text; then the capture is asked to open again.
    deaf: u32,
    reopen: bool,
}

impl Capture {
    pub fn new(emit: Emit) -> Capture {
        Capture { emit, vad: Vad::new(CAPTURE_RATE, true), wake: false, mic: 0, ring: Vec::new(), seg: None, take: None, jobs: 0, level: ZERO, deaf: 0, reopen: false }
    }

    /// The microphone is to stay open for the wake phrase, or not.
    pub fn waiting(&mut self, on: bool) {
        self.wake = on;
        if !on {
            self.seg = None;
            if matches!(state(), State::Waiting | State::Hearing) {
                self.rest();
            }
        }
    }

    /// A source feeds it from now on (the device is open): the floor is learned again.
    pub fn opened(&mut self) {
        self.vad.reset();
        self.ring.clear();
        (self.seg, self.reopen) = (None, false);
        if self.take.is_none() && matches!(state(), State::Idle | State::Hearing) {
            self.rest();
        }
    }

    /// The state between turns: waiting for the wake phrase, or idle.
    pub fn rest(&mut self) {
        set_state(&self.emit, if self.wake { State::Waiting } else { State::Idle });
    }

    /// A turn starts, or its next part.
    pub fn listen(&mut self, kind: ListenKind, held: bool, gen: u64) {
        let command = kind == ListenKind::Command;
        // What was just heard comes along with a command (the key may be pressed a word late). Not with
        // a reply: its first moment is the end of our own voice.
        let spoken = self.seg.is_some();
        let before = self.seg.take().map_or(std::mem::take(&mut self.ring), |seg| seg.audio);
        self.ring.clear();
        self.vad.rest();
        let mut take = Take::new(Turn::new(kind, held), held, gen, if command { before } else { Vec::new() });
        if command && spoken {
            (take.first, take.last) = (Some(0), take.audio.len());
        }
        if !command {
            take.skip = samples(FOLLOW_UP_MIC);
        }
        self.take = Some(take);
        set_state(&self.emit, if command { State::Listening } else { State::Asking });
    }

    /// The key says the turn is over: the held key went up, or it was tapped again.
    pub fn release(&mut self) {
        if let Some(take) = &mut self.take {
            take.turn.release();
        }
    }

    /// The capture path's entry: one poll of samples, from the microphone or from a file. Whoever calls
    /// it goes through `feed` below, which also asks the wake's sink.
    fn feed(&mut self, frame: &[f32]) -> Step {
        let dt = span(frame.len());
        if self.take.is_some() {
            self.record(frame, dt)
        } else if self.wake || state() == State::Speaking {
            // While it talks the detection runs whatever the wake's switch says: the sink keeps the one
            // word "stop" and nothing else (wake.rs).
            self.wait(frame, dt)
        } else {
            self.seg = None;
            Step::default()
        }
    }

    /// Waiting for the wake phrase: detection cuts segments, and long speech is looked at through a
    /// sliding window. Each goes to the wake's sink while the state is waiting, hearing or speaking.
    fn wait(&mut self, frame: &[f32], dt: Duration) -> Step {
        let vad = self.vad.feed(frame);
        let now = state();
        let mut step = Step { vad, ..Step::default() };
        if vad == VadEvent::Start {
            let ring = std::mem::take(&mut self.ring);
            self.seg = Some(Seg { frames: vec![(ring.len(), 0.0)], audio: ring, age: ZERO, window: WAKE_WINDOW_FIRST });
            if now == State::Waiting {
                set_state(&self.emit, State::Hearing);
            }
        }
        let Some(seg) = self.seg.as_mut() else {
            self.ring.extend_from_slice(frame);
            let over = self.ring.len().saturating_sub(samples(CAPTURE_RING));
            self.ring.drain(..over);
            return step;
        };
        seg.audio.extend_from_slice(frame);
        seg.frames.push((frame.len(), if self.vad.voiced() { self.vad.power() } else { 0.0 }));
        seg.age += dt;
        let asked = matches!(now, State::Waiting | State::Hearing | State::Speaking);
        if matches!(vad, VadEvent::End { .. }) {
            // Whatever its length the wake gets no more than the engine does quickly: the windows have
            // looked at the rest.
            let ended = seg.cut(STT_QUICK, self.vad.noise(), false);
            self.seg = None;
            if asked {
                step.segment = Some(ended);
            }
        } else if seg.age >= seg.window {
            seg.window += WAKE_WINDOW_EVERY;
            if asked {
                step.segment = Some(seg.cut(WAKE_WINDOW_SPAN, self.vad.noise(), true));
            }
        }
        step
    }

    /// What the wake's sink said of a segment: back to waiting, or on with the same buffer as a command.
    fn sunk(&mut self, segment: Segment, woke: bool) {
        if self.take.is_some() {
            return; // the key was faster
        }
        if !woke {
            if !segment.window && state() == State::Hearing {
                set_state(&self.emit, State::Waiting);
            }
            return;
        }
        let gen = mouth::halt();
        let quiet = if segment.window { ZERO } else { self.vad.segment_end() };
        self.seg = None;
        self.vad.rest();
        let mut take = Take::new(Turn::new(ListenKind::Command, false).heard(quiet), false, gen, segment.audio.samples);
        (take.woke, take.voiced, take.first) = (true, segment.voiced, Some(0));
        take.last = take.audio.len().saturating_sub(samples(quiet));
        self.take = Some(take);
        set_state(&self.emit, State::Listening);
        mouth::warm();
    }

    /// A turn: detection is bypassed, everything is kept, and the turn says when it is over.
    fn record(&mut self, frame: &[f32], dt: Duration) -> Step {
        self.vad.hear(frame);
        let voiced = self.vad.voiced();
        self.level += dt;
        let every = Duration::from_secs(1) / LEVEL_HZ;
        if self.level >= every {
            self.level = (self.level - every).min(every);
            (self.emit)(event::LEVEL, json!({ "mic": self.vad.level() }));
        }
        let Some(take) = self.take.as_mut() else { return Step::default() };
        if take.end.is_some() {
            return Step::default();
        }
        if take.skip > 0 {
            take.skip = take.skip.saturating_sub(frame.len());
            return Step::default();
        }
        take.audio.extend_from_slice(frame);
        if voiced {
            take.first.get_or_insert(take.audio.len() - frame.len());
            take.last = take.audio.len();
            take.voiced += dt;
        }
        take.since += dt;
        let turn = take.turn.step(voiced, dt);
        let job = match turn {
            // A partial about once a second while words come, as long as the engine is quick on it.
            TurnStep::Wait if voiced && take.since >= PARTIAL_EVERY && take.worth() && span(take.clip().1 - take.clip().0) <= STT_QUICK => take.ask(&mut self.jobs),
            TurnStep::Wait => None,
            TurnStep::Provisional if take.worth() && !take.fresh() => {
                let job = take.ask(&mut self.jobs);
                take.want = job.is_none();
                job
            }
            // A click, or nothing since the last transcript: there is nothing new to read.
            TurnStep::Provisional => {
                take.turn.extend(&take.text);
                None
            }
            TurnStep::End(end) => {
                take.end = Some(end);
                return Step { turn: Some(turn), job: self.conclude(), ..Step::default() };
            }
        };
        if voiced && state() == State::Asking {
            set_state(&self.emit, State::Listening);
        }
        Step { turn: Some(turn), job, ..Step::default() }
    }

    /// The engine's answer to a job: a partial for the page, words for the turn, or the final text.
    pub fn transcribed(&mut self, id: u64, heard: Result<String>, took: Duration) -> Option<Job> {
        let take = self.take.as_mut().filter(|take| take.job == Some(id))?;
        (take.job, take.seen) = (None, Some(take.asked));
        take.stt += took;
        take.runs += 1;
        match heard {
            Ok(text) => {
                let text = if take.woke { wakephrase::find(&text).map_or(text, |wake| wake.rest) } else { text };
                if text != take.text && take.end.is_none() {
                    (self.emit)(event::HEARD, json!(Transcript { text: text.clone(), is_final: false }));
                }
                (take.text, take.failed) = (text, None);
            }
            Err(e) => take.failed = Some(e),
        }
        if take.end.is_some() {
            return self.conclude();
        }
        if take.want && !take.fresh() {
            take.want = false;
            return take.ask(&mut self.jobs);
        }
        take.want = false;
        take.turn.extend(&take.text);
        None
    }

    /// The turn is over: its final text is the last transcript if that one covers everything, else one
    /// more run.
    fn conclude(&mut self) -> Option<Job> {
        let take = self.take.as_mut()?;
        let end = take.end?;
        if take.job.is_some() {
            return None; // its answer comes back here
        }
        // A key held to the end means words, whatever the detection made of them.
        let said = end != TurnEnd::NoSpeech && !take.audio.is_empty() && (take.worth() || take.held);
        if said && !take.fresh() && take.failed.is_none() {
            return take.ask(&mut self.jobs);
        }
        let mut text = if said { std::mem::take(&mut take.text) } else { String::new() };
        // What the engine writes on hum and clicks ("Thank you."), from a moment of sound: not a command.
        if take.voiced < Duration::from_millis(600) && wakephrase::stock(&text) {
            text.clear();
        }
        self.finish(text, end);
        None
    }

    fn finish(&mut self, text: String, end: TurnEnd) {
        let Some(take) = self.take.take() else { return };
        let ms = |d: Duration| d.as_millis() as u64;
        let (rule, audio, voiced, stt) = (ms(take.turn.rule(end)), ms(span(take.audio.len())), ms(take.voiced), ms(take.stt));
        let voice = take.first.map_or([0, 0], |first| [ms(span(first)), ms(span(take.last))]);
        // Numbers only (§8): never the words.
        notch_core::log::line(format!(
            "voice: turn end={end:?} rule={rule} audio={audio} voiced={voiced} stt={stt} runs={} words={}",
            take.runs,
            text.split_whitespace().count()
        ));
        (self.emit)(TURN, json!({ "gen": take.gen, "end": end, "rule": rule, "audio": audio, "voiced": voiced, "stt": stt, "runs": take.runs, "voice": voice }));
        if let Some(e) = &take.failed {
            (self.emit)(ERROR, json!({ "message": e.to_string() }));
        } else if !text.is_empty() {
            self.deaf = 0;
        } else if take.voiced >= DEAF_VOICED {
            // Voice and no words, again: the files are hashed afresh and the capture is opened anew.
            self.deaf += 1;
            let playing = svc_media::state().is_some_and(|media| media.playing);
            if self.deaf >= if playing { DEAF_RUNS_MEDIA } else { DEAF_RUNS } {
                (self.deaf, self.reopen) = (0, true);
                stt::forget();
                notch_core::log::line("voice: voice and no words: the engine is checked and the capture opened again");
            }
        }
        self.settle();
        (self.emit)(event::HEARD, json!(Transcript { text: text.clone(), is_final: true }));
        if text.is_empty() {
            self.close();
        } else {
            set_state(&self.emit, State::Thinking);
        }
    }

    /// After a turn: detection starts afresh on the floor it has, the meter goes to nothing.
    fn settle(&mut self) {
        self.vad.rest();
        self.ring.clear();
        (self.emit)(event::LEVEL, json!({ "mic": 0.0 }));
    }

    /// A turn cannot go on: its microphone is gone.
    fn fail(&mut self, error: VoiceError) {
        if self.take.take().is_none() {
            return;
        }
        self.settle();
        (self.emit)(ERROR, json!({ "message": error.to_string() }));
        (self.emit)(event::HEARD, json!(Transcript { text: String::new(), is_final: true }));
        self.close();
    }

    /// Closing, and to rest once the captions have had their time, unless a turn began meanwhile.
    fn close(&mut self) {
        set_state(&self.emit, State::Closing);
        // Whatever the generation is by then: a `mouth.stop` in between must not leave the state at closing.
        std::thread::spawn(move || {
            std::thread::sleep(CAPTIONS_STAY);
            let Some(core) = core() else { return };
            let mut capture = lock(&core);
            if state() == State::Closing && capture.take.is_none() {
                capture.rest();
            }
        });
    }
}

/// One poll through the path. The wake's sink is asked outside the path's lock: it runs the engine.
pub fn feed(core: &Mutex<Capture>, frame: &[f32]) -> Step {
    let mut step = lock(core).feed(frame);
    if let Some(segment) = step.segment.take() {
        let sink = lock(&SINK).clone();
        let woke = sink.is_some_and(|sink| sink(&segment));
        lock(core).sunk(segment, woke);
        step.woke = Some(woke);
    }
    step
}

/// Runs a job, and what its answer asks for next.
pub fn transcribe(core: &Mutex<Capture>, dir: &Path, job: Job) {
    let mut next = Some(job);
    while let Some(job) = next {
        let from = Instant::now();
        let heard = stt::transcribe(dir, &job.audio, &job.options);
        next = lock(core).transcribed(job.id, heard, from.elapsed());
    }
}

// ── 3. The microphone ─────────────────────────────────────────────────────────

/// Devices open now: one, or for a moment two (a turn that starts while the last one's device closes).
static MICS: AtomicU32 = AtomicU32::new(0);

/// The microphone counted as open for as long as this lives.
struct Open;

impl Open {
    fn new() -> Open {
        MICS.fetch_add(1, SeqCst);
        Open
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        MICS.fetch_sub(1, SeqCst);
    }
}
/// The capture gave up after `REBUILDS_PER_MINUTE`: `ears.status` says so, for the page's banner.
static STALLED: AtomicBool = AtomicBool::new(false);

unsafe fn id_of(device: &IMMDevice) -> windows::core::Result<String> {
    let id = unsafe { device.GetId() }?;
    let text = unsafe { id.to_string() }.unwrap_or_default();
    unsafe { CoTaskMemFree(Some(id.0 as _)) };
    Ok(text)
}

/// The capture thread: the default microphone, opened again when it changes, stalls or fails, until
/// nobody wants it. `id` is its claim on the path: another thread's id there, or none, ends it.
fn microphone(core: Arc<Mutex<Capture>>, dir: PathBuf, id: u64) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    // A device came or went, or another one is the default: looked at on this thread, at its next poll.
    let changed = Arc::new(AtomicBool::new(false));
    let flag = changed.clone();
    let _devices = svc_audio::on_endpoint_change(move |change| {
        if matches!(change, svc_audio::Change::Devices) {
            flag.store(true, SeqCst);
        }
    });
    let mut opened: Vec<Instant> = Vec::new();
    loop {
        let again = unsafe { record(&core, &dir, id, &changed) };
        let mut capture = lock(&core);
        if capture.mic != id {
            return;
        }
        match again {
            Ok(()) => notch_core::log::line("voice: the capture is opened again"),
            Err(e) => {
                notch_core::log::line(format!("voice: the microphone failed: 0x{:08X}", e.code().0 as u32));
                capture.fail(VoiceError::Failed(format!("the microphone could not be used ({})", e.message())));
                if !capture.wake {
                    capture.mic = 0;
                    return;
                }
            }
        }
        // Stop, a pause, start; never closer than `REBUILD_APART`, and after too many in a minute it gives up.
        let now = Instant::now();
        opened.retain(|at| now.duration_since(*at) < Duration::from_secs(60));
        if opened.len() as u32 >= REBUILDS_PER_MINUTE {
            notch_core::log::line("voice: the capture keeps failing: given up until voice is switched off and on");
            STALLED.store(true, SeqCst);
            capture.fail(VoiceError::Failed("the microphone keeps failing".into()));
            capture.mic = 0;
            capture.waiting(false);
            return;
        }
        let pause = opened.last().map_or(REBUILD_PAUSE, |last| REBUILD_APART.saturating_sub(now.duration_since(*last)).max(REBUILD_PAUSE));
        opened.push(now + pause);
        drop(capture);
        std::thread::sleep(pause);
    }
}

/// One opening of the default microphone. It comes back to be opened again (another default device, a
/// stall, a deaf engine), or with its claim on the path given up: nobody wants the microphone any more.
unsafe fn record(core: &Arc<Mutex<Capture>>, dir: &Path, id: u64, changed: &AtomicBool) -> windows::core::Result<()> {
    unsafe {
        let devices: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device = devices.GetDefaultAudioEndpoint(eCapture, eConsole)?;
        let name = id_of(&device)?;
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
        // Windows converts to what the engine takes: no resampling of ours.
        let format = WAVEFORMATEX {
            wFormatTag: 3, // WAVE_FORMAT_IEEE_FLOAT
            nChannels: 1,
            nSamplesPerSec: CAPTURE_RATE,
            nAvgBytesPerSec: CAPTURE_RATE * 4,
            nBlockAlign: 4,
            wBitsPerSample: 32,
            cbSize: 0,
        };
        client.Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, 50_000_000, 0, &format, None)?;
        let packets: IAudioCaptureClient = client.GetService()?;
        client.Start()?;
        let _open = Open::new();
        changed.store(false, SeqCst);
        lock(core).opened();

        let poll = samples(CAPTURE_POLL);
        let (mut pending, mut last) = (Vec::<f32>::new(), Instant::now());
        loop {
            std::thread::sleep(CAPTURE_POLL);
            while packets.GetNextPacketSize()? > 0 {
                let (mut data, mut frames, mut flags) = (std::ptr::null_mut(), 0u32, 0u32);
                packets.GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
                if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null() {
                    pending.resize(pending.len() + frames as usize, 0.0);
                } else {
                    pending.extend_from_slice(std::slice::from_raw_parts(data as *const f32, frames as usize));
                }
                packets.ReleaseBuffer(frames)?;
                last = Instant::now();
            }
            // In frames of one poll's length whatever the device hands over, so that a wait (the wake's
            // sink runs the engine on this thread) is caught up frame by frame.
            let mut fed = 0;
            while pending.len() - fed >= poll {
                let step = feed(core, &pending[fed..fed + poll]);
                fed += poll;
                if let Some(job) = step.job {
                    let (core, dir) = (core.clone(), dir.to_path_buf());
                    std::thread::spawn(move || transcribe(&core, &dir, job));
                }
            }
            pending.drain(..fed);
            {
                let mut capture = lock(core);
                if capture.mic != id {
                    break;
                }
                if !capture.wake && capture.take.is_none() && state() != State::Speaking {
                    // Given up under the lock a turn starts under: the next one opens a thread of its own.
                    capture.mic = 0;
                    break;
                }
                if std::mem::take(&mut capture.reopen) {
                    break;
                }
            }
            if last.elapsed() >= STALL {
                break;
            }
            if changed.swap(false, SeqCst) && devices.GetDefaultAudioEndpoint(eCapture, eConsole).and_then(|now| id_of(&now)).map_or(true, |now| now != name) {
                break;
            }
        }
        let _ = client.Stop();
        Ok(())
    }
}

// ── 4. The module ─────────────────────────────────────────────────────────────

/// The engine and the model together, as `whistle::FILES` pins them: the total of `ears.download`.
const SPEECH_BYTES: u64 = 18_488_687;

struct Ears {
    emit: Emit,
    /// The dictation feature's folder: the engine and the speech model.
    dir: PathBuf,
    core: Arc<Mutex<Capture>>,
    /// The talk key's hook and its thread, and the settings they were made for.
    keys: Option<(svc_input::Sub, mpsc::Sender<KeyEvent>, Value)>,
    mics: u64,
}

static EARS: Mutex<Option<Ears>> = Mutex::new(None);
static SINK: Mutex<Option<SegmentSink>> = Mutex::new(None);

fn core() -> Option<Arc<Mutex<Capture>>> {
    lock(&EARS).as_ref().map(|ears| ears.core.clone())
}

/// A capture thread for the path, if it has none.
fn open(ears: &mut Ears, capture: &mut Capture) {
    if capture.mic != 0 {
        return;
    }
    ears.mics += 1;
    capture.mic = ears.mics;
    STALLED.store(false, SeqCst);
    let (core, dir, id) = (ears.core.clone(), ears.dir.clone(), ears.mics);
    #[cfg(debug_assertions)]
    if std::env::var_os(CLIPS_ENV).is_some() {
        std::thread::spawn(move || pretend(core, dir, id));
        return;
    }
    std::thread::spawn(move || microphone(core, dir, id));
}

/// Debug builds only, and only with this variable set: recorded clips stand in for the microphone, which
/// is then never opened. `ears.feed {path}` queues a 16 kHz mono 16-bit file; between clips it is silent.
#[cfg(debug_assertions)]
const CLIPS_ENV: &str = "METEOR_VOICE_CLIPS";
#[cfg(debug_assertions)]
static CLIPS: Mutex<std::collections::VecDeque<f32>> = Mutex::new(std::collections::VecDeque::new());

/// The capture thread without a device: the loop of `record`, a poll of the queue every 30 ms of the clock.
#[cfg(debug_assertions)]
fn pretend(core: Arc<Mutex<Capture>>, dir: PathBuf, id: u64) {
    let _open = Open::new();
    lock(&core).opened();
    let poll = samples(CAPTURE_POLL);
    let mut next = Instant::now();
    loop {
        next += CAPTURE_POLL;
        std::thread::sleep(next.saturating_duration_since(Instant::now()));
        let frame: Vec<f32> = {
            let mut clips = lock(&CLIPS);
            (0..poll).map(|_| clips.pop_front().unwrap_or(0.0)).collect()
        };
        if let Some(job) = feed(&core, &frame).job {
            let (core, dir) = (core.clone(), dir.clone());
            std::thread::spawn(move || transcribe(&core, &dir, job));
        }
        let mut capture = lock(&core);
        if capture.mic != id {
            return;
        }
        if !capture.wake && capture.take.is_none() && state() != State::Speaking {
            capture.mic = 0;
            return;
        }
        capture.reopen = false;
    }
}

/// `ears.feed {path}`: a clip for the pretend microphone, played from now (what was queued is dropped).
#[cfg(debug_assertions)]
fn feed_clip(path: &str) -> Result<Value> {
    if std::env::var_os(CLIPS_ENV).is_none() {
        return Err(VoiceError::Refused(format!("{CLIPS_ENV} is not set")));
    }
    let bytes = std::fs::read(path).map_err(|e| VoiceError::Failed(e.to_string()))?;
    let (mut at, mut clip) = (12, Vec::new());
    while at + 8 <= bytes.len() {
        let size = u32::from_le_bytes([bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]]) as usize;
        let body = &bytes[at + 8..(at + 8 + size).min(bytes.len())];
        if &bytes[at..at + 4] == b"fmt " && (body[2] != 1 || u32::from_le_bytes([body[4], body[5], body[6], body[7]]) != CAPTURE_RATE || body[14] != 16) {
            return Err(VoiceError::Failed("not 16 kHz mono 16-bit".into()));
        }
        if &bytes[at..at + 4] == b"data" {
            clip = body.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0).collect();
        }
        at += 8 + size + size % 2;
    }
    let ms = span(clip.len()).as_millis() as u64;
    *lock(&CLIPS) = clip.into();
    Ok(json!({ "ms": ms }))
}

/// Its own speech began (`mouth`): the microphone opens for the one word "stop", if it can be heard at
/// all. It closes at the capture's next poll once the state has left speaking and nothing else wants it.
pub fn while_speaking() {
    let mut guard = lock(&EARS);
    let Some(ears) = guard.as_mut() else { return };
    if !stt::ready(&ears.dir) {
        return;
    }
    let core = ears.core.clone();
    let mut capture = lock(&core);
    open(ears, &mut capture);
}

/// A turn is being recorded.
fn recording() -> bool {
    core().is_some_and(|core| lock(&core).take.as_ref().is_some_and(|take| take.end.is_none()))
}

fn release() {
    if let Some(core) = core() {
        lock(&core).release();
    }
}

fn begin(kind: ListenKind, held: bool) -> Result<u64> {
    let mut guard = lock(&EARS);
    let ears = guard.as_mut().ok_or_else(|| VoiceError::Refused("voice is off".into()))?;
    if !stt::ready(&ears.dir) {
        return Err(VoiceError::Missing("the speech model".into()));
    }
    let gen = mouth::halt();
    let core = ears.core.clone();
    let mut capture = lock(&core);
    capture.listen(kind, held, gen);
    open(ears, &mut capture);
    if kind == ListenKind::Command {
        mouth::warm();
    }
    Ok(gen)
}

/// Esc, the capsule's stop button, `ears.stop`: the turn and what is being said end here.
fn stop_turn() {
    let Some(core) = core() else { return };
    let recording = lock(&core).take.take().is_some();
    // Speech outside a turn too (a chat answer read aloud, the picker's Test).
    if !recording && !mouth::speaking() && !matches!(state(), State::Listening | State::Thinking | State::Speaking | State::Asking) {
        return;
    }
    mouth::halt();
    let mut capture = lock(&core);
    capture.settle();
    capture.close();
}

/// Hooks the talk key (`svc-input`; the setting `key`, Right Ctrl alone by default: a tap starts a turn,
/// a hold talks until release) and opens the microphone when `wake::wanted`. Idempotent; `crate::resume`
/// calls it again, and so may `wake` whenever what `wanted` answers has changed.
pub fn start(cx: &Ctx) -> Result<()> {
    {
        let mut ears = lock(&EARS);
        if ears.is_none() {
            let emit = crate::emitter(cx);
            *ears = Some(Ears { core: Arc::new(Mutex::new(Capture::new(emit.clone()))), emit, dir: cx.data_dir("dictation"), keys: None, mics: 0 });
        }
    }
    settings(cx, &cx.settings(crate::ID));
    Ok(())
}

/// Closes the microphone, drops the turn and the key hook. `crate::pause` calls it too.
pub fn stop(_cx: &Ctx) {
    let Some(ears) = lock(&EARS).take() else { return };
    if let Some((_, keys, _)) = &ears.keys {
        let _ = keys.send(KeyEvent::Quit);
    }
    // The capture thread sees its claim gone at its next poll and closes the device.
    let mut capture = lock(&ears.core);
    (capture.mic, capture.wake, capture.take, capture.seg) = (0, false, None, None);
}

pub fn settings(cx: &Ctx, settings: &Value) {
    let mut guard = lock(&EARS);
    let Some(ears) = guard.as_mut() else { return };
    let made_for = json!([settings["key"], settings["hold"], settings["esc"]]);
    if ears.keys.as_ref().map(|keys| &keys.2) != Some(&made_for) {
        if let Some((_, keys, _)) = ears.keys.take() {
            let _ = keys.send(KeyEvent::Quit);
        }
        let vk = settings["key"].as_str().and_then(vk_of);
        let esc = settings["esc"].as_bool().unwrap_or(true);
        if vk.is_some() || esc {
            let (sub, keys) = hook(vk, settings["hold"].as_bool().unwrap_or(true), esc);
            ears.keys = Some((sub, keys, made_for));
        }
    }
    let wanted = wake::wanted(cx);
    let core = ears.core.clone();
    let mut capture = lock(&core);
    capture.waiting(wanted);
    if wanted {
        open(ears, &mut capture);
    }
}

/// `listen {kind?: ListenKind}` → `{gen}` · `stop` → `{}` (the turn ends, the state goes to closing and
/// then to rest) · `status` → `{mic: bool, model: bool, stalled: bool}` · `shortcut {id}` (the host's
/// chord `talk`: as a tap) · `download` → `{}` (the engine and the speech model, 19 MB; progress as
/// `voice:download {what: "whistle"}`).
pub fn call(cx: &Ctx, method: &str, args: Value) -> Result<Value> {
    match method {
        "listen" => {
            let kind = match args.get("kind") {
                None | Some(Value::Null) => ListenKind::default(),
                Some(kind) => serde_json::from_value(kind.clone()).map_err(|_| VoiceError::Failed("unknown kind".into()))?,
            };
            listen(cx, kind).map(|gen| json!({ "gen": gen }))
        }
        "stop" => {
            end_turn(cx);
            Ok(json!({}))
        }
        "status" => Ok(json!({ "mic": mic_open(), "model": stt::ready(&cx.data_dir("dictation")), "stalled": STALLED.load(SeqCst) })),
        "shortcut" => {
            talk(Talk::Tap);
            Ok(json!({}))
        }
        "download" => {
            download(crate::emitter(cx), cx.data_dir("dictation"));
            Ok(json!({}))
        }
        #[cfg(debug_assertions)]
        "feed" => feed_clip(args["path"].as_str().unwrap_or("")),
        _ => Err(VoiceError::Failed("unknown method".into())),
    }
}

/// The engine and the speech model, for a user who has not switched dictation on.
fn download(emit: Emit, dir: PathBuf) {
    static BUSY: AtomicBool = AtomicBool::new(false);
    if BUSY.swap(true, SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        let tell = |received, done, error| emit(event::DOWNLOAD, json!(Download { what: "whistle".into(), received, total: SPEECH_BYTES, done, error }));
        // Two files, one count; told every quarter megabyte.
        let (mut before, mut file, mut told) = (0, 0, 0);
        let fetched = tauri::async_runtime::block_on(whistle::download_with(&dir, |_, received, _| {
            if received < file {
                before += file;
            }
            file = received;
            if before + file >= told + (1 << 18) || received == 0 {
                told = before + file;
                tell(told, false, None);
            }
        }));
        stt::forget();
        BUSY.store(false, SeqCst);
        match fetched {
            Ok(()) => tell(SPEECH_BYTES, true, None),
            Err(e) => tell(0, true, Some(e)),
        }
    });
}

/// Starts a turn, or the next part of one (a reply): opens the microphone if it is closed, interrupts
/// what is being said (`mouth::halt`) and returns the generation. What `ears.listen` does.
pub fn listen(_cx: &Ctx, kind: ListenKind) -> Result<u64> {
    begin(kind, false)
}

/// Ends the turn: what `ears.stop` does, and what Esc does.
pub fn end_turn(_cx: &Ctx) {
    stop_turn();
}

/// True while this process holds the microphone open (the indicator rule of §8).
pub fn mic_open() -> bool {
    MICS.load(SeqCst) > 0
}

/// `wake`'s tap into the capture: called on the capture thread with every voice segment, and with the
/// sliding window of long speech, while the state is waiting, hearing or speaking. `true` means "that was
/// the wake phrase": ears goes on with the same buffer as a command (`ListenKind::Command`), and the
/// final transcript loses everything up to the phrase (`wakephrase::find`).
///
/// A segment is never longer than `STT_QUICK` and a window is `WAKE_WINDOW_SPAN`; the sink may run the
/// engine and block: the capture catches up afterwards.
pub type SegmentSink = Arc<dyn Fn(&Segment) -> bool + Send + Sync>;

/// One sink at most; `None` takes it away.
pub fn on_segment(sink: Option<SegmentSink>) {
    *lock(&SINK) = sink;
}
