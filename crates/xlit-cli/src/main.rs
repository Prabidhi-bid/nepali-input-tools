//! `xlit` — tiny REPL for eyeballing engine output during development.
//!
//!   xlit                 # interactive: type Latin, see candidates
//!   xlit namaste duniya  # one-shot

use std::io::{self, BufRead, Write};
use xlit_core::Engine;

fn main() {
    let engine = Engine::nepali();

    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.is_empty() {
        show(&engine, &args.join(" "));
        return;
    }

    eprintln!(
        "xlit [{}] — type Latin text, empty line quits",
        engine.script_name()
    );
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    loop {
        print!("> ");
        io::stdout().flush().ok();
        match lines.next() {
            Some(Ok(line)) if !line.trim().is_empty() => show(&engine, line.trim()),
            _ => break,
        }
    }
}

fn show(engine: &Engine, input: &str) {
    for (i, c) in engine.candidates(input).iter().enumerate() {
        println!("  {}. {}\t[{:?} {}]", i + 1, c.text, c.source, c.score);
    }
}
