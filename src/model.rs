//! Project data model — the shared contract between UI, engine, playback and export.
//! All times are seconds (f64). Keyframe times are clip-local (seconds from `clip.start`).
//! Serialized with serde_json as the `.sedit` project format.

use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};
use std::path::Path;

pub type Id = u64;
/// Smallest clip length / trim epsilon.
pub const MIN_CLIP: f64 = 0.001;
const EPS: f64 = 1e-6;
const KEY_EPS: f64 = 1e-4;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Hash)]
pub enum TrackKind {
    Video,
    Audio,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Hash)]
pub enum ClipKind {
    Video,
    Image,
    Text,
    Audio,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default, Hash)]
pub enum BlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    Add,
    Subtract,
    Difference,
    SoftLight,
    HardLight,
    ColorDodge,
    ColorBurn,
}

impl BlendMode {
    pub const ALL: [BlendMode; 13] = [
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Overlay,
        BlendMode::Darken,
        BlendMode::Lighten,
        BlendMode::Add,
        BlendMode::Subtract,
        BlendMode::Difference,
        BlendMode::SoftLight,
        BlendMode::HardLight,
        BlendMode::ColorDodge,
        BlendMode::ColorBurn,
    ];
    pub fn name(self) -> &'static str {
        match self {
            BlendMode::Normal => "Normal",
            BlendMode::Multiply => "Multiply",
            BlendMode::Screen => "Screen",
            BlendMode::Overlay => "Overlay",
            BlendMode::Darken => "Darken",
            BlendMode::Lighten => "Lighten",
            BlendMode::Add => "Add",
            BlendMode::Subtract => "Subtract",
            BlendMode::Difference => "Difference",
            BlendMode::SoftLight => "Soft Light",
            BlendMode::HardLight => "Hard Light",
            BlendMode::ColorDodge => "Color Dodge",
            BlendMode::ColorBurn => "Color Burn",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct Keyframe {
    pub t: f64,
    pub v: f64,
}

/// A scalar property that is either constant (`value`) or linearly keyframed (`keys`, sorted by t).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Animated {
    pub value: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keys: Vec<Keyframe>,
}

impl Animated {
    pub fn new(value: f64) -> Self {
        Self { value, keys: Vec::new() }
    }
    pub fn is_animated(&self) -> bool {
        !self.keys.is_empty()
    }
    pub fn is_default(&self, def: f64) -> bool {
        !self.is_animated() && (self.value - def).abs() < EPS
    }
    /// Value at clip-local time t (linear interpolation, clamped at the ends).
    pub fn at(&self, t: f64) -> f64 {
        let k = &self.keys;
        if k.is_empty() {
            return self.value;
        }
        if t <= k[0].t {
            return k[0].v;
        }
        let last = k[k.len() - 1];
        if t >= last.t {
            return last.v;
        }
        for w in k.windows(2) {
            if t < w[1].t {
                let span = w[1].t - w[0].t;
                let f = if span > 0.0 { (t - w[0].t) / span } else { 1.0 };
                return w[0].v + (w[1].v - w[0].v) * f;
            }
        }
        last.v
    }
    pub fn key_index_at(&self, t: f64) -> Option<usize> {
        self.keys.iter().position(|k| (k.t - t).abs() < KEY_EPS)
    }
    pub fn has_key_at(&self, t: f64) -> bool {
        self.key_index_at(t).is_some()
    }
    fn insert(&mut self, t: f64, v: f64) {
        let i = self.keys.partition_point(|k| k.t < t);
        self.keys.insert(i, Keyframe { t, v });
    }
    /// Set the value at time t: upserts a keyframe when animated, otherwise sets the constant.
    pub fn set_at(&mut self, t: f64, v: f64) {
        if !self.is_animated() {
            self.value = v;
            return;
        }
        match self.key_index_at(t) {
            Some(i) => self.keys[i].v = v,
            None => self.insert(t, v),
        }
    }
    /// Add a keyframe at t holding the current value, or remove the one already there.
    pub fn toggle_key(&mut self, t: f64) {
        if let Some(i) = self.key_index_at(t) {
            let v = self.keys.remove(i).v;
            if self.keys.is_empty() {
                self.value = v;
            }
        } else {
            let v = self.at(t);
            self.insert(t, v);
        }
    }
    /// Remove all keyframes, keeping the value at t.
    pub fn clear_keys(&mut self, t: f64) {
        self.value = self.at(t);
        self.keys.clear();
    }
    /// Shift all keyframe times by dt (used when trimming/splitting).
    pub fn shift(&mut self, dt: f64) {
        for k in &mut self.keys {
            k.t += dt;
        }
    }
}

/// Text clip styling. Sizes are in project pixels (at project resolution).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TextStyle {
    pub text: String,
    pub font: String,
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    pub color: [u8; 4],
    pub outline_width: f32,
    pub outline_color: [u8; 4],
    pub shadow: bool,
    pub shadow_color: [u8; 4],
    pub shadow_x: f32,
    pub shadow_y: f32,
    pub shadow_blur: f32,
    /// 0 = left, 1 = center, 2 = right
    pub align: u8,
    pub line_spacing: f32,
    pub letter_spacing: f32,
    /// Background box behind the text; alpha 0 = none.
    pub box_color: [u8; 4],
    pub box_padding: f32,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            text: "Text".into(),
            font: "Segoe UI".into(),
            size: 72.0,
            bold: false,
            italic: false,
            color: [255, 255, 255, 255],
            outline_width: 0.0,
            outline_color: [0, 0, 0, 255],
            shadow: false,
            shadow_color: [0, 0, 0, 160],
            shadow_x: 4.0,
            shadow_y: 4.0,
            shadow_blur: 2.0,
            align: 1,
            line_spacing: 1.0,
            letter_spacing: 0.0,
            box_color: [0, 0, 0, 0],
            box_padding: 8.0,
        }
    }
}

impl TextStyle {
    /// Stable hash of every field (for render caches).
    pub fn cache_key(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.text.hash(&mut h);
        self.font.hash(&mut h);
        for f in [
            self.size,
            self.outline_width,
            self.shadow_x,
            self.shadow_y,
            self.shadow_blur,
            self.line_spacing,
            self.letter_spacing,
            self.box_padding,
        ] {
            f.to_bits().hash(&mut h);
        }
        (self.bold, self.italic, self.shadow, self.align).hash(&mut h);
        (self.color, self.outline_color, self.shadow_color, self.box_color).hash(&mut h);
        h.finish()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct AudioStreamInfo {
    /// 0-based index among the file's audio streams (ffmpeg `0:a:N`).
    pub index: usize,
    pub channels: u32,
    pub sample_rate: u32,
    pub language: String,
    pub title: String,
    pub codec: String,
}

impl AudioStreamInfo {
    pub fn label(&self) -> String {
        if !self.title.is_empty() {
            self.title.clone()
        } else if !self.language.is_empty() && self.language != "und" {
            self.language.clone()
        } else {
            format!("Audio {}", self.index + 1)
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Asset {
    pub id: Id,
    pub path: String,
    /// Video, Image or Audio (never Text).
    pub kind: ClipKind,
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    #[serde(default)]
    pub audio_streams: Vec<AudioStreamInfo>,
    #[serde(default)]
    pub codec: String,
}

impl Asset {
    pub fn name(&self) -> String {
        Path::new(&self.path).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| self.path.clone())
    }
    pub fn has_video(&self) -> bool {
        matches!(self.kind, ClipKind::Video | ClipKind::Image)
    }
}

fn tru() -> bool {
    true
}
fn a0() -> Animated {
    Animated::new(0.0)
}
fn a1() -> Animated {
    Animated::new(1.0)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Clip {
    pub id: Id,
    /// Clips sharing a non-zero link id move/split/delete together (video + its audio).
    #[serde(default)]
    pub link: Id,
    pub kind: ClipKind,
    /// Asset id (0 for Text clips).
    #[serde(default)]
    pub asset: Id,
    pub name: String,
    /// Timeline start (seconds).
    pub start: f64,
    pub duration: f64,
    /// Offset into the source media at `start`.
    #[serde(default)]
    pub src_in: f64,
    /// For Audio clips: which audio stream of the asset (0-based among audio streams).
    #[serde(default)]
    pub audio_stream: usize,
    #[serde(default = "tru")]
    pub enabled: bool,
    // --- visual properties (project pixels / degrees / 0..1) ---
    #[serde(default = "a0")]
    pub x: Animated,
    #[serde(default = "a0")]
    pub y: Animated,
    #[serde(default = "a1")]
    pub scale: Animated,
    #[serde(default = "a0")]
    pub rotation: Animated,
    #[serde(default = "a1")]
    pub opacity: Animated,
    #[serde(default)]
    pub blend: BlendMode,
    // --- audio ---
    /// Linear gain (1 = unity).
    #[serde(default = "a1")]
    pub volume: Animated,
    /// Present for Text clips.
    #[serde(default)]
    pub text: Option<TextStyle>,
}

impl Clip {
    pub fn new(id: Id, kind: ClipKind, name: impl Into<String>, start: f64, duration: f64) -> Self {
        Self {
            id,
            link: 0,
            kind,
            asset: 0,
            name: name.into(),
            start,
            duration,
            src_in: 0.0,
            audio_stream: 0,
            enabled: true,
            x: a0(),
            y: a0(),
            scale: a1(),
            rotation: a0(),
            opacity: a1(),
            blend: BlendMode::Normal,
            volume: a1(),
            text: if kind == ClipKind::Text { Some(TextStyle::default()) } else { None },
        }
    }
    pub fn end(&self) -> f64 {
        self.start + self.duration
    }
    /// True for t in [start, end).
    pub fn contains(&self, t: f64) -> bool {
        t >= self.start - EPS && t < self.end() - EPS
    }
    /// Clip-local time.
    pub fn local(&self, t: f64) -> f64 {
        t - self.start
    }
    /// Source media time at timeline time t.
    pub fn src_time(&self, t: f64) -> f64 {
        self.src_in + (t - self.start)
    }
    pub fn src_end(&self) -> f64 {
        self.src_in + self.duration
    }
    pub fn is_visual(&self) -> bool {
        self.kind != ClipKind::Audio
    }
    pub fn uses_asset(&self) -> bool {
        self.kind != ClipKind::Text
    }
    /// Any non-default visual transform/blend (disqualifies lossless export).
    pub fn has_effects(&self) -> bool {
        !self.x.is_default(0.0)
            || !self.y.is_default(0.0)
            || !self.scale.is_default(1.0)
            || !self.rotation.is_default(0.0)
            || !self.opacity.is_default(1.0)
            || self.blend != BlendMode::Normal
    }
    pub fn animated_mut(&mut self) -> [&mut Animated; 6] {
        [&mut self.x, &mut self.y, &mut self.scale, &mut self.rotation, &mut self.opacity, &mut self.volume]
    }
    pub fn animated(&self) -> [&Animated; 6] {
        [&self.x, &self.y, &self.scale, &self.rotation, &self.opacity, &self.volume]
    }
    /// (label, property) pairs for the inspector — visual ones for visual clips, volume for audio.
    pub fn props_mut(&mut self) -> Vec<(&'static str, &mut Animated)> {
        if self.is_visual() {
            vec![
                ("Position X", &mut self.x),
                ("Position Y", &mut self.y),
                ("Scale", &mut self.scale),
                ("Rotation", &mut self.rotation),
                ("Opacity", &mut self.opacity),
            ]
        } else {
            vec![("Volume", &mut self.volume)]
        }
    }
    /// Sorted, de-duplicated clip-local keyframe times (for drawing diamonds).
    pub fn key_times(&self) -> Vec<f64> {
        let mut v: Vec<f64> = self.animated().iter().flat_map(|a| a.keys.iter().map(|k| k.t)).collect();
        v.sort_by(f64::total_cmp);
        v.dedup_by(|a, b| (*a - *b).abs() < KEY_EPS);
        v
    }
    /// Split at timeline time t. `self` becomes the left part; returns the right part with `new_id`.
    /// None if t is not strictly inside the clip.
    pub fn split(&mut self, t: f64, new_id: Id) -> Option<Clip> {
        if t <= self.start + MIN_CLIP || t >= self.end() - MIN_CLIP {
            return None;
        }
        let off = t - self.start;
        let mut right = self.clone();
        right.id = new_id;
        right.start = t;
        right.duration = self.end() - t;
        right.src_in = self.src_in + off;
        for a in right.animated_mut() {
            a.shift(-off);
        }
        self.duration = off;
        Some(right)
    }
    /// Move the left edge to `new_start`, keeping the right edge fixed (slip-trim).
    pub fn trim_start(&mut self, new_start: f64) {
        // media clips can't slip before the source's first frame; images/text are unbounded
        let min_start = if matches!(self.kind, ClipKind::Video | ClipKind::Audio) {
            self.start - self.src_in
        } else {
            f64::NEG_INFINITY
        };
        let ns = new_start.max(min_start).max(0.0).min(self.end() - MIN_CLIP);
        let d = ns - self.start;
        self.start = ns;
        self.duration -= d;
        self.src_in += d;
        for a in self.animated_mut() {
            a.shift(-d);
        }
    }
    /// Move the right edge to `new_end`. `max_duration` = longest allowed duration
    /// (asset.duration - src_in for media; f64::INFINITY for text/images).
    pub fn trim_end(&mut self, new_end: f64, max_duration: f64) {
        let ne = new_end.min(self.start + max_duration).max(self.start + MIN_CLIP);
        self.duration = ne - self.start;
    }
}

fn dh() -> f32 {
    60.0
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Track {
    pub id: Id,
    pub name: String,
    pub kind: TrackKind,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub solo: bool,
    /// UI height in points.
    #[serde(default = "dh")]
    pub height: f32,
    #[serde(default)]
    pub clips: Vec<Clip>,
}

impl Track {
    pub fn new(id: Id, kind: TrackKind, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            kind,
            muted: false,
            solo: false,
            height: if kind == TrackKind::Video { 64.0 } else { 56.0 },
            clips: Vec::new(),
        }
    }
    pub fn sort(&mut self) {
        self.clips.sort_by(|a, b| a.start.total_cmp(&b.start));
    }
    pub fn end(&self) -> f64 {
        self.clips.iter().map(|c| c.end()).fold(0.0, f64::max)
    }
    /// True if [start, start+dur) is free on this track, ignoring clips in `ignore`.
    pub fn fits(&self, start: f64, dur: f64, ignore: &[Id]) -> bool {
        start >= -EPS
            && !self
                .clips
                .iter()
                .any(|c| !ignore.contains(&c.id) && c.start < start + dur - EPS && start < c.end() - EPS)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Project {
    pub version: u32,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub assets: Vec<Asset>,
    /// Order: all video tracks (V1, V2, ...) then all audio tracks (A1, A2, ...).
    pub tracks: Vec<Track>,
    pub in_point: Option<f64>,
    pub out_point: Option<f64>,
    /// Set when the project was created by opening a video file directly; enables "Save" (overwrite).
    pub source_video: Option<String>,
    next_id: Id,
}

impl Default for Project {
    fn default() -> Self {
        Self::new()
    }
}

impl Project {
    pub fn new() -> Self {
        let mut p = Self {
            version: 1,
            name: "Untitled".into(),
            width: 1920,
            height: 1080,
            fps: 30.0,
            assets: Vec::new(),
            tracks: Vec::new(),
            in_point: None,
            out_point: None,
            source_video: None,
            next_id: 0,
        };
        p.add_track(TrackKind::Video);
        p.add_track(TrackKind::Audio);
        p
    }

    /// New project sized from a media asset, with the asset laid out at t=0 (video + one audio track per stream).
    pub fn from_media(asset: Asset) -> Self {
        let mut p = Self::new();
        if asset.has_video() && asset.width > 0 && asset.height > 0 {
            p.width = asset.width;
            p.height = asset.height;
            if asset.kind == ClipKind::Video && asset.fps > 1.0 {
                p.fps = asset.fps;
            }
        }
        p.name = Path::new(&asset.path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".into());
        if asset.kind == ClipKind::Video {
            p.source_video = Some(asset.path.clone());
        }
        let aid = p.add_asset(asset);
        p.insert_asset_clips(aid, 0.0, None);
        p
    }

    pub fn new_id(&mut self) -> Id {
        self.next_id += 1;
        self.next_id
    }

    // ---------- assets ----------
    pub fn asset(&self, id: Id) -> Option<&Asset> {
        self.assets.iter().find(|a| a.id == id)
    }
    pub fn asset_by_path(&self, path: &str) -> Option<&Asset> {
        self.assets.iter().find(|a| a.path.eq_ignore_ascii_case(path))
    }
    /// Adds an asset (de-duplicated by path) and returns its id.
    pub fn add_asset(&mut self, mut a: Asset) -> Id {
        if let Some(e) = self.asset_by_path(&a.path) {
            return e.id;
        }
        a.id = self.new_id();
        let id = a.id;
        self.assets.push(a);
        id
    }
    /// Removes an asset and every clip using it.
    pub fn remove_asset(&mut self, id: Id) {
        self.assets.retain(|a| a.id != id);
        for t in &mut self.tracks {
            t.clips.retain(|c| !(c.uses_asset() && c.asset == id));
        }
    }

    // ---------- queries ----------
    pub fn duration(&self) -> f64 {
        self.tracks.iter().map(|t| t.end()).fold(0.0, f64::max)
    }
    pub fn is_empty(&self) -> bool {
        self.tracks.iter().all(|t| t.clips.is_empty())
    }
    pub fn frame_dur(&self) -> f64 {
        1.0 / self.fps.max(1.0)
    }
    pub fn snap_frame(&self, t: f64) -> f64 {
        (t * self.fps).round() / self.fps
    }
    /// (track index, clip index) of a clip.
    pub fn find(&self, id: Id) -> Option<(usize, usize)> {
        for (ti, t) in self.tracks.iter().enumerate() {
            if let Some(ci) = t.clips.iter().position(|c| c.id == id) {
                return Some((ti, ci));
            }
        }
        None
    }
    pub fn clip(&self, id: Id) -> Option<&Clip> {
        self.find(id).map(|(t, c)| &self.tracks[t].clips[c])
    }
    pub fn clip_mut(&mut self, id: Id) -> Option<&mut Clip> {
        let (t, c) = self.find(id)?;
        Some(&mut self.tracks[t].clips[c])
    }
    pub fn track_of(&self, clip: Id) -> Option<usize> {
        self.find(clip).map(|(t, _)| t)
    }
    pub fn all_clips(&self) -> impl Iterator<Item = (usize, &Clip)> {
        self.tracks.iter().enumerate().flat_map(|(i, t)| t.clips.iter().map(move |c| (i, c)))
    }
    /// All clip ids linked with `id` (including itself).
    pub fn linked(&self, id: Id) -> Vec<Id> {
        match self.clip(id) {
            Some(c) if c.link != 0 => {
                let l = c.link;
                self.all_clips().filter(|(_, c)| c.link == l).map(|(_, c)| c.id).collect()
            }
            Some(_) => vec![id],
            None => Vec::new(),
        }
    }
    /// Expand a selection to include linked clips.
    pub fn expand_links(&self, ids: &[Id]) -> Vec<Id> {
        let mut out: Vec<Id> = Vec::new();
        for &id in ids {
            for l in self.linked(id) {
                if !out.contains(&l) {
                    out.push(l);
                }
            }
        }
        out
    }
    pub fn video_tracks(&self) -> Vec<usize> {
        (0..self.tracks.len()).filter(|&i| self.tracks[i].kind == TrackKind::Video).collect()
    }
    pub fn audio_tracks(&self) -> Vec<usize> {
        (0..self.tracks.len()).filter(|&i| self.tracks[i].kind == TrackKind::Audio).collect()
    }
    /// Sorted distinct clip boundaries (plus 0) for prev/next-cut navigation.
    pub fn cut_points(&self) -> Vec<f64> {
        let mut v = vec![0.0];
        for (_, c) in self.all_clips() {
            v.push(c.start);
            v.push(c.end());
        }
        v.sort_by(f64::total_cmp);
        v.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
        v
    }
    /// Mute/solo resolution: a track is audible/visible if (no solo among its kind && !muted) || solo.
    pub fn active(&self, tidx: usize) -> bool {
        let t = &self.tracks[tidx];
        let any_solo = self.tracks.iter().any(|o| o.kind == t.kind && o.solo);
        if any_solo {
            t.solo
        } else {
            !t.muted
        }
    }
    pub fn max_clip_duration(&self, clip: &Clip) -> f64 {
        match clip.kind {
            ClipKind::Text | ClipKind::Image => f64::INFINITY,
            _ => self.asset(clip.asset).map(|a| (a.duration - clip.src_in).max(MIN_CLIP)).unwrap_or(f64::INFINITY),
        }
    }

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

    // ---------- editing ----------
    /// Place an asset on the timeline at `at`: video/image clip on a video track (preferring `video_track`)
    /// plus one linked audio clip per audio stream (stream N preferring track A(N+1)). Returns new clip ids.
    pub fn insert_asset_clips(&mut self, asset_id: Id, at: f64, video_track: Option<usize>) -> Vec<Id> {
        let Some(asset) = self.asset(asset_id).cloned() else { return Vec::new() };
        let dur = if asset.kind == ClipKind::Image { 5.0 } else { asset.duration.max(MIN_CLIP) };
        let n_parts = asset.has_video() as usize + asset.audio_streams.len();
        let link = if n_parts > 1 { self.new_id() } else { 0 };
        let mut ids = Vec::new();
        if asset.has_video() {
            let ti = self.find_free_track(TrackKind::Video, at, dur, video_track);
            let mut c = Clip::new(self.new_id(), asset.kind, asset.name(), at, dur);
            c.asset = asset.id;
            c.link = link;
            ids.push(c.id);
            self.tracks[ti].clips.push(c);
            self.tracks[ti].sort();
        }
        for (i, s) in asset.audio_streams.iter().enumerate() {
            let prefer = self.audio_tracks().get(i).copied();
            let ti = self.find_free_track(TrackKind::Audio, at, dur, prefer);
            let name =
                if asset.audio_streams.len() > 1 { format!("{} [{}]", asset.name(), s.label()) } else { asset.name() };
            let mut c = Clip::new(self.new_id(), ClipKind::Audio, name, at, dur);
            c.asset = asset.id;
            c.audio_stream = s.index;
            c.link = link;
            ids.push(c.id);
            self.tracks[ti].clips.push(c);
            self.tracks[ti].sort();
        }
        ids
    }

    /// Add a text clip on the topmost video track that has room (or a new one).
    pub fn add_text_clip(&mut self, at: f64, dur: f64) -> Id {
        let prefer = self.video_tracks().last().copied();
        let ti = self.find_free_track(TrackKind::Video, at, dur, prefer);
        let c = Clip::new(self.new_id(), ClipKind::Text, "Text", at, dur);
        let id = c.id;
        self.tracks[ti].clips.push(c);
        self.tracks[ti].sort();
        id
    }

    /// Split every clip crossing t (or only the given ids). Right halves of a linked group stay linked
    /// to each other under a fresh link id. Returns the new (right-half) ids.
    pub fn split_at(&mut self, t: f64, only: Option<&[Id]>) -> Vec<Id> {
        let mut new_ids = Vec::new();
        let mut link_map: std::collections::HashMap<Id, Id> = std::collections::HashMap::new();
        for ti in 0..self.tracks.len() {
            let mut added = Vec::new();
            for ci in 0..self.tracks[ti].clips.len() {
                let c = &self.tracks[ti].clips[ci];
                if !c.contains(t) || only.map(|o| !o.contains(&c.id)).unwrap_or(false) {
                    continue;
                }
                let old_link = c.link;
                let nid = self.new_id();
                let new_link = if old_link != 0 {
                    match link_map.get(&old_link) {
                        Some(&l) => l,
                        None => {
                            let l = self.new_id();
                            link_map.insert(old_link, l);
                            l
                        }
                    }
                } else {
                    0
                };
                if let Some(mut right) = self.tracks[ti].clips[ci].split(t, nid) {
                    right.link = new_link;
                    new_ids.push(right.id);
                    added.push(right);
                }
            }
            if !added.is_empty() {
                self.tracks[ti].clips.extend(added);
                self.tracks[ti].sort();
            }
        }
        new_ids
    }

    /// Delete clips. With `ripple`, later clips close the gap (per track, only where no other clip overlaps the gap).
    pub fn delete_clips(&mut self, ids: &[Id], ripple: bool) {
        let mut ranges: Vec<(f64, f64)> = Vec::new();
        for &id in ids {
            if let Some(c) = self.clip(id) {
                ranges.push((c.start, c.end()));
            }
        }
        for t in &mut self.tracks {
            t.clips.retain(|c| !ids.contains(&c.id));
        }
        if ripple {
            ranges.sort_by(|a, b| b.0.total_cmp(&a.0)); // right to left
            ranges.dedup_by(|a, b| (a.0 - b.0).abs() < EPS && (a.1 - b.1).abs() < EPS);
            for (a, b) in ranges {
                self.close_gap(a, b);
            }
        }
    }

    /// Shift clips starting at/after `b` left by (b-a) on every track where [a,b) is free.
    fn close_gap(&mut self, a: f64, b: f64) {
        let len = b - a;
        if len <= 0.0 {
            return;
        }
        for t in &mut self.tracks {
            if !t.fits(a, len, &[]) {
                continue;
            }
            for c in &mut t.clips {
                if c.start >= b - EPS {
                    c.start -= len;
                }
            }
        }
    }

    /// Remove everything in [a,b) on all tracks and close the gap.
    pub fn ripple_delete_range(&mut self, a: f64, b: f64) {
        if b <= a + EPS {
            return;
        }
        self.split_at(a, None);
        self.split_at(b, None);
        let ids: Vec<Id> =
            self.all_clips().filter(|(_, c)| c.start >= a - EPS && c.end() <= b + EPS).map(|(_, c)| c.id).collect();
        self.delete_clips(&ids, false);
        self.close_gap(a, b);
    }

    /// Keep only [a,b), moving it to start at 0. Clears in/out points.
    pub fn trim_to_range(&mut self, a: f64, b: f64) {
        let end = self.duration().max(b) + 1.0;
        self.ripple_delete_range(b, end);
        self.ripple_delete_range(0.0, a);
        self.in_point = None;
        self.out_point = None;
    }

    /// Move clips by dt seconds; clips on tracks of `track_kind` also move by `dtrack` tracks within their kind.
    /// All-or-nothing: returns false (and changes nothing) if any destination is blocked or out of range.
    pub fn move_clips(&mut self, ids: &[Id], dt: f64, dtrack: i32, track_kind: Option<TrackKind>) -> bool {
        // (clip id, from track, to track, new start, dur)
        let mut plan: Vec<(Id, usize, usize, f64, f64)> = Vec::new();
        for &id in ids {
            let Some((ti, ci)) = self.find(id) else { return false };
            let c = &self.tracks[ti].clips[ci];
            let kind = self.tracks[ti].kind;
            let mut to = ti;
            if dtrack != 0 && track_kind == Some(kind) {
                let list = if kind == TrackKind::Video { self.video_tracks() } else { self.audio_tracks() };
                let pos = list.iter().position(|&x| x == ti).unwrap() as i32 + dtrack;
                if pos < 0 || pos >= list.len() as i32 {
                    return false;
                }
                to = list[pos as usize];
            }
            let ns = c.start + dt;
            if ns < -EPS {
                return false;
            }
            plan.push((id, ti, to, ns.max(0.0), c.duration));
        }
        // moved clips must not overlap each other (sorted per destination track: neighbours suffice)
        // or any clip that stays put
        plan.sort_by(|a, b| a.2.cmp(&b.2).then(a.3.total_cmp(&b.3)));
        for w in plan.windows(2) {
            if w[0].2 == w[1].2 && w[1].3 < w[0].3 + w[0].4 - EPS {
                return false;
            }
        }
        let moved: std::collections::HashSet<Id> = ids.iter().copied().collect();
        for (_, _, to, ns, dur) in &plan {
            let blocked = self.tracks[*to]
                .clips
                .iter()
                .any(|c| !moved.contains(&c.id) && c.start < ns + dur - EPS && *ns < c.end() - EPS);
            if blocked {
                return false;
            }
        }
        for (id, from, to, ns, _) in plan {
            let ci = self.tracks[from].clips.iter().position(|c| c.id == id).unwrap();
            let mut c = self.tracks[from].clips.remove(ci);
            c.start = ns;
            self.tracks[to].clips.push(c);
        }
        for t in &mut self.tracks {
            t.sort();
        }
        true
    }

    /// Set `enabled` on the clips (linked clips too).
    pub fn set_enabled(&mut self, ids: &[Id], enabled: bool) {
        for id in self.expand_links(ids) {
            if let Some(c) = self.clip_mut(id) {
                c.enabled = enabled;
            }
        }
    }
    /// Unlink the clips if any is linked; otherwise link them together.
    pub fn toggle_link(&mut self, ids: &[Id]) {
        let linked = ids.iter().any(|&id| self.clip(id).map(|c| c.link != 0).unwrap_or(false));
        let link = if linked { 0 } else { self.new_id() };
        for &id in ids {
            if let Some(c) = self.clip_mut(id) {
                c.link = link;
            }
        }
    }

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
            .chain(p.all_clips().map(|(_, c)| c.id.max(c.link)))
            .max()
            .unwrap_or(0);
        p.next_id = p.next_id.max(max_id);
        // hand-edited files: keep every value in the range the UI can produce (fps 0 would make NaN times)
        if !(p.fps >= 1.0 && p.fps <= 1000.0) {
            p.fps = 30.0;
        }
        p.width = p.width.max(16);
        p.height = p.height.max(16);
        for t in &mut p.tracks {
            t.clips.retain(|c| c.start >= 0.0 && c.duration.is_finite() && c.duration > 0.0);
            t.sort();
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(id: Id, dur: f64, streams: usize) -> Asset {
        Asset {
            id,
            path: format!("C:/v{id}.mp4"),
            kind: ClipKind::Video,
            duration: dur,
            width: 1280,
            height: 720,
            fps: 30.0,
            audio_streams: (0..streams)
                .map(|i| AudioStreamInfo { index: i, channels: 2, sample_rate: 48000, ..Default::default() })
                .collect(),
            codec: "h264".into(),
        }
    }

    #[test]
    fn animated_interp() {
        let mut a = Animated::new(5.0);
        assert_eq!(a.at(3.0), 5.0);
        a.toggle_key(0.0);
        a.set_at(2.0, 15.0);
        assert_eq!(a.keys.len(), 2);
        assert!((a.at(1.0) - 10.0).abs() < 1e-9);
        assert_eq!(a.at(-1.0), 5.0);
        assert_eq!(a.at(9.0), 15.0);
        a.toggle_key(2.0);
        a.toggle_key(0.0);
        assert!(!a.is_animated());
        assert_eq!(a.value, 5.0);
    }

    #[test]
    fn from_media_layout() {
        let p = Project::from_media(asset(0, 10.0, 2));
        assert_eq!(p.width, 1280);
        assert_eq!(p.tracks.len(), 3); // V1 A1 A2
        assert_eq!(p.tracks[0].clips.len(), 1);
        assert_eq!(p.tracks[2].clips[0].audio_stream, 1);
        let v = &p.tracks[0].clips[0];
        assert_eq!(p.linked(v.id).len(), 3);
        assert_eq!(p.duration(), 10.0);
        assert!(p.source_video.is_some());
    }

    #[test]
    fn split_delete_ripple() {
        let mut p = Project::from_media(asset(0, 10.0, 1));
        let new = p.split_at(4.0, None);
        assert_eq!(new.len(), 2);
        assert_eq!(p.tracks[0].clips.len(), 2);
        assert!((p.tracks[0].clips[1].src_in - 4.0).abs() < 1e-9);
        // right halves linked to each other but not to left halves
        assert_eq!(p.linked(new[0]).len(), 2);
        assert!(p.linked(new[0]).contains(&new[1]));
        let left = p.tracks[0].clips[0].id;
        assert!(!p.linked(left).contains(&new[0]));
        // ripple delete the left halves
        let ids = p.linked(left);
        p.delete_clips(&ids, true);
        assert_eq!(p.tracks[0].clips.len(), 1);
        assert!((p.tracks[0].clips[0].start).abs() < 1e-9);
        assert!((p.tracks[1].clips[0].start).abs() < 1e-9);
        assert!((p.duration() - 6.0).abs() < 1e-9);
    }

    #[test]
    fn trim_to_range() {
        let mut p = Project::from_media(asset(0, 10.0, 1));
        p.trim_to_range(2.0, 5.0);
        assert!((p.duration() - 3.0).abs() < 1e-9);
        let c = &p.tracks[0].clips[0];
        assert!((c.src_in - 2.0).abs() < 1e-9);
        assert!(c.start.abs() < 1e-9);
    }

    #[test]
    fn move_and_fit() {
        let mut p = Project::from_media(asset(0, 10.0, 1));
        let a2 = asset(1, 3.0, 0);
        let aid = p.add_asset(a2);
        let ids = p.insert_asset_clips(aid, 10.0, Some(0));
        assert_eq!(ids.len(), 1);
        assert_eq!(p.track_of(ids[0]), Some(0));
        // can't move onto the first clip
        assert!(!p.move_clips(&ids, -5.0, 0, None));
        // can move right
        assert!(p.move_clips(&ids, 5.0, 0, None));
        assert!((p.clip(ids[0]).unwrap().start - 15.0).abs() < 1e-9);
        // move to a new video track: none exists -> false
        assert!(!p.move_clips(&ids, 0.0, 1, Some(TrackKind::Video)));
        p.add_track(TrackKind::Video);
        assert!(p.move_clips(&ids, -15.0, 1, Some(TrackKind::Video)));
        assert_eq!(p.track_of(ids[0]), Some(1));
    }

    #[test]
    fn json_roundtrip() {
        let mut p = Project::from_media(asset(0, 10.0, 2));
        p.add_text_clip(1.0, 2.0);
        let s = p.to_json();
        let q = Project::from_json(&s).unwrap();
        assert_eq!(q.tracks.len(), p.tracks.len());
        assert_eq!(q.to_json(), s);
        let mut q = q;
        let id = q.new_id();
        assert!(id > p.next_id);
    }

    #[test]
    fn trims() {
        let mut c = Clip::new(1, ClipKind::Video, "c", 2.0, 5.0);
        c.src_in = 1.0;
        c.opacity.toggle_key(1.0);
        c.trim_start(0.0); // clamped to start - src_in = 1.0
        assert!((c.start - 1.0).abs() < 1e-9);
        assert!((c.src_in).abs() < 1e-9);
        assert!((c.duration - 6.0).abs() < 1e-9);
        assert!((c.opacity.keys[0].t - 2.0).abs() < 1e-9);
        c.trim_end(100.0, 10.0 - c.src_in);
        assert!((c.duration - 10.0).abs() < 1e-9);
        // images/text are unbounded on both sides
        let mut img = Clip::new(2, ClipKind::Image, "i", 5.0, 5.0);
        img.trim_start(3.0);
        assert!((img.start - 3.0).abs() < 1e-9);
        assert!((img.duration - 7.0).abs() < 1e-9);
    }

    #[test]
    fn move_many() {
        let mut p = Project::new();
        let ids: Vec<Id> = (0..4).map(|i| p.add_text_clip(i as f64 * 2.0, 2.0)).collect(); // V1: [0,2)[2,4)[4,6)[6,8)
        let blocker = p.add_text_clip(10.0, 1.0);
        assert!(p.move_clips(&ids, 1.0, 0, None)); // adjacent moved clips don't block each other
        assert!(!p.move_clips(&ids, 2.0, 0, None)); // last one would hit the blocker
        assert!(!p.move_clips(&[ids[1]], 1.0, 0, None)); // onto a stationary clip
        assert!((p.clip(ids[0]).unwrap().start - 1.0).abs() < 1e-9);
        assert!((p.clip(blocker).unwrap().start - 10.0).abs() < 1e-9);
    }

    #[test]
    fn from_json_sanitizes() {
        let mut p = Project::from_media(asset(0, 10.0, 1));
        p.add_text_clip(12.0, 1.0);
        let s = p.to_json().replace("\"fps\": 30.0", "\"fps\": 0.0").replace("\"width\": 1280", "\"width\": 0");
        let q = Project::from_json(&s).unwrap();
        assert_eq!(q.fps, 30.0);
        assert!(q.width >= 16);
        assert!(q.snap_frame(1.0).is_finite());
        let mut bad = Project::new();
        bad.tracks[0].clips.push(Clip::new(99, ClipKind::Text, "t", 0.0, -1.0));
        assert!(Project::from_json(&bad.to_json()).unwrap().is_empty());
    }

    #[test]
    fn save_is_atomic() {
        let dir = std::env::temp_dir().join("simple-editor-model-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("p.sedit");
        let p = Project::from_media(asset(0, 10.0, 1));
        p.save(&path).unwrap();
        p.save(&path).unwrap(); // overwrites
        assert!(!dir.join("p.sedit.tmp").exists());
        assert_eq!(Project::load(&path).unwrap().to_json(), p.to_json());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
