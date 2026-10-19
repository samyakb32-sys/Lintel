// misc-late-probe — proves what can be proved without anybody's keyboard,
// registry or Explorer:
//   1. backup: export of a made-up settings file has no secret, import shows a
//      plan, applying it round-trips, a dated copy is kept;
//   2. cleaning: the unlock counter over made-up key events, the time limit
//      arithmetic, and a live run whose limit (1.5 s) ends it by itself, hooks gone;
//   3. watch: headline and number parsing, the tracker's conditions, and a
//      number drawn here into a picture and read back by Windows' recogniser;
//   4. terminal tab: the chooser over a made-up list of tabs;
//   5. reset: plan and apply on made-up folders.
// Files go under target/misc-late-probe/. Nothing is sent to any window, no
// sound is played, the real settings, registry and Explorer are not touched.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering::SeqCst};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use notch_feat_misc_late::{backup, clean, reset, tabs, watch};
use serde_json::json;

fn ok(name: &str, pass: bool) {
    println!("{} {name}", if pass { "ok  " } else { "FAIL" });
    if !pass {
        std::process::exit(1);
    }
}

fn dir() -> PathBuf {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/misc-late-probe");
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.canonicalize().unwrap()
}

fn backup_round_trip(d: &PathBuf) {
    println!("-- backup");
    let original = json!({
        "soundVolume": 0.4, "language": "fr", "autostart": true, "hooksInstalled": true,
        "shortcuts": { "agents:open": { "key": "T", "ctrl": true } },
        "features": {
            "clipboard": { "max": 50, "apiToken": "abc", "nested": { "client_secret": "x", "ok": 1 } },
            "weather": { "city": "Paris", "provider": "sk-live-0123456789abcdef" },
        },
        "enabled": { "clipboard": true, "weather": false },
    });
    let settings = d.join("settings.json");
    std::fs::write(&settings, serde_json::to_string_pretty(&original).unwrap()).unwrap();
    let backup_file = d.join("export.json");
    backup::export_file(&settings, &backup_file, "0.3.0", "20261011-000000").unwrap();
    let text = std::fs::read_to_string(&backup_file).unwrap();
    ok("export has no apiToken, client_secret or key-shaped value", !text.contains("apiToken") && !text.contains("client_secret") && !text.contains("sk-live"));
    ok("export has no machine state", !text.contains("autostart") && !text.contains("hooksInstalled"));
    ok("export keeps the choices (incl. the shortcut's own `key`)", text.contains("\"fr\"") && text.contains("\"city\": \"Paris\"") && text.contains("\"key\": \"T\""));

    // The user changes things, then imports the backup.
    let mut changed = original.clone();
    changed["language"] = json!("en");
    changed["soundVolume"] = json!(0.9);
    changed["features"]["clipboard"]["max"] = json!(10);
    changed["features"]["weather"] = json!({});
    changed["enabled"]["weather"] = json!(true);
    // A secret that a hand-edited backup carries: must be left out of the plan.
    let mut file = backup::read_json(&backup_file).unwrap();
    file["settings"]["features"]["clipboard"]["apiToken"] = json!("planted");
    file["settings"]["unknownKey"] = json!(1);
    file["settings"]["soundVolume"] = json!("loud");
    let plan = backup::plan(&changed, &file).unwrap();
    println!("   plan: {} changes, {} left out", plan.changes.len(), plan.skipped);
    ok("plan lists the 5 differences", plan.changes.len() == 5);
    ok("plan leaves out the planted token, the unknown key and the mistyped value", plan.skipped >= 3);
    ok("plan never carries a token", !serde_json::to_string(&plan.features).unwrap().contains("planted"));
    let copy = backup::keep_copy(&settings, &d.join("backups"), "20261011-000001").unwrap();
    ok("a dated copy of the settings it replaces exists", copy.exists());
    let mut after = changed.clone();
    backup::apply_to(&mut after, &plan);
    // The mistyped soundVolume is kept as the user's (0.9); everything else is back.
    let mut want = original.clone();
    want["soundVolume"] = json!(0.9);
    for k in ["autostart", "hooksInstalled"] {
        after.as_object_mut().unwrap().remove(k);
        want.as_object_mut().unwrap().remove(k);
    }
    // The secrets the export never held are not restored either.
    want["features"]["clipboard"].as_object_mut().unwrap().remove("apiToken");
    want["features"]["clipboard"]["nested"].as_object_mut().unwrap().remove("client_secret");
    want["features"]["weather"].as_object_mut().unwrap().remove("provider");
    after["features"]["clipboard"].as_object_mut().unwrap().remove("apiToken");
    after["features"]["clipboard"]["nested"].as_object_mut().unwrap().remove("client_secret");
    after["features"]["weather"].as_object_mut().unwrap().remove("provider");
    ok("applying the plan brings the choices back", after == want);
    ok("a file from a newer version is refused", backup::plan(&changed, &json!({ "formatVersion": 99, "settings": {} })).is_err());
    ok("a file that is not a backup is refused", backup::plan(&changed, &json!({ "hello": 1 })).is_err());
}

fn cleaning() {
    println!("-- cleaning mode");
    let esc = clean::VK_ESCAPE;
    let press = |c: &mut clean::Counter, vk: u16, t: u64| {
        let s = c.key(vk, true, t);
        c.key(vk, false, t + 50);
        s
    };
    let mut c = clean::Counter::default();
    let steps: Vec<_> = (0..5).map(|i| press(&mut c, esc, i * 1000)).collect();
    ok("five Esc presses unlock", steps == [
        clean::Step::Progress(1), clean::Step::Progress(2), clean::Step::Progress(3), clean::Step::Progress(4), clean::Step::Unlocked,
    ]);
    let mut c = clean::Counter::default();
    press(&mut c, esc, 0);
    press(&mut c, esc, 1000);
    ok("another key resets the count", press(&mut c, 0x41, 2000) == clean::Step::Reset);
    ok("and the count starts again at one", press(&mut c, esc, 3000) == clean::Step::Progress(1));
    let mut c = clean::Counter::default();
    press(&mut c, esc, 0);
    ok("a pause over six seconds starts again", press(&mut c, esc, 7000) == clean::Step::Progress(1));
    let mut c = clean::Counter::default();
    press(&mut c, esc, 0);
    ok("a pause of six seconds still counts", press(&mut c, esc, 6000) == clean::Step::Progress(2));
    let mut c = clean::Counter::default();
    let first = c.key(esc, true, 0);
    let repeats: Vec<_> = (1..10).map(|i| c.key(esc, true, i * 30)).collect();
    ok("auto-repeat counts once", first == clean::Step::Progress(1) && repeats.iter().all(|s| *s == clean::Step::Nothing));
    let mut c = clean::Counter::default();
    press(&mut c, esc, 0);
    ok("a modifier resets the count too", press(&mut c, 0xA0, 100) == clean::Step::Reset);
    ok("limit: 120 s default counts down", clean::remaining(120, 0) == 120 && clean::remaining(120, 61_000) == 59 && clean::remaining(120, 120_000) == 0 && clean::remaining(120, 999_000) == 0);
    ok("limit is kept between 10 s and 10 min", clean::clamp_secs(0) == 10 && clean::clamp_secs(120) == 120 && clean::clamp_secs(99_999) == 600);

    // A live run: the lock ends by itself after 1.5 s, and the hooks go with it.
    let updates = Arc::new(AtomicU32::new(0));
    let ended = Arc::new(Mutex::new(None));
    let (u, e) = (updates.clone(), ended.clone());
    let began = Instant::now();
    let run = clean::start_ms(1500, move |_| { u.fetch_add(1, SeqCst); }, move |why| *e.lock().unwrap() = Some(why));
    std::thread::sleep(Duration::from_millis(300));
    let st = svc_input::status();
    ok("while locked, the keyboard and the wheel are hooked", st.keyboard && st.mouse);
    while ended.lock().unwrap().is_none() && began.elapsed() < Duration::from_secs(4) {
        std::thread::sleep(Duration::from_millis(50));
    }
    let took = began.elapsed();
    println!("   ended after {took:?}, {} updates", updates.load(SeqCst));
    ok("the time limit ends it by itself", *ended.lock().unwrap() == Some(clean::Ended::TimedOut) && took < Duration::from_millis(2500));
    ok("it was told the count at least twice", updates.load(SeqCst) >= 2);
    run.stop();
    std::thread::sleep(Duration::from_millis(150));
    let st = svc_input::status();
    ok("afterwards no hook is left", !st.keyboard && !st.mouse);
    // Stop by hand.
    let run = clean::start_ms(5000, |_| {}, |_| {});
    std::thread::sleep(Duration::from_millis(200));
    let t = Instant::now();
    run.stop();
    ok("stop removes the hooks at once", t.elapsed() < Duration::from_millis(500) && !svc_input::status().keyboard);
}

/// "CPU 73%" drawn black on white, 48 px, as pixels.
fn drawn(text: &str) -> svc_capture::Image {
    use windows::core::PCWSTR;
    use windows::Win32::Graphics::Gdi::*;
    unsafe {
        let (w, h) = (520i32, 120i32);
        let dc = CreateCompatibleDC(None);
        let mut info = BITMAPINFO::default();
        info.bmiHeader = BITMAPINFOHEADER { biSize: size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, ..Default::default() };
        let mut bits = std::ptr::null_mut();
        let bmp = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0).unwrap();
        let old = SelectObject(dc, bmp.into());
        let all = std::slice::from_raw_parts_mut(bits as *mut u8, (w * h * 4) as usize);
        all.fill(255);
        let name: Vec<u16> = "Segoe UI\0".encode_utf16().collect();
        let font = CreateFontW(64, 0, 0, 0, 400, 0, 0, 0, FONT_CHARSET(1), FONT_OUTPUT_PRECISION(0), FONT_CLIP_PRECISION(0), FONT_QUALITY(5), 0, PCWSTR(name.as_ptr()));
        let oldf = SelectObject(dc, font.into());
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, windows::Win32::Foundation::COLORREF(0));
        let wide: Vec<u16> = text.encode_utf16().collect();
        let _ = TextOutW(dc, 20, 20, &wide);
        let _ = GdiFlush();
        let mut rgba = Vec::with_capacity(all.len());
        for px in all.chunks(4) {
            rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
        }
        SelectObject(dc, oldf);
        SelectObject(dc, old);
        let _ = DeleteObject(font.into());
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(dc);
        svc_capture::Image { w: w as u32, h: h as u32, rgba }
    }
}

fn watching() {
    println!("-- watch");
    ok("headline: percentage first", watch::headline("CPU\nUsage 73 %\n12:30").as_deref() == Some("73%"));
    ok("headline: a clock", watch::headline("Time left\n12:30:45 remaining").as_deref() == Some("12:30:45"));
    ok("headline: number with unit", watch::headline("Downloaded 12.5 MB of 40").as_deref() == Some("12.5 MB"));
    ok("headline: first words, 22 characters", watch::headline("  Build finished successfully today\nx").as_deref() == Some("Build finished success"));
    ok("number: 1,234.5", watch::parse_number("1,234.5 MB") == Some(1234.5));
    ok("number: 1.234,5", watch::parse_number("1.234,5") == Some(1234.5));
    ok("number: 12,5", watch::parse_number("12,5%") == Some(12.5));
    ok("number: 1,234 is a thousand", watch::parse_number("1,234") == Some(1234.0));
    let mut t = watch::Tracker::new(watch::Cond::Reaches(80.0));
    ok("reaches, from below", !t.feed("73%", "", 0) && !t.feed("79%", "", 1) && t.feed("80%", "", 2));
    let mut t = watch::Tracker::new(watch::Cond::Reaches(10.0));
    ok("reaches, from above", !t.feed("50", "", 0) && t.feed("9", "", 1));
    let mut t = watch::Tracker::new(watch::Cond::Changes);
    ok("changes: only after three seconds", !t.feed("5", "", 0) && !t.feed("6", "", 1000) && !t.feed("6", "", 3000) && t.feed("6", "", 4100));
    let mut t = watch::Tracker::new(watch::Cond::Changes);
    ok("changes: a blip does not count", !t.feed("5", "", 0) && !t.feed("6", "", 1000) && !t.feed("5", "", 2000) && !t.feed("5", "", 9000));
    let mut t = watch::Tracker::new(watch::Cond::Contains("Done".into()));
    ok("contains, any case", !t.feed("", "building", 0) && t.feed("", "Build DONE", 1));

    if !svc_capture::ocr_available() {
        println!("skip: Windows has no text recogniser for the user's languages, so the picture was not read");
        return;
    }
    // (Windows' recogniser reads a bare "73%" as nothing; a line with a word in it is what a screen has.)
    let image = drawn("Downloaded 73%");
    let text = watch::read_text(&image).unwrap();
    let head = watch::headline(&text);
    println!("   read {text:?} -> {head:?}");
    ok("a number drawn into a picture is read back as the headline", head.as_deref() == Some("73%"));
    let print = watch::fingerprint(&image);
    ok("an unchanged picture has the same fingerprint, a different one does not", print == watch::fingerprint(&drawn("Downloaded 73%")) && print != watch::fingerprint(&drawn("CPU 99% CPU 99%")));
}

fn terminal_tabs() {
    println!("-- terminal tab");
    let tabs_ = ["PowerShell".to_string(), "claude  meteor".to_string(), "~/projects/api-server".to_string(), "Command Prompt".to_string(), "npm run dev - meteor".to_string()];
    let pick = |cwd: &str, title: &str| tabs::choose(&tabs_, &tabs::Hints { cwd: cwd.into(), title: title.into() });
    ok("the first tab naming the folder", pick(r"C:\Users\me\code\meteor", "") == Some(1));
    ok("another folder", pick(r"C:\Users\me\code\api-server\", "") == Some(2));
    ok("the whole path beats the folder name", pick("C:\\x", "") == None);
    ok("the session's title when the folder is in no tab", pick(r"C:\Users\me\other", "Command") == Some(3));
    ok("a tab that is nobody's is left alone", pick(r"C:\Users\me\elsewhere", "zzz").is_none());
    ok("a too-short name matches nothing", pick(r"C:\a", "x").is_none());
    let two = ["a - meteor".to_string(), "C:\\Users\\me\\meteor".to_string()];
    ok("path in a title wins over the folder in an earlier one", tabs::choose(&two, &tabs::Hints { cwd: r"C:\Users\me\meteor".into(), title: String::new() }) == Some(1));
    ok("folder name of a path", tabs::folder_name(r"C:\a\b\") == "b" && tabs::folder_name("/x/y") == "y");
}

fn resetting(d: &PathBuf) {
    println!("-- reset");
    let t = reset::Targets {
        run_value: "meteor-misc-late-probe-not-there".into(),
        secrets: vec!["a-key".into(), "b-key".into()],
        data: vec![("features", d.join("local/features")), ("inbox", d.join("local/inbox"))],
        relay: d.join("local/bin"),
        config: d.join("config"),
    };
    for p in ["local/features/clipboard", "local/inbox", "local/bin", "config"] {
        std::fs::create_dir_all(d.join(p)).unwrap();
        std::fs::write(d.join(p).join("x.txt"), "hello").unwrap();
    }
    let plan = reset::plan(&t, true, |k| k == "a-key");
    let items = plan["items"].as_array().unwrap();
    let get = |id: &str| items.iter().find(|i| i["id"] == id).unwrap().clone();
    ok("plan: the startup entry is not there, the key is, the folders are", get("startup")["present"] == false && get("secrets")["count"] == 1 && get("features")["bytes"] == 5 && get("hooks")["present"] == true);
    ok("plan: a reset does not list the settings folder", reset::plan(&t, false, |_| false)["items"].as_array().unwrap().iter().all(|i| i["id"] != "config"));
    ok("plan changed nothing", d.join("local/features/clipboard/x.txt").exists() && d.join("config/x.txt").exists());
    let cleared = std::cell::RefCell::new(Vec::new());
    let done = reset::apply(&t, &["features".into(), "secrets".into(), "hooks".into(), "startup".into()], |k| { cleared.borrow_mut().push(k.to_string()); Ok(()) }, || Ok(()));
    println!("   {done}");
    ok("apply removes what was ticked", !d.join("local/features").exists() && done["features"] == "removed" && cleared.borrow().len() == 2 && done["startup"] == "nothing");
    ok("hooks: the relay goes only with their entries", !d.join("local/bin").exists() && done["hooks"] == "removed");
    ok("apply leaves what was not ticked", d.join("local/inbox/x.txt").exists() && d.join("config/x.txt").exists());
    std::fs::create_dir_all(d.join("local/bin")).unwrap();
    let kept = reset::apply(&t, &["hooks".into()], |_| Ok(()), || Err("refused".into()));
    ok("a failed hook removal keeps the relay and says so", d.join("local/bin").exists() && kept["hooks"]["error"] == "refused");
}

fn main() {
    let d = dir();
    println!("files under {}", d.display());
    backup_round_trip(&d);
    cleaning();
    watching();
    terminal_tabs();
    resetting(&d);
    println!("all done");
}
