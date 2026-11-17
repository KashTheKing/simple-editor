---
name: style-podcast-clips
description: Pull 3-5 standalone clips from a podcast or interview, each with a hook, tight pauses, big captions and clean voice.
args: "[how many clips, length, topic; vertical or horizontal]"
tags: style, podcast, clips, interview, captions, social, preset
credit: parameters derived from cut-video (MIT) and video-use (MIT) caption/loudness rules; AES TD1008 talk loudness
---
Find the best self-contained moments in a long conversation and turn each into a clip people will watch without context.

Style card:

| parameter | value |
|---|---|
| clip length | 30-60 s (vertical) or 60-120 s (horizontal) |
| what makes a clip | one complete idea: a claim, story or answer with a payoff; starts on a strong first sentence, no "so, um, yeah" |
| pauses | `min_silence` 0.35, `padding` 0.08: tight but natural; speaker changes keep 0.4-0.6 s |
| framing | the active speaker on screen; with two cameras, cut to whoever speaks (multicam) |
| hook | the first 1-2 s: the boldest line, or a 2-3 s title with the question being answered |
| captions | social style, `Big Impact` with `pop`, 2-3 words, uppercase |
| audio | `audio.repair` `clarity`, voices matched, -16 LUFS for podcast feeds, -14 for social |

1. Call `editor.context`, `project.summary`, `timeline.list`. Source: the selected clip(s) or the whole timeline. Transcript: `transcript.get`, else `transcribe.run {clip_id, cues: false, islands: true, async: true}` and poll `transcribe.status {clip_id}` (long files take minutes; say so).
2. Read the transcript and pick candidates (count from $ARGS, default 3): rank by a clear hook, a payoff, the user's markers, and energy you can read from the words (questions, exclamations, laughter). For each: start and end word times, length, the hook line, one sentence on why. Show the list and wait for the user to choose.
3. Each chosen clip becomes its own sequence and the source edit stays untouched: `edit.begin {label: "Podcast clip N"}` (decide the ranges first: the group closes after 30 s without edits); place the range again in empty space AFTER the end of the main timeline with `timeline.place {asset_id, at: <timeline end + 2>, in, out, track}` (video and its linked audio come together), on word boundaries, then `timeline.nest {clip_ids, name: "Clip N - <topic>"}`. The nested clip sits after the main edit, never over it. `sequence.open {id}` to edit inside it.
4. Inside: `timeline.auto_cut {clip_ids, threshold_db, min_silence: 0.35, min_speech: 0.15, padding: 0.08, ripple: true}` (threshold probed as in `/cut-silences`). Fillers: remove them by time with `transcript.cut_words {clip_id, ranges}` using the original transcript's word times mapped into the sequence, or skip them. Back on the main timeline, trim the nested clip to its new length with `timeline.trim {clip_id, end}` (it keeps its old length otherwise).
5. Two cameras: `multicam.create {clip_ids}` then `multicam.switch {seq_clip_id, angle, t}` at each speaker change (from the transcript), cutting 0.2 s before the new speaker starts.
6. Vertical (when asked): reframe as in `/reframe` (scale up, centre the speaker, respect `guides.list`).
7. Hook: if the best line is not first, open with a 2-3 s text on a track above the video: `timeline.add_clip {text, at: 0, duration: 2.5, track}` (about 16 characters per line, break with `\n`), `keyblocks.apply {name: "Pop In", clip_ids: [clip_id], at: 0}`.
8. Captions: build the cues AFTER all cuts: the transcript keeps the original clip's source times, so map each word to the new timeline (piece start + word time - piece `src_in`, using the piece that contains it; word times are approximate, so a word that falls just outside a kept piece belongs to the nearest one: snap it in rather than drop it, and drop only words inside a removed stretch) and write them with `subtitles.set {cues}`; then `subtitles.style_preset {name: "Big Impact"}`, `subtitles.animation {kind: "pop"}`; 2-3 words per cue.
9. Audio: `audio.repair {clip_ids, preset: "clarity"}`, `audio.match_loudness {clip_ids}`. `edit.end`, `sequence.open` with no id to go back.
10. Verify each clip: `render.contact_sheet {from: 0, to: <end - 0.1>, count: 9}` inside its sequence; first frame and last word clean.

Request: $ARGS

Report back: each clip's sequence name, length, hook line, and that each was one undo step; offer `/deliver` to export them.
