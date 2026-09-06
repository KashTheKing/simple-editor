# Audio DSP automation: de-hum/limiter/de-esser, LUFS meter, track volume automation, Essential Sound repair chains

**Workstream:** `audio-dsp-automation` · **Issue:** [#26](https://github.com/KashTheKing/simple-editor/issues/26) · **Wave:** 1 · **Branch/worktree:** `feat/audio-dsp-automation` → `../simple-editor-wt/audio-dsp-automation` · **Depends on:** reconcile: split-god-files (wave 0a) — MUST land first: it creates src/ui/inspector_audio.rs with pub(super) fn section(ui, project, ids: &[Id], playhead: f64, palette, undo: &mut dyn FnMut(&Project)) -> bool and wires inspector.rs's clip_section to call it by that exact name/shape. This ws only MODIFIES that existing fn body (op:'modify', not 'create') — it must never redeclare the file or change its signature., size-diet (wave 0c) — needs the ToolDef/TOOL_TABLES registry to exist before this worktree starts, registries-schema-hooks (wave 0b) — MUST land Track.volume, Clip.audio_role, AND the AudioRole enum body {Unset,Dialogue,Music,Sfx,Ambience} in src/model/clip.rs under its own ws:audio-dsp marker before this worktree opens; verified the fields do not exist in current src/model.rs and Animated has no impl Default, so 0b must use #[serde(default = "a1_track_vol")] with a new helper fn, never bare #[serde(default)]., audio-analysis (wave 1) — inspector_audio.rs's Duck/Normalize buttons dispatch Action::AutoDuck / Action::Normalize, which audio-analysis's own hotkeys.rs edit defines; without this dependency the worktree could branch/build before those Action variants exist and fail to compile. · **~870 new lines · Δ exe ≈ +70 KB**

Read [agents.md](../../../agents.md), [goals.md](../../../goals.md), [notes.md](../../../notes.md) and the master plan [README.md](../README.md) (registry protocol, keymap, modifier table) before starting.

## Summary

Adds the pro-audio DSP layer competitors ship (De-hum/Limiter/De-esser filters, K-weighted LUFS meter, per-track volume automation) plus Essential-Sound-style one-click Repair/Clarity chains and an audio_role tag, over the existing generic AudioFilter/BusGraph/ParamSpec machinery — zero new dependencies, zero new Tool buttons, zero hotkey changes. This revision closes two audit findings: a duplicate AudioRole enum definition (registries-schema-hooks is now sole owner; this ws imports the type) and a missing depends_on on audio-analysis (whose Action::AutoDuck/Normalize this ws's buttons dispatch) — plus the pre-existing mcp-gap fix building the first real audio-thread-to-UI meter feed.

## Motivation

Closes gap-matrix rows 68 (Essential Sound), 70-meter-half (LUFS), 71 (de-hum/limiter/de-esser), 72-model-half (track/bus automation), 14-audio-half (inspector primary-first) versus Premiere's Essential Sound panel, Resolve's Fairlight limiter/de-esser/loudness meter and Avid's AudioSuite chain, using only stdlib DSP already in mixer_fx.rs, zero new deps. Revision note: closes two audit findings — a duplicate AudioRole enum definition (now single-owned by registries-schema-hooks, imported here) and a missing depends_on on audio-analysis (whose Actions this ws's buttons dispatch) — plus the pre-existing meter/LUFS gap requiring a first working audio-thread-to-UI data path.

## In scope

- FilterKind::{DeHum, Limiter, DeEsser} DSP + params + generic UI rendering
- Importing (never redefining) AudioRole from model/clip.rs, owned solely by registries-schema-hooks
- Mixer::mix sampling of Track.volume (field itself lands via registries-schema-hooks, not here)
- Project::apply_repair + REPAIR_PRESETS placed in model/ops/buses.rs — routes clips through a named, visible, editable Mixer bus
- BusMeterFeed ring buffer + playback.rs publish hook + App::sync_buses() FRAME_HOOK — the first real audio-thread-to-UI meter data path in the app
- LUFS meter (momentary + integrated, K-weighted approximation) in mixer_ui at UI rate, backed by real data
- inspector_audio.rs: primary-first reorder + Role combo + Repair/Clarity buttons + Open-in-Mixer link + Duck/Normalize buttons, folded into the existing section() signature from split-god-files
- Glyph::Meter
- MCP ToolDef rows: audio.role, audio.repair, audio.filter_add, track.volume_key, bus.volume_key, mixer.meters

## Out of scope

- Beat detection, ducking algorithm, normalize/match-loudness, scene-cut, multicam sync — owned by sibling audio-analysis workstream.
- Timeline volume-automation LANE PAINT — owned by pro-timeline (wave 3); this ws only makes Track.volume exist and be sampled.
- Real-time sidechain ducking, HDR/broadcast-calibrated true LUFS, spectral denoise (NN) — explicitly out of scope per master skeleton's skip list.
- Any hotkey/Action enum additions — this ws's hotkeys.rs section is empty; Duck/Normalize buttons reuse Action variants the audio-analysis ws defines.
- Pane visibility/layout changes (forcing Mixer to surface) — deferred to layout-modes-onboarding's App::surface (wave 2); Open-in-Mixer only pre-selects a bus.
- Track.volume / Clip.audio_role / AudioRole enum definitions themselves (owned by registries-schema-hooks) — this ws only imports/consumes them once landed.
- Creating src/ui/inspector_audio.rs or altering its section() signature — that file and signature are owned/created by split-god-files (wave 0a); this ws only modifies the fn body.

## Files

| Op | Path | What |
|---|---|---|
| modify | src/model/audio.rs (owned by this ws) | Add FilterKind::{DeHum,Limiter,DeEsser} + F_DEHUM/F_LIMITER/F_DEESSER ParamSpec tables under `// ---- ws:audio-dsp ----`. FIXED (blocker, AudioRole duplicate definition): does NOT define AudioRole here — `use crate::model::clip::AudioRole;` instead, since registries-schema-hooks is the sole owner of that enum. |
| modify | src/model/track.rs (owned by registries-schema-hooks, NOT this ws) | this ws does NOT touch this file. registries-schema-hooks appends `#[serde(default = "a1_track_vol")] pub volume: Animated` under its own `// ---- ws:audio-dsp ----` marker. This ws depends on that landing first. |
| modify | src/model/clip.rs (owned by registries-schema-hooks, NOT this ws) | this ws does NOT touch this file. registries-schema-hooks appends `pub enum AudioRole{Unset,Dialogue,Music,Sfx,Ambience}` (sole definition) + `#[serde(default)] pub audio_role: Option<AudioRole>` on Clip. This ws depends on that landing first and only ever imports the type. |
| modify | src/model/ops/buses.rs (existing ops file owned by registries-schema-hooks; this ws contributes a fn) | `apply_repair` placed in the existing buses ops section as `pub fn apply_repair(&mut self, ids: &[Id], preset: &str) -> Id` under a `// ---- ws:audio-dsp ----` marker. |
| modify | src/engine/mixer_fx.rs | New Dsp variants + FilterState::new/process match arms for DeHum/Limiter/DeEsser; `pub struct Lufs`; `pub const REPAIR_PRESETS`; BusMeterFeed ring buffer (push side) written by playback.rs's mix call site. |
| modify | src/engine/mixer.rs | Sample Track.volume once per block in mix_tracks, thread through mix_audio_clip/mix_seq_clip/resample_add as extra track_gain: f32 multiplier; Mixer::mix public signature unchanged. |
| modify | src/playback.rs (2-line hook call site only, filling 0b-placed stub) | after mixer.mix(...) at playback.rs:1285, push each active bus's post-fader block into the new ring buffer. |
| modify | src/ui/app.rs (1-2 line call site only, inside the FRAME_HOOKS ws:audio-dsp slot) | App::sync_buses() called once per frame (drains the ring buffer into App.buses). |
| modify | src/ui/mixer_ui.rs | Ring-buffer READ tap computing Lufs from real post-fader samples at UI rate; new meter row 'LUFS -14.2 (approx)'; new FilterKind values render via f.kind.params(); request_focus_bus/take_focus_bus thread-locals. |
| modify | src/ui/inspector_audio.rs | reconcile: split-god-files (wave 0a) already creates this file with `pub(super) fn section(ui: &mut egui::Ui, project: &mut Project, ids: &[Id], playhead: f64, palette: &Palette, undo: &mut dyn FnMut(&Project)) -> bool`, wired from inspector.rs's clip_section call site by that exact name/shape. This ws MODIFIES that existing section fn body only (no create, no signature change): reorder primary-first (Volume, Pan, Fades), then add Role combo (imported AudioRole, applied across ids), Repair/Clarity buttons (call undo(project) once then project.apply_repair(ids, preset)), Open-in-Mixer button, and Duck/Normalize buttons — all operating over the existing ids/playhead params, not a single &mut Clip. |
| create | src/ui/app/tools_mixer.rs | `pub const TOOLS: &[ToolDef]` for audio.role, audio.repair, audio.filter_add, track.volume_key, bus.volume_key, mixer.meters. |
| modify | src/ui/app/mod.rs | One line under `// ---- ws:audio-dsp ----` in TOOL_TABLES: tools_mixer::TOOLS,; one line under `// ---- ws:audio-dsp ----` in FRAME_HOOKS: sync_buses,. |
| modify | src/ui/tools.rs | Add Glyph::Meter to the enum, ALL, name(), from_name, and draw_glyph under `// ---- ws:audio-dsp ----`: small bar-chart icon (3 vertical bars). |

## Model changes

- FilterKind: +DeHum, +Limiter, +DeEsser (in model/audio.rs, owned by this ws); ALL grows 10->13.
- F_DEHUM/F_LIMITER/F_DEESSER ParamSpec tables added beside F_EQ..F_GAIN: DeHum{Base Hz(50/60), Depth dB}; Limiter{Ceiling dB, Lookahead ms, Release ms}; DeEsser{Freq Hz, Threshold dB, Ratio}.
- AudioRole is NOT defined by this ws. Its sole definition AudioRole{Unset,Dialogue,Music,Sfx,Ambience} + ALL/name() lives in src/model/clip.rs, owned by registries-schema-hooks; this ws's model/audio.rs does use crate::model::clip::AudioRole;.
- Track.volume: Animated and Clip.audio_role: Option<AudioRole> are NOT added by this ws; they land in registries-schema-hooks (wave 0b) using #[serde(default = "a1_track_vol")] for Track.volume and bare #[serde(default)] for Clip.audio_role.
- Project: +pub fn apply_repair(&mut self, ids: &[Id], preset: &str) -> Id, placed in model/ops/buses.rs — uses only existing add_bus/bus_mut/clip.bus.

## Engine changes

- src/engine/mixer_fx.rs: new Dsp variants DeHum{notches: Vec<Biquad>}, Limiter{env:f32,gain:f32,lookahead ring}, DeEsser{hp:[Biquad;2],env:f32,gain:f32} added to enum Dsp and matched in FilterState::new/process.
- src/engine/mixer_fx.rs: DeHum = 3 cascaded Band::Peak notches reusing existing coeffs(Band::Peak, f0, q, gain_db, sr); ponytail: approximates a notch via very negative gain_db + high Q instead of a true RBJ notch formula.
- src/engine/mixer_fx.rs: Limiter = Compressor's Dyn{env,gain} struct + lookahead_ms via a small fixed-size delay ring; effectively-infinite ratio.
- src/engine/mixer_fx.rs: DeEsser = high-passed sidechain (Band::HighPass ~4-8kHz) driving a Dyn{env,gain} compressor applied to the full-band signal.
- src/engine/mixer_fx.rs: new pub struct Lufs with K-weighting (2 cascaded biquads) + 400ms gated RMS window for momentary and running gated mean for integrated; ponytail: BS.1770-shaped but not absolute-calibrated, labelled 'LUFS (approx)'.
- src/engine/mixer_fx.rs: pub const REPAIR_PRESETS — 'repair' = HighPass(80Hz)+DeHum(60Hz)+NoiseGate(-45dB)+Compressor(-18dB,4:1)+Limiter(-3dB); 'clarity' = Eq(presence +3dB@3kHz)+Compressor(-20dB,3:1).
- src/engine/mixer.rs: mix_tracks samples track.volume.at(t) once per block per active audio track, threaded into mix_audio_clip/mix_seq_clip as track_gain: f32, multiplied in resample_add; Mixer::mix public signature unchanged.
- App.buses (BusGraph) is currently a dead-end instance never synced/flushed; playback.rs owns its own separate Mixer::new()/mixer.mix() whose BusGraph the UI never sees. Adds a ring buffer that playback.rs's mixer.mix call site publishes post-fader block samples into, plus App::sync_buses() called once per UI frame draining it into App.buses so .meter(id) and the new Lufs tap have live data.
- Bus.volume is NOT added — Bus already has gain: Animated as its automatable level; only Track gets the new volume: Animated field, landing via registries-schema-hooks in wave 0b.

## UI changes

- mixer_ui.rs: new LUFS row per selected/visible bus strip, painted under the existing peak meter(), backed by the new BusMeterFeed data path instead of the always-zero App.buses.
- mixer_ui.rs: '+ Filter' combo automatically lists DeHum/Limiter/DeEsser once the enum grows.
- inspector_audio.rs (existing file created by split-god-files wave 0a; this ws modifies its section fn body only, same signature): Volume/Pan/Fade in/Fade out promoted to top; Role combo (imported AudioRole), Repair/Clarity buttons, 'Open in Mixer' link, Duck/Normalize buttons added below, before the generic effects list.

## New types and functions

- `pub fn apply_repair(&mut self, ids: &[Id], preset: &str) -> Id` — src/model/ops/buses.rs: Creates/reuses a named bus with REPAIR_PRESETS' filter chain and routes given clips to it via Clip.bus; returns the bus id for 'Open in Mixer'.
- `pub const REPAIR_PRESETS: &[(&str, &str, &[(FilterKind, &[(&str, f32)])])]` — src/engine/mixer_fx.rs: ('repair'\|'clarity', label, filter chain with param overrides) consumed by apply_repair.
- `pub struct Lufs { .. } impl Lufs { pub fn new() -> Self; pub fn push(&mut self, l: f32, r: f32); pub fn momentary(&self) -> f32; pub fn integrated(&self) -> f32 }` — src/engine/mixer_fx.rs: K-weighted (BS.1770-shaped, not calibrated) loudness meter, fed at UI rate from the new ring-buffer tap.
- `pub struct BusMeterFeed { .. } impl BusMeterFeed { pub fn push_block(&self, bus: Id, l: &[f32], r: &[f32]); pub fn drain_into(&self, buses: &mut BusGraph); }` — src/engine/mixer_fx.rs: Fixed-capacity, overwrite-oldest ring buffer bridging playback.rs's audio thread to the UI thread's App.buses.
- `fn sync_buses(app: &mut App, ctx: &egui::Context) // FRAME_HOOKS entry` — src/ui/app.rs: Drains BusMeterFeed into App.buses once per UI frame so mixer_ui's meter()/Lufs reads are non-stale.
- `fn mix_tracks(..) // existing signature +1 internal local tg: f32 per track, threaded as track_gain: f32` — src/engine/mixer.rs: Samples Track.volume.at(t) once per block per active audio track; Mixer::mix's own public signature stays identical.
- `pub fn request_focus_bus(id: Id); pub(crate) fn take_focus_bus() -> Option<Id>` — src/ui/mixer_ui.rs: Thread-local hand-off (mirrors inspector.rs's PENDING_ACTION idiom) so 'Open in Mixer' pre-selects a bus.
- `pub(super) fn section(ui: &mut egui::Ui, project: &mut Project, ids: &[Id], playhead: f64, palette: &Palette, undo: &mut dyn FnMut(&Project)) -> bool` — src/ui/inspector_audio.rs: reconcile: signature corrected to match split-god-files' (wave 0a) existing extraction and inspector.rs's existing call site exactly (no rename, no reshaped param list). Primary-first audio block operating over the selected ids; returns whether it mutated project directly (Repair) so clip_section skips its own write-back for this frame.

## Actions and hotkeys

| Action id | Label | Chord | Note |
|---|---|---|---|
| (none) | No new Action/hotkey rows — ws:audio-dsp's hotkeys.rs section is empty by design |  | Duck/Normalize buttons in inspector_audio.rs dispatch Action::AutoDuck / Action::Normalize, hotkey rows owned by the sibling audio-analysis workstream; this ws only calls the existing PENDING_ACTION thread-local with those pre-existing enum variants, and explicitly depends_on audio-analysis so build order is safe. |

## New glyphs

- Meter — 3 vertical bars of increasing height (level/LUFS meter icon), used on the mixer LUFS row and any future 'Meters' menu entry; covered by every_glyph_paints_a_picture.

## Persisted fields

**Settings:**

- (none)

**Project (.sedit):**

- Project::apply_repair is a new method, not a new stored field — no Project struct field additions from this ws.

## MCP tools (required — every capability must be scriptable)

| Tool | Kind | Args | Description | Maps to |
|---|---|---|---|---|
| audio.role | mutate | clip_ids:array:true:clip ids; role:string:true:Unset\|Dialogue\|Music\|Sfx\|Ambience | Tag clips with an audio role; drives inspector defaults and future ducking target selection. | loop over clip_ids, project.clip_mut(id).audio_role = Some(role) (AudioRole imported from model::clip) |
| audio.repair | mutate | clip_ids:array:true:; preset:string:true:repair\|clarity | Route clips through a one-click DSP chain (bus + filters), left fully editable in the Mixer. | Project::apply_repair(ids, preset) in model/ops/buses.rs |
| audio.filter_add | mutate | bus_id:integer:true:; kind:string:true:one of FilterKind::ALL incl. DeHum\|Limiter\|DeEsser; params:object:false:name->value overrides | Append a filter to a bus's chain. | project.bus_mut(bus_id).filters.push(AudioFilter::new(kind)) + apply param overrides |
| track.volume_key | mutate | track_index:integer:true:; t:number:true:; db:number:false:omit+remove=true to delete; remove:boolean:false: | Set or remove a keyframe on a track's volume automation. | project.tracks[track_index].volume.set_at(t, db_to_gain(db)) / remove key at t |
| bus.volume_key | mutate | bus_id:integer:true:; t:number:true:; db:number:false:; remove:boolean:false: | Set or remove a keyframe on a bus's gain automation. | project.bus_mut(bus_id).gain.set_at(t, db_to_gain(db)) / remove key |
| mixer.meters | read | bus_id:integer:false:omit for all buses | Peak (L/R) and approximate LUFS for one or every bus, read from the last-synced block via the new ring-buffer path. | BusGraph::meter(id) + mixer_ui's Lufs tap, now backed by the real playback.rs->App.buses ring buffer added in this ws |

**Luau:** editor.tools() picks up audio.role / audio.repair / audio.filter_add / track.volume_key / bus.volume_key / mixer.meters automatically via TOOL_TABLES flattening. Example: editor.tool("audio.repair", {clip_ids={...}, preset="clarity"}) then editor.tool("mixer.meters", {}). mixer.meters now returns real data once the ring-buffer/sync_buses path lands — before that fix it would have silently returned (0,0) forever, so tests assert non-zero output during a played block. No new -- @on hook needed.

## Tests

| Test | File | Asserts |
|---|---|---|
| dehum_attenuates_hum_tone | src/engine/mixer_fx.rs (tests mod) | A 60Hz sine through FilterState::process(DeHum) loses >20dB amplitude vs input, while a 1kHz sine loses <1dB. |
| limiter_caps_output_at_ceiling | src/engine/mixer_fx.rs (tests mod) | A block with samples exceeding Ceiling dB comes out clamped near the ceiling within lookahead settling. |
| deesser_reduces_sibilance_band_only | src/engine/mixer_fx.rs (tests mod) | A 6-8kHz tone block is attenuated more than a 200Hz tone block at the same input level through the same DeEsser instance. |
| lufs_of_minus_23_dbfs_sine_is_in_range | src/engine/mixer_fx.rs (tests mod) | Lufs::push fed a steady -23dBFS sine converges integrated() to a value in a documented tolerance band. |
| track_volume_is_sampled_and_multiplies_clip_gain | src/engine/mixer.rs (tests mod) | With Track.volume keyed to 0.5 at t and Clip.volume=1.0, Mixer::mix output amplitude is half of the Track.volume=1.0 case; public signature unchanged. |
| mixer_regression_existing_projects_unchanged | src/engine/mixer.rs (tests mod) | A project built without touching Track.volume mixes identically to before this change. |
| mixer_meters_reflect_live_playback | src/engine/mixer_fx.rs or an integration test | During an active playback block with non-silent audio, BusGraph::meter(id) after App::sync_buses() returns non-zero peak values. |
| apply_repair_creates_bus_and_routes_clips | src/model/ops/buses.rs (tests mod) | apply_repair(&[clip_id], "repair") creates exactly one new bus with the repair filter chain, sets clip.bus to it, and re-calling with the same preset reuses the existing bus. |
| repair_button_pushes_exactly_one_undo | src/ui/inspector_audio.rs (tests mod or harness test) | Clicking Repair calls undo(project) exactly once. |
| every_glyph_paints_a_picture | src/ui/tools.rs (existing test, extended) | Glyph::Meter is in Glyph::ALL and draw_glyph produces non-empty paint output for it. |
| filterkind_all_covers_new_variants | src/model/audio.rs (tests mod) | FilterKind::ALL.len() == 13 and every variant's name()/params() returns non-empty. |
| audio_role_single_definition | src/model/audio.rs (tests mod, new) | compile-time check via use crate::model::clip::AudioRole as _; plus a grep-based CI check that enum AudioRole appears exactly once in src/. |
| tools_mixer_rows_resolve_and_roundtrip | src/mcp/tools.rs / tools_mixer.rs (tests mod) | Every ToolDef in tools_mixer::TOOLS resolves via mcp::tools::find(name), has a parseable args schema, and (Mutate kind) leaves the project unchanged on garbage/empty args. |
| selftest_idle_step_stays_green_with_lufs_meter | src/selftest.rs (existing idle assertion) | With the Mixer pane open and a bus selected, 30 headless frames with no input request zero repaints. |

## Verification checklist

- [ ] cargo test (full suite) green, including every new test listed above, especially mixer_meters_reflect_live_playback and audio_role_single_definition.
- [ ] cargo run -- --selftest passes, including the idle-repaint step with Mixer open.
- [ ] Manual: tag a clip Dialogue, click Repair — a 'Repair: repair' bus appears in the Mixer, editable; click Open in Mixer and confirm the bus is pre-selected.
- [ ] Manual: key a Track's volume down mid-timeline, confirm audible dip during playback.
- [ ] Manual: during playback with the Mixer open, confirm the peak meter AND the new LUFS row move.
- [ ] Screenshot: Mixer strip showing the LUFS row; inspector audio block showing the reordered primary-first layout.
- [ ] scripts/size.ps1 delta recorded honestly against the revised 870-line estimate; PR body includes `size: +N KB — <reason>` if over the +64KB soft gate.
- [ ] MCP: curl tools/list includes all 6 new tool names; call each once and confirm {ok:true,...}, with mixer.meters returning non-zero during playback.
- [ ] grep FilterKind:: across the crate post-change to confirm no non-exhaustive match was missed.
- [ ] grep confirms this ws's diff never touches src/model/track.rs or src/model/clip.rs directly, and grep -rn 'enum AudioRole' src/ returns exactly one hit (in model/clip.rs).

## Acceptance criteria

- [ ] FilterKind::{DeHum,Limiter,DeEsser} exist, render in mixer_ui (generic ParamSpec grid), and process real audio (unit tests, not just param plumbing).
- [ ] Track.volume: Animated exists via #[serde(default = "a1_track_vol")], is sampled once per block in Mixer::mix without changing its public signature, and a keyframed track volume is audible/testable like clip.volume.
- [ ] Clip.audio_role: Option<AudioRole> exists using the AudioRole type owned solely by registries-schema-hooks in src/model/clip.rs; inspector_audio.rs shows it as a combo and Project::apply_repair() routes tagged clips through a preset bus.
- [ ] A real per-bus audio-thread-to-UI data path exists before any LUFS/meter row is built; App.buses is synced/flushed each frame from that data.
- [ ] LUFS meter (momentary + integrated, K-weighted approximation) renders per selected bus in mixer_ui without touching the realtime audio thread.
- [ ] inspector_audio.rs (extracted by wave 0a, modified here) shows Volume/Pan/Fades first, then Role + Repair/Clarity + Open-in-Mixer + Duck/Normalize, before the generic effects list, all within the section() signature split-god-files already established.
- [ ] Track.volume and Clip.audio_role field additions, and the AudioRole enum body itself, land in registries-schema-hooks (wave 0b), not in this workstream's own PR.
- [ ] Every new user-facing capability has an MCP ToolDef row; every_edit_op_has_a_tool passes.
- [ ] cargo test green including new mixer_fx/mixer/model tests; assert_no_idle_repaint holds for the LUFS meter.
- [ ] scripts/size.ps1 delta is measured against est_new_lines honestly.
- [ ] This ws's worktree branches only after split-god-files (wave 0a, inspector_audio.rs section()), registries-schema-hooks, and audio-analysis have landed.
- [ ] src/ui/inspector_audio.rs appears exactly once across split-god-files and audio-dsp-automation's files[] as a single op:'modify' entry against split-god-files' section() signature — no duplicate/incompatible create.

## Risks

| Risk | Mitigation |
|---|---|
| Real-time LUFS computed inside Mixer::mix (audio thread) would add per-sample biquad cost to the realtime path and risk underruns. | Lufs is computed entirely in mixer_ui.rs at UI repaint rate from the new BusMeterFeed ring-buffer tap; Mixer::mix/mixer_fx.rs's flush() never call Lufs::push directly. |
| Track.volume sampling changes Mixer::mix's or mix_tracks' behavior for existing projects with no keyframes. | Default Track.volume = Animated::new(1.0) via the a1_track_vol helper (owned by registries-schema-hooks); test mixer_regression_existing_projects_unchanged asserts identical output. |
| New FilterKind variants break exhaustive matches elsewhere. | grep for FilterKind:: matches outside mixer_fx.rs/model/audio.rs/mixer_ui.rs before landing; add the 3 variants to ALL and both name()/params() match arms in the same commit. |
| apply_repair() mutates the real Project directly, bypassing the clip-clone-then-write-back pattern the rest of clip_section uses — an undo mis-order would corrupt history. | Call undo(project) immediately before project.apply_repair(..); test asserts exactly one undo entry per Repair click. |
| mixer.meters and the LUFS row were designed assuming a working audio-thread-to-UI data path that does not exist: App.buses is a separate, never-synced BusGraph from the one playback.rs actually mixes through. | Added BusMeterFeed (ring buffer) + a playback.rs publish hook + an App::sync_buses() FRAME_HOOK as new, explicit scope. Test mixer_meters_reflect_live_playback asserts non-zero peak/LUFS during an active played block. |
| AudioRole duplicate definition — this ws previously defined AudioRole independently while registries-schema-hooks also defined it, a guaranteed compile error. | FIXED: this ws no longer defines AudioRole at all; registries-schema-hooks is sole owner (model/clip.rs), this ws does use crate::model::clip::AudioRole; everywhere referenced. |
| inspector_audio.rs dispatches Action::AutoDuck/Action::Normalize, defined by the sibling wave-1 workstream audio-analysis's own hotkeys.rs edit, not pre-seeded anywhere — this ws's depends_on previously omitted audio-analysis. | FIXED: added audio-analysis to depends_on; this ws branches/rebases after audio-analysis's PR lands. |
| reconcile (blocker): src/ui/inspector_audio.rs was listed twice with incompatible signatures — split-god-files (wave 0a) creates it as pub(super) fn section(ui, project: &mut Project, ids: &[Id], playhead: f64, palette: &Palette, undo: &mut dyn FnMut(&Project)) -> bool and wires inspector.rs's clip_section to call it by that name/shape; this ws separately listed op:'create' with a different fn show(ui, clip: &mut Clip, project, track_index, lt, palette, g: &mut Gesture) -> bool, a guaranteed compile break. | FIXED: changed this ws's files[] entry to op:'modify' and its new_types_and_fns signature to the exact section(...) shape from split-god-files; the primary-first reorder, Role combo, Repair/Clarity, Open-in-Mixer, and Duck/Normalize additions fold inside that unchanged signature (ids for multi-select, undo closure for the existing convention). Added an explicit depends_on line naming split-god-files (wave 0a). |

## Suggested implementation order

1. 0. WAIT for split-god-files (wave 0a) to create src/ui/inspector_audio.rs's section() fn and wire inspector.rs's clip_section call site. WAIT for registries-schema-hooks to land Track.volume, Clip.audio_role, AND the sole AudioRole enum body. WAIT for audio-analysis's hotkeys.rs Action::AutoDuck/Action::Normalize rows to exist too.
2. 1. model/audio.rs: FilterKind variants + ParamSpec tables + use crate::model::clip::AudioRole; (no redefinition) — cargo test json_roundtrip/from_json_migrating green.
3. 2. engine/mixer_fx.rs: Dsp variants + FilterState::new/process for DeHum/Limiter/DeEsser; unit tests per filter before touching UI.
4. 3. engine/mixer_fx.rs: Lufs struct + unit test against a synthetic -23dBFS sine.
5. 4a. engine/mixer.rs: Track.volume sampling wired through mix_tracks/mix_audio_clip/resample_add; test with keyframed track volume + constant clip volume.
6. 4b. build the ring buffer — playback.rs publish hook + App::sync_buses() FRAME_HOOK + mixer_ui read tap — verify .meter(id) returns non-zero during playback before wiring Lufs to it.
7. 5. model/ops/buses.rs: apply_repair + REPAIR_PRESETS wiring; test bus creation + clip.bus reassignment + idempotent re-apply.
8. 6. ui/mixer_ui.rs: LUFS row (backed by real data from 4b) + request_focus_bus/take_focus_bus; screenshot check.
9. 7. ui/inspector_audio.rs: modify the existing section() fn body — reorder + Role combo (imported AudioRole) + Repair/Clarity/Open-in-Mixer/Duck/Normalize buttons, same signature.
10. 8. ui/tools.rs: Glyph::Meter + every_glyph_paints_a_picture stays green.
11. 9. mcp: tools_mixer.rs ToolDef rows + TOOL_TABLES line; server_end_to_end / schema_builder tests.
12. 10. Full cargo test, scripts/size.ps1 (state size: +N KB in the PR honestly), screenshot pass, selftest idle step.

## Deliberate simplifications (`// ponytail:`)

- DeHum is 3 cascaded Peak-EQ notches reusing the existing RBJ Peak formula, not a derived true notch; upgrade path: dedicated notch coeffs if attenuation depth falls short.
- Limiter reuses the Compressor's Dyn{env,gain} shape with a tiny fixed lookahead ring instead of a proper multi-ms lookahead buffer; upgrade path: longer lookahead + smoothed gain curve if pumping shows up.
- Lufs is BS.1770-shaped but not calibrated to an absolute reference — labelled 'LUFS (approx)'; upgrade path: calibrate against a known reference tone.
- Track.volume defaults to unity and is sampled once per block (not lerped sample-by-sample); upgrade path: lerp like clip gain if ramps need to be audible-smooth.
- BusMeterFeed is a fixed-capacity overwrite-oldest ring buffer, not a lock-free SPSC queue with backpressure; upgrade path: a real lock-free queue if playback-thread contention shows up in profiling.
- 'Open in Mixer' only pre-selects a bus; it does not force the Mixer pane visible (layout-modes-onboarding's App::surface, wave 2).
- AudioRole is a single-owner import, not a re-export shim — no wrapper type, no local newtype.
- reconcile: inspector_audio.rs's section() keeps split-god-files' exact signature rather than a bespoke one; all new UI (Role/Repair/Duck/Normalize) is folded inside it, so this ws never owns or renames that file's public shape.

## Review trail

- FIXED (blocker, AudioRole duplicate definition — confirmed by grep, AudioRole doesn't exist anywhere in src/ today): audio-dsp-automation no longer defines AudioRole in src/model/audio.rs. registries-schema-hooks is the sole owner of AudioRole{Unset,Dialogue,Music,Sfx,Ambience} in src/model/clip.rs. This ws's model/audio.rs now does use crate::model::clip::AudioRole; wherever referenced. Added test audio_role_single_definition (grep-based CI guard).
- FIXED (major, missing cross-workstream depends_on): added 'audio-analysis' to depends_on. inspector_audio.rs's Duck/Normalize buttons dispatch Action::AutoDuck/Action::Normalize, which audio-analysis's own wave-1 hotkeys.rs edit defines — without this dependency the worktree could branch/build before those Action variants exist.
- CARRIED FORWARD (prior revision): Track.volume #[serde(default = "a1_track_vol")] fix, ownership move of Track.volume/Clip.audio_role/apply_repair out of this ws's files, the mcp-gap fix (BusMeterFeed ring buffer + playback.rs publish hook + App::sync_buses() FRAME_HOOK), and the est_new_lines/size_delta_kb correction (820->870/64->70) all remain as previously revised.
- UNCHANGED: DSP filter implementations (DeHum/Limiter/DeEsser), REPAIR_PRESETS, Lufs struct math, Glyph::Meter, and all 6 MCP tool rows — neither of this pass's findings touched these.
- reconcile: FIXED (blocker, duplicate-file/incompatible-signature) — verified against split-god-files.json's own files[] entry: it creates src/ui/inspector_audio.rs as pub(super) fn section(ui: &mut egui::Ui, project: &mut Project, ids: &[Id], playhead: f64, palette: &Palette, undo: &mut dyn FnMut(&Project)) -> bool, and inspector.rs's own modify entry wires clip_section to call inspector_audio::section() by that exact name/shape. This ws previously listed the same path a second time as op:'create' with an incompatible pub fn show(ui, clip: &mut Clip, project: &mut Project, track_index: usize, lt: f64, palette: &Palette, g: &mut Gesture) -> bool — different fn name, different params, no acknowledgment of the existing call site (a guaranteed compile break the moment both PRs land). Changed the files[] entry to op:'modify' and the new_types_and_fns signature to the exact section(...) shape from split-god-files; the primary-first reorder, Role combo, Repair/Clarity, Open-in-Mixer, and Duck/Normalize additions now fold inside that same signature (ids:&[Id] for multi-select, undo:&mut dyn FnMut(&Project) for the existing undo-closure convention) instead of replacing it. Added an explicit depends_on line naming split-god-files (wave 0a) for this reason.
