# Installer — `Input by Prabidhi.bid` (MSI)

WiX v4/v5 authoring for a per-machine MSI that installs `xlit_tsf.dll` and
registers it as a Windows text service.

## Prerequisites (one time)

```
winget install Microsoft.DotNet.SDK.8
dotnet tool install --global wix
```

Any .NET SDK ≥ 6 works. The build script adds `WixToolset.UI.wixext` itself.

## Build

```
powershell -ExecutionPolicy Bypass -File frontends\windows-tsf\installer\build-msi.ps1
```

Options: `-Version 0.2.0`, `-Configuration debug`. Output:
`installer\Input-by-Prabidhi.bid-<version>-x64.msi` (the DLL is embedded in the
MSI's cab).

## Install / uninstall

| | command |
|---|---|
| install (wizard) | double-click the `.msi`, accept the UAC prompt |
| install (silent) | `msiexec /i "Input-by-Prabidhi.bid-0.1.0-x64.msi" /qn` |
| uninstall | `msiexec /x "Input-by-Prabidhi.bid-0.1.0-x64.msi" /qn` — or Apps & features |
| log | add `/l*v install.log` |

The MSI puts the DLL in `%ProgramFiles%\Prabidhi.bid Input\` and runs
`regsvr32` on it (which triggers the DLL's `DllRegisterServer`). After
install: add Nepali under **Settings → Time & Language → Language & region**,
then pick **Input by Prabidhi.bid** from the taskbar language button / `Win+Space`.

## What it does

- One component: `xlit_tsf.dll` (x64), key-path.
- `RegisterTsf` custom action (`regsvr32 /s`) after `InstallFiles`.
- `UnregisterTsf` (`regsvr32 /s /u`) before `RemoveFiles` on uninstall,
  skipped during a major upgrade (`UPGRADINGPRODUCTCODE`).
- `MajorUpgrade` scheduled `afterInstallInitialize`, so upgrading removes the
  old files first, then the new install re-registers.
- `WixUI_Minimal` (license + progress). License text: `License.rtf`.

## Limitations / TODO

- **Run elevated.** The register/unregister actions are immediate CAs, so they
  need an elevated context — double-clicking the MSI (UAC) or `msiexec` from an
  elevated prompt. A plain `msiexec /i` from a non-elevated shell won't have
  rights for the `HKLM` writes. Moving these to deferred `no-impersonate` CAs
  (via `WixToolset.Util.wixext`'s `QuietExec`) is the proper fix.
- **Per-user enable.** `DllRegisterServer` calls `EnableLanguageProfile`, which
  writes `HKCU`. Under the MSI that lands in the installing user's hive only;
  other users may need to enable the keyboard once from language settings. An
  `ActiveSetup` stub that runs a per-user enable on first logon is the fix.
- **x64 only.** No x86/ARM64 payload yet, so 32-bit apps won't load the TIP.
- **DLL locked while registered.** If a rebuild/reinstall fails to replace the
  file, sign out and back in (or use `..\rebuild-tsf.ps1` for dev iteration).
- No code signing — SmartScreen will warn on the `.msi`.
