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
    fn Activate(&self, ptim: Ref<'_, ITfThreadMgr>, tid: u32) -> Result<()> {
        let thread_mgr = ptim.ok()?;
        let sess = Session::new(tid);
        let sink: ITfKeyEventSink = KeyEventSink::new(sess.clone()).into();
        let keystroke: ITfKeystrokeMgr = thread_mgr.cast()?;

        unsafe { keystroke.AdviseKeyEventSink(tid, &sink, true)? };

        // Best-effort: another text service may already own this chord, which
        // costs us the toggle but must not stop the input method loading.
        let toggle = toggle_key();
        let desc: Vec<u16> = "Nepali on/off".encode_utf16().collect();
        if let Err(e) = unsafe { keystroke.PreserveKey(tid, &GUID_TOGGLE, &toggle, &desc) } {
            crate::debug(&format!("toggle key unavailable: {e}"));
        }

        *self.inner.borrow_mut() = Some(Active { tid, keystroke, sess, toggle });
        crate::debug(&format!("Activate (client id {tid})"));
        Ok(())
    }

    fn Deactivate(&self) -> Result<()> {
        if let Some(a) = self.inner.borrow_mut().take() {
            unsafe {
                let _ = a.keystroke.UnpreserveKey(&GUID_TOGGLE, &a.toggle);
                let _ = a.keystroke.UnadviseKeyEventSink(a.tid);
            }
            a.sess.borrow_mut().window.hide();
        }
        crate::debug("Deactivate");
        Ok(())
    }
}
