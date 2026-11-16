# Peak performance plan: playback that feels like Resolve

## Goal

Real projects should play at full frame rate with no drops, start instantly and never stall. A real project
here means nested sequences, 1440p sources, PNG-codec overlays and 9-minute timelines. The
reference is DaVinci Resolve / Premiere on the same machine (RTX 2060 SUPER, 64 GB RAM).

The [playback-never-stutters rule](../../goals.md) still holds: no heavy compute on the playback
path. Work is prepared ahead of time, playback falls back gracefully, and Play is gated on readiness.

## Evidence (measured 2026-11-15, isolated scratch instance, nothing in the user's project touched)

| Test | Result |
|---|---|
| Fresh project, 1 layer 1080p60 h264 / 1440p60 120 Mbit / 3 layers incl. PNG `.mov` | 0-1 dropped of ~480, UI p95 2-4 ms: **the player core is fine** |
| Copy of the real project (`rodev-gta-6-gameswap`), t = 14 s, nested sequence | still buffering after 6 s, 38 of 360 frames, UI p95 83 ms |
| Same, t = 60 s, 2560x1440 120 Mbit clips + Crop | 149 dropped / 6 s, decode worst 47 ms, worst gap 2.3 s |
| Same, t = 130 / 185 s, nested sequences (2 and 4 tracks) | 23 / 47 dropped, gaps 0.8-1.2 s |
| Live session counters | 5412 dropped of 58,706, worst gap **13.8 s**, UI p95 **37 ms while paused**, 7 GB RSS, 310 threads |
| Raw ffmpeg decode, `the-devs-silent.mp4` (1440p60, 120 Mbit) | **52 fps**: slower than real time in software |
| Cache dir | 37.5 GB (html 22.2, prerender 8.8, proxies 6.3); C: had 23 GB free |
| Opening the project | queues ~77 proxy builds + HTML renders + thumbnails, all at normal priority |

Caveat: the real-project runs shared the machine with that proxy queue. Wave 0 re-measures on a quiet
machine before anything is optimised.

## What the code does today (read, not guessed)

| # | Finding | Where |
|---|---|---|
| F1 | A nested sequence on screen costs a **whole `Project` clone plus a full CPU composite of the inner timeline every frame**. Its layers decode serially and its effects run on the CPU. | `playback.rs:2065-2100` (`ponytail:` note at 2086) |
| F2 | Hardware decode (DXVA) **copies every frame back to system memory as RGB32**, then the frame is re-uploaded to GL. That copy-back is why DXVA is only enabled at 1440p and above (1080p: 42 ms hardware vs 7 ms software). | `media/mf.rs:109-141, 383-401` |
| F3 | One render thread does pacing, decode scheduling and compositing, with a fixed 1.5 s read-ahead. Upcoming cuts are not pre-opened or pre-seeked, so a cut into a deep long-GOP spot stalls. | `playback.rs:449, 77` |
| F4 | Nothing sets thread priority anywhere. Proxy, HTML, thumbnail, prerender and transcribe workers compete with playback as equals. | `grep SetThreadPriority` finds nothing |
| F5 | The HTML cache has **no budget**: 22 GB and growing. Prerender is capped at 8 GB, but frames are raw RGBA, about 8.3 MB per 1080p frame, or ~500 MB/s at 60 fps. Serving it is disk-bound. | `engine/html.rs:2486`, `engine/prerender.rs:39` |
| F6 | Prerender invalidation: any nested-sequence edit rehashes wholesale. | `engine/prerender.rs:506` |
| F7 | The playback RAM cache auto-sizes to RAM/4, capped at 4 GB. RSS is still 7 GB, so decoder pools, spares (`KEEP = 16` frames) and stills are unaccounted for. | `playback.rs:68-76, 2167` |
| F8 | The UI frame is 8.7 ms average and 37 ms p95 **while paused** on a 545 s / 135-asset project. The timeline, waveforms and thumbnails redraw the whole visible range every frame. | live HUD; needs a profile (W0.2) |

## Budgets: the definition of "peak"

Reference machine: RTX 2060 SUPER, 64 GB. Reference project: a frozen copy of the real project, plus a
synthetic stress project (see W0.1).

| Metric | Today (real project) | Target |
|---|---|---|
| Dropped frames, full timeline play at 1x | thousands | **0** |
| Worst frame gap | 13.8 s | **< 50 ms** |
| Play press to first moving frame (cached) / (cold) | 0.15-1.9 s | **< 100 ms / < 400 ms** |
| Seek to a sharp frame | 0.07-2.0 s | **< 100 ms** (proxy) / < 250 ms (original) |
| UI frame p95, paused / playing | 37 / 83 ms | **< 8 ms / < 8 ms** |
| Idle CPU (paused, no jobs) | ~1 % | ~0 % (keep) |
| RSS | 7 GB | **< 3 GB** on the reference project |
| Cache on disk | 37.5 GB, unbounded | one total budget, default 30 GB, LRU |
| Exe size | 13.9 MB | ≤ +300 KB for the whole plan, no new crates |

## Workstreams

Issues (label `perf`): tracker #168. W0.1 #155, W0.2 #156, W1.1 #157, W1.2 #158, W1.3 #159, W1.4 #160,
W2.1 #161, W2.2 #162, W2.3 #163, W2.4 #164, W3.1 #165, W3.2 #166, W3.3 #167.

The workstreams follow the concurrent worktree protocol in `agents.md`. Wave 0 runs serially. Later waves
run in parallel only where their files don't overlap.

### Wave 0: measure first (serial)

**W0.1 `perf-bench`: a repeatable benchmark that can't touch real projects.**
- Add `simple-editor --bench <project.sedit> [--regions 14,60,130,185] [--secs 6]`. It loads a **copy** in
  a scratch profile (`APPDATA` and `LOCALAPPDATA` redirected) with background jobs off, then plays
  each region and writes CSV: dropped, published, worst gap, play-start, seek ms, decode avg and worst, UI p95, RSS.
- Add a synthetic stress project generator under `tests/bench/`, built with ffmpeg testsrc. It covers 1080p60 h264,
  1440p60 120 Mbit, a PNG `.mov` overlay, a 3-deep nested sequence and a 9-minute long-GOP file with a
  clip 300 s in.
- Add a CI-optional `cargo test --release bench_* -- --ignored` that asserts the budgets above on the
  synthetic project.

**W0.2 `perf-trace`: know where every millisecond goes.**
- Add zero-cost spans (an `AtomicU64` ring, no new crate) around decode, convert, upload, composite,
  nested render, UI timeline draw, waveform draw and the inspector.
- Add `playback.trace {secs}` (MCP), which dumps a Chrome-trace JSON.
- Profile F8, the paused UI cost, before W2.3 picks a fix.

### Wave 1: the big wins (parallel where noted)

**W1.1 `nested-gpu`: nested sequences become GPU layer trees (fixes F1).** It owns `playback.rs` layer building and `engine/gpu.rs`.
- `LayerSet` gains `Nested { id, size, layers: LayerSet }`. `GpuRenderer` composites the child set into a
  pooled FBO with the same shaders, then draws it as one layer. No `Project` clone and no CPU composite.
- Inner layers go through the same parallel `decode_layers`.
- The CPU path stays for export fallback and tests. Add a parity test: GPU nested equals CPU nested within 2 grey levels.
- Expected result: the 14 s and 185 s regions go from buffering to 0 drops.

**W1.2 `zero-copy-decode`: hardware frames stay on the GPU (fixes F2).** It owns `media/mf.rs` plus a new `engine/gpu/interop.rs`.
- Output NV12 D3D11 textures from the source reader (no RGB32, no copy-back).
- Share them to GL with `WGL_NV_DX_interop2`, which is supported on NVIDIA, AMD and Intel and loaded through the
  existing `windows` OpenGL feature. Convert NV12 (and P010 for HDR) to RGB in the layer shader.
- Then drop `DXVA_MIN_PIXELS` to 0: every h264, HEVC, VP9 and AV1 source decodes in hardware.
- Fall back to today's copy path per source on any interop failure, and log once.
- Expected result: 1440p 120 Mbit decodes ≥ 240 fps and CPU use while playing drops sharply.

**W1.3 `cut-preroll`: decode ahead of the playhead by timeline, not by clock (fixes F3).** It owns `media/mod.rs` (`DecoderPool`).
- Give each source a decoder thread with its own bounded frame queue (about 0.5 s).
- A scheduler walks the timeline up to 4 s ahead. It opens and seeks the decoder for every clip that starts in that window,
  so the first frame after a cut is already decoded when the cut arrives. Long-GOP seeks happen early, off the clock.
- Raise read-ahead adaptively: 1.5 s, extended up to 4 s when RAM allows and the frames ahead are expensive.

**W1.4 `job-priority`: playback always wins (fixes F4).** It owns `ui/app/jobs.rs`, `media/proxy.rs`, `engine/html.rs` workers and `prerender.rs`.
- Run every background worker at `THREAD_PRIORITY_BELOW_NORMAL` with `PROCESS_MODE_BACKGROUND_BEGIN` I/O priority.
  Spawn ffmpeg children with `BELOW_NORMAL_PRIORITY_CLASS`.
- While playing, pause proxy, HTML, thumbnail and transcribe work through a shared `PlayGate` flag, and resume on pause.
- Order the proxy queue by distance from the playhead, nearest first. Today it uses import order.

### Wave 2: make heavy media cheap (parallel)

**W2.1 `smart-proxy`: auto-proxy only what needs it, and build it fast.**
- Classify sources at import: proxy when decode cost exceeds the budget. That covers > 1080p, > 50 Mbit, intra-heavy codecs
  (PNG, QuickTime Animation, uncompressed) and anything MF can't decode in hardware. A 1080p 10 Mbit OBS file gets no proxy.
- Build proxies with NVENC, QSV or AMF through ffmpeg.exe (already present) as all-intra h264 at 1080p, 60 fps preserved. Keep alpha
  sources as alpha (ProRes 4444 or QTRLE); no new dependency.
- Add a proxy toggle per source and a project-wide "use proxies while playing" mode (Resolve's "Use Optimized Media if available").

**W2.2 `cache-budget`: one disk budget, LRU across all caches (fixes F5 and F6).** It owns `ui/app/caches.rs`.
- `Settings::cache_disk_gb` (default 30) covers html, prerender, proxies, thumbs and peaks. LRU by last use, pruned
  at startup and hourly on the idle thread. Proxies for sources used by the open project are evicted last.
- Store prerender as all-intra h264 or HEVC via NVENC instead of raw RGBA: 50-100x smaller, decoded by W1.2's zero-copy path.
- Hash prerender per sequence, so editing one nested sequence invalidates only its spans.
- Warn when free disk on the cache drive drops below 2x budget.

**W2.3 `ui-frame`: the editor UI stays under 8 ms (fixes F8).** It owns `ui/timeline/*`, `ui/inspector.rs` and waveform drawing.
- Fix whatever W0.2 shows. Likely candidates:
  - cache the timeline's clip, waveform and thumbnail meshes per zoom level and only rebuild on edit or zoom;
  - cull off-screen clips and sequence contents;
  - downsample waveform peaks to the pixel width once instead of per frame;
  - repaint only the playhead layer while playing.
- Each fix gets an `assert_no_idle_repaint_*` test plus a timed test on a 135-asset, 545 s generated project.

**W2.4 `memory-diet`: RSS under 3 GB (fixes F7).**
- Account every buffer against one budget: decoder pools, the spare pool (bytes instead of count), stills and the GPU upload staging.
- Close decoders idle for over 10 s that are not in the pre-roll window. Cap concurrent ffmpeg children.
- Audit thread count. 310 threads means per-source or per-job threads are never joined.

### Wave 3: graceful under any load

**W3.1 `adaptive-quality`: drop resolution before frames.**
- Add playback resolution Full, 1/2, 1/4 and Auto. Auto watches decode plus composite time and steps down before the
  queue drains, then steps back up when it can. Effects too heavy for real time (camera shake, chroma key on 4K) run at reduced
  resolution while playing and full on pause. That matches the Resolve and Premiere behaviour.

**W3.2 `smart-cache`: render the expensive spans when idle.**
- When paused and idle for 3 s, render spans whose measured frame cost exceeds the budget into the W2.2 prerender cache:
  nested sequences, stacked effects, HTML layers. They play from it next time. The timeline shows a red, yellow or blue
  bar, as in Resolve.

**W3.3 `perf-gate`: never regress.**
- Add W0.1's synthetic bench to `/se-verify` and record the results in a `perf_log.csv` next to `size_log.csv`. Over 10 % worse
  on any budget fails verification.

## Order and expected impact

| Order | Workstream | Fixes | Expected effect on the real project |
|---|---|---|---|
| 1 | W0.1, W0.2 | measurement | a trustworthy before and after |
| 2 | W1.1 nested-gpu | F1 | nested regions: buffering to ~0 drops |
| 2 | W1.4 job-priority | F4 | no stalls while proxies build |
| 3 | W1.2 zero-copy-decode | F2 | 1440p / 4K real time with low CPU |
| 3 | W1.3 cut-preroll | F3 | no stall at cuts or deep seeks; worst gap < 50 ms |
| 4 | W2.1-W2.4 | F5-F8 | snappy UI, bounded disk and RAM |
| 5 | W3.x | robustness | holds 60 fps on any project, never regresses |

W1.1 and W1.4 touch separate files and can run concurrently. W1.2 and W1.3 both touch the decode path, so they run one
after the other.

## Risks and decisions for the maintainer

- **WGL_NV_DX_interop2** is an extension, not core GL. The plan keeps today's copy path as a per-source fallback, and any driver
  without it behaves exactly as now. The alternative (moving the renderer to D3D11) is out of scope because it would mean a new egui backend.
- **Prerender as h264** loses bit-exactness against export. At high bitrate all-intra the difference is invisible, but the
  maintainer should confirm that preview-only cache is acceptable.
- **Proxy defaults change** (fewer proxies built automatically). Existing proxies stay valid.
- No new crates are planned, and every piece uses `windows`, `glow` or ffmpeg.exe, which the app already has.

## Working notes

- Never benchmark on a user's live project. Use `--bench` on a copy in a scratch profile, and kill test instances by PID, never by image name.
- Every workstream re-runs W0.1 and puts the before and after numbers in its PR body, next to the `size:` line.
