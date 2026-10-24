// What changes a user's own file: a new name, the Recycle Bin. And the file
// picker, which is how files are named to the app without a drag.
//
// debt: `recycle` belongs to svc-fs (docs/ARCHITECTURE.md §8) once that crate exists.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use windows::core::{implement, Ref, Result as WinResult, HRESULT, HSTRING, PCWSTR};
use windows::Win32::Foundation::E_ABORT;
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL, CLSCTX_INPROC_SERVER};
use windows::Win32::UI::Shell::{
    FileOpenDialog, FileOperation, IFileOpenDialog, IFileOperation, IFileOperationProgressSink,
    IFileOperationProgressSink_Impl, IShellItem, SHCreateItemFromParsingName, FOFX_EARLYFAILURE, FOFX_RECYCLEONDELETE,
    FOF_ALLOWUNDO, FOF_NO_UI, FOS_ALLOWMULTISELECT, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, SIGDN,
    SIGDN_DESKTOPABSOLUTEPARSING, SIGDN_FILESYSPATH, TSF_DELETE_RECYCLE_IF_POSSIBLE,
};

use crate::{said, sta, taken, wide};

// ── A new name ────────────────────────────────────────────────────────────────

/// Why a file cannot be given a name. The words are the caller's: it has the translations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameProblem {
    /// Nothing but spaces.
    Empty,
    /// One of `\ / : * ? " < > |`, a control character, or a dot or a space at the end.
    Characters,
    /// A name Windows keeps for a device (CON, NUL, COM1…), or `.` and `..`.
    Reserved,
    /// More than 255 characters.
    TooLong,
    /// Another file or folder there has that name.
    Taken,
    /// The file to rename is gone.
    Missing,
    /// Windows refused (the file is open in a program, the folder is read-only).
    Refused,
}

impl NameProblem {
    /// A word for each, for a caller that passes it on as data.
    pub fn code(self) -> &'static str {
        match self {
            NameProblem::Empty => "empty",
            NameProblem::Characters => "characters",
            NameProblem::Reserved => "reserved",
            NameProblem::TooLong => "long",
            NameProblem::Taken => "taken",
            NameProblem::Missing => "missing",
            NameProblem::Refused => "refused",
        }
    }
}

/// What is wrong with `name` as the name of a file or a folder, whatever folder it is in.
pub fn name_problem(name: &str) -> Option<NameProblem> {
    let name = name.trim();
    if name.is_empty() {
        return Some(NameProblem::Empty);
    }
    if name == "." || name == ".." {
        return Some(NameProblem::Reserved);
    }
    if name.chars().any(|c| c.is_control() || r#"\/:*?"<>|"#.contains(c)) || name.ends_with('.') {
        return Some(NameProblem::Characters);
    }
    // "nul.txt" is as reserved as "NUL".
    let stem = name.split('.').next().unwrap_or_default().trim_end().to_ascii_uppercase();
    let numbered = |prefix: &str| stem.strip_prefix(prefix).is_some_and(|n| matches!(n, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"));
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL") || numbered("COM") || numbered("LPT") {
        return Some(NameProblem::Reserved);
    }
    if name.encode_utf16().count() > 255 {
        return Some(NameProblem::TooLong);
    }
    None
}

/// Gives the file or folder at `path` the name `name`, where it is. Returns
/// its new path. A name that differs only in its capitals is a change too.
pub fn rename(path: &Path, name: &str) -> Result<PathBuf, NameProblem> {
    let name = name.trim();
    if let Some(problem) = name_problem(name) {
        return Err(problem);
    }
    let (Some(folder), Some(old)) = (path.parent(), path.file_name()) else { return Err(NameProblem::Missing) };
    if !path.exists() {
        return Err(NameProblem::Missing);
    }
    if old.to_string_lossy() == name {
        return Ok(path.to_path_buf());
    }
    let to = folder.join(name);
    // On a disk that ignores capitals the new name "exists" as the file itself.
    let same_file = old.to_string_lossy().to_lowercase() == name.to_lowercase();
    if to.exists() && !same_file {
        return Err(NameProblem::Taken);
    }
    std::fs::rename(path, &to).map(|()| to).map_err(|_| NameProblem::Refused)
}

// ── The Recycle Bin ───────────────────────────────────────────────────────────

/// A file that went to the Recycle Bin: where it was, and where it lies now.
#[derive(Debug, Clone)]
pub struct Recycled {
    pub from: PathBuf,
    pub bin: PathBuf,
}

unsafe fn name_of(item: &IShellItem, form: SIGDN) -> PathBuf {
    unsafe { item.GetDisplayName(form).map(|name| PathBuf::from(taken(name))).unwrap_or_default() }
}

/// Watches the deletes: stops one that would not go to the bin, notes the ones that did.
#[implement(IFileOperationProgressSink)]
struct Watcher {
    recycled: Rc<RefCell<Vec<Recycled>>>,
}

#[allow(non_snake_case)]
impl IFileOperationProgressSink_Impl for Watcher_Impl {
    fn PreDeleteItem(&self, flags: u32, _item: Ref<'_, IShellItem>) -> WinResult<()> {
        // Without this flag Windows is about to delete for good (a drive with no
        // bin, a file larger than the bin): the user asked for the bin, so no.
        if flags & TSF_DELETE_RECYCLE_IF_POSSIBLE.0 as u32 == 0 {
            return Err(E_ABORT.into());
        }
        Ok(())
    }
    fn PostDeleteItem(&self, _flags: u32, item: Ref<'_, IShellItem>, done: HRESULT, binned: Ref<'_, IShellItem>) -> WinResult<()> {
        if let (true, Some(item), Some(binned)) = (done.is_ok(), item.as_ref(), binned.as_ref()) {
            let (from, bin) = unsafe { (name_of(item, SIGDN_FILESYSPATH), name_of(binned, SIGDN_DESKTOPABSOLUTEPARSING)) };
            self.recycled.borrow_mut().push(Recycled { from, bin });
        }
        Ok(())
    }

    fn StartOperations(&self) -> WinResult<()> {
        Ok(())
    }
    fn FinishOperations(&self, _: HRESULT) -> WinResult<()> {
        Ok(())
    }
    fn PreRenameItem(&self, _: u32, _: Ref<'_, IShellItem>, _: &PCWSTR) -> WinResult<()> {
        Ok(())
    }
    fn PostRenameItem(&self, _: u32, _: Ref<'_, IShellItem>, _: &PCWSTR, _: HRESULT, _: Ref<'_, IShellItem>) -> WinResult<()> {
        Ok(())
    }
    fn PreMoveItem(&self, _: u32, _: Ref<'_, IShellItem>, _: Ref<'_, IShellItem>, _: &PCWSTR) -> WinResult<()> {
        Ok(())
    }
    fn PostMoveItem(&self, _: u32, _: Ref<'_, IShellItem>, _: Ref<'_, IShellItem>, _: &PCWSTR, _: HRESULT, _: Ref<'_, IShellItem>) -> WinResult<()> {
        Ok(())
    }
    fn PreCopyItem(&self, _: u32, _: Ref<'_, IShellItem>, _: Ref<'_, IShellItem>, _: &PCWSTR) -> WinResult<()> {
        Ok(())
    }
    fn PostCopyItem(&self, _: u32, _: Ref<'_, IShellItem>, _: Ref<'_, IShellItem>, _: &PCWSTR, _: HRESULT, _: Ref<'_, IShellItem>) -> WinResult<()> {
        Ok(())
    }
    fn PreNewItem(&self, _: u32, _: Ref<'_, IShellItem>, _: &PCWSTR) -> WinResult<()> {
        Ok(())
    }
    fn PostNewItem(&self, _: u32, _: Ref<'_, IShellItem>, _: &PCWSTR, _: &PCWSTR, _: u32, _: HRESULT, _: Ref<'_, IShellItem>) -> WinResult<()> {
        Ok(())
    }
    fn UpdateProgress(&self, _: u32, _: u32) -> WinResult<()> {
        Ok(())
    }
    fn ResetTimer(&self) -> WinResult<()> {
        Ok(())
    }
    fn PauseTimer(&self) -> WinResult<()> {
        Ok(())
    }
    fn ResumeTimer(&self) -> WinResult<()> {
        Ok(())
    }
}

/// Moves files and folders to the Recycle Bin, as Explorer's Delete does, and
/// says which went. Never deletes for good: what cannot go to the bin stays
/// where it is. Nothing is asked and nothing is shown: asking is the caller's.
pub fn recycle(paths: &[PathBuf]) -> Result<Vec<Recycled>, String> {
    sta(|| unsafe {
        let recycled = Rc::new(RefCell::new(Vec::new()));
        let work: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)?;
        work.SetOperationFlags(FOF_NO_UI | FOF_ALLOWUNDO | FOFX_RECYCLEONDELETE | FOFX_EARLYFAILURE)?;
        let watcher: IFileOperationProgressSink = Watcher { recycled: recycled.clone() }.into();
        let watching = work.Advise(&watcher)?;
        let mut any = false;
        for path in paths {
            // A drive is not a thing to recycle.
            if path.parent().is_none() || path.file_name().is_none() {
                continue;
            }
            if let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(&wide(path), None) {
                any |= work.DeleteItem(&item, None).is_ok();
            }
        }
        // A stopped delete (it would have been for good) is an error here; what
        // did go is in the list either way.
        if any {
            let _ = work.PerformOperations();
        }
        let _ = work.Unadvise(watching);
        let out = recycled.borrow().clone();
        Ok(out)
    })
    .map_err(said)
}

// ── The file picker ───────────────────────────────────────────────────────────

/// Windows' own "Open" dialog for any number of files, with `title` in its
/// title bar. Returns when it is closed, with what was chosen (nothing when it
/// was cancelled). Not on a UI thread: it waits.
// ponytail: files only. Windows has one dialog for files and another for
// folders; a folder still gets to the caller by a drag.
pub fn pick_files(title: &str) -> Vec<PathBuf> {
    let title = HSTRING::from(title);
    sta(|| unsafe {
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
        dialog.SetOptions(dialog.GetOptions()? | FOS_ALLOWMULTISELECT | FOS_FORCEFILESYSTEM | FOS_FILEMUSTEXIST)?;
        dialog.SetTitle(&title)?;
        // An error is the dialog cancelled.
        dialog.Show(None)?;
        let chosen = dialog.GetResults()?;
        let mut out = Vec::new();
        for i in 0..chosen.GetCount()? {
            out.push(name_of(&chosen.GetItemAt(i)?, SIGDN_FILESYSPATH));
        }
        WinResult::Ok(out)
    })
    .unwrap_or_default()
}

/// Windows' own folder dialog, with `title` in its title bar: the one folder chosen, or
/// nothing when it was cancelled. Not on a UI thread: it waits.
pub fn pick_folder(title: &str) -> Option<PathBuf> {
    let title = HSTRING::from(title);
    sta(|| unsafe {
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
        dialog.SetOptions(dialog.GetOptions()? | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM)?;
        dialog.SetTitle(&title)?;
        // An error is the dialog cancelled.
        dialog.Show(None)?;
        WinResult::Ok(name_of(&dialog.GetResult()?, SIGDN_FILESYSPATH))
    })
    .ok()
    .filter(|path| !path.as_os_str().is_empty())
}
