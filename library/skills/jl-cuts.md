---
name: jl-cuts
description: Turn hard cuts into J and L cuts (sound leads or trails the picture) so dialogue flows.
args: "[j, l or auto; lead in seconds, e.g. l 0.6]"
tags: j-cut, l-cut, split edit, dialogue, technique
---
Make cuts feel smooth by letting the sound change before the picture (J cut) or after it (L cut). Only the audio side of the cut moves; the picture cut stays where it is.

1. Call `editor.context` and `timeline.list`. Targets: the cuts between the selected clips, else every cut on the main video track where a linked audio clip is cut at the same time. Skip cuts with a transition, cuts on music and cuts where either side is shorter than 2 s.
2. Pick the kind per cut from $ARGS: `j` = the next clip's sound starts early (good for a new speaker or a new scene: we hear it coming), `l` = the outgoing sound carries on over the next picture (good for reactions and cutaways: we keep hearing the speaker). `auto` (the default): use J where the incoming clip starts with speech, L where the outgoing clip ends mid-thought. With a transcript (`transcript.get {clip_id}`) decide from the words: a cut right before a new sentence is a J, a cut inside a sentence is an L.
3. Offset: from $ARGS, else 0.5 s on footage with natural pauses and 0.3 s on footage that was silence-cut (its pauses are short, so a longer J would pull the end of a word across). Range 0.3-1.0 s. Never move an audio cut into a word: check the new cut time against the transcript (word times are approximate; map them through each piece's `src_in` if the clip was cut; pieces of a cut clip have no transcript of their own, and without any transcript use the quiet ranges from `autocut.detect {clip_ids}` as the safe places) and shift it into the nearest pause of at least 0.15 s. Keep the speaker-change gap you hear at 0.4-0.6 s.
4. Apply in one undo step: `edit.begin {label: "J/L cuts"}`. For each cut at time `c`, take the right-hand AUDIO clip of that cut (same track as its left neighbour) and roll only that cut: L = `timeline.roll {right_clip_id: <audio clip>, cut: c + offset}`, J = `timeline.roll {right_clip_id: <audio clip>, cut: c - offset}`. Rolling keeps both clips' total length, so nothing downstream moves. A roll that is refused (an error such as "roll refused") hit the end of the source media or another clip: try a smaller offset once, then skip that cut and note it. Decide every cut before `edit.begin`: the group closes after 30 s without edits.
5. Smooth the overlap: a very short audio fade helps when the levels differ. `clip.set {clip_id, fields: {fade_in: 0.03}}` on the incoming audio clip and `fade_out: 0.03` on the outgoing one.
6. `edit.end`. Verify with `timeline.list` (picture cuts unchanged, audio cuts offset; fades show in `project.get`, not in `timeline.list`) and `render.contact_sheet {from, to, count: 6}` around two of the cuts. You cannot listen; ask the user to play one.

Request: $ARGS

Report back: how many J and how many L cuts, the offset, any cuts skipped (no handles, transitions, music) and that one Undo restores the hard cuts.
