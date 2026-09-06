# trim-model: ripple/roll/slip/slide primitives, track flags, edit-point keyboard trimming

**Workstream:** `trim-model` · **Issue:** [#23](https://github.com/KashTheKing/simple-editor/issues/23) · **Wave:** 1 · **Branch/worktree:** `feat/trim-model` → `../simple-editor-wt/trim-model` · **Depends on:** size-diet · **~1460 new lines · Δ exe ≈ +118 KB**

Read [agents.md](../../../agents.md), [goals.md](../../../goals.md), [notes.md](../../../notes.md) and the master plan [README.md](../README.md) (registry protocol, keymap, modifier table) before starting.

## Summary

Adds Simple Editor's Avid/Premiere/Resolve trim engine as headless model primitives, with zero new UI: per-track locked/ripple/magnetic flags (consumed from wave-0b Track fields), a sequence-scoped Project::shift_time so ripple ops stop desyncing markers/cues/in-out without corrupting nested-sequence editing, and an O(n) per-track ripple core built by EXTENDING the existing close_gap/ripple_delete_range/ripple_open (not duplicating them) — which also fixes the legacy RippleDeleteInOut/PasteInsert actions' all-track scoping for free. Full primitive set: ripple_trim/roll/slip/slide/trim_edges/extend, splice/overwrite/lift/extract, join_through/duplicate/unnest/replace_clip/magnetic_move, plus edit-point keyboard trimming, select-forward/backward/under-playhead/keyframe-nav, and mark_from_clip. Every op gets a ToolDef (MCP/Luau), including new timeline.mark/timeline.in_out tools, and these tool names/Track.color usage are audit-confirmed canonical against source-monitor's and pro-timeline's later, conflicting proposals. No timeline UI, no gestures, no new panes/glyphs/settings fields — those are sibling wave-1/2/3 workstreams' job.

## Motivation

Closes goals.md's UX-principle gaps for Avid/Premiere/Resolve-grade trimming without a second Tool button: gap_matrix rows 24-29,33-40,41,45,48,49 (ripple/roll/slip/slide, track lock/ripple/magnetic, splice/overwrite/lift/extract, gap close, in/out handles' model half, track rename/color/move, un-nest, smart edits) plus critique hooks (shift_time desync fix, O(n^2) splice fix, sequence-scoping correctness, MCP parity for mark/in-out) and missing verbs (replace, join, duplicate, select forward/backward/under-playhead, prev/next keyframe). Delivers the skeleton's 'one primitive, many verbs' thesis: every gesture/hotkey/MCP tool in later waves is a thin binding over these ~22 model fns, with cross-plan audit fixes ensuring trim-model's tool/field names stay the single canonical registration other waves must reuse.

## In scope

- Track flag accessors/mutators (locked_of, ripple_tracks, set_track_flag, rename_track, set_track_color, move_track)
- shift_time (sequence-scoped) + O(n) per-track ripple core via the extended close_gap/ripple_delete_range/ripple_open (close_gap_at)
- Full trim primitive set: ripple_trim, roll_edit, slip, slide, trim_edges, extend_edit
- Insert/overwrite/lift/extract verbs incl. ranged three-point insert
- join_through, duplicate, unnest, replace_clip, magnetic_move, select/clips-at queries, mark_from_clip
- EditPoint/Side value types
- Keyboard Actions + trim_actions.rs dispatch for all of the above
- ToolDef rows (MCP) for every user-facing op incl. timeline.mark/timeline.in_out, held canonical against source-monitor's later wave-2 tool table (audit fix 3/4)
- Model-layer unit tests

## Out of scope

- Any timeline.rs / TimelineState / mouse gesture code (snap-engine, wave 1; timeline-trim-gestures, wave 2)
- Track header UI (lock/ripple/magnetic glyphs, rename box, drag-reorder) — timeline-trim-gestures/pro-timeline
- Track struct field declarations (locked/ripple/magnetic/color) — registries-schema-hooks, wave 0b
- Source monitor / three-point marks UI — source-monitor, wave 2
- Snap engine, guide line, cursor zones — snap-engine, wave 1
- video_dirty_spans / track-id-reorder safety — player-rate-loop, wave 1
- fire_hook("marker_added") hook call site — audit finding assigns this to audio-analysis's marker-creation call sites (e.g. autocut::mark), not trim-model; mark_from_clip sets in/out points, not markers, so it is out of scope here.

## Files

| Op | Path | What |
|---|---|---|
| modify | src/model/ops/tracks.rs | Add `enum TrackFlag{Locked,Ripple,Magnetic}`; `locked_of(&self,id)->bool`, `ripple_tracks(&self)->Vec<usize>` (query); `set_track_flag`, `rename_track`, `set_track_color`, `move_track` (mutate). Consumes Track.locked/ripple/magnetic/color fields landed by wave 0b — do not redeclare them. rename_track/set_track_color/move_track are skeleton row-45 (pro-timeline, wave3) territory landed early for single ownership; audit-confirmed canonical over pro-timeline's proposed duplicate (see risks). |
| modify | src/model/ops/editing.rs | Add `shift_time(from,dt,tracks)` filtering Project.markers by `m.sequence==self.editing` and skipping Project.subtitles while a sequence is open (ponytail note); extend the EXISTING close_gap(a,b), ripple_delete_range(a,b), ripple_open(at,span) with a `tracks:&[usize]` param + Vec<Id> return instead of adding parallel close_gap_tracks/ripple_open_tracks fns (fixes both the extract_range duplication and legacy RippleDeleteInOut/PasteInsert all-track scoping in one change); add close_gap_at, clips_from, clips_at, nearest_edit_point, mark_from_clip (queries/mutator); insert_asset_clips_ranged (new, range+audio_track params) with old insert_asset_clips becoming a thin non-breaking wrapper. |
| create | src/model/ops/trim.rs | New file: `EditPoint`/`Side` types + the trim primitive set: ripple_trim (ripple:bool param folds plain+ripple trim into one fn), roll_edit, slip, slide, trim_edges, extend_edit, overwrite_asset, splice_in, lift_range, extract_range (thin wrapper over the now-scoped Project::ripple_delete_range), join_through, duplicate, unnest, replace_clip, magnetic_move. |
| modify | src/model/tests.rs | Append ~20 tests for the new ops (see tests list), including sequence-scoped shift_time and legacy-action ripple-scoping regressions. |
| create | src/ui/app/tools_trim.rs | `pub const TOOLS: &[ToolDef]` — one row per user-facing op below plus timeline.mark (mutate) and timeline.in_out (read), appended to TOOL_TABLES. timeline.splice/overwrite/lift/extract/replace are the canonical registrations for these names project-wide (audit fix 3/4) — source-monitor (wave2) must reuse them, not re-register. |
| create | src/ui/app/trim_actions.rs | `pub fn act(app:&mut App, a:Action)->bool` registered in ACT_HANDLERS: keyboard trim/nav/select dispatch, edit-point selection, keyframe nav, mark_clip via Project::mark_from_clip (reads app.timeline.edit_point/selection/playhead, calls model ops, push_undo_labeled+after_edit only if changed). |
| modify | src/ui/app/actions.rs | Update the 2 existing external call sites for RippleDeleteInOut (was ripple_delete_range(a,b)) and PasteInsert (was ripple_open(at,span)) to pass &self.project.ripple_tracks() as the new tracks arg — mechanical signature-follow, not a new feature; file isn't exclusively owned by any wave-1 workstream this wave. Audit finding: forgiveness also edits this file same wave — land this 2-line signature-follow first (mechanical, low-conflict), forgiveness rebases after. |
| modify | src/hotkeys.rs | Append 25 bound + 6 unbound Action variants (31 total) into the three pre-seeded `// ---- ws:trim-model ----` sections with exactly the chords in actions_and_hotkeys. |
| modify | src/ui/app/mod.rs | One line in the pre-seeded mod list (`mod trim_actions;`), one in ACT_HANDLERS, one in TOOL_TABLES — trim-model's marker section only. |
| modify | src/ui/app/tools_registry_tests.rs | Add OP_TOOLS rows for every new `pub fn(&mut self` op (or OP_INTERNAL+reason for insert_asset_clips_ranged, superseded by splice_in/overwrite_asset). |

## Model changes

- ops/tracks.rs: TrackFlag enum; locked_of/ripple_tracks queries; set_track_flag/rename_track/set_track_color/move_track mutators — all consume Track.{locked,ripple,magnetic,color} fields assumed landed by wave 0b, never redeclared here. rename_track/set_track_color/move_track are skeleton-attributed to pro-timeline wave3; landed early for single file ownership, and audit-confirmed canonical over pro-timeline's independently-proposed duplicate Track.color:u8 (flagged in risks).
- ops/editing.rs: shift_time(from,dt,tracks) moves Project.markers WHERE m.sequence==self.editing (main-timeline markers have sequence=None), and Project.in_point/out_point if >= from; while self.editing.is_some() it does NOT touch Project.subtitles (Cue.start/end are always main-timeline-relative — ponytail-noted ceiling, ripple-editing inside a sequence never shifts subtitles).
- ops/editing.rs: the EXISTING close_gap(a,b)/ripple_delete_range(a,b)/ripple_open(at,span) are extended in place to close_gap(a,b,tracks)/ripple_delete_range(a,b,tracks)->Vec<Id>/ripple_open(at,span,tracks) instead of adding parallel close_gap_tracks/ripple_open_tracks fns — this both eliminates a would-be duplicate of extract_range's logic and makes the 2 pre-existing external callers (RippleDeleteInOut, PasteInsert) properly ripple-scoped once their call sites pass ripple_tracks(); close_gap_at finds the local gap bounds under (track,t) and delegates.
- ops/editing.rs: insert_asset_clips_ranged adds range:Option<(f64,f64)>/audio_track params for three-point editing; old insert_asset_clips becomes a non-breaking wrapper (range=None). mark_from_clip(clip_id or None) sets in_point/out_point from a clip's [start,end) and is shared by the X keyboard action and the timeline.mark MCP tool.
- ops/trim.rs (new): EditPoint{track,t,side}/Side{Left,Right,Both} value types (owned here; snap-engine's TimelineState.edit_point imports them). ripple_trim/roll_edit/slip/slide/trim_edges/extend_edit/overwrite_asset/splice_in/lift_range/extract_range(thin wrapper over the now-scoped ripple_delete_range)/join_through/duplicate/unnest/replace_clip/magnetic_move — every op mutates on clones and applies atomically (stable ids, same track index, Transition.id preserved), honours locked_of and ripple_tracks, calls shift_time where it ripples.

## Engine changes

- None. video_dirty_spans (src/playback.rs:791-853, re-verified) already ignores unread struct fields — Track.locked/ripple/magnetic never enter its diff, so toggling them yields Some(vec![]) (no eviction) with zero engine code changes. It also has no Track.id check at each zipped index (only track count and per-index kind), so a same-kind move_track reorder can diff mismatched tracks against each other — this is player-rate-loop's fix to land (Track.id reorder -> full clear), not trim-model's; do not edit playback.rs. Coordination note extended to cover move_track/track.move, not just flags_do_not_dirty_video.

## UI changes

- src/ui/app/trim_actions.rs (new): keyboard-only dispatch for edit-point selection, trim ±1/±10 frames, extend/top/tail, slip, mark clip (via mark_from_clip), go to in/out, splice/overwrite/lift/extract at playhead, join/duplicate/unnest/replace-with-library-selection, select forward/backward/at-playhead, prev/next keyframe, track flag toggles (placeholder target: first selected clip's track). No mouse gestures, no new panes, no glyphs — purely App::act arms reading existing selection/playhead/timeline.edit_point state.

## New types and functions

- `pub enum TrackFlag { Locked, Ripple, Magnetic }` — src/model/ops/tracks.rs: Selector for set_track_flag; avoids three near-identical setters.
- `pub fn locked_of(&self, id: Id) -> bool` — src/model/ops/tracks.rs: Query used as a guard at the top of every mutating op below.
- `pub fn ripple_tracks(&self) -> Vec<usize>` — src/model/ops/tracks.rs: Indices of tracks with ripple==true; the scope for every ripple shift, incl. the extended legacy fns.
- `pub fn set_track_flag(&mut self, ti: usize, flag: TrackFlag, on: bool) -> bool` — src/model/ops/tracks.rs: Header toggle + track.set MCP tool backend.
- `pub fn rename_track(&mut self, ti: usize, name: String) -> bool; pub fn set_track_color(&mut self, ti: usize, color: Option<[u8;3]>) -> bool; pub fn move_track(&mut self, ti: usize, up: bool) -> bool` — src/model/ops/tracks.rs: Header rename/colour/reorder model half (UI lands in pro-timeline wave 3; model must exist now, exclusive file owner — flagged in risks as front-running skeleton row 45, and audit-confirmed canonical over pro-timeline's proposed duplicate).
- `pub fn shift_time(&mut self, from: f64, dt: f64, tracks: &[usize])` — src/model/ops/editing.rs: Shifts markers WHERE m.sequence==self.editing (t, keeps duration) and in/out (if >= from); does NOT touch subtitles while self.editing.is_some(). Called by every ripple op below.
- `fn close_gap(&mut self, a: f64, b: f64, tracks: &[usize]) [EXTENDED, was (a,b)]; pub fn ripple_delete_range(&mut self, a: f64, b: f64, tracks: &[usize]) -> Vec<Id> [EXTENDED, was () return]; pub fn ripple_open(&mut self, at: f64, span: f64, tracks: &[usize]) [EXTENDED, was (at,span)]` — src/model/ops/editing.rs: Adds tracks scoping + an id return to the EXISTING fns rather than duplicating them as close_gap_tracks/ripple_open_tracks; the 2 external call sites (RippleDeleteInOut, PasteInsert) now pass ripple_tracks(), which also fixes those legacy actions' all-track scoping for free.
- `pub fn close_gap_at(&mut self, track: usize, t: f64) -> bool` — src/model/ops/editing.rs: Finds the local gap bounds under (track,t), calls close_gap(a,b, &self.ripple_tracks()).
- `pub fn insert_asset_clips_ranged(&mut self, asset_id: Id, at: f64, video_track: Option<usize>, audio_track: Option<usize>, range: Option<(f64,f64)>) -> Vec<Id>` — src/model/ops/editing.rs: Three-point-editing primitive; old insert_asset_clips(asset_id,at,video_track) becomes a 1-line wrapper (range=None) so every existing caller stays source-compatible.
- `pub fn clips_from(&self, t: f64, track: Option<usize>, backward: bool) -> Vec<Id>; pub fn clips_at(&self, t: f64) -> Vec<Id>` — src/model/ops/editing.rs: Select Forward/Backward/Under-Playhead queries.
- `pub fn nearest_edit_point(&self, t: f64, track: Option<usize>) -> Option<EditPoint>` — src/model/ops/editing.rs: Seam-click/keyboard U: nearest clip boundary to t, side=Both if shared with a neighbour else Left/Right.
- `pub fn mark_from_clip(&mut self, clip: Option<Id>) -> Option<(f64,f64)>` — src/model/ops/editing.rs: Sets in_point/out_point from a clip's [start,end) (defaulting to the clip under playhead); shared by the X action and timeline.mark so the logic exists once.
- `pub struct EditPoint { pub track: usize, pub t: f64, pub side: Side } pub enum Side { Left, Right, Both }` — src/model/ops/trim.rs: Owned here (trim primitives' natural home); snap-engine's TimelineState.edit_point: Option<EditPoint> imports this type — coordinate, don't redefine.
- `pub fn ripple_trim(&mut self, id: Id, start_edge: bool, new_edge: f64, ripple: bool) -> bool` — src/model/ops/trim.rs: One fn for plain AND ripple trim (ripple:bool folds two near-duplicate ops into one); plain path keeps today's fits()-guarded all-or-nothing behaviour.
- `pub fn roll_edit(&mut self, right: Id, new_cut: f64) -> bool; pub fn slip(&mut self, ids: &[Id], dsrc: f64) -> bool; pub fn slide(&mut self, id: Id, dt: f64) -> bool; pub fn trim_edges(&mut self, edges: &[(Id,bool)], dt: f64, ripple: bool) -> bool` — src/model/ops/trim.rs: Remaining core trims; compute on clones, apply atomically so tidy() never prunes a transition mid-op.
- `pub fn extend_edit(&mut self, ep: &EditPoint, to: f64) -> bool` — src/model/ops/trim.rs: E key: roll if Both, else a single-side ripple_trim.
- `pub fn overwrite_asset(&mut self, asset: Id, at: f64, track: Option<usize>, range: Option<(f64,f64)>) -> Vec<Id>; pub fn splice_in(&mut self, asset: Id, at: f64, track: Option<usize>, range: Option<(f64,f64)>) -> Vec<Id>` — src/model/ops/trim.rs: Overwrite = split_at both bounds + delete(ripple=false) + insert_asset_clips_ranged; Splice = ripple_open(scoped) + insert_asset_clips_ranged.
- `pub fn lift_range(&mut self, a: f64, b: f64, tracks: Option<&[usize]>) -> Vec<Id>; pub fn extract_range(&mut self, a: f64, b: f64, tracks: Option<&[usize]>) -> Vec<Id>` — src/model/ops/trim.rs: Lift leaves a gap (genuinely new). Extract is now a one-line wrapper: `self.ripple_delete_range(a,b, tracks.unwrap_or(&self.ripple_tracks()))` — no reimplementation of split+delete+close_gap.
- `pub fn join_through(&mut self, left: Id) -> bool` — src/model/ops/trim.rs: Merges left with its right neighbour when same asset/contiguous src_time/identical params; keeps left's id.
- `pub fn duplicate(&mut self, ids: &[Id]) -> Vec<Id>; pub fn unnest(&mut self, clip: Id) -> Vec<Id>; pub fn replace_clip(&mut self, clip: Id, asset: Id) -> bool; pub fn magnetic_move(&mut self, ids: &[Id], dt: f64, dtrack: i32) -> bool` — src/model/ops/trim.rs: Duplicate via find_free_track; unnest is nest_selection's inverse (refuses speed!=1/src_in!=0); replace swaps asset/src_in only; magnetic_move falls back to ripple_open(scoped)+move on a blocked magnetic-track destination instead of refusing.
- `pub fn act(app: &mut App, a: Action) -> bool` — src/ui/app/trim_actions.rs: ACT_HANDLERS entry: snapshot-only-if-changed dispatch for all 31 Actions above onto the model fns; push_undo_labeled per op name.

## Actions and hotkeys

| Action id | Label | Chord | Note |
|---|---|---|---|
| select_edit_point | Select Nearest Edit Point | U | sets app.timeline.edit_point via Project::nearest_edit_point(playhead, hovered_track) |
| cycle_edit_side | Cycle Edit Point Side | Shift+U | Both -> Left -> Right -> Both on the current edit_point |
| trim_left_1 | Trim Edit -1 Frame | [ | ripple_trim(id, side, -1 frame, ripple=track.magnetic) on edit_point or selection edge |
| trim_right_1 | Trim Edit +1 Frame | ] | mirror of trim_left_1 |
| trim_left_10 | Trim Edit -10 Frames | Ctrl+[ | Shift+[ reaches egui as logical `{`; Ctrl form keeps the character |
| trim_right_10 | Trim Edit +10 Frames | Ctrl+] | see trim_left_10 |
| extend_edit | Extend Edit to Playhead | E | extend_edit(edit_point, playhead): roll if Both, trim if Left/Right |
| trim_top | Trim Start to Playhead | Q | ripple_trim(clip, start_edge=true, playhead, ripple=track.magnetic) on selection/under-playhead clip |
| trim_tail | Trim End to Playhead | W | mirror of trim_top on the end edge |
| slip_left | Slip -1 Frame | Alt+, | slip(selection, -frame_dur) |
| slip_right | Slip +1 Frame | Alt+. | slip(selection, +frame_dur) |
| mark_clip | Mark Clip | X | sets in/out to [start,end) of the clip under playhead or first selected, via Project::mark_from_clip |
| go_to_in | Go to In | Shift+I | seek(in_point) |
| go_to_out | Go to Out | Shift+O | seek(out_point) |
| join_through | Join Through Edit | Ctrl+J | join_through(selected or under-playhead clip id) |
| duplicate | Duplicate Clips | Ctrl+D | duplicate(selection); places copies via find_free_track |
| select_forward | Select Forward from Playhead | A | selection = clips_from(playhead, None, backward=false) |
| select_backward | Select Backward from Playhead | Shift+A | selection = clips_from(playhead, None, backward=true) |
| select_at_playhead | Select Clips Under Playhead | Ctrl+Shift+D | selection = clips_at(playhead) |
| prev_keyframe | Previous Keyframe | Alt+ArrowLeft | seek to nearest Clip::key_times() entry before playhead on the selected clip |
| next_keyframe | Next Keyframe | Alt+ArrowRight | mirror, after playhead |
| splice | Splice (Insert) at Playhead | Shift+V | ponytail: uses the last-selected library asset until source-monitor (wave2) supplies real marks |
| overwrite | Overwrite at Playhead | B | see splice ponytail note |
| lift | Lift In->Out | ; | lift_range(in_point, out_point, None) |
| extract | Extract In->Out | ' | extract_range(in_point, out_point, None) — thin wrapper over the now-scoped Project::ripple_delete_range |
| close_gap | Close Gap at Playhead |  | unbound; close_gap_at(track, playhead) — gap-click gesture wiring is snap-engine's/trim-gestures' concern |
| unnest | Un-nest Sequence Clip |  | unbound; unnest(selected Sequence clip) |
| replace_clip | Replace with Library Selection |  | unbound; replace_clip(selected clip, last-selected library asset) |
| toggle_track_lock | Lock/Unlock Track under Cursor |  | unbound; set_track_flag(track of first-selected clip, Locked, !cur) — header glyph binds this properly in wave2 |
| toggle_track_ripple | Toggle Ripple (Sync) on Track |  | unbound; same placeholder pattern as toggle_track_lock |
| toggle_track_magnetic | Toggle Magnetic Track |  | unbound; same placeholder pattern |

## MCP tools (required — every capability must be scriptable)

| Tool | Kind | Args | Description | Maps to |
|---|---|---|---|---|
| track.set | mutate | index:integer:true, locked:boolean:false, ripple:boolean:false, magnetic:boolean:false, name:string:false, color:array:false ([r,g,b] or null) | Edit one track's flags/name/color. Canonical color form [r,g,b] (audit fix 2) — pro-timeline must not add a competing u8 color field/tool. | Project::set_track_flag / rename_track / set_track_color |
| track.move | mutate | index:integer:true, up:boolean:true | Reorder a track within its kind. Coordination: do not exercise until player-rate-loop's Track.id-reorder full-clear fix lands (see risks). | Project::move_track |
| track.list | read | (none) | Every track: index, kind, name, locked, ripple, magnetic, color, clip count. | iterate self.project.tracks |
| timeline.shift_time | mutate | from:number:true, dt:number:true, tracks:array:false (default: ripple_tracks()) | Shift markers (matching the current sequence scope)/cues/in-out at/after `from` by dt seconds. | Project::shift_time |
| timeline.close_gap | mutate | track:integer:true, t:number:true | Close the gap under t on that track (ripple tracks only). | Project::close_gap_at |
| timeline.mark | mutate | clip_id:integer:false (default: clip under playhead) | Set in/out to [start,end) of a clip; returns {in,out}. | Project::mark_from_clip |
| timeline.in_out | read | (none) | Current in_point/out_point, or null if unset. | self.project.in_point / out_point |
| timeline.ripple_trim | mutate | clip_id:integer:true, start:boolean:true, edge:number:true, ripple:boolean:false | Trim one edge; ripple=true shifts downstream ripple-tracked clips. | Project::ripple_trim |
| timeline.roll | mutate | right_clip_id:integer:true, cut:number:true | Move a shared cut; total length unchanged. | Project::roll_edit |
| timeline.slip | mutate | clip_ids:array:true, dsrc:number:true | Change source window in place. | Project::slip |
| timeline.slide | mutate | clip_id:integer:true, dt:number:true | Move a clip; neighbours absorb. | Project::slide |
| timeline.trim_edges | mutate | edges:array:true ([[clip_id,is_start]]), dt:number:true, ripple:boolean:false | Asymmetric multi-roller trim. | Project::trim_edges |
| timeline.extend | mutate | track:integer:true, t:number:true, side:string:true (Left\|Right\|Both), to:number:true | Extend the edit point to a time. | Project::extend_edit |
| timeline.splice | mutate | asset_id:integer:true, at:number:true, track:integer:false, in:number:false, out:number:false | Insert edit: ripple-open then place. Canonical registration (wave1); source-monitor (wave2) reuses this, does not re-register (audit fix 3). | Project::splice_in |
| timeline.overwrite | mutate | asset_id:integer:true, at:number:true, track:integer:false, in:number:false, out:number:false | Overwrite edit: no ripple. Canonical; source-monitor reuses this, does not re-register (audit fix 3). | Project::overwrite_asset |
| timeline.lift | mutate | a:number:true, b:number:true, tracks:array:false | Remove range, leave a gap. Canonical; source-monitor reuses this, does not re-register (audit fix 3). | Project::lift_range |
| timeline.extract | mutate | a:number:true, b:number:true, tracks:array:false | Remove range, close the gap. Canonical; source-monitor reuses this, does not re-register (audit fix 3). | Project::extract_range (wraps ripple_delete_range) |
| timeline.join | mutate | clip_id:integer:true | Merge with the right neighbour if contiguous/same asset. | Project::join_through |
| timeline.duplicate | mutate | clip_ids:array:false (default: selection) | Duplicate clips onto a free track. | Project::duplicate |
| timeline.unnest | mutate | clip_id:integer:true | Flatten a Sequence clip back onto the timeline. | Project::unnest |
| timeline.replace | mutate | clip_id:integer:true, asset_id:integer:true | Swap the asset, keeping duration/effects/transform. Canonical name (supersedes timeline.replace_clip); source-monitor calls Project::replace_clip directly instead of adding a second tool (audit fix 4). | Project::replace_clip |
| timeline.magnetic_move | mutate | clip_ids:array:true, dt:number:true, dtrack:integer:false | Move; shoves neighbours on magnetic tracks instead of refusing. | Project::magnetic_move |
| timeline.select_forward | read | t:number:false (default playhead), track:integer:false, backward:boolean:false | Clip ids from a time forward/backward. | Project::clips_from |
| timeline.clips_at | read | t:number:false (default playhead) | Clip ids under a time. | Project::clips_at |
| timeline.edit_point | ui | track:integer:false, t:number:false, side:string:false, clear:boolean:false | Get/set/clear the selected cut for keyboard trimming. | app.timeline.edit_point (type from model::ops::trim::EditPoint) |
| timeline.keyframe_nav | ui | direction:string:true (prev\|next) | Seek to the previous/next keyframe of the selected clip. | Clip::key_times() + App::seek |

**Luau:** Every op in tools_trim.rs is a ToolDef, so editor.tool("timeline.ripple_trim",{...}), editor.tool("timeline.mark",{}) etc. work immediately (same table feeds tools/list and editor.tools()). No -- @on hooks needed by this workstream (fire_hook is command-palette's wave-1 deliverable; marker_added is audio-analysis's, not trim-model's — see scope_out) — a script doing 30 splices in a loop must stay O(n) per call (pinned by the perf test) so a project-wide auto-cut-then-splice script stays under the scripting budget on a 1000-clip project.

## Tests

| Test | File | Asserts |
|---|---|---|
| ripple_trim_shifts_only_ripple_tracks_and_moves_main_timeline_markers_cues | src/model/tests.rs | ripple_trim(...,ripple=true) shifts downstream clips on ripple==true tracks only, and shift_time moves Project.markers with sequence==None/self.editing and Project.in_point/out_point at/after the trimmed edge; non-ripple tracks and markers before the edge stay put; subtitle Cue.start/end unaffected when self.editing is Some. |
| shift_time_never_touches_subtitles_or_other_sequence_markers_while_editing_open | src/model/tests.rs | opening a sequence, then ripple-trimming inside it: main-timeline (sequence=None) markers and Project.subtitles are byte-identical before/after; markers with sequence==Some(other_id) are also untouched. |
| legacy_ripple_actions_now_honour_ripple_tracks | src/model/tests.rs | calling the extended ripple_delete_range/ripple_open with tracks=ripple_tracks() (mirroring RippleDeleteInOut/PasteInsert's new call sites) leaves a non-ripple secondary track's clips untouched, where the pre-change all-track version would have shifted them. |
| roll_edit_keeps_total_length_and_transition_id | src/model/tests.rs | roll_edit on an abutting pair changes only the cut point (sum of durations unchanged) and a Transition at that cut keeps its id. |
| slip_clamps_to_source_window_forward_and_reversed | src/model/tests.rs | slip respects head_room/max_clip_duration for both reverse=false and reverse=true clips; duration/start never change, only src_in. |
| trim_edges_asymmetric_multi_clip | src/model/tests.rs | trim_edges on a mixed (start,end) edge set moves each independently and refuses (no-op) if any resulting clip would overlap a non-participant. |
| splice_in_is_linear_on_1000_clips | src/model/tests.rs | splice_in near the head of a 1000-clip single-ripple-track timeline completes in well under the previous O(n^2) cost (wall-clock bound, e.g. < 5ms) and every downstream clip shifted by exactly `span`. |
| overwrite_asset_clears_only_the_overlap | src/model/tests.rs | overwrite_asset splits at both bounds, deletes only clips fully inside [a,b), leaves clips outside untouched, no ripple. |
| lift_leaves_gap_extract_delegates_to_ripple_delete_range | src/model/tests.rs | lift_range leaves later clips' start unchanged; extract_range shifts them left by (b-a) on ripple tracks only, confirmed by asserting it produces the identical result as calling ripple_delete_range directly with the same tracks. |
| join_through_merges_contiguous_refuses_otherwise | src/model/tests.rs | same-asset contiguous src_time neighbours merge keeping the left id; different asset or a gap between them leaves both clips unchanged. |
| duplicate_places_on_free_track_with_new_ids | src/model/tests.rs | duplicate(ids) returns new ids distinct from the originals, placed without overlapping the source clips. |
| unnest_is_inverse_of_nest_selection | src/model/tests.rs | nest_selection then unnest on the resulting Sequence clip restores equivalent clips at their original timeline positions; unnest on speed!=1 or src_in!=0 returns empty. |
| replace_clip_keeps_duration_effects_transform | src/model/tests.rs | replace_clip swaps asset/src_in only; duration, effects, x/y/scale/rotation, label are unchanged. |
| magnetic_move_shoves_on_magnetic_track_else_refuses | src/model/tests.rs | a blocked move on a magnetic-track destination succeeds by ripple-opening space first; the same move on a non-magnetic track returns false unchanged (today's behaviour). |
| locked_track_refuses_every_new_op | src/model/tests.rs | parametrized over ripple_trim/roll_edit/slip/slide/trim_edges/splice_in/overwrite_asset/lift_range/extract_range/join_through/duplicate/magnetic_move: locked_of==true on the target track leaves the project byte-identical. |
| ripple_tracks_excludes_position_locked_secondaries | src/model/tests.rs | a track with ripple==false never shifts when a ripple op fires elsewhere, even though [a,b) is free on it. |
| close_gap_at_finds_local_gap | src/model/tests.rs | close_gap_at(track,t) computes [prev clip end, next clip start] on that track and shifts only ripple tracks by that span. |
| insert_asset_clips_unchanged_ranged_sets_window | src/model/tests.rs | old insert_asset_clips(asset,at,vt) output is byte-identical to before; insert_asset_clips_ranged with a range sets clip.src_in/duration from it. |
| track_flag_and_rename_color_move_setters | src/model/tests.rs | set_track_flag/rename_track/set_track_color/move_track mutate only the target track; move_track at a kind boundary returns false unchanged. |
| nearest_edit_point_side_both_at_shared_seam | src/model/tests.rs | a boundary shared by two clips on one track returns Side::Both; a track's own start/end returns the single applicable side. |
| mark_from_clip_sets_in_out_from_bounds | src/model/tests.rs | mark_from_clip(Some(id)) sets in_point/out_point to [clip.start, clip.start+clip.duration); mark_from_clip(None) uses the clip under playhead; returns None with no clip present. |
| trim_actions_snapshot_only_if_changed | src/ui/app/trim_actions.rs | each Action arm pushes exactly one undo entry when the underlying op returns true/non-empty, and zero when it returns false/empty (e.g. locked track, no selection). |
| every_edit_op_has_a_tool_covers_trim_model | src/ui/app/tools_registry_tests.rs | every new pub fn(&mut self in ops/{tracks,editing,trim}.rs appears in tools_trim::TOOLS' maps_to set or OP_INTERNAL with a reason (extends the wave-0b structural test); also asserts timeline.splice/overwrite/lift/extract/replace names appear exactly once project-wide (audit fix 3/4). |

## Verification checklist

- [ ] cargo test (full suite): existing count + ~22 new tests, all green, zero pre-existing test changed
- [ ] cargo run -- --selftest passes (no new idle-repaint source added)
- [ ] manual/scripted: MCP timeline.ripple_trim then markers.list/subtitles.get shows shifted main-timeline markers/cues and untouched subtitles when run inside an open sequence; track.set(locked=true) then timeline.slip on that track returns an error/no-op
- [ ] manual/scripted: MCP timeline.mark on a clip then timeline.in_out reflects [start,end); go_to_in/go_to_out seek correctly
- [ ] perf: splice_in on a synthetic 1000-clip project completes in the sub-5ms range asserted by the test, not just 'faster'
- [ ] scripts/size.ps1 -Note trim-model shows a delta close to the +118 KB estimate; PR body carries `size: +N KB`
- [ ] grep confirms src/playback.rs was NOT modified by this PR (flags_do_not_dirty_video / move_track id-reorder safety are coordination notes, not file changes here)
- [ ] diff review: insert_asset_clips's old call sites (app/files.rs, library.rs, tools_timeline.rs, drops.rs) compile unchanged; the 2 updated close_gap/ripple_delete_range/ripple_open call sites in actions.rs pass ripple_tracks() and behave identically when all tracks happen to be ripple==true (today's default)
- [ ] tool_names_unique_and_namespaced passes with timeline.splice/overwrite/lift/extract/replace registered exactly once (this file), confirming source-monitor's later wave-2 additions did not duplicate them

## Acceptance criteria

- [ ] cargo test passes with all new model/trim_actions tests green and no existing test changed
- [ ] every new `pub fn(&mut self` in ops/{tracks,editing,trim}.rs has an OP_TOOLS row (or OP_INTERNAL+reason) in tools_registry_tests.rs; every_edit_op_has_a_tool passes
- [ ] every new Action has a hotkey table row (chord or None) and is reachable via ui.action; ui_action_covers_every_action passes
- [ ] no_duplicate_defaults and reserved_chords_are_free stay green with the 25 new bound chords added (31 rows total incl. 6 unbound placeholders)
- [ ] ripple ops shift markers (scoped to marker.sequence == self.editing), subtitle cues (main timeline only) and in/out points via shift_time (verified by test, not just clips); a ripple op fired while a sequence is open leaves main-timeline markers/subtitles untouched
- [ ] locked tracks refuse every new op (move/trim/splice/overwrite/lift/extract/duplicate/join/unnest/replace/magnetic_move)
- [ ] ripple ops touch only tracks with ripple==true; secondary (non-ripple) tracks never shift — including the pre-existing RippleDeleteInOut/PasteInsert actions now that close_gap/ripple_delete_range/ripple_open take a tracks scope
- [ ] splice_in / ripple_open is O(n) on 1000 clips (perf test < 5ms, was O(n^2))
- [ ] roll_edit and slide preserve Transition.id on the touched cut
- [ ] timeline.mark and timeline.in_out are reachable over MCP (kind mutate / read) and cover mark_clip/go_to_in/go_to_out
- [ ] tool_names_unique_and_namespaced passes: timeline.splice/overwrite/lift/extract/replace stay singly-registered here, not duplicated by source-monitor's wave-2 tools_source.rs (audit fix 3/4)
- [ ] scripts/size.ps1 delta for this PR is within +120 KB (documented if over)
- [ ] cargo run -- --selftest passes; idle-repaint unaffected (no timers added)

## Risks

| Risk | Mitigation |
|---|---|
| Track.locked/ripple/magnetic fields are this workstream's hard precondition from wave 0b (registries-schema-hooks) but that workstream's actual field names/shapes aren't visible yet. | Verify src/model/track.rs before starting; if `ripple` isn't Option<bool> resolved-in-from_json as the skeleton decided, adapt ripple_tracks()'s unwrap_or(false) accordingly — do not add the fields yourself (schema-first principle: feature branches never touch a type definition). |
| EditPoint/Side is defined here but consumed by snap-engine's TimelineState.edit_point — a genuine cross-workstream (both wave 1, concurrent) type dependency, not expressible as disjoint files. | Land trim-model's model/ops/trim.rs early; snap-engine imports `crate::model::ops::trim::EditPoint` rather than redefining it. Flag this explicitly to whoever runs snap-engine. |
| insert_asset_clips signature change could silently break other callers if done in-place instead of via a wrapper. | Keep insert_asset_clips's old signature and behavior byte-identical; insert_asset_clips_ranged is strictly additive. |
| Live ripple ops shift every downstream clip's span; called from a future live-drag gesture (wave 2) this would evict per-frame. | Not this workstream's concern (no gestures here), but document in each fn's doc comment: 'apply on release only, never per drag-frame' for trim-gestures to read. |
| flags_do_not_dirty_video regression test belongs in playback.rs's own test module, which trim-model does not own this wave. The same applies to move_track/track.move: video_dirty_spans (playback.rs:791-846, verified) has no Track.id check at each zipped index, only count and per-index kind — a same-kind reorder diffs mismatched tracks and can produce wrong (not just stale) dirty spans, and the fix is player-rate-loop's, a concurrent wave-1 sibling. | Verify manually that video_dirty_spans never reads locked/ripple/magnetic (confirmed) and leave a PR note asking player-rate-loop to add the id-reorder pinning test; do not edit playback.rs. Additionally: land/merge trim-model's move_track+track.move only after player-rate-loop's fix is confirmed merged, or add an inline `// ponytail: unsafe with the preview cache until player-rate-loop lands its id-reorder fix` comment on move_track so a script calling track.move early fails loud in review rather than silently corrupting the cache. |
| rename_track/set_track_color/move_track (skeleton row 45) are attributed by the skeleton's own workstream feature lists to pro-timeline (wave 3), not trim-model. Landing them here front-runs scope not counted in wave-1's line/size budget and not known to whoever authors the pro-timeline plan later. | This risk entry itself is the notice: pro-timeline's wave-3 plan should be authored knowing these 3 fns already exist in src/model/ops/tracks.rs and only needs to add header UI (rename box, colour swatch, drag-reorder), not re-derive the model layer. |
| Q/W/[/]/;/'/X/A letters could collide with a not-yet-landed sibling wave-1 action. | Re-run no_duplicate_defaults after rebasing onto the latest wave-1 tip before landing; all 25 bound chords were verified free against the current hotkeys.rs table at plan time. |
| Extending close_gap/ripple_delete_range/ripple_open's signatures touches 2 call sites and 1 test outside trim-model's owns_files (src/ui/app/actions.rs, plus the old model.rs:6467-region test moved to tests.rs by wave 0a) — a small but real cross-file edit in a wave-1 PR. | Both are mechanical 1-arg-added follow-throughs, not new behaviour; actions.rs is not exclusively owned by any wave-1 workstream this wave, so the edit is low-conflict. Called out explicitly in files[] rather than left implicit. |
| Audit finding (blocker/major): source-monitor's wave-2 tools_source.rs plan lists timeline.splice/overwrite/lift/extract/replace_clip rows duplicating trim-model's already-registered timeline.splice/overwrite/lift/extract/replace (same underlying Project fns) — would fail tool_names_unique_and_namespaced. | trim-model's rows are canonical (wave1, lands first); annotated their mcp_tools descs accordingly. source-monitor's three-point-edit UI must call these existing tools/fns directly; only genuinely new names (match_frame, place, append_at_end, etc.) belong in tools_source.rs. |
| Audit finding (blocker): pro-timeline's wave-3 plan independently proposes a second Track.color:u8 field plus duplicate rename_track/set_track_color/move_track/track.rename/track.set_color, conflicting with registries-schema-hooks' Track.color:Option<[u8;3]> (wave0) and this plan's own fns. | Confirmed resolution: this plan's rename_track/set_track_color/move_track + track.set(color:[r,g,b]) are canonical. No change needed in trim-model itself; pro-timeline's plan must drop its from-scratch color field/tools and build only header UI over these. |

## Suggested implementation order

1. 1. model/ops/trim.rs: EditPoint/Side + ripple_trim/roll_edit/slip/slide/trim_edges (+unit tests) — the load-bearing primitives everything else composes.
2. 2. model/ops/editing.rs: shift_time (sequence-scoped, subtitles skipped while editing.is_some()), extend close_gap/ripple_delete_range/ripple_open in place with a tracks param+Vec<Id> return, close_gap_at, insert_asset_clips_ranged, clips_from/clips_at/nearest_edit_point, mark_from_clip (+tests incl. the O(n) perf test and the legacy-action scoping regression).
3. 3. src/ui/app/actions.rs: update the 2 call sites (RippleDeleteInOut, PasteInsert) to pass ripple_tracks() into the newly-scoped fns.
4. 4. model/ops/trim.rs cont.: extend_edit, overwrite_asset, splice_in, lift_range, extract_range (wrapper), join_through, duplicate, unnest, replace_clip, magnetic_move (+tests).
5. 5. model/ops/tracks.rs: TrackFlag, locked_of, ripple_tracks, set_track_flag, rename_track, set_track_color, move_track (+tests, incl. locked-track refusal matrix over every op from steps 1-4).
6. 6. ui/app/tools_trim.rs: ToolDef rows for every op plus timeline.mark/timeline.in_out; wire into TOOL_TABLES. Confirm no collision with source-monitor's planned tools_source.rs names (audit fix 3/4).
7. 7. hotkeys.rs: append the 31 Action variants/chords (25 bound, 6 unbound).
8. 8. ui/app/trim_actions.rs: act() dispatch for every Action; wire into ACT_HANDLERS.
9. 9. tools_registry_tests.rs: OP_TOOLS rows; run every_edit_op_has_a_tool, mutate_rows_roll_back_on_error, ui_action_covers_every_action, tool_names_unique_and_namespaced.
10. 10. Full cargo test, --selftest, scripts/size.ps1 -Note trim-model; open a PR noting the flags_do_not_dirty_video AND move_track/track.move coordination note for player-rate-loop, plus a note to source-monitor/pro-timeline authors about canonical tool/field ownership (audit fixes 2-4).

## Deliberate simplifications (`// ponytail:`)

- Splice/Overwrite hotkeys act on the last-selected library asset, not real three-point source marks — upgrade path: source-monitor (wave 2) supplies src_in/src_out.
- toggle_track_lock/ripple/magnetic Actions operate on the first selected clip's track as a keyboard-only placeholder — upgrade path: timeline-trim-gestures' header glyphs (wave 2) give them a real target.
- close_gap Action ships unbound with no gesture to set a gap selection yet — upgrade path: snap-engine/trim-gestures' GapSelect state calls close_gap_at directly once it lands.
- unnest refuses any clip with speed != 1 or src_in != 0 rather than compositing the retime into child clip starts — upgrade path noted inline, not scheduled.
- join_through requires exact same-asset/contiguous/identical-params neighbours (no fuzzy merge) — simplest correct rule, matches the critique's own scope.
- shift_time never shifts Project.subtitles while a sequence is open (Cue has no sequence tag, and cues are always main-timeline-relative) — upgrade path: give Cue a sequence field if per-sequence subtitle editing is ever wanted; until then this is a documented ceiling, not a bug.

## Review trail

- F1 (shift_time sequence-scoping, CONFIRMED via model.rs:2010 Marker.sequence doc + :3826-3868 open/close_sequence never touching markers/subtitles + :2975 subtitles has no sequence tag): shift_time now filters Project.markers by `m.sequence == self.editing` and skips Project.subtitles entirely while `self.editing.is_some()` (ponytail-noted ceiling); added a dedicated test.
- F2 (mark_clip/go_to_in/out unreachable over MCP, CONFIRMED — no in_point/out_point tool exists in mcp/tools.rs): added `timeline.mark` (mutate) and `timeline.in_out` (read) tools; factored the shared logic into a new Project::mark_from_clip fn reused by the keyboard action and both tools.
- F3 (extract_range duplicates Project::ripple_delete_range, CONFIRMED at model.rs:3483): dropped the planned close_gap_tracks/ripple_open_tracks new-fn pair; instead extended the existing close_gap/ripple_delete_range/ripple_open in place with a `tracks: &[usize]` param and a Vec<Id> return, and made extract_range a one-line wrapper over the extended ripple_delete_range.
- F4 (legacy RippleDeleteInOut/PasteInsert stay all-track, CONFIRMED at model.rs:3465/:3498, only 3 call sites total): resolved for free by F3's signature change — the 2 existing external call sites (RippleDeleteInOut, PasteInsert, both in the actions.rs split of app.rs) and the ripple_open test now pass ripple_tracks(), so the legacy actions become ripple-scoped too. Added to files[] and tests.
- F5 (move_track ships before player-rate-loop's Track.id-reorder full-clear fix, CONFIRMED — playback.rs:791-846 has no id check, only len/kind): added a risk entry alongside the existing flags_do_not_dirty_video coordination note; move_track/track.move now carry the same 'do not land before player-rate-loop's fix merges' flag, since trim-model cannot itself test a private fn in playback.rs.
- F6 (rename_track/set_track_color/move_track are skeleton row 45, attributed to pro-timeline wave 3 not trim-model): kept the fns here per the plan's own stated rationale (single file owner, avoid re-derivation later) but added an explicit risk entry so pro-timeline's wave-3 plan is authored knowing they already exist.
- F7 (hotkey count bookkeeping, CONFIRMED by direct recount of actions_and_hotkeys: 25 bound + 6 unbound = 31, not 22+8=30): corrected every '22 bound'/'30 total' reference in files[]/acceptance_criteria/implementation_order to 25 bound / 31 total; actions_and_hotkeys itself was already correct and is unchanged.
- A1 (audit fix 3, duplicate MCP tool names, blocker): confirmed trim-model's timeline.splice/overwrite/lift/extract/replace are canonical (wave1, land first); annotated their mcp_tools descs so source-monitor's wave-2 tools_source.rs plan reuses them instead of re-registering, avoiding a tool_names_unique_and_namespaced build failure. No fn/signature changes here.
- A2 (audit fix 4, timeline.replace vs timeline.replace_clip, major): same resolution as A1 — timeline.replace (this plan) is the canonical name; source-monitor must call Project::replace_clip directly rather than adding a second tool.
- A3 (audit fix 2, Track.color duplicate, blocker — re-verified against registries-schema-hooks' own project_fields listing Track.color:Option<[u8;3]>): added a risk entry confirming this plan's rename_track/set_track_color/move_track + track.set(color:[r,g,b]) are canonical, so pro-timeline's wave-3 plan (which independently proposed a conflicting Track.color:u8) must drop its version. No change to this plan's own fns/tools.
- A4 (audit fix 5, missing fire_hook("marker_added") owner, major): verified trim-model owns no marker-creation call site (mark_from_clip sets in/out points, not markers) — added an explicit scope_out line so this hook is not silently assumed covered here; owner is audio-analysis per the audit's own assignment.
- A5 (audit fix 1, actions.rs same-wave double-edit, major): re-confirmed the edit is a mechanical 2-arg follow-through (RippleDeleteInOut/PasteInsert call sites); added an explicit landing-order note (trim-model first, forgiveness rebases after) to files[] since no shared marker section covers this legacy match arm.
- Everything not touched by a finding (scope_in remainder, glyphs, luau, project_fields, settings_fields, worktree/wave/name/title, ui_changes, depends_on) is preserved verbatim.
