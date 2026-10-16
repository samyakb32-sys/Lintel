// Proves the hardware helper's pipe, peer check, input validation, fan policy
// and idle exit without elevation and without reading one real sensor:
//
//   dotnet build helper -c Debug
//   cargo run -p svc-hw --example hw-probe
//
// It starts the Debug build of meteor-hw.exe itself, as this user, with the
// made-up sensor source (`--fake`, which a Release build does not have), and
// talks to it the way the app does (svc-hw) and the way an attacker would (a
// raw pipe). It stops nothing it did not start, shows no window, and takes
// about 100 seconds: several of the things it proves are timeouts.
//
// One line per check, `pass` or `FAIL`; the exit code is the number of failures.
// What svc-hw writes to the log goes to a folder of the probe's own, not to
// the app's log.
//
//   cargo run -p svc-hw --example hw-probe -- --list
//
// starts the same Debug helper on the real sensor library instead, still as
// this user (no prompt, so no driver access), and prints what it lists here:
// kinds, types and names, and whether each was read. Nothing else.

use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::Value;
use svc_hw::Fan;

static FAILED: AtomicUsize = AtomicUsize::new(0);
/// %LOCALAPPDATA% as it was before the probe pointed it at its own folder.
static REAL_LOCAL: OnceLock<Option<OsString>> = OnceLock::new();

fn check(what: &str, ok: bool, detail: impl AsRef<str>) {
    if !ok {
        FAILED.fetch_add(1, Ordering::SeqCst);
    }
    println!("{}  {what}: {}", if ok { "pass" } else { "FAIL" }, detail.as_ref());
}

fn sleep(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

/// Opens the pipe the plain way, waiting while it is not there or still busy
/// with whoever was on it before.
fn raw_open(name: &str) -> Option<File> {
    let began = Instant::now();
    loop {
        match OpenOptions::new().read(true).write(true).open(name) {
            Ok(pipe) => return Some(pipe),
            Err(e) if began.elapsed() < Duration::from_secs(10) && matches!(e.raw_os_error(), Some(2 | 231)) => sleep(100),
            Err(_) => return None,
        }
    }
}

/// One line back, or None when the pipe was closed on us.
fn read_line(pipe: &mut File) -> Option<Value> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match pipe.read(&mut byte) {
            Ok(1) if byte[0] == b'\n' => return serde_json::from_slice(&line).ok(),
            Ok(1) => line.push(byte[0]),
            _ => return None,
        }
    }
}

fn ask(pipe: &mut File, line: &[u8]) -> Option<Value> {
    pipe.write_all(line).ok()?;
    pipe.write_all(b"\n").ok()?;
    read_line(pipe)
}

/// `hw-probe --stranger <pipe>`: this same program under another file name,
/// which is all it takes not to be the parent. Exit 0 when the helper would
/// not talk to it, 3 when it answered.
fn stranger(name: &str) -> ! {
    let answered = raw_open(name).and_then(|mut pipe| ask(&mut pipe, br#"{"v":1,"op":"hello"}"#)).is_some();
    std::process::exit(if answered { 3 } else { 0 })
}

struct Helper {
    child: Child,
    /// What it wrote on stderr (a Debug build says what it does).
    log: Arc<Mutex<Vec<String>>>,
    pipe: String,
    token: String,
}

/// `fake`: the file of the made-up sensors; without one, the real library.
fn start(helper: &Path, fake: Option<&Path>, sid: &str, round: u32) -> Helper {
    let token = format!("probe{:08x}{:08x}", std::process::id(), round);
    let me = std::env::current_exe().expect("this program's path");
    let mut command = Command::new(helper);
    command.args(["--parent-pid", &std::process::id().to_string(), "--parent-path"]).arg(&me).args(["--pipe", &token]);
    if let Some(file) = fake {
        command.arg("--fake").arg(file);
    }
    // The helper keeps the environment this program was started in.
    if let Some(local) = REAL_LOCAL.get().and_then(Option::as_ref) {
        command.env("LOCALAPPDATA", local);
    }
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the Debug helper starts unelevated");
    let log = Arc::new(Mutex::new(Vec::new()));
    let (lines, err) = (log.clone(), child.stderr.take().expect("stderr"));
    std::thread::spawn(move || {
        for line in BufReader::new(err).lines().map_while(Result::ok) {
            println!("      helper: {line}");
            lines.lock().unwrap().push(line);
        }
    });
    Helper { child, log, pipe: format!(r"\\.\pipe\meteor-hw-{sid}-{token}"), token }
}

impl Helper {
    fn said(&self, text: &str) -> bool {
        self.log.lock().unwrap().iter().any(|l| l.contains(text))
    }

    /// Waits for the process to end; how long it took, and its exit code.
    fn wait(&mut self, limit: Duration) -> Option<(Duration, Option<i32>)> {
        let began = Instant::now();
        while began.elapsed() < limit {
            if let Ok(Some(status)) = self.child.try_wait() {
                return Some((began.elapsed(), status.code()));
            }
            sleep(100);
        }
        None
    }
}

/// `hw-probe --list`: the real sensor library, as this user, for one reading.
/// Prints what it lists: kinds, types and names, and whether each was read.
fn list(helper: &Path, sid: &str) {
    let mut h = start(helper, None, sid, 9);
    match svc_hw::attach(helper, h.child.id(), &h.token) {
        Ok(handle) => {
            let hi = handle.hello().clone();
            println!("helper {}: elevated {}, PawnIO installed {}, in use {}", hi.version, hi.elevated, hi.driver, hi.in_use);
            let _ = svc_hw::snapshot();
            sleep(1500);
            let snap = svc_hw::snapshot().unwrap_or_default();
            println!("{} sensors (kind, type, hardware, name, read or not):", snap.sensors.len());
            for s in &snap.sensors {
                println!("  {:<12}{:<12}{:<36}{:<34}{}", s.kind, s.ty, s.hardware, s.name, if s.value.is_some() { "read" } else { "not read" });
            }
            for d in &snap.drives {
                println!("  drive       {:<48}[{}] {}", d.name, d.letters, d.status);
            }
            let sum = svc_hw::Summary::of(&snap);
            println!(
                "as the app shows it: processor temperature {}, package power {}, {} temperatures, {} fans, {} fans that can be set, why: {:?}",
                if sum.cpu.is_some() { "read" } else { "not read" },
                if sum.power.is_some() { "read" } else { "not read" },
                sum.temps.len(),
                sum.fans.len(),
                sum.fan.controllable,
                sum.why,
            );
            drop(handle);
        }
        Err(e) => println!("the helper did not answer: {e}"),
    }
    let _ = h.wait(Duration::from_secs(10));
    let _ = h.child.kill();
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--stranger") {
        stranger(args.get(2).map(String::as_str).unwrap_or_default());
    }

    let helper = Path::new(env!("CARGO_MANIFEST_DIR")).join(r"..\..\helper\bin\Debug\net8.0-windows\win-x64\meteor-hw.exe");
    let Ok(helper) = helper.canonicalize() else {
        eprintln!("The Debug helper is not built. Run:  dotnet build helper -c Debug");
        std::process::exit(2);
    };
    let sid = notch_core::win::user_sid().expect("this account's SID");
    let dir: PathBuf = std::env::temp_dir().join(format!("hw-probe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a folder of its own");
    // What svc-hw says in the log is the probe's, not the app's: it goes to this folder.
    let _ = REAL_LOCAL.set(std::env::var_os("LOCALAPPDATA"));
    std::env::set_var("LOCALAPPDATA", &dir);
    let log_now = || -> String {
        let logs = std::fs::read_dir(notch_core::win::local_dir()).into_iter().flatten().flatten();
        logs.filter(|f| f.path().extension().is_some_and(|e| e == "log")).filter_map(|f| std::fs::read_to_string(f.path()).ok()).collect()
    };

    if args.get(1).map(String::as_str) == Some("--list") {
        list(&helper, &sid);
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }

    let cpu_file = dir.join("cpu.txt");
    let fan_file = dir.join("cpu.txt.fan");
    let set_file = dir.join("cpu.txt.set");
    let driver_file = dir.join("cpu.txt.driver");
    let set_cpu = |celsius: u32| std::fs::write(&cpu_file, celsius.to_string()).expect("the made-up temperature");
    let fan_now = || std::fs::read_to_string(&fan_file).unwrap_or_default();
    set_cpu(45);

    let mut h = start(&helper, Some(&cpu_file), &sid, 1);

    // ── The peer check ──
    // The same code from a file that is not the parent's. It goes first: the
    // pipe has one instance, and nobody else may be on it meanwhile.
    let other = dir.join("hw-probe-stranger.exe");
    std::fs::copy(std::env::current_exe().unwrap(), &other).expect("a copy of this program");
    let verdict = Command::new(&other).args(["--stranger", &h.pipe]).status().ok().and_then(|s| s.code());
    sleep(300);
    check(
        "a client whose image path is not the parent's is refused",
        verdict == Some(0) && h.said("refused: image path"),
        format!("got no answer (exit {verdict:?}); the helper said why: {}", h.said("refused: image path")),
    );

    // ── Untrusted input, on a raw pipe, as the parent ──
    let mut raw = raw_open(&h.pipe).expect("the parent may connect");
    let hello = ask(&mut raw, br#"{"v":1,"op":"hello","id":7}"#).unwrap_or_default();
    check("raw hello", hello["ok"] == true && hello["id"] == 7, format!("version {}, id echoed", hello["version"]));
    let bad: [(&str, &[u8]); 15] = [
        ("a reopen with something to open", br#"{"v":1,"op":"reopen","mode":"C:\\x.sys"}"#),
        ("not JSON", b"hello"),
        ("an empty line", b""),
        ("an array", br#"[1,2,3]"#),
        ("an unknown op", br#"{"v":1,"op":"shell.run"}"#),
        ("an unknown field", br#"{"v":1,"op":"hello","cmd":"calc.exe"}"#),
        ("another protocol version", br#"{"v":2,"op":"hello"}"#),
        ("no version", br#"{"op":"hello"}"#),
        ("the same key twice", br#"{"v":1,"op":"hello","op":"quit"}"#),
        ("nesting too deep", br#"{"v":1,"op":"fan.apply","mode":"curve","curve":[[[[[[1]]]]]]}"#),
        ("a level that is text", br#"{"v":1,"op":"fan.apply","mode":"manual","level":"fast"}"#),
        ("a level of 400 %", br#"{"v":1,"op":"fan.apply","mode":"manual","level":400}"#),
        ("a curve that slows down as it heats", br#"{"v":1,"op":"fan.apply","mode":"curve","curve":[{"t":40,"level":80},{"t":70,"level":30}]}"#),
        ("a mode there is not", br#"{"v":1,"op":"fan.apply","mode":"turbo"}"#),
        ("a negative period", br#"{"v":1,"op":"subscribe","ms":-5}"#),
    ];
    let mut refused = Vec::new();
    for (what, line) in bad {
        match ask(&mut raw, line) {
            Some(reply) if reply["ok"] == false => refused.push(format!("{what} → {}", reply["error"])),
            other => check("malformed input is refused", false, format!("{what} got {other:?}")),
        }
    }
    check("malformed input is refused", refused.len() == bad.len(), refused.join("; "));

    // Longer than any request may be, and no line end in sight.
    let _ = raw.write_all(&[b'x'; 5000]);
    let reply = read_line(&mut raw).unwrap_or_default();
    let dropped = read_line(&mut raw).is_none();
    drop(raw);
    let mut raw = raw_open(&h.pipe).expect("the helper still listens");
    let alive = ask(&mut raw, br#"{"v":1,"op":"hello"}"#).is_some_and(|v| v["ok"] == true) && h.child.try_wait().is_ok_and(|s| s.is_none());
    check(
        "an oversized message is refused without a crash",
        reply["error"] == "too large" && dropped && alive,
        format!("5000 bytes → {}, connection closed: {dropped}, helper still answers: {alive}", reply["error"]),
    );
    drop(raw);

    // ── The way the app talks: svc-hw ──
    let handle = match svc_hw::attach(&helper, h.child.id(), &h.token) {
        Ok(handle) => handle,
        Err(e) => {
            check("hello through svc-hw", false, e);
            let _ = h.child.kill();
            std::process::exit(1);
        }
    };
    let hi = handle.hello().clone();
    check(
        "hello through svc-hw (the helper's path and PID checked)",
        hi.version == env!("CARGO_PKG_VERSION") && hi.pid == h.child.id() && hi.fake && !hi.elevated,
        format!("version {}, pid {}, elevated {}, made-up sensors {}", hi.version, hi.pid, hi.elevated, hi.fake),
    );

    let snap = svc_hw::snapshot().unwrap_or_default();
    let age = svc_hw::uptime_ms().saturating_sub(snap.uptime_ms);
    let stamped = snap.sensors.iter().all(|s| s.read_at_uptime_ms == snap.uptime_ms);
    check(
        "snapshot round-trip",
        snap.sensors.len() == 8 && snap.cpu_temp == Some(45.0) && snap.cpu_power == Some(35.5) && age < 3000 && stamped && snap.drives.len() == 1,
        format!(
            "{} sensors, CPU {:?} °C, package {:?} W, read {age} ms ago, drive {:?} health {:?} %",
            snap.sensors.len(),
            snap.cpu_temp,
            snap.cpu_power,
            snap.drives.first().map(|d| d.letters.as_str()),
            snap.drives.first().and_then(|d| d.life),
        ),
    );

    let count = Arc::new(AtomicUsize::new(0));
    let seen = count.clone();
    let sub = svc_hw::subscribe(1000, move |_| {
        seen.fetch_add(1, Ordering::SeqCst);
    });
    sleep(3600);
    let delivered = count.load(Ordering::SeqCst);
    check("subscribe delivers", (2..=4).contains(&delivered), format!("{delivered} readings in 3.6 s at one a second"));
    drop(sub);
    sleep(1500);
    let before = count.load(Ordering::SeqCst);
    sleep(3000);
    let after = count.load(Ordering::SeqCst);
    check("subscribe stops", after == before, format!("{} more in the 3 s after the subscriber left", after - before));

    // ── The fan policy, which is the helper's and not the caller's ──
    let state = svc_hw::fan_apply(&Fan::Manual(5)).unwrap_or_default();
    check(
        "fan.apply below the floor is clamped",
        state.applied == Some(20.0) && state.reason.as_deref() == Some("floor") && fan_now() == "20",
        format!("asked 5 %, applied {:?} % ({:?}), the fan was set to {}", state.applied, state.reason, fan_now()),
    );

    set_cpu(90);
    let began = Instant::now();
    let mut state = svc_hw::FanState::default();
    while began.elapsed() < Duration::from_secs(5) {
        state = svc_hw::snapshot().map(|s| s.fan).unwrap_or_default();
        if state.applied == Some(100.0) {
            break;
        }
        sleep(250);
    }
    check(
        "at 85 °C and over the thermal override takes the fans",
        state.applied == Some(100.0) && state.reason.as_deref() == Some("thermal") && state.mode == "manual" && fan_now() == "100",
        format!("CPU at 90 °C, manual 5 % → {:?} % ({:?}) after {:.1} s", state.applied, state.reason, began.elapsed().as_secs_f32()),
    );
    set_cpu(45);
    let began = Instant::now();
    while began.elapsed() < Duration::from_secs(5) && state.applied != Some(20.0) {
        sleep(250);
        state = svc_hw::snapshot().map(|s| s.fan).unwrap_or_default();
    }
    check("the override lets go when it has cooled", state.applied == Some(20.0), format!("CPU at 45 °C → {:?} %", state.applied));

    let state = svc_hw::fan_apply(&Fan::Curve(vec![(40, 20), (80, 100)])).unwrap_or_default();
    check("a curve is followed", state.mode == "curve" && state.applied == Some(30.0), format!("20 % at 40 °C to 100 % at 80 °C, CPU at 45 °C → {:?} %", state.applied));
    let wrong = svc_hw::fan_apply(&Fan::Manual(200));
    check("svc-hw cannot talk the helper into more than 100 %", wrong.is_err(), format!("{wrong:?}"));
    let state = svc_hw::fan_apply(&Fan::Manual(60)).unwrap_or_default();

    // The app is gone without a word: no quit, no more heartbeats.
    sleep(2500);
    let began = Instant::now();
    handle.detach();
    let mut raw = raw_open(&h.pipe).expect("the helper still listens");
    let mut fan = Value::Null;
    while began.elapsed() < Duration::from_secs(16) {
        fan = ask(&mut raw, br#"{"v":1,"op":"snapshot"}"#).map(|v| v["fan"].clone()).unwrap_or_default();
        if fan["mode"] == "auto" {
            break;
        }
        sleep(250);
    }
    let took = began.elapsed().as_secs_f32();
    check(
        "a missed heartbeat restores automatic control within about 10 s",
        state.applied == Some(60.0) && fan["mode"] == "auto" && fan["reason"] == "heartbeat" && (7.0..=12.5).contains(&took) && fan_now() == "auto",
        format!("manual 60 %, then silence: automatic after {took:.1} s ({}), the fan was set to {}", fan["reason"], fan_now()),
    );

    // Nobody connected, no fan held.
    drop(raw);
    let ended = h.wait(Duration::from_secs(45));
    let took = ended.map(|e| e.0.as_secs_f32()).unwrap_or(f32::NAN);
    check(
        "the helper exits about 30 s after the last connection",
        ended.is_some_and(|e| e.1 == Some(0)) && (27.0..=34.0).contains(&took) && h.said("exit: idle"),
        format!("exited after {took:.1} s with code {:?}", ended.and_then(|e| e.1)),
    );

    let _ = h.child.kill();

    // ── The last handle dropped: quit, at once, fans back first ──
    let mut h = start(&helper, Some(&cpu_file), &sid, 2);
    match svc_hw::attach(&helper, h.child.id(), &h.token) {
        Ok(handle) => {
            let state = svc_hw::fan_apply(&Fan::Manual(50)).unwrap_or_default();
            let held = fan_now();
            drop(handle);
            let ended = h.wait(Duration::from_secs(5));
            check(
                "dropping the last handle ends the helper, fans back first",
                state.applied == Some(50.0) && held == "50" && ended.is_some_and(|e| e.1 == Some(0)) && fan_now() == "auto" && !svc_hw::running(),
                format!("fan at {held} %, then exit after {:.1} s, the fan was set to {}", ended.map(|e| e.0.as_secs_f32()).unwrap_or(f32::NAN), fan_now()),
            );
        }
        Err(e) => check("dropping the last handle ends the helper, fans back first", false, e),
    }
    let _ = h.child.kill();

    // ── A laptop read before its driver was there; then the driver comes ──
    // The made-up hardware is now what the library makes of a processor it
    // cannot read: sensors listed and none read, a power of zero; a drive
    // that lists its two limits as temperatures; and no fan.
    set_cpu(45);
    std::fs::write(&set_file, "bare").expect("the made-up sensor set");
    let logged = log_now().len();
    let mut h = start(&helper, Some(&cpu_file), &sid, 3);
    match svc_hw::attach(&helper, h.child.id(), &h.token) {
        Ok(handle) => {
            // The helper reads at most once a second: a reading taken after this is a new one.
            let fresh = || {
                sleep(1100);
                let snap = svc_hw::snapshot().unwrap_or_default();
                (svc_hw::Summary::of(&snap), snap)
            };
            let began = Instant::now();
            let (sum, snap) = fresh();
            let limits = snap.sensors.iter().find(|s| s.id == "/fakenvme/0/temperature/0").map(|s| (s.warn, s.crit));
            let as_readings = snap.sensors.iter().filter(|s| s.kind == "storage").count();
            set_cpu(62);
            let amber = fresh().0.temps.first().map(|t| (t.value, t.tone));
            set_cpu(70);
            let red = fresh().0.temps.first().map(|t| (t.value, t.tone));
            set_cpu(45);
            check(
                "a drive's limits are not readings: its temperature is listed once, coloured by them",
                sum.temps.len() == 1 && sum.temps[0].value == 58.0 && sum.temps[0].tone.is_empty() && as_readings == 1 && limits == Some((Some(83.0), Some(87.0))) && amber == Some((75.0, "warn")) && red == Some((83.0, "bad")),
                format!("{} temperature listed for the drive ({as_readings} storage sensor sent), limits {limits:?}; at 58 °C plain, at 75 {:?}, at 83 {:?}", sum.temps.len(), amber.map(|a| a.1), red.map(|r| r.1)),
            );
            check(
                "a processor temperature and a power that were not read are missing, not zero, and why is said",
                snap.cpu_temp.is_none() && snap.cpu_power.is_none() && sum.cpu.is_none() && sum.power.is_none() && !snap.in_use && sum.why == "driver",
                format!("CPU {:?} °C, package {:?} W, driver in use {}, why: {:?}", sum.cpu, sum.power, snap.in_use, sum.why),
            );

            // The driver is installed while the helper runs. Nobody asks for anything.
            // (The readings wait until ten seconds after the first ones, and there are
            // three of them, so that the log takes both sensor lists.)
            let _ = std::fs::remove_file(&set_file);
            std::fs::write(&driver_file, "").expect("the made-up driver");
            sleep(11_000u64.saturating_sub(began.elapsed().as_millis() as u64));
            let unasked = fresh().1;
            fresh();
            let (sum, snap) = fresh();
            let same = svc_hw::hello().map(|h| h.pid) == Some(h.child.id()) && h.child.try_wait().is_ok_and(|s| s.is_none());
            check(
                "a driver that comes while the helper runs is taken up by the same process, unasked",
                unasked.in_use && unasked.sensors.len() == 8 && snap.in_use && sum.cpu == Some(45.0) && sum.power == Some(35.5) && sum.fans.len() == 2 && sum.why.is_empty() && same && h.said("reopened: driver"),
                format!("at the next reading: driver in use {}, {} sensors; then CPU {:?} °C, package {:?} W, {} fans, same process: {same}", unasked.in_use, unasked.sensors.len(), sum.cpu, sum.power, sum.fans.len()),
            );

            // The op. What there is to read is decided when the hardware is opened:
            // nothing changes until it is asked for, and it is not done twice in five seconds.
            std::fs::write(&set_file, "bare").expect("the made-up sensor set");
            let before = fresh().1.sensors.len();
            let asked = svc_hw::reopen();
            let (sum, snap) = fresh();
            let again = svc_hw::reopen();
            let same = svc_hw::hello().map(|h| h.pid) == Some(h.child.id());
            check(
                "reopen: the hardware is opened again on request, in the same process, at most every 5 s",
                before == 8 && asked == Ok(true) && snap.sensors.len() == 4 && same && h.said("reopened: request") && again == Err("too soon".into()) && sum.why == "reading",
                format!("{before} sensors before, asked: {asked:?}, {} after, same process: {same}, asked again at once: {again:?}, why: {:?}", snap.sensors.len(), sum.why),
            );

            // With the driver in use and a few readings taken, what is still missing is not coming.
            fresh();
            let (sum, snap) = fresh();
            check(
                "a processor temperature and a fan this PC does not have are said to be not reported",
                snap.in_use && snap.reads >= 3 && sum.cpu.is_none() && sum.power.is_none() && sum.fans.is_empty() && snap.fan.controllable == 0 && sum.why.is_empty(),
                format!("driver in use, {} readings: CPU {:?}, package {:?} W, {} fans, why: {:?} (nothing: not reported)", snap.reads, sum.cpu, sum.power, sum.fans.len(), sum.why),
            );

            let log = log_now();
            let log: Vec<&str> = log.get(logged..).unwrap_or_default().lines().filter(|l| l.contains(" hw: ")).collect();
            let has = |text: &str| log.iter().any(|l| l.contains(text));
            let lists = log.iter().filter(|l| l.contains("hw: sensors read/listed:")).count();
            check(
                "the log has the helper's start, the driver's change and the sensor lists, by kind and count",
                has("hw: helper ") && has("started, pid ") && has(", in use false, made-up sensors") && has("in use true") && lists >= 2 && has("cpu temperature 0/2") && has("cpu temperature 2/2") && has("processor temperature not read") && !log.iter().any(|l| l.contains("Made-up")),
                format!("{} lines, {lists} sensor lists", log.len()),
            );
            for line in &log {
                println!("      log: {}", line.split_once(" hw: ").map(|l| l.1).unwrap_or(line));
            }
            drop(handle);
            let _ = h.wait(Duration::from_secs(5));
        }
        Err(e) => check("a driver that comes while the helper runs is taken up by the same process, unasked", false, e),
    }
    let _ = h.child.kill();

    let _ = std::fs::remove_dir_all(&dir);
    let failed = FAILED.load(Ordering::SeqCst);
    println!("{}", if failed == 0 { "all checks passed".to_owned() } else { format!("{failed} checks FAILED") });
    std::process::exit(failed as i32);
}
