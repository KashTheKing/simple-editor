//! ---- ws:forgiveness ----
//! Debounced, off-thread autosave with rolling backups. `autosave_tick` is a FRAME_HOOK; the pure
//! decision core (`tick_pure`/`should_skip`) is split out so it's testable without a live `App` - see
//! `tools_registry_tests.rs`'s doc comment for why one isn't buildable in `#[test]`.
//!
//! deviation from the plan text: the plan's own risk table proposed reusing the "already-serialized
//! top-of-undo-stack JSON" to avoid a second `to_json()` call. That JSON is the snapshot from BEFORE
//! the most recent edit (`push_undo_json` snapshots pre-edit, matching every other undo push in this
//! crate) - reusing it would silently autosave a one-edit-stale project. Autosave only fires once every
//! `autosave_secs` (default 30s), so a single extra `to_json()` there is immaterial; this workstream
//! serializes the LIVE project at fire time instead, which is correct.

use super::*;
use std::time::SystemTime;

#[derive(Default)]
pub(super) struct AutosaveState {
    pub(super) due: Option<Instant>,
    #[allow(dead_code)] // read by a future diagnostics surface; written every successful write today
    pub(super) last_ok: Option<Instant>,
}

/// Project-name-or-"untitled" + a short pid suffix, so two instances editing different (or the same)
/// project never clobber each other's rolling autosave directory.
fn project_slug(project_path: Option<&Path>) -> String {
    let stem = project_path
        .and_then(|p| p.file_stem())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "untitled".into());
    format!("{stem}-{}", std::process::id())
}

fn slug_dir(slug: &str) -> PathBuf {
    Settings::autosave_dir().join(slug)
}

/// Writes `json` as a new rolling file in `dir`, then trims to the newest 20 by mtime. Takes the
/// directory directly (rather than resolving it from `Settings` internally) so a test can point it at
/// a temp dir.
fn write_and_trim(dir: &Path, json: &str) -> Option<PathBuf> {
    std::fs::create_dir_all(dir).ok()?;
    // seconds resolution: two writes within the same second overwrite each other - acceptable, the
    // debounce interval is measured in tens of seconds.
    // ponytail: second-resolution filenames, upgrade to a monotonic counter if a sub-second autosave
    // cadence is ever wanted.
    let secs = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).ok()?.as_secs();
    let path = dir.join(format!("{secs}.sedit"));
    std::fs::write(&path, json).ok()?;
    let mut files: Vec<(PathBuf, SystemTime)> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|e| e.metadata().ok().and_then(|m| m.modified().ok()).map(|t| (e.path(), t)))
        .collect();
    files.sort_by_key(|(_, t)| *t);
    while files.len() > 20 {
        let (oldest, _) = files.remove(0);
        let _ = std::fs::remove_file(oldest);
    }
    Some(path)
}

/// Deletes every rolling autosave for this project's slug - called after a clean save (there's nothing
/// left to recover past the file just written).
pub(super) fn clear_for(project_path: &Path) {
    let _ = std::fs::remove_dir_all(slug_dir(&project_slug(Some(project_path))));
}

/// `project.autosave` MCP tool: force-writes regardless of the debounce; returns the path written.
pub(super) fn force_write(app: &mut App) -> Option<PathBuf> {
    let json = app.project.to_json();
    recovery::note_snapshot(json.clone());
    let slug = project_slug(app.project_path.as_deref());
    write_and_trim(&slug_dir(&slug), &json)
}

/// True when autosave must not fire this frame at all: an export is running (never write over/along a
/// file the export thread might touch), or the project is both untitled AND empty (nothing to lose).
pub(super) fn should_skip(export_running: bool, project_path_set: bool, project_empty: bool) -> bool {
    export_running || (!project_path_set && project_empty)
}

/// Pure decision core: given whether the project is dirty, the current `due` deadline, "now" and the
/// configured interval, returns the new `due` and whether autosave should fire THIS frame.
pub(super) fn tick_pure(
    dirty: bool,
    due: Option<Instant>,
    now: Instant,
    interval_secs: u32,
) -> (Option<Instant>, bool) {
    if interval_secs == 0 || !dirty {
        return (None, false);
    }
    let due = due.unwrap_or_else(|| now + Duration::from_secs(interval_secs as u64));
    if now >= due {
        (None, true)
    } else {
        (Some(due), false)
    }
}

/// FRAME_HOOK.
pub(super) fn autosave_tick(app: &mut App, ctx: &egui::Context) {
    if should_skip(app.export.is_some(), app.project_path.is_some(), app.project.is_empty()) {
        return;
    }
    let (due, fire) = tick_pure(app.dirty, app.autosave.due, Instant::now(), app.settings.autosave_secs);
    app.autosave.due = due;
    if !fire {
        if let Some(d) = due {
            app.animate_until(ctx, d);
        }
        return;
    }
    let json = app.project.to_json();
    recovery::note_snapshot(json.clone());
    let slug = project_slug(app.project_path.as_deref());
    let dir = slug_dir(&slug);
    std::thread::spawn(move || {
        write_and_trim(&dir, &json);
    });
    app.autosave.last_ok = Some(Instant::now());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autosave_arms_once_and_writes_after_debounce() {
        let base = Instant::now();
        // dirty, no deadline yet: arms one 5s out, does not fire
        let (due, fire) = tick_pure(true, None, base, 5);
        assert!(!fire);
        let due = due.expect("must arm a deadline");
        // still before the deadline: keeps waiting
        let (due2, fire2) = tick_pure(true, Some(due), base + Duration::from_secs(2), 5);
        assert!(!fire2);
        assert_eq!(due2, Some(due), "the deadline must not move while still dirty and unreached");
        // at/after the deadline: fires exactly once and clears due
        let (due3, fire3) = tick_pure(true, Some(due), due, 5);
        assert!(fire3);
        assert_eq!(due3, None);

        // the actual write: exactly one file lands in a temp autosave dir
        let dir = std::env::temp_dir().join(format!("se-autosave-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = write_and_trim(&dir, "{\"fake\":true}").expect("write must succeed");
        assert!(path.exists());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn autosave_skips_while_untitled_and_while_exporting() {
        // untitled AND empty: skip
        assert!(should_skip(false, false, true));
        // untitled but non-empty (opened straight from a media file, never saved): must NOT skip
        assert!(!should_skip(false, false, false));
        // has a path: never skip on that basis alone
        assert!(!should_skip(false, true, true));
        // exporting always skips, regardless of the rest
        assert!(should_skip(true, true, false));
    }

    #[test]
    fn write_and_trim_keeps_only_the_newest_20() {
        let dir = std::env::temp_dir().join(format!("se-autosave-trim-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..25u32 {
            std::fs::write(dir.join(format!("fake-{i:03}.sedit")), "{}").unwrap();
        }
        write_and_trim(&dir, "{\"newest\":true}");
        let remaining = std::fs::read_dir(&dir).unwrap().count();
        assert_eq!(remaining, 20, "26 files written, trimmed to the newest 20");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
