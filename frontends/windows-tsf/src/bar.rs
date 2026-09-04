//! The floating status bar.
//!
//! A small always-on-top pill showing whether keystrokes are being
//! transliterated (**ने**) or passed through (**EN**), with a `+` that opens
//! the word editor. Click the left half to toggle; drag it anywhere and the
//! position is remembered.
//!
//! **One bar, many processes.** A text service is loaded into every process
//! that takes text input, so each one has its own `Session` and would put up
//! its own bar. Visibility is therefore tied to focus — [`StatusBar::show`] is
//! called from `OnSetFocus(true)` and [`StatusBar::hide`] from
//! `OnSetFocus(false)` — so only the focused application's bar is ever on
//! screen, and it looks like the single floating widget the user expects.
//!
//! The window owns a weak reference to its session so a click can flip the
//! mode. Weak, because the window is destroyed by the session's `Drop`: an
//! owning reference would be a cycle that never runs.

use std::cell::RefCell;
use std::rc::Weak;
use std::sync::OnceLock;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, EndPaint, FillRect, InvalidateRect,
    MonitorFromPoint, RoundRect, SelectObject, SetBkMode, SetTextColor, TextOutW, CLEARTYPE_QUALITY,
    CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, FF_DONTCARE, FW_SEMIBOLD, HGDIOBJ,
    MONITOR_DEFAULTTONULL, MONITOR_DEFAULTTOPRIMARY, MONITOR_FROM_FLAGS, OUT_DEFAULT_PRECIS,
    PAINTSTRUCT, TRANSPARENT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, GetWindowRect, LoadCursorW,
    RegisterClassW, SetWindowPos, ShowWindow, CS_HREDRAW, CS_VREDRAW,
    IDC_HAND, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE,
    SW_SHOWNOACTIVATE, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCDESTROY, WM_PAINT,
    WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};

use crate::session::Session;

const CLASS_NAME: PCWSTR = w!("xlit_tsf_statusbar");
const BAR_W: i32 = 74;
const BAR_H: i32 = 30;
/// Where the mode half ends and the `+` half begins.
const SPLIT_X: i32 = 46;
/// Pointer travel, in pixels, above which a press counts as a drag not a click.
const DRAG_SLOP: i32 = 4;

struct Bar {
    sess: Weak<RefCell<Session>>,
    font: HGDIOBJ,
    /// Cursor and window origin at the moment the button went down.
    drag: Option<(POINT, POINT)>,
    moved: bool,
}

impl Drop for Bar {
    fn drop(&mut self) {
        unsafe { _ = DeleteObject(self.font) };
    }
}

pub struct StatusBar {
    hwnd: HWND,
}

impl StatusBar {
    pub const fn new() -> Self {
        StatusBar { hwnd: HWND(std::ptr::null_mut()) }
    }

    /// Put the bar on screen, creating it the first time. `sess` is the session
    /// whose mode it shows and toggles.
    pub fn show(&mut self, sess: &Weak<RefCell<Session>>) {
        if self.hwnd.is_invalid() && !self.create(sess.clone()) {
            return;
        }
        unsafe {
            _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            _ = InvalidateRect(Some(self.hwnd), None, true);
        }
    }

    pub fn hide(&mut self) {
        if !self.hwnd.is_invalid() {
            unsafe { _ = ShowWindow(self.hwnd, SW_HIDE) };
        }
    }

    /// Repaint after the mode changed by some other route (the toggle key).
    pub fn refresh(&self) {
        if !self.hwnd.is_invalid() {
            unsafe { _ = InvalidateRect(Some(self.hwnd), None, true) };
        }
    }

    fn create(&mut self, sess: Weak<RefCell<Session>>) -> bool {
        register_class();
        let font = unsafe {
            HGDIOBJ(
                CreateFontW(
                    -16,
                    0,
                    0,
                    0,
                    FW_SEMIBOLD.0 as i32,
                    0,
                    0,
                    0,
                    DEFAULT_CHARSET,
                    OUT_DEFAULT_PRECIS,
                    CLIP_DEFAULT_PRECIS,
                    CLEARTYPE_QUALITY,
                    DEFAULT_PITCH.0 as u32 | FF_DONTCARE.0 as u32,
                    w!("Nirmala UI"),
                )
                .0,
            )
        };
        let state: *mut RefCell<Bar> = Box::into_raw(Box::new(RefCell::new(Bar {
            sess,
            font,
            drag: None,
            moved: false,
        })));

        let (x, y) = saved_position();
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                CLASS_NAME,
                PCWSTR::null(),
                WS_POPUP,
                x,
                y,
                BAR_W,
                BAR_H,
                None,
                None,
                Some(crate::dll_hmodule().into()),
                Some(state as *const _),
            )
        };
        match hwnd {
            Ok(h) if !h.is_invalid() => {
                self.hwnd = h;
                true
            }
            _ => {
                drop(unsafe { Box::from_raw(state) });
                crate::debug("status bar could not be created");
                false
            }
        }
    }
}

impl Drop for StatusBar {
    fn drop(&mut self) {
        if !self.hwnd.is_invalid() {
            unsafe { _ = DestroyWindow(self.hwnd) };
        }
    }
}

// ---------------------------------------------------------------------------
// Position, remembered across sessions
// ---------------------------------------------------------------------------

const POS_KEY: &str = r"Software\xlit";

/// Bottom-right of the work area, clear of the taskbar.
fn default_position() -> (i32, i32) {
    match work_area() {
        Some(w) => ((w.right - BAR_W - 24).max(w.left), (w.bottom - BAR_H - 24).max(w.top)),
        None => (600, 600),
    }
}

/// Where to put the bar, honouring a remembered position only if it would
/// actually land on screen.
///
/// The check is not paranoia. The position is saved by whichever application
/// the user dragged the bar in, and read back by every other one — and those
/// processes do not agree about coordinates, because a DPI-aware host sees
/// physical pixels while a DPI-unaware one sees scaled ones. A position saved
/// at the bottom-right of a 1920x1080 desktop reads as 300px off the edge of
/// the same desktop seen as 1536x912, and the bar vanishes with no way to drag
/// it back. Falling back to the default keeps it recoverable.
fn saved_position() -> (i32, i32) {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(k) = hkcu.open_subkey(POS_KEY) {
        if let (Ok(x), Ok(y)) = (k.get_value::<u32, _>("BarX"), k.get_value::<u32, _>("BarY")) {
            let (x, y) = (x as i32, y as i32);
            if on_screen(x, y) {
                return (x, y);
            }
            crate::debug("saved bar position is off screen; using the default");
        }
    }
    default_position()
}

/// Is a bar at this origin wholly within some monitor's work area?
fn on_screen(x: i32, y: i32) -> bool {
    match work_area_at(x, y) {
        Some(w) => x >= w.left && y >= w.top && x + BAR_W <= w.right && y + BAR_H <= w.bottom,
        None => false,
    }
}

fn save_position(x: i32, y: i32) {
    if !on_screen(x, y) {
        // Refuse to remember somewhere it cannot be reached from.
        return;
    }
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok((k, _)) = hkcu.create_subkey(POS_KEY) {
        let _ = k.set_value("BarX", &(x.max(0) as u32));
        let _ = k.set_value("BarY", &(y.max(0) as u32));
    }
}

fn work_area() -> Option<RECT> {
    work_area_of(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY)
}

/// Work area of the monitor containing a point, or `None` if it is on none of
/// them — which is the case we care about when validating a saved position.
fn work_area_at(x: i32, y: i32) -> Option<RECT> {
    work_area_of(POINT { x, y }, MONITOR_DEFAULTTONULL)
}

fn work_area_of(pt: POINT, flags: MONITOR_FROM_FLAGS) -> Option<RECT> {
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITORINFO};
    unsafe {
        let mon = MonitorFromPoint(pt, flags);
        if mon.is_invalid() {
            return None;
        }
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        GetMonitorInfoW(mon, &mut info).as_bool().then_some(info.rcWork)
    }
}

// ---------------------------------------------------------------------------
// Window class
// ---------------------------------------------------------------------------

fn register_class() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| unsafe {
        let class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            hInstance: crate::dll_hmodule().into(),
            hCursor: LoadCursorW(None, IDC_HAND).unwrap_or_default(),
            lpszClassName: CLASS_NAME,
            ..Default::default()
        };
        RegisterClassW(&class);
    });
}

fn state<'a>(hwnd: HWND) -> Option<&'a RefCell<Bar>> {
    if hwnd.is_invalid() {
        return None;
    }
    let ptr = unsafe { crate::candwin::get_userdata(hwnd) } as *const RefCell<Bar>;
    unsafe { ptr.as_ref() }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            windows::Win32::UI::WindowsAndMessaging::WM_NCCREATE => {
                let cs = lparam.0 as *const windows::Win32::UI::WindowsAndMessaging::CREATESTRUCTW;
                if let Some(cs) = cs.as_ref() {
                    crate::candwin::set_userdata(hwnd, cs.lpCreateParams as isize);
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_PAINT => {
                paint(hwnd);
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                if let Some(st) = state(hwnd) {
                    if let Ok(mut b) = st.try_borrow_mut() {
                        let mut cur = POINT::default();
                        let mut rc = RECT::default();
                        if GetCursorPos(&mut cur).is_ok() && GetWindowRect(hwnd, &mut rc).is_ok() {
                            b.drag = Some((cur, POINT { x: rc.left, y: rc.top }));
                            b.moved = false;
                            SetCapture(hwnd);
                        }
                    }
                }
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                drag_to(hwnd);
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                finish_click(hwnd, lparam);
                LRESULT(0)
            }
            WM_NCDESTROY => {
                let ptr = crate::candwin::get_userdata(hwnd) as *mut RefCell<Bar>;
                if !ptr.is_null() {
                    crate::candwin::set_userdata(hwnd, 0);
                    drop(Box::from_raw(ptr));
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// Move the window with the pointer while the button is held.
fn drag_to(hwnd: HWND) {
    let Some(st) = state(hwnd) else { return };
    let Ok(mut b) = st.try_borrow_mut() else { return };
    let Some((start_cur, start_win)) = b.drag else { return };
    unsafe {
        let mut cur = POINT::default();
        if GetCursorPos(&mut cur).is_err() {
            return;
        }
        let (dx, dy) = (cur.x - start_cur.x, cur.y - start_cur.y);
        if dx.abs() > DRAG_SLOP || dy.abs() > DRAG_SLOP {
            b.moved = true;
        }
        if b.moved {
            _ = SetWindowPos(
                hwnd,
                None,
                start_win.x + dx,
                start_win.y + dy,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
}

/// Button released: either finish a drag (remember where it landed) or treat it
/// as a click on whichever half was pressed.
fn finish_click(hwnd: HWND, lparam: LPARAM) {
    let Some(st) = state(hwnd) else { return };
    unsafe { _ = ReleaseCapture() };

    let (was_drag, sess) = {
        let Ok(mut b) = st.try_borrow_mut() else { return };
        b.drag = None;
        (b.moved, b.sess.clone())
    };

    if was_drag {
        let mut rc = RECT::default();
        if unsafe { GetWindowRect(hwnd, &mut rc) }.is_ok() {
            save_position(rc.left, rc.top);
        }
        return;
    }

    let x = (lparam.0 & 0xFFFF) as i16 as i32;
    if x < SPLIT_X {
        toggle_mode(&sess);
        unsafe { _ = InvalidateRect(Some(hwnd), None, true) };
    } else {
        crate::wordeditor::open(&sess);
    }
}

/// Flip between transliterating and passing through, committing any word in
/// progress first — using the context stashed by the last key event, since a
/// mouse click carries none of its own.
fn toggle_mode(sess: &Weak<RefCell<Session>>) {
    let Some(sess) = sess.upgrade() else { return };
    let ctx = {
        let Ok(s) = sess.try_borrow() else { return };
        s.composing().then(|| s.last_ctx.clone()).flatten()
    };
    if let Some(ctx) = ctx {
        crate::session::commit(&sess, &ctx, "");
    }
    if let Ok(mut s) = sess.try_borrow_mut() {
        s.enabled = !s.enabled;
        crate::debug(if s.enabled { "enabled (bar)" } else { "passthrough (bar)" });
    };
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

fn paint(hwnd: HWND) {
    let enabled = state(hwnd)
        .and_then(|st| st.try_borrow().ok())
        .and_then(|b| b.sess.upgrade())
        .and_then(|s| s.try_borrow().ok().map(|s| s.enabled))
        .unwrap_or(true);

    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut ps);
        let mut rc = RECT::default();
        _ = GetWindowRect(hwnd, &mut rc);
        let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
        let full = RECT { left: 0, top: 0, right: w, bottom: h };

        // Green when transliterating, grey when passing through - readable at a
        // glance without reading the label.
        let accent = if enabled { COLORREF(0x00_5B_9E_2D) } else { COLORREF(0x00_60_60_60) };
        let bg = CreateSolidBrush(accent);
        let pen_hold = SelectObject(hdc, HGDIOBJ(bg.0));
        FillRect(hdc, &full, bg);
        _ = RoundRect(hdc, 0, 0, w, h, 10, 10);
        SelectObject(hdc, pen_hold);
        _ = DeleteObject(HGDIOBJ(bg.0));

        if let Some(st) = state(hwnd) {
            if let Ok(b) = st.try_borrow() {
                let old = SelectObject(hdc, b.font);
                SetBkMode(hdc, TRANSPARENT);
                SetTextColor(hdc, COLORREF(0x00_FF_FF_FF));

                let label: Vec<u16> =
                    if enabled { "ने" } else { "EN" }.encode_utf16().collect();
                _ = TextOutW(hdc, 12, 4, &label);

                // Divider and the add-word affordance.
                let line = RECT { left: SPLIT_X - 1, top: 6, right: SPLIT_X, bottom: h - 6 };
                let div = CreateSolidBrush(COLORREF(0x00_FF_FF_FF));
                FillRect(hdc, &line, div);
                _ = DeleteObject(HGDIOBJ(div.0));

                let plus: Vec<u16> = "+".encode_utf16().collect();
                _ = TextOutW(hdc, SPLIT_X + 10, 4, &plus);
                SelectObject(hdc, old);
            }
        }
        _ = EndPaint(hwnd, &ps);
    }
}
