# Cloud hand-off (2026-11-19)

The user's PC was overloaded by too many local sessions, so the work moved to cloud sessions. Each cloud
session owns exactly one section below.

## Rules for every cloud session

- **Repository:** KashTheKing/simple-editor. Integration branch: `develop` (the 1.4.0 release branch,
  tested locally). Read `agents.md`, `goals.md` and `notes.md` first. CLAUDE.md says to.
- **Your branch:** `git fetch origin && git checkout <your branch>`. If your section says "new from
  develop", create `<your branch>` from `origin/develop` instead.
- **Platform:** Simple Editor is Windows-only (Media Foundation, D3D11/glow, winreg, SAPI). You can't
  run the app here. Check your work with:
  - `rustup target add x86_64-pc-windows-gnu`, then `cargo check --target x86_64-pc-windows-gnu --all-targets`.
    If a build script needs a C compiler, install `mingw-w64`.
  - any unit tests that build and run on Linux: try `cargo test --lib <filter>`. Skip what can't build here.
  The coordinator builds, tests and live-checks on Windows when it merges, so say exactly what you
  could and couldn't verify.
- **Done:** commit in small steps to your branch, and `git push origin <your branch>` after every
  meaningful step, so work survives. Write a short `notes/handoff/<your-branch>.md` with what's done,
  what's verified, what's untested and any risks. Never push to `develop`, `main` or anyone else's
  branch. No tags, no releases, no PRs.
- **CHANGELOG:** lines go under a `## Unreleased` heading at the top.
- **House rules:**
  - Labels are just the name, with explanations in tooltips.
  - References to other panes are buttons.
  - A button that opens a panel toggles it and is highlighted while open.
  - Use the aligned list rows (`ui::list_rows`).
  - No "Experimental" badges anywhere: that module is deleted.
  - Playback never stutters: no heavy compute on the playback or UI thread.
  - MIT licence: only permissively licensed code and models. No GPL linking.
  - Real SFX only, never synthesised.
  - The user's privacy: no personal data in requests.
- **develop already has:**
  - timeline tabs (`timeline_tabs.rs`, `Project::is_blank`)
  - instant trims (UndoEntry.json is a `Snap`; never serialise the project on the commit path)
  - the Home page (`home_ctl.rs`)
  - built-in crop and the simpler timeline tool row
  - the Assistant restyle with app-wide `on_accent` / `on_accent_over`
  - the page-switch freeze fix (`engine::bg_probe`, `Settings::save_async`)
  - TikTok/Snapchat text styles
  - Movie Mode partial re-render (`playback::video_dirty_spans`)
  - the save prompt that compares against the saved snapshot (`App::saved`, `still_clean`)
  If your branch is older, merge `origin/develop` into it first, and resolve keeping both sides' intent.

## Continue (branch has work in progress, committed as "wip: paused mid-task")

Each of these was written but not finished or compiled when paused. Merge `origin/develop`, make it compile
(`cargo check` as above), finish the scope, and add the tests.

### addon-deepfilter: `claude/addon-deepfilternet-fa944b`
- DeepFilterNet (MIT/Apache) as an optional speech-denoise add-on driving Clean Audio and the Denoise bake. ffmpeg `afftdn` stays the fallback.
- Install on demand like `src/engine/enhance.rs`, with a Settings row, a strength control and A/B.
- MCP: an `engine` option on the denoise and repair tools.

### addon-whisperx: `claude/addon-faster-whisper-6d9415`
- faster-whisper (MIT) plus WhisperX-style forced alignment (BSD-2) as an optional transcription add-on, with no Python required of the user.
- An engine choice next to whisper.cpp, and precise word timing on by default when installed.
- The same transcript model as today. MCP: `engine` and `align` options.

### addon-demucs: `claude/addon-demucs-e6f150`
- Demucs (MIT), Python-free, for stem separation: Separate Stems, Remove Vocals, Keep Only Vocals.
- Stems go into a Library bin, with optional tracks under the clip.
- Duck the vocals only; beats from the drums stem. MCP `audio.stems`.

### addon-sam2: `claude/addon-sam2-b6943f`
- SAM 2 (Apache-2.0) object masks: click points in the Preview, then Track through the clip.
- Uses a cached mask sequence (the helper exe already tracks at IoU 0.93–1.0), with a CPU fallback.
- MCP: `mask.object` and `mask.track`.

### addon-bgremove: `claude/addon-bg-removal-0f55aa`
- Background removal with MODNet (Apache, Fast) or BiRefNet (MIT, Best) via an onnxruntime helper.
- A cached alpha matte with temporal smoothing, an Edge slider and a Show Matte toggle.
- MCP `clip.remove_background`.

### addon-voice: `claude/addon-voice-tts-dc9111`
- Kokoro-82M (Apache) as the default realistic TTS, and Chatterbox (MIT) for expressive voices and cloning from a 5–15 s clip, with a consent line. SAPI stays as the fallback.
- Engine and voice picker with preview, speed and pitch, and "Speak selection" onto a voiceover track.
- MCP `tts.speak` options.

### styles-context: `claude/feat-styles-context-9e1d7f`
- Project Context pane: per-project markdown plus a digest of the planner, notes, moodboard and markers. The Assistant reads it every request.
- Editing Styles library (markdown with front matter) and `/style <name>`.
- "Learn this style" from a reference video: our analysis measures, and SmolVLM2 (Apache) describes.
- MCP `context.*` and `styles.*`.

### kden-markers-zones: `claude/feat-zones-markers-a61ff6`
- Effect zones: effects on a time range of a track or the whole timeline.
- Export between markers (one file per section).
- Marker categories with colours, plus ranged markers (a duration, a bar on the ruler, dragged from the list).
- Clickable timecodes in notes (a shared `ui::timecode_links`).
- Its build compiled and the full suite passed apart from load-timing tests. Finish the live-check items as tests.

### time-remap: `claude/feat-time-remap-8a3a4e`
- Clip time remapping: speed keyframes and curve, holds, reverse sections, Inspector section, and the Animator Speed view.
- Audio follows: use gap-pitch-speed's stretch function once it lands, or keep one shared stretch path.
- MCP `clip.time_remap`.

### gallery-stock: `claude/feat-gallery-stock-b07f2c`
- Stock tab in the Gallery. Keyless official APIs: Wikimedia Commons, Internet Archive, NASA, Openverse.
- Pexels, Pixabay and Freesound with the user's own key in Settings.
- Add downloads into the Library with licence and attribution. Credits export.
- MCP `stock.search` and `stock.add`.

### animator-props-blocks: `claude/feat-animator-props-blocks-f737e9`
- Collapsible per-clip property lanes, with a right-click show/hide list (Show All, Hide Unkeyed, Only Keyed, Collapse/Expand All, Reset).
- A Gallery Blocks tab by category, with drag and drop onto the Animator views and the timeline.
- 37 tests passed before the pause. Finish and push.

### gap-pitch-speed: `claude/gap-pitch-speed-f4c741`
- Pitch-preserving clip speed with a "Keep pitch" toggle, audible from 0.25x to 4x, in playback, export and bakes.
- No GPL (no Rubber Band). Prove it with a sine-frequency test.

### gap-group-sync: `claude/gap-group-sync-05fe02`
- Group/Ungroup clips (Ctrl+G / Ctrl+Shift+G; Alt+click picks one clip).
- Clip ▸ Sync by Audio, reusing the multicam cross-correlation.
- MCP `timeline.group` / `ungroup` / `sync_audio`.

### gap-alpha-export: `claude/gap-alpha-export-9a28d8`
- Transparent export: ProRes 4444, PNG MOV, QuickTime Animation, VP9 alpha.
- PNG, JPEG and TIFF image-sequence export.
- No dark fringes. MCP options.

### mixed-footage: `claude/feat-mixed-footage-83d1e8`
- Batch-wide project match that picks a frame rate the clips divide into evenly.
- High-frame-rate "Slow motion (every frame)" conform; frame sampling vs blending.
- 4K Punch-in zoom with a 1:1 marker; a proxy offer for heavy codecs; camera colour match and S-Log/HLG LUT offers.
- A docs page "Mixing cameras and frame rates".

## New from develop

### forum-ui: `claude/forum-ui-findability`
Implement "Session A: UI simplification and findability" (plus Session E if it fits) from
`plans/shotcut-forum-insights.md`. Group/Ungroup is owned by gap-group-sync, so don't redo it.

### forum-export: `claude/forum-export-reliability`
Implement "Session B: Export reliability and clarity" from `plans/shotcut-forum-insights.md`. Keep
your changes in separate functions from gap-alpha-export.

### forum-crash: `claude/forum-crash-recovery`
Implement "Session C: Crash resilience and recovery" from `plans/shotcut-forum-insights.md`. Leave out
item 7 (signing).

### forum-preview: `claude/forum-preview-perf`
Implement "Session D: Preview performance and phone footage" from `plans/shotcut-forum-insights.md`.
Hook into mixed-footage's proxy offer rather than duplicating it.

### reference-clips: `claude/feat-reference-clips`
- Find a Reference pane: browser search buttons for Yarn, PlayPhrase, Clip.cafe, Tenor, GIPHY and YouTube.
- In-app GIPHY/Tenor search via their official APIs, with the user's own free key in Settings and the "Powered by" attribution.
- "Just a part" range import on the yt-dlp URL importer.
- A References smart bin, auto-transcribed and searchable by quote.
- Assistant `references.suggest`, `search_links` and `import_range`.
- A one-time fair-use note, sources kept, and "Copy credits". Never scrape or bulk-download.

### installer: `claude/feat-installer`
- A Rust installer, updater and uninstaller, each exe under 1 MB: raw Win32 via `windows-sys`, WinHTTP, `opt-level="z"`, LTO, `panic=abort`, strip.
- Opens instantly.
- **Install:**
  - per-user, no admin, to `%LOCALAPPDATA%\Programs\Simple Editor`
  - FFmpeg missing: say so clearly, with Install FFmpeg ticked by default (a checksum-verified download)
  - an optional add-on checklist with one-line uses ("yt-dlp - Download videos from a URL", …); ones already installed show Installed
  - Start menu and optional desktop shortcuts, an uninstall entry, the `.sedit` association, and the same context-menu keys as `src/contextmenu.rs`
- **Updater:** GitHub latest-release check, download, wait for the app to close, atomic swap with rollback, settings kept.
- **Uninstaller:** asks Keep or Remove preferences; removes add-ons, the FFmpeg it installed, every registry key either the installer or the app added (legacy keys included) and the shortcuts; deletes itself; never touches projects.
- Tests use a redirected registry root and folder.
- A release script builds both the portable exe and the setup exe.

### size-diet: deferred
Needs Windows release builds, so the coordinator runs it locally after everything is merged.

### retro-pack: `claude/feat-retro-2000s-pack` (new from develop)
The user's request, verbatim: "Add legacy windows movie maker effects/transitions and stuff. Remake them. Blue background white text
comic sans type stuff with the slideshow stuff."
Build a "Classic 2000s" pack: our own remakes of that early-2000s home-movie editor look. Don't use Microsoft's
assets, names or logos; call it "Classic 2000s" or "Retro Slideshow", not "Movie Maker".
- **Transitions** (shaders in the library format; check `library/` for how existing transitions are packaged):
  - fade/dissolve, fade through black or white, wipes (bars, blinds, checkerboard, diagonal, clock)
  - circle, diamond, heart, star, keyhole, iris
  - page curl/peel, pixelate, rectangle zoom, flip, roll, shatter-in, sweep, whirlwind, ripple, spiral, zig-zag, bow tie
- **Effects:** sepia, black and white, old film (grain, scratches, flicker), blur, posterize, watercolour, 3D ripple,
  mirror, edge detect, film age, slow and fast "speed up/slow down" presets, and fade in/out from black/white.
- **Titles** (native text clips plus presets):
  - The signature card: a solid royal-blue background, white text, Comic Sans style font. Use Comic Sans MS when
    it's installed on the PC; never bundle it. Bundle "Comic Neue" (OFL) as the fallback.
  - Title animations: fade in/out, fly in, typewriter, news banner, scroll credits, zoom, spin, "moving titles"
    and "basic title" with a drop shadow.
  - Credits roll presets: "Directed by / Starring / ...".
- **Slideshow / AutoMovie:**
  - Select photos and clips, then Make Slideshow: a pan-and-zoom (Ken Burns) on each photo, a chosen or random
    classic transition between each, a blue title card at the start and a credits card at the end.
  - An option to fit to the length of the background music.
  - One undo step. Expose it as an MCP tool (`slideshow.make`) and in the Gallery under a "Classic 2000s" category
    with thumbnails.
- Everything goes in the Gallery under one "Classic 2000s" category, with previews.
- Tests: preset parse, slideshow timing (fit to music), font fallback when Comic Sans is missing.

### scripts-editor: `claude/feat-scripts-editor` (new from develop)
The user's request, verbatim: "make it so i can rename, delete, open, open folder on each of the scripts and also an "Add script"
button that opens up in the app's script editor with a button in the script editor panel to open externally and settings in
simple editor to open scripts externally by default."

Today: Window ▸ Scripts lists each script (`script_metas`, `run_script_path`, `src/scripting.rs` `scripts_dir`) plus
"Open Scripts Folder", and there's no script editor.

- **Scripts menu:** each script becomes a submenu with these items:
  - Run (the existing behaviour)
  - Open: the in-app editor, or the external editor if that setting is on
  - Open Folder: use `crate::contextmenu::reveal(path)`, which selects the file
  - Rename…: an inline name field, with the extension kept and collisions refused
  - Delete…: a confirm, moving the file to the Recycle Bin rather than a hard delete
  Below the list: **Add Script** (creates `New Script.luau` from a small commented template and opens it) and Open Scripts Folder.
  Show the Scripts submenu even when there are no scripts, so Add Script is reachable.
- **Script Editor pane** (a new `Pane::ScriptEditor` with its own toggle button):
  - A code editor reusing the app's existing code-editor widget (see `src/ui/html_code.rs` / `shader_ui.rs`), with
    Luau highlighting if the widget supports it.
  - File name, Save (Ctrl+S while focused), Run, and an error/output line from the script run.
  - **Open Externally**, which uses the OS default app for `.luau`/`.lua`, or `notepad` if none is registered.
  - Unsaved-change dot; reloads when the file changes on disk.
  - The palette and the Window menu list it.
- **Settings ▸ General:** "Open scripts externally" (off by default). When on, Open and Add Script launch the external editor instead.
- MCP: `scripts.create`, `scripts.rename`, `scripts.delete` and `scripts.open` next to the existing `scripts.list`/`scripts.run`.
- Tests: create, rename and delete on a scratch scripts dir; the template parses; the menu shows Add Script with zero scripts.

### plugins: `claude/feat-plugins` (new from develop; do it after scripts-editor lands, or merge that branch in first)
The user's request, verbatim: "Make it so people can make their own plugins that creates panels/scripts and stuff and can be
installed easily with a github link."

Build on what exists: Luau scripting with hooks and expressions (`src/scripting.rs`), skills, the online library
(effects, shaders, transitions, LUTs, HTML templates, icons), and the Gallery. A plugin is a folder that bundles these,
plus a new panel API.

- **Format:** a folder or zip with `plugin.json`:
  - id, name, version, author, description, min app version, homepage, icon
  - `contents`: scripts, panels, skills, effects/shaders/transitions/LUTs/templates/fonts (OFL only)
  - `permissions`: see below
  Write a `docs/plugins.md` spec and a `website/docs/plugins.mdx` guide with a complete example plugin, and add the
  example plugin under `examples/plugins/hello-panel/`.
- **Panels from Luau:**
  - A small immediate-mode UI API (`ui.label`, `ui.button`, `ui.slider`, `ui.text`, `ui.checkbox`, `ui.dropdown`,
    `ui.row`/`ui.column`, `ui.separator`, `ui.image`), drawn natively with the app's widgets and `ui::list_rows` style.
  - The panel's Luau gets the same project API scripts have (read the selection, run edit commands as one undo step).
  - A panel becomes a dockable pane, listed in Window ▸ Plugins with its own toggle button, highlighted while open.
  - It runs on the UI thread with a strict time budget per frame. Abort and show an error if a panel's draw
    exceeds it; playback must never stutter.
- **Install from GitHub:**
  - Plugins ▸ Install from URL. Paste `https://github.com/owner/repo` (or a release, tag or subfolder URL).
  - Download the latest release's zip asset if there is one, else the default branch archive from codeload.
  - Unpack into `%APPDATA%\SimpleEditor\plugins\<id>`.
  - Before installing, show a confirm with the name, author, version, contents and permissions, and the source URL.
- **Plugins manager:** a pane listing installed plugins, each with Enable, Disable, Update (checks GitHub for a newer
  version), Uninstall (removes the folder and everything it registered), Open Folder (`contextmenu::reveal`) and
  View on GitHub.
- **Security:** plugins are community code.
  - Luau is sandboxed: no `os`/`io` except through the app API.
  - Permissions are declared and granted at install: `network` (allowlisted hosts), `files` (the project folder only,
    or folders the user picks), `run_programs` (off by default; a big warning), and `mcp_tools` (which app tools).
  - Anything undeclared is refused at runtime with a clear message.
  - Never auto-run on install. Show "Only install plugins from people you trust".
- MCP: `plugins.list`, `plugins.install(url)` (still shows the confirm; never installs silently), `plugins.uninstall`
  and `plugins.enable`.
- Tests: manifest parse and validation; permission enforcement (a script without `network` can't fetch); install,
  uninstall and update from a local zip fixture and a mocked GitHub API response; the example plugin's panel draws.

## Shotcut parity (from plans/shotcut-comparison.md; each is a new branch from develop)
For each, read the matching item in `plans/shotcut-comparison.md`. Check what already exists before building, and
skip anything another section owns (pitch-speed, group-sync, alpha-export, time-remap, zones-markers, mixed-footage,
forum-*).

### parity-audio: `claude/parity-audio`
- Audio filters on a single clip, not only via buses.
- The missing filter types: band pass, notch, expander, declick, mid-side, stereo width, channel copy/swap/invert,
  downmix, 15-band EQ, two-pass normalise.
- An audio spectrum analyser and vectorscope on the Audio page.
- Track auto-fade and seam.
- A tone generator.

### parity-formats: `claude/parity-formats`
- Subtitles: ASS/SSA import, soft subtitle tracks in MP4/MKV export, and extracting subtitles from a media file.
- SVG and AVIF image import.
- Lottie (and Rive if a permissive runtime fits) as animated clips.
- Generators: colour bars, noise, plasma, counter/timer.

### parity-webcam: `claude/parity-webcam`
- Webcam capture next to Screen Recording: device and format pick, a preview, record to the Library.
- An optional webcam bubble while screen recording.
- Screen Snapshot: a still of the desktop or a window.

### parity-preview-tools: `claude/parity-preview-tools`
- Preview: a configurable grid and title-safe and action-safe overlays, with snapping; zoom levels at 10% and 25%.
- Video bakes: denoise (hqdn3d/nlmeans) and deinterlace (bwdif).
- Effects: corner pin and lens correction.

### parity-editing: `claude/parity-editing`
Item 8 of the queue in `plans/shotcut-comparison.md`:
- batch export from the Library, a slideshow/montage generator, names and comments on timeline clips
- chapters embedded in the export, a time-format choice (SMPTE/clock/frames), per-filter presets
- Library: sort by created/recorded date, filter by type, search; file-browser search and type filter
- nudging clips and groups, saving a trimmed clip as its own file, and importing Shotcut .mlt projects
- a starter asset pack: emoji, real SFX and graphics, with licences checked

### parity-effects: `claude/parity-effects`
Item 9:
- every missing Shotcut video filter as a native effect or library shader: old film, halftone, sketch, mosaic, fisheye
  and the rest
- the missing blend modes (Saturate, Exclusion, the four HSL modes)
- the custom gradient wipe, and eyedropper white balance

### parity-360: `claude/parity-360`
Item 10: the 360° video filters.

### parity-io: `claude/parity-io`
Item 11:
- network stream playback, full-range option, auto-rotate override
- a detailed media properties panel and a bitrate graph, search in recents
- GPU choice, Contour ShuttlePRO support

### parity-color-pipeline: `claude/parity-color-pipeline` (largest)
Item 12:
- Write `plans/color-pipeline.md` first.
- Then 16-bit float, linear-light GPU compositing and a 10-bit end-to-end path behind a Settings ▸ Processing Mode
  choice, with 8-bit kept as the fast default.
- Playback must never stutter in any mode.
