# audio-analysis: Peaks toolkit, beat/scene detection, ducking, normalize, sync offset

**Workstream:** `audio-analysis` · **Issue:** [#19](https://github.com/KashTheKing/simple-editor/issues/19) · **Wave:** 1 · **Branch/worktree:** `feat/audio-analysis` → `../simple-editor-wt/audio-analysis` · **Depends on:** size-diet · **~1130 new lines · Δ exe ≈ +90 KB**

Read [agents.md](../../../agents.md), [goals.md](../../../goals.md), [notes.md](../../../notes.md) and the master plan [README.md](../README.md) (registry protocol, keymap, modifier table) before starting.

## Summary

Wave-1 Peaks analysis toolkit (engine/analysis.rs, pure fns, no new deps): onsets/BPM, peak/RMS levels, cross-correlation offset, ffmpeg scene-cut parsing, plus two small clip-time conversion helpers so onset/cut times (source seconds) land correctly on trimmed/retimed clips. Verbs on top, all non-destructive-first: Detect Beats/Split at Beats, Auto-duck as editable volume keyframes, Normalize/Match Loudness, silence auto-cut + scene-cut 'Mark instead' (silence = range markers via mark_ranges; scene cuts = point markers via add_clip_marker directly, since a scene change is an instant, not a range) with per-segment toggles. Every marker-creation call site fires App::fire_hook("marker_added", ...) so this workstream owns that Luau event, not just its own tools. 9 new MCP tools (auto-Luau-exposed), 5 unbound Actions, 3 Settings fields. Zero model.rs changes — everything read/written is already public API.

## Motivation

Closes gap-matrix rows 68-74/90/96 (Essential Sound half, auto-ducking, normalize/loudness, dialogue DSP hook, beat detection, multicam sync fn, scene cuts, auto-cut as markers first) using the H12 Peaks-analysis-toolkit architecture hook. Delivers goals.md's 'AI as co-pilot: result stays editable' principle (keyframes/markers, never destructive-first) and the hard MCP-parity requirement (9 new tools, zero bespoke Luau code). Also closes the audit's marker_added event-ownership gap: this workstream is the only one that creates markers via mark_ranges/add_clip_marker for detection results, so it is the correct (and only remaining unclaimed) owner of that fire_hook call site. Zero new deps, all pure fns over the existing waveform::Peaks/autocut plumbing already in dont_rebuild.

## In scope

- engine/analysis.rs pure toolkit: levels, onsets, bpm, xcorr_offset, normalize, match_loudness, duck, scene_cuts, plus to_clip_local/to_timeline_t conversion helpers (mirror autocut::to_timeline's src_in/speed math, verified at autocut.rs:78-91) used at every marker/split call site so onset and scene-cut times land correctly on trimmed or retimed clips
- Auto-cut pane (autocut_ui.rs) gains: per-segment toggle + 'Mark instead' for silence auto-cut, and new Scene cuts / Beats / Loudness / Duck sections
- 5 new Actions + unbound hotkeys (DetectBeats, SplitAtBeats, AutoDuck, Normalize, MatchLoudness) wired through audio_actions.rs
- 9 new MCP tools in tools_audio.rs, automatically Luau-exposed
- 3 new Settings fields (duck_depth_db, duck_ramp_ms, beat_thr)
- Owning the `marker_added` Luau event: every call site in this workstream that adds a Marker (mark_ranges, detect_beats/split_at_beats markers, scene-cut markers) calls App::fire_hook("marker_added", payload) after the write — verified via grep that no fire_hook call exists anywhere in src/ yet, so this is a genuinely new, uncontested call site

## Out of scope

- Track/Bus volume automation lane + real K-weighted LUFS meter + limiter/de-esser/de-hum + Essential Sound role tag/UI (audio-dsp-automation workstream)
- Multicam UI: angle grid, nested-sequence creation, angle switching (pro-monitor workstream, wave 3) — this workstream ships only the xcorr_offset engine fn + audio.sync_offset tool it will consume
- Export loudnorm flag (export-deliver workstream)
- Timeline volume-line/lane paint (already exists for Clip.volume per-clip; track/bus lane paint is audio-dsp-automation's)
- Filler-word/pause removal, transcript-based cuts (transcript-captions workstream)
- Implementing the @on script-execution runtime for fire_hook (that's command-palette's wave-1 job per the mcp_parity narrative; this workstream only adds call sites for its own event)
- Editing audio-dsp-automation's own plan/files — the fix that audio-dsp-automation's depends_on must list audio-analysis (because its inspector_audio.rs dispatches Action::AutoDuck/Action::Normalize, declared here) is recorded here as a coordination note for that sibling plan, not applied to this plan's files

## Files

| Op | Path | What |
|---|---|---|
| modify | src/engine/mod.rs | Add `pub mod analysis;` (line 3, before `pub mod autocut;`, alphabetical). |
| create | src/engine/analysis.rs | Levels{peak_db,rms_db}, levels(), onsets(), bpm(), xcorr_offset(), NormMode{Peak,Rms}, normalize(), match_loudness(), duck(), scene_cuts()+parse_showinfo_pts(), plus to_clip_local(t_src,&Clip)->f64 and to_timeline_t(t_src,&Clip)->f64 (source-seconds -> clip-local / timeline time, same (t - src_in)/speed math as autocut::to_timeline, confirmed present at autocut.rs:78-91), plus #[cfg(test)] mod with ~10 tests. |
| modify | src/ui/autocut_ui.rs | Detection gains `included: Vec<bool>` (per-segment toggle) and a `kind` label; add mark_ranges(app: &mut App, ranges: &[(f64,f64)], name_prefix: &str) -> Vec<Id> helper (add_marker+marker_mut, mirrors subtitles_ui.rs:515-517) for genuine (start,end) silence ranges only — takes &mut App (not &mut Project) so it can call app.fire_hook("marker_added", payload) once per created marker after the write; add 4 new collapsible sections to show(): Scene cuts, Beats, Loudness, Duck — each with its own small State struct held on AutoCutState. |
| create | src/ui/app/tools_audio.rs | `pub const TOOLS: &[ToolDef]` with 9 rows (audio.analyze, audio.beats, audio.duck, audio.normalize, audio.match_loudness, audio.sync_offset, autocut.detect, autocut.mark, media.scene_cuts) and their `run` fns, using the wave-0b Args helper (tools_args.rs) for ids_or_selection etc. audio.beats (as_markers/split) and media.scene_cuts (as_markers/split) both run every detected source-time through analysis::to_clip_local / to_timeline_t before calling Project::add_clip_marker / split_at, and call app.fire_hook("marker_added", payload) once per marker written; media.scene_cuts's as_markers path adds one point marker (duration 0, via add_clip_marker directly) per cut, not a range via mark_ranges. |
| create | src/ui/app/audio_actions.rs | `pub fn act(app: &mut App, a: Action) -> bool` handling DetectBeats/SplitAtBeats/AutoDuck/Normalize/MatchLoudness by calling the same engine::analysis fns (including to_clip_local/to_timeline_t for beat markers/splits) the pane buttons and MCP tools call, with push_undo_labeled + after_edit; DetectBeats/SplitAtBeats fire app.fire_hook("marker_added", payload) per marker written (same shared inner fn as tools_audio.rs, never re-derived); returns false (not handled) for every other Action so it composes in ACT_HANDLERS. |
| modify | src/hotkeys.rs | Append 5 rows in a new `// ---- ws:audio-analysis ----` section at the end of the actions! block (after ToolSpacer, line 155): DetectBeats/SplitAtBeats/AutoDuck/Normalize/MatchLoudness, all `None` (unbound per skeleton keymap). Note for coordination: AutoDuck and Normalize are declared here, not in any wave-0b pre-seeded stub — audio-dsp-automation's inspector_audio.rs dispatches these Action variants and must therefore declare `depends_on: ["audio-analysis"]` in its own plan (fix recorded in changelog; not this workstream's file to edit). |
| modify | src/settings.rs | Append `pub duck_depth_db: f32`, `pub duck_ramp_ms: u32`, `pub beat_thr: f32` to the Settings struct (after panel_opacity, line 257) and their defaults (-12.0, 200, 1.6) to impl Default (after panel_opacity: 255, line 324), each under a `// ---- ws:audio-analysis ----` comment. |
| modify | src/ui/app/mod.rs | In the pre-seeded ws:audio-analysis marker lines: add `mod tools_audio; mod audio_actions;`, replace the ws:audio-analysis placeholder in TOOL_TABLES with `tools_audio::TOOLS,`, and in ACT_HANDLERS with `audio_actions::act,`. |

## Model changes

- None. Every field this workstream reads/writes already exists and is already `pub`: Clip.volume: Animated (model.rs:2427), Animated::toggle_key/set_at/has_key_at/is_animated (model.rs:335-359), Project::add_marker/add_clip_marker/marker_mut/split_at/expand_links/clip/clip_mut/asset (model.rs:3176-4316; add_clip_marker at 4309 clamps local_t to [0, c.duration] — clip-local time; split_at at 3383 checks `t` via c.contains(t) — timeline time). No new Clip/Track/Project serde field, no Project::VERSION bump.
- Deliberately NOT adding Clip.audio_role here even though the source designs mention it: that field belongs to the sibling audio-dsp-automation workstream. Duck's music/dialogue targets are picked explicitly (UI dropdown or explicit MCP args), not role-inferred — avoids a same-wave cross-workstream field dependency. See ponytail_notes.
- App::fire_hook itself is not this workstream's to build: it is the wave-0b (registries-schema-hooks) pre-seeded stub, made real by command-palette (also wave 1). This workstream only adds call sites; if command-palette's real implementation lands after this branch, fire_hook silently no-ops until it does (documented ceiling, not a bug).

## Engine changes

- src/engine/mod.rs: insert `pub mod analysis;` before `pub mod autocut;` (alphabetical, line 3) — the only shared-file edit outside the pre-seeded registries; one line, alphabetical-insert convention keeps it a non-conflicting diff.
- src/engine/analysis.rs (new): pure functions over media::waveform::Peaks (100 buckets/s) — no decoding, no App/UI types, no fire_hook call (kept pure; fire_hook lives at the App-layer call sites in audio_actions.rs/tools_audio.rs/autocut_ui.rs). Mirrors autocut.rs's existing style (pure fns + #[cfg(test)] mod at the bottom).
- src/engine/analysis.rs adds to_clip_local(t_src: f64, c: &Clip) -> f64 = (t_src - c.src_in) / c.speed and to_timeline_t(t_src: f64, c: &Clip) -> f64 = c.start + to_clip_local(t_src, c), the exact conversion autocut::to_timeline (verified at autocut.rs:78-91) already performs for silence ranges; onsets()/scene_cuts() return raw source-time values (same space loud_segments uses, verified at autocut.rs:29) and every call site that turns those into a Marker or a split point must pass through one of these two helpers first — root-caused once here instead of patched per call site.
- src/engine/autocut.rs: NO functional change. loud_segments/to_timeline are already pure and pub — owned here only for exclusivity (a sibling workstream must not also touch it this wave).
- duck()/normalize()/match_loudness() are free fns taking `project: &mut Project` (not new `impl Project` methods) — keeps every_edit_op_has_a_tool (which only scans src/model/ops/*.rs for `pub fn NAME(&mut self`) from ever seeing them; MCP/Luau parity comes from the explicit ToolDef rows instead, not that scanner.
- scene_cuts() spawns ffmpeg synchronously (ffpipe::command pattern from media/ffpipe.rs:77); the stderr `pts_time:` parser is split into its own pure fn so it's unit-testable without spawning a process; parsed times are source-seconds and go through to_clip_local/to_timeline_t exactly like onsets().

## UI changes

- Auto-cut pane (autocut_ui.rs): existing silence section gains per-segment checkboxes + 'Mark instead' button (genuine (start,end) range markers via mark_ranges) alongside Split only/Apply/Clear.
- New 'Scene cuts' section: threshold slider (0..1), Detect/Split/Mark instead buttons, operates on the selected video clip's source file; Mark instead adds one point marker (duration 0) per cut, not a range.
- New 'Beats' section: Detect Beats button (adds clip markers, converted to clip-local time via to_clip_local) + BPM readout, Split at Beats button (converted to timeline time via to_timeline_t before Project::split_at); targets audio_targets(selection) (existing helper, reused as-is).
- New 'Loudness' section: per-selected-clip peak/RMS readout, Normalize to Peak/RMS buttons, Match Loudness button (enabled at >=2 clips).
- New 'Duck' section: two-bucket picker (Music clip / Dialogue clips from current selection), depth_db/ramp_ms DragValues seeded from Settings, Apply Ducking button.
- No new Pane, no new Glyph, no new window — everything lives inside the existing Auto-cut pane tab.

## New types and functions

- `pub struct Levels { pub peak_db: f32, pub rms_db: f32 }
pub fn levels(p: &Peaks, a: f64, b: f64) -> Levels` — src/engine/analysis.rs: dBFS peak + bucket-approximated RMS over a source-time range; ponytail-documented as not a true LUFS meter.
- `pub fn onsets(p: &Peaks, a: f64, b: f64, refractory_s: f64, sensitivity: f32) -> Vec<f64>` — src/engine/analysis.rs: Local-max-over-rolling-mean*sensitivity onset picker with a refractory window, returned in SOURCE seconds (same space as autocut::loud_segments) — callers must convert via to_clip_local/to_timeline_t before writing a marker or split. sensitivity = Settings.beat_thr by default.
- `pub fn bpm(onsets: &[f64]) -> Option<f64>` — src/engine/analysis.rs: Autocorrelation of inter-onset intervals in [60,180) BPM; None if fewer than 4 onsets or no stable peak.
- `pub fn to_clip_local(t_src: f64, c: &Clip) -> f64
pub fn to_timeline_t(t_src: f64, c: &Clip) -> f64` — src/engine/analysis.rs: Convert a source-seconds time (as returned by onsets()/scene_cuts()) into clip-local time (for add_clip_marker, which clamps to [0, c.duration]) or timeline time (for split_at, which checks c.contains(t)); (t_src - c.src_in) / c.speed [+ c.start], identical math to autocut::to_timeline (verified autocut.rs:90-91). One shared helper, not duplicated per call site.
- `pub fn xcorr_offset(a: &Peaks, b: &Peaks, max_lag: f64) -> Option<f64>` — src/engine/analysis.rs: Best lag aligning b to a; envelopes downsampled to 25/s before the O(n*m) search, max_lag hard-capped at 120s.
- `pub enum NormMode { Peak, Rms }
pub fn normalize(project: &mut Project, ids: &[Id], target_dbfs: f64, mode: NormMode, peaks_of: &mut dyn FnMut(&Project, Id) -> Option<Arc<Peaks>>) -> usize` — src/engine/analysis.rs: Per-clip constant-gain scale to target_dbfs via Animated::set_at(0.0, gain); skips already-animated clips and clips with no peaks yet; returns clips changed.
- `pub fn match_loudness(project: &mut Project, ids: &[Id], peaks_of: &mut dyn FnMut(&Project, Id) -> Option<Arc<Peaks>>) -> usize` — src/engine/analysis.rs: Computes the average RMS dBFS across ids, then scales each clip's gain toward it (same skip rules as normalize).
- `pub fn duck(project: &mut Project, music: Id, dialogue: &[Id], depth_db: f64, ramp_s: f64, peaks_of: &mut dyn FnMut(&Project, Id) -> Option<Arc<Peaks>>) -> usize` — src/engine/analysis.rs: Finds dialogue loud segments via autocut::loud_segments+to_timeline(keep_quiet=true) mapped into the music clip's local time, merges overlapping ramp windows, seeds/upserts Animated keys (base volume outside windows, base*10^(depth_db/20) inside, ramp_s either side) via toggle_key+set_at. Returns keys written.
- `fn parse_showinfo_pts(stderr: &str) -> Vec<f64>
pub fn scene_cuts(path: &std::path::Path, thr: f32) -> Result<Vec<f64>, String>` — src/engine/analysis.rs: Spawns ffmpeg -vf select='gt(scene,thr)',showinfo -f null -, parses `pts_time:` lines from stderr via the pure, independently-tested parser; returned times are source-seconds, converted by the caller via to_clip_local/to_timeline_t.
- `fn mark_ranges(app: &mut App, ranges: &[(f64,f64)], name_prefix: &str) -> Vec<Id>` — src/ui/autocut_ui.rs: 'Mark instead' for genuine (start,end) ranges (silence auto-cut only): one range Marker per (start,end) via add_marker+marker_mut (subtitles_ui.rs:515-517 pattern), no clip mutation; calls app.fire_hook("marker_added", payload) once per marker created. Takes &mut App (not &mut Project) precisely for that hook call. Scene cuts are point events and use add_clip_marker directly instead, not this helper, but fire the same event.
- `pub fn act(app: &mut App, a: Action) -> bool` — src/ui/app/audio_actions.rs: ACT_HANDLERS row: handles DetectBeats/SplitAtBeats/AutoDuck/Normalize/MatchLoudness, false for everything else (falls through); DetectBeats/SplitAtBeats fire marker_added per marker.

## Actions and hotkeys

| Action id | Label | Chord | Note |
|---|---|---|---|
| detect_beats | Detect Beats → Markers |  | Unbound (skeleton keymap). Runs analysis::onsets+bpm on audio_targets(selection); adds a clip marker at each onset via add_clip_marker(clip, to_clip_local(t_src, clip), ...) and fires marker_added per marker — 'Mark instead' is the only mode (no destructive variant). |
| split_at_beats | Split at Beats |  | Unbound. Same onset detection, then Project::split_at(to_timeline_t(t_src, clip), ...) per beat restricted to expand_links(selection) — existing pub fn, no model change; fires marker_added is NOT applicable here (split creates no marker). |
| auto_duck | Duck Music under Dialogue |  | Unbound. Uses the Duck section state in the Auto-cut pane (needs a music pick + >=1 dialogue pick already made); writes Animated keys on Clip.volume via analysis::duck. Note: this Action is declared by audio-analysis; audio-dsp-automation's UI dispatches it and must depend_on this workstream. |
| normalize | Normalize Selection |  | Unbound. analysis::normalize(project, selection, Settings default target -1 dBFS, Peak); skips already-animated clips. Same cross-workstream dependency note as auto_duck applies. |
| match_loudness | Match Loudness across Selection |  | Unbound. analysis::match_loudness(project, selection); needs >=2 clips. |

## Persisted fields

**Settings:**

- pub duck_depth_db: f32 (default -12.0) — default ducking depth in dB, editable per-call in the Duck section/tool args
- pub duck_ramp_ms: u32 (default 200) — default fade length either side of a duck window
- pub beat_thr: f32 (default 1.6) — onset sensitivity: local-mean multiplier an envelope bucket must exceed to count as a beat

**Project (.sedit):**

- (none)

## MCP tools (required — every capability must be scriptable)

| Tool | Kind | Args | Description | Maps to |
|---|---|---|---|---|
| audio.analyze | read | clip_ids:array:false:defaults to selection | Peak/RMS dBFS (from Peaks) and onset count/BPM per clip; 'waveform still computing' if peaks aren't ready yet. | engine::analysis::levels + onsets/bpm |
| audio.beats | mutate | clip_ids:array:false:; refractory_s:number:false:default 0.25; as_markers:boolean:false:default false; split:boolean:false:default false | Detect beats on audio clip(s). With neither flag: pure detection, returns onset times (source secs) + BPM, no mutation. as_markers converts each onset to clip-local time (to_clip_local) and adds a clip marker, firing marker_added per marker; split converts to timeline time (to_timeline_t) and cuts at each beat (Project::split_at, existing pub fn, restricted to expand_links(ids)). | engine::analysis::onsets/bpm/to_clip_local/to_timeline_t + Project::add_clip_marker/split_at + App::fire_hook |
| audio.duck | mutate | music_id:integer:true:; dialogue_ids:array:true:; depth_db:number:false:default Settings.duck_depth_db; ramp_s:number:false:default Settings.duck_ramp_ms/1000; dry_run:boolean:false:default false | Duck the music clip's volume under every loud (speech) window found on the dialogue clips' peaks, with a ramp. dry_run returns the computed windows/key count without writing. One undo (unchanged JSON when dry_run or no speech found → no undo pushed). | engine::analysis::duck (writes Clip.volume Animated keys via existing pub Animated::toggle_key/set_at) |
| audio.normalize | mutate | clip_ids:array:false:defaults to selection; target_dbfs:number:false:default -1.0; mode:string:false:peak\|rms, default peak; dry_run:boolean:false:default false | Set each clip's constant gain so its own peak/RMS reaches target_dbfs. Clips with keyframed volume are skipped (reported, not silently ignored). | engine::analysis::normalize |
| audio.match_loudness | mutate | clip_ids:array:true:>=2 clips; dry_run:boolean:false:default false | Scale every listed clip's gain toward the selection's average (bucket-)RMS. | engine::analysis::match_loudness |
| audio.sync_offset | read | clip_a:integer:true:; clip_b:integer:true:; max_lag_s:number:false:default 60, hard cap 120 | Best lag (seconds) that aligns clip_b to clip_a via cross-correlation of their waveform envelopes. For multicam sync; the angle-grid UI is a later (pro-monitor) workstream. | engine::analysis::xcorr_offset |
| autocut.detect | read | clip_ids:array:false:defaults to audio_targets(selection); threshold_db:number:false:; min_silence:number:false:; min_speech:number:false:; padding:number:false:; keep_quiet:boolean:false: | Silence-detection preview: returns cut times + kept/removed ranges per clip without mutating the project (never applies). | engine::autocut::loud_segments + to_timeline (already pub/pure, verified autocut.rs:29,78) |
| autocut.mark | mutate | same as autocut.detect | 'Mark instead' of Apply: adds a project range marker (start,end) per detected segment (the one Apply would have cut/removed), leaving every clip untouched, and fires marker_added per marker. | autocut_ui::mark_ranges -> Project::add_marker + marker_mut (existing pub fns, subtitles_ui.rs:515-517 pattern) + App::fire_hook |
| media.scene_cuts | mutate | clip_id:integer:false:defaults to selection's first video clip; threshold:number:false:default 0.3; split:boolean:false:default false; as_markers:boolean:false:default false | ffmpeg select='gt(scene,T)' shot-change detection over the clip's source. Neither flag: pure detection, returns cut times (source secs). as_markers converts each cut to clip-local time and adds one point marker (duration 0) per cut via add_clip_marker directly — a scene change is an instant, not a range, so this does NOT go through mark_ranges, but still fires marker_added per marker. split converts to timeline time and cuts the same way audio.beats does. Synchronous (blocks the UI thread for the ffmpeg pass) — documented ceiling, see ponytail_notes. | engine::analysis::scene_cuts/to_clip_local/to_timeline_t + Project::add_clip_marker/split_at + App::fire_hook |

**Luau:** No new Luau surface to hand-write: editor.tool(name, args) and editor.tools() already flatten every TOOL_TABLES row generically (wave-0b mcp_parity mechanism), so the 9 ToolDef rows in tools_audio.rs are reachable from Luau the moment they're registered. This workstream additionally OWNS the `marker_added` @on event (audit-assigned): every marker it creates (mark_ranges, beat/scene-cut markers) calls App::fire_hook("marker_added", {marker_id, clip_id, t}), so a `-- @on marker_added` script (implemented by command-palette's wave-1 runtime) fires on detection results, not just manual marker adds.

## Tests

| Test | File | Asserts |
|---|---|---|
| onsets_finds_synthetic_clicks | src/engine/analysis.rs | Peaks with amplitude spikes at 0.5/1.0/1.5/2.0s on a quiet bed -> onsets() returns 4 times each within 30ms of truth. |
| bpm_of_evenly_spaced_onsets_is_120 | src/engine/analysis.rs | onsets 0.5s apart -> bpm() in [118,122]; fewer than 4 onsets -> None. |
| levels_of_known_amplitude | src/engine/analysis.rs | Constant 0.5 amplitude peaks -> peak_db within 0.1 of 20*log10(0.5); rms_db close to peak_db for a constant signal. |
| clip_time_conversion_matches_to_timeline_math_for_trimmed_and_retimed_clips | src/engine/analysis.rs | For a Clip with src_in=1.0, speed=2.0, start=5.0: to_clip_local(3.0, clip) == 1.0 and to_timeline_t(3.0, clip) == 6.0, matching autocut::to_timeline's (start + (t - src_in)/speed) formula; regression test for detect_beats/split_at_beats/scene_cuts landing at the wrong position on any trimmed or retimed clip. |
| xcorr_offset_recovers_known_lag | src/engine/analysis.rs | b = a shifted by 2.3s -> xcorr_offset(a,b,5.0) within 0.08s of 2.3; empty peaks -> None; lag beyond max_lag not returned. |
| normalize_peak_sets_gain_and_skips_animated_clips | src/engine/analysis.rs | Clip with known peak -> volume.value matches target_dbfs analytically; a second clip with an existing keyframe is untouched and excluded from the returned count. |
| match_loudness_converges_selection_toward_average | src/engine/analysis.rs | Two clips at different RMS both move toward the pre-computed average within tolerance; a clip missing peaks is skipped, not panicked on. |
| duck_writes_keys_at_segment_edges_with_ramp | src/engine/analysis.rs | One dialogue loud window [2,4]s over a 10s music clip (initial volume 0.8) -> is_animated() true; keys at ~0, ~1.8, ~2.0, ~4.0, ~4.2, ~10 with values alternating base/ducked; base value preserved (not hardcoded 1.0). |
| duck_writes_keys_with_overlapping_windows_merged | src/engine/analysis.rs | Two dialogue segments whose ramps overlap produce one continuous ducked span, no out-of-order or duplicate key times (Animated.keys stays sorted/unique). |
| parse_showinfo_pts_extracts_times | src/engine/analysis.rs | Canned ffmpeg showinfo stderr text with several `pts_time:N.NN` lines -> matching Vec<f64>, ignoring unrelated lines; empty/garbage input -> empty vec (no ffmpeg spawned in this test). |
| mark_instead_writes_range_markers_without_mutating_clips | src/ui/autocut_ui.rs | mark_ranges() adds project.markers.len() == ranges.len() new markers with correct t/duration; clip count and every clip's JSON are byte-identical before/after. |
| mark_instead_fires_marker_added_once_per_marker | src/ui/autocut_ui.rs | mark_ranges() over 3 ranges triggers App's fire_hook call counter exactly 3 times with event=="marker_added" (uses the same test seam command-palette's own fire_hook tests use, e.g. a test-only hook recorder on App). |
| per_segment_toggle_excludes_unchecked_ranges_from_apply | src/ui/autocut_ui.rs | Detection.included with one false entry -> Apply removes only the included ranges (project.auto_cut called with the filtered subset). |
| show_smoke_new_sections_no_idle_repaint | src/ui/autocut_ui.rs | assert_no_idle_repaint over the extended pane (Scene cuts/Beats/Loudness/Duck sections closed and open) across 30 frames with no input. |
| tools_audio_args_parse_and_round_trip | src/ui/app/tools_audio.rs | every_arg_spec_parses-style check: each of the 9 ToolDefs' arg docs parse; a garbage-typed call to each Mutate row leaves project JSON unchanged (mutate_rows_roll_back_on_error house style). |
| scene_cuts_as_markers_adds_point_markers_not_ranges | src/ui/app/tools_audio.rs | media.scene_cuts with as_markers=true adds Marker entries with duration == 0.0 (via add_clip_marker), not via mark_ranges; count == number of detected cuts; also fires marker_added that many times. |

## Verification checklist

- [ ] cargo test (all new + existing ~657 tests green)
- [ ] cargo run -- --selftest
- [ ] cargo run -- <clip-with-speech> --screenshot autocut.ppm on the Auto-cut pane with Beats/Loudness/Duck sections open; convert+view PNG
- [ ] Manual: Detect Beats on a music clip -> markers appear and are snap-eligible; Split at Beats cuts only that clip
- [ ] Manual: on a clip trimmed mid-source (src_in > 0) or retimed (speed != 1), Detect Beats places markers at the visually-correct beat, not shifted by src_in/speed
- [ ] Manual: Duck a dialogue clip under a music clip -> volume line on the music clip shows the keyframes, playback audibly ducks, Ctrl+Z removes it in one step
- [ ] Manual MCP: call each of the 9 tools once via /se-coedit or tools/list; confirm autocut.detect/media.scene_cuts (no flags) leave project JSON unchanged, and media.scene_cuts as_markers=true adds duration=0 point markers
- [ ] Manual/Luau: register a `-- @on marker_added` test script (once command-palette's runtime lands) and confirm it fires when Detect Beats / autocut.mark / media.scene_cuts as_markers add markers
- [ ] scripts/size.ps1 -Note audio-analysis; delta within ~90 KB or PR body carries a `size:` justification line

## Acceptance criteria

- [ ] cargo test passes with all new tests (analysis.rs, autocut_ui.rs, tools_audio.rs) plus the existing 657+ suite
- [ ] Detect Beats on a percussive audio clip produces markers within ~50ms of the true beat and a BPM readout within +-2, correctly positioned via to_clip_local even when the clip is trimmed (src_in > 0) or retimed (speed != 1)
- [ ] Split at Beats cuts only the target clip(s) (+ linked video) at detected times converted via to_timeline_t, not the whole timeline
- [ ] Auto-duck writes >=4 volume keyframes per speech window on the music clip, playable and re-editable on the timeline volume line, one undo step
- [ ] Normalize/Match Loudness change only clips with non-keyframed volume; animated clips are skipped and reported, one undo step
- [ ] Silence auto-cut's 'Mark instead' produces (start,end) range markers via mark_ranges with zero clip mutation; scene-cut's 'Mark instead' produces one point marker (duration 0) per cut via add_clip_marker directly, also with zero clip mutation — both verified by a project-JSON diff of only `markers`
- [ ] media.scene_cuts, audio.duck, audio.normalize, audio.match_loudness, audio.beats, audio.sync_offset, autocut.detect, autocut.mark, audio.analyze all appear in tools/list and are callable from Luau via editor.tool with no extra code
- [ ] DetectBeats/SplitAtBeats/AutoDuck/Normalize/MatchLoudness resolve through the command palette and toast a reason when disabled (no audio clip selected / peaks still computing)
- [ ] Every marker created by this workstream (mark_ranges, beat markers, scene-cut point markers) fires App::fire_hook("marker_added", ...) exactly once, verified by mark_instead_fires_marker_added_once_per_marker and scene_cuts_as_markers_adds_point_markers_not_ranges
- [ ] Release binary size delta stays within the ~90 KB budget (scripts/size.ps1 -Note audio-analysis)
- [ ] No new idle repaint from the extended Auto-cut pane (assert_no_idle_repaint)

## Risks

| Risk | Mitigation |
|---|---|
| Energy-envelope onset picking is noisy on non-percussive/ambient music, producing junk beat markers. | Settings.beat_thr slider exposed in the Beats section; result is always markers (never a destructive cut) so a bad detection is a few Ctrl+Z-free deletions, not lost work. |
| Cross-correlation is O(n*m) on long clips (multicam sync of two 30-min interviews). | Downsample both envelopes to 25/s before correlating and hard-cap max_lag at 120s; documented in the tool desc. |
| Ducking with tightly-spaced or long dialogue segments can produce overlapping ramp windows -> out-of-order or duplicate keyframe times. | Merge windows whose ramps overlap before writing (same merge-adjacent pattern as autocut::to_timeline); unit test duck_writes_keys_with_overlapping_windows_merged covers it. |
| media.scene_cuts blocks the UI thread for the ffmpeg pass on long/4K sources. | Documented ponytail ceiling; recommend running on a proxy; upgrade path is a generic job channel (not this workstream's scope). |
| Detect Beats/Split at Beats/media.scene_cuts write onset or ffmpeg cut times (source seconds) straight into add_clip_marker/split_at without converting for a clip's src_in/speed, silently misplacing markers or cuts on any trimmed or retimed clip. | Centralized once in analysis::to_clip_local/to_timeline_t (same math as autocut::to_timeline), called at every marker/split call site in audio_actions.rs and tools_audio.rs; covered by clip_time_conversion_matches_to_timeline_math_for_trimmed_and_retimed_clips with a non-trivial src_in/speed clip. |
| hotkeys.rs / settings.rs / src/ui/app/mod.rs are edited by every wave-1 workstream in the same PR wave. | Strictly additive edits inside this workstream's own pre-seeded `// ---- ws:audio-analysis ----` marker section; rebase (not merge) onto main immediately before opening the PR. |
| Normalize/match_loudness silently doing nothing on a clip whose peaks aren't computed yet (WaveformCache still working). | peaks_of returning None is treated as 'skip, not error'; the pane shows 'analysing…' (existing pattern from autocut_ui.rs:209-210) and the MCP tool's return JSON lists which clip ids were skipped and why. |
| audio-dsp-automation (sibling wave-1 workstream) dispatches Action::AutoDuck/Action::Normalize from its inspector_audio.rs, but those Action variants are declared by THIS workstream's hotkeys.rs edit, not any wave-0b pre-seeded stub. If audio-dsp-automation's worktree branches/builds before this PR lands and its plan's depends_on omits audio-analysis, its branch will not compile. | Recorded here for the coordinator: audio-dsp-automation's plan must add `depends_on: ["audio-analysis"]` (both stay wave-1, audio-dsp-automation just rebases after this PR merges). This plan's own files are unaffected; fix applies to the sibling plan document. |

## Suggested implementation order

1. 1. src/engine/mod.rs + src/engine/analysis.rs: pure fns (levels, onsets, bpm, xcorr_offset, normalize, match_loudness, duck, scene_cuts+parser, to_clip_local, to_timeline_t) with their unit tests — zero UI/App dependency, compiles and tests standalone.
2. 2. src/settings.rs: duck_depth_db/duck_ramp_ms/beat_thr fields + defaults.
3. 3. src/hotkeys.rs: 5 unbound Action rows.
4. 4. src/ui/autocut_ui.rs: per-segment `included` toggle + mark_ranges(app: &mut App, ...) (fires marker_added per marker) on the existing silence flow first (smallest diff, reuses existing tests as a regression net; ranges only), then the 3 new sections (Scene cuts, Beats, Loudness, Duck) each behind its own State struct — Scene cuts' Mark instead calls add_clip_marker directly per cut (also firing marker_added), not mark_ranges.
5. 5. src/ui/app/audio_actions.rs: wire the 5 hotkey Actions to the exact same analysis:: calls the new UI sections use (share a small module-private fn per verb so UI and Action never drift); beat markers/splits go through to_clip_local/to_timeline_t before touching the model, and fire marker_added at the same shared inner fn so the event fires exactly once regardless of entry point (UI button vs hotkey vs MCP).
6. 6. src/ui/app/tools_audio.rs: 9 ToolDef rows wrapping the same verbs, using Args/ids_or_selection; audio.beats and media.scene_cuts route every source-time value through the same to_clip_local/to_timeline_t calls and the same marker_added-firing inner fn as step 5, never re-deriving the math or the hook call.
7. 7. src/ui/app/mod.rs: fill the pre-seeded ws:audio-analysis registry lines (mod decls, TOOL_TABLES, ACT_HANDLERS).
8. 8. cargo test; cargo run -- --selftest; cargo run -- <clip> --screenshot to eyeball the new pane sections; scripts/size.ps1 -Note audio-analysis; rebase onto main before opening the PR; flag to the coordinator that audio-dsp-automation's depends_on needs audio-analysis added.

## Deliberate simplifications (`// ponytail:`)

- RMS/BPM/scene-detection are approximations over the 100/s min-max envelope or a single-pass energy heuristic, not spectral-flux beat tracking or broadcast LUFS — every detector ships a threshold/sensitivity control and a non-destructive 'Mark instead' path so a wrong guess costs nothing. Upgrade path: spectral-flux onsets, K-weighted LUFS (already the audio-dsp-automation workstream's Lufs struct, fed from live audio, not Peaks).
- media.scene_cuts runs ffmpeg synchronously on the calling thread (blocks the UI while decoding). Fine for typical clip lengths; upgrade path is the generic JSON-bearing Job channel once mcp_exec.rs supports more than ToolOutcome::Job(Progress,PathBuf) (file-only jobs today) — not this workstream's file to touch.
- Duck's music/dialogue targets are picked explicitly (UI two-bucket picker / MCP ids), not via Clip.audio_role, to avoid a same-wave dependency on audio-dsp-automation's field. Once that field lands, autocut_ui's pickers can default from it — additive, not a breaking change.
- match_loudness targets the selection's own average RMS (single pass), not a chosen reference clip or iterative EBU R128 program loudness — documented, cheap, good enough for 'make these roughly even'.
- No dry_run flag on autocut.detect/audio.beats/media.scene_cuts: the flag would be redundant since detection-only IS the default (mutation is opt-in via as_markers/split). dry_run is kept only where the tool mutates by default in spirit (audio.duck/normalize/match_loudness).
- to_clip_local/to_timeline_t duplicate (in spirit) autocut::to_timeline's src_in/speed formula rather than reusing that fn directly, because to_timeline works on (start,end) ranges with a keep_quiet toggle and merge-adjacent logic that a single onset/cut instant doesn't need — two one-line pub fns beat threading a beat-detector's timestamps through a ranges-shaped API. Upgrade path: extract a shared private time-map fn in autocut.rs if a third caller needs the same math.
- fire_hook payload shape ({marker_id, clip_id, t}) is a minimal placeholder matching what a marker-reacting script plausibly needs; if command-palette's real @on runtime settles on a different envelope shape for other events, this call site adapts to match it, not the other way around — this workstream doesn't own the envelope contract, only its own event's occurrence.

## Review trail

- Finding 1 (constraint, CONFIRMED): detect_beats/split_at_beats and media.scene_cuts were calling add_clip_marker/split_at directly with raw source-time values from onsets()/scene_cuts(), the same absolute source-time space autocut::loud_segments uses (verified at autocut.rs:29) — but add_clip_marker (model.rs:4309) takes clip-LOCAL time and split_at (model.rs:3383) takes TIMELINE time, exactly the conversion autocut::to_timeline (verified at autocut.rs:78-91) already performs and duck() already reuses. Fixed by adding pub fn to_clip_local()/to_timeline_t() to analysis.rs (same (t-src_in)/speed[+start] math) and routing every beat/scene-cut marker-or-split call site (audio_actions.rs, tools_audio.rs) through them; added test clip_time_conversion_matches_to_timeline_math_for_trimmed_and_retimed_clips with non-trivial src_in/speed, and called this out explicitly in acceptance_criteria, mcp_tools (audio.beats, media.scene_cuts), actions_and_hotkeys, implementation_order, and risks.
- Finding 2 (other, CONFIRMED): the plan's acceptance criterion said scene-cut Mark-instead produces 'range markers' like silence auto-cut, but mark_ranges() only accepts (f64,f64) ranges and a scene cut is a point event (Marker.duration=0 per model.rs:1993 doc), not a genuine range. Fixed by specifying scene-cut Mark-instead adds a point marker (duration 0) per cut via add_clip_marker directly, reserving mark_ranges for autocut's genuine (start,end) silence ranges only; updated media.scene_cuts's mcp_tools desc, the Scene cuts UI note, mark_ranges' purpose doc, the acceptance criterion wording, and added test scene_cuts_as_markers_adds_point_markers_not_ranges.
- Audit fix A (major, applied): mcp_parity's narrative promised six `@on` Luau hook events but only export_done had a committed fire_hook call site anywhere across the plans; grep of src/ confirmed zero fire_hook call sites exist yet, so the gap was real. Per the audit's assignment ('trim-model or audio-analysis fires marker_added at their marker-creation call sites'), this plan now owns marker_added: mark_ranges() takes &mut App (was &mut Project) and calls app.fire_hook("marker_added", ...) once per marker, and every beat/scene-cut marker call site (audio_actions.rs, tools_audio.rs) does the same via one shared inner fn. Added mark_instead_fires_marker_added_once_per_marker and extended scene_cuts_as_markers_adds_point_markers_not_ranges to also assert the fire count; updated scope_in, files, new_types_and_fns, mcp_tools descriptions, luau, verification, acceptance_criteria, implementation_order, and ponytail_notes accordingly. selection_changed/import/project_open/project_save remain unowned by this plan — out of scope here, flagged in scope_out as belonging to layout-modes-onboarding/forgiveness/media-library respectively per the audit's suggested assignment (not this workstream's file to edit).
- Audit fix B (major, applied as a coordination note): audio-dsp-automation's inspector_audio.rs dispatches Action::AutoDuck/Action::Normalize, which are declared by THIS workstream's hotkeys.rs edit under `// ---- ws:audio-analysis ----`, not by any wave-0b pre-seeded stub — confirmed by re-reading this plan's own hotkeys.rs diff description, which indeed adds those two rows fresh. audio-dsp-automation's depends_on (in its own, separate plan document) omitting audio-analysis is a real cross-plan gap per the audit, but the fix (adding 'audio-analysis' to that sibling plan's depends_on list) is a change to audio-dsp-automation's file, not this one. Applied here as: an explicit coordination note in files/hotkeys.rs, a flag in actions_and_hotkeys for auto_duck/normalize, a new risk entry naming the required sibling-plan fix, and a reminder in implementation_order step 8 to notify the coordinator — so the dependency is documented at its source (the workstream that creates the shared symbol) even though the authoritative fix lands in the other plan.
- Everything else (thesis-level scope, MCP tool list beyond the two description edits above, Settings fields, size budget 90 KB unchanged, most tests, ponytail_notes beyond the new fire_hook entry) preserved unchanged from the prior revision.
