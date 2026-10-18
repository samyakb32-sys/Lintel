// Companion — the mascot on the desktop. One small transparent, topmost window that
// never takes focus. Rust owns what the page cannot: the window's place and its flights,
// the drag, the hit region (clicks fall through everywhere but the character), and
// hiding for lock, sleep and full-screen apps. The drawing, the outfits and the
// reactions are the page's (src/features/companion, src/mascot/outfits.ts, reactions.ts).

mod pointer;

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering::Relaxed};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use notch_core::{Ctx, Feature, Group, Manifest, ShortcutDef, WindowSpec};
use serde_json::{json, Value};
use svc_sysevents::{Session, Sub};
use tauri::{Manager, PhysicalPosition, WebviewWindow};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{CombineRgn, CreateEllipticRgn, CreateRectRgn, DeleteObject, SetWindowRgn, HGDIOBJ, RGN_OR};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetWindowLongW, GetWindowRect, SendMessageW, SetWindowLongW, SetWindowPos, ShowWindow, GWL_STYLE,
    SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE, SW_SHOWNA, WM_NCCALCSIZE,
    WS_OVERLAPPEDWINDOW,
};

const ID: &str = "companion";
/// Logical size of the window; the page draws the character at (56, 68) in it.
const SIZE: f64 = 112.0;
const FLIGHT_MS: f64 = 340.0; // --d-slow

static MANIFEST: Manifest = Manifest {
    id: ID,
    name: "Companion",
    group: Group::Tools,
    default_on: true,
    uses: &[],
    shortcuts: &[ShortcutDef { id: "toggle", label: "Send the companion home or out", default: Some("Ctrl+Alt+B") }],
    // The way back once it was sent home: the tray menu, as well as the shortcut.
    tray: &[("toggle", "Send the companion home or out"), ("follow", "Follow the pointer on or off")],
    // outfit: auto | none | antenna | headphones | sprout | sunhat | woolly | leaf
    defaults: r#"{ "outfit": "auto", "docked": false, "x": null, "y": null, "reactions": true, "sleep": true, "sounds": true, "follow": true, "dwell": true }"#,
};

// What hides the window besides docking: bits of HIDE.
const FULLSCREEN: u8 = 1;
const AWAY: u8 = 2; // locked, asleep, display off
const NO_MASCOT: u8 = 4; // Settings, Mascot, "Show the mascot" is off
static HIDE: AtomicU8 = AtomicU8::new(0);
static DRAGGING: AtomicBool = AtomicBool::new(false);
static REDUCED: AtomicBool = AtomicBool::new(false);
/// Bumped to cancel a flight in the air.
static FLIGHT: AtomicU32 = AtomicU32::new(0);
/// Flights in the air; the pointer watcher leaves the window alone meanwhile.
static BUSY: AtomicU32 = AtomicU32::new(0);
/// Bumped to end the pointer watcher; each one runs for the number it was started with.
static WATCH: AtomicU32 = AtomicU32::new(0);
/// Settings, Companion: stay beside the pointer; read the page when the pointer rests on it.
static FOLLOW: AtomicBool = AtomicBool::new(true);
static DWELL: AtomicBool = AtomicBool::new(true);

/// Where the character's centre rests from the pointer, in logical px: down and to the right,
/// so that the pointer is never under it. (The window is placed so, the centre being at (56, 64) in it.)
const BESIDE: (f64, f64) = (40.0 - 56.0, 40.0 - 64.0);
/// The pointer is "resting" when it stays within this many px for this long.
const REST_PX: i32 = 8;
const REST_AFTER: Duration = Duration::from_millis(2500);
/// The most of a page's text that is read.
const PAGE_CHARS: usize = 3000;
/// Programs whose windows are never read: password managers and authenticators.
const NEVER_READ: &[&str] = &["keepass", "1password", "bitwarden", "lastpass", "dashlane", "enpass", "authenticator"];

#[derive(Default)]
pub struct Companion {
    subs: Mutex<Option<(Sub, Sub)>>,
}

/// The host's "Show the mascot" switch. debt: read from the settings file; Ctx has no host settings.
fn mascot_shown() -> bool {
    std::fs::read_to_string(notch_core::win::config_dir().join("settings.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v["showMascot"].as_bool())
        .unwrap_or(true)
}

fn window(cx: &Ctx) -> Option<WebviewWindow> {
    cx.app().get_webview_window(ID)
}

fn hwnd_of(w: &WebviewWindow) -> Option<isize> {
    w.hwnd().ok().map(|h| h.0 as isize)
}

fn hwnd(h: isize) -> HWND {
    HWND(h as *mut c_void)
}

fn set_bit(cx: &Ctx, bit: u8, on: bool) {
    if on {
        HIDE.fetch_or(bit, Relaxed);
    } else {
        HIDE.fetch_and(!bit, Relaxed);
    }
    refresh(cx);
}

/// Show or hide the window for lock, sleep and full-screen apps.
fn refresh(cx: &Ctx) {
    let hidden = HIDE.load(Relaxed) != 0;
    if let Some(h) = window(cx).as_ref().and_then(hwnd_of) {
        unsafe {
            let _ = ShowWindow(hwnd(h), if hidden { SW_HIDE } else { SW_SHOWNA });
        }
    }
    cx.emit(ID, "visible", json!({ "on": !hidden }));
}

// ── Place ─────────────────────────────────────────────────────────────────────

/// Work areas of the displays: left, top, right, bottom in physical pixels.
fn work_areas(cx: &Ctx) -> Vec<(i32, i32, i32, i32)> {
    cx.app()
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|m| {
            let a = m.work_area();
            (a.position.x, a.position.y, a.position.x + a.size.width as i32, a.position.y + a.size.height as i32)
        })
        .collect()
}

/// The window kept fully inside the work area of the display it is nearest to.
fn clamp(cx: &Ctx, x: i32, y: i32, size: i32) -> (i32, i32) {
    let (mx, my) = (x + size / 2, y + size / 2);
    let far = |m: &(i32, i32, i32, i32)| {
        let dx = (m.0 - mx).max(0).max(mx - m.2) as i64;
        let dy = (m.1 - my).max(0).max(my - m.3) as i64;
        dx * dx + dy * dy
    };
    match work_areas(cx).iter().min_by_key(|m| far(m)) {
        Some(m) => (x.clamp(m.0, (m.2 - size).max(m.0)), y.clamp(m.1, (m.3 - size).max(m.1))),
        None => (x, y),
    }
}

/// Where it rests: the saved spot kept on a display, else the bottom right of the main one.
fn spot(cx: &Ctx, size: i32) -> (i32, i32) {
    let s = cx.settings(ID);
    if let (Some(x), Some(y)) = (s["x"].as_i64(), s["y"].as_i64()) {
        return clamp(cx, x as i32, y as i32, size);
    }
    match cx.app().primary_monitor().ok().flatten() {
        Some(m) => {
            let a = m.work_area();
            let g = (24.0 * m.scale_factor()) as i32;
            (a.position.x + a.size.width as i32 - size - g, a.position.y + a.size.height as i32 - size - g)
        }
        None => (100, 100),
    }
}

/// Top centre of the island's display, where the character tucks in and comes out.
fn home_point(cx: &Ctx, size: i32) -> (i32, i32) {
    let m = cx
        .app()
        .get_webview_window("island")
        .and_then(|w| w.current_monitor().ok().flatten())
        .or_else(|| cx.app().primary_monitor().ok().flatten());
    match m {
        Some(m) => (m.position().x + m.size().width as i32 / 2 - size / 2, m.position().y - (size as f64 * 0.3) as i32),
        None => (0, 0),
    }
}

fn remember(cx: &Ctx, w: &WebviewWindow) {
    if let Ok(p) = w.outer_position() {
        cx.patch_settings(ID, json!({ "x": p.x, "y": p.y }));
    }
}

/// cubic-bezier(.2, .8, .2, 1): the design's --ease-out, solved for t by bisection.
fn ease_out(t: f64) -> f64 {
    let bez = |a: f64, b: f64, s: f64| 3.0 * (1.0 - s) * (1.0 - s) * s * a + 3.0 * (1.0 - s) * s * s * b + s * s * s;
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..20 {
        let mid = (lo + hi) / 2.0;
        if bez(0.2, 0.2, mid) < t {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    bez(0.8, 1.0, (lo + hi) / 2.0)
}

/// Moves the window from one point to the other in --d-slow; `then` runs on arrival
/// (unless something took the window meanwhile).
fn fly(h: isize, from: (i32, i32), to: (i32, i32), then: impl FnOnce() + Send + 'static) {
    /// Counts the flight out on every way out of the thread.
    struct InAir;
    impl Drop for InAir {
        fn drop(&mut self) {
            BUSY.fetch_sub(1, Relaxed);
        }
    }
    let gen = FLIGHT.fetch_add(1, Relaxed) + 1;
    BUSY.fetch_add(1, Relaxed);
    std::thread::spawn(move || {
        let _in_air = InAir;
        let at = |p: (i32, i32)| unsafe {
            let _ = SetWindowPos(hwnd(h), None, p.0, p.1, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        };
        if !REDUCED.load(Relaxed) {
            let t0 = Instant::now();
            loop {
                let t = (t0.elapsed().as_secs_f64() * 1000.0 / FLIGHT_MS).min(1.0);
                if FLIGHT.load(Relaxed) != gen {
                    return;
                }
                let k = ease_out(t);
                at((
                    (from.0 as f64 + (to.0 - from.0) as f64 * k) as i32,
                    (from.1 as f64 + (to.1 - from.1) as f64 * k) as i32,
                ));
                if t >= 1.0 {
                    break;
                }
                std::thread::sleep(Duration::from_millis(8));
            }
        }
        if FLIGHT.load(Relaxed) == gen {
            at(to);
            then();
        }
    });
}

fn size_px(w: &WebviewWindow) -> i32 {
    w.outer_size().map(|s| s.width as i32).unwrap_or((SIZE * 1.25) as i32)
}

/// Opens the window and, when asked, flies it out of the island to its spot.
fn out(cx: &Ctx, from_island: bool) -> Result<(), String> {
    cx.patch_settings(ID, json!({ "docked": false }));
    let w = cx.window().open(WindowSpec {
        label: ID.into(),
        feature: ID.into(),
        page: "main".into(),
        w: SIZE,
        h: SIZE,
        frameless: true,
        topmost: true,
        no_activate: true,
        transparent: true,
        ..Default::default()
    })?;
    let size = size_px(&w);
    let to = spot(cx, size);
    let h = hwnd_of(&w).ok_or("no window")?;
    if from_island {
        let from = home_point(cx, size);
        let _ = w.set_position(PhysicalPosition::new(from.0, from.1));
        let cx = cx.clone();
        fly(h, from, to, move || {
            if let Some(w) = window(&cx) {
                remember(&cx, &w);
            }
            cx.emit(ID, "landed", json!({}));
        });
    } else {
        let _ = w.set_position(PhysicalPosition::new(to.0, to.1));
    }
    refresh(cx);
    watch(cx);
    Ok(())
}

/// Flies back into the island and closes the window.
fn home(cx: &Ctx) {
    let Some(w) = window(cx) else { return };
    let Some(h) = hwnd_of(&w) else { return };
    WATCH.fetch_add(1, Relaxed);
    // Beside the pointer is not a place to come back to: the spot stays the one it was.
    if !FOLLOW.load(Relaxed) {
        remember(cx, &w);
    }
    cx.patch_settings(ID, json!({ "docked": true }));
    let size = size_px(&w);
    let from = w.outer_position().map(|p| (p.x, p.y)).unwrap_or((0, 0));
    let to = home_point(cx, size);
    cx.emit(ID, "homing", json!({}));
    let cx = cx.clone();
    fly(h, from, to, move || cx.window().close(ID));
}

// ── The pointer ───────────────────────────────────────────────────────────────

/// What the program under the pointer shows, as the page of the island hands it to the chat.
fn read_page(p: (i32, i32)) -> Option<Value> {
    let pid = pointer::pid_at(p.0, p.1).filter(|pid| *pid != std::process::id())?;
    let exe = notch_core::win::exe_name_of_pid(pid).unwrap_or_default();
    if NEVER_READ.iter().any(|name| exe.to_lowercase().contains(name)) {
        return None;
    }
    let page = pointer::read(p.0, p.1, PAGE_CHARS)?;
    if page.title.trim().is_empty() && page.text.trim().is_empty() {
        return None;
    }
    let stem = exe.strip_suffix(".exe").or_else(|| exe.strip_suffix(".EXE")).unwrap_or(&exe);
    let mut chars = stem.chars();
    let app: String = chars.next().map(|f| f.to_uppercase().chain(chars).collect()).unwrap_or_default();
    Some(json!({ "appName": app, "title": page.title, "text": page.text }))
}

/// Keeps the window beside the pointer and, when the pointer rests, reads the page under it and
/// tells the pages (`dwell`). Nothing is sent anywhere: the character holds it until it is clicked.
/// One thread while the window is out; it ends with the window, and there is none when both
/// options are off. Starting it ends the one before.
// debt: it polls (16 ms while the pointer moves, 150 ms while it rests) rather than hooking the mouse.
fn watch(cx: &Ctx) {
    let gen = WATCH.fetch_add(1, Relaxed) + 1;
    if !FOLLOW.load(Relaxed) && !DWELL.load(Relaxed) {
        return;
    }
    let cx = cx.clone();
    std::thread::spawn(move || {
        let mut shown: Option<(f64, f64)> = None;
        let mut last = Instant::now();
        let (mut cursor, mut target) = ((i32::MIN, i32::MIN), (0, 0));
        let mut moved_at = Instant::now();
        let (mut anchor, mut since, mut read) = ((i32::MIN, i32::MIN), Instant::now(), false);
        while WATCH.load(Relaxed) == gen {
            let Some(w) = window(&cx) else { break };
            let Some(h) = hwnd_of(&w) else { break };
            let now = Instant::now();
            let dt = now.duration_since(last).as_secs_f64();
            last = now;
            let paused = HIDE.load(Relaxed) != 0 || DRAGGING.load(Relaxed) || BUSY.load(Relaxed) > 0;
            let Some(p) = pointer::cursor().filter(|_| !paused) else {
                shown = None;
                std::thread::sleep(Duration::from_millis(200));
                continue;
            };
            if p != cursor {
                cursor = p;
                moved_at = now;
                let size = size_px(&w);
                let k = size as f64 / SIZE;
                target = clamp(&cx, p.0 + (BESIDE.0 * k) as i32, p.1 + (BESIDE.1 * k) as i32, size);
            }
            let mut moving = now.duration_since(moved_at) < Duration::from_millis(400);
            if FOLLOW.load(Relaxed) {
                let at = shown.unwrap_or_else(|| {
                    let mut r = RECT::default();
                    unsafe {
                        let _ = GetWindowRect(hwnd(h), &mut r);
                    }
                    (r.left as f64, r.top as f64)
                });
                let (tx, ty) = (target.0 as f64, target.1 as f64);
                let mut next = if REDUCED.load(Relaxed) {
                    (tx, ty)
                } else {
                    let a = 1.0 - (-dt / 0.08).exp();
                    (at.0 + (tx - at.0) * a, at.1 + (ty - at.1) * a)
                };
                if (tx - next.0).abs() < 0.6 && (ty - next.1).abs() < 0.6 {
                    next = (tx, ty);
                }
                if (next.0.round(), next.1.round()) != (at.0.round(), at.1.round()) {
                    unsafe {
                        let _ = SetWindowPos(
                            hwnd(h),
                            None,
                            next.0.round() as i32,
                            next.1.round() as i32,
                            0,
                            0,
                            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                        );
                    }
                    moving = true;
                }
                shown = Some(next);
            }
            if DWELL.load(Relaxed) {
                let down = unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) } as u16 & 0x8000 != 0;
                if down || (p.0 - anchor.0).abs() > REST_PX || (p.1 - anchor.1).abs() > REST_PX {
                    (anchor, since, read) = (p, now, false);
                } else if !read && now.duration_since(since) >= REST_AFTER {
                    read = true;
                    if let Some(page) = read_page(p) {
                        cx.emit(ID, "dwell", page);
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(if moving { 16 } else { 150 }));
        }
    });
}

/// Applies the two options: starts or ends the watcher, and sends the window back to its place
/// when it stops following.
fn options(cx: &Ctx, follow: bool, dwell: bool) {
    let was = FOLLOW.swap(follow, Relaxed);
    DWELL.store(dwell, Relaxed);
    let Some(w) = window(cx) else { return };
    watch(cx);
    if was && !follow {
        if let Some(h) = hwnd_of(&w) {
            let size = size_px(&w);
            let from = w.outer_position().map(|p| (p.x, p.y)).unwrap_or((0, 0));
            fly(h, from, spot(cx, size), || {});
        }
    }
}

// ── Hit region ────────────────────────────────────────────────────────────────

/// Takes the caption, the sizing frame and the title bar buttons out of the window's style.
/// A frameless window of the host still names them there (the frame is only given no room,
/// and the compositor, which draws it, has nothing to show). A window with a region is not
/// drawn by the compositor: Windows paints the frame itself, from the style, at the top of
/// the window, whenever the window is activated or its title is set. That was the piece of
/// a title bar over the character after the first click.
/// Proved by `cargo run -p notch-feat-companion --example companion-frame-probe`.
// debt: the host's window library writes its own style back when one of its flags changes;
// nothing changes them on this window, and `region` takes the frame out again each time.
/// Says whether there was a frame to take out.
#[doc(hidden)]
pub fn unframe(h: HWND) -> bool {
    unsafe {
        let style = GetWindowLongW(h, GWL_STYLE) as u32;
        if style & WS_OVERLAPPEDWINDOW.0 == 0 {
            return false;
        }
        SetWindowLongW(h, GWL_STYLE, (style & !WS_OVERLAPPEDWINDOW.0) as i32);
        // Windows also remembers that there was a caption, from the last time it measured the
        // frame itself, and goes on painting one from that. The host measures frameless windows
        // in its place, except when asked this way (no size is changing): Windows measures
        // again, finds no caption and forgets it. The rectangle it fills is not used.
        let mut r = RECT::default();
        let _ = GetWindowRect(h, &mut r);
        SendMessageW(h, WM_NCCALCSIZE, Some(WPARAM(0)), Some(LPARAM(&mut r as *mut RECT as isize)));
        let _ = SetWindowPos(h, None, 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
    }
    true
}

/// Only these ellipses (logical px, [cx, cy, rx, ry]) take the pointer; the rest of the window is not there.
fn region(w: &WebviewWindow, ellipses: &[[f64; 4]]) {
    let Some(h) = hwnd_of(w) else { return };
    let k = w.scale_factor().unwrap_or(1.0);
    if unframe(hwnd(h)) {
        // The page was last laid out in the room the frame left it (and a window with a caption is
        // never narrower than Windows' least width): sized again, the window is the square that was
        // asked for and the page fills it. Seen in the app: the character was cut off at the bottom.
        let _ = w.set_size(tauri::LogicalSize::new(SIZE, SIZE));
    }
    unsafe {
        let all = CreateRectRgn(0, 0, 0, 0);
        for e in ellipses {
            let r = CreateEllipticRgn(
                ((e[0] - e[2]) * k).floor() as i32,
                ((e[1] - e[3]) * k).floor() as i32,
                ((e[0] + e[2]) * k).ceil() as i32,
                ((e[1] + e[3]) * k).ceil() as i32,
            );
            let _ = CombineRgn(Some(all), Some(all), Some(r), RGN_OR);
            let _ = DeleteObject(HGDIOBJ(r.0));
        }
        // The system owns the region from here.
        SetWindowRgn(hwnd(h), Some(all), true);
    }
}

// ── Drag ──────────────────────────────────────────────────────────────────────

/// Follows the cursor while the left button is down; ponytail: an 8 ms loop that lives only for the drag.
fn drag(cx: &Ctx, w: &WebviewWindow) {
    let Some(h) = hwnd_of(w) else { return };
    if DRAGGING.swap(true, Relaxed) {
        return;
    }
    FLIGHT.fetch_add(1, Relaxed);
    let cx = cx.clone();
    std::thread::spawn(move || unsafe {
        let (mut p, mut r) = (POINT::default(), RECT::default());
        let _ = GetCursorPos(&mut p);
        let _ = GetWindowRect(hwnd(h), &mut r);
        let (dx, dy) = (r.left - p.x, r.top - p.y);
        while GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16 & 0x8000 != 0 {
            let _ = GetCursorPos(&mut p);
            let _ = SetWindowPos(hwnd(h), None, p.x + dx, p.y + dy, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
            std::thread::sleep(Duration::from_millis(8));
        }
        let _ = GetWindowRect(hwnd(h), &mut r);
        let (x, y) = clamp(&cx, r.left, r.top, r.right - r.left);
        if let Some(w) = window(&cx) {
            let _ = w.set_position(PhysicalPosition::new(x, y));
            cx.patch_settings(ID, json!({ "x": x, "y": y }));
        }
        DRAGGING.store(false, Relaxed);
        cx.emit(ID, "dragend", ());
    });
}

// ── Feature ───────────────────────────────────────────────────────────────────

impl Feature for Companion {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn start(&self, cx: &Ctx) -> Result<(), String> {
        let s = cx.settings(ID);
        FOLLOW.store(s["follow"].as_bool().unwrap_or(true), Relaxed);
        DWELL.store(s["dwell"].as_bool().unwrap_or(true), Relaxed);
        HIDE.store(if mascot_shown() { 0 } else { NO_MASCOT }, Relaxed);
        let session = {
            let cx = cx.clone();
            svc_sysevents::on_session(move |s| match s {
                Session::Lock | Session::Sleep | Session::DisplayOff => set_bit(&cx, AWAY, true),
                Session::Unlock | Session::Resume | Session::DisplayOn => set_bit(&cx, AWAY, false),
                _ => {}
            })
        };
        let front = {
            let cx = cx.clone();
            svc_sysevents::on_foreground(move |f| set_bit(&cx, FULLSCREEN, f.fullscreen))
        };
        *self.subs.lock().unwrap() = Some((session, front));
        if !cx.settings(ID)["docked"].as_bool().unwrap_or(false) {
            out(cx, true)?;
        }
        Ok(())
    }

    fn stop(&self, cx: &Ctx) {
        self.subs.lock().unwrap().take();
        WATCH.fetch_add(1, Relaxed);
        FLIGHT.fetch_add(1, Relaxed);
        cx.window().close(ID);
    }

    fn call(&self, cx: &Ctx, method: &str, args: Value) -> Result<Value, String> {
        match method {
            "ready" => Ok(json!({ "hidden": HIDE.load(Relaxed) != 0 })),
            "motion" => {
                REDUCED.store(args["reduced"].as_bool().unwrap_or(false), Relaxed);
                Ok(Value::Null)
            }
            "region" => {
                let e: Vec<[f64; 4]> = serde_json::from_value(args["ellipses"].clone()).map_err(|e| e.to_string())?;
                if let Some(w) = window(cx) {
                    region(&w, &e);
                }
                Ok(Value::Null)
            }
            "drag" => {
                if let Some(w) = window(cx) {
                    drag(cx, &w);
                }
                Ok(Value::Null)
            }
            // The island page tells of a moment (download done, agent finished, ...); the window plays it.
            "react" => {
                cx.emit(ID, "react", json!({ "kind": args["kind"], "preview": args["preview"] }));
                Ok(Value::Null)
            }
            // The window page heard the host's settings change.
            "mascot" => {
                set_bit(cx, NO_MASCOT, !args["shown"].as_bool().unwrap_or(true));
                Ok(Value::Null)
            }
            "home" => {
                home(cx);
                Ok(Value::Null)
            }
            // The window page heard the options (at its start and whenever they change).
            "options" => {
                options(cx, args["follow"].as_bool().unwrap_or(true), args["dwell"].as_bool().unwrap_or(true));
                Ok(Value::Null)
            }
            // The user clicked the character while it held a page: the island's chat takes it.
            "ask" => {
                cx.emit(ID, "ask", args);
                Ok(Value::Null)
            }
            "tray" if args["id"] == "follow" => {
                let follow = !FOLLOW.load(Relaxed);
                cx.patch_settings(ID, json!({ "follow": follow }));
                options(cx, follow, DWELL.load(Relaxed));
                Ok(Value::Null)
            }
            "out" => out(cx, true).map(|_| Value::Null),
            "toggle" | "shortcut" | "tray" => {
                if window(cx).is_some() {
                    home(cx);
                    Ok(Value::Null)
                } else {
                    out(cx, true).map(|_| Value::Null)
                }
            }
            _ => Err("unknown method".into()),
        }
    }

    fn strings(&self) -> &'static str {
        include_str!("../../../src/features/companion/strings.json")
    }
}

pub fn feature() -> std::sync::Arc<dyn Feature> {
    std::sync::Arc::new(Companion::default())
}
