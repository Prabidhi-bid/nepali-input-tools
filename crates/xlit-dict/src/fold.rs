// Folding a Latin key to the shape a typist would produce.
//
// `include!`d by `build.rs`, so everything here is plain comments and free
// functions: no module docs, no imports.
//
// Shared verbatim by `build.rs` (which folds every key in the word list) and
// by [`WordList`](crate::WordList) (which folds what was typed). Both sides
// *must* fold identically, so this file is `include!`d by the build script
// rather than duplicated — the two copies would drift on the first change and
// the failure would be silent: lookups that simply stop matching.
//
// This is canonicalisation, not transliteration. It does not have to be right
// about Nepali; it has to map the spellings of one word onto each other. The
// classes it collapses are the ones romanised Nepali genuinely does not
// distinguish:
//
// - **vowel length** (`aa`/`a`, `ii`/`i`, `uu`/`u`), which almost nobody marks.
// - **b and v and w** — बुवा is written `buwaa` or `buvaa` by the same person
//   on different days.
// - **the sibilants** स श ष, one Latin `s` in practice.
// - **aspirate doubling**: `chh` for छ, which people type as `ch`.
// - **nasal marks**, typed as `n` when they are typed at all.
// - **case**, which encodes retroflexes in the schema (`T` = ट) and which no
//   ordinary typist uses.
//
// What is deliberately *not* folded here is the inherent vowel — सरकार is keyed
// `sarakaar` and typed `sarkar` — nor a trailing `a`. Those are not classes a
// reader cannot distinguish; they are a choice the typist makes, and folding
// them symmetrically was a bug: `kahaa~` and `khaanaa` both collapse to `khan`
// once you may delete an `a` between two consonants, so typing `khana` offered
// कहाँ ahead of खाना. Enumerating the choice on the stored side instead —
// [`key_variants`] — keeps the query fold safe: it only ever merges spellings
// that sound the same.
//
// Collapsing even these classes means different words land on the same folded
// key: काम and कम both fold to `kam`. That is not a defect. A folded hit is
// offered as a candidate and scored below an exact key match; which one leads
// is then decided by the engine, and by how far each sits from the literal
// reading of what was typed.
/// Fold a Latin key (or a Latin input) to its canonical shape.
pub fn fold_key(s: &str) -> String {
    // Nasal marks and visarga first, while case still distinguishes them from
    // the consonants `m` and `h`.
    let mut out = String::with_capacity(s.len());
    let bytes: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        match c {
            'M' | '~' => {
                out.push('n');
                i += 1;
            }
            'H' => {
                i += 1;
            }
            '.' if i + 1 < bytes.len() && (bytes[i + 1] == 'n' || bytes[i + 1] == 'm') => {
                out.push('n');
                i += 2;
            }
            _ => {
                out.push(c.to_ascii_lowercase());
                i += 1;
            }
        }
    }

    // Digraphs, longest first. `jn` before `n`-anything so ज्ञ lands on the `gy`
    // people type; `chh` before `ch` so छ does not become `chh` -> `ch` -> `c`.
    for (from, to) in [
        ("j~n", "gy"),
        ("jn", "gy"),
        ("chh", "ch"),
        ("shh", "s"),
        ("sh", "s"),
        ("ss", "s"),
        ("aa", "a"),
        ("ii", "i"),
        ("ee", "i"),
        ("uu", "u"),
        ("oo", "u"),
        ("v", "b"),
        ("w", "b"),
        ("f", "ph"),
        ("z", "j"),
    ] {
        if out.contains(from) {
            out = out.replace(from, to);
        }
    }

    out
}

/// Every folded key a word should answer to, for the build side to store.
///
/// Beyond [`fold_key`] this enumerates the two things a typist chooses rather
/// than mishears:
///
/// - **the inherent vowel inside the word.** The generated keys spell every one
///   of them (`sarakaar`, `tarakaarii`); a typist drops some and keeps others
///   (`sarkar`, but `tarkari` — not `srkr`), and which ones is a matter of
///   syllable stress nobody applies consistently. So every combination is
///   stored, and the typist's spelling finds the word whichever they dropped.
/// - **a trailing `a`.** शब्द is keyed `shabd`; `sabda` is just as likely.
///
/// This multiplies the size of the compiled set — which is the price of not
/// doing the same deletions to the query, where they would merge words that are
/// nothing like each other: `kahaa~` and `khaanaa` both reduce to `khan`, and
/// typing `khana` then offered कहाँ ahead of खाना.
#[allow(dead_code)] // build.rs half of this file; unused inside the crate.
pub fn key_variants(s: &str) -> Vec<String> {
    let base = fold_key(s);
    if base.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(8);
    for stem in inherent_vowel_choices(&base) {
        if stem.len() < 2 {
            continue;
        }
        let toggled = match stem.strip_suffix('a') {
            Some(shorter) if shorter.len() >= 2 => shorter.to_string(),
            _ => format!("{stem}a"),
        };
        for v in [stem, toggled] {
            if !out.contains(&v) {
                out.push(v);
            }
        }
    }
    out
}

/// The key with every combination of its inherent vowels kept or dropped.
///
/// An inherent vowel is an `a` between two consonants, never the first
/// character — there it is a vowel someone actually typed. `sarakar` has three
/// and yields eight spellings, `sarkar` among them; the ones nobody would type
/// (`srkr`) cost an entry each and are simply never looked up.
///
/// Capped at [`MAX_INHERENT`] positions: beyond that only the all-kept and
/// all-dropped forms are produced, because 2^n entries for a long compound is a
/// lot of set for a spelling nobody uses.
#[allow(dead_code)] // build.rs half of this file; unused inside the crate.
fn inherent_vowel_choices(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let positions: Vec<usize> = (1..chars.len().saturating_sub(1))
        .filter(|&i| {
            chars[i] == 'a' && is_consonant(chars[i - 1]) && is_consonant(chars[i + 1])
        })
        .collect();

    if positions.is_empty() {
        return vec![s.to_string()];
    }
    if positions.len() > MAX_INHERENT {
        let dropped: String = chars
            .iter()
            .enumerate()
            .filter(|(i, _)| !positions.contains(i))
            .map(|(_, &c)| c)
            .collect();
        return vec![s.to_string(), dropped];
    }

    let mut out = Vec::with_capacity(1 << positions.len());
    for mask in 0..(1u32 << positions.len()) {
        let keep = |i: &usize| match positions.iter().position(|p| p == i) {
            Some(bit) => mask & (1 << bit) == 0,
            None => true,
        };
        out.push(
            chars
                .iter()
                .enumerate()
                .filter(|(i, _)| keep(i))
                .map(|(_, &c)| c)
                .collect::<String>(),
        );
    }
    out
}

/// Enumerating 2^n spellings is fine for the three or four inherent vowels a
/// real word has and pointless past that.
#[allow(dead_code)] // build.rs half of this file; unused inside the crate.
const MAX_INHERENT: usize = 4;

#[allow(dead_code)] // build.rs half of this file; unused inside the crate.
fn is_consonant(c: char) -> bool {
    c.is_ascii_alphabetic() && !matches!(c, 'a' | 'e' | 'i' | 'o' | 'u')
}
