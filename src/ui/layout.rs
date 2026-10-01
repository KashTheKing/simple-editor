//! Dockable editor layout (egui_tiles), one tree per page (`PAGES`: Media / Cut / Edit / Color / Audio /
//! Export, the Resolve-style switcher in the menu bar); every page's default puts the Inspector top
//! right. Panes can be split, tabbed, hidden/shown, popped out into their own OS windows (egui immediate
//! viewports, which reopen where they were) and saved/loaded as profiles (JSON - also written to / read
//! from `.sedit-layout` files so profiles can be shared). Every builder ends with `stack_unplaced`,
//! so every pane exists in every page, hidden behind one group until the Window menu or a tab bar's `+`
//! brings it in. Hidden panes stay in the tree (egui_tiles visibility), so they come back where they were.
//!
//! Panels are unlocked by default: a tab drags to re-dock (`Chrome::locked`, Window ▸ Layout ▸ Lock
//! panels, makes a tab click-only); split dividers resize either way and Undock always works from a
//! tab's right-click. A drag paints nine drop squares over the tile under the cursor (centre =
//! tabify, the eight around it = split), the dropped pane keeps the fraction of its parent it had, and
//! the move goes onto a small undo stack of its own so Ctrl+Z puts it back.
//!
//! Migration: a stored layout from an older version does not know the round-3 panes; `from_json` rejects
//! it (None) so the app falls back to the page's default instead of an editor with no Mixer/Markers.

use crate::hotkeys::Action;
use crate::ui::menu;
use crate::ui::tools::{draw_glyph, Glyph};
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Instant;

// ---- ws:pages ----
/// The pages in Resolve's order, left to right in the menu-bar switcher and Alt+1..6
/// (`Action::Workspace1..6`). Each page keeps its own tree (`Settings.layout` = the current one,
/// `Settings.page_layouts` the others); `page_layout` maps a name to its built-in builder.
pub const PAGES: &[&str] = &["Media", "Cut", "Edit", "Color", "Audio", "Export"];

/// The built-in builder behind a `PAGES` name (`None` for a name that isn't one).
pub fn page_layout(name: &str) -> Option<fn() -> Layout> {
    Some(match name {
        "Media" => Layout::media_layout,
        "Cut" => Layout::cut_layout,
        "Edit" => Layout::default_layout,
        "Color" => Layout::color_layout,
        "Audio" => Layout::audio_layout,
        "Export" => Layout::export_layout,
        _ => return None,
    })
}

/// The tool a page opens with: the Cut page cuts, every other page selects.
pub fn page_tool(page: &str) -> crate::ui::tools::Tool {
    use crate::ui::tools::Tool;
    if page == "Cut" {
        Tool::Cut
    } else {
        Tool::Select
    }
}

/// A page by name, case-insensitively, including the six pre-pages workspace names (Simple / Text ->
/// Edit, Deliver -> Export) so old scripts, `layout.workspace` calls and settings files keep working.
pub fn page_name(name: &str) -> Option<&'static str> {
    let name = name.trim();
    let old = [("Simple", "Edit"), ("Text", "Edit"), ("Deliver", "Export")];
    PAGES
        .iter()
        .copied()
        .find(|p| p.eq_ignore_ascii_case(name))
        .or_else(|| old.iter().find(|(w, _)| w.eq_ignore_ascii_case(name)).map(|&(_, p)| p))
}

/// How long a tab's accent underline ("glow") lasts after a selection asked for a pane that could not,
/// or must not, be switched to ("Stay on this tab", or following the selection switched off).
pub const GLOW_SECS: f32 = 1.2;

/// What a programmatic, selection-driven reveal attempt did - so the caller can glow the tab instead of
/// switching when the user has opted that group out (`Pinned`), skip a pane the user hid with the tab
/// bar's cross (`Hidden` - auto-surfacing never re-opens a closed pane, that would be exactly the
/// "panels jumping around" goals.md forbids), or fall through to another candidate (`Absent`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surfaced {
    /// Now visible and, if tabbed, the active tab (or it already was).
    Shown,
    /// Not switched: the pane itself, or the active tab of its group, is pinned.
    Pinned,
    /// Docked but hidden (`set_visible(false)`), e.g. closed with the tab bar's cross.
    Hidden,
    /// Not in the tree at all (an old profile); nothing was inserted.
    Absent,
}

/// Icon for a pane's tab / menu entry: the user's Settings → Appearance override first
/// ("none" = no icon), then the built-in default.
pub fn pane_icon(icons: &BTreeMap<String, String>, pane: Pane) -> Option<Glyph> {
    if let Some(name) = icons.get(&format!("pane.{}", pane.title())) {
        return if name == "none" { None } else { Glyph::from_name(name) };
    }
    Some(pane.glyph())
}

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
    /// Standalone moodboard (`Project.moodboard`) - gallery/list/slideshow of reference assets, separate
    /// from the per-task moodboards already on the Planner's items.
    Moodboard,
    /// Undo-stack history, grouped/searchable/filterable, exportable to Markdown.
    History,
    // ---- ws:registries-schema-hooks ----
    /// Dockable two-up source monitor (player + in/out marks); a placeholder this wave - its real
    /// content lands with ws:source-monitor (wave 2). Tab-stacked hidden behind Library in every
    /// preset (`stack_unplaced`), never in `ROUND3`.
    Source,
    // ---- ws:jobs-panel ----
    /// Every background job (running / queued / recent) with Cancel and queue reordering. Tab-stacked
    /// hidden in every preset (`stack_unplaced`, like `Source`), never in `ROUND3`.
    Jobs,
    // ---- ws:pages ----
    /// Waveform / Parade / Vectorscope / Histogram (was a floating window). The GPU's stats readback
    /// runs only while this was drawn last frame (`monitor_tick`).
    Scopes,
    /// The export settings (platform tiles + Advanced; was the Export window) - the Export page's
    /// left column. Ctrl+E switches to that page.
    Export,
    /// Recent files, linked folders and the disk (was the Library's "Browse" view) - under the Library
    /// on the Media page, hidden elsewhere.
    MediaBrowser,
    /// The Color page's Lift/Gamma/Gain/Offset wheels (or Primaries bars) with per-wheel keyframes.
    Grade,
    /// The Color page's clip thumbnail strip: one card per video clip, click to grade it.
    Clips,
}

impl Pane {
    /// A slice, not a fixed-size array (see `stack_unplaced`'s risk note): a new Pane variant only
    /// needs a line here, not a signature change at every `Pane::ALL` call site.
    pub const ALL: &'static [Pane] = &[
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
        // (Tools left with ws:viewer-surface: its tools live on the viewer's rail; a stored tile of it
        // is dropped on load - `drop_retired`)
        Pane::Nodes,
        Pane::Mixer,
        Pane::Markers,
        Pane::Presets,
        Pane::Tracking,
        Pane::Moodboard,
        Pane::History,
        // ---- ws:registries-schema-hooks ----
        Pane::Source,
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
        // ---- ws:jobs-panel ----
        Pane::Jobs,
        // ---- ws:pages ----
        Pane::Scopes,
        Pane::Export,
        Pane::MediaBrowser,
        // ---- ws:color-page ----
        Pane::Grade,
        Pane::Clips,
    ];
    /// Panes added in round 3 - a stored layout without them is from an older version (see `from_json`).
    /// Tools left this list with the pages (no page places it; the viewer's tool rail replaces it).
    pub const ROUND3: [Pane; 3] = [Pane::Nodes, Pane::Mixer, Pane::Markers];
    /// Default icon for this pane (tabs, the `+` menu, icon picker). Overridable in Settings → Appearance.
    pub fn glyph(self) -> Glyph {
        match self {
            Pane::Preview => Glyph::Clapperboard,
            Pane::Timeline => Glyph::FilmStrip,
            Pane::Library => Glyph::Folder,
            Pane::Inspector => Glyph::Sliders,
            Pane::Effects => Glyph::Bolt,
            Pane::Transitions => Glyph::Transition,
            Pane::Curves => Glyph::CurveIcon,
            Pane::Subtitles => Glyph::Subtitles,
            Pane::Planner => Glyph::Notepad,
            Pane::AutoCut => Glyph::Waveform,
            Pane::Tools => Glyph::Wrench,
            Pane::Nodes => Glyph::Nodes,
            Pane::Mixer => Glyph::SpeakerOn,
            Pane::Markers => Glyph::Flag,
            Pane::Presets => Glyph::Bookmark,
            Pane::Tracking => Glyph::Target,
            Pane::Moodboard => Glyph::GridIcon,
            Pane::History => Glyph::History,
            // ws:source-monitor (wave 2) may pick a more specific glyph later.
            Pane::Source => Glyph::Camera,
            // reuses export-deliver's queue glyph - no new Glyph variant
            Pane::Jobs => Glyph::Queue,
            Pane::Scopes => Glyph::Scope,
            Pane::Export => Glyph::ExportArrow,
            Pane::MediaBrowser => Glyph::Link,
            Pane::Grade => Glyph::Wheel,
            Pane::Clips => Glyph::FilmStrip,
        }
    }
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
            // the pane has drawn the Gallery since inspector-gallery; the variant keeps its persisted name
            Pane::Presets => "Gallery",
            Pane::Tracking => "Tracking",
            Pane::Moodboard => "Moodboard",
            Pane::History => "History",
            Pane::Source => "Source",
            Pane::Jobs => "Jobs",
            Pane::Scopes => "Scopes",
            Pane::Export => "Export",
            Pane::MediaBrowser => "Media Browser",
            Pane::Grade => "Color Wheels",
            Pane::Clips => "Clips",
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
    // ---- ws:registries-schema-hooks ----
    /// Panes pinned against auto-surfacing (a selection-driven reveal skips them); consumed for real
    /// by ws:layout-modes-onboarding (wave 2) - see `reveal_auto`.
    #[serde(default)]
    pub pinned: Vec<Pane>,
    // ---- ws:layout-modes-onboarding ----
    /// The maximised pane and the tree JSON `unmaximize` restores. Persisted (not skipped) so quitting
    /// while maximised still lets backtick restore the real arrangement after a restart.
    #[serde(default)]
    pub maximized: Option<(Pane, String)>,
    /// The pane whose tile the pointer was over when the tree was last drawn (`show` refreshes it
    /// every frame) - what "pane under cursor" means for MaximizePane / TogglePin.
    #[serde(skip)]
    pub hovered: Option<Pane>,
    /// Tabs currently glowing: (pane, when the glow started). Painted by `tab_ui` with an alpha that
    /// fades over `GLOW_SECS`; decayed by `ui::app::frame::tick`.
    #[serde(skip)]
    pub glow: Vec<(Pane, Instant)>,
    // ---- ws:ui-kit ----
    /// Where each docked pane was last drawn, its tab bar included (`show` refreshes it every frame) -
    /// what `ui.screenshot {pane}` crops to.
    #[serde(skip)]
    pub rects: Vec<(Pane, egui::Rect)>,
    // ---- ws:pages ----
    /// A tab just brought to the front (`+`, the Window menu, a selection): the next draw scrolls its
    /// tab bar to it, since a group with more tabs than fit would otherwise hide the tab it switched to.
    #[serde(skip)]
    pub scroll_to: Option<Pane>,
    /// Where each undocked pane's window was last: `[x, y, w, h]` in points - the outer position (what
    /// `ViewportBuilder::with_position` takes) and the inner size (`with_inner_size`). Kept after the
    /// pane docks, so undocking it again puts the window back where it was.
    #[serde(default)]
    pub popped_rects: Vec<(Pane, [f32; 4])>,
    /// Popped panes whose window is already open this session: the saved rect goes to the builder only
    /// on the frame a window opens, so egui never moves a window the user is dragging.
    #[serde(skip)]
    opened: Vec<Pane>,
    /// When a popped window last moved or resized: its rect is persisted once it has been still for
    /// `RECT_SETTLE`, not on every frame of the drag.
    #[serde(skip)]
    rect_moved: Option<Instant>,
}

/// How long a popped window must sit still before its new rect is persisted (see `Layout::rect_moved`).
const RECT_SETTLE: std::time::Duration = std::time::Duration::from_millis(500);

impl Default for Layout {
    fn default() -> Self {
        Self::default_layout()
    }
}

impl Layout {
    /// The Media page (import & organise): three full-height columns, [Library over Media Browser] ·
    /// [Source] · [Inspector]. No Timeline - it rides hidden behind Library with every other pane.
    pub fn media_layout() -> Self {
        let mut tiles = egui_tiles::Tiles::default();
        let t = &mut tiles;
        let library = Self::tabs(t, &[Pane::Library], 0);
        let browser = Self::tabs(t, &[Pane::MediaBrowser], 0);
        let left = Self::linear(t, egui_tiles::LinearDir::Vertical, &[(library, 0.5), (browser, 0.5)]);
        let source = t.insert_pane(Pane::Source);
        let inspector = t.insert_pane(Pane::Inspector);
        let row = [(left, 0.36), (source, 0.4), (inspector, 0.24)];
        let root = Self::linear(t, egui_tiles::LinearDir::Horizontal, &row);
        Self::stack_unplaced(t, library);
        Self::new(egui_tiles::Tree::new("layout", root, tiles))
    }

    /// The Cut page (fast assembly): [Library] · [Source | Preview] · [Inspector] over the Timeline.
    pub fn cut_layout() -> Self {
        Self::viewer_over_timeline(&[Pane::Library], [0.25, 0.5, 0.25])
    }

    /// The Edit page: [Library | Effects | Transitions | Gallery] · [Source | Preview] · [Inspector] over
    /// a full-width Timeline; every other pane rides hidden behind Library.
    pub fn default_layout() -> Self {
        let library = [Pane::Library, Pane::Effects, Pane::Transitions, Pane::Presets];
        Self::viewer_over_timeline(&library, [0.22, 0.5, 0.28])
    }

    /// Cut and Edit: [`library` as tabs] · [Source | Preview] · [Inspector] (`shares` of the row) over a
    /// full-width Timeline, every other pane hidden behind the first group.
    fn viewer_over_timeline(library: &[Pane], shares: [f32; 3]) -> Self {
        use egui_tiles::LinearDir::{Horizontal, Vertical};
        let mut tiles = egui_tiles::Tiles::default();
        let t = &mut tiles;
        let library = Self::tabs(t, library, 0);
        let viewer = Self::tabs(t, &[Pane::Source, Pane::Preview], 1);
        let inspector = t.insert_pane(Pane::Inspector);
        let top = Self::linear(t, Horizontal, &[(library, shares[0]), (viewer, shares[1]), (inspector, shares[2])]);
        let timeline = t.insert_pane(Pane::Timeline);
        let root = Self::linear(t, Vertical, &[(top, 0.6), (timeline, 0.4)]);
        Self::stack_unplaced(t, library);
        Self::new(egui_tiles::Tree::new("layout", root, tiles))
    }

    /// The Color page, laid out like Resolve's: [Gallery] · [Preview] · [Nodes] · [Inspector] on top, the
    /// Clips strip and a thin Timeline, then [Color Wheels] · [Curves] · [Scopes] along the bottom.
    pub fn color_layout() -> Self {
        use egui_tiles::LinearDir::{Horizontal, Vertical};
        let mut tiles = egui_tiles::Tiles::default();
        let t = &mut tiles;
        let gallery = Self::tabs(t, &[Pane::Presets], 0);
        let preview = t.insert_pane(Pane::Preview);
        let nodes = t.insert_pane(Pane::Nodes);
        let inspector = t.insert_pane(Pane::Inspector);
        let top = Self::linear(t, Horizontal, &[(gallery, 0.17), (preview, 0.43), (nodes, 0.2), (inspector, 0.2)]);
        let clips = t.insert_pane(Pane::Clips);
        let timeline = t.insert_pane(Pane::Timeline);
        let grade = t.insert_pane(Pane::Grade);
        let curves = t.insert_pane(Pane::Curves);
        let scopes = t.insert_pane(Pane::Scopes);
        let bottom = Self::linear(t, Horizontal, &[(grade, 0.5), (curves, 0.25), (scopes, 0.25)]);
        let root = Self::linear(t, Vertical, &[(top, 0.5), (clips, 0.11), (timeline, 0.1), (bottom, 0.29)]);
        Self::stack_unplaced(t, gallery);
        Self::new(egui_tiles::Tree::new("layout", root, tiles))
    }

    /// The Audio page: a small [Preview] · [Mixer | Subtitles] · [Inspector] over the Timeline.
    pub fn audio_layout() -> Self {
        use egui_tiles::LinearDir::{Horizontal, Vertical};
        let mut tiles = egui_tiles::Tiles::default();
        let t = &mut tiles;
        let preview = t.insert_pane(Pane::Preview);
        let mix = Self::tabs(t, &[Pane::Mixer, Pane::Subtitles], 0);
        let inspector = t.insert_pane(Pane::Inspector);
        let top = Self::linear(t, Horizontal, &[(preview, 0.3), (mix, 0.4), (inspector, 0.3)]);
        let timeline = t.insert_pane(Pane::Timeline);
        let root = Self::linear(t, Vertical, &[(top, 0.45), (timeline, 0.55)]);
        Self::stack_unplaced(t, mix);
        Self::new(egui_tiles::Tree::new("layout", root, tiles))
    }

    /// The Export page: [Export settings] · [Preview] · [Inspector over Jobs, the render queue] over the
    /// Timeline.
    pub fn export_layout() -> Self {
        use egui_tiles::LinearDir::{Horizontal, Vertical};
        let mut tiles = egui_tiles::Tiles::default();
        let t = &mut tiles;
        let export = Self::tabs(t, &[Pane::Export], 0);
        let preview = t.insert_pane(Pane::Preview);
        let inspector = t.insert_pane(Pane::Inspector);
        let jobs = t.insert_pane(Pane::Jobs);
        let side = Self::linear(t, Vertical, &[(inspector, 0.55), (jobs, 0.45)]);
        let top = Self::linear(t, Horizontal, &[(export, 0.3), (preview, 0.45), (side, 0.25)]);
        let timeline = t.insert_pane(Pane::Timeline);
        let root = Self::linear(t, Vertical, &[(top, 0.6), (timeline, 0.4)]);
        Self::stack_unplaced(t, export);
        Self::new(egui_tiles::Tree::new("layout", root, tiles))
    }

    /// A tab group of `panes`, `panes[active]` in front.
    fn tabs(tiles: &mut egui_tiles::Tiles<Pane>, panes: &[Pane], active: usize) -> egui_tiles::TileId {
        let ids: Vec<_> = panes.iter().map(|&p| tiles.insert_pane(p)).collect();
        let group = tiles.insert_tab_tile(ids.clone());
        if let Some(egui_tiles::Tile::Container(egui_tiles::Container::Tabs(t))) = tiles.get_mut(group) {
            t.set_active(ids[active]);
        }
        group
    }
    /// A row / column of `parts` with the given shares.
    fn linear(
        tiles: &mut egui_tiles::Tiles<Pane>,
        dir: egui_tiles::LinearDir,
        parts: &[(egui_tiles::TileId, f32)],
    ) -> egui_tiles::TileId {
        use egui_tiles::{Container, Linear, Tile};
        let mut lin = Linear::new(dir, parts.iter().map(|&(id, _)| id).collect());
        for &(id, share) in parts {
            lin.shares.set_share(id, share);
        }
        tiles.insert_new(Tile::Container(Container::Linear(lin)))
    }
    /// A layout around a tree, with an empty history.
    pub fn new(tree: egui_tiles::Tree<Pane>) -> Self {
        Self {
            tree,
            popped: Vec::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            pinned: Vec::new(),
            maximized: None,
            hovered: None,
            glow: Vec::new(),
            rects: Vec::new(),
            scroll_to: None,
            popped_rects: Vec::new(),
            opened: Vec::new(),
            rect_moved: None,
        }
    }
    /// Ensure every `Pane::ALL` member absent from `tiles` (a new variant a page builder never
    /// listed explicitly) ends up tab-stacked behind `anchor`, hidden but reachable from the Window
    /// menu / `+` - so a new Pane never needs every page builder edited, just this one call per builder.
    pub(crate) fn stack_unplaced(tiles: &mut egui_tiles::Tiles<Pane>, anchor: egui_tiles::TileId) {
        for &p in Pane::ALL {
            if tiles.find_pane(&p).is_none() {
                let id = tiles.insert_pane(p);
                if let Some(egui_tiles::Tile::Container(egui_tiles::Container::Tabs(tabs))) = tiles.get_mut(anchor) {
                    tabs.add_child(id);
                }
                tiles.set_visible(id, false);
            }
        }
    }
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
    /// None on malformed / incompatible JSON (caller falls back to the default layout). A layout saved
    /// before round 3 knows nothing about Tools / Nodes / Mixer / Markers: rather than dropping four panes
    /// into the root as loose tabs, it is rejected here and the caller resets to the new default.
    pub fn from_json(s: &str) -> Option<Self> {
        let mut l: Self = serde_json::from_str(s).ok()?;
        l.tree.root()?;
        l.drop_retired();
        let has_all = Pane::ROUND3.iter().all(|p| l.tree.tiles.find_pane(p).is_some() || l.popped.contains(p));
        has_all.then_some(l)
    }

    /// Same, but an explicitly saved profile is migrated instead of rejected: panes it predates ride
    /// hidden in one of its tab groups (like `stack_unplaced` - a profile from before Scopes / Export
    /// must not come back with two extra columns), rather than making the whole profile unloadable.
    pub fn from_json_migrating(s: &str) -> Option<Self> {
        let mut l: Self = serde_json::from_str(s).ok()?;
        l.tree.root()?;
        l.drop_retired();
        let group = l.tree.tiles.iter().find_map(|(&id, t)| {
            matches!(t, egui_tiles::Tile::Container(egui_tiles::Container::Tabs(_))).then_some(id)
        });
        match group {
            Some(g) => Self::stack_unplaced(&mut l.tree.tiles, g),
            None => {
                for &p in Pane::ALL {
                    if l.tree.tiles.find_pane(&p).is_none() && !l.popped.contains(&p) {
                        l.insert_into_root(p);
                    }
                }
            }
        }
        Some(l)
    }
    // ---- ws:viewer-surface ----
    /// A stored layout from before the viewer's tool rail still holds a Tools tab: drop it (and its
    /// popout / pin), so no page shows a pane the app no longer offers.
    fn drop_retired(&mut self) {
        if let Some(id) = self.tree.tiles.find_pane(&Pane::Tools) {
            self.tree.remove_recursively(id);
        }
        self.popped.retain(|&p| p != Pane::Tools);
        self.pinned.retain(|&p| p != Pane::Tools);
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
                self.scroll_to = Some(pane);
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
                self.scroll_to = Some(pane);
            }
            None => self.insert_into_root(pane),
        }
    }
    pub fn popout(&mut self, pane: Pane) {
        if self.popped.contains(&pane) {
            return;
        }
        // undocking out of a maximised tree would come back docked too once the stash is restored
        self.unmaximize();
        if let Some(id) = self.tree.tiles.find_pane(&pane) {
            self.tree.tiles.set_visible(id, false);
        }
        self.popped.push(pane);
    }
    pub fn dock(&mut self, pane: Pane) {
        self.popped.retain(|&p| p != pane);
        self.reveal(pane);
    }
    /// Back to `fresh` (a page's starting arrangement); the layout's own history stays.
    pub fn reset(&mut self, fresh: Layout) {
        let (undo, redo) = (std::mem::take(&mut self.undo), std::mem::take(&mut self.redo));
        *self = fresh;
        (self.undo, self.redo) = (undo, redo);
    }
    /// Record where `pane`'s window is (`[x, y, w, h]`, see `popped_rects`); true when that moved it.
    pub fn set_popped_rect(&mut self, pane: Pane, rect: [f32; 4]) -> bool {
        match self.popped_rects.iter_mut().find(|(p, _)| *p == pane) {
            Some((_, r)) if *r == rect => false,
            Some((_, r)) => {
                *r = rect;
                true
            }
            None => {
                self.popped_rects.push((pane, rect));
                true
            }
        }
    }
    // ---- ws:pages ----
    /// A tab bar's `+`: show `pane` as a tab of the tab group `group`, in front - moved there from
    /// wherever it sat hidden, or inserted when an old tree lacks it.
    pub fn add_to(&mut self, pane: Pane, group: egui_tiles::TileId) {
        self.popped.retain(|&p| p != pane);
        let id = self.tree.tiles.find_pane(&pane).unwrap_or_else(|| self.tree.tiles.insert_pane(pane));
        self.tree.tiles.set_visible(id, true);
        if self.tree.tiles.parent_of(id) != Some(group) {
            self.tree.move_tile_to_container(id, group, usize::MAX, false);
        }
        self.tree.make_active(|tid, _| tid == id);
        self.scroll_to = Some(pane);
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

    // ---- ws:layout-modes-onboarding ----
    /// What `reveal_auto(pane)` would do, without doing it - the shared decision for following the
    /// selection on or off (on switches on `Shown`, off only glows) so the two can never disagree.
    pub fn can_surface(&self, pane: Pane) -> Surfaced {
        if self.popped.contains(&pane) {
            return Surfaced::Shown;
        }
        let Some(id) = self.tree.tiles.find_pane(&pane) else { return Surfaced::Absent };
        if !self.tree.tiles.is_visible(id) {
            return Surfaced::Hidden;
        }
        if self.pinned.contains(&pane) {
            return Surfaced::Pinned;
        }
        // the active sibling tab is pinned: the user asked that group to stay put
        if let Some(parent) = self.tree.tiles.parent_of(id) {
            if let Some(egui_tiles::Container::Tabs(tabs)) = self.tree.tiles.get_container(parent) {
                let active = tabs.active.filter(|&a| a != id).and_then(|a| self.tree.tiles.get_pane(&a));
                if active.is_some_and(|p| self.pinned.contains(p)) {
                    return Surfaced::Pinned;
                }
            }
        }
        Surfaced::Shown
    }
    /// Pin-aware reveal for selection-driven surfacing: switches to the pane's tab only when
    /// `can_surface` says `Shown`; never inserts into the root (unlike `reveal`), never re-opens a pane
    /// the user hid, never moves a pane - it only changes which tab of an existing group is in front.
    pub fn reveal_auto(&mut self, pane: Pane) -> Surfaced {
        let r = self.can_surface(pane);
        if r == Surfaced::Shown {
            if let Some(id) = self.tree.tiles.find_pane(&pane) {
                self.tree.make_active(|tid, _| tid == id);
                self.scroll_to = Some(pane);
            }
        }
        r
    }
    pub fn toggle_pin(&mut self, pane: Pane) -> bool {
        match self.pinned.iter().position(|&p| p == pane) {
            Some(i) => {
                self.pinned.remove(i);
                false
            }
            None => {
                self.pinned.push(pane);
                true
            }
        }
    }
    pub fn set_pinned(&mut self, pane: Pane, on: bool) {
        self.pinned.retain(|&p| p != pane);
        if on {
            self.pinned.push(pane);
        }
    }
    /// Start (or refresh) a tab glow on `pane`.
    pub fn push_glow(&mut self, pane: Pane, now: Instant) {
        self.glow.retain(|(p, _)| *p != pane);
        self.glow.push((pane, now));
    }
    /// Show only `pane`, full-tile: the current tree JSON is stashed in `maximized` and the tree is
    /// replaced by a single-pane root (every other pane tab-stacked hidden behind it, so the stored
    /// layout still passes `from_json`'s round-3 check). egui_tiles 0.14 has no native maximise;
    /// ponytail: a stash/restore of the whole tree, revisit if egui_tiles grows a native maximise.
    pub fn maximize(&mut self, pane: Pane) {
        if self.maximized.is_some() {
            self.unmaximize();
        }
        if self.tree.tiles.find_pane(&pane).is_none() || self.popped.contains(&pane) {
            return;
        }
        let Ok(stash) = serde_json::to_string(&self.tree) else { return };
        let mut tiles = egui_tiles::Tiles::default();
        let id = tiles.insert_pane(pane);
        let root = tiles.insert_tab_tile(vec![id]);
        Self::stack_unplaced(&mut tiles, root);
        self.tree = egui_tiles::Tree::new("layout", root, tiles);
        self.maximized = Some((pane, stash));
    }
    pub fn unmaximize(&mut self) {
        if let Some((_, json)) = self.maximized.take() {
            if let Ok(tree) = serde_json::from_str(&json) {
                self.tree = tree;
            }
        }
    }
    /// Backtick semantics: maximised (any pane) -> restore; else maximise `pane`.
    pub fn toggle_maximize(&mut self, pane: Pane) {
        if self.maximized.is_some() {
            self.unmaximize();
        } else {
            self.maximize(pane);
        }
    }
}

/// How much of its linear parent a tile takes up (None when the parent is a tab bar - a tab has no share).
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
/// name that wraps makes the menu narrower, which wraps it harder - after a few frames "Editor 1" has
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

/// What the tab chrome needs from the app besides the tree itself (see `show`).
pub struct Chrome<'a> {
    /// `Settings.icon_overrides` (pane icons).
    pub icons: &'a BTreeMap<String, String>,
    /// Tab-bar fill when the editor background image shows through (else egui_tiles' default).
    pub tab_bar: Option<egui::Color32>,
    /// Cozy look: rounded tab tops, accent fill for the active tab instead of an accent outline.
    pub cozy: bool,
    // ---- ws:pages ----
    /// The sequence being edited (`project.editing`'s name): the Timeline's tab shows it beside "Main".
    pub editing: Option<String>,
    /// `Settings.panels_locked`: a tab only clicks. Unlocked (the default) it drags to re-dock.
    pub locked: bool,
}

/// What one frame of `show` changed, for the app to apply and persist.
#[derive(Default)]
pub struct Shown {
    /// The layout changed this frame - the caller persists it.
    pub changed: bool,
    /// A pane was dropped somewhere new (an undoable move).
    pub moved: bool,
    /// Icon picks from a tab's right-click: `None` = back to default, `Some("none")` = no icon.
    pub set_icon: Vec<(Pane, Option<String>)>,
    /// Actions the chrome asked for: the Timeline tab's "Main" / × = `OpenParentSequence`.
    pub actions: Vec<Action>,
}

struct Behaviour<'a> {
    draw: &'a mut dyn FnMut(&mut egui::Ui, Pane),
    chrome: &'a Chrome<'a>,
    hide: Vec<Pane>,
    pop: Vec<Pane>,
    /// Icon picks from the tab context menu: `None` = back to default, `Some("none")` = no icon.
    set_icon: Vec<(Pane, Option<String>)>,
    edited: bool,
    dropped: bool,
    // ---- ws:layout-modes-onboarding ----
    /// `Layout.pinned` (copied in): the tab paints a pin glyph after its title.
    pinned: Vec<Pane>,
    /// "Stay on this tab" toggles clicked this frame (tab context menu), applied by `show`.
    pin: Vec<Pane>,
    /// Maximise toggles (tab double-click / context menu), applied by `show`.
    maximize: Vec<Pane>,
    /// `Layout.maximized`'s pane, so the context menu reads "Restore panel" while maximised.
    maximized: Option<Pane>,
    /// Glowing tabs as (pane, alpha 0..1) for this frame - an accent underline that fades out.
    glow: Vec<(Pane, f32)>,
    /// The pane whose tile the pointer is over (set by `pane_ui`, copied to `Layout.hovered`).
    hovered: Option<Pane>,
    // ---- ws:ui-kit ----
    /// Every pane's content rect and every tab bar's rect drawn this frame (see `show`).
    panes: Vec<(Pane, egui::Rect)>,
    tab_bars: Vec<egui::Rect>,
    // ---- ws:pages ----
    /// `Layout.popped` (copied in): a popped pane is on screen, so `+` doesn't offer it.
    popped: Vec<Pane>,
    /// `+` picks: (pane, the tab group whose `+` it was), applied by `show` (`Layout::add_to`).
    add: Vec<(Pane, egui_tiles::TileId)>,
    /// Actions for the app (see `Shown::actions`).
    actions: Vec<Action>,
    /// `Layout.scroll_to`, taken: that tab scrolls its bar to itself this frame.
    scroll_to: Option<Pane>,
}

/// The panes a tab bar's `+` offers: every one not on screen (hidden, missing, and not popped out).
fn addable(tiles: &egui_tiles::Tiles<Pane>, popped: &[Pane]) -> Vec<Pane> {
    Pane::ALL
        .iter()
        .copied()
        .filter(|p| !popped.contains(p) && tiles.find_pane(p).is_none_or(|id| !tiles.is_visible(id)))
        .collect()
}

/// The "Set Icon" submenu shared by tab and toolbar-button context menus: Default / None / every glyph.
/// Returns the pick (`None` = no click yet, `Some(None)` = reset to default, `Some(Some(name))`).
pub fn icon_menu(ui: &mut egui::Ui) -> Option<Option<String>> {
    let mut pick = None;
    // ws:pages: `ui::menu` rows, so the entry lines up with the menu it sits in; the submenu scrolls
    menu::sub(ui, None, "Set Icon", |ui| {
        if menu::row(ui, None, "Default", "").clicked() {
            pick = Some(None);
        }
        if menu::row(ui, None, "None", "").clicked() {
            pick = Some(Some("none".to_string()));
        }
        ui.separator();
        for &g in Glyph::ALL {
            if menu::row(ui, Some(g), g.name(), "").clicked() {
                pick = Some(Some(g.name().to_string()));
            }
        }
    });
    pick
}

impl egui_tiles::Behavior<Pane> for Behaviour<'_> {
    fn pane_ui(&mut self, ui: &mut egui::Ui, _tile_id: egui_tiles::TileId, pane: &mut Pane) -> egui_tiles::UiResponse {
        // ---- ws:layout-modes-onboarding ----
        // "pane under cursor" for MaximizePane / TogglePin: the tile the pointer is over as it draws
        if ui.rect_contains_pointer(ui.max_rect()) {
            self.hovered = Some(*pane);
        }
        self.panes.push((*pane, ui.max_rect()));
        (self.draw)(ui, *pane);
        egui_tiles::UiResponse::None
    }
    fn tab_title_for_pane(&mut self, pane: &Pane) -> egui::WidgetText {
        pane.title().into()
    }
    /// Default tab (egui_tiles 0.14) minus the close button, plus the pane's icon before the title. The
    /// Timeline's tab is the sequence strip: "Main", and while a sequence is open its name with a ×
    /// (both lead back to Main).
    fn tab_ui(
        &mut self,
        tiles: &mut egui_tiles::Tiles<Pane>,
        ui: &mut egui::Ui,
        id: egui::Id,
        tile_id: egui_tiles::TileId,
        state: &egui_tiles::TabState,
    ) -> egui::Response {
        let pane = tiles.get_pane(&tile_id).copied();
        let glyph = pane.and_then(|p| pane_icon(self.chrome.icons, p));
        // ---- ws:layout-modes-onboarding ----
        let pinned = pane.is_some_and(|p| self.pinned.contains(&p));
        let glow = pane.and_then(|p| self.glow.iter().find(|(g, _)| *g == p).map(|(_, a)| *a));
        let pin_w = if pinned { 14.0 } else { 0.0 };
        // ---- ws:pages ----
        let timeline = pane == Some(Pane::Timeline);
        let text = if timeline { "Main".into() } else { self.tab_title_for_tile(tiles, tile_id) };
        let font_id = egui::TextStyle::Button.resolve(ui.style());
        let galley = text.into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, font_id.clone());
        let seq = self.chrome.editing.clone().filter(|_| timeline);
        let seq = seq.map(|name| ui.painter().layout_no_wrap(name, font_id, egui::Color32::PLACEHOLDER));
        let (seq_gap, close_w) = (12.0, 18.0);
        let seq_w = seq.as_ref().map_or(0.0, |g| seq_gap + g.size().x + 2.0 + close_w);
        let x_margin = self.tab_title_spacing(ui.visuals());
        let (icon_w, gap) = if glyph.is_some() { (16.0, 4.0) } else { (0.0, 0.0) };
        let width = galley.size().x + icon_w + gap + pin_w + seq_w + 2.0 * x_margin;
        let (_, tab_rect) = ui.allocate_space(egui::vec2(width, ui.available_height()));
        // locked, a tab only clicks: no drag starts, so nothing re-docks by accident
        let sense = if self.chrome.locked { egui::Sense::click() } else { egui::Sense::click_and_drag() };
        let tab_response = ui.interact(tab_rect, id, sense).on_hover_cursor(self.tab_hover_cursor_icon());
        if pane.is_some() && pane == self.scroll_to {
            ui.scroll_to_rect(tab_rect, None);
        }
        if tab_response.double_clicked() {
            self.maximize.extend(pane);
        }
        let main_end = tab_rect.left() + x_margin + icon_w + gap + galley.size().x + pin_w;
        let main_rect = egui::Rect::from_min_max(tab_rect.min, egui::pos2(main_end + seq_gap / 2.0, tab_rect.bottom()));
        let close_rect = egui::Rect::from_min_max(
            egui::pos2(tab_rect.right() - x_margin - close_w, tab_rect.top()),
            egui::pos2(tab_rect.right() - x_margin, tab_rect.bottom()),
        );
        let (mut main_hot, mut close_hot) = (false, false);
        if seq.is_some() {
            let back = menu::shortcut(Action::OpenParentSequence);
            let main = ui
                .interact(main_rect, id.with("seq_main"), egui::Sense::click())
                .on_hover_text(format!("Back to the main timeline   {back}"));
            let close =
                ui.interact(close_rect, id.with("seq_close"), egui::Sense::click()).on_hover_text("Close the sequence");
            if main.clicked() || close.clicked() {
                self.actions.push(Action::OpenParentSequence);
            }
            (main_hot, close_hot) = (main.hovered(), close.hovered());
        }
        if ui.is_rect_visible(tab_rect) && !state.is_being_dragged {
            let text_color;
            if self.chrome.cozy {
                // cozy: rounded top corners, no accent outline - the active tab IS the accent,
                // inactive tabs only light up on hover
                let accent = ui.visuals().widgets.active.bg_fill;
                let r = egui::CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 };
                if state.active {
                    ui.painter().rect_filled(tab_rect.shrink(0.5), r, accent);
                    text_color = crate::ui::tools::on_accent(accent);
                } else {
                    if tab_response.hovered() {
                        ui.painter().rect_filled(tab_rect.shrink(0.5), r, ui.visuals().widgets.hovered.weak_bg_fill);
                    }
                    text_color = self.tab_text_color(ui.visuals(), tiles, tile_id, state);
                }
            } else {
                let bg_color = self.tab_bg_color(ui.visuals(), tiles, tile_id, state);
                let stroke = self.tab_outline_stroke(ui.visuals(), tiles, tile_id, state);
                ui.painter().rect(tab_rect.shrink(0.5), 0.0, bg_color, stroke, egui::StrokeKind::Inside);
                if state.active {
                    // connect the tab with its contents
                    ui.painter().hline(
                        tab_rect.x_range(),
                        tab_rect.bottom(),
                        egui::Stroke::new(stroke.width + 1.0, bg_color),
                    );
                }
                text_color = self.tab_text_color(ui.visuals(), tiles, tile_id, state);
            }
            if let Some(g) = glyph {
                let icon_rect = egui::Rect::from_min_size(
                    egui::pos2(tab_rect.left() + x_margin, tab_rect.top()),
                    egui::vec2(icon_w, tab_rect.height()),
                );
                draw_glyph(ui.painter(), icon_rect, g, text_color);
            }
            let inner = tab_rect.shrink2(egui::vec2(x_margin, 0.0));
            let tp = egui::pos2(inner.left() + icon_w + gap, inner.center().y - galley.size().y / 2.0);
            let text_w = galley.size().x;
            // while a sequence is open "Main" is the way back: dimmed until hovered
            let main_color = if seq.is_some() && !main_hot { text_color.gamma_multiply(0.6) } else { text_color };
            ui.painter().galley(tp, galley, main_color);
            // ---- ws:layout-modes-onboarding ----
            // a tab kept in front ("Stay on this tab") wears a small pin after its title; a glowing one
            // an accent underline that fades out (the "look here" cue used instead of switching tabs)
            if pinned {
                let pin_rect = egui::Rect::from_min_size(
                    egui::pos2(tp.x + text_w + 1.0, tab_rect.top()),
                    egui::vec2(pin_w, tab_rect.height()),
                );
                draw_glyph(ui.painter(), pin_rect, Glyph::Pin, text_color);
            }
            // ---- ws:pages ----
            if let Some(g) = seq {
                let x = tp.x + text_w + pin_w + seq_gap;
                let divider = egui::Stroke::new(1.0, text_color.gamma_multiply(0.4));
                ui.painter().vline(x - seq_gap / 2.0, tab_rect.shrink(6.0).y_range(), divider);
                ui.painter().galley(egui::pos2(x, inner.center().y - g.size().y / 2.0), g, text_color);
                if close_hot {
                    let hot = ui.visuals().widgets.hovered.weak_bg_fill;
                    ui.painter().rect_filled(close_rect.shrink2(egui::vec2(1.0, 4.0)), 3.0, hot);
                }
                draw_glyph(ui.painter(), close_rect, Glyph::Cross, text_color);
            }
            if let Some(alpha) = glow {
                let accent = ui.visuals().selection.bg_fill.gamma_multiply(alpha.clamp(0.0, 1.0));
                let y = tab_rect.bottom() - 1.5;
                ui.painter().hline(tab_rect.shrink2(egui::vec2(2.0, 0.0)).x_range(), y, egui::Stroke::new(3.0, accent));
            }
        }
        self.on_tab_button(tiles, tile_id, tab_response)
    }
    /// egui_tiles defaults to `Grab` (Windows renders that as the 4-arrow move cursor) - a tab is
    /// draggable, but that's not the affordance a click-to-switch tab should advertise.
    fn tab_hover_cursor_icon(&self) -> egui::CursorIcon {
        egui::CursorIcon::Default
    }
    /// Right-click menu on a tab: Maximise, Undock, Close, "Stay on this tab" and the icon picker.
    fn on_tab_button(
        &mut self,
        tiles: &egui_tiles::Tiles<Pane>,
        tile_id: egui_tiles::TileId,
        button_response: egui::Response,
    ) -> egui::Response {
        let Some(&pane) = tiles.get_pane(&tile_id) else { return button_response };
        menu::context(&button_response, |ui| {
            // ---- ws:pages ----
            let max = if self.maximized == Some(pane) { "Restore panel" } else { "Maximise panel" };
            if menu::row(ui, Some(Glyph::Maximize), max, &menu::shortcut(Action::MaximizePane)).clicked() {
                self.maximize.push(pane);
            }
            if menu::row(ui, Some(Glyph::PopOut), "Undock", "").on_hover_text("Into a window of its own").clicked() {
                self.pop.push(pane);
            }
            let close = menu::row(ui, Some(Glyph::Cross), "Close", "").on_hover_text("The Window menu brings it back");
            if close.clicked() {
                self.hide.push(pane);
            }
            ui.separator();
            let stay = menu::check(ui, self.pinned.contains(&pane), "Stay on this tab", "")
                .on_hover_text("A selection never switches this group away from this tab");
            if stay.clicked() {
                self.pin.push(pane);
            }
            ui.separator();
            if let Some(pick) = icon_menu(ui) {
                self.set_icon.push((pane, pick));
            }
        });
        button_response
    }
    fn tab_bar_color(&self, visuals: &egui::Visuals) -> egui::Color32 {
        self.chrome.tab_bar.unwrap_or_else(|| {
            if visuals.dark_mode {
                visuals.extreme_bg_color
            } else {
                (egui::Rgba::from(visuals.panel_fill) * egui::Rgba::from_gray(0.8)).into()
            }
        })
    }
    /// ---- ws:pages ---- The tab bar's one button: `+`, a menu of the panes not on screen; a pick lands
    /// as this group's front tab.
    fn top_bar_right_ui(
        &mut self,
        tiles: &egui_tiles::Tiles<Pane>,
        ui: &mut egui::Ui,
        tile_id: egui_tiles::TileId,
        _tabs: &egui_tiles::Tabs,
        _scroll_offset: &mut f32,
    ) {
        self.tab_bars.push(ui.max_rect()); // ws:ui-kit: the whole bar - see `grab_cursor_fix`
        if self.maximized.is_some() {
            return; // the maximised tree is a stand-in: a pane added to it would vanish on restore
        }
        let hidden = addable(tiles, &self.popped);
        let r = ui.add_enabled_ui(!hidden.is_empty(), |ui| {
            menu::button(ui, "+", |ui| {
                menu::scroll(ui, |ui| {
                    for p in hidden {
                        if menu::row(ui, pane_icon(self.chrome.icons, p), p.title(), "").clicked() {
                            self.add.push((p, tile_id));
                        }
                    }
                })
            })
        });
        r.inner.response.on_hover_text("Add panel").on_disabled_hover_text("Every panel is already open");
    }
    fn simplification_options(&self) -> egui_tiles::SimplificationOptions {
        egui_tiles::SimplificationOptions { all_panes_must_have_tabs: true, ..Default::default() }
    }
    /// egui_tiles' own default (32.0) lets a split shrink a pane below its tab bar (24.0) plus one row
    /// of toolbar buttons (22.0 tall), clipping them mid-icon. Applies to every pane's min width and
    /// height, but no pane in this layout wants to go smaller than this anyway.
    fn min_size(&self) -> f32 {
        56.0
    }
    fn on_edit(&mut self, edit_action: egui_tiles::EditAction) {
        self.edited = true;
        self.dropped |= edit_action == egui_tiles::EditAction::TileDropped;
    }
    /// Nine drop squares over the hovered tile instead of egui_tiles' thin outline: the centre one tabs
    /// the pane in, the eight around it split in that direction. egui_tiles still owns the hit test, so a
    /// corner resolves to whichever of its two edges is nearer - the translucent fill is where it lands.
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

// ---- ws:ui-kit ----
/// The cursor to put back over a tab bar, if any. ponytail: egui_tiles 0.14.1 hard-codes
/// `CursorIcon::Grab` on the empty strip of every non-root tab bar (`container/tabs.rs:269-276`,
/// `Behavior` can't override it) and Windows draws Grab as the 4-arrow move cursor. `set_cursor_icon`
/// is last-write-wins, so after `tree.ui()` a Grab over a tab bar with nothing dragged goes back to
/// Default - delete this once an egui_tiles bump makes that cursor a `Behavior` hook.
fn grab_cursor_fix(
    cur: egui::CursorIcon,
    pointer: Option<egui::Pos2>,
    tab_bars: &[egui::Rect],
    dragging: bool,
) -> Option<egui::CursorIcon> {
    let over_bar = pointer.is_some_and(|p| tab_bars.iter().any(|r| r.contains(p)));
    (cur == egui::CursorIcon::Grab && over_bar && !dragging).then_some(egui::CursorIcon::Default)
}

/// Repair tab containers whose `active` no longer names one of their children - which is what leaves a
/// lone tab drawn as inactive after a rearrange, so it has to be clicked before its pane comes back.
fn activate_orphan_tabs(tree: &mut egui_tiles::Tree<Pane>) {
    let mut fix: Vec<(egui_tiles::TileId, egui_tiles::TileId)> = Vec::new();
    for (id, tile) in tree.tiles.iter() {
        if let egui_tiles::Tile::Container(egui_tiles::Container::Tabs(tabs)) = tile {
            let ok = tabs.active.is_some_and(|a| tabs.children.contains(&a));
            if !ok {
                if let Some(&first) = tabs.children.first() {
                    fix.push((*id, first));
                }
            }
        }
    }
    for (id, child) in fix {
        if let Some(egui_tiles::Tile::Container(egui_tiles::Container::Tabs(tabs))) = tree.tiles.get_mut(id) {
            tabs.set_active(child);
        }
    }
}

/// Draw the docked tree into `ui` and every popped pane in its own OS window; `draw(ui, pane)` renders
/// a pane's content. A popped window that the user closes is docked back automatically.
pub fn show(
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    layout: &mut Layout,
    chrome: &Chrome,
    draw: &mut dyn FnMut(&mut egui::Ui, Pane),
    // ---- ws:registries-schema-hooks ----
    // Polled inside each popped pane's own viewport, so Space/J/K/L work in a torn-off Preview (hotkeys
    // are otherwise only polled on the root ctx - see `App::update`).
    on_viewport: &mut dyn FnMut(&egui::Context),
) -> Shown {
    // the drop happens inside tree.ui(), so grab the "before" state while a drag is still in flight
    let dragged = layout.tree.dragged_id(ctx).map(|id| (id, share_fraction(&layout.tree, id), layout.to_json()));
    // ---- ws:layout-modes-onboarding ----
    let now = Instant::now();
    let glow: Vec<(Pane, f32)> = layout
        .glow
        .iter()
        .map(|&(p, at)| (p, 1.0 - now.duration_since(at).as_secs_f32() / GLOW_SECS))
        .filter(|&(_, a)| a > 0.0)
        .collect();
    let mut beh = Behaviour {
        draw,
        chrome,
        hide: Vec::new(),
        pop: Vec::new(),
        set_icon: Vec::new(),
        edited: false,
        dropped: false,
        pinned: layout.pinned.clone(),
        pin: Vec::new(),
        maximize: Vec::new(),
        maximized: layout.maximized.as_ref().map(|(p, _)| *p),
        glow,
        hovered: None,
        panes: Vec::new(),
        tab_bars: Vec::new(),
        popped: layout.popped.clone(),
        add: Vec::new(),
        actions: Vec::new(),
        scroll_to: layout.scroll_to.take(),
    };
    layout.tree.ui(&mut beh, ui);
    // ---- ws:pages ----
    // Locked panels never re-dock. Tabs don't sense drags then, but egui_tiles also drags a whole group
    // by its tab bar's strip - egui hands a press on a click-only tab to the draggable strip behind it
    // (`container/tabs.rs:269`, no `Behavior` hook). Drop that drag the frame it starts, before egui_tiles
    // can preview or land it.
    if chrome.locked && layout.tree.dragged_id(ctx).is_some() {
        ctx.stop_dragging();
    }
    // ---- ws:ui-kit ----
    let cursor = ctx.output(|o| o.cursor_icon);
    let pointer = ctx.pointer_hover_pos();
    if let Some(c) = grab_cursor_fix(cursor, pointer, &beh.tab_bars, ctx.dragged_id().is_some()) {
        ctx.set_cursor_icon(c);
    }
    layout.rects = std::mem::take(&mut beh.panes)
        .into_iter()
        .map(|(p, r)| {
            let bar =
                beh.tab_bars.iter().find(|b| (b.bottom() - r.top()).abs() < 2.0 && r.x_range().contains(b.center().x));
            (p, bar.map_or(r, |b| b.union(r)))
        })
        .collect();
    // ---- ws:layout-modes-onboarding ----
    layout.hovered = beh.hovered;
    let mut changed = false;
    for p in std::mem::take(&mut beh.pin) {
        layout.toggle_pin(p);
        changed = true;
    }
    for p in std::mem::take(&mut beh.maximize) {
        layout.toggle_maximize(p);
        changed = true;
    }
    let mut moved = false;
    if let (true, Some((id, fraction, before))) = (beh.dropped, dragged) {
        layout.push_undo(before);
        if let Some(f) = fraction {
            keep_share_fraction(&mut layout.tree, id, f);
        }
        // the tab you just dropped is the one you want to look at; without this it lands behind
        // whichever tab the container had active before
        layout.tree.make_active(|tid, _| tid == id);
        moved = true;
    }
    activate_orphan_tabs(&mut layout.tree);
    changed |= beh.edited;
    for p in beh.hide {
        if layout.is_visible(p) {
            layout.toggle(p);
            changed = true;
        }
    }
    for p in beh.pop {
        layout.popout(p);
        changed = true;
    }
    // ---- ws:pages ----
    for (p, group) in beh.add {
        layout.add_to(p, group);
        changed = true;
    }
    let (set_icon, actions) = (beh.set_icon, beh.actions);
    let mut to_dock: Vec<Pane> = Vec::new();
    let mut placed: Vec<(Pane, [f32; 4])> = Vec::new();
    layout.opened.retain(|p| layout.popped.contains(p));
    for pane in layout.popped.clone() {
        // a torn-off Timeline names the sequence it shows, since its tab bar stayed behind
        let title = match (&chrome.editing, pane) {
            (Some(seq), Pane::Timeline) => format!("Timeline - {seq}"),
            _ => pane.title().to_string(),
        };
        let mut builder = egui::ViewportBuilder::default().with_title(title);
        // where the user left it, on the frame the window opens only: a builder that changes while the
        // window is open is a move / resize command, and would fight the user's own drag.
        // ponytail: a rect on a monitor since unplugged opens off-screen - clamp to the monitors if that bites
        if !layout.opened.contains(&pane) {
            layout.opened.push(pane);
            builder = match layout.popped_rects.iter().find(|(p, _)| *p == pane) {
                Some(&(_, [x, y, w, h])) => builder.with_position([x, y]).with_inner_size([w, h]),
                None => builder.with_inner_size([800.0, 500.0]),
            };
        }
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of(("pane", pane)), builder, |ctx, _class| {
            let info = ctx.input(|i| i.viewport().clone());
            if info.close_requested() {
                to_dock.push(pane);
            }
            if let (Some(outer), Some(inner)) = (info.outer_rect, info.inner_rect) {
                placed.push((pane, [outer.min.x, outer.min.y, inner.width(), inner.height()]));
            }
            on_viewport(ctx);
            egui::CentralPanel::default().show(ctx, |ui| draw(ui, pane));
        });
    }
    for (pane, rect) in placed {
        if layout.set_popped_rect(pane, rect) {
            layout.rect_moved = Some(now);
        }
    }
    // persist a moved window once it is still (a drag moves it every frame)
    if let Some(at) = layout.rect_moved {
        let still = now.duration_since(at);
        if still >= RECT_SETTLE {
            layout.rect_moved = None;
            changed = true;
        } else {
            ctx.request_repaint_after(RECT_SETTLE - still);
        }
    }
    for p in to_dock {
        layout.dock(p);
        changed = true;
    }
    Shown { changed, moved, set_icon, actions }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- ws:ui-kit ----
    /// Each page opens with its tool: Cut cuts, the rest select.
    #[test]
    fn pages_open_with_their_default_tool() {
        use crate::ui::tools::Tool;
        let want = [Tool::Select, Tool::Cut, Tool::Select, Tool::Select, Tool::Select, Tool::Select];
        assert_eq!(PAGES.iter().map(|p| page_tool(p)).collect::<Vec<_>>(), want);
    }

    #[test]
    fn grab_over_a_tab_bar_becomes_default() {
        use egui::{pos2, CursorIcon as C, Rect};
        let bars = [Rect::from_min_max(pos2(0.0, 0.0), pos2(200.0, 24.0))];
        let on = Some(pos2(150.0, 10.0));
        assert_eq!(grab_cursor_fix(C::Grab, on, &bars, false), Some(C::Default), "the empty strip");
        assert_eq!(grab_cursor_fix(C::Grab, on, &bars, true), None, "leave a drag alone");
        assert_eq!(grab_cursor_fix(C::Grab, Some(pos2(150.0, 40.0)), &bars, false), None, "a pane's own Grab");
        assert_eq!(grab_cursor_fix(C::PointingHand, on, &bars, false), None, "only Grab is replaced");
        assert_eq!(grab_cursor_fix(C::Grab, None, &bars, false), None);
    }

    /// Every page's default holds every pane (the ones it doesn't lay out ride hidden via
    /// `stack_unplaced`), shows exactly the panes its design names, never shows Tools (the viewer's
    /// tool rail replaces it), and survives `from_json`'s round-3 check.
    #[test]
    fn every_page_contains_every_pane() {
        use Pane::*;
        let shown: [(&str, &[Pane]); 6] = [
            ("Media", &[Library, MediaBrowser, Source, Inspector]),
            ("Cut", &[Library, Source, Preview, Inspector, Timeline]),
            ("Edit", &[Library, Effects, Transitions, Presets, Source, Preview, Inspector, Timeline]),
            ("Color", &[Presets, Preview, Inspector, Timeline, Nodes, Curves, Scopes, Grade, Clips]),
            ("Audio", &[Preview, Mixer, Subtitles, Inspector, Timeline]),
            ("Export", &[Export, Preview, Inspector, Jobs, Timeline]),
        ];
        assert_eq!(PAGES, shown.map(|(p, _)| p), "Alt+1..6 map onto the six pages in Resolve's order");
        for (name, visible) in shown {
            let l = page_layout(name).unwrap_or_else(|| panic!("no builder for page {name}"))();
            for &p in Pane::ALL {
                assert!(l.tree.tiles.find_pane(&p).is_some(), "{p:?} missing from the {name} page");
                assert_eq!(l.is_visible(p), visible.contains(&p), "{p:?} visibility on the {name} page");
            }
            assert!(l.tree.tiles.find_pane(&Tools).is_none(), "Tools is on no page");
            assert!(Layout::from_json(&l.to_json()).is_some(), "{name} does not round-trip");
        }
        assert!(page_layout("Nope").is_none());
        // Media: Library over the Media Browser, half each, in the left column
        let l = Layout::media_layout();
        let col = |p| l.tree.tiles.parent_of(l.tree.tiles.parent_of(l.tree.tiles.find_pane(&p).unwrap()).unwrap());
        assert_eq!(col(Library), col(MediaBrowser), "Library and Media Browser share the left column");
        // Edit: Preview is the front tab of [Source | Preview]; the hidden panes sit behind Library
        let l = Layout::default_layout();
        assert!(in_front(&l, Preview) && !in_front(&l, Source) && in_front(&l, Library));
        let lib = l.tree.tiles.parent_of(l.tree.tiles.find_pane(&Library).unwrap()).unwrap();
        for p in [
            Subtitles, Markers, AutoCut, Planner, Moodboard, History, Tracking, Jobs, Mixer, Curves, Nodes, Scopes,
            Export,
        ] {
            let id = l.tree.tiles.find_pane(&p).unwrap();
            assert_eq!(l.tree.tiles.parent_of(id), Some(lib), "{p:?} is not tabbed behind Library on Edit");
        }
        assert!(in_front(&Layout::color_layout(), Nodes) && in_front(&Layout::audio_layout(), Mixer));
        assert!(in_front(&Layout::cut_layout(), Preview) && !in_front(&Layout::cut_layout(), Source));
    }

    /// The Inspector is the top-right tile of every page's default: walk the tree from the root taking
    /// the top-most child of a column, the right-most child of a row and the front tab of a group - as
    /// built, and as drawn (the first draw wraps every lone pane in a tab group of its own).
    #[test]
    fn inspector_is_top_right_on_every_page() {
        fn top_right(l: &Layout) -> Option<Pane> {
            use egui_tiles::{Container, LinearDir, Tile};
            let tiles = &l.tree.tiles;
            let mut id = l.tree.root()?;
            loop {
                id = match tiles.get(id)? {
                    Tile::Pane(p) => return Some(*p),
                    Tile::Container(Container::Linear(lin)) => {
                        let mut shown = lin.children.iter().copied().filter(|&k| tiles.is_visible(k));
                        if lin.dir == LinearDir::Horizontal {
                            shown.last()?
                        } else {
                            shown.next()?
                        }
                    }
                    Tile::Container(Container::Tabs(tabs)) => tabs.active?,
                    Tile::Container(Container::Grid(_)) => return None,
                };
            }
        }
        for &name in PAGES {
            let mut l = page_layout(name).unwrap()();
            assert_eq!(top_right(&l), Some(Pane::Inspector), "{name} as built");
            l.tree
                .simplify(&egui_tiles::SimplificationOptions { all_panes_must_have_tabs: true, ..Default::default() });
            assert_eq!(top_right(&l), Some(Pane::Inspector), "{name} as drawn");
        }
    }

    /// Tools left `ROUND3` with the pages (and `Pane::ALL` with the tool rail); Source / Jobs / Scopes /
    /// Export were never in it.
    #[test]
    fn pane_source_not_in_round3() {
        assert_eq!(Pane::ROUND3, [Pane::Nodes, Pane::Mixer, Pane::Markers]);
        assert!(!Pane::ALL.contains(&Pane::Tools), "the viewer's tool rail replaced the Tools pane");
        for p in [Pane::Source, Pane::Jobs, Pane::Scopes, Pane::Export, Pane::MediaBrowser, Pane::Grade, Pane::Clips] {
            assert!(Pane::ALL.contains(&p), "{p:?} is still a pane");
            assert!(!Pane::ROUND3.contains(&p), "{p:?} must not make a stored layout unloadable");
        }
    }

    // ---- ws:viewer-surface ----
    /// A layout saved while the Tools pane existed loads without it - as the auto-restored layout, as a
    /// profile, docked (even as the front tab) or popped out - and the rest survives.
    #[test]
    fn a_stored_tools_tab_is_dropped_on_load() {
        let mut old = Layout::default_layout();
        old.add_to(Pane::Tools, old.tree.tiles.parent_of(old.tree.tiles.find_pane(&Pane::Preview).unwrap()).unwrap());
        assert!(old.is_visible(Pane::Tools) && in_front(&old, Pane::Tools), "the old layout shows it");
        for l in [Layout::from_json(&old.to_json()).unwrap(), Layout::from_json_migrating(&old.to_json()).unwrap()] {
            assert!(l.tree.tiles.find_pane(&Pane::Tools).is_none(), "the Tools tab is gone");
            assert!(l.is_visible(Pane::Preview) && l.is_visible(Pane::Source), "its tab group survives");
        }
        let mut popped = Layout::default_layout();
        popped.popout(Pane::Tools);
        assert!(!Layout::from_json(&popped.to_json()).unwrap().popped.contains(&Pane::Tools), "and its popout");
    }

    /// The Edit page: a top row of three columns over a full-width Timeline, none opening unusably small.
    #[test]
    fn default_layout_arrangement() {
        let l = Layout::default_layout();
        let panes = |id: egui_tiles::TileId| -> Vec<Pane> {
            match l.tree.tiles.get(id) {
                Some(egui_tiles::Tile::Pane(p)) => vec![*p],
                Some(egui_tiles::Tile::Container(c)) => c
                    .children()
                    .filter(|&&k| l.tree.tiles.is_visible(k))
                    .filter_map(|k| l.tree.tiles.get_pane(k))
                    .copied()
                    .collect(),
                None => Vec::new(),
            }
        };
        let root = l.tree.root().unwrap();
        let Some(egui_tiles::Container::Linear(rows)) = l.tree.tiles.get_container(root) else { panic!("no rows") };
        assert_eq!(rows.dir, egui_tiles::LinearDir::Vertical);
        let (top, bottom) = (rows.children[0], rows.children[1]);
        assert!((share_fraction(&l.tree, top).unwrap() - 0.6).abs() < 1e-4);
        assert_eq!(panes(bottom), vec![Pane::Timeline], "the Timeline is the whole bottom row");
        let Some(egui_tiles::Container::Linear(cols)) = l.tree.tiles.get_container(top) else { panic!("no top row") };
        assert_eq!(cols.dir, egui_tiles::LinearDir::Horizontal);
        let got: Vec<(Vec<Pane>, f32)> =
            cols.children.iter().map(|&c| (panes(c), share_fraction(&l.tree, c).unwrap())).collect();
        let want = [
            (vec![Pane::Library, Pane::Effects, Pane::Transitions, Pane::Presets], 0.22),
            (vec![Pane::Source, Pane::Preview], 0.5),
            (vec![Pane::Inspector], 0.28),
        ];
        for ((g, share), (w, want_share)) in got.iter().zip(want) {
            assert_eq!(*g, w);
            assert!((share - want_share).abs() < 1e-4, "{w:?} opens at {share}");
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
        for &p in Pane::ALL {
            assert!(migrated.tree.tiles.find_pane(&p).is_some(), "{p:?} missing after the migration");
        }
        assert!(migrated.is_visible(Pane::Timeline), "the panes it did have are untouched");
        // ws:pages: ...and the ones it predates ride hidden in a tab group, not as new columns
        assert!(!migrated.is_visible(Pane::Mixer) && !migrated.is_visible(Pane::Nodes));
        let root = migrated.tree.root().unwrap();
        assert_eq!(migrated.tree.tiles.get_container(root).unwrap().children().count(), 2, "no new columns");
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
        l.toggle(Pane::Inspector);
        l.popout(Pane::Library);
        let json = l.to_json();
        let r = Layout::from_json(&json).expect("roundtrip");
        assert_eq!(r.popped, vec![Pane::Library]);
        assert!(!r.is_visible(Pane::Inspector));
        assert!(r.is_visible(Pane::Timeline));
        assert!(Layout::from_json("{").is_none());
        assert!(Layout::from_json("{}").is_none());
    }

    /// Moving a pane keeps the slice of its parent it had (not the even split a fresh container hands
    /// out), and the move is undoable / redoable on the layout's own stack.
    #[test]
    fn move_keeps_its_share_and_is_undoable() {
        let mut l = Layout::default_layout();
        let inspector = l.tree.tiles.find_pane(&Pane::Inspector).unwrap();
        let row = l.tree.tiles.parent_of(inspector).unwrap();
        let was = share_fraction(&l.tree, inspector).unwrap();
        assert!((was - 0.28).abs() < 1e-4, "the Inspector owns 28 % of the top row, got {was}");

        // move it into the root column, exactly what a drop on a horizontal/vertical edge ends up doing
        let root = l.tree.root().unwrap();
        let snapshot = l.to_json();
        l.tree.move_tile_to_container(inspector, root, 2, false);
        assert!((share_fraction(&l.tree, inspector).unwrap() - 0.5).abs() < 1e-4, "egui_tiles splits evenly");
        keep_share_fraction(&mut l.tree, inspector, was);
        assert!((share_fraction(&l.tree, inspector).unwrap() - was).abs() < 1e-4, "the recorded fraction is back");
        // and the two rows it joined keep their proportions to each other (0.6 : 0.4)
        let timeline = l.tree.tiles.find_pane(&Pane::Timeline).unwrap();
        let ratio = share_fraction(&l.tree, row).unwrap() / share_fraction(&l.tree, timeline).unwrap();
        assert!((ratio - 0.6 / 0.4).abs() < 1e-3, "the rest of the column was redistributed: {ratio}");

        l.push_undo(snapshot);
        assert!(l.undo(), "the move undoes");
        assert_eq!(l.tree.tiles.parent_of(l.tree.tiles.find_pane(&Pane::Inspector).unwrap()), Some(row));
        assert!(l.redo(), "and redoes");
        let inspector = l.tree.tiles.find_pane(&Pane::Inspector).unwrap();
        assert_eq!(l.tree.tiles.parent_of(inspector), Some(root));
        assert!((share_fraction(&l.tree, inspector).unwrap() - was).abs() < 1e-4, "the fraction survives the trip");
        assert!(!l.redo(), "nothing left to redo");
    }

    /// A menu is as wide as what it drew last frame, so feeding that width back in is what collapsed
    /// "Editor 1" into two lines: the entry must keep its width and stay one row tall.
    #[test]
    fn profile_name_stays_on_one_line() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts()); // size-diet: no default_fonts feature anymore
                                                   // a menu ui: top-down justified, as wide as the last frame's content
        let frame = |ctx: &egui::Context, w: f32, add: &mut dyn FnMut(&mut egui::Ui) -> f32| {
            let (mut inner, mut width) = (0.0, w);
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let rect = egui::Rect::from_min_size(ui.max_rect().min, egui::vec2(w, 200.0));
                    let menu = egui::Layout::top_down_justified(egui::Align::Min);
                    let b = egui::UiBuilder::new().max_rect(rect).layout(menu);
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

    // ---- ws:layout-modes-onboarding ----

    /// Is `pane` the active tab of its group (or a lone tile, which counts as "in front")?
    /// `to_json` as a Value with `tiles.invisible` sorted: egui_tiles stores it as a HashSet, so its
    /// serialised order varies once more than one tile is hidden.
    fn layout_value(l: &Layout) -> serde_json::Value {
        let mut v: serde_json::Value = serde_json::from_str(&l.to_json()).unwrap();
        if let Some(inv) = v.pointer_mut("/tree/tiles/invisible").and_then(|i| i.as_array_mut()) {
            inv.sort_by_key(|x| x.as_u64());
        }
        v
    }

    fn in_front(l: &Layout, pane: Pane) -> bool {
        let id = l.tree.tiles.find_pane(&pane).unwrap();
        match l.tree.tiles.parent_of(id).and_then(|p| l.tree.tiles.get_container(p)) {
            Some(egui_tiles::Container::Tabs(tabs)) => tabs.active == Some(id),
            _ => true,
        }
    }

    /// A pinned active tab blocks selection-driven switching in its group (the sibling glows instead -
    /// see `ui::app::frame`), a pinned pane is never switched to, a hidden pane is never re-opened, and
    /// a pane missing from the tree is reported rather than inserted.
    #[test]
    fn reveal_auto_respects_pinned_sibling() {
        let mut l = Layout::audio_layout();
        // Mixer | Subtitles share a group on the Audio page, Mixer in front
        assert!(in_front(&l, Pane::Mixer));
        assert_eq!(l.reveal_auto(Pane::Subtitles), Surfaced::Shown);
        assert!(in_front(&l, Pane::Subtitles) && !in_front(&l, Pane::Mixer));
        assert_eq!(l.reveal_auto(Pane::Subtitles), Surfaced::Shown, "already in front is still Shown");

        assert!(l.toggle_pin(Pane::Subtitles));
        assert_eq!(l.reveal_auto(Pane::Mixer), Surfaced::Pinned);
        assert!(in_front(&l, Pane::Subtitles), "the pinned tab stays in front");
        assert_eq!(l.reveal_auto(Pane::Subtitles), Surfaced::Pinned, "a pinned pane is never auto-switched to");
        // a pin in one group does not affect another
        assert_eq!(l.reveal_auto(Pane::Inspector), Surfaced::Shown);
        assert!(!l.toggle_pin(Pane::Subtitles));
        assert_eq!(l.reveal_auto(Pane::Mixer), Surfaced::Shown);

        // closed from its tab: auto-surface must not re-open it (only the Window menu / `+` do)
        l.toggle(Pane::Preview);
        assert!(!l.is_visible(Pane::Preview));
        assert_eq!(l.reveal_auto(Pane::Preview), Surfaced::Hidden);
        assert!(!l.is_visible(Pane::Preview));
        assert_eq!(l.reveal_auto(Pane::Curves), Surfaced::Hidden, "stacked hidden by the page itself");

        // absent from the tree: reported, nothing inserted (unlike `reveal`)
        let id = l.tree.tiles.find_pane(&Pane::Planner).unwrap();
        l.tree.tiles.remove(id);
        let n = l.tree.tiles.iter().count();
        assert_eq!(l.reveal_auto(Pane::Planner), Surfaced::Absent);
        assert_eq!(l.tree.tiles.iter().count(), n);
        assert!(l.tree.tiles.find_pane(&Pane::Planner).is_none());

        // popped panes count as shown
        l.popout(Pane::Markers);
        assert_eq!(l.reveal_auto(Pane::Markers), Surfaced::Shown);
    }

    /// Maximise stashes the tree, shows only that pane (everything else present but hidden, so the
    /// stored layout still loads), and unmaximise restores the exact arrangement. Equality is
    /// structural (`serde_json::Value`): egui_tiles serialises its tile map from a HashMap, so two
    /// equal trees need not be byte-identical.
    #[test]
    fn maximize_round_trips_tree() {
        let mut l = Layout::default_layout();
        l.toggle_pin(Pane::Inspector);
        let before: serde_json::Value = layout_value(&l);
        l.maximize(Pane::Effects);
        assert_eq!(l.maximized.as_ref().map(|(p, _)| *p), Some(Pane::Effects));
        assert!(l.is_visible(Pane::Effects) && in_front(&l, Pane::Effects));
        for p in [Pane::Timeline, Pane::Preview, Pane::Library] {
            assert!(!l.is_visible(p), "{p:?} still visible while Effects is maximised");
            assert!(l.tree.tiles.find_pane(&p).is_some(), "{p:?} dropped from the maximised tree");
        }
        assert!(Layout::from_json(&l.to_json()).is_some(), "a maximised layout must still load after a restart");
        let during: serde_json::Value = layout_value(&l);
        assert_ne!(during, before);
        l.unmaximize();
        assert!(l.maximized.is_none());
        let after: serde_json::Value = layout_value(&l);
        assert_eq!(after, before, "unmaximise must restore the pre-maximise tree");
        assert!(l.is_visible(Pane::Timeline) && in_front(&l, Pane::Library));
        assert_eq!(l.pinned, vec![Pane::Inspector], "pins survive the trip");
        // backtick semantics: maximised -> restore, else maximise; a second pane swaps cleanly
        l.toggle_maximize(Pane::Curves);
        assert!(in_front(&l, Pane::Curves) && !l.is_visible(Pane::Timeline));
        l.maximize(Pane::Library);
        assert_eq!(l.maximized.as_ref().map(|(p, _)| *p), Some(Pane::Library));
        l.toggle_maximize(Pane::Library);
        assert!(l.maximized.is_none() && l.is_visible(Pane::Timeline));
        let restored: serde_json::Value = layout_value(&l);
        assert_eq!(restored, before);
        // unmaximise with nothing maximised is a no-op
        l.unmaximize();
        assert_eq!(layout_value(&l), before);
    }

    #[test]
    fn pinned_survives_json_roundtrip() {
        let mut l = Layout::default_layout();
        l.set_pinned(Pane::Inspector, true);
        l.set_pinned(Pane::Inspector, true); // idempotent, no duplicate entry
        assert_eq!(l.pinned, vec![Pane::Inspector]);
        let r = Layout::from_json(&l.to_json()).expect("roundtrip");
        assert_eq!(r.pinned, vec![Pane::Inspector]);
        assert!(r.maximized.is_none());
        // a stored maximised state comes back too, and unmaximises to the stashed tree
        l.maximize(Pane::Mixer);
        let mut r = Layout::from_json(&l.to_json()).expect("roundtrip while maximised");
        assert_eq!(r.maximized.as_ref().map(|(p, _)| *p), Some(Pane::Mixer));
        r.unmaximize();
        assert!(r.is_visible(Pane::Timeline));
        // transient state is not persisted
        assert!(r.glow.is_empty() && r.hovered.is_none());
        l.set_pinned(Pane::Inspector, false);
        assert!(l.pinned.is_empty());
    }

    // ---- ws:pages ----

    /// A tab bar's `+` offers exactly the panes not on screen, and a pick lands in THAT group as its
    /// front tab - moved from where it sat hidden, or inserted when the tree lacks it.
    #[test]
    fn plus_adds_a_hidden_pane_as_the_active_tab() {
        let mut l = Layout::default_layout();
        let offered = addable(&l.tree.tiles, &l.popped);
        assert!(offered.contains(&Pane::Mixer) && !offered.contains(&Pane::Tools));
        assert!(!offered.contains(&Pane::Timeline) && !offered.contains(&Pane::Source), "on screen already");
        // the first draw wraps every lone pane in a tab group of its own (`all_panes_must_have_tabs`)
        l.tree.simplify(&egui_tiles::SimplificationOptions { all_panes_must_have_tabs: true, ..Default::default() });
        let timeline = l.tree.tiles.find_pane(&Pane::Timeline).unwrap();
        let group = l.tree.tiles.parent_of(timeline).unwrap();
        l.add_to(Pane::Mixer, group);
        let mixer = l.tree.tiles.find_pane(&Pane::Mixer).unwrap();
        assert_eq!(l.tree.tiles.parent_of(mixer), Some(group), "moved out from behind Library");
        assert!(l.is_visible(Pane::Mixer) && in_front(&l, Pane::Mixer) && !in_front(&l, Pane::Timeline));
        assert!(!addable(&l.tree.tiles, &l.popped).contains(&Pane::Mixer));
        // a popped pane is on screen: not offered; picked anyway, it docks here
        l.popout(Pane::Inspector);
        assert!(!addable(&l.tree.tiles, &l.popped).contains(&Pane::Inspector));
        l.add_to(Pane::Inspector, group);
        assert!(l.popped.is_empty() && in_front(&l, Pane::Inspector));
        // missing from an old tree: inserted into the group
        let id = l.tree.tiles.find_pane(&Pane::Planner).unwrap();
        l.tree.tiles.remove(id);
        l.add_to(Pane::Planner, group);
        let planner = l.tree.tiles.find_pane(&Pane::Planner).unwrap();
        assert_eq!(l.tree.tiles.parent_of(planner), Some(group));
    }

    /// One frame of `show` in a bare 1200x800 window.
    fn frame(ctx: &egui::Context, l: &mut Layout, chrome: &Chrome, events: Vec<egui::Event>, t: f64) -> Shown {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0))),
            time: Some(t),
            events,
            ..Default::default()
        };
        let mut out = Shown::default();
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| out = show(ctx, ui, l, chrome, &mut |_, _| {}, &mut |_| {}));
        });
        out
    }

    fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }
    }

    /// The Timeline's tab is the sequence strip: with a sequence open, both "Main" and the × ask for
    /// `OpenParentSequence`; with none open there is nothing to click back to.
    #[test]
    fn sequence_tab_click_emits_open_parent() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let icons = BTreeMap::new();
        let mut chrome =
            Chrome { icons: &icons, tab_bar: None, cozy: true, editing: Some("Sequence 1".into()), locked: false };
        let mut l = Layout::default_layout();
        let mut t = 0.0;
        for _ in 0..2 {
            frame(&ctx, &mut l, &chrome, vec![], t);
            t += 0.1;
        }
        let tab = l.tree.tiles.find_pane(&Pane::Timeline).unwrap().egui_id(l.tree.id());
        for part in ["seq_close", "seq_main"] {
            let at = ctx.read_response(tab.with(part)).unwrap_or_else(|| panic!("no {part} hit area")).rect.center();
            let mut got = Vec::new();
            for ev in [egui::Event::PointerMoved(at), button(at, true), button(at, false)] {
                got.extend(frame(&ctx, &mut l, &chrome, vec![ev], t).actions);
                t += 0.1;
            }
            assert_eq!(got, vec![Action::OpenParentSequence], "{part}");
            t += 1.0; // two clicks this close together would read as a double-click (maximise)
        }
        assert!(l.maximized.is_none());
        chrome.editing = None;
        frame(&ctx, &mut l, &chrome, vec![], t);
        frame(&ctx, &mut l, &chrome, vec![], t + 0.1);
        assert!(ctx.read_response(tab.with("seq_close")).is_none(), "no sequence open: just \"Main\"");
    }

    /// Unlocked (the default), dragging a tab onto another tile re-docks it there as one undoable move;
    /// locked, the same gesture drags nothing - neither the tab nor, through the tab bar behind it, its
    /// whole group - and the tree is untouched.
    #[test]
    fn locked_tabs_do_not_drag_unlocked_do() {
        for locked in [false, true] {
            let ctx = egui::Context::default();
            ctx.set_fonts(crate::theme::test_fonts());
            let icons = BTreeMap::new();
            let chrome = Chrome { icons: &icons, tab_bar: None, cozy: true, editing: None, locked };
            let mut l = Layout::default_layout();
            frame(&ctx, &mut l, &chrome, vec![], 0.0);
            frame(&ctx, &mut l, &chrome, vec![], 0.1);
            let before = layout_value(&l);
            let library_group = |l: &Layout| l.tree.tiles.parent_of(l.tree.tiles.find_pane(&Pane::Library).unwrap());
            let effects = l.tree.tiles.find_pane(&Pane::Effects).unwrap();
            assert_eq!(l.tree.tiles.parent_of(effects), library_group(&l));
            let at = ctx.read_response(effects.egui_id(l.tree.id())).expect("the Effects tab").rect.center();
            // down into the middle of the Timeline (the bottom 40 % of the 800 px window)
            let to = |i: i32| at + egui::vec2(0.0, 120.0 * i as f32);
            let mut t = 0.2;
            let (mut dragged, mut moved) = (false, false);
            for ev in [egui::Event::PointerMoved(at), button(at, true)]
                .into_iter()
                .chain((1..6).map(|i| egui::Event::PointerMoved(to(i))))
                .chain([button(to(5), false)])
            {
                moved |= frame(&ctx, &mut l, &chrome, vec![ev], t).moved;
                // (egui_tiles' own `dragged_id` also reports a drag stopped this very pass - ask egui)
                dragged |= ctx.dragged_id().is_some();
                t += 0.05;
            }
            assert_eq!(dragged, !locked, "locked={locked}: a tab drag");
            if locked {
                assert!(!moved);
                assert_eq!(layout_value(&l), before, "a locked layout never re-docks");
            } else {
                let effects = l.tree.tiles.find_pane(&Pane::Effects).unwrap();
                assert_ne!(l.tree.tiles.parent_of(effects), library_group(&l), "Effects left Library's group");
                assert!(moved && l.undo(), "the re-dock is one undoable move");
                assert_eq!(layout_value(&l), before, "and undoing it puts the tab back");
            }
        }
    }

    /// Where an undocked window sat survives a save / load (and a dock + undock: the rect is kept), and
    /// setting the same rect again reports no move, so an unmoved window never re-persists the layout.
    #[test]
    fn popped_rects_round_trip() {
        let mut l = Layout::default_layout();
        l.popout(Pane::Mixer);
        assert!(l.set_popped_rect(Pane::Mixer, [120.0, 80.0, 640.0, 360.0]));
        assert!(!l.set_popped_rect(Pane::Mixer, [120.0, 80.0, 640.0, 360.0]), "unchanged");
        assert!(l.set_popped_rect(Pane::Mixer, [130.0, 80.0, 640.0, 360.0]), "moved");
        l.dock(Pane::Mixer);
        let r = Layout::from_json(&l.to_json()).expect("roundtrip");
        assert_eq!(r.popped_rects, vec![(Pane::Mixer, [130.0, 80.0, 640.0, 360.0])]);
        assert!(r.popped.is_empty());
        // a layout stored before this field existed still loads, with no rects
        let mut v: serde_json::Value = serde_json::from_str(&l.to_json()).unwrap();
        v.as_object_mut().unwrap().remove("popped_rects");
        assert!(Layout::from_json(&v.to_string()).unwrap().popped_rects.is_empty());
    }
}
