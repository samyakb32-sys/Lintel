// Proof for the media-tools package, on files it makes itself under target/media-tools-probe/.
// Run: cargo run -p notch-feat-media-tools --example media-tools-probe
use notch_feat_media_tools::{convert, gif, img, text_of, video};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use svc_capture::Image;
use windows::core::HSTRING;
use windows::Foundation::TimeSpan;
use windows::Media::Editing::{MediaClip, MediaComposition, MediaTrimmingPreference};
use windows::Media::MediaProperties::{MediaEncodingProfile, VideoEncodingQuality};
use windows::Storage::{CreationCollisionOption, StorageFolder};
use windows::UI::Color;

fn check(ok: bool, what: String) {
    println!("{} {what}", if ok { "ok  " } else { "FAIL" });
    if !ok {
        std::process::exit(1);
    }
}

fn dims(p: &Path) -> (u32, u32) {
    img::size_of_file(p).unwrap()
}

fn main() {
    img::com();
    let dir = std::path::absolute("target/media-tools-probe").unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // ── Pictures ──
    let mut pic = Image { w: 800, h: 400, rgba: vec![0; 800 * 400 * 4] };
    for p in pic.rgba.chunks_exact_mut(4) {
        p[3] = 255;
    }
    img::watermark(&mut pic, "HELLO METEOR", 4, 1.0, 0, 0.8);
    for p in pic.rgba.chunks_exact_mut(4) {
        (p[0], p[1], p[2]) = (255 - p[0], 255 - p[1], 255 - p[2]);
    }
    let src = dir.join("drawn.png");
    std::fs::write(&src, svc_capture::encode_png(&pic).unwrap()).unwrap();
    check(dims(&src) == (800, 400), format!("png drawn {:?}", dims(&src)));

    let jpg = convert(&src, &json!({"imgFormat":"jpeg","imgQuality":0.7,"imgResize":"max","imgMax":400,"imgBg":"white","imgSubfolder":true,"imgPattern":"small {index:02} {width}x{height}"}), 1).unwrap();
    check(jpg.extension().unwrap() == "jpg" && dims(&jpg) == (400, 200), format!("png -> jpeg max side 400: {} {:?} {} bytes", jpg.file_name().unwrap().to_string_lossy(), dims(&jpg), jpg.metadata().unwrap().len()));
    let png = convert(&jpg, &json!({"imgFormat":"png","imgResize":"exact","imgFit":"fill","imgW":100,"imgH":100,"wmText":"meteor"}), 1).unwrap();
    check(dims(&png) == (100, 100), format!("jpeg -> png exact 100x100 fill with watermark: {:?}", dims(&png)));
    let pdf = convert(&src, &json!({"imgFormat":"pdf","imgResize":"none"}), 1).unwrap();
    let bytes = std::fs::read(&pdf).unwrap();
    check(bytes.starts_with(b"%PDF-1.4") && bytes.ends_with(b"%%EOF\n"), format!("pdf written {} bytes", bytes.len()));
    let again = convert(&src, &json!({"imgFormat":"png","imgResize":"none"}), 1).unwrap();
    check(again != src && src.exists(), format!("never over the original: {}", again.file_name().unwrap().to_string_lossy()));

    // ── Text ──
    if svc_capture::ocr_available() {
        let t = text_of(&src).unwrap();
        check(t.to_uppercase().contains("HELLO"), format!("text read: {t:?}"));
    } else {
        println!("skip text: no recogniser language installed");
    }

    // ── Video ──
    let clip = dir.join("clip.mp4");
    let comp = MediaComposition::new().unwrap();
    for (r, g, b) in [(255, 0, 0), (0, 0, 255)] {
        let c = MediaClip::CreateFromColor(Color { A: 255, R: r, G: g, B: b }, TimeSpan { Duration: 10_000_000 }).unwrap();
        comp.Clips().unwrap().Append(&c).unwrap();
    }
    let folder = StorageFolder::GetFolderFromPathAsync(&HSTRING::from(dir.to_string_lossy().as_ref())).unwrap().get().unwrap();
    let file = folder.CreateFileAsync(&HSTRING::from("clip.mp4"), CreationCollisionOption::ReplaceExisting).unwrap().get().unwrap();
    let profile = MediaEncodingProfile::CreateMp4(VideoEncodingQuality::Vga).unwrap();
    comp.RenderToFileWithProfileAsync(&file, MediaTrimmingPreference::Fast, &profile).unwrap().get().unwrap();
    let i = video::info(&clip).unwrap();
    check(i.secs > 1.8 && i.secs < 2.3, format!("generated clip {}x{} {:.2}s {:.0}fps {} bytes", i.w, i.h, i.secs, i.fps, clip.metadata().unwrap().len()));

    let cancel = AtomicBool::new(false);
    let last = Arc::new(std::sync::Mutex::new(0.0f64));
    let l2 = last.clone();
    let o = video::Opts { max_dim: 320, bpp: 0.07, fps: 15.0, keep_audio: false, target_mb: None, start: 0.0, end: 0.0 };
    let out: PathBuf = video::compress(&clip, &o, &cancel, Arc::new(move |f| *l2.lock().unwrap() = f)).unwrap();
    let j = video::info(&out).unwrap();
    check(j.w == 320 && j.h == 240 && out != clip && clip.exists(), format!("compressed -> {} {}x{} {:.2}s {} bytes (progress ended at {:.2})", out.file_name().unwrap().to_string_lossy(), j.w, j.h, j.secs, out.metadata().unwrap().len(), last.lock().unwrap()));
    let o = video::Opts { max_dim: 0, bpp: 0.0, fps: 30.0, keep_audio: false, target_mb: Some(1), start: 0.0, end: 0.0 };
    let out2 = video::compress(&clip, &o, &cancel, Arc::new(|_| {})).unwrap();
    let size2 = out2.metadata().unwrap().len();
    check(size2 <= 1_000_000, format!("size target 1 MB -> {size2} bytes"));
    // A cancelled run leaves nothing behind.
    let stop = AtomicBool::new(true);
    let o = video::Opts { max_dim: 320, bpp: 0.07, fps: 15.0, keep_audio: false, target_mb: None, start: 0.0, end: 0.0 };
    let r = video::compress(&clip, &o, &stop, Arc::new(|_| {}));
    let parts = std::fs::read_dir(&dir).unwrap().filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().ends_with(".part")).count();
    check(parts == 0, format!("cancel: {:?}, .part files left: {parts}", r.err()));

    // ── GIF ──
    let g = gif::make(&clip, &gif::Opts { fps: 5.0, width: 160, loops: true, target_mb: None, start: 0.0, end: 0.0 }, &cancel, Arc::new(|_| {})).unwrap();
    let n = img::frame_count(&g).unwrap();
    let first = img::load_frame(&g, 0, &|w, h| (w, h)).unwrap();
    let lastf = img::load_frame(&g, n - 1, &|w, h| (w, h)).unwrap();
    let (a, b) = (first.pixel(first.w / 2, first.h / 2).unwrap(), lastf.pixel(lastf.w / 2, lastf.h / 2).unwrap());
    check(
        (9..=11).contains(&n) && first.w == 160 && a[0] > 150 && a[2] < 100 && b[2] > 150 && b[0] < 100,
        format!("gif {} frames {}x{} first pixel {a:?} last {b:?} {} bytes", n, first.w, first.h, g.metadata().unwrap().len()),
    );
    let g2 = gif::make(&clip, &gif::Opts { fps: 12.0, width: 320, loops: false, target_mb: None, start: 0.5, end: 1.5 }, &cancel, Arc::new(|_| {})).unwrap();
    check(img::frame_count(&g2).unwrap() == 12, format!("gif trimmed to 1 s at 12 fps: {} frames", img::frame_count(&g2).unwrap()));
    println!("all passed");
}
