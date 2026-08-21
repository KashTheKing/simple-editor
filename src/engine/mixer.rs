//! Audio mixer: sums every audible audio clip at timeline time t into interleaved stereo f32.
//! Used by playback (real-time, block by block) and export (offline to WAV).

use crate::media::{DecoderPool, SAMPLE_RATE};
use crate::model::{Project, TrackKind};

pub struct Mixer {
    scratch: Vec<f32>,
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
        let frames = out.len() / 2;
        if frames == 0 {
            return;
        }
        let sr = SAMPLE_RATE as f64;
        let t_end = t + frames as f64 / sr;
        for (ti, track) in project.tracks.iter().enumerate() {
            if track.kind != TrackKind::Audio || !project.active(ti) {
                continue;
            }
            for clip in &track.clips {
                if !clip.enabled || clip.end() <= t || clip.start >= t_end {
                    continue;
                }
                let Some(asset) = project.asset(clip.asset) else {
                    continue;
                };
                // overlapping sample range within the block
                let i0 = (((clip.start.max(t) - t) * sr).round() as usize).min(frames);
                let i1 = (((clip.end().min(t_end) - t) * sr).round() as usize).min(frames);
                if i1 <= i0 {
                    continue;
                }
                let n = i1 - i0;
                let Some(src) = pool.audio(&asset.path, clip.audio_stream) else {
                    continue;
                };
                self.scratch.resize(n * 2, 0.0);
                let t0 = t + i0 as f64 / sr;
                src.read_at(clip.src_time(t0), &mut self.scratch);
                let v0 = clip.volume.at(clip.local(t0)) as f32;
                let v1 = clip.volume.at(clip.local(t + i1 as f64 / sr)) as f32;
                let dv = (v1 - v0) / n as f32;
                let dst = &mut out[i0 * 2..i1 * 2];
                for (k, (d, s)) in dst.chunks_exact_mut(2).zip(self.scratch.chunks_exact(2)).enumerate() {
                    let g = v0 + dv * k as f32;
                    d[0] += s[0] * g;
                    d[1] += s[1] * g;
                }
            }
        }
        for s in out.iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::{AudioSource, Backend};
    use crate::model::{Asset, AudioStreamInfo, ClipKind};

    struct Const(f32);
    impl AudioSource for Const {
        fn duration(&self) -> f64 {
            10.0
        }
        fn read_at(&mut self, t: f64, out: &mut [f32]) {
            for (i, s) in out.iter_mut().enumerate() {
                let tt = t + (i / 2) as f64 / SAMPLE_RATE as f64;
                *s = if tt < 10.0 { self.0 } else { 0.0 };
            }
        }
    }

    fn project() -> Project {
        let asset = Asset {
            id: 0,
            path: "Z:\\nope\\fake.wav".into(),
            kind: ClipKind::Audio,
            duration: 10.0,
            width: 0,
            height: 0,
            fps: 0.0,
            audio_streams: vec![AudioStreamInfo { index: 0, channels: 2, sample_rate: 48000, ..Default::default() }],
            codec: "pcm".into(),
        };
        Project::from_media(asset)
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
        c.volume.keys = vec![crate::model::Keyframe { t: 0.0, v: 0.0 }, crate::model::Keyframe { t: 1.0, v: 2.0 }];
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
}
