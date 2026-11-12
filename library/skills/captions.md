---
name: captions
description: Transcribe the timeline and add styled captions.
args: "[style, e.g. bold yellow, or a language]"
---
Add readable, well-timed captions to the open timeline, in the style the user asks for.

1. Call `editor.context`. Target clips: the selected clips, else the clips that carry speech on the main track; find them with `timeline.list`. If several unrelated clips could be meant and nothing is selected, ask one short question.
2. Check for an existing transcript with `transcript.get {clip_id}`. If the clip has none, tell the user transcription takes a while, then call `transcribe.run {clip_id, cues: true}` (add `language` if $ARGS names one; `islands: true` for long takes with pauses). For a long clip use `async: true` and poll `transcribe.status {clip_id}` until the state is done or failed. If a model is missing, `transcribe.install {model}` downloads it; mention the size (75-466 MB) and ask first.
3. Call `subtitles.get`. If cues already exist and the user did not ask to redo them, ask before replacing them.
4. Pick the style from $ARGS. Call `gallery.list {tab: "Captions"}` to see the named caption styles and use the closest with `subtitles.style_preset {name}`. For word-by-word emphasis use `subtitles.animation {kind}`: `highlight` (with `color` [r,g,b,a], yellow for "yellow"), `pop` or `typewriter`. With no $ARGS, use a plain high-contrast preset with `kind: "none"`.
5. Read the cues. Fix obvious mis-hearings (names, product words) with `subtitles.set {cues}`, keeping each cue's `start` and `end`. Keep cues to two short lines; split any cue longer than about 6 seconds.
6. Do the edits of steps 4-5 inside `edit.begin {label: "Captions"}` ... `edit.end`.
7. Verify: `render.contact_sheet {from, to, count: 9}` over a spoken stretch, or `render.frame {t}` at a cue. Check the text is inside the frame, does not cover a face, and is readable. Change the style if not.

Style or language requested by the user: $ARGS

Report back in two lines: how many cues were made, the style used, and that one Undo removes the captions.
