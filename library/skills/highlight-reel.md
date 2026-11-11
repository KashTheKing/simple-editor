---
name: highlight-reel
description: Build a short highlight reel from the best moments of the selected footage.
args: "[target length, e.g. 60s, and/or what to look for]"
---
Assemble a tight highlight sequence from the user's own footage, leaving the originals untouched.

1. Call `editor.context`, `timeline.list` and `media.list`. Source: the selected clips; with none selected, use the clips on the timeline and say so. Target length: from $ARGS (default 45 seconds). What to look for also comes from $ARGS ("goals", "funny moments"); with nothing, favour energy.
2. Find candidates. Spoken footage: `transcript.get` (run `transcribe.run {clip_id, cues: false}` first if needed) and pick complete, strong sentences. Action or sport: `audio.analyze {clip_ids}` shows loud peaks and onsets, `timeline.pacing` shows long dull stretches, `media.scene_cuts {clip_id}` shows shot changes. Existing markers (`markers.list`) are the user's own picks; honour them first.
3. Choose 6-12 moments of 2-8 seconds each, totalling about the target, in an order that builds (strong start, best moment near the end). List them with times and why, and ask to confirm before building when the source is longer than a few minutes.
4. Build on new tracks so the originals stay: for each moment `timeline.place {asset_id, at, in, out, mode: "top"}`, or `timeline.duplicate {clip_ids}` and trim the copies with `timeline.trim`. Close holes with `timeline.close_gap {track, t}`.
5. Join them: `timeline.add_transition {right_clip_id, kind: "CrossFade", duration: 0.3}` between moments, or plain cuts if the user wants it punchy. Even out sound with `audio.match_loudness {clip_ids}`.
6. Keep it all in one group: `edit.begin {label: "Highlight reel"}` ... `edit.end`.
7. Verify with `timeline.list` (total length near the target) and `render.contact_sheet {from, to, count: 12}`; check each moment starts and ends cleanly.

Request: $ARGS

Report back: the number of moments, the total length, which track holds the reel, and that one Undo removes it.
