//! ---- ws:zones-markers ----
//! Effect zones (`model::zone`) and marker categories (`Marker.category`, `Project.marker_categories`).

use crate::model::*;

/// The shortest a zone gets.
pub const MIN_ZONE: f64 = 0.1;

impl Project {
    // ---------- effect zones ----------

    /// Lay an effect zone over timeline [t, t + dur]: `track` None = every video track, Some(id) = that
    /// track's clips only. Made on the sequence open now. Returns its id.
    pub fn add_zone(&mut self, t: f64, dur: f64, track: Option<Id>, effects: Vec<Effect>) -> Id {
        let id = self.new_id();
        let track = track.filter(|tid| self.tracks.iter().any(|tr| tr.id == *tid && tr.kind == TrackKind::Video));
        self.zones.push(EffectZone {
            id,
            t: t.max(0.0),
            dur: dur.max(MIN_ZONE),
            track,
            effects,
            sequence: self.editing,
            ..Default::default()
        });
        id
    }

    /// Delete zone `id`.
    pub fn remove_zone(&mut self, id: Id) -> bool {
        let n = self.zones.len();
        self.zones.retain(|z| z.id != id);
        self.zones.len() != n
    }

    pub fn zone(&self, id: Id) -> Option<&EffectZone> {
        self.zones.iter().find(|z| z.id == id)
    }

    pub fn zone_mut(&mut self, id: Id) -> Option<&mut EffectZone> {
        self.zones.iter_mut().find(|z| z.id == id)
    }

    /// Move / resize zone `id` to [t, t + dur] (clamped to >= 0 and >= `MIN_ZONE`).
    pub fn set_zone_range(&mut self, id: Id, t: f64, dur: f64) -> bool {
        let Some(z) = self.zone_mut(id) else { return false };
        z.t = t.max(0.0);
        z.dur = dur.max(MIN_ZONE);
        true
    }

    /// Where a new zone goes: In/Out when both are set, else the selected clips' span (those on `track`
    /// when given), else five seconds from the playhead. (start, length)
    pub fn zone_range(&self, playhead: f64, selection: &[Id], track: Option<Id>) -> (f64, f64) {
        if let (Some(a), Some(b)) = (self.in_point, self.out_point) {
            if b > a {
                return (a, b - a);
            }
        }
        let clips: Vec<&Clip> = selection
            .iter()
            .filter_map(|id| self.find(*id))
            .filter(|(ti, _)| track.is_none_or(|t| self.tracks[*ti].id == t))
            .map(|(ti, ci)| &self.tracks[ti].clips[ci])
            .collect();
        if !clips.is_empty() {
            let a = clips.iter().map(|c| c.start).fold(f64::INFINITY, f64::min);
            let b = clips.iter().map(|c| c.end()).fold(0.0, f64::max);
            return (a, (b - a).max(MIN_ZONE));
        }
        (playhead.max(0.0), 5.0)
    }

    /// The zones of the timeline open now (main timeline or the open sequence), in list order.
    pub fn zones_here(&self) -> impl Iterator<Item = &EffectZone> {
        self.zones.iter().filter(move |z| z.sequence == self.editing)
    }

    // ---------- marker categories ----------

    /// Marker `id` (a project marker or a clip marker).
    pub fn marker(&self, id: Id) -> Option<&Marker> {
        self.markers
            .iter()
            .chain(self.tracks.iter().flat_map(|t| t.clips.iter()).flat_map(|c| c.markers.iter()))
            .find(|m| m.id == id)
    }

    /// Colour of marker category `name` (None for "" or an unknown name).
    pub fn category_color(&self, name: &str) -> Option<[u8; 3]> {
        self.marker_categories.iter().find(|c| !name.is_empty() && c.name.eq_ignore_ascii_case(name)).map(|c| c.color)
    }

    /// Add a marker category (no-op when one of that name exists). Returns its index (0-based).
    pub fn add_marker_category(&mut self, name: impl Into<String>, color: Option<[u8; 3]>) -> usize {
        let name = name.into();
        if let Some(i) = self.marker_categories.iter().position(|c| c.name.eq_ignore_ascii_case(&name)) {
            return i;
        }
        // a new one takes the next label colour nobody uses yet (grey once they are all taken)
        let color = color.unwrap_or_else(|| {
            LABEL_COLORS
                .iter()
                .map(|(_, c)| *c)
                .find(|c| !self.marker_categories.iter().any(|m| m.color == *c))
                .unwrap_or([128, 128, 128])
        });
        self.marker_categories.push(Label { name, color });
        self.marker_categories.len() - 1
    }

    /// Set marker `id`'s category ("" = none). An unknown name becomes a new category. True when the
    /// marker exists.
    pub fn set_marker_category(&mut self, id: Id, category: &str) -> bool {
        let category = category.trim();
        let name = match self.marker_categories.iter().find(|c| c.name.eq_ignore_ascii_case(category)) {
            Some(c) => c.name.clone(),
            None if category.is_empty() => String::new(),
            None => {
                let i = self.add_marker_category(category, None);
                self.marker_categories[i].name.clone()
            }
        };
        match self.marker_mut(id) {
            Some(m) => {
                m.category = name;
                true
            }
            None => false,
        }
    }

    /// Rename / recolour / remove category `index` (0-based). A rename carries its markers along; a
    /// removal leaves them uncategorised.
    pub fn edit_marker_category(&mut self, index: usize, name: Option<&str>, color: Option<[u8; 3]>, remove: bool) -> bool {
        let Some(old) = self.marker_categories.get(index).map(|c| c.name.clone()) else { return false };
        let new = if remove { String::new() } else { name.map(str::to_string).unwrap_or_else(|| old.clone()) };
        if remove {
            self.marker_categories.remove(index);
        } else {
            let c = &mut self.marker_categories[index];
            c.name = new.clone();
            if let Some(col) = color {
                c.color = col;
            }
        }
        if new != old {
            let fix = |m: &mut Marker| {
                if m.category == old {
                    m.category = new.clone();
                }
            };
            self.markers.iter_mut().for_each(fix);
            let mut all: Vec<&mut Track> = self.tracks.iter_mut().collect();
            if let Some(st) = &mut self.main_stash {
                all.extend(st.tracks.iter_mut());
            }
            for sq in &mut self.sequences {
                all.extend(sq.tracks.iter_mut());
            }
            for t in all {
                for c in &mut t.clips {
                    c.markers.iter_mut().for_each(fix);
                }
            }
        }
        true
    }

    /// The next (or, `forward` false, previous) marker time strictly after (before) `from` on the timeline
    /// open now - project markers of this sequence and clip markers - of `category` (None = any; Some("")
    /// = uncategorised ones).
    pub fn marker_step(&self, from: f64, forward: bool, category: Option<&str>) -> Option<f64> {
        let keep = |m: &Marker| category.is_none_or(|c| m.category.eq_ignore_ascii_case(c));
        let mut times: Vec<f64> = self.markers.iter().filter(|m| m.sequence == self.editing && keep(m)).map(|m| m.t).collect();
        for (_, c) in self.all_clips() {
            times.extend(c.markers.iter().filter(|m| keep(m)).map(|m| c.start + m.t));
        }
        const EPS: f64 = 1e-4;
        if forward {
            times.into_iter().filter(|&t| t > from + EPS).min_by(f64::total_cmp)
        } else {
            times.into_iter().filter(|&t| t < from - EPS).max_by(f64::total_cmp)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zones_add_move_remove_and_survive_save_load() {
        let mut p = Project::new();
        let v1 = p.tracks[p.video_tracks()[0]].id;
        let mut fx = Effect::new(EffectKind::Blur);
        fx.params[0].keys.push(Keyframe { t: 0.0, v: 0.0, ease: Ease::Linear });
        fx.params[0].keys.push(Keyframe { t: 2.0, v: 20.0, ease: Ease::Linear });
        let a = p.add_zone(30.0, 15.0, None, vec![fx]);
        let b = p.add_zone(1.0, 0.0, Some(v1), vec![Effect::new(EffectKind::Grayscale)]);
        assert_eq!(p.zone(b).unwrap().dur, MIN_ZONE, "never shorter than MIN_ZONE");
        assert_eq!(p.zone(b).unwrap().track, Some(v1));
        // an audio track is not a zone's track: it falls back to the whole timeline
        let a1 = p.tracks[p.audio_tracks()[0]].id;
        let c = p.add_zone(0.0, 1.0, Some(a1), Vec::new());
        assert_eq!(p.zone(c).unwrap().track, None);
        assert!(p.set_zone_range(a, 31.0, 14.0));
        let back = Project::from_json(&p.to_json()).unwrap();
        assert_eq!(back.zones, p.zones, "zones round-trip");
        assert_eq!(back.zone(a).unwrap().effects[0].params[0].keys.len(), 2, "keyframes survive");
        assert!(p.remove_zone(a) && !p.remove_zone(a));
        assert_eq!(p.zones.len(), 2);
    }

    #[test]
    fn a_new_zone_covers_in_out_else_the_selection_else_five_seconds() {
        let mut p = Project::new();
        let v1 = p.video_tracks()[0];
        p.tracks[v1].clips.push(Clip::new(1, ClipKind::Video, "a", 2.0, 3.0));
        p.next_id = 10;
        assert_eq!(p.zone_range(7.0, &[], None), (7.0, 5.0));
        assert_eq!(p.zone_range(7.0, &[1], None), (2.0, 3.0));
        assert_eq!(p.zone_range(7.0, &[1], Some(999)), (7.0, 5.0), "not that track's clip");
        (p.in_point, p.out_point) = (Some(30.0), Some(45.0));
        assert_eq!(p.zone_range(7.0, &[1], None), (30.0, 15.0));
    }

    #[test]
    fn categories_round_trip_and_old_projects_load_uncategorised() {
        let mut p = Project::new();
        let m = p.add_marker(1.0, "cue");
        assert!(p.set_marker_category(m, "music cue"), "matches case-insensitively");
        assert_eq!(p.marker(m).unwrap().category, "Music cue");
        let n = p.add_marker(2.0, "x");
        p.set_marker_category(n, "Sound design");
        assert!(p.category_color("Sound design").is_some(), "an unknown name becomes a category");
        let back = Project::from_json(&p.to_json()).unwrap();
        assert_eq!(back.marker(m).unwrap().category, "Music cue");
        assert_eq!(back.marker_categories, p.marker_categories);
        // a project saved before categories: no `category` on markers, no `marker_categories` list
        let mut v: serde_json::Value = serde_json::from_str(&p.to_json()).unwrap();
        v.as_object_mut().unwrap().remove("marker_categories");
        for mk in v["markers"].as_array_mut().unwrap() {
            mk.as_object_mut().unwrap().remove("category");
        }
        let old = Project::from_json(&v.to_string()).unwrap();
        assert!(old.markers.iter().all(|m| m.category.is_empty()), "old markers are uncategorised");
        assert_eq!(old.marker_categories, default_marker_categories(), "and the built-in types are there");
        // a rename carries the markers along, a removal leaves them uncategorised
        let i = p.marker_categories.iter().position(|c| c.name == "Music cue").unwrap();
        p.edit_marker_category(i, Some("Song"), None, false);
        assert_eq!(p.marker(m).unwrap().category, "Song");
        p.edit_marker_category(i, None, None, true);
        assert_eq!(p.marker(m).unwrap().category, "");
    }

    #[test]
    fn marker_step_filters_by_category() {
        let mut p = Project::new();
        for (t, c) in [(1.0, "Todo"), (2.0, "Fix"), (3.0, "Todo"), (4.0, "")] {
            let id = p.add_marker(t, "m");
            p.set_marker_category(id, c);
        }
        assert_eq!(p.marker_step(1.0, true, None), Some(2.0));
        assert_eq!(p.marker_step(1.0, true, Some("Todo")), Some(3.0));
        assert_eq!(p.marker_step(3.0, false, Some("todo")), Some(1.0));
        assert_eq!(p.marker_step(3.0, true, Some("Todo")), None);
        assert_eq!(p.marker_step(0.0, true, Some("")), Some(4.0), "uncategorised ones");
    }
}
