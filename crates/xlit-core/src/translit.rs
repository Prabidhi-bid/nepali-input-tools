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
    fn passthrough() {
        let e = eng();
        assert_eq!(e.transliterate("namaste sabai"), "नमस्ते सबै");
        assert_eq!(e.transliterate("web 2.0"), "वेब 2.0");
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
