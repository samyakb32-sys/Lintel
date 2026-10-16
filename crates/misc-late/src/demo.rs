// Demo: a scripted session played through the deployed relay, exactly as
// `npm run fake-session` does, so the island shows what an agent session looks
// like without one: steps, a readable diff, a permission request, a question,
// a second agent, the finish. Nothing in it runs: the "commands" are text for
// the cards. The two requests wait on the island like real ones (an answer
// ends the wait; so does Stop).
//
// debt: an Allow or Deny clicked in the demo is counted by the weekly recap like any other.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// A few seconds are enough: the relay waits for a card at most ~108 s.
const WAIT_MAX: Duration = Duration::from_secs(110);

pub fn relay() -> PathBuf {
    notch_core::win::local_dir().join("bin").join(notch_brand::HOOK_EXE)
}

/// The script: (seconds to wait before, session, event).
fn script() -> Vec<(f32, &'static str, Value)> {
    let ask = json!({
        "hook_event_name": "PermissionRequest", "tool_name": "AskUserQuestion",
        "tool_input": { "questions": [{
            "question": "Which folder should the build go to?", "header": "Folder", "multiSelect": false,
            "options": [
                { "label": "dist", "description": "The usual one" },
                { "label": "build", "description": "The one just tidied" },
            ],
        }] },
    });
    vec![
        (0.0, "a", json!({ "hook_event_name": "SessionStart", "source": "startup" })),
        (0.8, "a", json!({ "hook_event_name": "UserPromptSubmit", "prompt": "Tidy up the build folder" })),
        (1.2, "a", json!({ "hook_event_name": "PreToolUse", "tool_name": "Read", "tool_input": { "file_path": "C:\\demo\\sample-app\\package.json" } })),
        (0.9, "a", json!({ "hook_event_name": "PostToolUse", "tool_name": "Read" })),
        (0.6, "b", json!({ "hook_event_name": "SessionStart", "source": "startup", "meteor_agent": "codex" })),
        (0.3, "b", json!({ "hook_event_name": "PreToolUse", "tool_name": "Bash", "tool_input": { "command": "npm test" }, "meteor_agent": "codex" })),
        (1.0, "a", json!({ "hook_event_name": "PreToolUse", "tool_name": "Edit", "tool_input": {
            "file_path": "C:\\demo\\sample-app\\src\\build.ts",
            "old_string": "const out = \"build\";\nclean(out);\n",
            "new_string": "const out = \"dist\";\nclean(out, { keep: [\"cache\"] });\n" } })),
        (0.9, "a", json!({ "hook_event_name": "PostToolUse", "tool_name": "Edit" })),
        (0.8, "a", json!({ "hook_event_name": "PermissionRequest", "tool_name": "Bash", "tool_input": { "command": "rm -rf ./build  # demo: nothing runs" } })),
        (0.6, "a", ask),
        (0.8, "b", json!({ "hook_event_name": "PostToolUse", "tool_name": "Bash", "meteor_agent": "codex" })),
        (0.8, "a", json!({ "hook_event_name": "Stop", "message": "Build folder tidied", "last_assistant_message": "Moved the output to dist and kept the cache." })),
        (0.5, "b", json!({ "hook_event_name": "Stop", "message": "Tests pass", "meteor_agent": "codex" })),
        (7.0, "a", json!({ "hook_event_name": "SessionEnd" })),
        (0.2, "b", json!({ "hook_event_name": "SessionEnd", "meteor_agent": "codex" })),
    ]
}

pub struct Running {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Running {
    pub fn stop(mut self) {
        self.stop.store(true, Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }

    pub fn finished(&self) -> bool {
        self.thread.as_ref().is_none_or(|t| t.is_finished())
    }
}

fn wait(child: &mut Child, stop: &AtomicBool) {
    let began = Instant::now();
    while began.elapsed() < WAIT_MAX {
        if stop.load(Relaxed) {
            let _ = child.kill();
            break;
        }
        if matches!(child.try_wait(), Ok(Some(_)) | Err(_)) {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Plays the script; `step` is told the number of the event about to be sent.
pub fn start(step: impl Fn(usize, usize) + Send + 'static, end: impl FnOnce() + Send + 'static) -> Result<Running, String> {
    let relay = relay();
    if !relay.exists() {
        return Err("The relay is not there yet. Start the app once and try again.".into());
    }
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let id = format!("demo-{}", std::process::id());
    let thread = std::thread::spawn(move || {
        let events = script();
        let total = events.len();
        for (i, (pause, who, event)) in events.into_iter().enumerate() {
            for _ in 0..(pause * 10.0) as u32 {
                if flag.load(Relaxed) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            if flag.load(Relaxed) && event["hook_event_name"] != "SessionEnd" {
                // Stopped: only the endings still go, so the demo's pills go away.
                continue;
            }
            step(i, total);
            let mut payload = event.clone();
            payload["session_id"] = json!(format!("{id}-{who}"));
            payload["cwd"] = json!(if who == "a" { "C:\\demo\\sample-app" } else { "C:\\demo\\api-server" });
            let name = event["hook_event_name"].as_str().unwrap_or("").to_string();
            let mut cmd = Command::new(&relay);
            notch_core::win::no_console(&mut cmd);
            let Ok(mut child) = cmd.arg(&name).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() else {
                continue;
            };
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(payload.to_string().as_bytes());
            }
            if name == "PermissionRequest" {
                wait(&mut child, &flag);
            } else {
                let _ = child.wait();
            }
        }
        end();
    });
    Ok(Running { stop, thread: Some(thread) })
}
