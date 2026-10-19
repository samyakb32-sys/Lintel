// The base features, wrapped as built-in features: thin adapters that give
// each one a manifest, a real `start` and `stop`, and a switch in Settings.
// Their commands stay the base's own Tauri commands (lib.rs), refused while the
// feature is off (`registry::require`); nothing new goes through them, but for
// the chat's headless `ask` (chat/headless.rs), which is a `feature_call`.

use std::sync::Arc;

use notch_core::i18n::n_;
use notch_core::{Ctx, Feature, Group, Manifest, Use};
use serde_json::{json, Value};
use tauri::{Emitter, Manager};

use crate::island::WINDOW_LABEL;
use crate::{chat, hooks, integrations, pipe, recap, webview_drop};

pub fn builtin() -> Vec<Arc<dyn Feature>> {
    vec![Arc::new(Agents), Arc::new(Chat), Arc::new(Integrations), Arc::new(Drop), Arc::new(Recap)]
}

fn unknown() -> Result<Value, String> {
    Err("unknown method".into())
}

// ── Agents: the relay pipe, approvals, questions ──────────────────────────────

struct Agents;

static AGENTS: Manifest = Manifest {
    id: "agents",
    name: n_("Coding agents"),
    group: Group::Agents,
    default_on: true,
    uses: &[],
    shortcuts: &[],
    tray: &[],
    defaults: "{}",
};

impl Feature for Agents {
    fn manifest(&self) -> &'static Manifest {
        &AGENTS
    }
    /// Serves the relay pipe. Without it meteor-hook finds nobody and exits at
    /// once, so every agent CLI carries on as if METEOR V1 were closed.
    fn start(&self, cx: &Ctx) -> Result<(), String> {
        hooks::ensure_hook_exe(cx.app());
        pipe::start(cx.app().clone());
        Ok(())
    }
    /// Closes the pipe; a request still waiting gets no answer and its agent
    /// asks in the terminal.
    fn stop(&self, cx: &Ctx) {
        pipe::stop(cx.app());
    }
    fn call(&self, _cx: &Ctx, _method: &str, _args: Value) -> Result<Value, String> {
        unknown()
    }
}

// ── Chat ──────────────────────────────────────────────────────────────────────

struct Chat;

static CHAT: Manifest = Manifest {
    id: "chat",
    name: "Chat",
    group: Group::Chat,
    default_on: true,
    uses: &[Use::Network],
    shortcuts: &[],
    tray: &[],
    defaults: "{}",
};

impl Feature for Chat {
    fn manifest(&self) -> &'static Manifest {
        &CHAT
    }
    fn start(&self, _cx: &Ctx) -> Result<(), String> {
        Ok(())
    }
    /// Forgets the conversation; an answer still on its way is dropped.
    fn stop(&self, cx: &Ctx) {
        cx.app().state::<chat::Chat>().reset();
        chat::headless::stop_all();
    }
    /// The headless completion the voice assistant asks for (chat/headless.rs).
    /// The chat view's own turn stays the `chat_send` command.
    fn call(&self, cx: &Ctx, method: &str, args: Value) -> Result<Value, String> {
        let settings = || cx.app().state::<crate::Shared>().settings.lock().unwrap().clone();
        match method {
            "ask" => {
                let voice = match crate::registry::is_running(cx.app(), "voice") {
                    true => crate::registry::feature_settings_of(cx.app(), "voice"),
                    false => Value::Null,
                };
                chat::headless::permit(&args, &voice)?;
                let cx = cx.clone();
                let emit = move |event: &str, payload: Value| cx.emit("chat", event, payload);
                chat::headless::ask(args, &settings(), Arc::new(emit))
            }
            "cancel" => Ok(json!({ "cancelled": chat::headless::cancel(args["id"].as_str().unwrap_or("")) })),
            "reach" => Ok(json!({ "ok": chat::headless::reach(&settings()) })),
            _ => unknown(),
        }
    }
}

// ── Integrations: the pollers ─────────────────────────────────────────────────

struct Integrations;

static INTEGRATIONS: Manifest = Manifest {
    id: "integrations",
    name: "Integrations",
    group: Group::Agents,
    default_on: true,
    uses: &[Use::Network],
    shortcuts: &[],
    tray: &[],
    defaults: "{}",
};

impl Feature for Integrations {
    fn manifest(&self) -> &'static Manifest {
        &INTEGRATIONS
    }
    fn start(&self, cx: &Ctx) -> Result<(), String> {
        integrations::start(cx.app().clone());
        Ok(())
    }
    fn stop(&self, _cx: &Ctx) {
        integrations::stop();
    }
    fn call(&self, _cx: &Ctx, _method: &str, _args: Value) -> Result<Value, String> {
        unknown()
    }
}

// ── Drop: files dropped on the island ─────────────────────────────────────────

struct Drop;

static DROP: Manifest = Manifest {
    id: "drop",
    name: n_("File drop"),
    group: Group::Files,
    default_on: true,
    uses: &[],
    shortcuts: &[],
    tray: &[],
    defaults: "{}",
};

impl Feature for Drop {
    fn manifest(&self) -> &'static Manifest {
        &DROP
    }
    fn start(&self, _cx: &Ctx) -> Result<(), String> {
        webview_drop::set_enabled(true);
        Ok(())
    }
    fn stop(&self, _cx: &Ctx) {
        webview_drop::set_enabled(false);
    }
    fn call(&self, _cx: &Ctx, _method: &str, _args: Value) -> Result<Value, String> {
        unknown()
    }
}

// ── Weekly recap ──────────────────────────────────────────────────────────────

struct Recap;

static RECAP: Manifest = Manifest {
    id: "recap",
    name: "Weekly recap",
    group: Group::Agents,
    default_on: true,
    uses: &[],
    shortcuts: &[],
    tray: &[("open", "Weekly recap")],
    defaults: "{}",
};

impl Feature for Recap {
    fn manifest(&self) -> &'static Manifest {
        &RECAP
    }
    /// Counts turns and decisions from here on (recap.rs); nothing is counted
    /// while it is off.
    fn start(&self, _cx: &Ctx) -> Result<(), String> {
        recap::set_active(true);
        Ok(())
    }
    fn stop(&self, _cx: &Ctx) {
        recap::set_active(false);
    }
    fn call(&self, cx: &Ctx, method: &str, _args: Value) -> Result<Value, String> {
        match method {
            // The island owns the recap view (src/main.ts).
            "tray" => {
                let _ = cx.app().emit_to(WINDOW_LABEL, "tray", "recap".to_string());
                Ok(Value::Null)
            }
            _ => unknown(),
        }
    }
}
