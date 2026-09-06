# Transcript in the model: editable word-range cuts, karaoke captions, filler removal, whisper/TTS one-click

**Workstream:** `transcript-captions` · **Issue:** [#29](https://github.com/KashTheKing/simple-editor/issues/29) · **Wave:** 2 · **Branch/worktree:** `feat/transcript-captions` → `../simple-editor-wt/transcript-captions` · **Depends on:** trim-model · **~1400 new lines · Δ exe ≈ +112 KB**

Read [agents.md](../../../agents.md), [goals.md](../../../goals.md), [notes.md](../../../notes.md) and the master plan [README.md](../README.md) (registry protocol, keymap, modifier table) before starting.

## Summary

Persist whisper word timings into Project.transcripts (currently transient, dies on reopen: TranscribeState.raw_words subtitles_ui.rs:60/753). Add one cut_word_ranges(clip, ranges) generalizing the existing cut_dups body (subtitles_ui.rs:543-561: auto_cut model.rs:4060 + ripple_time-based retain_mut over cues/markers/words) so double-take, filler removal, the new Transcript section and MCP share one cut path. Add a collapsible Transcript section to the Subtitles pane: selectable word labels, drag-range->Delete ripple-cuts, click->seek, search-with-jump across every transcribed clip. Filler-word list with Mark-instead-first (mirrors mark_dups). Karaoke/highlight/pop/typewriter caption animation computed in engine::subtitles::cue_layer_at (the wave-0b extraction of compose.rs:226-252 / playback.rs:1015-1024) driven by the single active Project.transcripts entry (v1 ceiling: one active transcript drives karaoke, not per-Cue clip lookup — see ponytail_notes) plus Project.subtitle_anim. One-click "Get captions" wraps transcribe::download_model (transcribe.rs:113) already-progress-bar'd. transcribe.run and tracking.run (engine/tracking.rs TrackJob::start:101, currently UI-only via tracking_ui.rs:146) become ToolKind::Job MCP tools. TTS via powershell System.Speech shell-out, imported as a linked asset. NOTE: Project.transcripts and Project.subtitle_anim do not exist in wave-0b's scope today (verified: grep of src/model.rs/engine/ui finds no Transcript type, no subtitle_anim, no transcripts field) — this workstream requests them from registries-schema-hooks rather than declaring them itself (see risks).

## Motivation

Closes gap-matrix rows 78, 93, 94, 95, 97, 99, 100 and architecture hook H11 (Transcript in the model + shared word-range cut). Serves goals.md "AI outputs remain editable text, never opaque" and the MCP-parity mandate. Reconciles infra/beginner/custom designs' converging key API (cut_word_ranges, Transcript{clip,words}, SubtitleAnim) onto the skeleton's narrower authoritative scope (no Titles pane, no scene-cut, no auto-reframe -- those are other workstreams).

## In scope

- Project.transcripts persistence (field requested from wave-0b registries-schema-hooks, see risks; this workstream fills/reads it once landed)
- cut_word_ranges(clip, ranges) in src/model/ops/subtitles.rs, shared by double-take, fillers, Transcript delete, MCP
- Transcript collapsible section in Subtitles pane (new src/ui/transcript_ui.rs)
- Filler-word/pause removal with editable list + Mark-instead-first
- One-click whisper model download button + transcribe.run/transcribe.install/tracking.run as ToolKind::Job
- Karaoke/highlight/pop/typewriter via SubtitleAnim consumed in engine::subtitles::cue_layer_at, driven by the single active transcript (v1 ceiling, no per-Cue clip attribution)
- Word search across every Project.transcripts entry with jump-to-seek
- TTS via engine/tts.rs powershell System.Speech shell-out, imported as a linked asset
- MCP tools: transcript.get/cut_words/remove_fillers/search, transcribe.run/install, tracking.run, subtitles.animation, tts.speak

## Out of scope

- Titles pane / Essential Graphics, template exposed params, caption style preset gallery (rows 76-77) - separate workstream
- Text animation motion presets / reveal-wave on TextStyle scalars->Animated (row 79) - inspector-gallery/text-titles territory; only SubtitleAnim is in scope here
- Scene-cut detection (row 90) - audio-analysis workstream
- Auto-cut/detections as markers with dry_run for SILENCE auto-cut (row 96) - audio-analysis workstream; this plan's Mark-instead is scoped to fillers only
- Auto reframe (row 98) - pro-monitor/audio-analysis territory
- Pane::Titles, Pane::Transcript as a NEW dockable pane - skeleton decision: Transcript stays a section inside the existing Subtitles pane
- Editing Track.locked/ripple field definitions or shift_time's own signature - owned by trim-model (wave 1 dependency); this workstream only calls what trim-model lands
- Declaring Cue.clip / any Cue schema change, or Project.transcripts/subtitle_anim type definitions - these are wave-0b registries-schema-hooks' exclusive files (src/model/subtitle.rs, src/model/project.rs); this workstream requests the fields, does not add them itself

## Files

| Op | Path | What |
|---|---|---|
| modify | src/model/ops/subtitles.rs | Add cut_word_ranges, transcript_hits (search), transcript upsert helper. File was created in wave-0a as a pure move of add_cue/cue_at/split_cue/cues_to_text_clips/sort_cues (model.rs:3720-3775). Depends on Project.transcripts existing (requested from wave-0b, see risks) -- do not add the type here. |
| modify | src/engine/transcribe.rs | Add filler_ranges(words,&[&str],pad)->Vec<(f64,f64)> beside dup_ranges (line 528), reusing normalize() (464) and the MERGE_GAP merge pattern (536-543). Add transcript_hits/search helper. Keep MODELS/download_model/exe()/parse_line/group_words/to_cues untouched. |
| modify | src/engine/subtitles.rs | Fill the wave-0b cue_layer_at(project,t)->Option<(String,TextStyle)> stub (behavior-identical extraction of compose.rs:226-252 today) with karaoke: find the active word by scanning the SINGLE most-recently-active Project.transcripts entry (v1 ceiling -- Cue has no clip field, project.subtitles is one flat Vec<Cue>, so there is no per-Cue transcript attribution; documented ponytail, not blocking on a Cue.clip field), and inject a TextSpan (model.rs:461) for Highlight/PopWord, or truncate text to the elapsed prefix for Typewriter. |
| create | src/engine/tts.rs | pub fn speak_to_wav(text:&str, voice:Option<&str>, out:&Path)->Arc<Progress>: spawn_job wrapping a powershell -Command Add-Type System.Speech ... SetOutputToWaveFile call. Same export::spawn_job/Progress pattern as transcribe::download_model. |
| modify | src/ui/subtitles_ui.rs | Add SubtitlesState.show_transcript: bool fold (mirrors show_style:114); wire transcript_ui::show(...) as a new collapsible section after transcribe_section; on a successful word-timed transcribe (line ~753) also upsert Project.transcripts for st.clip. Add a single 'Get captions' CTA above the model combo. Note: rfd::MessageDialog at import_dialog:828 is forgiveness workstream's territory (wave 1, lands before this wave) -- do not touch if already confirm::ask. |
| create | src/ui/transcript_ui.rs | fn show(ui,&mut TranscriptUiState,&mut Project,playhead:&mut f64,undo)->TranscriptResponse: selectable word Labels (drag-range select), Delete -> cut_word_ranges, click -> seek, a search TextEdit + Prev/Next jump over transcript_hits across every Project.transcripts entry, filler-word chip list (Settings.filler_words) with 'Mark fillers' (mirrors mark_dups subtitles_ui.rs:510) then 'Remove fillers'. |
| create | src/ui/app/tools_text.rs | pub const TOOLS: &[ToolDef]: transcript.get, transcript.cut_words, transcript.remove_fillers, transcript.search (Read/Mutate), transcribe.run, transcribe.install, tracking.run (Job), subtitles.animation (Mutate), tts.speak (Job). Free fns beside the table per the god_file_split convention. |
| modify | src/ui/tools.rs | Add Glyph::Transcript (ws:transcript-captions section) + its draw_glyph arm: three horizontal text-lines of decreasing width with the middle line's leading segment highlighted. Add to Glyph::ALL/name/from_name in this workstream's marker section only. |
| modify | src/hotkeys.rs | Add 3 rows in actions! under a ws:transcript-captions section: GetCaptions/RemoveFillers/ToggleTranscript, all sc()=>None (unbound, per skeleton keymap). |
| modify | src/settings.rs | Add filler_words: Vec<String> (default from a FILLER_WORDS const: um, uh, uhh, like, you know, i mean, sort of, kind of) and filler_pad_ms: u32 (default 120), both #[serde(default)]. |
| modify | src/ui/app/mod.rs | Fill this workstream's pre-seeded ws:transcript-captions lines: mod transcript_ui; one line in TOOL_TABLES (tools_text::TOOLS,); one in ACT_HANDLERS if GetCaptions/RemoveFillers/ToggleTranscript need App-level state beyond the pane. |

## Model changes

- cut_word_ranges(&mut self, clip: Id, ranges: &[(f64,f64)]) -> usize in ops/subtitles.rs: merges overlapping/adjacent ranges (MERGE_GAP-style), calls self.auto_cut(&[clip], cuts, ranges, ripple) where ripple comes from the clip's owning Track (trim-model's Track.ripple/locked flags once landed; refuse with 0 if the track is locked), then retain_mut's self.subtitles / self.markers / the owning Transcript's words through transcribe::ripple_time exactly like today's cut_dups (subtitles_ui.rs:543-561) but generalized to any clip, not just the last-transcribed one.
- transcript_hits(transcripts: &[Transcript], q: &str) -> Vec<(Id, usize, f64)> (clip id, word index, word start) -- substring match over normalize()'d words, case-insensitive, no phonetics.
- Transcript upsert: on a completed word-timed transcribe (subtitles_ui.rs raw_words branch), write/replace project.transcripts entry for that clip id so 'Regenerate cues' and later Transcript-pane edits use the same words without re-transcribing.
- engine::subtitles::cue_layer_at gains karaoke: for SubtitleAnim::Highlight/PopWord, find the active word by time t against the single active Project.transcripts entry (v1 ceiling, see ponytail_notes -- Cue carries no clip reference so there is no way to disambiguate which transcript owns a given Cue when multiple clips are transcribed) and inject a TextSpan over its char range; for Typewriter, truncate text to chars whose word.start <= t.

## Engine changes

- engine::transcribe::filler_ranges(words: &[(f64,f64,String)], fillers: &[&str], pad_ms: u32) -> Vec<(f64,f64)>: normalize() each word, match against fillers (case/punct-insensitive), pad by pad_ms/1000.0 each side, merge with the existing MERGE_GAP const.
- engine::transcribe: no change to MODELS/download_model/exe()/parse_line/group_words/to_cues/dup_ranges/ripple_time -- all reused as-is.
- engine::tts::speak_to_wav (new file): powershell -Command shell-out via std::process::Command, zero new crate. Import output WAV as an Asset (existing add_asset path) linked (Clip.link) to the source text clip.
- engine::tracking::TrackJob wrapped for MCP: tracking.run tool spawns an export::spawn_job whose body constructs TrackJob::start(...) and loop-polls .poll() to completion (channel-based, not Arc<Progress>-based -- bridge via prog.set() each iteration), then applies points via project.apply_path(clip, &points) when apply=true (model.rs:4555).

## UI changes

- Subtitles pane: new 'Transcript' CollapsingHeader (SubtitlesState.show_transcript, mirrors show_style/transcribe.open toggles at lines 114/46), selectable word run, search box, filler chip list.
- A single 'Get captions' button surfaced above the model combo in transcribe_section: same download_model/progress flow, friendlier entry point naming the ~75MB tiny.en default before the click.
- Word click -> *playhead = word.0 (same &mut playhead already threaded through subtitles_ui::show, line 148/150).
- Drag-select word range -> highlight -> Delete key or a 'Cut selected words' button -> cut_word_ranges, one undo (edit_start/once pattern already used throughout this file).
- Filler list: chips of Settings.filler_words (add/remove text field), 'Mark fillers' (adds range markers first, like mark_dups) then 'Remove fillers' enabled once marks exist.
- TTS: a small panel with a text box, voice combo (cached powershell voices query), Speak button -> engine::tts::speak_to_wav as a job with a spinner.

## New types and functions

- `pub fn cut_word_ranges(&mut self, clip: Id, ranges: &[(f64,f64)]) -> usize` — src/model/ops/subtitles.rs: Single cut path behind double-take, filler removal, Transcript-pane Delete and transcript.cut_words/transcript.remove_fillers MCP tools.
- `pub fn transcript_hits(transcripts: &[Transcript], q: &str) -> Vec<(Id, usize, f64)>` — src/model/ops/subtitles.rs: PhraseFind-lite word search feeding both the Transcript-pane search box and transcript.search.
- `pub fn filler_ranges(words: &[(f64,f64,String)], fillers: &[&str], pad_ms: u32) -> Vec<(f64,f64)>` — src/engine/transcribe.rs: Filler-word detection beside dup_ranges; feeds cut_word_ranges via Mark-instead.
- `pub fn speak_to_wav(text: &str, voice: Option<&str>, out: &std::path::Path) -> std::sync::Arc<crate::engine::export::Progress>` — src/engine/tts.rs: powershell System.Speech shell-out; result WAV imported as an asset linked to the text clip.
- `pub fn voices() -> Vec<String>` — src/engine/tts.rs: Cached powershell voices query for the voice combo, run once via a OnceLock like transcribe's settings().
- `pub fn cue_layer_at(project: &Project, t: f64) -> Option<(String, crate::model::TextStyle)>` — src/engine/subtitles.rs: Wave-0b extraction point (behavior-identical to compose.rs:226-252 today); this workstream adds the SubtitleAnim/karaoke branch driven by the single active transcript.
- `fn show(ui: &mut egui::Ui, st: &mut TranscriptUiState, project: &mut Project, playhead: &mut f64, undone: &mut bool, undo: &mut dyn FnMut(&Project)) -> TranscriptResponse` — src/ui/transcript_ui.rs: The Transcript section: selectable words, search+jump, filler chips, Mark/Remove buttons.

## Actions and hotkeys

| Action id | Label | Chord | Note |
|---|---|---|---|
| GetCaptions | Get Captions (download whisper) |  | Unbound per skeleton keymap. Button-only entry point; runs transcribe::download_model for the default/selected model. |
| RemoveFillers | Remove Filler Words |  | Unbound. Runs filler_ranges on the selected transcribed clip then cut_word_ranges, or removes existing marks if Mark-instead ran first. |
| ToggleTranscript | Show / Hide Transcript section |  | Unbound. Folds SubtitlesState.show_transcript; same shape as the existing show_style toggle (subtitles_ui.rs:114). Audit-fix 2: absent from the skeleton's top-level keymap table -- doc gap only, not a chord collision (row is unbound); fix appends this row to the skeleton keymap doc, no change here. |

## New glyphs

- Transcript

## Persisted fields

**Settings:**

- filler_words: Vec<String> (#[serde(default)], seeded from a FILLER_WORDS const: um, uh, uhh, like, you know, i mean, sort of, kind of)
- filler_pad_ms: u32 (#[serde(default)], default 120)

**Project (.sedit):**

- REQUESTED from wave-0b registries-schema-hooks (not this workstream's to declare -- src/model/subtitle.rs + src/model/project.rs are that workstream's owns_files): transcripts: Vec<Transcript> where Transcript{clip: Id, words: Vec<(f64,f64,String)>}, and subtitle_anim: SubtitleAnim (None\|Highlight([u8;4])\|PopWord\|Typewriter), both #[serde(default)]. This workstream's first commit is a coordination PR comment/issue against registries-schema-hooks, not a type edit.
- Cue.words is DROPPED from this plan (was declared-but-dead: Cue has no clip field per model.rs:2333-2338 and project.subtitles is one flat Vec<Cue>, so a per-Cue word list could never be attributed to the right transcript; cue_layer_at instead reads the single active Project.transcripts entry directly, see ponytail_notes). Audit-fix 1 independently confirms this: registries-schema-hooks' own wave-0 schema had added a Cue.words field for karaoke with no consumer anywhere in any plan; that field is being dropped from registries-schema-hooks' schema per the audit, which matches (and is now doubly justified by) this workstream never having used it.

## MCP tools (required — every capability must be scriptable)

| Tool | Kind | Args | Description | Maps to |
|---|---|---|---|---|
| transcript.get | read | clip_id:integer:false:omit for all clips | Word timings for one or all transcribed clips. | Project.transcripts lookup |
| transcript.cut_words | mutate | clip_id:integer:true:; ranges:array:false:[[start,end],...]; word_indices:array:false:alternative to ranges | Ripple-cut the given word ranges (or resolved from indices) and shift cues/markers/words along. | Project::cut_word_ranges |
| transcript.remove_fillers | mutate | clip_id:integer:true:; words:array:false:overrides Settings.filler_words; dry_run:boolean:false:default false, Mark instead | Detect and remove (or mark) filler words in a transcribed clip. | filler_ranges + cut_word_ranges, or add_marker per hit when dry_run |
| transcript.search | read | query:string:true: | Word hits across every transcribed clip with seek positions. | transcript_hits |
| transcribe.run | job | clip_id:integer:true:; model:string:false:MODELS name/file; words:boolean:false:default true for MCP callers; language:string:false: | Transcribe a clip's audio; on completion persists into Project.transcripts and generates cues. | engine::transcribe::start + Transcript upsert |
| transcribe.install | job | model:string:false:defaults to Settings.transcribe_model | Download the whisper model (size named before starting). | engine::transcribe::download_model |
| tracking.run | job | clip_id:integer:true:; cx:number:true:; cy:number:true:; hw:number:true:; hh:number:true:; search:number:false:; backward:boolean:false:; apply:boolean:false:default true, writes X/Y keyframes | NCC point-track a region of a clip; optionally writes the path onto the clip's X/Y. | engine::tracking::TrackJob::start + Project::apply_path |
| subtitles.animation | mutate | kind:string:true:none\|highlight\|pop\|typewriter; color:array:false:[r,g,b,a] for highlight | Set the project's caption animation mode. | Project.subtitle_anim |
| tts.speak | job | text:string:true:; voice:string:false:; clip_id:integer:false:target text clip to link the asset to; at:number:false:timeline seconds if no clip_id | Windows SAPI text-to-speech; imports the WAV as a linked asset. | engine::tts::speak_to_wav + add_asset |

**Luau:** Every tools_text ToolDef row is callable via editor.tool("transcript.cut_words", {...}) etc, auto-listed by editor.tools() through the TOOL_TABLES flatten -- no per-workstream Luau code needed. Job-kind tools (transcribe.run/install, tracking.run, tts.speak) poll like existing McpJob-backed tools; a script can call editor.tool("transcribe.run", {clip_id=id}) and poll job.done. No new -- @on hook consumers added here; a future @on transcript_ready is a ponytail note, not built.

## Tests

| Test | File | Asserts |
|---|---|---|
| transcripts_survive_json_round_trip | src/model/ops/subtitles.rs (or project.rs tests) | Project with a populated transcripts entry serializes and deserializes with identical words; an old .sedit with no transcripts key loads with an empty Vec (serde default). Gated on the wave-0b field landing first. |
| cut_word_ranges_shifts_cues_markers_words | src/model/ops/subtitles.rs | Given a clip with a Transcript, cues and a marker inside/after a cut range: cut_word_ranges removes the clip span and retimes surviving cues/markers/words identically to the existing cut_dups behavior. |
| filler_ranges_pads_and_merges | src/engine/transcribe.rs | A word list with 'um'/'like' hits returns padded ranges merged across MERGE_GAP; case/punctuation-insensitive; a custom fillers list overrides the default. |
| transcript_hits_finds_words_across_clips | src/model/ops/subtitles.rs | A query matching a word in one of two transcripts returns (clip_id, index, start) only for the matching clip; case-insensitive; no match returns empty. |
| karaoke_active_word_span_at_t | src/engine/subtitles.rs | cue_layer_at with SubtitleAnim::Highlight injects a TextSpan covering exactly the active word's char range at t (using the single active transcript); gaps between words inject nothing; PopWord/Typewriter each produce the documented style/text change; behavior identical to today's plain path when SubtitleAnim::None. |
| cue_layer_at_matches_old_compose_output_when_anim_none | src/engine/subtitles.rs | Wave-0b parity check: for SubtitleAnim::None the returned (text, style) is byte-identical to the pre-refactor compose.rs inline logic on a fixed fixture project. |
| tts_speak_builds_expected_powershell_command | src/engine/tts.rs | Command args contain Add-Type -AssemblyName System.Speech and SetOutputToWaveFile with the given path, without actually spawning a process. |
| transcript_ui_headless_no_input_no_repaint | src/ui/transcript_ui.rs | assert_no_idle_repaint over 2 frames with a populated transcript and no interaction; matches subtitles_ui::show_headless pattern (line 897). |
| every_edit_op_has_a_tool_covers_subtitles_ops | src/ui/app/tools_registry_tests.rs (existing structural test, extended fixture) | cut_word_ranges and every other new pub fn ..(&mut self in ops/subtitles.rs appears in OP_TOOLS (name exists in mcp::tools::all()) or OP_INTERNAL with a reason. |

## Verification checklist

- [ ] cargo test (whole crate): all new + existing subtitles/transcribe/tracking tests green, no regression in duplicate_takes_are_marked_then_cut_with_the_cues or regenerate_replaces_its_own_cues_and_keeps_the_rest
- [ ] cargo run -- --selftest: idle step stays green with the Subtitles pane open and a transcript loaded
- [ ] Manual: Get captions -> download progress with cancel -> Transcribe -> Transcript section populates -> drag-select 3 words -> Delete -> clip shortens, cues/markers/words shift, one Undo restores everything
- [ ] Manual: type a filler into the list, click Mark fillers -> range markers appear -> Remove fillers -> clip cut, markers gone
- [ ] Manual: TTS panel -> type text -> Speak -> WAV asset appears in Library linked to the text clip, playable
- [ ] MCP: transcript.cut_words(clip_id, ranges=[[a,b]]) leaves exactly one History entry (push_undo_labeled); transcribe.run polls to completion and Project.transcripts contains the result after
- [ ] scripts/size.ps1 -Note transcript-captions: delta reported and compared against the +112 KB budget in the PR body
- [ ] Screenshot: Subtitles pane with Transcript section open, a word range selected, and a karaoke-highlighted cue over the preview frame
- [ ] Coordination check before wave-2 start: Project.transcripts + Project.subtitle_anim actually exist in main (landed by registries-schema-hooks); this workstream does not open its own PR against subtitle.rs/project.rs to add them
- [ ] Coordination check (audit-fix 1): confirm registries-schema-hooks' landed schema does NOT include a Cue.words field (per the audit, it was slated for removal as an unused consumer); if it lands anyway, this workstream still ignores it and reads Project.transcripts directly, no code change needed either way

## Acceptance criteria

- [ ] cargo test passes incl. every new test listed; existing regenerate_replaces_its_own_cues_and_keeps_the_rest and duplicate_takes_are_marked_then_cut_with_the_cues (src/ui/subtitles_ui.rs) still green unmodified
- [ ] A transcribed clip's Transcript persists across save/reopen (round-trip test), contingent on Project.transcripts landing via registries-schema-hooks first
- [ ] Selecting a word range in the Transcript section and pressing Delete ripple-cuts the clip and shifts cues/markers/words by the removed span, in one undo step
- [ ] 'Get captions' shows the exact MB size before any network call and never runs at process start
- [ ] transcribe.run / tracking.run / transcribe.install / tts.speak are ToolKind::Job in TOOL_TABLES and pollable like export.video
- [ ] every_edit_op_has_a_tool has an OP_TOOLS row (or OP_INTERNAL with reason) for cut_word_ranges and every other new pub fn(&mut self in ops/subtitles.rs
- [ ] assert_no_idle_repaint passes for the Transcript section and the caption-install progress bar
- [ ] no new egui::Modal / blocking rfd dialog added; TTS/whisper installs use the existing Progress+Cancel pattern
- [ ] cargo build --release size delta stays within the +112 KB budget from size_log.csv, or the PR states a size line with reason
- [ ] This workstream's diff touches no line in src/model/subtitle.rs or src/model/project.rs -- Project.transcripts/subtitle_anim arrive from registries-schema-hooks
- [ ] This workstream's diff does not read or write any Cue.words field, whether or not registries-schema-hooks retains it (audit-fix 1 recommends dropping it there as unused)

## Risks

| Risk | Mitigation |
|---|---|
| Project.transcripts / Project.subtitle_anim do not exist yet anywhere in the codebase and were never named in the skeleton's wave-0b decisions/god_file_split/registry_protocol text either -- so wave-0b was never actually tasked with pre-seeding them, and src/model/subtitle.rs + src/model/project.rs are registries-schema-hooks' exclusive owns_files (principle #4: feature branches never touch a type definition). | File the two fields as a request against registries-schema-hooks's wave-0b PR (or into this workstream's own ws:transcript-captions marker section within Settings/Project per the registry protocol, if 0b leaves a generic per-workstream Project-field slot). This workstream's src/model/ops/subtitles.rs code is written against the assumed shape but does not land until the fields exist in main; do not add them via a subtitle.rs/project.rs edit from this branch. |
| Cue has no clip reference (model.rs:2333-2338: id/start/end/text only) and project.subtitles is one flat Vec<Cue>, so a Cue.words field could never be attributed to the correct Transcript when more than one clip is transcribed -- the original plan declared Cue.words but never actually read or wrote it. | Cue.words is dropped from project_fields. cue_layer_at instead looks up the single most-recently-active Project.transcripts entry directly by time t (v1 ceiling: karaoke is correct for one active transcript at a time, wrong if two transcribed clips overlap on screen simultaneously -- documented as a ponytail note, not a Cue.clip field, since multi-clip simultaneous karaoke is not a stated goal). Audit-fix 1 independently arrived at the same conclusion from registries-schema-hooks' side (its Cue.words field has zero consumers across all 23 plans) and recommends dropping it there too -- fully consistent, no further change needed in this plan. |
| Word-to-timeline mapping is one linear (offset, scale) per clip (transcribe.rs retime:372); a speed-ramped (keyframed) clip's words drift from real audio position. | ponytail: refuse cut_word_ranges / Transcript editing on a clip with animated speed (check Clip.speed.is_animated()) and surface a toast; upgrade path is per-segment retime, documented, not built here. |
| Per-word PopWord animation needs a second TextRasterizer pass per frame if not cached. | Cache rasterized karaoke frames by (transcript.clip, active_word_index, style.cache_key()) the same way sub_key already caches (compose.rs:229-233); cold cache only on the frame the active word changes. |
| tracking.run as a Job wraps a channel-based TrackJob (not the Arc<Progress> shape every other Job uses), risking a mismatched McpJob poll contract. | Wrap TrackJob's poll loop inside export::spawn_job's closure (busy-poll with a short sleep, forwarding .progress into prog.set()) so the outer shape presented to ToolKind::Job is uniform Arc<Progress>; TrackJob itself stays untouched. |
| filler_ranges' word-matching is naive substring/exact-token match; contractions or ASR variants could be missed or over-matched. | Reuse normalize() (already proven by duplicate-take tests) rather than inventing new tokenization; document the ceiling with a ponytail comment. |
| SAPI TTS voices are OS-tier and vary per Windows install; a hardcoded voice name could fail silently on another machine. | voices() queries the live list and the combo only offers what's actually installed; Speak with voice=None uses the system default and UI copy says OS voices, not neural. |
| Audit-fix 2: the master skeleton keymap table omits this workstream's ToggleTranscript action id (and forgiveness's UndoSettings), a documentation-completeness gap noticed during cross-plan audit. | No code or plan change on this workstream's side -- ToggleTranscript is correctly unbound (sc()=>None) in hotkeys.rs per this plan already. Fix is appending both rows to the skeleton's top-level keymap array; that edit belongs to whichever workstream owns the skeleton/docs artifact (docs-refresh), not to this plan's files[]. |

## Suggested implementation order

1. 1. Coordinate with registries-schema-hooks (wave 0b) to land Project.transcripts: Vec<Transcript> and Project.subtitle_anim: SubtitleAnim with #[serde(default)] -- do NOT add these from this branch; block on that PR merging to main first
2. 2. engine::transcribe::filler_ranges + tests (pure fn, no UI dependency)
3. 3. model/ops/subtitles.rs: cut_word_ranges + transcript_hits + tests, generalizing the existing cut_dups logic (keep cut_dups calling the new shared fn)
4. 4. engine::subtitles::cue_layer_at karaoke branch (reads the single active Transcript entry) + parity test against the pre-refactor behavior
5. 5. engine::tts.rs (speak_to_wav, voices) + its Command-building unit test
6. 6. src/ui/transcript_ui.rs: word list, search+jump, filler chips, wired into subtitles_ui.rs behind show_transcript
7. 7. Settings.filler_words/filler_pad_ms fields + defaults
8. 8. hotkeys.rs 3 rows, tools.rs Glyph::Transcript + draw arm
9. 9. src/ui/app/tools_text.rs: all 9 MCP tools wired to the above, registered in TOOL_TABLES
10. 10. tracking.run Job wrapper (isolated risk item, last)
11. 11. Full test pass + --selftest + scripts/size.ps1 + screenshots

## Deliberate simplifications (`// ponytail:`)

- Word search is plain substring over normalize()'d tokens, no phonetic/fuzzy matching -- ScriptSync-lite is explicitly skipped project-wide; upgrade path is Needleman-Wunsch alignment if ever asked for.
- cut_word_ranges refuses (returns 0) on a clip with animated (keyframed) speed rather than attempting per-segment retime -- documented ceiling, upgrade path is segment-wise ripple_time.
- TTS voice list is queried live via one powershell call and cached for the session (OnceLock, same pattern as transcribe::settings()) rather than shipping a bundled voice catalogue.
- tracking.run's Job wrapper busy-polls TrackJob at a fixed short interval instead of restructuring TrackJob to use channels-as-Progress uniformly across the codebase -- smallest diff that presents a uniform Job contract without touching tracking.rs (not owned by this workstream).
- Karaoke drives off the single most-recently-active Project.transcripts entry rather than a per-Cue clip reference: Cue has no clip field and project.subtitles is one flat list, so exact multi-clip attribution would require adding Cue.clip (a type-definition change outside this workstream's owned files). Correct for the common case of one active caption source at a time; wrong only if two transcribed clips' captions overlap on screen simultaneously. Upgrade path: request a Cue.clip: Option<Id> field from registries-schema-hooks if that scenario is ever needed.

## Review trail

- FINDING 1 (ownership, confirmed by grep: no Transcript/subtitle_anim/transcripts symbol exists anywhere in src/; skeleton's god_file_split assigns src/model/subtitle.rs + src/model/project.rs exclusively to registries-schema-hooks, and principle #4 bars feature branches from touching type definitions): removed src/model/subtitle.rs and src/model/project.rs from files[]; changed project_fields from 'declare with #[serde(default)] if missing' to 'REQUESTED from wave-0b, not this workstream's to declare'; added a coordination step as implementation_order #1 and an acceptance_criterion that the diff touches neither file; added risk #1 documenting the gap and its mitigation (file the request against registries-schema-hooks, do not self-declare).
- FINDING 2 (wrong-path, confirmed by reading model.rs:2333-2338 and line 2975: Cue{id,start,end,text} has no clip field, project.subtitles is one flat Vec<Cue>, and the original plan's own cue_layer_at algorithm never actually reads/writes Cue.words): dropped Cue.words from project_fields entirely (was declared-but-dead); rewrote the karaoke design in engine_changes/new_types_and_fns/tests to read the single active Project.transcripts entry directly rather than 'the owning Transcript' via a nonexistent Cue-to-clip link; documented the resulting v1 ceiling (one active transcript drives karaoke; wrong only if two transcribed clips' captions overlap simultaneously) as ponytail_notes + risks entry rather than adding a Cue.clip field, since that field would be a second type-definition change outside this workstream's owned files and multi-clip simultaneous karaoke was never a stated requirement.
- AUDIT-FIX 1 applied (registries-schema-hooks' wave-0 Cue.words field has zero consumers across all 23 plans, confirmed by the auditor's cross-plan grep): this plan already independently reached the identical conclusion in FINDING 2 above -- Cue.words was dropped here before the audit ran, for the same root cause (no clip attribution possible, no reader/writer anywhere). No change to this plan's files/scope was needed; added a cross-reference note in risks and project_fields confirming the audit's recommendation (drop Cue.words from registries-schema-hooks' own schema) is consistent with, and doesn't require any edit to, this workstream. The actual field removal is registries-schema-hooks' file (src/model/subtitle.rs), outside this plan's owns_files.
- AUDIT-FIX 2 applied (skeleton's master keymap table omits ToggleTranscript from this workstream and UndoSettings from forgiveness -- a documentation-completeness gap, not a chord collision since both are unbound): confirmed ToggleTranscript is correctly declared unbound (sc()=>None) in this plan's hotkeys.rs edit and actions_and_hotkeys[]; no functional or file-scope change made here since the fix targets the skeleton's own top-level keymap array (owned by docs-refresh / the skeleton document), not this workstream's files. Added a note to actions_and_hotkeys[ToggleTranscript].note and a risks entry pointing at the correct owner for the doc edit.
- Everything else (scope, remaining files, ui_changes, settings_fields, other hotkeys, glyphs, MCP tool table, size_delta_kb, tests, risks 2-6 unchanged in substance, acceptance criteria, implementation order, ponytail notes) preserved verbatim.
