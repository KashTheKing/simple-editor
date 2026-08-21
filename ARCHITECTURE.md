# Simple Editor — architecture & module contracts

A tiny, fast video trimmer/editor for Windows. Rust + egui. Single binary (< 50 MB, typically ~10 MB).

Priorities, in order: **performance (startup, seek, playback, export), low memory/CPU/GPU, frictionless
open → cut → save, then everything else.** Bare Windows-Forms-style UI matching the OS theme; DaVinci-like
layout (library | preview | inspector / timeline) without styling.

## Stack

| Concern | Choice | Why |
|---|---|---|
| UI | `eframe`/`egui` 0.33 (glow backend) | immediate mode, custom-painted timeline trivial, repaints only on input/playback, ~8 MB |
| Decode | Windows **Media Foundation** (`windows` crate) — `media/mf.rs` | native, hardware decode, zero deps, instant seeks |
| Decode fallback / images / probe / export | `ffmpeg.exe` + `ffprobe.exe` child processes — `media/ffpipe.rs` | universal codecs, any output container; already on most machines |
| Audio out | `cpal` (WASAPI) | small pure-Rust |
| Text | `fontdb` (system fonts) + `ab_glyph` (raster) | small; outline = dilated coverage, shadow = offset+blur |
| Settings / project | `serde_json` | one settings file, `.sedit` project JSON |
| Registry (theme/accent/context menu) | `winreg` | tiny |
| Dialogs | `rfd` | native Windows dialogs |

Frames are **top-down RGBA8** everywhere (`media::Frame`). Audio is **interleaved stereo f32 @ 48 kHz**.

## Module map (file = owner; keep files disjoint)

```
src/main.rs            entry: args (--selftest, --screenshot), eframe run
src/model.rs           Project/Track/Clip/Asset/Animated/TextStyle + all edit ops (split, ripple, move, trim) + JSON
src/settings.rs        Settings (JSON in %APPDATA%\SimpleEditor), recent assets/projects
src/hotkeys.rs         Action enum, defaults, parse/format, poll(ctx)
src/theme.rs           system dark/light + accent → egui visuals, Segoe UI fonts; Palette for custom painting
src/contextmenu.rs     HKCU "Edit with Simple Editor" under SystemFileAssociations\.ext per video extension
src/media/mod.rs       Frame, VideoSource/AudioSource traits, Backend, probe/open dispatch, DecoderPool
src/media/mf.rs        Media Foundation decoders + probe                     (agent: media-mf)
src/media/ffpipe.rs    ffmpeg/ffprobe process decoders + probe + exe lookup  (agent: media-ffpipe)
src/media/waveform.rs  peaks cache (background compute, disk cache)          (agent: media-ffpipe)
src/engine/blend.rs    blend modes + composite                               (agent: engine-render)
src/engine/compose.rs  Compositor: timeline → RGBA canvas; placement()       (agent: engine-render)
src/engine/text.rs     TextRasterizer                                         (agent: engine-render)
src/engine/mixer.rs    Mixer: timeline → f32 stereo                           (agent: engine-render)
src/engine/export.rs   ffmpeg export (pipe), lossless cut, encoders, codec args (agent: engine-export)
src/engine/xmeml.rs    FCP7 XML for Premiere/Resolve                          (agent: engine-export)
src/playback.rs        Player: render thread + audio thread + clock           (agent: playback)
src/ui/app.rs          App: layout, menus, actions, undo, file ops, export flow (owner: integrator)
src/ui/timeline.rs     timeline widget                                        (agent: ui-timeline)
src/ui/preview.rs      preview + transport                                    (agent: ui-preview-inspector)
src/ui/inspector.rs    properties + keyframes + text settings                 (agent: ui-preview-inspector)
src/ui/library.rs      library + recent panels                                (agent: ui-library-settings)
src/ui/settings_ui.rs  settings window incl. hotkey editor                    (agent: ui-library-settings)
src/selftest.rs        headless end-to-end check                              (agent: selftest)
```

Every public signature in the stub files is the contract. **Do not change a signature in a file you
don't own**; if you truly need something, add a private helper in your own file or report it.

## Key behaviours

* **Open a video** → `Project::from_media(asset)`: project size/fps from the video, V1 with the clip,
  one audio track per audio stream (A1, A2, …), all linked. `source_video` set → **Save (Ctrl+S) =
  re-encode over the original** (temp file in the same folder, `Player::release_files()`, rename) — or the
  instant `-c copy` cut when `Settings.lossless_save` is on and `export::lossless_segments` applies.
* **Export As** → any extension; codec by extension/settings; Compositor frames piped to ffmpeg stdin,
  audio pre-mixed to a temp WAV. **Fast Lossless Cut** → `-c copy` when the project is a pure cut.
* **Preview == export**: both use `Compositor::render` and `Mixer::mix`, only the canvas size differs.
* **Undo** = JSON snapshots (`push_undo()` before each mutation; panels receive an `undo` closure and call
  it once per gesture *before* mutating).
* **Keyframes**: `Animated { value, keys }`, clip-local time, linear. Inspector "◆" toggles a key at the
  playhead; editing an animated property writes a key at the playhead (`set_at`).
* **Mute/Solo** per track (`Project::active(track)`); video tracks support it too (as "disable track").
* **Linked clips** (`Clip.link`) move/split/delete together; Ctrl+L toggles.
* **Placement** (`engine::compose::placement`): contain-fit into the canvas, then scale about centre,
  offset (project px), rotate. Text layers are rendered at canvas scale (no contain fit).
* **Theme**: `theme::apply` at start and when settings change; custom painting uses `theme::palette(ctx)`.
* **Context menu** installed on first run if `settings.context_menu` (HKCU, no admin).

## Threading

* UI thread: egui. Never blocks on media.
* `Player` render thread: owns a `DecoderPool` + `Compositor`; receives commands (mpsc); publishes
  `Arc<Frame>`; calls `ctx.request_repaint()`.
* `Player` audio thread: owns its own `DecoderPool` + `Mixer`; fills a ring buffer consumed by the cpal
  callback; wall clock is the master; ~120 ms lead.
* Export thread: its own pool/compositor/mixer. Waveform threads: their own `AudioSource`.
* MF objects are created and used on one thread each; wrap in a newtype with `unsafe impl Send` only
  when the object is genuinely moved (never shared) across threads, and initialise COM/MF on each thread
  that touches them (`CoInitializeEx(COINIT_MULTITHREADED)`, `MFStartup`).

## Conventions

* Rust 2021, `cargo fmt`, no warnings (`cargo build` must be clean of errors; warnings acceptable only
  for `#[allow(dead_code)]` scaffolding).
* No new dependencies without a strong reason — check the ladder: std → already-installed crate → code.
* Mark deliberate shortcuts with `// ponytail: <what> — <upgrade path>` comments.
* Any non-trivial logic leaves a test (`#[cfg(test)]`) or is covered by `--selftest`.
* Run `cargo test` + `cargo run -- --selftest` before declaring done.

## Verification

* `cargo test` — model/hotkey/engine unit tests.
* `cargo run -- --selftest` — headless media + engine + export check against ffmpeg-generated media.
* `cargo run -- <video> --screenshot out.ppm` — renders the UI and saves a screenshot (convert with
  `ffmpeg -i out.ppm out.png`).
