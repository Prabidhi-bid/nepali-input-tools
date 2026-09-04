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
/// Weight given to a hand-entered word, so it outranks the dictionary from the
/// first use instead of having to be picked repeatedly.
const PINNED_COUNT: u32 = 8;

#[derive(Clone, Serialize, Deserialize)]
struct Pick {
    input: String,
    chosen: String,
    count: u32,
    last_used: u64,
    /// Entered by hand in the word editor, as opposed to picked from the
    /// candidate list while typing.
    #[serde(default)]
    manual: bool,
    /// Kept so files written by older builds still load, and so the field
    /// survives a round trip. Nothing is uploaded.
    #[serde(default)]
    synced: bool,
}

pub struct LearnStore {
    path: Option<PathBuf>,
    picks: Mutex<HashMap<(String, String), Pick>>,
    /// Modification time of the file as we last saw it, so a change made by
    /// another process can be noticed. See [`LearnStore::reload_if_changed`].
    seen: Mutex<Option<SystemTime>>,
}

impl LearnStore {
    /// Non-persistent store (tests, one-shot use).
    pub fn in_memory() -> Self {
        LearnStore {
            path: None,
            picks: Mutex::new(HashMap::new()),
            seen: Mutex::new(None),
        }
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
        let seen = mtime(&path);
        Ok(LearnStore {
            path: Some(path),
            picks: Mutex::new(map),
            seen: Mutex::new(seen),
        })
    }

    /// Re-read the file if another process has written it since we last looked.
    ///
    /// The word editor is a separate process, so without this a word added
    /// there would not take effect until every application holding the input
    /// method was restarted. Costs one stat per call; the read only happens
    /// when the file has actually moved on.
    pub fn reload_if_changed(&self) {
        let Some(path) = &self.path else { return };
        let now = mtime(path);
        {
            let seen = self.seen.lock().unwrap();
            if *seen == now {
                return;
            }
        }
        let Ok(bytes) = std::fs::read(path) else { return };
        let Ok(list) = serde_json::from_slice::<Vec<Pick>>(&bytes) else { return };
        let mut map = HashMap::with_capacity(list.len());
        for p in list {
            map.insert((p.input.clone(), p.chosen.clone()), p);
        }
        *self.picks.lock().unwrap() = map;
        *self.seen.lock().unwrap() = now;
    }

    /// Replace the contents with words from the server.
    ///
    /// For the *shared* store only, which is a local cache of what the server
    /// holds — hence a replace rather than a merge, so words withdrawn upstream
    /// disappear here too. The user's own file is never passed to this.
    pub fn replace_all(&self, words: &[(String, String)]) -> io::Result<()> {
        {
            let mut g = self.picks.lock().unwrap();
            g.clear();
            for (input, chosen) in words {
                g.insert(
                    (input.clone(), chosen.clone()),
                    Pick {
                        input: input.clone(),
                        chosen: chosen.clone(),
                        // Enough to beat the dictionary, but below a word the
                        // user pinned themselves: their own choice wins.
                        count: 1,
                        last_used: now(),
                        manual: false,
                        synced: true,
                    },
                );
            }
        }
        self.flush()
    }

    /// Every remembered pair, most-used first.
    pub fn entries(&self) -> Vec<(String, String, u32)> {
        let g = self.picks.lock().unwrap();
        let mut out: Vec<(String, String, u32)> = g
            .values()
            .map(|p| (p.input.clone(), p.chosen.clone(), p.count))
            .collect();
        out.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
        out
    }

    /// Forget one pair, and persist.
    pub fn remove(&self, input: &str, chosen: &str) -> io::Result<()> {
        {
            let mut g = self.picks.lock().unwrap();
            g.remove(&(input.to_string(), chosen.to_string()));
        }
        self.flush()
    }

    /// Record `chosen` for `input` with enough weight to beat the dictionary
    /// outright — for words the user has entered by hand, which should win
    /// immediately rather than after being picked a few times.
    pub fn pin(&self, input: &str, chosen: &str) -> io::Result<()> {
        {
            let mut g = self.picks.lock().unwrap();
            let entry = g
                .entry((input.to_string(), chosen.to_string()))
                .or_insert(Pick {
                    input: input.to_string(),
                    chosen: chosen.to_string(),
                    count: 0,
                    last_used: 0,
                    manual: true,
                    synced: false,
                });
            entry.count = entry.count.max(PINNED_COUNT);
            entry.last_used = now();
            // Re-saving a word the user had only picked before promotes it to
            // a deliberate entry.
            entry.manual = true;
        }
        self.flush()
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
                manual: false,
                synced: false,
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
        // Remember our own write, so reload_if_changed does not treat it as
        // somebody else's and re-read the file we just produced.
        *self.seen.lock().unwrap() = mtime(path);
        Ok(())
    }
}

fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl Ranker for LearnStore {
    fn rank(&self, input: &str, mut cands: Vec<Candidate>) -> Vec<Candidate> {
        self.reload_if_changed();
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
