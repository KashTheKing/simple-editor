//! Downloads through WinHTTP (the system's proxy settings, TLS and redirects; nothing bundled).

use super::wide;
use crate::net::Net;
use std::ffi::c_void;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Networking::WinHttp::*;

pub struct WinNet;

struct H(*mut c_void);
impl Drop for H {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { WinHttpCloseHandle(self.0) };
        }
    }
}

fn last_error(what: &str) -> String {
    let e = unsafe { windows_sys::Win32::Foundation::GetLastError() };
    let hint = match e {
        12007 => " (no internet connection, or the name could not be found)",
        12002 => " (timed out)",
        12029 | 12030 => " (the connection failed)",
        12175 | 12045 | 12169 => " (a secure connection could not be made)",
        _ => "",
    };
    format!("{what} failed: error {e}{hint}")
}

impl WinNet {
    fn request(&self, url: &str, sink: &mut dyn FnMut(&[u8], Option<u64>) -> bool) -> Result<(), String> {
        let wurl = wide(url);
        let mut host = vec![0u16; 256];
        let mut path = vec![0u16; 4096];
        let mut uc: URL_COMPONENTS = unsafe { std::mem::zeroed() };
        uc.dwStructSize = std::mem::size_of::<URL_COMPONENTS>() as u32;
        uc.lpszHostName = host.as_mut_ptr();
        uc.dwHostNameLength = host.len() as u32;
        uc.lpszUrlPath = path.as_mut_ptr();
        uc.dwUrlPathLength = path.len() as u32;
        let mut extra = vec![0u16; 4096];
        uc.lpszExtraInfo = extra.as_mut_ptr();
        uc.dwExtraInfoLength = extra.len() as u32;
        if unsafe { WinHttpCrackUrl(wurl.as_ptr(), 0, 0, &mut uc) } == 0 {
            return Err(format!("not a web address: {url}"));
        }
        let host_s = String::from_utf16_lossy(&host[..uc.dwHostNameLength as usize]);
        let object = format!(
            "{}{}",
            String::from_utf16_lossy(&path[..uc.dwUrlPathLength as usize]),
            String::from_utf16_lossy(&extra[..uc.dwExtraInfoLength as usize])
        );
        let secure = uc.nScheme == WINHTTP_INTERNET_SCHEME_HTTPS;
        let agent = wide(concat!("SimpleEditorSetup/", env!("CARGO_PKG_VERSION")));
        // the system's automatic proxy (Windows 8.1+), else the configured one
        let mut session = H(unsafe { WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, null(), null(), 0) });
        if session.0.is_null() {
            session = H(unsafe { WinHttpOpen(agent.as_ptr(), WINHTTP_ACCESS_TYPE_DEFAULT_PROXY, null(), null(), 0) });
        }
        if session.0.is_null() {
            return Err(last_error("WinHttpOpen"));
        }
        // TLS 1.2 / 1.3 explicitly (older Windows 10 builds default to less)
        let protocols: u32 = WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2 | WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_3;
        unsafe { WinHttpSetOption(session.0, WINHTTP_OPTION_SECURE_PROTOCOLS, &protocols as *const u32 as *const c_void, 4) };
        unsafe { WinHttpSetTimeouts(session.0, 30_000, 30_000, 30_000, 60_000) };
        let whost = wide(&host_s);
        let conn = H(unsafe { WinHttpConnect(session.0, whost.as_ptr(), uc.nPort, 0) });
        if conn.0.is_null() {
            return Err(last_error("connecting"));
        }
        let verb = wide("GET");
        let wobj = wide(&object);
        let req = H(unsafe { WinHttpOpenRequest(conn.0, verb.as_ptr(), wobj.as_ptr(), null(), null(), null(), if secure { WINHTTP_FLAG_SECURE } else { 0 }) });
        if req.0.is_null() {
            return Err(last_error("the request"));
        }
        let headers = if host_s.eq_ignore_ascii_case("api.github.com") {
            wide("Accept: application/vnd.github+json\r\nX-GitHub-Api-Version: 2022-11-28\r\n")
        } else {
            wide("")
        };
        let hlen = if headers.len() > 1 { u32::MAX } else { 0 };
        if unsafe { WinHttpSendRequest(req.0, if hlen > 0 { headers.as_ptr() } else { null() }, hlen, null(), 0, 0, 0) } == 0 {
            return Err(last_error("sending the request"));
        }
        if unsafe { WinHttpReceiveResponse(req.0, null_mut()) } == 0 {
            return Err(last_error("the response"));
        }
        let number = |what: u32| -> Option<u64> {
            let mut v: u32 = 0;
            let mut len: u32 = 4;
            let ok = unsafe { WinHttpQueryHeaders(req.0, what | WINHTTP_QUERY_FLAG_NUMBER, null(), &mut v as *mut u32 as *mut c_void, &mut len, null_mut()) };
            (ok != 0).then_some(v as u64)
        };
        let status = number(WINHTTP_QUERY_STATUS_CODE).unwrap_or(0);
        if status != 200 {
            return Err(match status {
                403 | 429 if host_s.eq_ignore_ascii_case("api.github.com") => "GitHub is limiting requests from this network; try again in an hour".into(),
                404 => format!("not found: {url}"),
                s => format!("the server answered {s} for {url}"),
            });
        }
        let total = number(WINHTTP_QUERY_CONTENT_LENGTH);
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let mut got: u32 = 0;
            if unsafe { WinHttpReadData(req.0, buf.as_mut_ptr() as *mut c_void, buf.len() as u32, &mut got) } == 0 {
                return Err(last_error("downloading"));
            }
            if got == 0 {
                return Ok(());
            }
            if !sink(&buf[..got as usize], total) {
                return Err(crate::report::CANCELLED.into());
            }
        }
    }
}

impl Net for WinNet {
    fn get(&self, url: &str) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        self.request(url, &mut |c, _| {
            out.extend_from_slice(c);
            out.len() < 16 << 20
        })?;
        Ok(out)
    }
    fn stream(&self, url: &str, sink: &mut dyn FnMut(&[u8], Option<u64>) -> bool) -> Result<(), String> {
        self.request(url, sink)
    }
}
