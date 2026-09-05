#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Build the dictionary seed from the word database already in the repository.

    python tools/mkseed.py

Reads crates/xlit-dict/data/ne-words.sqlite3 — the same lexicon tools/mkwords.py
uses for the Latin-keyed list — and writes crates/xlit-dict/seed/ne.tsv, which
build.rs compiles into the FST behind `DictRanker::builtin()`.

Why this exists: the two dictionary layers ask different questions and, until
now, had wildly different vocabularies. `WordList` is keyed on the Latin that
was typed and has had all 8,597 words for a while. `DictRanker` is keyed on
Devanagari — it is what confirms that the rule engine's output is a real word,
corrects a one-character slip, and offers completions — and it had a
hand-written 263. That is why `dudh` had to be added by hand to be recognised at
all: the word was in the database, and the layer that validates spellings had
never seen it.

There are no frequencies in the database, so the second column is an honest
proxy rather than a count:

  * root entries outrank the inflected forms generated from them, because a
    typist reaches for the root more often than for any one of its forms;
  * shorter words outrank longer ones slightly, the one place where a real
    frequency list and word length agree;
  * hand-curated entries (seed/curated-ne.tsv) keep their own hand-set values,
    which are higher than anything here.

The proxy only feeds `freq_bonus`, a small additive term that decides which
three fuzzy hits are kept when there are more than three. It does not decide
where a candidate lands: that is distance from the literal reading, then
alphabetical order.
"""
import argparse
import os
import re
import sqlite3
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DB = os.path.join(ROOT, "crates/xlit-dict/data/ne-words.sqlite3")
SEED = os.path.join(ROOT, "crates/xlit-dict/seed/ne.tsv")
CURATED = os.path.join(ROOT, "crates/xlit-dict/seed/curated-ne.tsv")

ROOT_WEIGHT = 400
FORM_WEIGHT = 150

# Anusvara directly before a stop (the five varga rows क–म) is the Hindi
# spelling of a conjunct that Nepali writes out: अंग्रेजी for अङ्ग्रेजी, जंगली
# for जङ्गली. The rule engine already converts the one into the other — that is
# what `transliterate_variants`' nasal-conjunct variant is for — so seeding the
# Hindi form would undo it. Confirming a spelling as a real word is what keeps
# it in the top slot, and these are exactly the spellings the input method
# exists to correct. Thirteen words in the database, all of them conjuncts.
HINDI_ANUSVARA = re.compile("\u0902[\u0915-\u092e]")


def proxy_frequency(form, word):
    """A stand-in for a frequency the database does not have. See the module doc."""
    base = ROOT_WEIGHT if form == "root" else FORM_WEIGHT
    # Length costs a little, floored so a long word never falls to nothing.
    return max(base - 4 * max(0, len(word) - 4), base // 4)


def load_curated():
    if not os.path.exists(CURATED):
        return {}
    out = {}
    with open(CURATED, encoding="utf-8") as fh:
        for line in fh:
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            cols = line.split("\t")
            if len(cols) >= 2 and cols[1].strip().isdigit():
                out[cols[0].strip()] = int(cols[1])
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--db", default=DB)
    ap.add_argument("--out", default=SEED)
    args = ap.parse_args()

    db = sqlite3.connect(args.db)
    words = {}
    skipped = 0
    for word, form in db.execute("SELECT word, form FROM word"):
        word = (word or "").strip()
        if not word or " " in word:
            continue
        if HINDI_ANUSVARA.search(word):
            skipped += 1
            continue
        freq = proxy_frequency(form, word)
        words[word] = max(words.get(word, 0), freq)
    from_db = len(words)

    curated = load_curated()
    for word, freq in curated.items():
        words[word] = max(words.get(word, 0), freq)

    with open(args.out, "w", encoding="utf-8") as fh:
        fh.write(HEADER.format(
            total=len(words), from_db=from_db, curated=len(curated), skipped=skipped
        ))
        for word, freq in sorted(words.items(), key=lambda kv: (-kv[1], kv[0])):
            fh.write(f"{word}\t{freq}\n")
    print(f"wrote {args.out}: {len(words)} words", file=sys.stderr)


HEADER = """\
# Nepali dictionary seed: word <TAB> frequency proxy. GENERATED — do not edit.
#
# Regenerate with: python tools/mkseed.py
#
# {from_db} words from data/ne-words.sqlite3 + {curated} hand-curated = {total}.
# The curated entries live in seed/curated-ne.tsv, which is the file to edit.
#
# {skipped} database words were skipped: they spell a conjunct with the Hindi
# anusvara (अंग्रेजी rather than अङ्ग्रेजी), which is the spelling this input
# method converts away from — seeding it would confirm it and keep it on top.
#
# The database has no frequencies, so the second column is a proxy: roots above
# generated forms, shorter above longer. It feeds `freq_bonus`, which chooses
# which fuzzy hits to keep when there are more than three — it does not decide
# where a candidate lands. That is distance from the literal reading, then
# alphabetical.
"""

if __name__ == "__main__":
    main()
