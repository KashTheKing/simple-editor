# In-app assistant pane with Claude Code, custom CLI, API key and external backends

**Workstream:** `assistant-pane` · **Plan:** [plans/pro/README.md](../README.md) · **Wave:** 3 · **Coding batch:** B · **Branch:** `feat/pro`

Read [agents.md](../../../agents.md), [goals.md](../../../goals.md), [notes.md](../../../notes.md), [ARCHITECTURE.md](../../../ARCHITECTURE.md) and the master plan [README.md](../README.md) before starting: its findings table gives the file:line evidence, and its "Execution" section gives this workstream's file ownership and the rules for the first coding pass.

## Scope

- `Pane::Assistant`: prompt box, context chips (selection / range / comment, attached
  automatically from `editor.context`), a running log of what the agent says and which tools it
  calls, Stop, and "Undo all" per request. Also reachable from Ctrl+K with a prefix. Canned,
  user-editable prompts as buttons (Remove silences, Cut filler words, Caption this, Tighten
  selection, Add chapter markers).
- Backends (Settings ▸ General, next to the MCP section):
  - **Claude Code (default)**: the app spawns
    `claude -p <prompt> --output-format stream-json --verbose --mcp-config <temp file>
    --strict-mcp-config --allowedTools mcp__simple-editor --resume <session>` and reads the JSON
    lines on a worker thread (the `transcript_ctl.rs` job pattern). The agent gets the editor's
    tools only: no shell, no file access. One session id per project so follow-ups keep context.
    All flags confirmed present in the installed `claude` 2.1.316.
  - **Custom command**: a template with `{prompt}` and `{mcp_url}` for Codex or any other agent
    CLI; output shown as plain lines. Ship a Codex preset after checking `codex --help` (Codex is
    not installed on this machine, so its flags are unverified).
  - **API key**: stored in Windows Credential Manager (not settings.json) and passed to the
    spawned CLI as `ANTHROPIC_API_KEY` / `OPENAI_API_KEY`. A CLI-free in-app API client is not in
    this plan; add it if the app must run on machines without any agent CLI.
  - **External**: requests queue for whichever MCP client is waiting on `assistant.wait`.
- Using the panel switches the MCP server on (it is off by default) and wraps each request in one
  undo group.
- Guard rails: tools that leave the project (`export.*`, `project.open/new`, `settings.set`,
  `scripts.run`, `media.convert`) need a click in the app when an agent calls them
  (`ui.confirm_pending` plumbing exists).
- Needs `assert_no_idle_repaint_assistant`, a toggle Action + ToolDef row, a Glyph.


## Files owned

new `src/ui/assistant_ui.rs`, `src/ui/app/assistant_ctl.rs`, `src/engine/agent.rs`, `src/ui/settings_ui/assistant.rs` (never `general.rs`: it has uncommitted maintainer changes on `main`)

## Done when

- Everything under Scope is implemented, or the gap is named in the PR/report with a reason.
- Structural rules hold: ToolDef row for every new Action / Pane / `pub fn(&mut self)` on Project, an `assert_no_idle_repaint_<context>` test for every new pane or window, no new crates, menus through `ui::menu::*`.
- Verification from the master plan's "Verification" section passes for wave 3 (run later, on a quiet machine: `cargo test`, `--selftest`, a viewed `--screenshot`, `scripts/size.ps1 -Note assistant-pane`).
