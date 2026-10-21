//! Per-layout options window, opened with the "more" button of a layout row.
//! It sets the hotkey that switches straight to the layout; Save writes it to
//! the config, Cancel drops the changes.

use crate::{config::HotkeyOwner, hotkey_field, settings, ui};
use std::cell::RefCell;
use std::ptr::null_mut;
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus},
    UI::WindowsAndMessaging::*,
};

const CLASS_NAME: &str = "JlsLayoutDialog";

const ID_HOTKEY: i32 = 100;
const ID_HOTKEY_ENABLED: i32 = 101;
const ID_HOTKEY_CLEAR: i32 = 102;

const CLIENT_W: i32 = 400;
const CLIENT_H: i32 = 196;
const MARGIN: i32 = 16;
const BUTTON_W: i32 = 88;
const BUTTON_H: i32 = 28;
const FIELD_W: i32 = 200;
const CLEAR_W: i32 = 70;

/// Static control style; not exposed by the enabled windows-sys features.
const SS_NOPREFIX: u32 = 0x80;

struct Dialog {
    hwnd: HWND,
    /// Id of the layout being edited.
    id: String,
    hotkey: HWND,
    hotkey_clear: HWND,
    hotkey_enabled: HWND,
    /// Static controls drawn in gray.
    secondary: Vec<HWND>,
    /// The combination as last accepted; restored when a new one clashes.
    accepted: String,
}

thread_local! {
    static DIALOG: RefCell<Option<Dialog>> = const { RefCell::new(None) };
}

pub fn register() {
    let class = ui::wide(CLASS_NAME);
    unsafe {
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wnd_proc),
            hInstance: GetModuleHandleW(std::ptr::null()),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: GetSysColorBrush(COLOR_WINDOW),
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassExW(&wc);
    }
}

fn hwnd() -> HWND {
    DIALOG
        .try_with(|d| d.try_borrow().ok().and_then(|d| d.as_ref().map(|d| d.hwnd)))
        .ok()
        .flatten()
        .unwrap_or(null_mut())
}

/// Lets the dialog handle Tab, Enter (Save) and Esc (Cancel).
pub fn is_dialog_message(msg: &MSG) -> bool {
    let hwnd = hwnd();
    !hwnd.is_null() && unsafe { IsDialogMessageW(hwnd, msg) } != 0
}

/// Opens the window modally over `owner` for the layout `id` named `layout_name`.
pub fn open(owner: HWND, id: &str, layout_name: &str) {
    if !hwnd().is_null() {
        return;
    }
    let (hotkey_text, enabled) = settings::layout_hotkey(id);
    unsafe {
        let style = WS_POPUP | WS_CAPTION | WS_SYSMENU;
        let ex_style = WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT;
        let mut rc = RECT { left: 0, top: 0, right: ui::scale(CLIENT_W), bottom: ui::scale(CLIENT_H) };
        AdjustWindowRectEx(&mut rc, style, 0, ex_style);
        let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
        let mut owner_rc: RECT = std::mem::zeroed();
        GetWindowRect(owner, &mut owner_rc);
        let x = owner_rc.left + (owner_rc.right - owner_rc.left - w) / 2;
        let y = owner_rc.top + (owner_rc.bottom - owner_rc.top - h) / 2;

        let hinstance = GetModuleHandleW(std::ptr::null());
        let class = ui::wide(CLASS_NAME);
        let title = ui::wide(layout_name);
        let hwnd = CreateWindowExW(
            ex_style, class.as_ptr(), title.as_ptr(), style, x, y, w, h,
            owner, null_mut(), hinstance, std::ptr::null(),
        );
        if hwnd.is_null() {
            return;
        }

        let font = ui::message_font(1.0, false);
        let child = |class: &str, text: &str, style: u32, x, y, w, h, id: i32| {
            let class = ui::wide(class);
            let text = ui::wide(text);
            let ctl = CreateWindowExW(
                0, class.as_ptr(), text.as_ptr(), WS_CHILD | WS_VISIBLE | style,
                ui::scale(x), ui::scale(y), ui::scale(w), ui::scale(h),
                hwnd, id as isize as HMENU, hinstance, std::ptr::null(),
            );
            ui::set_font(ctl, font);
            ctl
        };
        child("STATIC", "Hotkey for this layout:", SS_NOPREFIX, MARGIN, MARGIN, CLIENT_W - 2 * MARGIN, 20, 0);
        let hotkey = child(hotkey_field::CLASS_NAME, &hotkey_text, WS_TABSTOP, MARGIN, 40, FIELD_W, 30, ID_HOTKEY);
        let hotkey_clear = child(
            "BUTTON", "Clear", WS_TABSTOP | BS_PUSHBUTTON as u32,
            MARGIN + FIELD_W + 8, 40, CLEAR_W, 30, ID_HOTKEY_CLEAR,
        );
        let enabled_x = MARGIN + FIELD_W + 8 + CLEAR_W + 12;
        let hotkey_enabled = child(
            "BUTTON", "Enabled", WS_TABSTOP | BS_AUTOCHECKBOX as u32,
            enabled_x, 43, CLIENT_W - MARGIN - enabled_x, 24, ID_HOTKEY_ENABLED,
        );
        let hint = child(
            "STATIC",
            "Switches straight to this layout when the switching mode is \
             \u{201C}Use a separate hotkey for each layout\u{201D}. \
             A turned off hotkey is kept but not tracked; \
             a cleared one is removed.",
            SS_NOPREFIX, MARGIN, 80, CLIENT_W - 2 * MARGIN, 56, 0,
        );
        let buttons_y = CLIENT_H - MARGIN - BUTTON_H;
        let cancel_x = CLIENT_W - MARGIN - BUTTON_W;
        let save = child(
            "BUTTON", "Save", WS_TABSTOP | BS_DEFPUSHBUTTON as u32,
            cancel_x - 8 - BUTTON_W, buttons_y, BUTTON_W, BUTTON_H, IDOK,
        );
        child(
            "BUTTON", "Cancel", WS_TABSTOP | BS_PUSHBUTTON as u32,
            cancel_x, buttons_y, BUTTON_W, BUTTON_H, IDCANCEL,
        );
        ui::set_checked(hotkey_enabled, enabled);
        EnableWindow(hotkey, enabled as BOOL);
        ui::update_clear_button(hotkey, hotkey_clear);

        DIALOG.with_borrow_mut(|d| {
            *d = Some(Dialog {
                hwnd,
                id: id.to_string(),
                hotkey,
                hotkey_clear,
                hotkey_enabled,
                secondary: vec![hint],
                accepted: hotkey_text,
            })
        });

        EnableWindow(owner, 0);
        ShowWindow(hwnd, SW_SHOW);
        SetFocus(save);
    }
}

fn close(hwnd: HWND) {
    unsafe {
        // Re-enable the owner first so it gets activated when we go away.
        EnableWindow(GetWindow(hwnd, GW_OWNER), 1);
        DestroyWindow(hwnd);
    }
}

/// Takes a newly recorded combination unless something else uses it already.
fn on_hotkey_changed(hwnd: HWND) {
    let Some((id, hotkey, accepted)) =
        DIALOG.with_borrow(|d| d.as_ref().map(|d| (d.id.clone(), d.hotkey, d.accepted.clone())))
    else {
        return;
    };
    let text = ui::window_text(hotkey);
    match settings::hotkey_clash(&text, &HotkeyOwner::Layout(id)) {
        Some(message) => {
            ui::set_window_text(hotkey, &accepted);
            settings::show_hotkey_clash(hwnd, &message);
        }
        None => DIALOG.with_borrow_mut(|d| {
            if let Some(d) = d {
                d.accepted = text;
            }
        }),
    }
    update_clear();
}

fn update_clear() {
    if let Some((hotkey, clear)) = DIALOG.with_borrow(|d| d.as_ref().map(|d| (d.hotkey, d.hotkey_clear))) {
        ui::update_clear_button(hotkey, clear);
    }
}

fn on_save(hwnd: HWND) {
    let Some((id, hotkey, enabled)) = DIALOG.with_borrow(|d| {
        d.as_ref().map(|d| (d.id.clone(), d.accepted.clone(), ui::is_checked(d.hotkey_enabled)))
    }) else {
        return;
    };
    if settings::set_layout_hotkey(hwnd, &id, &hotkey, enabled) {
        close(hwnd);
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_COMMAND => {
            match (ui::loword(wparam) as i32, ui::hiword(wparam)) {
                (IDOK, _) => on_save(hwnd),
                (IDCANCEL, _) => close(hwnd),
                (ID_HOTKEY, hotkey_field::HFN_CHANGED) => on_hotkey_changed(hwnd),
                (ID_HOTKEY_CLEAR, BN_CLICKED) => {
                    if let Some(hotkey) = DIALOG.with_borrow(|d| d.as_ref().map(|d| d.hotkey)) {
                        ui::set_window_text(hotkey, "");
                        on_hotkey_changed(hwnd);
                        // The button turns disabled now; don't leave the focus on it.
                        SetFocus(hotkey);
                    }
                }
                (ID_HOTKEY_ENABLED, BN_CLICKED) => {
                    let enabled = ui::is_checked(lparam as HWND);
                    if let Some(hotkey) = DIALOG.with_borrow(|d| d.as_ref().map(|d| d.hotkey)) {
                        EnableWindow(hotkey, enabled as BOOL);
                    }
                    update_clear();
                }
                _ => {}
            }
            0
        }
        WM_LBUTTONDOWN => {
            // Take the focus so the hotkey field cancels recording.
            SetFocus(hwnd);
            0
        }
        WM_CLOSE => {
            close(hwnd);
            0
        }
        WM_CTLCOLORSTATIC => {
            let ctl = lparam as HWND;
            let secondary = DIALOG
                .try_with(|d| d.try_borrow().map(|d| d.as_ref().is_some_and(|d| d.secondary.contains(&ctl))))
                .ok()
                .and_then(Result::ok)
                .unwrap_or(false);
            let hdc = wparam as HDC;
            SetTextColor(hdc, GetSysColor(if secondary { COLOR_GRAYTEXT } else { COLOR_WINDOWTEXT }));
            SetBkColor(hdc, GetSysColor(COLOR_WINDOW));
            GetSysColorBrush(COLOR_WINDOW) as LRESULT
        }
        WM_DESTROY => {
            DIALOG.with_borrow_mut(|d| {
                if d.as_ref().is_some_and(|d| d.hwnd == hwnd) {
                    *d = None;
                }
            });
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
