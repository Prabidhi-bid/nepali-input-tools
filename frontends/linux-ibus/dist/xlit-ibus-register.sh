#!/bin/sh
# Add the xlit input source to GNOME's list, once, on first login after install.
#
# A package cannot do this from its postinst: dpkg runs as root, the input
# source list is per-user dconf state, and at install time the user may not be
# logged in at all. So it happens here instead — a user service, run once per
# user — and the alternative is what everyone hit before this existed: the
# engine installed, registered with IBus, and invisible, because GNOME shows
# no input switcher at all until a second source is added by hand.
#
# Run once, and never again: someone who removes the Nepali source meant to
# remove it, and a login service that puts it back every time would be a bug
# with no workaround. The stamp file is what makes the difference.
set -eu

stamp="${XDG_STATE_HOME:-$HOME/.local/state}/xlit/input-source-added"
[ -e "$stamp" ] && exit 0

schema=org.gnome.desktop.input-sources
# Not a GNOME session: nothing here applies, and leaving no stamp means this
# will do the right thing if the user later logs into one.
gsettings writable "$schema" sources >/dev/null 2>&1 || exit 0

current=$(gsettings get "$schema" sources)
case "$current" in
    *xlit-ne*) ;;   # already there — someone was ahead of us
    *)
        # The value is a GVariant array literal: "[('xkb', 'us')]", or
        # "@a(ss) []" when it is empty. Append to the former, replace the
        # latter, because "@a(ss) [...]" with contents is not what gsettings
        # hands back and not worth constructing.
        case "$current" in
            *"[]"*) new="[('ibus', 'xlit-ne')]" ;;
            *) new="${current%]}, ('ibus', 'xlit-ne')]" ;;
        esac
        gsettings set "$schema" sources "$new"
        ;;
esac

mkdir -p "$(dirname "$stamp")"
: > "$stamp"
