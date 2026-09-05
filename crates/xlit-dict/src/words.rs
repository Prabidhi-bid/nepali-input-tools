//! Latin-keyed word list.
//!
//! The [`DictRanker`](crate::DictRanker) beside this one is keyed on
//! *Devanagari*: it takes what the rule engine produced and asks whether that is
//! a word. This layer works the other way round — it looks up the Latin the user
//! actually typed. That is what makes completions possible while a word is still
//! half-typed, when the rule engine's output (`घर` for `ghar`) is a prefix of
//! nothing in particular.
//!
//! The structure is an `fst::Set` of `"<latin>\t<devanagari>"` entries, compiled
//! at build time from `data/ne-words.tsv` (see `build.rs`) and included in the
//! binary. A prefix scan for `"ghar"` walks every word whose key starts with it;
//! the `\t` makes the exact-key subset trivially separable.

use fst::automaton::Str;
use fst::{Automaton, IntoStreamer, Set, Streamer};

use xlit_core::{merge_candidate, Candidate, Ranker, Source};

/// The compiled set, built by `build.rs`.
static WORDS_FST: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/ne-words.fst"));

/// Cap on completions offered for one input. The candidate window shows nine.
const MAX_COMPLETIONS: usize = 12;

/// Postpositions, as (what is typed, what is written).
///
/// Several Latin spellings per postposition on purpose: the keys in the word
/// list are folded to lowercase with short vowels, and a typist writes `laai`
/// or `lai`, `haru` or `haruu`. Longest first, so `laai` is tried before `lai`
/// and `ai` is never mistaken for the whole ending.
const POSTPOSITIONS: &[(&str, &str)] = &[
    ("haruu", "हरू"),
    ("haru", "हरू"),
    ("laai", "लाई"),
    ("lai", "लाई"),
    ("baaTa", "बाट"),
    ("baata", "बाट"),
    ("bata", "बाट"),
    ("ko", "को"),
    ("kaa", "का"),
    ("kii", "की"),
    ("ki", "की"),
    ("le", "ले"),
    ("maa", "मा"),
    ("ma", "मा"),
];

/// Postpositions attach to nominals, not to conjugated verbs — कामको is a word,
/// भयोको is not. Mirrors the same guard in the Devanagari-keyed layer.
fn takes_postposition(word: &str) -> bool {
    const VERBAL_ENDINGS: &[&str] = &["नु", "यो", "ेको", "छ", "दै", "दा", "ने"];
    !VERBAL_ENDINGS.iter().any(|e| word.ends_with(e))
}

pub struct WordList {
    set: Set<&'static [u8]>,
}

impl Default for WordList {
    fn default() -> Self {
        Self::new()
    }
}

impl WordList {
    pub fn new() -> Self {
        WordList {
            set: Set::new(WORDS_FST).expect("compiled-in word fst must be valid"),
        }
    }

    pub fn len(&self) -> usize {
        self.set.len()
    }

    pub fn is_empty(&self) -> bool {
        self.set.is_empty()
    }

    /// Every word whose Latin key starts with `prefix`, at most `limit` of them.
    ///
    /// The limit counts *words*, not entries. A word is in the list under
    /// several keys — its canonical spelling plus the lowercase and mixed
    /// vowel-length ones — so counting entries let a handful of words with many
    /// spellings fill the quota and pushed real completions out of the list.
    fn starting_with(&self, prefix: &str, limit: usize) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut stream = self.set.search(Str::new(prefix).starts_with()).into_stream();
        while let Some(k) = stream.next() {
            let Ok(s) = std::str::from_utf8(k) else { continue };
            let Some((_, word)) = s.split_once('\t') else { continue };
            if out.iter().any(|w| w == word) {
                continue;
            }
            out.push(word.to_string());
            if out.len() == limit {
                break;
            }
        }
        out
    }

    /// Words whose Latin key is exactly `input`.
    pub fn exact(&self, input: &str) -> Vec<String> {
        let mut out = Vec::new();
        let key = format!("{input}\t");
        let mut stream = self.set.search(Str::new(&key).starts_with()).into_stream();
        while let Some(k) = stream.next() {
            if let Ok(s) = std::str::from_utf8(k) {
                if let Some((_, word)) = s.split_once('\t') {
                    out.push(word.to_string());
                }
            }
        }
        out
    }
}

impl Ranker for WordList {
    fn rank(&self, input: &str, mut cands: Vec<Candidate>) -> Vec<Candidate> {
        if input.is_empty() {
            return cands;
        }

        // An exact key match is the strongest thing this layer can say: the user
        // typed a word, spelled the way the dictionary spells it.
        for word in self.exact(input) {
            merge_candidate(&mut cands, word, 320, Source::Confirmed);
        }

        // Word-final inherent vowels are dropped in the keys (अकबर is `akabar`,
        // not `akabara`), so a typist who does type the trailing `a` still has to
        // find the word. One retry, not a general fuzzy pass.
        if let Some(trimmed) = input.strip_suffix('a') {
            if trimmed.len() >= 2 {
                for word in self.exact(trimmed) {
                    merge_candidate(&mut cands, word, 310, Source::Confirmed);
                }
            }
        }

        // A postposition typed onto the end of a word. Nepali attaches these
        // productively, so no word list can contain the inflected forms —
        // तिमी is in the dictionary and तिमीलाई never will be. Splitting the
        // Latin at a known postposition and looking up the stem builds the form
        // from a word we know is real, which is the same bargain the
        // Devanagari-keyed layer strikes; the difference is that this one works
        // on what was *typed*, so it reaches the 8,000 words that layer cannot.
        for (latin, deva) in POSTPOSITIONS {
            let Some(stem) = input.strip_suffix(latin) else { continue };
            if stem.len() < 2 {
                continue;
            }
            for word in self.exact(stem) {
                if takes_postposition(&word) {
                    merge_candidate(&mut cands, format!("{word}{deva}"), 300, Source::Confirmed);
                }
            }
        }

        // Longer words that begin with what has been typed so far. These are
        // completions, not corrections, and are scored below both of the above.
        if input.len() >= 2 {
            for word in self.starting_with(input, MAX_COMPLETIONS) {
                merge_candidate(&mut cands, word, 95, Source::Dictionary);
            }
        }

        cands
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xlit_core::Engine;

    fn engine() -> Engine {
        Engine::nepali().with_ranker(Box::new(WordList::new()))
    }

    fn texts(input: &str) -> Vec<String> {
        engine().candidates(input).iter().map(|c| c.text.clone()).collect()
    }

    #[test]
    fn word_list_is_populated() {
        assert!(WordList::new().len() > 8000);
    }

    #[test]
    fn exact_key_finds_the_word() {
        // More than one word can share a key: the list carries a lowercase and a
        // short-vowel spelling of every word beside its canonical one, so घर and
        // anything else written `ghar` when length and case are dropped all
        // answer to it. That collision is the point — it is what lets someone
        // type `thulo` and reach ठूलो.
        let got = WordList::new().exact("ghar");
        assert!(got.iter().any(|w| w == "घर"), "got {got:?}");
    }

    #[test]
    fn case_and_vowel_length_are_not_required() {
        // Nobody types the capitals the schema needs for retroflexes, and nobody
        // marks vowel length reliably.
        assert!(texts("dhaal").iter().any(|t| t == "ढाल"), "retroflex ढ via lowercase");
        assert!(texts("thulo").iter().any(|t| t == "ठूलो"), "ठूलो from a short u");
        assert!(texts("nepal").iter().any(|t| t == "नेपाल"), "नेपाल from a short a");
    }

    #[test]
    fn typing_a_prefix_offers_completions() {
        let got = texts("ghar");
        assert_eq!(got[0], "घर", "the literal comes first: {got:?}");
        for want in ["घरबार", "घरधनी"] {
            assert!(got.iter().any(|t| t == want), "missing {want} in {got:?}");
        }
    }

    #[test]
    fn trailing_inherent_vowel_still_matches() {
        // Keys drop the final schwa (`akabar`), but typing it must still work.
        assert!(texts("akabara").iter().any(|t| t == "अकबर"));
    }

    #[test]
    fn generated_verb_forms_are_reachable() {
        assert!(texts("laageko").iter().any(|t| t == "लागेको"));
        assert!(texts("bhayo").iter().any(|t| t == "भयो"));
    }

    #[test]
    fn unknown_input_adds_nothing() {
        let got = texts("xyzqwq");
        assert_eq!(got.len(), 2, "rule output + raw only, got {got:?}");
    }
}
