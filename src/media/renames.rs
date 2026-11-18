//! Files renamed outside a project (the player's click-to-rename): the player writes `(from, to)` to
//! `%APPDATA%\SimpleEditor\renames.json`, and any editor - this process or another one, now or later -
//! repoints Library assets that went offline under `from` (`ui::app::media_sync`, when the offline scan
//! changes). Kept as a short log, not consumed: every project that holds the file gets relinked.

use std::path::{Path, PathBuf};

/// Entries kept (oldest dropped first).
const KEEP: usize = 200;

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub struct Rename {
    pub from: String,
    pub to: String,
}

pub fn log_path() -> PathBuf {
    crate::settings::Settings::dir().join("renames.json")
}

pub fn load(p: &Path) -> Vec<Rename> {
    std::fs::read(p).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Add `from -> to` to the log at `p`.
pub fn record(p: &Path, from: &Path, to: &Path) {
    let mut v = load(p);
    v.push(Rename { from: from.to_string_lossy().into_owned(), to: to.to_string_lossy().into_owned() });
    let over = v.len().saturating_sub(KEEP);
    v.drain(..over);
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Ok(b) = serde_json::to_vec(&v) {
        let _ = std::fs::write(p, b);
    }
}

/// Repoint every asset whose file is gone and was renamed to the newest name in the log (a -> b -> c
/// lands on c), when that file exists. Returns how many assets changed.
pub fn apply(project: &mut crate::model::Project, log: &[Rename]) -> usize {
    let mut n = 0;
    for (k, r) in log.iter().enumerate() {
        let hits: Vec<usize> = (0..project.assets.len()).filter(|&i| project.assets[i].path.eq_ignore_ascii_case(&r.from)).collect();
        if hits.is_empty() || Path::new(&r.from).exists() {
            continue;
        }
        // renamed again later: follow it to the last name
        let mut to = r.to.clone();
        for later in &log[k + 1..] {
            if later.from.eq_ignore_ascii_case(&to) {
                to = later.to.clone();
            }
        }
        if !Path::new(&to).is_file() {
            continue;
        }
        for i in hits {
            project.assets[i].path = to.clone();
            n += 1;
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file renamed in the player: an editor's Library asset (and its subclip) follows it, through
    /// a chain of renames; a file that is still there, or a new name that is gone, is left alone.
    #[test]
    fn a_rename_in_the_player_relinks_the_library_asset() {
        let dir = std::env::temp_dir().join(format!("se-renames-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b, c) = (dir.join("a.mp4"), dir.join("b.mp4"), dir.join("c.mp4"));
        std::fs::write(&a, b"x").unwrap();
        let mut p = crate::model::Project::new();
        let id = p.add_asset(crate::engine::import::placeholder(&a.to_string_lossy()));
        p.add_subclip(id, 0.0, 1.0, None).unwrap(); // a subclip shares the path
        let log = dir.join("renames.json");
        std::fs::rename(&a, &b).unwrap();
        record(&log, &a, &b);
        std::fs::rename(&b, &c).unwrap();
        record(&log, &b, &c);
        assert_eq!(apply(&mut p, &load(&log)), 2);
        assert!(p.assets.iter().all(|x| Path::new(&x.path) == c), "{:?}", p.assets[0].path);
        assert_eq!(apply(&mut p, &load(&log)), 0, "applied once");
        // a file still at its old path is not moved; a log entry whose new file is gone does nothing
        let mut q = crate::model::Project::new();
        q.add_asset(crate::engine::import::placeholder(&c.to_string_lossy()));
        assert_eq!(apply(&mut q, &[Rename { from: keep_path(&c), to: keep_path(&b) }]), 0, "c is still there");
        let mut r = crate::model::Project::new();
        r.add_asset(crate::engine::import::placeholder(&a.to_string_lossy()));
        assert_eq!(apply(&mut r, &[Rename { from: keep_path(&a), to: keep_path(&dir.join("gone.mp4")) }]), 0, "the new name is gone");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn keep_path(p: &Path) -> String {
        p.to_string_lossy().into_owned()
    }
}
