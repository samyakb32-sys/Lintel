// Proves the natural voice (crates/voice/src/speech.rs and meteor-speak.exe) without the app and without
// a sound: everything it makes goes into a file, nothing to the speakers.
//
//   CARGO_BUILD_JOBS=4 cargo build -p meteor-speak --release
//   CARGO_BUILD_JOBS=4 cargo run -p notch-feat-voice --example voice-kokoro-probe
//
// The probe starts itself twice. The first copy only watches. The second downloads the model into
// target/voice-dev/kokoro/ if it is not there (the address and SHA-256 the product uses), starts the
// server through speech.rs (so through `proc::spawn`), makes three sentences back to back into
// target/voice-dev/kokoro/probe.wav, measures, and exits without ending the server. The watcher then
// sees the server die with its parent: the job's kill-on-close.

use notch_feat_voice::{speech, AudioChunk, SynthRequest, VoiceError};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{FILETIME, HANDLE};
use windows::Win32::System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

/// A first chunk as the splitter cuts it (VOICE §7.3), then two whole sentences.
const SENTENCES: [&str; 3] = [
    "Sure, here is what I found.",
    "The meeting with the design team moved to half past three this afternoon, in the small room on the second floor.",
    "I also set a reminder ten minutes before it starts.",
];
const VOICE: &str = "af_heart";
/// `timing::PLAYER_GAP`.
const GAP: f64 = 0.150;

fn fail(what: &str) -> ! {
    println!("FAIL {what}");
    std::process::exit(1)
}

fn running(pid: u32) -> bool {
    notch_core::win::process_table().contains_key(&pid)
}

fn gone_within(pid: u32, limit: Duration) -> Option<Duration> {
    let from = Instant::now();
    while from.elapsed() < limit {
        if !running(pid) {
            return Some(from.elapsed());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    None
}

// PROCESS_MEMORY_COUNTERS_EX, declared here: the crate does not ask `windows` for the process-status set.
#[repr(C)]
#[derive(Default)]
struct Counters {
    cb: u32,
    page_faults: u32,
    peak_working_set: usize,
    working_set: usize,
    quota: [usize; 4],
    pagefile: usize,
    peak_pagefile: usize,
    private: usize,
}

#[link(name = "kernel32")]
extern "system" {
    fn K32GetProcessMemoryInfo(process: HANDLE, counters: *mut Counters, cb: u32) -> i32;
}

fn memory(process: HANDLE) -> Counters {
    let mut counters = Counters { cb: std::mem::size_of::<Counters>() as u32, ..Default::default() };
    unsafe { K32GetProcessMemoryInfo(process, &mut counters, counters.cb) };
    counters
}

/// Processor time the process has used so far, kernel and user.
fn cpu(process: HANDLE) -> Duration {
    let mut times = [FILETIME::default(); 4];
    let [created, exited, kernel, user] = &mut times;
    if unsafe { GetProcessTimes(process, created, exited, kernel, user) }.is_err() {
        fail("GetProcessTimes");
    }
    let ticks = |t: FILETIME| ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64;
    Duration::from_nanos((ticks(times[2]) + ticks(times[3])) * 100)
}

fn mb(bytes: usize) -> f64 {
    bytes as f64 / 1048576.0
}

struct Spoken {
    first: f64,
    whole: f64,
    audio: f64,
    frames: usize,
}

/// One chunk through `speech::synth`, its audio appended to `into`.
fn speak(text: &str, into: &mut Vec<f32>) -> Result<Spoken, VoiceError> {
    let request = SynthRequest { text: text.into(), voice: VOICE.into(), rate: 1.0, pitch: 1.0 };
    let from = Instant::now();
    let (mut first, mut frames, mut samples, mut rate) = (None, 0, 0usize, 1u32);
    speech::synth(&request, &mut |chunk: AudioChunk| {
        first.get_or_insert(from.elapsed().as_secs_f64());
        frames += 1;
        samples += chunk.samples.len();
        rate = chunk.rate;
        into.extend(chunk.samples);
        true
    })?;
    Ok(Spoken { first: first.unwrap_or(0.0), whole: from.elapsed().as_secs_f64(), audio: samples as f64 / rate as f64, frames })
}

fn wav(pcm: &[f32], rate: u32) -> Vec<u8> {
    let n = (pcm.len() * 2) as u32;
    let mut v = Vec::with_capacity(44 + n as usize);
    v.extend(b"RIFF");
    v.extend((36 + n).to_le_bytes());
    v.extend(b"WAVEfmt ");
    v.extend(16u32.to_le_bytes());
    v.extend(1u16.to_le_bytes()); // PCM
    v.extend(1u16.to_le_bytes()); // mono
    v.extend(rate.to_le_bytes());
    v.extend((rate * 2).to_le_bytes());
    v.extend(2u16.to_le_bytes());
    v.extend(16u16.to_le_bytes());
    v.extend(b"data");
    v.extend(n.to_le_bytes());
    for s in pcm {
        v.extend(((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    v
}

fn run() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/voice-dev/kokoro");
    speech::start_in(dir.clone(), Arc::new(|_, _| {}));

    // 1. The download, once: the product's own code, address and hash.
    let from = Instant::now();
    let there = speech::installed();
    let first_ask = from.elapsed().as_millis();
    if !there {
        println!("model: fetching {:.0} MB from {} and {}", mb(speech::DOWNLOAD_BYTES as usize), speech::MODEL_URL, speech::PHONEMES.0);
        let (from, mut received, mut tenth) = (Instant::now(), 0u64, 0u64);
        let fetched = speech::download(&mut |bytes| {
            received = bytes;
            if bytes * 10 / speech::DOWNLOAD_BYTES > tenth {
                tenth = bytes * 10 / speech::DOWNLOAD_BYTES;
                println!("model: {} % after {:.0} s", tenth * 10, from.elapsed().as_secs_f64());
            }
        });
        if let Err(e) = fetched {
            fail(&e.to_string());
        }
        println!("model: {received} bytes fetched, checked and unpacked in {:.0} s", from.elapsed().as_secs_f64());
    }
    for (name, sha256, bytes) in speech::FILES {
        let is = speech::file_sha(&dir.join("kokoro-multi-lang-v1_0").join(name)).unwrap_or_default();
        println!("model: {name}, {bytes} bytes, SHA-256 {is} {}", if is == sha256 { "as pinned" } else { "NOT as pinned" });
    }
    let again = Instant::now();
    let installed = speech::installed();
    println!(
        "model: installed = {installed} (first asked: {first_ask} ms with the hashing, a debug build's; asked again: {} µs); ready = {}; {} voices",
        again.elapsed().as_micros(),
        speech::ready(),
        speech::voices().len()
    );
    if !speech::ready() {
        fail("not ready: build meteor-speak first (cargo build -p meteor-speak --release)");
    }

    // 2. Start: the model's load.
    let from = Instant::now();
    if let Err(e) = speech::load() {
        fail(&e.to_string());
    }
    let pid = speech::server_pid().unwrap_or_else(|| fail("no server after load"));
    println!("server (pid {pid}): model loaded {} ms after its start (asked to load: {} ms)", speech::load_ms(), from.elapsed().as_millis());
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.unwrap_or_else(|_| fail("OpenProcess"));

    // 3. Three sentences back to back, into a file.
    let mut pcm = Vec::new();
    let mut spoken = Vec::new();
    for text in SENTENCES {
        let cpu_before = cpu(process);
        let s = speak(text, &mut pcm).unwrap_or_else(|e| fail(&e.to_string()));
        println!(
            "sentence ({:>3} letters): first chunk after {:>4.0} ms, whole after {:>4.0} ms, {:.2} s of audio in {} frame(s), real-time factor {:.2}, {:.1} cores busy",
            text.len(),
            s.first * 1000.0,
            s.whole * 1000.0,
            s.audio,
            s.frames,
            s.whole / s.audio,
            (cpu(process) - cpu_before).as_secs_f64() / s.whole,
        );
        pcm.extend(std::iter::repeat(0.0).take((GAP * 24_000.0) as usize));
        spoken.push(s);
    }
    let (work, audio): (f64, f64) = spoken.iter().fold((0.0, 0.0), |(w, a), s| (w + s.whole, a + s.audio));
    println!("all three: {audio:.2} s of audio in {work:.2} s, real-time factor {:.2}", work / audio);
    // As the player would run them: a sentence is made while the one before it plays.
    let (mut asked, mut free) = (0.0, 0.0);
    for (i, s) in spoken.iter().enumerate() {
        let start = f64::max(free, asked + s.first);
        if i > 0 {
            println!("played in order: sentence {} starts {:.0} ms later than the 150 ms gap wants", i + 1, (start - free) * 1000.0);
        }
        free = start + s.audio + GAP;
        asked += s.whole;
    }

    let file = dir.join("probe.wav");
    std::fs::write(&file, wav(&pcm, 24_000)).unwrap_or_else(|e| fail(&e.to_string()));
    let back = std::fs::read(&file).unwrap_or_else(|e| fail(&e.to_string()));
    let rate = u32::from_le_bytes([back[24], back[25], back[26], back[27]]);
    let samples: Vec<f32> = back[44..].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0).collect();
    let peak = samples.iter().fold(0f32, |m, s| m.max(s.abs()));
    let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len().max(1) as f32).sqrt();
    println!(
        "target/voice-dev/kokoro/probe.wav: read back {} Hz mono, {:.2} s, peak {:.2}, level {:.1} dBFS",
        rate,
        samples.len() as f64 / rate as f64,
        peak,
        20.0 * rms.max(1e-9).log10()
    );
    if rate != 24_000 || peak < 0.05 || (samples.len() as f64 / rate as f64) < 5.0 {
        fail("the file is not speech");
    }

    // 4. A sentence that is no longer wanted: the sink says no, the next one is still served.
    let refused = speech::synth(&SynthRequest { text: SENTENCES[1].into(), voice: VOICE.into(), rate: 1.0, pitch: 1.0 }, &mut |_| false);
    let after = speak(SENTENCES[2], &mut Vec::new()).unwrap_or_else(|e| fail(&e.to_string()));
    println!("a sentence the sink refuses: {refused:?}; the next one: first chunk after {:.0} ms, {:.2} s of audio", after.first * 1000.0, after.audio);
    let unknown = speech::synth(&SynthRequest { text: "Hello.".into(), voice: "nobody".into(), rate: 1.0, pitch: 1.0 }, &mut |_| true);
    println!("a voice it does not have: {unknown:?}");

    // 5. What it holds, and what it costs while it waits.
    let m = memory(process);
    println!("memory while loaded: working set {:.0} MB (peak {:.0} MB), private {:.0} MB", mb(m.working_set), mb(m.peak_working_set), mb(m.private));
    // The engine's worker threads spin for a moment after a sentence before they block: counted apart.
    let _ = speak(SENTENCES[0], &mut Vec::new());
    let before = cpu(process);
    std::thread::sleep(Duration::from_secs(1));
    let settled = cpu(process);
    std::thread::sleep(Duration::from_secs(20));
    let used = cpu(process) - settled;
    println!(
        "after a sentence: {:.0} ms of processor time in the first second; then idle for 20 s: {:.1} ms ({:.3} % of one core)",
        (settled - before).as_secs_f64() * 1000.0,
        used.as_secs_f64() * 1000.0,
        used.as_secs_f64() / 20.0 * 100.0
    );

    // 6. It ends itself after its idle time (2 s here, 5 min in the product), and comes back on demand.
    speech::unload();
    speech::keep(Duration::from_secs(2));
    speech::load().unwrap_or_else(|e| fail(&e.to_string()));
    let short = speech::server_pid().unwrap_or_else(|| fail("no server"));
    match gone_within(short, Duration::from_secs(6)) {
        Some(t) => println!("idle time 2 s: server (pid {short}) ended itself {:.1} s after it was loaded", t.as_secs_f64()),
        None => fail("the server did not end itself"),
    }
    speech::keep(Duration::from_secs(300));
    let from = Instant::now();
    speech::warm();
    let warm = from.elapsed();
    let cold = speak(SENTENCES[0], &mut Vec::new()).unwrap_or_else(|e| fail(&e.to_string()));
    println!(
        "on demand: warm() returned in {} µs; from nothing to the first chunk {:.0} ms (load {} ms)",
        warm.as_micros(),
        from.elapsed().as_secs_f64() * 1000.0 - (cold.whole - cold.first) * 1000.0,
        speech::load_ms()
    );

    // 7. Left running: no destructor runs in `exit`.
    let last = speech::server_pid().unwrap_or_else(|| fail("no server"));
    println!("SERVER {last}");
    let _ = std::io::stdout().flush();
    std::thread::sleep(Duration::from_millis(400));
    std::process::exit(0);
}

fn watcher() {
    let mut parent = Command::new(std::env::current_exe().expect("own path")).arg("run").stdin(Stdio::null()).stdout(Stdio::piped()).spawn().expect("start the parent");
    let mut server = 0u32;
    for line in BufReader::new(parent.stdout.take().expect("pipe")).lines().map_while(Result::ok) {
        if let Some(pid) = line.strip_prefix("SERVER ") {
            server = pid.trim().parse().unwrap_or(0);
            println!("server (pid {server}) running while its parent lives: {}", running(server));
            if !running(server) {
                fail("the server was not running");
            }
            // Not to the end of the pipe: the wait must not depend on who else holds it.
            break;
        }
        println!("{line}");
        if line.starts_with("FAIL") {
            std::process::exit(1);
        }
    }
    let status = parent.wait().expect("wait");
    if server == 0 || !status.success() {
        fail("the parent did not finish its checks");
    }
    match gone_within(server, Duration::from_secs(3)) {
        Some(t) => println!("parent ended without ending the server: gone {} ms later (job, kill on close)", t.as_millis()),
        None => fail("the server outlived its parent"),
    }
    println!("ok");
}

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("run") => run(),
        _ => watcher(),
    }
}
