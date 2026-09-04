//! `xlit` — dev REPL for the transliteration engine.
//!
//!   xlit                     interactive: type Latin -> ranked candidates.
//!                            Type a number to commit that candidate (learned,
//!                            persisted to ./xlit-learn.json).
//!   xlit namaste dhanyabaad  one-shot (rule + dictionary, no learning).

use std::io::{self, BufRead, Write};
use std::sync::Arc;

use xlit_core::{Candidate, Engine, Ranker};
use xlit_dict::DictRanker;
use xlit_learn::LearnStore;

fn main() {
    let one_shot: Vec<String> = std::env::args().skip(1).collect();

    if !one_shot.is_empty() {
        let engine = Engine::nepali().with_ranker(Box::new(DictRanker::builtin()));
        print_list(&engine.candidates(&one_shot.join(" ")));
        return;
    }

    let store = Arc::new(LearnStore::open("xlit-learn.json").unwrap_or_else(|e| {
        eprintln!("(learning disabled: {e})");
        LearnStore::in_memory()
    }));

    let engine = Engine::nepali()
        .with_ranker(Box::new(DictRanker::builtin()))
        .with_ranker(Box::new(SharedLearn(store.clone())));

    eprintln!(
        "xlit [{}] — type Latin; a number commits that candidate; empty line quits",
        engine.script_name()
    );

    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let mut last: Vec<Candidate> = Vec::new();
    let mut last_input = String::new();

    loop {
        print!("> ");
        io::stdout().flush().ok();
        let line = match lines.next() {
            Some(Ok(l)) => l,
            _ => break,
        };
        let line = line.trim().to_string();
        if line.is_empty() {
            break;
        }

        if let Ok(n) = line.parse::<usize>() {
            if (1..=last.len()).contains(&n) {
                let chosen = &last[n - 1].text;
                match store.record(&last_input, chosen) {
                    Ok(()) => println!("  learned: {last_input:?} -> {chosen}"),
                    Err(e) => println!("  (could not save: {e})"),
                }
                continue;
            }
        }

        last_input = line.clone();
        last = engine.candidates(&line);
        print_list(&last);
    }
}

/// Lets the learning store live behind an `Arc` while also being a `Ranker`
/// inside the engine.
struct SharedLearn(Arc<LearnStore>);

impl Ranker for SharedLearn {
    fn rank(&self, input: &str, cands: Vec<Candidate>) -> Vec<Candidate> {
        self.0.rank(input, cands)
    }
}

fn print_list(cands: &[Candidate]) {
    for (i, c) in cands.iter().enumerate() {
        println!("  {}. {}\t[{:?} {}]", i + 1, c.text, c.source, c.score);
    }
}
