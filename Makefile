# Build and install the Linux input method.
#
#   make                    build everything that can be built here
#   sudo make install       install engine, addon, daemon and CLI
#   sudo make uninstall     take them out again
#
# The distro packages under packaging/ all drive these same targets with
# DESTDIR set, so there is one description of where files go, not four.
#
# Overridable, because distributions disagree:
#
#   PREFIX          /usr             /usr/local for a hand build
#   LIBEXECDIR      $(PREFIX)/libexec   Arch puts IBus engines in /usr/lib/ibus
#   FCITX5_LIBDIR   auto-detected    where fcitx5 loads addons from

PREFIX ?= /usr
DESTDIR ?=
BINDIR ?= $(PREFIX)/bin
LIBEXECDIR ?= $(PREFIX)/libexec
DATADIR ?= $(PREFIX)/share
SYSTEMD_USER_DIR ?= $(PREFIX)/lib/systemd/user
DOCDIR ?= $(DATADIR)/doc/xlit

CARGO ?= cargo
CARGO_FLAGS ?= --release --locked
TARGET_DIR ?= target/release

INSTALL ?= install
CMAKE ?= cmake

# Where Fcitx5's own CMake package lives. Finding *this* rather than asking
# pkg-config is deliberate: Fcitx5 ships CMake config packages and Debian's
# libfcitx5core-dev installs no .pc file, so a pkg-config probe reported the
# development files missing on a machine that had them — and quietly built a
# release with the addon left out.
FCITX5_CMAKE_DIR := $(firstword $(wildcard     /usr/lib/*/cmake/Fcitx5Core /usr/lib/cmake/Fcitx5Core     /usr/lib64/cmake/Fcitx5Core /usr/local/lib/*/cmake/Fcitx5Core))

# Fcitx5 keeps its addons next to its own library: /usr/lib/x86_64-linux-gnu on
# Debian, /usr/lib64 on Fedora. Two directories up from the CMake package is
# exactly that, whichever it is.
FCITX5_LIBDIR ?= $(if $(FCITX5_CMAKE_DIR),$(abspath $(FCITX5_CMAKE_DIR)/../..),$(PREFIX)/lib)
FCITX5_DATADIR ?= $(DATADIR)/fcitx5

# Whether the Fcitx5 addon can be built at all: it is C++ and needs both the
# development package and cmake. Everything else builds with cargo alone.
HAVE_FCITX5 := $(shell test -n "$(FCITX5_CMAKE_DIR)" && command -v $(CMAKE) >/dev/null 2>&1 && echo yes)

.PHONY: all build build-fcitx5 check install install-common install-ibus \
        install-fcitx5 uninstall dist clean help print-%

all: build

# Read one variable's value, so a packaging script can ask this Makefile what it
# decided instead of re-implementing the decision and getting it wrong:
#   make -s print-HAVE_FCITX5
print-%:
	@echo '$($*)'

help:
	@echo "targets: build, build-fcitx5, check, install, uninstall, dist, clean"
	@echo "fcitx5 addon buildable here: $(if $(HAVE_FCITX5),yes,no — install fcitx5 development packages)"

build:
	$(CARGO) build $(CARGO_FLAGS) -p xlit-cli -p xlit-daemon -p xlit-ibus
ifeq ($(HAVE_FCITX5),yes)
	$(MAKE) build-fcitx5
endif

build-fcitx5:
	$(CARGO) build $(CARGO_FLAGS) -p xlit-fcitx5
	$(CMAKE) -B frontends/linux-fcitx5/build -S frontends/linux-fcitx5/cpp \
	    -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=$(PREFIX)
	$(CMAKE) --build frontends/linux-fcitx5/build --parallel

check:
	$(CARGO) test --locked

install: install-common install-ibus
ifeq ($(HAVE_FCITX5),yes)
	$(MAKE) install-fcitx5
endif

# The engine, the CLI and the documentation every frontend shares.
install-common:
	$(INSTALL) -Dm755 $(TARGET_DIR)/xlit $(DESTDIR)$(BINDIR)/xlit
	$(INSTALL) -Dm755 $(TARGET_DIR)/xlit-daemon $(DESTDIR)$(BINDIR)/xlit-daemon
	$(INSTALL) -Dm644 crates/xlit-daemon/dist/xlit-daemon.service \
	    $(DESTDIR)$(SYSTEMD_USER_DIR)/xlit-daemon.service
	# The shipped unit starts the daemon out of ~/.local/bin, which is right
	# for a hand build and wrong for a package.
	sed -i 's|ExecStart=.*|ExecStart=$(BINDIR)/xlit-daemon|' \
	    $(DESTDIR)$(SYSTEMD_USER_DIR)/xlit-daemon.service
	$(INSTALL) -Dm644 README.md $(DESTDIR)$(DOCDIR)/README.md
	$(INSTALL) -Dm644 LICENSE-MIT $(DESTDIR)$(DATADIR)/licenses/xlit/LICENSE-MIT
	$(INSTALL) -Dm644 LICENSE-APACHE $(DESTDIR)$(DATADIR)/licenses/xlit/LICENSE-APACHE

# IBus: a binary IBus starts, and an XML file telling it how.
install-ibus:
	$(INSTALL) -Dm755 $(TARGET_DIR)/xlit-ibus $(DESTDIR)$(LIBEXECDIR)/ibus-engine-xlit
	$(INSTALL) -Dm644 frontends/linux-ibus/data/xlit.xml \
	    $(DESTDIR)$(DATADIR)/ibus/component/xlit.xml
	# <exec> must be the absolute path of the installed binary, which is not
	# known until here — DESTDIR is a staging directory and must not appear.
	sed -i 's|<exec>.*</exec>|<exec>$(LIBEXECDIR)/ibus-engine-xlit --ibus</exec>|' \
	    $(DESTDIR)$(DATADIR)/ibus/component/xlit.xml

# Fcitx5: cmake knows where its own pieces go.
install-fcitx5:
	DESTDIR=$(DESTDIR) $(CMAKE) --install frontends/linux-fcitx5/build

uninstall:
	rm -f $(DESTDIR)$(BINDIR)/xlit $(DESTDIR)$(BINDIR)/xlit-daemon
	rm -f $(DESTDIR)$(SYSTEMD_USER_DIR)/xlit-daemon.service
	rm -f $(DESTDIR)$(LIBEXECDIR)/ibus-engine-xlit
	rm -f $(DESTDIR)$(DATADIR)/ibus/component/xlit.xml
	rm -f $(DESTDIR)$(FCITX5_LIBDIR)/fcitx5/xlit.so
	rm -f $(DESTDIR)$(FCITX5_DATADIR)/addon/xlit.conf
	rm -f $(DESTDIR)$(FCITX5_DATADIR)/inputmethod/xlit.conf
	rm -rf $(DESTDIR)$(DOCDIR) $(DESTDIR)$(DATADIR)/licenses/xlit

# Source tarball for the RPM spec and the PKGBUILD, which both expect
# xlit-$(VERSION)/ inside it.
VERSION ?= $(shell sed -n 's/^version = "\(.*\)"/\1/p' crates/xlit-cli/Cargo.toml | head -1)

dist:
	mkdir -p packaging/build
	git archive --format=tar.gz --prefix=xlit-$(VERSION)/ \
	    -o packaging/build/xlit-$(VERSION).tar.gz HEAD
	@echo "packaging/build/xlit-$(VERSION).tar.gz"

clean:
	$(CARGO) clean
	rm -rf frontends/linux-fcitx5/build packaging/build
