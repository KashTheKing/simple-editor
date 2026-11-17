//! ---- ws:gallery-audio ----
//! Hover audition for the Gallery's Audio cards. A few seconds of the selected audio clip (from the
//! playhead when it is inside the clip, else from the clip's start) are rendered through the card on a
//! low-priority thread (`media::bg`) into a WAV in the cache folder, then played by the Sounds tab's own
//! preview (`gallery_mine::preview`: the system's player, nothing on the playback path). Renders are keyed
//! by (file, stream, start, card) so the second hover plays at once. No audio clip selected: silence (the
//! card's picture still shows what it does).

use super::*;
use crate::ui::gallery::GalleryTab;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;

/// Seconds auditioned.
const LEN: f64 = 4.0;

#[derive(Default)]
struct State {
    /// The render in flight: its key and where its result arrives.
    pending: Option<(u64, mpsc::Receiver<Option<PathBuf>>)>,
    /// Finished renders by key (None = that card cannot be auditioned, e.g. J / L-cut).
    ready: HashMap<u64, Option<PathBuf>>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::default();
}

/// What a render needs, moved onto its thread.
struct Job {
    path: String,
    stream: usize,
    from: f64,
    tab: GalleryTab,
    name: String,
    block: Option<crate::model::KeyBlock>,
    out: PathBuf,
}

fn key(path: &str, stream: usize, from: f64, tab: GalleryTab, name: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::hash::DefaultHasher::new();
    (path, stream, (from * 10.0).round() as i64, tab.name(), name).hash(&mut h);
    h.finish()
}

/// This frame's audition wish from the Gallery (`GalleryResponse.audition`): start or poll its render and,
/// once it is there, ask the preview to play it (`gallery::wish_sound`, renewed every frame it is wanted).
pub(super) fn want(app: &mut App, ctx: &egui::Context, req: Option<(GalleryTab, String)>) {
    let Some((tab, name)) = req else { return };
    let Some(clip) = super::tools_gallery::audio_targets(&app.project, &app.selection)
        .first()
        .and_then(|&id| app.project.clip(id))
        .cloned()
    else {
        return;
    };
    let Some(path) = app.project.asset(clip.asset).map(|a| a.path.clone()) else { return };
    let from =
        if clip.contains(app.playhead) { clip.src_in + (app.playhead - clip.start) * clip.speed } else { clip.src_in };
    let k = key(&path, clip.audio_stream, from, tab, &name);
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        if let Some(done) = s.ready.get(&k) {
            if let Some(p) = done {
                crate::ui::gallery::wish_sound(p.clone());
            }
            return;
        }
        if let Some((pk, rx)) = &s.pending {
            if *pk == k {
                match rx.try_recv() {
                    Ok(r) => {
                        if let Some(p) = &r {
                            crate::ui::gallery::wish_sound(p.clone());
                        }
                        s.ready.insert(k, r);
                        s.pending = None;
                    }
                    Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(80)),
                    Err(mpsc::TryRecvError::Disconnected) => {
                        s.ready.insert(k, None);
                        s.pending = None;
                    }
                }
                return;
            }
        }
        // a new card (an older render, if any, finishes on its own and is dropped)
        let block = (tab == GalleryTab::AudioAnimate)
            .then(|| crate::model::find_block(&app.settings.key_blocks, &name))
            .flatten();
        let dir = crate::settings::Settings::cache_dir().join("gallery-audition");
        let job =
            Job { path, stream: clip.audio_stream, from, tab, name, block, out: dir.join(format!("{k:016x}.wav")) };
        let (tx, rx) = mpsc::channel();
        s.pending = Some((k, rx));
        std::thread::spawn(move || {
            crate::media::bg::lower_thread();
            let _ = tx.send(render(&job).ok());
        });
        ctx.request_repaint_after(std::time::Duration::from_millis(80));
    });
}

/// Interleaved stereo at `SAMPLE_RATE` from `path` starting at source second `from`.
fn read(path: &str, stream: usize, from: f64, secs: f64) -> Result<Vec<f32>, String> {
    let mut src = crate::media::open_audio(path, stream, crate::media::Backend::Auto)?;
    let mut buf = vec![0.0f32; (secs * crate::media::SAMPLE_RATE as f64) as usize * 2];
    src.read_at(from.max(0.0), &mut buf);
    Ok(buf)
}

/// The audition WAV for `job` (written to `job.out`).
fn render(job: &Job) -> Result<PathBuf, String> {
    let sr = crate::media::SAMPLE_RATE;
    let mut buf = read(&job.path, job.stream, job.from, LEN)?;
    process(job.tab, &job.name, job.block.as_ref(), &mut buf, || {
        read(&job.path, job.stream, job.from + LEN / 2.0, LEN)
    })?;
    std::fs::create_dir_all(job.out.parent().ok_or("no folder")?).map_err(|e| e.to_string())?;
    std::fs::write(&job.out, wav16(&buf, sr)).map_err(|e| e.to_string())?;
    Ok(job.out.clone())
}

/// Run `buf` (LEN seconds at SAMPLE_RATE) through an Audio card. A transition crossfades into `later`
/// (the same clip further on) across the middle; a J / L-cut has nothing to hear (Err).
pub(crate) fn process(
    tab: GalleryTab,
    name: &str,
    block: Option<&crate::model::KeyBlock>,
    buf: &mut [f32],
    later: impl FnOnce() -> Result<Vec<f32>, String>,
) -> Result<(), String> {
    let sr = crate::media::SAMPLE_RATE;
    let secs = (buf.len() / 2) as f64 / sr as f64;
    match tab {
        GalleryTab::AudioEffects => {
            let id = crate::engine::mixer_fx::fx_id(name).ok_or("unknown effect")?;
            let (_, chain) = crate::engine::mixer_fx::fx_chain(id, (0.0, secs)).ok_or("unknown effect")?;
            crate::engine::mixer_fx::run_chain(&chain, sr, 0.0, buf);
        }
        GalleryTab::AudioAnimate => {
            let b = block.ok_or("unknown block")?;
            let b = if b.gen.is_some() { crate::model::BlockCtl::new(b.clone()).resolve(b.dur) } else { b.clone() };
            let mut c = Clip::new(0, crate::model::ClipKind::Audio, "", 0.0, secs);
            b.write(&mut c, 0.0, b.dur.min(secs), 1.0, 1.0);
            for (i, fr) in buf.chunks_exact_mut(2).enumerate() {
                let t = i as f64 / sr as f64;
                let g = c.volume.at(t) as f32;
                let pan = c.pan.at(t).clamp(-1.0, 1.0) as f32;
                fr[0] *= g * (1.0 - pan).min(1.0);
                fr[1] *= g * (1.0 + pan).min(1.0);
            }
        }
        GalleryTab::AudioTransitions => {
            let (dip, curve) = match name {
                "Dip" => (true, crate::model::XfadeCurve::Linear),
                "Equal Power" => (false, crate::model::XfadeCurve::EqualPower),
                "Exponential" => (false, crate::model::XfadeCurve::Exponential),
                "Linear" => (false, crate::model::XfadeCurve::Linear),
                _ => return Err("nothing to hear".into()),
            };
            let b = later()?;
            let (a0, a1) = (secs * 0.3, secs * 0.7);
            for (i, (fr, br)) in buf.chunks_exact_mut(2).zip(b.chunks_exact(2)).enumerate() {
                let t = i as f64 / sr as f64;
                let (go, gi) = crate::model::audio_curve_gains(dip, curve, ((t - a0) / (a1 - a0)).clamp(0.0, 1.0));
                fr[0] = fr[0] * go as f32 + br[0] * gi as f32;
                fr[1] = fr[1] * go as f32 + br[1] * gi as f32;
            }
        }
        _ => return Err("not an audio card".into()),
    }
    Ok(())
}

/// 16-bit PCM stereo WAV bytes.
pub(crate) fn wav16(buf: &[f32], sr: u32) -> Vec<u8> {
    let data = (buf.len() * 2) as u32;
    let mut v = Vec::with_capacity(44 + data as usize);
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes()); // PCM
    v.extend_from_slice(&2u16.to_le_bytes()); // stereo
    v.extend_from_slice(&sr.to_le_bytes());
    v.extend_from_slice(&(sr * 4).to_le_bytes());
    v.extend_from_slice(&4u16.to_le_bytes());
    v.extend_from_slice(&16u16.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&data.to_le_bytes());
    for s in buf {
        v.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone() -> Vec<f32> {
        (0..(crate::media::SAMPLE_RATE as usize * 2))
            .flat_map(|i| {
                let s = (i as f32 * 0.05).sin() * 0.5;
                [s, s]
            })
            .collect()
    }

    #[test]
    fn every_audio_card_but_the_cut_helpers_can_be_heard() {
        let settings = crate::settings::Settings::default();
        for tab in [GalleryTab::AudioAnimate, GalleryTab::AudioEffects, GalleryTab::AudioTransitions] {
            for name in crate::ui::gallery::card_names(tab, &settings) {
                let block = crate::model::find_block(&[], &name);
                let mut buf = tone();
                let r = process(tab, &name, block.as_ref(), &mut buf, || Ok(vec![0.0; tone().len()]));
                if name.ends_with("-Cut") {
                    assert!(r.is_err(), "{name}");
                    continue;
                }
                r.unwrap_or_else(|e| panic!("{tab:?} {name}: {e}"));
                assert!(buf.iter().all(|s| s.is_finite()), "{name}");
                // (a gate / limiter / de-esser rightly leaves a plain moderate tone alone)
                if tab != GalleryTab::AudioEffects {
                    assert_ne!(buf, tone(), "{tab:?} {name} changed nothing");
                }
            }
        }
    }

    #[test]
    fn wav_header_is_stereo_16_bit() {
        let w = wav16(&[0.0, 1.0, -1.0, 0.5], 48_000);
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(u16::from_le_bytes([w[22], w[23]]), 2);
        assert_eq!(u32::from_le_bytes([w[40], w[41], w[42], w[43]]), 8);
        assert_eq!(w.len(), 44 + 8);
    }
}
