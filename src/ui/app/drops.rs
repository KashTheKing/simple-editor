use super::*;

// ---- ws:canvas-handles-monitor ----
/// Where a file drop landed, from the drop point against last frame's pane rects (all one-frame-stale
/// — see `MoodboardState::content_rect`'s doc comment). The monitor is checked before the moodboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DropTarget {
    Timeline,
    Monitor,
    Moodboard,
    Library,
}

pub(super) fn drop_target(pos: Option<egui::Pos2>, timeline: egui::Rect, monitor: egui::Rect, moodboard: egui::Rect) -> DropTarget {
    match pos {
        Some(p) if timeline.contains(p) => DropTarget::Timeline,
        Some(p) if monitor.contains(p) => DropTarget::Monitor,
        Some(p) if moodboard.contains(p) => DropTarget::Moodboard,
        _ => DropTarget::Library,
    }
}

impl App {
    pub(super) fn handle_drops(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        if dropped.is_empty() || self.export.is_some() {
            return;
        }
        // drop point in window points: OS cursor (physical px) / ppp − client-area origin
        let pos = ctx.input(|i| {
            let mut p = windows::Win32::Foundation::POINT::default();
            if !unsafe { GetCursorPos(&mut p) }.as_bool() {
                return None;
            }
            let inner = i.viewport().inner_rect?;
            Some(egui::pos2(p.x as f32 / i.pixels_per_point, p.y as f32 / i.pixels_per_point) - inner.min.to_vec2())
        });
        // a project file: open it
        if dropped.len() == 1
            && dropped[0].extension().map(|e| e.to_string_lossy().eq_ignore_ascii_case(PROJECT_EXT)).unwrap_or(false)
        {
            let dropped0 = dropped[0].clone();
            self.confirm_discard_then(move |app| app.open_project(&dropped0));
            return;
        }
        let ids = self.open_or_import(&dropped);
        if ids.is_empty() {
            return;
        }
        // one-frame-stale rects, like `lanes_rect` — see `MoodboardState::content_rect`'s doc comment
        match drop_target(pos, self.timeline.lanes_rect, self.preview.canvas_rect, self.moodboard.content_rect) {
            DropTarget::Timeline => {
                let p = pos.unwrap();
                let mut t = self.timeline.time_at(p.x).max(0.0);
                if self.settings.snap {
                    t = self.project.snap_frame(t);
                }
                let track = self.timeline.track_at(p.y, &self.project);
                let vt = track.filter(|&i| self.project.tracks[i].kind == TrackKind::Video);
                self.insert_at(ids, t, vt);
                self.after_edit();
            }
            // ws:canvas-handles-monitor: onto the monitor = "put it here, now" — a free video track at
            // the playhead, through the same insert_at a timeline drop uses
            DropTarget::Monitor => {
                self.insert_at(ids, self.playhead, None);
                self.after_edit();
            }
            DropTarget::Moodboard => {
                // snapshot after the import (which already pushed its own undo step if any file was fresh —
                // same two-steps-when-fresh/one-when-not pattern as `replace_container_dialog`) so adding the
                // moodboard entries is still undoable even when every dropped file was already a known asset
                let snap = self.project.to_json();
                let mut changed = false;
                for &id in &ids {
                    changed |= moodboard_ui::moodboard_add(&mut self.project, id);
                }
                if changed {
                    push_undo_json(&mut self.undo, &mut self.redo, snap);
                    self.after_edit();
                }
            }
            DropTarget::Library => {
                self.library.tab = 0;
                self.library.selected = ids.last().copied();
            }
        }
    }
}

// ---- ws:canvas-handles-monitor ----
#[cfg(test)]
mod tests {
    use super::*;
    use egui::{pos2, vec2, Rect};

    /// The monitor branch is `drop_target(..) == Monitor` + `insert_at(ids, playhead, None)`: the
    /// target decision and the placement it makes are each checked here (no headless `App` exists to
    /// drive `handle_drops` itself — see the App-construction note in tools_registry_tests.rs).
    #[test]
    fn drop_onto_monitor_places_on_a_free_track_at_playhead() {
        let timeline = Rect::from_min_size(pos2(0.0, 300.0), vec2(800.0, 200.0));
        let monitor = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 300.0));
        let mood = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 500.0)); // stacked under both
        assert_eq!(drop_target(Some(pos2(400.0, 150.0)), timeline, monitor, mood), DropTarget::Monitor);
        assert_eq!(drop_target(Some(pos2(400.0, 350.0)), timeline, monitor, mood), DropTarget::Timeline);
        assert_eq!(drop_target(Some(pos2(400.0, 150.0)), timeline, Rect::NOTHING, mood), DropTarget::Moodboard);
        assert_eq!(drop_target(None, timeline, monitor, mood), DropTarget::Library);
        assert_eq!(drop_target(Some(pos2(900.0, 900.0)), timeline, monitor, mood), DropTarget::Library);
        // what the Monitor arm does: V1 is busy at the playhead, so the clip lands on a free video track
        let mut p = Project::from_media(crate::model::Asset {
            id: 0,
            path: "C:/x.mp4".into(),
            kind: ClipKind::Video,
            duration: 4.0,
            width: 320,
            height: 240,
            fps: 30.0,
            audio_streams: Vec::new(),
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
        let a = p.assets[0].id;
        let playhead = 1.0;
        let ids = p.insert_asset_clips(a, playhead, None);
        let (ti, c) = p.all_clips().find(|(_, c)| c.id == ids[0]).expect("placed");
        assert_eq!(c.start, playhead, "at the playhead");
        assert_ne!(ti, 0, "V1 already holds a clip at 1.0, so a free video track took it");
        assert_eq!(p.tracks[ti].kind, TrackKind::Video);
    }
}
