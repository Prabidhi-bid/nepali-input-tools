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

IBus caches the engines it knows about, so a newly installed engine appears
after the daemon restarts:

```bash
ibus restart
```

Then add it in *Settings → Keyboard → Input Sources → + → Nepali → "Nepali
(transliteration)"*, or `ibus engine xlit-ne`. For Fcitx5, it is *System
Settings → Input Method → + → Nepali*.

The daemon is optional on Linux: each frontend is one long-lived process and
links the engine directly, so there is nothing to share. Start it if you want
one dictionary in memory for several frontends at once:

```bash
systemctl --user enable --now xlit-daemon
```

## Tested how far

The Debian packages are built and their contents checked on Ubuntu 24.04. The
RPM spec and the PKGBUILD are written against the same Makefile targets but
have **not** been built — there is no `rpmbuild` or `makepkg` on the machine
they were written on. Expect to fix something on the first real build, most
likely a missing `BuildRequires` or a path that Fedora spells differently.
