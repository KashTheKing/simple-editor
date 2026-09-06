use crate::model::*;
use serde::{Deserialize, Serialize};

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
    /// Explicit Polygon vertices in project px relative to the shape centre (the Polygon tool places
    /// and drags these). Empty = the regular `sides`-gon, so old projects keep their shape.
    pub points: Vec<(f32, f32)>,
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
            points: Vec::new(),
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
    /// The explicit vertex list, or None when the regular `sides`-gon applies (fewer than 3 points
    /// cannot enclose anything, so a half-placed path still draws as the n-gon).
    pub fn poly_points(&self) -> Option<&[(f32, f32)]> {
        (self.kind == ShapeKind::Polygon && self.points.len() >= 3).then_some(&self.points[..])
    }
    /// The outline as a timed path, ready to drive an animation: a drawing's strokes end to end, or an
    /// explicit polygon's vertices (one per second, so a closed shape still has a direction).
    pub fn path_points(&self) -> Vec<(f32, f32, f32)> {
        if self.kind != ShapeKind::Draw {
            return self.points.iter().enumerate().map(|(i, &(x, y))| (x, y, i as f32)).collect();
        }
        let mut out: Vec<(f32, f32, f32)> = Vec::new();
        for st in &self.strokes {
            // strokes from one take already share a clock; older ones each start at 0, so anything that
            // would step back in time is pushed behind what came before
            let off = (out.last().map_or(0.0, |&(_, _, t)| t) - st.points.first().map_or(0.0, |p| p.2)).max(0.0);
            out.extend(st.points.iter().map(|&(x, y, t)| (x, y, t + off)));
        }
        out
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
        // w/h were missing entirely: the rasteriser's own cache key only adds the LAYER size (built from
        // their absolute value, engine::shapes::half_size), so two Lines of the same length pointing
        // opposite ways hashed identically and one silently got served the other's cached pixels. Hash
        // the animated curve itself, sign included, the same way `points` and `strokes` are below.
        for a in [&self.w, &self.h] {
            a.value.to_bits().hash(&mut h);
            a.keys.len().hash(&mut h);
            for k in &a.keys {
                (k.t.to_bits(), k.v.to_bits()).hash(&mut h);
                // Ease is not Hash (Bezier carries f32 handles), so hash it by hand
                match k.ease {
                    Ease::Linear => 0u8.hash(&mut h),
                    Ease::EaseIn => 1u8.hash(&mut h),
                    Ease::EaseOut => 2u8.hash(&mut h),
                    Ease::EaseInOut => 3u8.hash(&mut h),
                    Ease::Hold => 4u8.hash(&mut h),
                    Ease::Bezier { x1, y1, x2, y2 } => {
                        5u8.hash(&mut h);
                        for f in [x1, y1, x2, y2] {
                            f.to_bits().hash(&mut h);
                        }
                    }
                }
            }
        }
        // vertices move under the mouse, so their values (not just the count) key the cache
        for p in &self.points {
            (p.0.to_bits(), p.1.to_bits()).hash(&mut h);
        }
        self.strokes.len().hash(&mut h);
        for st in &self.strokes {
            st.points.len().hash(&mut h);
            st.color.hash(&mut h);
            st.width.to_bits().hash(&mut h);
        }
        h.finish()
    }
}

/// A drawing or polygon outline kept on the project so it can be reused: as a motion path for a clip's
/// X/Y, as the centre of a mask node, or just as a sketch to look at again.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct PathAsset {
    pub id: Id,
    pub name: String,
    /// (x, y, t): project px relative to the canvas centre, t in seconds from the start of the path.
    pub points: Vec<(f32, f32, f32)>,
}

/// Turn a recorded path into X/Y keyframes spanning `duration` seconds. The recording's own timing is
/// kept when it has any (the replay keeps its rhythm) and the points are spread evenly otherwise — a
/// polygon has no clock. Key times are always strictly increasing: a still mouse records several points
/// at one instant, and two keys at the same time would swallow each other (`Animated::key_index_at`).
pub fn path_to_keys(points: &[(f32, f32, f32)], duration: f64) -> (Animated, Animated) {
    let (mut x, mut y) = (Animated::new(0.0), Animated::new(0.0));
    let (Some(first), Some(last)) = (points.first(), points.last()) else { return (x, y) };
    let span = (last.2 - first.2) as f64;
    let dur = duration.max(MIN_CLIP);
    for (i, p) in points.iter().enumerate() {
        let f = match (span > 0.0, points.len() > 1) {
            (true, _) => (p.2 - first.2) as f64 / span,
            (false, true) => i as f64 / (points.len() - 1) as f64,
            (false, false) => 0.0,
        };
        let t = (f * dur).clamp(0.0, dur);
        let t = match x.keys.last() {
            Some(k) if t <= k.t + KEY_EPS => k.t + KEY_EPS,
            _ => t,
        };
        x.keys.push(Keyframe { t, v: p.0 as f64, ease: Ease::Linear });
        y.keys.push(Keyframe { t, v: p.1 as f64, ease: Ease::Linear });
    }
    // duplicate timestamps pushed the tail past the end: squeeze the whole path back into `dur` so it
    // still finishes exactly where it was drawn
    if let Some(k) = x.keys.last().filter(|k| k.t > dur) {
        let s = dur / k.t;
        for (a, b) in x.keys.iter_mut().zip(y.keys.iter_mut()) {
            a.t *= s;
            b.t *= s;
        }
    }
    (x, y)
}
