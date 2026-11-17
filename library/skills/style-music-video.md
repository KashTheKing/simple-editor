---
name: style-music-video
description: Cut your clips to a song, music-video style - shots on the beat, faster in the chorus, flashes and speed ramps on hits.
args: "[energy: mellow, normal or hype]"
tags: style, music video, montage, beat, edit to music, preset
credit: beat/bar ratios and cut anticipation from a CC BY study of official music videos (arXiv 2108.00970); transition mix from online-video statistics (ClipShots)
---
Build a montage that moves with the music, from the user's song and clips.

Style card (`normal`; `mellow` / `hype` in brackets):

| parameter | value |
|---|---|
| cut grid | every 2 beats (every 4 = one per bar / every beat) |
| sections | double the rate in the loudest sections (chorus, drop), halve it in quiet intros and breakdowns |
| timing | cuts one frame (~0.03 s) before the beat |
| shot use | the moving part of each clip; never the same clip twice in a row; best shots saved for the chorus |
| accents | on the strongest hits: a `Zoom Punch` block or a 0.15 s flash transition (none / every 2nd bar) |
| speed | a `Montage` speed ramp on 1 shot in 8 (none / 1 in 4) |
| transitions | 80% hard cuts, the rest flash, whip or glitch from the shader library |
| look | one look for the whole video: `luts/teal-and-orange` (`luts/pastel-dream` / `luts/neon-night`) |

1. Call `editor.context`, `timeline.list`, `media.list`. Music: the selected audio clip, else the asset tagged Music or the only song (ask if unclear). Clips: the selected clips, else all video assets in the library except the talking ones; if there are fewer than about 6 clips, say the montage will repeat footage and ask.
2. Put the music on the timeline first if it is only in the library: `timeline.place {asset_id, at: 0, track}` (beat detection needs a timeline clip; right after placing, `audio.beats` may answer "waveform still computing" for 20-30 s: wait and retry). Then `audio.beats {clip_ids: [music]}` (detection only) for onsets and BPM; if the BPM looks halved or doubled, retry with `refractory_s: 0.4`. Sections: where onsets get denser or sparser over the song (`audio.analyze` only gives whole-clip numbers), and ask the user where the chorus is if it is not obvious. Convert onsets to timeline time (start + onset - src_in). Build the cut list from the card's grid; minimum shot 0.25 s.
3. Show the plan: BPM, number of shots, the section rates, which clips go in the chorus. Wait for a yes.
4. `edit.begin {label: "Music video style"}`.
5. For each slot (back to front is not needed; place in order): `timeline.place {asset_id, at: <slot start>, in, out, track: 0}` with `out - in` = the slot length, `in` on a moving part (use `media.scene_cuts {clip_id}` on long clips to find distinct shots). Mute picture audio: `clip.set {clip_id, fields: {volume: 0}}`.
6. Accents: `keyblocks.apply {name: "Zoom Punch", clip_ids: [clip], at}` on hits; on section changes a shader transition: first `timeline.add_transition {right_clip_id, kind: "CrossFade", duration: 0.15}` to set its length, then `library.apply {id: "transitions/flash", clip: <right clip>}` (or `transitions/whip-pan-left`, `transitions/glitch`), which keeps that length (on its own it lands at 1 s); `gallery.apply {tab: "SpeedRamps", name: "Montage", clip_ids: [clip]}` per the card.
7. Look: the card's LUT on every clip (`library.apply {id, clips}`), then `clip.effects_bulk {clip_ids, index, params: {Intensity: 0.7}}` (index = the LUT's place in the effect list). Library items that are not installed need `gallery.connect {}` and `library.install {id}` (a download: ask in step 3).
8. `edit.end`. Verify: `render.contact_sheet {from: 0, to: <end - 0.1>, count: 16}` and spot frames at three beat times (`render.frame {t}`): a cut should land on each chosen beat.

Energy: $ARGS

Report back: BPM, shots, the cut grid per section, accents, look, and that one Undo removes the montage.
