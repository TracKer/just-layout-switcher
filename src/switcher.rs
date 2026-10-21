//! Switches the keyboard layout by the global hotkey.
//!
//! A low-level keyboard hook feeds key events to the `Matcher`. The hook only
//! decides whether to pass an event on; the actual switching and the key
//! injection happen in a message-only window, so the hook returns quickly
//! (Windows silently drops hooks that are too slow).
//!
//! The next layout is taken from our own list: the configured order, skipping
//! disabled layouts and wrapping around from the last to the first. It is
//! requested from the focused window with `WM_INPUTLANGCHANGEREQUEST`, the
//! same way the system hotkey does it.

use crate::{config, hotkey_field, keys, layouts, matcher::Matcher, ui};
use std::cell::RefCell;
use std::ptr::null_mut;
use windows_sys::Win32::{
    Foundation::*,
    System::{LibraryLoader::GetModuleHandleW, SystemInformation::GetTickCount},
    UI::Input::KeyboardAndMouse::*,
    UI::WindowsAndMessaging::*,
};

const CLASS_NAME: &str = "JlsSwitcher";
const WM_SWITCH: u32 = WM_APP + 20;
/// wParam: vk of a key-up to replay after the mask (0 for none);
/// lParam: its scan code, bit 16 set for an extended key.
const WM_MASK: u32 = WM_APP + 21;

/// Marks the input we inject ourselves, so the hook lets it through.
const INJECTED_MARK: usize = 0x4A4C_5300;
/// An unassigned virtual key, used to break up modifier taps.
const VK_MASK: u16 = 0xE8;

/// How long a requested layout is assumed to be the current one while the
/// target window hasn't processed the request yet.
const PENDING_MS: u32 = 500;

struct Switcher {
    window: HWND,
    keyboard_hook: HHOOK,
    mouse_hook: HHOOK,
    matcher: Matcher,
    config: config::Config,
    /// The last requested layout: (GUI thread, HKL, tick).
    pending: Option<(u32, u32, u32)>,
}

thread_local! {
    static SWITCHER: RefCell<Switcher> = RefCell::new(Switcher {
        window: null_mut(),
        keyboard_hook: null_mut(),
        mouse_hook: null_mut(),
        matcher: Matcher::default(),
        config: config::Config::default(),
        pending: None,
    });
}

/// Creates the window and installs the keyboard hook. Must be called on the
/// thread that runs the message loop. The hotkey field installs its recording
/// hook later, so that one is called first and keeps the keys from us.
pub fn start(config: &config::Config) {
    let class = ui::wide(CLASS_NAME);
    unsafe {
        let hinstance = GetModuleHandleW(std::ptr::null());
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinstance,
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassExW(&wc);
        let window = CreateWindowExW(
            0, class.as_ptr(), class.as_ptr(), 0, 0, 0, 0, 0,
            HWND_MESSAGE, null_mut(), hinstance, std::ptr::null(),
        );
        let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), hinstance, 0);
        SWITCHER.with_borrow_mut(|s| {
            s.window = window;
            s.keyboard_hook = hook;
        });
    }
    apply(config);
}

/// Picks up a changed config.
pub fn apply(config: &config::Config) {
    // Only the first mode is implemented so far.
    let active = config.switch_mode == config::SwitchMode::Cycle;
    let hotkey = active.then(|| keys::parse(&config.hotkey)).flatten();
    let need_mouse = SWITCHER.with_borrow_mut(|s| {
        s.config = config.clone();
        s.matcher.set(hotkey, config.block_hotkey);
        s.matcher.fires_on_release()
    });
    set_mouse_hook(need_mouse);
}

pub fn stop() {
    set_mouse_hook(false);
    let (hook, window) = SWITCHER.with_borrow_mut(|s| {
        (std::mem::replace(&mut s.keyboard_hook, null_mut()), std::mem::replace(&mut s.window, null_mut()))
    });
    unsafe {
        if !hook.is_null() {
            UnhookWindowsHookEx(hook);
        }
        if !window.is_null() {
            DestroyWindow(window);
        }
    }
}

/// Clicks cancel a modifier-only hotkey (Shift+click selects text), so the
/// mouse is watched only while such a hotkey is set.
fn set_mouse_hook(on: bool) {
    let current = SWITCHER.with_borrow(|s| s.mouse_hook);
    unsafe {
        if on && current.is_null() {
            let hook = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), GetModuleHandleW(std::ptr::null()), 0);
            SWITCHER.with_borrow_mut(|s| s.mouse_hook = hook);
        } else if !on && !current.is_null() {
            UnhookWindowsHookEx(current);
            SWITCHER.with_borrow_mut(|s| s.mouse_hook = null_mut());
        }
    }
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && handle_key(&*(lparam as *const KBDLLHOOKSTRUCT)) {
        return 1;
    }
    CallNextHookEx(null_mut(), code, wparam, lparam)
}

/// Returns true if the key event must be swallowed.
fn handle_key(kb: &KBDLLHOOKSTRUCT) -> bool {
    if kb.flags & LLKHF_INJECTED != 0 && kb.dwExtraInfo == INJECTED_MARK {
        return false;
    }
    // The hook can be called from inside a message loop that runs while the
    // state is borrowed (e.g. a message box); just pass the key on then.
    let Ok(outcome) = SWITCHER.try_with(|s| {
        let mut s = s.try_borrow_mut().ok()?;
        if hotkey_field::is_capturing() {
            // The key belongs to the hotkey being recorded.
            s.matcher.reset();
            return None;
        }
        let up = kb.flags & LLKHF_UP != 0;
        let outcome = if up {
            s.matcher.key_up(kb.vkCode)
        } else {
            s.matcher.forget_released(|vk| unsafe { GetAsyncKeyState(vk as i32) } < 0);
            s.matcher.key_down(kb.vkCode)
        };
        Some((s.window, outcome))
    }) else {
        return false;
    };
    let Some((window, outcome)) = outcome else { return false };

    unsafe {
        if outcome.fire {
            PostMessageW(window, WM_SWITCH, 0, 0);
        }
        if outcome.mask {
            let (vk, scan) = if outcome.replay {
                let extended = if kb.flags & LLKHF_EXTENDED != 0 { 1 << 16 } else { 0 };
                (kb.vkCode as usize, (kb.scanCode | extended) as LPARAM)
            } else {
                (0, 0)
            };
            PostMessageW(window, WM_MASK, vk, scan);
        }
    }
    outcome.swallow
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32
        && matches!(
            wparam as u32,
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN | WM_MOUSEWHEEL | WM_MOUSEHWHEEL
        )
    {
        let _ = SWITCHER.try_with(|s| {
            if let Ok(mut s) = s.try_borrow_mut() {
                s.matcher.mouse_down();
            }
        });
    }
    CallNextHookEx(null_mut(), code, wparam, lparam)
}

/// Sends the mask key down and up, then the key-up we swallowed, if any.
unsafe fn send_mask(replay_vk: u16, replay_scan: u32) {
    let key = |vk: u16, scan: u16, flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: INJECTED_MARK },
        },
    };
    let mut inputs = vec![key(VK_MASK, 0, 0), key(VK_MASK, 0, KEYEVENTF_KEYUP)];
    if replay_vk != 0 {
        let extended = if replay_scan & (1 << 16) != 0 { KEYEVENTF_EXTENDEDKEY } else { 0 };
        inputs.push(key(replay_vk, replay_scan as u16, KEYEVENTF_KEYUP | extended));
    }
    SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32);
}

/// Switches the foreground application to the next enabled layout.
unsafe fn switch_layout() {
    let foreground = GetForegroundWindow();
    if foreground.is_null() {
        return;
    }
    let thread = GetWindowThreadProcessId(foreground, null_mut());
    let actual = GetKeyboardLayout(thread) as usize as u32;
    let now = GetTickCount();

    let (current, order) = SWITCHER.with_borrow(|s| {
        // Quick repeated presses may come before the window has processed
        // the previous request; continue from what was requested then.
        let current = match s.pending {
            Some((t, hkl, tick)) if t == thread && hkl != actual && now.wrapping_sub(tick) < PENDING_MS => hkl,
            _ => actual,
        };
        (current, s.config.layouts.clone())
    });
    let Some(next) = layouts::next_enabled(&layouts::switch_order(&order), current) else { return };

    // The focused control's window gets the request, like with the system
    // hotkey; fall back to the active and the foreground window.
    let mut info: GUITHREADINFO = std::mem::zeroed();
    info.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
    let target = if GetGUIThreadInfo(thread, &mut info) != 0 {
        [info.hwndFocus, info.hwndActive].into_iter().find(|h| !h.is_null()).unwrap_or(foreground)
    } else {
        foreground
    };
    // HKL handles are sign-extended 32-bit values.
    PostMessageW(target, WM_INPUTLANGCHANGEREQUEST, 0, next as i32 as LPARAM);
    SWITCHER.with_borrow_mut(|s| s.pending = Some((thread, next, now)));
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_SWITCH => {
            switch_layout();
            0
        }
        WM_MASK => {
            send_mask(wparam as u16, lparam as u32);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
