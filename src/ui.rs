//! Small Win32 helpers shared by the windows.

use std::sync::atomic::{AtomicU32, Ordering};
use windows_sys::Win32::{
    Foundation::*, Graphics::Gdi::*, UI::HiDpi::GetDpiForSystem, UI::WindowsAndMessaging::*,
};

static DPI: AtomicU32 = AtomicU32::new(96);

pub fn init_dpi() {
    DPI.store(unsafe { GetDpiForSystem() }, Ordering::Relaxed);
}

/// Scales a length given at 96 DPI to the current system DPI.
pub fn scale(v: i32) -> i32 {
    v * DPI.load(Ordering::Relaxed) as i32 / 96
}

/// Null-terminated UTF-16 string.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

pub fn loword(v: usize) -> u32 {
    (v & 0xFFFF) as u32
}

pub fn hiword(v: usize) -> u32 {
    ((v >> 16) & 0xFFFF) as u32
}

pub fn window_text(hwnd: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        let mut buf = vec![0u16; len as usize + 1];
        let copied = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        String::from_utf16_lossy(&buf[..copied as usize])
    }
}

pub fn set_window_text(hwnd: HWND, text: &str) {
    unsafe { SetWindowTextW(hwnd, wide(text).as_ptr()) };
}

/// The system UI font, optionally enlarged and/or bold.
pub fn message_font(size_factor: f32, bold: bool) -> HFONT {
    unsafe {
        let mut metrics: NONCLIENTMETRICSW = std::mem::zeroed();
        metrics.cbSize = std::mem::size_of::<NONCLIENTMETRICSW>() as u32;
        SystemParametersInfoW(
            SPI_GETNONCLIENTMETRICS,
            metrics.cbSize,
            &mut metrics as *mut _ as *mut _,
            0,
        );
        let mut font = metrics.lfMessageFont;
        font.lfHeight = (font.lfHeight as f32 * size_factor) as i32;
        if bold {
            font.lfWeight = FW_SEMIBOLD as i32;
        }
        CreateFontIndirectW(&font)
    }
}

pub fn set_font(hwnd: HWND, font: HFONT) {
    unsafe { SendMessageW(hwnd, WM_SETFONT, font as usize, 1) };
}
