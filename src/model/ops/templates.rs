use crate::model::*;

impl Project {
    // ---------- templates ----------
    /// Place a saved group of clips (times relative to the group start) at `at`. Assets are re-added by
    /// path (ids remapped); clip/link ids are fresh; tracks chosen by kind in order (new ones when blocked).
    pub fn place_clips(&mut self, clips: Vec<Clip>, assets: Vec<Asset>, at: f64) -> Vec<Id> {
        let mut asset_map: std::collections::HashMap<Id, Id> = std::collections::HashMap::new();
        for a in assets {
            let old = a.id;
            let new = self.add_asset(a);
            asset_map.insert(old, new);
        }
        let mut link_map: std::collections::HashMap<Id, Id> = std::collections::HashMap::new();
        let mut out = Vec::new();
        for mut c in clips {
            c.id = self.new_id();
            // clip markers carry ids too: a copy that kept them would shadow the original in `marker_mut`
            for i in 0..c.markers.len() {
                c.markers[i].id = self.new_id();
            }
            if c.link != 0 {
                let l = *link_map.entry(c.link).or_insert_with(|| 0);
                c.link = if l == 0 {
                    let nl = self.new_id();
                    link_map.insert(c.link, nl);
                    nl
                } else {
                    l
                };
            }
            if c.uses_asset() && c.asset != 0 {
                match asset_map.get(&c.asset) {
                    Some(&a) => c.asset = a,
                    None => continue,
                }
            }
            if c.kind == ClipKind::Sequence && self.sequence(c.sequence).is_none() {
                continue;
            }
            c.start += at;
            let kind = if c.kind == ClipKind::Audio { TrackKind::Audio } else { TrackKind::Video };
            let ti = self.find_free_track(kind, c.start, c.duration, None);
            out.push(c.id);
            self.tracks[ti].clips.push(c);
            self.tracks[ti].sort();
        }
        out
    }
}
