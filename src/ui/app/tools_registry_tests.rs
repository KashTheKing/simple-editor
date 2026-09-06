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
    ("fit_clip_to_screen", "viewer 'Fit' is a UI action, not an MCP tool"),
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

#[test]
fn ui_action_covers_every_action() {
    for &a in Action::ALL {
        let id = a.id();
        assert_eq!(Action::from_id(id), Some(a), "ui.action's resolver must round-trip every Action id ({id})");
    }
}

// deviation (see PR body): the plan's `mutate_rows_roll_back_on_error` / `action_enabled_toasts_reason`
// / `run_tool_undoable_snapshots_only_mutate` / `run_script_pushes_one_undo_per_script` tests all need a
// real `App` to call a `ToolDef::run` or `App::act`/`run_script` against — but `App::new` requires a
// real `eframe::CreationContext` (a live GL context from `eframe::run_native`), and this crate has no
// headless App-construction path anywhere (confirmed pre-existing: see the doc comment atop
// src/ui/app/tests.rs, and no other test in the crate calls an `&mut App` method). `ToolKind::Mutate`'s
// snapshot/rollback shape is instead verified by reading `App::handle_tool`/`run_script`'s
// implementation directly (`src/ui/app/mcp_exec.rs`: `run_snapshot_if_mutate` snapshots before every
// Mutate-kind call and `run_rollback` restores on `Err`, unchanged from the pre-refactor shape) and by
// the manual verification-checklist steps (enable the MCP server, call a tool with bad args; run a
// 3-tool Luau script and check the History panel shows one entry) — both listed in this PR's body.
