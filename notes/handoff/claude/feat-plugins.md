# Hand-off: plugins (`claude/feat-plugins`, from `origin/develop` 41c9b73)

Section "plugins" of `plans/cloud-handoff.md`. `claude/feat-scripts-editor` (up to 0c8f352) is merged in, and the
plugin pieces build on its Script Editor.

## Done

- **Format** (`src/engine/plugins/mod.rs`, spec in `docs/plugins.md`): `plugin.json` with id, name, version,
  author, description, `min_app_version`, homepage, icon; `contents` (scripts, panels, skills,
  effects/`shaders`, transitions, LUTs, templates, icons, fonts that must say `OFL-1.1`); `permissions`
  (`network` hosts, `files` project/pick, `run_programs`, `mcp_tools`). `validate` checks every field and file,
  and returns author-readable errors.
- **Install** (`mod.rs`, `zip.rs`, `github.rs`):
  - Links can be a repository, `releases/latest`, `releases/tag/<t>`, `tree/<ref>/<folder>` or `archive/...zip`.
  - The latest release's first `.zip` asset is used, else the tag's source archive, else
    `codeload.github.com/<o>/<r>/zip/HEAD`. Downloads go through `curl.exe`, like the library does, with a
    generic User-Agent.
  - Unpacking is in-process: a small zip reader on `miniz_oxide`, which png and flate2 already link (checked
    with `cargo tree`). It does path-safety, CRC and size caps, and refuses zip64 and encryption.
  - Staging happens in `plugins\.staging`, then the user sees the confirm. Commit moves the folder to
    `%APPDATA%\SimpleEditor\plugins\<id>` and copies library content into the library. When a name is taken,
    the plugin id goes in front; the user's own files are never overwritten.
  - `plugins\state.json` holds per plugin: enabled, source, tag, granted, picked folders and the registered
    items. It also holds `panel_slots`.
  - Disable, Enable, Uninstall and Update work. Update keeps the picked folders and the source link.
- **Runtime** (`runtime.rs`): its own Luau VM with the Assistant sandbox. That means no
  io/require/load/debug/getfenv/collectgarbage, an `os` cut down to the clock, 64 MB, and a resettable
  wall-clock deadline. It has the same project API (`editor.tool` through `script_ctl::script_may_call`, plus
  granted `mcp_tools`, and the PRELUDE's `library`/`project`/`query`).
  - With their permission it adds `net.get(url)` (non-blocking, per-panel cache, wakes the UI), `fs.read`,
    `fs.write`, `fs.list`, `fs.exists` (project folder or picked folders; `..` refused; resolved paths
    re-checked) and `programs.run`.
  - Every refusal names the plugin and the permission, and says whether it was not declared or not granted.
  - The API is a Lua bridge that calls into a host table swapped in for each run, so a VM kept between panel
    frames never holds a stale scoped borrow.
- **Panels** (`panel.rs`): `ui.heading/label/button/slider/text/checkbox/dropdown/row/column/separator/space/image`.
  - `draw()` records a `Node` tree, and `panel::draw` paints it with egui. Form rows sit in a two-column grid
    (labels at one x, inputs at one x).
  - Interactions come back as the widgets' return values on the next run, like immediate mode. Repeats get
    ids like `Label#2`, and `"Label##key"` works.
  - `draw()` runs only on open, an interaction (plus one follow-up run so widgets above a clicked button
    update), an edit or selection change (`revision()` hashes the undo stack and selection, not the
    playhead), a `net.get` landing, or the script changing on disk (checked at most 1×/s while drawn).
    While playback runs, only clicks are answered.
  - Budgets: 16 ms for an idle redraw, 250 ms when answering a click, 250 ms to load, 2000 widgets. Over a
    budget the panel stops and shows Retry, Edit Script and Open Folder.
  - The edits from one run are one undo step labelled "<plugin>: <panel>". The project is snapshotted lazily
    on the first Mutate call, so a read-only draw never serialises it. A failed run is rolled back.
- **App** (`src/ui/app/plugins_ctl.rs`):
  - `Pane::Plugins` is the manager. Each plugin is a `list_rows` row with Enable/Disable, Update (first a
    check, then the install confirm), Open Folder (`contextmenu::reveal`), View on GitHub and Uninstall (asks
    first). Opening a row shows the description, panel toggle buttons (lit while open), script buttons,
    "Edit:" links into the Script Editor, the granted permissions, and picked folders with Add Folder….
  - It also has Install from URL / Folder / Zip and Reload.
  - `Pane::Plugin(u16)` is a plugin panel. The slot→panel mapping is persisted in `state.json`, so saved
    layouts reopen it. The title and icon come from a static slot table (`panel::slot_*`); each distinct title
    is leaked once.
  - **Window ▸ Plugins** has the Plugins pane, Install from URL…, and then each enabled plugin. A plugin with
    one panel and no scripts gets a check row; otherwise it gets a submenu of panels and scripts.
  - The install confirm shows name, author, version (and the version it replaces), source, contents,
    description, and one checkbox per permission. `run_programs` starts unticked and shows a warning when
    ticked. The confirm carries "Only install plugins from people you trust" and never auto-runs anything.
  - Fonts of enabled plugins go to `TextRasterizer::load_user_fonts` (in the frame hook, `try_lock`).
  - MCP: `plugins.list`, `plugins.install` (downloads, then shows the confirm), `plugins.uninstall` (asks
    first) and `plugins.enable`. A plugin can't call any of them, because `script_may_call` denies them.
- **Script Editor integration**: `scripts_ctl::tick` (one 4-line hook marked `feat/plugins`) sends Run on a
  file inside an installed plugin to `plugins_ctl::run_if_plugin_file`. A plugin script runs sandboxed, and a
  panel script reloads its panel. Neither gets the full access of the user's own scripts.
- **Example** `examples/plugins/hello-panel` (a panel, a script and a LUT, with no permissions). Docs:
  `docs/plugins.md` (the spec), `website/docs/plugins.mdx` (the guide with the complete example, in the
  sidebar after Scripting), CHANGELOG `## Unreleased ▸ Plugins`, `docs/customizing.md` rows and an agents.md
  "Where things are" row.
- Small edits outside my files:
  - `scripting.rs` makes `PRELUDE`, `show`, `lua_to_json` and `json_to_lua` `pub(crate)`.
  - `engine/mod.rs` adds `pub mod plugins`.
  - `Cargo.toml` adds `miniz_oxide = "0.8"` (already in the tree).
  - `layout.rs` adds two Pane variants plus their glyph and title arms.
  - `menus.rs` adds one call.
  - `mod.rs` adds a field and registry lines.

## Verified here (Linux container)

- `cargo check --target x86_64-pc-windows-gnu --all-targets` passes with no warnings from these files.
- `cargo test --target x86_64-pc-windows-gnu --no-run` builds the Windows test exe. Running it needs Windows:
  wine 9.0 aborts (`free(): invalid pointer`) here even on a hello-world exe, as the scripts session found.
- **The engine tests really ran.** A throwaway Linux harness crate compiles the real `src/engine/plugins/*.rs`
  and `src/scripting.rs` against stubs for the tool registry, settings and library paths, with `egui`
  standing in for `eframe`. Result: **25 of 25 pass**, five runs in parallel. They cover:
  - zip round-trip, path refusal and CRC;
  - GitHub link parsing, release replies and version compare;
  - manifest validation (every rule);
  - granted ⊆ declared;
  - install, disable, enable, update and uninstall from a local zip;
  - install and update against a mocked GitHub (a release with a zip asset, one without, no release, offline);
  - no overwriting of the user's library items, bad zips, subfolder links;
  - permission enforcement: a script without `network` can't fetch, declared-but-not-granted, other hosts,
    `mcp_tools`, `files` (folders, `..`, real read and write), `run_programs`, and `ui.*` outside panels;
  - the sandbox (`io`, `require`, `load`, `os.execute`, `debug`), and the runaway-loop budget;
  - the panel record/answer cycle, budget and error stop, ids, playback holding redraws;
  - **the shipped example's panel drawing, and its buttons making the exact tool calls**;
  - `assert_no_idle_repaint_plugin_panel`.

  The harness isn't committed; it lives in the session scratchpad.

## For the coordinator to run on Windows

- `cargo test plugin` runs the 25 engine tests plus `ui::app::plugins_ctl::tests`:
  `assert_no_idle_repaint_plugins_pane`, `plugin_tools_are_registered` and
  `plugin_panes_have_titles_from_their_slots` (a `Pane::Plugin(n)` survives a layout save).
- The whole suite, especially `tools_registry_tests`, `every_arg_spec_parses` and the palette, layout and menu
  tests, now that `Pane::ALL` has `Plugins`.
- Live checks:
  - Install from Folder… on `examples\plugins\hello-panel`, check the confirm, then open Window ▸ Plugins ▸
    Hello Panel. Click both buttons; each should be one undo step. Type a marker name and drag the slider.
    Edit `hello.luau` in the Script Editor and save; the panel should reload. Break it with
    `while true do end`; the panel should show the error and Retry.
  - Install from URL with a real repo, after pushing the example to a test repository or using a
    `tree/<branch>/examples/plugins/hello-panel` link of this repo once it is public on `main`. Then
    Update, Disable (the LUT leaves the Gallery), Enable, and Uninstall (the folder and the LUT go).
  - Close and reopen the app with a plugin panel docked; it should come back with its title.
  - Check that an open plugin panel doesn't run during playback. With `SE_FRAME_TRACE`, a panel should never
    show up while playing.
  - Run the size gate. Expected small: no new crates, about 2,300 lines of Rust. My estimate is +100-200 KB,
    which would need a `size:` line.

## Risks / notes

- Panels run on the UI thread, as the plan says. A tool call inside an idle `draw()` counts against the
  16 ms. A draw that lists a huge timeline every time, like the example's `project.clips()`, could hit it on
  a slow PC. The panel then stops with a clear error, and there is no stutter.
- `net.get` uses `curl.exe` per URL on a thread. There is no request body, POST or headers yet (GET only).
- Plugin scripts take no `-- @on` hooks or hotkeys yet. Wiring hooks means `palette_ctl::fire_hook` must
  also run enabled plugins' hook scripts through `plugins_ctl` with their permissions. I left it out to stay
  out of another workstream's function.
- `plugins.install` / `plugins.enable` are not in `mcp::tools::leaves_project`. The install confirm and the
  uninstall ask are the guard. Decide whether `plugins.enable` should be held for Allow too.
- Leaked pane titles: one small string per distinct panel title per run. This is bounded and only matters if
  a plugin renamed its panel thousands of times.
- An update whose plugin.json declares *new* permissions shows them in the confirm (pre-ticked like a fresh
  install, `run_programs` unticked). A hand-edited plugin.json can't widen access, because runtime checks use
  granted ∩ declared.
