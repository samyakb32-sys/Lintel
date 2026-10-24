// The one folder the user lets the hub read. The agents' pages cannot reach the disk:
// the user browses this folder in the hub, picks a file and attaches its text to a
// hand-off, and it goes to an agent only when that hand-off is approved.
//
// Read-only. Everything is resolved against the chosen folder and refused if it ends up
// outside it (`..`, a link that leads out); a file over 200 KB, one that is not text and
// the ones that usually hold secrets are refused.

use std::path::{Path, PathBuf};

use notch_core::Ctx;
use serde_json::{json, Value};

const MAX_READ: u64 = 200 * 1024;
const MAX_LIST: usize = 500;
const SECRET_NAMES: &[&str] = &[".env", ".npmrc", ".netrc", "id_rsa", "id_ed25519", "credentials", "secrets.json"];
const SECRET_EXTENSIONS: &[&str] = &["pem", "key", "pfx", "p12", "ppk", "kdbx"];

fn secret(path: &Path) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    name.starts_with(".env") || SECRET_NAMES.contains(&name.as_str()) || SECRET_EXTENSIONS.contains(&ext.as_str())
}

fn root(cx: &Ctx) -> Result<PathBuf, String> {
    let chosen = cx.settings("hub")["folder"].as_str().unwrap_or_default().to_string();
    if chosen.is_empty() {
        return Err("no folder is chosen".into());
    }
    std::fs::canonicalize(chosen).map_err(|_| "the chosen folder is gone".to_string())
}

/// `rel` (slashes, no leading one) inside the chosen folder, or an error when it would leave it.
fn inside(root: &Path, rel: &str) -> Result<PathBuf, String> {
    if Path::new(rel).has_root() {
        return Err("a path inside the folder is expected".into());
    }
    let path = std::fs::canonicalize(root.join(rel)).map_err(|e| e.to_string())?;
    path.starts_with(root).then_some(path).ok_or_else(|| "that is outside the chosen folder".to_string())
}

pub fn current(cx: &Ctx) -> Value {
    json!({ "folder": cx.settings("hub")["folder"] })
}

/// Windows' folder dialog; the choice is remembered in the feature's settings.
pub fn pick(cx: &Ctx) -> Result<Value, String> {
    if let Some(path) = svc_shell::pick_folder("Choose a folder for your agents") {
        cx.patch_settings("hub", json!({ "folder": path.to_string_lossy() }));
    }
    Ok(current(cx))
}

pub fn clear(cx: &Ctx) -> Value {
    cx.patch_settings("hub", json!({ "folder": "" }));
    current(cx)
}

/// The entries of a folder (the chosen one when `rel` is empty): folders first, then files by name.
pub fn list(cx: &Ctx, rel: &str) -> Result<Value, String> {
    let root = root(cx)?;
    let dir = inside(&root, rel)?;
    let mut entries: Vec<(bool, String, u64, bool)> = std::fs::read_dir(&dir)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let meta = entry.metadata().ok()?;
            let path = entry.path();
            Some((meta.is_dir(), entry.file_name().to_string_lossy().into_owned(), meta.len(), secret(&path)))
        })
        .collect();
    entries.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase())));
    let truncated = entries.len() > MAX_LIST;
    let at = rel.trim_matches('/');
    let rows: Vec<Value> = entries
        .into_iter()
        .take(MAX_LIST)
        .map(|(dir, name, size, blocked)| {
            let path = if at.is_empty() { name.clone() } else { format!("{at}/{name}") };
            json!({ "name": name, "path": path, "dir": dir, "size": size, "blocked": blocked })
        })
        .collect();
    Ok(json!({ "path": at, "entries": rows, "truncated": truncated }))
}

/// The text of one file.
pub fn read(cx: &Ctx, rel: &str) -> Result<Value, String> {
    let path = inside(&root(cx)?, rel)?;
    if secret(&path) {
        return Err("files that usually hold secrets are not shared".into());
    }
    let meta = std::fs::metadata(&path).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("that is not a file".into());
    }
    if meta.len() > MAX_READ {
        return Err("that file is over 200 KB".into());
    }
    let text = String::from_utf8(std::fs::read(&path).map_err(|e| e.to_string())?).map_err(|_| "that is not a text file".to_string())?;
    Ok(json!({ "name": path.file_name().map(|n| n.to_string_lossy()), "text": text }))
}
