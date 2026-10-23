//! Text conversions for the selected text.

use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;

/// Lower case letters become upper case and the other way round.
pub fn invert_case(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_lowercase() {
            result.extend(c.to_uppercase());
        } else if c.is_uppercase() {
            result.extend(c.to_lowercase());
        } else {
            result.push(c);
        }
    }
    result
}

/// The text as if the same keys had been pressed in the layout `to` instead
/// of `from`. Characters that `from` doesn't type, or that need Ctrl or Alt
/// there, are kept; so are spaces and line breaks.
pub fn to_layout(text: &str, from: u32, to: u32) -> String {
    let from = from as i32 as isize as HKL;
    let to = to as i32 as isize as HKL;
    let mut result = String::with_capacity(text.len());
    for c in text.chars() {
        match (!c.is_whitespace()).then(|| unsafe { convert_char(c, from, to) }).flatten() {
            Some(converted) => result.push_str(&converted),
            None => result.push(c),
        }
    }
    result
}

unsafe fn convert_char(c: char, from: HKL, to: HKL) -> Option<String> {
    let mut unit = [0u16; 2];
    // Characters outside the BMP aren't on keyboards.
    let [unit] = c.encode_utf16(&mut unit) else { return None };
    let scan = VkKeyScanExW(*unit, from);
    if scan == -1 {
        return None;
    }
    let vk = (scan & 0xFF) as u32;
    let shift_state = (scan >> 8) & 0xFF;
    // Only plain and shifted keys; Ctrl/Alt combinations (AltGr) are left alone.
    if shift_state & !1 != 0 {
        return None;
    }
    let scan_code = MapVirtualKeyExW(vk, MAPVK_VK_TO_VSC, from);
    let to_vk = match MapVirtualKeyExW(scan_code, MAPVK_VSC_TO_VK_EX, to) {
        0 => vk,
        mapped => mapped,
    };
    let mut state = [0u8; 256];
    if shift_state & 1 != 0 {
        state[VK_SHIFT as usize] = 0x80;
    }
    let mut buf = [0u16; 8];
    // Flag 4 leaves the dead key state of this thread alone; a dead key
    // gives its own character (-1).
    let len = ToUnicodeEx(to_vk, scan_code, state.as_ptr(), buf.as_mut_ptr(), buf.len() as i32, 4, to);
    let len = (len.unsigned_abs() as usize).min(buf.len());
    (len > 0).then(|| String::from_utf16_lossy(&buf[..len]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_is_inverted_letter_by_letter() {
        assert_eq!(invert_case("hELLO, Мир 42"), "Hello, мИР 42");
    }
}
