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
                let video_count = self.video_tracks().len();
                let mut t = Track::new(id, TrackKind::Video, "Subtitles");
                t.ripple = Track::default_ripple(TrackKind::Video, video_count);
                self.tracks.push(t);
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

    // ---- ws:transcript-captions ----
    // ---------- transcripts ----------
    /// The persisted word timings of `clip` (timeline seconds), if it was ever transcribed.
    pub fn transcript(&self, clip: Id) -> Option<&Transcript> {
        self.transcripts.iter().find(|t| t.clip == clip)
    }
    /// Write (or replace) the transcript of `clip`. Empty `words` removes the entry instead of keeping
    /// a hollow one around. The one write path behind the Subtitles pane's transcribe, the clip
    /// menu's "Transcribe…", `transcribe.run`/`media.transcribe` and `transcript.set`.
    pub fn set_transcript(&mut self, clip: Id, mut words: Vec<(f64, f64, String)>) {
        words.retain(|w| !w.2.trim().is_empty());
        words.sort_by(|a, b| a.0.total_cmp(&b.0));
        self.transcripts.retain(|t| t.clip != clip);
        if !words.is_empty() {
            self.transcripts.push(Transcript { clip, words });
        }
    }
    /// Ripple-cut timeline `ranges` out of `clip` (its link group goes too), then drag the cues, the
    /// project markers and EVERY transcript's words along with the cut — the single cut path behind
    /// the double-take cutter, filler removal, the Transcript section's Delete and the
    /// `transcript.cut_words`/`transcript.remove_fillers` tools. Overlapping/abutting ranges are
    /// merged first; ranges are clamped to the clip. Ripple follows the owning track's `ripple` flag
    /// (a position-locked track just loses the pieces and nothing downstream moves — same rule as
    /// Delete); a locked track refuses (0). Returns how many clip pieces went.
    ///
    /// ponytail: refuses (0) on a clip with a keyframed speed ramp — its words sit on one linear
    /// (offset, scale) map (`engine::transcribe::retime`) and would drift between the keys. Upgrade
    /// path: per-segment retime.
    pub fn cut_word_ranges(&mut self, clip: Id, ranges: &[(f64, f64)]) -> usize {
        let Some(ti) = self.track_of(clip) else { return 0 };
        if self.locked_of(ti) {
            return 0;
        }
        let Some(c) = self.clip(clip) else { return 0 };
        if c.speed_curve.is_animated() {
            return 0;
        }
        let (lo, hi) = (c.start, c.end());
        let ripple = self.tracks[ti].ripple.unwrap_or(false);
        let mut r: Vec<(f64, f64)> =
            ranges.iter().map(|&(a, b)| (a.max(lo), b.min(hi))).filter(|&(a, b)| b > a + EPS).collect();
        r.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut merged: Vec<(f64, f64)> = Vec::new();
        for (a, b) in r {
            match merged.last_mut() {
                Some(l) if a <= l.1 + EPS => l.1 = l.1.max(b),
                _ => merged.push((a, b)),
            }
        }
        if merged.is_empty() {
            return 0;
        }
        let cuts: Vec<f64> = merged.iter().flat_map(|&(a, b)| [a, b]).collect();
        let n = self.auto_cut(&[clip], &cuts, &merged, ripple);
        if n == 0 {
            return 0;
        }
        let editing = self.editing;
        if ripple {
            use crate::engine::transcribe::ripple_time;
            self.subtitles.retain_mut(|c| {
                let Some(a) = ripple_time(c.start, &merged) else { return false };
                let b = ripple_time(c.end, &merged).unwrap_or(a + (c.end - c.start));
                (c.start, c.end) = (a, b.max(a + 0.1));
                true
            });
            self.sort_cues();
            self.markers.retain_mut(|m| {
                if m.sequence != editing {
                    return true;
                }
                let Some(a) = ripple_time(m.t, &merged) else { return false };
                if m.duration > 0.0 {
                    let b = ripple_time(m.t + m.duration, &merged).unwrap_or(a);
                    m.duration = (b - a).max(0.0);
                }
                m.t = a;
                true
            });
            self.sort_markers();
            for tr in &mut self.transcripts {
                tr.words.retain_mut(|w| match (ripple_time(w.0, &merged), ripple_time(w.1, &merged)) {
                    (Some(x), Some(y)) => {
                        (w.0, w.1) = (x, y.max(x));
                        true
                    }
                    _ => false,
                });
            }
        } else {
            // nothing moved: only the words that sat inside a removed span are gone
            for tr in &mut self.transcripts {
                tr.words.retain(|w| !merged.iter().any(|&(a, b)| w.0 >= a && w.0 < b));
            }
        }
        self.transcripts.retain(|t| !t.words.is_empty());
        n
    }
}

// ---- ws:transcript-captions ----
/// Word search across every transcript: `(clip id, word index, word start)` for every word run whose
/// normalized text contains the normalized query (case/punctuation-insensitive; a multi-word query
/// matches that many consecutive words). Feeds the Transcript section's search box and
/// `transcript.search`.
///
/// ponytail: plain substring over `normalize()`'d tokens, no phonetic/fuzzy matching — ScriptSync-lite
/// is explicitly skipped project-wide; upgrade path is Needleman-Wunsch alignment if ever asked for.
pub fn transcript_hits(transcripts: &[Transcript], q: &str) -> Vec<(Id, usize, f64)> {
    use crate::engine::transcribe::normalize;
    let qt = normalize(q);
    if qt.is_empty() {
        return Vec::new();
    }
    let n = qt.len();
    let needle = qt.join(" ");
    let mut out = Vec::new();
    for tr in transcripts {
        let toks: Vec<String> = tr.words.iter().map(|w| normalize(&w.2).join(" ")).collect();
        for i in 0..tr.words.len() {
            if i + n > tr.words.len() {
                break;
            }
            let run = toks[i..i + n].join(" ");
            if run.contains(&needle) {
                out.push((tr.clip, i, tr.words[i].0));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Asset, AudioStreamInfo};

    /// A clip with an asset, on V1 + A1, running 0..10 s of the source.
    fn clip_project() -> (Project, Id) {
        let mut p = Project::new();
        let aid = p.add_asset(Asset {
            id: 0,
            path: "C:/take.mp4".into(),
            kind: ClipKind::Video,
            duration: 10.0,
            width: 320,
            height: 240,
            fps: 30.0,
            audio_streams: vec![AudioStreamInfo { channels: 2, sample_rate: 48000, ..Default::default() }],
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
        p.insert_asset_clips(aid, 0.0, Some(0));
        let id = p.tracks[0].clips[0].id;
        (p, id)
    }

    fn words(list: &[(f64, f64, &str)]) -> Vec<(f64, f64, String)> {
        list.iter().map(|(a, b, w)| (*a, *b, w.to_string())).collect()
    }

    #[test]
    fn transcripts_survive_json_round_trip() {
        let (mut p, id) = clip_project();
        p.set_transcript(id, words(&[(0.0, 0.4, "Hello"), (0.5, 0.9, "there")]));
        p.subtitle_anim = SubtitleAnim::Highlight([255, 200, 0, 255]);
        let back = Project::from_json(&p.to_json()).expect("parses");
        assert_eq!(back.transcripts, p.transcripts, "words identical after a round trip");
        assert_eq!(back.subtitle_anim, p.subtitle_anim);
        // an old .sedit with no transcripts key loads with an empty Vec
        let mut json: serde_json::Value = serde_json::from_str(&p.to_json()).unwrap();
        json.as_object_mut().unwrap().remove("transcripts");
        json.as_object_mut().unwrap().remove("subtitle_anim");
        let old = Project::from_json(&json.to_string()).expect("old project parses");
        assert!(old.transcripts.is_empty());
        assert_eq!(old.subtitle_anim, SubtitleAnim::None);
        // set_transcript replaces, and an empty list removes
        p.set_transcript(id, words(&[(1.0, 1.5, "again")]));
        assert_eq!(p.transcripts.len(), 1);
        assert_eq!(p.transcript(id).unwrap().words[0].2, "again");
        p.set_transcript(id, Vec::new());
        assert!(p.transcript(id).is_none());
    }

    #[test]
    fn cut_word_ranges_shifts_cues_markers_words() {
        let (mut p, id) = clip_project();
        p.set_transcript(
            id,
            words(&[(0.0, 0.5, "one"), (1.0, 1.5, "um"), (2.0, 2.5, "two"), (3.0, 3.5, "uh"), (4.0, 4.5, "three")]),
        );
        p.add_cue(0.0, 0.8, "one");
        p.add_cue(1.0, 1.6, "um"); // inside the first cut: goes
        p.add_cue(4.0, 5.0, "three"); // after both cuts: moves up by 1.0
        let inside = p.add_marker(1.2, "inside");
        let after = p.add_marker(6.0, "after");
        let n = p.cut_word_ranges(id, &[(1.0, 1.5), (3.0, 3.5)]);
        assert_eq!(n, 4, "two spans, each a video piece plus its linked audio piece");
        assert!((p.duration() - 9.0).abs() < 1e-6, "1 s gone: {}", p.duration());
        assert!(p.markers.iter().all(|m| m.id != inside), "a marker inside the cut goes with it");
        let m = p.markers.iter().find(|m| m.id == after).expect("survivor");
        assert!((m.t - 5.0).abs() < 1e-6, "the later marker moved up: {}", m.t);
        assert_eq!(p.subtitles.len(), 2);
        assert!((p.subtitles[1].start - 3.0).abs() < 1e-6, "cue moved: {:?}", p.subtitles[1]);
        let tr = p.transcript(id).expect("transcript kept");
        let text: Vec<&str> = tr.words.iter().map(|w| w.2.as_str()).collect();
        assert_eq!(text, vec!["one", "two", "three"]);
        assert!((tr.words[1].0 - 1.5).abs() < 1e-6 && (tr.words[2].0 - 3.0).abs() < 1e-6, "{:?}", tr.words);
        // overlapping/abutting ranges merge; empty and out-of-clip ranges are nothing
        assert_eq!(p.cut_word_ranges(id, &[]), 0);
        assert_eq!(p.cut_word_ranges(id, &[(50.0, 60.0)]), 0);
        // a locked track refuses
        p.tracks[0].locked = true;
        assert_eq!(p.cut_word_ranges(id, &[(0.0, 0.5)]), 0);
    }

    #[test]
    fn cut_word_ranges_on_a_position_locked_track_leaves_the_gap() {
        let (mut p, id) = clip_project();
        for t in &mut p.tracks {
            t.ripple = Some(false);
        }
        p.set_transcript(id, words(&[(0.0, 0.5, "one"), (1.0, 1.5, "um"), (2.0, 2.5, "two")]));
        p.add_cue(2.0, 3.0, "two");
        let n = p.cut_word_ranges(id, &[(1.0, 1.5)]);
        assert_eq!(n, 2, "the video and its linked audio piece");
        assert!((p.duration() - 10.0).abs() < 1e-6, "nothing moved: {}", p.duration());
        assert!((p.subtitles[0].start - 2.0).abs() < 1e-6);
        let tr = p.transcript(id).unwrap();
        assert_eq!(tr.words.len(), 2, "the word inside the gap is gone");
        assert!((tr.words[1].0 - 2.0).abs() < 1e-6, "the later word stayed put");
    }

    #[test]
    fn transcript_hits_finds_words_across_clips() {
        let a = Transcript { clip: 1, words: words(&[(0.0, 0.4, "Hello,"), (0.5, 0.9, "world"), (1.0, 1.4, "you")]) };
        let b = Transcript { clip: 2, words: words(&[(5.0, 5.4, "you"), (5.5, 5.9, "know")]) };
        let all = [a, b];
        assert_eq!(transcript_hits(&all, "WORLD"), vec![(1, 1, 0.5)], "case-insensitive, one clip only");
        assert_eq!(transcript_hits(&all, "hello"), vec![(1, 0, 0.0)], "punctuation ignored");
        assert_eq!(transcript_hits(&all, "you"), vec![(1, 2, 1.0), (2, 0, 5.0)], "both clips");
        assert_eq!(transcript_hits(&all, "you know"), vec![(2, 0, 5.0)], "a phrase needs consecutive words");
        assert!(transcript_hits(&all, "nothing").is_empty());
        assert!(transcript_hits(&all, "   ").is_empty());
    }
}
