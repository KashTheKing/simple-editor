# kden-markers-zones (`claude/feat-zones-markers-a61ff6`): cloud hand-off

## Done
- Merged `origin/develop` into the paused WIP. The four conflicts were all neighbouring registry rows
  (`hotkeys.rs` actions and groups, `Project` fields, `Project::new`, `ACT_HANDLERS`): both sides were kept,
  with timeline-tabs / home-page first and zones-markers after.
- The WIP scope was already written: effect zones (model, zone lane, Inspector card, the render path through
  `engine::zones::drawn`), Export Each Section (`engine::sections`), marker categories, range-marker end
  drags, dropping a Markers-panel row on the timeline, clickable timecodes (`ui::timecode_links`), and MCP
  `zones.*`, `markers.categories` / `category_set` and `export.sections`.
- **Fixed: zone edits didn't refresh Movie Mode.** `App::after_edit` passes the raw project to
  `playback::video_dirty_spans`, and that diff ignored `zones`. A zone that is added, moved, edited or removed
  now dirties its old and new time range.
- **Fixed: zones never appeared in pre-rendered frames, and adding one could stall Movie Mode.**
  `PreRender::request` keyed each second on the raw project, but `tick` / `frame` key on the player's drawn
  copy. After a zone was added, the raw key still matched the old render on disk, so that second counted as
  done and was never queued again, while `frame` waited on the view's key. Both `request` call sites
  (`request_prerender`, `request_prerender_range`) now use `player.project()`, an `Arc` with no extra clone.
- Add Effect Zone and the zone card push an undo `Snap` (a project clone) instead of `to_json()`, which
  follows develop's rule of never serialising on the commit path.
- The zone-lane and range-marker drag math moved into small pure functions (`zone_lane::drag_to`,
  `marker_range::drag_end` / `place_marker`). A marker deleted mid-drag (by undo or an agent) no longer
  leaves an empty undo step on drop.
- `timecode_links::parse` is `#[cfg(test)]`: the app never called it, so it was a dead-code warning.
- CHANGELOG: the union merge had glued `### Timeline tools and crop` onto our list, so I added the
  missing blank line.

## Live-check items, now tests
| What | Test |
|---|---|
| A zone's bar: a click picks it, a body drag moves it, an end drag resizes it, one undo per gesture, and the lane only takes room while there is a zone | `timeline::tests::headless_zone_lane_click_picks_drag_moves_and_ends_resize` |
| A one-track zone's bar names its track | `headless_track_zone_bar_names_its_track` |
| No idle repaint over the zone lane | `assert_no_idle_repaint_timeline_zone_lane` |
| Dragging a range marker's end changes its length (the start stays), one undo | `headless_range_marker_end_drag_changes_its_length` |
| A Markers-panel row dropped on the tracks moves that marker there, one undo | `headless_markers_panel_row_drop_places_the_marker` |
| Row right-click ▸ Category ▸ Fix, one undo | `markers_ui::tests::row_menu_category_sets_it_with_one_undo` |
| The category filter, including Uncategorised and a filter whose category is gone | `category_filter_shows_one_category` |
| Dragging a row carries `MarkerDragId` | `dragging_a_row_carries_the_marker` |
| Drag limits (minimum length, the left end never passes the right, clamped at 0) | `zone_lane::tests::drags_move_and_resize_within_limits`, `marker_range::tests::end_drags_keep_the_other_end_and_a_minimum_length`, `a_dropped_marker_lands_where_dropped_and_clip_markers_stay_on_their_clip` |
| Movie Mode: zone edits dirty their window, and the pre-render key changes for the zone's seconds | `playback::tests::zone_edits_dirty_their_window`, `engine::zones::tests::a_zone_changes_the_prerender_key_of_its_seconds` |

The WIP's own tests are still there: `ops::zones` (4), `engine::zones` (4), `engine::sections` (5),
`timecode_links` (4, including a click reaching the app), `zone_lane` rows, `tools_zones` effect args, and
old projects loading with point markers.

## Verified here
- `cargo check --target x86_64-pc-windows-gnu --all-targets` has no errors. The only warnings in this
  branch's files are the `f32: From<f64>` fallback lints, which this container's rustc 1.97 raises
  everywhere in the crate. The pinned 1.89 doesn't.

## Not verified (coordinator, on Windows)
- **No test ran.** The crate is bin-only, winit refuses to build for Linux, and Wine 9.0 aborts with
  `free(): invalid pointer` here even on a hello-world exe. Please run:
  `cargo test zone`, `cargo test marker`, `cargo test sections`, `cargo test timecode`,
  `cargo test dirty`, then the full suite. The handoff said the WIP passed apart from load-timing tests.
- The new headless tests were written against the harness without running. If one fails, check the
  positions first: the zone row's y comes from `lanes_rect.top() - zone_lane::height`, and the range-end
  handle sits at the ruler's mid-height. The Category test opens the submenu by clicking it, the way the
  timeline tests do.
- Not covered by tests: the Inspector zone card (`zones_ui::card`), clicking Export Each Section (it opens
  a folder picker), and Next / Previous Marker (`App::act` has no headless App). `marker_step` is covered.
- No screenshot was taken. Please look at the zone lane, the range-marker bars and the Markers panel's
  category combo in the live app.

## Risks
- `rustfmt --check` reports drift in `zone_lane.rs` / `marker_range.rs` (long lines left by the WIP). I
  didn't reformat, per agents.md.
- Movie Mode now keys on the player's view, so projects with browser zoom details also re-render their
  cached seconds once (their key now includes the zoom clips). That's more correct, but it costs one
  re-render.
