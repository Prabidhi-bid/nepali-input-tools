//! `xlit-daemon` — one process per user that owns the transliteration engine.
//!
//! Frontends could link `xlit-core` directly, and on Linux they do: an IBus
//! engine is a single long-lived process, so there is nothing to share with.
//! Windows is the reason this exists. A TSF text service is loaded into *every*
//! process that accepts text — Chrome's dozen renderers, Explorer, Word — and a
//! dictionary held by the service is a dictionary paid for that many times.
//! Here it is paid for once, and the clients are a socket handle and a
//! composition buffer.
//!
//! Usage:
//!
//!   xlit-daemon                 run in the foreground, log to stderr
//!   xlit-daemon --socket PATH   listen somewhere other than the default
//!   xlit-daemon --stop          ask a running daemon to exit
//!   xlit-daemon --status        report whether one is listening
//!
//! `$XLIT_SOCKET` overrides the default endpoint for every xlit program.

use std::path::PathBuf;
use std::sync::Arc;

use xlit_core::{Candidate, Engine, Ranker, Source};
use xlit_dict::{DictRanker, WordList};
use xlit_ipc::{transport, Request, Response};
use xlit_learn::LearnStore;

/// Most candidates we ever return. IBus numbers a page 1-9 and the TSF
/// candidate window shows the same nine, so a longer list is only bytes.
const MAX_CANDIDATES: usize = 9;

fn main() {
    let mut endpoint = transport::default_endpoint();
    let mut data_dir: Option<PathBuf> = None;
    let mut mode = Mode::Serve;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--socket" | "-s" => match args.next() {
                Some(v) => endpoint = v,
                None => fail("--socket needs a path"),
            },
            "--data-dir" => match args.next() {
                Some(v) => data_dir = Some(PathBuf::from(v)),
                None => fail("--data-dir needs a path"),
            },
            "--stop" => mode = Mode::Stop,
            "--status" => mode = Mode::Status,
            "--help" | "-h" => {
                println!("{}", HELP);
                return;
            }
            other => fail(&format!("unknown argument {other:?} (try --help)")),
        }
    }

    match mode {
        Mode::Status => {
            if transport::is_running(&endpoint) {
                println!("xlit-daemon: running at {endpoint}");
            } else {
                println!("xlit-daemon: not running ({endpoint})");
                std::process::exit(1);
            }
        }
        Mode::Stop => match xlit_ipc::Client::connect_at(&endpoint) {
            Ok(mut c) => match c.call(&Request::Shutdown) {
                Ok(_) => println!("xlit-daemon: stopping"),
                Err(e) => fail(&format!("could not stop the daemon: {e}")),
            },
            Err(_) => println!("xlit-daemon: not running ({endpoint})"),
        },
        Mode::Serve => serve(&endpoint, data_dir),
    }
}

enum Mode {
    Serve,
    Stop,
    Status,
}

const HELP: &str = "\
xlit-daemon — transliteration engine host

  xlit-daemon                 run in the foreground
  xlit-daemon --socket PATH   listen on PATH (or \\\\.\\pipe\\... on Windows)
  xlit-daemon --data-dir DIR  keep the learning store in DIR
  xlit-daemon --stop          ask a running daemon to exit
  xlit-daemon --status        report whether one is listening";

fn fail(msg: &str) -> ! {
    eprintln!("xlit-daemon: {msg}");
    std::process::exit(2);
}

fn serve(endpoint: &str, data_dir: Option<PathBuf>) {
    let dir = data_dir.unwrap_or_else(default_data_dir);
    let _ = std::fs::create_dir_all(&dir);

    let learn = Arc::new(
        LearnStore::open(dir.join("xlit-learn.json")).unwrap_or_else(|e| {
            eprintln!("xlit-daemon: learning disabled: {e}");
            LearnStore::in_memory()
        }),
    );
    // Same layering, and the same reasoning, as the frontends' own `engine.rs`:
    // every ranker only raises scores, so registration order is precedence.
    let engine = Engine::nepali()
        .with_ranker(Box::new(DictRanker::builtin()))
        .with_ranker(Box::new(WordList::new()))
        .with_ranker(Box::new(SharedLearn(learn.clone())));

    let listener = match transport::bind(endpoint) {
        Ok(l) => l,
        Err(e) => fail(&format!("cannot listen on {endpoint}: {e}")),
    };
    eprintln!(
        "xlit-daemon: [{}] listening on {endpoint} (learning in {})",
        engine.script_name(),
        dir.display()
    );

    let state = Arc::new(State { engine, learn });
    let result = xlit_ipc::serve(listener, move |req| state.handle(req));
    if let Err(e) = result {
        fail(&format!("serve failed: {e}"));
    }
}

struct State {
    engine: Engine,
    learn: Arc<LearnStore>,
}

impl State {
    fn handle(&self, req: Request) -> Response {
        match req {
            Request::Ping | Request::Shutdown => Response::ok(),

            Request::Transliterate { text } => Response::text(self.engine.transliterate(&text)),

            Request::Candidates { text, limit } => {
                let limit = limit.unwrap_or(MAX_CANDIDATES).min(MAX_CANDIDATES);
                Response::candidates(self.candidates(&text, limit))
            }

            Request::Commit { input, chosen } => {
                if input.is_empty() || chosen.is_empty() {
                    return Response::error("commit needs a non-empty input and choice");
                }
                match self.learn.record(&input, &chosen) {
                    Ok(()) => Response::ok(),
                    // A failed write must not look like a failed keystroke to
                    // the frontend, but it should not be silent either.
                    Err(e) => Response::error(format!("could not save the pick: {e}")),
                }
            }
        }
    }

    /// Deduplicated, capped candidate list — the same shape the frontends build
    /// for themselves when they link the engine directly, so that switching a
    /// frontend to client mode changes nothing a user can see.
    fn candidates(&self, input: &str, limit: usize) -> Vec<xlit_ipc::Candidate> {
        if input.is_empty() || limit == 0 {
            return Vec::new();
        }
        // Another process may have edited the store (the word editor does).
        self.learn.reload_if_changed();

        let mut out: Vec<xlit_ipc::Candidate> = Vec::with_capacity(limit);
        for c in self.engine.candidates(input) {
            if c.text.is_empty() || out.iter().any(|o| o.text == c.text) {
                continue;
            }
            out.push(wire(c));
            if out.len() == limit {
                break;
            }
        }
        // Always leave a way back to exactly what was typed.
        if !out.iter().any(|c| c.text == input) && out.len() < limit {
            out.push(xlit_ipc::Candidate {
                text: input.to_string(),
                source: "raw".to_string(),
                score: 0,
            });
        }
        out
    }
}

fn wire(c: Candidate) -> xlit_ipc::Candidate {
    let source = match c.source {
        Source::Rule => "rule",
        Source::Dictionary => "dictionary",
        Source::Confirmed => "confirmed",
        Source::Learned => "learned",
        Source::Model => "model",
        Source::Raw => "raw",
    };
    xlit_ipc::Candidate {
        text: c.text,
        source: source.to_string(),
        score: c.score,
    }
}

struct SharedLearn(Arc<LearnStore>);

impl Ranker for SharedLearn {
    fn rank(&self, input: &str, cands: Vec<Candidate>) -> Vec<Candidate> {
        self.0.rank(input, cands)
    }
}

/// `$XDG_DATA_HOME/xlit` on Unix, `%APPDATA%\xlit` on Windows — the same
/// locations the frontends use, so a daemon and a directly-linked frontend
/// share one learning store rather than quietly keeping two.
fn default_data_dir() -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            return PathBuf::from(appdata).join("xlit");
        }
    }
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("xlit")
}
