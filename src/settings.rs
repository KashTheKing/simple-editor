//! App settings — one JSON file at %APPDATA%\SimpleEditor\settings.json.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RecentAsset {
    pub path: String,
    /// Unix seconds.
    pub last_used: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Directory containing ffmpeg.exe / ffprobe.exe. Empty = app dir, then PATH.
    pub ffmpeg_dir: String,
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
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            ffmpeg_dir: String::new(),
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
    /// Record a media file as recently used (most recent first, capped).
    pub fn touch_recent(&mut self, path: &str) {
        self.recent_assets.retain(|r| !r.path.eq_ignore_ascii_case(path));
        self.recent_assets.insert(0, RecentAsset { path: path.to_string(), last_used: Self::now() });
        self.recent_assets.truncate(200);
    }
    pub fn touch_recent_project(&mut self, path: &str) {
        self.recent_projects.retain(|r| !r.eq_ignore_ascii_case(path));
        self.recent_projects.insert(0, path.to_string());
        self.recent_projects.truncate(20);
    }
}
