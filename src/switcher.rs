//! Switches the keyboard layout by the global hotkey.
//!
//! A low-level keyboard hook feeds key events to the `Matcher`. The hook only
//! decides whether to pass an event on; the actual switching and the key
//! injection happen in a message-only window, so the hook returns quickly
//! (Windows silently drops hooks that are too slow).
//!
//! The layout is taken from our own list: the configured order without the
//! disabled layouts. Depending on the mode it is the next one (wrapping
//! around from the last to the first), the N-th one for N presses in a row,
//! or the one whose own hotkey was pressed. It is requested from the focused
//! window with `WM_INPUTLANGCHANGEREQUEST`, the same way the system hotkey
//! does it.
//!
//! The correction hotkey fixes a word typed in the wrong layout: the hook
//! remembers the keys of the last word, and on the hotkey they are erased
//! with Backspace and pressed again once the window has switched to the next
//! layout.

use crate::{config, hotkey_field, keys, layouts, matcher::Matcher, ui, word};
use std::cell::RefCell;
use std::ptr::null_mut;
use windows_sys::Win32::{
    Foundation::*,
    System::{LibraryLoader::GetModuleHandleW, SystemInformation::GetTickCount},
    UI::Input::KeyboardAndMouse::*,
    UI::WindowsAndMessaging::*,
};

const CLASS_NAME: &str = "JlsSwitcher";
/// wParam: 1 if the hotkey fired with its modifiers held since the last time;
/// lParam: index of the hotkey that fired.
const WM_SWITCH: u32 = WM_APP + 20;
/// wParam: vk of a key-up to replay after the mask (0 for none);
/// lParam: its scan code, bit 16 set for an extended key.
const WM_MASK: u32 = WM_APP + 21;

/// Marks the input we inject ourselves, so the hook lets it through.
const INJECTED_MARK: usize = 0x4A4C_5300;
/// An unassigned virtual key, used to break up modifier taps.
const VK_MASK: u16 = 0xE8;

const TIMER_CORRECTION: usize = 1;
/// How often to check whether the window has switched the layout before
/// the corrected word is typed.
const CORRECTION_POLL_MS: u32 = 10;
/// Type the word anyway if the window doesn't report the new layout by then
/// (consoles, for one, may never do).
const CORRECTION_WAIT_MS: u32 = 300;

/// How long a requested layout is assumed to be the current one while the
/// target window hasn't processed the request yet.
const PENDING_MS: u32 = 500;
/// In the press-count mode, a press within this time after the previous one
/// goes on to the next layout; after a longer pause counting starts over.
const SERIES_MS: u32 = 1000;

#[derive(Clone, Copy)]
enum Action {
    /// The next enabled layout.
    Cycle,
    /// The N-th enabled layout for the N-th press in a row.
    PressCount,
    /// This very layout (HKL).
    Layout(u32),
    /// Retype the last word in the next enabled layout.
    Correction,
}

/// A word waiting for its window to switch the layout.
struct Correction {
    /// GUI thread of the window.
    thread: u32,
    hkl: u32,
    keys: Vec<word::Key>,
    started: u32,
}

struct Switcher {
    window: HWND,
    keyboard_hook: HHOOK,
    mouse_hook: HHOOK,
    matcher: Matcher,
    /// What each of the matcher's hotkeys does.
    actions: Vec<Action>,
    config: config::Config,
    /// The last requested layout: (GUI thread, HKL, tick).
    pending: Option<(u32, u32, u32)>,
    /// Press-count mode: index of the last pressed layout and when.
    series: Option<(usize, u32)>,
    /// The keys of the word being typed.
    word: word::Word,
    correction: Option<Correction>,
}

thread_local! {
    static SWITCHER: RefCell<Switcher> = RefCell::new(Switcher {
        window: null_mut(),
        keyboard_hook: null_mut(),
        mouse_hook: null_mut(),
        matcher: Matcher::default(),
        actions: Vec::new(),
        config: config::Config::default(),
        pending: None,
        series: None,
        word: word::Word::default(),
        correction: None,
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
    let (hotkeys, actions) = hotkeys(config);
    let need_mouse = SWITCHER.with_borrow_mut(|s| {
        s.config = config.clone();
        s.actions = actions;
        s.series = None;
        s.matcher.set(hotkeys, config.block_hotkey);
        // Clicks also move the caret away from the remembered word.
        s.matcher.fires_on_release() || s.actions.iter().any(|a| matches!(a, Action::Correction))
    });
    set_mouse_hook(need_mouse);
}

/// The tracked hotkeys and what each one does: the common hotkey as the mode
/// says, plus the correction hotkey and the layouts' own hotkeys in every
/// mode.
fn hotkeys(config: &config::Config) -> (Vec<Vec<u32>>, Vec<Action>) {
    let main = match config.switch_mode {
        config::SwitchMode::Cycle => Some(Action::Cycle),
        config::SwitchMode::PressCount => Some(Action::PressCount),
        config::SwitchMode::PerLayout => None,
    }
    .and_then(|action| Some((keys::parse(&config.hotkey)?, action)));

    // A layout's own hotkey works even if the layout is left out of the
    // cycle; only turning the hotkey itself off stops it.
    let own = config.layouts.iter().filter(|l| l.hotkey_enabled).filter_map(|l| {
        let hkl = u32::from_str_radix(&l.id, 16).ok()?;
        Some((keys::parse(&l.hotkey)?, Action::Layout(hkl)))
    });
    let correction = keys::parse(&config.correction_hotkey).map(|keys| (keys, Action::Correction));
    main.into_iter().chain(correction).chain(own).unzip()
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
        if !up && outcome.fire.is_none() && !outcome.swallow {
            let held = |vk| s.matcher.held().iter().any(|&k| keys::normalize(k) == vk);
            let shortcut = held(VK_CONTROL as u32) || held(keys::VK_ALT) || held(keys::VK_WIN);
            let key = word::Key {
                vk: kb.vkCode,
                scan: kb.scanCode,
                extended: kb.flags & LLKHF_EXTENDED != 0,
                shift: held(VK_SHIFT as u32),
            };
            let window = unsafe { GetForegroundWindow() } as isize;
            s.word.key_down(key, window, shortcut);
        }
        Some((s.window, outcome))
    }) else {
        return false;
    };
    let Some((window, outcome)) = outcome else { return false };

    unsafe {
        if let Some(index) = outcome.fire {
            PostMessageW(window, WM_SWITCH, outcome.continues as WPARAM, index as LPARAM);
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
    let message = wparam as u32;
    if code == HC_ACTION as i32
        && matches!(
            message,
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN | WM_MOUSEWHEEL | WM_MOUSEHWHEEL
        )
    {
        let _ = SWITCHER.try_with(|s| {
            if let Ok(mut s) = s.try_borrow_mut() {
                s.matcher.mouse_down();
                // Scrolling leaves the caret where it was.
                if !matches!(message, WM_MOUSEWHEEL | WM_MOUSEHWHEEL) {
                    s.word.clear();
                }
            }
        });
    }
    CallNextHookEx(null_mut(), code, wparam, lparam)
}

fn key_input(vk: u16, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: INJECTED_MARK },
        },
    }
}

/// Sends the mask key down and up, then the key-up we swallowed, if any.
unsafe fn send_mask(replay_vk: u16, replay_scan: u32) {
    let mut inputs = vec![key_input(VK_MASK, 0, 0), key_input(VK_MASK, 0, KEYEVENTF_KEYUP)];
    if replay_vk != 0 {
        let extended = if replay_scan & (1 << 16) != 0 { KEYEVENTF_EXTENDEDKEY } else { 0 };
        inputs.push(key_input(replay_vk, replay_scan as u16, KEYEVENTF_KEYUP | extended));
    }
    SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32);
}

impl Switcher {
    /// The layout of the window's GUI thread. Quick repeated presses may come
    /// before the window has processed the previous request; go on from what
    /// was requested then.
    fn current_layout(&self, thread: u32, now: u32) -> u32 {
        let actual = unsafe { GetKeyboardLayout(thread) } as usize as u32;
        match self.pending {
            Some((t, hkl, tick)) if t == thread && hkl != actual && now.wrapping_sub(tick) < PENDING_MS => hkl,
            _ => actual,
        }
    }
}

/// Does what the hotkey with this index is for.
unsafe fn on_hotkey(index: usize, continues: bool) {
    // The config may have changed since the hotkey fired.
    match SWITCHER.with_borrow(|s| s.actions.get(index).copied()) {
        Some(Action::Correction) => start_correction(),
        Some(action) => switch_layout(action, continues),
        None => {}
    }
}

/// Switches the foreground application to the layout picked by `action`.
unsafe fn switch_layout(action: Action, continues: bool) {
    let foreground = GetForegroundWindow();
    if foreground.is_null() {
        return;
    }
    let thread = GetWindowThreadProcessId(foreground, null_mut());
    let now = GetTickCount();

    let next = SWITCHER.with_borrow_mut(|s| {
        let list = layouts::switch_order(&s.config.layouts);
        match action {
            Action::Layout(hkl) => list.iter().any(|&(l, _)| l == hkl).then_some(hkl),
            Action::PressCount => {
                let index = match s.series {
                    Some((last, tick)) if continues || now.wrapping_sub(tick) < SERIES_MS => last + 1,
                    _ => 0,
                };
                s.series = Some((index, now));
                layouts::nth_enabled(&list, index)
            }
            Action::Cycle => layouts::next_enabled(&list, s.current_layout(thread, now)),
            Action::Correction => None,
        }
    });
    if let Some(next) = next {
        request_layout(foreground, thread, next, now);
    }
}

/// Asks the window to switch to the layout `hkl`.
unsafe fn request_layout(foreground: HWND, thread: u32, hkl: u32, now: u32) {
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
    PostMessageW(target, WM_INPUTLANGCHANGEREQUEST, 0, hkl as i32 as LPARAM);
    SWITCHER.with_borrow_mut(|s| s.pending = Some((thread, hkl, now)));
}

/// Switches the foreground window to the next enabled layout and, once it
/// has switched, types the last word again.
unsafe fn start_correction() {
    let foreground = GetForegroundWindow();
    if foreground.is_null() {
        return;
    }
    let thread = GetWindowThreadProcessId(foreground, null_mut());
    let now = GetTickCount();
    let started = SWITCHER.with_borrow_mut(|s| {
        if s.correction.is_some() {
            return None;
        }
        let keys = s.word.keys(foreground as isize)?.to_vec();
        let list = layouts::switch_order(&s.config.layouts);
        let hkl = layouts::next_enabled(&list, s.current_layout(thread, now))?;
        s.correction = Some(Correction { thread, hkl, keys, started: now });
        Some((s.window, hkl))
    });
    let Some((window, hkl)) = started else { return };
    request_layout(foreground, thread, hkl, now);
    SetTimer(window, TIMER_CORRECTION, CORRECTION_POLL_MS, None);
}

/// Types the waiting word once its window has switched the layout or the
/// wait is over.
unsafe fn on_correction_timer(window: HWND) {
    let now = GetTickCount();
    let ready = SWITCHER.with_borrow_mut(|s| {
        let Some(c) = &s.correction else { return Some(None) };
        let switched = GetKeyboardLayout(c.thread) as usize as u32 == c.hkl;
        (switched || now.wrapping_sub(c.started) >= CORRECTION_WAIT_MS).then(|| s.correction.take())
    });
    // Still waiting.
    let Some(correction) = ready else { return };
    KillTimer(window, TIMER_CORRECTION);
    let Some(correction) = correction else { return };
    let held = SWITCHER.with_borrow(|s| s.matcher.held().to_vec());
    retype(&correction.keys, correction.hkl, &held);
}

/// Erases as many characters as there are keys and presses the keys again
/// in the layout `hkl`. Modifiers the user holds are released for that time,
/// or the keys would come as shortcuts (Ctrl+Backspace erases a word).
unsafe fn retype(keys: &[word::Key], hkl: u32, held: &[u32]) {
    let modifiers: Vec<u16> =
        held.iter().filter(|&&vk| keys::is_modifier(keys::normalize(vk))).map(|&vk| vk as u16).collect();
    let modifier_flags = |vk: u16| match vk {
        VK_RCONTROL | VK_RMENU | VK_LWIN | VK_RWIN => KEYEVENTF_EXTENDEDKEY,
        _ => 0,
    };
    // Releasing (and later pressing) Alt or Win alone opens a menu.
    let mask = modifiers.iter().any(|&vk| matches!(keys::normalize(vk as u32), keys::VK_ALT | keys::VK_WIN));

    let mut inputs = Vec::new();
    if mask {
        inputs.extend([key_input(VK_MASK, 0, 0), key_input(VK_MASK, 0, KEYEVENTF_KEYUP)]);
    }
    for &vk in &modifiers {
        inputs.push(key_input(vk, 0, KEYEVENTF_KEYUP | modifier_flags(vk)));
    }
    let back_scan = MapVirtualKeyW(VK_BACK as u32, MAPVK_VK_TO_VSC) as u16;
    for _ in keys {
        inputs.extend([key_input(VK_BACK, back_scan, 0), key_input(VK_BACK, back_scan, KEYEVENTF_KEYUP)]);
    }
    let shift_scan = MapVirtualKeyW(VK_LSHIFT as u32, MAPVK_VK_TO_VSC) as u16;
    for key in keys {
        // The same physical key; its virtual key depends on the layout,
        // except for the keypad and the extended keys.
        let vk = if key.extended || (0x60..=0x6F).contains(&key.vk) {
            key.vk
        } else {
            match MapVirtualKeyExW(key.scan, MAPVK_VSC_TO_VK_EX, hkl as i32 as isize as HKL) {
                0 => key.vk,
                vk => vk,
            }
        } as u16;
        let flags = if key.extended { KEYEVENTF_EXTENDEDKEY } else { 0 };
        if key.shift {
            inputs.push(key_input(VK_LSHIFT, shift_scan, 0));
        }
        let scan = key.scan as u16;
        inputs.extend([key_input(vk, scan, flags), key_input(vk, scan, flags | KEYEVENTF_KEYUP)]);
        if key.shift {
            inputs.push(key_input(VK_LSHIFT, shift_scan, KEYEVENTF_KEYUP));
        }
    }
    for &vk in &modifiers {
        inputs.push(key_input(vk, 0, modifier_flags(vk)));
    }
    if mask {
        inputs.extend([key_input(VK_MASK, 0, 0), key_input(VK_MASK, 0, KEYEVENTF_KEYUP)]);
    }
    SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32);
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_SWITCH => {
            on_hotkey(lparam as usize, wparam != 0);
            0
        }
        WM_TIMER if wparam == TIMER_CORRECTION => {
            on_correction_timer(hwnd);
            0
        }
        WM_MASK => {
            send_mask(wparam as u16, lparam as u32);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
