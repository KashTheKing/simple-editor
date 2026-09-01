---
description: Fix a bug or change the behavior of an existing feature in Simple Editor
argument-hint: <bug description or feature to change>
---

Read [agents.md](../../agents.md), [goals.md](../../goals.md), and [notes.md](../../notes.md) first.

Task: $ARGUMENTS

1. Reproduce/locate the root cause — grep every caller of the function you're about to touch,
   don't patch just the symptom path. Check the known test-harness pitfalls in the
   `simple-editor-test-pitfalls` memory before assuming a failing test is a real bug.
2. Make the smallest correct change. Don't refactor or add abstraction beyond what the fix needs.
3. If the fix touches a dependency or grows the binary meaningfully, check it against the budget
   in goals.md and say so.
4. Verify per agents.md's Verification section (`cargo test`, `--selftest`, and a real screenshot
   for anything UI-visible). Don't declare done on type-checking alone.
5. If you had to work around something (toolchain limit, egui quirk, etc.) worth remembering
   later, add a line to notes.md.
