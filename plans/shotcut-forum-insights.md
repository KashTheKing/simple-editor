# Shotcut forum pain report (read 2026-11-19)

Research only. Companion to `plans/shotcut-comparison.md` (feature gaps); this file is about **user pain**.

## Method and honesty notes

- Source: public Discourse JSON of forum.shotcut.org (`/categories.json`, `/c/<slug>/<id>/l/top.json`, `/t/<id>.json`). No blocks, robots.txt respected, generic User-Agent, about 1 request per second (one short stretch ran two fetch loops at once, so roughly 2/s for a few minutes).
- Listings read: Help (8), Bug (6) and Suggestion (7), top-by-views for the last year and all time, 1,200 topics. Thread text read for **about 250 topics**: the 220 most-viewed from Nov 2024 on, plus the most-viewed older ones. That is a bit under the 300–500 target. A fetch loop was still finishing, so the last older threads are not read.
- "Frequency" below is the thread count in the sample and the total views of the cited threads. Views are modest for recent threads (200–1,300 each) because the forum is small; the two huge ones are 2017–2019 start-up errors. Counts come from title keyword matches, so treat them as ±30%.
- Quotes are 15 words or fewer. Posts are summarised, not copied. All links are `https://forum.shotcut.org/t/<id>`.
- SE status was checked read-only in `rel-131`: `website/docs/*.mdx`, `CHANGELOG.md`, and grep of `src/`. Nothing was built or run, so "partly" and "not solved" mean "no evidence in docs or source", not "tested broken". Things I could not confirm are marked (unverified).

## What the forum says in one paragraph

Shotcut users are not asking for more features. They hit three walls: (1) the timeline and tracks behave in surprising ways after updates (new tracks appear by themselves, snapping got worse, trimming and dragging lag), (2) basic jobs (cut a section, freeze a frame, split screen, scrolling text, blur a face, beep a word) need a multi-step workaround that people only find through forum answers, and (3) things silently break: exports that stick at 99%, exports rejected for "missing data" with no way to find the files, crashes on drag or open, previews that stutter until you touch a setting. A third of answers from the maintainer are "doesn't reproduce for me" or "you need to convert to edit-friendly first". Users who are not technical never get unstuck.

## Ranked problems

Rank = how often it recurs × how angry the thread is. Each entry: description, signal, threads, SE status, fix.

### 1. Timeline editing is fragile and surprising (UI)

- Description: tracks created automatically, clips that won't snap exactly, drag/trim delays of seconds, audio track can't be moved, resizing clips "next to impossible", ripple behaviour hidden, track-name column too narrow.
- Signal: ~33 threads, ~10k views (largest cluster).
- Threads: [50151](https://forum.shotcut.org/t/50151) (every dropped clip adds more empty tracks; users argue the default is bad for beginners, maintainer calls it "an experiment"), [51986](https://forum.shotcut.org/t/51986) (3 s+ delay selecting a segment), [52133](https://forum.shotcut.org/t/52133) (magnetism got worse), [51122](https://forum.shotcut.org/t/51122) (how to turn on ripple), [51223](https://forum.shotcut.org/t/51223) (audio track stuck at bottom), [48015](https://forum.shotcut.org/t/48015), [50407](https://forum.shotcut.org/t/50407) (duplicate a track in one click).
- SE status: **partly solved.** The timeline toolbar is already reduced (Select, Ripple/Rolling, Slip/Slide, Blade, Hand/Zoom, Snapping, Linked Selection; CHANGELOG Unreleased). No auto-add-tracks feature exists (grep 0), so the surprise is absent. "Duplicate track" has no hit in docs or source. Whether snapping and drag stay instant on 3-hour timelines is (unverified).
- Fix: (a) never create tracks implicitly; dropping onto empty space creates one track, shown with an Undo toast. (b) Right-click a track header: Duplicate, Move up/down, Rename. (c) Resizable track-name column with a minimum that always fits ~10 characters. (d) One plain "Delete and close gap" (Shift+Delete) next to Delete, named in the toolbar tooltip, so "enable ripple" is never a question. (e) Add a drag-latency check on a 3-hour, many-clip project to the perf bench, budget < 50 ms.

### 2. Basic jobs are buried or need workarounds (UI, discoverability)

- Description: cut out a section, freeze frame, rotate 90°, split screens/grids, "show only this section", scrolling ticker text, progress bar, blur faces or confidential text, beep a word, photo frame. People ask "where" and the answers are 8-step recipes.
- Signal: ~17 "how do I / where" threads plus ~12 recipe requests, ~7k views; the all-time #3 thread ([3483](https://forum.shotcut.org/t/3483), "How to cut parts of the video", 110k views) is exactly this.
- Threads: [48029](https://forum.shotcut.org/t/48029) (freeze frame "unacceptable" workflow), [51607](https://forum.shotcut.org/t/51607) (cutting a segment is "buried under a bunch of stuff"), [51970](https://forum.shotcut.org/t/51970) and [51468](https://forum.shotcut.org/t/51468) (grids and side-by-side), [51636](https://forum.shotcut.org/t/51636) and [51744](https://forum.shotcut.org/t/51744) (scrolling text), [51857](https://forum.shotcut.org/t/51857) (beep out words), [50770](https://forum.shotcut.org/t/50770) (blur moving text), [51376](https://forum.shotcut.org/t/51376) (rotation direction), [52054](https://forum.shotcut.org/t/52054).
- SE status: **partly.** Command palette (Ctrl+P), effects search with chips, freeze frame in the right-click Speed menu, templates and Gallery text, mask and blur effects exist. Missing or unverified: grid/split-screen layout (1 source hit, none in docs), scrolling ticker (5 source hits, none in docs), beep/bleep (none), progress bar (docs mention, unverified as a ready preset), face/area blur with tracking (docs 1).
- Fix: a **"What do you want to do?" box** in the command palette that matches plain phrases ("cut out a part", "freeze", "split screen", "blur", "beep", "ticker", "rotate") to either a one-click action or a guided mini-wizard, and shows the keyboard shortcut. Add presets in Gallery ▸ Layouts: 2-up, 3-up, 2×2, 3×2, picture-in-picture, each with fit/crop done. Add **Censor** to the right-click menu of a selection (blur or beep over the marked range). Scrolling ticker as a Gallery text with a speed slider.

### 3. Crashes on common actions (crashes)

- Description: crash on opening an old project, on dragging media to the timeline, on a filter, on simple playback, on restoring a minimised window during export, on hardware encoding.
- Signal: ~16 threads, ~4.5k views; mostly 26.1–26.10 builds, so it is regression-driven.
- Threads: [51137](https://forum.shotcut.org/t/51137) (project crashed after a source file was relocated; one specific clip crashed every new timeline too), [51094](https://forum.shotcut.org/t/51094), [51090](https://forum.shotcut.org/t/51090) (one filter), [52119](https://forum.shotcut.org/t/52119), [52046](https://forum.shotcut.org/t/52046) and [52076](https://forum.shotcut.org/t/52076) (drag/drop), [51195](https://forum.shotcut.org/t/51195) (restore window during export), [51139](https://forum.shotcut.org/t/51139) (hardware encode takes the machine down).
- SE status: **partly solved.** A panic hook writes `crash.log` and snapshots the latest autosave; the next start offers "Recover unsaved project?"; autosave every 30 s (`getting-started.mdx`, `report-a-bug.mdx`, `ui/app/recovery.rs`). Not found: protection against a single bad media file taking the app down on every open, a safe mode, or isolation of the decoder and hardware encoder.
- Fix: see Session C. In short: probe and decode risky media on a guarded worker; if the app died while a file was being opened, the next start opens the project with that file offline and says so; a "Open without effects/media" safe start; hardware encoder tested once in a child process and never reused if it failed.

### 4. Preview lag and choppy playback (slow previews)

- Description: whole UI lags during playback, undo gets slower on long timelines, memory growth on long recordings, choppy playback even with proxies and preview scaling, lag at the start of each clip.
- Signal: ~15 threads, ~5.7k views.
- Threads: [50776](https://forum.shotcut.org/t/50776) (the user found that changing any playback setting made it smooth again, a stale-state bug nobody fixed in 25 replies), [51102](https://forum.shotcut.org/t/51102), [51565](https://forum.shotcut.org/t/51565) ("is my laptop just not good enough?"), [51567](https://forum.shotcut.org/t/51567) (undo slower with longer videos), [50935](https://forum.shotcut.org/t/50935), [50736](https://forum.shotcut.org/t/50736) (3-hour mkv), [51470](https://forum.shotcut.org/t/51470).
- SE status: **partly.** Hard rule exists (no heavy compute on the playback path, prepare ahead, gate before Play), plus Movie Mode, playback cache, preview quality, proxies in Settings ▸ Performance. Missing: any user-facing "why is it choppy" answer, automatic proxy proposal, a clear signal when the machine cannot keep up. Undo cost on long projects (unverified).
- Fix: see Session D. One **Playback health** chip in the viewer (green/amber) that says in words what is wrong and has a single "Fix it" button (make proxies for this clip, lower preview quality, close the cache). Test undo and seek on 3-hour footage in the bench.

### 5. Exports that stick, fail or are rejected (export)

- Description: stuck at 99%, "rendering froze", exit code 11/127, silent project rejected for "missing data", exports with no audio, blank video at quality 0.
- Signal: ~11 threads, ~3.3k views.
- Threads: [51851](https://forum.shotcut.org/t/51851) (sticks at 99% for an hour; cause suspected: a new plugin loader, resolved by guesswork), [51460](https://forum.shotcut.org/t/51460) (culprit was a clip at 0.0025× speed, found by trial), [51389](https://forum.shotcut.org/t/51389) (the missing-files dialog opens in the Downloads folder; real cause: deleted proxies), [50671](https://forum.shotcut.org/t/50671), [51789](https://forum.shotcut.org/t/51789), [51759](https://forum.shotcut.org/t/51759) (no audio), [51456](https://forum.shotcut.org/t/51456).
- SE status: **partly solved.** Pre-export check lists up to 5 missing files and offers "Export anyway?" (`ui/app/files.rs:315`); ffmpeg stderr tail is captured on failure (`engine/export.rs:1371`); hardware encoders are tested and dropped from the list with a software x264 fallback (`faster-exports.mdx`). Not found: plain-language failure text, "which clip broke it" detection, automatic retry on the software encoder, a stuck-at-99% watchdog, Relink from the export prompt.
- Fix: see Session B. Classify ffmpeg errors into 6–8 human sentences with one button each; on any encoder failure retry once with software x264 automatically and say so; "Find the problem" bisects by exporting the timeline in halves at low quality and names the clip; offer **Relink…** and "Rebuild proxies" right in the missing-files prompt.

### 6. "Which settings for YouTube / TikTok" and quality worries (export)

- Description: unsure which preset; platform recompression looks worse; export time/size surprises; default preset not editable; CPU not fully used.
- Signal: ~8 threads, ~2k views.
- Threads: [51395](https://forum.shotcut.org/t/51395) (uploads look worse on YouTube), [50942](https://forum.shotcut.org/t/50942) (TikTok over-compresses), [51027](https://forum.shotcut.org/t/51027) ("is my export time normal?"), [50816](https://forum.shotcut.org/t/50816) (default preset), [51100](https://forum.shotcut.org/t/51100) (file size vs length), [51355](https://forum.shotcut.org/t/51355).
- SE status: **mostly solved.** Platform cards (YouTube 1080p/4K, Shorts/Reels/TikTok 1080×1920), loudness normalise −14 LUFS on, saved export presets (`export.mdx`). Not found: estimated size before export, a "social upload tip" about re-compression, default-preset editing (presets exist, "default" unverified).
- Fix: show **Estimated size and time** next to the Export button; add a one-line note under TikTok/Shorts ("they re-encode; use this at 1080×1920 and high bitrate to keep detail"); remember last-used preset per project.

### 7. Phone footage, VFR, HDR and colour shifts (export, preview, "looks wrong")

- Description: iPhone HDR/HEVC clips look flat or shift after export; gamma looks brighter than VLC; audio drifts after cuts on variable-frame-rate recordings (screen recorders, phones); colour changes with some filters or transitions in 10-bit mode; LUT on log footage too dark.
- Signal: ~14 colour threads plus ~5 VFR/phone, ~5k views.
- Threads: [50964](https://forum.shotcut.org/t/50964) and [51335](https://forum.shotcut.org/t/51335) (what to do with iPhone HDR), [51574](https://forum.shotcut.org/t/51574) (looks right in the app, wrong after export), [50565](https://forum.shotcut.org/t/50565) (desync after splits; the advice is "convert to edit-friendly"), [50649](https://forum.shotcut.org/t/50649) (gamma), [50927](https://forum.shotcut.org/t/50927) (N-Log LUT), [51671](https://forum.shotcut.org/t/51671), [51827](https://forum.shotcut.org/t/51827).
- SE status: **partly.** HDR is tone-mapped to SDR for preview, proxies and SDR export (`project-settings.mdx`). No VFR detection or conform in `src/` (one comment on VFR gaps in `player/engine.rs`). The compositor is 8-bit RGBA, so some colour-shift classes cannot happen, which is a plus (see `shotcut-comparison.md`).
- Fix: at import, probe frame-rate mode, HDR and rotation. For VFR, silently conform to a constant-rate proxy and use it for edit and export, with a small "Fixed phone footage timing" badge. For HDR, tell the user once ("HDR clip, shown as standard colour") with a one-click Keep HDR on export (SE already has HDR export per the comparison).

### 8. Audio basics (other)

- Description: music louder than voice, ducking by hand, recording voice-over in the right format, one channel only, desync, normalise target confusion.
- Signal: ~15 threads, ~5k views.
- Threads: [51727](https://forum.shotcut.org/t/51727) ("audio ducking, again"), [50715](https://forum.shotcut.org/t/50715) (default normalise level -23 makes music louder), [51087](https://forum.shotcut.org/t/51087) and [51415](https://forum.shotcut.org/t/51415) (record format/quality), [50532](https://forum.shotcut.org/t/50532), [51491](https://forum.shotcut.org/t/51491).
- SE status: **solved or mostly.** Ducking, loudness match, role buses, voice-over record, -14 LUFS export (`audio.mdx`, MCP `audio_duck`, `audio_normalize`, `audio_role`).
- Fix: only surface it: a "Make the voice clear" button on the Audio page (duck music under voice, one click), and tell users where it is via the palette box from #2.

### 9. Text, titles and subtitles (other)

- Description: text can't be typed into the filter, rich text crashes or won't outline, speech-to-text fails or the model can't be downloaded, subtitle import says "ffmpeg not found", "how do I replace the template text", burned-in captions can't be removed.
- Signal: ~24 threads, ~8.6k views.
- Threads: [49539](https://forum.shotcut.org/t/49539), [51674](https://forum.shotcut.org/t/51674), [51500](https://forum.shotcut.org/t/51500), [50726](https://forum.shotcut.org/t/50726), [51191](https://forum.shotcut.org/t/51191) and [50559](https://forum.shotcut.org/t/50559) (speech-to-text produces nothing), [50846](https://forum.shotcut.org/t/50846) (model download blocked), [51805](https://forum.shotcut.org/t/51805), [51927](https://forum.shotcut.org/t/51927).
- SE status: **partly.** Native text clips with box/outline controls, Gallery Titles and Captions, a transcribe installer (`transcribe_install`) and captions doc exist. Download failure handling and "ffmpeg missing" messages: SE ships ffmpeg.exe (stack note); a clear offline or blocked-download message for the model is (unverified).
- Fix: model download that retries, can be pointed at a local file, and says exactly what failed; captions are a clip (select, Delete) so "remove burned-in text" is never a question.

### 10. Keyframes (UI)

- Description: stiff motion at stops, no copy/paste of keyframes, keyframe window not at the cursor on long clips, can't keep centre while resizing.
- Signal: ~8 threads, ~1.8k views.
- Threads: [51171](https://forum.shotcut.org/t/51171) (smooth stop), [50458](https://forum.shotcut.org/t/50458) (copy and paste), [50857](https://forum.shotcut.org/t/50857) and [51364](https://forum.shotcut.org/t/51364) (window jumps away from the playhead), [50420](https://forum.shotcut.org/t/50420), [50708](https://forum.shotcut.org/t/50708).
- SE status: **partly solved.** Keyframes in the Inspector, easing and "keyblocks" (MCP `keyblocks_*`), Animator plans. Copy/paste of keyframes and a "ease in/out" default are (unverified).
- Fix: one "Smooth" toggle on any animated value (applies ease in/out to all its keys); copy keys from one value to another via right-click; keyframe lane always scrolls to the playhead.

### 11. Look and feel: dark mode, rounded corners, toolbar (UI)

- Description: panels indistinguishable in dark mode, invisible filter labels, big rounded clips and thick playhead disliked after an update, toolbar changed under the user, "Open Other" menu vanished.
- Signal: ~8 threads, ~2.7k views.
- Threads: [51715](https://forum.shotcut.org/t/51715), [50615](https://forum.shotcut.org/t/50615), [52154](https://forum.shotcut.org/t/52154) and [52118](https://forum.shotcut.org/t/52118), [51073](https://forum.shotcut.org/t/51073), [48219](https://forum.shotcut.org/t/48219).
- SE status: **partly.** Accent text now picks white/black for 4.5:1 contrast (CHANGELOG); UI is themeable; a per-user setting for clip corner rounding / playhead width is (unverified).
- Fix: a Settings ▸ Look option for "Clear borders" (stronger panel separators) and clip/playhead width; never move toolbar buttons between versions without a "What moved" note in the What's new screen.

### 12. Install, start-up and Windows trust (crashes/other)

- Description: "Failed to read version.json", blocked by Smart App Control, freezes on first launch, slow first launch, portable build does not run.
- Signal: ~7 threads, ~1.6k views, plus the two biggest all-time threads ([4076](https://forum.shotcut.org/t/4076) and [10622](https://forum.shotcut.org/t/10622), together ~350k views: "Install OpenGL 2.0 or Higher" and a white window on old GPUs).
- Threads: [51976](https://forum.shotcut.org/t/51976), [52016](https://forum.shotcut.org/t/52016), [52141](https://forum.shotcut.org/t/52141), [50509](https://forum.shotcut.org/t/50509), [51815](https://forum.shotcut.org/t/51815).
- SE status: **not solved / unverified.** SE uses eframe with `glow` (OpenGL; `Cargo.toml`), so the same old-GPU "no OpenGL" and white-window failure is possible; I found no software-renderer fallback or friendly message in `main.rs`/`settings.rs`. Installer signing and Smart App Control are (unverified).
- Fix: on start, if OpenGL context creation fails, show a plain dialog with one button "Start in compatibility mode" (software GL or CPU preview) instead of an OS error. Sign the exe (SmartScreen / Smart App Control).

### 13. Projects, missing files, proxies and backups (UI/data)

- Description: proxies saved in the wrong place, proxies deleted so exports are rejected, forced re-create, hourly backup not working, 100+ markers lost, empty timeline after an update, replacing footage while keeping cuts.
- Signal: ~9 threads, ~2k views.
- Threads: [50741](https://forum.shotcut.org/t/50741), [50834](https://forum.shotcut.org/t/50834), [51211](https://forum.shotcut.org/t/51211), [51206](https://forum.shotcut.org/t/51206), [51428](https://forum.shotcut.org/t/51428), [51716](https://forum.shotcut.org/t/51716), [51389](https://forum.shotcut.org/t/51389).
- SE status: **partly.** Relink, Consolidate Media, a cache folder setting, rolling per-project autosave and Restore Autosave exist. Replace footage keeping cuts: `timeline_replace` exists in MCP; a visible UI is (unverified).
- Fix: a visible **Project health** line (missing files, missing proxies, cache size) that opens one fix list; **Replace Media…** in the Library right-click.

## Smaller items worth one-liners

- Subtitle workflow and bilingual subtitles: [51246](https://forum.shotcut.org/t/51246), [51685](https://forum.shotcut.org/t/51685).
- Multicam: [52188](https://forum.shotcut.org/t/52188). SE has multicam (MCP tools); keep it findable.
- Waveform on silent video, BPM grid, beat marking: [51155](https://forum.shotcut.org/t/51155), [50985](https://forum.shotcut.org/t/50985), [50538](https://forum.shotcut.org/t/50538). SE has a Beat Aligner.
- Per-filter docs link: [50991](https://forum.shotcut.org/t/50991). SE has tooltips only; add "?" per effect.
- "Shotcut Lite" for beginners and students: [51059](https://forum.shotcut.org/t/51059). SE's page model (Edit/Color/Audio/Export) is the answer; make Edit-only the first-run default.

## Where Simple Editor already beats these complaints (keep and promote)

Auto-recover after crash, Home page with recents, one toolbar with few tools, command palette, platform export cards with loudness normalise, missing-media warning before export, hardware encoder test with software fallback, built-in ducking and voice-over, built-in crop, transcription installer.

## Proposed fix sessions

Each owns disjoint files. A, B, C and D can run in parallel. Session E is small and goes last or is skipped.

### Session A: UI simplification and findability (problems 1, 2, 10, 11, 13 UI parts)

Files (don't share): `src/ui/timeline/*`, `src/ui/inspector.rs`, command palette module, `src/ui/library.rs`, Gallery layout presets, `src/hotkeys.rs`, `src/ui/settings_ui/*` look section, docs for these.
Scope:
1. Plain-phrase task box in the command palette ("cut out a part", "freeze", "split screen", "blur", "beep", "ticker", "rotate", "duck music"): action or small wizard, shows the shortcut.
2. Track header menu: Duplicate, Move, Rename; resizable name column; never auto-create tracks.
3. "Delete and close gap" named and bound; tooltip on Ripple says what it does.
4. Gallery ▸ Layouts: 2-up, 3-up, 2×2, 3×2, PiP, ticker text; right-click ▸ Censor (blur/beep).
5. Keyframes: Smooth toggle, copy keys between values, keyframe lane follows the playhead.
6. Look options: clearer panel borders, clip/playhead width, "What moved" in What's new.
7. Library right-click ▸ Replace Media…; Project health line.
Done means: each task-box phrase reaches its action in at most two clicks, shown by a scripted UI test; timeline drag/select latency on a large project is under 50 ms in the bench; nothing in the new UI has an explanatory tail on its label (plain-labels rule); one screenshot per new surface.

### Session B: Export reliability and clarity (problems 5, 6, part of 7)

Files: `src/engine/export.rs`, `src/ui/export_ui.rs`, `src/ui/app/tools_export.rs`, `src/ui/app/jobs.rs`, export docs.
Scope:
1. Error classifier: ffmpeg stderr to a short human sentence plus one action (encoder failed, disk full, missing media, odd frame rate, audio layout, out of memory, file locked).
2. Auto-retry once on software x264 after any hardware-encoder failure; say so in the Jobs row.
3. Stuck-job watchdog: no progress for N seconds shows "Still working / Cancel and retry on software".
4. "Find the problem": bisect export by halves at low quality, name the offending clip, offer to jump there.
5. Missing-files prompt gets Relink… and Rebuild proxies buttons; keep "Export anyway".
6. Estimated size and time next to Export; social re-compression note; last preset remembered per project.
7. Open the finished export in the player (already a user rule).
Done means: injected failures (kill encoder, bad frame rate, locked file, missing proxy, 0.0025× speed clip) each give a one-sentence message with a working button; a test where the hardware encoder fails still produces a file; export of the test timeline completes with no 99% hang.

### Session C: Crash resilience and recovery (problems 3, 12, part of 13)

Files: `src/ui/app/recovery.rs`, `src/ui/app/autosave.rs`, `src/main.rs`, `src/media/*` probe/decode entry points, `src/model/io.rs`, `src/model/repair.rs` (already new and uncommitted in main; coordinate with whoever owns it).
Scope:
1. Crash fingerprint: record the file or action being processed (open, drag, filter, export) to a small "last action" file; on next start, if the app died there, open the project with that file offline and say which file.
2. Safe start: "Open without media / without effects" from the recovery dialog.
3. Probe and decode of new files on a guarded worker so a bad file fails alone.
4. Hardware encoder test in a child process, result cached per driver version.
5. Graphics start failure shows a plain dialog with "Start in compatibility mode" (software preview) instead of a white window; setting to force it.
6. Backup rotation test (hourly, restore from a chosen time) and a marker-count stress test (200+ markers survive save/load).
7. Sign the exe and installer (SmartScreen / Smart App Control) if a certificate is available; otherwise document the prompt.
Done means: a fuzz or injected-crash test for each of drag, open, filter and export recovers the project with at most 30 s of lost work and names the culprit file; compat mode starts on a machine with the GPU context forced to fail; no new crash path on the playback thread.

### Session D: Preview performance and phone footage (problems 4, 7)

Files: `src/player/*`, `src/media/proxy.rs`, import probe, `src/ui/layout.rs` playback chip.
Scope:
1. Import probe: frame-rate mode (VFR), HDR, rotation, codec cost; silently conform VFR to a constant-rate proxy and tell the user once.
2. Playback health chip (green/amber) with plain wording and one Fix button (make proxies for this clip, lower preview quality).
3. Automatic proxy proposal when a clip fails a decode-speed test (4K HEVC, 3-hour mkv), with size estimate.
4. Bench: 3-hour H.264, 4K60 HEVC phone clip, VFR screen recording; seek, scrub, undo and start-of-clip lag budgets; memory growth on a 3-hour timeline.
5. Fix stale-state class from [50776]: any playback setting change should be unnecessary; add a test that settings are applied on open.
Done means: the three bench clips play without dropped frames after the proxy proposal is accepted; VFR clip stays in sync across 20 random cuts; undo on the 3-hour timeline under 200 ms; no heavy work on the playback path (rule check).

### Session E (optional): audio and text findability (problems 8, 9)

Files: `src/ui/assistant_ui.rs` aside, audio page, captions/transcribe UI, docs.
Scope: "Make the voice clear" button, recording format choice in Voiceover (WAV default), model download that retries and can use a local file, captions removable as a clip.
Done means: each shows in the palette task box; a blocked-network test gives a clear message.

## Top 10 (for the coordinator)

1. Timeline surprises: auto-added tracks, snap/drag/trim lag, ripple hidden.
2. Basic jobs buried: cut a section, freeze, split screen, blur, beep, ticker.
3. Crashes on open, drag and filters, with no recovery for a bad file.
4. Laggy or choppy preview, undo slow on long timelines, no explanation.
5. Exports stuck at 99% or rejected for missing files, errors unreadable.
6. "Which settings for YouTube/TikTok", size and time unknown.
7. Phone HDR/VFR footage: desync after cuts, colour looks different after export.
8. Text and speech-to-text fail without a clear reason.
9. Dark mode and panel clarity, toolbar changes between versions.
10. Install and start-up trust: GPU/OpenGL failure and Windows blocking.

Audio ducking and keyframes also recur but SE already has most of the answer; they need to be findable, not built.
