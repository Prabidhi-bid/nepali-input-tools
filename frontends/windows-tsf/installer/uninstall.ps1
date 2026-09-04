<#
.SYNOPSIS
    Uninstall "Input by Prabidhi.bid" - completely reverses install.ps1.
    Self-elevates. Invoked by the "Apps & features" entry, or run directly.

.DESCRIPTION
    Reverses, in order:
      1. Set-WinUserLanguageList  - remove this TIP from the user's language
         list (and ne-NP itself if nothing else is left under it)
      2. regsvr32 /s /u  for each installed bitness -> DllUnregisterServer
         removes the COM CLSID keys + the CTF\TIP profile and categories
      3. registry fallback - force-delete the CLSID / Wow6432Node\CLSID /
         CTF\TIP\{CLSID} keys in case a DLL could not be loaded to self-unregister
      4. HKCU CTF sweep - drop any cached Assemblies / SortOrder entries that
         still name our CLSID
      5. remove the Apps & features entry
      6. delete %ProgramFiles%\Prabidhi.bid Input\  (locked files are renamed
         aside and swept by a detached rmdir + on the next sign-out)
#>
[CmdletBinding()]
param([switch]$Elevated)
$ErrorActionPreference = 'Stop'

$AppName     = 'Input by Prabidhi.bid'
$InstallDir  = Join-Path $env:ProgramFiles 'Prabidhi.bid Input'
$UninstallRK = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\PrabidhibidInput'
$HostProcs   = @('ctfmon', 'TextInputHost')
$Clsid       = '{438E43E4-3800-4AB1-82A6-A2E831ABF107}'
$Tip         = '0461:{438E43E4-3800-4AB1-82A6-A2E831ABF107}{4BE59555-69DD-48CA-8BC8-AB450205A567}'

function Test-Admin {
    ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
    ).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

# --- run from a temp copy so we can delete our own install directory ---------
if ($PSCommandPath -like "$InstallDir*") {
    $tmp = Join-Path $env:TEMP "prabidhibid-uninstall-$PID.ps1"
    Copy-Item $PSCommandPath $tmp -Force
    $argv = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$tmp`"")
    if ($Elevated) { $argv += '-Elevated' }
    Start-Process powershell.exe -ArgumentList $argv -WindowStyle Hidden
    exit 0
}

# --- ask for elevation -----------------------------------------------------------
if (-not (Test-Admin)) {
    Write-Host 'Requesting administrator elevation...' -ForegroundColor Cyan
    $argv = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$PSCommandPath`"", '-Elevated')
    try { Start-Process powershell.exe -ArgumentList $argv -Verb RunAs }
    catch { Write-Host 'Elevation was cancelled or denied.' -ForegroundColor Red; exit 1 }
    exit 0
}

# native (64-bit) System32 even if this happens to be a 32-bit PowerShell
$sys = Join-Path $env:windir 'System32'
if ([Environment]::Is64BitOperatingSystem -and -not [Environment]::Is64BitProcess) {
    $sys = Join-Path $env:windir 'Sysnative'
}
$RegsvrNative = Join-Path $sys 'regsvr32.exe'
$Regsvr32bit  = Join-Path $env:windir 'SysWOW64\regsvr32.exe'

# regsvr32.exe is GUI-subsystem, so PowerShell's call operator returns before
# it has done anything. Without waiting, the registry sweep below can run while
# the DLL is still unregistering and put the keys back.
function Invoke-Regsvr32 {
    param([string]$Exe, [string]$Dll, [switch]$Unregister)
    $argv = @('/s')
    if ($Unregister) { $argv += '/u' }
    $argv += "`"$Dll`""
    try {
        $p = Start-Process -FilePath $Exe -ArgumentList $argv -Wait -PassThru -WindowStyle Hidden
        return $p.ExitCode
    } catch { return -1 }
}

try {
    # 1. remove from the user's language list
    try {
        $list = Get-WinUserLanguageList
        $ne = $list | Where-Object { $_.LanguageTag -eq 'ne-NP' }
        if ($ne) {
            foreach ($m in @($ne.InputMethodTips | Where-Object { $_ -ieq $Tip })) {
                [void]$ne.InputMethodTips.Remove($m)
            }
            if ($ne.InputMethodTips.Count -eq 0) { [void]$list.Remove($ne) }
            Set-WinUserLanguageList $list -Force
        }
    } catch { Write-Warning "language list: $_" }

    # 2. self-unregister each installed bitness
    $rootDll = Join-Path $InstallDir 'xlit_tsf.dll'
    $x86Dll  = Join-Path $InstallDir 'x86\xlit_tsf.dll'
    if (Test-Path $rootDll) {
        Write-Host "==> regsvr32 /s /u (native)" -ForegroundColor Cyan
        Invoke-Regsvr32 -Exe $RegsvrNative -Dll $rootDll -Unregister | Out-Null
    }
    if ((Test-Path $x86Dll) -and (Test-Path $Regsvr32bit)) {
        Write-Host "==> regsvr32 /s /u (x86)" -ForegroundColor Cyan
        Invoke-Regsvr32 -Exe $Regsvr32bit -Dll $x86Dll -Unregister | Out-Null
    }

    # 3. registry fallback (covers a missing/unloadable DLL)
    foreach ($k in @(
            "HKLM:\SOFTWARE\Classes\CLSID\$Clsid",
            "HKLM:\SOFTWARE\Classes\Wow6432Node\CLSID\$Clsid",
            "HKLM:\SOFTWARE\Microsoft\CTF\TIP\$Clsid")) {
        Remove-Item $k -Recurse -Force -ErrorAction SilentlyContinue
    }

    # 4. HKCU CTF sweep - cached assembly/sort entries that still name our CLSID
    foreach ($root in @(
            'HKCU:\SOFTWARE\Microsoft\CTF\Assemblies',
            'HKCU:\SOFTWARE\Microsoft\CTF\SortOrder\AssemblyItem')) {
        if (-not (Test-Path $root)) { continue }
        $hits = foreach ($key in (Get-ChildItem $root -Recurse -ErrorAction SilentlyContinue)) {
            try {
                if ($key.PSChildName -like "*$Clsid*") { $key.PSPath; continue }
                foreach ($v in $key.Property) {
                    if ("$($key.GetValue($v))" -like "*$Clsid*") { $key.PSPath; break }
                }
            } catch {}
        }
        foreach ($h in ($hits | Select-Object -Unique)) {
            Remove-Item $h -Recurse -Force -ErrorAction SilentlyContinue
        }
    }

    foreach ($p in $HostProcs) { Stop-Process -Name $p -Force -ErrorAction SilentlyContinue }
    Start-Sleep -Milliseconds 500

    # 5. Apps & features entry
    Remove-Item $UninstallRK -Recurse -Force -ErrorAction SilentlyContinue

    # 6. the install directory
    if (Test-Path $InstallDir) {
        try {
            Remove-Item $InstallDir -Recurse -Force -ErrorAction Stop
            Write-Host "Removed $InstallDir" -ForegroundColor DarkGray
        } catch {
            # Rename-Item takes a bare name, not a path. A mapped DLL usually
            # renames even though it cannot be deleted, which frees the name.
            Get-ChildItem $InstallDir -Recurse -Filter *.dll -ErrorAction SilentlyContinue | ForEach-Object {
                try {
                    Rename-Item -LiteralPath $_.FullName -NewName "$($_.Name).$(Get-Date -Format yyyyMMddHHmmss).old" -Force -ErrorAction Stop
                } catch {}
            }
            Start-Process cmd.exe -WindowStyle Hidden -ArgumentList `
                '/c', "timeout /t 3 /nobreak >nul & rmdir /s /q `"$InstallDir`""
            Write-Host 'Some files are in use; they are removed after you sign out / in.' -ForegroundColor Yellow
        }
    }

    Write-Host ''
    Write-Host "Uninstalled `"$AppName`"." -ForegroundColor Green
    $code = 0
}
catch {
    Write-Host ''
    Write-Host "ERROR: $_" -ForegroundColor Red
    $code = 1
}

# tidy the temp copy of this script, if we are running from one
if ($PSCommandPath -like "$env:TEMP*") {
    Start-Process cmd.exe -WindowStyle Hidden -ArgumentList `
        '/c', "timeout /t 2 /nobreak >nul & del /f /q `"$PSCommandPath`""
}

if ($Elevated) { Write-Host ''; Read-Host 'Press Enter to close' }
exit $code
