//! Rendering / mixing / export engine. Pure CPU, RGBA8 + f32 audio. Shared by preview and export.

// ---- ws:assistant-pane ----
pub mod agent;
pub mod analysis;
pub mod autocut;
pub mod bg_probe; // ws:page-switch-freeze: a probe worked out off the UI thread
pub mod blend;
// ---- ws:browser-layer ----
pub mod browser;
pub mod browsers; // the user's web browser (default or picked in Settings)
pub mod cache_lru;
pub mod cache_move; // Settings ▸ General ▸ Cache folder
pub mod audio_tap; // screen recording: system audio (WASAPI loopback) + mic to raw side files
pub mod capture;
pub mod screen_grab; // the Screen Recording pane: live preview + window list
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
pub mod prerender;
pub mod presets;
pub mod templates; // ws:animator-templates
pub mod html;
pub mod blitz; // ws:html-layer: the fast renderer (se-html helper)
pub mod enhance; // the AI enhancer add-on (Real-ESRGAN, downloaded on demand)
pub mod html_lint;
#[cfg(test)]
mod blitz_tests;
#[cfg(test)]
mod social_text_tests; // ws:social-text
pub mod library;
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
pub mod subtitles;
pub mod text;
// ---- ws:thread-viewers ----
pub mod thread;
pub mod tracking;
pub mod transcribe;
// ---- ws:transcript-captions ----
pub mod tts;
pub mod visualizer; // ws:audio-visualizer
// ---- ws:music-burn ----
pub mod disc;
pub mod mixvideo;
pub mod tags;
// ---- ws:voice-input ----
pub mod voice;
pub mod xmeml;
