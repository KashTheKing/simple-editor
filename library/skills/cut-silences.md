---
name: cut-silences
description: Remove silences and dead air from the selected clips (preview first).
args: "[how tight: loose, normal or tight, or a dB threshold]"
---
Cut the pauses out of talking footage so it moves, without clipping words.

1. Call `editor.context`. Targets are the selected clips; with nothing selected, ask which clip (do not cut the whole timeline unasked). Use `timeline.list` to confirm each target has audio.
2. Choose settings from $ARGS: `loose` = `threshold_db` -40, `min_silence` 0.8, `padding` 0.15; `normal` (the default, also when $ARGS is empty) = -35, 0.5, 0.1; `tight` = -32, 0.3, 0.06. A bare number in $ARGS is the `threshold_db`.
3. Preview with `autocut.detect {clip_ids, threshold_db, min_silence, min_speech, padding}`. It changes nothing. Tell the user how much time would be removed and how many cuts. If it would remove more than about 40% of the clip, say so and ask before continuing; the threshold is probably too high.
4. If a transcript exists (`transcript.get {clip_id}`), spot-check that no kept/removed boundary falls inside a word. If one does, raise `padding` and detect again.
5. If the footage is important or long, offer `autocut.mark {clip_ids, ...same settings}` first: it only drops range markers and cuts nothing, so the user can review.
6. Apply: `edit.begin {label: "Cut silences"}`, then `timeline.auto_cut {clip_ids, threshold_db, min_silence, min_speech, padding, ripple: true}`, then `edit.end`. Linked video follows the audio.
7. Verify with `timeline.list` (new duration, no tiny leftover clips) and `render.contact_sheet {from, to, count: 9}` around a few joins. If cuts feel abrupt over music or a continuous shot, say so; a short `timeline.add_transition` is an option, but do not add one unasked.

Setting requested by the user: $ARGS

Report back: time removed (length before and after), number of cuts, the settings used, and that Undo restores everything in one step.
