use super::*;

impl App {
    pub(super) fn draw_pane(&mut self, ui: &mut egui::Ui, pane: Pane) {
        if self.failed_panes.contains(&pane) {
            ui.weak(format!("{} is unavailable in this build.", pane.title()));
            return;
        }
        if guarded(|| self.draw_pane_inner(ui, pane)).is_none() {
            self.failed_panes.push(pane);
            self.toast(format!("{} failed to draw — the pane is disabled for this session", pane.title()));
        }
    }

    pub(super) fn draw_pane_inner(&mut self, ui: &mut egui::Ui, pane: Pane) {
        // ---- ws:registries-schema-hooks ----
        // A new Pane gets its own PANE_DRAWERS entry instead of an arm in this match (which every
        // wave-2+ workstream would otherwise share). Nothing is registered yet, so this is a no-op.
        for f in PANE_DRAWERS {
            if f(self, ui, pane) {
                return;
            }
        }
        match pane {
            Pane::Preview => preview_pane::draw(self, ui),
            Pane::Timeline => timeline_pane::draw(self, ui),
            Pane::Library => library_pane::draw(self, ui),
            Pane::Inspector => {
                let changed = {
                    let App {
                        project, selection, sel_transitions, playhead, undo, redo, fonts, palette, settings, ..
                    } = self;
                    let mut push = |p: &Project| push_undo_json(undo, redo, p.to_json());
                    let mut changed = false;
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        changed = inspector::show(
                            ui,
                            project,
                            selection,
                            sel_transitions,
                            *playhead,
                            fonts,
                            palette,
                            settings,
                            &mut push,
                        );
                    });
                    changed
                };
                if changed {
                    self.after_edit();
                }
                if let Some(a) = inspector::take_pending_action() {
                    self.pending_actions.push(a);
                }
            }
            Pane::Effects => {
                let resp = {
                    let App { project, selection, playhead, undo, redo, palette, .. } = self;
                    let mut push = |p: &Project| push_undo_json(undo, redo, p.to_json());
                    // the always-open catalogue is taller than the pane: without this the per-clip
                    // effect stack under it is unreachable
                    egui::ScrollArea::vertical()
                        .show(ui, |ui| effects_ui::show(ui, project, selection, *playhead, palette, &mut push))
                        .inner
                };
                if let Some(i) = resp.mask_for {
                    // ponytail: the viewport's mask tool edits clip.mask, not the effect's own mask —
                    // targeting an effect index is a preview.rs change, not an app one.
                    let shape = self
                        .selection
                        .first()
                        .and_then(|&id| self.project.clip(id))
                        .and_then(|c| c.effects.get(i))
                        .and_then(|fx| fx.mask.as_ref())
                        .map(|m| m.shape)
                        .unwrap_or(MaskShape::Ellipse);
                    self.tools.tool = Tool::Mask(shape);
                    self.layout.reveal(Pane::Tools);
                    self.layout_dirty = true;
                }
                if resp.open_nodes {
                    self.layout.reveal(Pane::Nodes);
                    self.layout_dirty = true;
                }
                if let Some(i) = resp.edit_shader {
                    // same clip the panel showed the stack of
                    let id = self.selection.iter().copied().find(|&id| self.project.clip(id).is_some()).unwrap_or(0);
                    let src = self
                        .project
                        .clip(id)
                        .and_then(|c| c.effects.get(i))
                        .filter(|fx| fx.kind == EffectKind::Shader)
                        .map(|fx| fx.shader.clone());
                    if let Some(src) = src {
                        self.shader_ui.edit(id, i, &src);
                        // a source the renderer already rejected opens with its log, not blank
                        let known = self.gpu.as_ref().and_then(|g| g.shader_error(&src));
                        self.shader_ui.error = known.unwrap_or_default().to_string();
                    }
                }
                if resp.edited {
                    self.after_edit();
                }
            }
            Pane::Transitions => {
                let changed = {
                    let App {
                        project,
                        selection,
                        sel_transitions,
                        playhead,
                        undo,
                        redo,
                        transitions_ui: st,
                        palette,
                        ..
                    } = self;
                    let mut push = |p: &Project| push_undo_json(undo, redo, p.to_json());
                    transitions_ui::show(ui, st, project, selection, sel_transitions, *playhead, palette, &mut push)
                };
                if changed {
                    self.after_edit();
                }
            }
            Pane::Curves => {
                let resp = {
                    let App { project, selection, playhead, undo, redo, curves: st, palette, mixer, .. } = self;
                    let bus = mixer.selected_bus;
                    let mut push = |p: &Project| push_undo_json(undo, redo, p.to_json());
                    curves::show(ui, st, project, selection, bus, playhead, palette, &mut push)
                };
                if resp.seeked {
                    self.player.pause();
                    self.player.seek(self.playhead);
                }
                if resp.edited {
                    self.after_edit();
                }
            }
            Pane::Subtitles => {
                let resp = {
                    let App { project, playhead, selection, undo, redo, subtitles_ui: st, fonts, palette, .. } = self;
                    let mut push = |p: &Project| push_undo_json(undo, redo, p.to_json());
                    subtitles_ui::show(ui, st, project, playhead, selection, fonts, palette, &mut push)
                };
                if resp.seeked {
                    self.player.seek(self.playhead);
                    if resp.play {
                        self.player.play();
                    } else {
                        self.player.pause();
                    }
                }
                if resp.open_folder {
                    self.open_subtitle_folder();
                }
                if resp.edited {
                    self.after_edit();
                }
            }
            Pane::Planner => {
                let resp = {
                    let App { project, undo, redo, planner: st, thumbs, palette, .. } = self;
                    let mut push = |p: &Project| push_undo_json(undo, redo, p.to_json());
                    planner::show(ui, st, project, thumbs, palette, &mut push)
                };
                if !resp.add_to_timeline.is_empty() {
                    self.push_undo();
                    self.insert_at(resp.add_to_timeline, self.playhead, None);
                    self.after_edit();
                }
                if resp.edited {
                    self.after_edit();
                }
            }
            Pane::Moodboard => {
                let resp = {
                    let App { project, undo, redo, moodboard: st, thumbs, palette, .. } = self;
                    let mut push = |p: &Project| push_undo_json(undo, redo, p.to_json());
                    moodboard_ui::show(ui, st, project, thumbs, palette, &mut push)
                };
                if !resp.add_to_timeline.is_empty() {
                    self.push_undo();
                    self.insert_at(resp.add_to_timeline, self.playhead, None);
                    self.after_edit();
                }
                // Import button / dragged-in linked-folder files: import, then board them — same
                // two-steps-when-fresh/one-when-not undo shape as the OS-file-drop path in handle_drops
                if !resp.import_paths.is_empty() {
                    let ids = self.import_files(&resp.import_paths);
                    let snap = self.project.to_json();
                    let mut changed = false;
                    for &id in &ids {
                        changed |= moodboard_ui::moodboard_add(&mut self.project, id);
                    }
                    if changed {
                        push_undo_json(&mut self.undo, &mut self.redo, snap);
                        self.after_edit();
                    }
                }
                if resp.edited {
                    self.after_edit();
                }
            }
            Pane::History => {
                // deleting entries mutates the undo stack directly, not the project — no undo/push_undo
                // of its own (history bookkeeping isn't itself a project edit).
                let App { history, undo, project, .. } = self;
                history_ui::show(ui, history, undo, project);
            }
            Pane::AutoCut => {
                self.autocut_drawing = true;
                let changed = {
                    let App { project, selection, undo, redo, autocut: st, waveforms, palette, .. } = self;
                    let mut push = |p: &Project| push_undo_json(undo, redo, p.to_json());
                    autocut_ui::show(ui, st, project, selection, waveforms, palette, &mut push)
                };
                if changed {
                    self.after_edit();
                }
            }
            Pane::Tracking => {
                self.tracking_drawing = true;
                let backend = self.backend();
                let changed = {
                    let App { project, selection, undo, redo, tracking: st, palette, .. } = self;
                    let mut push = |p: &Project| push_undo_json(undo, redo, p.to_json());
                    tracking_ui::show(ui, st, project, selection, backend, palette, &mut push)
                };
                if changed {
                    self.after_edit();
                }
            }
            Pane::Tools => {
                let was = self.tools.recording;
                let snap_was = self.settings.snap;
                {
                    let App { tools: st, palette, settings, hotkeys, .. } = self;
                    tools::show(ui, st, palette, &mut settings.snap, hotkeys);
                }
                if self.tools.recording != was {
                    self.toggle_draw_recording(self.tools.recording);
                }
                if self.settings.snap != snap_was {
                    self.settings.save();
                }
            }
            Pane::Nodes => {
                let resp = {
                    let App { project, selection, playhead, undo, redo, nodes: st, palette, .. } = self;
                    let mut push = |p: &Project| push_undo_json(undo, redo, p.to_json());
                    nodes::show(ui, st, project, selection, *playhead, palette, &mut push)
                };
                if resp.edited {
                    self.after_edit();
                }
            }
            Pane::Mixer => {
                let changed = {
                    let App { project, selection, playhead, mixer: st, buses, palette, undo, redo, .. } = self;
                    let mut push = |p: &Project| push_undo_json(undo, redo, p.to_json());
                    mixer_ui::show(ui, st, project, selection, buses, *playhead, palette, &mut push)
                };
                if changed {
                    self.after_edit();
                }
            }
            Pane::Presets => {
                // presets_ui.rs is deleted (verified-dead per the architecture's "Pane::Presets fate"
                // decision): this draws the same reuse rows as Library's Recent tab (Effects/Node
                // graphs/Adjustment layers, click-to-apply/place) until wave-2 inspector-gallery replaces
                // it with the Gallery pane. Save-from-selection/rename/delete are gone from this pane;
                // saving is still reachable via the templates.save MCP tool, place/apply via reuse_ui +
                // templates.list/apply (documented interim state, not a bug).
                let mut resp = library::LibraryResponse::default();
                library::reuse_ui(ui, 0, 0, 1.0, &self.project, &self.settings, &self.palette, &mut resp);
                // same 4 blocks Pane::Library already applies for these fields (library_pane.rs), copied
                // verbatim so the two panes' reuse rows behave identically.
                if let Some(kind) = resp.add_effect {
                    let targets: Vec<Id> = self
                        .selection
                        .iter()
                        .copied()
                        .filter(|&id| self.project.clip(id).is_some_and(|c| c.is_visual() && !c.uses_graph()))
                        .collect();
                    if targets.is_empty() {
                        self.toast("Select a clip first");
                    } else {
                        self.push_undo();
                        for id in targets {
                            if let Some(c) = self.project.clip_mut(id) {
                                c.effects.push(Effect::new(kind));
                            }
                        }
                        self.after_edit();
                    }
                }
                if let Some(i) = resp.apply_preset {
                    self.apply_effect_preset(i);
                }
                if let Some(from) = resp.copy_graph {
                    let graph = self.project.clip(from).and_then(|c| c.graph.clone());
                    let targets: Vec<Id> = self
                        .selection
                        .iter()
                        .copied()
                        .filter(|&id| id != from && self.project.clip(id).is_some_and(|c| c.is_visual()))
                        .collect();
                    match graph {
                        Some(g) if !targets.is_empty() => {
                            self.push_undo();
                            for id in targets {
                                if let Some(c) = self.project.clip_mut(id) {
                                    c.graph = Some(g.clone());
                                }
                            }
                            self.after_edit();
                        }
                        _ => self.toast("Select another clip to copy this node graph onto"),
                    }
                }
                for name in resp.place_template {
                    self.place_template(&name, self.playhead);
                }
            }
            Pane::Markers => {
                let resp = {
                    let App { project, selection, playhead, markers: st, palette, undo, redo, .. } = self;
                    let mut push = |p: &Project| push_undo_json(undo, redo, p.to_json());
                    markers_ui::show(ui, st, project, selection, *playhead, palette, &mut push)
                };
                if let Some(t) = resp.seek {
                    self.player.pause();
                    self.seek(t);
                }
                if resp.edited {
                    self.after_edit();
                }
            }
            // ---- ws:registries-schema-hooks ----
            // Pane::Source has no dedicated drawer yet — source-monitor (wave 2) is the first
            // PANE_DRAWERS entry (tried above) and turns this into its real two-up source monitor.
            #[allow(unreachable_patterns)]
            _ => {
                ui.weak(format!("{} isn't wired up yet.", pane.title()));
            }
        }
    }

    /// Hand the saved curve/motion presets (built-ins first) to the curve editor. Called on start and
    /// whenever the lists change — never per frame.
    pub(super) fn refresh_presets(&self) {
        curves::set_available_presets(self.settings.curve_presets.clone());
        let motions: Vec<crate::settings::MotionPreset> = crate::engine::presets::builtin_motions()
            .into_iter()
            .chain(self.settings.motion_presets.iter().cloned())
            .collect();
        curves::set_available_motions(motions);
    }
}
