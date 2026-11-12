# Story Boxes, agent-built library, and full-video templates

Planned 2026-11-11 on `feat/pro`. Nothing here is built yet. Every "today" statement below was read
from the code on this branch (file:line given), not assumed.

## What you asked for, in one paragraph

Agents should be able to **make** things, not only apply them: effect presets, effect chains, new
effects, shaders, titles, caption styles, whole templates, all saved straight into the library and
used in the project. They should use **real footage** from the libraries you have allowed,
transcribe when dialogue matters, and subtitle automatically. Templates should go as far as an
entire YouTube Short where you only swap footage and subtitles. And there should be **Story
Boxes**: self-contained sections of a video with their own genre, rules, beat sheet and allowed
assets, so the assistant works inside clear boundaries instead of re-reading or guessing the whole
project every time.

## What the code has today

| Area | Today | Where |
|---|---|---|
| Presets | Effect stacks, node graphs, motions, curves, key blocks, text styles and clip templates exist, but live only in the local `settings.json`. No file format to share them. | `src/settings.rs:71-137, 228-467`, `src/engine/presets.rs` |
| Agents and presets | Every save path is "capture from an existing clip". An agent cannot hand over a preset as data, cannot list or delete most kinds, cannot save a caption style at all (no code path), cannot append a preset (apply replaces the whole stack). | `presets.rs:169-180`, `tools_gallery.rs`, `whatsnew.rs:66` |
| Shaders | A Custom Shader effect exists (own GLSL or Shadertoy style, compiled with error lines), but **no tool can write shader source** or return the compile log; its 8 knobs are anonymous `u1..u8`; the "library" is 3 hard-coded examples; no audio input. | `model/effect.rs:46,375,514`, `engine/shaders.rs:394`, `engine/gpu.rs:895`, `ui/shader_ui.rs:20` |
| New effects | The effect list is a closed Rust enum. The only data-driven "new effect" is a shader, or a saved stack/graph. | `model/effect.rs:7-63` |
| Templates | A clip template stores full clips (effects, keyframes, text, slots). **Slots already exist**: "container" clips whose footage can be swapped while keeping every edit. A "project template" is only width/height/fps. | `presets.rs:413-437`, `model/ops/shapes.rs:27-140`, `settings.rs:137` |
| Libraries | Linked folders exist, but only the Library panel can browse them; **no tool lists their files**. `media.import` takes any disk path. | `model/project.rs:15`, `ui/library.rs:2707` |
| Knowing footage | An agent sees name, size, duration, tags, description. It cannot look at an asset without placing it; transcripts are per clip, not per file; scene and audio analysis need a placed clip. | `tools_media.rs:19`, `tools_audio.rs:308` |
| Subtitles | Transcribe ▸ cues ▸ style ▸ animation works end to end, but captions are one global list on the main timeline and are switched off inside nested sequences. | `project.rs:111`, `playback.rs:1885` |
| Nested timelines | Sequences render, mix, open in a tab, and sit on the timeline as one clip. A `Sequence` carries only name, size, fps, tracks: no notes, rules or colour. Only one can be open at a time. | `model/mod.rs:396-413`, `model/ops/sequences.rs` |
| Boundaries | Nothing limits which assets, clips or tracks a tool call may touch. Scripts and tasks skip the approval guard once started. | `mcp_exec.rs:30-69, 209, 260` |
| Sections / beats | Only range markers and the free-form planner tree. Nothing ties rules to a part of the video. | `model/marker.rs`, `model/mod.rs:466` |

## The design

### 1. Story Box = a nested sequence with a brief

A Story Box is a `Sequence` (already a self-contained mini-project that renders into the main video)
plus one new optional field, `story`:

- **genre** and a one-line **logline**
- **rules**: free text the assistant must follow inside this box (tone, pacing, do / don't)
- **beats**: an ordered list, each with a name, a purpose, a kind (hook, A-roll, B-roll, conflict,
  climax, resolution, outro, custom), a target length, its time range in the box, slot hints
  ("needs talking head", "needs B-roll of X"), and a done flag
- **allowed**: the asset ids, library folders and tags this box may draw from
- optional **caption style** and **music rule** for the box
- **status** per beat: empty, drafted, done

On the main timeline a box is one clip with its own tint, its name and beat ticks; double-click opens
it in a timeline tab exactly as sequences open today. Inside, a beat strip sits above the ruler.

Why a sequence and not a new kind of region: it already has its own tracks, its own length, renders
and mixes correctly, moves and trims as one unit, and can be saved and reused. A new region type
would need all of that rebuilt.

### 2. The assistant works inside the box

- **Scoped context.** When a request targets a box, the assistant receives the box brief instead of
  the project: genre, rules, beats with times and status, and a compact list of the allowed assets
  (name, length, description, tags, whether a transcript exists). New `story.get` tool, the same
  block inside `editor.context`, and an MCP resource per box.
- **Scoped edits without stealing your view.** Timeline tools only act on the open timeline today.
  Every tool call gains an optional `box` argument; the app swaps that box's tracks in for the
  duration of the call and back out in the same frame, so the assistant can build box 3 while you
  keep editing box 1. You see changed boxes glow, as edits do now.
- **Enforced boundaries.** One check at the single point every tool call passes
  (`App::handle_tool`), mirrored in the script and task runners that skip it today: inside a box,
  an asset must be in the box's allowed set and a clip must belong to the box, or the call fails
  with the rule it broke. Importing from outside your linked folders asks first.
- **Fill flow.** "Fill with Assistant" on a box (or one beat) queues one task per beat: pick
  footage from the allowed assets, place and trim it to the beat, transcribe if the beat has
  dialogue, caption it, mark the beat drafted. One undo step per beat. This rides on the Tasks tab
  and "Handle my comments" that already exist.

### 3. Story templates (the procedural part)

A story template is a box brief without footage: genre, rules, and a beat sheet with relative
lengths and per-beat instructions. Applying one to a target length creates a box with timed beats
and empty slots.

- Built-ins to start: Short (hook, setup, payoff, loop), three-act, the long form you described
  (intro, B-roll, lead back to story, B-roll, conflict, mega climax, resolution, second climax,
  full resolution, wrap-up), tutorial, review, vlog.
- **Create by talking**: describe the video in the Assistant; it writes the story template (and
  the boxes) through a new `story.template_save` / `story.create` tool pair. You edit the beats in
  the Inspector afterwards.
- A whole video is then a main timeline of boxes, each with its own brief.

### 4. Full-video templates

A template that is "an entire Short, just replace the footage and subtitles" is a project whose
media clips are **slots** (the container clips that already exist), with its boxes, beat sheets,
caption style, titles, effects and music bed in place.

- "Save project as template" strips media into named slots; "New from template" on the home screen.
- A **slot checklist** panel lists every empty slot with its beat and hint; drop footage on a row,
  or let the assistant fill them from the allowed library.
- Captions regenerate from the new footage (transcribe ▸ cues) in the template's style.
- Templates are plain files (`.sedit-template`) in a Templates folder, so they can be shared.

### 5. Agents build the library

- **One tool family, `library.*`**: list, save, delete, rename, apply, export, import, across every
  kind (effect chain, node graph, shader, motion, curve, key block, caption style, title, clip
  template, story template, project template). `library.save` takes the item as data, validates it
  by decoding it with the app's own types, and stores it. This closes "capture only".
- **Apply can append**, not only replace the clip's whole effect stack.
- **Caption styles can finally be saved** (the list is read today but nothing writes it).
- **Share**: `.sedit-pack` files bundle any set of library items; import adds them.
- **Gallery**: tabs for effect chains / graphs and shaders, with thumbnails rendered for user items.

### 6. Shaders, and new effects without Rust

- `clip.set_shader{clip, index, src}` writes the source and returns the compiler's log;
  `shader.check{src}` compiles without touching the project. An agent can iterate until it compiles.
- **Named knobs**: `// @param glow 0.5 0 2` lines at the top of a shader name and range its
  controls; the Inspector shows those names instead of `u1..u8`.
- A saved shader with named knobs **is** a new effect: it appears in the Effects list under Custom
  and is applied like any other.
- Audio-reactive inputs (level, bass / mid / treble) for visualizer-style shaders: second step,
  after the above works.

### 7. Real footage first

- **Browse allowed libraries**: `media.browse{folder}` lists files in linked folders (the
  libraries you have added); those are the only places the assistant imports from without asking.
- **Look before placing**: `media.frames{asset, count}` returns a contact sheet of a file straight
  from the library.
- **Know it once**: per-file transcript, scene cuts and an agent-written description and tags are
  cached with the asset, so the next request does not redo them. `media.search{query}` searches
  across them.
- **House rules** in the server's instructions: prefer footage from the allowed libraries over
  generated filler; transcribe when a beat needs dialogue (island mode, which fixed timing drift);
  caption speech by default.

### 8. Captions per box

Cues gain a box scope (the way markers already have one) and render inside the box, so captions made
in a box stay with it when it moves. Today captions are global and disabled inside nested timelines.

## Order of work

Follows the repo's wave protocol (serial foundations, then parallel workstreams with disjoint files).

| Wave | Workstream | What lands |
|---|---|---|
| 0 | `story-schema` | Model fields (`Sequence.story`, cue scope, asset knowledge cache), library store additions, marker lines, tool namespaces |
| 0 | `nested-perf` | Stop cloning the whole project per frame for nested sequences (known gap, `playback.rs:1875`). Boxes make nested timelines the normal case, so this comes first |
| 1 | `agent-library` | `library.*`, save-from-data, append apply, caption style saving, `.sedit-pack` |
| 1 | `agent-shaders` | `clip.set_shader`, `shader.check`, named knobs, shader as effect, Gallery tab |
| 1 | `media-knowledge` | `media.browse`, `media.frames`, per-file transcript / scenes / description cache, `media.search`, import-outside-library approval |
| 1 | `story-core` | `story.*` tools, `box` argument and track swap, boundary enforcement, scoped context |
| 2 | `story-ui` | Box tint and beat strip, Inspector section (genre, rules, beats, allowed assets), create / make-from-selection |
| 2 | `story-templates` | Built-in beat sheets, create-by-talking, Fill with Assistant per beat |
| 2 | `box-captions` | Per-box cues and rendering |
| 2 | `project-templates` | Save as template, new from template, slot checklist, `.sedit-template` |
| 3 | `docs-and-live-test` | Guide pages, tool tables, a live run building a Short from a template |

A usable first slice is waves 0-1 plus `story-ui`: boxes with briefs, an assistant that stays inside
them, and agent-made presets and shaders.

## Limits and risks, stated up front

- **Taste is not guaranteed.** The assistant picks footage from descriptions, transcripts and
  contact sheets. It does not watch motion. Good descriptions and tight beat instructions matter;
  expect to review each drafted beat.
- **Boundaries are enforced by the app, not the agent CLI**, which only offers all-or-nothing
  access to the editor's tools. The check sits where every call passes, and the plan closes the
  script/task path that skips it today.
- **Only one box can be open in the timeline at a time**, and boxes nest at most 8 deep (existing
  limits). The `box` argument lets agents work in closed boxes; the UI still shows one.
- **Size**: roughly +300 to 500 KB of exe, on top of the +0.5 MB already agreed for the Pro plan.
  Needs a line in `goals.md`.
- **Cost**: filling a long video beat by beat is many assistant requests. Tasks make each one
  visible and stoppable; nothing runs without you starting it or switching "Handle my comments" on.
- **Shader safety**: shaders run on the GPU inside the app's renderer; a bad one can be slow but
  cannot touch files. Compile errors are returned, not shown as a broken frame.

## Decisions I made that you may want to change

1. **A Story Box is a nested sequence**, not a coloured region on the main timeline.
2. **The assistant works in closed boxes** by default (you see a glow when it finishes), rather
   than switching your timeline to the box it is editing.
3. **The library stays in your settings file** as today, with `.sedit-pack` / `.sedit-template`
   files for sharing, rather than moving the whole library to files on disk.
4. **Allowed libraries = your linked folders.** Anything else needs your click.

## Verification, per wave

`cargo test`, `--selftest`, a viewed screenshot for UI, `scripts/size.ps1`, and the structural tests
(every new tool has its row; every new pane or window has its idle-repaint test). Live, on the
second monitor with a scratch profile: (1) an agent writes a shader that fails to compile, reads
the log, fixes it, saves it to the library, and it appears in the Gallery; (2) a box with two
allowed assets rejects a third; (3) "Fill" on a three-beat box places real footage, transcribes and
captions it, one undo step per beat; (4) a Short template opens with empty slots, footage is
dropped in, captions regenerate, export plays.
