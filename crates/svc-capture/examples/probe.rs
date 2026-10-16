// capture-probe: proves the Windows-facing code of svc-capture on this machine.
//
//   cargo run -p svc-capture --example capture-probe
//   cargo run -p svc-capture --example capture-probe -- --clipboard   (also copies a test picture, then puts the clipboard back)
//
// It grabs the whole screen and a region and checks their pixel sizes, then
// puts a window of its own on screen for a few seconds (a colour patch, a QR
// code, three lines of text) and reads all three back from grabs of it, shows
// that a window excluded from capture is not photographed, and takes the
// keyboard with a second window and hands it back.
//
// What it grabs of the screen is never shown: only sizes, timings and the
// pixels of its own window are printed. The one file it writes goes to
// target/capture-probe/ and is deleted before it exits.

use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use svc_capture::{Image, Rect};
use svc_inject::ForegroundGuard;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, CreateSolidBrush, DeleteDC, DeleteObject, EndPaint,
    EnumDisplaySettingsW, FillRect, GetDC, GetPixel, ReleaseDC, SelectObject, SetBkMode, SetDIBitsToDevice, SetTextColor,
    TextOutW, UpdateWindow, BITMAPINFO, DEVMODEW, DIB_RGB_COLORS, ENUM_CURRENT_SETTINGS, HDC, PAINTSTRUCT, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VK_ESCAPE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetForegroundWindow, PeekMessageW, RegisterClassW,
    ShowWindow, TranslateMessage, MSG, PM_REMOVE, SW_SHOWNOACTIVATE, WM_ERASEBKGND, WM_KEYDOWN, WM_PAINT, WNDCLASSW,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

const LINK: &str = "https://example.com/capture-probe?n=12345";
const BIG: &str = "Hello METEOR V1 12345";
const SMALL: &str = "Small text at fifteen pixels";
const TINY: &str = "Tiny text at twelve pixels 6789";
/// The accent of the design language, as a known colour to find again.
const PATCH: [u8; 3] = [0x8E, 0xA2, 0xFF];
/// A pixel inside the patch, in the window.
const AT: (i32, i32) = (70, 50);
const SIZE: (i32, i32) = (640, 420);

static CODE: OnceLock<(usize, Vec<bool>)> = OnceLock::new();
static ESC: AtomicBool = AtomicBool::new(false);
static FAILED: AtomicU32 = AtomicU32::new(0);

fn check(ok: bool, what: &str) -> bool {
    println!("{} {what}", if ok { "ok  " } else { "FAIL" });
    if !ok {
        FAILED.fetch_add(1, Ordering::Relaxed);
    }
    ok
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2])
}

fn rgb(c: [u8; 3]) -> COLORREF {
    COLORREF(c[0] as u32 | (c[1] as u32) << 8 | (c[2] as u32) << 16)
}

unsafe fn fill(dc: HDC, x: i32, y: i32, w: i32, h: i32, colour: [u8; 3]) {
    unsafe {
        let brush = CreateSolidBrush(rgb(colour));
        FillRect(dc, &RECT { left: x, top: y, right: x + w, bottom: y + h }, brush);
        let _ = DeleteObject(brush.into());
    }
}

unsafe fn text(dc: HDC, x: i32, y: i32, px: i32, what: &str) {
    unsafe {
        let font = CreateFontW(-px, 0, 0, 0, 400, 0, 0, 0, Default::default(), Default::default(), Default::default(), Default::default(), 0, w!("Segoe UI"));
        let before = SelectObject(dc, font.into());
        let wide: Vec<u16> = what.encode_utf16().collect();
        let _ = TextOutW(dc, x, y, &wide);
        SelectObject(dc, before);
        let _ = DeleteObject(font.into());
    }
}

/// The content window: white, the patch, the code, the three lines.
unsafe extern "system" fn content(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                let mut paint = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut paint);
                fill(dc, 0, 0, SIZE.0, SIZE.1, [255, 255, 255]);
                fill(dc, 20, 20, 100, 60, PATCH);
                if let Some((side, cells)) = CODE.get() {
                    for (i, dark) in cells.iter().enumerate() {
                        if *dark {
                            fill(dc, 30 + (i % side) as i32 * 6, 120 + (i / side) as i32 * 6, 6, 6, [0, 0, 0]);
                        }
                    }
                }
                SetBkMode(dc, TRANSPARENT);
                SetTextColor(dc, rgb([0, 0, 0]));
                text(dc, 290, 30, 28, BIG);
                text(dc, 290, 90, 15, SMALL);
                text(dc, 290, 130, 12, TINY);
                let _ = EndPaint(hwnd, &paint);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// The stand-in for the selector overlay: a window that takes the keyboard and hears Esc.
unsafe extern "system" fn keys(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_KEYDOWN && wparam.0 == VK_ESCAPE.0 as usize {
        ESC.store(true, Ordering::SeqCst);
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn pump(ms: u64) {
    let until = Instant::now() + Duration::from_millis(ms);
    let mut msg = MSG::default();
    while Instant::now() < until {
        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn key(up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VK_ESCAPE, dwFlags: if up { KEYEVENTF_KEYUP } else { Default::default() }, ..Default::default() } },
    }
}

/// `dib()` drawn by GDI itself, as a program pasting it would: the pixel that comes out at (x, y).
unsafe fn drawn(image: &Image, x: i32, y: i32) -> [u8; 3] {
    unsafe {
        let dib = svc_capture::dib(image);
        let screen = GetDC(None);
        let memory = CreateCompatibleDC(Some(screen));
        let bitmap = CreateCompatibleBitmap(screen, image.w as i32, image.h as i32);
        let before = SelectObject(memory, bitmap.into());
        SetDIBitsToDevice(
            memory, 0, 0, image.w, image.h, 0, 0, 0, image.h,
            dib[40..].as_ptr() as *const c_void, dib.as_ptr() as *const BITMAPINFO, DIB_RGB_COLORS,
        );
        let c = GetPixel(memory, x, y).0;
        SelectObject(memory, before);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(memory);
        ReleaseDC(None, screen);
        [c as u8, (c >> 8) as u8, (c >> 16) as u8]
    }
}

fn main() {
    let clipboard = std::env::args().any(|a| a == "--clipboard");
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/capture-probe");
    std::fs::create_dir_all(&dir).expect("target/capture-probe");

    // ── Displays ──────────────────────────────────────────────────────────────
    let monitors = svc_capture::monitors();
    check(!monitors.is_empty() && monitors[0].primary, &format!("monitors: {} found, the primary one first", monitors.len()));
    for m in &monitors {
        let mut mode = DEVMODEW { dmSize: size_of::<DEVMODEW>() as u16, ..Default::default() };
        let name: Vec<u16> = m.id.encode_utf16().chain([0]).collect();
        let known = unsafe { EnumDisplaySettingsW(PCWSTR(name.as_ptr()), ENUM_CURRENT_SETTINGS, &mut mode) }.as_bool();
        check(
            known && (mode.dmPelsWidth as i32, mode.dmPelsHeight as i32) == (m.rect.w, m.rect.h),
            &format!(
                "monitor {}: {}x{} at {},{} scale {} = its display mode {}x{} (physical pixels)",
                m.id, m.rect.w, m.rect.h, m.rect.x, m.rect.y, m.scale, mode.dmPelsWidth, mode.dmPelsHeight
            ),
        );
    }
    let Some(screen) = monitors.first().map(|m| m.rect) else { std::process::exit(1) };

    // ── The whole screen: size, PNG there and back ────────────────────────────
    let t = Instant::now();
    let full = svc_capture::grab(screen);
    let took = t.elapsed();
    match &full {
        Ok(full) => {
            check((full.w as i32, full.h as i32) == (screen.w, screen.h), &format!("full screen grab: {}x{} in {took:.0?}", full.w, full.h));
            let t = Instant::now();
            let png = svc_capture::encode_png(full).unwrap_or_default();
            let took = t.elapsed();
            let file = dir.join("full.png");
            std::fs::write(&file, &png).expect("write the grab");
            let back = std::fs::read(&file).ok().and_then(|bytes| svc_capture::decode_png(&bytes).ok());
            // Gone at once: nothing the probe does later can leave it behind.
            let _ = std::fs::remove_file(&file);
            check(back.as_ref() == Some(full), &format!("PNG: {} bytes in {took:.0?}, written, read back and decoded to the same pixels", png.len()));
            let t = Instant::now();
            let frame = svc_capture::dib(full);
            println!("     the same as the bitmap the clipboard gets: {} bytes in {:.0?}", frame.len(), t.elapsed());
        }
        Err(err) => {
            check(false, &format!("full screen grab: {err}"));
        }
    }
    drop(full);

    // ── A window of the probe's own ───────────────────────────────────────────
    let code = qrcode::QrCode::new(LINK.as_bytes()).expect("a code");
    let _ = CODE.set((code.width(), code.to_colors().into_iter().map(|c| c == qrcode::Color::Dark).collect()));
    let place = Rect { x: screen.x + 120, y: screen.y + 120, w: SIZE.0, h: SIZE.1 };
    let (window, overlay) = unsafe {
        let module = GetModuleHandleW(None).expect("module");
        for (name, proc) in [(w!("capture-probe-content"), content as unsafe extern "system" fn(_, _, _, _) -> _), (w!("capture-probe-keys"), keys)] {
            RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(proc), hInstance: module.into(), lpszClassName: name, ..Default::default() });
        }
        let window = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE, w!("capture-probe-content"), w!("capture-probe"), WS_POPUP,
            place.x, place.y, place.w, place.h, None, None, Some(module.into()), None,
        )
        .expect("the content window");
        // Not shown yet: the stand-in for the selector overlay.
        let overlay = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW, w!("capture-probe-keys"), w!("capture-probe"), WS_POPUP,
            place.x, place.y + place.h + 20, 240, 60, None, None, Some(module.into()), None,
        )
        .expect("the overlay window");
        let _ = ShowWindow(window, SW_SHOWNOACTIVATE);
        let _ = UpdateWindow(window);
        (window, overlay)
    };
    pump(500);

    // ── A region: exactly the pixels asked for, and the colour under a known one ─
    let region = svc_capture::grab(place);
    if let Ok(region) = &region {
        check((region.w as i32, region.h as i32) == SIZE, &format!("region grab {}x{} asked, {}x{} got", SIZE.0, SIZE.1, region.w, region.h));
        let c = region.pixel(AT.0 as u32, AT.1 as u32).unwrap_or_default();
        check(c == PATCH, &format!("colour under pixel {},{} of the window, from the region grab: {} (drawn {})", AT.0, AT.1, hex(c), hex(PATCH)));
    } else {
        check(false, "region grab");
    }
    let odd = Rect { x: place.x + 13, y: place.y + 7, w: 333, h: 211 };
    let piece = svc_capture::grab(odd).ok();
    let c = piece.as_ref().and_then(|p| p.pixel((AT.0 - 13) as u32, (AT.1 - 7) as u32)).unwrap_or_default();
    check(
        piece.as_ref().map(|p| (p.w, p.h)) == Some((333, 211)) && c == PATCH,
        &format!("region grab 333x211 at an odd offset: {:?} got, the same pixel is {}", piece.as_ref().map(|p| (p.w, p.h)), hex(c)),
    );
    let cut = region.as_ref().ok().map(|r| r.crop(Rect { x: 13, y: 7, w: 333, h: 211 }));
    check(cut.is_some() && cut == piece, "crop of the frozen picture = a grab of the same rectangle, pixel for pixel");

    // ── The window from its own buffer ────────────────────────────────────────
    let shot = svc_capture::grab_window(window.0 as isize);
    if let Ok(shot) = &shot {
        let c = shot.pixel(AT.0 as u32, AT.1 as u32).unwrap_or_default();
        check((shot.w as i32, shot.h as i32) == SIZE && c == PATCH, &format!("window grab: {}x{}, pixel {},{} is {}", shot.w, shot.h, AT.0, AT.1, hex(c)));
    } else {
        check(false, &format!("window grab: {:?}", shot.as_ref().err()));
    }
    println!("     windows a click could mean right now: {}", svc_capture::windows().len());

    // ── The code and the text the probe drew ──────────────────────────────────
    if let Ok(region) = &region {
        let t = Instant::now();
        let codes = svc_capture::qr(region);
        check(codes == [LINK], &format!("QR code decoded in {:.0?}: {codes:?}", t.elapsed()));

        if svc_capture::ocr_available() {
            let t = Instant::now();
            match svc_capture::ocr(region) {
                Ok(lines) => {
                    let took = t.elapsed();
                    let read: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
                    let has = |what: &str| read.iter().any(|line| line.contains(what));
                    check(has(BIG), &format!("OCR read the 28 px line in {took:.0?}: {read:?}"));
                    check(has(SMALL), "OCR read the 15 px line");
                    check(has(TINY), "OCR read the 12 px line");
                    let at = lines.iter().find(|l| l.text.contains(BIG)).map(|l| l.rect).unwrap_or_default();
                    check((280..300).contains(&at.x) && (25..45).contains(&at.y), &format!("the line's box is where it was drawn (290,30): {},{} {}x{}", at.x, at.y, at.w, at.h));
                }
                Err(err) => {
                    check(false, &format!("OCR: {err}"));
                }
            }
        } else {
            println!("     OCR: Windows has no text recognition for the user's languages here (the tool is hidden)");
        }
    }

    // ── The bitmap the clipboard gets ─────────────────────────────────────────
    let tiny = Image { w: 3, h: 2, rgba: [[255u8, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [10, 20, 30, 255], [40, 50, 60, 255], [70, 80, 90, 255]].concat() };
    let (first, last) = unsafe { (drawn(&tiny, 0, 0), drawn(&tiny, 2, 1)) };
    check(first == [255, 0, 0] && last == [70, 80, 90], &format!("clipboard bitmap drawn by GDI: top left {}, bottom right {}", hex(first), hex(last)));
    if clipboard {
        let kept = svc_inject::clip::open().map(|c| c.snapshot());
        let png = svc_capture::encode_png(&tiny).unwrap_or_default();
        let copied = svc_capture::clipboard_image(&tiny, &png);
        let there = svc_inject::clip::has(svc_inject::clip::CF_DIB) && svc_inject::clip::has(svc_inject::clip::format("PNG"));
        if let (Ok(open), Ok(kept)) = (svc_inject::clip::open(), &kept) {
            open.restore(kept);
        }
        check(copied.is_ok() && there, &format!("clipboard: a bitmap and a PNG were on it ({copied:?}); what was there before is back"));
    }

    // ── Excluded from capture ─────────────────────────────────────────────────
    let seen = |at: (i32, i32)| svc_capture::grab(Rect { x: place.x + at.0, y: place.y + at.1, w: 1, h: 1 }).ok().and_then(|p| p.pixel(0, 0));
    let set = svc_capture::exclude_from_capture(window.0 as isize, true);
    // As long as the capture feature waits before it photographs the screen without the island.
    pump(80);
    let hidden = seen(AT);
    check(
        set && svc_capture::excluded_from_capture(window.0 as isize) && hidden.is_some() && hidden != Some(PATCH),
        "a window excluded from capture is not in a grab (the pixel is no longer the patch)",
    );
    svc_capture::exclude_from_capture(window.0 as isize, false);
    pump(300);
    check(seen(AT) == Some(PATCH), "and it is back in the grab once included again");

    // ── The keyboard: taken by the overlay, Esc heard, handed back ────────────
    let front = ForegroundGuard::capture();
    unsafe {
        let _ = ShowWindow(overlay, SW_SHOWNOACTIVATE);
    }
    let taken = ForegroundGuard::of(overlay.0 as isize).restore();
    let ours = unsafe { GetForegroundWindow() } == overlay;
    check(taken.is_ok() && ours, &format!("the overlay took the keyboard (SetForegroundWindow checked: {taken:?})"));
    if ours {
        // A real key press, sent only while the probe's own window is the one in front.
        unsafe { SendInput(&[key(false), key(true)], size_of::<INPUT>() as i32) };
        pump(300);
        check(ESC.load(Ordering::SeqCst), "Esc, sent as a real key, reached the overlay");
    }
    let back = front.restore();
    let returned = unsafe { GetForegroundWindow() }.0 as isize == front.hwnd();
    check(
        (back.is_ok() && returned) || front.hwnd() == 0,
        &format!("the keyboard went back to the window that had it ({back:?})"),
    );

    unsafe {
        let _ = DestroyWindow(overlay);
        let _ = DestroyWindow(window);
    }
    let _ = std::fs::remove_dir_all(&dir);
    check(!dir.exists(), "target/capture-probe/ is gone again");
    let failed = FAILED.load(Ordering::Relaxed);
    println!("{}", if failed == 0 { "capture-probe: all good".to_string() } else { format!("capture-probe: {failed} failed") });
    std::process::exit(if failed == 0 { 0 } else { 1 });
}
