# Template sample footage: sources and licence

Every file in this folder was **generated** for Simple Editor with ffmpeg's built-in `gradients` source
(`-f lavfi -i gradients=...`, seeded per file, with a slow zoom), 960x540, 30 fps, H.264, no sound. No
third-party footage, no recordings and no music are in it. They are placeholders that show a template's
cuts and motion until you drop your own clips in.

Licence: CC0 1.0 (public domain dedication) - use them for anything.

| File | Source | Licence |
|---|---|---|
| hype-montage/city-bridge.mp4 | generated (ffmpeg gradients, linear) | CC0 1.0 |
| hype-montage/glass-break.mp4 | generated (ffmpeg gradients, radial) | CC0 1.0 |
| hype-montage/podium.mp4 | generated (ffmpeg gradients, circular) | CC0 1.0 |
| hype-montage/throw-pc.mp4 | generated (ffmpeg gradients, spiral) | CC0 1.0 |
| hype-montage/zoom-out.mp4 | generated (ffmpeg gradients, linear) | CC0 1.0 |
| meme-cut/pc-on-fire.mp4 | generated (ffmpeg gradients, radial) | CC0 1.0 |
| meme-cut/rage-quit.mp4 | generated (ffmpeg gradients, circular) | CC0 1.0 |
| meme-cut/xbox-in-bed.mp4 | generated (ffmpeg gradients, spiral) | CC0 1.0 |
| velocity-edit/car-iced.mp4 | generated (ffmpeg gradients, linear) | CC0 1.0 |
| velocity-edit/city-cinematic.mp4 | generated (ffmpeg gradients, radial) | CC0 1.0 |
| velocity-edit/frozen-npcs.mp4 | generated (ffmpeg gradients, circular) | CC0 1.0 |
| velocity-edit/glass-break.mp4 | generated (ffmpeg gradients, spiral) | CC0 1.0 |
| wasted/hit-by-truck.mp4 | generated (ffmpeg gradients, linear) | CC0 1.0 |

(The file names are the slots' sample names; they say nothing about the content.)

## Not shipped

- **Songs and sound effects.** The edit templates' song (`song.m4a`) and the Wasted sting (`wasted.m4a`)
  are never in this folder: a template uses them only from your own folder,
  `%APPDATA%\SimpleEditor\gallery\templates\media\<template>\<file>`, which also overrides any
  footage here. Without one, that clip is left out.
- **The Pricedown font** (Wasted's word) is not bundled: Typodermic's free licence does not allow shipping
  it in an app or an editable template. Install it yourself and the card uses it; otherwise Impact.
