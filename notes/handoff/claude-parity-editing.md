# Hand-off: parity-editing (`claude/zen-clarke-jd542a`)

Section **parity-editing** of `plans/cloud-handoff.md` (item 8 of `plans/shotcut-comparison.md`).
The cloud session's harness only allows pushing to `claude/zen-clarke-jd542a`, so the work lives there
instead of `claude/parity-editing`. It branches from `origin/develop` at `41c9b73` and has no merge commits.

## Audit of the 12 items

| Item | State before | Done here |
|---|---|---|
| Batch export from the Library | Missing (Convert To ran every file at once, next to the source, and imported it) | **Export Each As ▸ format**: one folder, one job, one file at a time (`convert::start_batch`, `ui/app/batch_export.rs`) |
| Slideshow / montage generator | Missing | **Make Montage** (Library menu) and MCP `timeline.montage` (`model/ops/montage.rs`) |
| Clip names and comments | Names existed (Inspector field, `clip.set name`) | **`Clip.comment`**: Inspector box, a folded corner and a hover note on the timeline, `clip.set comment` |
| Chapters embedded in the export | Text, CSV and Avid only | **Export ▸ Advanced ▸ Chapters from markers** (on by default), `export.video chapters` |
| Time format SMPTE / clock / frames | Missing | **Settings ▸ General ▸ Time display**; `ui::timecode` follows it app-wide; `settings.set time_format` |
| Per-filter presets | Half: Save to Gallery ▸ "This effect, with its settings" existed, but nothing loaded one into an existing effect | **Effect row ▸ Presets ▸ name** loads the saved settings in place (one undo) |
| Library: sort by created / recorded date | Missing (search and the type filter existed) | **Sort by Date Created / Date Recorded**, `media.list sort=created|recorded` (`engine/media_dates.rs`) |
| File-browser search and type filter | Already there (the Media Browser shares the Library's search and kind chips, `path_matches`) | Nothing |
| Nudge clips and groups | 1-frame nudge existed (`,` / `.`), selection + linked clips | **Shift+, / Shift+.** nudge 10 frames. Groups: whatever `expand_links` covers once gap-group-sync lands |
| Save a trimmed clip as its own file | Missing | **Clip right-click ▸ Save Clip as File…** (`ConvertOptions.range`, re-encode, then imported) |
| Import Shotcut `.mlt` | Missing | **File ▸ Import Timeline** reads `.mlt` (`engine/import/mlt.rs`) |
| Starter asset pack (emoji, real SFX, graphics) | Missing | **Not done**, see below |

## Verified

- `cargo check --target x86_64-pc-windows-gnu --all-targets`: clean after every commit (no new warnings in my files).
- The Linux test build is impossible: `winit` has no Linux backend enabled in this crate ("The platform you're
  compiling for is not supported by winit"). Wine was tried for the Windows test exe and aborts even on a
  hello-world in this sandbox (`free(): invalid pointer`), so **the crate's own tests compile but were not run**.
- What I could run for real:
  - `rustc --test src/engine/media_dates.rs`: both tests pass, and `read_dates` on an ffmpeg-made MP4 with
    `creation_time=2024-03-05T12:00:00Z` returns exactly that time.
  - The pure chapter / time-format / MLT-time functions, extracted with their tests into one file: 4 pass
    (`chapters_follow_markers_inside_the_exported_range`, `chapter_titles_survive_the_metadata_strip`,
    `time_formats_smpte_clock_and_frames`, `mlt_time_reads_frames_clock_and_smpte`).
  - The exact ffmpeg arguments with ffmpeg 6.1: chapters with titles in MP4, MKV and WebM; user metadata still
    controlled; `-ss a -i src -t len` gives the exact clip length. This found a real bug, now fixed: the export's
    bare `-map_metadata -1` also drops chapter titles, so with chapters it is `-map_metadata:g -1 -map_metadata:s -1`.

## Untested (please run on Windows)

- `cargo test` in full. New tests: `clip_comment_round_trips_and_stays_out_of_old_files`,
  `time_formats_smpte_clock_and_frames`, the three `chapters_*` / `review_comments_are_not_chapters`,
  `chapter_titles_survive_the_metadata_strip`, `save_clip_as_file_takes_the_used_source_range`,
  `a_filter_preset_loads_settings_but_keeps_the_effects_place_on_the_clip`,
  `batch_export_plans_one_file_each_with_unique_names`, the three in `model/ops/montage.rs`,
  `shotcut_project_imports_tracks_clips_transitions_and_markers`, and media_dates' two.
  The registry tests should pass: `make_montage` has an `OP_TOOLS` row and `montage.rs` is in `OP_FILES`; the
  three new Actions go through `ui.action`.
- No screenshots: the Inspector Comment box, the timeline's folded corner, the Time display combo, the Export
  checkbox, the Presets submenu and the Library menu rows have not been looked at.
- The Shotcut import was built from the `.mlt` format, not from a real Shotcut file: try one with transitions and
  a few filters. Transitions become dissolves with A and B each reaching half into the old transition, so the
  edit keeps its length.
- A live export with markers, then check the chapters in a player.

## Risks / notes for the merge

- `ExportOptions` gained `chapters` and `ConvertOptions` gained `range`: every struct literal got the field, so
  a branch that adds its own literal (gap-alpha-export, forum-export) needs `chapters: Vec::new()` /
  `range: None` after merging.
- `ui::timecode` now reads a process-wide `TimeFormat` (an atomic, default SMPTE, set from settings at start
  and by the setting). Tests never change it.
- Library date sorts read the files on a worker thread and repaint every 300 ms only while dates are still
  missing and that sort is chosen.
- retro-pack also builds a slideshow (`slideshow.make`, Classic 2000s). `Project::make_montage` /
  `montage_plan` are generic, so it can call them instead of a second layout routine.
- Shared files touched outside my own lines: `settings.rs` (new fields at the end + defaults),
  `hotkeys.rs` (3 actions after ws:timeline-tabs), `ui/app/mod.rs` (one `mod` line, one set_time_format line),
  `jobs_pane.rs` / `actions.rs` (one line each), `tools_registry_tests.rs` (two lines).

## Starter asset pack: not done, and why

- Real SFX only: the usual CC0 sources (Freesound, Kenney, OpenGameArt) are blocked from this container's
  network (only GitHub answered), and I would not ship sounds whose recording and licence I could not check.
- The library rules say everything in `library/` is MIT-licensed and there is no sounds / images category yet.
  Noto Emoji images (Apache-2.0) are reachable, but adding them needs a licence-rule decision and a library
  category that downloads images into the Library (which also touches parity-formats' SVG import).
- Suggested follow-up, on a machine with normal network: a `library/sounds/` + `library/graphics/` category with a
  `LICENSES.md` per source (CC0 recordings only for SFX; Noto Emoji under Apache-2.0 with its NOTICE), installed
  into a "Starter Pack" Library bin.
