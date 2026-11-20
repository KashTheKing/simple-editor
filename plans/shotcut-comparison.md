# Shotcut vs Simple Editor (checked 2026-11-19)

Compared against Shotcut's feature page and its 25.12 ("Higher & Higher", 30 Dec 2025) release notes,
using the rel-131 docs, CHANGELOG, MCP tool list and targeted source greps.
"(unverified)" = no evidence found either way. Sizes are rough: S/M/L.

Cross-cutting findings
- **8-bit compositing.** The GPU compositor uses RGBA8 textures only. HDR export is 10-bit, but the processing in between is 8-bit.
- **Audio filters live on Mixer buses.** A single clip gets one only by being routed to its own bus.

## Missing / partial

### Formats and codecs
- Lottie/Rive animation import: MISSING (M)
- SVG and AVIF image import: MISSING (S)
- ASS/SSA subtitle import: PARTIAL, .srt and .vtt only (S)
- Soft subtitle track in the exported file: PARTIAL, burn-in or a separate .srt only (S)
- Extract subtitles from a media file: MISSING (S)
- Webcam capture: MISSING (M)
- Network stream playback (HLS/RTMP/RTSP): MISSING (M)
- Generators (noise, colour bars, counter/timer): MISSING (S)
- Image-sequence export: MISSING (S)
- Export with alpha (ProRes 4444, PNG MOV, VP9 alpha): MISSING (M)
- 10-bit and linear compositing: PARTIAL, 8-bit in between (L)
- Deinterlace: MISSING (S)
- Full-range option, rotation override: MISSING (unverified) (S)

### Audio
- Audio spectrum and vector scopes: PARTIAL, peak and LUFS meters only (M)
- Filter set: PARTIAL. Missing band pass, notch, expander, declick, mid-side, stereo tools, channel swap/invert, downmix, 15-band EQ and two-pass normalize (M)
- Per-clip audio filters: PARTIAL, via a bus only (M)
- VST2/LV2 plugins: MISSING (L)
- Tone generator: MISSING (S)
- 5.1, quad and Ambisonic layouts: MISSING, stereo and mono only (M)
- Pitch-preserving speed change: MISSING, muted above 2x (M)
- Track auto-fade and seam: MISSING (S)

### Video effects
- Custom luma (gradient image) wipe: MISSING (S)
- Blend modes: PARTIAL. Missing Saturate, Exclusion and the HSL modes (S)
- Eyedropper white balance: MISSING (unverified) (S)
- Track matte or mask from file: PARTIAL, node editor only (S)
- Video denoise, deband, dither, spot remover: MISSING (S)
- Corner pin, lens correction, elastic scale: MISSING (S–M)
- Vibrance, posterize, sepia filters: MISSING (S)
- Drop shadow on any clip: PARTIAL (S)
- Timer and GPS overlays: MISSING (S/M)
- 360° filters: on `claude/parity-360` (six library shaders + 360 Stabilize)
- OpenFX plugins: MISSING (L)

### Editing
- Group and ungroup clips: MISSING (M)
- Sync clips by audio as a menu command: PARTIAL, multicam and MCP only (S)
- Per-filter presets: PARTIAL, Looks save the whole stack (S)
- Batch export from the Library: MISSING (S)
- Slideshow generator: MISSING (S)
- Chapters embedded in the export: PARTIAL, text, CSV and Avid only (S)
- Time display as clock or frames: MISSING (unverified) (S)

### Platform, display and misc
- macOS and Linux: MISSING, Windows only (L)
- UI translations: MISSING (M–L)
- Fully portable mode, with settings next to the exe: PARTIAL (S)
- Stream the output over IP: MISSING (M)
- PSNR/SSIM measurement, file integrity check, bitrate graph: MISSING (S)
- Detailed media properties (codec, streams, bitrate): PARTIAL (S)
- DeckLink/SDI: MISSING (L)
- Configurable grid and title-safe overlay: PARTIAL, safe zones only (S)
- Player zoom at 10% and 25%: PARTIAL (S)
- GPU choice with several GPUs: MISSING (S)
- Screen snapshot (a still of the desktop): MISSING (S)

### Shotcut 25.12 items
- 10/12-bit pipeline and linear colour: MISSING (8-bit compositing)
- Processing Mode setting: MISSING (only GPU preview, Playback Resolution and proxies)
- Filter menus that follow the layout: HAVE (pages switch the Gallery)
- HTML preset browser with thumbnails: HAVE (Gallery HTML tab, 43 templates)
- Screen Recording minimize and record audio: HAVE
- Screen Snapshot: MISSING
- Extract subtitles (mov_text, SSA): MISSING
- Finer WebM bitrate control: PARTIAL
- Opacity above 100%: MISSING (unverified)

## What Simple Editor has that Shotcut doesn't
- Animator and Animate page: blocks, physics, motion paths, speed ramps
- Assistant and MCP server: about 300 tools, permissions, one undo step per request, comments, skills
- Platform Emulator: 101 devices with real platform UI, export with the guide burned in
- HTML/CSS layers, a fast HTML renderer and Web Page Snapshot
- Chat, forum and comment thread layers
- Built-in media player: playlists, visualizers, waveform seek, disc burning
- Text-based editing: word cuts, filler removal, transcript search
- Auto-cut silence, scene-cut and double-take detection
- Beat Aligner and beat sync
- Multicam synced by audio
- Node-graph effects and a GLSL shader editor with a live compile check
- Online library: shaders, transitions, LUTs, templates, icons, skills
- FCP7 XML and EDL interchange, plus Premiere .prproj import
- Pro trims: source tape, slip/slide/roll, dynamic J/K/L trim, dual-frame trim view
- Templates and containers, Looks, Colour Match
- Auto-reframe from tracking, a live Draw tool, shape layers
- Luau scripting with expressions and value linking
- Planner, Moodboard and task timers
- Export: target file size, stems, −14 LUFS, platform cards, Quick Export
- yt-dlp importer, Burn to Disc
- Multi-monitor page layouts, keymap presets, themes imported from other editors
- Size: an 18 MB exe (1.3.1), against Shotcut's 143 MB installer (FFmpeg comes from the system)

## Top 10 gaps to close first
1. Pitch-preserving speed (M)
2. Group and ungroup clips (M)
3. Export with alpha, plus image-sequence export (M)
4. Per-clip audio filters, the missing filter types and a spectrum analyser (M)
5. Subtitles: ASS/SSA import, soft subtitles in MP4/MKV, extraction (S–M)
6. Webcam capture next to the screen recorder (M)
7. Configurable grid and title-safe overlay (S)
8. Video denoise and deinterlace bakes (S)
9. Corner pin and lens correction (S–M)
10. Lottie, SVG and AVIF import (S–M)

Left out on purpose:
- 10-bit and linear compositing (L)
- macOS and Linux (L)
- translations (L)

## Session queue (coordinator), 2026-11-19
Started:
1. Pitch-preserving speed
2. Group/ungroup + Sync by audio
3. Alpha + image-sequence export

Queued, to start as sessions finish:
4. Audio: per-clip filters, missing filter types, spectrum analyser
5. Formats: ASS/SSA import, soft subtitles, subtitle extraction, SVG/AVIF/Lottie import
6. Webcam capture
7. Preview grid/title-safe overlay and zoom levels, video denoise/deinterlace bakes, corner pin and lens correction
8. Shotcut editing-list parity (from the user's paste, 2026-11-19). Audit every item, then build the missing ones:
   - batch export from the Library
   - slideshow/montage generator
   - clip names and comments on timeline clips
   - chapters embedded in the export
   - time format: SMPTE, clock or frames
   - per-filter presets
   - Library: sort by creation/recording date, filter by type, search
   - file browser search and type filter
   - nudge clips and groups
   - save a trimmed clip as its own file
   - import Shotcut .mlt projects (for switchers)
   - a bundled starter asset pack: emoji, sounds (real SFX only), graphics
   Skip what other sessions are already doing: grouping, sync by audio, Lottie, ranged markers.
9. Effects parity: audit Shotcut's whole video-filter list against our effects and the shader library, then add every missing one as native GPU effects or library shaders:
   - old film (dust, grain, projector, scratches, Technicolor), halftone, sketch, mosaic, fisheye, wave, trails, vertigo, nervous, choppy
   - alpha strobe, chroma hold, key spill (simple and advanced), circle crop, deband, dither, drop shadow on any clip, elastic scale, gradient map, posterize, sepia, threshold, scan lines, vibrance, unpremultiply, outline
   - the missing blend modes: Saturate, Exclusion and the four HSL modes
   - wipes, including a custom gradient-image wipe
   - eyedropper white balance
   - generators: colour bars, noise, plasma, counter/timer
   - the noise-reduction variants
   - audio visualization filters, checked against our visualizer styles
10. 360° video: equirectangular mask, equirectangular to rectilinear/stereographic, hemispherical to equirectangular, rectilinear to equirectangular, 360 stabilize, 360 transform.
11. I/O and monitoring:
   - network stream playback (HTTP/HLS/RTMP/RTSP/UDP)
   - full-range input/processing/export option, auto-rotate with manual override
   - detailed media properties panel (codec, streams, bitrate, colour) and a bitrate graph viewer
   - search in recent files
   - Screen Snapshot (a desktop still, next to Screen Recording)
   - choosing a GPU when there are several
   - Contour ShuttlePRO jog/shuttle
   - Rive animation import (rive-cpp is MIT), alongside Lottie in #5
12. Colour pipeline (largest, last): 16-bit float, linear-light GPU compositing and 10-bit end to end, matching Shotcut 25.12. Design doc first, then build behind a Settings ▸ Processing Mode choice.
Not planned (tell the user): Blackmagic DeckLink/SDI (proprietary SDK and hardware), OpenFX plugin hosting (huge; Shotcut's is still a work in progress), Glaxnimate integration (the Animator covers it).
13. Windows installer, shipped next to the portable exe (user said "lets ship both", 2026-11-19). A per-user install with no admin (Inno Setup or WiX, free):
   - Start menu shortcut, plus an optional desktop icon
   - an uninstall entry in Settings ▸ Apps
   - the `.sedit` file association, plus the existing "Edit with Simple Editor" right-click menu and the media-player "Open with"
   - FFmpeg downloaded during setup, or bundled as an option
   - updates in place that keep settings
   - a release script that builds both artifacts, and docs
   - code signing comes later: the user needs a certificate
