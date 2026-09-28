//! Project data model - the shared contract between UI, engine, playback and export.
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

/// Preview/export canvas background: what unfilled area of the frame clears to
/// (`engine::gpu::GpuRenderer::render_canvas` is the one place that reads this). `Black` matches the
/// hardcoded behaviour every project had before this setting existed.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize, Default)]
pub enum BackgroundMode {
    /// Preview-only visual aid: bakes as literal grey squares on export, not real transparency.
    Checkerboard,
    #[default]
    Black,
    White,
    Custom([u8; 4]),
}

impl BackgroundMode {
    pub const ALL: [BackgroundMode; 3] = [BackgroundMode::Checkerboard, BackgroundMode::Black, BackgroundMode::White];
    pub fn name(self) -> &'static str {
        match self {
            BackgroundMode::Checkerboard => "Checkerboard",
            BackgroundMode::Black => "Black",
            BackgroundMode::White => "White",
            BackgroundMode::Custom(_) => "Custom",
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

/// y for the x = `f` on the cubic bezier (0,0) (x1,y1) (x2,y2) (1,1) - Newton iterations on the x polynomial.
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

/// A live driver for an `Animated` value: follow a saved path's X or Y over the clip's duration, or a
/// Luau expression of `t` (clip-local seconds) and `value` (the property's own keyframed/constant
/// value), ending with `return <number>`.
/// `Project::refresh_links` bakes active links into the transient `Animated::baked` samples that
/// `at()` then reads, so every render/mixer read site stays project-free.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub enum AnimLink {
    #[default]
    None,
    PathX(Id),
    PathY(Id),
    Expr(String),
}

impl AnimLink {
    pub fn is_none(&self) -> bool {
        *self == AnimLink::None
    }
}

/// A scalar property that is either constant (`value`) or keyframed (`keys`, sorted by t).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Animated {
    pub value: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keys: Vec<Keyframe>,
    /// Live driver (path / expression); `None` for a plain property.
    #[serde(default, skip_serializing_if = "AnimLink::is_none")]
    pub link: AnimLink,
    /// Transient samples of `link`; while non-empty they override `keys`/`value` in `at()`.
    #[serde(skip)]
    pub baked: Vec<Keyframe>,
    /// Change stamp for `baked` (hash of the link and its inputs); 0 = never baked.
    #[serde(skip)]
    pub baked_rev: u64,
    /// Last expression/path error, shown in the inspector. Transient.
    #[serde(skip)]
    pub link_err: Option<String>,
}

impl Animated {
    pub fn new(value: f64) -> Self {
        Self { value, keys: Vec::new(), link: AnimLink::None, baked: Vec::new(), baked_rev: 0, link_err: None }
    }
    pub fn is_animated(&self) -> bool {
        !self.keys.is_empty()
    }
    pub fn is_default(&self, def: f64) -> bool {
        !self.is_animated() && (self.value - def).abs() < EPS
    }
    /// Value at clip-local time t (eased interpolation, clamped at the ends). A baked live link
    /// (path / expression) overrides the property's own keys.
    pub fn at(&self, t: f64) -> f64 {
        if !self.baked.is_empty() {
            return Self::eval_keys(&self.baked, t);
        }
        self.base_at(t)
    }
    /// The keyframed/constant value ignoring any live link - what an expression sees as `v`.
    pub fn base_at(&self, t: f64) -> f64 {
        if self.keys.is_empty() {
            return self.value;
        }
        Self::eval_keys(&self.keys, t)
    }
    fn eval_keys(k: &[Keyframe], t: f64) -> f64 {
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
    /// Drop any live link, letting the property's own keys/constant show through again.
    pub fn unlink(&mut self) {
        self.link = AnimLink::None;
        self.baked.clear();
        self.baked_rev = 0;
        self.link_err = None;
    }
    /// Set the value at time t: upserts a keyframe when animated, otherwise sets the constant.
    /// A manual edit detaches any live link (AE-style: dragging a linked value breaks the link).
    pub fn set_at(&mut self, t: f64, v: f64) {
        if !self.link.is_none() {
            self.unlink();
        }
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

/// One entry in `Project.notes`: a markdown note with an optional title and colour label. Rendered
/// with `egui_commonmark` in the planner's Notes tab. `Project.notes` used to be a single free-form
/// string; `de_notes` migrates a non-empty old value into one untitled note.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Note {
    pub id: Id,
    pub title: String,
    /// Index into `Project.labels` + 1 (0 = none), same convention as `Marker::label`.
    pub label: u8,
    pub body: String,
}

/// `notes` was a single free-form string before it became a list of titled markdown notes; a
/// non-empty old string becomes one untitled note (its placeholder id 0 is fixed up to a real one in
/// `Project::from_json`, which is the only place with a `next_id` counter to draw from).
fn de_notes<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<Note>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StrOrNotes {
        Str(String),
        List(Vec<Note>),
    }
    Ok(match StrOrNotes::deserialize(d)? {
        StrOrNotes::Str(s) if !s.trim().is_empty() => vec![Note { body: s, ..Default::default() }],
        StrOrNotes::Str(_) => Vec::new(),
        StrOrNotes::List(v) => v,
    })
}

/// One item on the standalone Moodboard pane (`Project.moodboard`): a project asset plus free-form
/// label tags. These are plain user-typed words, not indices into `Project.labels` like everything
/// else that carries a `label` field - the moodboard's filter row is a text/chip filter, not a colour.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct MoodItem {
    pub asset: Id,
    pub labels: Vec<String>,
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
    /// Optional short checklist ("label", done) shown compactly under the item when non-empty.
    pub requirements: Vec<(String, bool)>,
    /// Seconds accumulated by the planner's Timer tab while linked to this item. The timer's own
    /// running state (mode, start time, whether it's live) is session-scoped UI state, not project
    /// data - it lives in `ui::planner::PlannerState`, not here.
    pub tracked_seconds: f64,
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
            requirements: Vec::new(),
            tracked_seconds: 0.0,
        }
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
    /// The text clip's wording (`TextStyle::text`) - separate from `text_style` so pasting a look
    /// doesn't overwrite the destination's own words.
    pub text_content: bool,
    /// Every visual text field (font/size/bold/italic/colour/outline/shadow/spacing/align/box/spans)
    /// except the wording itself.
    pub text_style: bool,
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
            text_content: false,
            text_style: false,
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
        text_content: false,
        text_style: false,
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
            ("Text content (wording)", &mut self.text_content),
            ("Text style", &mut self.text_style),
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

// ---------- shared serde `default = "..."` helpers ----------
pub(crate) fn tru() -> bool {
    true
}
pub(crate) fn one() -> f64 {
    1.0
}
pub(crate) fn a0() -> Animated {
    Animated::new(0.0)
}
pub(crate) fn a1() -> Animated {
    Animated::new(1.0)
}
pub(crate) fn dh() -> f32 {
    60.0
}

mod asset;
mod audio;
mod clip;
mod effect;
mod graph;
mod io;
mod marker;
pub mod ops;
mod path;
mod project;
mod shape;
mod subtitle;
mod text;
mod track;
mod transition;

#[cfg(test)]
mod tests;

pub use asset::*;
pub use audio::*;
pub use clip::*;
pub use effect::*;
pub use graph::*;
pub use marker::*;
pub use path::*;
pub use project::*;
pub use shape::*;
pub use subtitle::*;
pub use text::*;
pub use track::*;
pub use transition::*;
