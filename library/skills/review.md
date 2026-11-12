---
name: review
description: Review the edit and leave pacing notes as comments on the timeline.
args: "[focus: pacing, audio, text, or everything]"
---
Act as a second pair of eyes: look at the edit and leave concrete notes as timeline comments. Nothing is cut or changed.

1. Call `editor.context`, `project.summary` and `timeline.list`. Review the main timeline. Focus from $ARGS (default: pacing, audio, text, and look). Note the existing comments with `markers.list {open: true}` so you do not repeat them.
2. Pacing: `timeline.pacing` lists clips that are too long or too short. Add `media.scene_cuts {clip_id}` (detection only) for shot rhythm, and `autocut.detect {clip_ids}` for dead air in talking clips.
3. Audio: `audio.analyze {clip_ids}` and `mixer.meters` for clipping (peak above -1 dBFS), very quiet clips, and uneven levels between clips. Look at music against speech in `timeline.list`.
4. Picture and text: `render.contact_sheet {from: 0, to: <end>, count: 24}` over the whole timeline and look for black frames, jump cuts, off-centre framing, mismatched colour between neighbours, text near the edges or too small to read. `subtitles.get` for caption problems (long lines, cues over 6 s).
5. Write at most 12 notes, most important first. Each is specific, kind and actionable: what, where, suggested fix ("Long static shot, 14 s: cut to a cutaway or trim to 6 s"). No vague praise or "make it better". If something is good, one note at most.
6. Leave them with `edit.begin {label: "Review notes"}`, then `markers.comment {note, t, duration, author: "Assistant"}` for each (use `duration` for a stretch, none for a point), then `edit.end`. Comments are markers with notes; the user ticks them off later.
7. Summarise in chat: the three most valuable fixes, and offer to apply them (`/cut-silences`, `/clean-audio`, ...). Do not apply anything unasked.

Focus requested: $ARGS

Report back: the number of comments left, the top three issues, and that one Undo removes the comments (or the user can delete them in the Comments list).
