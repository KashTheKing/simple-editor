use super::*;

impl App {
    pub(super) fn open_path(&mut self, path: &Path) {
        if path.is_dir() {
            self.start_folder_import(path, crate::engine::folder_import::Options::default(), true);
        } else if path.extension().map(|e| e.to_string_lossy().eq_ignore_ascii_case(PROJECT_EXT)).unwrap_or(false) {
            self.open_project(path);
        } else {
            self.open_media(path);
        }
    }

    pub(super) fn open_media(&mut self, path: &Path) {
        let p = path.to_string_lossy().into_owned();
        match media::probe(&p, self.backend()) {
            Ok(asset) => {
                let project = Project::from_media(asset.clone());
                self.set_project(project, None);
                self.offer_match(&asset); // ws:project-color-fps: an HDR file offers an HDR project
                self.settings.touch_recent(&p);
                self.settings.save();
            }
            Err(e) => self.toast(format!("Can't open {}: {e}", path.display())),
        }
    }

    pub(super) fn open_project(&mut self, path: &Path) {
        // forum-crash: a death while loading names this project on the next start
        let _inflight = crate::crashguard::begin(crate::crashguard::Kind::Open, &path.to_string_lossy(), "");
        match Project::load(path) {
            Ok(mut project) => {
                for m in relocate_assets(&mut project, path.parent()) {
                    self.toast(format!("Missing media: {m}"));
                }
                self.check_lock_file(path);
                crate::engine::lut::set_project(Some(path)); // ws:online-library: LUTs copied beside it
                self.set_project(project, Some(path.to_path_buf()));
                self.settings.touch_recent_project(&path.to_string_lossy());
                self.settings.save();
                self.fire_hook("project_open", json!({"path": path.to_string_lossy()}));
            }
            Err(e) => self.repair_and_open(path, &e),
        }
    }

    /// A normal load failed: salvage what the file still holds. The damaged original is copied to
    /// `<name>.sedit.damaged` first (never overwritten), the repaired project opens unsaved (so nothing
    /// on disk changes until the user saves), and a toast lists what was lost.
    fn repair_and_open(&mut self, path: &Path, why: &str) {
        let Ok(r) = Project::load_repaired(path) else {
            return self.toast(format!("Can't open project: {why} (and it could not be repaired)"));
        };
        let mut bak = path.as_os_str().to_owned();
        bak.push(".damaged");
        let bak = PathBuf::from(bak);
        if !bak.exists() {
            let _ = std::fs::copy(path, &bak);
        }
        let mut project = r.project;
        relocate_assets(&mut project, path.parent());
        self.set_project(project, Some(path.to_path_buf()));
        self.dirty = true;
        self.saved = None; // the repair itself still needs saving
        self.toast(format!(
            "Repaired a damaged project ({why}). {} Original kept as {}",
            r.notes.join("; "),
            bak.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
        ));
    }

    /// `<path>.lock` sidecar: warn-only (never blocks open), always overwritten with the current pid -
    /// documented ceiling, not a real cross-instance mutex (see the PR body's risks note).
    fn lock_path(path: &Path) -> PathBuf {
        let mut s = path.as_os_str().to_os_string();
        s.push(".lock");
        PathBuf::from(s)
    }

    pub(super) fn check_lock_file(&mut self, path: &Path) {
        let lock = Self::lock_path(path);
        if self.settings.lock_warn {
            // ws:home-page: only a live other Simple Editor window counts (a lock outlives a closed one)
            if let Some(pid) = crate::ui::home_page::open_elsewhere(path, std::process::id()) {
                self.toast(format!("This project may already be open elsewhere (lock left by pid {pid})"));
            }
        }
        let _ = std::fs::write(&lock, std::process::id().to_string());
    }
    pub(super) fn media_dialog() -> rfd::FileDialog {
        // ws:parity-formats: SVG drawings and Lottie animations import too (converted on the way in)
        let mut exts = MEDIA_EXTS.to_vec();
        exts.extend_from_slice(formats_ctl::CONVERTED_EXTS);
        rfd::FileDialog::new()
            .add_filter("Media", &exts)
            .add_filter("Simple Editor project", &[PROJECT_EXT])
            .add_filter("All files", &["*"])
    }

    /// The one New path (File ▸ New, Ctrl+N, the home cards): ask to save, then replace the current
    /// project with an empty one in `guides::PRESETS[preset]`'s format (default: `Project::new()`'s).
    pub(crate) fn new_project(&mut self, preset: Option<usize>) {
        self.confirm_discard_then(move |app| {
            let mut p = Project::new();
            if let Some(f) = preset.and_then(|i| crate::ui::guides::PRESETS.get(i)) {
                (p.width, p.height, p.fps) = (f.w, f.h, f.fps);
                if crate::ui::guides::is_short_form(f.w, f.h) {
                    p.max_duration = Some(crate::ui::guides::SHORT_FORM_MAX);
                }
                if app.settings.guide.as_deref() != f.guide {
                    app.settings.guide = f.guide.map(str::to_string);
                    app.settings.save();
                }
            }
            app.set_project(p, None);
            app.home_dismissed = true; // an empty project would bring the home cards straight back
        });
    }

    pub(super) fn act_open_file(&mut self) {
        self.confirm_discard_then(|app| {
            if let Some(p) = Self::media_dialog().pick_file() {
                app.open_path(&p);
            }
        });
    }

    pub(super) fn act_open_project(&mut self) {
        self.confirm_discard_then(|app| {
            if let Some(p) = rfd::FileDialog::new().add_filter("Simple Editor project", &[PROJECT_EXT]).pick_file() {
                app.open_project(&p);
            }
        });
    }

    pub(super) fn act_import(&mut self) {
        if let Some(paths) = Self::media_dialog().pick_files() {
            let ids = self.import_files(&paths);
            if let Some(id) = ids.last() {
                self.library.selected = Some(*id);
                self.library.tab = 0;
            }
        }
    }

    pub(super) fn replace_container_dialog(&mut self, clip_id: Id, pair: bool) {
        if let Some(p) = Self::media_dialog().pick_file() {
            let ids = self.import_files(&[p]);
            let Some(&aid) = ids.first() else { return };
            let snap = self.project.to_json();
            let ok = if pair {
                self.project.replace_container_pair(clip_id, aid)
            } else {
                self.project.replace_container_media(clip_id, aid)
            };
            if ok {
                push_undo_json(&mut self.undo, &mut self.redo, snap);
                self.after_edit();
                self.toast("Container media replaced");
            }
        }
    }

    pub(super) fn save_project_as(&mut self) -> bool {
        let mut d = rfd::FileDialog::new()
            .add_filter("Simple Editor project", &[PROJECT_EXT])
            .set_file_name(format!("{}.{PROJECT_EXT}", self.project.name));
        if let Some(dir) = self.project_path.as_ref().and_then(|p| p.parent()) {
            d = d.set_directory(dir);
        } else if let Some(dir) = self.project.source_video.as_ref().and_then(|p| Path::new(p).parent()) {
            d = d.set_directory(dir);
        }
        match d.save_file() {
            Some(p) => {
                self.project_path = Some(p);
                self.save_project()
            }
            None => false,
        }
    }

    /// pub(crate): `confirm::draw`'s Save button (Discard prompt) calls this from outside the `app`
    /// module - see confirm.rs's module doc.
    pub(crate) fn save_project(&mut self) -> bool {
        let Some(path) = self.project_path.clone() else { return self.save_project_as() };
        match self.project.save_with_chats(&path, assistant_ctl::chats_json(self)) {
            Ok(()) => {
                self.dirty = false;
                self.saved = Some(self.project.to_json());
                // ---- ws:online-library ---- library LUTs the project uses go beside it
                crate::engine::lut::set_project(Some(&path));
                if let Ok(v) = serde_json::to_value(&self.project) {
                    if let Err(e) = crate::engine::lut::copy_beside_project(&v, &path) {
                        self.toast(format!("Couldn't copy the project's LUTs: {e}"));
                    }
                }
                self.settings.touch_recent_project(&path.to_string_lossy());
                self.settings.save();
                // ws:home-page: the lock stays while the project is open here (a rename elsewhere checks it)
                let _ = std::fs::write(Self::lock_path(&path), std::process::id().to_string());
                autosave::clear_for(&path);
                self.fire_hook("project_save", json!({"path": path.to_string_lossy()}));
                self.toast_with_folder("Project saved", path);
                true
            }
            Err(e) => {
                self.toast(format!("Save failed: {e}"));
                false
            }
        }
    }

    /// "Open folder" in the subtitles panel: write the current cues as an .srt sidecar into
    /// `<project dir>\<name> subtitles\` and open that folder in Explorer.
    pub(super) fn open_subtitle_folder(&mut self) {
        let dir = match self.project_path.as_ref().and_then(|p| p.parent()) {
            Some(d) => d.join(format!("{} subtitles", self.project.name)),
            None => {
                self.toast("Save the project first - the subtitle folder lives next to it");
                return;
            }
        };
        if let Err(e) = std::fs::create_dir_all(&dir) {
            self.toast(format!("Could not create {}: {e}", dir.display()));
            return;
        }
        if !self.project.subtitles.is_empty() {
            let srt = crate::engine::subtitles::to_srt(&self.project.subtitles);
            let _ = std::fs::write(dir.join(format!("{}.srt", self.project.name)), srt);
        }
        let _ = std::process::Command::new("explorer").arg(&dir).spawn();
    }

    /// Ctrl+S: project file if there is one; otherwise overwrite the opened video; otherwise Save As.
    pub(super) fn act_save(&mut self) {
        if self.project_path.is_some() {
            self.save_project();
        } else if self.project.source_video.is_some() {
            self.act_overwrite();
        } else {
            self.save_project_as();
        }
    }

    /// Ask to save unsaved changes, non-blocking: if the project isn't dirty, `on_yes` runs immediately
    /// (synchronously, same frame); otherwise a Save/Discard/Cancel confirm window is queued and
    /// `on_yes` runs on a LATER frame once it resolves Save (and the save succeeded) or Discard - never
    /// on Cancel. Replaces the old blocking `confirm_discard() -> bool`.
    pub(crate) fn confirm_discard_then(&mut self, on_yes: impl FnOnce(&mut App) + 'static) {
        if !self.dirty || self.still_clean() {
            on_yes(self);
            return;
        }
        // Yes saves a .sedit (the video itself only changes via Save / Overwrite Original Video) - say so
        let msg = if self.project_path.is_some() {
            "Save changes to the project?"
        } else {
            "Save changes as a project file (.sedit)?"
        };
        confirm::ask_discard(msg, on_yes);
    }

    /// Marked dirty, but the project equals how it was opened or last saved (a fresh project nobody
    /// touched, an edit that was undone, a no-op tweak): clears `dirty` and says so. Serialising the
    /// project here is fine: it only runs when a save prompt is about to show.
    pub(crate) fn still_clean(&mut self) -> bool {
        let same = self.saved.as_deref().is_some_and(|s| s == self.project.to_json());
        if same {
            self.dirty = false;
        }
        same
    }

    pub(super) fn ffmpeg_missing(&mut self) -> bool {
        if media::ffpipe::ffmpeg_exe().is_none() {
            self.toast(
                "ffmpeg.exe not found - install FFmpeg (winget install Gyan.FFmpeg) or set its folder in Settings",
            );
            return true;
        }
        false
    }

    pub(super) fn export_opts(&self, out_path: PathBuf) -> ExportOptions {
        ExportOptions {
            out_path,
            encoder: self.settings.encoder.clone(),
            crf: self.settings.crf,
            preset: export::effective_preset(&self.settings.preset, &self.settings.export_speed),
            backend: self.backend(),
            out_size: None,
            scaler: self.settings.export_scaler.clone(),
            frames: self.export_frames(),
            metadata: Vec::new(),
            // ---- ws:export-deliver ----
            range: None,
            loudnorm: self.settings.loudnorm,
            letterbox: false,
            // ---- ws:pro-schema ----
            // ws:export-audio-tracks
            audio: export::AudioLayout::parse(&self.settings.export_audio),
            stem_by: export::StemBy::parse(&self.settings.export_stem_by),
            overlay: None,
            fps: None,
            target_bytes: self.settings.export_target_bytes(),
            alpha: false,
            chapters: self.export_chapters(), // ws:parity-editing
        }
    }
    /// ws:parity-editing: the markers as chapters, when Export ▸ Chapters from markers is on.
    pub(super) fn export_chapters(&self) -> Vec<(f64, String)> {
        if self.settings.export_chapters {
            export::chapters_from_markers(&self.project)
        } else {
            Vec::new()
        }
    }

    pub(super) fn detect_encoders_once(&mut self) {
        // ws:page-switch-freeze: never `ffmpeg -encoders` on this thread (the Export pane calls this every
        // frame); the list fills in when the background probe lands, which also starts ws:export-encoders'
        // hardware probe. Empty until then (and without ffmpeg): the menus offer the built-in names.
        if self.encoders.is_empty() {
            if let Some(v) = export::encoders_nonblocking(wake_ui) {
                self.encoders = v;
            }
        }
    }

    /// Open the (non-blocking) Export window - after a non-blocking preflight when media used on the
    /// timeline is offline (a missing file renders black; say so before the save dialog, not after).
    pub(super) fn act_export(&mut self) {
        if self.ffmpeg_missing() || self.timeline_is_empty() {
            return;
        }
        self.detect_encoders_once();
        // ---- ws:export-deliver ----
        // ws:forum-export: missing files offer Relink…, a clip that would hang the export offers Go to Clip
        let offline = offline_assets(&self.project);
        if export_fix::preflight(self, &offline) {
            export_fix::open_pane(self);
        }
    }

    /// The Export window confirmed (or the queue popped, or Quick Export fired): start the export.
    /// Returns the job, or None when it was refused with a toast (slot busy, ffmpeg missing, empty
    /// timeline, or the path is a project source - re-checked HERE, at pop time, not only at enqueue).
    pub(super) fn start_export_choice(&mut self, choice: export_ui::ExportChoice) -> Option<Arc<Progress>> {
        if self.export.is_some() || self.ffmpeg_missing() || self.timeline_is_empty() {
            return None;
        }
        // writing over a file the player/decoders are reading from is the Overwrite path's job (release + reopen)
        if refuses_source(&self.project, &choice.opts.out_path) {
            self.toast("That file is a source of this project - use Overwrite Original Video (Ctrl+S) instead");
            return None;
        }
        self.player.pause();
        audio_only::bake_now(self); // ws:preview-audio-only: Timeline-mix visualizers export what plays
        let path = choice.opts.out_path.clone();
        let mut project = self.export_project();
        // "Use project background" checkbox: off → the export renders on black exactly as before the
        // background setting existed; on → the authored `preview_bg` (checkerboard bakes as grey tiles,
        // the tooltip says so). Only the exported clone is touched, never the live project.
        if !choice.use_project_bg {
            project.preview_bg = crate::model::BackgroundMode::Black;
        }
        // ws:export-pipeline: the same frame source every other export gets (GPU when it is on)
        let mut opts = ExportOptions { frames: self.export_frames(), ..choice.opts };
        self.deliver(&mut project, &mut opts, None, None);
        formats_ctl::export_started(self, &opts.out_path, opts.range); // ws:parity-formats: Subtitle track
        // a burnt-in guide or caption change needs the re-encode a lossless cut skips
        let prog = if choice.lossless && opts.overlay.is_none() && export::lossless_segments(&project).is_some() {
            export::start_lossless_cut(project, opts.out_path)
        } else {
            export::start_export(project, opts, self.text.clone())
        };
        note_export(&prog, &path); // forum-crash
        self.export = Some((prog.clone(), ExportKind::File { path }));
        // ---- ws:export-deliver ----
        export_fix::remember_tile(self, &choice.last); // ws:forum-export
        self.settings.last_export = Some(choice.last); // what Ctrl+M re-runs next time
        self.settings.save(); // the window remembers resolution/scaler in settings
        Some(prog)
    }

    pub(super) fn act_export_lossless(&mut self) {
        if self.export.is_some() || self.ffmpeg_missing() {
            return;
        }
        // ---- ws:export-deliver ----
        if self.export_ui.range {
            self.toast("Lossless cut can't honour In/Out points - untick Export In/Out Range first");
            return;
        }
        let project = self.export_project();
        if export::lossless_segments(&project).is_none() {
            self.toast("Lossless cut needs a plain cut of one video (no effects, text, overlays or extra media)");
            return;
        }
        let src = project.source_video.clone().or_else(|| project.assets.first().map(|a| a.path.clone()));
        let ext = src
            .as_ref()
            .and_then(|s| Path::new(s).extension().map(|e| e.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "mp4".into());
        let mut d = rfd::FileDialog::new()
            .add_filter(&format!("{} (same container)", ext.to_uppercase()), &[ext.as_str()])
            .set_file_name(format!("{}_cut.{ext}", project.name));
        if let Some(dir) = src.as_ref().and_then(|p| Path::new(p).parent()) {
            d = d.set_directory(dir);
        }
        let Some(out) = d.save_file() else { return };
        self.player.pause();
        let prog = export::start_lossless_cut(project, out.clone());
        self.export = Some((prog, ExportKind::File { path: out }));
    }

    pub(super) fn act_export_xml(&mut self) {
        let Some(out) = rfd::FileDialog::new()
            .add_filter("Final Cut Pro 7 XML (Premiere / Resolve)", &["xml"])
            .set_file_name(format!("{}.xml", self.project.name))
            .save_file()
        else {
            return;
        };
        match std::fs::write(&out, crate::engine::xmeml::export_xmeml(&self.export_project())) {
            Ok(()) => self.toast_with_folder(
                "XML exported - import it in Premiere (File > Import) or Resolve (File > Import > Timeline)",
                out,
            ),
            Err(e) => self.toast(format!("XML export failed: {e}")),
        }
    }

    pub(super) fn act_export_style(&mut self) {
        let Some(out) = rfd::FileDialog::new()
            .add_filter("Markdown", &["md"])
            .set_file_name(format!("{}_style.md", self.project.name))
            .save_file()
        else {
            return;
        };
        // the summary describes the MAIN timeline, even while a nested sequence is open
        match std::fs::write(&out, crate::engine::style::style_summary(&self.export_project())) {
            Ok(()) => self.toast_with_folder("Style summary exported", out),
            Err(e) => self.toast(format!("Style summary failed: {e}")),
        }
    }

    /// Re-encode the timeline over the opened video file (temp file in the same folder, then replace).
    /// Non-blocking now: up to two confirm windows chain via continuations instead of two synchronous
    /// blocking-dialog `.show()` calls.
    pub(super) fn act_overwrite(&mut self) {
        let Some(src) = self.project.source_video.clone() else {
            self.toast("No source video to overwrite - use Export Video As");
            return;
        };
        if self.export.is_some() {
            self.toast("An export is already running");
            return;
        }
        if self.timeline_is_empty() {
            self.toast("Timeline is empty - nothing to save");
            return;
        }
        if self.ffmpeg_missing() {
            return;
        }
        if self.settings.confirm_overwrite {
            confirm::ask_app(
                "Overwrite original video?",
                format!("{src}\n\nThe file will be replaced with the edited video. This cannot be undone."),
                move |app| app.act_overwrite_offer_save_first(src),
            );
        } else {
            self.act_overwrite_offer_save_first(src);
        }
    }

    /// Second stage of `act_overwrite`: the new file reloads as a fresh project afterward, so state
    /// that isn't burned into the video (subtitles, planner, notes, sequences, extra media) would be
    /// silently dropped - offer to save a .sedit first. Same Save/Discard/Cancel shape as
    /// `confirm_discard_then`, so it reuses `confirm::ask_discard`.
    pub(super) fn act_overwrite_offer_save_first(&mut self, src: String) {
        let p = &self.project;
        let loses = !p.plan.is_empty()
            || !p.notes.is_empty()
            || !p.subtitles.is_empty()
            || !p.sequences.is_empty()
            || p.assets.len() > 1;
        if loses && (self.dirty || self.project_path.is_none()) {
            confirm::ask_discard(
                "Overwriting reloads the new file as a fresh project - subtitles, planner, notes, \
                 sequences and imported media are not kept. Save a project file (.sedit) first?",
                move |app| app.act_overwrite_run(&src),
            );
        } else {
            self.act_overwrite_run(&src);
        }
    }

    /// Final stage: actually re-encode over the source. `src` is the same source path validated by
    /// `act_overwrite`.
    fn act_overwrite_run(&mut self, src: &str) {
        let original = PathBuf::from(src);
        let ext = original.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_else(|| "mp4".into());
        let temp = original.with_file_name(format!(
            ".{}.simple-editor-tmp.{ext}",
            original.file_stem().unwrap_or_default().to_string_lossy()
        ));
        self.player.pause();
        let mut project = self.export_project();
        // no background checkbox on this path - always render on black, like every export did before
        // `preview_bg` existed (a checkerboard preview aid must never bake into the overwritten original)
        project.preview_bg = crate::model::BackgroundMode::Black;
        // opt-in: a plain cut can be saved instantly with `-c copy` (keyframe-accurate) instead of re-encoding
        let lossless = self.settings.lossless_save && export::lossless_segments(&project).is_some();
        let prog = if lossless {
            export::start_lossless_cut(project, temp.clone())
        } else {
            let mut opts = self.export_opts(temp.clone());
            // ws:export-audio-tracks: stems would be named after the hidden temp file - an overwrite
            // keeps "separate tracks in file" but writes no stem folder
            if opts.audio == export::AudioLayout::Stems {
                opts.audio = export::AudioLayout::Mix;
            }
            export::start_export(project, opts, self.text.clone())
        };
        note_export(&prog, &original); // forum-crash
        self.export = Some((prog, ExportKind::Overwrite { original, temp }));
    }

    pub(super) fn finish_export(&mut self) {
        let Some((prog, kind)) = self.export.take() else { return };
        // ---- ws:export-deliver ----
        // `-- @on export_done` scripts hear every outcome (path + ok), success or not
        let hook_path = match &kind {
            ExportKind::File { path } => path.clone(),
            ExportKind::Overwrite { original, .. } => original.clone(),
        };
        // ws:quick-export: a Quick Export's result opens in the media player once it is in place
        let play = self.quick_open.take_if(|p| *p == hook_path);
        if let Some(e) = prog.error() {
            if let ExportKind::Overwrite { temp, .. } = &kind {
                let _ = std::fs::remove_file(temp);
            }
            if prog.is_cancelled() {
                // ws:forum-export: Retry in Software's own cancel keeps the queue and starts the retry
                if !export_fix::retrying(self) {
                    self.toast("Export cancelled");
                    self.export_queue.clear(); // a cancel means "stop", not "start the next one"
                }
            } else {
                export_fix::failed(self, &prog, &kind, &e); // ws:forum-export: one sentence + its fix
            }
            self.fire_hook("export_done", json!({"path": hook_path.to_string_lossy(), "ok": false}));
            formats_ctl::export_finished(self, &hook_path, false); // ws:parity-formats
            return;
        }
        self.fire_hook("export_done", json!({"path": hook_path.to_string_lossy(), "ok": true}));
        match kind {
            ExportKind::File { path } => {
                formats_ctl::export_finished(self, &path, true); // ws:parity-formats: Subtitle track
                // ws:player-metadata: a preset's Explorer tags, through Windows' own handler (off this thread)
                let kw = crate::ui::export_ui::export_keywords(self.export_ui.metadata_on, &self.export_ui.metadata);
                if !kw.is_empty() {
                    let p = path.clone();
                    std::thread::spawn(move || {
                        if crate::engine::tags::keywords_writable(&p) {
                            let _ = crate::engine::tags::write_keywords(&p, &kw);
                        }
                    });
                }
                let took = crate::ui::duration_text(prog.elapsed().as_secs_f64());
                let queued = self.export_queue.len();
                let msg = if queued > 0 {
                    format!("Export finished in {took} - {queued} more queued")
                } else {
                    format!("Export finished in {took}")
                };
                if let Some(p) = &play {
                    quick_export::open_in_player(p);
                }
                let t = feedback::Toast::with_folder(msg, path.clone()).kind(feedback::ToastKind::Success);
                let t = export_fix::succeeded(self, &prog, &path, t); // ws:forum-export: Play + speed
                self.push_toast(t);
            }
            ExportKind::Overwrite { original, temp } => {
                // ws:quick-export: an empty project first - `release_files` also empties the frame cache, and
                // a paused render thread re-rendered the current frame at once, reopening the source
                self.player.set_project(&Project::new());
                self.player.release_files();
                if let Some(s) = self.source.as_mut() {
                    s.player.release_files();
                }
                // the thumbnail worker holds a decoder (ffmpeg child) on the source - drop it while we retry
                self.thumbs.clear();
                // ws:quick-export: original kept as <name>.bak until the atomic rename succeeds
                match quick_export::swap_with_backup(&temp, &original) {
                    Ok(()) => {
                        self.toast_with_folder("Saved over the original video", original.clone());
                        // in-memory peaks are keyed by path only and the file behind it just changed
                        self.waveforms.clear();
                        self.open_media(&original);
                        if play.is_some() {
                            quick_export::open_in_player(&original);
                        }
                    }
                    Err(e) => {
                        // the original is untouched: keep the edit as a visible copy, never hidden leftovers
                        let name = original.file_name().unwrap_or_default().to_string_lossy().into_owned();
                        match quick_export::keep_as_copy(&temp, &original) {
                            Ok(copy) => self.toast_with_folder(
                                format!("{name} is open in another program, so it wasn't replaced. Your edit is saved as {}", copy.file_name().unwrap_or_default().to_string_lossy()),
                                copy,
                            ),
                            Err(_) => self.toast(format!("Couldn't replace {name} ({e}); your edit is at {}", temp.display())),
                        }
                        self.player.set_project(&self.project);
                    }
                }
            }
        }
    }

    pub(super) fn url_window(&mut self, ctx: &egui::Context) {
        let Some((mut url, mut audio_only)) = self.url_dialog.clone() else { return };
        let mut open = true;
        let mut start = false;
        let dir = self.download_dir();
        egui::Window::new("Import URL").open(&mut open).resizable(false).default_width(420.0).show(ctx, |ui| {
            ui.label("Paste a link (YouTube, Vimeo, X, TikTok, direct file, …)");
            let r = ui.add(egui::TextEdit::singleline(&mut url).desired_width(400.0).hint_text("https://…"));
            // Enter in the field downloads, like every other URL box
            start |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if crate::media::ffpipe::ffmpeg_exe().is_some() {
                ui.checkbox(&mut audio_only, "Audio only (music / SFX)");
                // ---- feat/reference-clips ----
                start |= super::reference_ctl::part_row(ui, &mut self.references, &url);
            }
            ui.horizontal(|ui| {
                ui.label("Save to");
                ui.weak(dir.to_string_lossy());
                if ui.small_button("Browse…").clicked() {
                    if let Some(p) = rfd::FileDialog::new().set_directory(&dir).pick_folder() {
                        self.settings.download_dir = p.to_string_lossy().into_owned();
                        self.settings.save();
                    }
                }
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                start |= ui.add_enabled(!url.trim().is_empty(), egui::Button::new("Download")).clicked();
                ui.weak(format!("{} running", self.downloads.len()));
            });
        });
        match (open, start) {
            (_, true) => {
                // ---- feat/reference-clips ----
                match super::reference_ctl::part_range(&self.references) {
                    Err(e) => {
                        self.toast(e);
                        self.url_dialog = Some((url, audio_only));
                    }
                    Ok(range) => {
                        self.url_dialog = None;
                        self.start_download(url.trim(), audio_only, range);
                    }
                }
            }
            (true, false) => self.url_dialog = Some((url, audio_only)),
            (false, false) => self.url_dialog = None,
        }
    }

    /// Configured download folder, or the user's Videos folder.
    pub(super) fn download_dir(&self) -> PathBuf {
        let d = self.settings.download_dir.trim();
        if d.is_empty() {
            crate::media::ytdlp::default_dir()
        } else {
            PathBuf::from(d)
        }
    }

    /// Look for a working yt-dlp on a background thread (it spawns `yt-dlp --version`, and a candidate
    /// that hangs must not hang the editor); the Library button appears once it reports success.
    pub(super) fn detect_ytdlp(&self, ctx: &egui::Context) {
        let flag = self.ytdlp_available.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let found = media::ytdlp::exe().is_some();
            flag.store(found, std::sync::atomic::Ordering::Relaxed);
            ctx.request_repaint(); // the Library button appears without waiting for the next input
        });
    }

    /// `range` = "Just a part" (source seconds). The download's progress, or None when yt-dlp is missing.
    pub(super) fn start_download(
        &mut self,
        url: &str,
        audio_only: bool,
        range: Option<(f64, f64)>,
    ) -> Option<Arc<crate::engine::export::Progress>> {
        if !self.ytdlp_available.load(std::sync::atomic::Ordering::Relaxed) {
            self.toast("yt-dlp not found - set its folder in Settings");
            return None;
        }
        let opts = crate::media::ytdlp::DownloadOptions { url: url.to_string(), dir: self.download_dir(), audio_only, range };
        self.toast(match range {
            Some((a, b)) => format!(
                "Downloading {}-{}…",
                crate::engine::references::fmt_time(a),
                crate::engine::references::fmt_time(b)
            ),
            None => "Downloading…".to_string(),
        });
        let d = crate::media::ytdlp::start_download(opts);
        let prog = d.progress.clone();
        self.downloads.push(d);
        Some(prog)
    }

    // ---- ws:parity-editing ----
    /// Clip ▸ Save Clip as File…: the part of the source the selected clip uses, re-encoded on its own
    /// (frame-accurate, no effects), then imported into the Library. Shotcut's "export selected clip".
    pub(super) fn save_clip_as_file(&mut self) {
        if self.ffmpeg_missing() {
            return;
        }
        let pick = self.selection.iter().filter_map(|&id| self.project.clip(id)).find(|c| c.uses_asset());
        let Some(c) = pick else {
            self.toast("Select a video or audio clip to save");
            return;
        };
        let Some(a) = self.project.asset(c.asset) else { return };
        let Some((src, range, streams, ext)) = clip_file_plan(c, a) else {
            self.toast("This clip has no media of its own to save");
            return;
        };
        let stem = src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "clip".into());
        let mut d = rfd::FileDialog::new().set_file_name(format!("{stem} (clip).{ext}")).add_filter(ext.to_uppercase(), &[ext]);
        if let Some(dir) = src.parent() {
            d = d.set_directory(dir);
        }
        let Some(mut out) = d.save_file() else { return };
        if out.extension().is_none() {
            out.set_extension(ext);
        }
        if out == src {
            self.toast("Pick a different name: the source file is never overwritten");
            return;
        }
        let opts = crate::engine::convert::ConvertOptions {
            audio_streams: streams,
            src,
            out: out.clone(),
            encoder: self.settings.encoder.clone(),
            crf: self.settings.crf,
            preset: self.settings.preset.clone(),
            out_size: None,
            scaler: self.settings.export_scaler.clone(),
            gif_fps: 15,
            target_bytes: None,
            vf_extra: None,
            af_extra: None,
            range: Some(range),
        };
        self.toast(format!("Saving the clip to {}…", out.file_name().unwrap_or_default().to_string_lossy()));
        self.convert_jobs.push((crate::engine::convert::start_convert(opts), out));
    }
    /// "Convert To…" on a library asset: transcode next to the source, then import the result.
    pub(super) fn start_asset_convert(&mut self, asset: Id, ext: &str) {
        if self.ffmpeg_missing() {
            return;
        }
        let Some(a) = self.project.asset(asset) else { return };
        let src = PathBuf::from(&a.path);
        let out = converted_path(&src, ext);
        let opts = crate::engine::convert::ConvertOptions {
            audio_streams: None,
            src,
            out: out.clone(),
            encoder: self.settings.encoder.clone(),
            crf: self.settings.crf,
            preset: self.settings.preset.clone(),
            out_size: None,
            scaler: self.settings.export_scaler.clone(),
            gif_fps: 15,
            target_bytes: None,
            vf_extra: None,
            af_extra: None,
            range: None,
        };
        self.toast(format!("Converting to {ext}…"));
        self.convert_jobs.push((crate::engine::convert::start_convert(opts), out));
    }
    pub(super) fn act_import_timeline(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Timelines (Shotcut, XML, EDL, prproj)", crate::engine::import::IMPORT_EXTS)
            .add_filter("All files", &["*"])
            .pick_file()
        else {
            return;
        };
        match guarded(|| crate::engine::import::import_file(&path)) {
            Some(Ok(report)) => {
                self.toast(format!("Imported {} clips on {} tracks", report.clips, report.tracks));
                self.import_ui.report = Some(report);
                self.import_ui.open = true;
            }
            Some(Err(e)) => self.toast(format!("Import failed: {e}")),
            None => self.toast("Timeline import is not available in this build"),
        }
    }
    pub(super) fn compress_window(&mut self, ctx: &egui::Context) {
        let Some(mut c) = self.compress.take() else { return };
        let (mut open, mut start) = (true, false);
        egui::Window::new("Compress").open(&mut open).resizable(false).default_width(320.0).show(ctx, |ui| {
            ui.label(c.src.file_name().unwrap_or_default().to_string_lossy());
            if let Some(b) = c.source_bytes {
                ui.weak(format!("{:.1} MB on disk", b as f64 / 1e6));
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.selectable_value(&mut c.by_size, false, "Amount");
                ui.selectable_value(&mut c.by_size, true, "Target size");
            });
            if c.by_size {
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut c.target_mb).speed(0.5).range(0.1..=20_000.0).suffix(" MB"));
                    if let Some(d) = c.duration.filter(|d| *d > 0.0) {
                        match crate::engine::convert::target_bitrate((c.target_mb * 1e6) as u64, d) {
                            Some(bps) => ui.weak(format!("\u{2248} {} kbps video", bps / 1000)),
                            None => ui.colored_label(ui.visuals().error_fg_color, "too small for this length"),
                        };
                    }
                });
            } else {
                ui.horizontal(|ui| {
                    ui.add(egui::Slider::new(&mut c.crf, 18..=40).text("CRF"));
                });
                ui.weak("Higher = smaller file, more artefacts. 23 is the usual default.");
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.selectable_value(&mut c.overwrite, false, "Save a copy");
                ui.selectable_value(&mut c.overwrite, true, "Overwrite original");
            });
            if c.overwrite {
                ui.colored_label(ui.visuals().warn_fg_color, "The original file is replaced when this finishes.");
            } else {
                ui.weak("Written next to the source as <name>_compressed.<ext> and added to the library.");
            }
            ui.add_space(4.0);
            start = ui.button("Compress").clicked();
        });
        if start {
            self.start_compress(&c);
            return; // window closes; the job window takes over
        }
        if open {
            self.compress = Some(c);
        }
    }

    pub(super) fn start_compress(&mut self, c: &Compress) {
        if self.ffmpeg_missing() {
            return;
        }
        let ext = c.src.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_else(|| "mp4".into());
        let out = if c.overwrite {
            c.src.clone()
        } else {
            let stem = c.src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "output".into());
            let mut p = c.src.with_file_name(format!("{stem}_compressed.{ext}"));
            let mut n = 2;
            while p.exists() {
                p = c.src.with_file_name(format!("{stem}_compressed_{n}.{ext}"));
                n += 1;
            }
            p
        };
        let opts = crate::engine::convert::ConvertOptions {
            audio_streams: None,
            src: c.src.clone(),
            out: out.clone(),
            encoder: self.settings.encoder.clone(),
            crf: c.crf,
            preset: self.settings.preset.clone(),
            out_size: None,
            scaler: self.settings.export_scaler.clone(),
            gif_fps: 15,
            target_bytes: c.by_size.then(|| (c.target_mb * 1e6) as u64),
            vf_extra: None,
            af_extra: None,
            range: None,
        };
        self.toast("Compressing\u{2026}");
        self.convert_jobs.push((crate::engine::convert::start_convert(opts), out));
    }
}

// ---- ws:export-deliver ----
/// Is `out` one of the project's own source files? (Canonicalised, so `..`/case/drive-letter forms
/// still match.) Writing over a file the player and decoders are reading is the Overwrite path's job
/// - the queue re-runs this at pop time via `start_export_choice`, not only when a job is added.
pub(super) fn refuses_source(project: &Project, out: &Path) -> bool {
    let Ok(out_c) = std::fs::canonicalize(out) else { return false };
    project.assets.iter().any(|a| std::fs::canonicalize(&a.path).is_ok_and(|p| p == out_c))
}

/// Paths of assets used on the timeline whose file is missing - the export preflight list.
pub(super) fn offline_assets(project: &Project) -> Vec<String> {
    let used = project.used_assets();
    project
        .assets
        .iter()
        .filter(|a| used.contains(&a.id) && !Path::new(&a.path).exists())
        .map(|a| a.path.clone())
        .collect()
}

// ---- ws:forgiveness ----
/// forum-crash: an export is in flight (`crashguard::Kind::Export`) until its job finishes, so a crash
/// during it names the output on the next start.
fn note_export(prog: &Arc<Progress>, out: &Path) {
    let p = prog.clone();
    crate::crashguard::linger_while(crate::crashguard::Kind::Export, &out.to_string_lossy(), "", move || !p.is_done());
}

#[cfg(test)]
mod tests {
    /// Structural (source-scan) tests, not App-level ones - this crate has no headless App-construction
    /// path anywhere (see tools_registry_tests.rs's doc comment for why), so a check that would
    /// otherwise call `open_project`/`save_project`/`confirm_discard_then` on a live `App` instead
    /// verifies the same fact by scanning each function's own body, the same technique
    /// `run_script_pushes_one_undo_per_script` (tools_registry_tests.rs) already uses in this crate.
    fn fn_body<'a>(src: &'a str, signature: &str) -> &'a str {
        let start = src.find(signature).unwrap_or_else(|| panic!("{signature} must exist"));
        let after = &src[start..];
        // bound to the next sibling fn at the same indentation (4 spaces) - good enough for this file's
        // flat `impl App { fn ... }` shape.
        let next_at = after[signature.len()..].find("\n    fn ").or_else(|| after[signature.len()..].find("\n    pub"));
        match next_at {
            Some(i) => &after[..signature.len() + i],
            None => after,
        }
    }

    #[test]
    fn open_and_save_fire_project_hooks() {
        let src = include_str!("files.rs");
        let open_body = fn_body(src, "pub(super) fn open_project(");
        assert_eq!(
            open_body.matches("fire_hook(\"project_open\"").count(),
            1,
            "open_project must fire project_open exactly once on success"
        );
        let save_body = fn_body(src, "pub(crate) fn save_project(");
        assert_eq!(
            save_body.matches("fire_hook(\"project_save\"").count(),
            1,
            "save_project must fire project_save exactly once on success"
        );
    }

    /// Every New entry point goes through `new_project` (save prompt + a real replace), and replacing
    /// the project also closes the old one's Source monitor.
    #[test]
    fn every_new_path_replaces_the_project() {
        let files = include_str!("files.rs");
        let body = fn_body(files, "pub(crate) fn new_project(");
        assert!(body.contains("confirm_discard_then(") && body.contains("set_project("));
        assert!(include_str!("actions.rs").contains("NewProject => self.new_project(None)"));
        let home = include_str!("layout_ctl.rs");
        assert!(home.contains("HomeAction::New(i) => app.new_project(i)"), "home cards must make a new project");
        let set = fn_body(include_str!("mod.rs"), "fn set_project(");
        assert!(set.contains("self.close_source()"), "the old project's Source clip must not survive");
    }

    /// An untouched project never asks to save: the save prompt and the window close compare against the
    /// snapshot taken when the project was opened, created or saved, and that snapshot is stable.
    #[test]
    fn an_unchanged_project_never_asks_to_save() {
        let a = crate::model::Project::new();
        assert_eq!(a.to_json(), crate::model::Project::new().to_json(), "a fresh project serialises the same every time");
        let body = fn_body(include_str!("files.rs"), "pub(crate) fn confirm_discard_then(");
        assert!(body.contains("self.still_clean()"));
        let set = fn_body(include_str!("mod.rs"), "fn set_project(");
        assert!(set.contains("self.saved = Some(self.project.to_json())"));
        let save = fn_body(include_str!("files.rs"), "pub(crate) fn save_project(");
        assert!(save.contains("self.saved = Some(self.project.to_json())"));
        assert!(include_str!("mod.rs").contains("self.dirty && !self.still_clean()"), "closing the window too");
    }

    #[test]
    fn confirm_discard_then_runs_continuation_without_blocking() {
        let src = include_str!("files.rs");
        let body = fn_body(src, "pub(crate) fn confirm_discard_then(");
        assert!(
            body.contains("if !self.dirty") && body.contains("on_yes(self)"),
            "a clean project must run the continuation immediately, not queue a confirm window"
        );
        assert!(
            body.contains("confirm::ask_discard("),
            "a dirty project must queue a non-blocking Save/Discard/Cancel window, never a blocking dialog"
        );
        // the crate-wide grep for the removed blocking-dialog API (see the PR body's verification
        // checklist) covers the "never blocks" property globally; not duplicated here as a literal
        // string, since that string would itself trip that same grep.
    }

    // ---- ws:export-deliver ----
    /// The moved `export_opts()` literal (app.rs:1461 before split-god-files) sets the three new
    /// fields explicitly - `range: None`, `letterbox: false`, and loudnorm from Settings - so a plain
    /// export's ffmpeg line is what it was (the byte comparison itself lives in
    /// `engine::export::tests::loudnorm_appends_af_filter_only_with_audio`: loudnorm off ⇒ no `-af`,
    /// letterbox off ⇒ the old plain `scale=`). Same source-scan technique as the two tests above.
    #[test]
    fn moved_option_literals_compile_with_new_fields() {
        let src = include_str!("files.rs");
        let body = fn_body(src, "pub(super) fn export_opts(");
        for field in ["range: None", "loudnorm: self.settings.loudnorm", "letterbox: false"] {
            assert!(body.contains(field), "export_opts must set `{field}` explicitly");
        }
        // finish_export fires export_done exactly twice (the failure return and the success path) and
        // never anywhere else in this file
        assert_eq!(src.matches("fire_hook(\"export_done\"").count(), 2);
        let finish = fn_body(src, "pub(super) fn finish_export(");
        assert_eq!(finish.matches("fire_hook(\"export_done\"").count(), 2);
    }
}

// ---- ws:parity-editing ----
/// What Save Clip as File writes for `c`: the source, the source range it uses, the audio streams to keep
/// and the output extension. An audio clip writes its own stream to a WAV (or the source's audio format);
/// a video clip keeps the source's container when ffmpeg can write it, else MP4. None for a still or a
/// clip without a source range.
pub(crate) fn clip_file_plan(
    c: &crate::model::Clip,
    a: &crate::model::Asset,
) -> Option<(PathBuf, (f64, f64), Option<Vec<usize>>, &'static str)> {
    use crate::model::ClipKind;
    if a.path.is_empty() || matches!(c.kind, ClipKind::Image) || c.freeze.is_some() {
        return None;
    }
    let src = PathBuf::from(&a.path);
    let len = c.duration * c.speed.max(1e-6);
    let mut b = c.src_in + len;
    if a.duration > 0.0 {
        b = b.min(a.duration);
    }
    let r = (c.src_in.max(0.0), b);
    if r.1 - r.0 <= 0.0 {
        return None;
    }
    let src_ext = crate::engine::export::ext_of(&src);
    let audio = c.kind == ClipKind::Audio;
    let ext: &'static str = if audio {
        crate::engine::export::AUDIO_EXTS.iter().copied().find(|e| *e == src_ext).unwrap_or("wav")
    } else {
        ["mp4", "mov", "mkv", "webm", "m4v"].into_iter().find(|e| *e == src_ext).unwrap_or("mp4")
    };
    let streams = audio.then(|| vec![c.audio_stream]);
    Some((src, r, streams, ext))
}

#[cfg(test)]
mod parity_editing_tests {
    use super::clip_file_plan;
    use crate::model::{Asset, Clip, ClipKind};

    fn asset(path: &str, dur: f64) -> Asset {
        serde_json::from_value(serde_json::json!({
            "id": 1, "path": path, "kind": "Video", "duration": dur, "width": 1920, "height": 1080, "fps": 30.0
        }))
        .unwrap()
    }

    #[test]
    fn save_clip_as_file_takes_the_used_source_range() {
        let a = asset(r"C:\footage\take3.mov", 20.0);
        let mut c = Clip::new(5, ClipKind::Video, "take3", 10.0, 4.0);
        c.asset = 1;
        c.src_in = 3.0;
        let (src, range, streams, ext) = clip_file_plan(&c, &a).unwrap();
        assert_eq!(src, std::path::PathBuf::from(r"C:\footage\take3.mov"));
        assert_eq!(range, (3.0, 7.0), "timeline position does not matter, the source window does");
        assert_eq!((streams, ext), (None, "mov"), "a video clip keeps every audio track and the container");
        // a 2x clip uses twice its length of source; the end is clamped to the file
        c.speed = 2.0;
        c.src_in = 15.0;
        assert_eq!(clip_file_plan(&c, &a).unwrap().1, (15.0, 20.0));
        // an audio clip writes its own stream to a WAV
        let mut au = Clip::new(6, ClipKind::Audio, "take3", 0.0, 2.0);
        au.asset = 1;
        au.audio_stream = 1;
        let (_, _, streams, ext) = clip_file_plan(&au, &a).unwrap();
        assert_eq!((streams, ext), (Some(vec![1]), "wav"));
        // stills and frozen frames have no range to save
        let mut f = c.clone();
        f.freeze = Some(1.0);
        assert!(clip_file_plan(&f, &a).is_none());
        // an odd container falls back to MP4
        assert_eq!(clip_file_plan(&Clip { speed: 1.0, src_in: 0.0, ..c }, &asset("C:/a.mts", 9.0)).unwrap().3, "mp4");
    }
}
