---
name: color-match
description: Make clips from different cameras or lighting look consistent.
args: "[reference clip name or id, or empty to use the first selected]"
---
Match the colour of the selected clips to one reference clip so cuts between them do not jump.

1. Call `editor.context` and `timeline.list`. Targets: the selected video clips. If fewer than two are selected, ask which clips to match. Reference: the clip named in $ARGS; else the first selected clip; else ask. Say which is the reference before starting.
2. Look first: `render.frame {t}` at the start of the reference and of each target (use `timeline.list` start times). If they differ in framing a lot, still match; if a clip is clearly a different scene (night versus day), warn that matching will look unnatural and ask whether to continue.
3. Measure: `frame.stats {t}` on each. It gives histogram and mean, so you can tell which clips are darker or warmer.
4. Match in one group, `edit.begin {label: "Colour match"}`. For each target: `color.match {clip_id, reference_clip_id}` (adds an editable Curves effect). If a clip still looks too warm or cool, refine with `color.primaries {clip_id, temp, tint}` by small steps (temp and tint within about 10). If the exposure is off, `color.auto {clip_id}` writes an editable Levels and Color pair.
5. Skin tones that went orange or green are the usual failure: render and look. If a clip got worse, remove its effect with `clip.remove_effect {clip_id, index}` (find the index in `timeline.list`) and try `color.auto` instead.
6. Do not apply a Look or LUT unasked. `clip.bypass {clip_ids, on: true}` temporarily disables the grade if the user wants to compare; set it back afterwards.
7. `edit.end`. Verify with `render.contact_sheet {from, to, count: 9}` across the cuts between matched clips, and compare `frame.stats` again.

Reference requested: $ARGS

Report back: the reference clip, which clips were matched, any that still differ, and that one Undo reverts all of it.
