use super::*;

impl App {
    pub(super) fn open_path(&mut self, path: &Path) {
        if path.extension().map(|e| e.to_string_lossy().eq_ignore_ascii_case(PROJECT_EXT)).unwrap_or(false) {
            self.open_project(path);
        } else {
            self.open_media(path);
        }
    }

    pub(super) fn open_media(&mut self, path: &Path) {
        let p = path.to_string_lossy().into_owned();
        match media::probe(&p, self.backend()) {
            Ok(asset) => {
                let project = Project::from_media(asset);
                self.set_project(project, None);
                self.settings.touch_recent(&p);
                self.settings.save();
            }
            Err(e) => self.toast(format!("Can't open {}: {e}", path.display())),
        }
    }

    pub(super) fn open_project(&mut self, path: &Path) {
        match Project::load(path) {
            Ok(mut project) => {
                for m in relocate_assets(&mut project, path.parent()) {
                    self.toast(format!("Missing media: {m}"));
                }
                self.check_lock_file(path);
                self.set_project(project, Some(path.to_path_buf()));
                self.settings.touch_recent_project(&path.to_string_lossy());
                self.settings.save();
                self.fire_hook("project_open", json!({"path": path.to_string_lossy()}));
            }
            Err(e) => self.toast(format!("Can't open project: {e}")),
        }
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
            if let Ok(existing_pid) = std::fs::read_to_string(&lock) {
                if !existing_pid.trim().is_empty() {
                    self.toast(format!(
                        "This project may already be open elsewhere (lock left by pid {})",
                        existing_pid.trim()
                    ));
                }
            }
        }
        let _ = std::fs::write(&lock, std::process::id().to_string());
    }
    pub(super) fn media_dialog() -> rfd::FileDialog {
        rfd::FileDialog::new()
            .add_filter("Media", MEDIA_EXTS)
            .add_filter("Simple Editor project", &[PROJECT_EXT])
            .add_filter("All files", &["*"])
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
        match self.project.save(&path) {
            Ok(()) => {
                self.dirty = false;
                self.settings.touch_recent_project(&path.to_string_lossy());
                self.settings.save();
                let _ = std::fs::remove_file(Self::lock_path(&path));
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
        if !self.dirty {
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
            preset: self.settings.preset.clone(),
            backend: self.backend(),
            out_size: None,
            scaler: self.settings.export_scaler.clone(),
            frames: self.export_frames(),
            metadata: Vec::new(),
            // ---- ws:export-deliver ----
            range: None,
            loudnorm: self.settings.loudnorm,
            letterbox: false,
        }
    }

    pub(super) fn detect_encoders_once(&mut self) {
        if self.encoders.is_empty() && media::ffpipe::ffmpeg_exe().is_some() {
            self.encoders = export::detect_encoders();
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
        let offline = offline_assets(&self.project);
        if offline.is_empty() {
            self.export_ui.open = true;
        } else {
            let list = offline.iter().take(5).map(|p| format!("  {p}")).collect::<Vec<_>>().join("\n");
            let more =
                if offline.len() > 5 { format!("\n  … and {} more", offline.len() - 5) } else { String::new() };
            confirm::ask_app(
                "Missing media",
                format!(
                    "{} file(s) used on the timeline can't be found - they export as black/silence:\n{list}{more}\n\nExport anyway?",
                    offline.len()
                ),
                |app| app.export_ui.open = true,
            );
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
        let path = choice.opts.out_path.clone();
        let mut project = self.export_project();
        // "Use project background" checkbox: off → the export renders on black exactly as before the
        // background setting existed; on → the authored `preview_bg` (checkerboard bakes as grey tiles,
        // the tooltip says so). Only the exported clone is touched, never the live project.
        if !choice.use_project_bg {
            project.preview_bg = crate::model::BackgroundMode::Black;
        }
        let prog = if choice.lossless {
            export::start_lossless_cut(project, choice.opts.out_path.clone())
        } else {
            export::start_export(project, choice.opts, self.text.clone())
        };
        self.export = Some((prog.clone(), ExportKind::File { path }));
        // ---- ws:export-deliver ----
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
    fn act_overwrite_offer_save_first(&mut self, src: String) {
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
            export::start_export(project, self.export_opts(temp.clone()), self.text.clone())
        };
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
        if let Some(e) = prog.error() {
            if let ExportKind::Overwrite { temp, .. } = &kind {
                let _ = std::fs::remove_file(temp);
            }
            if prog.is_cancelled() {
                self.toast("Export cancelled");
                self.export_queue.clear(); // a cancel means "stop", not "start the next one"
            } else {
                self.push_toast(feedback::Toast::new(format!("Export failed: {e}")).kind(feedback::ToastKind::Error));
            }
            self.fire_hook("export_done", json!({"path": hook_path.to_string_lossy(), "ok": false}));
            return;
        }
        self.fire_hook("export_done", json!({"path": hook_path.to_string_lossy(), "ok": true}));
        match kind {
            ExportKind::File { path } => {
                let took = crate::ui::duration_text(prog.elapsed().as_secs_f64());
                let queued = self.export_queue.len();
                let msg = if queued > 0 {
                    format!("Export finished in {took} - {queued} more queued")
                } else {
                    format!("Export finished in {took}")
                };
                self.push_toast(feedback::Toast::with_folder(msg, path).kind(feedback::ToastKind::Success));
            }
            ExportKind::Overwrite { original, temp } => {
                self.player.release_files();
                // the thumbnail worker holds a decoder (ffmpeg child) on the source - drop it while we retry
                self.thumbs.clear();
                // ponytail: killed ffmpeg children release their file handle a few ms after wait() returns - retry briefly
                let mut r = std::fs::rename(&temp, &original);
                let deadline = Instant::now() + Duration::from_millis(500);
                while r.is_err() && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(10));
                    r = std::fs::rename(&temp, &original);
                }
                match r {
                    Ok(()) => {
                        self.toast_with_folder("Saved over the original video", original.clone());
                        // in-memory peaks are keyed by path only and the file behind it just changed
                        self.waveforms.clear();
                        self.open_media(&original);
                    }
                    Err(e) => {
                        self.toast(format!(
                            "Couldn't replace the original ({e}); edited file left at {}",
                            temp.display()
                        ));
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
                self.url_dialog = None;
                self.start_download(url.trim(), audio_only);
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

    pub(super) fn start_download(&mut self, url: &str, audio_only: bool) {
        if !self.ytdlp_available.load(std::sync::atomic::Ordering::Relaxed) {
            self.toast("yt-dlp not found - set its folder in Settings");
            return;
        }
        let opts = crate::media::ytdlp::DownloadOptions { url: url.to_string(), dir: self.download_dir(), audio_only };
        self.toast("Downloading…");
        self.downloads.push(crate::media::ytdlp::start_download(opts));
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
        };
        self.toast(format!("Converting to {ext}…"));
        self.convert_jobs.push((crate::engine::convert::start_convert(opts), out));
    }
    pub(super) fn act_import_timeline(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Timelines (XML, EDL, prproj)", crate::engine::import::IMPORT_EXTS)
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
fn offline_assets(project: &Project) -> Vec<String> {
    let used = project.used_assets();
    project
        .assets
        .iter()
        .filter(|a| used.contains(&a.id) && !Path::new(&a.path).exists())
        .map(|a| a.path.clone())
        .collect()
}

// ---- ws:forgiveness ----
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
