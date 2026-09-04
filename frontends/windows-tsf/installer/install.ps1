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
$Clsid       = '{438E43E4-3800-4AB1-82A6-A2E831ABF107}'
$Tip         = "0461:$Clsid{4BE59555-69DD-48CA-8BC8-AB450205A567}"

function Test-Admin {
    ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
    ).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

# The language list is per-user and, in practice, does not reliably stick when
# written from the elevated child process - the keyboard silently fails to
# appear in the switcher. So this runs in the *original* (unelevated) session,
# after the elevated half has registered the profile it refers to.
function Add-NeKeyboard {
    try {
        $list = Get-WinUserLanguageList
        if (-not ($list | Where-Object { $_.LanguageTag -eq 'ne-NP' })) { $list.Add('ne-NP') }
        $ne = $list | Where-Object { $_.LanguageTag -eq 'ne-NP' }
        if ($ne -and ($ne.InputMethodTips -notcontains $Tip)) { $ne.InputMethodTips.Add($Tip) }
        Set-WinUserLanguageList $list -Force

        # Windows discards a TIP it cannot resolve to a registered profile and
        # reports success anyway, so read it back rather than trust the call.
        $after = Get-WinUserLanguageList | Where-Object { $_.LanguageTag -eq 'ne-NP' }
        if ($after -and ($after.InputMethodTips -contains $Tip)) {
            Write-Host 'Keyboard added to your language list.' -ForegroundColor Green
            return $true
        }
        Write-Warning "Windows did not keep the keyboard in your language list. Add 'Input by Prabidhi.bid' from Settings > Time & Language > Language & region > Nepali > Language options > Keyboards."
        return $false
    } catch {
        Write-Warning "couldn't add the keyboard ($_). Add it from Settings > Language > Nepali > Keyboards."
        return $false
    }
}

# --- ask for elevation first --------------------------------------------------
if (-not (Test-Admin)) {
    Write-Host 'Requesting administrator elevation...' -ForegroundColor Cyan
    $argv = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$PSCommandPath`"",
        '-Elevated', '-Configuration', $Configuration)
    if ($NoBuild) { $argv += '-NoBuild' }
    if ($SkipX86) { $argv += '-SkipX86' }
    try { $proc = Start-Process powershell.exe -ArgumentList $argv -Verb RunAs -Wait -PassThru }
    catch { Write-Host 'Elevation was cancelled or denied.' -ForegroundColor Red; exit 1 }
    # Judge by what is actually registered, not by the exit code. The elevated
    # window pauses on failure, and closing it rather than pressing Enter exits
    # with 0xC000013A - which is not a failed install, and refusing to add the
    # keyboard because of it would leave the input method half-installed.
    if (-not (Test-Path "HKLM:\SOFTWARE\Microsoft\CTF\TIP\$Clsid")) {
        Write-Host "The elevated step did not register the input method (exit $($proc.ExitCode))." -ForegroundColor Red
        exit 1
    }
    Add-NeKeyboard | Out-Null
    foreach ($p in $HostProcs) { Stop-Process -Name $p -Force -ErrorAction SilentlyContinue }
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

# regsvr32.exe is a GUI-subsystem program, so PowerShell's call operator does
# NOT wait for it: "& regsvr32 /s foo.dll" returns immediately, $LASTEXITCODE is
# whatever it was before, and the next line runs while registration is still in
# flight. That is how a deregister could race a register and leave no TSF
# profile at all. Start-Process -Wait is the only way to sequence these.
function Invoke-Regsvr32 {
    param([string]$Exe, [string]$Dll, [switch]$Unregister)
    $argv = @('/s')
    if ($Unregister) { $argv += '/u' }
    $argv += "`"$Dll`""
    $p = Start-Process -FilePath $Exe -ArgumentList $argv -Wait -PassThru -WindowStyle Hidden
    return $p.ExitCode
}

# A DLL still mapped by a running application cannot be deleted. It can almost
# always be *renamed* out of the way, which frees the name for the new copy
# while the old image stays mapped until that process exits. Returns whether the
# path is now free.
#
# Rename-Item wants a bare name, not a path - passing a full path fails.
function Clear-LockedFile([string]$path) {
    if (-not (Test-Path $path)) { return $true }
    try {
        Remove-Item $path -Force -ErrorAction Stop
        return $true
    } catch {}
    try {
        Rename-Item -LiteralPath $path -NewName "$(Split-Path $path -Leaf).$(Get-Date -Format yyyyMMddHHmmss).old" -Force -ErrorAction Stop
        Write-Host "    ($(Split-Path $path -Leaf) in use - renamed aside)" -ForegroundColor DarkGray
        return $true
    } catch {}
    Write-Warning "$(Split-Path $path -Leaf) is locked and could not be moved aside."
    return $false
}

# Sweep the *.old copies left behind by earlier runs, once their processes have
# gone. Best-effort; the ones still mapped simply stay until the next sign-out.
function Remove-StaleCopies([string]$dir) {
    if (-not (Test-Path $dir)) { return }
    Get-ChildItem $dir -Recurse -Filter '*.old' -ErrorAction SilentlyContinue |
        ForEach-Object { Remove-Item $_.FullName -Force -ErrorAction SilentlyContinue }
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
        Invoke-Regsvr32 -Exe $t.Regsvr -Dll $built -Unregister | Out-Null   # prior in-tree reg
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

    # The word editor is a normal executable, so one host-architecture build is
    # enough - unlike the DLL, it is never loaded into anybody else's process.
    if (-not $NoBuild -and $repoRoot) {
        Write-Host "==> cargo build -p xlit-config ($Configuration)" -ForegroundColor Cyan
        Push-Location $repoRoot
        try {
            $flags = @('build', '-p', 'xlit-config')
            if ($Configuration -eq 'release') { $flags += '--release' }
            & cargo @flags
            if ($LASTEXITCODE) { Write-Warning "the word editor did not build ($LASTEXITCODE); the '+' on the floating bar will do nothing" }
        } finally { Pop-Location }
    }

    foreach ($p in $HostProcs) { Stop-Process -Name $p -Force -ErrorAction SilentlyContinue }
    Start-Sleep -Milliseconds 400
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null

    # These run as four separate passes over the architectures, and the order
    # matters. DllUnregisterServer removes the *TSF profile*, which is
    # machine-wide and shared by both bitnesses - it is not per-architecture
    # like the CLSID keys. Deregistering and registering one architecture at a
    # time therefore has x86's deregister tear out the profile that x64 just
    # registered, and if x86 then fails to register you are left with no
    # profile at all: the input method vanishes from the language switcher even
    # though its files and COM keys look fine.
    #
    # So: resolve everything, deregister everything, copy everything, and only
    # then register.
    $installed = @()
    foreach ($t in $targets) {
        try {
            $t | Add-Member -NotePropertyName Src -NotePropertyValue (Resolve-Source $t) -Force
            $installed += $t
        } catch {
            if ($t.Fatal) { throw }
            Write-Warning "$($t.Name): $_  -- skipping (needs the i686 MSVC toolchain; 32-bit apps won't get the TIP)"
        }
    }
    if (-not $installed) { throw 'nothing installed' }

    # Stage every new DLL beside its destination *before* deregistering
    # anything. Writing a brand new file always works; replacing a mapped one
    # may not, and finding that out after deregistering would leave the input
    # method unregistered - which is exactly how it kept vanishing.
    foreach ($t in $installed) {
        New-Item -ItemType Directory -Force -Path (Split-Path $t.Dest) | Out-Null
        Copy-Item $t.Src "$($t.Dest).new" -Force
    }

    foreach ($t in $installed) {
        Invoke-Regsvr32 -Exe $t.Regsvr -Dll $t.Dest -Unregister | Out-Null   # best-effort
    }
    # Deregistering tells the input hosts to let go; give them a moment to
    # actually unmap before trying to move the file.
    foreach ($p in $HostProcs) { Stop-Process -Name $p -Force -ErrorAction SilentlyContinue }
    Start-Sleep -Milliseconds 600

    foreach ($t in $installed) {
        Write-Host "==> installing $($t.Name) -> $($t.Dest)" -ForegroundColor Cyan
        if (Clear-LockedFile $t.Dest) {
            Move-Item "$($t.Dest).new" $t.Dest -Force
        }
        else {
            # Keep the copy that is already there and register that instead of
            # aborting: an older build still working beats no input method.
            Remove-Item "$($t.Dest).new" -Force -ErrorAction SilentlyContinue
            if (Test-Path $t.Dest) {
                Write-Warning "$($t.Name): kept the installed copy - close your apps or sign out and re-run to update it."
            }
            else {
                throw "$($t.Name): could not write $($t.Dest)"
            }
        }
    }

    foreach ($t in $installed) {
        Write-Host "==> $(Split-Path $t.Regsvr -Leaf) /s ($($t.Name))" -ForegroundColor Cyan
        $rc = Invoke-Regsvr32 -Exe $t.Regsvr -Dll $t.Dest
        if ($rc) { throw "regsvr32 ($($t.Name)) failed ($rc)" }
    }
    Remove-StaleCopies $InstallDir

    # The profile is what the language switcher reads through; if it is missing
    # here, adding the keyboard below would silently do nothing.
    if (-not (Test-Path "HKLM:\SOFTWARE\Microsoft\CTF\TIP\$Clsid")) {
        throw "registration reported success but the TSF profile is missing - the input method would not appear"
    }

    # The word editor. A separate executable rather than a dialog inside the
    # DLL, which is loaded into every application that takes typing; the
    # floating bar's "+" launches it from beside the DLL.
    $cfgName = 'xlit-config.exe'
    $cfgDst = Join-Path $InstallDir $cfgName
    $cfgSrc = $null
    foreach ($c in @(
            (Join-Path $here $cfgName),
            $(if ($repoRoot) { Join-Path $repoRoot "target\x86_64-pc-windows-msvc\$Configuration\$cfgName" }),
            $(if ($repoRoot) { Join-Path $repoRoot "target\$Configuration\$cfgName" }))) {
        if ($c -and (Test-Path $c)) { $cfgSrc = $c; break }
    }
    if ($cfgSrc) {
        Write-Host "==> installing the word editor -> $cfgDst" -ForegroundColor Cyan
        Clear-LockedFile $cfgDst
        Copy-Item $cfgSrc $cfgDst -Force
    }
    else {
        Write-Warning "$cfgName not found - the '+' on the floating bar will do nothing. Build it with: cargo build -p xlit-config --release"
    }

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

    # Only meaningful when the script was started from an already-elevated
    # shell; the normal path does this in the unelevated parent instead.
    if (-not $Elevated) {
        Write-Host "==> adding the Nepali keyboard to your language list" -ForegroundColor Cyan
        Add-NeKeyboard | Out-Null
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

# Pause only when something went wrong and there is a message worth reading;
# on success the window closes itself, so nobody is tempted to dismiss it in a
# way that looks like a crash.
if ($Elevated -and $code -ne 0) { Write-Host ''; Read-Host 'Press Enter to close' }
exit $code
