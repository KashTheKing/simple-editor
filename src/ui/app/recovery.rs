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
        // forum-crash: a panic on the UI thread takes the app down: keep what was in flight for the next
        // start (worker panics are caught and survived, so they leave the fingerprint alone)
        if std::thread::current().name() == Some("main") {
            crate::crashguard::flush_for_crash();
        }
        default(info);
    }));
}

/// Called once from the existing first-frame `window_shown` gate (see boot.rs): queues a non-blocking
/// recovery prompt when the last run crashed (forum-crash: naming the file or action it died on, with
/// that media file already offline) or a newer autosave exists than the open project. Every recover
/// choice has a safe-start twin: without media (every file offline for this run) or without effects.
pub(super) fn boot(app: &mut App) {
    use crate::crashguard::{self, Kind};
    let left = crashguard::leftovers();
    // a graphics death was answered in `main` (compatibility mode), before the window existed
    let crash: Vec<crashguard::Entry> = left.entries.iter().filter(|e| e.kind != Kind::Graphics).cloned().collect();
    let culprits = crashguard::culprit_files(&crash);
    for c in &culprits {
        crate::media::offline::take_offline(c, true); // even on a quiet start: the file must not crash it again
    }
    if crate::winpos::quiet() {
        return; // no prompt on a quiet launch; the autosave stays for the next normal launch
    }
    let crash_project = (!crash.is_empty() && !left.project.is_empty()).then(|| PathBuf::from(&left.project));
    let project_file = app.project_path.clone().or(crash_project);
    let autosave = recover_candidate(project_file.as_deref());
    let said = crashguard::describe(&crash);
    let Some(plan) = RecoverPlan::new(said, culprits, autosave, project_file) else { return };
    let (title, body) = plan.text();
    let mut choices: Vec<(&'static str, &'static str, Box<dyn FnOnce(&mut App)>)> = Vec::new();
    if plan.loads() {
        for mode in [SafeStart::Normal, SafeStart::NoMedia, SafeStart::NoEffects] {
            let p = plan.clone();
            choices.push((mode.label(), mode.tip(), Box::new(move |app: &mut App| p.recover(app, mode))));
        }
    }
    if !plan.culprits.is_empty() {
        let files = plan.culprits.clone();
        choices.push((
            "Bring the File Back",
            "Try the file again: the next time it is played or imported, Simple Editor opens it as usual",
            Box::new(move |app: &mut App| {
                for f in &files {
                    crate::media::offline::bring_back(f);
                }
                app.player.set_project(&app.project); // decoders reopen it
                app.toast("The file is back online");
            }),
        ));
    }
    crate::ui::confirm::ask_many(title, body, choices);
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum SafeStart {
    Normal,
    NoMedia,
    NoEffects,
}

impl SafeStart {
    fn label(self) -> &'static str {
        match self {
            SafeStart::Normal => "Recover",
            SafeStart::NoMedia => "Recover Without Media",
            SafeStart::NoEffects => "Recover Without Effects",
        }
    }
    fn tip(self) -> &'static str {
        match self {
            SafeStart::Normal => "Open the project as it was, from the newest autosave when there is one",
            SafeStart::NoMedia => {
                "Open it with every media file offline for this session, so no file is decoded. \
                 Edits still work; restart to bring the media back"
            }
            SafeStart::NoEffects => "Open it with every effect switched off (one Undo turns them back on)",
        }
    }
}

/// What the recovery prompt offers, decided from the crash record and the autosaves (pure, testable).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct RecoverPlan {
    pub(super) said: Option<String>,
    pub(super) culprits: Vec<String>,
    pub(super) autosave: Option<PathBuf>,
    pub(super) project_file: Option<PathBuf>,
}

impl RecoverPlan {
    /// None when there is nothing to say or offer.
    pub(super) fn new(
        said: Option<String>,
        culprits: Vec<String>,
        autosave: Option<PathBuf>,
        project_file: Option<PathBuf>,
    ) -> Option<Self> {
        (said.is_some() || autosave.is_some()).then_some(Self { said, culprits, autosave, project_file })
    }

    /// There is a project to (re)open: an autosave, or after a crash the project file itself.
    pub(super) fn loads(&self) -> bool {
        self.autosave.is_some() || (self.said.is_some() && self.project_file.as_deref().is_some_and(Path::is_file))
    }

    pub(super) fn text(&self) -> (String, String) {
        let mut body = Vec::new();
        if let Some(s) = &self.said {
            body.push(s.clone());
        }
        match self.culprits.as_slice() {
            [] => {}
            [one] => body.push(format!(
                "{} is offline for now, so the project opens without it.",
                crate::crashguard::short(one)
            )),
            many => body.push(format!("{} files are offline for now, so the project opens without them.", many.len())),
        }
        if let Some(a) = &self.autosave {
            let age = std::fs::metadata(a)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| SystemTime::now().duration_since(t).ok())
                .map(|d| format!(" (saved {})", ago(d.as_secs())))
                .unwrap_or_default();
            body.push(format!("A newer autosave was found{age}:\n{}", a.display()));
        }
        let title = if self.said.is_some() { "Simple Editor closed unexpectedly" } else { "Recover unsaved project?" };
        (title.to_string(), body.join("\n\n"))
    }

    fn recover(&self, app: &mut App, mode: SafeStart) {
        let project = match &self.autosave {
            Some(a) => std::fs::read_to_string(a).ok().and_then(|j| Project::from_json(&j).ok()),
            None => self.project_file.as_deref().and_then(|p| Project::load(p).ok()),
        };
        let Some(project) = project else {
            return app.toast("Couldn't read that autosave");
        };
        if mode == SafeStart::NoMedia {
            // before set_project, so no decoder opens a file first
            for f in crate::model::repair::media_paths(&project) {
                crate::media::offline::take_offline(&f, false);
            }
        }
        let path = app.project_path.clone().or_else(|| self.project_file.clone());
        app.set_project(project, path);
        if self.autosave.is_some() {
            app.dirty = true; // the recovered state hasn't been saved as-is yet
            app.saved = None;
        }
        match mode {
            SafeStart::Normal => app.toast("Recovered the project"),
            SafeStart::NoMedia => app.toast("Recovered with the media offline. Restart to bring it back."),
            SafeStart::NoEffects => {
                let before = app.project.to_json();
                let n = crate::model::repair::disable_all_effects(&mut app.project);
                if n > 0 {
                    app.push_undo_labeled(before, "Switch off effects");
                    app.after_edit();
                    app.toast_undo(format!("Recovered with {n} effects switched off"), Action::Undo);
                } else {
                    app.toast("Recovered the project (it has no effects to switch off)"); // no Undo: nothing to undo
                }
            }
        }
    }
}

/// A rolling autosave's project, from its folder (`<project stem>-<pid>`).
pub(super) fn backup_name(path: &Path) -> String {
    // the parent folder's name, split by hand so either separator works
    let text = path.to_string_lossy();
    let dir = text.rsplit(['\\', '/']).filter(|s| !s.is_empty()).nth(1).unwrap_or_default().to_string();
    match dir.rsplit_once('-') {
        Some((stem, pid)) if pid.chars().all(|c| c.is_ascii_digit()) && !stem.is_empty() => stem.to_string(),
        _ => dir,
    }
}

thread_local! {
    /// Effect count per clip after the last edit (`note_new_effects`).
    static FX_COUNTS: std::cell::RefCell<std::collections::HashMap<Id, usize>> = Default::default();
}

/// How long a newly added effect stays "in flight": its first renders happen on the render thread.
const FILTER_LINGER_SECS: f32 = 8.0;

/// forum-crash: called after every edit. A clip that gained an effect is fingerprinted for a few
/// seconds (`crashguard::Kind::Filter`), so a crash in that effect's first renders names the effect and
/// the clip's file. One pass over the open timeline's clips, no allocation beyond the count map.
pub(super) fn note_new_effects(project: &Project) {
    FX_COUNTS.with(|m| {
        let mut m = m.borrow_mut();
        let mut next = std::collections::HashMap::with_capacity(m.len());
        for c in project.tracks.iter().flat_map(|t| t.clips.iter()) {
            let n = c.effects.len();
            if let Some(&before) = m.get(&c.id) {
                if n > before {
                    let path = project.asset(c.asset).map(|a| a.path.as_str()).unwrap_or("");
                    let name = c.effects.last().map(|e| e.name()).unwrap_or_default();
                    crate::crashguard::linger(crate::crashguard::Kind::Filter, path, &name, FILTER_LINGER_SECS);
                }
            }
            next.insert(c.id, n);
        }
        *m = next;
    });
}

/// "45 s ago", "12 min ago", "3 h ago", "2 days ago".
pub(super) fn ago(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs} s ago"),
        60..=3599 => format!("{} min ago", secs / 60),
        3600..=86_399 => format!("{} h ago", secs / 3600),
        _ => format!("{} days ago", secs / 86_400),
    }
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
        // forum-crash: aligned rows, the project's name and the age in their own columns (the newest
        // 20 plus one per hour for two days, so "an hour ago" is a row here)
        const CTRLS: &[crate::ui::list_rows::Ctrl] =
            &[crate::ui::list_rows::Ctrl { glyph: "↺", hint: "Load this autosave (Undo goes back)" }];
        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            let mut rows = crate::ui::list_rows::Rows::new(ui, "restore_autosave", CTRLS, &[90.0], false).fixed_order();
            for (i, (path, t)) in list.iter().enumerate() {
                let age = ago(SystemTime::now().duration_since(*t).unwrap_or_default().as_secs());
                let name = backup_name(path);
                let spec = crate::ui::list_rows::RowSpec { key: i as u64, label: &name, values: &[age.as_str()], ..Default::default() };
                let r = rows.row(ui, i, &spec);
                let twice = r.body.double_clicked();
                r.body.on_hover_text(path.display().to_string());
                if r.ctrl == Some(0) || twice {
                    pick = Some(path.clone());
                }
            }
            let _ = rows.finish(ui);
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

    /// forum-crash: the prompt names the culprit file, says it is offline, and offers the safe starts.
    #[test]
    fn crash_prompt_names_the_file_and_offers_safe_starts() {
        assert_eq!(RecoverPlan::new(None, vec![], None, None), None, "nothing to say");
        let said = crate::crashguard::describe(&[crate::crashguard::Entry {
            kind: crate::crashguard::Kind::Decode,
            path: r"C:\clips\bad.mkv".into(),
            detail: String::new(),
        }]);
        let plan = RecoverPlan::new(said, vec![r"C:\clips\bad.mkv".into()], None, None).unwrap();
        let (title, body) = plan.text();
        assert_eq!(title, "Simple Editor closed unexpectedly");
        assert!(body.contains("while playing bad.mkv"), "{body}");
        assert!(body.contains("bad.mkv is offline for now"), "{body}");
        assert!(!plan.loads(), "no autosave and no project file: nothing to reopen, only Bring the File Back");
        let with_save = RecoverPlan { autosave: Some(PathBuf::from("a.sedit")), ..plan };
        assert!(with_save.loads());
        // a plain newer autosave keeps the old title
        let plain = RecoverPlan::new(None, vec![], Some(PathBuf::from("b.sedit")), None).unwrap();
        assert_eq!(plain.text().0, "Recover unsaved project?");
        for m in [SafeStart::Normal, SafeStart::NoMedia, SafeStart::NoEffects] {
            assert!(!m.label().contains(" - ") && !m.label().contains(':'), "plain labels");
        }
    }

    #[test]
    fn backup_rows_show_the_project_name() {
        assert_eq!(backup_name(Path::new(r"C:\a\autosave\My Film-1234\1800000000.sedit")), "My Film");
        assert_eq!(backup_name(Path::new(r"C:\a\autosave\untitled-99\1.sedit")), "untitled");
    }

    #[test]
    fn ages_read_plainly() {
        assert_eq!(ago(5), "5 s ago");
        assert_eq!(ago(125), "2 min ago");
        assert_eq!(ago(7300), "2 h ago");
        assert_eq!(ago(200_000), "2 days ago");
    }

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
