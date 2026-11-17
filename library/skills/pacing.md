---
name: pacing
description: Measure the cut rhythm and tighten or loosen it toward a target style, without flattening it.
args: "[style: vlog, shorts, drama, documentary, slow, or a target seconds per shot]"
tags: pacing, rhythm, shot length, asl, technique
credit: shot-length figures from Cutting et al. 2010/2011, AVE (Argaw et al. 2022), cut-video (MIT)
---
Check how fast the edit cuts and bring it toward the rhythm the user wants. Measure first, change only with a yes.

Target average shot length (ASL) per style, derived from published shot-length studies (numbers only):

| style | ASL target | notes |
|---|---|---|
| shorts | 1.5-3 s | hook in the first 1-2 s |
| vlog (talking head, jump cuts) | 1.3-2.5 s | after silence removal a cut every ~1.3-1.5 s is typical |
| drama / modern feature | 3-6 s | movie scenes average about 3.8 s |
| documentary / nature | 5-15 s | almost all hard cuts |
| slow / classic | 8-12 s | long takes, let shots breathe |

1. Call `editor.context` and `timeline.list`. Measure the main video track: shot count, total length, ASL (length / shots), the shortest and longest shots. Pick the style from $ARGS (a bare number is the target ASL in seconds); with nothing, report the measurement, name the closest style and stop.
2. Find outliers with `timeline.pacing {short_s, long_s}`: `short_s` = 0.35 x target, `long_s` = 2.5 x target. It lists clips on every track; keep the video ones and compare each one's length with the two limits yourself to tell too long from too short. Long shots are the ones to tighten (or cover); very short ones may be flashes or mistakes.
3. Keep the rhythm alive: real edits vary shot length in runs (slower around emotional or explanatory moments, faster in action), so do not cut everything to the same length. Aim for the target as an average with roughly half to double the target as the normal spread.
4. Plan the changes as a short list: for each long shot, either trim its dull end (`timeline.trim`), remove a dead stretch inside it (for talk: `/cut-silences` or `/remove-fillers`), or cover it with a cutaway (`/b-roll`). For loosening, suggest which cuts to lengthen; do not invent footage. Show the plan and wait for a yes.
5. Apply in one undo step (decide everything first: the group closes after 30 s without edits): `edit.begin {label: "Pacing"}`; shorten a shot's end with `timeline.ripple_trim {clip_id, edge: <new end time>, start: false, ripple: true}` on the VIDEO clip (this ripples every later clip on all tracks), then trim its linked audio clip (same `link` in `timeline.list`) to the same edge WITHOUT ripple: `timeline.ripple_trim {clip_id: <audio>, edge: <same time>, start: false, ripple: false}`. Trims do not carry the linked audio; rippling it a second time doubles the shift and makes clips overlap. Never cut inside a word (check `transcript.get` when there is speech). `edit.end`. Trimming dull ends alone moves the average only a little; a big change of style needs `/cut-silences`, cutaways or a re-cut.
6. Measure again; in `timeline.list` check that no clip starts before the previous one on its track ends (a contact sheet does not show overlaps) and that each video clip and its linked audio share start and end. Verify the picture with `render.contact_sheet {from: 0, to: <end - 0.1>, count: 16}`.

Target: $ARGS

Report back: ASL before and after, shots changed, the longest shot now, and that one Undo restores the edit.
