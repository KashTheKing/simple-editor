---
name: title-card
description: Add an animated full-screen title card (an HTML scene).
args: "<title>, <optional subtitle>   (optional: at 0s, accent #ff5500)"
---
Create a full-frame animated title card as an HTML scene and place it on the timeline.

1. Call `editor.context` and `project.summary`. Parse $ARGS as `Title, Subtitle`. Placement: "at 12s" in $ARGS, else the playhead, else 0. Accent colour from $ARGS or a pleasing default. If the title is missing, ask one short question. Keep the user's wording and capitalisation.
2. Check `timeline.list` at the placement time. A scene is full frame and covers what is below it; if it lands in the middle of a clip, ask whether to insert it (ripple) or overlay it. To insert, split with `timeline.split {t}` and move the later clips with `timeline.move {clip_ids, dt}` by the card's length.
3. Write the HTML following the scene rules in the library README. Use these design rules:
   - `se:kind` scene, `se:size` `1920x1080`, `se:duration` `4s` (4-5 s), `se:desc` and `se:tags` set. A scene paints the whole frame: a solid or gradient background, and `html, body { margin: 0; overflow: hidden; }`.
   - Safe margins: nothing within 120 px of any edge. Centre the block; the title may use at most 80% of the width and wrap to two lines at most.
   - Type: title 110-140 px, weight 700-800, subtitle 40-48 px at 70% opacity, Segoe UI with Arial fallback. Contrast of at least 7:1 between text and background.
   - Motion: entrance 0 to 0.8 s (title rises 30 px and fades, subtitle follows 0.25 s later, an accent line grows from the centre), hold, exit 3.3 to 3.9 s (fade). `animation-fill-mode: both`. The held pose must be a good still in case the user stretches the clip.
   - Fields: `title | text | <Title>`, `subtitle | text | <Subtitle>`, `accent | color | #e6a100`, `background | color | #101418`. Use `{{title}}` etc. in the markup and in a `:root` variable block.
4. Optional: `library.list {kind: "html", query: "title"}`; if one fits, `library.apply {id, at}` and `html.set {clip, fields}`. Your own is fine.
5. `edit.begin {label: "Title card"}`, `html.create {html, at, duration: 4, name: "Title card", fields: {title, subtitle, accent, background}}`, `edit.end`.
6. Verify: `html.check {clip, t: 2}` and look at the image: text inside the safe area, nothing clipped, readable. Also `html.check {clip, t: 0.2}` for the entrance. Fix with `html.set {clip, html}` until it is right.

Title and options: $ARGS

Report back: the time and length, the text used, that title, subtitle and colours are editable in the Inspector, and that one Undo removes the card.
