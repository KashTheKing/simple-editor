//! Import a folder as a library bin (the engine half is `engine/folder_import.rs`).
//!
//! Every way in funnels through `App::take_folders` (`import_files` / `open_or_import` / `open_path`),
//! so a folder dropped on the window, picked from File ▸ Import ▸ Folder as Bin…, sent by the Media
//! Browser's menu or named in `media.import` / `media.import_folder` all end up in `start_folder_import`:
//! the scan runs on a worker as a `MediaJob::Folder`, `finish` (called from `media_sync::tick`) asks
//! first above `CONFIRM_ABOVE` files, and `apply_scan` files everything under ONE undo step.
//!
//! By default (`Options::linked`) none of that runs: the folder becomes a linked bin at once
//! (`link_folder`, see `engine::linked_bins`) and its files turn into assets only when they are used.
//! "Convert to regular bin" is the full import above, aimed at the linked bin it replaces.

use super::*;
use crate::engine::folder_import as fi;

impl App {
    /// Scan `dir` on a worker; `finish` applies the result when it lands. `confirm` = ask first above
    /// `fi::CONFIRM_ABOVE` files (the UI paths - an MCP call names the folder itself). The returned
    /// `Progress` ends once the result has been applied (what `media.import_folder` replies on).
    pub(super) fn start_folder_import(&mut self, dir: &Path, opts: fi::Options, confirm: bool) -> Arc<Progress> {
        if opts.linked {
            return self.link_folder(dir, opts.bin_name.as_deref());
        }
        let slot: Arc<Mutex<Option<fi::Scan>>> = Arc::default();
        let (out, d) = (slot.clone(), dir.to_path_buf());
        let name = dir.file_name().map_or_else(|| dir.display().to_string(), |n| n.to_string_lossy().into_owned());
        let prog = export::spawn_job("folder-import", move |p| {
            p.set(0.0, "Scanning…");
            *out.lock().unwrap_or_else(|e| e.into_inner()) = Some(fi::scan(&d, &opts, MEDIA_EXTS));
            Ok(())
        });
        let done = Progress::new();
        self.media_jobs.push(media_sync::MediaJob::Folder { prog, slot, confirm, done: done.clone(), name });
        self.library.tab = 0;
        done
    }

    /// Link `dir` as a bin: no walk, no probes - one undo step, a toast, and the bin opened in the Library.
    /// The returned `Progress` is already finished (what `media.import_folder` replies on).
    pub(super) fn link_folder(&mut self, dir: &Path, name: Option<&str>) -> Arc<Progress> {
        let done = Progress::new();
        let before = self.project.to_json();
        let had = self.project.linked_bins.len();
        let bin = self.project.link_bin(&dir.to_string_lossy(), name);
        let msg = if self.project.linked_bins.len() == had {
            format!("'{}' is already linked as '{bin}'", dir.display())
        } else {
            self.push_undo_labeled(before, "Link folder");
            self.after_edit();
            format!("Linked '{}' as the bin '{bin}'", dir.display())
        };
        // open it: listing one level is what the user asked to see, and it costs a worker, not this thread
        let key = format!("f:{bin}");
        if !self.library.flipped.contains(&key) {
            self.library.flipped.push(key);
        }
        self.library.tab = 0;
        self.toast_undo(msg.clone(), Action::Undo);
        done.set(1.0, msg);
        done.finish(None);
        done
    }

    /// Library ▸ linked bin ▸ Convert to regular bin: import every file under it as assets (the old full
    /// import, with the usual confirm above `CONFIRM_ABOVE`), filed under the same bins; the link goes in
    /// the same undo step.
    pub(super) fn convert_linked_bin(&mut self, bin: &str) -> Option<Arc<Progress>> {
        let dir = self.project.linked_bins.iter().find(|b| b.bin == bin).map(|b| PathBuf::from(&b.path))?;
        let opts = fi::Options { linked: false, into_bin: Some(bin.to_string()), ..Default::default() };
        Some(self.start_folder_import(&dir, opts, true))
    }

    /// The folders among `paths` each start an import; what is left are the plain files.
    /// ponytail: one `is_dir` stat per path on the UI thread (a drop is a handful; a dead network
    /// share could stall it, like the 2 s offline rescan already can).
    pub(super) fn take_folders(&mut self, paths: &[PathBuf]) -> Vec<PathBuf> {
        let mut files = Vec::with_capacity(paths.len());
        for p in paths {
            if p.is_dir() {
                self.start_folder_import(p, fi::Options::default(), true);
            } else {
                files.push(p.clone());
            }
        }
        files
    }

    /// File ▸ Import ▸ Folder as Bin… (one or several folders in the OS picker).
    pub(super) fn act_import_folder(&mut self) {
        let Some(dirs) = rfd::FileDialog::new().set_title("Import folder as a bin").pick_folders() else { return };
        for d in dirs {
            self.start_folder_import(&d, fi::Options::default(), true);
        }
    }
}

fn fail(app: &mut App, done: &Progress, msg: String) {
    app.toast(msg.clone());
    done.finish(Some(msg));
}

/// `MediaJob::Folder` finished scanning (called from `media_sync::tick`, once per job).
pub(super) fn finish(
    app: &mut App,
    prog: &Progress,
    slot: &Mutex<Option<fi::Scan>>,
    confirm: bool,
    done: Arc<Progress>,
    name: &str,
) {
    let scan = slot.lock().unwrap_or_else(|e| e.into_inner()).take();
    let Some(scan) = scan.filter(|_| prog.error().is_none()) else {
        let why = prog.error().unwrap_or_else(|| "it could not be read".into());
        return fail(app, &done, format!("Can't import '{name}': {why}"));
    };
    let n = scan.file_count();
    if n == 0 {
        let tail = if scan.skipped > 0 { format!(" ({} unsupported file(s) skipped)", scan.skipped) } else { String::new() };
        return fail(app, &done, format!("No media files found in '{name}'{tail}"));
    }
    if confirm && n > fi::CONFIRM_ABOVE {
        confirm::ask_app(
            "Import folder",
            format!("Import {n} files from '{name}' into the library as bins?\nOne Undo step takes it back."),
            move |app| apply_scan(app, scan, &done),
        );
        return;
    }
    apply_scan(app, scan, &done);
}

/// Put the scan into the project: ONE undo step for every bin and file, probes on up to four workers
/// (each ffprobe is ~100 ms, so a big folder would otherwise be minutes of "Loading…"), a toast.
fn apply_scan(app: &mut App, mut scan: fi::Scan, done: &Progress) {
    if !scan.runs.is_empty() && media::ffpipe::ffmpeg_exe().is_none() {
        scan.fold_runs(); // no ffmpeg to bake with: the frames come in as stills
    }
    let before = app.project.to_json();
    if let Some(bin) = &scan.unlink {
        app.project.unlink_bin(bin); // converting: the bin stops being a link in this same undo step
    }
    let applied = fi::apply(&mut app.project, &scan);
    let (top, total) = (scan.top().to_string(), scan.file_count());
    if scan.runs.is_empty() && app.project.to_json() == before {
        let msg = format!("Everything in '{top}' is already in the library");
        app.toast(msg.clone());
        done.set(1.0, msg);
        return done.finish(None);
    }
    app.push_undo_labeled(before, "Import folder");

    let workers = applied.fresh.len().clamp(1, 4);
    let mut lanes: Vec<Vec<(Id, String)>> = vec![Vec::new(); workers];
    for (i, f) in applied.fresh.iter().cloned().enumerate() {
        lanes[i % workers].push(f); // round-robin: every worker starts on the earliest files
    }
    let backend = app.backend();
    for lane in lanes.into_iter().filter(|l| !l.is_empty()) {
        app.probes.push(crate::engine::import::probe_async(lane, backend));
    }
    for (bin, frames) in &scan.runs {
        if let Err(e) = media_sync::start_import_sequence_in(app, &frames[0], None, bin) {
            app.toast(e);
        }
    }
    app.after_edit();

    let imported = total - applied.reused;
    let mut notes = Vec::new();
    if scan.skipped > 0 {
        notes.push(format!("{} skipped", scan.skipped));
    }
    if applied.reused > 0 {
        notes.push(format!("{} already in the library", applied.reused));
    }
    let notes = if notes.is_empty() { String::new() } else { format!(" ({})", notes.join(", ")) };
    let msg = format!("Imported {imported} file{} into '{top}'{notes}", if imported == 1 { "" } else { "s" });
    app.toast_undo(msg.clone(), Action::Undo);
    done.set(1.0, msg);
    done.finish(None);
}
