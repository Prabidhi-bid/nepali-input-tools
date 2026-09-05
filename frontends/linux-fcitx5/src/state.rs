//! The editing state machine, with no framework in it at all.
//!
//! Every frontend so far has reimplemented the same small logic — buffer,
//! candidates, selection, what a key does — against a different API, which is
//! why the same bug had to be fixed twice. Here it is written once as a plain
//! type: keys in, an [`Action`] out, no I/O and no framework types. The Fcitx5
//! addon is then a thin translation layer, and this file is testable on any
//! machine, including the Windows one it was written on.
//!
//! Behaviour is deliberately identical to the Windows text service and the IBus
//! engine: the preedit shows the raw Latin, conversion happens once at commit,
//! and only a deliberate pick is learned.

use crate::engine;

/// Key symbols, as X11 keysyms. Fcitx5 and IBus both use these, and for ASCII a
/// keysym is the character's own code point.
pub mod key {
    pub const BACKSPACE: u32 = 0xff08;
    pub const RETURN: u32 = 0xff0d;
    pub const KP_ENTER: u32 = 0xff8d;
    pub const ESCAPE: u32 = 0xff1b;
    pub const SPACE: u32 = 0x020;
    pub const UP: u32 = 0xff52;
    pub const DOWN: u32 = 0xff54;
    pub const PAGE_UP: u32 = 0xff55;
    pub const PAGE_DOWN: u32 = 0xff56;
    pub const ONE: u32 = 0x31;
    pub const NINE: u32 = 0x39;
    pub const LOWER_A: u32 = 0x61;
    pub const LOWER_Z: u32 = 0x7a;
    pub const UPPER_A: u32 = 0x41;
    pub const UPPER_Z: u32 = 0x5a;
}

pub mod modifier {
    pub const CONTROL: u32 = 1 << 2;
    pub const ALT: u32 = 1 << 3;
    pub const RELEASE: u32 = 1 << 30;
}

/// What the frontend should do after a key.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Action {
    /// Whether the key was consumed. False means hand it back to the application.
    pub handled: bool,
    /// Text to commit to the document, if any.
    pub commit: String,
    /// The preedit to display — always the raw Latin, never a conversion.
    pub preedit: String,
    /// Candidates to show, best first. Empty means hide the list.
    pub candidates: Vec<String>,
    /// Which candidate is highlighted.
    pub cursor: usize,
}

#[derive(Default)]
pub struct State {
    buf: String,
    cands: Vec<String>,
    sel: usize,
    /// Whether the user picked from the list rather than accepting the top
    /// candidate. Only a real choice is worth learning.
    chose: bool,
    enabled: bool,
}

impl State {
    pub fn new() -> Self {
        State { enabled: true, ..Default::default() }
    }

    pub fn composing(&self) -> bool {
        !self.buf.is_empty()
    }

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

    /// The current display state, with nothing committed.
    fn showing(&self, handled: bool) -> Action {
        Action {
            handled,
            commit: String::new(),
            preedit: self.buf.clone(),
            candidates: self.cands.clone(),
            cursor: self.sel,
        }
    }

    /// Commit the highlighted candidate plus `tail`, learn if it was a choice,
    /// and reset.
    fn commit_word(&mut self, tail: &str, handled: bool) -> Action {
        let (input, chosen, chose) = (self.buf.clone(), self.preview(), self.chose);
        self.clear();
        // Accepting the top candidate by pressing space is not a choice, it is
        // just typing. Recording it would let the first answer for an input —
        // right or wrong — outrank the dictionary for ever after.
        if chose && !input.is_empty() && chosen != input {
            engine::commit(&input, &chosen);
        }
        Action { handled, commit: format!("{chosen}{tail}"), ..Default::default() }
    }

    /// Abandon the conversion, leaving exactly what was typed.
    fn cancel(&mut self) -> Action {
        let raw = self.buf.clone();
        self.clear();
        Action { handled: true, commit: raw, ..Default::default() }
    }

    /// Focus loss and reset: drop the word without committing it. Committing
    /// here would drop a half-typed word into whatever was clicked on.
    pub fn reset(&mut self) -> Action {
        self.clear();
        self.showing(false)
    }

    pub fn key(&mut self, keysym: u32, state: u32) -> Action {
        // Act on press; still claim the release of a key whose press we claimed.
        if state & modifier::RELEASE != 0 {
            return Action { handled: self.composing(), ..Default::default() };
        }

        // Ctrl+Space toggles passthrough.
        if keysym == key::SPACE && state & modifier::CONTROL != 0 {
            let out = if self.composing() { self.cancel() } else { Action::default() };
            self.enabled = !self.enabled;
            return Action { handled: true, ..out };
        }
        if !self.enabled {
            return Action::default();
        }
        // Any other modified key is a shortcut, not typing.
        if state & (modifier::CONTROL | modifier::ALT) != 0 {
            return Action::default();
        }

        match keysym {
            key::BACKSPACE if self.composing() => {
                self.buf.pop();
                if self.buf.is_empty() {
                    self.clear();
                    Action { handled: true, ..Default::default() }
                } else {
                    self.recompute();
                    self.showing(true)
                }
            }
            key::ESCAPE if self.composing() => self.cancel(),
            key::SPACE if self.composing() => self.commit_word(" ", true),
            key::RETURN | key::KP_ENTER if self.composing() => self.commit_word("", true),
            key::UP | key::PAGE_UP if self.composing() && !self.cands.is_empty() => {
                let n = self.cands.len();
                self.sel = (self.sel + n - 1) % n;
                self.chose = true;
                self.showing(true)
            }
            key::DOWN | key::PAGE_DOWN if self.composing() && !self.cands.is_empty() => {
                self.sel = (self.sel + 1) % self.cands.len();
                self.chose = true;
                self.showing(true)
            }
            // 1-9 pick a candidate while composing, and are plain digits
            // otherwise: the number row does double duty.
            key::ONE..=key::NINE if self.composing() => {
                let idx = (keysym - key::ONE) as usize;
                if idx < self.cands.len() {
                    self.sel = idx;
                    self.chose = true;
                    self.commit_word("", true)
                } else {
                    self.showing(true)
                }
            }
            key::LOWER_A..=key::LOWER_Z | key::UPPER_A..=key::UPPER_Z => {
                let Some(c) = char::from_u32(keysym) else { return Action::default() };
                self.buf.push(c);
                self.recompute();
                self.showing(true)
            }
            // Anything else ends the word, then the application handles the key
            // itself — which is what `handled: false` beside a commit means.
            _ if self.composing() => self.commit_word("", false),
            _ => Action::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_word(s: &mut State, word: &str) -> Action {
        let mut last = Action::default();
        for c in word.chars() {
            last = s.key(c as u32, 0);
        }
        last
    }

    #[test]
    fn preedit_is_the_raw_latin() {
        let mut s = State::new();
        let a = type_word(&mut s, "namaste");
        assert_eq!(a.preedit, "namaste", "the preedit must never be a conversion");
        assert!(a.handled);
        assert!(a.commit.is_empty());
    }

    #[test]
    fn space_commits_the_conversion_and_a_space() {
        let mut s = State::new();
        type_word(&mut s, "ghar");
        let a = s.key(key::SPACE, 0);
        assert_eq!(a.commit, "घर ");
        assert!(!s.composing());
    }

    #[test]
    fn enter_commits_without_a_space() {
        let mut s = State::new();
        type_word(&mut s, "ghar");
        assert_eq!(s.key(key::RETURN, 0).commit, "घर");
    }

    #[test]
    fn escape_gives_back_exactly_what_was_typed() {
        let mut s = State::new();
        type_word(&mut s, "ghar");
        assert_eq!(s.key(key::ESCAPE, 0).commit, "ghar");
    }

    #[test]
    fn backspace_shortens_and_then_empties() {
        let mut s = State::new();
        type_word(&mut s, "gh");
        assert_eq!(s.key(key::BACKSPACE, 0).preedit, "g");
        let a = s.key(key::BACKSPACE, 0);
        assert!(a.handled && a.preedit.is_empty());
        assert!(!s.composing());
    }

    #[test]
    fn number_keys_pick_a_candidate() {
        let mut s = State::new();
        let a = type_word(&mut s, "ne");
        assert!(a.candidates.len() > 1, "expected a list, got {:?}", a.candidates);
        let second = a.candidates[1].clone();
        assert_eq!(s.key(key::ONE + 1, 0).commit, second);
    }

    #[test]
    fn a_digit_is_a_digit_when_not_composing() {
        let mut s = State::new();
        assert!(!s.key(key::ONE, 0).handled);
    }

    #[test]
    fn arrows_move_the_highlight() {
        let mut s = State::new();
        type_word(&mut s, "ne");
        let a = s.key(key::DOWN, 0);
        assert_eq!(a.cursor, 1);
        assert_eq!(s.key(key::UP, 0).cursor, 0);
    }

    #[test]
    fn ctrl_space_toggles_passthrough() {
        let mut s = State::new();
        assert!(s.key(key::SPACE, modifier::CONTROL).handled);
        // Disabled: letters go straight through.
        assert!(!s.key('a' as u32, 0).handled);
        s.key(key::SPACE, modifier::CONTROL);
        assert!(s.key('a' as u32, 0).handled);
    }

    #[test]
    fn losing_focus_drops_the_word_rather_than_committing_it() {
        let mut s = State::new();
        type_word(&mut s, "ghar");
        assert!(s.reset().commit.is_empty());
        assert!(!s.composing());
    }

    #[test]
    fn punctuation_commits_the_word_and_is_left_to_the_application() {
        let mut s = State::new();
        type_word(&mut s, "ghar");
        let a = s.key('.' as u32, 0);
        assert_eq!(a.commit, "घर");
        assert!(!a.handled, "the application types the full stop itself");
    }
}
