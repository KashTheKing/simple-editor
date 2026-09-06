use crate::model::*;

impl Project {
    // ---------- shapes / adjustment layers ----------
    /// Add a shape clip on the topmost video track that has room.
    pub fn add_shape_clip(&mut self, kind: ShapeKind, at: f64, dur: f64) -> Id {
        let prefer = self.video_tracks().last().copied();
        let ti = self.find_free_track(TrackKind::Video, at, dur, prefer);
        let mut c = Clip::new(self.new_id(), ClipKind::Shape, kind.name(), at, dur);
        c.shape = Some(ShapeStyle::new(kind));
        let id = c.id;
        self.tracks[ti].clips.push(c);
        self.tracks[ti].sort();
        id
    }
    /// Add an adjustment (automation) layer above everything at `at`.
    pub fn add_adjustment_clip(&mut self, at: f64, dur: f64) -> Id {
        let prefer = self.video_tracks().last().copied();
        let ti = self.find_free_track(TrackKind::Video, at, dur, prefer);
        let c = Clip::new(self.new_id(), ClipKind::Adjustment, "Adjustment", at, dur);
        let id = c.id;
        self.tracks[ti].clips.push(c);
        self.tracks[ti].sort();
        id
    }
    /// Add a container clip pair (video slot + audio slot linked together) at `at`.
    pub fn add_container_clip(&mut self, at: f64, dur: f64) -> (Id, Id) {
        let dur = dur.max(MIN_CLIP);
        let link = self.new_id();
        let prefer_v = self.video_tracks().last().copied();
        let vti = self.find_free_track(TrackKind::Video, at, dur, prefer_v);
        let mut vc = Clip::new(self.new_id(), ClipKind::Video, "Container", at, dur);
        vc.link = link;
        vc.container = true;
        let vid = vc.id;
        self.tracks[vti].clips.push(vc);
        self.tracks[vti].sort();

        let prefer_a = self.audio_tracks().first().copied();
        let ati = self.find_free_track(TrackKind::Audio, at, dur, prefer_a);
        let mut ac = Clip::new(self.new_id(), ClipKind::Audio, "Container Audio", at, dur);
        ac.link = link;
        ac.container = true;
        let aid = ac.id;
        self.tracks[ati].clips.push(ac);
        self.tracks[ati].sort();

        (vid, aid)
    }
    /// Insert asset clips marked as containers (slots).
    pub fn add_container_from_asset(&mut self, asset_id: Id, at: f64, video_track: Option<usize>) -> Vec<Id> {
        let ids = self.insert_asset_clips(asset_id, at, video_track);
        for &id in &ids {
            if let Some(c) = self.clip_mut(id) {
                c.container = true;
            }
        }
        ids
    }
    /// Replace the media of a container clip. Effects, transforms, keyframes, transitions and timeline duration are kept.
    pub fn replace_container_media(&mut self, clip_id: Id, new_asset_id: Id) -> bool {
        let Some(asset) = self.asset(new_asset_id).cloned() else { return false };
        let Some(clip) = self.clip_mut(clip_id) else { return false };
        if !clip.container {
            return false;
        }
        clip.asset = asset.id;
        clip.src_in = 0.0;
        if clip.kind == ClipKind::Audio {
            clip.audio_stream = 0;
        } else if asset.kind == ClipKind::Image {
            clip.kind = ClipKind::Image;
        } else if asset.kind == ClipKind::Video {
            clip.kind = ClipKind::Video;
        }
        let base_name = asset.name();
        clip.name = if !clip.container_label.is_empty() {
            format!("{} [{}]", base_name, clip.container_label)
        } else {
            base_name
        };
        true
    }
    /// Replace a video container clip and any linked audio container clips with the new asset.
    pub fn replace_container_pair(&mut self, video_id: Id, new_asset_id: Id) -> bool {
        let Some(asset) = self.asset(new_asset_id).cloned() else { return false };
        let Some(vclip) = self.clip(video_id).cloned() else { return false };
        if !vclip.container {
            return false;
        }
        let link = vclip.link;
        if !self.replace_container_media(video_id, new_asset_id) {
            return false;
        }
        if link != 0 {
            let mut stream_idx = 0;
            let linked_ids: Vec<Id> =
                self.all_clips().filter(|(_, c)| c.link == link && c.id != video_id).map(|(_, c)| c.id).collect();
            for aid in linked_ids {
                if let Some(c) = self.clip_mut(aid) {
                    if c.kind == ClipKind::Audio && c.container {
                        c.asset = asset.id;
                        c.src_in = 0.0;
                        if stream_idx < asset.audio_streams.len() {
                            c.audio_stream = asset.audio_streams[stream_idx].index;
                            stream_idx += 1;
                        } else {
                            c.audio_stream = 0;
                        }
                        let base_name = if asset.audio_streams.len() > 1 {
                            format!(
                                "{} [{}]",
                                asset.name(),
                                asset.audio_streams.get(c.audio_stream).map(|s| s.label()).unwrap_or_default()
                            )
                        } else {
                            asset.name()
                        };
                        c.name = if !c.container_label.is_empty() {
                            format!("{} [{}]", base_name, c.container_label)
                        } else {
                            base_name
                        };
                    }
                }
            }
        }
        true
    }
    /// Convert clips to containers (slots). Linked clips are included.
    pub fn make_container(&mut self, clip_ids: &[Id]) {
        let all = self.expand_links(clip_ids);
        for id in all {
            if let Some(c) = self.clip_mut(id) {
                c.container = true;
            }
        }
    }
    /// Remove container flag from clips. Linked clips are included.
    pub fn unmake_container(&mut self, clip_ids: &[Id]) {
        let all = self.expand_links(clip_ids);
        for id in all {
            if let Some(c) = self.clip_mut(id) {
                c.container = false;
                c.container_label.clear();
            }
        }
    }
}
