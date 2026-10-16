// The player: one WASAPI shared render stream on the default output, the mirror of dictation/capture.rs
// (not `MediaPlayer`, which would register a media session). Filled by package voice-mouth-win
// (VOICE §7.1, §7.3, §7.4). Its thread lives only while a chunk plays (plus a short linger so a sentence
// that is a moment late does not reopen the device).
//
// Chunks play in `seq` order within a generation, `timing::PLAYER_GAP` apart. Each voice has its own
// sample rate: the stream is opened at the rate of the first chunk and later ones are converted.
//
// Split in two so it can be proved without a loudspeaker: `Core` is the queue, the order, the gap, the
// fade and the level, and turns "what was queued" into "the next block of samples" (`fill`); `Out` is
// where blocks go. `Wasapi` is the product's `Out`; `set_output` swaps in another (the probe's file).

use crate::{timing, AudioChunk, Result, VoiceError};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

/// Where blocks of the stream's samples go. `wait` blocks until the output wants samples and says how
/// many frames; `write` hands them over. Dropping it closes the stream.
pub trait Out: Send {
    fn wait(&mut self) -> usize;
    fn write(&mut self, samples: &[f32]);
}

/// Opens an output at a rate (the first chunk's).
pub type Opener = Arc<dyn Fn(u32) -> std::result::Result<Box<dyn Out>, String> + Send + Sync>;

/// Called on the player's thread as a `seq` starts (→ `voice:spoke`) and, with `None`, when the queue
/// has run dry (or was stopped).
pub type OnPlay = Arc<dyn Fn(Option<(u64, u32)>) + Send + Sync>;

type Event = Option<(u64, u32)>;

/// Silence kept going after the queue is dry, before the stream closes.
const LINGER: Duration = Duration::from_millis(300);
/// Output level: RMS times this, clamped (speech RMS is about 0.1 to 0.2).
const LEVEL_GAIN: f32 = 4.0;

#[derive(Default)]
struct Entry {
    buf: Vec<f32>,
    pos: usize,
    done: bool,
}

#[derive(Default)]
struct Core {
    /// The stream's rate; 0 while no stream is open.
    rate: u32,
    gen: u64,
    next_seq: u32,
    queue: BTreeMap<u32, Entry>,
    cur: Option<u32>,
    /// The entry that was playing when `stop` came, and what is left of its fade.
    dying: Option<Entry>,
    fade_left: usize,
    gap: usize,
    /// Silent samples since the last sound.
    idle: usize,
    /// Something was queued and the dry event has not been sent.
    active: bool,
    sounded: bool,
    level: f32,
}

impl Core {
    fn push(&mut self, gen: u64, seq: u32, audio: AudioChunk, last: bool) {
        if gen < self.gen {
            return;
        }
        if gen > self.gen {
            self.gen = gen;
            self.next_seq = 0;
            // A newer generation than the one playing: what plays now is fading out via `stop`, which the
            // caller (mouth) has already asked for; anything still queued is older.
            self.queue.clear();
            self.cur = None;
        }
        if !self.active && seq == 0 {
            // A new turn in the same generation starts again from 0.
            self.next_seq = 0;
        }
        if seq < self.next_seq {
            return;
        }
        let rate = self.rate;
        let e = self.queue.entry(seq).or_default();
        if e.done {
            return;
        }
        e.buf.extend(resample(&audio.samples, audio.rate, rate));
        e.done = last;
        self.active = true;
        self.idle = 0;
    }

    /// Fades the current chunk in `PLAYER_FADE` and empties the queue. Returns events to fire.
    fn stop(&mut self) -> Vec<Event> {
        self.dying = self.cur.take().and_then(|s| self.queue.remove(&s));
        self.queue.clear();
        self.next_seq = 0;
        self.gap = 0;
        let mut ev = Vec::new();
        if self.dying.as_ref().is_some_and(|e| e.pos < e.buf.len()) {
            self.fade_left = self.fade_len();
        } else {
            self.dying = None;
            if std::mem::take(&mut self.active) {
                ev.push(None);
            }
        }
        ev
    }

    /// The fade is `PLAYER_FADE` minus one output block, so that with the block that may pass before the
    /// stop is seen the sound is gone within `PLAYER_FADE`.
    fn fade_len(&self) -> usize {
        let ms = timing::PLAYER_FADE.as_millis() as usize;
        self.rate as usize * ms.saturating_sub(BLOCK_MS) / 1000
    }

    fn take(entry: &mut Entry) -> Option<f32> {
        let s = entry.buf.get(entry.pos).copied();
        if s.is_some() {
            entry.pos += 1;
            if entry.pos >= 1 << 16 {
                entry.buf.drain(..entry.pos);
                entry.pos = 0;
            }
        }
        s
    }

    fn next(&mut self, ev: &mut Vec<Event>) -> f32 {
        if self.fade_left > 0 {
            let total = self.fade_len().max(1);
            let taken = self.dying.as_mut().and_then(Core::take);
            let s = taken.unwrap_or(0.0);
            let g = self.fade_left as f32 / total as f32;
            self.fade_left -= 1;
            if self.fade_left == 0 || taken.is_none() {
                self.fade_left = 0;
                self.dying = None;
                if std::mem::take(&mut self.active) {
                    ev.push(None);
                }
            }
            self.sounded = true;
            return s * g;
        }
        if self.gap > 0 {
            self.gap -= 1;
            return 0.0;
        }
        if self.cur.is_none() {
            while let Some(e) = self.queue.get(&self.next_seq) {
                if e.buf.is_empty() && e.done {
                    // A chunk its voice could not make: skipped without a sound or an event.
                    self.queue.remove(&self.next_seq);
                    self.next_seq += 1;
                } else {
                    if !e.buf.is_empty() {
                        self.cur = Some(self.next_seq);
                        ev.push(Some((self.gen, self.next_seq)));
                    }
                    break;
                }
            }
            if self.cur.is_none() {
                if self.queue.is_empty() && std::mem::take(&mut self.active) {
                    ev.push(None);
                }
                self.idle += 1;
                return 0.0;
            }
        }
        let seq = self.cur.unwrap();
        let e = self.queue.get_mut(&seq).unwrap();
        match Core::take(e) {
            Some(s) => {
                self.idle = 0;
                self.sounded = true;
                s
            }
            None => {
                if e.done {
                    self.queue.remove(&seq);
                    self.cur = None;
                    self.next_seq = seq + 1;
                    if self.queue.is_empty() {
                        if std::mem::take(&mut self.active) {
                            ev.push(None);
                        }
                    } else {
                        self.gap = self.rate as usize * timing::PLAYER_GAP.as_millis() as usize / 1000;
                    }
                }
                self.idle += 1; // an unfinished chunk waiting for its next piece is silence too
                0.0
            }
        }
    }

    fn fill(&mut self, out: &mut [f32]) -> Vec<Event> {
        let mut ev = Vec::new();
        self.sounded = false;
        for o in out.iter_mut() {
            *o = self.next(&mut ev);
        }
        let rms = (out.iter().map(|s| s * s).sum::<f32>() / out.len().max(1) as f32).sqrt();
        let now = (rms * LEVEL_GAIN).min(1.0);
        self.level = if now > self.level { now } else { self.level * 0.85 };
        ev
    }

    /// Nothing queued, nothing playing, quiet for `LINGER`: the stream may close.
    fn finished(&self) -> bool {
        self.queue.is_empty()
            && self.cur.is_none()
            && self.dying.is_none()
            && self.idle as u128 * 1000 > LINGER.as_millis() * self.rate.max(1) as u128
    }
}

/// Output block: how often the thread feeds the device (the file output of the probe too).
pub const BLOCK_MS: usize = 5;

static CORE: Mutex<Option<Core>> = Mutex::new(None);
static RUNNING: AtomicBool = AtomicBool::new(false);
static PLAYING: AtomicBool = AtomicBool::new(false);
static LEVEL: AtomicU32 = AtomicU32::new(0);
static ON_PLAY: Mutex<Option<OnPlay>> = Mutex::new(None);
static OPENER: Mutex<Option<Opener>> = Mutex::new(None);

fn core<T>(f: impl FnOnce(&mut Core) -> T) -> T {
    f(CORE.lock().unwrap().get_or_insert_with(Core::default))
}

fn fire(events: Vec<Event>) {
    if events.is_empty() {
        return;
    }
    let cb = ON_PLAY.lock().unwrap().clone();
    if let Some(cb) = cb {
        for e in events {
            cb(e);
        }
    }
}

/// Linear interpolation; enough for speech, and the identity when the rates agree.
fn resample(x: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || from == 0 || to == 0 || x.is_empty() {
        return x.to_vec();
    }
    // ponytail: linear, no low-pass; Kokoro 24 kHz and Windows 22.05 kHz to 48 kHz are fine, upgrade to a windowed sinc if it is heard.
    let n = (x.len() as u64 * to as u64 / from as u64) as usize;
    let step = from as f64 / to as f64;
    (0..n)
        .map(|i| {
            let p = i as f64 * step;
            let k = p as usize;
            let f = (p - k as f64) as f32;
            let a = x[k.min(x.len() - 1)];
            let b = x[(k + 1).min(x.len() - 1)];
            a + (b - a) * f
        })
        .collect()
}

/// Queues one synthesised chunk; a sentence that came in pieces is queued piece by piece under one `seq`,
/// the last with `last`. A `gen` older than `crate::generation()` is dropped. An empty chunk with `last`
/// stands for a sentence that could not be made: it is skipped.
pub fn play(gen: u64, seq: u32, audio: AudioChunk, last: bool) -> Result<()> {
    if gen < crate::generation() || audio.rate == 0 {
        return Ok(());
    }
    let spawn = {
        let mut g = CORE.lock().unwrap();
        let c = g.get_or_insert_with(Core::default);
        let spawn = !RUNNING.swap(true, Ordering::SeqCst);
        if spawn {
            *c = Core { gen: c.gen, rate: audio.rate, ..Core::default() };
        }
        c.push(gen, seq, audio, last);
        spawn
    };
    if spawn {
        std::thread::Builder::new()
            .name("voice-player".into())
            .spawn(run)
            .map_err(|e| VoiceError::Failed(e.to_string()))?;
    }
    Ok(())
}

fn run() {
    let rate = core(|c| c.rate);
    let opener = OPENER.lock().unwrap().clone();
    let opened = match opener {
        Some(o) => o(rate),
        None => Wasapi::open(rate, true).map(|w| Box::new(w) as Box<dyn Out>),
    };
    let mut out = match opened {
        Ok(o) => o,
        Err(e) => {
            notch_core::log::line(format!("voice: output did not open: {e}"));
            let ev = {
                let mut g = CORE.lock().unwrap();
                let c = g.get_or_insert_with(Core::default);
                let ev = c.stop();
                RUNNING.store(false, Ordering::SeqCst);
                ev
            };
            fire(ev);
            return;
        }
    };
    let mut buf: Vec<f32> = Vec::new();
    loop {
        let n = out.wait();
        buf.resize(n, 0.0);
        let (ev, end) = {
            let mut g = CORE.lock().unwrap();
            let c = g.get_or_insert_with(Core::default);
            let ev = c.fill(&mut buf);
            LEVEL.store(c.level.to_bits(), Ordering::Relaxed);
            PLAYING.store(c.sounded, Ordering::Relaxed);
            let end = c.finished();
            if end {
                RUNNING.store(false, Ordering::SeqCst);
                PLAYING.store(false, Ordering::Relaxed);
                LEVEL.store(0, Ordering::Relaxed);
            }
            (ev, end)
        };
        out.write(&buf);
        fire(ev);
        if end {
            break;
        }
    }
}

/// Fades in `timing::PLAYER_FADE` and empties the queue.
pub fn stop() {
    let ev = core(|c| c.stop());
    fire(ev);
}

/// Sound is leaving the stream right now: the exact "is speaking".
pub fn playing() -> bool {
    PLAYING.load(Ordering::Relaxed)
}

/// How many `seq` are queued and not yet started (the queue synthesises `timing::SYNTH_AHEAD` ahead).
pub fn queued() -> usize {
    core(|c| c.queue.keys().filter(|s| Some(**s) != c.cur).count())
}

/// The level of what is being played, 0 to 1 (`voice:level {out}`, the talking face).
pub fn level() -> f32 {
    f32::from_bits(LEVEL.load(Ordering::Relaxed))
}

/// The player's thread (and so the stream) is alive.
pub fn running() -> bool {
    RUNNING.load(Ordering::SeqCst)
}

pub fn on_play(f: Option<OnPlay>) {
    *ON_PLAY.lock().unwrap() = f;
}

/// Replaces the output device by another `Out` (the probe's file); `None` is the default device again.
pub fn set_output(opener: Option<Opener>) {
    *OPENER.lock().unwrap() = opener;
}

/// Opens the default output at `rate` without starting it (nothing is heard) and says how many frames its
/// buffer holds: proves the format is accepted.
pub fn probe_device(rate: u32) -> std::result::Result<u32, String> {
    Wasapi::open(rate, false).map(|w| w.buffer_frames)
}

// ── The product's output: WASAPI shared mode, event driven ────────────────────

struct Wasapi {
    ac: IAudioClient,
    render: IAudioRenderClient,
    event: windows::Win32::Foundation::HANDLE,
    buffer_frames: u32,
}

// The COM pointers are created on, and used by, the player's thread only.
unsafe impl Send for Wasapi {}

impl Wasapi {
    fn open(rate: u32, start: bool) -> std::result::Result<Wasapi, String> {
        let e = |e: windows::core::Error| format!("0x{:08X} {}", e.code().0 as u32, e.message());
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(e)?;
            let dev = en.GetDefaultAudioEndpoint(eRender, eConsole).map_err(e)?;
            let ac: IAudioClient = dev.Activate(CLSCTX_ALL, None).map_err(e)?;
            let fmt = WAVEFORMATEX {
                wFormatTag: 3, // WAVE_FORMAT_IEEE_FLOAT
                nChannels: 1,
                nSamplesPerSec: rate,
                nAvgBytesPerSec: rate * 4,
                nBlockAlign: 4,
                wBitsPerSample: 32,
                cbSize: 0,
            };
            // Windows converts rate and channels itself (AUTOCONVERTPCM), as capture.rs does.
            ac.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_EVENTCALLBACK | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                0,
                0,
                &fmt,
                None,
            )
            .map_err(e)?;
            let event = CreateEventW(None, false, false, None).map_err(e)?;
            ac.SetEventHandle(event).map_err(e)?;
            let buffer_frames = ac.GetBufferSize().map_err(e)?;
            let render: IAudioRenderClient = ac.GetService().map_err(e)?;
            // Debug builds only, with `METEOR_VOICE_MUTE` set: this app's own audio session (its row in the
            // volume mixer, never the device) is muted before the first sample; the drop puts it back.
            #[cfg(debug_assertions)]
            if std::env::var_os("METEOR_VOICE_MUTE").is_some() {
                let volume: ISimpleAudioVolume = ac.GetService().map_err(e)?;
                volume.SetMute(true, std::ptr::null()).map_err(e)?;
                if !volume.GetMute().map_err(e)?.as_bool() {
                    return Err("the audio session did not mute".into());
                }
                notch_core::log::line("voice: debug: this app's audio session is muted");
            }
            if start {
                ac.Start().map_err(e)?;
            }
            Ok(Wasapi { ac, render, event, buffer_frames })
        }
    }
}

impl Out for Wasapi {
    fn wait(&mut self) -> usize {
        unsafe {
            loop {
                WaitForSingleObject(self.event, 200);
                match self.ac.GetCurrentPadding() {
                    Ok(p) if self.buffer_frames > p => return (self.buffer_frames - p) as usize,
                    Ok(_) => {}
                    Err(_) => return 0,
                }
            }
        }
    }

    fn write(&mut self, samples: &[f32]) {
        unsafe {
            if samples.is_empty() {
                return;
            }
            if let Ok(p) = self.render.GetBuffer(samples.len() as u32) {
                std::ptr::copy_nonoverlapping(samples.as_ptr(), p as *mut f32, samples.len());
                let _ = self.render.ReleaseBuffer(samples.len() as u32, 0);
            }
        }
    }
}

impl Drop for Wasapi {
    fn drop(&mut self) {
        unsafe {
            let _ = self.ac.Stop();
            #[cfg(debug_assertions)]
            if std::env::var_os("METEOR_VOICE_MUTE").is_some() {
                if let Ok(volume) = self.ac.GetService::<ISimpleAudioVolume>() {
                    let _ = volume.SetMute(false, std::ptr::null());
                }
            }
            let _ = windows::Win32::Foundation::CloseHandle(self.event);
        }
    }
}
