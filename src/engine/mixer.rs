//! Audio mixer: sums every audible audio clip at timeline time t into interleaved stereo f32.
//! Used by playback (real-time, block by block) and export (offline to WAV).
//! Handles speed/reverse (linear resampling), freeze (silence), volume/pan/fades (gains lerped across
//! each block), transitions (gain crossfades with virtual clip extension — on video tracks too, so a
//! transition between Sequence clips crossfades their audio with the picture) and Sequence clips on
//! video tracks (their timeline mixed recursively, depth ≤ 8).

use crate::media::{AudioSource, DecoderPool, SAMPLE_RATE};
use crate::model::{Clip, ClipKind, Project, Track, TrackKind, Transition};

const MAX_DEPTH: usize = 8;

/// Transition windows touching a clip: (cut time, clamped half-window, transition, clip is the right
/// side). At most one per clip edge.
type Ext<'a> = [Option<(f64, f64, &'a Transition, bool)>; 2];

pub struct Mixer {
    /// Buffer pool, two per recursion depth: [2d] = source-read/resample buffer, [2d+1] = sequence
    /// sub-mix buffer. Grown once per size increase, never freed.
    scratch: Vec<Vec<f32>>,
}

impl Default for Mixer {
    fn default() -> Self {
        Self::new()
    }
}

impl Mixer {
    pub fn new() -> Self {
        Self { scratch: Vec::new() }
    }

    /// Mix into `out` (interleaved stereo, frames = out.len()/2) starting at timeline time `t`.
    /// `out` is zeroed first. For each audio track with `project.active(track)`, each enabled clip
    /// overlapping [t, t + frames/48000): read `pool.audio(asset.path, clip.audio_stream)` at
    /// `clip.src_time(..)` for the overlapping sub-range, apply `clip.volume` (ramped linearly from the
    /// value at the block start to the block end), add. Clamp the result to [-1, 1].
    pub fn mix(&mut self, project: &Project, t: f64, pool: &mut DecoderPool, out: &mut [f32]) {
        out.fill(0.0);
        if out.len() < 2 {
            return;
        }
        self.mix_tracks(project, &project.tracks, t, pool, out, 0);
        for s in out.iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }
    }

    /// Accumulate (no zeroing, no clamping) the audio of `tracks` over [t, t + out.len()/2 frames).
    fn mix_tracks(
        &mut self,
        project: &Project,
        tracks: &[Track],
        t: f64,
        pool: &mut DecoderPool,
        out: &mut [f32],
        depth: usize,
    ) {
        let frames = out.len() / 2;
        if frames == 0 {
            return;
        }
        let sr = SAMPLE_RATE as f64;
        let t_end = t + frames as f64 / sr;
        for (ti, track) in tracks.iter().enumerate() {
            if !active_in(tracks, ti) {
                continue;
            }
            match track.kind {
                TrackKind::Audio => {
                    for clip in &track.clips {
                        if !clip.enabled || clip.kind != ClipKind::Audio || clip.freeze.is_some() {
                            continue;
                        }
                        let ext = clip_transitions(track, clip);
                        let (estart, eend) = play_range(clip, &ext);
                        if eend <= t || estart >= t_end {
                            continue;
                        }
                        let Some(asset) = project.asset(clip.asset) else {
                            continue;
                        };
                        let i0 = (((estart.max(t) - t) * sr).round() as usize).min(frames);
                        let i1 = (((eend.min(t_end) - t) * sr).round() as usize).min(frames);
                        if i1 <= i0 {
                            continue;
                        }
                        self.mix_audio_clip(
                            clip,
                            &ext,
                            t + i0 as f64 / sr,
                            &asset.path,
                            pool,
                            &mut out[i0 * 2..i1 * 2],
                            depth,
                        );
                    }
                }
                TrackKind::Video => {
                    for clip in &track.clips {
                        if !clip.enabled
                            || clip.kind != ClipKind::Sequence
                            || clip.freeze.is_some()
                            || depth >= MAX_DEPTH
                        {
                            continue;
                        }
                        let ext = clip_transitions(track, clip);
                        let (estart, eend) = play_range(clip, &ext);
                        if eend <= t || estart >= t_end {
                            continue;
                        }
                        let i0 = (((estart.max(t) - t) * sr).round() as usize).min(frames);
                        let i1 = (((eend.min(t_end) - t) * sr).round() as usize).min(frames);
                        if i1 <= i0 || project.sequence_tracks(clip.sequence).is_none() {
                            continue;
                        }
                        self.mix_seq_clip(
                            project,
                            clip,
                            &ext,
                            t + i0 as f64 / sr,
                            pool,
                            &mut out[i0 * 2..i1 * 2],
                            depth,
                        );
                    }
                }
            }
        }
    }

    /// One audio clip: read ceil(n*speed)+1 source frames at the clip's source time, linearly resample
    /// into `out` (n frames) and add with per-channel gains lerped from the block start to the block end.
    fn mix_audio_clip(
        &mut self,
        clip: &Clip,
        ext: &Ext,
        t0: f64,
        path: &str,
        pool: &mut DecoderPool,
        out: &mut [f32],
        depth: usize,
    ) {
        let n = out.len() / 2;
        let m = (n as f64 * clip.speed).ceil() as usize + 1;
        let mut buf = self.take(depth * 2, m * 2);
        let sr = SAMPLE_RATE as f64;
        let s0 = if clip.reverse {
            // the source block ends at src_time(t0) and is walked backwards
            clip.src_time(t0) - (m - 1) as f64 / sr
        } else {
            clip.src_time(t0)
        };
        if let Some(src) = pool.audio(path, clip.audio_stream) {
            read_block(src, s0, &mut buf);
            resample_add(clip, ext, t0, &buf, out);
        }
        self.put(depth * 2, buf);
    }

    /// One Sequence clip on a video track: recursively mix its sequence's tracks at source rate into a
    /// scratch buffer, then treat that buffer exactly like clip source audio (resample + gains).
    #[allow(clippy::too_many_arguments)]
    fn mix_seq_clip(
        &mut self,
        project: &Project,
        clip: &Clip,
        ext: &Ext,
        t0: f64,
        pool: &mut DecoderPool,
        out: &mut [f32],
        depth: usize,
    ) {
        let n = out.len() / 2;
        let m = (n as f64 * clip.speed).ceil() as usize + 1;
        let mut buf = self.take(depth * 2 + 1, m * 2);
        let sr = SAMPLE_RATE as f64;
        let s0 = if clip.reverse { clip.src_time(t0) - (m - 1) as f64 / sr } else { clip.src_time(t0) };
        if let Some(tracks) = project.sequence_tracks(clip.sequence) {
            self.mix_tracks(project, tracks, s0, pool, &mut buf, depth + 1);
            resample_add(clip, ext, t0, &buf, out);
        }
        self.put(depth * 2 + 1, buf);
    }

    /// Take pool buffer `i`, zeroed and sized to `len` (capacity kept — grows once).
    fn take(&mut self, i: usize, len: usize) -> Vec<f32> {
        if self.scratch.len() <= i {
            self.scratch.resize_with(i + 1, Vec::new);
        }
        let mut b = std::mem::take(&mut self.scratch[i]);
        b.clear();
        b.resize(len, 0.0);
        b
    }
    fn put(&mut self, i: usize, b: Vec<f32>) {
        self.scratch[i] = b;
    }
}

/// Mute/solo resolution over an arbitrary track list (same rule as `Project::active`, which only knows
/// the top-level tracks).
fn active_in(tracks: &[Track], i: usize) -> bool {
    let t = &tracks[i];
    let any_solo = tracks.iter().any(|o| o.kind == t.kind && o.solo);
    if any_solo {
        t.solo
    } else {
        !t.muted
    }
}

/// The (still valid) transitions whose window this clip plays in (windows clamped to the cut's clips,
/// so an over-long transition cannot drag a clip past its neighbours — same rule as the compositor).
fn clip_transitions<'a>(track: &'a Track, clip: &Clip) -> Ext<'a> {
    let mut ext = [None, None];
    for tr in &track.transitions {
        let Some((l, r)) = track.transition_pair(tr) else { continue };
        let h = crate::engine::compose::trans_half(tr, l, r);
        if r.id == clip.id {
            ext[0] = Some((r.start, h, tr, true));
        } else if l.id == clip.id {
            ext[1] = Some((r.start, h, tr, false));
        }
    }
    ext
}

/// The clip's audible timeline range: its own extent, virtually extended into transition windows.
fn play_range(clip: &Clip, ext: &Ext) -> (f64, f64) {
    let mut s = clip.start;
    let mut e = clip.end();
    if let Some((cut, h, ..)) = ext[0] {
        s = s.min(cut - h);
    }
    if let Some((cut, h, ..)) = ext[1] {
        e = e.max(cut + h);
    }
    (s, e)
}

/// (left, right) gains at absolute time tt: volume × fade in/out × transition crossfade, panned.
/// Clip-local time is clamped into the clip for volume/pan/fades so virtual extensions hold the edge value.
fn gains(clip: &Clip, ext: &Ext, tt: f64) -> (f32, f32) {
    let lt = clip.local(tt).clamp(0.0, clip.duration);
    let mut g = clip.volume.at(lt) as f32;
    if clip.fade_in > 0.0 && lt < clip.fade_in {
        g *= (lt / clip.fade_in).clamp(0.0, 1.0) as f32;
    }
    if clip.fade_out > 0.0 && lt > clip.duration - clip.fade_out {
        g *= ((clip.duration - lt) / clip.fade_out).clamp(0.0, 1.0) as f32;
    }
    for e in ext.iter().flatten() {
        let (cut, h, tr, is_right) = *e;
        if tt >= cut - h && tt < cut + h {
            let p = crate::engine::compose::trans_progress_at(tr, cut, h, tt) as f32;
            g *= if is_right { p } else { 1.0 - p };
        }
    }
    let pan = clip.pan.at(lt).clamp(-1.0, 1.0) as f32;
    (g * (1.0 - pan).min(1.0), g * (1.0 + pan).min(1.0))
}

/// Fill `buf` from `src` starting at source time `s0`; time before 0 is silence.
fn read_block(src: &mut dyn AudioSource, s0: f64, buf: &mut [f32]) {
    if s0 >= 0.0 {
        src.read_at(s0, buf);
        return;
    }
    let skip = (((-s0) * SAMPLE_RATE as f64).round() as usize) * 2;
    if skip >= buf.len() {
        buf.fill(0.0);
        return;
    }
    buf[..skip].fill(0.0);
    src.read_at(0.0, &mut buf[skip..]);
}

/// Linearly resample `buf` (m source frames read at the clip's source time for `t0`) into `out`
/// (n frames) and add with gains lerped across the block.
/// ponytail: gains (volume keys, fades, transition ease) are sampled at the block ends and lerped —
/// exact for linear ramps, ≤ one block (≈100 ms) of shape error otherwise; split at kinks if it matters.
fn resample_add(clip: &Clip, ext: &Ext, t0: f64, buf: &[f32], out: &mut [f32]) {
    let n = out.len() / 2;
    let m = buf.len() / 2;
    if n == 0 || m == 0 {
        return;
    }
    let (l0, r0) = gains(clip, ext, t0);
    let (l1, r1) = gains(clip, ext, t0 + n as f64 / SAMPLE_RATE as f64);
    let dl = (l1 - l0) / n as f32;
    let dr = (r1 - r0) / n as f32;
    let last = m - 1;
    for (k, o) in out.chunks_exact_mut(2).enumerate() {
        let pos = if clip.reverse { last as f64 - k as f64 * clip.speed } else { k as f64 * clip.speed };
        let pos = pos.max(0.0);
        let j = (pos as usize).min(last);
        let j1 = (j + 1).min(last);
        let fr = (pos - j as f64) as f32;
        let a = &buf[j * 2..j * 2 + 2];
        let b = &buf[j1 * 2..j1 * 2 + 2];
        o[0] += (a[0] + (b[0] - a[0]) * fr) * (l0 + dl * k as f32);
        o[1] += (a[1] + (b[1] - a[1]) * fr) * (r0 + dr * k as f32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::{AudioSource, Backend};
    use crate::model::{Asset, AudioStreamInfo, ClipKind, Ease, TransitionKind};

    struct Const(f32);
    impl AudioSource for Const {
        fn duration(&self) -> f64 {
            10.0
        }
        fn read_at(&mut self, t: f64, out: &mut [f32]) {
            for (i, s) in out.iter_mut().enumerate() {
                let tt = t + (i / 2) as f64 / SAMPLE_RATE as f64;
                *s = if (0.0..10.0).contains(&tt) { self.0 } else { 0.0 };
            }
        }
    }

    /// Sample value == source time × 0.01 (stays inside [-1, 1] for 10 s media).
    struct Ramp;
    impl AudioSource for Ramp {
        fn duration(&self) -> f64 {
            10.0
        }
        fn read_at(&mut self, t: f64, out: &mut [f32]) {
            for (i, s) in out.iter_mut().enumerate() {
                let tt = t + (i / 2) as f64 / SAMPLE_RATE as f64;
                *s = if (0.0..10.0).contains(&tt) { (tt * 0.01) as f32 } else { 0.0 };
            }
        }
    }

    fn audio_asset(id: crate::model::Id, path: &str) -> Asset {
        Asset {
            id,
            path: path.into(),
            kind: ClipKind::Audio,
            duration: 10.0,
            width: 0,
            height: 0,
            fps: 0.0,
            audio_streams: vec![AudioStreamInfo { index: 0, channels: 2, sample_rate: 48000, ..Default::default() }],
            codec: "pcm".into(),
            folder: String::new(),
            tags: Vec::new(),
            label: 0,
            description: String::new(),
        }
    }

    fn project() -> Project {
        Project::from_media(audio_asset(0, "Z:\\nope\\fake.wav"))
    }

    fn pool() -> DecoderPool {
        let mut p = DecoderPool::new(Backend::Ffmpeg);
        p.insert_audio("Z:\\nope\\fake.wav", 0, Box::new(Const(0.5)));
        p
    }

    #[test]
    fn volume_mute_solo() {
        let mut p = project();
        let ai = p.audio_tracks()[0];
        p.tracks[ai].clips[0].volume.value = 0.5;
        let mut pool = pool();
        let mut mx = Mixer::new();
        let mut out = vec![1.0f32; 2048];
        mx.mix(&p, 1.0, &mut pool, &mut out);
        assert!(out.iter().all(|s| (s - 0.25).abs() < 1e-5), "{:?}", &out[..4]);

        // muted track → silence
        p.tracks[ai].muted = true;
        mx.mix(&p, 1.0, &mut pool, &mut out);
        assert!(out.iter().all(|s| *s == 0.0));
        p.tracks[ai].muted = false;

        // solo on another audio track silences this one; solo on this one keeps it
        let other = p.add_track(TrackKind::Audio);
        p.tracks[other].solo = true;
        mx.mix(&p, 1.0, &mut pool, &mut out);
        assert!(out.iter().all(|s| *s == 0.0));
        p.tracks[ai].solo = true;
        mx.mix(&p, 1.0, &mut pool, &mut out);
        assert!(out.iter().all(|s| (s - 0.25).abs() < 1e-5));

        // outside the clip → silence; clip ending mid-block → partial
        mx.mix(&p, 20.0, &mut pool, &mut out);
        assert!(out.iter().all(|s| *s == 0.0));
        let mut out = vec![0.0f32; 96]; // 48 frames = 1 ms
        mx.mix(&p, 10.0 - 0.0005, &mut pool, &mut out);
        assert!((out[0] - 0.25).abs() < 1e-5);
        assert_eq!(out[95], 0.0);
    }

    #[test]
    fn ramp_and_clamp() {
        let mut p = project();
        let ai = p.audio_tracks()[0];
        let c = &mut p.tracks[ai].clips[0];
        c.volume.keys = vec![
            crate::model::Keyframe { t: 0.0, v: 0.0, ease: Ease::Linear },
            crate::model::Keyframe { t: 1.0, v: 2.0, ease: Ease::Linear },
        ];
        let mut pool = pool();
        let mut mx = Mixer::new();
        let mut out = vec![0.0f32; 2 * 4800]; // 100 ms block from t=0 → volume 0 → 0.2 → samples 0 → 0.1
        mx.mix(&p, 0.0, &mut pool, &mut out);
        assert!(out[0].abs() < 1e-5);
        assert!((out[out.len() - 2] - 0.1).abs() < 1e-3, "{}", out[out.len() - 2]);
        assert!(out[2400 * 2] > out[1200 * 2]);
        // volume 2 → 1.0 → clamped at 1.0 on a 0.5 source? 0.5*2 = 1.0 exactly; use 4x to force clamp
        p.tracks[ai].clips[0].volume = crate::model::Animated::new(4.0);
        mx.mix(&p, 0.5, &mut pool, &mut out);
        assert!(out.iter().all(|s| *s == 1.0));
    }

    #[test]
    fn speed_reverse_freeze() {
        let mut p = project();
        let ai = p.audio_tracks()[0];
        let mut pool = DecoderPool::new(Backend::Ffmpeg);
        pool.insert_audio("Z:\\nope\\fake.wav", 0, Box::new(Ramp));
        let mut mx = Mixer::new();
        let mut out = vec![0.0f32; 2 * 480]; // 10 ms
        let sr = SAMPLE_RATE as f64;

        // speed 2: at t=1 the source time is 2, advancing 2× per output frame
        p.tracks[ai].clips[0].set_speed(2.0);
        assert!((p.tracks[ai].clips[0].duration - 5.0).abs() < 1e-9);
        mx.mix(&p, 1.0, &mut pool, &mut out);
        assert!((out[0] - 0.02).abs() < 1e-4, "{}", out[0]);
        let k = 400;
        let want = (2.0 + 2.0 * k as f64 / sr) * 0.01;
        assert!((out[k * 2] as f64 - want).abs() < 1e-4, "{} vs {want}", out[k * 2]);

        // reverse at speed 1: src_time(t) = 10 - t → decreasing ramp
        let c = &mut p.tracks[ai].clips[0];
        c.set_speed(1.0);
        c.reverse = true;
        mx.mix(&p, 1.0, &mut pool, &mut out);
        assert!((out[0] - 0.09).abs() < 1e-4, "{}", out[0]);
        let want = (9.0 - k as f64 / sr) * 0.01;
        assert!((out[k * 2] as f64 - want).abs() < 1e-4);
        assert!(out[0] > out[k * 2]);

        // freeze → silence
        p.tracks[ai].clips[0].freeze = Some(3.0);
        mx.mix(&p, 1.0, &mut pool, &mut out);
        assert!(out.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn pan_and_fades() {
        let mut p = project();
        let ai = p.audio_tracks()[0];
        let mut pool = pool();
        let mut mx = Mixer::new();
        let mut out = vec![0.0f32; 2 * 480];

        // pan 0.5 → L × 0.5, R × 1
        p.tracks[ai].clips[0].pan.value = 0.5;
        mx.mix(&p, 1.0, &mut pool, &mut out);
        assert!((out[0] - 0.25).abs() < 1e-5, "{}", out[0]);
        assert!((out[1] - 0.5).abs() < 1e-5, "{}", out[1]);
        // pan -1 → L × 1, R × 0
        p.tracks[ai].clips[0].pan.value = -1.0;
        mx.mix(&p, 1.0, &mut pool, &mut out);
        assert!((out[0] - 0.5).abs() < 1e-5);
        assert!(out[1].abs() < 1e-5);
        p.tracks[ai].clips[0].pan.value = 0.0;

        // fade_in 2 s: gain 0.5 at t=1; fade_out 2 s: gain 0.25 at t=9.5 (clip is 10 s)
        let c = &mut p.tracks[ai].clips[0];
        c.fade_in = 2.0;
        c.fade_out = 2.0;
        mx.mix(&p, 1.0, &mut pool, &mut out);
        assert!((out[0] - 0.25).abs() < 1e-3, "{}", out[0]);
        mx.mix(&p, 9.5, &mut pool, &mut out);
        assert!((out[0] - 0.125).abs() < 1e-3, "{}", out[0]);
        // edges are silent
        mx.mix(&p, 0.0, &mut pool, &mut out);
        assert!(out[0].abs() < 1e-3);
    }

    #[test]
    fn transition_crossfade() {
        // A1: Const(0.8) on [0,5) then Const(0.4) on [5,10), CrossFade of 2 s at the cut.
        let mut p = Project::new();
        let a = audio_asset(0, "Z:\\nope\\a.wav");
        let b = audio_asset(0, "Z:\\nope\\b.wav");
        let a = p.add_asset(a);
        let b = p.add_asset(b);
        let ai = p.audio_tracks()[0];
        let mut c1 = Clip::new(1000, ClipKind::Audio, "a", 0.0, 5.0);
        c1.asset = a;
        let mut c2 = Clip::new(1001, ClipKind::Audio, "b", 5.0, 5.0);
        c2.asset = b;
        c2.src_in = 5.0; // pre-roll into the transition window reads source [4, 5)
        let right = c2.id;
        p.tracks[ai].clips.push(c1);
        p.tracks[ai].clips.push(c2);
        p.tracks[ai].transitions.push(Transition {
            id: 2000,
            right,
            kind: TransitionKind::CrossFade,
            duration: 2.0,
            color: [0, 0, 0, 255],
            direction: 0,
            ease: Ease::Linear,
        });
        let mut pool = DecoderPool::new(Backend::Ffmpeg);
        pool.insert_audio("Z:\\nope\\a.wav", 0, Box::new(Const(0.8)));
        pool.insert_audio("Z:\\nope\\b.wav", 0, Box::new(Const(0.4)));
        let mut mx = Mixer::new();
        let mut out = vec![0.0f32; 2 * 480];
        // outside the window
        mx.mix(&p, 2.0, &mut pool, &mut out);
        assert!((out[0] - 0.8).abs() < 1e-4, "{}", out[0]);
        mx.mix(&p, 8.0, &mut pool, &mut out);
        assert!((out[0] - 0.4).abs() < 1e-4);
        // window is [4, 6): p=0.25 at 4.5 → 0.8·0.75 + 0.4·0.25 = 0.7 (right clip plays before its start)
        mx.mix(&p, 4.5, &mut pool, &mut out);
        assert!((out[0] - 0.7).abs() < 1e-3, "{}", out[0]);
        // p=0.75 at 5.5 → 0.8·0.25 + 0.4·0.75 = 0.5 (left clip extended past its end)
        mx.mix(&p, 5.5, &mut pool, &mut out);
        assert!((out[0] - 0.5).abs() < 1e-3, "{}", out[0]);
        // exactly at the cut: p=0.5 → 0.6
        mx.mix(&p, 5.0, &mut pool, &mut out);
        assert!((out[0] - 0.6).abs() < 1e-3, "{}", out[0]);
    }

    /// A transition of `dur` on the cut between two clips already pushed on `ti`.
    fn add_transition(p: &mut Project, ti: usize, right: crate::model::Id, dur: f64) {
        p.tracks[ti].transitions.push(Transition {
            id: 2000,
            right,
            kind: TransitionKind::CrossFade,
            duration: dur,
            color: [0, 0, 0, 255],
            direction: 0,
            ease: Ease::Linear,
        });
    }

    #[test]
    fn transition_window_clamped_to_clips() {
        // A [0,5) then a 1 s B, with an 8 s transition: the window is clamped to B → [4,6), so nothing
        // plays after 6 (an unclamped window would extend both clips out to 9).
        let mut p = Project::new();
        let a = p.add_asset(audio_asset(0, "Z:\\nope\\a.wav"));
        let b = p.add_asset(audio_asset(0, "Z:\\nope\\b.wav"));
        let ai = p.audio_tracks()[0];
        let mut c1 = Clip::new(1000, ClipKind::Audio, "a", 0.0, 5.0);
        c1.asset = a;
        let mut c2 = Clip::new(1001, ClipKind::Audio, "b", 5.0, 1.0);
        c2.asset = b;
        c2.src_in = 5.0;
        let right = c2.id;
        p.tracks[ai].clips.push(c1);
        p.tracks[ai].clips.push(c2);
        add_transition(&mut p, ai, right, 8.0);
        let mut pool = DecoderPool::new(Backend::Ffmpeg);
        pool.insert_audio("Z:\\nope\\a.wav", 0, Box::new(Const(0.8)));
        pool.insert_audio("Z:\\nope\\b.wav", 0, Box::new(Const(0.4)));
        let mut mx = Mixer::new();
        let mut out = vec![0.0f32; 2 * 480];
        mx.mix(&p, 2.0, &mut pool, &mut out);
        assert!((out[0] - 0.8).abs() < 1e-4, "before the clamped window: {}", out[0]);
        mx.mix(&p, 5.0, &mut pool, &mut out);
        assert!((out[0] - 0.6).abs() < 1e-3, "at the cut: {}", out[0]);
        mx.mix(&p, 7.0, &mut pool, &mut out);
        assert!(out.iter().all(|s| *s == 0.0), "audio past both clips: {}", out[0]);
    }

    #[test]
    fn sequence_transition_crossfades_audio() {
        // Two sequence clips on V1 with a 2 s CrossFade: their audio must dissolve with the picture.
        let mut p = Project::new();
        let mut seq = Vec::new();
        for (i, path) in ["Z:\\nope\\a.wav", "Z:\\nope\\b.wav"].into_iter().enumerate() {
            let asset = p.add_asset(audio_asset(0, path));
            let s = p.new_sequence("s", 320, 240, 30.0);
            let sq = p.sequence_mut(s).unwrap();
            let sai = sq.tracks.iter().position(|t| t.kind == TrackKind::Audio).unwrap();
            let mut inner = Clip::new(500 + i as crate::model::Id, ClipKind::Audio, "in", 0.0, 10.0);
            inner.asset = asset;
            sq.tracks[sai].clips.push(inner);
            seq.push(s);
        }
        let vi = p.video_tracks()[0];
        for (i, s) in seq.iter().enumerate() {
            let mut c = Clip::new(1000 + i as crate::model::Id, ClipKind::Sequence, "sc", i as f64 * 5.0, 5.0);
            c.sequence = *s;
            c.src_in = i as f64 * 5.0;
            p.tracks[vi].clips.push(c);
        }
        add_transition(&mut p, vi, 1001, 2.0);
        let mut pool = DecoderPool::new(Backend::Ffmpeg);
        pool.insert_audio("Z:\\nope\\a.wav", 0, Box::new(Const(0.8)));
        pool.insert_audio("Z:\\nope\\b.wav", 0, Box::new(Const(0.4)));
        let mut mx = Mixer::new();
        let mut out = vec![0.0f32; 2 * 480];
        // window [4,6): 0.25 in → 0.8·0.75 + 0.4·0.25 = 0.7 (was a hard cut at t=5)
        mx.mix(&p, 4.5, &mut pool, &mut out);
        assert!((out[0] - 0.7).abs() < 1e-3, "{}", out[0]);
        mx.mix(&p, 5.5, &mut pool, &mut out);
        assert!((out[0] - 0.5).abs() < 1e-3, "{}", out[0]);
        // outside the window each sequence plays alone
        mx.mix(&p, 2.0, &mut pool, &mut out);
        assert!((out[0] - 0.8).abs() < 1e-4, "{}", out[0]);
        mx.mix(&p, 8.0, &mut pool, &mut out);
        assert!((out[0] - 0.4).abs() < 1e-4, "{}", out[0]);
    }

    #[test]
    fn sequence_audio() {
        // Sequence with a Ramp audio clip [0,4); placed on V1 at t=1.
        let mut p = Project::new();
        let aid = p.add_asset(audio_asset(0, "Z:\\nope\\fake.wav"));
        let seq = p.new_sequence("s", 320, 240, 30.0);
        let mut inner = Clip::new(500, ClipKind::Audio, "in", 0.0, 4.0);
        inner.asset = aid;
        let s = p.sequence_mut(seq).unwrap();
        let sai = s.tracks.iter().position(|t| t.kind == TrackKind::Audio).unwrap();
        s.tracks[sai].clips.push(inner);
        let sc = p.insert_sequence_clip(seq, 1.0, None).expect("placed");
        let mut pool = DecoderPool::new(Backend::Ffmpeg);
        pool.insert_audio("Z:\\nope\\fake.wav", 0, Box::new(Ramp));
        let mut mx = Mixer::new();
        let mut out = vec![0.0f32; 2 * 480];
        // t=1.5 → sequence-local 0.5 → ramp value 0.005
        mx.mix(&p, 1.5, &mut pool, &mut out);
        assert!((out[0] - 0.005).abs() < 1e-4, "{}", out[0]);
        // clip volume/pan apply
        {
            let c = p.clip_mut(sc).unwrap();
            c.volume.value = 0.5;
            c.pan.value = 1.0;
        }
        mx.mix(&p, 1.5, &mut pool, &mut out);
        assert!(out[0].abs() < 1e-5, "L muted by pan, got {}", out[0]);
        assert!((out[1] - 0.0025).abs() < 1e-4, "{}", out[1]);
        {
            let c = p.clip_mut(sc).unwrap();
            c.volume.value = 1.0;
            c.pan.value = 0.0;
        }
        // speed 2 on the sequence clip: at t=1.5 sequence-local source time = 1.0
        p.clip_mut(sc).unwrap().set_speed(2.0);
        mx.mix(&p, 1.5, &mut pool, &mut out);
        assert!((out[0] - 0.01).abs() < 1e-4, "{}", out[0]);
        p.clip_mut(sc).unwrap().set_speed(1.0);
        // before the sequence clip → silence; frozen → silence; hidden video track → silence
        mx.mix(&p, 0.5, &mut pool, &mut out);
        assert!(out.iter().all(|s| *s == 0.0));
        p.clip_mut(sc).unwrap().freeze = Some(1.0);
        mx.mix(&p, 1.5, &mut pool, &mut out);
        assert!(out.iter().all(|s| *s == 0.0));
        p.clip_mut(sc).unwrap().freeze = None;
        let vi = p.video_tracks()[0];
        p.tracks[vi].muted = true;
        mx.mix(&p, 1.5, &mut pool, &mut out);
        assert!(out.iter().all(|s| *s == 0.0));
    }
}
