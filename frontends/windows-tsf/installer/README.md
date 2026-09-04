# Installer — `Input by Prabidhi.bid`

Two ways to install the Windows TSF text service:

- **`build-setup.ps1`** → a distributable `Setup.exe` (Inno Setup).
- **`install.ps1` / `uninstall.ps1`** → plain PowerShell, no toolchain, for
  dev boxes and power users.

Both register the DLL the same way (`regsvr32` → its `DllRegisterServer`).

## Distributable: `Setup.exe` (Inno Setup)

Prerequisite (one time): `winget install JRSoftware.InnoSetup`

```
powershell -ExecutionPolicy Bypass -File frontends\windows-tsf\installer\build-setup.ps1
```

→ `installer\Input-by-Prabidhi.bid-<ver>-setup.exe`, DLL embedded. Options:
`-Version 0.2.0`, `-Configuration debug`, `-Password <pw>` (encrypt payload).

[`xlit-tsf.iss`](xlit-tsf.iss) installs to `%ProgramFiles%\Prabidhi.bid Input\`,
runs `regsvr32 /s` on install and `/s /u` on uninstall, recycles
`ctfmon`/`TextInputHost`, and lets Inno own the Apps & features entry and file
removal. Keep `AppId` stable across releases.

## Dev: `install.ps1`

```
powershell -ExecutionPolicy Bypass -File frontends\windows-tsf\installer\install.ps1
```

What it does (elevated):

1. `cargo build -p xlit-tsf --release` (skip with `-NoBuild`; `-Configuration debug` for a debug build).
2. Copy `xlit_tsf.dll` + `uninstall.ps1` to `%ProgramFiles%\Prabidhi.bid Input\`.
3. `regsvr32 /s` the installed DLL — runs its `DllRegisterServer`, which writes
   the COM CLSID keys and the `HKLM\SOFTWARE\Microsoft\CTF\TIP` profile,
   `EnableLanguageProfile`, and the `TIPCAP_*` categories.
4. Add an **Apps & features** entry (`…\Uninstall\PrabidhibidInput`) whose
   uninstall command runs the copied `uninstall.ps1`.
5. Restart `ctfmon` / `TextInputHost` so the new profile is picked up.

If a running process (the Claude app, an open console) still maps an older
`xlit_tsf.dll`, the locked file is renamed to `xlit_tsf.dll.<timestamp>.old`
and a fresh one is written; the stale copy unloads on the next sign-out.

After installing: **Settings → Time & language → Language & region → Add a
language → Nepali**, then pick *Input by Prabidhi.bid* from the taskbar
language button (`Win+Space`). Sign out / in if it is not listed yet.

## Uninstall

Apps & features → *Input by Prabidhi.bid* → Uninstall, or:

```
powershell -ExecutionPolicy Bypass -File "%ProgramFiles%\Prabidhi.bid Input\uninstall.ps1"
```

`regsvr32 /s /u` (→ `DllUnregisterServer` removes the COM + CTF\TIP keys),
deletes the install folder, removes the Apps & features entry. Files still in
use are renamed aside and swept after the script exits / on next sign-out.

## Hardening against reverse engineering

You can raise the cost; you can't stop a determined analyst from reversing a
native DLL. What actually helps, most valuable first:

1. **Code-sign the DLL and the `Setup.exe`.** This is integrity / anti-tamper —
   the one that matters. An unsigned build can be patched and re-shipped
   silently. With an OV/EV Authenticode cert:
   `signtool sign /fd sha256 /a /tr http://timestamp.digicert.com /td sha256 <file>`
   and set `SignTool=` in [`xlit-tsf.iss`](xlit-tsf.iss) (commented example there).
2. **Ship a stripped release with no trace.** Done: the workspace release
   profile is `strip = true` + `lto` + `opt-level = "z"`, and the DebugView
   calls are behind the `trace` cargo feature (**off by default**). Only
   `cargo build -p xlit-tsf --release --features trace` emits `[xlit-tsf] …`.
3. **Encrypt the installer payload** — `build-setup.ps1 -Password <pw>` →
   Inno `Encryption`. Blocks casual `innounp` / `7z` extraction; the password
   still ships with the installer, so it's speed-bump grade.
4. **Commercial protectors** (VMProtect / Themida / Enigma) add VM / anti-debug.
   Caveat for a TSF TIP: the DLL loads into *every* process (Explorer, browsers,
   AV), and packed / anti-debug binaries routinely trip AV heuristics and
   Windows CIG/ACG in those hosts. Test broadly; several IME vendors don't pack
   for this reason.
5. **String obfuscation** (`obfstr` / `litcrypt`) only once something sensitive
   is embedded (license keys, endpoints). Nothing today warrants it — the CLSID
   and display name are in the registry after install anyway.

Not worth doing: anti-debugger tricks that fight security tooling — they cost
more in compatibility and support than they cost an analyst.

## Limitations / TODO

- **Per-user enable.** `DllRegisterServer` calls `EnableLanguageProfile`, an
  `HKCU` write — it covers the user who runs the installer. Other users may
  need to enable the keyboard once from language settings. An `ActiveSetup`
  stub (per-user command on first logon) is the fix.
- **x64 only.** No x86 / ARM64 payload yet, so 32-bit apps won't load the TIP.
- A signed MSI (deferred no-impersonate custom actions + Active Setup) is the
  eventual M6.5 target; the Inno `Setup.exe` covers it until then.
