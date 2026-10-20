//! A list whose items can be reordered by dragging them with the mouse.
//!
//! The dragged item moves live as the cursor crosses other rows. When the
//! drag ends with a changed order, the parent gets `WM_COMMAND(id, RLN_CHANGED)`.

use crate::ui;
use std::ptr::null_mut;
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::Controls::WM_MOUSELEAVE,
    UI::Input::KeyboardAndMouse::{
        ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
    },
    UI::WindowsAndMessaging::*,
};

pub const CLASS_NAME: &str = "JlsReorderList";
/// Notification code sent to the parent after the order has changed.
pub const RLN_CHANGED: u32 = 1;

const ITEM_H: i32 = 36;

pub struct Item {
    pub id: String,
    pub text: String,
}

struct Drag {
    /// Current index of the dragged item.
    index: usize,
    original: Vec<String>,
}

#[derive(Default)]
struct State {
    items: Vec<Item>,
    font: HFONT,
    hover: Option<usize>,
    drag: Option<Drag>,
}

pub fn register() {
    let class = ui::wide(CLASS_NAME);
    unsafe {
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            hInstance: GetModuleHandleW(std::ptr::null()),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassExW(&wc);
    }
}

pub fn set_items(hwnd: HWND, items: Vec<Item>) {
    if let Some(state) = unsafe { state(hwnd) } {
        state.items = items;
        state.hover = None;
        state.drag = None;
        unsafe { InvalidateRect(hwnd, std::ptr::null(), 0) };
    }
}

pub fn ids(hwnd: HWND) -> Vec<String> {
    unsafe { state(hwnd) }
        .map(|s| s.items.iter().map(|i| i.id.clone()).collect())
        .unwrap_or_default()
}

/// The control's state. The window procedure runs on a single thread and
/// never holds this reference across calls that could re-enter it.
unsafe fn state<'a>(hwnd: HWND) -> Option<&'a mut State> {
    (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State).as_mut()
}

/// Index of the item at client y, clamped to the existing items.
fn item_at(state: &State, y: i32) -> Option<usize> {
    if state.items.is_empty() || y < 0 {
        return None;
    }
    Some(((y / ui::scale(ITEM_H)) as usize).min(state.items.len() - 1))
}

fn notify_parent(hwnd: HWND) {
    unsafe {
        let id = GetDlgCtrlID(hwnd) as usize;
        SendMessageW(GetParent(hwnd), WM_COMMAND, (RLN_CHANGED as usize) << 16 | id, hwnd as LPARAM);
    }
}

fn finish_drag(hwnd: HWND) {
    let changed = {
        let Some(state) = (unsafe { state(hwnd) }) else { return };
        let Some(drag) = state.drag.take() else { return };
        state.items.iter().map(|i| &i.id).ne(drag.original.iter())
    };
    unsafe { InvalidateRect(hwnd, std::ptr::null(), 0) };
    if changed {
        notify_parent(hwnd);
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let y = ((lparam >> 16) & 0xFFFF) as u16 as i16 as i32;
    match msg {
        WM_NCCREATE => {
            let state = Box::into_raw(Box::<State>::default());
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_NCDESTROY => {
            let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            if !state.is_null() {
                drop(Box::from_raw(state));
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_SETFONT => {
            if let Some(state) = state(hwnd) {
                state.font = wparam as HFONT;
            }
            0
        }
        WM_GETFONT => state(hwnd).map_or(0, |s| s.font as LRESULT),
        WM_LBUTTONDOWN => {
            let Some(state) = state(hwnd) else { return 0 };
            if y >= (state.items.len() as i32) * ui::scale(ITEM_H) {
                return 0;
            }
            if let Some(index) = item_at(state, y) {
                state.drag = Some(Drag {
                    index,
                    original: state.items.iter().map(|i| i.id.clone()).collect(),
                });
                SetCapture(hwnd);
                SetCursor(LoadCursorW(null_mut(), IDC_SIZENS));
                InvalidateRect(hwnd, std::ptr::null(), 0);
            }
            0
        }
        WM_MOUSEMOVE => {
            let Some(state) = state(hwnd) else { return 0 };
            if let Some(drag) = &mut state.drag {
                SetCursor(LoadCursorW(null_mut(), IDC_SIZENS));
                let target = ((y.max(0) / ui::scale(ITEM_H)) as usize).min(state.items.len() - 1);
                if target != drag.index {
                    let item = state.items.remove(drag.index);
                    state.items.insert(target, item);
                    drag.index = target;
                    InvalidateRect(hwnd, std::ptr::null(), 0);
                }
            } else {
                let hover = (y < state.items.len() as i32 * ui::scale(ITEM_H))
                    .then(|| item_at(state, y))
                    .flatten();
                if hover != state.hover {
                    state.hover = hover;
                    InvalidateRect(hwnd, std::ptr::null(), 0);
                }
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                TrackMouseEvent(&mut tme);
            }
            0
        }
        WM_MOUSELEAVE => {
            if let Some(state) = state(hwnd) {
                if state.hover.take().is_some() {
                    InvalidateRect(hwnd, std::ptr::null(), 0);
                }
            }
            0
        }
        WM_SETCURSOR => {
            let over_item = state(hwnd).is_some_and(|s| s.hover.is_some());
            if over_item && (lparam & 0xFFFF) as u32 == HTCLIENT {
                SetCursor(LoadCursorW(null_mut(), IDC_SIZENS));
                1
            } else {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
        }
        WM_LBUTTONUP => {
            // Releasing capture sends WM_CAPTURECHANGED, which finishes the drag.
            ReleaseCapture();
            0
        }
        WM_CAPTURECHANGED => {
            finish_drag(hwnd);
            0
        }
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            paint(hwnd);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn paint(hwnd: HWND) {
    let mut ps: PAINTSTRUCT = std::mem::zeroed();
    let window_dc = BeginPaint(hwnd, &mut ps);
    let mut rc: RECT = std::mem::zeroed();
    GetClientRect(hwnd, &mut rc);

    // Draw off-screen to avoid flicker while dragging.
    let hdc = CreateCompatibleDC(window_dc);
    let bitmap = CreateCompatibleBitmap(window_dc, rc.right, rc.bottom);
    let old_bitmap = SelectObject(hdc, bitmap);

    FillRect(hdc, &rc, GetSysColorBrush(COLOR_WINDOW));
    if let Some(state) = state(hwnd) {
        let old_font = SelectObject(hdc, state.font);
        SetBkMode(hdc, TRANSPARENT as i32);
        let item_h = ui::scale(ITEM_H);
        let dragged = state.drag.as_ref().map(|d| d.index);

        for (i, item) in state.items.iter().enumerate() {
            let top = i as i32 * item_h;
            let row = RECT { left: 0, top, right: rc.right, bottom: top + item_h };
            let selected = dragged == Some(i);
            if selected {
                FillRect(hdc, &row, GetSysColorBrush(COLOR_HIGHLIGHT));
            } else if state.hover == Some(i) && dragged.is_none() {
                FillRect(hdc, &row, GetSysColorBrush(COLOR_BTNFACE));
            }
            if i > 0 && !selected && dragged != Some(i - 1) {
                let line = RECT { left: row.left, top, right: row.right, bottom: top + 1 };
                FillRect(hdc, &line, GetSysColorBrush(COLOR_BTNFACE));
            }

            let (text_color, dim_color) = if selected {
                (GetSysColor(COLOR_HIGHLIGHTTEXT), GetSysColor(COLOR_HIGHLIGHTTEXT))
            } else {
                (GetSysColor(COLOR_WINDOWTEXT), GetSysColor(COLOR_GRAYTEXT))
            };

            // Position number.
            let mut num_rc = RECT { left: row.left + ui::scale(8), right: row.left + ui::scale(32), ..row };
            SetTextColor(hdc, dim_color);
            let num = ui::wide(&format!("{}.", i + 1));
            DrawTextW(hdc, num.as_ptr(), -1, &mut num_rc, DT_SINGLELINE | DT_VCENTER | DT_RIGHT);

            // Name.
            let mut text_rc = RECT { left: row.left + ui::scale(42), right: row.right - ui::scale(36), ..row };
            SetTextColor(hdc, text_color);
            let text = ui::wide(&item.text);
            DrawTextW(
                hdc, text.as_ptr(), -1, &mut text_rc,
                DT_SINGLELINE | DT_VCENTER | DT_LEFT | DT_END_ELLIPSIS | DT_NOPREFIX,
            );

            // Grip: three short horizontal bars on the right.
            let grip = CreateSolidBrush(dim_color);
            let (gw, gh, gap) = (ui::scale(12), ui::scale(2).max(1), ui::scale(4));
            let gx = row.right - ui::scale(14) - gw;
            let gy = top + item_h / 2 - gh / 2 - gap;
            for k in 0..3 {
                let bar = RECT { left: gx, top: gy + k * gap, right: gx + gw, bottom: gy + k * gap + gh };
                FillRect(hdc, &bar, grip);
            }
            DeleteObject(grip);
        }
        SelectObject(hdc, old_font);
    }
    FrameRect(hdc, &rc, GetSysColorBrush(COLOR_BTNSHADOW));

    BitBlt(window_dc, 0, 0, rc.right, rc.bottom, hdc, 0, 0, SRCCOPY);
    SelectObject(hdc, old_bitmap);
    DeleteObject(bitmap);
    DeleteDC(hdc);
    EndPaint(hwnd, &ps);
}
