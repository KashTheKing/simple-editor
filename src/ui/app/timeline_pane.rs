use super::*;

// ws:pro-timeline: the in-widget sequence tab strip (timeline::show(), painted above its own ruler)
// now carries the Main/<sequence name> breadcrumb this file used to draw externally - removed here
// rather than kept alongside a near-duplicate (delete before add; see the PR body).

pub(super) fn draw(app: &mut App, ui: &mut egui::Ui) {
    // ---- ws:pro-timeline: view-preset combo + overview toggle ----
    ui.horizontal(|ui| {
        let views_len = app.settings.timeline_views.len();
        if views_len > 0 {
            let idx = app.timeline.view_idx.min(views_len - 1);
            egui::ComboBox::from_id_salt("tl_view_preset")
                .selected_text(app.settings.timeline_views[idx].name.clone())
                .show_ui(ui, |ui| {
                    for i in 0..views_len {
                        let name = app.settings.timeline_views[i].name.clone();
                        ui.selectable_value(&mut app.timeline.view_idx, i, name);
                    }
                });
        }
        let on = app.settings.overview;
        let label = if on { "Overview: On" } else { "Overview" };
        if crate::ui::tools::glyph_text_button(ui, crate::ui::tools::Glyph::Rows, label).clicked() {
            app.settings.overview = !on;
        }
    });
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
            autocut_shown,
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
                keep_ranges: if *autocut_shown { &autocut.overlay } else { &[] },
                prerender: &prerender_bar,
                tool,
                library_selected,
                view,
                overview: settings.overview,
                boring_thr: settings.boring_thr,
                realtime: &realtime_bar,
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
    app.pending_actions.extend(resp.actions);
}
