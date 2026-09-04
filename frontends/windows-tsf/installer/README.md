# Installer — `Input by Prabidhi.bid`

`install.ps1` / `uninstall.ps1` — plain PowerShell, no toolchain. Both
self-elevate.

```
powershell -ExecutionPolicy Bypass -File frontends\windows-tsf\installer\install.ps1
```

## What `install.ps1` does (elevated)

1. `cargo build -p xlit-tsf --release` for `x86_64-` **and** `i686-pc-windows-msvc`
   (`rustup target add` is attempted for you). An i686 failure is a warning, not
   fatal — you get an x64-only result. Flags: `-NoBuild`, `-SkipX86`,
   `-Configuration debug`.
2. Copy each DLL to its slot + `uninstall.ps1` to the install root.
3. Per bitness: `regsvr32 /s /u` then `/s` — a clean deregister → register.
   `System32\regsvr32.exe` for the native DLL, `SysWOW64\regsvr32.exe` for the
   x86 one. `DllRegisterServer` writes the COM CLSID keys and, via
   `ITfInputProcessorProfileMgr::RegisterProfile`, the
   `HKLM\SOFTWARE\Microsoft\CTF\TIP` profile (enabled-by-default) + the
   `TIPCAP_*` categories.
4. `Set-WinUserLanguageList` — add `ne-NP` + this TIP to your language list, so
   it shows in the taskbar / `Win+Space` switcher without a Settings visit.
5. Add an **Apps & features** entry (`…\Uninstall\PrabidhibidInput`) whose
   uninstall command runs the copied `uninstall.ps1`.
6. Restart `ctfmon` / `TextInputHost` so the switcher refreshes.

A DLL still mapped by a running process (the Claude app, a console) is renamed
to `xlit_tsf.dll.<timestamp>.old` and a fresh one is written; the stale copy
unloads on the next sign-out.

After installing it should already be in `Win+Space` — sign out / in if not.

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
powershell -ExecutionPolicy Bypass -File "%ProgramFiles%\Prabidhi.bid Input\uninstall.ps1"
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
- Not a shareable double-click installer. A signed MSI (deferred no-impersonate
  custom actions + Active Setup) is the eventual M6.5 target.
