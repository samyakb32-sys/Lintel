// What is under the pointer. Plain Win32 and UI Automation, nothing of the app in it: the
// window and the program under (x, y), and the text of the page there.
//
// Read-only and local. The text goes nowhere from here: the companion keeps it until the
// user clicks the character, and the chat sends it only when the user sends a message.

use windows::core::Interface;
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
use windows::Win32::UI::Accessibility::{CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTextPattern, UIA_TextPatternId};
use windows::Win32::UI::WindowsAndMessaging::{GetAncestor, GetCursorPos, GetWindowTextW, GetWindowThreadProcessId, WindowFromPoint, GA_ROOT};

/// The window under the pointer and what it shows.
pub struct Page {
    pub title: String,
    /// The page's text, at most the length asked for. Empty when the program shows none.
    pub text: String,
}

pub fn cursor() -> Option<(i32, i32)> {
    let mut p = POINT::default();
    unsafe { GetCursorPos(&mut p).ok()? };
    Some((p.x, p.y))
}

/// The top-level window under (x, y) and the program it belongs to.
fn window_at(x: i32, y: i32) -> Option<(HWND, u32)> {
    unsafe {
        let hit = WindowFromPoint(POINT { x, y });
        if hit.0.is_null() {
            return None;
        }
        let root = GetAncestor(hit, GA_ROOT);
        let root = if root.0.is_null() { hit } else { root };
        let mut pid = 0u32;
        GetWindowThreadProcessId(root, Some(&mut pid));
        (pid != 0).then_some((root, pid))
    }
}

/// The program that owns the window under (x, y).
pub fn pid_at(x: i32, y: i32) -> Option<u32> {
    window_at(x, y).map(|(_, pid)| pid)
}

fn title_of(window: HWND) -> String {
    let mut buf = [0u16; 512];
    let n = unsafe { GetWindowTextW(window, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// The page under (x, y): its window's title and up to `max` characters of its text. `None`
/// when there is nothing to read there, or the pointer is over a password field.
pub fn read(x: i32, y: i32, max: usize) -> Option<Page> {
    let (window, _) = window_at(x, y)?;
    let title = title_of(window);
    let init = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    let text = text_at(x, y, max);
    if init.is_ok() {
        unsafe { CoUninitialize() };
    }
    Some(Page { title, text: text? })
}

/// The text of the document the element under (x, y) is in, else the names of the element
/// and the ones around it. `None` for a password field.
fn text_at(x: i32, y: i32, max: usize) -> Option<String> {
    let ui: IUIAutomation = unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }.ok()?;
    let at = unsafe { ui.ElementFromPoint(POINT { x, y }) }.ok()?;
    if unsafe { at.CurrentIsPassword() }.map(|b| b.as_bool()).unwrap_or(false) {
        return None;
    }
    let walker = unsafe { ui.ControlViewWalker() }.ok()?;
    let mut names: Vec<String> = Vec::new();
    let mut element: IUIAutomationElement = at;
    // A browser or an editor answers for the whole document; the sixteenth parent is the end of any real tree.
    for step in 0..16 {
        if let Some(text) = document_text(&element, max) {
            return Some(text);
        }
        if step < 4 {
            let name = unsafe { element.CurrentName() }.map(|n| n.to_string()).unwrap_or_default();
            if !name.trim().is_empty() && !names.contains(&name) {
                names.push(name);
            }
        }
        match unsafe { walker.GetParentElement(&element) } {
            Ok(parent) => element = parent,
            Err(_) => break,
        }
    }
    Some(names.join("\n").chars().take(max).collect())
}

/// The whole text of `element` when it is a document (a text pattern), cut to `max` characters.
fn document_text(element: &IUIAutomationElement, max: usize) -> Option<String> {
    let pattern: IUIAutomationTextPattern = unsafe { element.GetCurrentPattern(UIA_TextPatternId) }.ok()?.cast().ok()?;
    let range = unsafe { pattern.DocumentRange() }.ok()?;
    let text = unsafe { range.GetText(max as i32) }.ok()?.to_string();
    (!text.trim().is_empty()).then_some(text)
}
