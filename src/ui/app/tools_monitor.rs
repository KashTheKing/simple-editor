//! ---- ws:pro-monitor ----
//! The 7 monitor actions (`act`) and 10 MCP tools: trim-view/compare/scopes state flips, save/apply
//! stills (existing `EffectPreset` storage — zero new storage), multicam create/sync/switch, and the
//! eyedropper's colour write-back (shared by the UI click path in `preview_pane.rs` and the `color.pick`
//! tool, so both write the exact same effect param the exact same way).
//!
//! deviation: `stats_now` below duplicates `tools_color.rs`'s private `stats_at` (fresh render + GPU
//! readback at an arbitrary time) almost verbatim — that file isn't in this workstream's Files table, so
//! rather than widen its visibility (`fn stats_at` -> `pub(super)`) this is its own few-line copy. Same
//! reasoning for `find_or_add`, a near-duplicate of `tools_color.rs`'s private `upsert_effect`.

use super::tools_args::Args;
use super::tools_helpers::*;
use super::*;
use crate::engine::gpu::FrameStats;
use crate::engine::{analysis, presets};
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::model::ops::trim::{EditPoint, Side};
use crate::ui::preview::{CompareMode, PickTarget};

/// Render at `t` through the live-preview path (never `render_frame`, which export/thumbnails also
/// call) and read back `gpu.stats()` — see this file's top-of-file deviation note.
fn stats_now(app: &mut App, t: f64) -> Option<FrameStats> {
    let w = app.project.width.max(16);
    let h = app.project.height.max(16);
    let layers = app.player.layers_once(t, w)?;
    let App { gpu, project, .. } = app;
    let gpu = gpu.as_mut()?;
    gpu.set_stats_wanted(true);
    guarded(|| gpu.render_preview_texture(project, t, w, h, &layers));
    gpu.stats().cloned()
}

fn find_or_add(c: &mut Clip, kind: EffectKind) -> &mut Effect {
    if let Some(i) = c.effects.iter().position(|e| e.kind == kind) {
        &mut c.effects[i]
    } else {
        c.effects.push(Effect::new(kind));
        c.effects.last_mut().unwrap()
    }
}

/// Rec.709-ish hue (0..360) of an RGB triple — the Qualifier centre an eyedropper pick writes.
fn rgb_hue_degrees(rgb: [u8; 3]) -> f32 {
    let (r, g, b) = (rgb[0] as f32 / 255.0, rgb[1] as f32 / 255.0, rgb[2] as f32 / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let d = max - min;
    if d < 1e-6 {
        return 0.0;
    }
    let h = if max == r {
        60.0 * (((g - b) / d).rem_euclid(6.0))
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    if h < 0.0 {
        h + 360.0
    } else {
        h
    }
}

/// The pure clip-mutation half of `write_picked_color` (find-or-add the effect, write the target's
/// fields) — split out so it's testable without a live `App`.
fn write_picked_color_on_clip(c: &mut Clip, target: PickTarget, rgb: [u8; 3]) {
    match target {
        PickTarget::Chroma => {
            let e = find_or_add(c, EffectKind::ChromaKey);
            e.params[0].value = rgb[0] as f64;
            e.params[1].value = rgb[1] as f64;
            e.params[2].value = rgb[2] as f64;
        }
        PickTarget::Qualifier => {
            let e = find_or_add(c, EffectKind::Qualifier);
            e.params[0].value = rgb_hue_degrees(rgb) as f64;
        }
    }
}

/// Shared by the eyedropper's UI click path (`preview_pane.rs`) and the `color.pick` MCP tool: writes
/// `rgb` into `clip_id`'s ChromaKey key colour or Qualifier hue centre (find-or-append the effect).
pub(super) fn write_picked_color(app: &mut App, clip_id: Id, target: PickTarget, rgb: [u8; 3]) -> Result<(), String> {
    let c = app.project.clip_mut(clip_id).ok_or("no such clip")?;
    write_picked_color_on_clip(c, target, rgb);
    Ok(())
}

/// Cross-correlate `ids[0]`'s waveform against every other clip's (their assets' stream-0 peaks,
/// `WaveformCache::get` — same "read stream 0" simplification `tools_helpers::asset_peaks` already makes
/// elsewhere in this crate). `ids.len() > 8` is refused up front (ponytail: keeps the pairwise xcorr call
/// count bounded — see the plan's own deliberate-simplifications note).
fn sync_offsets(app: &mut App, ids: &[Id]) -> Result<Vec<f64>, String> {
    if ids.len() < 2 {
        return Err("need at least 2 clips to sync".into());
    }
    if ids.len() > 8 {
        return Err("multicam sync is capped at 8 clips".into());
    }
    let paths: Option<Vec<String>> = ids
        .iter()
        .map(|&id| app.project.clip(id).and_then(|c| app.project.asset(c.asset)).map(|a| a.path.clone()))
        .collect();
    let paths = paths.ok_or("one or more clips have no asset")?;
    let peaks: Option<Vec<_>> = paths.iter().map(|p| app.waveforms.get(p, 0)).collect();
    let peaks = peaks.ok_or("waveform not ready yet — try again shortly")?;
    let mut offsets = vec![0.0];
    for p in &peaks[1..] {
        offsets.push(analysis::xcorr_offset(&peaks[0], p, 120.0).unwrap_or(0.0));
    }
    Ok(offsets)
}

/// Which video track (angle index) is currently enabled at `seq_clip`'s local time `t` — `multicam_ui`'s
/// "current" highlight and `NextAngle`/`PrevAngle`'s starting point. `None` when `seq_clip` isn't a
/// multicam sequence clip.
fn current_angle(project: &Project, seq_clip: Id, t: f64) -> Option<usize> {
    let clip = project.clip(seq_clip)?;
    if clip.kind != ClipKind::Sequence {
        return None;
    }
    let local_t = clip.src_time(t);
    let seq = project.sequence(clip.sequence)?;
    seq.tracks
        .iter()
        .filter(|tr| tr.kind == TrackKind::Video)
        .enumerate()
        .find(|(_, tr)| tr.clips.iter().any(|c| c.contains(local_t) && c.enabled))
        .map(|(i, _)| i)
}

/// The clip `NextAngle`/`PrevAngle`/the angle-grid window target: the first selected clip, else whatever
/// is under the playhead.
fn targeted_clip(app: &App) -> Option<Id> {
    app.selection.first().copied().or_else(|| app.project.clips_at(app.playhead).first().copied())
}

fn switch_angle(app: &mut App, delta: i32) {
    let Some(id) = targeted_clip(app) else {
        app.toast("Select a multicam clip first");
        return;
    };
    let angles = app.project.multicam_angles(id);
    if angles.is_empty() {
        app.toast("Not a multicam clip");
        return;
    }
    let cur = current_angle(&app.project, id, app.playhead).unwrap_or(0);
    let next = (cur as i32 + delta).rem_euclid(angles.len() as i32) as usize;
    let before = app.project.to_json();
    if app.project.multicam_switch(id, app.playhead, next) {
        app.push_undo_labeled(before, "Switch angle");
        app.after_edit();
    }
}

fn multicam_create_from_selection(app: &mut App) {
    let ids = app.selection.clone();
    if ids.len() < 2 {
        app.toast("Select at least 2 clips to create a multicam sequence");
        return;
    }
    match sync_offsets(app, &ids) {
        Ok(offsets) => {
            let before = app.project.to_json();
            if app.project.multicam_make(&ids, &offsets, "Multicam").is_some() {
                app.push_undo_labeled(before, "Create multicam");
                app.selection.clear();
                app.after_edit();
            } else {
                app.toast("Could not create the multicam sequence");
            }
        }
        Err(e) => app.toast(e),
    }
}

fn save_still_named(app: &mut App, clip_id: Option<Id>, name: Option<String>) -> Result<String, String> {
    let id = clip_id.or_else(|| app.selection.first().copied()).ok_or("no clip selected")?;
    let clip = app.project.clip(id).ok_or("no such clip")?;
    let name = name.unwrap_or_else(|| {
        let n = app.settings.effect_presets.iter().filter(|p| p.name.starts_with("Still ")).count();
        format!("Still {}", n + 1)
    });
    let preset = presets::capture_effects(&name, clip);
    app.settings.effect_presets.push(preset);
    app.settings.save();
    Ok(name)
}

/// ACT_HANDLERS entry for the 7 unbound actions. `SaveStill`/`CompareWipe`/`ToggleScopes`/
/// `ToggleTrimView` touch `Settings`/`PreviewState`, never `Project` — no undo. `MulticamCreate`/
/// `NextAngle`/`PrevAngle` call into `model/ops/multicam.rs` with `push_undo_labeled`.
pub(super) fn act(app: &mut App, a: Action) -> bool {
    match a {
        Action::ToggleTrimView => {
            app.settings.trim_view = !app.settings.trim_view;
            app.settings.save();
            true
        }
        Action::CompareWipe => {
            app.preview.compare = app.preview.compare.cycle();
            true
        }
        Action::SaveStill => {
            match save_still_named(app, None, None) {
                Ok(name) => app.toast(format!("Saved still '{name}'")),
                Err(e) => app.toast(e),
            }
            true
        }
        Action::ToggleScopes => {
            app.monitor.scopes_open = !app.monitor.scopes_open;
            true
        }
        Action::MulticamCreate => {
            multicam_create_from_selection(app);
            true
        }
        Action::NextAngle => {
            switch_angle(app, 1);
            true
        }
        Action::PrevAngle => {
            switch_angle(app, -1);
            true
        }
        _ => false,
    }
}

/// WINDOW_DRAWERS entry: the Scopes window, fed by `App.gpu.stats()` (real, not the plan's guessed
/// `gpu.frame_stats()` name — see `monitor.rs`'s deviation note).
pub(super) fn window_scopes(app: &mut App, ctx: &egui::Context) {
    let stats = app.gpu.as_ref().and_then(|g| g.stats());
    let App { monitor, settings, palette, .. } = app;
    crate::ui::scopes_ui::window(ctx, &mut monitor.scopes_open, &mut settings.scopes, stats, palette);
}

/// WINDOW_DRAWERS entry: the multicam angle grid, auto-shown (no dedicated toggle action — matches
/// trim_view's own "auto-shown when applicable" precedent) whenever the targeted clip is a multicam
/// sequence clip.
pub(super) fn window_multicam(app: &mut App, ctx: &egui::Context) {
    let Some(id) = targeted_clip(app) else { return };
    let angles = app.project.multicam_angles(id);
    if angles.is_empty() {
        return;
    }
    let cur = current_angle(&app.project, id, app.playhead).unwrap_or(0);
    let mut open = true;
    let clicked = crate::ui::multicam_ui::angle_grid(ctx, &mut open, &angles, cur, &app.palette);
    if let Some(angle) = clicked {
        let before = app.project.to_json();
        if app.project.multicam_switch(id, app.playhead, angle) {
            app.push_undo_labeled(before, "Switch angle");
            app.after_edit();
        }
    }
}

fn run(app: &mut App, name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "timeline.trim_view" => {
            app.settings.trim_view = req(arg_bool(args, "on"), "on")?;
            app.settings.save();
            Ok(json!({"ok": true}))
        }
        "timeline.dynamic_trim" => {
            let side = match req(arg_str(args, "edit_point_side"), "edit_point_side")? {
                "left" => Side::Left,
                "right" => Side::Right,
                "both" => Side::Both,
                _ => return Err("edit_point_side: left|right|both".into()),
            };
            let dt = req(arg_f64(args, "dt"), "dt")?;
            let ep0 =
                app.timeline.edit_point.ok_or("no edit point selected — call ui.action('select_edit_point') first")?;
            let ep = EditPoint { side, ..ep0 };
            let to = ep.t + dt;
            if !app.project.extend_edit(&ep, to) {
                return Err("trim refused (locked track or not enough room)".into());
            }
            app.timeline.edit_point = Some(EditPoint { t: to, ..ep });
            Ok(json!({"ok": true, "t": to}))
        }
        "preview.compare" => {
            app.preview.compare = match req(arg_str(args, "mode"), "mode")? {
                "off" => CompareMode::Off,
                "wipe" => CompareMode::Wipe(arg_f64(args, "x").unwrap_or(0.5) as f32),
                "side" => CompareMode::SideBySide,
                _ => return Err("mode: off|wipe|side".into()),
            };
            Ok(json!({"ok": true}))
        }
        "stills.save" => {
            let clip_id = arg_u64(args, "clip_id");
            let name = arg_str(args, "name").map(str::to_string);
            let saved = save_still_named(app, clip_id, name)?;
            Ok(json!({"ok": true, "name": saved}))
        }
        "stills.apply" => {
            let id = req(arg_u64(args, "clip_id"), "clip_id")?;
            let name = req(arg_str(args, "name"), "name")?;
            let preset = app
                .settings
                .effect_presets
                .iter()
                .find(|p| p.name == name)
                .cloned()
                .ok_or_else(|| format!("no saved still named '{name}'"))?;
            if !presets::apply_effects(&preset, &mut app.project, id) {
                return Err("could not apply that still".into());
            }
            Ok(json!({"ok": true}))
        }
        "scopes.read" => {
            let kind = arg_str(args, "kind").unwrap_or("all").to_string();
            let Some(stats) = stats_now(app, app.playhead) else {
                return Ok(json!({}));
            };
            // ponytail: the full per-pixel data the visual scopes draw from stays in-process (same
            // reasoning as `frame.stats`'s own doc comment) — every `kind` gets the same percentile/
            // histogram/mean summary; `kind` is echoed for parity with the UI's tabs, not a distinct shape.
            Ok(json!({
                "kind": kind, "p1": stats.p1, "p99": stats.p99, "mean": stats.mean,
                "hist": [stats.hist[0].to_vec(), stats.hist[1].to_vec(), stats.hist[2].to_vec()],
                "luma": stats.luma.to_vec(),
            }))
        }
        "multicam.sync" => {
            let ids = req(arg_ids(args, "clip_ids"), "clip_ids")?;
            Ok(json!({"offsets": sync_offsets(app, &ids)?}))
        }
        "multicam.create" => {
            let ids = req(arg_ids(args, "clip_ids"), "clip_ids")?;
            let name = arg_str(args, "name").unwrap_or("Multicam").to_string();
            let offsets = sync_offsets(app, &ids)?;
            let seq_id = app
                .project
                .multicam_make(&ids, &offsets, name)
                .ok_or("could not create the multicam sequence (need >= 2 valid clips)")?;
            Ok(json!({"ok": true, "sequence_id": seq_id}))
        }
        "multicam.switch" => {
            let seq_clip_id = req(arg_u64(args, "seq_clip_id"), "seq_clip_id")?;
            let t = Args(args).t_or_playhead("t", app);
            let angle = req(arg_u64(args, "angle"), "angle")? as usize;
            if !app.project.multicam_switch(seq_clip_id, t, angle) {
                return Err("switch failed (not a multicam clip, or angle out of range)".into());
            }
            Ok(json!({"ok": true}))
        }
        "color.pick" => {
            let x = req(arg_f64(args, "x"), "x")?;
            let y = req(arg_f64(args, "y"), "y")?;
            let target = match req(arg_str(args, "target"), "target")? {
                "chroma" => PickTarget::Chroma,
                "qualifier" => PickTarget::Qualifier,
                _ => return Err("target: chroma|qualifier".into()),
            };
            let stats = stats_now(app, app.playhead).ok_or("GPU rendering not available")?;
            let (sw, sh) = (stats.sample_w.max(1), stats.sample_h.max(1));
            let px = ((x.clamp(0.0, 0.999_999) * sw as f64) as u32).min(sw - 1);
            let py = ((y.clamp(0.0, 0.999_999) * sh as f64) as u32).min(sh - 1);
            let Some(&[r, g, b, _]) = stats.sample.get((py * sw + px) as usize) else {
                return Ok(json!({"rgb": [0, 0, 0]}));
            };
            if let Some(&id) = app.selection.first() {
                write_picked_color(app, id, target, [r, g, b])?;
            }
            Ok(json!({"rgb": [r, g, b]}))
        }
        _ => unreachable!(),
    }
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "timeline.trim_view",
        desc: "Show/hide the dual-frame trim view (only actually paints once an edit point is also selected).",
        args: &["on:boolean:true:show/hide the dual-frame trim view"],
        kind: ToolKind::Ui,
        run: |a, v| run(a, "timeline.trim_view", v).map(ToolOutcome::Done),
    },
    ToolDef {
        name: "timeline.dynamic_trim",
        desc: "Apply the same ripple/roll composition dynamic (JKL) trim performs on a rate-drop, without shuttling.",
        args: &["edit_point_side:string:true:left|right|both", "dt:number:true:seconds to apply, sign = direction"],
        kind: ToolKind::Mutate,
        run: |a, v| run(a, "timeline.dynamic_trim", v).map(ToolOutcome::Done),
    },
    ToolDef {
        name: "preview.compare",
        desc: "Set the monitor's grade-compare mode (state only — no bypass render exists yet, see the PR body).",
        args: &["mode:string:true:off|wipe|side", "x:number:false:wipe split 0..1, default 0.5"],
        kind: ToolKind::Ui,
        run: |a, v| run(a, "preview.compare", v).map(ToolOutcome::Done),
    },
    ToolDef {
        name: "stills.save",
        desc: "Snapshot a clip's effect stack as a named preset (a 'Look') into Settings.effect_presets.",
        args: &["clip_id:integer:false:default selection", "name:string:false:default auto-numbered"],
        kind: ToolKind::Ui,
        run: |a, v| run(a, "stills.save", v).map(ToolOutcome::Done),
    },
    ToolDef {
        name: "stills.apply",
        desc: "Paste a saved still's effect stack onto a clip (engine::presets::apply_effects, one undo).",
        args: &["clip_id:integer:true:target clip", "name:string:true:preset name from stills.save"],
        kind: ToolKind::Mutate,
        run: |a, v| run(a, "stills.apply", v).map(ToolOutcome::Done),
    },
    ToolDef {
        name: "scopes.read",
        desc: "Current-frame histogram/percentile/mean statistics (waveform/parade/vectorscope share the same summary — see the tool's own note).",
        args: &["kind:string:false:waveform|parade|vectorscope|histogram, default all"],
        kind: ToolKind::Read,
        run: |a, v| run(a, "scopes.read", v).map(ToolOutcome::Done),
    },
    ToolDef {
        name: "multicam.sync",
        desc: "Dry-run: compute cross-correlation offsets for a set of clips without creating a sequence.",
        args: &["clip_ids:array:true:asset-backed clip ids to align"],
        kind: ToolKind::Read,
        run: |a, v| run(a, "multicam.sync", v).map(ToolOutcome::Done),
    },
    ToolDef {
        name: "multicam.create",
        desc: "Sync + nest clips into a multicam sequence, one video track per angle (capped at 4 for the grid).",
        args: &["clip_ids:array:true:>=2 clips", "name:string:false:sequence name"],
        kind: ToolKind::Mutate,
        run: |a, v| run(a, "multicam.create", v).map(ToolOutcome::Done),
    },
    ToolDef {
        name: "multicam.switch",
        desc: "Switch the active angle from time t onward (split + enabled toggles, editable afterward).",
        args: &["seq_clip_id:integer:true:", "t:number:false:default playhead", "angle:integer:true:0-based"],
        kind: ToolKind::Mutate,
        run: |a, v| run(a, "multicam.switch", v).map(ToolOutcome::Done),
    },
    ToolDef {
        name: "color.pick",
        desc: "Sample the frame at (x,y); if a clip is selected, writes its ChromaKey/Qualifier target too.",
        args: &["x:number:true:0..1 in the letterbox", "y:number:true:", "target:string:true:chroma|qualifier"],
        kind: ToolKind::Mutate,
        run: |a, v| run(a, "color.pick", v).map(ToolOutcome::Done),
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_hue_degrees_matches_known_colors() {
        assert!((rgb_hue_degrees([255, 0, 0]) - 0.0).abs() < 0.5, "red");
        assert!((rgb_hue_degrees([0, 255, 0]) - 120.0).abs() < 0.5, "green");
        assert!((rgb_hue_degrees([0, 0, 255]) - 240.0).abs() < 0.5, "blue");
        assert_eq!(rgb_hue_degrees([128, 128, 128]), 0.0, "grey has no hue");
    }

    #[test]
    fn find_or_add_reuses_an_existing_effect() {
        let mut c = Clip::new(1, ClipKind::Video, "c", 0.0, 2.0);
        find_or_add(&mut c, EffectKind::ChromaKey).params[0].value = 10.0;
        assert_eq!(c.effects.len(), 1);
        find_or_add(&mut c, EffectKind::ChromaKey).params[1].value = 20.0;
        assert_eq!(c.effects.len(), 1, "a second call on the same kind edits in place");
        assert_eq!(c.effects[0].params[0].value, 10.0);
    }

    #[test]
    fn write_picked_color_writes_chroma_rgb_and_qualifier_hue() {
        let mut p = Project::new();
        let id = p.new_id();
        p.tracks[0].clips.push(Clip::new(id, ClipKind::Video, "v", 0.0, 4.0));
        let c = p.clip_mut(id).unwrap();
        find_or_add(c, EffectKind::ChromaKey).params[0].value = 0.0; // create it up front to exercise find (not add)
        write_picked_color_on_clip(c, PickTarget::Chroma, [10, 20, 30]);
        let e = c.effects.iter().find(|e| e.kind == EffectKind::ChromaKey).unwrap();
        assert_eq!((e.params[0].value, e.params[1].value, e.params[2].value), (10.0, 20.0, 30.0));
        write_picked_color_on_clip(c, PickTarget::Qualifier, [0, 255, 0]);
        let e = c.effects.iter().find(|e| e.kind == EffectKind::Qualifier).unwrap();
        assert!((e.params[0].value - 120.0).abs() < 0.5, "qualifier centre = green's hue");
    }

    #[test]
    fn current_angle_finds_the_enabled_track_at_local_time() {
        let mut p = Project::new();
        let ids: Vec<Id> = (0..2)
            .map(|i| {
                let aid = p.add_asset(crate::model::Asset {
                    id: 300 + i,
                    path: format!("C:/a{i}.mp4"),
                    kind: ClipKind::Video,
                    duration: 10.0,
                    width: 1920,
                    height: 1080,
                    fps: 30.0,
                    audio_streams: Vec::new(),
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
                let cid = p.new_id();
                let mut c = Clip::new(cid, ClipKind::Video, "cam", 0.0, 10.0);
                c.asset = aid;
                let ti = if i == 0 { 0 } else { p.add_track(TrackKind::Video) };
                p.tracks[ti].clips.push(c);
                cid
            })
            .collect();
        let seq_id = p.multicam_make(&ids, &[0.0, 0.0], "MC").unwrap();
        let clip_id = p.insert_sequence_clip(seq_id, 0.0, None).unwrap();
        // before any switch every angle's clip is still enabled (default construction) — the scan finds
        // the first (lowest-ranked) track, angle 0.
        assert_eq!(current_angle(&p, clip_id, 1.0), Some(0));
        assert!(p.multicam_switch(clip_id, 5.0, 1));
        assert_eq!(current_angle(&p, clip_id, 6.0), Some(1), "after the switch point: angle 1");
        assert_eq!(current_angle(&p, clip_id, 1.0), Some(0), "before the switch point: still angle 0");
    }
}
