//! The IBus engine object: one per input context, driven by key events.
//!
//! Mirrors the Windows frontend's editing model exactly, so the two behave the
//! same way:
//!
//!   * the **preedit shows the raw Latin**, underlined, not a converted guess —
//!     conversion happens once, on commit, using whatever the lookup table has
//!     highlighted;
//!   * the candidate list is advisory, picked with the number keys or the arrows;
//!   * only a *deliberate* pick is learned, never the top candidate accepted by
//!     pressing space.
//!
//! IBus is asynchronous and one-directional here: we never ask the application
//! anything, we only emit `UpdatePreeditText`, `UpdateLookupTable` and
//! `CommitText` and let it draw.

use zbus::object_server::SignalContext;
use zbus::{interface, zvariant::OwnedValue};

use crate::engine;
use crate::ibus::{self, key, modifier};

#[derive(Default)]
pub struct XlitEngine {
    /// What the user typed, in Latin.
    buf: String,
    /// Ranked candidates for `buf`, best first.
    cands: Vec<String>,
    /// Index into `cands` that Space / Enter would commit.
    sel: usize,
    /// Whether the user actually picked from the list, rather than accepting
    /// whatever was on top. Only a real choice is worth learning.
    chose: bool,
    /// False after the toggle: keystrokes pass straight through.
    pub enabled: bool,
}

impl XlitEngine {
    pub fn new() -> Self {
        XlitEngine { enabled: true, ..Default::default() }
    }

    fn composing(&self) -> bool {
        !self.buf.is_empty()
    }

    /// What commit would write.
    fn preview(&self) -> String {
        self.cands.get(self.sel).cloned().unwrap_or_else(|| self.buf.clone())
    }

    fn clear(&mut self) {
        self.buf.clear();
        self.cands.clear();
        self.sel = 0;
        self.chose = false;
    }

    fn recompute(&mut self) {
        self.cands = engine::candidates(&self.buf);
        self.sel = 0;
    }

    /// Push the current state to the application: preedit, then lookup table.
    async fn render(&self, ctx: &SignalContext<'_>) -> zbus::Result<()> {
        if !self.composing() {
            Self::hide_preedit_text(ctx).await?;
            Self::hide_lookup_table(ctx).await?;
            return Ok(());
        }
        // The preedit is the raw Latin - see the module comment. Cursor at the
        // end, visible.
        let chars = self.buf.chars().count() as u32;
        Self::update_preedit_text(ctx, ibus::owned(ibus::text(&self.buf)), chars, true).await?;

        if self.cands.is_empty() {
            Self::hide_lookup_table(ctx).await?;
        } else {
            let table = ibus::lookup_table(&self.cands, self.sel as u32, engine::MAX_CANDIDATES as u32);
            Self::update_lookup_table(ctx, ibus::owned(table), true).await?;
        }
        Ok(())
    }

    /// Write `text` into the document and drop all word state.
    async fn finish(&mut self, ctx: &SignalContext<'_>, text: &str) -> zbus::Result<()> {
        if !text.is_empty() {
            Self::commit_text(ctx, ibus::owned(ibus::plain_text(text))).await?;
        }
        self.clear();
        self.render(ctx).await
    }

    /// Finalise the word: write the highlighted candidate plus `tail`, the
    /// break character that triggered the commit, and teach the learning store.
    async fn commit_word(&mut self, ctx: &SignalContext<'_>, tail: &str) -> zbus::Result<()> {
        let (input, chosen, chose) = (self.buf.clone(), self.preview(), self.chose);
        self.finish(ctx, &format!("{chosen}{tail}")).await?;
        // Accepting the top candidate is not a choice, it is just typing.
        // Recording it would let the first answer for an input - right or wrong
        // - outrank the dictionary for ever after.
        if chose && !input.is_empty() && chosen != input {
            engine::commit(&input, &chosen);
        }
        Ok(())
    }
}

#[interface(name = "org.freedesktop.IBus.Engine")]
impl XlitEngine {
    /// The only method that matters. Returns whether we consumed the key; false
    /// hands it back to the application untouched.
    async fn process_key_event(
        &mut self,
        keyval: u32,
        _keycode: u32,
        state: u32,
        #[zbus(signal_context)] ctx: SignalContext<'_>,
    ) -> zbus::fdo::Result<bool> {
        // We act on press. A release still has to be claimed if we claimed its
        // press, or the application sees an unmatched release.
        if state & modifier::RELEASE != 0 {
            return Ok(self.composing());
        }

        // Ctrl+Space toggles passthrough, exactly as on Windows.
        if keyval == key::SPACE && state & modifier::CONTROL != 0 {
            if self.composing() {
                let raw = self.buf.clone();
                self.finish(&ctx, &raw).await?;
            }
            self.enabled = !self.enabled;
            return Ok(true);
        }
        if !self.enabled {
            return Ok(false);
        }
        // Any other modified key is a shortcut, not typing.
        if state & (modifier::CONTROL | modifier::ALT) != 0 {
            return Ok(false);
        }

        match keyval {
            key::BACKSPACE if self.composing() => {
                self.buf.pop();
                if self.buf.is_empty() {
                    self.finish(&ctx, "").await?;
                } else {
                    self.recompute();
                    self.render(&ctx).await?;
                }
                Ok(true)
            }
            key::ESCAPE if self.composing() => {
                // Abandon the conversion, leaving exactly what was typed.
                let raw = self.buf.clone();
                self.finish(&ctx, &raw).await?;
                Ok(true)
            }
            key::SPACE if self.composing() => {
                self.commit_word(&ctx, " ").await?;
                Ok(true)
            }
            key::RETURN | key::KP_ENTER if self.composing() => {
                self.commit_word(&ctx, "").await?;
                Ok(true)
            }
            key::UP | key::PAGE_UP if self.composing() && !self.cands.is_empty() => {
                let n = self.cands.len();
                self.sel = (self.sel + n - 1) % n;
                self.chose = true;
                self.render(&ctx).await?;
                Ok(true)
            }
            key::DOWN | key::PAGE_DOWN if self.composing() && !self.cands.is_empty() => {
                self.sel = (self.sel + 1) % self.cands.len();
                self.chose = true;
                self.render(&ctx).await?;
                Ok(true)
            }
            // 1-9 pick a candidate while composing, and are plain digits
            // otherwise - the number row does double duty, as on Windows.
            key::ONE..=key::NINE if self.composing() => {
                let idx = (keyval - key::ONE) as usize;
                if idx < self.cands.len() {
                    self.sel = idx;
                    self.chose = true;
                    self.commit_word(&ctx, "").await?;
                }
                Ok(true)
            }
            // Latin letters start or extend a word.
            key::LOWER_A..=key::LOWER_Z | key::UPPER_A..=key::UPPER_Z => {
                let Some(c) = char::from_u32(keyval) else { return Ok(false) };
                self.buf.push(c);
                self.recompute();
                self.render(&ctx).await?;
                Ok(true)
            }
            // Anything else ends the word and is then handled by the app.
            _ if self.composing() => {
                self.commit_word(&ctx, "").await?;
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    async fn focus_in(&mut self, #[zbus(signal_context)] ctx: SignalContext<'_>) -> zbus::fdo::Result<()> {
        self.render(&ctx).await?;
        Ok(())
    }

    /// Losing focus commits nothing: the half-typed word would land in whatever
    /// the user clicked on. Drop it and leave the document alone.
    async fn focus_out(&mut self, #[zbus(signal_context)] ctx: SignalContext<'_>) -> zbus::fdo::Result<()> {
        self.clear();
        self.render(&ctx).await?;
        Ok(())
    }

    async fn reset(&mut self, #[zbus(signal_context)] ctx: SignalContext<'_>) -> zbus::fdo::Result<()> {
        self.clear();
        self.render(&ctx).await?;
        Ok(())
    }

    async fn enable(&mut self) -> zbus::fdo::Result<()> {
        self.enabled = true;
        Ok(())
    }

    async fn disable(&mut self, #[zbus(signal_context)] ctx: SignalContext<'_>) -> zbus::fdo::Result<()> {
        self.clear();
        self.render(&ctx).await?;
        Ok(())
    }

    /// Clicking a candidate in the window.
    async fn candidate_clicked(
        &mut self,
        index: u32,
        _button: u32,
        _state: u32,
        #[zbus(signal_context)] ctx: SignalContext<'_>,
    ) -> zbus::fdo::Result<()> {
        let idx = index as usize;
        if idx < self.cands.len() {
            self.sel = idx;
            self.chose = true;
            self.commit_word(&ctx, "").await?;
        }
        Ok(())
    }

    /// IBus calls these on every context; answering them is cheaper than
    /// letting the daemon log an unknown-method error for each keystroke.
    async fn set_capabilities(&mut self, _caps: u32) -> zbus::fdo::Result<()> {
        Ok(())
    }

    async fn set_cursor_location(
        &mut self,
        _x: i32,
        _y: i32,
        _w: i32,
        _h: i32,
    ) -> zbus::fdo::Result<()> {
        Ok(())
    }

    async fn property_activate(&mut self, _name: String, _state: u32) -> zbus::fdo::Result<()> {
        Ok(())
    }

    async fn destroy(&mut self) -> zbus::fdo::Result<()> {
        self.clear();
        Ok(())
    }

    // --- signals: what we tell the application -----------------------------

    #[zbus(signal)]
    async fn commit_text(ctx: &SignalContext<'_>, text: OwnedValue) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn update_preedit_text(
        ctx: &SignalContext<'_>,
        text: OwnedValue,
        cursor_pos: u32,
        visible: bool,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn hide_preedit_text(ctx: &SignalContext<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn update_lookup_table(
        ctx: &SignalContext<'_>,
        table: OwnedValue,
        visible: bool,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn hide_lookup_table(ctx: &SignalContext<'_>) -> zbus::Result<()>;
}
