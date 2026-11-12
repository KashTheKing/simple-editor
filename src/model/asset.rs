use crate::model::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
    // ---- ws:html-layer ----
    /// An HTML layer / scene: its source and field values live here (the project is self-contained);
    /// `path` is only the rendered cache file (`engine::html`), re-made whenever it is missing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub html: Option<HtmlSource>,
}

// ---- ws:html-layer ----
/// The template behind an HTML asset (`library/README.md`, "HTML layers and scenes").
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct HtmlSource {
    /// Display name (the template's `<title>` unless the user named it).
    pub name: String,
    /// The whole HTML file, CSS included.
    pub src: String,
    /// Values for its `se:field`s; a field not listed here uses its default.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<String, String>,
    /// Seconds rendered.
    pub duration: f64,
    /// Values (constant or keyframed over the layer's own time, in seconds) of the page's `se:param`s:
    /// each is the CSS variable `--name` while the page is rendered. A param not listed is its default.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, Animated>,
    /// The width in CSS pixels a web page is laid out at (a page without `se:size`); 0 = the template's.
    /// A saved page has no say in this: a phone-width layout and a desktop one are different pages.
    #[serde(skip_serializing_if = "is_zero")]
    pub width: u32,
    /// A fake mouse over the page: where it is, and whether its button is down, over the layer's own
    /// time. The renderer really moves and presses a mouse in the page, so `:hover`, `:active` and the
    /// transitions they start run as they would in a browser (and a cursor is drawn). None = no mouse.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mouse: Option<Mouse>,
    /// A `.html` file kept in sync with `src` (edit it in any code editor, or the clip, and the other
    /// follows); empty = not linked. The project still stores `src`, so it opens without the file.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub file: String,
}

/// The tracks of a fake mouse: position in CSS pixels of the page's viewport, and the button
/// (a value of 0.5 or more is "pressed").
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Mouse {
    pub x: Animated,
    pub y: Animated,
    pub down: Animated,
}

impl Mouse {
    /// A mouse resting at (x, y), button up.
    pub fn at(x: f64, y: f64) -> Mouse {
        Mouse { x: Animated::new(x), y: Animated::new(y), down: Animated::new(0.0) }
    }
    /// (x, y, pressed) at the layer's second `t`.
    pub fn state(&self, t: f64) -> (f64, f64, bool) {
        (self.x.at(t), self.y.at(t), self.down.at(t) >= 0.5)
    }
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

#[cfg(test)]
mod html_tests {
    use super::*;

    #[test]
    fn a_linked_file_is_saved_and_an_unlinked_one_leaves_no_trace() {
        let mut h = HtmlSource { name: "x".into(), src: "<p>".into(), duration: 4.0, ..Default::default() };
        assert!(!serde_json::to_string(&h).unwrap().contains("file"), "unlinked: no field in the project file");
        h.file = "C:/x/card.html".into();
        let back: HtmlSource = serde_json::from_str(&serde_json::to_string(&h).unwrap()).unwrap();
        assert_eq!(back.file, "C:/x/card.html");
        let old: HtmlSource = serde_json::from_str(r#"{"name":"x","src":"<p>","duration":4.0}"#).unwrap();
        assert!(old.file.is_empty(), "a project from before linking opens unlinked");
    }
}

impl HtmlSource {
    pub fn field_list(&self) -> Vec<(String, String)> {
        self.fields.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }
}

impl Asset {
    pub fn name(&self) -> String {
        if let Some(h) = self.html.as_ref().filter(|h| !h.name.is_empty()) {
            return h.name.clone(); // ws:html-layer: the path is a cache hash
        }
        Path::new(&self.path).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| self.path.clone())
    }
    pub fn has_video(&self) -> bool {
        matches!(self.kind, ClipKind::Video | ClipKind::Image)
    }
}
