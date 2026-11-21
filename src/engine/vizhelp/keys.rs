//! ---- ws:visualizer-helper ----
//! Visualizer Templates: the hand keys the Olympus Storm shader reads, built from a song analysis and the
//! template's knobs (the keys part of `hsv_analyze.py`). A pure function: the same analysis and knobs give
//! the same keys, key for key.
//!
//! Keys written (param -> what it carries):
//! `kclock` one key per hit, value = hit number, linear between (fract() = the phase, floor() the count);
//! `kpow` each hit's strength (Hold); `sclock` the same clock for the screeches, ramping over 0.6 s;
//! `screech`, `vox`, `vib` the synth / vocal / vibrato curves; `fx` 0 before the first drop, 1 from it;
//! `kick` 1.2, raised to the "harder" dial from the second drop to the calm part; `beam` the pre-drop glow
//! rising 0.3 -> 1 and a white flash (3) on each drop decaying in 0.4 s. The app's own visualizer bake
//! keeps `level` / `band1` / `band4` / `wave`; nothing here touches them.

use super::dsp;
use crate::model::{Ease, Keyframe, SongViz};

/// The knobs of a template.
#[derive(Clone, Debug, PartialEq)]
pub struct Knobs {
    /// How readily a hit pops (0.25..2: below 1 only the big hits, above 1 the small ones too).
    pub sensitivity: f64,
    /// Smoothing of the voice and synth curves (0 = as analysed, 1 = very smooth).
    pub smoothing: f64,
    /// The kick level from the second drop to the calm part ("go harder"; 1.2 = no change).
    pub harder: f64,
    /// Spines popped per hit (1..6; the template's own is 4).
    pub spikes: f64,
    /// The metal before and after the first drop, and the electricity before / after (RGB 0..255).
    pub metal_pre: [u8; 3],
    pub metal_post: [u8; 3],
    pub elec_pre: [u8; 3],
    pub elec_post: [u8; 3],
    /// Which layers draw.
    pub ball: bool,
    pub wave: bool,
    pub bolts: bool,
    pub beam: bool,
}

impl Default for Knobs {
    fn default() -> Knobs {
        Knobs {
            sensitivity: 1.0,
            smoothing: 0.0,
            harder: 2.2,
            spikes: 4.0,
            metal_pre: [158, 163, 173],
            metal_post: [255, 184, 77],
            elec_pre: [115, 191, 255],
            elec_post: [255, 199, 89],
            ball: true,
            wave: true,
            bolts: true,
            beam: true,
        }
    }
}

/// The templates: (name, layers ball / wave / bolts / beam).
pub const TEMPLATES: &[(&str, [bool; 4])] = &[
    ("Olympus Storm", [true, true, true, true]),
    ("Chrome Waveform", [false, true, false, false]),
    ("Storm Lightning", [false, false, true, true]),
];

/// The params the template writes (in the shader's header order).
pub const PARAMS: [&str; 10] = ["kclock", "kpow", "sclock", "vox", "vib", "kick", "screech", "fx", "beam", "lyric"];

fn key(t: f64, v: f64, ease: Ease) -> Keyframe {
    Keyframe { t: dsp::r3(t), v: dsp::r3(v), ease }
}

/// A curve smoothed forwards and backwards with a one-pole of `tau` s, then simplified again at `tol`.
fn smooth(points: &[[f32; 2]], tau: f64, tol: f64) -> Vec<[f32; 2]> {
    if tau <= 1e-6 || points.len() < 3 {
        return points.to_vec();
    }
    let dt = 0.01;
    let end = points.last().map_or(0.0, |p| p[0] as f64);
    let n = (end / dt).floor() as usize + 1;
    let mut j = 0;
    let mut y: Vec<f64> = (0..n)
        .map(|i| {
            let t = i as f64 * dt;
            while j + 1 < points.len() && points[j + 1][0] as f64 <= t {
                j += 1;
            }
            match points.get(j + 1) {
                Some(b) => {
                    let a = points[j];
                    let f = ((t - a[0] as f64) / ((b[0] - a[0]) as f64).max(1e-9)).clamp(0.0, 1.0);
                    a[1] as f64 + (b[1] - a[1]) as f64 * f
                }
                None => points[j][1] as f64,
            }
        })
        .collect();
    let a = 1.0 - (-dt / tau).exp();
    for i in 1..n {
        y[i] = y[i - 1] + (y[i] - y[i - 1]) * a;
    }
    for i in (0..n.saturating_sub(1)).rev() {
        y[i] = y[i + 1] + (y[i] - y[i + 1]) * a;
    }
    dsp::curve(&y, dt, dt * 3.0, tol)
}

/// Keys in song (source) seconds, before windowing to a clip.
pub fn song_keys(v: &SongViz, k: &Knobs) -> Vec<(&'static str, Vec<Keyframe>)> {
    let sens = k.sensitivity.clamp(0.25, 2.0);
    let fx0 = v.drops.first().map(|&d| d as f64);
    let mut out = Vec::new();
    let mut kc = vec![key(0.0, 0.0, Ease::Linear)];
    let mut kp = vec![key(0.0, 0.0, Ease::Hold)];
    for (i, h) in v.hits.iter().enumerate() {
        kc.push(key(h[0] as f64, (i + 1) as f64, Ease::Linear));
        // strength^(1/sensitivity): 1 stays the boom, small hits rise or sink
        let w = (h[1] as f64).clamp(0.0, 1.0);
        kp.push(Keyframe { t: dsp::r3(h[0] as f64), v: (w.powf(1.0 / sens) * 100.0).round() / 100.0, ease: Ease::Hold });
    }
    out.push(("kclock", kc));
    out.push(("kpow", kp));
    let mut sc = vec![key(0.0, 0.0, Ease::Hold)];
    for (i, &t) in v.screeches.iter().enumerate() {
        let t = t as f64;
        let next = v.screeches.get(i + 1).map_or(t + 1.0, |&n| n as f64);
        sc.push(key(t, (i + 1) as f64, Ease::Linear));
        sc.push(key((t + 0.6).min(next - 0.01), (i + 1) as f64 + 0.99, Ease::Hold));
    }
    out.push(("sclock", sc));
    let tau = k.smoothing.clamp(0.0, 1.0) * 0.25;
    let line = |pts: &[[f32; 2]], tol: f64| -> Vec<Keyframe> { smooth(pts, tau, tol).iter().map(|p| key(p[0] as f64, p[1] as f64, Ease::Linear)).collect() };
    out.push(("vox", line(&v.vox, 0.035)));
    out.push(("vib", line(&v.vib, 0.09)));
    let calm = v.calm.map_or(v.duration, |c| c as f64);
    let mut kk = vec![key(0.0, 1.2, Ease::Hold)];
    if v.drops.len() >= 2 {
        kk.push(key(v.drops[1] as f64, k.harder.clamp(0.0, 3.0), Ease::Hold));
        kk.push(key(calm, 1.2, Ease::Hold));
    }
    out.push(("kick", kk));
    // the synth curve scales with the sensitivity too (the electricity reads smoothstep(0.8, 2.1, screech))
    let scr: Vec<Keyframe> = smooth(&v.screech, tau, 0.08).iter().map(|p| key(p[0] as f64, (p[1] as f64 * sens.sqrt()).min(3.0), Ease::Linear)).collect();
    out.push(("screech", scr));
    out.push(("fx", match fx0 {
        Some(d) => vec![key(0.0, 0.0, Ease::Hold), key(d, 1.0, Ease::Hold)],
        None => vec![key(0.0, 1.0, Ease::Hold)],
    }));
    let mut bm = vec![key(0.0, 0.3, Ease::Linear)];
    for &d in &v.drops {
        let d = d as f64;
        let last = bm.last().map_or(0.0, |k| k.t);
        bm.push(key((d - 6.0).max(last + 0.01), 0.45, Ease::EaseIn));
        bm.push(key(d - 0.02, 1.0, Ease::Linear));
        bm.push(key(d, 3.0, Ease::EaseOut));
        bm.push(key(d + 0.4, 1.0, Ease::Linear));
    }
    out.push(("beam", bm));
    out
}

/// The value of `keys` at `t` (Hold steps, everything else linear: what windowing needs).
fn value_at(keys: &[Keyframe], t: f64) -> f64 {
    let i = keys.partition_point(|k| k.t <= t);
    if i == 0 {
        return keys.first().map_or(0.0, |k| k.v);
    }
    let a = &keys[i - 1];
    match keys.get(i) {
        Some(b) if a.ease != Ease::Hold => a.v + (b.v - a.v) * ((t - a.t) / (b.t - a.t).max(1e-9)),
        _ => a.v,
    }
}

/// `keys` (song seconds) as a clip sees them: shifted so song second `from` is clip-local 0 (at `speed`),
/// cut to `0..len` with a key at each end carrying the value there.
pub fn window(keys: &[Keyframe], from: f64, len: f64, speed: f64) -> Vec<Keyframe> {
    let speed = if speed.is_finite() && speed > 0.0 { speed } else { 1.0 };
    let to_local = |t: f64| (t - from) / speed;
    let end = from + len * speed;
    let mut out: Vec<Keyframe> = Vec::new();
    let first_in = keys.iter().position(|k| k.t >= from);
    if first_in.is_none_or(|i| keys[i].t > from + 1e-9) && !keys.is_empty() {
        let before = keys.iter().rposition(|k| k.t < from);
        let ease = before.map_or(Ease::Hold, |i| keys[i].ease);
        out.push(Keyframe { t: 0.0, v: dsp::r3(value_at(keys, from)), ease });
    }
    for k in keys.iter().filter(|k| k.t >= from && k.t <= end) {
        out.push(Keyframe { t: dsp::r3(to_local(k.t)), v: k.v, ease: k.ease });
    }
    if let Some(last) = keys.iter().rposition(|k| k.t <= end) {
        if last + 1 < keys.len() && keys[last].ease != Ease::Hold && out.last().is_some_and(|k| k.t < len - 1e-6) {
            out.push(Keyframe { t: dsp::r3(len), v: dsp::r3(value_at(keys, end)), ease: Ease::Linear });
        }
    }
    out
}

/// The `lyric` keys of the visualizer: 1 while a lyric line is up (the waveform flattens under it), easing
/// over 0.2 s each side; lines closer than that merge. Spans in song seconds.
pub fn lyric_keys(lines: &[(f64, f64)]) -> Vec<Keyframe> {
    let mut spans: Vec<(f64, f64)> = Vec::new();
    for &(s, e) in lines {
        match spans.last_mut() {
            Some(l) if s - 0.2 <= l.1 + 0.2 => l.1 = l.1.max(e),
            _ => spans.push((s, e)),
        }
    }
    let mut out = vec![key(0.0, 0.0, Ease::Linear)];
    for (s, e) in spans {
        out.push(key((s - 0.2).max(out.last().map_or(0.0, |k| k.t + 0.001)), 0.0, Ease::Linear));
        out.push(key(s + 0.2, 1.0, Ease::Linear));
        out.push(key((e - 0.2).max(s + 0.2), 1.0, Ease::Linear));
        out.push(key(e + 0.2, 0.0, Ease::Linear));
    }
    out.dedup_by(|b, a| (a.t - b.t).abs() < 1e-9);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song() -> SongViz {
        SongViz {
            duration: 60.0,
            bpm: 150.0,
            hits: (0..100).map(|i| [0.5 + 0.4 * i as f32, if i % 4 == 0 { 1.0 } else { 0.6 }]).collect(),
            screeches: vec![21.0, 23.0, 31.5],
            drops: vec![20.0, 40.0],
            calm: Some(52.0),
            vox: vec![[0.0, 0.0], [6.0, 0.0], [6.2, 0.8], [9.0, 0.6], [9.5, 0.0], [60.0, 0.0]],
            vib: vec![[0.0, 0.0], [7.0, 0.3], [7.1, -0.3], [7.2, 0.0], [60.0, 0.0]],
            screech: vec![[0.0, 0.0], [20.0, 0.0], [21.0, 2.3], [22.0, 0.4], [60.0, 0.0]],
            ..Default::default()
        }
    }

    #[test]
    fn template_keys_are_deterministic() {
        let k = Knobs { sensitivity: 1.3, smoothing: 0.4, ..Default::default() };
        let a = song_keys(&song(), &k);
        let b = song_keys(&song(), &k);
        assert_eq!(a, b);
        let names: Vec<&str> = a.iter().map(|(n, _)| *n).collect();
        assert_eq!(names, vec!["kclock", "kpow", "sclock", "vox", "vib", "kick", "screech", "fx", "beam"]);
        // one clock key per hit (+ the start), the boom stays 1
        let kc = &a[0].1;
        assert_eq!(kc.len(), 101);
        assert_eq!((kc[1].t, kc[1].v), (0.5, 1.0));
        assert_eq!(a[1].1[1].v, 1.0);
        assert!(a[1].1[2].v > 0.6, "sensitivity above 1 lifts the small hits");
        // fx on at the first drop, the harder dial from the second drop to the calm part
        assert_eq!(a[7].1.iter().map(|k| (k.t, k.v)).collect::<Vec<_>>(), vec![(0.0, 0.0), (20.0, 1.0)]);
        assert_eq!(a[5].1.iter().map(|k| (k.t, k.v)).collect::<Vec<_>>(), vec![(0.0, 1.2), (40.0, 2.2), (52.0, 1.2)]);
        // the drop flash
        assert!(a[8].1.iter().any(|k| k.t == 20.0 && k.v == 3.0));
        // other knobs, other keys
        assert_ne!(song_keys(&song(), &Knobs::default()), a);
    }

    #[test]
    fn a_window_shifts_and_keeps_the_value_at_its_start() {
        let keys = vec![key(0.0, 0.0, Ease::Linear), key(10.0, 10.0, Ease::Linear), key(20.0, 20.0, Ease::Linear)];
        let w = window(&keys, 5.0, 10.0, 1.0);
        assert_eq!(w.iter().map(|k| (k.t, k.v)).collect::<Vec<_>>(), vec![(0.0, 5.0), (5.0, 10.0), (10.0, 15.0)]);
        let hold = vec![key(0.0, 1.0, Ease::Hold), key(8.0, 2.0, Ease::Hold)];
        assert_eq!(window(&hold, 5.0, 10.0, 1.0).iter().map(|k| (k.t, k.v)).collect::<Vec<_>>(), vec![(0.0, 1.0), (3.0, 2.0)]);
    }

    #[test]
    fn lyric_keys_merge_close_lines() {
        let k = lyric_keys(&[(1.0, 2.0), (2.1, 3.0), (6.0, 7.0)]);
        let v: Vec<(f64, f64)> = k.iter().map(|k| (k.t, k.v)).collect();
        assert_eq!(v, vec![(0.0, 0.0), (0.8, 0.0), (1.2, 1.0), (2.8, 1.0), (3.2, 0.0), (5.8, 0.0), (6.2, 1.0), (6.8, 1.0), (7.2, 0.0)]);
    }
}
