// cargo run -p notch-feat-dictation --example hold-dictate-probe
//
// Proves the hold shortcut on two windows of the probe's own, each with a text field (A also has a password
// field). Keys are sent only while one of them is in front; the microphone is never opened and nothing is
// played: the engine is a stand-in that answers with a made-up transcript. If the speech model is installed,
// the real engine code is also run on a tone the probe generates. A watchdog ends the process after 30 s.
use notch_feat_dictation::{hold, whistle};
use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering::SeqCst};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

static A: AtomicIsize = AtomicIsize::new(0);
static A_EDIT: AtomicIsize = AtomicIsize::new(0);
static A_PASS: AtomicIsize = AtomicIsize::new(0);
static B: AtomicIsize = AtomicIsize::new(0);
static B_EDIT: AtomicIsize = AtomicIsize::new(0);
static FINISHED: AtomicU32 = AtomicU32::new(0);
static CANCELLED: AtomicU32 = AtomicU32::new(0);

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, _wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        // WM_APP: focus the field whose handle is in lparam (SetFocus works from the window's own thread).
        WM_APP => {
            let _ = unsafe { SetFocus(Some(HWND(lparam.0 as *mut _))) };
            LRESULT(0)
        }
        WM_SETFOCUS => {
            let edit = if hwnd.0 as isize == A.load(SeqCst) { A_EDIT.load(SeqCst) } else { B_EDIT.load(SeqCst) };
            let _ = unsafe { SetFocus(Some(HWND(edit as *mut _))) };
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
        _ => unsafe { DefWindowProcW(hwnd, msg, _wparam, lparam) },
    }
}

fn text(edit: isize) -> String {
    let mut t = [0u16; 512];
    let n = unsafe { SendMessageW(HWND(edit as *mut _), WM_GETTEXT, Some(WPARAM(t.len())), Some(LPARAM(t.as_mut_ptr() as isize))) }.0;
    String::from_utf16_lossy(&t[..n as usize])
}

fn clear(edit: isize) {
    unsafe { SendMessageW(HWND(edit as *mut _), WM_SETTEXT, None, Some(LPARAM(w!("").as_ptr() as isize))) };
}

fn key(vk: u16, down: bool) {
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) } as u16,
                dwFlags: if down { KEYBD_EVENT_FLAGS(0) } else { KEYEVENTF_KEYUP },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
}

fn pause(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

fn front(w: &AtomicIsize, field: &AtomicIsize) -> bool {
    let hwnd = w.load(SeqCst);
    let ok = svc_inject::ForegroundGuard::of(hwnd).restore().is_ok();
    pause(150);
    unsafe { SendMessageW(HWND(hwnd as *mut _), WM_APP, None, Some(LPARAM(field.load(SeqCst)))) };
    pause(100);
    ok && unsafe { GetForegroundWindow() }.0 as isize == hwnd
}

struct Fake;
impl hold::Take for Fake {
    fn finish(self: Box<Self>) -> String {
        FINISHED.fetch_add(1, SeqCst);
        "made up transcript".into()
    }
    fn cancel(self: Box<Self>) {
        CANCELLED.fetch_add(1, SeqCst);
    }
}

fn report(ok: &mut bool, what: &str, pass: bool, detail: String) {
    println!("{} {what}: {detail}", if pass { "ok  " } else { "FAIL" });
    *ok &= pass;
}

fn exercise() -> bool {
    let mut ok = true;
    let states = Arc::new(Mutex::new(Vec::<String>::new()));
    let s2 = states.clone();
    let hooks = hold::Hooks {
        open: Box::new(|| Ok(Box::new(Fake))),
        emit: Arc::new(move |ev, v| s2.lock().unwrap().push(format!("{ev}:{}", v["state"].as_str().unwrap_or("")))),
    };
    println!("parse(\"Shift+Q\") = {:?}, parse(\"Ctrl+Nope\") = {:?}", hold::parse("Shift+Q"), hold::parse("Ctrl+Nope"));
    let hold = hold::Hold::new(hold::parse("Shift+Q").unwrap(), Arc::new(hooks));
    println!("input hooks: keyboard installed = {}", svc_input::status().keyboard);
    let (a, b) = (A_EDIT.load(SeqCst), B_EDIT.load(SeqCst));
    let held = |ms: u64| {
        key(0x10, true);
        pause(30);
        key(0x51, true);
        pause(ms);
    };
    let release = || {
        key(0x51, false);
        key(0x10, false);
        pause(250);
    };
    let reset = || {
        states.lock().unwrap().clear();
        FINISHED.store(0, SeqCst);
        CANCELLED.store(0, SeqCst);
    };

    if !front(&A, &A_EDIT) {
        println!("SKIP the probe's window could not be brought to the front; no key was sent");
        return false;
    }

    // 1. A long hold: the trigger key does not reach the field, the strip shows listening, release inserts.
    reset();
    held(700);
    let mid = states.lock().unwrap().clone();
    report(&mut ok, "held: key swallowed, listening shown", text(a).is_empty() && mid.contains(&"hold:listening".to_string()), format!("field {:?}, events {mid:?}", text(a)));
    release();
    pause(1200);
    report(&mut ok, "release inserts the transcript", text(a) == "made up transcript" && FINISHED.load(SeqCst) == 1, format!("field {:?}, events {:?}", text(a), states.lock().unwrap()));
    clear(a);

    // 2. A tap: nothing.
    reset();
    held(100);
    release();
    pause(400);
    report(
        &mut ok,
        "tap under 0.3 s ignored",
        text(a).is_empty() && FINISHED.load(SeqCst) == 0 && CANCELLED.load(SeqCst) == 1 && states.lock().unwrap().iter().all(|e| e != "hold:listening"),
        format!("field {:?}, events {:?}", text(a), states.lock().unwrap()),
    );

    // 3. Esc while holding cancels.
    reset();
    held(500);
    key(0x1B, true);
    key(0x1B, false);
    pause(100);
    release();
    pause(500);
    report(&mut ok, "Esc cancels", text(a).is_empty() && FINISHED.load(SeqCst) == 0 && CANCELLED.load(SeqCst) == 1, format!("field {:?}, finished {}, cancelled {}", text(a), FINISHED.load(SeqCst), CANCELLED.load(SeqCst)));

    // 4. The window in front changed between press and release.
    reset();
    held(500);
    let moved = front(&B, &B_EDIT);
    release();
    pause(600);
    let (ta, tb) = (text(a), text(b));
    report(&mut ok, "window changed: nothing inserted", moved && ta.is_empty() && tb.is_empty() && FINISHED.load(SeqCst) == 0, format!("moved {moved}, A {ta:?}, B {tb:?}, events {:?}", states.lock().unwrap()));

    // 5. A password field: nothing inserted.
    reset();
    let back = front(&A, &A_PASS);
    held(500);
    release();
    pause(600);
    let blocked = states.lock().unwrap().iter().any(|e| e == "hold:blocked");
    report(&mut ok, "password field refused", back && text(A_PASS.load(SeqCst)).is_empty() && FINISHED.load(SeqCst) == 0 && blocked, format!("password field {:?}, events {:?}", text(A_PASS.load(SeqCst)), states.lock().unwrap()));

    // 6. Everything else passes: a plain letter types into the field.
    front(&A, &A_EDIT);
    key(0x5A, true);
    key(0x5A, false);
    pause(150);
    report(&mut ok, "other keys pass", text(a) == "z", format!("field {:?}", text(a)));
    clear(a);

    drop(hold);
    pause(300);
    report(&mut ok, "hook removed after drop", !svc_input::status().keyboard, format!("keyboard installed = {}", svc_input::status().keyboard));

    // The engine itself, on a tone made here, only when the model is already installed.
    let dir = notch_core::win::local_dir().join("features").join("dictation");
    if whistle::ready(&dir) {
        let tone: Vec<f32> = (0..16000).map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / 16000.0).sin() * 0.2).collect();
        match whistle::transcribe(&dir, &tone) {
            Ok(t) => println!("ok   engine ran on a generated tone (telemetry variables set): transcript {:?}", t.text),
            Err(e) => println!("FAIL engine: {e}"),
        }
    } else {
        println!("SKIP engine: the speech model is not installed ({}); everything but the engine was proven", dir.display());
    }
    ok
}

fn main() {
    // Watchdog: the hooks go with the process.
    std::thread::spawn(|| {
        pause(30_000);
        println!("WATCHDOG: 30 s passed, ending");
        std::process::exit(2);
    });
    unsafe {
        let module = GetModuleHandleW(None).unwrap_or_default();
        let class = w!("meteor-hold-dictate-probe");
        RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: module.into(), lpszClassName: class, ..Default::default() });
        let make = |x: i32, title: windows::core::PCWSTR| {
            CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_TOPMOST, class, title, WS_POPUP | WS_CAPTION, x, 200, 420, 110, None, None, Some(module.into()), None).expect("window")
        };
        let field = |parent: HWND, y: i32, style: u32| {
            CreateWindowExW(Default::default(), w!("EDIT"), w!(""), WS_CHILD | WS_VISIBLE | WINDOW_STYLE(0x80 | style), 8, y, 390, 24, Some(parent), None, Some(module.into()), None).expect("field")
        };
        let (a, b) = (make(100, w!("hold-dictate-probe A")), make(560, w!("hold-dictate-probe B")));
        A.store(a.0 as isize, SeqCst);
        B.store(b.0 as isize, SeqCst);
        A_EDIT.store(field(a, 8, 0).0 as isize, SeqCst);
        A_PASS.store(field(a, 40, 0x20).0 as isize, SeqCst); // ES_PASSWORD
        B_EDIT.store(field(b, 8, 0).0 as isize, SeqCst);
        let _ = ShowWindow(a, SW_SHOW);
        let _ = ShowWindow(b, SW_SHOW);
        let worker = std::thread::spawn(move || {
            let ok = exercise();
            let _ = PostMessageW(Some(HWND(A.load(SeqCst) as *mut _)), WM_CLOSE, WPARAM(0), LPARAM(0));
            let _ = PostMessageW(Some(HWND(B.load(SeqCst) as *mut _)), WM_CLOSE, WPARAM(0), LPARAM(0));
            ok
        });
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let ok = worker.join().unwrap_or(false);
        println!("probe done. {}", if ok { "ALL OK" } else { "NOT ALL OK" });
    }
}
