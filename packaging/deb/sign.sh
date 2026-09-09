#!/bin/sh
# Sign the built .deb files, and teach this machine to verify the signature.
#
#   packaging/deb/sign.sh <key>     sign packaging/build/*.deb with <key>
#   packaging/deb/sign.sh --policy <key>
#                                   also install /etc/debsig (needs root)
#
# Why this exists: from Ubuntu 25.10 on, the desktop installer refuses a .deb
# that carries no signature — "the package does not contain a publisher's
# name" — because there is nothing in an unsigned archive that says who built
# it. The signature has to come from a key *we* hold; Canonical's key signs
# Canonical's archive and is not available to anyone else, so a package signed
# here is trusted only where its public key has been installed.
#
# <key> is anything gpg accepts: a key id, a fingerprint, an email address.
# Make one first if you have none:
#
#   gpg --quick-generate-key "Prabidhi.bid <prdpspkt@gmail.com>" default default never
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
out="$root/packaging/build"

policy=no
[ "${1:-}" = "--policy" ] && { policy=yes; shift; }
key=${1:-}
[ -n "$key" ] || { echo "usage: $0 [--policy] <gpg-key>"; exit 1; }

for tool in gpg debsigs; do
    command -v "$tool" >/dev/null || { echo "$tool not found (apt install debsigs debsig-verify)"; exit 1; }
done

# --policy needs root, but the signing key lives in the *invoking* user's
# keyring: run this under sudo and a plain `gpg` reads root's empty keybox and
# reports the key missing. Every gpg call goes back through the real user.
if [ "$(id -u)" = 0 ] && [ -n "${SUDO_USER:-}" ]; then
    user_gpg() { sudo -u "$SUDO_USER" gpg "$@"; }
else
    user_gpg() { gpg "$@"; }
fi

# debsig-verify keys its policies by the *fingerprint* of the signing key, so
# resolve whatever the caller passed down to one before writing any of it out.
fpr=$(user_gpg --with-colons --list-secret-keys "$key" 2>/dev/null |
      awk -F: '/^fpr:/ { print $10; exit }')
[ -n "$fpr" ] || {
    echo "no secret gpg key matches '$key'. Make one:"
    echo "  gpg --quick-generate-key '$key' default default never"
    exit 1
}

set -- "$out"/*.deb
[ -e "$1" ] || { echo "no packages in $out — run packaging/deb/build.sh first"; exit 1; }
for deb; do
    # debsigs shells out to gpg too, and has the same root problem.
    if [ "$(id -u)" = 0 ] && [ -n "${SUDO_USER:-}" ]; then
        sudo -u "$SUDO_USER" debsigs --sign=origin --default-key="$fpr" "$deb"
    else
        debsigs --sign=origin --default-key="$fpr" "$deb"
    fi
    echo "  signed $deb"
done

if [ "$policy" = yes ]; then
    [ "$(id -u)" = 0 ] || { echo "--policy writes to /etc/debsig; run it with sudo"; exit 1; }
    # Two halves, and both are required: the exported public key under
    # keyrings/<fpr>/, and a policy naming that same fingerprint.
    install -d "/usr/share/debsig/keyrings/$fpr" "/etc/debsig/policies/$fpr"
    user_gpg --export "$fpr" > "/usr/share/debsig/keyrings/$fpr/debsig.gpg"
    cat > "/etc/debsig/policies/$fpr/xlit.pol" <<POLICY
<?xml version="1.0"?>
<!DOCTYPE Policy SYSTEM "https://www.debian.org/debsig/1.0/policy.dtd">
<Policy xmlns="https://www.debian.org/debsig/1.0/">
  <Origin Name="xlit" id="$fpr" Description="Nepali transliteration input method"/>
  <Selection>
    <Required Type="origin" File="debsig.gpg" id="$fpr"/>
  </Selection>
  <Verification MinOptional="0">
    <Required Type="origin" File="debsig.gpg" id="$fpr"/>
  </Verification>
</Policy>
POLICY
    echo "  policy installed for $fpr"
fi

echo
echo "verify with: debsig-verify $out/$(basename "$1")"
