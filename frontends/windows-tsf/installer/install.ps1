<#
.SYNOPSIS
    Install "Input by Prabidhi.bid" (Windows TSF text service). No installer
    toolchain - plain PowerShell. Self-elevates.

.DESCRIPTION
    Builds xlit_tsf.dll (unless -NoBuild), copies it to
    %ProgramFiles%\Prabidhi.bid Input\, registers it with regsvr32, and adds an
    "Apps & features" entry that points back at uninstall.ps1.

    If a running process (the Claude app, a console) still maps an older copy of
    the DLL, the locked file is renamed aside so a fresh one can be written; the
    stale copy unloads on the next sign-out.

.PARAMETER NoBuild
    Skip the cargo build; install a prebuilt xlit_tsf.dll placed next to this
    script, or the existing target\<cfg>\xlit_tsf.dll.

.PARAMETER Configuration
    release (default) or debug.

.PARAMETER Elevated
    Internal - set on the relaunched elevated instance so it pauses on exit.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File frontends\windows-tsf\installer\install.ps1
#>
[CmdletBinding()]
param(
    [switch]$NoBuild,
    [ValidateSet('release', 'debug')][string]$Configuration = 'release',
    [switch]$Elevated
)
$ErrorActionPreference = 'Stop'

$AppName     = 'Input by Prabidhi.bid'
$AppVersion  = '0.1.0'
$Publisher   = 'Prabidhi.bid'
$InstallDir  = Join-Path $env:ProgramFiles 'Prabidhi.bid Input'
$UninstallRK = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\PrabidhibidInput'
$HostProcs   = @('ctfmon', 'TextInputHost')

function Test-Admin {
    ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
    ).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

# --- ask for elevation first --------------------------------------------------
if (-not (Test-Admin)) {
    Write-Host 'Requesting administrator elevation...' -ForegroundColor Cyan
    $argv = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$PSCommandPath`"",
        '-Elevated', '-Configuration', $Configuration)
    if ($NoBuild) { $argv += '-NoBuild' }
    try { Start-Process powershell.exe -ArgumentList $argv -Verb RunAs }
    catch { Write-Error 'Elevation was cancelled or denied.'; exit 1 }
    exit 0
}

# --- elevated from here -----------------------------------------------------------
$here     = $PSScriptRoot
$repoRoot = (Resolve-Path (Join-Path $here '..\..\..')).Path

function Clear-Dll([string]$path) {
    if (-not (Test-Path $path)) { return }
    try {
        Remove-Item $path -Force -ErrorAction Stop
    } catch {
        $aside = "$path.$(Get-Date -Format yyyyMMddHHmmss).old"
        Rename-Item $path $aside -Force
        Write-Host "    (in use - renamed aside: $(Split-Path $aside -Leaf))" -ForegroundColor DarkGray
    }
}

try {
    $targetDll = Join-Path $repoRoot "target\$Configuration\xlit_tsf.dll"
    $bundled   = Join-Path $here 'xlit_tsf.dll'

    # 1. get a fresh DLL
    if (-not $NoBuild) {
        Write-Host "==> cargo build -p xlit-tsf ($Configuration)" -ForegroundColor Cyan
        & regsvr32.exe /s /u $targetDll 2>$null   # drop any in-place registration
        foreach ($p in $HostProcs) { Stop-Process -Name $p -Force -ErrorAction SilentlyContinue }
        Start-Sleep -Milliseconds 500
        Clear-Dll $targetDll
        Push-Location $repoRoot
        try {
            $flags = @('build', '-p', 'xlit-tsf')
            if ($Configuration -eq 'release') { $flags += '--release' }
            & cargo @flags
            if ($LASTEXITCODE) { throw "cargo build failed ($LASTEXITCODE)" }
        } finally { Pop-Location }
        $src = $targetDll
    }
    elseif (Test-Path $bundled)   { $src = $bundled }
    elseif (Test-Path $targetDll) { $src = $targetDll }
    else { throw "no xlit_tsf.dll found. Build it (cargo build -p xlit-tsf --release) or drop one next to this script, or run without -NoBuild." }

    # 2. copy into Program Files
    Write-Host "==> installing to $InstallDir" -ForegroundColor Cyan
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    $destDll = Join-Path $InstallDir 'xlit_tsf.dll'
    & regsvr32.exe /s /u $destDll 2>$null         # unregister a previous install
    Clear-Dll $destDll
    Copy-Item $src $destDll -Force
    Copy-Item (Join-Path $here 'uninstall.ps1') (Join-Path $InstallDir 'uninstall.ps1') -Force

    # 3. register
    Write-Host "==> regsvr32 /s xlit_tsf.dll" -ForegroundColor Cyan
    & regsvr32.exe /s $destDll
    if ($LASTEXITCODE) { throw "regsvr32 failed ($LASTEXITCODE)" }

    # 4. Apps & features entry
    $uninstCmd = "powershell.exe -NoProfile -ExecutionPolicy Bypass -File `"$(Join-Path $InstallDir 'uninstall.ps1')`""
    New-Item -Path $UninstallRK -Force | Out-Null
    Set-ItemProperty $UninstallRK DisplayName          $AppName
    Set-ItemProperty $UninstallRK DisplayVersion       $AppVersion
    Set-ItemProperty $UninstallRK Publisher            $Publisher
    Set-ItemProperty $UninstallRK InstallLocation      $InstallDir
    Set-ItemProperty $UninstallRK UninstallString      $uninstCmd
    Set-ItemProperty $UninstallRK QuietUninstallString $uninstCmd
    Set-ItemProperty $UninstallRK EstimatedSize ([int]((Get-Item $destDll).Length / 1024)) -Type DWord
    Set-ItemProperty $UninstallRK NoModify 1 -Type DWord
    Set-ItemProperty $UninstallRK NoRepair 1 -Type DWord

    # 5. recycle input-host processes so the new profile is picked up
    foreach ($p in $HostProcs) { Stop-Process -Name $p -Force -ErrorAction SilentlyContinue }

    Write-Host ''
    Write-Host "Installed. `"$AppName`" is registered and enabled." -ForegroundColor Green
    Write-Host 'Next: Settings > Time & language > Language & region > Add a language > Nepali,'
    Write-Host 'then pick it from the taskbar language button (Win+Space).'
    Write-Host 'Sign out / in if it is not listed yet - apps already running keep the'
    Write-Host 'old registration mapped until then.'
    $code = 0
}
catch {
    Write-Host ''
    Write-Error $_
    $code = 1
}

if ($Elevated) { Write-Host ''; Read-Host 'Press Enter to close' }
exit $code
