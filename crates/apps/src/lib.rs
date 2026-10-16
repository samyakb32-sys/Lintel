// Apps — a METEOR V1 feature: the updates winget can install, a package search,
// the app's own update check, feedback and the release notes.
//
// Nothing runs before `start`, and after it only one thing does: the timer of
// the automatic check, when a schedule is picked. A scan, a search and an
// install each run winget once, on a click, and end with it. winget is always
// started by its full path with its arguments as an array (svc-apps::run);
// installs and upgrades run one at a time, in the order they were asked for.

pub mod logic;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use logic::{Found, Progress, Update};
use notch_core::i18n::t;
use notch_core::storage::{json_load, json_save_atomic};
use notch_core::{Ctx, Feature, Group, Manifest, Monitor, Use, WindowSpec};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use svc_apps::winget;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};

const ID: &str = "apps";

/// The project's page: releases, the update feed and the issue form hang off it.
// debt: belongs in crates/brand, and moves with the rename.
pub const REPO: &str = "https://github.com/shakibbinkabir/lintel";

const NO_WINGET: &str = "winget is not on this PC. It comes with App Installer, from the Microsoft Store.";

static MANIFEST: Manifest = Manifest {
    id: ID,
    name: "Apps",
    group: Group::Apps,
    default_on: true,
    uses: &[Use::Network],
    shortcuts: &[],
    tray: &[],
    // frequency: "off" | "daily" | "weekly" (the automatic check); notify: tell about what it found;
    // store: list Microsoft Store apps too; rules: [{ id, name, version? }] (skip a version, or the app).
    defaults: r#"{ "frequency": "off", "notify": true, "store": true, "rules": [] }"#,
};

/// What winget is asked to list the updates. It reads; it accepts nothing for the user.
pub const LIST: [&str; 2] = ["upgrade", "--disable-interactivity"];

/// What winget is asked for a search: the community source only, whose packages are reviewed.
pub fn search_args(query: &str) -> [&str; 8] {
    ["search", "--query", query, "--source", "winget", "--count", "30", "--disable-interactivity"]
}

/// winget's arguments for one install or upgrade. What the page shows before
/// the click and what is run are this same array.
pub fn job_args(action: &str, id: &str, source: &str) -> Vec<String> {
    let source = if source == "msstore" { "msstore" } else { "winget" };
    [action, "--id", id, "--exact", "--source", source, "--silent", "--accept-package-agreements", "--accept-source-agreements", "--disable-interactivity"]
        .map(String::from)
        .to_vec()
}

fn command(action: &str, id: &str, source: &str) -> String {
    format!("winget {}", job_args(action, id, source).join(" "))
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

struct Job {
    action: &'static str,
    id: String,
    name: String,
    source: String,
}

/// The install or upgrade that is running.
#[derive(Clone, Serialize)]
struct Op {
    action: &'static str,
    id: String,
    name: String,
    #[serde(flatten)]
    progress: Progress,
}

/// How the last one ended.
#[derive(Clone, Serialize)]
struct Ended {
    action: &'static str,
    name: String,
    ok: bool,
    message: String,
}

/// What outlives the app: when winget was last asked, and what was already announced.
#[derive(Default, Serialize, Deserialize)]
struct Saved {
    last_check: Option<u64>,
    notified: Vec<String>,
}

#[derive(Default)]
struct St {
    /// The last list, before the rules.
    updates: Vec<Update>,
    checked_at: Option<u64>,
    scanning: bool,
    /// Why the list could not be read, when it could not.
    note: String,
    /// The last search: only what it found can be installed.
    found: Vec<Found>,
    queue: VecDeque<Job>,
    working: bool,
    op: Option<Op>,
    ended: Option<Ended>,
    /// Stops the running job.
    cancel: Arc<AtomicBool>,
    /// The tab the window is asked to show.
    tab: String,
}

#[derive(Default)]
struct Inner {
    st: Mutex<St>,
    /// The schedule's thread, by the frequency it was armed for; it ends when the sender goes.
    timer: Mutex<Option<(String, Sender<()>)>>,
    /// Stops the running search: a newer one replaces it.
    search: Mutex<Arc<AtomicBool>>,
    /// The feature is off: a list being read is dropped.
    off: AtomicBool,
}

#[derive(Default)]
pub struct Apps(Arc<Inner>);

fn version(cx: &Ctx) -> String {
    cx.app().package_info().version.to_string()
}

/// "Windows 11 24H2 (build 26100)", for the technical details of a feedback.
fn windows_version() -> String {
    let read = |name: PCWSTR| {
        let mut buf = [0u16; 64];
        let mut bytes = (buf.len() * 2) as u32;
        let key = w!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion");
        let r = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, key, name, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut bytes)) };
        if r == ERROR_SUCCESS { String::from_utf16_lossy(&buf[..(bytes as usize / 2).saturating_sub(1)]) } else { String::new() }
    };
    let build = read(w!("CurrentBuild"));
    let name = if build.parse::<u32>().unwrap_or(0) >= 22000 { "11" } else { "10" };
    format!("Windows {name} {} (build {build})", read(w!("DisplayVersion")))
}

/// The updates the user is shown: the list without what a rule hides and, when
/// the Store is switched off, without its apps.
fn shown(cx: &Ctx, inner: &Inner) -> Vec<Update> {
    let s = cx.settings(ID);
    let store = s["store"].as_bool() != Some(false);
    let st = inner.st.lock().unwrap();
    logic::visible(&st.updates, &logic::rules(&s["rules"])).into_iter().filter(|u| store || u.source != "msstore").collect()
}

fn snapshot(cx: &Ctx, inner: &Inner) -> Value {
    let updates: Vec<Value> = shown(cx, inner)
        .iter()
        .map(|u| {
            let mut row = json!(u);
            row["cmd"] = command("upgrade", &u.id, &u.source).into();
            row
        })
        .collect();
    let st = inner.st.lock().unwrap();
    json!({
        "winget": winget::exe().is_some(),
        "updates": updates,
        "checkedAt": st.checked_at,
        "scanning": st.scanning,
        "note": st.note,
        "op": st.op,
        "waiting": st.queue.len(),
        "ended": st.ended,
        "tab": st.tab,
        "version": version(cx),
    })
}

fn tell(cx: &Ctx, inner: &Inner) {
    cx.emit(ID, "state", snapshot(cx, inner));
}

/// Asks winget what can be updated. One at a time; `automatic` is the
/// schedule's own pass, the only one that announces what it found.
fn scan(cx: &Ctx, inner: &Inner, automatic: bool) {
    {
        let mut st = inner.st.lock().unwrap();
        if st.scanning || inner.off.load(SeqCst) {
            return;
        }
        st.scanning = true;
    }
    tell(cx, inner);
    let result = match winget::exe() {
        None => Err(t(NO_WINGET)),
        Some(exe) => svc_apps::run(&exe, &LIST, Duration::from_secs(120), &inner.off, |_| {}).and_then(|out| match out {
            o if o.cancelled => Err(String::new()),
            o if o.timed_out => Err(t("winget did not answer.")),
            o => logic::parse_updates(o.code, &o.text),
        }),
    };
    let listed = result.is_ok();
    {
        let mut st = inner.st.lock().unwrap();
        st.scanning = false;
        match result {
            Ok(rows) => {
                st.updates = rows;
                st.checked_at = Some(now());
                st.note.clear();
            }
            Err(e) => st.note = if e.is_empty() { t("winget could not list the updates.") } else { e },
        }
    }

    // A pass that failed counts as a pass: the schedule does not ask again every three minutes.
    let path = cx.data_dir(ID).join("state.json");
    let mut saved: Saved = json_load(&path);
    saved.last_check = Some(now());
    let pending = shown(cx, inner);
    let fresh: Vec<&Update> = pending.iter().filter(|u| automatic && listed && !saved.notified.contains(&u.id)).collect();
    if listed {
        // Announced once: an app drops out when it is updated, so a later release is told again.
        saved.notified.retain(|id| pending.iter().any(|u| &u.id == id));
        saved.notified.extend(fresh.iter().map(|u| u.id.clone()));
    }
    let _ = json_save_atomic(&path, &saved);
    tell(cx, inner);
    if !fresh.is_empty() && cx.settings(ID)["notify"].as_bool() != Some(false) && !cx.paused() {
        cx.emit(ID, "found", json!({ "count": fresh.len(), "names": fresh.iter().take(3).map(|u| &u.name).collect::<Vec<_>>() }));
    }
}

/// Runs the queue, one winget at a time, until it is empty.
fn work(cx: &Ctx, inner: &Inner) {
    // Of this run of the queue: how many ran, which did not finish, and why the first of them.
    let (mut ran, mut failed, mut why) = (0, Vec::new(), String::new());
    loop {
        let (job, cancel) = {
            let mut st = inner.st.lock().unwrap();
            let Some(job) = st.queue.pop_front() else {
                st.working = false;
                // After several, the last result alone would hide the ones that failed before it.
                if ran > 1 && !failed.is_empty() {
                    st.ended = Some(Ended { action: "upgrade", name: failed.join(", "), ok: false, message: why });
                    drop(st);
                    tell(cx, inner);
                }
                return;
            };
            st.cancel = Arc::new(AtomicBool::new(false));
            st.op = Some(Op { action: job.action, id: job.id.clone(), name: job.name.clone(), progress: Progress::default() });
            st.ended = None;
            (job, st.cancel.clone())
        };
        tell(cx, inner);

        let args = job_args(job.action, &job.id, &job.source);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let mut progress = Progress::default();
        let mut sent = Instant::now();
        // Quiet for a quarter of an hour is a hang: an installer may take long, but it keeps talking.
        let result = winget::exe().ok_or_else(|| t(NO_WINGET)).and_then(|exe| {
            svc_apps::run(&exe, &args, Duration::from_secs(900), &cancel, |text| {
                progress.feed(text);
                if sent.elapsed() >= Duration::from_millis(150) {
                    sent = Instant::now();
                    if let Some(op) = inner.st.lock().unwrap().op.as_mut() {
                        op.progress = progress.clone();
                    }
                    cx.emit(ID, "progress", &progress);
                }
            })
        });
        let (ok, message) = match result {
            Err(e) => (false, e),
            Ok(o) if o.cancelled => (false, t("Cancelled.")),
            Ok(o) if o.timed_out => (false, t("winget stopped answering and was closed.")),
            Ok(o) if o.code == Some(0) => (true, String::new()),
            Ok(o) => (false, format!("{} ({:#010X})", logic::tail(&o.text), o.code.unwrap_or(-1))),
        };
        notch_core::log::line(format!("apps: {} {} {}", job.action, job.id, if ok { "done" } else { "did not finish" }));
        ran += 1;
        if !ok {
            failed.push(job.name.clone());
            if why.is_empty() {
                why = message.clone();
            }
        }
        {
            let mut st = inner.st.lock().unwrap();
            st.op = None;
            if ok && job.action == "upgrade" {
                st.updates.retain(|u| u.id != job.id);
            }
            st.ended = Some(Ended { action: job.action, name: job.name, ok, message });
        }
        tell(cx, inner);
    }
}

fn enqueue(cx: &Ctx, inner: &Arc<Inner>, jobs: Vec<Job>) {
    let start = {
        let mut st = inner.st.lock().unwrap();
        for job in jobs {
            if st.op.as_ref().is_none_or(|op| op.id != job.id) && !st.queue.iter().any(|j| j.id == job.id) {
                st.queue.push_back(job);
            }
        }
        !st.working && !st.queue.is_empty() && { st.working = true; true }
    };
    if start {
        let (cx, inner) = (cx.clone(), inner.clone());
        std::thread::spawn(move || work(&cx, &inner));
    } else {
        tell(cx, inner);
    }
}

fn search(inner: &Inner, query: &str) -> Result<Value, String> {
    let query = logic::clean_query(query);
    let cancel = Arc::new(AtomicBool::new(false));
    std::mem::replace(&mut *inner.search.lock().unwrap(), cancel.clone()).store(true, SeqCst);
    if query.chars().count() < 2 {
        return Ok(json!({ "results": [] }));
    }
    let Some(exe) = winget::exe() else { return Ok(json!({ "error": t(NO_WINGET) })) };
    let out = svc_apps::run(&exe, &search_args(&query), Duration::from_secs(30), &cancel, |_| {})?;
    if out.cancelled {
        return Ok(json!({ "stale": true }));
    }
    Ok(match logic::parse_search(out.code, &out.text) {
        Err(e) => json!({ "error": if e.is_empty() { t("winget could not search.") } else { e } }),
        Ok(found) => {
            let rows: Vec<Value> = found.iter().map(|f| json!({ "id": f.id, "name": f.name, "version": f.version, "cmd": command("install", &f.id, "winget") })).collect();
            inner.st.lock().unwrap().found = found;
            json!({ "results": rows })
        }
    })
}

/// Reads the release feed and says whether a newer METEOR V1 is out.
// debt: the feed is not signed yet (no signing key, the repository is private and the product
// will be renamed), so an update is only offered as its release page in the browser. Downloading
// and installing by itself waits for the key; nothing here downloads or runs an installer.
pub fn self_check(running: &str) -> Result<Value, String> {
    let feed = format!("{REPO}/releases/latest/download/latest.json");
    let body = tauri::async_runtime::block_on(notch_core::net::bounded_get_following(&feed, Duration::from_secs(20), 256 * 1024))?;
    let feed: Value = serde_json::from_slice(&body).map_err(|_| t("The update feed could not be read."))?;
    let latest = feed["version"].as_str().unwrap_or_default().trim().trim_start_matches(['v', 'V']);
    if latest.is_empty() || latest.len() > 40 {
        return Err(t("The update feed could not be read."));
    }
    Ok(json!({
        "current": running,
        "latest": latest,
        "newer": logic::release_is_newer(latest, running),
        "notes": feed["notes"].as_str().unwrap_or_default().chars().take(4000).collect::<String>(),
        "page": format!("{REPO}/releases/latest"),
    }))
}

impl Apps {
    /// The schedule: one thread that sleeps until the next check is due, and
    /// none at all while the schedule is off.
    fn arm(&self, cx: &Ctx) {
        let frequency = cx.settings(ID)["frequency"].as_str().unwrap_or("off").to_string();
        let mut timer = self.0.timer.lock().unwrap();
        if timer.as_ref().is_some_and(|(armed, _)| *armed == frequency) {
            return;
        }
        *timer = None;
        if logic::next_check(None, &frequency, 0).is_none() {
            return;
        }
        let (tx, rx) = channel::<()>();
        *timer = Some((frequency.clone(), tx));
        let (cx, inner) = (cx.clone(), self.0.clone());
        std::thread::spawn(move || {
            let last = || json_load::<Saved>(&cx.data_dir(ID).join("state.json")).last_check;
            let mut due = logic::next_check(last(), &frequency, now());
            while let Some(at) = due {
                let wait = at.saturating_sub(now());
                if wait == 0 && !cx.paused() {
                    scan(&cx, &inner, true);
                    due = logic::next_check(last(), &frequency, now());
                    continue;
                }
                // ponytail: wakes once an hour to read the clock again, which is what keeps a week
                // right across sleep and a changed clock; a waitable timer with an absolute due
                // time if that wake ever shows up anywhere.
                let nap = if wait == 0 { 600 } else { wait.min(3600) };
                if rx.recv_timeout(Duration::from_secs(nap)) != Err(RecvTimeoutError::Timeout) {
                    return;
                }
            }
        });
    }
}

impl Feature for Apps {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn start(&self, cx: &Ctx) -> Result<(), String> {
        self.0.off.store(false, SeqCst);
        self.arm(cx);
        Ok(())
    }

    fn stop(&self, _cx: &Ctx) {
        self.0.timer.lock().unwrap().take();
        self.0.off.store(true, SeqCst);
        self.0.search.lock().unwrap().store(true, SeqCst);
        // What waits is dropped. An install that is running is left to end by itself:
        // stopping an installer half-way can leave the program broken.
        self.0.st.lock().unwrap().queue.clear();
    }

    fn settings_changed(&self, cx: &Ctx, _settings: &Value) {
        self.arm(cx);
        tell(cx, &self.0); // a rule or the Store switch changes what is listed
    }

    fn call(&self, cx: &Ctx, method: &str, args: Value) -> Result<Value, String> {
        let inner = &self.0;
        let text = |k: &str| args[k].as_str().unwrap_or_default().trim().to_string();
        match method {
            "state" => return Ok(snapshot(cx, inner)),
            "check" => {
                let (cx, inner) = (cx.clone(), inner.clone());
                std::thread::spawn(move || scan(&cx, &inner, false));
            }
            // Only what winget itself listed can be upgraded, by the id it gave.
            "update" => {
                let ids: Vec<String> = serde_json::from_value(args["ids"].clone()).unwrap_or_default();
                let jobs = shown(cx, inner)
                    .into_iter()
                    .filter(|u| ids.contains(&u.id))
                    .map(|u| Job { action: "upgrade", id: u.id, name: u.name, source: u.source })
                    .collect();
                enqueue(cx, inner, jobs);
            }
            // Only what the last search found can be installed, and its id is checked again.
            "install" => {
                let id = text("id");
                let found = inner.st.lock().unwrap().found.iter().find(|f| f.id == id).cloned();
                let Some(found) = found.filter(|f| winget::valid_id(&f.id)) else { return Err("not a package of the last search".into()) };
                enqueue(cx, inner, vec![Job { action: "install", id: found.id, name: found.name, source: "winget".into() }]);
            }
            "cancel" => {
                let mut st = inner.st.lock().unwrap();
                st.queue.clear();
                st.cancel.store(true, SeqCst);
            }
            "search" => return search(inner, &text("q")),
            "selfCheck" => return Ok(self_check(&version(cx)).unwrap_or_else(|e| json!({ "error": e }))),
            "about" => {
                return Ok(json!({
                    "version": version(cx),
                    "windows": windows_version(),
                    "language": notch_core::i18n::current(),
                    "issues": format!("{REPO}/issues/new"),
                }))
            }
            "open" => {
                inner.st.lock().unwrap().tab = text("tab");
                cx.window().open(WindowSpec { label: ID.into(), feature: ID.into(), page: "main".into(), w: 760.0, h: 560.0, frameless: true, monitor: Monitor::Island, ..Default::default() })?;
                cx.emit(ID, "tab", text("tab"));
            }
            "close" => cx.window().close(ID),
            _ => return Err("unknown method".into()),
        }
        Ok(json!({}))
    }

    fn strings(&self) -> &'static str {
        include_str!("../../../src/features/apps/strings.json")
    }
}

pub fn feature() -> Arc<dyn Feature> {
    Arc::new(Apps::default())
}
