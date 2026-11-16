---
name: color-match
description: Make clips from different cameras or lighting look consistent.
args: "[reference clip name or id, or empty to use the first selected]"
tags: color, match, continuity, multicam
---
Match the colour of the selected clips to one reference clip so cuts between them do not jump. Matching is step 3 of a grade (after exposure and white balance, before any look); `/color-grade` runs the whole order.

1. Call `editor.context` and `timeline.list`. Targets: the selected video clips. If fewer than two are selected, ask which clips to match. Reference: the clip named in $ARGS; else the first selected clip; else ask. Pick the best-exposed, most neutral shot when the user leaves it to you, and say which is the reference before starting.
2. Look first: `render.frame {t}` at the start of the reference and of each target (use `timeline.list` start times). If a clip is clearly a different scene (night versus day), warn that matching will look unnatural and ask whether to continue.
3. Measure: `frame.stats {t}` on each. It gives histogram and mean, so you can tell which clips are darker or warmer.
4. Match in one group, `edit.begin {label: "Colour match"}` (measure first: it closes after 30 s without edits). First fix exposure that is far off: `playback.seek {t}` into the target clip (`color.auto` samples the frame under the playhead, so a playhead on another clip grades from the wrong picture), then `color.auto {clip_id}`; then `color.match {clip_id, reference_clip_id}` (adds an editable Curves effect). If a clip still looks too warm or cool, refine with `color.primaries {clip_id, temp, tint}` by small steps (temp and tint within about 10).
5. Skin tones that went orange or green are the usual failure: render and look. If a clip got worse, remove its effect with `clip.remove_effect {clip_id, index}` (find the index in `timeline.list`) and try `color.auto` alone instead.
6. Do not apply a Look or LUT unasked. For a before/after, `clip.bypass {clip_ids, on: false}` switches the grade off; switch it back on with `on: true`.
7. `edit.end`. Verify with `render.contact_sheet {from, to, count: 9}` across the cuts between matched clips, and compare `frame.stats` again (means within a few percent; measure at a slightly different time than before, e.g. t + 0.05, or it may return the cached numbers).

Reference requested: $ARGS

Report back: the reference clip, which clips were matched, any that still differ, and that one Undo reverts all of it.
