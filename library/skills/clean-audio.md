---
name: clean-audio
description: Clean up speech audio: repair, level it and duck music under it.
args: "[what bothers you: noise, hum, echo, quiet, or empty]"
---
Make the voice clear and evenly loud, and keep music out of its way.

1. Call `editor.context`, `timeline.list` and `track.list`. Classify audio clips: voice (talking clips), music, other. Tag them with `audio.role {clip_ids, role}` (`Dialogue`, `Music`, `Sfx`, `Ambience`) only if they are `Unset`. If unclear which clip is the voice, ask.
2. Measure first: `audio.analyze {clip_ids}` for peak and RMS, and `mixer.meters` for loudness if playing. Say what you found ("voice peaks at -3 dBFS, very quiet music").
3. Repair the voice: `audio.repair {clip_ids: [voice], preset: "repair"}` for noise and rumble; `preset: "clarity"` when the problem is muddy or dull speech. Use $ARGS to choose: noise/hum/echo -> `repair`, quiet/muddy -> `clarity`; empty -> `repair`. For heavy steady noise, `export.denoise` bakes a cleaned copy, but it is slow and replaces the clip, so ask first.
4. Level it: `audio.normalize {clip_ids: [voice], mode: "rms", target_dbfs: -18, dry_run: true}` to see, then apply. For several voice clips use `audio.match_loudness {clip_ids}` so they sit at one level. Clips with keyframed volume are skipped; report which.
5. Music: if a music clip runs under speech, `audio.duck {music_id, dialogue_ids: [voice], dry_run: true}` first, then apply with `depth_db` about -12 and `ramp_s` 0.4. Also set the music's base level with `clip.set {clip_id, fields: {volume: ...}}` if it is louder than the voice.
6. Apply all the real changes in one group: `edit.begin {label: "Clean audio"}` ... `edit.end`. Run the `dry_run` calls before it.
7. Verify with `audio.analyze` again and `mixer.meters` (voice about -16 to -12 LUFS area, no clipping). You cannot listen; say what you measured and suggest the user plays a few seconds.

What bothers the user: $ARGS

Report back: what was repaired, the before/after levels, whether music is ducked, and that one Undo reverts it. Mention the Mixer holds the editable filter chain.
