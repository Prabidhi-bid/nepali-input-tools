//! The transliteration engine, shared by every IBus engine instance.
//!
//! Deliberately the same shape as the Windows frontend's `engine.rs`: one
//! process-wide [`Engine`] built on first use. The difference is that on Linux
//! this really is one process — an IBus engine is a normal program the daemon
//! starts once — rather than a copy in every application, so there is no reason
//! to be shy about the dictionary's size.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use xlit_core::{Candidate, Engine, Ranker};
use xlit_dict::{DictRanker, WordList};
use xlit_learn::LearnStore;

/// Most candidates we ever hand to the lookup table. IBus numbers a page 1-9.
pub const MAX_CANDIDATES: usize = 9;

struct Shared {
    engine: Engine,
    learn: Arc<LearnStore>,
}

static SHARED: OnceLock<Shared> = OnceLock::new();

/// `$XDG_DATA_HOME/xlit`, falling back to `~/.local/share/xlit`.
fn data_dir() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join("xlit");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn shared() -> &'static Shared {
    SHARED.get_or_init(|| {
        let learn = Arc::new(
            LearnStore::open(data_dir().join("xlit-learn.json")).unwrap_or_else(|e| {
                eprintln!("xlit: learning disabled: {e}");
                LearnStore::in_memory()
            }),
        );
        // Same layering as the Windows frontend, and for the same reason: each
        // ranker only raises scores, so registration order is precedence.
        let engine = Engine::nepali()
            .with_ranker(Box::new(DictRanker::builtin()))
            .with_ranker(Box::new(WordList::new()))
            .with_ranker(Box::new(SharedLearn(learn.clone())));
        Shared { engine, learn }
    })
}

/// Ranked candidate texts for a Latin buffer, best first.
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
    // Always leave a way back to exactly what was typed.
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

/// Remember a deliberate pick. Best-effort; a failed write must not break typing.
pub fn commit(input: &str, chosen: &str) {
    if input.is_empty() || chosen.is_empty() {
        return;
    }
    if let Err(e) = shared().learn.record(input, chosen) {
        eprintln!("xlit: learn write failed: {e}");
    }
}

struct SharedLearn(Arc<LearnStore>);

impl Ranker for SharedLearn {
    fn rank(&self, input: &str, cands: Vec<Candidate>) -> Vec<Candidate> {
        self.0.rank(input, cands)
    }
}
