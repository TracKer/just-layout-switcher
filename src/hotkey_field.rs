//! A custom control that shows a key combination and records a new one.
//!
//! Clicking the control clears it and starts recording. Keys are captured with
//! a low-level keyboard hook (so Win, Alt, Alt+Tab etc. don't reach the system
//! while recording) and appear one by one as they are pressed. As soon as any
//! of the pressed keys is released, the combination is committed: the control
//! updates its window text and notifies the parent with
//! `WM_COMMAND(id, HFN_CHANGED)`.

use crate::{keys, ui};
use std::cell::RefCell;
use std::ptr::null_mut;
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::Input::KeyboardAndMouse::{GetAsyncKeyState, SetFocus},
    UI::WindowsAndMessaging::*,
};

pub const CLASS_NAME: &str = "JlsHotkeyField";
/// Notification code sent to the parent when a new combination is committed.
pub const HFN_CHANGED: u32 = 1;

const WM_REC_COMMIT: u32 = WM_APP + 10;
const WM_REC_UNHOOK: u32 = WM_APP + 11;

#[derive(PartialEq)]
enum Phase {
    Idle,
    /// Collecting keys.
    Recording,
    /// Committed, but some keys are still down: swallow them until released
    /// so the system never sees an unmatched key-up (e.g. Win opening Start).
    Draining,
}

struct Recorder {
    field: HWND,
    hook: HHOOK,
    phase: Phase,
    /// Keys of the combination in press order (normalized vk, name).
    keys: Vec<(u32, String)>,
    /// Keys we swallowed the key-down of and that are still held.
    held: Vec<u32>,
    /// Keys that were already down when recording started; the system saw
    /// their key-down, so their events are passed through untouched.
    pre_held: Vec<u32>,
}

thread_local! {
    static REC: RefCell<Recorder> = const {
        RefCell::new(Recorder {
            field: null_mut(),
            hook: null_mut(),
            phase: Phase::Idle,
            keys: Vec::new(),
            held: Vec::new(),
            pre_held: Vec::new(),
        })
    };
}

pub fn register() {
    let class = ui::wide(CLASS_NAME);
    unsafe {
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            hInstance: GetModuleHandleW(std::ptr::null()),
            hCursor: LoadCursorW(null_mut(), IDC_HAND),
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassExW(&wc);
    }
}

fn is_recording(hwnd: HWND) -> bool {
    REC.with_borrow(|r| r.field == hwnd && r.phase == Phase::Recording)
}

fn start_recording(hwnd: HWND) {
    let pre_held = (8..=254u32)
        .filter(|&vk| unsafe { GetAsyncKeyState(vk as i32) } < 0)
        .map(keys::normalize)
        .collect();

    let need_hook = REC.with_borrow_mut(|r| {
        r.field = hwnd;
        r.phase = Phase::Recording;
        r.keys.clear();
        r.held.clear();
        r.pre_held = pre_held;
        r.hook.is_null()
    });
    if need_hook {
        let hook = unsafe {
            SetWindowsHookExW(
                WH_KEYBOARD_LL,
                Some(keyboard_hook),
                GetModuleHandleW(std::ptr::null()),
                0,
            )
        };
        REC.with_borrow_mut(|r| r.hook = hook);
    }
    unsafe { InvalidateRect(hwnd, std::ptr::null(), 1) };
}

/// Stops recording without committing anything.
fn cancel_recording(hwnd: HWND) {
    let was_recording = REC.with_borrow_mut(|r| {
        if r.field != hwnd || r.phase != Phase::Recording {
            return false;
        }
        r.phase = Phase::Idle;
        r.keys.clear();
        true
    });
    if was_recording {
        unhook();
        unsafe { InvalidateRect(hwnd, std::ptr::null(), 1) };
    }
}

fn unhook() {
    let hook = REC.with_borrow_mut(|r| {
        r.held.clear();
        std::mem::replace(&mut r.hook, null_mut())
    });
    if !hook.is_null() {
        unsafe { UnhookWindowsHookEx(hook) };
    }
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let kb = &*(lparam as *const KBDLLHOOKSTRUCT);
        let up = kb.flags & LLKHF_UP != 0;
        let vk = keys::normalize(kb.vkCode);
        let extended = kb.flags & LLKHF_EXTENDED != 0;
        if REC.with_borrow_mut(|r| handle_key(r, vk, kb.scanCode, extended, up)) {
            return 1; // swallow
        }
    }
    CallNextHookEx(null_mut(), code, wparam, lparam)
}

/// Returns true if the key event must be swallowed.
fn handle_key(r: &mut Recorder, vk: u32, scan: u32, extended: bool, up: bool) -> bool {
    if let Some(pos) = r.pre_held.iter().position(|&k| k == vk) {
        if up {
            r.pre_held.remove(pos);
        }
        return false;
    }

    match r.phase {
        Phase::Idle => false,
        Phase::Recording if !up => {
            if !r.held.contains(&vk) {
                r.held.push(vk);
            }
            if !r.keys.iter().any(|(k, _)| *k == vk) {
                r.keys.push((vk, keys::name(vk, scan, extended)));
                unsafe { InvalidateRect(r.field, std::ptr::null(), 1) };
            }
            true
        }
        Phase::Recording | Phase::Draining => {
            let Some(pos) = r.held.iter().position(|&k| k == vk) else {
                return false;
            };
            if up {
                r.held.remove(pos);
                if r.phase == Phase::Recording {
                    r.phase = Phase::Draining;
                    unsafe { PostMessageW(r.field, WM_REC_COMMIT, 0, 0) };
                }
                if r.held.is_empty() {
                    r.phase = Phase::Idle;
                    // A hook must not unhook itself from inside its callback.
                    unsafe { PostMessageW(r.field, WM_REC_UNHOOK, 0, 0) };
                }
            }
            true
        }
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_LBUTTONDOWN => {
            SetFocus(hwnd);
            start_recording(hwnd);
            0
        }
        WM_KILLFOCUS => {
            cancel_recording(hwnd);
            0
        }
        WM_REC_COMMIT => {
            let text = REC.with_borrow_mut(|r| {
                let text = keys::format(&r.keys);
                r.keys.clear();
                text
            });
            if !text.is_empty() {
                ui::set_window_text(hwnd, &text);
                let id = GetDlgCtrlID(hwnd) as usize;
                SendMessageW(
                    GetParent(hwnd),
                    WM_COMMAND,
                    (HFN_CHANGED as usize) << 16 | id,
                    hwnd as LPARAM,
                );
            }
            InvalidateRect(hwnd, std::ptr::null(), 1);
            0
        }
        WM_REC_UNHOOK => {
            if REC.with_borrow(|r| r.phase == Phase::Idle) {
                unhook();
            }
            0
        }
        WM_SETTEXT => {
            let res = DefWindowProcW(hwnd, msg, wparam, lparam);
            InvalidateRect(hwnd, std::ptr::null(), 1);
            res
        }
        WM_SETFONT => {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, wparam as isize);
            0
        }
        WM_GETFONT => GetWindowLongPtrW(hwnd, GWLP_USERDATA) as LRESULT,
        WM_PAINT => {
            paint(hwnd);
            0
        }
        WM_DESTROY => {
            cancel_recording(hwnd);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn paint(hwnd: HWND) {
    let mut ps: PAINTSTRUCT = std::mem::zeroed();
    let hdc = BeginPaint(hwnd, &mut ps);
    let mut rc: RECT = std::mem::zeroed();
    GetClientRect(hwnd, &mut rc);

    let recording = is_recording(hwnd);
    let (text, placeholder) = if recording {
        let text = REC.with_borrow(|r| keys::format(&r.keys));
        if text.is_empty() {
            ("Press a key combination\u{2026}".to_string(), true)
        } else {
            (text, false)
        }
    } else {
        let text = ui::window_text(hwnd);
        if text.is_empty() {
            ("Not set".to_string(), true)
        } else {
            (text, false)
        }
    };

    FillRect(hdc, &rc, GetSysColorBrush(COLOR_WINDOW));
    let border_color = if recording { COLOR_HIGHLIGHT } else { COLOR_BTNSHADOW };
    let border = GetSysColorBrush(border_color);
    FrameRect(hdc, &rc, border);
    if recording {
        let mut inner = rc;
        InflateRect(&mut inner, -1, -1);
        FrameRect(hdc, &inner, border);
    }

    let font = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as HFONT;
    let old_font = SelectObject(hdc, font);
    SetBkMode(hdc, TRANSPARENT as i32);
    SetTextColor(
        hdc,
        GetSysColor(if placeholder { COLOR_GRAYTEXT } else { COLOR_WINDOWTEXT }),
    );
    let mut text_rc = rc;
    text_rc.left += ui::scale(8);
    text_rc.right -= ui::scale(8);
    let text = ui::wide(&text);
    DrawTextW(
        hdc,
        text.as_ptr(),
        -1,
        &mut text_rc,
        DT_SINGLELINE | DT_VCENTER | DT_LEFT | DT_END_ELLIPSIS | DT_NOPREFIX,
    );
    SelectObject(hdc, old_font);
    EndPaint(hwnd, &ps);
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_CONTROL, VK_MENU, VK_SHIFT};

    fn recorder(pre_held: Vec<u32>) -> Recorder {
        Recorder {
            field: null_mut(),
            hook: null_mut(),
            phase: Phase::Recording,
            keys: Vec::new(),
            held: Vec::new(),
            pre_held,
        }
    }

    const CTRL: u32 = VK_CONTROL as u32;
    const ALT: u32 = VK_MENU as u32;
    const SHIFT: u32 = VK_SHIFT as u32;
    const K: u32 = b'K' as u32;

    #[test]
    fn first_release_commits_and_remaining_keys_are_drained() {
        let mut r = recorder(vec![]);
        assert!(handle_key(&mut r, CTRL, 0, false, false));
        assert!(handle_key(&mut r, CTRL, 0, false, false)); // auto-repeat
        assert!(handle_key(&mut r, ALT, 0, false, false));
        assert!(handle_key(&mut r, K, 0, false, false));
        assert_eq!(keys::format(&r.keys), "Ctrl + Alt + K");

        assert!(handle_key(&mut r, ALT, 0, false, true));
        assert!(r.phase == Phase::Draining);
        assert_eq!(keys::format(&r.keys), "Ctrl + Alt + K");

        // Still-held keys stay swallowed, including their auto-repeat.
        assert!(handle_key(&mut r, CTRL, 0, false, false));
        assert!(handle_key(&mut r, CTRL, 0, false, true));
        assert!(handle_key(&mut r, K, 0, false, true));
        assert!(r.phase == Phase::Idle);

        // Nothing is swallowed afterwards.
        assert!(!handle_key(&mut r, CTRL, 0, false, false));
    }

    #[test]
    fn keys_held_before_recording_pass_through() {
        let mut r = recorder(vec![SHIFT]);
        assert!(!handle_key(&mut r, SHIFT, 0, false, false));
        assert!(!handle_key(&mut r, SHIFT, 0, false, true));
        assert!(r.keys.is_empty());
        assert!(r.phase == Phase::Recording);

        // Once released it is an ordinary key again.
        assert!(handle_key(&mut r, SHIFT, 0, false, false));
        assert_eq!(keys::format(&r.keys), "Shift");
    }
}
