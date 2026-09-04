//! xlit-core — portable transliteration engine.
//!
//! Design: a small deterministic **rule engine** always runs first. Optional
//! layers (dictionary, language model, neural fallback) refine and re-rank its
//! output. Only the rule engine exists today; the [`Ranker`] trait is the seam
//! the other layers plug into.
//!
//! Nothing here touches the OS. Platform frontends (Windows TSF, Linux IBus /
//! Fcitx5) link this crate or talk to a daemon that wraps it.

mod schema;
mod translit;

pub use schema::{Schema, Vowel};
pub use translit::RuleEngine;

/// Where a candidate came from. Higher layers get higher base scores.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Verbatim rule-engine transliteration.
    Rule,
    /// A dictionary word that matches / completes the input.
    Dictionary,
    /// Boosted because the user has picked it before.
    Learned,
    /// Neural fallback for out-of-vocabulary input.
    Model,
    /// The raw Latin text, always offered last.
    Raw,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub text: String,
    pub source: Source,
    pub score: i32,
}

/// Helper for [`Ranker`] implementations: insert `text` as a candidate, or — if
/// an entry with the same text already exists — raise its score and adopt
/// `source` when `score` is higher than what's there.
pub fn merge_candidate(cands: &mut Vec<Candidate>, text: String, score: i32, source: Source) {
    for c in cands.iter_mut() {
        if c.text == text {
            if score > c.score {
                c.score = score;
                c.source = source;
            }
            return;
        }
    }
    cands.push(Candidate { text, source, score });
}

/// A refinement layer. Implementors see the raw input and the candidates so far
/// and return a new list (add, drop, re-score). Layers run in registration order.
pub trait Ranker: Send + Sync {
    fn rank(&self, input: &str, cands: Vec<Candidate>) -> Vec<Candidate>;
}

pub struct Engine {
    rule: RuleEngine,
    rankers: Vec<Box<dyn Ranker>>,
}

impl Engine {
    pub fn new(schema: Schema) -> Self {
        Engine {
            rule: RuleEngine::new(schema),
            rankers: Vec::new(),
        }
    }

    /// Engine preloaded with the compiled-in Nepali schema.
    pub fn nepali() -> Self {
        Engine::new(Schema::nepali())
    }

    /// Register a refinement layer (dictionary, LM, ...). Chainable.
    pub fn with_ranker(mut self, r: Box<dyn Ranker>) -> Self {
        self.rankers.push(r);
        self
    }

    pub fn script_name(&self) -> &str {
        self.rule.name()
    }

    /// Ranked candidates for a raw Latin buffer, best first.
    pub fn candidates(&self, input: &str) -> Vec<Candidate> {
        let mut cands = Vec::new();

        // Primary literal transliteration at 100; orthographic variants (e.g.
        // the de-geminated form of a loanword) just below it, so they only
        // surface when a later layer — the dictionary — confirms them as words.
        for (rank, t) in self.rule.transliterate_variants(input).into_iter().enumerate() {
            if !t.is_empty() && t != input {
                let score = if rank == 0 { 100 } else { 96 };
                merge_candidate(&mut cands, t, score, Source::Rule);
            }
        }
        cands.push(Candidate {
            text: input.to_string(),
            source: Source::Raw,
            score: 1,
        });

        for r in &self.rankers {
            cands = r.rank(input, cands);
        }

        // Collapse duplicates, keeping the highest-scoring entry per text.
        let mut merged: Vec<Candidate> = Vec::with_capacity(cands.len());
        'next: for c in cands {
            for m in &mut merged {
                if m.text == c.text {
                    if c.score > m.score {
                        *m = c;
                    }
                    continue 'next;
                }
            }
            merged.push(c);
        }
        merged.sort_by(|a, b| b.score.cmp(&a.score));
        merged
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_ranks_rule_above_raw() {
        let e = Engine::nepali();
        let c = e.candidates("namaste");
        assert_eq!(c[0].text, "नमस्ते");
        assert_eq!(c[0].source, Source::Rule);
        assert_eq!(c.last().unwrap().text, "namaste");
    }

    #[test]
    fn geminate_variant_is_offered_but_not_promoted() {
        // With no dictionary the literal conjunct stays on top; the
        // de-geminated loanword form is still present as a lower candidate.
        let e = Engine::nepali();
        let c = e.candidates("hello");
        assert_eq!(c[0].text, "हेल्लो");
        let texts: Vec<_> = c.iter().map(|x| x.text.as_str()).collect();
        assert!(texts.contains(&"हेलो"), "got {texts:?}");
    }

    #[test]
    fn nasal_conjunct_variant_is_a_candidate() {
        // "raviiMdra" (Hindi-style anusvara) still offers the Nepali रवीन्द्र.
        let e = Engine::nepali();
        let texts: Vec<_> = e
            .candidates("raviiMdra")
            .iter()
            .map(|x| x.text.clone())
            .collect();
        assert!(texts.contains(&"रवीन्द्र".to_string()), "got {texts:?}");
    }
}
