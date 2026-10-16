// cargo run -p notch-feat-window-layout --example window-layout-probe
//
// Proves the Windows-facing code of the package on windows of the probe's own:
//
//   1. every placement, read back as the frame DWM reports (what is seen of
//      the window, without the resize border that is not drawn) and compared
//      with the part of the work area it should fill, worked out here and not
//      by the code under test; the gaps; the arrows' repeats; the way back;
//      a window with a minimum size and one that cannot be resized; the next
//      display when there is one;
//   2. focus following the pointer between two probe windows;
//   3. close protection on a probe window, through the real keyboard hook.
//
// What it never does: move, resize, focus, minimise or close a window it did
// not create (focus follows the pointer is given a list of the probe's own
// windows, and a confirmed close goes to a probe window by its handle), change
// a system setting, or play a sound. Close protection is driven with F24, a
// key no keyboard has and no program acts on, with the modifier state faked:
// no Alt+F4 exists anywhere while the probe runs. The pointer is moved over the
// probe's two windows for about two seconds and put back. The hooks are the
// shared hub's and go when the subscriptions are dropped; a watchdog ends the
// process after 30 seconds whatever happens, and Windows takes a process's
// hooks and windows with it.

use std::sync::atomic::{AtomicI32, AtomicIsize, AtomicU32, Ordering::SeqCst};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use notch_feat_window_layout::guard::{self, Event, Which};
use notch_feat_window_layout::layout::{self, Layout, Options};
use notch_feat_window_layout::follow;
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, SendInput, UnregisterHotKey, HOT_KEY_MODIFIERS, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT,
    KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_WIN, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_MOVE,
    MOUSEEVENTF_VIRTUALDESK, MOUSEINPUT, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos, GetForegroundWindow, GetSystemMetrics,
    GetWindowRect, IsWindow, IsZoomed, PeekMessageW, RegisterClassW, SetCursorPos, SetWindowPos, ShowWindow, TranslateMessage,
    HWND_BOTTOM, MINMAXINFO, MSG, PM_REMOVE, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_SHOWNOACTIVATE, WINDOW_EX_STYLE, WINDOW_STYLE, WM_ACTIVATE, WM_CLOSE,
    WM_GETMINMAXINFO, WM_KEYDOWN, WM_SYSKEYDOWN, WNDCLASSW, WS_CAPTION, WS_OVERLAPPED, WS_OVERLAPPEDWINDOW, WS_SYSMENU,
};

/// F24: the key the close guard is driven with.
const F24: u16 = 0x87;
/// The smallest the window with a minimum size lets itself be made (outer pixels).
const MIN_SIZE: (i32, i32) = (1100, 420);

static FAILED: AtomicU32 = AtomicU32::new(0);
static CHECKS: AtomicU32 = AtomicU32::new(0);
/// The window that enforces `MIN_SIZE`.
static STUBBORN: AtomicIsize = AtomicIsize::new(0);
/// F24 presses that reached a probe window.
static KEYS: AtomicU32 = AtomicU32::new(0);
/// Probe windows in the order they became the active one.
static ACTIVATED: Mutex<Vec<isize>> = Mutex::new(Vec::new());
static EVENTS: Mutex<Vec<Event>> = Mutex::new(Vec::new());
/// The window a confirmed close goes to.
static GUARDED: AtomicIsize = AtomicIsize::new(0);
/// Where the pointer was, to put it back (i32::MIN: it has not been moved).
static POINTER: [AtomicI32; 2] = [AtomicI32::new(i32::MIN), AtomicI32::new(0)];

fn check(ok: bool, what: &str, detail: String) {
    CHECKS.fetch_add(1, SeqCst);
    if !ok {
        FAILED.fetch_add(1, SeqCst);
    }
    println!("{} {what}: {detail}", if ok { "ok  " } else { "FAIL" });
}

fn show(r: RECT) -> String {
    format!("({},{})-({},{}) {}x{}", r.left, r.top, r.right, r.bottom, r.right - r.left, r.bottom - r.top)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_GETMINMAXINFO if STUBBORN.load(SeqCst) == hwnd.0 as isize => {
            unsafe { (*(lparam.0 as *mut MINMAXINFO)).ptMinTrackSize = POINT { x: MIN_SIZE.0, y: MIN_SIZE.1 } };
            LRESULT(0)
        }
        WM_KEYDOWN | WM_SYSKEYDOWN if wparam.0 as u16 == F24 => {
            KEYS.fetch_add(1, SeqCst);
            LRESULT(0)
        }
        WM_ACTIVATE => {
            if wparam.0 & 0xFFFF != 0 {
                ACTIVATED.lock().unwrap().push(hwnd.0 as isize);
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_CLOSE => {
            let _ = unsafe { DestroyWindow(hwnd) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// A probe window, shown without taking the keyboard. `behind` puts it under
/// every other window, where placements can be measured without covering anything.
fn window(title: windows::core::PCWSTR, style: WINDOW_STYLE, at: RECT, behind: bool) -> isize {
    unsafe {
        let module = GetModuleHandleW(None).unwrap();
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("lintel-window-layout-probe"),
            title,
            style,
            at.left,
            at.top,
            at.right - at.left,
            at.bottom - at.top,
            None,
            None,
            Some(module.into()),
            None,
        )
        .unwrap();
        if behind {
            let _ = SetWindowPos(hwnd, Some(HWND_BOTTOM), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        hwnd.0 as isize
    }
}

fn close_own(hwnd: isize) {
    unsafe {
        let _ = DestroyWindow(HWND(hwnd as *mut _));
    }
}

fn alive(hwnd: isize) -> bool {
    unsafe { IsWindow(Some(HWND(hwnd as *mut _))).as_bool() }
}

fn zoomed(hwnd: isize) -> bool {
    unsafe { IsZoomed(HWND(hwnd as *mut _)).as_bool() }
}

fn front() -> isize {
    unsafe { GetForegroundWindow() }.0 as isize
}

/// Runs the probe's windows for a while: they answer what other threads ask of them.
fn pump(ms: u64) {
    let until = Instant::now() + Duration::from_millis(ms);
    let mut msg = MSG::default();
    loop {
        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        if Instant::now() >= until {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The part of a work area between two fractions on each axis.
fn part(work: RECT, x: (f64, f64), y: (f64, f64)) -> RECT {
    let (w, h) = ((work.right - work.left) as f64, (work.bottom - work.top) as f64);
    RECT {
        left: work.left + (w * x.0).floor() as i32,
        right: work.left + (w * x.1).floor() as i32,
        top: work.top + (h * y.0).floor() as i32,
        bottom: work.top + (h * y.1).floor() as i32,
    }
}

fn outer(hwnd: isize) -> RECT {
    let mut r = RECT::default();
    unsafe {
        let _ = GetWindowRect(HWND(hwnd as *mut _), &mut r);
    }
    r
}

/// One action, its outcome, and the frame it should leave.
fn placed(layout: &Layout, hwnd: isize, action: &str, o: &Options, want: RECT, why: &str) {
    let result = layout.apply(hwnd, action, o);
    let got = layout::visible(hwnd);
    check(result == "done" && layout::close(got, want, 1), &format!("{action:<16}"), format!("{result}, seen {} = {why}; window {}", show(got), show(outer(hwnd))));
}

// ── Input the probe sends: unmarked, as another program's would be ────────────

fn tap(vk: u16, down: bool) {
    let flags = if down { KEYBD_EVENT_FLAGS(0) } else { KEYEVENTF_KEYUP };
    let input = INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(vk), wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } } };
    unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
}

/// The pointer to a point of the screen, as a mouse would move it (the hook hears it).
fn point_at(x: i32, y: i32) {
    unsafe {
        if POINTER[0].load(SeqCst) == i32::MIN {
            let mut at = POINT::default();
            let _ = GetCursorPos(&mut at);
            POINTER[1].store(at.y, SeqCst);
            POINTER[0].store(at.x, SeqCst);
        }
        let (left, top) = (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN));
        let (w, h) = (GetSystemMetrics(SM_CXVIRTUALSCREEN), GetSystemMetrics(SM_CYVIRTUALSCREEN));
        let scaled = |v: i32, size: i32| ((v as i64 * 65535 + (size as i64 - 1) / 2) / (size as i64 - 1).max(1)) as i32;
        let mi = MOUSEINPUT {
            dx: scaled(x - left, w),
            dy: scaled(y - top, h),
            mouseData: 0,
            dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
            time: 0,
            dwExtraInfo: 0,
        };
        SendInput(&[INPUT { r#type: INPUT_MOUSE, Anonymous: INPUT_0 { mi } }], std::mem::size_of::<INPUT>() as i32);
    }
}

fn pointer_back() {
    let x = POINTER[0].swap(i32::MIN, SeqCst);
    if x != i32::MIN {
        unsafe {
            let _ = SetCursorPos(x, POINTER[1].load(SeqCst));
        }
    }
}

fn middle(r: RECT) -> (i32, i32) {
    ((r.left + r.right) / 2, (r.top + r.bottom) / 2)
}

/// Alt held and nothing else: what the guard is told while F24 stands in for F4.
fn alt_held() -> [bool; 4] {
    [true, false, false, false]
}

fn events() -> Vec<Event> {
    std::mem::take(&mut *EVENTS.lock().unwrap())
}

fn main() {
    // Whatever happens: the pointer back, and out. Windows takes the hooks and the windows.
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(30));
        pointer_back();
        println!("FAIL watchdog: 30 seconds, the probe ends itself");
        std::process::exit(2);
    });
    unsafe {
        // Physical pixels everywhere, as in the app.
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let class = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: GetModuleHandleW(None).unwrap().into(), lpszClassName: w!("lintel-window-layout-probe"), ..Default::default() };
        RegisterClassW(&class);
    }

    // ── Displays ──────────────────────────────────────────────────────────────
    let all = layout::displays();
    for (i, d) in all.iter().enumerate() {
        println!("display {i}: {} work {} scaling {} % ({} dpi)", show(d.rect), show(d.work), d.dpi * 100 / 96, d.dpi);
    }
    let Some(here) = all.iter().find(|d| d.rect.left == 0 && d.rect.top == 0).or(all.first()).copied() else {
        println!("FAIL no display");
        std::process::exit(1);
    };
    let work = here.work;
    let px = |v: i32| v * here.dpi as i32 / 96;

    // ── Which shortcuts Windows still gives away ──────────────────────────────
    // Registered for an instant on this thread and released: nothing is pressed.
    let layers = [("Ctrl+Win", MOD_CONTROL | MOD_WIN), ("Ctrl+Alt+Win", MOD_CONTROL | MOD_ALT | MOD_WIN)];
    let keys = [("Left", 0x25u32), ("Right", 0x27), ("Up", 0x26), ("Down", 0x28), ("Home", 0x24), ("Backspace", 0x08), ("PageUp", 0x21), ("PageDown", 0x22)];
    for (layer, mods) in layers {
        let mut line = Vec::new();
        for (name, vk) in keys {
            let free = unsafe { RegisterHotKey(None, 1, HOT_KEY_MODIFIERS(mods.0) | MOD_NOREPEAT, vk) };
            if free.is_ok() {
                let _ = unsafe { UnregisterHotKey(None, 1) };
            }
            line.push(format!("{name} {}", if free.is_ok() { "free" } else { "TAKEN" }));
        }
        println!("hot keys {layer}+: {}", line.join(", "));
    }

    // ── Placements ────────────────────────────────────────────────────────────
    let layout = Layout::default();
    let plain = Options::default();
    let first = RECT { left: work.left + px(96), top: work.top + px(72), right: work.left + px(96) + 760, bottom: work.top + px(72) + 520 };
    let a = window(w!("METEOR V1 window-layout probe A"), WS_OVERLAPPEDWINDOW, first, true);
    pump(120);
    let start = layout::visible(a);
    let (o, v) = (outer(a), start);
    println!("probe window A: seen {}; window {}", show(v), show(o));
    let border = (v.left - o.left, v.top - o.top, o.right - v.right, o.bottom - v.bottom);
    check(
        border.0 > 0 && border.2 > 0 && border.3 > 0 && border.1 == 0,
        "resize border",
        format!("not drawn: left {} top {} right {} bottom {} px at {} %", border.0, border.1, border.2, border.3, here.dpi * 100 / 96),
    );

    let (h0, h1, t1, t2) = (0.0, 0.5, 1.0 / 3.0, 2.0 / 3.0);
    let tiles: [(&str, (f64, f64), (f64, f64), &str); 13] = [
        ("left-half", (h0, h1), (0.0, 1.0), "left half of the work area"),
        ("right-half", (h1, 1.0), (0.0, 1.0), "right half"),
        ("top-half", (0.0, 1.0), (h0, h1), "top half"),
        ("bottom-half", (0.0, 1.0), (h1, 1.0), "bottom half"),
        ("top-left", (h0, h1), (h0, h1), "top left quarter"),
        ("top-right", (h1, 1.0), (h0, h1), "top right quarter"),
        ("bottom-left", (h0, h1), (h1, 1.0), "bottom left quarter"),
        ("bottom-right", (h1, 1.0), (h1, 1.0), "bottom right quarter"),
        ("left-third", (0.0, t1), (0.0, 1.0), "left third"),
        ("center-third", (t1, t2), (0.0, 1.0), "centre third"),
        ("right-third", (t2, 1.0), (0.0, 1.0), "right third"),
        ("left-two-thirds", (0.0, t2), (0.0, 1.0), "left two thirds"),
        ("right-two-thirds", (t1, 1.0), (0.0, 1.0), "right two thirds"),
    ];
    for (action, x, y, why) in tiles {
        placed(&layout, a, action, &plain, part(work, x, y), why);
    }

    // Centre keeps the size (a quarter here); almost maximise leaves 5 % on every side.
    layout.apply(a, "top-left", &plain);
    let quarter = part(work, (0.0, 0.5), (0.0, 0.5));
    let (qw, qh) = (quarter.right - quarter.left, quarter.bottom - quarter.top);
    let (cx, cy) = ((work.left + work.right - qw) / 2, (work.top + work.bottom - qh) / 2);
    placed(&layout, a, "center", &plain, RECT { left: cx, top: cy, right: cx + qw, bottom: cy + qh }, "a quarter's size in the middle");
    let (mx, my) = ((work.right - work.left) * 5 / 100, (work.bottom - work.top) * 5 / 100);
    let inset = RECT { left: work.left + mx, top: work.top + my, right: work.right - mx, bottom: work.bottom - my };
    placed(&layout, a, "almost-maximize", &plain, inset, "the work area less 5 % on every side");

    // Gaps: 16 between windows, 8 at the screen's edges, both at 100 %.
    let gaps = Options { window_gap: 16, screen_gap: 8, ..plain };
    let (edge, half) = (px(8), px(16) / 2);
    let inner = RECT { left: work.left + edge, top: work.top + edge, right: work.right - edge, bottom: work.bottom - edge };
    let mut left = part(inner, (0.0, 0.5), (0.0, 1.0));
    left.right -= half;
    let mut right = part(inner, (0.5, 1.0), (0.0, 1.0));
    right.left += half;
    placed(&layout, a, "left-half", &gaps, left, &format!("left half, {edge} px from the edges"));
    placed(&layout, a, "right-half", &gaps, right, &format!("right half, {} px from where the left half ends", right.left - left.right));
    let mut corner = part(inner, (0.5, 1.0), (0.5, 1.0));
    corner.left += half;
    corner.top += half;
    placed(&layout, a, "bottom-right", &gaps, corner, "bottom right quarter with both gaps");

    // The arrows: a half, wider, narrower, the half again; across a half is a quarter.
    layout.apply(a, "center", &plain);
    let arrows: [(&str, RECT, &str); 8] = [
        ("left", part(work, (0.0, 0.5), (0.0, 1.0)), "left half"),
        ("left", part(work, (0.0, t2), (0.0, 1.0)), "again: left two thirds"),
        ("left", part(work, (0.0, t1), (0.0, 1.0)), "again: left third"),
        ("left", part(work, (0.0, 0.5), (0.0, 1.0)), "again: the left half"),
        ("up", part(work, (0.0, 0.5), (0.0, 0.5)), "up from the left half: top left quarter"),
        ("right", part(work, (0.5, 1.0), (0.0, 0.5)), "right from there: top right quarter"),
        ("up", part(work, (0.0, 1.0), (0.0, 0.5)), "up in its own corner: top half"),
        ("down", part(work, (0.0, 1.0), (0.5, 1.0)), "down: bottom half"),
    ];
    for (action, want, why) in arrows {
        placed(&layout, a, action, &plain, want, why);
    }
    let crossing = Options { cycle: false, ..plain };
    layout.apply(a, "right-half", &plain);
    let result = layout.apply(a, "right", &crossing);
    match layout::neighbour(&all, all.iter().position(|d| d.rect == here.rect).unwrap_or(0), layout::Dir::Right) {
        Some(next) => check(
            result == "done" && layout::close(layout::visible(a), part(all[next].work, (0.0, 0.5), (0.0, 1.0)), 1),
            "right, again    ",
            format!("{result}, on to the display at the right, its left half: {}", show(layout::visible(a))),
        ),
        None => check(
            result == "done" && layout::close(layout::visible(a), part(work, (0.5, 1.0), (0.0, 1.0)), 1),
            "right, again    ",
            format!("{result}, no display at the right: it stays on the right half"),
        ),
    }

    // The way back: every frame the window had, the first one last.
    close_own(a);
    let layout = Layout::default();
    let a = window(w!("METEOR V1 window-layout probe A"), WS_OVERLAPPEDWINDOW, first, true);
    pump(120);
    layout.apply(a, "left-half", &plain);
    layout.apply(a, "top-right", &plain);
    let back = layout.apply(a, "restore", &plain);
    check(back == "restored" && layout::close(layout::visible(a), part(work, (0.0, 0.5), (0.0, 1.0)), 1), "restore         ", format!("{back}, back on the left half: {}", show(layout::visible(a))));
    let back = layout.apply(a, "restore", &plain);
    check(back == "restored" && layout::close(layout::visible(a), start, 1), "restore         ", format!("{back}, the size and place before any placement: {} (was {})", show(layout::visible(a)), show(start)));
    let back = layout.apply(a, "restore", &plain);
    check(back == "nothing-to-restore", "restore         ", format!("{back}, with nothing left to go back to"));

    // A window with a minimum size is held against the edge its share touches.
    let c = window(w!("METEOR V1 window-layout probe C (minimum size)"), WS_OVERLAPPEDWINDOW, first, true);
    STUBBORN.store(c, SeqCst);
    pump(60);
    let result = layout.apply(c, "right-third", &plain);
    let (seen, win) = (layout::visible(c), outer(c));
    check(
        result == "done" && seen.right == work.right && seen.top == work.top && win.right - win.left == MIN_SIZE.0,
        "minimum size    ",
        format!("{result}, right third asked ({} wide), it kept {} and sits against the right edge: {}", (work.right - work.left) / 3, win.right - win.left, show(seen)),
    );
    let result = layout.apply(c, "left-third", &plain);
    let seen = layout::visible(c);
    check(result == "done" && seen.left == work.left, "minimum size    ", format!("{result}, left third asked: against the left edge: {}", show(seen)));
    close_own(c);

    // A window that cannot be resized is moved, never stretched.
    let d = window(w!("METEOR V1 window-layout probe D (fixed size)"), WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU, RECT { left: first.left, top: first.top, right: first.left + 420, bottom: first.top + 300 }, true);
    pump(60);
    let size = layout::visible(d);
    let result = layout.apply(d, "bottom-right", &plain);
    let seen = layout::visible(d);
    check(
        result == "done" && seen.right == work.right && seen.bottom == work.bottom && seen.right - seen.left == size.right - size.left && seen.bottom - seen.top == size.bottom - size.top,
        "fixed size      ",
        format!("{result}, bottom right asked: same size, in the corner: {}", show(seen)),
    );
    close_own(d);

    // Maximise, as the title bar's button does it; top half twice; the way back from it.
    layout.apply(a, "left-half", &plain);
    let result = layout.apply(a, "maximize", &plain);
    check(result == "done" && zoomed(a) && layout::close(layout::visible(a), work, 1), "maximize        ", format!("{result}, maximised: {} = the work area; window {}", show(layout::visible(a)), show(outer(a))));
    let back = layout.apply(a, "restore", &plain);
    check(back == "restored" && !zoomed(a) && layout::close(layout::visible(a), part(work, (0.0, 0.5), (0.0, 1.0)), 1), "restore         ", format!("{back}, from maximised back to the left half: {}", show(layout::visible(a))));
    layout.apply(a, "top-half", &plain);
    let result = layout.apply(a, "up", &plain);
    check(result == "done" && zoomed(a), "up, again       ", format!("{result}, up on the top half is maximised"));
    placed(&layout, a, "right-half", &plain, part(work, (0.5, 1.0), (0.0, 1.0)), "from maximised straight to the right half");

    // Two displays that are not there (this one at 125 % and one of 1920x1080 at
    // 100 % to its right): the arithmetic of a move between them, without a window.
    let rect = |l: i32, t: i32, r: i32, b: i32| RECT { left: l, top: t, right: r, bottom: b };
    let one = layout::Display { handle: 1, rect: rect(0, 0, 2240, 1400), work: rect(0, 0, 2240, 1340), dpi: 120 };
    let two = layout::Display { handle: 2, rect: rect(2240, 0, 4160, 1080), work: rect(2240, 0, 4160, 1040), dpi: 96 };
    let pair = [one, two];
    check(
        layout::adjacent(&pair, 0, true) == Some(1) && layout::adjacent(&pair, 1, true) == Some(0) && layout::adjacent(&pair[..1], 0, true).is_none(),
        "two displays    ",
        "next and previous go round both; with one there is no next".into(),
    );
    check(
        layout::neighbour(&pair, 0, layout::Dir::Right) == Some(1) && layout::neighbour(&pair, 0, layout::Dir::Left).is_none() && layout::neighbour(&pair, 1, layout::Dir::Left) == Some(0) && layout::neighbour(&pair, 0, layout::Dir::Down).is_none(),
        "two displays    ",
        "the one at the right is the right-hand neighbour, and nothing is below".into(),
    );
    let moved = layout::rect_for_display(rect(200, 100, 1200, 900), &one, &two);
    check(moved == rect(2400, 80, 3200, 720), "two displays    ", format!("1000x800 at 200,100 on 125 % lands as {} on 100 %: the same size and place to the eye", show(moved)));
    let moved = layout::rect_for_display(rect(1493, 0, 2240, 1340), &one, &two);
    check(moved.right == 4160 && moved.top == 0 && moved.bottom - moved.top == 1040, "two displays    ", format!("a right third stays against the right edge and shrinks to fit: {}", show(moved)));
    check(layout::rect_for_display(one.work, &one, &two) == two.work, "two displays    ", "a window that fills its display fills the next".into());

    // The next display, and back.
    let at = all.iter().position(|d| d.rect == here.rect).unwrap_or(0);
    match layout::adjacent(&all, at, true) {
        None => {
            let result = layout.apply(a, "next-display", &plain);
            check(result == "one-display", "next display    ", format!("{result}: this machine has one display, the move itself was not exercised"));
        }
        Some(next) => {
            let before = layout::visible(a);
            let want = layout::rect_for_display(before, &here, &all[next]);
            placed(&layout, a, "next-display", &plain, want, "the same place on the next display");
            layout.apply(a, "previous-display", &plain);
            check(layout::close(layout::visible(a), before, layout::TOLERANCE), "previous display", format!("back: {}", show(layout::visible(a))));
        }
    }

    // ── Focus follows the pointer ─────────────────────────────────────────────
    let b = window(w!("METEOR V1 window-layout probe B"), WS_OVERLAPPEDWINDOW, first, false);
    let (wa, wb) = (RECT { left: work.left + px(80), top: work.top + px(80), right: work.left + px(80) + 420, bottom: work.top + px(80) + 300 }, RECT { left: work.left + px(80) + 460, top: work.top + px(80), right: work.left + px(80) + 880, bottom: work.top + px(80) + 300 });
    unsafe {
        for (hwnd, r) in [(a, wa), (b, wb)] {
            let _ = SetWindowPos(HWND(hwnd as *mut _), None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOACTIVATE);
        }
    }
    // Both on top of the other windows, side by side: B first, so that A is the one in front.
    let up = svc_windows::activate(b) && svc_windows::activate(a);
    pump(150);
    if !up || front() != a {
        check(false, "focus follows   ", "the probe's windows could not be put in front: not exercised".into());
    } else {
        let own = [a, b];
        let following = follow::start(150, move |hwnd| own.contains(&hwnd));
        let hub = svc_input::status();
        println!("mouse hook installed: {} ({} subscriber)", hub.mouse, hub.mouse_subscribers);
        let (ax, ay) = middle(layout::visible(a));
        let (bx, by) = middle(layout::visible(b));
        point_at(bx, by);
        pump(500);
        check(front() == b, "focus follows   ", format!("the pointer rests on B: B is in front ({})", front() == b));
        point_at(ax, ay);
        pump(500);
        check(front() == a, "focus follows   ", format!("the pointer rests on A: A is in front ({})", front() == a));
        // Across B and back within the delay: B is merely crossed.
        ACTIVATED.lock().unwrap().clear();
        point_at(bx, by);
        pump(40);
        point_at(ax, ay);
        pump(500);
        let woken = ACTIVATED.lock().unwrap().clone();
        check(front() == a && !woken.contains(&b), "focus follows   ", format!("B crossed in 40 ms of a 150 ms rest: A stays in front, B was not woken ({})", !woken.contains(&b)));
        drop(following);
        pointer_back();
        pump(250);
        let hub = svc_input::status();
        check(!hub.mouse && hub.mouse_subscribers == 0, "focus follows   ", format!("switched off: mouse hook installed {}, {} subscribers; the pointer is back", hub.mouse, hub.mouse_subscribers));
    }
    close_own(b);

    // ── Close protection ──────────────────────────────────────────────────────
    let listener = |event: Event| {
        EVENTS.lock().unwrap().push(event);
        // A confirmed close goes to the probe's window, and only while it is the one in front.
        let hwnd = GUARDED.load(SeqCst);
        if matches!(event, Event::Confirmed(_)) && hwnd != 0 && front() == hwnd {
            svc_windows::close(hwnd);
        }
    };
    let guarded = |hold: bool, ms: u64| guard::start(guard::Config { keys: vec![(F24, Which::AltF4)], hold, ms, injected: true }, alt_held, listener);
    GUARDED.store(a, SeqCst);
    if !svc_windows::activate(a) {
        check(false, "close protection", "the probe's window could not be put in front: not exercised".into());
    } else {
        pump(100);
        // Without the guard the key arrives: the proof that what follows is the guard's doing.
        tap(F24, true);
        tap(F24, false);
        pump(150);
        check(KEYS.load(SeqCst) == 1, "close protection", format!("guard off: the stand-in key reaches the window ({} press)", KEYS.load(SeqCst)));

        let guard = guarded(true, 400);
        println!("keyboard hook installed: {}", svc_input::status().keyboard);
        tap(F24, true);
        pump(150);
        tap(F24, false);
        pump(450);
        let seen = events();
        check(
            alive(a) && KEYS.load(SeqCst) == 1 && seen == [Event::Asked(Which::AltF4), Event::Cancelled(Which::AltF4)],
            "close protection",
            format!("hold 400 ms, let go after 150: held back, window open ({}), key swallowed, events {seen:?}", alive(a)),
        );
        tap(F24, true);
        pump(650);
        tap(F24, false);
        pump(100);
        let seen = events();
        check(
            !alive(a) && KEYS.load(SeqCst) == 1 && seen == [Event::Asked(Which::AltF4), Event::Confirmed(Which::AltF4)],
            "close protection",
            format!("held 650 ms: confirmed, window closed ({}), events {seen:?}", !alive(a)),
        );
        drop(guard);

        // Pressed twice.
        let e = window(w!("METEOR V1 window-layout probe E"), WS_OVERLAPPEDWINDOW, wa, false);
        GUARDED.store(e, SeqCst);
        if !svc_windows::activate(e) {
            check(false, "close protection", "the second window could not be put in front: not exercised".into());
        } else {
            pump(100);
            let guard = guarded(false, 400);
            tap(F24, true);
            tap(F24, false);
            pump(600);
            let seen = events();
            check(alive(e) && seen == [Event::Asked(Which::AltF4)], "close protection", format!("twice within 400 ms, pressed once: window open ({}), events {seen:?}", alive(e)));
            tap(F24, true);
            tap(F24, false);
            pump(120);
            tap(F24, true);
            tap(F24, false);
            pump(250);
            let seen = events();
            check(
                !alive(e) && KEYS.load(SeqCst) == 1 && seen == [Event::Asked(Which::AltF4), Event::Confirmed(Which::AltF4)],
                "close protection",
                format!("600 ms later once (too late, asked again), then again after 120 ms: window closed ({}), events {seen:?}", !alive(e)),
            );
            drop(guard);
        }
        close_own(e);
    }
    GUARDED.store(0, SeqCst);
    close_own(a);
    pump(250);
    let hub = svc_input::status();
    check(
        !hub.keyboard && !hub.mouse && hub.keyboard_subscribers + hub.mouse_subscribers == 0,
        "hooks           ",
        format!("at the end: keyboard {}, mouse {}, subscribers {}", hub.keyboard, hub.mouse, hub.keyboard_subscribers + hub.mouse_subscribers),
    );

    let failed = FAILED.load(SeqCst);
    println!("{} checks, {failed} failed", CHECKS.load(SeqCst));
    std::process::exit(if failed == 0 { 0 } else { 1 });
}
