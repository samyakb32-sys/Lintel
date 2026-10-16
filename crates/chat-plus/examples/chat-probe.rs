// Proof without the app: the window under the centre of the primary screen, and the
// reply the relay prints for an Always click. Moves nothing, shows nothing, sends nothing.
//   cargo run -p notch-feat-chat-plus --example chat-probe

#[path = "../../../hook/src/reply.rs"]
#[allow(dead_code)]
mod reply;

use notch_feat_chat_plus::target::{self, Hit};
use serde_json::json;

fn main() {
    let (x, y) = target::primary_centre();
    match target::at(x, y, 0) {
        Hit::Window(t) => println!(
            "window at ({x},{y}): app={:?} title={:?} frame={:?}",
            target::app_name(t.pid),
            t.title,
            t.rect
        ),
        Hit::Own => println!("window at ({x},{y}): one of METEOR V1's own"),
        Hit::Nothing => println!("window at ({x},{y}): none"),
    }

    let request = json!({
        "hook_event_name": "PermissionRequest",
        "tool_name": "Bash",
        "tool_input": { "command": "cargo test" },
        "permission_suggestions": [{
            "type": "addRules",
            "rules": [{ "toolName": "Bash", "ruleContent": "cargo test:*" }],
            "behavior": "allow",
            "destination": "localSettings"
        }]
    });
    let s = request.get("permission_suggestions");
    println!("always, Claude Code : {}", reply::stdout_with("", "PermissionRequest", Some("always"), None, s).unwrap());
    println!("allow , Claude Code : {}", reply::stdout_with("", "PermissionRequest", Some("allow"), None, s).unwrap());
    println!("always, no suggestion: {}", reply::stdout_with("", "PermissionRequest", Some("always"), None, None).unwrap());
    println!("always, Codex       : {}", reply::stdout_with("codex", "PermissionRequest", Some("always"), None, s).unwrap());
    println!("no click            : {:?}", reply::stdout_with("", "PermissionRequest", None, None, s));
}
