use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;

/// Maps left/right variants of modifiers onto one generic key.
pub fn normalize(vk: u32) -> u32 {
    match vk as u16 {
        VK_LCONTROL | VK_RCONTROL => VK_CONTROL as u32,
        VK_LMENU | VK_RMENU => VK_MENU as u32,
        VK_LSHIFT | VK_RSHIFT => VK_SHIFT as u32,
        VK_RWIN => VK_LWIN as u32,
        _ => vk,
    }
}

/// Position of a (normalized) modifier in a formatted combination.
fn modifier_rank(vk: u32) -> Option<u8> {
    match vk as u16 {
        VK_CONTROL => Some(0),
        VK_MENU => Some(1),
        VK_SHIFT => Some(2),
        VK_LWIN => Some(3),
        _ => None,
    }
}

/// Display name of a normalized virtual key. `scan` and `extended` come from
/// the keyboard hook and are used for keys without a well-known name.
pub fn name(vk: u32, scan: u32, extended: bool) -> String {
    let fixed = match vk as u16 {
        VK_CONTROL => "Ctrl",
        VK_MENU => "Alt",
        VK_SHIFT => "Shift",
        VK_LWIN => "Win",
        VK_SPACE => "Space",
        VK_RETURN => "Enter",
        VK_ESCAPE => "Esc",
        VK_TAB => "Tab",
        VK_BACK => "Backspace",
        VK_DELETE => "Delete",
        VK_INSERT => "Insert",
        VK_HOME => "Home",
        VK_END => "End",
        VK_PRIOR => "PageUp",
        VK_NEXT => "PageDown",
        VK_LEFT => "Left",
        VK_RIGHT => "Right",
        VK_UP => "Up",
        VK_DOWN => "Down",
        VK_CAPITAL => "CapsLock",
        VK_NUMLOCK => "NumLock",
        VK_SCROLL => "ScrollLock",
        VK_PAUSE => "Pause",
        VK_SNAPSHOT => "PrintScreen",
        VK_APPS => "Menu",
        VK_MULTIPLY => "Num*",
        VK_ADD => "NumPlus",
        VK_SUBTRACT => "Num-",
        VK_DIVIDE => "Num/",
        VK_DECIMAL => "Num.",
        VK_OEM_1 => ";",
        VK_OEM_PLUS => "=",
        VK_OEM_COMMA => ",",
        VK_OEM_MINUS => "-",
        VK_OEM_PERIOD => ".",
        VK_OEM_2 => "/",
        VK_OEM_3 => "`",
        VK_OEM_4 => "[",
        VK_OEM_5 => "\\",
        VK_OEM_6 => "]",
        VK_OEM_7 => "'",
        _ => "",
    };
    if !fixed.is_empty() {
        return fixed.to_string();
    }

    let vk16 = vk as u16;
    if (0x30..=0x39).contains(&vk16) || (0x41..=0x5A).contains(&vk16) {
        return char::from(vk as u8).to_string();
    }
    if (VK_F1..=VK_F24).contains(&vk16) {
        return format!("F{}", vk16 - VK_F1 + 1);
    }
    if (VK_NUMPAD0..=VK_NUMPAD9).contains(&vk16) {
        return format!("Num{}", vk16 - VK_NUMPAD0);
    }

    let lparam = ((scan as i32) << 16) | if extended { 1 << 24 } else { 0 };
    let mut buf = [0u16; 64];
    let len = unsafe { GetKeyNameTextW(lparam, buf.as_mut_ptr(), buf.len() as i32) };
    if len > 0 {
        String::from_utf16_lossy(&buf[..len as usize])
    } else {
        format!("VK{vk:02X}")
    }
}

/// Formats keys as "Ctrl + Alt + K": modifiers first in a fixed order,
/// everything else in the order it was pressed.
pub fn format(keys: &[(u32, String)]) -> String {
    let mut mods: Vec<_> = keys
        .iter()
        .filter_map(|(vk, name)| modifier_rank(*vk).map(|rank| (rank, name.as_str())))
        .collect();
    mods.sort_by_key(|(rank, _)| *rank);

    mods.into_iter()
        .map(|(_, name)| name)
        .chain(
            keys.iter()
                .filter(|(vk, _)| modifier_rank(*vk).is_none())
                .map(|(_, name)| name.as_str()),
        )
        .collect::<Vec<_>>()
        .join(" + ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(vk: u16) -> (u32, String) {
        (vk as u32, name(vk as u32, 0, false))
    }

    #[test]
    fn modifiers_come_first_in_fixed_order() {
        let keys = [key(b'K' as u16), key(VK_SHIFT), key(VK_LWIN), key(VK_CONTROL), key(VK_MENU)];
        assert_eq!(format(&keys), "Ctrl + Alt + Shift + Win + K");
    }

    #[test]
    fn other_keys_keep_press_order() {
        let keys = [key(VK_CONTROL), key(VK_F5), key(b'1' as u16)];
        assert_eq!(format(&keys), "Ctrl + F5 + 1");
    }

    #[test]
    fn left_and_right_modifiers_are_merged() {
        assert_eq!(normalize(VK_RCONTROL as u32), VK_CONTROL as u32);
        assert_eq!(normalize(VK_LMENU as u32), VK_MENU as u32);
        assert_eq!(normalize(VK_RWIN as u32), VK_LWIN as u32);
    }
}
