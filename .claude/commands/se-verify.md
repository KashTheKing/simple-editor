---
description: Run Simple Editor's verification paths and report status against goals.md's budgets
---

Read [goals.md](../../goals.md) for the current budgets/targets first.

1. `cargo test`
2. `cargo run -- --selftest`
3. Build release (`cargo build --release`) and check the resulting exe size against the ~10 MB
   target in goals.md's Core goals — report the actual size and the delta.
4. If UI-relevant code changed since the last verify, take a real screenshot
   (`cargo run -- <video> --screenshot x.ppm` → `ffmpeg -i x.ppm x.png`) and look at it.
5. Report pass/fail for each step plainly, plus the binary size delta. Don't paper over a failure
   with a workaround unless asked — surface it.
