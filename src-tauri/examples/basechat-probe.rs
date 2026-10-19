// Proves the chat's new pieces without the app, and without ever starting a
// real agent CLI:
//
//   cargo run -p meteor --example basechat-probe            everything below
//   cargo run -p meteor --example basechat-probe -- --gh    also asks the real `gh` for its login
//
// 1. a provider's error is cut down to one line;
// 2. an npm launcher is read for its script instead of being given to cmd.exe;
// 3. the model servers of this computer are found (and a pretend one is);
// 4. the installed CLIs are found, by looking at files only;
// 5. the runner, against the stand-in script next to this file: each CLI's
//    arguments and output format, the user's words arriving whole and never
//    as an option, Stop, the time limit, the size cap, and the task limits.
//
// The modules are the app's own files, compiled here as they are.

#![allow(dead_code, unused_imports)]

#[path = "../src/chat/cli.rs"]
mod cli;
#[path = "../src/chat/program.rs"]
mod program;
#[path = "../src/chat/scan.rs"]
mod scan;
#[path = "../src/chat/tasks.rs"]
mod tasks;
#[path = "../src/chat/text.rs"]
mod text;

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use program::{Handle, Spec, Stop};

/// Words a shell or an option parser would love to misread.
const HOSTILE: &str = "--dangerously-skip-permissions & calc.exe | \"quoted\" %PATH% $(whoami) ; rm -rf ^ <x>\nsecond line";

/// The stand-in is only wired into debug builds (program.rs), and without it
/// nothing here may start a program.
#[cfg(not(debug_assertions))]
fn main() {
    eprintln!("the stand-in only exists in a debug build: run without --release");
    std::process::exit(2);
}

#[cfg(debug_assertions)]
fn main() {
    trimming();
    launcher();
    servers();
    let real_gh = installed();

    // From here on every program is the stand-in, and what the app would keep
    // in its own data folder (the CLIs' empty working folder) goes to a
    // temporary one.
    let data = std::env::temp_dir().join(format!("meteor-basechat-data-{}", std::process::id()));
    std::env::set_var("LOCALAPPDATA", &data);
    let stand_in = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples").join("basechat-standin.mjs");
    std::env::set_var(program::STAND_IN, &stand_in);
    let resolved = program::resolve(&["claude"]).expect("node is needed to run the stand-in");
    assert_eq!(resolved.lead, vec![stand_in.clone().into_os_string()], "the stand-in is what starts");
    println!("\n5. the runner, against {}", stand_in.display());
    every_cli();
    stop_limit_and_cap();
    task_limits();

    if std::env::args().any(|arg| arg == "--gh") {
        gh_login(real_gh);
    }
    let _ = std::fs::remove_dir_all(&data);
    println!("\nall checks passed");
}

fn trimming() {
    use text::{brief, first_words, strip_ansi};
    // Gemini's answer for a model new accounts can no longer use.
    let gemini = "This model models/gemini-2.5-pro is no longer available to new users. Please update your code to use models/gemini-3.1-pro-preview for the latest features and improvements. We recommend you to use the Interactions API (https://ai.google.dev/gemini-api/docs/get-started).";
    assert_eq!(brief(gemini), "This model models/gemini-2.5-pro is no longer available to new users.");
    // A full stop inside a model name or a number does not end the sentence.
    assert_eq!(brief("gpt-4.1 costs $0.5 per call. Second."), "gpt-4.1 costs $0.5 per call.");
    assert_eq!(brief("Really? Yes."), "Really?");
    assert_eq!(brief("first line\nsecond line"), "first line");
    assert_eq!(brief("  model 'x' not found  "), "model 'x' not found");
    assert_eq!(brief(""), "");
    assert_eq!(brief(&"é".repeat(500)).chars().count(), 200);
    assert_eq!(strip_ansi("\u{1b}[32m✓\u{1b}[0m Logged in"), "✓ Logged in");
    let trace = "\n{\n    at Object.<anonymous> (cli.js:1:1)\nError: Credit balance is too low. Visit the console.\n}";
    assert_eq!(first_words(trace), "Credit balance is too low.");
    println!("1. errors are one line: {:?}", brief(gemini));
}

fn launcher() {
    let dir = std::env::temp_dir().join(format!("meteor-basechat-probe-{}", std::process::id()));
    let pkg = dir.join("node_modules").join("@some").join("cli");
    std::fs::create_dir_all(&pkg).unwrap();
    std::fs::write(pkg.join("cli.js"), "console.log('hi')").unwrap();
    // What npm writes next to a global package's command.
    let npm = "@ECHO off\r\nGOTO start\r\n:find_dp0\r\nSET dp0=%~dp0\r\nEXIT /b\r\n:start\r\nSETLOCAL\r\nCALL :find_dp0\r\n\r\nIF EXIST \"%dp0%\\node.exe\" (\r\n  SET \"_prog=%dp0%\\node.exe\"\r\n) ELSE (\r\n  SET \"_prog=node\"\r\n  SET PATHEXT=%PATHEXT:;.JS;=;%\r\n)\r\n\r\nendLocal & goto #_undefined_# 2>NUL || title %COMSPEC% & \"%_prog%\"  \"%dp0%\\node_modules\\@some\\cli\\cli.js\" %*\r\n";
    assert_eq!(program::launcher_script(npm, &dir), Some(pkg.join("cli.js")));
    let pnpm = "@SETLOCAL\r\n@IF EXIST \"%~dp0\\node.exe\" (\r\n  \"%~dp0\\node.exe\"  \"%~dp0\\node_modules\\@some\\cli\\cli.js\" %*\r\n) ELSE (\r\n  node  \"%~dp0\\node_modules\\@some\\cli\\cli.js\" %*\r\n)\r\n";
    assert_eq!(program::launcher_script(pnpm, &dir), Some(pkg.join("cli.js")));
    // A launcher that starts something else than a script is not ours to start.
    assert_eq!(program::launcher_script("@echo off\r\npowershell -File \"%~dp0\\x.ps1\" %*\r\n", &dir), None);
    let _ = std::fs::remove_dir_all(&dir);
    println!("2. an npm launcher starts as node + its script, never through cmd.exe");
}

fn servers() {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let found = runtime.block_on(scan::scan());
    println!("3. model servers on this computer right now: {}", describe(&found));

    // A pretend Jan on its usual port, when that port is free.
    let Ok(listener) = std::net::TcpListener::bind("127.0.0.1:1337") else {
        println!("   (port 1337 is taken: the pretend server is skipped)");
        return;
    };
    std::thread::spawn(move || {
        if let Ok((mut conn, _)) = listener.accept() {
            let mut request = [0u8; 2048];
            let _ = conn.read(&mut request);
            let body = r#"{"object":"list","data":[{"id":"qwen2.5-coder"},{"id":"nomic-embed-text"},{"id":"llama3.2"}]}"#;
            let _ = write!(conn, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        }
    });
    let found = runtime.block_on(scan::scan());
    let jan = found.iter().find(|s| s.name == "Jan").expect("the pretend server is found");
    assert_eq!((jan.id, jan.url.as_str()), ("custom", "http://127.0.0.1:1337"));
    assert_eq!(jan.models, ["qwen2.5-coder", "llama3.2"], "embedding models are left out");
    println!("   with a pretend server on 1337: {}", describe(&found));
}

fn describe(found: &[scan::Found]) -> String {
    if found.is_empty() {
        return "none".into();
    }
    found.iter().map(|s| format!("{} at {} ({} models)", s.name, s.url, s.models.len())).collect::<Vec<_>>().join(", ")
}

/// The real CLIs and the real gh, found by their files: nothing is started.
fn installed() -> Option<program::Program> {
    let found = cli::installed();
    let names: Vec<String> = found
        .iter()
        .map(|c| format!("{} ({})", c.name, c.program().map(|p| p.exe.display().to_string()).unwrap_or_default()))
        .collect();
    println!("4. installed CLIs (files looked at, nothing started): {}", if names.is_empty() { "none".into() } else { names.join(", ") });
    program::resolve(&["gh"])
}

fn every_cli() {
    for id in ["claude", "codex", "gemini", "cursor", "kiro"] {
        let cli = cli::find(id).unwrap();
        let handle = Handle::default();
        let mut steps = 0;
        let answer = match cli::ask(cli, "You are a test.", HOSTILE, &handle, |_| steps += 1) {
            Ok(answer) => answer,
            Err(cli::Failure::Other(why)) => panic!("{id}: {why}"),
            Err(cli::Failure::Cancelled) => panic!("{id}: cancelled"),
        };
        // The stand-in echoes the message as it received it: whole, in one
        // piece, with every character a shell would have eaten.
        assert!(answer.ends_with(HOSTILE), "{id}: the message arrived changed:\n{answer}");
        let by_argument = answer.contains("one argument after --");
        assert_eq!(by_argument, matches!(id, "cursor" | "kiro"), "{id}");
        if id == "claude" {
            assert!(steps >= 2, "claude's answer grows as it is printed ({steps} steps)");
        }
        println!("   {:<11} answered; the message came as {}", cli.name, if by_argument { "one argument after --" } else { "the input, not an argument" });
    }

    // A CLI that says it is signed out: one line, saying what to do.
    let Err(cli::Failure::Other(why)) = cli::ask(cli::find("codex").unwrap(), "s", "STANDIN-FAIL", &Handle::default(), |_| {}) else {
        panic!("a failing CLI must fail");
    };
    assert_eq!(why, "Codex is not signed in. Sign in from a terminal, then try again.");
    println!("   a signed-out CLI: {why:?}");
}

fn stop_limit_and_cap() {
    let stand_in = program::resolve(&["claude"]).unwrap();
    let spec = |text: &str, timeout_ms: u64, max_output: usize| Spec {
        args: vec!["exec".into(), "-".into()],
        stdin: Some(text.into()),
        cwd: std::env::temp_dir(),
        timeout: Duration::from_millis(timeout_ms),
        max_output,
    };

    // Stop: the run ends well before the stand-in's two minutes.
    let handle = Arc::new(Handle::default());
    let stopper = handle.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(600));
        stopper.cancel();
    });
    let started = Instant::now();
    let run = program::run(&stand_in, &spec("STANDIN-SLOW", 60_000, 1 << 20), &handle, |_| {});
    assert_eq!(run.unwrap_err(), Stop::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(10));
    println!("   Stop ends a run: {:?} after {} ms", Stop::Cancelled, started.elapsed().as_millis());
    // Once stopped, a fallback does not start either.
    assert_eq!(program::run(&stand_in, &spec("hello", 60_000, 1 << 20), &handle, |_| {}).unwrap_err(), Stop::Cancelled);

    let started = Instant::now();
    let run = program::run(&stand_in, &spec("STANDIN-SLOW", 1_000, 1 << 20), &Handle::default(), |_| {});
    assert_eq!(run.unwrap_err(), Stop::TimedOut);
    println!("   the time limit ends a run: {:?} after {} ms", Stop::TimedOut, started.elapsed().as_millis());

    let run = program::run(&stand_in, &spec("STANDIN-BIG", 60_000, 64 * 1024), &Handle::default(), |_| {});
    assert_eq!(run.unwrap_err(), Stop::TooLarge);
    println!("   the size cap ends a run: {:?} (2 MB printed, 64 KB allowed)", Stop::TooLarge);
}

fn task_limits() {
    let folder = std::env::temp_dir().to_string_lossy().to_string();
    assert_eq!(tasks::start("x".into(), "relative/folder", |_, _| {}).unwrap_err(), "That folder does not exist.");
    assert!(tasks::start("x".into(), "Z:\\no\\such\\folder", |_, _| {}).is_err());

    // A task's answer comes back once, as Markdown.
    let (done, answers) = mpsc::channel();
    let tell = done.clone();
    let id = tasks::start(HOSTILE.into(), &folder, move |id, ended| tell.send((id, ended)).unwrap()).unwrap();
    let (ended_id, ended) = answers.recv_timeout(Duration::from_secs(30)).expect("the task ends");
    assert_eq!(ended_id, id);
    assert!(ended.ok && !ended.cancelled);
    assert!(ended.text.starts_with("## Done") && ended.text.ends_with(HOSTILE), "{}", ended.text);
    println!("   a task answers in Markdown, its message whole: {:?}…", &ended.text[..32]);

    // Three at once, the fourth is refused; Cancel ends each of them.
    let slow: Vec<u64> = (0..tasks::MAX_RUNNING)
        .map(|_| {
            let tell = done.clone();
            tasks::start("STANDIN-SLOW".into(), &folder, move |id, ended| tell.send((id, ended)).unwrap()).unwrap()
        })
        .collect();
    let refused = tasks::start("one too many".into(), &folder, |_, _| {}).unwrap_err();
    assert_eq!(refused, "Three tasks are already running. Let one finish first.");
    for id in &slow {
        tasks::cancel(*id);
    }
    for _ in &slow {
        let (id, ended) = answers.recv_timeout(Duration::from_secs(30)).expect("a cancelled task ends");
        assert!(slow.contains(&id) && ended.cancelled && !ended.ok);
    }
    println!("   three tasks at once, the fourth refused ({refused:?}), each cancelled");
}

/// `--gh`: the real GitHub CLI, asked for its login the way the GitHub card
/// asks. Only the length is printed; the token goes nowhere.
#[cfg(debug_assertions)]
fn gh_login(gh: Option<program::Program>) {
    std::env::remove_var(program::STAND_IN);
    let Some(gh) = gh else {
        println!("\ngh: not installed");
        return;
    };
    let spec = Spec {
        args: ["auth", "token", "--hostname", "github.com"].map(String::from).to_vec(),
        stdin: None,
        cwd: std::env::temp_dir(),
        timeout: Duration::from_secs(10),
        max_output: 4096,
    };
    let mut token = String::new();
    let run = program::run(&gh, &spec, &Handle::default(), |line| token = line.trim().to_string());
    match run {
        Ok(output) if output.success && !token.is_empty() => {
            println!("\ngh: {} answered with a login of {} characters (not shown, not kept)", gh.exe.display(), token.len());
        }
        other => println!("\ngh: no login ({other:?})"),
    }
}
