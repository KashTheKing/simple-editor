# Visualizer Helper — hand-off from the hardstyle visualizer build

This is how one hardstyle music video was built by hand over the live MCP in about 80 rounds of feedback: a spiky
chrome ball visualizer, a waveform, lightning, golden drops, karaoke lyrics, and a glass-shatter inversion into the
last drop. It also covers what we then automated into scripts, and a proposal for turning it into a
**Visualizer Helper** plugin.

Everything in `plans/visualizer-helper/` is real, working code. It runs today as Python plus generated Luau over the
MCP. The plugin's job is to make it one click.

## Contents of `plans/visualizer-helper/`

| file | what it is |
|---|---|
| `hsv_analyze.py` | Takes the song plus its Demucs stems and writes `analysis.json` (bpm, hits, drops, calm section) and `keys.lua` (every hand key for the visualizer). |
| `hsv_lyrics.py` | Takes the exact lyrics, the transcript and the vocal stem and writes `lyrics.lua` (text clips, slide, 3D turn and word-fill keys). |
| `hsv_shaders.py` | Takes the analysis and writes the per-song visualizer shader, the invert-layer shader, `shaders.lua` and `background.lua` (the grade). |
| `hsv_background.py` | Turns a low-fps GIF, clip or still into a seamless 1920×1080 60 fps ping-pong loop (ffmpeg minterpolate). |
| `hsv_install.py` | Copies a generated script into the app's scripts folder with ids filled in, so `scripts.run` executes it. This avoids sending a 50 KB `scripts_eval` payload. |
| `olympus-storm.template.glsl` | The visualizer shader. Its `{{BIGDROP}}`, `{{RIFT0}}`, `{{SUCK0}}`, `{{SUCK1}}`, `{{RIFTHIT0}}` and `{{RIFTHITS}}` placeholders are filled per song. |
| `olympus-storm.vegas.glsl`, `olympus-rift-invert.vegas.glsl` | The exact shaders of the finished video, for reference. |
| `lyric-chrome.glsl` | The lyric text shader: a silver/gold bevel and thin edge, a real perspective yaw, a motion-blur smear, the cylinder wrap and a soft dark halo. |
| `golden-hour.cube` | The background LUT. |

The scripts need Python 3 with numpy, scipy and Pillow, and ffmpeg on PATH. None of them write next to the user's
media.

---

## 1. The pipeline, in order

1. **Background.** The source was a 2-fps AI GIF.
   - Process: cover-crop to 1920×1080, stretch to 8 s, then `minterpolate=fps=60:mi_mode=mci:mc_mode=aobmc:vsbmc=1`.
   - Ping-pong (forward then `reverse`) so the loop point never jumps. Plain loops jumped visibly, and crossfades
     ghosted.
   - Encode at crf 21 (≈12 Mbps), loop it under the song and add `golden-hour.cube` as a LUT.
2. **Visualizer clip.**
   - Run `audio.visualizer {asset_id, style}` on the song. It bakes level, band1, band4 and the `u_wave` peak
     array as keys, linked to the song.
   - Then replace its shader with `clip.set_shader`. Never restyle again: a re-bake wipes the hand keys.
3. **Stems.**
   - `audio.stems separate` (Demucs htdemucs) gives vocals, drums, bass and other.
   - Every musical signal below comes from a stem, not the mix. Mix-based vocal or kick detection fails on
     hardstyle because the kick and the screech lead are as loud as the voice.
4. **Hits (hardstyle kicks).** The app's beat detection missed most hits.
   - Onset envelope from the STFT: 0.7 × low band (35–160 Hz) + 0.3 × full band log-flux, median-normalised.
   - Tempo by autocorrelation in 130–175 BPM. This song tracked at 151.
   - Ellis dynamic-programming beat tracker on that tempo.
   - Add off-beat stabs halfway between beats, plus any `find_peaks` onset the grid missed (rolls). Merge hits
     closer than 85 ms.
   - Downbeats: the beat phase mod 4 with the most low end. A downbeat gets strength 1.0, other grid beats
     0.55–0.8, off-grid stabs ≤ 0.5.
   - Kick gate: hits with no 40–150 Hz thump in the 60 ms after them drop to 0. Before the gate, the ball popped on
     hats.
   - After the first drop, strong full-band onsets the gate killed come back as medium hits (0.3–0.75). Without
     this, 0:42–0:45 looked dead in an intense part.
   - Output: `kclock` (hit number, linear, so fract() is the phase and floor() the count) and `kpow` (Hold keys).
5. **Screeches.**
   - Brightness jumps in the mix (3–9 kHz over 0.2–3 kHz, next 120 ms vs last 250 ms, above the 40th loudness
     percentile) give the `sclock` counter.
   - The synth stem (`other`) 1.5–8 kHz loudness envelope (30 ms attack, 250 ms release, 0..2.5) gives `screech`.
   - The electricity reads `smoothstep(0.8, 2.1, screech)`, so it goes berserk on the screeches and calms between
     them. This is what the user meant by "synced to the hardstyle synths".
6. **Vocals, from the vocal stem.**
   - Level in dB, normalised between the 55th and 99.5th percentiles, ^1.3.
   - Held envelope: 80 ms attack, 400 ms release, so a held "ahhhh" moves smoothly.
   - Talk vs sing: autocorrelation pitch (confidence > 0.8). The "singing" amount combines how much of each 0.5 s
     is in voiced runs ≥ 0.25 s, the voiced fraction, the pitch height and the loudness, smoothed over 0.3 s.
   - `vox = (1 − s)·fast·0.4 + s·held`, where fast is 30 ms/120 ms and s = sing^1.2. Talking twitches a little;
     singing swells.
   - Vibrato: pitch minus its 0.31 s median, band-passed 3.5–8.5 Hz, /55 cents, gated by the held level and the
     singing amount, giving `vib`.
   - All curves are simplified with Ramer–Douglas–Peucker (tolerance 0.035 for vox, 0.09 for vib) to keep key
     counts low (~200 vox, ~550 vib keys for 92 s).
7. **Drops.**
   - A drop is the bass stem returning after ≥ 1 s of silence. The first one counts; a later one counts only if the
     next 4 s are louder (bass + drums) than any earlier section.
   - The time is snapped to the nearest hit at the start of the bass rise.
   - Calm section: the first point after the last drop where the next 8 s of energy are 4 dB under the drop's.
   - Drives: `fx` 0→1 at the first drop (gold, lightning, bullets, beams), `kick` 2.2 in the biggest drop,
     `beam` 3 = a white flash at each drop decaying in 0.4 s, and the pre-drop beam rising 0.3→1.
8. **Ball, waveform and electricity shader** (`olympus-storm.template.glsl`, one 14-param shader on the
   visualizer clip).
   - Ray-marched SDF ball, 48 steps, with a bounding sphere so empty pixels skip the march.
   - Its layers:
     - a liquid body flowing with level, band1 and band4;
     - sea-urchin spines on a staggered lat/long grid, 1–4 popped per hit (more with intensity), each living
       4 beats;
     - "frozen" bursts on about 1 beat in 8, held for 6 beats;
     - round-edged beat spikes on the 3 nearest of 32 directions.
   - Cel shading (four tones plus an ink edge) with an NCS-style electric shell on the surface, from
     waveform-bent sheets and noise filaments.
   - Chrome bullets drawn flat, not marched: one per popped spine, flying out tip-first.
   - Lightning only on screeches. A ring hugs the ball and bursts on hard hits.
   - Holy beam rays behind, swelling with level, vocals, hits and screeches, and drawn over the waveform too.
   - The waveform along the bottom reads `u_wave` at "the next 1.26 s" across the width, so a hit reaches the left
     edge exactly when it plays. Its height is the 10 ms peak minus 80% of the local mean, so transients stand up.
     It flattens under lyrics (`lyric` param).
   - Silver before the first drop, gold after.
9. **Lyrics** (`hsv_lyrics.py`, `lyric-chrome.glsl`).
   - The user's exact words, one on-screen line each, upper-case. The Greek look maps A→Λ, E→Σ, O→Θ, Y→Ψ.
   - Each word is fuzzy-matched in order to the whisper transcript (small.en). Unmatched words are spread by
     letter count, and every start is snapped to a syllable onset on the vocal stem (a 10 ms log-level rise
     > 4 dB).
   - The last word is held until the voice really stops, so "NOWWWWW" fills slowly.
   - A line appears 0.35 s before its first word and leaves when the voice stops. Lines never overlap.
   - The `fill` keys are word edges measured with the real font (PIL `getlength`) as fractions of the text box.
   - Motion:
     - in from x = 1500 with an ease-out to a slow drift through the middle (±min(70, 40·d) px), then out to
       −450 with an ease-in;
     - `yaw` follows x, so a line faces the centre from either side;
     - `smear` is 1 → 0 → 0.45 for the side-on motion blur;
     - no vertical animation at all;
     - opacity fades over the last 0.3 s.
   - The line that ends into the biggest drop is the giant one, wrapped round a cylinder (`curve` 0.4) and rolling
     in and out.
10. **Drop moments.**
    - First drop: `fx`, a beam flash and an impact shake.
    - Biggest drop:
      - From 12.7 s before it, everything is sucked in: the beam radius shrinks (`rS = r·(1 + 2.2·suck)`), light
        streams inward along the rays, and lightning shortens.
      - On the first hit, one big glass shard breaks out of the ball's centre.
      - The ball shatters and inverts. Voronoi shards invert on every hit, more and further each time
        (`h < 0.4 + 0.6·prog²`, reach `0.85·prog + 1.8·prog³`), until almost everything is inverted. Then comes
        the flash.
      - Inversion is a separate clip on the top track: a copy of the visualizer shader cut down to the shard mask,
        drawing white with blend **Difference** over everything, including the lyric line.
    - Background grade: opacity 0 (black) before the first drop, brightness −0.45 → −0.12 → +0.06 at it,
      Levels In-white 0.44. In the biggest drop, a gold-only duotone: saturation 0.1, Color Tint (255, 176, 40) at
      0.72, contrast 1.2, In-white 0.85.

## 2. Tools relied on, and the gaps

**Used:**
- **Setup and analysis:** `editor.context`, `media.import`, `audio.visualizer`, `audio.stems`, `transcribe.run`,
  `media.transcript`.
- **Building:** `scripts_eval` (Luau, one undo step per edit: the workhorse, with `editor.tool` inside),
  `clip.set_shader`, `clip.keyframe`, `clip.set`, `clip.add_effect`, `clip.add_lut`, `shapes.add`,
  `timeline.add_clip` (text), `timeline.move` / `trim`.
- **Checking:** `render.frame`, `render.contact_sheet` (all visual checking), `perf.bench` and `playback.status`.
- **Output and recovery:** `export.video`, `history.list`, `ui.action undo`.

**Gaps (hand-built here; the plugin or the app should own them):**
1. Hardstyle-grade beat/kick detection with per-hit strength and downbeats. `beats.detect` misses most hits, so a
   numpy DP tracker was built.
2. Stem-driven keyframing: the vocal envelope, talk vs sing, vibrato and the synth envelope. Nothing in the app
   turns a stem into an envelope key track.
3. Drop and calm detection.
4. Karaoke from given lyrics: line clips, word fill from real glyph widths, and the slide/turn/smear animation.
   `subtitles` and `text.animate` don't do word-timed fill on a shader.
5. A way to run a big generated script without pasting it.
   - `scripts_eval` needs the whole 50–70 KB of keys in the call.
   - `scripts.run` reads a file but has a fixed 5 s budget and no return value.
   - Needed: `scripts.run {name, budget_ms}` returning the result, or a bulk `clip.keys {clip, property, keys[]}`.
6. A bulk key writer. Thousands of `clip.keyframe` calls (~1500/s) work but are slow and chatty.
7. The shader param cap (14). Per-song times had to be baked into the GLSL as constants. Song-level uniforms
   (drop times, beat counter) would remove the template step.
8. Layer access to the composited image below. The invert needed a second full copy of the shader on a Difference
   layer. A `u_below` sampler (or an Invert effect with a shader mask) would halve the cost.
9. No unlink tool: `ui_action link` misbehaved and deleted clips. `timeline.duplicate` on a linked visualizer
   also duplicates the song.
10. Text-clip fields: `clip.set` has no `width`/`height`/`link`, so size is keyed instead. The JSON says
    `scale_x`/`scale_y` but the keyframe properties are `width`/`height`.

## 3. Parameters that mattered (hardstyle values that worked)

| what | value |
|---|---|
| tempo search | 130–175 BPM (151 here); DP alpha 200 |
| hit merge window | 85 ms |
| kick gate | 40–150 Hz RMS in 60 ms after the hit, within 18 dB of the 95th percentile; onset > 0.2 |
| hit strengths | downbeat 1.0, grid 0.55–0.8, stab ≤ 0.5, calm outro ×0.35 |
| hits per 8 s in a drop | 25–40 (fewer = the detector is missing kicks) |
| screech | synth stem 1.5–8 kHz, 30/250 ms, 0..2.5; electricity smoothstep(0.8, 2.1) |
| vocal held envelope | 80 ms attack / 400 ms release, level ^1.3 |
| talking envelope | 30/120 ms, ×0.4 |
| vibrato | 3.5–8.5 Hz band, /55 cents |
| lyric snap | onset −0.18…+0.3 s of the transcript word; lead-in 0.35 s; hold until 0.15 s of silence |
| lyric motion | io = min(0.4, 0.22·d); drift ±min(70, 40·d) px; exit to −450 px; opacity out over 0.3 s |
| text box margin | scale 1.08 × 1.25 with the matching inset in the shader |
| shatter | build 12.7 s; first shard at Voronoi scale 4.5; shards at scale 7; ball inverts at prog 0.28 |
| ray march | 48 steps, bounding sphere; the result was 3.8 ms per frame and 0 dropped at 60 fps |

## 4. What went wrong and the fixes

- **Frame drops (137 to 197 dropped at the second drop).**
  - Bullets were ray-marched: they are now drawn flat. The spike search runs only near the surface.
  - Spine picks are precomputed per pixel (`g_pk`/`g_pe`), dead math is removed, the march is 96 → 48 steps, and
    lightning is skipped outside its reach.
  - Result: 0 dropped, 3.8 ms per frame.
- **Shader recompile stall.** Setting a big shader stalls about 1.5 s while it compiles. This is fine at edit time
  but must never happen during playback. The invert layer compiles its own copy.
- **Restyle wiped the hand keys.** Only `clip.set_shader` after the first bake.
- **Wrong param index.** Clearing `params[8]` (wave) instead of `[9]` (lyric) wiped 9 339 waveform keys. Undo
  fixed it; scripts now assert the param name.
- **Split clips and key times.** Key times are clip-local, not `src_in`-based, and the first attempt made the text
  invisible.
- **Stretched text and vertical bounce.**
  - Old `width`/`height` keys survived under `scale_x`/`scale_y`. Each line now keys height to a constant.
  - Overlapping lines: the effective end is min(clip end, next start).
- **Text clipping.** Curves and shadows went past the text box, so the shader now insets the content and the clip
  scale goes up to match.
- **Unreadable gold text.** A hard outline looked cheap, so it became a soft 12-tap blurred dark halo; a stronger
  inner bevel was "too 3D".
- **The "red" rift.**
  - A dark violet void with swirling gold read as red and fake. Concentric chord cracks looked like a rectangle
    pattern.
  - Replaced by Voronoi glass shards that just appear, inverted, with no outlines.
  - An expanding inversion circle "hurt the intensity", so more shards appear faster instead.
- **Invert layer misaligned.** The visualizer Shape is 960×540 internally and a 1920×1080 copy drew the shards at
  the wrong size. The layer must copy the visualizer's shape size.
- **Duplicating the visualizer duplicated the song** (they are linked). Trying to unlink through `ui_action link`
  deleted clips. The layer is now built with `shapes.add` and keys are copied from the visualizer.
- **Stem add-on install.** The release URL 404'd; it was installed locally.
- **Unicode in Windows Python output.** Use `PYTHONIOENCODING=utf8` and `encoding='utf8'` everywhere.
- **Electricity too early.** The user wanted the build from vocals into the first drop clean, so everything flashy
  is gated by `fx`. The outro calms via the calm-section strength scaling.

## 5. Proposed plugin design — "Visualizer Helper"

Four one-click helpers, each one undo step, each opening a small panel of knobs:

1. **Analyse Song** (on a song clip).
   - Runs the stems if missing, beat/kick tracking, downbeats, screeches, the vocal envelope (talk vs sing),
     vibrato, the synth envelope, and drop and calm detection.
   - Stores the result on the song asset (a `visualizer_analysis` record) and as markers: drops, calm, the first
     sung word.
   - Panel: tempo range, kick gate sensitivity, a "go harder in the biggest drop" dial, and draggable drop markers
     to correct a drop.
2. **Visualizer Templates.** Presets that read the analysis instead of a re-bake:
   - Spiky ball (Olympus Storm), waveform-only, particles/beams.
   - Exposed knobs: spike length, spines per hit, colours (pre-drop / post-drop), electricity amount, beam
     brightness, ball size, waveform height.
   - Hand keys are written as named key tracks the user can see and edit.
3. **Lyrics Karaoke.**
   - Paste the lyrics; it transcribes if needed, aligns, snaps to vocal onsets and creates the line clips with the
     Lyric Chrome shader.
   - Panel: font, size, alphabet look (normal / Greek / none), entry style (slide + 3D turn / pop / fade), silver
     → gold at the first drop, giant cylinder line yes/no.
   - Word times show as draggable ticks on the clip; moving one re-keys the fill.
4. **Drop FX** (on detected drops): flash + shake, gold switch, background grade (dark → bright → duotone), and
   "Shatter into drop" with a build length and shard speed.

**What it needs from the plugin API / app:**
- Read stems and decoded PCM for a clip (or call `audio.stems` and get the file paths) and run analysis off the UI
  thread.
- A bulk key writer: `clip.keys {clip, property, keys:[[t, v, ease]...]}`, thousands per call, one undo step.
- Create clips (text, shape) and set shaders/effects in a single transaction; read clip shape sizes.
- Song-level uniforms for shaders (drop times, hit counter, analysis curves as textures), so per-song constants
  are not baked into GLSL and the 14-param cap stops mattering.
- A `u_below` sampler or a shader-masked Invert effect, so the shatter needs no second full shader.
- Markers and per-asset metadata storage.
- Long-running `scripts.run` with a budget and a return value, or plugin-side compute (python-equivalent: STFT,
  filters, find_peaks, autocorrelation pitch). The numpy code in `hsv_analyze.py` is a direct reference port
  target.

## 6. Reusable assets

All in `plans/visualizer-helper/`. Usage, end to end:

```text
python hsv_analyze.py SONG STEMS_DIR OUT
python hsv_shaders.py OUT/analysis.json OUT
python hsv_lyrics.py LYRICS.txt WORDS.json STEMS_DIR/vocals.wav OUT --analysis OUT/analysis.json --greek --vis VIS --track N
python hsv_install.py OUT/keys.lua hsv-keys CLIP=VIS      # then scripts.run {name:"hsv-keys"}
python hsv_install.py OUT/lyrics.lua hsv-lyrics           # then scripts.run
python hsv_install.py OUT/background.lua hsv-bg BG=BG     # then scripts.run
python hsv_install.py OUT/shaders.lua hsv-shaders VIS=VIS # then scripts.run (last: the invert layer goes on top)
```

Validation on the original song: the analysis found 483 hits (the hand build had 483), 151 BPM, and drops at
26.39 s (hand: 26.389) and 52.12 s (hand: 52.68). The vocal, vibrato and screech key counts match the hand build.
