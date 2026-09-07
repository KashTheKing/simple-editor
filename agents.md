# Agent Guide — Simple Editor

Read this file, [goals.md](goals.md), and [notes.md](notes.md) before touching this project.
They hold the constraints, priorities, and running history that the code alone doesn't carry.
If a request conflicts with something in goals.md (e.g. it would blow the binary-size budget
or add a dependency), flag the conflict instead of silently proceeding.

## What this project is

A fast, native Windows video trimmer/editor written in Rust (eframe/egui). See
[ARCHITECTURE.md](ARCHITECTURE.md) for the technical contracts (module layout, project format,
render pipeline) and [CHANGELOG.md](CHANGELOG.md) for what's shipped. This file is about *how
to work on it*, not what it does.

## Toolchain constraints (do not violate without asking)

- egui/eframe is pinned to **0.33** because the installed rustc is 1.89 (egui ≥0.34 needs
  1.92+). Don't bump egui without bumping the toolchain first, and don't do that silently.
- Decode is Windows Media Foundation (`windows` crate) primary, `ffmpeg.exe` child process
  fallback/export. No libclang on the machine — no ffmpeg-sys, no bindgen-based crates.
- New dependencies are a last resort — see [goals.md](goals.md) for the size/dependency budget.
  If you think one is justified, say so explicitly and why the stdlib/existing deps don't cover it.

## Verification (run before declaring anything done)

1. `cargo test` — 80+ unit/integration tests.
2. `cargo run -- --selftest` — real media end-to-end smoke test.
3. `cargo run -- <video> --screenshot x.ppm` then `ffmpeg -i x.ppm x.png` and actually view the
   PNG for UI/rendering changes. Type-checking is not feature verification — look at the output.
4. For perf-sensitive changes: `cargo test --release bench_4k_preview -- --ignored --nocapture`.
5. Check the release binary size against the goal in goals.md when a change adds code/deps.

Known test-harness gotchas that look like product bugs but aren't (modifier state on
`RawInput` vs. per-event, double-click timing in synthetic clocks, `consume_shortcut` logical
matching, `cargo fmt` reformatting the whole crate): see the `simple-editor-test-pitfalls`
memory, or ask — don't re-debug these from scratch.

## Workflow

### Branching

Non-trivial features/fixes get their own git worktree, not a branch checked out in-place:
```
git worktree add ../simple-editor-wt/<name> -b <type>/<name>
```
(`fix/`, `feat/`, `chore/` prefixes.) Small one-line fixes can go straight on a branch in the
main tree. Never work directly on `main`.

### Commits & PRs

- Conventional-ish commit messages, focused on *why*.
- Only commit when asked. Never `git push --force` to `main` or skip hooks.
- Open PRs against `main` on `KashTheKing/simple-editor` via `gh pr create`.

### Multi-agent work

For large changes, this project has used: split by module → parallel agents → merge → a
dedicated playback/perf-review pass → multi-lens review → verify → per-file-group fixups. Don't
default to this for small tasks — it's for genuinely large, module-spanning work.

## Live co-editing (MCP)

The app itself hosts an MCP server (`src/mcp/`) so an agent can edit the *live, running*
project — not just the files on disk. It speaks Streamable HTTP JSON-RPC on
`127.0.0.1:<port>/mcp`, toggled from in-app Settings. Tool calls are forwarded to the UI thread
and applied through the normal undo stack. Use `/se-coedit` to connect. Prefer this over
hand-editing the `.sedit` JSON project file when the user has the app open and wants to see
changes live.

## Slash commands (`.claude/commands/`)

| Command | Use for |
|---|---|
| `/se-fix` | Fix a bug or change the behavior of an existing feature |
| `/se-plan` | Turn a feature/change idea into GitHub issue(s) with an implementation plan |
| `/se-implement` | Implement a specific plan or GitHub issue |
| `/se-coedit` | Connect to the app's live MCP co-editing server |
| `/se-goal` | Record a new goal/priority in goals.md (no implementation) |
| `/goal` | Record a goal in goals.md **and** implement it now |
| `/se-notes` | Append a note to notes.md |
| `/se-verify` | Run the verification paths above and report status against goals.md budgets |
| `/se-review` | Review the current diff against this project's goals and conventions |

## Things to never do

- Don't bump egui/eframe past 0.33 without a toolchain bump and explicit sign-off.
- Don't add a dependency to save a few lines of code — see goals.md's dependency budget.
- Don't run bare `cargo fmt` and commit the result — it reformats the whole crate (pre-existing
  drift). Format only the files you touched, or diff-check before committing.
- Don't claim a UI change works without actually rendering it (screenshot or live app).
- Don't treat a failing headless UI test as a product bug before checking the known
  test-harness pitfalls above.
- Don't silently drop the proxy/DXVA/cache perf work's invariants (e.g. selective cache
  invalidation tests exist on purpose — see ARCHITECTURE.md and the stack memory).

## Registry protocol (UI/UX overhaul, wave 0b+)

The 23-workstream UI/UX overhaul (`plans/ui-overhaul/`) needs concurrent worktrees to add code
without ever sharing a merge hunk. `refactor/registries-schema-hooks` (wave 0b) landed the
mechanism; every later workstream follows it.

**The 23 workstreams, in the fixed wave-then-name order every shared table uses:**

wave 0: `registries-schema-hooks`, `size-diet`, `split-god-files` — wave 1: `audio-analysis`,
`audio-dsp-automation`, `color-engine`, `command-palette`, `forgiveness`, `player-rate-loop`,
`snap-engine`, `trim-model` — wave 2: `canvas-handles-monitor`, `export-deliver`,
`inspector-gallery`, `layout-modes-onboarding`, `media-library`, `source-monitor`,
`timeline-trim-gestures`, `transcript-captions` — wave 3: `pro-monitor`, `pro-timeline`,
`text-titles` — wave 4: `docs-refresh`.

**The five dispatch registries** (`src/ui/app/mod.rs`): `TOOL_TABLES` (`&[&[ToolDef]]`,
flattened by `mcp::tools::all()` for `tools/list`/`editor.tools()`/the palette),
`ACT_HANDLERS` (`&[fn(&mut App, Action) -> bool]`, tried before `act()`'s match — first `true`
wins), `FRAME_HOOKS`/`WINDOW_DRAWERS` (`&[fn(&mut App, &egui::Context)]`, polled once per frame /
inside `windows()`), `PANE_DRAWERS` (`&[fn(&mut App, &mut egui::Ui, Pane) -> bool]`, tried before
`draw_pane_inner()`'s match). Each is pre-seeded with 23 `// ---- ws:<name> ----` comment-only
marker lines in the order above — landing your feature means replacing YOUR line with a real
entry (e.g. `tools_<ws>::TOOLS,`), never touching a neighbour's line. `hotkeys.rs`'s `actions!`
macro body, `ui/tools.rs`'s `Glyph` enum/`ALL`/`name()`/`draw_glyph()` (3 of its 5 call sites —
`from_name` is generic over `ALL` and needs no per-variant arm), and `Settings`'s struct +
`Default` impl carry the same 23 markers.

**Tool registry:** every MCP tool is a `ToolDef { name, desc, args, kind: ToolKind, run }` living
beside its handler in a `ui::app::tools_<group>.rs` file (a `pub const TOOLS: &[ToolDef]` row per
tool, registered once in `TOOL_TABLES`). `ToolKind::Mutate` replaces per-caller
snapshot/undo bookkeeping — a Project mutator only needs a `pub fn(&mut self)` plus a `ToolDef`
row; `tools_registry_tests::every_edit_op_has_a_tool` fails the build if a new
`src/model/ops/*.rs` fn has neither a `ToolDef` row nor a recorded `OP_INTERNAL` reason.

**Rules for every future PR:** edit only your own marker line/section in each shared table —
never another workstream's line, never a file this wave's plan names as another workstream's
exclusive owner. A PR that adds an `Action` variant, a `Gesture` kind, a `Pane`, or a
`pub fn(&mut self)` on `Project` must add a matching `ToolDef` row in the same PR. Run
`scripts/size.ps1 -Note <ws>` (once size-diet lands it) and put `size: ±N KB` in the PR body.
