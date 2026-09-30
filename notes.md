# Notes — Simple Editor

Freeform scratchpad: decisions, gotchas, things worth remembering that don't belong in
[agents.md](agents.md) (workflow) or [goals.md](goals.md) (priorities/roadmap). Append via
`/se-notes`, by editing this file directly, or by telling an agent to jot something down.
Newest at the top. No required format — a bullet or a short paragraph is fine.

---

- **docs-site (2026-09-29), screenshot pipeline gotchas (`scripts/docs-shots.ps1`):** a live
  `ui.screenshot` and the `--screenshot` CLI path use the very same eframe readback (glow
  `read_screen_rgba` right after painting, `ViewportCommand::Screenshot`), yet the live one read back
  black / stale frames while the Windows session was locked in wave 1 and the CLI one didn't - so the
  difference is environmental (most likely: a window created while locked renders fine, one that was on
  screen when the lock came doesn't), not a code path to fix; an agent can't lock the session to prove
  it. The pipeline therefore launches the app once per shot with `--screenshot` and the new
  `SE_SCREENSHOT_WHEN=<file>` (shoot once the file exists), drives it over MCP, then touches the file.
  Other traps it handles: the startup toasts ("GPU preview: …", "MCP server at …") live 5 s, so a shot
  waits until 5.5 s after launch; `media.import` replies before the probe lands, and `timeline.add_clip`
  on an unprobed asset makes a zero-length clip (wait for `media.list` durations); a killed run leaves an
  autosave AND a `<project>.sedit.lock` (the next launch would show "Recover unsaved project?" or a
  "may already be open" toast), and the app writes pages/layout back into settings.json, so the profile
  is reset before every launch; the shared target dir's exe may be another worktree's build (the script
  checks it contains `SE_SCREENSHOT_WHEN`; keep a copy of your build and pass `-FromExe <copy>` to skip
  cargo); and something maximised the test window mid-run once (the
  script compares the PPM's aspect with 1600x900 and retries). Coordinates for clicks are panel-relative
  (`layout.list` now reports each pane's `rect`), so a layout change mostly means re-running, not
  re-measuring. Found on the way: a reveal from inside a pane's draw is lost (`self.layout` is a
  placeholder `Layout` during `layout::show`), so a Library click loads the Source monitor but doesn't
  bring its tab forward; `source.open` over MCP does. And never `rustfmt src/main.rs` - it follows the
  `mod` tree and reformats the whole crate, like bare `cargo fmt`. Every launch sets `SE_BACKGROUND=1` and
  writes `"window_rect": [-2520, 112, 1600, 900]` into the scratch settings.json (the script's
  `-WindowRect`), so the windows open on the maintainer's second monitor without taking focus - fifty
  launches in a row used to jump in front of whatever the user was doing.

- **se-fix (2026-09-07), "line tool snaps to the wrong direction" bug report — already fixed, no
  code change:** investigated a report that shape drawing doesn't render during the drag and the
  Line tool snaps to a direction that doesn't match the drag on release. Root cause was real but
  already fixed by `6d5c6e1` ("fix(shapes): a line keeps the point it was started from",
  canvas-handles-monitor/pro-monitor era) — `constrain_drag`/`signed_min` in `ui/preview.rs` keep
  the drag's signed half-extents all the way through `draw_shape_preview` (live rubber band) and
  `drag_stopped` (final `new_shape`), and `engine::shapes::signed_half` renders Line/Arrow between
  the same signed corners, so the preview and the final render always agree. `6d5c6e1` is an
  ancestor of `main`@`00be9b0` (this worktree's base) and is covered by
  `line_tool_keeps_the_press_as_its_origin`, `a_shape_drag_emits_exactly_one_shape`,
  `shift_locks_a_line_to_45_degrees` and 4 more in `ui/preview.rs`'s test module — all pass.
  Cross-checked live against the actually-running `target/release/simple-editor.exe` (built same
  day from the same commit, confirmed via the MCP co-editing tools): `shapes_add` with a signed
  negative height rendered in the correct direction via `frame_export`, and `shapes_add` → `undo`
  cleanly removed the clip with one history entry. If this report recurs, check the reporter's exe
  build date/hash before re-investigating the drag math — it's solid as of `00be9b0`.

- **tab hover cursor (2026-09-07):** egui_tiles' `Behavior::tab_hover_cursor_icon()` defaults to
  `CursorIcon::Grab`, which Windows renders as the 4-arrow move cursor — misleading for a
  click-to-switch tab (dragging still works, it just doesn't need to announce itself with that
  icon). Overridden to `CursorIcon::Default` in `layout.rs`'s `Behaviour` impl. Also: a cursor-only
  change has no pixel diff a `--screenshot` render would show (it doesn't capture the OS cursor),
  so that verification step doesn't apply here — tests + `--selftest` are the only signal.
  Unrelated flakiness hit during verification: `media::thumbs::tests::video_thumbs_colours_aspect_and_cache`
  times out ("thumb within 3 s") intermittently even on `main`/isolated single-threaded runs —
  pre-existing, not caused by any of today's changes.

- **popup positioning (2026-09-07):** egui remembers a `Window`'s last dragged position across
  close/reopen (keyed by its `Id`), so `.default_pos(...)` only ever takes effect the *very first*
  time a stable-Id window (e.g. Settings) is shown — reopening it later just restores wherever it
  was left, not a fresh click/center point. To reposition on every open, force it for one frame only
  via `.current_pos(...)` on the frame `open` transitions false→true (tracked with a `prev_open`
  field), then let normal drag memory take back over. One-shot confirm/discard windows
  (`ui/confirm.rs`) don't have this problem — their `Id` includes the body text/index so they're
  fresh every time, and plain `.default_pos()` is enough. Shared click-or-center logic lives in
  `crate::ui::popup_open_pos`.

- **docs-refresh (2026-09-07), cross-cutting gotchas from the whole overhaul:** Luau's io/os/ffi
  sandbox means `editor.log()` only ever renders as a 5-10s auto-expiring toast
  (`ui/app/palette_ctl.rs`'s `fire_hook`, `app.rs`-descended toast draw) with no copy button —
  script-generated docs must be transcribed via `curl` on `tools/list`, never by reading toast
  output. Alt-render requests must check `self.export.is_some()` before firing (UI thread also
  services export's `GpuFrameRequest`s — the two must not fight over the GL context). Registry
  marker-section hunks never got a real union conflict in practice because `.rs` files were
  deliberately excluded from `.gitattributes merge=union` (only `size_log.csv`/`CHANGELOG.md` are).
  All six planned `-- @on` hook events (`selection_changed`, `import`, `export_done`,
  `project_open`, `project_save`, `marker_added`) do have real `fire_hook` call sites as of
  wave-3-complete — verified by grep against the merged tree, not assumed; see
  ARCHITECTURE.md's "Registries" section and `docs/customizing.md` for the owning file:line of
  each. `Settings.mcp_enabled` defaults to `false` and `PowerShell`'s `Set-Content -Encoding utf8`
  writes a UTF-8 BOM on Windows PowerShell 5.1 — a hand-written `settings.json` with a BOM gets
  silently quarantined to `settings.json.bad` on boot (parse failure), so MCP never starts; use
  `[System.IO.File]::WriteAllText(path, json, [System.Text.UTF8Encoding]::new($false))` instead.

- **UI/UX overhaul plan (2026-09-04):** lives in `plans/ui-overhaul/` — `README.md` is the master
  plan (thesis, decided keymap, frozen modifier table, registry protocol, size plan, waves),
  `issues/<workstream>.md` are issue-ready bodies for `/se-implement`. Rules an implementer must not
  bend: wave 0 is three *serial* PRs (0a pure moves → 0b registries/schema/hooks → 0c size-diet) and
  nothing else starts until 0c merges; a workstream edits only its own files plus its pre-seeded
  `// ---- ws:<name> ----` section in the shared tables; every new `pub fn(&mut self)` on Project
  needs a ToolDef row or `every_edit_op_has_a_tool` fails; plain edge-drag stays a plain trim and
  Delete stays non-ripple (pro behaviour is on modifiers / per-track flags). Verified size facts the
  plan rests on: `luau0-src` forces `opt_level(2)` (Luau's ~2 MB is fixed), eframe `default_fonts`
  embeds 1.41 MB of TTFs, egui_commonmark cost ~2.1 MB when added. Process gotchas: fix/revise
  agents sometimes wrap `name`/`worktree` in literal quotes — normalise before keying on them.

- **Playback-cache work (PRs #12+#14), gotchas worth keeping:** `Cache::avg_entry_bytes` must
  refuse empty/zero-byte caches — an empty timeline's GPU LayerSets insert at 0 bytes and the
  horizon math would divide by zero. `DecoderPool::set_proxies` keys decoders by RESOLVED path, so
  a remap must drop the source, its OLD proxy and its NEW proxy keys. The proxy "building" badge
  is set/cleared entirely inside the build job via a drop guard — setting it after `spawn_job`
  returns would race a fast-failing job and stick forever. The source-frame cache keys by (source
  path, exact f64 µs, w, h): replays hit because callers re-derive bit-identical times from the
  fps grid; animated-scale clips request a new size every frame and never hit (commented ceiling).
  History labels are derived from an entry's NEXT neighbour — any delete must clear the label
  cache (Sonnet review caught this; test pins it).

- **Auto-cut "Beats" section had no detect/commit split:** unlike Silence and Scene cuts (Detect
  populates a preview; separate Split/Mark instead/Apply buttons commit), the old "Detect Beats"
  button both detected onsets AND wrote markers in one click — inconsistent with `audio.beats`'s
  own MCP contract ("with neither flag: pure detection... no mutation"). Split
  `analysis::detect_beat_markers`/`split_beats` into a pure `detect_beats` (returns per-clip onset
  times + BPM, no mutation) plus `beat_markers`/`split_beats` (act on the cached preview). UI now
  has Detect Beats → Add Markers / Split at Beats, matching the other sections' shape.
