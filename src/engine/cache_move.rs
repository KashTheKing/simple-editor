//! Settings ▸ General ▸ Cache folder: check a new folder, and move the cache into it.
//!
//! The move is file by file: a file is renamed (same drive) or copied to `<name>.se-moving` and renamed
//! into place, given its old modified time (the pruner's LRU order), and only then removed from the old
//! folder. So at every moment each file is whole in one folder or the other: a move cut short (a crash,
//! a full disk, Cancel) leaves both folders consistent and simply runs again. A file another thread or
//! program has open for writing (a render in flight) is skipped and left where it is.

use std::path::{Path, PathBuf};

/// The new cache folder is fine to use: absolute, not the current one (nor inside it, nor around it),
/// not inside a folder the project's media lives in, not a drive root unless `root_ok`, and writable
/// (a probe file is written and removed).
pub fn validate(new: &Path, old: &Path, media_dirs: &[PathBuf], root_ok: bool) -> Result<(), String> {
    if new.as_os_str().is_empty() || !new.is_absolute() {
        return Err("Pick a full folder path (like D:\\SimpleEditorCache)".into());
    }
    let (n, o) = (norm(new), norm(old));
    if n == o {
        return Err("That is already the cache folder".into());
    }
    if n.starts_with(&o) || o.starts_with(&n) {
        return Err("The new cache folder can't be inside the current one, or hold it".into());
    }
    if let Some(m) = media_dirs.iter().find(|m| m.parent().is_some() && n.starts_with(norm(m))) {
        return Err(format!("{} holds this project's media: pick a folder of its own", m.display()));
    }
    if is_drive_root(new) && !root_ok {
        return Err("That is the top of a drive: tick \"Use the top of the drive\" to use it anyway".into());
    }
    std::fs::create_dir_all(new).map_err(|e| format!("Can't create {}: {e}", new.display()))?;
    let probe = new.join(".se-write-test");
    std::fs::write(&probe, b"ok").map_err(|e| format!("Can't write to {}: {e}", new.display()))?;
    let _ = std::fs::remove_file(probe);
    Ok(())
}

/// `C:\` / `D:\` (a folder with no parent).
pub fn is_drive_root(p: &Path) -> bool {
    p.is_absolute() && p.parent().is_none()
}

/// Lower case, `/` -> `\`, no trailing separator: two spellings of one folder compare equal.
fn norm(p: &Path) -> PathBuf {
    let s = p.to_string_lossy().replace('/', "\\").to_lowercase();
    PathBuf::from(s.trim_end_matches('\\').to_string() + "\\")
}

/// What a move did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Moved {
    pub files: usize,
    pub bytes: u64,
    /// Left in the old folder: open for writing (a render in flight) or locked by another program.
    pub in_use: usize,
}

fn walk(dir: &Path, out: &mut Vec<(PathBuf, u64)>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        match e.metadata() {
            Ok(m) if m.is_dir() => walk(&e.path(), out),
            Ok(m) => out.push((e.path(), m.len())),
            Err(_) => {}
        }
    }
}

/// Open `src` for reading only if nothing has it open for writing (share read, not write).
fn open_unshared(src: &Path) -> std::io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    std::fs::OpenOptions::new().read(true).share_mode(1 /* FILE_SHARE_READ */).open(src)
}

/// Move one file; Ok(false) = in use, left where it is.
fn move_file(src: &Path, dest: &Path) -> Result<bool, String> {
    let Ok(f) = open_unshared(src) else { return Ok(false) };
    if dest.exists() {
        // already made again in the new folder (it was in use when the app switched): the old copy goes
        drop(f);
        return Ok(std::fs::remove_file(src).is_ok());
    }
    if let Some(d) = dest.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let modified = f.metadata().and_then(|m| m.modified()).ok();
    drop(f);
    if std::fs::rename(src, dest).is_ok() {
        return Ok(true); // same drive: no copy
    }
    let Ok(mut f) = open_unshared(src) else { return Ok(false) };
    let mut tmp = dest.as_os_str().to_os_string();
    tmp.push(".se-moving");
    let tmp = PathBuf::from(tmp);
    let copied = (|| -> std::io::Result<()> {
        let mut out = std::fs::File::create(&tmp)?;
        std::io::copy(&mut f, &mut out)?;
        if let Some(t) = modified {
            out.set_modified(t)?;
        }
        out.sync_all()
    })();
    drop(f);
    if let Err(e) = copied.and_then(|_| std::fs::rename(&tmp, dest)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("{}: {e}", dest.display()));
    }
    // copied: the old one goes (if something opened it meanwhile it stays, and is a stale duplicate only)
    let _ = std::fs::remove_file(src);
    Ok(true)
}

/// Move everything under `from` into `to`, keeping the layout. `tick(done_bytes, total_bytes)` after each
/// file; it returns false to stop (Cancel), which ends with Err("cancelled") and both folders consistent.
/// Empty folders left in `from` are removed, and `from` itself when it ends up empty.
pub fn move_tree(from: &Path, to: &Path, mut tick: impl FnMut(u64, u64) -> bool) -> Result<Moved, String> {
    let mut files = Vec::new();
    walk(from, &mut files);
    let total: u64 = files.iter().map(|f| f.1).sum();
    let mut r = Moved::default();
    let mut done = 0u64;
    for (src, len) in files {
        if !tick(done, total) {
            return Err(crate::engine::export::CANCELLED.into());
        }
        let Ok(rel) = src.strip_prefix(from) else { continue };
        if src.extension().is_some_and(|e| e == "se-moving") {
            let _ = std::fs::remove_file(&src); // half a copy from a move that was cut short
            continue;
        }
        if move_file(&src, &to.join(rel))? {
            r.files += 1;
            r.bytes += len;
        } else {
            r.in_use += 1;
        }
        done += len;
    }
    tick(total, total);
    remove_empty_dirs(from);
    Ok(r)
}

/// The move in progress (or just finished, until `take_finished`), with its two folders.
type Job = (std::sync::Arc<crate::engine::export::Progress>, PathBuf, PathBuf);
static JOB: std::sync::Mutex<Option<Job>> = std::sync::Mutex::new(None);

/// Start moving `from` into `to` on a low-priority thread that waits while something plays.
pub fn start(from: PathBuf, to: PathBuf) -> std::sync::Arc<crate::engine::export::Progress> {
    let (f, t) = (from.clone(), to.clone());
    let prog = crate::engine::export::spawn_job("cache-move", move |p| {
        crate::media::bg::lower_thread();
        let r = move_tree(&f, &t, |done, total| {
            crate::media::bg::pause_point(|| p.is_cancelled());
            p.set(if total == 0 { 1.0 } else { done as f32 / total as f32 }, format!("{} of {} MB", done / 1_000_000, total / 1_000_000));
            !p.is_cancelled()
        })?;
        let left = if r.in_use > 0 { format!(", {} in use left in {}", r.in_use, f.display()) } else { String::new() };
        p.set(1.0, format!("moved {} files ({} MB){left}", r.files, r.bytes / 1_000_000));
        Ok(())
    });
    *JOB.lock().unwrap_or_else(|e| e.into_inner()) = Some((prog.clone(), from, to));
    prog
}

/// The move's progress while it runs (and once it ended, until `take_finished`).
pub fn running() -> Option<std::sync::Arc<crate::engine::export::Progress>> {
    JOB.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|j| j.0.clone())
}

/// A move that has ended: (what it said, or its error), and it is forgotten.
pub fn take_finished() -> Option<Result<String, String>> {
    let mut g = JOB.lock().unwrap_or_else(|e| e.into_inner());
    if !g.as_ref().is_some_and(|j| j.0.is_done()) {
        return None;
    }
    let (p, from, _) = g.take()?;
    Some(match p.error() {
        Some(e) if e == crate::engine::export::CANCELLED => Err(format!("Cache move stopped: the rest stays in {}", from.display())),
        Some(e) => Err(format!("Cache move stopped ({e}): the rest stays in {}", from.display())),
        None => Ok(format!("Cache {}", p.status())),
    })
}

fn remove_empty_dirs(dir: &Path) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                remove_empty_dirs(&e.path());
            }
        }
    }
    let _ = std::fs::remove_dir(dir); // only succeeds when empty
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("se-cmove-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_move_keeps_the_layout_and_dates_and_skips_files_being_written() {
        let d = temp("move");
        let (from, to) = (d.join("old"), d.join("new"));
        std::fs::create_dir_all(from.join("html")).unwrap();
        std::fs::write(from.join("html/a.mp4"), vec![1u8; 300]).unwrap();
        std::fs::write(from.join("b.peaks"), vec![2u8; 100]).unwrap();
        std::fs::write(from.join("busy.rgba"), vec![3u8; 50]).unwrap();
        let old_time = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
        std::fs::File::options().write(true).open(from.join("b.peaks")).unwrap().set_modified(old_time).unwrap();
        // a render in flight: open for writing
        let writer = std::fs::OpenOptions::new().write(true).open(from.join("busy.rgba")).unwrap();
        let mut ticks = 0;
        let r = move_tree(&from, &to, |_, _| {
            ticks += 1;
            true
        })
        .unwrap();
        assert_eq!(r, Moved { files: 2, bytes: 400, in_use: 1 });
        assert!(ticks >= 3);
        assert_eq!(std::fs::read(to.join("html/a.mp4")).unwrap(), vec![1u8; 300]);
        assert_eq!(std::fs::metadata(to.join("b.peaks")).unwrap().modified().unwrap(), old_time, "LRU order survives");
        assert!(!from.join("html").exists(), "emptied folders go");
        assert!(from.join("busy.rgba").is_file() && !to.join("busy.rgba").exists(), "a file being written stays put");
        drop(writer);
        // run again (resume): the rest comes over, the old folder goes
        let r = move_tree(&from, &to, |_, _| true).unwrap();
        assert_eq!((r.files, r.in_use), (1, 0));
        assert!(!from.exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_cancelled_move_leaves_every_file_whole_in_one_folder() {
        let d = temp("cancel");
        let (from, to) = (d.join("old"), d.join("new"));
        std::fs::create_dir_all(&from).unwrap();
        for i in 0..4 {
            std::fs::write(from.join(format!("{i}.bin")), vec![i as u8; 10]).unwrap();
        }
        std::fs::write(from.join("x.bin.se-moving"), b"half").unwrap(); // left by an earlier cut
        let mut n = 0;
        let r = move_tree(&from, &to, |_, _| {
            n += 1;
            n <= 2
        });
        assert_eq!(r, Err("cancelled".into()));
        for i in 0..4 {
            let name = format!("{i}.bin");
            let (a, b) = (from.join(&name).is_file(), to.join(&name).is_file());
            assert!(a != b, "{name} is in exactly one folder");
        }
        assert!(std::fs::read_dir(&to).unwrap().flatten().all(|e| e.path().extension().is_some_and(|x| x == "bin")), "no half copies");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_new_cache_folder_is_validated() {
        let d = temp("valid");
        let old = d.join("current");
        let media = d.join("footage");
        assert!(validate(Path::new("relative\\dir"), &old, &[], false).is_err());
        assert!(validate(&old, &old, &[], false).unwrap_err().contains("already"));
        assert!(validate(Path::new(&format!("{}\\", old.display().to_string().to_uppercase())), &old, &[], false).is_err(), "case and slash");
        assert!(validate(&old.join("sub"), &old, &[], false).is_err(), "inside the old one");
        assert!(validate(&d, &old, &[], false).is_err(), "holding the old one");
        assert!(validate(&media.join("c"), &old, &[media.clone()], false).unwrap_err().contains("media"));
        assert!(validate(Path::new("Z:\\"), &old, &[], false).unwrap_err().contains("top of a drive"));
        assert!(is_drive_root(Path::new("C:\\")) && !is_drive_root(&d));
        assert_eq!(validate(&d.join("fresh"), &old, &[media], false), Ok(()));
        assert!(d.join("fresh").is_dir() && !d.join("fresh/.se-write-test").exists(), "writable, probe removed");
        let _ = std::fs::remove_dir_all(&d);
    }
}
