//! Auto-cut pane (non-blocking; the timeline stays usable while it is open). Works on the selected audio
//! clips (a selected video clip uses its linked audio). Controls: threshold (dBFS slider -80..0),
//! min silence, min speech, padding (DragValues), "Keep: loud parts / quiet parts" toggle, "Ripple (close
//! gaps)" checkbox. It shows the detected segments live (count + total kept seconds) and publishes them to
//! `overlay` (timeline time ranges to KEEP, per selected clip) so the timeline can shade them; buttons:
//! "Split only" (cuts at the boundaries, removes nothing), "Apply" (Project::auto_cut with the cuts and the
//! quiet ranges; linked video follows), "Clear". Peaks come from WaveformCache (None while computing →
//! "analysing…"); detection uses engine::autocut. Undo once per Apply/Split. Returns what changed.

use crate::engine::analysis::{self, NormMode};
use crate::engine::autocut::{self, AutoCutParams};
use crate::media::waveform::{Peaks, WaveformCache};
use crate::model::{ClipKind, Id, Project};
use crate::settings::Settings;
use crate::theme::Palette;
use eframe::egui::{self, Button, DragValue, Slider};
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// Detection result for one audio clip (timeline seconds).
struct Detection {
    clip: Id,
    cuts: Vec<f64>,
    quiet: Vec<(f64, f64)>,
    /// Per-`quiet`-segment toggle: Apply/Mark-instead only act on the `true` entries.
    included: Vec<bool>,
    /// Cosmetic label for the segment list ("Silence" — the only kind this struct detects).
    kind: &'static str,
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
    pub scene_cuts: SceneCutState,
    pub beats: BeatsState,
    pub loudness: LoudnessState,
    pub duck: DuckState,
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
            scene_cuts: SceneCutState::default(),
            beats: BeatsState::default(),
            loudness: LoudnessState::default(),
            duck: DuckState::default(),
        }
    }
}

/// "Scene cuts" section state: last threshold used, the clip + detected (source-second) cut times from
/// the last "Detect" (so Split/Mark instead act on them without re-running ffmpeg).
pub struct SceneCutState {
    pub thr: f32,
    clip: Option<Id>,
    cuts: Vec<f64>,
    pub status: String,
}

impl Default for SceneCutState {
    fn default() -> Self {
        Self { thr: 0.3, clip: None, cuts: Vec::new(), status: String::new() }
    }
}

/// "Beats" section state: the last "Detect Beats" preview (per-clip onset times, source seconds; no
/// mutation) plus its BPM readout. "Add Markers"/"Split at Beats" act on this cached preview instead
/// of re-detecting, mirroring `SceneCutState`'s Detect-then-commit split.
#[derive(Default)]
pub struct BeatsState {
    cuts: Vec<(Id, Vec<f64>)>,
    pub bpm: Option<f64>,
    pub status: String,
}

/// "Loudness" section state.
pub struct LoudnessState {
    pub target_dbfs: f64,
    pub status: String,
}

impl Default for LoudnessState {
    fn default() -> Self {
        Self { target_dbfs: -1.0, status: String::new() }
    }
}

/// "Duck" section state: the two-bucket picker (one music clip, any number of dialogue clips) plus the
/// depth/ramp DragValues. `depth_db`/`ramp_ms` default to `Settings`'s own defaults (-12 dB / 200 ms);
/// ponytail: they don't live-reseed from a changed `Settings.duck_depth_db` afterward, only at startup —
/// a per-call override, same as the tool args.
pub struct DuckState {
    pub music: Option<Id>,
    pub dialogue: HashSet<Id>,
    pub depth_db: f32,
    pub ramp_ms: u32,
    pub status: String,
}

impl Default for DuckState {
    fn default() -> Self {
        Self { music: None, dialogue: HashSet::new(), depth_db: -12.0, ramp_ms: 200, status: String::new() }
    }
}

/// "Mark instead" for genuine (start,end) ranges (silence auto-cut only): one range `Marker` per
/// (start,end) via `add_marker`+`marker_mut` (mirrors `subtitles_ui.rs`'s `mark_dups`). No clip
/// mutation. Returns the created marker ids — callers fire `marker_added` once per id themselves: this
/// fn stays App-free (App's `project`/`fire_hook` aren't reachable from `ui::autocut_ui`, only from
/// `ui::app` and its descendants — see the audio-analysis PR's deviation note).
pub(crate) fn mark_ranges(project: &mut Project, ranges: &[(f64, f64)], name_prefix: &str) -> Vec<Id> {
    let mut ids = Vec::with_capacity(ranges.len());
    for (i, &(start, end)) in ranges.iter().enumerate() {
        let id = project.add_marker(start, format!("{name_prefix} {}", i + 1));
        if let Some(m) = project.marker_mut(id) {
            m.duration = (end - start).max(0.0);
        }
        ids.push(id);
    }
    ids
}

/// A `peaks_of` closure over `waveforms`, keyed by ASSET id (stream 0) — the shape
/// `analysis::normalize`/`match_loudness`/`duck` take. ponytail: a clip picking a non-zero
/// `audio_stream` reads stream 0 here; correct for the overwhelming single-stream-asset case.
fn asset_peaks(waveforms: &mut WaveformCache) -> impl FnMut(&Project, Id) -> Option<Arc<Peaks>> + '_ {
    move |project, asset_id| {
        let path = project.asset(asset_id)?.path.clone();
        waveforms.get(&path, 0)
    }
}

fn selected_video(project: &Project, selection: &[Id]) -> Option<Id> {
    selection.iter().copied().find(|&id| project.clip(id).is_some_and(|c| c.kind == ClipKind::Video))
}

/// Selected clips mapped to the audio clips to analyse (video → its linked audio), deduplicated.
/// `pub(crate)`: also the `autocut.detect`/`autocut.mark` MCP tools' default-target resolver
/// (`ui::app::tools_audio.rs`), named explicitly by the audio-analysis plan's tool arg docs.
pub(crate) fn audio_targets(project: &Project, selection: &[Id]) -> Vec<Id> {
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

/// Apply's ripple shifts every later clip left, which invalidates the absolute times cached for them —
/// so applying right-to-left keeps each detection valid until it has been used.
fn sort_right_to_left(project: &Project, cached: &mut [Detection]) {
    cached.sort_by(|a, b| {
        let s = |d: &Detection| project.clip(d.clip).map_or(0.0, |c| c.start);
        s(b).total_cmp(&s(a))
    });
}

pub fn show(
    ui: &mut egui::Ui,
    state: &mut AutoCutState,
    project: &mut Project,
    selection: &[Id],
    waveforms: &mut WaveformCache,
    settings: &Settings,
    _palette: &Palette,
    undo: &mut dyn FnMut(&Project),
) -> (bool, Vec<Id>) {
    let mut changed = false;
    let mut marked: Vec<Id> = Vec::new();
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
    } else if !state.active {
        if ui.button("Detect").clicked() {
            state.active = true;
        }
        if !state.status.is_empty() {
            ui.label(state.status.clone());
        }
    } else {
        show_silence(ui, state, project, &targets, waveforms, undo, &mut changed, &mut marked);
    }

    ui.separator();
    egui::CollapsingHeader::new("Scene cuts").id_salt("autocut_scene_cuts").show(ui, |ui| {
        scene_cuts_ui(ui, &mut state.scene_cuts, project, selection, undo, &mut changed, &mut marked);
    });
    egui::CollapsingHeader::new("Beats").id_salt("autocut_beats").show(ui, |ui| {
        beats_ui(ui, &mut state.beats, project, selection, waveforms, settings, undo, &mut changed, &mut marked);
    });
    egui::CollapsingHeader::new("Loudness").id_salt("autocut_loudness").show(ui, |ui| {
        loudness_ui(ui, &mut state.loudness, project, selection, waveforms, undo, &mut changed);
    });
    egui::CollapsingHeader::new("Duck").id_salt("autocut_duck").show(ui, |ui| {
        duck_ui(ui, &mut state.duck, project, selection, waveforms, undo, &mut changed);
    });

    (changed, marked)
}

/// The existing silence-detection flow (threshold/min-silence/min-speech/padding already drawn by the
/// caller): cached per-clip detections, per-segment include toggles, Split only / Apply / Mark instead /
/// Clear. Split out of `show` so the new Scene cuts/Beats/Loudness/Duck sections don't inflate one
/// already-large function.
#[allow(clippy::too_many_arguments)]
fn show_silence(
    ui: &mut egui::Ui,
    state: &mut AutoCutState,
    project: &mut Project,
    targets: &[Id],
    waveforms: &mut WaveformCache,
    undo: &mut dyn FnMut(&Project),
    changed: &mut bool,
    marked: &mut Vec<Id>,
) {
    // ---- detection (cached: recomputed only when params / peaks / clip geometry change) ----
    let mut peaks: Vec<(Id, Option<Arc<Peaks>>)> = Vec::with_capacity(targets.len());
    let mut analysing = false;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    state.params.threshold_db.to_bits().hash(&mut h);
    state.params.min_silence.to_bits().hash(&mut h);
    state.params.min_speech.to_bits().hash(&mut h);
    state.params.padding.to_bits().hash(&mut h);
    state.keep_quiet.hash(&mut h);
    for &id in targets {
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
            let included = vec![true; quiet.len()];
            state.cached.push(Detection {
                clip: *id,
                cuts,
                quiet,
                included,
                kind: "Silence",
                kept: keep.len(),
                kept_secs,
            });
        }
        sort_right_to_left(project, &mut state.cached);
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

    // ---- per-segment include toggles ----
    let total_segs: usize = state.cached.iter().map(|d| d.quiet.len()).sum();
    if total_segs > 0 {
        egui::CollapsingHeader::new(format!("Segments ({total_segs})")).id_salt("autocut_segments").show(ui, |ui| {
            egui::ScrollArea::vertical().max_height(120.0).id_salt("autocut_segments_scroll").show(ui, |ui| {
                for d in &mut state.cached {
                    let name = project.clip(d.clip).map(|c| c.name.clone()).unwrap_or_default();
                    let kind = d.kind;
                    for i in 0..d.quiet.len() {
                        let (a, b) = d.quiet[i];
                        if let Some(inc) = d.included.get_mut(i) {
                            ui.checkbox(inc, format!("{kind} [{name}]: {a:.2}s – {b:.2}s"));
                        }
                    }
                }
            });
        });
    }

    // ---- actions ----
    let ready = state.cached.iter().any(|d| !d.cuts.is_empty() || !d.quiet.is_empty());
    let mut act: Option<bool> = None; // Some(apply)
    let mut mark = false;
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
        if ui
            .add_enabled(ready, Button::new("Mark instead"))
            .on_hover_text("Add range markers over the included segments; cut nothing")
            .clicked()
        {
            mark = true;
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
                let subset: Vec<(f64, f64)> =
                    d.quiet.iter().zip(&d.included).filter(|(_, &inc)| inc).map(|(&r, _)| r).collect();
                removed += project.auto_cut(&[d.clip], &d.cuts, &subset, state.ripple);
            } else {
                project.auto_cut(&[d.clip], &d.cuts, &[], false);
            }
        }
        state.status =
            if apply { format!("Removed {removed} segments ({cuts} cuts)") } else { format!("Split at {cuts} cuts") };
        state.cache_key = 0; // clip ids changed → recompute next frame
        state.cached.clear();
        state.overlay.clear();
        *changed = true;
    }
    if mark {
        let ranges: Vec<(f64, f64)> = state
            .cached
            .iter()
            .flat_map(|d| d.quiet.iter().zip(&d.included).filter(|(_, &inc)| inc).map(|(&r, _)| r))
            .collect();
        if !ranges.is_empty() {
            undo(project);
            let ids = mark_ranges(project, &ranges, "Silence");
            state.status = format!("Marked {} silence range(s)", ids.len());
            marked.extend(ids);
            *changed = true;
        }
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
}

/// Scene cuts section: ffmpeg-detected shot changes on the selected VIDEO clip's source. Detect only
/// runs the (synchronous — see the engine's own ponytail note) ffmpeg pass; Split/Mark instead reuse the
/// last detection's cut times.
fn scene_cuts_ui(
    ui: &mut egui::Ui,
    state: &mut SceneCutState,
    project: &mut Project,
    selection: &[Id],
    undo: &mut dyn FnMut(&Project),
    changed: &mut bool,
    marked: &mut Vec<Id>,
) {
    ui.add(Slider::new(&mut state.thr, 0.0..=1.0).text("Threshold"));
    ui.horizontal(|ui| {
        if ui.button("Detect").clicked() {
            match selected_video(project, selection) {
                Some(clip) => {
                    let asset_id = project.clip(clip).map(|c| c.asset);
                    let path = asset_id.and_then(|a| project.asset(a)).map(|a| a.path.clone());
                    match path {
                        Some(path) => match analysis::scene_cuts(std::path::Path::new(&path), state.thr) {
                            Ok(cuts) => {
                                state.status = format!("{} scene cut(s)", cuts.len());
                                state.clip = Some(clip);
                                state.cuts = cuts;
                            }
                            Err(e) => {
                                state.status = e;
                                state.clip = None;
                                state.cuts.clear();
                            }
                        },
                        None => state.status = "Selected clip has no source file".into(),
                    }
                }
                None => state.status = "Select a video clip".into(),
            }
        }
        let has = state.clip.is_some() && !state.cuts.is_empty();
        if ui.add_enabled(has, Button::new("Split")).clicked() {
            if let Some(clip) = state.clip {
                undo(project);
                let n = analysis::split_scene_cuts(project, clip, &state.cuts);
                state.status = format!("Split at {n} cut(s)");
                if n > 0 {
                    *changed = true;
                }
            }
        }
        if ui.add_enabled(has, Button::new("Mark instead")).clicked() {
            if let Some(clip) = state.clip {
                undo(project);
                let ids = analysis::scene_cut_markers(project, clip, &state.cuts);
                state.status = format!("Marked {} scene cut(s)", ids.len());
                if !ids.is_empty() {
                    marked.extend(ids);
                    *changed = true;
                }
            }
        }
    });
    if !state.status.is_empty() {
        ui.label(&state.status);
    }
}

/// Beats section: "Detect Beats" is a pure preview (onset times + BPM, no mutation, no undo pushed) —
/// only "Add Markers" / "Split at Beats" write to the project, matching the Silence/Scene-cuts
/// sections' detect-then-commit shape. Shares `analysis::detect_beats`/`beat_markers`/`split_beats`
/// with `Action::DetectBeats`/`Action::SplitAtBeats` and `audio.beats` so the onset math is never
/// re-derived per entry point.
#[allow(clippy::too_many_arguments)]
fn beats_ui(
    ui: &mut egui::Ui,
    state: &mut BeatsState,
    project: &mut Project,
    selection: &[Id],
    waveforms: &mut WaveformCache,
    settings: &Settings,
    undo: &mut dyn FnMut(&Project),
    changed: &mut bool,
    marked: &mut Vec<Id>,
) {
    let targets = audio_targets(project, selection);
    if ui.add_enabled(!targets.is_empty(), Button::new("Detect Beats")).clicked() {
        let (per_clip, bpm) = analysis::detect_beats(project, &targets, 0.25, settings.beat_thr, &mut asset_peaks(waveforms));
        let total: usize = per_clip.iter().map(|(_, o)| o.len()).sum();
        state.status = if total == 0 { "No beats found".into() } else { format!("{total} beat(s) detected") };
        state.bpm = bpm;
        state.cuts = per_clip;
    }
    let ready = state.cuts.iter().any(|(_, o)| !o.is_empty());
    ui.horizontal(|ui| {
        if ui.add_enabled(ready, Button::new("Add Markers")).clicked() {
            undo(project);
            let ids = analysis::beat_markers(project, &state.cuts);
            state.status = format!("Added {} beat marker(s)", ids.len());
            if !ids.is_empty() {
                marked.extend(ids);
                *changed = true;
            }
        }
        if ui.add_enabled(ready, Button::new("Split at Beats")).clicked() {
            undo(project);
            let n = analysis::split_beats(project, &state.cuts);
            state.status = format!("Split at {n} beat(s)");
            if n > 0 {
                *changed = true;
            }
        }
    });
    if let Some(bpm) = state.bpm {
        ui.label(format!("BPM: {bpm:.0}"));
    }
    if !state.status.is_empty() {
        ui.label(&state.status);
    }
}

/// Loudness section: per-clip peak/RMS readout, Normalize to Peak/RMS, Match Loudness (>=2 clips).
fn loudness_ui(
    ui: &mut egui::Ui,
    state: &mut LoudnessState,
    project: &mut Project,
    selection: &[Id],
    waveforms: &mut WaveformCache,
    undo: &mut dyn FnMut(&Project),
    changed: &mut bool,
) {
    let ids = audio_targets(project, selection);
    for &id in &ids {
        let Some(c) = project.clip(id) else { continue };
        let (name, src_in, src_end, asset, audio_stream) =
            (c.name.clone(), c.src_in, c.src_in + c.duration * c.speed, c.asset, c.audio_stream);
        let Some(path) = project.asset(asset).map(|a| a.path.clone()) else { continue };
        let Some(peaks) = waveforms.get(&path, audio_stream) else { continue };
        let lv = analysis::levels(&peaks, src_in, src_end);
        ui.label(format!("{name}: peak {:.1} dB, rms {:.1} dB", lv.peak_db, lv.rms_db));
    }
    ui.add(DragValue::new(&mut state.target_dbfs).range(-60.0..=0.0).suffix(" dBFS"));
    ui.horizontal(|ui| {
        if ui.add_enabled(!ids.is_empty(), Button::new("Normalize to Peak")).clicked() {
            undo(project);
            let n = analysis::normalize(project, &ids, state.target_dbfs, NormMode::Peak, &mut asset_peaks(waveforms));
            state.status = format!("Normalized {n} clip(s) (peak)");
            if n > 0 {
                *changed = true;
            }
        }
        if ui.add_enabled(!ids.is_empty(), Button::new("Normalize to RMS")).clicked() {
            undo(project);
            let n = analysis::normalize(project, &ids, state.target_dbfs, NormMode::Rms, &mut asset_peaks(waveforms));
            state.status = format!("Normalized {n} clip(s) (RMS)");
            if n > 0 {
                *changed = true;
            }
        }
        if ui.add_enabled(ids.len() >= 2, Button::new("Match Loudness")).clicked() {
            undo(project);
            let n = analysis::match_loudness(project, &ids, &mut asset_peaks(waveforms));
            state.status = format!("Matched {n} clip(s)");
            if n > 0 {
                *changed = true;
            }
        }
    });
    if !state.status.is_empty() {
        ui.label(&state.status);
    }
}

/// Duck section: pick one Music clip + any number of Dialogue clips from the current selection, then
/// write ducking keyframes on the music clip via `analysis::duck`.
fn duck_ui(
    ui: &mut egui::Ui,
    state: &mut DuckState,
    project: &mut Project,
    selection: &[Id],
    waveforms: &mut WaveformCache,
    undo: &mut dyn FnMut(&Project),
    changed: &mut bool,
) {
    let targets = audio_targets(project, selection);
    for &id in &targets {
        let Some(name) = project.clip(id).map(|c| c.name.clone()) else { continue };
        ui.horizontal(|ui| {
            ui.label(&name);
            if ui.selectable_label(state.music == Some(id), "Music").clicked() {
                state.music = if state.music == Some(id) { None } else { Some(id) };
                state.dialogue.remove(&id);
            }
            let mut is_dlg = state.dialogue.contains(&id);
            if ui.checkbox(&mut is_dlg, "Dialogue").changed() {
                if is_dlg {
                    state.dialogue.insert(id);
                    if state.music == Some(id) {
                        state.music = None;
                    }
                } else {
                    state.dialogue.remove(&id);
                }
            }
        });
    }
    ui.add(DragValue::new(&mut state.depth_db).range(-60.0..=0.0).suffix(" dB"));
    ui.add(DragValue::new(&mut state.ramp_ms).range(0..=5000).suffix(" ms"));
    let ready = state.music.is_some() && !state.dialogue.is_empty();
    if ui.add_enabled(ready, Button::new("Apply Ducking")).clicked() {
        if let Some(music) = state.music {
            undo(project);
            let dialogue: Vec<Id> = state.dialogue.iter().copied().collect();
            let n = analysis::duck(
                project,
                music,
                &dialogue,
                state.depth_db as f64,
                state.ramp_ms as f64 / 1000.0,
                &mut asset_peaks(waveforms),
            );
            state.status = format!("Wrote {n} keyframe(s)");
            if n > 0 {
                *changed = true;
            }
        }
    }
    if !state.status.is_empty() {
        ui.label(&state.status);
    }
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
    fn detections_are_applied_right_to_left() {
        let mut p = Project::new();
        p.tracks[1].clips.push(Clip::new(1001, ClipKind::Audio, "a", 0.0, 10.0));
        p.tracks[1].clips.push(Clip::new(1002, ClipKind::Audio, "b", 10.0, 10.0));
        let det = |clip| Detection {
            clip,
            cuts: Vec::new(),
            quiet: Vec::new(),
            included: Vec::new(),
            kind: "Silence",
            kept: 0,
            kept_secs: 0.0,
        };
        let mut cached = vec![det(1001), det(1002)];
        sort_right_to_left(&p, &mut cached);
        assert_eq!(
            cached.iter().map(|d| d.clip).collect::<Vec<_>>(),
            vec![1002, 1001],
            "the later clip must be cut before ripple moves it"
        );
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
            rel_path: None,
            parent: None,
            range: None,
            effects: Vec::new(),
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
            rel_path: None,
            parent: None,
            range: None,
            effects: Vec::new(),
        });
        project.insert_asset_clips(aid, 0.0, Some(0));
        let aud = project.tracks[1].clips[0].id;
        let mut waves = WaveformCache::new(ctx.clone(), Backend::Ffmpeg);
        let mut state = AutoCutState::default();
        let settings = Settings::default();
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
                            let (changed, marked) =
                                show(ui, &mut state, &mut project, &selection, &mut waves, &settings, &pal, &mut undo);
                            assert!(!changed, "nothing should change without user action");
                            assert!(marked.is_empty(), "nothing should be marked without user action");
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

    /// The 4 new sections (Scene cuts/Beats/Loudness/Duck) must not request a repaint on their own —
    /// they only draw text/buttons from already-known state, same idle-CPU-0% contract as every other
    /// pane (see selftest.rs's own `idle_repaint` step; no shared `assert_no_idle_repaint` harness
    /// helper exists yet in this crate — forgiveness (wave 1) hasn't landed one — so this follows the
    /// same `ctx.has_requested_repaint()` check selftest.rs itself uses). Checked both CLOSED (the
    /// normal `show()` path, sections collapsed by default) and OPEN (the section fns called directly,
    /// as `CollapsingHeader` would once expanded — sidesteps depending on egui's undocumented
    /// persisted-open-state id scheme, which `.id_salt(...)` alone doesn't make predictable here).
    #[test]
    fn show_smoke_new_sections_no_idle_repaint() {
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
            rel_path: None,
            parent: None,
            range: None,
            effects: Vec::new(),
        });
        project.insert_asset_clips(aid, 0.0, Some(0));
        let vid = project.tracks[0].clips[0].id;
        let aud = project.tracks[1].clips[0].id;

        // ---- closed: the sections start collapsed, exercised through the real show() ----
        let ctx = egui::Context::default();
        let mut waves = WaveformCache::new(ctx.clone(), Backend::Ffmpeg);
        let mut state = AutoCutState::default();
        state.scene_cuts.clip = Some(vid);
        let settings = Settings::default();
        let pal = Palette::new(true, Color32::WHITE);
        let mut undos = 0;
        for _ in 0..30 {
            let _ = ctx.run(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 500.0))),
                    events: Vec::<Event>::new(),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let mut undo = |_: &Project| undos += 1;
                        show(ui, &mut state, &mut project, &[aud], &mut waves, &settings, &pal, &mut undo);
                    });
                },
            );
        }
        assert!(!ctx.has_requested_repaint(), "the collapsed Auto-cut pane must not request a repaint");
        assert_eq!(undos, 0, "no button was clicked");

        // ---- open: each section's own content, as if its CollapsingHeader were expanded ----
        let ctx2 = egui::Context::default();
        let mut waves2 = WaveformCache::new(ctx2.clone(), Backend::Ffmpeg);
        let mut sc = SceneCutState { clip: Some(vid), ..SceneCutState::default() };
        let mut bt = BeatsState::default();
        let mut ld = LoudnessState::default();
        let mut dk = DuckState::default();
        let mut changed = false;
        let mut marked = Vec::new();
        let mut undos2 = 0;
        for _ in 0..30 {
            let _ = ctx2.run(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 500.0))),
                    events: Vec::<Event>::new(),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let mut undo = |_: &Project| undos2 += 1;
                        scene_cuts_ui(ui, &mut sc, &mut project, &[aud], &mut undo, &mut changed, &mut marked);
                        beats_ui(
                            ui,
                            &mut bt,
                            &mut project,
                            &[aud],
                            &mut waves2,
                            &settings,
                            &mut undo,
                            &mut changed,
                            &mut marked,
                        );
                        loudness_ui(ui, &mut ld, &mut project, &[aud], &mut waves2, &mut undo, &mut changed);
                        duck_ui(ui, &mut dk, &mut project, &[aud], &mut waves2, &mut undo, &mut changed);
                    });
                },
            );
        }
        assert!(!ctx2.has_requested_repaint(), "an expanded Auto-cut section must not request a repaint");
        assert_eq!(undos2, 0, "no button was clicked");
        assert!(!changed && marked.is_empty(), "nothing should change without user action");
    }
}
