//! `xlit-eval` — how often does the engine put the right word first?
//!
//!   cargo run -p xlit-eval                     the built-in held-out set
//!   cargo run -p xlit-eval -- --set my.tsv     your own
//!   cargo run -p xlit-eval -- --failures 40    show the worst cases
//!   cargo run -p xlit-eval -- --tag casual     only rows with that tag
//!
//! Exit code is 1 when `--min-top1` / `--max-cer` are given and missed, so this
//! can gate a change rather than just describe one.

use std::collections::BTreeMap;

use xlit_core::Engine;
use xlit_dict::{DictRanker, WordList};
use xlit_eval::{builtin_set, parse, run, summarize, Case, Outcome, Report};

fn main() {
    let mut path: Option<String> = None;
    let mut failures = 10usize;
    let mut tag: Option<String> = None;
    let mut min_top1: Option<f64> = None;
    let mut max_cer: Option<f64> = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = |what: &str| -> String {
            args.next()
                .unwrap_or_else(|| fail(&format!("{what} needs a value")))
        };
        match arg.as_str() {
            "--set" | "-s" => path = Some(value("--set")),
            "--failures" | "-f" => {
                failures = value("--failures").parse().unwrap_or_else(|_| fail("--failures needs a number"))
            }
            "--tag" | "-t" => tag = Some(value("--tag")),
            "--min-top1" => {
                min_top1 = Some(value("--min-top1").parse().unwrap_or_else(|_| fail("--min-top1 needs a fraction")))
            }
            "--max-cer" => {
                max_cer = Some(value("--max-cer").parse().unwrap_or_else(|_| fail("--max-cer needs a fraction")))
            }
            "--help" | "-h" => {
                println!("{HELP}");
                return;
            }
            other => fail(&format!("unknown argument {other:?} (try --help)")),
        }
    }

    let mut cases = match &path {
        Some(p) => {
            let text = std::fs::read_to_string(p)
                .unwrap_or_else(|e| fail(&format!("cannot read {p}: {e}")));
            parse(&text).unwrap_or_else(|e| fail(&e))
        }
        None => builtin_set(),
    };
    if let Some(t) = &tag {
        cases.retain(|c| c.tags.iter().any(|x| x == t));
        if cases.is_empty() {
            fail(&format!("no cases tagged {t:?}"));
        }
    }

    // No learning store: the harness measures what the engine knows, not what
    // some machine happens to have been taught, so a run is reproducible
    // anywhere.
    let engine = Engine::nepali()
        .with_ranker(Box::new(DictRanker::builtin()))
        .with_ranker(Box::new(WordList::new()));

    let outcomes = run(&cases, |input| {
        engine.candidates(input).into_iter().map(|c| c.text).collect()
    });
    let (overall, by_tag) = summarize(&outcomes);

    println!(
        "{} cases from {}\n",
        overall.cases,
        path.as_deref().unwrap_or("the built-in set")
    );
    print_table(&overall, &by_tag);
    if failures > 0 {
        print_failures(&outcomes, failures);
    }

    let mut failed = false;
    if let Some(min) = min_top1 {
        if overall.top1_rate() < min {
            eprintln!(
                "FAIL: top-1 {:.1}% is below the required {:.1}%",
                overall.top1_rate() * 100.0,
                min * 100.0
            );
            failed = true;
        }
    }
    if let Some(max) = max_cer {
        if overall.cer() > max {
            eprintln!(
                "FAIL: CER {:.3} is above the allowed {max:.3}",
                overall.cer()
            );
            failed = true;
        }
    }
    if failed {
        std::process::exit(1);
    }
}

const HELP: &str = "\
xlit-eval — transliteration accuracy against a held-out word list

  --set PATH        evaluate this TSV (latin<TAB>devanagari<TAB>tags)
  --tag TAG         only rows carrying TAG (casual, strict, loan, ...)
  --failures N      list the N worst cases (default 10; 0 for none)
  --min-top1 F      exit 1 if top-1 accuracy is below F (0..1)
  --max-cer F       exit 1 if the character error rate is above F";

fn fail(msg: &str) -> ! {
    eprintln!("xlit-eval: {msg}");
    std::process::exit(2);
}

fn print_table(overall: &Report, by_tag: &BTreeMap<String, Report>) {
    println!(
        "{:<10} {:>6} {:>8} {:>8} {:>7} {:>7}",
        "", "cases", "top-1", "top-5", "MRR", "CER"
    );
    print_row("overall", overall);
    for (tag, report) in by_tag {
        print_row(tag, report);
    }
}

fn print_row(label: &str, r: &Report) {
    println!(
        "{:<10} {:>6} {:>7.1}% {:>7.1}% {:>7.3} {:>7.3}",
        label,
        r.cases,
        r.top1_rate() * 100.0,
        r.top5_rate() * 100.0,
        r.mrr(),
        r.cer()
    );
}

/// The cases worth looking at first: never offered at all, then offered but not
/// first, worst edit distance leading.
fn print_failures(outcomes: &[Outcome], limit: usize) {
    let mut bad: Vec<&Outcome> = outcomes.iter().filter(|o| o.rank != Some(1)).collect();
    if bad.is_empty() {
        println!("\nno failures");
        return;
    }
    bad.sort_by(|a, b| {
        rank_key(a)
            .cmp(&rank_key(b))
            .then_with(|| b.distance.cmp(&a.distance))
            .then_with(|| a.case.input.cmp(&b.case.input))
    });
    println!(
        "\n{} of {} cases are not top-1; worst {}:",
        bad.len(),
        outcomes.len(),
        limit.min(bad.len())
    );
    for o in bad.iter().take(limit) {
        let Case { input, expected, tags } = &o.case;
        let rank = match o.rank {
            Some(r) => format!("rank {r}"),
            None => "absent".to_string(),
        };
        println!(
            "  {input:<14} want {expected:<16} got {:<16} {rank:>8}  d={} [{}]",
            o.top1,
            o.distance,
            tags.join(",")
        );
    }
}

/// Absent from the list is worse than merely low in it.
fn rank_key(o: &Outcome) -> usize {
    o.rank.unwrap_or(usize::MAX)
}
