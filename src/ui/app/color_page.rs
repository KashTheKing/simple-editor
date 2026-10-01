//! ---- ws:color-page ----
//! The Resolve-style Color page's App side: the Grade pane (wheels / bars, `color_ui::grade_panel`),
//! the Clips strip, `color.keyframe_grade`, and `tick` - on the Color page the graded clip follows the
//! playhead, as in Resolve. Without it an empty selection left the Inspector on Project settings and
//! Nodes on "Select a clip", so nothing on the page could edit colour.

use super::*;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::ui::{color_ui, Gesture};
use std::cell::Cell;

thread_local! {
    /// The clip `tick` (or a Clips-strip click) picked; while the selection is still just that clip it
    /// keeps following the playhead. A clip the user selects on the timeline is left alone.
    static AUTO: Cell<Option<Id>> = const { Cell::new(None) };
}

/// The clip the picture shows at `t`: the topmost (last) video track's clip covering it.
pub(super) fn graded_clip_at(p: &Project, t: f64) -> Option<Id> {
    p.tracks
        .iter()
        .filter(|tr| tr.kind == TrackKind::Video)
        .flat_map(|tr| &tr.clips)
        .filter(|c| c.contains(t))
        .last()
        .map(|c| c.id)
}

/// Whether `tick` may retarget: nothing is selected, or only the clip it picked itself.
fn follows(selection: &[Id], auto: Option<Id>, p: &Project) -> bool {
    !selection.iter().any(|&i| p.clip(i).is_some()) || (selection.len() == 1 && Some(selection[0]) == auto)
}

pub(super) fn tick(app: &mut App, _ctx: &egui::Context) {
    if app.settings.page != "Color" || !follows(&app.selection, AUTO.get(), &app.project) {
        return;
    }
    if let Some(id) = graded_clip_at(&app.project, app.playhead) {
        if app.selection != [id] {
            app.selection = vec![id];
        }
        AUTO.set(Some(id));
    }
}

pub(super) fn draw(app: &mut App, ui: &mut egui::Ui, pane: Pane) -> bool {
    match pane {
        Pane::Grade => grade(app, ui),
        Pane::Clips => clips(app, ui),
        _ => return false,
    }
    true
}

/// The first selected clip on a video track - what the Grade pane edits.
fn target(app: &App) -> Option<Id> {
    let p = &app.project;
    app.selection.iter().copied().find(|&i| p.track_of(i).is_some_and(|t| p.tracks[t].kind == TrackKind::Video))
}

fn grade(app: &mut App, ui: &mut egui::Ui) {
    let Some(mut clip) = target(app).and_then(|id| app.project.clip(id).cloned()) else {
        ui.centered_and_justified(|ui| ui.weak("Put the playhead over a video clip to grade it"));
        return;
    };
    let lt = clip.local(app.playhead);
    let mut g = Gesture::default();
    egui::ScrollArea::both().show(ui, |ui| color_ui::grade_panel(ui, &mut clip, lt, &app.palette, &mut g));
    if g.start {
        push_undo_json(&mut app.undo, &mut app.redo, app.project.to_json());
    }
    if g.changed {
        if let Some(c) = app.project.clip_mut(clip.id) {
            c.effects = clip.effects;
        }
        app.after_edit();
    }
}

/// One card per video clip in timeline order: thumbnail, number and start timecode. Click = grade it
/// (select + jump the playhead there).
fn clips(app: &mut App, ui: &mut egui::Ui) {
    let p = &app.project;
    let mut list: Vec<(Id, f64, String, String, f64)> = p
        .tracks
        .iter()
        .filter(|tr| tr.kind == TrackKind::Video)
        .flat_map(|tr| &tr.clips)
        .map(|c| {
            let path = p.asset(c.asset).map(|a| a.path.clone()).unwrap_or_default();
            (c.id, c.start, c.name.clone(), path, c.src_in)
        })
        .collect();
    list.sort_by(|a, b| a.1.total_cmp(&b.1));
    if list.is_empty() {
        ui.centered_and_justified(|ui| ui.weak("No video clips on the timeline"));
        return;
    }
    let h = (ui.available_height() - 22.0).clamp(24.0, 120.0);
    let w = h * 16.0 / 9.0;
    let current = target(app);
    let mut picked = None;
    egui::ScrollArea::horizontal().show(ui, |ui| {
        ui.horizontal(|ui| {
            for (n, (id, start, name, path, src)) in list.iter().enumerate() {
                let r = ui
                    .vertical(|ui| {
                        let (rect, r) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::click());
                        let tex =
                            (!path.is_empty()).then(|| app.thumbs.texture(ui.ctx(), path, *src, h as u32)).flatten();
                        match tex {
                            Some((tex, _)) => {
                                let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                                ui.painter().image(tex, rect, uv, egui::Color32::WHITE);
                            }
                            None => {
                                ui.painter().rect_filled(rect, 2.0, app.palette.panel);
                            }
                        }
                        let (stroke, col) =
                            if current == Some(*id) { (2.0, app.palette.accent) } else { (1.0, app.palette.border) };
                        let kind = egui::StrokeKind::Inside;
                        ui.painter().rect_stroke(rect, 2.0, egui::Stroke::new(stroke, col), kind);
                        let tc = crate::ui::timecode(*start, app.project.fps);
                        ui.add(egui::Label::new(egui::RichText::new(format!("{:02}  {tc}", n + 1)).small()).truncate());
                        r.on_hover_text(name.as_str())
                    })
                    .inner;
                if r.clicked() {
                    picked = Some((*id, *start));
                }
            }
        });
    });
    if let Some((id, start)) = picked {
        app.selection = vec![id];
        AUTO.set(Some(id));
        app.player.pause();
        app.seek(start);
    }
}

pub const TOOLS: &[ToolDef] = &[ToolDef {
    name: "color.keyframe_grade",
    desc: "Add a keyframe at the playhead to every grade parameter (Primaries wheels, Curves, Levels, Hue, \
           Vignette) the clip has - the Color page's \"Keyframe grade\" button.",
    args: &["clip_id:integer:true:"],
    kind: ToolKind::Mutate,
    run: |app, args| {
        let id = super::tools_helpers::arg_u64(args, "clip_id").ok_or("missing clip_id")?;
        let t = app.playhead;
        let c = app.project.clip_mut(id).ok_or("no such clip")?;
        color_ui::key_grade(c, c.local(t));
        Ok(ToolOutcome::Done(serde_json::json!({"ok": true})))
    },
}];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_the_playhead_until_the_user_picks_a_clip() {
        let mut p = Project::new();
        let (a, b) = (1, 2);
        p.tracks[0].clips.push(Clip::new(a, ClipKind::Video, "a", 0.0, 2.0));
        p.tracks[0].clips.push(Clip::new(b, ClipKind::Video, "b", 2.0, 2.0));
        assert_eq!(graded_clip_at(&p, 1.0), Some(a));
        assert_eq!(graded_clip_at(&p, 3.0), Some(b));
        assert!(follows(&[], None, &p), "empty selection: grade what is under the playhead");
        assert!(follows(&[a], Some(a), &p), "our own pick keeps following");
        assert!(!follows(&[a], Some(b), &p), "a clip the user selected stays");
    }
}
