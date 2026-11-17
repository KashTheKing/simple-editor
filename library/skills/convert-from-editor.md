---
name: convert-from-editor
description: Translate a Premiere Pro, DaVinci Resolve, Avid or After Effects workflow, skill or recipe into Simple Editor steps.
args: "<paste the recipe, or describe it>   (add 'save as <name>' to save it as a new skill)"
tags: premiere, resolve, davinci, avid, after effects, convert, workflow, mcp
credit: concept mapping written for Simple Editor; tool names surveyed from open MCP servers for Premiere, Resolve and After Effects (MIT)
---
Turn a workflow written for another editor into one that runs here. The input is in $ARGS: a tutorial, a list of steps, an ExtendScript / Resolve-API / UXP script, an agent skill or MCP tool sequence for another editor. Output: an equivalent Simple Editor recipe, what has no equivalent, and (on request) a saved skill.

1. Read the recipe and list its steps in plain words: the goal of each step, not the other app's menu path. Note the inputs it assumes (bins, sequences, tracks, markers, effects, presets).
2. Map each step with the table below. Name only real tools; when unsure, check `tools/list` (or `ui.palette {query}` for menu actions). Prefer one undo group (`edit.begin` / `edit.end` or `edit.batch`) per logical change, a dry run or detection call before a destructive one, and `render.frame` / `render.contact_sheet` to look at the result.
3. Where there is no 1:1 feature, use the stated alternative and say so. Where there is none, put the step in a "Not supported" list with the closest manual workaround; never pretend.
4. Output, in this order: a short summary; a table "their step -> our step -> tool calls"; the "Not supported / different" list; then the converted recipe as numbered steps in the skill format of `library/README.md` (what to inspect first, which tools, what to check, when to ask).
5. If $ARGS says `save as <name>` (or the user agrees afterwards), save it with `skills.save {name, description, args, body}`: a lowercase-hyphen name, the body written in our terms (never paste the other app's text), `$ARGS` where the user's input goes, one undo group, nothing exported or deleted without asking. Then the user can run it as `/<name>`. Credit the source in the body's first line when it was someone else's published recipe, and only convert material whose licence allows it.
6. When the user asks to run the converted recipe right away, do it, then verify with `render.contact_sheet`.

## Concept map

| Their concept (Premiere / Resolve / Avid / After Effects) | Simple Editor | Tools |
|---|---|---|
| Project, bins, Media Pool, Smart Bins / Smart Collections | the library, folders, smart bins | `media.list`, `media.import`, `media.import_folder`, `media.set`, `media.smart_bin` |
| Subclip, Source monitor, mark In/Out, insert/overwrite (Avid splice-in / overwrite) | subclips and the Source monitor | `media.subclip`, `source.open`, `source.mark`, `source.insert`, `timeline.splice`, `timeline.overwrite`, `timeline.smart_edit` |
| Sequence / Timeline / Avid sequence; nest / compound clip / precomp | sequences; nesting | `sequence.list`, `sequence.open`, `timeline.nest`, `timeline.unnest` |
| Razor / Add Edit / blade all tracks | split | `timeline.split {t}` (no `clip_ids` = every track) |
| Ripple delete, lift, extract (Avid Lift / Extract) | ripple and lift | `timeline.delete {clip_ids, ripple}`, `timeline.extract {a, b}`, `timeline.lift {a, b}`, `timeline.close_gap` |
| Trim: ripple, roll, slip, slide; Avid trim mode; dynamic JKL trim | the same trims | `timeline.ripple_trim`, `timeline.roll`, `timeline.slip`, `timeline.slide`, `timeline.trim_edges`, `timeline.dynamic_trim`, `timeline.edit_point` |
| Split edit / J-cut / L-cut | roll the audio cut only | `timeline.roll {right_clip_id: <audio clip>, cut}` (`/jl-cuts`) |
| Replace edit, match frame | replace, match frame | `timeline.replace`, `timeline.place {mode: "overwrite"}`, `timeline.match_frame` |
| Markers (sequence / clip, colours, comments), Avid locators, AE comp markers | markers and comments | `markers.add`, `markers.set`, `markers.comment`, `markers.list`, `markers.export {format}` (csv, YouTube chapters, Avid marker text) |
| Scene Edit Detection / Resolve Scene Cut Detector | shot-change detection | `media.scene_cuts {clip_id, as_markers, split}` |
| Multicam source sequence, angle switching, sync by audio | multicam | `multicam.sync`, `multicam.create`, `multicam.switch`, `audio.sync_offset` |
| Text-based editing, transcript, Speech to Text | transcripts | `transcribe.run`, `transcript.get`, `transcript.search`, `transcript.cut_words`, `transcript.remove_fillers` |
| Captions track, SRT import/export, caption styles | subtitles | `subtitles.get`, `subtitles.set`, `subtitles.import`, `subtitles.style_preset`, `subtitles.animation`, `transcript.export` |
| Silence removal plug-ins / auto-editor style cutting | auto cut | `autocut.detect` (preview), `autocut.mark`, `timeline.auto_cut` |
| Auto Reframe | reframe from a tracked box | `tracking.run`, `timeline.reframe`, `clip.set {fields: {scale, x, y}}` (`/reframe`) |
| Motion / Transform / AE position, scale, rotation, opacity | clip properties | `clip.set {clip_id, fields}`, `clip.keyframe {clip_id, property, t, value, ease}` |
| Keyframe easing, Easy Ease, Bezier, hold keys | key easing | `clip.keyframe {ease}`: `EaseIn`, `EaseOut`, `EaseInOut`, `Hold`, `cubic-bezier(x1,y1,x2,y2)` |
| AE animation presets (.ffx), Premiere motion presets | keyframe blocks and motion presets | `keyblocks.list`, `keyblocks.apply`, `text.animate {preset}`, `clip.apply_motion` |
| AE expressions (wiggle, time, loopOut) | Link to expression (Luau) in the Inspector; Luau scripts | Inspector ▸ keyframe button ▸ Link to expression…; `scripts.run` |
| AE Convert Audio to Keyframes | bake audio level into keys | `audio.bake_keys {clip_id, target_id, effect, param}` (`/audio-to-keyframes`) |
| Adjustment layer | adjustment layer | `ui.action {id: "add_adjustment"}` |
| Effects (Lumetri, Gaussian Blur, Ultra Key, Warp Stabilizer, OFX, Resolve FX) | built-in effects, shader library | `clip.add_effect {clip_id, kind, params}`, `library.apply {id: "effects/..."}`, `clip.set_shader`, `export.stabilize` |
| Masks, Power Windows, roto | masks (shapes and points), tracker | `clip.add_mask`, `clip.set_mask`, `tracking.run` |
| Resolve node graph (serial / parallel / layer nodes) | node graph | `clip.add_node`, `clip.connect_nodes` |
| Lumetri Basic / Resolve primaries (lift, gamma, gain, offset, temp, tint), curves | Color page primaries, curves | `color.primaries`, `clip.add_effect {kind: "Color Curves"}`, `color.auto` |
| Resolve qualifier / HSL secondary | qualifier | `color.qualifier` |
| Shot match / Color Match | colour match | `color.match {clip_id, reference_clip_id}` (`/color-match`) |
| LUTs, Creative looks, grade stills / gallery, copy grade | LUTs, Looks, stills | `clip.add_lut`, `library.apply {id: "luts/..."}`, `gallery.apply {tab: "Luts"}`, `stills.save`, `stills.apply` |
| ASC CDL (slope, offset, power, saturation) | primaries approximation | `color.primaries` + `clip.add_effect {kind: "Hue / Saturation"}` (`/apply-cdl`) |
| Scopes (waveform, parade, vectorscope) | scopes | `scopes.read`, `frame.stats` |
| Transitions (Cross Dissolve, Dip to Black, Push, Wipe, plug-in transitions) | transitions, shader transitions | `timeline.add_transition {right_clip_id, kind, duration}` (kind `CrossFade`, `FadeToColor`, `Push` or `Wipe`), `gallery.apply {tab: "Transitions", where}`, `library.apply {id: "transitions/..."}` |
| Speed / Duration, reverse, freeze frame, time remapping, optical flow | speed, reverse, freeze, speed ramps, slow motion | `clip.set {fields: {speed, reverse, freeze}}`, `gallery.apply {tab: "SpeedRamps", name}`, `export.slowmo` |
| Essential Graphics / MOGRT, Fusion Titles, AE text layers | titles, HTML + CSS layers with fields | `titles.place`, `html.create`, `html.set {fields}`, `library.apply {id: "html/..."}` |
| Shape layers, solids | shapes | `shapes.add` |
| Essential Sound (Dialogue / Music / SFX / Ambience), Repair | audio roles, one-click repair chains | `audio.role`, `audio.repair {preset}` |
| Auto Ducking, Fairlight sidechain ducking | ducking | `audio.duck {music_id, dialogue_ids, depth_db}` |
| Loudness Radar, normalise, loudness match | analysis, normalise, match | `audio.analyze`, `audio.normalize`, `audio.match_loudness`, `mixer.meters` (`/loudness`) |
| Submixes, Fairlight buses, track FX, automation | buses, filters, volume keys | `audio.add_bus`, `audio.route`, `audio.filter_add`, `track.volume_key`, `bus.volume_key` |
| Beat detection, markers on music | beats | `audio.beats {clip_ids, as_markers, split}` |
| Export / Render Queue / Media Encoder / Deliver page, presets | export, render queue, presets | `export.presets`, `export.quick {path, preset}` (presets go here; `export.video` takes explicit size / crf instead), `export.queue`, `export.status` |
| Render in place, Smart Render, Resolve render cache | bake in place, automatic preview cache | `export.bake`, `render.range` |
| XML / EDL / AAF interchange | FCP7 XML and EDL out; XML, EDL and .prproj in | `export.xml`, `export.edl`, `timeline.import` |
| Proxies, relink, consolidate / Media Manager | automatic proxies, relink, consolidate | `jobs.proxy_next`, `media.relink`, `media.consolidate` |
| Scripting (ExtendScript, UXP, Resolve Python/Lua API, Avid) | Luau scripts and this MCP server | `scripts.list`, `scripts.run`, `edit.batch` |

## Not supported, or different

- 3D cameras and lights, AE 3D layers and the AE camera tracker: no 3D scene; use `clip.add_effect {kind: "3D Plane"}` for a tilted card, the point tracker `tracking.run` for 2D tracks.
- Fusion / AE node compositing beyond the clip node graph, particles, puppet pin, content-aware fill, roto brush: not available; suggest HTML layers or shader effects for stylised looks, or doing that shot in the other app and importing the render.
- Avid bin columns, ScriptSync, PhraseFind, group clips: use library columns, `transcript.search` and multicam instead.
- Resolve Fairlight ADR, Dolby Vision / HDR grading, ACES colour management, AAF / OMF export, Resolve collaboration: not available.
- Premiere Productions, Team Projects, Frame.io review: use markers with notes (`markers.comment`) and the project file.
- Generative extend, AI object removal, Magic Mask: not available; the AI upscaler (`enhance.upscale`) is the only AI picture tool.
- Plug-ins (OFX, VST, AE plug-ins) cannot load; use the built-in effects, shader library and audio filters.

Recipe to convert: $ARGS

Report back: the mapping table, what is not supported, the converted steps, and the skill name if one was saved.
