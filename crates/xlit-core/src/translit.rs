//! Deterministic rule engine: Latin buffer -> native script.
//!
//! Algorithm (works for Brahmic scripts like Devanagari):
//!   * Greedy longest-match over the schema key set.
//!   * A consonant is written in its inherent-vowel form and marked "pending".
//!   * A following vowel replaces the inherent vowel with its matra
//!     (empty matra for the inherent vowel itself).
//!   * A following consonant triggers a virama between the two (conjunct).
//!   * Anything else flushes the pending state and is emitted verbatim.

use crate::schema::Schema;
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Vowel,
    Consonant,
    Sign,
    Plain,
}

struct Entry {
    kind: Kind,
    independent: String,
    matra: String,
    out: String,
}

pub struct RuleEngine {
    map: HashMap<String, Entry>,
    max_key: usize,
    virama: String,
    name: String,
}

impl RuleEngine {
    pub fn new(schema: Schema) -> Self {
        let mut map: HashMap<String, Entry> = HashMap::new();

        for (k, v) in &schema.vowels {
            map.insert(
                k.clone(),
                Entry {
                    kind: Kind::Vowel,
                    independent: v.independent.clone(),
                    matra: v.matra.clone(),
                    out: String::new(),
                },
            );
        }
        for (k, v) in &schema.consonants {
            map.insert(k.clone(), plain_entry(Kind::Consonant, v));
        }
        for (k, v) in &schema.signs {
            map.insert(k.clone(), plain_entry(Kind::Sign, v));
        }
        for (k, v) in &schema.digits {
            map.insert(k.clone(), plain_entry(Kind::Plain, v));
        }

        let max_key = map.keys().map(|k| k.chars().count()).max().unwrap_or(1);

        RuleEngine {
            map,
            max_key,
            virama: schema.meta.virama.clone(),
            name: schema.meta.name.clone(),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The primary transliteration plus any orthographic variants worth
    /// offering as distinct candidates:
    ///
    /// * **nasal conjuncts** — `ं` before a stop rewritten to the homorganic
    ///   nasal + virama (`बंध` → `बन्ध`, `रवींद्र` → `रवीन्द्र`), the spelling
    ///   Nepali prefers over the Hindi-style anusvara.
    /// * **de-gemination** — the *same* consonant either side of a virama
    ///   collapsed to one (`हेल्लो` → `हेलो`), how Nepali writes most English
    ///   loanwords.
    ///
    /// Primary comes first; callers score the rest below it and let the
    /// dictionary layer decide which spelling is a real word. Conjuncts of
    /// *different* consonants (क्ष, स्त्र, प्र, …) are never touched.
    pub fn transliterate_variants(&self, input: &str) -> Vec<String> {
        let primary = self.transliterate(input);
        let mut out = vec![primary.clone()];
        for cand in [
            self.nepali_nasal_conjuncts(&primary),
            self.collapse_geminates(&primary),
        ] {
            if !cand.is_empty() && cand != primary && !out.contains(&cand) {
                out.push(cand);
            }
        }
        out
    }

    /// Rewrite every `X <virama> X` (identical consonant on both sides) to a
    /// single `X`. Leaves true conjuncts and everything else as-is.
    fn collapse_geminates(&self, s: &str) -> String {
        let mut vir = self.virama.chars();
        let (Some(virama), None) = (vir.next(), vir.next()) else {
            return s.to_string(); // multi-scalar virama: not a case we target
        };
        let chars: Vec<char> = s.chars().collect();
        let mut out = String::with_capacity(s.len());
        let mut i = 0;
        while i < chars.len() {
            if i + 2 < chars.len() && chars[i + 1] == virama && chars[i] == chars[i + 2] {
                out.push(chars[i]); // keep one copy, drop virama + duplicate
                i += 3;
            } else {
                out.push(chars[i]);
                i += 1;
            }
        }
        out
    }

    /// Rewrite `<anusvara> <stop>` to `<homorganic nasal> <virama> <stop>` —
    /// the conjunct spelling Nepali orthography uses where Hindi often keeps a
    /// plain anusvara (`अंडा`→`अण्डा`, `बंद`→`बन्द`). A following consonant with
    /// no homorganic nasal (sibilants, र, ल, य, व, ह) keeps the anusvara.
    fn nepali_nasal_conjuncts(&self, s: &str) -> String {
        const ANUSVARA: char = '\u{0902}';
        let mut vir = self.virama.chars();
        let (Some(virama), None) = (vir.next(), vir.next()) else {
            return s.to_string();
        };
        let chars: Vec<char> = s.chars().collect();
        let mut out = String::with_capacity(s.len() + 4);
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == ANUSVARA {
                if let Some(nasal) = chars.get(i + 1).copied().and_then(homorganic_nasal) {
                    out.push(nasal);
                    out.push(virama);
                    i += 1;
                    continue;
                }
            }
            out.push(chars[i]);
            i += 1;
        }
        out
    }

    /// Convert a whole Latin string. Unknown characters (spaces, punctuation,
    /// digits) pass through untouched and reset syllable state.
    pub fn transliterate(&self, input: &str) -> String {
        let chars: Vec<char> = input.chars().collect();
        let n = chars.len();
        let mut out = String::with_capacity(n * 3);
        let mut i = 0;
        let mut pending_consonant = false;

        while i < n {
            let max_len = self.max_key.min(n - i);
            let mut matched: Option<(&Entry, usize)> = None;
            for len in (1..=max_len).rev() {
                let key: String = chars[i..i + len].iter().collect();
                if let Some(e) = self.map.get(&key) {
                    matched = Some((e, len));
                    break;
                }
            }

            match matched {
                None => {
                    out.push(chars[i]);
                    pending_consonant = false;
                    i += 1;
                }
                Some((e, len)) => {
                    match e.kind {
                        Kind::Consonant => {
                            if pending_consonant {
                                out.push_str(&self.virama);
                            }
                            out.push_str(&e.out);
                            pending_consonant = true;
                        }
                        Kind::Vowel => {
                            if pending_consonant {
                                out.push_str(&e.matra);
                                pending_consonant = false;
                            } else {
                                out.push_str(&e.independent);
                            }
                        }
                        Kind::Sign | Kind::Plain => {
                            out.push_str(&e.out);
                            pending_consonant = false;
                        }
                    }
                    i += len;
                }
            }
        }
        out
    }
}

fn plain_entry(kind: Kind, out: &str) -> Entry {
    Entry {
        kind,
        independent: String::new(),
        matra: String::new(),
        out: out.to_string(),
    }
}

/// The nasal that shares place of articulation with a Devanagari stop, i.e. the
/// one written as the first half of a `nasal + stop` conjunct. `None` for
/// consonants that take a plain anusvara instead (sibilants, semivowels, ह).
fn homorganic_nasal(stop: char) -> Option<char> {
    Some(match stop {
        'क' | 'ख' | 'ग' | 'घ' | 'ङ' => 'ङ',
        'च' | 'छ' | 'ज' | 'झ' | 'ञ' => 'ञ',
        'ट' | 'ठ' | 'ड' | 'ढ' | 'ण' => 'ण',
        'त' | 'थ' | 'द' | 'ध' | 'न' => 'न',
        'प' | 'फ' | 'ब' | 'भ' | 'म' => 'म',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eng() -> RuleEngine {
        RuleEngine::new(Schema::nepali())
    }

    #[test]
    fn common_words() {
        let e = eng();
        assert_eq!(e.transliterate("namaste"), "नमस्ते");
        assert_eq!(e.transliterate("nepaal"), "नेपाल");
        // rule engine is literal: short "i" -> ि. The dictionary layer is what
        // will map the real word "nepaali" -> नेपाली.
        assert_eq!(e.transliterate("nepaali"), "नेपालि");
        assert_eq!(e.transliterate("nepaalii"), "नेपाली");
        assert_eq!(e.transliterate("dhanyabaad"), "धन्यबाद");
        assert_eq!(e.transliterate("strii"), "स्त्री");
        assert_eq!(e.transliterate("kSha"), "क्ष");
    }

    #[test]
    fn geminate_variant_offered() {
        let e = eng();
        // doubled consonant → literal conjunct plus a de-geminated variant
        assert_eq!(e.transliterate_variants("hello"), vec!["हेल्लो", "हेलो"]);
        // no gemination → just the one form
        assert_eq!(e.transliterate_variants("namaste"), vec!["नमस्ते"]);
        // conjunct of *different* consonants is left intact (no extra variant)
        assert_eq!(e.transliterate_variants("strii"), vec!["स्त्री"]);
    }

    #[test]
    fn nasal_conjunct_variant_offered() {
        let e = eng();
        // anusvara before a stop → homorganic nasal conjunct (Nepali spelling)
        assert_eq!(e.transliterate_variants("baMdha"), vec!["बंध", "बन्ध"]);
        assert_eq!(e.transliterate_variants("raviiMdra"), vec!["रवींद्र", "रवीन्द्र"]);
        assert_eq!(e.transliterate_variants("aMDaa"), vec!["अंडा", "अण्डा"]);
        // anusvara before a sibilant stays put — no homorganic nasal exists
        assert_eq!(e.transliterate_variants("saMsaar"), vec!["संसार"]);
    }

    #[test]
    fn passthrough() {
        let e = eng();
        assert_eq!(e.transliterate("namaste sabai"), "नमस्ते सबै");
    }

    #[test]
    fn digits_convert() {
        let e = eng();
        assert_eq!(e.transliterate("2025"), "२०२५");
        // digits do not disturb syllable state around them
        assert_eq!(e.transliterate("web 2.0"), "वेब २.०");
        assert_eq!(e.transliterate("saal 2082"), "साल २०८२");
    }

    #[test]
    fn anusvara_and_signs() {
        let e = eng();
        // bare "ng" is a literal conjunct; the anusvara needs an explicit "M"
        assert_eq!(e.transliterate("ganga"), "गन्ग");
        assert_eq!(e.transliterate("kaaThamaaDauM"), "काठमाडौं");
        assert_eq!(e.transliterate("prashna|"), "प्रश्न।"); // danda sign
    }
}
