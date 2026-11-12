# Creator Kit: HTML + CSS layers, assistant skills, shader effects and transitions, online library

## Context

You asked for five things at once, and to hand the build to a fresh agent:

1. **HTML + CSS scenes and layers** (no JavaScript) that render inside the editor, export with the
   video, and that AI can write and edit. Other features should be able to build on it.
2. **Assistant skills** in chat (`/captions`, `/shorts`, …), a skills folder, `/create-skill`, AI
   writing skills from a prompt, and a good set of built-in skills.
3. **100+ effects YouTubers actually use**: transitions (paper, fire, explosion, colour swipes…),
   effects (ghost camera…), many LUTs, and template assets (game-style chat bubble, Subscribe and
   Like animations, icons…), made by Opus 5.5 / Sonnet 5.5 sub-agents.
4. **An online library**: content lives in the repo and is downloaded on first use, not shipped in
   the exe.
5. **Adobe compatibility** beyond today's XML / EDL / `.cube` LUTs.

Your answers so far: render HTML with the Edge that ships with Windows; original content plus the
MIT-licensed gl-transitions collection, brand look-alikes without real logos; limited `.mogrt`
import; library in a `library/` folder in the repo, nothing pushed until you say so.

Planned 2026-11-11 on `feat/pro`. Nothing from this request is built yet. The hand-off prompt for the
agent that builds it is at the end of this file. A separate, later plan lives in
`plans/story-boxes/README.md`.

## What exists today (read from `feat/pro`)

| Need | Today | Where |
|---|---|---|
| HTML rendering | None. Edge is installed at `C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe`. | n/a |
| Transitions | A closed list of four (Cross Fade, Fade to Color, Push, Wipe). | `src/model/transition.rs` |
| Effects | 29 fixed kinds plus one Custom Shader effect: own GLSL or Shadertoy style, compiled with error lines, but 8 anonymous knobs `u1..u8`, 3 hard-coded examples, and no tool can write its source. | `src/model/effect.rs:46,375`, `src/engine/shaders.rs:394`, `src/engine/gpu.rs:895`, `src/ui/shader_ui.rs` |
| LUTs | `.cube` files work; the Gallery lists a folder set in Settings ▸ Color. | `src/engine/lut.rs`, `src/ui/gallery.rs:289` |
| Presets / templates | Live only in the local settings file; agents can only "capture from a clip"; slots ("container" clips) already allow swapping footage while keeping edits. | `src/settings.rs`, `src/engine/presets.rs`, `src/model/ops/shapes.rs` |
| Downloads | `curl.exe` fetches whisper models into the cache folder. | `src/engine/transcribe.rs` |
| Alpha media | Transparent `.mov` plays through a stacked-alpha proxy; PNG stills keep alpha. | `src/media/proxy.rs`, `src/media/ffpipe.rs` |
| Assistant | Tasks, Chat, History, model and effort pickers, pinned context, attachments. No skills, no `/` commands. | `src/ui/assistant_ui.rs`, `src/ui/app/assistant_ctl.rs` |
| Scripts folder | Luau scripts are discovered from a folder with a small comment header: the pattern skills will copy. | `src/scripting.rs` |

Five facts from the code that shape the build (each is handled in the design below):

- **Transitions have no shader at all today.** The four kinds are drawn by compositing the two
  clips straight onto the canvas (`engine/gpu.rs` `draw_transition`). A shader transition needs
  the two clips rendered to separate targets first, then one new two-input program.
- **Effect control names are compile-time constants** (`ParamSpec.name: &'static str`,
  `Effect::specs()`), read in about ten places. Named shader controls need that signature loosened.
- **Transparent media loses its alpha on two existing paths**: image sequences are baked to
  opaque H.264 (`ffpipe.rs:572`), and the fast stacked-alpha preview is only built for sources
  taller than the proxy height (`jobs.rs:314`). HTML layers must not go through either as-is.
- **There is no HTTP or WebSocket client in the app**, only `curl.exe`. Talking to Edge needs a
  small hand-written WebSocket client (localhost, no encryption); catalogue fetches use `curl.exe`.
- **The LUT folder setting has no UI** (`Settings.lut_dirs` is only editable by hand, and the
  Gallery rescans those folders every frame). The library replaces that with a managed folder.

## Design

### 1. HTML + CSS layers and scenes

**What the user sees**

- **Add ▸ HTML Layer** (transparent, sits over video) and **Add ▸ HTML Scene** (full frame, its own
  background). Both are ordinary clips: move, trim, scale, rotate, fade, add effects and
  transitions, export.
- An **HTML editor** window (same shape as the shader editor): HTML on the left, CSS below it, a
  live preview on the right that updates as you type, a scrub bar for the animation, and an
  **Ask the assistant** box ("make the button bounce twice", "match my accent colour").
- **Fields instead of code** for everyday use: a template declares its editable values (text,
  colour, number, image) and the Inspector shows them as normal controls. Changing one re-renders
  in the background; the old picture stays until the new one is ready.
- Double-click an HTML clip to edit it. Right-click ▸ **Save to library** to reuse it.

**How it works**

- No JavaScript. Motion is CSS only (`@keyframes`, `animation`, `transition`), which is exactly what
  AI models write well. Scripts in a template are stripped and blocked.
- Rendering: the app runs Edge headless in the background with a throwaway profile, loads the
  HTML, steps the animation clock frame by frame through Edge's DevTools connection (no script in
  the page needed), and captures each frame with a transparent background. Frames are encoded once
  to an alpha video in the cache; the timeline then plays it like any clip. A layer with no
  animation becomes a single PNG.
- The HTML, CSS and field values are stored **inside the project** (on the clip's asset), so a
  project is self-contained and re-renders anywhere. The rendered video is a cache: delete it and
  it rebuilds. A re-render swaps the asset's file in place, so the clip keeps its effects,
  keyframes and position (the existing "bake" path would drop them, so it is not used here).
- Transparency is kept end to end: the render is written as an alpha video the export reads
  directly, and the fast stacked-alpha preview is built for it at any size (today that preview is
  skipped for small sources).
- Rendered at the project's resolution and frame rate, so exports use the same frames you preview.
  Re-renders run as a background job with progress in the Jobs panel.
- Fields are declared in the HTML head and used as `{{name}}` in the markup or as CSS variables:

  ```html
  <meta name="se:size" content="1920x1080">
  <meta name="se:duration" content="4s">
  <meta name="se:field" content="label | text | Subscribe">
  <meta name="se:field" content="accent | color | #e62117">
  ```

- **For AI**: tools `html.create{html, at, duration, name?, fields?}`, `html.get`, `html.set`
  (source or field values), `html.render`, `html.check` (returns a rendered frame as an image so
  the model can look at its own work). The assistant instructions say: for titles, lower thirds,
  buttons, cards, charts and UI mock-ups, write an HTML layer.
- **For other features**: titles, captions, Story Box slates and templates can all use HTML
  layers as building blocks; the library's motion graphics are HTML layers.

**Limits, stated plainly**

- It is rendered, not live: a change takes roughly one to a few seconds for a short layer, longer
  for long or 4K ones. The first frame preview appears almost at once.
- No video or audio inside the HTML, no web fonts or images from the internet (fonts installed on
  the PC and images you add are fine). Network access is blocked for rendering.
- Needs Microsoft Edge, which Windows 10 and 11 include. Without it the layer shows a clear
  "Edge not found" message.
- The frame-stepping method must be proven first (spike, see Order of work). If Edge cannot step
  CSS animations exactly, the fallback is to pause every animation and set its time per frame
  through the same connection.

### 2. Assistant skills

- A **skill** is one Markdown file: a name, a one-line description, optional arguments, then the
  instructions the assistant follows. Skills live in `%APPDATA%\SimpleEditor\skills\`, plus
  built-ins and ones installed from the online library.
- In chat, typing `/` opens a list of skills with their descriptions, filtered as you type.
  `/captions bold yellow` runs the Captions skill with "bold yellow" as its argument. Skills also
  work in the Tasks box.
- A **Skills** tab in the Assistant: every skill with Run, Edit, Duplicate, Delete, "Open skills
  folder", and **New skill**.
- **New skill** asks two things: a name and "what should it do?". **Write it for me** has the
  assistant draft the instructions; you can edit before saving. `/create-skill <what it should do>`
  does the same from chat. Agents get `skills.list`, `skills.get`, `skills.save`, `skills.delete`.
- Skills from the online library show their full text before first run, since a skill is
  instructions for an AI.
- **Built-in skills (about 20)**: `/captions`, `/cut-silences`, `/remove-fillers`, `/shorts`
  (reframe to 9:16, captions, hook), `/chapters`, `/highlight-reel`, `/b-roll`, `/sync-to-beat`,
  `/zoom-punch`, `/clean-audio`, `/color-match`, `/sfx-pass`, `/intro-outro`, `/lower-third`
  (HTML layer), `/title-card` (HTML scene), `/thumbnail-frame`, `/description` (title, description
  and tags from the transcript), `/review` (pacing notes as comments), `/export-all`,
  `/translate-captions`, `/create-skill`. A handful ship in the exe; the rest come from the online
  library.

### 3. Shader effects and transitions without rebuilding the app

- **Named controls for shaders**: `// @param glow 0.5 0 2` lines name, range and default a
  shader's controls; the Inspector shows "Glow" instead of `u1`.
- **Shader effects as library items**: a `.glsl` file with a header (`@name`, `@desc`, `@tags`,
  `@param`) is an effect. It appears in the Effects list and applies like a built-in one.
- **Shader transitions**: a new transition kind that runs a shader with the outgoing picture, the
  incoming picture and a 0..1 progress. It uses the gl-transitions format
  (`vec4 transition(vec2 uv)` with `getFromColor` / `getToColor`), so that MIT collection drops in.
  Under the hood: one new "Shader" transition kind whose source and control values live on the
  transition itself (the kind enum stays a simple copyable value, as ~20 call sites rely on);
  the two clips are drawn to separate targets and one two-input program blends them; drag and
  hover-preview payloads carry the library item's name. On the CPU fallback and in EDL / XML
  export a shader transition degrades to a cross fade.
- **Agent tools**: `clip.set_shader` and `shader.check` return the compiler's log, so an agent can
  write, compile, read the error and fix; `library.save` stores the result.
- **Existing quick prompts become skills** (they are the closest thing the app has today), so
  there is one concept, not two.
- **Ghost camera / echo effects** need earlier frames. The app already hands neighbouring frames
  to motion effects; shader effects get the same two extra inputs when they ask for them
  (`// @needs previous`).

### 4. Online library

- Everything lives in the repo under `library/`:

  ```
  library/index.json            generated catalogue (name, kind, tags, file, size, licence, thumbnail)
  library/effects/*.glsl        shader effects
  library/transitions/*.glsl    shader transitions
  library/luts/*.cube           LUTs (also usable in Premiere and Resolve)
  library/html/<name>.html      HTML + CSS layers and scenes (one file each)
  library/icons/*.svg           icons
  library/skills/*.md           assistant skills
  library/thumbs/*              small previews
  ```

- The Gallery shows installed items and online items together; online ones carry a cloud mark.
  Clicking one downloads it (kilobytes, via the existing `curl.exe` path) and applies it. After
  that it is installed under `%APPDATA%\SimpleEditor\library\` and works offline.
- Whatever a project uses is copied into the project (shader source, HTML, the LUT file beside the
  project), so projects open on machines that never saw the library.
- The app reads the catalogue from the repo's `main` branch on GitHub. Until `library/` is on
  `main`, a setting points it at the local folder so everything is testable. **Nothing is pushed
  without your say-so.**
- Nothing from the library is compiled into the exe. `scripts/library-index.ps1` rebuilds
  `index.json` and thumbnails; a test fails if a file and the index disagree.

### 5. The content (made by sub-agents, checked by the build)

| Kind | Count | Examples | Model |
|---|---|---|---|
| Transitions | 40 | paper tear, page turn, crumple, burn-through, fire wipe, explosion flash, shockwave, colour swipes, ink bleed, glitch, RGB slide, zoom blur, whip pan x4, spin, film burn, light leak, flash, pixelate, blinds, clock, iris shapes, noise dissolve, TV off, VHS rewind, swirl, kaleidoscope, cross-warp, plus adapted gl-transitions (credited) | Opus 5.5 for fire / paper / explosion / liquid; Sonnet 5.5 for the rest |
| Effects | 40 | ghost camera, echo trails, chromatic aberration, CRT, VHS pro, film grain, halation, bloom, anamorphic streak, light leaks, lens flare, tilt-shift, radial and zoom blur, heat haze, rain on glass, snow, fire, sparks, fog, speed lines, halftone, cartoon, sketch, oil paint, pixel sort, glitch blocks, night vision, thermal, old film, duotone, gradient map, fisheye, mirror, handheld shake, zoom punch, beat pulse, cut-out outline and shadow | Opus 5.5 for the temporal and painterly ones; Sonnet 5.5 for the rest |
| LUTs | 30 | teal and orange, warm film, cool noir, bleach bypass, faded matte, vintage 70s, moody green, neon night, golden hour, winter blue, punchy vlog, gaming vibrant, horror, day for night, sepia, high-contrast black and white | Sonnet 5.5 (generated from colour recipes by a script, checked by the app's own `.cube` parser) |
| HTML layers and scenes | 36 | Subscribe button with click, Subscribe + bell, Like pop, bell ring, lower thirds x5, name tag, social handles, end screen, countdown, counter, progress bar, chapter card, quote card, callouts, emoji burst, comment bubble, game-style chat bubble, phone message bubbles, stream chat, notification toast, news ticker, scoreboard, versus card, sale badge, rating stars, poll bars, checklist, meme caption, intro titles x3, outro | Opus 5.5 for the hero ones; Sonnet 5.5 for the rest |
| Icons | 40 | like, bell, share, comment, play, arrows, check, cross, star, heart, fire, lightning, crown, trophy, coin, clock, pin, camera, mic, gamepad, rocket, sparkle, warning | Sonnet 5.5 |
| Skills | 20 | listed in section 2 | Sonnet 5.5 |

About 206 items. Brand look-alikes are drawn in the familiar style with no real logos or names.
Every shader is compiled by a test on the real graphics driver; every HTML file is rendered once
and checked for a non-empty frame; every LUT is parsed. A contact sheet of all thumbnails is
produced for you to look through, because "compiles" is not the same as "looks good".

### 6. Adobe files

- LUTs: `.cube` already works both ways, and the 30 new LUTs are plain `.cube`, usable in Premiere
  and Resolve.
- `.mogrt` (Motion Graphics Templates), limited import: read the name, preview image and editable
  text fields; turn simple Premiere-made templates into editable titles; mark After Effects-made
  ones "preview only", because they need After Effects to render. This part **needs a few sample
  `.mogrt` files from you** and is built last. I will not claim it works until it has been run on
  real files.

## Order of work

| Step | Work | Notes |
|---|---|---|
| 0 | **Spike: HTML rendering.** Prove Edge can step CSS animations frame by frame with a transparent background and how fast it is. | Decides the renderer's method before anything is built on it |
| 0 | **Formats.** Write `library/README.md`: the exact header and file format for effects, transitions, HTML, skills, icons, LUTs. | Lets content agents start in parallel with the engine |
| 1 | **Engine, 4 parallel workstreams**: `html-layer`, `shader-library` (named controls, shader effects, shader transitions, tools), `skills`, `online-library`. | Disjoint files; same worktree protocol as before |
| 1 | **Content, about 10 sub-agents in parallel** writing only under `library/`. | Opus 5.5 / Sonnet 5.5 by difficulty, per the table |
| 2 | **Validate**: compile all shaders, render all HTML, parse all LUTs, build the index and thumbnails; fix what fails; contact sheet. | |
| 3 | **Live test** on the second monitor with a scratch profile; install to the taskbar exe; commit locally. | |
| 4 | `.mogrt` limited import, once sample files exist. | |

## Risks

- **HTML render speed and exactness** are unproven until the spike. Everything else in section 1
  depends on it.
- **Quality of 200 generated items** will vary. The build checks that they work; you judge the
  look from the contact sheet, and weak ones get reworked or cut.
- **Exe size**: the engine work adds code (estimate +200 to 400 KB). Content adds nothing.
- **Online library goes live only after `library/` reaches `main`.**
- **Adobe templates**: After Effects-made `.mogrt` files cannot be rendered. This is a hard limit.

## Verification

- `cargo test` (includes: every library shader compiles on the real driver, every `.cube` parses,
  index matches files, skills parse, idle-repaint tests for new panes and windows), `--selftest`,
  `scripts/size.ps1`.
- Live, scratch profile, second monitor, driven over the co-edit server:
  1. `html.create` a Subscribe layer, change its label field, export, and check the exported frame.
  2. Ask the assistant to "add a lower third with my name"; it writes an HTML layer.
  3. Type `/` in chat, run `/captions`; run `/create-skill` and see the new skill in the folder.
  4. Install an online transition and an effect from the Gallery (local library folder), apply,
     export, check frames.
  5. Open the project on a clean profile: it still renders (content was copied into the project).

## Hand-off prompt for a new agent

Paste this into a fresh session opened on `D:\Projects\simple-editor`:

```text
You are continuing work on Simple Editor (Rust, egui 0.33, Windows). Build the "Creator Kit" plan.

READ FIRST, in this order (all in the worktree D:\Projects\simple-editor\.claude\worktrees\pro,
branch feat/pro, which is where ALL work happens):
1. plans/creator-kit/README.md   - the plan you are executing (design, order, content table)
2. agents.md, goals.md, ARCHITECTURE.md, and the top entries of notes.md (how the last large
   multi-agent build on this branch was run, live-test method, gotchas)
3. plans/story-boxes/README.md   - a separate, later plan; do not build it, but do not block it

HARD RULES
- Work only in that worktree. Never edit D:\Projects\simple-editor\src (my live checkout) and never
  edit src/ui/settings_ui/general.rs. Commit locally on feat/pro when a step is verified. Do NOT
  push, open a PR, or merge unless I say so.
- Never run `taskkill /IM simple-editor.exe` (it closes my own running app). Kill only a test
  instance you started, by process id.
- Test windows open on my second monitor without taking focus: scratch profile via APPDATA and
  LOCALAPPDATA pointing at a temp folder, SE_BACKGROUND=1, "window_rect": [-2520,112,1600,900] in
  its settings.json, and strip every CLAUDE*/ANTHROPIC* environment variable before launching
  (otherwise the app's own assistant fails with "Credit balance is too low"). Drive it with curl
  against http://127.0.0.1:<port>/mcp.
- No new crates. Menus only through ui::menu::*. Every new tool, Action and pane follows the
  registry rules in agents.md (ToolDef row, idle-repaint test). Run scripts/size.ps1 and report
  the exe size change.
- Do not claim something works until you have run it. Report failures plainly.

WHAT TO BUILD, IN ORDER
0a. Spike the HTML renderer before building on it: headless Microsoft Edge
    (C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe), throwaway profile, DevTools
    protocol over a hand-written WebSocket client (no crate; the app has no HTTP/WebSocket client
    today, only curl.exe), transparent background, CSS animations stepped frame by frame with NO
    JavaScript in the page. Keep alpha end to end: do NOT reuse ffpipe::bake_sequence (it encodes
    opaque yuv420p) or the bake pipeline (it drops a clip's effects); write an alpha video, swap
    the asset's file in place, and make the stacked-alpha preview proxy build at any height
    (jobs.rs proxy_eligible skips small sources today). Measure ms per 1080p frame.
    Write the result and the method you chose into notes.md. If exact stepping is not possible,
    say so and stop for my decision.
0b. Write library/README.md: the exact file formats (shader effect header with @name/@desc/@tags/
    @param/@needs, gl-transitions-style transition, HTML layer with se:size/se:duration/se:field
    meta tags and {{field}} placeholders, skill Markdown header, icon and LUT rules).
1.  Engine, as parallel sub-agents with disjoint files (Opus 5.5): html-layer, shader-library,
    skills, online-library - each exactly as designed in the plan.
    At the same time, content sub-agents writing ONLY under library/: use Opus 5.5 for the hard
    items and Sonnet 5.5 for the rest, per the plan's content table (40 transitions, 40 effects,
    30 LUTs, 36 HTML layers/scenes, 40 icons, 20 skills). Original work only, plus adaptations of
    the MIT-licensed gl-transitions collection with credit in each file. No real logos or brand
    names; "game-style chat bubble", not a trademark. HTML is HTML + CSS only: no <script>.
2.  Validate everything: a test that compiles every library shader on the real driver, renders
    every HTML file, parses every .cube; build index.json and thumbnails; fix or cut what fails.
    Produce one contact-sheet image of all thumbnails and send it to me.
3.  Live-test the five checks in the plan's Verification section, fix what they find, build
    `cargo build --profile release-fast`, and copy the exe over
    C:\Users\Kash\Downloads\simple-editor (1).exe (my taskbar shortcut; a 1.1.0 backup sits beside
    it; if the file is locked, tell me instead of closing my app).
4.  .mogrt limited import: only after I give you sample files. Ask me for them at the end.

HOW I LIKE TO WORK
- I do not want to chat. Make sensible decisions, write them down in notes.md, keep going.
- Keep me posted with short status lines. At the end give me: what works (verified how), what
  does not, what you cut, the exe size change, and the contact sheet.
```
