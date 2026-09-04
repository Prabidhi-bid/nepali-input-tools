# xlit — a cross-platform transliteration input method

A from-scratch, open replacement for Google Input Tools: type phonetically in
Latin letters, get **Nepali** in Devanagari (`namaste` → `नमस्ते`), system-wide,
on **Windows and Linux** (macOS later).

## Design goal: low RAM, high accuracy

One portable engine, thin per-OS frontends. The engine is a layered pipeline;
each layer is optional and cheap:

| Layer | Job | Disk | RAM (mmap) |
|-------|-----|------|-----------|
| **Rule engine** (`xlit-core`) | Latin → script, deterministic, OOV fallback | < 100 KB | ~0 |
| **Dictionary FST** (`xlit-dict`) | correct spelling, rank, complete common words | 2–8 MB | 3–10 MB |
| **n-gram LM** *(optional)* | sentence-context re-ranking | 10–30 MB | ~10 MB |
| **Neural ONNX** *(optional, lazy)* | out-of-vocabulary names / rare words only | 30–60 MB | 80–150 MB when loaded |
| **User learning** (`xlit-learn`) | personalization, JSON-backed | grows slowly | < 1 MB |

Target steady footprint: **~25–40 MB resident**, shared across every app via a
single daemon (frontends are ~2 MB clients). The neural model is off by default.

## Status

- [x] Workspace + rule engine (`xlit-core`) — Nepali/Devanagari schema
- [x] Dev REPL (`xlit-cli`)
- [x] Dictionary FST layer (`xlit-dict`) — exact / fuzzy / prefix, mmap-capable
- [x] User-learning store (`xlit-learn`) — JSON-backed, boosts past picks
- [ ] Daemon + IPC (named pipe / Unix socket)
- [ ] Linux IBus frontend
- [ ] Linux Fcitx5 frontend
- [ ] Windows TSF frontend
- [ ] (later) ONNX OOV fallback + training pipeline in `training/`

See [docs/architecture.md](docs/architecture.md) for the full plan and milestones.

## Build & run

Install Rust (one-time):

```bash
winget install Rustlang.Rustup
```

Then:

```bash
cargo run -p xlit-cli -- namaste duniya
```

```bash
cargo test
```

## Layout

```
crates/
  xlit-core/     engine: rule pipeline, schema loader, Ranker trait, merge_candidate
    schemas/     one .toml per language (ne.toml = Nepali)
  xlit-dict/     dictionary Ranker: FST of word->freq; exact/fuzzy/prefix
    seed/        ne.tsv — small built-in word list
    src/bin/     build.rs — TSV -> .fst compiler
  xlit-learn/    learning Ranker: remembers (input -> chosen), JSON-backed
  xlit-cli/      dev REPL / one-shot tester
frontends/       per-OS input-method plugins (added incrementally)
training/        (later) data prep + train + ONNX export for the OOV model
docs/            architecture & design notes
```

## Try the layers

```bash
cargo run -p xlit-cli -- nepaali
```

`नेपालि` (rule) is corrected to `नेपाली` by the dictionary. In interactive mode,
type a word, then type the number of the candidate you want — it's remembered and
floated to the top next time.

## License

MIT OR Apache-2.0
