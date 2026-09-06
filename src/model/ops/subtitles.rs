use crate::model::*;

impl Project {
    // ---------- subtitles ----------
    pub fn cue_at(&self, t: f64) -> Option<&Cue> {
        self.subtitles.iter().find(|c| t >= c.start && t < c.end)
    }
    pub fn add_cue(&mut self, start: f64, end: f64, text: impl Into<String>) -> Id {
        let id = self.new_id();
        self.subtitles.push(Cue { id, start, end: end.max(start + MIN_CLIP), text: text.into() });
        self.sort_cues();
        id
    }
    pub fn remove_cue(&mut self, id: Id) {
        self.subtitles.retain(|c| c.id != id);
    }
    /// Split cue `id` at `t` (both halves keep the text, like splitting a clip); the new right-half id,
    /// or None when `t` is outside the cue or too close to an edge for two readable halves.
    pub fn split_cue(&mut self, id: Id, t: f64) -> Option<Id> {
        let c = self.subtitles.iter_mut().find(|c| c.id == id)?;
        if t < c.start + 0.05 || t > c.end - 0.05 {
            return None;
        }
        let (end, text) = (c.end, c.text.clone());
        c.end = t;
        Some(self.add_cue(t, end, text))
    }
    /// Convert cues into editable Text clips on a topmost "Subtitles" video track, styled and placed
    /// like the burn-in. `only` limits it to those cue ids; converted cues are removed. Returns how many.
    pub fn cues_to_text_clips(&mut self, only: Option<&[Id]>) -> usize {
        let take: Vec<Cue> =
            self.subtitles.iter().filter(|c| only.is_none_or(|o| o.contains(&c.id))).cloned().collect();
        if take.is_empty() {
            return 0;
        }
        let ti = match self.tracks.iter().position(|t| t.kind == TrackKind::Video && t.name == "Subtitles") {
            Some(i) => i,
            None => {
                let id = self.new_id();
                self.tracks.push(Track::new(id, TrackKind::Video, "Subtitles"));
                self.tracks.len() - 1
            }
        };
        // bottom-centred like the burn-in; the box height is estimated as one line of text
        let y = self.height as f64 / 2.0 - self.subtitle_margin as f64 - self.subtitle_style.size as f64 * 0.75;
        for cue in &take {
            let mut c =
                Clip::new(self.new_id(), ClipKind::Text, "Subtitle", cue.start, (cue.end - cue.start).max(MIN_CLIP));
            let mut style = self.subtitle_style.clone();
            style.text.clone_from(&cue.text);
            c.text = Some(style);
            c.y = Animated::new(y);
            self.tracks[ti].clips.push(c);
        }
        self.tracks[ti].sort();
        let ids: Vec<Id> = take.iter().map(|c| c.id).collect();
        self.subtitles.retain(|c| !ids.contains(&c.id));
        take.len()
    }
    pub fn sort_cues(&mut self) {
        self.subtitles.sort_by(|a, b| a.start.total_cmp(&b.start));
    }
}
