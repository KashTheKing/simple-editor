---
name: deliver
description: Pre-flight check the edit (offline media, gaps, black frames, levels, captions, safe zones), then export with a preset.
args: "[preset name, e.g. YouTube 1080p or Shorts / Reels / TikTok; or 'check only']"
tags: export, deliver, render, qc, preflight, resolve, premiere, converted
credit: converted from the render-validation and guarded export steps of open DaVinci Resolve and Premiere Pro MCP servers (samuelgursky/davinci-resolve-mcp, hetpatel-11/Adobe_Premiere_Pro_MCP, MIT) through /convert-from-editor; self-check pattern after video-use (MIT)
---
Resolve's Deliver page and Premiere's "validate project for export" check a timeline before it renders. Do the same here: find problems, fix only what the user approves, then export once with their OK.

1. Call `editor.context`, `project.summary`, `timeline.list` and `media.list`.
2. Check, and collect findings (nothing is changed yet):
   - Offline or still-converting media: `media.status {id}` for each used asset. Offline media must be relinked (`media.relink {dir}`) before anything else.
   - Gaps and stray clips on the main track: holes between clips in `timeline.list`, clips shorter than 3 frames, disabled clips left in.
   - Picture: `render.contact_sheet {from: 0, to: <end - 0.1>, count: 16}`, plus `render.frame {t}` 1.5 s either side of a few cuts and on the first and last 2 seconds. Look for black or flash frames, unintended jumps, text cut off.
   - Audio: `audio.analyze {clip_ids}` on every audio clip (it reads the source file, so add each clip's gain: dB = 20 x log10(volume)): nothing peaking above -1 dBFS, no clip far quieter than its neighbours, music ducked under speech (see `/loudness`).
   - Captions: `subtitles.get`: no cue over 7 s, lines readable, nothing past the end of the video.
   - Vertical video: `guides.preview {platform, t}` at a captioned moment (platform ids from `guides.list`, e.g. `yt_shorts`, `tiktok`); text clear of the platform UI.
   - Open review comments: `markers.list {open: true}`; unresolved notes are listed, not silently exported.
3. Report the findings as a short checklist (pass / problem / where). Offer fixes with the matching skills (`/clean-audio`, `/loudness`, `/captions`, `/review`); apply nothing unasked. A fix pass runs at most three times; then report what is left.
4. With `check only` in $ARGS, stop here.
5. Export: `export.presets` lists the presets (each with size and loudness normalisation). Pick the one in $ARGS, else the one that matches the project frame (16:9 = `YouTube 1080p`, 9:16 = `Shorts / Reels / TikTok`, 1:1 = `Instagram 1:1`). Confirm the preset and the output path with the user, then `export.quick {path, preset}`; the app may ask the user to Allow it. For several versions at once, `export.queue` and `export.status`.
6. After it finishes, check the file exists and its length matches the timeline (`export.status`).

Request: $ARGS

Report back: the checklist, what was fixed, the preset and path exported (or that it was a check only).
