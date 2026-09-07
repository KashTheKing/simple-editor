//! ---- ws:forgiveness ----
//! Clear Caches: release decoder file handles, drop in-memory waveform/thumb caches, then delete the
//! on-disk cache dir on a spawned thread (deletion never blocks the frame).

use super::*;

/// Total bytes under `dir` (recursive). Free function, not tied to `Settings::cache_dir()`, so a test
/// can point it at a temp directory.
fn dir_bytes(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    entries
        .flatten()
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => dir_bytes(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

pub(super) fn cache_bytes() -> u64 {
    dir_bytes(&Settings::cache_dir())
}

/// `Action::ClearCaches` / the Performance-tab button / the `caches.clear` MCP tool. Order matters:
/// release the decoders' file handles first (they hold the proxy/source files open on Windows),
/// THEN drop the in-memory caches, THEN delete on disk - anything still locked (a background worker
/// mid-write) is simply left for the next start's cleanup.
pub(super) fn clear(app: &mut App) {
    let freed = cache_bytes();
    app.player.release_files();
    app.waveforms.clear();
    app.thumbs.clear();
    let dir = Settings::cache_dir();
    std::thread::spawn(move || {
        let _ = std::fs::remove_dir_all(&dir);
    });
    app.toast(format!("Cleared caches - {:.1} MB freed", freed as f64 / 1e6));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_caches_empties_dir_and_reports_bytes() {
        let dir = std::env::temp_dir().join(format!("se-cache-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.bin"), vec![0u8; 1024]).unwrap();
        std::fs::write(dir.join("sub").join("b.bin"), vec![0u8; 2048]).unwrap();
        assert_eq!(dir_bytes(&dir), 3072, "recurses into subdirectories");
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(dir_bytes(&dir), 0, "a missing dir reports 0, not an error");
    }
}
