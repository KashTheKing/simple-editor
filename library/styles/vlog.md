---
name: vlog
title: Fast-cut vlog
description: Talking-head YouTube vlog - jump cuts, punch-ins, B-roll, punchy captions.
avg_shot: 1.9
cut_on_beat: 0.1
transitions: [cut, whip 0.2, flash 0.2]
captions: Bold Outline, highlight, 2-4 words per cue
loudness: -14
grade: luts/punchy-vlog at 0.5
pacing: [1.8, 1.9, 2.0, 1.8]
words_per_minute: 175
skill: style-vlog
---
Energetic and personal: the speaker talks straight to camera and nothing is allowed to sit still for long.

- Cut every pause longer than about 0.3 s; up to half of the raw footage goes.
- Alternate the framing between jump cuts (every second piece scaled about 1.12 and re-centred on the face) so a cut never looks like a glitch.
- Punch in on key words - numbers, "never", "best", the payoff of a sentence - at most one every 5 s.
- One B-roll cutaway every 10-15 s, 2-3 s long, from the user's own library.
- Hard cuts; at most one cut in five gets a quick whip or flash, only for a change of topic.
- Voice cleaned and forward, music ducked about 15 dB under it.

Run `/style-vlog` to have the assistant do the whole pass.
