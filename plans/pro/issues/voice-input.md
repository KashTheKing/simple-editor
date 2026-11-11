# Push-to-talk voice input through local whisper

**Workstream:** `voice-input` · **Plan:** [plans/pro/README.md](../README.md) · **Wave:** 3 · **Coding batch:** B · **Branch:** `feat/pro`

Read [agents.md](../../../agents.md), [goals.md](../../../goals.md), [notes.md](../../../notes.md), [ARCHITECTURE.md](../../../ARCHITECTURE.md) and the master plan [README.md](../README.md) before starting: its findings table gives the file:line evidence, and its "Execution" section gives this workstream's file ownership and the rules for the first coding pass.

## Scope

- Hold a key (an unbound chord, checked against `reserved_chords_are_free`) or the panel's mic
  button: record the default microphone with a **cpal input stream** (cpal is already a
  dependency; avoids the ~300 ms ffmpeg start that would clip the first word), write 16 kHz mono
  WAV, run the existing whisper path (`engine/transcribe.rs:212`), put the text in the prompt box.
  Setting: send automatically on release, or leave it editable.
- Fixes it depends on: unique temp WAV per run (`transcribe.rs:222`), and `whisper_dir` /
  model re-read when settings change (`transcribe.rs:40-48`). If whisper is not installed, the
  mic button offers the existing `transcribe.install`.


## Files owned

new `src/engine/voice.rs`, `src/ui/app/voice_ctl.rs`; the temp-WAV and settings re-read fixes in `src/engine/transcribe.rs`

## Done when

- Everything under Scope is implemented, or the gap is named in the PR/report with a reason.
- Structural rules hold: ToolDef row for every new Action / Pane / `pub fn(&mut self)` on Project, an `assert_no_idle_repaint_<context>` test for every new pane or window, no new crates, menus through `ui::menu::*`.
- Verification from the master plan's "Verification" section passes for wave 3 (run later, on a quiet machine: `cargo test`, `--selftest`, a viewed `--screenshot`, `scripts/size.ps1 -Note voice-input`).
