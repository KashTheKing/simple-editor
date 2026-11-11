# Simple Editor "Pro" plan: fast playback, fast export, separate audio tracks, live Claude co-editing

## Context

Simple Editor has the features of a pro editor but does not feel like one yet: playback is slow
"as a media player", export is slower than the hardware allows, audio always exports as one stereo
mix, and Claude can only drive the app from an outside window with no shared view of what the user
has selected. This plan fixes those four things, in the order the user chose: **playback, then
export (speed + audio tracks), then the Claude assistant**.

Decisions already taken with the user:
- Assistant backends: external MCP client, a spawned agent CLI, and API key are all fine, and
  non-Claude agents (e.g. Codex) must work too.
- Voice: local push-to-talk through the whisper the app already uses.
- Audio export: one mix (today), one stream per track inside the video, or WAV stems; selectable.

Nothing was compiled or run to write this plan. Every finding below comes from reading the code;
the numbers that matter get measured in wave 0, on a quiet machine, before any optimisation lands.

### What the code says today (verified by reading)

| Area | Finding | Where |
|---|---|---|
| Playback | The **Source monitor** (the "media player" pane) never gets GPU, proxies or a cache: it plays original long-GOP files through the CPU compositor and re-uploads via egui every frame | `ui/app/source_pane.rs:125` (only `app.player` gets `set_gpu`/`set_proxies`/`set_cache_bytes`) |
| Playback | All layers of a frame decode **one after another on one thread** | `playback.rs:1488-1518` (`decode_layers`) |
| Playback | Per video frame: MF software YUV→BGRX, a per-pixel BGRX→RGBA loop, a memcpy, then `Arc::new(out.clone())` (alloc + full copy), plus zero-filled fresh buffers until the cache fills. All at `opt-level="s"` | `media/mf.rs:562-653`, `media/mod.rs:439,454` |
| Playback | Nothing measures UI-thread frame time; no HUD | `tools_playback.rs:156` is the only stats exit |
| Export | The **Export pane hard-codes the CPU compositor**, so it is single-threaded and silently drops GPU-only effects and node graphs | `ui/export_ui.rs:123` vs `ui/app/gpu.rs:35` (`export_frames`) |
| Export | GPU export does one frame per UI repaint (vsync), a synchronous `glReadPixels`, and a fresh zeroed 8 MB buffer per frame | `ui/app/gpu.rs:46-68`, `engine/gpu.rs:412` |
| Export | Audio is mixed in full before frame 1; decode, composite and pipe-write are serial; RGBA piped, ffmpeg converts to YUV | `engine/export.rs:280-399` |
| Export | "auto" is always libx264; hardware encoders are listed from `ffmpeg -encoders`, never test-run. This machine has NVENC/QSV/AMF builds available | `engine/export.rs:748-850` |
| Audio | One stereo mixdown; >2-channel sources keep only L/R on the MF path (5.1 loses its centre channel) | `engine/mixer.rs:105-190`, `media/mf.rs:766` |
| Interchange | FCP7 XML export/import and EDL import exist. **No Avid path** (no AAF, no EDL export), and markers, track names and levels are not in the XML | `engine/xmeml.rs`, `engine/import.rs` |
| Co-edit | MCP server (240 tools) works but: one undo step per call, no attribution, one call per frame, frames returned as ~1 MB base64 *text*, `markers.list` omits notes, no `markers.set`, selection snapshot incomplete, no in-app assistant, no mic input | `ui/app/mcp_exec.rs:138-197`, `mcp/mod.rs:295`, `ui/app/tools_subtitles.rs:108-145`, `ui/app/tools_ui.rs:51-63` |

### Conflict with goals.md to settle up front

goals.md targets a ~10 MB exe and no new crates. This plan needs **no new crates** (cpal, `windows`,
`curl.exe`, `ffmpeg.exe`, `claude` cover everything), but it adds code: estimate +250 to +450 KB
over the whole plan, most of it the assistant pane. Recommended: record a one-time size-budget
decision in goals.md in wave 0 ("pro plan: up to +0.5 MB"), then hold every PR to the existing
size gate.

---

## Shape of the work

Follows the repo's concurrent-worktree protocol (agents.md): wave 0 is serial, later waves are
parallel worktrees with disjoint files, one PR each, `scripts/size.ps1` on every PR.

| Wave | Workstreams | Runs |
|---|---|---|
| 0 | `perf-baseline`, `pro-schema` | serial |
| 1 Playback | `play-source-monitor`, `play-decode-path`, `play-render-thread` | parallel |
| 2 Export | `export-pipeline` then (`export-encoders`, `export-audio-tracks`, `interchange`) | first serial, rest parallel |
| 3 Assistant | `mcp-context` then (`assistant-pane`, `voice-input`, `transcribe-islands`) | first serial, rest parallel |
| 4 | `docs-release` | serial |

---

## Wave 0: measure, and pre-seed shared tables

### 0a `perf-baseline` (nothing gets optimised before this lands)
- **Frame-time HUD** (a View toggle + `playback.status` fields): UI-thread frame ms, GL composite
  ms, texture upload ms, per-frame decode ms (sum and worst layer), cache hit rate, dropped frames.
  Timers are `Instant` pairs around existing calls in `ui/app/mod.rs` (`update`),
  `engine/gpu.rs` (`upload`, `render_canvas`) and `playback.rs` (`decode_layers`).
- **`bench_export`** (`#[ignore]`, next to `bench_4k_preview`): a fixed 10 s 1080p60 two-layer
  project through CPU path, GPU path, libx264 and the best hardware encoder; prints fps.
- **`bench_source_monitor`**: a 1080p60 H.264 file through the Source player, frames shown vs due.
- Record baselines in notes.md and turn them into budgets in goals.md (targets to confirm against
  the baseline: 1080p60 single clip plays with 0 dropped frames in both monitors; UI frame p95
  under 8 ms while playing; export at least 3x the baseline fps on this machine).
- goals.md: the size-budget decision above.

### 0b `pro-schema` (so later worktrees never share a hunk)
- New marker sections for the wave 1-3 workstream names in `TOOL_TABLES` / `ACT_HANDLERS` /
  `FRAME_HOOKS` / `WINDOW_DRAWERS` / `PANE_DRAWERS` (`ui/app/mod.rs`), `actions!`
  (`hotkeys.rs`), `Glyph`, `Pane::ALL`, `Settings`, `App`.
- Serde fields, all `#[serde(default)]`: `Marker.done: bool`, `Marker.author: String`;
  `ExportOptions.audio: AudioLayout` (`Mix` | `Streams` | `Stems`) and `stem_by` (`Track` | `Bus` |
  `Role`); `Settings.assistant_*` (backend, command template, auto-send voice), `Settings.hud`.
- `Pane::Assistant` variant declared (empty drawer), following ARCHITECTURE.md's new-pane checklist.

---

## Wave 1: playback

### `play-source-monitor` (likely the biggest win for "slow as a media player")
- One `App::configure_player(&mut Player)` that does what `app.player` gets today (`set_gpu`,
  `set_cache_bytes`, `set_proxies`), called for the Source player (`ui/app/source_pane.rs:125`,
  `ui/source_ui.rs:468,545`) and wherever proxies/GPU state change (`ui/app/gpu.rs:12-24`,
  `ui/app/jobs.rs:296`).
- Source pane paints the GPU canvas texture like the Preview does (`ui/app/gpu.rs:86`
  `gpu_preview_texture`), using a second canvas target in `GpuRenderer`, the same way the
  alt-render channel already renders a second view (`ui/app/monitor.rs`). Deletes the
  `ColorImage::from_rgba_premultiplied` copy and the unconditional per-frame repaint
  (`source_pane.rs:187,198`).
- Source player gets a small cache share (e.g. 1/8 of `cache_mb`) rather than the 512 MB default.

### `play-decode-path` (owns `media/mf.rs`, `media/mod.rs`, `engine/gpu.rs` upload, one shader)
In order; each step is kept only if `bench_4k_preview` and the HUD say it is faster:
1. **Stop copying**: decode straight into a pooled buffer that becomes the cached `Arc<Frame>`
   (kills `Arc::new(out.clone())` at `media/mod.rs:439` and the `emit` memcpy at `mf.rs:602`);
   make `Frame::resize` not zero-fill when the decoder overwrites every byte; let the spare pool
   hold more than 8 buffers so a cold cache does not allocate per frame.
2. **Stop swizzling**: keep MF's BGRX as-is and upload with `GL_BGRA` (a `Frame.bgrx` flag; the
   shader forces alpha to 1; CPU consumers convert lazily). Removes the per-pixel Rust loop
   (`mf.rs:643`) from every video frame.
3. **Spike, measure, then decide: NV12 end to end.** Ask MF for NV12 instead of RGB32, upload two
   planes (R8 + RG8), convert in the layer shader. Expected: no CPU colour conversion, 2.7x fewer
   bytes to upload, 2.7x more frames in the same cache budget. Risk: MF's processor must still
   scale to the requested size in NV12; if it does not, keep step 2 and drop this.
4. Precompute source-cache keys (no `to_ascii_lowercase` String per lookup,
   `media/mod.rs:226,243`).
5. Only if the HUD still shows hot Rust loops after 1-3: move them into a workspace crate built at
   `opt-level=3` (the `se-engine` path already named in `plans/ui-overhaul/README.md`). Skipped
   otherwise.

### `play-render-thread` (owns `playback.rs`)
- **Parallel layer decode**: in `decode_layers`, video layers that need a real decode run on
  `std::thread::scope` threads, one per distinct source, the first on the render thread itself.
  `// ponytail: a spawn per layer per frame; persistent workers if spawn cost shows in the HUD`.
- Narrow the shared `TextRasterizer` lock to the text layers only (held across all video decodes
  today, `playback.rs:1426`).
- Motion-blur neighbours: decode the earlier sample first so MF reads forward and never seeks
  backwards each frame (`playback.rs:1687-1695`).
- Nested sequences: stop cloning the whole `Project` per frame (`playback.rs:1735`).
- Canvas size: step it (reuse `compose::layer_size`'s stepping) so dragging a pane divider or a
  DPI change does not clear both caches and renegotiate MF (`playback.rs:809-811`).
- `set_project`: skip the deep clone + JSON diff when the edit is audio/marker-only (the app
  already knows the edit category).

**Not doing unless the HUD demands it** (named so nobody re-derives them): PBO texture upload,
caching the timeline's egui shapes during playback, zero-copy DXVA (D3D11 texture shared into GL).

---

## Wave 2: export

### `export-pipeline` (serial first; owns `run_export`, `ui/app/gpu.rs`, `engine/gpu.rs` readback)
1. **Export pane uses the GPU**: `export_ui.rs:123` takes `export_frames()` like every other
   entry point. Fixes speed and the silently dropped GPU-only effects in one line; keep the
   existing CPU fallback and its `cpu_gaps` message.
2. **More than one frame per repaint**: the export thread sends a small window of requests
   (bounded channel of 4, replies in order); `serve_gpu_exports` serves for a ~12 ms budget per
   `update`; reply buffers are recycled instead of `Frame::default()` + zeroed resize per frame.
3. **Async readback**: a ring of 3 PBOs in `GpuRenderer::render_frame` so `glReadPixels` never
   stalls the pipeline.
4. **NV12 out of the GPU**: one extra shader pass converts the canvas to NV12 before readback;
   ffmpeg gets `-pix_fmt nv12` input. 2.7x fewer bytes read back and piped, and ffmpeg's swscale
   pass disappears. (CPU path keeps RGBA.)
5. **Overlap the stages**: audio mix on its own thread while video renders (it already has its own
   decoder pool and temp file); a writer thread with a 3-frame queue so the pipe write overlaps
   the next decode/render. Parallel layer decode from wave 1 applies here for free.
6. Pipe audio to ffmpeg as headerless `f32le` (`-f f32le -ar 48000 -ac 2`): deletes `wav_header`
   and its 4 GB / 3 hour ceiling (`export.rs:524`).
7. Fix found on the way: `loudnorm` without `-ar` can leave WAV/FLAC at 192 kHz; add `-ar 48000`.

### `export-encoders` (owns `codec_args`, a new `engine/encoders.rs`, Export pane Advanced grid)
- **Probe once, cache**: test-encode one tiny frame per hardware encoder
  (`ffmpeg -f lavfi -i color -frames:v 1 -c:v <enc> -f null -`), cache the working set keyed by
  ffmpeg path + GPU name. Encoders that cannot run are hidden instead of failing the export.
- **"auto" picks the fastest working encoder** (NVENC, then QSV, then AMF, then libx264), with a
  visible "Speed / Balanced / Smallest file" choice mapping to encoder presets (NVENC `p1/p4/p7`,
  x264 `veryfast/medium/slow`). Quality stays CRF-style (`-cq` / `-global_quality` / `-crf`).
- `*_mf` encoders get real rate arguments or are hidden.
- **Smart render (last, optional, behind its own toggle)**: extend `lossless_segments`
  (`export.rs:611-742`) from all-or-nothing to per-span: stream-copy untouched GOPs, re-encode
  only the spans an edit touches, concat. Only for H.264/HEVC sources whose parameters the
  re-encode can match, and every result is checked with a decode pass
  (`ffmpeg -v error -i out -f null -`); any error falls back to a full encode.

### `export-audio-tracks` (owns `engine/mixer.rs`, a new `engine/stems.rs`, export audio args)
- **Stems = one mix pass per group** on a cloned project with only that group audible (reuses
  `mix_to_wav` unchanged), groups run on parallel threads.
  `// ponytail: N passes decode each source N times; single-pass multi-destination mix_tracks if
  audio time ever matters`. A stem is post-fader through its bus chain, clamped per stem.
- Grouping: by **track** (default, named from `Track.name`), by **bus**, or by **role**
  (`AudioRole`: Dialogue / Music / SFX / Ambience).
- **Streams mode**: stream 0 is the full mix (default disposition), then one stream per group:
  `-i stemN -map N:a -metadata:s:a:N title=<name>`; loudnorm only on the mix
  (`-filter:a:0`). Offered for mp4/mov/mkv; greyed out with a reason elsewhere.
- **Stems mode**: `<output name> stems/NN <name>.wav`, 24-bit 48 kHz, beside the video.
- Export pane: an "Audio" row (Mix / Separate tracks in file / WAV stems + group-by);
  `export.video` gains `audio` and `stem_by` args (ToolDef row updated); size estimate counts
  streams (`estimate_export_bytes`).
- 5.1 fix: sources with more than 2 channels get a proper downmix instead of L/R only
  (`mf.rs:766`; simplest: route those files through the ffmpeg audio decoder, which uses `-ac 2`).

### `interchange` (owns `engine/xmeml.rs`, `engine/import.rs`, a new `engine/edl.rs`)
- FCP7 XML export gains: track names, clip levels/pan, `<link>` for A/V pairs, cross dissolves,
  sequence and clip **markers** (name, note, duration, colour). XML import reads markers back.
- **EDL (CMX3600) export**, one per video track plus audio: the route into **Avid** (and a
  fallback for Premiere/Resolve), paired with the separate-track audio export above.
- Markers export as an Avid marker text file next to the existing CSV / YouTube-chapters options
  (`ui/markers_ui.rs:236`).
- Not doing: AAF (binary structured-storage format; no dependency-free writer) and OTIO. Add OTIO
  when a user needs Resolve round-trips richer than XML.

---

## Wave 3: live co-editing with Claude (or any agent)

Design rule: **everything goes through the MCP server**, so the in-app panel, Claude Desktop,
Claude Code, Codex and any other MCP client get the same abilities.

### `mcp-context` (serial first; owns `mcp/mod.rs`, `ui/app/mcp_exec.rs`, marker tools + drawing)
- **`editor.context`** (Read): one snapshot of what the user is looking at: playhead, in/out,
  selected clips (names, times, tracks), selected markers, cue selection, edit point, selected
  transcript words *with their text*, current sequence and page. Built from `frame::SelSig`
  (`ui/app/frame.rs:21-51`) plus `timeline.selected_marker`, `markers.selected`,
  `TranscriptUiState.sel`.
- **Comments = markers with notes**: `markers.list` returns `note`/`icon`/`done`/`author`;
  new `markers.set` (uses the existing `Project::marker_mut`); `markers.add` fires
  `marker_added`. Timeline draws range markers as spans and shows the note on hover
  (`ui/timeline/mod.rs:1633-1661`, `paint.rs:222-262`). A "Comment" action adds a marker over
  the in/out range or selection and focuses its note. Agents can list open comments and mark
  them `done`.
- **One undo step per request, attributed**: `App::begin_group(label)` / `end_group()` around the
  snapshot logic in `handle_tool` / `run_tool_undoable` (same pattern `run_script` already uses,
  `mcp_exec.rs:27-86`); exposed as `edit.begin{label}` / `edit.end` and a `batch{calls}` tool.
  The undo entry is labelled "Claude: <label>" (`UndoEntry.label`, `mod.rs:491-555`); a group
  left open closes itself after 30 s idle.
- **Visible feedback**: when a group ends, the clips it changed glow on the timeline (reuse the
  existing glow in `ui/app/layout_ctl.rs`) and a toast offers Undo (`feedback.rs:74`).
- **Speed**: `poll_mcp` drains calls for a few ms per frame instead of one per frame
  (`mcp_exec.rs:158-163`); only `render.frame` keeps its own frame.
- **Agents can see**: tool results that are images return MCP image content blocks instead of
  base64 text (`mcp/mod.rs:290-298`); new `render.contact_sheet{from,to,count}` returns a grid of
  frames in one image.
- **Works for every client**: the `initialize` reply carries an `instructions` guide (call
  `editor.context` first, wrap edits in `edit.begin/end`, cut back-to-front, use island
  transcripts for cut points, check results with `render.frame`).
- **External live mode**: `assistant.wait{timeout}` (Job) blocks until the user sends a request
  from the panel or adds a comment, then returns it with the context. An outside agent loops on
  it; no server push needed.
- Fix `.claude/commands/se-coedit.md` and the "Copy command" text to report the port actually in
  use, and pick a free port instead of switching MCP off when the configured one is taken
  (`mcp_exec.rs:130-134`).

### `assistant-pane` (owns a new `ui/assistant_ui.rs`, `ui/app/assistant_ctl.rs`, settings section)
- `Pane::Assistant`: prompt box, context chips (selection / range / comment, attached
  automatically from `editor.context`), a running log of what the agent says and which tools it
  calls, Stop, and "Undo all" per request. Also reachable from Ctrl+K with a prefix. Canned,
  user-editable prompts as buttons (Remove silences, Cut filler words, Caption this, Tighten
  selection, Add chapter markers).
- Backends (Settings ▸ General, next to the MCP section):
  - **Claude Code (default)**: the app spawns
    `claude -p <prompt> --output-format stream-json --verbose --mcp-config <temp file>
    --strict-mcp-config --allowedTools mcp__simple-editor --resume <session>` and reads the JSON
    lines on a worker thread (the `transcript_ctl.rs` job pattern). The agent gets the editor's
    tools only: no shell, no file access. One session id per project so follow-ups keep context.
    All flags confirmed present in the installed `claude` 2.1.316.
  - **Custom command**: a template with `{prompt}` and `{mcp_url}` for Codex or any other agent
    CLI; output shown as plain lines. Ship a Codex preset after checking `codex --help` (Codex is
    not installed on this machine, so its flags are unverified).
  - **API key**: stored in Windows Credential Manager (not settings.json) and passed to the
    spawned CLI as `ANTHROPIC_API_KEY` / `OPENAI_API_KEY`. A CLI-free in-app API client is not in
    this plan; add it if the app must run on machines without any agent CLI.
  - **External**: requests queue for whichever MCP client is waiting on `assistant.wait`.
- Using the panel switches the MCP server on (it is off by default) and wraps each request in one
  undo group.
- Guard rails: tools that leave the project (`export.*`, `project.open/new`, `settings.set`,
  `scripts.run`, `media.convert`) need a click in the app when an agent calls them
  (`ui.confirm_pending` plumbing exists).
- Needs `assert_no_idle_repaint_assistant`, a toggle Action + ToolDef row, a Glyph.

### `voice-input` (owns a new `engine/voice.rs`)
- Hold a key (an unbound chord, checked against `reserved_chords_are_free`) or the panel's mic
  button: record the default microphone with a **cpal input stream** (cpal is already a
  dependency; avoids the ~300 ms ffmpeg start that would clip the first word), write 16 kHz mono
  WAV, run the existing whisper path (`engine/transcribe.rs:212`), put the text in the prompt box.
  Setting: send automatically on release, or leave it editable.
- Fixes it depends on: unique temp WAV per run (`transcribe.rs:222`), and `whisper_dir` /
  model re-read when settings change (`transcribe.rs:40-48`). If whisper is not installed, the
  mic button offers the existing `transcribe.install`.

### `transcribe-islands` (owns `engine/transcribe.rs` options)
- `transcribe.run{islands:true}`: detect speech islands with the existing `autocut.detect`,
  transcribe each island separately, so word times cannot drift across long pauses (the known
  failure that makes whisper timings useless for cut points on long takes). This is what makes
  "cut the bad takes" by voice or text reliable.
- Transcription tools reply with a job id at once and are polled via `jobs.*`, so agent clients
  stop timing out on long clips.

---

## Wave 4: `docs-release`
- `website/` pages for the assistant, voice, separate audio tracks and Avid hand-off; regenerate
  screenshots (`scripts/docs-shots.ps1`) and the shortcuts page (`--dump-hotkeys`).
- ARCHITECTURE.md (threads, export pipeline, assistant), `docs/customizing.md` tool table,
  CHANGELOG, goals.md "Achieved", notes.md with the measured before/after numbers.

---

## Verification

Per PR (agents.md): `cargo test`, `cargo run -- --selftest`, a `--screenshot` render that is
actually looked at for UI changes, `scripts/size.ps1 -Note <ws>`, and the structural tests
(`every_edit_op_has_a_tool`, `assert_no_idle_repaint_*`). Automation windows open on the second
monitor (`SE_BACKGROUND=1` + `window_rect`). Heavy builds and benches run when the user is not
using the machine; benches are only valid on a quiet machine.

End-to-end checks per wave:
- **Wave 1**: HUD + `bench_4k_preview`, `bench_multilayer_preview`, `bench_source_monitor` before
  and after; play the 15-track 1080p60 trailer project from notes.md (must stay at or under its 4
  dropped frames) and a single 1080p60 and 4K60 file in both monitors (0 dropped); scrub test over
  MCP (`playback.scrub`, `playback.status`).
- **Wave 2**: `bench_export` before/after; export the selftest project in all three audio modes
  and check with `ffprobe` (stream count, titles, sample rate) and by importing into Resolve and
  Premiere; a project with a GPU-only effect exported from the Export pane must show the effect;
  cancel mid-export leaves the destination untouched (`full_export_real`); EDL/XML re-imported
  through the app's own importers round-trip clips and markers.
- **Wave 3**: with the app running, connect over MCP (`/se-coedit`) and run a scripted session:
  `editor.context` matches the on-screen selection; a 10-call `batch` is one "Claude: ..." undo
  step; `render.frame` arrives as an image; a comment added in the UI is returned by
  `assistant.wait`. Then the same request typed in the panel and spoken with push-to-talk, checked
  on a screenshot.

---

## Execution: first coding pass (2026-11-10)

All workstreams are coded on **one branch, `feat/pro`, in one worktree**
(`.claude/worktrees/pro`), in batches whose agents own disjoint files. Nothing is compiled, run or
tested in this pass (the maintainer's machine is in use); a read-only review pass follows, and the
first `cargo test` happens later, on request. Issue-ready bodies are in `issues/<workstream>.md`.

| Batch | Workstreams | Owns (edit nothing outside your row except your own marker lines) |
|---|---|---|
| 0 (serial) | `pro-schema` | shared tables: `src/ui/app/mod.rs` registries + `App`, `src/hotkeys.rs`, `src/ui/tools.rs`, `src/ui/layout.rs`, `src/settings.rs`, `src/model/marker.rs`, `ExportOptions` in `src/engine/export.rs` |
| 0 (serial) | `perf-baseline` | new `src/ui/hud.rs`; timers in `src/ui/app/mod.rs` (`update`), `src/engine/gpu.rs`, `src/playback.rs`; `src/ui/app/tools_playback.rs`; benches; `goals.md`, `notes.md` |
| A | `play-source-monitor` | `src/ui/app/source_pane.rs`, `src/ui/source_ui.rs`, `configure_player` + source canvas texture in `src/ui/app/gpu.rs`, second canvas target in `src/engine/gpu.rs`, the `set_proxies` call in `src/ui/app/jobs.rs` |
| A | `play-decode-path` | `src/media/mf.rs`, `src/media/mod.rs`, `upload` in `src/engine/gpu.rs`, layer sampling in `src/engine/shaders.rs`, lazy BGRX conversion where `src/engine/compose.rs` reads a frame |
| A | `play-render-thread` | `src/playback.rs` |
| A | `export-pipeline` | `run_export` / `mix_to_wav` / `GpuScratch` in `src/engine/export.rs`, `serve_gpu_exports` in `src/ui/app/gpu.rs`, `render_frame` + readback in `src/engine/gpu.rs`, the `frames:` line in `src/ui/export_ui.rs` |
| A | `interchange` | `src/engine/xmeml.rs`, `src/engine/import.rs`, new `src/engine/edl.rs`, new `src/ui/app/tools_interchange.rs`, marker export options in `src/ui/markers_ui.rs` |
| A | `mcp-context` | `src/mcp/*`, `src/ui/app/mcp_exec.rs`, new `src/ui/app/tools_context.rs`, marker tools in `src/ui/app/tools_subtitles.rs`, `src/model/ops/markers.rs`, marker drawing in `src/ui/timeline/mod.rs` + `paint.rs`, `.claude/commands/se-coedit.md` |
| A | `transcribe-islands` | `src/engine/transcribe.rs`, `src/ui/app/tools_transcript.rs`, `src/ui/app/transcript_ctl.rs` |
| B | `export-encoders` | new `src/engine/encoders.rs`, `codec_args` / `detect_encoders` / `lossless_segments` in `src/engine/export.rs`, the Advanced grid in `src/ui/export_ui.rs`, `encoder_options` in `src/ui/mod.rs` |
| B | `export-audio-tracks` | new `src/engine/stems.rs`, `src/engine/mixer.rs`, audio inputs/maps in `run_export`, `estimate_export_bytes`, the Audio row in `src/ui/export_ui.rs`, `export.video` args in `src/ui/app/tools_export.rs`, the >2-channel downmix in `src/media/mf.rs` |
| B | `assistant-pane` | new `src/ui/assistant_ui.rs`, `src/ui/app/assistant_ctl.rs`, `src/engine/agent.rs`, `src/ui/settings_ui/assistant.rs` (never `general.rs`: it has uncommitted maintainer changes on `main`) |
| B | `voice-input` | new `src/engine/voice.rs`, `src/ui/app/voice_ctl.rs`; the temp-WAV and settings re-read fixes in `src/engine/transcribe.rs` |
| C | `docs-release` | `website/`, `ARCHITECTURE.md`, `docs/customizing.md`, `CHANGELOG.md`, `goals.md`, `notes.md` (no screenshot regeneration in this pass) |

Left out of this pass on purpose, because the plan gates them on measurements that have not been
taken: `play-decode-path` step 3 (NV12 decode) and step 5 (`se-engine` at `opt-level=3`).
Smart render is coded but ships switched off.

### Rules for every coding agent in this pass

- Work only inside `D:\Projects\simple-editor\.claude\worktrees\pro` (absolute paths). Never edit
  `D:\Projects\simple-editor\src` (the maintainer's live checkout).
- Do **not** run `cargo`, `rustc`, `clippy`, `rustfmt`, the app, tests, or `ffmpeg` encodes. Read
  and edit only. Do not commit, push, or switch branches; the orchestrator commits per batch.
- Other agents are editing this tree at the same time. Stay in your row's files; in shared tables
  replace only your own `// ---- ws:<name> ----` marker line. If an Edit fails because the file
  changed, re-read and retry; never overwrite a whole shared file with Write.
- Since nothing compiles, be strict about not inventing APIs: before calling a function, method,
  field or crate API, find its definition or an existing call site (crate sources are under
  `C:\Users\Kash\.cargo\registry\src\`). Copy the shape of neighbouring code.
- Keep the repo's rules: no new crates; menus through `ui::menu::*`; every new
  `pub fn ...(&mut self)` on `Project`, `Action`, or `Pane` gets its `ToolDef` row; every new
  pane/window gets an `assert_no_idle_repaint_<context>` test; non-trivial logic leaves one
  `#[cfg(test)]` test; deliberate shortcuts get a `// ponytail:` comment naming the ceiling.
- Finish with a short report: files changed, anything in the issue you did not do and why, and
  every place you were unsure the code compiles.
