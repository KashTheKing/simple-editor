//! App settings — one JSON file at %APPDATA%\SimpleEditor\settings.json.

use crate::theme::PaletteOverride;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RecentAsset {
    pub path: String,
    /// Unix seconds.
    pub last_used: u64,
    pub tags: Vec<String>,
    /// 0 = none, 1..=8 = LABEL_COLORS index + 1.
    pub label: u8,
    /// Pinned entries stay at the top and are never evicted by the cap.
    pub pinned: bool,
}

impl Default for RecentAsset {
    fn default() -> Self {
        Self { path: String::new(), last_used: 0, tags: Vec::new(), label: 0, pinned: false }
    }
}

/// A saved editor layout (egui_tiles tree + popped-out panes), as JSON (see ui/layout.rs).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct LayoutProfile {
    pub name: String,
    pub json: String,
}

/// A shareable theme — what "Export theme…" writes to a `.sedit-theme` file and
/// "Import theme…" reads back (palette override + look + the base theme pref).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ThemeFile {
    pub name: String,
    /// "system" | "dark" | "light" (Settings::theme)
    pub theme: String,
    /// "cozy" | "sharp" (Settings::ui_look)
    pub ui_look: String,
    pub palette: PaletteOverride,
    /// Background image settings ride along (the path only makes sense on machines that have the file).
    pub bg_image: String,
    pub bg_tint: [u8; 4],
    pub bg_blur: u8,
    pub panel_opacity: u8,
}

impl Default for ThemeFile {
    fn default() -> Self {
        Self {
            name: String::new(),
            theme: String::new(),
            ui_look: String::new(),
            palette: PaletteOverride::default(),
            bg_image: String::new(),
            bg_tint: [0, 0, 0, 120],
            bg_blur: 0,
            panel_opacity: 255,
        }
    }
}

/// A saved keyframe curve for one property. Key times are normalised to 0..1 of the clip length unless
/// `absolute` (seconds from the clip start); values are absolute.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct CurvePreset {
    pub name: String,
    pub keys: Vec<crate::model::Keyframe>,
    pub absolute: bool,
}

/// A saved motion: curves for several properties at once ("slide in", "Ken Burns", …).
/// Property names as in `Clip::props_mut` labels ("Position X", "Scale", …) or "<Effect>: <Param>".
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct MotionPreset {
    pub name: String,
    pub props: Vec<(String, CurvePreset)>,
}

/// A saved effect chain (a `Vec<Effect>` as JSON) or node graph (a `NodeGraph`), captured from a clip
/// by engine::presets. Machine-local like the other presets: it lives here, never in the project file.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct EffectPreset {
    pub name: String,
    pub json: String,
}

impl EffectPreset {
    /// Which of the two it holds — a graph serialises to an object, an effect stack to an array.
    pub fn is_graph(&self) -> bool {
        self.json.trim_start().starts_with('{')
    }
}

/// A reusable group of clips (relative times) + the assets they need, serialised by engine::presets.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct Template {
    pub name: String,
    pub json: String,
}

/// A saved text look (font/size/bold/italic/colour/letter-spacing) applied to a whole text clip or to
/// a selected range within one (as a `crate::model::TextSpan`). Only the fields the rasterizer actually
/// honours per-span are captured here — outline/shadow stay clip-wide (see `TextSpan`'s doc comment).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TextPreset {
    pub name: String,
    pub font: String,
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    pub color: [u8; 4],
    pub letter_spacing: f32,
}

impl Default for TextPreset {
    fn default() -> Self {
        Self {
            name: String::new(),
            font: "Segoe UI".into(),
            size: 72.0,
            bold: false,
            italic: false,
            color: [255, 255, 255, 255],
            letter_spacing: 0.0,
        }
    }
}

/// A saved project format ("My podcast 4K"): applied from the inspector's Presets section.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ProjectTemplate {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
}

// ---- ws:export-deliver ----
/// What Quick Export re-runs: a platform tile by name, or the custom size/container the Export
/// window was last confirmed with (`width`/`height` 0 = project size).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum ExportPresetRef {
    Preset(String),
    Custom { ext: String, width: u32, height: u32 },
}

fn default_loudnorm() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Directory containing ffmpeg.exe / ffprobe.exe. Empty = app dir, then PATH.
    pub ffmpeg_dir: String,
    /// Directory containing yt-dlp.exe (the optional URL import). Empty = app dir, then PATH.
    pub ytdlp_dir: String,
    /// Where URL downloads are written. Empty = the user's Videos folder.
    pub download_dir: String,
    /// "auto" (libx264 / libvpx-vp9 by extension) or an ffmpeg encoder name
    /// ("libx264", "h264_nvenc", "h264_qsv", "h264_amf", "libx265", "hevc_nvenc", ...).
    pub encoder: String,
    /// Quality (CRF / CQ), lower = better. 18 is visually lossless for x264.
    pub crf: u32,
    /// x264/x265 preset ("ultrafast".."veryslow"); NVENC/QSV/AMF map to their own presets.
    pub preset: String,
    pub confirm_overwrite: bool,
    /// Save (Ctrl+S) over the opened video uses the instant `-c copy` cut when the project is a plain cut
    /// (cuts snap to keyframes) instead of re-encoding.
    pub lossless_save: bool,
    /// Register "Edit with Simple Editor" in the Explorer context menu for videos.
    pub context_menu: bool,
    /// "system" | "dark" | "light"
    pub theme: String,
    /// "auto" | "mf" | "ffmpeg"  (auto = Media Foundation, ffmpeg fallback)
    pub decoder: String,
    /// Preview is rendered at most this wide (pixels) to keep CPU low.
    pub preview_max_width: u32,
    pub snap: bool,
    pub show_library: bool,
    pub show_inspector: bool,
    /// Action id -> shortcut text ("Ctrl+Shift+B"); only non-default bindings are stored. "" = unbound.
    pub hotkeys: BTreeMap<String, String>,
    pub recent_assets: Vec<RecentAsset>,
    pub recent_projects: Vec<String>,
    /// Current editor layout as JSON (empty = default layout).
    pub layout: String,
    /// Saved layout profiles (also exportable to / importable from `.sedit-layout` files).
    pub layout_profiles: Vec<LayoutProfile>,
    /// Keyframe / motion presets, effect-chain / node-graph presets and clip templates
    /// (global, reusable across projects — the Presets pane lists all of them).
    pub curve_presets: Vec<CurvePreset>,
    pub motion_presets: Vec<MotionPreset>,
    pub effect_presets: Vec<EffectPreset>,
    pub templates: Vec<Template>,
    /// Saved text-clip looks (font/size/bold/italic/colour/letter-spacing), appliable to a whole clip
    /// or a selected span. Also exportable to / importable from `.sedit-textstyle` files.
    pub text_presets: Vec<TextPreset>,
    /// Extra font files (.ttf/.otf) imported by the user (loaded by the text rasterizer + font lists).
    pub user_fonts: Vec<String>,
    /// Export: output scaling when the output size differs from the project
    /// (ffmpeg scale flags: "neighbor" | "bilinear" | "bicubic" | "lanczos" | "area" | "spline").
    pub export_scaler: String,
    /// Export: "project" or "WxH".
    pub export_resolution: String,
    /// MCP server (AI co-editing) on 127.0.0.1:mcp_port/mcp.
    pub mcp_enabled: bool,
    pub mcp_port: u16,
    // ---- round 3 ----
    /// Render the preview on the GPU (OpenGL) when a context is available.
    pub gpu: bool,
    /// Preview render scale in percent (100 = full canvas; lower is faster).
    pub preview_quality: u32,
    /// "Movie mode": play back pre-rendered full-quality frames.
    pub movie_mode: bool,
    /// Preview plays background-built all-intra proxies instead of the originals.
    pub use_proxies: bool,
    /// Proxy height in pixels (width keeps aspect).
    pub proxy_height: u32,
    /// Playback cache RAM in MB; 0 = automatic (a quarter of installed RAM, clamped 512 MB-4 GB).
    /// See `playback::cache_budget_bytes`. The decoded-source cache rides at a quarter of this.
    pub cache_mb: u32,
    /// User icon picks: "action.<Action>" / "pane.<Pane>" -> glyph name, or "none" to remove.
    pub icon_overrides: BTreeMap<String, String>,
    /// Screen capture defaults.
    pub capture_fps: u32,
    pub capture_bitrate_kbps: u32,
    pub capture_mic: String,
    pub capture_desktop_audio: bool,
    pub capture_cursor: bool,
    /// Start/stop screen recording automatically when the editor loses/gains focus.
    pub capture_on_blur: bool,
    pub capture_dir: String,
    /// Voiceover defaults.
    pub voice_device: String,
    pub voice_channels: u32,
    /// Export-frame defaults ("project" | "2x" | "4x" | "WxH") and image format.
    pub frame_resolution: String,
    pub frame_format: String,
    pub frame_quality: u32,
    /// Stock image the effects panel renders its thumbnails from (empty = the embedded default).
    pub effect_thumb_image: String,
    /// Settings > Appearance: override for the custom-painted (timeline/preview/waveform) palette.
    /// Default ("system" mode, every colour unset) reproduces today's Windows-derived palette exactly.
    pub palette: PaletteOverride,
    /// Directory containing a whisper.cpp binary (whisper-cli.exe). Empty = the cache's `whisper`
    /// folder, then the app dir, then PATH. Speech-to-text is off until a model is downloaded.
    pub whisper_dir: String,
    /// Model the Subtitles pane offers first: a `engine::transcribe::MODELS` name or file name.
    pub transcribe_model: String,
    /// Library asset preview: draw a heartbeat-style waveform trace over audio-only files that have
    /// no thumbnail. Off = plain black, on = the trace. Toggled from the preview's right-click menu.
    pub audio_visualizer: bool,
    /// Social-guide overlay drawn over the preview (None = off). A view preference, not project data.
    pub guide: Option<crate::ui::guides::Guide>,
    /// User-saved project formats, applied from the inspector's Presets section.
    pub project_templates: Vec<ProjectTemplate>,
    /// UI look: "cozy" (rounded corners, soft button borders) or "sharp" (the old flat look).
    pub ui_look: String,
    /// Editor background image (empty = none), its tint (RGBA over the image) and load-time blur radius.
    pub bg_image: String,
    pub bg_tint: [u8; 4],
    pub bg_blur: u8,
    /// Panel/tab background opacity over the background image (255 = opaque, ignored without an image).
    pub panel_opacity: u8,
    // ---- ws:registries-schema-hooks ----
    /// "dynamic" (contextual, auto-surfacing panels) or "granular" (classic fixed multi-panel);
    /// consumed by ws:layout-modes-onboarding (wave 2).
    pub layout_mode: String,
    /// The first-run welcome window has been shown (or dismissed) once; consumed by
    /// ws:layout-modes-onboarding (wave 2).
    pub onboarded: bool,
    /// Autosave interval in seconds (0 = off); consumed by ws:forgiveness (wave 1).
    pub autosave_secs: u32,
    // ---- ws:size-diet ----
    /// Last window rect [x, y, w, h] in physical pixels — replaces eframe's removed `persistence`
    /// feature for the window rect specifically. `None` until the window has moved/resized once.
    /// Written (debounced) by `winpos::tick`; read once by `main.rs` to seed the `ViewportBuilder`.
    pub window_rect: Option<[i32; 4]>,
    /// `env!("CARGO_PKG_VERSION")` last seen at startup — compared on the next launch to gate the
    /// What's New window (a version bump opens it once).
    pub last_seen_version: String,
    // ---- ws:split-god-files ----
    // ---- ws:audio-analysis ----
    /// Default ducking depth in dB, editable per-call in the Duck section / `audio.duck`'s `depth_db`.
    pub duck_depth_db: f32,
    /// Default fade length (ms) either side of a duck window.
    pub duck_ramp_ms: u32,
    /// Onset sensitivity: local-mean multiplier an envelope bucket must exceed to count as a beat.
    pub beat_thr: f32,
    // ---- ws:audio-dsp-automation ----
    // ---- ws:color-engine ----
    /// User-browsable folders scanned for `.cube` files by a later LUT-browser UI (inspector-gallery,
    /// wave 2) — this workstream only stores the setting, no UI reads it yet.
    pub lut_dirs: Vec<String>,
    // ---- ws:command-palette ----
    /// UI zoom factor (`ctx.set_zoom_factor`); the Hotkeys tab's scale slider.
    pub ui_scale: f32,
    /// Applied keymap preset name (`keymaps::PRESETS`).
    pub keymap_preset: String,
    /// Palette command ids (`Action::id()`, `"pane.<title>"`, a tool name, ...), most-recent-first,
    /// capped 20 — shown when the palette's query is empty instead of the full unsorted list.
    pub palette_recent: Vec<String>,
    // ---- ws:forgiveness ----
    /// Warn (toast, never block) when opening a project whose `.lock` sidecar shows another instance
    /// may already have it open. Not a real mutex — see the PR body's risks note.
    pub lock_warn: bool,
    // ---- ws:player-rate-loop ----
    /// Emit one BLOCK (~21 ms) of audio on every paused playhead change (scrub feedback); gates
    /// `playback_ctl::tick`'s scrub-on-paused-change hook.
    pub audio_scrub: bool,
    /// Symmetric pre/post roll (seconds) for Play Around Playhead.
    pub preroll_secs: f32,
    // ---- ws:snap-engine ----
    /// Markers (project + clip-local) count as timeline snap candidates. No per-field serde
    /// attribute: `Settings`' container-level `#[serde(default)]` already back-fills old files,
    /// exactly like the sibling `snap` field.
    pub snap_markers: bool,
    // ---- ws:trim-model ----
    // ---- ws:canvas-handles-monitor ----
    /// Let an effect / transition / look hover (or `preview.hover`) drive the monitor's alt render.
    pub hover_preview: bool,
    /// Canvas centre / edge / third / other-clip snapping while dragging a clip on the preview.
    pub canvas_snap: bool,
    // ---- ws:export-deliver ----
    /// The platform tiles in the Export window (a Vec, not a const table, so they're editable).
    /// Per-field default fn on top of the container-level one: an old settings.json without this key
    /// backfills the 4 shipped tiles, never `[]`.
    #[serde(default = "crate::engine::export::default_export_presets")]
    pub export_presets: Vec<crate::engine::export::ExportPreset>,
    /// What the last export used — Quick Export (Ctrl+M) re-runs it without opening the window.
    pub last_export: Option<ExportPresetRef>,
    /// Loudness-normalise exports to −14 LUFS (`export::LOUDNORM`). Default on, including for a
    /// settings.json upgrading from before this key existed (`default_loudnorm`, not bool's false).
    #[serde(default = "default_loudnorm")]
    pub loudnorm: bool,
    // ---- ws:inspector-gallery ----
    // ---- ws:layout-modes-onboarding ----
    /// Active workspace name (`ui::layout::WORKSPACES`), lit in the menu-bar strip / View menu.
    pub workspace: String,
    /// Show the Open / Import / Templates / Recent cards over an empty project (the home screen).
    pub home_screen: bool,
    // ---- ws:media-library ----
    /// Extra cells a Library list row shows after the name, in order — any of
    /// `library::COLUMNS` ("kind" | "duration" | "fps" | "size" | "label" | "tags" | "proxy").
    pub library_columns: Vec<String>,
    // ---- ws:source-monitor ----
    // ---- ws:timeline-trim-gestures ----
    // ---- ws:transcript-captions ----
    // ---- ws:pro-monitor ----
    // ---- ws:pro-timeline ----
    // ---- ws:text-titles ----
    // ---- ws:docs-refresh ----
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            ffmpeg_dir: String::new(),
            ytdlp_dir: String::new(),
            download_dir: String::new(),
            encoder: "auto".into(),
            crf: 18,
            preset: "veryfast".into(),
            confirm_overwrite: true,
            lossless_save: false,
            context_menu: true,
            theme: "system".into(),
            decoder: "auto".into(),
            preview_max_width: 1280,
            snap: true,
            show_library: true,
            show_inspector: true,
            hotkeys: BTreeMap::new(),
            recent_assets: Vec::new(),
            recent_projects: Vec::new(),
            layout: String::new(),
            layout_profiles: Vec::new(),
            curve_presets: Vec::new(),
            motion_presets: Vec::new(),
            effect_presets: Vec::new(),
            templates: Vec::new(),
            text_presets: Vec::new(),
            user_fonts: Vec::new(),
            export_scaler: "lanczos".into(),
            export_resolution: "project".into(),
            mcp_enabled: false,
            mcp_port: 7337,
            gpu: true,
            preview_quality: 100,
            movie_mode: false,
            use_proxies: true,
            proxy_height: 720,
            cache_mb: 0,
            icon_overrides: BTreeMap::new(),
            capture_fps: 30,
            capture_bitrate_kbps: 8000,
            capture_mic: String::new(),
            capture_desktop_audio: true,
            capture_cursor: true,
            capture_on_blur: false,
            capture_dir: String::new(),
            voice_device: String::new(),
            voice_channels: 1,
            frame_resolution: "project".into(),
            frame_format: "png".into(),
            frame_quality: 92,
            effect_thumb_image: String::new(),
            palette: PaletteOverride::default(),
            whisper_dir: String::new(),
            transcribe_model: String::new(),
            audio_visualizer: true,
            guide: None,
            project_templates: Vec::new(),
            ui_look: "cozy".into(),
            bg_image: String::new(),
            bg_tint: [0, 0, 0, 120],
            bg_blur: 0,
            panel_opacity: 255,
            // ---- ws:registries-schema-hooks ----
            layout_mode: "dynamic".into(),
            onboarded: false,
            autosave_secs: 30,
            // ---- ws:size-diet ----
            window_rect: None,
            last_seen_version: String::new(),
            // ---- ws:split-god-files ----
            // ---- ws:audio-analysis ----
            duck_depth_db: -12.0,
            duck_ramp_ms: 200,
            beat_thr: 1.6,
            // ---- ws:audio-dsp-automation ----
            // ---- ws:color-engine ----
            lut_dirs: Vec::new(),
            // ---- ws:command-palette ----
            ui_scale: 1.0,
            keymap_preset: "Simple Editor".into(),
            palette_recent: Vec::new(),
            // ---- ws:forgiveness ----
            lock_warn: true,
            // ---- ws:player-rate-loop ----
            audio_scrub: true,
            preroll_secs: 2.0,
            // ---- ws:snap-engine ----
            snap_markers: true,
            // ---- ws:trim-model ----
            // ---- ws:canvas-handles-monitor ----
            hover_preview: true,
            canvas_snap: true,
            // ---- ws:export-deliver ----
            export_presets: crate::engine::export::default_export_presets(),
            last_export: None,
            loudnorm: default_loudnorm(),
            // ---- ws:inspector-gallery ----
            // ---- ws:layout-modes-onboarding ----
            workspace: "Edit".into(),
            home_screen: true,
            // ---- ws:media-library ----
            // "tags" too, so a fresh install's rows look exactly as they did before columns existed
            library_columns: vec!["kind".into(), "duration".into(), "tags".into()],
            // ---- ws:source-monitor ----
            // ---- ws:timeline-trim-gestures ----
            // ---- ws:transcript-captions ----
            // ---- ws:pro-monitor ----
            // ---- ws:pro-timeline ----
            // ---- ws:text-titles ----
            // ---- ws:docs-refresh ----
        }
    }
}

impl Settings {
    /// %APPDATA%\SimpleEditor
    pub fn dir() -> PathBuf {
        let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        base.join("SimpleEditor")
    }
    /// %LOCALAPPDATA%\SimpleEditor\cache (waveform peaks etc.)
    pub fn cache_dir() -> PathBuf {
        let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(Self::dir);
        base.join("SimpleEditor").join("cache")
    }
    /// ---- ws:forgiveness ----
    /// %LOCALAPPDATA%\SimpleEditor\autosave (rolling per-project backups; see `ui::app::autosave`).
    /// Deliberately under %LOCALAPPDATA%, not the roaming %APPDATA% `dir()` — these can be multi-MB.
    pub fn autosave_dir() -> PathBuf {
        let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(Self::dir);
        base.join("SimpleEditor").join("autosave")
    }
    pub fn path() -> PathBuf {
        Self::dir().join("settings.json")
    }
    /// Parses `settings.json`; on a parse failure, best-effort renames it to `settings.json.bad` (so
    /// the corrupt file isn't silently overwritten by the next save) and returns
    /// `(Self::default(), Some(reason))`. A MISSING file (first run) is not an error: `(default, None)`.
    fn load_inner() -> (Self, Option<String>) {
        Self::load_from(&Self::path())
    }
    /// The actual quarantine logic, over an explicit path — split out so a test can point it at a temp
    /// file instead of the real %APPDATA%\SimpleEditor\settings.json.
    fn load_from(path: &std::path::Path) -> (Self, Option<String>) {
        let Ok(text) = std::fs::read_to_string(path) else {
            return (Self::default(), None); // no file yet — first run, not corruption
        };
        match serde_json::from_str(&text) {
            Ok(s) => (s, None),
            Err(e) => {
                let bad = path.with_extension("json.bad");
                let _ = std::fs::rename(path, &bad);
                (Self::default(), Some(e.to_string()))
            }
        }
    }
    /// Unchanged signature (`src/engine/transcribe.rs:46` is a real second caller) — now quarantines a
    /// corrupt file as a side effect of factoring `load_inner` out, for free.
    pub fn load() -> Self {
        Self::load_inner().0
    }
    /// Only new public surface: used solely by `App::new` so it can toast the quarantine reason.
    pub fn load_reporting() -> (Self, Option<String>) {
        Self::load_inner()
    }
    /// Writes a temp file then renames, so a failed write can't destroy the previous settings.
    pub fn save(&self) {
        let _ = std::fs::create_dir_all(Self::dir());
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let tmp = Self::dir().join("settings.json.tmp");
            if std::fs::write(&tmp, s).is_ok() {
                let _ = std::fs::rename(tmp, Self::path());
            }
        }
    }
    pub fn now() -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
    }
    /// Record a media file as recently used (most recent first; pinned entries first; capped).
    /// Keeps the entry's tags/label/pin if it already existed.
    pub fn touch_recent(&mut self, path: &str) {
        let existing = self.recent_assets.iter().position(|r| r.path.eq_ignore_ascii_case(path));
        let mut entry = existing.map(|i| self.recent_assets.remove(i)).unwrap_or_default();
        entry.path = path.to_string();
        entry.last_used = Self::now();
        self.recent_assets.insert(0, entry);
        self.sort_recent();
        // evict oldest unpinned beyond the cap
        let mut n = self.recent_assets.len();
        while n > 200 {
            if let Some(i) = self.recent_assets.iter().rposition(|r| !r.pinned) {
                self.recent_assets.remove(i);
                n -= 1;
            } else {
                break;
            }
        }
    }
    pub fn remove_recent(&mut self, path: &str) {
        self.recent_assets.retain(|r| !r.path.eq_ignore_ascii_case(path));
    }
    /// Pinned first, then most recently used.
    pub fn sort_recent(&mut self) {
        self.recent_assets.sort_by(|a, b| b.pinned.cmp(&a.pinned).then(b.last_used.cmp(&a.last_used)));
    }
    pub fn touch_recent_project(&mut self, path: &str) {
        self.recent_projects.retain(|r| !r.eq_ignore_ascii_case(path));
        self.recent_projects.insert(0, path.to_string());
        self.recent_projects.truncate(20);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recent_keeps_tags_and_pins() {
        let mut s = Settings::default();
        s.touch_recent("a.mp4");
        s.recent_assets[0].tags.push("x".into());
        s.recent_assets[0].pinned = true;
        s.touch_recent("b.mp4");
        s.touch_recent("A.MP4"); // same file, case-insensitive
        assert_eq!(s.recent_assets.len(), 2);
        assert_eq!(s.recent_assets[0].path, "A.MP4");
        assert!(s.recent_assets[0].pinned && s.recent_assets[0].tags == vec!["x"]);
        s.remove_recent("b.mp4");
        assert_eq!(s.recent_assets.len(), 1);
    }

    #[test]
    fn effect_thumb_image_round_trips() {
        assert!(Settings::default().effect_thumb_image.is_empty(), "empty = the embedded default");
        let mut s = Settings::default();
        s.effect_thumb_image = r"C:\pics\stock.png".into();
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.effect_thumb_image, s.effect_thumb_image);
        // an old settings file without the field still loads, back to the default
        let old: Settings = serde_json::from_str("{}").unwrap();
        assert!(old.effect_thumb_image.is_empty());
    }

    // ---- ws:forgiveness ----
    #[test]
    fn corrupt_settings_are_quarantined_not_overwritten() {
        let path = std::env::temp_dir().join(format!("se-settings-test-{}.json", std::process::id()));
        let bad = path.with_extension("json.bad");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&bad);

        let default_json = serde_json::to_string(&Settings::default()).unwrap();
        std::fs::write(&path, "{ not valid json").unwrap();
        let (s, reason) = Settings::load_from(&path);
        assert_eq!(serde_json::to_string(&s).unwrap(), default_json);
        assert!(reason.is_some(), "a parse failure must report a reason");
        assert!(!path.exists(), "the corrupt file must be moved out of the way");
        assert!(bad.exists(), "…to settings.json.bad");

        // a second load (nothing left at `path`) is just a fresh-install default — no re-corruption
        let (s2, reason2) = Settings::load_from(&path);
        assert_eq!(serde_json::to_string(&s2).unwrap(), default_json);
        assert!(reason2.is_none());

        let _ = std::fs::remove_file(&bad);
    }

    #[test]
    fn palette_override_round_trips() {
        assert_eq!(Settings::default().palette, PaletteOverride::default(), "unset = today's behaviour");
        let mut s = Settings::default();
        s.palette.mode = "custom".into();
        s.palette.accent = Some([200, 30, 40]);
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.palette, s.palette);
        // an old settings file without the field still loads, back to the default (system, no overrides)
        let old: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(old.palette, PaletteOverride::default());
    }

    #[test]
    fn look_and_background_round_trip() {
        let old: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(old.ui_look, "cozy");
        assert!(old.bg_image.is_empty());
        assert_eq!(old.panel_opacity, 255);
        let mut s = Settings::default();
        s.ui_look = "sharp".into();
        s.bg_image = r"C:\pics\bg.jpg".into();
        s.bg_tint = [10, 20, 30, 90];
        s.bg_blur = 8;
        s.panel_opacity = 180;
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(
            (back.ui_look, back.bg_image, back.bg_tint, back.bg_blur, back.panel_opacity),
            (s.ui_look, s.bg_image, s.bg_tint, s.bg_blur, s.panel_opacity)
        );
    }

    #[test]
    fn window_rect_and_version_round_trip() {
        let old: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(old.window_rect, None, "no persistence feature yet = never moved");
        assert!(old.last_seen_version.is_empty(), "empty = always show What's New once on first launch");
        let mut s = Settings::default();
        s.window_rect = Some([10, 20, 1400, 860]);
        s.last_seen_version = "0.2.0".into();
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.window_rect, s.window_rect);
        assert_eq!(back.last_seen_version, s.last_seen_version);
    }

    #[test]
    fn audio_scrub_and_preroll_round_trip() {
        let old: Settings = serde_json::from_str("{}").unwrap();
        assert!(old.audio_scrub, "default on");
        assert_eq!(old.preroll_secs, 2.0);
        let mut s = Settings::default();
        s.audio_scrub = false;
        s.preroll_secs = 0.5;
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.audio_scrub, s.audio_scrub);
        assert_eq!(back.preroll_secs, s.preroll_secs);
    }

    // ---- ws:canvas-handles-monitor ----
    #[test]
    fn hover_preview_and_canvas_snap_round_trip() {
        let old: Settings = serde_json::from_str("{}").unwrap();
        assert!(old.hover_preview && old.canvas_snap, "both default on");
        let mut s = Settings::default();
        s.hover_preview = false;
        s.canvas_snap = false;
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert!(!back.hover_preview && !back.canvas_snap);
    }

    #[test]
    fn theme_file_round_trips() {
        let tf = ThemeFile {
            name: "Mine".into(),
            theme: "dark".into(),
            ui_look: "cozy".into(),
            palette: crate::theme::preset("Dracula").unwrap(),
            bg_image: r"C:\pics\bg.jpg".into(),
            bg_tint: [10, 20, 30, 90],
            bg_blur: 6,
            panel_opacity: 200,
        };
        let back: ThemeFile = serde_json::from_str(&serde_json::to_string(&tf).unwrap()).unwrap();
        assert_eq!(back, tf);
        // an old theme file without the background fields still loads with sane defaults
        let old: ThemeFile = serde_json::from_str("{}").unwrap();
        assert_eq!(old.panel_opacity, 255);
    }

    // ---- ws:command-palette ----
    #[test]
    fn command_palette_settings_round_trip() {
        let old: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(old.ui_scale, 1.0);
        assert_eq!(old.keymap_preset, "Simple Editor");
        assert!(old.palette_recent.is_empty());
        let mut s = Settings::default();
        s.ui_scale = 1.25;
        s.keymap_preset = "Premiere".into();
        s.palette_recent = vec!["command_palette".into(), "pane.Library".into()];
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.ui_scale, s.ui_scale);
        assert_eq!(back.keymap_preset, s.keymap_preset);
        assert_eq!(back.palette_recent, s.palette_recent);
    }

    // ---- ws:export-deliver ----
    /// A settings.json from before this workstream (no `export_presets` / `loudnorm` / `last_export`
    /// keys) backfills the 4 tiles and loudnorm=true — never `[]` / false.
    #[test]
    fn settings_backfill_on_upgrade() {
        let old: Settings = serde_json::from_str(r#"{"crf": 20, "theme": "dark"}"#).unwrap();
        assert_eq!(old.export_presets, crate::engine::export::default_export_presets());
        assert_eq!(old.export_presets.len(), 4);
        assert!(old.loudnorm);
        assert_eq!(old.last_export, None);
        assert_eq!(old.crf, 20, "the keys that WERE there still load");
        // an explicit choice round-trips (a user who turned it off, or edited the tiles, keeps that)
        let mut s = Settings::default();
        s.loudnorm = false;
        s.export_presets.truncate(1);
        s.last_export = Some(ExportPresetRef::Custom { ext: "webm".into(), width: 640, height: 360 });
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert!(!back.loudnorm);
        assert_eq!(back.export_presets.len(), 1);
        assert_eq!(back.last_export, s.last_export);
    }

    // ---- ws:layout-modes-onboarding ----
    #[test]
    fn layout_mode_settings_round_trip() {
        let old: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(old.workspace, "Edit", "today's default layout is the Edit workspace");
        assert!(old.home_screen, "default on");
        assert_eq!(old.layout_mode, "dynamic");
        assert!(!old.onboarded, "a settings file without the flag sees the welcome once");
        let mut s = Settings::default();
        s.workspace = "Color".into();
        s.home_screen = false;
        s.layout_mode = "granular".into();
        s.onboarded = true;
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!((back.workspace.as_str(), back.home_screen, back.layout_mode.as_str(), back.onboarded), ("Color", false, "granular", true));
    }
}
