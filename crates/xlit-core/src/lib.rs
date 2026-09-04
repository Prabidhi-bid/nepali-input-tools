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

        let t = self.rule.transliterate(input);
        if !t.is_empty() && t != input {
            cands.push(Candidate {
                text: t,
                source: Source::Rule,
                score: 100,
            });
        }
        cands.push(Candidate {
            text: input.to_string(),
            source: Source::Raw,
            score: 1,
        });

        for r in &self.rankers {
            cands = r.rank(input, cands);
        }

        cands.sort_by(|a, b| b.score.cmp(&a.score));
        cands.dedup_by(|a, b| a.text == b.text);
        cands
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
}
