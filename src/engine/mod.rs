//! Rendering / mixing / export engine. Pure CPU, RGBA8 + f32 audio. Shared by preview and export.

// ---- ws:assistant-pane ----
pub mod agent;
pub mod analysis;
pub mod autocut;
pub mod bg_probe; // ws:page-switch-freeze: a probe worked out off the UI thread
pub mod blend;
// ---- ws:browser-layer ----
pub mod browser;
pub mod zones; // ws:zones-markers
pub mod sections; // ws:zones-markers: export between markers
pub mod browsers; // the user's web browser (default or picked in Settings)
pub mod cache_lru;
pub mod cache_move; // Settings ▸ General ▸ Cache folder
pub mod audio_tap; // screen recording: system audio (WASAPI loopback) + mic to raw side files
pub mod capture;
pub mod screen_grab; // the Screen Recording pane: live preview + window list
pub mod webcam; // parity-webcam: webcam source, devices / formats, the bubble
pub mod rec_pill; // Hide on Record: the Stop pill + the global stop hotkey
pub mod wgc; // application / game capture (Windows Graphics Capture)
pub mod compose;
pub mod convert;
// ---- ws:interchange ----
pub mod edl;
pub mod effects;
// ---- ws:export-encoders ----
pub mod encoders;
pub mod size_probe; // ws:export-size
pub mod export_doctor; // ws:forum-export
pub mod export;
pub mod gallery;
pub mod gallery_sync;
pub mod gpu;
pub mod folder_import;
pub mod linked_bins; // linked bins: a folder on disk browsed as a library bin
pub mod import;
pub mod lut;
pub mod mixer;
pub mod mixer_fx;
pub mod stretch; // pitch-keeping time stretch (Keep Pitch clips, the shuttle)
pub mod audio_tools; // ws:parity-audio
pub mod prerender;
pub mod presets;
pub mod references; // feat/reference-clips: Find a Reference
pub mod retro; // feat-retro-2000s-pack: the Classic 2000s pack
pub mod slideshow; // feat-retro-2000s-pack: Make Slideshow
pub mod templates; // ws:animator-templates
pub mod html;
pub mod blitz; // ws:html-layer: the fast renderer (se-html helper)
pub mod enhance; // the AI enhancer add-on (Real-ESRGAN, downloaded on demand)
pub mod deepfilter; // speech denoise add-on (DeepFilterNet CLI, downloaded on demand)
pub mod fast_whisper; // ws:addon-faster-whisper: the faster-whisper transcription add-on
pub mod demucs; // ws:addon-demucs: stem separation add-on (Demucs htdemucs via demucs.cpp, downloaded on demand)
pub mod sam2; // object masks: the SAM 2 add-on (downloaded on demand) and its mask cache
pub mod bgremove; // ws:addon-bg-removal: background removal add-on (MODNet / BiRefNet, downloaded on demand)
pub mod html_lint;
#[cfg(test)]
mod blitz_tests;
#[cfg(test)]
mod social_text_tests; // ws:social-text
pub mod library;
pub mod stock; // ws:gallery-stock
pub mod plugins; // feat/plugins: plugin.json bundles, install from GitHub, Luau panels
pub mod shaderlib;
pub mod skills;
pub mod webcap;
// ---- ws:browser-layer ----
pub mod webpage;
pub mod shaders;
pub mod shapes;
// ---- ws:export-audio-tracks ----
pub mod stems;
pub mod style;
// ---- feat/styles-context ----: Editing Styles library, Learn this style, the SmolVLM2 describer add-on
pub mod style_learn;
pub mod styles;
pub mod vlm;
pub mod subtitles;
pub mod text;
// ---- ws:thread-viewers ----
pub mod thread;
pub mod tracking;
pub mod transcribe;
// ---- ws:transcript-captions ----
pub mod tts;
pub mod tts_addon; // ws:addon-voice: Kokoro / Chatterbox voices, downloaded on demand
pub mod visualizer; // ws:audio-visualizer
// ---- ws:music-burn ----
pub mod disc;
pub mod mixvideo;
pub mod tags;
// ---- ws:voice-input ----
pub mod voice;
pub mod xmeml;
pub mod media_dates; // ws:parity-editing: Library ▸ Sort by Date Created / Date Recorded
// ---- ws:parity-formats ----
pub mod formats;
pub mod lottie;
pub mod synth;
