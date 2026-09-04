//! Dictionary layer.
//!
//! A finite-state transducer (BurntSushi `fst`) mapping **Devanagari word →
//! frequency**. Three cheap lookups per input, all against the same structure:
//!
//! 1. **exact**  — the rule output is a real word: lock it in.
//! 2. **fuzzy**  — edit-distance ≤ 1: fixes vowel-length (`ि`/`ी`), anusvara,
//!    and sibilant slips the rule engine can't know about.
//! 3. **prefix** — longer words starting with what was typed: completions.
//!
//! The FST is memory-mapped (`DictRanker::open`) so a large dictionary costs
//! almost no resident RAM; the compiled-in seed list (`DictRanker::builtin`)
//! is built in memory for out-of-the-box use and tests.
//!
//! The fuzzy pass is a bounded char-level edit-distance scan over the keys, not
//! `fst`'s `Levenshtein` automaton — that one is byte-oriented and silently
//! matches nothing for multi-byte UTF-8 (Devanagari). It's O(dict) per miss;
//! a BK-tree / SymSpell index is the upgrade if profiling ever asks for it.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use fst::automaton::Str;
use fst::{Automaton, IntoStreamer, Map, MapBuilder, Streamer};
use memmap2::Mmap;

use xlit_core::{merge_candidate, Candidate, Ranker, Source};

const SEED_TSV: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/seed/ne.tsv"));

#[derive(Clone, Copy)]
pub struct DictOpts {
    /// Max edit distance for the fuzzy pass (1 is plenty for typo correction).
    pub max_edit_distance: u32,
    /// Cap on fuzzy candidates added per input.
    pub max_fuzzy: usize,
    /// Cap on prefix completions added per input.
    pub max_completions: usize,
}

impl Default for DictOpts {
    fn default() -> Self {
        DictOpts { max_edit_distance: 1, max_fuzzy: 5, max_completions: 3 }
    }
}

pub struct DictRanker<D> {
    map: Map<D>,
    opts: DictOpts,
}

impl DictRanker<Vec<u8>> {
    /// Build from the compiled-in Nepali seed list (small, in-memory).
    pub fn builtin() -> Self {
        Self::from_tsv(SEED_TSV).expect("seed tsv must be valid")
    }

    /// Build an in-memory FST from `word<TAB>freq` lines (`#` comments allowed).
    pub fn from_tsv(text: &str) -> io::Result<Self> {
        let mut sorted: BTreeMap<String, u64> = BTreeMap::new();
        for (n, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut it = line.split('\t');
            let word = it.next().unwrap_or("").trim();
            let freq: u64 = it.next().unwrap_or("1").trim().parse().map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("bad frequency on line {}", n + 1),
                )
            })?;
            if word.is_empty() {
                continue;
            }
            let slot = sorted.entry(word.to_string()).or_insert(0);
            *slot = (*slot).max(freq);
        }

        let mut b = MapBuilder::new(Vec::new()).map_err(to_io)?;
        for (w, f) in &sorted {
            b.insert(w, *f).map_err(to_io)?;
        }
        let bytes = b.into_inner().map_err(to_io)?;
        Ok(DictRanker { map: Map::new(bytes).map_err(to_io)?, opts: DictOpts::default() })
    }
}

impl DictRanker<Mmap> {
    /// Memory-map a prebuilt `.fst` (see the `build` binary).
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = std::fs::File::open(path)?;
        // SAFETY: we treat the mapping as immutable read-only data for the life
        // of the `Map`. The caller must not truncate/rewrite the file meanwhile.
        let mmap = unsafe { Mmap::map(&file)? };
        Ok(DictRanker { map: Map::new(mmap).map_err(to_io)?, opts: DictOpts::default() })
    }
}

impl<D: AsRef<[u8]>> DictRanker<D> {
    pub fn with_opts(mut self, opts: DictOpts) -> Self {
        self.opts = opts;
        self
    }

    /// Number of words in the dictionary.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.len() == 0
    }

    fn collect<A: Automaton>(&self, aut: A, skip: &str, limit: usize) -> Vec<(String, u64)> {
        let mut hits: Vec<(String, u64)> = Vec::new();
        let mut stream = self.map.search(aut).into_stream();
        while let Some((k, v)) = stream.next() {
            if let Ok(s) = std::str::from_utf8(k) {
                if s != skip {
                    hits.push((s.to_string(), v));
                }
            }
        }
        hits.sort_by(|a, b| b.1.cmp(&a.1));
        hits.truncate(limit);
        hits
    }
}

/// Frequency → small additive bonus (log-scaled, capped).
fn freq_bonus(freq: u64) -> i32 {
    (((freq.max(1) as f64).ln() * 5.0) as i32).clamp(0, 80)
}

/// Char-level Levenshtein with an early cutoff. Returns the distance if it is
/// `<= max`, otherwise `None`. Two rolling rows; bails once a whole row exceeds
/// `max`.
fn bounded_levenshtein(a: &[char], b: &[char], max: usize) -> Option<usize> {
    let (n, m) = (a.len(), b.len());
    if n.abs_diff(m) > max {
        return None;
    }
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut curr: Vec<usize> = vec![0; m + 1];
    for i in 1..=n {
        curr[0] = i;
        let mut row_min = curr[0];
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
            row_min = row_min.min(curr[j]);
        }
        if row_min > max {
            return None;
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    (prev[m] <= max).then_some(prev[m])
}

impl<D: AsRef<[u8]> + Send + Sync> Ranker for DictRanker<D> {
    fn rank(&self, _input: &str, mut cands: Vec<Candidate>) -> Vec<Candidate> {
        let word = match cands.iter().find(|c| c.source == Source::Rule) {
            Some(c) => c.text.clone(),
            None => return cands,
        };
        if word.is_empty() {
            return cands;
        }
        let word_len = word.chars().count();

        // 1. exact match
        if let Some(freq) = self.map.get(word.as_bytes()) {
            merge_candidate(
                &mut cands,
                word.clone(),
                300 + freq_bonus(freq),
                Source::Dictionary,
            );
        }

        // 2. fuzzy match (typo correction) — bounded edit-distance scan.
        {
            let q: Vec<char> = word.chars().collect();
            let maxd = self.opts.max_edit_distance as usize;
            let mut hits: Vec<(String, u64)> = Vec::new();
            let mut stream = self.map.stream();
            while let Some((k, v)) = stream.next() {
                let Ok(s) = std::str::from_utf8(k) else { continue };
                if s == word {
                    continue;
                }
                let sc: Vec<char> = s.chars().collect();
                if sc.len().abs_diff(q.len()) > maxd {
                    continue;
                }
                if bounded_levenshtein(&q, &sc, maxd).is_some() {
                    hits.push((s.to_string(), v));
                }
            }
            hits.sort_by(|a, b| b.1.cmp(&a.1));
            for (s, v) in hits.into_iter().take(self.opts.max_fuzzy) {
                // same-length hits are substitutions (vowel length, nasal,
                // sibilant) — the common typo class; nudge them above
                // insertions/deletions of similar frequency.
                let same_len = s.chars().count() == word_len;
                let score = 200 + freq_bonus(v) + if same_len { 15 } else { 0 };
                merge_candidate(&mut cands, s, score, Source::Dictionary);
            }
        }

        // 3. prefix completions
        if word_len >= 2 {
            let pfx = Str::new(word.as_str()).starts_with();
            for (s, v) in self.collect(pfx, &word, self.opts.max_completions) {
                merge_candidate(&mut cands, s, 90 + freq_bonus(v) / 2, Source::Dictionary);
            }
        }

        cands
    }
}

fn to_io<E: std::fmt::Display>(e: E) -> io::Error {
    io::Error::new(io::ErrorKind::Other, e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use xlit_core::Engine;

    fn engine() -> Engine {
        Engine::nepali().with_ranker(Box::new(DictRanker::builtin()))
    }

    #[test]
    fn seed_loads() {
        assert!(DictRanker::builtin().len() > 50);
    }

    #[test]
    fn exact_word_is_validated() {
        let top = &engine().candidates("nepaal")[0];
        assert_eq!(top.text, "नेपाल");
        assert_eq!(top.source, Source::Dictionary);
    }

    #[test]
    fn fuzzy_fixes_vowel_length() {
        // rule engine gives नेपालि (literal short i); dict corrects to नेपाली
        assert_eq!(engine().candidates("nepaali")[0].text, "नेपाली");
    }

    #[test]
    fn fuzzy_fixes_consonant_slip() {
        // rule gives धन्यबाद; correct spelling is धन्यवाद (ब -> व)
        let texts: Vec<_> = engine()
            .candidates("dhanyabaad")
            .iter()
            .map(|c| c.text.clone())
            .collect();
        assert!(texts.contains(&"धन्यवाद".to_string()), "got {texts:?}");
    }

    #[test]
    fn prefix_completion_offered() {
        let texts: Vec<_> = engine()
            .candidates("nepaal")
            .iter()
            .map(|c| c.text.clone())
            .collect();
        assert!(
            texts.iter().any(|t| t.starts_with("नेपाल") && t != "नेपाल"),
            "got {texts:?}"
        );
    }

    #[test]
    fn unknown_word_untouched() {
        // a nonsense string the dict has never seen: rule output still wins
        let c = engine().candidates("xyzqwq");
        assert_eq!(c[0].source, Source::Rule);
    }
}
