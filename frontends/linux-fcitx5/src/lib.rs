//! Fcitx5 input method — the Rust half.
//!
//! Fcitx5 addons are C++ shared libraries implementing `fcitx::InputMethodEngine`;
//! there is no Rust API and no D-Bus route in (unlike IBus, where an engine is
//! just a program on the bus). So this crate is a `cdylib` exposing a small C
//! ABI, and [`cpp/xlit-engine.cpp`](../cpp/xlit-engine.cpp) is a thin C++ addon
//! that translates Fcitx5's callbacks into calls on it.
//!
//! Everything that decides behaviour lives in [`state`], which knows nothing
//! about any framework and is unit-tested on its own. The C ABI below is only
//! marshalling.
//!
//! # Safety contract for the C++ side
//!
//! * `xlit_new` returns a handle, or null if the engine could not be built.
//! * Every other function takes that handle and must not be called after
//!   `xlit_free`.
//! * Returned `const char*` point into the handle and stay valid until the next
//!   call on it. Copy before calling again.
//! * One handle per input context; a handle is not thread-safe.

pub mod engine;
pub mod state;

use std::ffi::{c_char, CString};

use state::{Action, State};

/// One input context. Owns the strings handed back to C++, so that the caller
/// never has to free anything.
pub struct XlitState {
    inner: State,
    last: Action,
    commit: CString,
    preedit: CString,
    candidates: Vec<CString>,
}

impl XlitState {
    fn new() -> Self {
        XlitState {
            inner: State::new(),
            last: Action::default(),
            commit: CString::default(),
            preedit: CString::default(),
            candidates: Vec::new(),
        }
    }

    /// Cache the action as NUL-terminated strings the C++ side can read.
    ///
    /// Interior NULs cannot occur — every string here is either Latin the user
    /// typed or Devanagari from the dictionary — but `CString::new` is fallible,
    /// so an empty string stands in rather than a panic across an FFI boundary.
    fn store(&mut self, action: Action) {
        self.commit = CString::new(action.commit.as_str()).unwrap_or_default();
        self.preedit = CString::new(action.preedit.as_str()).unwrap_or_default();
        self.candidates = action
            .candidates
            .iter()
            .map(|c| CString::new(c.as_str()).unwrap_or_default())
            .collect();
        self.last = action;
    }
}

/// Fcitx5's release bit, mirrored from [`state::modifier`] so the FFI layer can
/// recognise a release without reaching into the state machine.
const RELEASE: u32 = crate::state::modifier::RELEASE;

/// Create an input context. Null on failure.
#[no_mangle]
pub extern "C" fn xlit_new() -> *mut XlitState {
    Box::into_raw(Box::new(XlitState::new()))
}

/// Destroy an input context. Null is accepted and ignored.
///
/// # Safety
/// `handle` must have come from `xlit_new` and must not be used afterwards.
#[no_mangle]
pub unsafe extern "C" fn xlit_free(handle: *mut XlitState) {
    if !handle.is_null() {
        drop(Box::from_raw(handle));
    }
}

/// Feed a key event. Returns 1 if it was consumed, 0 to pass it to the
/// application. The results are then read with the accessors below.
///
/// # Safety
/// `handle` must be a live handle from `xlit_new`.
#[no_mangle]
pub unsafe extern "C" fn xlit_process_key(
    handle: *mut XlitState,
    keysym: u32,
    modifiers: u32,
) -> i32 {
    let Some(s) = handle.as_mut() else { return 0 };
    let action = s.inner.key(keysym, modifiers);
    let handled = action.handled;
    // A release produces no output, so storing its empty action would throw
    // away the preedit and candidates the press just produced — and the caller
    // would then read an empty commit string over a word it has not committed
    // yet. Report whether to claim the key and leave the state alone.
    if modifiers & RELEASE == 0 {
        s.store(action);
    }
    i32::from(handled)
}

/// Drop the word in progress without committing it — focus loss, or a reset.
///
/// # Safety
/// `handle` must be a live handle from `xlit_new`.
#[no_mangle]
pub unsafe extern "C" fn xlit_reset(handle: *mut XlitState) {
    if let Some(s) = handle.as_mut() {
        let action = s.inner.reset();
        s.store(action);
    }
}

/// Text to commit after the last key, or "" if there is none.
///
/// # Safety
/// `handle` must be live; the pointer is valid until the next call on it.
#[no_mangle]
pub unsafe extern "C" fn xlit_commit(handle: *const XlitState) -> *const c_char {
    match handle.as_ref() {
        Some(s) => s.commit.as_ptr(),
        None => c"".as_ptr(),
    }
}

/// The preedit to display — the raw Latin, never a conversion.
///
/// # Safety
/// `handle` must be live; the pointer is valid until the next call on it.
#[no_mangle]
pub unsafe extern "C" fn xlit_preedit(handle: *const XlitState) -> *const c_char {
    match handle.as_ref() {
        Some(s) => s.preedit.as_ptr(),
        None => c"".as_ptr(),
    }
}

/// How many candidates to show.
///
/// # Safety
/// `handle` must be live.
#[no_mangle]
pub unsafe extern "C" fn xlit_candidate_count(handle: *const XlitState) -> usize {
    handle.as_ref().map_or(0, |s| s.candidates.len())
}

/// Candidate `index`, or "" if out of range.
///
/// # Safety
/// `handle` must be live; the pointer is valid until the next call on it.
#[no_mangle]
pub unsafe extern "C" fn xlit_candidate(handle: *const XlitState, index: usize) -> *const c_char {
    match handle.as_ref().and_then(|s| s.candidates.get(index)) {
        Some(c) => c.as_ptr(),
        None => c"".as_ptr(),
    }
}

/// Which candidate is highlighted.
///
/// # Safety
/// `handle` must be live.
#[no_mangle]
pub unsafe extern "C" fn xlit_cursor(handle: *const XlitState) -> usize {
    handle.as_ref().map_or(0, |s| s.last.cursor)
}

#[cfg(test)]
mod ffi_tests {
    use super::*;
    use crate::state::{key, modifier};

    /// Press then release, as Fcitx5 actually delivers them. The release must
    /// leave the candidate list alone: storing its empty action drew the list
    /// on the press and erased it on the release, one flicker per keystroke.
    #[test]
    fn a_release_does_not_wipe_what_the_press_produced() {
        unsafe {
            let s = xlit_new();
            assert!(!s.is_null());
            for c in "ne".chars() {
                xlit_process_key(s, c as u32, 0);
            }
            let before = xlit_candidate_count(s);
            assert!(before > 1, "expected candidates, got {before}");

            let handled = xlit_process_key(s, 'e' as u32, modifier::RELEASE);
            assert_eq!(handled, 1, "a release is claimed while composing");
            assert_eq!(xlit_candidate_count(s), before);
            assert!(!xlit_preedit(s).is_null());

            // And a release with nothing in progress is not claimed at all.
            xlit_reset(s);
            assert_eq!(xlit_process_key(s, key::SPACE, modifier::RELEASE), 0);
            xlit_free(s);
        }
    }
}
