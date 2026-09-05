//! Per-thread editing state: the Latin buffer and the TSF composition showing it.
//!
//! While the user types letters we hold an open **composition** — the range the
//! app renders underlined as provisional text — and rewrite its whole contents
//! from the buffer on every keystroke. Rewriting wholesale (rather than editing
//! the tail) is what makes Backspace trivial: drop a Latin char, re-run the
//! engine, replace the text. There is never a mapping to maintain between Latin
//! characters and the Devanagari they produced.
//!
//! The composition shows the **raw Latin**, exactly as typed. Showing the top
//! candidate there instead meant a dictionary guess appeared under the caret
//! and changed with every keystroke — text the user never typed, moving while
//! they typed. Conversion happens once, at commit, using the candidate the
//! popup has highlighted; the popup is where alternatives are seen and chosen.
//!
//! All document access happens inside [`crate::editsession`] closures. Every
//! entry point here borrows the shared state only briefly and always drops that
//! borrow before opening an edit session, because the session calls straight
//! back into us.

use std::cell::RefCell;
use std::mem::ManuallyDrop;
use std::rc::{Rc, Weak};

use windows::core::{implement, Interface, Ref, Result, BOOL};
use windows::Win32::UI::TextServices::{
    ITfComposition, ITfCompositionSink, ITfCompositionSink_Impl, ITfContext, ITfContextComposition,
    ITfInsertAtSelection, ITfRange, INSERT_TEXT_AT_SELECTION_FLAGS, TF_AE_END, TF_ANCHOR_END,
    TF_IAS_QUERYONLY,
    TF_SELECTION, TF_SELECTIONSTYLE,
};

use crate::candwin::CandWindow;
use crate::{editsession, engine};

pub type SharedSession = Rc<RefCell<Session>>;

pub struct Session {
    /// TSF client id for this thread; every edit session needs it.
    pub tid: u32,
    /// What the user typed, in Latin.
    pub buf: String,
    /// Ranked candidates for `buf`, best first.
    pub cands: Vec<String>,
    /// Index into `cands` that Space / Enter would commit.
    pub sel: usize,
    /// Whether the user actually picked from the list for this word, rather
    /// than accepting whatever was on top. Only a real choice is worth
    /// learning — see [`commit`].
    chose: bool,
    /// The open composition, if we are mid-word.
    comp: Option<ITfComposition>,
    /// False after the toggle key: keystrokes pass straight through.
    pub enabled: bool,
    /// The candidate popup. Created lazily on the first word typed.
    pub window: CandWindow,
    /// The context of the last key event, so the status bar — which is clicked
    /// with the mouse and so has no context of its own — can still commit a
    /// word in progress before switching modes.
    pub last_ctx: Option<ITfContext>,
}

impl Session {
    pub fn new(tid: u32) -> SharedSession {
        Rc::new(RefCell::new(Session {
            tid,
            buf: String::new(),
            cands: Vec::new(),
            sel: 0,
            chose: false,
            comp: None,
            enabled: true,
            window: CandWindow::new(),
            last_ctx: None,
        }))
    }

    pub fn composing(&self) -> bool {
        self.comp.is_some() || !self.buf.is_empty()
    }

    /// What commit would write: the highlighted candidate, or the raw Latin
    /// before the engine has said anything.
    pub fn preview(&self) -> String {
        self.cands.get(self.sel).cloned().unwrap_or_else(|| self.buf.clone())
    }

    /// Text the composition displays while the word is in progress — the raw
    /// Latin, never a candidate. See the module comment.
    pub fn display(&self) -> String {
        self.buf.clone()
    }

    /// Drop all word state. Used when the composition is gone (committed,
    /// cancelled, or terminated by the application).
    fn clear(&mut self) {
        self.buf.clear();
        self.cands.clear();
        self.sel = 0;
        self.chose = false;
        self.comp = None;
        self.window.hide();
    }
}

// ---------------------------------------------------------------------------
// Key-driven operations. Each returns once the document has been updated.
// ---------------------------------------------------------------------------

/// Append a Latin character and re-render.
///
/// Returns false if the document refused the edit, having first put the buffer
/// back the way it was. The caller must still report the key as handled — we
/// claim every letter in `OnTestKeyDown` — so it writes the character as a
/// plain literal instead, and the keyboard degrades to Latin rather than
/// going dead.
pub fn insert(sess: &SharedSession, ctx: &ITfContext, c: char) -> bool {
    let tid = {
        let mut s = sess.borrow_mut();
        s.buf.push(c);
        s.cands = engine::candidates(&s.buf);
        s.sel = 0;
        s.tid
    };
    if render(sess, ctx, tid) {
        return true;
    }
    let mut s = sess.borrow_mut();
    s.buf.pop();
    s.cands = engine::candidates(&s.buf);
    s.sel = 0;
    false
}

/// Remove the last Latin character. Empties the composition entirely when the
/// buffer runs out, so Backspace never leaves a stray empty composition behind.
pub fn backspace(sess: &SharedSession, ctx: &ITfContext) {
    let (tid, empty) = {
        let mut s = sess.borrow_mut();
        s.buf.pop();
        s.cands = engine::candidates(&s.buf);
        s.sel = 0;
        (s.tid, s.buf.is_empty())
    };
    if empty {
        finish(sess, ctx, "");
    } else {
        render(sess, ctx, tid);
    }
}

/// Move the highlight within the candidate list and re-render the preview.
pub fn move_selection(sess: &SharedSession, ctx: &ITfContext, delta: i32) {
    let tid = {
        let mut s = sess.borrow_mut();
        if s.cands.is_empty() {
            return;
        }
        let n = s.cands.len() as i32;
        s.sel = (((s.sel as i32 + delta) % n + n) % n) as usize;
        s.chose = true;
        s.tid
    };
    render(sess, ctx, tid);
}

/// Pick candidate `idx` (0-based) and commit it. No-op if out of range.
pub fn select_and_commit(sess: &SharedSession, ctx: &ITfContext, idx: usize, tail: &str) -> bool {
    {
        let mut s = sess.borrow_mut();
        if idx >= s.cands.len() {
            return false;
        }
        s.sel = idx;
        s.chose = true;
    }
    commit(sess, ctx, tail);
    true
}

/// Finalise the word: write the highlighted candidate (plus `tail`, the
/// break character that triggered the commit), teach the learning store, and
/// close the composition.
pub fn commit(sess: &SharedSession, ctx: &ITfContext, tail: &str) {
    let (input, chosen, chose) = {
        let s = sess.borrow();
        (s.buf.clone(), s.preview(), s.chose)
    };
    finish(sess, ctx, &format!("{chosen}{tail}"));

    // Learn only what the user actually chose.
    //
    // Accepting the top candidate by pressing space is not a choice, it is just
    // typing, and recording it was a ratchet: the first answer for an input -
    // right or wrong - was written at a score that outranks the dictionary, so
    // it won for ever after. That is how "naam" came to mean काम and stayed
    // that way even once the ranking behind it was fixed.
    if chose && !input.is_empty() && chosen != input {
        engine::commit(&input, &chosen);
    }
}

/// Abandon the conversion, leaving exactly the Latin the user typed.
pub fn cancel(sess: &SharedSession, ctx: &ITfContext) {
    let raw = sess.borrow().buf.clone();
    finish(sess, ctx, &raw);
}

/// Insert `text` straight into the document with no composition.
///
/// For characters that need no conversion pass and no candidate list — a
/// Devanagari digit typed between words, where there is no composition open to
/// commit into and starting one just to end it immediately would flicker.
pub fn insert_literal(sess: &SharedSession, ctx: &ITfContext, text: &str) -> bool {
    let tid = sess.borrow().tid;
    let text: Vec<u16> = text.encode_utf16().collect();
    let r = editsession::run(ctx, tid, move |ec, ctx| {
        let insert: ITfInsertAtSelection = ctx.cast()?;
        // Flags 0, not TF_IAS_NOQUERY: NOQUERY performs the insert but leaves
        // `ppRange` NULL, and windows-rs turns a NULL out-parameter into an
        // `Err`. So the text went in and we still reported failure, which left
        // TSF and this sink disagreeing about whether the key was handled.
        let range = unsafe {
            insert.InsertTextAtSelection(ec, INSERT_TEXT_AT_SELECTION_FLAGS(0), &text)?
        };
        let end = unsafe { range.Clone()? };
        unsafe { end.Collapse(ec, TF_ANCHOR_END)? };
        set_selection(ec, ctx, end)
    });
    match r {
        Ok(()) => true,
        Err(e) => {
            crate::debug(&format!("literal insert failed: {e}"));
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Document plumbing
// ---------------------------------------------------------------------------

/// Push the current preview into the composition, opening one if needed.
/// Returns whether the document accepted the edit.
fn render(sess: &SharedSession, ctx: &ITfContext, tid: u32) -> bool {
    let s2 = sess.clone();
    let r = editsession::run(ctx, tid, move |ec, ctx| {
        open_composition(&s2, ec, ctx)?;
        let (text, comp) = {
            let s = s2.borrow();
            (s.display(), s.comp.clone())
        };
        let Some(comp) = comp else { return Ok(()) };
        let range = unsafe { comp.GetRange()? };
        write_range(ec, ctx, &range, &text)?;
        // Park the popup under the composition now that it has a size.
        let mut s = s2.borrow_mut();
        let rect = caret_rect(ec, ctx, &range);
        let (cands, sel) = (s.cands.clone(), s.sel);
        s.window.show(&cands, sel, rect);
        Ok(())
    });
    match r {
        Ok(()) => true,
        Err(e) => {
            crate::debug(&format!("render failed: {e}"));
            false
        }
    }
}

/// Write `text` as the final contents of the composition and close it.
fn finish(sess: &SharedSession, ctx: &ITfContext, text: &str) {
    let tid = sess.borrow().tid;
    let s2 = sess.clone();
    let text = text.to_string();
    let r = editsession::run(ctx, tid, move |ec, ctx| {
        let comp = s2.borrow_mut().comp.take();
        if let Some(comp) = comp {
            let range = unsafe { comp.GetRange()? };
            write_range(ec, ctx, &range, &text)?;
            unsafe { comp.EndComposition(ec)? };
        }
        s2.borrow_mut().clear();
        Ok(())
    });
    if let Err(e) = r {
        crate::debug(&format!("finish failed: {e}"));
        // The document may be in any state now, but our own state must not be
        // left claiming a composition that is gone.
        sess.borrow_mut().clear();
    }
}

/// Start a composition at the caret if we do not already have one.
fn open_composition(sess: &SharedSession, ec: u32, ctx: &ITfContext) -> Result<()> {
    if sess.borrow().comp.is_some() {
        return Ok(());
    }
    // A query-only insert yields an empty range at the current selection
    // without touching the document — the standard way to find the caret.
    let insert: ITfInsertAtSelection = ctx.cast()?;
    let range = unsafe { insert.InsertTextAtSelection(ec, TF_IAS_QUERYONLY, &[])? };
    let composition: ITfContextComposition = ctx.cast()?;
    let sink: ITfCompositionSink = CompositionSink { sess: Rc::downgrade(sess) }.into();
    let comp = unsafe { composition.StartComposition(ec, &range, &sink)? };
    sess.borrow_mut().comp = Some(comp);
    Ok(())
}

/// Replace a range's text and leave the caret at its end.
fn write_range(ec: u32, ctx: &ITfContext, range: &ITfRange, text: &str) -> Result<()> {
    let w: Vec<u16> = text.encode_utf16().collect();
    unsafe { range.SetText(ec, 0, &w)? };

    let end = unsafe { range.Clone()? };
    unsafe { end.Collapse(ec, TF_ANCHOR_END)? };
    set_selection(ec, ctx, end)
}

/// Put the caret at `at`, consuming the range.
fn set_selection(ec: u32, ctx: &ITfContext, at: ITfRange) -> Result<()> {
    // TF_SELECTION holds the range in a ManuallyDrop, so its reference is ours
    // to release once SetSelection has copied what it needs.
    let mut sel = TF_SELECTION {
        range: ManuallyDrop::new(Some(at)),
        style: TF_SELECTIONSTYLE { ase: TF_AE_END, fInterimChar: BOOL(0) },
    };
    let r = unsafe { ctx.SetSelection(ec, std::slice::from_ref(&sel)) };
    unsafe { ManuallyDrop::drop(&mut sel.range) };
    r
}

/// Screen rectangle of the composition, for positioning the candidate popup.
/// Best-effort: plenty of controls cannot answer, and the popup falls back to
/// the caret position when this is `None`.
fn caret_rect(ec: u32, ctx: &ITfContext, range: &ITfRange) -> Option<windows::Win32::Foundation::RECT> {
    unsafe {
        let view = ctx.GetActiveView().ok()?;
        let mut rc = windows::Win32::Foundation::RECT::default();
        let mut clipped = BOOL(0);
        view.GetTextExt(ec, range, &mut rc, &mut clipped).ok()?;
        (rc.right != 0 || rc.bottom != 0).then_some(rc)
    }
}

// ---------------------------------------------------------------------------
// Composition sink
// ---------------------------------------------------------------------------

/// Notified when something other than us ends the composition. Holds a `Weak`
/// reference: TSF keeps this sink alive for as long as the composition lives,
/// and the session owns the composition, so an `Rc` here would be a cycle.
#[implement(ITfCompositionSink)]
struct CompositionSink {
    sess: Weak<RefCell<Session>>,
}

impl ITfCompositionSink_Impl for CompositionSink_Impl {
    fn OnCompositionTerminated(&self, _ec: u32, _comp: Ref<'_, ITfComposition>) -> Result<()> {
        if let Some(sess) = self.sess.upgrade() {
            crate::debug("composition terminated by the application");
            // An application can terminate the composition from inside an edit
            // we are already running. `borrow_mut` would panic there, and this
            // DLL is built `panic = "abort"` inside somebody else's process, so
            // take the lock only if it is free — the edit in flight is our own
            // code and clears the same state on its way out.
            if let Ok(mut s) = sess.try_borrow_mut() {
                s.clear();
            }
        }
        Ok(())
    }
}
