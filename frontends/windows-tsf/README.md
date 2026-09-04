# xlit-tsf — Windows text input processor

A COM in-process DLL that plugs the `xlit` engine into Windows' Text Services
Framework, so you can type Nepali phonetically in any application.

## Status: M6.1 — registrable no-op

The DLL registers as an input method ("xlit Nepali (phonetic)") and can be
activated without crashing. It does **not** intercept keys yet — that's M6.2.
Use this stage only to confirm install / activate / uninstall work on your
machine.

## Build

```
cargo build -p xlit-tsf --release
```

Output: `target\release\xlit_tsf.dll` (x64).

## Install  (Run PowerShell / cmd **as Administrator**)

```
regsvr32 "C:\Users\DELL\Desktop\input tool\target\release\xlit_tsf.dll"
```

A success dialog means the COM keys + TSF profile + keyboard category were
written. Then add the input method:

1. Settings → Time & Language → Language & region → **Add a language** → Nepali
   (नेपाली). (You only need the language entry; no display pack required.)
2. Under Nepali → Language options → Keyboards, you should see
   **xlit Nepali (phonetic)**.
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
