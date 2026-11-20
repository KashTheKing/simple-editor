# Hand-off: time-remap (`claude/feat-time-remap-8a3a4e`)

Cloud session, 2026-11-20. `origin/develop` merged in (one conflict in `src/ui/app/mod.rs` `TOOL_TABLES`:
kept both the `ws:time-remap` and `ws:timeline-tabs` entries).

## What's done

- **Model** (`src/model/timemap.rs`, `src/model/ops/timeremap.rs`, `Clip::time_map`): speed keys with a
  monotone-cubic curve (smooth) or constant speed per segment, holds, reverse sections (back then forward
  again, no picture jump), add/remove/move keys, segment speed/length. Length changes ripple the clip's
  track; linked picture + sound edit together. Split, trims, slip, paste attributes, save/load (old
  projects load unchanged; a broken map is dropped) all keep the mapping. Enabling bakes the current
  speed / ramp / reverse / freeze into keys; disabling goes back to one constant speed.
- **Picture**: lookups land on the middle of a source frame (`TimeMap::fps`), so a hold asks for one
  frame; a backward stretch in preview reuses cached frames within 2 source frames (`DecoderPool::cached_near`,
  never in an export pool).
- **Sound**: varispeed per sample from the map (`mixer::resample_mapped`), muted in holds and backward
  stretches with a 2 ms ramp, capped at 16x the block.
- **UI**: Inspector ▸ Time Remap section (lane, Add Key, Hold + length, Reverse, picked segment's Speed /
  Length / Smooth, Remove Key, Turn Off) and the Animator Speed row's Ramps ▸ Time Remap toggle, which
  turns the row into the same lane (drag a key in time, drag a segment for speed, double-click to add).
- **MCP**: `clip.time_remap` (get / enable / disable / set / hold / reverse / add_key / remove_key /
  speed / length / smooth); row added to `docs/customizing.md`; `OP_TOOLS` rows for every new op.
- **This session's fixes**: Freeze Frame on a remapped clip now drops the map (the map used to win, so
  the freeze did nothing); the Retime window's Reverse no longer sets a dead `reverse` flag on a
  remapped clip; Paste Attributes ▸ Speed from a plain clip turns the map off before sizing the clip
  (the length used to stay the map's). "◆" button is now "Add Key". Tests for all three.

## Verified here

- `cargo check --target x86_64-pc-windows-gnu --all-targets`: clean (no errors; warnings are the
  crate's existing ones from the newer cloud rustc).
- `src/model/timemap.rs` tests run on Linux in a standalone scratch crate: **11/11 pass**.

## Not verified (needs Windows)

- The crate doesn't build on Linux (winit), and Wine 9 crashes on start in this container, so none of
  these ran: `model::ops::timeremap` tests (10, 3 new), `mixer` `time_remap_follows_the_map...`,
  `compose` `time_remap_export_frames_land...`, `tools_timeremap` registration test,
  `tools_registry_tests` (`every_edit_op_has_a_tool`). Run `cargo test time_remap` and `cargo test timeremap`
  then the full suite.
- Nothing rendered: check the Inspector section and the Animator lane in the live app (screenshot),
  and play a hold + reverse section for stutter (perf gate) since backward stretches lean on the cache.

## Risks / merge notes

- **gap-pitch-speed** (`claude/gap-pitch-speed-f4c741`) rewrites the same `mix_audio_clip` /
  `mix_seq_clip` lines: expect a conflict in `src/engine/mixer.rs`. Resolve to one stretch path:
  1. Keep gap-pitch-speed's `stretch::stretches(clip)` branch first; extend `stretches` to
     `keep_pitch && (speed != 1 || time_map.is_some())`.
  2. Its `nominal` clock already goes through `clip.src_time`, which follows the map (audio clips have
     `fps = 0`, so no frame snapping). For a remapped clip pass `dir = 1` and mute holds / backward
     stretches the same way `resample_mapped` gates them (`map.speed(l) > STILL`); `too_fast` should read
     `map.speed` instead of `clip.speed`.
  3. Without Keep Pitch, the remapped clip stays on `mapped_span` + `add_block` (varispeed). Its
     `resample_add` call gains the new `(…, clip.speed, clip.reverse)` arguments.
  4. Add a sine-frequency test for a remapped Keep Pitch clip next to gap-pitch-speed's.
- `analysis::duck` / beats / scene cuts and transcription skip remapped clips (same as reversed ones).
- Speeds faster than 16x the block in one segment skip sound past the first 16x (`mapped_span` cap).
