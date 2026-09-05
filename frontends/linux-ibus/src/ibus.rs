//! The wire types IBus expects, built by hand.
//!
//! IBus passes its objects — text, attribute lists, lookup tables — over D-Bus
//! as `IBusSerializable` variants rather than as plain types. Each one is a
//! struct whose first two fields are the type name and an (always empty here)
//! attachment dictionary, followed by that type's own fields. The C library
//! generates these; there is no Rust binding, so this module writes them
//! directly.
//!
//! Getting a signature wrong is not a compile error — it is an engine that
//! connects, runs, and silently shows nothing. The signatures are spelled out
//! in the comments beside each builder so they can be checked against
//! `ibus/src/ibustext.c` and friends without guessing.

use zvariant::{Array, Dict, OwnedValue, Signature, StructureBuilder, Value};

/// `a{sv}` — the attachment dictionary every serialisable carries. Always empty
/// for us; IBus uses it for things we do not set.
fn attachments<'a>() -> Dict<'a, 'a> {
    Dict::new(
        Signature::try_from("s").expect("valid"),
        Signature::try_from("v").expect("valid"),
    )
}

/// Wrap a value so it lands in a `v` (variant) field.
///
/// `StructureBuilder::add_field` takes each field's signature from the value
/// itself, so handing it a `Value` that already holds a struct produces that
/// struct's signature, not `v`. IBus declares several of its fields as variants,
/// and a mismatch here is invisible — the engine runs and draws nothing — so
/// the nesting is explicit everywhere it is needed.
fn variant(v: Value<'static>) -> Value<'static> {
    Value::Value(Box::new(v))
}

fn header(name: &str) -> StructureBuilder<'_> {
    StructureBuilder::new()
        .add_field(name.to_string())
        .append_field(Value::from(attachments()))
}

/// `IBusAttribute` — `(sa{sv}uuuu)`: type, value, start index, end index.
///
/// Type 1 is underline, and value 1 is a single underline. That is the whole
/// vocabulary we need: the preedit is drawn underlined, the way an unconverted
/// word is shown in every input method.
fn attribute(kind: u32, value: u32, start: u32, end: u32) -> Value<'static> {
    header("IBusAttribute")
        .add_field(kind)
        .add_field(value)
        .add_field(start)
        .add_field(end)
        .build()
        .into()
}

/// `IBusAttrList` — `(sa{sv}av)`.
fn attr_list(attrs: Vec<Value<'static>>) -> Value<'static> {
    let mut array = Array::new(Signature::try_from("v").expect("valid"));
    for a in attrs {
        array.append(variant(a)).expect("attr append");
    }
    header("IBusAttrList").append_field(Value::from(array)).build().into()
}

/// `IBusText` — `(sa{sv}sv)`: the string, then its attribute list.
pub fn text(s: &str) -> Value<'static> {
    // Character count, not byte count: IBus indexes attributes by character.
    let chars = s.chars().count() as u32;
    let attrs = if chars == 0 {
        attr_list(Vec::new())
    } else {
        attr_list(vec![attribute(1, 1, 0, chars)])
    };
    header("IBusText")
        .add_field(s.to_string())
        .append_field(variant(attrs))
        .build()
        .into()
}

/// Text with no underline — for candidates in the lookup table, which the
/// candidate window styles itself.
pub fn plain_text(s: &str) -> Value<'static> {
    header("IBusText")
        .add_field(s.to_string())
        .append_field(variant(attr_list(Vec::new())))
        .build()
        .into()
}

/// `IBusLookupTable` — `(sa{sv}uubbiavav)`: page size, cursor position, cursor
/// visible, round, orientation, candidates, labels.
///
/// Orientation 1 is vertical, matching the candidate window on the Windows
/// side. Labels are left empty so IBus numbers the candidates 1-9 itself.
pub fn lookup_table(candidates: &[String], cursor: u32, page_size: u32) -> Value<'static> {
    let mut cands = Array::new(Signature::try_from("v").expect("valid"));
    for c in candidates {
        cands.append(variant(plain_text(c))).expect("candidate append");
    }
    let labels = Array::new(Signature::try_from("v").expect("valid"));

    header("IBusLookupTable")
        .add_field(page_size)
        .add_field(cursor)
        .add_field(true) // cursor visible
        .add_field(false) // do not wrap around at the ends
        .add_field(1i32) // vertical
        .append_field(Value::from(cands))
        .append_field(Value::from(labels))
        .build()
        .into()
}

/// Convert to the owned form the signal signatures want.
pub fn owned(v: Value<'static>) -> OwnedValue {
    OwnedValue::try_from(v).expect("serialisable value")
}

// ---------------------------------------------------------------------------
// Key symbols and modifier bits
// ---------------------------------------------------------------------------

/// The X keysyms we act on. IBus passes X11 keyvals straight through.
pub mod key {
    /// The digit and letter rows, as keyvals. IBus passes X11 keysyms, which for
    /// ASCII are the character's own code point.
    pub const ZERO: u32 = 0x30;
    pub const ONE: u32 = 0x31;
    pub const NINE: u32 = 0x39;
    /// The numeric keypad's digits, which type numbers just as the top row does.
    pub const KP_ZERO: u32 = 0xffb0;
    pub const KP_NINE: u32 = 0xffb9;
    pub const LOWER_A: u32 = 0x61;
    pub const LOWER_Z: u32 = 0x7a;
    pub const UPPER_A: u32 = 0x41;
    pub const UPPER_Z: u32 = 0x5a;

    pub const BACKSPACE: u32 = 0xff08;
    pub const RETURN: u32 = 0xff0d;
    pub const KP_ENTER: u32 = 0xff8d;
    pub const ESCAPE: u32 = 0xff1b;
    pub const SPACE: u32 = 0x020;
    pub const UP: u32 = 0xff52;
    pub const DOWN: u32 = 0xff54;
    pub const PAGE_UP: u32 = 0xff55;
    pub const PAGE_DOWN: u32 = 0xff56;
}

/// Modifier bits in the `state` word of a key event.
pub mod modifier {
    pub const CONTROL: u32 = 1 << 2;
    pub const ALT: u32 = 1 << 3;
    /// Set on key *release*. We act on press only, but must still claim the
    /// release of a key we claimed the press of.
    pub const RELEASE: u32 = 1 << 30;
}
