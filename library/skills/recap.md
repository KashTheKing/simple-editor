---
name: recap
description: Build a short "previously on" / recap montage from the key moments of a longer edit.
args: "[length, e.g. 30s; and what to focus on]"
tags: recap, montage, summary, previously on, technique
---
Make a quick recap that reminds the viewer of the story so far: key lines, key images, fast but clear.

1. Call `editor.context`, `project.summary`, `timeline.list` and `markers.list`. Source: the selected clips or In/Out range, else the whole timeline. Length from $ARGS (default 30 s; 15-45 s is normal). The user's own markers are picks: use them first.
2. Find the beats of the story. With speech: `transcript.get` (run `transcribe.run {clip_id, cues: false}` first if there is none; it takes a while, say so) and pick 4-8 complete lines that carry the plot: setups, decisions, reveals. Use `transcript.search {query}` for topics named in $ARGS. Without speech: `media.scene_cuts {clip_id}` (ignore a cut reported at the very first frame) for visual high points.
3. Order: chronological. Each line keeps its full sentence (never cut mid-word: use word times, and end each line about 0.2 s after its last word's end time, since word ends are early); between lines add 1-2 s picture-only moments from the same section for breathing room. The recap speeds up slightly toward the end.
4. Show the plan (times, the line, why) and wait for a yes when the source is longer than five minutes.
5. Build it in empty space AFTER the end of the main timeline, so the original edit is untouched and nothing plays under it: `edit.begin {label: "Recap"}` (decide the moments first: the group closes after 30 s without edits); for each moment `timeline.place {asset_id, at, in, out, track}` on the main tracks, back to back (asset and source times from `timeline.list`: source time = timeline time - clip start + `src_in`). Short `timeline.add_transition {right_clip_id, kind: "CrossFade", duration: 0.25}` between non-consecutive moments, or `FadeToColor` 0.3 s for a stylised "memory" feel. Optional title: `titles.place {name: "Title Card", at}` and set its text with `clip.set {clip_id, fields: {text}}` ("Previously" or the user's text).
6. Level the lines with `audio.match_loudness {clip_ids}`, then `timeline.nest {clip_ids, name: "Recap"}` (in this order: after nesting the leveling changes nothing); tell the user they can drag the nested recap to the start. `edit.end`.
7. Verify: `timeline.list` for the length and `render.contact_sheet {from, to, count: 12}` over the recap.

Request: $ARGS

Report back: the number of moments, the length, where it was placed, and that one Undo removes it.
