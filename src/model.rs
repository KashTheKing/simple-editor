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
/// Two clips abut (for transitions) when their boundary times differ by less than this.
pub const ABUT_EPS: f64 = 1e-4;

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
    /// A nested timeline (`Project.sequences`) used as footage; `clip.sequence` is its id. Lives on video
    /// tracks; carries the sequence's audio too (the mixer walks video tracks for these).
    Sequence,
}

/// Resampling quality used when the compositor scales/rotates layers (export == preview).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default, Hash)]
pub enum Scaler {
    Nearest,
    #[default]
    Bilinear,
    Bicubic,
}

impl Scaler {
    pub const ALL: [Scaler; 3] = [Scaler::Nearest, Scaler::Bilinear, Scaler::Bicubic];
    pub fn name(self) -> &'static str {
        match self {
            Scaler::Nearest => "Nearest neighbour",
            Scaler::Bilinear => "Bilinear",
            Scaler::Bicubic => "Bicubic",
        }
    }
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

/// Interpolation of the segment that *starts* at a keyframe.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize, Default)]
pub enum Ease {
    #[default]
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
    /// Step: hold the key's value until the next key.
    Hold,
    /// CSS-style cubic bezier through (0,0) (x1,y1) (x2,y2) (1,1): the velocity handles of the curve
    /// editor. y may leave 0..1 (overshoot / anticipate).
    Bezier { x1: f32, y1: f32, x2: f32, y2: f32 },
}

impl Ease {
    /// The fixed kinds (the curve editor offers these plus the bezier presets below).
    pub const ALL: [Ease; 5] = [Ease::Linear, Ease::EaseIn, Ease::EaseOut, Ease::EaseInOut, Ease::Hold];
    /// Named velocity presets (bezier handles).
    pub const PRESETS: [(&'static str, Ease); 6] = [
        ("Smooth", Ease::Bezier { x1: 0.42, y1: 0.0, x2: 0.58, y2: 1.0 }),
        ("Snap", Ease::Bezier { x1: 0.9, y1: 0.0, x2: 0.1, y2: 1.0 }),
        ("Slow Start", Ease::Bezier { x1: 0.55, y1: 0.0, x2: 1.0, y2: 0.45 }),
        ("Slow End", Ease::Bezier { x1: 0.0, y1: 0.55, x2: 0.45, y2: 1.0 }),
        ("Overshoot", Ease::Bezier { x1: 0.34, y1: 1.56, x2: 0.64, y2: 1.0 }),
        ("Anticipate", Ease::Bezier { x1: 0.36, y1: 0.0, x2: 0.66, y2: -0.56 }),
    ];
    pub fn name(self) -> &'static str {
        match self {
            Ease::Linear => "Linear",
            Ease::EaseIn => "Ease In",
            Ease::EaseOut => "Ease Out",
            Ease::EaseInOut => "Ease In/Out",
            Ease::Hold => "Hold",
            Ease::Bezier { .. } => "Bezier",
        }
    }
    /// Bezier handles equivalent to this ease (for dragging handles in the curve editor).
    pub fn handles(self) -> (f32, f32, f32, f32) {
        match self {
            Ease::Linear | Ease::Hold => (0.33, 0.33, 0.67, 0.67),
            Ease::EaseIn => (0.42, 0.0, 1.0, 1.0),
            Ease::EaseOut => (0.0, 0.0, 0.58, 1.0),
            Ease::EaseInOut => (0.42, 0.0, 0.58, 1.0),
            Ease::Bezier { x1, y1, x2, y2 } => (x1, y1, x2, y2),
        }
    }
    /// Map a linear 0..1 progress to the eased progress.
    pub fn apply(self, f: f64) -> f64 {
        let f = f.clamp(0.0, 1.0);
        match self {
            Ease::Linear => f,
            Ease::EaseIn => f * f,
            Ease::EaseOut => 1.0 - (1.0 - f) * (1.0 - f),
            Ease::EaseInOut => f * f * (3.0 - 2.0 * f),
            Ease::Hold => 0.0,
            Ease::Bezier { x1, y1, x2, y2 } => cubic_bezier(f, x1 as f64, y1 as f64, x2 as f64, y2 as f64),
        }
    }
}

/// y for the x = `f` on the cubic bezier (0,0) (x1,y1) (x2,y2) (1,1) — Newton iterations on the x polynomial.
fn cubic_bezier(f: f64, x1: f64, y1: f64, x2: f64, y2: f64) -> f64 {
    let (x1, x2) = (x1.clamp(0.0, 1.0), x2.clamp(0.0, 1.0));
    let bx = |t: f64| 3.0 * (1.0 - t) * (1.0 - t) * t * x1 + 3.0 * (1.0 - t) * t * t * x2 + t * t * t;
    let by = |t: f64| 3.0 * (1.0 - t) * (1.0 - t) * t * y1 + 3.0 * (1.0 - t) * t * t * y2 + t * t * t;
    let dbx = |t: f64| 3.0 * (1.0 - t) * (1.0 - t) * x1 + 6.0 * (1.0 - t) * t * (x2 - x1) + 3.0 * t * t * (1.0 - x2);
    let mut t = f;
    for _ in 0..8 {
        let d = dbx(t);
        if d.abs() < 1e-6 {
            break;
        }
        t -= (bx(t) - f) / d;
        t = t.clamp(0.0, 1.0);
    }
    by(t)
}

fn is_linear(e: &Ease) -> bool {
    *e == Ease::Linear
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct Keyframe {
    pub t: f64,
    pub v: f64,
    /// Easing of the segment from this key to the next.
    #[serde(default, skip_serializing_if = "is_linear")]
    pub ease: Ease,
}

/// A scalar property that is either constant (`value`) or keyframed (`keys`, sorted by t).
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
    /// Value at clip-local time t (eased interpolation, clamped at the ends).
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
                return w[0].v + (w[1].v - w[0].v) * w[0].ease.apply(f);
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
        self.keys.insert(i, Keyframe { t, v, ease: Ease::Linear });
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
    /// Move key `i` to `new_t` (keeps keys sorted); returns its new index.
    pub fn move_key(&mut self, i: usize, new_t: f64) -> usize {
        if i >= self.keys.len() {
            return i;
        }
        let mut k = self.keys.remove(i);
        k.t = new_t;
        let j = self.keys.partition_point(|o| o.t < new_t);
        self.keys.insert(j, k);
        j
    }
    pub fn ease_at(&self, t: f64) -> Option<Ease> {
        self.key_index_at(t).map(|i| self.keys[i].ease)
    }
    pub fn set_ease_at(&mut self, t: f64, ease: Ease) {
        if let Some(i) = self.key_index_at(t) {
            self.keys[i].ease = ease;
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
    /// Default look for burnt-in subtitles.
    pub fn subtitle_default() -> Self {
        Self { text: String::new(), size: 48.0, outline_width: 3.0, ..Self::default() }
    }
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

/// Colour labels for assets (0 = none).
pub const LABEL_COLORS: [(&str, [u8; 3]); 8] = [
    ("Red", [220, 70, 70]),
    ("Orange", [230, 140, 50]),
    ("Yellow", [220, 200, 60]),
    ("Green", [80, 180, 90]),
    ("Teal", [60, 180, 180]),
    ("Blue", [70, 120, 220]),
    ("Purple", [150, 90, 200]),
    ("Gray", [150, 150, 150]),
];

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
    /// Library folder ("" = root, "Footage/Day 1" = nested).
    #[serde(default)]
    pub folder: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// 0 = none, 1..=8 = index+1 into LABEL_COLORS.
    #[serde(default)]
    pub label: u8,
    /// Free-form notes: what this asset is / what it's for (library + inspector).
    #[serde(default)]
    pub description: String,
}

impl Asset {
    pub fn name(&self) -> String {
        Path::new(&self.path).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| self.path.clone())
    }
    pub fn has_video(&self) -> bool {
        matches!(self.kind, ClipKind::Video | ClipKind::Image)
    }
}

// ---------- effects ----------

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Hash)]
pub enum EffectKind {
    Blur,
    Pixelate,
    Tint,
    Color,
    Vignette,
    Sharpen,
    Invert,
    Grayscale,
    Flip,
    Crop,
    Wobble,
}

/// One effect parameter: label, default and UI range.
#[derive(Clone, Copy, Debug)]
pub struct ParamSpec {
    pub name: &'static str,
    pub default: f64,
    pub min: f64,
    pub max: f64,
}

const fn ps(name: &'static str, default: f64, min: f64, max: f64) -> ParamSpec {
    ParamSpec { name, default, min, max }
}

impl EffectKind {
    pub const ALL: [EffectKind; 11] = [
        EffectKind::Blur,
        EffectKind::Pixelate,
        EffectKind::Tint,
        EffectKind::Color,
        EffectKind::Vignette,
        EffectKind::Sharpen,
        EffectKind::Invert,
        EffectKind::Grayscale,
        EffectKind::Flip,
        EffectKind::Crop,
        EffectKind::Wobble,
    ];
    pub fn name(self) -> &'static str {
        match self {
            EffectKind::Blur => "Blur",
            EffectKind::Pixelate => "Pixelate",
            EffectKind::Tint => "Color Tint",
            EffectKind::Color => "Color Correction",
            EffectKind::Vignette => "Vignette",
            EffectKind::Sharpen => "Sharpen",
            EffectKind::Invert => "Invert",
            EffectKind::Grayscale => "Black & White",
            EffectKind::Flip => "Flip",
            EffectKind::Crop => "Crop",
            EffectKind::Wobble => "Camera Shake",
        }
    }
    /// Parameters in index order (matches `Effect.params`). Pixel sizes are project pixels.
    pub fn params(self) -> &'static [ParamSpec] {
        match self {
            EffectKind::Blur => P_BLUR,
            EffectKind::Pixelate => P_PIXELATE,
            EffectKind::Tint => P_TINT,
            EffectKind::Color => P_COLOR,
            EffectKind::Vignette => P_VIGNETTE,
            EffectKind::Sharpen => P_SHARPEN,
            EffectKind::Invert => P_INVERT,
            EffectKind::Grayscale => P_GRAYSCALE,
            EffectKind::Flip => P_FLIP,
            EffectKind::Crop => P_CROP,
            EffectKind::Wobble => P_WOBBLE,
        }
    }
}

const P_BLUR: &[ParamSpec] = &[ps("Radius", 8.0, 0.0, 100.0)];
const P_PIXELATE: &[ParamSpec] = &[ps("Block size", 16.0, 1.0, 200.0)];
const P_TINT: &[ParamSpec] = &[
    ps("Red", 255.0, 0.0, 255.0),
    ps("Green", 128.0, 0.0, 255.0),
    ps("Blue", 0.0, 0.0, 255.0),
    ps("Amount", 0.5, 0.0, 1.0),
];
const P_COLOR: &[ParamSpec] = &[
    ps("Brightness", 0.0, -1.0, 1.0),
    ps("Contrast", 1.0, 0.0, 3.0),
    ps("Saturation", 1.0, 0.0, 3.0),
    ps("Hue", 0.0, -180.0, 180.0),
    ps("Gamma", 1.0, 0.1, 5.0),
];
const P_VIGNETTE: &[ParamSpec] =
    &[ps("Radius", 0.8, 0.0, 1.5), ps("Softness", 0.5, 0.0, 1.0), ps("Strength", 0.6, 0.0, 1.0)];
const P_SHARPEN: &[ParamSpec] = &[ps("Amount", 0.5, 0.0, 3.0), ps("Radius", 2.0, 0.5, 20.0)];
const P_INVERT: &[ParamSpec] = &[ps("Amount", 1.0, 0.0, 1.0)];
const P_GRAYSCALE: &[ParamSpec] = &[ps("Amount", 1.0, 0.0, 1.0)];
const P_FLIP: &[ParamSpec] = &[ps("Horizontal", 1.0, 0.0, 1.0), ps("Vertical", 0.0, 0.0, 1.0)];
const P_CROP: &[ParamSpec] = &[
    ps("Left", 0.0, 0.0, 0.5),
    ps("Right", 0.0, 0.0, 0.5),
    ps("Top", 0.0, 0.0, 0.5),
    ps("Bottom", 0.0, 0.0, 0.5),
    ps("Feather", 0.0, 0.0, 0.5),
];
const P_WOBBLE: &[ParamSpec] = &[
    ps("Amplitude X", 20.0, 0.0, 500.0),
    ps("Amplitude Y", 20.0, 0.0, 500.0),
    ps("Roll", 2.0, 0.0, 45.0),
    ps("Yaw", 3.0, 0.0, 45.0),
    ps("Pitch", 3.0, 0.0, 45.0),
    ps("Frequency", 2.0, 0.1, 30.0),
    ps("Seed", 1.0, 0.0, 1000.0),
];

/// An effect instance on a clip; `params[i]` follows `kind.params()[i]` (each keyframeable).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Effect {
    pub kind: EffectKind,
    #[serde(default = "tru")]
    pub enabled: bool,
    #[serde(default)]
    pub params: Vec<Animated>,
}

impl Effect {
    pub fn new(kind: EffectKind) -> Self {
        Self { kind, enabled: true, params: kind.params().iter().map(|p| Animated::new(p.default)).collect() }
    }
    /// Parameter i at clip-local time t (spec default when missing, e.g. older files).
    pub fn at(&self, i: usize, t: f64) -> f64 {
        self.params.get(i).map(|a| a.at(t)).unwrap_or_else(|| self.kind.params().get(i).map(|p| p.default).unwrap_or(0.0))
    }
    pub fn specs(&self) -> &'static [ParamSpec] {
        self.kind.params()
    }
}

// ---------- transitions ----------

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Hash)]
pub enum TransitionKind {
    /// Dissolve A → B.
    CrossFade,
    /// A → colour → B (dip to black/white/…).
    FadeToColor,
    /// B pushes A out (direction).
    Push,
    /// B wipes over A (direction).
    Wipe,
}

impl TransitionKind {
    pub const ALL: [TransitionKind; 4] =
        [TransitionKind::CrossFade, TransitionKind::FadeToColor, TransitionKind::Push, TransitionKind::Wipe];
    pub fn name(self) -> &'static str {
        match self {
            TransitionKind::CrossFade => "Cross Fade",
            TransitionKind::FadeToColor => "Fade to Color",
            TransitionKind::Push => "Push",
            TransitionKind::Wipe => "Wipe",
        }
    }
    pub fn has_direction(self) -> bool {
        matches!(self, TransitionKind::Push | TransitionKind::Wipe)
    }
}

/// A transition centred on the cut between a clip and its right neighbour on the same track:
/// it spans [right.start - duration/2, right.start + duration/2). Both clips are extended virtually
/// into that window (the engine clamps source times). Audio tracks use CrossFade (a gain crossfade).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Transition {
    pub id: Id,
    /// The clip on the right side of the cut.
    pub right: Id,
    pub kind: TransitionKind,
    pub duration: f64,
    #[serde(default = "black")]
    pub color: [u8; 4],
    /// 0 = left, 1 = right, 2 = up, 3 = down (Push / Wipe).
    #[serde(default)]
    pub direction: u8,
    #[serde(default)]
    pub ease: Ease,
}

fn black() -> [u8; 4] {
    [0, 0, 0, 255]
}

impl Transition {
    /// Progress 0..1 at timeline time t, given the cut time (right clip's start).
    pub fn progress(&self, cut: f64, t: f64) -> f64 {
        if self.duration <= 0.0 {
            return 1.0;
        }
        let f = (t - (cut - self.duration / 2.0)) / self.duration;
        self.ease.apply(f.clamp(0.0, 1.0))
    }
    pub fn window(&self, cut: f64) -> (f64, f64) {
        (cut - self.duration / 2.0, cut + self.duration / 2.0)
    }
}

// ---------- subtitles ----------

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Cue {
    pub id: Id,
    pub start: f64,
    pub end: f64,
    pub text: String,
}

fn tru() -> bool {
    true
}
fn one() -> f64 {
    1.0
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
    /// Earliest source time used by the clip (the source window is [src_in, src_in + duration*speed)).
    #[serde(default)]
    pub src_in: f64,
    /// For Audio clips: which audio stream of the asset (0-based among audio streams).
    #[serde(default)]
    pub audio_stream: usize,
    /// For Sequence clips: the nested timeline id.
    #[serde(default)]
    pub sequence: Id,
    #[serde(default = "tru")]
    pub enabled: bool,
    /// Colour label shown on the timeline: 0 = inherit the asset's label, 1..=8 = LABEL_COLORS index + 1.
    #[serde(default)]
    pub label: u8,
    // --- retime ---
    /// Playback rate (1 = normal, 2 = twice as fast, 0.5 = half speed). Always > 0.
    #[serde(default = "one")]
    pub speed: f64,
    /// Play the source window backwards.
    #[serde(default)]
    pub reverse: bool,
    /// Freeze frame: show/hold the source frame at this source time for the whole clip (audio is silent).
    #[serde(default)]
    pub freeze: Option<f64>,
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
    /// Effect stack, applied in order before blending.
    #[serde(default)]
    pub effects: Vec<Effect>,
    // --- audio ---
    /// Linear gain (1 = unity).
    #[serde(default = "a1")]
    pub volume: Animated,
    /// -1 = left … 0 = centre … 1 = right.
    #[serde(default = "a0")]
    pub pan: Animated,
    /// Fade in/out lengths in seconds (gain ramps at the clip edges).
    #[serde(default)]
    pub fade_in: f64,
    #[serde(default)]
    pub fade_out: f64,
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
            sequence: 0,
            enabled: true,
            label: 0,
            speed: 1.0,
            reverse: false,
            freeze: None,
            x: a0(),
            y: a0(),
            scale: a1(),
            rotation: a0(),
            opacity: a1(),
            blend: BlendMode::Normal,
            effects: Vec::new(),
            volume: a1(),
            pan: a0(),
            fade_in: 0.0,
            fade_out: 0.0,
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
    /// Length of the source window in source seconds.
    pub fn src_len(&self) -> f64 {
        self.duration * self.speed
    }
    /// Source media time at timeline time t (speed, reverse and freeze applied).
    pub fn src_time(&self, t: f64) -> f64 {
        if let Some(f) = self.freeze {
            return f;
        }
        let l = self.local(t) * self.speed;
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
    pub fn uses_asset(&self) -> bool {
        matches!(self.kind, ClipKind::Video | ClipKind::Image | ClipKind::Audio)
    }
    /// Speed, reverse or freeze in effect.
    pub fn is_retimed(&self) -> bool {
        (self.speed - 1.0).abs() > EPS || self.reverse || self.freeze.is_some()
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
    }
    pub fn animated_mut(&mut self) -> [&mut Animated; 7] {
        [
            &mut self.x,
            &mut self.y,
            &mut self.scale,
            &mut self.rotation,
            &mut self.opacity,
            &mut self.volume,
            &mut self.pan,
        ]
    }
    pub fn animated(&self) -> [&Animated; 7] {
        [&self.x, &self.y, &self.scale, &self.rotation, &self.opacity, &self.volume, &self.pan]
    }
    /// Every keyframeable property including effect parameters.
    pub fn all_animated_mut(&mut self) -> Vec<&mut Animated> {
        let mut v: Vec<&mut Animated> = vec![
            &mut self.x,
            &mut self.y,
            &mut self.scale,
            &mut self.rotation,
            &mut self.opacity,
            &mut self.volume,
            &mut self.pan,
        ];
        for e in &mut self.effects {
            v.extend(e.params.iter_mut());
        }
        v
    }
    pub fn all_animated(&self) -> Vec<&Animated> {
        let mut v: Vec<&Animated> = self.animated().to_vec();
        for e in &self.effects {
            v.extend(e.params.iter());
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
        let min_start = if headroom.is_finite() { self.start - headroom.max(0.0) / self.speed } else { f64::NEG_INFINITY };
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

fn dh() -> f32 {
    60.0
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Track {
    pub id: Id,
    pub name: String,
    pub kind: TrackKind,
    /// Audio: muted. Video: hidden (the "V" visibility toggle).
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub solo: bool,
    /// UI height in points.
    #[serde(default = "dh")]
    pub height: f32,
    #[serde(default)]
    pub clips: Vec<Clip>,
    #[serde(default)]
    pub transitions: Vec<Transition>,
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
            transitions: Vec::new(),
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
    /// The clip ending exactly where `right` starts (the left side of that cut).
    pub fn left_of(&self, right: &Clip) -> Option<&Clip> {
        self.clips.iter().find(|c| c.id != right.id && (c.end() - right.start).abs() < ABUT_EPS)
    }
    /// (transition, left clip, right clip) for a transition of this track, if it is still valid.
    pub fn transition_pair(&self, tr: &Transition) -> Option<(&Clip, &Clip)> {
        let right = self.clips.iter().find(|c| c.id == tr.right)?;
        let left = self.left_of(right)?;
        Some((left, right))
    }
    /// The transition whose window contains timeline time t, with its clips.
    pub fn transition_at(&self, t: f64) -> Option<(&Transition, &Clip, &Clip)> {
        self.transitions.iter().find_map(|tr| {
            let (l, r) = self.transition_pair(tr)?;
            let (a, b) = tr.window(r.start);
            (t >= a && t < b).then_some((tr, l, r))
        })
    }
    /// Drop transitions whose clips no longer abut.
    pub fn prune_transitions(&mut self) {
        let keep: Vec<Id> = self.transitions.iter().filter(|t| self.transition_pair(t).is_some()).map(|t| t.id).collect();
        self.transitions.retain(|t| keep.contains(&t.id));
    }
}

/// A nested timeline ("compound clip"): its own tracks/size/fps; placed on video tracks as a
/// `ClipKind::Sequence` clip and rendered/mixed recursively. Edited by swapping it into `Project.tracks`
/// (`open_sequence` / `close_sequence`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sequence {
    pub id: Id,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub tracks: Vec<Track>,
}

impl Sequence {
    pub fn duration(&self) -> f64 {
        self.tracks.iter().map(|t| t.end()).fold(0.0, f64::max)
    }
}

/// The main timeline's state while a sequence is swapped in for editing.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Stash {
    pub tracks: Vec<Track>,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub in_point: Option<f64>,
    pub out_point: Option<f64>,
}

/// Planner item: a checkable task with notes, colour, nested sub-tasks and a moodboard of assets.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PlanItem {
    pub id: Id,
    pub title: String,
    pub done: bool,
    pub notes: String,
    /// 0 = none, 1..=8 = LABEL_COLORS index + 1.
    pub color: u8,
    /// Moodboard: asset ids (each with an optional description in `asset_notes`).
    pub assets: Vec<Id>,
    pub asset_notes: Vec<String>,
    pub children: Vec<PlanItem>,
}

impl Default for PlanItem {
    fn default() -> Self {
        Self {
            id: 0,
            title: String::new(),
            done: false,
            notes: String::new(),
            color: 0,
            assets: Vec::new(),
            asset_notes: Vec::new(),
            children: Vec::new(),
        }
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
    /// Library folders that exist even while empty ("A/B" nesting by '/').
    pub folders: Vec<String>,
    /// Folders on disk browsed directly from the library (files are imported on drop).
    pub linked_folders: Vec<String>,
    /// Order: all video tracks (V1, V2, ...) then all audio tracks (A1, A2, ...). While `editing` is
    /// Some(seq), these are that sequence's tracks (the main timeline is in `main_stash`).
    pub tracks: Vec<Track>,
    pub in_point: Option<f64>,
    pub out_point: Option<f64>,
    /// Set when the project was created by opening a video file directly; enables "Save" (overwrite).
    pub source_video: Option<String>,
    /// Subtitle cues (kept sorted by start). Rendered bottom-centre by the compositor when `show_subtitles`.
    pub subtitles: Vec<Cue>,
    pub subtitle_style: TextStyle,
    /// Distance from the bottom edge in project pixels.
    pub subtitle_margin: f32,
    pub show_subtitles: bool,
    /// Compositor resampling quality.
    pub scaler: Scaler,
    /// Nested timelines (usable as footage via `ClipKind::Sequence`).
    pub sequences: Vec<Sequence>,
    /// The sequence currently swapped into `tracks` for editing (None = main timeline).
    pub editing: Option<Id>,
    pub main_stash: Option<Stash>,
    /// Planner (nested tasks with moodboards) and free-form notes (process / ideas / style).
    pub plan: Vec<PlanItem>,
    pub notes: String,
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
            folders: Vec::new(),
            linked_folders: Vec::new(),
            tracks: Vec::new(),
            in_point: None,
            out_point: None,
            source_video: None,
            subtitles: Vec::new(),
            subtitle_style: TextStyle::subtitle_default(),
            subtitle_margin: 60.0,
            show_subtitles: true,
            scaler: Scaler::Bilinear,
            sequences: Vec::new(),
            editing: None,
            main_stash: None,
            plan: Vec::new(),
            notes: String::new(),
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
    pub fn asset_mut(&mut self, id: Id) -> Option<&mut Asset> {
        self.assets.iter_mut().find(|a| a.id == id)
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
        self.tidy();
    }
    /// All folder names (explicit + used by assets), sorted.
    pub fn folder_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.folders.clone();
        v.extend(self.assets.iter().filter(|a| !a.folder.is_empty()).map(|a| a.folder.clone()));
        v.sort();
        v.dedup();
        v
    }
    /// Create (or ensure) a folder; returns false if the name is empty.
    pub fn add_folder(&mut self, name: &str) -> bool {
        let name = name.trim().trim_matches('/').to_string();
        if name.is_empty() {
            return false;
        }
        if !self.folders.contains(&name) {
            self.folders.push(name);
            self.folders.sort();
        }
        true
    }
    /// Remove a folder (and sub-folders); assets inside move to the root.
    pub fn remove_folder(&mut self, name: &str) {
        let prefix = format!("{name}/");
        self.folders.retain(|f| f != name && !f.starts_with(&prefix));
        for a in &mut self.assets {
            if a.folder == name || a.folder.starts_with(&prefix) {
                a.folder.clear();
            }
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
    /// Source seconds available before the clip's left edge (for `Clip::trim_start`).
    pub fn head_room(&self, clip: &Clip) -> f64 {
        if !matches!(clip.kind, ClipKind::Video | ClipKind::Audio) || clip.freeze.is_some() {
            return f64::INFINITY;
        }
        let Some(a) = self.asset(clip.asset) else { return f64::INFINITY };
        if clip.reverse {
            (a.duration - clip.src_end()).max(0.0)
        } else {
            clip.src_in.max(0.0)
        }
    }
    /// Longest duration the clip may have (right edge), given its source window, speed and direction.
    pub fn max_clip_duration(&self, clip: &Clip) -> f64 {
        if !matches!(clip.kind, ClipKind::Video | ClipKind::Audio) || clip.freeze.is_some() {
            return f64::INFINITY;
        }
        let Some(a) = self.asset(clip.asset) else { return f64::INFINITY };
        let extra = if clip.reverse { clip.src_in } else { a.duration - clip.src_end() };
        (clip.duration + extra.max(0.0) / clip.speed).max(MIN_CLIP)
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
    /// Re-sort clips and drop transitions whose cuts no longer exist. Call after any layout change.
    pub fn tidy(&mut self) {
        for t in &mut self.tracks {
            t.sort();
            t.prune_transitions();
        }
    }

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
        self.tidy();
        new_ids
    }

    /// Freeze-frame the clips at timeline time t: each clip is split at t and its right part becomes a
    /// still of the frame at t (linked clips of the same time too). Returns the frozen clip ids.
    pub fn freeze_at(&mut self, t: f64, ids: &[Id]) -> Vec<Id> {
        let ids = self.expand_links(ids);
        let mut frozen = Vec::new();
        for id in ids {
            let Some(c) = self.clip(id) else { continue };
            let src = c.src_time(t);
            let target = if c.contains(t) && t > c.start + MIN_CLIP {
                self.split_at(t, Some(&[id])).first().copied().unwrap_or(id)
            } else {
                id
            };
            if let Some(c) = self.clip_mut(target) {
                c.freeze = Some(src);
                frozen.push(target);
            }
        }
        frozen
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
        self.tidy();
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
        self.tidy();
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
            // transitions belong to the track; moving the right clip across tracks carries none
            self.tracks[to].clips.push(c);
        }
        self.tidy();
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
    /// Apply speed/reverse to the clips and their linked clips (keeps source windows; durations follow).
    /// Returns false if the new lengths would collide with neighbours (nothing changed).
    pub fn set_speed(&mut self, ids: &[Id], speed: f64, reverse: bool) -> bool {
        let ids = self.expand_links(ids);
        let mut tmp: Vec<(usize, usize, Clip)> = Vec::new();
        for &id in &ids {
            let Some((ti, ci)) = self.find(id) else { continue };
            let mut c = self.tracks[ti].clips[ci].clone();
            c.set_speed(speed);
            c.reverse = reverse;
            if !self.tracks[ti].fits(c.start, c.duration, &ids) {
                return false;
            }
            tmp.push((ti, ci, c));
        }
        for (ti, ci, c) in tmp {
            self.tracks[ti].clips[ci] = c;
        }
        self.tidy();
        true
    }

    // ---------- transitions ----------
    /// Add (or replace) a transition at the cut on the left of clip `right`. Linked audio clips whose
    /// left neighbour is linked with the video's left neighbour get an audio CrossFade of the same length.
    /// Returns the new transition id, or None if `right` has no abutting left neighbour.
    pub fn add_transition(&mut self, right: Id, kind: TransitionKind, duration: f64) -> Option<Id> {
        let (ti, _) = self.find(right)?;
        let r = self.clip(right)?.clone();
        self.tracks[ti].left_of(&r)?;
        let duration = duration.max(MIN_CLIP);
        let id = self.new_id();
        self.tracks[ti].transitions.retain(|t| t.right != right);
        self.tracks[ti].transitions.push(Transition {
            id,
            right,
            kind,
            duration,
            color: [0, 0, 0, 255],
            direction: 0,
            ease: Ease::Linear,
        });
        // mirror on linked clips (audio crossfade)
        if r.link != 0 {
            let partners: Vec<Id> = self.linked(right).into_iter().filter(|&p| p != right).collect();
            for p in partners {
                let Some((pti, _)) = self.find(p) else { continue };
                if pti == ti {
                    continue;
                }
                let pc = self.clip(p).unwrap().clone();
                if self.tracks[pti].left_of(&pc).is_none() {
                    continue;
                }
                let pid = self.new_id();
                let pkind = if self.tracks[pti].kind == TrackKind::Audio { TransitionKind::CrossFade } else { kind };
                self.tracks[pti].transitions.retain(|t| t.right != p);
                self.tracks[pti].transitions.push(Transition {
                    id: pid,
                    right: p,
                    kind: pkind,
                    duration,
                    color: [0, 0, 0, 255],
                    direction: 0,
                    ease: Ease::Linear,
                });
            }
        }
        Some(id)
    }
    pub fn remove_transition(&mut self, id: Id) {
        for t in &mut self.tracks {
            t.transitions.retain(|x| x.id != id);
        }
    }
    pub fn transition_mut(&mut self, id: Id) -> Option<&mut Transition> {
        self.tracks.iter_mut().flat_map(|t| t.transitions.iter_mut()).find(|x| x.id == id)
    }
    /// Transitions touching a clip (as left or right side): (track index, transition).
    pub fn transitions_of(&self, clip: Id) -> Vec<(usize, &Transition)> {
        let mut out = Vec::new();
        for (ti, t) in self.tracks.iter().enumerate() {
            for tr in &t.transitions {
                if let Some((l, r)) = t.transition_pair(tr) {
                    if l.id == clip || r.id == clip {
                        out.push((ti, tr));
                    }
                }
            }
        }
        out
    }

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
    pub fn sort_cues(&mut self) {
        self.subtitles.sort_by(|a, b| a.start.total_cmp(&b.start));
    }

    // ---------- sequences (nested timelines) ----------
    pub fn sequence(&self, id: Id) -> Option<&Sequence> {
        self.sequences.iter().find(|s| s.id == id)
    }
    pub fn sequence_mut(&mut self, id: Id) -> Option<&mut Sequence> {
        self.sequences.iter_mut().find(|s| s.id == id)
    }
    /// New empty sequence (V1 + A1) with the given format; returns its id.
    pub fn new_sequence(&mut self, name: impl Into<String>, width: u32, height: u32, fps: f64) -> Id {
        let id = self.new_id();
        let mut seq = Sequence { id, name: name.into(), width, height, fps, tracks: Vec::new() };
        let v = Track::new(self.new_id(), TrackKind::Video, "V1");
        let a = Track::new(self.new_id(), TrackKind::Audio, "A1");
        seq.tracks.push(v);
        seq.tracks.push(a);
        self.sequences.push(seq);
        id
    }
    /// Duration of a sequence (the live tracks when it is the one being edited).
    pub fn sequence_duration(&self, id: Id) -> f64 {
        if self.editing == Some(id) {
            return self.duration();
        }
        self.sequence(id).map(|s| s.duration()).unwrap_or(0.0)
    }
    /// Tracks of a sequence for rendering (its own, or the live `tracks` while it is being edited).
    pub fn sequence_tracks(&self, id: Id) -> Option<&[Track]> {
        if self.editing == Some(id) {
            return Some(&self.tracks);
        }
        self.sequence(id).map(|s| s.tracks.as_slice())
    }
    /// True if sequence `outer` contains `inner` directly or through nested sequence clips.
    pub fn sequence_contains(&self, outer: Id, inner: Id) -> bool {
        fn walk(p: &Project, tracks: &[Track], inner: Id, depth: u32) -> bool {
            if depth > 32 {
                return true; // treat runaway nesting as a cycle
            }
            tracks.iter().flat_map(|t| t.clips.iter()).any(|c| {
                c.kind == ClipKind::Sequence
                    && (c.sequence == inner
                        || p.sequence_tracks(c.sequence).map(|tr| walk(p, tr, inner, depth + 1)).unwrap_or(false))
            })
        }
        outer == inner || self.sequence_tracks(outer).map(|t| walk(self, t, inner, 0)).unwrap_or(false)
    }
    /// Swap sequence `id` into `tracks` for editing (closing any other open sequence first).
    pub fn open_sequence(&mut self, id: Id) -> bool {
        if self.editing == Some(id) {
            return true;
        }
        if self.sequence(id).is_none() {
            return false;
        }
        self.close_sequence();
        let stash = Stash {
            tracks: std::mem::take(&mut self.tracks),
            width: self.width,
            height: self.height,
            fps: self.fps,
            in_point: self.in_point.take(),
            out_point: self.out_point.take(),
        };
        let seq = self.sequence_mut(id).unwrap();
        let tracks = std::mem::take(&mut seq.tracks);
        let (w, h, fps) = (seq.width, seq.height, seq.fps);
        self.main_stash = Some(stash);
        self.tracks = tracks;
        self.width = w;
        self.height = h;
        self.fps = fps;
        self.editing = Some(id);
        true
    }
    /// Put the edited sequence back and restore the main timeline.
    pub fn close_sequence(&mut self) {
        let Some(id) = self.editing.take() else { return };
        let tracks = std::mem::take(&mut self.tracks);
        let (w, h, fps) = (self.width, self.height, self.fps);
        if let Some(seq) = self.sequence_mut(id) {
            seq.tracks = tracks;
            seq.width = w;
            seq.height = h;
            seq.fps = fps;
        }
        if let Some(st) = self.main_stash.take() {
            self.tracks = st.tracks;
            self.width = st.width;
            self.height = st.height;
            self.fps = st.fps;
            self.in_point = st.in_point;
            self.out_point = st.out_point;
        }
    }
    /// Place a sequence as a clip at `at` (video track `video_track` preferred). None on cycles / unknown id.
    pub fn insert_sequence_clip(&mut self, seq: Id, at: f64, video_track: Option<usize>) -> Option<Id> {
        let name = self.sequence(seq)?.name.clone();
        // a sequence can't contain itself: the timeline being edited (or main) must not be inside `seq`
        if let Some(cur) = self.editing {
            if self.sequence_contains(seq, cur) {
                return None;
            }
        }
        let dur = self.sequence_duration(seq).max(MIN_CLIP);
        let ti = self.find_free_track(TrackKind::Video, at, dur, video_track);
        let mut c = Clip::new(self.new_id(), ClipKind::Sequence, name, at, dur);
        c.sequence = seq;
        let id = c.id;
        self.tracks[ti].clips.push(c);
        self.tracks[ti].sort();
        Some(id)
    }
    /// Move the selected clips (+ linked) into a new sequence and replace them with one Sequence clip.
    /// Returns the new sequence id.
    pub fn nest_selection(&mut self, ids: &[Id], name: impl Into<String>) -> Option<Id> {
        let ids = self.expand_links(ids);
        if ids.is_empty() {
            return None;
        }
        let start = ids.iter().filter_map(|&id| self.clip(id)).map(|c| c.start).fold(f64::INFINITY, f64::min);
        let end = ids.iter().filter_map(|&id| self.clip(id)).map(|c| c.end()).fold(0.0, f64::max);
        if !start.is_finite() || end <= start {
            return None;
        }
        let (w, h, fps) = (self.width, self.height, self.fps);
        let seq_id = self.new_sequence(name, w, h, fps);
        // move clips: keep their track kind and relative order of tracks
        let mut moved: Vec<(TrackKind, usize, Clip)> = Vec::new(); // (kind, index within kind, clip)
        let mut top_video: Option<usize> = None;
        for &id in &ids {
            let Some((ti, ci)) = self.find(id) else { continue };
            let kind = self.tracks[ti].kind;
            let list = if kind == TrackKind::Video { self.video_tracks() } else { self.audio_tracks() };
            let pos = list.iter().position(|&x| x == ti).unwrap_or(0);
            if kind == TrackKind::Video {
                top_video = Some(top_video.map_or(ti, |t: usize| t.max(ti)));
            }
            let mut c = self.tracks[ti].clips.remove(ci);
            c.start -= start;
            moved.push((kind, pos, c));
        }
        self.tidy();
        let next_ids: Vec<Id> = (0..64).map(|_| self.new_id()).collect();
        let mut nid = next_ids.into_iter();
        if let Some(seq) = self.sequence_mut(seq_id) {
            for (kind, pos, c) in moved {
                // ensure track `pos` of this kind exists
                loop {
                    let have = seq.tracks.iter().filter(|t| t.kind == kind).count();
                    if have > pos {
                        break;
                    }
                    let id = nid.next().unwrap_or(0);
                    let n = have + 1;
                    let name = format!("{}{}", if kind == TrackKind::Video { "V" } else { "A" }, n);
                    let t = Track::new(id, kind, name);
                    let idx = if kind == TrackKind::Video {
                        seq.tracks.iter().rposition(|t| t.kind == TrackKind::Video).map(|i| i + 1).unwrap_or(0)
                    } else {
                        seq.tracks.len()
                    };
                    seq.tracks.insert(idx, t);
                }
                let ti = seq
                    .tracks
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| t.kind == kind)
                    .nth(pos)
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                seq.tracks[ti].clips.push(c);
                seq.tracks[ti].sort();
            }
        }
        let vt = top_video.or_else(|| self.video_tracks().last().copied());
        self.insert_sequence_clip(seq_id, start, vt);
        Some(seq_id)
    }

    // ---------- labels ----------
    /// Effective colour label of a clip (its own, else its asset's). 0 = none.
    pub fn clip_label(&self, clip: &Clip) -> u8 {
        if clip.label != 0 {
            return clip.label;
        }
        if clip.uses_asset() {
            return self.asset(clip.asset).map(|a| a.label).unwrap_or(0);
        }
        0
    }

    // ---------- planner ----------
    fn plan_find_in(items: &mut [PlanItem], id: Id) -> Option<&mut PlanItem> {
        for it in items {
            if it.id == id {
                return Some(it);
            }
            if let Some(f) = Self::plan_find_in(&mut it.children, id) {
                return Some(f);
            }
        }
        None
    }
    pub fn plan_item_mut(&mut self, id: Id) -> Option<&mut PlanItem> {
        Self::plan_find_in(&mut self.plan, id)
    }
    /// Add an item (under `parent` or at the top level); returns its id.
    pub fn plan_add(&mut self, parent: Option<Id>, title: impl Into<String>) -> Id {
        let id = self.new_id();
        let item = PlanItem { id, title: title.into(), ..Default::default() };
        match parent.and_then(|p| self.plan_item_mut(p)) {
            Some(p) => p.children.push(item),
            None => self.plan.push(item),
        }
        id
    }
    pub fn plan_remove(&mut self, id: Id) {
        fn rm(items: &mut Vec<PlanItem>, id: Id) {
            items.retain(|i| i.id != id);
            for i in items {
                rm(&mut i.children, id);
            }
        }
        rm(&mut self.plan, id);
    }
    /// All asset ids referenced by moodboards (never counted as "unused").
    pub fn plan_assets(&self) -> std::collections::HashSet<Id> {
        fn walk(items: &[PlanItem], out: &mut std::collections::HashSet<Id>) {
            for i in items {
                out.extend(i.assets.iter().copied());
                walk(&i.children, out);
            }
        }
        let mut s = std::collections::HashSet::new();
        walk(&self.plan, &mut s);
        s
    }

    // ---------- usage ----------
    /// Asset ids referenced by any clip (main timeline, stash, every sequence).
    pub fn used_assets(&self) -> std::collections::HashSet<Id> {
        let mut s = std::collections::HashSet::new();
        let mut add = |tracks: &[Track]| {
            for c in tracks.iter().flat_map(|t| t.clips.iter()) {
                if c.uses_asset() {
                    s.insert(c.asset);
                }
            }
        };
        add(&self.tracks);
        if let Some(st) = &self.main_stash {
            add(&st.tracks);
        }
        for seq in &self.sequences {
            add(&seq.tracks);
        }
        s
    }
    /// Remove assets no clip uses (planner moodboard assets are kept). Returns how many were removed.
    pub fn remove_unused_assets(&mut self) -> usize {
        let used = self.used_assets();
        let planned = self.plan_assets();
        let before = self.assets.len();
        self.assets.retain(|a| used.contains(&a.id) || planned.contains(&a.id));
        before - self.assets.len()
    }

    // ---------- auto-cut ----------
    /// Split the clips (+ linked) at every time in `cuts`, then delete the pieces lying inside any of the
    /// `remove` ranges (timeline times, half-open), optionally closing the gaps (ripple). Returns the number
    /// of pieces removed.
    pub fn auto_cut(&mut self, ids: &[Id], cuts: &[f64], remove: &[(f64, f64)], ripple: bool) -> usize {
        let ids = self.expand_links(ids);
        let mut group = ids.clone();
        for &t in cuts {
            let new = self.split_at(t, Some(&group));
            group.extend(new);
        }
        let victims: Vec<Id> = group
            .iter()
            .filter_map(|&id| self.clip(id))
            .filter(|c| {
                let mid = c.start + c.duration / 2.0;
                remove.iter().any(|&(a, b)| mid >= a && mid < b)
            })
            .map(|c| c.id)
            .collect();
        let n = victims.len();
        if n > 0 {
            self.delete_clips(&victims, ripple);
        }
        n
    }

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
            if c.uses_asset() {
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

    // ---------- motion helpers ----------
    /// Make two abutting clips "flow": for every visual property animated on either clip, the outgoing
    /// value/velocity of `a` continues into `b` (a gets an ease-in to the cut, b an ease-out from it, and
    /// both meet at the same value at the cut). Returns false if the clips don't abut.
    pub fn flow_clips(&mut self, a: Id, b: Id) -> bool {
        let (Some(ca), Some(cb)) = (self.clip(a).cloned(), self.clip(b).cloned()) else { return false };
        if (ca.end() - cb.start).abs() > ABUT_EPS {
            return false;
        }
        let props = ["x", "y", "scale", "rotation", "opacity"];
        fn pick(c: &mut Clip, i: usize) -> &mut Animated {
            match i {
                0 => &mut c.x,
                1 => &mut c.y,
                2 => &mut c.scale,
                3 => &mut c.rotation,
                _ => &mut c.opacity,
            }
        }
        let (da, db) = (ca.duration, cb.duration);
        let mut na = ca.clone();
        let mut nb = cb.clone();
        for i in 0..props.len() {
            let va_end = pick(&mut na, i).at(da);
            let vb_start = pick(&mut nb, i).at(0.0);
            let animated = pick(&mut na, i).is_animated() || pick(&mut nb, i).is_animated();
            if !animated {
                continue;
            }
            let meet = (va_end + vb_start) / 2.0;
            let pa = pick(&mut na, i);
            if !pa.is_animated() {
                pa.toggle_key(0.0);
            }
            pa.set_at(da, meet);
            if let Some(k) = pa.keys.iter_mut().rev().nth(1) {
                k.ease = Ease::EaseIn;
            }
            let pb = pick(&mut nb, i);
            if !pb.is_animated() {
                pb.toggle_key(db);
            }
            pb.set_at(0.0, meet);
            if let Some(k) = pb.keys.first_mut() {
                k.ease = Ease::EaseOut;
            }
        }
        *self.clip_mut(a).unwrap() = na;
        *self.clip_mut(b).unwrap() = nb;
        true
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
            .chain(p.tracks.iter().flat_map(|t| t.transitions.iter().map(|x| x.id)))
            .chain(p.subtitles.iter().map(|c| c.id))
            .chain(p.sequences.iter().map(|s| s.id))
            .chain(p.sequences.iter().flat_map(|s| s.tracks.iter().map(|t| t.id)))
            .chain(p.sequences.iter().flat_map(|s| s.tracks.iter().flat_map(|t| t.clips.iter().map(|c| c.id.max(c.link)))))
            .chain(p.main_stash.iter().flat_map(|s| s.tracks.iter().flat_map(|t| t.clips.iter().map(|c| c.id.max(c.link)))))
            .chain(p.all_clips().map(|(_, c)| c.id.max(c.link)))
            .max()
            .unwrap_or(0);
        fn plan_max(items: &[PlanItem]) -> Id {
            items.iter().map(|i| i.id.max(plan_max(&i.children))).max().unwrap_or(0)
        }
        p.next_id = p.next_id.max(max_id).max(plan_max(&p.plan));
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
            folder: String::new(),
            tags: Vec::new(),
            label: 0,
            description: String::new(),
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
    fn easing_and_key_moves() {
        let mut a = Animated::new(0.0);
        a.toggle_key(0.0);
        a.set_at(2.0, 10.0);
        a.set_ease_at(0.0, Ease::Hold);
        assert_eq!(a.at(1.0), 0.0);
        a.set_ease_at(0.0, Ease::EaseInOut);
        assert!((a.at(1.0) - 5.0).abs() < 1e-9);
        assert!(a.at(0.5) < 2.5);
        a.set_ease_at(0.0, Ease::EaseIn);
        assert!((a.at(1.0) - 2.5).abs() < 1e-9);
        let j = a.move_key(1, -1.0); // moves before the first key and stays sorted
        assert_eq!(j, 0);
        assert_eq!(a.keys[0].v, 10.0);
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
        p.add_cue(0.5, 1.5, "hi");
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
        c.trim_start(0.0, 1.0); // headroom 1 s → clamped to start - 1 = 1.0
        assert!((c.start - 1.0).abs() < 1e-9);
        assert!((c.src_in).abs() < 1e-9);
        assert!((c.duration - 6.0).abs() < 1e-9);
        assert!((c.opacity.keys[0].t - 2.0).abs() < 1e-9);
        c.trim_end(100.0, 10.0 - c.src_in);
        assert!((c.duration - 10.0).abs() < 1e-9);
        // images/text are unbounded on both sides
        let mut img = Clip::new(2, ClipKind::Image, "i", 5.0, 5.0);
        img.trim_start(3.0, f64::INFINITY);
        assert!((img.start - 3.0).abs() < 1e-9);
        assert!((img.duration - 7.0).abs() < 1e-9);
    }

    #[test]
    fn speed_reverse_freeze() {
        let mut p = Project::from_media(asset(0, 10.0, 1));
        let v = p.tracks[0].clips[0].id;
        // 2x: source window stays [0,10), duration halves, audio follows
        assert!(p.set_speed(&[v], 2.0, false));
        let c = p.clip(v).unwrap().clone();
        assert!((c.duration - 5.0).abs() < 1e-9);
        assert!((c.src_time(1.0) - 2.0).abs() < 1e-9);
        assert!((p.tracks[1].clips[0].duration - 5.0).abs() < 1e-9);
        assert!((p.max_clip_duration(&c) - 5.0).abs() < 1e-9);
        // split at 2 s: right half starts at source 4 s
        p.split_at(2.0, None);
        assert!((p.tracks[0].clips[1].src_in - 4.0).abs() < 1e-9);
        assert!((p.tracks[0].clips[1].src_time(3.0) - 6.0).abs() < 1e-9);
        // reverse the right half: t=2 shows source 10, t=5 shows source 4
        let r = p.tracks[0].clips[1].id;
        assert!(p.set_speed(&[r], 2.0, true));
        let c = p.clip(r).unwrap().clone();
        assert!((c.src_time(2.0) - 10.0).abs() < 1e-9);
        assert!((c.src_time(5.0) - 4.0).abs() < 1e-9);
        // reversed: head room is what is left after src_end (nothing), max duration adds src_in/speed
        assert!((p.head_room(&c)).abs() < 1e-9);
        assert!((p.max_clip_duration(&c) - 5.0).abs() < 1e-9);
        // extend the right edge by 1 s → earliest source time moves 2 s earlier
        let mut c2 = c.clone();
        c2.trim_end(6.0, p.max_clip_duration(&c));
        assert!((c2.src_in - 2.0).abs() < 1e-9);
        assert!((c2.src_time(6.0) - 2.0).abs() < 1e-9);
        // freeze at t=3 splits and holds source 8 (2x reversed clip: 10 - 1*2)
        let frozen = p.freeze_at(3.0, &[r]);
        assert_eq!(frozen.len(), 2); // video + linked audio
        let f = p.clip(frozen[0]).unwrap();
        assert_eq!(f.freeze, Some(8.0));
        assert!((f.src_time(4.0) - 8.0).abs() < 1e-9);
        assert!(p.max_clip_duration(f).is_infinite());
    }

    #[test]
    fn transitions_add_and_prune() {
        let mut p = Project::from_media(asset(0, 10.0, 1));
        let right = p.split_at(4.0, None)[0];
        let id = p.add_transition(right, TransitionKind::CrossFade, 1.0).unwrap();
        assert_eq!(p.tracks[0].transitions.len(), 1);
        assert_eq!(p.tracks[1].transitions.len(), 1); // mirrored on the linked audio cut
        let (tr, l, r) = p.tracks[0].transition_at(3.9).unwrap();
        assert_eq!(tr.id, id);
        assert!(l.end() == r.start);
        assert!((tr.progress(r.start, 3.5) - 0.0).abs() < 1e-9);
        assert!((tr.progress(r.start, 4.5) - 1.0).abs() < 1e-9);
        assert!(p.tracks[0].transition_at(4.6).is_none());
        // no left neighbour → None
        let first = p.tracks[0].clips[0].id;
        assert!(p.add_transition(first, TransitionKind::Push, 1.0).is_none());
        // moving the right clip away invalidates the transition
        assert!(p.move_clips(&p.expand_links(&[right]), 2.0, 0, None));
        assert!(p.tracks[0].transitions.is_empty());
        assert!(p.tracks[1].transitions.is_empty());
    }

    #[test]
    fn effects_params_and_keys() {
        let mut c = Clip::new(1, ClipKind::Video, "c", 0.0, 4.0);
        c.effects.push(Effect::new(EffectKind::Blur));
        assert_eq!(c.effects[0].at(0, 1.0), 8.0);
        c.effects[0].params[0].toggle_key(1.0);
        c.effects[0].params[0].set_at(3.0, 20.0);
        assert_eq!(c.key_times(), vec![1.0, 3.0]);
        c.move_keys(3.0, 2.0);
        assert_eq!(c.key_times(), vec![1.0, 2.0]);
        assert!(c.has_effects());
        let json = serde_json::to_string(&c).unwrap();
        let d: Clip = serde_json::from_str(&json).unwrap();
        assert_eq!(d.effects[0].kind, EffectKind::Blur);
    }

    #[test]
    fn subtitles_and_folders() {
        let mut p = Project::new();
        p.add_cue(2.0, 3.0, "b");
        p.add_cue(0.0, 1.0, "a");
        assert_eq!(p.subtitles[0].text, "a");
        assert_eq!(p.cue_at(2.5).unwrap().text, "b");
        assert!(p.cue_at(1.5).is_none());
        assert!(p.add_folder("Footage/Day 1"));
        assert!(!p.add_folder("  "));
        let aid = p.add_asset(asset(0, 1.0, 0));
        p.asset_mut(aid).unwrap().folder = "Footage/Day 1".into();
        assert_eq!(p.folder_names(), vec!["Footage/Day 1".to_string()]);
        p.remove_folder("Footage");
        assert!(p.folders.is_empty());
        assert_eq!(p.asset(aid).unwrap().folder, "");
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
    fn sequences_open_close_nest() {
        let mut p = Project::from_media(asset(0, 10.0, 1));
        let v = p.tracks[0].clips[0].id;
        // nest the whole clip (+ audio) into a sequence
        let seq = p.nest_selection(&[v], "Intro").unwrap();
        assert_eq!(p.sequences.len(), 1);
        assert_eq!(p.sequence(seq).unwrap().tracks.len(), 2);
        assert_eq!(p.tracks[0].clips.len(), 1);
        assert_eq!(p.tracks[0].clips[0].kind, ClipKind::Sequence);
        assert!((p.sequence_duration(seq) - 10.0).abs() < 1e-9);
        assert!((p.duration() - 10.0).abs() < 1e-9);
        assert!(p.used_assets().len() == 1); // the asset is used inside the sequence
        // open the sequence for editing: tracks swap, main is stashed
        assert!(p.open_sequence(seq));
        assert_eq!(p.editing, Some(seq));
        assert_eq!(p.tracks[0].clips[0].kind, ClipKind::Video);
        assert!(p.main_stash.is_some());
        // can't place itself inside itself
        assert!(p.insert_sequence_clip(seq, 0.0, None).is_none());
        p.close_sequence();
        assert_eq!(p.editing, None);
        assert_eq!(p.tracks[0].clips[0].kind, ClipKind::Sequence);
        // round trip keeps everything
        let q = Project::from_json(&p.to_json()).unwrap();
        assert_eq!(q.sequences.len(), 1);
        assert!((q.sequence_duration(seq) - 10.0).abs() < 1e-9);
    }

    #[test]
    fn planner_and_unused() {
        let mut p = Project::from_media(asset(0, 10.0, 1));
        let extra = p.add_asset(asset(1, 3.0, 0));
        let third = p.add_asset(asset(2, 3.0, 0));
        let a = p.plan_add(None, "Intro");
        let b = p.plan_add(Some(a), "Hook shot");
        p.plan_item_mut(b).unwrap().assets.push(extra);
        p.plan_item_mut(b).unwrap().done = true;
        assert_eq!(p.plan[0].children[0].title, "Hook shot");
        assert!(p.plan_assets().contains(&extra));
        assert_eq!(p.remove_unused_assets(), 1); // `third` gone, `extra` kept by the moodboard
        assert!(p.asset(extra).is_some() && p.asset(third).is_none());
        p.plan_remove(a);
        assert!(p.plan.is_empty());
    }

    #[test]
    fn auto_cut_removes_quiet_parts() {
        let mut p = Project::from_media(asset(0, 10.0, 1));
        let a = p.tracks[1].clips[0].id;
        // cuts at 2,4,6,8; remove [0,2) and [4,6) and [8,10) → keeps [2,4) and [6,8), rippled together
        let n = p.auto_cut(&[a], &[2.0, 4.0, 6.0, 8.0], &[(0.0, 2.0), (4.0, 6.0), (8.0, 10.0)], true);
        assert_eq!(n, 6); // 3 audio + 3 linked video pieces
        assert_eq!(p.tracks[1].clips.len(), 2);
        assert_eq!(p.tracks[0].clips.len(), 2);
        assert!((p.duration() - 4.0).abs() < 1e-9);
        assert!((p.tracks[0].clips[1].src_in - 6.0).abs() < 1e-9);
    }

    #[test]
    fn flow_and_place() {
        let mut p = Project::new();
        let a = p.add_text_clip(0.0, 2.0);
        let b = p.add_text_clip(2.0, 2.0);
        p.clip_mut(a).unwrap().x.toggle_key(0.0);
        p.clip_mut(a).unwrap().x.set_at(2.0, 100.0);
        p.clip_mut(b).unwrap().x.toggle_key(0.0);
        p.clip_mut(b).unwrap().x.set_at(2.0, -100.0); // b starts at 0 → after flow both meet at 50
        assert!(p.flow_clips(a, b));
        assert!((p.clip(a).unwrap().x.at(2.0) - 50.0).abs() < 1e-9);
        assert!((p.clip(b).unwrap().x.at(0.0) - 50.0).abs() < 1e-9);
        assert_eq!(p.clip(b).unwrap().x.keys[0].ease, Ease::EaseOut);
        // place the two clips again as a template at t=10
        let clips: Vec<Clip> = [a, b].iter().map(|&id| p.clip(id).unwrap().clone()).collect();
        let ids = p.place_clips(clips, Vec::new(), 10.0);
        assert_eq!(ids.len(), 2);
        assert!((p.clip(ids[0]).unwrap().start - 10.0).abs() < 1e-9);
        assert_eq!(p.tracks[0].clips.len(), 4);
    }

    #[test]
    fn bezier_ease() {
        let e = Ease::Bezier { x1: 0.42, y1: 0.0, x2: 0.58, y2: 1.0 };
        assert!((e.apply(0.0)).abs() < 1e-6 && (e.apply(1.0) - 1.0).abs() < 1e-6);
        assert!((e.apply(0.5) - 0.5).abs() < 1e-3);
        assert!(e.apply(0.25) < 0.25); // ease-in start
        let s = serde_json::to_string(&e).unwrap();
        assert_eq!(serde_json::from_str::<Ease>(&s).unwrap(), e);
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
