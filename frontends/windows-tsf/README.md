# xlit-tsf — Windows text input processor

A COM in-process DLL that plugs the `xlit` engine into Windows' Text Services
Framework, so you can type Nepali phonetically in any application.

## Status: M6.3 — it types, with a candidate list

The DLL registers as an input method ("Input by Prabidhi.bid"), and once you
switch to it, letters are converted inline:

| Key | Effect |
|-----|--------|
| `a`–`z`, `A`–`Z` | extend the word; the composition shows the converted form as you type |
| Space | commit, then insert the space |
| `.` `,` `?` … | commit, then insert that character |
| Enter / Tab | commit (no newline or tab is inserted — press again for that) |
| Backspace | drop one Latin letter and re-convert |
| Esc | give up on the conversion, leave the raw Latin |
| `1`–`9` | commit that numbered candidate |
| ↑ / ↓ | move through the candidates |
| **Ctrl+Space** | toggle between Nepali and plain Latin without switching input method |

Type `namaste` and you see नमस्ते build up underlined; Space commits it. The
case matters — the schema uses `M` for anusvara, `T` for ट, `S` for श — so
`kaaThamaaDauM` gives काठमाडौं.

Candidates are ranked by the same engine the CLI uses (rule → dictionary →
learning), so `nepaali` corrects to नेपाली and `hello` gives हेलो. Picking a
numbered candidate is remembered in `%APPDATA%\xlit\xlit-learn.json` and floats
that choice to the top next time.

A popup lists the candidates under the word as you type, with the current one
highlighted. It appears only when there is a real choice to make (two or more
candidates) and only once the control has told us where the caret is — a list
stranded in the corner of the screen would be worse than none, so in a control
that cannot answer, typing still works and the list simply stays hidden.

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

## Verify activation

Trace output is behind the `trace` feature (off by default so release /
installer builds stay quiet). Build with it on:

```
cargo build -p xlit-tsf --release --features trace
```

Then run [DebugView](https://learn.microsoft.com/sysinternals/downloads/debugview)
as admin (enable *Capture Global Win32*). Switching to the input method and
typing prints:

```
[xlit-tsf] Activate (client id N)
[xlit-tsf] engine ready
[xlit-tsf] Deactivate
```

`render failed: ...` there means the focused control refused a composition —
see *Troubleshooting* below.

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

## Troubleshooting: a blocked sign on the input method

Windows draws a prohibition sign over a text service it could not **load** or
whose **`Activate` returned an error**. It never says which, so work down:

```
powershell -ExecutionPolicy Bypass -File frontends\windows-tsf\installer\diagnose.ps1
```

That checks the COM registration per bitness, the TSF profile, your language
list, and whether the DLL still loads. Two causes are already designed out:

- **A missing runtime.** The DLL is linked with a static CRT
  (`.cargo/config.toml`), so it needs no `VCRUNTIME140.dll` in the host
  process — only core Windows DLLs. Confirm with
  `dumpbin /dependents xlit_tsf.dll`.
- **A failing `Activate`.** Every step of activation is best-effort and
  logged; it cannot return an error. If the key sink is refused, the input
  method still loads and types plain Latin, and the trace build says so.

If the sign persists, build with `--features trace` and watch DebugView while
switching to the input method.

## Troubleshooting: typing does nothing, or stays Latin

- **Nothing appears at all.** The control refused the composition. We report the
  letter as unhandled in that case, so you should still get plain Latin rather
  than a dead keyboard — if you get neither, build `--features trace` and look
  for `render failed:` in DebugView.
- **Latin comes out instead of Devanagari.** Either the input method is not the
  active one (check the taskbar indicator, Win+Space), or Ctrl+Space has toggled
  it to passthrough — press Ctrl+Space again.
- **Ctrl+Space does nothing.** Another text service already reserved the chord;
  the trace log says `toggle key unavailable`. Nothing else breaks.
- **A 32-bit app types Latin while 64-bit apps work** (or vice versa): only one
  bitness is registered. Re-run `install.ps1` without `-SkipX86`.

## Notes

- **x86 + x64.** A TSF DLL loads into every text-input process, so 64-bit
  Windows needs both a 64-bit and a 32-bit `xlit_tsf.dll` (32-bit apps load the
  32-bit one via the WOW64 registry view). The installer builds and registers
  both — `cargo build -p xlit-tsf --release --target x86_64-pc-windows-msvc` and
  `--target i686-pc-windows-msvc` (`rustup target add i686-pc-windows-msvc`; the
  i686 build also needs the x86 MSVC toolchain). ARM64: the x64 + x86 payloads
  run under emulation; no native ARM64 build yet.
- The DLL links `xlit-core`, `xlit-dict` and `xlit-learn` with the compiled-in
  seed dictionary directly (~430 KB). It will move to talking to `xlit-daemon`
  once M3 lands, so the engine data is loaded once instead of per-process.
- **Every key we claim is gone for good.** TSF asks `OnTestKeyDown` first and
  only calls `OnKeyDown` if that said yes; a key claimed there never reaches the
  application, even if `OnKeyDown` then declines it. That is why committing on
  Space re-inserts the space itself instead of letting it through.
- GUIDs (keep stable): CLSID `{438E43E4-3800-4AB1-82A6-A2E831ABF107}`,
  profile `{4BE59555-69DD-48CA-8BC8-AB450205A567}`, LANGID `0x0461`.

## Next (M6.4)

A language-bar icon showing (and toggling) Nepali vs passthrough, and whatever
Chrome / Electron / UWP quirks testing turns up.
