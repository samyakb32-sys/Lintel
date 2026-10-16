// cargo run -p notch-feat-command-bar --example command-bar-probe
//
// Proves the bar's Windows-facing code without the app:
//
//   1. the ranking, over a made-up catalogue;
//   2. the calculator, the conversions and the date arithmetic;
//   3. the list of open windows and the scan of installed apps;
//   4. the keyboard taken and handed back between windows of the probe's own
//      (the "app in front" is a second process, so that Windows' rules about
//      who may come to the front apply as they do in the app), with real keys;
//   5. the menu of a window read and one of its commands sent;
//   6. the on-screen keyboard test, the file walk, links, and the whole list
//      for a few queries against this PC's real apps and windows.
//
// It puts three small windows of its own on the screen for a few seconds and
// types into them. It never starts, focuses, moves or closes any other window,
// touches no clipboard, plays no sound, and runs none of the commands that
// change the PC (lock, sleep, shut down, empty the bin): those are listed only.
// Titles of windows that are not the probe's are never printed.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{mpsc, Mutex};
use std::time::Duration;

use notch_feat_command_bar::dates::{self, Day};
use notch_feat_command_bar::math::{self, Separators};
use notch_feat_command_bar::rows::{self, Catalog, Query, Row};
use notch_feat_command_bar::search::{rank, Candidate, Learned};
use notch_feat_command_bar::units;
use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateMenu, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, PostMessageW,
    PostQuitMessage, RegisterClassW, SetForegroundWindow, TranslateMessage, HMENU, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING, MSG,
    WINDOW_EX_STYLE, WM_APP, WM_CHAR, WM_CLOSE, WM_COMMAND, WM_DESTROY, WNDCLASSW, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

// ── Reporting ─────────────────────────────────────────────────────────────────

static FAILED: AtomicU32 = AtomicU32::new(0);

fn check(name: &str, ok: bool, detail: impl AsRef<str>) {
    if !ok {
        FAILED.fetch_add(1, Ordering::SeqCst);
    }
    println!("{}  {name}: {}", if ok { "ok  " } else { "FAIL" }, detail.as_ref());
}

// ── Windows of the probe's own ────────────────────────────────────────────────

/// What was typed into this process's windows, and the menu commands they were sent.
static TYPED: Mutex<Vec<u16>> = Mutex::new(Vec::new());
static COMMANDS: Mutex<Vec<u32>> = Mutex::new(Vec::new());
/// This process is the stand-in app: it tells the probe what it was typed, on its output.
static STAND_IN: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_CHAR => {
            TYPED.lock().unwrap().push(wparam.0 as u16);
            if STAND_IN.load(Ordering::SeqCst) {
                println!("char {}", wparam.0);
                let _ = std::io::stdout().flush();
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            COMMANDS.lock().unwrap().push(wparam.0 as u32 & 0xFFFF);
            LRESULT(0)
        }
        // The stand-in app types a key into itself: from then on it is the
        // process that had the last input, and Windows owes the probe nothing.
        WM_APP => {
            std::thread::spawn(|| svc_inject::type_text("k"));
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

/// File › New (Ctrl+N), File › Export › As PNG, File › Print (greyed), Help › About.
unsafe fn menu_bar() -> HMENU {
    unsafe {
        let (bar, file, export, help) = (CreateMenu().unwrap(), CreatePopupMenu().unwrap(), CreatePopupMenu().unwrap(), CreatePopupMenu().unwrap());
        let _ = AppendMenuW(export, MF_STRING, 103, w!("As &PNG..."));
        let _ = AppendMenuW(file, MF_STRING, 101, w!("&New\tCtrl+N"));
        let _ = AppendMenuW(file, MF_POPUP, export.0 as usize, w!("&Export"));
        let _ = AppendMenuW(file, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(file, MF_STRING | MF_GRAYED, 102, w!("&Print\tCtrl+P"));
        let _ = AppendMenuW(help, MF_STRING, 104, w!("&About the probe"));
        let _ = AppendMenuW(bar, MF_POPUP, file.0 as usize, w!("&File"));
        let _ = AppendMenuW(bar, MF_POPUP, help.0 as usize, w!("&Help"));
        bar
    }
}

/// A small window with its own message loop; this returns when it is closed.
fn window_loop(title: &str, at: i32, with_menu: bool, made: impl FnOnce(isize)) {
    unsafe {
        let module = GetModuleHandleW(None).unwrap();
        let class = WNDCLASSW { lpfnWndProc: Some(window_proc), hInstance: module.into(), lpszClassName: w!("lintel-command-bar-probe"), ..Default::default() };
        RegisterClassW(&class);
        let menu = with_menu.then(|| menu_bar());
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("lintel-command-bar-probe"),
            &HSTRING::from(title),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            80 + at * 40,
            80 + at * 150,
            460,
            130,
            None,
            menu,
            Some(module.into()),
            None,
        )
        .unwrap();
        made(hwnd.0 as isize);
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

fn own_window(title: &'static str, at: i32, with_menu: bool) -> isize {
    let (made, hwnd) = mpsc::channel();
    std::thread::spawn(move || window_loop(title, at, with_menu, |h| made.send(h).unwrap()));
    hwnd.recv().unwrap()
}

fn close_own(hwnd: isize) {
    unsafe {
        let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_CLOSE, WPARAM(0), LPARAM(0));
    }
}

fn front() -> isize {
    svc_windows::front().map_or(0, |w| w.hwnd)
}

/// Waits for `read` to give `want` (keys arrive a moment after they are sent).
fn arrives(want: &str, read: impl Fn() -> String) -> (bool, String) {
    for _ in 0..40 {
        if read() == want {
            return (true, want.to_string());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    (false, read())
}

// ── 1. Ranking ────────────────────────────────────────────────────────────────

fn ranking() {
    println!("\n── ranking, over a made-up catalogue");
    let catalogue = [
        ("Notepad", "", 80),
        ("Notes for the meeting", "", 0),
        ("Notion", "", 80),
        ("Display", "screen resolution monitor", -40),
        ("Brightness", "screen display", 0),
        ("Visual Studio Code", "code vscode", 80),
        ("Café Zürich", "", 0),
        ("Reset network", "", 0),
        ("Settings", "", 80),
    ];
    let candidates = |boosted: Option<usize>| -> Vec<Candidate<'static>> {
        catalogue
            .iter()
            .enumerate()
            .map(|(i, (title, keywords, bias))| Candidate { title, keywords, priority: 0, boost: *bias + if boosted == Some(i) { 300 } else { 0 } })
            .collect()
    };
    let titles = |query: &str, boosted| rank(&candidates(boosted), query).into_iter().map(|i| catalogue[i].0).collect::<Vec<_>>();

    let got = titles("not", None);
    check("start of a title", got == ["Notepad", "Notion", "Notes for the meeting"], format!("\"not\" -> {got:?}"));
    let got = titles("notepad", None);
    check("exact title leads", got.first() == Some(&"Notepad"), format!("\"notepad\" -> {got:?}"));
    let got = titles("code", None);
    check("whole word, and keywords", got == ["Visual Studio Code"], format!("\"code\" -> {got:?}"));
    let got = titles("display", None);
    check("title before keyword", got == ["Display", "Brightness"], format!("\"display\" -> {got:?}"));
    let got = titles("cafe zurich", None);
    check("accents and case", got == ["Café Zürich"], format!("\"cafe zurich\" -> {got:?}"));
    let got = titles("ntepad", None);
    check("a letter left out", got == ["Notepad"], format!("\"ntepad\" -> {got:?}"));
    let got = titles("noetpad", None);
    check("two letters swapped", got == ["Notepad"], format!("\"noetpad\" -> {got:?}"));
    let got = titles("set", None);
    check("a word's start before its middle", got == ["Settings", "Reset network"], format!("\"set\" -> {got:?}"));
    let got = titles("xyzzy", None);
    check("no match, no row", got.is_empty(), format!("\"xyzzy\" -> {got:?}"));
    let got = titles("not", Some(2));
    check("what is used most rises", got.first() == Some(&"Notion"), format!("\"not\", Notion used often -> {got:?}"));
    let got = titles("display", Some(4));
    check("but never over what was typed", got == ["Display", "Brightness"], format!("\"display\", Brightness used often -> {got:?}"));

    let mut learned = Learned::default();
    (0..5).for_each(|_| learned.record("app:a", 1000.0));
    learned.record("app:b", 1000.0);
    let (fresh, old) = (learned.boost("app:a", 1060.0), learned.boost("app:a", 1000.0 + 30.0 * 86400.0));
    check("habit fades", fresh == 60 && old == 10 && learned.boost("app:c", 1060.0) == 0, format!("5 runs: +{fresh} within two hours, +{old} a month later, +0 never run"));
    let most = learned.most_used(["app:b", "app:c", "app:a"].into_iter());
    check("most used first", most == ["app:a", "app:b"], format!("{most:?}"));
}

// ── 2. Answers ────────────────────────────────────────────────────────────────

fn answers() {
    println!("\n── calculator");
    let (en, de) = (Separators::default(), Separators { decimal: ',', grouping: '.' });
    let sum = |input: &str, sep| math::evaluate(input, sep).map(|a| a.text);
    for (input, want) in [
        ("2*(3+4)", Some("14")),
        ("2*(3+4", Some("14")),
        ("0.1 + 0.2", Some("0.3")),
        ("-2^2", Some("-4")),
        ("2^3^2", Some("512")),
        ("100 + 10%", Some("110")),
        ("15% of 80", Some("12")),
        ("2π", Some("6.28318531")),
        ("sqrt(16) + abs(-3)", Some("7")),
        ("1,234.5 * 2", Some("2,469")),
        ("1e3 / 4", Some("250")),
        ("10 / 4 =", Some("2.5")),
        ("2 x 3", Some("6")),
        ("9999999 * 9999999", Some("9.999998e13")),
        ("42", None),
        ("notepad", None),
        ("27/07/2026", None),
        ("10:30", None),
        ("1 / 0", None),
        ("2 +", None),
        ("x = 2", None),
    ] {
        let got = sum(input, en);
        check("sum", got.as_deref() == want, format!("{input:?} -> {got:?}"));
    }
    let got = sum("1.234,5 * 2", de);
    check("comma as the decimal point", got.as_deref() == Some("2.469"), format!("\"1.234,5 * 2\" where numbers are written 1.234,5 -> {got:?}"));
    let got = math::evaluate("2*(3+4", en).map(|a| a.closing);
    check("the missing bracket is supplied", got.as_deref() == Some(")"), format!("\"2*(3+4\" -> closing {got:?}"));
    let got = math::evaluate("1000 * -2.5", en).map(|a| math::reusable(&a, en));
    check("an answer to go on with", got.as_deref() == Some("(-2500)"), format!("\"1000 * -2.5\" -> Tab puts {got:?} in the field"));

    println!("\n── conversions");
    for (input, want) in [
        ("10 km in mi", Some("6.21 mi")),
        ("10km to mi", Some("6.21 mi")),
        ("20c to f", Some("68°F")),
        ("-40 f to c", Some("-40°C")),
        ("5 in to cm", Some("12.7 cm")),
        ("180 cm to ft", Some("5 ft 10.87 in")),
        ("5 gb to mb", Some("5,000 MB")),
        ("1 gib to mb", Some("1,073.7 MB")),
        ("90 min to h", Some("1.5 h")),
        ("2.5 kg to lb", Some("5.51 lb")),
        ("1 gal to l", Some("3.79 L")),
        ("10 km in kg", None),
        ("10 km", None),
        ("km to mi", None),
        ("open the door to mi", None),
    ] {
        let got = units::convert(input, en);
        check("conversion", got.as_deref() == want, format!("{input:?} -> {got:?}"));
    }
    let got = units::convert("1,5 km in m", de);
    check("comma as the decimal point", got.as_deref() == Some("1.500 m"), format!("\"1,5 km in m\" -> {got:?}"));

    println!("\n── dates (counted from Sunday 11 October 2026; written as this PC writes dates)");
    let today = Day { year: 2026, month: 10, day: 11 };
    let days = |n: u64| format!("{n} days");
    let on = |year, month, day| Day { year, month, day }.long();
    for (input, want) in [
        ("tomorrow + 3 days", Some(on(2026, 10, 15))),
        ("tomorrow+3 days", Some(on(2026, 10, 15))),
        ("today + 10 days", Some(on(2026, 10, 21))),
        ("in 3 weeks", Some(on(2026, 11, 1))),
        ("3 days ago", Some(on(2026, 10, 8))),
        ("yesterday - 2 weeks", Some(on(2026, 9, 26))),
        ("today + 4 months", Some(on(2027, 2, 11))),
        ("in 1 year", Some(on(2027, 10, 11))),
        ("demain + 3 jours", Some(on(2026, 10, 15))),
        ("3 days", None),
        ("days", None),
    ] {
        let got = dates::evaluate(input, today, true, days).map(|a| a.text);
        check("date", got == want, format!("{input:?} -> {got:?}"));
    }
    let end_of_month = dates::evaluate("today + 1 month", Day { year: 2027, month: 1, day: 31 }, true, days).map(|a| a.text);
    check("the 31st plus a month", end_of_month == Some(on(2027, 2, 28)), format!("31 January 2027 + 1 month -> {end_of_month:?}"));
    let leap = dates::evaluate("today + 1 day", Day { year: 2028, month: 2, day: 28 }, true, days).map(|a| a.text);
    check("a leap year", leap == Some(on(2028, 2, 29)), format!("28 February 2028 + 1 day -> {leap:?}"));
    let until = dates::evaluate("days until 25/12", today, true, days);
    check("days until", until.as_ref().map(|a| a.text.as_str()) == Some("75 days"), format!("\"days until 25/12\" -> {until:?}"));
    let until = dates::evaluate("days until 12/25", today, false, days).map(|a| a.text);
    check("month first where it is written so", until.as_deref() == Some("75 days"), format!("\"days until 12/25\" -> {until:?}"));
    let next = dates::evaluate("days until 1/3", today, true, days).map(|a| a.text);
    check("a day gone by means next year", next.as_deref() == Some("141 days"), format!("\"days until 1/3\" -> {next:?}"));
    let weekday = dates::evaluate("tomorrow + 3 days", today, true, days).map(|a| a.detail);
    println!("      the line under it: {weekday:?}; this PC writes numbers as {:?} and the day first: {}", dates::separators(), dates::day_first());
}

// ── 3–6. This PC ──────────────────────────────────────────────────────────────

fn search(cat: &Catalog, text: &str, chip: &str, extras: Vec<Row>) -> Vec<Row> {
    rows::search(&Query { text: text.into(), chip: chip.into(), extras }, cat, &Learned::default(), 0.0)
}

/// A row as the probe may print it: a window that is not the probe's keeps its title to itself.
fn shown(row: &Row) -> String {
    let title = if row.kind == "window" && !row.title.contains("command-bar-probe") { format!("<{} characters>", row.title.chars().count()) } else { row.title.clone() };
    format!("[{}] {}{}{}", row.kind, row.glyph, title, if row.subtitle.is_empty() || row.kind == "window" { String::new() } else { format!(" · {}", row.subtitle) })
}

fn first(rows: &[Row]) -> String {
    rows.first().map_or("nothing".into(), shown)
}

fn main() {
    // The stand-in for the app in front: this program again, as a second process with one window.
    if std::env::args().nth(1).as_deref() == Some("--stand-in") {
        STAND_IN.store(true, Ordering::SeqCst);
        window_loop("command-bar-probe stand-in app", 0, false, |hwnd| {
            println!("hwnd {hwnd}");
            let _ = std::io::stdout().flush();
        });
        return;
    }

    ranking();
    answers();

    println!("\n── the windows that are open, and the installed apps");
    let mut child = Command::new(std::env::current_exe().unwrap()).arg("--stand-in").stdout(Stdio::piped()).spawn().unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let app: isize = lines.next().and_then(|l| l.ok()).and_then(|l| l.strip_prefix("hwnd ").and_then(|h| h.parse().ok())).expect("the stand-in's window");
    static APP_TYPED: Mutex<Vec<u16>> = Mutex::new(Vec::new());
    std::thread::spawn(move || {
        for line in lines.map_while(Result::ok) {
            if let Some(code) = line.strip_prefix("char ").and_then(|c| c.parse::<u16>().ok()) {
                APP_TYPED.lock().unwrap().push(code);
            }
        }
    });
    let bar = own_window("command-bar-probe stand-in bar", 1, true);
    let other = own_window("command-bar-probe another window", 2, false);
    std::thread::sleep(Duration::from_millis(300));

    let windows = svc_windows::windows();
    let mine = |hwnd: isize| windows.iter().find(|w| w.hwnd == hwnd);
    check("open windows", windows.len() >= 3, format!("{} windows Alt+Tab would show, {} of them minimised", windows.len(), windows.iter().filter(|w| w.minimized).count()));
    check(
        "the probe's three are among them",
        [app, bar, other].iter().all(|h| mine(*h).is_some()),
        format!("{:?}", [app, bar, other].iter().filter_map(|h| mine(*h)).map(|w| w.title.as_str()).collect::<Vec<_>>()),
    );
    let stand_in = mine(app);
    check(
        "a window of another process, with its program",
        stand_in.is_some_and(|w| w.pid == child.id() && w.app() == "command-bar-probe" && std::path::Path::new(&w.exe).is_file()),
        format!("pid {:?} (the stand-in is {}), program \"{}\"", stand_in.map(|w| w.pid), child.id(), stand_in.map_or("", |w| w.app())),
    );
    let programs = windows.iter().filter(|w| !w.exe.is_empty()).count();
    println!("      {programs} of {} windows tell their program; none of the others' titles is printed", windows.len());

    let began = std::time::Instant::now();
    let apps = svc_windows::apps();
    let took = began.elapsed();
    let packaged = apps.iter().filter(|a| a.target.is_empty()).count();
    check("installed apps", apps.len() > 20, format!("{} apps in {} ms: {} with a program behind them, {packaged} packaged", apps.len(), took.as_millis(), apps.len() - packaged));
    for name in ["Notepad", "Calculator", "Settings", "File Explorer"] {
        let found = apps.iter().find(|a| a.name == name);
        check("a standard app", found.is_some(), format!("{name}: id {:?}", found.map(|a| a.id.as_str())));
    }
    let leftovers = apps.iter().filter(|a| a.name.to_lowercase().contains("uninstall") || a.target.to_lowercase().ends_with(".url")).count();
    check("no uninstallers or web links", leftovers == 0, format!("{leftovers} left in the list"));
    let icon = apps.iter().find(|a| a.name == "Notepad").map(|a| svc_shell::icon(&a.id, 32).map(|png| png.len()));
    check("an app's picture", matches!(icon, Some(Ok(n)) if n > 100), format!("Notepad at 32 px: {icon:?} bytes of PNG"));

    println!("\n── the keyboard, taken and handed back (real keys, into the probe's own windows)");
    let was_front = front();
    let came = svc_windows::activate(app);
    check("the stand-in app comes to the front", came && front() == app, format!("asked from a process that was not in front (in front before: {})", if [app, bar, other].contains(&was_front) { "one of the probe's" } else { "a window that is not the probe's" }));
    unsafe {
        let _ = PostMessageW(Some(HWND(app as *mut _)), WM_APP, WPARAM(0), LPARAM(0));
    }
    let (typed_itself, _) = arrives("k", || String::from_utf16_lossy(&APP_TYPED.lock().unwrap()));
    let before = svc_windows::remember();
    check("who has the keyboard is noted", typed_itself && before.hwnd() == app && before.still_there(), "the stand-in app, which has just been typed in by its own process");
    // What Windows answers to the plain request, before `activate` is asked: it may refuse it.
    let said = unsafe { SetForegroundWindow(HWND(bar as *mut _)) }.as_bool();
    std::thread::sleep(Duration::from_millis(80));
    let plain = front() == bar;
    if plain {
        svc_windows::activate(app);
    }
    let took = svc_windows::activate(bar);
    check(
        "the bar takes the keyboard",
        took && front() == bar,
        format!("SetForegroundWindow alone answered {said} and the bar was {}; activate: in front = {took}, read back from Windows", if plain { "in front" } else { "NOT in front (refused)" }),
    );
    let typed = svc_inject::type_text("bar ✓ é");
    let (ok, got) = arrives("bar ✓ é", || String::from_utf16_lossy(&TYPED.lock().unwrap()));
    check("keys reach the bar", typed.is_ok() && ok, format!("typed \"bar ✓ é\", the bar's window received {got:?} ({typed:?})"));
    let back = before.give_back(bar);
    check("the keyboard goes back", back && front() == app, "to the stand-in app, read back from Windows");
    let typed = svc_inject::type_text("app 🔥");
    let (ok, got) = arrives("kapp 🔥", || String::from_utf16_lossy(&APP_TYPED.lock().unwrap()));
    check("keys reach the app again", typed.is_ok() && ok, format!("typed \"app 🔥\" (an emoji row), the other process received {got:?} after its own k ({typed:?})"));
    check("and not the bar", String::from_utf16_lossy(&TYPED.lock().unwrap()) == "bar ✓ é", "the bar's window received nothing more");

    let before = svc_windows::remember();
    let took = svc_windows::activate(bar);
    let chose = svc_windows::activate(other);
    let back = before.give_back(bar);
    check("a window the user chose keeps the keyboard", took && chose && !back && front() == other, format!("another window came to the front while the bar was open: handed back = {back}, in front = {}", if front() == other { "that other window" } else { "something else" }));

    let before = svc_windows::remember();
    let took = svc_windows::activate(bar);
    close_own(other);
    std::thread::sleep(Duration::from_millis(200));
    let (there, back) = (before.still_there(), before.give_back(bar));
    check("a window that closed meanwhile is not gone back to", took && before.hwnd() == other && !there && !back && front() == bar, format!("still there = {there}, handed back = {back}; the bar stays in front until it hides"));
    check("a window that is gone cannot be put in front", !svc_windows::activate(other), "activate answers false");

    println!("\n── the menu of the window in front");
    let menu = rows::menu_commands(bar);
    let listed: Vec<String> = menu.iter().map(|m| format!("{}{} [{}] #{}", if m.path.is_empty() { String::new() } else { format!("{} › ", m.path) }, m.title, m.keys, m.id)).collect();
    check(
        "read without opening it",
        listed == ["File › New [Ctrl+N] #101", "File › Export › As PNG [] #103", "Help › About the probe [] #104"],
        format!("{listed:?} (the greyed Print is left out)"),
    );
    check("a window without a menu bar", rows::menu_commands(app).is_empty(), "the stand-in app: no rows");
    unsafe {
        let _ = PostMessageW(Some(HWND(bar as *mut _)), WM_COMMAND, WPARAM(103), LPARAM(0));
    }
    let (ok, got) = arrives("[103]", || format!("{:?}", COMMANDS.lock().unwrap()));
    check("a command is sent to the window", ok, format!("WM_COMMAND received: {got}"));

    println!("\n── the on-screen keyboard");
    let names = ["C:\\Windows\\System32\\osk.exe", "TabTip.exe", "C:\\Windows\\SystemApps\\MicrosoftWindows.Client.CBS_cw5n1h2txyewy\\TextInputHost.exe", "notepad.exe", "C:\\tools\\not-osk.exe", ""];
    let verdicts: Vec<bool> = names.iter().map(|n| svc_windows::is_keyboard_exe(n)).collect();
    check("its programs, by name", verdicts == [true, true, true, false, false, false], format!("{verdicts:?} for osk, TabTip, TextInputHost, notepad, not-osk, none"));
    let on_bar = svc_windows::is_onscreen_keyboard_point(80 + 40 + 200, 80 + 150 + 60);
    check("a click on another window is not a key press", !on_bar && !svc_windows::is_onscreen_keyboard(bar), "a point on the probe's own window: false");
    println!("      not tried with a real on-screen keyboard: the probe starts no program of the user's");

    println!("\n── the list, for a few queries (this PC's apps and windows)");
    let cat = Catalog {
        apps,
        windows: svc_windows::windows(),
        menu,
        menu_app: "command-bar-probe".into(),
        links: rows::links("gh = https://github.com/search?q={query}\nnotes = ~\\Documents\\notes"),
        sep: Separators::default(),
        day_first: true,
    };
    let none = Vec::new;
    let got = search(&cat, "stand-in app", "all", none());
    check("an open window", got.first().is_some_and(|r| r.id == format!("win:{app}")), first(&got));
    let got = search(&cat, "notepad", "all", none());
    check("an app", got.first().is_some_and(|r| r.kind == "app" && r.title == "Notepad"), first(&got));
    let got = search(&cat, "calc", "all", none());
    check("an app, by the start of its name", got.first().is_some_and(|r| r.kind == "app" && r.title == "Calculator"), first(&got));
    let got = search(&cat, ":fire", "all", none());
    check("an emoji, after a colon", got.first().is_some_and(|r| r.id == "emoji:🔥") && got.iter().all(|r| r.kind == "emoji"), first(&got));
    let got = search(&cat, "thanks", "emoji", none());
    check("an emoji, by another word for it", got.iter().any(|r| r.glyph == "🙏"), format!("{:?}", got.iter().map(|r| r.glyph.as_str()).collect::<Vec<_>>()));
    let got = search(&cat, "night light", "all", none());
    check("a page of Windows settings", got.first().is_some_and(|r| r.id == "os:ms-settings:nightlight"), first(&got));
    let extras = vec![
        Row { id: "set:island".into(), kind: "settings".into(), title: "Island".into(), subtitle: "METEOR V1 settings".into(), ..Default::default() },
        Row { id: "clipboard:42".into(), kind: "clipboard".into(), title: "island of the day".into(), subtitle: "Clipboard".into(), ..Default::default() },
    ];
    let got = search(&cat, "island", "all", extras.clone());
    check(
        "rows of other features join the list",
        got.iter().any(|r| r.id == "set:island") && got.iter().any(|r| r.id == "clipboard:42"),
        format!("{:?}", got.iter().filter(|r| r.kind != "window").map(shown).collect::<Vec<_>>()),
    );
    let got = search(&cat, "island", "clipboard", extras);
    check("a filter keeps one kind", got.len() == 1 && got[0].id == "clipboard:42", format!("under Clipboard: {:?}", got.iter().map(shown).collect::<Vec<_>>()));
    let got = search(&cat, "2*(3+4)", "all", none());
    check("a sum", got.first().is_some_and(|r| r.id == "answer" && r.title == "14" && r.copy == "14"), first(&got));
    let got = search(&cat, "10 km in mi", "all", none());
    check("a conversion", got.first().is_some_and(|r| r.id == "answer" && r.title == "6.21 mi"), first(&got));
    let got = search(&cat, "tomorrow + 3 days", "all", none());
    check("a date", got.first().is_some_and(|r| r.id == "answer" && r.title == Day::today().plus_days(4).long()), first(&got));
    let got = search(&cat, "export", "all", none());
    check("a menu command of the app in front", got.iter().any(|r| r.id == "menu:103"), format!("{:?}", got.iter().filter(|r| r.kind == "menu").map(shown).collect::<Vec<_>>()));
    let got = search(&cat, "gh METEOR V1 bar", "all", none());
    check("a saved search", got.first().is_some_and(|r| r.id == "link:gh" && r.subtitle == "https://github.com/search?q=METEOR V1%20bar"), first(&got));
    let got = search(&cat, "example.com/docs", "all", none());
    check("a web address", got.first().is_some_and(|r| r.id == "url" && r.title == "Open https://example.com/docs"), first(&got));
    let got = search(&cat, "", "all", none());
    let headings: Vec<&str> = got.iter().map(|r| r.section.as_str()).fold(Vec::new(), |mut seen, s| {
        if seen.last() != Some(&s) {
            seen.push(s);
        }
        seen
    });
    check("nothing typed", headings == ["Suggested", "Open windows"] && got.len() <= 14, format!("{} rows under {headings:?}", got.len()));
    let got = search(&cat, "", "windows", none());
    let others = cat.windows.iter().filter(|w| w.pid != std::process::id()).count();
    check("a filter with nothing typed", got.len() == others && got.iter().all(|r| r.kind == "window"), format!("{} windows (the bar's own program is left out)", got.len()));
    let got = search(&cat, "a", "all", none());
    let most = ["app", "window", "action", "settings"].map(|kind| got.iter().filter(|r| r.kind == kind).count());
    check("the mixed list is short and mixed", got.len() <= 12 && most[0] <= 5 && most[1] <= 4, format!("\"a\" -> {} rows: {} apps, {} windows, {} actions, {} settings", got.len(), most[0], most[1], most[2], most[3]));

    println!("\n── commands that change the PC: listed, not run");
    for row in search(&cat, "", "actions", none()).iter().filter(|r| r.id.starts_with("sys:")) {
        println!("      {} · {}", row.title, if row.confirm.is_empty() { "runs on Enter".to_string() } else { format!("asks first: \"{}\"", row.confirm) });
    }
    let asks = search(&cat, "shut down", "all", none());
    check("ending the session asks first", asks.first().is_some_and(|r| r.id == "sys:shutdown" && !r.confirm.is_empty()), first(&asks));
    let readings: Vec<String> = search(&cat, "", "actions", none()).iter().filter(|r| r.kind == "info").map(|r| format!("{} {}", r.title, r.subtitle)).collect();
    check("readings of this PC", readings.len() >= 4, format!("{readings:?}"));

    println!("\n── files by name (a folder of the probe's own, under target/)");
    let folder = std::env::current_exe().unwrap().with_file_name("command-bar-probe-files");
    let _ = std::fs::remove_dir_all(&folder);
    for file in ["Quarterly Report 2026.txt", "notes/Report draft.md", "notes/todo.txt", ".hidden/Report secret.txt", "node_modules/report.js", "Résumé.txt"] {
        let path = folder.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "probe").unwrap();
    }
    let names = |query: &str| rows::files(query, &folder.to_string_lossy()).into_iter().map(|r| r.title).collect::<Vec<_>>();
    let got = names("report");
    check("every word in the name, hidden and vendored folders left alone", got == ["Quarterly Report 2026.txt", "Report draft.md"], format!("\"report\" -> {got:?}"));
    let got = names("2026 quarterly");
    check("words in any order", got == ["Quarterly Report 2026.txt"], format!("\"2026 quarterly\" -> {got:?}"));
    let got = names("resume");
    check("accents", got == ["Résumé.txt"], format!("\"resume\" -> {got:?}"));
    check("one letter is not a search", names("r").is_empty(), "\"r\" -> nothing");
    let _ = std::fs::remove_dir_all(&folder);

    // The probe's windows go; Windows gives the front to whichever window it picks next.
    close_own(bar);
    close_own(app);
    let _ = child.wait();
    let failed = FAILED.load(Ordering::SeqCst);
    println!("\n{}", if failed == 0 { "command-bar-probe: every check passed".to_string() } else { format!("command-bar-probe: {failed} checks FAILED") });
    std::process::exit(if failed == 0 { 0 } else { 1 });
}
