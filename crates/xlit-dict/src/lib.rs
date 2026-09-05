//! Dictionary layer.
//!
//! A finite-state transducer (BurntSushi `fst`) mapping **Devanagari word →
//! frequency**. Three cheap lookups per input, all against the same structure:
//!
//! 1. **exact**  — the rule output is a real word: lock it in.
//! 2. **fuzzy**  — edit-distance ≤ 1, and *only* across characters the Latin
//!    spelling cannot settle: vowel length (`ि`/`ी`), sibilants, b/v, nasal
//!    marks. Anything else is a different word, not a correction.
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
//!
//! Restricting *which* substitutions count is what keeps this layer honest. An
//! unrestricted distance of 1 puts every short word within reach of every
//! other: काम is one edit from both साम and नाम, and outranked them both
//! because a dictionary hit scores above a literal transliteration.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use fst::automaton::Str;
use fst::{Automaton, IntoStreamer, Map, MapBuilder, Streamer};
use memmap2::Mmap;

use xlit_core::{merge_candidate, Candidate, Ranker, Source};

const SEED_TSV: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/seed/ne.tsv"));

/// Nepali postpositions and case markers, most frequent first.
///
/// Nepali attaches these productively, so a dictionary can never list every
/// inflected form - काम is in the seed, कामको and कामले never will be. Building
/// them from a word we know is real gives the candidate list the shape a
/// Nepali typist expects, at no cost in dictionary size.
const SUFFIXES: &[&str] = &[
    "\u{0915}\u{094B}",                     // को  genitive
    "\u{0932}\u{0947}",                     // ले  ergative / instrumental
    "\u{092E}\u{093E}",                     // मा  locative
    "\u{0932}\u{093E}\u{0908}",             // लाई dative / accusative
    "\u{0939}\u{0930}\u{0942}",             // हरू plural
    "\u{092C}\u{093E}\u{091F}",             // बाट ablative
];

#[derive(Clone, Copy)]
pub struct DictOpts {
    /// Max edit distance for the fuzzy pass (1 is plenty for typo correction).
    pub max_edit_distance: u32,
    /// Cap on fuzzy candidates added per input.
    pub max_fuzzy: usize,
    /// Cap on prefix completions added per input.
    pub max_completions: usize,
    /// Cap on postposition forms built from an exact dictionary hit.
    pub max_suffixes: usize,
}

impl Default for DictOpts {
    fn default() -> Self {
        DictOpts { max_edit_distance: 1, max_fuzzy: 3, max_completions: 3, max_suffixes: 4 }
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

/// Characters a reader would accept as the same word.
///
/// This is the whole difference between a spelling correction and a wrong
/// answer. The rule engine is never wrong about the *sounds* the user typed, so
/// a dictionary word may only outrank its literal output when the difference is
/// something the Latin spelling genuinely cannot settle: vowel length, which
/// sibilant, b versus v, and which nasal mark. A substitution outside these
/// classes means a different word, not a typo - `saam` is साम, and काम is not a
/// correction of it.
fn confusable(a: char, b: char) -> bool {
    /// Each string is one class of mutually interchangeable characters.
    const CLASSES: &[&str] = &[
        "\u{093F}\u{0940}", // ि ी  short/long i matra
        "\u{0941}\u{0942}", // ु ू  short/long u matra
        "\u{0947}\u{0948}", // े ै
        "\u{094B}\u{094C}", // ो ौ
        "\u{0907}\u{0908}", // इ ई
        "\u{0909}\u{090A}", // उ ऊ
        "\u{090F}\u{0910}", // ए ऐ
        "\u{0913}\u{0914}", // ओ औ
        "\u{0905}\u{0906}", // अ आ
        "\u{0938}\u{0936}\u{0937}", // स श ष
        "\u{092C}\u{0935}", // ब व
        // Dental vs retroflex. Romanised Nepali writes both with the same Latin
        // letter unless the typist shifts (t/T), and English loanwords take the
        // retroflex, so `kriket` for क्रिकेट is the norm rather than a mistake.
        "\u{0924}\u{091F}", // त ट
        "\u{0925}\u{0920}", // थ ठ
        "\u{0926}\u{0921}", // द ड
        "\u{0927}\u{0922}", // ध ढ
        "\u{0928}\u{0923}", // न ण
        "\u{0901}\u{0902}", // ँ ं
    ];
    a == b || CLASSES.iter().any(|c| c.contains(a) && c.contains(b))
}

/// Marks whose presence or absence is a plausible slip.
///
/// Only the nasal marks and visarga (U+0901..U+0903), which a typist genuinely
/// omits. Matras and virama are deliberately *not* here: dropping one makes a
/// different word, and allowing it offered नेपाल as a correction of नेपालि.
fn is_weak_mark(c: char) -> bool {
    matches!(c, '\u{0901}'..='\u{0903}')
}

/// Char-level edit distance with an early cutoff, counting only the edits above
/// as costing one; anything else is priced beyond `max` so the pair is rejected
/// rather than merely ranked lower. Returns the distance if it is `<= max`.
/// Two rolling rows; bails once a whole row exceeds `max`.
fn bounded_levenshtein(a: &[char], b: &[char], max: usize) -> Option<usize> {
    let (n, m) = (a.len(), b.len());
    if n.abs_diff(m) > max {
        return None;
    }
    // One past the budget: arithmetic stays saturating, so this is "impossible"
    // without needing a separate sentinel.
    let over = max + 1;
    let gap = |c: char| if is_weak_mark(c) { 1 } else { over };
    let swap = |x: char, y: char| {
        if x == y {
            0
        } else if confusable(x, y) {
            1
        } else {
            over
        }
    };

    let mut prev: Vec<usize> = Vec::with_capacity(m + 1);
    prev.push(0);
    for j in 1..=m {
        let v = prev[j - 1].saturating_add(gap(b[j - 1])).min(over);
        prev.push(v);
    }
    let mut curr: Vec<usize> = vec![0; m + 1];
    for i in 1..=n {
        curr[0] = prev[0].saturating_add(gap(a[i - 1])).min(over);
        let mut row_min = curr[0];
        for j in 1..=m {
            curr[j] = prev[j]
                .saturating_add(gap(a[i - 1]))
                .min(curr[j - 1].saturating_add(gap(b[j - 1])))
                .min(prev[j - 1].saturating_add(swap(a[i - 1], b[j - 1])))
                .min(over);
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
        // Validate every literal form the rule engine offered — the primary
        // transliteration *and* any orthographic variant (e.g. the de-geminated
        // spelling of a loanword). A variant only rises if the dictionary
        // vouches for it.
        let mut words: Vec<String> = cands
            .iter()
            .filter(|c| c.source == Source::Rule && !c.text.is_empty())
            .map(|c| c.text.clone())
            .collect();
        words.dedup();
        if words.is_empty() {
            return cands;
        }

        for word in words {
            let word_len = word.chars().count();
            // 1. exact match
            let mut is_a_word = false;
            if let Some(freq) = self.map.get(word.as_bytes()) {
                is_a_word = true;
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

            // 3. inflected forms, but only of a word the dictionary vouches for
            // — suffixing a guess would just multiply the guess. Scored below
            // the exact hit and in postposition-frequency order, so `kaam`
            // reads काम, कामको, कामले, ...
            if is_a_word {
                for (i, suffix) in SUFFIXES.iter().take(self.opts.max_suffixes).enumerate() {
                    merge_candidate(
                        &mut cands,
                        format!("{word}{suffix}"),
                        150 - i as i32,
                        Source::Dictionary,
                    );
                }
            }

            // 4. prefix completions: real longer words that start with this one.
            if word_len >= 2 {
                let pfx = Str::new(word.as_str()).starts_with();
                for (s, v) in self.collect(pfx, &word, self.opts.max_completions) {
                    merge_candidate(&mut cands, s, 90 + freq_bonus(v) / 2, Source::Dictionary);
                }
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
    fn literal_wins_over_an_unrelated_word() {
        // Regression. With an unrestricted edit distance of 1, काम sat one
        // substitution from both साम and नाम, and a dictionary hit outscores a
        // literal transliteration - so typing either produced काम, with the
        // right answer third. A different consonant is a different word.
        for (input, expect) in [("saam", "साम"), ("naam", "नाम")] {
            let c = engine().candidates(input);
            assert_eq!(c[0].text, expect, "{input} gave {:?}", c[0].text);
            assert!(
                !c.iter().any(|x| x.text == "काम"),
                "{input} should not offer काम at all, got {:?}",
                c.iter().map(|x| &x.text).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn dental_retroflex_is_still_corrected() {
        // The other half of the trade: t/T is a real ambiguity in romanised
        // Nepali, and English loanwords take the retroflex.
        assert_eq!(engine().candidates("kriket")[0].text, "क्रिकेट");
    }

    #[test]
    fn only_confusable_substitutions_count() {
        assert!(confusable('\u{093F}', '\u{0940}')); // ि ी  vowel length
        assert!(confusable('\u{0938}', '\u{0936}')); // स श  sibilants
        assert!(confusable('\u{0924}', '\u{091F}')); // त ट  dental/retroflex
        assert!(!confusable('\u{0938}', '\u{0915}')); // स क  different words
        assert!(!confusable('\u{0928}', '\u{0915}')); // न क
    }

    #[test]
    fn known_word_offers_its_postpositions() {
        let texts: Vec<String> = engine()
            .candidates("kaam")
            .iter()
            .map(|c| c.text.clone())
            .collect();
        assert_eq!(texts[0], "काम", "the bare word comes first");
        for want in ["कामको", "कामले"] {
            assert!(texts.iter().any(|t| t == want), "missing {want} in {texts:?}");
        }
    }

    #[test]
    fn postpositions_are_not_built_from_guesses() {
        // साम is not in the dictionary, so there is nothing to inflect; making
        // सामको out of an unverified word would just multiply the guess.
        let texts: Vec<String> = engine()
            .candidates("saam")
            .iter()
            .map(|c| c.text.clone())
            .collect();
        assert!(
            !texts.iter().any(|t| t.starts_with("सामक")),
            "should not inflect an unknown word, got {texts:?}"
        );
    }

    #[test]
    fn unknown_word_untouched() {
        // a nonsense string the dict has never seen: rule output still wins
        let c = engine().candidates("xyzqwq");
        assert_eq!(c[0].source, Source::Rule);
    }

    #[test]
    fn loanword_degeminated_variant_wins() {
        // rule engine: "hello" -> हेल्लो (literal) + हेलो (de-geminated variant);
        // हेलो is a seed loanword, so the dictionary promotes it to the top.
        let c = engine().candidates("hello");
        assert_eq!(c[0].text, "हेलो");
        assert_eq!(c[0].source, Source::Dictionary);
    }

    #[test]
    fn different_consonant_conjunct_survives() {
        // क्र is two distinct consonants — the de-gemination pass must not touch
        // it, and the exact seed spelling क्रिकेट should come back on top.
        let c = engine().candidates("kriket");
        assert_eq!(c[0].text, "क्रिकेट");
        assert_eq!(c[0].source, Source::Dictionary);
    }

    #[test]
    fn degeminated_variant_stays_down_when_not_a_word() {
        // "briffo": neither the literal ब्रिफ्फो nor its de-geminated form ब्रिफो
        // is a dictionary word, so the literal transliteration stays on top.
        let c = engine().candidates("briffo");
        assert_eq!(c[0].text, "ब्रिफ्फो");
        assert_eq!(c[0].source, Source::Rule);
    }

    #[test]
    fn hindi_style_name_normalizes_to_nepali() {
        // "raviiMdranaath" -> रवींद्रनाथ literally; the nasal-conjunct variant
        // रवीन्द्रनाथ is a seed proper noun, so it wins.
        let c = engine().candidates("raviiMdranaath");
        assert_eq!(c[0].text, "रवीन्द्रनाथ");
        assert_eq!(c[0].source, Source::Dictionary);
    }

    #[test]
    fn anusvara_loanword_normalizes_to_nepali_spelling() {
        // "aMgrejii" -> अंग्रेजी literally; Nepali अङ्ग्रेजी (ङ् conjunct) is the
        // seed spelling and should come out on top.
        let c = engine().candidates("aMgrejii");
        assert_eq!(c[0].text, "अङ्ग्रेजी");
        assert_eq!(c[0].source, Source::Dictionary);
    }

    #[test]
    fn anusvara_before_sibilant_kept() {
        // संसार keeps its anusvara — there is no स-homorganic nasal.
        let c = engine().candidates("saMsaar");
        assert_eq!(c[0].text, "संसार");
    }
}
