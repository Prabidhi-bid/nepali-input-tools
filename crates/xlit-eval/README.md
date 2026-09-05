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

```
            cases    top-1    top-5     MRR     CER      (was, at introduction)
overall       148    81.1%    87.2%   0.840   0.064      62.8% / 70.3% / 0.177
casual        131    78.6%    85.5%   0.819   0.073      58.0% / 66.4% / 0.202
core           88    75.0%    83.0%   0.788   0.101      unchanged
inflect        22    72.7%    81.8%   0.773   0.060      unchanged
loan           16   100.0%   100.0%   1.000   0.000      25.0% / 31.2% / 0.553
proper         22   100.0%   100.0%   1.000   0.000      31.8% / 36.4% / 0.281
strict         17   100.0%   100.0%   1.000   0.000      unchanged
```

**Read the `loan` and `proper` rows carefully.** They went to 100% because
`xlit-dict/data/latin-keys-ne.tsv` now contains those words, keyed by the Latin
people actually type. That is coverage, and coverage is what a dictionary layer
*is* — but the rows no longer measure generalisation, because the set they are
scored against is inside the list. What they are now is a tripwire: if they fall,
the list has stopped being compiled in or stopped being consulted.

For a word that is *not* in the list, nothing has changed: `hospital` still
comes out होस्पितल and `sarangkot` सरन्ग्कोत. Growing the number means growing
the list.

The rest of the table is the honest part:

- **The rule engine is exact** on everything spelled its way. Nothing to fix
  there; the `strict` row is a regression tripwire.
- **Core vocabulary at 75%** mostly fails by one character — `साठी` for `साथी`,
  `फुल` for `फूल` — where the fuzzy pass does find the word but ranks it second.
  A ranking problem rather than a coverage one, and the cheapest thing left.
- **Inflected forms at 73%** are the same story one step along.

`tests/accuracy.rs` holds floors a little under these numbers so a change cannot
quietly cost more than it buys. They are a ratchet: when the numbers go up, the
floors go up in the same commit.
