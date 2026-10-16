// What the bar lists, and in which order: the rows it makes itself (apps, open
// windows, the menu of the app in front, system commands, folders, Windows
// settings, emoji, links, files, answers) put together with the rows the page
// hands over from other features, then ranked for what was typed.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use notch_core::i18n::{t, tf, tn};
use serde::{Deserialize, Serialize};
use svc_windows::{App, Win};
use windows::core::{HSTRING, PWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::UI::WindowsAndMessaging::{
    GetMenu, GetMenuItemCount, GetMenuItemInfoW, HMENU, MENUITEMINFOW, MFS_GRAYED, MFT_OWNERDRAW, MFT_SEPARATOR, MIIM_FTYPE,
    MIIM_ID, MIIM_STATE, MIIM_STRING, MIIM_SUBMENU,
};

use crate::dates::{self, Day};
use crate::math::{self, Separators};
use crate::search::{fold, rank, Candidate, Learned};
use crate::{emoji, units};

/// One row of the bar. The page draws it; `id` says what Enter does.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Row {
    /// `app:<id>`, `win:<handle>`, `sys:<name>`, `folder:<name>`, `os:<address>`,
    /// `info:<name>`, `emoji:<glyph>`, `menu:<command>`, `link:<name>`, `file:<path>`,
    /// `url`, `answer`; a row of another feature keeps the id its feature gave it.
    pub id: String,
    /// app, window, action, settings, emoji, menu, link, file, info, answer, or
    /// the id of the feature a row comes from.
    pub kind: String,
    pub title: String,
    pub subtitle: String,
    /// Extra words the row is found by.
    #[serde(skip_serializing)]
    pub keywords: String,
    /// An icon of the shared set, or one of the bar's own (the page knows them).
    pub icon: String,
    /// What to ask `icon` for instead: an app id, a program or a file.
    pub picture: String,
    /// An emoji, drawn where the icon goes.
    pub glyph: String,
    /// Keys shown at the right end: a menu command's own, a feature's shortcut.
    pub hint: String,
    /// The app has a window open.
    pub running: bool,
    /// What the bar asks before it runs the row. Empty: Enter runs it.
    pub confirm: String,
    /// What Enter copies (an answer) and Tab puts back in the field.
    pub copy: String,
    /// Counted when it is run, so that it rises; never for what is gone tomorrow (a window, a copied text).
    pub learn: bool,
    /// A heading the list shows above the first row that carries it.
    pub section: String,
}

fn row(id: String, kind: &str, title: String, subtitle: String, icon: &str) -> Row {
    Row { id, kind: kind.into(), title, subtitle, icon: icon.into(), ..Default::default() }
}

/// A row with an English name: found by it whatever language it is shown in.
fn named(id: &str, kind: &str, name: &str, subtitle: &str, icon: &str, more: &str) -> Row {
    Row { keywords: format!("{name} {more}"), learn: true, ..row(id.into(), kind, t(name), t(subtitle), icon) }
}

// ── Apps and windows ──────────────────────────────────────────────────────────

fn same_file(a: &str, b: &str) -> bool {
    !a.is_empty() && a.eq_ignore_ascii_case(b)
}

fn app_rows(apps: &[App], windows: &[Win]) -> Vec<Row> {
    let me = std::env::current_exe().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
    apps.iter()
        .filter(|a| !same_file(&a.target, &me))
        .map(|a| {
            let stem = Path::new(&a.target).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            Row {
                keywords: if fold(&stem) == fold(&a.name) { String::new() } else { stem },
                picture: a.id.clone(),
                running: windows.iter().any(|w| same_file(&w.exe, &a.target)),
                learn: true,
                ..row(format!("app:{}", a.id), "app", a.name.clone(), t("App"), "grid")
            }
        })
        .collect()
}

fn window_rows(apps: &[App], windows: &[Win]) -> Vec<Row> {
    let me = std::process::id();
    windows
        .iter()
        .filter(|w| w.pid != me)
        .map(|w| {
            // The name the Start menu gives the program, else the program's own.
            let app = apps.iter().find(|a| same_file(&a.target, &w.exe)).map_or(w.app(), |a| a.name.as_str());
            Row { picture: w.exe.clone(), ..row(format!("win:{}", w.hwnd), "window", w.title.clone(), app.to_string(), "monitor") }
        })
        .collect()
}

// ── The menu of the app in front (classic Win32 menus only) ───────────────────

/// One command of a window's menu bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuCommand {
    /// What the window is sent in WM_COMMAND.
    pub id: u32,
    pub title: String,
    /// "File › Export".
    pub path: String,
    /// The shortcut the menu shows beside it.
    pub keys: String,
}

unsafe fn walk_menu(menu: HMENU, path: &str, depth: u32, out: &mut Vec<MenuCommand>) {
    let count = unsafe { GetMenuItemCount(Some(menu)) }.max(0) as u32;
    for at in 0..count {
        let mut text = [0u16; 256];
        let mut info = MENUITEMINFOW {
            cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
            fMask: MIIM_STRING | MIIM_ID | MIIM_SUBMENU | MIIM_STATE | MIIM_FTYPE,
            dwTypeData: PWSTR(text.as_mut_ptr()),
            cch: text.len() as u32,
            ..Default::default()
        };
        if unsafe { GetMenuItemInfoW(menu, at, true, &mut info) }.is_err() {
            continue;
        }
        // Lines, items the program draws itself (no text to read) and what is greyed out.
        if info.fType.0 & (MFT_SEPARATOR.0 | MFT_OWNERDRAW.0) != 0 || info.fState.0 & MFS_GRAYED.0 != 0 {
            continue;
        }
        let raw = String::from_utf16_lossy(&text[..(info.cch as usize).min(text.len())]);
        let (label, keys) = raw.split_once('\t').unwrap_or((&raw, ""));
        let label = label.replace('&', "").trim().trim_end_matches(['.', '…']).to_string();
        if label.is_empty() {
            continue;
        }
        if !info.hSubMenu.is_invalid() {
            if depth < 4 {
                let below = if path.is_empty() { label } else { format!("{path} › {label}") };
                unsafe { walk_menu(info.hSubMenu, &below, depth + 1, out) };
            }
        } else if out.len() < 400 {
            out.push(MenuCommand { id: info.wID, title: label, path: path.to_string(), keys: keys.trim().to_string() });
        }
    }
}

/// The commands of a window's menu bar, read without opening anything and
/// without a message to the program (so a hung one cannot stall the bar).
/// Empty for a program that draws its menus itself (a ribbon, a browser).
pub fn menu_commands(hwnd: isize) -> Vec<MenuCommand> {
    let mut out = Vec::new();
    unsafe {
        let menu = GetMenu(HWND(hwnd as *mut _));
        if !menu.is_invalid() {
            walk_menu(menu, "", 0, &mut out);
        }
    }
    out
}

fn menu_rows(menu: &[MenuCommand], app: &str) -> Vec<Row> {
    menu.iter()
        .map(|m| {
            let crumb = if m.path.is_empty() { app.to_string() } else { format!("{app} › {}", m.path) };
            // Found by the menus it sits in as well: "export png".
            Row { hint: m.keys.clone(), keywords: m.path.replace('›', " "), ..row(format!("menu:{}", m.id), "menu", m.title.clone(), crumb, "sliders") }
        })
        .collect()
}

// ── System commands, folders, answers, Windows settings ───────────────────────

fn system_rows() -> Vec<Row> {
    let sure = |mut r: Row, question: &str| {
        r.confirm = t(question);
        r
    };
    let folder = t("Folder");
    let mut rows = vec![
        named("sys:lock", "action", "Lock the PC", "System", "lock", "lock screen"),
        named("sys:sleep", "action", "Sleep", "System", "moon", "suspend standby"),
        sure(named("sys:restart", "action", "Restart the PC", "System", "refresh", "reboot"), "Restart this PC now?"),
        sure(named("sys:shutdown", "action", "Shut down the PC", "System", "power", "power off turn off"), "Shut down this PC now?"),
        sure(named("sys:signout", "action", "Sign out", "System", "external", "log out logoff"), "Sign out of Windows now?"),
        sure(named("sys:bin", "action", "Empty the recycle bin", "System", "trash", "delete trash"), "Delete everything in the recycle bin for good?"),
    ];
    for (shell, name) in [
        ("shell:Downloads", "Downloads"),
        ("shell:Personal", "Documents"),
        ("shell:Desktop", "Desktop"),
        ("shell:My Pictures", "Pictures"),
        ("shell:My Music", "Music"),
        ("shell:My Video", "Videos"),
        ("shell:Profile", "Home folder"),
        ("shell:RecycleBinFolder", "Recycle bin"),
    ] {
        rows.push(Row { subtitle: folder.clone(), ..named(&format!("folder:{shell}"), "action", name, "", "folder", "folder open") });
    }
    rows
}

/// Readings of this PC, each with its value; Enter copies the value.
fn info_rows(sep: Separators) -> Vec<Row> {
    let info = |id: &str, name: &str, value: String, icon: &str| Row { copy: value.clone(), ..named(id, "info", name, "", icon, "") }.with_subtitle(value);
    let mut rows = Vec::new();
    unsafe {
        let mut power = SYSTEM_POWER_STATUS::default();
        if GetSystemPowerStatus(&mut power).is_ok() && power.BatteryLifePercent <= 100 {
            rows.push(info("info:battery", "Battery", format!("{} %", power.BatteryLifePercent), "battery"));
        }
        let mut memory = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
        if GlobalMemoryStatusEx(&mut memory).is_ok() {
            rows.push(info("info:memory", "Memory in use", format!("{} %", memory.dwMemoryLoad), "cpu"));
        }
        let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
        let (mut free, mut total) = (0u64, 0u64);
        if GetDiskFreeSpaceExW(&HSTRING::from(format!("{drive}\\")), Some(&mut free), Some(&mut total), None).is_ok() && total > 0 {
            let gb = |bytes: u64| math::grouped(bytes as f64 / 1e9, 1, sep);
            rows.push(info("info:storage", "Free storage", tf("{free} GB free of {total} GB", &[("free", &gb(free)), ("total", &gb(total))]), "download"));
        }
    }
    let today = Day::today();
    rows.push(info("info:today", "Today", today.long(), "calendar"));
    rows.push(info("info:time", "Time now", dates::time_now(), "timer"));
    rows
}

impl Row {
    fn with_subtitle(mut self, subtitle: String) -> Self {
        self.subtitle = subtitle;
        self
    }
}

/// The pages of the Windows Settings app: address after `ms-settings:`, name, extra words.
const OS_SETTINGS: [(&str, &str, &str); 40] = [
    ("display", "Display", "screen resolution scale monitor hdr"),
    ("nightlight", "Night light", "blue light warm"),
    ("sound", "Sound", "audio volume speakers output input"),
    ("notifications", "Notifications", "do not disturb banners"),
    ("quiethours", "Focus", "do not disturb quiet"),
    ("powersleep", "Power and battery", "sleep energy saver screen off"),
    ("storagesense", "Storage", "disk space cleanup"),
    ("multitasking", "Multitasking", "snap windows desktops alt tab"),
    ("clipboard", "Clipboard settings", "history sync"),
    ("about", "About this PC", "device name specifications version"),
    ("bluetooth", "Bluetooth and devices", "pair headphones"),
    ("printers", "Printers and scanners", "print"),
    ("mousetouchpad", "Mouse", "pointer speed buttons scroll"),
    ("devices-touchpad", "Touchpad", "gestures tap"),
    ("typing", "Typing", "keyboard autocorrect suggestions"),
    ("network", "Network and internet", "connection ethernet"),
    ("network-wifi", "Wi-Fi", "wireless wifi"),
    ("network-vpn", "VPN", "virtual private network"),
    ("network-proxy", "Proxy", ""),
    ("network-airplanemode", "Airplane mode", "flight radio"),
    ("network-mobilehotspot", "Mobile hotspot", "tethering share"),
    ("personalization", "Personalization", "look appearance"),
    ("personalization-background", "Background", "wallpaper desktop picture"),
    ("colors", "Colors", "dark mode light accent theme"),
    ("themes", "Themes", ""),
    ("lockscreen", "Lock screen", ""),
    ("taskbar", "Taskbar", "system tray pinned"),
    ("appsfeatures", "Installed apps", "uninstall remove programs"),
    ("defaultapps", "Default apps", "browser file types open with"),
    ("startupapps", "Startup apps", "autostart boot login"),
    ("optionalfeatures", "Optional features", ""),
    ("yourinfo", "Your account", "profile picture microsoft"),
    ("signinoptions", "Sign-in options", "password pin fingerprint face hello"),
    ("dateandtime", "Date and time", "clock time zone"),
    ("regionlanguage", "Language and region", "keyboard layout input format"),
    ("easeofaccess", "Accessibility", "text size contrast narrator magnifier"),
    ("privacy", "Privacy and security", "permissions"),
    ("privacy-microphone", "Microphone access", "privacy permission"),
    ("privacy-webcam", "Camera access", "privacy permission webcam"),
    ("windowsupdate", "Windows Update", "updates patch"),
];

fn os_settings_rows() -> Vec<Row> {
    OS_SETTINGS.iter().map(|(page, name, more)| named(&format!("os:ms-settings:{page}"), "settings", name, "Windows settings", "gear", more)).collect()
}

fn emoji_rows() -> Vec<Row> {
    emoji::EMOJI
        .lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let (glyph, name, more) = (parts.next()?, parts.next()?, parts.next().unwrap_or_default());
            Some(Row { glyph: glyph.into(), keywords: more.into(), learn: true, ..row(format!("emoji:{glyph}"), "emoji", name.into(), t("Emoji"), "") })
        })
        .collect()
}

// ── Links ─────────────────────────────────────────────────────────────────────

/// A link, a folder or a file the user named: `name = destination`, one per
/// line in Settings. `{query}` in a destination makes it a search: "gh METEOR V1"
/// opens the link named gh with "lintel" in that place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub name: String,
    pub destination: String,
}

pub fn links(setting: &str) -> Vec<Link> {
    setting
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(name, destination)| Link { name: name.trim().to_string(), destination: destination.trim().to_string() })
        .filter(|l| !l.name.is_empty() && !l.destination.is_empty())
        .take(60)
        .collect()
}

fn is_place(destination: &str) -> bool {
    let bytes = destination.as_bytes();
    destination.starts_with('~') || destination.starts_with("\\\\") || (bytes.len() > 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic())
}

fn percent(text: &str) -> String {
    text.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// Where a link leads once what was typed after its name is put in its place.
/// A folder or a file takes the text as it is, a web address takes it escaped.
pub fn destination(link: &Link, argument: &str) -> String {
    if is_place(&link.destination) {
        let home = notch_core::win::home_dir().to_string_lossy().to_string();
        let path = link.destination.replace("{query}", argument);
        return path.strip_prefix('~').map_or(path.clone(), |rest| format!("{home}{rest}"));
    }
    let address = link.destination.replace("{query}", &percent(argument));
    if address.contains("://") || address.starts_with("mailto:") { address } else { format!("https://{address}") }
}

/// The link a query calls by name ("gh METEOR V1"), with what follows the name.
pub fn saved_search<'a>(links: &'a [Link], query: &'a str) -> Option<(&'a Link, &'a str)> {
    let (name, argument) = query.trim().split_once(' ')?;
    let link = links.iter().find(|l| l.destination.contains("{query}") && l.name.eq_ignore_ascii_case(name))?;
    Some((link, argument.trim())).filter(|(_, argument)| !argument.is_empty())
}

fn link_rows(links: &[Link]) -> Vec<Row> {
    links
        .iter()
        .map(|l| {
            let icon = if is_place(&l.destination) { "folder" } else { "link" };
            Row { learn: true, ..row(format!("link:{}", l.name), "link", l.name.clone(), l.destination.replace("{query}", "…"), icon) }
        })
        .collect()
}

/// A web address typed on its own: "example.com/page", "https://example.com".
pub fn web_address(query: &str) -> Option<String> {
    let query = query.trim();
    let rest = query.strip_prefix("https://").or_else(|| query.strip_prefix("http://")).unwrap_or(query);
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let labels: Vec<&str> = host.split('.').collect();
    let plain = |label: &&str| !label.is_empty() && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    let is_host = labels.len() >= 2 && labels.iter().all(plain) && labels.last().is_some_and(|tld| tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic()));
    (is_host && !query.contains(' ')).then(|| if rest.len() == query.len() { format!("https://{query}") } else { query.to_string() })
}

// ── Files by name ─────────────────────────────────────────────────────────────

/// Folders nobody looks for a file in.
const SKIPPED: [&str; 5] = ["node_modules", "vendor", "appdata", "$recycle.bin", "__pycache__"];

/// The files and folders under `folders` whose name holds every word of the
/// query, by name. Nothing is indexed or kept: a walk, bounded in depth, in
/// entries and in time, that leaves hidden and system entries alone.
// ponytail: a walk per query, capped at 20 000 entries and 400 ms; the Windows Search index (or a kept index) if folders grow past that.
pub fn files(query: &str, folders: &str) -> Vec<Row> {
    use std::os::windows::fs::MetadataExt;
    let query = fold(query);
    let words: Vec<&str> = query.split(' ').collect();
    if query.chars().count() < 2 {
        return Vec::new();
    }
    let home = notch_core::win::home_dir();
    let mut stack: Vec<(PathBuf, u32)> = folders
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| (l.strip_prefix('~').map_or(PathBuf::from(l), |rest| home.join(rest.trim_start_matches(['\\', '/']))), 0))
        .collect();
    let (began, mut seen, mut found) = (Instant::now(), 0u32, Vec::new());
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            seen += 1;
            if seen > 20_000 || found.len() >= 200 || began.elapsed() > Duration::from_millis(400) {
                stack.clear();
                break;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let Ok(meta) = entry.metadata() else { continue };
            // Hidden (0x2) and system (0x4) entries, and names that begin with a dot.
            if name.starts_with('.') || meta.file_attributes() & 0x6 != 0 {
                continue;
            }
            if meta.is_dir() && depth < 6 && !SKIPPED.contains(&name.to_lowercase().as_str()) {
                stack.push((entry.path(), depth + 1));
            }
            let folded = fold(&name);
            if words.iter().all(|w| folded.contains(w)) {
                found.push((name, entry.path(), meta.is_dir()));
            }
        }
    }
    found.sort();
    found
        .into_iter()
        .take(40)
        .map(|(name, path, is_dir)| {
            let full = path.to_string_lossy().to_string();
            let parent = path.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
            let shown = parent.strip_prefix(&*home.to_string_lossy()).map_or(parent.clone(), |rest| format!("~{rest}"));
            Row { picture: full.clone(), ..row(format!("file:{full}"), "file", name, shown, if is_dir { "folder" } else { "file" }) }
        })
        .collect()
}

// ── The list ──────────────────────────────────────────────────────────────────

/// What the bar knows while it is open.
#[derive(Default)]
pub struct Catalog {
    pub apps: Vec<App>,
    pub windows: Vec<Win>,
    /// The menu of the window that was in front, and that program's name.
    pub menu: Vec<MenuCommand>,
    pub menu_app: String,
    pub links: Vec<Link>,
    pub sep: Separators,
    pub day_first: bool,
}

/// What was asked: the text, the filter, and the rows other features offer for it.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Query {
    pub text: String,
    /// all, apps, windows, actions, clipboard, emoji, settings, files.
    pub chip: String,
    pub extras: Vec<Row>,
}

/// The filter a kind of row is listed under.
fn chip_of(kind: &str) -> &'static str {
    match kind {
        "app" => "apps",
        "window" => "windows",
        "settings" => "settings",
        "emoji" => "emoji",
        "clipboard" => "clipboard",
        "file" => "files",
        _ => "actions",
    }
}

/// How many rows of one kind the mixed list holds.
fn cap(kind: &str) -> usize {
    match kind {
        "app" | "action" | "menu" => 5,
        "emoji" => 6,
        "info" => 2,
        _ => 4,
    }
}

/// Where a row comes from weighs a little: an app before a page of settings with the same name.
fn bias(kind: &str) -> i32 {
    match kind {
        "app" => 80,
        "menu" => -80,
        "file" | "settings" => -40,
        _ => 0,
    }
}

/// The answer to what was typed, when it is a sum, a conversion or a date question.
pub fn answer(text: &str, cat: &Catalog, today: Day) -> Option<Row> {
    let made = |title: String, subtitle: String, copy: String| Row { copy, ..row("answer".into(), "answer", title, subtitle, "calc") };
    if let Some(sum) = math::evaluate(text, cat.sep) {
        let asked = format!("{}{} =", text.trim().trim_end_matches('=').trim(), sum.closing);
        return Some(made(sum.text.clone(), asked, math::reusable(&sum, cat.sep)));
    }
    if let Some(converted) = units::convert(text, cat.sep) {
        return Some(made(converted.clone(), text.trim().to_string(), converted));
    }
    let days = |n: u64| tn("{count} day", "{count} days", n, &[]);
    dates::evaluate(text, today, cat.day_first, days).map(|date| Row { icon: "calendar".into(), ..made(date.text.clone(), date.detail, date.text) })
}

fn own_rows(cat: &Catalog, with_menu: bool) -> Vec<Row> {
    let mut rows = app_rows(&cat.apps, &cat.windows);
    rows.extend(window_rows(&cat.apps, &cat.windows));
    rows.extend(system_rows());
    rows.extend(link_rows(&cat.links));
    rows.extend(os_settings_rows());
    rows.extend(info_rows(cat.sep));
    if with_menu {
        rows.extend(menu_rows(&cat.menu, &cat.menu_app));
    }
    rows
}

/// What an empty bar shows first when nothing has been used yet.
const SUGGESTED: [&str; 5] = ["set:general", "sys:lock", "folder:shell:Downloads", "os:ms-settings:display", "info:battery"];

/// The rows for a query, best first: at most twelve in the mixed list, forty under a filter.
pub fn search(query: &Query, cat: &Catalog, learned: &Learned, now: f64) -> Vec<Row> {
    let typed = query.text.trim();
    let chip = if query.chip.is_empty() { "all" } else { query.chip.as_str() };
    // A colon in front asks for emoji from anywhere.
    let (chip, typed) = match typed.strip_prefix(':') {
        Some(rest) if chip == "all" => ("emoji", rest.trim()),
        _ => (chip, typed),
    };
    let mixed = chip == "all";

    let mut rows = if chip == "emoji" { emoji_rows() } else { own_rows(cat, typed.chars().count() >= 2) };
    rows.extend(query.extras.iter().cloned());
    if !mixed {
        rows.retain(|r| chip_of(&r.kind) == chip);
    }
    let pinned = |id: &str| learned.pins.iter().any(|p| p == id);

    if typed.is_empty() {
        if !mixed {
            // A filter with nothing typed: what is used most, then the rest as it comes.
            let used = learned.most_used(rows.iter().map(|r| r.id.as_str())).into_iter().map(str::to_string).collect::<Vec<_>>();
            rows.sort_by_key(|r| (!pinned(&r.id), used.iter().position(|id| *id == r.id).unwrap_or(usize::MAX)));
            rows.truncate(60);
            return rows;
        }
        return home(rows, learned);
    }

    let mut out = Vec::new();
    if matches!(chip, "all" | "actions") {
        out.extend(answer(typed, cat, Day::today()));
        if let Some(address) = web_address(typed) {
            out.push(row("url".into(), "link", tf("Open {address}", &[("address", &address)]), t("In the browser"), "external"));
        }
        if let Some((link, argument)) = saved_search(&cat.links, typed) {
            out.push(row(format!("link:{}", link.name), "link", format!("{}: {argument}", link.name), destination(link, argument), "search"));
        }
    }

    let order = {
        let candidates: Vec<Candidate> = rows
            .iter()
            .map(|r| Candidate {
                title: &r.title,
                keywords: &r.keywords,
                priority: 0,
                boost: learned.boost(&r.id, now) + bias(&r.kind) + if r.running { 20 } else { 0 } + if pinned(&r.id) { 60 } else { 0 },
            })
            .collect();
        rank(&candidates, typed)
    };
    let limit = if mixed { 12 } else { 40 };
    let mut taken: Vec<(String, usize)> = Vec::new();
    for at in order {
        if out.len() >= limit {
            break;
        }
        let r = &rows[at];
        if out.iter().any(|o: &Row| o.id == r.id) {
            continue;
        }
        if mixed {
            let at = taken.iter().position(|(kind, _)| *kind == r.kind).unwrap_or_else(|| {
                taken.push((r.kind.clone(), 0));
                taken.len() - 1
            });
            if taken[at].1 >= cap(&r.kind) {
                continue;
            }
            taken[at].1 += 1;
        }
        out.push(r.clone());
    }
    out
}

/// Nothing typed: what is pinned, what is used most, the windows that are open.
fn home(rows: Vec<Row>, learned: &Learned) -> Vec<Row> {
    let mut out: Vec<Row> = Vec::new();
    let add = |out: &mut Vec<Row>, r: &Row, section: &str| {
        if out.len() < 14 && !out.iter().any(|o| o.id == r.id) {
            out.push(Row { section: t(section), ..r.clone() });
        }
    };
    for pin in &learned.pins {
        if let Some(r) = rows.iter().find(|r| r.id == *pin) {
            add(&mut out, r, "Pinned");
        }
    }
    let used: Vec<String> = learned.most_used(rows.iter().map(|r| r.id.as_str())).into_iter().take(6).map(str::to_string).collect();
    let suggested = used.iter().map(String::as_str).chain(SUGGESTED).take(6).collect::<Vec<_>>();
    for id in suggested {
        if let Some(r) = rows.iter().find(|r| r.id == id) {
            add(&mut out, r, "Suggested");
        }
    }
    for r in rows.iter().filter(|r| r.kind == "window").take(6) {
        add(&mut out, r, "Open windows");
    }
    out
}
