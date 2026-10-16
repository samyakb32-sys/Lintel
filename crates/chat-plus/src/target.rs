// Which window is under a screen point. Skips click-through overlays (the halo
// among them), tool windows, the shell and cloaked windows; a point over one of
// METEOR V1's own windows is `Own`, so dropping back on the island attaches nothing.

use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::UI::WindowsAndMessaging::*;

#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub hwnd: isize,
    pub pid: u32,
    pub title: String,
    /// Left, top, right, bottom in screen pixels (the visible frame, no shadow).
    pub rect: (i32, i32, i32, i32),
}

pub enum Hit {
    Nothing,
    Own,
    Window(Target),
}

struct Scan {
    x: i32,
    y: i32,
    skip: isize,
    me: u32,
    hit: Hit,
}

const SHELL: &[&str] = &["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd"];

unsafe extern "system" fn each(hwnd: HWND, lp: LPARAM) -> BOOL {
    let s = unsafe { &mut *(lp.0 as *mut Scan) };
    unsafe {
        if hwnd.0 as isize == s.skip || !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return true.into();
        }
        let ex = WINDOW_EX_STYLE(GetWindowLongW(hwnd, GWL_EXSTYLE) as u32);
        let mut cloaked = 0u32;
        let _ = DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut _, 4);
        if cloaked != 0 {
            return true.into();
        }
        let mut r = RECT::default();
        if DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, &mut r as *mut _ as *mut _, size_of::<RECT>() as u32).is_err()
            && GetWindowRect(hwnd, &mut r).is_err()
        {
            return true.into();
        }
        if s.x < r.left || s.x >= r.right || s.y < r.top || s.y >= r.bottom {
            return true.into();
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == s.me {
            s.hit = Hit::Own;
            return false.into();
        }
        if ex.contains(WS_EX_TRANSPARENT) || ex.contains(WS_EX_TOOLWINDOW) {
            return true.into();
        }
        let mut class = [0u16; 64];
        let n = GetClassNameW(hwnd, &mut class);
        if SHELL.contains(&String::from_utf16_lossy(&class[..n.max(0) as usize]).as_str()) {
            return true.into();
        }
        let mut text = [0u16; 512];
        let n = GetWindowTextW(hwnd, &mut text);
        if n == 0 {
            return true.into();
        }
        s.hit = Hit::Window(Target {
            hwnd: hwnd.0 as isize,
            pid,
            title: String::from_utf16_lossy(&text[..n as usize]),
            rect: (r.left, r.top, r.right, r.bottom),
        });
    }
    false.into()
}

/// The window at screen point (x, y), front to back. `skip` is a window to ignore (the halo).
pub fn at(x: i32, y: i32, skip: isize) -> Hit {
    let mut s = Scan { x, y, skip, me: std::process::id(), hit: Hit::Nothing };
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut s as *mut _ as isize));
    }
    s.hit
}

/// `Code.exe` becomes `Code`.
pub fn app_name(pid: u32) -> String {
    let exe = notch_core::win::exe_name_of_pid(pid).unwrap_or_default();
    let stem = exe.strip_suffix(".exe").or_else(|| exe.strip_suffix(".EXE")).unwrap_or(&exe);
    let mut c = stem.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

pub fn cursor() -> Option<(i32, i32)> {
    let mut p = windows::Win32::Foundation::POINT::default();
    unsafe { GetCursorPos(&mut p).ok()? };
    Some((p.x, p.y))
}

pub fn primary_centre() -> (i32, i32) {
    unsafe { (GetSystemMetrics(SM_CXSCREEN) / 2, GetSystemMetrics(SM_CYSCREEN) / 2) }
}
