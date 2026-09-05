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
| **User learning** (`xlit-learn`) | personalization, JSON-backed | grows slowly | < 1 MB |

Target steady footprint: **~25–40 MB resident**, shared across every app via a
single daemon (frontends are ~2 MB clients).

There is deliberately **no neural model**. Rule + dictionary + learning covers
the language, and a lazy ONNX layer would have cost 80–150 MB resident whenever
it fired — against the whole point of the design. Unknown words fall back to the
rule engine's literal transliteration, which is always right about the sounds
even when it cannot know the spelling.

## Status

- [x] Workspace + rule engine (`xlit-core`) — Nepali/Devanagari schema
- [x] Dev REPL (`xlit-cli`)
- [x] Dictionary FST layer (`xlit-dict`) — exact / fuzzy / prefix, mmap-capable
- [x] User-learning store (`xlit-learn`) — JSON-backed, boosts past picks
- [x] Daemon + IPC (`xlit-daemon`, `xlit-ipc`) — Unix socket / named pipe,
      length-prefixed JSON; `xlit --client` is a client of it
      (see [crates/xlit-daemon/README.md](crates/xlit-daemon/README.md))
- [x] Linux IBus frontend ([frontends/linux-ibus](frontends/linux-ibus/README.md))
- [x] Linux Fcitx5 frontend ([frontends/linux-fcitx5](frontends/linux-fcitx5/README.md))
- [~] Windows TSF frontend (`xlit-tsf`) — types Nepali in real apps with a
      candidate window and an installer; ARM64 and the language-bar icon remain
      (see [frontends/windows-tsf/README.md](frontends/windows-tsf/README.md))
- [x] Accuracy harness (`xlit-eval`) — top-1 / top-5 / MRR / CER against a
      held-out word list ([crates/xlit-eval/README.md](crates/xlit-eval/README.md))
- [x] Latin-keyed loanwords and proper nouns — `computer`, `kathmandu`, spelled
      the way people type them (`xlit-dict/data/latin-keys-ne.tsv`)
- [ ] Corpus-derived dictionary seed — every word outside the lists still falls
      back to the literal reading
- [ ] Packaging: per-distro Linux packages, language packs

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

Or against the shared engine process, which is what the frontends talk to:

```bash
cargo run -p xlit-daemon
```

```bash
cargo run -p xlit-cli -- --client namaste duniya
```

```bash
cargo test
```

How well does it actually work? 81.1% of a held-out list comes out right on the
first candidate today, and 100% of the words spelled the schema's own way:

```bash
cargo run -p xlit-eval
```

## Layout

```
crates/
  xlit-core/     engine: rule pipeline, schema loader, Ranker trait, merge_candidate
    schemas/     one .toml per language (ne.toml = Nepali)
  xlit-dict/     dictionary Ranker: FST of word->freq; exact/fuzzy/prefix
    seed/        ne.tsv — small built-in word list
    data/        ne-words.tsv (generated) + latin-keys-ne.tsv (hand-written)
    src/bin/     build.rs — TSV -> .fst compiler
  xlit-learn/    learning Ranker: remembers (input -> chosen), JSON-backed
  xlit-ipc/      protocol + transport between frontends and the daemon
  xlit-daemon/   one engine process per user; frontends are thin clients
  xlit-cli/      dev REPL / one-shot tester (--client talks to the daemon)
  xlit-eval/     accuracy harness: top-1 / top-5 / MRR / CER
    data/        ne-eval.tsv — held-out word list
  xlit-config/   word editor (Windows)
  xlit-install/  self-contained Windows installer
frontends/
  windows-tsf/   Windows text service (xlit_tsf.dll) + installer
  linux-ibus/    IBus engine (GNOME and most distros)
  linux-fcitx5/  Fcitx5 addon (KDE)
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
