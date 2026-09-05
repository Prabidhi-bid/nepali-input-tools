# Installer — `Input by Prabidhi.bid`

Two ways in:

- **`xlit-install.exe`** (`crates/xlit-install`) -> one self-contained
  executable. Both DLLs and the word editor are compiled into it, so the machine
  it runs on needs nothing else - no Rust, no scripts beside it, no
  redistributable. Self-elevating. This is the normal way in.
- **`xlit-tsf.iss` + `build-setup.ps1`** -> a distributable **`Setup.exe`**
  (Inno Setup), if you want a wizard.

```
cargo build -p xlit-tsf --target x86_64-pc-windows-msvc --release
cargo build -p xlit-tsf --target i686-pc-windows-msvc --release
cargo build -p xlit-config --release
cargo build -p xlit-install --release      # embeds the three artifacts above
target\release\xlit-install.exe
```

Build order matters: `xlit-install`'s `build.rs` embeds whatever is in `target/`
at the time it is compiled. It warns rather than fails when a payload is
missing, so a stale installer is possible - rebuild it last, every time.

Uninstall with `xlit-install.exe --uninstall`, or from Apps & features.

## What `xlit-install.exe` does

1. Elevates (the COM keys and the TSF profile live in HKLM).
2. Writes both DLLs, the word editor and a copy of itself to
   `%ProgramFiles%\Prabidhi.bid Input\`. A file still mapped by a running
   process is renamed to `<name>.<timestamp>.old` and a fresh one written; the
   stale copy unloads on the next sign-out and is swept by the next install.
3. Registers the 64-bit DLL by calling its own `DllRegisterServer` - the call
   that writes the COM CLSID keys and, via
   `ITfInputProcessorProfileMgr::RegisterProfile`, the
   `HKLM\SOFTWARE\Microsoft\CTF\TIP` profile plus the `TIPCAP_*` categories.
4. Writes the **32-bit** CLSID keys straight into the WOW6432Node view.
   Deliberately *not* a second `DllRegisterServer`: the TSF profile and the
   categories are machine-wide and bitness-independent, so step 3 already did
   them, and all the 32-bit side needs is its own `InprocServer32`. Doing it
   this way means the installer never has to load the 32-bit DLL - which a
   64-bit process cannot do anyway, and which is why the old script had to shell
   out to the SysWOW64 `regsvr32`.
5. Back in the user's own (unelevated) session, adds the keyboard to the
   language list with `InstallLayoutOrTip`. This is per-user: written from the
   elevated half it lands in the *administrator's* profile and the keyboard
   silently never appears.
6. Restarts `ctfmon` / `TextInputHost` so the switcher refreshes.
7. Adds an **Apps & features** entry whose uninstall command is the installed
   copy of the installer with `--uninstall`.

### Why not `regsvr32`

`regsvr32` is a GUI-subsystem program: it cannot write to a console, so a
failure arrives as a bare exit code with the underlying Win32 error discarded.
A 32-bit DLL that would not load reported `3` and nothing else. The installer
makes the same three calls itself - `LoadLibraryEx`, `GetProcAddress`,
`DllRegisterServer` - and reports the actual error, so *"a DLL it depends on is
missing (126)"* replaces *"3"*.

## Build `Setup.exe`

Prerequisite (one time): `winget install JRSoftware.InnoSetup`

```
powershell -ExecutionPolicy Bypass -File frontends\windows-tsf\installer\build-setup.ps1
```

→ `installer\Input-by-Prabidhi.bid-<ver>-setup.exe`, both DLLs embedded.
Options: `-Version 0.2.0`, `-Configuration debug`, `-SkipX86`,
`-Password <pw>` (encrypt payload), `-Iscc "<path>\ISCC.exe"` (if
auto-detection misses it — it also checks Inno's registry install location and
the usual folders). `xlit-tsf.iss` registers per bitness (`System32` regsvr32
for the native DLL, `SysWOW64` for the x86 one), writes the keyboard from a
`runascurrentuser` step, and on uninstall reverses it (language list, `regsvr32 /u`, an HKLM key
force-delete fallback, an HKCU CTF sweep) plus Inno's own file/ARP removal. Keep `AppId`
stable across releases.

After installing it should already be in `Win+Space` - sign out / in if not.

## Why two DLLs

A TSF text service is loaded into every process that takes text input; 32-bit
apps load the 32-bit `xlit_tsf.dll` from the WOW64 registry view.

| | 64-bit Windows | 32-bit Windows |
|---|---|---|
| x64 DLL | `{app}\xlit_tsf.dll` (System32 `regsvr32`) | — |
| x86 DLL | `{app}\x86\xlit_tsf.dll` (SysWOW64 `regsvr32`) | `{app}\xlit_tsf.dll` (System32 `regsvr32`) |

`{app}` = `%ProgramFiles%\Prabidhi.bid Input\`. The i686 build needs the x86
MSVC toolchain ("MSVC v143 x86/x64 build tools" in the VS Installer). ARM64:
x64 + x86 run under emulation; no native ARM64 build.

## Uninstall

Apps & features → *Input by Prabidhi.bid* → Uninstall, or:

```
"%ProgramFiles%\Prabidhi.bid Input\xlit-install.exe" --uninstall
```

Completely reverses the install, in order:

1. `Set-WinUserLanguageList` — remove this TIP from your language list (and
   `ne-NP` itself if nothing else is left under it).
2. `regsvr32 /s /u` for each installed bitness → `DllUnregisterServer` clears
   the COM CLSID keys + the CTF\TIP profile and categories.
3. Registry fallback — force-delete `Classes\CLSID\{CLSID}`,
   `Wow6432Node\CLSID\{CLSID}`, `CTF\TIP\{CLSID}` in case a DLL couldn't be
   loaded to self-unregister.
4. HKCU CTF sweep — drop cached `Assemblies` / `SortOrder` entries still naming
   the CLSID.
5. Remove the Apps & features entry.
6. Delete `%ProgramFiles%\Prabidhi.bid Input\` (locked files renamed aside +
   swept by a detached `rmdir` / on next sign-out).

Not touched (development-environment, not installation state): the
`rustup target add i686-pc-windows-msvc` toolchain component and
`target\**\xlit_tsf.dll*` build outputs.

## Hardening against reverse engineering

You can raise the cost; you can't stop a determined analyst from reversing a
native DLL. What actually helps, most valuable first:

1. **Code-sign the DLL** with an OV/EV Authenticode cert — integrity /
   anti-tamper, the one that matters. An unsigned build can be patched and
   re-shipped silently.
   `signtool sign /fd sha256 /a /tr http://timestamp.digicert.com /td sha256 <file>`
2. **Ship a stripped release with no trace.** Done: the workspace release
   profile is `strip = true` + `lto` + `opt-level = "z"`, and the DebugView
   calls are behind the `trace` cargo feature (**off by default**). Only
   `cargo build -p xlit-tsf --release --features trace` emits `[xlit-tsf] …`.
3. **Commercial protectors** (VMProtect / Themida / Enigma) add VM / anti-debug.
   Caveat for a TSF TIP: the DLL loads into *every* process (Explorer, browsers,
   AV), and packed / anti-debug binaries routinely trip AV heuristics and
   Windows CIG/ACG in those hosts. Test broadly; several IME vendors don't pack.
4. **String obfuscation** (`obfstr` / `litcrypt`) only once something sensitive
   is embedded (license keys, endpoints). Nothing today warrants it.

Not worth doing: anti-debugger tricks that fight security tooling — they cost
more in compatibility and support than they cost an analyst.

## Limitations / TODO

- **Per-user.** `RegisterProfile` is machine-wide (HKLM), but the
  `Set-WinUserLanguageList` step that puts it in the switcher is `HKCU` — it
  covers the user who runs the installer. Other users get the registration but
  must add the keyboard themselves (Settings → Language → Nepali → Keyboards).
  An `ActiveSetup` stub (per-user command on first logon) is the fix.
- **No native ARM64 payload.**
- `COMLESS` category is deliberately not registered (classic COM server only) —
  it was hiding the TIP from the modern switcher.
- `Setup.exe` is unsigned — SmartScreen will warn. Code-sign it (and the DLL)
  before wider distribution; a proper signed MSI with Active Setup for other
  users is the eventual M6.5 target.
