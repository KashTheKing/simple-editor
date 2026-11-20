//! ---- ws:addon-deepfilternet ----
//! Clean Audio: the Inspector's Audio-section row ([Clean] [Strength] [A | B], plus an Install prompt
//! while the speech-denoise add-on is missing) and the pure project edits behind it. A Clean re-points
//! the clip at a cleaned WAV in the cache (`engine::deepfilter`); A / B flips it between the original
//! file and that WAV, one undo step each, so the two can be compared while playing. The original file
//! and asset are never changed.
//!
//! The row cannot start a job itself (the Inspector has the project, not the App): a click leaves a
//! request here and `ui::app::denoise_ctl::tick` picks it up on the same frame.

use crate::engine::deepfilter::{self, Sidecar};
use crate::model::{Id, Project, TrackKind};
use eframe::egui;
use std::path::Path;

/// A Clean asked for by the Inspector row: the clips and the strength.
#[derive(Clone, Debug, PartialEq)]
pub struct CleanRequest {
    pub ids: Vec<Id>,
    pub strength: f64,
}

static REQUEST: std::sync::Mutex<Option<CleanRequest>> = std::sync::Mutex::new(None);

pub fn request_clean(ids: Vec<Id>, strength: f64) {
    *REQUEST.lock().unwrap_or_else(|e| e.into_inner()) = Some(CleanRequest { ids, strength });
}

pub(crate) fn take_request() -> Option<CleanRequest> {
    REQUEST.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// The clips a Clean or an A / B acts on: `ids` and their linked partners, the ones on audio tracks.
pub fn targets(project: &Project, ids: &[Id]) -> Vec<Id> {
    project
        .expand_links(ids)
        .into_iter()
        .filter(|&id| project.track_of(id).is_some_and(|t| project.tracks[t].kind == TrackKind::Audio))
        .collect()
}

fn same_path(a: &str, b: &str) -> bool {
    Path::new(a) == Path::new(b) || a.eq_ignore_ascii_case(b)
}

/// The source seconds a clip plays: `(src_in, src_in + duration·speed)`, or up to the end of its file
/// when a speed curve drives it.
pub(crate) fn window(project: &Project, id: Id) -> Option<(f64, f64)> {
    let c = project.clip(id)?;
    let len = project.asset(c.asset).map(|a| a.duration).filter(|d| *d > 0.0).unwrap_or(f64::INFINITY);
    Some(crate::engine::enhance::source_range(c, len, 1000.0, 0.0))
}

/// Where a clip stands in an A / B.
#[derive(Clone, Debug, PartialEq)]
pub enum Ab {
    /// Plays its original file; `b` is a cleaned asset in the project that covers it (None = none yet).
    A { b: Option<(Id, Sidecar)> },
    /// Plays a cleaned file; `a` is the original asset (None = no longer in the project).
    B { a: Option<Id>, side: Sidecar },
}

/// `sidecar` reads a cleaned file's sidecar (`deepfilter::sidecar_of` in the app; a map in tests).
pub fn ab_state(project: &Project, id: Id, sidecar: &dyn Fn(&str) -> Option<Sidecar>) -> Option<Ab> {
    let c = project.clip(id)?;
    let asset = project.asset(c.asset)?;
    if let Some(side) = sidecar(&asset.path) {
        let a = project.assets.iter().find(|x| same_path(&x.path, &side.src) && sidecar(&x.path).is_none()).map(|x| x.id);
        return Some(Ab::B { a, side });
    }
    let (lo, hi) = window(project, id)?;
    let b = project
        .assets
        .iter()
        .rev() // the newest Clean first
        .filter_map(|x| sidecar(&x.path).map(|s| (x.id, s)))
        .find(|(_, s)| same_path(&s.src, &asset.path) && s.covers(lo, hi));
    Some(Ab::A { b })
}

/// Point `clip` at `asset`, moving `src_in` by `shift` (the new file's t=0 is a different source second).
/// False (nothing changed) on a locked track or a missing asset.
pub fn repoint(project: &mut Project, clip: Id, asset: Id, shift: f64) -> bool {
    let Some(c) = project.clip(clip) else { return false };
    let (src_in, freeze) = (c.src_in, c.freeze);
    if !project.replace_clip(clip, asset) {
        return false;
    }
    let Some(c) = project.clip_mut(clip) else { return false };
    c.src_in = (src_in + shift).max(0.0);
    c.freeze = freeze.map(|f| (f + shift).max(0.0));
    true
}

/// A / B: every target clip to its original (`b` false) or its cleaned file (`b` true). Returns how many
/// clips changed.
pub fn set_ab(project: &mut Project, ids: &[Id], b: bool, sidecar: &dyn Fn(&str) -> Option<Sidecar>) -> usize {
    let mut n = 0;
    for id in targets(project, ids) {
        let ok = match ab_state(project, id, sidecar) {
            Some(Ab::B { a: Some(orig), side }) if !b => repoint(project, id, orig, side.t0),
            Some(Ab::A { b: Some((clean, side)) }) if b => repoint(project, id, clean, -side.t0),
            _ => false,
        };
        n += ok as usize;
    }
    n
}

fn live_sidecar(path: &str) -> Option<Sidecar> {
    deepfilter::sidecar_of(Path::new(path))
}

/// The Clean Audio row, inside the Inspector's Audio section. Returns true if the project changed (A / B).
pub fn row(ui: &mut egui::Ui, project: &mut Project, ids: &[Id], undo: &mut dyn FnMut(&Project)) -> bool {
    let clips = targets(project, ids);
    let Some(&first) = clips.first() else { return false };
    let state = ab_state(project, first, &live_sidecar);
    let sid = egui::Id::new("denoise_strength");
    let current = match &state {
        Some(Ab::B { side, .. }) => Some(side.strength),
        _ => None,
    };
    let mut strength: f64 =
        ui.data(|d| d.get_temp(sid)).or(current).unwrap_or(deepfilter::DEFAULT_STRENGTH);
    let mut changed = false;
    let installed = deepfilter::available();
    let engine = if installed { deepfilter::Engine::DeepFilter } else { deepfilter::Engine::Ffmpeg };
    crate::ui::prop_field(ui, "Clean Audio", |ui| {
        let tip = format!(
            "Remove background noise from speech ({}). The original file is kept; A / B compares them.",
            engine.label()
        );
        if ui.button("Clean").on_hover_text(tip).clicked() {
            request_clean(clips.clone(), strength);
        }
        let mut pct = strength * 100.0;
        let r = ui
            .add(egui::DragValue::new(&mut pct).range(0.0..=100.0).suffix(" %").max_decimals(0).speed(1.0))
            .on_hover_text("Strength: how much of the noise is removed. Click Clean again to apply a new strength.");
        if r.changed() {
            strength = pct / 100.0;
            ui.data_mut(|d| d.insert_temp(sid, strength));
        }
        if let Some(st) = &state {
            let (on_b, has_other) = match st {
                Ab::A { b } => (false, b.is_some()),
                Ab::B { a, .. } => (true, a.is_some()),
            };
            ui.add_enabled_ui(has_other || on_b, |ui| {
                if ui.selectable_label(!on_b, "A").on_hover_text("Original").clicked() && on_b && has_other {
                    undo(project);
                    changed |= set_ab(project, &clips, false, &live_sidecar) > 0;
                }
            });
            ui.add_enabled_ui(has_other || !on_b, |ui| {
                if ui.selectable_label(on_b, "B").on_hover_text("Cleaned").clicked() && !on_b && has_other {
                    undo(project);
                    changed |= set_ab(project, &clips, true, &live_sidecar) > 0;
                }
            });
        }
    });
    if !installed {
        install_prompt(ui);
    }
    changed
}

/// The inline offer to install the add-on (Clean still works without it, through ffmpeg afftdn).
pub fn install_prompt(ui: &mut egui::Ui) {
    crate::ui::prop_field(ui, "", |ui| match deepfilter::installing() {
        Some(p) => {
            ui.add(egui::Spinner::new());
            ui.weak(format!("{:.0}%", p.fraction() * 100.0));
        }
        None => {
            if ui
                .small_button("Install speech denoise")
                .on_hover_text(
                    "DeepFilterNet removes fans, traffic and room noise far better than the built-in filter. \
                     About 27 MB, downloaded once; remove it any time in Settings ▸ General.",
                )
                .clicked()
            {
                deepfilter::start_install();
            }
            if let Some(e) = deepfilter::install_error() {
                ui.colored_label(ui.visuals().error_fg_color, "install failed").on_hover_text(e);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::deepfilter::Engine;
    use crate::model::{Asset, Clip, ClipKind};

    fn asset(id: Id, path: &str) -> Asset {
        Asset {
            id,
            path: path.into(),
            kind: ClipKind::Audio,
            duration: 60.0,
            width: 0,
            height: 0,
            fps: 0.0,
            audio_streams: vec![],
            codec: String::new(),
            folder: String::new(),
            tags: vec![],
            label: 0,
            description: String::new(),
            rel_path: None,
            parent: None,
            range: None,
            proxy: None,
            effects: vec![],
            html: None,
        }
    }

    /// Audio clip 10 plays 5..7 s of `C:/v/talk.wav` (asset 1); asset 2 is a Clean of 4..8 s.
    fn project() -> Project {
        let mut p = Project::new();
        p.assets.push(asset(1, "C:/v/talk.wav"));
        p.assets.push(asset(2, "C:/cache/denoise/talk.1.wav"));
        let ti = p.audio_tracks()[0];
        let mut c = Clip::new(10, ClipKind::Audio, "talk", 3.0, 2.0);
        c.asset = 1;
        c.src_in = 5.0;
        p.tracks[ti].clips.push(c);
        p
    }

    fn side(path: &str) -> Option<Sidecar> {
        path.contains("/denoise/").then(|| Sidecar {
            src: "c:/V/TALK.wav".into(),
            t0: 4.0,
            len: 4.0,
            strength: 0.6,
            engine: Engine::DeepFilter,
        })
    }

    #[test]
    fn a_b_flips_between_the_original_and_the_cleaned_file_keeping_the_timing() {
        let mut p = project();
        assert!(matches!(ab_state(&p, 10, &side), Some(Ab::A { b: Some((2, _)) })));
        assert_eq!(set_ab(&mut p, &[10], true, &side), 1);
        let c = p.clip(10).unwrap();
        assert_eq!((c.asset, c.src_in, c.start, c.duration), (2, 1.0, 3.0, 2.0));
        assert!(matches!(ab_state(&p, 10, &side), Some(Ab::B { a: Some(1), .. })));
        // already on B: nothing to do
        assert_eq!(set_ab(&mut p, &[10], true, &side), 0);
        assert_eq!(set_ab(&mut p, &[10], false, &side), 1);
        let c = p.clip(10).unwrap();
        assert_eq!((c.asset, c.src_in), (1, 5.0));
        // the original is never touched
        assert_eq!(p.asset(1).unwrap().path, "C:/v/talk.wav");
    }

    #[test]
    fn a_clean_that_does_not_cover_the_clip_is_not_offered() {
        let mut p = project();
        p.clip_mut(10).unwrap().src_in = 1.0; // now plays 1..3 s; the Clean is 4..8 s
        assert_eq!(ab_state(&p, 10, &side), Some(Ab::A { b: None }));
        assert_eq!(set_ab(&mut p, &[10], true, &side), 0);
        assert_eq!(p.clip(10).unwrap().asset, 1);
    }

    #[test]
    fn only_audio_track_clips_are_targets_and_locked_tracks_refuse() {
        let mut p = project();
        let vt = p.video_tracks()[0];
        let mut v = Clip::new(11, ClipKind::Video, "pic", 3.0, 2.0);
        v.asset = 1;
        p.tracks[vt].clips.push(v);
        assert_eq!(targets(&p, &[10, 11]), vec![10]);
        let at = p.track_of(10).unwrap();
        p.tracks[at].locked = true;
        assert_eq!(set_ab(&mut p, &[10], true, &side), 0);
    }

    #[test]
    fn the_inspector_request_is_taken_once() {
        request_clean(vec![10], 0.5);
        assert_eq!(take_request(), Some(CleanRequest { ids: vec![10], strength: 0.5 }));
        assert_eq!(take_request(), None);
    }
}
