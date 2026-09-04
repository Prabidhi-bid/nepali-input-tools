<#
.SYNOPSIS
    Uninstall "Input by Prabidhi.bid". Self-elevates. Invoked by the
    "Apps & features" entry, or run directly.

.DESCRIPTION
    Unregisters the text service (regsvr32 /u -> DllUnregisterServer, which
    removes the COM + CTF\TIP keys), removes the Apps & features entry, and
    deletes %ProgramFiles%\Prabidhi.bid Input\. If the DLL is still mapped by a
    running process the file removal is deferred to after the script exits (and
    completes fully on the next sign-out).
#>
[CmdletBinding()]
param([switch]$Elevated)
$ErrorActionPreference = 'Stop'

$AppName     = 'Input by Prabidhi.bid'
$InstallDir  = Join-Path $env:ProgramFiles 'Prabidhi.bid Input'
$UninstallRK = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\PrabidhibidInput'
$HostProcs   = @('ctfmon', 'TextInputHost')

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
    Start-Process powershell.exe -ArgumentList $argv
    exit 0
}

# --- ask for elevation -----------------------------------------------------------
if (-not (Test-Admin)) {
    Write-Host 'Requesting administrator elevation...' -ForegroundColor Cyan
    $argv = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$PSCommandPath`"", '-Elevated')
    try { Start-Process powershell.exe -ArgumentList $argv -Verb RunAs }
    catch { Write-Error 'Elevation was cancelled or denied.'; exit 1 }
    exit 0
}

try {
    $dll = Join-Path $InstallDir 'xlit_tsf.dll'
    if (Test-Path $dll) {
        Write-Host "==> regsvr32 /s /u xlit_tsf.dll" -ForegroundColor Cyan
        & regsvr32.exe /s /u $dll
    }
    foreach ($p in $HostProcs) { Stop-Process -Name $p -Force -ErrorAction SilentlyContinue }
    Start-Sleep -Milliseconds 500

    Remove-Item $UninstallRK -Recurse -Force -ErrorAction SilentlyContinue

    if (Test-Path $InstallDir) {
        try {
            Remove-Item $InstallDir -Recurse -Force -ErrorAction Stop
            Write-Host "Removed $InstallDir" -ForegroundColor DarkGray
        } catch {
            # DLL still mapped - rename aside, then let a detached cmd sweep the
            # folder once this process (and its lock on uninstall.ps1) is gone.
            Get-ChildItem $InstallDir -Filter *.dll -ErrorAction SilentlyContinue | ForEach-Object {
                try { Rename-Item $_.FullName "$($_.FullName).$(Get-Date -Format yyyyMMddHHmmss).old" -Force } catch {}
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
    Write-Error $_
    $code = 1
}

if ($Elevated) { Write-Host ''; Read-Host 'Press Enter to close' }
exit $code
