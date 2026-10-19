// Proves the recorder without the app:
// `cargo run -p notch-feat-recorder --example recorder-probe`.
//
// It opens a small window of its own in the bottom right corner (never
// activated, closed by itself), with a block that moves in it, and records
// that window for about four seconds into target/recorder-probe/ with two
// sound tracks: what the computer plays, and a tone made in memory standing in
// for the microphone. The microphone itself is never opened, nothing is
// played, and no key or pointer event is made. Then the file is read back with
// Media Foundation: its length, every stream as Windows describes it, the
// pictures counted and the sound decoded. The file is deleted at the end
// (`--keep` leaves it). A watchdog ends the probe after 40 s whatever happens.
//
//   --region   the display, cut to the rectangle of the probe's window (the path a whole display takes)
//   --small    the Small quality: half the pixels
//   --30       30 pictures a second instead of 60
//   --software Windows' own encoder instead of the graphics card's
//   --still    the block stops after a second: three seconds of a screen that does not change
//   --pause    a pause of a second and a half in the middle, which must not be in the file
//   --close    the window closes in mid-recording: the file must still be whole

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::mpsc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use notch_feat_recorder::rec::{self, Event, Options, Quality, Recording, Sound, Stop, Target, Track, RATE};
use svc_capture::Rect;
use windows::core::{w, Result as WinResult, GUID, HSTRING};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::Graphics::Gdi::{BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, InvalidateRect, PAINTSTRUCT};
use windows::Win32::Media::MediaFoundation::{
    IMFMediaType, MFAudioFormat_PCM, MFCreateMediaType, MFCreateSourceReaderFromURL, MFMediaType_Audio, MFMediaType_Video,
    MFShutdown, MFStartup, MFSTARTUP_FULL, MF_MT_AUDIO_AVG_BYTES_PER_SECOND, MF_MT_AUDIO_NUM_CHANNELS,
    MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_AVG_BITRATE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE,
    MF_PD_DURATION, MF_SOURCE_READERF_ENDOFSTREAM, MF_SOURCE_READER_ALL_STREAMS, MF_SOURCE_READER_MEDIASOURCE, MF_VERSION,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect, GetMessageW, GetWindowRect, PostMessageW, PostQuitMessage,
    RegisterClassW, ShowWindow, SystemParametersInfoW, MSG, SPI_GETWORKAREA, SW_SHOWNOACTIVATE,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, WM_CLOSE, WM_DESTROY, WM_PAINT, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};

const W: i32 = 320;
const H: i32 = 200;

static BEGAN: OnceLock<Instant> = OnceLock::new();
/// The block stops: from then on the window is a still screen.
static STILL: AtomicBool = AtomicBool::new(false);

fn check(what: &str, ok: bool) -> bool {
    println!("  {} {what}", if ok { "ok  " } else { "FAIL" });
    ok
}

// ── The window that is filmed ─────────────────────────────────────────────────

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_PAINT => {
                let mut paint = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut paint);
                let mut all = RECT::default();
                let _ = GetClientRect(hwnd, &mut all);
                // A block that crosses the window every two seconds and changes colour on the way.
                let t = (BEGAN.get_or_init(Instant::now).elapsed().as_millis() % 2000) as i32;
                let x = (all.right - 60) * t / 2000;
                let block = RECT { left: x, top: all.bottom / 2 - 30, right: x + 60, bottom: all.bottom / 2 + 30 };
                for (rect, colour) in [(all, 0x0020_1810u32), (block, 0x0040_80ff + ((t as u32 * 255 / 2000) << 16))] {
                    let brush = CreateSolidBrush(COLORREF(colour));
                    FillRect(dc, &rect, brush);
                    let _ = DeleteObject(brush.into());
                }
                let _ = EndPaint(hwnd, &paint);
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// Opens the window in the bottom right corner of the work area, says which it
/// is and where, and runs it until it is closed. It never takes the keyboard.
fn window(told: mpsc::Sender<(isize, Rect)>) {
    unsafe {
        let instance = GetModuleHandleW(None).expect("module");
        let class = WNDCLASSW { lpfnWndProc: Some(proc), hInstance: instance.into(), lpszClassName: w!("meteor-recorder-probe"), ..Default::default() };
        RegisterClassW(&class);
        let mut work = RECT::default();
        let _ = SystemParametersInfoW(SPI_GETWORKAREA, 0, Some(&mut work as *mut _ as *mut c_void), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
        let hwnd = CreateWindowExW(
            WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            w!("meteor-recorder-probe"),
            w!("recorder probe"),
            WS_POPUP,
            work.right - W - 16,
            work.bottom - H - 16,
            W,
            H,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .expect("the probe's window");
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        // Painted again at every refresh of the screen: at least the 60 pictures a second that are asked for.
        let raw = hwnd.0 as isize;
        std::thread::spawn(move || while DwmFlush().is_ok() && (STILL.load(Relaxed) || InvalidateRect(Some(HWND(raw as *mut c_void)), None, false).as_bool()) {});
        let mut at = RECT::default();
        let _ = GetWindowRect(hwnd, &mut at);
        let _ = told.send((hwnd.0 as isize, Rect { x: at.left, y: at.top, w: at.right - at.left, h: at.bottom - at.top }));
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            DispatchMessageW(&msg);
        }
    }
}

// ── The stand-in for the microphone ───────────────────────────────────────────

/// A 440 Hz tone at a quarter of full scale, handed out at the pace of a real input.
struct Tone {
    began: Instant,
    given: u64,
}

impl Sound for Tone {
    fn rate(&self) -> u32 {
        RATE
    }
    fn channels(&self) -> u16 {
        2
    }
    fn read(&mut self, out: &mut Vec<f32>) -> Result<usize, String> {
        let due = (self.began.elapsed().as_secs_f64() * f64::from(RATE)) as u64;
        let frames = due.saturating_sub(self.given);
        for n in self.given..due {
            let sample = (n as f64 * 440.0 * std::f64::consts::TAU / f64::from(RATE)).sin() as f32 * 0.25;
            out.extend([sample, sample]);
        }
        self.given = due;
        Ok(frames as usize)
    }
}

// ── Reading the file back ─────────────────────────────────────────────────────

/// "H264" from a video subtype, the number of an audio one.
fn code(sub: &GUID) -> String {
    let bytes = sub.data1.to_le_bytes();
    if bytes.iter().all(|b| b.is_ascii_alphanumeric()) { String::from_utf8_lossy(&bytes).into_owned() } else { format!("format 0x{:04x}", sub.data1) }
}

struct Stream {
    video: bool,
    what: String,
    /// Samples as they lie in the file (pictures, for the video).
    samples: u64,
    /// For sound: seconds decoded, and the loudest sample from 0 to 1.
    seconds: f64,
    peak: f32,
}

/// Every sample of one stream; sound is decoded on the way.
unsafe fn stream(path: &Path, index: u32) -> WinResult<Option<Stream>> {
    unsafe {
        let reader = MFCreateSourceReaderFromURL(&HSTRING::from(path.as_os_str()), None)?;
        let Ok(native) = reader.GetNativeMediaType(index, 0) else { return Ok(None) };
        let video = native.GetGUID(&MF_MT_MAJOR_TYPE)? == MFMediaType_Video;
        let name = code(&native.GetGUID(&MF_MT_SUBTYPE)?);
        let what = if video {
            let (size, rate) = (native.GetUINT64(&MF_MT_FRAME_SIZE)?, native.GetUINT64(&MF_MT_FRAME_RATE)?);
            let bits = native.GetUINT32(&MF_MT_AVG_BITRATE).unwrap_or(0);
            format!("video {name} {}x{}, {}/{} pictures a second, {} kbit/s", size >> 32, size as u32, rate >> 32, rate as u32, bits / 1000)
        } else {
            format!(
                "audio {name} {} Hz, {} channels, {} kbit/s",
                native.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND)?,
                native.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS)?,
                native.GetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND).unwrap_or(0) * 8 / 1000
            )
        };
        reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)?;
        reader.SetStreamSelection(index, true)?;
        if !video {
            let plain: IMFMediaType = MFCreateMediaType()?;
            plain.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
            plain.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
            reader.SetCurrentMediaType(index, None, &plain)?;
        }
        let mut out = Stream { video, what, samples: 0, seconds: 0.0, peak: 0.0 };
        loop {
            let (mut flags, mut sample) = (0u32, None);
            reader.ReadSample(index, 0, None, Some(&mut flags), None, Some(&mut sample))?;
            if let Some(sample) = sample {
                out.samples += 1;
                if !video {
                    let buffer = sample.ConvertToContiguousBuffer()?;
                    let (mut data, mut length) = (std::ptr::null_mut(), 0u32);
                    buffer.Lock(&mut data, None, Some(&mut length))?;
                    // 16 bits, two channels: what the decoder gives for this file.
                    let heard = std::slice::from_raw_parts(data.cast::<i16>(), length as usize / 2);
                    out.peak = heard.iter().fold(out.peak, |peak, s| peak.max(f32::from(*s).abs() / 32768.0));
                    out.seconds += heard.len() as f64 / 2.0 / f64::from(RATE);
                    buffer.Unlock()?;
                }
            }
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                return Ok(Some(out));
            }
        }
    }
}

/// The file's length as Windows reads it, and its streams.
unsafe fn read_back(path: &Path) -> WinResult<(f64, Vec<Stream>)> {
    unsafe {
        let reader = MFCreateSourceReaderFromURL(&HSTRING::from(path.as_os_str()), None)?;
        let length = reader.GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_DURATION)?;
        let seconds = length.Anonymous.Anonymous.Anonymous.uhVal as f64 / 1e7;
        let mut streams = Vec::new();
        while let Some(found) = stream(path, streams.len() as u32)? {
            streams.push(found);
        }
        // Media Foundation lists a file's tracks last to first: back into the file's order.
        streams.reverse();
        Ok((seconds, streams))
    }
}

fn main() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(40));
        eprintln!("recorder-probe: still running after 40 s, ended by its watchdog");
        std::process::exit(2);
    });
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let (told, opened) = mpsc::channel();
    let ui = std::thread::spawn(move || window(told));
    let (hwnd, at) = opened.recv().expect("the probe's window");
    std::thread::sleep(Duration::from_millis(300));

    let (region, small) = (has("--region"), has("--small"));
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/recorder-probe");
    std::fs::create_dir_all(&dir).expect("target/recorder-probe");
    let path = dir.join(if region { "probe-region.mp4" } else { "probe-window.mp4" });
    let _ = std::fs::remove_file(&path);
    println!("recorder-probe: {} at {}, {}, {}x{}", if region { "the display, cut to the probe's window" } else { "the probe's own window" }, at.x, at.y, at.w, at.h);

    let fps = if has("--30") { 30 } else { 60 };
    let tone = Track { name: "microphone", open: Box::new(|| Ok(Box::new(Tone { began: Instant::now(), given: 0 }) as Box<dyn Sound>)) };
    let (finished, done) = mpsc::channel();
    let options = Options {
        target: if region { Target::Display(at) } else { Target::Window(hwnd) },
        path: path.clone(),
        fps,
        quality: if small { Quality::Small } else { Quality::Balanced },
        pointer: false,
        border: false,
        software: has("--software"),
        tracks: vec![rec::system_sound(), tone],
    };
    let recording = match Recording::start(options, move |event| match event {
        Event::Silent { track, why } => println!("  the {track} track is silent: {why}"),
        Event::Done(result) => {
            let _ = finished.send(result);
        }
    }) {
        Ok(recording) => recording,
        Err(why) => {
            println!("  FAIL the recording did not start: {why}");
            std::process::exit(1);
        }
    };
    let info = &recording.info;
    let encoder = if info.hardware { format!("the graphics card's encoder ({})", info.vendor) } else { "Windows' own encoder".into() };
    println!("  recording {}x{}, {} kbit/s at most, {encoder}", info.w, info.h, info.bit_rate / 1000);

    let mut expected = 4.0;
    std::thread::sleep(Duration::from_secs(1));
    STILL.store(has("--still"), Relaxed);
    std::thread::sleep(Duration::from_secs(1));
    if has("--pause") {
        recording.pause(true);
        std::thread::sleep(Duration::from_millis(1500));
        println!("  paused: the clock stands at {} ms", recording.elapsed_ms());
        recording.pause(false);
    }
    if has("--close") {
        // The probe's own window, by a message of its own: no input is made.
        unsafe {
            let _ = PostMessageW(Some(HWND(hwnd as *mut c_void)), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        expected = 2.0;
    } else {
        std::thread::sleep(Duration::from_secs(2));
        recording.stop(Stop::User);
    }
    let result = done.recv_timeout(Duration::from_secs(10)).unwrap_or_else(|_| {
        // It should have ended by itself: ended by hand, so that the file can be taken away.
        recording.stop(Stop::User);
        let _ = done.recv_timeout(Duration::from_secs(10));
        Err("the recording did not end by itself".into())
    });
    unsafe {
        let _ = PostMessageW(Some(HWND(hwnd as *mut c_void)), WM_CLOSE, WPARAM(0), LPARAM(0));
    }
    let _ = ui.join();

    let mut ok = true;
    match result {
        Err(why) => ok = check(&format!("the recording was saved ({why})"), false),
        Ok(saved) => {
            println!("  the recorder says: {:.2} s, {} pictures, {} bytes, ended by: {}", saved.seconds, saved.frames, saved.bytes, saved.why.name());
            ok &= check("it ended for the reason it was given", saved.why == if has("--close") { Stop::Closed } else { Stop::User });
            unsafe {
                let _ = MFStartup(MF_VERSION, MFSTARTUP_FULL);
                match read_back(&path) {
                    Err(err) => ok = check(&format!("Windows reads the file ({})", err.message()), false),
                    Ok((seconds, streams)) => {
                        println!("  file: {} bytes, {seconds:.2} s as Windows reads it", std::fs::metadata(&path).map_or(0, |m| m.len()));
                        for (i, s) in streams.iter().enumerate() {
                            let read = if s.video { format!("{} pictures", s.samples) } else { format!("{:.2} s decoded, loudest {:.3}", s.seconds, s.peak) };
                            println!("  track {}: {} -> {read}", i + 1, s.what);
                        }
                        let sounds: Vec<&Stream> = streams.iter().filter(|s| !s.video).collect();
                        ok &= check("one H.264 stream and two AAC streams", streams.len() == 3 && sounds.len() == 2 && streams[0].what.contains("H264") && sounds.iter().all(|s| s.what.contains("0x1610")));
                        let pictures = streams.iter().find(|s| s.video).map_or(0, |s| s.samples);
                        ok &= check(&format!("about {expected} s long"), (seconds - expected).abs() < 0.6);
                        ok &= check("as many pictures in the file as were written", pictures == saved.frames);
                        let rate = pictures as f64 / seconds;
                        if has("--still") {
                            // A second of motion, then one picture a second and the last one again at the end.
                            ok &= check(&format!("a still screen is written about once a second ({pictures} pictures in all)"), (f64::from(fps) * 0.7 + 3.0..=f64::from(fps) + 6.0).contains(&(pictures as f64)));
                        } else {
                            ok &= check(&format!("the block's motion is in it ({rate:.0} pictures a second, {fps} at most)"), (f64::from(fps) * 0.7..=f64::from(fps) + 1.5).contains(&rate));
                        }
                        ok &= check("both sound tracks are as long as the film", sounds.iter().all(|s| (s.seconds - seconds).abs() < 0.25));
                        ok &= check("the stand-in tone is the second sound track, at its level", sounds.get(1).is_some_and(|s| (s.peak - 0.25).abs() < 0.05));
                        println!("  (the first sound track holds what the computer played: {})", if sounds.first().is_some_and(|s| s.peak > 0.001) { "sound" } else { "silence, nothing was playing" });
                    }
                }
                let _ = MFShutdown();
            }
        }
    }
    if !has("--keep") {
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }
    println!("recorder-probe: {}", if ok { "all good" } else { "something FAILED" });
    std::process::exit(if ok { 0 } else { 1 });
}
