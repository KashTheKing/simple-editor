---
name: sfx-pass
description: Add sound effects at cuts, text and key moments from your library.
args: "[style: subtle, punchy, or a sound name]"
---
Place sound effects on the timeline from the user's own audio files (whooshes, hits, clicks). The app has no sound-effect store; do not invent audio.

1. Call `editor.context`, `timeline.list` and `media.list`. Look for audio assets whose name, tags or description suggest sound effects (whoosh, hit, click, pop, riser). Use $ARGS to narrow ("whoosh"). If there are none, stop and tell the user to import some sound files; mention `media.import` takes file paths if they give them.
2. Plan where effects go: on transitions and cuts (`timeline.list`), on text or title clips appearing, and at emphasised beats. `subtitles.get` and `markers.list` also show candidate moments. Keep it restrained: `subtle` (default) is one effect every 10-20 seconds at the biggest changes; `punchy` is one at each cut.
3. Show a short plan (time, effect, why) when there are more than 6 placements, and wait for a yes.
4. Place each effect on its own audio track so the dialogue is never touched: `timeline.place {asset_id, at, mode: "top"}` with `in`/`out` to shorten a long file. Put the sound just before the visual event (0.05-0.1 s early) so it feels in sync.
5. Mix: `audio.role {clip_ids, role: "Sfx"}` for the new clips, then set their level well under dialogue with `clip.set {clip_id, fields: {volume: ...}}` (about 0.4 to 0.6) and `fade_in` / `fade_out` of 0.02 to avoid clicks. `audio.normalize {clip_ids, mode: "peak", target_dbfs: -10, dry_run: true}` first if levels vary a lot.
6. Wrap in `edit.begin {label: "SFX pass"}` ... `edit.end`.
7. Verify with `timeline.list` that no effect overlaps speech badly, and `audio.analyze {clip_ids}` that nothing peaks above 0 dBFS. You cannot listen; ask the user to play it.

Request: $ARGS

Report back: how many effects, which files, which track, and that one Undo removes them.
