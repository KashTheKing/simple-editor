---
name: cut-silences
description: Remove silences and dead air from the selected clips (preview first).
args: "[tone: aggressive, balanced, gentle or documentary; or a dB threshold]"
tags: silence, jump cut, dead air, talking head, autocut
credit: tone presets and noise-floor threshold adapted from cut-video (MIT) and auto-editor (public domain)
---
Cut the pauses out of talking footage so it moves, without clipping words.

1. Call `editor.context`. Targets are the selected clips; with nothing selected, ask which clip (do not cut the whole timeline unasked). Use `timeline.list` to confirm each target has audio.
2. Set the threshold from the recording, not a fixed number: the goal is a few dB above the room noise between words (`audio.analyze` gives only peak and RMS, so probe). Run `autocut.detect` (step 4) at -40 dB first: if it finds almost no quiet ranges, the room is noisy, so try -35 then -30; if it removes words, go lower. Clean studio or synthetic audio (digital silence between words) works at -40. If no threshold separates speech from the room (speech barely above the noise), say so and prefer word-based cuts with `/remove-fillers` or `transcript.cut_words`. A bare number in $ARGS overrides the threshold.
3. Tone from $ARGS (default `balanced`): how much of a pause survives.

   | tone | `min_silence` | `padding` | feel |
   |---|---|---|---|
   | aggressive | 0.25 | 0.05 | fast jump-cut vlog, a cut every ~1.3 s |
   | balanced | 0.4 | 0.1 | natural, breaths kept short |
   | gentle | 0.8 | 0.2 | interviews, emotional moments keep their space |
   | documentary | 1.5 | 0.35 | only long dead air goes |

   Pauses under 0.15 s are never cut (they are inside phrases). Keep `min_speech` at about 0.15 so single short words survive.
4. Preview with `autocut.detect {clip_ids, threshold_db, min_silence, min_speech, padding}`. It changes nothing; add up its `quiet_ranges` yourself for the time removed (the first and last may just trim the head and tail, which are not joins), and count the joins left between kept ranges. Tell the user both. Talking footage normally loses 25-50%; if it would remove more than about 55% (65% for `aggressive`), say so and ask before continuing: the threshold is probably too high.
5. If a transcript exists (`transcript.get {clip_id}`), spot-check that no kept/removed boundary falls inside a word; keep laughs and deliberate dramatic pauses. Whisper word times can be off by up to about a second (better with `islands: true`), so trust a detected silence over a word time that overlaps it slightly. If a word is really clipped, raise `padding` by 0.05 and detect again.
6. For important or long footage, offer `autocut.mark {clip_ids, threshold_db, min_silence, min_speech, padding}` first: it only drops range markers and cuts nothing, so the user can review.
7. Apply: `edit.begin {label: "Cut silences"}`, then `timeline.auto_cut {clip_ids, threshold_db, min_silence, min_speech, padding, ripple: true}`, then `edit.end`. Linked video follows the audio. Its `removed` count counts pieces on every linked track, not cuts. The transcript stays on the original clip in source time: map words through each piece's `src_in` from `timeline.list` afterwards.
8. Verify with `timeline.list` (new duration, no tiny leftover clips) and `render.contact_sheet {from, to, count: 9}` around a few joins. Jump cuts in the same framing can be hidden with `/zoom-punch` (alternate scale on every other piece) or `/b-roll`; offer these, do not add them unasked.

Setting requested by the user: $ARGS

Report back: time removed (length before and after), number of cuts, the tone and threshold used, and that Undo restores everything in one step.
