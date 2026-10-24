// Hub — a Slack-like message bus for agents (docs/ARCHITECTURE.md §2).
//
// Channels hold messages; participants (the user, CLI agents through the hook
// pipe, web agents through their adapters) post to them. The hub only stores
// and announces: it never forwards a message to an agent by itself, so nothing
// is sent for the user without a click. Adapters call `post` with their own
// participant id; delivery to an agent is a separate, confirmed step.

use std::collections::BTreeMap;
use std::sync::Mutex;

use notch_core::{Ctx, Feature, Group, Manifest};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

static MANIFEST: Manifest = Manifest {
    id: "hub",
    name: "Hub",
    group: Group::Agents,
    default_on: false,
    uses: &[],
    shortcuts: &[],
    tray: &[],
    defaults: "{}",
};

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    User,
    Cli,
    Web,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Participant {
    pub id: String,
    pub name: String,
    pub kind: Kind,
}

#[derive(Clone, Serialize)]
pub struct Message {
    pub id: u64,
    pub channel: String,
    pub from: String,
    pub text: String,
    pub at: u64,
}

#[derive(Default)]
struct Bus {
    next: u64,
    participants: BTreeMap<String, Participant>,
    channels: BTreeMap<String, Vec<Message>>,
}

#[derive(Default)]
pub struct Hub {
    bus: Mutex<Bus>,
}

fn arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or_else(|| format!("missing {key}"))
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

impl Feature for Hub {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn start(&self, _cx: &Ctx) -> Result<(), String> {
        let mut bus = self.bus.lock().unwrap();
        bus.channels.entry("general".into()).or_default();
        bus.participants.insert("you".into(), Participant { id: "you".into(), name: "You".into(), kind: Kind::User });
        Ok(())
    }

    fn stop(&self, _cx: &Ctx) {
        *self.bus.lock().unwrap() = Bus::default();
    }

    fn call(&self, cx: &Ctx, method: &str, args: Value) -> Result<Value, String> {
        let mut bus = self.bus.lock().unwrap();
        match method {
            "channels" => Ok(json!(bus.channels.keys().collect::<Vec<_>>())),
            "participants" => Ok(json!(bus.participants.values().collect::<Vec<_>>())),
            "create_channel" => {
                bus.channels.entry(arg(&args, "id")?.to_string()).or_default();
                Ok(json!({}))
            }
            "join" => {
                let p: Participant = serde_json::from_value(args).map_err(|e| e.to_string())?;
                bus.participants.insert(p.id.clone(), p);
                Ok(json!({}))
            }
            "post" => {
                let (channel, from, text) = (arg(&args, "channel")?, arg(&args, "from")?, arg(&args, "text")?);
                if !bus.participants.contains_key(from) {
                    return Err("unknown participant".into());
                }
                bus.next += 1;
                let msg = Message { id: bus.next, channel: channel.into(), from: from.into(), text: text.into(), at: now() };
                bus.channels.get_mut(channel).ok_or("unknown channel")?.push(msg.clone());
                cx.emit("hub", "message", msg.clone());
                Ok(json!(msg))
            }
            // debt: history is in memory only and unbounded
            "history" => Ok(json!(bus.channels.get(arg(&args, "channel")?).ok_or("unknown channel")?)),
            _ => Err("unknown method".into()),
        }
    }

    fn strings(&self) -> &'static str {
        include_str!("../../../src/features/hub/strings.json")
    }
}

pub fn feature() -> std::sync::Arc<dyn Feature> {
    std::sync::Arc::new(Hub::default())
}
