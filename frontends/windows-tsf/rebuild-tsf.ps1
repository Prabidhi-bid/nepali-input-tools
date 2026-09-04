<#
.SYNOPSIS
    Dev loop for xlit-tsf: unregister -> free the locked DLL -> build -> re-register.

.DESCRIPTION
    Once the text service is registered and enabled, Windows maps xlit_tsf.dll into
    ctfmon.exe, TextInputHost.exe, explorer.exe and every app taking text input, so
    `cargo build` cannot overwrite target\release\xlit_tsf.dll ("unable to delete
    existing file"). This script tears the install down, recycles the host
    processes, rebuilds, and registers the fresh DLL.

    It self-elevates: if not already running as Administrator it relaunches itself
    through UAC and exits the unelevated copy.

    NOTE: DllRegisterServer's RegisterProfile is machine-wide (HKLM). The
    per-user "add to language list" step (Set-WinUserLanguageList) runs in this
    script's context, so approve the UAC prompt as the SAME user you log in with
    or the keyboard is added to the other account's list.

.PARAMETER SkipRegister
    Build only; leave the service unregistered.

.PARAMETER Elevated
    Internal: set on the relaunched elevated instance so it pauses before closing.
#>
[CmdletBinding()]
param(
    [switch]$SkipRegister,
    [switch]$Elevated
)

$ErrorActionPreference = 'Stop'

function Test-Admin {
    $id = [Security.Principal.WindowsIdentity]::GetCurrent()
    ([Security.Principal.WindowsPrincipal]$id).IsInRole(
        [Security.Principal.WindowsBuiltInRole]::Administrator)
}

# --- ask for elevation first -------------------------------------------------
if (-not (Test-Admin)) {
    Write-Host 'Requesting administrator elevation...' -ForegroundColor Cyan
    $argv = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$PSCommandPath`"", '-Elevated')
    if ($SkipRegister) { $argv += '-SkipRegister' }
    try {
        Start-Process -FilePath 'powershell.exe' -ArgumentList $argv -Verb RunAs
    } catch {
        Write-Error 'Elevation was cancelled or denied.'
        exit 1
    }
    exit 0
}

# --- from here on we are elevated ------------------------------------------------
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$dll      = Join-Path $repoRoot 'target\release\xlit_tsf.dll'
$hosts    = @('ctfmon', 'TextInputHost')

function Invoke-Step($label, [scriptblock]$body) {
    Write-Host "==> $label" -ForegroundColor Cyan
    & $body
}

try {
    Invoke-Step 'Unregister current install (ok if not registered)' {
        if (Test-Path $dll) {
            & regsvr32.exe /u /s $dll
        } else {
            Write-Host '    no existing DLL, skipping' -ForegroundColor DarkGray
        }
    }

    Invoke-Step 'Recycle text-input host processes to release the DLL lock' {
        foreach ($p in $hosts) {
            Stop-Process -Name $p -Force -ErrorAction SilentlyContinue
        }
        Stop-Process -Name explorer -Force -ErrorAction SilentlyContinue
        Start-Sleep -Milliseconds 700
        if (-not (Get-Process -Name explorer -ErrorAction SilentlyContinue)) {
            Start-Process explorer.exe
        }
    }

    Invoke-Step 'Clear the target DLL (rename aside if a process still maps it)' {
        $dir = Split-Path $dll

        # sweep earlier throwaways that are no longer mapped
        Get-ChildItem -Path $dir -Filter 'xlit_tsf.dll.*.old' -ErrorAction SilentlyContinue |
            ForEach-Object { Remove-Item $_.FullName -Force -ErrorAction SilentlyContinue }

        if (Test-Path $dll) {
            try {
                Remove-Item $dll -Force -ErrorAction Stop
                Write-Host '    removed' -ForegroundColor DarkGray
            } catch {
                # Windows lets you *rename* a mapped DLL even when it can't be
                # deleted/overwritten. cargo then writes a fresh copy; the stale
                # mapping in e.g. the Claude app or a console unloads on next
                # sign-out. Killing those processes is not worth it.
                $holders = Get-Process -ErrorAction SilentlyContinue |
                    Where-Object { try { $_.Modules.ModuleName -contains 'xlit_tsf.dll' } catch { $false } } |
                    Select-Object -ExpandProperty ProcessName -Unique
                $leaf = "xlit_tsf.dll.$(Get-Date -Format yyyyMMddHHmmss).old"
                Rename-Item -Path $dll -NewName $leaf -Force -ErrorAction Stop
                Write-Host "    still mapped by: $($holders -join ', ')" -ForegroundColor Yellow
                Write-Host "    renamed to $leaf; a fresh DLL will be built" -ForegroundColor DarkGray
            }
        } else {
            Write-Host '    already clear' -ForegroundColor DarkGray
        }
    }

    Invoke-Step 'cargo build -p xlit-tsf --release' {
        $cargo = (Get-Command cargo -ErrorAction SilentlyContinue).Source
        if (-not $cargo) { $cargo = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe' }
        if (-not (Test-Path $cargo)) { throw 'cargo not found on PATH or in ~\.cargo\bin' }
        Push-Location $repoRoot
        try {
            & $cargo build -p xlit-tsf --release
            if ($LASTEXITCODE -ne 0) { throw "cargo build failed ($LASTEXITCODE)" }
        } finally {
            Pop-Location
        }
    }

    if (-not $SkipRegister) {
        Invoke-Step 'Register the fresh DLL' {
            & regsvr32.exe /s /u $dll 2>$null   # clean any prior registration first
            & regsvr32.exe /s $dll
            if ($LASTEXITCODE -ne 0) { throw "regsvr32 failed ($LASTEXITCODE)" }
        }
        Invoke-Step 'Add the Nepali keyboard to your language list' {
            try {
                $tip = '0461:{438E43E4-3800-4AB1-82A6-A2E831ABF107}{4BE59555-69DD-48CA-8BC8-AB450205A567}'
                $list = Get-WinUserLanguageList
                if (-not ($list | Where-Object { $_.LanguageTag -eq 'ne-NP' })) { $list.Add('ne-NP') }
                $ne = $list | Where-Object { $_.LanguageTag -eq 'ne-NP' }
                if ($ne -and ($ne.InputMethodTips -notcontains $tip)) { $ne.InputMethodTips.Add($tip) }
                Set-WinUserLanguageList $list -Force
            } catch {
                Write-Warning "couldn't auto-add ($_) - add it from Settings > Language > Nepali"
            }
        }
        Invoke-Step 'Recycle hosts again so they load the new registration' {
            foreach ($p in $hosts) {
                Stop-Process -Name $p -Force -ErrorAction SilentlyContinue
            }
        }
        Write-Host ''
        Write-Host 'Done. "Input by Prabidhi.bid" is registered and added to your keyboard list.' -ForegroundColor Green
        Write-Host 'Apps already running (this console, the Claude app) keep the old copy'
        Write-Host 'mapped until you sign out/in. If it is not in the switcher yet, sign out/in.'
    } else {
        Write-Host ''
        Write-Host 'Build done; service left unregistered (-SkipRegister).' -ForegroundColor Green
    }
    $exit = 0
} catch {
    Write-Host ''
    Write-Error $_
    $exit = 1
}

if ($Elevated) {
    Write-Host ''
    Read-Host 'Press Enter to close'
}
exit $exit
