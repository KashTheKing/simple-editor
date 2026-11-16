---
name: clean-audio
description: Clean up speech audio: repair, level it and duck music under it.
args: "[what bothers you: noise, hum, echo, quiet, or empty]"
tags: audio, voice, noise, ducking, repair
---
Make the voice clear and evenly loud, and keep music out of its way.

1. Call `editor.context`, `timeline.list` and `track.list`. Classify audio clips: voice (talking clips), music, other. Tag them with `audio.role {clip_ids, role}` (`Dialogue`, `Music`, `Sfx`, `Ambience`) only if they are `Unset`. If unclear which clip is the voice, ask.
2. Measure first: `audio.analyze {clip_ids}` for peak and RMS, and `mixer.meters` for loudness if playing. Say what you found ("voice peaks at -3 dBFS, very quiet music").
3. Repair the voice: `audio.repair {clip_ids: [voice], preset: "repair"}` for noise and rumble; `preset: "clarity"` when the problem is muddy or dull speech. Use $ARGS to choose: noise/hum/echo -> `repair`, quiet/muddy -> `clarity`; empty -> `repair`. For heavy steady noise, `export.denoise` bakes a cleaned copy, but it is slow and replaces the clip, so ask first.
4. Level it: `audio.normalize {clip_ids: [voice], mode: "rms", target_dbfs: -18, dry_run: true}` to see, then apply. For two or more voice clips use `audio.match_loudness {clip_ids}` so they sit at one level (it needs at least two). Clips with keyframed volume are skipped; report which.
5. Music: if a music clip runs under speech, FIRST set its base level with `clip.set {clip_id, fields: {volume: ...}}` (`volume` is a linear gain: 0.5 is about -6 dB, 0.25 about -12 dB) so music alone sits 8-12 dB under the voice; THEN duck it: `audio.duck {music_id, dialogue_ids: [voice], depth_db: -15, ramp_s: 0.4, dry_run: true}`, then the same call without `dry_run` (range -12 subtle to -20 strong). In this order: setting the volume after ducking wipes the duck keys.
6. Apply all the real changes in one group: `edit.begin {label: "Clean audio"}` ... `edit.end`. Run the `dry_run` calls before it.
7. Verify: `audio.analyze` reads the source file and ignores clip gain, so work out the new levels yourself (analysed level + the gain you set, in dB = 20 x log10(volume)), check `project.get` for the duck keys, and ask the user to play a loud passage while you read `mixer.meters`. Targets: speech peaks around -6 dBFS, nothing above -1 dBFS; the export's loudness pass then lands it at -14 LUFS (see `/loudness` for podcast or broadcast). You cannot listen; say what you measured.

What bothers the user: $ARGS

Report back: what was repaired, the before/after levels, whether music is ducked, and that one Undo reverts it. Mention the Mixer holds the editable filter chain.
