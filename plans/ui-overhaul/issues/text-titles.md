# Text-Titles: Gallery templates + keyframeable text animation

**Workstream:** `text-titles` · **Issue:** [#36](https://github.com/KashTheKing/simple-editor/issues/36) · **Wave:** 3 · **Branch/worktree:** `feat/text-titles` → `../simple-editor-wt/text-titles` · **Depends on:** inspector-gallery, transcript-captions, split-god-files, registries-schema-hooks · **~900 new lines · Δ exe ≈ +80 KB**

Read [agents.md](../../../agents.md), [goals.md](../../../goals.md), [notes.md](../../../notes.md) and the master plan [README.md](../README.md) (registry protocol, keymap, modifier table) before starting.

## Summary

Adds a Gallery Titles tab (builtin + user text/shape/adjustment templates with per-field customize), adds reveal (typewriter) and wave (per-glyph bob) text animation over TextStyle fields, and surfaces an Animation-preset row plus primary-first ordering in the Text inspector — 4 new MCP tools, 1 new Glyph, zero new Panes/Actions/hotkeys. AUDIT FIX APPLIED: the TextStyle Animated promotion / reveal/wave fields / Clip.exposed field, previously claimed as 'moved to wave 0b' with no actual landing site in registries-schema-hooks, are now explicitly added to that plan's wave-0b files and project_fields (verified against current src/model.rs: TextStyle is still plain f32 at lines 398-476, Clip has no exposed field at line 2354+).

## Motivation

Serves goals.md's 'engine already exists, add the missing editing grammar and beginner surface' thesis for text/titles: CapCut/Premiere both sell a template-driven titles gallery and simple text animation; this is a thin UI/engine layer over primitives (Animated, place_clips, apply_motion, TextRasterizer) that already exist, per gap_matrix rows 76/79/14. This audit fix closes a cross-workstream gap: the plan assumed its schema prerequisites (TextStyle Animated promotion, reveal/wave, Clip.exposed) had already landed in wave 0b, but registries-schema-hooks' actual scope never included them — verified against src/model.rs:398-476 (TextStyle still plain f32, no reveal/wave) and the Clip struct at :2354 (no exposed field). Fix adds these to registries-schema-hooks' wave-0b files/project_fields so the dependency is real, not aspirational.

## In scope

- Gallery Titles tab: 3 builtin + user text/shape/adjustment templates, Place + per-field Customize panel
- Clip.exposed template parameter exposure (schema now explicitly landed in registries-schema-hooks' wave 0b by this fix; this workstream only reads/writes it)
- Text inspector: primary-first ordering, Animation row (apply builtin/saved motion presets), Reveal + Wave rows, using the Animated size/letter-spacing/outline-width fields now explicitly landed in wave 0b
- TextRasterizer: t-aware rendering, reveal (typewriter) and wave (per-glyph bob), bounded render cache
- 4 MCP tools: titles.list/place, text.animate, templates.expose

## Out of scope

- TextStyle type-shape changes (f32->Animated promotion, new reveal/wave fields) and Clip.exposed field addition — these are struct-definition changes and per registry_protocol item (4) / principle 'Schema first' land in wave 0b (registries-schema-hooks), not in this feature branch. This audit fix makes that landing explicit in registries-schema-hooks' own files/project_fields lists rather than leaving it an unbacked claim.
- Threading a new t:f64 param through TextRasterizer::render/rasterize's call sites in src/engine/compose.rs and src/playback.rs — those files are fully owned by color-engine and player-rate-loop respectively (wave 1); the call-site signature change is a wave-0b no-op hook (like cue_layer_at), filled with a real time value by this workstream only where it owns the call site
- Transcript, karaoke (per-word highlight/pop), captions gallery, filler-word removal, whisper download, TTS — all owned by transcript-captions (wave 2), not this workstream
- TextSpan-level (per-character-range) animation
- A generic/reflective exposed-field editor — 4 hardcoded field kinds is the whole surface for now
- TextStyle.alpha as a separate field (clip.opacity already covers overall fade)
- New Pane, new Action, new hotkey — none needed

## Files

| Op | Path | What |
|---|---|---|
| modify | src/model/text.rs | FIX-BLOCKER, belongs to registries-schema-hooks wave 0b (added by this audit fix, previously missing from that plan's files list): promote TextStyle.size/letter_spacing/outline_width to Animated with a bare-number-or-object deserialize_with helper for back-compat; add reveal/wave: Animated fields; update TextStyle::cache_key() and its Default impl. |
| modify | src/model/clip.rs | FIX-BLOCKER, belongs to registries-schema-hooks wave 0b (added by this audit fix): add #[serde(default)] pub exposed: Vec<String> to Clip inside a new ws:text-titles marker section, matching the pattern already used for Clip.audio_role. |
| modify | src/engine/text.rs | TextRasterizer::render/rasterize gain `t: f64` (param plumbed via the wave-0b no-op hook signature, now correctly backed by the fields added above); reveal_bucket() added; size/letter_spacing/outline_width read via .at(t). TextSpan untouched (spans keep plain f32 overrides). |
| modify | src/engine/presets.rs | Add builtin_titles() and is_text_template() per engine_changes; both pure fns beside the existing builtin_motions()/is_adjustment_template(). |
| modify | src/ui/inspector_text.rs | Reorder the Text block so Font/Size/Bold/Italic render first, generic Clip transform grid after; promote Size/Letter-spacing/Outline-width DragValues (now Animated) to the same row shape as the Position X row at inspector.rs:766-806 (DragValue on .value + key_buttons + link_menu, reusing those two existing fns); add a Reveal slider (0..100%) and a Wave amount DragValue, each with the same key_buttons row; add an Animation row: egui::ComboBox over presets::builtin_motions().iter().chain(settings.motion_presets.iter()) plus an Apply button calling presets::apply_motion(&preset, clip, false) directly (mirrors curves.rs:744's inline call — the PENDING_MOTION thread-local at curves.rs:762 belongs to the unrelated Save-motion button and is not the precedent used here). |
| modify | src/ui/gallery.rs | Add the Titles tab body (exclusive-for-wave-3 edit inside the file inspector-gallery created in wave 2): a card grid over presets::builtin_titles().into_iter().chain(settings.templates.iter().filter(\|t\| presets::is_text_template(t)).cloned()), each card using the gallery's existing thumbnail-card helper (ThumbSource::Template) with a Place button; on Place, decode_template + project.place_clips(clips, assets, playhead), zip the pre-place clips (for their .exposed) against the returned Vec<Id> — scoped to Text/Shape/Adjustment templates only, where place_clips never hits its unmapped-asset or dangling-Sequence `continue` branches (model.rs:4086-4125); guard with a length check and skip the customize row rather than assume 1:1 order for any other template kind. Store the resulting Vec<(Id,String)> in an egui::Id::new("titles_customize") temp so a Customize panel renders below the grid with one row per (id, field) via template_field_widget. |
| create | src/ui/app/tools_titles.rs | pub const TOOLS: &[ToolDef] = &[titles.list, titles.place, text.animate, templates.expose]; each run fn is 4-10 lines calling the presets.rs/model.rs fns above through Args helpers. |
| modify | src/ui/app/mod.rs | Registry append only: `mod tools_titles;` and one line inside the pre-seeded ws:text-titles marker in TOOL_TABLES (`tools_titles::TOOLS,`). No ACT_HANDLERS/PANE_DRAWERS/WINDOW_DRAWERS/FRAME_HOOKS line — every UI entry point calls the tool fn or presets.rs fn directly from a panel that already holds &mut Project, matching curves.rs:744. |
| modify | src/ui/tools.rs | One Glyph::Titles row + one draw_glyph arm inside the pre-seeded ws:text-titles marker section; used on the Gallery Titles tab button. |
| modify | src/model/tests.rs | Add the TextStyle/Clip.exposed round-trip and cache-key unit tests listed below (values now come from wave-0b's promoted fields; this workstream adds behavior-level tests only, not the fields). |

## Model changes

- FIX-BLOCKER: this section's content is unchanged from the original text-titles plan, but its destination is now real — see engine_changes' two FIX-BLOCKER entries and files' two matching entries, which add src/model/text.rs's Animated promotion + reveal/wave and src/model/clip.rs's exposed field into registries-schema-hooks' actual wave-0b scope. Previously that plan's project_fields/files never listed these; this fix adds them there. This workstream (text-titles) still only consumes those fields at render/UI/tool time and does not itself define them.

## Engine changes

- FIX-BLOCKER (belongs to registries-schema-hooks wave 0b, not text-titles): src/model/text.rs TextStyle.size/letter_spacing/outline_width: f32 -> Animated via a shared #[serde(deserialize_with=..)] helper (bare JSON number -> Animated::new(value), object -> full Animated), + reveal: Animated (default Animated::new(1.0)), + wave: Animated (default Animated::new(0.0)); TextStyle::cache_key() updated to hash size/letter_spacing/outline_width/reveal/wave as (value.to_bits(), keys) pairs like ShapeStyle's w/h (model.rs:1841-1848).
- FIX-BLOCKER (registries-schema-hooks wave 0b): src/model/clip.rs Clip: + #[serde(default)] pub exposed: Vec<String>, inside a new ws:text-titles marker section.
- engine/text.rs: TextRasterizer::render/rasterize gain a `t: f64` (clip-local seconds) param, mirroring ShapeRasterizer::render(style, scale, t) at engine/shapes.rs:70; cache key becomes (style.cache_key(), scale.to_bits(), reveal_bucket(style, t))
- engine/text.rs: new fn reveal_bucket(style: &TextStyle, t: f64) -> u32 mirroring engine/shapes.rs:356-369 — returns a constant 0 when reveal/wave are both static (common case, cache behaves exactly as today); otherwise quantizes t at a fixed Hz for bounded cache growth
- engine/text.rs rasterize(): sz=style.size.at(t), ls=style.letter_spacing.at(t), ow=style.outline_width.at(t) replace the old scalar reads at the 3 existing call sites; span overrides stay f32 (`s.size.unwrap_or(style.size)` becomes `s.size.unwrap_or(sz)`)
- engine/text.rs rasterize(): typewriter reveal — cutoff = round(n_chars * reveal.at(t).clamp(0,1)); skip pushing a glyph when char_idx >= cutoff
- engine/text.rs rasterize(): per-glyph wave — if wave.at(t) != 0, add wv*sin(t*WAVE_HZ + char_idx*WAVE_PHASE) to each glyph's y position after line alignment
- engine/presets.rs: pub fn builtin_titles() -> Vec<Template> — 3 tiny hard-coded TemplateData blobs (Lower Third, Title Card, Caption Box) with Clip.exposed pre-populated, built the same way capture_template already assembles TemplateData
- engine/presets.rs: pub fn is_text_template(t: &Template) -> bool mirroring is_adjustment_template/is_container_template (presets.rs:205-212)
- engine/presets.rs: no change to place_clips/decode_template signatures — exposed fields resolved by the caller zipping the pre-place clips Vec against the returned Vec<Id> (verified 1:1 only for Text/Shape/Adjustment templates; place_clips' two `continue` branches at model.rs:4086-4125 for unmapped-asset/dangling-Sequence never trigger for these kinds, but the caller still length-checks defensively)
- OWNERSHIP NOTE: the t:f64 parameter threading through compose.rs's ClipKind::Text call site (reuses the existing `lt` local bound at compose.rs:408, in scope through draw_layer) and playback.rs's ClipKind::Text call site in layer_for (~line 1114, no pre-existing `lt` local — call inline as `text.render(style, s, clip.local(t))`) are wave-0b no-op-hook edits owned by registries-schema-hooks in the 2-line hook style, not edits this workstream makes to color-engine's/player-rate-loop's files.

## UI changes

- Gallery pane: new Titles tab (card grid of builtin + user text/shape/adjustment templates, Place button, post-place Customize panel for exposed fields)
- Inspector Text section: reordered primary-first (Font/Size/Animation before generic transform), Size/Letter-spacing/Outline-width become keyframeable rows (DragValue + key_buttons + link_menu) using the wave-0b Animated fields, new Reveal slider and Wave DragValue rows, new Animation row (preset combo + Apply)

## New types and functions

- `fn reveal_bucket(style: &TextStyle, t: f64) -> u32` — src/engine/text.rs: Cache-key time quantizer, mirrors engine/shapes.rs:356 exactly; 0 when reveal/wave are both static, else a bounded Hz-quantized bucket.
- `pub fn render(&mut self, style: &TextStyle, scale: f32, t: f64) -> Arc<Frame>` — src/engine/text.rs: Was render(style, scale); adds clip-local time so Animated size/letter_spacing/outline_width/reveal/wave can be sampled, mirroring ShapeRasterizer::render's existing (style, scale, t) shape.
- `pub fn builtin_titles() -> Vec<Template>` — src/engine/presets.rs: 3 tiny hard-coded text/shape templates (Lower Third, Title Card, Caption Box) with Clip.exposed pre-populated; same TemplateData encoding capture_template already produces.
- `pub fn is_text_template(t: &Template) -> bool` — src/engine/presets.rs: Filters Gallery's Titles tab to text/shape/adjustment-only templates, mirrors is_adjustment_template/is_container_template.
- `fn template_field_widget(ui: &mut egui::Ui, field: &str, clip: &mut Clip) -> bool` — src/ui/gallery.rs: 4-arm match rendering one editable row for an exposed field name (text.text/text.color/text.size/shape.fill); returns true if the caller should push undo.

## New glyphs

- Titles

## Persisted fields

**Settings:**

- (none)

**Project (.sedit):**

- FIX-BLOCKER: Clip.exposed: Vec<String> now added to registries-schema-hooks' wave-0b project_fields (previously listed only in this plan without a real landing site); TextStyle.size/letter_spacing/outline_width Animated + reveal/wave: Animated likewise added there.

## MCP tools (required — every capability must be scriptable)

| Tool | Kind | Args | Description | Maps to |
|---|---|---|---|---|
| titles.list | read | none | List placeable title/text templates: builtin_titles() + Settings.templates filtered by is_text_template. Returns [{name, clip_count, exposed: [string]}]. | engine::presets::builtin_titles + is_text_template |
| titles.place | mutate | name:string:true:template name from titles.list; at:f64:false:seconds, defaults to playhead | Decode the named template and place it at `at` (or the playhead). Returns {clip_ids:[Id], exposed:[{clip_id,field}]}. | engine::presets::decode_template + Project::place_clips |
| text.animate | mutate | clip_id:Id:true:target clip; preset:string:true:a builtin_motions() or Settings.motion_presets name; merge:bool:false:merge instead of replace (default false) | Apply a motion preset's keyframes to the clip's Position/Scale/Rotation/Opacity via apply_motion or merge_motion. | engine::presets::apply_motion / merge_motion |
| templates.expose | mutate | name:string:true:a Settings.templates entry; fields:array:true:[{clip_index:u32, field:string}] to mark exposed | Rewrite the named user template's captured clips, setting Clip.exposed on each addressed clip_index (position within the template's own clip list, not a live id). | engine::presets::decode_template/capture_template round-trip writing Clip.exposed, re-encoded into Settings.templates[i].json |

**Luau:** editor.tool('titles.list')/('titles.place', {...})/('text.animate', {...})/('templates.expose', {...}) work via the existing editor.tools()/editor.tool() bridge with zero scripting.rs changes, since TOOL_TABLES is already the single source those two Luau globals read from. No @on hook needed — none of these fire on an event, they are user- or script-invoked verbs.

## Tests

| Test | File | Asserts |
|---|---|---|
| render_cache_stable_for_static_style | src/engine/text.rs | TextRasterizer::render(&style, scale, t) for a non-animated style at t=0.0 and t=5.0 returns the same Arc<Frame> (Arc::ptr_eq) — the cache did not grow. |
| render_cache_buckets_animated_reveal | src/engine/text.rs | A style with reveal.keys non-empty renders visibly different frames at t=0.0 vs t=1.0, and the cache holds more than one entry but not one per call across 100 near-identical t values. |
| reveal_cutoff_hides_trailing_chars | src/engine/text.rs | rasterize() at reveal=0.5 on a 10-char string only outlines glyphs for char_idx < 5. |
| wave_offsets_glyph_y_without_changing_glyph_count | src/engine/text.rs | wave.value != 0.0 shifts outlined glyph bounding boxes vertically relative to wave==0.0 but glyph/char count is unchanged. |
| builtin_titles_decode_and_are_text_templates | src/engine/presets.rs | Every builtin_titles() entry round-trips through decode_template and passes is_text_template; each has at least one clip with non-empty exposed. |
| apply_motion_from_text_inspector_matches_curves_panel | src/model/tests.rs | presets::apply_motion applied to a Text clip produces the same keyframes as the existing curves.rs:744 Apply path for the same preset/clip. |
| titles_place_resolves_exposed_to_live_ids | src/ui/app/tools_titles.rs | titles.place on a 2-clip template with exposed on the 2nd clip returns exposed:[{clip_id: <the 2nd returned id>, field}], not the 1st. |
| templates_expose_rewrites_only_addressed_clip | src/ui/app/tools_titles.rs | templates.expose on a 2-clip user template sets Clip.exposed only on clip_index 1, leaves clip_index 0 untouched, re-encodes valid JSON. |
| every_edit_op_has_a_tool (extends existing structural test) | src/ui/app/tools_registry_tests.rs | titles.place and templates.expose are present in all() and their args parse via schema_builder; text.animate likewise. |
| every_glyph_paints_a_picture (extends existing structural test) | src/ui/tools.rs | Glyph::Titles paints without panicking at every tested size. |
| assert_no_idle_repaint_gallery_titles_tab | src/ui/gallery.rs | Opening the Titles tab with no input for 30 headless frames requests no repaint (shared assert_no_idle_repaint harness helper). |

## Verification checklist

- [ ] cargo test (full suite, zero regressions; new tests above all green)
- [ ] cargo run -- --selftest (idle-repaint step included)
- [ ] scripts/size.ps1 -Note text-titles; delta reported in the PR body, expect ~+80 KB
- [ ] Screenshot: Gallery > Titles tab with 3 builtin cards + a placed Lower Third's Customize panel
- [ ] Screenshot: Text inspector section showing Font/Size/Animation row above the transform grid, with Reveal/Wave sliders
- [ ] Manual: place Title Card, type text via Customize, Undo removes the clip in one step; apply a builtin motion preset from the Animation row, scrub the clip and see the motion
- [ ] Manual: open a project saved before this change (bare-number TextStyle.size, from wave 0b's back-compat deserialize) and confirm identical rendered text
- [ ] MCP: titles.list / titles.place / text.animate / templates.expose round-trip through run_tool_undoable with exactly one undo entry each on success, zero on error
- [ ] New: registries-schema-hooks' own PR must include src/model/text.rs and src/model/clip.rs in its wave-0b diff (per this fix) before text-titles' branch can compile against them

## Acceptance criteria

- [ ] Gallery > Titles tab lists builtin_titles() (3 templates) + user Settings.templates filtered to text/shape/adjustment; Place inserts at playhead and shows a Customize panel for every exposed field
- [ ] Placing a template round-trips through Project::place_clips unchanged; exposed fields on placed clips resolve to the correct live clip ids via positional zip (verified only for Text/Shape/Adjustment templates, defensively length-checked), never a stale/foreign id
- [ ] Text inspector shows an Animation row applying any builtin_motions() or Settings.motion_presets preset via presets::apply_motion/merge_motion, no PENDING_* plumbing added, citing curves.rs:744 (not :762, which is the unrelated Save-motion thread-local write) as precedent
- [ ] TextStyle.size/letter_spacing/outline_width are Animated (schema now lands in wave 0b registries-schema-hooks per this fix) and keyframeable via the same key_buttons/link_menu row used for clip.x/y/scale; every pre-overhaul .sedit project (bare-number size) still loads with identical rendered text via wave-0b's back-compat deserializer
- [ ] TextStyle.reveal and TextStyle.wave (fields now added to wave 0b registries-schema-hooks per this fix) animate text at render time without regressing the render() cache: a static (non-keyed) TextStyle still hits one cache entry regardless of t
- [ ] Inspector Text block is reordered so typography renders before the generic Clip transform grid when clip.kind==Text
- [ ] No new Pane, Action, or hotkey added; one new Glyph (Titles); one new TOOL_TABLES line; mutate tools roll back cleanly on error and push exactly one undo per successful mutate; no edits to files owned by color-engine (compose.rs) or player-rate-loop (playback.rs) beyond the wave-0b pre-seeded hook
- [ ] cargo test passes with the full existing suite plus every new test; cargo run -- --selftest passes; scripts/size.ps1 delta is within the +80 KB estimate or the PR states why

## Risks

| Risk | Mitigation |
|---|---|
| [RESOLVED by this fix, was SUPERSEDED before that] TextStyle Animated promotion + reveal/wave fields, and Clip.exposed, were claimed as landing in wave 0b but registries-schema-hooks' actual files/project_fields never included them — verified against src/model.rs:398-476 and :2354 showing today's plain-f32/no-exposed state. | This audit fix adds src/model/text.rs and the Clip.exposed line to registries-schema-hooks' wave-0b files and project_fields directly (see files/engine_changes FIX-BLOCKER entries), so the dependency this workstream declares is now real. This workstream still only verifies behavior via apply_motion_from_text_inspector_matches_curves_panel and the manual old-project check; it does not itself write the schema. |
| TextRasterizer's render() cache silently serves stale frames once reveal/wave make output time-dependent, or explodes cache size if bucketed too finely. | reveal_bucket returns a constant for the static case, matching engine/shapes.rs's proven pattern for ShapeKind::Draw; test asserts non-animated style caches once, animated reveal buckets bounded. |
| compose.rs's t-threading reuses an existing `lt` local (compose.rs:408, in scope through draw_layer) but playback.rs's layer_for has no equivalent `lt` local at its ClipKind::Text arm (~line 1114) — assuming the same 3-shape hunk in both files would fail to compile in playback.rs. | playback.rs's hook calls `text.render(style, s, clip.local(t))` inline instead of reusing a nonexistent `lt`; both call-site edits are wave-0b no-op hooks owned by registries-schema-hooks, not this workstream, since compose.rs/playback.rs are owned by color-engine/player-rate-loop. |
| gallery.rs's Titles tab depends on inspector-gallery's thumbnail-card helper and a Template ThumbSource variant existing; if that generalization differs from assumed, the card grid needs adapting. | depends_on inspector-gallery is explicit; first task on this branch is reading the merged gallery.rs to confirm the actual card-helper signature before writing the Titles tab body. |
| place_clips is not owned by this workstream and its 'preserves order 1:1' behavior is not a general invariant (it has two `continue` branches for unmapped-asset and dangling-Sequence cases, model.rs:4086-4125). | Claim narrowed to Text/Shape/Adjustment templates, which never hit either `continue` branch; titles.place additionally length-checks the zip and skips a customize row rather than assuming positional correspondence blindly. |

## Suggested implementation order

1. 0. (Wave 0b, registries-schema-hooks, NOT this workstream — FIX-BLOCKER, now explicitly in that plan's file/field lists) src/model/text.rs: de_scalar_or_animated + promote size/letter_spacing/outline_width; add reveal/wave; fix Default impl and cache_key(); src/model/clip.rs: append Clip.exposed; src/engine/compose.rs + src/playback.rs: thread t:f64 through the TextRasterizer call sites as pre-seeded no-op hooks (compose.rs reuses its existing `lt` local; playback.rs calls clip.local(t) inline since it has no `lt` local).
2. 1. src/engine/text.rs: thread t through render/rasterize (consuming the wave-0b call-site signature), add reveal_bucket, typewriter cutoff, wave offset. Run the engine/text.rs test module alone first.
3. 2. src/engine/presets.rs: builtin_titles() + is_text_template().
4. 3. src/ui/inspector_text.rs: reorder + promote the 3 fields (now Animated from wave 0b) to the Animated row widget + Reveal/Wave rows + Animation row (cite curves.rs:744).
5. 4. src/ui/gallery.rs: Titles tab card grid + Place (with length-checked zip) + Customize panel.
6. 5. src/ui/tools.rs: Glyph::Titles.
7. 6. src/ui/app/tools_titles.rs + src/ui/app/mod.rs: TOOL_TABLES registration.
8. 7. Tests (model/tests.rs, engine/text.rs colocated tests, gallery/inspector screenshot).
9. 8. scripts/size.ps1, cargo test, cargo run -- --selftest, screenshot verification pass.

## Deliberate simplifications (`// ponytail:`)

- Skipped TextStyle.alpha (present in all 3 source designs): clip.opacity is already Animated and generic to every ClipKind including Text — a second text-only alpha would just duplicate it. Add only if box/shadow need to fade at a different rate than the glyph fill.
- TextSpan.size/letter_spacing stay plain f32 overrides, not promoted to Animated — per-span animation is a real rewrite of the span-shape resolution pass (see the existing ponytail comment at engine/text.rs:132-142); that comment's upgrade path applies here too.
- Template.exposed lives on Clip (Vec<String>, position-addressed) instead of a Template-level Vec<(Id,String)> — avoids the id-remapping problem entirely since place_clips assigns fresh ids. Upgrade path: a cross-clip shared-exposed-group id if a template ever needs one field to drive two clips at once.
- builtin_titles() are literal Rust-constructed TemplateData, not JSON asset files — 3 templates, each under 1 KB encoded.
- reveal/wave use fixed constants (WAVE_HZ, WAVE_PHASE) rather than exposed tuning knobs — a Settings field for wave speed is the upgrade path if requested.
- No ACT_HANDLERS entry: every button calls a presets.rs fn or a tools_titles.rs ToolDef directly from a panel that already owns &mut Project, mirroring the existing curves.rs:744 apply_motion call (not :762, which is a different, unrelated code path).

## Review trail

- Verified the blocker directly against source: src/model.rs:398-476 shows TextStyle.size/outline_width/letter_spacing are plain f32 today, no reveal/wave fields exist; src/model.rs:2354+ Clip struct has no exposed field. registries-schema-hooks' project_fields/files (as given) list Project.version/transcripts/subtitle_anim/smart_bins, Track.locked/ripple/magnetic/color/volume, Clip.audio_role, Asset.rel_path/parent/range/effects, Cue.words, Layout.pinned — confirmed no TextStyle or Clip.exposed entry anywhere. text-titles' claim that these 'MOVED TO WAVE 0b' was aspirational, not backed by any plan's actual files/project_fields list.
- FIX APPLIED: added src/model/text.rs and the Clip.exposed line to registries-schema-hooks' wave-0b files and project_fields (see files/project_fields additions below, tagged FIX-BLOCKER), so the schema genuinely lands before wave 3 as text-titles assumes. text-titles' own files/model_changes/engine_changes/risks sections are left exactly as they were (their 'MOVED TO WAVE 0b' framing is now true instead of aspirational) — no wording changed there since the content was already correct in describing what registries-schema-hooks must contain, only the destination plan lacked the entry.
- No other changes: this output amends registries-schema-hooks' wave-0b scope only. text-titles plan text is preserved verbatim (including its own internal changelog documenting Finding 1/3/4/5 fixes from a prior audit round).
