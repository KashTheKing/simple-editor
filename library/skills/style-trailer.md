---
name: style-trailer
description: Cut a cinematic three-act trailer from your footage - setup, rising conflict, a fast montage to the title, then a button.
args: "[length, e.g. 60s or 2m; title text; genre: action, drama, horror, comedy]"
tags: style, trailer, cinematic, teaser, three act, preset
credit: structure from common trailer practice (three acts and a final button); shot-length bands from Cutting et al. 2011 and AVE (Argaw et al. 2022), numbers only
---
Make a trailer that sells the footage: establish, complicate, accelerate, title, button.

Style card (for a 90 s trailer; scale the acts to the requested length):

| act | share of length | average shot | content |
|---|---|---|---|
| 1 setup | 30% | 3-5 s | the world and the main person; one key line of dialogue; calm music |
| 2 conflict | 35% | 2-3 s | the problem appears; dialogue lines separated by black or title cards; music builds |
| 3 montage | 25% | 0.5-1.5 s, getting shorter | the biggest moments, cut on beats, to the peak |
| title + button | the rest (about 8 s) | - | 1 s black, the title card (3-4 s), then a 2-4 s "button": one last joke, scare or line |

Genre: `action` and `horror` run act 3 faster (0.4-1 s shots); `drama` keeps 1.5-2 s even in act 3; `comedy` ends on the funniest line as the button. Look: `luts/teal-and-orange` (action), `luts/cool-noir` (drama, horror), `luts/warm-film` (comedy), at 0.6 strength (see step 8). Transitions: hard cuts and dips to black (`FadeToColor` 0.3-0.5 s) between acts; flashes (`transitions/flash`) on act 3 hits.

1. Call `editor.context`, `project.summary`, `timeline.list`, `media.list`, `markers.list`. Length, title and genre from $ARGS (default 60 s, ask for the title if missing). Music: an audio asset tagged Music, else ask (the app has no music library).
2. Find material (all before the undo group, which closes after 30 s without edits; transcription takes minutes): dialogue lines from transcripts (`transcript.get`; `transcribe.run {clip_id, cues: false}` if needed): short, intriguing lines without spoilers. Visual high points from `media.scene_cuts {clip_id}` (ignore a reported cut at the very first frame) and `audio.beats` onsets on the clips' sound. The user's markers come first.
3. Beats: `audio.beats {clip_ids: [music]}` for BPM and onsets; act 3 cuts land on beats (one frame early).
4. Show the plan per act (shots, the lines used, the title, the button) and wait for a yes.
5. Build in empty space AFTER the end of the main timeline so the source edit stays: `edit.begin {label: "Trailer"}`; place shots in order from there with `timeline.place {asset_id, at, in, out, track}`; lines keep their sound, montage shots are muted (`clip.set {clip_id, fields: {volume: 0}}`). Between acts `timeline.add_transition {right_clip_id, kind: "FadeToColor", duration: 0.4}`. When done, `timeline.nest {clip_ids, name: "Trailer"}` over all its clips.
6. Title: `library.apply {id: "html/intro-title-cinematic", at}` then `html.set {clip, fields}` with the title (or `titles.place {name: "Title Card", at}` and its text with `clip.set {clip_id, fields: {text}}`); trim it to 3-4 s with `timeline.trim {clip_id, end}` (templates are longer), on black.
7. Sound: music under everything, `audio.duck {music_id, dialogue_ids, depth_db: -12, ramp_s: 0.2}` so lines cut through; let the music hit its peak on the last act-3 shot and cut to silence for the title.
8. Look: the genre LUT on every shot (`library.apply {id, clips}`; not installed = `gallery.connect {}` + `library.install {id}`, a download, so ask in step 4), then `clip.effects_bulk {clip_ids, index, params: {Intensity: 0.6}}` (index = the LUT's place in the effect list).
9. `edit.end`. Verify: `render.contact_sheet {from: 0, to: <end - 0.1>, count: 16}`: act 3 visibly faster, title readable, no spoilers. Measure the average shot length per act from `timeline.list`.

Request: $ARGS

Report back: length, shots per act and their average length, the lines used, the title and button, and that one Undo removes the trailer.
