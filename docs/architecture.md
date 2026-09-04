# Architecture

## Principle

**One portable engine, thin per-OS frontends.** Google Input Tools failed on
Linux because the smart part was welded to the Windows text stack. We keep them
separate so the same engine binary serves every platform.

```
            ┌──────────────────────────────────────────┐
            │  xlit-core  (portable Rust, no OS deps)   │
            │                                          │
            │  input: "namaste"                        │
            │    │                                     │
            │    ▼                                     │
            │  1. RuleEngine    → नमस्ते  (always)      │
            │  2. Dictionary FST → re-rank / complete   │
            │  3. n-gram LM      → context (optional)   │
            │  4. ONNX fallback  → OOV only (optional)  │
            │  5. Learning store → personal boost       │
            │    │                                     │
            │    ▼                                     │
            │  ranked Vec<Candidate>                   │
            └──────────────────────────────────────────┘
                  ▲              ▲               ▲
          link as lib   OR   xlit-daemon (one process, mmap'd data)
                  │              │               │
        ┌─────────┴──┐   ┌───────┴──────┐  ┌─────┴────────┐
        │ Windows    │   │ Linux IBus   │  │ Linux Fcitx5 │
        │ TSF TIP    │   │ engine       │  │ addon        │
        └────────────┘   └──────────────┘  └──────────────┘
```

## Why this hits the low-RAM target

- **mmap everything.** The dictionary FST and the LM are memory-mapped read-only
  files. The OS pages in only what's touched and reclaims under pressure;
  multiple readers share the same physical pages.
- **One daemon, not one-per-app.** A TSF TIP is loaded into *every* process
  (Chrome, Word, Explorer…). If it held the data directly, each app would pay
  for it. The daemon holds the data once; frontends are ~2 MB clients speaking a
  small IPC protocol.
- **Neural model is opt-in and lazy.** Off by default. Loaded only on first OOV
  miss, unloaded after idle. Steady state never pays for it.
- **Rust, `opt-level="z"`, `panic="abort"`, `strip`.** Small static binaries,
  no runtime, minimal allocator pressure.

Expected resident set for the daemon: **~25–40 MB** with rule + dictionary +
learning; **+~10 MB** if the LM is enabled; **+80–150 MB** transiently if the
neural fallback fires.

## Engine internals (`xlit-core`)

- `Schema` — data-driven, one TOML per language (`schemas/ne.toml` = Nepali).
  Consonants, vowels (independent + matra), signs, digits, virama. Adding a
  language = adding a file.
- `RuleEngine` — greedy longest-match; consonant → inherent-vowel form + "pending"
  flag; vowel → matra; consonant-after-consonant → auto virama (conjuncts fall
  out for free). Non-schema chars pass through and reset state.
- `Ranker` trait — the plug-in seam. `rank(input, cands) -> cands`. Dictionary,
  LM, learning, and model layers are each a `Ranker`, run in order.
- `Candidate { text, source, score }`, `Source { Rule, Dictionary, Learned,
  Model, Raw }`.

## Dictionary layer (next)

- Build: word list + corpus frequencies → `fst::Map<word, packed(freq, flags)>`
  (BurntSushi `fst` crate). One file per language, mmap'd.
- Use: exact match boosts the matching candidate; prefix search yields
  completions; entries the rule engine can't reach (irregular spellings) are
  added as candidates.
- Data sources for Nepali: Leipzig Corpora (nep_news / nep_wikipedia) for
  frequencies, the Nepali National Corpus, `nepali-spellcheck` / Hunspell
  `ne_NP` word lists, FLORES / NLLB parallel data for romanization pairs.

## Learning store (next)

- SQLite via `rusqlite` (bundled). Table `picks(input, chosen, count, last_used)`.
- On commit, bump the row; `Ranker` adds a score bonus scaled by recency/count.
- Fully local. Never leaves the machine.

## Daemon + IPC (next)

- `xlit-daemon`: owns the `Engine`, opens the mmap'd data once.
- Transport: Unix domain socket (Linux) / named pipe (Windows).
- Protocol: length-prefixed JSON, request/response.
  - `{"op":"candidates","text":"namaste"}` → `{"candidates":[...]}`
  - `{"op":"commit","input":"namaste","chosen":"नमस्ते"}` → `{"ok":true}`
- Frontends hold no state beyond the current composition buffer.

## Frontends

| OS | Framework | Crate / language | Notes |
|----|-----------|------------------|-------|
| Linux | **IBus** | Rust (`ibus` via C FFI) or Python prototype | GNOME + most distros; easiest, do first |
| Linux | **Fcitx5** | C++ addon | KDE + power users |
| Windows | **TSF** (Text Services Framework) | Rust + `windows` crate, or fork Weasel | loads into every process; hardest, do last |
| macOS | InputMethodKit | later | |

Each frontend: capture keystrokes → maintain composition buffer → ask engine for
candidates → draw candidate window → on selection, send text to the app + tell
the engine to `commit` for learning.

## Milestones

1. **M1 — engine core** *(done)*: rule engine, Nepali schema, CLI, tests.
2. **M2 — dictionary + learning**: FST layer, SQLite store, accuracy harness
   (top-1 / top-5 / CER against a held-out word list).
3. **M3 — daemon**: `xlit-daemon` + IPC, CLI switches to client mode.
4. **M4 — Linux IBus**: end-to-end typing in real apps on Linux.
5. **M5 — Fcitx5**.
6. **M6 — Windows TSF**.
7. **M7 — ONNX OOV fallback**: `training/` pipeline (Dakshina `ne` +
   Aksharantar `nep` → small char transformer → int8 ONNX), lazy-loaded `Ranker`.
8. **M8 — packaging**: signed installers, per-distro packages, language packs.

## Non-goals (for now)

- Handwriting / voice input.
- A settings GUI (config file first).
- Languages beyond the first until the pipeline is proven end-to-end.
