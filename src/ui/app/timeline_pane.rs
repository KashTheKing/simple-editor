use super::*;

pub(super) fn draw(app: &mut App, ui: &mut egui::Ui) {
    // sequence breadcrumb: a thin strip above the timeline while editing a nested sequence
    if let Some(seq) = app.project.editing {
        let name = app.project.sequence(seq).map(|s| s.name.clone()).unwrap_or_default();
        ui.horizontal(|ui| {
            let back = crate::ui::tools::glyph_text_button(
                ui,
                crate::ui::tools::Glyph::Tri(crate::ui::tools::Dir::Left),
                "Back",
            );
            if back.on_hover_text("Back to the main timeline (Alt+Up)").clicked() {
                app.pending_actions.push(Action::OpenParentSequence);
            }
            ui.label(format!("Main > {name}"));
        });
    }
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
            ..
        } = app;
        let tool = tools.tool;
        let prerender_bar =
            if settings.movie_mode { guarded(|| prerender.segments()).unwrap_or_default() } else { vec![] };
        let mut push = |p: &Project| push_undo_json(undo, redo, p.to_json());
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
            },
        )
    };
    if resp.seeked {
        // clicking the timeline means "play the timeline" — drop any library asset
        // preview so it stops owning the Preview pane and the transport
        app.lib_preview = None;
        app.lib_preview_tex = None;
        app.lib_preview_live = None;
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
            app.insert_at(ids, t, vt);
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
