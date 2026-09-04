<#
.SYNOPSIS
    Build "Input-by-Prabidhi.bid-<ver>-setup.exe" with Inno Setup.

.DESCRIPTION
    1. cargo build -p xlit-tsf --release   (no `trace` feature -> no DebugView
       strings in the shipped DLL)
    2. locate ISCC.exe
    3. iscc xlit-tsf.iss -> setup.exe next to this script

    One-time prerequisite:
        winget install JRSoftware.InnoSetup

.PARAMETER Version
    Version stamped into the installer (default 0.1.0).

.PARAMETER Configuration
    release (default) or debug — which target\<cfg>\xlit_tsf.dll to embed.

.PARAMETER Password
    If given, encrypt the installer's embedded payload (Inno `Encryption`).
    Speed-bump only — see installer/README.md.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File frontends\windows-tsf\installer\build-setup.ps1
#>
[CmdletBinding()]
param(
    [string]$Version = '0.1.0',
    [ValidateSet('release', 'debug')][string]$Configuration = 'release',
    [string]$Password
)
$ErrorActionPreference = 'Stop'

$here     = $PSScriptRoot
$repoRoot = (Resolve-Path (Join-Path $here '..\..\..')).Path
$dll      = Join-Path $repoRoot "target\$Configuration\xlit_tsf.dll"

Write-Host "==> cargo build -p xlit-tsf ($Configuration, no trace)" -ForegroundColor Cyan
Push-Location $repoRoot
try {
    $flags = @('build', '-p', 'xlit-tsf')
    if ($Configuration -eq 'release') { $flags += '--release' }
    & cargo @flags
    if ($LASTEXITCODE) { throw "cargo build failed ($LASTEXITCODE)" }
} finally {
    Pop-Location
}
if (-not (Test-Path $dll)) { throw "DLL not found: $dll" }

# locate the Inno Setup compiler
$iscc = (Get-Command iscc.exe -ErrorAction SilentlyContinue).Source
if (-not $iscc) {
    foreach ($p in @(
            "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
            "${env:ProgramFiles}\Inno Setup 6\ISCC.exe")) {
        if (Test-Path $p) { $iscc = $p; break }
    }
}
if (-not $iscc) {
    throw "Inno Setup not found. Install it with:  winget install JRSoftware.InnoSetup"
}

$isccArgs = @("/DDllPath=$dll", "/DAppVersion=$Version")
if ($Password) { $isccArgs += "/DSetupPassword=$Password" }
$isccArgs += (Join-Path $here 'xlit-tsf.iss')

Write-Host "==> $(Split-Path $iscc -Leaf) xlit-tsf.iss" -ForegroundColor Cyan
& $iscc @isccArgs
if ($LASTEXITCODE) { throw "iscc failed ($LASTEXITCODE)" }

$out = Join-Path $here "Input-by-Prabidhi.bid-$Version-setup.exe"
Write-Host ''
Write-Host "built $out" -ForegroundColor Green
Write-Host 'Sign it before distributing:  signtool sign /fd sha256 /a /tr http://timestamp.digicert.com /td sha256 "<setup.exe>"'
