; Inno Setup 6.3+ script for "Input by Prabidhi.bid" (Windows TSF text service).
;
; A TSF text service loads into every text-input process, so on 64-bit Windows
; you ship BOTH a 64-bit and a 32-bit xlit_tsf.dll (32-bit apps load the 32-bit
; one from the WOW64 registry view). Layout:
;   64-bit OS:  {app}\xlit_tsf.dll        (x64, registered with System32\regsvr32)
;               {app}\x86\xlit_tsf.dll    (x86, registered with SysWOW64\regsvr32)
;   32-bit OS:  {app}\xlit_tsf.dll        (x86, registered with System32\regsvr32)
;
; Build via installer\build-setup.ps1, or directly:
;   iscc /DDllPathX64="...\x86_64-pc-windows-msvc\release\xlit_tsf.dll" ^
;        [/DDllPathX86="...\i686-pc-windows-msvc\release\xlit_tsf.dll"] ^
;        [/DAppVersion=0.1.0] [/DSetupPassword=<pw>] xlit-tsf.iss
;
; Each regsvr32 runs DllRegisterServer -> COM CLSID keys +
; ITfInputProcessorProfileMgr::RegisterProfile (enabled-by-default) +
; the TIPCAP_* categories. A runascurrentuser PowerShell step then adds
; ne-NP + this TIP to the user's language list so it shows in the switcher.

#ifndef DllPathX64
  #error Pass /DDllPathX64=<full path to the x64 xlit_tsf.dll>
#endif
#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif

#define AppName "Input by Prabidhi.bid"
#define AppPublisher "Prabidhi.bid"

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
UninstallDisplayIcon={app}\xlit_tsf.dll
; run on x86, x64 and arm64 (x86 emulation); use 64-bit dirs where available
ArchitecturesAllowed=x86compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
PrivilegesRequired=admin
OutputDir=.
OutputBaseFilename=Input-by-Prabidhi.bid-{#AppVersion}-setup
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern

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
; x64 DLL -> {app} (64-bit install mode only)
Source: "{#DllPathX64}"; DestDir: "{app}"; DestName: "xlit_tsf.dll"; \
  Check: Is64; Flags: ignoreversion restartreplace uninsrestartdelete 64bit
#ifdef DllPathX86
; x86 DLL -> {app}\x86 on 64-bit Windows, {app} on 32-bit Windows
Source: "{#DllPathX86}"; DestDir: "{app}\x86"; DestName: "xlit_tsf.dll"; \
  Check: Is64; Flags: ignoreversion restartreplace uninsrestartdelete 32bit
Source: "{#DllPathX86}"; DestDir: "{app}"; DestName: "xlit_tsf.dll"; \
  Check: Is32; Flags: ignoreversion restartreplace uninsrestartdelete 32bit
#endif

[Run]
; ----- deregister any prior copy (no-op on a fresh install) -----
Filename: "{sys}\regsvr32.exe"; Parameters: "/s /u ""{app}\xlit_tsf.dll"""; \
  Flags: runhidden waituntilterminated
#ifdef DllPathX86
Filename: "{syswow64}\regsvr32.exe"; Parameters: "/s /u ""{app}\x86\xlit_tsf.dll"""; \
  Check: Is64; Flags: runhidden waituntilterminated
#endif
; ----- register -----
Filename: "{sys}\regsvr32.exe"; Parameters: "/s ""{app}\xlit_tsf.dll"""; \
  StatusMsg: "Registering the input method..."; Flags: runhidden waituntilterminated
#ifdef DllPathX86
Filename: "{syswow64}\regsvr32.exe"; Parameters: "/s ""{app}\x86\xlit_tsf.dll"""; \
  StatusMsg: "Registering the input method (32-bit)..."; \
  Check: Is64; Flags: runhidden waituntilterminated
#endif
; ----- add the keyboard to the user's language list -----
Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; \
  Parameters: "-NoProfile -NonInteractive -ExecutionPolicy Bypass -WindowStyle Hidden -File ""{tmp}\ne-kbd-add.ps1"""; \
  StatusMsg: "Adding the Nepali keyboard to your language list..."; \
  Flags: runascurrentuser runhidden waituntilterminated

[UninstallRun]
Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; \
  Parameters: "-NoProfile -NonInteractive -ExecutionPolicy Bypass -WindowStyle Hidden -File ""{tmp}\ne-kbd-remove.ps1"""; \
  RunOnceId: "RemoveNeKeyboard"; Flags: runascurrentuser runhidden waituntilterminated
Filename: "{sys}\regsvr32.exe"; Parameters: "/s /u ""{app}\xlit_tsf.dll"""; \
  RunOnceId: "UnregNative"; Flags: runhidden waituntilterminated
#ifdef DllPathX86
Filename: "{syswow64}\regsvr32.exe"; Parameters: "/s /u ""{app}\x86\xlit_tsf.dll"""; \
  RunOnceId: "UnregX86"; Check: Is64; Flags: runhidden waituntilterminated
#endif

[Code]
const
  NeTag = 'ne-NP';
  // langid : CLSID + profile GUID -- must match register.rs / lib.rs
  NeTip = '0461:{438E43E4-3800-4AB1-82A6-A2E831ABF107}{4BE59555-69DD-48CA-8BC8-AB450205A567}';

function Is64: Boolean;
begin
  Result := Is64BitInstallMode;
end;

function Is32: Boolean;
begin
  Result := not Is64BitInstallMode;
end;

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

{ Re-install case: unregister an older copy (both bitnesses) and free the input
  hosts *before* [Files] tries to overwrite the locked DLL. }
procedure PreUnregisterIfInstalled;
var
  RootDll, X86Dll: String;
  ResultCode: Integer;
begin
  RootDll := ExpandConstant('{app}\xlit_tsf.dll');
  X86Dll  := ExpandConstant('{app}\x86\xlit_tsf.dll');
  if FileExists(RootDll) then
    Exec(ExpandConstant('{sys}\regsvr32.exe'), '/s /u "' + RootDll + '"',
      '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  if Is64BitInstallMode and FileExists(X86Dll) then
    Exec(ExpandConstant('{syswow64}\regsvr32.exe'), '/s /u "' + X86Dll + '"',
      '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  if FileExists(RootDll) or FileExists(X86Dll) then
    RecycleInputHosts;
end;

function InitializeSetup: Boolean;
begin
  Result := True;
#ifndef DllPathX86
  if not Is64BitInstallMode then
  begin
    MsgBox('This build has no 32-bit payload and cannot install on 32-bit Windows.',
      mbCriticalError, MB_OK);
    Result := False;
  end;
#endif
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
