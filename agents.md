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
