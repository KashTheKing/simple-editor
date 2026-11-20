# Notes — Simple Editor

Freeform scratchpad: decisions, gotchas, things worth remembering that don't belong in
[agents.md](agents.md) (workflow) or [goals.md](goals.md) (priorities/roadmap). Append via
`/se-notes`, by editing this file directly, or by telling an agent to jot something down.
Newest at the top. No required format — a bullet or a short paragraph is fine.

---

- **bg-removal (2026-11-19): background removal add-on, and its matte cache format.** `engine::bgremove` +
  `tools/se-matte` (ONNX Runtime 1.22 DirectML build via `ort =2.0.0-rc.10` `load-dynamic`; MODNet = Fast,
  BiRefNet lite = Best; RobustVideoMatting is GPL-3, not used). **Cache format (for unifying with SAM 2 later):**
  `<cache>\mattes\<16 hex>-<fast|best>\` per (source file, quality); hash = FNV-1a 64 of the lower-cased
  backslash path, then the file's size and mtime (u64 LE each). `matte.json` = `{"format":1,"source","quality",
  "fps","width","height"}`; frames are 8-bit greyscale PNGs named `<n>.png`, `n = floor(src_t·fps + 0.001)` at
  the source's fps (a still is `0.png`), white = keep. Written as `<n>.png.tmp` then renamed. A missing file =
  not computed = the clip draws unkeyed. Stored at long side 512 (Fast) / 1024 (Best), never above the source.
  **Playback**: `layer_for` reads the PNG (`matte_frame`, Edge applied there: erode + remap to shrink, two box
  blurs to feather) into `LayerSet::mattes` (counted in the read-ahead's bytes); `gpu::draw_clip` multiplies the
  layer's alpha with the existing `K_MATTE` pass BEFORE the effect chain (so Chroma Key / masks still apply after),
  Show Matte draws the matte instead; `compose.rs` does the same on the CPU (`apply_cpu`). `covers` is false for a
  keyed clip. New frames reach a paused preview through `Player::mattes_landed` (`Cmd::Mattes`, evicts
  `spans_using_source`); while playing nothing is evicted (no re-decode under the playhead). **Job**: only the
  frames the clip shows + 0.5 s handles, missing runs only; ffmpeg (`fps=,scale=` to the matte size, rgb24) piped
  straight into the helper's stdin, mattes back on stdout, a 3-frame median + small jitter pull (`smooth`), one
  job on the GPU at a time, children below normal while anything plays. **Gotchas**: (1) DirectML runs out of GPU
  memory for BiRefNet at 1024² when other GPU work holds the 8 GB card, and it only shows on the FIRST run, not
  at session creation - the helper warms up on grey and `--ep auto` falls back to the CPU then; (2) MODNet is a
  portrait model: on a plain shape it finds little (see the real-run test's printed IoU) - Best is the one for
  objects; (3) Windows 11 ships DirectML 1.15.x in System32, so the 200 MB DirectML NuGet is not downloaded;
  (4) `mandelbrot` is a bad "busy background" for a salient-object model (it IS the subject) - the test uses
  moving noise.

- **social-guides (2026-11-13): the guides are data, measured on the real pages.** `assets/guides.json`
  (embedded, 20 KB; a copy in `library/guides/`) replaces the hard-coded zones; `ui::social` turns it into
  draw lists (`Op`) that egui (`paint`) and a small bitmap renderer (`raster`, same shapes, text through
  `TextRasterizer`) both replay, so `render.frame` / `guides.preview` match the viewer. Rectangles are dp on a
  reference screen and every coordinate sticks to its nearest screen edge (`Map`), which is what lets one
  layout serve 16:9, 19.5:9 and 20:9 phones. Measuring (Brave over CDP, logged out): mobile web is NOT the
  app - YouTube Shorts mobile web letterboxes the video, shows a 70% comments sheet that covers and dims it and
  never animates; TikTok mobile web shows a 2:3 video and a 300 ms ease sheet that does not move the video. The
  native apps' comment behaviour (the video shrinking above the sheet) and TikTok / Reels app layouts are
  estimates (`src: "est"`), TikTok publishes no pixel safe zone, Meta's 14 / 35 / 6 % is from its ads guide
  (Instagram; Facebook's own page 404ed). Instagram, X and Facebook hide almost everything without a login.
  Profiles live in settings.json with the avatar as a base64 96x96 PNG; the only network code is
  `profile::fetch` (curl.exe, one public page + its og:image, behind a click or the assistant guard). Gotchas:
  YouTube Shorts: the real Shorts app shows a thumbs-down next to the like; the logged-out mobile web player we
  measured has none, and it was removed from the guide at Kash's request (no toggle kept: it would cost size for
  a button nobody wants). The like is a thumbs-up on YouTube and a heart elsewhere. Device emulator: the screen
  corners are clipped by a mask ring (backdrop-coloured ring from the rounded edge outwards, then the bezel ring)
  plus an op-level cull of anything outside the screen; a test checks every device x platform x viewer shape.
  Desktop long form has default / theatre / fullscreen from measured boxes (`vw`; fullscreen is a screenshot proxy
  because headless Chrome cannot enter it); T / F / Esc and the control-bar buttons drive it. Heredocs with
  apostrophes break the agent shell (write scripts to files); a 'shrink' sheet must not
  change the render size (`Scene.base` is the unshrunk rect) or every slide re-renders the frame.

- **creator-kit build (2026-11-11): how it was run, what the live run found.**
  Order: spike (entry below) -> `library/README.md` (formats) -> wave 0 by the lead (marker sections,
  registered stub modules with the cross-workstream signatures, the installed-library core, shader
  header parsing + wrappers, a real-driver test over `library/**/*.glsl`; `plans/creator-kit/
  engine-contract.md` is the shared brief) -> four engine agents in the SAME worktree on disjoint files
  (html-layer, shader-library, skills, online-library; `cargo check` allowed, others' errors ignored)
  next to twelve content agents writing only under `library/`. What made the content good: every
  author could LOOK at its work with the app's own renderer - a preview tool built from the spike for
  HTML / icons, and the built test binary re-run with `SE_ONLY` / `SE_SHOTS` for shaders (files are read
  at run time, so the binary need not be rebuilt). Merge: 2 test failures, both from the lead's own
  integration edits; then `cargo test` 1417 passed, `--selftest` OK.
  **Live, release-fast, scratch profile, second monitor, over MCP** (all five checks of the plan):
  (1) `html.create` with the library Subscribe button -> over video with clean alpha at its natural
  size; `html.set` label + accent -> re-rendered; the exported frame shows it. (2) "Add a lower third
  with my name": the assistant found `html/lower-third-clean` with `library.list`, installed and
  applied it, set name / role with `html.set`, looked with `html.check`; 24 s. (3) the `/` list
  filters as typed, the Skills tab lists built-ins and "Yours"; `/create-skill …` wrote
  `skills\punch-words.md` in 32 s; `/captions` ran as a skill and stopped to ask before the 142 MB
  speech model download (not installed in the scratch profile, so captioning itself was not run).
  (4) a cloud transition clicked in the Gallery (library read from the local `library/` folder) was
  copied in and applied with a toast; `library.apply` for an effect and a LUT; export matches the
  preview. (5) the saved project on a clean profile with the first profile's library and cache hidden:
  the HTML layer re-rendered from the project, the shader effect and transition drew, the LUT came from
  `kit LUTs\` beside the project (pixel-identical bars).
  **Found and fixed**: the Gallery's tab row scrolled sideways, hiding the library tabs in a narrow
  pane (it wraps now); `project.save` over MCP skipped the LUT copy that File > Save does; the outgoing
  clip of EVERY transition (built-in Push too, so older than the kit) went black after the cut when the
  clip used its file to the last frame - a source time within 0.1 ms of the end came back with no
  frame; `playback::clip_src` now clamps a 60th of a second before the end. The countdown template's
  thumbnail fell on a frame where its digit is mid pop-in (`se:thumb` added).
  **Gotchas**: launching with a FILE ARGUMENT starts as a full-screen player that plays at once - a test
  window then "goes black" over the gaps of the timeline and `layout.list` shows no pane rects: use
  `project.open` after launch, or `ui.action fullscreen`. A test profile that is not reset saves its
  window rect as the OUTER size, so the window grows 16x39 px per relaunch. Python heredocs in the agent
  shell eat backslashes (`\\n` in a patch became a real newline): write patch scripts to a file. Two
  shader authors overwrote each other's helper script in the shared scratch folder: give each agent its
  own folder. Disk D: fell to 4 GB during the build (one debug + one release-fast target).
  **Not done / ceilings**: `.mogrt` import (needs sample files); nothing fetched over the network yet
  (the catalogue URL is `main`, where `library/` is not yet); `html.check` blocks the UI thread while it
  renders (0.1-0.3 s warm, +1.3 s when Edge has to start); one Edge, one render at a time; an export
  started while an HTML layer is still rendering is stopped with a toast; library transitions show a
  plain dissolve card in the Transitions pane (the Gallery has the real thumbnails); `thumbs.rs` still
  lists `lut_dirs` every frame the Looks / Luts tab is open; no hover preview for library items; QSV /
  the Codex backend etc. are as untested as before. **Size** (`scripts/size.ps1`, release): 12,326,400 B before
  the kit (`f44595e`, built from an exported copy) -> 13,067,264 B (`a17154b`): +740,864 B (+0.71 MB),
  above the plan's 0.2-0.4 MB estimate. The row before those two in `size_log.csv` predates the Pro plan.

- **creator-kit spike (2026-11-11): headless Edge can step CSS animations exactly, with alpha. Go.**
  Edge 157, RTX 2060 SUPER, machine in normal use. A std-only throwaway program (TcpStream WebSocket
  client, serde_json) started `msedge.exe --headless --remote-debugging-port=0 --user-data-dir=<temp>
  --host-resolver-rules="MAP * ~NOTFOUND"`, read the port and browser path from
  `<profile>\DevToolsActivePort`, and drove one tab over the browser socket with flat sessions
  (`Target.createTarget` + `Target.attachToTarget{flatten:true}`): `Emulation.setDeviceMetricsOverride`,
  `Emulation.setDefaultBackgroundColorOverride{a:0}`, `Emulation.setScriptExecutionDisabled{true}`,
  `Page.navigate` to a `file:///` URL, `Page.loadEventFired`, then per frame "set every animation's time"
  and `Page.captureScreenshot{format:"png", optimizeForSpeed:true}`. PNG bytes went straight down
  ffmpeg's stdin (`-f image2pipe -framerate F -c:v png -i - -c:v copy out.mov`): PNG-in-MOV, rgba,
  no re-encode.
  **Exactness**: a test page (linear `left`, delayed opacity fade, `steps()` on a `::after`, an
  infinite alternate compositor `transform`, a 50 % white box) rendered 90 frames at 1080p30; a Python
  check of the decoded `.mov` matched the expected position / alpha of every element on all 90 frames,
  background alpha 0, the half-transparent box (255,255,255,128) = straight alpha. A `<script>` in the
  page did not run. Two ways to set the time were tried and gave byte-identical files:
  (a) `Runtime.evaluate("for(const a of document.getAnimations()){a.pause();a.currentTime=T}")` - runs
  from the host although page scripts are disabled; (b) no JavaScript at all: `Animation.enable` +
  `Animation.setPlaybackRate{0}` before the load, ids from `Animation.animationStarted`,
  `Animation.seekAnimations{ids, currentTime:T}`. **Chosen: (a)** - it always sees every animation
  (pseudo-elements, delayed ones) and can also ask for the longest `endTime`, so frames after the last
  animation ends need not be rendered again; (b) depends on having caught every start event. The page
  itself has no JavaScript either way (stripped by the app and disabled in Edge).
  **Speed** (ms per frame, seek + screenshot + base64 + pipe): 640x360 ~40, **1080p 52-61** (screenshot
  is ~48 of it), 4K ~148. Edge start to first frame ~1.3 s cold, so the renderer keeps one Edge alive.
  Tried and dropped: 2/4/6 windows in parallel (frames identical, 45/34/37 ms a frame, but every extra
  window costs its own load: no net gain on a 3 s layer; each needs `newWindow:true`, a background tab
  never answers a screenshot); `--disable-gpu`, `--disable-gpu-vsync --disable-frame-rate-limit`,
  `--run-all-compositor-stages-before-draw`: none faster. `Target.createTarget` refuses `width`/`height`
  without `newWindow`. Decode of the result through ffmpeg: PNG-in-MOV 8 ms a 1080p frame, `qtrle` 10
  (smaller on flat art, 1.8 vs 4.5 MB for the test, but worse on gradients): PNG kept.
  **Decisions for the engine**: (1) a render writes a NEW content-addressed file
  (`cache\html\<hash of html + fields + size + fps + duration>.mov`, or `.png` when the page has no
  animation) and points the asset's `path` at it - Windows cannot rename over a file a decoder has
  open, the clip only knows the asset id so it keeps its effects / keyframes / position, and undo of a
  field change finds the old file still there; (2) alpha sources get the stacked proxy at ANY height
  (the same ffmpeg filter was run on a 640x360 alpha `.mov` at height 360 and gave a correct 640x720
  colour-over-alpha file) - `proxy_eligible` must stop requiring `height > proxy height` for them and
  build at `min(proxy height, source height)`; (3) neither `ffpipe::bake_sequence` nor the bake
  pipeline is used. Spike source: not kept in the repo (the engine's `html` module supersedes it).

- **assistant pickers + history (2026-11-11):** the header has an exact-model menu (ids from
  Anthropic's model table in `assistant_data::CLAUDE_MODELS`, the four family aliases on top, "Other…"
  for anything newer), an effort menu (`--effort low|medium|high|xhigh|max`, Claude Code agent only,
  `Settings.assistant_effort`) and a History tab listing every saved chat of the project. Live-checked:
  a request with `claude-haiku-4-5` + `low` answered. The model list is fixed in the build
  (`ponytail:` note there); update it when models change.

- **assistant redesign (2026-11-11, after the maintainer tried the first pane):** the pane is a tab
  beside the Inspector; Tasks (the agent writes a Luau script per described task into a queue of
  cards: Run / Edit / Save to Scripts, "Handle my comments") and Chat (bubbles, collapsed actions,
  history per project under `%APPDATA%\SimpleEditorssistant`, pinned context chips, image
  attachments served to any agent through `assistant.attachment`). No vendor name in the app's own
  wording. Live-checked over MCP: a described task became a script and ran as one undo step
  ("Assistant task: ..."); a frame attached from the `+` menu was fetched and described by the agent;
  history survived a relaunch; "Add to Assistant" is in the clip menu. Gotchas: (1) a row of chips
  built from `Frame { horizontal { label, x } }` does not wrap inside `horizontal_wrapped` and widens
  the whole pane (Send got clipped) - a chip is one `Button` now; (2) `Response::has_focus()` is
  false while the WINDOW has no OS focus, so Enter-to-send cannot be driven through `ui.input` on an
  `SE_BACKGROUND` window (typing still works: `TextEdit` reads memory focus) - the headless test
  `enter_sends_from_the_prompt` covers it instead; (3) never `taskkill /IM simple-editor.exe` to free
  the exe for a rebuild - it closes the maintainer's own running app; kill the test instance by pid.
  Not live-tested: Codex backend flags, Image file..., "Handle my comments", task Edit / Save to
  Scripts, the marker / ruler / transcript "Add to Assistant" rows.

- **pro first live run (2026-11-11):** release-fast exe driven over MCP with a scratch profile
  (`APPDATA`/`LOCALAPPDATA` pointed at a temp folder, `SE_BACKGROUND=1`), RTX 2060 SUPER, ffmpeg 7.1.
  Test project: 18 s, 1080p60, two H.264 clips (the first 28 Mbit/s), a PNG on V2, two audio tracks.
  Worked: timeline playback 60 fps / 0 dropped with the BGRX upload (colours match the CPU thumbnails);
  Source monitor on the GPU canvas; `render.frame` / `render.contact_sheet` as MCP images; export
  through the GPU window + PBO + NV12 path (1080 frames, bt709/tv tags, pixels within 2/255 of the
  app's own RGB frame); encoder probe (found `h264_nvenc`, `hevc_nvenc`, wrote `encoders.json`) and
  auto picking NVENC; Streams (mix + one stream per track) and Stems (24-bit WAVs in `<name> stems/`);
  EDL / XML export; `editor.context`; `edit.batch` as one undo step labelled "Claude: ...", its
  rollback on a failing call; comments; the Assistant pane spawning `claude -p` (it read the selection
  from the context and called `markers_add`); mic capture -> whisper ("Didn't catch that" in a silent
  room); island transcription of a speech clip with 3 s pauses (every word inside its island).
  Timings, same timeline, one run each, machine in normal use: CPU compositor + libx264 39.0 s,
  GPU path + libx264 12.0 s, GPU path + NVENC 9.6-11.1 s. The encoder is not the limit any more
  (x264 vs NVENC is ~1 s apart); full-size software decode on the export thread probably is.
  Found and fixed: (1) audio stream titles vanish in mp4/mov - ffmpeg only keeps `handler_name` there,
  so both are written now; (2) the Assistant pane's Send row was cut off when chips/quick prompts
  wrapped (fixed reserve) - the input block is a bottom panel now - and agent text is rendered as
  markdown; (3) the HUD sat on the Preview's tool strip - moved to the top right; (4) `ui.screenshot`
  with the window minimised panicked inside egui_glow (0 px wide readback) and took the app down -
  it now returns an error; this one predates the branch; (5) Export Frame / a full-size `render.frame`
  came out soft: one-shot renders reused the preview's decode size (MF only renegotiates after 3
  identical requests, and the paused read-ahead keeps interleaving its own) and its proxy. Same logic
  on main, so also older than the branch. `DecoderPool::set_exact` now makes a one-shot wider than
  the preview canvas read the original at exactly the requested size, past the source cache.
  Gotchas: launching the app from inside a Claude Code session leaks `CLAUDE_CODE_*` /
  `ANTHROPIC_BASE_URL` into the spawned `claude`, which then answers "Credit balance is too low" -
  strip those variables in test launchers; untagged HD files are decoded as BT.709 by MF but as
  BT.601 by ffmpeg, so compare an export against the app's own frame, not against ffmpeg's decode
  of the source. Still not tried: QSV/AMF, the software retry after a failed hardware encoder,
  the custom-CLI and external backends, the API key in Credential Manager, the approval prompt
  (it was switched off for the run), push-to-talk from the keyboard, the benches.

- **pro first compile (2026-11-10, same day, later):** `cargo check --all-targets` found ONE error
  (`slot` scoped inside an `unsafe` block in `GpuRenderer::pbo_read`); after that `cargo test` was
  1350 passed / 2 failed, both test expectations (a BGRX layer read as RGBA in
  `gpu_mode_publishes_layers`; `<transitionitem>` counted without the mirrored audio cross fades),
  then 1352 passed, and `--selftest` is OK including `export/mp4`, `export/wav` (audio over
  `tcp://127.0.0.1`), `lossless`, `xmeml`, `prerender`. NOT exercised by either: the GPU export path
  (request window, PBO readback, NV12) and the BGRX texture upload on a real driver, the
  hardware-encoder probe and retry, Streams/Stems exports through a real ffmpeg, the Assistant's
  `claude` spawn and Credential Manager calls, the cpal microphone stream, island transcription.
  Those need the live app. No perf number has been measured and `scripts/size.ps1` has not run.

- **pro first coding pass (2026-11-10): for whoever compiles `feat/pro` first.** Nothing in it has
  been compiled, run, tested or measured: every Pro workstream (#140-#153) was written by reading only.
  Expect compile errors; the docs (website pages, ARCHITECTURE.md "coded 2026-11-10" blocks,
  customizing.md rows, CHANGELOG "Unreleased") describe what the code is written to do, not what was seen.
  Verify in this order: (1) `cargo test` (then fix, repeat); (2) `--selftest`; (3) a `--screenshot`,
  looked at, with a video clip in the Preview and the Source monitor; (4) the benches from the
  perf-baseline note below, on a quiet machine, before/after against `main`; (5) `scripts/size.ps1`
  (budget: +0.5 MB for the whole plan). Look at these first, they are the riskiest:
  - scoped-thread decode (`DecoderPool::frames_arc`, `media/mod.rs`): MF readers taken out of the pool
    and driven from `std::thread::scope` threads - check COM/MF init on those threads and that a
    decoder used from a new thread every frame is allowed at all;
  - the texture alpha swizzle for BGRX layers (`engine/gpu.rs` `upload`: `GL_BGRA` +
    `TEXTURE_SWIZZLE_A = ONE`, set per upload on a texture that can switch between RGBA and BGRX) -
    wrong colours or a transparent layer would show here;
  - NV12 readback (`NV12_MAIN` / `render_export_*`): row order (GL rows vs. image rows, "no flip"),
    the chroma plane layout, and the colour flags (`-colorspace bt709 -color_range tv`) - compare an
    export against the same frame from the CPU path;
  - audio over `tcp://127.0.0.1:<port>` into ffmpeg (`run_export`): accept/connect ordering, ffmpeg
    waiting on one socket while another's writer blocks, a firewall prompt, cancel/cleanup;
  - the Credential Manager calls (`engine/agent.rs` `key_load` / `key_save` / `key_delete`, the
    `windows` crate features they need in `Cargo.toml`);
  - `claude -p` reading the prompt from stdin, and `claude.cmd` through std's batch-file handling;
    check `--resume` with a stored session and the stream-json parsing against a real run.
  Also: regenerate the docs screenshots (`scripts/docs-shots.ps1`) and the generated shortcuts page
  (`--dump-hotkeys` -> `website/docs/shortcuts.md`; new actions: Performance HUD, Assistant, Export Avid
  EDL…, Add Comment, Push to Talk). No new screenshots were referenced by the new pages.

- **perf-baseline (2026-11-10): how to take the Pro plan's baseline. Numbers: not measured yet.**
  On a quiet machine (nothing else encoding, plugged in), release build:
  `cargo test --release bench_export -- --ignored --nocapture` (10 s 1080p60 two-layer project, CPU
  path, libx264 + the hardware H.264 encoder if ffmpeg lists one: fps each),
  `cargo test --release bench_source_monitor -- --ignored --nocapture` (1080p60 through the Source
  player: frames published vs due, dropped), plus the existing `bench_4k_preview` and
  `bench_multilayer_preview`. Then in the app turn on Playback ▸ Performance HUD (or MCP
  `playback.hud`), play a 1080p60 clip for 10 s in each monitor and read UI avg/p95, GL composite,
  upload, decode avg/worst, cache %, dropped (the same numbers come back from `playback.status`).
  GPU-path export is not benched (needs the UI's GL context) - time it from the app.
  Baseline: not measured yet. Record it here, then confirm the budgets in goals.md.

- **thread-viewers (2026-11-13): chat / forum threads are DATA that generate an HTML layer.** The thread
  (cast, items, look, animation) is JSON inside the page (`<meta name="se:thread">`, `://` escaped so lint
  stays clean), so Save to Library / Gallery / `html.get` carry it with no model change. `engine::thread`
  parses it, edits it and writes the page; `ui::thread_ui` (Inspector > Thread) and `tools_thread` (`thread.*`)
  are two front ends on the same calls. Motion is all CSS timing from the data: an item is `.it` (a grid whose
  `grid-template-rows` goes 0fr -> 1fr, so the newest pushes the rest up with no height estimates) > `.in`
  (clipped; pops / slides / fades); the viewport is bottom-anchored (`justify-content:flex-end`) with a
  `min-height:100%` column, which IS the auto-scroll, and top-anchored when animation is off. Gotchas:
  `infinite` animations make every frame "moving" in `render_to_file`, so the typing dots run a counted
  number of times; per-item times are inline `animation-delay` / `--d`; vote counting is a stepped strip of
  the numbers (`steps(var(--n))`), not `@property`, so it stays plain CSS. New files written with CRLF put
  `\r\n` into the page through `include_str!`: the golden test for the two library templates catches it
  (`SE_UPDATE_THREAD_LIBRARY=1 cargo test library_templates_are` rewrites them).

- **thread-viewers: avatars vanished from the rendered frames (a held transform animation).** In the timeline / exported
  render an avatar (`border-radius` + `overflow:hidden` circle) was not painted once an ANCESTOR had run a `transform`
  animation with fill-mode `both` (the end state is held): the text stayed, the circle and its picture were gone, and a
  whole forum comment vanished the same way. `html.check` showed it at t=3 s and not at t=5 s, and `animation:none` on
  the ancestor cured it. The pop / slide effects now move with `top` / `left` + `opacity` instead of `transform`.
  Do NOT "fix" it with `animation-fill-mode: backwards` instead of `both`: a finished animation without a forwards fill
  drops out of `document.getAnimations()`, so the renderer's `seek` can no longer rewind it and, when Edge is slow (a
  loaded machine), early frames show the END state. The avatar picture is the circle's `background-image` (no `<img>`).
  `ui.input` has a `drag` event now (press, glide, release) for testing drags.

- **alpha-proxy (2026-10-06): a `.mov` with alpha previewed on black because its proxy was yuv420p.**
  Export was always right (it reads the source). Such a source now gets a STACKED proxy, `<hash>-a.mp4`:
  still all-intra H.264 that Media Foundation reads, twice as tall, premultiplied colour on top and the
  alpha plane as grey below; `proxy::StackedAlpha` folds it back to straight alpha. Measured on a
  1080p60 PNG `.mov` with a 720p proxy (release-fast, median): plain proxy 2.5 ms a frame, stacked 7.5 ms
  (4.6 ms to decode 1280x1440, ~3 ms to fold); decoding the source itself through the ffmpeg pipe is
  36 ms. `cargo test bench_stacked_alpha_proxy -- --ignored --nocapture`.
  Gotchas: (1) scale in PREMULTIPLIED space (`premultiply` before `scale`) - scaling straight
  alpha pulls the black under transparent pixels into every edge; (2) never let the decoder scale a
  stacked frame: MF's scaler blends the rows either side of the seam, and the top row of the picture
  turns partly opaque wherever the bottom row is bright (a line across the preview), so `StackedAlpha`
  always decodes full size and samples it down itself; (3) the alpha half comes back from limited-range
  H.264 a few levels off 0/255 - snap the ends or opaque areas let the layer below through; (4)
  `premultiply=inplace=1` needs a planar format (`gbrap`), and `alphaextract` fails if the format
  negotiated after `scale` lost the alpha - pin `format=gbrap` on both sides; (5) the iterator
  version of the fold loop cost 4.5 ms in a dev build, the indexed one 1-2. Not done: VP9/HEVC alpha
  (ffmpeg's native decoders drop it, so export loses it too), and an alpha source at or below proxy
  height still plays straight from the ffmpeg pipe.

- **preview-perf2 (2026-10-07): what #135 left open.** Same trailer. (1) The 16-decoder LRU was the
  big one: the timeline has 23 stills and 10 videos, so stills were evicted and re-opened (ffprobe
  340 ms + ffmpeg 150-550 ms each) on every loop - 50-120 dropped frames a pass, hundreds once the
  cache was full, 9 frames in a 144-seek scrub. Stills are now decoded at project open in the
  background, kept as a chain of halvings capped at the size they are shown at, and budgeted by
  bytes. (2) A source that is not open in time is left out of the frame and the frame redone when it
  arrives (`DecoderPool::set_deadline` / `collect`). Gotcha: `video` can take a finished open in
  itself, so `collect` must still report it (`arrived`) or the partial frames are never redone - a
  flaky test found that. (3) Cached frames due during a read-ahead decode are handed to the UI with
  their times (`Shared::ahead`). (4) Memory: RSS climbed 5.7 -> 6.6 GB over ten loops because cache
  entries were costed by `len`, and a recycled buffer keeps the capacity of the biggest frame it
  ever held; costed by capacity it sits flat at 5.8 GB (4 GB cache + 1 GB source cache by design on
  a 64 GB machine). Windows logs `RADAR_PRE_LEAK_64` for that growth rate; it is a notice, not a
  kill. (5) Hidden layers: on this project only 0.4 % of decoded pixels were invisible (208
  opacity-0 layer-frames, 8 off-canvas, nothing under a cover), so culling is correctness-tested
  but not a speed-up here. (6) A paused playhead now reads ahead once it has rested 60 ms.
  Measuring gotchas: `Player::seek` moves the clock, so a seek FORWARD ends the pacing window by
  itself - cause 5 of #135 only reproduces with a seek back; another agent's instance on the same
  MCP port makes the app save `mcp_enabled: false`; `rustfmt src/ui/app/mod.rs` reformats every
  child module; and none of the frame counts mean anything while something else has the CPU at
  100 % (they were taken with a game running - redo them on a quiet machine).

- **preview-perf (2026-10-06): "dropping hundreds of frames, ridiculously slow on stacked layers" was
  five things, none of them the compositor.** Measured on a real 15-track 1080p60 trailer (63 clips,
  mostly PNG stills with keyframed pop-ins over an H.264 clip) at 75 % quality: the 30.9 s timeline
  took over 150 s to play and showed 30 % of its frames; after, 31.0 s and 4 dropped. In order of
  cost: (1) `ffpipe::ImageSource` ran ffmpeg.exe per requested SIZE (~120 ms a run) behind an 8-entry
  cache - a keyframed scale is a new size every frame, and ten bubbles of ten sizes thrash the cache
  even when static: 0.5-1.6 s per frame. Now one run at open, smaller sizes shrunk in Rust. (2) DXVA
  was attached from 720p up, which is exactly the proxy height: the GPU->CPU copy back made a 720p
  proxy 13-43 ms a frame against 2.4 ms in software (1080p: 42 vs 7; 4K still wins, 7.0 vs 9.4), so
  a single video layer could not hold 60 fps. Now only above 1080p. (3) the layer decode size
  followed the placement to the pixel: a zooming clip was CPU-resampled every frame (25 ms once the
  zoom outgrew its 720p proxy - placement uses the ASSET's size, the decoder has the PROXY's), an odd
  canvas height cost a 6.5 ms resample for one row, and stills were re-copied and re-uploaded per
  frame. `playback::layer_size` now asks for stepped fractions of the decoder's real size and the GPU
  scales; a cached still is one shared `Arc` (`DecoderPool::frame_arc`), so `gpu::upload` skips it.
  (4) opening a source (ffprobe + ffmpeg, 250 ms per PNG) happened on first use on the render thread;
  `DecoderPool::warm` opens upcoming clips on their own threads. (5) after Pause the render thread
  finished its whole read-ahead before looking at a Seek. Gotchas: the app's main crate is
  `opt-level = "s"`, so per-pixel Rust loops (box filters) are 3-5x slower than you would guess -
  measure before adding one to a per-frame path; MF's DXVA readback waits are timer-tick sized, so a
  bare test process (15.6 ms ticks) shows 31/62 ms where the app shows ~10; and heredocs in the
  agent shell halve backslashes - write helper scripts to a file. Still open: `GpuRenderer` never frees a clip's
  layer texture. (The alpha `.mov` previewing on black is fixed - see the entry above.)

- **size-recovery (2026-09-29): an egui menu cost ~30 KB of exe per call site.** The simplify wave
  grew the release exe 12.44 -> 14.54 MB with ~2k source lines and no new deps. `cargo llvm-lines
  --bin simple-editor` (dev build, a few minutes) showed `egui::containers::popup::Popup::show` as the
  crate's #1 item (243 copies, 5.9 % of all IR); `cargo bloat --release -n 0 --message-format json`
  put `Ui::menu_button` at 76 instances x ~32 KB = 2.49 MB. egui 0.33 boxes the body closure for
  Window / ScrollArea / ComboBox / panels / CollapsingHeader (`show_dyn`), but NOT for `Popup::show`,
  `SubMenu::show`, `MenuButton::ui` or `Response::context_menu`, so each `.context_menu(|ui| …)`,
  `ui.menu_button(…)` and generic `menu::sub` call site compiled its own copy, and opt-level "s" +
  fat LTO can't merge copies that differ only by the inlined body. Fix: `ui::menu::{context, button,
  sub, scroll}` pass the body through one `&mut dyn FnMut` (`erased`) - 14,536,192 -> 10,797,568 B
  (-3.57 MB, 1.6 MB under wave 0), pixels and bench_4k_preview unchanged. Rule: never call
  `.context_menu(` / `.menu_button(` directly (`menus_go_through_the_erased_wrappers` fails if you do);
  before wrapping another egui container that takes `impl FnOnce(&mut Ui)` with no `show_dyn`
  inside, check `cargo llvm-lines` for its copy count. cargo-bloat forces `strip=false` +
  `debug=true` on MSVC, so its first run rebuilds every dependency (~20 min, separate artifacts);
  its exe size matched the real one within 4 KB.

- **docs-site (2026-09-29), screenshot pipeline gotchas (`scripts/docs-shots.ps1`):** a live
  `ui.screenshot` and the `--screenshot` CLI path use the very same eframe readback (glow
  `read_screen_rgba` right after painting, `ViewportCommand::Screenshot`), yet the live one read back
  black / stale frames while the Windows session was locked in wave 1 and the CLI one didn't - so the
  difference is environmental (most likely: a window created while locked renders fine, one that was on
  screen when the lock came doesn't), not a code path to fix; an agent can't lock the session to prove
  it. The pipeline therefore launches the app once per shot with `--screenshot` and the new
  `SE_SCREENSHOT_WHEN=<file>` (shoot once the file exists), drives it over MCP, then touches the file.
  Other traps it handles: the startup toasts ("GPU preview: …", "MCP server at …") live 5 s, so a shot
  waits until 5.5 s after launch; `media.import` replies before the probe lands, and `timeline.add_clip`
  on an unprobed asset makes a zero-length clip (wait for `media.list` durations); a killed run leaves an
  autosave AND a `<project>.sedit.lock` (the next launch would show "Recover unsaved project?" or a
  "may already be open" toast), and the app writes pages/layout back into settings.json, so the profile
  is reset before every launch; the shared target dir's exe may be another worktree's build (the script
  checks it contains `SE_SCREENSHOT_WHEN`; keep a copy of your build and pass `-FromExe <copy>` to skip
  cargo); and something maximised the test window mid-run once (the
  script compares the PPM's aspect with 1600x900 and retries). Coordinates for clicks are panel-relative
  (`layout.list` now reports each pane's `rect`), so a layout change mostly means re-running, not
  re-measuring. Found on the way: a reveal from inside a pane's draw is lost (`self.layout` is a
  placeholder `Layout` during `layout::show`), so a Library click loads the Source monitor but doesn't
  bring its tab forward; `source.open` over MCP does. And never `rustfmt src/main.rs` - it follows the
  `mod` tree and reformats the whole crate, like bare `cargo fmt`. Every launch sets `SE_BACKGROUND=1` and
  writes `"window_rect": [-2520, 112, 1600, 900]` into the scratch settings.json (the script's
  `-WindowRect`), so the windows open on the maintainer's second monitor without taking focus - fifty
  launches in a row used to jump in front of whatever the user was doing.

- **se-fix (2026-09-07), "line tool snaps to the wrong direction" bug report — already fixed, no
  code change:** investigated a report that shape drawing doesn't render during the drag and the
  Line tool snaps to a direction that doesn't match the drag on release. Root cause was real but
  already fixed by `6d5c6e1` ("fix(shapes): a line keeps the point it was started from",
  canvas-handles-monitor/pro-monitor era) — `constrain_drag`/`signed_min` in `ui/preview.rs` keep
  the drag's signed half-extents all the way through `draw_shape_preview` (live rubber band) and
  `drag_stopped` (final `new_shape`), and `engine::shapes::signed_half` renders Line/Arrow between
  the same signed corners, so the preview and the final render always agree. `6d5c6e1` is an
  ancestor of `main`@`00be9b0` (this worktree's base) and is covered by
  `line_tool_keeps_the_press_as_its_origin`, `a_shape_drag_emits_exactly_one_shape`,
  `shift_locks_a_line_to_45_degrees` and 4 more in `ui/preview.rs`'s test module — all pass.
  Cross-checked live against the actually-running `target/release/simple-editor.exe` (built same
  day from the same commit, confirmed via the MCP co-editing tools): `shapes_add` with a signed
  negative height rendered in the correct direction via `frame_export`, and `shapes_add` → `undo`
  cleanly removed the clip with one history entry. If this report recurs, check the reporter's exe
  build date/hash before re-investigating the drag math — it's solid as of `00be9b0`.

- **tab hover cursor (2026-09-07):** egui_tiles' `Behavior::tab_hover_cursor_icon()` defaults to
  `CursorIcon::Grab`, which Windows renders as the 4-arrow move cursor — misleading for a
  click-to-switch tab (dragging still works, it just doesn't need to announce itself with that
  icon). Overridden to `CursorIcon::Default` in `layout.rs`'s `Behaviour` impl. Also: a cursor-only
  change has no pixel diff a `--screenshot` render would show (it doesn't capture the OS cursor),
  so that verification step doesn't apply here — tests + `--selftest` are the only signal.
  Unrelated flakiness hit during verification: `media::thumbs::tests::video_thumbs_colours_aspect_and_cache`
  times out ("thumb within 3 s") intermittently even on `main`/isolated single-threaded runs —
  pre-existing, not caused by any of today's changes.

- **popup positioning (2026-09-07):** egui remembers a `Window`'s last dragged position across
  close/reopen (keyed by its `Id`), so `.default_pos(...)` only ever takes effect the *very first*
  time a stable-Id window (e.g. Settings) is shown — reopening it later just restores wherever it
  was left, not a fresh click/center point. To reposition on every open, force it for one frame only
  via `.current_pos(...)` on the frame `open` transitions false→true (tracked with a `prev_open`
  field), then let normal drag memory take back over. One-shot confirm/discard windows
  (`ui/confirm.rs`) don't have this problem — their `Id` includes the body text/index so they're
  fresh every time, and plain `.default_pos()` is enough. Shared click-or-center logic lives in
  `crate::ui::popup_open_pos`.

- **docs-refresh (2026-09-07), cross-cutting gotchas from the whole overhaul:** Luau's io/os/ffi
  sandbox means `editor.log()` only ever renders as a 5-10s auto-expiring toast
  (`ui/app/palette_ctl.rs`'s `fire_hook`, `app.rs`-descended toast draw) with no copy button —
  script-generated docs must be transcribed via `curl` on `tools/list`, never by reading toast
  output. Alt-render requests must check `self.export.is_some()` before firing (UI thread also
  services export's `GpuFrameRequest`s — the two must not fight over the GL context). Registry
  marker-section hunks never got a real union conflict in practice because `.rs` files were
  deliberately excluded from `.gitattributes merge=union` (only `size_log.csv`/`CHANGELOG.md` are).
  All six planned `-- @on` hook events (`selection_changed`, `import`, `export_done`,
  `project_open`, `project_save`, `marker_added`) do have real `fire_hook` call sites as of
  wave-3-complete — verified by grep against the merged tree, not assumed; see
  ARCHITECTURE.md's "Registries" section and `docs/customizing.md` for the owning file:line of
  each. `Settings.mcp_enabled` defaults to `false` and `PowerShell`'s `Set-Content -Encoding utf8`
  writes a UTF-8 BOM on Windows PowerShell 5.1 — a hand-written `settings.json` with a BOM gets
  silently quarantined to `settings.json.bad` on boot (parse failure), so MCP never starts; use
  `[System.IO.File]::WriteAllText(path, json, [System.Text.UTF8Encoding]::new($false))` instead.

- **UI/UX overhaul plan (2026-09-04):** lives in `plans/ui-overhaul/` — `README.md` is the master
  plan (thesis, decided keymap, frozen modifier table, registry protocol, size plan, waves),
  `issues/<workstream>.md` are issue-ready bodies for `/se-implement`. Rules an implementer must not
  bend: wave 0 is three *serial* PRs (0a pure moves → 0b registries/schema/hooks → 0c size-diet) and
  nothing else starts until 0c merges; a workstream edits only its own files plus its pre-seeded
  `// ---- ws:<name> ----` section in the shared tables; every new `pub fn(&mut self)` on Project
  needs a ToolDef row or `every_edit_op_has_a_tool` fails; plain edge-drag stays a plain trim and
  Delete stays non-ripple (pro behaviour is on modifiers / per-track flags). Verified size facts the
  plan rests on: `luau0-src` forces `opt_level(2)` (Luau's ~2 MB is fixed), eframe `default_fonts`
  embeds 1.41 MB of TTFs, egui_commonmark cost ~2.1 MB when added. Process gotchas: fix/revise
  agents sometimes wrap `name`/`worktree` in literal quotes — normalise before keying on them.

- **Playback-cache work (PRs #12+#14), gotchas worth keeping:** `Cache::avg_entry_bytes` must
  refuse empty/zero-byte caches — an empty timeline's GPU LayerSets insert at 0 bytes and the
  horizon math would divide by zero. `DecoderPool::set_proxies` keys decoders by RESOLVED path, so
  a remap must drop the source, its OLD proxy and its NEW proxy keys. The proxy "building" badge
  is set/cleared entirely inside the build job via a drop guard — setting it after `spawn_job`
  returns would race a fast-failing job and stick forever. The source-frame cache keys by (source
  path, exact f64 µs, w, h): replays hit because callers re-derive bit-identical times from the
  fps grid; animated-scale clips request a new size every frame and never hit (commented ceiling).
  History labels are derived from an entry's NEXT neighbour — any delete must clear the label
  cache (Sonnet review caught this; test pins it).

- **Auto-cut "Beats" section had no detect/commit split:** unlike Silence and Scene cuts (Detect
  populates a preview; separate Split/Mark instead/Apply buttons commit), the old "Detect Beats"
  button both detected onsets AND wrote markers in one click — inconsistent with `audio.beats`'s
  own MCP contract ("with neither flag: pure detection... no mutation"). Split
  `analysis::detect_beat_markers`/`split_beats` into a pure `detect_beats` (returns per-clip onset
  times + BPM, no mutation) plus `beat_markers`/`split_beats` (act on the cached preview). UI now
  has Detect Beats → Add Markers / Split at Beats, matching the other sections' shape.

## Web snapshots, params, fake mouse (2026-11-12)
- Snapshot from an address = a private, network-enabled Edge (`capture_mhtml`) -> `.mhtml` under
  `%APPDATA%\SimpleEditor\web\snapshots` -> `import_with_undo_at`. Signed-in pages cannot be reached natively
  (Edge profile is private by design), so "From my browser" = extension (`tools/browser-extension`,
  `pageCapture.saveAsMHTML`) + a 1 s Downloads watcher (new, stable `.mhtml` newer than window-open).
- Fake mouse = real `Input.dispatchMouseEvent`; transitions stepped by `seek_rel` so they are frame-exact.
  Coordinates are viewport CSS px (top viewport only; a tall page's lower part is not hoverable - use Scroll).
- Decision: Record mode (live cursor -> few keyframes, click-through to next snapshot) is in goals.md, not built.
- Live-tested 2026-11-12 in a scratch instance: real snapshots of wikipedia.org, roblox.com (full page, tall) and
  youtube.com (signed-out feed is empty - that is what "From my browser" is for) render with CSS/images/fonts;
  Downloads watcher imported a dropped .mhtml in 2 s; fake mouse cursor drawn; Mouse section works.
  Found live: Chrome/Edge MHTML links its stylesheets as `cid:css-...@mhtml.blink` (fixed in webcap).
- NOT verified: the extension itself inside Brave (needs Developer mode > Load unpacked by hand), a .mhtml saved by
  Brave's own Ctrl+S, the editor OS window / F11 on a second monitor, params UI with keyframes in the Inspector.
- Size: 13,457,920 B (+390,656 vs a17154b; +1,131,520 vs f44595e).

## Release 1.2.0 (2026-11-12)
- First release of the Pro plan + Creator Kit + web snapshots (feat/pro, 15 commits, squash-merged). Version bumped,
  CHANGELOG `Unreleased` -> `1.2.0`, docs refreshed for agents (agents.md: map, MCP driving rules, triage, releasing;
  llms.txt; README; ARCHITECTURE; customizing.md tool count re-taken from a live `tools/list`: 298).
- New: Help > Report a Bug... (opens the GitHub issue form with the version prefilled), `.github/ISSUE_TEMPLATE/`
  (bug_report.yml, feature_request.yml, config.yml: blank issues off, Discord link), website pages `html-layers`,
  `online-library`, `report-a-bug`.
- Verified: cargo test 1439 passed (one perf test, `headless_1000_assets_stays_fast`, failed once while an npm build
  ran alongside and passed alone: timing flake under load), `--selftest` OK on the release exe, docs site builds,
  live `tools/list` = 298, Help menu shows "Report a Bug...". Release exe 13,458,944 B (+2,119,168 B vs 1.1.0).
- Blitz helper (feat/blitz-helper, 2026-11-13): HTML layers can be drawn by `se-html` (tools/se-html, a separate crate
  and exe, ~11 MB, NOT linked into simple-editor.exe: linking measured +10.2 MB) instead of Edge: ~25 MB RAM vs ~700 MB,
  ms per frame, no start-up. Client `engine/blitz.rs`, protocol `tools/se-html/src/proto.rs` (shared file), lint
  `engine/html_lint.rs`, per-layer `HtmlSource.renderer` (Auto / Blitz / Edge), install on a click (`blitz::install`:
  library `bin/se-html.json` + exe, size and SHA-256 checked), build + manifest `scripts/build-helper.ps1`. Blitz is beta:
  it needs `floats` (without it a plain float hangs layout), beta.4 declares rustc 1.91 and is built with
  `--ignore-rust-version` on the pinned 1.90, colour filters make vello_cpu panic (caught per command -> Edge), and an
  inline svg cannot be animated from the inside. Pitfalls found: Windows names "Arial Black" / "Segoe UI Black" by weight
  so the helper adds @font-face rules for them; transformed siblings paint above plain ones in a browser but not in Blitz
  (give the top copy `position: relative; z-index: 1`); a zero-area `clip-path: inset(0 100% 0 0)` is not clipped.
- NOT verified: clicking Report a Bug opening the browser at the prefilled form (the form field id is `version`;
  GitHub accepts `?template=bug_report.yml&version=...`), the extension inside Brave.


- **Gallery: scroll bug, Effects/Transitions merge, drag onto the timeline (2026-11-12, fix/gallery-scroll).** Root cause
  of "tons of items in one row, can't scroll": `ui::drag_source` draws its body in a `ui.scope`, which takes the rest of
  the row instead of wrapping, so any card drawn *inside* it in a `horizontal_wrapped` grid never wraps (the Animate tab,
  28 blocks; `effects_ui::effect_card` had met it before and allocated first). Every Gallery card now goes through
  `gallery::card`, which `allocate_exact_size`s its slot first and is itself the drag source. A second trap found on the
  way: a widget wider than the pane (the 150 pt search box in a 120 pt pane) widens the scroll area's `max_rect`, and the
  wrapping rows below it with it - keep fixed-width widgets under `available_width()`. Effects / Transitions are Gallery
  tabs (`Pane::Effects`/`Transitions` are gone; their names still deserialise as `Pane::Presets`, `Layout::fold_gallery`
  keeps one tile). `ui.input` gained `down` / `up` so a drag can be driven (move, screenshot, release). Hard-won test
  facts: a widget outside the clip rect is not painted (assert on a scrolled pane or on recorded rects, not painted
  text); `vertical_scroll_offset` is NOT clamped in the frame it is set (pass `content - viewport`); egui drops a dnd
  payload on Esc but a held button keeps `is_being_dragged`, so a drag source must remember it was cancelled
  (`gallery::drag_lifted`). `engine::library::index_tests::library_index_matches_files` fails on a CRLF checkout
  (sizes in index.json are LF sizes) - unrelated, pre-existing.

- **Gallery: one grid, one header, online library as a switch (2026-11-13, fix/gallery-scroll).** The "use online library or
  a folder" screen is gone: `Settings.library_online` (default false) is set by the "Browse online library" banner (or
  `gallery.connect`); `library_ctl::ensure` returns before any thread/request unless `connected(settings)` (a folder counts),
  and then only reads the installed items + the catalogue already saved on disk. Rows of the catalogue that are not
  installed show only when connected (`OnlineState::rows`); a thumbnail is only queued when connected. Every tab draws
  through `gallery_filter::grid` (measures the pane, picks whole columns, publishes the card size in a ctx temp that
  `card` reads) under one header (`gallery::header`). Traps: egui's `ComboBox` is ~100 pt wide whatever `.width()` says and
  widens a narrow pane (and the scroll area under it) - use `menu::button` + `menu::check` rows; `ctx.run` may run the
  closure more than once per frame, so counts accumulate into `count_acc` that `show` resets each pass; Ctrl+F is the
  Find action (polled before the panes draw), so `Action::Find` over `Pane::Presets` calls `gallery::focus_search()` instead.
  The Library (media) pane's reuse list keeps only node graphs now - effects, looks and adjustment layers are Gallery cards.

## Browser layer + fake mouse (2026-11-13, feat/browser-layer, not committed)
- `ClipKind::Browser`: the canvas so far becomes the page inside a drawn window. No shader, no web engine: both
  compositors use what they had (clear, placed bitmap, masked placed bitmap). Bitmaps go through the `LayerSet`
  under `browser::key(clip, slot)`; the window art is cached and only the address / load bar are redrawn.
- Scroll moves the layers BELOW the browser (`scroll_shift`), so a tall screenshot scrolls at its own resolution;
  culling in `decode_layers` is off while a browser clip is on screen.
- Gotcha found live: `gpu::render_mask` sends polygon points absolute but the shader subtracts `m_center`, while
  the CPU's points are relative to the centre. The two only agree around (0, 0) - the browser's page mask is
  rasterised there (`MASK_ORIGIN`). A user's polygon clip mask may be displaced on the GPU for the same reason:
  not checked, not changed.
- Size trap: `serde_json::to_value` / `from_value` on a new struct compiles a second copy of its (de)serialisers
  (104 KB here, cargo-bloat). Go through text (`style_json`) when the type is already in the project format.
- A first import into an empty project sets the project to that file's size (a 1920x4200 screenshot made a
  portrait project): scripts should `project.set` first.
- Verified: 26 new tests; `cargo test`: 6 failed, none of them here, the rest passed (5 `engine::html` tests:
  "Microsoft Edge exited at start"; `library_index_matches_files`: file sizes differ in a CRLF checkout);
  `--selftest` OK on the release exe; live in a scratch release instance over MCP (demo project, frames, contact
  sheet, Inspector, Record-mouse start / stop). Release exe 13,762,048 B (+303,104 B = +296 KB vs 1.2.0).
- NOT verified: Record mouse and Place waypoints with a real hand on the mouse (no agent can move it; the take
  pipeline was fed 476 synthetic 120 Hz samples through `cursor.from_samples`), export of a browser project to a
  file, a browser inside a nested sequence in the app (unit-tested through the CPU compositor only), favicon and
  background pictures on the GPU path beyond one favicon, the docs site build with the new page.

## Browser layer, phase 2: real pages, sharp zoom, click and go (2026-11-13, not committed)
- Zoom moved from "scale the picture of the frame" to the layers below (`engine::browser::LayerView`, with
  scroll): a page with more pixels than the frame is decoded at the size shown. Three places must agree or
  the zoom is soft: the compositors' placement, `layer_for`'s decode size (`zoom_on_screen`) and - the one
  found live - `playback::still_need` (`max_zoom_over`), because a preview pool opens a still no bigger than
  the largest size it is "ever shown at".
- Import (`engine::webpage`): `html::capture_mhtml` -> `webcap::mhtml_to_folder` -> one PNG of the whole page
  at up to 3x (36 MP budget) + a `.json` beside it (`PageMap`: url, title, CSS size, density, elements). Any
  picture with such a `.json` beside it is a page. `Page.captureScreenshot` with `clip.scale` really re-draws
  at that scale (checked at 8x), which is what `browser.sharpen` uses for "Zoom detail" clips.
- html.rs hunks (the HTML agent owns the file): `PageSession` / `with_page` / `CAPTURE_PROFILE` appended at the
  end, and three lines in `Edge::launch_with`. Against `perf/html-load` this is ONE conflict (the
  `match Self::connect(&mut child, ...)` line: keep theirs with `&data`) plus `with_edge(true, |e| ...)` in
  `with_page`. `browser-demo/combo.sh` + `resolve.py` build that combination; the live test ran on it, because
  on this machine main's Edge launch fails ("Microsoft Edge exited at start").
- Fixed on the way: GPU polygon masks were drawn a layer-centre away from where the CPU draws them
  (`gpu::render_mask` sent absolute points to a shader that subtracts the centre). Test:
  `gpu::browser_tests::a_polygon_mask_is_where_the_cpu_puts_it`.
- Live (release exe of the combined tree, scratch profile, port 7492, pid-checked before every call):
  roblox.com/charts and the Forsaken game page imported by address (8 s and 11-27 s), 507 and 41 elements;
  scroll; zoom 3x and 8x sharp after Sharpen zooms; click_go on the real "Forsaken" tile; hover + click on the
  real Play button (an icon-only button: named from its markup); export to mp4; nested sequence; background
  picture. Game thumbnails stay grey placeholders: the site only loads them for a real browser session.
- NOT verified: the sign-in window and an import "as me" (needs the user to sign in; never done by an agent),
  the Import Web Page window and the Inspector's new buttons by hand (driven over MCP only), slide / zoom page
  transitions live (fade was), Record mouse by hand (as before).
- Size: 13,963,264 B = +504,320 B (+492 KB) vs 1.2.0 on this branch alone (phase 1 was +296 KB).

## Browser layer, phase 3: pages are HTML layers, Page only, the Forsaken demo (2026-11-13, not committed)
- An imported page is now an ordinary linked HTML layer clip (`html_ctl::import_file_w`), not a picture: the
  import (`engine::webpage::import`) only captures, unpacks, freezes animations (a `<style>` that stops them:
  with the site's loading shimmers the HTML engine rendered a 6000 px tall FILM, 45-75 s and a decode per zoom
  step) and writes the element map as `index.json` beside `index.html`. The first render says how tall the
  page is, so "fit as page" waits for it (`BrowserCtl::fit_due`).
- Pictures: the capture now waits until images are complete and no `shimmer` / `skeleton` placeholders are
  left (at most 20 s, or 3 s without change) - a small change inside `html::capture_mhtml`'s script. Before
  that roblox.com's game icons were all grey. The map counts what is still missing and the import says so.
- Sharp zoom: the page's own render is at project scale (1 px per CSS px at 1920 wide). Wherever the zoom rests
  above that, `browser_ctl::details_wanted` asks `webpage::render_region` for that part from the HTML (with
  its field values) at exactly that scale - "Zoom detail" clips, made by themselves 0.7 s after an edit (no
  undo step), budget `Settings.page_zoom_megapixels` (16). While a zoom is MOVING the 1x render is enlarged;
  it is sharp once inside the detail. The fake cursor does not grow with the zoom.
- `BrowserStyle.display`: Window (default; old projects) or Page (no window art, no background, the page is
  the frame; the compositors only draw the mouse and overlays).
- "Make editable" (`webpage::make_editable`): leaf headings / button / link / short paragraph texts become
  `se:field`s (plus the page background colour) by rewriting the DOM in Edge and saving its outerHTML.
- html.rs hunks now: the appended `PageSession` / `with_page` / `CAPTURE_PROFILE` block, three lines in
  `Edge::launch_with`, and the wait added to `capture_mhtml`'s script. Against perf/html-load still one
  conflict (the `match Self::connect` line) + `with_edge(true, ...)`.
- Live (combined tree, release, port 7492, pid-checked): the acceptance demo `browser-demo/drive4.py` ->
  `roblox-forsaken-demo.sedit` / `.mp4` (1920x1080, 15.5 s): Page only, real icons, zoom into the Forsaken tile,
  click, fade through to the game page, scroll to "Active 80,243", zoom on it, zoom out, hand to Play, click.
  Also: an effect on the whole page, a masked blur on a region, a heading retyped through its field.
- NOT verified: playback smoothness by eye (frames and the export were checked, not the interactive
  preview at 60 fps), slide / zoom presets of Click and go (the demo scripts its own zoom-through with a
  fade), the Import Web Page window by hand, the sign-in profile, Record mouse by hand.
- Size: 13,988,352 B = +529,408 B (+517 KB) vs 1.2.0.

## Browser layer, phase 4: Window zoom, nothing stretched, the demo again (2026-11-13, not committed)
- Layers under a browser are no longer drawn into a picture of the frame that is then fitted into the window:
  `engine::browser::LayerView` is now the whole transform (scroll, page Zoom, the window's page area, its
  rotation, the Window zoom) and each layer is placed straight where it shows, at that size. `Plan::page` is
  always the whole canvas. Push / wipe transitions under a browser use frame-sized offsets and are a little off
  on the CPU path.
- Window zoom (`BrowserStyle::wzoom` + `wzoom_x/y`): `Plan::zoom_about` scales frame, page area, mask and mouse
  about a frame point; the window art and the cursor sprite are drawn again at that size in steps of sqrt(2)
  (`level`, `raster_scale`, bitmap capped near 4096 px wide), text / shape layers below too (`draw_boost`,
  GPU path only), stills are decoded for the largest zoom they reach.
- Sharp while moving: `zoom_levels` samples the clip at 15 Hz and gives, for every stretch where the page is
  shown bigger than 2^(n-1) and at most 2^n, the part seen; `browser_ctl::details_wanted` has the HTML engine
  draw each at 2^n (through `webpage::render_region` -> `html::with_page`, region + scale: the call a faster
  renderer can serve). So every frame of a move shows a picture at least as sharp as the screen, smaller if
  anything. This is a level cache, not one render per frame; it is the same in preview and export.
- The browser model's JSON is written by hand through `serde_json::Value` (`BrowserStyle::to_json` /
  `set_json`); `browser.set` is `check_fields` + `set_json`. That, and removing the old rests code, paid for
  phase 4's additions: the exe moved from +517 KB to +506 KB, far from the -200 KB asked for.
- Make editable now also makes image fields of the biggest pictures (cap 24 fields).
- Lists in the Inspector (tabs, events) are fixed-column rows with controls on the right and up / down for
  tabs; no drag handle or Alt+Up/Down yet (waiting for the shared `ui::list_rows`).
- Live: `browser-demo/drive4.py` on the combined tree -> `roblox-forsaken-demo.mp4`, read back with ffprobe:
  h264 1920x1080, 30 fps, 465 frames, 15.5 s; frames taken from the mp4 at 2.9 s (mid zoom), 4.3 s, 10 s, 11 s
  and 14.25 s are sharp and the hand grows with the zoom; the Top Songs pictures are real.
- NOT verified: interactive playback by eye; a Window zoom with the chrome ON in the app (unit and GPU tests
  only - the demo is Page only); the Import Web Page window through the UI; slide / zoom presets of Click and
  go; image fields from Make editable live; the new tab / event rows by eye; sign-in; Record mouse by hand.
- Size: 13,977,600 B = +518,656 B (+506 KB) vs 1.2.0.

## Browser layer, phase 5: size, load bar, live checks (2026-11-13, not committed)
- Size, measured: a 1.2.0 tree (`b05471e`) and this tree built with the same flags (release + debuginfo, no strip, so
  `cargo bloat` can read them; that flavour is +4,608 B over the plain release) and the symbols diffed. Official
  numbers are plain `cargo build --release` exes.

  | step | exe B | vs 1.2.0 (13,458,944) |
  |---|---|---|
  | handover (phase 4 row of size_log.csv) | 13,977,600 | +518,656 (+506 KB) |
  | B1: 21 tools -> 7, table-driven JSON (`F` / `fields`) and Inspector rows, tab rows on `ui::list_rows`, `elements` as a loop (bloat flavour) | 13,909,504 | +445,952 vs the 1.2.0 build in the same flavour |
  | B2: + insertion sorts, `Animated::clone` out of line, shorter descriptions, `sharp_state` without `format!` (bloat flavour) | 13,903,360 | +439,808 vs the same |
  | final plain release | **13,905,408** | **+446,464 (+436 KB)** |

  Not met: -200 KB (target +306 KB). What was tried and what it bought: tools 21 -> 7 and shorter descriptions
  (~20 KB), hand-written JSON driven by one field table (model::browser 73 -> 38 KB), Inspector helpers
  (browser_ui 42 -> ~23 KB without the tab rows), insertion sorts and the out-of-line `Animated::clone` (only ~6 KB in all:
  the sort copies and clones were mostly merged by LTO already), `list_rows` adoption cost ~13 KB (the shared helper:
  it is paid once when feat/thread-viewers merges).
- Where the rest is (B1 symbols, KB): Inspector sections + tab rows 51 (13 of it list_rows), import + page placement +
  Make editable 56, MCP tools and helpers 46, model style / cursor / JSON / take 49, model edit ops 15, engine geometry +
  window art + cursor sprite 36, Record mouse / waypoints 28, Sharpen / zoom details 26, Click and go / move_to 10. The code is
  ordinary Rust at `opt-level = "s"`, about 90 B per source line: there is no single big thing left, so another -200 KB
  means dropping features or changing the whole app's profile (`opt-level = "z"` for the crate: not tried, a perf
  decision for the maintainer; `panic = "abort"` is out, the decoders and jobs rely on `catch_unwind`).
- The 1.2.0 diff is not just additions: `html_ctl::import_file_w` and the snapshot job shrank 22 KB because
  `mhtml_to_folder` / `capture_mhtml` became shared functions (+21 KB) - the import reuses the HTML engine's code.
- Load bar (Page only): it draws on the GPU path and in an exported mp4. `gpu::browser_tests::
  the_page_only_load_bar_is_drawn_on_the_gpu` passes (the bar is the `BG` slot's 1x1 frame, placed along the top edge);
  the CPU twin is in `engine::browser::tests::the_page_below_shows_inside_the_window`. Live: click-and-go with a 1.6 s
  load, exported 1920x1080 mp4: the accent bar is 892, 1456 and 1849 px wide at 2.5, 2.9 and 3.4 s and absent before
  the click and after the load. I could not reproduce a "wrong slot" bug: whatever it was, it is not there now.
- Tests added: `gpu::browser_tests::a_window_zoom_with_the_chrome_on_is_the_window_drawn_at_that_scale` (Window zoom with
  tabs, address bar and mouse == the same window at that Scale, at rest and mid-move, and as sharp);
  `the_page_only_load_bar_is_drawn_on_the_gpu`; load-bar assertions in the CPU page test. The scaled-frame ==
  direct-render and the mid-zoom sharpness tests were already there (`a_zoomed_frame_is_a_direct_render_at_that_scale_not_
  an_enlarged_picture`: 0..3 grey levels, sharpness metric at t = 1 s mid-move).
- `browser.animate {preset: zoom_to, target: "<element text>"}` already held the zoom on an element by name (page zoom
  and `whole`); checked live on a local page (anchor kept at the button's place). No separate `browser.zoom_to` tool.
- Tools are now: browser.add, browser.set (read when no `fields`), browser.animate, browser.page (fit, next, click_go,
  elements, sharpen, make_editable), browser.import (import, jobs, sign_in), cursor.set (move_to, path, event, press,
  tidy, samples), cursor.record (start, stop). Old names are gone (no alias).
- Live (combined tree: origin/main + html-load's Edge fix + this branch, release exe, scratch profile, port 7492,
  pid checked before every call, window on DISPLAY2 left): Window zoom with the chrome ON, exported mp4 (h264 1920x1080,
  360 frames): frames at 1.7, 2.9 and 6.0 s show the address text, the tab and the cursor growing together, sharp (no
  enlarged bitmap); the Import Web Page window opens from the action and shows the right text (Import stays disabled
  until an address is typed); a local page imported by `browser.import`, Make editable made 10 fields incl. `image_1`;
  changing the image field re-drew the page with the other picture; the tab rows in the Inspector are aligned columns
  with the grip, truncated title, [front] [icon] [remove] and a title box under the list.
- A gotcha that cost an hour: the FIRST clip added to an empty project resizes the project to its picture (a 1920x4200
  screenshot -> a 1920x4200 project): exports then look horizontally stretched. `project.set` AFTER the first clip.
- NOT verified: dragging a tab row and Alt+Up/Down with a real hand (list_rows has its own tests in feat/thread-viewers;
  nothing here drives egui input); sign-in; Record mouse by hand; slide / zoom page presets live; playback smoothness by eye.
- list_rows: `src/ui/list_rows.rs` is a byte-for-byte copy of feat/thread-viewers's (uncommitted there), plus one line in
  `ui/mod.rs`. If that branch merges first, drop this copy (identical content: git should take it as the same file).
- se-html helper publishing (2026-11-14): the install 404ed because library/bin/se-html.json was never on main. Fixed: exe is the release asset se-html-v0.1.0, manifest committed to origin/main only. How to ship an update: tools/se-html/README.md, "Ship (and update) the helper".
- Corrupt-project repair (2026-11-15): `model/repair.rs` (`Project::load_repaired`) salvages BOM/NUL/trailing garbage, truncated files, and drops only damaged fields/entries/clips. `open_project` falls back to it (original copied to `<name>.sedit.damaged`, opens unsaved, toast lists losses). `import_files` now opens a `.sedit` as a project instead of probing it as media (the "Can't import ... tedcaptions" error). Test: `SE_REPAIR_FILE=<path> cargo test real_file`. NOT live-checked in the UI. export_ui.rs tests were already broken by someone's uncommitted edit (missing `frame` helper).
- Editing skills (2026-11-16, feat/editing-skills): 16 new library skills (techniques, 6 `style-*`, `/convert-from-editor` +
  4 converted ones) and 13 upgraded, all run end to end on scratch instances twice (reports + contact sheets in the side-session
  notes folder). `library_skills_name_real_tools_and_args` fails the build when a skill names a missing tool/argument or a
  lowercase `clip.keyframe` property. Skill-writing gotchas found live: `edit.begin` closes after 30 s idle (measure first);
  transcripts stay in source time on the original clip after cuts; trims don't carry linked audio (ripple video, then audio
  without ripple); `audio.analyze` ignores clip gain; `color.auto` samples the playhead frame; `library.apply` has no
  intensity (`clip.effects_bulk {params: {Intensity}}`); contact sheets cap at 16 and `to = end` is black.
  `connecting_loads_the_index_once` fails on a machine with installed library effects in the real %APPDATA% (environmental).

## Animator / Animate page (feat/animator, 2026-11-16)
- Design approved from an HTML preview (artifact "Animator Window"); hand-over notes and screenshots in the session notes folder.
- Blocks stay keyframes: `AppliedBlock.ctl` (BlockCtl) regenerates a block's keys from its controls; `KeyBlock.gen` marks a simulated
  block. Old applied blocks have no `ctl` and stay plain keys (no migration).
- `Clip::src_time` now integrates the speed ramp (`src_offset`, Simpson per key segment). Before, it was local x rate, which jumped
  between keys; existing ramped clips now land on different (correct) source frames.
- Flow reads speeds at a hand-off exactly from the ease handles (`edge_speed`); sampling a frame-fraction away disagreed with what
  `set_block_speed` wrote.
- `apply_key_block` keeps chaining (the Gallery's repeat clicks); the Animator's exact placements use `place_key_block`.
- Pre-existing failures on origin/main seen while testing (not this branch): `library_index_matches_files` (index.json out of date),
  `connecting_loads_the_index_once`, `video_thumbs_colours_aspect_and_cache` (3 s timeout under load), and the `engine::html` tests
  when Edge will not start ("no DevTools port after 20 s").
- NOT verified: the compact list by eye (only its idle-repaint test), dragging blocks / handles by hand, Draw a motion path with a
  real pencil stroke, Link to layer through the menu, onion skin while scrubbing by eye, playback smoothness of ramps.

## Animation templates (claude/animator-templates-5e186d, cut from feat/animator, 2026-11-17)
- A template is still the Gallery `template` item (`{clips, assets}`); `engine::templates::TemplateData` adds optional `transitions`,
  `layers` (track order), `slots` and `thumb_t`. Old templates decode with those empty and place exactly as `place_clips` did.
- A slot IS a container clip (the existing `Clip.container`); `Slot` (name, clip id, wants, fit, card) lives in the template, not on
  the clip. `Project::place_template` + `Project::fill_slot` (`model/ops/templates.rs`) do the placing; `templates::apply` is the one
  entry (UI, drop, MCP all go through the `templates.use` tool = one undo).
- Fit: Trim = slot keeps its length (longer footage cut, shorter slowed via `clip.speed`); Stretch = slot takes the footage length,
  later template clips shift, keys / blocks in a clip's second half move with its end (a shrinking clip is scaled instead).
- An empty slot of a built / built-in template has a colour card (a Shape clip one layer below, same blocks); `apply` removes the
  card when the slot is filled, and a track that was added only for cards.
- Built-ins (`templates::builtin(name, w, h)`) are generated for the project's frame size, not stored JSON.
- Sharing: one `.setemplate` file = `SETEMPLATE1\n` + u64 header length + JSON header (name, files, data) + raw file bytes. Chosen over
  a zip because the repo only has an UNzip (PowerShell); this is stdlib, streamed and unit-testable. Footage unpacks to
  `gallery\templates\footage\<stamp>-<slug>\`. Asset paths in the header are file names; any other absolute-path string is scrubbed.
- Legacy `capture_template` (clipboard, Save to Gallery > Template) is unchanged on purpose (no layers): paste behaviour stays.
- A slot plays footage sound only if the clip it was made from had a linked sound half; built-in slots are picture only.
- User templates are not rescaled when placed in a project of another frame size (built-ins are generated per size).
- Checked live (debug exe, scratch profile, port 7513, window on DISPLAY2, driven over MCP + ui.input / ui.screenshot): the
  Templates tab with the ten cards and their slot marks; Use Template (rows, Use selected media in selection order, Place);
  Photo Slideshow / Photo Grid / Title Intro / Before and After placed and rendered (frame.export contact sheets); the Template
  Builder window on a selection; templates.build (card picture written), export with footage (2.36 MB) and without (36 KB),
  import of both (numbered names, footage unpacked), a stretch slot filled with a 6 s clip (fade out stayed at the end).
- Found doing that, fixed here: the Inspector's Scale box (`inspector_audio::prop_row`) clamped a shown scale of 0 to 0.01 and
  counted it as an edit, which the multi-selection wrote to every selected clip (placing a template selects its clips with the
  playhead on a Pop In's first frame). `clamp_existing_to_range(false)` + a test.
- The empty-container label box is see-through now (alpha 220 -> 96, `playback.rs` + `compose.rs`) so a slot's colour card shows
  under it. The label itself is still the old one: drawn small and stretched to the frame (blurry). Worth a real placeholder look.
- NOT verified: dragging media from the Media Library onto a slot row, the row's file picker (...), reordering builder rows by
  hand, the Export... file dialog from a card's menu, dropping a .setemplate on the window, the other six built-ins by eye, playback
  smoothness of a placed template. An imported template's card picture showed only after the next repaint.
- D: ran out of space mid-session (0 bytes); this worktree's `target/` was moved to the session scratchpad on C: and deleted from D:.

## Gallery Video | Audio + title / template pictures (2026-11-17, feat/gallery-audio-video)
- Audio side tabs are `GalleryTab::AudioAnimate | AudioTransitions | AudioEffects` + `Sounds`; `GalleryTab::VIDEO` /
  `AUDIO` are the strips. Side + last tab per side live in `Settings.gallery_audio` / `gallery_tab` / `gallery_audio_tab`
  (restored by `GalleryState::from_settings`, saved by `gallery_ctl` on change). A NEW selection pre-picks the side
  (`auto_side`) only when the pointer is not over the pane; a video clip with linked sound leaves it alone.
- Audio effects are Mixer buses: `mixer_fx::AUDIO_FX` / `AUDIO_SWEEPS` through `Project::apply_repair` (also by
  `audio.repair`). Sweeps key the cutoff over the clips and get a bus each. New `FilterKind::Pitch` (two-tap delay line).
- Audio transitions: `Transition.curve` (`XfadeCurve`, skipped when Linear so old files are unchanged); FadeToColor on
  an audio track = a dip to silence (`audio_side_gain`, used by `mixer::gains`). J / L-cut = `roll_edit` on the sound only.
  An audio card leaves linked video transitions exactly as they were (snapshot + restore around `add_transition`).
- Hover audition (`gallery_audition.rs`): 4 s of the selected audio clip rendered off-thread to a WAV in
  `<cache>/gallery-audition`, played by the Sounds preview (PlaySound). NOT heard live (would play on the user's speakers).
- Title / Template pictures (`template_thumbs.rs`): HTML via `html::render_frame` (Blitz first), else a scratch project
  through the CPU compositor at 4x, the first of 9 moments with >= 80 % of the peak "ink", framed around the content;
  `<cache>/gallery-thumbs/<key>.rgba` keyed by the template JSON. A failed render retries once, then the type icon.
- In a fresh worktree `core.autocrlf=true` checks `library/` out as CRLF, so `library_index_matches_files` and
  `connecting_loads_the_index_once` fail there (sizes); not a code problem.
- Quick Export (2026-11-17, feat/quick-export): top-bar button (`Action::QuickExportAs`, `ui/app/quick_export.rs`, MCP
  `export.quick_file`). "Single footage" = `source_video` set, it is the only asset, every clip with media uses it, no
  sequences, file on disk (module doc). Overwrite renders beside the file, hard-links `<name>.bak`, renames over, deletes
  the backup. Found while testing: Overwrite Original Video could never replace the file - `Player::release_files` also
  empties the frame cache, so the paused render thread re-rendered the current frame and REOPENED the source before the
  rename. Fix: `player.set_project(&Project::new())` before `release_files`. `explorer <path>` opens Documents for a
  forward-slash path - normalise first. NOT verified: the native Save As dialog and the Replace window by hand
  (computer-use was denied); the overwrite was driven through `export.quick_file` + the in-app save prompt.

## Ctrl+P palette (2026-11-17, feat/ctrl-p-palette)
- `ui/palette.rs`: `mode()` picks by the FIRST character only (plain = go to, `>` commands, `/` assistant, `:` tools by name).
  Go-to rows score an `Item` index built by `palette::index` once per project change (`palette_ctl::project_sig`: undo/redo
  length + top entry time, open sequence, counts, project path) and on each open; rows are cached per (query, sig).
- What a go-to does is a pure `palette::plan` (tested without an App); `palette_ctl::go` applies it. Clips inside sequences
  open that sequence; `timelines()` knows the open one lives in `Project.tracks` and main in `main_stash`.
- `/` reuses `assistant_ctl::ask` (a `/skill args` text runs the skill there) and `assistant_ctl::palette_row` - the only
  touch in assistant files is `..Default::default()` in that row (Row grew `sub` / `group` / `alt`).
- Shortcut: default Ctrl+P (nothing held it). `Hotkeys::from_settings`: an untouched default yields to a chord the user bound
  themselves, and the palette then falls back to Ctrl+K. Ctrl+K stays an alias in `palette_ctl::tick` while unbound
  (the Premiere preset gives it to split, so the preset no longer moves the palette).
- `ui.palette {query, pick, alt}` lists or runs rows over MCP (kind/group/subtitle/alt per row).
- Gotcha: a Tab on the frame right after opening beat egui's focus lock (the lock exists only once the box has focus);
  the box now re-takes focus whenever it lost it. Git Bash rewrites a lone "/" argument into C:/Program Files/Git/ -
  use MSYS_NO_PATHCONV=1 when driving `/` queries through ui.input from bash.
- NOT verified: Alt+Enter on a Gallery card live (gallery.apply path; tested only as a plan), very large projects by timing.

## Assistant chat app (2026-11-17, feat/assistant-chat)

- The "copy gives a giant string" bug: Ctrl+C with clips selected fires `CopyClips`, whose template JSON (HTML layers
  included) goes to the OS clipboard (`App::os_clipboard`). Copy / Cut of clips now stand aside while a label holds a
  text selection (`hotkeys::fires_with_text_selected`) and while the pointer is over the Assistant. Message copies
  go through `markdown::plain` (Copy text) or the raw Markdown; tool-call lines are never copied.
- egui only makes a label selection from a real drag: headless synthetic drags (and `ui.input` keys, which are
  `Key` events, not `Event::Copy`) don't select or copy. Live-checked in a scratch instance instead.
- A right-click on a message is `menu::context_area` (a rect, not a response): its selectable labels take the
  right-click themselves, so `menu::context` on the message never fired.
- Picture paste: egui-winit sends no event for Ctrl+V when the clipboard holds no text, so `assistant_ctl::paste_tick`
  reads the V key with `GetAsyncKeyState` while the composer has focus and takes the picture / file list through
  `arboard` (already in the build via egui-winit).
- Captures ask for a screenshot one frame after they are queued (`Shot`), so the menu or the area overlay is gone from
  the picture; the crop is `tools_uikit::crop_frame`. Captures keep up to 2048 px (`CAPTURE_MAX`), other images 1024.
- Line-by-line agents (codex, custom) now merge consecutive stdout lines into one Markdown text, so fenced blocks and
  lists render whole.

## Panels + multi-screen (2026-11-17, feat/panels-multiscreen)
- Voiceover / Transcript / Rendering are panes (`ui::voiceover_ui`, `app::voiceover_pane`, `subtitles_ui::show_transcript_pane` + `app::transcript_pane`, `app::render_pane`); the Voiceover, View transcript and Exporting… windows and the Export Frame window are gone. The Subtitles pane lost its Transcript tab and Transcribe… toggle (a Transcript button opens the pane).
- Audio page default rebuilt (`Layout::audio_layout`); the Beat Aligner is found by title (`timeline_tabs`), so merging feat/beat-aligner needs its own `mix` line in audio_layout dropped (it would tab the aligner twice) and `BeatAligner` added to the Audio row of `every_page_contains_every_pane`.
- Multi-screen: `Layout.screens` (one tile tree per extra window, panes keep a hidden tile in the main tree), drawn as immediate viewports in `layout::show_screen`; monitors / prompt / assignment in `ui::screens`; per-count layouts (`Settings.screen_layouts`, `"<n>|<page>"`) and placement in `app::screens_ctl`; `layout.screens` MCP tool. SE_BACKGROUND keeps screen windows on the main window's monitor (never the user's).
- Live check: screen capture of the scratch instance's windows came back white for the main window too (environment), so the 2/3-screen windows were checked by window enumeration (titles, rects) plus ui.screenshot of the main viewport.

## Export size (2026-11-17, fix/export-size)
- Why exports were huge: "auto" picks `h264_nvenc`, and `codec_args` gave it `-cq <crf> -b:v 0` with ffmpeg's NVENC defaults (Main profile, no B-frames, no lookahead, no AQ). NVENC's CQ is not x264's CRF: CQ 18 saturated at 16 Mb/s on the user's aria-math (720p60, Voxel Blocks visualizer, 2:12) = 267 MB. Container, pix_fmt (yuv420p 8-bit), audio (AAC 192k) and GOP (250) were all fine.
- Method: the user's 267 MB export (the rendered frames) re-encoded with each arg set, VMAF (libvmaf, subsampled) against it; plus a clean synthetic 1080p60 30 s clip (testsrc2 + light temporal grain, crf 6 master). VMAF vs the NVENC reference understates absolute quality (it is itself lossy) but ranks fairly.
- aria-math (MiB, VMAF): old default nvenc cq18 254.6 / 95.2 · old Fast preview cq28 123.1 / 88.4 · old Max quality x264 slow crf16 217.2 / 93.7 · x264 medium crf20 109.3 / 89.1 · crf23 65.4 / 84.8 · tuned nvenc cq23 178.6 / 92.5 · cq26 122.5 / 90.1 · cq29 82.0 / 86.8 · hevc_nvenc cq26 118.6 / 89.5 · 50 MB target: nvenc multipass 48.5 MiB (50.9 MB, +1.7 %) / 81.2, hevc 48.6 / 82.5, x264 ABR 46.8 MiB (49.1 MB, -1.8 %) / 80.7.
- Synthetic 1080p60 30 s: old default nvenc cq18 133.8 MiB / 98.3 (37 Mb/s) · x264 crf20 49.9 / 95.8 · crf23 32.8 / 94.0 · tuned nvenc cq23 95.5 / 97.0 · cq26 59.9 / 95.9 · hevc cq26 54.1 / 95.7.
- Decisions: tuned NVENC (`-tune hq -rc vbr -rc-lookahead 20 -spatial-aq 1 -temporal-aq 1 -bf 3`, High, `b_ref_mode middle`) matches x264 medium per bit here at ~5x the speed, so auto stays NVENC; CQ = CRF + 6 (cq26 ≈ crf20 on both clips). Default CRF 23 (NVENC cq29), YouTube tiles 21, Shorts/IG 23; old stored defaults migrate once (`Settings::migrate_export_defaults`). HEVC on this Turing card is only ~5-10 % smaller: opt-in, auto stays H.264. `-g` = 2 s everywhere. Quick Export without a previous export now uses the default quality, not Max quality.
- aria-math at 50 MB is visibly softer on the grass texels at 100 % (pixel-art hash textures under an orbiting camera are about the hardest content there is); the default lands at ~82 MiB (-69 %), Target size 50 MB hits it.
- Target size: `ExportOptions.target_bytes` -> `target_args` (NVENC `-multipass fullres` = two-pass inside the encoder; x264/x265 single-pass ABR with VBV; both within ±2 % on aria-math). No intermediate render needed.
- Estimate: `engine::size_probe` encodes 5 x 2 s samples (one GOP each) with the real settings (GPU frames, waits while playing, cancelled by a new key) and scales up; the formula (0.075 bpp at CRF 23) only shows, with a trailing "…", until it has measured.
- QSV/AMF arg mapping untouched (no hardware here to measure).
- D: filled to 111 MB free mid-task (other sessions' builds); this worktree builds with CARGO_TARGET_DIR on C:.

## Device emulator (2026-11-18, feat/device-emulator)
- Panes / tools: `ui::emulator` (gallery + large view), `app::emulator_ctl` (`emulator.open`, `emulator.screenshot`); pages in `ui::social::{youtube, vertical, sheets, frame}`; devices + layouts in `assets/guides.json`.
- Icons: `library/icons/platforms/<net>/*.svg` -> `bash scripts/platform-icons.sh` (headless Edge) -> `assets/social_icons.png`, order = `atlas.txt`. X's action icons come from the post page's `<symbol id="icon-*">` defs (the buttons only hold `<use href>`): read `document.getElementById(id)` paths, not the button's SVG.
- Shorts' own comment / remix icons are still the drawn shapes: youtube.com/shorts never lays out its action bar in the Claude browser pane (desktop or mobile emulation, pane hidden = throttled page). Try again from a visible, real Chrome window.
- `emulator_shots` (ignored) renders every platform x device x state: `SE_SHOT_DIR=<dir> cargo test emulator_shots -- --ignored`; compare with the refs in the side-session notes `device-emulator/refs`.
- With the Preview pane hidden (another page, or the emulator maximised) the pane uploads the pending frame itself (`emulator_ctl::draw`): checked live.
- Launching a scratch instance from a Claude Code session: strip CLAUDE_* / ANTHROPIC_* from the child's environment, or the in-app Assistant inherits the host session's auth.

## Performance pane "freeze" report (2026-11-18, fix/perf-freeze) - not reproduced, parked
- Report: opening the Performance tab froze the installed 1.3.1. Clue from the user: the Device Emulator panel was open at the same time; it may have been a general system slowdown. Parked until it happens again.
- Tried (installed release exe, scratch profile with a copy of the user's settings, copies of the aria-math and untitled-21080 autosaves): pane on Edit and Export, Project and Live tabs, idle and playing, Jump-to the Audio Visualizer clip (1.3 MB of baked shader keys), and the Device Emulator open together with the pane. UI round trips over MCP stayed at 2-220 ms; Live showed 59-62 fps, 0 dropped, UI p95 ~1.2 ms. No stall.
- Ruled out by reading or measuring: `perf_sample` (brief `ahead` lock), the HUD frame window (capped at 120), the donut point counts (bounded even for NaN/inf), `cache_bytes` (own thread; the user's cache is 2.6 GB / 2,066 files), and `fs::metadata` on V: (a local SATA HDD, ~0.1 ms). The exponential `nest()` walk is already fixed in 166d9b3.
- If it comes back: grab thread stacks of the frozen PID before killing it (`procdump -ma <pid>`), and note the page, tab, panes open and whether it was playing.

## Device emulator devices + gallery fix (2026-11-18, fix/emulator-devices)
- Bug "switching from YouTube Shorts changes nothing": reproduced on the installed 1.3.1 with the Gallery open. The gallery draws a fixed set of pairings (its first cards are YouTube Shorts) and ignored the toolbar pickers; the pick was saved but nothing on screen changed. `emulator::toolbar` now leaves the gallery for the large view on any platform / device / orientation pick. The large view itself always redrew; regression test `switching_platform_device_or_orientation_redraws` drives the real toolbar (clicks found via egui's debug widget info: `style.debug.show_interactive_widgets` keeps labels; a ComboBox's text is `current_text_value`, not `label`).
- 101 devices in `assets/guides.json`, generated by a small script from the verified spec table in the side-session notes `device-emulator/device-specs.md` (Apple / ios-resolution / iosref / useyourloaf / yesviz; corner radii from BezelKit). Not added, sources could not confirm: Pixel 9a, Galaxy A15 / A16 / A56. Android dp that sources disagree on: took px / DPR.
- New `Device` fields: `sc` (pixels per dp), `px` (real panel pixels when not w x h x sc: Plus models 1080x1920, minis 1080x2340, Macs' "looks like" sizes), `face` [side, forehead, chin] dp for home-button models (from body mm x the screen's pt per mm). `group()` drives the picker; `frame::insets` gives the body per side (turned: forehead on the left).
- Phone platform UIs were inset by `st - 24` (they overlapped the status bar by 24 pt everywhere); the mobile-web references start under the bar, so the inset is now the whole `st`.
- Slow all-states device tests run on `sample_devices` (smallest + largest of each picker group), not all 101.
- Export (Device Emulator button, `emulator.export`): `raster::device_export` is a `FrameOverlay` whose second field is the size it returns (`export.rs` feeds ffmpeg that size and stops recycling buffers). The moving ops are found once by building the layout at the start and the end and diffing; everything else is drawn once into `Layers` (under the video, still overlay and corner mask as premultiplied colour + coverage from a black and a white pass). Each frame = copy, blit, blends, the moving ops. Live: 8 s Shorts on an SE took 76 s on a machine pegged at 100 % by other builds, where a plain Quick Export of the same project took over 280 s.
- Fixed (fix/emulator-scrim): full-width scrims, bars and progress bars stopped 4 dp short of both screen edges (`Map::axis` applied the 4 dp side margin to every edge); an edge on the reference screen's own edge now stays on the screen edge, buttons keep the margin.
- MCP permissions (2026-11-18, feat/mcp-permissions): `mcp/perms.rs` (Allow/Ask/Block per tool, defaults from ToolKind;
  modes Ask / Accept edits / Bypass; `--mcp-full-access` / `SE_MCP_FULL_ACCESS`, on by default for debug and worktree
  builds, `=0` off). Settings ▸ Permissions tab; per-chat mode in the composer; the agent's MCP URL carries `?chat=<token>`
  so only that chat's calls get its mode; `permissions.list` / `permissions.request` (answered only by the user's click,
  inline in the chat). Note: `media.import` and others are registered as `ToolKind::Read`, so they default to Allow.
  Not live-checked: a real claude/codex agent calling permissions.request (tested with a curl "custom" agent).

## Screen recording (2026-11-18, fix/screen-record)
- Root cause of "moov atom not found" (user's screen-1795024517.mp4): the stop path was fine (`q` on stdin), but
  `poll_recordings` imported as soon as `out.exists()`, which is true from the first mdat byte - before ffmpeg wrote
  the moov. The user's file was finalised a moment later and plays (72.5 s, h264 + aac, decodes clean): nothing to recover.
- Now: ffmpeg records video only (gdigrab, `-tune zerolatency`, `-g 2*fps`, `-cluster_time_limit 1000`,
  `-flush_packets 1`) into `<name>.rec.mkv`; `engine::audio_tap` (cpal/WASAPI; loopback = input stream on the default
  render device) writes raw f32 to `<name>.system.f32` / `.mic.f32`; `<name>.rec.json` holds paths, formats and start
  times. gdigrab stamps frames with wall-clock µs (ffmpeg's `start:` line at -loglevel info), the taps stamp their
  first sample on the same clock -> `adelay` / `atrim` in `capture::mux_args`. `finalize` muxes (video copy, AAC,
  `-t <video length>`: `-shortest` never ends a graph with apad/anullsrc), probes, then deletes the pieces.
  The app imports on the capture's Progress done + no error (`PendingRecording.done`), never on "exists".
- Crash safety found by testing: x264's default frame threads held every frame of a 3.5 s recording in memory and
  ffmpeg's write buffer held the rest (a killed MKV was 587 bytes) -> zerolatency + flush_packets. ffmpeg ignores
  stdin closing, so after an app crash it kept recording forever -> `html::tie_to_us` (kill-on-close job). The
  Chocolatey ffmpeg.exe is a shim: `Child::kill` kills only the shim -> the stop timeout uses `html::kill_tree`.
- `unfinished(dir)` (pieces untouched for 30 s) runs at startup (`recover_recordings`); checked live: app killed
  mid-recording -> relaunch -> mp4 rebuilt, imported, toast. Closing the window mid-recording stops, finishes,
  imports, then closes.
- Live (debug exe, scratch profile, port 7541, `capture.screen` MCP tool): 10 s in each mode with a 1 kHz tone
  played; system and both have the tone at full level, mic only hears it acoustically (-34 dB), none has no audio
  stream. NOT explained: in Both mode the tone's level varied run to run (-12 to -26 dB) while system-only stayed
  at -12; the mux keeps levels (checked with a synthetic tone), so it is in Windows' playback chain - possibly
  communications ducking when the mic opens (UserDuckingPreference is the default, reduce by 80 %).
- gdigrab reached ~27 fps at 2560x1440 while other sessions were compiling. Tests that record for real write to
  `SE_TEST_DIR` (else temp). Pre-existing on release/1.3.1: `library_index_matches_files` and
  `connecting_loads_the_index_once` fail in CRLF worktree checkouts; `clear_forgets_rewritten_file` is flaky.

## Recorder 2 (2026-11-18, feat/recorder-2)
- Preview: `screen_grab::FrameSlot` (newest frame + generation + `wanted`). Producers: the recording's own ffmpeg
  (second output `fps=30,scale=480:270,pad,rgba -f rawvideo pipe:1`; progress moved to `-progress pipe:2`) for
  regions / windows; `wgc::AppCapture` for applications and whole monitors (GPU mip-chain downscale, only the
  small level read back, double-buffered staging so the thread never waits on the GPU). Idle: `Preview::start`
  (preview-only ffmpeg or WGC). Nothing runs unless looked at: pane drawn, its viewport not minimised,
  `Settings.capture_preview` (right-click toggle). No occlusion signal in egui 0.33, so a covered window keeps
  previewing. A gdigrab idle preview of a 2560x1440 monitor at 30 fps cost ~80 % of a core in ffmpeg: that is
  why monitors go through WGC.
- App capture: WGC frames -> raw BGRA on ffmpeg stdin with `-use_wallclock_as_timestamps 1` (sync like
  gdigrab); EOF stops it. Fixed output size = window size at start (resize crops / pads). App audio:
  process loopback (`audio_tap/app_audio.rs`), hand-made static COM completion handler like `disc::imapi`.
  The windows crate has `Drop for PROPVARIANT` (extensions/.../StructuredStorage.rs) = PropVariantClear: a
  PROPVARIANT pointing at a stack blob must be ManuallyDrop, or CoTaskMemFree corrupts the heap
  (STATUS_HEAP_CORRUPTION at the first recording with an App audio track).
- Tracks: `TrackSpec{name, kind: Mix|System|Mic|App, on}`; `mux_args` reads each piece once, `asplit`s it for
  several tracks, a silent source -> `anullsrc` stream (count and order always match), `-metadata:s:a:N title=`.
- Hide on Record: `rec_pill` (Win32 popup + RegisterHotKey on one thread, WDA_EXCLUDEFROMCAPTURE, ShowWindowAsync
  so it never waits on the UI thread). Bins: `jobs::put_in_bin`. `capture.screen` gained source 'app',
  window 'pid:<n>', preview, dock.
- Measured (debug exe, 2560x1440 screen 1, other sessions compiling; CPU = % of one core, app + ffmpeg tree):
  before (fix/screen-record, 4 fps GDI preview): idle preview 15 %; recording alone 137 %, with preview 160 %.
  after: idle screen preview (WGC monitor) 30 fps, 30 %; recording with preview 195-235 % vs 220 % paused
  (ffmpeg varied with the video on screen; the preview's own share is the app's ~30 % vs 7 % paused);
  app capture recording (1450x1008, 3 tracks) app 43 % + ffmpeg 47 %. A gdigrab 30 fps idle preview of
  the monitor was 108 % (why monitors use WGC). Lag (`preview_bench`, capture -> frame stored, + up to one
  33 ms UI frame): ffmpeg path 30 fps, median 93 ms (worst 102; 119 ms with the fps filter); WGC 30 fps,
  median 65 ms (worst 66-82).
- Live: app capture of a second scratch instance playing a 440 Hz video: video 1450x1008 (its window), 3
  streams - All audio (tone -10 dB), Microphone (acoustic only, -31), App audio (the tone alone, -18).
  Imported into the Recordings bin. Hide on Record: editor + popped pane minimised and affinity 17
  (excluded) while recording, pill visible and excluded, WM_HOTKEY to the pill stopped the recording and
  restored both. NOT shown: the cursor in a screenshot (the user's pointer was hidden; not moved).
- launch.ps1 children die with the PowerShell tool's console; launch through `cmd /c` with CreateNoWindow.
  Never rewrite a scratch settings.json with PowerShell's ConvertTo-Json/Out-File (BOM): the app then
  loads defaults - capture_dir "" = the user's Videos folder.

## 2026-11-19 UI rule: panel buttons toggle
A button that opens a panel/pane/window (e.g. the top-right Performance button) toggles it: a second click closes it, and the button is highlighted (accent selected fill) while its panel is open, following the real state even when the panel is closed another way. Applies to every such button; use the shared toggle helper.

## 2026-11-19 docs for 1.4.0 (claude/1-4-0-b2656e)
- New pages: media-player, animator, project-settings, screen-recording (moved out of audio.mdx). Updated
  interface, getting-started, assistant (chat layout + permissions), export (Target size, Quick Export), inspector,
  music-and-discs, README features; shortcuts.md regenerated with `--dump-hotkeys`.
- docs-shots.ps1: new rows for the 1.4.0 pages, per-shot `env` and `settings`, a `pick` step (Library
  selection), and `$PlayerShots` for the media player (SE_PLAYER_SHOT + SE_PLAYER_SHOT_SETUP, no MCP there).
  Player setup must not use `at=`: it runs at reveal, before the file's length is known, and the seek is lost
  (black frame, 0:00 length). The player takes no `--size` (it opens at window_rect). Every player demo file is
  silent (anullsrc) - the older demo clips have sine tones, never open them in the player.
- Dev builds have MCP full access, which shows a "Full access" badge: the assistant / permissions shots run with
  SE_MCP_FULL_ACCESS=0 (+ assistant_guard false so `act settings` doesn't wait for an Allow).
- Pass -Only as an array (`-Command "& .\scripts\docs-shots.ps1 -Only @('a','b')"`); with -File it arrives as one
  string and nothing matches. Windows can keep demo media open after a run: use a fresh -Work if it can't be wiped.

## Page-switch freeze (2026-11-19, claude/fix-page-switch-freeze)
- User report: clicking a page (Audio) sometimes froze the editor. Three blocking reads on the UI thread:
  the Voiceover pane's first draw ran `ffmpeg -list_devices -f dshow` (4.7-16 s on this machine), the Export
  pane's first draw ran `ffmpeg -encoders` (2.4-3.1 s), and the Inspector (Asset section) plus every Library
  row stat'ed the source file each frame (`proxy::needs_proxy`, 30-90 ms under disk load).
- Fix: `engine::bg_probe::BgProbe` (answer or start-once-and-None), `capture::audio_devices_nonblocking`,
  `export::encoders_nonblocking` (+ `forget_encoders` on an ffmpeg folder change), warmed on the first frame
  (`panes::warm_probes`); `proxy::file_len` = one stat worker + a 30 s cache. Regression test:
  `audio_page_first_draw_does_no_blocking_probe` (no UI code calls the blocking probes).
- Also on every switch: `switch_page` wrote settings.json on the UI thread (40-300 ms on a busy disk). It uses
  `Settings::save_async` now (snapshot, write on a thread, newest save wins by a generation number, one lock
  around write+rename; `Settings::save_pending` -> a sync `save` when the window closes). The other ~110
  `settings.save()` callers are still synchronous - a candidate for the same treatment.
- `SE_FRAME_TRACE=<ms>` (perf.rs): every UI frame slower than <ms> goes to stderr (from a writer thread)
  with cumulative section marks (@hooks/@pre-layout/@layout/@acts/@windows) and each pane over 1 ms, plus
  "frame gap" lines with eframe's update+paint for the previous frame. Writing the lines on the UI thread
  showed up as fake 0.5 s hitches while cargo hammered the disk - keep the writer thread.
- Measuring through MCP on a loaded machine: `layout.page` round-trips of 1-25 s were idle time (HTTP), not UI
  work - read the trace, not the call time.

## 2026-11-19 TikTok / Snapchat text (claude/feat-social-text-styles-4016d6)
- `TextStyle.box_mode` (BOX_BLOCK / BOX_LINES / BOX_BAR) + `box_radius`, both `#[serde(default)]` (old files = one
  square block, as before). The box is a coverage mask (`engine::text::box_mask`) drawn in the rasterizer, so the
  cached bitmap carries it and preview (CPU + GPU via `playback::layer_for`), export, proxies and thumbnails all get it.
- BOX_LINES: `line_boxes` gives one rounded rect per line plus concave quarter-circle fillets where widths step;
  neighbours meet on a whole-pixel row (no AA seam). Radius and fillet split the width step (each <= half of it).
- BOX_BAR: needs the frame width, so `TextRasterizer::render_in(style, scale, t, frame_w)` (cache key gains the width
  only for bars). Callers pass `compose::text_frame_w(clip, lt, canvas_w)` (canvas width / the clip's x scale);
  `placement_w` centres a bar and drops rotation; the Preview drag keeps x for a bar. Subtitles pass the canvas width.
- Fonts bundled for the first time: `assets/fonts/` TikTok Sans + Inter, static Bold Latin subsets (~46 + 50 KB)
  made with fontTools instancer + subset from the Google Fonts variable files (OFL 1.1, no Reserved Font Name);
  `TextRasterizer::load_system_fonts` registers them first. ~96 KB of exe: over the +64 KB line, needs the release
  PR's size note.
- `reveal_bucket` now clamps t to the reveal's key range, so a typewriter stops re-rasterising after its last key.
- `SE_SOCIAL_SHOTS=<dir> cargo test each_social_preset_renders_its_look` dumps a PPM of every preset (over grey).

## 2026-11-20 crash fingerprint and safe start (claude/forum-crash-recovery)
- `crashguard`: in-memory in-flight table (probe/decode/open/drag/filter/export/graphics/encoder test), mirrored to
  `inflight-<pid>.json` by a writer thread after 150 ms, written synchronously by the UI-thread panic hook and the
  unhandled-exception filter. Never write it from the caller's thread: decoder opens happen on the render thread.
- A file the last run died on goes into `media::offline` (`offline.json`): every in-process open refuses it until
  Bring the File Back. The recovery prompt (`RecoverPlan`) names it and offers Recover / Without Media / Without Effects.
- `graphics.rs`: compatibility mode (CPU preview, no vsync, optional Mesa `opengl32.dll` in `compat-gl` via
  SetDllDirectoryW). `SE_FORCE_GL_FAIL=1` fakes a failed graphics start.
- Wine in the cloud container can't run any exe; test pure modules by including the real file in a scratch crate.

## Keep pitch: speed changes keep the pitch (claude/gap-pitch-speed, 2026-11-19, Shotcut gap #1)
- One stretch path: `engine::stretch::Grains` (WSOLA, 1024-frame Hann grains every 512, +-240-frame nudge by
  normalised correlation). `Mixer` runs every `stretch::stretches(clip)` clip through it (`stretch_add`), so
  playback, paused scrub, export and the audio bakes match; a sequence clip stretches its sub-mix the same way.
  Grains are placed from `clip.src_time`, so speed ramps (and a time-remap that keeps using `src_time`) stretch
  with no extra code: a time-remap session should call `stretch::stretches` / keep `src_time` as the mapping.
- `Grains` remembers chosen grain positions by grain index (blocks of any size join, tested 1024 vs 700) and keeps
  the source frames it read, so forwards each block reads only new frames: the decoder sees one contiguous read
  (MF drops everything before the last read; the ffmpeg pipe would respawn on a big step back).
- `Clip.keep_pitch`: true from `Clip::new`, serde default false + skipped when false, so old projects load and
  save unchanged. Above `stretch::MAX_RATE` (4x) a keep-pitch clip is muted per block (`too_fast`).
- Shuttle (JKL, `playback.rs` audio thread): 0 < rate <= 4 runs the timeline mix through a `Grains` (rate 1 path
  untouched); reverse / above 4x stay muted. The underrun catch-up now counts `queued * rate` of timeline (at
  0.25x the old check snapped every block).
- No crate: nothing small and permissive in Rust (Rubber Band GPL; Signalsmith binding needs C++), and ffmpeg
  `atempo` for export would have split export from playback. The standalone media player (`player/engine.rs`,
  `MAX_AUDIBLE_RATE` = 2, resamples in the device callback) is NOT changed: its ring is source-timed, so it needs
  its own rework to stretch.

## 2026-11-20 mixed footage (claude/feat-mixed-footage-83d1e8)
- `media::footage` holds camera make/model + gamma hint (S-Log3 / S-Log2 / HLG) + 10-bit + `hdr_tagged`, cached by the
  import probe (`ffpipe::probe` -> `footage::remember`); UI code only reads `footage::of` (never probes).
- Conversion LUTs are generated (`footage::lut_for`) and installed into the library on first use; tagged HLG is
  tone-mapped on decode, so only untagged HLG gets a LUT (`wants_lut`).
- One entry point for "make proxies?": `App::offer_proxies` (footage_ctl). Other slow-footage checks call it.
- Wine 9 in the cloud container crashes on any exe (`free(): invalid pointer`), so windows-gnu test binaries can't run
  there; pure modules can be tested by pasting them into a scratch crate.
