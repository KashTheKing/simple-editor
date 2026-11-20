# parity-webcam: `claude/parity-webcam` (new from `origin/develop` @ 41c9b73)

Section "parity-webcam" of `plans/cloud-handoff.md`: webcam capture next to Screen Recording, an optional
webcam bubble while screen recording, and Screen Snapshot.

## What's done

- **Webcam source** (`src/engine/webcam.rs`, `CaptureSource::Webcam(CamSpec)` in `src/engine/capture.rs`).
  "Webcam" is a source Type in the Screen Recording pane, not a separate recorder. It goes through the same
  crash-safe pipeline as a screen: MKV pieces, WASAPI mic/system taps, `finalize` mux + probe, then the Library bin.
  The input is `ffmpeg -f dshow -use_wallclock_as_timestamps 1 [-vcodec mjpeg | -pixel_format X]
  -video_size WxH -framerate F -i video=<name>`. Wall-clock stamps make ffmpeg's `start:` line usable by the
  existing audio alignment (`start_line_us` needs > 1e9).
  - Devices: `ffmpeg -list_devices` parsed by `capture::parse_dshow(stderr, video)` (shared with the audio
    parser, now tri-state so `(none)` devices are skipped). Formats: `-list_options`, parsed by
    `webcam::parse_formats` (one entry per kind+size at its top fps, largest first). Both probed off the UI
    thread (`devices_nonblocking` = BgProbe, `formats_nonblocking` = per-device cache + thread).
  - Mirror flips the recording (`hflip` before the even-crop) and the preview (`preview_output_args(pre)`).
  - The idle preview opens the camera through `screen_grab::Preview` (ffmpeg branch, `source_input`). Record
    and Snapshot drop it first, because a webcam opens for one process at a time.
- **Bubble** (`webcam::Bubble`, `webcam::bubble_graph`, `ScreenCaptureOptions::bubble`). The camera is input 1
  of the screen recording's own ffmpeg: square crop, scale to the size, a `geq` alpha circle with a one-pixel soft
  edge, then `overlay` in a corner. `-copyts` keeps both inputs on the wall clock so frames pair by real time,
  and `setpts=PTS-STARTPTS` after the overlay starts the output at the screen's first frame, as before. The
  pane preview taps the composite (`split` -> `[pv]`). It works on the gdigrab and WGC (rawvideo stdin) paths.
  It's skipped for a webcam source or with no camera.
- **Screen Snapshot** (`capture::snapshot`, `capture::snapshot_args`, `App::take_snapshot` in `jobs.rs`).
  - A Snapshot button under Record, and `Action::ScreenSnapshot` ("Screen Snapshot", File ▸ Record, palette,
    `ui.action screen_snapshot`).
  - It takes one gdigrab frame as PNG. An App source is grabbed by its title, and a webcam skips its first
    second while exposure settles (`photo-<time>.png`).
  - Runs on its own thread. With Hide on Record, our windows get `WDA_EXCLUDEFROMCAPTURE` and are minimised,
    then restored. Settings ▸ (pane) Misc ▸ Snapshot delay: 0/3/5/10 s.
  - Imported through `pending_recordings`, which gained a `snapshot` flag, into the capture bin. Toast:
    "Snapshot imported".
- **Pane** (`src/ui/capture_ui.rs`):
  - A new "Webcam" fold with Camera (+ Refresh), Format, Mirror, Bubble, Corner and Size. Labels are names
    only; explanations are in tooltips.
  - "Webcam" is in the Type combo with a Camera row.
  - Record and Snapshot are disabled with a reason ("Looking for webcams…" / "No webcam found").
  - `recorder_pane::sync_cam` resolves the camera each frame from the cached probes.
- **Settings** (`// ---- parity-webcam ----` section after `capture_dir`): `cam_device`, `cam_format`,
  `cam_mirror` (off), `cam_bubble` (off), `cam_bubble_size` (280), `cam_bubble_corner` ("br"),
  `snapshot_delay` (0).
- **MCP** `capture.screen`: source `webcam`, args `camera` / `format` / `bubble`, actions `devices` (webcams and
  the picked camera's format keys; blocking ffmpeg on the MCP call, not a frame) and `snapshot`. Status reports
  source / camera / bubble / snapping. There are no new tools, so the tool count is unchanged.
- Docs: `website/docs/screen-recording.mdx` has Webcam and Screen Snapshot sections. CHANGELOG has a line under
  `## Unreleased`.

## Verified here (Linux container)

- `cargo check --target x86_64-pc-windows-gnu --all-targets`: passes (only pre-existing warnings).
- `cargo test --target x86_64-pc-windows-gnu --no-run`: the test binary builds. I couldn't run it: Wine 9.0 in
  this container dies at start (`free(): invalid pointer`, even for `wine cmd /c echo`). The crate itself can't
  build for Linux (winit refuses).
- `src/engine/webcam.rs`'s 6 unit tests ran and passed on Linux in a scratch harness that compiles the file
  with stub neighbours (`parse_dshow` copied in): device and format parsing, key round trip, input args, bubble
  size, graph text, corners.
- **The bubble graph was run through a real ffmpeg (6.1, Linux).** Two lavfi inputs stood in for the screen and
  the camera, with `-copyts` and `-itsoffset 1795000000` / `+0.5 s` to mimic wall-clock stamps and a camera
  that opens 0.5 s late. Results:
  - The output starts at 0.000 and runs the full 3.0 s.
  - The screen's own timecode matches output time.
  - There is no bubble before the camera's first frame.
  - The circle sits in the right corner.
  - The preview output got all 90 frames.
  - A camera that turns bright at its own t = 1.0 s turns bright at output 1.5 s (dark at 1.4, bright at 1.6),
    so the camera is time-aligned.

## Not verified (needs Windows and a real camera)

- Every Windows-only test I added or touched (`capture.rs`: `cameras_parse_from_the_device_listing`,
  `webcam_source_args`, `bubble_args_composite_the_camera`, `snapshot_args_take_one_frame`; `capture_ui.rs`:
  `webcam_source_and_bubble_options`, the extended `assert_no_idle_repaint_screen_recording_pane`). They compile
  but never ran. Run `cargo test capture` first.
- A real dshow camera: device and format listing on ffmpeg 7.1 for Windows, preview, recording, mirror. Also
  that `-use_wallclock_as_timestamps 1` on dshow gives the `start:` line the audio alignment needs (it is the
  generic libavformat option, so it should).
- The bubble live on gdigrab and on the WGC path: CPU cost of `geq` at ~280 px (estimated small but not
  measured), and `-copyts` with gdigrab / rawvideo on Windows ffmpeg 7.1.
- Snapshot: gdigrab of an app window by title (GPU-drawn games may come out black under GDI, as the code
  comment says), Hide on Record + delay, and the webcam photo.
- No screenshots of the pane were taken (`--screenshot` needs Windows). The Webcam fold and the Snapshot button
  are only checked by the headless layout test.
- Regenerate the shortcuts page (`--dump-hotkeys`), which has a new unbound action "Screen Snapshot", and the
  docs screenshot of the Screen Recording window if wanted.
- Size gate (`scripts/size.ps1`) not run. Expected small: one module of string building and parsing, and no
  new dependency.

## Risks

- **Camera stall with the bubble**: `overlay` waits for the camera's next frame. A camera that stops
  delivering without EOF (some drivers on sleep) would hold the screen recording's frames back. An unplug gives
  EOF, and `eof_action=pass` carries on with the screen alone.
- **Device busy**: the idle webcam preview owns the camera. Record and Snapshot drop it first, and
  `feed_preview` keeps it closed while a webcam snapshot runs. A `-list_options` probe running at the same
  moment as Record could still collide (rare).
- The bubble isn't in the before-recording preview, which shows the screen alone. Its tooltip says so.
- Two identical cameras share a dshow name, and only the first is reachable by name. Using the "Alternative
  name" would fix it.
