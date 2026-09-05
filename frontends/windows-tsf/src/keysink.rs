//! Keyboard interception.
//!
//! TSF asks twice about every key: `OnTestKeyDown` ("would you handle this?")
//! and then, only if that said yes, `OnKeyDown` ("handle it"). Both answers must
//! agree, so the decision lives in one place — [`classify`] — which reads state
//! but never changes it.
//!
//! The consequence worth knowing: a key we claim in `OnTestKeyDown` does **not**
//! reach the application afterwards, even if `OnKeyDown` changes its mind. So a
//! break key that ends a word is claimed and we re-insert its character
//! ourselves as part of the commit. That is why `Action::CommitWith` carries
//! text. We only ever claim keys mid-word; with no composition open, everything
//! except a letter falls through untouched.

use windows::core::{implement, Ref, Result, BOOL, GUID};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, GetKeyboardState, ToUnicode, VK_BACK, VK_CAPITAL, VK_CONTROL, VK_DOWN, VK_ESCAPE,
    VK_MENU, VK_NUMPAD0, VK_NUMPAD9, VK_SHIFT, VK_UP,
};
use windows::Win32::UI::TextServices::{ITfContext, ITfKeyEventSink, ITfKeyEventSink_Impl};

use crate::session::{self, SharedSession};

#[implement(ITfKeyEventSink)]
pub struct KeyEventSink {
    sess: SharedSession,
}

impl KeyEventSink {
    pub fn new(sess: SharedSession) -> Self {
        KeyEventSink { sess }
    }
}

/// What a keystroke means to us right now.
enum Action {
    /// Not ours — let the application have it.
    Ignore,
    /// Extend the Latin buffer.
    Letter(char),
    /// Shorten it.
    Backspace,
    /// Throw the conversion away, keep the Latin.
    Cancel,
    /// Commit candidate `n` (0-based).
    Pick(usize),
    /// Move the highlight by `n` places.
    Move(i32),
    /// Finish the word, then insert this text after it.
    CommitWith(String),
    /// Finish the word and insert nothing.
    Commit,
    /// A number-row key that is not picking a candidate: type the Devanagari
    /// digit, committing any word in progress first.
    Digit(char),
}

fn down(vk: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY) -> bool {
    unsafe { GetKeyState(vk.0 as i32) < 0 }
}

fn caps_lock() -> bool {
    unsafe { GetKeyState(VK_CAPITAL.0 as i32) & 1 != 0 }
}

/// The character this key produces on the user's actual layout, or `None` for
/// keys that produce none (Enter, Tab, arrows, function keys).
///
/// `ToUnicode` normally consumes pending dead-key state; bit 2 of `wflags`
/// asks it not to, which matters because we call this from the *test* pass too.
fn char_for_key(vk: u32, scan: u32) -> Option<char> {
    unsafe {
        let mut state = [0u8; 256];
        GetKeyboardState(&mut state).ok()?;
        let mut buf = [0u16; 8];
        let n = ToUnicode(vk, scan, Some(&state), &mut buf, 4);
        if n <= 0 {
            return None;
        }
        let c = char::decode_utf16(buf[..n as usize].iter().copied())
            .next()?
            .ok()?;
        // Control characters (Enter -> \r, Tab -> \t, Esc) are not text.
        (!c.is_control()).then_some(c)
    }
}

/// Decide what a key means. Pure: safe to call from `OnTestKeyDown`.
fn classify(vk: u16, scan: u32, composing: bool, ncands: usize) -> Action {
    // Any Ctrl/Alt chord belongs to the application (Ctrl+A, Alt+F4, ...).
    // Mid-word that means our composition stays open; if the app disturbs the
    // document, it tells us via OnCompositionTerminated.
    if down(VK_CONTROL) || down(VK_MENU) {
        return Action::Ignore;
    }
    let shift = down(VK_SHIFT);

    // Letters always start or extend a word. The Nepali schema is
    // case-sensitive (M = anusvara, T = ट, S = श), so preserve the case.
    if (0x41..=0x5A).contains(&vk) {
        let upper = (vk as u8) as char;
        let c = if shift ^ caps_lock() { upper } else { upper.to_ascii_lowercase() };
        return Action::Letter(c);
    }

    // The number row and the numeric keypad both do double duty. Mid-word they
    // pick from the candidate list, the way every other IME does; the rest of
    // the time they type a Devanagari digit.
    //
    // Shift+number on the main row is punctuation (@, #, ...) and is neither.
    // The keypad needs no such guard: with NumLock on, Shift turns those keys
    // back into Home/End/arrows, so a VK_NUMPAD* only ever arrives as a plain
    // digit — and with NumLock off they never arrive as one at all.
    let digit = match vk {
        0x30..=0x39 if !shift => Some(vk - 0x30),
        v if (VK_NUMPAD0.0..=VK_NUMPAD9.0).contains(&v) => Some(vk - VK_NUMPAD0.0),
        _ => None,
    };
    if let Some(d) = digit {
        // 1-9 pick candidates 0-8; 0 picks nothing.
        if composing && d != 0 && ((d - 1) as usize) < ncands {
            return Action::Pick((d - 1) as usize);
        }
        return Action::Digit((b'0' + d as u8) as char);
    }

    if !composing {
        return Action::Ignore;
    }

    match vk {
        v if v == VK_BACK.0 => Action::Backspace,
        v if v == VK_ESCAPE.0 => Action::Cancel,
        v if v == VK_UP.0 => Action::Move(-1),
        v if v == VK_DOWN.0 => Action::Move(1),
        _ => match char_for_key(vk as u32, scan) {
            Some(c) => Action::CommitWith(c.to_string()),
            None => Action::Commit,
        },
    }
}

impl KeyEventSink_Impl {
    /// Shared prologue: bail out unless we have a context and are switched on.
    fn state(&self) -> Option<(bool, usize)> {
        let s = self.sess.borrow();
        s.enabled.then(|| (s.composing(), s.cands.len()))
    }
}

impl ITfKeyEventSink_Impl for KeyEventSink_Impl {
    /// Focus moved between documents.
    ///
    /// This is one of the three rules that keep the floating bar singular -
    /// see [`crate::bar`]. Every process that takes input has its own session
    /// and its own bar, and only the one whose document holds the focus is on
    /// screen; showing it is also what tells the others to stand down.
    fn OnSetFocus(&self, fforeground: BOOL) -> Result<()> {
        crate::debug(&format!("OnSetFocus foreground={}", fforeground.as_bool()));
        // The candidate popup belongs to the document we just left. Borrow
        // only if it is free: this arrives from other people's processes at
        // moments we do not choose, and a panic here aborts them.
        if let Ok(mut s) = self.sess.try_borrow_mut() {
            s.window.hide();
        }
        if fforeground.as_bool() {
            crate::bar::show(&std::rc::Rc::downgrade(&self.sess));
        } else {
            crate::bar::hide();
        }
        Ok(())
    }

    fn OnTestKeyDown(
        &self,
        pic: Ref<'_, ITfContext>,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> Result<BOOL> {
        if pic.is_null() {
            return Ok(BOOL(0));
        }
        let Some((composing, ncands)) = self.state() else {
            return Ok(BOOL(0));
        };
        let vk = (wparam.0 & 0xFFFF) as u16;
        let scan = ((lparam.0 >> 16) & 0xFF) as u32;
        let eaten = !matches!(classify(vk, scan, composing, ncands), Action::Ignore);
        Ok(BOOL(eaten.into()))
    }

    fn OnKeyDown(&self, pic: Ref<'_, ITfContext>, wparam: WPARAM, lparam: LPARAM) -> Result<BOOL> {
        // Mirror OnTestKeyDown's null guard exactly. Returning `Err` here
        // would leave TSF's `BOOL *pfEaten` out-parameter unwritten —
        // windows-rs returns the HRESULT without touching it — and the
        // framework then branches on whatever was in that memory.
        if pic.is_null() {
            return Ok(BOOL(0));
        }
        let ctx = pic.ok()?;
        // Stash the context so a click on the status bar, which has none of its
        // own, can still commit a word in progress.
        if let Ok(mut s) = self.sess.try_borrow_mut() {
            s.last_ctx = Some(ctx.clone());
        }
        // Typing is the strongest evidence there is of which application the
        // bar belongs over, and the focus callbacks that normally decide it
        // come from processes we do not control. If one went missing, this is
        // where the bar comes back.
        crate::bar::ensure_visible(&std::rc::Rc::downgrade(&self.sess));
        let Some((composing, ncands)) = self.state() else {
            return Ok(BOOL(0));
        };
        let vk = (wparam.0 & 0xFFFF) as u16;
        let scan = ((lparam.0 >> 16) & 0xFF) as u32;
        crate::debug(&format!("OnKeyDown vk=0x{vk:02X} composing={composing}"));

        match classify(vk, scan, composing, ncands) {
            Action::Ignore => return Ok(BOOL(0)),
            Action::Letter(c) => {
                // A control that refuses composition still gets the letter,
                // as a plain literal insert, so the keyboard degrades to
                // Latin rather than going dead. What we must not do is call
                // it unhandled: OnTestKeyDown already claimed the key, and
                // answering BOOL(0) leaves TSF's record of the keystroke
                // disagreeing with the edit we just made.
                if !session::insert(&self.sess, ctx, c) {
                    session::insert_literal(&self.sess, ctx, &c.to_string());
                }
            }
            Action::Backspace => session::backspace(&self.sess, ctx),
            Action::Cancel => session::cancel(&self.sess, ctx),
            Action::Move(d) => session::move_selection(&self.sess, ctx, d),
            Action::Pick(i) => {
                session::select_and_commit(&self.sess, ctx, i, "");
            }
            Action::CommitWith(tail) => session::commit(&self.sess, ctx, &tail),
            Action::Commit => session::commit(&self.sess, ctx, ""),
            Action::Digit(d) => {
                let deva = crate::engine::literal(d);
                if composing {
                    session::commit(&self.sess, ctx, &deva);
                } else {
                    // Report the key as handled whether or not the edit landed.
                    // OnTestKeyDown already claimed it, so it never reaches the
                    // application either way; answering BOOL(0) here does not
                    // hand the digit back, it only tells TSF something other
                    // than what OnTestKeyDown promised.
                    session::insert_literal(&self.sess, ctx, &deva);
                }
            }
        }
        Ok(BOOL(1))
    }

    fn OnTestKeyUp(&self, _pic: Ref<'_, ITfContext>, _w: WPARAM, _l: LPARAM) -> Result<BOOL> {
        Ok(BOOL(0))
    }

    fn OnKeyUp(&self, _pic: Ref<'_, ITfContext>, _w: WPARAM, _l: LPARAM) -> Result<BOOL> {
        Ok(BOOL(0))
    }

    /// The toggle chord (see `service.rs`). Flips between transliterating and
    /// passing keys straight through, committing anything half-typed first.
    fn OnPreservedKey(&self, pic: Ref<'_, ITfContext>, _rguid: *const GUID) -> Result<BOOL> {
        if let Ok(ctx) = pic.ok() {
            if self.sess.borrow().composing() {
                session::commit(&self.sess, ctx, "");
            }
        }
        let enabled = {
            let mut s = self.sess.borrow_mut();
            s.enabled = !s.enabled;
            s.enabled
        };
        crate::debug(if enabled { "enabled" } else { "passthrough" });
        crate::bar::refresh();
        Ok(BOOL(1))
    }
}
