---
name: translate-captions
description: Translate the existing captions into another language, keeping their timing.
args: "<target language, e.g. Spanish>"
---
Replace the caption text with a faithful translation while keeping every cue's timing.

1. Call `editor.context` and `subtitles.get`. If there are no cues, say so and offer `/captions` first. Stop; do not transcribe unasked.
2. Target language comes from $ARGS. If it is empty, ask which language (one short question). If the captions are already in that language, say so and stop.
3. Because `subtitles.set` replaces all cues, keep a copy of the originals: tell the user one Undo brings them back. If they want a permanent copy of the original text, save it with `notes.set {text, append: true}` only when they ask.
4. Translate every cue. Rules: keep `start` and `end` exactly; translate meaning, not word for word; keep names, brands and numbers; keep each cue to two lines of about 42 characters, and tighten the wording rather than changing timing. Preserve line breaks the speaker intended. Reading speed should stay under about 17 characters per second; if a translation is longer, shorten it. Keep tone (formal or casual) consistent.
5. Apply in one group: `edit.begin {label: "Translate captions"}`, `subtitles.set {cues: [{start, end, text}, ...]}` with the whole list, `edit.end`. The caption style is kept.
6. Verify: `subtitles.get` returns the same number of cues with the same times; `render.frame {t}` at one or two cues to check the text fits the frame (some scripts need a smaller size or a different font; say so rather than changing the style unasked).
7. Tell the user you cannot guarantee professional quality for rare languages or specialised terms and that a native speaker should check important videos.

Target language: $ARGS

Report back: the language, the number of cues translated, any lines you shortened, and that one Undo restores the original captions.
