# Notes — Simple Editor

Freeform scratchpad: decisions, gotchas, things worth remembering that don't belong in
[agents.md](agents.md) (workflow) or [goals.md](goals.md) (priorities/roadmap). Append via
`/se-notes`, by editing this file directly, or by telling an agent to jot something down.
Newest at the top. No required format — a bullet or a short paragraph is fine.

---

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
