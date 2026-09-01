---
description: Turn a feature/change idea into GitHub issue(s) with an implementation plan
argument-hint: <feature or change to plan>
---

Read [agents.md](../../agents.md), [goals.md](../../goals.md), and [notes.md](../../notes.md) first.

Task: plan out — $ARGUMENTS

1. Check goals.md's core goals and "things to avoid" before designing anything — a plan that
   blows the size/dependency budget or reproduces a listed anti-pattern needs to say so up front,
   not get discovered during implementation.
2. Read the relevant existing code (ARCHITECTURE.md + the actual modules) so the plan is grounded
   in how this codebase actually works, not a generic approach.
3. Write a concrete implementation plan: files touched, new types/functions, verification steps,
   and anything it puts at risk (perf invariants, cache tests, toolchain constraints).
4. If the work is large enough to split, break it into multiple issues with a clear dependency
   order; otherwise one issue is fine. Small enough to just fix directly? Say so instead of
   manufacturing an issue.
5. Create the issue(s) with `gh issue create` against `KashTheKing/simple-editor` — title short,
   body has the plan, checkboxes for the verification steps. Confirm with the user before creating
   if anything about scope is ambiguous.
6. Report the issue URL(s) back.
