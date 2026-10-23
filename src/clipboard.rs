//! The clipboard: saving and restoring its contents, reading and writing text.

use crate::ui;
use windows_sys::Win32::{
    Foundation::*,
    System::{DataExchange::*, Memory::*, Threading::Sleep},
};

const CF_UNICODETEXT: u32 = 13;

/// Formats whose data is a GDI or owner-drawn handle rather than memory, so
/// it can't be copied byte by byte. Windows makes up the usual ones (a bitmap
/// from a DIB, for one) from the formats that are kept.
fn is_handle_format(format: u32) -> bool {
    matches!(format, 2 | 3 | 9 | 14 | 0x80..=0x8F | 0x300..=0x3FF)
}

/// A copy of what the clipboard held.
pub struct Backup(Vec<(u32, Vec<u8>)>);

/// The clipboard is shared: another program may hold it open for a moment.
fn open(window: HWND) -> bool {
    for _ in 0..10 {
        if unsafe { OpenClipboard(window) } != 0 {
            return true;
        }
        unsafe { Sleep(5) };
    }
    false
}

/// Copies the contents of every format that is plain memory.
pub fn backup(window: HWND) -> Backup {
    let mut formats = Vec::new();
    if !open(window) {
        return Backup(formats);
    }
    unsafe {
        let mut format = EnumClipboardFormats(0);
        while format != 0 {
            if !is_handle_format(format) {
                if let Some(data) = global_bytes(GetClipboardData(format)) {
                    formats.push((format, data));
                }
            }
            format = EnumClipboardFormats(format);
        }
        CloseClipboard();
    }
    Backup(formats)
}

/// Puts the saved contents back.
pub fn restore(window: HWND, backup: Backup) {
    if !open(window) {
        return;
    }
    unsafe {
        EmptyClipboard();
        for (format, data) in backup.0 {
            if let Some(memory) = global_from(&data) {
                if SetClipboardData(format, memory).is_null() {
                    GlobalFree(memory);
                }
            }
        }
        CloseClipboard();
    }
}

pub fn empty(window: HWND) {
    if open(window) {
        unsafe {
            EmptyClipboard();
            CloseClipboard();
        }
    }
}

/// The text on the clipboard, if any.
pub fn text(window: HWND) -> Option<String> {
    if !open(window) {
        return None;
    }
    let data = unsafe { global_bytes(GetClipboardData(CF_UNICODETEXT)) };
    unsafe { CloseClipboard() };
    let units: Vec<u16> =
        data?.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).take_while(|&u| u != 0).collect();
    Some(String::from_utf16_lossy(&units))
}

/// Replaces the contents with `text`, kept out of the clipboard history.
pub fn set_text(window: HWND, text: &str) -> bool {
    if !open(window) {
        return false;
    }
    // NUL-terminated, as the format requires.
    let data: Vec<u8> = ui::wide(text).iter().flat_map(|u| u.to_le_bytes()).collect();
    unsafe {
        EmptyClipboard();
        let ok = global_from(&data).is_some_and(|memory| {
            let set = !SetClipboardData(CF_UNICODETEXT, memory).is_null();
            if !set {
                GlobalFree(memory);
            }
            set
        });
        // Win+V history and cloud clipboard skip content marked this way.
        let exclude = ui::wide("ExcludeClipboardContentFromMonitorProcessing");
        let format = RegisterClipboardFormatW(exclude.as_ptr());
        if let Some(memory) = global_from(&[0]) {
            if SetClipboardData(format, memory).is_null() {
                GlobalFree(memory);
            }
        }
        CloseClipboard();
        ok
    }
}

/// Changes every time anything is put on the clipboard.
pub fn sequence() -> u32 {
    unsafe { GetClipboardSequenceNumber() }
}

unsafe fn global_bytes(memory: HANDLE) -> Option<Vec<u8>> {
    if memory.is_null() {
        return None;
    }
    let size = GlobalSize(memory);
    let ptr = GlobalLock(memory) as *const u8;
    if ptr.is_null() {
        return None;
    }
    let data = std::slice::from_raw_parts(ptr, size).to_vec();
    GlobalUnlock(memory);
    Some(data)
}

unsafe fn global_from(data: &[u8]) -> Option<HGLOBAL> {
    let memory = GlobalAlloc(GMEM_MOVEABLE, data.len().max(1));
    if memory.is_null() {
        return None;
    }
    let ptr = GlobalLock(memory) as *mut u8;
    if ptr.is_null() {
        GlobalFree(memory);
        return None;
    }
    std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
    GlobalUnlock(memory);
    Some(memory)
}
