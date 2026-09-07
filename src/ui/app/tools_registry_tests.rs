//! ---- ws:registries-schema-hooks ----
//! Structural parity tests: every `Project` edit op is either scriptable (has an MCP tool) or has a
//! recorded reason it isn't, and every `Action` resolves through `ui.action`'s id round-trip.
//! Test-only (`#[cfg(test)] mod tools_registry_tests;` in `mod.rs`) — `OP_TOOLS`/`OP_INTERNAL` exist
//! purely to drive `every_edit_op_has_a_tool`.

#![cfg(test)]
use super::*;

/// (Project op fn name, the MCP tool that reaches it). Seeded from the ~79 `pub fn ...(&mut self`
/// fns in `src/model/ops/*.rs` after split-god-files.
const OP_TOOLS: &[(&str, &str)] = &[
    ("add_asset", "media.import"),
    ("add_subclip", "media.subclip"),
    ("auto_cut", "timeline.auto_cut"),
    ("add_bus", "audio.add_bus"),
    ("insert_asset_clips", "timeline.add_clip"),
    ("add_text_clip", "timeline.add_clip"),
    ("split_at", "timeline.split"),
    ("delete_clips", "timeline.delete"),
    ("move_clips", "timeline.move"),
    ("set_speed", "clip.set"),
    ("add_node", "clip.add_node"),
    ("add_label", "labels.set"),
    ("remove_label", "labels.set"),
    ("add_marker", "markers.add"),
    ("remove_marker", "markers.remove"),
    ("add_clip_marker", "markers.add"),
    ("plan_add", "plan.add"),
    ("plan_remove", "plan.remove"),
    ("new_sequence", "timeline.nest"),
    ("open_sequence", "sequence.open"),
    ("close_sequence", "sequence.open"),
    ("insert_sequence_clip", "timeline.add_clip"),
    ("nest_selection", "timeline.nest"),
    ("add_shape_clip", "shapes.add"),
    ("add_container_clip", "container.add"),
    ("replace_container_media", "container.replace"),
    ("replace_container_pair", "container.replace"),
    ("make_container", "container.make"),
    ("unmake_container", "container.unmake"),
    ("add_cue", "subtitles.set"),
    ("place_clips", "templates.apply"),
    ("add_transition", "timeline.add_transition"),
    ("add_edge_transition", "timeline.add_transition"),
    // trim/keyframe/effect/mask/graph/route ops reached through clip.*/audio.* live in
    // src/ui/app/tools_clip.rs directly (they mutate a `Clip`/`Bus` field in place rather than
    // calling a named `Project::` op), so they never appear as a `pub fn NAME(&mut self` here.
    // ---- ws:trim-model ----
    ("close_gap_at", "timeline.close_gap"),
    ("shift_time", "timeline.shift_time"),
    ("mark_from_clip", "timeline.mark"),
    ("set_track_flag", "track.set"),
    ("rename_track", "track.set"),
    ("set_track_color", "track.set"),
    ("move_track", "track.move"),
    ("ripple_trim", "timeline.ripple_trim"),
    ("roll_edit", "timeline.roll"),
    ("slip", "timeline.slip"),
    ("slide", "timeline.slide"),
    ("trim_edges", "timeline.trim_edges"),
    ("extend_edit", "timeline.extend"),
    ("overwrite_asset", "timeline.overwrite"),
    ("splice_in", "timeline.splice"),
    ("lift_range", "timeline.lift"),
    ("extract_range", "timeline.extract"),
    ("join_through", "timeline.join"),
    ("duplicate", "timeline.duplicate"),
    ("unnest", "timeline.unnest"),
    ("replace_clip", "timeline.replace"),
    ("magnetic_move", "timeline.magnetic_move"),
    // ---- ws:audio-dsp-automation ----
    ("apply_repair", "audio.repair"),
    // ---- ws:canvas-handles-monitor ----
    ("fit_clip_to_screen", "clip.fit"),
    // ---- ws:source-monitor ----
    ("subclip_from_marks", "source.subclip"),
    // ---- ws:media-library ----
    ("apply_consolidate", "media.consolidate"),
    // Documentation only (like forgiveness's rows below): `consolidate_assets_copy` is an associated
    // fn (file I/O, no `&mut self`), so the scan never sees it — it is media.consolidate's copy phase.
    ("consolidate_assets_copy", "media.consolidate"),
    // ---- ws:transcript-captions ----
    ("set_transcript", "transcript.set"),
    ("cut_word_ranges", "transcript.cut_words"),
];

/// (Project op fn name, why it has no MCP tool yet). Every entry is a real, deliberate gap — either a
/// pure accessor/internal helper, or a feature that's UI/hotkey-only today (a later workstream may add
/// the tool; this table just keeps the omission honest instead of silently missing).
const OP_INTERNAL: &[(&str, &str)] = &[
    ("new_id", "id allocator, not a user edit"),
    ("asset_mut", "accessor; edits go through media.set"),
    ("remove_asset", "no MCP tool removes a library asset yet (UI-only)"),
    ("add_folder", "library folder management is UI-only, no MCP tool yet"),
    ("remove_folder", "library folder management is UI-only, no MCP tool yet"),
    ("paste_attributes", "Action::PasteAttributes is a hotkey/UI action, not an MCP tool"),
    ("main_bus", "accessor (ensures bus 0 exists), not a user edit"),
    ("bus_mut", "accessor; edits go through audio.add_filter/audio.route"),
    ("remove_bus", "no MCP tool removes a bus yet (UI-only)"),
    ("tidy", "internal cleanup pass run after every op, not itself a user edit"),
    ("freeze_at", "Action::FreezeFrame is a hotkey/UI action, not an MCP tool"),
    ("ripple_delete_range", "Action::RippleDeleteInOut is a hotkey/UI action, not an MCP tool"),
    ("ripple_open", "internal helper used by the ripple ops above, no direct tool"),
    ("trim_to_range", "Action::TrimToInOut is a hotkey/UI action, not an MCP tool"),
    ("set_enabled", "Action::ToggleEnabled is a hotkey/UI action; clip.set's fields.enabled sets one clip directly"),
    ("toggle_link", "Action::LinkToggle is a hotkey/UI action, not an MCP tool"),
    ("flow_clips", "Action::ApplyFlow is a hotkey/UI action, not an MCP tool"),
    ("ensure_graph", "internal — called by clip.add_node on first use, not itself exposed"),
    ("unlink_graph", "no MCP tool yet (UI-only)"),
    ("snap_marker_to_nearest_clip", "UI-only marker-drag snapping, not a tool argument"),
    ("link_marker_to_closest_clip", "UI-only marker-drag snapping, not a tool argument"),
    ("marker_mut", "accessor; edits go through markers.add/markers.remove"),
    ("sort_markers", "internal bookkeeping, not a user edit"),
    ("add_path", "Draw-tool / motion-path capture is UI-only, no MCP tool"),
    ("apply_path", "Draw-tool / motion-path capture is UI-only, no MCP tool"),
    ("link_path", "Draw-tool / motion-path capture is UI-only, no MCP tool"),
    ("refresh_links", "internal per-frame live-link rebake, not a user edit"),
    ("plan_item_mut", "accessor; edits go through plan.set"),
    ("add_note", "notes.set writes note[0] directly; no tool adds further titled notes yet"),
    ("note_mut", "accessor, Notes tab is UI-only for anything beyond notes.set"),
    ("remove_note", "Notes tab is UI-only for removing a titled note"),
    ("clip_mut", "accessor; edits go through clip.set/clip.keyframe/etc."),
    ("remove_unused_assets", "'Consolidate Media' is a UI action, no MCP tool yet"),
    ("sequence_mut", "accessor; edits go through sequence.open"),
    ("add_adjustment_clip", "Action::AddAdjustment is a hotkey/UI action, not an MCP tool"),
    ("add_container_from_asset", "library drag-drop container creation is UI-only"),
    ("remove_cue", "Subtitles pane is UI-only for removing one cue; subtitles.set replaces the list"),
    ("split_cue", "Subtitles pane is UI-only for splitting one cue"),
    ("cues_to_text_clips", "'Burn in as text clips' is a UI action, no MCP tool yet"),
    ("sort_cues", "internal bookkeeping, not a user edit"),
    ("add_track", "Action::AddVideoTrack/AddAudioTrack is a hotkey/UI action, not an MCP tool"),
    ("remove_track", "no MCP tool yet (UI-only track-header menu)"),
    ("find_free_track", "internal placement helper, not itself a user edit"),
    ("remove_transition", "no MCP tool yet (UI-only, e.g. right-click remove transition)"),
    ("transition_mut", "accessor; edits go through timeline.add_transition"),
    // ---- ws:trim-model ----
    (
        "insert_asset_clips_ranged",
        "superseded by splice_in/overwrite_asset, which call it directly — no separate MCP tool",
    ),
    // ---- ws:forgiveness ----
    // Documentation only: `scan_mut_self_fns` only scans OP_FILES (src/model/ops/*.rs); none of these
    // three live there (they live in src/ui/app/*), so the scan can never find or exercise these
    // entries — they exist purely so a reader of this table isn't left wondering why history.restore/
    // caches.clear/project.recover (all real MCP tools, see tools_project.rs) have no OP_TOOLS row.
    ("caches::clear", "lives in src/ui/app/caches.rs, not src/model/ops/*.rs — exempt from this scan"),
    ("recovery::recover_candidate", "lives in src/ui/app/recovery.rs, not src/model/ops/*.rs — exempt from this scan"),
    ("history_ui::restore_at", "lives in src/ui/history_ui.rs, not src/model/ops/*.rs — exempt from this scan"),
];

const OP_FILES: &[&str] = &[
    include_str!("../../model/ops/assets.rs"),
    include_str!("../../model/ops/attrs.rs"),
    include_str!("../../model/ops/autocut.rs"),
    include_str!("../../model/ops/buses.rs"),
    include_str!("../../model/ops/editing.rs"),
    include_str!("../../model/ops/graph.rs"),
    include_str!("../../model/ops/markers.rs"),
    include_str!("../../model/ops/paths.rs"),
    include_str!("../../model/ops/planner.rs"),
    include_str!("../../model/ops/queries.rs"),
    include_str!("../../model/ops/sequences.rs"),
    include_str!("../../model/ops/shapes.rs"),
    include_str!("../../model/ops/subtitles.rs"),
    include_str!("../../model/ops/templates.rs"),
    include_str!("../../model/ops/tracks.rs"),
    include_str!("../../model/ops/transitions.rs"),
    include_str!("../../model/ops/trim.rs"),
];

/// Every `pub fn NAME(...)` in `src` whose PARAMETER LIST (the balanced-paren span right after the
/// name, not an arbitrary trailing window) contains `&mut self`. Scoping to the parameter list (rather
/// than "the next N chars", which the plan's own text suggested) avoids a false positive from the next
/// function down starting within that window — e.g. `asset(&self, ...)` immediately followed by
/// `asset_mut(&mut self, ...)` a few dozen characters later.
fn scan_mut_self_fns(src: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut search_from = 0usize;
    while let Some(rel) = src[search_from..].find("pub fn ") {
        let name_start = search_from + rel + "pub fn ".len();
        let rest = &src[name_start..];
        let name_len = rest.find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(rest.len());
        let name = &rest[..name_len];
        let after_name = &rest[name_len..];
        if after_name.as_bytes().first() == Some(&b'(') {
            let mut depth = 0i32;
            let mut end = None;
            for (i, b) in after_name.bytes().enumerate() {
                match b {
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = Some(i);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if let Some(end) = end {
                if after_name[..=end].contains("&mut self") {
                    names.push(name.to_string());
                }
            }
        }
        search_from = name_start + name_len.max(1);
    }
    names
}

#[test]
fn every_edit_op_has_a_tool() {
    for src in OP_FILES {
        for name in scan_mut_self_fns(src) {
            let tool = OP_TOOLS.iter().find(|(n, _)| *n == name).map(|(_, t)| *t);
            let internal = OP_INTERNAL.iter().any(|(n, _)| *n == name);
            assert!(
                tool.is_some() || internal,
                "Project::{name} has neither an OP_TOOLS entry nor an OP_INTERNAL reason — add one"
            );
            if let Some(tool) = tool {
                assert!(
                    crate::mcp::tools::find(tool).is_some(),
                    "OP_TOOLS maps {name} -> '{tool}', but no such tool is registered"
                );
            }
        }
    }
}

// ---- ws:color-engine ----
/// clip.add_lut/color.auto/color.match/looks.list/looks.apply are declared canonical/sole-registration
/// crate-wide (see plans/ui-overhaul/issues/color-engine.md) — a later-wave workstream (inspector-
/// gallery, source-monitor) re-declaring one under a different-args duplicate must fail here (and
/// tool_names_unique_and_namespaced would also catch an exact-name collision, but a same-purpose tool
/// under a *different* name would slip past that test, not this one's fixed-name list).
#[test]
fn tool_names_are_sole_registration() {
    for name in ["clip.add_lut", "color.auto", "color.match", "looks.list", "looks.apply"] {
        let count = mcp::tools::all().filter(|t| t.name == name).count();
        assert_eq!(count, 1, "'{name}' must be registered exactly once in TOOL_TABLES, found {count}");
    }
}

// ---- ws:timeline-trim-gestures ----
/// Every gesture/act this workstream binds to the mouse is a 1:1 use of an already-tooled trim-model
/// primitive — no new MCP surface. (label, tool name(s) that must resolve in `mcp::tools::all()`);
/// Segment composes two ops (extract at the source, splice at the destination), so it lists both.
const GESTURE_TOOL_TWINS: &[(&str, &[&str])] = &[
    ("Roll", &["timeline.roll"]),
    ("Slip", &["timeline.slip"]),
    ("Slide", &["timeline.slide"]),
    ("RippleTrim", &["timeline.ripple_trim"]),
    ("MultiRippleTrim", &["timeline.trim_edges"]),
    ("Segment", &["timeline.extract", "timeline.splice"]),
    ("MagneticMove", &["timeline.magnetic_move"]),
    ("CloseGap", &["timeline.close_gap"]),
    ("JoinThroughEdit", &["timeline.join"]),
    ("Duplicate", &["timeline.duplicate"]),
    ("Unnest", &["timeline.unnest"]),
    ("ReplaceClip", &["timeline.replace"]),
    ("DropSplice", &["timeline.splice"]),
    ("DropOverwrite", &["timeline.overwrite"]),
];

#[test]
fn gestures_have_tool_twins() {
    for (label, tools) in GESTURE_TOOL_TWINS {
        for tool in *tools {
            assert!(
                mcp::tools::all().any(|t| t.name == *tool),
                "{label}'s tool twin '{tool}' is not registered in mcp::tools::all()"
            );
        }
    }
}

#[test]
fn ui_action_covers_every_action() {
    for &a in Action::ALL {
        let id = a.id();
        assert_eq!(Action::from_id(id), Some(a), "ui.action's resolver must round-trip every Action id ({id})");
    }
}

// deviation (see PR body): `App::new` requires a real `eframe::CreationContext` (a live GL context from
// `eframe::run_native`), and this crate has no headless App-construction path anywhere (confirmed
// pre-existing: see the doc comment atop src/ui/app/tests.rs, and no other test in the crate calls an
// `&mut App` method) — `eframe::CreationContext`'s fields are private with no public constructor, so
// there is no way to build one in a `#[test]` without eframe itself running a window. The four tests
// below are the narrower, non-App-dependent versions: `run_snapshot_if_mutate`'s and `App::enabled`'s
// decision logic were each split into a plain function (`snapshot_if_mutate` in mcp_exec.rs,
// `App::enabled_for` in mod.rs) that takes bare values instead of `&self`, so the actual match arms run
// under test instead of only being read. `run_script`'s one-undo-per-script shape is checked by scanning
// its own source, the same technique `scan_mut_self_fns` above already uses in this file.

/// `run_snapshot_if_mutate` (via the extracted `snapshot_if_mutate`) must snapshot before a Mutate-kind
/// call and nothing else — a Read/Job/Ui tool must never pay for a `to_json()` it can't roll back to
/// anything (nothing pushes undo for it either).
#[test]
fn run_tool_undoable_snapshots_only_mutate() {
    use super::mcp_exec::snapshot_if_mutate;
    use crate::mcp::tools::ToolKind;
    let p = Project::from_media(crate::model::Asset { duration: 4.0, ..default_asset() });
    assert_eq!(snapshot_if_mutate(&p, ToolKind::Mutate), Some(p.to_json()));
    assert_eq!(snapshot_if_mutate(&p, ToolKind::Read), None);
    assert_eq!(snapshot_if_mutate(&p, ToolKind::Job), None);
    assert_eq!(snapshot_if_mutate(&p, ToolKind::Ui), None);
}

fn default_asset() -> crate::model::Asset {
    crate::model::Asset {
        id: 0,
        path: "C:/x.mp4".into(),
        kind: ClipKind::Video,
        duration: 1.0,
        width: 0,
        height: 0,
        fps: 0.0,
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
    }
}

/// A Mutate tool that mutates the project then returns `Err` must be a no-op: `run_rollback` restores
/// the exact pre-call JSON (`handle_tool`/`run_script` both call it this way — see mcp_exec.rs). Full
/// `App::run_rollback` needs a live `App` (see the App-construction deviation noted above), so this picks
/// real `ToolKind::Mutate` rows out of the registry via `mcp::tools::all()` and exercises the actual
/// `snapshot_if_mutate` + `rollback_project` pair `run_rollback` is built from (rather than
/// re-implementing `Project::from_json` inline) — a bug in either (e.g. dropping the restore) fails this.
#[test]
fn mutate_rows_roll_back_on_error() {
    use super::mcp_exec::{rollback_project, snapshot_if_mutate};
    use crate::mcp::tools::ToolKind;
    let mutate_names: Vec<&str> =
        mcp::tools::all().filter(|d| d.kind == ToolKind::Mutate).map(|d| d.name).take(2).collect();
    assert_eq!(mutate_names.len(), 2, "expected at least 2 ToolKind::Mutate rows in the registry");
    for name in mutate_names {
        let mut p = Project::from_media(default_asset());
        let snap = snapshot_if_mutate(&p, ToolKind::Mutate).expect("Mutate kind always snapshots");
        // simulate `name`'s tool partly mutating the project before discovering its own error
        p.tracks[0].clips[0].name = format!("corrupted mid-call by {name}");
        p.add_marker(1.0, "stray");
        assert_ne!(p.to_json(), snap, "the simulated failing call must have actually mutated something");
        // this is exactly App::run_rollback's real body, via the same pure fn it calls
        p = rollback_project(&snap).unwrap();
        assert_eq!(p.to_json(), snap, "rollback must restore the pre-call state exactly for '{name}'");
    }
}

/// The bug this whole review found: `App::enabled`'s guard match must cover every one of its documented
/// arms — this is what makes `act()`'s new prelude (and `ui.action`) actually toast a reason instead of
/// silently no-op'ing. Exercised via `enabled_for` since `enabled` itself needs a live `App`.
#[test]
fn action_enabled_toasts_reason() {
    use crate::hotkeys::Action;
    // an export running blocks Save/SaveProjectAs/ExportVideo/ExportLossless...
    for a in [Action::Save, Action::SaveProjectAs, Action::ExportVideo, Action::ExportLossless] {
        assert_eq!(App::enabled_for(a, true, false, true), Err("An export is running — try again when it finishes"));
    }
    // ...but does not block an unrelated action
    assert_eq!(App::enabled_for(Action::Undo, true, false, true), Ok(()));
    // an empty timeline blocks export specifically (checked ahead of the export-running arm's absence)
    for a in [Action::ExportVideo, Action::ExportLossless] {
        assert_eq!(App::enabled_for(a, false, true, true), Err("Nothing to export — the timeline is empty"));
    }
    // nothing copied blocks PasteAttributes
    assert_eq!(
        App::enabled_for(Action::PasteAttributes, false, false, true),
        Err("Copy attributes from a clip first (Ctrl+Alt+C)")
    );
    assert_eq!(App::enabled_for(Action::PasteAttributes, false, false, false), Ok(()));
    // and act()'s prelude must actually call this — not just have it exist unused
    let src = include_str!("actions.rs");
    let dispatch_start = src.find("for f in ACT_HANDLERS").expect("act()'s ACT_HANDLERS loop");
    let prelude = &src[..dispatch_start];
    assert!(
        prelude.contains("self.enabled(a)"),
        "act()'s prelude must call self.enabled(a) before dispatch, mirroring ui.action, \
         or hotkey/menu-triggered actions never get the centralized guard/toast"
    );
}

/// `run_script` must push exactly ONE undo entry for the whole script, not one per tool call inside it
/// — verified by scanning its own source: the per-call closure must never push undo itself, and the
/// function must push undo exactly once, after the whole script has run.
#[test]
fn run_script_pushes_one_undo_per_script() {
    let src = include_str!("mcp_exec.rs");
    let fn_start = src.find("pub(super) fn run_script").expect("run_script must exist");
    let after_fn = &src[fn_start..];
    // bound the scan to just this function's body: up to the next sibling fn in the impl block
    let next_fn_at = after_fn[1..].find("pub(super) fn ").map(|i| i + 1).unwrap_or(after_fn.len());
    let body = &after_fn[..next_fn_at];
    let closure_start = body.find("let mut call = |tool").expect("run_script's per-call closure");
    let scripting_run_at = body.find("crate::scripting::run(").expect("run_script must call the interpreter");
    let closure_body = &body[closure_start..scripting_run_at];
    assert!(
        !closure_body.contains("push_undo_json"),
        "each tool call inside a script must not push its own undo entry"
    );
    let after_run = &body[scripting_run_at..];
    assert_eq!(
        after_run.matches("push_undo_json").count(),
        1,
        "run_script must push exactly one undo entry for the whole script, after it finishes"
    );
}
