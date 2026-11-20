# Hand-off: `claude/forum-ui-findability` (forum-ui)

Session A of `plans/shotcut-forum-insights.md` (UI simplification and findability), branched from
`origin/develop`. Group/Ungroup left to gap-group-sync as asked. Nine small commits, each pushed.

## Done

| Plan item | What landed | Where |
|---|---|---|
| 1. Plain-phrase task box | Ctrl+P go-to mode lists up to 4 **Do** rows first for phrases like *cut out a part, freeze, split screen, blur, beep, ticker, rotate, duck music, make the voice clear, remove silences, captions, missing files, export* (25 tasks). Each row shows its shortcut and a hint tooltip; Enter runs it. | `src/ui/tasks.rs` (table + scorer + tests), `Command::Task` in `ui/palette.rs`, `palette_ctl::run_task` |
| 2. Track header | **Duplicate Track** (right-click). The track-name column **resizes by dragging its right edge** (min = the old 150 pt default, which fits ~10 characters; max 420), saved in `Settings.timeline_header_w`. Rename / Move were already there. | `model/ops/findability.rs::duplicate_track`, `timeline/header.rs`, `timeline/mod.rs` (`HEADER_W`, `header_w_done`), `app/timeline_pane.rs` |
| 2. Never auto-create tracks | Already true on develop (tracks are only added by dragging past the last row, or `find_free_track` when there is no room). Not changed; no Undo toast added. | - |
| 3. Delete and close gap | Labels: **Delete and Close Gap** (Delete, was "Ripple Delete") and **Delete and Leave Gap** (Backspace). **Shift+Delete** also runs it unless the user bound it. Ripple / Rolling / Slip / Slide tooltips gain a "what a drag does" line. Action ids unchanged, so saved bindings keep working. Docs said Shift+Delete was ripple delete; fixed to match the real keys. | `hotkeys.rs`, `ui/tools.rs::panel_tool_does`, `palette_ctl::tick` |
| 4. Layouts | Clip right-click ▸ **Layout**: Side by Side, Three Across, 2 × 2, 3 × 2, Picture in Picture (scale-to-cover + crop per cell, using the asset's native size). Also from the palette. | `findability.rs::{Layout, fill_cell, apply_layout}`, `timeline/menus.rs` |
| 4. Ticker | **News Ticker** and **Scrolling Credits** built-in titles (keyframed x / y). | `engine/presets.rs::builtin_titles` |
| 4. Censor | Clip right-click ▸ **Censor ▸ Blur** (an adjustment clip with Blur r=40 inside a feathered rect mask, over In→Out else the selection's span) and **Censor ▸ Mute** (volume keys dip to 0 over the range, 20 ms ramps). | `findability.rs::{censor, censor_range}` |
| 5. Keyframes | **Smooth** tick in the shared keyframe menu (all keys EaseInOut / back to Linear). Copy / paste keys between values already existed (`key_menu`). The **Animator lanes follow the playhead** when it leaves the view (seek or playback), never fighting a pan. | `ui/mod.rs::{is_smooth, set_smooth}`, `animator.rs::follow_playhead` |
| 6. Look | Settings ▸ Appearance ▸ **Clear borders** (1.5 pt, brighter panel / window strokes) and **Playhead width** (1–4 pt). "What moved" is a section in CHANGELOG's Unreleased, which the What's New window renders. | `theme.rs::clear_borders`, `settings_ui/appearance.rs`, `Settings.{clear_borders, playhead_w}` |
| 7. Library | Right-click ▸ **Replace Media…** (file picker, repoints the asset and its subclips; clips keep cuts). A **health line** ("N files missing" + Relink…) above the Library list while anything is offline, nothing when healthy. | `findability.rs::replace_asset_path`, `library.rs::health_line`, `app/library_pane.rs` |

New MCP tools (all `Mutate`, one undo step, rows in `app/tools_findability.rs`, registered under a
`// ---- ws:forum-ui ----` marker at the end of `TOOL_TABLES`, `OP_TOOLS` rows added):
`track.duplicate`, `clip.layout`, `clip.censor`, `clip.rotate`, `media.replace`. Listed in
`docs/customizing.md`. CHANGELOG (Unreleased: "Find it faster" + "What moved") and
`website/docs/{interface,timeline,shortcuts,right-click}` updated.

## Verified here (Linux container)

- `cargo check --target x86_64-pc-windows-gnu --all-targets`: clean (no new errors; warnings are the
  pre-existing ones) after every commit.
- `cargo test --target x86_64-pc-windows-gnu --no-run`: the test binary builds.

## Not verified (the coordinator should)

- **No test was run.** `cargo test` cannot build for Linux (winit refuses the platform), and Wine
  crashes at start in this container (`free(): invalid pointer` even for `wine cmd /c echo`). New tests to
  run: `findability`, `tasks::`, `smooth_toggles_ease_in_out_on_every_key`,
  `lanes_follow_a_playhead_that_leaves_the_view`, `header_column_resizes_by_dragging_its_edge`,
  `health_line_only_while_files_are_missing`, plus `tools_registry_tests` (OP_FILES now includes
  `findability.rs`).
- No screenshots, no `--selftest`, no live check, no perf bench (the plan's 50 ms drag/select budget on a
  3-hour project was not measured).
- Things to look at live:
  - the header resize handle (5 pt strip inside the header's right edge) wins the drag over the header
    cell and does not steal the eye / mute button's clicks;
  - `Clear borders` reads well on light themes too;
  - Layout on clips with odd aspect ratios (portrait phone footage into a 16:9 cell) looks right;
  - Censor ▸ Blur's box is where users expect (frame centre, 1/4 of the frame) and is draggable in the
    Preview like any clip mask.

## Risks / notes

- Existing tests that might care: the palette's go-to rows now start with task rows for matching text
  (none of the existing tests query a phrase that matches, as far as I can see); the Animator now
  changes `st.win` when the harness moves `ph` out of the window between frames (existing tests set `ph`
  before the first frame, so `last_ph` is `None` and nothing moves).
- Censor ▸ **Mute** instead of a beep: the house rule is "real SFX only, never synthesised", and there is
  no real bleep sound in `library/`. Adding a real bleep sample later only needs a third `Censor` arm.
- `media.replace` repoints the path only (like Relink); it does not re-probe duration / size. A shorter
  replacement file leaves clips running past its end, as a relink would.
- Left out: a Gallery ▸ **Layouts** tab (layouts are in the clip menu and the palette instead); the
  "dropping onto empty space adds one track with an Undo toast" toast; Session E (voice / recording format
  / model download), apart from the palette's "make the voice clear" row, which runs Duck.
- Labels follow the plain-labels rule; explanations are in tooltips.
