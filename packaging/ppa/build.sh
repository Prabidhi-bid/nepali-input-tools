#!/bin/sh
# Build signed source packages for a Launchpad PPA, one per Ubuntu series.
#
#   packaging/ppa/build.sh ppa:prdpspkt/xlit               current series only
#   packaging/ppa/build.sh ppa:prdpspkt/xlit noble plucky resolute
#   packaging/ppa/build.sh --upload ppa:prdpspkt/xlit noble
#
# It builds; it does not upload unless asked. An upload to a PPA is public and
# a version number can never be reused, so the last step is yours.
#
# Why a PPA at all: a .deb handed to someone directly has no publisher the
# desktop installer recognises, and `apt install ./xlit-ibus.deb` cannot
# resolve `Depends: xlit-common` because that name exists in no archive. Both
# problems are the same problem — the packages need to come from a repository —
# and Launchpad signs the archive with the key the user adds when they add the
# PPA.
#
# Needs: devscripts dput debhelper cargo (apt install devscripts dput debhelper)
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
out="$root/packaging/build/ppa"

upload=no
[ "${1:-}" = "--upload" ] && { upload=yes; shift; }

ppa=${1:-}
case "$ppa" in
    ppa:*/*) shift ;;
    *) echo "usage: $0 [--upload] ppa:<owner>/<name> [series...]"; exit 1 ;;
esac

# Default to the series this machine runs, which is the one the user asking
# for a package is almost always on.
if [ $# -gt 0 ]; then
    series=$*
else
    series=$(lsb_release -cs)
fi

for tool in debuild dpkg-source cargo; do
    command -v "$tool" >/dev/null || { echo "$tool not found (apt install devscripts dput debhelper cargo)"; exit 1; }
done
[ "$upload" = no ] || command -v dput >/dev/null || { echo "dput not found (apt install dput)"; exit 1; }

version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/crates/xlit-cli/Cargo.toml" | head -1)

rm -rf "$out"
mkdir -p "$out"

for s in $series; do
    # `~` sorts *before* the plain version, so the same upstream release can go
    # to several series at once and each stays distinguishable and upgradeable.
    ver="$version~${s}1"
    dir="$out/pb-input-$ver"

    # Build from what is committed, not from the working tree: a stray target/
    # or a half-edited file must not end up in a published source package.
    mkdir -p "$dir"
    ( cd "$root" && git archive --format=tar HEAD ) | tar -x -C "$dir"

    # Launchpad builds offline. Vendor the crates into the source package, with
    # --locked so the published build uses the same versions as a local one.
    ( cd "$dir" && cargo vendor --locked --versioned-dirs vendor >/dev/null )

    # Retarget the top changelog entry at this series.
    sed -i "1s/^pb-input (.*) .*; urgency=/pb-input ($ver) $s; urgency=/" "$dir/debian/changelog"

    ( cd "$dir" && debuild -S -sa -d )
    echo "  $out/pb-input_${ver}_source.changes"

    if [ "$upload" = yes ]; then
        dput "$ppa" "$out/pb-input_${ver}_source.changes"
    fi
done

echo
if [ "$upload" = yes ]; then
    echo "uploaded to $ppa — Launchpad emails you when the builds finish"
else
    echo "upload with: dput $ppa $out/pb-input_*_source.changes"
fi
echo "then, on any machine:"
echo "  sudo add-apt-repository $ppa && sudo apt install pb-input"
