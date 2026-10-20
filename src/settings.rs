//! The settings window: a page list on the left, the selected page on the right.

use crate::{config, hotkey_field, icon, ui};
use std::cell::RefCell;
use std::ptr::null_mut;
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::Controls::*,
    UI::Input::KeyboardAndMouse::SetFocus,
    UI::WindowsAndMessaging::*,
};

const CLASS_NAME: &str = "JlsSettingsWindow";
pub const APP_NAME: &str = "Just Layout Switcher";

const ID_NAV: i32 = 100;
const ID_HOTKEY: i32 = 101;

const CLIENT_W: i32 = 580;
const CLIENT_H: i32 = 360;
const NAV_W: i32 = 160;
const MARGIN: i32 = 16;
const CONTENT_X: i32 = MARGIN + NAV_W + 24;
const CONTENT_W: i32 = CLIENT_W - CONTENT_X - MARGIN;

#[derive(Clone, Copy, PartialEq)]
pub enum Page {
    General = 0,
    About = 1,
}

/// Static control style; not exposed by the enabled windows-sys features.
const SS_NOPREFIX: u32 = 0x80;

const PAGE_TITLES: [&str; 2] = ["General", "About"];

struct Settings {
    hwnd: HWND,
    nav: HWND,
    hotkey: HWND,
    /// Static controls drawn in gray.
    secondary: Vec<HWND>,
    pages: [Vec<HWND>; 2],
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
    let (hwnd, nav, hotkey) = SETTINGS.with_borrow_mut(|s| {
        let s = s.as_mut().unwrap();
        s.config = config.clone();
        (s.hwnd, s.nav, s.hotkey)
    });
    ui::set_window_text(hotkey, &config.hotkey);

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
    let hotkey_label = label("Layout switch hotkey:", 68, 20, font);
    let hotkey = child(hotkey_field::CLASS_NAME, "", WS_TABSTOP, 0, CONTENT_X, 92, 260, 30, ID_HOTKEY, font);
    let hotkey_hint = label(
        "Click the field and press a new key combination. \
         It is saved as soon as any of the keys is released.",
        132, 40, font,
    );

    // About
    let about_title = label(APP_NAME, MARGIN, 36, title_font);
    let version = label(&format!("Version {}", env!("CARGO_PKG_VERSION")), 56, 20, font);
    let description = label("Switches the keyboard layout with a single, configurable hotkey.", 92, 40, font);
    let author = label(&format!("\u{00A9} 2026 {}", env!("CARGO_PKG_AUTHORS")), 140, 20, font);
    let license = label("Licensed under the GNU General Public License v3.0.", 164, 20, font);

    Some(Settings {
        hwnd,
        nav,
        hotkey,
        secondary: vec![hotkey_hint, version, license],
        pages: [
            vec![general_title, hotkey_label, hotkey, hotkey_hint],
            vec![about_title, version, description, author, license],
        ],
        config: config::Config::default(),
    })
}

fn on_hotkey_changed(hotkey: HWND) {
    let text = ui::window_text(hotkey);
    let result = SETTINGS.with_borrow_mut(|s| {
        let s = s.as_mut()?;
        s.config.hotkey = text;
        Some((s.hwnd, config::save(&s.config)))
    });
    if let Some((hwnd, Err(err))) = result {
        let message = ui::wide(&format!(
            "Failed to save {}:\n{err}",
            config::path().display()
        ));
        let caption = ui::wide(APP_NAME);
        unsafe { MessageBoxW(hwnd, message.as_ptr(), caption.as_ptr(), MB_ICONERROR | MB_OK) };
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
                (ID_HOTKEY, hotkey_field::HFN_CHANGED) => on_hotkey_changed(lparam as HWND),
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
