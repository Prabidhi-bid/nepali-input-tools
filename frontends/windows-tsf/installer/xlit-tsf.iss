; Inno Setup 6.3+ script for "Input by Prabidhi.bid" (Windows TSF text service).
;
; Build via installer\build-setup.ps1, or directly:
;   iscc /DDllPath="..\..\..\target\release\xlit_tsf.dll" [/DAppVersion=0.1.0] ^
;        [/DSetupPassword=<pw>] xlit-tsf.iss
;
; Registration is delegated to the DLL's own DllRegisterServer via regsvr32
; (it writes the COM CLSID keys + the HKLM\SOFTWARE\Microsoft\CTF\TIP profile,
; EnableLanguageProfile, and the TIPCAP_* categories). Inno itself creates the
; Apps & features entry and removes files on uninstall.

#ifndef DllPath
  #error Pass /DDllPath=<full path to xlit_tsf.dll>
#endif
#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif

#define AppName "Input by Prabidhi.bid"
#define AppPublisher "Prabidhi.bid"
#define DllName "xlit_tsf.dll"

[Setup]
; keep this GUID stable across releases (drives upgrade + uninstall)
AppId={{9C4B7A21-8E33-4D5F-A1B0-2F6E9C3D74A5}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
DefaultDirName={autopf}\Prabidhi.bid Input
DisableDirPage=yes
DisableProgramGroupPage=yes
UninstallDisplayName={#AppName}
UninstallDisplayIcon={app}\{#DllName}
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
PrivilegesRequired=admin
OutputDir=.
OutputBaseFilename=Input-by-Prabidhi.bid-{#AppVersion}-setup
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern

; --- optional: encrypt the embedded payload (speed-bump against innounp/7z) ---
#ifdef SetupPassword
Encryption=yes
Password={#SetupPassword}
#endif

; --- optional: Authenticode signing (the real anti-tamper measure) ---
; Register a signer once (Tools > Configure Sign Tools in the Inno IDE, or):
;   iscc /Sstandard="\"C:\...\signtool.exe\" sign /fd sha256 /a ^
;        /tr http://timestamp.digicert.com /td sha256 $f" xlit-tsf.iss
; then uncomment:
; SignTool=standard
; SignedUninstaller=yes

[Files]
Source: "{#DllPath}"; DestDir: "{app}"; Flags: ignoreversion 64bit

[Run]
Filename: "{sys}\regsvr32.exe"; Parameters: "/s ""{app}\{#DllName}"""; \
  StatusMsg: "Registering the input method..."; \
  Flags: runhidden waituntilterminated

[UninstallRun]
Filename: "{sys}\regsvr32.exe"; Parameters: "/s /u ""{app}\{#DllName}"""; \
  RunOnceId: "UnregisterXlitTsf"; Flags: runhidden waituntilterminated

[Code]
{ Restart the input-method hosts so the new/removed profile is picked up
  without a full sign-out. }
procedure RecycleInputHosts;
var
  ResultCode: Integer;
begin
  Exec(ExpandConstant('{cmd}'),
    '/c taskkill /f /im ctfmon.exe >nul 2>&1 & taskkill /f /im TextInputHost.exe >nul 2>&1',
    '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
    RecycleInputHosts;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
    RecycleInputHosts;
end;

procedure CurPageChanged(CurPageID: Integer);
begin
  if CurPageID = wpFinished then
    WizardForm.FinishedLabel.Caption :=
      'Registered.  Add Nepali under Settings > Time & language > Language & region,' + #13#10 +
      'then pick "Input by Prabidhi.bid" from the taskbar language button (Win+Space).' + #13#10 +
      'Sign out / in if it is not listed yet.';
end;
