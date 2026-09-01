---
description: Implement a specific plan or GitHub issue for Simple Editor
argument-hint: <issue number, URL, or pasted plan>
---

Read [agents.md](../../agents.md), [goals.md](../../goals.md), and [notes.md](../../notes.md) first.

Target: $ARGUMENTS

1. If given an issue number/URL, `gh issue view` it to pull the full plan; otherwise use the
   plan text given directly.
2. For non-trivial work, create a worktree per agents.md's branching section
   (`git worktree add ../simple-editor-wt/<name> -b feat/<name>` or `fix/<name>`).
3. Implement following the plan, but don't follow it off a cliff — if something in the plan
   turns out to be wrong once you're reading the real code, fix the plan, don't force it.
4. Verify per agents.md (`cargo test`, `--selftest`, screenshot for UI changes, release size check
   if the change adds code/deps).
5. Update goals.md's achieved/in-progress lists if this closes out a goal, and notes.md with
   anything non-obvious hit along the way.
6. Report status; only commit/open a PR if asked.
