# Hand-off: scripts-editor (`claude/feat-scripts-editor`, from `origin/develop` 41c9b73)

Section "scripts-editor" of `plans/cloud-handoff.md`.

## Done

- **Window ▸ Scripts** (`ui::script_editor::scripts_menu`, `ui::app::scripts_ctl::menu`): each script is a
  submenu with Run (the old behaviour: `run_script_path`), Open, Open Folder (`contextmenu::reveal`), Rename…
  and Delete…. Under the list: Add Script and Open Scripts Folder. The submenu is drawn even with no scripts
  (the old `if !self.script_metas().is_empty()` guard is gone).
  - Rename… opens a small name box where the menu row was (`rename_box`, a WINDOW_DRAWER). Enter / Rename
    commits, Esc / Cancel closes, and an error (taken name, bad characters, reserved names) shows under the
    field. The extension is kept and a case-only change is allowed.
  - Delete… asks (`confirm::ask_app`), then `scripting::delete` moves the file to the Recycle Bin
    (`SHFileOperationW` + `FOF_ALLOWUNDO`). Nothing hard-deletes on Windows.
  - Add Script writes `New Script.luau` (`New Script 2.luau`, … when taken) from `scripting::TEMPLATE` with
    `create_new` (never overwrites), then opens it.
- **Script Editor pane** (`Pane::ScriptEditor`, `ui::script_editor`, `ui::app::scripts_ctl`):
  - The code editor is egui's `TextEdit` with a Luau layouter. It uses `html_code`'s colour table (`colour`
    is now `pub(crate)`) with a new Luau tokenizer: keywords, the script API's globals, calls, members,
    strings (long ones too), numbers, comments and header `@keys`. The HTML completion list is not used
    because it only knows HTML and CSS.
  - The header has the file name, an unsaved dot, Save, Run, Open Folder and Open Externally. Ctrl+S saves
    while the editor has focus, and the project's Save never sees that press because hotkeys stand down while
    a text field has focus.
  - Run saves, compiles (`scripting::check`) and runs the script on the next frame through `FRAME_HOOKS`
    (`scripts_ctl::tick` → `run_script_src`). The result line shows "ran without errors" or the first error
    line. Logs stay toasts, as before.
  - The pane looks at the file's modified time at most once a second while it is drawn. A changed file
    reloads. With unsaved edits it shows "Changed on disk" with Reload or Keep Mine instead.
  - Open Externally uses the program from `AssocQueryStringW(.luau)` through `ShellExecuteW`, or `notepad.exe`
    when nothing is registered (OpenWith.exe also counts as nothing).
  - Ways to open it: the top bar's toggle button (Terminal glyph, left of Jobs, lit while open), Window ▸
    More ▸ Script Editor, and the palette (`Action::ToggleScriptEditor`, unbound).
- **Settings ▸ General ▸ "Open scripts externally"** (`Settings::scripts_external`, off): when it's on,
  Open and Add Script use the external program.
- **MCP**: `scripts.create {name?, content?, open?}`, `scripts.rename {name, to}`, `scripts.delete {name}` and
  `scripts.open {name, external?}`. Create, rename and delete are in `mcp::tools::leaves_project`, so the
  Assistant guard holds them for Allow. `scripts.open` is held only with `external: true`, and that tool
  ignores the setting on purpose so it never launches a program unasked.
- Docs: the CHANGELOG `## Unreleased` lines, `website/docs/scripting.mdx` ("Managing and editing scripts")
  and the `docs/customizing.md` tool rows.

## Verified here (Linux container)

- `cargo check --target x86_64-pc-windows-gnu --all-targets` passes, with no new warnings from these files.
- `cargo test --target x86_64-pc-windows-gnu --no-run` builds the Windows test exe.
- **No test was run.** On Linux the crate doesn't build because winit rejects the platform. Wine 9.0 crashes
  in this container (`free(): invalid pointer`), even on a hello-world exe.

## For the coordinator to run on Windows

- `cargo test script` covers these tests:
  - `scripting::file_tests::*`: create / rename / delete on a scratch temp folder, that the template
    compiles and runs, and `valid_stem`.
  - `ui::script_editor::tests::*`: the tokenizer, `menu_shows_add_script_with_zero_scripts`, save and reload
    against the disk, and `assert_no_idle_repaint_script_editor`.
  - `ui::app::scripts_ctl::tests::*`: the registry and guard wiring.
- `delete_removes_the_file_and_refuses_a_missing_one` runs the real Recycle Bin path on Windows. It bins one
  temp file each run.
- Live checks:
  - The Scripts menu with zero scripts (delete `%APPDATA%\SimpleEditor\scripts\*`; `example.luau` is only
    recreated when the folder itself is missing).
  - Rename… box placement and Enter / Esc, Delete… into the Recycle Bin, and Add Script with the setting on
    and off.
  - Open Externally with and without a `.luau` association, and Ctrl+S inside the pane not triggering
    project Save.
  - Editing in Notepad while the pane is open.
  - The size gate: two small modules, so it's expected to be well under +64 KB.

## Risks / notes

- The top bar gets one more always-visible button. Moving it into the Scripts menu only is a one-line change
  in `menus.rs`.
- `scripting::check` builds a throwaway `mlua::Lua` per Run, which is cheap.
- `scripts.run` / the menu's Run still look scripts up by `m.name`, which is `@name` when a header sets it.
  The new tools accept the `@name` or the file stem.
- For the plugins session: `ui::script_editor::{ScriptEditor, highlight}` and `scripting::{create_in,
  rename, delete, open_external, check}` are reusable for plugin scripts and panels.
