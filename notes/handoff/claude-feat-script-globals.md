# claude/feat-script-globals — plain Luau globals

The user's requests, verbatim: "can you make print the same as editor.log" and "i dont like the editor global
either, its useless. just make tools and tool globals, use print, warn, error like normal".

## Done
- **Globals in all four contexts** (Script, Hook, Agent, Plugin): `tool(name, args)`, `tools()`, `print(...)`,
  `warn(...)`; hooks get `event`. Agent and Plugin keep `json(v)` (was `editor.json`); Agent gets `purpose`
  (was `editor.purpose`). `error(...)` is plain Luau.
- **Output**: `print` formats like the Agent context did (`scripting::show`: tables as compact JSON,
  arguments tab-joined).
  - Script/Hook: lines go into `logs`; the app shows them as toasts (`App::script_line_toast`).
  - Agent: into `ApiRun::output`.
  - Plugin: into `Env::log`, which becomes toasts for scripts and lines under the panel.
- **`warn`**: the same line with the prefix `scripting::WARN` (`"warning: "`). `scripting::warning(line)`
  reads it back. Toasts use `ToastKind::Warn`, panel lines use the warning colour, and Agent output carries
  the `warning: ` line as text. I kept `Vec<String>` instead of a typed line, so no caller signature changed.
- **`run` / `run_hook`** now share one body (`run_own`), so they can't drift.
- **Plugin BOOT** binds the host functions under internal `core.*` keys. `print = core.print` replaces
  `print = editor.log`.
- **Hidden `editor` alias** (`scripting::legacy_editor`, and the `editor = {...}` line in plugin BOOT): `tool`,
  `tools`, `log` (= `print`), plus `event` (Hook) or `json` / `purpose` (Agent) or `json` (Plugin).
  - It is not in `scripting_api::API`, not completed and not in the docs.
  - `api_matches_the_registered_names` filters `editor` / `editor.*` out of the registered names, with a comment saying why.
- **Script Editor**: a new `Diag::hint` field.
  - Old code gets a quiet hint (weak text colour): "editor.tool is now tool", "editor.log is now print",
    "editor.event is now event", or for a bare `editor`, "editor is gone: use tool, tools, print, warn".
  - A file that declares its own `editor` gets no hint.
  - Tool-name completion and hover work in `tool("…` and, quietly, in the old `editor.tool("…`.
- **Docs and prompts**:
  - `docs/scripting-api.md` (regenerated), `docs/customizing.md`, `docs/plugins.md`,
    `website/docs/scripting.mdx` and `plugins.mdx`, and `examples/plugins/hello-panel/**`
  - `scripts/gen_docs.luau`, ARCHITECTURE.md, agents.md, goals.md and the CHANGELOG (`## Unreleased`)
  - the Assistant's TASK_BRIEF and `tasks.add` arg doc, and the `scripts.eval` description
  - the script template and example
- **Tests**:
  - Every `editor.*` in tests and fixtures is updated.
  - New: `scripting::globals_tests::print_warn_error_tool_and_tools_are_plain_luau_in_every_context`. In
    Script, Hook, Agent and Plugin it checks that `print` lands in the logs or output, that `warn` is a
    `WARN` line, that `error` fails the run with its message and stops it, that `tool` / `tools` are globals,
    and that old `editor.tool` / `editor.log` (and `editor.event` in the hook) still work.
  - New: `script_intel::old_editor_code_gets_a_quiet_hint`.

## Verified here (Linux container)
- `cargo check --target x86_64-pc-windows-gnu --all-targets`: clean, and no warnings in the touched files.
- The crate's tests don't build on Linux (winit), so they ran in a scratch harness crate. It compiles the
  **real** `scripting.rs`, `scripting_api.rs`, `engine/plugins/runtime.rs` (the real plugin `Vm` and BOOT)
  and `ui/script_intel.rs`, plus the token part of `ui/script_editor.rs`. The rest is shims: an 11-row
  `mcp::tools`, `Settings::dir` and the plugin types. **32/32 pass**, covering:
  - every `scripting`, `scripting_api` and `script_intel` test, the plugin runtime tests and `luau_tokens`
  - the new cross-context test and the hint test
- One condition on that run: `api_matches_the_registered_names` passed only with `package.cpath` masked.
  mlua registers that name on Linux but not on Windows (develop's last commit removed it from the API for that
  reason). The mask was for that one run and is not committed.
- The same run caught a real bug, now fixed: `event` is a global table now, so the name walker listed the hook
  payload's own keys (`event.clip_ids`). The walker skips `event`'s fields.
- `docs/scripting-api.md` was regenerated through the harness (`SE_WRITE_API_DOCS=1`) and is current.

## Not verified / for the coordinator
- I couldn't run the app here: a live check on Windows is still needed.
  - Run a script with `print` and `warn`: one plain toast, one warning toast.
  - Fire a hook.
  - Open the hello-panel plugin.
  - Open an old `editor.tool` script in the Script Editor: the hint should be grey, not orange.
- `script_editor.rs`'s headless typing test (`typing_a_member_opens_the_list_and_enter_takes_it`) now types
  `utf8.`, then Down, then Enter (giving `utf8.charpattern`), then ` .. tool(` for signature help. It was not
  run here (it needs egui), so run it on Windows.
- Run `cargo test` on Windows, mainly `api_matches_the_registered_names` (real `mcp::tools`) and the
  `globals_tests` test.

## Risks
- Inside the Script/Hook VM, `print` used to write to stdout. It now goes to toasts, so a script that prints
  in a loop makes many toasts. Toasts dedupe repeated identical lines, though, and `print` was invisible before.
- A user script that defined its own global `tool`, `tools`, `warn`, `json` or `event` shadows ours. That is plain Luau behaviour.
