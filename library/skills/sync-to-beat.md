---
name: sync-to-beat
description: Cut or mark the video to the beat of the music track.
args: "[mark, or cut every N beats]"
---
Line the edit up with the music: place markers on the beats, or cut the picture on them.

1. Call `editor.context` and `timeline.list`. Find the music clip: the selected audio clip, else the audio clip tagged Music or the longest one. If two candidates are plausible, ask which.
2. Detect without changing anything: `audio.beats {clip_ids: [music], refractory_s: 0.25}`. It returns onset times and the BPM. Report the BPM. If the BPM looks wrong (half or double of what the user hears), raise `refractory_s` to 0.4 and detect again.
3. Decide the mode from $ARGS. `mark` (or empty): markers only, nothing is cut. `cut` or a number N: cut the picture every N beats (default 4, so one cut per bar); cutting every single beat is only right for very fast music, so ask first if N is 1.
4. Markers: `edit.begin {label: "Beat markers"}`, `audio.beats {clip_ids: [music], as_markers: true}`, `edit.end`. Markers sit on the music clip; tell the user to snap cuts to them.
5. Cutting: take the onset times from step 2, keep every Nth, convert source time to timeline time using the music clip's start and `src_in` from `timeline.list`. Work back to front so earlier times do not move: for each time `timeline.split {t, clip_ids: [video clips]}` on the picture tracks only (not the music). If the picture is one long clip, tell the user the cuts will repeat the same shot and offer `/b-roll` or to rotate through other library clips with `timeline.replace {clip_id, asset_id}`; do not swap footage unasked.
6. Optionally make cuts feel on-beat with a small punch: `clip.keyframe {clip_id, property: "scale", t: 0, value: 1.08}` then 1.0 a quarter-second later. Only if the user asked.
7. Verify: `render.contact_sheet {from, to, count: 12}` and `markers.list`. Check the first cut lands on a downbeat.

Mode requested: $ARGS

Report back: the BPM, how many beats were marked or cuts made, and that one Undo reverts it all.
