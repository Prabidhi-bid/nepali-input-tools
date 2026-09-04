<#
.SYNOPSIS
    Install "Input by Prabidhi.bid" (Windows TSF text service). No installer
    toolchain - plain PowerShell. Self-elevates.

.DESCRIPTION
    A TSF text service is loaded into every process that takes text input, so on
    64-bit Windows you need BOTH a 64-bit and a 32-bit xlit_tsf.dll (32-bit apps
    load the 32-bit one from the WOW64 registry view). This script:

      * builds both (x86_64- and i686-pc-windows-msvc); i686 build failure is a
        warning, not fatal, on a 64-bit OS
      * lays them out under %ProgramFiles%\Prabidhi.bid Input\  (x64 at the root,
        x86 under x86\ ; on a 32-bit OS just the x86 build at the root)
      * deregisters any prior copy, then regsvr32 /s each with the matching
        bitness of regsvr32 (System32 = native, SysWOW64 = 32-bit)
      * adds ne-NP + this TIP to the user's language list
      * adds an Apps & features entry -> uninstall.ps1

    A DLL still mapped by a running process (the Claude app, a console) is
    renamed aside so a fresh copy can be written; the stale one unloads on the
    next sign-out.

.PARAMETER NoBuild
    Skip cargo; use target\<triple>\<cfg>\xlit_tsf.dll, or a bundled xlit_tsf.dll
    (x64) / xlit_tsf.x86.dll next to this script.

.PARAMETER SkipX86
    64-bit OS only: install just the 64-bit DLL (32-bit apps won't get the TIP).

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
    [switch]$SkipX86,
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
$Tip         = '0461:{438E43E4-3800-4AB1-82A6-A2E831ABF107}{4BE59555-69DD-48CA-8BC8-AB450205A567}'

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
    if ($SkipX86) { $argv += '-SkipX86' }
    try { Start-Process powershell.exe -ArgumentList $argv -Verb RunAs }
    catch { Write-Host 'Elevation was cancelled or denied.' -ForegroundColor Red; exit 1 }
    exit 0
}

# --- elevated from here -----------------------------------------------------------
$here     = $PSScriptRoot
$repoRoot = $null
try { $repoRoot = (Resolve-Path (Join-Path $here '..\..\..') -ErrorAction Stop).Path } catch {}

# native (64-bit) System32 even if this happens to be a 32-bit PowerShell;
# SysWOW64 is always the 32-bit one.
$sysNative = Join-Path $env:windir 'System32'
if ([Environment]::Is64BitOperatingSystem -and -not [Environment]::Is64BitProcess) {
    $sysNative = Join-Path $env:windir 'Sysnative'
}
$RegsvrNative = Join-Path $sysNative 'regsvr32.exe'
$Regsvr32bit  = Join-Path $env:windir 'SysWOW64\regsvr32.exe'

# rustup writes "info: ..." to stderr; under $ErrorActionPreference='Stop' a
# *redirected* native stderr line becomes a terminating error, so run it once,
# unredirected, with the preference relaxed. Never redirect a native command's
# stderr anywhere else in this script for the same reason.
function Add-RustTargets {
    if ($NoBuild) { return }
    $old = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try { rustup target add x86_64-pc-windows-msvc i686-pc-windows-msvc } catch {}
    $ErrorActionPreference = $old
}

function Clear-LockedFile([string]$path) {
    if (-not (Test-Path $path)) { return }
    try {
        Remove-Item $path -Force -ErrorAction Stop
    } catch {
        Rename-Item $path "$path.$(Get-Date -Format yyyyMMddHHmmss).old" -Force
        Write-Host "    ($(Split-Path $path -Leaf) in use - renamed aside)" -ForegroundColor DarkGray
    }
}

# --- arch layout for this OS ----------------------------------------------------
$is64 = [Environment]::Is64BitOperatingSystem
$targets = @()
if ($is64) {
    $targets += [pscustomobject]@{ Name = 'x64'; Triple = 'x86_64-pc-windows-msvc'
        Dest = (Join-Path $InstallDir 'xlit_tsf.dll'); Regsvr = $RegsvrNative
        Bundled = (Join-Path $here 'xlit_tsf.dll'); Fatal = $true }
    if (-not $SkipX86) {
        $targets += [pscustomobject]@{ Name = 'x86'; Triple = 'i686-pc-windows-msvc'
            Dest = (Join-Path $InstallDir 'x86\xlit_tsf.dll'); Regsvr = $Regsvr32bit
            Bundled = (Join-Path $here 'xlit_tsf.x86.dll'); Fatal = $false }
    }
}
else {
    $targets += [pscustomobject]@{ Name = 'x86'; Triple = 'i686-pc-windows-msvc'
        Dest = (Join-Path $InstallDir 'xlit_tsf.dll'); Regsvr = $RegsvrNative
        Bundled = (Join-Path $here 'xlit_tsf.x86.dll'); Fatal = $true }
}

function Resolve-Source($t) {
    $built = if ($repoRoot) { Join-Path $repoRoot "target\$($t.Triple)\$Configuration\xlit_tsf.dll" } else { $null }
    if (-not $NoBuild) {
        if (-not $repoRoot) { throw "not in the repo tree - re-run with -NoBuild and a bundled DLL" }
        Write-Host "==> cargo build -p xlit-tsf --target $($t.Triple) ($Configuration)" -ForegroundColor Cyan
        & $t.Regsvr /s /u $built            # best-effort deregister of a prior in-tree reg
        Clear-LockedFile $built
        Push-Location $repoRoot
        try {
            $flags = @('build', '-p', 'xlit-tsf', '--target', $t.Triple)
            if ($Configuration -eq 'release') { $flags += '--release' }
            & cargo @flags
            if ($LASTEXITCODE) { throw "cargo build ($($t.Name)) failed ($LASTEXITCODE)" }
        } finally { Pop-Location }
        if (-not (Test-Path $built)) { throw "cargo reported success but $built is missing" }
        return $built
    }
    if (Test-Path $t.Bundled)              { return $t.Bundled }
    if ($built -and (Test-Path $built))    { return $built }
    throw "no $($t.Name) xlit_tsf.dll - build without -NoBuild, or place one next to this script"
}

try {
    Add-RustTargets
    foreach ($p in $HostProcs) { Stop-Process -Name $p -Force -ErrorAction SilentlyContinue }
    Start-Sleep -Milliseconds 400
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null

    $installed = @()
    foreach ($t in $targets) {
        try {
            $src = Resolve-Source $t
        } catch {
            if ($t.Fatal) { throw }
            Write-Warning "$($t.Name): $_  -- skipping (needs the i686 MSVC toolchain; 32-bit apps won't get the TIP)"
            continue
        }
        Write-Host "==> installing $($t.Name) -> $($t.Dest)" -ForegroundColor Cyan
        New-Item -ItemType Directory -Force -Path (Split-Path $t.Dest) | Out-Null
        & $t.Regsvr /s /u $t.Dest                # deregister a previous copy (silent, best-effort)
        Clear-LockedFile $t.Dest
        Copy-Item $src $t.Dest -Force
        Write-Host "==> $(Split-Path $t.Regsvr -Leaf) /s ($($t.Name))" -ForegroundColor Cyan
        & $t.Regsvr /s $t.Dest
        if ($LASTEXITCODE) { throw "regsvr32 ($($t.Name)) failed ($LASTEXITCODE)" }
        $installed += $t
    }
    if (-not $installed) { throw 'nothing installed' }

    # copy the uninstaller alongside the DLLs (needed by the Apps & features entry)
    $uninstSrc = Join-Path $here 'uninstall.ps1'
    $uninstDst = Join-Path $InstallDir 'uninstall.ps1'
    if (Test-Path $uninstSrc) {
        Copy-Item $uninstSrc $uninstDst -Force
    } else {
        Write-Warning "uninstall.ps1 not found next to install.ps1 - Apps & features 'Uninstall' will not work; keep both files together."
    }

    # Apps & features entry
    $uninstCmd  = "powershell.exe -NoProfile -ExecutionPolicy Bypass -File `"$uninstDst`""
    $sizeKb     = [int](((Get-Item ($installed.Dest)) | Measure-Object Length -Sum).Sum / 1024)
    New-Item -Path $UninstallRK -Force | Out-Null
    Set-ItemProperty $UninstallRK DisplayName          $AppName
    Set-ItemProperty $UninstallRK DisplayVersion       $AppVersion
    Set-ItemProperty $UninstallRK Publisher            $Publisher
    Set-ItemProperty $UninstallRK InstallLocation      $InstallDir
    Set-ItemProperty $UninstallRK DisplayIcon          ($installed[0].Dest)
    Set-ItemProperty $UninstallRK UninstallString      $uninstCmd
    Set-ItemProperty $UninstallRK QuietUninstallString $uninstCmd
    Set-ItemProperty $UninstallRK EstimatedSize        $sizeKb -Type DWord
    Set-ItemProperty $UninstallRK NoModify 1 -Type DWord
    Set-ItemProperty $UninstallRK NoRepair 1 -Type DWord

    # add ne-NP + this TIP to the user's language list -> shows in the switcher
    Write-Host "==> adding the Nepali keyboard to your language list" -ForegroundColor Cyan
    try {
        $list = Get-WinUserLanguageList
        if (-not ($list | Where-Object { $_.LanguageTag -eq 'ne-NP' })) { $list.Add('ne-NP') }
        $ne = $list | Where-Object { $_.LanguageTag -eq 'ne-NP' }
        if ($ne -and ($ne.InputMethodTips -notcontains $Tip)) { $ne.InputMethodTips.Add($Tip) }
        Set-WinUserLanguageList $list -Force
    } catch {
        Write-Warning "couldn't auto-add the keyboard ($_). Add it from Settings > Language > Nepali > Keyboards."
    }

    foreach ($p in $HostProcs) { Stop-Process -Name $p -Force -ErrorAction SilentlyContinue }

    Write-Host ''
    Write-Host ("Installed [{0}]. `"{1}`" is registered and added to your keyboard list." -f `
        (($installed | ForEach-Object Name) -join ', '), $AppName) -ForegroundColor Green
    Write-Host 'Switch to it with the taskbar language button or Win+Space.'
    Write-Host 'Sign out / in if it is not shown yet - apps already running keep the'
    Write-Host 'old registration mapped until then.'
    $code = 0
}
catch {
    # Write-Error would itself throw under $ErrorActionPreference='Stop' and
    # skip the pause below, so the elevated window would vanish with the error.
    Write-Host ''
    Write-Host "ERROR: $_" -ForegroundColor Red
    Write-Host $_.ScriptStackTrace -ForegroundColor DarkGray
    $code = 1
}

if ($Elevated) { Write-Host ''; Read-Host 'Press Enter to close' }
exit $code
