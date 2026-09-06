use super::tools_helpers::*;
use super::*;

impl App {
    pub(super) fn act(&mut self, a: Action) {
        use Action::*;
        // an export writes the timeline: block only saving/exporting, keep editing usable
        if self.export.is_some() && matches!(a, Save | SaveProjectAs | ExportVideo | ExportLossless) {
            self.toast("An export is running — try again when it finishes");
            return;
        }
        match a {
            NewProject => {
                if self.confirm_discard() {
                    self.set_project(Project::new(), None);
                }
            }
            ToolSelect | ToolText | ToolDraw | ToolMask | ToolMarker | ToolCut | ToolStretch | ToolZoom
            | ToolSpacer => {
                // normally already consumed by tools::handle_hotkeys before this table is polled; this
                // arm only fires for a caller that dispatches the action directly (scripting/MCP).
                if let Some(t) = tools::tool_for_action(a, self.tools.tool) {
                    self.tools.tool = t;
                    self.layout.reveal(Pane::Tools);
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
                if let Some(entry) = self.undo.pop() {
                    if entry.json == LAYOUT_STEP {
                        self.layout.undo();
                        self.layout_dirty = true;
                        self.redo.push(entry);
                    } else {
                        let redo_at = now_secs();
                        self.redo.push(UndoEntry {
                            json: self.project.to_json(),
                            label: entry.label.clone(),
                            category: HistoryCategory::Editing,
                            at: redo_at,
                        });
                        if let Ok(p) = Project::from_json(&entry.json) {
                            self.project = p;
                            self.after_edit();
                        }
                    }
                }
            }
            Redo => {
                if let Some(entry) = self.redo.pop() {
                    if entry.json == LAYOUT_STEP {
                        self.layout.redo();
                        self.layout_dirty = true;
                        self.undo.push(entry);
                    } else {
                        let undo_at = now_secs();
                        self.undo.push(UndoEntry {
                            json: self.project.to_json(),
                            label: entry.label.clone(),
                            category: HistoryCategory::Editing,
                            at: undo_at,
                        });
                        if let Ok(p) = Project::from_json(&entry.json) {
                            self.project = p;
                            self.after_edit();
                        }
                    }
                }
            }
            PlayPause => {
                // a library asset preview owns the Preview pane while it's open, so space controls
                // its player instead of the timeline's — otherwise pressing play while looking at a
                // library asset would silently start the timeline playing behind it
                if let Some(lp) = self.lib_preview.as_mut() {
                    lp.player.toggle();
                } else if self.buffer_stall {
                    self.buffer_stall = false; // buffering held the clock: space means "stop waiting"
                } else {
                    if !self.player.is_playing() && self.playhead >= self.project.duration() - 1e-6 {
                        self.seek(0.0);
                    }
                    self.player.toggle();
                }
            }
            Stop => {
                self.buffer_stall = false;
                self.player.pause();
            }
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
                let snap = self.project.to_json();
                let mut did = !self.project.split_at(self.playhead, only.as_deref()).is_empty();
                // cues selected on the timeline's subtitle lane split too, like clips
                for id in self.timeline.sub_sel.clone() {
                    did |= self.project.split_cue(id, self.playhead).is_some();
                }
                if did {
                    push_undo_json(&mut self.undo, &mut self.redo, snap);
                    self.after_edit();
                }
            }
            Delete | RippleDelete => {
                let ids = self.project.expand_links(&self.selection);
                let trs = std::mem::take(&mut self.sel_transitions);
                if !ids.is_empty() || !trs.is_empty() {
                    self.push_undo();
                    for tid in trs {
                        self.project.remove_transition(tid);
                    }
                    self.project.delete_clips(&ids, a == RippleDelete);
                    self.selection.clear();
                    self.after_edit();
                }
            }
            SelectAll => self.selection = self.project.all_clips().map(|(_, c)| c.id).collect(),
            Deselect => self.selection.clear(),
            // in/out marks are saved with the project: undoable, and they make it dirty like any other edit
            MarkIn => {
                self.push_undo();
                self.project.in_point = Some(self.playhead);
                if let Some(o) = self.project.out_point {
                    if o <= self.playhead {
                        self.project.out_point = None;
                    }
                }
                self.after_edit();
            }
            MarkOut => {
                self.push_undo();
                self.project.out_point = Some(self.playhead);
                if let Some(i) = self.project.in_point {
                    if i >= self.playhead {
                        self.project.in_point = None;
                    }
                }
                self.after_edit();
            }
            ClearInOut => {
                if self.project.in_point.is_some() || self.project.out_point.is_some() {
                    self.push_undo();
                    self.project.in_point = None;
                    self.project.out_point = None;
                    self.after_edit();
                }
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
                let ids = self.project.expand_links(&self.selection);
                if !ids.is_empty() {
                    let dt = if a == NudgeLeft { -self.project.frame_dur() } else { self.project.frame_dur() };
                    let snap = self.project.to_json();
                    if self.project.move_clips(&ids, dt, 0, None) {
                        push_undo_json(&mut self.undo, &mut self.redo, snap);
                        self.after_edit();
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
            ToggleLibrary => self.toggle_pane(Pane::Library),
            ToggleMarkers => self.toggle_pane(Pane::Markers),
            ToggleNodes => self.toggle_pane(Pane::Nodes),
            ToggleMixer => self.toggle_pane(Pane::Mixer),
            ToggleTools => self.toggle_pane(Pane::Tools),
            AddLastTransition => {
                // the panel state IS the memory: every apply path records into it (transitions_ui)
                let (kind, dur) = (self.transitions_ui.kind(), self.transitions_ui.duration);
                if self.selection.is_empty() {
                    self.toast("Select a clip next to the cut first");
                } else {
                    let snap = self.project.to_json();
                    let ids = self.selection.clone();
                    let st = &mut self.transitions_ui;
                    if transitions_ui::add_transitions(&mut self.project, &ids, st, kind, dur, false) > 0 {
                        push_undo_json(&mut self.undo, &mut self.redo, snap);
                        self.after_edit();
                        self.toast(format!("{} ({dur:.2} s)", kind.name()));
                    } else {
                        self.toast("Could not add a transition here");
                    }
                }
            }
            CopyAttributes => match self.selection.first().and_then(|&id| self.project.copy_attributes(id)) {
                Some(c) => {
                    let name = c.name.clone();
                    self.attrs = Some(c);
                    self.toast(format!("Copied attributes from '{name}'"));
                }
                None => self.toast("Select a clip to copy attributes from"),
            },
            PasteAttributes => {
                if self.attrs.is_none() {
                    self.toast("Copy attributes from a clip first (Ctrl+Alt+C)");
                } else if self.selection.is_empty() {
                    self.toast("Select the clips to paste onto");
                } else {
                    self.paste_ui.open = true;
                }
            }
            CopyClips | CutClips => {
                let ids = self.project.expand_links(&self.selection);
                if ids.is_empty() {
                    self.toast("Select the clips to copy first");
                } else {
                    let t = crate::engine::presets::capture_template("clipboard", &self.project, &ids);
                    // the JSON is what makes Ctrl+V fire at all (see App::os_clipboard); it is also
                    // readable, so a copy can be pasted into another instance by hand
                    self.os_clipboard = Some(t.json.clone());
                    self.clipboard = Some(t);
                    if a == CutClips {
                        self.push_undo();
                        self.project.delete_clips(&ids, false);
                        self.selection.clear();
                        self.after_edit();
                    }
                }
            }
            PasteClips | PasteInPlace | PasteInsert | PasteAtTop => {
                match self.clipboard.as_ref().and_then(crate::engine::presets::decode_template) {
                    Some((clips, assets)) => {
                        // Paste In Place ignores the clicked row: place_clips takes the first track with room
                        let target = (a == PasteClips).then_some(self.timeline.last_track).flatten();
                        let snap = self.project.to_json();
                        if a == PasteInsert {
                            // ripple: everything at or after the playhead slides right by the paste's span
                            let span = clips.iter().map(|c| c.start + c.duration).fold(0.0_f64, f64::max);
                            self.project.ripple_open(self.playhead, span);
                        }
                        if a == PasteAtTop {
                            let kind = clips
                                .iter()
                                .find(|c| c.kind != ClipKind::Audio)
                                .map_or(TrackKind::Audio, |_| TrackKind::Video);
                            self.project.add_track(kind);
                        }
                        let ids = timeline::paste_clips(&mut self.project, clips, assets, self.playhead, target);
                        if ids.is_empty() {
                            self.toast("Nothing could be pasted here");
                        } else {
                            push_undo_json(&mut self.undo, &mut self.redo, snap);
                            self.selection = ids;
                            self.after_edit();
                        }
                    }
                    None => self.toast("Nothing copied yet — Ctrl+C copies the selected clips"),
                }
            }
            AddMarker => {
                let t = self.playhead;
                // per the ask, the hotkey attaches the marker to the selected clip when the playhead
                // is over one — otherwise it stays a plain timeline marker (same as the panel buttons)
                let on_clip = self
                    .selection
                    .iter()
                    .find_map(|&id| self.project.clip(id).filter(|c| t >= c.start && t <= c.end()).map(|c| c.id));
                self.push_undo();
                let name = format!("Marker at {}", crate::ui::timecode(t, self.project.fps));
                let id = match on_clip {
                    Some(cid) => {
                        let local = self.project.clip(cid).map(|c| (t - c.start).clamp(0.0, c.duration)).unwrap_or(0.0);
                        self.project.add_clip_marker(cid, local, name)
                    }
                    None => Some(self.project.add_marker(t, name)),
                };
                if let Some(id) = id {
                    self.markers.selected = vec![id];
                }
                self.layout.reveal(Pane::Markers);
                self.layout_dirty = true;
                self.after_edit();
            }
            AddShape => {
                let kind = match self.tools.tool {
                    Tool::Shape(k) => k,
                    Tool::Draw => ShapeKind::Draw,
                    _ => ShapeKind::Rect,
                };
                self.add_shape(kind, None);
            }
            AddAdjustment => {
                self.push_undo();
                let id = self.project.add_adjustment_clip(self.playhead, 5.0);
                self.selection = vec![id];
                self.after_edit();
            }
            AddMask => {
                let shape = match self.tools.tool {
                    Tool::Mask(s) => s,
                    _ => MaskShape::Ellipse,
                };
                let Some(&id) = self.selection.first() else {
                    self.toast("Select a clip (or an effect on it) to mask");
                    return;
                };
                // snapshot first, commit on success: popping the undo entry afterwards would leave the
                // redo stack cleared for an edit that never happened
                let snap = self.project.to_json();
                if add_mask(&mut self.project, id, shape) {
                    push_undo_json(&mut self.undo, &mut self.redo, snap);
                    self.tools.tool = Tool::Mask(shape);
                    self.layout.reveal(Pane::Tools);
                    self.layout_dirty = true;
                    self.after_edit();
                } else if self.project.clip(id).is_some_and(|c| !c.is_visual()) {
                    self.toast("A mask shapes pixels — an audio clip has none");
                } else {
                    self.toast("That clip already has a mask");
                }
            }
            ExportFrame => {
                if self.timeline_is_empty() {
                    self.toast("Timeline is empty — nothing to export");
                } else if !self.ffmpeg_missing() {
                    self.frame_ui.open = true;
                }
            }
            ScreenCapture => self.capture_ui.screen_open = !self.capture_ui.screen_open,
            Voiceover => self.capture_ui.voice_open = !self.capture_ui.voice_open,
            ImportTimeline => self.act_import_timeline(),
            MovieMode => {
                self.settings.movie_mode = !self.settings.movie_mode;
                self.settings.save();
                if self.settings.movie_mode {
                    self.request_prerender();
                } else {
                    guarded(|| self.prerender.clear());
                }
                self.toast(if self.settings.movie_mode { "Movie mode on" } else { "Movie mode off" });
            }
            ToggleInspector => self.toggle_pane(Pane::Inspector),
            ToggleEffects => self.toggle_pane(Pane::Effects),
            ToggleTransitions => self.toggle_pane(Pane::Transitions),
            ToggleCurves => self.toggle_pane(Pane::Curves),
            ToggleSubtitles => self.toggle_pane(Pane::Subtitles),
            TogglePlanner => self.toggle_pane(Pane::Planner),
            AutoCut => {
                self.layout.reveal(Pane::AutoCut);
                self.layout_dirty = true;
            }
            Retime => self.retime.open = !self.retime.open,
            FreezeFrame => {
                if self.selection.is_empty() {
                    self.toast("Select a clip to freeze");
                } else {
                    let snap = self.project.to_json();
                    let ids = self.selection.clone();
                    let frozen = self.project.freeze_at(self.playhead, &ids);
                    if frozen.is_empty() {
                        self.toast("Nothing to freeze");
                    } else {
                        push_undo_json(&mut self.undo, &mut self.redo, snap);
                        self.selection = frozen;
                        self.after_edit();
                    }
                }
            }
            Fullscreen => {
                // the caller sends ViewportCommand::Fullscreen (needs the ctx)
                self.fullscreen = !self.fullscreen;
                self.player.seek(self.playhead); // re-render at the new canvas size
            }
            AddTransition | AddTransitionEnd => {
                let at_end = a == AddTransitionEnd;
                if self.selection.is_empty() {
                    self.toast("Select a clip next to the cut first");
                } else {
                    let snap = self.project.to_json();
                    let ids = self.selection.clone();
                    let (kind, dur) = (TransitionKind::CrossFade, 1.0);
                    let st = &mut self.transitions_ui;
                    let added = transitions_ui::add_transitions(&mut self.project, &ids, st, kind, dur, at_end);
                    if added > 0 {
                        push_undo_json(&mut self.undo, &mut self.redo, snap);
                        self.after_edit();
                    } else {
                        self.toast("Could not add a transition here");
                    }
                }
            }
            AddSubtitle => {
                self.push_undo();
                let id = self.project.add_cue(self.playhead, self.playhead + 2.0, "Subtitle");
                self.subtitles_ui.selected = Some(id);
                self.layout.reveal(Pane::Subtitles);
                self.layout_dirty = true;
                self.after_edit();
            }
            NestSequence => {
                if self.selection.is_empty() {
                    self.toast("Select the clips to nest");
                } else {
                    let snap = self.project.to_json();
                    let name = format!("Sequence {}", self.project.sequences.len() + 1);
                    let ids = self.selection.clone();
                    match self.project.nest_selection(&ids, name.clone()) {
                        Some(_) => {
                            push_undo_json(&mut self.undo, &mut self.redo, snap);
                            self.selection.clear();
                            self.after_edit();
                            self.toast(format!("Nested into '{name}' — double-click / open it to edit inside"));
                        }
                        None => self.toast("Nothing to nest"),
                    }
                }
            }
            OpenParentSequence => {
                if self.project.editing.is_some() {
                    self.project.close_sequence();
                    self.sequence_view_changed(self.playhead); // clamps into the parent timeline
                }
            }
            SaveTemplate => {
                if self.selection.is_empty() {
                    self.toast("Select the clips to save as a template");
                } else {
                    self.template_name = Some(String::new());
                }
            }
            ApplyFlow => {
                let mut sel: Vec<&Clip> = self.selection.iter().filter_map(|&id| self.project.clip(id)).collect();
                sel.sort_by(|a, b| a.start.total_cmp(&b.start));
                if sel.len() != 2 {
                    self.toast("Flow needs exactly two selected clips");
                } else {
                    let (a_id, b_id) = (sel[0].id, sel[1].id);
                    let snap = self.project.to_json();
                    if self.project.flow_clips(a_id, b_id) {
                        push_undo_json(&mut self.undo, &mut self.redo, snap);
                        self.after_edit();
                    } else {
                        self.toast("Flow needs two abutting clips (no gap at the cut)");
                    }
                }
            }
            AddContainer => {
                let snap = self.project.to_json();
                let (vid, aid) = self.project.add_container_clip(self.playhead, 5.0);
                push_undo_json(&mut self.undo, &mut self.redo, snap);
                self.selection = vec![vid, aid];
                self.after_edit();
                self.toast("Container clip added");
            }
            ReplaceContainerMedia => {
                if let Some(&id) = self.selection.first() {
                    self.replace_container_dialog(id, false);
                } else {
                    self.toast("Select a container clip to replace");
                }
            }
            MakeContainer => {
                if !self.selection.is_empty() {
                    let snap = self.project.to_json();
                    self.project.make_container(&self.selection);
                    push_undo_json(&mut self.undo, &mut self.redo, snap);
                    self.after_edit();
                    self.toast("Converted to container");
                } else {
                    self.toast("Select clips to convert to container");
                }
            }
            UnmakeContainer => {
                if !self.selection.is_empty() {
                    let snap = self.project.to_json();
                    self.project.unmake_container(&self.selection);
                    push_undo_json(&mut self.undo, &mut self.redo, snap);
                    self.after_edit();
                    self.toast("Container removed");
                }
            }
        }
    }

    pub(super) fn toggle_pane(&mut self, p: Pane) {
        self.layout.toggle(p);
        self.layout_dirty = true;
    }

    pub(super) fn add_shape(&mut self, kind: ShapeKind, place: Option<(f32, f32, f32, f32)>) -> Id {
        self.push_undo();
        let id = self.project.add_shape_clip(kind, self.playhead, 5.0);
        let App { project, tools, .. } = self;
        if let Some(c) = project.clip_mut(id) {
            if let Some((cx, cy, _, _)) = place {
                c.x.value = cx as f64;
                c.y.value = cy as f64;
            }
            if let Some(s) = c.shape.as_mut() {
                // shared with the preview's live drag (tools::shape_style_from_tools) so what was
                // previewed is exactly what lands on the clip
                let styled = tools::shape_style_from_tools(tools, kind);
                s.fill = styled.fill;
                s.stroke = styled.stroke;
                s.stroke_width = styled.stroke_width;
                s.sides = styled.sides;
                s.corner = styled.corner;
                s.draw_rate = styled.draw_rate;
                s.page = styled.page;
                if let Some((_, _, w, h)) = place {
                    s.w.value = w as f64;
                    s.h.value = h as f64;
                }
            }
        }
        self.selection = vec![id];
        self.after_edit();
        id
    }

    /// Text tool drag-to-add: places a new text clip's centre where the user dragged on the viewport
    /// (`PreviewResponse::new_text`). The drag's half-extents have no matching `TextStyle` field (text
    /// boxes size to their content, not a fixed rect) so only the centre is used.
    pub(super) fn add_text(&mut self, cx: f32, cy: f32) -> Id {
        self.push_undo();
        let id = self.project.add_text_clip(self.playhead, 5.0);
        if let Some(c) = self.project.clip_mut(id) {
            c.x.value = cx as f64;
            c.y.value = cy as f64;
        }
        self.selection = vec![id];
        self.after_edit();
        id
    }

    /// The Draw tool's play/record button: the video plays and every stroke joins one drawing until the
    /// take is stopped (button, video stopped, or the tool put away). A voiceover running at the same
    /// time owns the transport, so it is left playing and its take is not cut short.
    pub(super) fn toggle_draw_recording(&mut self, on: bool) {
        if on {
            let id = self.add_shape(ShapeKind::Draw, None);
            self.draw_rec = Some((id, self.playhead));
            if !self.player.is_playing() {
                self.pending_actions.push(Action::PlayPause);
            }
            return;
        }
        let Some((id, _)) = self.draw_rec.take() else { return };
        // the clip was placed at record-press time with a placeholder length; pin its real bounds to
        // exactly the first and last point drawn (`add_stroke` already times every point from that press)
        // instead of the press-to-stop span, which pads the clip with dead time on either side.
        let bounds = self.project.clip(id).and_then(|c| c.shape.as_ref()).map(|s| {
            let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
            for st in &s.strokes {
                for p in &st.points {
                    lo = lo.min(p.2);
                    hi = hi.max(p.2);
                }
            }
            (lo, hi)
        });
        match bounds.filter(|(lo, _)| lo.is_finite()) {
            Some((lo, hi)) => {
                if let Some(c) = self.project.clip_mut(id) {
                    c.start += lo as f64;
                    c.duration = ((hi - lo).max(0.0) as f64).max(MIN_CLIP);
                    if let Some(s) = c.shape.as_mut() {
                        for st in &mut s.strokes {
                            for p in &mut st.points {
                                p.2 -= lo;
                            }
                        }
                    }
                }
            }
            None => self.project.delete_clips(&[id], false), // nothing was drawn: leave no stub behind
        }
        if self.voice_rec.is_none() {
            self.player.pause();
        }
        self.after_edit();
    }

    /// A stroke drawn in the viewport: append it to the take (or the selected drawing), or start a new one.
    pub(super) fn add_stroke(&mut self, mut stroke: crate::model::Stroke) {
        stroke.color = self.tools.brush;
        stroke.width = self.tools.brush_width.max(0.5);
        // during a take the stroke is timed from where the playhead was when the pen went down: its own
        // points are timed from the press, so the last one dates the whole stroke
        let rec = self.draw_rec.filter(|&(id, _)| self.project.clip(id).is_some());
        if let Some((_, at)) = rec {
            let off = ((self.playhead - at) - stroke.points.last().map_or(0.0, |p| p.2 as f64)).max(0.0) as f32;
            for p in &mut stroke.points {
                p.2 += off;
            }
        }
        let onto = rec.map(|(id, _)| id).or_else(|| {
            self.selection.first().copied().filter(|&id| {
                self.project.clip(id).and_then(|c| c.shape.as_ref()).is_some_and(|s| s.kind == ShapeKind::Draw)
            })
        });
        let id = match onto {
            Some(id) => {
                self.push_undo();
                id
            }
            None => self.add_shape(ShapeKind::Draw, None),
        };
        if let Some(c) = self.project.clip_mut(id) {
            if let Some(s) = c.shape.as_mut() {
                s.strokes.push(stroke);
            }
            if rec.is_some() {
                // still recording: keep the preview at least as long as what's drawn so far; the exact
                // start/end are pinned once the take stops (toggle_draw_recording)
                let len = c.shape.as_ref().map_or(0.0, |s| s.draw_duration());
                c.duration = c.duration.max(len);
            } else {
                // not part of a running take (a plain drag): pin the clip to exactly what's drawn, instead
                // of leaving it at add_shape's placeholder duration if the stroke was shorter than that
                let bounds = c.shape.as_mut().map(|s| {
                    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
                    for st in &s.strokes {
                        for p in &st.points {
                            lo = lo.min(p.2);
                            hi = hi.max(p.2);
                        }
                    }
                    if lo > 0.0 {
                        for st in &mut s.strokes {
                            for p in &mut st.points {
                                p.2 -= lo;
                            }
                        }
                        hi -= lo;
                    }
                    (lo.max(0.0), hi.max(0.0))
                });
                if let Some((lo, hi)) = bounds {
                    c.start += lo as f64;
                    c.duration = (hi as f64).max(MIN_CLIP);
                }
            }
        }
        self.selection = vec![id];
        self.after_edit();
    }
    pub(super) fn poll_panels(&mut self) {
        // saved presets from the effects / curves panels
        if let Some(m) = effects_ui::take_pending_motion() {
            self.settings.motion_presets.retain(|p| p.name != m.name);
            self.settings.motion_presets.push(m);
            self.settings.save();
            self.toast("Motion preset saved");
        }
        if let Some(m) = curves::take_pending_motion_preset() {
            self.settings.motion_presets.retain(|p| p.name != m.name);
            self.settings.motion_presets.push(m);
            self.settings.save();
            self.refresh_presets();
            self.toast("Motion preset saved");
        }
        if let Some(c) = curves::take_pending_curve_preset() {
            self.settings.curve_presets.retain(|p| p.name != c.name);
            self.settings.curve_presets.push(c);
            self.settings.save();
            // hand them over only when they change (this used to clone the whole list every frame)
            self.refresh_presets();
            self.toast("Curve preset saved");
        }
        // font import from the inspector
        if let Some(path) = inspector::take_pending_font_import() {
            if !self.settings.user_fonts.iter().any(|f| f.eq_ignore_ascii_case(&path)) {
                self.settings.user_fonts.push(path);
                self.settings.save();
            }
        }
        if self.loaded_fonts != self.settings.user_fonts.len() {
            let fonts = self.settings.user_fonts.clone();
            if let Ok(mut t) = self.text.try_lock() {
                t.load_user_fonts(&fonts);
                self.fonts = t.families().to_vec();
                self.loaded_fonts = fonts.len();
            }
        }
        if let Some(id) = inspector::take_open_sequence() {
            self.enter_sequence(id);
        }
        // "Edit in viewport" / "Open node editor" from the inspector
        if let Some(id) = inspector::take_edit_mask() {
            let shape =
                self.project.clip(id).and_then(|c| c.mask.as_ref()).map(|m| m.shape).unwrap_or(MaskShape::Ellipse);
            self.selection = vec![id];
            self.tools.tool = Tool::Mask(shape);
            self.layout.reveal(Pane::Tools);
            self.layout_dirty = true;
        }
        if let Some(id) = inspector::take_open_nodes() {
            self.selection = vec![id];
            self.layout.reveal(Pane::Nodes);
            self.layout_dirty = true;
        }
        if let Some(id) = inspector::take_unlink_nodes() {
            self.push_undo();
            match self.project.unlink_graph(id) {
                Ok(n) => {
                    self.after_edit();
                    self.toast(format!("Unlinked — {n} effect layer{}", if n == 1 { "" } else { "s" }));
                }
                // nothing changed, so the snapshot above would be a no-op undo entry
                Err(e) => {
                    self.undo.pop();
                    self.toast(format!("Can't unlink this graph: {e}"));
                }
            }
        }
        // URL downloads: import the finished file, report failures
        let mut fetched: Vec<(Option<PathBuf>, Option<String>, bool)> = Vec::new();
        self.downloads.retain(|d| {
            if d.progress.is_done() {
                fetched.push((d.path(), d.progress.error(), d.progress.is_cancelled()));
                false
            } else {
                true
            }
        });
        for (path, err, cancelled) in fetched {
            match (path, err) {
                (Some(p), None) => {
                    let ids = self.import_files(&[p.clone()]);
                    self.library.selected = ids.last().copied();
                    self.library.tab = 0;
                    self.toast(format!("Imported {}", p.file_name().unwrap_or_default().to_string_lossy()));
                }
                // cancelling is not a failure — matches finish_export's wording
                (_, Some(_)) if cancelled => self.toast("Download cancelled"),
                (_, Some(e)) => self.toast(format!("Download failed: {e}")),
                (None, None) => self.toast("Download finished but produced no file"),
            }
        }

        // library conversions
        let mut done: Vec<(PathBuf, Option<String>)> = Vec::new();
        self.convert_jobs.retain(|(prog, out)| {
            if prog.is_done() {
                done.push((out.clone(), prog.error()));
                false
            } else {
                true
            }
        });
        for (out, err) in done {
            match err {
                Some(e) => self.toast(format!("Convert failed: {e}")),
                None => {
                    let ids = self.import_files(&[out.clone()]);
                    self.library.selected = ids.last().copied();
                    self.toast_with_folder(
                        format!("Converted → {}", out.file_name().unwrap_or_default().to_string_lossy()),
                        out,
                    );
                }
            }
        }
    }

    /// Open a nested timeline for editing (keeps the player/preview in sync).
    pub(super) fn enter_sequence(&mut self, id: Id) {
        if self.project.editing == Some(id) {
            return;
        }
        if self.project.open_sequence(id) {
            self.sequence_view_changed(0.0);
        } else {
            self.toast("That sequence no longer exists");
        }
    }

    /// Navigating in/out of a nested sequence is a view change, not an edit: re-sync the player,
    /// but no undo step and no dirty flag (that made "just looking" prompt to save on exit).
    pub(super) fn sequence_view_changed(&mut self, t: f64) {
        self.selection.clear();
        self.player.set_project(&self.project);
        self.seek(t);
    }

    /// "Save from selection" in the Presets pane: one clip's node graph or effect stack becomes an
    /// effect preset, anything else (adjustment layers, several clips) becomes a clip template — that is
    /// the only flavour that can be *placed* rather than applied.
    pub(super) fn save_preset(&mut self, name: &str) {
        let fx = match self.selection.as_slice() {
            [id] => self
                .project
                .clip(*id)
                .filter(|c| c.kind != ClipKind::Adjustment && (c.graph.is_some() || !c.effects.is_empty()))
                .map(|c| crate::engine::presets::capture_effects(name, c)),
            _ => None,
        };
        if let Some(p) = fx {
            self.settings.effect_presets.retain(|x| x.name != name);
            self.settings.effect_presets.push(p);
        } else if self.selection.is_empty() {
            return self.toast("Select a clip first");
        } else {
            let t = crate::engine::presets::capture_template(name, &self.project, &self.selection);
            self.settings.templates.retain(|x| x.name != name);
            self.settings.templates.push(t);
        }
        self.settings.save();
        self.toast(format!("Saved \"{name}\""));
    }

    /// Apply a saved effect chain / node graph to every selected clip (one undo entry).
    pub(super) fn apply_effect_preset(&mut self, i: usize) {
        let Some(p) = self.settings.effect_presets.get(i).cloned() else { return };
        if self.selection.is_empty() {
            return self.toast("Select a clip first");
        }
        self.push_undo();
        let mut n = 0;
        for id in self.selection.clone() {
            n += crate::engine::presets::apply_effects(&p, &mut self.project, id) as usize;
        }
        if n == 0 {
            return self.toast("That preset is corrupted");
        }
        self.after_edit();
    }

    pub(super) fn place_template(&mut self, name: &str, t: f64) {
        let Some(tpl) = self.settings.templates.iter().find(|x| x.name == name).cloned() else {
            self.toast(format!("Template '{name}' not found"));
            return;
        };
        match crate::engine::presets::decode_template(&tpl) {
            Some((clips, assets)) => {
                self.push_undo();
                let ids = self.project.place_clips(clips, assets, t);
                self.selection = ids;
                self.after_edit();
            }
            None => self.toast(format!("Template '{name}' is corrupted")),
        }
    }
}
