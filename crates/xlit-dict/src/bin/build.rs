//! Build a `.fst` dictionary from a `word<TAB>freq` TSV.
//!
//!   cargo run -p xlit-dict --bin build -- words.tsv words.fst
//!
//! Lines starting with `#` and blank lines are ignored. Duplicate words keep
//! the highest frequency. Keys are written in sorted order as `fst` requires.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, BufWriter};

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let (input, output) = match (args.next(), args.next()) {
        (Some(i), Some(o)) => (i, o),
        _ => {
            eprintln!("usage: build <input.tsv> <output.fst>");
            std::process::exit(2);
        }
    };

    let reader = BufReader::new(std::fs::File::open(&input)?);
    let mut sorted: BTreeMap<String, u64> = BTreeMap::new();
    for (n, line) in reader.lines().enumerate() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut it = line.split('\t');
        let word = it.next().unwrap_or("").trim().to_string();
        let freq: u64 = match it.next().unwrap_or("1").trim().parse() {
            Ok(f) => f,
            Err(_) => {
                eprintln!("skipping line {}: bad frequency", n + 1);
                continue;
            }
        };
        if word.is_empty() {
            continue;
        }
        let slot = sorted.entry(word).or_insert(0);
        *slot = (*slot).max(freq);
    }

    let writer = BufWriter::new(std::fs::File::create(&output)?);
    let mut b = fst::MapBuilder::new(writer).map_err(io_err)?;
    for (w, f) in &sorted {
        b.insert(w, *f).map_err(io_err)?;
    }
    b.finish().map_err(io_err)?;

    println!("wrote {output} ({} entries)", sorted.len());
    Ok(())
}

fn io_err<E: std::fmt::Display>(e: E) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, e.to_string())
}
