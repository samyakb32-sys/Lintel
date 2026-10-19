// MCP servers for the coding agents: a short list kept here, connected to an
// agent's own config one agent at a time.
//
// The same rules as the hooks (config_file.rs): the exact diff first, a dated
// backup that must succeed, a write only after a click, and only the one entry
// is added or removed. An entry that someone else wrote, or that was edited since,
// is refused rather than replaced.
//
// Claude Code keeps its user-wide servers in ~/.claude.json, Cursor in
// ~/.cursor/mcp.json; both under "mcpServers". No secret keys here: a server that
// needs one is not supported yet, so nothing secret is written to a file.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::agents::Relay;
use crate::config_file::{self, FileEdit, Plan};
use crate::{i18n, log, platform};

// ── The list ──────────────────────────────────────────────────────────────────

/// One server: a program to start (`command`, `args`) or a link (`url`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Server {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub command: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
}

impl Server {
    /// How it reads in the list: the command line, or the link.
    fn summary(&self) -> String {
        if self.url.is_empty() {
            std::iter::once(self.command.as_str()).chain(self.args.iter().map(String::as_str)).collect::<Vec<_>>().join(" ")
        } else {
            self.url.clone()
        }
    }
}

fn list_path() -> PathBuf {
    notch_core::win::local_dir().join("mcp-servers.json")
}

fn load() -> Vec<Server> {
    std::fs::read(list_path())
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Vec<Server>>(&bytes).ok())
        .unwrap_or_default()
}

fn store(list: &[Server]) -> Result<(), String> {
    let path = list_path();
    if let Some(dir) = path.parent() {
        notch_core::win::ensure_private_dir(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(list).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| e.to_string())
}

/// `line` as a server: a lone http(s) link, else a command followed by its
/// arguments (words, with "double quotes" for one that holds spaces).
fn parse(name: &str, line: &str) -> Result<Server, String> {
    let name = name.trim();
    let valid = !name.is_empty()
        && name.len() <= 40
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !valid {
        return Err(i18n::t("Use letters, digits, - and _ for the name."));
    }
    let words = split_words(line);
    let Some(first) = words.first() else {
        return Err(i18n::t("Enter a command or a link."));
    };
    let is_link = words.len() == 1 && (first.starts_with("https://") || first.starts_with("http://"));
    Ok(if is_link {
        Server { name: name.into(), command: String::new(), args: Vec::new(), url: first.clone() }
    } else {
        Server { name: name.into(), command: first.clone(), args: words[1..].to_vec(), url: String::new() }
    })
}

fn split_words(line: &str) -> Vec<String> {
    let (mut words, mut word, mut quoted, mut any) = (Vec::new(), String::new(), false, false);
    for c in line.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any {
                    words.push(std::mem::take(&mut word));
                    any = false;
                }
            }
            c => {
                word.push(c);
                any = true;
            }
        }
    }
    if any {
        words.push(word);
    }
    words
}

// ── The agents ────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    ClaudeCode,
    Cursor,
}

impl Target {
    pub const ALL: &'static [Target] = &[Target::ClaudeCode, Target::Cursor];

    pub fn id(self) -> &'static str {
        match self {
            Target::ClaudeCode => "claude",
            Target::Cursor => "cursor",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Target::ClaudeCode => "Claude Code",
            Target::Cursor => "Cursor",
        }
    }

    fn file(self, home: &Path) -> PathBuf {
        match self {
            Target::ClaudeCode => home.join(".claude.json"),
            Target::Cursor => home.join(".cursor").join("mcp.json"),
        }
    }

    /// The entry this agent wants for `server`.
    fn entry(self, server: &Server, relay: &Relay) -> Value {
        if !server.url.is_empty() {
            return match self {
                Target::ClaudeCode => json!({ "type": "http", "url": server.url }),
                Target::Cursor => json!({ "url": server.url }),
            };
        }
        let (mut command, mut args) = (server.command.clone(), server.args.clone());
        // On Windows Claude Code starts a program directly, and npx and its kin
        // are .cmd scripts: they go through cmd /c.
        let script = matches!(command.to_ascii_lowercase().as_str(), "npx" | "npm" | "pnpm" | "yarn");
        if self == Target::ClaudeCode && relay.windows && script {
            args.splice(0..0, ["/c".to_string(), command]);
            command = "cmd".into();
        }
        match self {
            Target::ClaudeCode => json!({ "type": "stdio", "command": command, "args": args, "env": {} }),
            Target::Cursor => json!({ "command": command, "args": args }),
        }
    }
}

fn find_target(id: &str) -> Result<Target, String> {
    Target::ALL
        .iter()
        .copied()
        .find(|t| t.id() == id)
        .ok_or_else(|| i18n::tf("Unknown agent \"{id}\".", &[("id", id)]))
}

fn find_server(name: &str) -> Result<Server, String> {
    load()
        .into_iter()
        .find(|s| s.name == name)
        .ok_or_else(|| i18n::tf("Unknown server \"{name}\".", &[("name", name)]))
}

type JsonChange = Box<dyn Fn(&Value) -> Result<Option<Value>, String>>;

/// The change to one agent's config: adds (or takes out) the entry for `server`.
fn change(target: Target, server: &Server, relay: &Relay, install: bool) -> JsonChange {
    let name = server.name.clone();
    let ours = target.entry(server, relay);
    Box::new(move |root| {
        let Some(map) = root.as_object() else { return Err(unexpected(target, "")) };
        let mut servers: Map<String, Value> = match map.get("mcpServers") {
            None => Map::new(),
            Some(Value::Object(m)) => m.clone(),
            Some(_) => return Err(unexpected(target, "mcpServers")),
        };
        let found = servers.get(&name).cloned();
        match (found.as_ref(), install) {
            (None, true) => {
                servers.insert(name.clone(), ours.clone());
            }
            (Some(found), true) if *found == ours => {}
            (None, false) => {}
            (Some(found), false) if *found == ours => {
                servers.shift_remove(&name);
            }
            // Somebody else's entry of that name, or ours edited since.
            (Some(_), _) => return Err(i18n::tf("\"{name}\" is in {agent}'s config but is not what {app} added — {app} has not touched it.", &[("name", &name), ("agent", target.name())])),
        }
        let mut next = map.clone();
        next.insert("mcpServers".into(), Value::Object(servers));
        Ok(Some(Value::Object(next)))
    })
}

fn unexpected(target: Target, key: &str) -> String {
    let what = if key.is_empty() { target.name().to_string() } else { format!("\"{key}\"") };
    i18n::tf("{what} has an unexpected type — {app} has not touched it.", &[("what", &what)])
}

fn edits(target: Target, server: &Server, home: &Path, relay: &Relay, install: bool) -> Vec<FileEdit<'static>> {
    let path = target.file(home);
    let label = path.display().to_string();
    vec![FileEdit { path, edit: config_file::json_edit(label, change(target, server, relay, install)) }]
}

/// True when the agent's config holds exactly the entry METEOR V1 would write.
/// Never fails: a file that cannot be read reads as "not connected".
fn connected(target: Target, server: &Server, home: &Path, relay: &Relay) -> bool {
    config_file::read(&target.file(home))
        .ok()
        .flatten()
        .and_then(|bytes| config_file::parse_json(Some(&bytes), "").ok())
        .and_then(|root| root.get("mcpServers").and_then(|s| s.get(&server.name)).cloned())
        .is_some_and(|found| found == target.entry(server, relay))
}

// ── Public API ────────────────────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentState {
    pub id: &'static str,
    pub name: &'static str,
    pub connected: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerStatus {
    pub name: String,
    pub summary: String,
    pub agents: Vec<AgentState>,
}

pub fn list() -> Vec<ServerStatus> {
    let (home, relay) = (platform::home_dir(), Relay::current());
    load()
        .into_iter()
        .map(|server| ServerStatus {
            summary: server.summary(),
            agents: Target::ALL
                .iter()
                .map(|&t| AgentState { id: t.id(), name: t.name(), connected: connected(t, &server, &home, &relay) })
                .collect(),
            name: server.name,
        })
        .collect()
}

pub fn save(name: &str, line: &str) -> Result<(), String> {
    let server = parse(name, line)?;
    let mut list = load();
    if list.iter().any(|s| s.name == server.name) {
        return Err(i18n::tf("\"{name}\" is already in the list.", &[("name", &server.name)]));
    }
    list.push(server);
    store(&list)
}

/// Only a server no agent has any more: the list never forgets an entry that is
/// still in an agent's config.
pub fn delete(name: &str) -> Result<(), String> {
    let server = find_server(name)?;
    let (home, relay) = (platform::home_dir(), Relay::current());
    if Target::ALL.iter().any(|&t| connected(t, &server, &home, &relay)) {
        return Err(i18n::tf("Disconnect \"{name}\" from every agent first.", &[("name", name)]));
    }
    let mut list = load();
    list.retain(|s| s.name != name);
    store(&list)
}

pub fn preview(name: &str, agent: &str, install: bool) -> Result<Plan, String> {
    let (server, target) = (find_server(name)?, find_target(agent)?);
    config_file::preview(&edits(target, &server, &platform::home_dir(), &Relay::current(), install))
}

/// Only ever called from an explicit click. Returns the backups taken, one per line.
pub fn apply(name: &str, agent: &str, install: bool, fingerprint: &str) -> Result<String, String> {
    let (server, target) = (find_server(name)?, find_target(agent)?);
    let backups = config_file::apply(&edits(target, &server, &platform::home_dir(), &Relay::current(), install), fingerprint)?;
    log::line(format!("mcp server {name} {} for {agent}", if install { "connected" } else { "disconnected" }));
    Ok(backups.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn relay(windows: bool) -> Relay {
        Relay { exe: "x".into(), windows }
    }

    fn server() -> Server {
        parse("vercel", "npx -y @vercel/mcp").unwrap()
    }

    #[test]
    fn a_link_or_a_command_with_quoted_words() {
        assert_eq!(parse("a", "https://x.dev/mcp").unwrap().url, "https://x.dev/mcp");
        let s = parse("a", r#"node "C:\my tools\srv.js" --flag"#).unwrap();
        assert_eq!((s.command.as_str(), s.args), ("node", vec![r"C:\my tools\srv.js".to_string(), "--flag".to_string()]));
        assert!(parse("a b", "node").is_err());
        assert!(parse("a", "   ").is_err());
    }

    #[test]
    fn claude_gets_its_own_format_and_cmd_for_npx_on_windows() {
        let s = server();
        assert_eq!(Target::ClaudeCode.entry(&s, &relay(false))["command"], "npx");
        let win = Target::ClaudeCode.entry(&s, &relay(true));
        assert_eq!((win["command"].clone(), win["args"][0].clone(), win["args"][1].clone()), (json!("cmd"), json!("/c"), json!("npx")));
        assert_eq!(Target::Cursor.entry(&s, &relay(true)), json!({ "command": "npx", "args": ["-y", "@vercel/mcp"] }));
    }

    #[test]
    fn connect_keeps_everything_else_and_disconnect_takes_only_ours() {
        let s = server();
        let root = json!({ "history": [1, 2], "mcpServers": { "mine": { "command": "x" } } });
        let added = change(Target::Cursor, &s, &relay(false), true)(&root).unwrap().unwrap();
        assert_eq!(added["history"], json!([1, 2]));
        assert_eq!(added["mcpServers"]["mine"], json!({ "command": "x" }));
        assert!(added["mcpServers"]["vercel"].is_object());
        let removed = change(Target::Cursor, &s, &relay(false), false)(&added).unwrap().unwrap();
        assert_eq!(removed, root);
    }

    #[test]
    fn an_entry_someone_else_wrote_is_refused_both_ways() {
        let s = server();
        let root = json!({ "mcpServers": { "vercel": { "command": "other" } } });
        assert!(change(Target::Cursor, &s, &relay(false), true)(&root).is_err());
        assert!(change(Target::Cursor, &s, &relay(false), false)(&root).is_err());
        assert!(change(Target::Cursor, &s, &relay(false), true)(&json!({ "mcpServers": [] })).is_err());
    }
}
