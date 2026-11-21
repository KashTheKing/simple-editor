use crate::model::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Project {
    pub version: u32,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub assets: Vec<Asset>,
    /// Library folders that exist even while empty ("A/B" nesting by '/').
    pub folders: Vec<String>,
    /// Folders on disk browsed directly from the library (files are imported on drop).
    pub linked_folders: Vec<String>,
    /// Linked bins: library bins that ARE a folder on disk (see `engine::linked_bins`). Only the folder and
    /// the bin are saved - files are listed when the bin is opened and become assets when they are used.
    pub linked_bins: Vec<LinkedBin>,
    /// Order: all video tracks (V1, V2, ...) then all audio tracks (A1, A2, ...). While `editing` is
    /// Some(seq), these are that sequence's tracks (the main timeline is in `main_stash`).
    pub tracks: Vec<Track>,
    pub in_point: Option<f64>,
    pub out_point: Option<f64>,
    /// Set when the project was created by opening a video file directly; enables "Save" (overwrite).
    pub source_video: Option<String>,
    /// Subtitle cues (kept sorted by start). Rendered bottom-centre by the compositor when `show_subtitles`.
    pub subtitles: Vec<Cue>,
    pub subtitle_style: TextStyle,
    /// Distance from the bottom edge in project pixels.
    pub subtitle_margin: f32,
    pub show_subtitles: bool,
    /// Prepended/appended to a generated cue where a sentence continues across the cue split
    /// (e.g. "…" / " - "). Applied by "Regenerate cues", not retroactively.
    pub subtitle_cont_prefix: String,
    pub subtitle_cont_suffix: String,
    /// Compositor resampling quality.
    pub scaler: Scaler,
    /// Preview/export canvas background (checkerboard / solid / custom colour).
    pub preview_bg: BackgroundMode,
    /// Colour labels (name + colour), editable by the user.
    pub labels: Vec<Label>,
    /// Timeline markers (sorted by time).
    pub markers: Vec<Marker>,
    /// Mixer buses; `buses[0]` is Main and always exists.
    pub buses: Vec<Bus>,
    /// Nested timelines (usable as footage via `ClipKind::Sequence`).
    pub sequences: Vec<Sequence>,
    /// The sequence currently swapped into `tracks` for editing (None = main timeline).
    pub editing: Option<Id>,
    pub main_stash: Option<Stash>,
    /// Planner (nested tasks with moodboards) and free-form notes (process / ideas / style).
    pub plan: Vec<PlanItem>,
    #[serde(default, deserialize_with = "de_notes")]
    pub notes: Vec<Note>,
    /// Standalone moodboard (`ui::moodboard_ui`) - separate from the per-task moodboards in `plan`.
    pub moodboard: Vec<MoodItem>,
    /// Saved drawing / polygon outlines, reusable as motion paths (see `PathAsset`).
    pub paths: Vec<PathAsset>,
    /// ws:animator: transition spans over the main timeline (`ops::spans`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub spans: Vec<TransitionSpan>,
    pub(crate) next_id: Id,
    // ---- ws:registries-schema-hooks ----
    /// Per-clip word timing (timeline seconds) for the transcript panel / word-range cuts; filled by
    /// ws:transcript-captions (wave 2). Empty until then.
    #[serde(default)]
    pub transcripts: Vec<Transcript>,
    /// How generated captions animate (karaoke-style highlight/pop/typewriter); consumed by
    /// ws:transcript-captions (wave 2).
    #[serde(default)]
    pub subtitle_anim: SubtitleAnim,
    /// Saved library search filters ("Smart Bins"); consumed by ws:media-library (wave 2).
    #[serde(default)]
    pub smart_bins: Vec<SmartBin>,
    /// Library display names a user gave plain (non-subclip) assets; the file on disk keeps its name.
    #[serde(default)]
    pub asset_names: std::collections::HashMap<Id, String>,
    // ---- ws:size-diet ----
    // ---- ws:split-god-files ----
    // ---- ws:audio-analysis ----
    // ---- ws:audio-dsp-automation ----
    // ---- ws:color-engine ----
    // ---- ws:command-palette ----
    // ---- ws:forgiveness ----
    // ---- ws:player-rate-loop ----
    // ---- ws:snap-engine ----
    // ---- ws:trim-model ----
    // ---- ws:canvas-handles-monitor ----
    // ---- ws:export-deliver ----
    // ---- ws:inspector-gallery ----
    // ---- ws:layout-modes-onboarding ----
    // ---- ws:media-library ----
    // ---- ws:source-monitor ----
    // ---- ws:timeline-trim-gestures ----
    // ---- ws:transcript-captions ----
    // ---- ws:pro-monitor ----
    // ---- ws:pro-timeline ----
    // ---- ws:text-titles ----
    // ---- ws:docs-refresh ----
    // ---- ws:quick-export ----
    /// The last (folder, extension) a normal Quick Export saved to; the next dialog starts there.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quick_export: Option<(String, String)>,
    // ---- ws:beat-aligner ----
    /// The Beat Aligner's song, grid, marks and sections (`model::beats`).
    #[serde(skip_serializing_if = "crate::model::beats::is_empty")]
    pub beats: BeatSheet,
    // ---- ws:music-burn ----
    /// Playlists of library songs (`model::ops::playlists`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub playlists: Vec<Playlist>,
    // ---- ws:preview-audio-only ----
    /// Preview ▸ Audio Only: the Preview plays the sound and shows a visualizer of the mix instead of the
    /// picture (no video decoded). Export is unaffected. Kept across undo / redo.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub audio_only_preview: bool,
    // ---- ws:max-duration ----
    /// Max length in seconds of the timeline being edited (None = no limit). Like width/height/fps it is
    /// the open sequence's own while one is open (`open_sequence` swaps it). Nothing is ever cut to it:
    /// the timeline shades past it and exports stop at it (`limit_range`) unless the user includes the rest.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_duration: Option<f64>,
    // ---- ws:project-color-fps ----
    /// The colour space the project renders and exports in (Inspector ▸ Color). SDR in old files.
    #[serde(skip_serializing_if = "ColorSpace::is_sdr")]
    pub color: ColorSpace,
    // ---- feat/styles-context ----
    /// Project Context (`model::context`): the user's Markdown document about the project - About, Rules,
    /// Style guide, Links - which every Assistant request carries. An applied editing style sits inside it.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub context: String,
    // ---- ws:timeline-tabs ----
    /// The timelines open as tabs above the timeline, in tab order (None = the main timeline). Empty in
    /// old files: just Main (plus the open sequence). A view list, kept across undo / redo; read it
    /// through `timeline_tabs()`, which drops deleted sequences and always holds the open one.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub timeline_tabs: Vec<Option<Id>>,
    // ---- ws:zones-markers ----
    /// Named marker types, each with a colour (`Marker.category` holds the name). Missing in files saved
    /// before categories existed: the built-in set (`default_marker_categories`), every marker uncategorised.
    pub marker_categories: Vec<Label>,
    /// Effect zones (`ops::zones`, drawn by `engine::zones::render_view`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub zones: Vec<EffectZone>,
    // ---- feat/reference-clips ----
    /// Where each reference clip came from and what is said in it (`model::RefSource`, Find a Reference).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<RefSource>,
    // ---- ws:parity-io ----
    /// Per-source decode overrides (rotation, YUV range), by source path: Media Properties /
    /// `media.set_io`. The decoders follow it through `media::io_override::sync`.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub media_io: std::collections::BTreeMap<String, crate::media::io_override::MediaIo>,
    // ---- ws:visualizer-helper ----
    /// Visualizer Helper's song analyses (hits, drops, vocal envelope, ...), one per analysed song asset,
    /// plus the clips each step made from it (`model::vizhelp`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub viz: Vec<SongViz>,
}

// ---- ws:project-color-fps ----
/// A project's render/export colour space. The compositor works on SDR-referred pixels either way:
/// HDR sources are tone-mapped on decode, and an HDR export maps SDR white to 203 nits (BT.2408) in a
/// 10-bit BT.2020 PQ / HLG file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorSpace {
    #[default]
    Sdr,
    Hdr10,
    Hlg,
}

impl ColorSpace {
    pub const ALL: [ColorSpace; 3] = [ColorSpace::Sdr, ColorSpace::Hdr10, ColorSpace::Hlg];
    pub fn is_sdr(&self) -> bool {
        *self == ColorSpace::Sdr
    }
    pub fn is_hdr(self) -> bool {
        !self.is_sdr()
    }
    pub fn name(self) -> &'static str {
        match self {
            ColorSpace::Sdr => "SDR Rec.709",
            ColorSpace::Hdr10 => "HDR10 (Rec.2100 PQ)",
            ColorSpace::Hlg => "HDR HLG",
        }
    }
    /// Short form for summaries: "SDR" / "HDR10" / "HLG".
    pub fn short(self) -> &'static str {
        match self {
            ColorSpace::Sdr => "SDR",
            ColorSpace::Hdr10 => "HDR10",
            ColorSpace::Hlg => "HLG",
        }
    }
    /// From a probed `color_transfer` (ffprobe names): PQ / HLG, else SDR.
    pub fn of_transfer(trc: &str) -> Self {
        match trc {
            "smpte2084" => ColorSpace::Hdr10,
            "arib-std-b67" => ColorSpace::Hlg,
            _ => ColorSpace::Sdr,
        }
    }
    /// From the MCP / Luau string: "sdr" | "hdr10" | "pq" | "hlg".
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "sdr" | "rec709" | "bt709" => Some(ColorSpace::Sdr),
            "hdr10" | "hdr" | "pq" => Some(ColorSpace::Hdr10),
            "hlg" => Some(ColorSpace::Hlg),
            _ => None,
        }
    }
}

/// Frame rates a project can pick from a list (Inspector, the match prompt's snap targets).
pub const STANDARD_FPS: &[f64] = &[23.976, 24.0, 25.0, 29.97, 30.0, 48.0, 50.0, 59.94, 60.0, 90.0, 120.0, 144.0, 240.0];

/// A typed / stored rate made clean: at most 3 decimals, within 1..=240 (NaN -> 30).
pub fn clean_fps(f: f64) -> f64 {
    if !f.is_finite() {
        return 30.0;
    }
    ((f.clamp(1.0, 240.0)) * 1000.0).round() / 1000.0
}

/// A MEASURED source rate (variable-frame-rate phone and screen recordings report 26.0221...): the
/// nearest standard rate when within 3 %, else `clean_fps`. 29.97002997 -> 29.97, 59.94 stays.
pub fn snap_fps(f: f64) -> f64 {
    let f = clean_fps(f);
    STANDARD_FPS
        .iter()
        .copied()
        .min_by(|a, b| (a - f).abs().total_cmp(&(b - f).abs()))
        .filter(|s| (s - f).abs() <= s * 0.03)
        .unwrap_or(f)
}

/// What a loaded file's rate becomes: a clean value (<= 3 decimals) is kept as typed; a raw measured
/// float (an older build copied the source's rate as-is) is snapped.
pub fn normalize_fps(f: f64) -> f64 {
    if f.is_finite() && (f * 1000.0 - (f * 1000.0).round()).abs() < 1e-6 && (1.0..=240.0).contains(&f) {
        f
    } else {
        snap_fps(f)
    }
}

impl Default for Project {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-clip word timing (timeline seconds) captured by a transcription pass - `words` is
/// `(start, end, text)` triples in timeline time, so a cut range maps straight onto clip trims.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Transcript {
    pub clip: Id,
    #[serde(default)]
    pub words: Vec<(f64, f64, String)>,
}

/// How a generated caption's current word is drawn as the playhead crosses it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubtitleAnim {
    #[default]
    None,
    Highlight([u8; 4]),
    PopWord,
    Typewriter,
}

/// A saved library search ("Smart Bin"): a name and the query text that reproduces it.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SmartBin {
    pub name: String,
    pub query: String,
}

/// A library bin that points at a folder on disk. `bin` is a top-level entry in `Project::folders`; its
/// sub-folders on disk show as nested bins ("Video Assets/SFX") without being stored anywhere.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LinkedBin {
    pub bin: String,
    pub path: String,
}
