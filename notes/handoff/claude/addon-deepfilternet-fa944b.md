# Handoff: addon-deepfilter (`claude/addon-deepfilternet-fa944b`)

Cloud session, 2026-11-20. The scope from `plans/cloud-handoff.md` is done. The branch has `origin/develop`
merged in (clean merge) and compiles for Windows. It needs a Windows test run and a live check.

## What's in it
- **`src/engine/deepfilter.rs`**: the add-on. It installs on demand like `engine::enhance`: the manifest
  `library/bin/deepfilter.json` points at the upstream `deep-filter-0.5.6-x86_64-pc-windows-msvc.exe` (MIT /
  Apache-2.0, model built in), and the exe lands in `%APPDATA%\SimpleEditor\bin\deepfilter`. The module covers
  the size and SHA-256 check, a `--version` check, uninstall, the `Engine` choice (DeepFilterNet, or ffmpeg
  `afftdn` as the fallback), strength mapped to `-a` (attenuation limit) or `afftdn nr`, a range-only clean
  into `<cache>\denoise` with a JSON sidecar and a stable FNV cache key, and `remux` for the Denoise bake.
  Child processes drop to below-normal priority while playback runs.
- **`src/ui/denoise_ui.rs`**: the Clean Audio row in Inspector ▸ Audio (Clean, Strength %, A / B), the inline
  "Install speech denoise" button, and the pure A / B and re-point edits.
- **`src/ui/app/denoise_ctl.rs`**: one job per source file, shown in the Jobs pane (with cancel). A FRAME_HOOK
  imports the cleaned WAV into the "Cleaned audio" bin and re-points the clips as one labelled undo step.
  It also holds the MCP tools `audio.clean` (engine, strength, ab), `denoise.status` and `denoise.install`
  (uninstall).
- `export.denoise` takes `engine` and `strength` (`BakeKind::Denoise(Option<f64>)`). `audio.repair` with
  preset `clean` goes through Clean Audio, with `engine` and `strength`.
- Settings ▸ General ▸ Speech denoise: Install, or "installed (0.5.6)" plus Remove.
- CHANGELOG (Unreleased), `docs/customizing.md`, `library/README.md` and `website/docs/audio.mdx`.

## This session's fixes on top of the WIP
- `deepfilter::available()` keeps its answer for 5 s, and install or uninstall resets it. The Inspector and
  Settings call it every frame, and the stat ran on the UI thread (see the page-switch freeze notes).
- Speed-curve clips: their source window was measured against an infinite file, so a Clean never re-pointed
  them. They're now measured to the asset's real end, with a test (`a_speed_curve_clip_is_cleaned_...`).
- `set_low` is `cfg(windows)`, with a no-op elsewhere. `install_state` is kept for the installer branch and
  carries `allow(dead_code)`.
- House rules: the Settings tooltip had a broken string continuation that put 18 spaces into the text. The
  "optional, about 27 MB" aside moved into the tooltip, and the install error now sits in a tooltip. The
  toasts that said "see Jobs" now have a **Jobs** button (`Action::ToggleJobs`).

## Verified here (Linux container)
- `cargo check --target x86_64-pc-windows-gnu --all-targets`: no errors, and no warnings in the denoise files.
- The manifest matches the real upstream exe: I downloaded it, size 26,912,256 and the same SHA-256.
- I ran the real CLI using the Linux build of DeepFilterNet 0.5.6. The version string is `deep_filter 0.5.6`
  (what `check_runs` looks for), and `-D`, `-a` and `-o` are the right flags. The output keeps the input
  file's name.
- The app crate can't build on Linux: winit doesn't support this platform, and Wine in this container crashes
  even on a hello-world exe. So I compiled the **real `deepfilter.rs`** into a scratch harness, with shims for
  `blitz`, `export::Progress`, `ffpipe`, `library` and `settings`. Under that harness:
  - All of `deepfilter.rs`'s unit tests pass except `the_cache_key_changes_...`. That one fails only because
    `\` isn't a path separator on Linux; it should pass on Windows.
  - The ignored `real_deepfilter_run_lowers_the_noise_floor_and_keeps_the_length` passes: the noise floor goes
    from -23.8 dB to -inf, the length is exact, and it takes 1.9 s for 3 s of audio.
  - The ffmpeg fallback test passes.
  - `remux` (the Denoise bake) keeps the h264 picture and the duration, and swaps in the cleaned audio.
  - `install()` from a folder library copies, verifies, checks `--version`, swaps in and reports version
    0.5.6. A wrong checksum is refused and leaves nothing behind. Uninstall works.

## Not verified (needs Windows)
- The full `cargo test`, including `denoise_ctl` / `denoise_ui` / `tools_export` tests and
  `denoise_tools_are_registered_once_with_the_right_kinds`, plus the tool-registry and palette tests.
- The UI. Nothing was rendered: take a screenshot of Inspector ▸ Audio with an audio clip selected (the Clean
  Audio row and the install button) and of Settings ▸ General.
- A live Clean: the MF probe of the cleaned WAV, the Jobs pane rows and cancel, A / B while playing, the one
  undo step, and the priority switch while playing.
- The real download on Windows. `Source::Url` reads `bin/deepfilter.json` from the library on **`main`**, so
  Install fails until that manifest is on `main` (same rule as the enhancer, see `library/README.md`). To test
  before that, use a folder library or set `SE_DEEPFILTER=<path to deep-filter.exe>`.
- `cargo test real_deepfilter -- --ignored --nocapture` with `SE_DEEPFILTER` set.

## Risks
- Re-pointing a speed-curve clip shifts `src_in` by the Clean's start. That's right if the curve's keys are
  clip-local (the model comment says keyframe times are clip-local), but check one by ear.
- `audio.repair` is a `Mutate` tool. With preset `clean` plus `ab`, which isn't documented, `clean_tool` pushes
  its own undo and `commit_tool_edit` may push a second one. Plain `clean` returns a Job, so it's unaffected.
- Size: no exe growth worth noting. The model stays outside the exe (27 MB download).
