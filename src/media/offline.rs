//! ---- forum-crash ----
//! Media files taken offline after the app died on them (`crashguard`). Every in-process open
//! (`media::probe`, `open_video`, `open_audio`) refuses an offline path, so one bad file shows as
//! missing instead of taking the app down again on every start. Kept in `offline.json` in the settings
//! folder until the user brings the file back ("Try again" on the recovery prompt).
//!
//! A safe start ("Recover without media") takes every asset offline for this run only: `session`
//! entries are never written to the file.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};

#[derive(Default)]
struct Set {
    /// Persisted: the files a previous run died on.
    saved: HashSet<String>,
    /// This run only (a safe start).
    session: HashSet<String>,
    file: Option<PathBuf>,
}

fn set() -> &'static RwLock<Set> {
    static S: OnceLock<RwLock<Set>> = OnceLock::new();
    S.get_or_init(Default::default)
}

/// Case-insensitive, slash-agnostic key (Windows paths).
fn key(path: &str) -> String {
    path.replace('/', "\\").to_lowercase()
}

/// Load the saved list from `dir/offline.json` (main, once).
pub fn init(dir: PathBuf) {
    let file = dir.join("offline.json");
    let saved: HashSet<String> = std::fs::read_to_string(&file)
        .ok()
        .and_then(|j| serde_json::from_str::<Vec<String>>(&j).ok())
        .unwrap_or_default()
        .iter()
        .map(|p| key(p))
        .collect();
    let mut s = set().write().unwrap_or_else(|e| e.into_inner());
    s.saved = saved;
    s.file = Some(file);
}

fn persist(s: &Set) {
    let Some(f) = &s.file else { return };
    if s.saved.is_empty() {
        let _ = std::fs::remove_file(f);
        return;
    }
    let mut v: Vec<&String> = s.saved.iter().collect();
    v.sort();
    if let Ok(j) = serde_json::to_string(&v) {
        // a few hundred bytes, written once after a crash or a "Try again": fine on the UI thread
        let _ = std::fs::write(f, j);
    }
}

/// Cheap (a read lock and a hash): called on every decoder open.
pub fn is_offline(path: &str) -> bool {
    let s = set().read().unwrap_or_else(|e| e.into_inner());
    if s.saved.is_empty() && s.session.is_empty() {
        return false;
    }
    let k = key(path);
    s.saved.contains(&k) || s.session.contains(&k)
}

/// Take `path` offline; `remember` keeps it across restarts.
pub fn take_offline(path: &str, remember: bool) {
    let mut s = set().write().unwrap_or_else(|e| e.into_inner());
    if remember {
        if s.saved.insert(key(path)) {
            persist(&s);
        }
    } else {
        s.session.insert(key(path));
    }
}

/// Bring `path` back online (the next open tries it again).
pub fn bring_back(path: &str) {
    let mut s = set().write().unwrap_or_else(|e| e.into_inner());
    let k = key(path);
    s.session.remove(&k);
    if s.saved.remove(&k) {
        persist(&s);
    }
}

/// Bring every file back (the safe start's "Bring media back").
pub fn bring_all_back() {
    let mut s = set().write().unwrap_or_else(|e| e.into_inner());
    s.session.clear();
    if !s.saved.is_empty() {
        s.saved.clear();
        persist(&s);
    }
}

/// The error an open of an offline file returns.
pub const REFUSED: &str = "offline: Simple Editor closed while reading this file last time";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_files_are_refused_until_brought_back() {
        let p = r"C:\Clips\Crashy.MP4";
        assert!(!is_offline(p));
        take_offline(p, false);
        assert!(is_offline("c:/clips/crashy.mp4"), "case and slashes do not matter on Windows");
        assert!(crate::media::open_video(p, crate::media::Backend::Auto).err().unwrap().starts_with("offline"));
        assert!(crate::media::probe(p, crate::media::Backend::Auto).unwrap_err().starts_with("offline"));
        bring_back(p);
        assert!(!is_offline(p));
    }
}
