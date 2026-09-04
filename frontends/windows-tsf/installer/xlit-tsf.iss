; Inno Setup 6.3+ script for "Input by Prabidhi.bid" (Windows TSF text service).
;
; Build via installer\build-setup.ps1, or directly:
;   iscc /DDllPath="..\..\..\target\release\xlit_tsf.dll" [/DAppVersion=0.1.0] ^
;        [/DSetupPassword=<pw>] xlit-tsf.iss
;
; Install flow (elevated via PrivilegesRequired=admin):
;   1. copy xlit_tsf.dll to {app}
;   2. regsvr32 /s  -> DllRegisterServer writes the COM CLSID keys, the
;      HKLM\SOFTWARE\Microsoft\CTF\TIP profile, EnableLanguageProfile, and the
;      TIPCAP_* categories (machine-wide)
;   3. as the *current* user (runascurrentuser): add "ne-NP" + this TIP to the
;      user's language list via Set-WinUserLanguageList, so it shows up in the
;      taskbar / Win+Space switcher without a manual Settings visit
;   4. recycle ctfmon / TextInputHost so the switcher refreshes
; Uninstall reverses 3 -> 2, then Inno removes the files.

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
Source: "{#DllPath}"; DestDir: "{app}"; Flags: ignoreversion 64bit restartreplace uninsrestartdelete

[Run]
; deregister any prior registration first, then register clean. On a fresh
; install the /u is a harmless no-op; on a re-install it clears stale state
; (old path, the removed COMLESS category, a leftover rebuild-tsf.ps1 reg).
Filename: "{sys}\regsvr32.exe"; Parameters: "/s /u ""{app}\{#DllName}"""; \
  Flags: runhidden waituntilterminated
Filename: "{sys}\regsvr32.exe"; Parameters: "/s ""{app}\{#DllName}"""; \
  StatusMsg: "Registering the input method..."; \
  Flags: runhidden waituntilterminated
Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; \
  Parameters: "-NoProfile -NonInteractive -ExecutionPolicy Bypass -WindowStyle Hidden -File ""{tmp}\ne-kbd-add.ps1"""; \
  StatusMsg: "Adding the Nepali keyboard to your language list..."; \
  Flags: runascurrentuser runhidden waituntilterminated

[UninstallRun]
Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; \
  Parameters: "-NoProfile -NonInteractive -ExecutionPolicy Bypass -WindowStyle Hidden -File ""{tmp}\ne-kbd-remove.ps1"""; \
  RunOnceId: "RemoveNeKeyboard"; Flags: runascurrentuser runhidden waituntilterminated
Filename: "{sys}\regsvr32.exe"; Parameters: "/s /u ""{app}\{#DllName}"""; \
  RunOnceId: "UnregisterXlitTsf"; Flags: runhidden waituntilterminated

[Code]
const
  NeTag = 'ne-NP';
  // langid : CLSID + profile GUID -- must match register.rs / lib.rs
  NeTip = '0461:{438E43E4-3800-4AB1-82A6-A2E831ABF107}{4BE59555-69DD-48CA-8BC8-AB450205A567}';

{ PowerShell that adds ne-NP + this TIP to the running user's language list. }
function AddScript: String;
begin
  Result :=
    '$ErrorActionPreference=''SilentlyContinue'';' + #13#10 +
    '$t=''' + NeTip + ''';' + #13#10 +
    '$l=Get-WinUserLanguageList;' + #13#10 +
    'if(-not ($l | Where-Object { $_.LanguageTag -eq ''' + NeTag + ''' })) { $l.Add(''' + NeTag + ''') }' + #13#10 +
    '$n=$l | Where-Object { $_.LanguageTag -eq ''' + NeTag + ''' };' + #13#10 +
    'if ($n -and ($n.InputMethodTips -notcontains $t)) { $n.InputMethodTips.Add($t) }' + #13#10 +
    'Set-WinUserLanguageList $l -Force';
end;

{ ...and removes it again (dropping ne-NP if nothing else is left under it). }
function RemoveScript: String;
begin
  Result :=
    '$ErrorActionPreference=''SilentlyContinue'';' + #13#10 +
    '$t=''' + NeTip + ''';' + #13#10 +
    '$l=Get-WinUserLanguageList;' + #13#10 +
    '$n=$l | Where-Object { $_.LanguageTag -eq ''' + NeTag + ''' };' + #13#10 +
    'if ($n) {' + #13#10 +
    '  [void]$n.InputMethodTips.Remove($t)' + #13#10 +
    '  if ($n.InputMethodTips.Count -eq 0) { [void]$l.Remove($n) }' + #13#10 +
    '  Set-WinUserLanguageList $l -Force' + #13#10 +
    '}';
end;

procedure WriteTmpScript(const FileName, Body: String);
begin
  SaveStringToFile(ExpandConstant('{tmp}\' + FileName), Body, False);
end;

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

{ Re-install case: if an older copy is already registered, unregister it and
  free the input hosts *before* [Files] tries to overwrite the locked DLL. }
procedure PreUnregisterIfInstalled;
var
  Dll: String;
  ResultCode: Integer;
begin
  Dll := ExpandConstant('{app}\{#DllName}');
  if FileExists(Dll) then
  begin
    Exec(ExpandConstant('{sys}\regsvr32.exe'), '/s /u "' + Dll + '"',
      '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
    RecycleInputHosts;
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssInstall then
  begin
    PreUnregisterIfInstalled;
    WriteTmpScript('ne-kbd-add.ps1', AddScript);
  end;
  if CurStep = ssPostInstall then
    RecycleInputHosts;
end;

function InitializeUninstall: Boolean;
begin
  WriteTmpScript('ne-kbd-remove.ps1', RemoveScript);
  Result := True;
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
      '"Input by Prabidhi.bid" is installed and added to your Nepali keyboard list.' + #13#10 +
      'Switch to it with the taskbar language button or Win+Space.' + #13#10 +
      'If it is not shown yet, sign out and back in.';
end;
