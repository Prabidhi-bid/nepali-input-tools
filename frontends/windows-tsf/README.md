# xlit-tsf — Windows text input processor

A COM in-process DLL that plugs the `xlit` engine into Windows' Text Services
Framework, so you can type Nepali phonetically in any application.

## Status: M6.1 — registrable no-op

The DLL registers as an input method ("Input by Prabidhi.bid") and can be
activated without crashing. It does **not** intercept keys yet — that's M6.2.
Use this stage only to confirm install / activate / uninstall work on your
machine.

## Build

```
cargo build -p xlit-tsf --release
```

Output: `target\release\xlit_tsf.dll` (x64).

## Install

See [`installer/`](installer) for both routes:

- **`build-setup.ps1`** — a distributable `Setup.exe` (Inno Setup).
- **`install.ps1`** — self-elevating, no toolchain.

```
powershell -ExecutionPolicy Bypass -File frontends\windows-tsf\installer\install.ps1
```

Either one: deregisters any prior copy, registers the DLL, **adds the Nepali
keyboard to your language list**, and recycles the input hosts — so
**Input by Prabidhi.bid** should already be in the taskbar / Win+Space switcher
(sign out / in if not). `rebuild-tsf.ps1` does the same for the dev loop.

**Manual (dev, register only):** PowerShell / cmd **as Administrator**:

```
regsvr32 "C:\Users\DELL\Desktop\input tool\target\release\xlit_tsf.dll"
```

then add the keyboard yourself: Settings → Time & Language → Language & region →
**Add a language** → Nepali (नेपाली) → Language options → Keyboards → add
**Input by Prabidhi.bid**.

At M6.1 typing still produces normal Latin — activation is only logged.

## Verify activation

Trace output is behind the `trace` feature (off by default so release /
installer builds stay quiet). Build with it on:

```
cargo build -p xlit-tsf --release --features trace
```

Then run [DebugView](https://learn.microsoft.com/sysinternals/downloads/debugview)
as admin (enable *Capture Global Win32*). Switching to the input method prints:

```
[xlit-tsf] Activate (client id N)
[xlit-tsf] Deactivate
```

## Uninstall (Administrator)

```
regsvr32 /u "C:\Users\DELL\Desktop\input tool\target\release\xlit_tsf.dll"
```

Remove the Nepali keyboard from Settings afterwards if you don't want it.

## Dev rebuild loop

Once the service is registered, Windows keeps `xlit_tsf.dll` loaded, so a plain
`cargo build` fails with *"unable to delete existing file"*. Use the helper — it
asks for elevation, unregisters, recycles the input-host processes, rebuilds, and
re-registers:

```
powershell -ExecutionPolicy Bypass -File frontends\windows-tsf\rebuild-tsf.ps1
```

Add `-SkipRegister` to build without reinstalling. Approve the UAC prompt as the
same user you log in with (see note in the script header).

If a process you can't close (the Claude desktop app, an open console) still maps
the DLL, the script **renames the old file aside** (`xlit_tsf.dll.<timestamp>.old`)
and builds a fresh one — the stale copy unloads on your next sign-out. It sweeps
those `.old` files on the next run.

## Troubleshooting: not in the taskbar switcher

- The switcher reads **your language list**, not the registry. `install.ps1` /
  the Setup.exe add `ne-NP` + this TIP with `Set-WinUserLanguageList`; if you
  registered by hand with `regsvr32`, add the keyboard yourself in Settings.
- `DllRegisterServer` uses `ITfInputProcessorProfileMgr::RegisterProfile`
  (enabled-by-default, machine-wide/HKLM) plus the `IMMERSIVESUPPORT` /
  `SYSTRAYSUPPORT` categories and does **not** claim `COMLESS`. An install from
  before those changes is stale — `regsvr32 /u` then `regsvr32` again, then sign
  out / in.
- `Set-WinUserLanguageList` writes per-user (HKCU); run it as the account you
  log in with (the installers self-elevate as the same user).
- The taskbar input indicator appears only with 2+ input methods; the built-in
  English keyboard plus this one is enough.

## Notes

- **x64 only** right now. 32-bit apps won't load it until we also ship an x86
  build.
- The DLL links `xlit-core` + the compiled-in seed dictionary directly
  (~0.4 MB). It will move to talking to `xlit-daemon` once M3 lands, so the
  engine data is loaded once instead of per-process.
- GUIDs (keep stable): CLSID `{438E43E4-3800-4AB1-82A6-A2E831ABF107}`,
  profile `{4BE59555-69DD-48CA-8BC8-AB450205A567}`, LANGID `0x0461`.

## Next (M6.2)

Key-event sink + inline composition: buffer ASCII letters as a TSF composition,
and on a break key (space / punctuation / Enter) replace the composition with
the engine's top candidate.
