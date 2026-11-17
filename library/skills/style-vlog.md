---
name: style-vlog
description: Edit talking-head footage in the fast-cut YouTube vlog style - jump cuts, punch-ins, B-roll, punchy captions.
args: "[intensity: chill, normal or hyper]"
tags: style, vlog, youtube, jump cut, talking head, preset
credit: parameters derived from cut-video (MIT) pacing targets, video-use (MIT) caption and loudness rules, and online-video transition statistics (ClipShots)
---
Apply the fast-cut vlog style to the user's talking footage. Everything is done with this app's tools in one undo step, after the user approves a short plan.

Style card (`normal`; `chill` / `hyper` in brackets):

| parameter | value |
|---|---|
| pauses | `timeline.auto_cut` with `min_silence` 0.3 (0.5 / 0.2), `padding` 0.07 (0.1 / 0.05), threshold = noise floor + 6 dB |
| rhythm | a cut every 1.3-2.5 s after the silence pass; up to 50% of runtime removed (40 / 60) |
| jump cuts | alternate framing: every 2nd piece scaled 1.12 (1.08 / 1.18), re-centred on the face |
| emphasis | a punch-in (`Zoom Punch` block) on key words, at most one per 5 s (8 / 3) |
| B-roll | one cutaway every 10-15 s, 2-3 s long, from the user's library only |
| transitions | hard cuts; at most 1 in 5 cuts gets a quick 0.2 s effect (whip or flash) for section changes |
| captions | yes for `normal` and `hyper`: `Bold Outline` with `highlight`, 2-4 words per cue |
| look | `luts/punchy-vlog` at 0.5 after exposure fixes |
| audio | voice cleaned, music ducked -15 dB, mixed for -14 LUFS |

1. Call `editor.context`, `project.summary`, `timeline.list`, `media.list`. Target: the selected talking clip(s), else the main-track clips with speech. Get or make a transcript (`transcript.get {clip_id}`; else `transcribe.run {clip_id, cues: false, islands: true}`, say it takes a while).
2. Measure (all of this before the undo group, which closes after 30 s without edits): probe the threshold with `autocut.detect {clip_ids, threshold_db, min_silence, min_speech: 0.15, padding}` at -40, then -35 / -30 if it finds few quiet ranges (as in `/cut-silences`). Plan the B-roll spots and punch-in words from the transcript now. If the look or transitions are library items that are not installed (`library.list {kind, query}` shows `installed`), they need `gallery.connect {}` and `library.install {id}`: these download, so ask, or leave them out. Show the plan: time removed, cut count, B-roll spots, captions yes/no, look. Wait for a yes.
3. `edit.begin {label: "Vlog style"}`.
4. Cut: `transcript.remove_fillers {clip_id, words: ["um", "uh", "erm"]}` (the default list also cuts "like" and "kind of", often real words), then `timeline.auto_cut {clip_ids, threshold_db, min_silence, min_speech: 0.15, padding, ripple: true}`.
5. Jump-cut framing: `timeline.list`, then for every 2nd resulting piece of the same shot `clip.set {clip_id, fields: {scale, x, y}}` with the card's scale (multiply the current scale; shift `x` / `y` so the face stays centred - check with `render.frame {t}`).
6. Emphasis: on the strongest words (numbers, "never", "best", the payoff of a sentence) `keyblocks.apply {name: "Zoom Punch", clip_ids: [piece], at}`, respecting the spacing. After the cut the transcript stays on the original clip in source time: a word's new timeline time = piece start + (word source time - piece `src_in`), using the piece that contains it.
7. B-roll: where the transcript names something the library has, `timeline.place {asset_id, at, in, out, mode: "top"}` for 2-3 s from the word (mapped as in step 6), `clip.set {clip_id, fields: {volume: 0}}`; `mode: "top"` makes a new track for the first one, then pass that `track` for the rest.
8. Section changes (a new topic in the transcript): optional `library.apply {id: "transitions/whip-pan-left", clip: <right clip>}` or `transitions/flash`, at most 1 cut in 5.
9. Captions (unless `chill`): `subtitles.style_preset {name: "Bold Outline"}`, `subtitles.animation {kind: "highlight", color: [255, 214, 0, 255]}`; cues from the transcript, 2-4 words each, at least 0.35 s.
10. Look and sound: for exposure, `playback.seek {t}` to the clip's middle and `color.auto {clip_id}` (it samples the frame under the playhead). Then the LUT if installed: `library.apply {id: "luts/punchy-vlog", clips}`, and lower it to half strength with `clip.effects_bulk {clip_ids, index, params: {Intensity: 0.5}}` (index = the LUT's place in the clip's effect list in `timeline.list`; group clips that share the same index). `audio.repair {clip_ids, preset: "clarity"}`, music `audio.duck {music_id, dialogue_ids, depth_db: -15}` if there is music.
11. `edit.end`. Verify: `render.contact_sheet {from: 0, to: <end - 0.1>, count: 16}` (framing alternates, faces in frame, captions readable), and the new average shot length from `timeline.list`.

Intensity: $ARGS

Report back: length before and after, cuts, punch-ins, B-roll placed (and spots without footage), captions, look, and that one Undo restores the original.
