# Hand-off: forum-preview (`claude/forum-preview-perf`)

Session D of `plans/shotcut-forum-insights.md`: preview performance and phone footage. Branched from
`origin/develop` at `a3a0538`. Cloud session, Linux container: the app was never run.

## What's done

| Scope item | Where | State |
|---|---|---|
| 1. Import probe: VFR | `media/playcheck.rs` (`traits`, `vfr_from_pts`, `vfr_from_rates`), started from `ffpipe::probe` (`playcheck::announce`) | Done. One small ffprobe of the first 20 s of packet timestamps, on its own thread, once per path per session. |
| 1. VFR → constant-rate proxy, tell once | `media/proxy.rs` (`wants`, `eligible`, `cfr_rate`, `cfr_args`, name hash), `ui/app/health_ctl.rs` (`announce_vfr`) | Done. VFR sources want a proxy; it's built with `-fps_mode cfr -r <nominal rate>` under its own name. Toast "Fixed phone footage timing: <file>" once, only for files imported this session and only with Proxies on. |
| 1. HDR, rotation | already on develop | Not repeated: `ffpipe::probe` swaps rotated sizes, `project_match` offers an HDR project. |
| 1. Codec cost | `playcheck::cost_score`, `HEAVY`, `proxy_bytes`, `fmt_bytes` | Done (rule of thumb vs 1080p30 H.264). |
| 2. Playback health chip | `ui/health_chip.rs` (draw), `health_ctl::chip` (called at the end of `preview_pane::draw`) | Done. Green dot while playing; amber **Choppy** chip with **Make proxy** or **Lower resolution** and **✕**. Choppy toggles the Performance pane and is lit while it's open. |
| 3. Proxy proposal on a failed decode-speed test, with size | `playcheck::Meter`, `playcheck::advise`, `health_ctl::tick` | Done. The test is playback itself: on Amber the Performance sampling turns on for 15 s (`perf.report_until`), so `perf::source_stat` measures each source; `advise` names the slowest unsettled source (measured > 60 % of a frame, or unmeasured and heavy by score) and the tooltip gives the proxy's estimated size. The fix goes through `perf_ctl::make_proxy` (extracted from the Performance pane's Fix, so both use one path). |
| 4. Bench | `media/playcheck_bench.rs` (`bench_forum_clips`, ignored), `health_ctl::tests::bench_undo_three_hour_timeline` (ignored) | Written, not run (see below). |
| 5. Stale-state class (forum 50776) | `playback.rs` (`Applied`, `Player::applied`, `published_frames`), `health_ctl::reconcile` | Done. Each player records the backend / GPU mode / cache budget it was given; a FRAME_HOOK compares that with the settings every frame and pushes only what differs. Also fixes a real gap: a decoder change in Settings never reached an open Source monitor. |
| MCP | `playback.health` (`health_ctl::TOOLS`) | Done; row added to `docs/customizing.md`. |
| Docs | `website/docs/viewer.mdx` ("When playback is choppy", "Phone and screen recordings"), `CHANGELOG.md` Unreleased | Done. |

Registry entries (my own marker lines, appended at the end of each table): `TOOL_TABLES` and
`FRAME_HOOKS` (`// ---- ws:forum-preview ----`), `App.health` field + init, `mod health_ctl`, `pub mod
playcheck`, `pub mod health_chip`.

## Hooking into mixed-footage (`claude/feat-mixed-footage-83d1e8`)

mixed-footage is not on develop, so nothing here calls it. When both are merged:

- Its import card (`footage_ctl::footage_landed` / `heavy`) stays THE import-time proxy offer; this branch
  adds no import card. Suggested one-line change in its `heavy()`: also count
  `crate::media::playcheck::cost_score(&a.codec, a.width, a.height, a.fps, 0.0, ten_bit) >= playcheck::HEAVY`,
  and its card can show `playcheck::fmt_bytes(playcheck::proxy_bytes(..))` per clip.
- Its `make_proxies(ids)` and this branch's `perf_ctl::make_proxy(id)` do the same thing (`asset.proxy =
  Some(true)`, Proxies on, an undo step). If you want one path, make `make_proxies` loop over
  `perf_ctl::make_proxy` or the reverse; they don't conflict as they are.
- VFR sources: `proxy::wants` returns true for them, so mixed-footage's `!(auto && needs_proxy(a))` filter
  still offers them in its card when they're also heavy. Harmless (the answer is the same proxy).

## Verified here

- `cargo check --target x86_64-pc-windows-gnu --all-targets`: clean (no new warnings from these files).
- `media/playcheck.rs` unit tests (9) run on Linux in a scratch crate with the module included by
  `#[path]` and two stub modules: all pass. They found two real problems, fixed: the test's "VFR" rates
  (30 vs 29.99) weren't VFR, and the meter's threshold was stricter than its doc. VFR clips now conform
  at their nominal rate (30), not a sagging average (26).
- With the Linux ffmpeg 6/ffprobe: a synthetic VFR clip (every 4th frame missing, timestamps kept) and a
  synthetic VFR screen recording are both found VFR by the real `playcheck::probe`; a CFR file isn't.
  A proxy built with `proxy::ENCODE` + `-fps_mode cfr -r 30` is constant 30 fps and shows the same source
  frame as the original at 20 random cut times (raw Y plane compared).
- The bench's ffmpeg recipes (4K60 10-bit HEVC, VFR screen recording) run with this ffmpeg.

## Not verified (needs the Windows box)

- **No Rust test from this branch has run.** The crate only builds for Windows, and Wine aborts in this
  container (`free(): invalid pointer` even for `wine cmd`). Run:
  `cargo test playcheck`, `cargo test health`, `cargo test perf_ctl`, then the full suite.
- `playcheck::tests::vfr_proxy_shows_the_original_frame_at_20_cuts`: the same sync check through the app's
  own decoders (MF / ffpipe). Needs ffmpeg + ffprobe on PATH or in the app's ffmpeg dir.
- `health_chip::tests::amber_chip_buttons_answer` probes clicks along the chip's row at y = 36; if the
  theme's popup frame or button height differs, adjust the probe line.
- The chip itself: no screenshot was taken. Check it over a playing 4K HEVC clip (Playback ▸ Proxies on
  and off), that **Make proxy** queues the right file and the chip clears after ~8 s of clean play, and
  that it doesn't sit on top of the multicam Angles list (that's top-right) or the social guides.
- Benches: `cargo test bench_forum_clips -- --ignored --nocapture` (makes a 3-hour 640x360 file once in
  `%TEMP%\se-forum-bench`, a few minutes) and `cargo test bench_undo_three_hour_timeline -- --ignored
  --nocapture`. Budgets: proxy seek / scrub ≤ 50 ms, first frame ≤ 150 ms, undo ≤ 200 ms. Memory growth on
  a 3-hour timeline isn't benched; `scripts/perf-bench.ps1 -Project` on a long project covers it.
- "Plays without dropped frames after the proxy is accepted" and "VFR stays in sync across 20 random cuts"
  in the live app: run `perf.bench` over MCP on the three bench clips with their proxies built.
- Size: not measured (release step only). Expect a few KB: no new dependency.

## Risks

- `proxy::eligible` now also waits for the VFR probe, so a NEW proxy build starts one probe later
  (~10-50 ms a file, serial). Existing proxies are used straight away (`usable` is unchanged).
- A VFR source that already had a plain proxy gets a second, constant-rate one built once (new name).
  The disk-cache pruner treats the old one as unreferenced.
- `reconcile` sends `set_gpu(false)` to the timeline player before the GPU renderer starts. The render
  thread ignores a Gpu command that doesn't change anything, so nothing is cleared.
- VFR detection reads the first 20 s only; a file that turns VFR later is treated as constant-rate
  (same as before this branch).
- The chip samples the counters twice a second and only on frames that are drawn anyway; it asks for no
  repaint (`assert_no_idle_repaint_health_chip`).
