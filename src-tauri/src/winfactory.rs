// Every window but the island comes from here (docs/ARCHITECTURE.md §6): the
// settings window and the floating windows of features.
//
// Feature windows are created when they are asked for. The base's note that a
// WebView2 window created after launch comes up blank was a thread problem, not
// a timing one: building a webview on the main thread from inside a synchronous
// Tauri command runs inside another webview's message callback, which WebView2
// does not re-enter — the builder never returns and the window stays
// `about:blank`. Built from any other thread (an async command, a blocking
// task) or from `setup`, a late window loads normally. The registry keeps
// feature code off that path, so `open` is safe from `start`, `stop` and `call`.

use notch_core::{Monitor, WindowSpec};
use tauri::utils::config::Color;
use tauri::{AppHandle, Manager, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::{i18n, island, log, platform, Shared};

/// WebView2 allows exactly one browser environment per app, and its options are
/// fixed by whichever webview is created first. Every window therefore asks for
/// the *same* arguments as the island (`additionalBrowserArgs` in tauri.conf.json).
const BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --autoplay-policy=no-user-gesture-required";

/// The page behind a window before its own CSS paints: never a white flash.
const BACKDROP: Color = Color(11, 12, 14, 255);

/// What goes into a window label or the page's query string.
fn plain(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn monitor(app: &AppHandle, which: Monitor) -> Option<tauri::Monitor> {
    let pref = match which {
        Monitor::Island => app.state::<Shared>().settings.lock().unwrap().screen.clone(),
        Monitor::Cursor => "cursor".to_string(),
        Monitor::Primary => "primary".to_string(),
    };
    island::target_monitor(app, &pref)
}

/// Creates the window, or shows it again when its label already exists.
pub fn open(app: &AppHandle, spec: WindowSpec) -> Result<WebviewWindow, String> {
    if let Some(win) = app.get_webview_window(&spec.label) {
        let _ = win.unminimize();
        if spec.no_activate {
            platform::show_inactive(&win);
        } else {
            let _ = win.show();
            let _ = win.set_focus();
        }
        return Ok(win);
    }
    if !(plain(&spec.label) && plain(&spec.feature) && plain(&spec.page)) {
        return Err(format!("window {:?}: label, feature and page are letters, digits, - and _", spec.label));
    }
    // `transparent` tells the page to leave its background out.
    let see_through = if spec.transparent { "&transparent=1" } else { "" };
    let url = WebviewUrl::App(format!("window.html?feature={}&page={}{see_through}", spec.feature, spec.page).into());
    let mut builder = WebviewWindowBuilder::new(app, &spec.label, url)
        .additional_browser_args(BROWSER_ARGS)
        .title(notch_brand::NAME)
        .inner_size(spec.w, spec.h)
        .decorations(!spec.frameless)
        .transparent(spec.transparent)
        .shadow(!spec.transparent)
        .always_on_top(spec.topmost)
        .content_protected(spec.exclude_from_capture)
        .skip_taskbar(spec.no_activate)
        .focused(!spec.no_activate)
        .focusable(!spec.no_activate)
        // Shown once it is in place.
        .visible(false);
    if !spec.transparent {
        builder = builder.background_color(BACKDROP);
    }
    let win = builder.build().map_err(|e| format!("window {}: {e}", spec.label))?;
    // Its taskbar button wears the mascot in the product's colour, when that is not the program's own.
    crate::brand_icon::dress(app, &win);
    if spec.no_activate {
        platform::make_non_activating(&win);
    }
    // Centred on its display, in physical pixels: displays may differ in scale.
    if let (Some(m), Ok(size)) = (monitor(app, spec.monitor), win.outer_size()) {
        let (origin, area) = (m.position(), m.size());
        let x = origin.x + (area.width as i32 - size.width as i32) / 2;
        let y = origin.y + (area.height as i32 - size.height as i32) / 2;
        let _ = win.set_position(PhysicalPosition::new(x, y));
    }
    if spec.no_activate {
        platform::show_inactive(&win);
    } else {
        let _ = win.show();
    }
    log::line(format!("window {} opened ({}/{})", spec.label, spec.feature, spec.page));
    Ok(win)
}

pub fn close(app: &AppHandle, label: &str) {
    if label == island::WINDOW_LABEL || label == SETTINGS {
        return;
    }
    if let Some(win) = app.get_webview_window(label) {
        let _ = win.destroy();
    }
}

/// A feature was switched off: none of its windows stays.
pub fn close_all_of(app: &AppHandle, feature: &str) {
    for (label, win) in app.webview_windows() {
        let Ok(url) = win.url() else { continue };
        let its = url.path().ends_with("/window.html") && url.query_pairs().any(|(k, v)| k == "feature" && v == feature);
        if its {
            close(app, &label);
        }
    }
}

// ── Settings window ───────────────────────────────────────────────────────────

const SETTINGS: &str = "settings";

/// Created hidden at launch and only ever shown and hidden afterwards, so it
/// opens at once and keeps its place.
pub fn create_settings(app: &AppHandle) {
    match WebviewWindowBuilder::new(app, SETTINGS, WebviewUrl::App("settings.html".into()))
        .additional_browser_args(BROWSER_ARGS)
        .title(i18n::t("Settings — {app}"))
        .inner_size(760.0, 620.0)
        .min_inner_size(640.0, 480.0)
        .background_color(BACKDROP)
        .resizable(true)
        .visible(false)
        // Built hidden at launch: it must not take the keyboard from whatever
        // the user is typing in (it did, invisibly). `show_settings` gives it
        // the focus when it is asked for.
        .focused(false)
        .center()
        .build()
    {
        Ok(win) => {
            // Closing it must only hide it, or it could never be reopened.
            let hidden = win.clone();
            win.on_window_event(move |event| {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    platform::settings_closing(&hidden);
                    let _ = hidden.hide();
                }
            });
        }
        Err(err) => log::line(format!("settings window failed: {err}")),
    }
}

pub fn show_settings(app: &AppHandle) {
    let Some(win) = app.get_webview_window(SETTINGS) else {
        log::line("settings window missing");
        return;
    };
    platform::settings_opening(&win);
    let _ = win.unminimize();
    let _ = win.show();
    let _ = win.set_focus();
}

/// Shows Settings on one of its pages (`#/<page>`, the router of src/settings/main.ts). A page id is
/// letters, digits and dashes; anything else only shows the window.
pub fn show_settings_page(app: &AppHandle, page: &str) {
    show_settings(app);
    if page.is_empty() || !page.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return;
    }
    if let Some(win) = app.get_webview_window(SETTINGS) {
        let _ = win.eval(&format!("location.hash = '#/{page}'"));
    }
}

/// The title follows the language.
pub fn retitle_settings(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(SETTINGS) {
        let _ = window.set_title(&i18n::t("Settings — {app}"));
    }
}
