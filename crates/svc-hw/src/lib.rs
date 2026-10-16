// Hw — a shared service (docs/ARCHITECTURE.md §8): the app's side of the
// hardware helper `lintel-hw.exe` (§11), the one process of METEOR V1 that runs as
// administrator. Not a feature: the features that need it list this crate in
// their own Cargo.toml.
//
// Nothing here runs by itself. `launch` (one UAC prompt, only ever from a
// click) starts the helper and returns a `Handle`; the helper is told to quit
// when the last `Handle` is dropped. `subscribe` asks the helper for readings and
// the `Sub` it returns ends that on drop: the helper reads the hardware only
// while somebody holds one. A manual fan mode is kept alive by a heartbeat
// from here; if the app stops, freezes or goes away the helper gives the fans
// back to the motherboard by itself.
//
// A PawnIO driver installed while the helper runs is taken up by the helper
// itself, at its next reading (`reopen` asks for it at once): never a second
// prompt. `Snapshot::in_use` says whether it reads through the driver, and
// `Summary::why` why a reading is missing. The log (lintel.log, lines that
// begin `hw:`) is told when a helper starts, when the driver starts or stops
// being used, and when the sensor list changes: kinds and counts only.
//
// The helper is checked before a word is sent to it: the process serving the
// pipe must be the one that was started, running the file that was started.
// What it sends is still read with care: capped, parsed, never executed.
//
// Probe: `cargo run -p svc-hw --example hw-probe` (a Debug helper, unelevated,
// on made-up sensors).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::time::{Duration, Instant};

use notch_core::log;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use windows::core::{w, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_CANCELLED, ERROR_FILE_NOT_FOUND, ERROR_IO_PENDING, ERROR_PIPE_BUSY, ERROR_SUCCESS, HANDLE, WAIT_TIMEOUT,
};
use windows::Win32::Security::Cryptography::{BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_FLAG_OVERLAPPED, FILE_SHARE_MODE, OPEN_EXISTING, SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT,
};
use windows::Win32::System::Pipes::GetNamedPipeServerProcessId;
use windows::Win32::System::Registry::{RegCloseKey, RegOpenKeyExW, HKEY, HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE};
use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::Win32::System::Threading::{
    CreateEventW, GetProcessId, OpenProcess, QueryFullProcessImageNameW, WaitForSingleObject, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

/// The helper's file name, beside the app.
pub const HELPER: &str = "lintel-hw.exe";
/// What `winget` calls the driver the helper reads through.
pub const DRIVER_PACKAGE: &str = "namazso.PawnIO";

/// What the pipe grants this user: read and write, without the right to
/// create another instance of the pipe (FILE_GENERIC_READ | FILE_GENERIC_WRITE
/// & !FILE_CREATE_PIPE_INSTANCE).
const PIPE_ACCESS: u32 = 0x0012_019B;
/// The helper opens the hardware before it opens the pipe: a few seconds on some boards.
const CONNECT: Duration = Duration::from_secs(20);
const ANSWER: Duration = Duration::from_secs(8);
/// Well inside the ten seconds after which the helper gives the fans back.
const BEAT: Duration = Duration::from_secs(2);
/// A reading older than this is not shown as the present.
const STALE_MS: u64 = 10_000;
/// No line of the helper's comes near this.
const MAX_LINE: usize = 1 << 20;

pub const NOT_RUNNING: &str = "The sensors helper is not running.";

// ── What the helper says ──────────────────────────────────────────────────────

/// One sensor. `kind`: the hardware it sits on (`cpu`, `gpu`, `motherboard`,
/// `storage`, `controller`). `ty`: `temperature` (°C), `fan` (RPM), `control`
/// (percent) or `power` (W). `ctl`: this fan can be set from here.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Sensor {
    pub id: String,
    pub hardware: String,
    pub kind: String,
    #[serde(rename = "type")]
    pub ty: String,
    pub name: String,
    pub value: Option<f32>,
    pub min: Option<f32>,
    pub max: Option<f32>,
    /// When it was read, in milliseconds since Windows started (`uptime_ms`).
    pub read_at_uptime_ms: u64,
    pub ctl: bool,
    /// The limits the hardware names for this temperature (a drive's warning
    /// and critical temperatures), where it does. Never readings themselves.
    pub warn: Option<f32>,
    pub crit: Option<f32>,
}

/// What a drive says of its own health (SMART). `status`: `good`, `caution`,
/// `bad` or `unknown`; `life` in percent; `letters` like `"CD"`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Drive {
    pub name: String,
    pub letters: String,
    pub status: String,
    pub life: Option<f32>,
    pub celsius: Option<f32>,
}

/// `mode`: `auto`, `manual` or `curve`. `applied`: what the fans are told now,
/// in percent (not always what was asked). `reason`: why not (`floor`,
/// `thermal`), or why the motherboard has them back (`heartbeat`,
/// `temperature`, `control`).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FanState {
    pub mode: String,
    pub level: i32,
    pub applied: Option<f32>,
    pub reason: Option<String>,
    /// How many fans can be set from here.
    pub controllable: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Snapshot {
    pub uptime_ms: u64,
    /// The helper reads through the PawnIO driver. Without it a processor
    /// lists its sensors and reads none of them.
    pub in_use: bool,
    /// Readings since the helper last opened the hardware.
    pub reads: u32,
    /// The processor's temperature as one number, and its package power.
    pub cpu_temp: Option<f32>,
    pub cpu_power: Option<f32>,
    pub sensors: Vec<Sensor>,
    pub drives: Vec<Drive>,
    pub fan: FanState,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Hello {
    pub version: String,
    pub pid: u32,
    pub elevated: bool,
    /// The PawnIO driver was installed when the helper started, and it read
    /// through it. Both can change while it runs: `Snapshot::in_use` is the present.
    pub driver: bool,
    pub in_use: bool,
    /// Made-up sensors (a Debug helper under the probe).
    pub fake: bool,
}

/// What to ask of the fans. A level is a percentage; a curve is points of
/// (°C, percent), temperatures rising and speeds never falling. The helper
/// has the last word on every number.
#[derive(Clone, Debug, PartialEq)]
pub enum Fan {
    Auto,
    Manual(u8),
    Curve(Vec<(u8, u8)>),
}

/// Milliseconds since Windows started: the clock the helper stamps its readings with.
pub fn uptime_ms() -> u64 {
    unsafe { GetTickCount64() }
}

// ── The pipe ──────────────────────────────────────────────────────────────────

/// A handle this code owns.
struct Owned(HANDLE);
unsafe impl Send for Owned {}
unsafe impl Sync for Owned {}
impl Drop for Owned {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// The client end, opened for overlapped I/O: one thread waits in a read
/// while another writes.
struct Pipe {
    handle: Owned,
    closed: AtomicBool,
}

impl Pipe {
    fn done(&self, started: windows::core::Result<()>, overlapped: &OVERLAPPED) -> Result<usize, String> {
        if let Err(e) = started {
            if e.code() != ERROR_IO_PENDING.to_hresult() {
                return Err(e.message());
            }
        }
        let mut n = 0u32;
        unsafe { GetOverlappedResult(self.handle.0, overlapped, &mut n, true) }.map_err(|e| e.message())?;
        Ok(n as usize)
    }

    fn read(&self, buf: &mut [u8]) -> Result<usize, String> {
        let event = Owned(unsafe { CreateEventW(None, true, false, PCWSTR::null()) }.map_err(|e| e.message())?);
        let mut overlapped = OVERLAPPED { hEvent: event.0, ..Default::default() };
        let started = unsafe { ReadFile(self.handle.0, Some(buf), None, Some(&mut overlapped)) };
        self.done(started, &overlapped)
    }

    fn write(&self, buf: &[u8]) -> Result<(), String> {
        let event = Owned(unsafe { CreateEventW(None, true, false, PCWSTR::null()) }.map_err(|e| e.message())?);
        let mut overlapped = OVERLAPPED { hEvent: event.0, ..Default::default() };
        let started = unsafe { WriteFile(self.handle.0, Some(buf), None, Some(&mut overlapped)) };
        self.done(started, &overlapped).map(|_| ())
    }

    /// Ends the read the reader thread waits in.
    fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        unsafe {
            let _ = CancelIoEx(self.handle.0, None);
        }
    }
}

/// On its own thread for as long as the pipe is open: a line with `ev` is a
/// reading for the subscribers, any other is the answer to a request.
fn reader(pipe: Arc<Pipe>, replies: Sender<Value>) {
    let mut pending: Vec<u8> = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    'pipe: while let Ok(n @ 1..) = pipe.read(&mut chunk) {
        pending.extend_from_slice(&chunk[..n]);
        while let Some(end) = pending.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = pending.drain(..=end).collect();
            let Ok(said) = serde_json::from_slice::<Value>(&line) else { continue };
            if said["ev"] == "snapshot" {
                if let Ok(snap) = serde_json::from_value::<Snapshot>(said) {
                    tell(snap);
                }
            } else if replies.send(said).is_err() {
                break 'pipe;
            }
        }
        if pending.len() > MAX_LINE {
            break;
        }
    }
    pipe.closed.store(true, Ordering::SeqCst);
    svc().latest = None;
    log::line("hw: the helper's pipe closed");
}

// ── One helper ────────────────────────────────────────────────────────────────

struct Conn {
    pipe: Arc<Pipe>,
    /// Held for the length of a request: one question at a time.
    replies: Mutex<Receiver<Value>>,
    next_id: AtomicU64,
    hello: Hello,
    /// Changes with every fan request: the heartbeat of the mode before ends.
    beat: AtomicU64,
    /// Close the pipe without telling the helper to quit.
    detached: AtomicBool,
}

impl Conn {
    fn request(&self, mut ask: Value) -> Result<Value, String> {
        let replies = self.replies.lock().unwrap_or_else(|e| e.into_inner());
        if self.pipe.closed.load(Ordering::SeqCst) {
            return Err(NOT_RUNNING.into());
        }
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        ask["v"] = json!(1);
        ask["id"] = json!(id);
        let mut line = serde_json::to_vec(&ask).map_err(|e| e.to_string())?;
        line.push(b'\n');
        self.pipe.write(&line).map_err(|_| NOT_RUNNING.to_owned())?;
        let until = Instant::now() + ANSWER;
        loop {
            let reply = replies.recv_timeout(until.saturating_duration_since(Instant::now())).map_err(|e| match e {
                RecvTimeoutError::Timeout => "The sensors helper did not answer.".to_owned(),
                RecvTimeoutError::Disconnected => NOT_RUNNING.to_owned(),
            })?;
            // An answer that came after its question was given up on.
            if reply["id"].as_u64() != Some(id) {
                continue;
            }
            return if reply["ok"] == true { Ok(reply) } else { Err(reply["error"].as_str().unwrap_or("refused").to_owned()) };
        }
    }
}

impl Drop for Conn {
    fn drop(&mut self) {
        if !self.detached.load(Ordering::SeqCst) {
            // It gives the fans back and exits; without this it would do both by itself, later.
            let _ = self.request(json!({ "op": "quit" }));
        }
        self.pipe.close();
    }
}

/// The helper runs for as long as one of these is alive (they are clones of
/// one another). Dropping the last tells it to quit.
#[derive(Clone)]
pub struct Handle(Arc<Conn>);

impl Handle {
    pub fn hello(&self) -> &Hello {
        &self.0.hello
    }

    /// Closes the pipe without telling the helper to quit: it gives the fans
    /// back when the heartbeat is ten seconds old and exits by itself thirty
    /// seconds later. What happens when the app dies; the probe does it on purpose.
    pub fn detach(self) {
        self.0.detached.store(true, Ordering::SeqCst);
    }
}

// ── The service ───────────────────────────────────────────────────────────────

type Callback = Arc<dyn Fn(&Snapshot) + Send + Sync>;

#[derive(Default)]
struct Svc {
    conn: Weak<Conn>,
    subs: Vec<(u64, u32, Callback)>,
    next_sub: u64,
    latest: Option<Snapshot>,
    /// What the log was last told of the driver and of the sensor list, and when of the list.
    logged_in_use: Option<bool>,
    logged_list: String,
    logged_list_at: Option<Instant>,
}

fn svc() -> MutexGuard<'static, Svc> {
    static SVC: OnceLock<Mutex<Svc>> = OnceLock::new();
    SVC.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
}

fn current() -> Result<Arc<Conn>, String> {
    let conn = svc().conn.upgrade();
    conn.filter(|c| !c.pipe.closed.load(Ordering::SeqCst)).ok_or_else(|| NOT_RUNNING.to_owned())
}

/// The sensor list as the log tells it: of each kind, how many were read out
/// of how many are listed. Kinds and counts only: no name or number of a device.
fn listed(snap: &Snapshot) -> String {
    let mut kinds: BTreeMap<(&str, &str), (u32, u32)> = BTreeMap::new();
    for s in &snap.sensors {
        let n = kinds.entry((&s.kind, &s.ty)).or_default();
        n.0 += s.value.is_some() as u32;
        n.1 += 1;
    }
    let mut said: Vec<String> = kinds.iter().map(|((kind, ty), (read, all))| format!("{kind} {ty} {read}/{all}")).collect();
    said.push(format!("drives {}", snap.drives.len()));
    said.push(format!("fans that can be set {}", snap.fan.controllable));
    said.push(format!("processor temperature {}", if snap.cpu_temp.is_some() { "read" } else { "not read" }));
    said.push(format!("package power {}", if snap.cpu_power.is_some() { "read" } else { "not read" }));
    said.join(", ").chars().take(400).collect()
}

/// `snap` is the last reading from now on. The log is told when the driver
/// starts or stops being read through, and when the sensor list is another:
/// once the helper has read a few times since it opened the hardware (the
/// first reading is not all of them), and at most every ten seconds (a sensor
/// that comes and goes fills no log).
fn keep(svc: &mut Svc, snap: &Snapshot) {
    if svc.logged_in_use != Some(snap.in_use) {
        svc.logged_in_use = Some(snap.in_use);
        log::line(format!("hw: PawnIO driver installed {}, in use {}", driver_installed(), snap.in_use));
    }
    let list = listed(snap);
    if snap.reads >= SETTLED && list != svc.logged_list && svc.logged_list_at.is_none_or(|at| at.elapsed() >= Duration::from_secs(10)) {
        log::line(format!("hw: sensors read/listed: {list}"));
        svc.logged_list = list;
        svc.logged_list_at = Some(Instant::now());
    }
    svc.latest = Some(snap.clone());
}

fn tell(snap: Snapshot) {
    let subs: Vec<Callback> = {
        let mut svc = svc();
        keep(&mut svc, &snap);
        svc.subs.iter().map(|s| s.2.clone()).collect()
    };
    for f in subs {
        f(&snap);
    }
}

/// Tells the helper the period the subscribers there are now want, or to stop
/// reading. On a thread of its own: it waits for the helper's answer, which
/// the caller may be the one thread able to read.
fn resubscribe() {
    static ORDER: Mutex<()> = Mutex::new(());
    std::thread::spawn(|| {
        let _one = ORDER.lock().unwrap_or_else(|e| e.into_inner());
        let ms = svc().subs.iter().map(|s| s.1.max(1000)).min().unwrap_or(0);
        if let Ok(conn) = current() {
            let _ = conn.request(json!({ "op": "subscribe", "ms": ms }));
        }
    });
}

/// Ends the readings when dropped.
pub struct Sub(u64);

/// `f` is called with every reading, about every `ms` (a second at least), on
/// the service's thread, for as long as the `Sub` lives and a helper runs.
/// With no helper it waits for one. Do the slow work elsewhere.
pub fn subscribe(ms: u32, f: impl Fn(&Snapshot) + Send + Sync + 'static) -> Sub {
    let id = {
        let mut svc = svc();
        svc.next_sub += 1;
        let id = svc.next_sub;
        svc.subs.push((id, ms, Arc::new(f)));
        id
    };
    resubscribe();
    Sub(id)
}

impl Drop for Sub {
    fn drop(&mut self) {
        svc().subs.retain(|s| s.0 != self.0);
        resubscribe();
    }
}

/// The helper answers on its pipe.
pub fn running() -> bool {
    current().is_ok()
}

/// What the running helper said of itself.
pub fn hello() -> Option<Hello> {
    current().ok().map(|c| c.hello.clone())
}

/// Reads the sensors now (the helper reads the hardware once a second at most).
pub fn snapshot() -> Result<Snapshot, String> {
    let reply = current()?.request(json!({ "op": "snapshot" }))?;
    let snap: Snapshot = serde_json::from_value(reply).map_err(|e| e.to_string())?;
    keep(&mut svc(), &snap);
    Ok(snap)
}

/// Asks the helper to close the hardware and open it again, in the process
/// that already runs: no Windows prompt. It is how a PawnIO driver installed
/// after the helper started is taken up at once; the helper also does it by
/// itself at its next reading. `Ok`: whether it reads through the driver now.
/// The helper refuses more than one every five seconds (`too soon`).
pub fn reopen() -> Result<bool, String> {
    Ok(current()?.request(json!({ "op": "reopen" }))?["inUse"] == true)
}

/// The last reading, if it is still the present.
pub fn latest() -> Option<Snapshot> {
    let snap = svc().latest.clone()?;
    (uptime_ms().saturating_sub(snap.uptime_ms) < STALE_MS).then_some(snap)
}

fn fan_state(reply: Value) -> Result<FanState, String> {
    serde_json::from_value(reply["fan"].clone()).map_err(|e| e.to_string())
}

/// Asks for a fan mode. While it is not `Auto` a heartbeat goes to the helper
/// every two seconds, from a thread that ends with the mode or the helper.
/// Errors: the helper's own words (`no controllable fan`, `bad field`).
pub fn fan_apply(fan: &Fan) -> Result<FanState, String> {
    let conn = current()?;
    let ask = match fan {
        Fan::Auto => json!({ "op": "fan.apply", "mode": "auto" }),
        Fan::Manual(level) => json!({ "op": "fan.apply", "mode": "manual", "level": level }),
        Fan::Curve(points) => {
            let curve: Vec<Value> = points.iter().map(|(t, level)| json!({ "t": t, "level": level })).collect();
            json!({ "op": "fan.apply", "mode": "curve", "curve": curve })
        }
    };
    let turn = conn.beat.fetch_add(1, Ordering::SeqCst) + 1;
    let state = fan_state(conn.request(ask)?)?;
    if state.mode != "auto" {
        let conn = Arc::downgrade(&conn);
        std::thread::spawn(move || loop {
            std::thread::sleep(BEAT);
            let Some(conn) = conn.upgrade() else { return };
            if conn.beat.load(Ordering::SeqCst) != turn {
                return;
            }
            // The helper may have given the fans back itself: then there is nothing to keep alive.
            match conn.request(json!({ "op": "fan.heartbeat" })) {
                Ok(reply) if reply["fan"]["mode"] != "auto" => {}
                _ => return,
            }
        });
    }
    Ok(state)
}

/// The fans go back to the motherboard.
pub fn fan_restore() -> Result<FanState, String> {
    fan_apply(&Fan::Auto)
}

/// The fan mode as the helper has it now.
pub fn fan_state_now() -> Result<FanState, String> {
    Ok(snapshot()?.fan)
}

// ── What the System section shows ─────────────────────────────────────────────

#[derive(Clone, Debug, Serialize)]
pub struct Named {
    pub id: String,
    pub name: String,
    pub hardware: String,
    pub value: f32,
    pub ctl: bool,
    /// How a temperature stands against the limits its hardware names:
    /// `warn` (amber), `bad` (red), or nothing.
    pub tone: &'static str,
}

/// Amber from ten degrees under the warning limit, red from five under the
/// critical one. A limit of zero is a drive that names none.
pub fn tone(value: f32, warn: Option<f32>, crit: Option<f32>) -> &'static str {
    let named = |limit: Option<f32>| limit.filter(|l| l.is_finite() && *l > 0.0);
    match (named(warn), named(crit)) {
        (_, Some(crit)) if value >= crit - 5.0 => "bad",
        (Some(warn), _) if value >= warn - 10.0 => "warn",
        (None, Some(crit)) if value >= crit - 15.0 => "warn",
        _ => "",
    }
}

/// The helper has read this often since it opened the hardware before a
/// sensor that is still silent is taken to be one this PC does not have.
const SETTLED: u32 = 3;

/// The readings a person looks at, out of everything the helper reads.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Summary {
    /// The processor, °C.
    pub cpu: Option<f32>,
    /// The processor's package, W. Never zero: that is no reading.
    pub power: Option<f32>,
    /// Every temperature, °C. Limits are not among them: they are `tone`.
    pub temps: Vec<Named>,
    /// Every fan, RPM.
    pub fans: Vec<Named>,
    pub drives: Vec<Drive>,
    pub fan: FanState,
    /// Why a reading may be missing: `driver` (the helper does not read
    /// through PawnIO), `reading` (it opened the hardware a moment ago), or
    /// nothing (what is missing now, this PC does not report).
    pub why: &'static str,
}

impl Summary {
    pub fn of(snap: &Snapshot) -> Summary {
        let pick = |ty: &str, low: f32, high: f32| -> Vec<Named> {
            snap.sensors
                .iter()
                .filter(|s| s.ty == ty)
                .filter_map(|s| {
                    let value = s.value.filter(|v| v.is_finite() && *v >= low && *v <= high)?;
                    Some(Named { id: s.id.clone(), name: s.name.clone(), hardware: s.hardware.clone(), value, ctl: s.ctl, tone: tone(value, s.warn, s.crit) })
                })
                .collect()
        };
        Summary {
            cpu: snap.cpu_temp.filter(|v| v.is_finite()),
            power: snap.cpu_power.filter(|v| v.is_finite() && *v > 0.0),
            temps: pick("temperature", 1.0, 125.0),
            fans: pick("fan", 0.0, 20_000.0),
            drives: snap.drives.clone(),
            fan: snap.fan.clone(),
            why: if !snap.in_use { "driver" } else if snap.reads < SETTLED { "reading" } else { "" },
        }
    }
}

/// The last reading as the System section shows it; `None` with no helper, or
/// when the reading is too old to be shown as the present.
pub fn summary() -> Option<Summary> {
    latest().map(|snap| Summary::of(&snap))
}

// ── Starting the helper ───────────────────────────────────────────────────────

/// The PawnIO driver is installed. Without it the helper still reads graphics
/// cards and drives; processor temperatures and motherboard fans need it.
pub fn driver_installed() -> bool {
    let mut key = HKEY::default();
    let found = unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, w!(r"SYSTEM\CurrentControlSet\Services\PawnIO"), None, KEY_QUERY_VALUE, &mut key) } == ERROR_SUCCESS;
    if found {
        unsafe {
            let _ = RegCloseKey(key);
        }
    }
    found
}

/// Where the helper is: beside the app.
pub fn helper_path() -> PathBuf {
    let beside = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join(HELPER))).unwrap_or_else(|| PathBuf::from(HELPER));
    // A development build has no installer to put the helper beside it: it
    // takes the one `dotnet publish helper -c Release -o helper/bin/publish` leaves.
    #[cfg(debug_assertions)]
    if !beside.is_file() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2).unwrap_or(Path::new("."));
        let published = root.join("helper").join("bin").join("publish").join(HELPER);
        if published.is_file() {
            return published;
        }
    }
    beside
}

fn same_file(a: &Path, b: &Path) -> bool {
    let full = |p: &Path| std::fs::canonicalize(p).ok().map(|p| p.to_string_lossy().to_lowercase());
    matches!((full(a), full(b)), (Some(a), Some(b)) if a == b)
}

fn image_of(process: HANDLE) -> Option<PathBuf> {
    let mut buf = vec![0u16; 32768];
    let mut len = buf.len() as u32;
    unsafe { QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len) }.ok()?;
    Some(PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])))
}

fn exited(process: &Owned) -> bool {
    unsafe { WaitForSingleObject(process.0, 0) != WAIT_TIMEOUT }
}

/// Starts `file` as administrator: Windows asks the user first.
fn runas(file: &Path, args: &str) -> Result<Owned, String> {
    let (file, args) = (HSTRING::from(file.as_os_str()), HSTRING::from(args));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(args.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut info) }.map_err(|e| {
        if e.code() == ERROR_CANCELLED.to_hresult() {
            "Windows was not given permission, so the helper was not started.".to_owned()
        } else {
            e.message()
        }
    })?;
    if info.hProcess.is_invalid() {
        return Err("The helper could not be started.".into());
    }
    Ok(Owned(info.hProcess))
}

fn me() -> Result<String, String> {
    let path = std::env::current_exe().map_err(|e| e.to_string())?;
    let text = path.to_str().filter(|p| !p.contains('"')).ok_or("This program's own path cannot be passed on.")?;
    Ok(text.to_owned())
}

/// Starts the helper as administrator and connects to it. **Windows shows its
/// permission prompt: call this only from an explicit user action.**
pub fn launch() -> Result<Handle, String> {
    if let Ok(conn) = current() {
        return Ok(Handle(conn));
    }
    let helper = helper_path();
    if !helper.is_file() {
        return Err("The sensors helper is missing. Install the app again.".into());
    }
    let mut random = [0u8; 16];
    if unsafe { BCryptGenRandom(None, &mut random, BCRYPT_USE_SYSTEM_PREFERRED_RNG) }.is_err() {
        return Err("The helper could not be started.".into());
    }
    let token: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let args = format!("--parent-pid {} --parent-path \"{}\" --pipe {token}", std::process::id(), me()?);
    let started = runas(&helper, &args).and_then(|process| connect(&helper, &process, &token));
    if let Err(e) = &started {
        log::line(format!("hw: the helper did not start: {e}"));
    }
    started
}

/// Connects to a helper that was started with `--pipe <token>` and checks it:
/// the process serving the pipe must be `pid` and must be running `helper`.
/// `launch` does this itself; the probe calls it for a helper it started
/// unelevated.
pub fn attach(helper: &Path, pid: u32, token: &str) -> Result<Handle, String> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE, false, pid) }.map_err(|e| e.message())?;
    connect(helper, &Owned(process), token)
}

fn connect(helper: &Path, process: &Owned, token: &str) -> Result<Handle, String> {
    let sid = notch_core::win::user_sid().ok_or("This account's identity could not be read.")?;
    let name = HSTRING::from(format!(r"\\.\pipe\lintel-hw-{sid}-{token}"));
    let began = Instant::now();
    let handle = loop {
        // Identification only: the helper may learn who we are, never act as us.
        let flags = FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION;
        match unsafe { CreateFileW(&name, PIPE_ACCESS, FILE_SHARE_MODE(0), None, OPEN_EXISTING, flags, None) } {
            Ok(handle) => break Owned(handle),
            // Not there yet, or still letting go of somebody it refused.
            Err(e) if e.code() == ERROR_FILE_NOT_FOUND.to_hresult() || e.code() == ERROR_PIPE_BUSY.to_hresult() => {}
            Err(e) => return Err(e.message()),
        }
        if exited(process) {
            return Err("The sensors helper stopped as it started.".into());
        }
        if began.elapsed() > CONNECT {
            return Err("The sensors helper did not start in time.".into());
        }
        std::thread::sleep(Duration::from_millis(150));
    };

    // Who is on the other end, before anything is said to it.
    let mut server = 0u32;
    unsafe { GetNamedPipeServerProcessId(handle.0, &mut server) }.map_err(|e| e.message())?;
    let started = unsafe { GetProcessId(process.0) } == server;
    let image = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, server) }.ok().map(Owned).and_then(|p| image_of(p.0));
    if !started || !image.is_some_and(|image| same_file(&image, helper)) {
        return Err("Something other than the sensors helper answered. Nothing was sent to it.".into());
    }

    let pipe = Arc::new(Pipe { handle, closed: AtomicBool::new(false) });
    let (tx, rx) = mpsc::channel();
    let theirs = pipe.clone();
    std::thread::Builder::new().name("hw-pipe".into()).spawn(move || reader(theirs, tx)).map_err(|e| e.to_string())?;
    let mut conn = Conn {
        pipe,
        replies: Mutex::new(rx),
        next_id: AtomicU64::new(1),
        hello: Hello::default(),
        beat: AtomicU64::new(0),
        detached: AtomicBool::new(false),
    };
    conn.hello = serde_json::from_value(conn.request(json!({ "op": "hello" }))?).map_err(|e| e.to_string())?;
    let hi = &conn.hello;
    log::line(format!(
        "hw: helper {} started, pid {}, elevated {}, PawnIO driver installed {}, in use {}{}",
        hi.version.chars().take(16).collect::<String>(),
        hi.pid,
        hi.elevated,
        hi.driver,
        hi.in_use,
        if hi.fake { ", made-up sensors" } else { "" },
    ));
    let conn = Arc::new(conn);
    {
        let mut svc = svc();
        svc.conn = Arc::downgrade(&conn);
        // The log has this helper's driver state; its sensor list comes with its first reading.
        svc.logged_in_use = Some(conn.hello.in_use);
        svc.logged_list.clear();
        svc.logged_list_at = None;
    }
    // Whoever subscribed while there was no helper is served from now on.
    resubscribe();
    Ok(Handle(conn))
}
