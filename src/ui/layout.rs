//! Dockable editor layout (egui_tiles): panes can be rearranged by dragging their tabs, split, tabbed,
//! hidden/shown, popped out into their own OS windows (egui immediate viewports) and saved/loaded as
//! profiles (JSON — also written to / read from `.sedit-layout` files so profiles can be shared).
//!
//! Default layout (DaVinci-like), two rows of four columns:
//! top    = [Library | Preview with the Tools strip beneath it | Inspector | tabs(Curves, Nodes, Tracking)]
//! bottom = [tabs(Effects, Transitions, Presets) | Timeline | tabs(Mixer, Auto-cut, Subtitles) |
//!           tabs(Markers, Planner)].
//! The Timeline is a column of its own rather than a tab, so nothing can ever hide it. `show` draws the
//! tree in the given `ui` and each popped pane in its own viewport (closing that window docks the pane
//! back); `draw(ui, pane)` renders a pane. Behaviour: tab titles = Pane::title, a "⧉" button in the tab
//! bar pops the active pane out, "✕" hides it. Hidden panes stay in the tree (egui_tiles visibility) so
//! they come back exactly where they were.
//!
//! Dragging a tab paints nine drop squares over the tile under the cursor (centre = tabify, the eight
//! around it = split), the dropped pane keeps the fraction of its parent it had instead of taking half of
//! wherever it landed, and the move goes onto a small undo stack of its own so Ctrl+Z puts it back.
//!
//! Migration: a stored layout from an older version does not know the round-3 panes; `from_json` rejects
//! it (None) so the app falls back to this default instead of an editor with no Tools/Mixer/Markers.

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
    // ---- round 3 ----
    Tools,
    Nodes,
    Mixer,
    Markers,
    /// Machine-local reusable things (effect chains, node graphs, adjustment layers, templates).
    Presets,
    /// Point / area tracking of a clip into a reusable project path.
    Tracking,
}

impl Pane {
    pub const ALL: [Pane; 16] = [
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
        Pane::Tools,
        Pane::Nodes,
        Pane::Mixer,
        Pane::Markers,
        Pane::Presets,
        Pane::Tracking,
    ];
    /// Panes added in round 3 — a stored layout without them is from an older version (see `from_json`).
    pub const ROUND3: [Pane; 4] = [Pane::Tools, Pane::Nodes, Pane::Mixer, Pane::Markers];
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
            Pane::Tools => "Tools",
            Pane::Nodes => "Nodes",
            Pane::Mixer => "Mixer",
            Pane::Markers => "Markers",
            Pane::Presets => "Presets",
            Pane::Tracking => "Tracking",
        }
    }
}

/// The whole layout: the docked tree plus panes currently popped out into their own windows.
#[derive(Clone, Serialize, Deserialize)]
pub struct Layout {
    pub tree: egui_tiles::Tree<Pane>,
    #[serde(default)]
    pub popped: Vec<Pane>,
    /// The layout's own undo history (serialised layouts): the layout is not the project, so it cannot
    /// ride the project's snapshots. Never persisted, and 20 steps is plenty for "put that tab back".
    #[serde(skip)]
    pub undo: Vec<String>,
    #[serde(skip)]
    pub redo: Vec<String>,
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
        let tabs = |tiles: &mut egui_tiles::Tiles<Pane>, panes: &[Pane]| {
            let ids: Vec<_> = panes.iter().map(|&p| tiles.insert_pane(p)).collect();
            tiles.insert_tab_tile(ids) // the first pane is the active tab
        };
        // a row of four columns with the given fractions; panes are listed explicitly so a variant this
        // default does not know about is simply absent (the View menu still brings it in)
        let row = |tiles: &mut egui_tiles::Tiles<Pane>, cols: [(egui_tiles::TileId, f32); 4]| {
            let mut lin = Linear::new(LinearDir::Horizontal, cols.iter().map(|&(id, _)| id).collect());
            for (id, share) in cols {
                lin.shares.set_share(id, share);
            }
            tiles.insert_new(Tile::Container(Container::Linear(lin)))
        };
        // centre column: the viewport with the Tools strip directly beneath it
        let preview = tiles.insert_pane(Pane::Preview);
        let tools = tiles.insert_pane(Pane::Tools);
        let mut centre_col = Linear::new(LinearDir::Vertical, vec![preview, tools]);
        centre_col.shares.set_share(preview, 0.9);
        centre_col.shares.set_share(tools, 0.1);
        let centre = tiles.insert_new(Tile::Container(Container::Linear(centre_col)));
        let library = tiles.insert_pane(Pane::Library);
        let inspector = tiles.insert_pane(Pane::Inspector);
        // ponytail: Presets / Tracking ride along as trailing tabs of the group they belong with — the
        // requested arrangement does not place them, and a homeless pane is only reachable via the menu
        let graphs = tabs(&mut tiles, &[Pane::Curves, Pane::Nodes, Pane::Tracking]);
        let top = row(&mut tiles, [(library, 0.18), (centre, 0.44), (inspector, 0.2), (graphs, 0.18)]);
        // the Timeline is a column of its own, not a tab: nothing can end up in front of it
        let looks = tabs(&mut tiles, &[Pane::Effects, Pane::Transitions, Pane::Presets]);
        let timeline = tiles.insert_pane(Pane::Timeline);
        let mix = tabs(&mut tiles, &[Pane::Mixer, Pane::AutoCut, Pane::Subtitles]);
        let notes = tabs(&mut tiles, &[Pane::Markers, Pane::Planner]);
        let bottom = row(&mut tiles, [(looks, 0.18), (timeline, 0.44), (mix, 0.2), (notes, 0.18)]);
        let mut rows = Linear::new(LinearDir::Vertical, vec![top, bottom]);
        rows.shares.set_share(top, 0.62);
        rows.shares.set_share(bottom, 0.38);
        let root = tiles.insert_new(Tile::Container(Container::Linear(rows)));
        Self::new(egui_tiles::Tree::new("layout", root, tiles))
    }
    /// A layout around a tree, with an empty history.
    pub fn new(tree: egui_tiles::Tree<Pane>) -> Self {
        Self { tree, popped: Vec::new(), undo: Vec::new(), redo: Vec::new() }
    }
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
    /// None on malformed / incompatible JSON (caller falls back to the default layout). A layout saved
    /// before round 3 knows nothing about Tools / Nodes / Mixer / Markers: rather than dropping four panes
    /// into the root as loose tabs, it is rejected here and the caller resets to the new default.
    pub fn from_json(s: &str) -> Option<Self> {
        let l: Self = serde_json::from_str(s).ok()?;
        l.tree.root()?;
        let has_all = Pane::ROUND3.iter().all(|p| l.tree.tiles.find_pane(p).is_some() || l.popped.contains(p));
        has_all.then_some(l)
    }

    /// Same, but an explicitly saved profile is migrated instead of rejected: panes it predates are added
    /// to the root rather than making the whole profile unloadable.
    pub fn from_json_migrating(s: &str) -> Option<Self> {
        let mut l: Self = serde_json::from_str(s).ok()?;
        l.tree.root()?;
        for p in Pane::ALL {
            if l.tree.tiles.find_pane(&p).is_none() && !l.popped.contains(&p) {
                l.insert_into_root(p);
            }
        }
        Some(l)
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
        let (undo, redo) = (std::mem::take(&mut self.undo), std::mem::take(&mut self.redo));
        *self = Self::default_layout();
        (self.undo, self.redo) = (undo, redo);
    }
    /// Remember the arrangement `snapshot` was taken from (before the move), so Ctrl+Z can go back to it.
    pub fn push_undo(&mut self, snapshot: String) {
        self.undo.push(snapshot);
        if self.undo.len() > 20 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
    pub fn undo(&mut self) -> bool {
        self.step(true)
    }
    pub fn redo(&mut self) -> bool {
        self.step(false)
    }
    fn step(&mut self, undoing: bool) -> bool {
        let current = self.to_json();
        let Some(json) = (if undoing { &mut self.undo } else { &mut self.redo }).pop() else { return false };
        let Some(other) = Self::from_json(&json) else { return false };
        (self.tree, self.popped) = (other.tree, other.popped);
        if undoing { &mut self.redo } else { &mut self.undo }.push(current);
        true
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

/// How much of its linear parent a tile takes up (None when the parent is a tab bar — a tab has no share).
fn share_fraction(tree: &egui_tiles::Tree<Pane>, id: egui_tiles::TileId) -> Option<f32> {
    let parent = tree.tiles.parent_of(id)?;
    let egui_tiles::Container::Linear(lin) = tree.tiles.get_container(parent)? else { return None };
    let total: f32 = lin.children.iter().map(|&c| lin.shares[c]).sum();
    (total > 0.0).then(|| lin.shares[id] / total)
}

/// Give a just-dropped tile the same fraction of its new parent as it had in the old one: a fresh split
/// hands out 1:1 shares, which silently halves whatever you dropped the pane onto.
fn keep_share_fraction(tree: &mut egui_tiles::Tree<Pane>, id: egui_tiles::TileId, fraction: f32) {
    let fraction = fraction.clamp(0.05, 0.95);
    let Some(parent) = tree.tiles.parent_of(id) else { return };
    let Some(egui_tiles::Tile::Container(egui_tiles::Container::Linear(lin))) = tree.tiles.get_mut(parent) else {
        return;
    };
    let others: f32 = lin.children.iter().filter(|&&c| c != id).map(|&c| lin.shares[c]).sum();
    if others > 0.0 {
        lin.shares.set_share(id, others * fraction / (1.0 - fraction));
    }
}

/// One entry of the "Load profile" menu. A menu sizes itself from the previous frame's content, so a
/// name that wraps makes the menu narrower, which wraps it harder — after a few frames "Editor 1" has
/// collapsed to "Edito / r 1". Measuring the name pins the width instead of letting it feed back, and
/// anything past the cap truncates on one line with the whole name on hover.
pub fn profile_button(ui: &mut egui::Ui, name: &str) -> egui::Response {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let text = ui.painter().layout_no_wrap(name.to_owned(), font, egui::Color32::PLACEHOLDER).size().x;
    let wanted = text + ui.spacing().button_padding.x * 2.0;
    ui.scope(|ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        ui.set_min_width(wanted.min(240.0));
        ui.button(name)
    })
    .inner
    .on_hover_text(name)
}

struct Behaviour<'a> {
    draw: &'a mut dyn FnMut(&mut egui::Ui, Pane),
    hide: Vec<Pane>,
    pop: Vec<Pane>,
    edited: bool,
    dropped: bool,
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
    fn on_edit(&mut self, edit_action: egui_tiles::EditAction) {
        self.edited = true;
        self.dropped |= edit_action == egui_tiles::EditAction::TileDropped;
    }
    /// Nine drop squares over the hovered tile instead of egui_tiles' thin outline: the centre one tabs
    /// the pane in, the eight around it split in that direction. egui_tiles still owns the hit test, so a
    /// corner resolves to whichever of its two edges is nearer — the translucent fill is where it lands.
    fn paint_drag_preview(
        &self,
        visuals: &egui::Visuals,
        painter: &egui::Painter,
        parent_rect: Option<egui::Rect>,
        preview_rect: egui::Rect,
    ) {
        let area = parent_rect.unwrap_or(preview_rect);
        let stroke = self.drag_preview_stroke(visuals);
        let fill = self.drag_preview_color(visuals);
        painter.rect_filled(preview_rect, 1.0, fill.gamma_multiply(0.35));
        painter.rect_stroke(area, 1.0, stroke, egui::StrokeKind::Inside);
        let side = (area.size().min_elem() * 0.14).clamp(14.0, 34.0);
        let step = side * 1.18;
        // which ninth the pointer is in, so the square under it can light up
        let hot = painter.ctx().pointer_interact_pos().map(|p| {
            let cell = |v: f32, min: f32, len: f32| (3.0 * (v - min) / len.max(1.0)).floor().clamp(0.0, 2.0) as i32;
            (cell(p.x, area.left(), area.width()), cell(p.y, area.top(), area.height()))
        });
        for row in 0..3 {
            for col in 0..3 {
                let c = area.center() + egui::vec2((col - 1) as f32, (row - 1) as f32) * step;
                let sq = egui::Rect::from_center_size(c, egui::Vec2::splat(side));
                let bg = if hot == Some((col, row)) { stroke.color } else { fill.gamma_multiply(0.6) };
                painter.rect(sq, 2.0, bg, stroke, egui::StrokeKind::Inside);
            }
        }
    }
}

/// Draw the docked tree into `ui` and every popped pane in its own OS window; `draw(ui, pane)` renders
/// a pane's content. A popped window that the user closes is docked back automatically.
/// Returns (layout changed this frame — caller persists it, a pane was dropped somewhere new).
pub fn show(
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    layout: &mut Layout,
    draw: &mut dyn FnMut(&mut egui::Ui, Pane),
) -> (bool, bool) {
    // the drop happens inside tree.ui(), so grab the "before" state while a drag is still in flight
    let dragged = layout.tree.dragged_id(ctx).map(|id| (id, share_fraction(&layout.tree, id), layout.to_json()));
    let mut beh = Behaviour { draw, hide: Vec::new(), pop: Vec::new(), edited: false, dropped: false };
    layout.tree.ui(&mut beh, ui);
    let mut moved = false;
    if let (true, Some((id, fraction, before))) = (beh.dropped, dragged) {
        layout.push_undo(before);
        if let Some(f) = fraction {
            keep_share_fraction(&mut layout.tree, id, f);
        }
        moved = true;
    }
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
    (changed, moved)
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
        // the Timeline is a column of its own, so no tab can sit in front of it
        let timeline = l.tree.tiles.find_pane(&Pane::Timeline).unwrap();
        assert!(matches!(l.tree.tiles.get(timeline), Some(egui_tiles::Tile::Pane(Pane::Timeline))));
    }

    /// The requested default: two rows of four columns, none of them opening unusably small.
    #[test]
    fn default_layout_arrangement() {
        let l = Layout::default_layout();
        let panes = |id: egui_tiles::TileId| -> Vec<Pane> {
            match l.tree.tiles.get(id) {
                Some(egui_tiles::Tile::Pane(p)) => vec![*p],
                Some(egui_tiles::Tile::Container(c)) => {
                    c.children().filter_map(|k| l.tree.tiles.get_pane(k)).copied().collect()
                }
                None => Vec::new(),
            }
        };
        let columns = |id: egui_tiles::TileId| -> Vec<Vec<Pane>> {
            let Some(egui_tiles::Container::Linear(lin)) = l.tree.tiles.get_container(id) else {
                panic!("{id:?} is not a row")
            };
            assert_eq!(lin.dir, egui_tiles::LinearDir::Horizontal);
            for &c in &lin.children {
                let f = share_fraction(&l.tree, c).unwrap();
                assert!(f >= 0.15, "a column opens at {f} of the row");
            }
            lin.children.iter().map(|&c| panes(c)).collect()
        };
        let root = l.tree.root().unwrap();
        let Some(egui_tiles::Container::Linear(rows)) = l.tree.tiles.get_container(root) else { panic!("no rows") };
        assert_eq!(rows.dir, egui_tiles::LinearDir::Vertical);
        let (top, bottom) = (rows.children[0], rows.children[1]);
        assert_eq!(
            columns(top),
            vec![
                vec![Pane::Library],
                vec![Pane::Preview, Pane::Tools], // the Tools strip lives under the viewport
                vec![Pane::Inspector],
                vec![Pane::Curves, Pane::Nodes, Pane::Tracking],
            ]
        );
        assert_eq!(
            columns(bottom),
            vec![
                vec![Pane::Effects, Pane::Transitions, Pane::Presets],
                vec![Pane::Timeline],
                vec![Pane::Mixer, Pane::AutoCut, Pane::Subtitles],
                vec![Pane::Markers, Pane::Planner],
            ]
        );
        // the Preview column really is vertical (Tools beneath, not beside)
        let col = l.tree.tiles.parent_of(l.tree.tiles.find_pane(&Pane::Preview).unwrap()).unwrap();
        match l.tree.tiles.get_container(col).unwrap() {
            egui_tiles::Container::Linear(lin) => assert_eq!(lin.dir, egui_tiles::LinearDir::Vertical),
            c => panic!("Preview column is {c:?}, expected a vertical linear"),
        }
    }

    /// A layout stored before round 3 must not survive: it has no Tools / Mixer / Nodes / Markers.
    #[test]
    fn old_layouts_are_rejected_so_the_app_resets() {
        let l = Layout::default_layout();
        let json = l.to_json();
        assert!(Layout::from_json(&json).is_some());
        // drop one round-3 pane from the tree: that is what an old profile looks like
        let mut old = l.clone();
        let id = old.tree.tiles.find_pane(&Pane::Mixer).unwrap();
        old.tree.tiles.remove(id);
        assert!(Layout::from_json(&old.to_json()).is_none());
        // …unless it is popped out into its own window
        let mut popped = l.clone();
        popped.popout(Pane::Markers);
        assert!(Layout::from_json(&popped.to_json()).is_some());
    }

    /// A named profile is worth migrating rather than reporting as corrupt: the panes it predates are
    /// added back and everything it did lay out survives.
    #[test]
    fn old_profiles_are_migrated_not_rejected() {
        let mut old = Layout::default_layout();
        for p in [Pane::Mixer, Pane::Nodes] {
            let id = old.tree.tiles.find_pane(&p).unwrap();
            old.tree.tiles.remove(id);
        }
        let json = old.to_json();
        assert!(Layout::from_json(&json).is_none(), "the auto-restored layout still resets");
        let migrated = Layout::from_json_migrating(&json).expect("a saved profile still loads");
        for p in Pane::ALL {
            assert!(migrated.tree.tiles.find_pane(&p).is_some(), "{p:?} missing after the migration");
        }
        assert!(migrated.is_visible(Pane::Timeline), "the panes it did have are untouched");
        // genuinely broken JSON is still refused
        assert!(Layout::from_json_migrating("not json").is_none());
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

    /// Moving a pane keeps the slice of its parent it had (not the even split a fresh container hands
    /// out), and the move is undoable / redoable on the layout's own stack.
    #[test]
    fn move_keeps_its_share_and_is_undoable() {
        let mut l = Layout::default_layout();
        let tools = l.tree.tiles.find_pane(&Pane::Tools).unwrap();
        let column = l.tree.tiles.parent_of(tools).unwrap();
        let was = share_fraction(&l.tree, tools).unwrap();
        assert!((was - 0.1).abs() < 1e-4, "Tools owns a tenth of the centre column, got {was}");

        // move it into the root row, exactly what a drop on a horizontal/vertical edge ends up doing
        let root = l.tree.root().unwrap();
        let snapshot = l.to_json();
        l.tree.move_tile_to_container(tools, root, 2, false);
        assert!((share_fraction(&l.tree, tools).unwrap() - 0.5).abs() < 1e-4, "egui_tiles splits evenly");
        keep_share_fraction(&mut l.tree, tools, was);
        assert!((share_fraction(&l.tree, tools).unwrap() - was).abs() < 1e-4, "the recorded fraction is back");
        // and the two rows it joined keep their proportions to each other (0.68 : 0.32)
        let top = l.tree.tiles.parent_of(l.tree.tiles.find_pane(&Pane::Preview).unwrap()).unwrap();
        let top = l.tree.tiles.parent_of(top).unwrap();
        let bottom = l.tree.tiles.parent_of(l.tree.tiles.find_pane(&Pane::Timeline).unwrap()).unwrap();
        let ratio = share_fraction(&l.tree, top).unwrap() / share_fraction(&l.tree, bottom).unwrap();
        assert!((ratio - 0.62 / 0.38).abs() < 1e-3, "the rest of the row was redistributed: {ratio}");

        l.push_undo(snapshot);
        assert!(l.undo(), "the move undoes");
        assert_eq!(l.tree.tiles.parent_of(l.tree.tiles.find_pane(&Pane::Tools).unwrap()), Some(column));
        assert!(l.redo(), "and redoes");
        let tools = l.tree.tiles.find_pane(&Pane::Tools).unwrap();
        assert_eq!(l.tree.tiles.parent_of(tools), Some(root));
        assert!((share_fraction(&l.tree, tools).unwrap() - was).abs() < 1e-4, "the fraction survives the trip");
        assert!(!l.redo(), "nothing left to redo");
    }

    /// A menu is as wide as what it drew last frame, so feeding that width back in is what collapsed
    /// "Editor 1" into two lines: the entry must keep its width and stay one row tall.
    #[test]
    fn profile_name_stays_on_one_line() {
        let ctx = egui::Context::default();
        // a menu ui: top-down justified, as wide as the last frame's content
        let frame = |ctx: &egui::Context, w: f32, add: &mut dyn FnMut(&mut egui::Ui) -> f32| {
            let (mut inner, mut width) = (0.0, w);
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let rect = egui::Rect::from_min_size(ui.max_rect().min, egui::vec2(w, 200.0));
                    let b = egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::top_down_justified(egui::Align::Min));
                    ui.scope_builder(b, |ui| {
                        inner = add(ui);
                        width = ui.min_rect().width();
                    });
                });
            });
            (inner, width)
        };
        let mut width = 400.0;
        let mut height = 0.0;
        for _ in 0..5 {
            (height, width) = frame(&ctx, width, &mut |ui| profile_button(ui, "Editor 1").rect.height());
        }
        assert!(width > 40.0, "the menu collapsed to {width} px wide");
        // in a pane too narrow for it the name truncates instead of wrapping onto a second line
        let (wrapped, _) = frame(&ctx, 30.0, &mut |ui| ui.button("Editor 1").rect.height());
        let (kept, _) = frame(&ctx, 30.0, &mut |ui| profile_button(ui, "Editor 1").rect.height());
        assert!(kept < wrapped, "the name still wraps: {kept} px vs {wrapped} px for a plain button");
        assert!((kept - height).abs() < 1.0, "one row either way: {kept} px vs {height} px");
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
