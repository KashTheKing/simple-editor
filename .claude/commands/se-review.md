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

Report findings plainly — file:line, what's wrong, what it should be instead. Don't apply fixes
unless asked.
