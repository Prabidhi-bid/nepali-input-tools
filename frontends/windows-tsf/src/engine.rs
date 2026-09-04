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
use xlit_dict::DictRanker;
use xlit_learn::LearnStore;

/// Most candidates we ever hand to the UI (the candidate window selects with
/// the 1-9 number keys, so there is no point offering more).
pub const MAX_CANDIDATES: usize = 9;

struct Shared {
    engine: Engine,
    learn: Arc<LearnStore>,
}

static SHARED: OnceLock<Shared> = OnceLock::new();

/// `%APPDATA%\xlit\xlit-learn.json` — per-user, never leaves the machine.
/// Falls back to the temp directory if `APPDATA` is unset (service accounts).
fn learn_path() -> PathBuf {
    let dir = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("xlit");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("xlit-learn.json")
}

fn shared() -> &'static Shared {
    SHARED.get_or_init(|| {
        let learn = Arc::new(LearnStore::open(learn_path()).unwrap_or_else(|e| {
            crate::debug(&format!("learning disabled: {e}"));
            LearnStore::in_memory()
        }));
        let engine = Engine::nepali()
            .with_ranker(Box::new(DictRanker::builtin()))
            .with_ranker(Box::new(SharedLearn(learn.clone())));
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
