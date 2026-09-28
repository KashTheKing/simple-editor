use crate::model::*;
use serde::{Deserialize, Serialize};

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
    // --- round 4 ---
    ColorReplace,
    // ---- ws:color-engine ----
    /// Lift/Gamma/Gain colour wheels + temperature/tint (the professional grading primitive).
    Primaries,
    /// HSL-band key -> alpha, for scopes/qualifier-driven secondary grades (no RGB change, matte only).
    Qualifier,
    /// A `.cube` 3D LUT (`Effect.lut` = file path), mixed in by its Intensity param.
    Lut,
    /// GPU-only: blends the current frame with a shutter-window neighbour (`Effect.params[0]` = Amount).
    /// Distinct from `MotionBlur` (which averages several intra-frame samples): this blends *between*
    /// two decoded frames, so it needs exactly the same prev/next samples `needs_motion()` already wires.
    FrameBlend,
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

pub(crate) const fn ps(name: &'static str, default: f64, min: f64, max: f64) -> ParamSpec {
    ParamSpec { name, default, min, max }
}

impl EffectKind {
    pub const ALL: [EffectKind; 29] = [
        EffectKind::Blur,
        EffectKind::MotionBlur,
        EffectKind::Pixelate,
        EffectKind::JpegCompress,
        EffectKind::Vhs,
        EffectKind::ChromaKey,
        EffectKind::ColorReplace,
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
        EffectKind::Primaries,
        EffectKind::Qualifier,
        EffectKind::Lut,
        EffectKind::FrameBlend,
        EffectKind::Shader,
    ];
    /// Catalogue grouping for the effects panel.
    pub fn category(self) -> &'static str {
        use EffectKind::*;
        match self {
            Color | Curves | Levels | HueShift | Tint | Grayscale | Invert | Threshold => "Adjustments",
            Blur | MotionBlur | FrameBlend | Sharpen | Pixelate | JpegCompress | Vhs | EdgeGlow | Vignette | RecDot => {
                "Stylize"
            }
            ChromaKey | ColorReplace | Crop | BlobTrack => "Keying & Matte",
            Flip | Plane3d | Wobble => "Transform",
            Primaries | Qualifier | Lut => "Color",
            Shader => "Custom",
        }
    }
    /// Effects that make sense on an audio clip, for the catalogue's audio-aware filtering - an audio
    /// clip has no pixels, so every current kind (all pixel/GLSL effects) is false here. Exhaustive on
    /// purpose (see `engine::effects::apply`'s tail) so a future audio kind is a compile-time reminder
    /// to flip it on.
    // ponytail: no audio EffectKind exists yet; add one and flip its arm here when it does.
    pub fn applies_to_audio(self) -> bool {
        use EffectKind::*;
        match self {
            Blur | Pixelate | Tint | Color | Vignette | Sharpen | Invert | Grayscale | Flip | Crop | Wobble
            | ChromaKey | Curves | Levels | HueShift | JpegCompress | MotionBlur | Plane3d | EdgeGlow | Threshold
            | BlobTrack | Vhs | RecDot | ColorReplace | Primaries | Qualifier | Lut | FrameBlend | Shader => false,
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
            EffectKind::ColorReplace => "Color Replace",
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
            EffectKind::Primaries => "Primaries",
            EffectKind::Qualifier => "Qualifier",
            EffectKind::Lut => "LUT",
            EffectKind::FrameBlend => "Frame Blend",
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
            EffectKind::ColorReplace => P_COLOR_REPLACE,
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
            EffectKind::Primaries => P_PRIMARIES,
            EffectKind::Qualifier => P_QUALIFIER,
            EffectKind::Lut => P_LUT,
            EffectKind::FrameBlend => P_FRAME_BLEND,
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
                | (EffectKind::Threshold, 2)
                | (EffectKind::JpegCompress, 2)
                | (EffectKind::MotionBlur, 2)
                | (EffectKind::EdgeGlow, 6)
        )
    }
    /// Effects whose output depends on neighbouring frames (the renderer must supply them).
    pub fn needs_motion(self) -> bool {
        matches!(self, EffectKind::MotionBlur | EffectKind::FrameBlend)
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
/// Swap one colour for another: everything within `Tolerance` of the source colour is blended to the
/// target, `Softness` widening the falloff past the tolerance.
const P_COLOR_REPLACE: &[ParamSpec] = &[
    ps("From R", 255.0, 0.0, 255.0),
    ps("From G", 0.0, 0.0, 255.0),
    ps("From B", 0.0, 0.0, 255.0),
    ps("To R", 0.0, 0.0, 255.0),
    ps("To G", 128.0, 0.0, 255.0),
    ps("To B", 255.0, 0.0, 255.0),
    ps("Tolerance", 0.2, 0.0, 1.0),
    ps("Softness", 0.1, 0.0, 1.0),
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
/// Lift/Gamma/Gain per channel + temperature/tint. 11 of the 12 `PARAM_NAMES` (gpu.rs) slots - a
/// separate per-channel Offset (redundant with Lift) is deliberately dropped to fit.
const P_PRIMARIES: &[ParamSpec] = &[
    ps("Lift R", 0.0, -1.0, 1.0),
    ps("Lift G", 0.0, -1.0, 1.0),
    ps("Lift B", 0.0, -1.0, 1.0),
    ps("Gamma R", 1.0, 0.1, 5.0),
    ps("Gamma G", 1.0, 0.1, 5.0),
    ps("Gamma B", 1.0, 0.1, 5.0),
    ps("Gain R", 1.0, 0.0, 3.0),
    ps("Gain G", 1.0, 0.0, 3.0),
    ps("Gain B", 1.0, 0.0, 3.0),
    ps("Temp", 0.0, -100.0, 100.0),
    ps("Tint", 0.0, -100.0, 100.0),
];
/// HSL-band secondary key: hue centre/width (`Hue Width` is the half-width in degrees, so its max of
/// 180 covers the whole hue circle), saturation and luminance bands, edge softness. Defaults are wide
/// open (Hue Width at its max, Sat/Lum spanning the full 0..1) so a freshly-added Qualifier matches
/// every pixel (identity, alpha unchanged) instead of keying the frame out until narrowed via
/// `color.qualifier` or the (later, inspector-gallery) UI - same non-destructive-by-default spirit as
/// every other effect here.
const P_QUALIFIER: &[ParamSpec] = &[
    ps("Hue", 120.0, 0.0, 360.0),
    ps("Hue Width", 180.0, 0.0, 180.0),
    ps("Sat Min", 0.0, 0.0, 1.0),
    ps("Sat Max", 1.0, 0.0, 1.0),
    ps("Lum Min", 0.0, 0.0, 1.0),
    ps("Lum Max", 1.0, 0.0, 1.0),
    ps("Softness", 0.15, 0.0, 1.0),
];
const P_LUT: &[ParamSpec] = &[ps("Intensity", 1.0, 0.0, 1.0)];
const P_FRAME_BLEND: &[ParamSpec] = &[ps("Amount", 0.5, 0.0, 1.0)];
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
    ps("Frequency", 2.0, 0.05, 30.0),
    ps("Seed", 1.0, 0.0, 1000.0),
    // 0 = Sine (one clean wave), 1 = Layered (three sines, the old look), 2 = Cubic (smoothed random
    // steps), 3 = Triangle, 4 = Random (stepped hold). See `WOBBLE_MOTIONS` / `engine::effects::wobble`.
    ps("Motion", 1.0, 0.0, 4.0),
    // 0 = every wiggle as-is, 1 = heavily smoothed (a slow drift). Divides the effective frequency.
    ps("Smoothness", 0.0, 0.0, 1.0),
];

/// Names for the Camera Shake "Motion" knob, in value order (the inspector shows these).
pub const WOBBLE_MOTIONS: [&str; 5] = ["Sine", "Layered", "Cubic", "Triangle", "Random"];

/// Starting point for `EffectKind::Shader`: `tex` = the layer, `uv` = 0..1, `u_time` = clip-local
/// seconds, `u1..u8` = the eight knobs, `u_res` = layer size in px. Output goes to `out_color`
/// (straight alpha, same convention as every other effect).
pub const DEFAULT_SHADER: &str = r#"// custom effect - edit freely
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
    #[serde(default = "crate::model::tru")]
    pub enabled: bool,
    #[serde(default)]
    pub params: Vec<Animated>,
    /// Limits the effect to (or outside) a shape.
    #[serde(default)]
    pub mask: Option<Mask>,
    /// `EffectKind::Shader` only: the GLSL fragment shader source.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub shader: String,
    /// `EffectKind::Lut` only: the `.cube` file path (set via `clip.add_lut`, never a compiled-in default).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub lut: String,
    /// Clip-local second the effect switches on (CapCut-style window inside the clip).
    #[serde(default)]
    pub start: f64,
    /// How long it stays on; `<= 0` means "to the end of the clip", which is what every effect written
    /// before this field existed deserialises to.
    #[serde(default)]
    pub len: f64,
}

impl Effect {
    pub fn new(kind: EffectKind) -> Self {
        Self {
            kind,
            enabled: true,
            params: kind.params().iter().map(|p| Animated::new(p.default)).collect(),
            mask: None,
            shader: if kind == EffectKind::Shader { DEFAULT_SHADER.to_string() } else { String::new() },
            lut: String::new(),
            start: 0.0,
            len: 0.0,
        }
    }
    /// Is the effect on at clip-local time `t`? Enabled + inside its window (the default window is the
    /// whole clip, so this is just `enabled` for every project that never touched the timing).
    pub fn on_at(&self, t: f64) -> bool {
        self.enabled && t >= self.start - EPS && (self.len <= 0.0 || t <= self.start + self.len + EPS)
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
