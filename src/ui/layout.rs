//! Dockable editor layout (egui_tiles): panes can be rearranged by dragging their tabs, split, tabbed,
//! hidden/shown, popped out into their own OS windows (egui immediate viewports) and saved/loaded as
//! profiles (JSON — also written to / read from `.sedit-layout` files so profiles can be shared).
//!
//! Default layout (DaVinci-like): top row = [Library | Preview | tabs(Inspector, Effects, Transitions,
//! Subtitles)], bottom = tabs(Timeline, Curves). `show` draws the tree in the given `ui` and each popped
//! pane in its own viewport (closing that window docks the pane back); `draw(ui, pane)` renders a pane.
//! Behaviour: tab titles = Pane::title, a "⧉" button in the tab bar pops the pane out, "✕" hides it.

use eframe::egui;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum Pane {
    Preview,
    Timeline,
    Library,
    Inspector,
    Effects,
    Transitions,
    Curves,
    Subtitles,
    Planner,
    AutoCut,
}

impl Pane {
    pub const ALL: [Pane; 10] = [
        Pane::Preview,
        Pane::Timeline,
        Pane::Library,
        Pane::Inspector,
        Pane::Effects,
        Pane::Transitions,
        Pane::Curves,
        Pane::Subtitles,
        Pane::Planner,
        Pane::AutoCut,
    ];
    pub fn title(self) -> &'static str {
        match self {
            Pane::Preview => "Preview",
            Pane::Timeline => "Timeline",
            Pane::Library => "Library",
            Pane::Inspector => "Inspector",
            Pane::Effects => "Effects",
            Pane::Transitions => "Transitions",
            Pane::Curves => "Curves",
            Pane::Subtitles => "Subtitles",
            Pane::Planner => "Planner",
            Pane::AutoCut => "Auto-cut",
        }
    }
}

/// The whole layout: the docked tree plus panes currently popped out into their own windows.
#[derive(Clone, Serialize, Deserialize)]
pub struct Layout {
    pub tree: egui_tiles::Tree<Pane>,
    #[serde(default)]
    pub popped: Vec<Pane>,
}

impl Default for Layout {
    fn default() -> Self {
        Self::default_layout()
    }
}

impl Layout {
    pub fn default_layout() -> Self {
        todo!("ui::layout::Layout::default_layout")
    }
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
    /// None on malformed / incompatible JSON (caller falls back to the default layout).
    pub fn from_json(s: &str) -> Option<Self> {
        serde_json::from_str(s).ok()
    }
    /// Visible = docked in the tree or popped out.
    pub fn is_visible(&self, _pane: Pane) -> bool {
        todo!("ui::layout::Layout::is_visible")
    }
    /// Show (dock into a sensible place — next to a related pane, else a new tab in the largest
    /// container) or hide (remove from the tree / close the popout).
    pub fn toggle(&mut self, _pane: Pane) {
        todo!("ui::layout::Layout::toggle")
    }
    pub fn popout(&mut self, _pane: Pane) {
        todo!("ui::layout::Layout::popout")
    }
    pub fn dock(&mut self, _pane: Pane) {
        todo!("ui::layout::Layout::dock")
    }
}

/// Draw the docked tree into `ui` and every popped pane in its own OS window; `draw(ui, pane)` renders
/// a pane's content. A popped window that the user closes is docked back automatically.
pub fn show(_ctx: &egui::Context, _ui: &mut egui::Ui, _layout: &mut Layout, _draw: &mut dyn FnMut(&mut egui::Ui, Pane)) {
    todo!("ui::layout::show")
}
