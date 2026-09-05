//! Calling a DLL's own COM self-registration, with the error kept.
//!
//! This is what `regsvr32` does — `LoadLibrary`, `GetProcAddress`, call — and
//! the only reason to reimplement it is the part regsvr32 throws away. It is a
//! GUI-subsystem program, so it cannot write to a console; a failure comes back
//! as a bare exit code (3 = the DLL would not load) with the actual Win32 error
//! discarded. That is how a missing dependency looked identical to a corrupt
//! file for as long as this was installed by script.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows::core::HRESULT;
use windows::Win32::Foundation::{FreeLibrary, GetLastError, HMODULE};
use windows::Win32::System::LibraryLoader::{
    GetProcAddress, LoadLibraryExW, LOAD_WITH_ALTERED_SEARCH_PATH,
};

/// What went wrong, in terms a person can act on.
pub enum RegError {
    /// `LoadLibrary` failed. Carries the Win32 code, which is the whole point.
    Load { code: u32 },
    /// The DLL loaded but does not export the entry point.
    MissingExport { name: &'static str },
    /// The entry point ran and returned a failure HRESULT.
    Failed { hr: HRESULT },
}

impl std::fmt::Display for RegError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegError::Load { code } => {
                write!(f, "the DLL could not be loaded (Win32 error {code}: {})", explain(*code))
            }
            RegError::MissingExport { name } => {
                write!(f, "the DLL does not export {name} — is it really the text service?")
            }
            RegError::Failed { hr } => {
                write!(f, "self-registration returned {hr:?} ({})", explain_hr(*hr))
            }
        }
    }
}

/// The handful of load failures that actually happen here, spelled out. Anything
/// else falls through to the number, which is still better than regsvr32's "3".
fn explain(code: u32) -> &'static str {
    match code {
        2 => "the file is not there",
        5 => "access denied — this needs to run elevated",
        126 => "a DLL it depends on is missing",
        193 => "wrong architecture — a 32-bit process cannot load a 64-bit DLL, or vice versa",
        1114 => "the DLL's initialisation routine failed",
        _ => "see the Windows system error list",
    }
}

fn explain_hr(hr: HRESULT) -> &'static str {
    match hr.0 as u32 {
        0x8007_0005 => "access denied — HKLM needs elevation",
        0x8000_4005 => "unspecified COM failure",
        _ => "see the HRESULT list",
    }
}

fn wide(p: &Path) -> Vec<u16> {
    OsStr::new(p).encode_wide().chain(std::iter::once(0)).collect()
}

/// Call `DllRegisterServer` (or `DllUnregisterServer`) in `dll`.
///
/// `LOAD_WITH_ALTERED_SEARCH_PATH` makes the DLL's own directory the first place
/// its dependencies are looked for, which is what a COM server installed outside
/// the search path needs.
fn call(dll: &Path, export: &'static str) -> Result<(), RegError> {
    let path = wide(dll);
    let module: HMODULE =
        unsafe { LoadLibraryExW(windows::core::PCWSTR(path.as_ptr()), None, LOAD_WITH_ALTERED_SEARCH_PATH) }
            .map_err(|_| RegError::Load { code: unsafe { GetLastError().0 } })?;

    let mut name = export.as_bytes().to_vec();
    name.push(0);
    let proc = unsafe { GetProcAddress(module, windows::core::PCSTR(name.as_ptr())) };
    let Some(proc) = proc else {
        unsafe { let _ = FreeLibrary(module); };
        return Err(RegError::MissingExport { name: export });
    };

    // The DLL's exports are `extern "system" fn() -> HRESULT`, per COM.
    let entry: extern "system" fn() -> HRESULT = unsafe { std::mem::transmute(proc) };
    let hr = entry();

    // Deliberately not freeing on success: unloading a text service immediately
    // after it registered itself has no benefit, and the process is about to
    // exit anyway. Freeing on failure keeps a retry honest.
    if hr.is_ok() {
        Ok(())
    } else {
        unsafe { let _ = FreeLibrary(module); };
        Err(RegError::Failed { hr })
    }
}

pub fn register(dll: &Path) -> Result<(), RegError> {
    call(dll, "DllRegisterServer")
}

pub fn unregister(dll: &Path) -> Result<(), RegError> {
    call(dll, "DllUnregisterServer")
}
