// Notification mirroring: the toasts of other apps, through the Windows
// notification listener. Windows gives that to an app only when it has a
// package identity and the user has allowed it, so there are three steps and
// none of them happens by itself: `register` (the identity package, for this
// user), a restart, `request` (Windows asks the user). Until then `missing`
// says which step is next, and nothing listens.
//
// What is read is handed on and forgotten here: never stored, never logged.
//
// Unverified: this half was only compile-checked. No package was registered
// and Windows was never asked while it was written.

use std::path::Path;

use serde::Serialize;
use windows::core::{Ref, Result, HSTRING};
use windows::ApplicationModel::Package;
use windows::Foundation::{TypedEventHandler, Uri};
use windows::Management::Deployment::{PackageManager, RegisterPackageOptions};
use windows::UI::Notifications::Management::{UserNotificationListener, UserNotificationListenerAccessStatus as Access};
use windows::UI::Notifications::{KnownNotificationBindings, UserNotification, UserNotificationChangedEventArgs, UserNotificationChangedKind};
use windows_future::AsyncOperationCompletedHandler;

/// The identity package (a package with external location): no files of the app, only who it is.
const MANIFEST: &str = include_str!("../../../src-tauri/sparse/AppxManifest.xml");
const LOGO: &[u8] = include_bytes!("../../../src-tauri/icons/128x128.png");
/// `Identity Name` and `Publisher` of that manifest.
const PACKAGE: &str = "dev.lintel.app";
const PUBLISHER: &str = "CN=Lintel";

/// A toast as it arrived.
#[derive(Clone, Serialize)]
pub struct Toast {
    pub id: u32,
    pub app: String,
    /// What opens the app that sent it.
    pub aumid: String,
    pub title: String,
    pub body: String,
}

/// This process runs with a package identity.
pub fn has_identity() -> bool {
    Package::Current().is_ok()
}

/// The identity package, when it is registered for this user.
fn package() -> Option<Package> {
    let found = PackageManager::new().ok()?.FindPackagesByUserSecurityIdNamePublisher(&HSTRING::new(), &HSTRING::from(PACKAGE), &HSTRING::from(PUBLISHER));
    found.ok()?.into_iter().next()
}

pub fn registered() -> bool {
    package().is_some()
}

/// What is still missing before notifications can be read, in the order it is
/// asked for: `identity` (no package), `restart` (the package is there, this
/// process started without it), `consent` (Windows has not asked yet),
/// `denied`. `None`: nothing, listen.
pub fn missing() -> Option<&'static str> {
    if !has_identity() {
        return Some(if registered() { "restart" } else { "identity" });
    }
    match UserNotificationListener::Current().and_then(|l| l.GetAccessStatus()) {
        Ok(Access::Allowed) => None,
        Ok(Access::Denied) => Some("denied"),
        _ => Some("consent"),
    }
}

/// Registers the identity package for this user. The manifest is written to
/// `dir` and stays there; the folder of the running program is the package's
/// external location. Nothing is copied or installed elsewhere, and no
/// administrator is needed.
// debt: loose files, which Windows accepts only with Developer Mode on. A release needs the package signed (.msix) and `AddPackageByUriAsync`.
pub fn register(dir: &Path) -> std::result::Result<(), String> {
    let io = |e: std::io::Error| e.to_string();
    let win = |e: windows::core::Error| e.message();
    let manifest = dir.join("AppxManifest.xml");
    std::fs::write(&manifest, MANIFEST).map_err(io)?;
    std::fs::write(dir.join("logo.png"), LOGO).map_err(io)?;
    let exe = std::env::current_exe().map_err(io)?;
    let uri = |path: &Path| Uri::CreateUri(&HSTRING::from(path.as_os_str())).map_err(win);

    let options = RegisterPackageOptions::new().map_err(win)?;
    options.SetExternalLocationUri(&uri(exe.parent().unwrap_or(Path::new(".")))?).map_err(win)?;
    options.SetDeveloperMode(true).map_err(win)?;
    PackageManager::new().map_err(win)?.RegisterPackageByUriAsync(&uri(&manifest)?, &options).map_err(win)?.get().map_err(win)?;
    Ok(())
}

/// Takes the identity package away again. Windows closes a program that runs with the identity.
pub fn unregister() -> std::result::Result<(), String> {
    let win = |e: windows::core::Error| e.message();
    let Some(package) = package() else { return Ok(()) };
    let name = package.Id().and_then(|id| id.FullName()).map_err(win)?;
    PackageManager::new().map_err(win)?.RemovePackageAsync(&name).map_err(win)?.get().map_err(win)?;
    Ok(())
}

/// Has Windows ask the user whether METEOR V1 may read notifications. Call it on
/// the UI thread; `done` hears the answer on another one.
pub fn request(done: impl Fn(bool) + Send + 'static) -> Result<()> {
    let asked = UserNotificationListener::Current()?.RequestAccessAsync()?;
    asked.SetCompleted(&AsyncOperationCompletedHandler::new(move |op, _| {
        done(op.as_ref().and_then(|op| op.GetResults().ok()) == Some(Access::Allowed));
        Ok(())
    }))
}

fn clip(text: &str, max: usize) -> String {
    text.trim().chars().take(max).collect()
}

fn toast(n: &UserNotification) -> Result<Toast> {
    let binding = n.Notification()?.Visual()?.GetBinding(&KnownNotificationBindings::ToastGeneric()?)?;
    let texts: Vec<String> = binding.GetTextElements()?.into_iter().filter_map(|t| t.Text().ok()).map(|t| t.to_string()).filter(|t| !t.trim().is_empty()).collect();
    let app = n.AppInfo().ok();
    Ok(Toast {
        id: n.Id()?,
        app: app.as_ref().and_then(|a| a.DisplayInfo().ok()).and_then(|d| d.DisplayName().ok()).map(|s| s.to_string()).unwrap_or_default(),
        aumid: app.and_then(|a| a.AppUserModelId().ok()).map(|s| s.to_string()).unwrap_or_default(),
        title: clip(texts.first().map_or("", String::as_str), 200),
        body: clip(&texts.iter().skip(1).cloned().collect::<Vec<_>>().join(" "), 1000),
    })
}

/// Dropping it stops listening.
pub struct Listener {
    listener: UserNotificationListener,
    token: i64,
}

impl Drop for Listener {
    fn drop(&mut self) {
        let _ = self.listener.RemoveNotificationChanged(self.token);
    }
}

/// Tells of every toast that arrives from now on. Event-driven: Windows calls, nothing is polled.
pub fn listen(tell: impl Fn(Toast) + Send + 'static) -> Result<Listener> {
    let listener = UserNotificationListener::Current()?;
    let token = listener.NotificationChanged(&TypedEventHandler::new(
        move |from: Ref<UserNotificationListener>, change: Ref<UserNotificationChangedEventArgs>| {
            let (Some(from), Some(change)) = (from.as_ref(), change.as_ref()) else { return Ok(()) };
            if change.ChangeKind()? == UserNotificationChangedKind::Added {
                // A notification without a toast to show (a tile, a badge) is not mirrored.
                if let Ok(toast) = from.GetNotification(change.UserNotificationId()?).and_then(|n| toast(&n)) {
                    tell(toast);
                }
            }
            Ok(())
        },
    ))?;
    Ok(Listener { listener, token })
}
