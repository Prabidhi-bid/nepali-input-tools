//! The floating status bar.
//!
//! A small always-on-top pill showing whether keystrokes are being
//! transliterated (**ने**) or passed through (**EN**), beside a `+` that opens
//! the word editor. Click the left half to toggle; drag it anywhere and the
//! position is remembered.
//!
//! **One bar on the desktop.** That is the whole difficulty here. A text
//! service is loaded into every process that takes text input, and every
//! activation of it makes a fresh [`Session`]. Three rules keep that from
//! becoming a column of floating pills:
//!
//! 1. **The window belongs to the thread, not to the session.** It lives in a
//!    thread-local slot, put up by [`show`] and taken down by [`destroy`] on
//!    `Deactivate`. Windows re-activates threads it never deactivated, and
//!    while the window hung off the session, each of those activations built
//!    another one and left the previous one on screen.
//! 2. **Only the focused thread puts one up.** Switching input method
//!    activates the text service in every process that holds an input context,
//!    not just the application in front, so `Activate` asks
//!    `ITfThreadMgr::IsThreadFocus` first and the other threads wait for
//!    `OnSetFocus(true)`. Those background bars did not even stack neatly: a
//!    DPI-unaware process reads the same saved coordinates as different
//!    pixels, so each one landed somewhere else - see [`saved_position`].
//! 3. **Putting one up takes the rest down.** Rules 1 and 2 both depend on
//!    processes we do not control calling us back, and when one does not, its
//!    bar is still on screen. So [`show`] posts a stand-down message to every
//!    window of our class on the desktop, whichever process owns it, and they
//!    hide themselves.
//!
//! The window holds a weak reference to its session so a click can flip the
//! mode. Weak, because the window outlives every individual session on its
//! thread, and because an owning reference would be a cycle.

use std::cell::{Cell, RefCell};
use std::rc::Weak;
use std::sync::OnceLock;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreatePen, CreateRoundRectRgn, CreateSolidBrush, DeleteObject,
    DrawTextW, EndPaint, FillRect, GetStockObject, InvalidateRect, MonitorFromPoint, RoundRect,
    SelectObject, SetBkMode, SetTextColor, SetWindowRgn, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS,
    DEFAULT_CHARSET, DEFAULT_PITCH, DT_CENTER, DT_SINGLELINE, DT_VCENTER, FF_DONTCARE, FW_BOLD,
    HDC, HGDIOBJ, HOLLOW_BRUSH, MONITOR_DEFAULTTONULL, MONITOR_DEFAULTTOPRIMARY,
    MONITOR_FROM_FLAGS, OUT_DEFAULT_PRECIS, PAINTSTRUCT, PS_SOLID, TRANSPARENT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, FindWindowExW, GetCursorPos, GetWindowRect,
    IsWindowVisible, LoadCursorW, PostMessageW, RegisterClassW, RegisterWindowMessageW,
    SetWindowPos, ShowWindow,
    CS_DROPSHADOW, CS_HREDRAW, CS_VREDRAW, IDC_HAND, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER,
    SW_HIDE, SW_SHOWNOACTIVATE, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCDESTROY, WM_PAINT,
    WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};

use crate::session::Session;

const CLASS_NAME: PCWSTR = w!("xlit_tsf_statusbar");
const BAR_W: i32 = 74;
const BAR_H: i32 = 30;
/// Where the mode half ends and the `+` half begins.
const SPLIT_X: i32 = 46;
/// Corner radius, used for the outline and for the window region alike.
const CORNER: i32 = 12;
/// Pointer travel, in pixels, above which a press counts as a drag not a click.
const DRAG_SLOP: i32 = 4;

// Fixed palette: the bar floats over somebody else's window and has to look
// the same wherever it lands. COLORREF is 0x00BBGGRR, not RGB.
/// The pill.
const BG: COLORREF = COLORREF(0x00_FF_FF_FF);
/// The pill while keys pass through - the same white, dimmed just enough to
/// tell the two states apart without reading the label.
const BG_OFF: COLORREF = COLORREF(0x00_F0_F0_F0);
/// Hairline edge, so the pill still reads as a surface on a white document.
const BORDER: COLORREF = COLORREF(0x00_D6_D6_D6);
/// Label and icon: grey at half strength. The pill is a flat white, so
/// compositing #555555 over it at 50% alpha is simply the colour halfway
/// between the two - (0x55 + 0xFF) / 2 - with none of the cost of an
/// alpha-blended layer.
const INK: COLORREF = COLORREF(0x00_AA_AA_AA);

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

thread_local! {
    /// This thread's one and only bar. See rule 1 in the module comment.
    static BAR: Cell<HWND> = const { Cell::new(HWND(std::ptr::null_mut())) };
}

fn bar_hwnd() -> HWND {
    BAR.with(|b| b.get())
}

/// Put this thread's bar on screen - creating it if this is the first time -
/// and take every other bar on the desktop down.
///
/// `sess` is the session whose mode it shows and toggles, rebound on every
/// call: the thread gets a new session on every activation, while the window
/// stays.
pub fn show(sess: &Weak<RefCell<Session>>) {
    let mut hwnd = bar_hwnd();
    if hwnd.is_invalid() {
        let Some(h) = create(sess.clone()) else { return };
        hwnd = h;
    } else if let Some(st) = state(hwnd) {
        if let Ok(mut b) = st.try_borrow_mut() {
            b.sess = sess.clone();
        }
    }
    unsafe {
        _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        _ = InvalidateRect(Some(hwnd), None, true);
    }
    evict_others(hwnd);
}

/// Put the bar back if it is not on screen, and otherwise do nothing.
///
/// The last line of defence, called from the key sink: whatever else went
/// wrong, the bar the user is typing under should be the one they can see.
/// `show` alone would be correct but wasteful - it walks the desktop's window
/// list - and this runs on every keystroke we claim.
pub fn ensure_visible(sess: &Weak<RefCell<Session>>) {
    let hwnd = bar_hwnd();
    if !hwnd.is_invalid() && unsafe { IsWindowVisible(hwnd) }.as_bool() {
        return;
    }
    show(sess);
}

pub fn hide() {
    let hwnd = bar_hwnd();
    if !hwnd.is_invalid() {
        unsafe { _ = ShowWindow(hwnd, SW_HIDE) };
    }
}

/// Repaint after the mode changed by some other route (the toggle key).
pub fn refresh() {
    let hwnd = bar_hwnd();
    if !hwnd.is_invalid() {
        unsafe { _ = InvalidateRect(Some(hwnd), None, true) };
    }
}

/// Take this thread's bar down for good, on `Deactivate`.
///
/// Destroying rather than hiding is what holds a thread to one window: the
/// next `Activate` builds a fresh one and there is nothing left over to
/// reappear. Idempotent, and a no-op on a thread that never showed one.
pub fn destroy() {
    let hwnd = BAR.with(|b| b.replace(HWND(std::ptr::null_mut())));
    if !hwnd.is_invalid() {
        unsafe { _ = DestroyWindow(hwnd) };
    }
}

fn create(sess: Weak<RefCell<Session>>) -> Option<HWND> {
    register_class();
    let font = unsafe {
        HGDIOBJ(
            CreateFontW(
                -16,
                0,
                0,
                0,
                FW_BOLD.0 as i32,
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
            round_corners(h);
            BAR.with(|b| b.set(h));
            Some(h)
        }
        _ => {
            drop(unsafe { Box::from_raw(state) });
            crate::debug("status bar could not be created");
            None
        }
    }
}

/// Clip the window itself to the pill shape.
///
/// Painting rounded corners is not enough: the corners of a white pill would
/// be white squares over whatever is behind them. A region cuts them out of
/// the window, and the drop shadow follows it.
fn round_corners(hwnd: HWND) {
    unsafe {
        // The region's right and bottom edges are exclusive.
        let rgn = CreateRoundRectRgn(0, 0, BAR_W + 1, BAR_H + 1, CORNER, CORNER);
        if rgn.is_invalid() {
            return;
        }
        // On success the window owns the region and frees it; on failure it is
        // still ours to delete.
        if SetWindowRgn(hwnd, Some(rgn), false) == 0 {
            _ = DeleteObject(HGDIOBJ(rgn.0));
        }
    }
}

// ---------------------------------------------------------------------------
// One bar across all processes
// ---------------------------------------------------------------------------

/// The message that asks a bar in another process to stand down.
///
/// Registered, so the id is unique desktop-wide and cannot collide with
/// anything else the window might be sent.
fn stand_down_msg() -> u32 {
    static MSG: OnceLock<u32> = OnceLock::new();
    *MSG.get_or_init(|| unsafe { RegisterWindowMessageW(w!("xlit_tsf_stand_down")) })
}

/// Ask every other bar on the desktop to hide itself.
///
/// Posted rather than sent: the owning process may be busy or wedged, and this
/// is housekeeping. A bar that never gets the message is no worse off than it
/// was, while a blocked `SendMessage` would take the typing user down with it.
fn evict_others(mine: HWND) {
    let msg = stand_down_msg();
    if msg == 0 {
        return;
    }
    let mut after: Option<HWND> = None;
    // Bounded rather than `loop`: the window list can change underneath the
    // walk, and no desktop has 256 of these.
    for _ in 0..256 {
        // `Err` is how "no more windows of this class" arrives.
        let Ok(h) = (unsafe { FindWindowExW(None, after, CLASS_NAME, PCWSTR::null()) }) else {
            return;
        };
        if h != mine {
            unsafe { _ = PostMessageW(Some(h), msg, WPARAM(0), LPARAM(0)) };
        }
        after = Some(h);
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
            // The shadow is what separates a white pill from the white
            // document underneath it.
            style: CS_HREDRAW | CS_VREDRAW | CS_DROPSHADOW,
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
    // Another process has just put its bar up; ours is the stale one now.
    if msg == stand_down_msg() {
        unsafe { _ = ShowWindow(hwnd, SW_HIDE) };
        return LRESULT(0);
    }
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

        // Flat fill, then the outline on top of it. The window region has
        // already cut the corners away, so the fill can be the whole rectangle.
        let bg = CreateSolidBrush(if enabled { BG } else { BG_OFF });
        FillRect(hdc, &RECT { left: 0, top: 0, right: w, bottom: h }, bg);
        _ = DeleteObject(HGDIOBJ(bg.0));

        let pen = CreatePen(PS_SOLID, 1, BORDER);
        let old_pen = SelectObject(hdc, HGDIOBJ(pen.0));
        // Hollow brush, or RoundRect would fill the pill with the last brush
        // selected instead of just drawing its edge.
        let old_brush = SelectObject(hdc, GetStockObject(HOLLOW_BRUSH));
        _ = RoundRect(hdc, 0, 0, w, h, CORNER, CORNER);
        SelectObject(hdc, old_brush);
        SelectObject(hdc, old_pen);
        _ = DeleteObject(HGDIOBJ(pen.0));

        if let Some(st) = state(hwnd) {
            if let Ok(b) = st.try_borrow() {
                let old = SelectObject(hdc, b.font);
                SetBkMode(hdc, TRANSPARENT);
                SetTextColor(hdc, INK);
                let mut label: Vec<u16> =
                    if enabled { "ने" } else { "EN" }.encode_utf16().collect();
                // Centred by measurement rather than by a hand-tuned offset:
                // the two labels are different scripts and different widths.
                let mut half = RECT { left: 0, top: 0, right: SPLIT_X, bottom: h };
                DrawTextW(hdc, &mut label, &mut half, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
                SelectObject(hdc, old);
            }
        }

        // Divider, then the add-word affordance.
        let line = RECT { left: SPLIT_X, top: 8, right: SPLIT_X + 1, bottom: h - 8 };
        let div = CreateSolidBrush(BORDER);
        FillRect(hdc, &line, div);
        _ = DeleteObject(HGDIOBJ(div.0));

        draw_plus(hdc, (SPLIT_X + w) / 2, h / 2);

        _ = EndPaint(hwnd, &ps);
    }
}

/// The `+`, drawn as two bars rather than typed as a glyph.
///
/// A font `+` sits wherever its face puts it — above the optical centre in
/// most UI faces, and somewhere else again in whatever GDI substitutes when
/// Nirmala UI is missing. Two rectangles are the same crisp, centred cross on
/// every machine.
fn draw_plus(hdc: HDC, cx: i32, cy: i32) {
    // Half an arm, and half the stroke: an 11px cross, 3px thick.
    const ARM: i32 = 5;
    const HALF: i32 = 1;
    unsafe {
        let ink = CreateSolidBrush(INK);
        let across =
            RECT { left: cx - ARM, top: cy - HALF, right: cx + ARM + 1, bottom: cy + HALF + 1 };
        let down =
            RECT { left: cx - HALF, top: cy - ARM, right: cx + HALF + 1, bottom: cy + ARM + 1 };
        FillRect(hdc, &across, ink);
        FillRect(hdc, &down, ink);
        _ = DeleteObject(HGDIOBJ(ink.0));
    }
}
