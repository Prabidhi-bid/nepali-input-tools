//! Compile `data/ne-words.tsv` into an `fst::Set` at build time.
//!
//! The set holds one entry per word, `"<latin key>\t<devanagari>"`, so a prefix
//! scan for `"ghar"` finds घर and every longer word whose key starts with it,
//! and a scan for `"ghar\t"` finds just the words that key exactly. Encoding the
//! word into the key is what lets one Latin spelling map to several words —
//! an `fst::Map` value is a single `u64` and could not.
//!
//! Building here rather than at startup matters: the Windows text service is
//! loaded into every process that takes keyboard input, and none of them should
//! pay to re-parse 8,000 lines.

use std::io::{BufRead, BufReader, BufWriter};
use std::path::PathBuf;

fn main() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data/ne-words.tsv");
    println!("cargo:rerun-if-changed={}", src.display());

    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR")).join("ne-words.fst");
    let file = std::fs::File::open(&src)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", src.display()));

    // Already sorted by the generator; SetBuilder rejects anything out of order,
    // so a mis-sorted file fails the build rather than silently losing words.
    let mut b = fst::SetBuilder::new(BufWriter::new(
        std::fs::File::create(&out).expect("create fst"),
    ))
    .expect("fst builder");

    let mut n = 0usize;
    let mut prev = String::new();
    for line in BufReader::new(file).lines() {
        let line = line.expect("read line");
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, word)) = line.split_once('\t') else { continue };
        if key.is_empty() || word.is_empty() {
            continue;
        }
        let entry = format!("{key}\t{word}");
        if entry <= prev {
            panic!("ne-words.tsv is not sorted: {entry:?} follows {prev:?}");
        }
        b.insert(&entry).expect("insert");
        prev = entry;
        n += 1;
    }
    b.finish().expect("finish fst");
    println!("cargo:warning=ne-words.fst: {n} entries");
}
