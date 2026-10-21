//! Per-layout options window, opened with the "more" button of a layout row.
//! For now it is a stub with Save and Cancel buttons that just close it.

use crate::ui;
use std::cell::Cell;
use std::ptr::null_mut;
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus},
    UI::WindowsAndMessaging::*,
};

const CLASS_NAME: &str = "JlsLayoutDialog";

const CLIENT_W: i32 = 380;
const CLIENT_H: i32 = 150;
const MARGIN: i32 = 16;
const BUTTON_W: i32 = 88;
const BUTTON_H: i32 = 28;

thread_local! {
    static DIALOG: Cell<HWND> = const { Cell::new(null_mut()) };
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

/// Lets the dialog handle Tab, Enter (Save) and Esc (Cancel).
pub fn is_dialog_message(msg: &MSG) -> bool {
    let hwnd = DIALOG.get();
    !hwnd.is_null() && unsafe { IsDialogMessageW(hwnd, msg) } != 0
}

/// Opens the window modally over `owner` for the layout named `layout_name`.
pub fn open(owner: HWND, layout_name: &str) {
    if !DIALOG.get().is_null() {
        return;
    }
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
        DIALOG.set(hwnd);

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
        child(
            "STATIC", "Layout options will be here.", 0,
            MARGIN, MARGIN, CLIENT_W - 2 * MARGIN, 40, 0,
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

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_COMMAND => {
            match ui::loword(wparam) as i32 {
                // Nothing to save yet.
                IDOK | IDCANCEL => close(hwnd),
                _ => {}
            }
            0
        }
        WM_CLOSE => {
            close(hwnd);
            0
        }
        WM_CTLCOLORSTATIC => {
            SetBkColor(wparam as HDC, GetSysColor(COLOR_WINDOW));
            GetSysColorBrush(COLOR_WINDOW) as LRESULT
        }
        WM_DESTROY => {
            if DIALOG.get() == hwnd {
                DIALOG.set(null_mut());
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
