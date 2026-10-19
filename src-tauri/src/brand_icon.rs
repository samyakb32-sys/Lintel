// The product's colour outside the island. The taskbar button of METEOR V1's own windows and the
// icon of METEOR V1's own shortcuts wear the mascot in the colour the user gave the VS Code pill,
// like the tray (tray.rs). The island's page draws it (src/island/tray-mascot.ts) at the start
// and when the colour changes: nothing here runs on a timer, and with the catalog's colour
// nothing here runs at all (the icon the program carries is that one).

use std::path::PathBuf;
use std::sync::Mutex;

use tauri::image::Image;
use tauri::ipc::Request;
use tauri::{AppHandle, Manager, State, WebviewWindow};

use ::windows::core::{Interface, HSTRING};
use ::windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, IPersistFile, CLSCTX_INPROC_SERVER, STGM_READWRITE};
use ::windows::Win32::System::Ole::{OleInitialize, OleUninitialize};
use ::windows::Win32::UI::Shell::{
    FOLDERID_Desktop, FOLDERID_Programs, IShellLinkW, SHChangeNotify, SHGetKnownFolderPath, ShellLink, KF_FLAG_DEFAULT,
    SHCNE_UPDATEITEM, SHCNF_PATHW,
};

use crate::shortcut_rule::{may_change, own_links, plain};
use crate::{log, platform, tray};

// ── Windows ───────────────────────────────────────────────────────────────────

/// The picture the windows wear; None: the icon the program carries.
#[derive(Default)]
pub struct WindowIcon(Mutex<Option<Image<'static>>>);

/// The mascot in the product's colour, square RGBA, for every window METEOR V1 has; an empty body
/// puts the program's own icon back.
#[tauri::command]
pub fn window_icon(app: AppHandle, held: State<WindowIcon>, request: Request<'_>) {
    let icon = tray::picture(tray::sent_bytes(&request));
    let was = std::mem::replace(&mut *held.0.lock().unwrap(), icon.clone());
    let icon = match (icon, was) {
        (Some(icon), _) => icon,
        // Back to the program's own, for windows that wore another.
        (None, Some(_)) => match app.default_window_icon() {
            Some(own) => own.clone().to_owned(),
            None => return,
        },
        (None, None) => return,
    };
    for win in app.webview_windows().values() {
        let _ = win.set_icon(icon.clone());
    }
}

/// A window that was just made wears it too (winfactory.rs).
pub fn dress(app: &AppHandle, win: &WebviewWindow) {
    if let Some(icon) = app.state::<WindowIcon>().0.lock().unwrap().clone() {
        let _ = win.set_icon(icon);
    }
}

// ── Shortcuts ─────────────────────────────────────────────────────────────────

/// The largest .ico taken from the page: eight pictures of the mascot are about 50 KB.
const ICO_LIMIT: usize = 1 << 20;

/// The same mascot as an .ico, for METEOR V1's own Desktop and Start menu shortcuts (Settings ›
/// Mascot); an empty body puts them back on the icon the program carries. The icon inside
/// `meteor.exe` cannot change while it runs, so the file goes into METEOR V1's data folder and the
/// shortcuts are pointed at it. Which shortcuts, and when: shortcut_rule.rs.
#[tauri::command]
pub fn shortcut_icon(request: Request<'_>) {
    let ico = tray::sent_bytes(&request);
    // Files and the shell: not on the main thread. Shell objects want OLE up on theirs.
    std::thread::spawn(move || {
        let done = unsafe {
            let _ = OleInitialize(None);
            let done = point_shortcuts(&ico);
            OleUninitialize();
            done
        };
        match done {
            Ok(0) => {}
            Ok(changed) => log::line(format!("shortcut icon: {changed} changed")),
            Err(why) => log::line(format!("shortcut icon: {why}")),
        }
    });
}

/// How many shortcuts were given another icon.
unsafe fn point_shortcuts(ico: &[u8]) -> Result<u32, String> {
    // One at a time: the start and a colour picked a moment later must not cross.
    static ONE: Mutex<()> = Mutex::new(());
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    if !ico.is_empty() && (ico.len() > ICO_LIMIT || !ico.starts_with(&[0, 0, 1, 0])) {
        return Err("not an icon".into());
    }
    let dir = platform::local_dir().join("icons");
    // Named after what is in it: another colour is another file, so Explorer never shows the
    // picture it remembers for the old one.
    let hash = ico.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3));
    let file = (!ico.is_empty()).then(|| dir.join(format!("mascot-{hash:016x}.ico")));
    let old: Vec<PathBuf> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| Some(path) != file.as_ref())
        .collect();
    // The program's own colour and nothing of ours on disk: no shortcut is even opened.
    if file.is_none() && old.is_empty() {
        return Ok(0);
    }
    if let Some(file) = file.as_ref().filter(|file| !file.is_file()) {
        std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(file, ico)).map_err(|e| e.to_string())?;
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?.to_string_lossy().into_owned();
    let icons = dir.to_string_lossy().into_owned();
    let want = file.map(|file| file.to_string_lossy().into_owned()).unwrap_or_default();
    let changed = relink(&exe, &icons, &want).map_err(|e| e.to_string())?;
    // Only once no shortcut can still name them.
    for stale in old {
        let _ = std::fs::remove_file(stale);
    }
    Ok(changed)
}

/// Points the shortcuts that are METEOR V1's own at `want` (empty: at the program's own icon), and
/// tells the shell about each one it wrote.
unsafe fn relink(exe: &str, icons: &str, want: &str) -> ::windows::core::Result<u32> {
    let folder = |id| -> ::windows::core::Result<PathBuf> {
        let path = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None)?;
        let text = path.to_string().unwrap_or_default();
        CoTaskMemFree(Some(path.0 as _));
        Ok(PathBuf::from(text))
    };
    let text = |buffer: &[u16]| String::from_utf16_lossy(&buffer[..buffer.iter().position(|c| *c == 0).unwrap_or(0)]);
    let mut changed = 0;
    let (desktop, programs) = (folder(&FOLDERID_Desktop)?, folder(&FOLDERID_Programs)?);
    // Debug builds only: `METEOR_LINK_DIR` stands in for both folders (its `desktop` and `programs`),
    // so the rule can be tried on shortcuts of one's own and the real ones are never opened.
    #[cfg(debug_assertions)]
    let (desktop, programs) = match std::env::var_os("METEOR_LINK_DIR").map(PathBuf::from) {
        Some(dir) => (dir.join("desktop"), dir.join("programs")),
        None => (desktop, programs),
    };
    for link in own_links(&desktop, &programs, notch_brand::NAME) {
        if !link.is_file() {
            continue;
        }
        let path = HSTRING::from(link.as_os_str());
        let shell: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        let file = shell.cast::<IPersistFile>()?;
        let (mut target, mut icon, mut index) = ([0u16; 1024], [0u16; 1024], 0i32);
        if file.Load(&path, STGM_READWRITE).is_err() || shell.GetPath(&mut target, std::ptr::null_mut(), 0).is_err() {
            continue;
        }
        let _ = shell.GetIconLocation(&mut icon, &mut index);
        let (target, icon) = (text(&target), text(&icon));
        if !may_change(&target, &icon, exe, icons) || plain(&icon) == plain(want) {
            continue;
        }
        shell.SetIconLocation(&HSTRING::from(want), 0)?;
        file.Save(&path, true)?;
        SHChangeNotify(SHCNE_UPDATEITEM, SHCNF_PATHW, Some(path.as_ptr() as _), None);
        changed += 1;
    }
    Ok(changed)
}
