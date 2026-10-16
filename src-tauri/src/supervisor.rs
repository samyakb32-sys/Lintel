// Brings the app back after it dies of an error.
//
// The first process only watches. It starts the app again as a child of itself and waits for it
// (blocked on the child: no timer, no polling, 0 % CPU). When the child quits the way the user asked
// (Quit in the tray, a second copy that gave way: exit code 0) or is ended from outside (Task Manager,
// the installer: exit code 1) the watcher ends too. When the child dies of an error (a Rust panic, which
// aborts, an access violation, a stack overflow: an exit code that is an NTSTATUS error) the watcher
// starts it again after a short pause, and gives up after a few restarts in a minute so that an app that
// dies at once every time is not started for ever. Debug builds are not watched.

use std::process::Command;
use std::time::{Duration, Instant};

/// Set in the environment of the child: this process is the app, not the watcher.
const CHILD_MARK: &str = "METEOR_SUPERVISED";
/// Before the app is started again.
const PAUSE: Duration = Duration::from_secs(2);
/// More restarts than this inside one window and the watcher gives up.
const MAX_RESTARTS: usize = 5;
const WINDOW: Duration = Duration::from_secs(60);

/// A Windows exit code that says the process died of an error and did not quit.
pub fn crashed(code: i32) -> bool {
    (code as u32) >= 0xC000_0000
}

/// The restarts of the last minute.
#[derive(Default)]
pub struct Budget {
    starts: Vec<Instant>,
}

impl Budget {
    /// Another restart now? Counts it when yes.
    pub fn allow(&mut self, now: Instant) -> bool {
        self.starts.retain(|t| now.duration_since(*t) < WINDOW);
        if self.starts.len() >= MAX_RESTARTS {
            return false;
        }
        self.starts.push(now);
        true
    }
}

/// `true`: this process was the watcher and is done, `main` returns. `false`: run the app in this process.
pub fn watch() -> bool {
    if cfg!(debug_assertions) || std::env::var_os(CHILD_MARK).is_some() {
        return false;
    }
    let Ok(exe) = std::env::current_exe() else { return false };
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let mut budget = Budget::default();
    let mut first = true;
    loop {
        let child = Command::new(&exe).args(&args).env(CHILD_MARK, "1").spawn();
        let Ok(mut child) = child else {
            // Could not start a child at all: the very first time, run the app here instead.
            return !first;
        };
        first = false;
        let Ok(status) = child.wait() else { return true };
        match status.code() {
            Some(code) if crashed(code) => {
                notch_core::log::line(format!("the app stopped with an error ({code:#x}); starting it again"));
            }
            _ => return true,
        }
        if !budget.allow(Instant::now()) {
            notch_core::log::line("the app stopped with an error again and again; not starting it any more");
            return true;
        }
        std::thread::sleep(PAUSE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_error_exit_code_is_a_crash() {
        assert!(!crashed(0));
        assert!(!crashed(1));
        assert!(crashed(0xC000_0005_u32 as i32)); // access violation
        assert!(crashed(0xC000_0409_u32 as i32)); // a Rust abort
        assert!(!crashed(0x4000_0000));
    }

    #[test]
    fn a_dying_app_is_started_again_a_few_times_a_minute_and_then_left() {
        let mut budget = Budget::default();
        let t = Instant::now();
        for _ in 0..MAX_RESTARTS {
            assert!(budget.allow(t));
        }
        assert!(!budget.allow(t));
        assert!(budget.allow(t + WINDOW + Duration::from_secs(1)));
    }
}
