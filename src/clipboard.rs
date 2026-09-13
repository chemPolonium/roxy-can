//! Windows clipboard glue. dear-imgui-rs routes every clipboard access
//! (input-text Ctrl+C/X/V and the trace "Copy" menu items) through an
//! app-installed [`dear_imgui_rs::ClipboardBackend`], and ships none for
//! the desktop; this module is that backend, on the Win32 API directly.

use dear_imgui_rs::ClipboardBackend;
use windows_sys::Win32::Foundation::GlobalFree;
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    SetClipboardData,
};
use windows_sys::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
};

/// CF_UNICODETEXT
const CF_TEXT: u32 = 13;

/// The process-wide OS clipboard handle. Stateless; any number of copies
/// may exist.
#[derive(Default, Clone, Copy)]
pub struct Clipboard;

impl Clipboard {
    /// Writes UTF-8 text to the clipboard (UTF-16 on the wire).
    pub fn set(&self, text: &str) {
        set_win(text);
    }

    /// Reads the clipboard as UTF-8, when it holds text.
    pub fn get(&self) -> Option<String> {
        get_win()
    }
}

impl ClipboardBackend for Clipboard {
    fn get(&mut self) -> Option<String> {
        Clipboard.get()
    }

    fn set(&mut self, value: &str) {
        Clipboard.set(value);
    }
}

fn set_win(text: &str) {
    unsafe {
        if OpenClipboard(0) == 0 {
            return;
        }
        // Taking ownership of the clipboard (a NULL-window open) requires
        // an explicit emptying before the data lands.
        EmptyClipboard();
        // The clipboard owns everything handed to SetClipboardData; only
        // an allocation failure here leaves a handle for GlobalFree.
        let mut free_after = None;
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let handle = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2);
        if !handle.is_null() {
            let dst = GlobalLock(handle);
            if !dst.is_null() {
                std::ptr::copy_nonoverlapping(wide.as_ptr(), dst as *mut u16, wide.len());
                GlobalUnlock(handle as _);
                if SetClipboardData(CF_TEXT, handle as _) == 0 {
                    free_after = Some(handle);
                }
            } else {
                free_after = Some(handle);
            }
        }
        if let Some(h) = free_after {
            GlobalFree(h);
        }
        CloseClipboard();
    }
}

fn get_win() -> Option<String> {
    unsafe {
        if IsClipboardFormatAvailable(CF_TEXT) == 0 || OpenClipboard(0) == 0 {
            return None;
        }
        let out = (|| {
            let handle = GetClipboardData(CF_TEXT);
            if handle == 0 {
                return None;
            }
            let src = GlobalLock(handle as _);
            if src.is_null() {
                return None;
            }
            // The data is a NUL-terminated UTF-16 run; GlobalSize is the
            // allocation's byte ceiling, so walk to the terminator and
            // stay inside it.
            let size = GlobalSize(handle as _) / 2;
            let mut units: Vec<u16> = Vec::new();
            for i in 0..size {
                let unit = *(src as *const u16).add(i);
                if unit == 0 {
                    break;
                }
                units.push(unit);
            }
            GlobalUnlock(handle as _);
            Some(String::from_utf16_lossy(&units))
        })();
        CloseClipboard();
        out
    }
}
