# Hand-off: claude/feat-visualizer-helper (Visualizer Helper add-on)

Branch from `origin/develop` with `origin/docs/visualizer-helper` merged in. It turns the hardstyle visualizer
hand build (`plans/visualizer-helper.md`) into a library plugin, **Visualizer Helper**, with no Python.

## What's done

- **The plugin**, `library/plugins/visualizer-helper/`: `plugin.json`, the panel
  `panels/visualizer-helper.luau` (five steps in order, one button and a short status each, plain labels with
  `ui.tip` hover text, no badges), the library effects `effects/olympus-storm.glsl` and `effects/lyric-chrome.glsl`,
  and `luts/golden-hour.cube`. It needs no permissions.
- **Ships with the library**: the Plugins pane's new **From the Library** menu (`plugins::LIBRARY`,
  `plugins::stage_library`, `plugins_ctl::start_library_install`). From a library folder it stages the folder; from
  the online library it fetches `plugins/<id>/plugin.json`, then every file it lists (`library::fetch_fresh`), and
  shows the usual install confirm.
- **Native side, `src/engine/vizhelp/`** (the Python ported to Rust):
  - `analyze.rs`: hits (log-flux onset envelope, tempo by autocorrelation, Ellis DP, off-beat stabs, rolls, 85 ms
    merge, downbeat phase, kick gate, medium hits back after the first drop, calm outro ×0.35), drops and the calm
    part (bass + drums stems), screeches, the synth envelope, the vocal envelope (talk vs sing), vibrato, syllable
    onsets, voiced stretches and the first sung syllable.
  - `lyrics.rs`: lyric parsing (`[Section]` and `[Drop]` headers), syllable counting, word timing (transcript
    anchors when the song clip has one, else syllables spread over the singing), then a monotonic DP that puts
    words on vocal onsets. Also line spans (0.35 s lead-in, the last word held, never overlapping), gold and giant
    lines, and fill keys from real glyph edges.
  - `keys.rs`: the template's hand keys (kclock, kpow, sclock, vox, vib, kick, screech, fx, beam, lyric), knobs,
    palettes and templates. It is a pure function.
  - `shader.rs`: the `const` lines of the Olympus Storm effect (drop times, colours, spines, layer toggles) and the
    invert-mask surgery from `hsv_shaders.py`.
  - `loopbg.rs`: the two ffmpeg passes (minterpolate + ping-pong).
  - `dsp.rs`: Butterworth band-pass (filtfilt), percentile, median, moving average, find_peaks, RDP.
- **Tools** (`src/ui/app/vizhelp_ctl.rs`), all `Read` or `Mutate`, so a plugin may call them:
  - `audio.analyze_song` (`start` / `status` / `stems`): start separates stems with the Stem Separation add-on when
    it is installed and analyses in a background job. The result lands on `Project::viz` with drop / calm /
    first-vocal markers as one "Analyse Song" undo step. The job also warms `visualizer::analysis`, so
    Visualizer Templates never decodes on the UI thread. (`audio.analyze` already existed, so this name.)
  - `visualizer.template`, `lyrics.karaoke` and `visualizer.drop_fx`: plain edits, one undo step per panel click.
  - `media.loop` (`start` / `status`): runs ffmpeg in a background job. On landing it imports the loop and replaces
    the clip, tiled under the analysed song, as one "Background Loop" undo step.
- **Plugin API**: `ui.tip(text)` (`Node::Tip`), and `TextRasterizer::char_edges`. Both are documented in
  `src/scripting_api.rs` (the `ui.tip` row, plus a "Tools for plugins" table `PLUGIN_TOOLS` with a test that each
  is a real tool), with `docs/scripting-api.md` regenerated from it. Also updated: `docs/plugins.md`,
  `docs/customizing.md` (5 tool rows), `website/docs/plugins.mdx`, `CHANGELOG.md`, `agents.md` and `notes.md`.
- **The `tool()` global**: `feat-script-globals` is already in develop (f99dcf7), so the plugin uses `tool()`.
  `editor.tool` stays as the hidden alias.

## The "what went wrong" lessons, kept as defaults

| Lesson | Default |
|---|---|
| Signals came from the mix | Analysis uses the stems when the add-on is installed. It says when it fell back to the mix. |
| Ball popped on hats and claps | Kick gate on (18 dB, the panel's Kick gate). |
| Intense part looked dead | Medium hits come back after the first drop. |
| Electricity too early | Everything flashy is gated by `fx`. The outro calms down. |
| Frame drops | Ray march bounded by a sphere, 48 steps, spike search skipped when no spike is out, bullets drawn flat. Layers turned off by the template skip their cost (`SHOW_BALL` 0 skips the march). |
| Restyle wiped the hand keys | The same style only swaps the source. Keys are set by param name, never by index. |
| Text stretched | Font size × the shader margin, clip scale ≤ 1, and no vertical animation. |
| Invert layer misaligned | The shard layer is a clone of the visualizer clip, so it has the same shape size. |
| Duplicating the visualizer duplicated the song | The layer is a fresh unlinked clip. Nothing is duplicated. |
| Overlapping lines | Line end = min(clip end, next start − lead-in). |

## Verified here (Linux cloud container)

- `cargo check --target x86_64-pc-windows-gnu --all-targets`: clean, with no warnings from the new code.
- **Engine tests**: the app's test binary can't run in this container, so `src/engine/vizhelp/*` and
  `src/model/vizhelp.rs` were run in a scratch crate that `#[path]`-includes the real files. All 18 pass, including:
  - `finds_the_kicks_and_the_vocal_onset_of_a_synthetic_song`: a 150 BPM kick plus a sung voice from 6.0 s. It finds
    the BPM within 2, ≥ 90 % of the kicks within 30 ms, the vocal onset within 60 ms and ≥ 16 of 20 syllables.
  - `lyrics_words_land_on_the_vocal_onsets` (uneven onsets, no transcript) and
    `a_stray_onset_is_skipped_and_a_transcript_anchors_words`.
  - `template_keys_are_deterministic`, the shader const and invert-mask tests, the dsp tests and the loop arguments.
- **The panel Luau** ran in real Luau (mlua 0.10.5, a scratch harness with a fake `ui` / `tool`). Every button calls
  its tool, statuses update, and with no song it shows "Put a song on the timeline first".
- **GLSL**: the shipped effects, a shatter-specialised Olympus Storm, the generated Rift Invert mask and a Chrome
  Waveform + Neon variant all compile with `glslangValidator`, assembled like `shaders::user_shader` (prelude, param
  defines, `u_wave`, MAIN). The real GL driver was not used.

## Not verified (please check on Windows)

1. `cargo test` in full. New tests that only run there:
   - `panel::tests::the_visualizer_helper_panel_draws_five_steps_and_calls_its_tools`: every argument the panel
     sends must be one the tool declares.
   - `a_tip_explains_the_widget_before_it`
   - `plugins::tests::the_library_plugins_validate_and_stage_from_a_library_folder`
   - `vizhelp_ctl::tests::*`
   - `scripting_api::tests::plugin_tools_are_real_tools` and `api_docs_are_current` (the docs were regenerated with
     the same `markdown()`).
   - `mcp::tools` duplicate-name test (the reason for `audio.analyze_song`).
2. **Live run, the order to try** (Settings ▸ Library folder pointed at this checkout's `library/` installs from
   disk):
   - Plugins pane ▸ **From the Library ▸ Visualizer Helper** ▸ Install. Then Window ▸ Plugins ▸ Visualizer Helper.
   - Song on the timeline: **Analyse Song**.
     - With Stem Separation installed, it runs Demucs first.
     - Watch Jobs. When it finishes: a toast, markers at the drops, and one undo step.
     - On the original song, expect about 151 BPM, about 483 hits, and drops near 26.4 s and 52.1–52.7 s (the
       Python's validation numbers).
   - **Apply Template**: it makes a visualizer linked to the song.
     - Check playback at the second drop with `perf.bench`: 0 dropped is the bar.
     - Expect one compile stall (~1.5 s) at the click, never during playback.
   - Paste lyrics, **Make Lyrics**: check the fill hits word edges (`char_edges` vs the rasterizer's margins), the
     slide / yaw / smear, gold after the first drop, the giant line, and that text is sharp.
   - **Add Drop FX**: a flash plus a one-beat invert at each drop, and the shard layer on the top "Drop FX" track
     lining up with the ball over the lyrics.
   - Select a GIF, **Make Loop**: a 60 fps loop tiled under the song. Then Drop FX again with Grade on: black until
     the first drop, then gold.
3. **The online library**: `stage_library` from `Settings.library_url` needs `library/plugins/visualizer-helper/` on
   the published library (the pushed `main`). Until then, From the Library only works with a library folder.

## Risks and notes

- The lyric `y` (330 px at 1080p, +down) and x keys (1500 / −450 px at 1920) assume the hand build's coordinate
  convention. Check that the lines sit above the waveform.
- Words with no transcript are placed by syllable count. Melisma, ad-libs and long instrumental gaps can shift words
  by a syllable. A song transcript (Transcribe on the song clip) anchors them, as the hand build's whisper pass did.
- Status in the panel refreshes when a job lands (it pushes an undo step). A failed job only shows on the next
  interaction (its toast says it at once).
- Step 1 has a second button, **Get Stem Separation**, only while the add-on is missing. The house rule says a
  reference to another place is a button.
- The shard layer is a visualizer (it declares `level`) with no link, so the timeline-mix bake may re-key its
  level / band params from the mix. The values are the same song, so this is harmless.
- Size: no new dependencies. The Rust adds roughly 60–90 KB of release exe (not measured, since the size gate runs
  only at release). The shaders and LUT live in the library, not the exe.
- No PR opened (cloud-handoff rules: the coordinator merges into develop).
