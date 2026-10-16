// Chat through an agent CLI the user has installed and signed in to, on their
// own plan. The CLI is started once per message in its non-interactive mode,
// with its tools off, in an empty folder of METEOR V1's own, and the answer is
// read from what it prints. METEOR V1 reads none of its tokens or settings, and
// writes none.
//
// The arguments and the output formats are those of upstream pull requests 400
// (Claude Code, Codex, Gemini CLI: `CLIChatTools.swift`), 151 (Cursor:
// `cursor_chat.rs`) and 155 (Kiro: `kiro_chat.rs`).
//
// What the user typed goes to the CLI's input wherever the CLI reads one. The
// two that only take it as an argument (Cursor, Kiro) get it after `--`, so
// it can never be read as an option.

use std::path::PathBuf;
use std::time::Duration;

use notch_core::i18n::{t, tf};
use serde_json::Value;

use super::program::{self, Handle, Program, Spec, Stop};
use super::text::{first_words, strip_ansi};

pub struct Cli {
    /// Stored in the settings: never changes.
    pub id: &'static str,
    pub name: &'static str,
    /// The names it is installed under, best first.
    binaries: &'static [&'static str],
}

pub const CLIS: &[Cli] = &[
    Cli { id: "claude", name: "Claude Code", binaries: &["claude"] },
    Cli { id: "codex", name: "Codex", binaries: &["codex"] },
    Cli { id: "gemini", name: "Gemini CLI", binaries: &["gemini"] },
    Cli { id: "cursor", name: "Cursor", binaries: &["cursor-agent", "agent"] },
    Cli { id: "kiro", name: "Kiro", binaries: &["kiro-cli"] },
];

/// As upstream: an answer may take a while, never more than this.
const TIME_LIMIT: Duration = Duration::from_secs(300);
/// Everything a CLI may print for one answer.
const MAX_OUTPUT: usize = 4 * 1024 * 1024;
/// A conversation handed over as an argument has to fit a Windows command
/// line (32 767 characters): the oldest turns go first.
const MAX_ARGUMENT: usize = 24_000;

const GEMINI_TASK: &str = "Reply to the conversation given on stdin. Answer the last User message.";

pub fn find(id: &str) -> Option<&'static Cli> {
    CLIS.iter().find(|cli| cli.id == id)
}

/// The CLIs found on this computer. Files are looked for; nothing is started.
pub fn installed() -> Vec<&'static Cli> {
    CLIS.iter().filter(|cli| cli.program().is_some()).collect()
}

impl Cli {
    pub fn program(&self) -> Option<Program> {
        self.binaries.iter().find_map(|name| {
            let found = program::resolve(&[name])?;
            // "agent" is anybody's name for a program: only Cursor's own counts.
            (*name != "agent" || path_mentions(&found, "cursor")).then_some(found)
        })
    }
}

fn path_mentions(program: &Program, word: &str) -> bool {
    std::iter::once(program.exe.as_os_str())
        .chain(program.lead.iter().map(|s| s.as_os_str()))
        .any(|part| part.to_string_lossy().to_lowercase().contains(word))
}

/// The arguments for one question, and what is written to the CLI's input.
fn invocation(id: &str, system: &str, conversation: &str) -> (Vec<String>, Option<String>) {
    let fixed = |args: &[&str]| args.iter().map(|a| a.to_string()).collect::<Vec<_>>();
    let whole = format!("{system}\n\n{conversation}");
    // The end of the conversation is what must survive a cut.
    let as_argument = || {
        let room = MAX_ARGUMENT.saturating_sub(system.chars().count());
        let skip = conversation.chars().count().saturating_sub(room);
        format!("{system}\n\n{}", conversation.chars().skip(skip).collect::<String>())
    };
    match id {
        // No tools, no hooks, no MCP servers, no skills and nothing saved: a
        // plain chat, which does not show up as an agent session.
        "claude" => {
            let mut args = fixed(&[
                "-p", "--output-format", "stream-json", "--verbose", "--include-partial-messages",
                "--tools", "", "--no-session-persistence", "--setting-sources", "",
                "--strict-mcp-config", "--disable-slash-commands", "--system-prompt",
            ]);
            args.push(system.to_string());
            (args, Some(conversation.to_string()))
        }
        // Read-only sandbox, nothing saved, the user's own config and rules
        // left out (so no hooks run); `-` reads the prompt from the input.
        "codex" => (
            fixed(&[
                "exec", "--json", "--skip-git-repo-check", "--sandbox", "read-only", "--ephemeral",
                "--ignore-user-config", "--ignore-rules", "-",
            ]),
            Some(whole),
        ),
        // Headless, no extensions; a tool that needs approval is refused.
        "gemini" => (
            fixed(&["-p", GEMINI_TASK, "-o", "stream-json", "--skip-trust", "--approval-mode", "default", "-e", "none"]),
            Some(whole),
        ),
        // `--mode ask` is Cursor's read-only mode.
        "cursor" => {
            let mut args = fixed(&["-p", "--output-format", "json", "--mode", "ask", "--trust", "--"]);
            args.push(as_argument());
            (args, None)
        }
        // `--trust-tools=` trusts no tool: answers only.
        _ => {
            let mut args = fixed(&["chat", "--no-interactive", "--trust-tools=", "--wrap", "never", "--"]);
            args.push(as_argument());
            (args, None)
        }
    }
}

/// What a CLI has printed so far, read line by line.
#[derive(Default)]
struct Printed {
    /// The answer as it grows (the CLIs that stream).
    shown: String,
    /// The whole answer, from the CLIs that give it at the end.
    finished: Option<String>,
    failed: Option<String>,
    /// Lines that are no event: usually the CLI's own words about what went wrong.
    other: String,
}

impl Printed {
    /// Takes one line; true when the text to show has grown.
    fn read(&mut self, id: &str, line: &str) -> bool {
        // Kiro answers in plain text.
        if id == "kiro" {
            self.shown.push_str(&strip_ansi(line));
            self.shown.push('\n');
            return true;
        }
        let Some(event) = serde_json::from_str::<Value>(line).ok().filter(Value::is_object) else {
            if self.other.len() < 2048 {
                self.other.push_str(line);
                self.other.push('\n');
            }
            return false;
        };
        let text = |pointer: &str| event.pointer(pointer).and_then(Value::as_str);
        let failed = event.get("is_error").and_then(Value::as_bool) == Some(true);
        match (id, text("/type").unwrap_or("")) {
            ("claude", "stream_event") if text("/event/delta/type") == Some("text_delta") => {
                self.shown.push_str(text("/event/delta/text").unwrap_or(""));
                return true;
            }
            ("gemini", "message") if text("/role") == Some("assistant") => {
                self.shown.push_str(text("/content").unwrap_or(""));
                return true;
            }
            ("claude", "result") | ("cursor", _) if failed || event.get("result").is_some() => {
                // A failure may come with its kind only ("error_max_turns").
                let said = text("/result").or(text("/subtype")).unwrap_or("").to_string();
                if failed {
                    self.failed = Some(said);
                } else {
                    self.finished = Some(said);
                }
            }
            ("codex", "item.completed") if text("/item/type") == Some("agent_message") => {
                self.finished = text("/item/text").map(str::to_string);
            }
            ("codex", "turn.failed") | ("codex", "error") => {
                self.failed = Some(text("/error/message").or(text("/message")).unwrap_or("").to_string());
            }
            ("gemini", "result") if text("/status") == Some("error") => {
                self.failed = Some(text("/error/message").unwrap_or("").to_string());
            }
            _ => {}
        }
        false
    }
}

pub enum Failure {
    /// The user stopped it: nothing to say, and no other CLI is tried.
    Cancelled,
    /// One line, to show.
    Other(String),
}

/// One question to `cli`. `on_text` gets the answer as it grows, for the CLIs
/// that print it bit by bit. Blocking: call it off the async runtime.
pub fn ask(
    cli: &Cli,
    system: &str,
    conversation: &str,
    handle: &Handle,
    mut on_text: impl FnMut(&str),
) -> Result<String, Failure> {
    let name = cli.name;
    let fail = |text: String| Failure::Other(text);
    let program = cli.program().ok_or_else(|| fail(tf("{name} is not installed.", &[("name", name)])))?;
    let (args, stdin) = invocation(cli.id, system, conversation);
    let spec = Spec { args, stdin, cwd: private_dir(), timeout: TIME_LIMIT, max_output: MAX_OUTPUT };

    let mut printed = Printed::default();
    let run = program::run(&program, &spec, handle, |line| {
        if printed.read(cli.id, line) {
            on_text(printed.shown.trim());
        }
    });
    let output = match run {
        Ok(output) => output,
        Err(Stop::Cancelled) => return Err(Failure::Cancelled),
        Err(Stop::TimedOut) => return Err(fail(tf("{name} took too long to answer.", &[("name", name)]))),
        Err(Stop::TooLarge) => return Err(fail(t("The answer is too large."))),
        Err(Stop::NotStarted(why)) => return Err(fail(format!("{name}: {why}"))),
    };

    let said = |words: &str| {
        let lower = words.to_lowercase();
        let signed_out = ["not logged in", "/login", "no authentication", "set an auth method", "invalid authentication", "unauthenticated"];
        if signed_out.iter().any(|sign| lower.contains(sign)) {
            return fail(tf("{name} is not signed in. Sign in from a terminal, then try again.", &[("name", name)]));
        }
        match first_words(words) {
            line if line.is_empty() => fail(t("No response text.")),
            line => fail(format!("{name}: {line}")),
        }
    };
    if let Some(failed) = &printed.failed {
        return Err(said(failed));
    }
    let answer = printed.finished.as_deref().unwrap_or(&printed.shown).trim();
    // Kiro prints its answer after a `>` mark, and its errors with no answer.
    let answer = if cli.id == "kiro" { answer.trim_start_matches('>').trim() } else { answer };
    if !answer.is_empty() && (output.success || cli.id != "kiro") {
        return Ok(answer.to_string());
    }
    Err(said(&format!("{}\n{}", output.stderr, printed.other)))
}

/// An empty folder of METEOR V1's own: no project, no instructions and no files
/// for a CLI to pick up.
fn private_dir() -> PathBuf {
    let dir = notch_core::win::local_dir().join("cli-chat");
    let _ = std::fs::create_dir_all(&dir);
    dir
}
