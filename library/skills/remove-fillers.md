---
name: remove-fillers
description: Cut "um", "uh" and other filler words out of a transcribed clip.
args: "[extra words to cut, comma separated]"
---
Remove filler words from speech, cutting between words so nothing sounds clipped.

1. Call `editor.context`. The target is the selected clip; with nothing selected, ask which clip. Only clips with speech make sense.
2. Check `transcript.get {clip_id}`. If there are no words, say transcription is needed (it takes a while), then run `transcribe.run {clip_id, cues: false, islands: true}`; with `async: true`, poll `transcribe.status {clip_id}`. Word-level timing is required.
3. Build the word list. With no `words` the app's default fillers apply. If $ARGS lists extra words or phrases ("you know, like"), pass `words` with the defaults plus those. Warn that "like" and "so" are often real words; include them only if the user named them.
4. Preview: `transcript.remove_fillers {clip_id, words, dry_run: true}`. Report the count and the first few hits with their times. If there are more than about 40 hits, or the list looks wrong, show it and ask before cutting.
5. For a careful review, `transcript.remove_fillers {clip_id, as_markers: true}` marks every hit as a range marker and cuts nothing; offer it when the user wants to decide hit by hit.
6. Apply inside one group: `edit.begin {label: "Remove fillers"}`, `transcript.remove_fillers {clip_id, words}` (leave `pad_ms` at its default unless cuts clip a word; then raise it by 20), `edit.end`. Captions, markers and words shift along automatically.
7. Verify: `timeline.list` for the new length, and `transcript.search {query: "um"}` finds no leftovers. Look at a join with `render.contact_sheet` only if a visible jump cut is worth checking.

Extra words from the user: $ARGS

Report back: how many fillers were cut, time saved, and that one Undo restores them.
