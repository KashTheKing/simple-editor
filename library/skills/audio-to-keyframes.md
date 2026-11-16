---
name: audio-to-keyframes
description: Make a layer pulse with the music - After Effects' Convert Audio to Keyframes, plus hit markers.
args: "[what should react: scale, glow, shake, a shader parameter; and how strong]"
tags: audio reactive, keyframes, music, pulse, after effects, converted
credit: converted from the audio-peaks-to-markers recipe of open After Effects MCP servers (TheLlamainator/after-effects-mcp, MIT) through /convert-from-editor
---
After Effects editors turn a song into an "Audio Amplitude" layer, then drive properties from it with expressions, or drop markers on the peaks. Here: bake the music's level straight into an effect parameter's keyframes, or mark the hits and key a punch on each.

1. Call `editor.context` and `timeline.list`. Music: the selected audio clip, else the clip tagged Music, else the longest audio clip (ask if unclear). Target: the selected visual clip (video, picture, text, HTML or shape); with none, ask.
2. Pick the method from $ARGS:
   a. Continuous pulse (glow, blur, a shader parameter, "beat pulse"): the target needs an effect with a numeric parameter. If it has none, add one: `library.apply {id: "effects/beat-pulse", clip}` (or `effects/bloom`, `effects/zoom-punch`, `effects/handheld-shake`), or `clip.add_effect {clip_id, kind}`. Then `audio.bake_keys {clip_id: <music>, target_id: <target>, effect, param, fps: 30}`; `effect` is the effect's index (default the last one) and `param` its parameter name, always given (the default `level` exists on none of these effects). Library effects must be installed first (`gallery.connect {}`, then `library.install {id}`: a download, so ask). Parameter names are the shader's `@param` lines (`project.get` lists the values in that order), e.g. `effects/bloom` `intensity`, `effects/zoom-punch` `amount`, `effects/handheld-shake` `amount`, `effects/beat-pulse` `flash` or `bump`. `effects/beat-pulse` also pulses on its own clock (`bpm`, 128 by default): set its `bpm` to the song's BPM or prefer bloom `intensity`, which has no clock.
   b. Hits (scale punch on every kick): `audio.beats {clip_ids: [music]}` for onset times and BPM (a dry run), keep every beat or every 2nd for busy music; `audio.beats {clip_ids: [music], as_markers: true}` also drops them as markers if the user wants to see them. Convert each onset to the target's clip-local time (timeline time = music start + onset - music `src_in`; local = timeline - target start) and key a base key one frame before the hit (`clip.keyframe {clip_id, property: "Scale", t: t - 0.03, value: <base>, ease: "Hold"}`), the punch on the hit (`value: <base * 1.08>`, `ease: "EaseOut"`) and back to base 0.15 s later with `ease: "Hold"`, so the scale snaps instead of drifting up between hits. (`clip.keyframe` property names are the Inspector's: `Scale`, `Position X`, `Position Y`, `Rotation`, `Opacity`.) Strength from $ARGS: subtle 1.04, normal 1.08, strong 1.15.
3. Do it in one group: `edit.begin {label: "Audio to keyframes"}` ... `edit.end`. Baking writes many keys; that is expected and costs nothing at playback.
4. Verify: `render.frame {t}` on a hit and one between two hits (a contact sheet can hide a subtle pulse), plus `render.contact_sheet {from, to, count: 12}`; the effect must be visibly stronger on the hits and never cover the subject. For a whole-frame audio visualizer instead, use `audio.visualizer {clip_id, style}` (styles in `audio.visualizer_styles`).

Request: $ARGS

Report back: the method, the music clip, what parameter now follows the music (or how many punches were keyed), and that one Undo removes it.
