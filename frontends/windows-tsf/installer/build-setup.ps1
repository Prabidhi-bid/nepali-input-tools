<#
.SYNOPSIS
    Build "Input-by-Prabidhi.bid-<ver>-setup.exe" with Inno Setup.

.DESCRIPTION
    1. cargo build -p xlit-tsf --release for x86_64- AND i686-pc-windows-msvc
       (a TSF DLL loads into every process, so 64-bit Windows needs both).
       An i686 build failure is a warning, not fatal - the Setup.exe is then
       x64-only.
    2. locate ISCC.exe
    3. iscc xlit-tsf.iss -> setup.exe next to this script

    One-time prerequisites:
        winget install JRSoftware.InnoSetup
        rustup target add i686-pc-windows-msvc    (the script also tries this)
    Building i686 also needs the x86 MSVC toolchain (any full VS / Build Tools
    install has it).

.PARAMETER Version
    Version stamped into the installer (default 0.1.1).

.PARAMETER Configuration
    release (default) or debug.

.PARAMETER SkipX86
    Build x64 only (the Setup.exe won't serve 32-bit apps).

.PARAMETER Password
    Encrypt the installer's embedded payload (Inno Encryption). Speed-bump only.

.PARAMETER Iscc
    Full path to ISCC.exe (or its folder) if auto-detection fails.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File frontends\windows-tsf\installer\build-setup.ps1
#>
[CmdletBinding()]
param(
    [string]$Version = '0.1.1',
    [ValidateSet('release', 'debug')][string]$Configuration = 'release',
    [switch]$SkipX86,
    [string]$Password,
    [string]$Iscc
)
$ErrorActionPreference = 'Stop'

$here     = $PSScriptRoot
$repoRoot = $null
try { $repoRoot = (Resolve-Path (Join-Path $here '..\..\..') -ErrorAction Stop).Path } catch {}
if (-not $repoRoot) { throw "run this from inside the repo (couldn't resolve $here\..\..\..)" }

function Clear-LockedFile([string]$path) {
    if (-not (Test-Path $path)) { return }
    try { Remove-Item $path -Force -ErrorAction Stop }
    catch {
        Rename-Item $path "$path.$(Get-Date -Format yyyyMMddHHmmss).old" -Force
        Write-Host "    ($(Split-Path $path -Leaf) in use - renamed aside)" -ForegroundColor DarkGray
    }
}

# rustup writes its "info: ..." lines to stderr; under $ErrorActionPreference =
# 'Stop' a redirected native stderr line is turned into a terminating error, so
# run it once, unredirected, with the preference relaxed.
function Add-RustTargets {
    $old = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try { rustup target add x86_64-pc-windows-msvc i686-pc-windows-msvc } catch {}
    $ErrorActionPreference = $old
}

function Invoke-CargoBuild([string]$triple) {
    $dll = Join-Path $repoRoot "target\$triple\$Configuration\xlit_tsf.dll"
    Get-ChildItem (Split-Path $dll) -Filter 'xlit_tsf.dll.*.old' -ErrorAction SilentlyContinue |
        ForEach-Object { Remove-Item $_.FullName -Force -ErrorAction SilentlyContinue }
    Clear-LockedFile $dll
    Push-Location $repoRoot
    try {
        $flags = @('build', '-p', 'xlit-tsf', '--target', $triple)
        if ($Configuration -eq 'release') { $flags += '--release' }
        & cargo @flags
        if ($LASTEXITCODE) { throw "cargo build failed for $triple ($LASTEXITCODE)" }
    } finally { Pop-Location }
    if (-not (Test-Path $dll)) { throw "expected $dll" }
    $dll
}

function Resolve-Iscc {
    param([string]$Explicit)
    if ($Explicit) {
        if (Test-Path $Explicit -PathType Leaf) { return (Resolve-Path $Explicit).Path }
        $c = Join-Path $Explicit 'ISCC.exe'
        if (Test-Path $c -PathType Leaf) { return $c }
        throw "ISCC.exe not found at -Iscc '$Explicit'"
    }
    $g = Get-Command 'ISCC.exe' -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($g) { return $g.Source }
    $dirs = New-Object System.Collections.Generic.List[string]
    foreach ($scope in 'Machine', 'User') {
        $p = [Environment]::GetEnvironmentVariable('Path', $scope)
        if ($p) { $p.Split(';') | ForEach-Object { $dirs.Add($_) } }
    }
    foreach ($rk in @(
            'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Inno Setup 6_is1',
            'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Inno Setup 6_is1',
            'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Inno Setup 6_is1',
            'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Inno Setup 5_is1',
            'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Inno Setup 5_is1')) {
        try {
            $loc = (Get-ItemProperty -LiteralPath $rk -ErrorAction Stop).InstallLocation
            if ($loc) { $dirs.Add($loc) }
        } catch {}
    }
    $pf86 = [Environment]::GetEnvironmentVariable('ProgramFiles(x86)')
    foreach ($d in @("$pf86\Inno Setup 6", "$env:ProgramFiles\Inno Setup 6",
            "$env:LOCALAPPDATA\Programs\Inno Setup 6",
            "$pf86\Inno Setup 5", "$env:ProgramFiles\Inno Setup 5")) {
        $dirs.Add($d)
    }
    foreach ($d in ($dirs | Where-Object { $_ } | Select-Object -Unique)) {
        $c = Join-Path ($d.Trim().TrimEnd('\')) 'ISCC.exe'
        if (Test-Path $c -PathType Leaf) { return $c }
    }
    return $null
}

Add-RustTargets

Write-Host "==> building x64 (x86_64-pc-windows-msvc, $Configuration)" -ForegroundColor Cyan
$dllX64 = Invoke-CargoBuild 'x86_64-pc-windows-msvc'

$dllX86 = $null
if ($SkipX86) {
    Write-Warning 'SkipX86: the installer will not serve 32-bit apps.'
} else {
    Write-Host "==> building x86 (i686-pc-windows-msvc, $Configuration)" -ForegroundColor Cyan
    try {
        $dllX86 = Invoke-CargoBuild 'i686-pc-windows-msvc'
    } catch {
        Write-Warning "x86 build failed ($_). Continuing x64-only - install the x86 MSVC toolchain to fix."
        $dllX86 = $null
    }
}

$iscc = Resolve-Iscc -Explicit $Iscc
if (-not $iscc) {
    throw @'
Inno Setup (ISCC.exe) not found. Either install it:
    winget install JRSoftware.InnoSetup
or point at it directly:
    build-setup.ps1 -Iscc "C:\Program Files (x86)\Inno Setup 6\ISCC.exe"
'@
}
Write-Host "using $iscc" -ForegroundColor DarkGray

$isccArgs = @("/DDllPathX64=$dllX64", "/DAppVersion=$Version")
if ($dllX86)  { $isccArgs += "/DDllPathX86=$dllX86" }
if ($Password) { $isccArgs += "/DSetupPassword=$Password" }
$isccArgs += (Join-Path $here 'xlit-tsf.iss')

Write-Host "==> $(Split-Path $iscc -Leaf) xlit-tsf.iss" -ForegroundColor Cyan
& $iscc @isccArgs
if ($LASTEXITCODE) { throw "iscc failed ($LASTEXITCODE)" }

$out = Join-Path $here "Input-by-Prabidhi.bid-$Version-setup.exe"
if (-not (Test-Path $out)) {
    $found = Get-ChildItem $here -Filter '*setup.exe' -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if ($found) { $out = $found.FullName } else { throw "iscc reported success but no setup .exe found under $here" }
}
Write-Host ''
Write-Host ("built {0}  ({1})" -f $out, $(if ($dllX86) { 'x64 + x86' } else { 'x64 only' })) -ForegroundColor Green
Write-Host 'Sign it before distributing:  signtool sign /fd sha256 /a /tr http://timestamp.digicert.com /td sha256 "<setup.exe>"'
