//! Audio mixer: sums every audible audio clip at timeline time t into interleaved stereo f32.
//! Used by playback (real-time, block by block) and export (offline to WAV).
//! Handles speed/reverse (linear resampling), freeze (silence), volume/pan/fades (gains lerped across
//! each block), transitions (gain crossfades with virtual clip extension) and Sequence clips (their
//! timeline mixed recursively, depth ≤ 8). Only audio tracks sound: a nested sequence plays from its
//! Sequence clip on an audio track (the linked video-track twin is picture-only), so it is heard once
//! and mutes, cuts and routes with that audio track. Export mixes through this same code.
//!
//! Routing: when the project has buses, every top-level clip's contribution lands in
//! `Project::bus_of(track, clip)` instead of straight in the output, then `BusGraph` flushes the buses
//! leaves-first (filters → gain/pan/mono → sum into the output bus) with Main summing into `out`.
//! Bus mute/solo mirrors track mute/solo - any solo among the buses silences the un-soloed ones, except
//! Main, which is the master everything sums through. Projects with no buses (the default) skip all of
//! that and mix straight into `out`.

use crate::engine::mixer_fx::BusGraph;
use crate::media::{AudioSource, DecoderPool, SAMPLE_RATE};
use crate::model::{Clip, ClipKind, Id, Project, Track, TrackKind, Transition};

const MAX_DEPTH: usize = 8;

/// Transition windows touching a clip: (cut time, clamped half-window, transition, clip is the right
/// side). At most one per clip edge.
type Ext<'a> = [Option<(f64, f64, &'a Transition, bool)>; 2];

/// Where a clip's samples go: a plain buffer (no buses / a nested sequence sub-mix) or the bus graph.
enum Dest<'a> {
    Buf(&'a mut [f32]),
    /// The graph plus the block length in frames (bus buffers are all that long).
    Buses(&'a mut BusGraph, usize),
}

impl Dest<'_> {
    fn frames(&self) -> usize {
        match self {
            Dest::Buf(b) => b.len() / 2,
            Dest::Buses(_, f) => *f,
        }
    }
    /// True when `slice`'s `bus` argument matters (only the top level routes).
    fn routed(&self) -> bool {
        matches!(self, Dest::Buses(..))
    }
    /// The [i0, i1) frame window of the buffer a clip on `bus` writes into.
    fn slice(&mut self, bus: Id, i0: usize, i1: usize) -> &mut [f32] {
        match self {
            Dest::Buf(b) => &mut b[i0 * 2..i1 * 2],
            Dest::Buses(g, frames) => &mut g.buffer(bus, *frames)[i0 * 2..i1 * 2],
        }
    }
}

/// Buffer pool, two per recursion depth: [2d] = source-read/resample buffer, [2d+1] = sequence
/// sub-mix buffer. Grown once per size increase, never freed.
#[derive(Default)]
struct Scratch(Vec<Vec<f32>>, Voices);

/// The (asset, audio stream) of every audio clip mixed so far in this block. A clip's voice is how
/// many earlier ones read the same stream: clips that sound together each get a decoder of their own
/// (`DecoderPool::audio`), and a clip alone on its file is voice 0 - cuts of one file share a decoder.
/// ponytail: a clip that starts under a later-ordered one of the same file takes over its voice, at
/// one seek each; and the same file imported twice counts as two.
type Voices = Vec<(Id, usize)>;

impl Scratch {
    /// Take pool buffer `i`, zeroed and sized to `len` (capacity kept - grows once).
    fn take(&mut self, i: usize, len: usize) -> Vec<f32> {
        if self.0.len() <= i {
            self.0.resize_with(i + 1, Vec::new);
        }
        let mut b = std::mem::take(&mut self.0[i]);
        b.clear();
        b.resize(len, 0.0);
        b
    }
    fn put(&mut self, i: usize, b: Vec<f32>) {
        self.0[i] = b;
    }
}

#[derive(Default)]
pub struct Mixer {
    scratch: Scratch,
    graph: BusGraph,
    /// `graph.order()` copied once per block so the flush loop can hold `&mut graph`.
    order: Vec<Id>,
}

impl Mixer {
    pub fn new() -> Self {
        Self::default()
    }

    /// The bus graph as of the last `mix` - the mixer panel reads its meters from here.
    pub fn graph(&self) -> &BusGraph {
        &self.graph
    }

    /// Mix into `out` (interleaved stereo, frames = out.len()/2) starting at timeline time `t`.
    /// `out` is zeroed first. For each audio track with `project.active(track)`, each enabled clip
    /// overlapping [t, t + frames/48000): read `pool.audio(asset.path, clip.audio_stream, voice)` at
    /// `clip.src_time(..)` for the overlapping sub-range, apply `clip.volume` (ramped linearly from the
    /// value at the block start to the block end), add into the clip's bus. Buses are then flushed in
    /// evaluation order and the result clamped to [-1, 1].
    pub fn mix(&mut self, project: &Project, t: f64, pool: &mut DecoderPool, out: &mut [f32]) {
        out.fill(0.0);
        if out.len() < 2 {
            return;
        }
        let Mixer { scratch, graph, order } = self;
        scratch.1.clear();
        if project.buses.is_empty() {
            mix_tracks(scratch, project, &project.tracks, t, pool, &mut Dest::Buf(out), 0);
        } else {
            let frames = out.len() / 2;
            graph.sync(project);
            graph.begin(frames);
            mix_tracks(scratch, project, &project.tracks, t, pool, &mut Dest::Buses(graph, frames), 0);
            order.clear();
            order.extend_from_slice(graph.order_ref());
            for &id in order.iter() {
                if let Some(bus) = project.bus(id) {
                    graph.flush(bus, t, out);
                }
            }
        }
        for s in out.iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }
    }
}

/// Accumulate (no zeroing, no clamping) the audio of `tracks` over [t, t + dest.frames() frames).
fn mix_tracks(
    scratch: &mut Scratch,
    project: &Project,
    tracks: &[Track],
    t: f64,
    pool: &mut DecoderPool,
    dest: &mut Dest,
    depth: usize,
) {
    let frames = dest.frames();
    if frames == 0 {
        return;
    }
    let sr = SAMPLE_RATE as f64;
    let t_end = t + frames as f64 / sr;
    for (ti, track) in tracks.iter().enumerate() {
        // video tracks never sound: a Sequence clip there is the picture half of a linked pair
        if track.kind != TrackKind::Audio || !active_in(tracks, ti) {
            continue;
        }
        // ---- ws:registries-schema-hooks ----
        // ---- ws:audio-dsp-automation ----
        // Track volume automation, sampled once at the block start and held for the block.
        // ponytail: not lerped across the block like clip gain - a keyed ramp steps every ≈21 ms;
        // sample it at both block ends in `resample_add` if the steps ever become audible.
        let tg = track.volume.at(t) as f32;
        for clip in &track.clips {
            if !clip.enabled || clip.freeze.is_some() {
                continue;
            }
            let ext = clip_transitions(track, clip);
            let (estart, eend) = play_range(clip, &ext);
            if eend <= t || estart >= t_end {
                continue;
            }
            let i0 = (((estart.max(t) - t) * sr).round() as usize).min(frames);
            let i1 = (((eend.min(t_end) - t) * sr).round() as usize).min(frames);
            if i1 <= i0 {
                continue;
            }
            // an Audio clip reads its asset; a Sequence clip (None) sub-mixes its nested timeline
            let path = match clip.kind {
                ClipKind::Audio => match project.asset(clip.asset) {
                    Some(a) => Some(&a.path),
                    None => continue,
                },
                ClipKind::Sequence if depth < MAX_DEPTH && project.sequence_tracks(clip.sequence).is_some() => None,
                _ => continue,
            };
            let bus = if dest.routed() { project.bus_of(ti, clip) } else { 0 };
            let t0 = t + i0 as f64 / sr;
            let out = dest.slice(bus, i0, i1);
            match path {
                Some(path) => mix_audio_clip(scratch, clip, &ext, t0, path, pool, out, depth, tg),
                None => mix_seq_clip(scratch, project, clip, &ext, t0, pool, out, depth, tg),
            }
        }
    }
}

/// One audio clip: read ceil(n*speed)+1 source frames at the clip's source time, linearly resample
/// into `out` (n frames) and add with per-channel gains lerped from the block start to the block end.
#[allow(clippy::too_many_arguments)]
fn mix_audio_clip(
    scratch: &mut Scratch,
    clip: &Clip,
    ext: &Ext,
    t0: f64,
    path: &str,
    pool: &mut DecoderPool,
    out: &mut [f32],
    depth: usize,
    track_gain: f32,
) {
    let n = out.len() / 2;
    let m = (n as f64 * clip.speed).ceil() as usize + 1;
    let mut buf = scratch.take(depth * 2, m * 2);
    let sr = SAMPLE_RATE as f64;
    let s0 = if clip.reverse {
        // the source block ends at src_time(t0) and is walked backwards
        clip.src_time(t0) - (m - 1) as f64 / sr
    } else {
        clip.src_time(t0)
    };
    let key = (clip.asset, clip.audio_stream);
    let voice = scratch.1.iter().filter(|v| **v == key).count();
    scratch.1.push(key);
    if let Some(src) = pool.audio(path, clip.audio_stream, voice) {
        read_block(src, s0, &mut buf);
        resample_add(clip, ext, t0, &buf, out, track_gain);
    }
    scratch.put(depth * 2, buf);
}

/// One Sequence clip on an audio track: recursively mix its sequence's tracks at source rate into a
/// scratch buffer, then treat that buffer exactly like clip source audio (resample + gains).
/// Nested tracks keep their own mute/solo but not their own buses - the whole sub-mix goes to the
/// bus of the Sequence clip that hosts it.
#[allow(clippy::too_many_arguments)]
fn mix_seq_clip(
    scratch: &mut Scratch,
    project: &Project,
    clip: &Clip,
    ext: &Ext,
    t0: f64,
    pool: &mut DecoderPool,
    out: &mut [f32],
    depth: usize,
    track_gain: f32,
) {
    let n = out.len() / 2;
    let m = (n as f64 * clip.speed).ceil() as usize + 1;
    let mut buf = scratch.take(depth * 2 + 1, m * 2);
    let sr = SAMPLE_RATE as f64;
    let s0 = if clip.reverse { clip.src_time(t0) - (m - 1) as f64 / sr } else { clip.src_time(t0) };
    if let Some(tracks) = project.sequence_tracks(clip.sequence) {
        mix_tracks(scratch, project, tracks, s0, pool, &mut Dest::Buf(&mut buf), depth + 1);
        resample_add(clip, ext, t0, &buf, out, track_gain);
    }
    scratch.put(depth * 2 + 1, buf);
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
/// so an over-long transition cannot drag a clip past its neighbours - same rule as the compositor).
fn clip_transitions<'a>(track: &'a Track, clip: &Clip) -> Ext<'a> {
    let mut ext = [None, None];
    for tr in &track.transitions {
        let Some((l, r)) = track.transition_clips(tr) else { continue };
        let Some((cut, h)) = tr.cut_half(l, r) else { continue };
        // right side gains in (a cut's incoming clip, or an In edge), left side gains out
        if r.is_some_and(|r| r.id == clip.id) {
            ext[0] = Some((cut, h, tr, true));
        } else if l.is_some_and(|l| l.id == clip.id) {
            ext[1] = Some((cut, h, tr, false));
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
    let mut g = clip.volume.at(lt) as f32 * clip.fade_mult(lt) as f32;
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
/// ponytail: gains (volume keys, fades, transition ease) are sampled at the block ends and lerped -
/// exact for linear ramps, ≤ one block of shape error otherwise; split at kinks if it matters. Both
/// callers use 1024-frame blocks (≈21 ms: `playback::BLOCK` and `export::MIX_BLOCK`), so playback and
/// export shape a fade identically.
/// `track_gain` (ws:audio-dsp-automation: the hosting track's `volume` at the block start) scales
/// both channels on top of the clip's own gains.
fn resample_add(clip: &Clip, ext: &Ext, t0: f64, buf: &[f32], out: &mut [f32], track_gain: f32) {
    let n = out.len() / 2;
    let m = buf.len() / 2;
    if n == 0 || m == 0 {
        return;
    }
    let (l0, r0) = gains(clip, ext, t0);
    let (l1, r1) = gains(clip, ext, t0 + n as f64 / SAMPLE_RATE as f64);
    let (l0, r0, l1, r1) = (l0 * track_gain, r0 * track_gain, l1 * track_gain, r1 * track_gain);
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

    // ---- ws:audio-dsp-automation ----
    /// A real 440 Hz tone at amplitude `.0` - unlike `Const`, this has AC content, so K-weighted LUFS
    /// (which high-passes out DC) reads something other than silence for it.
    struct Sine(f32);
    impl AudioSource for Sine {
        fn duration(&self) -> f64 {
            10.0
        }
        fn read_at(&mut self, t: f64, out: &mut [f32]) {
            for (i, s) in out.iter_mut().enumerate() {
                let tt = t + (i / 2) as f64 / SAMPLE_RATE as f64;
                *s = if (0.0..10.0).contains(&tt) {
                    self.0 * (std::f64::consts::TAU * 440.0 * tt).sin() as f32
                } else {
                    0.0
                };
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
            rel_path: None,
            parent: None,
            range: None,
            effects: Vec::new(),
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

    /// Two clips of one file that sound together read through a decoder each, both straight on.
    /// (Through one shared decoder every block was two seeks - an ffmpeg respawn each on that backend.)
    #[test]
    fn overlapping_clips_of_one_file_never_seek() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        /// Counts reads that do not carry on within a frame of where the last one stopped.
        struct Seq(Arc<AtomicUsize>, Option<f64>);
        impl AudioSource for Seq {
            fn duration(&self) -> f64 {
                10.0
            }
            fn read_at(&mut self, t: f64, out: &mut [f32]) {
                // (the mixer reads one guard frame past each block)
                if self.1.is_some_and(|next| (t - next).abs() > 1.5 / SAMPLE_RATE as f64) {
                    self.0.fetch_add(1, Ordering::SeqCst);
                }
                self.1 = Some(t + (out.len() / 2 - 1) as f64 / SAMPLE_RATE as f64);
                out.fill(0.25);
            }
        }
        let mut p = project();
        let ai = p.audio_tracks()[0];
        let mut late = p.tracks[ai].clips[0].clone();
        (late.id, late.start, late.duration) = (999, 1.0, 5.0);
        let other = p.add_track(TrackKind::Audio);
        p.tracks[other].clips.push(late);
        let seeks = Arc::new(AtomicUsize::new(0));
        let mut pool = DecoderPool::new(Backend::Ffmpeg);
        for voice in 0..2 {
            pool.insert_audio_voice("Z:\\nope\\fake.wav", 0, voice, Box::new(Seq(seeks.clone(), None)));
        }
        let mut mx = Mixer::new();
        let mut out = vec![0.0f32; 2048];
        for block in 0..200 {
            mx.mix(&p, 1.0 + block as f64 * 1024.0 / SAMPLE_RATE as f64, &mut pool, &mut out);
            assert!((out[0] - 0.5).abs() < 1e-5, "both clips sound: {}", out[0]);
        }
        assert_eq!(seeks.load(Ordering::SeqCst), 0);
    }

    // ---- ws:audio-dsp-automation ----
    #[test]
    fn track_volume_is_sampled_and_multiplies_clip_gain() {
        let mut p = project();
        let ai = p.audio_tracks()[0];
        let mut pool = pool();
        let mut mx = Mixer::new();
        let mut out = vec![0.0f32; 2048];
        mx.mix(&p, 1.0, &mut pool, &mut out);
        assert!(out.iter().all(|s| (s - 0.5).abs() < 1e-5), "unity track volume: {}", out[0]);
        // a constant 0.5 halves the clip's 0.5 source
        p.tracks[ai].volume = crate::model::Animated::new(0.5);
        mx.mix(&p, 1.0, &mut pool, &mut out);
        assert!(out.iter().all(|s| (s - 0.25).abs() < 1e-5), "half track volume: {}", out[0]);
        // keyed: 1.0 at t=0, 0.0 at t=2 → 0.5 at t=1, sampled once at the block start and held
        p.tracks[ai].volume.keys = vec![
            crate::model::Keyframe { t: 0.0, v: 1.0, ease: Ease::Linear },
            crate::model::Keyframe { t: 2.0, v: 0.0, ease: Ease::Linear },
        ];
        mx.mix(&p, 1.0, &mut pool, &mut out);
        assert!((out[0] - 0.25).abs() < 1e-3, "keyed track volume at 1 s: {}", out[0]);
        assert!((out[out.len() - 2] - out[0]).abs() < 1e-6, "held for the block");
        mx.mix(&p, 1.9, &mut pool, &mut out);
        assert!((out[0] - 0.025).abs() < 1e-3, "keyed track volume at 1.9 s: {}", out[0]);
        // and it applies with buses in the path too (same sample, routed through Main)
        p.main_bus();
        mx.mix(&p, 1.0, &mut pool, &mut out);
        assert!((out[0] - 0.25).abs() < 1e-3, "through buses: {}", out[0]);
        // a sequence clip takes its AUDIO track's volume (an audio-only sequence lands on A1 alone)
        let mut p2 = Project::new();
        let aid = p2.add_asset(audio_asset(0, "Z:\\nope\\fake.wav"));
        let seq = p2.new_sequence("s", 320, 240, 30.0);
        let mut inner = Clip::new(500, ClipKind::Audio, "in", 0.0, 4.0);
        inner.asset = aid;
        let s = p2.sequence_mut(seq).unwrap();
        let sai = s.tracks.iter().position(|t| t.kind == TrackKind::Audio).unwrap();
        s.tracks[sai].clips.push(inner);
        let sc = p2.insert_sequence_clip(seq, 0.0, None).expect("placed");
        let ai = p2.track_of(sc).unwrap();
        assert_eq!(p2.tracks[ai].kind, TrackKind::Audio);
        p2.tracks[ai].volume = crate::model::Animated::new(0.5);
        mx.mix(&p2, 1.0, &mut pool, &mut out);
        assert!(out.iter().all(|s| (s - 0.25).abs() < 1e-5), "sequence clip × A1 volume: {}", out[0]);
    }

    /// A project that never touched `Track.volume` (every track at the `a1()` default, including one
    /// loaded from JSON without the field) mixes bit-identically to a build without the sampling.
    #[test]
    fn mixer_regression_existing_projects_unchanged() {
        let mut p = project();
        let ai = p.audio_tracks()[0];
        p.tracks[ai].clips[0].volume.value = 0.5;
        p.tracks[ai].clips[0].fade_in = 0.5;
        let loaded = Project::from_json(&p.to_json()).expect("round-trip");
        assert_eq!(loaded.tracks[ai].volume.value, 1.0);
        let mut pool = pool();
        let mut mx = Mixer::new();
        let mut reference = vec![0.0f32; 2 * 1024];
        let mut out = vec![0.0f32; 2 * 1024];
        for t in [0.0, 0.25, 1.0, 9.99] {
            // the reference is what the pre-change mixer produced: clip gain × fade, no track factor
            mx.mix(&p, t, &mut pool, &mut out);
            let (l0, _) = gains(&p.tracks[ai].clips[0], &[None, None], t);
            let (l1, _) = gains(&p.tracks[ai].clips[0], &[None, None], t + 1024.0 / SAMPLE_RATE as f64);
            for (k, fr) in reference.chunks_exact_mut(2).enumerate() {
                let tt = t + k as f64 / SAMPLE_RATE as f64;
                let g = if (0.0..10.0).contains(&tt) { 0.5 * (l0 + (l1 - l0) * k as f32 / 1024.0) } else { 0.0 };
                fr[0] = g;
                fr[1] = g;
            }
            for (a, b) in out.iter().zip(&reference) {
                assert!((a - b).abs() < 1e-5, "t={t}: {a} vs {b}");
            }
            let mut out2 = vec![0.0f32; 2 * 1024];
            mx.mix(&loaded, t, &mut pool, &mut out2);
            assert_eq!(out, out2, "a JSON-loaded project (no volume field) mixes identically");
        }
    }

    /// The meter data path end to end: a mixed block's post-fader bus output reaches a UI-side graph
    /// through `BusMeterFeed` with a non-zero peak and a finite LUFS reading (what `App::sync_buses`
    /// does every frame; `App` itself can't be built headless - see tools_registry_tests.rs).
    #[test]
    fn mixer_meters_reflect_live_playback() {
        use crate::engine::mixer_fx::{BusGraph, BusMeterFeed};
        let mut p = project();
        let main = p.main_bus();
        // a real tone, not the shared `pool()`'s DC `Const` - K-weighting high-passes DC out to silence
        // (correctly: a DC offset has no loudness), which would make the LUFS assertions below bogus.
        let mut pool = DecoderPool::new(Backend::Ffmpeg);
        pool.insert_audio("Z:\\nope\\fake.wav", 0, Box::new(Sine(0.5)));
        let mut mx = Mixer::new();
        let mut out = vec![0.0f32; 2 * 1024];
        let feed = BusMeterFeed::new();
        let mut ui = BusGraph::new();
        ui.sync(&p);
        assert_eq!(ui.meter(main), (0.0, 0.0), "nothing synced yet");
        // half a second of blocks, as playback.rs's audio thread would publish them
        for i in 0..24 {
            mx.mix(&p, 1.0 + i as f64 * 1024.0 / SAMPLE_RATE as f64, &mut pool, &mut out);
            mx.graph().publish(&feed);
        }
        feed.drain_into(&mut ui);
        let (l, r) = ui.meter(main);
        // a 0.5-amplitude sine's peak-hold settles near 0.5 but sampling won't land exactly on a peak
        assert!(l > 0.3 && r > 0.3, "Main peak {l} {r}");
        let (mo, int) = ui.lufs(main);
        assert!(mo.is_finite() && mo > -20.0 && mo < 0.0, "momentary {mo}");
        assert!(int.is_finite() && (int - mo).abs() < 1.0, "integrated {int} vs momentary {mo}");
        // silence afterwards leaves the peak decaying, not stuck
        let ai = p.audio_tracks()[0];
        p.tracks[ai].muted = true;
        for i in 0..24 {
            mx.mix(&p, 1.0 + i as f64 * 1024.0 / SAMPLE_RATE as f64, &mut pool, &mut out);
            mx.graph().publish(&feed);
        }
        feed.drain_into(&mut ui);
        assert!(ui.meter(main).0 < 0.1, "decayed: {:?}", ui.meter(main));
    }

    #[test]
    fn bus_routing() {
        use crate::model::{AudioFilter, FilterKind};
        let mut p = project();
        let ai = p.audio_tracks()[0];
        p.main_bus();
        let a = p.add_bus("A");
        p.tracks[ai].bus = a;
        let mut pool = pool();
        let mut mx = Mixer::new();
        let mut out = vec![0.0f32; 2 * 480];

        // A → Main with unity gain: the source comes through untouched
        mx.mix(&p, 1.0, &mut pool, &mut out);
        assert!(out.iter().all(|s| (s - 0.5).abs() < 1e-5), "{}", out[0]);

        // a clip on a muted bus is silent
        p.bus_mut(a).unwrap().muted = true;
        mx.mix(&p, 1.0, &mut pool, &mut out);
        assert!(out.iter().all(|s| *s == 0.0), "muted bus: {}", out[0]);
        p.bus_mut(a).unwrap().muted = false;

        // a solo elsewhere mutes A; soloing A brings it back (Main is never solo-silenced)
        let b = p.add_bus("B");
        p.bus_mut(b).unwrap().solo = true;
        mx.mix(&p, 1.0, &mut pool, &mut out);
        assert!(out.iter().all(|s| *s == 0.0), "other bus soloed: {}", out[0]);
        p.bus_mut(a).unwrap().solo = true;
        mx.mix(&p, 1.0, &mut pool, &mut out);
        assert!(out.iter().all(|s| (s - 0.5).abs() < 1e-5), "A soloed: {}", out[0]);
        p.bus_mut(a).unwrap().solo = false;
        p.bus_mut(b).unwrap().solo = false;

        // bus gain and pan
        p.bus_mut(a).unwrap().gain.value = 0.5;
        mx.mix(&p, 1.0, &mut pool, &mut out);
        assert!(out.iter().all(|s| (s - 0.25).abs() < 1e-5), "bus gain: {}", out[0]);
        p.bus_mut(a).unwrap().gain.value = 1.0;

        // mono folds L and R: pan the clip hard left, then fold
        p.tracks[ai].clips[0].pan.value = -1.0;
        mx.mix(&p, 1.0, &mut pool, &mut out);
        assert!((out[0] - 0.5).abs() < 1e-5 && out[1].abs() < 1e-5, "{:?}", &out[..2]);
        p.bus_mut(a).unwrap().mono = true;
        mx.mix(&p, 1.0, &mut pool, &mut out);
        assert!(out.iter().all(|s| (s - 0.25).abs() < 1e-5), "mono: {:?}", &out[..2]);
        p.bus_mut(a).unwrap().mono = false;
        p.tracks[ai].clips[0].pan.value = 0.0;

        // the bus filter chain runs on the summed bus
        let mut f = AudioFilter::new(FilterKind::Gain);
        f.params[0].value = -6.0;
        p.bus_mut(a).unwrap().filters.push(f);
        mx.mix(&p, 1.0, &mut pool, &mut out);
        let want = 0.5 * crate::engine::mixer_fx::db_to_lin(-6.0);
        assert!(out.iter().all(|s| (s - want).abs() < 1e-4), "bus filter: {} vs {want}", out[0]);
        p.bus_mut(a).unwrap().filters.clear();

        // a clip override beats its track's bus
        p.bus_mut(a).unwrap().muted = true;
        p.tracks[ai].clips[0].bus = b;
        mx.mix(&p, 1.0, &mut pool, &mut out);
        assert!(out.iter().all(|s| (s - 0.5).abs() < 1e-5), "clip override: {}", out[0]);

        // removing that bus clears the override, so the clip inherits its (still muted) track bus…
        p.remove_bus(b);
        mx.mix(&p, 1.0, &mut pool, &mut out);
        assert!(out.iter().all(|s| *s == 0.0), "back to the muted track bus: {}", out[0]);
        // …and clearing the track bus falls back to Main
        p.tracks[ai].bus = 0;
        mx.mix(&p, 1.0, &mut pool, &mut out);
        assert!(out.iter().all(|s| (s - 0.5).abs() < 1e-5), "fallback to Main: {}", out[0]);
    }

    #[test]
    fn bus_chain_sums_into_main() {
        // A → B → Main: the deeper bus must be flushed first, and its gain must apply on the way.
        let mut p = project();
        let ai = p.audio_tracks()[0];
        p.main_bus();
        let a = p.add_bus("A");
        let b = p.add_bus("B");
        p.bus_mut(a).unwrap().output = b;
        p.bus_mut(b).unwrap().gain.value = 0.5;
        p.tracks[ai].bus = a;
        let mut pool = pool();
        let mut mx = Mixer::new();
        let mut out = vec![0.0f32; 2 * 480];
        mx.mix(&p, 1.0, &mut pool, &mut out);
        assert!(out.iter().all(|s| (s - 0.25).abs() < 1e-5), "{}", out[0]);
        assert!((mx.graph().meter(a).0 - 0.5).abs() < 1e-4, "A meters pre-B: {:?}", mx.graph().meter(a));
        assert!((mx.graph().meter(b).0 - 0.25).abs() < 1e-4, "{:?}", mx.graph().meter(b));
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

    /// A fade-in at the very start of playback (block-sized like `playback::BLOCK`) must not bleed full
    /// volume: sample 0 is near-silent and every following sample in the block is >= the one before it.
    #[test]
    fn fade_in_first_block_near_silent_and_rising() {
        let mut p = project();
        let ai = p.audio_tracks()[0];
        p.tracks[ai].clips[0].fade_in = 0.5;
        let mut pool = pool();
        let mut mx = Mixer::new();
        let mut out = vec![0.0f32; 2 * 1024]; // playback::BLOCK
        mx.mix(&p, 0.0, &mut pool, &mut out);
        assert!(out[0].abs() < 1e-4, "sample 0 not near-silent: {}", out[0]);
        let mut prev = -1.0f32;
        for s in out.chunks_exact(2).map(|s| s[0]) {
            assert!(s + 1e-6 >= prev, "gain dipped: {s} after {prev}");
            prev = s;
        }
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
            edge: Default::default(),
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
            edge: Default::default(),
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
        // Two sequence pairs (V picture + A sound) back to back with a 2 s CrossFade added on V1:
        // `add_transition` mirrors it onto the linked audio twins, so the audio dissolves with the picture.
        let mut p = Project::new();
        let mut v = Vec::new();
        for (i, path) in ["Z:\\nope\\a.wav", "Z:\\nope\\b.wav"].into_iter().enumerate() {
            let asset = p.add_asset(audio_asset(0, path));
            let s = p.new_sequence("s", 320, 240, 30.0);
            let sq = p.sequence_mut(s).unwrap();
            let mut inner = Clip::new(500 + i as crate::model::Id, ClipKind::Audio, "in", 0.0, 10.0);
            inner.asset = asset;
            sq.tracks[1].clips.push(inner);
            sq.tracks[0].clips.push(Clip::new(600 + i as crate::model::Id, ClipKind::Video, "pic", 0.0, 10.0));
            let vc = p.insert_sequence_clip(s, i as f64 * 5.0, None).expect("placed");
            for id in p.linked(vc) {
                let c = p.clip_mut(id).unwrap(); // 5 s each; b plays its [5,10)
                (c.duration, c.src_in) = (5.0, i as f64 * 5.0);
            }
            v.push(vc);
        }
        assert!(p.add_transition(v[1], TransitionKind::CrossFade, 2.0).is_some());
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
        // Sequence with a picture and a Ramp audio clip [0,4); placed at t=1 as a linked V + A pair.
        let mut p = Project::new();
        let aid = p.add_asset(audio_asset(0, "Z:\\nope\\fake.wav"));
        let seq = p.new_sequence("s", 320, 240, 30.0);
        let mut inner = Clip::new(500, ClipKind::Audio, "in", 0.0, 4.0);
        inner.asset = aid;
        let s = p.sequence_mut(seq).unwrap();
        s.tracks[1].clips.push(inner);
        s.tracks[0].clips.push(Clip::new(501, ClipKind::Video, "pic", 0.0, 4.0));
        let sv = p.insert_sequence_clip(seq, 1.0, None).expect("placed");
        let sc = p.linked(sv).into_iter().find(|&c| c != sv).expect("audio twin");
        let (vi, ai) = (p.track_of(sv).unwrap(), p.track_of(sc).unwrap());
        assert_eq!((p.tracks[vi].kind, p.tracks[ai].kind), (TrackKind::Video, TrackKind::Audio));
        let mut pool = DecoderPool::new(Backend::Ffmpeg);
        pool.insert_audio("Z:\\nope\\fake.wav", 0, Box::new(Ramp));
        let mut mx = Mixer::new();
        let mut out = vec![0.0f32; 2 * 480];
        // t=1.5 → sequence-local 0.5 → ramp value 0.005, heard once (both halves playing would be 0.01)
        mx.mix(&p, 1.5, &mut pool, &mut out);
        assert!((out[0] - 0.005).abs() < 1e-4, "{}", out[0]);
        // the picture half's volume means nothing; the audio twin's volume/pan apply
        p.clip_mut(sv).unwrap().volume.value = 0.0;
        mx.mix(&p, 1.5, &mut pool, &mut out);
        assert!((out[0] - 0.005).abs() < 1e-4, "video half's volume: {}", out[0]);
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
        // before the sequence clip → silence; frozen → silence
        mx.mix(&p, 0.5, &mut pool, &mut out);
        assert!(out.iter().all(|s| *s == 0.0));
        p.clip_mut(sc).unwrap().freeze = Some(1.0);
        mx.mix(&p, 1.5, &mut pool, &mut out);
        assert!(out.iter().all(|s| *s == 0.0));
        p.clip_mut(sc).unwrap().freeze = None;
        // hiding the video track keeps the sound; muting the audio track silences it
        p.tracks[vi].muted = true;
        mx.mix(&p, 1.5, &mut pool, &mut out);
        assert!((out[0] - 0.005).abs() < 1e-4, "hidden V1: {}", out[0]);
        p.tracks[vi].muted = false;
        p.tracks[ai].muted = true;
        mx.mix(&p, 1.5, &mut pool, &mut out);
        assert!(out.iter().all(|s| *s == 0.0), "muted A1: {}", out[0]);
    }
}
