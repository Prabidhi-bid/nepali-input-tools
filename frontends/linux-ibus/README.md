# `xlit-ibus` — the Linux frontend

An [IBus](https://github.com/ibus/ibus) engine. Separate from
`frontends/windows-tsf`, which is untouched: the two share the portable crates
(`xlit-core`, `xlit-dict`, `xlit-learn`) and nothing else, because their
plumbing has nothing in common — TSF is an in-process COM server loaded into
every application, IBus is one program talking D-Bus.

Everything that decides *what you get when you type* is shared, so the two
behave identically: the same 8,597-word dictionary, the same ranking, the same
verb forms, the same case and vowel folding.

## Build and install

```sh
./frontends/linux-ibus/install.sh           # system-wide, needs sudo
./frontends/linux-ibus/install.sh --user    # into ~/.local, no sudo
```

Then pick **Nepali (transliteration)** in Settings > Keyboard > Input Sources,
or `ibus engine xlit-ne`.

## How it behaves

Deliberately the same as the Windows text service:

| Key | Effect |
|---|---|
| letters | extend the word; the preedit shows the **raw Latin**, underlined |
| Space | commit the highlighted candidate, then a space |
| Enter | commit with no trailing space |
| 1-9 | pick that candidate and commit |
| Up / Down | move the highlight |
| Backspace | drop a letter |
| Esc | give up on the conversion, leave exactly what was typed |
| Ctrl+Space | passthrough on/off |

The preedit shows Latin rather than a converted guess on purpose: a guess that
changes with every keystroke is text you never typed moving under your cursor.
Conversion happens once, at commit.

Only a *deliberate* pick is learned. Accepting the top candidate with Space is
not a choice, and recording it would let the first answer for an input — right
or wrong — outrank the dictionary for ever after.

## Notes on the implementation

- **No Rust binding for IBus exists**, so [`src/ibus.rs`](src/ibus.rs) writes the
  `IBusText`, `IBusAttrList` and `IBusLookupTable` D-Bus structures by hand. Each
  builder carries its signature in a comment: a wrong signature is not a compile
  error, it is an engine that connects, runs, and silently draws nothing.
- **Variant fields need explicit nesting.** `StructureBuilder::add_field` takes a
  field's signature from the value itself, so a `Value` holding a struct produces
  that struct's signature rather than `v`. See `variant()` in `src/ibus.rs`.
- **Learned picks** live in `$XDG_DATA_HOME/xlit/xlit-learn.json`
  (`~/.local/share/xlit/` by default), the same file format as on Windows.
- The crate compiles to a stub `main` on non-Unix targets so that
  `cargo check --workspace` still works on a Windows development machine.

## Status

Type-checked against `x86_64-unknown-linux-gnu`, **not yet run**: it was written
on a Windows machine, so no part of it has faced a live `ibus-daemon`. Expect the
first session to need adjustment — most likely in the lookup-table signature or
the key handling, which are the parts a type-checker cannot vouch for.
