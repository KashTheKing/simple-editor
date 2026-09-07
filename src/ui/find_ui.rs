//! ---- ws:pro-timeline ----
//! Ctrl+F Find: substring search over clip names, marker names/notes, subtitle cues and sequence
//! names. `find()`/`jump_for()` are pure (no `App`), so they're unit-testable; the actual WINDOW_DRAWER
//! (`App`-shaped, `fn(&mut App, &egui::Context)`) lives in `ui::app::tools_timeline_pro::window` — this
//! module is a SIBLING of `ui::app` (like `ui::confirm`, `ui::timeline`), so it cannot reach `App`'s
//! private `playhead`/`player`/`selection`/`layout` fields the way a child module of `ui::app` can (see
//! the PR body: a deviation from the plan's own "WINDOW_DRAWERS (find_ui::window)" phrasing).

use crate::model::{Id, Project};
use crate::ui::layout::Pane;
use eframe::egui;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitKind {
    Clip,
    Marker,
    Cue,
    Sequence,
}

#[derive(Clone, Debug)]
pub struct Hit {
    pub kind: HitKind,
    pub id: Id,
    pub t: f64,
    pub text: String,
}

/// Case-insensitive substring search across clip names, marker names/notes, subtitle cues and sequence
/// names. Clips/markers/cues are the CURRENT scope only (`Project.tracks` — the main timeline, or
/// whichever sequence is open via `Project.editing`/the swap in `open_sequence`).
/// ponytail: a clip buried in an unopened sequence isn't found by name (open that sequence first);
/// sequences themselves are always searched by name so Find can at least locate them.
pub fn find(p: &Project, q: &str) -> Vec<Hit> {
    if q.trim().is_empty() {
        return Vec::new();
    }
    let ql = q.to_lowercase();
    let mut hits = Vec::new();
    for (_, c) in p.all_clips() {
        if c.name.to_lowercase().contains(&ql) {
            hits.push(Hit { kind: HitKind::Clip, id: c.id, t: c.start, text: c.name.clone() });
        }
    }
    for m in &p.markers {
        if m.name.to_lowercase().contains(&ql) || m.note.to_lowercase().contains(&ql) {
            hits.push(Hit { kind: HitKind::Marker, id: m.id, t: m.t, text: m.name.clone() });
        }
    }
    for cue in &p.subtitles {
        if cue.text.to_lowercase().contains(&ql) {
            hits.push(Hit { kind: HitKind::Cue, id: cue.id, t: cue.start, text: cue.text.clone() });
        }
    }
    for s in &p.sequences {
        if s.name.to_lowercase().contains(&ql) {
            hits.push(Hit { kind: HitKind::Sequence, id: s.id, t: 0.0, text: s.name.clone() });
        }
    }
    hits
}

/// Where a chosen `Hit` sends the editor: playhead time, the pane to surface, and what to select.
/// `select` is `None` for a Sequence hit — it isn't "selected", it's opened. `kind` carries the hit's
/// `HitKind` through so the `App`-touching caller (which owns the several DIFFERENT selection fields —
/// clip vs. marker vs. cue — this module can't see) knows which one `select`'s id belongs to.
pub struct Jump {
    pub t: f64,
    pub pane: Option<Pane>,
    pub select: Option<Id>,
    pub kind: HitKind,
}

pub fn jump_for(hit: &Hit) -> Jump {
    match hit.kind {
        HitKind::Clip => Jump { t: hit.t, pane: Some(Pane::Timeline), select: Some(hit.id), kind: hit.kind },
        HitKind::Marker => Jump { t: hit.t, pane: Some(Pane::Timeline), select: Some(hit.id), kind: hit.kind },
        HitKind::Cue => Jump { t: hit.t, pane: Some(Pane::Subtitles), select: Some(hit.id), kind: hit.kind },
        // ponytail: sequences live in the Library, not on the timeline — reveal there rather than
        // opening it outright (opening changes the edit context, a bigger action than a Find jump).
        HitKind::Sequence => Jump { t: 0.0, pane: Some(Pane::Library), select: None, kind: hit.kind },
    }
}

#[derive(Default)]
pub struct FindState {
    pub open: bool,
    pub query: String,
}

/// Non-blocking Ctrl+F window body: a query field and a results list. Returns the chosen `Hit`'s `Jump`
/// on Enter or a result click (which also closes the window); the caller applies it to `App`.
pub fn window(ctx: &egui::Context, state: &mut FindState, p: &Project) -> Option<Jump> {
    if !state.open {
        return None;
    }
    let mut jump = None;
    let mut open = true;
    let esc = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    egui::Window::new("Find").open(&mut open).default_width(360.0).show(ctx, |ui| {
        let resp =
            ui.add(egui::TextEdit::singleline(&mut state.query).hint_text("Search clips, markers, cues, sequences…"));
        resp.request_focus();
        let hits = find(p, &state.query);
        if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            if let Some(h) = hits.first() {
                jump = Some(jump_for(h));
            }
        }
        ui.separator();
        egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
            if hits.is_empty() && !state.query.trim().is_empty() {
                ui.weak("No matches");
            }
            for h in &hits {
                let kind = match h.kind {
                    HitKind::Clip => "Clip",
                    HitKind::Marker => "Marker",
                    HitKind::Cue => "Cue",
                    HitKind::Sequence => "Sequence",
                };
                if ui.selectable_label(false, format!("{kind}  {}", h.text)).clicked() {
                    jump = Some(jump_for(h));
                }
            }
        });
    });
    state.open = if jump.is_some() { false } else { open && !esc };
    jump
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Clip, ClipKind, Cue, Marker, Sequence};

    fn seeded() -> Project {
        let mut p = Project::new();
        p.tracks[0].clips.push(Clip::new(1, ClipKind::Video, "Intro Shot", 0.0, 3.0));
        p.markers.push(Marker { id: 2, name: "Beat drop".into(), ..Default::default() });
        p.subtitles.push(Cue { id: 3, start: 1.0, end: 2.0, text: "hello beat world".into() });
        p.sequences.push(Sequence {
            id: 4,
            name: "Beat Sequence".into(),
            width: 1920,
            height: 1080,
            fps: 30.0,
            tracks: vec![],
        });
        p
    }

    #[test]
    fn find_matches_clips_markers_cues_sequences() {
        let p = seeded();
        let hits = find(&p, "beat");
        assert_eq!(hits.iter().filter(|h| h.kind == HitKind::Marker).count(), 1);
        assert_eq!(hits.iter().filter(|h| h.kind == HitKind::Cue).count(), 1);
        assert_eq!(hits.iter().filter(|h| h.kind == HitKind::Sequence).count(), 1);
        // case-insensitive: "Intro Shot" matches "SHOT"
        let hits2 = find(&p, "SHOT");
        assert_eq!(hits2.len(), 1);
        assert_eq!(hits2[0].kind, HitKind::Clip);
        assert!(find(&p, "").is_empty());
        assert!(find(&p, "nope-nothing-matches-this").is_empty());
    }

    #[test]
    fn jump_for_picks_the_right_pane_and_selection() {
        let clip = Hit { kind: HitKind::Clip, id: 1, t: 2.0, text: "x".into() };
        let j = jump_for(&clip);
        assert_eq!(j.t, 2.0);
        assert_eq!(j.pane, Some(Pane::Timeline));
        assert_eq!(j.select, Some(1));
        assert_eq!(j.kind, HitKind::Clip);

        let marker = Hit { kind: HitKind::Marker, id: 2, t: 1.5, text: "x".into() };
        let j = jump_for(&marker);
        assert_eq!(j.t, 1.5);
        assert_eq!(j.pane, Some(Pane::Timeline));
        // a Marker's id must NOT land in the clip-selection field's `select` -- the caller routes it
        // to `TimelineState.selected_marker` based on `kind`, never `app.selection`.
        assert_eq!(j.select, Some(2));
        assert_eq!(j.kind, HitKind::Marker);

        let cue = Hit { kind: HitKind::Cue, id: 3, t: 1.0, text: "x".into() };
        let j = jump_for(&cue);
        assert_eq!(j.pane, Some(Pane::Subtitles));
        assert_eq!(j.select, Some(3));
        assert_eq!(j.kind, HitKind::Cue);

        let seq = Hit { kind: HitKind::Sequence, id: 4, t: 0.0, text: "x".into() };
        let j = jump_for(&seq);
        assert_eq!(j.pane, Some(Pane::Library));
        assert_eq!(j.select, None);
        assert_eq!(j.kind, HitKind::Sequence);
    }
}
