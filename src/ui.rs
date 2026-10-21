//! Small Win32 helpers shared by the windows.

use std::sync::atomic::{AtomicU32, Ordering};
use windows_sys::Win32::{
    Foundation::*, Graphics::Gdi::*, UI::HiDpi::GetDpiForSystem, UI::WindowsAndMessaging::*,
};

static DPI: AtomicU32 = AtomicU32::new(96);

/// Makes standard controls (buttons etc.) use the modern visual style.
///
/// The exe has no manifest, so this activates the Common Controls v6
/// manifest that ships inside shell32.dll (resource 124) for this thread.
/// Must be called before any window is created.
pub fn enable_visual_styles() {
    use windows_sys::Win32::System::{
        ApplicationInstallationAndServicing::{ActivateActCtx, CreateActCtxW, ACTCTXW},
        SystemInformation::GetSystemDirectoryW,
    };
    const ACTCTX_FLAG_ASSEMBLY_DIRECTORY_VALID: u32 = 0x4;
    const ACTCTX_FLAG_RESOURCE_NAME_VALID: u32 = 0x8;

    unsafe {
        let mut dir = [0u16; 260];
        if GetSystemDirectoryW(dir.as_mut_ptr(), dir.len() as u32) == 0 {
            return;
        }
        let source = wide("shell32.dll");
        let ctx = ACTCTXW {
            cbSize: std::mem::size_of::<ACTCTXW>() as u32,
            dwFlags: ACTCTX_FLAG_RESOURCE_NAME_VALID | ACTCTX_FLAG_ASSEMBLY_DIRECTORY_VALID,
            lpSource: source.as_ptr(),
            lpAssemblyDirectory: dir.as_ptr(),
            lpResourceName: 124 as _,
            ..std::mem::zeroed()
        };
        let handle = CreateActCtxW(&ctx);
        if handle != INVALID_HANDLE_VALUE {
            let mut cookie = 0;
            ActivateActCtx(handle, &mut cookie);
        }
    }
}

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
