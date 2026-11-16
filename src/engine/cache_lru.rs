//! ws:cache-budget: one disk budget for the html, prerender, proxy and peaks caches, pruned least recently
//! used first. Pure over a directory (a test points it at a temp dir). Models, the library and the other
//! small files in the cache dir are never counted or deleted.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Sub-folders of the cache dir that count against the budget (peaks are `*.peaks` in the dir itself).
const FOLDERS: [&str; 3] = ["html", "prerender", "proxies"];
/// A file touched this recently may be mid-write: never deleted.
const FRESH: Duration = Duration::from_secs(120);
/// `touch_if_stale` rewrites an mtime at most this often per file.
const STALE: Duration = Duration::from_secs(600);

/// Files kept by name stem (the file name up to its first '.': the content hash).
#[derive(Default)]
pub struct Protect {
    /// Evicted only when nothing else is left to reach the budget (the open project's proxies, prerender seconds).
    pub soft: HashSet<String>,
    /// Never evicted (html outputs the open project's assets point at).
    pub hard: HashSet<String>,
}

#[derive(Debug, Default, PartialEq)]
pub struct Pruned {
    pub files: usize,
    pub bytes: u64,
}

struct Entry {
    path: PathBuf,
    len: u64,
    used: SystemTime,
    stem: String,
}

pub fn stem_of(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().split('.').next().unwrap_or("").to_string()).unwrap_or_default()
}

/// The budget in bytes; `None` = unlimited (0 GB).
pub fn budget_bytes(gb: u32) -> Option<u64> {
    (gb > 0).then(|| gb as u64 * (1 << 30))
}

/// The cache drive is short of room: less than twice the budget is free.
pub fn low_on_space(free: u64, budget: Option<u64>) -> bool {
    budget.is_some_and(|b| free < b.saturating_mul(2))
}

fn entries(root: &Path) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut take = |dir: PathBuf, only_peaks: bool| {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let Ok(m) = e.metadata() else { continue };
            let p = e.path();
            if !m.is_file() || (only_peaks && p.extension().is_none_or(|x| x != "peaks")) {
                continue;
            }
            // mtime only (bumped by `touch_if_stale` on use): Windows access times are lazy and set at creation
            let used = m.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            out.push(Entry { len: m.len(), used, stem: stem_of(&p), path: p });
        }
    };
    for f in FOLDERS {
        take(root.join(f), false);
    }
    take(root.to_path_buf(), true);
    out
}

/// Bytes under budget (what the pruner counts).
pub fn budgeted_bytes(root: &Path) -> u64 {
    entries(root).iter().map(|e| e.len).sum()
}

/// Delete least recently used files until the budgeted total fits `budget`. Order: unprotected oldest
/// first, then `soft` oldest first; `hard` and fresh files never. A file that will not delete (a decoder
/// has it open) is skipped. Protected files are touched (their "last use" is now).
pub fn prune(root: &Path, budget: u64, protect: &Protect, now: SystemTime) -> Pruned {
    let mut files = entries(root);
    let protected: Vec<PathBuf> = files
        .iter()
        .filter(|e| protect.soft.contains(&e.stem) || protect.hard.contains(&e.stem))
        .map(|e| e.path.clone())
        .collect();
    let mut total: u64 = files.iter().map(|e| e.len).sum();
    let mut done = Pruned::default();
    let tier = |e: &Entry| protect.soft.contains(&e.stem) as u8;
    files.retain(|e| !protect.hard.contains(&e.stem) && now.duration_since(e.used).is_ok_and(|age| age >= FRESH));
    files.sort_by_key(|e| (tier(e), e.used));
    for e in files {
        if total <= budget {
            break;
        }
        if std::fs::remove_file(&e.path).is_ok() {
            total = total.saturating_sub(e.len);
            done.files += 1;
            done.bytes += e.len;
        }
    }
    protected.iter().for_each(|p| touch_if_stale(p));
    done
}

/// Mark `p` as used now (its mtime), unless that was done recently. Errors are ignored: a locked file just keeps its age.
pub fn touch_if_stale(p: &Path) {
    let Ok(m) = std::fs::metadata(p) else { return };
    let fresh = m.modified().ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age < STALE);
    if !fresh {
        if let Ok(f) = std::fs::OpenOptions::new().write(true).open(p) {
            let _ = f.set_modified(SystemTime::now());
        }
    }
}

/// Free bytes on the drive holding `dir` (None when it cannot be read).
pub fn free_bytes(dir: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    let mut dir = dir;
    while !dir.exists() {
        dir = dir.parent()?;
    }
    let w: Vec<u16> = dir.as_os_str().encode_wide().chain([0]).collect();
    let mut free = 0u64;
    // SAFETY: `w` is NUL-terminated and outlives the call; `free` is a valid out pointer.
    // (declared here: the `windows` crate's FileSystem feature would rebuild the whole crate)
    #[link(name = "kernel32")]
    extern "system" {
        fn GetDiskFreeSpaceExW(dir: *const u16, avail: *mut u64, total: *mut u64, free: *mut u64) -> i32;
    }
    if unsafe { GetDiskFreeSpaceExW(w.as_ptr(), &mut free, std::ptr::null_mut(), std::ptr::null_mut()) } == 0 {
        return None;
    }
    Some(free)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("se-lru-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Write `len` bytes whose last use was `age_s` seconds ago.
    fn put(root: &Path, rel: &str, len: usize, age_s: u64) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, vec![0u8; len]).unwrap();
        let f = std::fs::OpenOptions::new().write(true).open(&p).unwrap();
        f.set_modified(SystemTime::now() - Duration::from_secs(age_s)).unwrap();
    }

    fn left(root: &Path, rel: &str) -> bool {
        root.join(rel).exists()
    }

    #[test]
    fn budget_math() {
        assert_eq!(budget_bytes(0), None);
        assert_eq!(budget_bytes(30), Some(30 << 30));
        assert!(low_on_space(59 << 30, budget_bytes(30)));
        assert!(!low_on_space(60 << 30, budget_bytes(30)));
        assert!(!low_on_space(0, None), "unlimited never warns");
        assert!(free_bytes(&std::env::temp_dir()).is_some_and(|f| f > 0));
    }

    #[test]
    fn prunes_oldest_first_across_caches_and_spares_the_rest() {
        let d = temp("order");
        put(&d, "html/a.mov", 100, 5000);
        put(&d, "prerender/b.rgba", 100, 4000);
        put(&d, "proxies/c.mp4", 100, 3000);
        put(&d, "e.peaks", 100, 2000);
        put(&d, "models/big.bin", 10_000, 9000); // not budgeted: never counted, never deleted
        assert_eq!(budgeted_bytes(&d), 400);
        let r = prune(&d, 250, &Protect::default(), SystemTime::now());
        assert_eq!(r, Pruned { files: 2, bytes: 200 });
        assert!(!left(&d, "html/a.mov") && !left(&d, "prerender/b.rgba"));
        assert!(left(&d, "proxies/c.mp4") && left(&d, "e.peaks") && left(&d, "models/big.bin"));
        assert_eq!(prune(&d, 1000, &Protect::default(), SystemTime::now()), Pruned::default(), "under budget: nothing");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn protection_and_fresh_files() {
        let d = temp("protect");
        put(&d, "html/hard.mov", 100, 9000); // oldest, but the open project uses it
        put(&d, "proxies/soft.mp4", 100, 8000); // open project's proxy: last to go
        put(&d, "proxies/other.mp4", 100, 700);
        put(&d, "proxies/new.mp4", 100, 1); // fresh: maybe mid-write
        let mut p = Protect::default();
        p.hard.insert("hard".into());
        p.soft.insert("soft".into());
        prune(&d, 300, &p, SystemTime::now());
        assert!(left(&d, "html/hard.mov") && left(&d, "proxies/new.mp4"));
        assert!(!left(&d, "proxies/other.mp4"), "unprotected goes before the older soft one");
        assert!(left(&d, "proxies/soft.mp4"));
        // still over budget with only soft left: soft goes, hard and fresh never
        put(&d, "proxies/soft.mp4", 100, 8000);
        prune(&d, 0, &p, SystemTime::now());
        assert!(!left(&d, "proxies/soft.mp4"));
        assert!(left(&d, "html/hard.mov") && left(&d, "proxies/new.mp4"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn touching_a_protected_file_refreshes_its_age() {
        let d = temp("touch");
        put(&d, "proxies/soft.mp4", 10, 9000);
        let mut p = Protect::default();
        p.soft.insert("soft".into());
        prune(&d, 100, &p, SystemTime::now());
        let age = std::fs::metadata(d.join("proxies/soft.mp4")).unwrap().modified().unwrap().elapsed().unwrap();
        assert!(age < Duration::from_secs(60));
        let _ = std::fs::remove_dir_all(&d);
    }
}
