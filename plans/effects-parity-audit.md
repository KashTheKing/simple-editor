# Shotcut video filters vs Simple Editor (parity-effects, 2026-11-20)

Item 9 of `plans/shotcut-comparison.md`. Each Shotcut video filter, what covers it here, and its status.
"Native" = a built-in `EffectKind` or a clip property; "Library" = `library/effects/*.glsl` (online library);
"Other section" = owned by another cloud section, so not built here.

| Shotcut filter | Simple Editor | Status |
|---|---|---|
| Alpha Channel: Adjust | Library `alpha-adjust` | added |
| Alpha Channel: View | Library `alpha-view` | added |
| Alpha Strobe | Library `alpha-strobe` | added |
| Audio Dance / Light / Spectrum / Waveform Visualization | Audio visualizer styles (`audio-visualizers.mdx`), library `spectrum-bars`, `waveform`, `beat-pulse` | have (Dance / Light map to the visualizer's beat-driven styles) |
| Blend Mode | Clip blend mode (19 modes) | completed: Exclusion, Saturate, Hue, Saturation, Color, Luminosity added |
| Blur: Box / Exponential / Gaussian / Low Pass | Native Blur, Motion Blur, library `radial-blur`, `zoom-blur`, `tilt-shift` | have |
| Brightness, Contrast, Gamma, Saturation | Native Color Correction, Levels, Primaries, Hue / Saturation | have |
| Choppy | Library `choppy` | added |
| Chroma Hold | Library `chroma-hold` | added |
| Chroma Key: Simple / Advanced | Native Chroma Key (+ eyedropper) | have |
| Circular Frame / Crop: Circle | Library `circle-crop` (transparent or coloured outside) | added |
| Color Grading / Lift Gamma Gain | Native Primaries | have |
| Contrast | Native Levels / Curves | have |
| Corner Pin | - | other section (parity-preview-tools) |
| Crop: Rectangle / Source | Built-in clip crop | have |
| Deband | Library `deband` | added |
| Distort | Library `wave`, `heat-haze` | have |
| Dither | Library `dither` | added |
| Drop Shadow | Library `cutout-shadow` (clips with transparency) | partial: a shadow outside an opaque clip's own rectangle needs layer padding (not a shader job) |
| Dust / Grain / Scratches (Old Film) | Library `old-film`, `film-grain` | have |
| Elastic Scale | Library `elastic-scale` | added |
| Fade In / Out Video | Clip fades | have |
| Fisheye / Defish | Library `fisheye` (negative strength pinches) | have |
| Flip | Native Flip | have |
| Glitch | Library `glitch-blocks`, `vhs-pro`, native VHS | have |
| Glow | Library `bloom`, `halation`, native Edge Glow | have |
| Gradient | Library `color-gradient` | added |
| Gradient Map | Library `gradient-map` | have |
| Grid | Library `grid-lines` (the Preview's guide grid is parity-preview-tools) | added |
| Halftone | Library `halftone` | have |
| Hue / Lightness / Saturation | Native Hue / Saturation | have |
| Invert Colors | Native Invert | have |
| Key Spill: Simple / Advanced | Library `key-spill` (any key hue) | added |
| Lens Correction | - | other section (parity-preview-tools) |
| Levels | Native Levels | have |
| LUT (3D) | Native LUT | have |
| Mask: Simple Shape / Apply | Effect masks, clip masks | have |
| Mask: From File | Node editor matte | partial (unchanged; separate gap in shotcut-comparison.md) |
| Mirror | Library `mirror` | have |
| Mosaic | Native Pixelate | have |
| Motion Tracker | Blob tracking, `engine::tracking` | have |
| Nervous | Library `nervous` | added |
| Noise: Fast / Keyframes | Library `film-grain` | have (noise generators are parity-formats) |
| Old Film: Projector | Library `film-projector` | added |
| Old Film: Technocolor | Library `technicolor` | added |
| Opacity | Clip opacity | have |
| Outline | Library `cutout-outline` | have |
| Posterize | Library `posterize` | added |
| Reduce Noise: Smart Blur | Library `smart-blur` (real time) | added |
| Reduce Noise: HQDN3D / Wavelet | - | other section (parity-preview-tools: denoise bakes) |
| Rotate and Scale, Size Position Rotate | Clip transform | have |
| Rutt-Etra-Izer | Library `rutt-etra` | added |
| Scan Lines | Library `scan-lines` (also `crt`) | added |
| Sepia Tone | Library `sepia` | added |
| Sharpen | Native Sharpen | have |
| Sketch | Library `sketch` | have |
| Spot Remover | Library `spot-remover` | added |
| Stabilize | Stabilize bake (`export.stabilize`, `BakeKind::Stabilize`) | have |
| Text: Simple / Rich / Typewriter | Native text clips | have |
| Threshold | Native Threshold | have |
| Timer / GPS | - | other section (parity-formats: counter / timer generator) |
| Trails | Library `echo-trails` | have |
| Unpremultiply Alpha | Library `unpremultiply` | added |
| Vertigo | Library `vertigo` | added |
| Vibrance | Library `vibrance` | added |
| Video Zoom | Clip transform / library `zoom-punch` | have |
| Wave | Library `wave` | added |
| White Balance | Native Primaries Temp / Tint, plus the new White Balance eyedropper | completed |
| (Emboss) | Library `emboss` | added (frei0r emboss) |
| Luma wipe (custom gradient image) | Library transition `gradient-wipe` (patterns or any image) | added |
| 360° filters | - | other section (parity-360) |
| Generators: colour bars, noise, plasma, counter | - | other section (parity-formats) |

Not done here, and why:
- Drop shadow outside an opaque clip: effects draw inside the layer's own pixels; it needs the compositor to
  pad the layer (a separate change to `compose.rs` / `gpu.rs`).
- Pillar Echo (blurred fill behind a narrow clip): same reason; the layer has no pixels outside the clip.
- The CPU compositor cross-fades every shader transition (as documented in `library/README.md`), so a gradient
  wipe is a cross fade there and in XML / EDL export.
