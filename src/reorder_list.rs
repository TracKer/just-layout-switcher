//! A list whose items can be reordered by dragging them with the mouse and
//! switched on and off with a checkbox.
//!
//! The dragged item moves live as the cursor crosses other rows. Disabled items
//! are grayed out but keep their number; the last enabled item can't be disabled.
//! When the order or an item's state changes, the parent gets
//! `WM_COMMAND(id, RLN_CHANGED)`; a click on an item's "more" button sends
//! `WM_COMMAND(id, RLN_MORE)` (see [`active_item`]).

use crate::ui;
use std::ptr::null_mut;
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::Controls::{
        CloseThemeData, DrawThemeBackground, GetThemePartSize, OpenThemeData, BP_CHECKBOX,
        CBS_CHECKEDDISABLED, CBS_CHECKEDHOT, CBS_CHECKEDNORMAL, CBS_UNCHECKEDHOT,
        CBS_UNCHECKEDNORMAL, HTHEME, TS_DRAW, WM_MOUSELEAVE,
    },
    UI::Input::KeyboardAndMouse::{
        ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
    },
    UI::WindowsAndMessaging::*,
};

pub const CLASS_NAME: &str = "JlsReorderList";
/// Notification code sent to the parent after the order or an item's state has changed.
pub const RLN_CHANGED: u32 = 1;
/// Notification code sent to the parent when an item's "more" button is clicked.
pub const RLN_MORE: u32 = 2;

const ITEM_H: i32 = 36;
// Columns, left to right: grip, checkbox, number, name, "more" button.
const GRIP_W: i32 = 24;
const CHECK_W: i32 = 30;
const NUMBER_W: i32 = 22;
const MORE_W: i32 = 40;
const CHECK_SIZE: i32 = 16;

/// The part of a row under the cursor.
#[derive(Clone, Copy, PartialEq, Default)]
enum Zone {
    /// Anywhere that starts a drag, including the grip.
    #[default]
    Row,
    Check,
    More,
}

fn zone_at(hwnd: HWND, x: i32) -> Zone {
    let mut rc: RECT = unsafe { std::mem::zeroed() };
    unsafe { GetClientRect(hwnd, &mut rc) };
    if x >= rc.right - ui::scale(MORE_W) {
        Zone::More
    } else if (ui::scale(GRIP_W)..ui::scale(GRIP_W + CHECK_W)).contains(&x) {
        Zone::Check
    } else {
        Zone::Row
    }
}

pub struct Item {
    pub id: String,
    pub text: String,
    pub enabled: bool,
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
    /// Visual style for drawing checkboxes; 0 when themes are off.
    theme: HTHEME,
    hover: Option<usize>,
    /// Part of the hovered item under the cursor.
    hover_zone: Zone,
    drag: Option<Drag>,
    /// Item whose "more" button was clicked last.
    active: Option<usize>,
}

fn open_theme(hwnd: HWND) -> HTHEME {
    let class = ui::wide("Button");
    unsafe { OpenThemeData(hwnd, class.as_ptr()) }
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

/// Items as (id, enabled), top to bottom.
pub fn entries(hwnd: HWND) -> Vec<(String, bool)> {
    unsafe { state(hwnd) }
        .map(|s| s.items.iter().map(|i| (i.id.clone(), i.enabled)).collect())
        .unwrap_or_default()
}

/// The item (id, text) whose "more" button was clicked last.
pub fn active_item(hwnd: HWND) -> Option<(String, String)> {
    let state = unsafe { state(hwnd) }?;
    let item = state.items.get(state.active?)?;
    Some((item.id.clone(), item.text.clone()))
}

/// Toggles an item unless it is the last enabled one. Returns true if toggled.
fn toggle(items: &mut [Item], index: usize) -> bool {
    let enabled = items.iter().filter(|i| i.enabled).count();
    let item = &mut items[index];
    if item.enabled && enabled <= 1 {
        return false;
    }
    item.enabled = !item.enabled;
    true
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

fn notify_parent(hwnd: HWND, code: u32) {
    unsafe {
        let id = GetDlgCtrlID(hwnd) as usize;
        SendMessageW(GetParent(hwnd), WM_COMMAND, (code as usize) << 16 | id, hwnd as LPARAM);
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
        notify_parent(hwnd, RLN_CHANGED);
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let x = (lparam & 0xFFFF) as u16 as i16 as i32;
    let y = ((lparam >> 16) & 0xFFFF) as u16 as i16 as i32;
    match msg {
        WM_NCCREATE => {
            let state = Box::into_raw(Box::<State>::default());
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_CREATE => {
            if let Some(state) = state(hwnd) {
                state.theme = open_theme(hwnd);
            }
            0
        }
        WM_THEMECHANGED => {
            if let Some(state) = state(hwnd) {
                if state.theme != 0 {
                    CloseThemeData(state.theme);
                }
                state.theme = open_theme(hwnd);
                InvalidateRect(hwnd, std::ptr::null(), 0);
            }
            0
        }
        WM_NCDESTROY => {
            let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            if !state.is_null() {
                let state = Box::from_raw(state);
                if state.theme != 0 {
                    CloseThemeData(state.theme);
                }
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
            let Some(index) = item_at(state, y) else { return 0 };
            match zone_at(hwnd, x) {
                Zone::Check => {
                    if toggle(&mut state.items, index) {
                        InvalidateRect(hwnd, std::ptr::null(), 0);
                        notify_parent(hwnd, RLN_CHANGED);
                    }
                    return 0;
                }
                Zone::More => {
                    state.active = Some(index);
                    notify_parent(hwnd, RLN_MORE);
                    return 0;
                }
                Zone::Row => {}
            }
            state.drag = Some(Drag {
                index,
                original: state.items.iter().map(|i| i.id.clone()).collect(),
            });
            SetCapture(hwnd);
            SetCursor(LoadCursorW(null_mut(), IDC_SIZENS));
            InvalidateRect(hwnd, std::ptr::null(), 0);
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
                let hover_zone = zone_at(hwnd, x);
                if hover != state.hover || hover_zone != state.hover_zone {
                    state.hover = hover;
                    state.hover_zone = hover_zone;
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
                state.hover_zone = Zone::Row;
                if state.hover.take().is_some() {
                    InvalidateRect(hwnd, std::ptr::null(), 0);
                }
            }
            0
        }
        WM_SETCURSOR => {
            let mut pt = POINT { x: 0, y: 0 };
            GetCursorPos(&mut pt);
            ScreenToClient(hwnd, &mut pt);
            let over_item = state(hwnd).is_some_and(|s| s.hover.is_some());
            if over_item && zone_at(hwnd, pt.x) == Zone::Row && (lparam & 0xFFFF) as u32 == HTCLIENT {
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

/// Draws a checkbox centered in `cell`: the system one if themes are on.
/// `locked` is a checked box that can't be unchecked.
unsafe fn draw_check(hdc: HDC, theme: HTHEME, cell: RECT, checked: bool, locked: bool, hot: bool, on_highlight: bool) {
    if theme != 0 {
        let state_id = match (checked, locked, hot) {
            (true, true, _) => CBS_CHECKEDDISABLED,
            (true, false, true) => CBS_CHECKEDHOT,
            (true, false, false) => CBS_CHECKEDNORMAL,
            (false, _, true) => CBS_UNCHECKEDHOT,
            (false, _, false) => CBS_UNCHECKEDNORMAL,
        };
        let mut size = SIZE { cx: 0, cy: 0 };
        if GetThemePartSize(theme, hdc, BP_CHECKBOX, state_id, std::ptr::null(), TS_DRAW, &mut size) >= 0 {
            let left = cell.left + (cell.right - cell.left - size.cx) / 2;
            let top = cell.top + (cell.bottom - cell.top - size.cy) / 2;
            let rc = RECT { left, top, right: left + size.cx, bottom: top + size.cy };
            if DrawThemeBackground(theme, hdc, BP_CHECKBOX, state_id, &rc, std::ptr::null()) >= 0 {
                return;
            }
        }
    }

    let size = ui::scale(CHECK_SIZE);
    let left = cell.left + (cell.right - cell.left - size) / 2;
    let top = cell.top + (cell.bottom - cell.top - size) / 2;
    draw_checkbox(
        hdc,
        RECT { left, top, right: left + size, bottom: top + size },
        checked,
        locked,
        on_highlight,
    );
}

/// A grid of square dots; (x, y) is the top-left corner of the first dot.
#[allow(clippy::too_many_arguments)]
unsafe fn draw_dots(hdc: HDC, x: i32, y: i32, cols: i32, rows: i32, step: i32, size: i32, color: COLORREF) {
    let brush = CreateSolidBrush(color);
    for row in 0..rows {
        for col in 0..cols {
            let (left, top) = (x + col * step, y + row * step);
            FillRect(hdc, &RECT { left, top, right: left + size, bottom: top + size }, brush);
        }
    }
    DeleteObject(brush);
}

/// Fallback checkbox for when visual styles are off.
unsafe fn draw_checkbox(hdc: HDC, rc: RECT, checked: bool, locked: bool, on_highlight: bool) {
    let background = GetSysColor(if on_highlight { COLOR_HIGHLIGHT } else { COLOR_WINDOW });
    let accent = GetSysColor(if on_highlight {
        COLOR_HIGHLIGHTTEXT
    } else if locked {
        COLOR_GRAYTEXT
    } else {
        COLOR_HIGHLIGHT
    });
    let border = if checked || on_highlight { accent } else { GetSysColor(COLOR_GRAYTEXT) };

    let pen = CreatePen(PS_SOLID, 1, border);
    let brush = CreateSolidBrush(if checked { accent } else { background });
    let old_pen = SelectObject(hdc, pen);
    let old_brush = SelectObject(hdc, brush);
    let radius = ui::scale(4);
    RoundRect(hdc, rc.left, rc.top, rc.right, rc.bottom, radius, radius);

    if checked {
        let mark = CreatePen(PS_SOLID, ui::scale(2).max(1), background);
        SelectObject(hdc, mark);
        let w = rc.right - rc.left;
        MoveToEx(hdc, rc.left + w * 4 / 16, rc.top + w * 8 / 16, null_mut());
        LineTo(hdc, rc.left + w * 7 / 16, rc.top + w * 11 / 16);
        LineTo(hdc, rc.left + w * 12 / 16, rc.top + w * 5 / 16);
        SelectObject(hdc, pen);
        DeleteObject(mark);
    }

    SelectObject(hdc, old_pen);
    SelectObject(hdc, old_brush);
    DeleteObject(pen);
    DeleteObject(brush);
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
        let enabled_count = state.items.iter().filter(|i| i.enabled).count();

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
            } else if item.enabled {
                (GetSysColor(COLOR_WINDOWTEXT), GetSysColor(COLOR_GRAYTEXT))
            } else {
                (GetSysColor(COLOR_GRAYTEXT), GetSysColor(COLOR_GRAYTEXT))
            };

            let hovered = state.hover == Some(i) && dragged.is_none();

            // Grip: two columns of three dots.
            let dot = ui::scale(3);
            let step = ui::scale(6);
            let grip_x = row.left + (ui::scale(GRIP_W) - (step + dot)) / 2 + ui::scale(2);
            let grip_y = top + (item_h - (2 * step + dot)) / 2;
            draw_dots(hdc, grip_x, grip_y, 2, 3, step, dot, dim_color);

            let check_cell = RECT {
                left: row.left + ui::scale(GRIP_W),
                right: row.left + ui::scale(GRIP_W + CHECK_W),
                ..row
            };
            draw_check(
                hdc,
                state.theme,
                check_cell,
                item.enabled,
                item.enabled && enabled_count <= 1,
                hovered && state.hover_zone == Zone::Check,
                selected,
            );

            // Position in the list; disabled items keep theirs too.
            let label = format!("{}.", i + 1);
            let number_left = row.left + ui::scale(GRIP_W + CHECK_W);
            let mut num_rc = RECT { left: number_left, right: number_left + ui::scale(NUMBER_W), ..row };
            SetTextColor(hdc, dim_color);
            let num = ui::wide(&label);
            DrawTextW(hdc, num.as_ptr(), -1, &mut num_rc, DT_SINGLELINE | DT_VCENTER | DT_RIGHT);

            // Name.
            let mut text_rc = RECT {
                left: number_left + ui::scale(NUMBER_W + 10),
                right: row.right - ui::scale(MORE_W),
                ..row
            };
            SetTextColor(hdc, text_color);
            let text = ui::wide(&item.text);
            DrawTextW(
                hdc, text.as_ptr(), -1, &mut text_rc,
                DT_SINGLELINE | DT_VCENTER | DT_LEFT | DT_END_ELLIPSIS | DT_NOPREFIX,
            );

            // "More" button: three dots, with a rounded background on hover.
            let more = RECT { left: row.right - ui::scale(MORE_W), ..row };
            if hovered && state.hover_zone == Zone::More {
                let pad = ui::scale(6);
                let brush = CreateSolidBrush(GetSysColor(COLOR_3DLIGHT));
                let old_brush = SelectObject(hdc, brush);
                let old_pen = SelectObject(hdc, GetStockObject(NULL_PEN));
                let radius = ui::scale(6);
                RoundRect(
                    hdc, more.left + pad, more.top + pad, more.right - pad + 1, more.bottom - pad + 1,
                    radius, radius,
                );
                SelectObject(hdc, old_pen);
                SelectObject(hdc, old_brush);
                DeleteObject(brush);
            }
            let more_color = GetSysColor(if selected { COLOR_HIGHLIGHTTEXT } else { COLOR_WINDOWTEXT });
            let more_x = more.left + (ui::scale(MORE_W) - (2 * step + dot)) / 2;
            draw_dots(hdc, more_x, top + (item_h - dot) / 2, 3, 1, step, dot, more_color);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn items(enabled: &[bool]) -> Vec<Item> {
        enabled
            .iter()
            .enumerate()
            .map(|(i, &enabled)| Item { id: i.to_string(), text: String::new(), enabled })
            .collect()
    }

    #[test]
    fn toggles_items_back_and_forth() {
        let mut list = items(&[true, true]);
        assert!(toggle(&mut list, 0));
        assert!(!list[0].enabled);
        assert!(toggle(&mut list, 0));
        assert!(list[0].enabled);
    }

    #[test]
    fn last_enabled_item_cannot_be_disabled() {
        let mut list = items(&[true, false]);
        assert!(!toggle(&mut list, 0));
        assert!(list[0].enabled);
        assert!(toggle(&mut list, 1));
    }
}
