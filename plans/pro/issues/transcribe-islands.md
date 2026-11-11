# Island transcription for drift-free word timings

**Workstream:** `transcribe-islands` · **Plan:** [plans/pro/README.md](../README.md) · **Wave:** 3 · **Coding batch:** A · **Branch:** `feat/pro`

Read [agents.md](../../../agents.md), [goals.md](../../../goals.md), [notes.md](../../../notes.md), [ARCHITECTURE.md](../../../ARCHITECTURE.md) and the master plan [README.md](../README.md) before starting: its findings table gives the file:line evidence, and its "Execution" section gives this workstream's file ownership and the rules for the first coding pass.

## Scope

- `transcribe.run{islands:true}`: detect speech islands with the existing `autocut.detect`,
  transcribe each island separately, so word times cannot drift across long pauses (the known
  failure that makes whisper timings useless for cut points on long takes). This is what makes
  "cut the bad takes" by voice or text reliable.
- Transcription tools reply with a job id at once and are polled via `jobs.*`, so agent clients
  stop timing out on long clips.


## Files owned

`src/engine/transcribe.rs`, `src/ui/app/tools_transcript.rs`, `src/ui/app/transcript_ctl.rs`

## Done when

- Everything under Scope is implemented, or the gap is named in the PR/report with a reason.
- Structural rules hold: ToolDef row for every new Action / Pane / `pub fn(&mut self)` on Project, an `assert_no_idle_repaint_<context>` test for every new pane or window, no new crates, menus through `ui::menu::*`.
- Verification from the master plan's "Verification" section passes for wave 3 (run later, on a quiet machine: `cargo test`, `--selftest`, a viewed `--screenshot`, `scripts/size.ps1 -Note transcribe-islands`).
