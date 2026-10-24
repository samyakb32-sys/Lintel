// Hub — a Slack-like message bus for agents (docs/ARCHITECTURE.md §2).
//
// Channels hold messages; participants (the user, CLI agents through the hook
// pipe, web agents through their adapters) post to them. The hub only stores
// and announces: it never forwards a message to an agent by itself, so nothing
// is sent for the user without a click. Adapters call `post` with their own
// participant id; delivery to an agent is a separate, confirmed step.
//
// Hand-offs: an agent that is stuck (or the user) asks for a capability the
// agent lacks ("supabase", "browse"). The hub picks a participant that has it,
// as read from that agent's connected-tools list and reported with `set_caps`,
// and records a task. The task is only proposed: it is delivered when the user
// approves it, or at once in a channel the user listed in `autoRelay`.

mod webviews;

use std::collections::BTreeMap;
use std::sync::Mutex;

use notch_core::{Ctx, Feature, Group, Manifest, Monitor, WindowSpec};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

static MANIFEST: Manifest = Manifest {
    id: "hub",
    name: "Hub",
    group: Group::Agents,
    default_on: true,
    uses: &[],
    shortcuts: &[],
    tray: &[],
    defaults: r#"{ "autoRelay": [] }"#,
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
    #[serde(default)]
    pub caps: Vec<String>,
}

#[derive(Clone, Copy, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Proposed,
    Sent,
    Done,
    Skipped,
}

#[derive(Clone, Serialize)]
pub struct Task {
    pub id: u64,
    pub channel: String,
    pub from: String,
    pub to: String,
    pub need: String,
    pub text: String,
    pub status: Status,
    pub result: Option<String>,
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
    tasks: BTreeMap<u64, Task>,
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
        bus.participants.insert("you".into(), Participant { id: "you".into(), name: "You".into(), kind: Kind::User, caps: vec![] });
        Ok(())
    }

    fn stop(&self, _cx: &Ctx) {
        *self.bus.lock().unwrap() = Bus::default();
    }

    fn call(&self, cx: &Ctx, method: &str, args: Value) -> Result<Value, String> {
        match method {
            "open" | "tray" => {
                cx.window().open(WindowSpec { label: "hub".into(), feature: "hub".into(), page: "main".into(), w: 1120.0, h: 720.0, frameless: true, monitor: Monitor::Island, ..Default::default() })?;
                return Ok(json!({}));
            }
            "agent_show" => return webviews::show(cx, &args),
            "agent_hide" => {
                webviews::hide_all(cx);
                return Ok(json!({}));
            }
            "close" => {
                cx.window().close("hub");
                return Ok(json!({}));
            }
            _ => {}
        }
        let auto = cx.settings("hub")["autoRelay"].clone();
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
            "set_caps" => {
                let caps: Vec<String> = serde_json::from_value(args["caps"].clone()).map_err(|e| e.to_string())?;
                bus.participants.get_mut(arg(&args, "id")?).ok_or("unknown participant")?.caps = caps;
                Ok(json!({}))
            }
            "handoff" => {
                let (channel, from, need, text) = (arg(&args, "channel")?, arg(&args, "from")?, arg(&args, "need")?, arg(&args, "text")?);
                let to = bus
                    .participants
                    .values()
                    .find(|p| p.id != from && p.kind != Kind::User && p.caps.iter().any(|c| c == need))
                    .ok_or_else(|| format!("no agent has {need}"))?
                    .id
                    .clone();
                bus.next += 1;
                let relay = auto.as_array().is_some_and(|a| a.iter().any(|c| c == channel));
                let status = if relay { Status::Sent } else { Status::Proposed };
                let task = Task { id: bus.next, channel: channel.into(), from: from.into(), to, need: need.into(), text: text.into(), status, result: None };
                bus.tasks.insert(task.id, task.clone());
                cx.emit("hub", if relay { "deliver" } else { "proposal" }, task.clone());
                Ok(json!(task))
            }
            "approve" | "skip" => {
                let task = bus.tasks.get_mut(&args["task"].as_u64().ok_or("missing task")?).ok_or("unknown task")?;
                if task.status != Status::Proposed {
                    return Err("task is not waiting".into());
                }
                task.status = if method == "approve" { Status::Sent } else { Status::Skipped };
                if method == "approve" {
                    cx.emit("hub", "deliver", task.clone());
                }
                Ok(json!(task))
            }
            "finish" => {
                let task = bus.tasks.get_mut(&args["task"].as_u64().ok_or("missing task")?).ok_or("unknown task")?;
                task.status = Status::Done;
                task.result = Some(arg(&args, "result")?.into());
                cx.emit("hub", "done", task.clone());
                Ok(json!(task))
            }
            // "is it done?": one task, or all of them
            "tasks" => Ok(match args["task"].as_u64() {
                Some(id) => json!(bus.tasks.get(&id).ok_or("unknown task")?),
                None => json!(bus.tasks.values().collect::<Vec<_>>()),
            }),
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
