//! The registry, behind a trait: the real HKCU (optionally redirected under a test root key) on Windows,
//! an in-memory tree in tests. All key paths are relative to HKCU.

#[cfg(test)]
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Sz(String),
    Dword(u32),
}

pub trait Reg {
    fn set(&mut self, key: &str, name: &str, v: Val) -> Result<(), String>;
    fn get(&self, key: &str, name: &str) -> Option<Val>;
    fn exists(&self, key: &str) -> bool;
    /// Delete the key and everything under it; a missing key is fine.
    fn delete_tree(&mut self, key: &str) -> Result<(), String>;
    fn delete_value(&mut self, key: &str, name: &str) -> Result<(), String>;
    /// Value names and subkey names (to decide whether a key is now empty).
    fn is_empty(&self, key: &str) -> bool;

    fn set_sz(&mut self, key: &str, name: &str, v: &str) -> Result<(), String> {
        self.set(key, name, Val::Sz(v.to_string()))
    }
    fn get_sz(&self, key: &str, name: &str) -> Option<String> {
        match self.get(key, name)? {
            Val::Sz(s) => Some(s),
            Val::Dword(_) => None,
        }
    }
}

/// An in-memory registry (tests). Key names compare case-insensitively, as in Windows.
#[cfg(test)]
#[derive(Default, Debug)]
pub struct MemReg {
    pub keys: BTreeMap<String, BTreeMap<String, Val>>,
}

#[cfg(test)]
fn norm(k: &str) -> String {
    k.trim_matches('\\').to_ascii_lowercase()
}

#[cfg(test)]
impl MemReg {
    fn ensure(&mut self, key: &str) {
        // every ancestor exists too, like the real registry
        let k = norm(key);
        let mut acc = String::new();
        for part in k.split('\\') {
            if !acc.is_empty() {
                acc.push('\\');
            }
            acc.push_str(part);
            self.keys.entry(acc.clone()).or_default();
        }
    }
    /// Every key at or under `prefix` (tests).
    pub fn under(&self, prefix: &str) -> Vec<String> {
        let p = norm(prefix);
        self.keys.keys().filter(|k| *k == &p || k.starts_with(&format!("{p}\\"))).cloned().collect()
    }
}

#[cfg(test)]
impl Reg for MemReg {
    fn set(&mut self, key: &str, name: &str, v: Val) -> Result<(), String> {
        self.ensure(key);
        self.keys.get_mut(&norm(key)).unwrap().insert(name.to_ascii_lowercase(), v);
        Ok(())
    }
    fn get(&self, key: &str, name: &str) -> Option<Val> {
        self.keys.get(&norm(key))?.get(&name.to_ascii_lowercase()).cloned()
    }
    fn exists(&self, key: &str) -> bool {
        self.keys.contains_key(&norm(key))
    }
    fn delete_tree(&mut self, key: &str) -> Result<(), String> {
        for k in self.under(key) {
            self.keys.remove(&k);
        }
        Ok(())
    }
    fn delete_value(&mut self, key: &str, name: &str) -> Result<(), String> {
        if let Some(m) = self.keys.get_mut(&norm(key)) {
            m.remove(&name.to_ascii_lowercase());
        }
        Ok(())
    }
    fn is_empty(&self, key: &str) -> bool {
        let k = norm(key);
        self.keys.get(&k).map(|m| m.is_empty()).unwrap_or(true) && self.under(key).len() <= 1
    }
}
