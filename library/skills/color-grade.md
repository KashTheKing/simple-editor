---
name: color-grade
description: Grade the edit in the colourist's order - normalise, balance, match, then a creative look.
args: "[look, e.g. teal-and-orange, warm film, noir; or 'correct only']"
tags: color, grade, look, lut, colourist, technique
credit: order follows the standard colourist workflow taught in Blackmagic's colour guides (paraphrased)
---
Grade the timeline the way colourists do: fix each shot, make them match, and only then add the look. A look on uncorrected footage looks different on every shot.

1. Call `editor.context`, `timeline.list` and `project.summary`. Targets: the selected video clips, else every video clip on the main track. Skip text, HTML and visualizer clips. Look at a few frames first: `render.contact_sheet {from: 0, to: <end - 0.1>, count: 12}`.
2. Measure before changing anything (the undo group closes after 30 s without edits, so look first): `frame.stats {t}` at each clip's middle (after an edit, measure at a slightly different time, e.g. t + 0.05, since a repeated time can return the cached old numbers). Pick the hero shot (step 5) and the look (step 7) now. If the look is a library LUT, `library.list {kind: "lut", query}` shows whether it is `installed` (it lists nothing until the online library is on: `gallery.connect {}`); one that is not needs `library.install {id}`, a download, so ask first.
3. `edit.begin {label: "Colour grade"}`. Exposure: clips with mean far off mid-grey or clipped highlights get `playback.seek {t}` to their middle, then `color.auto {clip_id}` (it samples the frame under the playhead; an editable Levels + Color pair). Leave well-exposed shots alone.
4. Balance white and contrast: for casts, nudge `color.primaries {clip_id, temp, tint}` in small steps (within about 10); for flat footage, a gentle contrast with `color.primaries {clip_id, lift_r, lift_g, lift_b, gain_r, gain_g, gain_b}` (lift slightly down, gain slightly up, equal on all three channels). Check skin tones with `render.frame {t}`: neither orange nor green.
5. Match to the hero shot (the best-looking main angle; say which). For every other shot of the same scene: `color.match {clip_id, reference_clip_id}`. Different scenes are not matched to each other.
6. Secondaries only when asked or clearly needed (a sky, a shirt that distracts): `color.qualifier {clip_id, hue, hue_width, sat_min, sat_max, lum_min, lum_max, softness}`.
7. Look last, the same on every shot, at partial strength: from $ARGS pick a library LUT (e.g. `luts/teal-and-orange`, `luts/warm-film`, `luts/cool-noir`, `luts/muted-documentary`) and add it after the corrections with `library.apply {id, clips: [...]}`; it goes on at full strength, so set it to 0.5-0.7 with `clip.effects_bulk {clip_ids, index, params: {Intensity: 0.6}}` (index = the LUT's place in the clip's effect list in `timeline.list`; group clips that share the same index). Do not use the built-in Looks (`looks.apply` or `gallery.apply {tab: "Looks"}`) here: they replace the clip's whole effect stack, corrections included. With `correct only` in $ARGS, skip this step.
8. `edit.end`. Verify with `render.contact_sheet` again over the same times and compare: shots should cut together without jumps. `clip.bypass {clip_ids, on: false}` shows the ungraded version for a before/after; switch it back with `on: true`.

Look requested: $ARGS

Report back: the hero shot, how many shots were corrected and matched, the look and its strength, anything that still jumps, and that one Undo reverts the grade.
