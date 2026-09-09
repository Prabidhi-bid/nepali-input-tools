# Linux packages

One description of where files go — the top-level `Makefile` — and three
wrappers around it. A package that disagreed with `make install` about a path
would be a bug nobody notices until an upgrade leaves a stale engine behind, so
none of these formats gets to have its own opinion.

## What gets built

Three packages, because the two frontends pull in different desktop stacks and
nobody running GNOME should have to install Fcitx5 to type Nepali:

| Package | Contents | Depends on |
|---------|----------|-----------|
| `xlit` / `xlit-common` | `xlit`, `xlit-daemon`, the systemd user unit, docs | libc |
| `xlit-ibus` | `ibus-engine-xlit` + the IBus component XML | the above, `ibus` |
| `xlit-fcitx5` | `xlit.so` addon + its two `.conf` files | the above, `fcitx5` |

The base package is called `xlit-common` on Debian and `xlit` on Fedora and
Arch, following each distribution's habit.

## Debian / Ubuntu

```bash
packaging/deb/build.sh
```

```bash
sudo apt install ./packaging/build/*.deb
```

Builds with `dpkg-deb` alone — no `debhelper`, no `dpkg-dev` source package —
because this is a small project and a `debian/` directory is a lot of
machinery to keep correct for three lists of files. `--no-build` reuses what is
already in `target/release`.

`xlit-fcitx5` is skipped when the Fcitx5 development files are missing; the
script says so rather than failing.

### Signing (Ubuntu 25.10 and later)

`sudo apt install ./*.deb` installs an unsigned package fine. Double-clicking
one does not: since 25.10 the desktop installer rejects it with *"the package
does not contain a publisher's name"*, because nothing in an unsigned archive
says who built it.

There is no way to sign as Ubuntu — Canonical's key signs Canonical's archive
and nobody else has it. What is possible is signing as *us*, which satisfies
`debsig-verify` on any machine that has been given our public key:

```bash
sudo apt install debsigs debsig-verify
```

```bash
packaging/deb/sign.sh --policy 'prdpspkt@gmail.com'
```

(without `sudo`, drop `--policy`: the packages are still signed, but this
machine has no policy to check them against). `build.sh` signs as it builds
when `XLIT_SIGN_KEY` is set to the same key.

A signature from a key the user has never seen still is not a *publisher* to
the desktop installer. For that, packages need to come from an apt repository
whose key the user has added — a PPA is the least work — and the terminal
install above remains the answer for everyone else.

## Ubuntu PPA

The answer to both problems a loose `.deb` has: no publisher the desktop
installer recognises, and `apt install ./xlit-ibus.deb` failing on
`Depends: xlit-common` because that name exists in no archive. A repository
fixes both — Launchpad signs it, and apt resolves the dependency from it.

```bash
sudo apt install devscripts dput debhelper cargo
```

```bash
packaging/ppa/build.sh ppa:prdpspkt/xlit noble plucky resolute
```

That builds one signed *source* package per series into `packaging/build/ppa/`
and prints the `dput` line; pass `--upload` to send them. Uploads are public
and version numbers can never be reused, so the script will not upload unless
asked. Users then get:

```bash
sudo add-apt-repository ppa:prdpspkt/xlit && sudo apt install xlit-ibus
```

`debian/` drives the same `make install-*` targets as everything else here.
Two things it has to do that a local build does not: build offline, since
Launchpad builders have no network — hence `cargo vendor` into the source
package and a source replacement written into `CARGO_HOME` — and version each
upload as `0.1.1~<series>1`, which sorts before `0.1.1` so one release can
target several series at once.

## What cannot be bundled

The Rust binaries are already static apart from glibc and libgcc, so
`xlit-common` is close to version-independent on its own. The frontends are
not, and no amount of bundling changes that:

- `xlit-fcitx5` is a Fcitx5 *addon*, loaded into the Fcitx5 process and linked
  against `Fcitx5::Core`. It must match the Fcitx5 on the machine; shipping our
  own copy would mean shipping a second input method daemon that the desktop
  never talks to.
- `xlit-ibus` speaks D-Bus to the session's IBus and is registered by an XML
  file IBus reads from a system directory. A sandboxed or relocated copy is not
  visible to the desktop's input framework at all.

This is why there is no Flatpak or AppImage here: an input method is not an
application the user launches, it is a component the desktop loads. What makes
one package cover many distribution versions is building against the *oldest*
glibc still supported — which is what the per-series PPA builds do, each
against its own series.

## Fedora / RHEL / openSUSE

```bash
make dist
```

```bash
rpmbuild -ba packaging/rpm/xlit.spec --define "_sourcedir $PWD/packaging/build"
```

## Arch

```bash
make dist && cp packaging/build/xlit-0.1.0.tar.gz packaging/arch/
```

```bash
cd packaging/arch && makepkg -si
```

## By hand, without a package

```bash
make && sudo make install
```

`sudo make uninstall` reverses it. `PREFIX=/usr/local` is the sensible choice
for a hand build so a distribution package cannot collide with it.

## After installing

IBus caches the engines it knows about, so a newly installed one appears only
after the daemon restarts:

```bash
ibus restart; sleep 3; ibus list-engine | grep xlit
```

The `sleep` is not decoration. `ibus restart` returns as soon as it has asked
the daemon to go away, and anything chained onto it with `&&` runs while the
daemon is still coming back:

```text
IBUS-CRITICAL **: ibus_bus_set_global_engine: assertion 'IBUS_IS_BUS (bus)' failed
Set global engine failed.
```

That is the race, not a broken install. Also: never `sudo ibus`. IBus is
per-user and lives on your session bus, and root has no session bus — it can
only answer `Can't connect to IBus`. Installing the package needs root;
nothing after it does.

### Selecting it

On **GNOME**, the desktop owns the list of input sources and re-asserts it, so
`ibus engine xlit-ne` alone does not stick — the next restart puts you back on
the keyboard layout you had. Add it as a source instead:

*Settings → Keyboard → Input Sources → + → Nepali → "Nepali (PB-Ne)"*

or, equivalently:

```bash
gsettings set org.gnome.desktop.input-sources sources "[('xkb', 'us'), ('ibus', 'xlit-ne')]"
```

Then **Super+Space** switches between them. To remove it again, set that key
back to `[('xkb', 'us')]`.

On desktops that do not manage the list themselves, `ibus engine xlit-ne` is
enough. For Fcitx5 it is *System Settings → Input Method → + → Nepali*.

The daemon is optional on Linux: each frontend is one long-lived process and
links the engine directly, so there is nothing to share. Start it if you want
one dictionary in memory for several frontends at once:

```bash
systemctl --user enable --now xlit-daemon
```

## Tested how far

The Debian packages are built, installed and run on Ubuntu 24.04 (GNOME,
Wayland): `apt install` of both, IBus lists `xlit-ne` from the packaged
component file, selecting it starts `/usr/libexec/ibus-engine-xlit`, `xlit` and
`xlit-daemon` work from `/usr/bin`, and the shipped unit starts under
`systemctl --user` and serves clients over its socket. Resident memory is about
4 MB for each of the daemon and the engine.

The RPM spec and the PKGBUILD are written against the same Makefile targets but
have **not** been built — there is no `rpmbuild` or `makepkg` on the machine
they were written on. Expect to fix something on the first real build, most
likely a missing `BuildRequires` or a path that Fedora spells differently.
