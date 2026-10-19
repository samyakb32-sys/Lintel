// Sysevents — a shared service (docs/ARCHITECTURE.md §8). Not a feature: the
// features that need it list this crate in their own Cargo.toml. Refcounted:
// nothing here runs without a live subscriber, and the handle `subscribe`
// returns unsubscribes on drop.
//
// One hidden window on its own thread receives what Windows only tells
// windows: power, session, display, device and clock changes, and which window
// is in front. Every `on_*` returns a `Sub`; the thread and its window exist
// from the first `Sub` to the drop of the last. Callbacks run on that thread:
// keep them short and never block in them.

use std::cell::RefCell;
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;

use windows::core::{w, GUID};
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::{
    GetSystemPowerStatus, RegisterPowerSettingNotification, UnregisterPowerSettingNotification, POWERBROADCAST_SETTING,
    SYSTEM_POWER_STATUS,
};
use windows::Win32::System::RemoteDesktop::{
    WTSRegisterSessionNotification, WTSUnRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION,
};
use windows::Win32::System::SystemServices::{
    GUID_ACDC_POWER_SOURCE, GUID_BATTERY_PERCENTAGE_REMAINING, GUID_CONSOLE_DISPLAY_STATE, GUID_LIDSWITCH_STATE_CHANGE,
    GUID_POWER_SAVING_STATUS,
};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClassNameW, GetForegroundWindow, GetMessageW, GetWindowLongW,
    GetWindowRect, GetWindowThreadProcessId, KillTimer, PostMessageW, PostQuitMessage, RegisterClassW, SetTimer,
    UnregisterClassW, DBT_DEVNODES_CHANGED, DEVICE_NOTIFY_WINDOW_HANDLE, EVENT_OBJECT_LOCATIONCHANGE,
    EVENT_SYSTEM_FOREGROUND, GWL_EXSTYLE, GWL_STYLE, MSG, OBJID_WINDOW, PBT_APMPOWERSTATUSCHANGE, PBT_APMRESUMEAUTOMATIC,
    PBT_APMSUSPEND, PBT_POWERSETTINGCHANGE, WINEVENT_OUTOFCONTEXT, WM_CLOSE, WM_DESTROY, WM_DEVICECHANGE,
    WM_DISPLAYCHANGE, WM_POWERBROADCAST, WM_TIMECHANGE, WM_TIMER, WM_WTSSESSION_CHANGE, WNDCLASSW, WS_DLGFRAME,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP, WS_THICKFRAME, WTS_SESSION_LOCK, WTS_SESSION_UNLOCK,
};

// ── What subscribers are told ─────────────────────────────────────────────────

/// `GetSystemPowerStatus`, read when asked and on every power event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PowerStatus {
    /// On the charger (or a PC without a battery).
    pub ac: bool,
    /// Charge, 0–100. `None` on a PC without a battery.
    pub percent: Option<u8>,
    pub charging: bool,
    /// Windows' energy saver is on.
    pub saver: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Session {
    Lock,
    Unlock,
    Sleep,
    Resume,
    DisplayOff,
    DisplayOn,
    LidClosed,
    LidOpen,
}

/// The window in front.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Foreground {
    pub hwnd: isize,
    pub pid: u32,
    /// File name of its process (`chrome.exe`); empty when it cannot be read.
    pub exe: String,
    /// It covers its whole monitor and has no frame: a game, a video, F11.
    pub fullscreen: bool,
}

#[derive(Debug, Clone)]
enum Event {
    Power(PowerStatus),
    Session(Session),
    DisplayChange,
    DeviceChange,
    TimeChange,
    Foreground(Foreground),
}

pub fn power_status() -> PowerStatus {
    let mut s = SYSTEM_POWER_STATUS::default();
    let _ = unsafe { GetSystemPowerStatus(&mut s) };
    // BatteryFlag: 8 charging, 128 no battery, 255 unknown. Percent 255 = unknown.
    let battery = s.BatteryFlag != 255 && s.BatteryFlag & 128 == 0 && s.BatteryLifePercent <= 100;
    PowerStatus {
        ac: s.ACLineStatus != 0,
        percent: battery.then_some(s.BatteryLifePercent),
        charging: battery && s.BatteryFlag & 8 != 0,
        saver: s.SystemStatusFlag == 1,
    }
}

/// The window in front right now (`on_foreground` only tells of changes).
pub fn foreground() -> Foreground {
    describe(unsafe { GetForegroundWindow() })
}

// ── Subscribing ───────────────────────────────────────────────────────────────

/// A live subscription. Dropping it unsubscribes; dropping the last one ends
/// the service's thread and window.
#[must_use = "dropping the handle unsubscribes"]
pub struct Sub(u64);

/// AC, charge, charging or energy saver changed (the first reading is not an event).
pub fn on_power(f: impl Fn(PowerStatus) + Send + Sync + 'static) -> Sub {
    subscribe(move |e| if let Event::Power(p) = e { f(*p) })
}

/// Lock and unlock, sleep and resume, the display going off and on, the lid.
pub fn on_session(f: impl Fn(Session) + Send + Sync + 'static) -> Sub {
    subscribe(move |e| if let Event::Session(s) = e { f(*s) })
}

/// A monitor was added or removed, or a resolution changed (one call per burst).
pub fn on_display_change(f: impl Fn() + Send + Sync + 'static) -> Sub {
    subscribe(move |e| if let Event::DisplayChange = e { f() })
}

/// A device arrived or left (one call per burst).
pub fn on_device_change(f: impl Fn() + Send + Sync + 'static) -> Sub {
    subscribe(move |e| if let Event::DeviceChange = e { f() })
}

/// The clock was set or the time zone changed.
pub fn on_time_change(f: impl Fn() + Send + Sync + 'static) -> Sub {
    subscribe(move |e| if let Event::TimeChange = e { f() })
}

/// Another window came to the front, or the one in front went full screen or left it.
pub fn on_foreground(f: impl Fn(&Foreground) + Send + Sync + 'static) -> Sub {
    subscribe(move |e| if let Event::Foreground(fg) = e { f(fg) })
}

type Handler = Arc<dyn Fn(&Event) + Send + Sync>;

struct Hub {
    next: u64,
    subs: Vec<(u64, Handler)>,
    /// The window (to close it) and its thread (to wait for it).
    thread: Option<(isize, JoinHandle<()>)>,
}

static HUB: Mutex<Hub> = Mutex::new(Hub { next: 0, subs: Vec::new(), thread: None });

fn subscribe(f: impl Fn(&Event) + Send + Sync + 'static) -> Sub {
    let mut hub = HUB.lock().unwrap();
    hub.next += 1;
    let id = hub.next;
    hub.subs.push((id, Arc::new(f)));
    if hub.thread.is_none() {
        let (tx, rx) = mpsc::channel();
        let handle = std::thread::spawn(move || run(tx));
        // The window exists (or failed: 0) before anyone can ask to close it.
        hub.thread = Some((rx.recv().unwrap_or(0), handle));
    }
    Sub(id)
}

impl Drop for Sub {
    fn drop(&mut self) {
        let ended = {
            let mut hub = HUB.lock().unwrap();
            hub.subs.retain(|s| s.0 != self.0);
            if hub.subs.is_empty() { hub.thread.take() } else { None }
        };
        let Some((hwnd, handle)) = ended else { return };
        unsafe {
            let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        // Dropped from inside a callback: the thread is this one and ends on its own.
        if handle.thread().id() != std::thread::current().id() {
            let _ = handle.join();
        }
    }
}

fn emit(event: Event) {
    // Copied out first: a callback may subscribe or drop a `Sub`.
    let subs: Vec<Handler> = HUB.lock().unwrap().subs.iter().map(|s| s.1.clone()).collect();
    for f in subs {
        f(&event);
    }
}

// ── The window thread ─────────────────────────────────────────────────────────

/// Bursts (a dock, a monitor waking) are told once, this long after the last message.
const SETTLE_MS: u32 = 300;
const TIMER_DISPLAY: usize = 1;
const TIMER_DEVICE: usize = 2;

const SETTINGS: [GUID; 5] = [
    GUID_ACDC_POWER_SOURCE,
    GUID_BATTERY_PERCENTAGE_REMAINING,
    GUID_POWER_SAVING_STATUS,
    GUID_CONSOLE_DISPLAY_STATE,
    GUID_LIDSWITCH_STATE_CHANGE,
];

struct Local {
    /// Last values told, so that the reading Windows sends on registration is not an event.
    power: PowerStatus,
    display_on: bool,
    lid_open: bool,
    front: Foreground,
    /// Follows the moves of the window in front (see `winevent`).
    moves: HWINEVENTHOOK,
}

thread_local! {
    static LOCAL: RefCell<Option<Local>> = const { RefCell::new(None) };
}

fn run(ready: mpsc::Sender<isize>) {
    unsafe {
        let module = GetModuleHandleW(None).unwrap_or_default();
        let class = w!("meteor-sysevents");
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: module.into(),
            lpszClassName: class,
            ..Default::default()
        });
        // A top-level window that is never shown, not a message-only one:
        // display, device and clock changes are broadcasts, and message-only
        // windows get none.
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class,
            w!(""),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(module.into()),
            None,
        ) else {
            let _ = ready.send(0);
            return;
        };
        LOCAL.set(Some(Local {
            power: power_status(),
            display_on: true,
            lid_open: true,
            front: foreground(),
            moves: HWINEVENTHOOK::default(),
        }));
        let power: Vec<_> = SETTINGS
            .iter()
            .filter_map(|guid| RegisterPowerSettingNotification(HANDLE(hwnd.0), guid, DEVICE_NOTIFY_WINDOW_HANDLE).ok())
            .collect();
        let _ = WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION);
        let front = SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(winevent),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        );
        follow(GetForegroundWindow());
        let _ = ready.send(hwnd.0 as isize);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            DispatchMessageW(&msg);
        }

        let _ = UnhookWinEvent(front);
        if let Some(local) = LOCAL.take() {
            let _ = UnhookWinEvent(local.moves);
        }
        let _ = WTSUnRegisterSessionNotification(hwnd);
        for handle in power {
            let _ = UnregisterPowerSettingNotification(handle);
        }
        let _ = UnregisterClassW(class, Some(module.into()));
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_POWERBROADCAST => {
            match wparam.0 as u32 {
                PBT_POWERSETTINGCHANGE => {
                    let setting = unsafe { &*(lparam.0 as *const POWERBROADCAST_SETTING) };
                    let value = if setting.DataLength >= 4 {
                        unsafe { (setting.Data.as_ptr() as *const u32).read_unaligned() }
                    } else {
                        0
                    };
                    power_setting(setting.PowerSetting, value);
                }
                PBT_APMPOWERSTATUSCHANGE => power_changed(),
                PBT_APMSUSPEND => emit(Event::Session(Session::Sleep)),
                PBT_APMRESUMEAUTOMATIC => emit(Event::Session(Session::Resume)),
                _ => {}
            }
            return LRESULT(1);
        }
        WM_WTSSESSION_CHANGE => match wparam.0 as u32 {
            WTS_SESSION_LOCK => emit(Event::Session(Session::Lock)),
            WTS_SESSION_UNLOCK => emit(Event::Session(Session::Unlock)),
            _ => {}
        },
        WM_DISPLAYCHANGE => unsafe {
            SetTimer(Some(hwnd), TIMER_DISPLAY, SETTLE_MS, None);
        },
        WM_DEVICECHANGE if wparam.0 as u32 == DBT_DEVNODES_CHANGED => unsafe {
            SetTimer(Some(hwnd), TIMER_DEVICE, SETTLE_MS, None);
        },
        WM_TIMER => {
            let _ = unsafe { KillTimer(Some(hwnd), wparam.0) };
            emit(if wparam.0 == TIMER_DISPLAY { Event::DisplayChange } else { Event::DeviceChange });
        }
        WM_TIMECHANGE => emit(Event::TimeChange),
        WM_DESTROY => unsafe { PostQuitMessage(0) },
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
    LRESULT(0)
}

/// Told only when something a subscriber can see moved.
fn power_changed() {
    let now = power_status();
    let changed = LOCAL.with_borrow_mut(|l| l.as_mut().is_some_and(|l| std::mem::replace(&mut l.power, now) != now));
    if changed {
        emit(Event::Power(now));
    }
}

fn power_setting(setting: GUID, value: u32) {
    if setting == GUID_CONSOLE_DISPLAY_STATE || setting == GUID_LIDSWITCH_STATE_CHANGE {
        // Display: 0 off, 1 on, 2 dimmed (not told). Lid: 0 closed, 1 open.
        if value > 1 {
            return;
        }
        let display = setting == GUID_CONSOLE_DISPLAY_STATE;
        let on = value == 1;
        let changed = LOCAL.with_borrow_mut(|l| {
            l.as_mut().is_some_and(|l| {
                let slot = if display { &mut l.display_on } else { &mut l.lid_open };
                std::mem::replace(slot, on) != on
            })
        });
        if changed {
            emit(Event::Session(match (display, on) {
                (true, true) => Session::DisplayOn,
                (true, false) => Session::DisplayOff,
                (false, true) => Session::LidOpen,
                (false, false) => Session::LidClosed,
            }));
        }
    } else {
        power_changed();
    }
}

// ── The window in front ───────────────────────────────────────────────────────

fn describe(hwnd: HWND) -> Foreground {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    Foreground {
        hwnd: hwnd.0 as isize,
        pid,
        exe: notch_core::win::exe_name_of_pid(pid).unwrap_or_default(),
        fullscreen: is_fullscreen(hwnd),
    }
}

/// The window covers its monitor and has no frame. A maximised window has one
/// (and with an auto-hiding taskbar it covers the monitor too); the desktop is
/// never a full-screen app.
fn is_fullscreen(hwnd: HWND) -> bool {
    if hwnd.is_invalid() {
        return false;
    }
    unsafe {
        let mut class = [0u16; 64];
        let len = GetClassNameW(hwnd, &mut class) as usize;
        if matches!(String::from_utf16_lossy(&class[..len]).as_str(), "Progman" | "WorkerW") {
            return false;
        }
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        if style & (WS_DLGFRAME.0 | WS_THICKFRAME.0) != 0 || ex_style & WS_EX_TOOLWINDOW.0 != 0 {
            return false;
        }
        let mut rect = RECT::default();
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if GetWindowRect(hwnd, &mut rect).is_err()
            || !GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut info).as_bool()
        {
            return false;
        }
        let m = info.rcMonitor;
        rect.left <= m.left && rect.top <= m.top && rect.right >= m.right && rect.bottom >= m.bottom
    }
}

/// Listens to the moves of `hwnd`'s thread only. A window that goes full screen
/// in place (F11, a video, a game once it has loaded) changes no foreground.
fn follow(hwnd: HWND) {
    let mut pid = 0u32;
    let thread = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    LOCAL.with_borrow_mut(|l| {
        let Some(l) = l.as_mut() else { return };
        unsafe {
            let _ = UnhookWinEvent(l.moves);
            l.moves = if thread == 0 {
                HWINEVENTHOOK::default()
            } else {
                SetWinEventHook(
                    EVENT_OBJECT_LOCATIONCHANGE,
                    EVENT_OBJECT_LOCATIONCHANGE,
                    None,
                    Some(winevent),
                    pid,
                    thread,
                    WINEVENT_OUTOFCONTEXT,
                )
            };
        }
    });
}

unsafe extern "system" fn winevent(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    object: i32,
    _child: i32,
    _thread: u32,
    _time: u32,
) {
    if object != OBJID_WINDOW.0 {
        return;
    }
    let front = if event == EVENT_SYSTEM_FOREGROUND {
        follow(hwnd);
        let front = describe(hwnd);
        LOCAL.with_borrow_mut(|l| l.as_mut().map(|l| l.front = front.clone()));
        Some(front)
    } else {
        // A move or a resize of the window in front: news only if it crossed full screen.
        LOCAL.with_borrow_mut(|l| {
            let l = l.as_mut()?;
            if hwnd.0 as isize != l.front.hwnd {
                return None;
            }
            let full = is_fullscreen(hwnd);
            (full != l.front.fullscreen).then(|| {
                l.front.fullscreen = full;
                l.front.clone()
            })
        })
    };
    if let Some(front) = front {
        emit(Event::Foreground(front));
    }
}
