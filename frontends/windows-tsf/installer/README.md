# Installer — `Input by Prabidhi.bid`

Two ways to install the Windows TSF text service:

- **`build-setup.ps1`** → a distributable `Setup.exe` (Inno Setup).
- **`install.ps1` / `uninstall.ps1`** → plain PowerShell, no toolchain, for
  dev boxes and power users.

Both do the same thing: **deregister any prior copy**, register the DLL
(`regsvr32` → its `DllRegisterServer`), then add `ne-NP` + this TIP to the
current user's language list (`Set-WinUserLanguageList`) so it shows in the
taskbar / `Win+Space` switcher without a manual Settings visit. Uninstall
reverses both.

## Distributable: `Setup.exe` (Inno Setup)

Prerequisite (one time): `winget install JRSoftware.InnoSetup`

```
powershell -ExecutionPolicy Bypass -File frontends\windows-tsf\installer\build-setup.ps1
```

→ `installer\Input-by-Prabidhi.bid-<ver>-setup.exe`, DLL embedded. Options:
`-Version 0.2.0`, `-Configuration debug`, `-Password <pw>` (encrypt payload),
`-Iscc "<path>\ISCC.exe"` (if auto-detection misses it — e.g. Inno was added to
PATH after this shell started; the script also checks Inno's registry install
location and the usual folders).

[`xlit-tsf.iss`](xlit-tsf.iss) installs to `%ProgramFiles%\Prabidhi.bid Input\`
and, in `[Run]`: `regsvr32 /s /u` (clear prior state) → `regsvr32 /s` → a
`runascurrentuser` PowerShell step that adds the keyboard to the user's list →
recycle `ctfmon`/`TextInputHost`. On a re-install, `[Code]` also unregisters the
old copy and frees the hosts *before* `[Files]` overwrites the DLL (with
`restartreplace` as the last-resort fallback for a still-locked file). Inno owns
the Apps & features entry and file removal. Keep `AppId` stable across releases.

## Dev: `install.ps1`

```
powershell -ExecutionPolicy Bypass -File frontends\windows-tsf\installer\install.ps1
```

What it does (elevated):

1. `cargo build -p xlit-tsf --release` (skip with `-NoBuild`; `-Configuration debug` for a debug build).
2. Copy `xlit_tsf.dll` + `uninstall.ps1` to `%ProgramFiles%\Prabidhi.bid Input\`.
3. `regsvr32 /s /u` then `/s` the installed DLL — a clean deregister → register.
   `DllRegisterServer` writes the COM CLSID keys, the
   `HKLM\SOFTWARE\Microsoft\CTF\TIP` profile, `EnableLanguageProfile`, and the
   `TIPCAP_*` categories.
4. `Set-WinUserLanguageList` — add `ne-NP` + this TIP to your language list.
5. Add an **Apps & features** entry (`…\Uninstall\PrabidhibidInput`) whose
   uninstall command runs the copied `uninstall.ps1`.
6. Restart `ctfmon` / `TextInputHost` so the switcher refreshes.

If a running process (the Claude app, an open console) still maps an older
`xlit_tsf.dll`, the locked file is renamed to `xlit_tsf.dll.<timestamp>.old`
and a fresh one is written; the stale copy unloads on the next sign-out.

After installing it should already be in the taskbar / `Win+Space` switcher —
sign out / in if not.

## Uninstall

Apps & features → *Input by Prabidhi.bid* → Uninstall, or:

```
powershell -ExecutionPolicy Bypass -File "%ProgramFiles%\Prabidhi.bid Input\uninstall.ps1"
```

Removes this TIP from your language list, `regsvr32 /s /u` (→
`DllUnregisterServer` removes the COM + CTF\TIP keys), deletes the install
folder, removes the Apps & features entry. Files still in use are renamed aside
and swept after the script exits / on next sign-out.

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

- **Per-user.** `EnableLanguageProfile` and `Set-WinUserLanguageList` both write
  `HKCU` — they cover the user who runs the installer. Other users on the same
  machine get the machine-wide registration but must add the keyboard themselves
  (Settings → Language → Nepali → Keyboards). An `ActiveSetup` stub (per-user
  command on first logon) is the fix.
- **x64 only.** No x86 / ARM64 payload yet, so 32-bit apps won't load the TIP.
- `COMLESS` category is deliberately not registered (classic COM server only) —
  it was hiding the TIP from the modern switcher.
- A signed MSI (deferred no-impersonate custom actions + Active Setup) is the
  eventual M6.5 target; the Inno `Setup.exe` covers it until then.
