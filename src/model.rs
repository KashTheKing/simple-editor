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
    /// A vector shape or a recorded drawing (`Clip.shape`).
    Shape,
    /// An automation / adjustment layer: its effects (or node graph) apply to everything composited
    /// below it on lower video tracks, for as long as the clip lasts. Draws nothing of its own.
    Adjustment,
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
    Bezier {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
    },
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
    /// Move key `i` to `new_t`, keeping the list sorted. A key already sitting at `new_t` is replaced
    /// (dragging one keyframe onto another merges them instead of stacking duplicates).
    pub fn move_key(&mut self, i: usize, new_t: f64) -> usize {
        if i >= self.keys.len() {
            return i;
        }
        let mut k = self.keys.remove(i);
        k.t = new_t;
        if let Some(j) = self.key_index_at(new_t) {
            self.keys[j] = k;
            return j;
        }
        let j = self.keys.partition_point(|o| o.t < new_t);
        self.keys.insert(j, k);
        j
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
    // --- round 3 ---
    ChromaKey,
    Curves,
    Levels,
    HueShift,
    JpegCompress,
    MotionBlur,
    Plane3d,
    EdgeGlow,
    Threshold,
    BlobTrack,
    Vhs,
    RecDot,
    /// A user-written GLSL fragment shader (source in `Effect.shader`, up to 8 generic knobs).
    Shader,
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
    pub const ALL: [EffectKind; 24] = [
        EffectKind::Blur,
        EffectKind::MotionBlur,
        EffectKind::Pixelate,
        EffectKind::JpegCompress,
        EffectKind::Vhs,
        EffectKind::ChromaKey,
        EffectKind::Threshold,
        EffectKind::EdgeGlow,
        EffectKind::Tint,
        EffectKind::Color,
        EffectKind::Curves,
        EffectKind::Levels,
        EffectKind::HueShift,
        EffectKind::Grayscale,
        EffectKind::Invert,
        EffectKind::Vignette,
        EffectKind::Sharpen,
        EffectKind::Flip,
        EffectKind::Crop,
        EffectKind::Plane3d,
        EffectKind::Wobble,
        EffectKind::BlobTrack,
        EffectKind::RecDot,
        EffectKind::Shader,
    ];
    /// Catalogue grouping for the effects panel.
    pub fn category(self) -> &'static str {
        use EffectKind::*;
        match self {
            Color | Curves | Levels | HueShift | Tint | Grayscale | Invert | Threshold => "Adjustments",
            Blur | MotionBlur | Sharpen | Pixelate | JpegCompress | Vhs | EdgeGlow | Vignette | RecDot => "Stylize",
            ChromaKey | Crop | BlobTrack => "Keying & Matte",
            Flip | Plane3d | Wobble => "Transform",
            Shader => "Custom",
        }
    }
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
            EffectKind::ChromaKey => "Chroma Key",
            EffectKind::Curves => "Color Curves",
            EffectKind::Levels => "Levels",
            EffectKind::HueShift => "Hue / Saturation",
            EffectKind::JpegCompress => "JPEG Compression",
            EffectKind::MotionBlur => "Motion Blur",
            EffectKind::Plane3d => "3D Plane",
            EffectKind::EdgeGlow => "Edge Glow",
            EffectKind::Threshold => "Threshold",
            EffectKind::BlobTrack => "Blob Tracking",
            EffectKind::Vhs => "VHS",
            EffectKind::RecDot => "Security Camera REC",
            EffectKind::Shader => "Custom Shader",
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
            EffectKind::ChromaKey => P_CHROMA,
            EffectKind::Curves => P_CURVES,
            EffectKind::Levels => P_LEVELS,
            EffectKind::HueShift => P_HUE,
            EffectKind::JpegCompress => P_JPEG,
            EffectKind::MotionBlur => P_MOTIONBLUR,
            EffectKind::Plane3d => P_PLANE3D,
            EffectKind::EdgeGlow => P_EDGEGLOW,
            EffectKind::Threshold => P_THRESHOLD,
            EffectKind::BlobTrack => P_BLOBTRACK,
            EffectKind::Vhs => P_VHS,
            EffectKind::RecDot => P_RECDOT,
            EffectKind::Shader => P_SHADER,
        }
    }
    /// Parameters shown as a checkbox (stored as 0/1) rather than a number.
    pub fn is_bool_param(self, i: usize) -> bool {
        matches!(
            (self, i),
            (EffectKind::Flip, 0)
                | (EffectKind::Flip, 1)
                | (EffectKind::ChromaKey, 5)
                | (EffectKind::Vhs, 6)
                | (EffectKind::RecDot, 3)
                | (EffectKind::BlobTrack, 4)
        )
    }
    /// Effects whose output depends on neighbouring frames (the renderer must supply them).
    pub fn needs_motion(self) -> bool {
        self == EffectKind::MotionBlur
    }
    /// Effects the compositor applies by moving the layer instead of touching pixels.
    pub fn is_geometric(self) -> bool {
        matches!(self, EffectKind::Wobble | EffectKind::Plane3d)
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
const P_CHROMA: &[ParamSpec] = &[
    ps("Key R", 0.0, 0.0, 255.0),
    ps("Key G", 255.0, 0.0, 255.0),
    ps("Key B", 0.0, 0.0, 255.0),
    ps("Similarity", 0.4, 0.0, 1.0),
    ps("Smoothness", 0.1, 0.0, 1.0),
    ps("Show mask", 0.0, 0.0, 1.0),
    ps("Spill removal", 0.5, 0.0, 1.0),
    ps("Edge shrink", 0.0, -5.0, 5.0),
];
/// Five control points per channel (input -> output, 0..1) drive a monotone spline; the UI draws a
/// classic curve editor and writes these values.
const P_CURVES: &[ParamSpec] = &[
    ps("Master 1/4", 0.25, 0.0, 1.0),
    ps("Master 2/4", 0.5, 0.0, 1.0),
    ps("Master 3/4", 0.75, 0.0, 1.0),
    ps("Red 1/4", 0.25, 0.0, 1.0),
    ps("Red 2/4", 0.5, 0.0, 1.0),
    ps("Red 3/4", 0.75, 0.0, 1.0),
    ps("Green 1/4", 0.25, 0.0, 1.0),
    ps("Green 2/4", 0.5, 0.0, 1.0),
    ps("Green 3/4", 0.75, 0.0, 1.0),
    ps("Blue 1/4", 0.25, 0.0, 1.0),
    ps("Blue 2/4", 0.5, 0.0, 1.0),
    ps("Blue 3/4", 0.75, 0.0, 1.0),
];
const P_LEVELS: &[ParamSpec] = &[
    ps("In black", 0.0, 0.0, 1.0),
    ps("In white", 1.0, 0.0, 1.0),
    ps("Gamma", 1.0, 0.1, 5.0),
    ps("Out black", 0.0, 0.0, 1.0),
    ps("Out white", 1.0, 0.0, 1.0),
];
const P_HUE: &[ParamSpec] =
    &[ps("Hue", 0.0, -180.0, 180.0), ps("Saturation", 1.0, 0.0, 3.0), ps("Lightness", 0.0, -1.0, 1.0)];
const P_JPEG: &[ParamSpec] =
    &[ps("Quality", 20.0, 1.0, 100.0), ps("Block size", 8.0, 2.0, 32.0), ps("Chroma subsample", 1.0, 0.0, 1.0)];
const P_MOTIONBLUR: &[ParamSpec] =
    &[ps("Shutter angle", 180.0, 0.0, 360.0), ps("Samples", 8.0, 2.0, 32.0), ps("Adaptive", 1.0, 0.0, 1.0)];
/// A 3D plane: the layer is mapped through a perspective transform (also usable as a node).
const P_PLANE3D: &[ParamSpec] = &[
    ps("Yaw", 0.0, -89.0, 89.0),
    ps("Pitch", 0.0, -89.0, 89.0),
    ps("Roll", 0.0, -180.0, 180.0),
    ps("Distance", 2.0, 0.5, 20.0),
    ps("Field of view", 45.0, 5.0, 120.0),
    ps("Offset Z", 0.0, -5.0, 5.0),
];
const P_EDGEGLOW: &[ParamSpec] = &[
    ps("Threshold", 0.2, 0.0, 1.0),
    ps("Width", 2.0, 0.5, 20.0),
    ps("Glow", 1.0, 0.0, 4.0),
    ps("R", 120.0, 0.0, 255.0),
    ps("G", 200.0, 0.0, 255.0),
    ps("B", 255.0, 0.0, 255.0),
    ps("Keep source", 1.0, 0.0, 1.0),
];
const P_THRESHOLD: &[ParamSpec] =
    &[ps("Level", 0.5, 0.0, 1.0), ps("Softness", 0.05, 0.0, 0.5), ps("Per channel", 0.0, 0.0, 1.0)];
/// Tracks the largest blob matching a colour; its centre drives `BlobTrack`-linked properties.
const P_BLOBTRACK: &[ParamSpec] = &[
    ps("Target R", 255.0, 0.0, 255.0),
    ps("Target G", 0.0, 0.0, 255.0),
    ps("Target B", 0.0, 0.0, 255.0),
    ps("Tolerance", 0.25, 0.0, 1.0),
    ps("Show overlay", 1.0, 0.0, 1.0),
    ps("Smoothing", 0.5, 0.0, 1.0),
];
const P_VHS: &[ParamSpec] = &[
    ps("Noise", 0.3, 0.0, 1.0),
    ps("Chroma bleed", 0.5, 0.0, 1.0),
    ps("Scanlines", 0.4, 0.0, 1.0),
    ps("Tracking jitter", 0.2, 0.0, 1.0),
    ps("Head switching", 0.3, 0.0, 1.0),
    ps("Sharpen ringing", 0.4, 0.0, 1.0),
    ps("Colour bleed only", 0.0, 0.0, 1.0),
    ps("Tape wear", 0.2, 0.0, 1.0),
];
const P_RECDOT: &[ParamSpec] = &[
    ps("Size", 18.0, 2.0, 200.0),
    ps("Blink Hz", 0.5, 0.0, 5.0),
    ps("Corner", 0.0, 0.0, 3.0),
    ps("Timecode", 1.0, 0.0, 1.0),
    ps("Margin", 40.0, 0.0, 500.0),
];
const P_SHADER: &[ParamSpec] = &[
    ps("u1", 0.0, -10.0, 10.0),
    ps("u2", 0.0, -10.0, 10.0),
    ps("u3", 0.0, -10.0, 10.0),
    ps("u4", 0.0, -10.0, 10.0),
    ps("u5", 0.0, -10.0, 10.0),
    ps("u6", 0.0, -10.0, 10.0),
    ps("u7", 0.0, -10.0, 10.0),
    ps("u8", 0.0, -10.0, 10.0),
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

/// Starting point for `EffectKind::Shader`: `tex` = the layer, `uv` = 0..1, `u_time` = clip-local
/// seconds, `u1..u8` = the eight knobs, `u_res` = layer size in px. Output goes to `out_color`
/// (straight alpha, same convention as every other effect).
pub const DEFAULT_SHADER: &str = r#"// custom effect — edit freely
vec4 effect(vec4 src, vec2 uv) {
    // u1 = amount, u2 = speed
    float wave = sin(uv.y * 40.0 + u_time * max(u2, 0.0) * 6.28318) * u1 * 0.02;
    return texture(tex, vec2(uv.x + wave, uv.y));
}
"#;

/// A mask shape. Points are in project pixels relative to the layer centre; a mask limits where an
/// effect applies (or where the whole clip is visible when it sits on `Clip.mask`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Hash, Default)]
pub enum MaskShape {
    #[default]
    Rect,
    Ellipse,
    /// Straight-edged polygon through `points`.
    Polygon,
    /// Free-hand / bezier path through `points` (smoothed).
    Path,
}

impl MaskShape {
    pub const ALL: [MaskShape; 4] = [MaskShape::Rect, MaskShape::Ellipse, MaskShape::Polygon, MaskShape::Path];
    pub fn name(self) -> &'static str {
        match self {
            MaskShape::Rect => "Rectangle",
            MaskShape::Ellipse => "Ellipse",
            MaskShape::Polygon => "Polygon",
            MaskShape::Path => "Path",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Mask {
    pub shape: MaskShape,
    /// Rect/Ellipse: centre + half-size. Polygon/Path: ignored (see `points`).
    pub cx: Animated,
    pub cy: Animated,
    pub rx: Animated,
    pub ry: Animated,
    /// Polygon/Path vertices (project px, relative to the layer centre).
    pub points: Vec<(f32, f32)>,
    pub rotation: Animated,
    /// Soft edge in project px.
    pub feather: Animated,
    /// Grow (+) / shrink (-) the shape in project px.
    pub expand: Animated,
    /// Mask strength 0..1.
    pub opacity: Animated,
    pub invert: bool,
    pub enabled: bool,
}

impl Default for Mask {
    fn default() -> Self {
        Self {
            shape: MaskShape::Rect,
            cx: Animated::new(0.0),
            cy: Animated::new(0.0),
            rx: Animated::new(200.0),
            ry: Animated::new(200.0),
            points: Vec::new(),
            rotation: Animated::new(0.0),
            feather: Animated::new(0.0),
            expand: Animated::new(0.0),
            opacity: Animated::new(1.0),
            invert: false,
            enabled: true,
        }
    }
}

impl Mask {
    pub fn new(shape: MaskShape) -> Self {
        Self { shape, ..Default::default() }
    }
    pub fn animated_mut(&mut self) -> Vec<&mut Animated> {
        vec![
            &mut self.cx,
            &mut self.cy,
            &mut self.rx,
            &mut self.ry,
            &mut self.rotation,
            &mut self.feather,
            &mut self.expand,
            &mut self.opacity,
        ]
    }
    pub fn animated(&self) -> Vec<&Animated> {
        vec![&self.cx, &self.cy, &self.rx, &self.ry, &self.rotation, &self.feather, &self.expand, &self.opacity]
    }
}

/// An effect instance on a clip; `params[i]` follows `kind.params()[i]` (each keyframeable).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Effect {
    pub kind: EffectKind,
    #[serde(default = "tru")]
    pub enabled: bool,
    #[serde(default)]
    pub params: Vec<Animated>,
    /// Limits the effect to (or outside) a shape.
    #[serde(default)]
    pub mask: Option<Mask>,
    /// `EffectKind::Shader` only: the GLSL fragment shader source.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub shader: String,
}

impl Effect {
    pub fn new(kind: EffectKind) -> Self {
        Self {
            kind,
            enabled: true,
            params: kind.params().iter().map(|p| Animated::new(p.default)).collect(),
            mask: None,
            shader: if kind == EffectKind::Shader { DEFAULT_SHADER.to_string() } else { String::new() },
        }
    }
    /// Parameter i at clip-local time t (spec default when missing, e.g. older files).
    pub fn at(&self, i: usize, t: f64) -> f64 {
        self.params
            .get(i)
            .map(|a| a.at(t))
            .unwrap_or_else(|| self.kind.params().get(i).map(|p| p.default).unwrap_or(0.0))
    }
    pub fn specs(&self) -> &'static [ParamSpec] {
        self.kind.params()
    }
}

// ---------- node graph ----------

/// What a node does. `Input` is the clip's own decoded layer; `Output` is what the compositor draws.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum NodeKind {
    /// The clip's decoded layer (or, in an adjustment clip, everything below it).
    Input,
    /// A solid colour (RGBA) filling the canvas.
    Color([u8; 4]),
    /// Another clip's layer at the same time (compositing across tracks).
    Clip(Id),
    Effect(Effect),
    /// Combines two inputs (`a` under `b`) with a blend mode and opacity.
    Blend {
        mode: BlendMode,
        opacity: Animated,
    },
    /// Uses `b`'s luminance (or alpha) as a matte for `a`.
    Matte {
        invert: bool,
        use_alpha: bool,
    },
    /// A standalone mask (matte generator).
    Mask(Mask),
    Output,
}

impl NodeKind {
    /// How many inputs the node consumes.
    pub fn inputs(&self) -> usize {
        match self {
            NodeKind::Input | NodeKind::Color(_) | NodeKind::Clip(_) | NodeKind::Mask(_) => 0,
            NodeKind::Effect(_) | NodeKind::Output => 1,
            NodeKind::Blend { .. } | NodeKind::Matte { .. } => 2,
        }
    }
    pub fn title(&self) -> String {
        match self {
            NodeKind::Input => "Input".into(),
            NodeKind::Color(_) => "Color".into(),
            NodeKind::Clip(_) => "Clip".into(),
            NodeKind::Effect(e) => e.kind.name().into(),
            NodeKind::Blend { .. } => "Blend".into(),
            NodeKind::Matte { .. } => "Matte".into(),
            NodeKind::Mask(m) => format!("Mask ({})", m.shape.name()),
            NodeKind::Output => "Output".into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Node {
    pub id: Id,
    pub kind: NodeKind,
    /// Editor position (graph units).
    pub x: f32,
    pub y: f32,
    #[serde(default = "tru")]
    pub enabled: bool,
}

/// `to`'s input `port` is fed by `from`'s output.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct Edge {
    pub from: Id,
    pub to: Id,
    pub port: usize,
}

/// A clip's effect chain as a DAG. When present it replaces `Clip.effects` (kept as the simple linear
/// stack for clips that never opened the node editor). Always contains exactly one `Output`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct NodeGraph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

impl NodeGraph {
    /// Input -> Output, ready to have effects dropped in between.
    pub fn new(next_id: &mut impl FnMut() -> Id) -> Self {
        let (i, o) = (next_id(), next_id());
        Self {
            nodes: vec![
                Node { id: i, kind: NodeKind::Input, x: 0.0, y: 0.0, enabled: true },
                Node { id: o, kind: NodeKind::Output, x: 320.0, y: 0.0, enabled: true },
            ],
            edges: vec![Edge { from: i, to: o, port: 0 }],
        }
    }
    /// A graph equivalent to a linear effect stack (used when a clip's stack is converted).
    pub fn from_effects(effects: &[Effect], next_id: &mut impl FnMut() -> Id) -> Self {
        let mut g = Self::new(next_id);
        let out = g.output().unwrap();
        let mut prev = g.nodes[0].id;
        g.edges.clear();
        for (i, e) in effects.iter().enumerate() {
            let id = next_id();
            g.nodes.push(Node {
                id,
                kind: NodeKind::Effect(e.clone()),
                x: 160.0 * (i + 1) as f32,
                y: 0.0,
                enabled: e.enabled,
            });
            g.edges.push(Edge { from: prev, to: id, port: 0 });
            prev = id;
        }
        if let Some(o) = g.nodes.iter_mut().find(|n| n.id == out) {
            o.x = 160.0 * (effects.len() + 1) as f32;
        }
        g.edges.push(Edge { from: prev, to: out, port: 0 });
        g
    }
    pub fn node(&self, id: Id) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }
    pub fn node_mut(&mut self, id: Id) -> Option<&mut Node> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }
    pub fn output(&self) -> Option<Id> {
        self.nodes.iter().find(|n| n.kind == NodeKind::Output).map(|n| n.id)
    }
    /// The node feeding `to`'s input `port`.
    pub fn input_of(&self, to: Id, port: usize) -> Option<Id> {
        self.edges.iter().find(|e| e.to == to && e.port == port).map(|e| e.from)
    }
    /// Connect (replacing whatever fed that port). Refused when it would create a cycle.
    pub fn connect(&mut self, from: Id, to: Id, port: usize) -> bool {
        if from == to || self.node(from).is_none() || self.node(to).is_none() {
            return false;
        }
        let prev: Vec<Edge> = self.edges.iter().filter(|e| e.to == to && e.port == port).copied().collect();
        self.edges.retain(|e| !(e.to == to && e.port == port));
        self.edges.push(Edge { from, to, port });
        if self.has_cycle() {
            self.edges.pop();
            self.edges.extend(prev);
            return false;
        }
        true
    }
    pub fn disconnect(&mut self, to: Id, port: usize) {
        self.edges.retain(|e| !(e.to == to && e.port == port));
    }
    /// Remove a node (the Output can never be removed) and re-link its first input to its consumers.
    pub fn remove_node(&mut self, id: Id) {
        if self.node(id).map(|n| n.kind == NodeKind::Output).unwrap_or(true) {
            return;
        }
        let up = self.input_of(id, 0);
        let down: Vec<(Id, usize)> = self.edges.iter().filter(|e| e.from == id).map(|e| (e.to, e.port)).collect();
        self.nodes.retain(|n| n.id != id);
        self.edges.retain(|e| e.from != id && e.to != id);
        if let Some(up) = up {
            for (to, port) in down {
                self.connect(up, to, port);
            }
        }
    }
    /// Depth-first cycle check (the editor refuses connections that would loop).
    pub fn has_cycle(&self) -> bool {
        fn visit(g: &NodeGraph, id: Id, state: &mut std::collections::HashMap<Id, u8>) -> bool {
            match state.get(&id) {
                Some(1) => return true,
                Some(2) => return false,
                _ => {}
            }
            state.insert(id, 1);
            let ins: Vec<Id> = g.edges.iter().filter(|e| e.to == id).map(|e| e.from).collect();
            for i in ins {
                if visit(g, i, state) {
                    return true;
                }
            }
            state.insert(id, 2);
            false
        }
        let mut state = std::collections::HashMap::new();
        self.nodes.iter().any(|n| visit(self, n.id, &mut state))
    }
    /// Nodes reachable from the output, inputs before consumers.
    pub fn eval_order(&self) -> Vec<Id> {
        let Some(out) = self.output() else { return Vec::new() };
        let mut order = Vec::new();
        let mut seen = std::collections::HashSet::new();
        fn walk(g: &NodeGraph, id: Id, seen: &mut std::collections::HashSet<Id>, order: &mut Vec<Id>) {
            if !seen.insert(id) {
                return;
            }
            let mut ins: Vec<(usize, Id)> = g.edges.iter().filter(|e| e.to == id).map(|e| (e.port, e.from)).collect();
            ins.sort();
            for (_, from) in ins {
                walk(g, from, seen, order);
            }
            order.push(id);
        }
        walk(self, out, &mut seen, &mut order);
        order
    }
    /// Every animated property in the graph (effect params, blend opacity, mask properties).
    pub fn animated_mut(&mut self) -> Vec<&mut Animated> {
        let mut v = Vec::new();
        for n in &mut self.nodes {
            match &mut n.kind {
                NodeKind::Effect(e) => {
                    v.extend(e.params.iter_mut());
                    if let Some(m) = &mut e.mask {
                        v.extend(m.animated_mut());
                    }
                }
                NodeKind::Blend { opacity, .. } => v.push(opacity),
                NodeKind::Mask(m) => v.extend(m.animated_mut()),
                _ => {}
            }
        }
        v
    }
    pub fn animated(&self) -> Vec<&Animated> {
        let mut v = Vec::new();
        for n in &self.nodes {
            match &n.kind {
                NodeKind::Effect(e) => {
                    v.extend(e.params.iter());
                    if let Some(m) = &e.mask {
                        v.extend(m.animated());
                    }
                }
                NodeKind::Blend { opacity, .. } => v.push(opacity),
                NodeKind::Mask(m) => v.extend(m.animated()),
                _ => {}
            }
        }
        v
    }
}

// ---------- shapes & drawing ----------

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Hash, Default)]
pub enum ShapeKind {
    #[default]
    Rect,
    Ellipse,
    Triangle,
    Polygon,
    Star,
    Line,
    Arrow,
    /// Free-hand strokes (see `ShapeStyle.strokes`).
    Draw,
}

impl ShapeKind {
    pub const ALL: [ShapeKind; 8] = [
        ShapeKind::Rect,
        ShapeKind::Ellipse,
        ShapeKind::Triangle,
        ShapeKind::Polygon,
        ShapeKind::Star,
        ShapeKind::Line,
        ShapeKind::Arrow,
        ShapeKind::Draw,
    ];
    pub fn name(self) -> &'static str {
        match self {
            ShapeKind::Rect => "Rectangle",
            ShapeKind::Ellipse => "Ellipse",
            ShapeKind::Triangle => "Triangle",
            ShapeKind::Polygon => "Polygon",
            ShapeKind::Star => "Star",
            ShapeKind::Line => "Line",
            ShapeKind::Arrow => "Arrow",
            ShapeKind::Draw => "Drawing",
        }
    }
}

/// One free-hand stroke: points in project px (relative to the layer centre) with the clip-local time
/// each point was drawn, so a recorded sketch can play back at any rate.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct Stroke {
    pub color: [u8; 4],
    pub width: f32,
    /// (x, y, t) - t in clip-local seconds at the recording rate.
    pub points: Vec<(f32, f32, f32)>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ShapeStyle {
    pub kind: ShapeKind,
    pub fill: [u8; 4],
    pub stroke: [u8; 4],
    pub stroke_width: f32,
    /// Rect corner radius / Arrow head size (project px).
    pub corner: f32,
    /// Polygon / Star sides.
    pub sides: u32,
    /// Half-size in project px (Rect/Ellipse/...) or the offset of the far end (Line/Arrow).
    pub w: Animated,
    pub h: Animated,
    /// Free-hand strokes for `ShapeKind::Draw`.
    pub strokes: Vec<Stroke>,
    /// Playback rate of a recorded drawing (1 = as recorded, 2 = twice as fast, 0 = all at once).
    pub draw_rate: f32,
    /// Page behind a drawing (alpha 0 = transparent, e.g. a white sketch page).
    pub page: [u8; 4],
}

impl Default for ShapeStyle {
    fn default() -> Self {
        Self {
            kind: ShapeKind::Rect,
            fill: [255, 255, 255, 255],
            stroke: [0, 0, 0, 0],
            stroke_width: 4.0,
            corner: 0.0,
            sides: 5,
            w: Animated::new(300.0),
            h: Animated::new(200.0),
            strokes: Vec::new(),
            draw_rate: 1.0,
            page: [0, 0, 0, 0],
        }
    }
}

impl ShapeStyle {
    pub fn new(kind: ShapeKind) -> Self {
        let mut s = Self { kind, ..Default::default() };
        if matches!(kind, ShapeKind::Line | ShapeKind::Arrow | ShapeKind::Draw) {
            s.stroke = [255, 255, 255, 255];
            s.fill = [0, 0, 0, 0];
        }
        s
    }
    /// Recorded length of a drawing in seconds.
    pub fn draw_duration(&self) -> f64 {
        self.strokes.iter().flat_map(|s| s.points.iter().map(|p| p.2 as f64)).fold(0.0, f64::max)
    }
    pub fn cache_key(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.kind.hash(&mut h);
        (self.fill, self.stroke, self.page).hash(&mut h);
        for f in [self.stroke_width, self.corner, self.draw_rate] {
            f.to_bits().hash(&mut h);
        }
        self.sides.hash(&mut h);
        self.strokes.len().hash(&mut h);
        for st in &self.strokes {
            st.points.len().hash(&mut h);
            st.color.hash(&mut h);
            st.width.to_bits().hash(&mut h);
        }
        h.finish()
    }
}

// ---------- markers ----------

/// A note on the timeline (or, in `Clip.markers`, on a clip). The AI tools read these too.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Marker {
    pub id: Id,
    /// Timeline seconds (project markers) or clip-local seconds (clip markers).
    pub t: f64,
    /// 0 = a point marker; > 0 = a range.
    pub duration: f64,
    pub name: String,
    pub note: String,
    /// Index into `Project.labels` + 1 (0 = none).
    pub label: u8,
}

impl Default for Marker {
    fn default() -> Self {
        Self { id: 0, t: 0.0, duration: 0.0, name: String::new(), note: String::new(), label: 0 }
    }
}

// ---------- labels ----------

/// A user-editable colour label. `Project.labels` starts as `default_labels()`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Label {
    pub name: String,
    pub color: [u8; 3],
}

pub fn default_labels() -> Vec<Label> {
    LABEL_COLORS.iter().map(|(n, c)| Label { name: (*n).to_string(), color: *c }).collect()
}

// ---------- audio buses ----------

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Hash)]
pub enum FilterKind {
    /// 3-band parametric EQ (low shelf, peak, high shelf).
    Eq,
    HighPass,
    LowPass,
    Reverb,
    Echo,
    Distortion,
    Compressor,
    NoiseGate,
    /// Adds noise (white / pink / a pure tone).
    Noise,
    Gain,
}

impl FilterKind {
    pub const ALL: [FilterKind; 10] = [
        FilterKind::Eq,
        FilterKind::HighPass,
        FilterKind::LowPass,
        FilterKind::Reverb,
        FilterKind::Echo,
        FilterKind::Distortion,
        FilterKind::Compressor,
        FilterKind::NoiseGate,
        FilterKind::Noise,
        FilterKind::Gain,
    ];
    pub fn name(self) -> &'static str {
        match self {
            FilterKind::Eq => "EQ (3-band)",
            FilterKind::HighPass => "High-pass",
            FilterKind::LowPass => "Low-pass",
            FilterKind::Reverb => "Reverb",
            FilterKind::Echo => "Echo / Delay",
            FilterKind::Distortion => "Distortion",
            FilterKind::Compressor => "Compressor",
            FilterKind::NoiseGate => "Noise gate",
            FilterKind::Noise => "Noise",
            FilterKind::Gain => "Gain",
        }
    }
    pub fn params(self) -> &'static [ParamSpec] {
        match self {
            FilterKind::Eq => F_EQ,
            FilterKind::HighPass | FilterKind::LowPass => F_PASS,
            FilterKind::Reverb => F_REVERB,
            FilterKind::Echo => F_ECHO,
            FilterKind::Distortion => F_DIST,
            FilterKind::Compressor => F_COMP,
            FilterKind::NoiseGate => F_GATE,
            FilterKind::Noise => F_NOISE,
            FilterKind::Gain => F_GAIN,
        }
    }
}

const F_EQ: &[ParamSpec] = &[
    ps("Low gain dB", 0.0, -24.0, 24.0),
    ps("Low freq", 120.0, 20.0, 1000.0),
    ps("Mid gain dB", 0.0, -24.0, 24.0),
    ps("Mid freq", 1000.0, 100.0, 8000.0),
    ps("Mid Q", 1.0, 0.1, 10.0),
    ps("High gain dB", 0.0, -24.0, 24.0),
    ps("High freq", 6000.0, 1000.0, 20000.0),
];
const F_PASS: &[ParamSpec] = &[ps("Frequency", 200.0, 20.0, 20000.0), ps("Resonance", 0.7, 0.1, 10.0)];
const F_REVERB: &[ParamSpec] = &[
    ps("Room size", 0.5, 0.0, 1.0),
    ps("Damping", 0.5, 0.0, 1.0),
    ps("Width", 1.0, 0.0, 1.0),
    ps("Mix", 0.25, 0.0, 1.0),
    ps("Pre-delay ms", 20.0, 0.0, 200.0),
];
const F_ECHO: &[ParamSpec] = &[
    ps("Delay ms", 350.0, 1.0, 2000.0),
    ps("Feedback", 0.35, 0.0, 0.95),
    ps("Mix", 0.3, 0.0, 1.0),
    ps("Ping-pong", 0.0, 0.0, 1.0),
];
const F_DIST: &[ParamSpec] = &[ps("Drive", 4.0, 1.0, 50.0), ps("Tone", 0.5, 0.0, 1.0), ps("Mix", 1.0, 0.0, 1.0)];
const F_COMP: &[ParamSpec] = &[
    ps("Threshold dB", -18.0, -60.0, 0.0),
    ps("Ratio", 4.0, 1.0, 20.0),
    ps("Attack ms", 10.0, 0.1, 200.0),
    ps("Release ms", 120.0, 5.0, 2000.0),
    ps("Makeup dB", 0.0, -12.0, 24.0),
];
const F_GATE: &[ParamSpec] =
    &[ps("Threshold dB", -45.0, -80.0, 0.0), ps("Attack ms", 2.0, 0.1, 100.0), ps("Release ms", 120.0, 5.0, 2000.0)];
const F_NOISE: &[ParamSpec] =
    &[ps("Level dB", -40.0, -80.0, 0.0), ps("Type", 0.0, 0.0, 2.0), ps("Tone Hz", 1000.0, 20.0, 18000.0)];
const F_GAIN: &[ParamSpec] = &[ps("Gain dB", 0.0, -60.0, 24.0)];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AudioFilter {
    pub kind: FilterKind,
    #[serde(default = "tru")]
    pub enabled: bool,
    #[serde(default)]
    pub params: Vec<Animated>,
}

impl AudioFilter {
    pub fn new(kind: FilterKind) -> Self {
        Self { kind, enabled: true, params: kind.params().iter().map(|p| Animated::new(p.default)).collect() }
    }
    /// Parameter i at time t (spec default when missing).
    pub fn at(&self, i: usize, t: f64) -> f64 {
        self.params
            .get(i)
            .map(|a| a.at(t))
            .unwrap_or_else(|| self.kind.params().get(i).map(|p| p.default).unwrap_or(0.0))
    }
}

/// A mixer bus. The first bus is always "Main"; every other bus routes into it (or into another bus).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Bus {
    pub id: Id,
    pub name: String,
    pub gain: Animated,
    pub pan: Animated,
    pub muted: bool,
    pub solo: bool,
    /// Fold to mono after the filter chain.
    pub mono: bool,
    pub filters: Vec<AudioFilter>,
    /// Where this bus sends its output (0 = Main). Main sends nowhere.
    pub output: Id,
}

impl Default for Bus {
    fn default() -> Self {
        Self {
            id: 0,
            name: "Bus".into(),
            gain: Animated::new(1.0),
            pan: Animated::new(0.0),
            muted: false,
            solo: false,
            mono: false,
            filters: Vec::new(),
            output: 0,
        }
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
    /// Half the transition length, clamped to the clips it joins: an over-long transition must not
    /// reach past either neighbour (it would hide the clips beside them).
    pub fn half(&self, left: &Clip, right: &Clip) -> f64 {
        (self.duration / 2.0).min(left.duration).min(right.duration)
    }
    /// The window [cut - half, cut + half) actually played, clamped to the clips.
    pub fn window(&self, left: &Clip, right: &Clip) -> (f64, f64) {
        let h = self.half(left, right);
        (right.start - h, right.start + h)
    }
    /// Eased progress 0..1 across a window of `cut ± half`.
    pub fn progress_at(&self, cut: f64, half: f64, t: f64) -> f64 {
        if half <= 0.0 {
            return 1.0;
        }
        self.ease.apply(((t - (cut - half)) / (2.0 * half)).clamp(0.0, 1.0))
    }
    /// Eased progress 0..1 at timeline time t over the clamped window.
    pub fn progress(&self, left: &Clip, right: &Clip, t: f64) -> f64 {
        self.progress_at(right.start, self.half(left, right), t)
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
            bus: 0,
            text: if kind == ClipKind::Text { Some(TextStyle::default()) } else { None },
            shape: if kind == ClipKind::Shape { Some(ShapeStyle::default()) } else { None },
            mask: None,
            graph: None,
            markers: Vec::new(),
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
    /// Draws pixels of its own (an Adjustment layer only re-processes what is below it).
    pub fn draws(&self) -> bool {
        self.is_visual() && self.kind != ClipKind::Adjustment
    }
    /// The effect chain to render: the node graph when the clip has one, else the linear stack.
    pub fn has_chain(&self) -> bool {
        self.graph.as_ref().map(|g| g.nodes.len() > 2).unwrap_or(false) || !self.effects.is_empty()
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
            || self.mask.is_some()
            || self.graph.as_ref().map(|g| g.nodes.len() > 2).unwrap_or(false)
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
    /// Audio tracks: the bus every clip feeds unless the clip overrides it (0 = Main).
    #[serde(default)]
    pub bus: Id,
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
            bus: 0,
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
    /// The transition playing at timeline time t (clamped window), with its clips.
    pub fn transition_at(&self, t: f64) -> Option<(&Transition, &Clip, &Clip)> {
        self.transitions.iter().find_map(|tr| {
            let (l, r) = self.transition_pair(tr)?;
            let (a, b) = tr.window(l, r);
            (t >= a && t < b).then_some((tr, l, r))
        })
    }
    /// Drop transitions whose clips no longer abut.
    pub fn prune_transitions(&mut self) {
        let keep: Vec<Id> =
            self.transitions.iter().filter(|t| self.transition_pair(t).is_some()).map(|t| t.id).collect();
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
    /// Colour labels (name + colour), editable by the user.
    pub labels: Vec<Label>,
    /// Timeline markers (sorted by time).
    pub markers: Vec<Marker>,
    /// Mixer buses; `buses[0]` is Main and always exists.
    pub buses: Vec<Bus>,
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
            labels: default_labels(),
            markers: Vec::new(),
            buses: Vec::new(),
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
    /// Removes an asset and every clip using it — in the live timeline, the stashed main timeline and
    /// every nested sequence (a leftover clip would render black / silent).
    pub fn remove_asset(&mut self, id: Id) {
        self.assets.retain(|a| a.id != id);
        let drop_clips = |tracks: &mut Vec<Track>| {
            for t in tracks.iter_mut() {
                t.clips.retain(|c| !(c.uses_asset() && c.asset == id));
                t.prune_transitions();
            }
        };
        drop_clips(&mut self.tracks);
        if let Some(st) = &mut self.main_stash {
            drop_clips(&mut st.tracks);
        }
        for seq in &mut self.sequences {
            drop_clips(&mut seq.tracks);
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
            if !c.contains(t) {
                continue; // freezing a clip the playhead is not over would store an out-of-range source time
            }
            let src = c.src_time(t);
            let target =
                if t > c.start + MIN_CLIP { self.split_at(t, Some(&[id])).first().copied().unwrap_or(id) } else { id };
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
                let ti =
                    seq.tracks.iter().enumerate().filter(|(_, t)| t.kind == kind).nth(pos).map(|(i, _)| i).unwrap_or(0);
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

    // ---------- labels ----------
    /// Colour of label index `idx` (1-based; 0 or unknown = None).
    pub fn label_color(&self, idx: u8) -> Option<[u8; 3]> {
        (idx > 0).then(|| self.labels.get(idx as usize - 1).map(|l| l.color)).flatten()
    }
    pub fn label_name(&self, idx: u8) -> &str {
        if idx == 0 {
            return "None";
        }
        self.labels.get(idx as usize - 1).map(|l| l.name.as_str()).unwrap_or("None")
    }
    /// Add a label; returns its 1-based index.
    pub fn add_label(&mut self, name: impl Into<String>, color: [u8; 3]) -> u8 {
        self.labels.push(Label { name: name.into(), color });
        self.labels.len() as u8
    }
    /// Remove a label; clips/assets/markers using it fall back to "none", higher indices shift down.
    pub fn remove_label(&mut self, idx: u8) {
        if idx == 0 || idx as usize > self.labels.len() {
            return;
        }
        self.labels.remove(idx as usize - 1);
        let fix = |l: &mut u8| {
            if *l == idx {
                *l = 0;
            } else if *l > idx {
                *l -= 1;
            }
        };
        for a in &mut self.assets {
            fix(&mut a.label);
        }
        for m in &mut self.markers {
            fix(&mut m.label);
        }
        let mut all: Vec<&mut Track> = self.tracks.iter_mut().collect();
        if let Some(st) = &mut self.main_stash {
            all.extend(st.tracks.iter_mut());
        }
        for sq in &mut self.sequences {
            all.extend(sq.tracks.iter_mut());
        }
        for t in all {
            for c in &mut t.clips {
                fix(&mut c.label);
                for m in &mut c.markers {
                    fix(&mut m.label);
                }
            }
        }
    }

    // ---------- markers ----------
    pub fn add_marker(&mut self, t: f64, name: impl Into<String>) -> Id {
        let id = self.new_id();
        self.markers.push(Marker { id, t: t.max(0.0), name: name.into(), ..Default::default() });
        self.sort_markers();
        id
    }
    pub fn remove_marker(&mut self, id: Id) {
        self.markers.retain(|m| m.id != id);
        for t in &mut self.tracks {
            for c in &mut t.clips {
                c.markers.retain(|m| m.id != id);
            }
        }
    }
    pub fn marker_mut(&mut self, id: Id) -> Option<&mut Marker> {
        if let Some(i) = self.markers.iter().position(|m| m.id == id) {
            return self.markers.get_mut(i);
        }
        self.tracks.iter_mut().flat_map(|t| t.clips.iter_mut()).flat_map(|c| c.markers.iter_mut()).find(|m| m.id == id)
    }
    pub fn sort_markers(&mut self) {
        self.markers.sort_by(|a, b| a.t.total_cmp(&b.t));
    }
    /// Add a marker on a clip (time is clip-local).
    pub fn add_clip_marker(&mut self, clip: Id, local_t: f64, name: impl Into<String>) -> Option<Id> {
        let id = self.new_id();
        let name = name.into();
        let c = self.clip_mut(clip)?;
        c.markers.push(Marker { id, t: local_t.clamp(0.0, c.duration), name, ..Default::default() });
        c.markers.sort_by(|a, b| a.t.total_cmp(&b.t));
        Some(id)
    }
    /// Every marker in timeline time: project markers plus clip markers offset by their clip.
    pub fn markers_in_timeline(&self) -> Vec<(Id, f64, f64, String, u8)> {
        let mut v: Vec<(Id, f64, f64, String, u8)> =
            self.markers.iter().map(|m| (m.id, m.t, m.duration, m.name.clone(), m.label)).collect();
        for (_, c) in self.all_clips() {
            for m in &c.markers {
                v.push((m.id, c.start + m.t, m.duration, m.name.clone(), m.label));
            }
        }
        v.sort_by(|a, b| a.1.total_cmp(&b.1));
        v
    }

    // ---------- buses ----------
    /// The Main bus id, creating the default bus set on first use.
    pub fn main_bus(&mut self) -> Id {
        if self.buses.is_empty() {
            let id = self.new_id();
            self.buses.push(Bus { id, name: "Main".into(), output: 0, ..Default::default() });
        }
        self.buses[0].id
    }
    pub fn bus(&self, id: Id) -> Option<&Bus> {
        self.buses.iter().find(|b| b.id == id)
    }
    pub fn bus_mut(&mut self, id: Id) -> Option<&mut Bus> {
        self.buses.iter_mut().find(|b| b.id == id)
    }
    pub fn add_bus(&mut self, name: impl Into<String>) -> Id {
        let main = self.main_bus();
        let id = self.new_id();
        self.buses.push(Bus { id, name: name.into(), output: main, ..Default::default() });
        id
    }
    /// Remove a bus (never Main); tracks/clips and sends fall back to Main.
    pub fn remove_bus(&mut self, id: Id) {
        let main = self.main_bus();
        if id == main {
            return;
        }
        self.buses.retain(|b| b.id != id);
        for b in &mut self.buses {
            if b.output == id {
                b.output = main;
            }
        }
        for t in &mut self.tracks {
            if t.bus == id {
                t.bus = 0;
            }
            for c in &mut t.clips {
                if c.bus == id {
                    c.bus = 0;
                }
            }
        }
    }
    /// Which bus a clip feeds: its own override, else its track's, else Main.
    pub fn bus_of(&self, track: usize, clip: &Clip) -> Id {
        let main = self.buses.first().map(|b| b.id).unwrap_or(0);
        if clip.bus != 0 && self.bus(clip.bus).is_some() {
            return clip.bus;
        }
        let t = self.tracks.get(track).map(|t| t.bus).unwrap_or(0);
        if t != 0 && self.bus(t).is_some() {
            t
        } else {
            main
        }
    }

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

    // ---------- node graphs ----------
    /// Give the clip a node graph built from its current effect stack (idempotent).
    pub fn ensure_graph(&mut self, clip: Id) -> bool {
        let Some(c) = self.clip(clip) else { return false };
        if c.graph.is_some() {
            return true;
        }
        let effects = c.effects.clone();
        let mut next = || {
            self.next_id += 1;
            self.next_id
        };
        let g = NodeGraph::from_effects(&effects, &mut next);
        if let Some(c) = self.clip_mut(clip) {
            c.graph = Some(g);
        }
        true
    }
    /// Add a node to a clip's graph at editor position (x, y); returns its id.
    pub fn add_node(&mut self, clip: Id, kind: NodeKind, x: f32, y: f32) -> Option<Id> {
        self.ensure_graph(clip);
        let id = self.new_id();
        let c = self.clip_mut(clip)?;
        let g = c.graph.as_mut()?;
        g.nodes.push(Node { id, kind, x, y, enabled: true });
        Some(id)
    }

    // ---------- copy / paste attributes ----------
    /// Snapshot of a clip to paste attributes from (Ctrl+Alt+C).
    pub fn copy_attributes(&self, clip: Id) -> Option<Clip> {
        self.clip(clip).cloned()
    }
    /// Apply the selected attributes of `src` onto `ids` (timing and media are never touched).
    /// Returns how many clips changed.
    pub fn paste_attributes(&mut self, src: &Clip, ids: &[Id], set: AttrSet) -> usize {
        let mut n = 0;
        for &id in ids {
            let Some(c) = self.clip_mut(id) else { continue };
            if c.id == src.id {
                continue;
            }
            if set.transform {
                c.x = src.x.clone();
                c.y = src.y.clone();
                c.scale = src.scale.clone();
                c.rotation = src.rotation.clone();
            }
            if set.opacity {
                c.opacity = src.opacity.clone();
            }
            if set.blend {
                c.blend = src.blend;
            }
            if set.effects {
                c.effects = src.effects.clone();
            }
            if set.graph {
                c.graph = src.graph.clone();
            }
            if set.mask {
                c.mask = src.mask.clone();
            }
            if set.speed {
                c.set_speed(src.speed);
                c.reverse = src.reverse;
                c.freeze = src.freeze;
            }
            if set.audio {
                c.volume = src.volume.clone();
                c.pan = src.pan.clone();
                c.fade_in = src.fade_in;
                c.fade_out = src.fade_out;
                c.bus = src.bus;
            }
            if set.text {
                if let Some(t) = &src.text {
                    c.text = Some(t.clone());
                }
            }
            if set.shape {
                if let Some(sh) = &src.shape {
                    c.shape = Some(sh.clone());
                }
            }
            if set.label {
                c.label = src.label;
            }
            if set.markers {
                c.markers = src.markers.clone();
            }
            n += 1;
        }
        if n > 0 {
            self.tidy();
        }
        n
    }

    // ---------- track helpers ----------
    /// Insert a new video track directly above `above_kind_index` (index within the video tracks) and
    /// return its index in `tracks`. Used when a clip is dragged past the top of the timeline.
    pub fn insert_video_track_above(&mut self, video_pos: usize) -> usize {
        let idx = self.add_track(TrackKind::Video); // appended after the existing video tracks
        let list = self.video_tracks();
        let target = video_pos.min(list.len().saturating_sub(1));
        // `add_track` already puts it last among video tracks (i.e. the topmost row)
        let _ = target;
        self.rename_tracks();
        idx
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
            .chain(p.markers.iter().map(|m| m.id))
            .chain(p.buses.iter().map(|b| b.id))
            .chain(p.all_clips().flat_map(|(_, c)| c.markers.iter().map(|m| m.id)))
            .chain(p.all_clips().filter_map(|(_, c)| c.graph.as_ref()).flat_map(|g| g.nodes.iter().map(|n| n.id)))
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
        if p.labels.is_empty() {
            p.labels = default_labels();
        }
        p.markers.retain(|m| m.t.is_finite() && m.t >= 0.0);
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
        assert!((tr.progress(l, r, 3.5) - 0.0).abs() < 1e-9);
        assert!((tr.progress(l, r, 4.5) - 1.0).abs() < 1e-9);
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

/// Which parts of a clip `Project::paste_attributes` copies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttrSet {
    pub transform: bool,
    pub opacity: bool,
    pub blend: bool,
    pub effects: bool,
    pub graph: bool,
    pub mask: bool,
    pub speed: bool,
    pub audio: bool,
    pub text: bool,
    pub shape: bool,
    pub label: bool,
    pub markers: bool,
}

impl Default for AttrSet {
    fn default() -> Self {
        Self {
            transform: true,
            opacity: true,
            blend: true,
            effects: true,
            graph: true,
            mask: true,
            speed: false,
            audio: true,
            text: false,
            shape: false,
            label: true,
            markers: false,
        }
    }
}

impl AttrSet {
    pub const NONE: AttrSet = AttrSet {
        transform: false,
        opacity: false,
        blend: false,
        effects: false,
        graph: false,
        mask: false,
        speed: false,
        audio: false,
        text: false,
        shape: false,
        label: false,
        markers: false,
    };
    /// (label, field) pairs for the paste dialog.
    pub fn fields(&mut self) -> Vec<(&'static str, &mut bool)> {
        vec![
            ("Transform (position, scale, rotation)", &mut self.transform),
            ("Opacity", &mut self.opacity),
            ("Blend mode", &mut self.blend),
            ("Effects", &mut self.effects),
            ("Node graph", &mut self.graph),
            ("Mask", &mut self.mask),
            ("Speed / reverse / freeze", &mut self.speed),
            ("Audio (volume, pan, fades, bus)", &mut self.audio),
            ("Text style", &mut self.text),
            ("Shape style", &mut self.shape),
            ("Colour label", &mut self.label),
            ("Markers", &mut self.markers),
        ]
    }
    pub fn any(&self) -> bool {
        let mut me = *self;
        me.fields().iter().any(|(_, v)| **v)
    }
}
