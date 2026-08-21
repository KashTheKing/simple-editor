//! App settings — one JSON file at %APPDATA%\SimpleEditor\settings.json.

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
    /// Current editor layout as JSON (empty = default layout).
    pub layout: String,
    /// Saved layout profiles (also exportable to / importable from `.sedit-layout` files).
    pub layout_profiles: Vec<LayoutProfile>,
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
            layout: String::new(),
            layout_profiles: Vec::new(),
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
}
