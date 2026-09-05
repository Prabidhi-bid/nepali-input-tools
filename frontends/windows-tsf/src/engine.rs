//! The transliteration engine, as the TIP sees it.
//!
//! One process-wide [`Engine`] (rule + dictionary + learning), built on first
//! use and shared by every thread the DLL is loaded on. A TSF text service is
//! mapped into *every* text-input process, so this stays deliberately small:
//! the compiled-in seed dictionary only. Once `xlit-daemon` (M3) exists this
//! module becomes an IPC client and the data is loaded once machine-wide
//! instead of once per process.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use xlit_core::{Candidate, Engine, Ranker};
use xlit_dict::{DictRanker, WordList};
use xlit_learn::LearnStore;

/// Most candidates we ever hand to the UI (the candidate window selects with
/// the 1-9 number keys, so there is no point offering more).
pub const MAX_CANDIDATES: usize = 9;

struct Shared {
    engine: Engine,
    learn: Arc<LearnStore>,
}

static SHARED: OnceLock<Shared> = OnceLock::new();

/// `%APPDATA%\xlit\`, falling back to the temp directory if `APPDATA` is unset
/// (service accounts).
fn data_dir() -> PathBuf {
    let dir = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("xlit");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// The user's own picks and hand-added words. Never leaves the machine.
fn learn_path() -> PathBuf {
    data_dir().join("xlit-learn.json")
}

/// Local cache of the shared dictionary, refreshed by the word editor's
/// *Update dictionary* button. Read-only here.
fn shared_path() -> PathBuf {
    data_dir().join("shared.json")
}

fn shared() -> &'static Shared {
    SHARED.get_or_init(|| {
        let learn = Arc::new(LearnStore::open(learn_path()).unwrap_or_else(|e| {
            crate::debug(&format!("learning disabled: {e}"));
            LearnStore::in_memory()
        }));
        // Order sets precedence, because each layer only raises scores:
        // built-in dictionary, then the downloaded one, then the user's own
        // picks last and highest. A word the user added by hand outranks
        // anything the server sent, which outranks the compiled-in seed.
        // DictRanker is keyed on Devanagari and validates what the rule engine
        // produced; WordList is keyed on the Latin actually typed, which is what
        // makes completions possible mid-word. They answer different questions,
        // so both run.
        let mut engine = Engine::nepali()
            .with_ranker(Box::new(DictRanker::builtin()))
            .with_ranker(Box::new(WordList::new()));
        match LearnStore::open(shared_path()) {
            Ok(s) if !s.is_empty() => {
                crate::debug(&format!("shared dictionary: {} words", s.len()));
                engine = engine.with_ranker(Box::new(SharedLearn(Arc::new(s))));
            }
            Ok(_) => {}
            Err(e) => crate::debug(&format!("no shared dictionary: {e}")),
        }
        let engine = engine.with_ranker(Box::new(SharedLearn(learn.clone())));
        crate::debug("engine ready");
        Shared { engine, learn }
    })
}

/// Ranked candidate texts for a Latin buffer, best first, capped at
/// [`MAX_CANDIDATES`]. Empty only when `input` is empty.
pub fn candidates(input: &str) -> Vec<String> {
    if input.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<String> = Vec::with_capacity(MAX_CANDIDATES);
    for c in shared().engine.candidates(input) {
        if c.text.is_empty() || out.iter().any(|t| t == &c.text) {
            continue;
        }
        out.push(c.text);
        if out.len() == MAX_CANDIDATES {
            break;
        }
    }
    // Always leave the user a way back to exactly what they typed.
    if !out.iter().any(|t| t == input) && out.len() < MAX_CANDIDATES {
        out.push(input.to_string());
    }
    out
}

/// The script's own form of a single character — Devanagari digits, mostly.
///
/// Deliberately the bare rule engine rather than [`candidates`]: for a
/// one-character input the dictionary's fuzzy pass can out-score the correct
/// literal with some unrelated short word, so `2` would come back as anything
/// but २.
pub fn literal(c: char) -> String {
    let s = c.to_string();
    let out = shared().engine.transliterate(&s);
    if out.is_empty() {
        s
    } else {
        out
    }
}

/// Remember that `chosen` was committed for `input`, so it ranks first next
/// time. Best-effort: a failed write must never break typing.
pub fn commit(input: &str, chosen: &str) {
    if input.is_empty() || chosen.is_empty() {
        return;
    }
    if let Err(e) = shared().learn.record(input, chosen) {
        crate::debug(&format!("learn write failed: {e}"));
    }
}

/// Lets the learning store be both a `Ranker` inside the engine and a handle we
/// can call `record()` on after a commit.
struct SharedLearn(Arc<LearnStore>);

impl Ranker for SharedLearn {
    fn rank(&self, input: &str, cands: Vec<Candidate>) -> Vec<Candidate> {
        self.0.rank(input, cands)
    }
}
