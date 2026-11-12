---
name: lower-third
description: Add an animated name and role caption over the video.
args: "<Name>, <Role>   (optional: at 12s, accent #ff5500)"
---
Add a transparent HTML lower third with the person's name and role, animated in and out.

1. Call `editor.context`. Placement time: "at 12s" in $ARGS, else the playhead, else just after the speaker first appears (default 1 second into the clip). Parse $ARGS as `Name, Role`; an accent colour like `#ff5500` or a plain colour word is optional. If there is no name, ask one short question. Keep the name and role exactly as typed.
2. Check `timeline.list` for overlapping text or lower thirds at that time (do not stack them) and for the video resolution (`project.summary`).
3. Write the HTML following the layer rules in the library README. Use these design rules:
   - `se:kind` layer, `se:size` `1920x1080` (position matters), `se:duration` `5s`, `se:desc` and `se:tags` set, `html, body { margin: 0; background: transparent; overflow: hidden; }`.
   - Safe margins: keep the box 120 px from the left edge and 150 px from the bottom edge, never closer than 90 px to any side.
   - Type: name 56-64 px Segoe UI Semibold, role 32-36 px Segoe UI, always with fallbacks (`"Segoe UI", Arial, sans-serif`). One line each, `max-width` about 900 px, text-overflow ellipsis.
   - Readable on any footage: a dark backing bar (rgba 0,0,0,0.65) with an accent stripe 8 px wide, and a soft text shadow.
   - Motion: entrance at 0 to 0.6 s (slide 40 px + fade, `cubic-bezier(.2,.8,.2,1)`), hold, exit from 4.3 to 4.9 s, all with `animation-fill-mode: both`. Stagger the role 0.15 s after the name. Total 4-5 s; the end pose must be hidden (opacity 0).
   - Fields: `<meta name="se:field" content="name | text | <Name>">`, `role | text | <Role>`, `accent | color | #e6a100` (or the user's colour). Use `{{name}}`, `{{role}}`, `{{accent}}` in the markup and CSS.
4. Optional: `library.list {kind: "html", query: "lower third"}` and, if a good one exists, `library.apply {id, at}` then edit its fields with `html.set {clip, fields}`. Writing your own is fine.
5. `edit.begin {label: "Lower third"}`, `html.create {html, at, duration: 5, name: "Lower third", fields: {name, role, accent}}`, `edit.end`.
6. Verify: `html.check {clip, t: 2}` and look at the image. Confirm text is not clipped, the bar clears the safe margin, and it reads on that footage (also `render.frame {t: at + 2}`). Fix and call `html.set {clip, html}` until it does.

Name, role and options: $ARGS

Report back: the time placed, the text used, and that the name, role and colour are editable in the Inspector; one Undo removes the layer.
