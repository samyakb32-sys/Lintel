// Notification-area icon. The menu is Open, then the items of the features
// that run (`Manifest.tray`), then Settings, Pause and Quit. It is rebuilt
// whenever a feature is switched on or off and when the language changes.
//
// The icon is the program's own until the island's page sends a picture of its
// mascot (`tray_icon`): the page draws it with the engine the island uses and
// sends a frame only when it differs from the last (src/island/tray-mascot.ts).

use tauri::image::Image;
use tauri::ipc::{InvokeBody, Request};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Wry};

use crate::i18n::{n_, t};
use crate::island::WINDOW_LABEL;
use crate::{registry, winfactory};

/// A feature's item is `feature:<feature id>:<item id>`.
const FEATURE: &str = "feature:";

fn menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let item = |id: &str, label: &str| MenuItem::with_id(app, id, t(label), true, None::<&str>);
    let menu = Menu::new(app)?;
    menu.append(&item("open", n_("Open {app}"))?)?;
    for (feature, id, label) in registry::tray_items(app) {
        menu.append(&item(&format!("{FEATURE}{feature}:{id}"), label)?)?;
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&item("settings", n_("Settings…"))?)?;
    // debt: English only until the next translation task (a plain string, not n_).
    menu.append(&item("putback", "Put the island back")?)?;
    menu.append(&item("pause", n_("Pause"))?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&item("quit", n_("Quit"))?)?;
    Ok(menu)
}

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let mut builder = TrayIconBuilder::with_id(notch_brand::SLUG)
        .tooltip(notch_brand::NAME)
        .menu(&menu(app)?)
        .on_menu_event(|app: &AppHandle, event| match event.id.as_ref() {
            "quit" => app.exit(0),
            "settings" => winfactory::show_settings(app),
            "open" => registry::notify(app, "hub", "tray", "open"),
            id => match id.strip_prefix(FEATURE).and_then(|rest| rest.split_once(':')) {
                Some((feature, item)) => registry::notify(app, feature, "tray", item),
                None => {
                    let _ = app.emit_to(WINDOW_LABEL, "tray", id.to_string());
                }
            },
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }

    builder.build(app)?;
    Ok(())
}

/// The bytes a page sent as the body of a call: as they are, or as the list of
/// numbers the web view's second way of calling makes of them.
pub fn sent_bytes(request: &Request<'_>) -> Vec<u8> {
    match request.body() {
        InvokeBody::Raw(bytes) => bytes.clone(),
        InvokeBody::Json(value) => {
            let numbers = value.as_array().map(|list| list.iter());
            numbers.into_iter().flatten().filter_map(|n| n.as_u64().and_then(|n| u8::try_from(n).ok())).collect()
        }
    }
}

/// A square RGBA picture of 16 to 256 pixels a side; None for anything else.
pub fn picture(rgba: Vec<u8>) -> Option<Image<'static>> {
    let side = ((rgba.len() / 4) as f64).sqrt().round() as usize;
    ((16..=256).contains(&side) && side * side * 4 == rgba.len()).then(|| Image::new_owned(rgba, side as u32, side as u32))
}

/// One frame of the mascot from the island's page, at the size the notification
/// area shows. Answers whether the desktop can be seen: false while Windows is
/// locked or shows a prompt of its own (the cursor cannot be read then,
/// island.rs), and the page stops moving its mascot until someone is back.
#[tauri::command]
pub fn tray_icon(app: AppHandle, request: Request<'_>) -> bool {
    if let (Some(tray), Some(icon)) = (app.tray_by_id(notch_brand::SLUG), picture(sent_bytes(&request))) {
        let _ = tray.set_icon(Some(icon));
    }
    crate::platform::cursor_physical().is_some()
}

/// The menu again, from the features that run now and in the current language.
pub fn rebuild(app: &AppHandle) {
    let Some(tray) = app.tray_by_id(notch_brand::SLUG) else { return };
    match menu(app) {
        Ok(menu) => {
            let _ = tray.set_menu(Some(menu));
        }
        Err(err) => crate::log::line(format!("tray menu: {err}")),
    }
}
