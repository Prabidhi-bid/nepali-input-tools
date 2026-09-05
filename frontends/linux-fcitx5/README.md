# `xlit-fcitx5` — the KDE / Fcitx5 frontend

Fcitx5 is KDE Plasma's native input method framework. This is a separate crate
from `linux-ibus` and `windows-tsf`; all three share the portable crates
(`xlit-core`, `xlit-dict`, `xlit-learn`) and nothing else.

**You may not need this.** KDE runs IBus perfectly well — set the input method
module to `ibus` and install `frontends/linux-ibus`. Use this one if you already
run Fcitx5, or want the framework KDE ships with.

## Build and install

```sh
./frontends/linux-fcitx5/install.sh
```

Needs the Fcitx5 development packages and a C++ compiler:

| | |
|---|---|
| Debian / Ubuntu | `sudo apt install fcitx5 libfcitx5core-dev libfcitx5utils-dev cmake g++` |
| Fedora | `sudo dnf install fcitx5-devel cmake gcc-c++` |
| Arch | `sudo pacman -S fcitx5 cmake gcc` |

Then add it in System Settings > Input Method: **+**, search Nepali, pick
**Nepali (PB-Ne)**.

## Why there are two halves

Fcitx5 loads input methods as C++ shared libraries implementing
`fcitx::InputMethodEngine`. There is no Rust API, and — unlike IBus, where an
engine is just a program on the bus — no way in over D-Bus. So:

- **[`src/state.rs`](src/state.rs)** is the editing model as a plain Rust type:
  keys in, an `Action` out, no framework and no I/O. Every decision about what a
  key means lives here.
- **[`src/lib.rs`](src/lib.rs)** wraps it in a small C ABI and builds a `cdylib`.
- **[`cpp/xlit-engine.cpp`](cpp/xlit-engine.cpp)** is the addon Fcitx5 actually
  loads, translating its callbacks into calls on that ABI. It does no thinking.

That split is worth the extra file. The state machine is the part that had been
written three times against three different APIs, which is how the same bug got
fixed twice. Here it is written once and **unit-tested on any machine** —
including the Windows one it was written on:

```sh
cargo test -p xlit-fcitx5
```

Eleven tests cover the behaviour that matters: the preedit is the raw Latin,
Space commits with a trailing space and Enter without one, Esc gives back
exactly what was typed, the number row picks a candidate while composing and
types a digit otherwise, losing focus drops the word rather than committing it,
and punctuation commits the word but is left for the application to insert.

## Status

| | |
|---|---|
| `src/state.rs`, `src/lib.rs` | **tested** — 11 passing tests, run on Windows |
| `cpp/xlit-engine.cpp`, CMake, `.conf` files | **unverified** — written without Fcitx5 headers to compile against |

The C++ half has never been built: it was written on a Windows machine with no
Fcitx5 development packages. Expect the first `install.sh` to need a fix or two
in the addon or the CMake glue. The Rust half underneath it is exercised.
