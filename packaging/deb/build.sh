#!/bin/sh
# Build Debian/Ubuntu packages.
#
#   packaging/deb/build.sh            all packages the machine can build
#   packaging/deb/build.sh --no-build use binaries already in target/release
#
# Output: packaging/build/*.deb
#
# Three packages, because the two frontends pull in different desktop stacks
# and nobody should have to install Fcitx5 to type Nepali under GNOME:
#
#   xlit-common   the engine binaries: `xlit`, `xlit-daemon`, the user unit
#   xlit-ibus     the IBus engine (GNOME and most distributions)
#   xlit-fcitx5   the Fcitx5 addon (KDE) — built only where its headers are
#
# Everything about *where files go* lives in the Makefile, not here: this
# script stages `make install-*` into a directory and wraps the result.
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
out="$root/packaging/build"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/crates/xlit-cli/Cargo.toml" | head -1)
arch=$(dpkg --print-architecture)
maintainer="Prabidhi.bid <prdpspkt@gmail.com>"

build=yes
[ "${1:-}" = "--no-build" ] && build=no

# Ubuntu 25.10 and later refuse to install a .deb from the desktop when it
# carries no signature: "the package does not contain a publisher's name".
# Set XLIT_SIGN_KEY to a GPG key id (or an email that selects one) and every
# package built here is signed with it — see packaging/deb/sign.sh, which also
# installs the debsig policy that makes the signature verifiable locally.
# Canonical's own key cannot be used; only Canonical holds it.
sign_key=${XLIT_SIGN_KEY:-}
if [ -n "$sign_key" ]; then
    command -v debsigs >/dev/null || {
        echo "XLIT_SIGN_KEY is set but debsigs is missing (apt install debsigs)"
        exit 1
    }
fi

command -v dpkg-deb >/dev/null || { echo "dpkg-deb not found (apt install dpkg-dev)"; exit 1; }

if [ "$build" = yes ]; then
    ( cd "$root" && make build )
fi

rm -rf "$out"
mkdir -p "$out"

# stage <package> <make-target> <depends> <description...>
stage() {
    pkg=$1; target=$2; depends=$3; shift 3
    stagedir="$out/$pkg"
    rm -rf "$stagedir"
    mkdir -p "$stagedir/DEBIAN"
    # Debian wants each binary package's documentation under its own name.
    ( cd "$root" && make "$target" DESTDIR="$stagedir" PREFIX=/usr \
        DOCDIR="/usr/share/doc/$pkg" >/dev/null )

    # Debian keeps licences in /usr/share/doc/<pkg>/copyright, not the
    # /usr/share/licenses the Makefile uses for everyone else.
    rm -rf "$stagedir/usr/share/licenses"
    mkdir -p "$stagedir/usr/share/doc/$pkg"
    cp "$root/packaging/deb/copyright" "$stagedir/usr/share/doc/$pkg/copyright"

    installed=$(du -ks "$stagedir" | cut -f1)
    cat > "$stagedir/DEBIAN/control" <<CONTROL
Package: $pkg
Version: $version
Section: utils
Priority: optional
Architecture: $arch
Depends: $depends
Maintainer: $maintainer
Installed-Size: $installed
Description: $*
CONTROL
    deb="$out/${pkg}_${version}_${arch}.deb"
    dpkg-deb --build --root-owner-group "$stagedir" "$deb" >/dev/null
    rm -rf "$stagedir"
    # An `origin` signature is the one debsig-verify looks for when the archive
    # itself, rather than a repository, is what has to vouch for the files.
    if [ -n "$sign_key" ]; then
        debsigs --sign=origin --default-key="$sign_key" "$deb"
        echo "  $deb (signed by $sign_key)"
    else
        echo "  $deb"
    fi
}

echo "==> packages"
# Rust binaries against glibc: libc and libgcc, nothing else. Listed by hand
# rather than by dpkg-shlibdeps so that this script needs only dpkg-deb.
stage xlit-common install-common 'libc6 (>= 2.34), libgcc-s1' \
    "Nepali transliteration engine
 Type Nepali phonetically in Latin letters and get Devanagari: namaste
 becomes नमस्ते. This package holds the shared pieces — the xlit command
 line tool and xlit-daemon, which serves candidates to the input method
 frontends. Install xlit-ibus or xlit-fcitx5 to type with it."

stage xlit-ibus install-ibus "xlit-common (= $version), ibus" \
    "Nepali transliteration input method for IBus
 An IBus engine for Nepali phonetic input: type namaste, get नमस्ते, with
 a candidate list for the spellings the sounds leave open. IBus is the
 input framework GNOME and most distributions use by default."

# `make` knows how to look for Fcitx5 (a CMake package, not a pkg-config one);
# ask it rather than keeping a second, differently-wrong probe here.
if [ "$( cd "$root" && make -s print-HAVE_FCITX5 )" = yes ]; then
    ( cd "$root" && make build-fcitx5 >/dev/null )
    stage xlit-fcitx5 install-fcitx5 "xlit-common (= $version), fcitx5" \
        "Nepali transliteration input method for Fcitx5
 A Fcitx5 addon for Nepali phonetic input: type namaste, get नमस्ते. Fcitx5
 is the input framework KDE Plasma uses by default."
else
    echo "  (skipping xlit-fcitx5: Fcitx5 development files not installed)"
fi

echo
echo "install with: sudo apt install $out/*.deb"
[ -n "$sign_key" ] || echo "unsigned; see packaging/deb/sign.sh to sign for desktop installs"
