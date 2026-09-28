use crate::model::*;
use serde::{Deserialize, Serialize};

// ---------- markers ----------

/// A note on the timeline (or, in `Clip.markers`, on a clip). The AI tools read these too.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Marker {
    pub id: Id,
    /// Timeline seconds (project markers) or clip-local seconds (clip markers).
    pub t: f64,
    /// 0 = a point marker; > 0 = a range.
    pub duration: f64,
    pub name: String,
    pub note: String,
    /// Index into `Project.labels` + 1 (0 = none).
    pub label: u8,
    /// Glyph name (`ui::tools::Glyph::name()` / `from_name`) - same string convention as
    /// `Settings.icon_overrides`. Missing on old projects: the container-level `#[serde(default)]`
    /// above pulls it (and `sequence`) from `Marker::default()` below.
    pub icon: String,
    /// Which sequence this marker was created on (`None` = the main timeline). Project-level
    /// markers only, for scoping `Project.markers` per sequence - clip markers already scope
    /// through their clip and ignore this field.
    pub sequence: Option<Id>,
}

impl Default for Marker {
    fn default() -> Self {
        Self {
            id: 0,
            t: 0.0,
            duration: 0.0,
            name: String::new(),
            note: String::new(),
            label: 0,
            icon: "flag".to_string(),
            sequence: None,
        }
    }
}

// ---------- labels ----------

/// A user-editable colour label. `Project.labels` starts as `default_labels()`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Label {
    pub name: String,
    pub color: [u8; 3],
}

pub fn default_labels() -> Vec<Label> {
    LABEL_COLORS.iter().map(|(n, c)| Label { name: (*n).to_string(), color: *c }).collect()
}
