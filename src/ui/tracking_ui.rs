//! Tracking pane (non-blocking): follow a point or a rectangular area through a clip and keep the
//! result as a reusable project path.
//!
//! Pick the clip (the selection by default), place the tracker box — the Preview draws it and drags it
//! while this pane is on screen — set its size and the search radius, then "Track forward" /
//! "Track backward". `engine::tracking` does the matching on its own thread and this pane shows a
//! progress bar and repaints; "Cancel" just drops the job. The result saves into `Project.paths`
//! ("Save as path") and can be dropped straight onto the clip's X/Y keyframes ("Apply to clip"),
//! which is the manual-tracking workflow the paths exist for.

use crate::engine::tracking::TrackJob;
use crate::media::Backend;
use crate::model::{Id, Project};
use crate::theme::Palette;
use crate::ui::tools::{glyph_text_button, Glyph};
use eframe::egui::{self, Button, DragValue};

pub struct TrackState {
    /// Clip to track; None follows the selection.
    pub clip: Option<Id>,
    /// Tracker box in project px relative to the canvas centre (the Preview draws and drags it).
    pub cx: f32,
    pub cy: f32,
    pub hw: f32,
    pub hh: f32,
    /// How far from the last position the template is looked for, in px.
    pub search: f32,
    /// Re-grab the template every N frames (0 = never; a rigid feature drifts less without it).
    pub refresh: u32,
    pub name: String,
    pub status: String,
    job: Option<TrackJob>,
    points: Vec<(f32, f32, f32)>,
}

impl Default for TrackState {
    fn default() -> Self {
        Self {
            clip: None,
            cx: 0.0,
            cy: 0.0,
            hw: 32.0,
            hh: 32.0,
            search: 24.0,
            refresh: 10,
            name: String::new(),
            status: String::new(),
            job: None,
            points: Vec::new(),
        }
    }
}

impl TrackState {
    /// The box the Preview should draw, in project px relative to the canvas centre.
    pub fn box_rect(&self) -> (f32, f32, f32, f32) {
        (self.cx, self.cy, self.hw, self.hh)
    }
}

/// The clip being tracked: the explicit pick if it still exists, else the first visual clip selected.
fn target(project: &Project, state: &TrackState, selection: &[Id]) -> Option<Id> {
    state
        .clip
        .filter(|&id| project.clip(id).is_some())
        .or_else(|| selection.iter().copied().find(|&id| project.clip(id).is_some_and(|c| c.is_visual())))
}

pub fn show(
    ui: &mut egui::Ui,
    state: &mut TrackState,
    project: &mut Project,
    selection: &[Id],
    backend: Backend,
    _palette: &Palette,
    undo: &mut dyn FnMut(&Project),
) -> bool {
    let mut changed = false;
    // drain the worker first, so the bar and the point count are this frame's truth
    if state.job.as_mut().is_some_and(|j| j.poll()) {
        ui.ctx().request_repaint();
    } else if let Some(j) = state.job.take() {
        state.status = format!("tracked {} points", j.points.len());
        state.points = j.points;
    }
    let running = state.job.is_some();
    let target = target(project, state, selection);

    ui.strong("Tracking");
    ui.horizontal(|ui| {
        ui.label("Clip");
        let name = target.and_then(|id| project.clip(id)).map(|c| c.name.clone()).unwrap_or_else(|| "—".into());
        ui.monospace(name);
        if ui.add_enabled(!running, Button::new("Use selected")).clicked() {
            state.clip = selection.first().copied();
        }
    });
    egui::Grid::new("track_params").num_columns(2).show(ui, |ui| {
        let rows: [(&str, f32, f32); 5] = [
            ("X", -10000.0, 10000.0),
            ("Y", -10000.0, 10000.0),
            ("Width", 4.0, 2000.0),
            ("Height", 4.0, 2000.0),
            ("Search radius", 1.0, 512.0),
        ];
        for (label, lo, hi) in rows {
            ui.label(label);
            // width/height are shown whole; the tracker keeps them as half-extents
            let (v, half) = match label {
                "X" => (&mut state.cx, false),
                "Y" => (&mut state.cy, false),
                "Width" => (&mut state.hw, true),
                "Height" => (&mut state.hh, true),
                _ => (&mut state.search, false),
            };
            let mut shown = if half { *v * 2.0 } else { *v };
            if ui.add_enabled(!running, DragValue::new(&mut shown).range(lo..=hi).speed(1.0).suffix(" px")).changed() {
                *v = if half { shown / 2.0 } else { shown };
            }
            ui.end_row();
        }
        ui.label("Refresh every");
        ui.add_enabled(!running, DragValue::new(&mut state.refresh).range(0..=240).suffix(" frames"))
            .on_hover_text("Re-grab the template that often (0 = never, best for a rigid feature)");
        ui.end_row();
    });

    let mut go = None;
    ui.horizontal(|ui| {
        ui.add_enabled_ui(!running && target.is_some(), |ui| {
            if glyph_text_button(ui, Glyph::Target, "Track forward").clicked() {
                go = Some(false);
            }
            if glyph_text_button(ui, Glyph::Target, "Track backward").clicked() {
                go = Some(true);
            }
        });
        if running && ui.button("Cancel").clicked() {
            state.job = None; // hanging up the channel stops the worker
            state.status = "cancelled".into();
        }
    });
    if let (Some(backward), Some(id)) = (go, target) {
        state.points.clear();
        state.status.clear();
        match TrackJob::start(project, id, state.box_rect(), state.search, state.refresh, backward, backend) {
            Ok(j) => state.job = Some(j),
            Err(e) => state.status = e,
        }
    }
    if let Some(j) = &state.job {
        ui.add(egui::ProgressBar::new(j.progress).show_percentage());
    }

    let have = state.points.len() >= 2;
    ui.separator();
    ui.horizontal(|ui| {
        ui.label("Name");
        ui.add(egui::TextEdit::singleline(&mut state.name).desired_width(110.0).hint_text("Path"));
        if ui.add_enabled(have, Button::new("Save as path")).clicked() {
            undo(project);
            project.add_path(state.name.clone(), state.points.clone());
            state.status = format!("saved {} points to the project paths", state.points.len());
            changed = true;
        }
        if ui.add_enabled(have && target.is_some(), Button::new("Apply to clip")).clicked() {
            undo(project);
            if let Some(id) = target {
                changed = project.apply_path(id, &state.points);
                state.status =
                    if changed { "keyframed the clip's position".into() } else { "the track is too short".to_string() };
            }
        }
    });
    if !state.status.is_empty() {
        ui.weak(&state.status);
    }
    changed
}
