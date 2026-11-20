//! ---- ws:parity-360 ----
//! 360° (equirectangular) video: the sphere maths the library's 360 shaders use, and 360 Stabilize.
//!
//! Convention, shared with `library/effects/360-*.glsl`: an equirect picture's `u = 0.5` looks forward
//! and longitude grows to the right; `v = 0` is straight up. A direction is (x right, y up, z forward).
//! The 360 Transform shader turns each output direction by `turn(d, yaw, pitch, roll)` and reads the
//! source there, so +yaw looks right and +pitch looks up.
//!
//! 360 Stabilize: between each pair of frames, ~80 patches spread over the sphere (latitudes within
//! ±40°, where equirect pixels are least stretched) are matched with zero-mean NCC, coarse then fine,
//! longitude wrapping round. Each match is a small move of a point on the sphere, and the rotation
//! that explains them all is one 3x3 least-squares solve (`fit_rotation`), refit without outliers
//! (parallax from things close to the camera). The rotations chain into the camera's path; the path
//! smoothed over `smooth` seconds (or not at all: locked to the first frame) is what is kept, and the
//! difference becomes yaw / pitch / roll keys on a 360 Transform effect. Everything here runs on a job
//! thread; the UI only writes the keys.

use crate::engine::export::Progress;
use crate::media::{Backend, DecoderPool, Frame};
use std::f64::consts::PI;

pub type V3 = [f64; 3];

/// The 360 Transform shader, compiled in so 360 Stabilize works before the library is downloaded.
pub const TRANSFORM_SHADER: &str = include_str!("../../library/effects/360-transform.glsl");
/// Its `@name`, which is how a clip's existing 360 Transform effect is found.
pub const TRANSFORM_NAME: &str = "360 Transform";

/// The analysis size: an equirect frame is decoded at this size whatever its own (2:1).
const AW: u32 = 512;
const AH: u32 = 256;

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scale(a: V3, s: f64) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn norm(a: V3) -> f64 {
    dot(a, a).sqrt()
}

/// The direction an equirect position (0..1 each, v = 0 at the top) looks along.
pub fn eq_dir(u: f64, v: f64) -> V3 {
    let lon = (u - 0.5) * 2.0 * PI;
    let lat = (0.5 - v) * PI;
    [lat.cos() * lon.sin(), lat.sin(), lat.cos() * lon.cos()]
}

/// The equirect position (u in 0..1, v in 0..1) of a direction; the inverse of `eq_dir`.
pub fn dir_eq(d: V3) -> (f64, f64) {
    let n = norm(d).max(1e-12);
    let lon = d[0].atan2(d[2]);
    let lat = (d[1] / n).clamp(-1.0, 1.0).asin();
    ((lon / (2.0 * PI) + 0.5).rem_euclid(1.0), 0.5 - lat / PI)
}

/// `c` turned by roll about z, then pitch about x (+ tips forward up), then yaw about y (+ turns
/// forward right): exactly the 360 shaders' `turn`. Angles in radians. (The app only needs its
/// inverse, `Quat::ypr`; this is the reference the tests hold the quaternions and the shaders to.)
#[cfg(test)]
pub fn turn(c: V3, [yaw, pitch, roll]: V3) -> V3 {
    let (sr, cr) = roll.sin_cos();
    let c = [cr * c[0] - sr * c[1], sr * c[0] + cr * c[1], c[2]];
    let (sp, cp) = pitch.sin_cos();
    let c = [c[0], cp * c[1] + sp * c[2], -sp * c[1] + cp * c[2]];
    let (sy, cy) = yaw.sin_cos();
    [cy * c[0] + sy * c[2], c[1], -sy * c[0] + cy * c[2]]
}

/// A rotation as a unit quaternion.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quat {
    pub w: f64,
    pub v: V3,
}

impl Quat {
    pub const IDENTITY: Quat = Quat { w: 1.0, v: [0.0; 3] };

    /// The rotation by |r| radians about r (right-handed).
    pub fn from_rotvec(r: V3) -> Quat {
        let a = norm(r);
        if a < 1e-12 {
            return Quat { w: 1.0, v: scale(r, 0.5) }.normalized();
        }
        Quat { w: (a / 2.0).cos(), v: scale(r, (a / 2.0).sin() / a) }
    }
    fn axis(axis: V3, a: f64) -> Quat {
        Self::from_rotvec(scale(axis, a))
    }
    /// The rotation `turn` applies for these angles (radians).
    pub fn from_ypr([yaw, pitch, roll]: V3) -> Quat {
        // `turn`'s pitch step is a right-handed rotation about x by -pitch
        Self::axis([0.0, 1.0, 0.0], yaw).mul(Self::axis([1.0, 0.0, 0.0], -pitch)).mul(Self::axis([0.0, 0.0, 1.0], roll))
    }
    /// `self ∘ o`: o first, then self.
    pub fn mul(self, o: Quat) -> Quat {
        Quat {
            w: self.w * o.w - dot(self.v, o.v),
            v: add(add(scale(o.v, self.w), scale(self.v, o.w)), cross(self.v, o.v)),
        }
    }
    /// The axis x angle of this rotation (the inverse of `from_rotvec`).
    pub fn rotvec(self) -> V3 {
        let q = if self.w < 0.0 { Quat { w: -self.w, v: scale(self.v, -1.0) } } else { self };
        let s = norm(q.v);
        if s < 1e-12 {
            return scale(q.v, 2.0);
        }
        scale(q.v, 2.0 * s.atan2(q.w) / s)
    }
    pub fn conj(self) -> Quat {
        Quat { w: self.w, v: scale(self.v, -1.0) }
    }
    pub fn normalized(self) -> Quat {
        let n = (self.w * self.w + dot(self.v, self.v)).sqrt().max(1e-12);
        Quat { w: self.w / n, v: scale(self.v, 1.0 / n) }
    }
    pub fn rotate(self, p: V3) -> V3 {
        let t = scale(cross(self.v, p), 2.0);
        add(add(p, scale(t, self.w)), cross(self.v, t))
    }
    /// Yaw, pitch and roll (radians) such that `from_ypr` gives this rotation back.
    pub fn ypr(self) -> V3 {
        let f = self.rotate([0.0, 0.0, 1.0]);
        let ex = self.rotate([1.0, 0.0, 0.0]);
        let ey = self.rotate([0.0, 1.0, 0.0]);
        [f[0].atan2(f[2]), f[1].clamp(-1.0, 1.0).asin(), ex[1].atan2(ey[1])]
    }
}

/// Luma of an equirect frame, for matching. Reads wrap round in x (longitude) and clamp in y.
#[derive(Clone, Debug, Default)]
pub struct Plane {
    pub w: usize,
    pub h: usize,
    pub px: Vec<f32>,
}

impl Plane {
    pub fn from_frame(f: &Frame) -> Plane {
        let (w, h) = (f.width as usize, f.height as usize);
        let stride = f.stride();
        let mut px = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let p = &f.rgba[y * stride + x * 4..];
                px.push(p[0] as f32 * 0.299 + p[1] as f32 * 0.587 + p[2] as f32 * 0.114);
            }
        }
        Plane { w, h, px }
    }
    fn at(&self, x: i32, y: i32) -> f32 {
        let x = x.rem_euclid(self.w as i32) as usize;
        let y = y.clamp(0, self.h as i32 - 1) as usize;
        self.px[y * self.w + x]
    }
    /// Half the size, 2x2 averaged.
    pub fn half(&self) -> Plane {
        let (w, h) = (self.w / 2, self.h / 2);
        let mut px = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let (x2, y2) = (2 * x as i32, 2 * y as i32);
                px.push((self.at(x2, y2) + self.at(x2 + 1, y2) + self.at(x2, y2 + 1) + self.at(x2 + 1, y2 + 1)) * 0.25);
            }
        }
        Plane { w, h, px }
    }
}

const HALF: i32 = 6; // 13x13 patches

/// Where the `HALF`-patch of `a` at (x, y) best matches in `b` within `r` px of (px, py): the sub-pixel
/// position. None when the patch is flat (sky, a wall) or nothing correlates well.
fn match_patch(a: &Plane, b: &Plane, (x, y): (i32, i32), (px, py): (i32, i32), r: i32) -> Option<(f64, f64)> {
    let side = (2 * HALF + 1) as usize;
    let n = (side * side) as f32;
    let mut tpl = Vec::with_capacity(side * side);
    for j in -HALF..=HALF {
        for i in -HALF..=HALF {
            tpl.push(a.at(x + i, y + j));
        }
    }
    let tmean = tpl.iter().sum::<f32>() / n;
    tpl.iter_mut().for_each(|v| *v -= tmean);
    let tvar: f32 = tpl.iter().map(|v| v * v).sum();
    if tvar < n * 16.0 {
        return None; // < 4 levels of contrast: nothing to lock onto
    }
    // an edge or stripes (a horizon, a railing) only pins the move across them: skip the patch
    let (mut xx, mut xy, mut yy) = (0.0f32, 0.0f32, 0.0f32);
    for j in -HALF..=HALF {
        for i in -HALF..=HALF {
            let gx = a.at(x + i + 1, y + j) - a.at(x + i - 1, y + j);
            let gy = a.at(x + i, y + j + 1) - a.at(x + i, y + j - 1);
            (xx, xy, yy) = (xx + gx * gx, xy + gx * gy, yy + gy * gy);
        }
    }
    let (tr, det) = (xx + yy, xx * yy - xy * xy);
    let lmin = tr / 2.0 - (tr * tr / 4.0 - det).max(0.0).sqrt();
    if lmin < tr * 0.05 {
        return None;
    }
    let sz = (2 * r + 1) as usize;
    let mut scores = vec![f32::MIN; sz * sz];
    let mut best = (f32::MIN, 0usize, 0usize);
    for (sj, dy) in (-r..=r).enumerate() {
        for (si, dx) in (-r..=r).enumerate() {
            let (ox, oy) = (px + dx, py + dy);
            let (mut s, mut ss, mut d) = (0.0f32, 0.0f32, 0.0f32);
            let mut k = 0;
            for j in -HALF..=HALF {
                for i in -HALF..=HALF {
                    let v = b.at(ox + i, oy + j);
                    s += v;
                    ss += v * v;
                    d += v * tpl[k];
                    k += 1;
                }
            }
            let var = ss - s * s / n;
            if var <= 1e-3 {
                continue;
            }
            let sc = d / (tvar * var).sqrt();
            scores[sj * sz + si] = sc;
            if sc > best.0 {
                best = (sc, si, sj);
            }
        }
    }
    let (sc, si, sj) = best;
    if sc < 0.7 {
        return None;
    }
    // a parabola through the peak and its neighbours, per axis: the start for the sub-pixel refinement
    let sub = |m: f32, c: f32, p: f32| {
        let den = m - 2.0 * c + p;
        if m == f32::MIN || p == f32::MIN || den >= -1e-6 {
            0.0
        } else {
            (0.5 * (m - p) / den).clamp(-0.5, 0.5) as f64
        }
    };
    let ox = if si > 0 && si + 1 < sz { sub(scores[sj * sz + si - 1], sc, scores[sj * sz + si + 1]) } else { 0.0 };
    let oy = if sj > 0 && sj + 1 < sz { sub(scores[(sj - 1) * sz + si], sc, scores[(sj + 1) * sz + si]) } else { 0.0 };
    let (bx, by) = ((px - r + si as i32) as f64, (py - r + sj as i32) as f64);
    Some(refine(b, &tpl, (bx, by), (bx + ox, by + oy)))
}

/// Bilinear read, wrapping in x like `Plane::at`.
fn bilinear(p: &Plane, x: f64, y: f64) -> f32 {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = ((x - x0) as f32, (y - y0) as f32);
    let (x0, y0) = (x0 as i32, y0 as i32);
    let top = p.at(x0, y0) * (1.0 - fx) + p.at(x0 + 1, y0) * fx;
    let bot = p.at(x0, y0 + 1) * (1.0 - fx) + p.at(x0 + 1, y0 + 1) * fx;
    top * (1.0 - fy) + bot * fy
}

/// Lucas-Kanade on the zero-mean template `tpl` (centred at `start` in `b`): a few Gauss-Newton steps
/// to the sub-pixel position, kept within a pixel of the integer peak `peak` (else `start` stands).
fn refine(b: &Plane, tpl: &[f32], peak: (f64, f64), start: (f64, f64)) -> (f64, f64) {
    let mut s = start;
    let side = (2 * HALF + 1) as usize;
    let mut win = vec![0.0f32; side * side];
    for _ in 0..5 {
        let mut k = 0;
        for j in -HALF..=HALF {
            for i in -HALF..=HALF {
                win[k] = bilinear(b, s.0 + i as f64, s.1 + j as f64);
                k += 1;
            }
        }
        let mean = win.iter().sum::<f32>() / win.len() as f32;
        let (mut hxx, mut hxy, mut hyy, mut gx_e, mut gy_e) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
        let mut k = 0;
        for j in -HALF..=HALF {
            for i in -HALF..=HALF {
                let (x, y) = (s.0 + i as f64, s.1 + j as f64);
                let gx = ((bilinear(b, x + 0.5, y) - bilinear(b, x - 0.5, y)) as f64).clamp(-255.0, 255.0);
                let gy = ((bilinear(b, x, y + 0.5) - bilinear(b, x, y - 0.5)) as f64).clamp(-255.0, 255.0);
                let e = ((win[k] - mean) - tpl[k]) as f64;
                hxx += gx * gx;
                hxy += gx * gy;
                hyy += gy * gy;
                gx_e += gx * e;
                gy_e += gy * e;
                k += 1;
            }
        }
        let det = hxx * hyy - hxy * hxy;
        if det.abs() < 1e-6 {
            break;
        }
        let dx = -(hyy * gx_e - hxy * gy_e) / det;
        let dy = -(hxx * gy_e - hxy * gx_e) / det;
        s = (s.0 + dx, s.1 + dy);
        if (s.0 - peak.0).abs() > 1.0 || (s.1 - peak.1).abs() > 1.0 {
            return start; // walked off: the parabola's answer is the better one
        }
        if dx * dx + dy * dy < 1e-4 {
            break;
        }
    }
    s
}

/// The sample directions: 16 longitudes x 5 latitudes (±40°).
fn samples() -> Vec<V3> {
    let mut v = Vec::with_capacity(80);
    for lat in [-40.0f64, -20.0, 0.0, 20.0, 40.0] {
        for k in 0..16 {
            let lon = (k as f64 + if lat == 0.0 { 0.0 } else { 0.5 }) * 22.5 - 180.0;
            let (lat, lon) = (lat.to_radians(), lon.to_radians());
            v.push([lat.cos() * lon.sin(), lat.sin(), lat.cos() * lon.cos()]);
        }
    }
    v
}

/// The small rotation r (axis x angle) that best moves each direction `d` to `d + move`
/// (`r x d ≈ move`): solves `Σ(I - d dᵀ) r = Σ d x move`. None with too few points.
pub fn fit_rotation(pairs: &[(V3, V3)]) -> Option<V3> {
    if pairs.len() < 6 {
        return None;
    }
    let mut m = [[0.0f64; 3]; 3];
    let mut b = [0.0f64; 3];
    for (d, mv) in pairs {
        for (i, row) in m.iter_mut().enumerate() {
            for (j, e) in row.iter_mut().enumerate() {
                *e += if i == j { 1.0 } else { 0.0 } - d[i] * d[j];
            }
        }
        b = add(b, cross(*d, *mv));
    }
    solve3(m, b)
}

/// Cramer's rule; None when the system is (near) singular.
fn solve3(m: [[f64; 3]; 3], b: V3) -> Option<V3> {
    let det = |m: [[f64; 3]; 3]| dot(m[0], cross(m[1], m[2]));
    let d = det(m);
    if d.abs() < 1e-9 {
        return None;
    }
    let mut out = [0.0; 3];
    for (k, o) in out.iter_mut().enumerate() {
        let mut mk = m;
        for (r, row) in mk.iter_mut().enumerate() {
            row[k] = b[r];
        }
        *o = det(mk) / d;
    }
    Some(out)
}

/// The rotation r with `b(rot(r) d) ≈ a(d)`: how far the picture turned from `a` to `b`
/// (`a` and `b` the same size). None when too little matched (a flat or a cut frame).
pub fn estimate(a: &Plane, b: &Plane) -> Option<V3> {
    let pts = samples();
    let pos = |p: &Plane, d: V3| {
        let (u, v) = dir_eq(d);
        (u * p.w as f64 - 0.5, v * p.h as f64 - 0.5)
    };
    let back = |p: &Plane, (x, y): (f64, f64)| eq_dir((x + 0.5) / p.w as f64, (y + 0.5) / p.h as f64);
    let moves = |a: &Plane, b: &Plane, guess: V3, r: i32| -> Vec<(V3, V3)> {
        let q = Quat::from_rotvec(guess);
        let mut out = Vec::with_capacity(pts.len());
        for &d in &pts {
            let (x, y) = pos(a, d);
            let (gx, gy) = pos(b, q.rotate(d));
            // the patch centre is the pixel nearest the sample; the move is measured from it
            let (xi, yi) = (x.round() as i32, y.round() as i32);
            let (gx, gy) = (xi as f64 + wrap_dx(gx - x, a.w as f64), gy - y + yi as f64);
            if let Some(hit) = match_patch(a, b, (xi, yi), (gx.round() as i32, gy.round() as i32), r) {
                let d0 = back(a, (xi as f64, yi as f64));
                out.push((d0, sub(back(b, hit), d0)));
            }
        }
        out
    };
    // coarse: half size, ±10 px (±14°); fine: full size, ±3 px around the coarse answer
    let (ah, bh) = (a.half(), b.half());
    let coarse = robust_fit(&moves(&ah, &bh, [0.0; 3], 10))?;
    let fine = moves(a, b, coarse, 3);
    let Some(r) = robust_fit(&fine) else { return Some(coarse) };
    // `fit_rotation` is first order (good to ~1 % of the angle); refit what is left after turning by r
    // so a fast pan (10°+ a frame) still comes out to a fraction of a pixel
    let q = Quat::from_rotvec(r);
    let rest: Vec<(V3, V3)> = fine.iter().map(|(d, mv)| (q.rotate(*d), sub(add(*d, *mv), q.rotate(*d)))).collect();
    Some(robust_fit(&rest).map_or(r, |e| Quat::from_rotvec(e).mul(q).rotvec()))
}

/// x distance on a wrapping row, in -w/2..w/2.
fn wrap_dx(dx: f64, w: f64) -> f64 {
    (dx + w / 2.0).rem_euclid(w) - w / 2.0
}

/// `fit_rotation`, then again without the points it explains worst (a moving subject, parallax).
fn robust_fit(pairs: &[(V3, V3)]) -> Option<V3> {
    let r = fit_rotation(pairs)?;
    let mut res: Vec<f64> = pairs.iter().map(|(d, mv)| norm(sub(cross(r, *d), *mv))).collect();
    let mut sorted = res.clone();
    sorted.sort_by(f64::total_cmp);
    let cut = sorted[sorted.len() / 2] * 2.5 + 2e-3;
    let keep: Vec<(V3, V3)> = pairs.iter().zip(res.iter_mut()).filter(|(_, e)| **e <= cut).map(|(p, _)| *p).collect();
    fit_rotation(&keep).or(Some(r))
}

/// What to keep steady.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StabilizeOptions {
    /// Seconds of camera motion to average over: the slow turns are kept and the shake removed.
    /// 0 locks the view to the first frame's.
    pub smooth: f64,
    /// Correct yaw, pitch and roll (each can be left alone, e.g. keep a pan's yaw but level the horizon).
    pub axes: [bool; 3],
}

impl Default for StabilizeOptions {
    fn default() -> Self {
        Self { smooth: 1.0, axes: [true; 3] }
    }
}

/// The 360 Transform angles (degrees) per frame that undo the camera's shake, from the frame-to-frame
/// rotations `deltas` (`estimate`; None = no reading, taken as no turn). One more value than `deltas`.
pub fn correction(deltas: &[Option<V3>], fps: f64, o: StabilizeOptions) -> Vec<V3> {
    // the camera's path: Q_t = rot(r_t) Q_t-1, so frame t's picture is frame 0's turned by Q_t
    let mut path = Vec::with_capacity(deltas.len() + 1);
    path.push(Quat::IDENTITY);
    for d in deltas {
        let prev = *path.last().unwrap_or(&Quat::IDENTITY);
        path.push(Quat::from_rotvec(d.unwrap_or([0.0; 3])).mul(prev).normalized());
    }
    let sigma = o.smooth.max(0.0) * fps.max(1.0) / 2.0;
    let reach = (sigma * 3.0).ceil() as isize;
    let n = path.len() as isize;
    (0..n)
        .map(|t| {
            let q = path[t as usize];
            // the smoothed path near t (a weighted quaternion mean, signs aligned with q's)
            let c = if sigma < 1e-3 {
                Quat::IDENTITY
            } else {
                let (mut w, mut v, mut sum) = (0.0, [0.0; 3], 0.0);
                for k in (t - reach).max(0)..=(t + reach).min(n - 1) {
                    let p = path[k as usize];
                    let g = (-((k - t) as f64).powi(2) / (2.0 * sigma * sigma)).exp();
                    let s = if p.w * q.w + dot(p.v, q.v) < 0.0 { -g } else { g };
                    w += p.w * s;
                    v = add(v, scale(p.v, s));
                    sum += g;
                }
                Quat { w: w / sum, v: scale(v, 1.0 / sum) }.normalized()
            };
            // show frame t turned by Q_t C_t⁻¹: the shake (Q against its smoothed self) taken out
            let ypr = q.mul(c.conj()).normalized().ypr();
            let mut out = [0.0; 3];
            for i in 0..3 {
                if o.axes[i] {
                    out[i] = ypr[i].to_degrees();
                }
            }
            out
        })
        .collect()
}

/// Keys (t, v) worth keeping: drops each one a straight line between its neighbours already gives
/// within `tol`, so a mostly-still clip does not get a key on every frame.
pub fn thin_keys(keys: &[(f64, f64)], tol: f64) -> Vec<(f64, f64)> {
    if keys.len() <= 2 {
        return keys.to_vec();
    }
    let mut out = vec![keys[0]];
    for i in 1..keys.len() - 1 {
        let (a, b, c) = (*out.last().unwrap_or(&keys[0]), keys[i], keys[i + 1]);
        let f = if c.0 > a.0 { (b.0 - a.0) / (c.0 - a.0) } else { 0.0 };
        if (a.1 + (c.1 - a.1) * f - b.1).abs() > tol {
            out.push(b);
        }
    }
    out.push(keys[keys.len() - 1]);
    out
}

/// Decode the clip's frames at `times` (source seconds) and measure the turn between each pair.
/// Runs on the job thread; checks `prog` for a cancel every frame.
pub fn analyse(path: &str, times: &[f64], backend: Backend, prog: &Progress) -> Result<Vec<Option<V3>>, String> {
    crate::media::bg::lower_thread();
    let mut pool = DecoderPool::new(backend);
    let mut frame = Frame::default();
    let mut prev: Option<Plane> = None;
    let mut out = Vec::with_capacity(times.len());
    let total = times.len().max(1) as f32;
    for (i, &st) in times.iter().enumerate() {
        if prog.is_cancelled() {
            return Err(crate::engine::export::CANCELLED.into());
        }
        let dec = pool.video(path).ok_or("the clip's footage could not be opened")?;
        if !dec.frame_at(st, AW, AH, &mut frame) {
            if i == 0 {
                return Err("no frames could be decoded".into());
            }
            break;
        }
        frame.make_rgba();
        let cur = Plane::from_frame(&frame);
        if let Some(p) = &prev {
            out.push(estimate(p, &cur));
        }
        prev = Some(cur);
        prog.set((i + 1) as f32 / total * 0.99, "Analysing 360° motion…");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A textured sphere: three octaves of 3D value noise, so every patch has detail in both directions
    /// (a sum of sine waves gives stripes a patch can slide along).
    pub fn sphere(d: V3) -> f32 {
        fn hash(x: i64, y: i64, z: i64) -> f64 {
            let mut h = (x.wrapping_mul(73_856_093) ^ y.wrapping_mul(19_349_663) ^ z.wrapping_mul(83_492_791)) as u64;
            h ^= h >> 13;
            h = h.wrapping_mul(0x5bd1_e995);
            h ^= h >> 15;
            (h & 0xffff) as f64 / 65535.0
        }
        fn noise(p: V3) -> f64 {
            let f = p.map(f64::floor);
            let t = [p[0] - f[0], p[1] - f[1], p[2] - f[2]].map(|t| t * t * (3.0 - 2.0 * t));
            let [x, y, z] = f.map(|v| v as i64);
            let mut v = 0.0;
            for (dx, dy, dz) in (0..8).map(|k| (k & 1, (k >> 1) & 1, k >> 2)) {
                let w = [(dx, 0), (dy, 1), (dz, 2)]
                    .iter()
                    .map(|&(o, i)| if o == 1 { t[i] } else { 1.0 - t[i] })
                    .product::<f64>();
                v += w * hash(x + dx, y + dy, z + dz);
            }
            v
        }
        let v = noise(scale(d, 9.0)) * 0.5 + noise(scale(d, 21.0)) * 0.3 + noise(scale(d, 43.0)) * 0.2;
        (v * 255.0) as f32
    }

    /// The equirect picture of `sphere` seen after the camera turned by q (frame t of a video whose
    /// frame 0 is `render(IDENTITY)`): pixel d shows the sphere at q⁻¹ d.
    pub fn render(q: Quat, w: usize, h: usize) -> Plane {
        let inv = q.conj();
        let mut px = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let d = eq_dir((x as f64 + 0.5) / w as f64, (y as f64 + 0.5) / h as f64);
                px.push(sphere(inv.rotate(d)));
            }
        }
        Plane { w, h, px }
    }

    fn close(a: V3, b: V3, tol: f64) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() <= tol)
    }

    #[test]
    fn equirect_positions_and_directions_round_trip() {
        assert!(close(eq_dir(0.5, 0.5), [0.0, 0.0, 1.0], 1e-12), "the middle looks forward");
        assert!(close(eq_dir(0.75, 0.5), [1.0, 0.0, 0.0], 1e-12), "right of the middle looks right");
        assert!(eq_dir(0.3, 0.0)[1] > 0.999, "the top row looks up");
        for (u, v) in [(0.1, 0.2), (0.5, 0.5), (0.9, 0.7), (0.0, 0.5)] {
            let (u2, v2) = dir_eq(eq_dir(u, v));
            assert!((u2 - u).abs() < 1e-9 && (v2 - v).abs() < 1e-9, "{u},{v} -> {u2},{v2}");
        }
    }

    #[test]
    fn turn_matches_the_quaternion_and_its_angles_come_back() {
        let ypr = [0.4, -0.3, 0.2];
        let q = Quat::from_ypr(ypr);
        for c in [[0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.3, 0.5, -0.8]] {
            assert!(close(turn(c, ypr), q.rotate(c), 1e-12));
        }
        assert!(close(q.ypr(), ypr, 1e-12));
        // +yaw looks right, +pitch looks up (what the shader's header promises)
        assert!(turn([0.0, 0.0, 1.0], [0.5, 0.0, 0.0])[0] > 0.0);
        assert!(turn([0.0, 0.0, 1.0], [0.0, 0.5, 0.0])[1] > 0.0);
    }

    #[test]
    fn fit_rotation_recovers_an_exact_small_turn() {
        let r = [0.01, -0.02, 0.015];
        let q = Quat::from_rotvec(r);
        let pairs: Vec<(V3, V3)> = samples().into_iter().map(|d| (d, sub(q.rotate(d), d))).collect();
        let got = fit_rotation(&pairs).unwrap();
        assert!(close(got, r, 2e-4), "{got:?}");
        assert!(fit_rotation(&pairs[..3]).is_none(), "too few points");
    }

    #[test]
    fn estimate_measures_the_turn_between_two_frames() {
        let (w, h) = (AW as usize, AH as usize);
        let a = render(Quat::IDENTITY, w, h);
        // small shakes to an 11° jump; the coarse pass reaches ±14° a frame
        for r in [[0.0, 0.05, 0.0], [0.03, 0.0, 0.0], [0.0, 0.0, -0.04], [0.003, -0.002, 0.001], [0.1, 0.15, -0.08]] {
            let b = render(Quat::from_rotvec(r), w, h);
            let got = estimate(&a, &b).expect("a textured sphere matches");
            // within 0.03° (a pixel of a 512-wide picture is 0.7°)
            assert!(close(got, r, 5e-4), "turned {r:?}, measured {got:?}");
        }
        assert!(close(Quat::from_rotvec([0.3, -0.2, 0.1]).rotvec(), [0.3, -0.2, 0.1], 1e-12));
        let flat = Plane { w, h, px: vec![100.0; w * h] };
        assert!(estimate(&flat, &flat).is_none(), "nothing to match on a flat frame");
    }

    #[test]
    fn locked_correction_undoes_the_camera_path_and_smoothing_keeps_the_pan() {
        // a camera that pans right steadily and shakes in pitch and roll
        let n = 40;
        let fps = 30.0;
        let path: Vec<Quat> = (0..n)
            .map(|t| {
                // frame 0 is the reference the path is measured from: no shake there
                let s = if t == 0 {
                    0.0
                } else if t % 2 == 0 {
                    1.0
                } else {
                    -1.0
                };
                Quat::from_ypr([0.01 * t as f64, 0.02 * s, 0.015 * s])
            })
            .collect();
        let deltas: Vec<Option<V3>> =
            path.windows(2).map(|p| Some(p[1].mul(p[0].conj()).normalized().rotvec())).collect();
        let lock = correction(&deltas, fps, StabilizeOptions { smooth: 0.0, axes: [true; 3] });
        assert_eq!(lock.len(), n);
        for (t, c) in lock.iter().enumerate() {
            // the correction applied to frame t's picture gives frame 0's: Q_t ∘ C⁻¹ with C = I is Q_t
            let want = path[t].ypr().map(f64::to_degrees);
            assert!(close(*c, want, 1e-6), "frame {t}: {c:?} vs {want:?}");
        }
        let smooth = correction(&deltas, fps, StabilizeOptions { smooth: 0.5, axes: [true; 3] });
        // the output shows frame 0 turned by the virtual camera R_t⁻¹ Q_t: it keeps the steady pan and
        // loses the alternating pitch / roll shake
        for t in n / 2 - 5..n / 2 + 5 {
            let r = Quat::from_ypr(smooth[t].map(f64::to_radians));
            let cam = r.conj().mul(path[t]).normalized().ypr();
            assert!((cam[0] - 0.01 * t as f64).abs() < 0.01, "frame {t}: {cam:?}");
            assert!(cam[1].abs() < 0.003 && cam[2].abs() < 0.003, "frame {t}: {cam:?}");
        }
        let level = correction(&deltas, fps, StabilizeOptions { smooth: 0.0, axes: [false, false, true] });
        assert!(level.iter().all(|c| c[0] == 0.0 && c[1] == 0.0));
    }

    #[test]
    fn a_shaky_sequence_is_measured_and_locked_end_to_end() {
        let (w, h) = (AW as usize, AH as usize);
        // a camera wobbling a few degrees on every axis, frame to frame
        let path: Vec<Quat> = (0..10)
            .map(|t| {
                let t = t as f64;
                Quat::from_ypr([0.04 * (t * 1.3).sin(), 0.03 * (t * 2.1).cos() - 0.03, 0.05 * (t * 0.7).sin()])
            })
            .collect();
        let frames: Vec<Plane> = path.iter().map(|q| render(*q, w, h)).collect();
        let deltas: Vec<Option<V3>> = frames.windows(2).map(|f| estimate(&f[0], &f[1])).collect();
        let corr = correction(&deltas, 30.0, StabilizeOptions { smooth: 0.0, axes: [true; 3] });
        for (t, c) in corr.iter().enumerate() {
            // locked to frame 0: the correction is the camera's turn since then
            let want = path[t].mul(path[0].conj()).normalized().ypr().map(f64::to_degrees);
            assert!(close(*c, want, 0.1), "frame {t}: {c:?} vs {want:?}");
        }
    }

    #[test]
    fn thin_keys_drops_what_a_line_already_gives() {
        let line: Vec<(f64, f64)> = (0..10).map(|i| (i as f64, i as f64 * 2.0)).collect();
        assert_eq!(thin_keys(&line, 0.01), vec![(0.0, 0.0), (9.0, 18.0)]);
        let bump = vec![(0.0, 0.0), (1.0, 0.0), (2.0, 5.0), (3.0, 0.0), (4.0, 0.0)];
        assert!(thin_keys(&bump, 0.01).contains(&(2.0, 5.0)));
    }

    #[test]
    fn the_compiled_in_transform_shader_is_the_library_file() {
        let m = crate::engine::shaderlib::parse_meta(TRANSFORM_SHADER);
        assert_eq!(m.name, TRANSFORM_NAME);
        let ids: Vec<&str> = m.params.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["yaw", "pitch", "roll"], "360 Stabilize writes params 0, 1 and 2");
        assert!(crate::engine::shaderlib::lint(TRANSFORM_SHADER, false).is_empty());
    }
}
