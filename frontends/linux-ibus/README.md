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

Or install the packages, which is what most people should do:
[`packaging/`](../../packaging/README.md).

Either way, IBus has to re-read its component directory before it will admit
the engine exists:

```sh
ibus restart; sleep 3; ibus list-engine | grep xlit
```

Give it that moment. `ibus restart && ibus engine xlit-ne` fails with
`IBUS-CRITICAL ... assertion 'IBUS_IS_BUS (bus)' failed` because the second
command connects while the daemon is still starting. And never `sudo ibus`:
IBus is per-user, root has no session bus, and it can only say `Can't connect
to IBus`.

Then pick **Nepali (PB-Ne)** in Settings > Keyboard > Input Sources.
On GNOME that step is not optional — the desktop owns the source list and
re-asserts it, so `ibus engine xlit-ne` on its own is undone by the next
restart. The command-line equivalent:

```sh
gsettings set org.gnome.desktop.input-sources sources "[('xkb', 'us'), ('ibus', 'xlit-ne')]"
```

**Super+Space** then switches between English and Nepali.

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

Installed from the `.deb` and run against a live `ibus-daemon` on Ubuntu 24.04
(GNOME, Wayland): the component file is read, `xlit-ne` appears in
`ibus list-engine`, and selecting it starts `/usr/libexec/ibus-engine-xlit`,
which stays up at about 4 MB resident.

What that does *not* cover is the keystroke path — the preedit, the candidate
window, the commit — because driving those means typing into a focused window,
which no test here can do. The engine behind them is the same code the CLI
exercises, but the D-Bus surface in [`src/ibus.rs`](src/ibus.rs) is hand-written
and a wrong signature draws nothing rather than failing loudly. If something
looks wrong while typing, watch it happen:

```sh
journalctl --user -f | grep xlit
```
