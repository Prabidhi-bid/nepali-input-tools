//! Edit sessions.
//!
//! A TIP may not touch the document from a key handler; every read or write
//! goes through `ITfContext::RequestEditSession`, and the edit cookie handed to
//! `DoEditSession` is what authorises the calls. We always ask for a
//! **synchronous** read/write session, so the whole edit happens inside the
//! `RequestEditSession` call and a keystroke is fully applied before we return
//! "eaten" to TSF.
//!
//! [`run`] wraps that in a closure so callers read top-to-bottom instead of
//! defining a COM class per edit.

use std::cell::RefCell;

use windows::core::{implement, Result};
use windows::Win32::UI::TextServices::{
    ITfContext, ITfEditSession, ITfEditSession_Impl, TF_ES_READWRITE, TF_ES_SYNC,
};

type Op = Box<dyn FnOnce(u32, &ITfContext) -> Result<()>>;

#[implement(ITfEditSession)]
struct EditSession {
    ctx: ITfContext,
    /// `FnOnce` behind a `RefCell` because `DoEditSession` only gets `&self`.
    /// Taken on the first (and only) call; a second call is a no-op.
    op: RefCell<Option<Op>>,
}

impl ITfEditSession_Impl for EditSession_Impl {
    fn DoEditSession(&self, ec: u32) -> Result<()> {
        let op = self.op.borrow_mut().take();
        match op {
            Some(op) => op(ec, &self.ctx),
            None => Ok(()),
        }
    }
}

/// Run `f` in a synchronous read/write edit session on `ctx`.
///
/// Two failures are possible and distinct: `RequestEditSession` itself can
/// refuse (the app denied a synchronous session — it returns `TF_E_SYNCHRONOUS`
/// rather than calling us), or the session can run and `f` can fail. Both come
/// back as `Err`; the caller treats either as "this keystroke did nothing".
pub fn run<F>(ctx: &ITfContext, tid: u32, f: F) -> Result<()>
where
    F: FnOnce(u32, &ITfContext) -> Result<()> + 'static,
{
    let session: ITfEditSession = EditSession {
        ctx: ctx.clone(),
        op: RefCell::new(Some(Box::new(f))),
    }
    .into();
    let hr = unsafe { ctx.RequestEditSession(tid, &session, TF_ES_SYNC | TF_ES_READWRITE)? };
    hr.ok()
}
