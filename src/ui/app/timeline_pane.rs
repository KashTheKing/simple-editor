use super::*;
use crate::ui::menu;
use crate::ui::tools::{icon_button, Glyph, Tool};

// The sequence tabs live in the Timeline pane's own tab (ws:pages); view presets and the overview
// strip are in the ruler's right-click ▸ View (ws:timeline-surface).

/// "Name (key)", or just the name while the action is unbound.
fn tip(name: &str, a: Action) -> String {
    match menu::shortcut(a) {
        k if k.is_empty() => name.to_string(),
        k => format!("{name} ({k})"),
    }
}

/// The Timeline's one toolbar row: Select · Blade · Rate stretch, Snap, and zoom on the right. Returns
/// the Actions its buttons ask for; the zoom slider edits `zoom` (px/s) in place.
fn toolbar(
    ui: &mut egui::Ui,
    pal: &crate::theme::Palette,
    tool: Tool,
    snap: (bool, bool),
    zoom: &mut f32,
) -> Vec<Action> {
    let (snap, gaps) = snap;
    let mut out = Vec::new();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (t, a, g, name) in [
            (Tool::Select, Action::ToolSelect, Glyph::Cursor, "Select"),
            (Tool::Cut, Action::ToolCut, Glyph::Razor, "Blade"),
            (Tool::Stretch, Action::ToolStretch, Glyph::Speed, "Rate Stretch"),
        ] {
            if icon_button(ui, pal, ui.id().with(("tl_tool", name)), g, &tip(name, a), tool == t).clicked() {
                out.push(a);
            }
        }
        ui.add_space(8.0);
        if icon_button(ui, pal, ui.id().with("tl_snap"), Glyph::Magnet, &tip("Snapping", Action::ToggleSnap), snap)
            .clicked()
        {
            out.push(Action::ToggleSnap);
        }
        let gtip = tip("Auto Close Gaps", Action::ToggleAutoCloseGaps);
        if icon_button(ui, pal, ui.id().with("tl_gaps"), Glyph::Spacer, &gtip, gaps).clicked() {
            out.push(Action::ToggleAutoCloseGaps);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(4.0);
            if icon_button(
                ui,
                pal,
                ui.id().with("tl_fit"),
                Glyph::Maximize,
                &tip("Zoom to Fit", Action::ZoomFit),
                false,
            )
            .clicked()
            {
                out.push(Action::ZoomFit);
            }
            let zin = tip("Zoom In", Action::ZoomIn);
            if icon_button(ui, pal, ui.id().with("tl_zin"), Glyph::Letter('+'), &zin, false).clicked() {
                out.push(Action::ZoomIn);
            }
            ui.spacing_mut().slider_width = 110.0;
            ui.add(egui::Slider::new(zoom, 0.5..=2000.0).logarithmic(true).show_value(false)).on_hover_text("Zoom");
            let zout = tip("Zoom Out", Action::ZoomOut);
            if icon_button(ui, pal, ui.id().with("tl_zout"), Glyph::Letter('−'), &zout, false).clicked() {
                out.push(Action::ZoomOut);
            }
        });
    });
    out
}

pub(super) fn draw(app: &mut App, ui: &mut egui::Ui) {
    // ---- ws:timeline-surface: one toolbar row ----
    let mut zoom = app.timeline.zoom;
    let acts = toolbar(ui, &app.palette, app.tools.tool, (app.settings.snap, app.settings.auto_close_gaps), &mut zoom);
    app.pending_actions.extend(acts);
    if zoom != app.timeline.zoom {
        // the slider zooms around the playhead while it is on screen, like Ctrl+wheel around the pointer
        let ph = app.timeline.x_at(app.playhead);
        let anchor = app.timeline.lanes_rect.x_range().contains(ph).then_some(ph);
        app.timeline.zoom_by(zoom / app.timeline.zoom, anchor);
    }
    app.timeline.show_kinds = app.settings.track_kinds();
    let autocut_shown = app.pane_drawn(Pane::AutoCut);
    let resp = {
        let App {
            project,
            selection,
            sel_transitions,
            playhead,
            undo,
            redo,
            waveforms,
            thumbs,
            autocut,
            timeline: tl,
            settings,
            player,
            palette,
            tools,
            prerender,
            library,
            ..
        } = app;
        let tool = tools.tool;
        let prerender_bar =
            if settings.movie_mode { guarded(|| prerender.segments()).unwrap_or_default() } else { vec![] };
        // ws:pro-timeline: realtime-safety segments for paint_realtime_bar (already merged by
        // export-deliver's segments_with_heavy -- no re-derivation of Clip::has_effects spans here)
        let realtime_bar = if settings.movie_mode {
            guarded(|| prerender.segments_with_heavy(&*project)).unwrap_or_default()
        } else {
            vec![]
        };
        // ws:pro-timeline: resolved active TimelineView (falls back to "everything on" if the user has
        // cleared Settings.timeline_views down to nothing)
        let fallback_view = crate::settings::TimelineView {
            name: String::new(),
            waves: true,
            thumbs: true,
            keys: true,
            clip_text: true,
            row_h: 64.0,
        };
        let view_idx = tl.view_idx.min(settings.timeline_views.len().saturating_sub(1));
        let view = settings.timeline_views.get(view_idx).unwrap_or(&fallback_view);
        // ws:timeline-trim-gestures: exactly one Library asset selected (the anchor alone counts when
        // the multi-select set is empty, e.g. straight after an import)
        let library_selected = match library.sel_ids.as_slice() {
            [one] => Some(*one),
            [] => library.selected,
            _ => None,
        };
        // labelled like `App::push_undo_labeled` (which needs `&mut self` - the fields are split here)
        let mut push = |p: &Project, label: &'static str| {
            push_undo_json(undo, redo, p.to_json());
            if !label.is_empty() {
                if let Some(e) = undo.last_mut() {
                    e.label = label.to_string();
                }
            }
        };
        timeline::show(
            ui,
            tl,
            timeline::TimelineCtx {
                project,
                selection,
                sel_transitions,
                playhead,
                undo: &mut push,
                waveforms,
                palette,
                snap: settings.snap,
                snap_markers: settings.snap_markers,
                playing: player.is_playing(),
                thumbs: Some(thumbs),
                // only while the Auto-cut pane is on screen: a stale overlay would keep
                // shading the timeline after the pane is hidden or a new project is opened
                keep_ranges: if autocut_shown { &autocut.overlay } else { &[] },
                prerender: &prerender_bar,
                tool,
                library_selected,
                view,
                overview: settings.overview,
                boring_thr: settings.boring_thr,
                realtime: &realtime_bar,
                views: &settings.timeline_views,
            },
        )
    };
    // ws:source-monitor: any press on the timeline (a clip select as much as a seek) means "the
    // timeline is the transport now" - Space/JKL/I/O come back here from the Source monitor
    // (was: drop the library preview on seek).
    if resp.seeked || source_pane::pressed_in(ui) {
        app.source_focus = false;
    }
    if resp.seeked {
        app.player.pause();
        app.player.seek(app.playhead);
    }
    if resp.edited {
        app.after_edit();
    }
    if !resp.dropped_files.is_empty() {
        for (path, t, track) in resp.dropped_files {
            let ids = app.import_files(&[path]);
            let vt = track.filter(|&i| app.project.tracks[i].kind == TrackKind::Video);
            app.place_assets(&ids, t, vt, DropMode::Place);
        }
        app.after_edit();
    }
    for (payload, t, track) in resp.dropped_other {
        let vt = track.filter(|&i| app.project.tracks[i].kind == TrackKind::Video);
        match payload {
            DragPayload::Sequence(id) => {
                let snap = app.project.to_json();
                if app.project.insert_sequence_clip(id, t, vt).is_none() {
                    app.toast("A sequence can't contain itself");
                } else {
                    push_undo_json(&mut app.undo, &mut app.redo, snap);
                    app.after_edit();
                }
            }
            DragPayload::Template(name) => app.place_template(&name, t),
            // dropped onto a clip that can actually take this kind (see effects_ui's own
            // click-to-add gate); a miss says so rather than swallowing the gesture
            DragPayload::Effect(kind) => {
                let hit = track
                    .and_then(|ti| app.project.tracks.get(ti))
                    .and_then(|tr| tr.clips.iter().find(|c| c.contains(t)))
                    .filter(|c| (c.kind == ClipKind::Audio) == kind.applies_to_audio());
                match hit.map(|c| (c.id, c.uses_graph())) {
                    Some((id, false)) => {
                        let snap = app.project.to_json();
                        if let Some(c) = app.project.clip_mut(id) {
                            c.effects.push(Effect::new(kind));
                        }
                        push_undo_json(&mut app.undo, &mut app.redo, snap);
                        app.after_edit();
                        app.toast(format!("{} added", kind.name()));
                    }
                    Some((_, true)) => app.toast("That clip renders from its node graph"),
                    None => app.toast(format!("Drop {} on a clip it applies to", kind.name())),
                }
            }
            // a transition belongs to a cut, so the half of the clip it lands on picks which
            // one: left half = the cut at its start, right half = the cut at its end
            DragPayload::Transition(kind) => {
                let hit = track
                    .and_then(|ti| app.project.tracks.get(ti))
                    .and_then(|tr| tr.clips.iter().find(|c| c.contains(t)))
                    .map(|c| (c.id, transitions_ui::drop_at_end(c, t)));
                match hit {
                    Some((id, at_end)) => {
                        let snap = app.project.to_json();
                        let dur = app.transitions_ui.duration;
                        let st = &mut app.transitions_ui;
                        if transitions_ui::add_transitions(&mut app.project, &[id], st, kind, dur, at_end) > 0 {
                            push_undo_json(&mut app.undo, &mut app.redo, snap);
                            app.after_edit();
                            app.toast(format!("{} ({dur:.2} s)", kind.name()));
                        } else {
                            app.toast("Could not add a transition here");
                        }
                    }
                    None => app.toast("Drop a transition on the clip beside the cut"),
                }
            }
            _ => {}
        }
    }
    if let Some(id) = resp.open_sequence {
        app.enter_sequence(id);
    }
    if let Some((cid, pair)) = resp.replace_container {
        app.replace_container_dialog(cid, pair);
    }
    if resp.import_subtitles {
        import_subtitles(app);
    }
    app.pending_actions.extend(resp.actions);
}

/// Import Subtitles…: the one import path, for the subtitle lane's right-click and the Subtitles pane's ⋯
/// menu alike - pick an .srt/.vtt, parse it, then replace inline (an empty project has nothing to lose)
/// or ask before replacing (Cancel keeps the existing cues and discards the import).
pub(super) fn import_subtitles(app: &mut App) {
    let Some(path) = rfd::FileDialog::new().add_filter("Subtitles", &["srt", "vtt"]).pick_file() else { return };
    let cues = std::fs::read_to_string(&path).map(|t| crate::engine::subtitles::parse(&t)).unwrap_or_default();
    if cues.is_empty() {
        app.toast("No subtitles found in that file");
    } else if app.project.subtitles.is_empty() {
        app.push_undo();
        subtitles_ui::apply_import(&mut app.project, &cues, true);
        app.after_edit();
    } else {
        let n = app.project.subtitles.len();
        confirm::ask(
            "Import subtitles",
            format!("Replace the existing {n} subtitle(s)? (Cancel keeps them and discards this import.)"),
            confirm::ConfirmAction::ReplaceSubtitles(cues),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The toolbar's buttons ask for the same Actions the tool / snap / zoom keys run.
    #[test]
    fn toolbar_buttons_push_their_actions() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let pal = crate::theme::Palette::new(true, egui::Color32::from_rgb(0, 120, 212));
        let mut zoom = 40.0;
        let t = std::cell::Cell::new(0.0);
        let mut frame = |events: Vec<egui::Event>| -> Vec<Action> {
            t.set(t.get() + 0.05);
            let mut out = Vec::new();
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 100.0))),
                time: Some(t.get()),
                events,
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default()
                    .show(ctx, |ui| out = toolbar(ui, &pal, Tool::Select, (true, false), &mut zoom));
            });
            out
        };
        let mut click = |pos: egui::Pos2| -> Vec<Action> {
            let btn = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            t.set(t.get() + 1.0); // clicks well apart, so none merge into a double-click
            frame(vec![egui::Event::PointerMoved(pos)]);
            frame(vec![btn(true)]);
            frame(vec![btn(false)])
        };
        // 24 x 22 icon buttons, 2 pt apart, from the panel's 8 pt margin: Select, Blade, Rate Stretch
        assert_eq!(click(egui::pos2(8.0 + 26.0 + 12.0, 19.0)), vec![Action::ToolCut]);
        assert_eq!(click(egui::pos2(8.0 + 52.0 + 12.0, 19.0)), vec![Action::ToolStretch]);
        // then an 8 pt gap and the magnet
        assert_eq!(click(egui::pos2(8.0 + 78.0 + 8.0 + 12.0, 19.0)), vec![Action::ToggleSnap]);
        assert_eq!(click(egui::pos2(8.0 + 104.0 + 8.0 + 12.0, 19.0)), vec![Action::ToggleAutoCloseGaps]);
        // right-aligned from the 792 pt edge, 4 pt in: Zoom to Fit
        assert_eq!(click(egui::pos2(792.0 - 4.0 - 12.0, 19.0)), vec![Action::ZoomFit]);
    }
}
