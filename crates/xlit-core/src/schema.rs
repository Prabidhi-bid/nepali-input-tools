//! Data-driven transliteration schema (one file per language).

use serde::Deserialize;
use std::collections::HashMap;

/// The Nepali schema, compiled into the binary as the default.
pub const BUILTIN_NEPALI: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/schemas/ne.toml"));

#[derive(Debug, Clone, Deserialize)]
pub struct Schema {
    pub meta: Meta,
    #[serde(default)]
    pub vowels: HashMap<String, Vowel>,
    #[serde(default)]
    pub consonants: HashMap<String, String>,
    #[serde(default)]
    pub signs: HashMap<String, String>,
    #[serde(default)]
    pub digits: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Meta {
    pub name: String,
    pub script: String,
    #[serde(default = "default_inherent")]
    pub inherent_vowel: String,
    #[serde(default = "default_virama")]
    pub virama: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Vowel {
    pub independent: String,
    #[serde(default)]
    pub matra: String,
}

fn default_inherent() -> String {
    "a".to_string()
}

fn default_virama() -> String {
    "\u{094D}".to_string() // Devanagari sign virama
}

impl Schema {
    pub fn from_toml_str(s: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(s)
    }

    /// The compiled-in Nepali schema.
    pub fn nepali() -> Self {
        Self::from_toml_str(BUILTIN_NEPALI).expect("builtin nepali schema must parse")
    }
}
