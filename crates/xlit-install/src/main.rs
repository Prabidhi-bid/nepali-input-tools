//! Installer for "Input by Prabidhi.bid".
//!
//! One self-contained executable. Both text-service DLLs and the word editor are
//! compiled into it (see `build.rs`), so the machine it runs on needs nothing
//! installed — no Rust, no PowerShell script, no redistributable, no download.
//!
//! What it does, in the order that matters:
//!
//!   1. elevate (the COM keys and the profile live in HKLM);
//!   2. write both DLLs and the word editor to `%ProgramFiles%\Prabidhi.bid Input`;
//!   3. register the 64-bit DLL by calling its own `DllRegisterServer`, which is
//!      what puts the TSF profile and the TIP categories on the machine;
//!   4. write the 32-bit CLSID keys directly into the WOW6432Node view;
//!   5. drop back to the user's own session to add the keyboard to the language
//!      list, which is per-user and does not stick when written while elevated.
//!   6. restart the processes that host text input — including the shell,
//!      which otherwise keeps the previous DLL mapped until the next logon.
//!
//! **Why step 4 is registry writes rather than a second `DllRegisterServer`.**
//! The TSF profile and categories are machine-wide and bitness-independent —
//! step 3 already registered them. All the 32-bit side needs is its own
//! `InprocServer32` under the 32-bit registry view, so that a 32-bit application
//! can find the DLL. Doing that directly means the installer never has to load
//! the 32-bit DLL, which a 64-bit process cannot do anyway; the old script's
//! answer was to shell out to the SysWOW64 `regsvr32`, and when that failed it
//! could only report the number 3.

#![cfg(windows)]

mod selfreg;

use std::path::{Path, PathBuf};

use windows::core::PCWSTR;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_WOW64_32KEY, KEY_WRITE};
use winreg::RegKey;

mod payload {
    include!(concat!(env!("OUT_DIR"), "/payload.rs"));
}

const APP_NAME: &str = "Input by Prabidhi.bid";
const APP_VERSION: &str = "0.1.1";
const PUBLISHER: &str = "Prabidhi.bid";
const CLSID: &str = "{438E43E4-3800-4AB1-82A6-A2E831ABF107}";
const PROFILE_GUID: &str = "{4BE59555-69DD-48CA-8BC8-AB450205A567}";
const UNINSTALL_KEY: &str =
    r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\PrabidhibidInput";
/// The TIP string the language switcher uses: `<langid>:<clsid><profile guid>`.
fn tip_string() -> String {
    format!("0461:{CLSID}{PROFILE_GUID}")
}

fn install_dir() -> PathBuf {
    PathBuf::from(std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into()))
        .join("Prabidhi.bid Input")
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = args.first().map(String::as_str);

    let code = match mode {
        Some("--uninstall") => run(uninstall_flow()),
        Some("--machine-install") => run(machine_install()),
        Some("--machine-uninstall") => run(machine_uninstall()),
        Some("--help" | "-h" | "/?") => {
            usage();
            0
        }
        None => run(install_flow()),
        Some(other) => {
            eprintln!("unknown option {other:?}");
            usage();
            2
        }
    };
    // An elevated child gets its own console window, which would vanish with the
    // message still on it.
    if matches!(mode, Some("--machine-install" | "--machine-uninstall")) && code != 0 {
        pause();
    }
    std::process::exit(code);
}

fn usage() {
    println!("xlit-install                install the input method");
    println!("xlit-install --uninstall    remove it");
    println!();
    println!("Everything it installs is inside this executable; nothing else is needed.");
}

fn run(r: Result<(), String>) -> i32 {
    match r {
        Ok(()) => 0,
        Err(e) => {
            eprintln!();
            eprintln!("ERROR: {e}");
            1
        }
    }
}

fn pause() {
    eprintln!();
    eprint!("Press Enter to close. ");
    let mut s = String::new();
    let _ = std::io::stdin().read_line(&mut s);
}

// ---------------------------------------------------------------------------
// Elevation
// ---------------------------------------------------------------------------

fn is_elevated() -> bool {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut size = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut _),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut size,
        )
        .is_ok();
        ok && elevation.TokenIsElevated != 0
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Re-run this same executable elevated, with `arg`, and wait for it.
///
/// `ShellExecuteW` with the `runas` verb is what raises the UAC prompt. It hands
/// back an `HINSTANCE` that is really a status code — anything above 32 means the
/// process started.
fn elevate_self(arg: &str) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot find my own path: {e}"))?;
    let verb = wide("runas");
    let file = wide(&exe.to_string_lossy());
    let params = wide(arg);

    let rc = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(file.as_ptr()),
            PCWSTR(params.as_ptr()),
            None,
            SW_SHOWNORMAL,
        )
    };
    if rc.0 as usize <= 32 {
        return Err("elevation was cancelled or denied".into());
    }
    // ShellExecuteW does not give us the process handle, so wait on the state we
    // actually care about instead of on the process: the registration appearing.
    Ok(())
}

// ---------------------------------------------------------------------------
// The user-session half
// ---------------------------------------------------------------------------

fn install_flow() -> Result<(), String> {
    if is_elevated() {
        machine_install()?;
    } else {
        println!("Requesting administrator elevation...");
        elevate_self("--machine-install")?;
        wait_for(|| tsf_profile_registered(), true)?;
    }
    add_keyboard()?;
    restart_input_hosts();
    restart_tip_hosts();
    println!();
    println!("Installed. \"{APP_NAME}\" is registered and added to your keyboard list.");
    println!("Switch to it with the taskbar language button or Win+Space.");
    Ok(())
}

fn uninstall_flow() -> Result<(), String> {
    remove_keyboard();
    if is_elevated() {
        machine_uninstall()?;
    } else {
        elevate_self("--machine-uninstall")?;
        wait_for(|| !tsf_profile_registered(), false)?;
    }
    restart_input_hosts();
    restart_tip_hosts();
    println!("Removed.");
    Ok(())
}

/// Poll for up to a minute while the elevated half does its work. The user is
/// looking at a UAC prompt for most of it.
fn wait_for(done: impl Fn() -> bool, want: bool) -> Result<(), String> {
    for _ in 0..600 {
        if done() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Err(if want {
        "the elevated step did not register the input method".into()
    } else {
        "the elevated step did not remove the input method".into()
    })
}

fn tsf_profile_registered() -> bool {
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(format!(r"SOFTWARE\Microsoft\CTF\TIP\{CLSID}"))
        .is_ok()
}

/// Add (or remove) the TIP from the user's language list.
///
/// `InstallLayoutOrTip` in `input.dll` is the call Windows' own language
/// settings make. It is per-user, which is why it runs here in the user's
/// session rather than in the elevated half — written from the elevated process
/// it lands in the *administrator's* profile and the keyboard silently never
/// appears.
fn layout_or_tip(flags: u32) -> bool {
    let dll = wide("input.dll");
    unsafe {
        let Ok(module) = windows::Win32::System::LibraryLoader::LoadLibraryW(PCWSTR(dll.as_ptr()))
        else {
            return false;
        };
        let name = b"InstallLayoutOrTip\0";
        let Some(proc) = windows::Win32::System::LibraryLoader::GetProcAddress(
            module,
            windows::core::PCSTR(name.as_ptr()),
        ) else {
            return false;
        };
        let f: extern "system" fn(PCWSTR, u32) -> i32 = std::mem::transmute(proc);
        let tip = wide(&tip_string());
        f(PCWSTR(tip.as_ptr()), flags) != 0
    }
}

fn add_keyboard() -> Result<(), String> {
    // 0x0 = add to the language list without making it the default.
    if layout_or_tip(0) {
        println!("Keyboard added to your language list.");
        Ok(())
    } else {
        // Not fatal: the text service is registered and can still be added by
        // hand, so say how rather than failing an otherwise good install.
        println!(
            "Could not add the keyboard automatically. Add \"{APP_NAME}\" from \
             Settings > Time & Language > Language & region > Nepali > Language options > Keyboards."
        );
        Ok(())
    }
}

fn remove_keyboard() {
    // 0x2 = ILOT_UNINSTALL.
    let _ = layout_or_tip(2);
}

/// The cheap half: the two brokers that cache the registration. They come back
/// on their own and restarting them costs the user nothing, so this runs from
/// both halves of the install. Applications that have already *loaded* the DLL
/// need [`restart_tip_hosts`] instead.
fn restart_input_hosts() {
    for proc in ["ctfmon.exe", "TextInputHost.exe"] {
        let _ = std::process::Command::new("taskkill")
            .args(["/f", "/im", proc])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

/// Restart the long-lived processes that host text input for the whole
/// session.
///
/// A process that has already mapped the text service keeps that DLL image
/// for its lifetime, whatever we write to disk — renaming the old file does
/// not touch it. `explorer.exe` (the taskbar and every File Explorer window)
/// and `SearchHost.exe` (the Start menu search box) run from logon to logoff,
/// so without this an upgrade never reaches the two places everybody types
/// into, and a bug fixed in the new build goes on looking unfixed.
///
/// Only the user-session half may call this. The elevated child must not: a
/// shell it relaunched would inherit elevation, and everything started from
/// the taskbar afterwards would run as administrator.
fn restart_tip_hosts() {
    println!("Restarting the desktop shell so it picks up the new version...");
    // SearchHost is started on demand by the shell, so it needs no relaunch.
    for proc in ["SearchHost.exe", "explorer.exe"] {
        let _ = std::process::Command::new("taskkill")
            .args(["/f", "/im", proc])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    // Windows brings the shell back by itself when AutoRestartShell is set,
    // which is the default. Wait for that rather than racing it.
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(250));
        if process_running("explorer.exe") {
            return;
        }
    }
    if is_elevated() {
        // Starting it here would hand the whole desktop our elevated token.
        println!("warning: the desktop did not come back on its own. Open Task");
        println!("         Manager (Ctrl+Shift+Esc) and run \"explorer.exe\".");
    } else {
        let _ = std::process::Command::new("explorer.exe").spawn();
    }
}

/// Whether any process with this image name is running.
fn process_running(image: &str) -> bool {
    std::process::Command::new("tasklist")
        .args(["/fi", &format!("IMAGENAME eq {image}"), "/nh"])
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .to_lowercase()
                .contains(&image.to_lowercase())
        })
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// The elevated half
// ---------------------------------------------------------------------------

fn machine_install() -> Result<(), String> {
    if !is_elevated() {
        return Err("this step needs administrator rights".into());
    }
    let dir = install_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    std::fs::create_dir_all(dir.join("x86")).ok();

    let x64_dll = dir.join("xlit_tsf.dll");
    let x86_dll = dir.join("x86").join("xlit_tsf.dll");

    // Deregister before overwriting: a mapped DLL cannot be replaced, and the
    // deregister is what makes the hosts let go of it.
    let _ = selfreg::unregister(&x64_dll);
    restart_input_hosts();
    std::thread::sleep(std::time::Duration::from_millis(600));

    let Some(x64) = payload::X64 else {
        return Err("this installer was built without the 64-bit DLL — rebuild it after \
                    `cargo build -p xlit-tsf --target x86_64-pc-windows-msvc --release`"
            .into());
    };
    write_file(&x64_dll, x64)?;
    println!("installed {}", x64_dll.display());

    if let Some(x86) = payload::X86 {
        write_file(&x86_dll, x86)?;
        println!("installed {}", x86_dll.display());
    }
    if let Some(cfg) = payload::CONFIG {
        let p = dir.join("xlit-config.exe");
        write_file(&p, cfg)?;
        println!("installed {}", p.display());
    }

    // The installer is its own uninstaller, so it has to live somewhere stable.
    let me = std::env::current_exe().map_err(|e| format!("{e}"))?;
    let installed_me = dir.join("xlit-install.exe");
    if me != installed_me {
        write_file(&installed_me, &std::fs::read(&me).map_err(|e| format!("{e}"))?)?;
    }

    // 64-bit registration: the DLL's own DllRegisterServer. This is the call
    // that registers the TSF profile and the TIP categories, machine-wide.
    selfreg::register(&x64_dll)
        .map_err(|e| format!("registering the 64-bit text service failed: {e}"))?;
    println!("registered the 64-bit text service");

    // 32-bit registration: the CLSID keys in the 32-bit registry view, so that
    // 32-bit applications can find their DLL. See the note at the top of this
    // file for why this is not a second DllRegisterServer.
    if payload::X86.is_some() {
        match write_wow64_clsid(&x86_dll) {
            Ok(()) => println!("registered the 32-bit text service"),
            Err(e) => println!("warning: 32-bit registration failed ({e}); 32-bit applications will not get the input method"),
        }
    }

    write_uninstall_entry(&dir, &installed_me)?;
    sweep_old_copies(&dir);
    Ok(())
}

/// Delete the `.old` copies left by earlier runs. Best-effort by nature: the
/// ones still mapped by a running process fail to delete and simply wait for the
/// next install or sign-out.
fn sweep_old_copies(dir: &Path) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            sweep_old_copies(&path);
        } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("old")) {
            let _ = std::fs::remove_file(&path);
        }
    }
}

fn machine_uninstall() -> Result<(), String> {
    if !is_elevated() {
        return Err("this step needs administrator rights".into());
    }
    let dir = install_dir();
    let _ = selfreg::unregister(&dir.join("xlit_tsf.dll"));
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let _ = hklm.delete_subkey_all(format!(r"SOFTWARE\Classes\CLSID\{CLSID}"));
    if let Ok(view) = hklm.open_subkey_with_flags(r"SOFTWARE\Classes\CLSID", KEY_WRITE | KEY_WOW64_32KEY)
    {
        let _ = view.delete_subkey_all(CLSID);
    }
    let _ = hklm.delete_subkey_all(UNINSTALL_KEY);
    restart_input_hosts();
    std::thread::sleep(std::time::Duration::from_millis(400));
    // Files last, and best-effort: one still mapped by a running application
    // simply stays until the next sign-out.
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

/// Write `bytes` to `path`, moving an in-use file aside rather than failing.
///
/// A DLL mapped by a running process cannot be deleted, but it can almost always
/// be renamed, which frees the name for the new copy while the old image stays
/// mapped until that process exits.
fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if path.exists() && std::fs::write(path, bytes).is_ok() {
        return Ok(());
    }
    if path.exists() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let aside = path.with_file_name(format!("{name}.{stamp}.old"));
        std::fs::rename(path, &aside)
            .map_err(|e| format!("{} is in use and could not be moved aside: {e}", path.display()))?;
        println!("    ({name} was in use — moved aside)");
    }
    std::fs::write(path, bytes).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// The 32-bit view of `HKLM\Software\Classes\CLSID\{clsid}\InprocServer32`.
fn write_wow64_clsid(dll: &Path) -> Result<(), String> {
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let (clsid, _) = hklm
        .create_subkey_with_flags(
            format!(r"SOFTWARE\Classes\CLSID\{CLSID}"),
            KEY_WRITE | KEY_WOW64_32KEY,
        )
        .map_err(|e| format!("{e}"))?;
    clsid.set_value("", &APP_NAME).map_err(|e| format!("{e}"))?;
    let (inproc, _) = clsid
        .create_subkey_with_flags("InprocServer32", KEY_WRITE | KEY_WOW64_32KEY)
        .map_err(|e| format!("{e}"))?;
    inproc
        .set_value("", &dll.to_string_lossy().to_string())
        .map_err(|e| format!("{e}"))?;
    inproc
        .set_value("ThreadingModel", &"Apartment")
        .map_err(|e| format!("{e}"))?;
    Ok(())
}

fn write_uninstall_entry(dir: &Path, exe: &Path) -> Result<(), String> {
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let (k, _) = hklm.create_subkey(UNINSTALL_KEY).map_err(|e| format!("{e}"))?;
    let cmd = format!("\"{}\" --uninstall", exe.display());
    let size = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter_map(|e| e.metadata().ok())
                .map(|m| m.len())
                .sum::<u64>()
                / 1024
        })
        .unwrap_or(0) as u32;
    let set = |name: &str, v: &str| k.set_value(name, &v.to_string()).map_err(|e| format!("{e}"));
    set("DisplayName", APP_NAME)?;
    set("DisplayVersion", APP_VERSION)?;
    set("Publisher", PUBLISHER)?;
    set("InstallLocation", &dir.to_string_lossy())?;
    set("DisplayIcon", &dir.join("xlit_tsf.dll").to_string_lossy())?;
    set("UninstallString", &cmd)?;
    set("QuietUninstallString", &cmd)?;
    k.set_value("EstimatedSize", &size).map_err(|e| format!("{e}"))?;
    k.set_value("NoModify", &1u32).map_err(|e| format!("{e}"))?;
    k.set_value("NoRepair", &1u32).map_err(|e| format!("{e}"))?;
    Ok(())
}
