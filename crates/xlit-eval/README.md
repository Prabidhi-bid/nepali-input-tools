# xlit-eval — accuracy harness

The engine is a stack of heuristics that trade inputs against each other. Every
change to the rule engine or to the dictionary passes fixes some words and
breaks others, and without a number attached, "this fixes `hello`" and "this
breaks twenty words nobody tried" look the same from the outside.

```bash
cargo run -p xlit-eval
```

```bash
cargo run -p xlit-eval -- --tag loan --failures 20
```

| Flag | Meaning |
|------|---------|
| `--set PATH` | evaluate your own TSV instead of the built-in set |
| `--tag TAG` | only rows carrying `TAG` |
| `--failures N` | list the N worst cases (default 10, `0` for none) |
| `--min-top1 F` | exit 1 if top-1 accuracy is below `F` |
| `--max-cer F` | exit 1 if the character error rate is above `F` |

## What it measures

- **top-1** — the intended word is the first candidate. The one that matters:
  it is what pressing space gives you.
- **top-5** — it is somewhere in the visible list, so a number key reaches it.
- **MRR** — mean reciprocal rank. Notices movement inside the list that the two
  accuracies cannot.
- **CER** — character error rate of the top-1 answer against the intended word,
  summed over the corpus rather than averaged per word. A first candidate one
  matra out (`नेपालि` for `नेपाली`) is a different failure from a first
  candidate that is a different word, and only CER tells them apart.

Runs have no learning store attached, so the number is a property of the engine
and not of whichever machine ran it.

## The set

`data/ne-eval.tsv` — `latin <TAB> devanagari <TAB> tags`, 148 rows, hand-written
and **held out**: not generated from the engine and not taken from
`xlit-dict`'s seed. A set produced by round-tripping the transliterator would
only prove that the transliterator agrees with itself.

Two tags describe how a row is typed, and the split is the point of the whole
exercise:

- `strict` — spelled in the schema's ITRANS conventions (`Thuulo`, `bhaaShaa`).
  The rule engine alone should get these right; anything less is a bug in the
  rule engine rather than a gap in the dictionary.
- `casual` — how people really type: no capitals for retroflexes, no doubled
  vowels for length (`thulo`, `bhasa`, `kathmandu`). Only the dictionary can
  recover the spelling from these.

The rest — `core`, `inflect`, `loan`, `proper` — are vocabulary groups.

## Baseline

As of the commit that added this crate:

```
            cases    top-1    top-5     MRR     CER
overall       148    62.8%    70.3%   0.664   0.177
casual        131    58.0%    66.4%   0.621   0.202
core           88    75.0%    83.0%   0.788   0.101
inflect        22    72.7%    81.8%   0.773   0.060
loan           16    25.0%    31.2%   0.281   0.553
proper         22    31.8%    36.4%   0.341   0.281
strict         17   100.0%   100.0%   1.000   0.000
```

Read that as a map of where the work is, not as a verdict:

- **The rule engine is exact** on everything spelled its way. Nothing to fix
  there; the `strict` row is a tripwire for regressions.
- **Loanwords and proper nouns are the hole**, at 25% and 32%. `computer` comes
  out चोम्पुतेर, `kathmandu` कथ्मन्दु — the letter-by-letter reading of English
  orthography, which no amount of ranking can repair because the right answer is
  never generated in the first place. These need dictionary entries and, for
  names, a Latin key to reach them by; the seed's own notes point at the same
  fix.
- **Core vocabulary at 75%** mostly fails by one character — `साठी` for `साथी`,
  `फुल` for `फूल` — where the fuzzy pass does find the word but ranks it second.
  That is a ranking problem rather than a coverage one, and the cheaper half of
  what is left.

`tests/accuracy.rs` holds floors a little under these numbers so a change cannot
quietly cost more than it buys. They are a ratchet: when the numbers go up, the
floors go up in the same commit.
