//! Keyboard layouts installed in the system.

use crate::{config::LayoutOrder, ui};
use std::ptr::null_mut;
use windows_sys::Win32::{
    Globalization::{GetLocaleInfoW, LOCALE_SLOCALIZEDDISPLAYNAME},
    System::Registry::*,
    UI::Input::KeyboardAndMouse::GetKeyboardLayoutList,
};

const LAYOUTS_KEY: &str = r"SYSTEM\CurrentControlSet\Control\Keyboard Layouts";

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// HKL as 8 hex digits; stored in the config.
    pub id: String,
    pub name: String,
    pub enabled: bool,
}

/// Layouts the user can switch between, in the system order.
pub fn system() -> Vec<Layout> {
    unsafe {
        let count = GetKeyboardLayoutList(0, null_mut());
        let mut hkls = vec![null_mut(); count.max(0) as usize];
        let count = GetKeyboardLayoutList(hkls.len() as i32, hkls.as_mut_ptr());
        hkls.truncate(count.max(0) as usize);
        hkls.into_iter()
            .map(|hkl| {
                let hkl = hkl as usize as u32;
                Layout {
                    id: format!("{hkl:08X}"),
                    name: display_name(hkl),
                    enabled: true,
                }
            })
            .collect()
    }
}

/// Sorts layouts by the configured order and applies their enabled state.
/// Layouts missing from the config are enabled, keep their system order and
/// go after the configured ones.
pub fn apply_order(mut layouts: Vec<Layout>, order: &[LayoutOrder]) -> Vec<Layout> {
    let configured = |layout: &Layout| order.iter().find(|o| o.id == layout.id);
    for layout in &mut layouts {
        layout.enabled = configured(layout).is_none_or(|o| o.enabled);
    }
    layouts.sort_by_key(|layout| configured(layout).map_or(u32::MAX, |o| o.order));
    layouts
}

/// "English (United States) — US"; just the language if the layout is named the same.
fn display_name(hkl: u32) -> String {
    let lang = hkl & 0xFFFF;
    let language = language_name(lang).unwrap_or_else(|| format!("{hkl:08X}"));
    match layout_klid(hkl).and_then(|klid| layout_name(&klid)) {
        Some(layout) if layout != language => format!("{language} \u{2014} {layout}"),
        _ => language,
    }
}

fn language_name(lang: u32) -> Option<String> {
    let mut buf = [0u16; 128];
    let len = unsafe { GetLocaleInfoW(lang, LOCALE_SLOCALIZEDDISPLAYNAME, buf.as_mut_ptr(), buf.len() as i32) };
    (len > 1).then(|| String::from_utf16_lossy(&buf[..len as usize - 1]))
}

/// Registry name of the layout (KLID), e.g. "00000409".
fn layout_klid(hkl: u32) -> Option<String> {
    let device = hkl >> 16;
    if device & 0xF000 != 0xF000 {
        let id = if device == 0 { hkl & 0xFFFF } else { device };
        return Some(format!("{id:08X}"));
    }
    // A layout variant: find the KLID by its "Layout Id".
    let layout_id = format!("{:04X}", device & 0x0FFF);
    let root = RegKey::open(HKEY_LOCAL_MACHINE, LAYOUTS_KEY)?;
    (0..)
        .map_while(|i| root.subkey_name(i))
        .find(|klid| {
            RegKey::open(root.0, klid)
                .and_then(|key| key.string("Layout Id"))
                .is_some_and(|id| id.eq_ignore_ascii_case(&layout_id))
        })
}

fn layout_name(klid: &str) -> Option<String> {
    let key = RegKey::open(HKEY_LOCAL_MACHINE, &format!(r"{LAYOUTS_KEY}\{klid}"))?;
    key.mui_string("Layout Display Name")
        .or_else(|| key.string("Layout Text"))
}

struct RegKey(HKEY);

impl RegKey {
    fn open(parent: HKEY, path: &str) -> Option<Self> {
        let path = ui::wide(path);
        let mut key = null_mut();
        let status = unsafe { RegOpenKeyExW(parent, path.as_ptr(), 0, KEY_READ, &mut key) };
        (status == 0).then_some(Self(key))
    }

    fn subkey_name(&self, index: u32) -> Option<String> {
        let mut buf = [0u16; 256];
        let mut len = buf.len() as u32;
        let status = unsafe {
            RegEnumKeyExW(
                self.0, index, buf.as_mut_ptr(), &mut len,
                null_mut(), null_mut(), null_mut(), null_mut(),
            )
        };
        (status == 0).then(|| String::from_utf16_lossy(&buf[..len as usize]))
    }

    fn string(&self, name: &str) -> Option<String> {
        let name = ui::wide(name);
        let mut buf = [0u16; 256];
        let mut size = std::mem::size_of_val(&buf) as u32;
        let status = unsafe {
            RegGetValueW(
                self.0, std::ptr::null(), name.as_ptr(), RRF_RT_REG_SZ,
                null_mut(), buf.as_mut_ptr() as *mut _, &mut size,
            )
        };
        (status == 0).then(|| from_wide(&buf))
    }

    /// A localized string such as "@%SystemRoot%\system32\input.dll,-5000".
    fn mui_string(&self, name: &str) -> Option<String> {
        let name = ui::wide(name);
        let mut buf = [0u16; 256];
        let status = unsafe {
            RegLoadMUIStringW(
                self.0, name.as_ptr(), buf.as_mut_ptr(), std::mem::size_of_val(&buf) as u32,
                null_mut(), 0, std::ptr::null(),
            )
        };
        (status == 0).then(|| from_wide(&buf)).filter(|s| !s.is_empty())
    }
}

impl Drop for RegKey {
    fn drop(&mut self) {
        unsafe { RegCloseKey(self.0) };
    }
}

fn from_wide(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(id: &str) -> Layout {
        Layout { id: id.to_string(), name: String::new(), enabled: true }
    }

    fn order(id: &str, order: u32) -> LayoutOrder {
        LayoutOrder { id: id.to_string(), order, enabled: true }
    }

    fn ids(layouts: &[Layout]) -> Vec<&str> {
        layouts.iter().map(|l| l.id.as_str()).collect()
    }

    #[test]
    fn no_config_keeps_system_order() {
        let sorted = apply_order(vec![layout("A"), layout("B"), layout("C")], &[]);
        assert_eq!(ids(&sorted), ["A", "B", "C"]);
    }

    #[test]
    fn configured_order_is_applied_and_unknown_layouts_go_last() {
        let sorted = apply_order(
            vec![layout("A"), layout("B"), layout("C"), layout("D")],
            &[order("C", 1), order("A", 2), order("GONE", 3)],
        );
        assert_eq!(ids(&sorted), ["C", "A", "B", "D"]);
    }

    #[test]
    fn enabled_state_comes_from_config_and_new_layouts_are_enabled() {
        let sorted = apply_order(
            vec![layout("A"), layout("B")],
            &[LayoutOrder { enabled: false, ..order("A", 1) }],
        );
        let enabled: Vec<_> = sorted.iter().map(|l| (l.id.as_str(), l.enabled)).collect();
        assert_eq!(enabled, [("A", false), ("B", true)]);
    }

    #[test]
    fn missing_enabled_field_means_enabled() {
        let config: crate::config::Config =
            serde_yaml::from_str("layouts:\n- id: '04090409'\n  order: 1\n").unwrap();
        assert!(config.layouts[0].enabled);
    }
}
