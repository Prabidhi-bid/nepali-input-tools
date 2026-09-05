//! The text service object itself.
//!
//! TSF creates one of these per thread that uses the input method and calls
//! `Activate` with the thread manager and our client id. Everything the TIP
//! does afterwards hangs off that: the key sink is advised here, the toggle
//! chord is reserved here, and `Deactivate` must undo both — a sink left
//! advised keeps the DLL pinned in the host process.

use std::cell::RefCell;

use windows::core::{implement, Interface, Ref, Result, GUID};
use windows::Win32::UI::Input::KeyboardAndMouse::VK_SPACE;
use windows::Win32::UI::TextServices::{
    ITfKeyEventSink, ITfKeystrokeMgr, ITfTextInputProcessor, ITfTextInputProcessor_Impl,
    ITfThreadMgr, TF_MOD_CONTROL, TF_PRESERVEDKEY,
};

use crate::keysink::KeyEventSink;
use crate::session::{Session, SharedSession};

/// Identity of the toggle chord. Arbitrary, but must stay stable for as long as
/// the key is reserved so `UnpreserveKey` can find it again.
const GUID_TOGGLE: GUID = GUID::from_u128(0x7F1A2C34_5B6D_4E8F_9A0B_1C2D3E4F5061);

/// Ctrl+Space switches between transliterating and typing plain Latin, so a
/// stray English word does not need a full input-method switch.
fn toggle_key() -> TF_PRESERVEDKEY {
    TF_PRESERVEDKEY {
        uVKey: VK_SPACE.0 as u32,
        uModifiers: TF_MOD_CONTROL,
    }
}

/// Live per-thread state, present only between `Activate` and `Deactivate`.
struct Active {
    tid: u32,
    keystroke: ITfKeystrokeMgr,
    sess: SharedSession,
    toggle: TF_PRESERVEDKEY,
}

#[implement(ITfTextInputProcessor)]
pub struct TextService {
    inner: RefCell<Option<Active>>,
}

impl TextService {
    pub fn new() -> Self {
        crate::module_add_ref();
        TextService { inner: RefCell::new(None) }
    }
}

impl Drop for TextService {
    fn drop(&mut self) {
        crate::module_release();
    }
}

impl ITfTextInputProcessor_Impl for TextService_Impl {
    /// Bring the input method up on this thread.
    ///
    /// **This must not fail.** Windows marks a text service whose `Activate`
    /// returns an error as unavailable and draws the blocked sign over it in
    /// the switcher — with no clue as to why. So every step here is
    /// best-effort and logged: a TIP that activates but cannot intercept keys
    /// is a bad day, while a TIP that refuses to activate is an unexplainable
    /// one. The trace build says which happened.
    fn Activate(&self, ptim: Ref<'_, ITfThreadMgr>, tid: u32) -> Result<()> {
        // Windows can activate a thread that was never deactivated; advising a
        // second sink for the same client id would fail, so tear down first.
        self.teardown();

        let Ok(thread_mgr) = ptim.ok() else {
            crate::debug("Activate: no thread manager");
            return Ok(());
        };
        let keystroke: ITfKeystrokeMgr = match thread_mgr.cast() {
            Ok(k) => k,
            Err(e) => {
                crate::debug(&format!("Activate: no keystroke manager: {e}"));
                return Ok(());
            }
        };

        let sess = Session::new(tid);
        let sink: ITfKeyEventSink = KeyEventSink::new(sess.clone()).into();

        // Foreground registration is what gets us keys ahead of the
        // application; if something already holds it, a background advise is
        // still better than none.
        let advised = unsafe { keystroke.AdviseKeyEventSink(tid, &sink, true) }
            .or_else(|e| {
                crate::debug(&format!("foreground key sink refused ({e}); trying background"));
                unsafe { keystroke.AdviseKeyEventSink(tid, &sink, false) }
            });
        if let Err(e) = advised {
            crate::debug(&format!("Activate: key sink refused: {e} - keys will pass through"));
            return Ok(());
        }

        // Another text service may already own this chord, which costs us the
        // toggle but must not stop the input method loading.
        let toggle = toggle_key();
        let desc: Vec<u16> = "Nepali on/off".encode_utf16().collect();
        if let Err(e) = unsafe { keystroke.PreserveKey(tid, &GUID_TOGGLE, &toggle, &desc) } {
            crate::debug(&format!("toggle key unavailable: {e}"));
        }

        // Put the floating bar up now rather than waiting for the first focus
        // change: activation is the moment the user switched *to* this keyboard,
        // and that is when they expect to see it. OnSetFocus keeps it to the
        // focused application from here on.
        //
        // Only on the thread that actually holds the focus, though. Switching
        // input method activates the text service in *every* process that has
        // a text input context, not only the application in front, and when
        // each of them put a bar up the user was left with a column of them -
        // one per running application, scattered rather than stacked because
        // DPI-unaware processes read the same saved position as different
        // pixels. Everybody else waits for `OnSetFocus(true)`; a thread
        // manager that will not answer gets a bar anyway, since no bar at all
        // is the worse failure.
        let focused = unsafe { thread_mgr.IsThreadFocus() }
            .map(|f| f.as_bool())
            .unwrap_or(true);
        if focused {
            crate::bar::show(&std::rc::Rc::downgrade(&sess));
        }

        *self.inner.borrow_mut() = Some(Active { tid, keystroke, sess, toggle });
        crate::debug(&format!("Activate (client id {tid})"));
        Ok(())
    }

    fn Deactivate(&self) -> Result<()> {
        self.teardown();
        crate::debug("Deactivate");
        Ok(())
    }
}

impl TextService_Impl {
    /// Release everything `Activate` took. Safe to call when nothing is active,
    /// and safe to call twice — a sink left advised pins this DLL in the host
    /// process for the rest of its life.
    fn teardown(&self) {
        // Deactivate means the user switched away from this keyboard, so the
        // bar goes with it. First, and without touching the session: the bar
        // belongs to the thread, and a `try_borrow_mut` that failed here would
        // leave it on screen for the rest of the process's life.
        crate::bar::destroy();
        let Some(a) = self.inner.borrow_mut().take() else { return };
        unsafe {
            let _ = a.keystroke.UnpreserveKey(&GUID_TOGGLE, &a.toggle);
            let _ = a.keystroke.UnadviseKeyEventSink(a.tid);
        }
        if let Ok(mut s) = a.sess.try_borrow_mut() {
            s.window.hide();
        };
    }
}
