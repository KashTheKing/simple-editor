---
name: intro-outro
description: Add a short intro and outro (title, fade and end card) to the video.
args: "[title text, optionally 'outro: end text']"
---
Frame the video with a short intro and an outro, built from the app's own title and HTML scenes.

1. Call `editor.context`, `project.summary` and `timeline.list`. Note the project size and the total length. Read $ARGS: the first text is the title; text after `outro:` is the closing line (default "Thanks for watching"). If $ARGS is empty, ask for the title (one short question).
2. Check for existing intros: `markers.list` and the first 5 seconds in `timeline.list`. If something similar is already there, ask before adding another.
3. Pushing the video later would break its timing, so tell the user the intro (3-4 s) shifts everything right. Then `edit.begin {label: "Intro and outro"}`.
4. Make room: `timeline.shift_time {from: 0, dt: 4}` shifts markers, cues and in/out; move the clips with `timeline.move {clip_ids: <all clips>, dt: 4}`. Do the move once, over every track (clip ids from `timeline.list`).
5. Intro: a full-frame scene at time 0 with `html.create {html, at: 0, duration: 4, name: "Intro"}`. Follow the HTML scene rules in the library README (no JavaScript, CSS animation, `se:kind` scene, `se:field`s for the title and an accent colour). Keep it simple: background, the title large in the middle, a short fade/slide entrance. If a built-in title is enough, `titles.list` then `titles.place {name, at: 0}` and edit the text with `clip.set`.
6. Outro: same approach at the end of the timeline (use the old total length plus 4), duration 5 s, closing line plus an accent bar. Add `timeline.add_transition {right_clip_id, kind: "FadeToColor", duration: 0.5}` into and out of the scenes if they butt up against video.
7. `html.check {clip}` on both and look at the images; fix overlaps or unreadable text. `edit.end`. Then `render.contact_sheet {from: 0, to: <end>, count: 12}`.

Texts from the user: $ARGS

Report back: the intro and outro lengths, that all clips moved by 4 s, and that one Undo restores the original timeline.
