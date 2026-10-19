// Hardware sensors — a METEOR V1 feature. The contract is docs/ARCHITECTURE.md §2:
// nothing runs before `start`, everything is gone after `stop`, and every
// command of the feature arrives through `call`.
//
// The feature owns the one `svc_hw::Handle` of the app: the elevated helper
// `meteor-hw.exe` (docs/ARCHITECTURE.md §11) runs for as long as this holds it.
// Turning the feature on starts nothing: the helper is launched only from the
// switch on the Settings page (`launch`, one Windows prompt each time).
// Nothing is installed from here by itself: the driver offer is a button, and
// it runs winget only when pressed. "Start automatically" is not built: see
// the note at the end of helper/Program.cs.
//
// Calls: `status` → Status, `launch`, `quit`, `installDriver`,
// `fan { mode, level?, curve? }` → { fan } or { error }.
// Events: `changed` (ask for the status again).

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use notch_core::i18n::t;
use notch_core::{Ctx, Feature, Group, Manifest, Use};
use serde_json::{json, Value};
use svc_hw::Fan;

const ID: &str = "hw-helper";

static MANIFEST: Manifest = Manifest {
    id: ID,
    name: "Hardware sensors",
    group: Group::System,
    // Opt-in: it asks for administrator rights.
    default_on: true,
    uses: &[Use::Elevation],
    shortcuts: &[],
    tray: &[],
    // consent: the explanation was read and accepted once. pins: sensor ids shown in the System section.
    // fanLevel: the manual speed last chosen. fanCurve: percent at 40, 50, 60, 70 and 80 °C.
    defaults: r#"{ "consent": false, "pins": [], "fanLevel": 60, "fanCurve": [20, 30, 50, 75, 100] }"#,
};

/// The temperatures the five speeds of the curve stand at, in °C.
const CURVE_AT: [u8; 5] = [40, 50, 60, 70, 80];

#[derive(Default)]
pub struct HwHelper {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    st: Mutex<St>,
}

#[derive(Default)]
struct St {
    on: bool,
    /// The helper runs while this is held.
    handle: Option<svc_hw::Handle>,
    /// Why the last launch did not work, in the user's language.
    error: Option<String>,
    /// winget is installing the driver.
    installing: bool,
    /// How that ended.
    install_note: Option<String>,
}

/// The helper's short refusals, as sentences.
fn fan_error(code: &str) -> String {
    match code {
        "no controllable fan" => t("No fan of this PC can be controlled from here."),
        "bad field" => t("The helper refused that fan setting."),
        other => t(other),
    }
}

impl Inner {
    fn status(&self) -> Value {
        // A helper that went away (closed by Windows, by the user, by itself) is let go of.
        let gone = {
            let mut st = self.st.lock().unwrap();
            if st.handle.is_some() && !svc_hw::running() { st.handle.take() } else { None }
        };
        drop(gone);
        // Asking is also what lets the helper notice a driver that came while it runs.
        let snap = svc_hw::snapshot().ok();
        let st = self.st.lock().unwrap();
        let running = st.handle.is_some() && svc_hw::running();
        let snap = snap.filter(|_| running);
        // What the registry says now, not what the helper found when it started.
        let driver = svc_hw::driver_installed();
        json!({
            "running": running,
            "error": st.error,
            "driver": driver,
            // The running helper reads through it.
            "inUse": snap.as_ref().is_some_and(|s| s.in_use),
            "helperFound": svc_hw::helper_path().is_file(),
            "winget": svc_apps::winget::exe().is_some(),
            "installing": st.installing,
            // How an install that did not work ended; with the driver there, nothing is left to say.
            "installNote": st.install_note.as_ref().filter(|_| !driver),
            "snap": snap.as_ref().map(svc_hw::Summary::of),
        })
    }

    /// Keeps `handle` if the feature is still on; otherwise the helper is told to quit.
    fn keep(&self, launched: Result<svc_hw::Handle, String>) {
        let mut st = self.st.lock().unwrap();
        let unused = match launched {
            Ok(handle) if st.on => {
                st.error = None;
                st.handle.replace(handle)
            }
            Ok(handle) => Some(handle),
            Err(e) => {
                st.error = Some(t(&e));
                None
            }
        };
        drop(st);
        drop(unused);
    }

    fn fan(&self, cx: &Ctx, args: &Value) -> Value {
        let percent = |v: &Value| v.as_u64().map(|n| n.min(100) as u8);
        let fan = match args["mode"].as_str() {
            Some("manual") => percent(&args["level"]).map(Fan::Manual),
            Some("curve") => args["curve"].as_array().filter(|levels| levels.len() == CURVE_AT.len()).and_then(|levels| {
                let points: Option<Vec<(u8, u8)>> = CURVE_AT.iter().zip(levels).map(|(at, level)| Some((*at, percent(level)?))).collect();
                points.map(Fan::Curve)
            }),
            Some("auto") => Some(Fan::Auto),
            _ => None,
        };
        let Some(fan) = fan else { return json!({ "error": fan_error("bad field") }) };
        match svc_hw::fan_apply(&fan) {
            Ok(state) => {
                // What was chosen is remembered; the mode is not: a new session starts on automatic.
                match &fan {
                    Fan::Manual(level) => cx.patch_settings(ID, json!({ "fanLevel": level })),
                    Fan::Curve(_) => cx.patch_settings(ID, json!({ "fanCurve": args["curve"] })),
                    Fan::Auto => {}
                }
                json!({ "fan": state })
            }
            Err(e) => json!({ "error": fan_error(&e) }),
        }
    }

    /// Runs `winget install namazso.PawnIO`. Only ever from the button.
    fn install_driver(self: &Arc<Self>, cx: &Ctx) {
        let Some(winget) = svc_apps::winget::exe() else { return };
        {
            let mut st = self.st.lock().unwrap();
            if st.installing || !st.on {
                return;
            }
            st.installing = true;
            st.install_note = None;
        }
        let (inner, cx) = (self.clone(), cx.clone());
        std::thread::spawn(move || {
            let args = ["install", "--id", svc_hw::DRIVER_PACKAGE, "--exact", "--source", "winget", "--accept-package-agreements", "--accept-source-agreements", "--disable-interactivity"];
            let done = svc_apps::run(&winget, &args, Duration::from_secs(300), &AtomicBool::new(false), |_| {});
            let installed = svc_hw::driver_installed();
            notch_core::log::line(format!("hw: winget install of PawnIO ended, driver installed {installed}"));
            let note = if installed {
                // A helper that already runs opens the hardware again, with the driver: the
                // same process, so Windows asks nothing. With no helper there is nobody to tell.
                let _ = svc_hw::reopen();
                None
            } else if done.is_ok_and(|o| o.code == Some(0)) {
                Some(t("winget finished. Restart Windows if PawnIO asks for it."))
            } else {
                Some(t("PawnIO was not installed. You can also get it from pawnio.eu."))
            };
            {
                let mut st = inner.st.lock().unwrap();
                st.installing = false;
                st.install_note = note;
            }
            cx.emit(ID, "changed", ());
        });
    }
}

impl Feature for HwHelper {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn start(&self, _cx: &Ctx) -> Result<(), String> {
        // Never a permission prompt from here: the helper starts from its switch, on a click.
        self.inner.st.lock().unwrap().on = true;
        Ok(())
    }

    fn stop(&self, _cx: &Ctx) {
        let ended = std::mem::take(&mut *self.inner.st.lock().unwrap());
        // Dropping the handle tells the helper to quit; no lock is held meanwhile.
        drop(ended);
    }

    fn call(&self, cx: &Ctx, method: &str, args: Value) -> Result<Value, String> {
        let inner = &self.inner;
        match method {
            "status" => Ok(inner.status()),
            // Only ever from the switch on the Settings page: Windows asks for permission.
            "launch" => {
                let launched = svc_hw::launch();
                if launched.is_ok() {
                    cx.patch_settings(ID, json!({ "consent": true }));
                }
                inner.keep(launched);
                Ok(inner.status())
            }
            "quit" => {
                let ended = {
                    let mut st = inner.st.lock().unwrap();
                    st.error = None;
                    st.handle.take()
                };
                drop(ended);
                Ok(inner.status())
            }
            // Only ever from its button.
            "installDriver" => {
                inner.install_driver(cx);
                Ok(inner.status())
            }
            "fan" => Ok(inner.fan(cx, &args)),
            _ => Err("unknown method".into()),
        }
    }

    fn strings(&self) -> &'static str {
        include_str!("../../../src/features/hw-helper/strings.json")
    }
}

pub fn feature() -> std::sync::Arc<dyn Feature> {
    std::sync::Arc::new(HwHelper::default())
}
