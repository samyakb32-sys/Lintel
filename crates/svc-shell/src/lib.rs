// Shell — a shared service (docs/ARCHITECTURE.md §8): what Explorer can do
// with a file, for any feature. Thumbnails and icons as PNG, open, open with,
// reveal, the clipboard, an OLE drag out of the app, the Share sheet, zip;
// what a drop or the clipboard brings in (incoming.rs); a new name, the
// Recycle Bin and the file picker (fileops.rs); an entry in Explorer's own
// menus (menus.rs).
//
// Nothing here runs on its own: every function works on the caller's time and
// leaves no thread behind. The only thing kept is the image cache.

mod fileops;
mod incoming;
mod menus;

pub use fileops::{name_problem, pick_files, pick_folder, recycle, rename, NameProblem, Recycled};
pub use incoming::{clipboard, free_name, incoming, safe_name};
pub use menus::{add_to_menus, menu_command, remove_from_menus, shortcut_target, MenuEntry, Places};

use std::collections::HashMap;
use std::ffi::c_void;
use std::mem::{size_of, ManuallyDrop};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{LazyLock, Mutex};
use std::time::UNIX_EPOCH;

use windows::core::{factory, w, IInspectable, Interface, Result as WinResult, HRESULT, HSTRING, PCWSTR, PWSTR};
use windows::ApplicationModel::DataTransfer::{
    DataPackage, DataRequestedEventArgs, DataTransferManager, ShareCompletedEventArgs,
};
use windows::Foundation::{TypedEventHandler, Uri};
use windows::Storage::{IStorageItem, StorageFile};
use windows::Win32::Foundation::{COLORREF, E_FAIL, HGLOBAL, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::Graphics::Gdi::{
    DeleteObject, GetDC, GetDIBits, GetObjectW, MonitorFromPoint, ReleaseDC, BITMAP, BITMAPINFO, BITMAPINFOHEADER,
    DIB_RGB_COLORS, HBITMAP, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_ContainerFormatPng, GUID_WICPixelFormat32bppBGRA, IWICImagingFactory,
    WICBitmapEncoderNoCache,
};
use windows::Win32::System::Com::StructuredStorage::{CreateStreamOnHGlobal, GetHGlobalFromStream};
use windows::Win32::System::Com::{
    CoCreateInstance, CoTaskMemFree, IDataObject, CLSCTX_INPROC_SERVER, DVASPECT_CONTENT, FORMATETC, STATFLAG_NONAME,
    STATSTG, STGMEDIUM, STGMEDIUM_0, TYMED_HGLOBAL,
};
use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::{
    OleFlushClipboard, OleInitialize, OleSetClipboard, OleUninitialize, CF_UNICODETEXT, DROPEFFECT_COPY,
    DROPEFFECT_LINK, DROPEFFECT_MOVE,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_MOVE, MOUSEINPUT, VK_LBUTTON, VK_RBUTTON,
};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    BHID_DataObject, CLSID_DragDropHelper, IAssocHandler, IDataTransferManagerInterop, IDragSourceHelper,
    ILCreateFromPathW, ILFree, IShellItemImageFactory, SHAssocEnumHandlers, SHCreateDataObject,
    SHCreateItemFromParsingName, SHCreateShellItemArrayFromIDLists, SHDoDragDrop, SHOpenFolderAndSelectItems,
    SHOpenWithDialog, ShellExecuteW, ASSOC_FILTER_RECOMMENDED, OAIF_EXEC, OAIF_HIDE_REGISTRATION, OPENASINFO,
    SHDRAGIMAGE, SIIGBF, SIIGBF_ICONONLY, SIIGBF_RESIZETOFIT, SIIGBF_THUMBNAILONLY,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetForegroundWindow, GetSystemMetrics, GetWindowLongW,
    GetWindowRect, GetWindowThreadProcessId, IsIconic, IsWindowVisible, PostMessageW, RegisterClassW,
    SetForegroundWindow, SetLayeredWindowAttributes, SetTimer, ShowWindow, GWL_EXSTYLE, LWA_ALPHA, SM_CXSCREEN,
    SM_CYSCREEN, SM_SWAPBUTTON, SW_SHOW, SW_SHOWNORMAL,
    WM_ACTIVATE, WM_APP, WM_CLOSE, WM_TIMER, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};
use windows_collections::IIterable;

/// Something that leaves the app: dragged out, shared or copied.
#[derive(Debug, Clone)]
pub enum Item {
    File(PathBuf),
    Text(String),
    /// A web address.
    Link(String),
}

/// What the receiver of a drag did with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropEffect {
    None,
    Copy,
    Move,
    Link,
}

/// An app that can open a file: an entry of "Open with".
#[derive(Debug, Clone)]
pub struct Handler {
    /// As Explorer shows it, and what `open_with` takes.
    pub name: String,
    /// The program behind it (a path, or an app id for a packaged app).
    pub exe: String,
}

/// The path as the shell takes it: absolute, with backslashes, without the
/// `\\?\` of a canonical path. Written any other way ("C:/Users/me/a.jpg") the
/// shell's parser refuses it ("The parameter is incorrect"), and the file gets
/// neither its picture nor its icon.
pub fn shell_path(path: &Path) -> PathBuf {
    let full = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    match full.to_str().and_then(|text| text.strip_prefix(r"\\?\")) {
        Some(rest) if !rest.starts_with(r"UNC\") => PathBuf::from(rest),
        _ => full,
    }
}

pub(crate) fn wide(path: &Path) -> HSTRING {
    HSTRING::from(shell_path(path).as_os_str())
}

pub(crate) fn said(err: windows::core::Error) -> String {
    let text = err.message();
    if text.is_empty() { format!("Windows error {:#010x}", err.code().0) } else { text }
}

/// Shell objects want a single-threaded apartment with OLE up, and callers
/// come from thread pools in any state: the work gets a thread of its own.
pub(crate) fn sta<T: Send>(work: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|scope| {
        scope
            .spawn(|| unsafe {
                let _ = OleInitialize(None);
                let out = work();
                OleUninitialize();
                out
            })
            .join()
            .expect("shell worker")
    })
}

// ── Thumbnails and icons ──────────────────────────────────────────────────────

type ImageKey = (String, u32, u32, u64);

static IMAGES: LazyLock<Mutex<HashMap<ImageKey, Vec<u8>>>> = LazyLock::new(Default::default);

/// The file's own preview (a picture, a video frame, a PDF page) as PNG, at
/// most `px` on its longer side. An error when the file has none: use `icon`.
pub fn thumbnail(path: &Path, px: u32) -> Result<Vec<u8>, String> {
    image(&shell_path(path).to_string_lossy(), px, SIIGBF_THUMBNAILONLY)
}

/// The icon of a file, a folder, a program or an installed app (by its
/// AppUserModelID) as PNG, `px` square. Cached by path and modification time.
pub fn icon(target: &str, px: u32) -> Result<Vec<u8>, String> {
    if Path::new(target).exists() {
        image(&shell_path(Path::new(target)).to_string_lossy(), px, SIIGBF_ICONONLY)
    } else {
        image(&format!("shell:AppsFolder\\{target}"), px, SIIGBF_ICONONLY)
    }
}

/// "The data necessary to complete this operation is not yet available."
const E_PENDING: HRESULT = HRESULT(0x8000_000A_u32 as i32);
/// "The process cannot access the file because it is being used by another process."
const E_SHARING: HRESULT = HRESULT(0x8007_0020_u32 as i32);

fn image(name: &str, px: u32, flags: SIIGBF) -> Result<Vec<u8>, String> {
    let changed = std::fs::metadata(name)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    let key = (name.to_lowercase(), px, flags.0 as u32, changed);
    if let Some(hit) = IMAGES.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return Ok(hit.clone());
    }
    let png = sta(|| unsafe {
        // "Not yet available" is the shell still fetching it (the same icon asked
        // for twice at once), and a file that has just landed is often still held
        // by whoever wrote it or by the virus scanner: both pass in a moment.
        let mut tries = 0;
        let bitmap = loop {
            match shell_bitmap(name, px, flags) {
                Err(err) if (err.code() == E_PENDING || err.code() == E_SHARING) && tries < 10 => {
                    tries += 1;
                    std::thread::sleep(std::time::Duration::from_millis(60));
                }
                other => break other?,
            }
        };
        let png = png_of(bitmap);
        let _ = DeleteObject(bitmap.into());
        png
    })
    .map_err(said)?;
    let mut cache = IMAGES.lock().unwrap_or_else(|e| e.into_inner());
    // ponytail: the cache is emptied when it is full; an LRU if that ever shows.
    if cache.len() >= 256 {
        cache.clear();
    }
    cache.insert(key, png.clone());
    Ok(png)
}

/// A 32-bit bitmap with premultiplied alpha, to be freed with `DeleteObject`.
unsafe fn shell_bitmap(name: &str, px: u32, flags: SIIGBF) -> WinResult<HBITMAP> {
    unsafe {
        let item: IShellItemImageFactory = SHCreateItemFromParsingName(&HSTRING::from(name), None)?;
        item.GetImage(SIZE { cx: px as i32, cy: px as i32 }, flags)
    }
}

unsafe fn bitmap_size(bitmap: HBITMAP) -> (i32, i32) {
    let mut info = BITMAP::default();
    unsafe { GetObjectW(bitmap.into(), size_of::<BITMAP>() as i32, Some(&mut info as *mut _ as *mut c_void)) };
    (info.bmWidth, info.bmHeight)
}

unsafe fn png_of(bitmap: HBITMAP) -> WinResult<Vec<u8>> {
    unsafe {
        let (w, h) = bitmap_size(bitmap);
        if w <= 0 || h <= 0 {
            return Err(E_FAIL.into());
        }
        // Top-down BGRA, whatever way up the shell made it.
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        let dc = GetDC(None);
        let lines = GetDIBits(dc, bitmap, 0, h as u32, Some(pixels.as_mut_ptr().cast()), &mut info, DIB_RGB_COLORS);
        ReleaseDC(None, dc);
        if lines == 0 {
            return Err(E_FAIL.into());
        }
        // The shell premultiplies; PNG does not. A bitmap whose alpha is all
        // zero has no alpha at all (old thumbnail providers): it is opaque.
        let no_alpha = pixels.chunks_exact(4).all(|p| p[3] == 0);
        for p in pixels.chunks_exact_mut(4) {
            let a = p[3] as u32;
            if no_alpha {
                p[3] = 255;
            } else if a != 0 && a != 255 {
                for c in &mut p[..3] {
                    *c = (*c as u32 * 255 / a).min(255) as u8;
                }
            }
        }

        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let stream = CreateStreamOnHGlobal(HGLOBAL::default(), true)?;
        let encoder = wic.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
        encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
        let mut frame = None;
        encoder.CreateNewFrame(&mut frame, std::ptr::null_mut())?;
        let frame = frame.ok_or(E_FAIL)?;
        frame.Initialize(None)?;
        frame.SetSize(w as u32, h as u32)?;
        let mut format = GUID_WICPixelFormat32bppBGRA;
        frame.SetPixelFormat(&mut format)?;
        frame.WritePixels(h as u32, (w * 4) as u32, &pixels)?;
        frame.Commit()?;
        encoder.Commit()?;

        let mut stat = STATSTG::default();
        stream.Stat(&mut stat, STATFLAG_NONAME)?;
        let memory = GetHGlobalFromStream(&stream)?;
        let from = GlobalLock(memory) as *const u8;
        if from.is_null() {
            return Err(E_FAIL.into());
        }
        let png = std::slice::from_raw_parts(from, stat.cbSize as usize).to_vec();
        let _ = GlobalUnlock(memory);
        Ok(png)
    }
}

// ── Open, open with, reveal ───────────────────────────────────────────────────

/// Opens a file, a folder or a web address with whatever Windows opens it
/// with. A file nothing is registered for gets the "Open with" dialog.
pub fn open(target: &str) -> Result<(), String> {
    let code = sta(|| unsafe {
        ShellExecuteW(None, w!("open"), &HSTRING::from(target), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL).0 as usize
    });
    match code {
        33.. => Ok(()),
        // SE_ERR_NOASSOC
        31 => open_with_dialog(Path::new(target)),
        _ => Err(format!("Windows could not open {target} (code {code}).")),
    }
}

pub(crate) unsafe fn taken(text: PWSTR) -> String {
    unsafe {
        let out = text.to_string().unwrap_or_default();
        CoTaskMemFree(Some(text.0 as *const c_void));
        out
    }
}

/// The apps Windows recommends for this kind of file, in its order.
unsafe fn handlers(path: &Path) -> Vec<(Handler, IAssocHandler)> {
    let mut out = Vec::new();
    let Some(ext) = path.extension() else { return out };
    let ext = HSTRING::from(format!(".{}", ext.to_string_lossy()));
    unsafe {
        let Ok(list) = SHAssocEnumHandlers(&ext, ASSOC_FILTER_RECOMMENDED) else { return out };
        loop {
            let mut one = [None];
            let mut got = 0u32;
            if list.Next(&mut one, Some(&mut got)).is_err() || got == 0 {
                break;
            }
            let Some(handler) = one[0].take() else { break };
            let name = handler.GetUIName().map(|p| taken(p)).unwrap_or_default();
            let exe = handler.GetName().map(|p| taken(p)).unwrap_or_default();
            if !name.is_empty() {
                out.push((Handler { name, exe }, handler));
            }
        }
    }
    out
}

/// The entries of "Open with" for a file.
pub fn open_with_menu(path: &Path) -> Vec<Handler> {
    sta(|| unsafe { handlers(path).into_iter().map(|(handler, _)| handler).collect() })
}

/// Opens the file with one of the apps `open_with_menu` listed, by its name.
pub fn open_with(path: &Path, name: &str) -> Result<(), String> {
    sta(|| unsafe {
        let Some((_, handler)) = handlers(path).into_iter().find(|(h, _)| h.name == name) else {
            return Err(format!("{name} is not offered for this file any more."));
        };
        let object = data_object(&[Item::File(path.to_path_buf())]).map_err(said)?;
        handler.Invoke(&object).map_err(said)
    })
}

/// Windows' own "Open with" chooser. Returns when it is closed.
pub fn open_with_dialog(path: &Path) -> Result<(), String> {
    let file = wide(path);
    sta(|| unsafe {
        let info = OPENASINFO {
            pcszFile: PCWSTR(file.as_ptr()),
            pcszClass: PCWSTR::null(),
            oaifInFlags: OAIF_EXEC | OAIF_HIDE_REGISTRATION,
        };
        SHOpenWithDialog(None, &info)
    })
    .map_err(said)
}

/// Opens the file's folder in Explorer with the file selected.
pub fn reveal(path: &Path) -> Result<(), String> {
    let file = wide(path);
    sta(|| unsafe {
        let item = ILCreateFromPathW(&file);
        if item.is_null() {
            return Err(format!("{} is not there any more.", path.display()));
        }
        let done = SHOpenFolderAndSelectItems(item, None, 0);
        ILFree(Some(item));
        done.map_err(said)
    })
}

// ── The data object: what a drag or the clipboard carries ─────────────────────

unsafe fn put(object: &IDataObject, format: u16, bytes: &[u8]) -> WinResult<()> {
    unsafe {
        let memory = GlobalAlloc(GMEM_MOVEABLE, bytes.len())?;
        let to = GlobalLock(memory) as *mut u8;
        if to.is_null() {
            return Err(E_FAIL.into());
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), to, bytes.len());
        let _ = GlobalUnlock(memory);
        let format = FORMATETC {
            cfFormat: format,
            ptd: std::ptr::null_mut(),
            dwAspect: DVASPECT_CONTENT.0,
            lindex: -1,
            tymed: TYMED_HGLOBAL.0 as u32,
        };
        let medium = STGMEDIUM {
            tymed: TYMED_HGLOBAL.0 as u32,
            u: STGMEDIUM_0 { hGlobal: memory },
            pUnkForRelease: ManuallyDrop::new(None),
        };
        // `true`: the object owns the memory from here.
        object.SetData(&format, &medium, true)
    }
}

/// The OLE data object for `items`, on a thread where OLE is up. Files travel
/// as the shell's own object (CF_HDROP, Shell IDList Array…), as if dragged
/// from Explorer. With no file in `items` the texts go as CF_UNICODETEXT, and a
/// single link also as a URL (`UniformResourceLocator`, `…W`).
pub fn data_object(items: &[Item]) -> WinResult<IDataObject> {
    unsafe {
        let ids: Vec<*const ITEMIDLIST> = items
            .iter()
            .filter_map(|item| match item {
                Item::File(path) => Some(ILCreateFromPathW(&wide(path)) as *const ITEMIDLIST),
                _ => None,
            })
            .filter(|id| !id.is_null())
            .collect();
        if !ids.is_empty() {
            let object = SHCreateShellItemArrayFromIDLists(&ids)
                .and_then(|array| array.BindToHandler::<_, IDataObject>(None, &BHID_DataObject));
            for id in ids {
                ILFree(Some(id));
            }
            return object;
        }

        // An empty shell data object takes any format given to it.
        let object: IDataObject = SHCreateDataObject(None, None, None)?;
        let utf16 = |s: &str| s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect::<Vec<u8>>();
        let text = items
            .iter()
            .filter_map(|item| match item {
                Item::Text(s) | Item::Link(s) => Some(s.as_str()),
                Item::File(_) => None,
            })
            .collect::<Vec<_>>()
            .join("\r\n");
        put(&object, CF_UNICODETEXT.0, &utf16(&text))?;
        if let [Item::Link(url)] = items {
            put(&object, RegisterClipboardFormatW(w!("UniformResourceLocatorW")) as u16, &utf16(url))?;
            let ansi: Vec<u8> = url.bytes().chain([0]).collect();
            put(&object, RegisterClipboardFormatW(w!("UniformResourceLocator")) as u16, &ansi)?;
        }
        Ok(object)
    }
}

/// Puts files (as Explorer's Copy does) or text on the clipboard.
pub fn copy_to_clipboard(items: &[Item]) -> Result<(), String> {
    sta(|| unsafe {
        OleSetClipboard(&data_object(items)?)?;
        // Renders everything now, so nothing depends on this thread living on.
        OleFlushClipboard()
    })
    .map_err(said)
}

// ── Drag out ──────────────────────────────────────────────────────────────────

/// The picture that follows the pointer during a drag: a bitmap of the first
/// file, made ahead of the drag on any thread.
pub struct DragPicture(isize);

impl Drop for DragPicture {
    fn drop(&mut self) {
        if self.0 != 0 {
            unsafe {
                let _ = DeleteObject(HBITMAP(self.0 as *mut c_void).into());
            }
        }
    }
}

/// The first file's picture, for `drag_out_with`. Reading it can take a while
/// (a large photo seen for the first time): not on the UI thread.
pub fn drag_picture(items: &[Item]) -> Option<DragPicture> {
    let path = items.iter().find_map(|item| match item {
        Item::File(path) => Some(shell_path(path)),
        _ => None,
    })?;
    let bitmap = sta(|| unsafe { shell_bitmap(&path.to_string_lossy(), 96, SIIGBF_RESIZETOFIT).map(|b| b.0 as isize) }).ok()?;
    Some(DragPicture(bitmap))
}

unsafe fn set_drag_picture(object: &IDataObject, mut picture: DragPicture) {
    unsafe {
        let bitmap = HBITMAP(picture.0 as *mut c_void);
        let (w, h) = bitmap_size(bitmap);
        let image = SHDRAGIMAGE {
            sizeDragImage: SIZE { cx: w, cy: h },
            ptOffset: POINT { x: w / 2, y: h / 2 },
            hbmpDragImage: bitmap,
            // CLR_NONE: the bitmap has its own alpha.
            crColorKey: COLORREF(0xFFFF_FFFF),
        };
        let set = CoCreateInstance::<_, IDragSourceHelper>(&CLSID_DragDropHelper, None, CLSCTX_INPROC_SERVER)
            .and_then(|helper| helper.InitializeFromBitmap(&image, object));
        // On success the helper owns the bitmap.
        if set.is_ok() {
            picture.0 = 0;
        }
    }
}

/// Runs an OLE drag of `items` and returns when the button is released: files
/// as files (Explorer, a browser's upload field, a mail), otherwise text and
/// links. The receiver may copy, move or link.
///
/// Call it on the thread that owns the window the press landed in (the UI
/// thread), with the button still down: OLE takes the mouse through that
/// thread's input queue.
pub fn drag_out(items: &[Item]) -> DropEffect {
    drag_out_with(items, drag_picture(items))
}

/// `drag_out` with the picture made beforehand (`drag_picture`), so that the UI
/// thread does not read the file itself.
pub fn drag_out_with(items: &[Item], picture: Option<DragPicture>) -> DropEffect {
    unsafe {
        // Released before we got here: there is nothing to drag any more.
        let button = if GetSystemMetrics(SM_SWAPBUTTON) != 0 { VK_RBUTTON } else { VK_LBUTTON };
        if GetAsyncKeyState(button.0 as i32) >= 0 {
            return DropEffect::None;
        }
        let ole = OleInitialize(None).is_ok();
        let effect = data_object(items).and_then(|object| {
            if let Some(picture) = picture {
                set_drag_picture(&object, picture);
            }
            // No drop source of ours: the shell's default one does what Explorer does.
            SHDoDragDrop(None, &object, None, DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK)
        });
        if ole {
            OleUninitialize();
        }
        match effect {
            Ok(e) if e.contains(DROPEFFECT_MOVE) => DropEffect::Move,
            Ok(e) if e.contains(DROPEFFECT_COPY) => DropEffect::Copy,
            Ok(e) if e.contains(DROPEFFECT_LINK) => DropEffect::Link,
            _ => DropEffect::None,
        }
    }
}

// ── Share ─────────────────────────────────────────────────────────────────────

/// What the Share sheet is given: the files, or failing any the text (and the
/// link). Looking the files up waits on Windows: not on a UI thread. The
/// package itself may be handed to one (`share_sheet`).
pub fn share_package(items: &[Item]) -> WinResult<DataPackage> {
    let data = DataPackage::new()?;
    let mut files: Vec<Option<IStorageItem>> = Vec::new();
    let mut texts: Vec<&str> = Vec::new();
    for item in items {
        match item {
            // Folders are left out: no share target takes one.
            Item::File(path) if path.is_dir() => {}
            Item::File(path) => files.push(Some(StorageFile::GetFileFromPathAsync(&wide(path))?.get()?.cast()?)),
            Item::Text(s) | Item::Link(s) => texts.push(s),
        }
    }
    // The sheet refuses a package without a title: the first item's name, and
    // how many more there are (no words, so nothing to translate).
    let mut title: String = match items.first() {
        Some(Item::File(path)) => path.file_name().unwrap_or_default().to_string_lossy().into_owned(),
        Some(Item::Text(s) | Item::Link(s)) => s.lines().next().unwrap_or_default().chars().take(80).collect(),
        None => String::new(),
    };
    if items.len() > 1 {
        title.push_str(&format!(" +{}", items.len() - 1));
    }
    data.Properties()?.SetTitle(&HSTRING::from(title))?;
    if !files.is_empty() {
        data.SetStorageItemsReadOnly(&IIterable::<IStorageItem>::from(files))?;
    } else {
        if let [Item::Link(url)] = items {
            data.SetWebLink(&Uri::CreateUri(&HSTRING::from(url.as_str()))?)?;
        }
        data.SetText(&HSTRING::from(texts.join("\n")))?;
    }
    Ok(data)
}

/// A share that is up: its window, and what was set on it.
struct Sharing {
    owner: HWND,
    manager: DataTransferManager,
    token: i64,
    /// Whose keyboard it was before the share.
    back: HWND,
    /// The sheet took the keyboard from the owner: when it comes back, the sheet is closed.
    away: bool,
}

thread_local! {
    static SHARING: std::cell::RefCell<Option<Sharing>> = const { std::cell::RefCell::new(None) };
}

/// The package says the share is over (sent, or given up).
const SHARE_OVER: u32 = WM_APP + 1;

/// Where Windows 11 puts the sheet for the point it is asked at, and how large
/// the sheet is (measured with shell-probe, in pixels at 100 %): its corner is
/// this far left of and above the point.
// ponytail: the numbers of one Windows build. A sheet a little off its place is all a change would cost.
const SHEET_CORNER: (i32, i32) = (210, 212);
const SHEET_SIZE: (i32, i32) = (528, 526);

/// Pixels at 100 % as pixels of the screen the point is on.
unsafe fn scaled_at(point: (i32, i32)) -> impl Fn(i32) -> i32 {
    let (mut dpi, mut _y) = (96u32, 96u32);
    unsafe {
        let screen = MonitorFromPoint(POINT { x: point.0, y: point.1 }, MONITOR_DEFAULTTONEAREST);
        let _ = GetDpiForMonitor(screen, MDT_EFFECTIVE_DPI, &mut dpi, &mut _y);
    }
    move |px| px * dpi.max(96) as i32 / 96
}

/// The window the user was in: the one in front or, when that is one of this
/// app's own (the island, holding the keyboard for the click on Share), the
/// first one down the stack that takes keys. Always-on-top windows are passed
/// over: they sit above the window the user works in whether or not they were
/// used last.
fn users_window() -> HWND {
    unsafe {
        let front = GetForegroundWindow();
        let mut pid = 0u32;
        GetWindowThreadProcessId(front, Some(&mut pid));
        if !front.is_invalid() && pid != std::process::id() {
            return front;
        }
        notch_core::win::top_windows()
            .into_iter()
            .map(|(raw, pid, _)| (HWND(raw as *mut c_void), pid))
            .find(|(window, pid)| {
                let style = GetWindowLongW(*window, GWL_EXSTYLE) as u32;
                let mut cloaked = 0u32;
                let _ = DwmGetWindowAttribute(*window, DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut c_void, 4);
                *pid != std::process::id()
                    && style & (WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0 | WS_EX_TOPMOST.0) == 0
                    && cloaked == 0
                    && !IsIconic(*window).as_bool()
            })
            .map(|(window, _)| window)
            .unwrap_or_default()
    }
}

/// Makes `window` the one in front, and says whether it now is. Windows lets a
/// process do that only while it is in front itself or was the last to get or
/// send input, and a click in a web view goes to the web view's own process,
/// not to the app's. A mouse event that moves nothing makes this process the
/// last to have sent input (the island takes the keyboard the same way).
unsafe fn to_front(window: HWND) -> bool {
    unsafe {
        let _ = SetForegroundWindow(window);
        if GetForegroundWindow() == window {
            return true;
        }
        let nudge = INPUT { r#type: INPUT_MOUSE, Anonymous: INPUT_0 { mi: MOUSEINPUT { dwFlags: MOUSEEVENTF_MOVE, ..Default::default() } } };
        SendInput(&[nudge], size_of::<INPUT>() as i32);
        let _ = SetForegroundWindow(window);
        GetForegroundWindow() == window
    }
}

/// Takes the share's window down. If the keyboard is with it (the sheet has
/// closed and Windows gave it back), it goes on to the window the user was in;
/// if the user is already somewhere else, it stays there.
fn end_share() {
    if let Some(share) = SHARING.take() {
        let _ = share.manager.RemoveDataRequested(share.token);
        unsafe {
            if GetForegroundWindow() == share.owner && IsWindowVisible(share.back).as_bool() && !IsIconic(share.back).as_bool() {
                to_front(share.back);
            }
            let _ = DestroyWindow(share.owner);
        }
    }
}

unsafe extern "system" fn share_owner(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_ACTIVATE => {
                let active = wparam.0 & 0xffff != 0;
                let back = SHARING.with_borrow_mut(|share| match share {
                    Some(share) if share.owner == hwnd => {
                        let was_away = share.away;
                        share.away |= !active;
                        was_away && active
                    }
                    _ => false,
                });
                // The sheet has closed. Not from inside the activation itself.
                if back {
                    let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
                LRESULT(0)
            }
            // Over without the keyboard coming back here (the user clicked another
            // window, or the app shared with took over): a moment for that to settle.
            SHARE_OVER => {
                SetTimer(Some(hwnd), 1, 500, None);
                LRESULT(0)
            }
            WM_TIMER | WM_CLOSE => {
                if SHARING.with_borrow(|share| share.as_ref().is_some_and(|share| share.owner == hwnd)) {
                    end_share();
                } else {
                    let _ = DestroyWindow(hwnd);
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// Shows the Windows Share sheet for what `share_package` made, hanging under
/// the screen point `under`: its top edge there, its middle below the point.
/// Call it on a thread that runs a message loop (the UI thread), for something
/// the user just asked for: it takes the keyboard.
///
/// The sheet comes up only for the window that is in front. For a hidden
/// window, or for one that does not take the keyboard (the island), Windows
/// answers "done" and shows nothing. So the sheet gets a window of its own: one
/// transparent pixel that takes the keyboard, hands it to the sheet and, when
/// the sheet has closed, hands it back to the window the user was in and goes.
pub fn share_sheet(data: &DataPackage, under: (i32, i32)) -> Result<(), String> {
    end_share();
    unsafe {
        let scale = scaled_at(under);
        let at = (under.0 - scale(SHEET_SIZE.0 / 2 - SHEET_CORNER.0), under.1 + scale(SHEET_CORNER.1));
        let back = users_window();
        let module = GetModuleHandleW(None).map_err(said)?;
        let class = w!("svc-shell.share-owner");
        // Registered once; again is refused and harmless.
        RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(share_owner), hInstance: module.into(), lpszClassName: class, ..Default::default() });
        let owner = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_TOPMOST,
            class,
            PCWSTR::null(),
            WS_POPUP,
            at.0,
            at.1,
            1,
            1,
            None,
            None,
            Some(module.into()),
            None,
        )
        .map_err(said)?;
        let _ = SetLayeredWindowAttributes(owner, COLORREF(0), 0, LWA_ALPHA);
        let _ = ShowWindow(owner, SW_SHOW);
        let shown = (|| {
            if !to_front(owner) {
                return Err("Windows did not let the Share sheet come to the front.".to_string());
            }
            let interop: IDataTransferManagerInterop = factory::<DataTransferManager, IDataTransferManagerInterop>().map_err(said)?;
            let manager: DataTransferManager = interop.GetForWindow(owner).map_err(said)?;
            let package = data.clone();
            let token = manager
                .DataRequested(&TypedEventHandler::<DataTransferManager, DataRequestedEventArgs>::new(move |_, args| {
                    args.ok()?.Request()?.SetData(&package)
                }))
                .map_err(said)?;
            SHARING.set(Some(Sharing { owner, manager, token, back, away: false }));
            // Told from whatever thread Windows has for it: the window is only sent a message.
            let raw = owner.0 as isize;
            let over = move || {
                let _ = PostMessageW(Some(HWND(raw as *mut c_void)), SHARE_OVER, WPARAM(0), LPARAM(0));
            };
            let _ = data.ShareCompleted(&TypedEventHandler::<DataPackage, ShareCompletedEventArgs>::new(move |_, _| {
                over();
                Ok(())
            }));
            let _ = data.ShareCanceled(&TypedEventHandler::<DataPackage, IInspectable>::new(move |_, _| {
                over();
                Ok(())
            }));
            interop.ShowShareUIForWindow(owner).map_err(said)
        })();
        if shown.is_err() {
            if SHARING.with_borrow(|share| share.is_some()) {
                end_share();
            } else {
                let _ = DestroyWindow(owner);
            }
        }
        shown
    }
}

/// The Share sheet for `items`, for a caller that has a window rather than a
/// place: over the middle of the window `hwnd` when that is on screen, else in
/// the middle of the main screen. On the thread that owns the window. The
/// sheet is not that window's (see `share_sheet`): asked for a window that is
/// hidden, or one that does not take the keyboard, Windows shows none.
///
/// It looks the files up itself and keeps its thread waiting meanwhile: a
/// caller that minds makes the package elsewhere and calls `share_sheet`.
pub fn share(items: &[Item], hwnd: isize) -> Result<(), String> {
    let data = std::thread::scope(|scope| scope.spawn(|| share_package(items)).join().expect("share worker")).map_err(said)?;
    unsafe {
        let window = HWND(hwnd as *mut c_void);
        let mut at = RECT::default();
        let middle = if IsWindowVisible(window).as_bool() && !IsIconic(window).as_bool() && GetWindowRect(window, &mut at).is_ok() {
            ((at.left + at.right) / 2, (at.top + at.bottom) / 2)
        } else {
            (GetSystemMetrics(SM_CXSCREEN) / 2, GetSystemMetrics(SM_CYSCREEN) / 2)
        };
        share_sheet(&data, (middle.0, middle.1 - scaled_at(middle)(SHEET_SIZE.1 / 2)))
    }
}

// ── Zip ───────────────────────────────────────────────────────────────────────

/// Packs files and folders into the zip `out`, each at the top of the archive,
/// with the tar.exe that ships with Windows 10 1803 and later.
pub fn zip(paths: &[PathBuf], out: &Path) -> Result<(), String> {
    // System32's bsdtar by its full path: a tar found on PATH may not write zip.
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    let mut tar = Command::new(Path::new(&root).join("System32").join("tar.exe"));
    tar.arg("-a").arg("-c").arg("-f").arg(out);
    for path in paths {
        let (Some(folder), Some(name)) = (path.parent(), path.file_name()) else {
            return Err(format!("{} cannot be zipped.", path.display()));
        };
        // ".\name": a file called "-x" or "@list" must not read as an option.
        tar.arg("-C").arg(folder).arg(Path::new(".").join(name));
    }
    let done = notch_core::win::no_console(&mut tar).output().map_err(|e| format!("tar.exe did not start: {e}"))?;
    if done.status.success() {
        return Ok(());
    }
    let _ = std::fs::remove_file(out);
    Err(String::from_utf8_lossy(&done.stderr).trim().to_string())
}
