//! Accuracy measurement for the transliteration pipeline.
//!
//! The engine is a stack of heuristics — rule engine, then dictionary passes
//! that re-rank it — and every change to one of them trades some inputs for
//! others. Without a number attached, "this fixes `hello`" and "this breaks
//! twenty words nobody tried" look identical. This crate attaches the number.
//!
//! What it reports, against a held-out list of (Latin typed, Devanagari meant):
//!
//! - **top-1** — the first candidate is the intended word. This is the metric
//!   that matters: it is what pressing space gives you.
//! - **top-5** — the word is somewhere in the visible candidate list, so a
//!   number key reaches it.
//! - **MRR** — mean reciprocal rank; 1.0 if everything is first, 0.5 if
//!   everything is second. Sensitive to movement inside the list in a way the
//!   two accuracies are not.
//! - **CER** — character error rate of the top-1 answer against the intended
//!   word (Levenshtein over Unicode scalar values / length of the intended
//!   word). A wrong first candidate that is one matra out is a different
//!   failure from a wrong first candidate that is a different word, and only
//!   CER separates them.
//!
//! Scores are also broken down by tag, because the aggregate hides the thing
//! worth knowing: `strict` rows test the rule engine alone and should be at
//! 100%, while `casual` rows can only be got right by the dictionary.

use std::collections::BTreeMap;

/// One row of the evaluation set.
#[derive(Debug, Clone)]
pub struct Case {
    /// What the user types.
    pub input: String,
    /// What they meant.
    pub expected: String,
    pub tags: Vec<String>,
}

/// What the engine did with one [`Case`].
#[derive(Debug, Clone)]
pub struct Outcome {
    pub case: Case,
    /// 1-based rank of the expected word, or `None` if it was not offered at all.
    pub rank: Option<usize>,
    /// The candidate the user would get by pressing space.
    pub top1: String,
    /// Edit distance from `top1` to the expected word, in characters.
    pub distance: usize,
}

/// Aggregate scores over a set of outcomes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Report {
    pub cases: usize,
    pub top1: usize,
    pub top5: usize,
    /// Sum of 1/rank; divide by `cases` for MRR.
    pub reciprocal_rank: f64,
    /// Sum of edit distances and of expected-word lengths, so that CER is a
    /// corpus-level ratio rather than an average of per-word ratios (a
    /// three-character word being wrong should not weigh as much as a
    /// fifteen-character one being wrong).
    pub distance: usize,
    pub expected_len: usize,
}

impl Report {
    pub fn add(&mut self, o: &Outcome) {
        self.cases += 1;
        if o.rank == Some(1) {
            self.top1 += 1;
        }
        if matches!(o.rank, Some(r) if r <= 5) {
            self.top5 += 1;
        }
        if let Some(r) = o.rank {
            self.reciprocal_rank += 1.0 / r as f64;
        }
        self.distance += o.distance;
        self.expected_len += o.case.expected.chars().count();
    }

    pub fn top1_rate(&self) -> f64 {
        ratio(self.top1, self.cases)
    }

    pub fn top5_rate(&self) -> f64 {
        ratio(self.top5, self.cases)
    }

    pub fn mrr(&self) -> f64 {
        if self.cases == 0 {
            0.0
        } else {
            self.reciprocal_rank / self.cases as f64
        }
    }

    pub fn cer(&self) -> f64 {
        ratio(self.distance, self.expected_len)
    }
}

fn ratio(a: usize, b: usize) -> f64 {
    if b == 0 {
        0.0
    } else {
        a as f64 / b as f64
    }
}

/// Parse an evaluation set: `latin <TAB> devanagari <TAB> tag,tag`.
/// Blank lines and `#` comments are skipped; a missing tag column is allowed.
pub fn parse(text: &str) -> Result<Vec<Case>, String> {
    let mut cases = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut cols = line.split('\t');
        let input = cols.next().unwrap_or_default().trim();
        let expected = cols.next().unwrap_or_default().trim();
        if input.is_empty() || expected.is_empty() {
            return Err(format!("line {}: need `latin<TAB>devanagari`", n + 1));
        }
        let tags = cols
            .next()
            .unwrap_or("")
            .split(',')
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .collect();
        cases.push(Case {
            input: input.to_string(),
            expected: expected.to_string(),
            tags,
        });
    }
    Ok(cases)
}

/// The set that ships with the crate.
pub fn builtin_set() -> Vec<Case> {
    parse(include_str!("../data/ne-eval.tsv")).expect("built-in eval set is malformed")
}

/// Run every case through `candidates` and score the result.
///
/// Takes the candidate function rather than an `Engine` so the same harness can
/// score a differently-layered engine, or one behind the daemon, without the
/// scoring code knowing the difference.
pub fn run<F>(cases: &[Case], mut candidates: F) -> Vec<Outcome>
where
    F: FnMut(&str) -> Vec<String>,
{
    cases
        .iter()
        .map(|case| {
            let cands = candidates(&case.input);
            let rank = cands.iter().position(|c| c == &case.expected).map(|i| i + 1);
            let top1 = cands.first().cloned().unwrap_or_default();
            Outcome {
                distance: edit_distance(&top1, &case.expected),
                case: case.clone(),
                rank,
                top1,
            }
        })
        .collect()
}

/// Overall report plus one per tag, in tag order.
pub fn summarize(outcomes: &[Outcome]) -> (Report, BTreeMap<String, Report>) {
    let mut overall = Report::default();
    let mut by_tag: BTreeMap<String, Report> = BTreeMap::new();
    for o in outcomes {
        overall.add(o);
        for tag in &o.case.tags {
            by_tag.entry(tag.clone()).or_default().add(o);
        }
    }
    (overall, by_tag)
}

/// Levenshtein distance in Unicode scalar values.
///
/// Characters, not bytes and not grapheme clusters: a matra is its own code
/// point and getting one wrong is exactly the single-character error this is
/// meant to count as one.
pub fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    // One row of the matrix; `prev` carries the diagonal.
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            let next = (row[j + 1] + 1).min(row[j] + 1).min(prev + cost);
            prev = row[j + 1];
            row[j + 1] = next;
        }
    }
    row[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_distance_counts_characters_not_bytes() {
        // One matra apart: नेपालि vs नेपाली is a single-character error, even
        // though each of those characters is three UTF-8 bytes.
        assert_eq!(edit_distance("नेपालि", "नेपाली"), 1);
        assert_eq!(edit_distance("", "घर"), 2);
        assert_eq!(edit_distance("घर", "घर"), 0);
    }

    #[test]
    fn parse_reads_tags_and_skips_comments() {
        let cases = parse("# note\n\nghar\tघर\tcasual,core\npaani\tपानी\n").unwrap();
        assert_eq!(cases.len(), 2);
        assert_eq!(cases[0].tags, vec!["casual", "core"]);
        assert!(cases[1].tags.is_empty());
    }

    #[test]
    fn parse_rejects_a_row_without_an_expected_word() {
        assert!(parse("ghar\n").is_err());
    }

    #[test]
    fn scoring_follows_the_rank_of_the_expected_word() {
        let cases = parse("a\tक\n b\tख\n").unwrap();
        let outcomes = run(&cases, |input| match input {
            "a" => vec!["क".into(), "ख".into()],
            _ => vec!["ग".into(), "घ".into(), "ङ".into(), "च".into(), "ख".into()],
        });
        assert_eq!(outcomes[0].rank, Some(1));
        assert_eq!(outcomes[1].rank, Some(5));

        let (report, _) = summarize(&outcomes);
        assert_eq!(report.top1, 1);
        assert_eq!(report.top5, 2);
        assert!((report.mrr() - (1.0 + 0.2) / 2.0).abs() < 1e-9);
    }

    #[test]
    fn a_word_that_is_never_offered_scores_nothing_but_still_counts() {
        let cases = parse("a\tक\n").unwrap();
        let outcomes = run(&cases, |_| vec!["ख".into()]);
        assert_eq!(outcomes[0].rank, None);
        let (report, _) = summarize(&outcomes);
        assert_eq!(report.cases, 1);
        assert_eq!(report.top5, 0);
        assert_eq!(report.mrr(), 0.0);
        assert_eq!(report.cer(), 1.0);
    }

    #[test]
    fn the_builtin_set_parses_and_is_not_trivially_small() {
        let cases = builtin_set();
        assert!(cases.len() > 100, "only {} cases", cases.len());
        for c in &cases {
            assert!(
                c.input.is_ascii(),
                "{:?} is what the user types: it should be Latin",
                c.input
            );
            assert!(
                !c.expected.is_ascii(),
                "{:?} expects ASCII, which cannot be right",
                c.input
            );
        }
    }
}
