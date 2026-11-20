//! ---- ws:parity-formats ----
//! Lottie (Bodymovin JSON) animations as clips: a small native renderer, no runtime dependency
//! (`goals.md`: no new crates; lottie-web would need page scripts, which the HTML renderer keeps off).
//!
//! `Comp::parse` reads the JSON into a typed tree once; `Comp::render` draws one frame into straight
//! RGBA at any size; `engine::synth::encode` turns the frames into a clip (H.264, colour over alpha
//! when the animation is transparent), imported like any other media.
//!
//! Supported - what After Effects' Bodymovin export and LottieFiles animations mostly use:
//! * layers: shape (4), solid (1), null (3, for parenting), precomp (0, nested `assets`), with
//!   in / out points, start time and stretch, parenting, hidden layers skipped
//! * transforms: anchor, position (also split X / Y, and spatial bezier paths), scale, rotation, opacity
//! * shapes: group, path, rectangle (rounded), ellipse, star / polygon, fill (non-zero / even-odd),
//!   stroke (round joins, butt or round caps), linear / radial gradient fill and stroke, trim paths
//! * keyframes: linear, bezier eased, hold; the old `s` / `e` keyframe format too
//!
//! Left out (drawn without them, never a failure): masks and track mattes, image / text layers,
//! expressions (their static values are used), merge paths, repeaters, effects, time remapping,
//! layer blend modes, and 3D layers (drawn flat). `Comp::unsupported` names what a file uses of these
//! so the caller can say so.

use serde_json::Value;
use std::collections::{BTreeSet, HashMap};

// ---------------------------------------------------------------------------------------------------
// values

#[derive(Clone, Debug)]
struct Key {
    t: f64,
    s: Option<Vec<f64>>,
    e: Option<Vec<f64>>,
    hold: bool,
    /// Easing out of this key / into the next: cubic-bezier(o.x, o.y, i.x, i.y).
    o: (f64, f64),
    i: (f64, f64),
    /// Spatial tangents (position paths): out of this key, into the next.
    to: Option<Vec<f64>>,
    ti: Option<Vec<f64>>,
}

/// A property: a constant or keyframed vector of numbers (a path is flattened into one: see `bez_vec`).
#[derive(Clone, Debug)]
enum Prop {
    Static(Vec<f64>),
    Anim(Vec<Key>),
}

fn nums(v: &Value) -> Option<Vec<f64>> {
    match v {
        Value::Number(n) => n.as_f64().map(|x| vec![x]),
        Value::Array(a) => {
            // a shape keyframe value is `[{v,i,o,c}]`
            if let Some(obj) = a.first().filter(|x| x.is_object()) {
                return bez_vec(obj).map(|(v, _)| v);
            }
            a.iter().map(Value::as_f64).collect()
        }
        Value::Object(_) => bez_vec(v).map(|(v, _)| v),
        _ => None,
    }
}

fn first_f(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::Array(a) => a.first().and_then(Value::as_f64),
        _ => None,
    }
}

impl Prop {
    fn constant(x: &[f64]) -> Prop {
        Prop::Static(x.to_vec())
    }

    fn parse(v: Option<&Value>, default: &[f64]) -> Prop {
        let Some(v) = v else { return Prop::constant(default) };
        let k = v.get("k").unwrap_or(v);
        let keyed = v.get("a").and_then(Value::as_i64) == Some(1)
            || k.as_array().is_some_and(|a| a.first().is_some_and(|x| x.get("t").is_some()));
        if keyed {
            let keys: Vec<Key> = k
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|kf| {
                    let t = kf.get("t")?.as_f64()?;
                    let ease = |name: &str, d: (f64, f64)| {
                        kf.get(name).map_or(d, |e| {
                            (e.get("x").and_then(first_f).unwrap_or(d.0), e.get("y").and_then(first_f).unwrap_or(d.1))
                        })
                    };
                    Some(Key {
                        t,
                        s: kf.get("s").and_then(nums),
                        e: kf.get("e").and_then(nums),
                        hold: kf.get("h").and_then(Value::as_i64) == Some(1),
                        o: ease("o", (0.0, 0.0)),
                        i: ease("i", (1.0, 1.0)),
                        to: kf.get("to").and_then(nums),
                        ti: kf.get("ti").and_then(nums),
                    })
                })
                .collect();
            if keys.is_empty() {
                return Prop::constant(default);
            }
            return Prop::Anim(keys);
        }
        nums(k).map(Prop::Static).unwrap_or_else(|| Prop::constant(default))
    }

    fn animated(&self) -> bool {
        matches!(self, Prop::Anim(k) if k.len() > 1)
    }

    fn at(&self, t: f64) -> Vec<f64> {
        match self {
            Prop::Static(v) => v.clone(),
            Prop::Anim(keys) => {
                let start = |i: usize| -> Option<&Vec<f64>> {
                    keys[i].s.as_ref().or_else(|| if i > 0 { keys[i - 1].e.as_ref() } else { None })
                };
                let first = start(0).cloned().unwrap_or_default();
                if t <= keys[0].t || keys.len() == 1 {
                    return first;
                }
                let n = keys.iter().rposition(|k| k.t <= t).unwrap_or(0);
                let k = &keys[n];
                let a = start(n).cloned().unwrap_or_else(|| first.clone());
                if n + 1 >= keys.len() {
                    return a;
                }
                let next = &keys[n + 1];
                if k.hold {
                    return a;
                }
                let b = k.e.clone().or_else(|| start(n + 1).cloned()).unwrap_or_else(|| a.clone());
                let span = next.t - k.t;
                if span <= 0.0 {
                    return b;
                }
                let u = ease(k.o, k.i, ((t - k.t) / span).clamp(0.0, 1.0));
                // a position along a curved motion path
                if let (Some(to), Some(ti)) = (&k.to, &k.ti) {
                    if a.len() >= 2 && b.len() >= 2 && to.len() >= 2 && ti.len() >= 2 && (to.iter().chain(ti).any(|x| x.abs() > 1e-9)) {
                        return (0..a.len().min(b.len()))
                            .map(|d| {
                                let (p0, p3) = (a[d], b[d]);
                                let (p1, p2) = (p0 + to.get(d).copied().unwrap_or(0.0), p3 + ti.get(d).copied().unwrap_or(0.0));
                                cubic(p0, p1, p2, p3, u)
                            })
                            .collect();
                    }
                }
                a.iter().zip(b.iter()).map(|(x, y)| x + (y - x) * u).collect()
            }
        }
    }

    fn f(&self, t: f64) -> f64 {
        self.at(t).first().copied().unwrap_or(0.0)
    }

    fn v2(&self, t: f64) -> [f64; 2] {
        let v = self.at(t);
        [v.first().copied().unwrap_or(0.0), v.get(1).copied().unwrap_or(0.0)]
    }
}

fn cubic(p0: f64, p1: f64, p2: f64, p3: f64, u: f64) -> f64 {
    let m = 1.0 - u;
    m * m * m * p0 + 3.0 * m * m * u * p1 + 3.0 * m * u * u * p2 + u * u * u * p3
}

/// CSS-style cubic-bezier easing: the curve's y where its x is `x` (Newton, then bisection).
fn ease(o: (f64, f64), i: (f64, f64), x: f64) -> f64 {
    let (x1, y1, x2, y2) = (o.0.clamp(0.0, 1.0), o.1, i.0.clamp(0.0, 1.0), i.1);
    if (x1 - y1).abs() < 1e-9 && (x2 - y2).abs() < 1e-9 {
        return x; // linear
    }
    let bx = |u: f64| cubic(0.0, x1, x2, 1.0, u);
    let mut u = x;
    for _ in 0..8 {
        let d = 3.0 * (1.0 - u) * (1.0 - u) * x1 + 6.0 * (1.0 - u) * u * (x2 - x1) + 3.0 * u * u * (1.0 - x2);
        if d.abs() < 1e-6 {
            break;
        }
        u = (u - (bx(u) - x) / d).clamp(0.0, 1.0);
    }
    if (bx(u) - x).abs() > 1e-4 {
        let (mut lo, mut hi) = (0.0, 1.0);
        for _ in 0..40 {
            u = (lo + hi) / 2.0;
            if bx(u) < x {
                lo = u;
            } else {
                hi = u;
            }
        }
    }
    cubic(0.0, y1, y2, 1.0, u)
}

/// A bezier path `{v, i, o, c}` as one flat vector `[vx, vy, ix, iy, ox, oy]*n` (so keyframed paths
/// interpolate like any vector) and its closed flag.
fn bez_vec(v: &Value) -> Option<(Vec<f64>, bool)> {
    let pts = |k: &str| -> Option<Vec<[f64; 2]>> {
        v.get(k)?.as_array()?.iter().map(|p| Some([p.get(0)?.as_f64()?, p.get(1)?.as_f64()?])).collect()
    };
    let vs = pts("v")?;
    let is = pts("i").unwrap_or_else(|| vec![[0.0; 2]; vs.len()]);
    let os = pts("o").unwrap_or_else(|| vec![[0.0; 2]; vs.len()]);
    let mut out = Vec::with_capacity(vs.len() * 6);
    for n in 0..vs.len() {
        let (i, o) = (is.get(n).copied().unwrap_or([0.0; 2]), os.get(n).copied().unwrap_or([0.0; 2]));
        out.extend([vs[n][0], vs[n][1], i[0], i[1], o[0], o[1]]);
    }
    Some((out, v.get("c").and_then(Value::as_bool).unwrap_or(false)))
}

// ---------------------------------------------------------------------------------------------------
// geometry

/// A 2D affine matrix: x' = a x + c y + e, y' = b x + d y + f.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Mat([f64; 6]);

impl Mat {
    fn mul(self, o: Mat) -> Mat {
        let [a, b, c, d, e, f] = self.0;
        let [a2, b2, c2, d2, e2, f2] = o.0;
        Mat([a * a2 + c * b2, b * a2 + d * b2, a * c2 + c * d2, b * c2 + d * d2, a * e2 + c * f2 + e, b * e2 + d * f2 + f])
    }
    fn apply(&self, p: [f64; 2]) -> [f64; 2] {
        let [a, b, c, d, e, f] = self.0;
        [a * p[0] + c * p[1] + e, b * p[0] + d * p[1] + f]
    }
    fn scale_factor(&self) -> f64 {
        let [a, b, c, d, ..] = self.0;
        (a * d - b * c).abs().sqrt()
    }
    fn translate(x: f64, y: f64) -> Mat {
        Mat([1.0, 0.0, 0.0, 1.0, x, y])
    }
    fn scale(x: f64, y: f64) -> Mat {
        Mat([x, 0.0, 0.0, y, 0.0, 0.0])
    }
    fn rotate(deg: f64) -> Mat {
        let (s, c) = deg.to_radians().sin_cos();
        Mat([c, s, -s, c, 0.0, 0.0])
    }
}

/// A flattened outline in pixels.
#[derive(Clone, Debug)]
struct Poly {
    pts: Vec<[f32; 2]>,
    closed: bool,
}

/// Flatten one cubic segment into `out` (the end point included, the start not).
fn flatten_cubic(p0: [f64; 2], p1: [f64; 2], p2: [f64; 2], p3: [f64; 2], out: &mut Vec<[f32; 2]>) {
    let d = |a: [f64; 2], b: [f64; 2]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
    let len = d(p0, p1) + d(p1, p2) + d(p2, p3);
    let straight = d(p0, p1) + d(p2, p3) < 1e-6;
    let n = if straight { 1 } else { ((len / 3.0).ceil() as usize).clamp(2, 64) };
    for k in 1..=n {
        let u = k as f64 / n as f64;
        out.push([cubic(p0[0], p1[0], p2[0], p3[0], u) as f32, cubic(p0[1], p1[1], p2[1], p3[1], u) as f32]);
    }
}

/// A bezier path (`bez_vec` layout) through matrix `m`, flattened.
fn flatten_bez(v: &[f64], closed: bool, m: &Mat) -> Option<Poly> {
    let n = v.len() / 6;
    if n == 0 {
        return None;
    }
    let pt = |k: usize| [v[k * 6], v[k * 6 + 1]];
    let tin = |k: usize| [v[k * 6] + v[k * 6 + 2], v[k * 6 + 1] + v[k * 6 + 3]];
    let tout = |k: usize| [v[k * 6] + v[k * 6 + 4], v[k * 6 + 1] + v[k * 6 + 5]];
    let first = m.apply(pt(0));
    let mut pts = vec![[first[0] as f32, first[1] as f32]];
    let segs = if closed { n } else { n - 1 };
    for k in 0..segs {
        let j = (k + 1) % n;
        flatten_cubic(m.apply(pt(k)), m.apply(tout(k)), m.apply(tin(j)), m.apply(pt(j)), &mut pts);
    }
    if closed && pts.len() > 1 && pts.first() == pts.last() {
        pts.pop();
    }
    Some(Poly { pts, closed })
}

const KAPPA: f64 = 0.552_284_749_8;

fn ellipse_bez(c: [f64; 2], s: [f64; 2]) -> Vec<f64> {
    let (rx, ry) = (s[0] / 2.0, s[1] / 2.0);
    let (kx, ky) = (rx * KAPPA, ry * KAPPA);
    // top, right, bottom, left - clockwise, as After Effects draws it
    vec![
        c[0], c[1] - ry, -kx, 0.0, kx, 0.0, //
        c[0] + rx, c[1], 0.0, -ky, 0.0, ky, //
        c[0], c[1] + ry, kx, 0.0, -kx, 0.0, //
        c[0] - rx, c[1], 0.0, ky, 0.0, -ky,
    ]
}

fn rect_bez(c: [f64; 2], s: [f64; 2], r: f64) -> Vec<f64> {
    let (hw, hh) = (s[0] / 2.0, s[1] / 2.0);
    let r = r.max(0.0).min(hw).min(hh);
    let (l, t, rt, b) = (c[0] - hw, c[1] - hh, c[0] + hw, c[1] + hh);
    if r <= 0.0 {
        return vec![rt, t, 0.0, 0.0, 0.0, 0.0, rt, b, 0.0, 0.0, 0.0, 0.0, l, b, 0.0, 0.0, 0.0, 0.0, l, t, 0.0, 0.0, 0.0, 0.0];
    }
    let k = r * KAPPA;
    vec![
        rt - r, t, 0.0, 0.0, k, 0.0, //
        rt, t + r, 0.0, -k, 0.0, 0.0, //
        rt, b - r, 0.0, 0.0, 0.0, k, //
        rt - r, b, k, 0.0, 0.0, 0.0, //
        l + r, b, 0.0, 0.0, -k, 0.0, //
        l, b - r, 0.0, k, 0.0, 0.0, //
        l, t + r, 0.0, 0.0, 0.0, -k, //
        l + r, t, -k, 0.0, 0.0, 0.0,
    ]
}

fn star_bez(star: bool, pts: f64, c: [f64; 2], rot: f64, ir: f64, or: f64) -> Vec<f64> {
    let n = (pts.round() as usize).clamp(3, 100);
    let count = if star { n * 2 } else { n };
    let mut v = Vec::with_capacity(count * 6);
    for k in 0..count {
        let r = if star && k % 2 == 1 { ir } else { or };
        let a = (rot - 90.0).to_radians() + std::f64::consts::TAU * k as f64 / count as f64;
        v.extend([c[0] + r * a.cos(), c[1] + r * a.sin(), 0.0, 0.0, 0.0, 0.0]);
    }
    v
}

/// Total length of a polyline (closing edge included when closed).
fn poly_len(p: &Poly) -> f32 {
    let mut l = 0.0;
    for w in p.pts.windows(2) {
        l += dist(w[0], w[1]);
    }
    if p.closed && p.pts.len() > 1 {
        l += dist(p.pts[p.pts.len() - 1], p.pts[0]);
    }
    l
}

fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// The part of `p` between lengths `a` and `b` (0 <= a <= b <= length) as an open polyline.
fn sub_path(p: &Poly, a: f32, b: f32) -> Option<Poly> {
    let mut pts: Vec<[f32; 2]> = p.pts.clone();
    if p.closed {
        pts.push(p.pts[0]);
    }
    let mut out = Vec::new();
    let mut at = 0.0;
    for w in pts.windows(2) {
        let l = dist(w[0], w[1]);
        let (s0, s1) = (at, at + l);
        at = s1;
        if s1 < a || s0 > b || l <= 0.0 {
            continue;
        }
        let lerp = |s: f32| {
            let u = ((s - s0) / l).clamp(0.0, 1.0);
            [w[0][0] + (w[1][0] - w[0][0]) * u, w[0][1] + (w[1][1] - w[0][1]) * u]
        };
        if out.is_empty() {
            out.push(lerp(a.max(s0)));
        }
        out.push(lerp(b.min(s1)));
    }
    (out.len() >= 2).then_some(Poly { pts: out, closed: false })
}

/// Trim paths, each path on its own: start / end in percent, offset in degrees (a full turn = the
/// whole length). The kept part may wrap past the end of a closed path (two pieces then).
fn trim(paths: Vec<Poly>, s: f64, e: f64, o: f64) -> Vec<Poly> {
    let (mut s, mut e) = (s.clamp(0.0, 100.0) / 100.0, e.clamp(0.0, 100.0) / 100.0);
    if s > e {
        std::mem::swap(&mut s, &mut e);
    }
    if (e - s) >= 1.0 - 1e-9 {
        return paths;
    }
    if e - s <= 1e-9 {
        return Vec::new();
    }
    let off = (o / 360.0).rem_euclid(1.0);
    let (s, e) = (s + off, e + off);
    let mut out = Vec::new();
    for p in paths {
        let len = poly_len(&p);
        if len <= 0.0 {
            continue;
        }
        let piece = |a: f64, b: f64| sub_path(&p, (a * len as f64) as f32, (b * len as f64) as f32);
        if e <= 1.0 {
            out.extend(piece(s, e));
        } else if s >= 1.0 {
            out.extend(piece(s - 1.0, e - 1.0));
        } else {
            out.extend(piece(s, 1.0));
            out.extend(piece(0.0, e - 1.0));
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------------
// the tree

#[derive(Clone, Debug)]
enum Position {
    Joined(Prop),
    Split(Prop, Prop),
}

#[derive(Clone, Debug)]
struct Transform {
    anchor: Prop,
    pos: Position,
    scale: Prop,
    rot: Prop,
    opacity: Prop,
}

impl Transform {
    fn parse(v: Option<&Value>) -> Transform {
        let g = |k: &str| v.and_then(|v| v.get(k));
        let pos = match g("p") {
            Some(p) if p.get("s").and_then(Value::as_bool) == Some(true) || p.get("x").is_some() => {
                Position::Split(Prop::parse(p.get("x"), &[0.0]), Prop::parse(p.get("y"), &[0.0]))
            }
            p => Position::Joined(Prop::parse(p, &[0.0, 0.0])),
        };
        Transform {
            anchor: Prop::parse(g("a"), &[0.0, 0.0]),
            pos,
            scale: Prop::parse(g("s"), &[100.0, 100.0]),
            rot: Prop::parse(g("r").or_else(|| g("rz")), &[0.0]),
            opacity: Prop::parse(g("o"), &[100.0]),
        }
    }

    fn matrix(&self, t: f64) -> Mat {
        let a = self.anchor.v2(t);
        let p = match &self.pos {
            Position::Joined(p) => p.v2(t),
            Position::Split(x, y) => [x.f(t), y.f(t)],
        };
        let s = self.scale.at(t);
        let (sx, sy) = (s.first().copied().unwrap_or(100.0) / 100.0, s.get(1).copied().unwrap_or(100.0) / 100.0);
        Mat::translate(p[0], p[1]).mul(Mat::rotate(self.rot.f(t))).mul(Mat::scale(sx, sy)).mul(Mat::translate(-a[0], -a[1]))
    }

    fn opacity(&self, t: f64) -> f64 {
        (self.opacity.f(t) / 100.0).clamp(0.0, 1.0)
    }

    fn animated(&self) -> bool {
        let pos = match &self.pos {
            Position::Joined(p) => p.animated(),
            Position::Split(x, y) => x.animated() || y.animated(),
        };
        pos || self.anchor.animated() || self.scale.animated() || self.rot.animated() || self.opacity.animated()
    }
}

#[derive(Clone, Debug)]
struct Gradient {
    radial: bool,
    stops: usize,
    colors: Prop,
    start: Prop,
    end: Prop,
}

#[derive(Clone, Debug)]
enum Item {
    Group { items: Vec<Item>, tr: Option<Transform> },
    Path { d: Prop, closed: bool },
    Rect { p: Prop, s: Prop, r: Prop },
    Ellipse { p: Prop, s: Prop },
    Star { star: bool, pts: Prop, p: Prop, r: Prop, ir: Prop, or: Prop },
    Fill { c: Prop, o: Prop, evenodd: bool },
    Stroke { c: Prop, o: Prop, w: Prop, round_cap: bool },
    GFill { g: Gradient, o: Prop, evenodd: bool },
    GStroke { g: Gradient, o: Prop, w: Prop, round_cap: bool },
    Trim { s: Prop, e: Prop, o: Prop },
    Other,
}

fn parse_gradient(it: &Value) -> Gradient {
    let g = it.get("g");
    Gradient {
        radial: it.get("t").and_then(Value::as_i64) == Some(2),
        stops: g.and_then(|g| g.get("p")).and_then(Value::as_u64).unwrap_or(2) as usize,
        colors: Prop::parse(g.and_then(|g| g.get("k")), &[0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0]),
        start: Prop::parse(it.get("s"), &[0.0, 0.0]),
        end: Prop::parse(it.get("e"), &[100.0, 0.0]),
    }
}

fn parse_items(v: Option<&Value>, unsupported: &mut BTreeSet<&'static str>) -> Vec<Item> {
    let mut out = Vec::new();
    for it in v.and_then(Value::as_array).into_iter().flatten() {
        if it.get("hd").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let p = |k: &str, d: &[f64]| Prop::parse(it.get(k), d);
        let ty = it.get("ty").and_then(Value::as_str).unwrap_or("");
        let item = match ty {
            "gr" => {
                let all = it.get("it").and_then(Value::as_array).cloned().unwrap_or_default();
                let tr = all.iter().find(|x| x.get("ty").and_then(Value::as_str) == Some("tr")).map(|t| Transform::parse(Some(t)));
                let rest: Vec<Value> = all.into_iter().filter(|x| x.get("ty").and_then(Value::as_str) != Some("tr")).collect();
                Item::Group { items: parse_items(Some(&Value::Array(rest)), unsupported), tr }
            }
            "sh" => {
                let k = it.get("ks");
                // the closed flag: the static path's, or the first keyframe's
                let first = k.and_then(|k| k.get("k")).map(|k| match k {
                    Value::Array(a) => a.first().and_then(|kf| kf.get("s")).map(|s| s.get(0).unwrap_or(s).clone()),
                    o => Some(o.clone()),
                });
                let closed = first.flatten().and_then(|b| b.get("c").and_then(Value::as_bool)).unwrap_or(false);
                Item::Path { d: Prop::parse(k, &[]), closed }
            }
            "rc" => Item::Rect { p: p("p", &[0.0, 0.0]), s: p("s", &[100.0, 100.0]), r: p("r", &[0.0]) },
            "el" => Item::Ellipse { p: p("p", &[0.0, 0.0]), s: p("s", &[100.0, 100.0]) },
            "sr" => Item::Star {
                star: it.get("sy").and_then(Value::as_i64) != Some(2),
                pts: p("pt", &[5.0]),
                p: p("p", &[0.0, 0.0]),
                r: p("r", &[0.0]),
                ir: p("ir", &[25.0]),
                or: p("or", &[50.0]),
            },
            "fl" => Item::Fill { c: p("c", &[0.0, 0.0, 0.0, 1.0]), o: p("o", &[100.0]), evenodd: it.get("r").and_then(Value::as_i64) == Some(2) },
            "st" => Item::Stroke {
                c: p("c", &[0.0, 0.0, 0.0, 1.0]),
                o: p("o", &[100.0]),
                w: p("w", &[1.0]),
                round_cap: it.get("lc").and_then(Value::as_i64) == Some(2),
            },
            "gf" => Item::GFill { g: parse_gradient(it), o: p("o", &[100.0]), evenodd: it.get("r").and_then(Value::as_i64) == Some(2) },
            "gs" => Item::GStroke {
                g: parse_gradient(it),
                o: p("o", &[100.0]),
                w: p("w", &[1.0]),
                round_cap: it.get("lc").and_then(Value::as_i64) == Some(2),
            },
            "tm" => Item::Trim { s: p("s", &[0.0]), e: p("e", &[100.0]), o: p("o", &[0.0]) },
            other => {
                unsupported.insert(match other {
                    "mm" => "merge paths",
                    "rp" => "repeaters",
                    "rd" => "rounded corners",
                    "pb" => "pucker / bloat",
                    "tw" => "twist",
                    "zz" => "zig zag",
                    "op" => "offset paths",
                    _ => "unknown shape items",
                });
                Item::Other
            }
        };
        out.push(item);
    }
    out
}

#[derive(Clone, Debug)]
enum Kind {
    Shape(Vec<Item>),
    Solid { color: [f32; 4], w: f64, h: f64 },
    Null,
    Precomp(String),
    Skip,
}

#[derive(Clone, Debug)]
struct Layer {
    ind: Option<i64>,
    parent: Option<i64>,
    kind: Kind,
    ks: Transform,
    ip: f64,
    op: f64,
    st: f64,
    sr: f64,
}

fn parse_layers(v: Option<&Value>, unsupported: &mut BTreeSet<&'static str>) -> Vec<Layer> {
    let mut out = Vec::new();
    for l in v.and_then(Value::as_array).into_iter().flatten() {
        let f = |k: &str, d: f64| l.get(k).and_then(Value::as_f64).unwrap_or(d);
        if l.get("hd").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        if l.get("masksProperties").and_then(Value::as_array).is_some_and(|m| !m.is_empty()) {
            unsupported.insert("masks");
        }
        if l.get("tt").is_some() {
            unsupported.insert("track mattes");
        }
        if l.get("ef").and_then(Value::as_array).is_some_and(|m| !m.is_empty()) {
            unsupported.insert("effects");
        }
        if l.get("tm").is_some() {
            unsupported.insert("time remapping");
        }
        let kind = match l.get("ty").and_then(Value::as_i64).unwrap_or(-1) {
            4 => Kind::Shape(parse_items(l.get("shapes"), unsupported)),
            1 => Kind::Solid { color: hex_color(l.get("sc").and_then(Value::as_str).unwrap_or("#000000")), w: f("sw", 0.0), h: f("sh", 0.0) },
            3 => Kind::Null,
            0 => Kind::Precomp(l.get("refId").and_then(Value::as_str).unwrap_or("").to_string()),
            2 => {
                unsupported.insert("image layers");
                Kind::Skip
            }
            5 => {
                unsupported.insert("text layers");
                Kind::Skip
            }
            _ => Kind::Skip,
        };
        let sr = f("sr", 1.0);
        out.push(Layer {
            ind: l.get("ind").and_then(Value::as_i64),
            parent: l.get("parent").and_then(Value::as_i64),
            kind,
            ks: Transform::parse(l.get("ks")),
            ip: f("ip", 0.0),
            op: f("op", f64::INFINITY),
            st: f("st", 0.0),
            sr: if sr.abs() < 1e-9 { 1.0 } else { sr },
        });
    }
    out
}

fn hex_color(s: &str) -> [f32; 4] {
    let h = s.trim_start_matches('#');
    let c = |i: usize| u8::from_str_radix(h.get(i..i + 2).unwrap_or("00"), 16).unwrap_or(0) as f32 / 255.0;
    [c(0), c(2), c(4), 1.0]
}

/// A parsed Lottie animation.
#[derive(Clone, Debug)]
pub struct Comp {
    pub width: u32,
    pub height: u32,
    /// Frames per second.
    pub fps: f64,
    /// In and out frame.
    pub ip: f64,
    pub op: f64,
    layers: Vec<Layer>,
    assets: HashMap<String, Vec<Layer>>,
    /// Features the file uses that are drawn without (see the module doc), for a note to the user.
    pub unsupported: Vec<&'static str>,
}

/// Does this JSON text look like a Lottie animation (the Bodymovin top-level keys)? Cheap enough to
/// ask of any dropped `.json`.
pub fn is_lottie(text: &str) -> bool {
    let head: String = text.chars().take(64 * 1024).collect();
    let has = |k: &str| head.contains(&format!("\"{k}\""));
    has("layers") && has("fr") && has("ip") && has("op") && (has("w") || has("h"))
}

impl Comp {
    pub fn parse(text: &str) -> Result<Comp, String> {
        let v: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| format!("not a Lottie file: {e}"))?;
        if v.get("layers").and_then(Value::as_array).is_none() {
            return Err("not a Lottie file: it has no layers".into());
        }
        let num = |k: &str| v.get(k).and_then(Value::as_f64);
        let (w, h) = (num("w").unwrap_or(512.0), num("h").unwrap_or(512.0));
        if !(w >= 1.0 && h >= 1.0 && w <= 16384.0 && h <= 16384.0) {
            return Err(format!("bad Lottie size {w} x {h}"));
        }
        let fps = num("fr").unwrap_or(30.0);
        if !(fps > 0.0 && fps <= 240.0) {
            return Err(format!("bad Lottie frame rate {fps}"));
        }
        let (ip, op) = (num("ip").unwrap_or(0.0), num("op").unwrap_or(fps));
        if op <= ip {
            return Err("the animation has no frames (out point before in point)".into());
        }
        let mut unsupported = BTreeSet::new();
        if v.get("ddd").and_then(Value::as_i64) == Some(1) {
            unsupported.insert("3D layers");
        }
        let layers = parse_layers(v.get("layers"), &mut unsupported);
        let mut assets = HashMap::new();
        for a in v.get("assets").and_then(Value::as_array).into_iter().flatten() {
            if let (Some(id), Some(_)) = (a.get("id").and_then(Value::as_str), a.get("layers")) {
                assets.insert(id.to_string(), parse_layers(a.get("layers"), &mut unsupported));
            }
        }
        Ok(Comp {
            width: w.round() as u32,
            height: h.round() as u32,
            fps,
            ip,
            op,
            layers,
            assets,
            unsupported: unsupported.into_iter().collect(),
        })
    }

    /// Frames from in to out point.
    pub fn frames(&self) -> usize {
        (self.op - self.ip).ceil().max(1.0) as usize
    }

    /// Draw comp frame `frame` (in the file's own frame numbers) into `out`, straight RGBA,
    /// `w` x `h` (the whole composition scaled to fit exactly).
    pub fn render(&self, frame: f64, w: u32, h: u32, out: &mut Vec<u8>) {
        let mut cv = Canvas::new(w, h);
        let root = Mat::scale(w as f64 / self.width as f64, h as f64 / self.height as f64);
        self.draw_layers(&self.layers, frame, root, 1.0, &mut cv, 0);
        cv.to_rgba(out);
    }

    fn draw_layers(&self, layers: &[Layer], t: f64, root: Mat, opacity: f64, cv: &mut Canvas, depth: usize) {
        if depth > 16 {
            return; // a precomp that includes itself
        }
        let by_ind: HashMap<i64, &Layer> = layers.iter().filter_map(|l| Some((l.ind?, l))).collect();
        // the first layer is the top one: paint from the last
        for l in layers.iter().rev() {
            if t < l.ip || t >= l.op || matches!(l.kind, Kind::Null | Kind::Skip) {
                continue;
            }
            let lt = (t - l.st) / l.sr;
            // the parent chain: each parent's transform at its own time; opacity is not inherited
            let mut m = l.ks.matrix(lt);
            let mut parent = l.parent;
            let mut hops = 0;
            while let Some(p) = parent.and_then(|i| by_ind.get(&i)) {
                m = p.ks.matrix((t - p.st) / p.sr).mul(m);
                parent = p.parent;
                hops += 1;
                if hops > 64 {
                    break;
                }
            }
            let m = root.mul(m);
            let o = opacity * l.ks.opacity(lt);
            if o <= 0.0 {
                continue;
            }
            match &l.kind {
                Kind::Shape(items) => draw_items(items, m, o, lt, cv),
                Kind::Solid { color, w, h } => {
                    let bez = rect_bez([w / 2.0, h / 2.0], [*w, *h], 0.0);
                    if let Some(p) = flatten_bez(&bez, true, &m) {
                        cv.fill(&[p], false, &Paint::Solid(*color), o as f32);
                    }
                }
                Kind::Precomp(id) => {
                    if let Some(inner) = self.assets.get(id) {
                        self.draw_layers(inner, lt, m, o, cv, depth + 1);
                    }
                }
                Kind::Null | Kind::Skip => {}
            }
        }
    }

    /// Does anything move? A still animation becomes a picture instead of a clip.
    pub fn animated(&self) -> bool {
        fn items(v: &[Item]) -> bool {
            v.iter().any(|it| match it {
                Item::Group { items: i, tr } => items(i) || tr.as_ref().is_some_and(Transform::animated),
                Item::Path { d, .. } => d.animated(),
                Item::Rect { p, s, r } => p.animated() || s.animated() || r.animated(),
                Item::Ellipse { p, s } => p.animated() || s.animated(),
                Item::Star { pts, p, r, ir, or, .. } => [pts, p, r, ir, or].iter().any(|x| x.animated()),
                Item::Fill { c, o, .. } => c.animated() || o.animated(),
                Item::Stroke { c, o, w, .. } => c.animated() || o.animated() || w.animated(),
                Item::GFill { g, o, .. } | Item::GStroke { g, o, .. } => {
                    o.animated() || g.colors.animated() || g.start.animated() || g.end.animated()
                }
                Item::Trim { s, e, o } => s.animated() || e.animated() || o.animated(),
                Item::Other => false,
            })
        }
        let layer = |l: &Layer| {
            l.ks.animated()
                || l.ip > self.ip
                || l.op < self.op
                || match &l.kind {
                    Kind::Shape(i) => items(i),
                    Kind::Precomp(_) => true,
                    _ => false,
                }
        };
        self.layers.iter().any(layer)
    }
}

// ---------------------------------------------------------------------------------------------------
// drawing

/// The paths a style at index `k` of `items` paints: every path before it (groups' paths through
/// their own transforms), with the trims between them applied.
fn collect(items: &[Item], m: Mat, t: f64, out: &mut Vec<Poly>) {
    for it in items {
        match it {
            Item::Group { items: inner, tr } => {
                let gm = tr.as_ref().map_or(m, |tr| m.mul(tr.matrix(t)));
                collect(inner, gm, t, out);
            }
            Item::Path { d, closed } => out.extend(flatten_bez(&d.at(t), *closed, &m)),
            Item::Rect { p, s, r } => out.extend(flatten_bez(&rect_bez(p.v2(t), s.v2(t), r.f(t)), true, &m)),
            Item::Ellipse { p, s } => out.extend(flatten_bez(&ellipse_bez(p.v2(t), s.v2(t)), true, &m)),
            Item::Star { star, pts, p, r, ir, or } => {
                out.extend(flatten_bez(&star_bez(*star, pts.f(t), p.v2(t), r.f(t), ir.f(t), or.f(t)), true, &m))
            }
            Item::Trim { s, e, o } => *out = trim(std::mem::take(out), s.f(t), e.f(t), o.f(t)),
            _ => {}
        }
    }
}

/// Paint a group's styles, bottom first: a style at index k paints the paths before it; a nested
/// group paints its own styles where it sits in the stack.
fn draw_items(items: &[Item], m: Mat, opacity: f64, t: f64, cv: &mut Canvas) {
    for k in (0..items.len()).rev() {
        let paths = |cv_paths: &mut Vec<Poly>| collect(&items[..k], m, t, cv_paths);
        match &items[k] {
            Item::Group { items: inner, tr } => {
                let (gm, go) = tr.as_ref().map_or((m, 1.0), |tr| (m.mul(tr.matrix(t)), tr.opacity(t)));
                if go > 0.0 {
                    draw_items(inner, gm, opacity * go, t, cv);
                }
            }
            Item::Fill { c, o, evenodd } => {
                let mut ps = Vec::new();
                paths(&mut ps);
                cv.fill(&ps, *evenodd, &Paint::Solid(color(&c.at(t))), (opacity * o.f(t) / 100.0) as f32);
            }
            Item::GFill { g, o, evenodd } => {
                let mut ps = Vec::new();
                paths(&mut ps);
                cv.fill(&ps, *evenodd, &gradient_paint(g, t, &m), (opacity * o.f(t) / 100.0) as f32);
            }
            Item::Stroke { c, o, w, round_cap } => {
                let mut ps = Vec::new();
                paths(&mut ps);
                let width = (w.f(t) * m.scale_factor()) as f32;
                let outline = stroke_polys(&ps, width, *round_cap);
                cv.fill(&outline, false, &Paint::Solid(color(&c.at(t))), (opacity * o.f(t) / 100.0) as f32);
            }
            Item::GStroke { g, o, w, round_cap } => {
                let mut ps = Vec::new();
                paths(&mut ps);
                let width = (w.f(t) * m.scale_factor()) as f32;
                let outline = stroke_polys(&ps, width, *round_cap);
                cv.fill(&outline, false, &gradient_paint(g, t, &m), (opacity * o.f(t) / 100.0) as f32);
            }
            _ => {}
        }
    }
}

/// A colour value: 0-1 components (0-255 in some old files), alpha 1 when missing.
fn color(v: &[f64]) -> [f32; 4] {
    let big = v.iter().take(3).any(|x| *x > 1.0);
    let c = |i: usize, d: f64| {
        let x = v.get(i).copied().unwrap_or(d);
        (if big && i < 3 { x / 255.0 } else { x }).clamp(0.0, 1.0) as f32
    };
    [c(0, 0.0), c(1, 0.0), c(2, 0.0), c(3, 1.0)]
}

fn gradient_paint(g: &Gradient, t: f64, m: &Mat) -> Paint {
    let raw = g.colors.at(t);
    let n = g.stops.max(1);
    let mut stops: Vec<(f32, [f32; 4])> = (0..n)
        .filter_map(|k| {
            let s = raw.get(k * 4..k * 4 + 4)?;
            Some((s[0] as f32, [s[1] as f32, s[2] as f32, s[3] as f32, 1.0]))
        })
        .collect();
    // alpha stops follow the colour stops as (offset, alpha) pairs
    let alphas: Vec<(f32, f32)> = raw[(n * 4).min(raw.len())..].chunks_exact(2).map(|c| (c[0] as f32, c[1] as f32)).collect();
    if !alphas.is_empty() {
        for st in stops.iter_mut() {
            st.1[3] = sample_1d(&alphas, st.0);
        }
    }
    if stops.is_empty() {
        stops.push((0.0, [0.0, 0.0, 0.0, 1.0]));
    }
    let to32 = |p: [f64; 2]| {
        let q = m.apply(p);
        [q[0] as f32, q[1] as f32]
    };
    let (a, b) = (to32(g.start.v2(t)), to32(g.end.v2(t)));
    if g.radial {
        Paint::Radial { c: a, r: dist(a, b).max(1e-3), stops }
    } else {
        Paint::Linear { a, b, stops }
    }
}

fn sample_1d(stops: &[(f32, f32)], x: f32) -> f32 {
    match stops.iter().position(|s| s.0 >= x) {
        Some(0) => stops[0].1,
        None => stops.last().map_or(1.0, |s| s.1),
        Some(i) => {
            let (a, b) = (stops[i - 1], stops[i]);
            let u = if b.0 > a.0 { (x - a.0) / (b.0 - a.0) } else { 0.0 };
            a.1 + (b.1 - a.1) * u
        }
    }
}

/// A stroke as fill outlines: one quad per segment, a disc at each corner (round joins) and, with
/// round caps, at the ends. Every piece wound the same way, so a non-zero fill is their union.
fn stroke_polys(paths: &[Poly], width: f32, round_cap: bool) -> Vec<Poly> {
    let hw = width / 2.0;
    if hw <= 0.01 {
        return Vec::new();
    }
    let mut out = Vec::new();
    for p in paths {
        let mut pts = p.pts.clone();
        if p.closed && pts.len() > 2 {
            pts.push(pts[0]);
        }
        if pts.len() < 2 {
            continue;
        }
        for w in pts.windows(2) {
            let (a, b) = (w[0], w[1]);
            let l = dist(a, b);
            if l <= 1e-4 {
                continue;
            }
            let (nx, ny) = (-(b[1] - a[1]) / l * hw, (b[0] - a[0]) / l * hw);
            out.push(ccw(vec![[a[0] + nx, a[1] + ny], [b[0] + nx, b[1] + ny], [b[0] - nx, b[1] - ny], [a[0] - nx, a[1] - ny]]));
        }
        // joins where the direction turns (a finely flattened curve needs none)
        let n = pts.len();
        for k in 1..n - 1 {
            let (a, b, c) = (pts[k - 1], pts[k], pts[k + 1]);
            let (u, v) = ([b[0] - a[0], b[1] - a[1]], [c[0] - b[0], c[1] - b[1]]);
            let cos = (u[0] * v[0] + u[1] * v[1]) / (dist(a, b) * dist(b, c)).max(1e-6);
            if cos < 0.97 {
                out.push(disc(b, hw));
            }
        }
        if p.closed && n > 2 {
            out.push(disc(pts[0], hw));
        } else if round_cap {
            out.push(disc(pts[0], hw));
            out.push(disc(pts[n - 1], hw));
        }
    }
    out
}

fn ccw(pts: Vec<[f32; 2]>) -> Poly {
    let area: f32 = (0..pts.len()).map(|i| {
        let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
        a[0] * b[1] - b[0] * a[1]
    }).sum();
    let mut pts = pts;
    if area < 0.0 {
        pts.reverse();
    }
    Poly { pts, closed: true }
}

fn disc(c: [f32; 2], r: f32) -> Poly {
    let n = ((r * 1.5).ceil() as usize).clamp(8, 48);
    ccw((0..n)
        .map(|k| {
            let a = std::f32::consts::TAU * k as f32 / n as f32;
            [c[0] + r * a.cos(), c[1] + r * a.sin()]
        })
        .collect())
}

enum Paint {
    Solid([f32; 4]),
    Linear { a: [f32; 2], b: [f32; 2], stops: Vec<(f32, [f32; 4])> },
    Radial { c: [f32; 2], r: f32, stops: Vec<(f32, [f32; 4])> },
}

fn sample_stops(stops: &[(f32, [f32; 4])], x: f32) -> [f32; 4] {
    let x = x.clamp(0.0, 1.0);
    match stops.iter().position(|s| s.0 >= x) {
        Some(0) => stops[0].1,
        None => stops.last().map_or([0.0; 4], |s| s.1),
        Some(i) => {
            let (a, b) = (&stops[i - 1], &stops[i]);
            let u = if b.0 > a.0 { (x - a.0) / (b.0 - a.0) } else { 0.0 };
            std::array::from_fn(|k| a.1[k] + (b.1[k] - a.1[k]) * u)
        }
    }
}

impl Paint {
    fn at(&self, x: f32, y: f32) -> [f32; 4] {
        match self {
            Paint::Solid(c) => *c,
            Paint::Linear { a, b, stops } => {
                let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                let l2 = (dx * dx + dy * dy).max(1e-6);
                sample_stops(stops, ((x - a[0]) * dx + (y - a[1]) * dy) / l2)
            }
            Paint::Radial { c, r, stops } => sample_stops(stops, dist([x, y], *c) / r),
        }
    }
}

/// Sub-scanlines per pixel row (horizontal coverage is exact).
const SUB: usize = 4;

/// Premultiplied float RGBA.
struct Canvas {
    w: usize,
    h: usize,
    px: Vec<[f32; 4]>,
    cov: Vec<f32>,
}

impl Canvas {
    fn new(w: u32, h: u32) -> Canvas {
        let (w, h) = (w.max(1) as usize, h.max(1) as usize);
        Canvas { w, h, px: vec![[0.0; 4]; w * h], cov: vec![0.0; w + 2] }
    }

    /// Fill `polys` (each closed implicitly) with `paint` at `opacity`, anti-aliased, source-over.
    fn fill(&mut self, polys: &[Poly], evenodd: bool, paint: &Paint, opacity: f32) {
        if opacity <= 0.0 || polys.is_empty() {
            return;
        }
        // edges: (x0, y0, x1, y1, winding), y0 < y1
        let mut edges: Vec<(f32, f32, f32, f32, i32)> = Vec::new();
        let (mut ymin, mut ymax) = (f32::INFINITY, f32::NEG_INFINITY);
        for p in polys {
            let n = p.pts.len();
            if n < 2 {
                continue;
            }
            for k in 0..n {
                let (a, b) = (p.pts[k], p.pts[(k + 1) % n]);
                if !(a[0].is_finite() && a[1].is_finite() && b[0].is_finite() && b[1].is_finite()) || a[1] == b[1] {
                    continue;
                }
                let e = if a[1] < b[1] { (a[0], a[1], b[0], b[1], 1) } else { (b[0], b[1], a[0], a[1], -1) };
                ymin = ymin.min(e.1);
                ymax = ymax.max(e.3);
                edges.push(e);
            }
        }
        if edges.is_empty() {
            return;
        }
        edges.sort_by(|a, b| a.1.total_cmp(&b.1));
        let row0 = ymin.floor().max(0.0) as usize;
        let row1 = (ymax.ceil().max(0.0) as usize).min(self.h);
        let mut xs: Vec<(f32, i32)> = Vec::new();
        // the edges crossing the current sub-row: sub-rows only go down, so edges join in y0 order
        // and leave once their y1 is passed
        let (mut next, mut active) = (0usize, Vec::<usize>::new());
        for row in row0..row1 {
            let (mut lo, mut hi) = (usize::MAX, 0usize);
            for s in 0..SUB {
                let y = row as f32 + (s as f32 + 0.5) / SUB as f32;
                xs.clear();
                while next < edges.len() && edges[next].1 <= y {
                    active.push(next);
                    next += 1;
                }
                active.retain(|&i| edges[i].3 > y);
                for &i in &active {
                    let e = edges[i];
                    let u = (y - e.1) / (e.3 - e.1);
                    xs.push((e.0 + (e.2 - e.0) * u, e.4));
                }
                if xs.len() < 2 {
                    continue;
                }
                xs.sort_by(|a, b| a.0.total_cmp(&b.0));
                let mut wind = 0;
                for k in 0..xs.len() - 1 {
                    wind += xs[k].1;
                    let inside = if evenodd { wind % 2 != 0 } else { wind != 0 };
                    if inside {
                        let (a, b) = (xs[k].0.max(0.0), xs[k + 1].0.min(self.w as f32));
                        if b > a {
                            add_span(&mut self.cov, a, b, 1.0 / SUB as f32);
                            lo = lo.min(a as usize);
                            hi = hi.max((b.ceil() as usize).min(self.w));
                        }
                    }
                }
            }
            if lo >= hi {
                continue;
            }
            let base = row * self.w;
            for x in lo..hi {
                let c = self.cov[x].min(1.0);
                self.cov[x] = 0.0;
                if c <= 0.0 {
                    continue;
                }
                let col = paint.at(x as f32 + 0.5, row as f32 + 0.5);
                let a = col[3] * opacity * c;
                let d = &mut self.px[base + x];
                for k in 0..3 {
                    d[k] = col[k] * a + d[k] * (1.0 - a);
                }
                d[3] = a + d[3] * (1.0 - a);
            }
            let n = self.cov.len();
            self.cov[hi..(hi + 2).min(n)].iter_mut().for_each(|v| *v = 0.0);
        }
    }

    fn to_rgba(&self, out: &mut Vec<u8>) {
        out.clear();
        out.reserve(self.px.len() * 4);
        for p in &self.px {
            let a = p[3].clamp(0.0, 1.0);
            let un = |c: f32| if a > 0.0 { (c / a * 255.0).round().clamp(0.0, 255.0) as u8 } else { 0 };
            out.extend([un(p[0]), un(p[1]), un(p[2]), (a * 255.0).round() as u8]);
        }
    }
}

/// Add coverage `amt` over [a, b) of a row, with the partial end pixels weighted.
fn add_span(row: &mut [f32], a: f32, b: f32, amt: f32) {
    let (ia, ib) = (a.floor() as usize, b.floor() as usize);
    if ia >= row.len() {
        return;
    }
    if ia == ib {
        row[ia] += (b - a) * amt;
        return;
    }
    row[ia] += (ia as f32 + 1.0 - a) * amt;
    let n = row.len();
    for v in row.iter_mut().take(ib.min(n)).skip(ia + 1) {
        *v += amt;
    }
    if ib < row.len() {
        row[ib] += (b - ib as f32) * amt;
    }
}

/// The animation of a `.lottie` (dotLottie) package or a `.json` file, as text: a `.lottie` is a zip
/// whose `manifest.json` names its animations (the first is used); Windows' own `tar` opens it.
pub fn read_file(path: &std::path::Path) -> Result<String, String> {
    let is_pkg = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("lottie"));
    if !is_pkg {
        return crate::engine::subtitles::read_file(path);
    }
    let dir = std::env::temp_dir().join(format!("simple-editor-lottie-{}-{}", std::process::id(), stamp()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut c = std::process::Command::new("tar");
    c.arg("-xf").arg(path).arg("-C").arg(&dir);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let r = c.output();
    let found = (|| {
        let ok = r.map_err(|e| format!("could not run tar: {e}"))?;
        if !ok.status.success() {
            return Err(format!("could not open the .lottie package: {}", String::from_utf8_lossy(&ok.stderr).lines().next().unwrap_or("")));
        }
        let anim_dir = dir.join("animations");
        let manifest: Option<Value> = std::fs::read_to_string(dir.join("manifest.json")).ok().and_then(|t| serde_json::from_str(&t).ok());
        let id = manifest.as_ref().and_then(|m| m.get("animations")?.as_array()?.first()?.get("id")?.as_str().map(String::from));
        let named = id.map(|id| anim_dir.join(format!("{id}.json"))).filter(|p| p.is_file());
        let any = || {
            let mut v: Vec<_> = std::fs::read_dir(&anim_dir).ok()?.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "json")).collect();
            v.sort();
            v.into_iter().next()
        };
        let file = named.or_else(any).ok_or("the .lottie package has no animation in it")?;
        crate::engine::subtitles::read_file(&file)
    })();
    let _ = std::fs::remove_dir_all(&dir);
    found
}

fn stamp() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
}

/// `(the rendered size, fps)` for a comp: its own size fitted inside 4K, even sides; its own frame
/// rate (Lottie files are 24-60 fps).
pub fn output_size(c: &Comp) -> (u32, u32) {
    let fit = (3840.0 / c.width as f64).min(2160.0 / c.height as f64).min(1.0);
    let even = |v: f64| ((((v * fit).round() as u32) + 1) & !1).max(2);
    (even(c.width as f64), even(c.height as f64))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(buf: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * w + x) * 4) as usize;
        [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
    }

    /// A 100x100, 30 fps, 1 s animation: a red rectangle moving right over a blue ellipse (the rect
    /// is the first layer, so on top), parented to a null that shifts everything down 10 px.
    const MOVING: &str = r#"{
        "v":"5.7.4","fr":30,"ip":0,"op":30,"w":100,"h":100,"nm":"Test","ddd":0,"assets":[],
        "layers":[
          {"ddd":0,"ind":1,"ty":4,"nm":"rect","parent":3,"ks":{"o":{"a":0,"k":100},"r":{"a":0,"k":0},
            "p":{"a":1,"k":[{"t":0,"s":[20,40],"i":{"x":[1],"y":[1]},"o":{"x":[0],"y":[0]}},{"t":30,"s":[80,40]}]},
            "a":{"a":0,"k":[0,0]},"s":{"a":0,"k":[100,100]}},
            "shapes":[{"ty":"gr","it":[
              {"ty":"rc","p":{"a":0,"k":[0,0]},"s":{"a":0,"k":[20,20]},"r":{"a":0,"k":0}},
              {"ty":"fl","c":{"a":0,"k":[1,0,0,1]},"o":{"a":0,"k":100},"r":1},
              {"ty":"tr","p":{"a":0,"k":[0,0]},"a":{"a":0,"k":[0,0]},"s":{"a":0,"k":[100,100]},"r":{"a":0,"k":0},"o":{"a":0,"k":100}}
            ]}],"ip":0,"op":30,"st":0},
          {"ddd":0,"ind":2,"ty":4,"nm":"circle","ks":{"p":{"a":0,"k":[50,50]}},
            "shapes":[{"ty":"el","p":{"a":0,"k":[0,0]},"s":{"a":0,"k":[60,60]}},
                      {"ty":"fl","c":{"a":0,"k":[0,0,1,1]},"o":{"a":0,"k":100}}],"ip":0,"op":30,"st":0},
          {"ddd":0,"ind":3,"ty":3,"nm":"null","ks":{"p":{"a":0,"k":[0,10]}},"ip":0,"op":30,"st":0}
        ]}"#;

    #[test]
    fn parses_the_header() {
        let c = Comp::parse(MOVING).unwrap();
        assert_eq!((c.width, c.height, c.fps, c.frames()), (100, 100, 30.0, 30));
        assert!(c.unsupported.is_empty(), "{:?}", c.unsupported);
        assert!(c.animated());
        assert!(is_lottie(MOVING));
        assert!(!is_lottie(r#"{"name":"package","version":"1.0"}"#));
        assert!(Comp::parse("{}").is_err() && Comp::parse("[1,2]").is_err() && Comp::parse("nope").is_err());
        assert!(Comp::parse(r#"{"fr":30,"ip":10,"op":5,"w":10,"h":10,"layers":[]}"#).is_err(), "no frames");
    }

    #[test]
    fn draws_layers_in_order_with_parenting_and_motion() {
        let c = Comp::parse(MOVING).unwrap();
        let mut buf = Vec::new();
        c.render(0.0, 100, 100, &mut buf);
        assert_eq!(buf.len(), 100 * 100 * 4);
        // the rect: centred at (20, 40) + the null's (0, 10) = (20, 50); 20x20
        assert_eq!(px(&buf, 100, 20, 50), [255, 0, 0, 255], "the red rect at frame 0");
        assert_eq!(px(&buf, 100, 50, 50), [0, 0, 255, 255], "the blue circle at the centre");
        assert_eq!(px(&buf, 100, 2, 2)[3], 0, "transparent outside");
        // the rect's x keyframes are linear: halfway at frame 15 = x 50, on top of the circle
        c.render(15.0, 100, 100, &mut buf);
        assert_eq!(px(&buf, 100, 50, 50), [255, 0, 0, 255], "the first layer is the top one");
        assert_eq!(px(&buf, 100, 26, 50), [0, 0, 255, 255], "the circle shows where the rect was");
        // scaled output: half size, same picture
        c.render(0.0, 50, 50, &mut buf);
        assert_eq!(px(&buf, 50, 10, 25), [255, 0, 0, 255]);
        // edges are anti-aliased: a pixel half on the circle's rim is neither empty nor full
        c.render(0.0, 100, 100, &mut buf);
        let partial = buf.chunks_exact(4).filter(|p| p[3] > 10 && p[3] < 245).count();
        assert!(partial > 40, "a 60 px circle has a soft rim: {partial} partial pixels");
    }

    #[test]
    fn eases_holds_and_old_keyframes() {
        // a hold keeps the first value up to the next key
        let hold: Value = serde_json::from_str(r#"{"a":1,"k":[{"t":0,"s":[0],"h":1},{"t":10,"s":[100]}]}"#).unwrap();
        let p = Prop::parse(Some(&hold), &[0.0]);
        assert_eq!(p.f(9.9), 0.0);
        assert_eq!(p.f(10.0), 100.0);
        // ease in-out: slower than linear at a quarter, faster in the middle, symmetric
        let eased: Value = serde_json::from_str(
            r#"{"a":1,"k":[{"t":0,"s":[0],"o":{"x":[0.42],"y":[0]},"i":{"x":[0.58],"y":[1]}},{"t":100,"s":[100]}]}"#,
        )
        .unwrap();
        let p = Prop::parse(Some(&eased), &[0.0]);
        assert!(p.f(25.0) < 25.0 && p.f(25.0) > 5.0, "{}", p.f(25.0));
        assert!((p.f(50.0) - 50.0).abs() < 0.5);
        assert!((p.f(25.0) + p.f(75.0) - 100.0).abs() < 0.5);
        // the old format: the end value is the key's own `e`; past the last key it stays
        let old: Value = serde_json::from_str(r#"{"a":1,"k":[{"t":0,"s":[10,0],"e":[20,0]},{"t":10}]}"#).unwrap();
        let p = Prop::parse(Some(&old), &[0.0, 0.0]);
        assert_eq!(p.v2(5.0), [15.0, 0.0]);
        assert_eq!(p.v2(50.0), [20.0, 0.0]);
        assert_eq!(p.v2(-5.0), [10.0, 0.0]);
        // a position along a spatial bezier is pulled off the straight line
        let curved: Value =
            serde_json::from_str(r#"{"a":1,"k":[{"t":0,"s":[0,0],"to":[0,30],"ti":[0,30]},{"t":10,"s":[100,0]}]}"#).unwrap();
        let p = Prop::parse(Some(&curved), &[0.0, 0.0]);
        assert!(p.v2(5.0)[1] > 15.0, "{:?}", p.v2(5.0));
    }

    #[test]
    fn strokes_trims_stars_and_gradients_draw() {
        let src = r#"{"fr":30,"ip":0,"op":10,"w":100,"h":100,"layers":[
          {"ty":4,"ind":1,"ks":{},"ip":0,"op":10,"st":0,"shapes":[
            {"ty":"sh","ks":{"a":0,"k":{"c":false,"v":[[10,10],[90,10]],"i":[[0,0],[0,0]],"o":[[0,0],[0,0]]}}},
            {"ty":"tm","s":{"a":0,"k":0},"e":{"a":0,"k":50},"o":{"a":0,"k":0}},
            {"ty":"st","c":{"a":0,"k":[0,1,0,1]},"o":{"a":0,"k":100},"w":{"a":0,"k":6},"lc":1}
          ]},
          {"ty":4,"ind":2,"ks":{},"ip":0,"op":10,"st":0,"shapes":[
            {"ty":"sr","sy":2,"pt":{"a":0,"k":6},"p":{"a":0,"k":[50,60]},"r":{"a":0,"k":0},"or":{"a":0,"k":20}},
            {"ty":"gf","t":1,"g":{"p":2,"k":{"a":0,"k":[0,1,1,1,1,0,0,0]}},"s":{"a":0,"k":[30,0]},"e":{"a":0,"k":[70,0]},"o":{"a":0,"k":100}}
          ]}]}"#;
        let c = Comp::parse(src).unwrap();
        assert!(!c.animated(), "nothing moves");
        let mut buf = Vec::new();
        c.render(0.0, 100, 100, &mut buf);
        assert_eq!(px(&buf, 100, 30, 10), [0, 255, 0, 255], "the trimmed stroke's first half");
        assert_eq!(px(&buf, 100, 70, 10)[3], 0, "its second half is trimmed away");
        assert_eq!(px(&buf, 100, 30, 14)[3], 0, "6 px wide: nothing 4 px off the line");
        // the hexagon's gradient runs white (left) to black (right)
        let (l, r) = (px(&buf, 100, 36, 60), px(&buf, 100, 64, 60));
        assert_eq!((l[3], r[3]), (255, 255));
        assert!(l[0] > 200 && r[0] < 60, "{l:?} {r:?}");
    }

    #[test]
    fn notes_what_it_draws_without() {
        let src = r#"{"fr":30,"ip":0,"op":10,"w":10,"h":10,"ddd":1,"layers":[
          {"ty":5,"ks":{},"ip":0,"op":10},
          {"ty":4,"ks":{},"ip":0,"op":10,"tt":1,"masksProperties":[{}],"shapes":[{"ty":"rp"}]}]}"#;
        let c = Comp::parse(src).unwrap();
        for f in ["3D layers", "masks", "repeaters", "text layers", "track mattes"] {
            assert!(c.unsupported.contains(&f), "{f}: {:?}", c.unsupported);
        }
        let mut buf = Vec::new();
        c.render(0.0, 10, 10, &mut buf); // never panics on what it skips
    }

    #[test]
    fn precomps_and_solids() {
        let src = r##"{"fr":25,"ip":0,"op":50,"w":40,"h":40,"assets":[{"id":"comp_0","layers":[
            {"ty":1,"ind":1,"sc":"#ffff00","sw":10,"sh":10,"ks":{"p":{"a":0,"k":[5,5]},"a":{"a":0,"k":[5,5]}},"ip":0,"op":50,"st":0}]}],
          "layers":[{"ty":0,"ind":1,"refId":"comp_0","ks":{"p":{"a":0,"k":[20,20]},"a":{"a":0,"k":[0,0]},"s":{"a":0,"k":[200,200]}},"ip":0,"op":50,"st":0,"w":40,"h":40}]}"##;
        let c = Comp::parse(src).unwrap();
        let mut buf = Vec::new();
        c.render(0.0, 40, 40, &mut buf);
        // the 10x10 solid at (0..10) in the precomp, scaled 2x and moved to (20, 20): (20..40)
        assert_eq!(px(&buf, 40, 25, 25), [255, 255, 0, 255]);
        assert_eq!(px(&buf, 40, 15, 15)[3], 0);
        assert_eq!(output_size(&c), (40, 40));
    }

    #[test]
    fn trims_wrap_around_closed_paths() {
        let sq = Poly { pts: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]], closed: true };
        assert!((poly_len(&sq) - 40.0).abs() < 1e-4);
        let t = trim(vec![sq.clone()], 0.0, 25.0, 270.0); // 75%..100%: the left side, going up
        assert_eq!(t.len(), 1);
        assert!((poly_len(&t[0]) - 10.0).abs() < 1e-3, "{:?}", t[0]);
        let t = trim(vec![sq.clone()], 50.0, 100.0, 180.0); // wraps: 100%..150%
        assert!((t.iter().map(poly_len).sum::<f32>() - 20.0).abs() < 1e-3);
        assert!(trim(vec![sq.clone()], 30.0, 30.0, 0.0).is_empty());
        assert_eq!(trim(vec![sq], 0.0, 100.0, 0.0).len(), 1);
    }
}
