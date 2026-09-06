use crate::model::*;

impl Project {
    // ---------- tracks ----------
    /// Adds a track of `kind` (video tracks stay before audio tracks) and returns its index.
    pub fn add_track(&mut self, kind: TrackKind) -> usize {
        let n = self.tracks.iter().filter(|t| t.kind == kind).count() + 1;
        let id = self.new_id();
        let name = format!("{}{}", if kind == TrackKind::Video { "V" } else { "A" }, n);
        let track = Track::new(id, kind, name);
        let idx = match kind {
            TrackKind::Video => self.video_tracks().last().map(|i| i + 1).unwrap_or(0),
            TrackKind::Audio => self.tracks.len(),
        };
        self.tracks.insert(idx, track);
        idx
    }
    pub fn remove_track(&mut self, idx: usize) {
        if idx < self.tracks.len() {
            self.tracks.remove(idx);
            self.rename_tracks();
        }
    }
    fn rename_tracks(&mut self) {
        let (mut v, mut a) = (0, 0);
        for t in &mut self.tracks {
            match t.kind {
                TrackKind::Video => {
                    v += 1;
                    t.name = format!("V{v}");
                }
                TrackKind::Audio => {
                    a += 1;
                    t.name = format!("A{a}");
                }
            }
        }
    }
    /// First track of `kind` (preferring `prefer`) where [start,start+dur) is free; adds one if needed.
    pub fn find_free_track(&mut self, kind: TrackKind, start: f64, dur: f64, prefer: Option<usize>) -> usize {
        if let Some(p) = prefer {
            if p < self.tracks.len() && self.tracks[p].kind == kind && self.tracks[p].fits(start, dur, &[]) {
                return p;
            }
        }
        let list = if kind == TrackKind::Video { self.video_tracks() } else { self.audio_tracks() };
        for i in list {
            if self.tracks[i].fits(start, dur, &[]) {
                return i;
            }
        }
        self.add_track(kind)
    }
}
