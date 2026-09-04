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

- **`Setup.exe`** — `powershell -File installer\build-setup.ps1` (needs
  `winget install JRSoftware.InnoSetup`) → a distributable
  `installer\Input-by-Prabidhi.bid-<ver>-setup.exe`.
- **`installer\install.ps1`** — self-elevating, no toolchain:

```
powershell -ExecutionPolicy Bypass -File frontends\windows-tsf\installer\install.ps1
```

Either builds x64 + x86, deregisters any prior copy, registers both DLLs,
**adds the Nepali keyboard to your language list**, and recycles the input hosts
— so **Input by Prabidhi.bid** should already be in the taskbar / Win+Space
switcher (sign out / in if not). `-SkipX86` for a faster x64-only run;
[`uninstall.ps1`](installer/uninstall.ps1) / the Setup.exe uninstaller reverse
it. See [`installer/README.md`](installer/README.md).

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
`cargo build` fails with *"unable to delete existing file"*. `install.ps1`
handles that (it deregisters, recycles the input hosts, and renames a still-
mapped DLL aside before rebuilding), so re-run it after each change —
`-SkipX86` keeps it to a single x64 build:

```
powershell -ExecutionPolicy Bypass -File frontends\windows-tsf\installer\install.ps1 -SkipX86
```

The stale mapped copy in apps you can't close (the Claude app, a console)
unloads on your next sign-out; leftover `xlit_tsf.dll.<timestamp>.old` files are
swept on the next run.

## Troubleshooting: not in the taskbar switcher

- The switcher reads **your language list**, not the registry. `install.ps1`
  adds `ne-NP` + this TIP with `Set-WinUserLanguageList`; if you registered by
  hand with `regsvr32`, add the keyboard yourself in Settings.
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

- **x86 + x64.** A TSF DLL loads into every text-input process, so 64-bit
  Windows needs both a 64-bit and a 32-bit `xlit_tsf.dll` (32-bit apps load the
  32-bit one via the WOW64 registry view). The installer builds and registers
  both — `cargo build -p xlit-tsf --release --target x86_64-pc-windows-msvc` and
  `--target i686-pc-windows-msvc` (`rustup target add i686-pc-windows-msvc`; the
  i686 build also needs the x86 MSVC toolchain). ARM64: the x64 + x86 payloads
  run under emulation; no native ARM64 build yet.
- The DLL links `xlit-core` + the compiled-in seed dictionary directly
  (~0.4 MB). It will move to talking to `xlit-daemon` once M3 lands, so the
  engine data is loaded once instead of per-process.
- GUIDs (keep stable): CLSID `{438E43E4-3800-4AB1-82A6-A2E831ABF107}`,
  profile `{4BE59555-69DD-48CA-8BC8-AB450205A567}`, LANGID `0x0461`.

## Next (M6.2)

Key-event sink + inline composition: buffer ASCII letters as a TSF composition,
and on a break key (space / punctuation / Enter) replace the composition with
the engine's top candidate.
