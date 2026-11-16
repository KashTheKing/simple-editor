---
name: reframe
description: Reframe a horizontal edit for vertical or square (Auto Reframe) keeping the subject in the safe zone.
args: "[9:16, 1:1 or 4:5; optional: track the subject]"
tags: reframe, auto reframe, vertical, 9:16, square, premiere, converted
credit: converted from the Auto Reframe step of open Premiere Pro MCP servers (antipaster/Adobe-Premiere-Pro-MCP, MIT) through /convert-from-editor
---
The Premiere "Auto Reframe Sequence" recipe, done with this app's tools: change the frame, then keep each shot's subject centred and clear of platform UI. This app does not detect faces; you place the framing from rendered frames.

1. Call `editor.context`, `project.summary` and `timeline.list`. Target ratio from $ARGS (default 9:16): 9:16 = 1080x1920, 1:1 = 1080x1080, 4:5 = 1080x1350. Changing the project frame affects the whole project: say so and ask first, unless the user already asked for it. Suggest duplicating the project file first if they want to keep the horizontal version.
2. Look before editing (the undo group closes after 30 s without edits): for every video clip on the picture tracks (skip text, HTML and full-frame graphics, which need their own layout) `render.frame {t}` at its start, middle and end, and note where the subject is (the person or object the shot is about) and whether it moves. The current `scale`, `x` and `y` of each clip are in `project.get` (not in `timeline.list`).
3. `edit.begin {label: "Reframe"}`, then `project.set {width, height}`. For each clip:
   - Fill the new frame: `clip.set {clip_id, fields: {scale}}`. From 16:9 the cover scale is about 3.16 for 9:16, 1.78 for 1:1 and 2.2 for 4:5 relative to "fit"; confirm with `render.frame {t}`: no black bars.
   - Static subject: shift `x` (and `y`) with `clip.set` so it sits on the centre line, faces in the upper-middle third.
   - Moving subject: follow it with a few position keys, one per move, from the frames you looked at: `clip.keyframe {clip_id, property: "Position X", t, value, ease: "EaseInOut"}` (clip-local seconds). Fewer, smooth keys read like a camera operator; one per frame looks jittery. (Do not use `tracking.run` for this: its keys make the clip follow the subject around the frame, the opposite of reframing, and `timeline.reframe` only works on a box tracked in the Inspector.)
4. Safe zones for vertical: `guides.list` gives each platform's UI zones as fractions of the frame; the area clear on all of them is the safe box (roughly the middle 80% of the width, from about 15% below the top to about 35% above the bottom). Faces and on-screen text stay inside it; move captions with the caption style if needed (`/captions social`).
5. `edit.end`. Verify with `render.contact_sheet {from: 0, to: <end - 0.1>, count: 12}` and `guides.preview {platform, t}` on two shots; fix any shot where the subject is cut off.

Request: $ARGS

Report back: the new frame size, how many clips were reframed (and how many follow a moving subject), clips that need a manual look, and that one Undo restores the original frame and framing.
