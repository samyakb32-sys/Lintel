// meteor-speak.exe: the speech server of the voice assistant (docs/VOICE.md §7.2). It loads Kokoro once
// and waits: one request per line on stdin, frames of 24 kHz PCM on stdout, no port. Between sentences it
// is blocked on its pipe and uses no processor (measured: none from one second after a sentence; in the
// half second before that the engine's worker threads spin, which its C API has no switch for). It ends
// when stdin ends (its parent is gone), and by itself `--idle` seconds after the last sentence;
// crates/voice/src/speech.rs starts it again on demand.
//
// The engine is sherpa-onnx through its C API (sherpa-onnx-c-api.dll and onnxruntime.dll, which sit
// beside this exe: Windows has an older onnxruntime.dll of its own in System32, and only the exe's own
// folder is searched before that one).
//
// The wire is one file shared with the client, included here by path so that neither crate depends on
// the other.

#[allow(dead_code)]
#[path = "../../voice/src/speak_proto.rs"]
mod proto;
mod voices;

use sherpa_onnx::{GenerationConfig, OfflineTts, OfflineTtsConfig, OfflineTtsKokoroModelConfig, OfflineTtsModelConfig};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const USAGE: &str = "usage: meteor-speak --model <folder> [--threads <1-8>] [--idle <seconds, 0 = stay>]";

/// Exit codes, read by the client.
const EXIT_USAGE: i32 = 2;
const EXIT_ENGINE: i32 = 3;

/// What the engine is given, by name, from inside the model folder.
const FILES: [&str; 6] = ["model.onnx", "voices.bin", "tokens.txt", "lexicon-us-en.txt", "lexicon-zh.txt", "espeak-ng-data"];

#[derive(Debug, PartialEq)]
struct Args {
    /// The unpacked `kokoro-multi-lang-v1_0` folder (model.onnx, voices.bin, tokens, lexicon, espeak-ng data).
    model: PathBuf,
    /// Bursts of this many threads while a sentence is synthesised.
    threads: u32,
    /// The server ends this long after its last sentence; 0 = only when stdin ends.
    idle: u64,
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Args, String> {
    let (mut model, mut threads, mut idle) = (None, 4u32, 0u64);
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--model" => model = Some(PathBuf::from(value()?)),
            "--threads" => threads = value()?.parse().ok().filter(|n| (1..=8).contains(n)).ok_or("--threads takes 1 to 8")?,
            "--idle" => idle = value()?.parse().map_err(|_| "--idle takes seconds")?,
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let model = model.ok_or("--model is missing")?;
    if !model.is_dir() {
        return Err(format!("{} is not a folder", model.display()));
    }
    Ok(Args { model, threads, idle })
}

fn load(args: &Args) -> Result<OfflineTts, String> {
    // By name from inside the folder: the engine opens some of its files through the ANSI code page, and
    // a user name with a letter outside it would break a full path.
    std::env::set_current_dir(&args.model).map_err(|e| e.to_string())?;
    // The engine ends the whole process on a file it cannot find; say it first.
    if let Some(missing) = FILES.iter().find(|f| !Path::new(f).exists()) {
        return Err(format!("{missing} is missing"));
    }
    // ponytail: one load, with the American lexicon: the eight British voices keep their accent but read
    // by it. A second load with lexicon-gb-en.txt (another 0.3 GB) if anyone hears the difference.
    let config = OfflineTtsConfig {
        model: OfflineTtsModelConfig {
            kokoro: OfflineTtsKokoroModelConfig {
                model: Some("model.onnx".into()),
                voices: Some("voices.bin".into()),
                tokens: Some("tokens.txt".into()),
                data_dir: Some("espeak-ng-data".into()),
                dict_dir: Path::new("dict").is_dir().then(|| "dict".into()),
                lexicon: Some("lexicon-us-en.txt,lexicon-zh.txt".into()),
                ..Default::default()
            },
            num_threads: args.threads as i32,
            ..Default::default()
        },
        // One sentence at a time, so a chunk of several sentences comes back piece by piece.
        max_num_sentences: 1,
        ..Default::default()
    };
    let tts = OfflineTts::create(&config).ok_or("the engine did not load the model")?;
    if tts.num_speakers() < voices::VOICES.len() as i32 {
        return Err(format!("not the expected model ({} speakers)", tts.num_speakers()));
    }
    Ok(tts)
}

fn header(id: u64, rate: u32) -> proto::Header {
    proto::Header { id, rate, samples: 0, last: false, error: None }
}

fn error(id: u64, why: impl Into<String>) -> io::Result<()> {
    proto::write_frame(&mut io::stdout().lock(), &proto::Header { last: true, error: Some(why.into()), ..header(id, proto::RATE) }, &[])
}

/// One request: its audio as frames while it is made, then the frame that says `last`. An error of the
/// pipe means the client is gone.
fn speak(tts: &OfflineTts, request: proto::Request) -> io::Result<()> {
    let Some(speaker) = voices::VOICES.iter().position(|v| *v == request.voice) else {
        return error(request.id, format!("unknown voice {}", request.voice));
    };
    let head = header(request.id, tts.sample_rate() as u32);
    let config = GenerationConfig { sid: speaker as i32, speed: request.speed.clamp(0.5, 2.0), ..Default::default() };
    let sent = Arc::new(AtomicUsize::new(0));
    let piece = {
        let (head, sent) = (head.clone(), sent.clone());
        move |pcm: &[f32], _progress: f32| {
            // `false` stops the engine: nobody reads any more.
            let written = pcm.chunks(proto::MAX_SAMPLES as usize).all(|part| proto::write_frame(&mut io::stdout().lock(), &head, part).is_ok());
            sent.fetch_add(pcm.len(), Ordering::Relaxed);
            written
        }
    };
    // A zero byte would end the text early inside the engine (and the wrapper refuses it).
    match tts.generate_with_config(&request.text.replace('\0', " "), &config, Some(piece)) {
        Some(audio) => {
            // Whatever the engine did not hand over piece by piece.
            let rest = audio.samples().get(sent.load(Ordering::Relaxed)..).unwrap_or(&[]);
            let rest = &rest[..rest.len().min(proto::MAX_SAMPLES as usize)];
            proto::write_frame(&mut io::stdout().lock(), &proto::Header { last: true, ..head }, rest)
        }
        None => error(request.id, "nothing was synthesised"),
    }
}

/// Ends the process once nothing was asked for `idle`. The thread sleeps until the moment that could be
/// true: one wake per idle period, none while a sentence is made.
fn end_when_idle(idle: Duration, since: Arc<Mutex<Option<Instant>>>) {
    std::thread::spawn(move || loop {
        let quiet = since.lock().unwrap().map(|at| at.elapsed());
        match quiet {
            Some(quiet) if quiet >= idle => std::process::exit(0),
            Some(quiet) => std::thread::sleep(idle - quiet),
            None => std::thread::sleep(idle),
        }
    });
}

fn main() {
    let args = match parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(e) => {
            eprintln!("meteor-speak: {e}\n{USAGE}");
            std::process::exit(EXIT_USAGE);
        }
    };
    let tts = match load(&args) {
        Ok(tts) => tts,
        Err(e) => {
            // "Engine not available": the client keeps the Windows voice.
            let _ = error(proto::READY, e);
            std::process::exit(EXIT_ENGINE);
        }
    };
    if proto::write_frame(&mut io::stdout().lock(), &proto::Header { last: true, ..header(proto::READY, tts.sample_rate() as u32) }, &[]).is_err() {
        return;
    }
    // `None` while a sentence is made.
    let since = Arc::new(Mutex::new(Some(Instant::now())));
    if args.idle > 0 {
        end_when_idle(Duration::from_secs(args.idle), since.clone());
    }
    let mut stdin = io::stdin().lock();
    loop {
        match proto::read_request(&mut stdin) {
            Ok(Some(request)) => {
                *since.lock().unwrap() = None;
                let written = speak(&tts, request);
                *since.lock().unwrap() = Some(Instant::now());
                if written.is_err() {
                    break;
                }
            }
            // A line that is not a request has no id to answer to.
            Err(e) if e.kind() == io::ErrorKind::InvalidData => {}
            Ok(None) | Err(_) => break,
        }
    }
    let _ = io::stdout().flush();
}
