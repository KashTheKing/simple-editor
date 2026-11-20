# Handoff: parity-io (`claude/parity-io`)

Branch from `origin/develop` (41c9b73). Section "parity-io" of `plans/cloud-handoff.md`, item 11 of
`plans/shotcut-comparison.md`. Screen Snapshot (parity-webcam) and Rive (parity-formats) are other sections and
are not here.

## What's done

- **Media Properties** (`engine::media_info`, `ui::io_ui::props_window`, `ui::app::io_ctl`): Library item
  right-click ▸ **Properties…**, the Info window's **Properties…** button, or `Action::MediaProperties`
  (palette; uses the Library anchor, else the first selected clip). Container (format, duration, size, bitrate,
  tags) and every stream (codec + profile, size, fps, pixel format, colour primaries / transfer / matrix /
  range, rotation, field order, frames, sample rate, channels + layout, language, title, default). **Copy**
  copies it as text. ffprobe runs on a worker thread; the window never blocks.
- **Bitrate graph**: **Show Graph** (Video / Audio) demuxes once on a worker (`ffprobe -show_entries
  packet=pts_time,dts_time,size,flags`), 1 s buckets, average line, keyframe ticks, hover readout. Results are
  cached per (file, stream) for the session.
- **Rotation / range overrides** (`media::io_override`, `Project::media_io`, `model/ops/media_io.rs`): Media
  Properties ▸ Decode ▸ Rotation (From file / 0 / 90 / 180 / 270) and Range (As tagged / Limited / Full). One undo
  step; stored by source path (`media_io` in the .sedit, omitted when empty). A quarter turn swaps the asset's
  width/height. `io_ctl::tick` mirrors the map into a process global the decoders read (`io_override::sync`) and
  re-decodes only the changed sources (a proxy-map round trip: `DecoderPool::set_proxies` drops their decoders
  and evicts their spans). A source with an override: always the ffmpeg pipe (`-noautorotate` + `transpose` /
  `scale=in_range=…`), never zero-copy, never its proxy; `ffpipe::probe` reports the overridden display size.
- **Full-range export**: Settings ▸ Export ▸ Levels ▸ **Full range** (`export_full_range`). `export::full_range`
  rewrites `rgba_bt709`'s limited conversion + tags to `pc`, or expands the GPU's NV12 with
  `scale=in_range=tv:out_range=pc`. SDR 4:2:0 outputs only (HDR, GIF, ProRes untouched). The flag reaches the
  export through a static set every frame by `io_ctl::tick` (no `ExportOptions` field, which has 27 literals).
- **Network streams** (`engine::netstream`, `ui::io_ui::stream_window`): File ▸ Import ▸ **Open Network
  Stream…** (`Action::OpenStream`, toggles). http(s)/HLS/DASH, rtmp(s), rtsp(s), srt, udp, rtp, tcp, mms.
  Probe (live or duration, codecs), a live preview (ffmpeg child, 12 fps, 640×360 RGBA; the UI only uploads the
  newest frame), **Record to Library** (stream copy to `<capture folder>\Streams\<host> <unix time>.mkv`,
  optional **Stop after** N minutes, Stop = ffmpeg's `q`), imported through `import_recording` when done. RTSP
  uses TCP, HTTP reconnects, 15 s I/O timeout. Recent addresses (10) in `Settings.stream_urls`. Both ffmpeg
  children join the app's kill-on-close job (`html::tie_to_us`). A recording with no media (stopped during the
  first seconds' probe of a live source) is reported, not imported.
  Not Shotcut's full "play a stream as a clip": the stream is recorded, then edited. The preview has no sound.
- **Search in recents**: Home page Projects header gets a Search box (with 2+ projects); File ▸ Open Recent gets
  one with more than 6. `home_page::matches_recent` (every word, any case, name or folder).
- **GPU choice** (`engine::gpu_pick`): Settings ▸ Performance ▸ **GPU**: Automatic / Power saving / High
  performance / each DXGI adapter by name (software adapters left out). On change: writes Windows' per-app
  `HKCU\Software\Microsoft\DirectX\UserGpuPreferences\<exe> = "GpuPreference=N;"` (removed for Automatic; a
  named card maps to whichever of 1 / 2 selects it). At start (`App::build`, before any decoder) the MF decode
  device is created on the chosen adapter (`decode_adapter`), falling back to the default device if that fails.
  Applies after a restart (toast says so).
- **Contour ShuttlePRO / v2 / ShuttleXpress** (`engine::shuttle`): a reader thread lists HID interfaces with
  `CM_Get_Device_Interface_ListW` (cfgmgr32 via `windows::core::link!`, no new crate features), opens VID 0x0B33
  with `std::fs::File`, blocks in `read`; with no device it looks again every 3 s. Report decoding with wrap-aware
  jog. Ring = `play_gated(rate)` on a ¼× … 16× ladder, centre = Stop; wheel = StepForward / StepBack per detent;
  buttons = `Settings.shuttle_buttons` Action ids (defaults in `shuttle::default_buttons`). Settings ▸ Hotkeys ▸
  **Jog / Shuttle** (on/off, connected model, a combo per button, Reset Buttons). On by default.
- **MCP**: `media.properties`, `media.bitrate` (pending → data), `media.set_io` (Mutate, `OP_TOOLS` row for
  `set_media_io`), `stream.open` and `stream.record` (both wait for the Assistant guard's Allow:
  `leaves_project`), `gpu.list`, `shuttle.status`; `settings.set/get` take `gpu_adapter`, `export_full_range`,
  `shuttle` (the last two also in the palette's toggles).
- Registries: `// ---- ws:parity-io ----` at the end of TOOL_TABLES / ACT_HANDLERS / FRAME_HOOKS /
  WINDOW_DRAWERS, `App.io`, `Project.media_io`, the Settings block, the Actions block and group arm.
- Docs: CHANGELOG (Unreleased ▸ I/O and monitoring), `docs/customizing.md` rows, new guide page
  `website/docs/streams-and-devices.mdx` (sidebar after Screen recording).

## Verified here (Linux container)

- `cargo check --target x86_64-pc-windows-gnu --all-targets`: clean; no warnings from the new files (the crate's
  ~300 warnings are pre-existing).
- The Windows test binary builds and links (`cargo test --target x86_64-pc-windows-gnu --no-run`), but it can't
  run here: Wine in this container aborts even on a hello-world (`free(): invalid pointer`).
- **`tools/linux-harness/`** (new, its own `[workspace]`): compiles `media_info`, `netstream`, `shuttle` and
  `io_override` straight from `src/` with stubs for the Windows bits and runs their unit tests (16 pass), plus
  `tests/real.rs` against the real ffmpeg / ffprobe 6.x here (6 pass):
  - probe of a 90°-flagged, full-range, H.264 + AAC MP4 (coded size, rotation 270 cw, range `pc`, 25 fps, audio);
  - bitrate buckets and one keyframe per second for `-g 25`;
  - the exact override filter chains decode at the expected sizes (upright 320×180, 90 / 270 → 180×320);
  - range: a Y=16 full-range grey reads 16 as Full, 0 as Limited;
  - both full-range export chains produce files ffprobe reports as `pc`;
  - HTTP recording with a 2 s limit (2.10 s, video + audio copied), the live preview (640×360 frames),
    a live UDP recording stopped with `q` (7.96 s, playable), and Stop-during-probe → "nothing was received".
  Run: `cd tools/linux-harness && cargo test -- --test-threads 1 --skip overridden_source_decodes`.

## Untested (needs Windows)

- Everything drawn: the two windows, the Settings rows, the Library menu item / Info button, the menu entry,
  the recents search boxes. Their idle-repaint tests (`assert_no_idle_repaint_media_properties`,
  `_network_stream`, `_shuttle_settings`) and the other new unit tests compile but were not run.
- `io_override::tests::overridden_source_decodes_through_ffmpeg_turned` (real `media::open_video` with an
  override, needs ffmpeg on PATH), `model::ops::media_io` test, `export::full_range_export_filters_and_tags`,
  `io_ctl` tests (tool registration, guard), `tools_registry_tests` (new `OP_FILES` / `OP_TOOLS` rows).
- The override re-decode in the live preview (proxy-map round trip) and undo / redo of an override.
- GPU: DXGI enumeration, the registry write, `decode_adapter` and zero-copy on a two-GPU machine. If the decode
  device lands on another GPU than the GL window, zero-copy interop should switch itself off (existing fallback);
  worth a look on a laptop.
- ShuttlePRO: no device here. The report layout (shuttle i8, jog u8 counter, buttons in bytes 3-4, Windows'
  report-id 0 first) is the documented one; check button numbering on a real unit, and whether
  `File::read` with a 64-byte buffer is accepted by the driver (it should be: ≥ InputReportByteLength).
- Media Foundation-only machines (no ffprobe): Properties says ffprobe is missing; overrides need ffmpeg.

## Risks / notes for the merge

- `src/media/ffpipe.rs` probe: the rotation swap became `io.display_size(...)` (same result without an
  override). `VideoPipe::respawn` builds `-vf` from tone map + override and inserts `-noautorotate` before `-i`.
- `src/media/mod.rs`: `open_known` / `open_video` / `DecoderPool::resolved` each gained one override check.
- `src/media/mf.rs`: the D3D11 device creation became a closure with adapter + fallback.
- Library thumbnails and waveforms don't follow a rotation override (they have their own decode paths).
- The Shuttle reader thread runs whenever Settings `shuttle` is on (default), headless test Apps included:
  one cfgmgr32 call every 3 s, no device opened unless one with VID 0x0B33 is present.
- Size: not measured (no Windows release build here): run `scripts/size.ps1 -Note parity-io`.
