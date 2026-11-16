---
name: b-roll
description: Cover talking-head sections with cutaway footage from your library.
args: "[topic or clip name to use as cutaways]"
tags: b-roll, cutaway, talking head, jump cut
---
Add cutaway (B-roll) clips over a talking head to hide jump cuts and illustrate what is said, using only the user's own media. The app has no stock footage search; do not pretend to find any.

1. Call `editor.context`, `timeline.list` and `media.list`. The talking clip: the selected video clip, else the longest clip on the main track (confirm if unclear). Candidate B-roll: library assets other than the talking clip, preferring tags, labels and descriptions that match $ARGS; if $ARGS names a clip, use that one.
2. If the library has no suitable footage, stop and say what is missing ("import clips of X with the Import button or drag them into the library"). Do not invent assets.
3. Find where cutaways help, in this order: (a) over jump cuts (joins inside the same shot in `timeline.list`), (b) when the transcript names something you have footage of (`transcript.get`, `transcript.search {query}`): start the cutaway on the word itself, (c) over long static stretches (`timeline.pacing`). Aim for one cutaway every 8-15 seconds, each 2-4 seconds long. Keep the speaker on screen for the first sentence, for emotional or emphatic lines, and for the last line before a cut to a new topic.
4. When the plan has more than 5 cutaways, confirm it in one short list ("00:12 beach clip, 3 s") before placing.
5. Place each one above the talking track so the voice keeps playing: `timeline.place {asset_id, at, in, out, mode: "top"}` for the first (it makes a new track), then `timeline.place {asset_id, at, in, out, track}` on that track for the rest. Use a moving part of the source (`in` past any static start). Mute its own sound with `clip.set {clip_id, fields: {volume: 0}}`, or keep it at about 0.15 for room tone when it helps. For a little life on still or static shots add a slow push-in: `clip.keyframe {clip_id, property: "Scale", t: 0, value: 1.0}` and a later keyframe at 1.06.
6. Cut on action where you can: start a cutaway when something moves in it. Use hard cuts by default; soften only entries that feel jarring with `timeline.add_transition {right_clip_id, kind: "CrossFade", duration: 0.2}`. Keep everything in `edit.begin {label: "B-roll"}` ... `edit.end`.
7. Verify with `render.contact_sheet` over the edited stretch, and check the voice track plays unbroken (`timeline.list`).

Cutaway request: $ARGS

Report back: how many cutaways, which clips, and that one Undo removes them. Name the stretches you could not cover for lack of footage.
