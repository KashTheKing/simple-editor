//! ---- ws:gap-group-sync ----
//! Clip groups and Clip ▸ Sync by Audio.
//!
//! A group (`Clip.group`, Ctrl+G / Ctrl+Shift+G) is a selection unit across any tracks: a click on a
//! member selects them all (`timeline::group_unit`), so move, delete, copy / paste and ripple run on the
//! whole group through the ordinary selection path. Alt+click picks one clip.
//!
//! Sync by Audio lines the selected clips up on the first one's sound with the same cross-correlation
//! multicam uses (`analysis::xcorr_match`, the scored `xcorr_offset`). One undo step; a weak match moves
//! nothing.

use super::feedback::{Toast, ToastKind};
use super::tools_helpers::*;
use super::*;
use crate::engine::analysis;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::media::waveform::Peaks;
use std::sync::Arc;

/// Under this correlation the clips probably don't share a recording: report, don't move.
const WEAK: f64 = 0.4;

/// One clip's place against the first: `offset` is how far its sound lags the first's in source time,
/// `dt` how far it moves on the timeline, `unit` the clip and its linked partners (they move together).
#[derive(Debug)]
pub(crate) struct SyncHit {
    pub clip: Id,
    pub offset: f64,
    pub confidence: f64,
    pub dt: f64,
    pub unit: Vec<Id>,
}

/// The selection's link units in order, each as (the clip whose audio is read, the unit).
fn units(p: &Project, ids: &[Id]) -> Vec<(Id, Vec<Id>)> {
    let mut out: Vec<(Id, Vec<Id>)> = Vec::new();
    for &id in ids {
        if p.clip(id).is_none() || out.iter().any(|(_, u)| u.contains(&id)) {
            continue;
        }
        let unit = p.linked(id);
        let rep = unit.iter().copied().find(|&c| p.clip(c).is_some_and(|c| c.kind == ClipKind::Audio)).unwrap_or(id);
        out.push((rep, unit));
    }
    out
}

/// What the correlation needs, copied out of the project so it can run off the UI thread: each unit's
/// audio clip (as it was), the unit and the clip's source peaks. The first entry is the one that stays.
pub(crate) struct SyncInput {
    clips: Vec<(Clip, Vec<Id>, Arc<Peaks>)>,
}

impl SyncInput {
    /// True while every clip still sits where it did when the input was gathered (same place, same
    /// source window), so the planned moves still mean what they meant.
    fn still_valid(&self, p: &Project) -> bool {
        self.clips.iter().all(|(c, unit, _)| {
            p.clip(c.id).is_some_and(|now| now.start == c.start && now.src_in == c.src_in)
                && unit.iter().all(|&id| p.clip(id).is_some())
        })
    }
}

/// Read the clips and their peaks (cheap: the peaks are cached `Arc`s). `peaks_of` reads a clip's
/// source audio.
pub(crate) fn gather(
    p: &Project,
    ids: &[Id],
    peaks_of: &mut dyn FnMut(&Clip) -> Result<Arc<Peaks>, String>,
) -> Result<SyncInput, String> {
    let units = units(p, ids);
    if units.len() < 2 {
        return Err("Select two or more clips with audio".into());
    }
    let mut clips = Vec::with_capacity(units.len());
    for (rep, unit) in units {
        let c = p.clip(rep).ok_or("no such clip")?.clone();
        // ponytail: source time = timeline time only at normal speed; retimed clips are refused
        if (c.speed - 1.0).abs() > 1e-9 || c.reverse {
            return Err(format!("{} is retimed - Sync by Audio needs clips at normal speed", c.name));
        }
        if !p.asset(c.asset).is_some_and(|a| !a.audio_streams.is_empty()) {
            return Err(format!("{} has no audio", c.name));
        }
        let peaks = peaks_of(&c)?;
        clips.push((c, unit, peaks));
    }
    Ok(SyncInput { clips })
}

/// Where each clip after the first goes. The heavy part (up to +-120 s of lags over whole files): the
/// Clip menu / hotkey path runs it on a worker thread.
pub(crate) fn correlate(input: &SyncInput) -> Vec<SyncHit> {
    let (a, _, pa) = &input.clips[0];
    input.clips[1..]
        .iter()
        .map(|(b, unit, pb)| {
            let (offset, confidence) = analysis::xcorr_match(pa, pb, 120.0).unwrap_or((0.0, 0.0));
            // b's source at t + offset sounds like a's at t: put both under the same timeline instant
            let start = a.start - a.src_in + b.src_in - offset;
            SyncHit { clip: b.id, offset, confidence, dt: start - b.start, unit: unit.clone() }
        })
        .collect()
}

/// `gather` + `correlate` in one go (MCP, tests).
pub(crate) fn plan(
    p: &Project,
    ids: &[Id],
    peaks_of: &mut dyn FnMut(&Clip) -> Result<Arc<Peaks>, String>,
) -> Result<Vec<SyncHit>, String> {
    gather(p, ids, peaks_of).map(|i| correlate(&i))
}

fn gather_app(app: &mut App, ids: &[Id]) -> Result<SyncInput, String> {
    let App { project, waveforms, .. } = app;
    gather(project, ids, &mut |c| {
        let path = project.asset(c.asset).map(|a| a.path.clone()).unwrap_or_default();
        waveforms.get(&path, c.audio_stream).ok_or_else(|| "Waveforms still loading - try again in a moment".into())
    })
}

/// Move the clips when every match is good. `Ok(moved)`; on `Err` the project may be half moved -
/// callers restore their snapshot.
fn apply(project: &mut Project, hits: &[SyncHit]) -> Result<bool, String> {
    if hits.iter().any(|h| h.confidence < WEAK) {
        return Ok(false);
    }
    for h in hits {
        if h.dt.abs() > 1e-9 && !project.move_clips(&h.unit, h.dt, 0, None) {
            let name = project.clip(h.clip).map_or(String::new(), |c| c.name.clone());
            return Err(format!("{name} can't move {:+.2} s there (another clip or 0:00 is in the way)", h.dt));
        }
    }
    Ok(true)
}

/// Synchronous plan + apply (MCP, like `audio.xcorr` and multicam sync).
fn sync(app: &mut App, ids: &[Id]) -> Result<(Vec<SyncHit>, bool), String> {
    let input = gather_app(app, ids)?;
    let hits = correlate(&input);
    let moved = apply(&mut app.project, &hits)?;
    Ok((hits, moved))
}

/// A Sync by Audio running on its worker thread (`App::group_sync`), picked up by `tick`.
#[derive(Default)]
pub(crate) struct SyncState {
    pending: Option<(Arc<SyncInput>, std::sync::mpsc::Receiver<Vec<SyncHit>>)>,
}

fn summary(app: &App, hits: &[SyncHit]) -> String {
    let name = |id| app.project.clip(id).map_or(String::new(), |c| c.name.clone());
    hits.iter().map(|h| format!("{} {:+.2} s (match {:.2})", name(h.clip), h.dt, h.confidence)).collect::<Vec<_>>().join(", ")
}

fn sync_selection(app: &mut App) {
    if app.group_sync.pending.is_some() {
        app.toast("Already syncing - one moment");
        return;
    }
    let ids = app.selection.clone();
    let input = match gather_app(app, &ids) {
        Ok(i) => Arc::new(i),
        Err(e) => {
            app.push_toast(Toast::new(e).kind(ToastKind::Warn));
            return;
        }
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let job = input.clone();
    // `tick` polls (and keeps frames coming) until the result lands
    std::thread::spawn(move || {
        let _ = tx.send(correlate(&job));
    });
    app.group_sync.pending = Some((input, rx));
}

/// FRAME_HOOKS: land a finished Sync by Audio - one undo step - unless the clips moved meanwhile.
pub(super) fn tick(app: &mut App, ctx: &egui::Context) {
    let Some((_, rx)) = &app.group_sync.pending else { return };
    let hits = match rx.try_recv() {
        Ok(h) => h,
        Err(std::sync::mpsc::TryRecvError::Empty) => {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
            return;
        }
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
            app.group_sync.pending = None;
            app.push_toast(Toast::new("Sync by Audio failed").kind(ToastKind::Warn));
            return;
        }
    };
    let Some((input, _)) = app.group_sync.pending.take() else { return };
    if !input.still_valid(&app.project) {
        app.push_toast(Toast::new("The clips changed while syncing, nothing moved - try again").kind(ToastKind::Warn));
        return;
    }
    land(app, &hits);
}

fn land(app: &mut App, hits: &[SyncHit]) {
    let before = app.project.clone();
    match apply(&mut app.project, hits) {
        Ok(true) => {
            app.push_undo_labeled(before, "Sync by audio");
            app.after_edit();
            let msg = format!("Synced by audio: {}", summary(app, hits));
            app.push_toast(Toast::new(msg).kind(ToastKind::Success).undo(Action::Undo));
        }
        Ok(false) => {
            let msg = format!("The audio doesn't match well enough to sync, nothing moved: {}", summary(app, hits));
            app.push_toast(Toast::new(msg).kind(ToastKind::Warn));
        }
        Err(e) => {
            app.project = before;
            app.push_toast(Toast::new(e).kind(ToastKind::Warn));
        }
    }
}

fn group(app: &mut App, ids: &[Id]) -> Option<Id> {
    let before = app.project.clone();
    let g = app.project.group_clips(ids)?;
    app.push_undo_labeled(before, "Group");
    app.selection = crate::ui::timeline::group_unit(&app.project, &[ids[0]], true);
    app.after_edit();
    Some(g)
}

fn ungroup(app: &mut App, ids: &[Id]) -> usize {
    let before = app.project.clone();
    let n = app.project.ungroup_clips(ids);
    if n > 0 {
        app.push_undo_labeled(before, "Ungroup");
        app.after_edit();
    }
    n
}

pub(super) fn act(app: &mut App, a: Action) -> bool {
    match a {
        Action::GroupClips => {
            let ids = app.selection.clone();
            if group(app, &ids).is_none() {
                app.toast("Select two or more clips to group");
            }
        }
        Action::UngroupClips => {
            let ids = app.selection.clone();
            if ungroup(app, &ids) == 0 {
                app.toast("The selection isn't grouped");
            }
        }
        Action::SyncByAudio => sync_selection(app),
        _ => return false,
    }
    true
}

fn ids_or_selection(app: &App, args: &Value) -> Vec<Id> {
    arg_ids(args, "clip_ids").unwrap_or_else(|| app.selection.clone())
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "timeline.group",
        desc: "Group clips (any tracks; their linked A/V partners too) so a click selects them all and they \
               move, copy and delete together. Grouping clips already in groups merges them. Returns the group id.",
        args: &["clip_ids:array:false:defaults to the selection; 2+ clips"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let ids = ids_or_selection(app, args);
            let g = app.project.group_clips(&ids).ok_or("need 2 or more clips to group")?;
            let members = app.project.expand_groups(&ids);
            Ok(ToolOutcome::Done(json!({"ok": true, "group": g, "clip_ids": members})))
        },
    },
    ToolDef {
        name: "timeline.ungroup",
        desc: "Dissolve the groups the clips belong to. Returns how many clips left a group.",
        args: &["clip_ids:array:false:defaults to the selection"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let ids = ids_or_selection(app, args);
            Ok(ToolOutcome::Done(json!({"ok": true, "ungrouped": app.project.ungroup_clips(&ids)})))
        },
    },
    ToolDef {
        name: "timeline.sync_audio",
        desc: "Line clips up by their sound (audio cross-correlation, as multicam sync). The first clip stays; \
               each other clip (with its linked partners) moves. Returns per clip offset_s (source lag), moved_s \
               (timeline move) and confidence (0-1); if any confidence is under 0.4 nothing moves (moved: false).",
        args: &["clip_ids:array:false:defaults to the selection; first = the one that stays"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let ids = ids_or_selection(app, args);
            let (hits, moved) = sync(app, &ids)?;
            let rows: Vec<Value> = hits
                .iter()
                .map(|h| json!({"clip_id": h.clip, "offset_s": h.offset, "moved_s": h.dt, "confidence": h.confidence}))
                .collect();
            Ok(ToolOutcome::Done(json!({"ok": moved, "moved": moved, "clips": rows})))
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::waveform::PEAKS_PER_SEC;
    use crate::model::{Asset, AudioStreamInfo};

    /// Noise-like bursts (a fixed pseudo-random pattern) starting `lead` seconds into a `dur` s file.
    fn bursts(lead: f64, dur: f64) -> Peaks {
        let n = (dur * PEAKS_PER_SEC as f64) as usize;
        let mut max = vec![0.01f32; n];
        let mut x: u32 = 7;
        for k in 0..(20.0 * PEAKS_PER_SEC as f64) as usize {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let i = (lead * PEAKS_PER_SEC as f64) as usize + k;
            if i < n {
                max[i] = ((x >> 16) % 1000) as f32 / 1000.0;
            }
        }
        let min = max.iter().map(|v| -v).collect();
        Peaks { min, max }
    }

    fn audio_project() -> (Project, Id, Id) {
        let mut p = Project::default();
        let mut ids = Vec::new();
        for name in ["a.wav", "b.wav"] {
            let a = Asset {
                id: 0,
                path: name.into(),
                kind: ClipKind::Audio,
                duration: 40.0,
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
                html: None,
                proxy: None,
            };
            let aid = p.add_asset(a);
            ids.extend(p.insert_asset_clips(aid, 0.0, None));
        }
        (p, ids[0], ids[1])
    }

    #[test]
    fn sync_by_audio_lands_within_a_frame() {
        let (mut p, a, b) = audio_project();
        // the clap is 3.0 s into a's file and 5.36 s into b's: b must start 2.36 s earlier than a
        p.clip_mut(a).unwrap().start = 10.0;
        let pa = Arc::new(bursts(3.0, 40.0));
        let pb = Arc::new(bursts(5.36, 40.0));
        let hits = plan(&p, &[a, b], &mut |c| Ok(if c.id == a { pa.clone() } else { pb.clone() })).unwrap();
        assert_eq!(hits.len(), 1);
        let h = &hits[0];
        assert!(h.confidence > 0.8, "{h:?}");
        assert!(p.move_clips(&h.unit, h.dt, 0, None));
        let (sa, sb) = (p.clip(a).unwrap().start, p.clip(b).unwrap().start);
        let frame = 1.0 / 30.0;
        assert!(((sa - 3.0) - (sb - 5.36)).abs() < frame, "a clap at {}, b clap at {}", sa - 3.0, sb - 5.36);
        assert_eq!(sa, 10.0, "the first clip stays put");
    }

    #[test]
    fn unrelated_audio_is_a_weak_match() {
        let (p, a, b) = audio_project();
        let pa = Arc::new(bursts(3.0, 40.0));
        let flat = Arc::new(Peaks { min: vec![-0.5; 4000], max: vec![0.5; 4000] });
        let hits = plan(&p, &[a, b], &mut |c| Ok(if c.id == a { pa.clone() } else { flat.clone() })).unwrap();
        assert!(hits[0].confidence < WEAK, "{:?}", hits[0]);
    }

    fn app() -> App {
        let scratch = std::env::temp_dir().join(format!("se-group-test-{}", std::process::id()));
        *crate::settings::TEST_DIR.lock().unwrap_or_else(|e| e.into_inner()) = Some(scratch);
        let app = App::build(&egui::Context::default(), None, None);
        *crate::settings::TEST_DIR.lock().unwrap_or_else(|e| e.into_inner()) = None;
        app
    }

    /// The Clip menu path: the correlation runs on a worker, `tick` lands it as one undo step - and drops
    /// it if the clips moved while it ran.
    #[test]
    fn worker_sync_lands_once_and_respects_edits() {
        let ctx = egui::Context::default();
        let pa = Arc::new(bursts(3.0, 40.0));
        let pb = Arc::new(bursts(5.36, 40.0));
        for edit_meanwhile in [false, true] {
            let mut app = app();
            let (p, a, b) = audio_project();
            app.project = p;
            app.project.clip_mut(a).unwrap().start = 10.0;
            let input = Arc::new(
                gather(&app.project, &[a, b], &mut |c| Ok(if c.id == a { pa.clone() } else { pb.clone() })).unwrap(),
            );
            let (tx, rx) = std::sync::mpsc::channel();
            let job = input.clone();
            std::thread::spawn(move || tx.send(correlate(&job)).unwrap()).join().unwrap();
            app.group_sync.pending = Some((input, rx));
            if edit_meanwhile {
                app.project.clip_mut(b).unwrap().start = 1.0;
            }
            let b_before = app.project.clip(b).unwrap().start;
            tick(&mut app, &ctx);
            assert!(app.group_sync.pending.is_none(), "the result was picked up");
            let sb = app.project.clip(b).unwrap().start;
            if edit_meanwhile {
                assert_eq!(sb, b_before, "a stale plan moves nothing");
            } else {
                assert!(((10.0 - 3.0) - (sb - 5.36)).abs() < 1.0 / 30.0, "b lands on a's clap: {sb}");
                app.act(Action::Undo);
                assert_eq!(app.project.clip(b).unwrap().start, b_before, "one undo step puts it back");
            }
        }
    }

    #[test]
    fn group_delete_undo_and_save_load() {
        let mut app = app();
        let a = app.project.add_text_clip(0.0, 5.0);
        let b = app.project.add_text_clip(10.0, 5.0);
        let c = app.project.add_text_clip(20.0, 5.0);
        app.selection = vec![a, b];
        app.act(Action::GroupClips);
        let g = app.project.clip(a).unwrap().group;
        assert!(g != 0 && app.project.clip(b).unwrap().group == g, "Ctrl+G groups the selection");
        assert_eq!(app.project.clip(c).unwrap().group, 0);
        // saved with the project; an ungrouped clip writes nothing, so old files load with no groups
        let json = app.project.to_json();
        let mut p = Project::from_json(&json).unwrap();
        assert_eq!(p.expand_groups(&[b]), vec![a, b]);
        assert!(!serde_json::to_string(p.clip(c).unwrap()).unwrap().contains("\"group\""));
        assert!(p.new_id() > g, "a loaded group id is never handed out again");
        // a click on one member selects both; Delete takes both, undo brings the group back
        app.selection = crate::ui::timeline::group_unit(&app.project, &[b], true);
        app.act(Action::Delete);
        assert!(app.project.clip(a).is_none() && app.project.clip(b).is_none() && app.project.clip(c).is_some());
        app.act(Action::Undo);
        assert_eq!((app.project.clip(a).unwrap().group, app.project.clip(b).unwrap().group), (g, g));
        // moving the selected group moves every member by the same amount
        app.selection = app.project.expand_groups(&[a]);
        assert!(app.project.move_clips(&app.selection.clone(), 1.0, 0, None));
        assert_eq!((app.project.clip(a).unwrap().start, app.project.clip(b).unwrap().start), (1.0, 11.0));
        // Ctrl+Shift+G dissolves it
        app.selection = vec![a];
        app.act(Action::UngroupClips);
        assert_eq!((app.project.clip(a).unwrap().group, app.project.clip(b).unwrap().group), (0, 0));
        app.act(Action::Undo);
        assert_eq!(app.project.clip(b).unwrap().group, g);
    }

    #[test]
    fn copies_are_new_groups_and_sequences_keep_theirs() {
        let mut app = app();
        let a = app.project.add_text_clip(0.0, 5.0);
        let b = app.project.add_text_clip(10.0, 5.0);
        let g = app.project.group_clips(&[a, b]).unwrap();
        let dup = app.project.duplicate(&[a, b]);
        let dg = app.project.clip(dup[0]).unwrap().group;
        assert!(dg != 0 && dg != g && app.project.clip(dup[1]).unwrap().group == dg, "a duplicate is its own group");
        // copy / paste: the same through place_clips
        let clips = vec![app.project.clip(a).unwrap().clone(), app.project.clip(b).unwrap().clone()];
        let pasted = app.project.place_clips(clips, Vec::new(), 40.0);
        let pg = app.project.clip(pasted[0]).unwrap().group;
        assert!(pg != 0 && pg != g && app.project.clip(pasted[1]).unwrap().group == pg);
        assert_eq!(app.project.expand_groups(&[a]).len(), 2, "the original group gained nobody");
        // a group made inside a sequence tab is still there after switching away and back
        let s = app.project.new_sequence("S", 1920, 1080, 30.0);
        app.enter_sequence(s);
        let x = app.project.add_text_clip(0.0, 5.0);
        let y = app.project.add_text_clip(6.0, 5.0);
        app.selection = vec![x, y];
        app.act(Action::GroupClips);
        app.act(Action::OpenParentSequence);
        assert!(app.project.clip(x).is_none());
        app.enter_sequence(s);
        assert_eq!(app.project.expand_groups(&[y]), vec![x, y]);
    }
}
