# Creator Kit: engine contract (step 1)

Four workstreams build the engine at the same time, **in the same worktree** (`feat/pro`), with
disjoint files: `html-layer`, `shader-library`, `skills`, `online-library`. This file is what they
share. The design is `plans/creator-kit/README.md`; the file formats are `library/README.md`; the
renderer method and measurements are the "creator-kit spike" entry at the top of `notes.md`.

Wave 0 is already in the tree and compiles (`cargo check` is clean): one `// ---- ws:<name> ----`
marker per workstream after `ws:docs-release` in every shared table (`src/ui/app/mod.rs`: App fields,
App init, `TOOL_TABLES`, `ACT_HANDLERS`, `FRAME_HOOKS`, `WINDOW_DRAWERS`, `PANE_DRAWERS`, the `mod`
list; `src/settings.rs`; `src/hotkeys.rs`), the stub modules below already registered, and:

- `src/engine/library.rs` - REAL: `Kind`, `Item`, `root()`, `slug`, `path_of`, `installed(kind)`,
  `read(kind, id)`, `save(kind, id, text)`, `remove(kind, id)`: the installed library on this PC,
  `%APPDATA%\SimpleEditor\library\<kind dir>\<id>.<ext>`.
- `src/engine/shaderlib.rs` - REAL: `parse_meta`, `ShaderMeta`, `ShaderParam`, `label`, `lint`.
- `src/engine/shaders.rs` - `user_shader` now defines every `@param` id and declares `u_prev` /
  `u_next` / `u_frames` for `@needs previous`; NEW `transition_shader(src)`.
- `src/engine/gpu/library_tests.rs` - compiles and draws every `library/**/*.glsl` on the real driver.
- `src/engine/html.rs` - `icon_page(name, svg)` only; the rest is html-layer's.
- `Settings.library_url`, `Settings.library_dir`.

## Rules for every workstream

1. Work only in `D:\Projects\simple-editor\.claude\worktrees\pro`. Never touch
   `D:\Projects\simple-editor\src` or `src/ui/settings_ui/general.rs`. No git commands that change
   anything (no commit, stash, checkout, reset): the lead commits. No push.
2. Read `agents.md`, `ARCHITECTURE.md` ("Registries", "New-pane checklist", "Size & idle-CPU gates",
   "Conventions") and the files you are about to change before writing. Never invent an API: read it.
3. **Your files are yours; everyone else's are not.** The table below names each workstream's files.
   In a file you do not own, a small, clearly marked edit is allowed only where the table says so
   (your `// ---- ws:<name> ----` section in a shared table, a struct literal that needs your new
   field, one menu row). Re-read the file right before editing it: another agent may have changed it
   a minute ago. Never reformat, reorder or "tidy" a shared file. Never fix another workstream's code:
   report it instead.
4. Registries: fill your own marker section only. Every new MCP tool is a `ToolDef` row in YOUR
   `tools_*.rs` file. Every new `pub fn …(&mut self)` on `Project` (`src/model/ops/*.rs`) needs a
   `ToolDef` row or an `OP_INTERNAL` reason (`tools_registry_tests.rs`). Every new window / pane / tab
   gets its own `assert_no_idle_repaint_<context>` test. Every new `Action` goes in your section of
   `hotkeys.rs` (both the `actions!` list and the group match). Menus only through `ui::menu::*`
   (never `.context_menu(` / `.menu_button(`). No new crates. No blocking work and no network on the
   UI thread or at startup; idle CPU stays 0 (no unconditional `request_repaint`).
5. Size: the whole kit has a budget of a few hundred KB of exe. Reuse what exists (`ffpipe::command`,
   `export::spawn_job`, `export::temp_output`, `mcp::png_encode`, the transcribe module's curl download,
   `ui::menu`, `confirm`, toasts, the Jobs pane), avoid generic-heavy code, mark deliberate shortcuts
   `// ponytail: <what> - <upgrade path>`.
6. Old projects and settings files must still load: every new serde field has `#[serde(default)]`
   (and `skip_serializing_if` when empty is the common case).
7. Building. `cargo check` (about 1 minute, one runs at a time) is allowed and expected before you
   finish. Other agents are mid-edit in the same tree: errors in files you do not own are theirs -
   ignore them, wait a minute and retry if they block you, never fix them. `cargo test <your filter>`
   takes about 7 minutes to build: run it at most twice, near the end, for your own tests. Do NOT run
   the whole suite, `--release` builds, `scripts/size.ps1`, bare `cargo fmt` (it reformats the crate;
   `rustfmt --edition 2021 <one of your own new files>` is fine, never on a `mod.rs` or `main.rs`).
   Drive D: has about 5 GB free: no extra target folders, no large scratch files there (use
   `C:\Users\Kash\AppData\Local\Temp\claude\1f7891f1823f0ac2\7d8bb949-973a-42da-b19b-a0c5ab7dbf3f\scratchpad\<ws>\`).
8. Do not start the app's window (the lead runs the live test). Never `taskkill /IM` anything
   (`simple-editor.exe` and `msedge.exe` are also the user's own running programs): stop only a
   process you started, by its id.
9. Do not claim something works unless you ran it. Your final report lists: files changed; new tools
   (name, kind, args); new Actions, Settings fields, serde fields; tests added and which you ran;
   what is not done or not verified; anything another workstream or the lead must do.

## Files

| Workstream | Owns (create / change freely) | May touch, marked and minimal |
|---|---|---|
| `html-layer` | `src/engine/html.rs`, `src/ui/html_ui.rs` (new), `src/ui/app/html_ctl.rs`, `src/ui/app/tools_html.rs`, `src/model/ops/html.rs` (new, if needed) | `src/model/asset.rs` (+ every `Asset { … }` literal that then needs the new field), `src/media/proxy.rs` and `proxy_eligible` / `proxy_candidates` in `src/ui/app/jobs.rs` (alpha proxy at any height), `src/ui/inspector.rs` (the Fields section of an HTML clip), `src/ui/app/menus.rs` (Add ▸ HTML Layer / HTML Scene), `src/ui/timeline/menus.rs` (clip menu rows: Edit HTML…, Save to Library) and the timeline's clip double-click, `src/ui/app/jobs_pane.rs` (a row for a render), `src/ui/mod.rs` (`pub mod html_ui;`), `src/mcp/mod.rs` `INSTRUCTIONS` (one sentence), its marker sections |
| `shader-library` | `src/engine/shaderlib.rs`, `src/engine/shaders.rs`, `src/engine/gpu.rs`, `src/engine/gpu/library_tests.rs`, `src/model/effect.rs`, `src/model/transition.rs`, `src/model/ops/transitions.rs`, `src/model/ops/effects.rs`, `src/engine/compose.rs`, `src/engine/effects.rs`, `src/ui/transitions_ui.rs`, `src/ui/shader_ui.rs`, `src/ui/effects_ui.rs`, `src/ui/app/tools_shader.rs`, `src/ui/app/thumbs.rs` | `src/playback.rs` (only what a shader transition or `@needs previous` needs), `src/engine/xmeml.rs` / `src/engine/edl.rs` / `src/engine/import.rs` (a shader transition exports as a cross fade), `src/ui/mod.rs` (`DragPayload`), `src/ui/timeline/*` (only the transition parts), `src/ui/inspector.rs` (only effect-parameter labels, if they are drawn there), every exhaustive `match` on `TransitionKind` / `ParamSpec` use elsewhere that stops compiling, `src/mcp/mod.rs` `INSTRUCTIONS` (one sentence), `library/README.md` (effects / transitions sections, only to keep them true), its marker sections |
| `skills` | `src/engine/skills.rs`, `src/ui/assistant_ui.rs`, `src/ui/assistant_data.rs`, `src/ui/app/assistant_ctl.rs`, `src/ui/settings_ui/assistant.rs`, `src/ui/app/tools_skills.rs`, `src/engine/agent.rs` | `src/mcp/mod.rs` `INSTRUCTIONS` (one sentence), its marker sections |
| `online-library` | `src/engine/library.rs`, `src/ui/gallery.rs`, `src/ui/app/gallery_ctl.rs`, `src/ui/app/library_ctl.rs`, `src/ui/app/tools_library.rs`, `src/ui/app/tools_gallery.rs`, `src/ui/color_ui.rs`, `src/engine/lut.rs`, `scripts/library-index.ps1` (new) | a settings tab other than `general.rs` if a visible setting is needed, `src/ui/app/files.rs` (copying a used LUT beside the project on save), its marker sections |

The lead owns everything else, `library/` content, `notes.md`, `goals.md`, `ARCHITECTURE.md`,
`CHANGELOG.md`, `docs/`, and merges the pieces.

## Cross-workstream API (signatures are fixed; the stubs exist so callers compile today)

| Provider | Symbol | Used by |
|---|---|---|
| html-layer | `App::add_html_clip(&mut self, html: &str, name: Option<&str>, at: Option<f64>, duration: Option<f64>, fields: &[(String, String)]) -> Result<Id, String>` (`src/ui/app/html_ctl.rs`): adds the asset + clip, starts the render, returns the clip id; pushes no undo itself | online-library (apply an HTML template or an icon), `html.create` |
| html-layer | `crate::engine::html::render_frame(html: &str, fields: &[(String, String)], scale: f64, t: f64) -> Result<crate::media::Frame, String>`: blocking one-shot render of one frame (straight-alpha RGBA) through Edge; never called on the UI thread | `html.check`, the editor preview, library thumbnails |
| html-layer | `crate::engine::html::lint(src: &str) -> Vec<String>`: what breaks the library rules (empty = fine) | the library validation test |
| html-layer | `crate::engine::html::icon_page(name, svg) -> String` (exists) | online-library (an icon is placed as an HTML layer) |
| shader-library | `App::apply_shader_effect(&mut self, src: &str, clip: Option<Id>) -> Result<usize, String>` (`src/ui/app/tools_shader.rs`) | online-library |
| shader-library | `App::apply_transition_shader(&mut self, src: &str, clip: Option<Id>) -> Result<Id, String>` (same file) | online-library |
| shader-library | `Effect::from_shader(src: &str) -> Effect`: kind `Shader`, source = `src`, params = the `@param` defaults | anyone building an effect from a library file |
| online-library | `crate::engine::library::{Kind, Item, installed, read, save, remove, path_of, slug}` (exists) | html-layer ("Save to Library"), shader-library (the Effects list and the Transitions window show `installed(Kind::Effect / Transition)`), skills (`installed(Kind::Skill)` are skills too) |
| skills | reads `library::installed(Kind::Skill)`; nothing calls into it | - |

If you need something another workstream owns and it is not in this table, do not edit their file:
write the smallest thing on your side, and name the gap in your report.

Tool names are fixed (the built-in skills already call them): `html.create {html, at?, duration?,
name?, fields?}` (`fields` is an object `{name: value}`), `html.get {clip}`, `html.set {clip, html?,
fields?}`, `html.render {clip}`, `html.check {clip? | html?, t?}` (an image result); `clip.set_shader
{clip, index?, src}`, `shader.check {src, kind?}`; `skills.list`, `skills.get {name}`, `skills.save
{name, description, args?, body}`, `skills.delete {name}`; `library.list {kind?, query?}`,
`library.install {id}`, `library.apply {id, clip?, at?}`, `library.save {kind, name, text}`,
`library.remove {id}`. `library.*` here is the content library (transitions, effects, LUTs, HTML
templates, icons, skills): say so in each description, because `library.select` / `library.columns`
(the media library panel) already exist. An item id is `<kind dir>/<file id>`, e.g.
`transitions/paper-tear`.

## Decisions already made (do not reopen)

- HTML rendering: the method in notes.md (Edge headless, DevTools over a std-only WebSocket client,
  each animation's time set per frame through `Runtime.evaluate`, PNG screenshots piped to ffmpeg as
  PNG-in-MOV). One Edge is kept alive between renders and closed with `Browser.close` (then waited
  for, then killed by its own process id only if it is still there); it is started lazily, never at
  app start. A page without animations is one PNG. Frames after the last finite animation ends are
  the same picture: render one and repeat it. Output files are content-addressed in
  `cache_dir()/html/` and the asset's `path` is pointed at the new file (the clip keeps its id,
  effects, keyframes and position). The HTML source and field values live in the project, on the
  asset (`Asset.html`). `ffpipe::bake_sequence` and the bake pipeline are not used.
- Render scale: CSS pixels are 1080-reference pixels; the device scale factor is
  `min(project width, project height) / 1080`. A new HTML clip sits at its natural size (a 640x220
  button is not blown up to fill the frame), centred; a 1920x1080 layer fills a 16:9 frame.
- A source with alpha gets its stacked proxy at any height (`min(proxy height, its own height)`).
- Shader transitions: one new `TransitionKind::Shader`; the source and the control values live on
  the `Transition` (`shader: String`, `params: Vec<f64>`); its name is the source's `@name`. The two
  sides are drawn to separate targets and blended by `shaders::transition_shader`. On the CPU
  compositor and in EDL / XML export it is a cross fade.
- A shader effect is `EffectKind::Shader` with the library file as its `Effect.shader`; its controls
  and its display name come from the header (`shaderlib::parse_meta`). Nothing is added to the
  `EffectKind` enum.
- Skills: Markdown with a `---` header (`library/README.md`). Three sources, in this order of
  precedence: the user's `%APPDATA%\SimpleEditor\skills\*.md`, installed library skills
  (`library::installed(Kind::Skill)`), built-ins embedded with `include_str!` from `library/skills/`
  (only: `captions`, `cut-silences`, `shorts`, `lower-third`, `title-card`, `create-skill`).
- The catalogue is `index.json` (format in `library/README.md`), read from `Settings.library_dir`
  when set, else fetched with `curl.exe` from `Settings.library_url`, on a background thread, only
  when the Gallery's library is opened or a `library.*` tool asks. Item files and thumbnails are
  fetched the same way, on demand.
