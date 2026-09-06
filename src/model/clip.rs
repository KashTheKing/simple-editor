use crate::model::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Clip {
    pub id: Id,
    /// Clips sharing a non-zero link id move/split/delete together (video + its audio).
    #[serde(default)]
    pub link: Id,
    pub kind: ClipKind,
    /// True when this clip is a container (slot) whose media can be replaced without losing effects/transforms.
    #[serde(default)]
    pub container: bool,
    /// User-facing label for the container slot (e.g. "Main Shot", "B-Roll 1"). Empty = unnamed.
    #[serde(default)]
    pub container_label: String,
    /// Asset id (0 for Text clips).
    #[serde(default)]
    pub asset: Id,
    pub name: String,
    /// Timeline start (seconds).
    pub start: f64,
    pub duration: f64,
    /// Earliest source time used by the clip (the source window is [src_in, src_in + duration*speed)).
    #[serde(default)]
    pub src_in: f64,
    /// For Audio clips: which audio stream of the asset (0-based among audio streams).
    #[serde(default)]
    pub audio_stream: usize,
    /// For Sequence clips: the nested timeline id.
    #[serde(default)]
    pub sequence: Id,
    #[serde(default = "crate::model::tru")]
    pub enabled: bool,
    /// Colour label shown on the timeline: 0 = inherit the asset's label, 1..=8 = LABEL_COLORS index + 1.
    #[serde(default)]
    pub label: u8,
    // --- retime ---
    /// Playback rate (1 = normal, 2 = twice as fast, 0.5 = half speed). Always > 0.
    #[serde(default = "crate::model::one")]
    pub speed: f64,
    /// Keyframed speed ramp (curve editor). Only consulted while it has keys — the constant `speed`
    /// above still owns the source window and the duration.
    #[serde(default = "crate::model::a1")]
    pub speed_curve: Animated,
    /// Play the source window backwards.
    #[serde(default)]
    pub reverse: bool,
    /// Freeze frame: show/hold the source frame at this source time for the whole clip (audio is silent).
    #[serde(default)]
    pub freeze: Option<f64>,
    // --- visual properties (project pixels / degrees / 0..1) ---
    #[serde(default = "crate::model::a0")]
    pub x: Animated,
    #[serde(default = "crate::model::a0")]
    pub y: Animated,
    #[serde(default = "crate::model::a1")]
    pub scale: Animated,
    /// Additional horizontal-only multiplier on top of `scale` (1 = no-op). Groundwork for independent
    /// non-uniform scaling; final horizontal factor is `scale.at(lt) * scale_x.at(lt)`.
    #[serde(default = "crate::model::a1")]
    pub scale_x: Animated,
    /// Additional vertical-only multiplier on top of `scale` (1 = no-op). See `scale_x`.
    #[serde(default = "crate::model::a1")]
    pub scale_y: Animated,
    #[serde(default = "crate::model::a0")]
    pub rotation: Animated,
    #[serde(default = "crate::model::a1")]
    pub opacity: Animated,
    #[serde(default)]
    pub blend: BlendMode,
    /// Effect stack, applied in order before blending.
    #[serde(default)]
    pub effects: Vec<Effect>,
    // --- audio ---
    /// Linear gain (1 = unity).
    #[serde(default = "crate::model::a1")]
    pub volume: Animated,
    /// -1 = left … 0 = centre … 1 = right.
    #[serde(default = "crate::model::a0")]
    pub pan: Animated,
    /// Fade in/out lengths in seconds (gain ramps at the clip edges).
    #[serde(default)]
    pub fade_in: f64,
    #[serde(default)]
    pub fade_out: f64,
    /// Which bus this audio clip feeds (0 = its track's bus).
    #[serde(default)]
    pub bus: Id,
    /// Present for Text clips.
    #[serde(default)]
    pub text: Option<TextStyle>,
    /// Present for Shape clips.
    #[serde(default)]
    pub shape: Option<ShapeStyle>,
    /// Limits where the whole clip is visible.
    #[serde(default)]
    pub mask: Option<Mask>,
    /// Node graph; when present it replaces `effects` for rendering.
    #[serde(default)]
    pub graph: Option<NodeGraph>,
    /// Clip-local markers.
    #[serde(default)]
    pub markers: Vec<Marker>,
}

impl Clip {
    pub fn new(id: Id, kind: ClipKind, name: impl Into<String>, start: f64, duration: f64) -> Self {
        Self {
            id,
            link: 0,
            kind,
            container: false,
            container_label: String::new(),
            asset: 0,
            name: name.into(),
            start,
            duration,
            src_in: 0.0,
            audio_stream: 0,
            sequence: 0,
            enabled: true,
            label: 0,
            speed: 1.0,
            speed_curve: a1(),
            reverse: false,
            freeze: None,
            x: a0(),
            y: a0(),
            scale: a1(),
            scale_x: a1(),
            scale_y: a1(),
            rotation: a0(),
            opacity: a1(),
            blend: BlendMode::Normal,
            effects: Vec::new(),
            volume: a1(),
            pan: a0(),
            fade_in: 0.0,
            fade_out: 0.0,
            bus: 0,
            text: if kind == ClipKind::Text { Some(TextStyle::default()) } else { None },
            shape: if kind == ClipKind::Shape { Some(ShapeStyle::default()) } else { None },
            mask: None,
            graph: None,
            markers: Vec::new(),
        }
    }
    /// True when this container has no media (asset == 0 for video/image/audio).
    pub fn is_empty_container(&self) -> bool {
        self.container && self.asset == 0
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
    /// Gain/opacity multiplier of the fade in/out ramps at clip-local time `lt`. Time is clamped
    /// into the clip so virtual transition extensions hold the edge value (mixer rule).
    pub fn fade_mult(&self, lt: f64) -> f64 {
        let lt = lt.clamp(0.0, self.duration);
        let mut g = 1.0;
        if self.fade_in > 0.0 && lt < self.fade_in {
            g *= (lt / self.fade_in).clamp(0.0, 1.0);
        }
        if self.fade_out > 0.0 && lt > self.duration - self.fade_out {
            g *= ((self.duration - lt) / self.fade_out).clamp(0.0, 1.0);
        }
        g
    }
    /// Length of the source window in source seconds.
    pub fn src_len(&self) -> f64 {
        self.duration * self.speed
    }
    /// Playback rate at clip-local time `l` (the ramp when keyframed, else the constant).
    pub fn rate(&self, l: f64) -> f64 {
        if self.speed_curve.is_animated() {
            self.speed_curve.at(l).clamp(0.01, 100.0)
        } else {
            self.speed
        }
    }
    /// Source media time at timeline time t (speed, reverse and freeze applied).
    pub fn src_time(&self, t: f64) -> f64 {
        if let Some(f) = self.freeze {
            return f;
        }
        // ponytail: a keyframed ramp samples the rate at the clip-local time instead of integrating it, so
        // the source lands where the keys say at the keys and drifts between them. Sum the eased segments
        // if the drift ever shows.
        let l = self.local(t) * self.rate(self.local(t));
        if self.reverse {
            self.src_in + self.src_len() - l
        } else {
            self.src_in + l
        }
    }
    /// Latest source time used by the clip.
    pub fn src_end(&self) -> f64 {
        self.src_in + self.src_len()
    }
    pub fn is_visual(&self) -> bool {
        self.kind != ClipKind::Audio
    }
    /// Draws pixels of its own (an Adjustment layer only re-processes what is below it).
    pub fn draws(&self) -> bool {
        self.is_visual() && self.kind != ClipKind::Adjustment
    }
    pub fn uses_asset(&self) -> bool {
        matches!(self.kind, ClipKind::Video | ClipKind::Image | ClipKind::Audio)
    }
    /// Does the renderer evaluate the node graph instead of the linear effect stack? The node editor is
    /// opt-in: a bare Input→Output graph says nothing the stack does not, so it never shadows it —
    /// otherwise a clip that once had the node pane pointed at it could never take a plain effect again.
    pub fn uses_graph(&self) -> bool {
        self.graph.as_ref().is_some_and(|g| g.nodes.len() > 2)
    }
    /// Speed, reverse or freeze in effect.
    pub fn is_retimed(&self) -> bool {
        (self.speed - 1.0).abs() > EPS || self.speed_curve.is_animated() || self.reverse || self.freeze.is_some()
    }
    /// Anything that needs re-rendering (disqualifies a lossless `-c copy` export).
    pub fn has_effects(&self) -> bool {
        !self.x.is_default(0.0)
            || !self.y.is_default(0.0)
            || !self.scale.is_default(1.0)
            || !self.rotation.is_default(0.0)
            || !self.opacity.is_default(1.0)
            || self.blend != BlendMode::Normal
            || self.is_retimed()
            || !self.effects.is_empty()
            || self.mask.is_some()
            || self.uses_graph()
            || !self.pan.is_default(0.0)
            || self.fade_in > 0.0
            || self.fade_out > 0.0
    }
    /// Change the playback rate keeping the source window and the timeline start (duration follows).
    pub fn set_speed(&mut self, speed: f64) {
        let speed = if speed.is_finite() { speed.clamp(0.01, 100.0) } else { 1.0 };
        if self.freeze.is_none() {
            let len = self.src_len();
            self.duration = (len / speed).max(MIN_CLIP);
        }
        self.speed = speed;
        // keep the curve's constant in step so the graph editor starts a ramp from the real rate
        if !self.speed_curve.is_animated() {
            self.speed_curve.value = speed;
        }
    }
    pub fn animated(&self) -> [&Animated; 10] {
        [
            &self.x,
            &self.y,
            &self.scale,
            &self.scale_x,
            &self.scale_y,
            &self.rotation,
            &self.opacity,
            &self.volume,
            &self.pan,
            &self.speed_curve,
        ]
    }
    /// Every keyframeable property including effect parameters.
    pub fn all_animated_mut(&mut self) -> Vec<&mut Animated> {
        let mut v: Vec<&mut Animated> = vec![
            &mut self.x,
            &mut self.y,
            &mut self.scale,
            &mut self.scale_x,
            &mut self.scale_y,
            &mut self.rotation,
            &mut self.opacity,
            &mut self.volume,
            &mut self.pan,
            &mut self.speed_curve,
        ];
        for e in &mut self.effects {
            v.extend(e.params.iter_mut());
            if let Some(m) = &mut e.mask {
                v.extend(m.animated_mut());
            }
        }
        if let Some(m) = &mut self.mask {
            v.extend(m.animated_mut());
        }
        if let Some(g) = &mut self.graph {
            v.extend(g.animated_mut());
        }
        if let Some(sh) = &mut self.shape {
            v.push(&mut sh.w);
            v.push(&mut sh.h);
        }
        v
    }
    pub fn all_animated(&self) -> Vec<&Animated> {
        let mut v: Vec<&Animated> = self.animated().to_vec();
        for e in &self.effects {
            v.extend(e.params.iter());
            if let Some(m) = &e.mask {
                v.extend(m.animated());
            }
        }
        if let Some(m) = &self.mask {
            v.extend(m.animated());
        }
        if let Some(g) = &self.graph {
            v.extend(g.animated());
        }
        if let Some(sh) = &self.shape {
            v.push(&sh.w);
            v.push(&sh.h);
        }
        v
    }
    /// (label, property) pairs for the inspector — visual ones for visual clips, volume/pan for audio.
    pub fn props_mut(&mut self) -> Vec<(&'static str, &mut Animated)> {
        if self.is_visual() {
            vec![
                ("Position X", &mut self.x),
                ("Position Y", &mut self.y),
                ("Scale", &mut self.scale),
                ("Scale X", &mut self.scale_x),
                ("Scale Y", &mut self.scale_y),
                ("Rotation", &mut self.rotation),
                ("Opacity", &mut self.opacity),
            ]
        } else {
            vec![("Volume", &mut self.volume), ("Pan", &mut self.pan)]
        }
    }
    /// Sorted, de-duplicated clip-local keyframe times across all properties (for drawing diamonds).
    pub fn key_times(&self) -> Vec<f64> {
        let mut v: Vec<f64> = self.all_animated().iter().flat_map(|a| a.keys.iter().map(|k| k.t)).collect();
        v.sort_by(f64::total_cmp);
        v.dedup_by(|a, b| (*a - *b).abs() < KEY_EPS);
        v
    }
    /// Shift every keyframe (all properties, effects included) by dt.
    pub fn shift_keys(&mut self, dt: f64) {
        for a in self.all_animated_mut() {
            a.shift(dt);
        }
    }
    /// Move every keyframe sitting at clip-local `t_old` to `t_new` (clamped inside the clip).
    pub fn move_keys(&mut self, t_old: f64, t_new: f64) {
        let t_new = t_new.clamp(0.0, self.duration);
        for a in self.all_animated_mut() {
            if let Some(i) = a.key_index_at(t_old) {
                a.move_key(i, t_new);
            }
        }
    }
    /// Split at timeline time t. `self` becomes the left part; returns the right part with `new_id`.
    /// None if t is not strictly inside the clip. Speed/reverse aware.
    pub fn split(&mut self, t: f64, new_id: Id) -> Option<Clip> {
        if t <= self.start + MIN_CLIP || t >= self.end() - MIN_CLIP {
            return None;
        }
        let off = t - self.start;
        let mut right = self.clone();
        right.id = new_id;
        right.start = t;
        right.duration = self.end() - t;
        if self.freeze.is_none() {
            if self.reverse {
                // left plays the later part of the source window, right the earlier part
                let src_in = self.src_in;
                self.src_in = src_in + (self.duration - off) * self.speed;
                right.src_in = src_in;
            } else {
                right.src_in = self.src_in + off * self.speed;
            }
        }
        right.shift_keys(-off);
        self.duration = off;
        Some(right)
    }
    /// Move the left edge to `new_start`, keeping the right edge fixed (slip-trim).
    /// `headroom` = source seconds available before the left edge (`Project::head_room`), INFINITY = unbounded.
    pub fn trim_start(&mut self, new_start: f64, headroom: f64) {
        let min_start =
            if headroom.is_finite() { self.start - headroom.max(0.0) / self.speed } else { f64::NEG_INFINITY };
        let ns = new_start.max(min_start).max(0.0).min(self.end() - MIN_CLIP);
        let d = ns - self.start;
        self.start = ns;
        self.duration -= d;
        if self.freeze.is_none() && !self.reverse {
            self.src_in += d * self.speed;
        }
        self.shift_keys(-d);
    }
    /// Move the right edge to `new_end`. `max_duration` = longest allowed duration (`Project::max_clip_duration`).
    pub fn trim_end(&mut self, new_end: f64, max_duration: f64) {
        let ne = new_end.min(self.start + max_duration).max(self.start + MIN_CLIP);
        let d = ne - self.end();
        self.duration = ne - self.start;
        if self.freeze.is_none() && self.reverse {
            // the right edge of a reversed clip is the earliest source time
            self.src_in = (self.src_in - d * self.speed).max(0.0);
        }
    }
}
