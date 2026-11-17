---
name: style-shorts
description: Restyle a vertical clip for TikTok / Shorts / Reels - hook first, tight cuts, word-by-word captions, punch-ins, safe zones.
args: "[clean, bold or meme]"
tags: style, shorts, tiktok, reels, captions, vertical, preset
credit: parameters derived from short-form datasets (AutoShot: ~13.6 shots per short), video-use (MIT) caption rules, platform safe-zone guidance
---
Make an existing short feel native to vertical feeds. For turning a long horizontal video into a short first, run `/shorts` (or `/reframe`).

Style card (`bold`; `clean` / `meme` in brackets):

| parameter | value |
|---|---|
| hook | the strongest line or image in the first 1-2 s; a 2.5 s hook text over it (none / meme-caption box) |
| rhythm | a visual change every 1.5-3 s: a cut, a punch-in, a cutaway or text |
| pauses | `min_silence` 0.25, `padding` 0.05 |
| captions | 1-3 words per cue, uppercase, `Big Impact` + `pop` (`Minimal` + `none` / `Yellow Pop` + `highlight`), centre-lower, inside the safe zone |
| punch-ins | `Zoom Punch` on every 2nd sentence (every 4th / every sentence) |
| sound | voice forward, music ducked -15 dB, a whoosh or pop on text entrances if the library has them |
| safe zone | nothing important in the top ~15%, the bottom ~35% or the right-hand button rail |
| ending | end on the payoff, no outro card; a loopable last frame is a bonus |

1. Call `editor.context`, `project.summary`, `timeline.list`. The project should be vertical (1080x1920); if not, offer `/reframe` first and stop.
2. Transcript: `transcript.get {clip_id}`, else `transcribe.run {clip_id, cues: false}`. Find the hook line and measure pauses (`autocut.detect {clip_ids, threshold_db, min_silence: 0.25, min_speech: 0.15, padding: 0.05}`). Show the plan (hook, time removed, caption style) and wait for a yes.
3. `edit.begin {label: "Shorts style"}`.
4. Tighten: `transcript.remove_fillers {clip_id, words: ["um", "uh", "erm"]}` first (it needs the clip's own transcript, which the cut pieces do not have), then `timeline.auto_cut {clip_ids, threshold_db, min_silence: 0.25, min_speech: 0.15, padding: 0.05, ripple: true}`. If the hook is not first, move it: cut it out and place it at 0 (`timeline.extract`, then `timeline.place {asset_id, at: 0, in, out, mode: "splice"}`).
5. Hook text (unless `clean`): `timeline.add_clip {text, at: 0, duration: 2.5, track}` on a video track above the footage (else it is hidden under it), under 8 words and about 16 characters per line (text does not wrap: break lines with `\n`), then `clip.set` (large, bold, top-centre below the top 15%) and `keyblocks.apply {name: "Pop In", clip_ids: [clip_id], at: 0}`; for `meme` use `library.apply {id: "html/meme-caption", at: 0}` and set its text with `html.set {clip, fields}`.
6. Punch-ins: `keyblocks.apply {name: "Zoom Punch", clip_ids, at}` per the card; never two within 2 s.
7. Captions: build the cues AFTER all cuts: the transcript keeps the original clip's source times, so map each word to the new timeline (piece start + word time - piece `src_in`, using the piece that contains it; word times are approximate, so a word that falls just outside a kept piece belongs to the nearest one: snap it in rather than drop it, and drop only words inside a removed stretch) and write them with `subtitles.set {cues}`, 1-3 words per cue, uppercase, at least 0.35 s each; then `subtitles.style_preset {name}`, `subtitles.animation {kind}` per the card. Check one cue with `guides.preview {platform: "tiktok", t}`: the caption presets sit close to the bottom, under the vertical apps' bottom bar, and no tool moves them, so ask the user to raise the bottom margin in the Subtitles style (to about a third of the height) and say so in the report.
8. Sound: `audio.duck {music_id, dialogue_ids, depth_db: -15}` if there is music; SFX only from the user's library (`/sfx-pass punchy`).
9. `edit.end`. Verify: `guides.preview {platform: "tiktok", t}` (and Shorts / Reels ids from `guides.list`) at two captioned moments, and `render.contact_sheet {from: 0, to: <end - 0.1>, count: 12}`: a visible change in nearly every tile.

Variant: $ARGS

Report back: length before and after, the hook, caption style, punch-ins, and that one Undo restores it. Export with `Shorts / Reels / TikTok` when asked.
