---
name: b-roll
description: Cover talking-head sections with cutaway footage from your library.
args: "[topic or clip name to use as cutaways]"
---
Add cutaway (B-roll) clips over a talking head to hide jump cuts and illustrate what is said, using only the user's own media. The app has no stock footage search; do not pretend to find any.

1. Call `editor.context`, `timeline.list` and `media.list`. The talking clip: the selected video clip, else the longest clip on the main track (confirm if unclear). Candidate B-roll: library assets other than the talking clip, preferring tags, labels and descriptions that match $ARGS; if $ARGS names a clip, use that one.
2. If the library has no suitable footage, stop and say what is missing ("import clips of X with the Import button or drag them into the library"). Do not invent assets.
3. Find where cutaways help: after cuts (`timeline.list` shows the joins), over long static stretches (`timeline.pacing`), or when the transcript mentions something you have footage of (`transcript.get`, `transcript.search {query}`). Aim for one cutaway every 8-15 seconds, each 2-5 seconds long, never over the first 2 seconds.
4. When the plan has more than 5 cutaways, confirm it in one short list ("00:12 beach clip, 3 s") before placing.
5. Place each one above the talking track so the voice keeps playing: `timeline.place {asset_id, at, in, out, mode: "top"}`. Mute its own sound with `clip.set {clip_id, fields: {volume: 0}}`. For a little life add a slow push-in: `clip.keyframe {clip_id, property: "scale", t: 0, value: 1.0}` and a later keyframe at 1.06.
6. Soften entries that feel jarring with `timeline.add_transition {right_clip_id, kind: "CrossFade", duration: 0.2}`. Keep everything in `edit.begin {label: "B-roll"}` ... `edit.end`.
7. Verify with `render.contact_sheet` over the edited stretch, and check the voice track plays unbroken (`timeline.list`).

Cutaway request: $ARGS

Report back: how many cutaways, which clips, and that one Undo removes them. Name the stretches you could not cover for lack of footage.
