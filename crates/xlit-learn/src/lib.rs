//! Learning store.
//!
//! Remembers `(latin input → committed text)` and boosts that text on future
//! lookups, scaled by how often and how recently it was picked. Backed by a
//! small pretty-printed JSON file, single writer, atomic replace on save.
//! (If this ever grows large, swap the map for SQLite behind the same API.)
//!
//! Entirely local — nothing here leaves the machine.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use xlit_core::{merge_candidate, Candidate, Ranker, Source};

const DAY_SECS: u64 = 86_400;

#[derive(Clone, Serialize, Deserialize)]
struct Pick {
    input: String,
    chosen: String,
    count: u32,
    last_used: u64,
}

pub struct LearnStore {
    path: Option<PathBuf>,
    picks: Mutex<HashMap<(String, String), Pick>>,
}

impl LearnStore {
    /// Non-persistent store (tests, one-shot use).
    pub fn in_memory() -> Self {
        LearnStore { path: None, picks: Mutex::new(HashMap::new()) }
    }

    /// Open (or start) a JSON-backed store at `path`.
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let mut map = HashMap::new();
        if path.exists() {
            let bytes = std::fs::read(&path)?;
            let list: Vec<Pick> = serde_json::from_slice(&bytes).unwrap_or_default();
            for p in list {
                map.insert((p.input.clone(), p.chosen.clone()), p);
            }
        }
        Ok(LearnStore { path: Some(path), picks: Mutex::new(map) })
    }

    /// Record that `chosen` was committed for `input`, and persist.
    pub fn record(&self, input: &str, chosen: &str) -> io::Result<()> {
        {
            let mut g = self.picks.lock().unwrap();
            let entry = g.entry((input.to_string(), chosen.to_string())).or_insert(Pick {
                input: input.to_string(),
                chosen: chosen.to_string(),
                count: 0,
                last_used: 0,
            });
            entry.count += 1;
            entry.last_used = now();
        }
        self.flush()
    }

    /// Number of distinct (input, chosen) pairs remembered.
    pub fn len(&self) -> usize {
        self.picks.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn flush(&self) -> io::Result<()> {
        let Some(path) = &self.path else { return Ok(()) };
        let json = {
            let g = self.picks.lock().unwrap();
            let list: Vec<&Pick> = g.values().collect();
            serde_json::to_vec_pretty(&list)?
        };
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, &json)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl Ranker for LearnStore {
    fn rank(&self, input: &str, mut cands: Vec<Candidate>) -> Vec<Candidate> {
        let g = self.picks.lock().unwrap();
        let now = now();
        for ((inp, chosen), p) in g.iter() {
            if inp != input {
                continue;
            }
            let recency = if now.saturating_sub(p.last_used) < DAY_SECS { 40 } else { 0 };
            let score = 400 + (p.count.min(50) as i32) * 12 + recency;
            merge_candidate(&mut cands, chosen.clone(), score, Source::Learned);
        }
        cands
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(text: &str) -> Vec<Candidate> {
        vec![Candidate { text: text.to_string(), source: Source::Rule, score: 100 }]
    }

    #[test]
    fn learned_pick_floats_to_top() {
        let store = LearnStore::in_memory();
        store.record("kaam", "काम").unwrap();
        store.record("kaam", "काम").unwrap();

        let ranked = store.rank("kaam", rule("काम्"));
        let top = ranked.iter().max_by_key(|c| c.score).unwrap();
        assert_eq!(top.text, "काम");
        assert_eq!(top.source, Source::Learned);
    }

    #[test]
    fn other_inputs_unaffected() {
        let store = LearnStore::in_memory();
        store.record("kaam", "काम").unwrap();
        let ranked = store.rank("ghar", rule("घर"));
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].source, Source::Rule);
    }

    #[test]
    fn round_trips_through_disk() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("xlit-learn-test-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);

        {
            let s = LearnStore::open(&path).unwrap();
            s.record("nepaal", "नेपाल").unwrap();
        }
        let reopened = LearnStore::open(&path).unwrap();
        assert_eq!(reopened.len(), 1);
        let ranked = reopened.rank("nepaal", rule("नेपाल"));
        assert_eq!(ranked[0].source, Source::Learned);

        let _ = std::fs::remove_file(&path);
    }
}
