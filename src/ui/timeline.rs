//! Timeline widget (custom painted). Header column (track name, M/S buttons, height drag handle at the
//! bottom edge), ruler (ticks, in/out markers, click/drag scrubs), track lanes (video tracks top→bottom =
//! Vn..V1, then A1..An), clips (name, waveform for audio via WaveformCache, keyframe diamonds, disabled
//! = dimmed, selected = accent outline), playhead (drag by ruler or line).
//!
//! Interaction: click select (Ctrl toggles), drag clip body = move (linked clips follow; Project::move_clips
//! all-or-nothing; up/down across tracks of the same kind), drag clip edges = trim (Clip::trim_start /
//! trim_end with Project::max_clip_duration), snapping (playhead, clip edges, in/out, 0; ~8 px) when
//! `snap`, right-click context menu (Split, Delete, Ripple Delete, Link/Unlink, Enable/Disable, Add
//! Video/Audio Track, Remove Track (on headers), Mute/Solo). Ctrl+Scroll = zoom around the cursor,
//! Shift+Scroll = horizontal pan, Alt+Scroll = height of the track under the cursor, plain scroll =
//! vertical pan. Accepts egui dnd payloads `DragPayload` (Asset → Project::insert_asset_clips at the drop
//! time/track; Path → returned in `dropped_files`). Every mutation calls `c.undo(&project)` first (once per
//! gesture, at drag start) and sets `edited`. While `playing`, auto-scroll keeps the playhead visible.

use crate::media::waveform::WaveformCache;
use crate::model::{Id, Project};
use crate::theme::Palette;
use eframe::egui;
use std::path::PathBuf;

pub struct TimelineState {
    /// Pixels per second.
    pub zoom: f32,
    /// Horizontal scroll in seconds at the left edge of the lanes.
    pub scroll_x: f64,
    /// Vertical scroll in points.
    pub scroll_y: f32,
    /// Width of the header column in points.
    pub header_w: f32,
    /// Content rect (lanes area, excluding header & ruler) from the last frame — used by the app for file drops.
    pub lanes_rect: egui::Rect,
}

impl Default for TimelineState {
    fn default() -> Self {
        Self { zoom: 40.0, scroll_x: 0.0, scroll_y: 0.0, header_w: 150.0, lanes_rect: egui::Rect::NOTHING }
    }
}

impl TimelineState {
    /// Timeline time at an absolute x position (points).
    pub fn time_at(&self, x: f32) -> f64 {
        self.scroll_x + ((x - self.lanes_rect.left()) / self.zoom) as f64
    }
    pub fn x_at(&self, t: f64) -> f32 {
        self.lanes_rect.left() + ((t - self.scroll_x) as f32) * self.zoom
    }
    /// Track index under an absolute y position (points), given the project (for track heights).
    pub fn track_at(&self, _y: f32, _project: &Project) -> Option<usize> {
        todo!("ui::timeline::TimelineState::track_at")
    }
    /// Fit [0, duration] into `avail_w` points.
    pub fn zoom_to_fit(&mut self, duration: f64, avail_w: f32) {
        let d = duration.max(1.0) as f32;
        self.zoom = ((avail_w - 20.0).max(50.0) / d).clamp(0.5, 2000.0);
        self.scroll_x = 0.0;
    }
    /// Multiply zoom, keeping the time under `anchor_x` (absolute points) fixed if given.
    pub fn zoom_by(&mut self, factor: f32, anchor_x: Option<f32>) {
        let anchor_t = anchor_x.map(|x| self.time_at(x));
        self.zoom = (self.zoom * factor).clamp(0.5, 2000.0);
        if let (Some(t), Some(x)) = (anchor_t, anchor_x) {
            self.scroll_x = (t - ((x - self.lanes_rect.left()) / self.zoom) as f64).max(0.0);
        }
    }
    /// Scroll so `t` is visible (used when the playhead runs off-screen).
    pub fn ensure_visible(&mut self, t: f64) {
        let w = (self.lanes_rect.width() / self.zoom) as f64;
        if w <= 0.0 {
            return;
        }
        if t < self.scroll_x || t > self.scroll_x + w {
            self.scroll_x = (t - w * 0.1).max(0.0);
        }
    }
}

pub struct TimelineCtx<'a> {
    pub project: &'a mut Project,
    pub selection: &'a mut Vec<Id>,
    pub playhead: &'a mut f64,
    /// Call with the project *before* mutating it (once per gesture) — pushes an undo snapshot.
    pub undo: &'a mut dyn FnMut(&Project),
    pub waveforms: &'a mut WaveformCache,
    pub palette: &'a Palette,
    pub snap: bool,
    pub playing: bool,
}

#[derive(Default)]
pub struct TimelineResponse {
    /// The project was mutated (app marks dirty & re-renders).
    pub edited: bool,
    /// The playhead was moved by the user.
    pub seeked: bool,
    /// Files dropped via dnd `DragPayload::Path` — (path, timeline time, track index).
    pub dropped_files: Vec<(PathBuf, f64, Option<usize>)>,
}

pub fn show(_ui: &mut egui::Ui, _state: &mut TimelineState, _c: TimelineCtx<'_>) -> TimelineResponse {
    todo!("ui::timeline::show")
}
