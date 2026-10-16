// The natural voice: the client of the speech server `meteor-speak.exe` (crates/voice-speak), which runs
// Kokoro. Package voice-kokoro (VOICE §10; §7.2, §7.3). The wire is `crate::speak_proto`: requests on the
// server's stdin, frames on its stdout, no port.
//
// The server is started through `crate::proc` by `warm` (the start of the first turn, or the first chat
// answer read aloud) or by the first sentence. It loads once, waits blocked on its pipe, and ends itself
// `keepVoice` minutes after its last sentence (`timing::SPEECH_IDLE`); the next sentence starts it again.
// The model is downloaded on one click, each file checked by SHA-256 before it is kept, and the files the
// server loads are checked again before it starts.

use crate::speak_proto::{self as proto, Header};
use crate::{event, proc, timing, AudioChunk, AudioSink, Download, Emit, Engine, Result, SynthRequest, VoiceDesc, VoiceError};
use notch_core::Ctx;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

/// Kokoro's own names of its voices, the list the server has.
#[path = "../../voice-speak/src/voices.rs"]
mod names;

/// The model: Kokoro-82M v1.0 as the author of sherpa-onnx exported it, at one revision of his repository.
/// (The same files are one GitHub release asset of sherpa-onnx; from there this PC got 0.15 MB a second,
/// from here 20.)
pub const MODEL_URL: &str = "https://huggingface.co/csukuangfj/kokoro-multi-lang-v1_0/resolve/f7b96bb6bef5c5da4d3aa4f4e0498fbbf62dc78b/";
/// What the server loads by name: name under `MODEL_URL`, SHA-256, bytes.
pub const FILES: [(&str, &str, u64); 5] = [
    ("model.onnx", "b40f62b166ac8164b0627ef48a0b358eda0985e272fb03ef5252e7206305da11", 325_560_556),
    ("voices.bin", "1c5a5b983d3d50d8586d437a51f3faa2da7919ce76a013c081e65671a3447c29", 28_200_960),
    ("tokens.txt", "6ebb6bb288f20f3ae8d004d3c2ca27697da27c037d75e81a60e2a6a663f95425", 687),
    ("lexicon-us-en.txt", "7daaab53a181be9885b853a8582bf1838186317e5dadacbcef9c426d6fa0da14", 5_956_885),
    ("lexicon-zh.txt", "902bc2d20ac7c449c5ecbbbb23c89d12119c03ed5b8ed27f114f0260c9e35229", 2_365_182),
];
/// espeak-ng's phoneme tables (some 350 small files, for words the lexicon does not have): one archive, a
/// GitHub release asset of sherpa-onnx. Those have no revision: the hash is the pin.
// debt: checked by its hash when it is unpacked, not again before each start.
pub const PHONEMES: (&str, &str, u64) = (
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/espeak-ng-data.zip",
    "bc4525eafe31b4e3f5e43aea495f3169e97dd2544f1bbfe95514ce8a61baee39",
    9_037_020,
);
/// The folder that archive unpacks to, and a file that is in it.
const PHONEMES_FOLDER: &str = "espeak-ng-data";
const PHONEMES_FILE: &str = "espeak-ng-data/phontab";
/// The whole download.
pub const DOWNLOAD_BYTES: u64 = FILES[0].2 + FILES[1].2 + FILES[2].2 + FILES[3].2 + FILES[4].2 + PHONEMES.2;
/// The model's folder under the feature's.
const FOLDER: &str = "kokoro-multi-lang-v1_0";

const SERVER_EXE: &str = "meteor-speak.exe";
/// The engine, which must sit beside the server: System32 has an older onnxruntime.dll of its own.
const SERVER_DLLS: [&str; 2] = ["sherpa-onnx-c-api.dll", "onnxruntime.dll"];
/// Bursts of this many threads while a sentence is made.
const THREADS: &str = "4";
/// After an engine that did not load, the Windows voice speaks this long before another try.
const RETRY: Duration = Duration::from_secs(60);
/// The whole download, on a slow line.
const DOWNLOAD_LIMIT: Duration = Duration::from_secs(2 * 3600);

/// Where the download lives and where events go; `None` while voice is off.
static HOME: Mutex<Option<(PathBuf, Emit)>> = Mutex::new(None);
/// The pipes, held for a whole sentence. The process is in `CHILD`, so that it can be ended meanwhile.
static SERVER: Mutex<Option<Server>> = Mutex::new(None);
static CHILD: Mutex<Option<proc::Child>> = Mutex::new(None);
/// Seconds the server stays after its last sentence (the setting `keepVoice`).
static KEEP: AtomicU64 = AtomicU64::new(timing::SPEECH_IDLE.as_secs());
static DOWNLOADING: AtomicBool = AtomicBool::new(false);
/// Raised by `stop` and `remove`: a download from before it gives up.
static EPOCH: AtomicU64 = AtomicU64::new(0);
/// How long the last start took until the model was loaded.
static LOAD_MS: AtomicU64 = AtomicU64::new(0);
/// Size and date of `FILES` when their hashes last matched: they are hashed once, not per sentence.
static VERIFIED: Mutex<Option<Vec<(u64, Option<SystemTime>)>>> = Mutex::new(None);
/// When the engine last failed to load.
static FAILED: Mutex<Option<Instant>> = Mutex::new(None);

struct Server {
    stdin: ChildStdin,
    frames: Receiver<(Header, Vec<f32>)>,
    next: u64,
}

fn failed(e: impl std::fmt::Display) -> VoiceError {
    VoiceError::Failed(e.to_string())
}

fn home() -> Option<(PathBuf, Emit)> {
    HOME.lock().unwrap().clone()
}

fn model_dir() -> Option<PathBuf> {
    home().map(|(dir, _)| dir.join(FOLDER))
}

/// `start` without the app, for a probe: the folder of the download and where `voice:download` goes.
pub fn start_in(dir: PathBuf, emit: Emit) {
    *HOME.lock().unwrap() = Some((dir, emit));
    *FAILED.lock().unwrap() = None;
}

pub fn start(cx: &Ctx) -> Result<()> {
    start_in(cx.data_dir(crate::ID).join("kokoro"), crate::emitter(cx));
    settings(cx, &cx.settings(crate::ID));
    Ok(())
}

/// Ends the server and a download. (`crate::pause` ends the server through `proc::kill_all`; the next
/// sentence starts it again.)
pub fn stop(_cx: &Ctx) {
    *HOME.lock().unwrap() = None;
    EPOCH.fetch_add(1, Ordering::SeqCst);
    unload();
}

pub fn settings(_cx: &Ctx, settings: &Value) {
    if let Some(minutes) = settings["keepVoice"].as_f64().filter(|m| *m > 0.0) {
        keep(Duration::from_secs_f64(minutes * 60.0));
    }
}

/// How long the server stays after its last sentence.
// ponytail: a server that runs keeps the time it was started with; it is told again at its next start.
pub fn keep(idle: Duration) {
    KEEP.store(idle.as_secs().max(1), Ordering::Relaxed);
}

/// `status` → `{installed, downloading, running, engine, bytes}` (`engine`: the server shipped with the
/// app is there; `bytes`: the size of the download) · `download` → `{}` (progress as
/// `voice:download {what: "speech"}`) · `remove` → `{}` (deletes the download) · `warm` → `{}` ·
/// `unload` → `{}`.
pub fn call(_cx: &Ctx, method: &str, _args: Value) -> Result<Value> {
    match method {
        "status" => {
            return Ok(json!({
                "installed": installed(),
                "downloading": DOWNLOADING.load(Ordering::SeqCst),
                "running": server_pid().is_some(),
                "engine": server_exe().is_some(),
                "bytes": DOWNLOAD_BYTES,
            }))
        }
        "download" => download_in_background(),
        "remove" => {
            EPOCH.fetch_add(1, Ordering::SeqCst);
            unload();
            if let Some((dir, _)) = home() {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
        "warm" => warm(),
        "unload" => unload(),
        _ => return Err(failed("unknown method")),
    }
    Ok(json!({}))
}

// ── The download ──────────────────────────────────────────────────────────────

fn hex(hash: impl AsRef<[u8]>) -> String {
    hash.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

pub fn file_sha(path: &Path) -> std::io::Result<String> {
    let mut hasher = Sha256::new();
    std::io::copy(&mut BufReader::with_capacity(1 << 20, std::fs::File::open(path)?), &mut hasher)?;
    Ok(hex(hasher.finalize()))
}

/// The model is downloaded and the files the server loads match their hashes.
pub fn installed() -> bool {
    let Some(dir) = model_dir() else { return false };
    if !dir.join(PHONEMES_FILE).is_file() {
        return false;
    }
    let now: Option<Vec<_>> = FILES.iter().map(|(name, ..)| std::fs::metadata(dir.join(name)).ok().map(|m| (m.len(), m.modified().ok()))).collect();
    let Some(now) = now else { return false };
    // Held while hashing: a second asker waits for the answer instead of hashing 0.3 GB too.
    let mut verified = VERIFIED.lock().unwrap();
    if verified.as_ref() != Some(&now) {
        *verified = FILES.iter().all(|(name, sha, _)| file_sha(&dir.join(name)).is_ok_and(|h| h == *sha)).then_some(now);
    }
    verified.is_some()
}

/// The server and its engine, as the installer lays them out (`speak\`), then where a build leaves
/// them: beside a debug app (`tauri dev`, as the relay), beside an example.
fn server_exe() -> Option<PathBuf> {
    let app = std::env::current_exe().ok()?;
    let here = app.parent()?;
    ["speak", "../release", "../../release", ".", ".."]
        .iter()
        .map(|folder| here.join(folder))
        .find(|folder| folder.join(SERVER_EXE).is_file() && SERVER_DLLS.iter().all(|dll| folder.join(dll).is_file()))
        .map(|folder| folder.join(SERVER_EXE))
}

/// One file to `to`, checked against its pin. `step` gets the bytes received so far.
async fn fetch(url: &str, to: &Path, sha256: &str, bytes: u64, epoch: u64, step: &mut dyn FnMut(u64)) -> Result<()> {
    let url = reqwest::Url::parse(url).map_err(failed)?;
    // Both hosts answer with a redirect to their file store; `net::client` follows it, to https only.
    let client = notch_core::net::client(&url, DOWNLOAD_LIMIT).map_err(failed)?;
    let mut response = client.get(url).send().await.map_err(failed)?;
    if !response.status().is_success() {
        return Err(failed(format!("download: HTTP {}", response.status().as_u16())));
    }
    let mut file = std::fs::File::create(to).map_err(failed)?;
    let (mut hasher, mut received) = (Sha256::new(), 0u64);
    while let Some(chunk) = response.chunk().await.map_err(failed)? {
        received += chunk.len() as u64;
        if EPOCH.load(Ordering::SeqCst) != epoch {
            return Err(VoiceError::Cancelled);
        }
        if received > bytes {
            return Err(failed("download: longer than the pinned file"));
        }
        file.write_all(&chunk).map_err(failed)?;
        hasher.update(&chunk);
        step(received);
    }
    if hex(hasher.finalize()) != sha256 {
        return Err(failed("download: checksum mismatch, file deleted"));
    }
    Ok(())
}

/// `fetch` into a `.part` beside `to`, which becomes `to` only when it matched; else it is deleted.
fn fetch_checked(url: &str, to: &Path, sha256: &str, bytes: u64, epoch: u64, step: &mut dyn FnMut(u64)) -> Result<()> {
    let mut part = to.as_os_str().to_owned();
    part.push(".part");
    let got = tauri::async_runtime::block_on(fetch(url, Path::new(&part), sha256, bytes, epoch, step)).and_then(|()| std::fs::rename(&part, to).map_err(failed));
    if got.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    got
}

fn unpack(archive: &Path, into: &Path) -> Result<()> {
    // Windows' own tar reads zip. The archive matched its hash, so the names in it are the known ones.
    let tar = PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into())).join("System32").join("tar.exe");
    let _ = std::fs::remove_dir_all(into.join(PHONEMES_FOLDER));
    let mut tar = proc::spawn(Command::new(tar).arg("-xf").arg(archive).arg("-C").arg(into).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()))?;
    match tar.0.wait() {
        Ok(status) if status.success() => Ok(()),
        _ => Err(failed("the natural voice could not be unpacked")),
    }
}

/// Fetches what is missing or wrong, each file checked before it is kept, and checks what the server
/// will load. Blocks; `step` gets the bytes there so far, of `DOWNLOAD_BYTES`.
// debt: a file whose download broke starts again from its first byte (the model is nine tenths of it all).
pub fn download(step: &mut dyn FnMut(u64)) -> Result<()> {
    let Some(dir) = model_dir() else { return Err(VoiceError::Cancelled) };
    let epoch = EPOCH.load(Ordering::SeqCst);
    std::fs::create_dir_all(&dir).map_err(failed)?;
    let mut have = 0;
    for (name, sha256, bytes) in FILES {
        let to = dir.join(name);
        if !(std::fs::metadata(&to).is_ok_and(|m| m.len() == bytes) && file_sha(&to).is_ok_and(|h| h == sha256)) {
            fetch_checked(&format!("{MODEL_URL}{name}"), &to, sha256, bytes, epoch, &mut |n| step(have + n))?;
        }
        have += bytes;
        step(have);
    }
    if !dir.join(PHONEMES_FILE).is_file() {
        let (url, sha256, bytes) = PHONEMES;
        let archive = dir.join("espeak-ng-data.zip");
        let unpacked = fetch_checked(url, &archive, sha256, bytes, epoch, &mut |n| step(have + n)).and_then(|()| unpack(&archive, &dir));
        let _ = std::fs::remove_file(&archive);
        unpacked?;
    }
    *FAILED.lock().unwrap() = None;
    if installed() {
        Ok(())
    } else {
        Err(failed("the natural voice did not unpack whole"))
    }
}

fn download_in_background() {
    let Some((_, emit)) = home() else { return };
    if DOWNLOADING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        let tell = |received: u64, done: bool, error: Option<String>| {
            emit(event::DOWNLOAD, json!(Download { what: "speech".into(), received, total: DOWNLOAD_BYTES, done, error }));
        };
        let mut told = 0u64;
        let result = download(&mut |received| {
            // Every 2 MB: some 180 events.
            if received.saturating_sub(told) >= 2 << 20 {
                told = received;
                tell(received, false, None);
            }
        });
        DOWNLOADING.store(false, Ordering::SeqCst);
        match result {
            Ok(()) => tell(DOWNLOAD_BYTES, true, None),
            Err(e) => tell(told, true, Some(e.to_string())),
        }
    });
}

// ── The server ────────────────────────────────────────────────────────────────

/// The natural voice can speak now: model, server and engine are there, and the engine did not just fail.
pub fn ready() -> bool {
    !FAILED.lock().unwrap().is_some_and(|at| at.elapsed() < RETRY) && server_exe().is_some() && installed()
}

/// Kokoro's 28 English voices, ids `kokoro:<name>`; empty until the model is downloaded.
pub fn voices() -> Vec<VoiceDesc> {
    if !installed() {
        return Vec::new();
    }
    names::VOICES
        .iter()
        .map(|own| {
            let given = own.split('_').nth(1).unwrap_or(own);
            VoiceDesc {
                id: format!("kokoro:{own}"),
                name: given[..1].to_uppercase() + &given[1..],
                engine: Engine::Kokoro,
                lang: if own.starts_with('b') { "en-GB" } else { "en-US" }.into(),
            }
        })
        .collect()
}

/// The running server's process.
pub fn server_pid() -> Option<u32> {
    CHILD.lock().unwrap().as_mut().and_then(|child| child.alive().then(|| child.pid()))
}

/// How long the last start took until the model was loaded, in milliseconds.
pub fn load_ms() -> u64 {
    LOAD_MS.load(Ordering::Relaxed)
}

/// Ends the server now, also in the middle of a sentence (whose `synth` then fails).
pub fn unload() {
    *CHILD.lock().unwrap() = None;
    if let Ok(mut server) = SERVER.try_lock() {
        *server = None;
    }
}

fn start_server() -> Result<Server> {
    let (true, Some(exe), Some(dir)) = (ready(), server_exe(), model_dir()) else {
        return Err(VoiceError::Missing("the natural voice".into()));
    };
    let from = Instant::now();
    let mut child = proc::spawn(
        Command::new(exe)
            .arg("--model")
            .arg(dir)
            .args(["--threads", THREADS, "--idle", &KEEP.load(Ordering::Relaxed).to_string()])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null()),
    )?;
    let (Some(stdin), Some(stdout)) = (child.0.stdin.take(), child.0.stdout.take()) else {
        return Err(failed("no pipe to the speech server"));
    };
    // A thread of its own reads: a wait here can then have a limit, and the server never waits on a full
    // pipe. It ends with the server.
    let (tx, frames) = channel();
    std::thread::spawn(move || {
        let mut stdout = BufReader::new(stdout);
        while let Ok(frame) = proto::read_frame(&mut stdout) {
            if tx.send(frame).is_err() {
                break;
            }
        }
    });
    *CHILD.lock().unwrap() = Some(child);
    // The frame that says "loaded", or why not.
    let loaded = match frames.recv_timeout(timing::CHILD_START) {
        Ok((header, _)) if header.id == proto::READY && header.error.is_none() => Ok(()),
        Ok((header, _)) => Err(failed(header.error.unwrap_or_else(|| "the speech server spoke out of turn".into()))),
        Err(RecvTimeoutError::Timeout) => Err(VoiceError::Timeout("the natural voice")),
        Err(RecvTimeoutError::Disconnected) => Err(failed("the speech engine is not available")),
    };
    if let Err(e) = loaded {
        // Ended from outside while it loaded (a pause, voice off: exit code 1) is not a broken engine.
        let ended = CHILD.lock().unwrap().take().is_none_or(|mut child| !matches!(e, VoiceError::Timeout(_)) && child.0.wait().is_ok_and(|s| s.code() == Some(1)));
        if !ended {
            *FAILED.lock().unwrap() = Some(Instant::now());
            notch_core::log::line(format!("voice: speech server did not load: {e}"));
        }
        return Err(e);
    }
    let ms = from.elapsed().as_millis() as u64;
    LOAD_MS.store(ms, Ordering::Relaxed);
    notch_core::log::line(format!("voice: speech server loaded in {ms} ms"));
    Ok(Server { stdin, frames, next: 1 })
}

/// A loaded server in `slot`: the one that runs, or a new one (none yet, or it ended: idle, a pause).
fn ensure(slot: &mut Option<Server>) -> Result<&mut Server> {
    if slot.is_none() || server_pid().is_none() {
        *slot = None;
        *slot = Some(start_server()?);
    }
    Ok(slot.as_mut().expect("just set"))
}

/// Starts the server if it is not running and waits until its model is loaded.
pub fn load() -> Result<()> {
    ensure(&mut SERVER.lock().unwrap()).map(|_| ())
}

/// Starts the server if it is not running and returns at once.
pub fn warm() {
    std::thread::spawn(load);
}

/// One chunk through the server: 24 kHz pieces to `sink` as its frames arrive, starting the server first
/// if it must. Blocks until the last frame. One request at a time: `mouth` calls from one thread.
// ponytail: the wire has no "stop". A sentence the sink no longer wants is finished by the server (a
// second or two of work) and its frames are dropped here by their id; add a cancel line if it shows.
pub fn synth(request: &SynthRequest, sink: AudioSink) -> Result<()> {
    let mut slot = SERVER.lock().unwrap();
    let line = |id| proto::Request { id, text: request.text.clone(), voice: request.voice.clone(), speed: request.rate };
    let mut server = ensure(&mut slot)?;
    if proto::write_request(&mut server.stdin, &line(server.next)).is_err() {
        // It ended itself (idle) between the look and the line: once more, with a new one.
        *slot = None;
        server = ensure(&mut slot)?;
        proto::write_request(&mut server.stdin, &line(server.next)).map_err(failed)?;
    }
    let id = server.next;
    server.next += 1;
    loop {
        let (header, pcm) = match server.frames.recv_timeout(timing::CHILD_START) {
            Ok(frame) => frame,
            Err(RecvTimeoutError::Timeout) => {
                *CHILD.lock().unwrap() = None;
                return Err(VoiceError::Timeout("the natural voice"));
            }
            Err(RecvTimeoutError::Disconnected) => return Err(failed("the natural voice stopped")),
        };
        // What is left of a sentence that was no longer wanted.
        if header.id != id {
            continue;
        }
        if let Some(error) = header.error {
            return Err(failed(error));
        }
        if !pcm.is_empty() && !sink(AudioChunk { rate: header.rate, samples: pcm }) {
            return Err(VoiceError::Cancelled);
        }
        if header.last {
            return Ok(());
        }
    }
}
