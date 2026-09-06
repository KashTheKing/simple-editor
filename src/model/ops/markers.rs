use crate::model::*;

impl Project {
    // ---------- labels ----------
    /// Effective colour label of a clip (its own, else its asset's). 0 = none.
    pub fn clip_label(&self, clip: &Clip) -> u8 {
        if clip.label != 0 {
            return clip.label;
        }
        if clip.uses_asset() {
            return self.asset(clip.asset).map(|a| a.label).unwrap_or(0);
        }
        0
    }

    // ---------- labels ----------
    /// Colour of label index `idx` (1-based; 0 or unknown = None).
    pub fn label_color(&self, idx: u8) -> Option<[u8; 3]> {
        (idx > 0).then(|| self.labels.get(idx as usize - 1).map(|l| l.color)).flatten()
    }
    pub fn label_name(&self, idx: u8) -> &str {
        if idx == 0 {
            return "None";
        }
        self.labels.get(idx as usize - 1).map(|l| l.name.as_str()).unwrap_or("None")
    }
    /// Add a label; returns its 1-based index.
    pub fn add_label(&mut self, name: impl Into<String>, color: [u8; 3]) -> u8 {
        self.labels.push(Label { name: name.into(), color });
        self.labels.len() as u8
    }
    /// Remove a label; clips/assets/markers using it fall back to "none", higher indices shift down.
    pub fn remove_label(&mut self, idx: u8) {
        if idx == 0 || idx as usize > self.labels.len() {
            return;
        }
        self.labels.remove(idx as usize - 1);
        let fix = |l: &mut u8| {
            if *l == idx {
                *l = 0;
            } else if *l > idx {
                *l -= 1;
            }
        };
        for a in &mut self.assets {
            fix(&mut a.label);
        }
        for m in &mut self.markers {
            fix(&mut m.label);
        }
        let mut all: Vec<&mut Track> = self.tracks.iter_mut().collect();
        if let Some(st) = &mut self.main_stash {
            all.extend(st.tracks.iter_mut());
        }
        for sq in &mut self.sequences {
            all.extend(sq.tracks.iter_mut());
        }
        for t in all {
            for c in &mut t.clips {
                fix(&mut c.label);
                for m in &mut c.markers {
                    fix(&mut m.label);
                }
            }
        }
    }

    // ---------- markers ----------
    /// Stamped with `self.editing` so the marker only shows on the sequence (or main timeline) it was
    /// created on — see `markers_ui::rows`.
    pub fn add_marker(&mut self, t: f64, name: impl Into<String>) -> Id {
        let id = self.new_id();
        self.markers.push(Marker {
            id,
            t: t.max(0.0),
            name: name.into(),
            sequence: self.editing,
            ..Default::default()
        });
        self.sort_markers();
        id
    }
    /// Move a project-level marker to the nearest clip EDGE (start or end) on the current sequence's
    /// tracks (`self.tracks`) — a marker just before a clip's end must snap forward to that end, not
    /// jump back to the clip's start. No-op if the marker or a clip doesn't exist.
    pub fn snap_marker_to_nearest_clip(&mut self, id: Id) {
        let Some(t) = self.markers.iter().find(|m| m.id == id).map(|m| m.t) else { return };
        let Some(nearest) = self
            .all_clips()
            .flat_map(|(_, c)| [c.start, c.end()])
            .min_by(|a, b| (a - t).abs().total_cmp(&(b - t).abs()))
        else {
            return;
        };
        if let Some(m) = self.markers.iter_mut().find(|m| m.id == id) {
            m.t = nearest.max(0.0);
        }
        self.sort_markers();
    }
    /// Convert a project-level marker into a clip-local marker on the nearest clip (by clip start),
    /// keeping its name/note/label/icon and converting `t` from timeline-absolute to clip-local
    /// (clamped to the clip's duration). Returns `false` (no-op) without a marker or a clip to attach to.
    pub fn link_marker_to_closest_clip(&mut self, id: Id) -> bool {
        let Some(pos) = self.markers.iter().position(|m| m.id == id) else { return false };
        let t = self.markers[pos].t;
        let Some(clip_id) = self
            .all_clips()
            .map(|(_, c)| (c.id, c.start))
            .min_by(|(_, a), (_, b)| (a - t).abs().total_cmp(&(b - t).abs()))
            .map(|(id, _)| id)
        else {
            return false;
        };
        let m = self.markers.remove(pos);
        let Some(c) = self.clip_mut(clip_id) else {
            self.markers.push(m); // clip vanished mid-lookup (shouldn't happen) — put it back
            self.sort_markers();
            return false;
        };
        let local_t = (t - c.start).clamp(0.0, c.duration);
        c.markers.push(Marker { t: local_t, sequence: None, ..m });
        c.markers.sort_by(|a, b| a.t.total_cmp(&b.t));
        true
    }
    pub fn remove_marker(&mut self, id: Id) {
        self.markers.retain(|m| m.id != id);
        for t in &mut self.tracks {
            for c in &mut t.clips {
                c.markers.retain(|m| m.id != id);
            }
        }
    }
    pub fn marker_mut(&mut self, id: Id) -> Option<&mut Marker> {
        if let Some(i) = self.markers.iter().position(|m| m.id == id) {
            return self.markers.get_mut(i);
        }
        self.tracks.iter_mut().flat_map(|t| t.clips.iter_mut()).flat_map(|c| c.markers.iter_mut()).find(|m| m.id == id)
    }
    pub fn sort_markers(&mut self) {
        self.markers.sort_by(|a, b| a.t.total_cmp(&b.t));
    }
    /// Add a marker on a clip (time is clip-local).
    pub fn add_clip_marker(&mut self, clip: Id, local_t: f64, name: impl Into<String>) -> Option<Id> {
        let id = self.new_id();
        let name = name.into();
        let c = self.clip_mut(clip)?;
        c.markers.push(Marker { id, t: local_t.clamp(0.0, c.duration), name, ..Default::default() });
        c.markers.sort_by(|a, b| a.t.total_cmp(&b.t));
        Some(id)
    }
    /// Every marker in timeline time: project markers plus clip markers offset by their clip.
    pub fn markers_in_timeline(&self) -> Vec<(Id, f64, f64, String, u8)> {
        let mut v: Vec<(Id, f64, f64, String, u8)> =
            self.markers.iter().map(|m| (m.id, m.t, m.duration, m.name.clone(), m.label)).collect();
        for (_, c) in self.all_clips() {
            for m in &c.markers {
                v.push((m.id, c.start + m.t, m.duration, m.name.clone(), m.label));
            }
        }
        v.sort_by(|a, b| a.1.total_cmp(&b.1));
        v
    }
}
