---
name: zoom-punch
description: Add quick punch-in zooms on emphasised moments to a talking clip.
args: "[strength: subtle, normal or strong]"
---
Make a talking-head clip feel alive with short punch-in zooms at the key words and at cuts.

1. Call `editor.context`. Target: the selected video clip(s); with nothing selected, ask which clip. Use `timeline.list` for the clip's start, duration and current scale.
2. Strength from $ARGS: `subtle` = 1.05, `normal` (default, also when empty) = 1.10, `strong` = 1.18 scale. Applied as multiples of the clip's current scale.
3. Find moments: if a transcript exists (`transcript.get {clip_id}`), pick emphasised words (numbers, "never", "important", the first word after a pause) and the start of each new sentence; otherwise use `audio.analyze {clip_ids}` onsets and `autocut.detect {clip_ids}` pause ends. Aim for one punch every 4-8 seconds, never two within 3 seconds.
4. Never let the zoom crop a face badly: check one frame with `render.frame {t}` at the strongest zoom, and lower the strength if the head is cut off.
5. Apply in one group, `edit.begin {label: "Zoom punch"}`. For each moment at clip-local time `t`: `clip.keyframe {clip_id, property: "scale", t: t - 0.05, value: base}`, `clip.keyframe {..., t: t + 0.1, value: base * strength, ease: "EaseOut"}`, then hold. Alternate with a return keyframe at the next sentence start (`ease: "EaseInOut"`). Use clip-local seconds (timeline time minus the clip's start). If the clip is split into pieces, only keyframe the piece that covers `t`.
6. Alternative for many cuts: split at each moment with `timeline.split {t}` and `clip.set {clip_id, fields: {scale: ...}}` on every second piece, which gives a hard punch instead of a smooth one. Use it only if the user asks for jump-cut style.
7. `edit.end`. Verify with `render.contact_sheet {from, to, count: 9}` that zooms alternate and stay framed.

Strength requested: $ARGS

Report back: how many punches, the zoom amount, and that one Undo removes the keyframes.
