# Hand-off: styles-context (`claude/feat-styles-context-9e1d7f`)

Cloud session, 2026-11-20. Scope from `plans/cloud-handoff.md` § styles-context.

## State

`origin/develop` is merged in (no conflicts). Every scope item was already written on the branch before the pause.
This session made it compile, ran what can run on Linux, and fixed what that turned up.

| Scope item | Where | State |
|---|---|---|
| Project Context pane (per-project Markdown + digest of planner, notes, moodboard, markers/comments), read by the Assistant on every request | `src/model/context.rs`, `src/model/ops/context.rs`, `src/ui/styles_ui.rs` (`context_pane`), `src/ui/app/styles_ctl.rs`, `assistant_ctl.rs` (`for_request` in chat, task and comment requests, and now the external backend) | done |
| Editing Styles library (Markdown + front matter), `/style <name>`, `/style off` | `src/engine/styles.rs`, `library/styles/*.md` (6 built-ins), Assistant ▸ Editing Styles tab | done |
| Learn this style (our measurements + SmolVLM2 descriptions) | `src/engine/style_learn.rs`, `src/engine/vlm.rs`, `library/bin/vlm.json`, Learn window in `styles_ui.rs` | done |
| MCP `context.*`, `styles.*` | `styles_ctl::TOOLS` (12 tools) | done |
| Docs | `website/docs/project-context.mdx`, `website/docs/editing-styles.mdx`, CHANGELOG under `## Unreleased` | done |

## What this session changed

1. **The built-in `shorts` style is now `short-form`** (`library/styles/short-form.md`). A built-in skill is already called
   `shorts`, and `built_ins_parse_with_rules` asserts no built-in style shares a skill's name, so that test would have failed.
   Its companion skill `/style-shorts` keeps its name. Docs and CHANGELOG updated.
2. **Learn this style missed cuts between similar colours.** ffmpeg scores yellow to white at 0.25, under the 0.3
   threshold. Scores from 0.12 up are now candidates (`scene_scores`, `parse_scene_scores`). A weak candidate counts only
   when the 10 fps frames jump across it at least 4x more than they move in the 0.3 s around it (`confirm_cuts`), so pans,
   zooms and dissolves don't turn into cuts. `analysis::scene_cuts` itself (owned elsewhere) is untouched.
3. **A found dissolve was one frame too long at each end** (0.9 s for a 0.7 s mix). The run's held end frames are trimmed now.
4. **`colour_of` contrast** used `sqrt(E[y²] - E[y]²)` in f32, which read about 1e-3 on a flat frame. It's two-pass now.
5. **The external-agent backend** queued requests for `assistant.wait` without Project Context. `external_text()` adds it,
   with a test (`external_requests_carry_project_context`).
6. **The digest's Planner / Moodboard / Markers buttons** toggle their pane and stay lit while it is open (house rule). They
   used to only surface it.
7. **Project Context could drop a keystroke**: when the box lost the keyboard in the frame of an edit, `Commit`
   overwrote `Edited`, and the next frame reset the buffer to the project's older text. The edit goes out first now, and
   the undo step closes on the next frame (`CtxState::commit_next`). It was found by reading the code; no headless test
   covers it, because focus loss and a text change in one frame are hard to synthesise.

## Verified here (Linux container)

- `cargo check --target x86_64-pc-windows-gnu --all-targets`: compiles, 0 errors. The warnings in the new files are minor
  (`styles::get_in` and `entry_of` unused).
- `cargo build --target x86_64-pc-windows-gnu --tests` builds the test exe. Wine 9.0 aborts at start-up in this container
  (`free(): invalid pointer`, even for `cmd /c echo`), so the Windows test binary was **not run**.
- The crate can't build for Linux (winit, Win32). So `engine/styles.rs`, `engine/style_learn.rs` and `engine/vlm.rs`
  were compiled into a scratch harness, with small stubs for `export::Progress`, `ffpipe`, `bg`, `blitz`, `transcribe`,
  and verbatim copies of `analysis::onsets` / `bpm` / `scene_cuts`. Their tests ran against **real ffmpeg 6.1**:
  **12 passed, 1 ignored** (`describes_a_shot_with_smolvlm2` needs the add-on). That includes
  `measures_a_generated_clip_with_known_cuts`, the full measure pipeline on a generated clip.
- An ad-hoc clip (red→blue 0.8 s xfade, a cut to moving testsrc2, a cut to a zooming Mandelbrot, 13.2 s) measured exactly:
  a dissolve at 2.60 s lasting 0.80 s, cuts at 5.20 and 9.20 s, and no false cuts in the moving sections.
- `vlm.json`: the llama.cpp runtime zip exists, and its size matches. Licences: MIT runtime, Apache-2.0 models.

## Not verified (coordinator, on Windows)

- `cargo test` for the rest: `model::context` tests, `styles_ctl` tests (`context_round_trips…`, `slash_style_applies_and_undoes`,
  `styles_tools_list_get_and_learn_status`), `styles_ui` idle-repaint tests, the new assistant_ctl test, the Project Context typing / undo-burst behaviour, and the registry
  parity tests (`every_edit_op_has_a_tool` etc.).
- The four SmolVLM2 GGUF URLs, sizes and sha256s: huggingface.co is blocked by this container's proxy. Run
  `styles.install_describer` once, then `cargo test describes_a_shot_with_smolvlm2 -- --ignored`.
- Live UI: the Project Context pane (typing gives one undo step per burst, the digest folds), the Styles tab rows, the
  Learn window end to end on a real video, `/style` completion, and Window ▸ Project Context. No screenshots were taken.
- Size gate (`scripts/size.ps1 -Note styles-context`): not run. The branch adds about 4k lines of Rust plus six
  embedded style files and `vlm.json`, so expect it over +64 KB, which needs a `size:` line.
- ffmpeg on Windows may be a different version from the 6.1 used here. The scene-score thresholds were measured on 6.1.

## Risks / follow-ups

- The describer has no Uninstall: `vlm::uninstall` was deleted before the pause because it was unused. If the other
  add-ons get a Settings ▸ Add-ons row with Remove, add this one there.
- `confirm_cuts` thresholds (0.12 candidates, 0.04 step, 4x motion) were tuned on synthetic clips. Real footage with
  flashes (strobes, muzzle flashes) could add a weak cut. The 4x motion rule should reject most of these. Worth checking
  on a music video.
