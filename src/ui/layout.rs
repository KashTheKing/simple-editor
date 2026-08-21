//! Dockable editor layout (egui_tiles): panes can be rearranged by dragging their tabs, split, tabbed,
//! hidden/shown, popped out into their own OS windows (egui immediate viewports) and saved/loaded as
//! profiles (JSON — also written to / read from `.sedit-layout` files so profiles can be shared).
//!
//! Default layout (DaVinci-like): top row = [Library | Preview | tabs(Inspector, Effects, Transitions,
//! Subtitles, Planner)], bottom row (full width) = tabs(Timeline, Curves, Auto-cut). `show` draws the
//! tree in the given `ui` and each popped pane in its own viewport (closing that window docks the pane
//! back); `draw(ui, pane)` renders a pane. Behaviour: tab titles = Pane::title, a "⧉" button in the tab
//! bar pops the active pane out, "✕" hides it. Hidden panes stay in the tree (egui_tiles visibility) so
//! they come back exactly where they were.

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
        use egui_tiles::{Container, Linear, LinearDir, Tile};
        let mut tiles = egui_tiles::Tiles::default();
        let library = tiles.insert_pane(Pane::Library);
        let preview = tiles.insert_pane(Pane::Preview);
        let right_panes: Vec<_> = [Pane::Inspector, Pane::Effects, Pane::Transitions, Pane::Subtitles, Pane::Planner]
            .iter()
            .map(|&p| tiles.insert_pane(p))
            .collect();
        let right = tiles.insert_tab_tile(right_panes);
        let mut top_row = Linear::new(LinearDir::Horizontal, vec![library, preview, right]);
        top_row.shares.set_share(library, 0.2);
        top_row.shares.set_share(preview, 0.6);
        top_row.shares.set_share(right, 0.2);
        let top = tiles.insert_new(Tile::Container(Container::Linear(top_row)));
        let bottom_panes: Vec<_> =
            [Pane::Timeline, Pane::Curves, Pane::AutoCut].iter().map(|&p| tiles.insert_pane(p)).collect();
        let bottom = tiles.insert_tab_tile(bottom_panes); // Timeline is the active tab (first child)
        let mut rows = Linear::new(LinearDir::Vertical, vec![top, bottom]);
        rows.shares.set_share(top, 0.7);
        rows.shares.set_share(bottom, 0.3);
        let root = tiles.insert_new(Tile::Container(Container::Linear(rows)));
        Self { tree: egui_tiles::Tree::new("layout", root, tiles), popped: Vec::new() }
    }
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
    /// None on malformed / incompatible JSON (caller falls back to the default layout).
    pub fn from_json(s: &str) -> Option<Self> {
        let l: Self = serde_json::from_str(s).ok()?;
        l.tree.root().is_some().then_some(l)
    }
    /// Visible = docked in the tree (and not hidden) or popped out.
    pub fn is_visible(&self, pane: Pane) -> bool {
        if self.popped.contains(&pane) {
            return true;
        }
        self.tree.tiles.find_pane(&pane).map(|id| self.tree.tiles.is_visible(id)).unwrap_or(false)
    }
    /// Show (back where it was in the tree, or into the root when it is gone) or hide (invisible in the
    /// tree / close the popout).
    pub fn toggle(&mut self, pane: Pane) {
        if let Some(i) = self.popped.iter().position(|&p| p == pane) {
            self.popped.remove(i);
            return;
        }
        match self.tree.tiles.find_pane(&pane) {
            Some(id) if self.tree.tiles.is_visible(id) => self.tree.tiles.set_visible(id, false),
            Some(id) => {
                self.tree.tiles.set_visible(id, true);
                self.tree.make_active(|tid, _| tid == id);
            }
            None => self.insert_into_root(pane),
        }
    }
    /// Make the pane visible (docked or popped) and, if it is a tab, the active one.
    pub fn reveal(&mut self, pane: Pane) {
        if self.popped.contains(&pane) {
            return;
        }
        match self.tree.tiles.find_pane(&pane) {
            Some(id) => {
                self.tree.tiles.set_visible(id, true);
                self.tree.make_active(|tid, _| tid == id);
            }
            None => self.insert_into_root(pane),
        }
    }
    pub fn popout(&mut self, pane: Pane) {
        if self.popped.contains(&pane) {
            return;
        }
        if let Some(id) = self.tree.tiles.find_pane(&pane) {
            self.tree.tiles.set_visible(id, false);
        }
        self.popped.push(pane);
    }
    pub fn dock(&mut self, pane: Pane) {
        self.popped.retain(|&p| p != pane);
        self.reveal(pane);
    }
    pub fn reset(&mut self) {
        *self = Self::default_layout();
    }
    /// A pane that fell out of the tree entirely (e.g. an old profile): add it as a new tab in the root.
    fn insert_into_root(&mut self, pane: Pane) {
        let id = self.tree.tiles.insert_pane(pane);
        match self.tree.root().and_then(|r| self.tree.tiles.get_mut(r)) {
            Some(egui_tiles::Tile::Container(c)) => c.add_child(id),
            _ => {
                let root = self.tree.tiles.insert_tab_tile(vec![id]);
                self.tree = egui_tiles::Tree::new("layout", root, std::mem::take(&mut self.tree.tiles));
            }
        }
        self.tree.make_active(|tid, _| tid == id);
    }
}

struct Behaviour<'a> {
    draw: &'a mut dyn FnMut(&mut egui::Ui, Pane),
    hide: Vec<Pane>,
    pop: Vec<Pane>,
    edited: bool,
}

impl egui_tiles::Behavior<Pane> for Behaviour<'_> {
    fn pane_ui(&mut self, ui: &mut egui::Ui, _tile_id: egui_tiles::TileId, pane: &mut Pane) -> egui_tiles::UiResponse {
        (self.draw)(ui, *pane);
        egui_tiles::UiResponse::None
    }
    fn tab_title_for_pane(&mut self, pane: &Pane) -> egui::WidgetText {
        pane.title().into()
    }
    fn top_bar_right_ui(
        &mut self,
        tiles: &egui_tiles::Tiles<Pane>,
        ui: &mut egui::Ui,
        _tile_id: egui_tiles::TileId,
        tabs: &egui_tiles::Tabs,
        _scroll_offset: &mut f32,
    ) {
        // U+1F5D9 / U+1F5D6: in egui's bundled emoji-icon font (✕ and ⧉ are not, and Segoe UI lacks both)
        let Some(&pane) = tabs.active.and_then(|id| tiles.get_pane(&id)) else { return };
        if ui.small_button("🗙").on_hover_text("Hide (View menu shows it again)").clicked() {
            self.hide.push(pane);
        }
        if ui.small_button("🗖").on_hover_text("Pop out into its own window").clicked() {
            self.pop.push(pane);
        }
    }
    fn simplification_options(&self) -> egui_tiles::SimplificationOptions {
        egui_tiles::SimplificationOptions { all_panes_must_have_tabs: true, ..Default::default() }
    }
    fn on_edit(&mut self, _edit_action: egui_tiles::EditAction) {
        self.edited = true;
    }
}

/// Draw the docked tree into `ui` and every popped pane in its own OS window; `draw(ui, pane)` renders
/// a pane's content. A popped window that the user closes is docked back automatically.
/// Returns true when the layout changed this frame (caller persists it).
pub fn show(
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    layout: &mut Layout,
    draw: &mut dyn FnMut(&mut egui::Ui, Pane),
) -> bool {
    let mut beh = Behaviour { draw, hide: Vec::new(), pop: Vec::new(), edited: false };
    layout.tree.ui(&mut beh, ui);
    let (hide, pop, mut changed) = (beh.hide, beh.pop, beh.edited);
    for p in hide {
        if layout.is_visible(p) {
            layout.toggle(p);
            changed = true;
        }
    }
    for p in pop {
        layout.popout(p);
        changed = true;
    }
    let mut to_dock: Vec<Pane> = Vec::new();
    for &pane in &layout.popped {
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of(("pane", pane)),
            egui::ViewportBuilder::default().with_title(pane.title()).with_inner_size([800.0, 500.0]),
            |ctx, _class| {
                if ctx.input(|i| i.viewport().close_requested()) {
                    to_dock.push(pane);
                }
                egui::CentralPanel::default().show(ctx, |ui| draw(ui, pane));
            },
        );
    }
    for p in to_dock {
        layout.dock(p);
        changed = true;
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_layout_contains_every_pane() {
        let l = Layout::default_layout();
        for p in Pane::ALL {
            assert!(l.tree.tiles.find_pane(&p).is_some(), "{p:?} missing from the default layout");
            assert!(l.is_visible(p), "{p:?} should be visible (a tab may be inactive but is not hidden)");
        }
        // Timeline is the active bottom tab: Curves / AutoCut are behind it (inactive, still visible)
        let timeline = l.tree.tiles.find_pane(&Pane::Timeline).unwrap();
        let parent = l.tree.tiles.parent_of(timeline).unwrap();
        match l.tree.tiles.get_container(parent).unwrap() {
            egui_tiles::Container::Tabs(t) => assert_eq!(t.active, Some(timeline)),
            c => panic!("Timeline parent is {c:?}, expected tabs"),
        }
    }

    #[test]
    fn toggle_hides_and_shows() {
        let mut l = Layout::default_layout();
        assert!(l.is_visible(Pane::Library));
        l.toggle(Pane::Library);
        assert!(!l.is_visible(Pane::Library));
        l.toggle(Pane::Library);
        assert!(l.is_visible(Pane::Library));
        // reveal is idempotent and never hides
        l.reveal(Pane::Curves);
        l.reveal(Pane::Curves);
        assert!(l.is_visible(Pane::Curves));
    }

    #[test]
    fn popout_and_dock() {
        let mut l = Layout::default_layout();
        l.popout(Pane::Inspector);
        assert!(l.popped.contains(&Pane::Inspector));
        assert!(l.is_visible(Pane::Inspector)); // popped counts as visible
        let id = l.tree.tiles.find_pane(&Pane::Inspector).unwrap();
        assert!(!l.tree.tiles.is_visible(id), "popped pane must not draw in the tree too");
        // toggling a popped pane closes the popout
        l.toggle(Pane::Inspector);
        assert!(!l.is_visible(Pane::Inspector) && l.popped.is_empty());
        l.popout(Pane::Effects);
        l.dock(Pane::Effects);
        assert!(l.popped.is_empty());
        assert!(l.is_visible(Pane::Effects));
    }

    #[test]
    fn json_roundtrip() {
        let mut l = Layout::default_layout();
        l.toggle(Pane::Planner);
        l.popout(Pane::Library);
        let json = l.to_json();
        let r = Layout::from_json(&json).expect("roundtrip");
        assert_eq!(r.popped, vec![Pane::Library]);
        assert!(!r.is_visible(Pane::Planner));
        assert!(r.is_visible(Pane::Timeline));
        assert!(Layout::from_json("{").is_none());
        assert!(Layout::from_json("{}").is_none());
    }

    #[test]
    fn lost_pane_is_reinserted() {
        let mut l = Layout::default_layout();
        // simulate a profile that lost a pane entirely
        let id = l.tree.tiles.find_pane(&Pane::Planner).unwrap();
        l.tree.tiles.remove(id);
        assert!(!l.is_visible(Pane::Planner));
        l.toggle(Pane::Planner);
        assert!(l.is_visible(Pane::Planner));
    }
}
