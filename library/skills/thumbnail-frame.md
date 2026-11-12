---
name: thumbnail-frame
description: Find strong still frames and export one as a thumbnail image.
args: "[where to look, e.g. 'intro', or a time in seconds]"
---
Pick the best frame for a thumbnail and save it as an image. Nothing in the project changes.

1. Call `editor.context` and `project.summary` (length, size). Search range: $ARGS if it gives a time or section; else the in/out range if set (`timeline.in_out`); else the whole timeline. A bare time like `42` means "use this frame".
2. Survey: `render.contact_sheet {from, to, count: 24, width: 320}`; for a long timeline do two passes (24 across the whole video, then 24 across the best area). Look for: a clear face with open eyes and an expressive look, strong contrast, one clear subject, empty room at one side for a title, no motion blur, no mid-blink. Avoid black frames and the first/last 0.5 s.
3. Narrow down to 3 candidates: `render.frame {t, width: 1280}` for each and judge them. Use `frame.stats {t}` if exposure is uncertain (a very dark or washed-out frame loses to a balanced one).
4. Show the user the three candidates with their times and a one-line reason each, and your pick. Ask for a save location: propose a path beside the project, for example `<project folder>\thumbnail.jpg` (get the folder from `project.summary`). Do not write a file before they agree to the name and place, and never overwrite an existing file; pick a numbered name instead.
5. Export: `frame.export {path, t, width: 1920, height: 1080, with_effects: true, quality: 92}`. Use a `.jpg` unless the user asked for PNG. If the project is vertical, keep the project size instead of forcing 16:9 (`width` / `height` left unset).
6. If the user wants text on it, suggest `/title-card` or an HTML layer, then export again. Do not add text unasked.

Where to look: $ARGS

Report back: the chosen time, the file path written, the size, and the two runner-up times. No undo needed since the project was not edited.
