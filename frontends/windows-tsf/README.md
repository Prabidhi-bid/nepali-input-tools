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

**MSI (recommended):** build `installer\Input-by-Prabidhi.bid-<ver>-x64.msi`
with [`installer\build-msi.ps1`](installer/build-msi.ps1) and double-click it.
See [`installer/README.md`](installer/README.md).

**Manual (dev):** run PowerShell / cmd **as Administrator**:

```
regsvr32 "C:\Users\DELL\Desktop\input tool\target\release\xlit_tsf.dll"
```

A success dialog means the COM keys + TSF profile + keyboard categories were
written. Either way, then add the input method:

1. Settings → Time & Language → Language & region → **Add a language** → Nepali
   (नेपाली). (You only need the language entry; no display pack required.)
2. Under Nepali → Language options → Keyboards, you should see
   **Input by Prabidhi.bid**.
3. Switch to it with the taskbar language button (or Win+Space).

At M6.1 typing still produces normal Latin — activation is only logged.

## Verify activation

Run [DebugView](https://learn.microsoft.com/sysinternals/downloads/debugview)
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

- The profile only shows once **Nepali is in your language list** (step 1) — the
  TIP is bound to LANGID `0x0461`, so with no ne-NP entry there is nothing to
  attach to.
- `DllRegisterServer` now calls `EnableLanguageProfile` and registers the
  `TIPCAP_IMMERSIVESUPPORT` / `SYSTRAYSUPPORT` categories; an install from before
  that change is stale. Re-run `regsvr32 /u ...` then `regsvr32 ...`, or just
  `regsvr32 ...` again to rewrite the keys, then sign out / in.
- `EnableLanguageProfile` writes per-user (HKCU) state. Run `regsvr32` elevated
  **as the same user** you log in as; a separate "Administrator" account enables
  it only for that account.
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
