//! The candidate popup.
//!
//! A plain Win32 layered window rather than TSF's `ITfCandidateListUIElement`:
//! the UI-less protocol only lets the *application* draw the list, and almost
//! none do, so a TIP that wants a list visible everywhere draws it itself.
//!
//! Two constraints shape this:
//!
//! * **It must never take focus.** `WS_EX_NOACTIVATE` plus `SW_SHOWNOACTIVATE`;
//!   the moment the popup steals focus the composition it is describing dies.
//! * **It must render Devanagari.** The default GUI font does not, so we ask
//!   for *Nirmala UI* — the Windows Indic UI face — and let GDI substitute if
//!   it is somehow missing.
//!
//! Paint state lives in a box owned by the window (via `GWLP_USERDATA`) instead
//! of in `CandWindow`, so the window procedure never reaches back into the
//! session — it can be called re-entrantly and must not need a `RefCell` we
//! might already hold.

use std::cell::RefCell;
use std::sync::OnceLock;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, EndPaint, FillRect, GetDC,
    GetSysColor, GetTextExtentPoint32W, InvalidateRect, MonitorFromPoint, ReleaseDC, SelectObject,
    SetBkMode, SetTextColor, TextOutW, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, COLOR_HIGHLIGHT,
    COLOR_HIGHLIGHTTEXT, COLOR_WINDOW, COLOR_WINDOWTEXT, DEFAULT_CHARSET, DEFAULT_PITCH,
    FF_DONTCARE, FW_NORMAL, HDC, HFONT, HGDIOBJ, MONITOR_DEFAULTTONEAREST, OUT_DEFAULT_PRECIS,
    PAINTSTRUCT, SYS_COLOR_INDEX, TRANSPARENT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, LoadCursorW, RegisterClassW, SetWindowPos,
    ShowWindow, CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA, HWND_TOPMOST, IDC_ARROW,
    SWP_NOACTIVATE, SW_HIDE, SW_SHOWNOACTIVATE, WM_NCDESTROY, WM_PAINT, WNDCLASSW,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

const CLASS_NAME: PCWSTR = w!("xlit_tsf_candidates");
/// Point size of the list, in logical units.
const FONT_HEIGHT: i32 = -18;
/// Breathing room around the text block.
const PAD_X: i32 = 10;
const PAD_Y: i32 = 6;
/// Extra leading between rows.
const ROW_GAP: i32 = 6;
/// Gap between the composition and the top of the popup.
const ANCHOR_GAP: i32 = 4;

/// What the window procedure needs in order to paint. Owned by the window.
struct Paint {
    rows: Vec<String>,
    sel: usize,
    font: HFONT,
}

impl Drop for Paint {
    fn drop(&mut self) {
        unsafe { _ = DeleteObject(HGDIOBJ(self.font.0)) };
    }
}

pub struct CandWindow {
    hwnd: HWND,
    visible: bool,
    /// Whether we have ever had a caret rectangle to anchor to.
    placed: bool,
}

impl CandWindow {
    pub const fn new() -> Self {
        CandWindow { hwnd: HWND(std::ptr::null_mut()), visible: false, placed: false }
    }

    /// Draw `cands` with `sel` highlighted, hung under `at` — the screen
    /// rectangle of the composition. `None` means the control could not tell us
    /// where the caret is; we then keep the last position rather than guessing,
    /// and stay hidden entirely if we have never had one — a list pinned to the
    /// corner of the screen is worse than no list.
    pub fn show(&mut self, cands: &[String], sel: usize, at: Option<RECT>) {
        if cands.len() < 2 || (at.is_none() && !self.placed) {
            // A single candidate is already inline in the composition; a
            // one-row list would just be noise.
            self.hide();
            return;
        }
        if self.hwnd.is_invalid() && !self.create() {
            return;
        }
        let rows: Vec<String> = cands
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{}. {}", i + 1, c))
            .collect();

        let Some((w, h)) = self.update_paint(rows, sel) else { return };
        if let Some(rc) = at {
            self.place(rc, w, h);
            self.placed = true;
        }
        unsafe {
            _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            _ = InvalidateRect(Some(self.hwnd), None, true);
        }
        self.visible = true;
    }

    pub fn hide(&mut self) {
        if self.visible && !self.hwnd.is_invalid() {
            unsafe { _ = ShowWindow(self.hwnd, SW_HIDE) };
        }
        self.visible = false;
    }

    /// Swap in new rows and measure them. Returns the window size needed.
    fn update_paint(&self, rows: Vec<String>, sel: usize) -> Option<(i32, i32)> {
        let state = paint_state(self.hwnd)?;
        let mut p = state.try_borrow_mut().ok()?;
        p.rows = rows;
        p.sel = sel;

        let n = p.rows.len() as i32;
        let (mut wide, mut line) = (0i32, 0i32);
        unsafe {
            let hdc = GetDC(Some(self.hwnd));
            let old = SelectObject(hdc, HGDIOBJ(p.font.0));
            for r in &p.rows {
                let w: Vec<u16> = r.encode_utf16().collect();
                let mut sz = Default::default();
                if GetTextExtentPoint32W(hdc, &w, &mut sz).as_bool() {
                    wide = wide.max(sz.cx);
                    line = line.max(sz.cy);
                }
            }
            SelectObject(hdc, old);
            ReleaseDC(Some(self.hwnd), hdc);
        }
        if line == 0 {
            return None;
        }
        Some((wide + PAD_X * 2, n * (line + ROW_GAP) + PAD_Y * 2))
    }

    /// Hang the popup under the composition, flipping above it and sliding
    /// sideways as needed to stay on the monitor.
    fn place(&self, at: RECT, w: i32, h: i32) {
        let mut x = at.left;
        let mut y = at.bottom + ANCHOR_GAP;
        if let Some(mon) = work_area(at.left, at.bottom) {
            if x + w > mon.right {
                x = (mon.right - w).max(mon.left);
            }
            if y + h > mon.bottom {
                // No room below: sit above the line instead.
                y = (at.top - ANCHOR_GAP - h).max(mon.top);
            }
        }
        unsafe {
            _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), x, y, w, h, SWP_NOACTIVATE);
        }
    }

    fn create(&mut self) -> bool {
        register_class();
        let font = unsafe {
            CreateFontW(
                FONT_HEIGHT,
                0,
                0,
                0,
                FW_NORMAL.0 as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                DEFAULT_PITCH.0 as u32 | FF_DONTCARE.0 as u32,
                // Nirmala UI is the Windows Devanagari UI face; without it the
                // candidates render as boxes.
                w!("Nirmala UI"),
            )
        };
        let state: *mut RefCell<Paint> = Box::into_raw(Box::new(RefCell::new(Paint {
            rows: Vec::new(),
            sel: 0,
            font,
        })));

        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                CLASS_NAME,
                PCWSTR::null(),
                WS_POPUP,
                0,
                0,
                0,
                0,
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
                // Reclaim the state the window never took ownership of.
                drop(unsafe { Box::from_raw(state) });
                crate::debug("candidate window could not be created");
                false
            }
        }
    }
}

impl Drop for CandWindow {
    fn drop(&mut self) {
        if !self.hwnd.is_invalid() {
            unsafe { _ = DestroyWindow(self.hwnd) };
        }
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
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: CLASS_NAME,
            ..Default::default()
        };
        // A second load of this DLL in the same process finds the class already
        // there; the failure is expected and harmless.
        RegisterClassW(&class);
    });
}

/// The paint state a window is carrying, if any.
fn paint_state<'a>(hwnd: HWND) -> Option<&'a RefCell<Paint>> {
    if hwnd.is_invalid() {
        return None;
    }
    let ptr = unsafe { get_userdata(hwnd) } as *const RefCell<Paint>;
    unsafe { ptr.as_ref() }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            // CREATESTRUCT.lpCreateParams carries the boxed paint state.
            windows::Win32::UI::WindowsAndMessaging::WM_NCCREATE => {
                let cs = lparam.0 as *const windows::Win32::UI::WindowsAndMessaging::CREATESTRUCTW;
                if let Some(cs) = cs.as_ref() {
                    set_userdata(hwnd, cs.lpCreateParams as isize);
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_PAINT => {
                paint(hwnd);
                LRESULT(0)
            }
            WM_NCDESTROY => {
                let ptr = get_userdata(hwnd) as *mut RefCell<Paint>;
                if !ptr.is_null() {
                    set_userdata(hwnd, 0);
                    drop(Box::from_raw(ptr));
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

fn paint(hwnd: HWND) {
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut ps);
        if let Some(state) = paint_state(hwnd) {
            if let Ok(p) = state.try_borrow() {
                draw(hdc, &ps.rcPaint, &p);
            }
        }
        _ = EndPaint(hwnd, &ps);
    }
}

fn draw(hdc: HDC, rc: &RECT, p: &Paint) {
    unsafe {
        let bg = CreateSolidBrush(sys(COLOR_WINDOW));
        FillRect(hdc, rc, bg);
        _ = DeleteObject(HGDIOBJ(bg.0));

        let old = SelectObject(hdc, HGDIOBJ(p.font.0));
        SetBkMode(hdc, TRANSPARENT);

        let mut y = PAD_Y;
        for (i, row) in p.rows.iter().enumerate() {
            let text: Vec<u16> = row.encode_utf16().collect();
            let mut sz = Default::default();
            _ = GetTextExtentPoint32W(hdc, &text, &mut sz);
            let row_h = sz.cy + ROW_GAP;

            if i == p.sel {
                let hl = CreateSolidBrush(sys(COLOR_HIGHLIGHT));
                let band = RECT {
                    left: rc.left,
                    top: y - ROW_GAP / 2,
                    right: rc.right,
                    bottom: y - ROW_GAP / 2 + row_h,
                };
                FillRect(hdc, &band, hl);
                _ = DeleteObject(HGDIOBJ(hl.0));
                SetTextColor(hdc, sys(COLOR_HIGHLIGHTTEXT));
            } else {
                SetTextColor(hdc, sys(COLOR_WINDOWTEXT));
            }
            _ = TextOutW(hdc, PAD_X, y, &text);
            y += row_h;
        }
        SelectObject(hdc, old);
    }
}

fn sys(index: SYS_COLOR_INDEX) -> COLORREF {
    COLORREF(unsafe { GetSysColor(index) })
}

/// Work area (screen minus taskbar) of the monitor holding a point.
fn work_area(x: i32, y: i32) -> Option<RECT> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITORINFO};
    unsafe {
        let mon = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        GetMonitorInfoW(mon, &mut info).as_bool().then_some(info.rcWork)
    }
}

// `GetWindowLongPtrW` only exists on 64-bit; the 32-bit build (which a TSF
// service needs, for 32-bit host apps) uses the non-Ptr form.
#[cfg(target_pointer_width = "64")]
unsafe fn get_userdata(hwnd: HWND) -> isize {
    windows::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(hwnd, GWLP_USERDATA)
}
#[cfg(target_pointer_width = "64")]
unsafe fn set_userdata(hwnd: HWND, v: isize) {
    windows::Win32::UI::WindowsAndMessaging::SetWindowLongPtrW(hwnd, GWLP_USERDATA, v);
}
#[cfg(not(target_pointer_width = "64"))]
unsafe fn get_userdata(hwnd: HWND) -> isize {
    windows::Win32::UI::WindowsAndMessaging::GetWindowLongW(hwnd, GWLP_USERDATA) as isize
}
#[cfg(not(target_pointer_width = "64"))]
unsafe fn set_userdata(hwnd: HWND, v: isize) {
    windows::Win32::UI::WindowsAndMessaging::SetWindowLongW(hwnd, GWLP_USERDATA, v as i32);
}
