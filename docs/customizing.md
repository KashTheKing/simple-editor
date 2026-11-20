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

### Assistant scripts (`scripts.eval`)

The Assistant (or any MCP client) can run a Luau script **straight away** with `scripts.eval {code, purpose}`
(`src/ui/app/script_ctl.rs`, runner `scripting::run_api`). It is how an agent organises a library of hundreds of
assets, audits a timeline or makes many edits in one go without reading 60 KB of JSON.

- **One undo step**, labelled `Script: <purpose>` (inside an Assistant request it joins the request's step). If the
  script errors, every edit it made is undone. `dry_run: true` runs it, reports, then undoes it.
- **Output**: `print(...)` (tables print as JSON) and the chunk's `return` value come back, capped at `max_output`
  characters (default 16 000) with a "truncated: refine your query" note.
- **API** (the prelude in `src/scripting.rs`): `editor.tool(name, args)` for any project / library tool,
  `library.assets(filter)` (every field: id, name, path, ext, kind, duration, width, height, fps, codec, size, tags,
  label, folder, description, used, uses), `library.get / summary / set / update / move / tag`, `library.bins /
  create_bin / rename_bin / move_bin / delete_bin`, `library.linked / linked_files` (read-only listing of linked
  folders), `project.get / summary / timeline / clips / tracks / markers / subtitles / sequences / spans / blocks`,
  and `query.filter / find / map / pluck / ids / sort / group_by / count_by / sum / keys / has`.
- **Sandbox**: no `io`, no `require` / `package` (mlua's Luau `require` would read files), no `loadstring` /
  `getfenv` / `setfenv`, an `os` cut down to `clock / time / date / difftime`. `editor.tool` admits project and
  library tools only (`script_may_call`): Read / Mutate tools, none that take a path, folder-on-disk or URL argument,
  none of `export.*`, `project.open / new / save`, `settings.*`, `scripts.*`, `tasks.*`, `skills.*`, `assistant.*`,
  `ui.*`, the content library's downloads, imports / exports to files. The only file-system effect is listing a
  linked folder the user added (`media.linked`), read only, never outside it. Asset paths are strings.
- **Budgets**: `budget_ms` wall-clock (default 10 s, at most 60 s), 20 million interpreter steps, 256 MB of memory.
- **Guard**: because a script cannot leave the project, `scripts.eval` is **not** held for the Allow prompt even with
  Settings ▸ Assistant's guard on - the user asked for library-wide scripting without a prompt per run. Scripts with
  full access stay held: `scripts.run` (the user's own script files) and `tasks.run`.

The library tools work without a script too: `media.summary` (counts by kind / extension / bin / used, the bin tree,
in a few KB), `media.list {kind, folder, nested, used, query, tags, label, ext, untagged, ids, fields, sort, limit,
offset}`, `media.bulk_set {ids | filter, folder, tags, add_tags, remove_tags, label, description}` and
`library.bins {action: list | create | rename | move | delete, bin, to}`. The built-in skill `/organize-library`
uses them end to end.

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

**Verified against a live `tools/list` on the 1.2.0 release build (2026-11-12): 298 tools. Added since (each listed in its feature's rows below): `media.import_folder`, `gallery.tab` and the `where` / `skip_existing` arguments of `gallery.apply` and `library.apply`, the HTML renderer, thread and social-guides tools, and 7 browser-layer tools (21 rows folded into `action` arguments). The merged integration/nov13 build lists 329 (live `tools/list`, 2026-11-13).** The tables below
were written per feature as it landed; if a row disagrees with `tools/list`, `tools/list` wins.

> **Pro plan (built and live-tested, 1.2.0):** the rows it added or changed
> (`assistant.*`, `edit.*`, `editor.*`, `voice.*`, `export.edl`, `export.xml`, `markers.comment`,
> `markers.set`, `playback.hud`, `render.contact_sheet`, `transcribe.status`, and new arguments on
> `export.queue`, `export.video`, `markers.add`, `markers.list`, `markers.export`, `media.transcribe`,
> `transcribe.run`) are listed below.
> With `Settings.assistant_guard` on (default), `export.*`, `project.open`, `project.new`,
> `settings.set`, `scripts.run`, `tasks.run`, `media.convert` and `media.batch_convert` called over MCP
> wait for the user's Allow (`ui/app/assistant_ctl.rs`, `leaves_project`; `profile.fetch` too). `scripts.eval` is not
> held: it reaches the project and library only (see "Assistant scripts" above). The Assistant redesign added
> `assistant.add_selection`, `assistant.attachments`, `assistant.attachment` and the five `tasks.*` tools.

> **Creator Kit (1.2.0).** `skills.save` and `skills.delete`
> are also held for the user's Allow. `library.*` below is the CONTENT library (transitions, effects,
> LUTs, HTML templates, icons, skills); `library.select` / `library.columns` are the media library panel.

| Tool | Kind | Arguments | What it does |
|---|---|---|---|
| `html.create` | Mutate | html, at?, duration?, name?, fields? | Add an HTML + CSS layer or scene clip (format: `library/README.md`); rendered in the background. `fields` is an object `{name: value}`. |
| `browser.add` | Mutate | at?, duration?, fields?, fit? | Add a browser window layer above what is there: the tracks below show inside it. Returns `{clip_id, max_scroll}`. |
| `browser.set` | Mutate | clip?, fields? | Without `fields`: read `{clip_id, start, end, style, page}` (the page area in cursor coordinates). With `fields`: change any key of that style (`display` `window` / `page`, `theme`, `tabs`, `url`, `bg`, `scroll`, `zoom`, `cursor: {...}`...); a number for a keyframable field is a constant. |
| `browser.animate` | Mutate | clip?, preset, at?, duration?, value?, target?, at_cursor?, whole?, x?, y? | Presets as keyframes: `type_url`, `load`, `switch_tab`, `scroll_to`, `zoom_to` (the point held still: an element's text as `target`, the mouse, or `x`, `y`; `whole` zooms the whole browser). |
| `browser.page` | Mutate | clip?, action, at?, target?, nth?, x?, y?, asset?, url?, title?, favicon?, load?, transition?, transition_secs?, type_url?, query?, max? | `action`: `fit` (pictures below page-wide, top at the top), `next` (cut the clips below with a transition, run the load bar, change address / title / icon), `click_go` (mouse move with overshoot, hand, highlight, click, then `next`: one undo step), `elements` (links, buttons and inputs of the imported page with their boxes), `sharpen` (redraw the zoomed parts of an imported page at exactly their zoom; background job), `make_editable` (headings and labels become fields of the page's HTML layer; background job). |
| `browser.import` | Ui | action?, src?, width?, at?, place?, signed_in? | `action`: `import` (default: a real page by address, `.mhtml` or `.html` as an HTML layer clip with an element map; background job), `jobs` (pending page imports / zoom details and what the last one said), `sign_in` (open the sign-in browser window; the user signs in, the tool reads nothing). |
| `cursor.set` | Mutate | clip?, action, at?, target?, nth?, x?, y?, duration?, hover?, hold?, click?, points?, mode?, kind?, value?, press?, tolerance?, from?, to?, samples?, smoothing? | `action`: `move_to` (to an element by its text, or a point, scrolling there first if needed), `path` (`[[t, x, y], ...]`, `stops` or `glide`), `event` (click / double / down / up / scroll / key / shape), `press` (the clip under the cursor dips and springs back), `tidy` (remove jitter, keys under clicks stay), `samples` (raw pointer samples reduced to keys and events, as Record mouse does). |
| `cursor.record` | Ui | clip?, action, at?, discard? | `action` `start`: Record mouse (the user's real mouse over the viewer); `stop`: write the take (one undo step), or `discard` it. |
| `html.get` | Read | clip | The clip's source, fields with current values, size, duration, render state, lint. |
| `html.set` | Mutate | clip, html?, fields?, params?, width?, name? | Replace the source and/or set field values (`null` = default) and `se:param` values (a number or `[{t,v,ease}]` keyframes); the clip keeps position, effects and keyframes. |
| `html.import` | Mutate | path, at?, link? | Add a `.html` file as a clip, linked to the file (edit either side). |
| `html.renderer` | Mutate | clip, renderer? | Which program draws an HTML layer: `auto` (the fast helper when installed and the layer passes the lint, else Edge), `blitz`, `edge`; reports the engine in use, why, and the CSS the fast renderer cannot draw. |
| `html.helper` | Read | install? | Status of the fast HTML renderer helper (`se-html`, ~11 MB, `%APPDATA%\SimpleEditor\bin`); `install: true` downloads it from the library as a job (only when the user asked). |
| `html.link` | Mutate | clip, path, write? | Link a clip to a `.html` file (read it, or write the clip into it); empty path unlinks. |
| `html.snapshot` | Mutate | url, width? | Snapshot a real web page by address (private Edge, scripts dropped) and add it as a linked clip. |
| `html.render` | Read | clip | Render again when the current source has no render (also retries a failed one). |
| `thread.create` | Mutate | kind?, mode?, style?, script?, patch?, at?, duration?, name? | Add a fake chat / forum / video-comment thread layer made from data. kind: chat (messages only), chat_typing (DM / group DM / server where `me` types into the message box and the others "are typing"), forum, comments, comments_typing (a comment typed, Comment pressed, posted). Styles: Dark / Light / Compact chat, Classic / Card / Dark forum, Light / Dark / Compact comments. |
| `thread.get` | Read | clip | The thread (look, cast, items with ids, post, animation), when each item appears (and when its typing starts), its length, the Comment button's rectangle and press time, and it as script lines. |
| `thread.set` | Mutate | clip, id?, patch?, thread?, mouse? | Patch an item (`id`) or the thread (toggles by name: rail, channels, sidebar, topbar, header, composer, indicator, dividers, unread_line, scrollbar, transparent, mode, me, anim.compose / cps / typos / pause); `thread` replaces everything; `mouse: true` aims the fake mouse at the Comment button. |
| `thread.add` | Mutate | clip, who?, text?, item?, at?, after?, duplicate? | Add a message / comment (`depth` nests a forum reply); a new speaker joins the cast. |
| `thread.remove` | Mutate | clip, id | Remove an item. |
| `thread.move` | Mutate | clip, id, to? \| by? \| depth? \| indent? | Reorder an item, or nest a forum comment. |
| `thread.from_script` | Mutate | clip, script, mode? | Plain lines (`name (4:32 PM): text`, `--- divider`, `> reply`) become items (append or replace). |
| `thread.cast` | Mutate | clip, action?, name?, avatar?, color?, badge?, to? | List / set / remove / rename cast members; their messages follow. |
| `html.check` | Read (image) | clip? \| html?, t?, fields? | One rendered frame as an image, plus lint. |
| `clip.set_shader` | Mutate | clip, index?, src | Set a clip's shader effect source; returns `{ok, log, params}` (a compile error is a result, not a failure). |
| `shader.check` | Read | src, kind? | Compile an effect or transition shader without changing anything. |
| `transition.set_shader` | Mutate | id? \| clip?, src, params? | Make a transition a shader transition (gl-transitions format). |
| `skills.list` | Read | - | Assistant skills: name, description, args, source. |
| `skills.get` | Read | name | One skill's text and parsed parts. |
| `skills.save` | Ui (held) | name, description, args?, body | Write a skill into the user's skills folder. |
| `skills.delete` | Ui (held) | name | Delete a user skill. |
| `library.list` | Read | kind?, query? | Installed items and, once the online library is connected (`gallery.connect`), the online ones, each with `installed`; `catalogue` is `not connected`, `loading`, `ok` or a note. |
| `library.install` | Job | id | Download an item (`transitions/paper-tear`). |
| `library.apply` | Mutate | at, clip, clips, id, track, where | Use a CONTENT library item (not the media library): a transition goes on the clip's cut, an effect or LUT on the clip (default: the selection), an HTML template or icon is added as an HTML layer at `at` (default: the playhead), a skill is installed for the Assistant. An online item must be installed first (library.install). One undo step. |
| `library.save` | Ui | kind, name, text | Save a shader, HTML layer, skill or `.cube` text as a library item (checked first). |
| `library.remove` | Ui | id | Remove an installed item. |
| `guides.list` | Read | - | The social guides: every platform (id, network, aspect, verified date, source, zones as fractions of the frame, how each sheet moves the video), the devices and the current setting. |
| `guides.set` | Ui | platform?, mode?, device?, profile?, worst_case?, keep_aspect?, draw_in_render?, state? | What the viewer's guide shows: platform (`off` hides it), `safe` / `ui` / `device`, device class, the active profile, and which sheet the device preview has open. View only: never the project or the export. |
| `guides.preview` | Read (image) | platform?, device?, mode?, state?, profile?, t?, playing?, width? | A picture of the guide: the frame at `t` inside the device with the platform UI and the sheet open (`state`: `default`, `theatre`, `fullscreen` (desktop long form), `comments`, `desc`, `share`, `more`, `profile`, `sound`), or with the safe-zone / UI overlay. Deterministic. |
| `profile.get` | Read | id?, network? | The social profiles (fake accounts the guide shows) and which is active per network. |
| `profile.set` | Ui | id? \| network?, fields?, avatar_path?, active?, delete? | Create or edit a profile: handle, display, verified, followers, caption, tags, sound, counts, link, `comment_list`; a picture file; make it active. No credentials exist anywhere. |
| `profile.fetch` | Ui | id, url?, apply? | Fill a profile from its PUBLIC page (curl, no login, no cookies): name, picture, follower text. Returns what was found and from where. Held for the user's Allow like `export.*`: it is the only call that reads the web. |

`kind` isn't part of the wire response (`mcp::tools::list_json` only serialises
`name`/`description`/`inputSchema` — see `src/mcp/tools.rs`), so the `Kind` column below is a source
cross-reference (each tool's own `ToolDef.kind` in its `tools_*.rs` file), not something you'll see
in the raw JSON. `Mutate` pushes one undo step iff the project JSON actually changed and rolls back
on error; `Read`/`Ui` never touch the undo stack; `Job` starts a background job and replies when it
finishes. Names, args and descriptions below are transcribed directly from the `tools/list` response
above — not from `editor.tools()`/`editor.log()`, which is a lossy, auto-expiring bridge (see
"Luau scripting" above).

**Totals by kind**: 109 Mutate, 70 Read, 48 Ui, 13 Job.

**`assistant.*`** (7 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `assistant.add_selection` | Ui | what | Pin a snapshot of the user's selection to the Assistant's next chat message and show the pane (same as right-click ▸ Add to Assistant). what: selection (clips, else transcript words, a marker, else In/Out), clips, words, marker or range. what: default selection. |
| `assistant.attachment` | Read | index | One image the user attached to the current Assistant request, as an image (PNG). index: 0-based, see assistant.attachments. |
| `assistant.attachments` | Read | - | The images the user attached to the current Assistant request: [{index, name, width, height}]. assistant.attachment{index} returns one. |
| `assistant.send` | Ui | prompt | Send a request to the Assistant pane's chat, as if the user typed it (one request at a time). |
| `assistant.status` | Read | - | The Assistant pane: whether a request is running, the backend, requests waiting (for assistant.wait, and the queue behind the running one), and the last 20 lines of the shown conversation. |
| `assistant.stop` | Ui | - | Stop the Assistant pane's running request (its edits so far stay, as one undo step). |
| `assistant.wait` | Job | timeout_s | Block until the user sends a request from the Assistant pane or adds a comment (a marker with a note); returns {request, context}, or {timeout: true}. Loop on it for live mode. timeout_s: default 300, max 1500. |

**`audio.*`** (13 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `audio.add_bus` | Mutate | name | Create a bus (it feeds Main until routed elsewhere). |
| `audio.add_filter` | Mutate | bus, kind, params | Add a filter to a bus with optional params {name: value}. |
| `audio.analyze` | Read | clip_ids | Peak/RMS dBFS and onset count/BPM per clip (from cached waveform peaks of the source file), plus `gain_db` (the clip's own volume), `peak_db_out` / `rms_db_out` (levels after it) and `volume_keyed`; 'waveform still computing' if peaks aren't ready yet. |
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
| `clip.crop` | Mutate | at, bottom, clip_id, left, right, top | Set the clip's built-in crop (Crop Left / Right / Top / Bottom, the share of the layer cut off each edge, 0..0.5) at time `at`. Keys it when that property is already keyframed. |
| `clip.effects_bulk` | Mutate | clip_ids, index, params | Set one stack-index effect's params on every listed clip whose effect at that index shares its kind (Project::bulk_set_effect_params). |
| `clip.fit` | Mutate | clip_id, mode | Fit (contain, native aspect) or stretch (fill canvas, non-uniform) the clip. |
| `clip.keyframe` | Mutate | clip_id, ease, property, remove, t, value | Set (or remove with remove=true) a keyframe of a property at clip-local time t. |
| `clip.mask_target` | Ui | effect | Point the existing mask-tool canvas drag at the clip's own mask, or one effect's. |
| `clip.remove_effect` | Mutate | clip_id, index | Remove effect at index. |
| `clip.reorder_effect` | Mutate | clip_id, from, to | Move an effect to a new stack index (Project::reorder_effect). |
| `clip.set` | Mutate | clip_id, fields | Set clip fields: name, enabled, label, speed, reverse, freeze (source time or null), blend, fade_in, fade_out, and constant values of properties (x, y, scale, rotation, crop_left, crop_right, crop_top, crop_bottom (0..0.5), opacity, volume, pan); text clips: text style fields (text, font, size, color [r,g,b,a], outline_width, …). |
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

**`edit.*`** (3 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `edit.batch` | Ui | author, calls, label | Run several tool calls (calls: [{name, args}] in order) in one frame, as one undo step; returns each result. A failing call stops the batch and rolls the whole batch back (the error names the call). label default 'Batch edit', author default Claude. |
| `edit.begin` | Ui | author, idle_s, label | Start one undo step for a request: until edit.end every edit joins it, and the user can undo it all at once ('<author>: <label>' in History). Closes by itself after `idle_s` (5..600, default 30) seconds without calls. |
| `edit.end` | Ui | - | End the undo step edit.begin started: one History entry if anything changed, the changed clips glow, the user gets an Undo toast. |

**`editor.*`** (1 tool)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `editor.context` | Read | - | What the user is looking at: playhead, in/out, selected clips (name, kind, track, times), transitions, subtitle cues, markers (with notes), edit point, selected transcript words (text, start, end), sequence and page. Call it first in every request. |

**`export.*`** (11 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `export.bake` | Job | clip_ids | Render the given clip(s) in place (effects flattened into a new asset) and swap them onto it; the original asset stays in the library. Blocks until the swap lands. |
| `export.denoise` | Job | clip_ids | Bake the clip(s) through ffmpeg afftdn spectral audio denoise, then swap them onto the result. |
| `export.edl` | Read | path | Write the main timeline as CMX 3600 EDL (for Avid): one file per video track (the first at path, others as <stem>_V2.edl …), the first also carrying every audio track; dissolves as D events. |
| `export.preset_apply` | Ui | name | Apply an export preset card (any kind) to the Export pane, as clicking it does. |
| `export.preset_delete` | Ui | name | Delete a custom export preset. Built-in and platform presets refuse. |
| `export.preset_rename` | Ui | color, icon, name, new_name | Rename a custom export preset (optionally a new icon and #rrggbb color, or color none). Built-in and platform presets refuse. |
| `export.preset_save` | Ui | color, icon, name | Save the Export pane's current settings (size, format, quality, encoder, speed, audio) as a custom preset card; Quick Export finds it by name too. Overwrites a custom preset of the same name; built-in and platform names are refused. |
| `export.presets` | Read | - | List every export preset card in order: the 2 built-ins (project size; max quality / fast preview), the platform tiles, then the user's own. width/height 0 = the project's size; kind = builtin \| platform \| custom. |
| `export.queue` | Ui | audio, path, preset, range_in, range_out, stem_by | Append an export job to the render queue; returns immediately with its queue position. audio: mix \| streams \| stems, stem_by: track \| bus \| role (default the Export pane's). |
| `export.quick` | Job | path, preset | Export with the last-used (or given) preset/options; blocks until the file is written (up to 30 min). |
| `export.slowmo` | Job | clip_ids, factor | Bake the clip(s) through setpts+minterpolate optical-flow slow motion, then swap them onto the result (the clip keeps its length and now shows the first part of the slowed footage — extend its end to reveal the rest). |
| `export.stabilize` | Job | clip_ids | Bake the clip(s) through ffmpeg deshake, then swap them onto the result. |
| `export.status` | Read | - | Current export progress (fraction, status, ETA seconds) and the queue / bake counts. |
| `export.xml` | Read | path | Write the main timeline as Final Cut Pro 7 XML (xmeml v5) for Premiere / Resolve: clips, audio level/pan, A/V links, cross dissolves and sequence + clip markers (track names are not part of xmeml). |
| `export.video` | Job | audio, captions, crf, encoder, guide, height, path, scaler, stem_by, width | Export the timeline to a file (blocks until done, up to 30 min); encoder/crf/audio/stem_by default to settings. audio=streams adds one audio track per stem after the mix (mp4/mov/mkv/m4v, else one mix); audio=stems writes '<name> stems/NN <stem>.wav' beside the file. `guide` / `captions` burn in the social guide / subtitles (default: the Export settings). |

**`frame.*`** (2 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `frame.export` | Read | height, path, quality, resize, t, width, with_effects | Save the frame at time t as PNG/JPG/WebP (by the path's extension). |
| `frame.stats` | Read | t | Histogram/percentile/mean of the frame at t (256x144 downsample), sourced from the live preview render path; {} when the GPU or the requested frame is unavailable. |

**`gallery.*`** (8 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `gallery.apply` | Mutate | at, clip_ids, intensity, name, skip_existing, tab, track, where | Apply a Gallery card (Look/LUT/Caption/SpeedRamp/Effect/Transition) to clip_ids (default selection); Captions is project-wide. Effects: name = an effect's display name or a library id (effects/glow), where=track adds it to every clip on the selection's track. Transitions: name = Cross Fade\|Fade to Color\|Push\|Wipe or a library id (transitions/x); where = start\|end (an edge transition on that side of each clip), connection (the cut at its start/end where a clip abuts), connection_start\|connection_end, all (the cut at each clip's start, else a fade-in) - omit for the old default (= all, unclamped). Duration is the Transitions tab's, clamped to half the shorter clip. Templates place at the playhead instead - use templates.apply. |
| `gallery.hover` | Ui | name, tab | Set/clear the monitor's alt-render preview to a Gallery card (name:null clears). UI-only, no mutation, no undo. |
| `gallery.import` | Ui | paths | Import to Gallery: Gallery item files, audio files (into Sounds), folders (searched through) and .zip files, into your gallery folder. A name that is taken is numbered, nothing is overwritten. Returns what was added and what was skipped. |
| `gallery.list` | Read | mine, tab, source, query, category, sort, detail | Card names in one Gallery tab (Looks\|Luts\|Captions\|SpeedRamps\|Effects\|Transitions\|Templates\|Titles\|Animate\|Sounds), or every tab when omitted. Looks excludes saved node-graph presets and reuses color-engine's builtin_looks() - the sole tool surface for Looks. With `source`, `query`, `category` or `sort` (the Gallery header's chips, search box, category menu and sort order) the list is filtered the same way and also holds the library's items for Effects / Transitions / Luts (installed ones always; online ones once the online library is connected: see gallery.connect). `detail` returns {name, source (Built-in\|Mine\|Installed\|Online), id, categories, uses} per card. |
| `gallery.connect` | Ui | on | Switch the online library on or off for the Gallery (the "Browse online library" button / the tab menu's "Turn the online library off"). Off by default: until it is on nothing is fetched from the network, and the Gallery lists only built-in, your own and already-installed items. On: the small catalogue index is fetched (nothing else; an item downloads when it is used). Remembered between sessions. UI-only, no undo. |
| `gallery.manage` | Ui | action, kind, name, new_name, tags | Change one of your saved Gallery items: action = rename (new_name) \| tags (tags: replaces them) \| duplicate \| replace (with the current selection's version) \| delete (moved to the gallery folder's .trash). kind as gallery.save's: look\|effect\|transition\|caption\|speed\|animate\|template\|sound. |
| `gallery.reveal` | Ui | kind, name | Show a saved Gallery item's file in Explorer (kind + name), or - with only kind (or nothing) - open that gallery folder. The files are plain JSON (sounds: the audio file) you can zip and share. |
| `gallery.save` | Ui | clip_id, index, kind, name, replace, tags, transition_id | Save something of yours to your own Gallery (files in %APPDATA%\SimpleEditor\gallery\<kind>\, shown in the matching tab marked Mine). kind: look (a clip's effect stack) \| effect (index = one effect, else the whole stack, added on use) \| transition (transition_id, default the selected one: length, colour, direction, shader controls) \| caption (the project's style) \| speed (a clip's speed curve) \| animate (a clip's keyframes) \| template (the selected clips; titles too) \| sound (an audio clip: its whole file, or the trimmed range) \| shader_effect (index) \| shader_transition \| html (an HTML layer) \| lut (index) - the last four go to the local library. A taken name is numbered unless replace=true. No undo (it is a file). |
| `gallery.tab` | Ui | tab | Open the Gallery pane on a tab (Looks\|Luts\|Captions\|SpeedRamps\|Effects\|Transitions\|Templates\|Titles\|Animate, or the library tabs HTML\|Icons\|Skills), the same as clicking it; the pane is shown if it was hidden. What Ctrl+3 (Effects) and Ctrl+4 (Transitions) do. UI-only, no undo. |

**`help.*`** (1 tool)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `help.changelog` | Read | - | Current app version and the full CHANGELOG.md text (what the What's New window shows). |

**`history.*`** (2 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `history.list` | Read | limit | Undo-stack entries with their lazily-derived labels and categories, newest first. |
| `history.restore` | Mutate | index | Restores the project to the state BEFORE the given history entry (`index` from `history.list`, 0 = oldest): undoes that entry and everything after it, as a new undo step. Refuses Layout entries. |

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

**`jobs.*`** (4 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `jobs.cancel` | Ui | id | Cancel a job by its jobs.list id (sets its cancel flag / drops a queued export / stops a recording); errors for ids that cannot be cancelled. |
| `jobs.list` | Read | - | Every background job the Jobs pane shows (running / queued), with progress, ETA and whether it can be cancelled or reordered. |
| `jobs.proxy_next` | Ui | path | Build this source's proxy next (path = the asset path); it must be a video above proxy size without a proxy yet. |
| `jobs.reorder` | Ui | delta, id | Move a queued export (id 'queue:<i>') by delta positions in the render queue (only the export queue has an order). |

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

**`library.*`** (3 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `library.bins` | Mutate | action, bin, to | Media library bins (nested by `/`): `list` (default) every bin as `{bin, assets, total}`; `create`; `rename` (`to` = new name); `move` (`to` = new parent, `""` = top level; onto an existing bin it merges); `delete` (its assets go to the root, never deleted). |
| `library.columns` | Ui | columns | Get/set Settings.library_columns — the cells a list row shows after the name, in order. |
| `library.select` | Ui | ids, paths | Set the library selection (asset ids and/or file paths); omit both to clear it. |

**`looks.*`** (2 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `looks.apply` | Mutate | clip_id, intensity, name | Apply a Look (built-in or user-saved) to a clip, replacing its effect stack, scaled by intensity (0 = no-op, 1 = the Look unmodified). Same resolution/result as gallery.apply(tab=Looks) for the same name. |
| `looks.list` | Read | - | Names of every Look: the 12 built-ins plus any user-saved (non-graph) Settings.effect_presets — same list gallery.list(tab=Looks) returns. |

**`markers.*`** (7 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `markers.add` | Mutate | author, clip_id, duration, label, name, note, t | Add a marker at a timeline time (on a clip with clip_id). author: your name, so the user sees who left it. |
| `markers.comment` | Mutate | author, duration, note, t | Leave a comment (a marker with a note, author = you) over In/Out, else the selected clips, else the playhead - or at t / duration when given. Same as the Add Comment action. |
| `markers.export` | Read | format, path | Write every marker in timeline order to a CSV, a YouTube-chapters or an Avid Markers text file (format: csv \| youtube_chapters \| avid, default csv). |
| `markers.import` | Mutate | path | Add project markers from a CSV file (time,name[,note,label] — or markers.export's own header form). |
| `markers.list` | Read | open | Every marker in timeline time (project markers + clip markers) with name, note, icon, done, author ("" = the user). Markers with notes are the user's review comments. open: only open comments (not done, with a note). |
| `markers.remove` | Mutate | id | Remove a marker by id. |
| `markers.set` | Mutate | author, done, duration, icon, id, label, name, note, t | Edit a marker: only the fields given change (done=true ticks a comment off). t: timeline seconds (timeline markers only). |

**`media.*`** (19 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `media.batch_convert` | Read | ext, height, ids, scaler, width | Convert every id with the same options (the scripted batch path — the Convert… window stays single-target); starts one job per asset and returns their output paths at once. |
| `media.consolidate` | Mutate | dir | Copy every asset from outside dir (default: the project's own folder) into it and repoint the paths — both halves run here, synchronously, as one undo step. |
| `media.convert` | Job | ext, height, path, scaler, width | Convert a file with ffmpeg (gif/mp4/mov/mkv/webm/mp3/wav…); returns the output path when done (blocks up to 10 min). |
| `media.import` | Read | paths | Import media files into the library; returns asset ids. A folder among the paths imports as a bin in the background (no ids; use `media.import_folder` for options or to wait). |
| `media.import_folder` | Job | path, bin_name, linked, recursive, extensions | Import a folder as a library bin named after it (`bin_name` overrides). `linked` (default true) links it instantly: nothing scanned or probed, sub-folders show as nested bins, files are listed when a bin opens and become assets when used; the project saves only folder + bin. `linked: false` imports every supported file now, sorted naturally, sub-folders as nested bins (up to 8 deep, `recursive` false = top level only), `extensions` filters (`["wav","mp3"]`), skipping and counting hidden/system/unsupported files, reusing files already in the library; replies when applied ("Imported 84 files into 'SFX' (3 skipped)"). One undo step either way. |
| `media.linked_bin` | Read | action, bin, path | Linked bins: `list` (default; with `bin` also that folder's sub-bins and files, each with its asset id once used), `unlink` (used files stay as assets), `relink` (to `path`), `refresh`, `convert` (import every file under it and make it a regular bin, in the background). One undo step per change. |
| `media.import_sequence` | Job | fps, path | Detect the numbered still run `path` belongs to (3+ frames, same prefix/extension) and bake it to one video asset in the background; fires the `import` hook once when it lands. |
| `media.bulk_set` | Mutate | ids, filter, folder, tags, add_tags, remove_tags, label, description | File, tag or label many assets in one undo step: the assets in `ids`, or every asset `filter` matches (media.list's filter keys; `{}` = all). `folder` is a bin path, created when missing. Never deletes. |
| `media.linked` | Read | bin, sub | Linked folders `{bin, path}`; with `bin`, one level of that folder (`sub` below it): sub-folders and media files with their asset ids. Read only, never outside a linked folder. |
| `media.list` | Read | kind, folder, nested, used, query, tags, label, ext, untagged, ids, fields, sort, limit, offset | No arguments on a library of up to 100 assets: the plain array of every asset. Otherwise a page `{total, offset, count, next_offset, assets, note?}` of the matching assets with just `fields` (default id, name, kind, folder, duration, used, tags, label; `"all"` adds path, ext, width, height, fps, codec, size, audio_streams, description, uses, parent, range, proxy). |
| `media.summary` | Read | - | The library in a few KB: counts by kind, extension, bin and used / unused, untagged / unlabelled, top tags, the bin tree `{bin, assets, total}`, linked folders, smart bins, sequences. |
| `media.relink` | Mutate | dir, ids | Relink offline assets: by file name, then by duration within one frame, inside dir (+ one level of subfolders); returns {relinked, still_missing}. |
| `media.scene_cuts` | Mutate | as_markers, clip_id, split, threshold | ffmpeg select='gt(scene,T)' shot-change detection over the clip's source. Neither flag: pure detection, returns cut times (source secs). as_markers adds one point marker (duration 0) per cut and fires marker_added; split cuts the clip at each one. Synchronous (blocks the UI thread for the ffmpeg pass) — documented ceiling. A cut reported at the very first frame is dropped. |
| `media.set` | Mutate | description, folder, id, label, tags | Edit asset metadata. |
| `media.set_effects` | Mutate | asset_id, effects | Replace an Asset's master effects (applied to every clip using it, prepended before clip-local effects). Effects: [{kind:string,params:object}], same shape as clip.add_effect. |
| `media.smart_bin` | Mutate | id, name, op | Manage Project.smart_bins: save the current library filter under a name, apply one (by index or name) to the library, list them, or delete one. |
| `media.status` | Read | id | AssetStatus per asset id (Ready \| Decoding \| Offline \| ProxyBuilding, with percent) from the same tick that drives the library badge. |
| `media.subclip` | Mutate | asset_id, in, name, out | Create a subclip asset (a named in/out range into an existing library asset). |
| `media.transcribe` | Job | async, asset_id, clip_id, islands, model | Transcribe a clip (or the first timeline clip of an asset) with whisper — same job as transcribe.run without generating cues; fills Project.transcripts when done (read with media.transcript). islands / async as in transcribe.run (async: poll transcribe.status). |
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

**`permissions.*`** (2 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `permissions.list` | Read | query, group | Every tool with its permission (allow, ask, block) as set in Settings ▸ Permissions, its default, the running chat's permission mode, the mode for other MCP clients and whether full access is on. |
| `permissions.request` | Job | tool, reason | Ask the user for access to a tool set to ask: the question shows in the chat with Allow once / Always allow / Deny and the call waits for the answer. Returns {tool, granted, state: allow, block, once, always or denied}. Only the user's click changes a permission. |

**`plan.*`** (4 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `plan.add` | Mutate | assets, notes, parent, title | Add a planner item (optionally under a parent). |
| `plan.get` | Read | - | Planner tree + notes. |
| `plan.remove` | Mutate | id | Remove a planner item. |
| `plan.set` | Mutate | done, id, notes, title | Update a planner item. |

**`playback.*`** (10 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `playback.hud` | Ui | on | Show or hide the performance HUD over the Preview; omit `on` to toggle. |
| `playback.loop` | Ui | in, on, out | Enable/disable Loop In->Out playback. |
| `playback.pause` | Read | - | Pause playback. |
| `playback.play` | Read | - | Start playback. With Settings > Performance > "Buffer before playing" on (default) it first waits, at most 3 s, until the next second of frames and audio is ready, so it returns at once with `state` "buffering" or "playing": poll `playback.status` until `settled` is true. |
| `playback.play_range` | Ui | mode | Play In->Out, Play Around Playhead, or Play to Out, auto-stopping at the target. |
| `playback.rate` | Ui | rate | Set shuttle/playback rate (starts playing if paused); -8..8, 0 rejected (use playback.pause). |
| `playback.scrub` | Ui | t | Emit one BLOCK (~21ms) of audio at t without moving the clock (paused only). |
| `playback.seek` | Read | t | Move the playhead. |
| `playback.status` | Read | - | Current rate, time, dropped and published frame counts, longest wait between two frames since the last call (ms), state (playing / buffering / paused), `buffering` with `buffer_fraction` (0..1 or null) and `waiting_on` while Play waits for the frames ahead or the player refills after a stall, `settled` (true when nothing is buffering: poll for it), `start_wait_ms` / `start_capped` (how the last Play started), `audio_start_lag_ms`, loop range, and the performance HUD's numbers (UI frame avg/p95, GL composite, texture upload, decode avg/worst ms, cache hit rate). |
| `playback.health` | Ui | `fix` (bool) | Playback health as the Preview's chip shows it: `level` green / amber (frames dropped in the last few seconds of play), the one `advice` (a proxy for the video at the playhead that is slow to decode, with `proxy_bytes`, or a lower playback resolution), `sources_at_playhead` with their decode cost, and `vfr_sources` playing from a constant-rate proxy. `fix: true` applies the advice. |
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
| `preview.audio_only` | Ui | on | Audio Only preview: the Preview plays the sound with a visualizer of the mix, no picture decoded; omit `on` to toggle. Saved with the project, not an undo step; export unaffected. |
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
| `project.recent` | Ui | action, path, name | The recent projects list. action list (default): path, title, exists, width, height, fps per row; rename: renames the .sedit file at path to name (refuses a taken name, a missing file, or a project open in another window); remove: drops path from the list (the file stays). |
| `home.show` | Ui | open | Shows the Home page (recent projects and New cards), closing the project through the usual save prompt; open:false returns to the editor. Needs Settings home_page on. |
| `project.save` | Read | path | Save the project (.sedit). Without a path: the current project file (error if none). |
| `project.set` | Mutate | fps, height, name, width | Change project format/name. |
| `project.summary` | Read | - | Project overview: format, duration, tracks, clips per track, assets, sequences, subtitles count, planner progress, notes — and the markdown style summary. |

**`render.*`** (5 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `render.contact_sheet` | Read | count, from, to, width | Tile `count` frames (1..16, default 9; tiles past the last frame are clamped to it) evenly spread over [from, to] into one PNG image (left to right, top to bottom; `times` lists each tile's time). width: px per tile, default 320. |
| `render.frame` | Read | t, width | Render the timeline at time t as a PNG image, max `width` px wide (default 640). MCP clients get an image content block; Luau gets {width, height, data_url}. |
| `render.layers_async` | Ui | max_w, t | Queue a non-blocking one-shot layer decode; returns a request id. |
| `render.poll_layers` | Read | id | Poll for the async layer-decode reply (not ready until it matches the given id, or it was superseded by a newer request). |
| `render.range` | Ui | a, b | Pre-render [a, b) into the movie-mode cache without touching the in/out points. |

**`scopes.*`** (1 tool)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `scopes.read` | Read | kind | Current-frame histogram/percentile/mean statistics (waveform/parade/vectorscope share the same summary — see the tool's own note). |

**`scripts.*`** (3 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `scripts.eval` | Ui | code, purpose, dry_run, budget_ms, max_output | Run a Luau script now against the project and library, as one undo step `Script: <purpose>` (undone if it errors); sandboxed to project / library tools, so not held for Allow. See "Assistant scripts" above. |
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

**`tasks.*`** (5 tools)

The Assistant's Tasks queue (per conversation, saved with it). A task is a Luau script an agent wrote for
the user to review and run; each run is one undo step labelled `Assistant task: <title>`.

| Tool | Kind | Args | Description |
|---|---|---|---|
| `tasks.add` | Ui | code, note, title | Hand a Luau script over to the Assistant's Tasks queue for the user to review and run (it is not run now). title: a few words; note: one line on what it does. |
| `tasks.list` | Read | - | The Assistant's Tasks queue in order: [{id, title, note, status, error, code}]. |
| `tasks.remove` | Ui | id | Remove a task from the Assistant's queue. |
| `tasks.run` | Ui | id | Run one task, or (no id) every Ready task in queue order; each run is one undo step and a failing task stops the queue. |
| `tasks.set` | Ui | code, id, title | Change a task's title and/or code (new code makes it Ready again). |

**`templates.*`** (4 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `templates.apply` | Mutate | at, name | Place a template (saved or built-in) at a time, slots left as they are (templates.use fills them). |
| `templates.build` | Ui | clip_ids, name, placeholders, slots, tags, thumb_t | Save clips of the timeline as a template in your Gallery (what the Template Builder does): slots [{clip_id, name, wants, fit}] in fill order, placeholders = clip ids saved without their footage. |
| `templates.expose` | Ui | fields, name | Rewrite a saved user template's captured clips, setting Clip.exposed on each addressed clip_index (position within the template's own clip list, not a live id). Settings-level (Settings.templates) — ToolKind::Ui, not Mutate. |
| `templates.export` | Ui | footage, name, path | Write a template to one .setemplate file, with (default) or without its footage. |
| `templates.import` | Ui | path | Add a shared .setemplate file to your Gallery; footage not in the file becomes empty slots. |
| `templates.list` | Read | - | Saved clip templates, the built-in animation templates and motion presets. |
| `templates.slots` | Read | name | A template's slots: index, name, wants (any/image/video), seconds, start, fit (trim/stretch), has_footage. |
| `templates.use` | Mutate | at, fills, name, track | Place a template with its animation blocks, transitions and layers and fill its slots: fills = an array in slot order or an object keyed by slot name / 1-based index, values = asset id or file path. One undo step. |
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

**`transcribe.*`** (3 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `transcribe.install` | Job | model | Download a whisper model into the cache (job; 75–466 MB). Already downloaded = done at once. |
| `transcribe.run` | Job | async, clip_id, cues, islands, language, model | Transcribe a clip's audio with whisper (background job; word timings always on). On completion the words are in Project.transcripts (media.transcript) and, with cues=true (default), captions are generated. islands=true transcribes each speech island separately (accurate cut points on long takes with pauses; needs the clip's waveform). async=true replies at once - poll transcribe.status {clip_id} until state is done/failed. |
| `transcribe.status` | Read | clip_id | State of the clip's latest whisper run (for transcribe.run / media.transcribe with async=true): {state: running (+fraction, status) \| done \| failed (+error) \| none}. When done, read the words with media.transcript. |

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

**`voice.*`** (3 tools)

| Tool | Kind | Args | Description |
|---|---|---|---|
| `voice.start` | Ui | - | Start dictating into the Assistant prompt from the microphone (stops by itself after 60 s). |
| `voice.status` | Read | - | Dictation state: 'recording', 'transcribing' or 'idle', the pane's status line, and the last take's 'text' or 'error'. |
| `voice.stop` | Ui | - | Stop dictating and transcribe. whisper takes a few seconds: poll voice.status until state is 'idle', then 'text' (also appended to the prompt) or 'error'. |

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
| Premiere | `split` → `Ctrl+K` (Premiere's own "Add Edit" muscle memory), `export` → `Ctrl+M` |
| Resolve | `split` → `B`, `command_palette` → `Ctrl+Space`, `toggle_transitions` → `Ctrl+Shift+T` |
| Avid | `command_palette` → `Ctrl+Alt+K`, `split` → `Ctrl+Shift+B` |

`Ctrl+P` (and `Ctrl+K` while nothing else is bound to it) opens the **palette**. Plain text goes to
clips (every timeline, nested ones too), timelines, Library media and bins, markers / comments, pages,
panels and Gallery cards; `>` lists every `Action`, `Pane`, arg-free `ToolDef`, Luau script and on/off
setting; `/` is the assistant (`/ask <message>`, `/<skill> [args]`); `:` any tool by name, with a mini
argument form built from the tool's own arg docs. `ui.palette {query, pick?, alt?}` lists (or runs) the
same rows over MCP. `F1` opens the **cheat sheet** (a second palette entry point, listing every bound
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

## Social guides (data and profiles)

`assets/guides.json` (embedded in the exe, 20 KB) is the data behind the viewer's Social Guides, and
`library/guides/guides.json` is the same file for the online library. Platforms in
`%APPDATA%\SimpleEditor\library\guides\guides.json` replace the built-in ones with the same `id`
(new ids are added; a file that fails validation is ignored). Format, per platform:

- `id`, `n` (name), `net` (account family: youtube, tiktok, instagram, facebook, snapchat, x), `pg` (page shape in
  device preview: `full`, `feed` or `watch`), `a` (the player's aspect `[w, h]`), `ver` (the date it was
  verified) and `src` (what it was measured on; say when something is an estimate).
- `lay.phone` / `lay.desktop`: `ref` (the reference screen in dp the rectangles were measured on), `fit`
  (`cover` or `contain`), `zones` (`k`: `d` danger, `c` caution, `s` safe, `x` crop; `r`: `[x0, y0, x1, y1]`
  in dp; `l`: label), `safe` (the safe rectangle), and `ui`: the silhouettes. Every coordinate sticks to its
  nearest screen edge, so the action column stays at the right edge and the caption at the bottom on a taller
  phone. An element `t` is `scr` (gradient scrim), `bar`, `ico` (glyph `i`, count `n`, label `lbl`), `ava`,
  `txt` (profile field `f` or literal `x`, size `s`, `l` collapsed lines, `more`), `pil`, `prg` (progress
  bar, filled by the playhead) or `chp`. `tap` names the sheet it opens in device preview; `o: 1` = outside the frame.
- `vw` (long form only): the desktop player's `default`, `theatre` and `full` boxes as `[x0, y0, x1, y1]` px of a
  1920x1080 viewport (`full` = the whole screen).
- `st`: `comments`, `desc`, `share`, `more`, each `{m, sh, ms}`: `shrink` (the video shrinks into the part
  above the sheet), `cover` (the sheet covers and dims it) or `page` (long form: the page scrolls under the
  player); `sh` = where the sheet's top edge is, as a fraction of the screen; `ms` = the slide time.

`ui::social::profile` holds `SocialProfile` (Settings `social_profiles`, `social_active`): no secret of any
kind. Its only network call, `fetch`, runs from the Settings button or `profile.fetch`.
