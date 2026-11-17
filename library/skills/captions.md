---
name: captions
description: Transcribe the timeline and add styled, readable captions.
args: "[style, e.g. bold yellow, social, or a language]"
tags: captions, subtitles, transcript, accessibility
credit: readability rules follow published timed-text style guides (line length, reading speed); social style after video-use (MIT)
---
Add readable, well-timed captions to the open timeline, in the style the user asks for.

1. Call `editor.context`. Target clips: the selected clips, else the clips that carry speech on the main track; find them with `timeline.list`. If several unrelated clips could be meant and nothing is selected, ask one short question.
2. Check for an existing transcript with `transcript.get {clip_id}`. If the clip has none, tell the user transcription takes a while, then call `transcribe.run {clip_id, cues: true}` (add `language` if $ARGS names one; `islands: true` for long takes with pauses). For a long clip use `async: true` and poll `transcribe.status {clip_id}` until the state is done or failed. If a model is missing, `transcribe.install {model}` downloads it; mention the size (75-466 MB) and ask first.
3. Call `subtitles.get`. If cues already exist and the user did not ask to redo them, ask before replacing them. If the clip has a transcript but no cues (it was transcribed with `cues: false`), build the cues yourself from the transcript's words with `subtitles.set {cues}` following step 5; word times are in timeline seconds for the clip they belong to.
4. Open the undo group now: `edit.begin {label: "Captions"}` (it closes by itself after 30 s without edits, so do the reading and planning first).
5. Pick the style from $ARGS. Call `gallery.list {tab: "Captions"}` to see the named caption styles and use the closest with `subtitles.style_preset {name}`. For word-by-word emphasis use `subtitles.animation {kind}`: `highlight` (with `color` [r,g,b,a], yellow for "yellow"), `pop` or `typewriter`. With no $ARGS, use a plain high-contrast preset with `kind: "none"`. `social` means short-form style (step 6b).
6. Read the cues and fix them with `subtitles.set {cues}`, keeping each cue's timing on the words:
   a. Long-form (default): at most 2 lines of about 42 characters, one line when it fits; reading speed at most about 20 characters per second (17 for children's content); each cue on screen at least 0.8 s and at most 7 s. Break lines after punctuation or before "and", "but", "to", "of"; never between an article and its noun, a first and last name, or a pronoun and its verb. Make the lower line the longer one.
   b. Social: 1-3 words per cue (about 2), uppercase, no cue shorter than 0.35 s (merge a word into its neighbour instead), break on punctuation and on pauses of 0.3 s or more. A bold outlined preset (`Big Impact`, `Bold Outline` or `Yellow Pop`) with `pop` or `highlight` animation.
   Fix obvious mis-hearings (names, product words). Never change what was said.
7. Keep captions clear of platform UI on vertical video: `guides.list` gives each platform's unsafe zones; captions belong in the lower-middle, above the bottom caption/button area (roughly the lower third's top half on 9:16).
8. `edit.end`.
9. Verify: `render.contact_sheet {from, to, count: 9}` over a spoken stretch, or `render.frame {t}` at a cue. Check the text is inside the frame, does not cover a face, and is readable, also with `render.frame {t, width: 1280}` (small renders can make animated words look as if they overlap). Change the style if not.

Style or language requested by the user: $ARGS

Report back in two lines: how many cues were made, the style used, and that one Undo removes the captions.
