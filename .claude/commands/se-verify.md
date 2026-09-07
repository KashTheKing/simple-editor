---
description: Run Simple Editor's verification paths and report status against goals.md's budgets
---

Read [goals.md](../../goals.md) for the current budgets/targets first.

1. `cargo test` — all green, no count regression unaccounted for.
2. `cargo run -- --selftest` — every step PASS (or an accepted SKIP), including the `idle_repaint`
   line: idle-CPU-0% gate for timed-repaint code added from wave 0 onward (size-diet). A FAIL there
   fails this verify pass.
3. `scripts/size.ps1` instead of a manual `cargo build --release` + eyeball: it builds release, appends
   `<sha>,<bytes>,<note>` to `size_log.csv`, and prints the delta vs the previous row. Fail this verify
   pass when that delta exceeds +65536 bytes and the PR body has no `size: +N KB — reason` line; a
   delta over +300 KB needs a named offset regardless.
4. If UI-relevant code changed since the last verify, take a real screenshot
   (`cargo run -- <video> --screenshot x.ppm` → `ffmpeg -i x.ppm x.png`) and look at it.
5. Report pass/fail for each step plainly, plus the binary size delta. Don't paper over a failure
   with a workaround unless asked — surface it.
