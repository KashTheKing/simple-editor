//! Tracking pane (non-blocking): follow a point or a rectangular area through a clip and keep the
//! result as a reusable project path.
//!
//! Pick the clip (the selection by default; right-click ▸ "Use selected clip" pins the current one), place
//! the tracker box - "Place box" puts it on the Preview to drag while this pane is on screen - set its
//! size and the search radius, then "Track forward" /
//! "Track backward". `engine::tracking` does the matching on its own thread, which the app polls every
//! frame (`TrackState::poll`, so a hidden tab never stalls it), and this pane shows a progress bar;
//! "Cancel" just drops the job. The result saves into `Project.paths`
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
    /// "Place box": the Preview draws and drags the box only while this is on, so an open Tracking
    /// pane does not take canvas drags away from the clip under the box.
    pub placing: bool,
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
            placing: false,
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

    /// The box, while "Place box" is on.
    pub fn preview_box(&self) -> Option<(f32, f32, f32, f32)> {
        self.placing.then(|| self.box_rect())
    }

    /// Drain the worker (called by the app every frame, whether or not the pane is drawn).
    pub fn poll(&mut self, ctx: &egui::Context) {
        if self.job.as_mut().is_some_and(|j| j.poll()) {
            ctx.request_repaint();
        } else if let Some(j) = self.job.take() {
            self.status = format!("tracked {} points", j.points.len());
            self.points = j.points;
        }
    }
}

// ---- ws:canvas-handles-monitor ----
impl TrackState {
    /// The finished track this pane holds, for Auto Reframe: the clip it belongs to (the explicit
    /// pick, else the first selected visual clip - the pane's own rule) and its points, once at least
    /// two frames were tracked and the worker is done. None = nothing to reframe from.
    pub(crate) fn tracked(&self, project: &Project, selection: &[Id]) -> Option<(Id, &[(f32, f32, f32)])> {
        if self.job.is_some() || self.points.len() < 2 {
            return None;
        }
        target(project, self, selection).map(|id| (id, &self.points[..]))
    }
}

// ---- ws:canvas-handles-monitor ----
#[cfg(test)]
impl TrackState {
    /// A finished track with `points` already in hand - `points`/`job` are private (no live `App`
    /// exists to drive `TrackJob::start` for real in a test; see `app::monitor`'s tests), so
    /// `app::monitor::reframe`'s own tests build one through this instead of the tracking pane's UI.
    pub(crate) fn with_points(points: Vec<(f32, f32, f32)>) -> Self {
        Self { points, ..Default::default() }
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
    palette: &Palette,
    undo: &mut dyn FnMut(&Project),
) -> bool {
    let bg = crate::ui::markers_ui::menu_area(ui);
    let changed = egui::ScrollArea::vertical()
        .id_salt("tracking_pane")
        .auto_shrink([false, false])
        .show(ui, |ui| body(ui, state, project, selection, backend, palette, undo))
        .inner;
    bg.context_menu(|ui| {
        let ok = state.job.is_none() && !selection.is_empty();
        let r = ui.add_enabled_ui(ok, |ui| crate::ui::menu::row(ui, None, "Use selected clip", "")).inner;
        if r.on_hover_text("Track the selected clip even after the selection moves on").clicked() {
            state.clip = selection.first().copied();
        }
        let r =
            ui.add_enabled_ui(state.clip.is_some(), |ui| crate::ui::menu::row(ui, None, "Follow the selection", ""));
        if r.inner.clicked() {
            state.clip = None;
        }
    });
    changed
}

fn body(
    ui: &mut egui::Ui,
    state: &mut TrackState,
    project: &mut Project,
    selection: &[Id],
    backend: Backend,
    _palette: &Palette,
    undo: &mut dyn FnMut(&Project),
) -> bool {
    let mut changed = false;
    let running = state.job.is_some();
    let target = target(project, state, selection);

    ui.horizontal(|ui| {
        ui.label("Clip");
        let name = target.and_then(|id| project.clip(id)).map(|c| c.name.clone()).unwrap_or_else(|| " - ".into());
        ui.monospace(name).on_hover_text(if state.clip.is_some() {
            "Pinned - right-click ▸ Follow the selection"
        } else {
            "Follows the selection - right-click ▸ Use selected clip to pin it"
        });
        ui.toggle_value(&mut state.placing, "Place box")
            .on_hover_text("Show the tracker box on the Preview - drag it onto the feature to follow");
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Asset, AudioStreamInfo, ClipKind};
    use std::time::{Duration, Instant};

    fn project() -> (Project, Id) {
        let mut p = Project::new();
        let aid = p.add_asset(Asset {
            id: 0,
            path: format!("C:/does-not-exist-{}.mp4", std::process::id()),
            kind: ClipKind::Video,
            duration: 2.0,
            width: 320,
            height: 240,
            fps: 30.0,
            audio_streams: vec![AudioStreamInfo::default()],
            codec: String::new(),
            folder: String::new(),
            tags: Vec::new(),
            label: 0,
            description: String::new(),
            rel_path: None,
            parent: None,
            range: None,
            effects: Vec::new(),
        });
        p.insert_asset_clips(aid, 0.0, Some(0));
        let id = p.tracks[0].clips[0].id;
        (p, id)
    }

    /// Opening the pane changes nothing, asks for no repaint, and keeps the box off the Preview until
    /// "Place box" is on.
    #[test]
    fn assert_no_idle_repaint_tracking_pane() {
        let (mut p, clip) = project();
        let before = p.to_json();
        let mut st = TrackState::default();
        let pal = Palette::new(true, egui::Color32::WHITE);
        let ctx = egui::Context::default();
        for _ in 0..30 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut undo = |_: &Project| panic!("no undo without a click");
                    assert!(!show(ui, &mut st, &mut p, &[clip], Backend::Ffmpeg, &pal, &mut undo));
                });
            });
        }
        assert_eq!(p.to_json(), before);
        assert!(!ctx.has_requested_repaint(), "an idle Tracking pane must not spin");
        assert!(st.preview_box().is_none(), "no box on the Preview until Place box");
        st.placing = true;
        assert_eq!(st.preview_box(), Some(st.box_rect()));
    }

    /// `poll` alone finishes a job: the app calls it every frame, so a hidden pane never stalls one.
    #[test]
    fn a_job_finishes_without_the_pane() {
        let (p, clip) = project();
        let mut st = TrackState::default();
        st.job = Some(TrackJob::start(&p, clip, st.box_rect(), 24.0, 0, false, Backend::Ffmpeg).unwrap());
        let ctx = egui::Context::default();
        let t0 = Instant::now();
        while st.job.is_some() && t0.elapsed() < Duration::from_secs(20) {
            st.poll(&ctx);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(st.job.is_none(), "the worker hung up (missing footage) and poll took the result");
        assert_eq!(st.status, "tracked 0 points");
    }
}
