---
name: description
description: Write a title, description and tags for the video from its transcript (answered in chat).
args: "[platform or tone, e.g. 'short, funny']"
---
Draft publishing text for the finished video. This skill only answers in chat; it edits nothing.

1. Call `editor.context` and `project.summary` (length, name, notes). Read `notes.get` for any brief the user wrote down.
2. Get the words: `transcript.get` with no argument returns every transcribed clip. If there is no transcript, say that writing from the picture alone is a guess, and ask whether to transcribe (`transcribe.run {clip_id, cues: false}` takes a while). If the user declines, use project name, notes, marker names (`markers.list`) and a look at `render.contact_sheet {from: 0, to: <end>, count: 9}`.
3. Understand the video: what it is about, who for, the promise, the strongest moment. Use only what the transcript and pictures support. Never invent facts, numbers, names or links.
4. Write, using $ARGS for platform and tone (default: long-form video site, friendly and clear):
   - 3 title options, each under 60 characters, no clickbait that the video does not deliver.
   - A description: a first line that works alone (the first 125 characters show in search), then 2-4 short lines on what is covered, then a line for links the user will fill in as `[link]`.
   - Chapters if the video is over 3 minutes: build them from `markers.list`; if there are none, suggest running `/chapters` instead of guessing times.
   - 8-15 tags, most specific first, plain words with no hashtags unless the platform asks.
   - For short vertical videos: one 100-character caption and 3-5 hashtags.
5. Keep the user's language. Mention one thing you could not verify (names spelled from audio, claims).
6. Do not call any edit or export tool. If the user likes it, offer to save it with `notes.set {text, append: true}` and wait for a yes before doing so.

Platform or tone: $ARGS

Report back by giving the text itself in chat; end with one line offering to save it to the project notes.
