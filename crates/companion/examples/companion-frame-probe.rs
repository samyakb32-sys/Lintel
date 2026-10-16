// Proves the companion's window frame logic without the app:
// `cargo run -p notch-feat-companion --example companion-frame-probe`.
// One 112 px window at the bottom left of the screen for a moment, never activated:
// it has the style the host gives a frameless window and gives the frame no room, as the
// host does. The messages a click and a drag send go to it by hand (no input is made), and
// the window's own pixels are read back: any that is not its fill was drawn by Windows as a
// frame. Three rounds: the host's style alone, then with a region (the fault: a caption
// appears), then with a region after `unframe` (the fix: nothing does, and no caption or
// border is left in the style or in the window's metrics). Exits 1 when a round is not so.

use notch_feat_companion::unframe;
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateEllipticRgn, CreateSolidBrush, DeleteDC, DeleteObject, GetPixel,
    GetWindowDC, PtInRegion, RedrawWindow, ReleaseDC, SelectObject, SetWindowRgn, HRGN, RDW_ERASE, RDW_INVALIDATE, RDW_UPDATENOW,
    SRCCOPY,
};
use windows::Win32::UI::Input::KeyboardAndMouse::GetActiveWindow;
use windows::Win32::UI::WindowsAndMessaging::*;

const SIDE: i32 = 112;
const FILL: COLORREF = COLORREF(0x000e_0c0b);
const FRAME: u32 = WS_OVERLAPPEDWINDOW.0;

unsafe extern "system" fn proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // The host's frameless window: the frame is in the style and gets no room.
    if msg == WM_NCCALCSIZE && wp.0 != 0 {
        return LRESULT(0);
    }
    DefWindowProcW(h, msg, wp, lp)
}

/// Pixels of the window (of `shape`, when it has a region) that are not its own fill, and the rows they are in.
unsafe fn drawn(h: HWND, shape: Option<HRGN>) -> (u32, i32, i32) {
    // One copy of the window: reading it pixel by pixel takes most of a minute.
    let dc = GetWindowDC(Some(h));
    let copy = CreateCompatibleDC(Some(dc));
    let bitmap = CreateCompatibleBitmap(dc, SIDE, SIDE);
    let old = SelectObject(copy, bitmap.into());
    let _ = BitBlt(copy, 0, 0, SIDE, SIDE, Some(dc), 0, 0, SRCCOPY);
    ReleaseDC(Some(h), dc);
    let (mut n, mut top, mut bottom) = (0, SIDE, -1);
    for y in 0..SIDE {
        for x in 0..SIDE {
            if shape.is_none_or(|s| PtInRegion(s, x, y).as_bool()) && GetPixel(copy, x, y) != FILL {
                n += 1;
                (top, bottom) = (top.min(y), bottom.max(y));
            }
        }
    }
    SelectObject(copy, old);
    let _ = DeleteObject(bitmap.into());
    let _ = DeleteDC(copy);
    (n, top, bottom)
}

/// Sends what a click and a drag send, each to a freshly painted window; the most pixels any of them drew.
unsafe fn round(h: HWND, shape: Option<HRGN>, name: &str) -> u32 {
    let title = w!("meteor");
    let steps: [(&str, u32, usize, isize); 6] = [
        ("click (WM_MOUSEACTIVATE)", WM_MOUSEACTIVATE, h.0 as usize, (HTCLIENT | (WM_LBUTTONDOWN << 16)) as isize),
        ("activated (WM_NCACTIVATE 1)", WM_NCACTIVATE, 1, 0),
        ("deactivated (WM_NCACTIVATE 0)", WM_NCACTIVATE, 0, 0),
        ("frame paint (WM_NCPAINT)", WM_NCPAINT, 1, 0),
        ("title (WM_SETTEXT)", WM_SETTEXT, 0, title.0 as isize),
        ("drag (SetWindowPos)", 0, 0, 0),
    ];
    println!("{name}");
    let mut most = 0;
    for (what, msg, wp, lp) in steps {
        let _ = RedrawWindow(Some(h), None, None, RDW_ERASE | RDW_INVALIDATE | RDW_UPDATENOW);
        if msg == 0 {
            // The drag: the window follows the cursor, as crates/companion/src/lib.rs moves it.
            for d in [4, 8, 0] {
                let _ = SetWindowPos(h, None, 8 + d, GetSystemMetrics(SM_CYSCREEN) - 200 - d, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
            }
        } else {
            SendMessageW(h, msg, Some(WPARAM(wp)), Some(LPARAM(lp)));
        }
        let (n, top, bottom) = drawn(h, shape);
        match n {
            0 => println!("  {what:<32} nothing drawn by Windows"),
            _ => println!("  {what:<32} {n} px drawn by Windows, rows {top} to {bottom}"),
        }
        most = most.max(n);
    }
    most
}

fn main() {
    unsafe {
        let class = WNDCLASSW { lpfnWndProc: Some(proc), hbrBackground: CreateSolidBrush(FILL), lpszClassName: w!("companion-frame-probe"), ..Default::default() };
        RegisterClassW(&class);
        // What tao's `to_window_styles` gives a frameless, topmost window that cannot be focused.
        let h = CreateWindowExW(
            WS_EX_WINDOWEDGE | WS_EX_ACCEPTFILES | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
            class.lpszClassName,
            w!("meteor"),
            WS_CAPTION | WS_CLIPSIBLINGS | WS_SYSMENU | WS_SIZEBOX | WS_MAXIMIZEBOX | WS_MINIMIZEBOX,
            8,
            GetSystemMetrics(SM_CYSCREEN) - 200,
            SIDE,
            SIDE,
            None,
            None,
            None,
            None,
        )
        .expect("window");
        let _ = SetWindowPos(h, None, 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        let _ = ShowWindow(h, SW_SHOWNOACTIVATE);

        let plain = round(h, None, "host's style, no region (every other frameless window)");
        // The character's body and head, as the page asks for them with an outfit on. The window
        // takes its region for itself, so the probe keeps a second one to look through.
        let shape = || CreateEllipticRgn(20, 12, 92, 96);
        SetWindowRgn(h, Some(shape()), true);
        let fault = round(h, Some(shape()), "host's style, with a region (the companion before the fix)");
        unframe(h);
        let fixed = round(h, Some(shape()), "after unframe, with a region (the companion now)");

        let style = GetWindowLongW(h, GWL_STYLE) as u32;
        let mut info = WINDOWINFO { cbSize: std::mem::size_of::<WINDOWINFO>() as u32, ..Default::default() };
        let mut bar = TITLEBARINFO { cbSize: std::mem::size_of::<TITLEBARINFO>() as u32, ..Default::default() };
        let _ = GetWindowInfo(h, &mut info);
        let _ = GetTitleBarInfo(h, &mut bar);
        let no_bar = bar.rgstate[0] & 0x8000 != 0; // STATE_SYSTEM_INVISIBLE: the window has no title bar
        println!("style {style:#010x} (frame bits {:#010x}), ex style {:#010x}", style & FRAME, info.dwExStyle.0);
        println!("borders {}x{}, client = window: {}, title bar: {}", info.cxWindowBorders, info.cyWindowBorders, info.rcClient == info.rcWindow, if no_bar { "none" } else { "present" });
        // The host's library writing its style back is undone by the next call.
        SetWindowLongW(h, GWL_STYLE, (style | FRAME) as i32);
        unframe(h);
        let again = GetWindowLongW(h, GWL_STYLE) as u32 & FRAME == 0;
        let kept = info.dwExStyle.contains(WS_EX_NOACTIVATE | WS_EX_TOPMOST);
        let quiet = GetForegroundWindow() != h && GetActiveWindow() != h;
        let _ = DestroyWindow(h);

        let checks = [
            ("no region: Windows draws no frame", plain == 0),
            ("region: Windows draws a caption (the fault)", fault > 0),
            ("region after unframe: nothing drawn", fixed == 0),
            ("no caption, sizing frame or buttons in the style", style & FRAME == 0),
            ("no borders, no title bar, client is the whole window", info.cxWindowBorders == 0 && info.cyWindowBorders == 0 && no_bar && info.rcClient == info.rcWindow),
            ("the style written back is stripped again", again),
            ("still topmost and not to be activated", kept),
            ("it never became the active window or the one in front", quiet),
        ];
        for (what, ok) in checks {
            println!("{} {what}", if ok { "ok  " } else { "FAIL" });
        }
        std::process::exit(checks.iter().any(|c| !c.1) as i32);
    }
}
