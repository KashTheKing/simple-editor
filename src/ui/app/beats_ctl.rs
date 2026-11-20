//! ---- ws:beat-aligner ----
//! App side of the Beat Aligner (`ui::beat_aligner`): the pane, tap keys and continuous marking while the
//! song plays, detection from the cached song analysis, and the `beats.*` tools.
//!
//! Tap timing: a tap is the player clock read when the frame that received the key starts (frame hooks
//! run first thing in `update`), minus `Settings.beat_latency_ms` - what Calibrate measures from taps
//! against the detected grid (keyboard, frame and audio output delay together).

use super::tools_helpers::*;
use super::*;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::model::beats::{self, BeatFmt};
use crate::model::ops::beats::CutScope;
use crate::model::{BeatSheet, MarkKind};
use crate::ui::beat_aligner as ba;

/// Tap keys and continuous marking. Runs before the hotkeys, so an armed Space taps instead of pausing.
pub(super) fn frame(app: &mut App, ctx: &egui::Context) {
    if !app.pane_drawn(Pane::BeatAligner) || app.project.beats.asset == 0 {
        app.beats.filled_to = None;
        return;
    }
    let playing = app.player.is_playing();
    if app.beats.armed && playing && !ctx.wants_keyboard_input() {
        let taps = ctx.input_mut(|i| {
            [egui::Key::Space, egui::Key::B, egui::Key::M]
                .iter()
                .filter(|k| i.consume_key(egui::Modifiers::NONE, **k))
                .count()
        });
        for _ in 0..taps.min(1) {
            tap(app);
        }
    }
    // continuous: the grid beats played since last frame (one undo step per play)
    if app.beats.continuous && playing && app.project.beats.bpm > 0.0 && !app.project.beats.voice {
        let now = app.project.beat_map().to_src(app.player.time());
        if let Some(from) = app.beats.filled_to.filter(|f| now > *f && now - f < 2.0) {
            let mut s = app.project.beats.clone();
            if s.fill(from, now) > 0 {
                if !app.beats_run_undo {
                    app.push_undo();
                    app.beats_run_undo = true;
                }
                app.project.beats = s;
                app.dirty = true;
            }
        }
        app.beats.filled_to = Some(now);
    } else {
        app.beats.filled_to = None;
        app.beats_run_undo = false;
    }
}

/// A mark at the player position (or the calibration tap while calibrating).
fn tap(app: &mut App) {
    let map = app.project.beat_map();
    let t = if app.player.is_playing() { app.player.time() } else { app.playhead };
    let heard = map.to_src(t);
    if let Some(cal) = &mut app.beats.cal {
        cal.push(heard);
        if cal.len() >= 8 {
            let cal = app.beats.cal.take().unwrap_or_default();
            match beats::calibrate(&app.project.beats, &cal) {
                Some(lat) => {
                    // song seconds -> real seconds
                    app.settings.beat_latency_ms = (lat / map.speed * 1000.0).round();
                    app.toast(format!("Tap latency set to {} ms", app.settings.beat_latency_ms));
                }
                None => app.toast("Calibration needs a tempo: press Detect first"),
            }
        }
        return;
    }
    app.push_undo();
    let st = &app.beats;
    let snap = (st.snap && app.project.beats.bpm > 0.0).then_some((st.strength as f64, st.div));
    let lat = app.settings.beat_latency_ms / 1000.0 * map.speed;
    let id = app.project.beats.tap(heard, lat, snap);
    app.beats.sel = vec![id];
    app.dirty = true; // the sheet is not the picture: no player / cache refresh mid-playback
}

/// The song's file (asset path) and length.
fn song(app: &App) -> Option<(String, f64)> {
    app.project.asset(app.project.beats.asset).map(|a| (a.path.clone(), a.duration))
}

/// Detect from the cached analysis (music) or the waveform (voice). False while those are still being
/// computed in the background (the pane retries when they land).
fn detect(app: &mut App, ctx: &egui::Context) -> Result<bool, String> {
    let (path, dur) = song(app).ok_or("pick a song first")?;
    if app.project.beats.voice {
        let Some(peaks) = app.waveforms.get(&path, 0) else { return Ok(false) };
        let found = beats::phrases(&peaks, 0.0, dur, 0.35);
        app.push_undo();
        let s = &mut app.project.beats;
        s.marks.retain(|m| !matches!(m.kind, MarkKind::Phrase | MarkKind::Pause));
        for (t, k) in found {
            s.add_mark(t, k);
        }
        app.dirty = true;
        return Ok(true);
    }
    let path = super::stems_ctl::beat_source(&path); // ws:addon-demucs: the drums stem when separated
    let c = ctx.clone();
    let Some(a) = crate::engine::visualizer::analysis_async(&path, 0, app.backend(), move || c.request_repaint())
    else {
        return Ok(false);
    };
    let mut s = app.project.beats.clone();
    apply_analysis(&mut s, &a, dur)?;
    app.push_undo();
    app.project.beats = s;
    app.dirty = true;
    Ok(true)
}

/// Tempo, grid and downbeat from a song analysis onto the sheet.
pub(super) fn apply_analysis(s: &mut BeatSheet, a: &crate::engine::visualizer::SongAnalysis, dur: f64) -> Result<(), String> {
    let (bpm, off) = beats::fit_grid(&a.beats).ok_or("no steady beat found - set the BPM or tap the tempo")?;
    s.bpm = (bpm * 100.0).round() / 100.0;
    s.offset = off;
    let hz = crate::engine::visualizer::ANALYSIS_HZ;
    let level = |t: f64| a.raw.get((t * hz).round() as usize).map_or(0.0, |f| f[0]);
    s.pick_downbeat(level, dur);
    Ok(())
}

pub(super) fn draw(app: &mut App, ui: &mut egui::Ui, pane: Pane) -> bool {
    if pane != Pane::BeatAligner {
        return false;
    }
    // follow the timeline: a newly selected audio clip of another song becomes the song (no undo: a pick)
    if app.project.beats.asset == 0 {
        if let Some(c) = app
            .selection
            .iter()
            .filter_map(|id| app.project.clip(*id))
            .find(|c| c.kind == ClipKind::Audio && c.asset != 0)
        {
            let id = c.id;
            app.project.set_beat_source(Some(id), None, false);
            app.beats.detect_pending = true;
        }
    }
    let ctx = ui.ctx().clone();
    if app.beats.detect_pending {
        match detect(app, &ctx) {
            Ok(true) => app.beats.detect_pending = false,
            Ok(false) => {}
            Err(e) => {
                app.beats.detect_pending = false;
                app.toast(e);
            }
        }
    }
    let (path, dur) = song(app).unwrap_or_default();
    let peaks = (!path.is_empty()).then(|| app.waveforms.get(&path, 0)).flatten();
    let p = &app.project;
    let mut songs: Vec<(Id, Id, String)> = p
        .all_clips()
        .filter(|(_, c)| c.kind == ClipKind::Audio && c.asset != 0)
        .map(|(_, c)| (c.id, c.asset, format!("{} (timeline {})", c.name, clock(c.start))))
        .collect();
    let sounds: Vec<(Id, String)> =
        p.assets.iter().filter(|a| a.kind == ClipKind::Audio).map(|a| (a.id, a.name())).collect();
    songs.extend(sounds.iter().map(|(id, n)| (0, *id, n.clone())));
    let mut blocks: Vec<String> = crate::model::builtin_blocks().into_iter().map(|b| b.name).collect();
    blocks.extend(app.settings.key_blocks.iter().map(|b| b.name.clone()));
    let map = p.beat_map();
    let on_timeline = p.all_clips().any(|(_, c)| c.asset == p.beats.asset && p.beats.asset != 0);
    let playing = app.player.is_playing();
    let view = ba::View {
        peaks: peaks.as_deref(),
        dur,
        songs: &songs,
        sounds: &sounds,
        blocks: &blocks,
        now: on_timeline.then(|| map.to_src(app.playhead)),
        playing,
        latency_ms: app.settings.beat_latency_ms,
    };
    let resp = {
        let App { project, undo, redo, beats: st, palette, .. } = app;
        let mut push = |p: &Project| push_undo_json(undo, redo, p.to_json());
        ba::show(ui, st, project, &view, palette, &mut push)
    };
    if resp.edited {
        app.dirty = true;
    }
    if resp.detect {
        app.beats.detect_pending = true;
        ctx.request_repaint();
    }
    if let Some(t) = resp.seek {
        app.seek(t);
    }
    if resp.play {
        if playing {
            app.player.pause();
        } else {
            app.play_gated(1.0);
        }
    }
    if resp.tap {
        tap(app);
    }
    if let Some(ms) = resp.latency_ms {
        app.settings.beat_latency_ms = ms;
    }
    if resp.calibrate {
        app.beats.cal = match app.beats.cal {
            Some(_) => None,
            None if app.project.beats.bpm > 0.0 => {
                app.toast("Play the song and tap Space on the beat 8 times");
                app.beats.armed = true;
                Some(Vec::new())
            }
            None => {
                app.toast("Calibration needs a tempo: press Detect first");
                None
            }
        };
    }
    if resp.add_to_timeline {
        app.push_undo();
        let a = app.project.beats.asset;
        let ids = app.project.insert_asset_clips(a, app.playhead, None);
        if let Some(&id) = ids.iter().find(|id| app.project.clip(**id).is_some_and(|c| c.kind == ClipKind::Audio)) {
            app.project.beats.clip = id;
        }
        app.after_edit();
    }
    let every = app.beats.every;
    let edit = |app: &mut App, what: &str, f: &mut dyn FnMut(&mut App) -> usize| {
        app.push_undo();
        let n = f(app);
        if n == 0 {
            app.undo.pop();
            app.toast(format!("No {what}"));
        } else {
            app.after_edit();
            app.toast(format!("{n} {what}"));
        }
    };
    if resp.to_markers {
        edit(app, "timeline markers added", &mut |app| app.project.beats_to_markers(true, true));
    }
    if resp.cut {
        let scope = if app.beats.cut_all_video {
            CutScope::AllVideo
        } else {
            let ti = app.selection.first().and_then(|id| app.project.track_of(*id));
            match ti {
                Some(ti) => CutScope::Track(ti),
                None => {
                    app.toast("Select a clip on the track to cut");
                    return true;
                }
            }
        };
        edit(app, "cuts made", &mut |app| app.project.beats_cut(&scope, every));
    }
    if resp.sfx {
        let (sfx, lead) = (app.beats.sfx, app.beats.sfx_lead);
        edit(app, "sounds placed", &mut |app| app.project.beats_sfx(sfx, every, lead).len());
    }
    if resp.snap_clips {
        let ids = app.selection.clone();
        edit(app, "clip edges snapped", &mut |app| app.project.beats_snap_clips(&ids, 0.25));
    }
    if resp.effect {
        let name = app.beats.block.clone();
        match crate::model::find_block(&app.settings.key_blocks, &name) {
            Some(b) => {
                let ids: Vec<Id> = app
                    .selection
                    .iter()
                    .copied()
                    .filter(|id| app.project.clip(*id).is_some_and(|c| c.is_visual()))
                    .collect();
                edit(app, "effects placed", &mut |app| app.project.beats_effect(&b, &ids, every))
            }
            None => app.toast(format!("No block called {name}")),
        }
    }
    if let Some(fmt) = resp.export {
        let name = format!("{}_beats.{}", app.project.name, fmt.ext());
        if let Some(out) = rfd::FileDialog::new().add_filter("Markers", &[fmt.ext()]).set_file_name(name).save_file() {
            let text = beats::export(&app.project.beats, app.project.beat_map(), app.project.fps, fmt);
            match std::fs::write(&out, text) {
                Ok(()) => app.toast(format!("Saved {}", out.display())),
                Err(e) => app.toast(e.to_string()),
            }
        }
    }
    if resp.import {
        if let Some(path) = rfd::FileDialog::new().add_filter("Markers", &["csv", "edl", "txt"]).pick_file() {
            match std::fs::read_to_string(&path) {
                Ok(text) => {
                    app.push_undo();
                    let map = app.project.beat_map();
                    let n = beats::import(&mut app.project.beats, map, app.project.fps, &text);
                    app.dirty = true;
                    app.toast(format!("{n} marks and sections imported"));
                }
                Err(e) => app.toast(e.to_string()),
            }
        }
    }
    if let Some(t) = resp.toast {
        app.toast(t);
    }
    true
}

fn clock(t: f64) -> String {
    format!("{}:{:02}", (t / 60.0).floor() as u64, (t % 60.0).floor() as u64)
}

// ---------- tools ----------

fn sheet_json(p: &Project) -> Value {
    let map = p.beat_map();
    let s = &p.beats;
    json!({
        "asset_id": s.asset, "clip_id": s.clip, "voice": s.voice,
        "bpm": s.bpm, "offset": map.to_tl(s.offset), "meter": s.meter,
        "marks": s.marks.iter().map(|m| json!({"id": m.id, "t": map.to_tl(m.t), "kind": m.kind.name()})).collect::<Vec<_>>(),
        "sections": s.sections.iter().map(|x| json!({"id": x.id, "a": map.to_tl(x.a), "b": map.to_tl(x.b), "name": x.name, "label": x.label})).collect::<Vec<_>>(),
    })
}

fn ok(app: &App) -> Result<ToolOutcome, String> {
    Ok(ToolOutcome::Done(sheet_json(&app.project)))
}

fn need_song(app: &App) -> Result<(), String> {
    if app.project.beats.asset == 0 {
        return Err("no song: call beats.source first".into());
    }
    Ok(())
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "beats.source",
        desc: "Point the Beat Aligner at a song or voiceover: a timeline audio clip or a library asset. A different song starts a fresh sheet.",
        args: &["clip_id:integer:false:", "asset_id:integer:false:", "voice:boolean:false:phrases and pauses instead of beats"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let voice = arg_bool(args, "voice").unwrap_or(false);
            if !app.project.set_beat_source(arg_u64(args, "clip_id"), arg_u64(args, "asset_id"), voice) {
                return Err("give a clip_id or an asset_id that exists".into());
            }
            ok(app)
        },
    },
    ToolDef {
        name: "beats.detect",
        desc: "Detect the tempo, beat grid and downbeat of the song (voice: phrase and pause marks). Runs the song analysis on this thread when it is not cached yet. Music beats come from the song's drums stem once it has been separated (audio.stems): a cleaner grid on a busy mix, as the Beat Aligner's Detect does.",
        args: &["source:string:false:auto (default: the drums stem when separated, else the song) | song | drums"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            need_song(app)?;
            let (path, dur) = song(app).ok_or("the song's asset is gone")?;
            // ---- ws:addon-demucs ----
            let path = match arg_str(args, "source").unwrap_or("auto") {
                "auto" if app.project.beats.voice => path,
                "auto" => super::stems_ctl::beat_source(&path),
                "song" => path,
                "drums" => super::stems_ctl::cached_stem(&path, crate::engine::demucs::Stem::Drums)
                    .ok_or("the song has no stems yet: audio.stems action=separate first")?
                    .to_string_lossy()
                    .into_owned(),
                _ => return Err("source: auto, song or drums".into()),
            };
            if app.project.beats.voice {
                let peaks = app.waveforms.get(&path, 0).ok_or("waveform still computing - retry in a few seconds")?;
                let s = &mut app.project.beats;
                s.marks.retain(|m| !matches!(m.kind, MarkKind::Phrase | MarkKind::Pause));
                for (t, k) in beats::phrases(&peaks, 0.0, dur, 0.35) {
                    s.add_mark(t, k);
                }
            } else {
                let a = crate::engine::visualizer::analysis(&path, 0, app.backend())?;
                apply_analysis(&mut app.project.beats, &a, dur)?;
            }
            ok(app)
        },
    },
    ToolDef {
        name: "beats.grid",
        desc: "Set the beat grid: bpm, offset (timeline seconds of a downbeat), meter (beats per bar), scale (0.5 half time, 2 double time); fill marks every grid beat.",
        args: &["bpm:number:false:", "offset:number:false:timeline seconds", "meter:integer:false:", "scale:number:false:0.5 or 2", "fill:boolean:false:"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            need_song(app)?;
            let map = app.project.beat_map();
            let dur = song(app).map_or(0.0, |s| s.1);
            let s = &mut app.project.beats;
            if let Some(b) = arg_f64(args, "bpm") {
                s.bpm = b.clamp(0.0, 400.0);
            }
            if let Some(o) = arg_f64(args, "offset") {
                s.offset = map.to_src(o).max(0.0);
            }
            if let Some(m) = arg_u64(args, "meter") {
                s.meter = (m as u32).clamp(1, 12);
            }
            if let Some(k) = arg_f64(args, "scale") {
                s.scale_tempo(k);
            }
            if arg_bool(args, "fill") == Some(true) {
                s.fill(-1.0, dur);
            }
            ok(app)
        },
    },
    ToolDef {
        name: "beats.tap",
        desc: "Drop a mark at timeline time t (default: the playhead), the tap latency taken off and optionally pulled toward the grid.",
        args: &["t:number:false:timeline seconds", "latency_ms:number:false:default Settings' calibrated latency", "snap:number:false:strength 0..1", "div:integer:false:1 beats, 2 eighths, 4 sixteenths"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            need_song(app)?;
            let map = app.project.beat_map();
            let t = arg_f64(args, "t").unwrap_or(app.playhead);
            let lat = arg_f64(args, "latency_ms").unwrap_or(app.settings.beat_latency_ms) / 1000.0 * map.speed;
            let snap = arg_f64(args, "snap").map(|s| (s, arg_u64(args, "div").unwrap_or(1) as u32));
            let id = app.project.beats.tap(map.to_src(t), lat, snap);
            Ok(ToolOutcome::Done(json!({"id": id, "t": map.to_tl(app.project.beats.marks.iter().find(|m| m.id == id).map_or(0.0, |m| m.t))})))
        },
    },
    ToolDef {
        name: "beats.get",
        desc: "The Beat Aligner's song, tempo, marks and sections (timeline seconds).",
        args: &[],
        kind: ToolKind::Read,
        run: |app, _| ok(app),
    },
    ToolDef {
        name: "beats.edit",
        desc: "Edit marks: add (timeline times, kind Tap/Beat/Downbeat/Phrase/Pause), delete ids, or move ids by dt (snap_div lands them on the grid).",
        args: &["add:array:false:timeline seconds", "kind:string:false:", "delete:array:false:mark ids", "move:array:false:mark ids", "dt:number:false:seconds", "snap_div:integer:false:", "clear:boolean:false:remove every mark"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            need_song(app)?;
            let map = app.project.beat_map();
            let s = &mut app.project.beats;
            if arg_bool(args, "clear") == Some(true) {
                s.marks.clear();
            }
            let kind = MarkKind::parse(arg_str(args, "kind").unwrap_or("Tap"));
            for t in args.get("add").and_then(|v| v.as_array()).into_iter().flatten().filter_map(|v| v.as_f64()) {
                s.add_mark(map.to_src(t), kind);
            }
            if let Some(ids) = arg_ids(args, "delete") {
                s.delete_marks(&ids);
            }
            if let Some(ids) = arg_ids(args, "move") {
                let div = arg_u64(args, "snap_div").map(|d| d as u32);
                s.move_marks(&ids, arg_f64(args, "dt").unwrap_or(0.0) * map.speed, div);
            }
            ok(app)
        },
    },
    ToolDef {
        name: "beats.section",
        desc: "Sections: op add (a, b timeline seconds, name - Intro/Verse/Chorus/Drop... get their colour), rename (id, name), recolor (id, label 1-8), delete (id).",
        args: &["op:string:true:add | rename | recolor | delete", "a:number:false:", "b:number:false:", "name:string:false:", "id:integer:false:", "label:integer:false:"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            need_song(app)?;
            let map = app.project.beat_map();
            let s = &mut app.project.beats;
            let id = arg_u64(args, "id");
            match req(arg_str(args, "op"), "op")? {
                "add" => {
                    let (a, b) = (req(arg_f64(args, "a"), "a")?, req(arg_f64(args, "b"), "b")?);
                    let id = s.add_section(map.to_src(a), map.to_src(b), arg_str(args, "name").unwrap_or("Section"));
                    return Ok(ToolOutcome::Done(json!({"id": id})));
                }
                "rename" => s.section_mut(req(id, "id")?).ok_or("no such section")?.name = req(arg_str(args, "name"), "name")?.into(),
                "recolor" => s.section_mut(req(id, "id")?).ok_or("no such section")?.label = req(arg_u64(args, "label"), "label")?.min(8) as u8,
                "delete" => {
                    if !s.delete_section(req(id, "id")?) {
                        return Err("no such section".into());
                    }
                }
                op => return Err(format!("unknown op '{op}'")),
            }
            ok(app)
        },
    },
    ToolDef {
        name: "beats.apply",
        desc: "Use the marks on the timeline: markers (timeline markers for marks and sections), cut (split at every Nth mark: scope all_video, track or clips), sfx (place audio asset_id at each mark, lead seconds early), snap_clips (clip_ids' edges to marks within tolerance), effect (keyframe block - Zoom Punch, Shake, Flash... - on each mark, on clip_ids or every video clip).",
        args: &[
            "action:string:true:markers | cut | sfx | snap_clips | effect",
            "every:integer:false:use every Nth mark, default 1",
            "scope:string:false:all_video (default) | track | clips",
            "track:integer:false:track index for scope track",
            "clip_ids:array:false:",
            "asset_id:integer:false:the sound for sfx",
            "lead:number:false:sfx seconds early, default 0.03",
            "tolerance:number:false:snap_clips reach, default 0.25 s",
            "block:string:false:effect block name, default Zoom Punch",
            "marks:boolean:false:markers: include marks, default true",
            "sections:boolean:false:markers: include sections, default true",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            need_song(app)?;
            let every = arg_u64(args, "every").unwrap_or(1).max(1) as usize;
            let ids = arg_ids(args, "clip_ids").unwrap_or_default();
            let p = &mut app.project;
            let n = match req(arg_str(args, "action"), "action")? {
                "markers" => p.beats_to_markers(arg_bool(args, "marks").unwrap_or(true), arg_bool(args, "sections").unwrap_or(true)),
                "cut" => {
                    let scope = match arg_str(args, "scope").unwrap_or("all_video") {
                        "all_video" => CutScope::AllVideo,
                        "track" => CutScope::Track(req(arg_u64(args, "track"), "track")? as usize),
                        "clips" => CutScope::Clips(ids),
                        s => return Err(format!("unknown scope '{s}'")),
                    };
                    p.beats_cut(&scope, every)
                }
                "sfx" => p.beats_sfx(req(arg_u64(args, "asset_id"), "asset_id")?, every, arg_f64(args, "lead").unwrap_or(0.03)).len(),
                "snap_clips" => p.beats_snap_clips(&ids, arg_f64(args, "tolerance").unwrap_or(0.25)),
                "effect" => {
                    let name = arg_str(args, "block").unwrap_or("Zoom Punch");
                    let b = crate::model::find_block(&app.settings.key_blocks, name).ok_or_else(|| format!("no block called {name}"))?;
                    p.beats_effect(&b, &ids, every)
                }
                a => return Err(format!("unknown action '{a}'")),
            };
            Ok(ToolOutcome::Done(json!({"ok": true, "count": n})))
        },
    },
    ToolDef {
        name: "beats.export",
        desc: "Write the marks and sections (timeline time) to a CSV, a marker EDL (DaVinci Resolve) or a YouTube-chapters file.",
        args: &["path:string:true:", "format:string:false:csv | edl | chapters, default csv"],
        kind: ToolKind::Read,
        run: |app, args| {
            let path = req(arg_str(args, "path"), "path")?;
            let fmt = BeatFmt::parse(arg_str(args, "format").unwrap_or("csv")).ok_or("format must be csv | edl | chapters")?;
            let text = beats::export(&app.project.beats, app.project.beat_map(), app.project.fps, fmt);
            std::fs::write(path, text).map_err(|e| e.to_string())?;
            Ok(ToolOutcome::Done(json!({"ok": true, "path": path})))
        },
    },
    ToolDef {
        name: "beats.import",
        desc: "Read a CSV / EDL / chapters marker file onto the sheet: ranges become sections, points marks.",
        args: &["path:string:true:"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            need_song(app)?;
            let text = std::fs::read_to_string(req(arg_str(args, "path"), "path")?).map_err(|e| e.to_string())?;
            let map = app.project.beat_map();
            let n = beats::import(&mut app.project.beats, map, app.project.fps, &text);
            Ok(ToolOutcome::Done(json!({"ok": true, "added": n})))
        },
    },
    ToolDef {
        name: "beats.calibrate",
        desc: "Set the tap latency: from taps (timeline seconds when keys were pressed along to the beat, 4 or more) against the grid, or directly as ms.",
        args: &["taps:array:false:timeline seconds", "ms:number:false:"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let map = app.project.beat_map();
            let ms = match (arg_f64(args, "ms"), args.get("taps").and_then(|v| v.as_array())) {
                (Some(ms), _) => ms,
                (None, Some(t)) => {
                    let heard: Vec<f64> = t.iter().filter_map(|v| v.as_f64()).map(|t| map.to_src(t)).collect();
                    beats::calibrate(&app.project.beats, &heard).ok_or("needs a tempo and 4+ taps")? / map.speed * 1000.0
                }
                _ => return Ok(ToolOutcome::Done(json!({"latency_ms": app.settings.beat_latency_ms}))),
            };
            app.settings.beat_latency_ms = ms.round();
            Ok(ToolOutcome::Done(json!({"latency_ms": app.settings.beat_latency_ms})))
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The pane stays still when nothing moves: no repaint requested on an idle frame.
    #[test]
    fn assert_no_idle_repaint_beat_aligner() {
        let ctx = egui::Context::default();
        let mut st = ba::AlignState::default();
        let mut p = Project::new();
        p.beats.asset = 1;
        p.beats.bpm = 120.0;
        p.beats.add_mark(1.0, MarkKind::Tap);
        p.beats.add_section(0.0, 4.0, "Intro");
        let pal = crate::theme::Palette::new(true, egui::Color32::LIGHT_BLUE);
        let peaks = crate::media::waveform::Peaks { min: vec![-0.5; 1000], max: vec![0.5; 1000] };
        let blocks = vec!["Zoom Punch".to_string()];
        for _ in 0..30 {
            let view = ba::View {
                peaks: Some(&peaks),
                dur: 10.0,
                songs: &[],
                sounds: &[],
                blocks: &blocks,
                now: Some(0.0),
                playing: false,
                latency_ms: 0.0,
            };
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1100.0, 700.0))),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ba::show(ui, &mut st, &mut p, &view, &pal, &mut |_| {});
                });
            });
        }
        assert!(!ctx.has_requested_repaint(), "an idle Beat Aligner must not repaint");
    }

    /// Dragging a mark moves it (onto the 1/16 grid with Snap on) in one undo step - the press, not the
    /// pointer after the drag threshold, picks the mark.
    #[test]
    fn dragging_a_mark_snaps_it_in_one_undo_step() {
        let ctx = egui::Context::default();
        let mut st = ba::AlignState::default();
        (st.snap, st.div) = (true, 4);
        let mut p = Project::new();
        p.beats.asset = 1;
        p.beats.bpm = 120.0;
        p.beats.add_mark(2.0, MarkKind::Tap);
        let pal = crate::theme::Palette::new(true, egui::Color32::LIGHT_BLUE);
        let blocks: Vec<String> = Vec::new();
        let mut undos = 0;
        let frame = |events: Vec<egui::Event>, st: &mut ba::AlignState, p: &mut Project, undos: &mut usize| {
            let view = ba::View {
                peaks: None,
                dur: 10.0,
                songs: &[],
                sounds: &[],
                blocks: &blocks,
                now: None,
                playing: false,
                latency_ms: 0.0,
            };
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 800.0))),
                events,
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ba::show(ui, st, p, &view, &pal, &mut |_| *undos += 1);
                });
            });
        };
        frame(vec![], &mut st, &mut p, &mut undos);
        let r = st.canvas;
        assert!(r.width() > 100.0, "the waveform was drawn");
        let x_of = |t: f64| r.left() + (t / 10.0) as f32 * r.width();
        let y = r.bottom() - 30.0;
        let press = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        let at = egui::pos2(x_of(2.0), y);
        frame(vec![egui::Event::PointerMoved(at), press(at, true)], &mut st, &mut p, &mut undos);
        for t in [2.05, 2.1, 2.13] {
            frame(vec![egui::Event::PointerMoved(egui::pos2(x_of(t), y))], &mut st, &mut p, &mut undos);
        }
        let end = egui::pos2(x_of(2.13), y);
        frame(vec![press(end, false)], &mut st, &mut p, &mut undos);
        frame(vec![], &mut st, &mut p, &mut undos);
        assert_eq!(p.beats.marks.len(), 1);
        assert!((p.beats.marks[0].t - 2.125).abs() < 1e-9, "dragged to {}", p.beats.marks[0].t);
        assert_eq!(undos, 1, "one undo step for the whole drag");
    }

    /// A section chip in the band: dragging its middle moves it, its edge resizes it, each snapped to the
    /// grid and one undo step.
    #[test]
    fn dragging_a_section_chip_moves_and_resizes_it() {
        let ctx = egui::Context::default();
        let mut st = ba::AlignState::default();
        (st.snap, st.div) = (true, 1);
        let mut p = Project::new();
        p.beats.asset = 1;
        p.beats.bpm = 120.0;
        let id = p.beats.add_section(2.0, 4.0, "Verse");
        let pal = crate::theme::Palette::new(true, egui::Color32::LIGHT_BLUE);
        let blocks: Vec<String> = Vec::new();
        let mut undos = 0;
        let frame = |events: Vec<egui::Event>, st: &mut ba::AlignState, p: &mut Project, undos: &mut usize| {
            let view = ba::View {
                peaks: None,
                dur: 10.0,
                songs: &[],
                sounds: &[],
                blocks: &blocks,
                now: None,
                playing: false,
                latency_ms: 0.0,
            };
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 800.0))),
                events,
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ba::show(ui, st, p, &view, &pal, &mut |_| *undos += 1);
                });
            });
        };
        frame(vec![], &mut st, &mut p, &mut undos);
        let r = st.canvas;
        let x_of = |t: f64| r.left() + (t / 10.0) as f32 * r.width();
        let y = r.top() + 22.0 + 9.0; // the section band, under the ruler
        let press = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        let mut drag = |from: f64, to: f64, st: &mut ba::AlignState, p: &mut Project, undos: &mut usize| {
            let at = egui::pos2(x_of(from), y);
            frame(vec![egui::Event::PointerMoved(at), press(at, true)], st, p, undos);
            for k in 1..=4 {
                let t = from + (to - from) * k as f64 / 4.0;
                frame(vec![egui::Event::PointerMoved(egui::pos2(x_of(t), y))], st, p, undos);
            }
            frame(vec![press(egui::pos2(x_of(to), y), false)], st, p, undos);
            frame(vec![], st, p, undos);
        };
        // move: grab the middle (3.0), drop 1.2 s later -> starts at 3.2, snapped to 3.0 (whole beats)
        drag(3.0, 4.2, &mut st, &mut p, &mut undos);
        let s = p.beats.sections.iter().find(|s| s.id == id).unwrap().clone();
        assert_eq!((s.a, s.b), (3.0, 5.0), "moved whole, onto the grid");
        assert_eq!(undos, 1);
        // resize: the end edge (5.0) to 6.3 -> 6.5
        drag(5.0, 6.3, &mut st, &mut p, &mut undos);
        let s = p.beats.sections.iter().find(|s| s.id == id).unwrap().clone();
        assert_eq!((s.a, s.b), (3.0, 6.5), "end edge resized, onto the grid");
        assert_eq!(undos, 2);
    }
}
