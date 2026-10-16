// METEOR V1 runs without a console window: Mascot is the whole UI.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // `lintel.exe --uninstall-hooks [--home <dir>]`: what the uninstaller runs
    // before it removes the relay. No window, no tray, nothing else starts.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--uninstall-hooks") {
        std::process::exit(notch_lib::uninstall_hooks(&args));
    }
    notch_lib::run()
}
