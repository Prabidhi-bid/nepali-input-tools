# Fedora / RHEL / openSUSE package.
#
#   rpmbuild -ba packaging/rpm/xlit.spec --define "_sourcedir $PWD"
#
# Everything about where files go lives in the Makefile; this file describes
# the split into packages and the build dependencies.
#
# Not built or tested on this machine — the author works on Debian. Treat the
# first build on a Fedora box as the real test.

%global _name xlit

Name:           xlit
Version:        0.1.1
Release:        1%{?dist}
Summary:        Nepali transliteration input method

License:        MIT OR Apache-2.0
URL:            https://github.com/prdpspkt/nepali-input-tools
Source0:        %{name}-%{version}.tar.gz

BuildRequires:  cargo
BuildRequires:  rust
BuildRequires:  make
BuildRequires:  gcc
# For the Fcitx5 addon only; drop this and %%package fcitx5 to build without it.
BuildRequires:  cmake
BuildRequires:  fcitx5-devel
BuildRequires:  pkgconfig

%description
Type Nepali phonetically in Latin letters and get Devanagari: namaste becomes
नमस्ते. A rule engine converts the sounds, a dictionary fixes the spelling, and
what you pick is remembered. This package holds the shared pieces — the xlit
command line tool and xlit-daemon. Install xlit-ibus or xlit-fcitx5 to type
with it.

%package ibus
Summary:        Nepali transliteration input method for IBus
Requires:       %{name} = %{version}-%{release}
Requires:       ibus

%description ibus
An IBus engine for Nepali phonetic input: type namaste, get नमस्ते, with a
candidate list for the spellings the sounds leave open. IBus is the input
framework GNOME and most distributions use by default.

%package fcitx5
Summary:        Nepali transliteration input method for Fcitx5
Requires:       %{name} = %{version}-%{release}
Requires:       fcitx5

%description fcitx5
A Fcitx5 addon for Nepali phonetic input: type namaste, get नमस्ते. Fcitx5 is
the input framework KDE Plasma uses by default.

%prep
%autosetup

%build
# The Makefile drives cargo and cmake; --locked keeps the build reproducible.
make build

%install
# Fedora keeps IBus engines in %%{_libexecdir} and 64-bit libraries in
# /usr/lib64, which is what %%{_libdir} expands to.
make install \
    DESTDIR=%{buildroot} \
    PREFIX=%{_prefix} \
    LIBEXECDIR=%{_libexecdir} \
    FCITX5_LIBDIR=%{_libdir}

%check
make check

%files
%license LICENSE-MIT LICENSE-APACHE
%doc README.md
%{_bindir}/xlit
%{_bindir}/xlit-daemon
%{_prefix}/lib/systemd/user/xlit-daemon.service

%files ibus
%{_libexecdir}/ibus-engine-xlit
%{_datadir}/ibus/component/xlit.xml

%files fcitx5
%{_libdir}/fcitx5/xlit.so
%{_datadir}/fcitx5/addon/xlit.conf
%{_datadir}/fcitx5/inputmethod/xlit.conf

%changelog
* Sun Sep 06 2026 Prabidhi.bid <prdpspkt@gmail.com> - 0.1.1-1
- Fcitx5 addon builds and loads; its candidate list no longer flickers.
- Windows: one floating bar on the desktop, numeric-keypad digits.

* Fri Sep 05 2025 Prabidhi.bid <prdpspkt@gmail.com> - 0.1.0-1
- First packaged release: IBus and Fcitx5 frontends, engine daemon, CLI.
