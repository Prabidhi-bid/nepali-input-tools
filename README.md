# xlit — a cross-platform transliteration input method

A from-scratch, open replacement for Google Input Tools: type phonetically in
Latin letters, get **Nepali** in Devanagari (`namaste` → `नमस्ते`), system-wide,
on **Windows and Linux** (macOS later).

## Design goal: low RAM, high accuracy

One portable engine, thin per-OS frontends. The engine is a layered pipeline;
each layer is optional and cheap:

| Layer | Job | Disk | RAM (mmap) |
|-------|-----|------|-----------|
| **Rule engine** | Latin → script, deterministic, OOV fallback | < 100 KB | ~0 |
| **Dictionary (FST)** | correct spelling, rank, complete common words | 2–8 MB | 3–10 MB |
| **n-gram LM** *(optional)* | sentence-context re-ranking | 10–30 MB | ~10 MB |
| **Neural ONNX** *(optional, lazy)* | out-of-vocabulary names / rare words only | 30–60 MB | 80–150 MB when loaded |
| **User learning (SQLite)** | personalization | grows slowly | ~1 MB |

Target steady footprint: **~25–40 MB resident**, shared across every app via a
single daemon (frontends are ~2 MB clients). The neural model is off by default.

## Status

- [x] Workspace + rule engine (`xlit-core`) — Nepali/Devanagari schema
- [x] Dev REPL (`xlit-cli`)
- [ ] Dictionary FST layer (`fst` crate, mmap)
- [ ] User-learning store (SQLite)
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
  xlit-core/     engine: rule pipeline, schema loader, Ranker trait
    schemas/     one .toml per language (ne.toml = Nepali)
  xlit-cli/      dev REPL / one-shot tester
frontends/       per-OS input-method plugins (added incrementally)
training/        (later) data prep + train + ONNX export for the OOV model
docs/            architecture & design notes
```

## License

MIT OR Apache-2.0
