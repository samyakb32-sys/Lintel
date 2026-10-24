// The one folder the user shares with the hub. The agents' pages cannot reach the disk:
// the user browses this folder in the hub, picks a file and attaches its text to a
// hand-off, and it goes to an agent only when that hand-off is approved.
//
// Reads freely (an agent asks, the user clicks, or auto mode); writes only after the user approved that
// exact file and text, never in auto mode. Everything is resolved against the chosen folder and refused if it ends up
// outside it (`..`, a link that leads out); a file over 200 KB, one that is not text and
// the ones that usually hold secrets are refused.

use std::path::{Component, Path, PathBuf};

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

/// Where a file that may not exist yet would go: `rel` is plain (no `..`, no drive), and what exists of its
/// way down resolves inside the chosen folder.
fn inside_new(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let plain = Path::new(rel);
    if rel.is_empty() || plain.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err("a plain path inside the folder is expected".into());
    }
    let target = root.join(plain);
    let mut ancestor = target.as_path();
    while !ancestor.exists() {
        ancestor = ancestor.parent().ok_or("that path does not lead anywhere")?;
    }
    let real = std::fs::canonicalize(ancestor).map_err(|e| e.to_string())?;
    if !real.starts_with(root) {
        return Err("that is outside the chosen folder".into());
    }
    if secret(&target) {
        return Err("files that usually hold secrets are not written".into());
    }
    if target.is_dir() {
        return Err("that is a folder".into());
    }
    Ok(target)
}

/// Checks that a write could happen, and says whether it would replace a file.
pub fn check_write(cx: &Ctx, rel: &str, bytes: usize) -> Result<bool, String> {
    if bytes as u64 > MAX_READ {
        return Err("that file would be over 200 KB".into());
    }
    Ok(inside_new(&root(cx)?, rel)?.exists())
}

/// Writes one file of the shared folder, after the user said yes. A file that is replaced is first
/// copied to the hub's own data folder (never into the shared one). Written beside the target and
/// renamed over it, so a crash never leaves half a file.
pub fn write(cx: &Ctx, rel: &str, text: &str) -> Result<Value, String> {
    let target = inside_new(&root(cx)?, rel)?;
    if text.len() as u64 > MAX_READ {
        return Err("that file would be over 200 KB".into());
    }
    let replaced = target.exists();
    if replaced {
        let backups = cx.data_dir("hub").join("backups");
        std::fs::create_dir_all(&backups).map_err(|e| e.to_string())?;
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        std::fs::copy(&target, backups.join(format!("{stamp}-{name}"))).map_err(|e| e.to_string())?;
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = target.with_extension("hub-tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &target).map_err(|e| e.to_string())?;
    Ok(json!({ "bytes": text.len(), "replaced": replaced }))
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

#[cfg(test)]
mod tests {
    use super::inside_new;

    #[test]
    fn a_new_file_stays_inside_the_folder() {
        let dir = std::env::temp_dir().join(format!("hub-folder-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("docs")).unwrap();
        let root = std::fs::canonicalize(&dir).unwrap();

        assert!(inside_new(&root, "docs/new.md").is_ok());
        assert!(inside_new(&root, "deeper/than/it/is/new.md").is_ok());
        assert!(inside_new(&root, "../escape.md").is_err());
        assert!(inside_new(&root, "docs/../../escape.md").is_err());
        assert!(inside_new(&root, "/etc/passwd").is_err());
        // A drive prefix only means something on Windows, where the hub runs.
        #[cfg(windows)]
        assert!(inside_new(&root, "C:\\x.md").is_err());
        assert!(inside_new(&root, "").is_err());
        assert!(inside_new(&root, ".env").is_err());
        assert!(inside_new(&root, "keys/server.pem").is_err());
        assert!(inside_new(&root, "docs").is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
