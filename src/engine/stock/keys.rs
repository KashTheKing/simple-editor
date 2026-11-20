//! ---- ws:gallery-stock ----
//! The user's own API keys for the keyed stock sites (Pexels, Pixabay, Freesound), in Windows Credential
//! Manager under `SimpleEditor/stock/<source>`: never in settings.json, never sent anywhere but that site's API.
//! Tests never touch the real credential store.

use super::Source;
use std::collections::HashMap;
use std::sync::Mutex;

static KEYS: Mutex<Option<HashMap<Source, Option<String>>>> = Mutex::new(None);

/// The user's own key for a keyed source (Windows Credential Manager, read once per run).
pub fn get(s: Source) -> Option<String> {
    if !s.needs_key() {
        return None;
    }
    let mut g = KEYS.lock().unwrap_or_else(|e| e.into_inner());
    let m = g.get_or_insert_with(HashMap::new);
    m.entry(s).or_insert_with(|| store::load(s)).clone()
}

/// Save (or with "" remove) the key of `s`.
pub fn set(s: Source, k: &str) -> Result<(), String> {
    let k = k.trim();
    if k.is_empty() {
        store::delete(s);
    } else {
        store::save(s, k)?;
    }
    KEYS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(s, (!k.is_empty()).then(|| k.to_string()));
    Ok(())
}

#[cfg(windows)]
mod store {
    use super::super::Source;
    fn target(s: Source) -> String {
        format!("SimpleEditor/stock/{}", s.id())
    }
    pub fn load(s: Source) -> Option<String> {
        if cfg!(test) {
            return None; // never the user's real credential store from a test
        }
        use windows::core::HSTRING;
        use windows::Win32::Security::Credentials::{CredFree, CredReadW, CREDENTIALW, CRED_TYPE_GENERIC};
        let mut p: *mut CREDENTIALW = std::ptr::null_mut();
        unsafe { CredReadW(&HSTRING::from(target(s)), CRED_TYPE_GENERIC, None, &mut p) }.ok()?;
        if p.is_null() {
            return None;
        }
        let bytes = unsafe {
            let c = &*p;
            let b = if c.CredentialBlob.is_null() {
                Vec::new()
            } else {
                std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize).to_vec()
            };
            CredFree(p as *const std::ffi::c_void);
            b
        };
        String::from_utf8(bytes).ok().filter(|s| !s.trim().is_empty())
    }
    pub fn save(s: Source, key: &str) -> Result<(), String> {
        if cfg!(test) {
            return Ok(());
        }
        use windows::core::PWSTR;
        use windows::Win32::Security::Credentials::{CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC};
        let mut t: Vec<u16> = target(s).encode_utf16().chain(Some(0)).collect();
        let mut user: Vec<u16> = "SimpleEditor".encode_utf16().chain(Some(0)).collect();
        let mut blob = key.as_bytes().to_vec();
        let cred = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: PWSTR(t.as_mut_ptr()),
            UserName: PWSTR(user.as_mut_ptr()),
            CredentialBlobSize: blob.len() as u32,
            CredentialBlob: blob.as_mut_ptr(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            ..Default::default()
        };
        unsafe { CredWriteW(&cred, 0) }.map_err(|e| e.to_string())
    }
    pub fn delete(s: Source) {
        if cfg!(test) {
            return;
        }
        use windows::core::HSTRING;
        use windows::Win32::Security::Credentials::{CredDeleteW, CRED_TYPE_GENERIC};
        let _ = unsafe { CredDeleteW(&HSTRING::from(target(s)), CRED_TYPE_GENERIC, None) };
    }
}

#[cfg(not(windows))]
mod store {
    use super::super::Source;
    pub fn load(_: Source) -> Option<String> {
        None
    }
    pub fn save(_: Source, _: &str) -> Result<(), String> {
        Err("keys are stored in Windows Credential Manager".into())
    }
    pub fn delete(_: Source) {}
}

