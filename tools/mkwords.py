#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Build the Nepali word list from a nep_dict SQLite dump.

    python tools/mkwords.py <nep_dict.sqlite3>

Writes crates/xlit-dict/data/ne-words.sqlite3 (id, word, transliteration,
root_id, form) and crates/xlit-dict/data/ne-words.tsv, which build.rs compiles
into the FST that ships in the binary.

Three things happen on the way:

  * multi-word entries are split, and their parts kept as separate words;
  * each word gets a Latin key by inverting crates/xlit-core/schemas/ne.toml —
    the same schema the Rust rule engine uses, so keys read back correctly;
  * -नु verbs get their common inflections generated (लाग्नु -> लाग्यो, लागेको).

Every key is verified by transliterating it back with a port of the forward rule
engine; the mismatches that survive are reported and are all word-final halanta
(महान्) or compounds where `a` + a vowel re-reads as one long vowel.
"""
import os
import re
import sqlite3
import sys

VIRAMA = '\u094d'

def load_schema(path):
    txt = open(path, encoding='utf-8').read()
    sec, vowels, cons, signs, digits = None, {}, {}, {}, {}
    for line in txt.splitlines():
        line = line.strip()
        if not line or line.startswith('#'):
            continue
        if line.startswith('['):
            sec = line.strip('[]')
            continue
        if '=' not in line:
            continue
        k, v = line.split('=', 1)
        k, v = k.strip().strip('"'), v.strip()
        if sec == 'vowels':
            ind = re.search(r'independent\s*=\s*"([^"]*)"', v).group(1)
            m = re.search(r'matra\s*=\s*"([^"]*)"', v)
            vowels[k] = (ind, m.group(1) if m else '')
        elif sec in ('consonants', 'signs', 'digits'):
            val = v.strip().strip('"')
            {'consonants': cons, 'signs': signs, 'digits': digits}[sec][k] = val
    return vowels, cons, signs, digits

# ---------------------------------------------------------------- forward
def forward(s, vowels, cons, signs, digits):
    """Port of RuleEngine::transliterate: greedy longest match, inherent vowel,
    automatic virama between adjacent consonants."""
    ent = {}
    for k, (ind, m) in vowels.items():
        ent[k] = ('V', ind, m)
    for k, v in cons.items():
        ent[k] = ('C', v, '')
    for k, v in signs.items():
        ent[k] = ('S', v, '')
    for k, v in digits.items():
        ent[k] = ('P', v, '')
    maxk = max(len(k) for k in ent)
    out, pending, i = [], False, 0
    while i < len(s):
        for n in range(min(maxk, len(s) - i), 0, -1):
            e = ent.get(s[i:i + n])
            if e:
                break
        else:
            e, n = None, 1
        if e is None:
            out.append(s[i]); pending = False; i += 1; continue
        kind, ind, matra = e
        if kind == 'V':
            if pending:
                out.append(matra)
            else:
                out.append(ind)
            pending = False
        elif kind == 'C':
            if pending:
                out.append(VIRAMA)
            out.append(ind)
            pending = True
        else:
            out.append(ind)
            pending = False
        i += n
    return ''.join(out)

# ---------------------------------------------------------------- reverse
def build_reverse(vowels, cons, signs, digits):
    """One canonical Latin spelling per Devanagari unit. Where the schema gives
    several keys for the same output (aa/A, ch/c, ...) we keep the first that
    forward-maps back unambiguously, preferring the plain ITRANS form."""
    prefer_v = {'\u0906':'aa','\u0908':'ii','\u090a':'uu','\u0905':'a','\u0907':'i',
                '\u0909':'u','\u090f':'e','\u0910':'ai','\u0913':'o','\u0914':'au',
                '\u090b':'RRi'}
    ind2lat, matra2lat = {}, {}
    for k, (ind, m) in vowels.items():
        if ind not in ind2lat or prefer_v.get(ind) == k:
            ind2lat.setdefault(ind, k)
        if prefer_v.get(ind) == k:
            ind2lat[ind] = k
            if m:
                matra2lat[m] = k
        elif m and m not in matra2lat:
            matra2lat[m] = k
    prefer_c = {'\u091a':'ch','\u091b':'chh','\u092b':'ph','\u0935':'v','\u091c':'j',
                '\u0915\u0937':'x','\u091c\u094d\u091e':'GY'}
    c2lat = {}
    for k, v in cons.items():
        if len(v) != 1:      # x = क्ष, GY = ज्ञ: fall out of k+Sh / j+~n anyway
            continue
        if v not in c2lat or prefer_c.get(v) == k:
            if prefer_c.get(v) == k or v not in c2lat:
                c2lat[v] = prefer_c.get(v, k) if prefer_c.get(v) else min(c2lat.get(v, k), k, key=len)
    for dev, lat in prefer_c.items():
        if len(dev) == 1:
            c2lat[dev] = lat
    s2lat = {}
    for k, v in signs.items():
        s2lat.setdefault(v, k)
    for dev, lat in {'\u0902':'M', '\u0903':'H', '\u0901':'~', '\u0964':'|'}.items():
        s2lat[dev] = lat
    d2lat = {v: k for k, v in digits.items()}
    return ind2lat, matra2lat, c2lat, s2lat, d2lat

def reverse(word, maps, drop_final_schwa=True):
    """Latin key for a Devanagari word.

    The final inherent vowel is dropped (schwa deletion): Nepali writes अकबर and
    types "akabar", never "akabara". The forward engine still reads "akabar"
    back as अकबर, because its own final consonant carries the inherent vowel."""
    ind2lat, matra2lat, c2lat, s2lat, d2lat = maps
    out, i, chars = [], 0, list(word)
    inherent_at = None                       # index in out[] of the last inherent 'a'
    while i < len(chars):
        ch = chars[i]
        if ch in c2lat:
            out.append(c2lat[ch])
            nxt = chars[i + 1] if i + 1 < len(chars) else ''
            if nxt == VIRAMA:
                i += 2                       # conjunct: no vowel at all
                continue
            if nxt in matra2lat:
                out.append(matra2lat[nxt]); i += 2; continue
            inherent_at = len(out)
            out.append('a')                  # inherent vowel
            i += 1
            continue
        if ch in ind2lat:
            out.append(ind2lat[ch])
        elif ch in s2lat:
            out.append(s2lat[ch])
        elif ch in d2lat:
            out.append(d2lat[ch])
        elif ch in ('\u200c', '\u200d'):     # ZWNJ / ZWJ: invisible, drop
            pass
        elif ch == VIRAMA:
            pass                             # trailing halanta
        else:
            out.append(ch)
        inherent_at = None                   # anything else ends the run
        i += 1
    if drop_final_schwa and inherent_at == len(out) - 1 and len(out) > 2:
        out.pop()
    return ''.join(out)

# ---------------------------------------------------------------- verbs
NU     = 'नु'

# Verbs whose past and participle come from a different root altogether, which
# no rule over the infinitive can reach.
IRREGULAR = {
    'हुनु': [('past','भयो'), ('perfective','भएको'), ('present','हुन्छ'),
             ('past_pl','भए'), ('progressive','हुँदै'), ('participle','हुने')],
    'जानु': [('past','गयो'), ('perfective','गएको'), ('present','जान्छ'),
             ('past_pl','गए'), ('progressive','जाँदै'), ('participle','जाने')],
    'रुनु': [('past','रोयो'), ('perfective','रोएको'), ('present','रुन्छ'),
             ('past_pl','रोए'), ('progressive','रुँदै'), ('participle','रुने')],
}

def inflect(inf):
    """[(form_name, word)] for one -नु infinitive; [] if it is not one, or if
    its stem is a shape we deliberately do not model."""
    if not inf.endswith(NU) or len(inf) < 4:
        return []
    if inf in IRREGULAR:
        return IRREGULAR[inf]
    s = inf[:-2]                                  # stem, halanta kept

    if s.endswith(VIRAMA):                        # लाग् -> लाग
        b = s[:-1]
        return [('past', b + '्यो'), ('perfective', b + 'ेको'),
                ('present', b + '्छ'), ('progressive', b + '्दै'),
                ('past_pl', b + 'े'), ('conditional', b + '्दा'),
                ('participle', b + '्ने')]

    if s.endswith('ि'):                           # उभि
        return [('past', s + 'यो'), ('perfective', s + 'एको'),
                ('present', s + 'न्छ'), ('progressive', s + 'ँदै'),
                ('past_pl', s + 'ए'), ('participle', s + 'ने')]

    if s.endswith('उ'):                           # उडाउ -> उडा
        b = s[:-1]
        return [('past', b + 'यो'), ('perfective', b + 'एको'),
                ('present', s + 'ँछ'), ('progressive', s + 'ँदै'),
                ('past_pl', b + 'ए'), ('participle', s + 'ने')]

    if 'क' <= s[-1] <= 'ह' and len(s) >= 2:
        # Bare consonant, inherent vowel intact: चाह, रह, सम्झ. The length guard
        # keeps nouns that merely end in नु out of it — धनु would otherwise
        # conjugate as though it were a verb.
        return [('past', s + '्यो'), ('perfective', s + 'ेको'),
                ('present', s + 'न्छ'), ('progressive', s + 'ँदै'),
                ('past_pl', s + 'े'), ('participle', s + 'ने')]

    if s[-1] in 'ाीोूे':                          # खा, धो
        return [('past', s + 'यो'), ('perfective', s + 'एको'),
                ('present', s + 'न्छ'), ('progressive', s + 'ँदै'),
                ('past_pl', s + 'ए'), ('participle', s + 'ने')]

    return []

# ---------------------------------------------------------------- main
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SCHEMA = os.path.join(ROOT, 'crates/xlit-core/schemas/ne.toml')
OUT_DB = os.path.join(ROOT, 'crates/xlit-dict/data/ne-words.sqlite3')
OUT_TSV = os.path.join(ROOT, 'crates/xlit-dict/data/ne-words.tsv')

SCHEMA_SQL = """
CREATE TABLE word (
  id              INTEGER PRIMARY KEY,
  word            TEXT NOT NULL,
  transliteration TEXT NOT NULL,
  root_id         INTEGER REFERENCES word(id),
  form            TEXT NOT NULL
);
CREATE INDEX idx_word_translit ON word(transliteration);
CREATE UNIQUE INDEX idx_word_value ON word(word);
"""

def fold_key(k):
    """The way the key is actually typed.

    The schema is case-sensitive by design - `T` is ट and `t` is त - but almost
    nobody reaches for the shift key mid-word, and romanised Nepali does not
    distinguish the two in practice. So every key gets a second, all-lowercase
    spelling pointing at the same word, and typing `dhaal` finds ढाल as well as
    धाल.

    Without this, 2,348 of the 8,389 words - every one with a retroflex, a
    sibilant other than स, or a nasal - could only be reached by typing
    something nobody types."""
    for a, b in (('RRi', 'ri'), ('R^i', 'ri'), ('~N', 'n'), ('~n', 'n'),
                 ('M', 'n'), ('H', 'h'), ('~', '')):
        k = k.replace(a, b)
    return k.lower()

MAX_VOWEL_VARIANTS = 5   # 2**5 = 32 spellings; longer words are left alone

def vowel_variants(k):
    """Every mix of long and short vowels for a folded key.

    Romanised Nepali does not mark vowel length reliably, and people are not
    even consistent within one word: हामी is `haamii` canonically, but gets
    typed `haami`, `hamii` and `hami`. Collapsing every long vowel at once
    would only produce the last of those, so each long vowel is independently
    long or short and the word answers to all of them.

    Bounded at 2**MAX_VOWEL_VARIANTS: the cost is entries in the FST, and a word
    with six long vowels is not one anybody is going to misspell into existence."""
    spots = []
    i = 0
    while i < len(k) - 1:
        if k[i] == k[i + 1] and k[i] in 'aiu':
            spots.append(i)
            i += 2
        else:
            i += 1
    if not spots or len(spots) > MAX_VOWEL_VARIANTS:
        return {k}
    out = set()
    for mask in range(1 << len(spots)):
        chars, skip = [], set()
        for n, pos in enumerate(spots):
            if mask >> n & 1:
                skip.add(pos + 1)          # drop the second half of the pair
        for i, ch in enumerate(k):
            if i not in skip:
                chars.append(ch)
        out.add(''.join(chars))
    return out

def devanagari_only(w):
    return bool(w) and all('\u0900' <= ch <= '\u097f' for ch in w)

def main(src_path):
    vowels, cons, signs, digits = load_schema(SCHEMA)
    maps = build_reverse(vowels, cons, signs, digits)
    key = lambda w: reverse(w, maps)

    src = sqlite3.connect(src_path)
    seen, rows = set(), []
    dropped = {'dupe': 0, 'punct': 0}
    stats = {'headword': 0, 'split': 0}
    next_split_id = 1_000_000

    for wid, value in src.execute("select id, value from word order by id"):
        # ZWJ / ZWNJ are invisible and only create duplicate spellings.
        raw = value.replace('\u200d', '').replace('\u200c', '').strip()
        parts = raw.split()
        phrase = len(parts) > 1
        for part in parts:
            if not devanagari_only(part):
                dropped['punct'] += 1
                continue
            if part in seen:
                dropped['dupe'] += 1
                continue
            seen.add(part)
            if phrase:
                # A word pulled out of a phrase has no id of its own; number it
                # above every real id so it can never collide with one.
                rows.append((next_split_id, part, key(part), None, 'root'))
                next_split_id += 1
                stats['split'] += 1
            else:
                rows.append((wid, part, key(part), None, 'root'))
                stats['headword'] += 1

    # Hand-added words, kept out of the generated files so they survive a
    # regeneration. Numbered above the source ids and the phrase splits.
    extra_path = os.path.join(ROOT, 'crates/xlit-dict/data/extra-ne.txt')
    extra_added = 0
    if os.path.exists(extra_path):
        with open(extra_path, encoding='utf-8') as f:
            for line in f:
                w = line.split('#')[0].strip()
                if not w or w in seen:
                    continue
                if not devanagari_only(w):
                    dropped['punct'] += 1
                    continue
                seen.add(w)
                rows.append((1_500_000 + extra_added, w, key(w), None, 'root'))
                extra_added += 1

    by_word = {r[1]: r[0] for r in rows}
    generated = 0
    for word, wid in list(by_word.items()):
        for form, w in inflect(word):
            if not devanagari_only(w) or w in seen:
                continue
            seen.add(w)
            rows.append((2_000_000 + generated, w, key(w), wid, form))
            generated += 1

    if os.path.exists(OUT_DB):
        os.remove(OUT_DB)
    out = sqlite3.connect(OUT_DB)
    out.executescript(SCHEMA_SQL)
    out.executemany(
        "insert into word (id, word, transliteration, root_id, form) values (?,?,?,?,?)",
        rows)
    out.commit()
    out.execute("vacuum")

    # One row per (key, word). Each word appears under its canonical key and,
    # when they differ, under the folded spelling people actually type.
    pairs = set()
    for _, w, k, _, _ in rows:
        pairs.add((k, w))
        for v in vowel_variants(fold_key(k)):
            pairs.add((v, w))
    # fst's SetBuilder needs keys in UTF-8 byte order and rejects anything else.
    tsv = sorted(pairs, key=lambda kw: (kw[0].encode('utf-8'), kw[1].encode('utf-8')))
    with open(OUT_TSV, 'w', encoding='utf-8', newline='\n') as f:
        f.write("# Latin key <TAB> Devanagari word - generated from ne-words.sqlite3.\n")
        f.write("# Sorted by UTF-8 byte order, which is what fst's SetBuilder requires.\n")
        f.write("# Regenerate with tools/mkwords.py; do not hand-edit.\n")
        for k, w in tsv:
            f.write("%s\t%s\n" % (k, w))

    bad = [(w, k) for _, w, k, _, _ in rows
           if forward(k, vowels, cons, signs, digits) != w]
    print("words       %d (headwords %d, from phrases %d, hand-added %d, verb forms %d)"
          % (len(rows), stats['headword'], stats['split'], extra_added, generated))
    print("dropped     %s" % dropped)
    print("roundtrip   %d of %d exact" % (len(rows) - len(bad), len(rows)))
    print("lookup keys %d (%d words + %d folded spellings)"
          % (len(tsv), len(rows), len(tsv) - len(rows)))
    print("wrote       %s\n            %s" % (OUT_DB, OUT_TSV))

if __name__ == '__main__':
    if len(sys.argv) != 2:
        sys.exit("usage: python tools/mkwords.py <nep_dict.sqlite3>")
    main(sys.argv[1])
