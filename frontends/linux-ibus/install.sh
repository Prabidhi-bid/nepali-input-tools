#!/bin/sh
# Install the IBus engine. Run from anywhere; needs sudo for the system paths.
#
#   ./frontends/linux-ibus/install.sh
#
# Pass --user to install into ~/.local instead, with no sudo.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)

if [ "${1:-}" = "--user" ]; then
    bindir="$HOME/.local/lib/ibus"
    compdir="$HOME/.local/share/ibus/component"
    sudo=""
else
    bindir="/usr/lib/ibus"
    compdir="/usr/share/ibus/component"
    sudo="sudo"
fi

echo "==> cargo build -p xlit-ibus --release"
( cd "$root" && cargo build -p xlit-ibus --release )

echo "==> installing to $bindir"
$sudo mkdir -p "$bindir" "$compdir"
$sudo install -m 755 "$root/target/release/xlit-ibus" "$bindir/xlit-ibus"

# The component file names the binary by absolute path, so rewrite <exec> to
# wherever we actually put it rather than shipping two copies of the file.
sed "s|<exec>.*</exec>|<exec>$bindir/xlit-ibus --ibus</exec>|" "$here/data/xlit.xml" \
    | $sudo tee "$compdir/xlit.xml" > /dev/null

echo "==> restarting ibus"
ibus restart 2>/dev/null || ibus-daemon -drx

cat <<'MSG'

Installed. Add it in Settings > Keyboard > Input Sources > + > Nepali >
"Nepali (transliteration)", or:

    ibus engine xlit-ne

If it does not appear, check that the daemon sees it:

    ibus list-engine | grep xlit

and run the binary by hand to see why it will not start:

    /usr/lib/ibus/xlit-ibus
MSG
