<#
.SYNOPSIS
    Report where "Input by Prabidhi.bid" is in the install chain, and what on
    this machine might be stopping it.

.DESCRIPTION
    A TSF text service has to clear four separate hurdles before a keystroke
    reaches it, and a failure at any one of them looks identical from the
    outside ("I switched to it and nothing happened"). This checks each in
    order:

      1. the COM class is registered, per bitness, and points at a file
         that is actually on disk
      2. the TSF profile is registered
      3. the keyboard is in *your* language list (the switcher reads this,
         not the registry)
      4. the input hosts are running and the DLL can be loaded at all

    It then lists endpoint-protection software known to interfere with text
    services, because a TIP is a DLL that loads into every process that takes
    input - which is also what a keylogger does, so anti-keylogging features
    tend to block them.

    Read-only: it changes nothing.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File frontends\windows-tsf\installer\diagnose.ps1
#>
[CmdletBinding()]
param()

$Clsid   = '{438E43E4-3800-4AB1-82A6-A2E831ABF107}'
$Profile = '{4BE59555-69DD-48CA-8BC8-AB450205A567}'
$Tip     = "0461:$Clsid$Profile"
$AppName = 'Input by Prabidhi.bid'

$script:problems = @()

function Say([string]$msg, [string]$colour = 'Gray') { Write-Host $msg -ForegroundColor $colour }
function Ok  ([string]$msg) { Write-Host "  [ok]   $msg" -ForegroundColor Green }
function Bad ([string]$msg) {
    Write-Host "  [FAIL] $msg" -ForegroundColor Red
    $script:problems += $msg
}
function Note([string]$msg) { Write-Host "  [note] $msg" -ForegroundColor Yellow }

Say ''
Say "=== $AppName - install diagnosis ===" Cyan
Say ''

# --- 1. COM registration ------------------------------------------------------
Say '1. COM registration' Cyan

# On 64-bit Windows the 32-bit view lives under Wow6432Node. A TIP needs both:
# 64-bit apps load the first, 32-bit apps the second.
$views = @(
    [pscustomobject]@{ Name = 'x64'; Path = "HKLM:\SOFTWARE\Classes\CLSID\$Clsid\InprocServer32" }
    [pscustomobject]@{ Name = 'x86'; Path = "HKLM:\SOFTWARE\Classes\Wow6432Node\CLSID\$Clsid\InprocServer32" }
)
$anyDll = $false
foreach ($v in $views) {
    if (-not (Test-Path $v.Path)) {
        Note "$($v.Name): not registered"
        continue
    }
    $dll = (Get-ItemProperty $v.Path -ErrorAction SilentlyContinue).'(default)'
    if (-not $dll) {
        Bad "$($v.Name): registered but no DLL path"
    }
    elseif (Test-Path $dll) {
        $size = [int]((Get-Item $dll).Length / 1KB)
        Ok "$($v.Name): $dll ($size KB)"
        $anyDll = $true
    }
    else {
        Bad "$($v.Name): registered but the file is missing - $dll"
    }
}
if (-not $anyDll) { Bad 'no bitness is registered - run install.ps1' }

# --- 2. TSF profile -----------------------------------------------------------
Say ''
Say '2. TSF profile' Cyan
$tipKey = "HKLM:\SOFTWARE\Microsoft\CTF\TIP\$Clsid"
if (Test-Path $tipKey) {
    Ok 'profile registered under HKLM\SOFTWARE\Microsoft\CTF\TIP'
    $lang = Join-Path $tipKey "LanguageProfile\0x00000461\$Profile"
    if (Test-Path $lang) {
        $enabled = (Get-ItemProperty $lang -ErrorAction SilentlyContinue).Enable
        if ($null -eq $enabled -or $enabled -eq 1) { Ok 'Nepali profile enabled' }
        else { Bad 'Nepali profile is present but disabled (Enable = 0)' }
    }
    else { Bad 'Nepali (0x0461) language profile missing' }
}
else { Bad 'no TSF profile - DllRegisterServer did not complete' }

# --- 3. the user's language list ---------------------------------------------
Say ''
Say '3. Your language list (this is what the switcher reads)' Cyan
try {
    $list = Get-WinUserLanguageList
    $ne = $list | Where-Object { $_.LanguageTag -like 'ne*' }
    if (-not $ne) {
        Bad 'Nepali is not in your language list - the switcher cannot show the TIP'
    }
    elseif ($ne.InputMethodTips -contains $Tip) {
        Ok "Nepali present, with this input method ($Tip)"
    }
    else {
        Bad "Nepali present but this input method is not attached to it"
        Note "  it has: $($ne.InputMethodTips -join ', ')"
    }
    Say "  languages: $(($list | ForEach-Object LanguageTag) -join ', ')"
    if ($list.Count -lt 2) {
        Note 'only one language - the taskbar input indicator stays hidden; use Win+Space'
    }
}
catch { Bad "could not read the language list: $_" }

# --- 4. input hosts -----------------------------------------------------------
Say ''
Say '4. Input hosts' Cyan
foreach ($p in 'ctfmon', 'TextInputHost') {
    if (Get-Process -Name $p -ErrorAction SilentlyContinue) { Ok "$p running" }
    else { Note "$p not running (normal for TextInputHost until an input method is used)" }
}

# --- 5. can the DLL actually load? -------------------------------------------
Say ''
Say '5. Loading the DLL' Cyan
# regsvr32 re-running DllRegisterServer is the cheapest end-to-end proof that
# the file loads, its dependencies resolve, and nothing is blocking it.
if ($anyDll) {
    $x64 = (Get-ItemProperty $views[0].Path -ErrorAction SilentlyContinue).'(default)'
    if ($x64 -and (Test-Path $x64)) {
        $p = Start-Process -FilePath "$env:windir\System32\regsvr32.exe" `
            -ArgumentList '/s', "`"$x64`"" -Wait -PassThru -ErrorAction SilentlyContinue
        if ($p -and $p.ExitCode -eq 0) { Ok 'the x64 DLL loads and self-registers' }
        else { Bad "regsvr32 could not load the DLL (exit $($p.ExitCode)) - needs admin, or something is blocking it" }
    }
}
else { Note 'skipped - nothing registered' }

# --- 6. software known to block text services --------------------------------
Say ''
Say '6. Endpoint protection that can block a text service' Cyan
# A TIP is a DLL injected into every process that takes keyboard input. That is
# also the shape of a keylogger, so anti-keylogging and "privacy" features are
# the usual culprits when everything above is green but typing still does
# nothing.
$suspects = @(
    @{ Match = 'Dell*Privacy*';        What = 'Dell/Alienware Privacy (keyboard + camera protection)' }
    @{ Match = 'Dell*Optimizer*';      What = 'Dell Optimizer' }
    @{ Match = 'Dell*Trusted*';        What = 'Dell Trusted Device' }
    @{ Match = 'Dell*DataVault*';      What = 'Dell Data Vault' }
    @{ Match = 'Alienware*';           What = 'Alienware Command Center / services' }
    @{ Match = '*SupportAssist*';      What = 'Dell SupportAssist' }
    @{ Match = '*Kaspersky*';          What = 'Kaspersky (secure keyboard input)' }
    @{ Match = '*Norton*';             What = 'Norton' }
    @{ Match = '*McAfee*';             What = 'McAfee' }
    @{ Match = '*Bitdefender*';        What = 'Bitdefender' }
    @{ Match = '*ESET*';               What = 'ESET' }
)
$found = $false
$procs = Get-Process -ErrorAction SilentlyContinue
$svcs  = Get-Service -ErrorAction SilentlyContinue
foreach ($s in $suspects) {
    $hitP = $procs | Where-Object { $_.ProcessName -like $s.Match } | Select-Object -First 1
    $hitS = $svcs  | Where-Object { $_.Name -like $s.Match -or $_.DisplayName -like $s.Match } | Select-Object -First 1
    if ($hitP -or $hitS) {
        $found = $true
        $how = if ($hitP) { "process $($hitP.ProcessName)" } else { "service $($hitS.Name)" }
        Note "$($s.What) - $how"
    }
}
if (-not $found) { Ok 'nothing on the watch list is running' }

# Windows' own DLL-injection blocking, which unsigned in-proc servers can trip.
try {
    $cg = Get-CimInstance -ClassName Win32_DeviceGuard `
        -Namespace root\Microsoft\Windows\DeviceGuard -ErrorAction Stop
    if ($cg.CodeIntegrityPolicyEnforcementStatus -gt 0) {
        Note "Code Integrity enforcement is ON (status $($cg.CodeIntegrityPolicyEnforcementStatus)) - an unsigned DLL may be refused"
    }
}
catch {}

# --- summary ------------------------------------------------------------------
Say ''
if ($problems.Count -eq 0) {
    Say 'Everything the installer controls is in place.' Green
    Say 'If typing still does nothing, the block is at load time - see section 6,' Gray
    Say 'and check Event Viewer > Windows Logs > Application around the moment you typed.' Gray
}
else {
    Say "$($problems.Count) problem(s) found:" Red
    $problems | ForEach-Object { Say "  - $_" Red }
}
Say ''
