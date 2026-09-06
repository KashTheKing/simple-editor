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
        Ok(p)
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
