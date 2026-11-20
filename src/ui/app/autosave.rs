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

/// The newest autosaves always kept (10 minutes at the default 30 s).
const KEEP_NEWEST: usize = 20;
/// forum-crash: and one per hour for this long, so "restore from an hour ago" works (a forum report:
/// hourly backups that weren't there).
const KEEP_HOURLY_SECS: u64 = 48 * 3600;

/// Which rolling files to delete, given each one's time (unix seconds) and now: everything but the
/// newest `KEEP_NEWEST` and the newest file of each clock hour within `KEEP_HOURLY_SECS`.
pub(super) fn to_delete(files: &[(PathBuf, u64)], now: u64) -> Vec<PathBuf> {
    let mut v: Vec<&(PathBuf, u64)> = files.iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.0.cmp(&a.0))); // newest first
    let mut hours_kept = std::collections::HashSet::new();
    let mut out = Vec::new();
    for (i, (p, t)) in v.into_iter().enumerate() {
        let recent = now.saturating_sub(*t) <= KEEP_HOURLY_SECS;
        let first_of_hour = recent && hours_kept.insert(t / 3600);
        if i >= KEEP_NEWEST && !first_of_hour {
            out.push(p.clone());
        }
    }
    out
}

/// A rolling file's time: its name (`<unix secs>.sedit`), else its mtime.
fn file_secs(path: &Path, mtime: SystemTime) -> u64 {
    path.file_stem()
        .and_then(|s| s.to_str())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or_else(|| mtime.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0))
}

/// Writes `json` as a new rolling file in `dir`, then trims it (`to_delete`). Takes the directory
/// directly (rather than resolving it from `Settings` internally) so a test can point it at a temp dir.
fn write_and_trim(dir: &Path, json: &str) -> Option<PathBuf> {
    std::fs::create_dir_all(dir).ok()?;
    // seconds resolution: two writes within the same second overwrite each other - acceptable, the
    // debounce interval is measured in tens of seconds.
    // ponytail: second-resolution filenames, upgrade to a monotonic counter if a sub-second autosave
    // cadence is ever wanted.
    let secs = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).ok()?.as_secs();
    let path = dir.join(format!("{secs}.sedit"));
    std::fs::write(&path, json).ok()?;
    let files: Vec<(PathBuf, u64)> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|e| e.metadata().ok().and_then(|m| m.modified().ok()).map(|t| (file_secs(&e.path(), t), e.path())))
        .map(|(t, p)| (p, t))
        .collect();
    for old in to_delete(&files, secs) {
        let _ = std::fs::remove_file(old);
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
    // (ws:timeline-tabs: clips on any timeline, not just the tab in front)
    if should_skip(app.export.is_some(), app.project_path.is_some(), app.project.no_clips_anywhere()) {
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

    /// forum-crash: three hours of 30 s autosaves keep the newest 20 plus one per hour, and a restore
    /// from "about two hours ago" finds a file within that hour.
    #[test]
    fn hourly_backups_survive_the_rolling_trim() {
        let now: u64 = 1_800_000_000 - 1_800_000_000 % 3600 + 1800; // half past an hour
        let files: Vec<(PathBuf, u64)> =
            (0..360u64).map(|i| (PathBuf::from(format!("{}.sedit", now - i * 30)), now - i * 30)).collect();
        let gone = to_delete(&files, now);
        let kept: Vec<u64> = files.iter().filter(|(p, _)| !gone.contains(p)).map(|(_, t)| *t).collect();
        // 20 newest (the last 10 minutes) + the newest of each of the 3 earlier clock hours it spans
        assert_eq!(kept.len(), 23, "{kept:?}");
        for h in 1..=3u64 {
            let hour = now / 3600 - h;
            assert_eq!(kept.iter().filter(|t| *t / 3600 == hour).count(), 1, "one file for hour -{h}");
        }
        let want = now - 2 * 3600;
        let best = kept.iter().min_by_key(|t| t.abs_diff(want)).unwrap();
        assert!(best.abs_diff(want) <= 3600, "a backup within an hour of the chosen time");
        // a file older than two days goes, even when it is the only one of its hour
        let old = vec![(PathBuf::from("1.sedit"), now - 49 * 3600)];
        let mut many: Vec<(PathBuf, u64)> = files[..20].to_vec();
        many.extend(old);
        assert_eq!(to_delete(&many, now), vec![PathBuf::from("1.sedit")]);
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
