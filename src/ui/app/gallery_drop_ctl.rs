//! Dragging Gallery cards onto the timeline, App side: the timeline resolves what the pointer is over
//! (`timeline::gallery_drop::aim`, the same function that lit the hover) and reports it with the payload;
//! `execute` turns the pair into an edit through the paths a click already uses (`gallery.apply`,
//! `keyblocks.apply`, `library.apply` after an install, `add_transitions`, the template placers), so a
//! drop is exactly a click aimed somewhere. Every drop is one undo step.

use super::*;
use crate::engine::library::{self, Kind};
use crate::ui::gallery::GalleryTab;
use crate::ui::timeline::gallery_drop::{pull_to_track, Aim};

/// How long (seconds) the clip a Gallery card would place is, for the hover ghost. Templates and titles
/// measure their clips; an installed HTML layer its `se:duration`; anything else a plain 3 s.
pub(super) fn drop_len(app: &App, p: &DragPayload) -> f64 {
    let span = |t: &crate::settings::Template| {
        let (clips, _) = crate::engine::presets::decode_template(t)?;
        let a = clips.iter().map(|c| c.start).fold(f64::MAX, f64::min);
        let b = clips.iter().map(|c| c.start + c.duration).fold(0.0, f64::max);
        (b > a).then_some(b - a)
    };
    let found = match p {
        DragPayload::Template(name) => template_ctl::find(app, name).map(|d| d.duration()).filter(|d| *d > 0.0),
        DragPayload::GalleryCard(GalleryTab::Titles, name) => crate::engine::presets::builtin_titles()
            .iter()
            .chain(app.settings.templates.iter())
            .find(|t| &t.name == name)
            .and_then(span),
        DragPayload::GalleryCard(GalleryTab::Sounds, name) => {
            crate::engine::gallery::find(&crate::engine::gallery::root(), crate::engine::gallery::Kind::Sound, name)
                .and_then(|i| i.data.get("duration").and_then(|d| d.as_f64()))
        }
        DragPayload::LibItem(id) => match library::split_id(id) {
            Some((Kind::Html, short)) => {
                library::read(Kind::Html, &short).ok().map(|h| crate::engine::html::parse(&h).duration)
            }
            Some((Kind::Icon, _)) => Some(5.0),
            _ => None,
        },
        _ => None,
    };
    found.filter(|d| d.is_finite() && *d > 0.0).unwrap_or(3.0)
}

/// What a miss says (the pointer was on the timeline, but not on something this card can land on).
fn miss(p: &DragPayload) -> &'static str {
    use crate::ui::timeline::gallery_drop::{drop_kind, DropKind};
    match drop_kind(p) {
        Some(DropKind::OnClip) => "Drop it on a clip it applies to",
        Some(DropKind::OnEdge) => "Drop a transition on a cut or a clip's edge",
        Some(DropKind::OnTrack) => "Drop it on an empty part of a video track, or below the last track",
        Some(DropKind::OnAudio) => "Drop a sound on an audio track, or below the last track",
        None => "That cannot be dropped on the timeline",
    }
}

/// Which `transitions_ui::Where` puts a library transition on `clip`'s start / end edge the way the old
/// drop did: the cut when a clip abuts there, else a fade from / to nothing.
fn edge_where(project: &Project, clip: Id, out: bool) -> &'static str {
    use crate::ui::transitions_ui::{right_neighbor, Where};
    match (out, right_neighbor(project, clip).is_some()) {
        (false, _) => Where::All.name(),
        (true, true) => Where::ConnEnd.name(),
        (true, false) => Where::End.name(),
    }
}

/// Carry out one dropped Gallery card. `aim` None = the pointer was not on a valid target.
pub(super) fn execute(app: &mut App, payload: DragPayload, aim: Option<Aim>) {
    let Some(aim) = aim else {
        app.toast(miss(&payload));
        return;
    };
    match (payload, aim) {
        (DragPayload::Effect(kind), Aim::Clips { clips, .. }) => {
            apply_tool(app, GalleryTab::Effects, kind.name(), &clips);
        }
        // your own saved effect / transition / speed ramp (resolved by name in your Gallery)
        (DragPayload::GalleryCard(GalleryTab::Transitions, name), Aim::Edge { clip, out }) => {
            let w = edge_where(&app.project, clip, out);
            let args = json!({"tab": "Transitions", "name": format!("mine:{name}"), "clip_ids": [clip], "where": w});
            match app.run_tool_undoable("gallery.apply", &args) {
                Ok(_) => app.toast(format!("{name} added")),
                Err(e) => app.toast(e),
            }
        }
        (DragPayload::GalleryCard(GalleryTab::Effects, name), Aim::Clips { clips, .. }) => {
            apply_tool(app, GalleryTab::Effects, &format!("mine:{name}"), &clips)
        }
        (DragPayload::GalleryCard(GalleryTab::Sounds, name), Aim::Place { track, t }) => {
            app.push_undo();
            match gallery_mine::place_sound(app, &name, t, track, track.is_none()) {
                Ok(ids) => {
                    app.selection = ids;
                    app.after_edit();
                    app.toast(format!("{name} added"));
                }
                Err(e) => app.toast(e),
            }
        }
        (DragPayload::GalleryCard(tab, name), Aim::Clips { clips, .. }) => apply_tool(app, tab, &name, &clips),
        // ws:preview-audio-only: a visualizer layer where it was dropped
        (DragPayload::GalleryCard(GalleryTab::Visualizers, name), Aim::Place { track, t }) => {
            let args = json!({"tab": "Visualizers", "name": name, "at": t, "track": track});
            match app.run_tool_undoable("gallery.apply", &args) {
                Ok(_) => app.toast(format!("{name} added")),
                Err(e) => app.toast(e),
            }
        }
        // ws:keyframe-blocks: starting where it was dropped
        (DragPayload::KeyBlock(name), Aim::Clips { clips, t, .. }) => {
            let args = json!({"name": name, "clip_ids": clips, "at": t});
            match app.run_tool_undoable("keyblocks.apply", &args) {
                Ok(_) => app.toast(format!("{name} applied")),
                Err(e) => app.toast(e),
            }
        }
        // a transition belongs to a cut: the edge nearest the pointer (or the half of the clip it is over)
        (DragPayload::Transition(kind), Aim::Edge { clip, out }) => {
            let snap = app.project.to_json();
            let dur = app.transitions_ui.duration;
            let st = &mut app.transitions_ui;
            if transitions_ui::add_transitions(&mut app.project, &[clip], st, kind, dur, out) > 0 {
                push_undo_json(&mut app.undo, &mut app.redo, snap);
                app.after_edit();
                app.toast(format!("{} ({dur:.2} s)", kind.name()));
            } else {
                app.toast("Could not add a transition here");
            }
        }
        (DragPayload::LibItem(id), aim) => {
            let mut target = library_ctl::Target::default();
            match aim {
                Aim::Clips { clips, t, .. } => {
                    target.clips = clips;
                    target.at = Some(t);
                }
                Aim::Edge { clip, out } => {
                    target.place = Some(edge_where(&app.project, clip, out).to_string());
                    target.clips = vec![clip];
                }
                Aim::Place { track, t } => {
                    target.at = Some(t);
                    target.track = Some(track);
                }
            }
            library_ctl::click(app, &id, target); // installs first when it must, then does what a click does
        }
        (DragPayload::Template(name), Aim::Place { track, t }) => {
            // ws:animator-templates: with its layers, transitions and the library's selection in its slots
            let fills = template_ctl::library_fills(app, &name);
            template_ctl::place(app, &name, t, track, fills);
        }
        // Titles: the click's own post-step too (its Customize rows)
        (DragPayload::GalleryCard(GalleryTab::Titles, name), Aim::Place { track, t }) => {
            match app.place_title_template(&name, t) {
                Ok((ids, customize)) => {
                    if pull_to_track(&mut app.project, &ids, track) {
                        app.after_edit();
                    }
                    app.gallery.customize = customize;
                    if app.gallery.customize.is_empty() {
                        app.toast(format!("Placed \"{name}\" (nothing to customize)"));
                    }
                }
                Err(e) => app.toast(e),
            }
        }
        _ => {}
    }
}

/// `gallery.apply` on `clips`, one undo step, a toast either way.
fn apply_tool(app: &mut App, tab: GalleryTab, name: &str, clips: &[Id]) {
    let args = json!({"tab": tab.name(), "name": name, "clip_ids": clips, "intensity": 1.0});
    match app.run_tool_undoable("gallery.apply", &args) {
        Ok(_) => app.toast(match clips.len() {
            1 => format!("{name} applied"),
            n => format!("{name} applied to {n} clips"),
        }),
        Err(e) => app.toast(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Clip, ClipKind};

    /// A library transition dropped on a clip's start is the cut-or-fade-in; on its end, the cut when a clip
    /// abuts there, else a fade-out - the old drop's rule, spelled as `gallery.apply`'s `where`.
    #[test]
    fn an_edge_drop_maps_to_the_old_start_end_rule() {
        let mut p = Project::new();
        p.tracks[0].clips.push(Clip::new(1, ClipKind::Video, "a", 0.0, 4.0));
        p.tracks[0].clips.push(Clip::new(2, ClipKind::Video, "b", 4.0, 4.0));
        p.tracks[0].sort();
        assert_eq!(edge_where(&p, 1, false), "all");
        assert_eq!(edge_where(&p, 1, true), "connection_end", "B abuts A's end");
        assert_eq!(edge_where(&p, 2, true), "end", "nothing after B: a fade-out");
    }

    #[test]
    fn a_miss_says_what_to_aim_at() {
        for p in [
            DragPayload::Effect(crate::model::EffectKind::Blur),
            DragPayload::Transition(crate::model::TransitionKind::Wipe),
            DragPayload::Template("t".into()),
        ] {
            assert!(miss(&p).starts_with("Drop"), "{p:?}");
        }
    }
}
