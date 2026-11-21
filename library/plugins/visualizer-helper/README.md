# Visualizer Helper

A Simple Editor plugin (MIT) that turns the way the hardstyle music video was built by hand
(`plans/visualizer-helper.md`) into five one-click steps. Open it from **Window ▸ Plugins ▸ Visualizer Helper**.

1. **Analyse Song**: separates the stems with the Stem Separation add-on when it is installed (else works from the
   mix), then finds every hit with its strength and the downbeats, the drops, the calm part, the screeches, the
   vocal envelope, its onsets and vibrato. The result is stored on the project, with markers at the drops.
2. **Visualizer Templates**: Olympus Storm (the spiky chrome ball, waveform and lightning), Chrome Waveform or Storm
   Lightning, keyed from the analysis. Knobs: colours, spikes, sensitivity, smoothing, "go harder".
3. **Lyrics Karaoke**: paste the exact lyrics; every word fills on its sung syllable (Lyric Chrome shader), each line
   slides in with a 3D turn. Size comes from the font size, never a scale above 1.
4. **Drop FX**: a flash and a one-beat Invert at each drop, the glass shatter into the biggest drop, and the
   background grade.
5. **Background Loop**: a low-fps GIF or a still as a seamless 60 fps ping-pong loop (ffmpeg minterpolate).

Each step is one undo step and replaces only what it made before. The heavy work runs in the app in the background
(`audio.analyze_song`, `visualizer.template`, `lyrics.karaoke`, `visualizer.drop_fx`, `media.loop`; see
`docs/scripting-api.md`), so playback never stutters and no Python is needed.

Contents: `panels/visualizer-helper.luau` (the panel), `effects/olympus-storm.glsl` and `effects/lyric-chrome.glsl`
(library effects; the visualizer's `const` lines are filled per song), `luts/golden-hour.cube` (the background look).
