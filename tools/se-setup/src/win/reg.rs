//! The real registry: HKCU, or a test root key under it (`WinReg::under`).

use super::wide;
use crate::reg::{Reg, Val};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::*;

pub struct WinReg {
    root: String,
}

impl WinReg {
    pub fn hkcu() -> WinReg {
        WinReg { root: String::new() }
    }
    #[cfg(test)]
    /// Every key below `root` (a key under HKCU): tests write here instead of the live keys.
    pub fn under(root: &str) -> WinReg {
        WinReg { root: root.trim_matches('\\').to_string() }
    }
    fn path(&self, key: &str) -> String {
        if self.root.is_empty() {
            key.to_string()
        } else {
            format!(r"{}\{}", self.root, key.trim_matches('\\'))
        }
    }
    fn open(&self, key: &str, access: REG_SAM_FLAGS) -> Option<Key> {
        let mut h: HKEY = null_mut();
        let w = wide(&self.path(key));
        let r = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, w.as_ptr(), 0, access, &mut h) };
        (r == ERROR_SUCCESS).then_some(Key(h))
    }
    fn create_key(&self, key: &str) -> Result<Key, String> {
        let mut h: HKEY = null_mut();
        let w = wide(&self.path(key));
        let r = unsafe { RegCreateKeyExW(HKEY_CURRENT_USER, w.as_ptr(), 0, null(), REG_OPTION_NON_VOLATILE, KEY_READ | KEY_WRITE, null(), &mut h, null_mut()) };
        if r == ERROR_SUCCESS {
            Ok(Key(h))
        } else {
            Err(format!(r"HKCU\{}: error {r}", self.path(key)))
        }
    }
}

struct Key(HKEY);
impl Drop for Key {
    fn drop(&mut self) {
        unsafe { RegCloseKey(self.0) };
    }
}

impl Reg for WinReg {
    fn set(&mut self, key: &str, name: &str, v: Val) -> Result<(), String> {
        let k = self.create_key(key)?;
        let n = wide(name);
        let r = match v {
            Val::Sz(s) => {
                let d = wide(&s);
                unsafe { RegSetValueExW(k.0, n.as_ptr(), 0, REG_SZ, d.as_ptr() as *const u8, (d.len() * 2) as u32) }
            }
            Val::Dword(x) => unsafe { RegSetValueExW(k.0, n.as_ptr(), 0, REG_DWORD, &x as *const u32 as *const u8, 4) },
        };
        if r == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(format!(r"HKCU\{} {name}: error {r}", self.path(key)))
        }
    }
    fn get(&self, key: &str, name: &str) -> Option<Val> {
        let k = self.open(key, KEY_READ)?;
        let n = wide(name);
        let (mut ty, mut len) = (0u32, 0u32);
        if unsafe { RegQueryValueExW(k.0, n.as_ptr(), null(), &mut ty, null_mut(), &mut len) } != ERROR_SUCCESS {
            return None;
        }
        let mut buf = vec![0u8; len as usize + 2];
        if unsafe { RegQueryValueExW(k.0, n.as_ptr(), null(), &mut ty, buf.as_mut_ptr(), &mut len) } != ERROR_SUCCESS {
            return None;
        }
        match ty {
            REG_SZ | REG_EXPAND_SZ => {
                let u: Vec<u16> = buf[..len as usize].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                let end = u.iter().position(|&c| c == 0).unwrap_or(u.len());
                Some(Val::Sz(String::from_utf16_lossy(&u[..end])))
            }
            REG_DWORD if len >= 4 => Some(Val::Dword(u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]))),
            _ => None,
        }
    }
    fn exists(&self, key: &str) -> bool {
        self.open(key, KEY_READ).is_some()
    }
    fn delete_tree(&mut self, key: &str) -> Result<(), String> {
        let w = wide(&self.path(key));
        let r = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, w.as_ptr()) };
        // RegDeleteTree leaves the key itself when given a subkey path; delete it too
        let r2 = unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, w.as_ptr()) };
        if [r, r2].iter().all(|&x| x == ERROR_SUCCESS || x == ERROR_FILE_NOT_FOUND) {
            Ok(())
        } else {
            Err(format!(r"HKCU\{}: error {r}/{r2}", self.path(key)))
        }
    }
    fn delete_value(&mut self, key: &str, name: &str) -> Result<(), String> {
        let Some(k) = self.open(key, KEY_SET_VALUE) else { return Ok(()) };
        let n = wide(name);
        let r = unsafe { RegDeleteValueW(k.0, n.as_ptr()) };
        if r == ERROR_SUCCESS || r == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            Err(format!(r"HKCU\{} {name}: error {r}", self.path(key)))
        }
    }
    fn is_empty(&self, key: &str) -> bool {
        let Some(k) = self.open(key, KEY_READ) else { return true };
        let (mut subkeys, mut values) = (0u32, 0u32);
        let r = unsafe {
            RegQueryInfoKeyW(k.0, null_mut(), null_mut(), null(), &mut subkeys, null_mut(), null_mut(), &mut values, null_mut(), null_mut(), null_mut(), null_mut())
        };
        r == ERROR_SUCCESS && subkeys == 0 && values == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The same install / uninstall, against the REAL registry under a throwaway test root.
    #[test]
    fn real_registry_round_trip_under_a_test_root() {
        let root = format!(r"Software\SimpleEditorSetupTest\{}", std::process::id());
        let mut r = WinReg::under(&root);
        let t = crate::testutil::TempDir::new("winreg");
        let p = crate::paths::Paths::under(t.path());
        std::fs::create_dir_all(&p.install).unwrap();
        crate::setup::register(&mut r, &p, "1.4.0", true).unwrap();
        assert_eq!(r.get_sz(crate::shell_keys::UNINSTALL_KEY, "DisplayVersion").as_deref(), Some("1.4.0"));
        assert!(r.exists(&crate::shell_keys::menu_key("mkv")));
        crate::setup::unregister(&mut r).unwrap();
        assert!(!r.exists(crate::shell_keys::UNINSTALL_KEY));
        assert!(!r.exists(&crate::shell_keys::menu_key("mkv")));
        assert!(!r.exists(crate::shell_keys::SEDIT_KEY));
        WinReg::hkcu().delete_tree(&root).unwrap();
    }
}
