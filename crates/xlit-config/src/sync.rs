//! Talking to prabidhi.bid.
//!
//! Nothing is ever sent. The client only reads the shared dictionary; words
//! the user types, corrects, or adds by hand stay on their machine.
//!
//! # Wire format
//!
//! `GET https://prabidhi.bid/input/dictionary/download`, JSON over HTTPS. The
//! server does not exist yet, so this *defines* the contract rather than
//! following one.
//!
//! ```json
//! { "words": [ { "latin": "kathmandu", "text": "काठमाडौँ" } ] }
//! ```
//!
//! The reply is read leniently, so the server can be simpler than the strictest
//! reading of the above: a bare JSON array works, `input`/`chosen` are accepted
//! as field names alongside `latin`/`text`, and a plain-text body of
//! `latin<TAB>देवनागरी` lines works too.
//!
use std::fmt;

use windows::core::{w, PCWSTR};
use windows::Win32::Networking::WinHttp::{
    WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryDataAvailable,
    WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest,
    INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE,
    WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE,
};

const HOST: PCWSTR = w!("prabidhi.bid");
const PATH_DOWNLOAD: PCWSTR = w!("/input/dictionary/download");
const AGENT: PCWSTR = w!("xlit-config/0.1.0");

pub enum SyncError {
    /// No network, DNS failure, TLS failure — retry later.
    Offline(String),
    /// Reached the server and it said no.
    Http(u32),
    /// Reached the server and could not make sense of the reply.
    Malformed(String),
}

impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SyncError::Offline(e) => write!(f, "offline ({e})"),
            SyncError::Http(404) => write!(f, "server has no such endpoint (404)"),
            SyncError::Http(c) => write!(f, "server returned {c}"),
            SyncError::Malformed(e) => write!(f, "unexpected reply ({e})"),
        }
    }
}

/// Fetch the shared dictionary.
pub fn download() -> Result<Vec<(String, String)>, SyncError> {
    let (status, body) = request(PATH_DOWNLOAD)?;
    if !(200..300).contains(&status) {
        return Err(SyncError::Http(status));
    }
    parse_words(&String::from_utf8_lossy(&body))
}

/// Pull `latin`/`text` pairs out of whatever the server sent.
///
/// Deliberately forgiving: this contract is being defined before the server
/// exists, and a client that rejects a nearly-right reply would be a poor way
/// to find that out.
fn parse_words(body: &str) -> Result<Vec<(String, String)>, SyncError> {
    let trimmed = body.trim_start();

    // Tab-separated lines, if it plainly is not JSON.
    if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
        let mut out = Vec::new();
        for line in body.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((a, b)) = line.split_once('\t') {
                let (a, b) = (a.trim(), b.trim());
                if !a.is_empty() && !b.is_empty() {
                    out.push((a.to_string(), b.to_string()));
                }
            }
        }
        return if out.is_empty() {
            Err(SyncError::Malformed("not JSON, and no tab-separated pairs".into()))
        } else {
            Ok(out)
        };
    }

    // Otherwise scan for objects and take the first and second string fields we
    // recognise, under either naming.
    let mut out = Vec::new();
    for obj in trimmed.split('{').skip(1) {
        let obj = obj.split('}').next().unwrap_or("");
        let latin = json_field(obj, "latin").or_else(|| json_field(obj, "input"));
        let text = json_field(obj, "text")
            .or_else(|| json_field(obj, "chosen"))
            .or_else(|| json_field(obj, "devanagari"));
        if let (Some(l), Some(t)) = (latin, text) {
            if !l.is_empty() && !t.is_empty() {
                out.push((l, t));
            }
        }
    }
    if out.is_empty() {
        Err(SyncError::Malformed("no latin/text pairs in the reply".into()))
    } else {
        Ok(out)
    }
}

/// Value of `"key":"..."` within one flat JSON object, honouring backslash
/// escapes for the two characters that can appear inside one.
fn json_field(obj: &str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\"");
    let start = obj.find(&pat)? + pat.len();
    let rest = obj[start..].trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let mut val = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(val),
            '\\' => match chars.next() {
                Some('n') => val.push('\n'),
                Some('t') => val.push('\t'),
                Some('u') => {
                    let hex: String = chars.by_ref().take(4).collect();
                    match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                        Some(c) => val.push(c),
                        None => return None,
                    }
                }
                Some(other) => val.push(other),
                None => return None,
            },
            c => val.push(c),
        }
    }
    None
}

/// One HTTPS GET.
fn request(path: PCWSTR) -> Result<(u32, Vec<u8>), SyncError> {
    unsafe {
        let session = WinHttpOpen(
            AGENT,
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        );
        if session.is_null() {
            return Err(SyncError::Offline("could not start WinHTTP".into()));
        }
        let _s = Handle(session);

        let conn = WinHttpConnect(session, HOST, INTERNET_DEFAULT_HTTPS_PORT, 0);
        if conn.is_null() {
            return Err(SyncError::Offline(format!("connect failed: {}", last_error())));
        }
        let _c = Handle(conn);

        let req = WinHttpOpenRequest(
            conn,
            w!("GET"),
            path,
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null_mut(),
            WINHTTP_FLAG_SECURE,
        );
        if req.is_null() {
            return Err(SyncError::Offline(format!("request failed: {}", last_error())));
        }
        let _r = Handle(req);

        let headers = w!("Accept: application/json");
        if WinHttpSendRequest(req, Some(headers.as_wide()), None, 0, 0, 0).is_err() {
            return Err(SyncError::Offline(format!("send failed: {}", last_error())));
        }
        if WinHttpReceiveResponse(req, std::ptr::null_mut()).is_err() {
            return Err(SyncError::Offline(format!("no response: {}", last_error())));
        }

        let mut status: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        if WinHttpQueryHeaders(
            req,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(&mut status as *mut u32 as *mut core::ffi::c_void),
            &mut size,
            std::ptr::null_mut(),
        )
        .is_err()
        {
            return Err(SyncError::Malformed("no status code".into()));
        }

        let mut out = Vec::new();
        loop {
            let mut avail: u32 = 0;
            if WinHttpQueryDataAvailable(req, &mut avail).is_err() || avail == 0 {
                break;
            }
            let mut chunk = vec![0u8; avail as usize];
            let mut read: u32 = 0;
            if WinHttpReadData(
                req,
                chunk.as_mut_ptr() as *mut core::ffi::c_void,
                avail,
                &mut read,
            )
            .is_err()
                || read == 0
            {
                break;
            }
            chunk.truncate(read as usize);
            out.extend_from_slice(&chunk);
            // A dictionary should never be this big; stop rather than let a
            // misbehaving endpoint exhaust memory.
            if out.len() > 8 * 1024 * 1024 {
                break;
            }
        }
        Ok((status, out))
    }
}

fn last_error() -> u32 {
    unsafe { windows::Win32::Foundation::GetLastError().0 }
}

/// Closes a WinHTTP handle on the way out of `request`, on every path.
struct Handle(*mut core::ffi::c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { _ = WinHttpCloseHandle(self.0) };
    }
}
