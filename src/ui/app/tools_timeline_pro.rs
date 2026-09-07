//! ---- ws:pro-timeline ----
//! Find (Ctrl+F), the overview-strip toggle, and the header inline-rename trigger (ACT_HANDLER: `act`);
//! the Find window (WINDOW_DRAWER: `window` - the real `App`-touching half of `find_ui`, see that
//! module's doc comment for why it isn't `find_ui::window` directly); and `timeline.find`/
//! `timeline.view_preset`/`timeline.dupes`/`timeline.pacing`/`timeline.overview` (TOOL_TABLES: `TOOLS`).
//! Track rename/colour/reorder are NOT tools here - the header UI (`ui::timeline::header`) dispatches
//! straight to trim-model's `track.set`/`track.move` via the local `Act` enum inside `timeline::show()`
//! (no `App` reachable there); see the audit note in the plan's Review trail and this PR's body.

use super::tools_helpers::*;
use super::*;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::ui::find_ui;
use crate::ui::timeline::{dupe_groups, pacing_spans};

pub(super) fn act(app: &mut App, a: Action) -> bool {
    match a {
        Action::Find => {
            app.find.open = true;
            true
        }
        Action::ToggleOverview => {
            app.settings.overview = !app.settings.overview;
            true
        }
        Action::RenameTrack => {
            // mirrors trim_actions.rs's toggle_track_flag: no hover state exists headlessly, so the
            // first selected clip's track stands in for "the track the header double-click would hit".
            if let Some(ti) = app.selection.first().and_then(|&id| app.project.track_of(id)) {
                app.timeline.track_rename = Some((ti, app.project.tracks[ti].name.clone()));
            }
            true
        }
        _ => false,
    }
}

pub(super) fn window(app: &mut App, ctx: &egui::Context) {
    let Some(jump) = find_ui::window(ctx, &mut app.find, &app.project) else { return };
    app.seek(jump.t);
    if let Some(id) = jump.select {
        // route by hit kind: `app.selection` is the CLIP selection set, so a Marker/Cue hit must
        // not land there (it would both fail to highlight and silently clear the clip selection).
        match jump.kind {
            find_ui::HitKind::Clip => {
                app.selection = vec![id];
                app.sel_transitions.clear();
            }
            find_ui::HitKind::Marker => app.timeline.selected_marker = Some(id),
            find_ui::HitKind::Cue => app.subtitles_ui.selected = Some(id),
            find_ui::HitKind::Sequence => {}
        }
    }
    if let Some(pane) = jump.pane {
        app.layout.reveal_auto(pane);
    }
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "timeline.find",
        desc: "Search clip names, marker names/notes, subtitle cues and sequence names (case-insensitive substring).",
        args: &["query:string:true:"],
        kind: ToolKind::Read,
        run: |app, args| {
            let q = req(arg_str(args, "query"), "query")?;
            let hits: Vec<Value> = find_ui::find(&app.project, q)
                .into_iter()
                .map(|h| json!({"kind": format!("{:?}", h.kind), "id": h.id, "t": h.t, "text": h.text}))
                .collect();
            Ok(ToolOutcome::Done(json!({"hits": hits})))
        },
    },
    ToolDef {
        name: "timeline.view_preset",
        desc: "Get or set the active timeline view preset (row heights/element toggles). Omit `name` to \
               get the current preset; give it to switch (must already exist in Settings.timeline_views).",
        args: &[
            "name:string:false:switch to this preset (omit = get current)",
            "waves:boolean:false:override on the target preset",
            "thumbs:boolean:false:override on the target preset",
            "keys:boolean:false:override on the target preset",
            "clip_text:boolean:false:override on the target preset",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            if let Some(name) = arg_str(args, "name") {
                let idx = app.settings.timeline_views.iter().position(|v| v.name == name);
                let idx = idx.ok_or_else(|| format!("unknown view preset '{name}'"))?;
                app.timeline.view_idx = idx;
            }
            let idx = app.timeline.view_idx.min(app.settings.timeline_views.len().saturating_sub(1));
            if let Some(v) = app.settings.timeline_views.get_mut(idx) {
                if let Some(b) = arg_bool(args, "waves") {
                    v.waves = b;
                }
                if let Some(b) = arg_bool(args, "thumbs") {
                    v.thumbs = b;
                }
                if let Some(b) = arg_bool(args, "keys") {
                    v.keys = b;
                }
                if let Some(b) = arg_bool(args, "clip_text") {
                    v.clip_text = b;
                }
                Ok(ToolOutcome::Done(json!({
                    "name": v.name, "waves": v.waves, "thumbs": v.thumbs, "keys": v.keys, "clip_text": v.clip_text,
                })))
            } else {
                Err("no timeline view presets configured".into())
            }
        },
    },
    ToolDef {
        name: "timeline.dupes",
        desc: "Groups of clip ids sharing (asset, src_in..src_out) -- duplicate-source detection.",
        args: &[],
        kind: ToolKind::Read,
        run: |app, _args| Ok(ToolOutcome::Done(json!({"groups": dupe_groups(&app.project)}))),
    },
    ToolDef {
        name: "timeline.pacing",
        desc: "Clip ids/spans outside the boring-detector thresholds (too long / too short).",
        args: &[
            "short_s:number:false:default Settings.boring_thr.0",
            "long_s:number:false:default Settings.boring_thr.1",
        ],
        kind: ToolKind::Read,
        run: |app, args| {
            let (s0, s1) = app.settings.boring_thr;
            let thr = (arg_f64(args, "short_s").unwrap_or(s0 as f64), arg_f64(args, "long_s").unwrap_or(s1 as f64));
            let spans: Vec<Value> = pacing_spans(&app.project, thr)
                .into_iter()
                .map(|(id, a, b)| json!({"id": id, "from": a, "to": b}))
                .collect();
            Ok(ToolOutcome::Done(json!({"spans": spans})))
        },
    },
    ToolDef {
        name: "timeline.overview",
        desc: "Show/hide the inline overview minimap strip.",
        args: &["enabled:boolean:false:omit = toggle"],
        kind: ToolKind::Ui,
        run: |app, args| {
            app.settings.overview = arg_bool(args, "enabled").unwrap_or(!app.settings.overview);
            Ok(ToolOutcome::Done(json!({"ok": true, "enabled": app.settings.overview})))
        },
    },
];
