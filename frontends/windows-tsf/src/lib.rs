//! xlit — Windows TSF text input processor (M6.1: registrable no-op).
//!
//! This DLL is a COM in-process server loaded by Windows into every process
//! that takes text input. Stage M6.1 gets it *registered* and *activatable*
//! (visible in the Windows language bar / input-method list); it does not yet
//! intercept keys — that's M6.2.
//!
//! Build:  cargo build -p xlit-tsf --release
//! Install (elevated): regsvr32 target\release\xlit_tsf.dll
//! Remove  (elevated): regsvr32 /u target\release\xlit_tsf.dll

#![cfg(windows)]
#![allow(non_snake_case)]
#![allow(clippy::missing_safety_doc)]

mod bar;
mod candwin;
mod editsession;
mod engine;
mod keysink;
mod register;
mod service;
mod session;
mod wordeditor;

use core::ffi::c_void;
use std::sync::atomic::{AtomicIsize, AtomicPtr, Ordering};

use windows::core::{implement, Interface, Ref, Result, GUID, HRESULT};
#[cfg(feature = "trace")]
use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_POINTER, HMODULE, S_FALSE, S_OK,
};
use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl};
use windows::Win32::System::LibraryLoader::{DisableThreadLibraryCalls, GetModuleFileNameW};
use windows::Win32::UI::TextServices::ITfTextInputProcessor;

use crate::service::TextService;

/// COM class id of this text service. Must match `register.rs` and the docs.
pub(crate) const CLSID_XLIT: GUID = GUID::from_u128(0x438E43E4_3800_4AB1_82A6_A2E831ABF107);
/// Language-profile id for the Nepali profile.
pub(crate) const GUID_PROFILE: GUID = GUID::from_u128(0x4BE59555_69DD_48CA_8BC8_AB450205A567);
/// LANGID for ne-NP (primary 0x61, sublang 0x01).
pub(crate) const LANGID_NE_NP: u16 = 0x0461;

pub(crate) const CLSID_XLIT_STR: &str = "{438E43E4-3800-4AB1-82A6-A2E831ABF107}";
/// Display name shown in the Windows language bar / keyboard list.
pub(crate) const SERVICE_DESC: &str = "Input by Prabidhi.bid";

// ---------------------------------------------------------------------------
// Module state
// ---------------------------------------------------------------------------

static MODULE_REFS: AtomicIsize = AtomicIsize::new(0);
static DLL_HMODULE: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());

pub(crate) fn module_add_ref() {
    MODULE_REFS.fetch_add(1, Ordering::SeqCst);
}
pub(crate) fn module_release() {
    MODULE_REFS.fetch_sub(1, Ordering::SeqCst);
}

pub(crate) fn dll_hmodule() -> HMODULE {
    HMODULE(DLL_HMODULE.load(Ordering::SeqCst))
}

/// Full path of this DLL on disk (for COM registration).
pub(crate) fn dll_path() -> String {
    let mut buf = [0u16; 1024];
    let n = unsafe { GetModuleFileNameW(Some(dll_hmodule()), &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..n])
}

/// Emit a trace line, to the Win32 debugger (DebugView) *and* to
/// `%APPDATA%\xlit\xlit-tsf.log`.
///
/// The file matters more than the debugger in practice: this DLL runs inside
/// other people's processes on someone else's machine, where attaching a
/// debugger is a big ask and a log that can be read afterwards is not.
///
/// Compiled to a no-op unless the `trace` feature is set, so release and
/// installer builds neither leak internal flow nor touch the disk on every
/// keystroke. Troubleshoot with `--features trace`.
#[cfg(feature = "trace")]
pub(crate) fn debug(msg: &str) {
    let mut w: Vec<u16> = format!("[xlit-tsf] {msg}\r\n").encode_utf16().collect();
    w.push(0);
    unsafe { windows::Win32::System::Diagnostics::Debug::OutputDebugStringW(PCWSTR(w.as_ptr())) };

    // Best-effort and never fatal: a failed trace write must not disturb the
    // application we are living inside. Every process appends, so the line
    // carries the process id to keep them apart.
    use std::io::Write;
    let Some(appdata) = std::env::var_os("APPDATA") else { return };
    let dir = std::path::PathBuf::from(appdata).join("xlit");
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("xlit-tsf.log"))
    {
        let exe = std::env::current_exe()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "?".into());
        let _ = writeln!(f, "[{}:{}] {msg}", exe, std::process::id());
    }
}

#[cfg(not(feature = "trace"))]
pub(crate) fn debug(_msg: &str) {}

// ---------------------------------------------------------------------------
// DllMain
// ---------------------------------------------------------------------------

#[no_mangle]
extern "system" fn DllMain(hinst: HMODULE, reason: u32, _reserved: *mut c_void) -> i32 {
    const DLL_PROCESS_ATTACH: u32 = 1;
    if reason == DLL_PROCESS_ATTACH {
        DLL_HMODULE.store(hinst.0, Ordering::SeqCst);
        unsafe { _ = DisableThreadLibraryCalls(hinst) };
    }
    1
}

// ---------------------------------------------------------------------------
// COM entry points
// ---------------------------------------------------------------------------

#[no_mangle]
extern "system" fn DllCanUnloadNow() -> HRESULT {
    if MODULE_REFS.load(Ordering::SeqCst) == 0 {
        S_OK
    } else {
        S_FALSE
    }
}

#[no_mangle]
extern "system" fn DllGetClassObject(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    unsafe {
        if ppv.is_null() || rclsid.is_null() || riid.is_null() {
            return E_POINTER;
        }
        *ppv = core::ptr::null_mut();
        if *rclsid != CLSID_XLIT {
            return CLASS_E_CLASSNOTAVAILABLE;
        }
        debug("DllGetClassObject: our CLSID requested");
        let factory: IClassFactory = ClassFactory.into();
        factory.query(riid, ppv)
    }
}

#[no_mangle]
extern "system" fn DllRegisterServer() -> HRESULT {
    match register::register() {
        Ok(()) => S_OK,
        Err(e) => {
            debug(&format!("register failed: {e}"));
            e.code()
        }
    }
}

#[no_mangle]
extern "system" fn DllUnregisterServer() -> HRESULT {
    match register::unregister() {
        Ok(()) => S_OK,
        Err(e) => {
            debug(&format!("unregister failed: {e}"));
            e.code()
        }
    }
}

// ---------------------------------------------------------------------------
// Class factory
// ---------------------------------------------------------------------------

#[implement(IClassFactory)]
struct ClassFactory;

impl IClassFactory_Impl for ClassFactory_Impl {
    fn CreateInstance(
        &self,
        punkouter: Ref<'_, windows::core::IUnknown>,
        riid: *const GUID,
        ppvobject: *mut *mut c_void,
    ) -> Result<()> {
        if !punkouter.is_null() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        crate::debug("CreateInstance: text service created");
        let service: ITfTextInputProcessor = TextService::new().into();
        unsafe { service.query(riid, ppvobject).ok() }
    }

    fn LockServer(&self, flock: windows::core::BOOL) -> Result<()> {
        if flock.as_bool() {
            module_add_ref();
        } else {
            module_release();
        }
        Ok(())
    }
}

