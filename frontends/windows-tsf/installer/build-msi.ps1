<#
.SYNOPSIS
    Build "Input-by-Prabidhi.bid-<ver>-x64.msi" from xlit-tsf.wxs.

.DESCRIPTION
    1. cargo build -p xlit-tsf (release by default)
    2. ensure the WiX CLI + UI extension are present
    3. wix build xlit-tsf.wxs -> .msi next to this script

    One-time prerequisites:
        winget install Microsoft.DotNet.SDK.8      (any dotnet >= 6)
        dotnet tool install --global wix
    The UI extension is added automatically (idempotent).

.PARAMETER Version
    Product version stamped into the MSI (default 0.1.0).

.PARAMETER Configuration
    'release' (default) or 'debug' — which target\<cfg>\xlit_tsf.dll to package.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File frontends\windows-tsf\installer\build-msi.ps1
#>
[CmdletBinding()]
param(
    [string]$Version = '0.1.0',
    [ValidateSet('release', 'debug')][string]$Configuration = 'release'
)

$ErrorActionPreference = 'Stop'

$here     = $PSScriptRoot
$repoRoot = (Resolve-Path (Join-Path $here '..\..\..')).Path
$dll      = Join-Path $repoRoot "target\$Configuration\xlit_tsf.dll"
$out      = Join-Path $here "Input-by-Prabidhi.bid-$Version-x64.msi"

Write-Host "==> cargo build -p xlit-tsf ($Configuration)" -ForegroundColor Cyan
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

Write-Host "==> checking WiX" -ForegroundColor Cyan
if (-not (Get-Command wix -ErrorAction SilentlyContinue)) {
    throw "WiX CLI not found. Install it with:  dotnet tool install --global wix"
}
& wix extension add -g WixToolset.UI.wixext | Out-Null   # no-op if already added

Write-Host "==> wix build -> $(Split-Path $out -Leaf)" -ForegroundColor Cyan
Push-Location $here
try {
    & wix build 'xlit-tsf.wxs' `
        -arch x64 `
        -ext WixToolset.UI.wixext `
        -d "DllPath=$dll" `
        -d "ProductVersion=$Version" `
        -o $out
    if ($LASTEXITCODE) { throw "wix build failed ($LASTEXITCODE)" }
} finally {
    Pop-Location
}

Write-Host ''
Write-Host "built $out" -ForegroundColor Green
Write-Host 'install:    double-click the .msi (accept UAC)   or   msiexec /i "<msi>"'
Write-Host 'silent:     msiexec /i "<msi>" /qn'
Write-Host 'uninstall:  msiexec /x "<msi>" /qn'
Write-Host ''
Write-Host 'Then add Nepali under Settings > Time & Language, and pick'
Write-Host '"Input by Prabidhi.bid" from the taskbar language button (Win+Space).'
