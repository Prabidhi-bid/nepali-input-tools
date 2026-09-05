#!/bin/sh
# Build and install the Fcitx5 input method (KDE Plasma's native framework).
#
#   ./frontends/linux-fcitx5/install.sh
#
# Needs the Fcitx5 development packages:
#   Debian/Ubuntu  sudo apt install fcitx5 libfcitx5core-dev libfcitx5utils-dev cmake g++
#   Fedora         sudo dnf install fcitx5-devel cmake gcc-c++
#   Arch           sudo pacman -S fcitx5 cmake gcc
set -eu

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)

echo "==> cargo build -p xlit-fcitx5 --release"
( cd "$root" && cargo build -p xlit-fcitx5 --release )

echo "==> cmake"
cmake -B "$here/build" -S "$here/cpp" -DCMAKE_BUILD_TYPE=Release
cmake --build "$here/build" --parallel

echo "==> installing"
sudo cmake --install "$here/build"

echo "==> restarting fcitx5"
fcitx5 -r -d 2>/dev/null || true

cat <<'MSG'

Installed. Add it in System Settings > Input Method (or the Fcitx5
configuration tool): click "+", search for Nepali, pick
"Nepali (transliteration)".

If it does not appear, check that fcitx5 loaded the addon:

    fcitx5 --verbose '*=5' 2>&1 | grep -i xlit
MSG
