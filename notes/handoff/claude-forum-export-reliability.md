# Hand-off: forum-export (`claude/forum-export-reliability`)

Session B of `plans/shotcut-forum-insights.md`: export reliability and clarity. Branched from `origin/develop`
at `a3a0538`. The same commits are also on `claude/inspiring-babbage-ic6xxu` (this cloud session's own branch).

## What's done

| Plan item | Where | State |
|---|---|---|
| 1. Error classifier (ffmpeg stderr to one sentence + one fix) | `engine/export_doctor.rs` `classify`; toast in `ui/app/export_fix.rs` `failed`; Jobs "Recent" row shows the sentence (`jobs_pane.rs`) | done |
| 2. Auto-retry on software x264 after any hardware failure | `export.rs` `run_export` (one new match arm) + `export_doctor::retry_in_software` | done |
| 3. Stuck-job watchdog | `Progress::idle_for` (`export.rs`), `export_doctor::STALL` (60 s), Jobs row note + **Retry in Software** button (`jobs_ui.rs`, `jobs_pane.rs`) | done |
| 4. Find the Problem (bisect at low quality, name the clip, jump there) | `export_doctor::{segments, bisect, find_problem}`; job + result in `export_fix.rs` | done |
| 5. Missing-files prompt with Relink… | `export_fix::preflight` (replaces the Yes/No in `files.rs` `act_export`) | done, see "Rebuild proxies" below |
| 6. Estimated size **and time**, social re-encode note, last preset per project | `export_ui.rs` `estimated_time`, `social_note`; `Settings.export_mpx_rate`, `Settings.export_tile_by_project` | done |
| 7. Open the finished export in the player | **Play** on the success toast (`Action::PlayLastExport`) | done |

Also: a pre-export lint (`export_doctor::lint`) flags a clip below 0.01x / above 100x speed, a broken clip length or
a bad project frame rate (forum 51460's 0.0025x clip), with **Go to Clip**. Three new Actions (Retry Export in
Software, Find Export Problem, Play Last Export; "File" group), three MCP tools (`export.diagnose`,
`export.find_problem`, `export.retry_software`). Docs: `website/docs/export.mdx` (new "Before the export starts" and
"When an export fails"), `docs/customizing.md`, `website/docs/shortcuts.md` (hand-edited in the generator's order;
regenerate with `--dump-hotkeys` to be sure), CHANGELOG under `## Unreleased`.

## Verified here (Linux container)

- `cargo check --target x86_64-pc-windows-gnu --all-targets`: compiles, no new warnings in touched files.
- **No test ran.** The crate has no lib target and is Windows-only, so `cargo test` can't build for Linux, and wine
  is broken in this container (even `wine cmd /c echo` aborts with "free(): invalid pointer"). The test binary
  for the Windows target does build.

## Untested (please run on Windows)

- `cargo test export_doctor export_fix jobs_ui` - new tests: classifier on 13 injected error strings, retry rule,
  watchdog, `Progress::idle_for`, time estimate, lint (0.0025x clip), segments, bisect (every position, a
  two-sided failure, cancel), culprit clip order, test-export options, the stalled Jobs row button, time/social text,
  and a source check that `finish_export` routes through `export_fix`.
- Full `cargo test` and `--selftest`: `tools_registry_tests` should pass (new Actions resolve via `ui.action`; the
  new ToolDefs are not `Project` ops).
- Live: force a hardware failure (pick `h264_nvenc` on a PC without NVIDIA) and check the export finishes with
  "stopped partway - finished in software" or "failed to start - retried in software" in the Jobs row; a locked
  output file gives "The file can't be written" with Choose Another File; delete a source file and check the
  Missing media window's Relink…; add a 0.0025x clip and check Go to Clip; run Find Export Problem on a project with
  a damaged file and check it selects that clip; a 60 s stall (hard to force; pausing ffmpeg in Process Explorer
  works) shows Retry in Software; check the "~40 s to export" text and the TikTok note in the Export pane
  (screenshot for the docs: `scripts/docs-shots.ps1`).

## Risks and merge notes

- **gap-alpha-export** touches `export.rs` heavily. My `export.rs` changes are three small hunks: the `Progress`
  struct/`new`/`set` (+`idle_for`), `start_export` (remembers the export for Retry / Find), and one new arm in
  `run_export`. Everything else is in new files, as the hand-off asked. gap-alpha adds an `ExportOptions` field: the
  literal in `export_doctor::tests::test_exports_are_small_software_and_even` will then need that field (a compile
  error, on purpose).
- `export_doctor::remember` keeps a clone of the last exported project in a static (one at a time) so a retry
  re-runs exactly what failed. Memory: one `Project` clone.
- Retry in Software on a running export cancels it first; `finish_export` then skips the "Export cancelled" toast
  and the queue clear while a retry is pending (`export_fix::retrying`).
- Find the Problem uses the CPU frame source and x264 at 320 px wide, CRF 40: a failure that only happens on the
  GPU path or with the hardware encoder is reported as "the clips are fine, the encoder or the size is the problem"
  (with Retry in Software), which is the right answer for that case.
- Each test section that makes no progress for 30 s counts as failing (the "stuck at 99%" case).
- **"Rebuild proxies" in the missing-files prompt was left out on purpose**: Simple Editor exports from the original
  files, never from proxies, so a deleted proxy can't block an export (the Shotcut failure in forum 51389 can't
  happen here). The offline check only lists original files.
- The time estimate starts at 100 Mpx-frames/s (about 1080p50) until the first finished export measures this PC;
  each export then counts for half.
- The preset card is remembered per project *file* in settings (not in the project, so an export does not make the
  project "unsaved"); unsaved projects are not remembered. Applied once per project per session when the Export
  pane opens through `act_export`.
