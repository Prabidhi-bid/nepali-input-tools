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
            │  4. Learning store → personal boost       │
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
- **No neural layer at all.** An ONNX out-of-vocabulary model was planned and
  has been dropped: it would have cost 80–150 MB resident whenever it fired,
  which is more than the rest of the system put together, to improve a case the
  rule engine already handles legibly. Unknown words get the literal
  transliteration — right about the sounds, occasionally wrong about the
  spelling — and the dictionary seed is the cheaper place to spend effort.
- **Rust, `opt-level="z"`, `panic="abort"`, `strip`.** Small static binaries,
  no runtime, minimal allocator pressure.

Expected resident set for the daemon: **~25–40 MB** with rule + dictionary +
learning; **+~10 MB** if the LM is enabled.

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
- **Latin-keyed list** (`WordList`, `data/ne-words.tsv` + `data/latin-keys-ne.tsv`).
  The Devanagari-keyed passes above can only re-rank what the rule engine
  produced, which is why they never fixed loanwords: `computer` reads as
  चोम्पुतेर, and कम्प्युटर is not one edit away from it — the right answer is
  never generated, so there is nothing to promote. The Latin-keyed list looks up
  what was *typed* instead. `ne-words.tsv` is generated by `tools/mkwords.py`,
  which derives each key by inverting the Devanagari spelling; that works for
  Nepali words and not at all for an English spelling, so `latin-keys-ne.tsv`
  carries ~390 hand-written pairs — loanwords, Nepali places, world places,
  common names — keyed the way people type them (`kathmandu`, not
  `kaaThamaaDauM`), with alternates where usage is split. An exact key match
  scores 320 as `Source::Confirmed`, which leads the list unless the literal
  reading is itself a word.

## Learning store (`xlit-learn`) — done

- File-backed: a pretty-printed JSON array of `{input, chosen, count, last_used}`,
  single writer, atomic replace (`write tmp` + `rename`). No C dependency, no
  runtime deps beyond `serde_json`. Swap for SQLite behind the same API only if
  the data ever gets large.
- On commit, bump `count` + `last_used`; the `Ranker` adds `400 + count*12
  (+40 if used in the last day)` to the matching candidate as `Source::Learned`.
- Fully local. Never leaves the machine. CLI writes `./xlit-learn.json`.

## Daemon + IPC (`xlit-daemon`, `xlit-ipc`) — done

- `xlit-daemon` owns the `Engine` and the learning store and opens the mmap'd
  data once; `xlit-ipc` is the protocol, the transport, and the `Client` both
  ends share. `xlit --client` is the first client; see
  [crates/xlit-daemon/README.md](../crates/xlit-daemon/README.md).
- Transport: Unix domain socket (`$XDG_RUNTIME_DIR/xlit/xlit.sock`, mode 0700)
  on Unix, named pipe (`\\.\pipe\xlit-daemon-$USERNAME`, creator's ACL) on
  Windows. `$XLIT_SOCKET` overrides both. `Stream` is `Read + Write` on both
  platforms, so the framing code is written once.
- Protocol: length-prefixed JSON (`u32` LE length, then body, 1 MiB cap),
  request/response, many pairs per connection so a frontend connects once.
  - `{"op":"candidates","text":"namaste"}` → `{"ok":true,"candidates":[...]}`
  - `{"op":"commit","input":"namaste","chosen":"नमस्ते"}` → `{"ok":true}`
  - also `transliterate`, `ping`, `shutdown`.
- Errors are `{"ok":false,"error":"..."}`, never a dropped connection: losing
  the daemon mid-word would cost the user their composition, so the daemon
  answers even when it cannot help. `source` is a string, not an enum, so a new
  ranking layer cannot break an older client.
- A stale socket (daemon killed, not stopped) is detected by *connecting* — the
  inode outlives the process, so existence proves nothing — and replaced. A
  second daemon on a live endpoint refuses to start rather than take half the
  connections.
- Frontends hold no state beyond the current composition buffer. The Linux ones
  still link the engine directly, which on Linux costs nothing: an IBus engine
  is already one process per session. Windows is where the daemon earns its
  keep.

## Accuracy harness (`xlit-eval`) — done

- `data/ne-eval.tsv`: 148 hand-written, held-out `latin → devanagari` pairs.
  Not generated from the engine and not drawn from the dictionary seed; a set
  round-tripped through the transliterator would only prove it agrees with
  itself. Tagged `strict` (spelled the schema's way — the rule engine alone
  should be exact) vs `casual` (how people really type — only the dictionary can
  recover it), plus `core` / `inflect` / `loan` / `proper`.
- Reports top-1, top-5, MRR and CER (character error rate of the top candidate,
  summed corpus-wide), overall and per tag, and lists the worst failures.
  `--min-top1` / `--max-cer` make it a gate; `tests/accuracy.rs` holds floors
  just under the current numbers as a ratchet.
- Baseline at introduction: **62.8% top-1 / 70.3% top-5 / CER 0.177** overall,
  with loanwords at 25% and proper nouns at 32% — the letter-by-letter reading of
  English orthography, never repairable by ranking because the right answer was
  not generated at all. The hand-written Latin-keyed list (above) was the answer
  to exactly that, and moved the overall figure to **81.1% / 87.2% / CER 0.064**.
  Loanwords and proper nouns now score 100%, but those rows are inside the list
  by construction: they measure coverage and act as a tripwire, not as evidence
  about words nobody has added — `hospital` still comes out होस्पितल. What is
  left on the honest side of the table is core vocabulary at 75% and inflected
  forms at 73%, which mostly miss by one character with the right word sitting
  at rank 2. That is a ranking problem, and the cheapest thing remaining.

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
   *Still open:* corpus-derived seed list — the accuracy harness (`xlit-eval`,
   below) now says what it would buy: loanwords and proper nouns are where the
   engine loses.
3. **M3 — daemon** *(done)*: `xlit-daemon` + `xlit-ipc`, `xlit --client`.
4. **M4 — Linux IBus** *(done)*: end-to-end typing in real apps on Linux.
5. **M5 — Fcitx5** *(done)*.
6. **M6 — Windows TSF** (`frontends/windows-tsf`, `xlit-tsf.dll`), staged:
   - **M6.1** *(done)*: registrable COM DLL — `regsvr32` writes the CLSID keys,
     the TSF profile via `ITfInputProcessorProfileMgr::RegisterProfile`
     (`Input by Prabidhi.bid`, LANGID `0x0461`, enabled-by-default), and the
     keyboard + `TIPCAP_*` categories (`IMMERSIVESUPPORT` / `SYSTRAYSUPPORT` for
     the modern switcher; **not** `COMLESS` — this is a classic COM server and
     declaring it hid the TIP from `TextInputHost`). Activatable; no key
     handling yet. Install: `frontends/windows-tsf/installer/`.
   - **M6.2** *(done)*: `ITfKeyEventSink` + inline TSF composition. The Latin
     buffer is re-converted and rewritten whole on every keystroke, so Backspace
     needs no Latin↔Devanagari mapping; the composition shows the converted top
     candidate, not the raw Latin. A break key commits and **re-inserts its own
     character** — a key claimed in `OnTestKeyDown` never reaches the app, so
     letting Space through is not an option. Number/arrow selection and
     `xlit-learn` commits work already; they are just invisible until M6.3.
     Failure modes matter here because the DLL is `panic="abort"` inside every
     text-input process: a refused composition reports the letter as unhandled
     (plain Latin, not a dead keyboard), and `OnCompositionTerminated` takes the
     session lock with `try_borrow_mut` so a re-entrant teardown cannot panic.
   - **M6.3** *(done)*: candidate window — an owner-drawn `WS_EX_NOACTIVATE`
     popup, not `ITfCandidateListUIElement` (the UI-less protocol leaves drawing
     to the application, and almost none do it). Rendered in *Nirmala UI*, the
     Windows Devanagari face; anchored with `ITfContextView::GetTextExt` and
     flipped above the line near a screen edge. Its paint state is owned by the
     window via `GWLP_USERDATA` rather than read back out of the session, so the
     window procedure can never re-enter a `RefCell` an edit already holds.
     Shown only when there are 2+ candidates *and* the control reported a caret
     position.
   - **M6.4**: language-bar icon, Chrome/Electron/UWP fixes. The Ctrl+Space
     passthrough toggle landed early, with M6.2.
   - **M6.5** *(initial)*: `installer/` — `xlit-tsf.iss` + `build-setup.ps1`
     produce a distributable Inno Setup `Setup.exe`; `crates/xlit-install`
     builds a self-contained `xlit-install.exe` with both DLLs embedded, which
     registers through the TSF APIs rather than `regsvr32`. Both cover **x64 + x86**
     (`--target x86_64-` / `i686-pc-windows-msvc`), lay them out as
     `{app}\xlit_tsf.dll` + `{app}\x86\xlit_tsf.dll`, deregister→register each
     with the matching-bitness `regsvr32`, and add the keyboard with
     `Set-WinUserLanguageList`; uninstall fully reverses (incl. HKLM key
     force-delete + HKCU CTF sweep). Release DLL is stripped, trace behind a
     cargo feature. TODO: native ARM64.
7. **M7 — packaging**: per-distro Linux packages, language packs. The daemon
   ships a systemd user unit (`crates/xlit-daemon/dist/`) already.

## Dropped

- **Neural OOV fallback.** Was M7: a `training/` pipeline (Dakshina `ne` +
  Aksharantar `nep` → small char transformer → int8 ONNX) behind a lazy
  `Ranker`. Cut for the RAM reason above; `training/` is deleted (see git
  history if it is ever wanted back).
- **Code signing.** No Authenticode certificate, so no signed `Setup.exe` and no
  signed MSI. Windows SmartScreen will warn on first run of the installer; that
  is the accepted cost.

## Non-goals (for now)

- Handwriting / voice input.
- A settings GUI (config file first).
- Languages beyond the first until the pipeline is proven end-to-end.
