---
name: loudness
description: Mix to a delivery loudness target (web, podcast, broadcast) with dialogue on top and music ducked.
args: "[web, podcast or broadcast; or a LUFS number]"
tags: loudness, lufs, mix, ducking, audio, technique
credit: targets from EBU R128, AES TD1008 and platform guidance; duck depth from video-use (MIT)
---
Set the mix so speech is clear and the export lands on a standard loudness.

Targets (integrated loudness, true peak): `web` (YouTube, social, the default) -14 LUFS, -1 dBTP; `podcast` -16 LUFS, -1 dBTP; `broadcast` (EBU R128) -23 LUFS, -1 dBTP. A bare number in $ARGS is the LUFS target.

1. Call `editor.context`, `timeline.list` and `track.list`. Classify audio clips into dialogue, music, effects and ambience; tag untagged ones with `audio.role {clip_ids, role}` (`Dialogue`, `Music`, `Sfx`, `Ambience`). Ask if the voice clip is unclear.
2. Measure: `audio.analyze {clip_ids}` per clip (peak and RMS dBFS). Note clipping (peak above -1 dBFS) and clips far quieter than the rest.
3. Balance in this order, all as dry runs first (`dry_run: true`), then for real inside `edit.begin {label: "Loudness"}`:
   - Dialogue first: with two or more voice clips, `audio.match_loudness {clip_ids: [dialogue]}` so they sit at one level (it refuses a single clip), then `audio.normalize {clip_ids: [dialogue], mode: "rms", target_dbfs: -20}` (speech peaks then land around -6 dBFS).
   - Music under it: base level FIRST with `clip.set {clip_id, fields: {volume: ...}}` (`volume` is linear gain: 0.5 is about -6 dB, 0.25 about -12 dB; setting it after ducking wipes the duck keys) so music alone sits about 8-12 dB under the voice, then `audio.duck {music_id, dialogue_ids, depth_db: -15, ramp_s: 0.4}` (range -12 subtle to -20 strong; use -12 for music-led videos, -18 for talk with busy music).
   - Effects: `audio.normalize {clip_ids: [sfx], mode: "peak", target_dbfs: -10}`; they should punctuate, never cover words.
4. `edit.end`. `audio.analyze` reads the source files and ignores clip gain, so compute the result (analysed level + gain set) rather than re-measuring, confirm the duck keys in `project.get`, and read `mixer.meters` while the user plays a loud passage for the live loudness.
5. Delivery: every export preset in `export.presets` with `loudnorm: true` normalises to -14 LUFS / -1 dBTP on export, which suits `web`. For `podcast` or `broadcast` say that the export's loudness pass targets -14, so the mix itself must be brought to the target and loudness normalisation turned off for that export (Export settings); do not change settings unasked.
6. Never export unasked.

Target: $ARGS

Report back: the target, dialogue and music levels before and after, ducking depth and how many duck keys were written, any clipping left, and that one Undo reverts the mix.
