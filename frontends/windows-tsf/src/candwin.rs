//! Candidate window (M6.3).
//!
//! Placeholder with the shape the session already calls. M6.2 converts inline
//! with no UI; the popup lands next.

use windows::Win32::Foundation::RECT;

pub struct CandWindow {
    visible: bool,
}

impl CandWindow {
    pub fn new() -> Self {
        CandWindow { visible: false }
    }

    /// Draw `cands` with `sel` highlighted, anchored under `at` (screen
    /// coordinates of the composition) when the control could tell us where
    /// that is.
    pub fn show(&mut self, cands: &[String], _sel: usize, _at: Option<RECT>) {
        self.visible = !cands.is_empty();
    }

    pub fn hide(&mut self) {
        self.visible = false;
    }

    pub fn visible(&self) -> bool {
        self.visible
    }
}
