---
description: Review the current diff against Simple Editor's goals and conventions
argument-hint: [optional PR number or branch — defaults to the working diff]
---

Read [agents.md](../../agents.md) and [goals.md](../../goals.md) first.

Review target: $ARGUMENTS (default: current uncommitted diff / branch vs main)

Beyond ordinary correctness review, specifically check the diff against this project's stated
priorities:

1. **Size/deps** — any new dependency? Any egui/toolchain version bump? Flag it against the
   budget in goals.md even if the diff is otherwise fine.
2. **Perf invariants** — does it touch playback/cache/decode paths? Check it doesn't defeat
   selective cache invalidation, the DecoderPool LRU, proxy routing, or DXVA gating (see
   ARCHITECTURE.md and the perf notes in agents.md).
3. **UX principles** — if it's UI-facing, check it against goals.md's UX principles section
   (contextual disclosure, predictable snapping, no panel-jumping, direct manipulation before
   numeric fields, etc.) and the "things to avoid" list.
4. **Test-harness pitfalls** — if new UI tests are included, sanity-check them against the four
   known gotchas (modifiers on RawInput, double-click timing, shortcut specificity, `cargo fmt`
   scope) so a real bug isn't dismissed as a test artifact, or vice versa.
5. **MCP/tool parity** — flag a new `Action`/`Gesture`/`Pane`/`pub fn ...(&mut self)` on `Project`
   with no matching `ToolDef` row (`every_edit_op_has_a_tool`/`OP_TOOLS` should catch this at build
   time, but check the diff directly too), a new pane/overlay/window with no
   `assert_no_idle_repaint_<context>` test, and a documented `-- @on <event>` with no `fire_hook(...)`
   call site anywhere in the diff (a plan or doc that *names* an event isn't the same as a PR that
   *wires* it — see ARCHITECTURE.md's "Registries" section for what "real" means here).
6. **Size gate** — flag a diff that grows compiled code (new `.rs` lines, a new dependency) without
   a `size: +N KB` / `size: -N KB` note in the PR body (see agents.md's "Size gate" for the exact
   thresholds: >64 KB needs a reason, >300 KB needs a named offset).
7. **Wave ordering** — flag a plan whose `depends_on` includes a workstream in the *same* wave —
   it must move to the next wave instead (see agents.md's "Concurrent worktree protocol").

Report findings plainly — file:line, what's wrong, what it should be instead. Don't apply fixes
unless asked.
