use crate::model::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct AudioStreamInfo {
    /// 0-based index among the file's audio streams (ffmpeg `0:a:N`).
    pub index: usize,
    pub channels: u32,
    pub sample_rate: u32,
    pub language: String,
    pub title: String,
    pub codec: String,
}

impl AudioStreamInfo {
    pub fn label(&self) -> String {
        if !self.title.is_empty() {
            self.title.clone()
        } else if !self.language.is_empty() && self.language != "und" {
            self.language.clone()
        } else {
            format!("Audio {}", self.index + 1)
        }
    }
}

/// Colour labels for assets (0 = none).
pub const LABEL_COLORS: [(&str, [u8; 3]); 8] = [
    ("Red", [220, 70, 70]),
    ("Orange", [230, 140, 50]),
    ("Yellow", [220, 200, 60]),
    ("Green", [80, 180, 90]),
    ("Teal", [60, 180, 180]),
    ("Blue", [70, 120, 220]),
    ("Purple", [150, 90, 200]),
    ("Gray", [150, 150, 150]),
];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Asset {
    pub id: Id,
    pub path: String,
    /// Video, Image or Audio (never Text).
    pub kind: ClipKind,
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    #[serde(default)]
    pub audio_streams: Vec<AudioStreamInfo>,
    #[serde(default)]
    pub codec: String,
    /// Library folder ("" = root, "Footage/Day 1" = nested).
    #[serde(default)]
    pub folder: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// 0 = none, 1..=8 = index+1 into LABEL_COLORS.
    #[serde(default)]
    pub label: u8,
    /// Free-form notes: what this asset is / what it's for (library + inspector).
    #[serde(default)]
    pub description: String,
    // ---- ws:registries-schema-hooks ----
    /// Path relative to the project file, when it was saved with one - kept alongside `path` (the
    /// absolute/last-known path) so a moved project directory can be relinked without touching every
    /// clip. Unused this wave (schema only).
    #[serde(default)]
    pub rel_path: Option<String>,
    /// Set for a subclip: the library asset it was cut from (`Project::add_subclip`).
    #[serde(default)]
    pub parent: Option<Id>,
    /// Set for a subclip: the (in, out) source-time range of `parent` it covers.
    #[serde(default)]
    pub range: Option<(f64, f64)>,
    /// Master/source-clip effects (rendered by ws:color-engine, wave 1) - apply to every clip using
    /// this asset, ahead of the clip's own effect stack. Unused this wave (schema only).
    #[serde(default)]
    pub effects: Vec<Effect>,
}

impl Asset {
    pub fn name(&self) -> String {
        Path::new(&self.path).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| self.path.clone())
    }
    pub fn has_video(&self) -> bool {
        matches!(self.kind, ClipKind::Video | ClipKind::Image)
    }
}
