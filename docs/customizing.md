# Customizing Simple Editor

Scripts, MCP tools, keymaps, and layout — the extension points behind goals.md's "deep user
customizability" core goal. Everything here is transcribed from the running app at wave-3-complete
(`86c1793`), not from planning documents: the tool table below is a direct transcription of a live
`curl` against `tools/list`, and the `-- @on` event table is built from a grep of real `fire_hook(`
call sites in the merged tree.

## Luau scripting

Scripts live in `%APPDATA%\SimpleEditor\scripts\*.luau` (`src/scripting.rs`; the folder and a
starter `example.luau` are created the first time it's listed). Run one from the Scripts menu, bind
it to a hotkey, or trigger it over MCP (`scripts.run`).

**Sandbox**: no `io`/`os`/`ffi` — a script can only reach the editor through `editor.tool`/
`editor.tools`/`editor.log` (and `editor.event` when fired as a hook). A manual run gets a 5-second
wall-clock budget; a fired `-- @on` hook gets 250 ms by default (`DEFAULT_HOOK_BUDGET`), overridable
per script with `@budget_ms`. A hook that blows its budget is disabled for the rest of the session
with exactly one toast — an ordinary UI event (a selection change, an import finishing) must never
be allowed to make the editor feel like it hitched.

**Bridge surface** (`editor` table, `src/scripting.rs`):
- `editor.tool(name, args)` — call any MCP tool by name, same catalogue as the table below.
- `editor.tools()` — every tool as `{name, description}` (no `kind`/`args` — those aren't part of
  this bridge's return shape; see "A note on `gen_docs.luau`" below).
- `editor.log(text)` — shows a toast. **This is a 5-10 second auto-expiring toast with no copy
  button** (`palette_ctl.rs`'s `fire_hook`/the app's toast draw) — never a place to read output you
  need to keep. If you need real output, write to a file with `editor.tool("frame.export", …)` /
  `editor.tool("transcript.export", …)` style tools that write files, or query the same data over
  `curl` against the MCP endpoint directly.
- `editor.event` — set only inside a fired `-- @on` hook, to the event's JSON payload.

**Header metadata** (parsed without starting the VM, so it's cheap to list every script):
```
-- @name My Hook
-- @desc Does a thing
-- @icon bolt
-- @hotkey Ctrl+Shift+H
-- @on selection_changed
-- @on export_done
-- @budget_ms 500
```
`@icon` accepts one of a small fixed set (`bolt`, `terminal`, `wrench`, `gear`, `clock`, `magnet`,
`target`, `waveform`, `notepad`, `bookmark`, `film-reel`, `clapperboard`, `sliders`, `search`,
`keyboard`) — an unknown name is silently dropped, never an error. `@on` may repeat for multiple
events. Header parsing stops at the first line that isn't a recognised `-- @key` comment.

### `-- @on` event list (verified against the merged tree, not the original plan)

The overhaul's plan named six candidate events. All six now have a real `App::fire_hook(...)` call
site — verified here by grepping the actual merged tree (`grep -rn "fire_hook(" src/`), not by
restating the plan's intent:

| Event | Fires from | Payload |
|---|---|---|
| `selection_changed` | `src/ui/app/palette_ctl.rs:80`, inside `tick()` (a `FRAME_HOOKS` entry) — compares a `SelSig` signature so it also fires on a transition/subtitle-cue/edit-point selection, not just a clip-id diff | `{"clip_ids": [...]}` |
| `import` | `src/ui/app/media_sync.rs:197`, inside `finish_sequence` — once per finished image-sequence bake/import job | `{"asset_id", "path", "frames"}` |
| `export_done` | `src/ui/app/files.rs:454` (failure) and `:457` (success), inside `finish_export` — fires on every export outcome, not just success | `{"path", "ok": bool}` |
| `project_open` | `src/ui/app/files.rs:35`, inside `open_project` | `{"path"}` |
| `project_save` | `src/ui/app/files.rs:143`, inside `save_project` | `{"path"}` |
| `marker_added` | `src/ui/app/mod.rs:1758` (`fire_marker_added_for_each`), reached via `App::fire_markers_added` (`mod.rs:1749`) — one call per created marker, from every marker-creation path (`audio_actions.rs:34`, `panes.rs:248`, `tools_audio.rs:117/301/335`, `tools_transcript.rs:186`, `transcript_ctl.rs:190`) | `{"marker_id"}` |

No gap: every originally-planned event landed a real call site by wave-3-complete. What's still
true from the plan's own caution is that `App::fire_hook`'s doc comment (written by
`command-palette`, wave 1, before the other events' owners had wired their own call sites) is
stale — it still says only `selection_changed` is wired. Treat this table, not that comment, as the
current truth; a future doc pass can update the comment itself (out of scope here — this workstream
touches no `.rs` file).

## MCP tool catalogue

The app hosts an MCP server (Streamable HTTP, JSON-RPC 2.0) toggled in Settings — `Settings.mcp_enabled`
(default `false`) and `Settings.mcp_port` (default `7337`). Once on:

```
curl -s -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' -H 'content-type: application/json' \
  http://127.0.0.1:<port>/mcp
```

**Verified against a live call at wave-3-complete: 236 tools, exactly matching this table's count.**
`kind` isn't part of the wire response (`mcp::tools::list_json` only serialises
`name`/`description`/`inputSchema` — see `src/mcp/tools.rs`), so the `Kind` column below is a source
cross-reference (each tool's own `ToolDef.kind` in its `tools_*.rs` file), not something you'll see
in the raw JSON. `Mutate` pushes one undo step iff the project JSON actually changed and rolls back
on error; `Read`/`Ui` never touch the undo stack; `Job` starts a background job and replies when it
finishes. Names, args and descriptions below are transcribed directly from the `tools/list` response
above — not from `editor.tools()`/`editor.log()`, which is a lossy, auto-expiring bridge (see
"Luau scripting" above).

**Totals by kind**: 109 Mutate, 69 Read, 45 Ui, 13 Job.

**`audio.*`** (13 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `audio.add_bus` | Mutate | name | Create a bus (it feeds Main until routed elsewhere). |
| `audio.add_filter` | Mutate | bus, kind, params | Add a filter to a bus with optional params {name: value}. |
| `audio.analyze` | Read | clip_ids | Peak/RMS dBFS and onset count/BPM per clip (from cached waveform peaks); 'waveform still computing' if peaks aren't ready yet. |
| `audio.beats` | Mutate | as_markers, clip_ids, refractory_s, split | Detect beats on audio clip(s). With neither flag: pure detection, returns onset times (source secs) + BPM, no mutation. as_markers adds a clip marker per onset (fires marker_added); split cuts each target clip's link group at every beat. |
| `audio.buses` | Read | - | Mixer buses (id, name, gain, pan, mute/solo/mono, output, filters). |
| `audio.duck` | Mutate | depth_db, dialogue_ids, dry_run, music_id, ramp_s | Duck the music clip's volume under every loud (speech) window found on the dialogue clips' peaks, with a ramp. dry_run returns the computed key count without writing. |
| `audio.filter_add` | Mutate | bus_id, kind, params | Append a filter to a bus's chain (any FilterKind incl. DeHum\|Limiter\|DeEsser) with optional {param name: value} overrides. |
| `audio.match_loudness` | Mutate | clip_ids, dry_run | Scale every listed clip's gain toward the selection's average (bucket-)RMS. |
| `audio.normalize` | Mutate | clip_ids, dry_run, mode, target_dbfs | Set each clip's constant gain so its own peak/RMS reaches target_dbfs. Clips with keyframed volume are skipped (reported, not silently ignored). |
| `audio.repair` | Mutate | clip_ids, preset | Route clips through a one-click DSP chain (a named bus + filters), left fully editable in the Mixer; re-applying reuses the bus. Returns the bus id. |
| `audio.role` | Mutate | clip_ids, role | Tag clips with an Essential-Sound role; drives the inspector's audio defaults and future ducking target selection. |
| `audio.route` | Mutate | bus, clip_id, from_bus, track | Send a clip, a track or a bus into a bus (bus 0 = Main / inherit). |
| `audio.sync_offset` | Read | clip_a, clip_b, max_lag_s | Best lag (seconds) that aligns clip_b to clip_a via cross-correlation of their waveform envelopes. For multicam sync; the angle-grid UI is a later (pro-monitor) workstream. |

**`autocut.*`** (2 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `autocut.detect` | Read | clip_ids, keep_quiet, min_silence, min_speech, padding, threshold_db | Silence-detection preview: returns cut times + kept/removed ranges per clip without mutating the project (never applies). |
| `autocut.mark` | Mutate | clip_ids, keep_quiet, min_silence, min_speech, padding, threshold_db | 'Mark instead' of Apply: adds a project range marker (start,end) per detected silence segment (the one Apply would have cut/removed), leaving every clip untouched, and fires marker_added per marker. |

**`bus.*`** (1 tool)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `bus.volume_key` | Mutate | bus_id, db, remove, t | Set (db) or remove (remove=true) a keyframe on a bus's gain automation at t. |

**`caches.*`** (2 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `caches.clear` | Ui | - | Releases decoder file handles, clears in-memory waveform/thumb caches, deletes the on-disk cache dir; returns bytes freed. |
| `caches.size` | Read | - | Returns the on-disk cache directory size in bytes. |

**`clip.*`** (16 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `clip.add_effect` | Mutate | clip_id, kind, params | Append an effect with optional params {name: value}. Kinds: Blur, Motion Blur, Pixelate, JPEG Compression, VHS, Chroma Key, Color Replace, Threshold, Edge Glow, Color Tint, Color Correction, Color Curves, Levels, Hue / Saturation, Black & White, Invert, Vignette, Sharpen, Flip, Crop, 3D Plane, Camera Shake, Blob Tracking, Security Camera REC, Primaries, Qualifier, LUT, Frame Blend, Custom Shader. |
| `clip.add_lut` | Mutate | clip_id, intensity, path | Add an EffectKind::Lut effect (a .cube 3D LUT) to a clip; parses the file to validate before pushing. |
| `clip.add_mask` | Mutate | clip_id, effect, shape | Add a mask to a clip (or to one of its effects with `effect`). |
| `clip.add_node` | Mutate | clip_id, kind, x, y | Add a node to the clip's node graph (created from its effect stack on first use). |
| `clip.apply_motion` | Mutate | clip_id, name, scaled | Apply a motion preset (built-in or saved) to a clip. |
| `clip.bypass` | Mutate | clip_ids, on | Enable/disable every colour-grade effect (Color/Primaries/Curves/Levels/HueShift/Qualifier/Lut) on the given clips in one step. |
| `clip.connect_nodes` | Mutate | clip_id, from, port, to | Wire one node's output into another node's input port (cycles are refused). |
| `clip.crop` | Mutate | at, bottom, clip_id, feather, left, right, top | Find-or-append a Crop effect on the clip and set its fraction params (0..0.5 each) at time `at`. |
| `clip.effects_bulk` | Mutate | clip_ids, index, params | Set one stack-index effect's params on every listed clip whose effect at that index shares its kind (Project::bulk_set_effect_params). |
| `clip.fit` | Mutate | clip_id, mode | Fit (contain, native aspect) or stretch (fill canvas, non-uniform) the clip. |
| `clip.keyframe` | Mutate | clip_id, ease, property, remove, t, value | Set (or remove with remove=true) a keyframe of a property at clip-local time t. |
| `clip.mask_target` | Ui | effect | Point the existing mask-tool canvas drag at the clip's own mask, or one effect's. |
| `clip.remove_effect` | Mutate | clip_id, index | Remove effect at index. |
| `clip.reorder_effect` | Mutate | clip_id, from, to | Move an effect to a new stack index (Project::reorder_effect). |
| `clip.set` | Mutate | clip_id, fields | Set clip fields: name, enabled, label, speed, reverse, freeze (source time or null), blend, fade_in, fade_out, and constant values of properties (x, y, scale, rotation, opacity, volume, pan); text clips: text style fields (text, font, size, color [r,g,b,a], outline_width, …). |
| `clip.set_mask` | Mutate | clip_id, effect, fields | Edit a mask: fields {shape, cx, cy, rx, ry, rotation, feather, expand, opacity, invert, enabled, points:[[x,y],…]} in project pixels relative to the layer centre. |

**`color.*`** (5 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `color.auto` | Mutate | clip_id | Sample the frame at the playhead (via the live preview render path) and write/update an editable Levels+Color pair on the clip. |
| `color.match` | Mutate | clip_id, reference_clip_id | Match a clip's histogram to a reference clip's (both rendered at their own timeline start via the live preview path) as an editable Curves effect. |
| `color.pick` | Mutate | target, x, y | Sample the frame at (x,y); if a clip is selected, writes its ChromaKey/Qualifier target too. |
| `color.primaries` | Mutate | clip_id, gain_b, gain_g, gain_r, gamma_b, gamma_g, gamma_r, lift_b, lift_g, lift_r, temp, tint | Find-or-create the clip's Primaries (lift/gamma/gain colour wheels + temp/tint) effect and set given fields (unset fields keep their current value). |
| `color.qualifier` | Mutate | clip_id, hue, hue_width, lum_max, lum_min, sat_max, sat_min, softness | Find-or-create the clip's Qualifier (HSL-band secondary key) effect and set given fields. |

**`container.*`** (5 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `container.add` | Mutate | at, duration, label | Add a container clip pair (video slot + audio slot) at a time. |
| `container.list` | Read | - | List all container clips on the timeline. |
| `container.make` | Mutate | clip_ids | Convert clips to containers (slots). |
| `container.replace` | Mutate | asset_id, clip_id, pair | Replace media in a container clip (effects, transforms, keyframes preserved). |
| `container.unmake` | Mutate | clip_ids | Remove container flag from clips. |

**`export.*`** (9 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `export.bake` | Job | clip_ids | Render the given clip(s) in place (effects flattened into a new asset) and swap them onto it; the original asset stays in the library. Blocks until the swap lands. |
| `export.denoise` | Job | clip_ids | Bake the clip(s) through ffmpeg afftdn spectral audio denoise, then swap them onto the result. |
| `export.presets` | Read | - | List platform export presets (name, ext, size, crf, loudnorm). |
| `export.queue` | Ui | path, preset, range_in, range_out | Append an export job to the render queue; returns immediately with its queue position. |
| `export.quick` | Job | path, preset | Export with the last-used (or given) preset/options; blocks until the file is written (up to 30 min). |
| `export.slowmo` | Job | clip_ids, factor | Bake the clip(s) through setpts+minterpolate optical-flow slow motion, then swap them onto the result (the clip keeps its length and now shows the first part of the slowed footage — extend its end to reveal the rest). |
| `export.stabilize` | Job | clip_ids | Bake the clip(s) through ffmpeg deshake, then swap them onto the result. |
| `export.status` | Read | - | Current export progress (fraction, status, ETA seconds) and the queue / bake counts. |
| `export.video` | Job | crf, encoder, height, path, scaler, width | Export the timeline to a file (blocks until done, up to 30 min); encoder/crf default to settings. |

**`frame.*`** (2 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `frame.export` | Read | height, path, quality, resize, t, width, with_effects | Save the frame at time t as PNG/JPG/WebP (by the path's extension). |
| `frame.stats` | Read | t | Histogram/percentile/mean of the frame at t (256x144 downsample), sourced from the live preview render path; {} when the GPU or the requested frame is unavailable. |

**`gallery.*`** (3 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `gallery.apply` | Mutate | clip_ids, intensity, name, tab | Apply a Gallery card (Look/LUT/Caption/SpeedRamp/Transition) to clip_ids (default selection); Captions is project-wide. Templates place at the playhead instead — use templates.apply. |
| `gallery.hover` | Ui | name, tab | Set/clear the monitor's alt-render preview to a Gallery card (name:null clears). UI-only, no mutation, no undo. |
| `gallery.list` | Read | tab | Card names in one Gallery tab (Looks\|Luts\|Captions\|SpeedRamps\|Transitions\|Templates), or every tab when omitted. Looks excludes saved node-graph presets and reuses color-engine's builtin_looks() — the sole tool surface for Looks. |

**`help.*`** (1 tool)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `help.changelog` | Read | - | Current app version and the full CHANGELOG.md text (what the What's New window shows). |

**`history.*`** (2 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `history.list` | Read | limit | Undo-stack entries with their lazily-derived labels and categories, newest first. |
| `history.restore` | Mutate | index | Restores the project to an undo-stack snapshot by index (0 = oldest); refuses Layout entries. |

**`hotkeys.*`** (3 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `hotkeys.get` | Read | - | Every action id, label, chord text and section. |
| `hotkeys.preset` | Ui | name | Apply a keymap preset. |
| `hotkeys.set` | Ui | action, chord | Rebind one action. Empty chord unbinds. |

**`inspector.*`** (1 tool)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `inspector.folds` | Ui | open, section | Set one inspector section's remembered open/closed state (Settings.inspector_folds). No undo — Settings-level, like other UI prefs. |

**`labels.*`** (2 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `labels.list` | Read | - | Colour labels of the project (index is 1-based; 0 = none). |
| `labels.set` | Mutate | color, index, name, remove | Rename / recolour a label (index), add one (no index) or remove one (index + remove=true). |

**`layout.*`** (6 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `layout.list` | Read | - | Current layout mode, workspace (and every workspace name), pinned panes, the maximised pane, and each pane's visibility / popped-out state. |
| `layout.maximize` | Ui | pane | Maximise a pane to the full tile (the arrangement is stashed), or omit 'pane' to restore it. |
| `layout.mode` | Ui | mode | Set the layout mode: 'dynamic' (a selection surfaces the pane that edits it) or 'granular' (panes stay put, the helpful tab only glows). Re-evaluates the current selection right away. |
| `layout.pin` | Ui | on, pane | Pin (on=true) or unpin a pane against selection-driven auto-surfacing: a pinned active tab keeps its group from switching, and a pinned pane is never switched to (it glows instead). |
| `layout.surface` | Ui | force, pane | Reveal a pane now, the pin-aware way (result: Shown \| Pinned \| Hidden \| Absent — Pinned/Hidden mean the tab was NOT switched); force=true ignores pins and re-opens a hidden pane like the View menu. |
| `layout.workspace` | Ui | name | Switch to a named workspace (see layout.list's 'workspaces') — the same undo-preserving swap as Alt+1..6 / the menu-bar strip. |

**`library.*`** (2 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `library.columns` | Ui | columns | Get/set Settings.library_columns — the cells a list row shows after the name, in order. |
| `library.select` | Ui | ids, paths | Set the library selection (asset ids and/or file paths); omit both to clear it. |

**`looks.*`** (2 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `looks.apply` | Mutate | clip_id, intensity, name | Apply a Look (built-in or user-saved) to a clip, replacing its effect stack, scaled by intensity (0 = no-op, 1 = the Look unmodified). Same resolution/result as gallery.apply(tab=Looks) for the same name. |
| `looks.list` | Read | - | Names of every Look: the 12 built-ins plus any user-saved (non-graph) Settings.effect_presets — same list gallery.list(tab=Looks) returns. |

**`markers.*`** (5 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `markers.add` | Mutate | clip_id, duration, label, name, note, t | Add a marker at a timeline time (on a clip with clip_id). |
| `markers.export` | Read | format, path | Write every marker in timeline order to a CSV or a YouTube-chapters text file. |
| `markers.import` | Mutate | path | Add project markers from a CSV file (time,name[,note,label] — or markers.export's own header form). |
| `markers.list` | Read | - | Every marker in timeline time (project markers + clip markers). |
| `markers.remove` | Mutate | id | Remove a marker by id. |

**`media.*`** (15 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `media.batch_convert` | Read | ext, height, ids, scaler, width | Convert every id with the same options (the scripted batch path — the Convert… window stays single-target); starts one job per asset and returns their output paths at once. |
| `media.consolidate` | Mutate | dir | Copy every asset from outside dir (default: the project's own folder) into it and repoint the paths — both halves run here, synchronously, as one undo step. |
| `media.convert` | Job | ext, height, path, scaler, width | Convert a file with ffmpeg (gif/mp4/mov/mkv/webm/mp3/wav…); returns the output path when done (blocks up to 10 min). |
| `media.import` | Read | paths | Import media files into the library; returns asset ids. |
| `media.import_sequence` | Job | fps, path | Detect the numbered still run `path` belongs to (3+ frames, same prefix/extension) and bake it to one video asset in the background; fires the `import` hook once when it lands. |
| `media.list` | Read | - | Library assets (id, path, kind, duration, size, tags, label, folder, description, used). |
| `media.relink` | Mutate | dir, ids | Relink offline assets: by file name, then by duration within one frame, inside dir (+ one level of subfolders); returns {relinked, still_missing}. |
| `media.scene_cuts` | Mutate | as_markers, clip_id, split, threshold | ffmpeg select='gt(scene,T)' shot-change detection over the clip's source. Neither flag: pure detection, returns cut times (source secs). as_markers adds one point marker (duration 0) per cut and fires marker_added; split cuts the clip at each one. Synchronous (blocks the UI thread for the ffmpeg pass) — documented ceiling. |
| `media.set` | Mutate | description, folder, id, label, tags | Edit asset metadata. |
| `media.set_effects` | Mutate | asset_id, effects | Replace an Asset's master effects (applied to every clip using it, prepended before clip-local effects). Effects: [{kind:string,params:object}], same shape as clip.add_effect. |
| `media.smart_bin` | Mutate | id, name, op | Manage Project.smart_bins: save the current library filter under a name, apply one (by index or name) to the library, list them, or delete one. |
| `media.status` | Read | id | AssetStatus per asset id (Ready \| Decoding \| Offline \| ProxyBuilding, with percent) from the same tick that drives the library badge. |
| `media.subclip` | Mutate | asset_id, in, name, out | Create a subclip asset (a named in/out range into an existing library asset). |
| `media.transcribe` | Job | asset_id, clip_id, model | Transcribe a clip (or the first timeline clip of an asset) with whisper — same job as transcribe.run without generating cues; fills Project.transcripts when done (read with media.transcript). |
| `media.transcript` | Read | asset_id, clip_id | {clip_id, words:[{start,end,text}], text} for one clip (clip_id, or asset_id's first timeline clip), or every transcript when both are omitted. A clip without one gives empty words plus a hint. |

**`mixer.*`** (1 tool)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `mixer.meters` | Read | bus_id | Peak (L/R, linear + dBFS) and approximate LUFS (momentary, integrated; null before any audio) for one or every bus, from the blocks playback has published since the last frame. |

**`multicam.*`** (3 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `multicam.create` | Mutate | clip_ids, name | Sync + nest clips into a multicam sequence, one video track per angle (capped at 4 for the grid). |
| `multicam.switch` | Mutate | angle, seq_clip_id, t | Switch the active angle from time t onward (split + enabled toggles, editable afterward). |
| `multicam.sync` | Read | clip_ids | Dry-run: compute cross-correlation offsets for a set of clips without creating a sequence. |

**`notes.*`** (2 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `notes.get` | Read | - | Free-form project notes. |
| `notes.set` | Mutate | append, text | Replace the project notes (append=true to append a paragraph). |

**`onboarding.*`** (1 tool)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `onboarding.reset` | Ui | - | Re-arm the first-run welcome wizard (settings.onboarded=false) and open it now. |

**`plan.*`** (4 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `plan.add` | Mutate | assets, notes, parent, title | Add a planner item (optionally under a parent). |
| `plan.get` | Read | - | Planner tree + notes. |
| `plan.remove` | Mutate | id | Remove a planner item. |
| `plan.set` | Mutate | done, id, notes, title | Update a planner item. |

**`playback.*`** (9 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `playback.loop` | Ui | in, on, out | Enable/disable Loop In->Out playback. |
| `playback.pause` | Read | - | Pause playback. |
| `playback.play` | Read | - | Start playback. |
| `playback.play_range` | Ui | mode | Play In->Out, Play Around Playhead, or Play to Out, auto-stopping at the target. |
| `playback.rate` | Ui | rate | Set shuttle/playback rate (starts playing if paused); -8..8, 0 rejected (use playback.pause). |
| `playback.scrub` | Ui | t | Emit one BLOCK (~21ms) of audio at t without moving the clock (paused only). |
| `playback.seek` | Read | t | Move the playhead. |
| `playback.status` | Read | - | Current rate, dropped-frame count, buffering flag, and loop range. |
| `playback.step` | Ui | frames | Step the playhead by N frames (negative = back); pauses first. |

**`playhead.*`** (2 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `playhead.get` | Read | - | Current playhead time in seconds. |
| `playhead.set_timecode` | Ui | text | Parse a timecode/relative string and seek — same parser as the transport label's click-to-edit. |

**`preview.*`** (4 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `preview.compare` | Ui | mode, x | Set the monitor's grade-compare mode (state only — no bypass render exists yet, see the PR body). |
| `preview.drop` | Mutate | asset_ids | Place assets on a free video track at the playhead, as if dropped on the monitor. |
| `preview.hover` | Ui | kind, name | Manually drive the monitor's alt-render preview. Same App.alt_render pipeline inspector-gallery's gallery.hover targets via AltRequest::Gallery. |
| `preview.view` | Ui | fit, pan_x, pan_y, zoom | Read or set the canvas zoom/pan. |

**`project.*`** (8 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `project.autosave` | Ui | force | Force-writes the current project to the autosave folder now, regardless of the debounce; returns the path written. |
| `project.get` | Read | - | Full project JSON (the .sedit document). |
| `project.new` | Read | fps, height, width | New empty project (discards unsaved changes). |
| `project.open` | Read | path | Open a .sedit project or a media file (creates a project around it). |
| `project.recover` | Mutate | path | Loads an autosave/backup as the live project. Omit path for the newest candidate. |
| `project.save` | Read | path | Save the project (.sedit). Without a path: the current project file (error if none). |
| `project.set` | Mutate | fps, height, name, width | Change project format/name. |
| `project.summary` | Read | - | Project overview: format, duration, tracks, clips per track, assets, sequences, subtitles count, planner progress, notes — and the markdown style summary. |

**`render.*`** (4 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `render.frame` | Read | t, width | Render the timeline at time t as a PNG (base64 data url), max `width` px wide (default 640). |
| `render.layers_async` | Ui | max_w, t | Queue a non-blocking one-shot layer decode; returns a request id. |
| `render.poll_layers` | Read | id | Poll for the async layer-decode reply (not ready until it matches the given id, or it was superseded by a newer request). |
| `render.range` | Ui | a, b | Pre-render [a, b) into the movie-mode cache without touching the in/out points. |

**`scopes.*`** (1 tool)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `scopes.read` | Read | kind | Current-frame histogram/percentile/mean statistics (waveform/parade/vectorscope share the same summary — see the tool's own note). |

**`scripts.*`** (2 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `scripts.list` | Read | - | Luau scripts with @name/@desc/@hotkey/@on metadata. |
| `scripts.run` | Mutate | event, name | Run a script by name (file stem). Pass 'event' to test-run it as if an @on hook fired with that JSON payload (its own @budget_ms applies); omit 'event' to queue a plain run, same as picking it from the Scripts menu. |

**`selection.*`** (2 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `selection.get` | Read | - | Current clip/transition selection ids and the dominant SelectionKind. |
| `selection.set` | Ui | clip_ids, transition_ids | Replace the current selection (UI state, not a project edit — no undo). |

**`sequence.*`** (2 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `sequence.list` | Read | - | Sequences (id, name, format, duration). |
| `sequence.open` | Read | id | Edit a sequence (swap it into the timeline); omit id to go back to the main timeline. |

**`settings.*`** (2 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `settings.get` | Read | key | Read whitelisted scalar settings (omit 'key' for all of them). |
| `settings.set` | Ui | key, value | Set one whitelisted scalar setting ('value' is a stringified scalar). |

**`shapes.*`** (1 tool)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `shapes.add` | Mutate | at, duration, fill, height, kind, sides, stroke, stroke_width, width | Add a vector shape clip. |

**`source.*`** (7 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `source.focus` | Ui | - | Give the Source monitor transport focus (Space/JKL/I/O route there) and bring the pane forward. |
| `source.get` | Read | - | The Source monitor: asset id, path, duration, fps, marks, playhead, playing, tape, focused (null when empty). |
| `source.insert` | Mutate | at, mode, track | Insert the open source clip's marked range into the timeline (three-point edit). |
| `source.mark` | Ui | in, out | Set the source in/out marks (source seconds); omit a field to clear that mark. UI state only. |
| `source.open` | Ui | asset_id, path, seek | Open a library asset (or any media file path) in the Source monitor; it takes transport focus. |
| `source.subclip` | Mutate | name | Create a library subclip asset from the current source in/out marks. |
| `source.tape` | Ui | asset_ids, off | Build/refresh the Source Tape (the bin laid end to end) from asset_ids; omitted = the library selection, else every asset. off:true returns to the single clip. |

**`stills.*`** (2 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `stills.apply` | Mutate | clip_id, name | Paste a saved still's effect stack onto a clip (engine::presets::apply_effects, one undo). |
| `stills.save` | Ui | clip_id, name | Snapshot a clip's effect stack as a named preset (a 'Look') into Settings.effect_presets. |

**`style.*`** (1 tool)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `style.summary` | Read | - | Markdown style summary of the project (how it was edited) — for writing style guides. |

**`subtitles.*`** (5 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `subtitles.animation` | Mutate | color, kind | Set how generated captions animate against the transcript: none \| highlight (color [r,g,b,a]) \| pop \| typewriter. |
| `subtitles.get` | Read | - | Subtitle cues. |
| `subtitles.import` | Mutate | path | Import .srt/.vtt (replaces). |
| `subtitles.set` | Mutate | cues | Replace all cues: [{start,end,text}]. |
| `subtitles.style_preset` | Mutate | name | Set project.subtitle_style from a named caption style (builtin or Settings.caption_presets). |

**`templates.*`** (4 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `templates.apply` | Mutate | at, name | Place a saved template at a time. |
| `templates.expose` | Ui | fields, name | Rewrite a saved user template's captured clips, setting Clip.exposed on each addressed clip_index (position within the template's own clip list, not a live id). Settings-level (Settings.templates) — ToolKind::Ui, not Mutate. |
| `templates.list` | Read | - | Saved clip templates and motion presets. |
| `templates.save` | Ui | clip_ids, name | Save the given (or currently selected) clips as a reusable effect chain / node graph / clip template in settings.json (mirrors the deleted presets_ui.rs "Save from selection" button). |

**`text.*`** (1 tool)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `text.animate` | Mutate | clip_id, merge, preset | Apply a motion preset's keyframes to a clip's Position/Scale/Rotation/Opacity — apply_motion (replace, stretched to the clip's length) or merge_motion (merge, layered on from the playhead) when merge is true. |

**`timeline.*`** (49 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `timeline.add_clip` | Mutate | asset_id, at, duration, sequence_id, text, track | Place an asset, a sequence, or a new text clip at a time. |
| `timeline.add_transition` | Mutate | duration, kind, right_clip_id | Transition at the cut on the left of a clip (a fade-in from nothing when no clip abuts there). |
| `timeline.auto_cut` | Mutate | clip_ids, keep_quiet, min_silence, min_speech, padding, ripple, threshold_db | Silence-based auto-cut of audio clips (+ linked video). |
| `timeline.clips_at` | Read | t | Clip ids covering a time. |
| `timeline.close_gap` | Mutate | t, track | Close the gap under (track, t) — ripple tracks only. |
| `timeline.delete` | Mutate | clip_ids, ripple | Delete clips (linked clips follow). |
| `timeline.dupes` | Read | - | Groups of clip ids sharing (asset, src_in..src_out) — duplicate-source detection. |
| `timeline.duplicate` | Mutate | clip_ids | Duplicate clips (default: selection) onto a free track each. |
| `timeline.dynamic_trim` | Mutate | dt, edit_point_side | Apply the same ripple/roll composition dynamic (JKL) trim performs on a rate-drop, without shuttling. |
| `timeline.edit_point` | Ui | clear, side, t, track | Get/set/clear the selected cut for keyboard trimming (U / Shift+U). Omit track+t to just read it. |
| `timeline.extend` | Mutate | side, t, to, track | Extend an edit point to a time (rolls a shared cut, ripple-trims a single side). |
| `timeline.extract` | Mutate | a, b, tracks | Remove a range and close the gap (ripple tracks by default). |
| `timeline.find` | Read | query | Search clip names, marker names/notes, subtitle cues and sequence names (case-insensitive substring). |
| `timeline.import` | Read | path, replace | Import a timeline from another editor (FCP7 XML, EDL, .prproj); returns the report and opens it in the app (replace=true swaps the project in). |
| `timeline.in_out` | Read | - | Current in_point/out_point (null if unset). |
| `timeline.join` | Mutate | clip_id | Merge a clip with its right neighbour if contiguous/same asset. |
| `timeline.keyframe_nav` | Ui | direction | Seek to the previous/next keyframe of the selected clip. |
| `timeline.lift` | Mutate | a, b, tracks | Remove a range, leaving a gap. |
| `timeline.list` | Read | - | Tracks and clips of the timeline being edited (main or the open sequence): ids, kind, asset, start, duration, src_in, speed, effects, label. |
| `timeline.magnetic_move` | Mutate | clip_ids, dt, dtrack | Move clips; a blocked move onto a magnetic track opens space first instead of refusing. |
| `timeline.mark` | Mutate | clip_id | Set in/out to a clip's [start,end) — defaults to the clip under the playhead. Returns {in,out}. |
| `timeline.match_frame` | Ui | clip_id | Open a clip's source asset in the Source monitor at the source time under the playhead. |
| `timeline.move` | Mutate | clip_ids, dt, dtrack | Move clips by dt seconds (and dtrack tracks within their kind). |
| `timeline.nest` | Mutate | clip_ids, name | Nest clips into a new sequence; returns the sequence id. |
| `timeline.overview` | Ui | enabled | Show/hide the inline overview minimap strip. |
| `timeline.overwrite` | Mutate | asset_id, at, in, out, track | Overwrite edit: no ripple. |
| `timeline.pacing` | Read | long_s, short_s | Clip ids/spans outside the boring-detector thresholds (too long / too short). |
| `timeline.place` | Mutate | asset_id, at, in, mode, out, track | Place an asset with a DropMode: place (free track), splice (ripple-insert), overwrite (on a clip body = replace edit keeping duration/effects), top (new track above). |
| `timeline.reframe` | Mutate | clip_id, ratio | Auto-reframe using the clip's EXISTING tracked box only (no auto-detect). |
| `timeline.replace` | Mutate | asset_id, clip_id | Swap a clip's asset, keeping duration/effects/transform. |
| `timeline.ripple_trim` | Mutate | clip_id, edge, ripple, start | Trim one edge of a clip; ripple=true shifts downstream ripple-tracked clips (end edge) or just carries markers/cues (start edge). |
| `timeline.roll` | Mutate | cut, right_clip_id | Move a shared cut; total length of the two clips is unchanged. |
| `timeline.select_forward` | Read | backward, t, track | Clip ids from a time forward, or backward. |
| `timeline.set_in_out` | Mutate | in, out | Sets in/out, snapped, clamped in<=out; one undo pushed only if changed. |
| `timeline.shift_time` | Mutate | dt, from, tracks | Shift markers, main-timeline subtitle cues, and in/out at/after `from` by `dt` seconds. |
| `timeline.slide` | Mutate | clip_id, dt | Move a clip; its immediate neighbours absorb the change. |
| `timeline.slip` | Mutate | clip_ids, dsrc | Change the source window in place (start/duration unchanged). |
| `timeline.smart_edit` | Mutate | kind | One of the four smart edits at the playhead using the open source clip's marked range. |
| `timeline.snap_get` | Read | - | Current snapping-enabled state. |
| `timeline.snap_query` | Read | exclude_ids, t | Runs the tiered snap engine (playhead > cursor > selected edge > adjacent edge > marker > transition edge > in/out > 0) and returns the hit and its tier. |
| `timeline.snap_set` | Ui | enabled | Toggle snapping (mirrors the bare-S hotkey); not project data, no undo. |
| `timeline.splice` | Mutate | asset_id, at, in, out, track | Insert edit: ripple-opens space then places the asset. |
| `timeline.split` | Mutate | clip_ids, t | Split clips at t (all clips crossing t when clip_ids is omitted). |
| `timeline.trim` | Mutate | clip_id, end, start | Trim a clip's edges to new timeline times. |
| `timeline.trim_edges` | Mutate | dt, edges, ripple | Asymmetric multi-roller trim: every listed edge moves by the same dt, all-or-nothing. |
| `timeline.trim_view` | Ui | on | Show/hide the dual-frame trim view (only actually paints once an edit point is also selected). |
| `timeline.unnest` | Mutate | clip_id | Flatten a Sequence clip back onto the timeline at its original positions. |
| `timeline.view_preset` | Mutate | clip_text, keys, name, thumbs, waves | Get or set the active timeline view preset (row heights/element toggles). |
| `timeline.zones` | Read | x, y | Debug hit-test: which arm.rs Zone a point would land on (Body/BodyBottom/Edge/Seam/Lane/RulerInOut/...). |

**`titles.*`** (2 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `titles.list` | Read | - | List placeable title/text templates: builtin_titles() + Settings.templates filtered by is_text_template. Returns [{name, clip_count, exposed: [string]}]. |
| `titles.place` | Mutate | at, name | Decode the named title template (builtin or Settings.templates) and place it at `at` (or the playhead). Returns {clip_ids, exposed}. |

**`track.*`** (4 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `track.list` | Read | - | Every track: index, kind, name, locked, ripple, magnetic, color, clip count. |
| `track.move` | Mutate | index, up | Reorder a track one slot up/down within its own kind (video tracks / audio tracks stay contiguous). |
| `track.set` | Mutate | color, index, locked, magnetic, name, ripple | Edit one track's flags/name/colour. |
| `track.volume_key` | Mutate | db, remove, t, track_index | Set (db) or remove (remove=true) a keyframe on a track's volume automation at t. |

**`tracking.*`** (1 tool)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `tracking.run` | Job | apply, backward, clip_id, cx, cy, hh, hw, search | NCC point-track a region of a clip (job): box centre (cx,cy) and half-size (hw,hh) in canvas px relative to the centre. With apply (default true) the path is written as the clip's X/Y keyframes when done. |

**`transcribe.*`** (2 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `transcribe.install` | Job | model | Download a whisper model into the cache (job; 75–466 MB). Already downloaded = done at once. |
| `transcribe.run` | Job | clip_id, cues, language, model | Transcribe a clip's audio with whisper (background job; word timings always on). On completion the words are in Project.transcripts, and with cues=true (default), captions are generated. |

**`transcript.*`** (6 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `transcript.cut_words` | Mutate | clip_id, ranges, word_indices | Ripple-cut timeline ranges (or the given word indices) out of a transcribed clip; cues, markers and words shift along. |
| `transcript.export` | Read | clip_id, format, path | Write a clip's transcript to a file as txt (plain), srt (sentences → cues) or json ({clip_id, words}); format defaults to the path's extension. |
| `transcript.get` | Read | clip_id | Word timings (timeline seconds) for one transcribed clip, or every transcript when clip_id is omitted. |
| `transcript.remove_fillers` | Mutate | as_markers, clip_id, dry_run, pad_ms, words | Find filler words in a transcribed clip and ripple-cut them. dry_run returns the ranges only; as_markers drops a range marker per hit instead. |
| `transcript.search` | Read | query | Word hits across every transcribed clip: [{clip_id, index, t}] (case/punctuation-insensitive; a phrase matches consecutive words). |
| `transcript.set` | Mutate | clip_id, words | Write (replace) a clip's transcript: words as [{start,end,text}] in timeline seconds; an empty list removes it. |

**`tts.*`** (1 tool)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `tts.speak` | Job | at, clip_id, text, voice | Windows SAPI text-to-speech (job): writes a WAV and imports it at `at` (default: the playhead), linked to the text clip `clip_id` when given. OS voices only. |

**`ui.*`** (6 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `ui.action` | Ui | args, id | Dispatch a UI Action by id (see ui.actions). Toasts App::enabled's reason and no-ops when disabled. |
| `ui.actions` | Read | - | List every Action: id, label, default chord text, whether it's currently enabled. |
| `ui.confirm_pending` | Read | - | Titles/bodies of currently-open non-blocking confirm windows, for a script that must wait on user input rather than racing it. |
| `ui.palette` | Read | query | List palette rows (Actions, Panes, arg-free Tools, Scripts, Workspaces), optionally fuzzy-filtered by 'query'. |
| `ui.toast` | Ui | kind, text | Shows a toast identical to the app's own (dedupes against the last toast with the same text). |
| `ui.zoom_factor` | Ui | value | Get/set the UI zoom factor (0.5-2.5); omit 'value' to read. |

### A note on `gen_docs.luau`

`scripts/gen_docs.luau` (next to this doc, copy into `%APPDATA%\SimpleEditor\scripts`) is a live
spot-check, not a way to (re)generate the table above. It calls `editor.tools()`, which only returns
`{name, description}` — no `kind`, no `args` — because that's genuinely all `editor.tools()`'s Lua
bridge exposes (`src/scripting.rs`); it isn't a documentation shortfall, it's the bridge's real
shape. Run it from the Scripts menu, eyeball the toast's name count and a few names against this
table before it auto-expires (5-10 s, no copy button), and treat any mismatch as a signal the table
above has drifted since a newer tool landed — not as a way to regenerate the table itself. See the
"Deliberate simplifications" note in `plans/ui-overhaul/issues/docs-refresh.md` for the full
reasoning and the documented `docs.tools_md` upgrade path (not built).

## Keymaps

Defaults live in `src/hotkeys.rs`. Rebind anything in Settings ▸ Hotkeys, or over MCP
(`hotkeys.set`). Named presets (`hotkeys.preset`, or the Hotkeys tab's preset combo) are **small diff
tables over the defaults**, not full remaps — real per-app accuracy wasn't the point, a small,
genuinely-differing diff per app was (`src/keymaps.rs`'s own doc comment):

| Preset | Rebinds |
|---|---|
| Simple Editor (default) | — (this preset is exactly `Hotkeys::reset_all()`) |
| Premiere | `split` → `Ctrl+K` (Premiere's own "Add Edit" muscle memory), `command_palette` → `Ctrl+Shift+P` (moved off Ctrl+K), `export` → `Ctrl+M` |
| Resolve | `split` → `B`, `command_palette` → `Ctrl+Space`, `toggle_transitions` → `Ctrl+Shift+T` |
| Avid | `command_palette` → `Ctrl+Alt+K`, `split` → `Ctrl+Shift+B` |

`Ctrl+K` opens the **command palette** over every `Action`, `Pane`, arg-free `ToolDef` (`:` opens a
mini argument form built from the tool's own arg docs), Luau script and workspace — fuzzy-searchable,
recency-ordered. `F1` opens the **cheat sheet** (a second palette entry point, listing every bound
chord). See ARCHITECTURE.md's "Gesture model" table for the timeline's mouse+modifier chords — that
table is the authoritative source; this doc doesn't duplicate it.

## Layout

`Settings.layout_mode` is `"dynamic"` (a selection auto-surfaces the pane that edits it — e.g.
selecting an audio clip switches a tabbed group to Inspector's audio section) or `"granular"`
(panes stay put; the relevant tab just glows instead of switching) — chosen at first run, changeable
later via `layout.mode` or Settings. Six named **workspaces** (`Simple`, `Edit`, `Color`, `Audio`,
`Text`, `Deliver`) are one keystroke away on `Alt+1`.."`Alt+6`" (`WORKSPACES` in `src/ui/layout.rs`);
every workspace places every `Pane::ALL` member somewhere (visibly or tab-stacked), so a stored
layout is never missing a pane. Any panel can be **pinned** (`layout.pin`) to opt out of
selection-driven auto-surfacing without turning off Dynamic mode globally. Panes can be **popped
out** into their own OS window (hotkeys are polled on the popped-out viewport too, so Space/J/K/L
keep working there). Layout profiles and themes are shareable files: `.sedit-layout` (a layout
profile) and `.sedit-theme` (a theme, exported/imported from Settings ▸ Appearance).

## The `lite` feature set (documented, not built)

A `--no-default-features --features lite` build that drops scripting, tracking, the planner, color
grading, titles and multicam was scoped as a size-budget escape hatch (projected ≈9.5 MB) — but it
is **documentation only**. No `Cargo.toml` feature gate, no `cfg` attribute, and no CI build for it
exist in the tree. If a future size crunch makes this worth actually building, the cut points are
the modules named above (`src/engine/tracking.rs`, `src/engine/presets.rs`'s template half,
`src/ui/nodes.rs`'s color-graph usage, `titles.*`/`multicam.*`'s MCP tools and their UI) — not a
promise that the cut is trivial, just where the plan expected the boundary to fall.
