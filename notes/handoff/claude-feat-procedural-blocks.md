# claude/feat-procedural-blocks — handoff

Branch from `origin/develop`. Procedural motion blocks: Shake / Camera Shake, Wobble, Wiggle, Pulse, Drift hold
parameters, not keys, and fade in and out from the clip's placed value.

## What's done

- **Model** (`src/model/procedural.rs`, new): `ProcKind`, `ProcCtl` (strength, speed, fade_in, fade_out, decay, seed,
  axes, cover, pid), `ProcLayer` (one block's motion on one property), `envelope` (smoothstep in/out, exactly 0 at
  and outside both ends), deterministic `noise1` (splitmix64 hash + quintic value noise, no allocation).
  - `Animated.procs: Vec<ProcLayer>` (serde default, skipped when empty). `Animated::at` = `keyed_at` + layers.
    `keyed_at` is the old `at`; key edits (`set_at`, `toggle_key`, `clear_keys`) and block origins (`KeyBlock::write`,
    `capture_block`) build on it so a shake never gets baked into a key by accident. `is_default` now also needs
    `procs` empty (otherwise a shaken-but-unkeyed property was skipped on save). `shift` moves layers with keys.
  - `KeyBlock.proc: Option<ProcKind>` marks a preset; `BlockCtl.proc: Option<ProcCtl>` holds the controls;
    `BlockCtl.unbaked` remembers a converted block for Make Procedural.
  - `Clip::write_proc / clear_proc / sync_procs / next_pid / proc_offset`. Layers are tied to their block by `pid`.
  - Ops that understand them: place (`place_ctl`), `set_block_ctl`, `retime_key_block` (calls `sync_procs`; speed is
    kept, so stretching = longer), `remove_key_block`, template stretch (`sync_procs`). Loop / ease / split / link /
    flow refuse or skip procedural blocks (they already loop; fades are their ease). `block_keys` returns nothing for
    them, `touch_blocks` / `loop_keys` / `in_block` / hand runs ignore them, so keys under a shake stay "hand keys".
  - Builtins: the keyed "Shake", "Pulse", "Wiggle", "Drift" and the simulated "Camera Shake" / "Wobble" presets are
    replaced by `proc_blocks()`. `Gen::Shake` / `Gen::Wobble` and `simulate` stay so old applied blocks (their
    `ctl.src` is stored in the project) load, play and re-simulate unchanged; `sim_blocks()` no longer lists them.
- **Convert to Keyframes / Make Procedural** (`src/model/ops/procedural.rs`, new): `bake_key_block` samples on the
  project's frame grid plus both ends exactly, cuts the curve at the ends (`cut_at`) so nothing outside changes,
  removes the layers, and makes the block a plain key block (`abs_block`). `unbake_key_block` only while the keys are
  exactly what the bake wrote (`can_unbake`), restoring the keys it sampled over (and the re-eased key before it).
- **UI** (`src/ui/animator.rs`): `proc_wave` draws the live waveform in the block bar (envelope visible as tapering
  ends); Curves draws the keyed curve solid and the procedural result as a fainter ghost (read-only: no key dots);
  `proc_controls` in the Inspector (Strength, Speed, Duration, Fade In, Fade Out, Decay, Seed + Randomise, Moves
  X/Y/Rotation/Scale, Cover Edges for shakes, Convert to Keyframes); a converted block shows Make Procedural.
  Right-click: Convert to Keyframes / Make Procedural; Ease / Loop / Split hidden on procedural blocks. `kind_of`:
  Wobble reads Physics, Camera Shake reads Camera. Gallery cards picture them through `BlockCtl::sampled`.
- **MCP**: `keyblocks.apply` `params` {strength, speed, fade_in, fade_out, decay, seed, cover, axes}; returns the
  placed `procedural` blocks' indices. `keyblocks.set` takes `params` on a procedural block. `keyblocks.list` shows
  `procedural` (catalogue and applied) and `can_make_procedural`. New `keyblocks.bake`, `keyblocks.make_procedural`
  (OP_TOOLS rows + OP_FILES entry added).
- CHANGELOG under Unreleased.

## Verified (here, Linux container)

- `cargo check --target x86_64-pc-windows-gnu --all-targets`: clean.
- The pure module's 5 tests pass in a scratch crate that `#[path]`-includes the real `src/model/procedural.rs` with
  small stubs (Wine can't run the windows-gnu test exe, winit doesn't build on Linux): deterministic per seed, a
  different seed differs, offset exactly 0 at both ends for every kind / axis / fade setting and no step anywhere
  (20k-sample continuity), fades never overlap, stretching keeps the frequency (same samples mid-block, zero
  crossings per second = speed), noise in range and smooth.

## Not verified (needs the Windows run)

- Every other new test compiles but has not run: `model::ops::procedural::tests` (no keys written, bake = sampling at
  frame times with first/last = base, Make Procedural only while unedited, move/stretch/remove carry layers, controls
  rewrite layers keeping the pid, save/load round trip, old baked Camera Shake project loads and plays unchanged,
  export placement = preview sample at two sizes and exactly placed at the ends).
- Updated tests: `looping_a_block_grows_it…` and `a_loop_repeats_the_cycle…` use a local keyed Pulse;
  the retime test uses an old simulated Camera Shake; `the_lanes_open_on_the_blocks…` uses Pan Swing for `wiggles`.
- Nothing seen by eye: the bar waveform, the ghost curve, the Inspector layout, drag/double-click from the Blocks tab.
  Please screenshot the Animator with a Camera Shake on a clip (Blocks and Curves views, Inspector open).
- Playback smoothness / perf gate not run. Cost per sample is tiny (only properties with layers pay anything).

## Risks / follow-ups

- `Clip::split` / trims shift layers with the keys (`Animated::shift`) but, as before this branch, don't shift
  `clip.blocks`; a procedural block on the right half of a split keeps its layer in step with its keys, while the bar
  may sit at the old time (pre-existing for key blocks too). `sync_procs` after a block op realigns the layer to the bar.
- Saving a procedural block as your own preset ("Save as preset") keeps its kind and length, not its slider values.
- XML / Lottie exports read keys only, so procedural motion is not in those exports (Convert to Keyframes first).
- A Viewer drag during a shake writes `value = at(t) + delta` through `set_at`; it builds on the keyed value for the
  delta, but the dragged-to position includes the shake offset at that instant.
