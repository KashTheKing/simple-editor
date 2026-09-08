//! Peaks analysis toolkit: onsets/BPM, peak/RMS levels, cross-correlation offset, normalize/match
//! loudness, auto-ducking, and ffmpeg scene-cut detection — all pure functions over
//! media::waveform::Peaks (100 buckets/s), no decoding, no App/UI types. Mirrors autocut.rs's style.
//!
//! `onsets()`/`scene_cuts()` return times in SOURCE seconds (same space as autocut::loud_segments,
//! see autocut.rs:29); every call site that turns one of those into a Marker or a split point must
//! first convert via `to_clip_local`/`to_timeline_t` — the same (t - src_in)/speed[+start] math
//! autocut::to_timeline already performs (autocut.rs:78-91), centralized here so it's fixed once.

use crate::engine::autocut::{self, AutoCutParams};
use crate::media::waveform::{Peaks, PEAKS_PER_SEC};
use crate::model::{Animated, Clip, Id, Project};
use std::sync::Arc;

/// Convert a source-seconds time (as returned by `onsets()`/`scene_cuts()`) into clip-LOCAL time,
/// for `Project::add_clip_marker` (which clamps to `[0, c.duration]`).
pub fn to_clip_local(t_src: f64, c: &Clip) -> f64 {
    (t_src - c.src_in) / c.speed
}

/// Convert a source-seconds time into TIMELINE time, for `Project::split_at` (which checks
/// `c.contains(t)`).
pub fn to_timeline_t(t_src: f64, c: &Clip) -> f64 {
    c.start + to_clip_local(t_src, c)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Levels {
    pub peak_db: f32,
    pub rms_db: f32,
}

fn to_db(lin: f32) -> f32 {
    20.0 * lin.max(1e-7).log10()
}

/// Peak dBFS + bucket-approximated RMS dBFS over a source-time range [a, b).
pub fn levels(p: &Peaks, a: f64, b: f64) -> Levels {
    let (i0, i1) = bucket_range(p, a, b);
    if i0 >= i1 {
        return Levels { peak_db: to_db(0.0), rms_db: to_db(0.0) };
    }
    let mut peak = 0.0f32;
    let mut sum_sq = 0.0f64;
    for i in i0..i1 {
        let m = p.min[i].abs().max(p.max[i].abs());
        peak = peak.max(m);
        // approximate bucket RMS as the RMS of its min/max extremes
        sum_sq += ((p.min[i] as f64).powi(2) + (p.max[i] as f64).powi(2)) / 2.0;
    }
    let rms = (sum_sq / (i1 - i0) as f64).sqrt() as f32;
    Levels { peak_db: to_db(peak), rms_db: to_db(rms) }
}

fn bucket_range(p: &Peaks, a: f64, b: f64) -> (usize, usize) {
    let per = PEAKS_PER_SEC as f64;
    let i0 = ((a * per).floor().max(0.0)) as usize;
    let i1 = ((b * per).ceil().max(0.0) as usize).min(p.len());
    (i0.min(i1), i1)
}

/// Local-max-over-rolling-mean onset picker with a refractory window; returned in SOURCE seconds.
/// `sensitivity` is the multiplier the rolling mean must be exceeded by (Settings.beat_thr default).
pub fn onsets(p: &Peaks, a: f64, b: f64, refractory_s: f64, sensitivity: f32) -> Vec<f64> {
    let per = PEAKS_PER_SEC as f64;
    let (i0, i1) = bucket_range(p, a, b);
    if i1 <= i0 {
        return Vec::new();
    }
    let env: Vec<f32> = (i0..i1).map(|i| p.min[i].abs().max(p.max[i].abs())).collect();
    let win = (0.5 * per).round().max(1.0) as usize; // 0.5s rolling mean window
    let refractory = (refractory_s * per).round().max(1.0) as usize;
    let mut out = Vec::new();
    let mut last_pick: Option<usize> = None;
    for i in 0..env.len() {
        let lo = i.saturating_sub(win / 2);
        let hi = (i + win / 2 + 1).min(env.len());
        let mean: f32 = env[lo..hi].iter().sum::<f32>() / (hi - lo) as f32;
        let thr = mean * sensitivity;
        if env[i] <= thr || env[i] <= 1e-6 {
            continue;
        }
        // local max within the refractory window around i
        let rlo = i.saturating_sub(refractory);
        let rhi = (i + refractory + 1).min(env.len());
        let is_local_max = env[rlo..rhi].iter().all(|&v| v <= env[i]);
        if !is_local_max {
            continue;
        }
        if let Some(lp) = last_pick {
            if i - lp < refractory {
                continue;
            }
        }
        last_pick = Some(i);
        out.push((i0 + i) as f64 / per);
    }
    out
}

/// Autocorrelation of inter-onset intervals in [60,180) BPM; `None` if fewer than 4 onsets or no
/// stable peak (interval spread too wide to trust).
pub fn bpm(onsets: &[f64]) -> Option<f64> {
    if onsets.len() < 4 {
        return None;
    }
    let mut intervals: Vec<f64> = onsets.windows(2).map(|w| w[1] - w[0]).filter(|&d| d > 1e-3).collect();
    if intervals.is_empty() {
        return None;
    }
    intervals.sort_by(f64::total_cmp);
    let median = intervals[intervals.len() / 2];
    if median <= 0.0 {
        return None;
    }
    let mut bpm = 60.0 / median;
    while bpm >= 180.0 {
        bpm /= 2.0;
    }
    while bpm < 60.0 {
        bpm *= 2.0;
    }
    // reject if intervals are too spread out to trust the median as a stable tempo
    let spread = intervals[intervals.len() - 1] - intervals[0];
    if spread > median * 1.5 {
        return None;
    }
    Some(bpm)
}

/// Best lag (seconds) aligning `b` to `a` via cross-correlation of their downsampled envelopes.
/// `max_lag` is hard-capped at 120s. `None` on empty input.
pub fn xcorr_offset(a: &Peaks, b: &Peaks, max_lag: f64) -> Option<f64> {
    if a.is_empty() || b.is_empty() {
        return None;
    }
    let max_lag = max_lag.min(120.0).max(0.0);
    const RATE: f64 = 25.0;
    let dur_a = a.len() as f64 / PEAKS_PER_SEC as f64;
    let dur_b = b.len() as f64 / PEAKS_PER_SEC as f64;
    let env = |p: &Peaks, dur: f64| -> Vec<f32> {
        let n = (dur * RATE).ceil().max(1.0) as usize;
        (0..n)
            .map(|i| {
                let t0 = i as f64 / RATE;
                let t1 = t0 + 1.0 / RATE;
                let (lo, hi) = p.range(t0, t1);
                lo.abs().max(hi.abs())
            })
            .collect()
    };
    let ea = env(a, dur_a);
    let eb = env(b, dur_b);
    if ea.is_empty() || eb.is_empty() {
        return None;
    }
    let max_lag_n = (max_lag * RATE).round() as isize;
    let mut best_score = f64::MIN;
    let mut best_lag_n = 0isize;
    for lag in -max_lag_n..=max_lag_n {
        let mut score = 0.0f64;
        let mut count = 0usize;
        for i in 0..ea.len() {
            let j = i as isize + lag;
            if j < 0 || j as usize >= eb.len() {
                continue;
            }
            score += (ea[i] as f64) * (eb[j as usize] as f64);
            count += 1;
        }
        if count == 0 {
            continue;
        }
        if score > best_score {
            best_score = score;
            best_lag_n = lag;
        }
    }
    if best_score <= 0.0 {
        return None;
    }
    Some(best_lag_n as f64 / RATE)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NormMode {
    Peak,
    Rms,
}

fn clip_levels(c: &Clip, peaks_of: &mut dyn FnMut(Id) -> Option<Arc<Peaks>>) -> Option<Levels> {
    let peaks = peaks_of(c.asset)?;
    Some(levels(&peaks, c.src_in, c.src_in + c.duration * c.speed))
}

/// Per-clip constant-gain scale to `target_dbfs`. Clips with keyframed volume are skipped, as are
/// clips with no peaks yet. Returns the number of clips changed.
pub fn normalize(
    project: &mut Project,
    ids: &[Id],
    target_dbfs: f64,
    mode: NormMode,
    peaks_of: &mut dyn FnMut(&Project, Id) -> Option<Arc<Peaks>>,
) -> usize {
    let mut changed = 0;
    for &id in ids {
        let Some(clip) = project.clip(id) else { continue };
        if clip.volume.is_animated() {
            continue;
        }
        let Some(lv) = clip_levels(clip, &mut |asset| peaks_of(project, asset)) else { continue };
        let current = match mode {
            NormMode::Peak => lv.peak_db,
            NormMode::Rms => lv.rms_db,
        } as f64;
        let gain = 10f64.powf((target_dbfs - current) / 20.0);
        if let Some(clip) = project.clip_mut(id) {
            let v = clip.volume.value * gain;
            clip.volume.set_at(0.0, v);
            changed += 1;
        }
    }
    changed
}

/// Scale every listed clip's gain toward the selection's average (bucket-)RMS. Same skip rules as
/// `normalize`. Needs >=2 clips to be meaningful but doesn't enforce it (callers do).
pub fn match_loudness(
    project: &mut Project,
    ids: &[Id],
    peaks_of: &mut dyn FnMut(&Project, Id) -> Option<Arc<Peaks>>,
) -> usize {
    let mut rms_sum = 0.0f64;
    let mut rms_n = 0usize;
    for &id in ids {
        if let Some(clip) = project.clip(id) {
            if let Some(lv) = clip_levels(clip, &mut |asset| peaks_of(project, asset)) {
                rms_sum += lv.rms_db as f64;
                rms_n += 1;
            }
        }
    }
    if rms_n == 0 {
        return 0;
    }
    let target = rms_sum / rms_n as f64;
    normalize(project, ids, target, NormMode::Rms, peaks_of)
}

/// Duck `music`'s volume under every loud (speech) window found on `dialogue`'s peaks, with a ramp.
/// Windows whose ramps overlap are merged before writing. Returns the number of keys written.
pub fn duck(
    project: &mut Project,
    music: Id,
    dialogue: &[Id],
    depth_db: f64,
    ramp_s: f64,
    peaks_of: &mut dyn FnMut(&Project, Id) -> Option<Arc<Peaks>>,
) -> usize {
    let Some(m) = project.clip(music) else { return 0 };
    let (m_start, m_dur) = (m.start, m.duration);
    let base = m.volume.value;

    let mut windows: Vec<(f64, f64)> = Vec::new();
    for &id in dialogue {
        let Some(c) = project.clip(id) else { continue };
        if c.reverse || c.freeze.is_some() {
            continue;
        }
        let (start, src_in, duration, speed) = (c.start, c.src_in, c.duration, c.speed);
        let Some(peaks) = peaks_of(project, c.asset) else { continue };
        let segs = autocut::loud_segments(&peaks, src_in, duration * speed, &AutoCutParams::default());
        let (_, loud) = autocut::to_timeline(&segs, start, src_in, duration, speed, true);
        for (a, b) in loud {
            // into the music clip's local time
            let (a, b) = ((a - m_start).max(0.0), (b - m_start).min(m_dur));
            if b > a {
                windows.push((a, b));
            }
        }
    }
    if windows.is_empty() {
        return 0;
    }
    windows.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for (a, b) in windows {
        match merged.last_mut() {
            Some(last) if a - ramp_s <= last.1 + ramp_s => last.1 = last.1.max(b),
            _ => merged.push((a, b)),
        }
    }

    let volume = match project.clip_mut(music) {
        Some(c) => &mut c.volume,
        None => return 0,
    };
    let mut keys_written = 0;
    seed_key(volume, 0.0, base, &mut keys_written);
    seed_key(volume, m_dur, base, &mut keys_written);
    for (a, b) in merged {
        seed_key(volume, (a - ramp_s).max(0.0), base, &mut keys_written);
        seed_key(volume, a, base * 10f64.powf(depth_db / 20.0), &mut keys_written);
        seed_key(volume, b, base * 10f64.powf(depth_db / 20.0), &mut keys_written);
        seed_key(volume, (b + ramp_s).min(m_dur), base, &mut keys_written);
    }
    keys_written
}

fn seed_key(a: &mut Animated, t: f64, v: f64, count: &mut usize) {
    if !a.has_key_at(t) {
        a.toggle_key(t);
    }
    a.set_at(t, v);
    *count += 1;
}

/// ffmpeg `select='gt(scene,thr)',showinfo` shot-change detection over `path`. Synchronous (blocks
/// the calling thread) — documented ceiling, see the audio-analysis plan's ponytail notes.
/// Returned times are SOURCE seconds.
pub fn scene_cuts(path: &std::path::Path, thr: f32) -> Result<Vec<f64>, String> {
    let exe = crate::media::ffpipe::ffmpeg_exe().ok_or("ffmpeg not found")?;
    let filter = format!("select='gt(scene,{thr})',showinfo");
    let out = crate::media::ffpipe::command(&exe)
        .args(["-i"])
        .arg(path)
        .args(["-vf", &filter, "-f", "null", "-"])
        .output()
        .map_err(|e| e.to_string())?;
    Ok(parse_showinfo_pts(&String::from_utf8_lossy(&out.stderr)))
}

/// Pure detection: onsets on every clip in `targets` (SOURCE seconds, no mutation). Shared by the
/// Beats section's "Detect Beats" button and `audio.beats`'s no-flag path, so a preview never has a
/// side effect — mirrors `audio.beats`' documented "pure detection, no mutation" default. Returns
/// (per-clip onsets, combined BPM across every target's onsets — `None` if fewer than 4 total).
pub fn detect_beats(
    project: &Project,
    targets: &[Id],
    refractory_s: f64,
    sensitivity: f32,
    peaks_of: &mut dyn FnMut(&Project, Id) -> Option<Arc<Peaks>>,
) -> (Vec<(Id, Vec<f64>)>, Option<f64>) {
    let mut per_clip = Vec::new();
    let mut all_onsets = Vec::new();
    for &clip_id in targets {
        let Some(c) = project.clip(clip_id).cloned() else { continue };
        if c.reverse || c.freeze.is_some() {
            continue;
        }
        let Some(peaks) = peaks_of(project, c.asset) else { continue };
        let found = onsets(&peaks, c.src_in, c.src_in + c.duration * c.speed, refractory_s, sensitivity);
        all_onsets.extend(found.iter().copied());
        per_clip.push((clip_id, found));
    }
    all_onsets.sort_by(f64::total_cmp);
    (per_clip, bpm(&all_onsets))
}

/// Add one clip marker per onset in `per_clip` (converted through `to_clip_local` so it lands
/// correctly on a trimmed/retimed clip) — the explicit commit step after `detect_beats`'s preview.
/// Shared by the Beats section's "Add Markers" button, `Action::DetectBeats` and `audio.beats`'s
/// `as_markers` path (this fn stays App-free: `App`'s fields aren't reachable from `ui::autocut_ui`,
/// see the audio-analysis PR's deviation note). Returns the marker ids written.
pub fn beat_markers(project: &mut Project, per_clip: &[(Id, Vec<f64>)]) -> Vec<Id> {
    let mut ids = Vec::new();
    for (clip_id, found) in per_clip {
        let Some(c) = project.clip(*clip_id).cloned() else { continue };
        for &t_src in found {
            if let Some(id) = project.add_clip_marker(*clip_id, to_clip_local(t_src, &c), "Beat") {
                ids.push(id);
            }
        }
    }
    ids
}

/// Split each clip in `per_clip` at its onsets (timeline time via `to_timeline_t`), restricted to
/// that clip's own link group so a beat on one clip never cuts an unrelated clip. Returns the number
/// of cuts made. The explicit commit step after `detect_beats`'s preview.
pub fn split_beats(project: &mut Project, per_clip: &[(Id, Vec<f64>)]) -> usize {
    let mut cuts = 0usize;
    for (clip_id, found) in per_clip {
        let Some(c) = project.clip(*clip_id).cloned() else { continue };
        // `group` grows with each split's new right-half piece (mirrors Project::auto_cut) — a fixed
        // restrict set would only ever cut the ORIGINAL clip, missing every onset past the first cut.
        let mut group = project.expand_links(&[*clip_id]);
        for &t_src in found {
            let new = project.split_at(to_timeline_t(t_src, &c), Some(&group));
            if !new.is_empty() {
                cuts += 1;
                group.extend(new);
            }
        }
    }
    cuts
}

/// Add one point marker (duration 0) per detected scene-cut time (SOURCE seconds), converting through
/// `to_clip_local`. Shared by the Scene cuts section's "Mark instead" button and `media.scene_cuts`'s
/// `as_markers` path. A scene change is an instant, not a range, so this goes through
/// `Project::add_clip_marker` directly rather than `mark_ranges`.
pub fn scene_cut_markers(project: &mut Project, clip: Id, cuts_src: &[f64]) -> Vec<Id> {
    let mut ids = Vec::new();
    let Some(c) = project.clip(clip).cloned() else { return ids };
    if c.reverse || c.freeze.is_some() {
        return ids;
    }
    for &t_src in cuts_src {
        if let Some(id) = project.add_clip_marker(clip, to_clip_local(t_src, &c), "Scene cut") {
            ids.push(id);
        }
    }
    ids
}

/// Split `clip` at each detected scene-cut time (SOURCE seconds), converting through `to_timeline_t`.
/// Shared by the Scene cuts section's "Split" button and `media.scene_cuts`'s `split` path.
pub fn split_scene_cuts(project: &mut Project, clip: Id, cuts_src: &[f64]) -> usize {
    let Some(c) = project.clip(clip).cloned() else { return 0 };
    if c.reverse || c.freeze.is_some() {
        return 0;
    }
    // see split_beats' comment: `group` must grow with each split's new piece.
    let mut group = project.expand_links(&[clip]);
    let mut cuts = 0usize;
    for &t_src in cuts_src {
        let new = project.split_at(to_timeline_t(t_src, &c), Some(&group));
        if !new.is_empty() {
            cuts += 1;
            group.extend(new);
        }
    }
    cuts
}

fn parse_showinfo_pts(stderr: &str) -> Vec<f64> {
    let mut out = Vec::new();
    for line in stderr.lines() {
        if let Some(i) = line.find("pts_time:") {
            let rest = &line[i + "pts_time:".len()..];
            let end = rest.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-')).unwrap_or(rest.len());
            if let Ok(t) = rest[..end].parse::<f64>() {
                out.push(t);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peaks_with_clicks(clicks: &[f64], dur: f64) -> Peaks {
        let n = (dur * PEAKS_PER_SEC as f64) as usize;
        let mut min = vec![-0.01f32; n];
        let mut max = vec![0.01f32; n];
        for &t in clicks {
            let i = (t * PEAKS_PER_SEC as f64) as usize;
            for k in i.saturating_sub(1)..(i + 2).min(n) {
                min[k] = -0.9;
                max[k] = 0.9;
            }
        }
        Peaks { min, max }
    }

    fn flat_peaks(amp: f32, dur: f64) -> Peaks {
        let n = (dur * PEAKS_PER_SEC as f64) as usize;
        Peaks { min: vec![-amp; n], max: vec![amp; n] }
    }

    fn test_clip(start: f64, src_in: f64, duration: f64, speed: f64) -> Clip {
        let mut c = Clip::new(0, crate::model::ClipKind::Audio, "clip", start, duration);
        c.src_in = src_in;
        c.speed = speed;
        c
    }

    #[test]
    fn onsets_finds_synthetic_clicks() {
        let p = peaks_with_clicks(&[0.5, 1.0, 1.5, 2.0], 3.0);
        let got = onsets(&p, 0.0, 3.0, 0.1, 1.6);
        assert_eq!(got.len(), 4, "{got:?}");
        for (g, want) in got.iter().zip([0.5, 1.0, 1.5, 2.0]) {
            assert!((g - want).abs() < 0.03, "{g} vs {want}");
        }
    }

    #[test]
    fn bpm_of_evenly_spaced_onsets_is_120() {
        let onsets: Vec<f64> = (0..8).map(|i| i as f64 * 0.5).collect();
        let b = bpm(&onsets).expect("should detect bpm");
        assert!((118.0..=122.0).contains(&b), "{b}");
        assert!(bpm(&[0.0, 0.5, 1.0]).is_none());
    }

    #[test]
    fn levels_of_known_amplitude() {
        let p = flat_peaks(0.5, 1.0);
        let lv = levels(&p, 0.0, 1.0);
        let want = 20.0 * 0.5f32.log10();
        assert!((lv.peak_db - want).abs() < 0.1, "{lv:?}");
        assert!((lv.rms_db - lv.peak_db).abs() < 0.1, "{lv:?}");
    }

    #[test]
    fn clip_time_conversion_matches_to_timeline_math_for_trimmed_and_retimed_clips() {
        let c = test_clip(5.0, 1.0, 10.0, 2.0);
        assert!((to_clip_local(3.0, &c) - 1.0).abs() < 1e-9);
        assert!((to_timeline_t(3.0, &c) - 6.0).abs() < 1e-9);
    }

    #[test]
    fn xcorr_offset_recovers_known_lag() {
        let a = peaks_with_clicks(&[1.0, 3.0, 5.0], 8.0);
        let b = peaks_with_clicks(&[3.3, 5.3, 7.3], 10.0);
        let lag = xcorr_offset(&a, &b, 5.0).expect("should find a lag");
        assert!((lag - 2.3).abs() < 0.08, "{lag}");
        let empty = Peaks { min: vec![], max: vec![] };
        assert!(xcorr_offset(&empty, &b, 5.0).is_none());
    }

    fn asset_peaks(dur: f64, amp: f32) -> Peaks {
        flat_peaks(amp, dur)
    }

    #[test]
    fn normalize_peak_sets_gain_and_skips_animated_clips() {
        let mut project = Project::default();
        let mut c1 = test_clip(0.0, 0.0, 2.0, 1.0);
        c1.id = 1;
        c1.asset = 1;
        c1.volume = Animated::new(1.0);
        let mut c2 = test_clip(0.0, 0.0, 2.0, 1.0);
        c2.id = 2;
        c2.asset = 2;
        c2.volume = Animated::new(1.0);
        c2.volume.toggle_key(0.0); // animated
        let mut track = crate::model::Track::new(1, crate::model::TrackKind::Audio, "t");
        track.clips = vec![c1, c2];
        project.tracks = vec![track];

        let peaks = asset_peaks(2.0, 0.5);
        let changed = normalize(&mut project, &[1, 2], -1.0, NormMode::Peak, &mut |_p, asset| {
            if asset == 1 {
                Some(Arc::new(Peaks { min: peaks.min.clone(), max: peaks.max.clone() }))
            } else {
                None
            }
        });
        assert_eq!(changed, 1);
        let want_gain = 10f64.powf((-1.0 - (20.0 * 0.5f64.log10())) / 20.0);
        assert!((project.clip(1).unwrap().volume.value - want_gain).abs() < 1e-6);
        assert!(!project.clip(2).unwrap().volume.is_default(1.0)); // untouched, still animated
        assert!(project.clip(2).unwrap().volume.is_animated());
    }

    #[test]
    fn match_loudness_converges_selection_toward_average() {
        let mut project = Project::default();
        let mut c1 = test_clip(0.0, 0.0, 2.0, 1.0);
        c1.id = 1;
        c1.asset = 1;
        c1.volume = Animated::new(1.0);
        let mut c2 = test_clip(0.0, 0.0, 2.0, 1.0);
        c2.id = 2;
        c2.asset = 2;
        c2.volume = Animated::new(1.0);
        let mut c3 = test_clip(0.0, 0.0, 2.0, 1.0);
        c3.id = 3;
        c3.asset = 3; // no peaks available -> skipped
        let mut track = crate::model::Track::new(1, crate::model::TrackKind::Audio, "t");
        track.clips = vec![c1, c2, c3];
        project.tracks = vec![track];

        let changed = match_loudness(&mut project, &[1, 2, 3], &mut |_p, asset| match asset {
            1 => Some(Arc::new(flat_peaks(0.2, 2.0))),
            2 => Some(Arc::new(flat_peaks(0.8, 2.0))),
            _ => None,
        });
        assert_eq!(changed, 2);
        let lv1 = levels(&flat_peaks(0.2, 2.0), 0.0, 2.0).rms_db as f64;
        let lv2 = levels(&flat_peaks(0.8, 2.0), 0.0, 2.0).rms_db as f64;
        let avg = (lv1 + lv2) / 2.0;
        let g1 = 10f64.powf((avg - lv1) / 20.0);
        let g2 = 10f64.powf((avg - lv2) / 20.0);
        assert!((project.clip(1).unwrap().volume.value - g1).abs() < 1e-6);
        assert!((project.clip(2).unwrap().volume.value - g2).abs() < 1e-6);
    }

    fn duck_setup(dialogue_loud: &[(f64, f64)]) -> (Project, Id, Vec<Id>) {
        let mut project = Project::default();
        let mut music = test_clip(0.0, 0.0, 10.0, 1.0);
        music.id = 1;
        music.asset = 1;
        music.volume = Animated::new(0.8);
        let mut dlg = test_clip(0.0, 0.0, 10.0, 1.0);
        dlg.id = 2;
        dlg.asset = 2;
        let mut track = crate::model::Track::new(1, crate::model::TrackKind::Audio, "t");
        track.clips = vec![music, dlg];
        project.tracks = vec![track];
        let _ = dialogue_loud;
        (project, 1, vec![2])
    }

    fn peaks_with_loud_ranges(ranges: &[(f64, f64)], dur: f64) -> Peaks {
        let n = (dur * PEAKS_PER_SEC as f64) as usize;
        let mut min = vec![-0.0001f32; n];
        let mut max = vec![0.0001f32; n];
        for &(a, b) in ranges {
            for i in (a * PEAKS_PER_SEC as f64) as usize..((b * PEAKS_PER_SEC as f64) as usize).min(n) {
                min[i] = -0.5;
                max[i] = 0.5;
            }
        }
        Peaks { min, max }
    }

    #[test]
    fn duck_writes_keys_at_segment_edges_with_ramp() {
        let (mut project, music, dialogue) = duck_setup(&[(2.0, 4.0)]);
        let peaks = peaks_with_loud_ranges(&[(2.0, 4.0)], 10.0);
        let written = duck(&mut project, music, &dialogue, -12.0, 0.2, &mut |_p, _asset| {
            Some(Arc::new(Peaks { min: peaks.min.clone(), max: peaks.max.clone() }))
        });
        assert!(written > 0);
        let vol = &project.clip(music).unwrap().volume;
        assert!(vol.is_animated());
        // loud_segments pads the detected (2.0,4.0) window by AutoCutParams::default()'s 0.1s
        for t in [0.0, 1.7, 1.9, 4.1, 4.3, 10.0] {
            assert!(vol.has_key_at(t), "missing key at {t}");
        }
        assert!((vol.base_at(0.0) - 0.8).abs() < 1e-6);
        assert!(vol.base_at(3.0) < 0.8); // ducked mid-window
    }

    #[test]
    fn duck_writes_keys_with_overlapping_windows_merged() {
        let (mut project, music, dialogue) = duck_setup(&[(1.0, 2.0), (2.3, 3.0)]);
        let peaks = peaks_with_loud_ranges(&[(1.0, 2.0), (2.3, 3.0)], 10.0);
        duck(&mut project, music, &dialogue, -12.0, 0.5, &mut |_p, _asset| {
            Some(Arc::new(Peaks { min: peaks.min.clone(), max: peaks.max.clone() }))
        });
        let vol = &project.clip(music).unwrap().volume;
        let mut times: Vec<f64> = vol.keys.iter().map(|k| k.t).collect();
        times.sort_by(f64::total_cmp);
        for w in times.windows(2) {
            assert!(w[1] > w[0] + 1e-9, "keys not strictly increasing: {times:?}");
        }
    }

    #[test]
    fn parse_showinfo_pts_extracts_times() {
        let sample = "frame=1 pts_time:0.50 x\nsome other line\nframe=2 pts_time:1.25 y\ngarbage";
        assert_eq!(parse_showinfo_pts(sample), vec![0.5, 1.25]);
        assert!(parse_showinfo_pts("").is_empty());
        assert!(parse_showinfo_pts("no matches here").is_empty());
    }

    /// One audio clip (trimmed + retimed: src_in=1.0, speed=2.0, start=5.0) with clicks at source
    /// times 1.5/2.5/3.5/4.5s -> onsets land at clip-local (0.25, 0.75, 1.25, 1.75)s, i.e.
    /// to_clip_local(t_src, clip).
    fn beat_setup() -> (Project, Id) {
        let mut project = Project::default();
        let mut c = test_clip(5.0, 1.0, 5.0, 2.0);
        c.id = 1;
        c.asset = 1;
        c.kind = crate::model::ClipKind::Audio;
        let mut track = crate::model::Track::new(1, crate::model::TrackKind::Audio, "t");
        track.clips = vec![c];
        project.tracks = vec![track];
        (project, 1)
    }

    #[test]
    fn detect_beat_markers_converts_to_clip_local_and_returns_bpm() {
        let (mut project, clip) = beat_setup();
        let clicks = [1.5, 2.5, 3.5, 4.5];
        let peaks = peaks_with_clicks(&clicks, 10.0);
        let (per_clip, bpm) = detect_beats(&project, &[clip], 0.1, 1.6, &mut |_p, _asset| {
            Some(Arc::new(Peaks { min: peaks.min.clone(), max: peaks.max.clone() }))
        });
        let ids = beat_markers(&mut project, &per_clip);
        assert_eq!(ids.len(), 4, "{ids:?}");
        let c = project.clip(clip).unwrap();
        let mut got: Vec<f64> = c.markers.iter().map(|m| m.t).collect();
        got.sort_by(f64::total_cmp);
        for (g, t_src) in got.iter().zip(clicks) {
            let want = to_clip_local(t_src, c);
            assert!((g - want).abs() < 0.03, "{g} vs {want}");
        }
        assert!(bpm.is_some(), "4 evenly-spaced onsets should read a BPM");
    }

    #[test]
    fn split_beats_cuts_only_the_target_clips_link_group() {
        let (mut project, clip) = beat_setup();
        // an unrelated clip on another track must never be touched by this clip's beats
        let mut other = crate::model::Track::new(2, crate::model::TrackKind::Video, "v");
        other.clips.push(crate::model::Clip::new(99, crate::model::ClipKind::Video, "solo", 0.0, 20.0));
        project.tracks.push(other);
        let clicks = [1.5, 2.5, 3.5, 4.5];
        let peaks = peaks_with_clicks(&clicks, 10.0);
        let (per_clip, _bpm) = detect_beats(&project, &[clip], 0.1, 1.6, &mut |_p, _asset| {
            Some(Arc::new(Peaks { min: peaks.min.clone(), max: peaks.max.clone() }))
        });
        let cuts = split_beats(&mut project, &per_clip);
        assert_eq!(cuts, 4, "one cut per onset");
        assert_eq!(project.tracks[1].clips.len(), 1, "the unrelated clip was never split");
        assert!(project.tracks[0].clips.len() > 1, "the target clip was split");
    }

    #[test]
    fn scene_cut_markers_adds_point_markers_at_converted_times() {
        let (mut project, clip) = beat_setup();
        let ids = scene_cut_markers(&mut project, clip, &[1.5, 3.5]);
        assert_eq!(ids.len(), 2);
        let c = project.clip(clip).unwrap();
        for m in &c.markers {
            assert_eq!(m.duration, 0.0, "a scene cut is a point marker, not a range");
        }
        let want0 = to_clip_local(1.5, c);
        assert!(c.markers.iter().any(|m| (m.t - want0).abs() < 1e-9));
    }

    #[test]
    fn split_scene_cuts_splits_at_converted_timeline_times() {
        let (mut project, clip) = beat_setup();
        let cuts = split_scene_cuts(&mut project, clip, &[1.5, 3.5]);
        assert_eq!(cuts, 2);
        assert!(project.tracks[0].clips.len() >= 3, "two cuts make (up to) three pieces");
    }

    /// `to_clip_local`/`to_timeline_t` assume a forward src mapping (`src_in + l`); a reverse or frozen
    /// clip's real mapping is different (see `Clip::src_time`), so `detect_beat_markers` must skip such
    /// clips entirely rather than mirror-flip the onsets onto the wrong clip-local times — same guard
    /// `detect_silence` (tools_audio.rs) already applies before calling into this module.
    #[test]
    fn detect_beat_markers_skips_reverse_and_frozen_clips() {
        let clicks = [1.5, 2.5, 3.5, 4.5];
        let peaks = peaks_with_clicks(&clicks, 10.0);

        let (mut project, clip) = beat_setup();
        project.clip_mut(clip).unwrap().reverse = true;
        let (per_clip, bpm) = detect_beats(&project, &[clip], 0.1, 1.6, &mut |_p, _asset| {
            Some(Arc::new(Peaks { min: peaks.min.clone(), max: peaks.max.clone() }))
        });
        assert!(per_clip.is_empty(), "reverse clip must be skipped, not mirror-flipped: {per_clip:?}");
        assert!(bpm.is_none());

        let (mut project, clip) = beat_setup();
        project.clip_mut(clip).unwrap().freeze = Some(1.0);
        let (per_clip, bpm) = detect_beats(&project, &[clip], 0.1, 1.6, &mut |_p, _asset| {
            Some(Arc::new(Peaks { min: peaks.min.clone(), max: peaks.max.clone() }))
        });
        assert!(per_clip.is_empty(), "frozen clip must be skipped: {per_clip:?}");
        assert!(bpm.is_none());
    }

    /// Each loud-segment endpoint is clamped independently against the music clip's range; a segment
    /// entirely outside `[m_start, m_start+m_dur]` must never seed an inverted (a > b) window into the
    /// volume curve.
    #[test]
    fn duck_skips_window_when_dialogue_loud_segment_is_outside_music_range() {
        let mut project = Project::default();
        let mut music = test_clip(0.0, 0.0, 2.0, 1.0);
        music.id = 1;
        music.asset = 1;
        music.volume = Animated::new(0.8);
        let mut dlg = test_clip(10.0, 0.0, 2.0, 1.0); // timeline [10,12], entirely past music's [0,2]
        dlg.id = 2;
        dlg.asset = 2;
        let mut track = crate::model::Track::new(1, crate::model::TrackKind::Audio, "t");
        track.clips = vec![music, dlg];
        project.tracks = vec![track];

        let peaks = peaks_with_loud_ranges(&[(0.5, 1.5)], 3.0);
        let written = duck(&mut project, 1, &[2], -12.0, 0.2, &mut |_p, _asset| {
            Some(Arc::new(Peaks { min: peaks.min.clone(), max: peaks.max.clone() }))
        });
        assert_eq!(written, 0, "out-of-range loud segment must not seed an inverted window");
        assert!(!project.clip(1).unwrap().volume.is_animated());
    }
}
