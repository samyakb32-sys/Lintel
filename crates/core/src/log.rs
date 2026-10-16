// Small append-only log at %LOCALAPPDATA%\meteor\meteor.log. Nothing leaves the
// machine.

use std::io::Write;

use crate::win;

pub fn line(message: impl AsRef<str>) {
    let t = win::local_time();
    let stamp = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        t.year, t.month, t.day, t.hour, t.minute, t.second
    );
    let dir = win::local_dir();
    if win::ensure_private_dir(&dir).is_err() {
        return;
    }
    let path = dir.join(format!("{}.log", notch_brand::SLUG));
    // Keep it from growing forever: start fresh past ~1 MB.
    if std::fs::metadata(&path).map(|m| m.len() > 1_000_000).unwrap_or(false) {
        let _ = std::fs::remove_file(&path);
    }
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    // Readable by us only, like the macOS log.
    if let Ok(mut file) = options.open(path) {
        let _ = writeln!(file, "{stamp} {}", one_line(message.as_ref()));
    }
}

/// An event name, a request id or an error text can come from another program:
/// a line break in one would write a second, forged entry. Every control
/// character becomes a space.
fn one_line(text: &str) -> String {
    text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn control_characters_stay_out_of_the_log() {
        assert_eq!(super::one_line("SessionStart\nforged"), "SessionStart forged");
        assert_eq!(super::one_line("ok\r\nok\tok"), "ok  ok ok");
    }
}
