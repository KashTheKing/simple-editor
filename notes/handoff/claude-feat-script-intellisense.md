# Handoff: claude/feat-script-intellisense

Request: "The script editor needs intellisense". Branched from `origin/develop`.

## Done

- **`src/scripting_api.rs`**: the one API description. One `ApiItem` row per name (globals, tables, functions with
  `name: type` parameters and return types, values), with a one-line doc and a context bit set: `SCRIPT` (Scripts
  folder / Run / hotkey), `HOOK` (`-- @on`, adds `editor.event`), `AGENT` (`scripts.eval`: prelude `library` /
  `project` / `query`, `print`, `editor.json` / `purpose`), `PLUGIN` (BOOT's `net` / `fs` / `programs` / `ui`,
  `plugin`). Luau 0.663's standard library as each sandbox keeps it (e.g. `debug`, `require`, `getfenv` only for
  the user's own scripts). Keywords and type names too.
  - `api_matches_the_registered_names` starts each VM the way the app does (`scripting::run`, `run_hook`,
    `run_api`, plugin `Vm::new`), lists what it really registers and fails both ways (registered without a row,
    or a row that isn't registered). Names set after `sandbox(true)` can't be listed with `pairs(_G)`, so the
    walker also reads each candidate directly: every described global plus names scraped from the registration
    code (`globals().set("x"`, `x = ` / `function x.` lines in PRELUDE / BOOT).
  - `docs/scripting-api.md` is generated (`markdown()`); `api_docs_are_current` fails when it's stale.
    Regenerate: `SE_WRITE_API_DOCS=1 cargo test api_docs_are_current`. `docs/customizing.md` links it.
  - The Luau highlighter's hand-kept `GLOBALS` list is gone; it reads the API table.
- **`src/ui/script_intel.rs`** (no egui): lexer, declaration scan (locals, local functions, params, `for` vars,
  global functions / assignments, `M.f` / `M.x` members, `type X`), completion (prefix > case-blind prefix > word
  start > fuzzy, file locals declared before the cursor, members after `.` / `:`, string methods on `s:`, types
  after `local x:`, tool names inside `editor.tool("…`), signature help (`call_at`: commas only at the call's own
  depth; strings, comments, nested calls and tables don't count), hover, diagnostics (`syntax` = Luau compile via
  `scripting::check`, never run; `unknown_globals` skips members, table keys, type annotations, `type` decls).
- **`src/ui/script_editor.rs`** pane: the list (kind icon, name, signature, doc line under it; selected row in
  `selection.bg_fill` with `tools::on_accent_over` text), Up/Down, Tab/Enter, Esc, Ctrl+Space, click to take;
  signature help above the line with the current parameter in the accent colour; hover docs; red / warning
  squiggles with the message on hover; the status line shows `Line N: message (+k more)` before the run result.
  The check runs on a `script-check` thread 300 ms after typing stops (immediately on open/reload) and wakes the
  UI with `request_repaint`; no repaint while idle.
- `ScriptEditor::plugin` is set on open (`plugins_ctl::owns_file`), so a plugin's file completes and checks
  against the plugin API.
- CHANGELOG under `## Unreleased`.

## Verified here (Linux cloud box)

- `cargo check --target x86_64-pc-windows-gnu --all-targets`: clean, no new warnings.
- The crate's tests can't build on Linux (winit) and Wine crashes in this container ("free(): invalid pointer" even
  for a hello-world exe). So I ran the tests through a scratch harness crate that compiles the **real**
  `scripting.rs`, `scripting_api.rs`, `ui/html_code.rs`, `ui/script_intel.rs`, `ui/script_editor.rs` with shims for
  the rest (a 2-tool `mcp::tools`, a stub `engine::html::parse`, menu/tools stubs, and a plugin `Vm` that runs the
  real BOOT text from `runtime.rs` with the same sandbox steps as `Vm::new`). All new tests pass there:
  prefix filter and ranking, member completion (`editor.`, `project.`, `math.`, `string.`, `s:`, file tables,
  tool names), local symbol scan, signature arg index (nested calls, strings, long strings, comments, tables,
  methods), syntax diagnostic line number, unknown globals, hover, the API-vs-registrations check in all four
  contexts, docs current, a headless typing test (type `editor.`, Down, Enter -> `editor.tool`, `(` opens
  signature help, Esc keeps focus), and `assert_no_idle_repaint_script_editor` (now waits for the check thread).
  The two harness failures are its shims, not the product (`errors_and_budget` wants >10 tools,
  `html_code::completion_knows_where_it_is` needs the real HTML parser).
- Rendered the pane headlessly (CPU rasteriser over egui's tessellated output) and looked at the list, member list,
  signature help, squiggles + hover message, and hover docs: aligned and readable in the dark theme.
- **Cost**: the whole check of a 2,000-line script is ~4.6-5.6 ms (lex + scan ~1.4 ms, the rest Luau's compile;
  harness, deps at opt-level 3, main code at 1). Over the 2 ms budget, which is why it runs off the UI thread.
  The list and signature help re-lex on the UI thread only when the text or cursor changes (~1.4 ms at 2,000 lines;
  nothing per idle frame), hover only when the pointer moves to another character.

## Not verified (needs Windows)

- `cargo test` on Windows, especially `api_matches_the_registered_names` with the real plugin `Vm` and the real
  tool catalogue, and the live pane: real theme colours / `on_accent_over`, IME / Ctrl+Space on a real keyboard,
  the popup near the pane's bottom edge, a long file's typing feel.

## Notes / risks

- There is no `video` global in the Luau API (the request's example); the member test uses `editor.`, `project.`,
  `math.` and `string.`. The video bridge is `editor.tool(...)`.
- The unknown-global check is "declared anywhere in the file" (no scopes), as asked. `M.x = 1` on an undeclared
  `M` still warns on `M`.
- Enter accepts while the list is open; the list closes by itself once the typed word equals the only/best label
  (so Enter after typing `end` is a new line).
