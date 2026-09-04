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
  `transliterate_variants()` returns the literal form plus, when applicable,
  orthographic variants — a de-geminated copy (`X ्X` → `X`, so `hello` → हेलो)
  and a nasal-conjunct copy (`ं` + stop → homorganic nasal + virama, so
  `रवींद्र` → रवीन्द्र, the Nepali spelling vs the Hindi anusvara). Variants are
  offered alongside the literal; the dictionary layer picks the winner.
- `Ranker` trait — the plug-in seam. `rank(input, cands) -> cands`. Dictionary,
  LM, learning, and model layers are each a `Ranker`, run in order.
- `Candidate { text, source, score }`, `Source { Rule, Dictionary, Learned,
  Model, Raw }`.

## Dictionary layer (`xlit-dict`) — done

- Structure: `fst::Map<Devanagari word → u64 freq>` (BurntSushi `fst`).
  `DictRanker::builtin()` builds it in memory from the compiled-in seed
  (`seed/ne.tsv`); `DictRanker::open(path)` memory-maps a large prebuilt `.fst`
  (`src/bin/build.rs` compiles a TSV → `.fst`).
- Per input, three passes run against **every** literal candidate the rule engine
  produced — the primary transliteration and any variant (e.g. the de-geminated
  loanword form), so a variant can only win with dictionary backing:
  1. **exact** — `map.get(word)` → score 300 + log-freq bonus, `Source::Dictionary`.
  2. **fuzzy** — `Levenshtein(word, 1)` → 200 + freq bonus (+15 when the hit is
     the same character length, i.e. a substitution: vowel length `ि`/`ी`,
     anusvara, sibilant). This is what turns `नेपालि` → `नेपाली`.
  3. **prefix** — `Str(word).starts_with()` → up to 3 completions at ~90 + bonus/2.
- All results funnel through `xlit_core::merge_candidate` (dedupe, keep max score).
- The seed (`seed/ne.tsv`) has three groups: core vocabulary, common English
  loanwords (हेलो, बस, स्कुल, डाक्टर, …), and proper nouns — places, countries,
  people (पोखरा, वीरगञ्ज, रवीन्द्रनाथ, …). Combined with the engine's
  de-geminated and nasal-conjunct variants, this is what keeps `hello` off
  हेल्लो and names/loanwords in Nepali (अङ्ग्रेजी, not अंग्रेजी) rather than
  Hindi orthography.
- Data sources to grow the seed: Leipzig Corpora (`nep_news` / `nep_wikipedia`)
  for frequencies, Nepali National Corpus, Hunspell `ne_NP` word list.

## Learning store (`xlit-learn`) — done

- File-backed: a pretty-printed JSON array of `{input, chosen, count, last_used}`,
  single writer, atomic replace (`write tmp` + `rename`). No C dependency, no
  runtime deps beyond `serde_json`. Swap for SQLite behind the same API only if
  the data ever gets large.
- On commit, bump `count` + `last_used`; the `Ranker` adds `400 + count*12
  (+40 if used in the last day)` to the matching candidate as `Source::Learned`.
- Fully local. Never leaves the machine. CLI writes `./xlit-learn.json`.

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
2. **M2 — dictionary + learning** *(done)*: `xlit-dict` FST layer,
   `xlit-learn` JSON store, wired into the CLI (numbers commit picks).
   *Still open:* corpus-derived seed list, accuracy harness (top-1 / top-5 / CER
   against a held-out word list).
3. **M3 — daemon**: `xlit-daemon` + IPC, CLI switches to client mode.
4. **M4 — Linux IBus**: end-to-end typing in real apps on Linux.
5. **M5 — Fcitx5**.
6. **M6 — Windows TSF** (`frontends/windows-tsf`, `xlit-tsf.dll`), staged:
   - **M6.1** *(done)*: registrable COM DLL — `regsvr32` writes the CLSID keys,
     TSF profile (`Input by Prabidhi.bid`, LANGID `0x0461`), `EnableLanguageProfile`,
     and the keyboard + `TIPCAP_*` categories (so it shows in the Win10/11
     switcher). Activatable; no key handling yet. Install/uninstall:
     `frontends/windows-tsf/installer/*.ps1`.
   - **M6.2**: `ITfKeyEventSink` + inline TSF composition — buffer ASCII, replace
     with the engine's top candidate on a break key.
   - **M6.3**: candidate window (layered popup) + number-key select + `xlit-learn`.
   - **M6.4**: language-bar icon, x86 build, Chrome/Electron/UWP fixes, toggle key.
   - **M6.5** *(initial)*: `installer/install.ps1` + `uninstall.ps1` — copy the
     DLL to Program Files, `regsvr32`, Apps & features entry; no toolchain.
     TODO: a signed MSI (deferred no-impersonate CAs, per-user
     `EnableLanguageProfile` via Active Setup), x86/ARM64 payloads.
7. **M7 — ONNX OOV fallback**: `training/` pipeline (Dakshina `ne` +
   Aksharantar `nep` → small char transformer → int8 ONNX), lazy-loaded `Ranker`.
8. **M8 — packaging**: signed installers, per-distro packages, language packs.

## Non-goals (for now)

- Handwriting / voice input.
- A settings GUI (config file first).
- Languages beyond the first until the pipeline is proven end-to-end.
