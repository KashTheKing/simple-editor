//! ---- ws:pages ----
//! The menu bar: File / Edit / Clip / Timeline / Playback / Window / Help, then the page switcher
//! (centred) and the running-jobs indicator (right). Every Action row is `ui::menu::action_item` and
//! every other row `ui::menu::row` / `check` / `sub`, so labels, icons and shortcut text line up and
//! match the right-click menus and the palette. The panes each menu reaches the rest through Ctrl+K.

use super::thumbs::*;
use super::*;
use crate::ui::menu;
use std::sync::atomic::Ordering;

/// Action rows (`None` = separator): `menu::action_menu`'s rows without its own scroll area, so a menu
/// mixing them with submenus and plain rows scrolls once and every row shares one width.
fn acts(ui: &mut egui::Ui, items: &[Option<Action>]) {
    for &it in items {
        match it {
            Some(a) => {
                menu::action_item(ui, a);
            }
            None => {
                ui.separator();
            }
        }
    }
}

/// `acts`, greyed unless `on` - a guard only this menu knows ("something is selected").
fn acts_if(ui: &mut egui::Ui, on: bool, items: &[Option<Action>]) {
    ui.add_enabled_ui(on, |ui| acts(ui, items));
}

/// Window menu: the everyday panels first, then the niche ones under More ▸ (hidden, not deleted).
const MAIN_PANES: [Pane; 14] = [
    Pane::Library,
    Pane::Effects,
    Pane::Transitions,
    Pane::Presets,
    Pane::Source,
    Pane::Preview,
    Pane::Inspector,
    Pane::Timeline,
    Pane::Mixer,
    Pane::Subtitles,
    Pane::Markers,
    Pane::Scopes,
    Pane::Export,
    Pane::Jobs,
];
const MORE_PANES: [Pane; 7] =
    [Pane::Curves, Pane::Nodes, Pane::Tracking, Pane::Planner, Pane::Moodboard, Pane::History, Pane::AutoCut];

/// The Action that toggles a pane: its Window-menu row's shortcut text, and its one palette row.
pub(super) fn toggle_action(p: Pane) -> Option<Action> {
    use Action::*;
    Some(match p {
        Pane::Library => ToggleLibrary,
        Pane::Inspector => ToggleInspector,
        Pane::Effects => ToggleEffects,
        Pane::Transitions => ToggleTransitions,
        Pane::Curves => ToggleCurves,
        Pane::Subtitles => ToggleSubtitles,
        Pane::Planner => TogglePlanner,
        Pane::Markers => ToggleMarkers,
        Pane::Nodes => ToggleNodes,
        Pane::Mixer => ToggleMixer,
        Pane::Tools => ToggleTools,
        Pane::Source => ToggleSource,
        Pane::Jobs => ToggleJobs,
        Pane::Scopes => ToggleScopes,
        _ => return None,
    })
}

/// Where Help ▸ Tutorial & docs goes (the Docusaurus site, wave 2 of plans/simplify).
const DOCS_URL: &str = "https://kashtheking.github.io/simple-editor/";

impl App {
    /// (Re)load the editor background image texture when the path or blur setting changed.
    pub(super) fn ensure_bg_texture(&mut self, ctx: &egui::Context) {
        let (path, blur) = (self.settings.bg_image.clone(), self.settings.bg_blur);
        if path.is_empty() {
            self.bg_tex = None;
            return;
        }
        if matches!(&self.bg_tex, Some((p, b, _)) if *p == path && *b == blur) {
            return;
        }
        let mut f = Frame::default();
        let ok = media::open_video(&path, self.backend())
            .ok()
            .map(|mut src| src.frame_at(0.0, 1280, 720, &mut f) && !f.is_empty())
            .unwrap_or(false);
        if !ok {
            self.toast("Background image could not be read");
            self.settings.bg_image.clear();
            self.settings.save();
            self.bg_tex = None;
            return;
        }
        if blur > 0 {
            box_blur(&mut f.rgba, f.width as usize, f.height as usize, blur as usize);
        }
        let img = egui::ColorImage::from_rgba_premultiplied([f.width as usize, f.height as usize], &f.rgba);
        let tex = ctx.load_texture("editor-bg", img, egui::TextureOptions::LINEAR);
        self.bg_tex = Some((path, blur, tex));
    }

    /// The menu bar. Action rows queue through `ui::menu`; the few check rows that flip an Action
    /// return it here.
    pub(super) fn menu_bar(&mut self, ui: &mut egui::Ui) -> Vec<Action> {
        let mut out = Vec::new();
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| menu::scroll(ui, |ui| self.file_menu(ui)));
            ui.menu_button("Edit", |ui| menu::scroll(ui, |ui| self.edit_menu(ui)));
            ui.menu_button("Clip", |ui| menu::scroll(ui, |ui| self.clip_menu(ui)));
            ui.menu_button("Timeline", |ui| menu::scroll(ui, |ui| self.timeline_menu(ui, &mut out)));
            ui.menu_button("Playback", |ui| menu::scroll(ui, |ui| self.playback_menu(ui, &mut out)));
            ui.menu_button("Window", |ui| menu::scroll(ui, |ui| self.window_menu(ui)));
            ui.menu_button("Help", |ui| self.help_menu(ui));
            layout_ctl::page_switcher(self, ui);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // ---- ws:jobs-panel ----
                jobs_pane::indicator(self, ui);
            });
        });
        out
    }

    fn file_menu(&mut self, ui: &mut egui::Ui) {
        use Action::*;
        let has_clips = !self.timeline_is_empty();
        acts(ui, &[Some(NewProject), Some(OpenFile)]);
        self.recent_menu(ui);
        acts(ui, &[Some(Save), Some(SaveProjectAs), None]);
        menu::sub(ui, Some(tools::Glyph::ImportArrow), "Import", |ui| {
            acts(ui, &[Some(ImportMedia), Some(ImportTimeline)]);
            let yt = self.ytdlp_available.load(Ordering::Relaxed);
            let url = ui.add_enabled_ui(yt, |ui| menu::row(ui, None, "From URL…", "")).inner;
            if url.on_disabled_hover_text("yt-dlp not found - set its folder in Settings ▸ General").clicked() {
                self.url_dialog = Some((String::new(), false));
            }
        });
        menu::sub(ui, Some(tools::Glyph::ExportArrow), "Export", |ui| {
            acts(ui, &[Some(ExportVideo)]);
            acts_if(ui, has_clips, &[Some(QuickExport), Some(ExportFrame), Some(ExportXml), Some(ExportLossless)]);
            let can_overwrite = self.project.source_video.is_some() && has_clips;
            let r = ui.add_enabled_ui(can_overwrite, |ui| menu::row(ui, None, "Overwrite Original Video…", "")).inner;
            if r.on_disabled_hover_text("Only for a project opened from a single video").clicked() {
                self.act_overwrite();
            }
            acts(ui, &[None, Some(ExportMarkers)]);
            if menu::row(ui, None, "Style Summary (.md)…", "").clicked() {
                self.act_export_style();
            }
        });
        menu::sub(ui, Some(tools::Glyph::Record), "Record", |ui| acts(ui, &[Some(Voiceover), Some(ScreenCapture)]));
        acts(ui, &[None, Some(Settings), None]);
        if menu::row(ui, None, "Exit", "").clicked() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// File ▸ Open Recent ▸: a row per project (folder as the shortcut text, full path on hover,
    /// right-click to forget it).
    fn recent_menu(&mut self, ui: &mut egui::Ui) {
        let recents = self.settings.recent_projects.clone();
        let mut forget: Option<Option<String>> = None; // Some(path) = drop one, None = clear all
        let mut open = None;
        menu::sub(ui, Some(tools::Glyph::Folder), "Open Recent", |ui| {
            ui.set_max_width(420.0);
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
            if recents.is_empty() {
                ui.add_enabled_ui(false, |ui| menu::row(ui, None, "(none)", ""));
            }
            for r in &recents {
                let p = Path::new(r);
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| r.clone());
                let folder = p.parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
                let resp = menu::row(ui, None, &name, &folder);
                resp.context_menu(|ui| {
                    if menu::row(ui, Some(tools::Glyph::Cross), "Remove from recent", "").clicked() {
                        forget = Some(Some(r.clone()));
                    }
                });
                if resp.on_hover_text(r).clicked() {
                    open = Some(Path::new(r).to_path_buf());
                }
            }
            if !recents.is_empty() {
                ui.separator();
                if menu::row(ui, None, "Clear history", "").clicked() {
                    forget = Some(None);
                }
            }
        });
        if let Some(path) = open {
            self.confirm_discard_then(move |app| app.open_project(&path));
        }
        if let Some(one) = forget {
            match one {
                Some(path) => self.settings.recent_projects.retain(|p| *p != path),
                None => self.settings.recent_projects.clear(),
            }
            self.settings.save();
        }
    }

    fn edit_menu(&mut self, ui: &mut egui::Ui) {
        use Action::*;
        let has_sel = !self.selection.is_empty();
        let has_clips = !self.timeline_is_empty();
        let pasteable = self.clipboard.is_some();
        acts(ui, &[Some(Undo), Some(Redo), None]);
        acts_if(ui, has_sel, &[Some(CutClips), Some(CopyClips)]);
        acts_if(ui, pasteable, &[Some(PasteClips)]);
        menu::sub(ui, Some(tools::Glyph::Paste), "Paste Special", |ui| {
            acts_if(ui, pasteable, &[Some(PasteInsert), Some(PasteAtTop), Some(PasteInPlace)]);
            acts_if(ui, has_sel, &[Some(PasteAttributes)]);
        });
        acts_if(ui, has_sel, &[Some(CopyAttributes), None, Some(Delete), Some(RippleDelete), None]);
        acts_if(ui, has_clips, &[Some(SelectAll)]);
        acts_if(ui, has_sel, &[Some(Deselect)]);
        menu::sub(ui, None, "Select", |ui| {
            acts_if(ui, has_clips, &[Some(SelectForward), Some(SelectBackward), Some(SelectAtPlayhead)]);
        });
        acts(ui, &[None, Some(Find)]);
    }

    fn clip_menu(&mut self, ui: &mut egui::Ui) {
        use Action::*;
        let has_sel = !self.selection.is_empty();
        acts_if(ui, !self.timeline_is_empty(), &[Some(Split)]);
        acts_if(ui, has_sel, &[Some(DuplicateClips), Some(ToggleEnabled), Some(LinkToggle), None]);
        acts_if(ui, has_sel, &[Some(Retime), Some(FreezeFrame), None]);
        menu::sub(ui, None, "Add", |ui| {
            acts(ui, &[Some(AddText), Some(AddShape), Some(AddAdjustment)]);
            acts_if(ui, has_sel, &[Some(AddMask)]);
            acts(ui, &[Some(AddSubtitle), Some(AddContainer)]);
        });
        menu::sub(ui, Some(tools::Glyph::Transition), "Transition", |ui| {
            acts_if(ui, has_sel, &[Some(AddTransition), Some(AddLastTransition), Some(AddTransitionEnd)]);
        });
        ui.separator();
        acts_if(ui, has_sel, &[Some(NestSequence), Some(UnnestClip)]);
        acts_if(ui, self.selection.len() >= 2, &[Some(MulticamCreate)]);
        acts_if(ui, tools_monitor::multicam_available(self), &[Some(MulticamAngles)]);
        acts(ui, &[None, Some(MatchFrame), Some(RevealInLibrary), None]);
        acts_if(ui, has_sel, &[Some(SaveTemplate)]);
        acts_if(ui, self.selection.len() == 2, &[Some(ApplyFlow)]);
    }

    fn timeline_menu(&mut self, ui: &mut egui::Ui, out: &mut Vec<Action>) {
        use Action::*;
        let has_clips = !self.timeline_is_empty();
        menu::sub(ui, None, "Mark", |ui| {
            let items = [MarkIn, MarkOut, MarkClip, ClearInOut].map(Some);
            acts(ui, &items);
            acts(ui, &[None, Some(GoToIn), Some(GoToOut)]);
        });
        acts_if(ui, has_clips, &[Some(LiftInOut), Some(ExtractInOut), Some(TrimToInOut), Some(CloseGapAtPlayhead)]);
        menu::sub(ui, None, "Trim", |ui| {
            acts(ui, &[Some(SelectEditPoint), Some(CycleEditSide), None]);
            acts(ui, &[TrimLeft1, TrimRight1, TrimLeft10, TrimRight10].map(Some));
            acts(ui, &[None, Some(ExtendEdit), Some(TrimTop), Some(TrimTail), None, Some(SlipLeft), Some(SlipRight)]);
        });
        acts(ui, &[Some(AddMarker), None]);
        menu::sub(ui, Some(tools::Glyph::Waveform), "Auto", |ui| {
            acts(ui, &[Some(Action::AutoCut)]);
            // scene detection lives in the Auto-cut panel, beside silence detection
            if menu::row(ui, None, "Scene Cuts…", "").clicked() {
                out.push(Action::AutoCut);
            }
            acts(ui, &[Some(DetectBeats), Some(SplitAtBeats), Some(RemoveFillers), Some(TranscribeClip), None]);
            acts(ui, &[Some(AutoDuck), Some(Normalize), Some(MatchLoudness), None, Some(AutoColor), Some(ColorMatch)]);
        });
        menu::sub(ui, None, "Tracks", |ui| {
            acts(ui, &[Some(AddVideoTrack), Some(AddAudioTrack), None]);
            acts(ui, &[ToggleTrackLock, ToggleTrackRipple, ToggleTrackMagnetic, RenameTrack].map(Some));
        });
        acts(ui, &[None, Some(ZoomIn), Some(ZoomOut), Some(ZoomFit)]);
        if menu::check(ui, self.settings.snap, "Snapping", &menu::shortcut(ToggleSnap)).clicked() {
            out.push(ToggleSnap);
        }
        ui.separator();
        acts(ui, &[Some(RenderSelection)]);
        acts_if(ui, !self.selection.is_empty(), &[Some(BakeSelection)]);
        menu::sub(ui, None, "Scaling Quality", |ui| {
            for s in Scaler::ALL {
                if menu::check(ui, self.project.scaler == s, s.name(), "").clicked() && self.project.scaler != s {
                    self.push_undo();
                    self.project.scaler = s;
                    self.after_edit();
                }
            }
        });
    }

    fn playback_menu(&mut self, ui: &mut egui::Ui, out: &mut Vec<Action>) {
        use Action::*;
        acts(ui, &[Some(PlayPause), Some(ShuttleBack), Some(Stop), Some(ShuttleFwd), None]);
        acts(ui, &[Some(PlayInOut), Some(LoopInOut), Some(PlayAround), None]);
        acts(ui, &[StepBack, StepForward, StepBack10, StepFwd10, PrevCut, NextCut, GoStart, GoEnd].map(Some));
        ui.separator();
        if menu::check(ui, self.settings.use_proxies, "Proxies", &menu::shortcut(ToggleProxies)).clicked() {
            out.push(ToggleProxies);
        }
        if menu::check(ui, self.settings.movie_mode, "Movie Mode (pre-render)", &menu::shortcut(MovieMode)).clicked() {
            out.push(MovieMode);
        }
        menu::sub(ui, None, "Playback Resolution", |ui| {
            for q in crate::ui::preview::QUALITIES {
                if menu::check(ui, self.settings.preview_quality == q, &format!("{q} %"), "").clicked() {
                    self.settings.preview_quality = q;
                    self.settings.save();
                }
            }
        });
        acts(ui, &[None, Some(Fullscreen)]);
    }

    fn window_menu(&mut self, ui: &mut egui::Ui) {
        menu::sub(ui, None, "Pages", |ui| {
            for (&page, &a) in layout::PAGES.iter().zip(&layout_ctl::PAGE_ACTIONS) {
                if menu::check(ui, self.settings.page == page, a.label(), &menu::shortcut(a)).clicked() {
                    layout_ctl::switch_page(self, page);
                }
            }
        });
        ui.separator();
        for p in MAIN_PANES {
            self.pane_row(ui, p);
        }
        menu::sub(ui, None, "More", |ui| {
            for p in MORE_PANES {
                self.pane_row(ui, p);
            }
        });
        ui.separator();
        menu::sub(ui, Some(tools::Glyph::Guides), "Social Guides", |ui| {
            use crate::ui::guides::Guide;
            for g in std::iter::once(None).chain(Guide::ALL.map(Some)) {
                let name = g.map_or("Off", |g| g.name());
                if menu::check(ui, self.settings.guide == g, name, "").clicked() {
                    self.settings.guide = g;
                    self.settings.save();
                }
            }
        });
        menu::sub(ui, None, "Layout", |ui| self.layout_menu(ui));
        acts(ui, &[Some(Action::MaximizePane)]);
        // ---- ws:command-palette ---- (per-script icon/hotkey/desc from ScriptMeta)
        if !self.script_metas().is_empty() {
            menu::sub(ui, Some(tools::Glyph::Terminal), "Scripts", |ui| {
                for m in self.script_metas().to_vec() {
                    let glyph = m.icon.and_then(tools::Glyph::from_name).unwrap_or(tools::Glyph::Terminal);
                    let r = menu::row(ui, Some(glyph), &m.name, m.hotkey.as_deref().unwrap_or(""));
                    let r = if m.desc.is_empty() { r } else { r.on_hover_text(&m.desc) };
                    if r.clicked() {
                        self.run_script_path = Some(m.path.clone());
                    }
                }
                ui.separator();
                if menu::row(ui, Some(tools::Glyph::Folder), "Open Scripts Folder", "").clicked() {
                    let _ = std::process::Command::new("explorer").arg(crate::scripting::scripts_dir()).spawn();
                }
            });
        }
    }

    /// A Window-menu row: the pane's name, ticked while it is on screen; a click shows or hides it.
    fn pane_row(&mut self, ui: &mut egui::Ui, p: Pane) {
        let key = toggle_action(p).map(menu::shortcut).unwrap_or_default();
        if menu::check(ui, self.layout.is_visible(p), p.title(), &key).clicked() {
            self.toggle_pane(p);
        }
    }

    /// Window ▸ Layout ▸: Lock panels, this page's reset / saved default, and the layout profiles (which
    /// load onto the page on screen).
    fn layout_menu(&mut self, ui: &mut egui::Ui) {
        let lock = menu::check(ui, self.settings.panels_locked, "Lock Panels", "")
            .on_hover_text("Tabs only click. Unlocked, drag a tab to re-dock it; dividers resize either way.");
        if lock.clicked() {
            self.settings.panels_locked = !self.settings.panels_locked;
            self.settings.save();
        }
        ui.separator();
        let page = layout::page_name(&self.settings.page).unwrap_or("Edit");
        let saved = self.settings.page_defaults.contains_key(page);
        let hint =
            if saved { "Back to the default you saved for this page" } else { "Back to this page's built-in layout" };
        if menu::row(ui, None, "Reset Page Layout", "").on_hover_text(hint).clicked() {
            layout_ctl::reset_page(self, page, false);
        }
        let builtin = ui.add_enabled_ui(saved, |ui| menu::row(ui, None, "Reset to Built-in Layout", "")).inner;
        if builtin.on_disabled_hover_text(layout_ctl::NO_SAVED_DEFAULT).clicked() {
            layout_ctl::reset_page(self, page, true);
        }
        let save = menu::row(ui, None, "Save as This Page's Default", "")
            .on_hover_text("Reset Page Layout comes back to this arrangement");
        if save.clicked() {
            layout_ctl::save_page_default(&mut self.settings, &self.layout);
            self.settings.save();
            self.toast(format!("Saved as the {page} page's default"));
        }
        ui.separator();
        if menu::row(ui, None, "Save Profile…", "").clicked() {
            self.profile_name = Some(String::new());
        }
        let profiles = self.settings.layout_profiles.clone();
        menu::sub(ui, None, "Load Profile", |ui| {
            if profiles.is_empty() {
                ui.add_enabled_ui(false, |ui| menu::row(ui, None, "(none)", ""));
            }
            for p in profiles {
                if layout::profile_button(ui, &p.name).clicked() {
                    ui.close();
                    match Layout::from_json_migrating(&p.json) {
                        Some(l) => {
                            self.layout = l;
                            self.layout_dirty = true;
                        }
                        None => self.toast(format!("Profile '{}' could not be read", p.name)),
                    }
                }
            }
        });
        if menu::row(ui, None, "Export Profile to File…", "").clicked() {
            if let Some(out) = rfd::FileDialog::new()
                .add_filter("Simple Editor layout", &["sedit-layout"])
                .set_file_name("layout.sedit-layout")
                .save_file()
            {
                match std::fs::write(&out, self.layout.to_json()) {
                    Ok(()) => self.toast_with_folder("Layout exported", out),
                    Err(e) => self.toast(format!("Layout export failed: {e}")),
                }
            }
        }
        if menu::row(ui, None, "Import Profile from File…", "").clicked() {
            if let Some(p) =
                rfd::FileDialog::new().add_filter("Simple Editor layout", &["sedit-layout", "json"]).pick_file()
            {
                match std::fs::read_to_string(&p).ok().and_then(|s| Layout::from_json_migrating(&s)) {
                    Some(l) => {
                        let name = p
                            .file_stem()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "Imported".into());
                        self.settings.layout_profiles.retain(|x| x.name != name);
                        self.settings.layout_profiles.push(crate::settings::LayoutProfile { name, json: l.to_json() });
                        self.settings.save();
                        self.layout = l;
                        self.layout_dirty = true;
                    }
                    None => self.toast("Not a valid layout file"),
                }
            }
        }
    }

    fn help_menu(&mut self, ui: &mut egui::Ui) {
        use Action::*;
        if menu::row(ui, None, "Tutorial & Docs", "").on_hover_text(DOCS_URL).clicked() {
            let _ = std::process::Command::new("explorer").arg(DOCS_URL).spawn();
        }
        acts(ui, &[Some(CheatSheet), Some(CommandPalette), None, Some(WhatsNew), Some(ShowWelcome), None]);
        ui.weak(format!("Simple Editor {}", env!("CARGO_PKG_VERSION")));
        ui.weak(match media::ffpipe::ffmpeg_exe() {
            Some(p) => format!("ffmpeg: {}", p.display()),
            None => "ffmpeg: not found (export disabled)".into(),
        });
    }
}
