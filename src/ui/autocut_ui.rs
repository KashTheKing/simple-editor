//! Auto-cut pane (non-blocking; the timeline stays usable while it is open). Works on the selected audio
//! clips (a selected video clip uses its linked audio). Controls: threshold (dBFS slider -80..0),
//! min silence, min speech, padding (DragValues), "Keep: loud parts / quiet parts" toggle, "Ripple (close
//! gaps)" checkbox. It shows the detected segments live (count + total kept seconds) and publishes them to
//! `overlay` (timeline time ranges to KEEP, per selected clip) so the timeline can shade them; buttons:
//! "Split only" (cuts at the boundaries, removes nothing), "Apply" (Project::auto_cut with the cuts and the
//! quiet ranges; linked video follows), "Clear". Peaks come from WaveformCache (None while computing →
//! "analysing…"); detection uses engine::autocut. Undo once per Apply/Split. Returns what changed.

use crate::engine::autocut::{self, AutoCutParams};
use crate::media::waveform::{Peaks, WaveformCache};
use crate::model::{ClipKind, Id, Project};
use crate::theme::Palette;
use eframe::egui::{self, Button, DragValue, Slider};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// Detection result for one audio clip (timeline seconds).
struct Detection {
    clip: Id,
    cuts: Vec<f64>,
    quiet: Vec<(f64, f64)>,
    kept: usize,
    kept_secs: f64,
}

pub struct AutoCutState {
    pub params: AutoCutParams,
    pub keep_quiet: bool,
    pub ripple: bool,
    /// Timeline ranges to keep (shaded by the timeline), recomputed each frame from the selection.
    pub overlay: Vec<(f64, f64)>,
    /// False after "Clear": detection (and the overlay) pauses until a control changes or "Detect".
    pub active: bool,
    /// Result of the last Apply / Split, shown as a status line.
    pub status: String,
    cache_key: u64,
    cached: Vec<Detection>,
}

impl Default for AutoCutState {
    fn default() -> Self {
        Self {
            params: AutoCutParams::default(),
            keep_quiet: false,
            ripple: true,
            overlay: Vec::new(),
            active: true,
            status: String::new(),
            cache_key: 0,
            cached: Vec::new(),
        }
    }
}

/// Selected clips mapped to the audio clips to analyse (video → its linked audio), deduplicated.
fn audio_targets(project: &Project, selection: &[Id]) -> Vec<Id> {
    let mut out = Vec::new();
    for &id in selection {
        let Some(c) = project.clip(id) else { continue };
        let target = if c.kind == ClipKind::Audio {
            Some(id)
        } else {
            project.linked(id).into_iter().find(|&l| project.clip(l).is_some_and(|c| c.kind == ClipKind::Audio))
        };
        if let Some(t) = target {
            if !out.contains(&t) {
                out.push(t);
            }
        }
    }
    out
}

/// Complement of `remove` inside [start, end) — the ranges that survive the cut.
fn keep_ranges(start: f64, end: f64, remove: &[(f64, f64)], out: &mut Vec<(f64, f64)>) {
    let mut rs: Vec<(f64, f64)> = remove.to_vec();
    rs.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut cur = start;
    for (a, b) in rs {
        let (a, b) = (a.max(start), b.min(end));
        if b <= a {
            continue;
        }
        if a > cur + 1e-9 {
            out.push((cur, a));
        }
        cur = cur.max(b);
    }
    if end > cur + 1e-9 {
        out.push((cur, end));
    }
}

pub fn show(
    ui: &mut egui::Ui,
    state: &mut AutoCutState,
    project: &mut Project,
    selection: &[Id],
    waveforms: &mut WaveformCache,
    _palette: &Palette,
    undo: &mut dyn FnMut(&Project),
) -> bool {
    let mut changed = false;
    ui.strong("Auto-cut");
    let mut tweaked = false;
    egui::Grid::new("autocut_params").num_columns(2).show(ui, |ui| {
        ui.label("Threshold");
        tweaked |= ui.add(Slider::new(&mut state.params.threshold_db, -80.0..=0.0).suffix(" dBFS")).changed();
        ui.end_row();
        ui.label("Min silence");
        tweaked |=
            ui.add(DragValue::new(&mut state.params.min_silence).range(0.0..=10.0).speed(0.01).suffix(" s")).changed();
        ui.end_row();
        ui.label("Min speech");
        tweaked |=
            ui.add(DragValue::new(&mut state.params.min_speech).range(0.0..=10.0).speed(0.01).suffix(" s")).changed();
        ui.end_row();
        ui.label("Padding");
        tweaked |=
            ui.add(DragValue::new(&mut state.params.padding).range(0.0..=5.0).speed(0.01).suffix(" s")).changed();
        ui.end_row();
        ui.label("Keep");
        ui.horizontal(|ui| {
            tweaked |= ui.selectable_value(&mut state.keep_quiet, false, "Loud parts").changed();
            tweaked |= ui.selectable_value(&mut state.keep_quiet, true, "Quiet parts").changed();
        });
        ui.end_row();
    });
    ui.checkbox(&mut state.ripple, "Ripple (close gaps)");
    if tweaked {
        state.active = true;
    }

    let targets = audio_targets(project, selection);
    if targets.is_empty() {
        ui.label("Select an audio clip (or a video clip with linked audio)");
        state.overlay.clear();
        state.cached.clear();
        state.cache_key = 0;
        return false;
    }
    if !state.active {
        if ui.button("Detect").clicked() {
            state.active = true;
        }
        if !state.status.is_empty() {
            ui.label(state.status.clone());
        }
        return false;
    }

    // ---- detection (cached: recomputed only when params / peaks / clip geometry change) ----
    let mut peaks: Vec<(Id, Option<Arc<Peaks>>)> = Vec::with_capacity(targets.len());
    let mut analysing = false;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    state.params.threshold_db.to_bits().hash(&mut h);
    state.params.min_silence.to_bits().hash(&mut h);
    state.params.min_speech.to_bits().hash(&mut h);
    state.params.padding.to_bits().hash(&mut h);
    state.keep_quiet.hash(&mut h);
    for &id in &targets {
        let Some(c) = project.clip(id) else { continue };
        let p = project.asset(c.asset).map(|a| a.path.clone()).and_then(|path| waveforms.get(&path, c.audio_stream));
        if p.is_none() {
            analysing = true;
        }
        (id, c.start.to_bits(), c.duration.to_bits(), c.src_in.to_bits(), c.speed.to_bits(), c.audio_stream)
            .hash(&mut h);
        p.as_ref().map_or(0usize, |a| Arc::as_ptr(a) as *const () as usize).hash(&mut h);
        peaks.push((id, p));
    }
    let key = h.finish();
    if key != state.cache_key {
        state.cache_key = key;
        state.cached.clear();
        state.overlay.clear();
        let mut keep = Vec::new();
        for (id, p) in &peaks {
            let Some(p) = p else { continue };
            if p.is_empty() {
                continue;
            }
            let Some(c) = project.clip(*id) else { continue };
            if c.reverse || c.freeze.is_some() {
                continue; // ponytail: reversed/frozen clips are not auto-cut (engine maps forward only)
            }
            let segs = autocut::loud_segments(p, c.src_in, c.src_len(), &state.params);
            let (cuts, quiet) = autocut::to_timeline(&segs, c.start, c.src_in, c.duration, c.speed, state.keep_quiet);
            keep.clear();
            keep_ranges(c.start, c.end(), &quiet, &mut keep);
            let kept_secs: f64 = keep.iter().map(|(a, b)| b - a).sum();
            state.overlay.extend(keep.iter().copied());
            state.cached.push(Detection { clip: *id, cuts, quiet, kept: keep.len(), kept_secs });
        }
    }

    // ---- live status ----
    if analysing {
        ui.label("analysing…");
    } else {
        let kept: usize = state.cached.iter().map(|d| d.kept).sum();
        let secs: f64 = state.cached.iter().map(|d| d.kept_secs).sum();
        let removed: usize = state.cached.iter().map(|d| d.quiet.len()).sum();
        ui.label(format!("{kept} segments to keep ({secs:.1} s), {removed} to remove"));
    }

    // ---- actions ----
    let ready = state.cached.iter().any(|d| !d.cuts.is_empty() || !d.quiet.is_empty());
    let mut act: Option<bool> = None; // Some(apply)
    let mut clear = false;
    ui.horizontal(|ui| {
        if ui
            .add_enabled(ready, Button::new("Split only"))
            .on_hover_text("Cut at the boundaries, keep everything")
            .clicked()
        {
            act = Some(false);
        }
        if ui.add_enabled(ready, Button::new("Apply")).on_hover_text("Cut and remove the unwanted parts").clicked() {
            act = Some(true);
        }
        if ui.button("Clear").clicked() {
            clear = true;
        }
    });
    if let Some(apply) = act {
        undo(project);
        let mut cuts = 0usize;
        let mut removed = 0usize;
        for d in &state.cached {
            cuts += d.cuts.len();
            if apply {
                removed += project.auto_cut(&[d.clip], &d.cuts, &d.quiet, state.ripple);
            } else {
                project.auto_cut(&[d.clip], &d.cuts, &[], false);
            }
        }
        state.status =
            if apply { format!("Removed {removed} segments ({cuts} cuts)") } else { format!("Split at {cuts} cuts") };
        state.cache_key = 0; // clip ids changed → recompute next frame
        state.cached.clear();
        state.overlay.clear();
        changed = true;
    }
    if clear {
        state.active = false;
        state.overlay.clear();
        state.cached.clear();
        state.cache_key = 0;
    }
    if !state.status.is_empty() {
        ui.label(state.status.clone());
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::Backend;
    use crate::model::{Asset, AudioStreamInfo, Clip};
    use eframe::egui::{vec2, Color32, Event, Pos2, RawInput, Rect};

    #[test]
    fn keep_ranges_is_complement() {
        let mut out = Vec::new();
        keep_ranges(0.0, 10.0, &[(2.0, 4.0), (6.0, 7.0)], &mut out);
        assert_eq!(out, vec![(0.0, 2.0), (4.0, 6.0), (7.0, 10.0)]);
        out.clear();
        keep_ranges(0.0, 10.0, &[], &mut out);
        assert_eq!(out, vec![(0.0, 10.0)]);
        out.clear();
        keep_ranges(0.0, 10.0, &[(0.0, 10.0)], &mut out);
        assert!(out.is_empty());
        out.clear();
        // unsorted + out-of-clip ranges are clamped
        keep_ranges(1.0, 9.0, &[(8.0, 12.0), (-1.0, 2.0)], &mut out);
        assert_eq!(out, vec![(2.0, 8.0)]);
    }

    #[test]
    fn audio_targets_map_video_to_linked_audio() {
        let mut p = Project::new();
        let aid = p.add_asset(Asset {
            id: 0,
            path: "C:/t.mp4".into(),
            kind: ClipKind::Video,
            duration: 5.0,
            width: 320,
            height: 240,
            fps: 30.0,
            audio_streams: vec![AudioStreamInfo { channels: 2, sample_rate: 48000, ..Default::default() }],
            codec: String::new(),
            folder: String::new(),
            tags: Vec::new(),
            label: 0,
            description: String::new(),
        });
        p.insert_asset_clips(aid, 0.0, Some(0));
        let vid = p.tracks[0].clips[0].id;
        let aud = p.tracks[1].clips[0].id;
        assert_eq!(audio_targets(&p, &[vid]), vec![aud], "video selection maps to its linked audio");
        assert_eq!(audio_targets(&p, &[vid, aud]), vec![aud], "deduplicated");
        // unlinked video-only clip maps to nothing
        p.tracks[0].clips.push(Clip::new(99, ClipKind::Video, "solo", 6.0, 1.0));
        assert!(audio_targets(&p, &[99]).is_empty());
    }

    #[test]
    fn show_smoke_no_selection_and_pending_peaks() {
        let ctx = egui::Context::default();
        let mut project = Project::new();
        let aid = project.add_asset(Asset {
            id: 0,
            path: format!("C:/does-not-exist-{}.mp4", std::process::id()),
            kind: ClipKind::Video,
            duration: 5.0,
            width: 320,
            height: 240,
            fps: 30.0,
            audio_streams: vec![AudioStreamInfo { channels: 2, sample_rate: 48000, ..Default::default() }],
            codec: String::new(),
            folder: String::new(),
            tags: Vec::new(),
            label: 0,
            description: String::new(),
        });
        project.insert_asset_clips(aid, 0.0, Some(0));
        let aud = project.tracks[1].clips[0].id;
        let mut waves = WaveformCache::new(ctx.clone(), Backend::Ffmpeg);
        let mut state = AutoCutState::default();
        let pal = Palette::new(true, Color32::WHITE);
        let mut undos = 0;
        for selection in [vec![], vec![aud]] {
            for _ in 0..3 {
                let _ = ctx.run(
                    RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 400.0))),
                        events: Vec::<Event>::new(),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            let mut undo = |_: &Project| undos += 1;
                            let changed = show(ui, &mut state, &mut project, &selection, &mut waves, &pal, &mut undo);
                            assert!(!changed, "nothing should change without user action");
                        });
                    },
                );
            }
        }
        assert_eq!(undos, 0);
        // missing file → the cache resolves to empty peaks → no detections, no overlay
        assert!(state.cached.is_empty());
        assert!(state.overlay.is_empty());
    }
}
