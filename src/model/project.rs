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
    /// (e.g. "…" / " —"). Applied by "Regenerate cues", not retroactively.
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
    /// Standalone moodboard (`ui::moodboard_ui`) — separate from the per-task moodboards in `plan`.
    pub moodboard: Vec<MoodItem>,
    /// Saved drawing / polygon outlines, reusable as motion paths (see `PathAsset`).
    pub paths: Vec<PathAsset>,
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
}

impl Default for Project {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-clip word timing (timeline seconds) captured by a transcription pass — `words` is
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
