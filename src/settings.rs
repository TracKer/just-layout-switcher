//! The settings window: a page list on the left, the selected page on the right.

use crate::{config, hotkey_field, icon, layout_dialog, layouts, reorder_list, switcher, ui};
use std::cell::RefCell;
use std::ptr::null_mut;
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::Controls::*,
    UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus},
    UI::WindowsAndMessaging::*,
};

const CLASS_NAME: &str = "JlsSettingsWindow";
pub const APP_NAME: &str = "Just Layout Switcher";

const ID_NAV: i32 = 100;
const ID_HOTKEY: i32 = 101;
const ID_LAYOUTS: i32 = 102;
const ID_BLOCK_HOTKEY: i32 = 103;
const ID_HOTKEY_CLEAR: i32 = 104;
const ID_CORRECTION_HOTKEY: i32 = 105;
const ID_CORRECTION_CLEAR: i32 = 106;
/// Radio buttons for the switch modes, in `MODES` order.
const ID_MODE_FIRST: i32 = 110;
const ID_MODE_LAST: i32 = ID_MODE_FIRST + MODES.len() as i32 - 1;

const MODES: [(config::SwitchMode, &str); 3] = [
    (config::SwitchMode::Cycle, "Switch to the next layout on each press"),
    (config::SwitchMode::PressCount, "Press once for the first layout, twice for the second, and so on"),
    (config::SwitchMode::PerLayout, "Use only the hotkeys of the layouts (set in the Layouts list)"),
];

const RECORD_HINT: &str = "Click the field and press a new key combination. \
     It is saved as soon as any of the keys is released.";

const CLIENT_W: i32 = 580;
const CLIENT_H: i32 = 400;
const NAV_W: i32 = 160;
const MARGIN: i32 = 16;
const CONTENT_X: i32 = MARGIN + NAV_W + 24;
const CONTENT_W: i32 = CLIENT_W - CONTENT_X - MARGIN;

#[derive(Clone, Copy, PartialEq)]
pub enum Page {
    General = 0,
    Layouts = 1,
    #[allow(dead_code)] // Opened from the page list only.
    Correction = 2,
    About = 3,
}

/// Static control style; not exposed by the enabled windows-sys features.
const SS_NOPREFIX: u32 = 0x80;

const PAGE_TITLES: [&str; 4] = ["General", "Layouts", "Correction", "About"];

struct Settings {
    hwnd: HWND,
    nav: HWND,
    modes: [HWND; 3],
    hotkey: HWND,
    block_hotkey: HWND,
    layouts: HWND,
    correction_hotkey: HWND,
    /// Static controls drawn in gray.
    secondary: Vec<HWND>,
    pages: [Vec<HWND>; 4],
    config: config::Config,
}

thread_local! {
    static SETTINGS: RefCell<Option<Settings>> = const { RefCell::new(None) };
}

pub fn register() {
    let class = ui::wide(CLASS_NAME);
    unsafe {
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wnd_proc),
            hInstance: GetModuleHandleW(std::ptr::null()),
            hIcon: icon::create(GetSystemMetrics(SM_CXICON)),
            hIconSm: icon::create(GetSystemMetrics(SM_CXSMICON)),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: GetSysColorBrush(COLOR_WINDOW),
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassExW(&wc);
    }
}

/// Shows the settings window (creating it on first use) on the given page.
pub fn open(page: Page) {
    if SETTINGS.with_borrow(|s| s.is_none()) {
        let Some(settings) = (unsafe { create() }) else { return };
        SETTINGS.with_borrow_mut(|s| *s = Some(settings));
    }

    // Pick up manual edits of the config file.
    let config = config::load();
    switcher::apply(&config);
    let (hwnd, nav, modes, hotkey, block_hotkey, correction_hotkey) = SETTINGS.with_borrow_mut(|s| {
        let s = s.as_mut().unwrap();
        s.config = config.clone();
        (s.hwnd, s.nav, s.modes, s.hotkey, s.block_hotkey, s.correction_hotkey)
    });
    show_mode(&modes, hotkey, config.switch_mode);
    ui::set_window_text(hotkey, &config.hotkey);
    ui::set_checked(block_hotkey, config.block_hotkey);
    ui::set_window_text(correction_hotkey, &config.correction_hotkey);

    unsafe {
        SendMessageW(nav, LB_SETCURSEL, page as usize, 0);
        show_page(page as usize);
        ShowWindow(hwnd, SW_SHOW);
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }
        SetForegroundWindow(hwnd);
        SetFocus(nav);
    }
}

pub fn destroy() {
    if let Some(hwnd) = SETTINGS.with_borrow(|s| s.as_ref().map(|s| s.hwnd)) {
        unsafe { DestroyWindow(hwnd) };
    }
}

fn show_page(index: usize) {
    if index == Page::Layouts as usize {
        refresh_layouts();
    }
    SETTINGS.with_borrow(|s| {
        let Some(s) = s else { return };
        for (i, page) in s.pages.iter().enumerate() {
            let cmd = if i == index { SW_SHOW } else { SW_HIDE };
            for &ctl in page {
                unsafe { ShowWindow(ctl, cmd) };
            }
        }
    });
}

/// Reads the layouts from the system and sorts them as the config says.
fn refresh_layouts() {
    let Some((list, order)) = SETTINGS.with_borrow(|s| s.as_ref().map(|s| (s.layouts, s.config.layouts.clone())))
    else {
        return;
    };
    let items = layouts::apply_order(layouts::system(), &order)
        .into_iter()
        .map(|l| reorder_list::Item { id: l.id, text: l.name, enabled: l.enabled })
        .collect();
    reorder_list::set_items(list, items);
}

unsafe fn create() -> Option<Settings> {
    let hinstance = GetModuleHandleW(std::ptr::null());
    let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN;
    let mut rc = RECT { left: 0, top: 0, right: ui::scale(CLIENT_W), bottom: ui::scale(CLIENT_H) };
    AdjustWindowRectEx(&mut rc, style, 0, 0);
    let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
    let x = (GetSystemMetrics(SM_CXSCREEN) - w) / 2;
    let y = (GetSystemMetrics(SM_CYSCREEN) - h) / 2;

    let class = ui::wide(CLASS_NAME);
    let title = ui::wide(&format!("{APP_NAME} \u{2014} Settings"));
    let hwnd = CreateWindowExW(
        0, class.as_ptr(), title.as_ptr(), style, x, y, w, h,
        null_mut(), null_mut(), hinstance, std::ptr::null(),
    );
    if hwnd.is_null() {
        return None;
    }

    let font = ui::message_font(1.0, false);
    let title_font = ui::message_font(1.6, true);
    let child = |class: &str, text: &str, style: u32, ex_style: u32, x, y, w, h, id: i32, font| {
        let class = ui::wide(class);
        let text = ui::wide(text);
        let ctl = CreateWindowExW(
            ex_style, class.as_ptr(), text.as_ptr(), WS_CHILD | style,
            ui::scale(x), ui::scale(y), ui::scale(w), ui::scale(h),
            hwnd, id as isize as HMENU, hinstance, std::ptr::null(),
        );
        ui::set_font(ctl, font);
        ctl
    };
    let label = |text: &str, y, h, font| child("STATIC", text, SS_NOPREFIX, 0, CONTENT_X, y, CONTENT_W, h, 0, font);

    let nav = child(
        "LISTBOX", "",
        WS_VISIBLE | WS_TABSTOP | (LBS_NOTIFY | LBS_OWNERDRAWFIXED | LBS_HASSTRINGS | LBS_NOINTEGRALHEIGHT) as u32,
        0, MARGIN, MARGIN, NAV_W, CLIENT_H - 2 * MARGIN, ID_NAV, font,
    );
    for title in PAGE_TITLES {
        let text = ui::wide(title);
        SendMessageW(nav, LB_ADDSTRING, 0, text.as_ptr() as LPARAM);
    }

    // General
    let general_title = label("General", MARGIN, 36, title_font);
    let mode_label = label("Switching mode:", 68, 20, font);
    let modes: [HWND; 3] = std::array::from_fn(|i| {
        // The group starts at the first button: Tab lands on the checked one,
        // the arrow keys move between them.
        let style = if i == 0 { WS_TABSTOP | WS_GROUP } else { 0 };
        let y = 92 + 26 * i as i32;
        child("BUTTON", MODES[i].1, style | BS_AUTORADIOBUTTON as u32, 0, CONTENT_X, y, CONTENT_W, 24, ID_MODE_FIRST + i as i32, font)
    });
    let hotkey_label = label("Layout switch hotkey:", 180, 20, font);
    let hotkey = child(
        hotkey_field::CLASS_NAME, "", WS_TABSTOP | WS_GROUP, 0, CONTENT_X, 204, 260, 30, ID_HOTKEY, font,
    );
    let hotkey_clear = child(
        "BUTTON", "Clear", WS_TABSTOP | BS_PUSHBUTTON as u32, 0, CONTENT_X + 268, 204, 80, 30, ID_HOTKEY_CLEAR, font,
    );
    let hotkey_hint = label(RECORD_HINT, 244, 40, font);
    let block_hotkey = child(
        "BUTTON", "Block the hotkey after switching",
        WS_TABSTOP | BS_AUTOCHECKBOX as u32, 0, CONTENT_X, 292, CONTENT_W, 24, ID_BLOCK_HOTKEY, font,
    );
    let block_hotkey_hint = label(
        "When on, the key combination is not passed on to the active application. \
         When off, it is passed on after switching.",
        320, 40, font,
    );

    // Layouts
    let layouts_title = label("Layouts", MARGIN, 36, title_font);
    let layouts_label = label("Drag layouts to set the switching order. Uncheck a layout to skip it:", 68, 20, font);
    let layouts = child(reorder_list::CLASS_NAME, "", 0, 0, CONTENT_X, 92, CONTENT_W, CLIENT_H - 92 - MARGIN, ID_LAYOUTS, font);

    // Correction
    let correction_title = label("Correction", MARGIN, 36, title_font);
    let correction_label = label("Last word correction hotkey:", 68, 20, font);
    let correction_hotkey = child(
        hotkey_field::CLASS_NAME, "", WS_TABSTOP, 0, CONTENT_X, 92, 260, 30, ID_CORRECTION_HOTKEY, font,
    );
    let correction_clear = child(
        "BUTTON", "Clear", WS_TABSTOP | BS_PUSHBUTTON as u32, 0, CONTENT_X + 268, 92, 80, 30, ID_CORRECTION_CLEAR, font,
    );
    let correction_hint = label(
        "Press it right after a word typed in the wrong layout: the word is erased \
         and typed again in the next layout. Press it again for the layout after that.",
        180, 60, font,
    );
    let correction_record_hint = label(RECORD_HINT, 132, 40, font);

    // About
    let about_title = label(APP_NAME, MARGIN, 36, title_font);
    let version = label(&format!("Version {}", env!("CARGO_PKG_VERSION")), 56, 20, font);
    let description = label("Switches the keyboard layout with a single, configurable hotkey.", 92, 40, font);
    let author = label(&format!("\u{00A9} 2026 {}", env!("CARGO_PKG_AUTHORS")), 140, 20, font);
    let license = label("Licensed under the GNU General Public License v3.0.", 164, 20, font);

    Some(Settings {
        hwnd,
        nav,
        modes,
        hotkey,
        block_hotkey,
        layouts,
        correction_hotkey,
        secondary: vec![hotkey_hint, block_hotkey_hint, correction_record_hint, version, license],
        pages: [
            vec![
                general_title, mode_label, modes[0], modes[1], modes[2],
                hotkey_label, hotkey, hotkey_clear, hotkey_hint, block_hotkey, block_hotkey_hint,
            ],
            vec![layouts_title, layouts_label, layouts],
            vec![correction_title, correction_label, correction_hotkey, correction_clear, correction_hint, correction_record_hint],
            vec![about_title, version, description, author, license],
        ],
        config: config::Config::default(),
    })
}

/// The layouts as the list shows them, keeping the hotkeys from `old`.
fn order_from_list(list: HWND, old: &[config::LayoutOrder]) -> Vec<config::LayoutOrder> {
    reorder_list::entries(list)
        .into_iter()
        .zip(1..)
        .map(|((id, enabled), order)| match old.iter().find(|o| o.id == id) {
            Some(o) => config::LayoutOrder { order, enabled, ..o.clone() },
            None => config::LayoutOrder::new(id, order, enabled),
        })
        .collect()
}

fn on_layouts_changed(list: HWND) {
    let result = SETTINGS.with_borrow_mut(|s| {
        let s = s.as_mut()?;
        s.config.layouts = order_from_list(list, &s.config.layouts);
        Some((s.hwnd, save(&s.config)))
    });
    if let Some((hwnd, Err(err))) = result {
        show_save_error(hwnd, &err);
    }
}

/// If `hotkey` is already used by something other than `owner`, returns
/// a message saying by what.
pub fn hotkey_clash(hotkey: &str, owner: &config::HotkeyOwner) -> Option<String> {
    let user = SETTINGS.with_borrow(|s| s.as_ref()?.config.hotkey_user(hotkey, owner))?;
    let what = match user {
        config::HotkeyOwner::Main => "the layout switch hotkey on the General page".to_string(),
        config::HotkeyOwner::Correction => "the last word correction hotkey on the Correction page".to_string(),
        config::HotkeyOwner::Layout(id) => {
            let name = layouts::system().into_iter().find(|l| l.id == id).map_or(id, |l| l.name);
            format!("the hotkey of the layout \u{201C}{name}\u{201D}")
        }
    };
    Some(format!("{hotkey} is already used as {what}.\n\nOne key combination can't do two things; choose another one."))
}

pub fn show_hotkey_clash(hwnd: HWND, message: &str) {
    let message = ui::wide(message);
    let caption = ui::wide(APP_NAME);
    unsafe { MessageBoxW(hwnd, message.as_ptr(), caption.as_ptr(), MB_ICONWARNING | MB_OK) };
}

/// The hotkey of a layout and whether it is turned on.
pub fn layout_hotkey(id: &str) -> (String, bool) {
    SETTINGS
        .with_borrow(|s| {
            let l = s.as_ref()?.config.layouts.iter().find(|l| l.id == id)?;
            Some((l.hotkey.clone(), l.hotkey_enabled))
        })
        .unwrap_or_else(|| (String::new(), true))
}

/// Saves the hotkey of a layout. Returns false if saving failed (the error
/// is shown over `dialog`).
pub fn set_layout_hotkey(dialog: HWND, id: &str, hotkey: &str, enabled: bool) -> bool {
    let result = SETTINGS.with_borrow_mut(|s| {
        let s = s.as_mut()?;
        // A layout gets its config entry once the order is saved; until then
        // take the order the list shows.
        let saved = s.config.layouts.clone();
        s.config.layouts = order_from_list(s.layouts, &s.config.layouts);
        let layout = s.config.layouts.iter_mut().find(|l| l.id == id)?;
        layout.hotkey = hotkey.to_string();
        layout.hotkey_enabled = enabled;
        let result = save(&s.config);
        if result.is_err() {
            s.config.layouts = saved;
        }
        Some(result)
    });
    match result {
        Some(Err(err)) => {
            show_save_error(dialog, &err);
            false
        }
        _ => true,
    }
}

/// Saves the config and makes the hotkey use it.
fn save(config: &config::Config) -> std::io::Result<()> {
    config::save(config)?;
    switcher::apply(config);
    Ok(())
}

fn show_save_error(hwnd: HWND, err: &std::io::Error) {
    let message = ui::wide(&format!("Failed to save {}:\n{err}", config::path().display()));
    let caption = ui::wide(APP_NAME);
    unsafe { MessageBoxW(hwnd, message.as_ptr(), caption.as_ptr(), MB_ICONERROR | MB_OK) };
}

/// The config entry of a hotkey set in this window (the common or the
/// correction hotkey).
fn hotkey_entry<'a>(config: &'a mut config::Config, owner: &config::HotkeyOwner) -> &'a mut String {
    match owner {
        config::HotkeyOwner::Correction => &mut config.correction_hotkey,
        _ => &mut config.hotkey,
    }
}

fn on_hotkey_cleared(owner: config::HotkeyOwner) {
    let field = SETTINGS.with_borrow(|s| {
        let s = s.as_ref()?;
        Some(if owner == config::HotkeyOwner::Correction { s.correction_hotkey } else { s.hotkey })
    });
    if let Some(field) = field {
        ui::set_window_text(field, "");
        on_hotkey_changed(field, owner);
    }
}

fn on_hotkey_changed(field: HWND, owner: config::HotkeyOwner) {
    let text = ui::window_text(field);
    if let Some(message) = hotkey_clash(&text, &owner) {
        let (hwnd, saved) = SETTINGS.with_borrow_mut(|s| {
            let s = s.as_mut().unwrap();
            (s.hwnd, hotkey_entry(&mut s.config, &owner).clone())
        });
        ui::set_window_text(field, &saved);
        show_hotkey_clash(hwnd, &message);
        return;
    }
    let result = SETTINGS.with_borrow_mut(|s| {
        let s = s.as_mut()?;
        let saved = std::mem::replace(hotkey_entry(&mut s.config, &owner), text);
        let result = save(&s.config);
        if result.is_err() {
            *hotkey_entry(&mut s.config, &owner) = saved;
        }
        Some((s.hwnd, hotkey_entry(&mut s.config, &owner).clone(), result))
    });
    if let Some((hwnd, saved, Err(err))) = result {
        // Show what is actually in the config.
        ui::set_window_text(field, &saved);
        show_save_error(hwnd, &err);
    }
}

/// Checks the radio button of `mode`. The common hotkey is unused when
/// every layout has its own, so its field is disabled then.
fn show_mode(modes: &[HWND; 3], hotkey: HWND, mode: config::SwitchMode) {
    for (&radio, (m, _)) in modes.iter().zip(MODES) {
        ui::set_checked(radio, m == mode);
    }
    unsafe { EnableWindow(hotkey, (mode != config::SwitchMode::PerLayout) as BOOL) };
}

fn on_mode_changed(index: usize) {
    let mode = MODES[index].0;
    let result = SETTINGS.with_borrow_mut(|s| {
        let s = s.as_mut()?;
        let saved = std::mem::replace(&mut s.config.switch_mode, mode);
        let result = save(&s.config);
        if result.is_err() {
            s.config.switch_mode = saved;
        }
        Some((s.hwnd, s.modes, s.hotkey, s.config.switch_mode, result))
    });
    let Some((hwnd, modes, hotkey, mode, result)) = result else { return };
    // Show what is actually in the config.
    show_mode(&modes, hotkey, mode);
    if let Err(err) = result {
        show_save_error(hwnd, &err);
    }
}

fn on_block_hotkey_changed(checkbox: HWND) {
    let block = ui::is_checked(checkbox);
    let result = SETTINGS.with_borrow_mut(|s| {
        let s = s.as_mut()?;
        s.config.block_hotkey = block;
        let result = save(&s.config);
        if result.is_err() {
            s.config.block_hotkey = !block;
        }
        Some((s.hwnd, result))
    });
    if let Some((hwnd, Err(err))) = result {
        ui::set_checked(checkbox, !block);
        show_save_error(hwnd, &err);
    }
}

unsafe fn draw_nav_item(item: &DRAWITEMSTRUCT) {
    if item.itemID == u32::MAX {
        return;
    }
    let selected = item.itemState & ODS_SELECTED != 0;
    let mut rc = item.rcItem;
    FillRect(item.hDC, &rc, GetSysColorBrush(COLOR_WINDOW));
    if selected {
        let mut sel = rc;
        InflateRect(&mut sel, 0, -ui::scale(2));
        let brush = GetSysColorBrush(COLOR_HIGHLIGHT);
        FillRect(item.hDC, &sel, brush);
    }

    let mut buf = [0u16; 64];
    let len = SendMessageW(item.hwndItem, LB_GETTEXT, item.itemID as usize, buf.as_mut_ptr() as LPARAM);
    if len <= 0 {
        return;
    }
    SetBkMode(item.hDC, TRANSPARENT as i32);
    SetTextColor(
        item.hDC,
        GetSysColor(if selected { COLOR_HIGHLIGHTTEXT } else { COLOR_WINDOWTEXT }),
    );
    rc.left += ui::scale(12);
    DrawTextW(item.hDC, buf.as_ptr(), len as i32, &mut rc, DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX);
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_CLOSE => {
            // Keep the window around; the app lives in the tray.
            ShowWindow(hwnd, SW_HIDE);
            0
        }
        WM_LBUTTONDOWN => {
            // Clicks on the background or labels don't move focus by
            // themselves; take it so the hotkey field cancels recording.
            SetFocus(hwnd);
            0
        }
        WM_COMMAND => {
            let id = ui::loword(wparam) as i32;
            let code = ui::hiword(wparam);
            match (id, code) {
                (ID_NAV, LBN_SELCHANGE) => {
                    let index = SendMessageW(lparam as HWND, LB_GETCURSEL, 0, 0);
                    if index >= 0 {
                        show_page(index as usize);
                    }
                }
                (ID_HOTKEY, hotkey_field::HFN_CHANGED) => on_hotkey_changed(lparam as HWND, config::HotkeyOwner::Main),
                (ID_HOTKEY_CLEAR, BN_CLICKED) => on_hotkey_cleared(config::HotkeyOwner::Main),
                (ID_CORRECTION_HOTKEY, hotkey_field::HFN_CHANGED) => {
                    on_hotkey_changed(lparam as HWND, config::HotkeyOwner::Correction)
                }
                (ID_CORRECTION_CLEAR, BN_CLICKED) => on_hotkey_cleared(config::HotkeyOwner::Correction),
                (ID_MODE_FIRST..=ID_MODE_LAST, BN_CLICKED) => on_mode_changed((id - ID_MODE_FIRST) as usize),
                (ID_BLOCK_HOTKEY, BN_CLICKED) => on_block_hotkey_changed(lparam as HWND),
                (ID_LAYOUTS, reorder_list::RLN_CHANGED) => on_layouts_changed(lparam as HWND),
                (ID_LAYOUTS, reorder_list::RLN_MORE) => {
                    if let Some((id, name)) = reorder_list::active_item(lparam as HWND) {
                        layout_dialog::open(hwnd, &id, &name);
                    }
                }
                _ => {}
            }
            0
        }
        WM_MEASUREITEM => {
            let item = &mut *(lparam as *mut MEASUREITEMSTRUCT);
            item.itemHeight = ui::scale(32) as u32;
            1
        }
        WM_DRAWITEM => {
            draw_nav_item(&*(lparam as *const DRAWITEMSTRUCT));
            1
        }
        WM_CTLCOLORSTATIC => {
            let hdc = wparam as HDC;
            let ctl = lparam as HWND;
            let secondary = SETTINGS
                .try_with(|s| s.try_borrow().map(|s| s.as_ref().is_some_and(|s| s.secondary.contains(&ctl))))
                .ok()
                .and_then(Result::ok)
                .unwrap_or(false);
            SetTextColor(hdc, GetSysColor(if secondary { COLOR_GRAYTEXT } else { COLOR_WINDOWTEXT }));
            SetBkColor(hdc, GetSysColor(COLOR_WINDOW));
            GetSysColorBrush(COLOR_WINDOW) as LRESULT
        }
        WM_DESTROY => {
            SETTINGS.with_borrow_mut(|s| *s = None);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
