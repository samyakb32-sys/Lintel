// Agent usage — a METEOR V1 feature. Reads the local Claude Code and Codex logs (read-only, counters only),
// tailing them from a change notification on each log folder, never from a poll. Nothing runs before
// `start`; `stop` cancels the watches and the worker exits.

pub mod usage;

use notch_core::{Ctx, Feature, Group, Manifest};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use usage::{now_ms, Store};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::IO::CancelIoEx;
use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL};

static MANIFEST: Manifest = Manifest {
    id: "agent-usage",
    name: "Agent usage",
    group: Group::Agents,
    default_on: true,
    uses: &[],
    shortcuts: &[],
    tray: &[],
    // finishNotice: tell when a turn ends; minSeconds: the shortest turn that counts; liveWork: show running turns
    defaults: r#"{ "finishNotice": true, "minSeconds": 120, "liveWork": true }"#,
};

struct Runner {
    stopped: Arc<AtomicBool>,
    handles: Vec<isize>,
    tx: Option<Sender<()>>,
    /// This run's counters. The worker locks them for one log at a time, so nothing that asks
    /// (the page, `stop`, quitting) ever waits for the whole history, which takes a minute or more
    /// the first time. A run that is stopped leaves its store to its worker and is forgotten.
    store: Arc<Mutex<Store>>,
    /// The first full read is done: until then the page is told nothing rather than a part.
    ready: Arc<AtomicBool>,
}

#[derive(Default)]
pub struct AgentUsage {
    run: Mutex<Option<Runner>>,
}

/// One recursive ReadDirectoryChangesW watch; every notification pokes the worker.
fn watch(dir: PathBuf, tx: Sender<()>, stopped: Arc<AtomicBool>) -> Result<isize, String> {
    let wide: Vec<u16> = dir.as_os_str().to_string_lossy().encode_utf16().chain(Some(0)).collect();
    let h = unsafe {
        CreateFileW(PCWSTR(wide.as_ptr()), FILE_LIST_DIRECTORY.0, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, None, OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS, None)
    }
    .map_err(|e| format!("cannot watch: {e}"))?;
    let handle = h.0 as isize;
    std::thread::spawn(move || {
        let h = HANDLE(handle as *mut _);
        let mut buf = vec![0u8; 16 * 1024];
        while !stopped.load(Ordering::SeqCst) {
            let mut got = 0u32;
            let r = unsafe { ReadDirectoryChangesW(h, buf.as_mut_ptr() as *mut _, buf.len() as u32, true, FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_SIZE | FILE_NOTIFY_CHANGE_LAST_WRITE, Some(&mut got), None, None) };
            if r.is_err() || tx.send(()).is_err() {
                break;
            }
        }
        unsafe {
            let _ = CloseHandle(h);
        }
    });
    Ok(handle)
}

impl Feature for AgentUsage {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn start(&self, cx: &Ctx) -> Result<(), String> {
        let mut run = self.run.lock().unwrap();
        if run.is_some() {
            return Ok(());
        }
        let roots = usage::roots();
        let stopped = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel::<()>();
        let handles = roots.iter().filter_map(|(_, p)| watch(p.clone(), tx.clone(), stopped.clone()).ok()).collect();
        let (store, ready) = (Arc::new(Mutex::new(Store::default())), Arc::new(AtomicBool::new(false)));
        *run = Some(Runner { stopped: stopped.clone(), handles, tx: Some(tx), store: store.clone(), ready: ready.clone() });

        let cx = cx.clone();
        std::thread::spawn(move || {
            unsafe {
                let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
            }
            let mut first = true;
            loop {
                let now = now_ms();
                let mut ended = vec![];
                store.lock().unwrap().found(&roots);
                // debt: one very large log is still read in one go; nothing is kept between launches.
                for (prov, log) in usage::logs(&roots, now) {
                    if stopped.load(Ordering::SeqCst) {
                        return;
                    }
                    store.lock().unwrap().read(prov, &log, now, !first, &mut ended);
                }
                first = false;
                ready.store(true, Ordering::SeqCst);
                let s = cx.settings("agent-usage");
                if s["finishNotice"].as_bool() != Some(false) {
                    let min = s["minSeconds"].as_i64().unwrap_or(120).clamp(0, 3600) * 1000;
                    for e in ended.into_iter().filter(|e| e.duration_ms >= min && now_ms() - e.at < 300_000) {
                        cx.emit("agent-usage", "finished", json!({ "provider": e.prov.id(), "name": e.prov.name(), "project": e.project, "session": e.session, "durationMs": e.duration_ms }));
                    }
                }
                let summary = store.lock().unwrap().summary(now_ms());
                if stopped.load(Ordering::SeqCst) {
                    return;
                }
                cx.emit("agent-usage", "usage", summary);
                // idle: block until a log changes, then let a burst of writes settle
                if rx.recv().is_err() {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(1000));
                while rx.try_recv().is_ok() {}
            }
        });
        Ok(())
    }

    fn stop(&self, _cx: &Ctx) {
        if let Some(mut r) = self.run.lock().unwrap().take() {
            r.stopped.store(true, Ordering::SeqCst);
            for h in &r.handles {
                unsafe {
                    let _ = CancelIoEx(HANDLE(*h as *mut _), None);
                }
            }
            r.tx.take(); // the worker exits once the watchers have let go of their senders
        }
    }

    fn call(&self, _cx: &Ctx, method: &str, _args: Value) -> Result<Value, String> {
        match method {
            "state" => {
                let run = self.run.lock().unwrap().as_ref().map(|r| (r.store.clone(), r.ready.load(Ordering::SeqCst)));
                // Still reading for the first time: the `usage` event brings the totals when they are whole.
                let Some((store, true)) = run else { return Ok(Value::Null) };
                let v = store.lock().unwrap().summary(now_ms());
                Ok(v)
            }
            _ => Err("unknown method".into()),
        }
    }

    fn strings(&self) -> &'static str {
        include_str!("../../../src/features/agent-usage/strings.json")
    }
}

pub fn feature() -> Arc<dyn Feature> {
    Arc::new(AgentUsage::default())
}
