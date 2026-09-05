//! Accuracy floors, so that a change to the ranking layers cannot quietly cost
//! more than it buys.
//!
//! The floors are set a little under what the engine scores today (see the
//! table in the crate README). They are a ratchet, not a target: when a change
//! raises the numbers, raise the floors with it in the same commit.

use xlit_core::Engine;
use xlit_dict::{DictRanker, WordList};
use xlit_eval::{builtin_set, run, summarize, Case};

fn engine() -> Engine {
    // No learning store: a run must score the same on every machine.
    Engine::nepali()
        .with_ranker(Box::new(DictRanker::builtin()))
        .with_ranker(Box::new(WordList::new()))
}

fn score(cases: &[Case]) -> xlit_eval::Report {
    let engine = engine();
    let outcomes = run(cases, |input| {
        engine.candidates(input).into_iter().map(|c| c.text).collect()
    });
    summarize(&outcomes).0
}

#[test]
fn overall_accuracy_does_not_regress() {
    let report = score(&builtin_set());
    assert!(
        report.top1_rate() >= 0.90,
        "top-1 fell to {:.1}%",
        report.top1_rate() * 100.0
    );
    assert!(
        report.top5_rate() >= 0.97,
        "top-5 fell to {:.1}%",
        report.top5_rate() * 100.0
    );
    assert!(report.cer() <= 0.04, "CER rose to {:.3}", report.cer());
}

/// Rows spelled in the schema's own conventions need no dictionary at all: if
/// any of them is not first, the rule engine itself has broken, and that is a
/// different and more serious thing than the dictionary missing a word.
#[test]
fn the_rule_engine_alone_is_exact_on_strict_spellings() {
    let cases: Vec<Case> = builtin_set()
        .into_iter()
        .filter(|c| c.tags.iter().any(|t| t == "strict"))
        .collect();
    assert!(!cases.is_empty());

    let engine = engine();
    for case in &cases {
        let got = engine.candidates(&case.input);
        assert_eq!(
            got.first().map(|c| c.text.as_str()),
            Some(case.expected.as_str()),
            "{:?} should transliterate to {} outright",
            case.input,
            case.expected
        );
    }
}

/// The whole reason the dictionary layer exists: casual typing that the rule
/// engine cannot get right on its own. Scored separately so that a change
/// which trades casual accuracy for something else has to say so out loud.
#[test]
fn casual_typing_still_clears_its_own_floor() {
    let cases: Vec<Case> = builtin_set()
        .into_iter()
        .filter(|c| c.tags.iter().any(|t| t == "casual"))
        .collect();
    let report = score(&cases);
    assert!(
        report.top1_rate() >= 0.89,
        "casual top-1 fell to {:.1}%",
        report.top1_rate() * 100.0
    );
}

/// Loanwords and proper nouns are reached through hand-written Latin keys in
/// `xlit-dict/data/latin-keys-ne.tsv` — `computer`, not a transliteration of
/// कम्प्युटर. These rows are in that list by construction, so this is a
/// tripwire for the list still being compiled in and still being consulted,
/// not evidence about words nobody has added yet.
#[test]
fn the_latin_keyed_list_is_reachable() {
    for tag in ["loan", "proper"] {
        let cases: Vec<Case> = builtin_set()
            .into_iter()
            .filter(|c| c.tags.iter().any(|t| t == tag))
            .collect();
        assert!(!cases.is_empty());
        let report = score(&cases);
        assert!(
            report.top1_rate() >= 0.95,
            "{tag} top-1 fell to {:.1}%",
            report.top1_rate() * 100.0
        );
    }
}
