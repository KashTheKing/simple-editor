//! The application: owns the project, undo stack, settings, player, and wires the panels together.

use crate::engine::export::{self, ExportOptions, Progress};
use crate::engine::text::TextRasterizer;
use crate::hotkeys::{Action, Hotkeys};
use crate::media::waveform::WaveformCache;
use crate::media::{self, Backend};
use crate::model::{Id, Project, TrackKind};
use crate::playback::Player;
use crate::settings::Settings;
use crate::theme::{self, Palette};
use crate::ui::{inspector, library, preview, settings_ui, timeline};
use eframe::egui;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

const PROJECT_EXT: &str = "sedit";
const MEDIA_EXTS: &[&str] = &[
    "mp4", "mov", "mkv", "webm", "avi", "m4v", "wmv", "ts", "m2ts", "mts", "flv", "3gp", "mpg", "mpeg", "gif", "mp3",
    "wav", "m4a", "aac", "flac", "ogg", "opus", "wma", "png", "jpg", "jpeg", "bmp", "webp", "tif", "tiff",
];

enum ExportKind {
    File,
    Overwrite { original: PathBuf, temp: PathBuf },
}

pub struct App {
    project: Project,
    project_path: Option<PathBuf>,
    dirty: bool,
    undo: Vec<String>,
    redo: Vec<String>,
    settings: Settings,
    hotkeys: Hotkeys,
    text: Arc<Mutex<TextRasterizer>>,
    fonts: Vec<String>,
    player: Player,
    waveforms: WaveformCache,
    timeline: timeline::TimelineState,
    preview: preview::PreviewState,
    library: library::LibraryState,
    settings_ui: settings_ui::SettingsUi,
    selection: Vec<Id>,
    playhead: f64,
    export: Option<(Arc<Progress>, ExportKind)>,
    encoders: Vec<String>,
    toasts: Vec<(String, Instant)>,
    screenshot: Option<PathBuf>,
    started: Instant,
    screenshot_requested: bool,
    close_confirmed: bool,
    palette: Palette,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, open: Option<PathBuf>, screenshot: Option<PathBuf>) -> Self {
        let settings = Settings::load();
        media::ffpipe::set_dir(&settings.ffmpeg_dir);
        theme::apply(&cc.egui_ctx, &settings.theme);
        let backend = Backend::parse(&settings.decoder);
        let text = Arc::new(Mutex::new(TextRasterizer::new()));
        {
            // warm the font list off-thread so the first text clip / inspector doesn't hitch
            let t = text.clone();
            std::thread::spawn(move || {
                if let Ok(mut t) = t.lock() {
                    t.load_system_fonts();
                }
            });
        }
        if settings.context_menu && !crate::contextmenu::is_installed() {
            let _ = crate::contextmenu::install();
        }
        let player = Player::new(cc.egui_ctx.clone(), backend, text.clone());
        let waveforms = WaveformCache::new(cc.egui_ctx.clone(), backend);
        let hotkeys = Hotkeys::from_settings(&settings);
        let palette = theme::palette(&cc.egui_ctx);
        let mut app = Self {
            project: Project::new(),
            project_path: None,
            dirty: false,
            undo: Vec::new(),
            redo: Vec::new(),
            settings,
            hotkeys,
            text,
            fonts: Vec::new(),
            player,
            waveforms,
            timeline: timeline::TimelineState::default(),
            preview: preview::PreviewState::default(),
            library: library::LibraryState::default(),
            settings_ui: settings_ui::SettingsUi::default(),
            selection: Vec::new(),
            playhead: 0.0,
            export: None,
            encoders: Vec::new(),
            toasts: Vec::new(),
            screenshot,
            started: Instant::now(),
            screenshot_requested: false,
            close_confirmed: false,
            palette,
        };
        app.player.set_project(&app.project);
        if let Some(p) = open {
            app.open_path(&p);
        }
        app
    }

    // ---------------- helpers ----------------

    fn toast(&mut self, msg: impl Into<String>) {
        self.toasts.push((msg.into(), Instant::now()));
    }

    fn push_undo(&mut self) {
        self.undo.push(self.project.to_json());
        if self.undo.len() > 200 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// After any project mutation.
    fn after_edit(&mut self) {
        self.dirty = true;
        let p = &self.project;
        self.selection.retain(|id| p.clip(*id).is_some());
        self.player.set_project(&self.project);
    }

    fn set_project(&mut self, project: Project, path: Option<PathBuf>) {
        self.project = project;
        self.project_path = path;
        self.dirty = false;
        self.undo.clear();
        self.redo.clear();
        self.selection.clear();
        self.playhead = 0.0;
        self.player.pause();
        self.player.set_project(&self.project);
        self.player.seek(0.0);
        self.timeline.zoom_to_fit(self.project.duration(), self.timeline.lanes_rect.width().max(800.0));
    }

    fn title(&self) -> String {
        let name = self
            .project_path
            .as_ref()
            .map(|p| p.file_name().unwrap_or_default().to_string_lossy().into_owned())
            .unwrap_or_else(|| self.project.name.clone());
        format!("{}{} — Simple Editor", if self.dirty { "*" } else { "" }, name)
    }

    fn seek(&mut self, t: f64) {
        self.playhead = t.clamp(0.0, self.project.duration().max(0.0));
        self.player.seek(self.playhead);
    }

    fn backend(&self) -> Backend {
        Backend::parse(&self.settings.decoder)
    }

    // ---------------- file operations ----------------

    fn open_path(&mut self, path: &Path) {
        if path.extension().map(|e| e.to_string_lossy().eq_ignore_ascii_case(PROJECT_EXT)).unwrap_or(false) {
            self.open_project(path);
        } else {
            self.open_media(path);
        }
    }

    fn open_media(&mut self, path: &Path) {
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

    fn open_project(&mut self, path: &Path) {
        match Project::load(path) {
            Ok(project) => {
                self.set_project(project, Some(path.to_path_buf()));
                self.settings.touch_recent_project(&path.to_string_lossy());
                self.settings.save();
            }
            Err(e) => self.toast(format!("Can't open project: {e}")),
        }
    }

    /// Import media files into the library (probe each). Returns new asset ids.
    fn import_files(&mut self, paths: &[PathBuf]) -> Vec<Id> {
        let mut ids = Vec::new();
        let mut any = false;
        for path in paths {
            let p = path.to_string_lossy().into_owned();
            match media::probe(&p, self.backend()) {
                Ok(asset) => {
                    if !any {
                        self.push_undo();
                        any = true;
                    }
                    let id = self.project.add_asset(asset);
                    ids.push(id);
                    self.settings.touch_recent(&p);
                }
                Err(e) => self.toast(format!("Can't import {}: {e}", path.display())),
            }
        }
        if any {
            self.settings.save();
            if self.project.is_empty() && self.project.assets.len() == ids.len() {
                // first media into an empty project: adopt its format
                if let Some(a) = ids.first().and_then(|id| self.project.asset(*id)).cloned() {
                    if a.has_video() && a.width > 0 {
                        self.project.width = a.width;
                        self.project.height = a.height;
                        if a.fps > 1.0 {
                            self.project.fps = a.fps;
                        }
                    }
                }
            }
            self.after_edit();
        }
        ids
    }

    fn media_dialog() -> rfd::FileDialog {
        rfd::FileDialog::new()
            .add_filter("Media", MEDIA_EXTS)
            .add_filter("Simple Editor project", &[PROJECT_EXT])
            .add_filter("All files", &["*"])
    }

    fn act_open_file(&mut self) {
        if !self.confirm_discard() {
            return;
        }
        if let Some(p) = Self::media_dialog().pick_file() {
            self.open_path(&p);
        }
    }

    fn act_open_project(&mut self) {
        if !self.confirm_discard() {
            return;
        }
        if let Some(p) = rfd::FileDialog::new().add_filter("Simple Editor project", &[PROJECT_EXT]).pick_file() {
            self.open_project(&p);
        }
    }

    fn act_import(&mut self) {
        if let Some(paths) = Self::media_dialog().pick_files() {
            let ids = self.import_files(&paths);
            if let Some(id) = ids.last() {
                self.library.selected = Some(*id);
                self.library.tab = 0;
            }
        }
    }

    fn save_project_as(&mut self) -> bool {
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

    fn save_project(&mut self) -> bool {
        let Some(path) = self.project_path.clone() else { return self.save_project_as() };
        match self.project.save(&path) {
            Ok(()) => {
                self.dirty = false;
                self.settings.touch_recent_project(&path.to_string_lossy());
                self.settings.save();
                self.toast("Project saved");
                true
            }
            Err(e) => {
                self.toast(format!("Save failed: {e}"));
                false
            }
        }
    }

    /// Ctrl+S: project file if there is one; otherwise overwrite the opened video; otherwise Save As.
    fn act_save(&mut self) {
        if self.project_path.is_some() {
            self.save_project();
        } else if self.project.source_video.is_some() {
            self.act_overwrite();
        } else {
            self.save_project_as();
        }
    }

    /// Ask to save unsaved changes. Returns false if the user cancelled.
    fn confirm_discard(&mut self) -> bool {
        if !self.dirty {
            return true;
        }
        let r = rfd::MessageDialog::new()
            .set_title("Simple Editor")
            .set_description("Save changes to the project?")
            .set_buttons(rfd::MessageButtons::YesNoCancel)
            .set_level(rfd::MessageLevel::Warning)
            .show();
        match r {
            rfd::MessageDialogResult::Yes => self.save_project(),
            rfd::MessageDialogResult::No => true,
            _ => false,
        }
    }

    fn ffmpeg_missing(&mut self) -> bool {
        if media::ffpipe::ffmpeg_exe().is_none() {
            self.toast(
                "ffmpeg.exe not found — install FFmpeg (winget install Gyan.FFmpeg) or set its folder in Settings",
            );
            return true;
        }
        false
    }

    fn export_opts(&self, out_path: PathBuf) -> ExportOptions {
        ExportOptions {
            out_path,
            encoder: self.settings.encoder.clone(),
            crf: self.settings.crf,
            preset: self.settings.preset.clone(),
            backend: self.backend(),
        }
    }

    fn act_export(&mut self) {
        if self.export.is_some() || self.ffmpeg_missing() || self.project.is_empty() {
            return;
        }
        let mut d = rfd::FileDialog::new()
            .add_filter("MP4 video", &["mp4"])
            .add_filter("MOV video", &["mov"])
            .add_filter("MKV video", &["mkv"])
            .add_filter("WebM video", &["webm"])
            .add_filter("AVI video", &["avi"])
            .add_filter("GIF", &["gif"])
            .add_filter("MP3 audio", &["mp3"])
            .add_filter("WAV audio", &["wav"])
            .add_filter("M4A audio", &["m4a"])
            .add_filter("FLAC audio", &["flac"])
            .add_filter("Any (by extension)", &["*"])
            .set_file_name(format!("{}_edit.mp4", self.project.name));
        if let Some(dir) = self.project.source_video.as_ref().and_then(|p| Path::new(p).parent()) {
            d = d.set_directory(dir);
        }
        let Some(out) = d.save_file() else { return };
        self.player.pause();
        let prog = export::start_export(self.project.clone(), self.export_opts(out), self.text.clone());
        self.export = Some((prog, ExportKind::File));
    }

    fn act_export_lossless(&mut self) {
        if self.export.is_some() || self.ffmpeg_missing() {
            return;
        }
        if export::lossless_segments(&self.project).is_none() {
            self.toast("Lossless cut needs a plain cut of one video (no effects, text, overlays or extra media)");
            return;
        }
        let src = self.project.source_video.clone().or_else(|| self.project.assets.first().map(|a| a.path.clone()));
        let ext = src
            .as_ref()
            .and_then(|s| Path::new(s).extension().map(|e| e.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "mp4".into());
        let mut d = rfd::FileDialog::new()
            .add_filter(&format!("{} (same container)", ext.to_uppercase()), &[ext.as_str()])
            .set_file_name(format!("{}_cut.{ext}", self.project.name));
        if let Some(dir) = src.as_ref().and_then(|p| Path::new(p).parent()) {
            d = d.set_directory(dir);
        }
        let Some(out) = d.save_file() else { return };
        self.player.pause();
        let prog = export::start_lossless_cut(self.project.clone(), out);
        self.export = Some((prog, ExportKind::File));
    }

    fn act_export_xml(&mut self) {
        let Some(out) = rfd::FileDialog::new()
            .add_filter("Final Cut Pro 7 XML (Premiere / Resolve)", &["xml"])
            .set_file_name(format!("{}.xml", self.project.name))
            .save_file()
        else {
            return;
        };
        match std::fs::write(&out, crate::engine::xmeml::export_xmeml(&self.project)) {
            Ok(()) => {
                self.toast("XML exported — import it in Premiere (File ▸ Import) or Resolve (File ▸ Import ▸ Timeline)")
            }
            Err(e) => self.toast(format!("XML export failed: {e}")),
        }
    }

    /// Re-encode the timeline over the opened video file (temp file in the same folder, then replace).
    fn act_overwrite(&mut self) {
        let Some(src) = self.project.source_video.clone() else {
            self.toast("No source video to overwrite — use Export Video As");
            return;
        };
        if self.export.is_some() || self.ffmpeg_missing() || self.project.is_empty() {
            return;
        }
        if self.settings.confirm_overwrite {
            let r = rfd::MessageDialog::new()
                .set_title("Overwrite original video?")
                .set_description(format!(
                    "{src}\n\nThe file will be replaced with the edited video. This cannot be undone."
                ))
                .set_buttons(rfd::MessageButtons::OkCancel)
                .set_level(rfd::MessageLevel::Warning)
                .show();
            if r != rfd::MessageDialogResult::Ok {
                return;
            }
        }
        let original = PathBuf::from(&src);
        let ext = original.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_else(|| "mp4".into());
        let temp = original.with_file_name(format!(
            ".{}.simple-editor-tmp.{ext}",
            original.file_stem().unwrap_or_default().to_string_lossy()
        ));
        self.player.pause();
        let prog = export::start_export(self.project.clone(), self.export_opts(temp.clone()), self.text.clone());
        self.export = Some((prog, ExportKind::Overwrite { original, temp }));
    }

    fn finish_export(&mut self) {
        let Some((prog, kind)) = self.export.take() else { return };
        if let Some(e) = prog.error() {
            if let ExportKind::Overwrite { temp, .. } = &kind {
                let _ = std::fs::remove_file(temp);
            }
            if prog.is_cancelled() {
                self.toast("Export cancelled");
            } else {
                self.toast(format!("Export failed: {e}"));
            }
            return;
        }
        match kind {
            ExportKind::File => self.toast("Export finished"),
            ExportKind::Overwrite { original, temp } => {
                self.player.release_files();
                match std::fs::rename(&temp, &original) {
                    Ok(()) => {
                        self.toast("Saved over the original video");
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

    // ---------------- editing actions ----------------

    fn selected_or_under_playhead(&self) -> Vec<Id> {
        if !self.selection.is_empty() {
            return self.project.expand_links(&self.selection);
        }
        Vec::new()
    }

    fn act(&mut self, a: Action, ctx: &egui::Context) {
        use Action::*;
        match a {
            NewProject => {
                if self.confirm_discard() {
                    self.set_project(Project::new(), None);
                }
            }
            OpenFile => self.act_open_file(),
            OpenProject => self.act_open_project(),
            Save => self.act_save(),
            SaveProjectAs => {
                self.save_project_as();
            }
            ExportVideo => self.act_export(),
            ExportLossless => self.act_export_lossless(),
            ExportXml => self.act_export_xml(),
            ImportMedia => self.act_import(),
            Settings => self.settings_ui.open = !self.settings_ui.open,
            Undo => {
                if let Some(json) = self.undo.pop() {
                    self.redo.push(self.project.to_json());
                    if let Ok(p) = Project::from_json(&json) {
                        self.project = p;
                        self.after_edit();
                    }
                }
            }
            Redo => {
                if let Some(json) = self.redo.pop() {
                    self.undo.push(self.project.to_json());
                    if let Ok(p) = Project::from_json(&json) {
                        self.project = p;
                        self.after_edit();
                    }
                }
            }
            PlayPause => {
                if !self.player.is_playing() && self.playhead >= self.project.duration() - 1e-6 {
                    self.seek(0.0);
                }
                self.player.toggle();
            }
            Stop => self.player.pause(),
            StepBack => {
                self.player.pause();
                let t = self.project.snap_frame(self.playhead - self.project.frame_dur());
                self.seek(t);
            }
            StepForward => {
                self.player.pause();
                let t = self.project.snap_frame(self.playhead + self.project.frame_dur());
                self.seek(t);
            }
            GoStart => self.seek(0.0),
            GoEnd => self.seek(self.project.duration()),
            PrevCut => {
                let cuts = self.project.cut_points();
                if let Some(&t) = cuts.iter().rev().find(|&&c| c < self.playhead - 1e-4) {
                    self.seek(t);
                }
            }
            NextCut => {
                let cuts = self.project.cut_points();
                if let Some(&t) = cuts.iter().find(|&&c| c > self.playhead + 1e-4) {
                    self.seek(t);
                }
            }
            Split => {
                let only =
                    if self.selection.is_empty() { None } else { Some(self.project.expand_links(&self.selection)) };
                self.push_undo();
                let new = self.project.split_at(self.playhead, only.as_deref());
                if new.is_empty() {
                    self.undo.pop();
                } else {
                    self.after_edit();
                }
            }
            Delete | RippleDelete => {
                let ids = self.selected_or_under_playhead();
                if !ids.is_empty() {
                    self.push_undo();
                    self.project.delete_clips(&ids, a == RippleDelete);
                    self.selection.clear();
                    self.after_edit();
                }
            }
            SelectAll => self.selection = self.project.all_clips().map(|(_, c)| c.id).collect(),
            Deselect => self.selection.clear(),
            MarkIn => {
                self.project.in_point = Some(self.playhead);
                if let Some(o) = self.project.out_point {
                    if o <= self.playhead {
                        self.project.out_point = None;
                    }
                }
            }
            MarkOut => {
                self.project.out_point = Some(self.playhead);
                if let Some(i) = self.project.in_point {
                    if i >= self.playhead {
                        self.project.in_point = None;
                    }
                }
            }
            ClearInOut => {
                self.project.in_point = None;
                self.project.out_point = None;
            }
            TrimToInOut | RippleDeleteInOut => {
                let a0 = self.project.in_point.unwrap_or(0.0);
                let b0 = self.project.out_point.unwrap_or(self.project.duration());
                if b0 > a0 {
                    self.push_undo();
                    if a == TrimToInOut {
                        self.project.trim_to_range(a0, b0);
                        self.seek(0.0);
                    } else {
                        self.project.ripple_delete_range(a0, b0);
                        self.project.in_point = None;
                        self.project.out_point = None;
                        self.seek(a0);
                    }
                    self.after_edit();
                }
            }
            AddText => {
                self.push_undo();
                let id = self.project.add_text_clip(self.playhead, 5.0);
                self.selection = vec![id];
                self.after_edit();
            }
            ZoomIn => self.timeline.zoom_by(1.25, None),
            ZoomOut => self.timeline.zoom_by(0.8, None),
            ZoomFit => self.timeline.zoom_to_fit(self.project.duration(), self.timeline.lanes_rect.width()),
            LinkToggle => {
                if !self.selection.is_empty() {
                    self.push_undo();
                    let ids = self.selection.clone();
                    self.project.toggle_link(&ids);
                    self.after_edit();
                }
            }
            ToggleEnabled => {
                if let Some(first) = self.selection.first().and_then(|id| self.project.clip(*id)) {
                    let en = !first.enabled;
                    self.push_undo();
                    let ids = self.selection.clone();
                    self.project.set_enabled(&ids, en);
                    self.after_edit();
                }
            }
            NudgeLeft | NudgeRight => {
                let ids = self.selected_or_under_playhead();
                if !ids.is_empty() {
                    let dt = if a == NudgeLeft { -self.project.frame_dur() } else { self.project.frame_dur() };
                    self.push_undo();
                    if self.project.move_clips(&ids, dt, 0, None) {
                        self.after_edit();
                    } else {
                        self.undo.pop();
                    }
                }
            }
            ToggleSnap => {
                self.settings.snap = !self.settings.snap;
                self.settings.save();
            }
            AddVideoTrack | AddAudioTrack => {
                self.push_undo();
                self.project.add_track(if a == AddVideoTrack { TrackKind::Video } else { TrackKind::Audio });
                self.after_edit();
            }
            ToggleLibrary => {
                self.settings.show_library = !self.settings.show_library;
                self.settings.save();
            }
            ToggleInspector => {
                self.settings.show_inspector = !self.settings.show_inspector;
                self.settings.save();
            }
        }
        let _ = ctx;
    }

    // ---------------- UI pieces ----------------

    fn menu_item(&mut self, ui: &mut egui::Ui, a: Action, enabled: bool, out: &mut Vec<Action>) {
        let text = self.hotkeys.text(a);
        let b = egui::Button::new(a.label()).shortcut_text(text);
        if ui.add_enabled(enabled, b).clicked() {
            out.push(a);
            ui.close();
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) -> Vec<Action> {
        use Action::*;
        let mut out = Vec::new();
        let has_sel = !self.selection.is_empty();
        let has_clips = !self.project.is_empty();
        let can_overwrite = self.project.source_video.is_some() && has_clips;
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                self.menu_item(ui, NewProject, true, &mut out);
                self.menu_item(ui, OpenFile, true, &mut out);
                self.menu_item(ui, OpenProject, true, &mut out);
                let recents = self.settings.recent_projects.clone();
                ui.menu_button("Open Recent Project", |ui| {
                    if recents.is_empty() {
                        ui.label("(none)");
                    }
                    for r in recents {
                        if ui.button(&r).clicked() {
                            ui.close();
                            if self.confirm_discard() {
                                self.open_project(Path::new(&r));
                            }
                        }
                    }
                });
                self.menu_item(ui, ImportMedia, true, &mut out);
                ui.separator();
                self.menu_item(ui, Save, true, &mut out);
                self.menu_item(ui, SaveProjectAs, true, &mut out);
                ui.separator();
                self.menu_item(ui, ExportVideo, has_clips, &mut out);
                self.menu_item(ui, ExportLossless, has_clips, &mut out);
                if ui.add_enabled(can_overwrite, egui::Button::new("Overwrite Original Video…")).clicked() {
                    ui.close();
                    self.act_overwrite();
                }
                self.menu_item(ui, ExportXml, has_clips, &mut out);
                ui.separator();
                self.menu_item(ui, Settings, true, &mut out);
                ui.separator();
                if ui.button("Exit").clicked() {
                    ui.close();
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Edit", |ui| {
                self.menu_item(ui, Undo, !self.undo.is_empty(), &mut out);
                self.menu_item(ui, Redo, !self.redo.is_empty(), &mut out);
                ui.separator();
                self.menu_item(ui, Split, has_clips, &mut out);
                self.menu_item(ui, Delete, has_sel, &mut out);
                self.menu_item(ui, RippleDelete, has_sel, &mut out);
                self.menu_item(ui, NudgeLeft, has_sel, &mut out);
                self.menu_item(ui, NudgeRight, has_sel, &mut out);
                ui.separator();
                self.menu_item(ui, SelectAll, has_clips, &mut out);
                self.menu_item(ui, Deselect, has_sel, &mut out);
                self.menu_item(ui, LinkToggle, has_sel, &mut out);
                self.menu_item(ui, ToggleEnabled, has_sel, &mut out);
                ui.separator();
                self.menu_item(ui, AddText, true, &mut out);
                ui.separator();
                self.menu_item(ui, MarkIn, true, &mut out);
                self.menu_item(ui, MarkOut, true, &mut out);
                self.menu_item(ui, ClearInOut, true, &mut out);
                self.menu_item(ui, TrimToInOut, has_clips, &mut out);
                self.menu_item(ui, RippleDeleteInOut, has_clips, &mut out);
            });
            ui.menu_button("Timeline", |ui| {
                self.menu_item(ui, AddVideoTrack, true, &mut out);
                self.menu_item(ui, AddAudioTrack, true, &mut out);
                ui.separator();
                self.menu_item(ui, ZoomIn, true, &mut out);
                self.menu_item(ui, ZoomOut, true, &mut out);
                self.menu_item(ui, ZoomFit, true, &mut out);
                ui.separator();
                let mut snap = self.settings.snap;
                if ui.checkbox(&mut snap, format!("Snapping   {}", self.hotkeys.text(ToggleSnap))).changed() {
                    out.push(ToggleSnap);
                }
            });
            ui.menu_button("Playback", |ui| {
                for a in [PlayPause, Stop, StepBack, StepForward, PrevCut, NextCut, GoStart, GoEnd] {
                    self.menu_item(ui, a, true, &mut out);
                }
            });
            ui.menu_button("View", |ui| {
                let mut l = self.settings.show_library;
                if ui.checkbox(&mut l, format!("Library   {}", self.hotkeys.text(ToggleLibrary))).changed() {
                    out.push(ToggleLibrary);
                }
                let mut i = self.settings.show_inspector;
                if ui.checkbox(&mut i, format!("Inspector   {}", self.hotkeys.text(ToggleInspector))).changed() {
                    out.push(ToggleInspector);
                }
            });
            ui.menu_button("Help", |ui| {
                ui.label(format!("Simple Editor {}", env!("CARGO_PKG_VERSION")));
                ui.label(match media::ffpipe::ffmpeg_exe() {
                    Some(p) => format!("ffmpeg: {}", p.display()),
                    None => "ffmpeg: not found (export disabled)".into(),
                });
                ui.label(format!(
                    "Context menu: {}",
                    if crate::contextmenu::is_installed() { "installed" } else { "not installed" }
                ));
                ui.label("Mouse: Ctrl+Scroll zoom · Shift+Scroll pan · Alt+Scroll track height");
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(format!(
                        "{}  /  {}",
                        crate::ui::timecode(self.playhead, self.project.fps),
                        crate::ui::timecode(self.project.duration(), self.project.fps)
                    ))
                    .monospace(),
                );
                if self.player.is_playing() {
                    ui.label("▶");
                }
            });
        });
        out
    }

    fn handle_drops(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        if dropped.is_empty() {
            return;
        }
        let pos = ctx.input(|i| i.pointer.latest_pos()).or_else(|| ctx.input(|i| i.pointer.hover_pos()));
        // a project file: open it
        if dropped.len() == 1
            && dropped[0].extension().map(|e| e.to_string_lossy().eq_ignore_ascii_case(PROJECT_EXT)).unwrap_or(false)
        {
            if self.confirm_discard() {
                self.open_project(&dropped[0]);
            }
            return;
        }
        // empty project + single media file: open it as the project
        if self.project.is_empty() && self.project.assets.is_empty() && dropped.len() == 1 {
            self.open_media(&dropped[0]);
            return;
        }
        let ids = self.import_files(&dropped);
        if ids.is_empty() {
            return;
        }
        let on_timeline = pos.map(|p| self.timeline.lanes_rect.contains(p)).unwrap_or(false);
        if on_timeline {
            let p = pos.unwrap();
            let mut t = self.timeline.time_at(p.x).max(0.0);
            if self.settings.snap {
                t = self.project.snap_frame(t);
            }
            let track = self.timeline.track_at(p.y, &self.project);
            let vt = track.filter(|&i| self.project.tracks[i].kind == TrackKind::Video);
            for id in ids {
                let new = self.project.insert_asset_clips(id, t, vt);
                if let Some(c) = new.first().and_then(|c| self.project.clip(*c)) {
                    t = c.end();
                }
            }
            self.after_edit();
        } else {
            self.library.tab = 0;
            self.library.selected = ids.last().copied();
        }
    }

    fn screenshot_tick(&mut self, ctx: &egui::Context) {
        let Some(path) = self.screenshot.clone() else { return };
        // save when the screenshot event arrives
        let img = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = img {
            let [w, h] = img.size;
            let mut data = format!("P6\n{w} {h}\n255\n").into_bytes();
            for p in &img.pixels {
                data.extend_from_slice(&[p.r(), p.g(), p.b()]);
            }
            let _ = std::fs::write(&path, data);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            self.screenshot = None;
            return;
        }
        if !self.screenshot_requested && self.started.elapsed().as_secs_f32() > 2.5 {
            self.screenshot_requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.palette = theme::palette(ctx);
        if self.fonts.is_empty() {
            if let Ok(t) = self.text.try_lock() {
                if t.is_loaded() {
                    self.fonts = t.families().to_vec();
                }
            }
        }

        // close handling: confirm unsaved changes
        if ctx.input(|i| i.viewport().close_requested()) && !self.close_confirmed {
            if self.dirty && self.screenshot.is_none() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                if self.confirm_discard() {
                    self.close_confirmed = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            } else {
                self.close_confirmed = true;
            }
        }

        // playback clock
        if self.player.is_playing() {
            self.playhead = self.player.time();
            self.timeline.ensure_visible(self.playhead);
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
        let new_frame = self.player.take_frame();

        // export progress
        if let Some((prog, _)) = &self.export {
            if prog.is_done() {
                self.finish_export();
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
        }

        self.handle_drops(ctx);
        self.screenshot_tick(ctx);

        // hotkeys
        let mut actions = self.hotkeys.poll(ctx);
        if !ctx.wants_keyboard_input() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::Y)) {
            actions.push(Action::Redo);
        }
        if !ctx.wants_keyboard_input() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace))
        {
            actions.push(Action::Delete);
        }

        ctx.send_viewport_cmd(egui::ViewportCommand::Title(self.title()));

        // ---- layout ----
        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            actions.extend(self.menu_bar(ui));
        });

        egui::TopBottomPanel::bottom("timeline").resizable(true).default_height(300.0).min_height(120.0).show(
            ctx,
            |ui| {
                let App {
                    project,
                    selection,
                    playhead,
                    undo,
                    redo,
                    waveforms,
                    timeline: tl,
                    settings,
                    player,
                    palette,
                    ..
                } = self;
                let mut push = |p: &Project| {
                    undo.push(p.to_json());
                    redo.clear();
                };
                let resp = timeline::show(
                    ui,
                    tl,
                    timeline::TimelineCtx {
                        project,
                        selection,
                        playhead,
                        undo: &mut push,
                        waveforms,
                        palette,
                        snap: settings.snap,
                        playing: player.is_playing(),
                    },
                );
                if resp.seeked {
                    player.pause();
                    let t = *playhead;
                    player.seek(t);
                }
                if resp.edited {
                    self.after_edit();
                }
                if !resp.dropped_files.is_empty() {
                    for (path, t, track) in resp.dropped_files {
                        let ids = self.import_files(&[path]);
                        let vt = track.filter(|&i| self.project.tracks[i].kind == TrackKind::Video);
                        let mut at = t;
                        for id in ids {
                            let new = self.project.insert_asset_clips(id, at, vt);
                            if let Some(c) = new.first().and_then(|c| self.project.clip(*c)) {
                                at = c.end();
                            }
                        }
                    }
                    self.after_edit();
                }
            },
        );

        if self.settings.show_library {
            egui::SidePanel::left("library").resizable(true).default_width(230.0).show(ctx, |ui| {
                let resp = library::show(ui, &mut self.library, &self.project, &self.settings, &self.palette);
                if resp.import {
                    self.act_import();
                }
                if !resp.add_to_timeline.is_empty() {
                    self.push_undo();
                    let mut t = self.playhead;
                    for id in resp.add_to_timeline {
                        let new = self.project.insert_asset_clips(id, t, None);
                        if let Some(c) = new.first().and_then(|c| self.project.clip(*c)) {
                            t = c.end();
                        }
                    }
                    self.after_edit();
                }
                if !resp.open_paths.is_empty() {
                    if self.project.is_empty() && self.project.assets.is_empty() && resp.open_paths.len() == 1 {
                        self.open_media(&resp.open_paths[0]);
                    } else {
                        let ids = self.import_files(&resp.open_paths);
                        self.library.selected = ids.last().copied();
                        self.library.tab = 0;
                    }
                }
                if !resp.remove.is_empty() {
                    self.push_undo();
                    for id in resp.remove {
                        self.project.remove_asset(id);
                    }
                    self.after_edit();
                }
                if resp.clear_recent {
                    self.settings.recent_assets.clear();
                    self.settings.save();
                }
            });
        }

        if self.settings.show_inspector {
            egui::SidePanel::right("inspector").resizable(true).default_width(290.0).show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let App { project, selection, playhead, undo, redo, fonts, palette, .. } = self;
                    let mut push = |p: &Project| {
                        undo.push(p.to_json());
                        redo.clear();
                    };
                    if inspector::show(ui, project, selection, *playhead, fonts, palette, &mut push) {
                        self.after_edit();
                    }
                });
            });
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            let App { project, selection, playhead, undo, redo, preview: pv, player, palette, .. } = self;
            let mut push = |p: &Project| {
                undo.push(p.to_json());
                redo.clear();
            };
            let resp = preview::show(
                ui,
                pv,
                preview::PreviewCtx {
                    project,
                    selection,
                    playhead: *playhead,
                    playing: player.is_playing(),
                    palette,
                    undo: &mut push,
                    frame: new_frame,
                },
            );
            player.set_canvas(resp.canvas.0, resp.canvas.1, self.settings.preview_max_width);
            if let Some(t) = resp.seek {
                self.seek(t);
            }
            if resp.toggle_play {
                actions.push(Action::PlayPause);
            }
            if resp.stop {
                actions.push(Action::Stop);
            }
            if resp.step < 0 {
                actions.push(Action::StepBack);
            }
            if resp.step > 0 {
                actions.push(Action::StepForward);
            }
            if resp.prev_cut {
                actions.push(Action::PrevCut);
            }
            if resp.next_cut {
                actions.push(Action::NextCut);
            }
            if resp.go_start {
                actions.push(Action::GoStart);
            }
            if resp.go_end {
                actions.push(Action::GoEnd);
            }
            if resp.mark_in {
                actions.push(Action::MarkIn);
            }
            if resp.mark_out {
                actions.push(Action::MarkOut);
            }
            if resp.clear_in_out {
                actions.push(Action::ClearInOut);
            }
            if resp.edited {
                self.after_edit();
            }
        });

        for a in actions {
            self.act(a, ctx);
        }

        // settings window
        if self.settings_ui.open {
            let backend_before = self.settings.decoder.clone();
            let theme_before = self.settings.theme.clone();
            let ctxmenu_before = self.settings.context_menu;
            let ffdir_before = self.settings.ffmpeg_dir.clone();
            if self.encoders.is_empty() && media::ffpipe::ffmpeg_exe().is_some() {
                self.encoders = export::detect_encoders();
            }
            let changed = settings_ui::show(
                ctx,
                &mut self.settings_ui,
                &mut self.settings,
                &mut self.hotkeys,
                &self.encoders,
                &self.palette,
            );
            if changed {
                self.hotkeys.to_settings(&mut self.settings);
                self.settings.save();
                if self.settings.theme != theme_before {
                    theme::apply(ctx, &self.settings.theme);
                }
                if self.settings.ffmpeg_dir != ffdir_before {
                    media::ffpipe::set_dir(&self.settings.ffmpeg_dir);
                    self.encoders.clear();
                }
                if self.settings.decoder != backend_before {
                    let b = self.backend();
                    self.player.set_backend(b);
                    self.waveforms.set_backend(b);
                }
                if self.settings.context_menu != ctxmenu_before {
                    let r = if self.settings.context_menu {
                        crate::contextmenu::install()
                    } else {
                        crate::contextmenu::uninstall()
                    };
                    if let Err(e) = r {
                        self.toast(format!("Context menu: {e}"));
                    }
                }
            }
        }

        // export progress modal
        if let Some((prog, _)) = &self.export {
            let prog = prog.clone();
            egui::Modal::new(egui::Id::new("export_modal")).show(ctx, |ui| {
                ui.set_width(360.0);
                ui.heading("Exporting…");
                ui.add(egui::ProgressBar::new(prog.fraction()).show_percentage());
                ui.label(prog.status());
                if ui.button("Cancel").clicked() {
                    prog.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
                }
            });
        }

        // toasts
        self.toasts.retain(|(_, t)| t.elapsed().as_secs_f32() < 5.0);
        if !self.toasts.is_empty() {
            egui::Area::new(egui::Id::new("toasts"))
                .anchor(egui::Align2::RIGHT_BOTTOM, [-12.0, -12.0])
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    for (msg, _) in &self.toasts {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            ui.label(msg);
                        });
                    }
                });
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
    }
}
