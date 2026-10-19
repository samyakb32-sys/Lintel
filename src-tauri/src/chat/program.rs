// Starting someone else's program for the chat: an agent CLI the user
// installed, or `gh` for its login. Every run goes through `run`, so the rules
// hold for all of them:
//
// - the program is found first and started by its full path;
// - its arguments are an array, never a command line, and no shell is ever in
//   between: an npm `.cmd` launcher is read for the script it starts, and node
//   is started with that script instead of handing the launcher to cmd.exe;
// - a run can be cancelled, has a time limit, and what it prints is capped.
//
// Nothing here decides *when* a program starts: that is always the user's Send
// (chat.rs), or their opening of the GitHub card (github.rs).

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use notch_core::win::{find_on_path, home_dir, no_console};

/// The end of what a program wrote to its error output, kept for the message
/// shown when it fails.
const STDERR_TAIL: usize = 2048;

/// Debug builds only: the full path of a script (or an .exe) that is started in
/// place of every program this module would start, so that nothing real runs,
/// and nobody's plan is spent, while the chat is being worked on. See
/// `src-tauri/examples/basechat-standin.mjs`.
#[cfg(debug_assertions)]
pub const STAND_IN: &str = "METEOR_CLI_STAND_IN";

/// A program that can be started without a shell.
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub exe: PathBuf,
    /// What comes before the arguments: the script, for a program node runs.
    pub lead: Vec<OsString>,
}

/// The first of `names` that is installed: on %PATH%, then where the usual
/// installers put a CLI (an app started from the desktop often lacks those on
/// its PATH). Only files are looked at: nothing is started.
pub fn resolve(names: &[&str]) -> Option<Program> {
    #[cfg(debug_assertions)]
    if let Some(stand_in) = std::env::var_os(STAND_IN) {
        // Set but unusable must not fall through to the real program.
        return startable(Path::new(&stand_in));
    }
    names.iter().flat_map(|name| candidates(name)).find_map(|path| startable(&path))
}

fn candidates(name: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = find_on_path(name).into_iter().collect();
    let dir = |var: &str| std::env::var_os(var).map(PathBuf::from).filter(|p| p.is_absolute());
    let home = home_dir();
    out.push(home.join(".local").join("bin").join(format!("{name}.exe")));
    if let Some(appdata) = dir("APPDATA") {
        out.push(appdata.join("npm").join(format!("{name}.cmd")));
    }
    if let Some(local) = dir("LOCALAPPDATA") {
        out.push(local.join("Volta").join("bin").join(format!("{name}.exe")));
        out.push(local.join("pnpm").join(format!("{name}.cmd")));
    }
    out.push(home.join(".bun").join("bin").join(format!("{name}.exe")));
    out.retain(|p| p.is_absolute() && p.is_file());
    out
}

/// How to start `path` without a shell, or None when there is no such way. An
/// executable starts as it is. A `.cmd` launcher is a batch file, which only
/// cmd.exe can run, and cmd.exe would read the user's words as syntax: the
/// launcher is read for the script it hands to node, and node gets that script.
fn startable(path: &Path) -> Option<Program> {
    let dir = path.parent()?;
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "exe" | "com" => Some(Program { exe: path.to_path_buf(), lead: Vec::new() }),
        "cmd" | "bat" => with_node(dir, launcher_script(&std::fs::read_to_string(path).ok()?, dir)?),
        "js" | "mjs" | "cjs" => with_node(dir, path.to_path_buf()),
        _ => None,
    }
}

/// node next to the launcher (as the launcher itself prefers), else on %PATH%.
fn with_node(dir: &Path, script: PathBuf) -> Option<Program> {
    // A full path to node.exe, never a launcher and never whatever "node"
    // means in the folder we happen to be in.
    let exe = |p: &PathBuf| p.is_absolute() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe"));
    let node = Some(dir.join("node.exe")).filter(|p| p.is_file()).or_else(|| find_on_path("node").filter(exe))?;
    Some(Program { exe: node, lead: vec![script.into_os_string()] })
}

/// The script an npm or pnpm launcher runs: the last `"%dp0%\….js"` on the
/// line that passes the arguments on (`%*`).
/// debt: other launcher formats are not read; such a CLI counts as not installed.
pub fn launcher_script(launcher: &str, dir: &Path) -> Option<PathBuf> {
    let line = launcher.lines().rev().find(|line| line.contains("%*"))?;
    let relative = line
        .split('"')
        .filter_map(|part| part.strip_prefix("%dp0%").or_else(|| part.strip_prefix("%~dp0")))
        .map(|rest| rest.trim_start_matches(['\\', '/']))
        .filter(|rest| {
            matches!(Path::new(rest).extension().and_then(|e| e.to_str()), Some("js" | "mjs" | "cjs"))
        })
        .last()?;
    let script = dir.join(relative);
    script.is_file().then_some(script)
}

// ── Running ───────────────────────────────────────────────────────────────────

pub struct Spec {
    /// One argument each, exactly as the program will see them.
    pub args: Vec<String>,
    /// Written to the program's input, which is then closed.
    pub stdin: Option<String>,
    pub cwd: PathBuf,
    pub timeout: Duration,
    /// The most the program may print before it is ended.
    pub max_output: usize,
}

/// Why a run gave nothing.
#[derive(Debug, Clone, PartialEq)]
pub enum Stop {
    Cancelled,
    TimedOut,
    TooLarge,
    NotStarted(String),
}

#[derive(Debug)]
pub struct Output {
    /// The program's own verdict: it ended with exit code 0.
    pub success: bool,
    /// The end of what it wrote to its error output.
    pub stderr: String,
}

/// The off switch of one run. `cancel` may come from any thread, before the
/// run starts or while it lasts.
#[derive(Default)]
pub struct Handle {
    cancelled: AtomicBool,
    stop: Mutex<Option<Sender<()>>>,
}

impl Handle {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        if let Some(stop) = self.stop.lock().unwrap().as_ref() {
            let _ = stop.send(());
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/// Starts `program` and gives `on_line` each line it prints, until it ends.
/// Blocking: call it off the async runtime. One `handle` may serve several
/// runs in a row (a fallback): once cancelled, none of them starts.
pub fn run(
    program: &Program,
    spec: &Spec,
    handle: &Handle,
    mut on_line: impl FnMut(&str),
) -> Result<Output, Stop> {
    let mut cmd = Command::new(&program.exe);
    cmd.args(&program.lead)
        .args(&spec.args)
        .current_dir(&spec.cwd)
        .stdin(if spec.stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Its own folder first on PATH: a CLI started through node looks for the
    // programs that came with it there.
    if let Some(dir) = program.exe.parent() {
        let mut dirs = vec![dir.to_path_buf()];
        if let Some(path) = std::env::var_os("PATH") {
            dirs.extend(std::env::split_paths(&path));
        }
        if let Ok(joined) = std::env::join_paths(dirs) {
            cmd.env("PATH", joined);
        }
    }
    no_console(&mut cmd);

    let (stop, stopped) = mpsc::channel();
    *handle.stop.lock().unwrap() = Some(stop);
    // Whatever way this returns, the switch is taken back: the watchdog below
    // ends when it is, and a later `cancel` finds nothing to stop.
    let _armed = Armed(handle);
    if handle.is_cancelled() {
        return Err(Stop::Cancelled);
    }

    let mut child = cmd.spawn().map_err(|err| Stop::NotStarted(err.to_string()))?;
    let pid = child.id();
    let (stdin, stdout, stderr) = (child.stdin.take(), child.stdout.take(), child.stderr.take());
    let child = Arc::new(Mutex::new(child));
    let Some(stdout) = stdout else {
        kill(pid, &child);
        return Err(Stop::NotStarted("no output".into()));
    };

    // The input goes in from its own thread: a program that prints before it
    // has read everything would otherwise block on a full pipe, and so would we.
    if let (Some(mut stdin), Some(text)) = (stdin, spec.stdin.clone()) {
        std::thread::spawn(move || {
            let _ = stdin.write_all(text.as_bytes());
        });
    }

    // The error output is drained so its pipe can never fill; its end is kept.
    let tail = Arc::new(Mutex::new(Vec::<u8>::new()));
    let (drained, draining) = mpsc::channel::<()>();
    if let Some(mut stderr) = stderr {
        let tail = tail.clone();
        std::thread::spawn(move || {
            let _keep = drained;
            let mut buf = [0u8; 1024];
            while let Ok(n) = stderr.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let mut kept = tail.lock().unwrap();
                kept.extend_from_slice(&buf[..n]);
                let extra = kept.len().saturating_sub(STDERR_TAIL);
                kept.drain(..extra);
            }
        });
    }

    // The watchdog: Cancel or the time limit ends the program, and it goes
    // away by itself when the run is over (the switch is dropped).
    let why = Arc::new(Mutex::new(None::<Stop>));
    {
        let (child, why, timeout) = (child.clone(), why.clone(), spec.timeout);
        std::thread::spawn(move || {
            let stop = match stopped.recv_timeout(timeout) {
                Ok(()) => Stop::Cancelled,
                Err(RecvTimeoutError::Timeout) => Stop::TimedOut,
                Err(RecvTimeoutError::Disconnected) => return,
            };
            *why.lock().unwrap() = Some(stop);
            kill(pid, &child);
        });
    }

    let mut reader = BufReader::new(stdout);
    let mut line = Vec::new();
    let mut total = 0usize;
    let mut too_large = false;
    loop {
        line.clear();
        let room = spec.max_output.saturating_sub(total) as u64 + 1;
        match (&mut reader).take(room).read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => break,
            Ok(n) => total += n,
        }
        if total > spec.max_output {
            too_large = true;
            kill(pid, &child);
            break;
        }
        on_line(String::from_utf8_lossy(&line).trim_end_matches(['\r', '\n']));
    }
    let status = child.lock().unwrap().wait();
    // A moment for the last of the error output; a helper the program left
    // behind may hold that pipe open, and is not waited for.
    let _ = draining.recv_timeout(Duration::from_millis(300));
    let stderr = String::from_utf8_lossy(&tail.lock().unwrap()).trim().to_string();

    if let Some(stop) = why.lock().unwrap().take() {
        return Err(stop);
    }
    if too_large {
        return Err(Stop::TooLarge);
    }
    Ok(Output { success: status.is_ok_and(|s| s.success()), stderr })
}

struct Armed<'a>(&'a Handle);

impl Drop for Armed<'_> {
    fn drop(&mut self) {
        self.0.stop.lock().unwrap().take();
    }
}

/// Ends the program and everything it started: under node, the answer comes
/// from a child of the process that was started here. `child` is still held
/// while this runs, so Windows cannot have given its id to another process.
/// debt: a helper whose parent has already gone survives; a job object would
/// catch it, and needs a `windows` feature this crate does not declare.
fn kill(pid: u32, child: &Mutex<Child>) {
    let taskkill = std::env::var_os("SystemRoot")
        .map(|root| PathBuf::from(root).join("System32").join("taskkill.exe"))
        .filter(|p| p.is_file());
    if let Some(taskkill) = taskkill {
        let mut cmd = Command::new(taskkill);
        cmd.args(["/T", "/F", "/PID", &pid.to_string()]).stdout(Stdio::null()).stderr(Stdio::null());
        let _ = no_console(&mut cmd).status();
    }
    // The reader may be waiting on it with the lock held: it is dead by now.
    if let Ok(mut child) = child.try_lock() {
        let _ = child.kill();
    }
}
