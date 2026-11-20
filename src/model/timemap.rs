//! Time remapping: a clip's source time as a function of its own (clip-local) time.
//!
//! A `TimeMap` is a list of keys `(t, s)`: at clip-local second `t` the clip shows source second `s`.
//! Between two keys the source runs at a constant speed (`smooth` off) or eases through a monotone cubic
//! (`smooth` on), so a run of keys gives a speed ramp whose speed changes without a jump. Two keys with
//! the same `s` are a freeze (hold); a falling `s` plays that stretch backwards. Monotone cubic
//! interpolation (harmonic-mean tangents) never overshoots, so a hold stays a hold and a forward
//! stretch never steps back.
//!
//! The first key sits at t = 0 and the last at the clip's duration: changing a segment's length moves
//! the keys after it, so the clip's length follows (the caller ripples the track).
//!
//! Edits that must not change what plays (adding a key, a hold, a rewind, a split or a trim) first pin
//! every key's tangents (`m_in` / `m_out`): a piece of a cubic with the same end values and slopes is
//! the same cubic, so those edits are exact. Edits that do change the timing (moving a key, setting a
//! speed) drop the pins and let the tangents follow the new keys.

use serde::{Deserialize, Serialize};

/// Shortest segment between two keys (clip seconds).
pub const MIN_SEG: f64 = 1.0 / 240.0;
/// Speeds below this count as standing still (holds; audio is muted there).
pub const STILL: f64 = 0.01;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TimeKey {
    /// Clip-local seconds.
    pub t: f64,
    /// Source seconds shown at `t`.
    pub s: f64,
    /// The segment from this key to the next eases (speed changes smoothly); off = constant speed.
    #[serde(default = "crate::model::tru")]
    pub smooth: bool,
    /// Pinned slope arriving at this key (source s per clip s); None = from the neighbouring keys.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub m_in: Option<f64>,
    /// Pinned slope leaving this key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub m_out: Option<f64>,
}

impl TimeKey {
    pub fn new(t: f64, s: f64) -> Self {
        Self { t, s, smooth: true, m_in: None, m_out: None }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TimeMap {
    pub keys: Vec<TimeKey>,
    /// Source frame rate: video lookups land on the middle of a source frame, so a hold asks for the
    /// same frame every time and a rewind finds the frames the forward pass just decoded. 0 = off.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub fps: f64,
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

/// Keep `v` on its own side and no steeper than 3x `d` (monotone cubic condition).
fn tame(v: f64, d: f64) -> f64 {
    if v * d <= 0.0 {
        0.0
    } else {
        v.signum() * v.abs().min(3.0 * d.abs())
    }
}

impl TimeMap {
    /// A straight run from source `s0` at t = 0 to `s1` at t = `dur`.
    pub fn linear(dur: f64, s0: f64, s1: f64, fps: f64) -> Self {
        Self { keys: vec![TimeKey::new(0.0, s0), TimeKey::new(dur.max(MIN_SEG), s1)], fps }
    }
    /// Clip length the keys span.
    pub fn duration(&self) -> f64 {
        self.keys.last().map_or(0.0, |k| k.t)
    }
    /// Sort, drop bad and crowded keys, start at t = 0. False if fewer than two keys are left.
    pub fn normalize(&mut self) -> bool {
        self.keys.retain(|k| k.t.is_finite() && k.s.is_finite());
        self.keys.sort_by(|a, b| a.t.total_cmp(&b.t));
        self.keys.dedup_by(|b, a| b.t - a.t < MIN_SEG);
        if let Some(t0) = self.keys.first().map(|k| k.t) {
            self.keys.iter_mut().for_each(|k| k.t -= t0);
        }
        if !self.fps.is_finite() || self.fps < 0.0 {
            self.fps = 0.0;
        }
        self.keys.len() >= 2
    }
    fn secant(&self, i: usize) -> f64 {
        let (a, b) = (self.keys[i], self.keys[i + 1]);
        (b.s - a.s) / (b.t - a.t).max(1e-12)
    }
    /// The slope from the neighbouring keys alone (no pins).
    fn auto(&self, i: usize) -> f64 {
        let n = self.keys.len();
        let left = (i > 0).then(|| (self.secant(i - 1), self.keys[i - 1].smooth));
        let right = (i + 1 < n).then(|| (self.secant(i), self.keys[i].smooth));
        match (left, right) {
            (Some((a, sa)), Some((b, sb))) => match (sa, sb) {
                // next to a constant-speed segment: carry its speed on, so the speed has no jump there
                (false, true) => tame(a, b),
                (true, false) => tame(b, a),
                _ if a * b <= 0.0 => 0.0,
                _ => 2.0 * a * b / (a + b),
            },
            (Some((d, _)), None) | (None, Some((d, _))) => d,
            (None, None) => 1.0,
        }
    }
    fn tan_in(&self, i: usize) -> f64 {
        self.keys[i].m_in.unwrap_or_else(|| self.auto(i))
    }
    fn tan_out(&self, i: usize) -> f64 {
        self.keys[i].m_out.unwrap_or_else(|| self.auto(i))
    }
    /// The segment `t` falls in (clamped to the first / last one).
    pub fn seg(&self, t: f64) -> usize {
        let n = self.keys.len();
        if n < 2 {
            return 0;
        }
        self.keys.partition_point(|k| k.t <= t).saturating_sub(1).min(n - 2)
    }
    /// (source, speed) at clip-local `t`. Outside the keys the edge slope carries on in a straight line.
    pub fn eval2(&self, t: f64) -> (f64, f64) {
        let n = self.keys.len();
        match n {
            0 => return (t, 1.0),
            1 => return (self.keys[0].s, 0.0),
            _ => {}
        }
        let (first, last) = (self.keys[0], self.keys[n - 1]);
        if t <= first.t {
            let m = self.tan_out(0);
            return (first.s + (t - first.t) * m, m);
        }
        if t >= last.t {
            let m = self.tan_in(n - 1);
            return (last.s + (t - last.t) * m, m);
        }
        let i = self.seg(t);
        let (a, b) = (self.keys[i], self.keys[i + 1]);
        let h = (b.t - a.t).max(1e-12);
        if !a.smooth {
            let d = (b.s - a.s) / h;
            return (a.s + (t - a.t) * d, d);
        }
        let (m0, m1) = (self.tan_out(i) * h, self.tan_in(i + 1) * h);
        let u = (t - a.t) / h;
        let (u2, u3) = (u * u, u * u * u);
        let s = (2.0 * u3 - 3.0 * u2 + 1.0) * a.s + (u3 - 2.0 * u2 + u) * m0 + (-2.0 * u3 + 3.0 * u2) * b.s + (u3 - u2) * m1;
        let ds = (6.0 * u2 - 6.0 * u) * a.s + (3.0 * u2 - 4.0 * u + 1.0) * m0 + (-6.0 * u2 + 6.0 * u) * b.s + (3.0 * u2 - 2.0 * u) * m1;
        (s, ds / h)
    }
    /// Source seconds at clip-local `t`.
    pub fn eval(&self, t: f64) -> f64 {
        self.eval2(t).0
    }
    /// Signed speed at clip-local `t` (1 = normal, 0 = hold, negative = backwards).
    pub fn speed(&self, t: f64) -> f64 {
        self.eval2(t).1
    }
    /// `eval` landed on the middle of its source frame (`fps`), for picture lookups.
    pub fn frame_time(&self, t: f64) -> f64 {
        let s = self.eval(t);
        if self.fps > 0.0 {
            ((s * self.fps + 1e-6).floor() + 0.5) / self.fps
        } else {
            s
        }
    }
    /// Lowest and highest source time shown over clip-local [0, dur] (segments are monotone, so the
    /// extremes sit on keys or the ends).
    pub fn range(&self, dur: f64) -> (f64, f64) {
        let (a, b) = (self.eval(0.0), self.eval(dur));
        let inner = self.keys.iter().filter(|k| k.t > 0.0 && k.t < dur).map(|k| k.s);
        inner.fold((a.min(b), a.max(b)), |(lo, hi), s| (lo.min(s), hi.max(s)))
    }
    /// Pin every key's slopes to what they are now, so a structural edit keeps the curve exactly.
    pub fn pin_all(&mut self) {
        let pins: Vec<(f64, f64)> = (0..self.keys.len()).map(|i| (self.tan_in(i), self.tan_out(i))).collect();
        for (k, (mi, mo)) in self.keys.iter_mut().zip(pins) {
            (k.m_in, k.m_out) = (Some(mi), Some(mo));
        }
    }
    /// Let the slopes follow the keys again (after an edit that changes the timing).
    pub fn unpin(&mut self) {
        for k in &mut self.keys {
            (k.m_in, k.m_out) = (None, None);
        }
    }
    /// Shift keys `from..` by `dt` clip seconds.
    fn shift_from(&mut self, from: usize, dt: f64) {
        self.keys.iter_mut().skip(from).for_each(|k| k.t += dt);
    }
    /// The key at `t` (within half a segment minimum), else a new one there that changes nothing.
    /// Returns its index.
    pub fn insert_exact(&mut self, t: f64) -> usize {
        if let Some(i) = self.keys.iter().position(|k| (k.t - t).abs() < MIN_SEG / 2.0) {
            return i;
        }
        self.pin_all();
        let (s, m) = self.eval2(t);
        let i = self.keys.partition_point(|k| k.t < t);
        let smooth = if i == 0 { false } else { self.keys[i - 1].smooth };
        self.keys.insert(i, TimeKey { t, s, smooth, m_in: Some(m), m_out: Some(m) });
        if i + 1 == self.keys.len() && i > 0 {
            self.keys[i - 1].smooth = false; // past the last key: the straight extrapolation
        }
        if i == 0 {
            // a key before the first one: the stretch to the old first key is the straight extrapolation
            self.keys[0].smooth = false;
        }
        i
    }
    /// The part of the map over clip-local [a, b], re-based so `a` is t = 0 - exact, the same picture
    /// at every time it keeps. Past the ends the edge speed carries on.
    pub fn crop(&self, a: f64, b: f64) -> TimeMap {
        let mut m = self.clone();
        m.pin_all();
        let b = b.max(a + MIN_SEG);
        m.insert_exact(a);
        m.insert_exact(b);
        m.keys.retain(|k| k.t > a - MIN_SEG / 2.0 && k.t < b + MIN_SEG / 2.0);
        m.keys.iter_mut().for_each(|k| k.t -= a);
        if let Some(k) = m.keys.first_mut() {
            k.t = 0.0;
        }
        m
    }
    /// Freeze the frame at `t` for `len` clip seconds: everything after moves `len` later.
    /// Returns the change in duration.
    pub fn hold(&mut self, t: f64, len: f64) -> f64 {
        let len = len.max(MIN_SEG);
        let t = t.clamp(0.0, self.duration());
        self.pin_all();
        let k = self.insert_exact(t);
        let mut end = self.keys[k];
        end.t += len;
        end.m_in = Some(0.0);
        self.keys[k].smooth = false;
        self.keys[k].m_out = Some(0.0);
        self.shift_from(k + 1, len);
        self.keys.insert(k + 1, end);
        len
    }
    /// Play segment `i` backwards right after it, then forwards again to where it was (the picture never
    /// jumps). Returns the change in duration (twice the segment's length).
    pub fn rewind(&mut self, i: usize) -> f64 {
        if i + 1 >= self.keys.len() {
            return 0.0;
        }
        self.pin_all();
        let (a, b) = (self.keys[i], self.keys[i + 1]);
        let len = b.t - a.t;
        let (ma, mb) = (a.m_out.unwrap_or(0.0), b.m_in.unwrap_or(0.0));
        let back = TimeKey { t: b.t + len, s: a.s, smooth: a.smooth, m_in: Some(-ma), m_out: Some(ma) };
        let again = TimeKey { t: b.t + 2.0 * len, s: b.s, smooth: b.smooth, m_in: Some(mb), m_out: b.m_out };
        self.keys[i + 1].m_out = Some(-mb);
        self.keys[i + 1].smooth = a.smooth;
        self.shift_from(i + 2, 2.0 * len);
        self.keys.insert(i + 2, back);
        self.keys.insert(i + 3, again);
        2.0 * len
    }
    /// Give segment `i` this speed (its sign is the segment's own; the source it covers stays, so its
    /// length changes and the keys after it move). A hold has no speed: `speed` is ignored there.
    /// Returns the change in duration.
    pub fn set_segment_speed(&mut self, i: usize, speed: f64) -> f64 {
        if i + 1 >= self.keys.len() || !speed.is_finite() || speed.abs() < STILL {
            return 0.0;
        }
        let (a, b) = (self.keys[i], self.keys[i + 1]);
        let ds = (b.s - a.s).abs();
        if ds < 1e-9 {
            return 0.0;
        }
        let len = (ds / speed.abs()).max(MIN_SEG);
        self.unpin();
        let dt = len - (b.t - a.t);
        self.shift_from(i + 1, dt);
        dt
    }
    /// Make segment `i` (a hold or any other) last `len` clip seconds. Returns the change in duration.
    pub fn set_segment_len(&mut self, i: usize, len: f64) -> f64 {
        if i + 1 >= self.keys.len() || !len.is_finite() {
            return 0.0;
        }
        let dt = len.max(MIN_SEG) - (self.keys[i + 1].t - self.keys[i].t);
        self.unpin();
        self.shift_from(i + 1, dt);
        dt
    }
    /// Move key `i` to clip time `t`; the keys after it move with it (their segments keep their
    /// lengths). Key 0 stays at 0. Returns the change in duration.
    pub fn move_key(&mut self, i: usize, t: f64) -> f64 {
        if i == 0 || i >= self.keys.len() || !t.is_finite() {
            return 0.0;
        }
        let t = t.max(self.keys[i - 1].t + MIN_SEG);
        let dt = t - self.keys[i].t;
        self.unpin();
        self.shift_from(i, dt);
        dt
    }
    /// Show source `s` at key `i` (the speeds either side change; the length does not).
    pub fn set_key_source(&mut self, i: usize, s: f64) {
        if let Some(k) = self.keys.get_mut(i).filter(|_| s.is_finite()) {
            k.s = s;
            self.unpin();
        }
    }
    /// Remove an inner key (the ends stay). The length does not change.
    pub fn remove_key(&mut self, i: usize) -> bool {
        if i == 0 || i + 1 >= self.keys.len() {
            return false;
        }
        self.keys.remove(i);
        self.unpin();
        true
    }
    /// Constant speed or easing for segment `i`.
    pub fn set_smooth(&mut self, i: usize, smooth: bool) {
        if let Some(k) = self.keys.get_mut(i) {
            k.smooth = smooth;
            self.unpin();
        }
    }
    /// Every speed times `f` (the clip's length divides by it).
    pub fn scale_speed(&mut self, f: f64) {
        if f.is_finite() && f > 0.0 {
            for k in &mut self.keys {
                k.t /= f;
                k.m_in = k.m_in.map(|m| m * f);
                k.m_out = k.m_out.map(|m| m * f);
            }
        }
    }
    /// Move every source time by `ds` (a slip).
    pub fn slip(&mut self, ds: f64) {
        self.keys.iter_mut().for_each(|k| k.s += ds);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    fn ramp() -> TimeMap {
        // 1x for 2 s, then a smooth slow-down to a hold, then 1x again
        let mut m = TimeMap::linear(2.0, 10.0, 12.0, 0.0);
        m.keys[0].smooth = false;
        m.keys.push(TimeKey::new(4.0, 13.0));
        m.keys.push(TimeKey::new(5.0, 13.0));
        m.keys[2].smooth = false;
        m.keys.push(TimeKey::new(6.0, 14.0));
        m
    }

    #[test]
    fn linear_map_is_constant_speed() {
        let m = TimeMap::linear(4.0, 1.0, 9.0, 0.0);
        assert!(near(m.eval(0.0), 1.0) && near(m.eval(2.0), 5.0) && near(m.eval(4.0), 9.0));
        assert!(near(m.speed(1.3), 2.0));
        // past the ends the edge speed carries on
        assert!(near(m.eval(5.0), 11.0) && near(m.eval(-1.0), -1.0));
    }

    #[test]
    fn smooth_segments_are_monotone_and_hit_their_keys() {
        let m = ramp();
        for k in &m.keys {
            assert!(near(m.eval(k.t), k.s), "key {k:?}");
        }
        let mut last = f64::NEG_INFINITY;
        for i in 0..=600 {
            let s = m.eval(i as f64 * 0.01);
            assert!(s >= last - 1e-9, "goes back at {}", i as f64 * 0.01);
            last = s;
        }
        // speed eases from 1x down to 0 into the hold, no jump at the key after the constant segment
        assert!(near(m.speed(2.0 - 1e-9), 1.0) && (m.speed(2.0 + 1e-6) - 1.0).abs() < 1e-3);
        assert!(m.speed(3.9) < 0.2);
        assert!(m.speed(4.5).abs() < 1e-9, "hold");
    }

    #[test]
    fn reverse_segment_runs_backwards() {
        let mut m = TimeMap::linear(2.0, 5.0, 7.0, 0.0);
        m.keys.push(TimeKey::new(3.0, 6.0));
        let mut last = f64::INFINITY;
        for i in 0..=100 {
            let s = m.eval(2.0 + i as f64 * 0.01);
            assert!(s <= last + 1e-9);
            last = s;
        }
        assert!(m.speed(2.5) < 0.0);
    }

    #[test]
    fn hold_inserts_a_freeze_and_keeps_the_rest() {
        let orig = ramp();
        let mut m = orig.clone();
        let d = m.hold(1.0, 0.5);
        assert!(near(d, 0.5) && near(m.duration(), orig.duration() + 0.5));
        for i in 0..=100 {
            let t = i as f64 * 0.01;
            assert!(near(m.eval(t), orig.eval(t)), "before the hold");
        }
        for i in 0..=50 {
            assert!(near(m.eval(1.0 + i as f64 * 0.01), orig.eval(1.0)), "held");
        }
        for i in 0..=400 {
            let t = 1.0 + i as f64 * 0.0125;
            assert!(near(m.eval(t + 0.5), orig.eval(t)), "after the hold, at {t}");
        }
    }

    #[test]
    fn hold_inside_a_smooth_segment_is_exact() {
        let orig = ramp();
        let mut m = orig.clone();
        m.hold(3.0, 1.0);
        for i in 0..=300 {
            let t = i as f64 * 0.01;
            assert!(near(m.eval(t), orig.eval(t)));
            assert!(near(m.eval(t + 3.0 + 1.0), orig.eval(t + 3.0)) || t + 3.0 > 6.0);
        }
    }

    #[test]
    fn rewind_goes_back_then_forward_without_a_jump() {
        let orig = TimeMap::linear(4.0, 0.0, 4.0, 0.0);
        let mut m = orig.clone();
        let i = m.insert_exact(1.0);
        m.insert_exact(2.0);
        let d = m.rewind(i);
        assert!(near(d, 2.0) && near(m.duration(), 6.0));
        assert!(near(m.eval(2.0), 2.0) && near(m.eval(3.0), 1.0) && near(m.eval(4.0), 2.0));
        assert!(m.speed(2.5) < 0.0 && m.speed(3.5) > 0.0);
        assert!(near(m.eval(5.0), 3.0) && near(m.eval(6.0), 4.0));
        // continuous everywhere
        for i in 0..600 {
            let t = i as f64 * 0.01;
            assert!((m.eval(t + 0.01) - m.eval(t)).abs() < 0.02, "jump at {t}");
        }
    }

    #[test]
    fn segment_speed_and_key_moves_ripple_the_duration() {
        let mut m = TimeMap::linear(2.0, 0.0, 2.0, 0.0);
        m.keys.push(TimeKey::new(3.0, 3.0));
        assert!(near(m.set_segment_speed(0, 0.5), 2.0));
        assert!(near(m.duration(), 5.0) && near(m.eval(4.0), 2.0));
        assert!(near(m.move_key(1, 3.0), -1.0) && near(m.duration(), 4.0));
        let d = m.set_segment_len(1, 2.0);
        assert!(near(d, 1.0) && near(m.duration(), 5.0));
        assert!(m.remove_key(1) && near(m.duration(), 5.0) && m.keys.len() == 2);
    }

    #[test]
    fn crop_is_exact_and_rebased() {
        let orig = ramp();
        let c = orig.crop(1.5, 4.5);
        assert!(near(c.duration(), 3.0));
        for i in 0..=300 {
            let t = i as f64 * 0.01;
            assert!(near(c.eval(t), orig.eval(1.5 + t)), "at {t}");
        }
        // extending past the end carries the last speed on
        let e = orig.crop(0.0, 7.0);
        assert!(near(e.eval(7.0), 15.0));
    }

    #[test]
    fn range_and_frame_snapping() {
        let mut m = TimeMap::linear(2.0, 5.0, 7.0, 30.0);
        m.keys.push(TimeKey::new(3.0, 3.0));
        assert_eq!(m.range(3.0), (3.0, 7.0));
        let f = m.frame_time(0.0);
        assert!(near(f, (150.0 + 0.5) / 30.0));
        // a hold asks for the same frame every time
        let mut h = TimeMap::linear(1.0, 2.0, 2.0, 30.0);
        h.keys[0].smooth = false;
        assert_eq!(h.frame_time(0.1).to_bits(), h.frame_time(0.9).to_bits());
    }

    #[test]
    fn normalize_sorts_and_rebases() {
        let mut m = TimeMap { keys: vec![TimeKey::new(3.0, 1.0), TimeKey::new(1.0, 0.0), TimeKey::new(1.0001, 5.0)], fps: f64::NAN };
        assert!(m.normalize());
        assert_eq!(m.keys.len(), 2);
        assert!(near(m.keys[0].t, 0.0) && near(m.keys[1].t, 2.0) && m.fps == 0.0);
        let mut one = TimeMap { keys: vec![TimeKey::new(0.0, 0.0)], fps: 0.0 };
        assert!(!one.normalize());
    }

    #[test]
    fn scale_speed_divides_the_length() {
        let mut m = ramp();
        let o = m.clone();
        m.scale_speed(2.0);
        assert!(near(m.duration(), 3.0));
        assert!(near(m.eval(1.7), o.eval(3.4)));
    }
}
