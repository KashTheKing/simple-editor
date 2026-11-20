# Hand-off: gap-group-sync (`claude/gap-group-sync-05fe02`)

## Done
- Merged `origin/develop` (one conflict in `src/ui/app/mod.rs`: kept both `mod group_sync` and `mod os_paste`).
- **Groups** (`Clip.group`, saved only when non-zero so old files load unchanged): Ctrl+G / Ctrl+Shift+G
  (`Action::GroupClips` / `UngroupClips`), in the Clip menu and the clip right-click menu. A click, a press-drag
  or a right-click on any member takes the whole group (`timeline::group_unit`); Alt+click picks one clip.
  Move, delete, copy / paste and duplicate run on the group through the normal selection path; a duplicate or a
  paste becomes a new group. Grouping clips already in groups merges them. Each member wears a thin outer edge
  in its group's label colour. Loaded group ids are reserved so `new_id` never hands them out again.
- **Clip ▸ Sync by Audio** (`Action::SyncByAudio`): `analysis::xcorr_match` = multicam's `xcorr_offset` plus a
  Pearson confidence. The first selected clip stays; the rest (with linked partners) move. Any match under 0.4
  moves nothing and says so; retimed clips are refused. This session moved the correlation **off the UI thread**:
  `gather` (cheap, cached `Arc<Peaks>`) on the UI thread, `correlate` on a worker, and the `group_sync::tick`
  frame hook lands it as one undo step, or drops it with a toast if the clips moved or were deleted meanwhile.
- **MCP**: `timeline.group`, `timeline.ungroup`, `timeline.sync_audio` (synchronous, like `audio.xcorr` and
  multicam sync). Rows in `docs/customizing.md`, `OP_TOOLS` entries in `tools_registry_tests.rs`.
- Docs: CHANGELOG (Unreleased), `website/docs/timeline.mdx`, `website/docs/shortcuts.md`.

## Verified here
- `cargo check --target x86_64-pc-windows-gnu --all-targets`: no errors (only the toolchain-wide float-fallback
  warnings the rest of the crate also has; this container has rustc 1.97, the project pins 1.89).
- `cargo test --target x86_64-pc-windows-gnu --no-run`: the test binary builds and links.

## Not verified (run on Windows)
- **No test ran.** The crate can't build for Linux (winit), and Wine 9.0 in this container crashes on start
  (`free(): invalid pointer`, even for `wine cmd`). Please run:
  `cargo test group_sync` and `cargo test group_click_selects_members_and_drag_moves_them`, then the full suite
  (hotkey-conflict, `ui_action_covers_every_action`, `every_edit_op_has_a_tool`, idle-repaint tests).
- Tests waiting for you: `sync_by_audio_lands_within_a_frame`, `unrelated_audio_is_a_weak_match`,
  `worker_sync_lands_once_and_respects_edits` (new), `group_delete_undo_and_save_load`,
  `copies_are_new_groups_and_sequences_keep_theirs`, and the timeline harness test above.
- Live check: Ctrl+G two clips on different tracks, drag one, Delete, Undo; Alt+click; Sync by Audio on two
  camera clips with a clap; the group edge colour in both themes.

## Risks
- `tick` polls every 100 ms only while a sync is pending; idle frames return at once (idle-repaint safe).
- Copying one member alone (Alt+click) and pasting gives a group of one: harmless, Ctrl+Shift+G clears it.
- Splitting a grouped clip keeps both halves in the group (Premiere does the same).
- The MCP `timeline.sync_audio` still correlates on the calling (UI) thread; for hour-long files that is about
  half a second, the same cost `audio.xcorr` and multicam sync already have.
