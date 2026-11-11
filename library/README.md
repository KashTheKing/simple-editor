# Simple Editor library

Content the app downloads on first use: shader effects, shader transitions, LUTs, HTML + CSS layers
and scenes, icons and assistant skills. Nothing here is compiled into the exe (except a handful of
skills). The app reads `index.json`; `scripts/library-index.ps1` rebuilds it and the thumbnails, and
`cargo test library` fails when a file breaks one of the rules below.

```
library/index.json            generated - never edit by hand
library/effects/*.glsl        shader effects
library/transitions/*.glsl    shader transitions
library/luts/*.cube           3D LUTs
library/html/*.html           HTML + CSS layers and scenes (one file each)
library/icons/*.svg           icons
library/skills/*.md           assistant skills
library/thumbs/*              generated previews
```

Rules for every file:

- File name: `kebab-case`, ASCII, lowercase (`paper-tear.glsl`). The name without extension is the id.
- Original work, or an adaptation of the MIT-licensed gl-transitions collection with a `@credit` line.
  No real logos, brand names, trademarks or copied artwork: "game-style chat bubble", "video-site
  subscribe button", never the brand.
- Everything is MIT-licensed like the app. Text files are UTF-8 without BOM, LF line ends.
- Self-contained: no network, no other file. A project copies what it uses into itself.

## Shader effects (`effects/*.glsl`)

A fragment-shader body (GLSL 3.30 core) with a comment header. The app adds `#version`, the uniforms
and `main()`; the file must not contain `#version`, `precision`, `uniform`, `in`/`out` declarations
or a `main()`.

```glsl
// @name Chromatic Aberration
// @desc Splits red and blue apart toward the edges of the frame.
// @tags lens, rgb, glitch
// @param amount 0.4 0 2
// @param falloff 1.5 0.5 4
vec4 effect(vec4 src, vec2 uv) {
    vec2 d = (uv - 0.5) * amount * 0.02 * pow(length(uv - 0.5) * 2.0, falloff);
    return vec4(texture(tex, uv + d).r, src.g, texture(tex, uv - d).b, src.a);
}
```

Header lines (all `// @key value`, at the top of the file, before any code):

| Line | Meaning |
|---|---|
| `// @name <text>` | required; shown in the Effects list |
| `// @desc <text>` | required; one sentence |
| `// @tags a, b, c` | required; lowercase search words |
| `// @param <id> <default> <min> <max>` | 0 to 8 controls. `<id>` is a lowercase identifier (`[a-z][a-z0-9_]*`); the Inspector shows it as "Glow Size" for `glow_size`; in the code it is a `float` of that name. Every control is keyframeable. Defaults must give a visible, good-looking result. |
| `// @needs previous [n]` | optional; the effect also gets earlier frames (below). `n` = how many frames back the earliest one is, 1 to 8 (default 1); the "ghost camera" / echo look wants 3 to 6 |
| `// @credit <text>` | optional; required for adapted work |

Entry point: `vec4 effect(vec4 src, vec2 uv)`. `src` is the layer's pixel at `uv`; return the new
pixel. (`void mainImage(out vec4, in vec2)` in the shadertoy style also works, with `iResolution`,
`iTime`, `iChannel0`.)

In scope:

| Name | What |
|---|---|
| `sampler2D tex` | the layer (use `texture(tex, uv)`) |
| `vec2 u_res` | layer size in pixels |
| `float u_time` | clip-local seconds (use it for all motion; never assume a frame rate) |
| `float u_scale` | preview scale: multiply every pixel-sized amount by it so preview and export match |
| `luma(vec3)`, `hash12(vec2)`, `hash11(float)`, `rgb2hsl(vec3)`, `hsl2rgb(vec3)`, `hue2rgb`, `sample_clamped(sampler2D, vec2)`, `mask_at`, `apply_mask` | helpers that already exist: do not define functions with these names |
| `sampler2D u_prev`, `sampler2D u_next`, `float u_frames` | only with `@needs previous [n]`: the same layer `n` frames earlier (`u_prev`), `n / 2` frames earlier (`u_next`, the in-between sample), and how many of the two are real (`u_frames` 0, 1 or 2; with 1 both are the same frame). Only footage has earlier frames: on a still, text or nested sequence, in a thumbnail, or when the decoder could not deliver them, `u_frames` is 0 - fall back to `src`. Near the start of the source file the earlier samples are clamped to its first frame. |

Conventions: colours are **straight alpha**, 0..1, not premultiplied; keep `src.a` unless the effect
is about transparency. `uv` is 0..1 with **y = 0 at the top**. Sampling outside 0..1 clamps to the
edge; use `sample_clamped` or handle it. Aspect-correct shapes with `u_res.x / u_res.y`. Keep it
real-time at 1080p: at most ~64 texture samples per pixel, no loops with non-constant bounds. Names
taken: `tex`, `u_*`, `p0`..`p7`, `u1`..`u8`, `v_uv`, `out_color`, `main`, `effect`, the helpers above.

## Shader transitions (`transitions/*.glsl`)

The gl-transitions format, so that collection drops in. Same header (`@name`, `@desc`, `@tags`,
`@param`, `@credit`), same "no `#version` / `uniform` / `main()`" rule.

```glsl
// @name Noise Dissolve
// @desc The incoming picture eats through the outgoing one in a noisy pattern.
// @tags dissolve, noise, organic
// @param grain 60 5 400
// @param softness 0.1 0 0.5
vec4 transition(vec2 uv) {
    float n = hash12(floor(uv * vec2(grain * ratio, grain)));
    float m = smoothstep(n - softness, n + softness, progress * (1.0 + 2.0 * softness) - softness);
    return mix(getFromColor(uv), getToColor(uv), m);
}
```

| Name | What |
|---|---|
| `vec4 transition(vec2 uv)` | the entry point: return the pixel at `uv` |
| `vec4 getFromColor(vec2 uv)` | the outgoing picture |
| `vec4 getToColor(vec2 uv)` | the incoming picture |
| `float progress` | 0 at the start, 1 at the end (already eased by the transition's easing) |
| `float ratio` | width / height |
| `vec2 u_res`, `float u_scale`, the helper functions | as for effects |

Rules:

- **`uv` has y = 0 at the BOTTOM here** (the gl-transitions convention), unlike effects.
- `progress = 0` must return exactly `getFromColor(uv)` and `progress = 1` exactly `getToColor(uv)`:
  no pop at either end.
- Only `progress` drives motion (no `u_time`), so scrubbing is exact.
- Adapting a gl-transitions file: every `uniform float x; // = 0.3` becomes `// @param x 0.3 <min> <max>`;
  other uniform types (`vec2`, `vec3`, `int`, `bool`, `sampler2D`) become `const`s or float params;
  keep the author and licence in `// @credit Author Name, gl-transitions (MIT)`.
- On an edge transition (fade a single clip in or out) the missing side is transparent black.
- Each picture is its clip alone (effects, mask and placement applied) on a transparent frame-sized
  canvas; the result is laid over the tracks below. On the CPU fallback and in XML / EDL export a
  shader transition is a cross fade, and so is one whose source does not compile.
- Either picture may have transparency: return straight alpha.

## LUTs (`luts/*.cube`)

Adobe / Iridas `.cube`, 3D only, readable by Premiere and Resolve.

```
# @name Teal and Orange
# @desc Warm skin, teal shadows: the blockbuster look.
# @tags cinematic, warm, teal
TITLE "Teal and Orange"
LUT_3D_SIZE 17
0.000000 0.000000 0.000000
...                          (17*17*17 = 4913 rows "r g b", red changing fastest)
```

`LUT_3D_SIZE 17` (about 130 KB), six decimals, values 0..1, no `DOMAIN_MIN` / `DOMAIN_MAX`. Input and
output are display-referred sRGB / Rec.709. A LUT must keep black near black and white near white
unless the look is the point (faded matte). LUTs are generated by `scripts/library-luts.py` from the
recipes in that script: change the recipe, not the file.

## HTML layers and scenes (`html/*.html`)

One HTML file with its CSS in a `<style>` block. **No JavaScript**: no `<script>`, no `on*=`
attributes, no `javascript:` URLs (the app strips them and Edge runs with scripts off). Motion is CSS
only: `@keyframes` + `animation`.

```html
<!doctype html>
<html>
<head>
<meta charset="utf-8">
<title>Subscribe Button</title>
<meta name="se:kind" content="layer">
<meta name="se:desc" content="A subscribe button that pops in, gets clicked and turns grey.">
<meta name="se:tags" content="subscribe, button, social">
<meta name="se:size" content="640x220">
<meta name="se:duration" content="4s">
<meta name="se:field" content="label | text | Subscribe">
<meta name="se:field" content="accent | color | #e62117">
<meta name="se:field" content="count | number | 120 | 0 | 100000">
<meta name="se:field" content="avatar | image | ">
<style>
  :root { --accent: {{accent}}; }
  html, body { margin: 0; background: transparent; overflow: hidden; }
  .btn { background: var(--accent); animation: pop .5s cubic-bezier(.2, 1.4, .4, 1) both; }
  @keyframes pop { from { transform: scale(0); } to { transform: scale(1); } }
</style>
</head>
<body><div class="btn">{{label}}</div></body>
</html>
```

| Tag | Meaning |
|---|---|
| `<title>` | required; the name |
| `se:kind` | `layer` (transparent, sits over video) or `scene` (full frame with its own background). Default `layer`. |
| `se:desc`, `se:tags` | required; as for shaders |
| `se:size` | `WxH` in CSS pixels, measured against a **1080-pixel-high frame**. Default `1920x1080`. The app renders at project resolution (a 4K project renders at 2x). Use `1920x1080` when the position in the frame matters (lower thirds, tickers, end screens, every scene). Use a tight size for free-floating things the user drags around (buttons, badges, bubbles), with ~40 px of padding for shadows and overshoot. Even numbers, at most 3840x2160. |
| `se:duration` | default clip length, e.g. `4s` (0.5 to 60). |
| `se:thumb` | optional; the second the preview picture is taken at (default: 60 % of the duration). |
| `se:field` | zero or more editable values: `name \| type \| default` (number: `name \| number \| default \| min \| max`). Types: `text`, `color` (`#rrggbb` or `#rrggbbaa`), `number`, `image` (a file the user picks; empty default). `name` is a lowercase identifier; the Inspector shows `first_line` as "First Line". At most 12. |

Fields: every `{{name}}` in the file is replaced before rendering (text is HTML-escaped; an image
becomes a `file:///` URL, or a transparent pixel when empty). Use them in markup or CSS, e.g.
`:root { --accent: {{accent}}; }`. An unknown `{{name}}` is left as it is, so do not use double
braces for anything else.

Rules:

- Time starts at 0 for every animation (the clip's first frame). Sequence with `animation-delay`.
  Give animations `animation-fill-mode: both` so the look before and after is defined. Finite
  animations should end on a pose that can hold: the user may stretch the clip. Looping
  (`infinite`) animations are fine. The app sets each animation's time per frame, so what you write is
  what renders, at any frame rate.
- Nothing that needs a pointer, a clock or a script: no `:hover`, no `transition` triggered by state,
  no `<video>`, `<audio>`, `<iframe>`, `<canvas>`, forms that need input, `prefers-*` media queries.
- No network: no `http(s)://` URLs, no `@import`, no web fonts. Fonts: installed Windows fonts only,
  always with fallbacks, e.g. `"Segoe UI", Arial, sans-serif`; safe choices are Segoe UI (all
  weights incl. Semibold / Black), Arial, Arial Black, Impact, Bahnschrift, Trebuchet MS, Verdana,
  Georgia, Times New Roman, Consolas, Courier New, Comic Sans MS, Segoe UI Emoji (emoji), Segoe UI
  Symbol. Pictures: inline `<svg>`, CSS shapes and gradients, or `data:` URIs.
- A `layer` keeps `html, body { background: transparent; }`. A `scene` paints the whole frame.
- `html, body { margin: 0; overflow: hidden; }` and lay out against the `se:size` box
  (`width: 100vw; height: 100vh`).
- Readable on any footage: text on a layer needs its own backing, outline or shadow.
- Counting numbers without script: animate a registered custom property
  (`@property --n { syntax: "<integer>"; inherits: false; initial-value: 0; }`) and print it with
  `counter-reset: n var(--n); content: counter(n);`.

## Icons (`icons/*.svg`)

```xml
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor">
<title>Like</title>
<desc>thumbs up, like, social</desc>
<path d="..."/>
</svg>
```

Solid style on a 24 x 24 grid, drawn with `fill="currentColor"` (the app colours it), about 1 unit of
padding, readable at 32 px. `<title>` is the name, `<desc>` the tags. Paths and basic shapes only: no
`<text>`, `<image>`, `<style>`, `<script>`, gradients, filters, ids or external references. At most
2 KB. One consistent family: rounded corners, same visual weight.

## Skills (`skills/*.md`)

A skill is instructions for the assistant, run from chat as `/name arguments`.

```markdown
---
name: captions
description: Transcribe the timeline and add styled captions.
args: "[style, e.g. bold yellow]"
---
Add captions to the open timeline.

1. ...
Style requested by the user: $ARGS
```

Header between the two `---` lines, one `key: value` per line: `name` (required; lowercase letters,
digits and `-`; equals the file name), `description` (required; one line, shown in the `/` list),
`args` (optional hint shown after the name). The body is Markdown. `$ARGS` is replaced by what the
user typed after the name (empty when nothing); a body without `$ARGS` gets the arguments appended as
a last line. Write for an assistant that edits through the app's tools (`docs/customizing.md` lists
them): say what to inspect first, which tools to call, what to check, and when to stop and ask. Name
real tools only. Keep the user in control: one undo group, no export or file deletion without asking.

## index.json (generated)

```json
{ "version": 1, "items": [
  { "id": "transitions/paper-tear", "kind": "transition", "name": "Paper Tear",
    "desc": "…", "tags": ["paper", "tear"], "file": "transitions/paper-tear.glsl", "size": 1834,
    "credit": "", "thumb": "thumbs/transitions-paper-tear.jpg" }
] }
```

`kind` is one of `effect`, `transition`, `lut`, `html`, `icon`, `skill`. Skills have no thumbnail.
