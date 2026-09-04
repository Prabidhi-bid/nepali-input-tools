//! COM + TSF (un)registration, driven by `regsvr32`.
//!
//! Writes the in-proc-server CLSID keys under `HKLM\Software\Classes`, then uses
//! the TSF APIs to register the text service, its Nepali language profile, and
//! the keyboard-TIP category. `regsvr32` must be run elevated (HKLM writes).

use windows::core::{Error, Result, GUID, PCWSTR};
use windows::Win32::Foundation::{E_FAIL, S_FALSE};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::TextServices::{
    ITfCategoryMgr, ITfInputProcessorProfiles, CLSID_TF_CategoryMgr,
    CLSID_TF_InputProcessorProfiles, GUID_TFCAT_TIPCAP_COMLESS,
    GUID_TFCAT_TIPCAP_IMMERSIVESUPPORT, GUID_TFCAT_TIPCAP_INPUTMODECOMPARTMENT,
    GUID_TFCAT_TIPCAP_SECUREMODE, GUID_TFCAT_TIPCAP_SYSTRAYSUPPORT,
    GUID_TFCAT_TIPCAP_UIELEMENTENABLED, GUID_TFCAT_TIP_KEYBOARD,
};
use winreg::enums::HKEY_LOCAL_MACHINE;
use winreg::RegKey;

use crate::{dll_path, CLSID_XLIT, CLSID_XLIT_STR, GUID_PROFILE, LANGID_NE_NP, SERVICE_DESC};

fn io_err<E: std::fmt::Display>(e: E) -> Error {
    Error::new(E_FAIL, format!("{e}"))
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// RAII COM apartment for the registration call. `regsvr32` usually initialises
/// COM already; a second init just returns S_FALSE, which we treat as "not
/// ours to uninitialise".
struct ComGuard(bool);

impl ComGuard {
    fn enter() -> Self {
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        ComGuard(hr != S_FALSE && hr.is_ok())
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

fn clsid_key_path() -> String {
    format!(r"Software\Classes\CLSID\{CLSID_XLIT_STR}")
}

fn write_com_keys() -> Result<()> {
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let (clsid, _) = hklm.create_subkey(clsid_key_path()).map_err(io_err)?;
    clsid.set_value("", &SERVICE_DESC).map_err(io_err)?;

    let (inproc, _) = clsid.create_subkey("InprocServer32").map_err(io_err)?;
    inproc.set_value("", &dll_path()).map_err(io_err)?;
    inproc
        .set_value("ThreadingModel", &"Apartment")
        .map_err(io_err)?;
    Ok(())
}

fn delete_com_keys() {
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let _ = hklm.delete_subkey_all(clsid_key_path());
}

pub(crate) fn register() -> Result<()> {
    write_com_keys()?;

    let _com = ComGuard::enter();
    unsafe {
        let profiles: ITfInputProcessorProfiles =
            CoCreateInstance(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_INPROC_SERVER)?;
        profiles.Register(&CLSID_XLIT)?;

        let desc = wide(SERVICE_DESC);
        profiles.AddLanguageProfile(
            &CLSID_XLIT,
            LANGID_NE_NP,
            &GUID_PROFILE,
            &desc,
            &[], // no icon file yet
            0,
        )?;
        // AddLanguageProfile leaves the profile disabled; without this it never
        // shows in the taskbar language flyout or Win+Space.
        profiles.EnableLanguageProfile(&CLSID_XLIT, LANGID_NE_NP, &GUID_PROFILE, true)?;

        let categories: ITfCategoryMgr =
            CoCreateInstance(&CLSID_TF_CategoryMgr, None, CLSCTX_INPROC_SERVER)?;
        for cat in CATEGORIES {
            categories.RegisterCategory(&CLSID_XLIT, cat, &CLSID_XLIT)?;
        }
    }

    crate::debug("registered");
    let _ = PCWSTR::null(); // silence unused import in minimal builds
    Ok(())
}

pub(crate) fn unregister() -> Result<()> {
    let _com = ComGuard::enter();
    unsafe {
        if let Ok(categories) =
            CoCreateInstance::<_, ITfCategoryMgr>(&CLSID_TF_CategoryMgr, None, CLSCTX_INPROC_SERVER)
        {
            for cat in CATEGORIES {
                let _ = categories.UnregisterCategory(&CLSID_XLIT, cat, &CLSID_XLIT);
            }
        }
        if let Ok(profiles) = CoCreateInstance::<_, ITfInputProcessorProfiles>(
            &CLSID_TF_InputProcessorProfiles,
            None,
            CLSCTX_INPROC_SERVER,
        ) {
            let _ = profiles.Unregister(&CLSID_XLIT);
        }
    }
    delete_com_keys();
    crate::debug("unregistered");
    Ok(())
}

/// Categories this text service claims. `GUID_TFCAT_TIP_KEYBOARD` makes it a
/// keyboard TIP; the `TIPCAP_*` entries tell Windows 8+ it is safe to surface in
/// immersive/UWP contexts and the taskbar input indicator. Without
/// `IMMERSIVESUPPORT` the modern switcher hides the profile entirely.
const CATEGORIES: &[GUID] = &[
    GUID_TFCAT_TIP_KEYBOARD,
    GUID_TFCAT_TIPCAP_IMMERSIVESUPPORT,
    GUID_TFCAT_TIPCAP_SYSTRAYSUPPORT,
    GUID_TFCAT_TIPCAP_UIELEMENTENABLED,
    GUID_TFCAT_TIPCAP_SECUREMODE,
    GUID_TFCAT_TIPCAP_COMLESS,
    GUID_TFCAT_TIPCAP_INPUTMODECOMPARTMENT,
];

// Keep the GUID import meaningful for future icon/profile work.
#[allow(dead_code)]
const _PROFILE_REF: GUID = GUID_PROFILE;
