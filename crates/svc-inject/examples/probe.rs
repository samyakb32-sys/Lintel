// cargo run -p svc-inject --example inject-probe
//
// Proves the service on a window of the probe's own: a small box with one text
// field. It is brought to the front, typed into, pasted into, read back and
// closed; then the window that was in front before comes back. Keys are only
// ever sent while the probe's own window is the one in front, and the
// clipboard is compared with what it held before.

use std::sync::atomic::{AtomicIsize, Ordering};
use std::time::Duration;

use svc_inject::{clip, ForegroundGuard};
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetForegroundWindow, GetMessageW, PostMessageW,
    PostQuitMessage, RegisterClassW, SendMessageW, ShowWindow, TranslateMessage, MSG, SW_SHOW, WINDOW_STYLE, WM_CLOSE,
    WM_DESTROY, WM_GETTEXT, WM_SETFOCUS, WNDCLASSW, WS_CAPTION, WS_CHILD, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
    WS_VISIBLE,
};

static EDIT: AtomicIsize = AtomicIsize::new(0);

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        // The keyboard goes to the field.
        WM_SETFOCUS => {
            let _ = unsafe { SetFocus(Some(HWND(EDIT.load(Ordering::SeqCst) as *mut _))) };
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = unsafe { DestroyWindow(hwnd) };
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn field_text() -> String {
    let mut text = [0u16; 512];
    let edit = HWND(EDIT.load(Ordering::SeqCst) as *mut _);
    let len = unsafe { SendMessageW(edit, WM_GETTEXT, Some(WPARAM(text.len())), Some(LPARAM(text.as_mut_ptr() as isize))) }.0;
    String::from_utf16_lossy(&text[..len as usize])
}

fn check(what: &str, expected: &str) -> bool {
    std::thread::sleep(Duration::from_millis(250));
    let got = field_text();
    let ok = got == expected;
    println!("{} {what}: field holds {:?}", if ok { "ok  " } else { "FAIL" }, got);
    ok
}

/// Runs beside the window's message loop.
fn exercise(own: isize) -> bool {
    let mine = || unsafe { GetForegroundWindow() }.0 as isize == own;
    let before = ForegroundGuard::capture();
    println!("window in front before: elevated={}, is the probe's own={}", before.elevated(), before.hwnd() == own);

    match ForegroundGuard::of(own).restore() {
        Ok(()) => println!("ok   ForegroundGuard::restore brought the probe's window to the front"),
        Err(err) => {
            println!("SKIP the probe's window could not be brought to the front ({err}); no key was sent");
            return false;
        }
    }
    std::thread::sleep(Duration::from_millis(150));
    println!("is_elevated(own window) = {}", svc_inject::is_elevated(own));
    println!("wait_modifiers_up() = {}", svc_inject::wait_modifiers_up());

    let mut all = true;
    let mut step = |name: &str, run: &dyn Fn() -> Result<(), String>, expected: &str| {
        if !mine() {
            println!("SKIP {name}: another window came to the front; no key was sent");
            all = false;
            return;
        }
        match run() {
            Ok(()) => all &= check(name, expected),
            Err(err) => {
                println!("FAIL {name}: {err}");
                all = false;
            }
        }
    };

    let typed = "inject-probe: h\u{e9}llo \u{2713} \u{65e5}\u{672c}";
    step("type_text", &|| svc_inject::type_text(typed), typed);
    // A plain EDIT field has no Ctrl+A: Home, then Shift+End, selects its line.
    let select = || svc_inject::send_keys("Home").and_then(|_| svc_inject::send_keys("Shift+End"));
    step("send_keys(\"Home\", \"Shift+End\") then type_text", &|| select().and_then(|_| svc_inject::type_text("replaced")), "replaced");

    // paste_text borrows the clipboard: what is there now must be there again afterwards.
    let held = clip::open().map(|c| c.snapshot()).unwrap_or_default();
    println!("clipboard before paste_text: {} formats, {} bytes (content not shown)", held.formats.len(), held.bytes());
    step("paste_text", &|| svc_inject::paste_text(" + pasted"), "replaced + pasted");
    let after = clip::open().map(|c| c.snapshot()).unwrap_or_default();
    let same = after == held;
    println!(
        "{} clipboard after paste_text: {} formats, {} bytes, identical to before: {same}; marked as the app's own write: {}",
        if same { "ok  " } else { "FAIL" },
        after.formats.len(),
        after.bytes(),
        clip::is_own()
    );
    all &= same;

    println!("send_keys(\"Ctrl+Nope\") = {:?}", svc_inject::send_keys("Ctrl+Nope"));

    // Back to where the user was.
    if before.hwnd() != 0 && before.hwnd() != own {
        println!("window in front before, restored: {:?}", before.restore());
    }
    all
}

fn main() {
    unsafe {
        let module = GetModuleHandleW(None).unwrap_or_default();
        let class = w!("meteor-inject-probe");
        RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: module.into(), lpszClassName: class, ..Default::default() });
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            class,
            w!("inject-probe (closes by itself)"),
            WS_POPUP | WS_CAPTION,
            200,
            200,
            420,
            90,
            None,
            None,
            Some(module.into()),
            None,
        )
        .expect("window");
        // ES_AUTOHSCROLL
        let edit = CreateWindowExW(
            Default::default(),
            w!("EDIT"),
            w!(""),
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(0x80),
            8,
            8,
            390,
            24,
            Some(hwnd),
            None,
            Some(module.into()),
            None,
        )
        .expect("field");
        EDIT.store(edit.0 as isize, Ordering::SeqCst);
        let _ = ShowWindow(hwnd, SW_SHOW);

        let own = hwnd.0 as isize;
        let worker = std::thread::spawn(move || {
            let ok = exercise(own);
            let _ = PostMessageW(Some(HWND(own as *mut _)), WM_CLOSE, WPARAM(0), LPARAM(0));
            ok
        });

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let ok = worker.join().unwrap_or(false);
        println!("probe window closed. {}", if ok { "ALL OK" } else { "NOT ALL OK" });
    }
}
