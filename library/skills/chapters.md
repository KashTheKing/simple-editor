---
name: chapters
description: Find the topic changes and add chapter markers with titles.
args: "[number of chapters, or leave empty for automatic]"
---
Add chapter markers with short titles at the real topic changes.

1. Call `editor.context` and `timeline.list`. Chapters cover the whole main timeline. If a sequence is open, say so and ask which timeline to chapter.
2. Make sure there is a transcript: `transcript.get` with no argument lists every transcribed clip. For each spoken clip without one, tell the user transcription takes a while and run `transcribe.run {clip_id, cues: false}`. If there is no speech at all, use `media.scene_cuts {clip_id}` (detection only, no flags) to find shot changes and say the titles will be generic.
3. Read the text and the timeline times. Find topic changes: a new subject, a "next" or "so now", a long pause (`autocut.detect {clip_ids}` lists long silences without changing anything). Use $ARGS as the target count if it is a number; otherwise one chapter per 1-3 minutes, at least 3, the first at 0:00.
4. Titles: 2-5 words, in the speaker's language, Title Case, no timestamps in the name. Never invent facts the transcript does not support.
5. Call `markers.list` first so you do not duplicate existing chapters. If markers already exist, ask whether to keep them.
6. Add them in one group: `edit.begin {label: "Chapters"}`, then for each chapter `markers.add {t, name: "<title>", author: "Assistant"}`, then `edit.end`.
7. Show the user the list as `0:00 Title` lines. If they plan to upload, offer `markers.export {format: "youtube_chapters", path}`, but write the file only after they give or confirm a path. Video-site chapter lists need the first chapter at 0:00 and at least 3 chapters of 10 seconds or more; mention it if the result breaks that.

Requested count: $ARGS

Report back: the chapter list, and that one Undo removes the markers.
