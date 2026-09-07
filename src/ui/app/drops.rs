use super::*;

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
        let on_timeline = pos.map(|p| self.timeline.lanes_rect.contains(p)).unwrap_or(false);
        // one-frame-stale, like `lanes_rect` above — see `MoodboardState::content_rect`'s doc comment
        let on_moodboard = pos.map(|p| self.moodboard.content_rect.contains(p)).unwrap_or(false);
        if on_timeline {
            let p = pos.unwrap();
            let mut t = self.timeline.time_at(p.x).max(0.0);
            if self.settings.snap {
                t = self.project.snap_frame(t);
            }
            let track = self.timeline.track_at(p.y, &self.project);
            let vt = track.filter(|&i| self.project.tracks[i].kind == TrackKind::Video);
            // ---- ws:source-monitor ----
            // the drop-modifier table: Ctrl = Splice, Alt = Overwrite (replace edit on a clip body),
            // Shift = Place on Top, none = Place
            let mode = DropMode::from_modifiers(ctx.input(|i| i.modifiers));
            self.place_assets(&ids, t, vt, mode);
            self.after_edit();
        } else if on_moodboard {
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
        } else {
            self.library.tab = 0;
            self.library.selected = ids.last().copied();
        }
    }
}
