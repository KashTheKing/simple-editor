use super::thumbs::*;
use super::*;

impl App {
    pub(super) fn glyph_for(&self, a: Action) -> Option<tools::Glyph> {
        if let Some(name) = self.settings.icon_overrides.get(&format!("action.{}", a.id())) {
            return if name == "none" { None } else { tools::Glyph::from_name(name) };
        }
        tools::action_glyph(a)
    }

    pub(super) fn menu_item(&mut self, ui: &mut egui::Ui, a: Action, enabled: bool, out: &mut Vec<Action>) {
        let text = self.hotkeys.text(a);
        let glyph = self.glyph_for(a);
        // ponytail: the glyph is painted over a left gutter made of spaces in the label — that keeps
        // egui's own menu-button sizing/shortcut layout instead of reimplementing the widget.
        // Gutter must clear the 24px-wide icon box drawn below (starts at +4px); at the 13px menu
        // font a space is ~3px wide, so 5 spaces (~15px) undershot it and the label crowded the icon.
        let label = match glyph {
            Some(_) => format!("         {}", a.label()),
            None => a.label().to_string(),
        };
        let b = egui::Button::new(label).shortcut_text(text);
        let r = ui.add_enabled(enabled, b);
        if let Some(g) = glyph {
            let rect = egui::Rect::from_min_size(
                egui::pos2(r.rect.min.x + 4.0, r.rect.center().y - 11.0),
                egui::vec2(24.0, 22.0),
            );
            let fg = if enabled { ui.visuals().text_color() } else { ui.visuals().weak_text_color() };
            tools::draw_glyph(ui.painter(), rect, g, fg);
        }
        // right-click any menu action: pick its icon in place (same picker as the tab context menu)
        let mut set: Option<Option<String>> = None;
        r.context_menu(|ui| {
            if let Some(pick) = layout::icon_menu(ui) {
                set = Some(pick);
            }
        });
        if let Some(pick) = set {
            let key = format!("action.{}", a.id());
            match pick {
                Some(name) => drop(self.settings.icon_overrides.insert(key, name)),
                None => drop(self.settings.icon_overrides.remove(&key)),
            }
            self.settings.save();
        }
        if r.clicked() {
            out.push(a);
            ui.close();
        }
    }

    /// Icon shown next to a pane (View menu, icon picker), with the user's Settings override first.
    pub(super) fn pane_glyph(&self, p: Pane) -> Option<tools::Glyph> {
        layout::pane_icon(&self.settings.icon_overrides, p)
    }

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

    pub(super) fn view_menu(&mut self, ui: &mut egui::Ui, out: &mut Vec<Action>) {
        use Action::*;
        const PANES: [(Pane, Option<Action>); 19] = [
            (Pane::Preview, None),
            (Pane::Timeline, None),
            (Pane::Tools, Some(ToggleTools)),
            (Pane::Library, Some(ToggleLibrary)),
            (Pane::Inspector, Some(ToggleInspector)),
            (Pane::Effects, Some(ToggleEffects)),
            (Pane::Transitions, Some(ToggleTransitions)),
            (Pane::Curves, Some(ToggleCurves)),
            (Pane::Nodes, Some(ToggleNodes)),
            (Pane::Subtitles, Some(ToggleSubtitles)),
            (Pane::Markers, Some(ToggleMarkers)),
            (Pane::Mixer, Some(ToggleMixer)),
            (Pane::Presets, None),
            (Pane::Planner, Some(TogglePlanner)),
            (Pane::AutoCut, Some(Action::AutoCut)),
            (Pane::Tracking, None),
            (Pane::Moodboard, None),
            (Pane::History, None),
            // ---- ws:registries-schema-hooks ----
            (Pane::Source, None),
        ];
        for (pane, action) in PANES {
            let mut v = self.layout.is_visible(pane);
            let label = match action {
                Some(a) => format!("{}   {}", pane.title(), self.hotkeys.text(a)),
                None => pane.title().to_string(),
            };
            let changed = ui
                .horizontal(|ui| {
                    match self.pane_glyph(pane) {
                        Some(g) => {
                            tools::glyph_label(ui, g, ui.visuals().text_color());
                        }
                        None => ui.add_space(18.0),
                    }
                    ui.checkbox(&mut v, label).changed()
                })
                .inner;
            if changed {
                // AutoCut's action only reveals; go through the layout directly so unchecking works too
                match action {
                    Some(a) if a != Action::AutoCut => out.push(a),
                    _ => self.toggle_pane(pane),
                }
            }
        }
        ui.separator();
        ui.menu_button("Pop out", |ui| {
            for &pane in Pane::ALL {
                if ui.button(pane.title()).clicked() {
                    ui.close();
                    self.layout.popout(pane);
                    self.layout_dirty = true;
                }
            }
        });
        ui.menu_button("Layout", |ui| {
            ui.menu_button("Preset", |ui| {
                for (name, make) in [
                    ("Default", Layout::default_layout as fn() -> Layout),
                    ("Colorist", Layout::colorist_layout),
                    ("Fast-Cut Assembly", Layout::fastcut_layout),
                ] {
                    if ui.button(name).clicked() {
                        ui.close();
                        self.layout.push_undo(self.layout.to_json());
                        let (undo, redo) =
                            (std::mem::take(&mut self.layout.undo), std::mem::take(&mut self.layout.redo));
                        self.layout = make();
                        (self.layout.undo, self.layout.redo) = (undo, redo);
                        self.layout_dirty = true;
                    }
                }
            });
            if ui.button("Save profile…").clicked() {
                ui.close();
                self.profile_name = Some(String::new());
            }
            let profiles = self.settings.layout_profiles.clone();
            ui.menu_button("Load profile", |ui| {
                if profiles.is_empty() {
                    ui.label("(none)");
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
            if ui.button("Export profile to file…").clicked() {
                ui.close();
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
            if ui.button("Import profile from file…").clicked() {
                ui.close();
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
                            self.settings
                                .layout_profiles
                                .push(crate::settings::LayoutProfile { name, json: l.to_json() });
                            self.settings.save();
                            self.layout = l;
                            self.layout_dirty = true;
                        }
                        None => self.toast("Not a valid layout file"),
                    }
                }
            }
            ui.separator();
            if ui.button("Reset layout").clicked() {
                ui.close();
                self.layout.reset();
                self.layout_dirty = true;
            }
        });
        ui.separator();
        ui.menu_button("Social Guides", |ui| {
            use crate::ui::guides::Guide;
            let mut pick = |ui: &mut egui::Ui, label: &str, v: Option<Guide>| {
                if ui.radio(self.settings.guide == v, label).clicked() {
                    self.settings.guide = v;
                    self.settings.save();
                    ui.close();
                }
            };
            pick(ui, "Off", None);
            ui.separator();
            for g in Guide::ALL {
                pick(ui, g.name(), Some(g));
            }
        });
        let mut movie = self.settings.movie_mode;
        if ui.checkbox(&mut movie, "Movie mode (pre-rendered playback)").changed() {
            out.push(MovieMode);
        }
        self.menu_item(ui, Fullscreen, true, out);
    }

    pub(super) fn menu_bar(&mut self, ui: &mut egui::Ui) -> Vec<Action> {
        use Action::*;
        let mut out = Vec::new();
        let has_sel = !self.selection.is_empty();
        let has_clips = !self.timeline_is_empty();
        let can_overwrite = self.project.source_video.is_some() && has_clips;
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                self.menu_item(ui, NewProject, true, &mut out);
                self.menu_item(ui, OpenFile, true, &mut out);
                self.menu_item(ui, OpenProject, true, &mut out);
                let recents = self.settings.recent_projects.clone();
                let mut forget: Option<Option<String>> = None; // Some(path) = drop one, None = clear all
                ui.menu_button("Open Recent Project", |ui| {
                    ui.set_max_width(420.0);
                    if recents.is_empty() {
                        ui.label("(none)");
                    }
                    for r in &recents {
                        let p = Path::new(r);
                        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| r.clone());
                        let folder = p.parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
                        let b = egui::Button::new(name).shortcut_text(folder).wrap_mode(egui::TextWrapMode::Truncate);
                        let resp = ui.add(b);
                        resp.context_menu(|ui| {
                            if ui.button("Remove from recent").clicked() {
                                ui.close();
                                forget = Some(Some(r.clone()));
                            }
                        });
                        if resp.on_hover_text(r).clicked() {
                            ui.close();
                            if self.confirm_discard() {
                                self.open_project(Path::new(r));
                            }
                        }
                    }
                    if !recents.is_empty() {
                        ui.separator();
                        if ui.button("Clear history").clicked() {
                            ui.close();
                            forget = Some(None);
                        }
                    }
                });
                if let Some(one) = forget {
                    match one {
                        Some(path) => self.settings.recent_projects.retain(|p| *p != path),
                        None => self.settings.recent_projects.clear(),
                    }
                    self.settings.save();
                }
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
                self.menu_item(ui, ExportFrame, has_clips, &mut out);
                if ui.button("Export Style Summary (.md)…").clicked() {
                    ui.close();
                    self.act_export_style();
                }
                ui.separator();
                self.menu_item(ui, ImportTimeline, true, &mut out);
                self.menu_item(ui, ScreenCapture, true, &mut out);
                self.menu_item(ui, Voiceover, true, &mut out);
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
                self.menu_item(ui, CopyClips, has_sel, &mut out);
                self.menu_item(ui, CutClips, has_sel, &mut out);
                self.menu_item(ui, PasteClips, self.clipboard.is_some(), &mut out);
                self.menu_item(ui, PasteInPlace, self.clipboard.is_some(), &mut out);
                self.menu_item(ui, PasteInsert, self.clipboard.is_some(), &mut out);
                self.menu_item(ui, PasteAtTop, self.clipboard.is_some(), &mut out);
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
                self.menu_item(ui, CopyAttributes, has_sel, &mut out);
                self.menu_item(ui, PasteAttributes, has_sel && self.attrs.is_some(), &mut out);
                ui.separator();
                self.menu_item(ui, MarkIn, true, &mut out);
                self.menu_item(ui, MarkOut, true, &mut out);
                self.menu_item(ui, ClearInOut, true, &mut out);
                self.menu_item(ui, TrimToInOut, has_clips, &mut out);
                self.menu_item(ui, RippleDeleteInOut, has_clips, &mut out);
            });
            ui.menu_button("Clip", |ui| {
                self.menu_item(ui, AddText, true, &mut out);
                self.menu_item(ui, AddShape, true, &mut out);
                self.menu_item(ui, AddAdjustment, true, &mut out);
                self.menu_item(ui, AddMask, has_sel, &mut out);
                self.menu_item(ui, AddMarker, true, &mut out);
                self.menu_item(ui, AddSubtitle, true, &mut out);
                self.menu_item(ui, AddTransition, has_sel, &mut out);
                self.menu_item(ui, AddLastTransition, has_sel, &mut out);
                self.menu_item(ui, AddTransitionEnd, has_sel, &mut out);
                self.menu_item(ui, Retime, has_sel, &mut out);
                self.menu_item(ui, FreezeFrame, has_sel, &mut out);
                self.menu_item(ui, NestSequence, has_sel, &mut out);
                self.menu_item(ui, OpenParentSequence, self.project.editing.is_some(), &mut out);
                self.menu_item(ui, SaveTemplate, has_sel, &mut out);
                self.menu_item(ui, ApplyFlow, self.selection.len() == 2, &mut out);
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
                ui.menu_button("Scaling quality", |ui| {
                    for s in Scaler::ALL {
                        if ui.radio(self.project.scaler == s, s.name()).clicked() {
                            ui.close();
                            if self.project.scaler != s {
                                self.push_undo();
                                self.project.scaler = s;
                                self.after_edit();
                            }
                        }
                    }
                });
            });
            ui.menu_button("Playback", |ui| {
                for a in [PlayPause, Stop, StepBack, StepForward, PrevCut, NextCut, GoStart, GoEnd] {
                    self.menu_item(ui, a, true, &mut out);
                }
            });
            ui.menu_button("View", |ui| self.view_menu(ui, &mut out));
            ui.menu_button("Scripts", |ui| {
                // re-reading the folder on every open IS the refresh mechanism
                let scripts = crate::scripting::list();
                for p in &scripts {
                    let label = p.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    if crate::ui::tools::glyph_text_button(ui, crate::ui::tools::Glyph::Terminal, &label).clicked() {
                        self.run_script_path = Some(p.clone());
                        ui.close();
                    }
                }
                if scripts.is_empty() {
                    ui.weak("No scripts yet");
                }
                ui.separator();
                if ui.button("Open Scripts Folder").clicked() {
                    let _ = std::process::Command::new("explorer").arg(crate::scripting::scripts_dir()).spawn();
                    ui.close();
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
                    crate::ui::tools::glyph_label(ui, crate::ui::tools::Glyph::Play, ui.visuals().text_color());
                }
                if let Some(seq) = self.project.editing {
                    let name = self.project.sequence(seq).map(|s| s.name.as_str()).unwrap_or("?").to_string();
                    ui.label(egui::RichText::new(format!("editing: Main > {name}")).weak());
                }
            });
        });
        out
    }
}
