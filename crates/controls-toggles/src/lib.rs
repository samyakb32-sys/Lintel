// Controls toggles — a METEOR V1 feature. The contract is docs/ARCHITECTURE.md §2: nothing
// runs before `start`, everything is gone after `stop`, and every command of
// the feature arrives through `call`.

pub mod sys;

use notch_core::{Ctx, Feature, Group, Manifest, Use};
use serde_json::{json, Value};
use std::path::PathBuf;

static MANIFEST: Manifest = Manifest {
    id: "controls-toggles",
    name: "Controls",
    group: Group::Island,
    default_on: true,
    uses: &[Use::FileDelete],
    shortcuts: &[],
    tray: &[],
    defaults: "{}",
};

fn state() -> Value {
    let (app, system) = sys::theme();
    let vol = svc_audio::endpoint_volume(svc_audio::Flow::Render).ok();
    json!({
        "themeApp": app, "themeSystem": system,
        "hidden": sys::hidden_shown(),
        "icons": sys::icons_shown(),
        "bin": sys::bin().map(|(n, b)| json!({ "items": n, "bytes": b })),
        "volume": vol.map(|v| json!({ "level": v.scalar, "mute": v.mute })),
    })
}

#[derive(Default)]
pub struct ControlsToggles;

impl Feature for ControlsToggles {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn start(&self, _cx: &Ctx) -> Result<(), String> {
        Ok(())
    }

    fn stop(&self, _cx: &Ctx) {}

    fn call(&self, cx: &Ctx, method: &str, args: Value) -> Result<Value, String> {
        let pad = cx.data_dir("controls-toggles").join("scratchpad.json");
        match method {
            "state" => Ok(state()),
            "volume" => {
                if let Some(l) = args["level"].as_f64() {
                    svc_audio::set_volume(svc_audio::Flow::Render, l as f32)?;
                }
                if let Some(m) = args["mute"].as_bool() {
                    svc_audio::set_mute(svc_audio::Flow::Render, m)?;
                }
                Ok(Value::Null)
            }
            "brightness_get" => Ok(json!(sys::brightness())),
            "brightness" => sys::set_brightness(args["level"].as_f64().unwrap_or(0.5) as f32).map(|_| Value::Null),
            // Flip one toggle, then read Windows again: if it did not move, say so.
            "toggle" => {
                let what = args["what"].as_str().unwrap_or("");
                let before = state();
                match what {
                    "theme" => sys::set_dark(!sys::theme().0)?,
                    "hidden" => sys::set_hidden_shown(!sys::hidden_shown())?,
                    "icons" => sys::toggle_icons()?,
                    _ => return Err("unknown toggle".into()),
                }
                std::thread::sleep(std::time::Duration::from_millis(250));
                let after = state();
                let key = match what { "theme" => "themeApp", "hidden" => "hidden", _ => "icons" };
                if before[key] == after[key] {
                    return Err("Windows did not apply it".into());
                }
                Ok(after)
            }
            "lock" => sys::lock().map(|_| Value::Null),
            "display_off" => {
                sys::display_off();
                Ok(Value::Null)
            }
            // The page has shown the count and size and the user said yes.
            "bin_empty" => {
                if args["confirm"] != true {
                    return Err("not confirmed".into());
                }
                sys::bin_empty()?;
                Ok(state())
            }
            "scratch_load" => Ok(std::fs::read_to_string(&pad).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null)),
            "scratch_save" => {
                let tmp = pad.with_extension("tmp");
                std::fs::write(&tmp, args["data"].to_string()).map_err(|e| e.to_string())?;
                std::fs::rename(&tmp, &pad).map_err(|e| e.to_string())?;
                Ok(Value::Null)
            }
            // Writes <name>.md into Downloads, never over an existing file.
            "scratch_export" => {
                let name: String = args["name"].as_str().unwrap_or("note").chars().filter(|c| c.is_alphanumeric() || " -_".contains(*c)).collect();
                let name = if name.trim().is_empty() { "note".into() } else { name.trim().to_string() };
                let ext = if args["ext"] == "txt" { "txt" } else { "md" };
                let dir = PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default()).join("Downloads");
                let _ = std::fs::create_dir_all(&dir);
                let mut path = dir.join(format!("{name}.{ext}"));
                let mut n = 2;
                while path.exists() {
                    path = dir.join(format!("{name} ({n}).{ext}"));
                    n += 1;
                }
                std::fs::write(&path, args["text"].as_str().unwrap_or("")).map_err(|e| e.to_string())?;
                Ok(json!(path.to_string_lossy()))
            }
            _ => Err("unknown method".into()),
        }
    }

    fn strings(&self) -> &'static str {
        include_str!("../../../src/features/controls-toggles/strings.json")
    }
}

pub fn feature() -> std::sync::Arc<dyn Feature> {
    std::sync::Arc::new(ControlsToggles)
}
