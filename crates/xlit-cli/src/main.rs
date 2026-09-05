//! `xlit` — dev REPL for the transliteration engine.
//!
//!   xlit                     interactive: type Latin -> ranked candidates.
//!                            Type a number to commit that candidate (learned,
//!                            persisted to ./xlit-learn.json).
//!   xlit namaste dhanyabaad  one-shot (rule + dictionary, no learning).
//!   xlit --client [...]      talk to a running `xlit-daemon` instead of
//!                            building an engine in-process. Same output; it is
//!                            the daemon's dictionary and learning store.
//!   xlit --socket PATH       endpoint for --client (default: $XLIT_SOCKET, or
//!                            the platform default).

use std::io::{self, BufRead, Write};
use std::sync::Arc;

use xlit_core::{Candidate, Engine, Ranker, Source};
use xlit_dict::{DictRanker, WordList};
use xlit_ipc::Client;
use xlit_learn::LearnStore;

fn main() {
    let mut client_mode = false;
    let mut endpoint: Option<String> = None;
    let mut words: Vec<String> = Vec::new();

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--client" | "-c" => client_mode = true,
            "--socket" | "-s" => match args.next() {
                Some(v) => {
                    endpoint = Some(v);
                    client_mode = true;
                }
                None => {
                    eprintln!("xlit: --socket needs a path");
                    std::process::exit(2);
                }
            },
            "--help" | "-h" => {
                println!("{HELP}");
                return;
            }
            other => words.push(other.to_string()),
        }
    }

    let mut backend = match Backend::open(client_mode, endpoint) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("xlit: {e}");
            std::process::exit(1);
        }
    };

    if !words.is_empty() {
        print_list(&backend.candidates(&words.join(" ")));
        return;
    }

    eprintln!(
        "xlit [{}] — type Latin; a number commits that candidate; empty line quits",
        backend.describe()
    );

    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let mut last: Vec<Shown> = Vec::new();
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
                let chosen = last[n - 1].text.clone();
                match backend.commit(&last_input, &chosen) {
                    Ok(()) => println!("  learned: {last_input:?} -> {chosen}"),
                    Err(e) => println!("  (could not save: {e})"),
                }
                continue;
            }
        }

        last_input = line.clone();
        last = backend.candidates(&line);
        print_list(&last);
    }
}

const HELP: &str = "\
xlit — transliteration REPL

  xlit                    interactive; a number commits that candidate
  xlit namaste duniya     one-shot
  xlit --client           use a running xlit-daemon instead of a local engine
  xlit --socket PATH      daemon endpoint (implies --client)";

/// One candidate as the REPL shows it — the two backends produce the same rows
/// from different places, so the printing code does not care which ran.
struct Shown {
    text: String,
    source: String,
    score: i32,
}

enum Backend {
    /// Engine built in this process. Learns into `./xlit-learn.json`.
    Local {
        engine: Engine,
        store: Arc<LearnStore>,
    },
    /// Thin client of `xlit-daemon`; the daemon owns the data and the learning.
    Remote(Box<Client>),
}

impl Backend {
    fn open(client_mode: bool, endpoint: Option<String>) -> io::Result<Self> {
        if client_mode {
            let client = match &endpoint {
                Some(e) => Client::connect_at(e),
                None => Client::connect(),
            }
            .map_err(|e| {
                io::Error::other(format!(
                    "no daemon at {}: {e} (start one with `xlit-daemon`)",
                    endpoint.unwrap_or_else(xlit_ipc::default_endpoint)
                ))
            })?;
            return Ok(Backend::Remote(Box::new(client)));
        }

        let store = Arc::new(LearnStore::open("xlit-learn.json").unwrap_or_else(|e| {
            eprintln!("(learning disabled: {e})");
            LearnStore::in_memory()
        }));
        let engine = Engine::nepali()
            .with_ranker(Box::new(DictRanker::builtin()))
            .with_ranker(Box::new(WordList::new()))
            .with_ranker(Box::new(SharedLearn(store.clone())));
        Ok(Backend::Local { engine, store })
    }

    fn describe(&self) -> String {
        match self {
            Backend::Local { engine, .. } => engine.script_name().to_string(),
            Backend::Remote(_) => "daemon".to_string(),
        }
    }

    fn candidates(&mut self, input: &str) -> Vec<Shown> {
        match self {
            Backend::Local { engine, .. } => engine
                .candidates(input)
                .into_iter()
                .map(|c| Shown {
                    text: c.text,
                    source: source_name(c.source).to_string(),
                    score: c.score,
                })
                .collect(),
            Backend::Remote(client) => match client.candidates(input, None) {
                Ok(cands) => cands
                    .into_iter()
                    .map(|c| Shown {
                        text: c.text,
                        source: c.source,
                        score: c.score,
                    })
                    .collect(),
                Err(e) => {
                    eprintln!("xlit: daemon: {e}");
                    Vec::new()
                }
            },
        }
    }

    fn commit(&mut self, input: &str, chosen: &str) -> io::Result<()> {
        match self {
            Backend::Local { store, .. } => store.record(input, chosen),
            Backend::Remote(client) => client.commit(input, chosen),
        }
    }
}

fn source_name(s: Source) -> &'static str {
    match s {
        Source::Rule => "rule",
        Source::Dictionary => "dictionary",
        Source::Confirmed => "confirmed",
        Source::Learned => "learned",
        Source::Model => "model",
        Source::Raw => "raw",
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

fn print_list(cands: &[Shown]) {
    for (i, c) in cands.iter().enumerate() {
        println!("  {}. {}\t[{} {}]", i + 1, c.text, c.source, c.score);
    }
}
