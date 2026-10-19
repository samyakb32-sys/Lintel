// Whistle: Cactus Compute's on-device speech model, run through their Needle engine.
//
// Engine choice: Option A, `needle.exe` as a child process on a temp WAV. Nothing to link, nothing to
// vendor, and the process (and the model in its memory) is gone after every clip.
// debt: the model (17 MB) is loaded again for every phrase (about 0.2 s) and the exchange goes through
// a WAV file. Linking libneedle.a (MSVC may not take a GNU archive) or `needle.exe --serve` would keep it
// loaded. The child is ended after ENGINE_TIMEOUT.
//
// Both files are downloaded only on the user's click, pinned to a repository revision, and checked by
// SHA-256 before the download is kept and again before the engine is run.

use sha2::{Digest, Sha256};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::io::{Read, Write};
use std::time::{Duration, Instant};

/// How long one phrase may take in the engine before its process is ended.
const ENGINE_TIMEOUT: Duration = Duration::from_secs(30);
/// The error of a run that was ended for it.
pub const TIMED_OUT: &str = "the speech engine did not answer in time";

pub struct File {
    pub name: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub max: usize,
}

pub const FILES: [File; 2] = [
    File {
        name: "whistle.cact",
        url: "https://huggingface.co/Cactus-Compute/whistle/resolve/ca5287601bef25af26dcf2e1b2bdc0843a7c19e5/whistle.cact",
        sha256: "b6e02f048568ac5d01a2042556c658061e699acbc0aa2a1439f52f3d461dffeb",
        max: 20 << 20,
    },
    File {
        name: "needle.exe",
        url: "https://huggingface.co/Cactus-Compute/needle3/resolve/93e88ede867e4311c39151285e74df513e4b835b/windows-x86_64/needle.exe",
        sha256: "47cd411a022d22e393d87ce091848fd446e6d25df2ce31b12a7328337557c85b",
        max: 4 << 20,
    },
];

fn hex(digest: impl AsRef<[u8]>) -> String {
    digest.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

fn sha(bytes: &[u8]) -> String {
    hex(Sha256::digest(bytes))
}

/// True when both files exist and match their pinned hash.
pub fn ready(dir: &Path) -> bool {
    FILES.iter().all(|f| std::fs::read(dir.join(f.name)).is_ok_and(|b| sha(&b) == f.sha256))
}

/// Downloads what is missing or wrong; `step` is told each file as it starts.
pub async fn download(dir: &Path, mut step: impl FnMut(&str)) -> Result<(), String> {
    download_with(dir, |name, received, _| {
        if received == 0 {
            step(name)
        }
    })
    .await
}

/// As `download`, telling `progress(file, received, total)` as a file starts (0 received) and after every
/// piece of it (`total` is 0 when the server does not say).
///
/// A file goes to disk as it arrives, into `<name>.part`, hashed on the way, and takes its name only when
/// the hash matches: a download that was cut is never taken for the model. `net::bounded_get` refuses
/// redirects and Hugging Face redirects to a CDN, so this uses `net::client` (which follows them) and
/// insists on https at the end.
pub async fn download_with(dir: &Path, mut progress: impl FnMut(&str, u64, u64)) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    for f in &FILES {
        let path = dir.join(f.name);
        if std::fs::read(&path).is_ok_and(|b| sha(&b) == f.sha256) {
            continue;
        }
        progress(f.name, 0, 0);
        let url = reqwest::Url::parse(f.url).map_err(|e| e.to_string())?;
        let client = notch_core::net::client(&url, Duration::from_secs(600))?;
        let mut resp = client.get(url).send().await.map_err(|e| e.to_string())?;
        if resp.url().scheme() != "https" {
            return Err("download redirected away from https".into());
        }
        if !resp.status().is_success() {
            return Err(format!("{}: HTTP {}", f.name, resp.status().as_u16()));
        }
        let total = resp.content_length().unwrap_or(0);
        let part = dir.join(format!("{}.part", f.name));
        let fetched: Result<String, String> = async {
            let mut out = std::fs::File::create(&part).map_err(|e| e.to_string())?;
            let (mut hash, mut received) = (Sha256::new(), 0u64);
            while let Some(piece) = resp.chunk().await.map_err(|e| e.to_string())? {
                received += piece.len() as u64;
                if received > f.max as u64 {
                    return Err(format!("{}: larger than it should be", f.name));
                }
                hash.update(&piece);
                out.write_all(&piece).map_err(|e| e.to_string())?;
                progress(f.name, received, total);
            }
            Ok(hex(hash.finalize()))
        }
        .await;
        match fetched {
            Ok(hash) if hash == f.sha256 => std::fs::rename(&part, &path).map_err(|e| e.to_string())?,
            other => {
                let _ = std::fs::remove_file(&part);
                return Err(other.err().unwrap_or_else(|| format!("{}: checksum mismatch, file deleted", f.name)));
            }
        }
    }
    Ok(())
}

fn wav(pcm: &[f32]) -> Vec<u8> {
    let n = (pcm.len() * 2) as u32;
    let mut v = Vec::with_capacity(44 + n as usize);
    v.extend(b"RIFF");
    v.extend((36 + n).to_le_bytes());
    v.extend(b"WAVEfmt ");
    v.extend(16u32.to_le_bytes());
    v.extend(1u16.to_le_bytes()); // PCM
    v.extend(1u16.to_le_bytes()); // mono
    v.extend(16000u32.to_le_bytes());
    v.extend(32000u32.to_le_bytes());
    v.extend(2u16.to_le_bytes());
    v.extend(16u16.to_le_bytes());
    v.extend(b"data");
    v.extend(n.to_le_bytes());
    for s in pcm {
        v.extend(((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    v
}

struct Temp(PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub struct Transcript {
    pub text: String,
    pub language: String,
}

/// What the voice assistant asks of the engine on top of dictation's plain run.
#[derive(Clone, Copy, Default)]
pub struct Options<'a> {
    /// `--audio-language`: the language is taken as this, not detected.
    pub language: Option<&'a str>,
    /// `--audio-keywords`: words and phrases to favour (the wake phrase).
    pub keywords: &'a [String],
}

/// 16 kHz mono PCM in, text out. The clip lives in a temp file only while the engine runs.
pub fn transcribe(dir: &Path, pcm: &[f32]) -> Result<Transcript, String> {
    if !ready(dir) {
        return Err("speech model missing or damaged".into());
    }
    transcribe_with(dir, pcm, Options::default())
}

/// As `transcribe`, with the engine's options, for a caller that has checked `ready` itself: the voice
/// assistant runs the engine several times in a turn and keeps the result of the check between them.
pub fn transcribe_with(dir: &Path, pcm: &[f32], options: Options) -> Result<Transcript, String> {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let name = format!("meteor-dictation-{}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    let tmp = Temp(std::env::temp_dir().join(format!("{name}.wav")));
    std::fs::write(&tmp.0, wav(pcm)).map_err(|e| e.to_string())?;
    let mut cmd = Command::new(dir.join("needle.exe"));
    cmd.arg("--model").arg(dir.join("whistle.cact")).arg("--audio").arg(&tmp.0);
    if let Some(language) = options.language {
        cmd.arg("--audio-language").arg(language);
    }
    // The engine reads its keywords from a file, one per line; it goes when the run ends, as the clip does.
    let keywords = Temp(std::env::temp_dir().join(format!("{name}.txt")));
    if !options.keywords.is_empty() {
        std::fs::write(&keywords.0, options.keywords.join("\n") + "\n").map_err(|e| e.to_string())?;
        cmd.arg("--audio-keywords").arg(&keywords.0);
    }
    let mut child = cmd
        // The engine's README says its telemetry is on unless these are set. METEOR V1 sends none.
        .env("NEEDLE_TELEMETRY", "0")
        .env("DO_NOT_TRACK", "1")
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .spawn()
        .map_err(|e| e.to_string())?;
    // Read on a thread so a chatty engine cannot fill the pipe and wait on us while we wait on it.
    let mut pipe = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        if let Some(p) = pipe.as_mut() {
            let _ = p.read_to_end(&mut out);
        }
        out
    });
    // A clip is at most 30 s of speech and takes a second or two: an engine still going after this is stuck.
    let deadline = Instant::now() + ENGINE_TIMEOUT;
    let code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Err(TIMED_OUT.into());
            }
        }
    };
    drop((tmp, keywords));
    let stdout = reader.join().unwrap_or_default();
    let stdout = String::from_utf8_lossy(&stdout);
    let v = stdout
        .lines()
        .rev()
        .find_map(|l| serde_json::from_str::<serde_json::Value>(l.trim()).ok().filter(|v| v.get("text").is_some()))
        .ok_or_else(|| format!("engine gave no transcript (exit {code:?})"))?;
    Ok(Transcript {
        text: v["text"].as_str().unwrap_or("").trim().to_string(),
        language: v["language"].as_str().unwrap_or("").to_string(),
    })
}
