---
name: style-documentary
description: Edit interviews and footage in a calm documentary style - long takes, L-cuts into B-roll, natural pauses, muted grade.
args: "[tone: observational, interview or nature]"
tags: style, documentary, interview, b-roll, slow, preset
credit: parameters derived from documentary shot-boundary statistics (BBC Planet Earth annotations: ~97% hard cuts), shot-length studies (Cutting et al.), cut-video (MIT) documentary tone
---
Apply a documentary style: let people speak, let images breathe, and carry the voice over the pictures it describes.

Style card:

| parameter | value |
|---|---|
| pauses | only dead air goes: `min_silence` 1.5, `padding` 0.35 (thoughtful pauses stay) |
| rhythm | average shot 5-15 s (`nature` 8-15, `interview` 5-10); vary it, never uniform |
| interview cutaways | B-roll over 40-60% of interview time, each 4-8 s, entered on the noun being described |
| split edits | L-cuts out of interview into B-roll, J-cuts into a new speaker or place, 0.5-1 s |
| transitions | hard cuts (~97%); a dissolve (0.5-1 s) only for a change of time or place |
| captions | off by default; lower thirds for each new speaker (`/lower-third`) |
| look | natural: exposure and balance per shot, matched; `luts/muted-documentary` at 0.4 |
| audio | dialogue -20 dBFS RMS, ambience kept under it, music only between interview lines, ducked -18 dB |

1. Call `editor.context`, `project.summary`, `timeline.list`, `media.list`, `markers.list`. Interviews: clips with speech (get transcripts with `transcript.get`, or `transcribe.run {clip_id, cues: false, islands: true}`). Footage: the other library assets.
2. Plan and show it (wait for a yes): sentences kept per interview (the user's markers first, then complete thoughts; never cut inside a sentence), B-roll spots matched to what is said, where music sits, the speakers that need a lower third.
3. `edit.begin {label: "Documentary style"}` (plan first: the group closes after 30 s without edits).
4. Interviews: first `transcript.remove_fillers {clip_id, words: ["um", "uh"]}` (it needs the clip's own transcript, which cut pieces do not have), then `timeline.auto_cut {clip_ids, threshold_db, min_silence: 1.5, min_speech: 0.3, padding: 0.35, ripple: true}` (threshold probed with `autocut.detect` as in `/cut-silences`, starting at -40).
5. B-roll: `timeline.place {asset_id, at, in, out, mode: "top"}` for the first cutaway (it makes a new track), then `track` for the rest, over the interview, 4-8 s each, starting on the described word (after the cut, a word's timeline time = piece start + word time - piece `src_in`). Cutaways with their own sound: keep it low (`clip.set {clip_id, fields: {volume: 0.2}}`) for texture. Return to the speaker for emotional lines.
6. Split edits at speaker and scene changes: on the audio clip right of the cut, `timeline.roll {right_clip_id, cut}` by 0.5-1 s (L: later, J: earlier), never inside a word.
7. Transitions: none, except `timeline.add_transition {right_clip_id, kind: "CrossFade", duration: 0.8}` where the story jumps in time or place.
8. Grade: `color.auto {clip_id}` on badly exposed shots, `color.match {clip_id, reference_clip_id}` per scene, then the look at low strength (`/color-grade muted-documentary`).
9. Audio: `audio.role` the clips, `audio.normalize {clip_ids: [dialogue], mode: "rms", target_dbfs: -20}`, music `audio.duck {music_id, dialogue_ids, depth_db: -18, ramp_s: 0.8}` (slow ramps suit the style).
10. `edit.end`. Verify with `render.contact_sheet {from: 0, to: <end - 0.1>, count: 16}` and the average shot length (total / shots on the main track). Short interviews with few cutaways land below 5 s; say so rather than padding shots.

Tone: $ARGS

Report back: interview time kept, B-roll coverage, split edits, the average shot length, and that one Undo restores the original.
