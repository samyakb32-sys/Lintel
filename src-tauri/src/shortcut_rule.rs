// Which shortcuts are METEOR V1's own, and which of them may be given another icon (brand_icon.rs).
// Plain rules over paths as text: no file is opened here, so they are tested without a desktop
// (`cargo test -p lintel shortcut_rule`; the file stands alone, so `rustc --test` runs it too).

use std::path::{Path, PathBuf};

/// A path as Windows compares it: case, the way a slash leans, quotes and a slash at the end do not count.
pub fn plain(path: &str) -> String {
    path.trim().trim_matches('"').replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

/// Two names of one file. Nothing is the same as an empty name.
pub fn same_path(a: &str, b: &str) -> bool {
    !plain(a).is_empty() && plain(a) == plain(b)
}

/// The only shortcuts this app ever opens: the two the installer makes for this user, by the name
/// it gives them, in the user's own Desktop and Start menu Programs folders. Never a folder shared
/// by every user, never a folder below these, never a shortcut by another name.
pub fn own_links(desktop: &Path, programs: &Path, product: &str) -> [PathBuf; 2] {
    let name = format!("{product}.lnk");
    [desktop.join(&name), programs.join(&name)]
}

/// One of those is touched only while it starts this very program (`exe`), and only while its icon
/// is still the program's own (none named, or the program itself) or one this app wrote into its
/// `icons` folder. An icon the user chose by hand is theirs.
pub fn may_change(target: &str, icon: &str, exe: &str, icons: &str) -> bool {
    let written_here = !plain(icons).is_empty() && plain(icon).starts_with(&format!("{}\\", plain(icons)));
    same_path(target, exe) && (plain(icon).is_empty() || same_path(icon, exe) || written_here)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &str = r"C:\Users\ada\AppData\Local\lintel\lintel.exe";
    const ICONS: &str = r"C:\Users\ada\AppData\Local\lintel\icons";

    #[test]
    fn paths_compare_as_windows_compares_them() {
        assert!(same_path(EXE, r#""c:/users/ADA/appdata/local/lintel/LINTEL.EXE""#));
        assert!(same_path(ICONS, &format!("{ICONS}\\")));
        assert!(!same_path(EXE, r"C:\Users\ada\AppData\Local\lintel\lintel-hook.exe"));
        assert!(!same_path("", ""));
    }

    #[test]
    fn only_the_two_shortcuts_the_installer_makes_for_this_user() {
        let links = own_links(Path::new(r"C:\Users\ada\Desktop"), Path::new(r"C:\Users\ada\AppData\Roaming\Microsoft\Windows\Start Menu\Programs"), "lintel");
        assert_eq!(links[0], Path::new(r"C:\Users\ada\Desktop\lintel.lnk"));
        assert_eq!(links[1], Path::new(r"C:\Users\ada\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\lintel.lnk"));
    }

    #[test]
    fn a_shortcut_is_ours_while_it_starts_this_program() {
        assert!(may_change(EXE, "", EXE, ICONS));
        assert!(may_change(&EXE.to_uppercase(), "", EXE, ICONS));
        // Another program by the same name, another install, nothing at all.
        assert!(!may_change(r"D:\Tools\lintel\lintel.exe", "", EXE, ICONS));
        assert!(!may_change(r"C:\Windows\System32\notepad.exe", "", EXE, ICONS));
        assert!(!may_change("", "", EXE, ICONS));
    }

    #[test]
    fn its_icon_is_changed_only_while_it_is_the_programs_own_or_one_written_here() {
        assert!(may_change(EXE, EXE, EXE, ICONS));
        assert!(may_change(EXE, &format!("{ICONS}\\mascot-00ff.ico"), EXE, ICONS));
        // The user's own choice stays, and so does a file that only sits beside the folder.
        assert!(!may_change(EXE, r"C:\Users\ada\Pictures\mine.ico", EXE, ICONS));
        assert!(!may_change(EXE, r"C:\Users\ada\AppData\Local\lintel\icons-old\a.ico", EXE, ICONS));
        assert!(!may_change(EXE, r"C:\Windows\System32\shell32.dll", EXE, ICONS));
        // No folder of ours known: nothing counts as written here.
        assert!(!may_change(EXE, r"\mascot.ico", EXE, ""));
    }
}
