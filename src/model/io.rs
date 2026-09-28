use crate::model::*;
use std::path::Path;

impl Project {
    // ---------- persistence ----------
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
    pub fn from_json(s: &str) -> Result<Self, String> {
        let mut p: Project = serde_json::from_str(s).map_err(|e| e.to_string())?;
        let max_id = p
            .assets
            .iter()
            .map(|a| a.id)
            .chain(p.tracks.iter().map(|t| t.id))
            .chain(p.tracks.iter().flat_map(|t| t.transitions.iter().map(|x| x.id)))
            .chain(p.subtitles.iter().map(|c| c.id))
            .chain(p.markers.iter().map(|m| m.id))
            .chain(p.buses.iter().map(|b| b.id))
            .chain(p.all_clips().flat_map(|(_, c)| c.markers.iter().map(|m| m.id)))
            .chain(p.all_clips().filter_map(|(_, c)| c.graph.as_ref()).flat_map(|g| g.nodes.iter().map(|n| n.id)))
            .chain(p.notes.iter().map(|n| n.id))
            .chain(p.sequences.iter().map(|s| s.id))
            .chain(p.sequences.iter().flat_map(|s| s.tracks.iter().map(|t| t.id)))
            .chain(
                p.sequences.iter().flat_map(|s| s.tracks.iter().flat_map(|t| t.clips.iter().map(|c| c.id.max(c.link)))),
            )
            .chain(
                p.main_stash
                    .iter()
                    .flat_map(|s| s.tracks.iter().flat_map(|t| t.clips.iter().map(|c| c.id.max(c.link)))),
            )
            .chain(p.all_clips().map(|(_, c)| c.id.max(c.link)))
            .max()
            .unwrap_or(0);
        fn plan_max(items: &[PlanItem]) -> Id {
            items.iter().map(|i| i.id.max(plan_max(&i.children))).max().unwrap_or(0)
        }
        p.next_id = p.next_id.max(max_id).max(plan_max(&p.plan));
        // a note migrated from the old flat-string format (or a hand-edited one) has no real id yet
        for i in 0..p.notes.len() {
            if p.notes[i].id == 0 {
                p.notes[i].id = p.new_id();
            }
        }
        if p.editing.is_some_and(|id| p.sequence(id).is_none()) {
            p.editing = None;
        }
        // hand-edited files: keep every value in the range the UI can produce (fps 0 would make NaN times)
        if !(p.fps >= 1.0 && p.fps <= 1000.0) {
            p.fps = 30.0;
        }
        p.width = p.width.max(16);
        p.height = p.height.max(16);
        for t in &mut p.tracks {
            t.clips.retain(|c| c.start >= 0.0 && c.duration.is_finite() && c.duration > 0.0);
            for c in &mut t.clips {
                if !(c.speed.is_finite() && c.speed > 0.0) {
                    c.speed = 1.0;
                }
                c.speed_curve.keys.retain(|k| k.t.is_finite() && k.v.is_finite() && k.v > 0.0);
                if !c.speed_curve.is_animated() {
                    c.speed_curve.value = c.speed; // older files have no curve at all
                }
                for e in &mut c.effects {
                    // older files / edited files: make sure every parameter exists
                    while e.params.len() < e.kind.params().len() {
                        let d = e.kind.params()[e.params.len()].default;
                        e.params.push(Animated::new(d));
                    }
                }
            }
        }
        p.subtitles.retain(|c| c.start.is_finite() && c.end.is_finite() && c.end > c.start);
        if p.labels.is_empty() {
            p.labels = default_labels();
        }
        p.markers.retain(|m| m.t.is_finite() && m.t >= 0.0);
        p.moodboard.retain(|m| p.assets.iter().any(|a| a.id == m.asset));
        p.sort_markers();
        p.tidy();
        p.sort_cues();
        // ---- ws:registries-schema-hooks ----
        // a file with no `ripple` field (or a hand-edited one) never keeps a track at None: resolve
        // through the same per-kind-index rule every construction site uses. Every independent track
        // list gets this (the main `tracks`, each nested sequence's own tracks, and `main_stash`'s
        // tracks when a sequence was open at save time) - each with its own V1/A1 counters, since a
        // sequence's V1 is not the main timeline's V1.
        fn resolve_ripple(tracks: &mut [Track]) {
            let (mut vi, mut ai) = (0usize, 0usize);
            for t in tracks {
                let kind = t.kind;
                let idx = if kind == TrackKind::Video {
                    vi += 1;
                    vi - 1
                } else {
                    ai += 1;
                    ai - 1
                };
                if t.ripple.is_none() {
                    t.ripple = Track::default_ripple(kind, idx);
                }
            }
        }
        resolve_ripple(&mut p.tracks);
        for s in &mut p.sequences {
            resolve_ripple(&mut s.tracks);
        }
        if let Some(st) = &mut p.main_stash {
            resolve_ripple(&mut st.tracks);
        }
        // an older (or missing/default) version is silently brought forward - every field it lacks
        // already resolved a default above; a NEWER version is left alone so `newer_than_app` can
        // still tell the caller (a hard downgrade would be the only real data-loss risk, and this
        // never removes fields, only adds them)
        if p.version < Self::VERSION {
            p.version = Self::VERSION;
        }
        Ok(p)
    }
    /// The `.sedit` schema version this build writes and reads without a migration. A file saved by a
    /// newer build (`version > VERSION`) still loads - unknown fields are simply dropped on the next
    /// save - but the caller should toast a warning (see `newer_than_app`); this is not itself an error.
    pub const VERSION: u32 = 2;
    /// True when `self.version` is newer than this build understands (see `VERSION`). Unused outside
    /// tests this wave - the caller that toasts it (`App::open_project`) is outside this workstream's
    /// files; this only exposes the bool.
    #[allow(dead_code)]
    pub fn newer_than_app(&self) -> bool {
        self.version > Self::VERSION
    }
    /// Writes beside the target then renames, so a failed write can't destroy the previous save.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        std::fs::write(&tmp, self.to_json())?;
        std::fs::rename(&tmp, path)
    }
    pub fn load(path: &Path) -> Result<Self, String> {
        let s = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        Self::from_json(&s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `.sedit` with no `ripple` field on any track (every project saved before this workstream)
    /// resolves through the same per-kind-index rule as a fresh construction, and its version is
    /// brought forward.
    #[test]
    fn ripple_resolves_on_load() {
        let mut p = Project::new(); // V1, A1
        p.add_track(TrackKind::Video); // V2
        let mut json: serde_json::Value = serde_json::from_str(&p.to_json()).unwrap();
        json["version"] = serde_json::json!(1);
        for t in json["tracks"].as_array_mut().unwrap() {
            t.as_object_mut().unwrap().remove("ripple");
        }
        let loaded = Project::from_json(&json.to_string()).unwrap();
        // video tracks stay before audio tracks: [V1, V2, A1], not insertion order
        assert_eq!(loaded.tracks[0].ripple, Some(true), "V1");
        assert_eq!(loaded.tracks[1].ripple, Some(false), "V2 (second video track)");
        assert_eq!(loaded.tracks[2].ripple, Some(true), "A1");
        assert_eq!(loaded.version, Project::VERSION, "brought forward to the current version");
    }

    /// The same resolution must also reach a nested sequence's own tracks and, when the file was saved
    /// while a sequence was open (so the real main timeline sits in `main_stash`), that stash's tracks
    /// too - each with its own independent V1/A1 counters. Regression test for a bug where the loop only
    /// walked `p.tracks`.
    #[test]
    fn ripple_resolves_on_load_for_sequences_and_main_stash() {
        // a closed sequence: its tracks live in `p.sequences[0].tracks`
        let mut p = Project::new(); // main V1, A1
        let seq_id = p.new_sequence("Seq", 1920, 1080, 30.0); // sequence's own V1, A1
        let mut json: serde_json::Value = serde_json::from_str(&p.to_json()).unwrap();
        json["version"] = serde_json::json!(1);
        for t in json["sequences"][0]["tracks"].as_array_mut().unwrap() {
            t.as_object_mut().unwrap().remove("ripple");
        }
        let loaded = Project::from_json(&json.to_string()).unwrap();
        let seq = loaded.sequence(seq_id).unwrap();
        assert_eq!(seq.tracks[0].ripple, Some(true), "sequence's own V1");
        assert_eq!(seq.tracks[1].ripple, Some(true), "sequence's own A1");
        // the sequence is open when saved: the real main timeline's tracks are stashed in `main_stash`
        let mut p2 = Project::new();
        let seq_id2 = p2.new_sequence("Seq2", 1920, 1080, 30.0);
        assert!(p2.open_sequence(seq_id2));
        let mut json2: serde_json::Value = serde_json::from_str(&p2.to_json()).unwrap();
        json2["version"] = serde_json::json!(1);
        for t in json2["main_stash"]["tracks"].as_array_mut().unwrap() {
            t.as_object_mut().unwrap().remove("ripple");
        }
        let loaded2 = Project::from_json(&json2.to_string()).unwrap();
        let stash = loaded2.main_stash.as_ref().expect("sequence was open, so main_stash must round-trip");
        assert_eq!(stash.tracks[0].ripple, Some(true), "stashed main V1");
        assert_eq!(stash.tracks[1].ripple, Some(true), "stashed main A1");
    }

    #[test]
    fn newer_project_warns() {
        let p = Project::new();
        let mut json: serde_json::Value = serde_json::from_str(&p.to_json()).unwrap();
        json["version"] = serde_json::json!(Project::VERSION + 1);
        let loaded = Project::from_json(&json.to_string()).unwrap();
        assert!(loaded.newer_than_app(), "a version newer than this build must be flagged");
        assert_eq!(loaded.version, Project::VERSION + 1, "the newer version itself is preserved, not clamped");
    }
}
