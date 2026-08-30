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
    /// UI look: "cozy" (rounded corners, soft button borders) or "sharp" (the old flat look).
    pub ui_look: String,
    /// Editor background image (empty = none), its tint (RGBA over the image) and load-time blur radius.
    pub bg_image: String,
    pub bg_tint: [u8; 4],
    pub bg_blur: u8,
    /// Panel/tab background opacity over the background image (255 = opaque, ignored without an image).
    pub panel_opacity: u8,
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
            ui_look: "cozy".into(),
            bg_image: String::new(),
            bg_tint: [0, 0, 0, 120],
            bg_blur: 0,
            panel_opacity: 255,
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
    pub fn path() -> PathBuf {
        Self::dir().join("settings.json")
    }
    pub fn load() -> Self {
        std::fs::read_to_string(Self::path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
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
        assert_eq!((back.ui_look, back.bg_image, back.bg_tint, back.bg_blur, back.panel_opacity),
                   (s.ui_look, s.bg_image, s.bg_tint, s.bg_blur, s.panel_opacity));
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
}
