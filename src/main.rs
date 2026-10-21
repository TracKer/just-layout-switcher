#![windows_subsystem = "windows"]

mod config;
mod hotkey_field;
mod icon;
mod keys;
mod layout_dialog;
mod layouts;
mod matcher;
mod reorder_list;
mod settings;
mod switcher;
mod ui;

use settings::Page;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicU32, Ordering};
use windows_sys::Win32::{
    Foundation::*,
    System::{LibraryLoader::GetModuleHandleW, Threading::CreateMutexW},
    UI::{HiDpi::*, Shell::*, WindowsAndMessaging::*},
};

const TRAY_CLASS_NAME: &str = "JlsTrayWindow";
const WM_TRAY: u32 = WM_APP + 1;
const TRAY_ID: u32 = 1;

const ID_SETTINGS: usize = 1;
const ID_ABOUT: usize = 2;
const ID_EXIT: usize = 3;

/// Broadcast by Explorer when the taskbar is (re)created.
static WM_TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

fn main() {
    unsafe {
        // Only one instance may live in the tray.
        let mutex_name = ui::wide("Local\\JustLayoutSwitcher.SingleInstance");
        let _mutex = CreateMutexW(std::ptr::null(), 0, mutex_name.as_ptr());
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return;
        }

        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_SYSTEM_AWARE);
        ui::init_dpi();
        let config = config::load(); // creates the default config file if missing

        ui::enable_visual_styles();
        hotkey_field::register();
        layout_dialog::register();
        reorder_list::register();
        settings::register();
        let Some(tray) = create_tray_window() else { return };
        add_tray_icon(tray);
        switcher::start(&config);

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            if layout_dialog::is_dialog_message(&msg) {
                continue;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

unsafe fn create_tray_window() -> Option<HWND> {
    let hinstance = GetModuleHandleW(std::ptr::null());
    let class = ui::wide(TRAY_CLASS_NAME);
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(tray_wnd_proc),
        hInstance: hinstance,
        lpszClassName: class.as_ptr(),
        ..std::mem::zeroed()
    };
    RegisterClassExW(&wc);

    let taskbar_created = ui::wide("TaskbarCreated");
    WM_TASKBAR_CREATED.store(RegisterWindowMessageW(taskbar_created.as_ptr()), Ordering::Relaxed);

    // A hidden top-level window (not message-only, so it gets TaskbarCreated).
    let hwnd = CreateWindowExW(
        0, class.as_ptr(), class.as_ptr(), WS_OVERLAPPED, 0, 0, 0, 0,
        null_mut(), null_mut(), hinstance, std::ptr::null(),
    );
    (!hwnd.is_null()).then_some(hwnd)
}

fn tray_data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = hwnd;
    data.uID = TRAY_ID;
    data
}

unsafe fn add_tray_icon(hwnd: HWND) {
    let mut data = tray_data(hwnd);
    data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    data.uCallbackMessage = WM_TRAY;
    data.hIcon = icon::create(GetSystemMetrics(SM_CXSMICON));
    let tip: Vec<u16> = settings::APP_NAME.encode_utf16().collect();
    data.szTip[..tip.len()].copy_from_slice(&tip);
    Shell_NotifyIconW(NIM_ADD, &data);
}

unsafe fn remove_tray_icon(hwnd: HWND) {
    Shell_NotifyIconW(NIM_DELETE, &tray_data(hwnd));
}

unsafe fn show_tray_menu(hwnd: HWND) {
    let menu = CreatePopupMenu();
    let settings_text = ui::wide("Settings");
    let about_text = ui::wide("About");
    let exit_text = ui::wide("Exit");
    AppendMenuW(menu, MF_STRING, ID_SETTINGS, settings_text.as_ptr());
    AppendMenuW(menu, MF_STRING, ID_ABOUT, about_text.as_ptr());
    AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
    AppendMenuW(menu, MF_STRING, ID_EXIT, exit_text.as_ptr());
    SetMenuDefaultItem(menu, ID_SETTINGS as u32, 0);

    let mut pt = POINT { x: 0, y: 0 };
    GetCursorPos(&mut pt);
    // Required so the menu closes when the user clicks elsewhere.
    SetForegroundWindow(hwnd);
    let cmd = TrackPopupMenu(
        menu,
        TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN | TPM_RIGHTALIGN,
        pt.x, pt.y, 0, hwnd, std::ptr::null(),
    );
    PostMessageW(hwnd, WM_NULL, 0, 0);
    DestroyMenu(menu);

    match cmd as usize {
        ID_SETTINGS => settings::open(Page::General),
        ID_ABOUT => settings::open(Page::About),
        ID_EXIT => {
            switcher::stop();
            settings::destroy();
            DestroyWindow(hwnd);
        }
        _ => {}
    }
}

unsafe extern "system" fn tray_wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TRAY => {
            match lparam as u32 {
                WM_RBUTTONUP => show_tray_menu(hwnd),
                WM_LBUTTONDBLCLK => settings::open(Page::General),
                _ => {}
            }
            0
        }
        WM_DESTROY => {
            remove_tray_icon(hwnd);
            PostQuitMessage(0);
            0
        }
        _ if msg == WM_TASKBAR_CREATED.load(Ordering::Relaxed) => {
            add_tray_icon(hwnd);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
