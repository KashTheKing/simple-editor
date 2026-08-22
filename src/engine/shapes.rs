//! Vector shapes and recorded drawings -> RGBA layers (CPU rasteriser; the GPU path samples the result).
//!
//! `render(style, scale, local_t)` draws the shape centred in its own tight layer:
//!  * Rect / Ellipse / Triangle / Polygon / Star: `fill` plus an optional `stroke` outline of
//!    `stroke_width` (project px x scale); rounded corners for Rect via `corner`.
//!  * Line / Arrow: from (-w, -h) to (+w, +h) in layer space, head sized by `corner`.
//!  * Draw: the recorded strokes revealed up to `local_t * draw_rate` (0 = all at once) on the optional
//!    `page` background.
//! Anti-aliased with a coverage rasteriser (no new deps). Cached by `ShapeStyle::cache_key()` + size +
//! reveal bucket.

use crate::media::Frame;
use crate::model::{ShapeKind, ShapeStyle};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// Vertical sub-scanlines per pixel row (horizontal coverage is exact, so 4 is plenty).
const SUB: usize = 4;
/// Layer dimensions are clamped to this; `scale` shrinks instead so the geometry stays consistent.
const MAX_DIM: f32 = 8192.0;
/// ...and so is the total pixel count (64 MB RGBA), which a 4K full-canvas shape stays well under.
const MAX_PIXELS: f32 = 16.0e6;
/// Reveal-time quantisation for the cache (a drawing re-rasterises 30x/s at most).
const REVEAL_HZ: f64 = 30.0;
/// Star inner radius as a fraction of the outer one.
const STAR_INNER: f32 = 0.45;
const CACHE_MAX: usize = 48;

#[derive(Default)]
pub struct ShapeRasterizer {
    cache: std::collections::HashMap<(u64, u32, u32, u32), Arc<Frame>>,
    /// Scratch: one coverage buffer, one polygon soup, one edge table, one outline.
    cov: Vec<f32>,
    path: Path,
    edges: Vec<Edge>,
    active: Vec<usize>,
    xs: Vec<(f32, i32)>,
    outline: Vec<[f32; 2]>,
}

impl ShapeRasterizer {
    pub fn new() -> Self {
        Self::default()
    }
    /// Layer size in project px for a style at clip-local time `t` (before `scale`).
    pub fn size(style: &ShapeStyle, t: f64) -> (f32, f32) {
        let (w, h) = half_size(style, t);
        let sw = if style.stroke_width.is_finite() { style.stroke_width.max(0.0) } else { 0.0 };
        let (fw, fh) = match style.kind {
            ShapeKind::Draw => draw_half_size(style),
            ShapeKind::Line => {
                let pad = sw.max(1.0);
                (w + pad * 0.5, h + pad * 0.5)
            }
            ShapeKind::Arrow => {
                let pad = sw.max(1.0).max(arrow_head(style));
                (w + pad * 0.5, h + pad * 0.5)
            }
            // Only pad for a stroke that is actually visible, so a plain rect gets an exact layer.
            _ => {
                let pad = if style.stroke[3] > 0 { sw * 0.5 } else { 0.0 };
                (w + pad, h + pad)
            }
        };
        ((fw * 2.0).max(1.0), (fh * 2.0).max(1.0))
    }

    /// Rasterise at `scale` (canvas px per project px) for clip-local time `t`.
    pub fn render(&mut self, style: &ShapeStyle, scale: f32, t: f64) -> Arc<Frame> {
        let (pw, ph) = Self::size(style, t);
        let mut s = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
        s = s.min(MAX_DIM / pw).min(MAX_DIM / ph);
        let area = pw * s * ph * s;
        if area > MAX_PIXELS {
            s *= (MAX_PIXELS / area).sqrt();
        }
        s = s.max(1e-3);
        let lw = (pw * s).round().clamp(1.0, MAX_DIM) as u32;
        let lh = (ph * s).round().clamp(1.0, MAX_DIM) as u32;
        let bucket = reveal_bucket(style, t);
        let mut hh = DefaultHasher::new();
        style.cache_key().hash(&mut hh);
        for f in [s, pw, ph] {
            f.to_bits().hash(&mut hh);
        }
        let key = (hh.finish(), lw, lh, bucket);
        if let Some(f) = self.cache.get(&key) {
            return f.clone();
        }
        let frame = Arc::new(self.rasterize(style, s, t, lw, lh, bucket));
        if self.cache.len() >= CACHE_MAX {
            // ponytail: drop-all cache — LRU if shape-heavy projects thrash
            self.cache.clear();
        }
        self.cache.insert(key, frame.clone());
        frame
    }

    fn rasterize(&mut self, style: &ShapeStyle, s: f32, t: f64, lw: u32, lh: u32, bucket: u32) -> Frame {
        let mut out = Frame::new(lw, lh);
        let (cx, cy) = (lw as f32 * 0.5, lh as f32 * 0.5);
        let (w, h) = half_size(style, t);
        let sw = (style.stroke_width.max(0.0) * s).max(0.0);
        self.path.clear();
        match style.kind {
            ShapeKind::Draw => {
                if style.page[3] > 0 {
                    out.fill(style.page);
                }
                let reveal = bucket_time(bucket);
                for st in &style.strokes {
                    revealed(&st.points, reveal, &mut self.outline);
                    if self.outline.is_empty() {
                        continue;
                    }
                    to_layer(&mut self.outline, cx, cy, s);
                    self.path.clear();
                    add_stroke(&mut self.path, &self.outline, false, st.width.max(0.1) * s * 0.5);
                    self.flush(&mut out, st.color);
                }
                return out;
            }
            ShapeKind::Line | ShapeKind::Arrow => {
                let head = if style.kind == ShapeKind::Arrow { arrow_head(style) * s } else { 0.0 };
                let (a, b) = ([cx - w * s, cy - h * s], [cx + w * s, cy + h * s]);
                let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                let len = (dx * dx + dy * dy).sqrt();
                if len > 1e-4 {
                    let (ux, uy) = (dx / len, dy / len);
                    // stop the shaft inside the head so it does not poke through the tip
                    let end = [b[0] - ux * head * 0.6, b[1] - uy * head * 0.6];
                    self.outline.clear();
                    self.outline.push(a);
                    self.outline.push(end);
                    add_stroke(&mut self.path, &self.outline, false, (sw * 0.5).max(0.35));
                    if head > 0.0 {
                        let base = [b[0] - ux * head, b[1] - uy * head];
                        let (px, py) = (-uy * head * 0.5, ux * head * 0.5);
                        self.path.push(b);
                        self.path.push([base[0] + px, base[1] + py]);
                        self.path.push([base[0] - px, base[1] - py]);
                        self.path.end();
                    }
                } else if sw > 0.0 {
                    add_disc(&mut self.path, [cx, cy], (sw * 0.5).max(0.35));
                }
                self.flush(&mut out, style.stroke);
                return out;
            }
            _ => shape_outline(style, w, h, s, &mut self.outline),
        }
        to_layer(&mut self.outline, cx, cy, s);
        if style.fill[3] > 0 {
            for p in self.outline.iter() {
                self.path.push(*p);
            }
            self.path.end();
            self.flush(&mut out, style.fill);
        }
        if style.stroke[3] > 0 && sw > 0.0 {
            self.path.clear();
            add_stroke(&mut self.path, &self.outline, true, (sw * 0.5).max(0.35));
            self.flush(&mut out, style.stroke);
        }
        out
    }

    /// Rasterise the current path into the scratch coverage buffer and composite `color` over `out`.
    fn flush(&mut self, out: &mut Frame, color: [u8; 4]) {
        if color[3] == 0 {
            self.path.clear();
            return;
        }
        let (w, h) = (out.width as usize, out.height as usize);
        let band = fill_path(&self.path, w, h, &mut self.cov, &mut self.edges, &mut self.active, &mut self.xs);
        self.path.clear();
        let Some((x0, y0, x1, y1)) = band else { return };
        for y in y0..y1 {
            for x in x0..x1 {
                let c = self.cov[y * w + x];
                if c > 0.0015 {
                    let i = (y * w + x) * 4;
                    blend(&mut out.rgba[i..i + 4], color, c);
                }
            }
        }
    }
}

// ---------- geometry ----------

fn half_size(style: &ShapeStyle, t: f64) -> (f32, f32) {
    let f = |a: f64| {
        let v = a as f32;
        if v.is_finite() {
            v.abs().min(20000.0)
        } else {
            0.0
        }
    };
    (f(style.w.at(t)), f(style.h.at(t)))
}

/// Arrow head length in project px (`corner`, or a stroke-relative default when it is 0).
fn arrow_head(style: &ShapeStyle) -> f32 {
    if style.corner > 0.0 && style.corner.is_finite() {
        style.corner
    } else {
        (style.stroke_width.max(0.0) * 4.0).max(8.0)
    }
}

/// Half-size of a drawing: the strokes are relative to the layer centre, so the layer is symmetric.
fn draw_half_size(style: &ShapeStyle) -> (f32, f32) {
    let (mut hx, mut hy) = (0.0f32, 0.0f32);
    for st in &style.strokes {
        let r = st.width.max(0.5) * 0.5;
        for p in &st.points {
            if p.0.is_finite() && p.1.is_finite() {
                hx = hx.max(p.0.abs() + r);
                hy = hy.max(p.1.abs() + r);
            }
        }
    }
    if style.page[3] > 0 {
        let (w, h) = (style.w.value as f32, style.h.value as f32);
        hx = hx.max(w.abs());
        hy = hy.max(h.abs());
    }
    (hx.max(0.5), hy.max(0.5))
}

/// Closed outline of a filled shape in project px, centred at the origin. `s` only sets how finely
/// curves are subdivided (so a big shape at a big scale stays smooth).
fn shape_outline(style: &ShapeStyle, w: f32, h: f32, s: f32, out: &mut Vec<[f32; 2]>) {
    out.clear();
    match style.kind {
        ShapeKind::Rect => {
            let r = style.corner.max(0.0).min(w).min(h);
            if r <= 0.01 {
                out.extend_from_slice(&[[-w, -h], [w, -h], [w, h], [-w, h]]);
            } else {
                let n = arc_steps(r * s);
                // corners clockwise from top-left, y down
                for (i, (cx, cy)) in
                    [(-w + r, -h + r), (w - r, -h + r), (w - r, h - r), (-w + r, h - r)].into_iter().enumerate()
                {
                    let a0 = std::f32::consts::PI * (1.0 + 0.5 * i as f32);
                    for k in 0..=n {
                        let a = a0 + std::f32::consts::FRAC_PI_2 * (k as f32 / n as f32);
                        out.push([cx + r * a.cos(), cy + r * a.sin()]);
                    }
                }
            }
        }
        ShapeKind::Ellipse => {
            let n = arc_steps(w.max(h) * s) * 4;
            for i in 0..n {
                let a = std::f32::consts::TAU * (i as f32 / n as f32);
                out.push([w * a.cos(), h * a.sin()]);
            }
        }
        ShapeKind::Triangle => out.extend_from_slice(&[[0.0, -h], [w, h], [-w, h]]),
        ShapeKind::Star => out.extend(ngon(style.sides, w, h, true)),
        _ => out.extend(ngon(style.sides, w, h, false)),
    }
}

/// Regular polygon (or star: 2x the vertices, alternating radii) inscribed in the w x h ellipse,
/// first vertex pointing up.
fn ngon(sides: u32, w: f32, h: f32, star: bool) -> Vec<[f32; 2]> {
    let n = sides.clamp(3, 64) as usize;
    let count = if star { n * 2 } else { n };
    (0..count)
        .map(|i| {
            let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * (i as f32 / count as f32);
            let r = if star && i % 2 == 1 { STAR_INNER } else { 1.0 };
            [w * r * a.cos(), h * r * a.sin()]
        })
        .collect()
}

/// Segments per quarter turn for a curve of pixel radius `r_px` (sagitta stays well under a pixel).
fn arc_steps(r_px: f32) -> usize {
    ((r_px.max(1.0) / 3.0) as usize).clamp(4, 64)
}

/// Points of a stroke revealed at `reveal` seconds (the last segment is cut proportionally).
fn revealed(points: &[(f32, f32, f32)], reveal: f32, out: &mut Vec<[f32; 2]>) {
    out.clear();
    if points.is_empty() {
        return;
    }
    if !reveal.is_finite() {
        out.extend(points.iter().map(|p| [p.0, p.1]));
        return;
    }
    for (i, p) in points.iter().enumerate() {
        if p.2 <= reveal {
            out.push([p.0, p.1]);
            continue;
        }
        if i > 0 {
            let q = points[i - 1];
            let span = p.2 - q.2;
            let f = if span > 0.0 { ((reveal - q.2) / span).clamp(0.0, 1.0) } else { 0.0 };
            if f > 0.0 {
                out.push([q.0 + (p.0 - q.0) * f, q.1 + (p.1 - q.1) * f]);
            }
        }
        break;
    }
}

fn to_layer(pts: &mut [[f32; 2]], cx: f32, cy: f32, s: f32) {
    for p in pts {
        p[0] = cx + p[0] * s;
        p[1] = cy + p[1] * s;
    }
}

/// Reveal bucket for the cache key: 0 = not a drawing, `u32::MAX` = fully revealed.
fn reveal_bucket(style: &ShapeStyle, t: f64) -> u32 {
    if style.kind != ShapeKind::Draw {
        return 0;
    }
    let rate = style.draw_rate as f64;
    if !(rate > 0.0) || !rate.is_finite() {
        return u32::MAX;
    }
    let r = t.max(0.0) * rate;
    if !r.is_finite() || r >= style.draw_duration() {
        return u32::MAX;
    }
    ((r * REVEAL_HZ) as u32).min(u32::MAX - 1)
}

fn bucket_time(bucket: u32) -> f32 {
    if bucket == u32::MAX {
        f32::INFINITY
    } else {
        (bucket as f64 / REVEAL_HZ) as f32
    }
}

// ---------- polygon soup ----------

/// `pts[ends[i-1]..ends[i]]` is one closed subpath, all wound the same way so a nonzero fill
/// unions overlapping pieces instead of cancelling them.
#[derive(Default)]
struct Path {
    pts: Vec<[f32; 2]>,
    ends: Vec<usize>,
}

impl Path {
    fn clear(&mut self) {
        self.pts.clear();
        self.ends.clear();
    }
    fn push(&mut self, p: [f32; 2]) {
        self.pts.push(p);
    }
    /// Close the subpath started after the previous one, dropping degenerate ones.
    fn end(&mut self) {
        let start = self.ends.last().copied().unwrap_or(0);
        if self.pts.len() < start + 3 {
            self.pts.truncate(start);
            return;
        }
        let seg = &mut self.pts[start..];
        let mut area = 0.0f32;
        for i in 0..seg.len() {
            let a = seg[i];
            let b = seg[(i + 1) % seg.len()];
            area += a[0] * b[1] - b[0] * a[1];
        }
        if area < 0.0 {
            seg.reverse();
        }
        self.ends.push(self.pts.len());
    }
}

fn add_disc(path: &mut Path, c: [f32; 2], r: f32) {
    let n = ((r * 2.0) as usize).clamp(6, 40);
    for i in 0..n {
        let a = std::f32::consts::TAU * (i as f32 / n as f32);
        path.push([c[0] + r * a.cos(), c[1] + r * a.sin()]);
    }
    path.end();
}

/// Thick polyline in layer px: one quad per segment plus a disc at every vertex (round caps/joins).
fn add_stroke(path: &mut Path, pts: &[[f32; 2]], closed: bool, half: f32) {
    let half = half.max(0.35);
    let n = pts.len();
    if n == 0 {
        return;
    }
    let segs = if closed { n } else { n.saturating_sub(1) };
    for i in 0..segs {
        let a = pts[i];
        let b = pts[(i + 1) % n];
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = (dx * dx + dy * dy).sqrt();
        if !(len > 1e-5) {
            continue;
        }
        let (nx, ny) = (-dy / len * half, dx / len * half);
        path.push([a[0] + nx, a[1] + ny]);
        path.push([b[0] + nx, b[1] + ny]);
        path.push([b[0] - nx, b[1] - ny]);
        path.push([a[0] - nx, a[1] - ny]);
        path.end();
    }
    if half > 0.6 || segs == 0 {
        for p in pts {
            add_disc(path, *p, half);
        }
    }
}

// ---------- coverage rasteriser ----------

/// A non-horizontal edge, top to bottom.
struct Edge {
    y0: f32,
    y1: f32,
    x0: f32,
    dxdy: f32,
    dir: i32,
}

/// Scanline fill with 4x vertical sub-sampling and exact horizontal coverage. Writes `cov`
/// (zeroing only the band it touches) and returns that band as (x0, y0, x1, y1).
fn fill_path(
    path: &Path,
    w: usize,
    h: usize,
    cov: &mut Vec<f32>,
    edges: &mut Vec<Edge>,
    active: &mut Vec<usize>,
    xs: &mut Vec<(f32, i32)>,
) -> Option<(usize, usize, usize, usize)> {
    if path.ends.is_empty() || w == 0 || h == 0 {
        return None;
    }
    edges.clear();
    let (mut mnx, mut mny, mut mxx, mut mxy) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    let mut start = 0;
    for &end in &path.ends {
        let sub = &path.pts[start..end];
        for i in 0..sub.len() {
            let a = sub[i];
            let b = sub[(i + 1) % sub.len()];
            if !(a[0].is_finite() && a[1].is_finite() && b[0].is_finite() && b[1].is_finite()) {
                continue;
            }
            mnx = mnx.min(a[0]);
            mxx = mxx.max(a[0]);
            mny = mny.min(a[1]);
            mxy = mxy.max(a[1]);
            if a[1] == b[1] {
                continue;
            }
            let (top, bot, dir) = if a[1] < b[1] { (a, b, 1) } else { (b, a, -1) };
            edges.push(Edge { y0: top[1], y1: bot[1], x0: top[0], dxdy: (bot[0] - top[0]) / (bot[1] - top[1]), dir });
        }
        start = end;
    }
    if edges.is_empty() || mnx > mxx {
        return None;
    }
    let x0 = (mnx.floor().max(0.0) as usize).min(w);
    let x1 = ((mxx.ceil().max(0.0) as usize) + 1).min(w);
    let y0 = (mny.floor().max(0.0) as usize).min(h);
    let y1 = ((mxy.ceil().max(0.0) as usize) + 1).min(h);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    if cov.len() < w * h {
        cov.resize(w * h, 0.0);
    }
    for y in y0..y1 {
        cov[y * w + x0..y * w + x1].fill(0.0);
    }
    edges.sort_by(|a, b| a.y0.partial_cmp(&b.y0).unwrap_or(std::cmp::Ordering::Equal));
    active.clear();
    let mut cursor = 0usize;
    let amt = 1.0 / SUB as f32;
    for y in y0..y1 {
        for s in 0..SUB {
            let sy = y as f32 + (s as f32 + 0.5) / SUB as f32;
            while cursor < edges.len() && edges[cursor].y0 <= sy {
                active.push(cursor);
                cursor += 1;
            }
            active.retain(|&i| edges[i].y1 > sy);
            if active.len() < 2 {
                continue;
            }
            xs.clear();
            for &i in active.iter() {
                let e = &edges[i];
                if e.y0 <= sy {
                    xs.push((e.x0 + (sy - e.y0) * e.dxdy, e.dir));
                }
            }
            if xs.len() < 2 {
                continue;
            }
            xs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            let row = &mut cov[y * w..y * w + w];
            let mut wind = 0;
            for i in 0..xs.len() - 1 {
                wind += xs[i].1;
                if wind != 0 {
                    add_span(row, xs[i].0, xs[i + 1].0, amt, x0, x1);
                }
            }
        }
    }
    Some((x0, y0, x1, y1))
}

fn add_span(row: &mut [f32], a: f32, b: f32, amt: f32, lo: usize, hi: usize) {
    let a = a.max(lo as f32);
    let b = b.min(hi as f32);
    if !(b > a) {
        return;
    }
    let i0 = a.floor() as usize;
    let i1 = (b.ceil() as usize).min(hi);
    if i1 <= i0 || i0 >= hi {
        return;
    }
    if i1 - i0 == 1 {
        row[i0] += (b - a) * amt;
        return;
    }
    row[i0] += ((i0 + 1) as f32 - a) * amt;
    for c in &mut row[i0 + 1..i1 - 1] {
        *c += amt;
    }
    row[i1 - 1] += (b - (i1 - 1) as f32) * amt;
}

/// Source-over in straight (non-premultiplied) alpha.
fn blend(px: &mut [u8], color: [u8; 4], cov: f32) {
    let sa = color[3] as f32 / 255.0 * cov.clamp(0.0, 1.0);
    if sa <= 0.0 {
        return;
    }
    let da = px[3] as f32 / 255.0;
    let oa = sa + da * (1.0 - sa);
    if oa <= 0.0 {
        return;
    }
    for i in 0..3 {
        let v = (color[i] as f32 * sa + px[i] as f32 * da * (1.0 - sa)) / oa;
        px[i] = v.round().clamp(0.0, 255.0) as u8;
    }
    px[3] = (oa * 255.0).round().clamp(0.0, 255.0) as u8;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Animated, Stroke};

    fn style(kind: ShapeKind, w: f32, h: f32) -> ShapeStyle {
        ShapeStyle {
            w: Animated::new(w as f64),
            h: Animated::new(h as f64),
            stroke: [0, 0, 0, 0],
            ..ShapeStyle::new(kind)
        }
    }
    fn px(f: &Frame, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * f.width + x) * 4) as usize;
        [f.rgba[i], f.rgba[i + 1], f.rgba[i + 2], f.rgba[i + 3]]
    }

    #[test]
    fn rect_fills_its_tight_layer() {
        let mut r = ShapeRasterizer::new();
        let s = style(ShapeKind::Rect, 50.0, 30.0);
        assert_eq!(ShapeRasterizer::size(&s, 0.0), (100.0, 60.0));
        let f = r.render(&s, 1.0, 0.0);
        assert_eq!((f.width, f.height), (100, 60));
        assert_eq!(px(&f, 50, 30), [255, 255, 255, 255], "middle must be opaque fill");
        assert_eq!(px(&f, 0, 0), [255, 255, 255, 255], "a tight rect covers its whole layer");
    }

    #[test]
    fn ellipse_is_transparent_outside() {
        let mut r = ShapeRasterizer::new();
        let f = r.render(&style(ShapeKind::Ellipse, 50.0, 30.0), 1.0, 0.0);
        assert_eq!((f.width, f.height), (100, 60));
        assert_eq!(px(&f, 50, 30)[3], 255, "centre filled");
        assert_eq!(px(&f, 0, 0)[3], 0, "corner outside the ellipse");
        assert_eq!(px(&f, 99, 59)[3], 0, "corner outside the ellipse");
        // the extreme points of the ellipse are covered
        assert!(px(&f, 50, 1)[3] > 100, "top of the ellipse: {:?}", px(&f, 50, 1));
    }

    #[test]
    fn rounded_rect_clears_its_corners() {
        let mut r = ShapeRasterizer::new();
        let mut s = style(ShapeKind::Rect, 50.0, 30.0);
        s.corner = 20.0;
        let f = r.render(&s, 1.0, 0.0);
        assert_eq!(px(&f, 1, 1)[3], 0, "rounded corner must be transparent");
        assert_eq!(px(&f, 50, 30)[3], 255);
    }

    #[test]
    fn stroke_only_shape_is_hollow() {
        let mut r = ShapeRasterizer::new();
        let mut s = style(ShapeKind::Rect, 50.0, 30.0);
        s.fill = [0, 0, 0, 0];
        s.stroke = [255, 0, 0, 255];
        s.stroke_width = 6.0;
        // the layer grows by the stroke width (half sticks out on each side)
        assert_eq!(ShapeRasterizer::size(&s, 0.0), (106.0, 66.0));
        let f = r.render(&s, 1.0, 0.0);
        assert_eq!(px(&f, 53, 33)[3], 0, "centre must be hollow");
        assert_eq!(px(&f, 53, 1), [255, 0, 0, 255], "top edge is stroked");
        assert_eq!(px(&f, 1, 33), [255, 0, 0, 255], "left edge is stroked");
    }

    #[test]
    fn scale_changes_the_pixel_size_only() {
        let mut r = ShapeRasterizer::new();
        let s = style(ShapeKind::Rect, 50.0, 30.0);
        let f = r.render(&s, 2.0, 0.0);
        assert_eq!((f.width, f.height), (200, 120));
        assert_eq!(px(&f, 100, 60)[3], 255);
    }

    #[test]
    fn draw_reveals_progressively() {
        let mut r = ShapeRasterizer::new();
        let mut s = style(ShapeKind::Draw, 0.0, 0.0);
        s.strokes =
            vec![Stroke { color: [0, 255, 0, 255], width: 8.0, points: vec![(-100.0, 0.0, 0.0), (100.0, 0.0, 1.0)] }];
        s.draw_rate = 1.0;
        assert_eq!(ShapeRasterizer::size(&s, 0.0), (208.0, 8.0));
        let mid = r.render(&s, 1.0, 0.5);
        let cy = mid.height / 2;
        assert!(px(&mid, 20, cy)[3] > 200, "start of the stroke is drawn: {:?}", px(&mid, 20, cy));
        assert!(px(&mid, 104, cy)[3] > 200, "halfway point is drawn: {:?}", px(&mid, 104, cy));
        assert_eq!(px(&mid, 180, cy)[3], 0, "the tail is not revealed yet");
        // draw_rate 0 = the whole sketch at once, at any time
        s.draw_rate = 0.0;
        let all = r.render(&s, 1.0, 0.0);
        assert!(px(&all, 180, cy)[3] > 200, "rate 0 draws everything: {:?}", px(&all, 180, cy));
        assert!(px(&all, 20, cy)[3] > 200);
    }

    #[test]
    fn draw_page_paints_the_background() {
        let mut r = ShapeRasterizer::new();
        let mut s = style(ShapeKind::Draw, 60.0, 40.0);
        s.page = [10, 20, 30, 255];
        s.strokes = vec![Stroke { color: [255, 0, 0, 255], width: 4.0, points: vec![(0.0, 0.0, 0.0)] }];
        let f = r.render(&s, 1.0, 0.0);
        assert_eq!((f.width, f.height), (120, 80));
        assert_eq!(px(&f, 2, 2), [10, 20, 30, 255], "page fills the layer");
        assert_eq!(px(&f, 60, 40), [255, 0, 0, 255], "the dot sits on the page");
    }

    #[test]
    fn line_and_arrow_are_drawn() {
        let mut r = ShapeRasterizer::new();
        let mut s = style(ShapeKind::Line, 40.0, 0.0);
        s.stroke = [255, 255, 255, 255];
        s.stroke_width = 4.0;
        let f = r.render(&s, 1.0, 0.0);
        let (cx, cy) = (f.width / 2, f.height / 2);
        assert!(px(&f, cx, cy)[3] > 200, "the line crosses the centre");
        let mut a = style(ShapeKind::Arrow, 40.0, 0.0);
        a.stroke = [255, 255, 255, 255];
        a.stroke_width = 4.0;
        a.corner = 16.0;
        // head is 16 px long, so the (symmetric) layer is padded by it
        assert_eq!(ShapeRasterizer::size(&a, 0.0), (96.0, 16.0));
        let g = r.render(&a, 1.0, 0.0);
        assert!(px(&g, 84, 8)[3] > 200, "near the tip: {:?}", px(&g, 84, 8));
        assert!(px(&g, 76, 4)[3] > 200, "the head flares wider than the shaft: {:?}", px(&g, 76, 4));
        assert_eq!(px(&g, 20, 4)[3], 0, "the shaft is thin away from the head");
    }

    #[test]
    fn ngon_vertex_counts() {
        assert_eq!(ngon(6, 10.0, 10.0, false).len(), 6);
        assert_eq!(ngon(3, 10.0, 10.0, false).len(), 3);
        assert_eq!(ngon(5, 10.0, 10.0, true).len(), 10, "a star has two vertices per side");
        assert_eq!(ngon(8, 10.0, 10.0, true).len(), 16);
        assert_eq!(ngon(1, 10.0, 10.0, false).len(), 3, "sides are clamped to a triangle");
        assert_eq!(ngon(999, 10.0, 10.0, false).len(), 64, "and to 64");
        // first vertex points up (negative y)
        let p = ngon(5, 10.0, 10.0, false)[0];
        assert!(p[1] < -9.0, "{p:?}");
    }

    #[test]
    fn star_and_polygon_render_inside_the_layer() {
        let mut r = ShapeRasterizer::new();
        for kind in [ShapeKind::Star, ShapeKind::Polygon, ShapeKind::Triangle] {
            let f = r.render(&style(kind, 50.0, 50.0), 1.0, 0.0);
            assert_eq!((f.width, f.height), (100, 100), "{kind:?}");
            assert_eq!(px(&f, 50, 55)[3], 255, "{kind:?} centre filled");
            assert_eq!(px(&f, 2, 2)[3], 0, "{kind:?} top-left corner is empty");
        }
    }

    #[test]
    fn identical_requests_hit_the_cache() {
        let mut r = ShapeRasterizer::new();
        let s = style(ShapeKind::Ellipse, 40.0, 40.0);
        let a = r.render(&s, 1.0, 0.0);
        let b = r.render(&s, 1.0, 0.0);
        assert!(Arc::ptr_eq(&a, &b), "cache hit expected");
        let c = r.render(&s, 2.0, 0.0);
        assert!(!Arc::ptr_eq(&a, &c), "a different scale is a different layer");
    }

    #[test]
    fn draw_reveal_buckets_share_cache_entries() {
        let mut r = ShapeRasterizer::new();
        let mut s = style(ShapeKind::Draw, 0.0, 0.0);
        s.strokes =
            vec![Stroke { color: [255, 255, 255, 255], width: 4.0, points: vec![(0.0, 0.0, 0.0), (50.0, 0.0, 2.0)] }];
        let a = r.render(&s, 1.0, 0.5);
        let b = r.render(&s, 1.0, 0.51); // same 1/30 s bucket
        assert!(Arc::ptr_eq(&a, &b), "reveal is bucketed for the cache");
        let c = r.render(&s, 1.0, 1.0);
        assert!(!Arc::ptr_eq(&a, &c));
        // past the end everything collapses onto one fully-revealed entry
        let d = r.render(&s, 1.0, 9.0);
        let e = r.render(&s, 1.0, 99.0);
        assert!(Arc::ptr_eq(&d, &e), "finished drawings share one entry");
    }

    #[test]
    fn degenerate_styles_never_panic() {
        let mut r = ShapeRasterizer::new();
        for kind in ShapeKind::ALL {
            let mut s = style(kind, 0.0, 0.0);
            s.stroke_width = 0.0;
            s.sides = 0;
            let f = r.render(&s, 0.0, -5.0);
            assert!(f.width >= 1 && f.height >= 1, "{kind:?}");
            let mut odd = style(kind, f32::INFINITY, f32::NAN);
            odd.stroke = [255, 255, 255, 255];
            odd.corner = -3.0;
            let g = r.render(&odd, 3.0, f64::NAN);
            assert!(g.width >= 1 && g.height >= 1, "{kind:?}");
        }
    }

    #[test]
    fn oversized_layers_are_clamped() {
        let mut r = ShapeRasterizer::new();
        let s = style(ShapeKind::Rect, 100_000.0, 1.0);
        // half-size is capped at 20000 project px, then the layer is capped at MAX_DIM
        assert_eq!(ShapeRasterizer::size(&s, 0.0), (40_000.0, 2.0));
        let f = r.render(&s, 1.0, 0.0);
        assert_eq!(f.width, MAX_DIM as u32);
        assert!(f.height >= 1 && f.height < 4);
    }

    #[test]
    fn revealed_cuts_the_last_segment() {
        let pts = [(0.0f32, 0.0f32, 0.0f32), (100.0, 0.0, 1.0)];
        let mut out = Vec::new();
        revealed(&pts, 0.5, &mut out);
        assert_eq!(out, vec![[0.0, 0.0], [50.0, 0.0]]);
        revealed(&pts, f32::INFINITY, &mut out);
        assert_eq!(out, vec![[0.0, 0.0], [100.0, 0.0]]);
        revealed(&pts, -1.0, &mut out);
        assert!(out.is_empty(), "nothing drawn before the first point");
    }
}
