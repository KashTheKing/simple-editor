# Orchestrator prompt — implement the remaining UI-overhaul issues

Paste everything below the line into a fresh Claude Code session (Sonnet, ultracode on) opened in
`D:\Projects\simple-editor`. It is restartable: gh issue/PR state is the source of truth, so if the
session dies, paste it again and it resumes from whatever is still open.

---

ultracode. You are the sole orchestrator for the Simple Editor UI/UX overhaul. Implement every
remaining open GitHub issue labelled `ui-overhaul` on KashTheKing/simple-editor, each in its own git
worktree, in dependency order, using ONE Workflow script that you author and run. Do not implement
anything yourself in the main checkout — your job is to schedule, verify, merge, and report.

## Read first

1. `CLAUDE.md` → `agents.md`, `goals.md`, `notes.md` (workflow, budgets, verification, gotchas).
2. `plans/ui-overhaul/README.md` — the master plan: registry protocol, frozen keymap, frozen modifier
   table, size plan and gate, wave order. `plans/ui-overhaul/issues/<name>.md` mirrors each issue.
3. Wave 0 is serial: #17 registries-schema-hooks → #18 size-diet (#16 split-god-files is merged).
   Nothing in waves 1–4 may start until #18 is merged. Waves 1 and 2 are up to 8 concurrent
   worktrees each; wave 3 is 3; wave 4 (#38 docs-refresh) is last.

## Discover the work (do this inline before authoring the script)

- `gh issue list --repo KashTheKing/simple-editor --label ui-overhaul --state open --json number,title,body,labels`
  and parse each body's `**Depends on:** #N (name), …` line into a dependency map. A dependency is
  satisfied when that issue is CLOSED (its PR merged into `main`).
- `git worktree list` and `git branch -a`: a worktree may already exist under
  `../simple-editor-wt/<name>` (there is one for `feat/audio-analysis`). Reuse it — rebase it onto
  current `main` — never create a duplicate worktree or branch for the same issue.
- Detect whether `scripts/size.ps1` exists yet (size-diet #18 creates it). Before it exists, measure
  `target/release/simple-editor.exe` bytes directly and compare with the previous merged size.

## The Workflow script you must author and run

Use the Workflow tool (load the `workflow-authoring` skill first). Requirements:

- `model: 'sonnet'` on every `agent()` call. Never put `maxLength` on any schema string. Keep the
  script LF-only. Normalise any identifier an agent returns (strip stray quotes) before using it as
  a key. Reject placeholder outputs (e.g. `summary: "test"`) and re-run that agent.
- A dependency-driven scheduler, not fixed waves: loop `while (open issues remain)`:
  `ready = open issues whose dependencies are all closed`; run `pipeline(ready.slice(0, 4), …)`
  through the stages below; after the batch, re-query gh and recompute `ready`. Cap concurrency at 4
  (each worktree does full Rust release builds). If nothing is ready but issues remain, log which
  PRs are blocking (open, unmerged) and stop with a report — never spin.
- Stages per issue (a `pipeline`, so a fast issue is never held back by a slow sibling):
  1. **implement** — the agent creates `git worktree add ../simple-editor-wt/<name> -b <branch>` from
     fresh `main` (branch name is in the issue header), optionally seeds `target/` by copying it from
     the main checkout to save a cold build, then implements the issue exactly as its body says:
     files, model/engine/UI changes, every listed MCP tool row, every listed test, actions with the
     frozen chords only. It edits only the files the issue owns plus its own `// ---- ws:<name> ----`
     registry sections. It runs `cargo test`, `cargo run -- --selftest`, a real screenshot for any
     UI change (`cargo run -- <video> --screenshot x.ppm` → `ffmpeg -i x.ppm x.png`, then LOOK at
     it), and the size measurement. It commits (conventional message on *why*, ending with the
     `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>` trailer), pushes, and opens a PR
     against `main` whose body is the issue's verification checklist with real results filled in,
     the measured exe delta, and `Closes #N`; end the PR body with
     `🤖 Generated with [Claude Code](https://claude.com/claude-code)`. Returns structured data:
     `{issue, branch, worktree, pr, tests_passed, selftest_passed, screenshot_checked,
     size_bytes_before, size_bytes_after, unresolved: []}`.
  2. **review** — three adversarial reviewers in `parallel`, each on the PR diff in that worktree,
     each with one lens: (a) correctness against the issue's acceptance criteria and tests;
     (b) goals.md budgets — no new crate, egui stays 0.33, exe delta within the issue's estimate or
     justified, idle CPU (no per-frame polling, timed repaints only via the sanctioned helper), no
     blocking rfd/Modal, playback-cache invariants (`video_dirty_spans` selective-invalidation tests
     still pass, ops mutate clips in place); (c) MCP parity and test pins — every new
     `pub fn(&mut self)` on Project has a ToolDef row, structural tests
     (`every_edit_op_has_a_tool`, `ui_action_covers_every_action`, `every_glyph_paints_a_picture`,
     `no_duplicate_defaults`, `default_layout_contains_every_pane`) pass, no chord outside the
     frozen keymap, no new Tool button. Each returns findings with file:line evidence and a fix.
  3. **fix** — one agent applies every finding in the same worktree, re-runs the full verification,
     pushes. Loop review→fix at most twice.
  4. **gate** — for #17 and #18 run the `wave0-gatekeeper-review` skill (Skill tool) on the PR; for
     every other issue a gatekeeper agent that re-runs `cargo test`, `--selftest` and the size
     measurement itself (never trusts the implementer's numbers) and returns PASS / FAIL / ASK with
     reasons.
  5. **merge** — on PASS: `gh pr merge <pr> --merge --delete-branch`, `git -C D:\Projects\simple-editor pull`,
     `git worktree remove ../simple-editor-wt/<name>`, confirm the issue auto-closed. On FAIL: one
     more fix round, then leave the PR open and record why. On ASK: leave the PR open, record the
     question, continue with other ready issues.
- Siblings in the same wave land one after another on `main`; before opening its PR, each
  implementer rebases onto current `main`. Registry sections are per-workstream, so conflicts should
  be trivial — resolve by keeping both sides; if a conflict is not trivial, stop that issue with ASK.

## Hard rules for every agent

- Never `git push --force`, never skip hooks, never run bare `cargo fmt` (use `rustfmt <file>` on
  touched files only), never bump egui/eframe, never add a crate. Any of these → ASK, not workaround.
- Don't treat a failing headless UI test as a product bug before checking the known harness
  pitfalls in agents.md/notes.md (modifiers on RawInput, double-click clock, logical-key matching).
- If the issue text turns out to be wrong against the real code, fix the approach, note the deviation
  in the PR body, and keep the issue's acceptance criteria as the target — don't follow it off a cliff.
- Only the orchestrator merges. Merging is required so later waves can start; if I say "PR only",
  skip the merge stage and stop when nothing is ready.

## Report

When the loop ends, print one table: issue → PR → state (merged / open-FAIL / open-ASK) → measured
exe delta → what is unresolved, plus the current `main` exe size against the ~10 MB goal, and update
`goals.md`'s "In progress / open" entry for the overhaul with what landed.
