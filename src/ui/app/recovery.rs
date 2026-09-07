//! ---- ws:forgiveness ----
//! Crash recovery: a panic hook that writes crash.log (plus the latest known autosaved JSON), and a
//! non-blocking "recover unsaved project?" offer shown once at startup when a newer autosave exists
//! than the currently open project file.

use super::*;
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

/// Refreshed by `autosave::autosave_tick`/`force_write` on every write attempt, so a panic can dump the
/// freshest known project state even though the write itself happens on its own spawned thread.
static LAST_SNAPSHOT: OnceLock<Mutex<Option<String>>> = OnceLock::new();

pub(super) fn note_snapshot(json: String) {
    // best-effort: a poisoned mutex (another thread panicked mid-write) just means the crash dump is
    // missing, never worth panicking a SECOND time over.
    if let Ok(mut g) = LAST_SNAPSHOT.get_or_init(|| Mutex::new(None)).lock() {
        *g = Some(json);
    }
}

/// Every autosave file across every project's slug directory, newest first.
pub fn backups() -> Vec<(PathBuf, SystemTime)> {
    let mut out = Vec::new();
    let Ok(projects) = std::fs::read_dir(Settings::autosave_dir()) else { return out };
    for proj_dir in projects.flatten() {
        let Ok(files) = std::fs::read_dir(proj_dir.path()) else { continue };
        for f in files.flatten() {
            if let Some(t) = f.metadata().ok().and_then(|m| m.modified().ok()) {
                out.push((f.path(), t));
            }
        }
    }
    out.sort_by(|a, b| b.1.cmp(&a.1));
    out
}

/// Pure half of `recover_candidate`: the newest of `candidates` strictly newer than `baseline` (split
/// out so a test can exercise it with synthetic timestamps instead of real files).
pub(super) fn pick_newest(candidates: &[(PathBuf, SystemTime)], baseline: SystemTime) -> Option<PathBuf> {
    candidates.iter().filter(|(_, t)| *t > baseline).max_by_key(|(_, t)| *t).map(|(p, _)| p.clone())
}

/// The newest autosave strictly newer than `project_path`'s saved mtime (or "now - 1h" for an untitled
/// project) - `None` when there's nothing worth offering.
pub fn recover_candidate(project_path: Option<&Path>) -> Option<PathBuf> {
    let baseline = match project_path {
        Some(p) => std::fs::metadata(p).and_then(|m| m.modified()).ok()?,
        None => SystemTime::now().checked_sub(Duration::from_secs(3600))?,
    };
    pick_newest(&backups(), baseline)
}

/// `main.rs`'s only new call, before `eframe::run_native`.
pub fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let dir = Settings::dir();
        let _ = std::fs::create_dir_all(&dir);
        let snapshot = LAST_SNAPSHOT.get_or_init(|| Mutex::new(None)).lock().ok().and_then(|g| g.clone());
        let bytes = snapshot.as_deref().map(str::len).unwrap_or(0);
        let body = format!(
            "{info}\n\n---\nLatest known autosaved project ({bytes} bytes) follows:\n{}",
            snapshot.unwrap_or_default()
        );
        let _ = std::fs::write(dir.join("crash.log"), body);
        default(info);
    }));
}

/// Called once from the existing first-frame `window_shown` gate (see boot.rs): queues a non-blocking
/// Recover offer when a newer autosave exists than the currently open project.
pub(super) fn boot(app: &mut App) {
    let Some(path) = recover_candidate(app.project_path.as_deref()) else { return };
    let path2 = path.clone();
    crate::ui::confirm::ask_app(
        "Recover unsaved project?",
        format!("A newer autosave was found:\n{}\n\nLoad it?", path.display()),
        move |app| match std::fs::read_to_string(&path2).ok().and_then(|j| Project::from_json(&j).ok()) {
            Some(project) => {
                app.set_project(project, app.project_path.clone());
                app.dirty = true; // the recovered state hasn't been saved as-is yet
                app.toast("Recovered the autosaved project");
            }
            None => app.toast("Couldn't read that autosave"),
        },
    );
}

/// WINDOW_DRAWER: `Action::RestoreBackup`'s "Restore Autosave…" window - lists every autosave across
/// every project by mtime; picking one loads it via `App::set_project` (after a labeled undo push if
/// the current project is dirty, so the in-progress edit isn't silently lost).
pub(super) fn restore_window(app: &mut App, ctx: &egui::Context) {
    if !app.restore_backup_open {
        return;
    }
    let mut open = true;
    let mut pick: Option<PathBuf> = None;
    egui::Window::new("Restore Autosave…").open(&mut open).default_width(420.0).show(ctx, |ui| {
        let list = backups();
        if list.is_empty() {
            ui.weak("No autosaves found.");
        }
        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            for (path, t) in &list {
                let age = SystemTime::now().duration_since(*t).unwrap_or_default();
                ui.horizontal(|ui| {
                    ui.label(format!("{} - {} s ago", path.display(), age.as_secs()));
                    if ui.small_button("Load").clicked() {
                        pick = Some(path.clone());
                    }
                });
            }
        });
    });
    app.restore_backup_open = open;
    if let Some(path) = pick {
        if let Some(project) = std::fs::read_to_string(&path).ok().and_then(|j| Project::from_json(&j).ok()) {
            let before = app.project.to_json();
            app.project = project;
            app.push_undo_labeled(before, "Restore backup");
            app.after_edit();
            app.toast_with_folder("Backup restored", path);
        } else {
            app.toast("Couldn't read that autosave");
        }
        app.restore_backup_open = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_prefers_newer_autosave() {
        let now = SystemTime::now();
        let older = now - Duration::from_secs(120);
        let newer = now + Duration::from_secs(60);
        let candidates = vec![(PathBuf::from("a.sedit"), older), (PathBuf::from("b.sedit"), newer)];
        assert_eq!(pick_newest(&candidates, now), Some(PathBuf::from("b.sedit")));
        // nothing newer than the baseline: None
        assert_eq!(pick_newest(&candidates, newer), None);
    }
}
