---
name: shorts
description: Turn a clip into a vertical 9:16 short with captions and a hook.
args: "[start-end in seconds, or what the short is about]"
---
Make a vertical short (about 20-60 seconds) from the user's footage: reframed to 9:16, captioned, with a hook in the first seconds.

1. Call `editor.context` and `project.summary`. Source: the selected clip, else ask. Range: if $ARGS gives `start-end` seconds, use it; if it describes a topic, find it with `transcript.search {query}` (run `transcribe.run {clip_id}` first if there is no transcript); with nothing, pick the strongest 30-45 s stretch from the transcript and say why.
2. The project must be vertical. If it is not 1080x1920, tell the user this changes the whole project's frame and ask whether to change it here. If they agree: `project.set {width: 1080, height: 1920}`. Never do this silently.
3. Open the undo group: `edit.begin {label: "Short"}`. Trim to the range: `timeline.split {t}` at both ends, then `timeline.extract {a, b}` for the parts outside it. Cut between words, never mid-word (use `transcript.get` word times).
4. Reframe the video: horizontal footage must be scaled up to fill the tall frame. Use `clip.set {clip_id, fields: {scale: ...}}` (roughly 1.8 to 3.2 for 16:9 into 9:16) and shift `x` to centre the speaker. If the clip already has a tracked box, `timeline.reframe {clip_id}` can do it; the app cannot detect faces itself and the call errors cleanly when there is no box. Check with `render.frame {t}` and adjust `x` / `y`. For a moving subject, `clip.keyframe {clip_id, property: "x", t, value}`.
5. Captions: `transcript.get`, then `subtitles.style_preset {name}` (see `gallery.list {tab: "Captions"}`) and `subtitles.animation {kind: "pop"}`. Keep them in the lower middle, clear of the bottom 15% of the frame.
6. Hook: pick the most striking sentence in the range. If it is not already first, either start the short on it or add a text clip with `timeline.add_clip {text, at: 0, duration: 2.5}`, styled with `clip.set` (size, color) and `text.animate {clip_id, preset}`. Keep it under 8 words.
7. Verify: `render.contact_sheet {from: 0, to: <end>, count: 12}`. Check the face is in frame, text is not cut off, and the first 2 seconds grab attention. Then `edit.end`.

What the user asked for: $ARGS

Report back: the range used, its length, whether the project format changed, and that one Undo reverts the short.
