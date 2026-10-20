//! The app icon, drawn at runtime: a blue rounded square with a white "A".

use crate::ui;
use std::ptr::null_mut;
use windows_sys::Win32::{Graphics::Gdi::*, UI::WindowsAndMessaging::*};

/// Creates an icon of `size` x `size` pixels.
pub fn create(size: i32) -> HICON {
    unsafe {
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size,
            biHeight: -size, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB as u32,
            ..std::mem::zeroed()
        };
        let mut bits: *mut core::ffi::c_void = null_mut();
        let color = CreateDIBSection(null_mut(), &bmi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if color.is_null() {
            return LoadIconW(null_mut(), IDI_APPLICATION);
        }
        let pixels = std::slice::from_raw_parts_mut(bits as *mut u32, (size * size) as usize);

        // Rounded square with 4x4 supersampled edges.
        let radius = size as f32 * 0.22;
        let mut coverage = vec![0u8; pixels.len()];
        for y in 0..size {
            for x in 0..size {
                let mut hits = 0;
                for sy in 0..4 {
                    for sx in 0..4 {
                        let px = x as f32 + (sx as f32 + 0.5) / 4.0;
                        let py = y as f32 + (sy as f32 + 0.5) / 4.0;
                        if inside_rounded_square(px, py, size as f32, radius) {
                            hits += 1;
                        }
                    }
                }
                let i = (y * size + x) as usize;
                coverage[i] = (hits * 255 / 16) as u8;
                pixels[i] = 0x0000_78D6; // #0078D6 as 0x00RRGGBB
            }
        }

        let dc = CreateCompatibleDC(null_mut());
        let old_bitmap = SelectObject(dc, color);
        let face = ui::wide("Segoe UI");
        let font = CreateFontW(
            -(size * 4 / 5),
            0,
            0,
            0,
            FW_BOLD as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            ANTIALIASED_QUALITY as u32,
            DEFAULT_PITCH as u32,
            face.as_ptr(),
        );
        let old_font = SelectObject(dc, font);
        SetBkMode(dc, TRANSPARENT as i32);
        SetTextColor(dc, 0x00FF_FFFF);
        let mut rc = windows_sys::Win32::Foundation::RECT { left: 0, top: 0, right: size, bottom: size };
        let text = ui::wide("A");
        DrawTextW(dc, text.as_ptr(), -1, &mut rc, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
        GdiFlush();
        SelectObject(dc, old_font);
        SelectObject(dc, old_bitmap);
        DeleteObject(font);
        DeleteDC(dc);

        // GDI text rendering clobbers alpha, so apply it last.
        for (pixel, alpha) in pixels.iter_mut().zip(&coverage) {
            *pixel = (*pixel & 0x00FF_FFFF) | ((*alpha as u32) << 24);
        }

        let mask_bits = vec![0u8; (((size + 15) / 16) * 2 * size) as usize];
        let mask = CreateBitmap(size, size, 1, 1, mask_bits.as_ptr() as *const _);
        let info = ICONINFO {
            fIcon: 1,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        };
        let icon = CreateIconIndirect(&info);
        DeleteObject(mask);
        DeleteObject(color);
        if icon.is_null() {
            LoadIconW(null_mut(), IDI_APPLICATION)
        } else {
            icon
        }
    }
}

fn inside_rounded_square(x: f32, y: f32, size: f32, r: f32) -> bool {
    let cx = x.clamp(r, size - r);
    let cy = y.clamp(r, size - r);
    let (dx, dy) = (x - cx, y - cy);
    dx * dx + dy * dy <= r * r
}
