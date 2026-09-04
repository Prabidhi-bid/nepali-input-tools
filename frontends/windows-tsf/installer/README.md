# Installer — `Input by Prabidhi.bid`

Plain-PowerShell install / uninstall for the Windows TSF text service. No
installer toolchain (no WiX, no .NET). Both scripts self-elevate.

## Install

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

## Limitations / TODO

- **Per-user enable.** `DllRegisterServer` calls `EnableLanguageProfile`, an
  `HKCU` write — it covers the user who runs the installer. Other users may
  need to enable the keyboard once from language settings. An `ActiveSetup`
  stub (per-user command on first logon) is the fix.
- **x64 only.** No x86 / ARM64 payload yet, so 32-bit apps won't load the TIP.
- **Not signed.** SmartScreen / PowerShell may warn.
- This is a script, not a redistributable `.exe`/`.msi`. A signed MSI (deferred
  no-impersonate custom actions + Active Setup) is the eventual M6.5 target.
