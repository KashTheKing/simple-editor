//! ---- ws:addon-bg-removal ----
//! A clip's background removal (no green screen): which model made its matte and how the matte is shaped
//! when drawn. The mattes themselves are a cache on disk (`engine::bgremove`), never in the project.
use serde::{Deserialize, Serialize};

/// Which model computes the matte.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BgQuality {
    /// MODNet: fast, made for people (portrait matting).
    #[default]
    Fast,
    /// BiRefNet: slower, any subject, finer edges.
    Best,
}

impl BgQuality {
    pub fn id(self) -> &'static str {
        match self {
            BgQuality::Fast => "fast",
            BgQuality::Best => "best",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            BgQuality::Fast => "Fast",
            BgQuality::Best => "Best",
        }
    }
    pub fn parse(s: &str) -> Option<BgQuality> {
        match s.trim().to_ascii_lowercase().as_str() {
            "fast" | "modnet" => Some(BgQuality::Fast),
            "best" | "birefnet" => Some(BgQuality::Best),
            _ => None,
        }
    }
}

/// Present on a clip whose background is removed. Until its matte frames are computed the clip draws
/// unkeyed (playback only ever reads the cache).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BgRemoval {
    #[serde(default)]
    pub quality: BgQuality,
    /// -1 (shrink the matte, harder edge) .. 0 (as computed) .. 1 (feather, softer edge).
    #[serde(default)]
    pub edge: f32,
    /// Draw the matte itself (white = kept) instead of the keyed picture.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub show_matte: bool,
}
