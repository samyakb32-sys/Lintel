// A one-time step for people who ran the code-named build; a later version may drop it.
// At the first start, when this product's settings folder is not there yet and the code-named one is,
// the user's files and saved keys are copied over. Copied, never moved or deleted: the old install
// keeps working until its owner removes it. The agents' hooks are not touched.

use std::path::Path;

/// The slug the product had before it was named.
pub const OLD_SLUG: &str = "notch";
/// The slug of the build that was named Lintel, before this one was renamed.
pub const PREVIOUS_SLUG: &str = "lintel";

/// Not the user's, so left behind: the staged relay, what the old installer laid down, the start counter.
const LEFT: &[&str] = &["bin", "speak", "licenses", "LICENSE", "NOTICE.md", "starting"];

fn left(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    LEFT.iter().any(|l| l.eq_ignore_ascii_case(name)) || [".exe", ".dll", ".log"].iter().any(|end| lower.ends_with(end))
}

/// Copies what is under `from` into `to` and counts the files. A file already there is kept.
fn copy_tree(from: &Path, to: &Path, top: bool) -> u64 {
    let Ok(entries) = std::fs::read_dir(from) else { return 0 };
    if std::fs::create_dir_all(to).is_err() {
        return 0;
    }
    let mut copied = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        if top && left(&name.to_string_lossy()) {
            continue;
        }
        let (source, target) = (entry.path(), to.join(&name));
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => copied += copy_tree(&source, &target, false),
            Ok(kind) if kind.is_file() && !target.exists() => copied += std::fs::copy(&source, &target).is_ok() as u64,
            _ => {}
        }
    }
    copied
}

/// The settings folder and the user's part of the data folder, old to new. `None`: nothing to do,
/// because this product's settings folder is already there or there is no code-named folder.
pub fn copy_folders(old_config: &Path, old_local: &Path, config: &Path, local: &Path) -> Option<u64> {
    if config.exists() || !(old_config.is_dir() || old_local.is_dir()) {
        return None;
    }
    let files = copy_tree(old_local, local, true) + copy_tree(old_config, config, true);
    let _ = std::fs::create_dir_all(config);
    Some(files)
}

/// The saved keys named in `keys`, from one keyring service to another. A key already saved is kept.
pub fn copy_secrets(from: &str, to: &str, keys: &[&str]) -> u64 {
    let mut copied = 0;
    for key in keys {
        let (Ok(old), Ok(new)) = (keyring::Entry::new(from, key), keyring::Entry::new(to, key)) else { continue };
        if let (Ok(value), Err(keyring::Error::NoEntry)) = (old.get_password(), new.get_password()) {
            copied += new.set_password(&value).is_ok() as u64;
        }
    }
    copied
}

/// Run once, before anything reads the settings. Says what it did, for the log. The Lintel-named build's
/// folders come first, then the code-named ones.
pub fn from_code_name() -> Option<String> {
    let (config, local) = (crate::win::config_dir(), crate::win::local_dir());
    for old_slug in [PREVIOUS_SLUG, OLD_SLUG] {
        let old = |dir: std::path::PathBuf| dir.with_file_name(old_slug);
        let Some(files) = copy_folders(&old(config.clone()), &old(local.clone()), &config, &local) else { continue };
        // Debug builds only: `METEOR_MIGRATE_KEYS` (comma-separated) stands in for the list, to try the
        // step with a made-up key and leave the real ones alone.
        let stand_in = if cfg!(debug_assertions) { std::env::var("METEOR_MIGRATE_KEYS").ok() } else { None };
        let keys: Vec<&str> = match &stand_in {
            Some(list) => list.split(',').filter(|k| !k.is_empty()).collect(),
            None => crate::secrets::KNOWN_KEYS.to_vec(),
        };
        let secrets = copy_secrets(old_slug, notch_brand::SLUG, &keys);
        return Some(format!(
            "first start: copied {files} files and {secrets} saved keys from the earlier build ({old_slug}); the old folders are left as they were"
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn the_users_files_are_copied_the_programs_are_left_and_a_second_run_does_nothing() {
        let root = std::env::temp_dir().join(format!("meteor-migrate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (old_config, old_local) = (root.join("roaming").join(OLD_SLUG), root.join("local").join(OLD_SLUG));
        let (config, local) = (root.join("roaming").join("meteor"), root.join("local").join("meteor"));
        write(&old_config.join("settings.json"), "{\"language\":\"fr\"}");
        write(&old_config.join("sounds").join("done.wav"), "wav");
        write(&old_config.join("starting"), "1");
        write(&old_local.join("recap.json"), "{}");
        write(&old_local.join("features").join("shelf").join("items.json"), "[]");
        for program in ["notch.exe", "notch-hook.exe", "uninstall.exe", "notch.log", "NOTICE.md", "bin/notch-hook.exe", "speak/onnxruntime.dll", "licenses/a.txt"] {
            write(&old_local.join(program), "x");
        }
        // The installer has already made the new data folder: that alone must not stop the step.
        write(&local.join("meteor.exe"), "new");

        assert_eq!(copy_folders(&old_config, &old_local, &config, &local), Some(4));
        assert_eq!(std::fs::read_to_string(config.join("settings.json")).unwrap(), "{\"language\":\"fr\"}");
        assert!(config.join("sounds").join("done.wav").is_file() && local.join("recap.json").is_file());
        assert!(local.join("features").join("shelf").join("items.json").is_file());
        for gone in ["notch.exe", "notch-hook.exe", "uninstall.exe", "notch.log", "NOTICE.md", "bin", "speak", "licenses"] {
            assert!(!local.join(gone).exists(), "{gone} is not the user's");
        }
        assert!(!config.join("starting").exists());
        // Copied, not moved.
        assert!(old_config.join("settings.json").is_file() && old_local.join("notch.exe").is_file());

        // Nothing when the settings folder is there, even with newer files in the old one.
        write(&old_config.join("late.json"), "{}");
        assert_eq!(copy_folders(&old_config, &old_local, &config, &local), None);
        assert!(!config.join("late.json").exists());
        // Nothing when there never was a code-named build.
        assert_eq!(copy_folders(&root.join("no"), &root.join("none"), &root.join("fresh"), &root.join("fresh-local")), None);
        assert!(!root.join("fresh").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_saved_key_is_copied_once_and_never_over_one_that_is_there() {
        // Made-up services and made-up keys: no real key is read or written. Removed at the end.
        let (from, to) = (format!("meteor-migrate-test-old-{}", std::process::id()), format!("meteor-migrate-test-new-{}", std::process::id()));
        let entry = |service: &str, key: &str| keyring::Entry::new(service, key).unwrap();
        entry(&from, "made-up-a").set_password("one").unwrap();
        entry(&from, "made-up-b").set_password("old").unwrap();
        entry(&to, "made-up-b").set_password("kept").unwrap();

        assert_eq!(copy_secrets(&from, &to, &["made-up-a", "made-up-b", "made-up-absent"]), 1);
        assert_eq!(entry(&to, "made-up-a").get_password().unwrap(), "one");
        assert_eq!(entry(&to, "made-up-b").get_password().unwrap(), "kept");
        assert_eq!(entry(&from, "made-up-a").get_password().unwrap(), "one");

        for (service, key) in [(&from, "made-up-a"), (&from, "made-up-b"), (&to, "made-up-a"), (&to, "made-up-b")] {
            entry(service, key).delete_credential().unwrap();
        }
    }
}
