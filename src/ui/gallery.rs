//! ---- ws:inspector-gallery ----
//! The Gallery pane (`Pane::Presets`'s drawer): tabs of click-to-apply, drag-to-timeline cards - Animate,
//! Looks, LUTs, Captions, Speed Ramps, Effects, Transitions, Templates, Titles, then the online library's
//! HTML / Icons / Skills. Effects and Transitions each show the built-in items and the library's shader
//! items (installed or online, cloud icon = not installed yet) in ONE grid under one search box; they
//! replaced the separate Effects and Transitions panes.
//! Looks/LUTs get real GPU thumbnails (`ui::app::thumbs::build_gallery_thumbnails`, read here through a
//! thread-local cache - same convention `effects_ui.rs`'s own `THUMBS` uses, since `App` isn't reachable
//! from this sibling module); Captions get a painted colour swatch (cheap, no GPU); Speed
//! Ramps/Templates draw a plain name tile. `// ponytail:` - a fancier preview for those is a pure visual
//! upgrade, not a functional gap (`gallery.list`/`gallery.apply` already cover every tab).
//!
//! Looks reuse color-engine's `builtin_looks()`/`apply_look()` unchanged (never redefined here) and are
//! filtered to `!EffectPreset::is_graph()` - a saved node-graph preset is not a "Look" card. Applying any
//! card is one undo (`App::run_tool_undoable("gallery.apply", …)`, called by the caller - this file only
//! returns intent via `GalleryResponse`, it never touches `Project` itself). Hovering a card >=150ms sets
//! `App.alt_render` to `AltRequest::Gallery(tab, name)` (canvas-handles-monitor's mechanism, reserved for
//! exactly this) via the same `hover` field, not a new App field.
//!
//! Every card draws through `card`, which claims its grid slot first (so a wrapping row wraps at any pane
//! width) and is a drag source carrying a `DragPayload` (a skill is the one card that is not).

use crate::model::{Clip, Id};
use crate::settings::Settings;
use crate::theme::Palette;
use crate::ui::app::thumbs::ThumbSource;
use crate::ui::gallery_filter::{self as gf, Filters, Size, Sort, Src};
use crate::ui::menu;
use crate::ui::transitions_ui;
use eframe::egui::{self, Color32, Rect, Response, Sense, Stroke, StrokeKind};
use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    static THUMBS: RefCell<HashMap<ThumbSource, (egui::TextureId, [u32; 2])>> = RefCell::new(HashMap::new());
}

/// The app hands a rendered Gallery card thumbnail here (`build_gallery_thumbnails`).
pub(crate) fn set_thumbnail(key: ThumbSource, tex: egui::TextureId, size: [u32; 2]) {
    THUMBS.with(|t| t.borrow_mut().insert(key, (tex, size)));
}
pub(crate) fn has_thumbnail(key: &ThumbSource) -> bool {
    THUMBS.with(|t| t.borrow().contains_key(key))
}
fn thumbnail(key: &ThumbSource) -> Option<(egui::TextureId, [u32; 2])> {
    THUMBS.with(|t| t.borrow().get(key).copied())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GalleryTab {
    #[default]
    Looks,
    Luts,
    Captions,
    SpeedRamps,
    /// The built-in effects AND the installed / online library's shader effects, one search, one grid
    /// (this tab replaced the Effects pane).
    Effects,
    /// The built-in transitions AND the library's shader transitions (replaced the Transitions pane).
    Transitions,
    Templates,
    // ---- ws:text-titles ----
    /// Curated title/text/shape/adjustment templates: `presets::builtin_titles()` + `Settings.templates`
    /// filtered to `is_text_template`. Distinct from the generic `Templates` tab (any saved template,
    /// placed with no Customize step) - a Titles card always shows a Customize panel for its exposed
    /// fields right after Place.
    Titles,
    // ---- ws:keyframe-blocks ----
    /// Keyframe blocks (`model::builtin_blocks()` + `Settings.key_blocks`): click = apply at the playhead
    /// to the selection (chaining after a block the playhead is inside), drag onto a clip = apply there.
    Animate,
    /// Your own sound effects: audio files in the gallery's `sounds` folder.
    Sounds,
}

impl GalleryTab {
    pub const ALL: [GalleryTab; 10] = [
        Self::Animate,
        Self::Looks,
        Self::Luts,
        Self::Captions,
        Self::SpeedRamps,
        Self::Effects,
        Self::Transitions,
        Self::Templates,
        Self::Titles,
        Self::Sounds,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Looks => "Looks",
            Self::Luts => "Luts",
            Self::Captions => "Captions",
            Self::SpeedRamps => "SpeedRamps",
            Self::Effects => "Effects",
            Self::Transitions => "Transitions",
            Self::Templates => "Templates",
            Self::Titles => "Titles",
            Self::Animate => "Animate",
            Self::Sounds => "Sounds",
        }
    }
    pub fn from_name(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.name().eq_ignore_ascii_case(s))
    }
    fn label(self) -> &'static str {
        match self {
            Self::SpeedRamps => "Speed", // short, so every tab fits a ~350 pt pane
            other => other.name(),
        }
    }
}

// ---- ws:online-library ----
/// Library tabs after the Gallery's own (LUTs from the library show in the Luts tab).
/// (Effects and Transitions are Gallery tabs of their own, each showing its library items too.)
pub const LIB_TABS: [(crate::engine::library::Kind, &str); 3] = {
    use crate::engine::library::Kind;
    [(Kind::Html, "HTML"), (Kind::Icon, "Icons"), (Kind::Skill, "Skills")]
};

/// One library cell, prepared by `ui::app::library_ctl` each frame.
pub struct LibCell {
    pub id: String,
    pub kind: crate::engine::library::Kind,
    pub name: String,
    pub desc: String,
    pub installed: bool,
    /// Download progress while downloading.
    pub busy: Option<f32>,
    pub skill: bool,
    /// The catalogue's tags (search and the category filter look in them).
    pub tags: Vec<String>,
    /// An item of your own (saved, not from the catalogue).
    pub yours: bool,
}

/// What the library part of the Gallery draws. `thumb` is only called for cells on screen.
pub struct LibView<'a> {
    pub cells: Vec<LibCell>,
    pub local: bool,
    pub note: String,
    pub loading: bool,
    /// The online library is switched on (`Settings.library_online`, or a folder): until then the cells are
    /// only what is installed, and nothing has been fetched.
    pub connected: bool,
    pub thumb: &'a mut dyn FnMut(&str) -> Option<(egui::TextureId, [u32; 2])>,
}

/// A library click / menu choice, handled by `ui::app::gallery_ctl`.
#[derive(Clone, Debug, PartialEq)]
pub enum LibAct {
    Apply(String),
    /// A library transition applied at an edge of the selection: (id, `transitions_ui::Where` name).
    ApplyAt(String, &'static str),
    Install(String),
    Remove(String),
    Reveal(String),
    View(String),
    Refresh,
    UseFolder,
    UseOnline,
    /// "Browse online library": remember the choice and fetch the small index (once).
    Connect,
    /// Switch the online library off again (what is installed stays).
    Disconnect,
}

thread_local! {
    /// The Luts tab's file listing and the folder stamps it was made from (a folder's modified time
    /// changes when a file is added or removed), so it is not re-scanned every frame.
    static LUT_LIST: RefCell<Option<(Vec<Option<std::time::SystemTime>>, Vec<String>)>> = const { RefCell::new(None) };
}

/// Re-list the LUT folders next time (after an install / remove).
pub(crate) fn forget_lut_listing() {
    LUT_LIST.with(|l| *l.borrow_mut() = None);
}

/// `.cube` files in the managed library folder, then in `Settings.lut_dirs` (24 per folder, as before).
fn lut_files(settings: &Settings) -> Vec<String> {
    let dirs: Vec<std::path::PathBuf> = std::iter::once(crate::engine::library::root().join("luts"))
        .chain(settings.lut_dirs.iter().map(std::path::PathBuf::from))
        .collect();
    let stamps: Vec<_> = dirs.iter().map(|d| std::fs::metadata(d).and_then(|m| m.modified()).ok()).collect();
    if let Some(list) = LUT_LIST.with(|l| l.borrow().as_ref().filter(|(s, _)| *s == stamps).map(|(_, v)| v.clone())) {
        return list;
    }
    let list: Vec<String> = dirs
        .iter()
        .flat_map(|d| {
            std::fs::read_dir(d)
                .into_iter()
                .flatten()
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("cube")))
                .take(24)
        })
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    LUT_LIST.with(|l| *l.borrow_mut() = Some((stamps, list.clone())));
    list
}

/// A small cloud in a tile's top-right corner: online, not installed yet.
fn cloud(p: &egui::Painter, tile: Rect) {
    let c = egui::pos2(tile.right() - 13.0, tile.top() + 10.0);
    let col = Color32::from_white_alpha(230);
    p.rect_filled(Rect::from_center_size(c + egui::vec2(0.0, 2.0), egui::vec2(16.0, 6.0)), 3.0, col);
    p.circle_filled(c + egui::vec2(-3.0, 0.0), 3.5, col);
    p.circle_filled(c + egui::vec2(2.5, -1.5), 4.5, col);
}

thread_local! {
    /// Ctrl+F while the pointer is over the Gallery: the next frame's header focuses its search box.
    static FOCUS_SEARCH: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Ask the Gallery's search box to take the keyboard (Ctrl+F over the pane).
pub(crate) fn focus_search() {
    FOCUS_SEARCH.with(|f| f.set(true));
}

/// What each tab's items can come from: the chips under the search box.
fn sources(tab: GalleryTab, lib_tab: bool) -> &'static [Src] {
    match tab {
        _ if lib_tab => &[Src::All, Src::Mine, Src::Installed, Src::Online],
        GalleryTab::Luts => &[Src::All, Src::Mine, Src::Installed, Src::Online],
        GalleryTab::Effects | GalleryTab::Transitions => {
            &[Src::All, Src::BuiltIn, Src::Mine, Src::Installed, Src::Online]
        }
        GalleryTab::Sounds => &[],
        _ => &[Src::All, Src::BuiltIn, Src::Mine],
    }
}

/// The library kind a tab shows library items of.
pub fn lib_kind_of(tab: GalleryTab) -> Option<crate::engine::library::Kind> {
    use crate::engine::library::Kind;
    match tab {
        GalleryTab::Effects => Some(Kind::Effect),
        GalleryTab::Transitions => Some(Kind::Transition),
        GalleryTab::Luts => Some(Kind::Lut),
        _ => None,
    }
}

fn cell_src(c: &LibCell) -> Src {
    if c.yours {
        Src::Mine
    } else if c.installed {
        Src::Installed
    } else {
        Src::Online
    }
}

/// Does a library cell pass the search, the source chip and the category?
fn cell_ok(f: &Filters, c: &LibCell, extra_cat: Option<&str>) -> bool {
    let tags: Vec<&str> = c.tags.iter().map(String::as_str).chain(extra_cat).collect();
    f.passes(cell_src(c), &tags, &format!("{} {} {} {}", c.name, c.id, c.desc, c.tags.join(" ")))
}

/// The key a card's apply count is stored under (`Settings.gallery_used`).
pub fn used_key(tab: &str, name: &str) -> String {
    format!("{tab}:{}", name.strip_prefix("mine:").unwrap_or(name))
}

fn used_of(settings: &Settings, tab: &str, name: &str) -> u32 {
    settings.gallery_used.get(&used_key(tab, name)).copied().unwrap_or(0)
}

/// One header for every tab: search (Ctrl+F focuses it, Esc clears it) with the result count, the source
/// chips, a category, a sort order, the card size, and a small menu (folder, import, the online library).
/// Online items are a chip like any other: the library is a switch in the banner or the menu, never a
/// separate screen.
#[allow(clippy::too_many_arguments)]
fn header(
    ui: &mut egui::Ui,
    state: &mut GalleryState,
    srcs: &[Src],
    cats: &[String],
    lib: Option<&LibView>,
    folder: Option<GKind>,
    sounds: bool,
    out: &mut GalleryResponse,
) {
    let over = ui.rect_contains_pointer(ui.max_rect());
    let want_focus = FOCUS_SEARCH.with(|f| f.replace(false));
    ui.horizontal(|ui| {
        let w = (ui.available_width() - 56.0).max(60.0);
        let r = ui.add(
            egui::TextEdit::singleline(&mut state.f.q)
                .id(egui::Id::new("gallery_search"))
                .hint_text("Search name, tags, description")
                .desired_width(w),
        );
        if want_focus {
            r.request_focus();
        }
        if (r.has_focus() || over) && !state.f.q.is_empty() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            state.f.q.clear();
            r.surrender_focus();
        }
        let (shown, total) = state.count;
        if total > 0 {
            ui.weak(if state.f.active() { format!("{shown}/{total}") } else { format!("{total}") })
                .on_hover_text("Cards shown / in this tab");
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        if srcs.len() > 1 {
            for &s in srcs {
                if ui.selectable_label(state.f.src == s, s.label()).clicked() {
                    state.f.src = s;
                }
            }
        }
        // (menu buttons, not combo boxes: a combo box is a fixed ~100 pt wide and widens a narrow pane)
        if !cats.is_empty() {
            let shown = if state.f.cat.is_empty() { "All categories" } else { state.f.cat.as_str() };
            menu::button(ui, format!("{shown} ▾"), |ui| {
                if menu::check(ui, state.f.cat.is_empty(), "All categories", "").clicked() {
                    state.f.cat.clear();
                }
                menu::scroll(ui, |ui| {
                    for c in cats {
                        if menu::check(ui, state.f.cat.eq_ignore_ascii_case(c), c, "").clicked() {
                            state.f.cat = c.clone();
                        }
                    }
                });
            });
        }
        menu::button(ui, format!("{} ▾", state.f.sort.label()), |ui| {
            for s in Sort::ALL {
                if menu::check(ui, state.f.sort == s, s.label(), "").clicked() {
                    state.f.sort = s;
                }
            }
        });
        for s in Size::ALL {
            if ui.selectable_label(state.f.size == s, s.label()).on_hover_text("Card size").clicked() {
                state.f.size = s;
            }
        }
        menu::button(ui, "⋯", |ui| {
            if let Some(l) = lib {
                if l.connected {
                    if menu::row(ui, None, "Refresh the online catalogue", "").clicked() {
                        out.lib = Some(LibAct::Refresh);
                    }
                    if menu::row(ui, None, "Turn the online library off", "").clicked() {
                        out.lib = Some(LibAct::Disconnect);
                    }
                }
                if menu::row(ui, None, "Use a folder as the library…", "").clicked() {
                    out.lib = Some(LibAct::UseFolder);
                }
                if l.local && menu::row(ui, None, "Use the online library instead", "").clicked() {
                    out.lib = Some(LibAct::UseOnline);
                }
                ui.separator();
            }
            if folder.is_some() {
                if menu::row(ui, None, "Open the gallery folder", "").clicked() {
                    out.open_folder = Some(folder);
                }
                if menu::row(ui, None, "Import to Gallery…", "").clicked() {
                    out.import = Some(false);
                }
                if menu::row(ui, None, "Import a folder…", "").clicked() {
                    out.import = Some(true);
                }
            }
            if sounds && menu::check(ui, !state.no_hover_preview, "Preview on hover", "").clicked() {
                state.no_hover_preview = !state.no_hover_preview;
            }
        });
    });
    if let Some(l) = lib {
        if !l.connected {
            if ui
                .small_button("Browse online library")
                .on_hover_text("Fetch the list of free items. Until then only built-in and installed ones show; nothing else is downloaded until you click an item.")
                .clicked()
            {
                out.lib = Some(LibAct::Connect);
            }
        } else if l.loading {
            ui.weak("Loading the online list…");
        } else if !l.note.is_empty() {
            ui.weak(&l.note);
        }
    }
}

/// What a library cell's menu adds for a transition: the selection's `offer`.
type Rows<'a> = Option<&'a transitions_ui::Offer>;

/// The transition menu rows ("Apply to Start / End / Connection / All"), shared by the built-in and the
/// library cards: `pick` gets the `Where` of the row clicked. Rows that cannot apply are greyed out with
/// the reason; the connection rows and "All" exist only when they can mean something.
fn transition_rows(ui: &mut egui::Ui, off: &transitions_ui::Offer, mut pick: impl FnMut(transitions_ui::Where)) {
    use transitions_ui::Where;
    for (label, why, w) in [("Apply to Start", off.start, Where::Start), ("Apply to End", off.end, Where::End)] {
        let r = ui.add_enabled_ui(why.is_none(), |ui| menu::row(ui, None, label, "")).inner;
        if r.on_disabled_hover_text(why.unwrap_or("")).clicked() {
            pick(w);
        }
    }
    let conn: Vec<(&str, Where)> = match (off.conn_start, off.conn_end) {
        (true, true) => {
            vec![("Apply to Connection (start)", Where::ConnStart), ("Apply to Connection (end)", Where::ConnEnd)]
        }
        (true, false) => vec![("Apply to Connection", Where::ConnStart)],
        (false, true) => vec![("Apply to Connection", Where::ConnEnd)],
        (false, false) => Vec::new(),
    };
    for (label, w) in conn {
        if menu::row(ui, None, label, "").clicked() {
            pick(w);
        }
    }
    if off.all && menu::row(ui, None, "Apply to All", "").clicked() {
        pick(Where::All);
    }
}

/// One library cell, in every grid that lists library items (`mark`: flag installed ones, for the
/// merged Effects / Transitions grids where built-in cards sit beside them). `rows` = the transition
/// menu rows when the cell is a transition. A skill is not draggable (it makes no sense on a timeline).
fn lib_cell(
    ui: &mut egui::Ui,
    c: &LibCell,
    thumb: &mut dyn FnMut(&str) -> Option<(egui::TextureId, [u32; 2])>,
    mark: bool,
    rows: Rows,
    out: &mut GalleryResponse,
) -> Response {
    let drag = (!c.skill).then(|| crate::ui::DragPayload::LibItem(c.id.clone()));
    let short = crate::engine::library::split_id(&c.id).map(|(_, s)| s).unwrap_or_default();
    let r = card(ui, ("lib", c.id.as_str()), &c.name, drag, false, |p, t| {
        if p.clip_rect().intersects(t) {
            // an installed shader effect shows what it does (rendered by the GPU), else the catalogue picture
            let own = (c.kind == crate::engine::library::Kind::Effect && c.installed)
                .then(|| crate::ui::effects_ui::library_thumbnail(&short))
                .flatten()
                .map(|tex| (tex, [0, 0]));
            thumb_or_blank(p, t, own.or_else(|| thumb(&c.id)));
        }
        if !c.installed {
            cloud(p, t);
        } else if mark {
            p.circle_filled(t.left_top() + egui::vec2(8.0, 8.0), 3.5, Color32::from_rgb(90, 160, 255));
        }
        if let Some(f) = c.busy {
            let bar = Rect::from_min_size(t.left_bottom() - egui::vec2(0.0, 4.0), egui::vec2(t.width(), 4.0));
            p.rect_filled(bar, 0.0, Color32::from_black_alpha(160));
            let w = t.width() * f.clamp(0.1, 1.0);
            p.rect_filled(Rect::from_min_size(bar.min, egui::vec2(w, 4.0)), 0.0, Color32::LIGHT_BLUE);
        }
    });
    let r = if c.desc.is_empty() { r } else { r.on_hover_text(&c.desc) };
    if r.clicked() {
        out.lib = Some(if c.skill { LibAct::View(c.id.clone()) } else { LibAct::Apply(c.id.clone()) });
    }
    menu::context(&r, |ui| {
        if let Some(off) = rows {
            transition_rows(ui, off, |w| out.lib = Some(LibAct::ApplyAt(c.id.clone(), w.name())));
            ui.separator();
        }
        let items: [(bool, &str, fn(String) -> LibAct); 5] = [
            (!c.skill, "Apply", LibAct::Apply),
            (!c.installed, "Install", LibAct::Install),
            (c.skill, "View text", LibAct::View),
            (true, "Show in folder", LibAct::Reveal),
            (c.installed, "Remove", LibAct::Remove),
        ];
        for (show, label, act) in items {
            if show && menu::row(ui, None, label, "").clicked() {
                out.lib = Some(act(c.id.clone()));
            }
        }
    });
    r
}

/// The cells that pass the filters, in the chosen order (counted into the header's "shown / total").
fn filtered_cells<'a>(
    state: &mut GalleryState,
    cells: &'a [LibCell],
    settings: &Settings,
    extra_cat: Option<&str>,
) -> Vec<&'a LibCell> {
    let f = &state.f;
    let mut hits: Vec<&LibCell> = cells.iter().filter(|c| cell_ok(f, c, extra_cat)).collect();
    gf::sort_by(&mut hits, f.sort, |c| (c.name.clone(), used_of(settings, "lib", &c.id), 0));
    state.count_acc.0 += hits.len();
    state.count_acc.1 += cells.len();
    hits
}

/// The library tabs (HTML / Icons / Skills): the cells that pass the filters in one grid.
fn library_grid(
    ui: &mut egui::Ui,
    state: &mut GalleryState,
    lib: &mut LibView,
    settings: &Settings,
    out: &mut GalleryResponse,
) {
    let LibView { cells, thumb, connected, loading, .. } = lib;
    let hits = filtered_cells(state, cells, settings, None);
    if hits.is_empty() && !*loading {
        ui.weak(match (cells.is_empty(), *connected) {
            (true, false) => "Nothing installed yet. Browse the online library to find some.",
            (true, true) => "Nothing here yet.",
            _ => "Nothing matches.",
        });
    }
    gf::grid(ui, |ui| {
        for c in hits {
            lib_cell(ui, c, &mut **thumb, false, None, out);
        }
    });
}

/// The categories a tab's category menu offers (cheap, so the header can sit above the scroll area).
fn tab_cats(tab: GalleryTab, lib: Option<&LibView>, lib_tab: bool, now: f64) -> Vec<String> {
    let mut v: Vec<String> = match tab {
        _ if lib_tab => Vec::new(),
        GalleryTab::Effects => crate::ui::effects_ui::categories().into_iter().map(String::from).collect(),
        GalleryTab::Transitions => TR_CATS.iter().map(|s| s.to_string()).collect(),
        _ => Vec::new(),
    };
    let mut extra: Vec<String> = Vec::new();
    if !lib_tab {
        if let Some(k) = mine_kind(tab) {
            extra.extend(mine_items(k, now).iter().flat_map(|i| i.tags.iter().cloned()));
        }
    }
    if let Some(l) = lib {
        extra.extend(l.cells.iter().flat_map(|c| c.tags.iter().cloned()));
    }
    extra.sort_by_key(|t| t.to_lowercase());
    for t in extra {
        if !v.iter().any(|x| x.eq_ignore_ascii_case(&t)) {
            v.push(t);
        }
    }
    v.truncate(60);
    v
}

/// The Transitions tab's categories.
const TR_CATS: [&str; 3] = ["Fades", "Slides & wipes", "Shader"];

fn tr_cat(k: crate::model::TransitionKind) -> &'static str {
    use crate::model::TransitionKind as K;
    match k {
        K::CrossFade | K::FadeToColor => TR_CATS[0],
        K::Push | K::Wipe => TR_CATS[1],
        K::Shader => TR_CATS[2],
    }
}

/// The Effects tab: your saved effects, the built-in effects and the library's shader effects in ONE grid
/// under the shared header. A card click adds the effect to every eligible selected clip; right-click:
/// selected clips / every clip on the track; a press-and-move drags it onto a clip.
#[allow(clippy::too_many_arguments)]
fn effects_tab(
    ui: &mut egui::Ui,
    state: &mut GalleryState,
    project: &crate::model::Project,
    selection: &[Id],
    palette: &Palette,
    lib: Option<&mut LibView>,
    settings: &Settings,
    out: &mut GalleryResponse,
) {
    use crate::model::{ClipKind, EffectKind};
    enum E<'a> {
        Mine(&'a MineItem),
        Built(EffectKind),
        Lib(&'a LibCell),
    }
    // the first selected clip that still exists drives the audio-aware filter
    let audio = selection.iter().find_map(|&id| project.clip(id)).map(|c| c.kind == ClipKind::Audio);
    let has_sel = audio.is_some();
    let now = ui.input(|i| i.time);
    let cats = crate::ui::effects_ui::categories();
    let mine_all = mine_items(GKind::Effect, now);
    let mut built: Vec<EffectKind> =
        EffectKind::ALL.into_iter().filter(|&k| crate::ui::effects_ui::fits_selection(k, audio)).collect();
    built.sort_by_key(|k| cats.iter().position(|c| *c == k.category())); // grouped by category, catalogue order
    let (mut thumb, cells): (Option<&mut dyn FnMut(&str) -> Option<(egui::TextureId, [u32; 2])>>, &[LibCell]) =
        match lib {
            Some(l) => (Some(&mut *l.thumb), &l.cells[..]),
            None => (None, &[]),
        };
    let pixel = audio != Some(true); // saved effects and library ones are pixel effects
    let mut all: Vec<E> = Vec::new();
    if pixel {
        all.extend(mine_all.iter().map(E::Mine));
    }
    all.extend(built.iter().map(|&k| E::Built(k)));
    if pixel {
        all.extend(cells.iter().map(E::Lib));
    }
    let total = all.len();
    let f = state.f.clone();
    all.retain(|e| match e {
        E::Mine(i) => {
            let t: Vec<&str> = i.tags.iter().map(String::as_str).collect();
            f.passes(Src::Mine, &t, &format!("{} {}", i.name, i.tags.join(" ")))
        }
        E::Built(k) => f.passes(Src::BuiltIn, &[k.category()], &format!("{} {}", k.name(), k.category())),
        E::Lib(c) => cell_ok(&f, c, None),
    });
    gf::sort_by(&mut all, f.sort, |e| match e {
        E::Mine(i) => (i.name.clone(), used_of(settings, "Effects", &i.name), i.created),
        E::Built(k) => (k.name().to_string(), used_of(settings, "Effects", k.name()), 0),
        E::Lib(c) => (c.name.clone(), used_of(settings, "lib", &c.id), 0),
    });
    state.count_acc.0 += all.len();
    state.count_acc.1 += total;
    if all.is_empty() {
        ui.weak(if audio == Some(true) && total == 0 { "No audio effects yet" } else { "No effect matches" });
    }
    gf::grid(ui, |ui| {
        for e in all {
            match e {
                E::Mine(it) => {
                    let first = it
                        .data
                        .get("effects")
                        .and_then(|e| e.get(0))
                        .and_then(|e| serde_json::from_value::<crate::model::Effect>(e.clone()).ok());
                    let drag = Some(crate::ui::DragPayload::GalleryCard(GalleryTab::Effects, it.name.clone()));
                    let r = card(ui, ("fxm", it.slug.as_str()), &it.name, drag, false, |p, t| {
                        match first.as_ref().and_then(|e| crate::ui::effects_ui::thumbnail(e.kind)) {
                            Some((tex, _)) => thumb_or_blank(p, t, Some((tex, [0, 0]))),
                            None => named_tile(p, t, &it.name, palette),
                        }
                        mine_badge(p, t, palette.accent);
                    });
                    let n = it.data.get("effects").and_then(|e| e.as_array()).map_or(0, |a| a.len());
                    let r = r.on_hover_text(format!("{} - {n} effect{}", it.name, if n == 1 { "" } else { "s" }));
                    let want = format!("mine:{}", it.name);
                    if r.clicked() {
                        out.apply = Some((GalleryTab::Effects, want.clone(), 1.0));
                    }
                    menu::context(&r, |ui| {
                        fx_rows(ui, has_sel, &want, out);
                        mine_rows(ui, out, GKind::Effect, &it.name);
                    });
                }
                E::Built(kind) => {
                    let r = card(
                        ui,
                        ("fx", kind),
                        kind.name(),
                        Some(crate::ui::DragPayload::Effect(kind)),
                        false,
                        |p, t| effect_tile(p, t, kind, palette),
                    );
                    let r = r.on_hover_text(format!("{} · {}", kind.name(), kind.category()));
                    if r.clicked() {
                        out.apply = Some((GalleryTab::Effects, kind.name().to_string(), 1.0));
                    }
                    menu::context(&r, |ui| fx_rows(ui, has_sel, kind.name(), out));
                }
                E::Lib(c) => {
                    if let Some(t) = thumb.as_deref_mut() {
                        lib_cell(ui, c, t, true, None, out);
                    }
                }
            }
        }
    });
}

/// "Add to selected clip(s)" / "Add to all clips on this track" for an effect card.
fn fx_rows(ui: &mut egui::Ui, has_sel: bool, name: &str, out: &mut GalleryResponse) {
    ui.add_enabled_ui(has_sel, |ui| {
        if menu::row(ui, None, "Add to selected clip(s)", "").clicked() {
            out.apply = Some((GalleryTab::Effects, name.to_string(), 1.0));
        }
        if menu::row(ui, None, "Add to all clips on this track", "").clicked() {
            out.apply = Some((GalleryTab::Effects, name.to_string(), 1.0));
            out.apply_where = Some("track");
        }
    });
}

/// An effect card's picture: the GPU thumbnail, a neutral named tile without one, or a solid green tile
/// with a note for an audio effect (nothing to render).
fn effect_tile(p: &egui::Painter, tile: Rect, kind: crate::model::EffectKind, palette: &Palette) {
    if kind.applies_to_audio() {
        p.rect_filled(tile, 2.0, palette.clip_audio);
        crate::ui::tools::draw_glyph(p, tile, crate::ui::tools::Glyph::MusicNote, Color32::WHITE);
    } else if let Some((tex, _)) = crate::ui::effects_ui::thumbnail(kind) {
        thumb_or_blank(p, tile, Some((tex, [0, 0])));
    } else {
        named_tile(p, tile, kind.name(), palette);
    }
}

/// No picture yet (no GL / not rendered): a neutral tile that still names the card.
fn named_tile(p: &egui::Painter, tile: Rect, name: &str, palette: &Palette) {
    let font = egui::FontId::proportional(11.0);
    p.rect_filled(tile, 2.0, palette.header);
    let g = p.layout(name.to_string(), font, palette.text_dim, gf::dims(p.ctx()).0 - 8.0);
    p.galley(tile.center() - g.size() / 2.0, g, palette.text_dim);
}

/// The Transitions tab: the defaults row, then the built-in transitions, yours and the library's shader
/// transitions in ONE grid under the shared header. A card click applies it at the start of the selection
/// (and picks it for Ctrl+T); a press-and-move drags it onto a cut; right-click: Apply to Start / End /
/// Connection / All.
#[allow(clippy::too_many_arguments)]
fn transitions_tab(
    ui: &mut egui::Ui,
    state: &mut GalleryState,
    project: &crate::model::Project,
    selection: &[Id],
    palette: &Palette,
    tr: &mut transitions_ui::TransitionsState,
    lib: Option<&mut LibView>,
    settings: &Settings,
    out: &mut GalleryResponse,
) {
    use crate::model::TransitionKind;
    enum E<'a> {
        Built(usize, TransitionKind),
        Mine(&'a MineItem, TransitionKind),
        Lib(&'a LibCell),
    }
    transitions_ui::defaults_row(ui, tr);
    let now = ui.input(|i| i.time);
    let off = transitions_ui::offer(project, selection, tr.duration);
    let has_sel = off.start != Some("Select a clip first");
    let installed = transitions_ui::library_items(crate::engine::library::Kind::Transition, now);
    let mine_all = mine_items(GKind::Transition, now);
    let (mut thumb, cells): (Option<&mut dyn FnMut(&str) -> Option<(egui::TextureId, [u32; 2])>>, &[LibCell]) =
        match lib {
            Some(l) => (Some(&mut *l.thumb), &l.cells[..]),
            None => (None, &[]),
        };
    let mut all: Vec<E> = TransitionKind::ALL.into_iter().enumerate().map(|(i, k)| E::Built(i, k)).collect();
    for it in mine_all.iter() {
        let kind = it.data.get("kind").and_then(|k| k.as_str()).and_then(TransitionKind::from_name);
        let kind = kind.unwrap_or(if it.data.get("shader").and_then(|s| s.as_str()).is_some_and(|s| !s.is_empty()) {
            TransitionKind::Shader
        } else {
            TransitionKind::CrossFade
        });
        all.push(E::Mine(it, kind));
    }
    all.extend(cells.iter().map(E::Lib));
    let total = all.len();
    let f = state.f.clone();
    all.retain(|e| match e {
        E::Built(_, k) => f.passes(Src::BuiltIn, &[tr_cat(*k)], &format!("{} {}", k.name(), tr_cat(*k))),
        E::Mine(i, k) => {
            let t: Vec<&str> = i.tags.iter().map(String::as_str).chain([tr_cat(*k)]).collect();
            f.passes(Src::Mine, &t, &format!("{} {}", i.name, i.tags.join(" ")))
        }
        E::Lib(c) => cell_ok(&f, c, Some(TR_CATS[2])),
    });
    gf::sort_by(&mut all, f.sort, |e| match e {
        E::Built(_, k) => (k.name().to_string(), used_of(settings, "Transitions", k.name()), 0),
        E::Mine(i, _) => (i.name.clone(), used_of(settings, "Transitions", &i.name), i.created),
        E::Lib(c) => (c.name.clone(), used_of(settings, "lib", &c.id), 0),
    });
    state.count_acc.0 += all.len();
    state.count_acc.1 += total;
    if all.is_empty() {
        ui.weak("No transition matches");
    }
    gf::grid(ui, |ui| {
        for e in all {
            match e {
                E::Built(i, k) => {
                    let r = card(
                        ui,
                        ("tr", i),
                        k.name(),
                        Some(crate::ui::DragPayload::Transition(k)),
                        tr.kind == i,
                        |p, t| match transitions_ui::stock_id() {
                            Some(tex) => transitions_ui::paint_preview(p, t, tex, k, &*tr, palette),
                            None => named_tile(p, t, k.name(), palette),
                        },
                    );
                    let r = r.on_hover_text(format!(
                        "{} - click: add at the start of the selection · drag onto a cut",
                        k.name()
                    ));
                    if crate::ui::hover_after(ui, r.id, &r, 150.0) {
                        out.hover = Some((GalleryTab::Transitions, k.name().to_string()));
                    }
                    // a quick action also picks the kind, so Ctrl+T repeats what was just applied
                    if r.clicked() {
                        tr.kind = i;
                        if has_sel {
                            out.apply = Some((GalleryTab::Transitions, k.name().to_string(), 1.0));
                            out.apply_where = Some(transitions_ui::Where::All.name());
                        }
                    }
                    menu::context(&r, |ui| {
                        transition_rows(ui, &off, |w| {
                            tr.kind = i;
                            out.apply = Some((GalleryTab::Transitions, k.name().to_string(), 1.0));
                            out.apply_where = Some(w.name());
                        });
                    });
                }
                E::Mine(it, kind) => {
                    // your own saved transitions (length, colour, direction, shader controls)
                    let mut ts = transitions_ui::TransitionsState::default();
                    if let Some(c) =
                        it.data.get("color").and_then(|c| serde_json::from_value::<[u8; 4]>(c.clone()).ok())
                    {
                        ts.color = c;
                    }
                    ts.direction = it.data.get("direction").and_then(|d| d.as_u64()).unwrap_or(0) as u8;
                    let drag = Some(crate::ui::DragPayload::GalleryCard(GalleryTab::Transitions, it.name.clone()));
                    let r = card(ui, ("trm", it.slug.as_str()), &it.name, drag, false, |p, t| {
                        match transitions_ui::stock_id() {
                            Some(tex) => transitions_ui::paint_preview(p, t, tex, kind, &ts, palette),
                            None => named_tile(p, t, &it.name, palette),
                        }
                        mine_badge(p, t, palette.accent);
                    });
                    let dur = it.data.get("duration").and_then(|d| d.as_f64()).unwrap_or(1.0);
                    let r =
                        r.on_hover_text(format!("{} - {dur:.2} s · click: add at the start of the selection", it.name));
                    let want = format!("mine:{}", it.name);
                    if r.clicked() && has_sel {
                        out.apply = Some((GalleryTab::Transitions, want.clone(), 1.0));
                        out.apply_where = Some(transitions_ui::Where::All.name());
                    }
                    menu::context(&r, |ui| {
                        transition_rows(ui, &off, |w| {
                            out.apply = Some((GalleryTab::Transitions, want.clone(), 1.0));
                            out.apply_where = Some(w.name());
                        });
                        mine_rows(ui, out, GKind::Transition, &it.name);
                    });
                }
                E::Lib(c) => {
                    let Some(t) = thumb.as_deref_mut() else { continue };
                    let r = lib_cell(ui, c, t, true, Some(&off), out);
                    // an installed one previews in the monitor: its source runs there
                    if c.installed && crate::ui::hover_after(ui, r.id, &r, 150.0) {
                        let short = crate::engine::library::split_id(&c.id).map(|(_, s)| s).unwrap_or_default();
                        out.hover_shader = installed.iter().find(|i| i.id == short).map(|i| i.src.to_string());
                    }
                }
            }
        }
    });
}

#[derive(Default)]
pub struct GalleryState {
    pub tab: GalleryTab,
    // ---- ws:online-library ----
    /// A library tab (`LIB_TABS`) is open instead of `tab`.
    pub lib: Option<crate::engine::library::Kind>,
    /// The header's search, source chip, category, sort and card size (shared by every tab; the search,
    /// chip and category reset when the tab changes).
    pub f: Filters,
    /// The tab the filters were last used on.
    seen: Option<(GalleryTab, Option<crate::engine::library::Kind>)>,
    /// (cards shown, cards in the tab), counted while drawing and shown by the header next frame.
    pub count: (usize, usize),
    count_acc: (usize, usize),
    /// The Sounds tab does not preview a sound while the pointer rests on it.
    pub no_hover_preview: bool,
    // ---- ws:text-titles ----
    /// (clip id, exposed field name) rows for the Customize panel below the Titles grid, set by the
    /// caller (`ui::app::gallery_ctl::draw`) right after a Titles card is placed - a plain state field
    /// rather than an `egui::Id`-keyed temp, since `App` already owns `GalleryState` per frame.
    pub customize: Vec<(Id, String)>,
}

#[derive(Default)]
pub struct GalleryResponse {
    /// (tab, card name, intensity) - Looks/Luts/Captions/SpeedRamps/Transitions; routed through
    /// `App::run_tool_undoable("gallery.apply", …)` by the caller.
    pub apply: Option<(GalleryTab, String, f32)>,
    /// (tab, card name) hovered >=150ms; `None` clears. Templates never hover-preview (no catalogue
    /// data model to render from without a synthetic mini-Project pass - see the module doc comment).
    pub hover: Option<(GalleryTab, String)>,
    /// A Templates card was clicked - placed at the playhead by the caller (`App::place_template`,
    /// already the established path, not new wiring).
    pub place: Option<String>,
    /// ---- ws:text-titles ----: a Titles card was clicked - resolved across `builtin_titles()` +
    /// `Settings.templates` and placed by the caller (`gallery_ctl::draw`), which then fills
    /// `GalleryState.customize` from the placed clips' `exposed` fields.
    pub place_title: Option<String>,
    /// ws:text-titles: Customize-panel rows edited this frame, as edited clones of the placed clips -
    /// written back by the caller with one undo.
    pub customized: Vec<(Id, Clip)>,
    /// ws:keyframe-blocks: forget this saved block (Animate card menu).
    pub forget_block: Option<String>,
    /// ws:online-library: a library cell click / menu choice.
    pub lib: Option<LibAct>,
    /// How `apply` lands: "track" (an effect on every clip of the selection's track) or a
    /// `transitions_ui::Where` name for the Transitions tab. None = the default (the selection / its start).
    pub apply_where: Option<&'static str>,
    /// An installed library transition hovered >=150ms: its source, for `AltRequest::ShaderTransition`.
    pub hover_shader: Option<String>,
    /// A right-click choice on one of your saved cards.
    pub mine: Option<(MineAct, MineRef)>,
    /// "Import…" clicked (`true` = a folder).
    pub import: Option<bool>,
    /// "Folder" clicked: open the gallery folder of that kind.
    pub open_folder: Option<Option<GKind>>,
    /// The empty-space menu's "Save to Gallery…".
    pub save_dialog: bool,
    /// A sound card was clicked: add it at the playhead.
    pub sound: Option<String>,
}

// ---- your own Gallery ("Mine") ----
use crate::engine::gallery::{self as store, Item as MineItem, Kind as GKind};
use crate::ui::app::gallery_mine::{MineAct, MineRef};
use std::rc::Rc;

thread_local! {
    /// `engine::gallery` listings per kind, re-read only after a change (`forget_mine`) or when the folder's
    /// modified time moved (a file dropped in by hand), checked at most every 2 s of UI time.
    static MINE: RefCell<HashMap<GKind, (f64, Option<std::time::SystemTime>, Rc<Vec<MineItem>>)>> = RefCell::default();
    /// The Sounds tab: audio files whose waveform a visible card wants, and the peaks that arrived.
    static SOUND_WANT: RefCell<Vec<std::path::PathBuf>> = const { RefCell::new(Vec::new()) };
    static SOUND_PEAKS: RefCell<HashMap<std::path::PathBuf, std::sync::Arc<crate::media::waveform::Peaks>>> = RefCell::default();
    /// The sound under the pointer for long enough to preview (this frame's wish).
    static SOUND_WISH: RefCell<Option<std::path::PathBuf>> = const { RefCell::new(None) };
}

/// Saved items of `kind` (cheap: cached).
pub fn mine_items(kind: GKind, now: f64) -> Rc<Vec<MineItem>> {
    let dir = store::root().join(kind.dir());
    let stamp = std::fs::metadata(&dir).and_then(|m| m.modified()).ok();
    MINE.with(|m| {
        let mut m = m.borrow_mut();
        if let Some((at, st, items)) = m.get_mut(&kind) {
            if now - *at < 2.0 && now >= *at {
                return items.clone();
            }
            *at = now;
            if *st == stamp {
                return items.clone();
            }
        }
        let items = Rc::new(store::list(&store::root(), kind));
        m.insert(kind, (now, stamp, items.clone()));
        items
    })
}

/// Something was saved / renamed / deleted / imported: re-read on the next frame.
pub fn forget_mine() {
    MINE.with(|m| m.borrow_mut().clear());
}

pub(crate) fn set_sound_peaks(path: std::path::PathBuf, p: std::sync::Arc<crate::media::waveform::Peaks>) {
    SOUND_PEAKS.with(|m| m.borrow_mut().insert(path, p));
}

/// The audio files visible cards asked a waveform for (drained by the App, which decodes a few per frame).
pub(crate) fn take_sound_wants() -> Vec<std::path::PathBuf> {
    SOUND_WANT.with(|w| std::mem::take(&mut *w.borrow_mut()))
}

/// The sound whose preview should be playing now (None = silence); reset each time it is read.
pub(crate) fn take_sound_wish() -> Option<std::path::PathBuf> {
    SOUND_WISH.with(|w| w.borrow_mut().take())
}

/// The GalleryTab's saved-item kind.
pub fn mine_kind(tab: GalleryTab) -> Option<GKind> {
    Some(match tab {
        GalleryTab::Looks => GKind::Look,
        GalleryTab::Captions => GKind::Caption,
        GalleryTab::SpeedRamps => GKind::Speed,
        GalleryTab::Effects => GKind::Effect,
        GalleryTab::Transitions => GKind::Transition,
        GalleryTab::Templates | GalleryTab::Titles => GKind::Template,
        GalleryTab::Animate => GKind::Animate,
        GalleryTab::Sounds => GKind::Sound,
        GalleryTab::Luts => return None,
    })
}

/// One card as `gallery.list` reports it: the same source / category / search / sort rules as the header.
pub struct CardInfo {
    pub name: String,
    /// A library item's id (`effects/glow`).
    pub id: Option<String>,
    pub src: Src,
    pub cats: Vec<String>,
    pub uses: u32,
}

/// The cards of `tab` that pass `f`, in its order: the built-in and saved ones, then the library rows
/// (`rows`: what `OnlineState::rows` knows - only what is installed until the user connects).
pub fn card_infos(
    tab: GalleryTab,
    settings: &Settings,
    rows: &[crate::engine::library::Row],
    f: &Filters,
    now: f64,
) -> Vec<CardInfo> {
    let lib_luts = crate::engine::library::root().join("luts");
    let mut v: Vec<(CardInfo, u64)> = card_names(tab, settings)
        .into_iter()
        .filter(|n| tab != GalleryTab::Luts || !std::path::Path::new(n).starts_with(&lib_luts))
        .map(|n| {
            let (mut cats, created) = mine_meta(tab, &n, now);
            match tab {
                GalleryTab::Effects => cats.extend(
                    crate::model::EffectKind::ALL
                        .into_iter()
                        .filter(|k| k.name() == n)
                        .map(|k| k.category().to_string()),
                ),
                GalleryTab::Transitions => {
                    cats.extend(crate::model::TransitionKind::from_name(&n).map(|k| tr_cat(k).to_string()))
                }
                _ => {}
            }
            let mine = tab == GalleryTab::Luts || is_mine(tab, &n, settings, now);
            let src = if mine { Src::Mine } else { Src::BuiltIn };
            (CardInfo { uses: used_of(settings, tab.name(), &n), name: n, id: None, src, cats }, created)
        })
        .collect();
    for r in rows.iter().filter(|r| Some(r.kind) == lib_kind_of(tab)) {
        let mut cats = r.tags.clone();
        if tab == GalleryTab::Transitions {
            cats.push(TR_CATS[2].to_string());
        }
        let src = if r.yours {
            Src::Mine
        } else if r.installed {
            Src::Installed
        } else {
            Src::Online
        };
        v.push((
            CardInfo { name: r.name.clone(), id: Some(r.id.clone()), src, cats, uses: used_of(settings, "lib", &r.id) },
            0,
        ));
    }
    v.retain(|(c, _)| {
        let cats: Vec<&str> = c.cats.iter().map(String::as_str).collect();
        f.passes(c.src, &cats, &format!("{} {}", c.name, c.cats.join(" ")))
    });
    gf::sort_by(&mut v, f.sort, |(c, created)| (c.name.clone(), c.uses, *created));
    v.into_iter().map(|(c, _)| c).collect()
}

/// The tags and save time of one of your saved items by name (empty for a built-in).
fn mine_meta(tab: GalleryTab, name: &str, now: f64) -> (Vec<String>, u64) {
    mine_kind(tab)
        .and_then(|k| mine_items(k, now).iter().find(|i| i.name == name).map(|i| (i.tags.clone(), i.created)))
        .unwrap_or_default()
}

/// Is the card `name` one of yours (a saved item) rather than a built-in?
pub fn is_mine(tab: GalleryTab, name: &str, settings: &Settings, now: f64) -> bool {
    match tab {
        GalleryTab::Looks => settings.effect_presets.iter().any(|p| p.name == name),
        GalleryTab::Captions => settings.caption_presets.iter().any(|s| s.text == name),
        GalleryTab::Animate => settings.key_blocks.iter().any(|b| b.name == name),
        GalleryTab::Templates | GalleryTab::Titles => settings.templates.iter().any(|t| t.name == name),
        GalleryTab::SpeedRamps | GalleryTab::Effects | GalleryTab::Transitions | GalleryTab::Sounds => {
            mine_kind(tab).is_some_and(|k| mine_items(k, now).iter().any(|i| i.name == name))
        }
        GalleryTab::Luts => false,
    }
}

/// The small "M" badge in a card's top-right corner: this one is yours.
fn mine_badge(p: &egui::Painter, tile: Rect, accent: Color32) {
    let r = Rect::from_min_size(tile.right_top() + egui::vec2(-15.0, 3.0), egui::vec2(12.0, 12.0));
    p.rect_filled(r, 3.0, accent);
    p.text(r.center(), egui::Align2::CENTER_CENTER, "M", egui::FontId::proportional(9.0), Color32::WHITE);
}

/// Rename / Edit tags / Duplicate / Replace with current / Open file location / Delete - the right-click
/// rows every saved card has.
fn mine_rows(ui: &mut egui::Ui, out: &mut GalleryResponse, kind: GKind, name: &str) {
    ui.separator();
    let r = MineRef { kind, name: name.to_string() };
    let mut rows = vec![("Rename…", MineAct::Rename), ("Edit tags…", MineAct::Tags), ("Duplicate", MineAct::Duplicate)];
    if kind != GKind::Sound {
        rows.push(("Replace with current", MineAct::ReplaceCurrent));
    }
    rows.push(("Open file location", MineAct::Reveal));
    rows.push(("Delete", MineAct::Delete));
    for (label, act) in rows {
        if menu::row(ui, None, label, "").clicked() {
            out.mine = Some((act, r.clone()));
        }
    }
}

/// The Sounds tab: one card per audio file in the gallery's `sounds` folder - waveform, length, name; the
/// pointer resting on one previews it, a click adds it at the playhead, a drag drops it on a track.
fn sounds_tab(
    ui: &mut egui::Ui,
    state: &mut GalleryState,
    palette: &Palette,
    settings: &Settings,
    out: &mut GalleryResponse,
) {
    use crate::ui::DragPayload;
    let all = mine_items(GKind::Sound, ui.input(|i| i.time));
    let f = state.f.clone();
    let mut hits: Vec<&MineItem> = all
        .iter()
        .filter(|i| {
            let t: Vec<&str> = i.tags.iter().map(String::as_str).collect();
            f.passes(Src::Mine, &t, &format!("{} {}", i.name, i.tags.join(" ")))
        })
        .collect();
    gf::sort_by(&mut hits, f.sort, |i| (i.name.clone(), used_of(settings, "Sounds", &i.name), i.created));
    state.count_acc.0 += hits.len();
    state.count_acc.1 += all.len();
    if all.is_empty() {
        ui.weak("No sounds yet. Drop audio files or folders here, or use Import in the ⋯ menu.");
    } else if hits.is_empty() {
        ui.weak("No sound matches.");
    }
    gf::grid(ui, |ui| {
        for it in hits {
            let Some(file) = it.file.clone() else { continue };
            let dur = it.data.get("duration").and_then(|d| d.as_f64());
            let drag = Some(DragPayload::GalleryCard(GalleryTab::Sounds, it.name.clone()));
            let r = card(ui, ("snd", it.slug.as_str()), &it.name, drag, false, |p, t| {
                p.rect_filled(t, 2.0, palette.clip_audio.gamma_multiply(0.35));
                if !p.clip_rect().intersects(t) {
                    return;
                }
                let peaks = SOUND_PEAKS.with(|m| m.borrow().get(&file).cloned());
                match peaks {
                    Some(pk) if !pk.is_empty() => {
                        let n = 48usize;
                        let secs = pk.len() as f64 / crate::media::waveform::PEAKS_PER_SEC as f64;
                        for i in 0..n {
                            let (a, b) = (secs * i as f64 / n as f64, secs * (i + 1) as f64 / n as f64);
                            let (lo, hi) = pk.range(a, b);
                            let x = t.left() + 2.0 + (t.width() - 4.0) * (i as f32 + 0.5) / n as f32;
                            let (cy, hh) = (t.center().y, t.height() * 0.45);
                            p.line_segment(
                                [egui::pos2(x, cy - hi.max(0.02) * hh), egui::pos2(x, cy - lo.min(-0.02) * hh)],
                                Stroke::new(1.5, palette.clip_audio),
                            );
                        }
                        let len = dur.unwrap_or(secs);
                        p.text(
                            t.left_bottom() + egui::vec2(3.0, -2.0),
                            egui::Align2::LEFT_BOTTOM,
                            format!("{len:.1}s"),
                            egui::FontId::proportional(9.0),
                            Color32::WHITE,
                        );
                    }
                    _ => {
                        SOUND_WANT.with(|w| {
                            let mut w = w.borrow_mut();
                            if !w.contains(&file) {
                                w.push(file.clone());
                            }
                        });
                    }
                }
                mine_badge(p, t, Color32::from_rgb(90, 140, 255));
            });
            let tags = if it.tags.is_empty() { String::new() } else { format!("\n{}", it.tags.join(", ")) };
            let r = r.on_hover_text(format!("{} - click: add at the playhead · drag onto a track{tags}", it.name));
            if !state.no_hover_preview
                && crate::ui::hover_after(ui, r.id, &r, 300.0)
                && !ui.ctx().is_being_dragged(r.id)
            {
                SOUND_WISH.with(|w| *w.borrow_mut() = Some(file.clone()));
            }
            if r.clicked() {
                out.sound = Some(it.name.clone());
            }
            menu::context(&r, |ui| {
                if menu::row(ui, None, "Add at playhead", "").clicked() {
                    out.sound = Some(it.name.clone());
                }
                mine_rows(ui, out, GKind::Sound, &it.name);
            });
        }
    });
}

/// Is the card `id` being dragged for real: past the drag threshold, and not cancelled? egui drops the
/// payload when Esc is pressed but the drag itself carries on while the button is down - so a card that
/// had a payload last frame and has none now was cancelled, and stays put until the button is released.
fn drag_lifted(ui: &egui::Ui, id: egui::Id) -> bool {
    let ctx = ui.ctx();
    let (on, was) = (egui::Id::new("gallery_drag_on"), egui::Id::new("gallery_drag_cancelled"));
    let me = id.value();
    if !ctx.input(|i| i.pointer.any_down()) {
        ctx.data_mut(|d| {
            d.remove_temp::<u64>(on);
            d.remove_temp::<u64>(was);
        });
        return false;
    }
    if !(ctx.is_being_dragged(id) && crate::ui::drag_intent(ui)) {
        return false;
    }
    if ctx.data(|d| d.get_temp::<u64>(was)) == Some(me) {
        return false;
    }
    if ctx.data(|d| d.get_temp::<u64>(on)) == Some(me) && !egui::DragAndDrop::has_any_payload(ctx) {
        ctx.data_mut(|d| d.insert_temp(was, me));
        return false;
    }
    ctx.data_mut(|d| d.insert_temp(on, me));
    true
}

/// One Gallery card: `picture` paints the tile's contents (thumbnail / swatch / nothing - the name is
/// drawn under it either way). Returns the card's response for hover/click.
///
/// The card claims its slot with `allocate_exact_size` BEFORE anything is drawn, which is what makes a
/// `horizontal_wrapped` grid wrap (a card drawn inside `ui::drag_source`'s `scope` takes whatever is left
/// of the row and never wraps - the whole tab ran off the pane's right edge in one row). With `drag` the
/// card is also a drag source: the payload is only handed to egui once the pointer has travelled past the
/// normal drag threshold (`ui::drag_intent`), so a click is never mistaken for a drag; the card is then
/// lifted under the cursor (a faint placeholder stays in its slot) and Esc cancels (egui's own dnd).
fn card(
    ui: &mut egui::Ui,
    salt: impl std::hash::Hash,
    name: &str,
    drag: Option<crate::ui::DragPayload>,
    selected: bool,
    picture: impl FnOnce(&egui::Painter, Rect),
) -> Response {
    let font = egui::TextStyle::Small.resolve(ui.style());
    let name_h = ui.text_style_height(&egui::TextStyle::Small);
    let (cw, ch) = gf::dims(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(egui::vec2(cw, ch + name_h + 2.0), Sense::hover());
    let id = ui.id().with(salt);
    let r = ui.interact(rect, id, if drag.is_some() { Sense::click_and_drag() } else { Sense::click() });
    #[cfg(test)]
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("gallery_card", name)), rect));
    let tile = Rect::from_min_size(rect.min, egui::vec2(cw, ch));
    let lifted = drag.is_some() && drag_lifted(ui, id);
    let p = if lifted {
        // paint into its own tooltip-order layer, then move that layer by how far the pointer travelled
        let layer = egui::LayerId::new(egui::Order::Tooltip, id);
        ui.painter().rect_filled(tile, 2.0, ui.visuals().extreme_bg_color.gamma_multiply(0.5));
        ui.painter().clone().with_layer_id(layer).with_clip_rect(rect.expand(3.0))
    } else {
        ui.painter().clone()
    };
    p.rect_filled(tile, 2.0, ui.visuals().extreme_bg_color);
    picture(&p, tile);
    let accent = ui.visuals().selection.stroke.color;
    let border = if r.hovered() || selected { accent } else { ui.visuals().widgets.noninteractive.bg_stroke.color };
    p.rect_stroke(tile, 2.0, Stroke::new(if r.hovered() || selected { 2.0 } else { 1.0 }, border), StrokeKind::Inside);
    let g = p.layout(name.to_string(), font, ui.visuals().text_color(), cw - 4.0);
    let tx = (rect.left() + (cw - g.size().x) / 2.0).max(rect.left());
    p.galley(egui::pos2(tx, tile.bottom() + 2.0), g, ui.visuals().text_color());
    if let (true, Some(payload)) = (lifted, drag) {
        let (now, origin) = ui.ctx().input(|i| (i.pointer.interact_pos(), i.pointer.press_origin()));
        if let (Some(now), Some(origin)) = (now, origin) {
            ui.ctx().transform_layer_shapes(p.layer_id(), egui::emath::TSTransform::from_translation(now - origin));
        }
        // re-set every frame of the drag: egui drops the payload on release, never mid-gesture
        egui::DragAndDrop::set_payload(ui.ctx(), payload);
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    }
    r.on_hover_text(name)
}

fn thumb_or_blank(p: &egui::Painter, tile: Rect, thumb: Option<(egui::TextureId, [u32; 2])>) {
    if let Some((tex, _)) = thumb {
        p.image(tex, tile, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
    }
}

fn swatch(p: &egui::Painter, tile: Rect, color: [u8; 4], outline: [u8; 4]) {
    let c = Color32::from_rgba_unmultiplied(color[0], color[1], color[2], 255);
    let o = Color32::from_rgba_unmultiplied(outline[0], outline[1], outline[2], 255);
    p.rect_filled(tile.shrink(10.0), 3.0, c);
    p.rect_stroke(tile.shrink(10.0), 3.0, Stroke::new(2.0, o), StrokeKind::Outside);
}

/// Card names for `tab`, builtins first - the same list `gallery.list` returns, minus JSON wrapping.
/// Looks excludes any `Settings.effect_presets` entry with `is_graph()==true` (a saved node-graph
/// preset is not a "Look"). `mut Settings` isn't needed here (read-only), but the caller already has
/// `&Settings`, not `&mut`, for every other tab too - kept uniform.
pub fn card_names(tab: GalleryTab, settings: &Settings) -> Vec<String> {
    match tab {
        GalleryTab::Looks => crate::engine::presets::builtin_looks()
            .into_iter()
            .map(|p| p.name)
            .chain(settings.effect_presets.iter().filter(|p| !p.is_graph()).map(|p| p.name.clone()))
            .collect(),
        // ---- ws:online-library ---- cached listing: the managed library folder, then `lut_dirs`
        GalleryTab::Luts => lut_files(settings),
        GalleryTab::Captions => crate::engine::presets::builtin_caption_styles()
            .into_iter()
            .map(|s| s.text)
            .chain(settings.caption_presets.iter().map(|s| s.text.clone()))
            .collect(),
        GalleryTab::SpeedRamps => crate::engine::presets::builtin_speed_ramps()
            .into_iter()
            .map(|r| r.name)
            .chain(mine_items(GKind::Speed, 0.0).iter().map(|i| i.name.clone()))
            .collect(),
        GalleryTab::Sounds => mine_items(GKind::Sound, 0.0).iter().map(|i| i.name.clone()).collect(),
        GalleryTab::Effects => crate::model::EffectKind::ALL
            .into_iter()
            .map(|k| k.name().to_string())
            .chain(mine_items(GKind::Effect, 0.0).iter().map(|i| i.name.clone()))
            .collect(),
        GalleryTab::Transitions => crate::model::TransitionKind::ALL
            .into_iter()
            .map(|k| k.name().to_string())
            .chain(mine_items(GKind::Transition, 0.0).iter().map(|i| i.name.clone()))
            .collect(),
        GalleryTab::Templates => settings.templates.iter().map(|t| t.name.clone()).collect(),
        // ---- ws:text-titles ----
        GalleryTab::Titles => crate::engine::presets::builtin_titles()
            .into_iter()
            .map(|t| t.name)
            .chain(
                settings
                    .templates
                    .iter()
                    .filter(|t| crate::engine::presets::is_text_template(t))
                    .map(|t| t.name.clone()),
            )
            .collect(),
        GalleryTab::Animate => crate::model::builtin_blocks()
            .into_iter()
            .map(|b| b.name)
            .chain(settings.key_blocks.iter().map(|b| b.name.clone()))
            .collect(),
    }
}

/// An Animate card's picture: each animated property's curve over the block, normalised to the tile,
/// in the block's bar colour (the one its timeline bar uses).
fn block_picture(p: &egui::Painter, tile: Rect, b: &crate::model::KeyBlock) {
    let [r, g, bl] = crate::model::block_color(&b.name);
    let col = Color32::from_rgb(r, g, bl);
    let mut c = Clip::new(0, crate::model::ClipKind::Video, "", 0.0, 1.0);
    b.write(&mut c, 0.0, 1.0, 1.0, 1.0);
    let area = tile.shrink(8.0);
    for tr in &b.tracks {
        // volume blocks skip a video clip - preview them on an audio one
        let mut ca;
        let src = if tr.prop == "volume" {
            ca = Clip::new(0, crate::model::ClipKind::Audio, "", 0.0, 1.0);
            b.write(&mut ca, 0.0, 1.0, 1.0, 1.0);
            &ca
        } else {
            &c
        };
        let Some(a) = crate::model::prop_ref(src, &tr.prop) else { continue };
        let v: Vec<f64> = (0..=32).map(|i| a.at(i as f64 / 32.0)).collect();
        let (lo, hi) = v.iter().fold((f64::MAX, f64::MIN), |(l, h), &x| (l.min(x), h.max(x)));
        let pts = v
            .iter()
            .enumerate()
            .map(|(i, &x)| {
                let fy = if hi - lo > 1e-9 { ((x - lo) / (hi - lo)) as f32 } else { 0.5 };
                egui::pos2(area.left() + area.width() * i as f32 / 32.0, area.bottom() - fy * area.height())
            })
            .collect();
        p.add(egui::Shape::line(pts, Stroke::new(1.5, col)));
    }
}

/// The pane's one scroll area: vertical only (the grid wraps to the pane's width, so there is never a
/// sideways scroll bar), always as big as the pane. Tests may park it at an offset (`gallery_scroll_to`).
fn scroll_area(ui: &egui::Ui) -> egui::ScrollArea {
    let area = egui::ScrollArea::vertical().auto_shrink([false, false]);
    #[cfg(test)]
    let area = match ui.ctx().data(|d| d.get_temp::<f32>(egui::Id::new("gallery_scroll_to"))) {
        Some(y) => area.vertical_scroll_offset(y),
        None => area,
    };
    let _ = ui;
    area
}

/// Test-only: the scroll area's viewport and content size, for the "every card is reachable" checks.
#[cfg(test)]
fn note_scroll(ctx: &egui::Context, s: &egui::scroll_area::ScrollAreaOutput<()>) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("gallery_scroll"), (s.inner_rect, s.content_size)));
}

/// Right-click on empty space: save something of yours.
fn bg_menu(bg: &Response, out: &mut GalleryResponse) {
    menu::context(bg, |ui| {
        if menu::row(ui, None, "Save to Gallery…", "").clicked() {
            out.save_dialog = true;
        }
    });
}

/// The Gallery pane: the tab strip (pinned), then the card grid and - after a Titles card was placed -
/// its Customize rows, scrolling together. A card click applies it (Templates / Titles place at the
/// playhead); its right-click offers the same, and the viewer previews the card while that menu is open.
/// Right-click empty space for "Save look from selection…". Effects and Transitions are tabs of their
/// own (built-in and library items in one grid, one search) - the old Effects / Transitions panes are
/// gone. Every card except a skill can also be dragged onto the timeline (see `card`).
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    state: &mut GalleryState,
    settings: &mut Settings,
    project: &crate::model::Project,
    selection: &[Id],
    palette: &Palette,
    tr: &mut transitions_ui::TransitionsState,
    lib: Option<LibView>,
) -> GalleryResponse {
    use crate::ui::DragPayload;
    let mut out = GalleryResponse::default();
    let bg = crate::ui::markers_ui::menu_area(ui);
    // the tabs wrap onto a second row in a narrow pane (a sideways-scrolling row hid the library tabs)
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for tab in GalleryTab::ALL {
            if ui.selectable_label(state.lib.is_none() && state.tab == tab, tab.label()).clicked() {
                state.tab = tab;
                state.lib = None;
                settings.gallery_tab = tab.name().to_string();
            }
        }
        // ---- ws:online-library ----
        for (kind, label) in LIB_TABS {
            if ui.selectable_label(state.lib == Some(kind), label).clicked() {
                state.lib = Some(kind);
            }
        }
    });
    ui.separator();
    // a new tab starts unfiltered (the search, chip and category of the old one mean nothing here)
    let key = (state.tab, state.lib);
    if state.seen != Some(key) {
        if state.seen.is_some() {
            state.f.clear();
        }
        state.seen = Some(key);
    }
    gf::set_size(ui.ctx(), state.f.size);
    let now = ui.input(|i| i.time);
    let lib_tab = state.lib.is_some();
    let mut lib = lib;
    // "Online" is a chip only once the library is on (until then the banner offers it)
    let on = lib.as_ref().is_some_and(|l| l.connected);
    let srcs: Vec<Src> = sources(state.tab, lib_tab).iter().copied().filter(|s| *s != Src::Online || on).collect();
    let cats = tab_cats(state.tab, lib.as_ref(), lib_tab, now);
    if !srcs.contains(&state.f.src) {
        state.f.src = Src::All;
    }
    if !state.f.cat.is_empty() && !cats.iter().any(|c| c.eq_ignore_ascii_case(&state.f.cat)) {
        state.f.cat.clear();
    }
    let folder = if lib_tab { None } else { mine_kind(state.tab) };
    header(ui, state, &srcs, &cats, lib.as_ref(), folder, state.tab == GalleryTab::Sounds && !lib_tab, &mut out);
    ui.separator();
    state.count_acc = (0, 0);
    // ---- ws:online-library ---- a library tab, or the library's LUTs among the Luts tab's cards
    if lib_tab {
        let _scroll = scroll_area(ui).show(ui, |ui| {
            if let Some(lib) = lib.as_mut() {
                library_grid(ui, state, lib, settings, &mut out);
            }
        });
        state.count = state.count_acc;
        #[cfg(test)]
        note_scroll(ui.ctx(), &_scroll);
        return out;
    }
    let lib_luts = crate::engine::library::root().join("luts");
    // (Effects / Transitions / Sounds filter and draw their own entries)
    let own = matches!(state.tab, GalleryTab::Effects | GalleryTab::Transitions | GalleryTab::Sounds);
    let mut names = if own { Vec::new() } else { card_names(state.tab, settings) };
    if state.tab == GalleryTab::Luts {
        names.retain(|n| !std::path::Path::new(n).starts_with(&lib_luts)); // drawn from the library's cells
    }
    // search / chip / category / sort over the names (a LUT is searched by its file name)
    let tab = state.tab;
    let total_names = names.len();
    {
        let f = state.f.clone();
        let mut rows: Vec<(String, String, Src, Vec<String>, u64)> = names
            .drain(..)
            .map(|n| {
                let short = if tab == GalleryTab::Luts {
                    std::path::Path::new(&n).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or(n.clone())
                } else {
                    n.clone()
                };
                let origin =
                    if tab == GalleryTab::Luts || is_mine(tab, &n, settings, now) { Src::Mine } else { Src::BuiltIn };
                let (tags, created) = mine_meta(tab, &n, now);
                (n, short, origin, tags, created)
            })
            .filter(|(_, short, origin, tags, _)| {
                let t: Vec<&str> = tags.iter().map(String::as_str).collect();
                f.passes(*origin, &t, &format!("{short} {}", tags.join(" ")))
            })
            .collect();
        gf::sort_by(&mut rows, f.sort, |r| (r.1.clone(), used_of(settings, tab.name(), &r.0), r.4));
        names = rows.into_iter().map(|r| r.0).collect();
    }
    state.count_acc.0 += names.len();
    state.count_acc.1 += total_names;
    let _scroll = scroll_area(ui).show(ui, |ui| {
        match state.tab {
            GalleryTab::Effects => {
                return effects_tab(ui, state, project, selection, palette, lib.as_mut(), settings, &mut out)
            }
            GalleryTab::Transitions => {
                return transitions_tab(ui, state, project, selection, palette, tr, lib.as_mut(), settings, &mut out)
            }
            GalleryTab::Sounds => return sounds_tab(ui, state, palette, settings, &mut out),
            _ => {}
        }
        // the library's LUTs ride in the same grid
        let (mut lut_cells, mut lut_thumb): (
            Vec<&LibCell>,
            Option<&mut dyn FnMut(&str) -> Option<(egui::TextureId, [u32; 2])>>,
        ) = (Vec::new(), None);
        if tab == GalleryTab::Luts {
            if let Some(l) = lib.as_mut() {
                let LibView { cells, thumb, .. } = l;
                lut_cells = filtered_cells(state, cells, settings, None);
                lut_thumb = Some(&mut **thumb);
            }
        }
        if names.is_empty() && lut_cells.is_empty() && !matches!(tab, GalleryTab::Sounds) {
            ui.weak(if total_names == 0 && state.count_acc.1 == 0 {
                match tab {
                    GalleryTab::Luts => {
                        "No .cube files found. Add a folder in Settings ▸ Color, or browse the online library."
                    }
                    GalleryTab::Templates => "No saved templates yet.",
                    GalleryTab::Titles => "No title templates yet.", // builtin_titles() is never empty; user-only edge case
                    _ => "Nothing here yet.",
                }
            } else {
                "Nothing matches."
            });
        }
        gf::grid(ui, |ui| {
            if let Some(t) = lut_thumb.as_deref_mut() {
                for c in lut_cells.drain(..) {
                    lib_cell(ui, c, t, false, None, &mut out);
                }
            }
            for name in &names {
                let r = match tab {
                    GalleryTab::Looks => {
                        let idx = crate::engine::presets::builtin_looks().iter().position(|p| &p.name == name);
                        // (a saved look has no rendering of its own: its first effect's thumbnail stands in)
                        let thumb = idx.and_then(|i| thumbnail(&ThumbSource::Look(i))).or_else(|| {
                            let p = settings.effect_presets.iter().find(|p| &p.name == name)?;
                            let fx: Vec<crate::model::Effect> = serde_json::from_str(&p.json).ok()?;
                            crate::ui::effects_ui::thumbnail(fx.first()?.kind)
                        });
                        let drag = DragPayload::GalleryCard(tab, name.clone());
                        card(ui, ("look", name), name, Some(drag), false, |p, t| thumb_or_blank(p, t, thumb))
                    }
                    GalleryTab::Luts => {
                        let thumb = thumbnail(&ThumbSource::Lut(std::path::PathBuf::from(name)));
                        let short = std::path::Path::new(name)
                            .file_stem()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_else(|| name.clone());
                        let drag = DragPayload::GalleryCard(tab, name.clone());
                        card(ui, ("lut", name), &short, Some(drag), false, |p, t| thumb_or_blank(p, t, thumb))
                    }
                    GalleryTab::Captions => {
                        let style = crate::engine::presets::builtin_caption_styles()
                            .into_iter()
                            .chain(settings.caption_presets.iter().cloned())
                            .find(|s| &s.text == name);
                        let drag = DragPayload::GalleryCard(tab, name.clone());
                        card(ui, ("cap", name), name, Some(drag), false, |p, t| {
                            if let Some(s) = &style {
                                swatch(p, t, s.color, s.outline_color);
                            }
                        })
                    }
                    GalleryTab::Animate => {
                        let b = crate::model::find_block(&settings.key_blocks, name);
                        let drag = DragPayload::KeyBlock(name.clone());
                        card(ui, ("blk", name), name, Some(drag), false, |p, t| {
                            if let Some(b) = &b {
                                block_picture(p, t, b);
                            }
                        })
                        .on_hover_text(format!("{name} - click: at the playhead · drag onto a clip"))
                    }
                    GalleryTab::Templates => {
                        card(ui, ("tpl", name), name, Some(DragPayload::Template(name.clone())), false, |_, _| {})
                    }
                    _ => {
                        let drag = DragPayload::GalleryCard(tab, name.clone());
                        card(ui, ("card", tab.name(), name), name, Some(drag), false, |_, _| {})
                    }
                };
                let mine = is_mine(tab, name, settings, now);
                if mine {
                    mine_badge(
                        ui.painter(),
                        Rect::from_min_size(r.rect.min, egui::vec2(gf::dims(ui.ctx()).0, gf::dims(ui.ctx()).1)),
                        palette.accent,
                    );
                }
                if tab != GalleryTab::Animate && crate::ui::hover_after(ui, r.id, &r, 150.0) {
                    out.hover = Some((tab, name.clone()));
                }
                let mut go = r.clicked();
                let menu = menu::context(&r, |ui| {
                    let (label, needs_clip) = match tab {
                        GalleryTab::Templates | GalleryTab::Titles => ("Place at playhead", false),
                        GalleryTab::Captions => ("Apply to every caption", false),
                        _ => ("Apply to selection", true),
                    };
                    let ok = !needs_clip || !selection.is_empty();
                    let r = ui.add_enabled_ui(ok, |ui| menu::row(ui, None, label, "")).inner;
                    go |= r.on_disabled_hover_text("Select a clip first").clicked();
                    if let (true, Some(k)) = (mine, mine_kind(tab)) {
                        mine_rows(ui, &mut out, k, name);
                    }
                });
                // Preview: the viewer shows the card for as long as its menu is open (hover does the same)
                if menu.is_some() && !matches!(tab, GalleryTab::Templates | GalleryTab::Titles | GalleryTab::Animate) {
                    out.hover = Some((tab, name.clone()));
                }
                if go {
                    match tab {
                        GalleryTab::Templates => out.place = Some(name.clone()),
                        GalleryTab::Titles => out.place_title = Some(name.clone()),
                        _ => out.apply = Some((tab, name.clone(), 1.0)),
                    }
                }
            }
        });
        // ---- ws:text-titles ----: the placed title's exposed fields, right under the cards
        if !state.customize.is_empty() {
            ui.separator();
            ui.strong("Customize");
            // edit CLONES, never the live project while drawing, so the caller's undo can snapshot first
            for (cid, field) in &state.customize {
                if let Some(orig) = project.clip(*cid) {
                    let mut clone = orig.clone();
                    if template_field_widget(ui, field, &mut clone) {
                        out.customized.push((*cid, clone));
                    }
                }
            }
        }
    });
    state.count = state.count_acc;
    #[cfg(test)]
    note_scroll(ui.ctx(), &_scroll);
    bg_menu(&bg, &mut out);
    out
}

// ---- ws:text-titles ----
/// One editable row for a `Clip.exposed` field name, the Titles-tab Customize panel's whole surface (the
/// caller, `ui::app::gallery_ctl::draw`, renders one of these per `GalleryState.customize` entry below
/// the card grid - this fn has no `&mut Project`, only the one clip it's editing). 4 hardcoded field
/// kinds - text.text / text.color / text.size / shape.fill - cover every field the 3 builtin templates
/// expose; a generic/reflective exposed-field editor is explicitly out of scope (see the plan's
/// deliberate-simplifications note) until a template needs a 5th kind. Returns true if the caller should
/// push undo.
pub(crate) fn template_field_widget(ui: &mut egui::Ui, field: &str, clip: &mut Clip) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(field);
        match field {
            "text.text" => {
                if let Some(t) = clip.text.as_mut() {
                    changed |= ui.text_edit_singleline(&mut t.text).changed();
                }
            }
            "text.color" => {
                if let Some(t) = clip.text.as_mut() {
                    changed |= ui.color_edit_button_srgba_unmultiplied(&mut t.color).changed();
                }
            }
            "text.size" => {
                if let Some(t) = clip.text.as_mut() {
                    let mut v = t.size.value;
                    if ui.add(egui::DragValue::new(&mut v).range(1.0..=1000.0)).changed() {
                        t.size.value = v;
                        changed = true;
                    }
                }
            }
            "shape.fill" => {
                if let Some(s) = clip.shape.as_mut() {
                    changed |= ui.color_edit_button_srgba_unmultiplied(&mut s.fill).changed();
                }
            }
            _ => {
                ui.weak(format!("(unknown field '{field}')"));
            }
        }
    });
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::EffectPreset;

    fn ctx() -> egui::Context {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        ctx
    }

    fn run(ctx: &egui::Context, state: &mut GalleryState, settings: &mut Settings) -> GalleryResponse {
        let palette = Palette::new(true, Color32::WHITE);
        let mut out = GalleryResponse::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                out = show(
                    ui,
                    state,
                    settings,
                    &crate::model::Project::new(),
                    &[7],
                    &palette,
                    &mut Default::default(),
                    None,
                );
            });
        });
        out
    }

    // ---- ws:online-library ----
    fn run_lib(ctx: &egui::Context, state: &mut GalleryState, settings: &mut Settings, asked: &mut Vec<String>) {
        let palette = Palette::new(true, Color32::WHITE);
        let cells = || {
            (0..40)
                .map(|i| LibCell {
                    kind: crate::engine::library::Kind::Effect,
                    id: format!("effects/e{i}"),
                    name: format!("Effect {i}"),
                    desc: String::new(),
                    installed: i % 2 == 0,
                    busy: None,
                    skill: false,
                    tags: Vec::new(),
                    yours: false,
                })
                .collect()
        };
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0))),
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let mut thumb = |id: &str| -> Option<(egui::TextureId, [u32; 2])> {
                    asked.push(id.to_string());
                    None
                };
                let lib = LibView {
                    cells: cells(),
                    local: true,
                    note: String::new(),
                    loading: false,
                    connected: true,
                    thumb: &mut thumb,
                };
                show(
                    ui,
                    state,
                    settings,
                    &crate::model::Project::new(),
                    &[7],
                    &palette,
                    &mut Default::default(),
                    Some(lib),
                );
            });
        });
    }

    // ---- every card wraps to the pane and the pane scrolls to all of them ----
    const MANY: usize = 120;

    /// One Gallery frame in a pane `w` x `h`: `wheel` is a scroll delta played this frame, `t` the clock.
    fn frame(
        ctx: &egui::Context,
        state: &mut GalleryState,
        settings: &mut Settings,
        w: f32,
        h: f32,
        wheel: f32,
        t: f64,
    ) {
        let palette = Palette::new(true, Color32::WHITE);
        let mut events = vec![egui::Event::PointerMoved(egui::pos2(w / 2.0, h / 2.0))];
        if wheel != 0.0 {
            events.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, wheel),
                modifiers: egui::Modifiers::NONE,
            });
        }
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, h))),
            time: Some(t),
            events,
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let mut thumb = |_: &str| -> Option<(egui::TextureId, [u32; 2])> { None };
                let lib = (state.lib.is_some()
                    || matches!(state.tab, GalleryTab::Effects | GalleryTab::Transitions | GalleryTab::Luts))
                .then(|| LibView {
                    cells: (0..MANY)
                        .map(|i| LibCell {
                            kind: crate::engine::library::Kind::Effect,
                            id: format!("effects/e{i}"),
                            name: format!("Item {i}"),
                            desc: String::new(),
                            installed: i % 2 == 0,
                            busy: None,
                            skill: false,
                            tags: Vec::new(),
                            yours: false,
                        })
                        .collect(),
                    local: true,
                    note: String::new(),
                    loading: false,
                    connected: true,
                    thumb: &mut thumb,
                });
                show(ui, state, settings, &crate::model::Project::new(), &[], &palette, &mut Default::default(), lib);
            });
        });
    }

    /// Settings with `MANY` of everything a tab lists.
    fn crowded() -> Settings {
        let mut s = Settings::default();
        for i in 0..MANY {
            s.effect_presets.push(EffectPreset { name: format!("Item {i}"), json: "[]".into() });
            let mut b = crate::model::builtin_blocks().remove(0);
            b.name = format!("Item {i}");
            s.key_blocks.push(b);
            let mut c = crate::engine::presets::builtin_caption_styles().remove(0);
            c.text = format!("Item {i}");
            s.caption_presets.push(c);
        }
        s
    }

    /// The card rect `show` recorded for `name`, and the scroll area's (viewport, content size).
    fn card_rect(ctx: &egui::Context, name: &str) -> Rect {
        ctx.data(|d| d.get_temp(egui::Id::new(("gallery_card", name)))).unwrap_or_else(|| panic!("no card {name}"))
    }
    fn scroll(ctx: &egui::Context) -> (Rect, egui::Vec2) {
        ctx.data(|d| d.get_temp(egui::Id::new("gallery_scroll"))).expect("scroll area recorded")
    }

    /// Whatever the tab and the pane's size: every card sits inside the pane's width, the cards wrap into
    /// rows (not one long row), the scroll area's content covers every row, and scrolling to the end brings
    /// the last card into the viewport while the tab strip stays where it is.
    #[test]
    fn every_tab_wraps_cards_to_the_pane_and_scrolls_to_the_last_one() {
        for (tab, lib) in [
            (GalleryTab::Looks, None),
            (GalleryTab::Animate, None),
            (GalleryTab::Captions, None),
            (GalleryTab::Looks, Some(crate::engine::library::Kind::Html)),
            (GalleryTab::Effects, None),
            (GalleryTab::Transitions, None),
            (GalleryTab::Luts, None),
        ] {
            for w in [120.0, 205.0, 350.0, 700.0, 1400.0] {
                let h = 900.0; // (the tab strip and the header wrap to many rows in a narrow pane; a card must fit the viewport whole)
                let ctx = ctx();
                let mut state = GalleryState { tab, lib, ..Default::default() };
                let mut settings = crowded();
                let mut t = 0.0;
                for _ in 0..3 {
                    frame(&ctx, &mut state, &mut settings, w, h, 0.0, t);
                    t += 0.016;
                }
                let at = format!("{tab:?}/{lib:?} at {w} pt");
                let (view, content) = scroll(&ctx);
                let (first, last) = (card_rect(&ctx, "Item 0"), card_rect(&ctx, &format!("Item {}", MANY - 1)));
                for i in 0..MANY {
                    let r = card_rect(&ctx, &format!("Item {i}"));
                    assert!(r.left() >= -0.5 && r.right() <= w + 0.5, "{at}: card {i} leaves the pane: {r:?}");
                }
                let (cw, ch) = (first.width(), first.height() - 14.0);
                let per_row = ((w / cw).floor() as usize).max(1);
                let rows = MANY.div_ceil(per_row) as f32;
                assert!(content.x <= w + 0.5, "{at}: content is {} pt wide", content.x);
                // the first row reaches the right edge of the pane (whole columns, stretched to fill it)
                let right = (0..MANY)
                    .map(|i| card_rect(&ctx, &format!("Item {i}")))
                    .filter(|r| (r.top() - first.top()).abs() < 1.0)
                    .map(|r| r.right())
                    .fold(0.0, f32::max);
                assert!(
                    right >= w - 8.0 - 1.5 || cw >= 144.0 - 0.5,
                    "{at}: the first row stops at {right}, the pane is {w} wide"
                );
                assert!(last.top() > first.top() + (rows - 2.0).max(0.0) * ch, "{at}: not wrapped into rows");
                assert!(content.y >= last.bottom() - view.top() - 4.0, "{at}: content height misses the last row");
                // scroll to the very end: the largest offset the content allows
                ctx.data_mut(|d| {
                    d.insert_temp(egui::Id::new("gallery_scroll_to"), (content.y - view.height()).max(0.0))
                });
                for _ in 0..3 {
                    frame(&ctx, &mut state, &mut settings, w, h, 0.0, t);
                    t += 0.016;
                }
                let (view2, _) = scroll(&ctx);
                let last = card_rect(&ctx, &format!("Item {}", MANY - 1));
                assert_eq!(view2, view, "{at}: the viewport (tab strip pinned above it) moved");
                assert!(
                    last.bottom() <= view2.bottom() + 1.0 && last.top() >= view2.top() - 1.0,
                    "{at}: the last card is not reachable: {last:?} vs viewport {view2:?}"
                );
            }
        }
    }

    // ---- Effects / Transitions tabs, drag sources ----
    use crate::engine::library::Kind;
    use crate::model::{Clip, ClipKind};
    use crate::ui::DragPayload;
    use egui::{Event, Modifiers, PointerButton, Pos2};

    /// A Gallery pane 400 x 700 with a project, a selection and library cells, driven frame by frame.
    struct Gx {
        ctx: egui::Context,
        state: GalleryState,
        settings: Settings,
        tr: transitions_ui::TransitionsState,
        project: crate::model::Project,
        selection: Vec<Id>,
        cells: Vec<(&'static str, &'static str, Kind, bool)>, // (id, name, kind, installed)
        /// The online library is on (the "Browse online library" banner is gone).
        connected: bool,
        t: f64,
        texts: Vec<String>,
        items: Vec<(String, Pos2)>,
        /// What the frames since the last `take()` asked for.
        got: GalleryResponse,
    }

    impl Gx {
        fn new(tab: GalleryTab) -> Self {
            let mut project = crate::model::Project::new();
            for (id, start, dur) in [(101, 0.0, 5.0), (102, 5.0, 5.0), (103, 20.0, 4.0)] {
                project.tracks[0].clips.push(Clip::new(id, ClipKind::Video, "v", start, dur));
            }
            Self {
                ctx: ctx(),
                state: GalleryState { tab, ..Default::default() },
                settings: Settings::default(),
                tr: Default::default(),
                project,
                selection: Vec::new(),
                cells: Vec::new(),
                connected: true,
                t: 0.0,
                texts: Vec::new(),
                items: Vec::new(),
                got: GalleryResponse::default(),
            }
        }
        fn frame(&mut self, events: Vec<Event>) {
            self.t += 0.05;
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 700.0))),
                time: Some(self.t),
                events,
                ..Default::default()
            };
            let palette = Palette::new(true, Color32::WHITE);
            let Gx { ctx, state, settings, tr, project, selection, cells, got, connected, .. } = self;
            let connected = *connected;
            let kind_tab = matches!(state.tab, GalleryTab::Effects | GalleryTab::Transitions | GalleryTab::Luts)
                || state.lib.is_some();
            let mut thumb = |_: &str| -> Option<(egui::TextureId, [u32; 2])> { None };
            let full = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let lib = kind_tab.then(|| LibView {
                        cells: cells
                            .iter()
                            .map(|&(id, name, kind, installed)| LibCell {
                                id: id.into(),
                                kind,
                                name: name.into(),
                                desc: String::new(),
                                installed,
                                busy: None,
                                skill: kind == Kind::Skill,
                                tags: Vec::new(),
                                yours: false,
                            })
                            .collect(),
                        local: true,
                        note: String::new(),
                        loading: false,
                        connected,
                        thumb: &mut thumb,
                    });
                    let o = show(ui, state, settings, project, selection, &palette, tr, lib);
                    got.apply = got.apply.take().or(o.apply);
                    got.apply_where = got.apply_where.or(o.apply_where);
                    got.lib = got.lib.take().or(o.lib);
                    got.place = got.place.take().or(o.place);
                    got.place_title = got.place_title.take().or(o.place_title);
                    got.mine = got.mine.take().or(o.mine);
                    got.sound = got.sound.take().or(o.sound);
                    got.import = got.import.or(o.import);
                    got.open_folder = got.open_folder.or(o.open_folder);
                    got.save_dialog |= o.save_dialog;
                });
            });
            fn texts(s: &egui::epaint::Shape, out: &mut Vec<(String, Pos2)>) {
                match s {
                    egui::epaint::Shape::Text(t) => {
                        out.push((t.galley.text().to_string(), t.visual_bounding_rect().center()))
                    }
                    egui::epaint::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
                    _ => {}
                }
            }
            let mut items = Vec::new();
            full.shapes.iter().for_each(|c| texts(&c.shape, &mut items));
            self.items = items;
            self.texts = self.items.iter().map(|(t, _)| t.clone()).collect();
        }
        fn take(&mut self) -> GalleryResponse {
            std::mem::take(&mut self.got)
        }
        fn rect(&self, name: &str) -> Rect {
            card_rect(&self.ctx, name)
        }
        fn mouse(&mut self, pos: Pos2, button: PointerButton, pressed: bool) {
            self.frame(vec![Event::PointerButton { pos, button, pressed, modifiers: Modifiers::NONE }]);
        }
        /// A click on the card `name`.
        fn click(&mut self, name: &str) {
            self.frame(vec![]);
            let at = self.rect(name).center();
            self.frame(vec![Event::PointerMoved(at)]);
            self.mouse(at, PointerButton::Primary, true);
            self.mouse(at, PointerButton::Primary, false);
            self.frame(vec![]);
        }
        /// Right-click the card `name` and let its menu draw.
        fn open_menu(&mut self, name: &str) {
            self.frame(vec![]);
            let at = self.rect(name).center();
            self.frame(vec![Event::PointerMoved(at)]);
            self.mouse(at, PointerButton::Secondary, true);
            self.mouse(at, PointerButton::Secondary, false);
            self.frame(vec![]);
        }
        fn has_text(&self, s: &str) -> bool {
            self.texts.iter().any(|t| t == s)
        }
        /// Click the text `label` (a button of the header) and let what it opens draw.
        fn click_text(&mut self, label: &str) {
            self.frame(vec![]);
            let pos = self.text_pos(label).unwrap_or_else(|| panic!("no '{label}' in {:?}", self.texts));
            self.frame(vec![Event::PointerMoved(pos)]);
            self.mouse(pos, PointerButton::Primary, true);
            self.mouse(pos, PointerButton::Primary, false);
            self.frame(vec![]);
        }
        /// Click the menu row reading `label`.
        fn pick(&mut self, label: &str) {
            let pos = self.text_pos(label).unwrap_or_else(|| panic!("no menu row '{label}' in {:?}", self.texts));
            self.t += 1.0;
            self.frame(vec![Event::PointerMoved(pos)]);
            self.mouse(pos, PointerButton::Primary, true);
            self.mouse(pos, PointerButton::Primary, false);
            self.frame(vec![]);
        }
        fn text_pos(&self, label: &str) -> Option<Pos2> {
            self.items.iter().rev().find(|(t, _)| t == label).map(|(_, p)| *p)
        }
        fn payload(&self) -> Option<DragPayload> {
            egui::DragAndDrop::payload::<DragPayload>(&self.ctx).map(|p| (*p).clone())
        }
        /// Press on `name`, then move `dist` points away (the drag), leaving the button down.
        fn drag(&mut self, name: &str, dist: f32) -> Pos2 {
            self.frame(vec![]);
            let at = self.rect(name).center();
            self.frame(vec![Event::PointerMoved(at)]);
            self.mouse(at, PointerButton::Primary, true);
            let to = at + egui::vec2(dist, dist);
            self.frame(vec![Event::PointerMoved(to)]);
            self.frame(vec![Event::PointerMoved(to + egui::vec2(1.0, 1.0))]);
            to
        }
    }

    #[test]
    fn effects_and_transitions_are_tabs_of_their_own() {
        use crate::engine::library::Kind;
        assert!(GalleryTab::ALL.contains(&GalleryTab::Effects) && GalleryTab::ALL.contains(&GalleryTab::Transitions));
        let lib_kinds: Vec<Kind> = LIB_TABS.iter().map(|(k, _)| *k).collect();
        assert!(
            !lib_kinds.contains(&Kind::Effect) && !lib_kinds.contains(&Kind::Transition),
            "one tab each, no second list of the same things: {lib_kinds:?}"
        );
        let mut g = Gx::new(GalleryTab::Looks);
        g.frame(vec![]);
        for label in ["Effects", "Transitions"] {
            assert_eq!(g.texts.iter().filter(|t| *t == label).count(), 1, "one {label} tab in the strip");
        }
        // selecting them: click the label, the tab follows
        let at = g.text_pos("Effects").expect("tab");
        g.mouse(at, PointerButton::Primary, true);
        g.mouse(at, PointerButton::Primary, false);
        g.frame(vec![]);
        assert_eq!(g.state.tab, GalleryTab::Effects);
        assert!(g.has_text("Blur") || g.has_text("Adjustments"), "the built-in effects are listed");
        let at = g.text_pos("Transitions").expect("tab");
        g.mouse(at, PointerButton::Primary, true);
        g.mouse(at, PointerButton::Primary, false);
        g.frame(vec![]);
        assert_eq!(g.state.tab, GalleryTab::Transitions);
        assert!(g.has_text("Cross Fade") && g.has_text("Duration"), "built-in transitions and the defaults row");
    }

    #[test]
    fn effects_tab_lists_built_ins_and_library_items_together() {
        let mut g = Gx::new(GalleryTab::Effects);
        g.cells = vec![("effects/glow", "Glow", Kind::Effect, true), ("effects/rain", "Rain", Kind::Effect, false)];
        g.frame(vec![]);
        g.frame(vec![]);
        // all three are cards of the ONE grid: no "Library" section, no second list
        let (blur, glow, rain) = (g.rect("Blur"), g.rect("Glow"), g.rect("Rain"));
        assert!(glow.top() >= blur.top() && rain.top() >= glow.top() - 1.0, "library cards follow the built-ins");
        assert!(!g.has_text("Library"), "no separate library header: {:?}", g.texts);
    }

    #[test]
    fn one_search_covers_built_in_and_library_cards() {
        let mut g = Gx::new(GalleryTab::Effects);
        g.cells =
            vec![("effects/blurry", "Blurry Glow", Kind::Effect, true), ("effects/rain", "Rain", Kind::Effect, false)];
        g.state.f.q = "blur".into();
        g.frame(vec![]);
        let _ = g.rect("Blur");
        let _ = g.rect("Blurry Glow");
        assert!(!g.has_text("Vignette"), "built-ins that do not match are gone");
        assert!(!g.has_text("Rain"), "library items that do not match are gone");
        g.state.f.q = "zzzz".into();
        g.frame(vec![]);
        assert!(g.has_text("No effect matches"), "nothing matches: one hint");
    }

    #[test]
    fn effect_card_click_asks_to_add_it_to_the_selection() {
        let mut g = Gx::new(GalleryTab::Effects);
        g.selection = vec![101];
        g.click("Blur");
        let out = g.take();
        assert_eq!(out.apply, Some((GalleryTab::Effects, "Blur".into(), 1.0)));
        assert_eq!(out.apply_where, None);
        // the quick action reaches the whole track
        g.open_menu("Blur");
        assert!(g.has_text("Add to selected clip(s)") && g.has_text("Add to all clips on this track"));
        g.pick("Add to all clips on this track");
        let out = g.take();
        assert_eq!(out.apply.map(|a| a.1), Some("Blur".into()));
        assert_eq!(out.apply_where, Some("track"));
    }

    #[test]
    fn an_audio_selection_is_offered_no_pixel_effects() {
        let mut g = Gx::new(GalleryTab::Effects);
        g.project.tracks[1].clips.push(Clip::new(301, ClipKind::Audio, "a", 0.0, 3.0));
        g.selection = vec![301];
        g.frame(vec![]);
        assert!(g.has_text("No audio effects yet"));
        assert!(!g.has_text("Blur"));
    }

    #[test]
    fn transition_click_applies_at_the_start_of_the_selection_or_just_picks() {
        let mut g = Gx::new(GalleryTab::Transitions);
        g.click("Wipe");
        let out = g.take();
        assert_eq!(out.apply, None, "nothing selected: a click only picks the card");
        assert_eq!(g.tr.kind(), crate::model::TransitionKind::Wipe, "Ctrl+T and the drag use it");
        g.selection = vec![102];
        g.click("Push");
        let out = g.take();
        assert_eq!(out.apply, Some((GalleryTab::Transitions, "Push".into(), 1.0)));
        assert_eq!(out.apply_where, Some("all"), "the cut at its start, else a fade-in");
    }

    /// The right-click menu of a transition card, for every kind of selection.
    #[test]
    fn transition_menu_rows_follow_the_selection() {
        // (selection, rows present, rows absent)
        let cases: [(&[Id], &[&str], &[&str]); 5] = [
            (&[], &["Apply to Start", "Apply to End"], &["Apply to Connection", "Apply to All"]),
            (&[103], &["Apply to Start", "Apply to End"], &["Apply to Connection", "Apply to All"]),
            (&[101], &["Apply to Start", "Apply to End", "Apply to Connection"], &["Apply to All"]),
            (&[102], &["Apply to Start", "Apply to End", "Apply to Connection"], &["Apply to All"]),
            (&[101, 103], &["Apply to Start", "Apply to End", "Apply to Connection", "Apply to All"], &[]),
        ];
        for (sel, present, absent) in cases {
            let mut g = Gx::new(GalleryTab::Transitions);
            g.selection = sel.to_vec();
            g.open_menu("Cross Fade");
            for row in present {
                assert!(g.has_text(row), "{sel:?}: missing '{row}' in {:?}", g.texts);
            }
            for row in absent {
                assert!(!g.has_text(row), "{sel:?}: '{row}' should not be there");
            }
        }
        // connected on both sides: the connection row splits in two
        let mut g = Gx::new(GalleryTab::Transitions);
        g.project.tracks[0].clips.push(Clip::new(104, ClipKind::Video, "v", 10.0, 5.0));
        g.project.tracks[0].sort();
        g.selection = vec![102];
        g.open_menu("Cross Fade");
        assert!(g.has_text("Apply to Connection (start)") && g.has_text("Apply to Connection (end)"));
        assert!(!g.has_text("Apply to Connection"), "never both the split rows and the single one");
    }

    #[test]
    fn transition_menu_rows_say_where() {
        for (row, want) in [
            ("Apply to Start", "start"),
            ("Apply to End", "end"),
            ("Apply to Connection", "connection_start"),
            ("Apply to All", "all"),
        ] {
            let mut g = Gx::new(GalleryTab::Transitions);
            g.selection = vec![102, 103];
            g.open_menu("Wipe");
            g.pick(row);
            let out = g.take();
            assert_eq!(out.apply, Some((GalleryTab::Transitions, "Wipe".into(), 1.0)), "{row}");
            assert_eq!(out.apply_where, Some(want), "{row}");
        }
    }

    #[test]
    fn a_disabled_transition_row_does_nothing() {
        let mut g = Gx::new(GalleryTab::Transitions);
        g.open_menu("Cross Fade");
        assert!(g.has_text("Apply to Start"), "greyed out, not hidden");
        g.pick("Apply to Start");
        assert_eq!(g.take().apply, None, "nothing selected: nothing to apply");
    }

    /// A library transition (installed or not) gets the same rows; an online one installs first, which the
    /// App does when it sees `ApplyAt`.
    #[test]
    fn library_transition_menu_has_the_apply_rows() {
        let mut g = Gx::new(GalleryTab::Transitions);
        g.cells = vec![("transitions/zoom", "Zoom Blur", Kind::Transition, false)];
        g.selection = vec![101];
        g.open_menu("Zoom Blur");
        for row in ["Apply to Start", "Apply to End", "Apply to Connection", "Install", "Apply"] {
            assert!(g.has_text(row), "'{row}' in {:?}", g.texts);
        }
        g.pick("Apply to End");
        assert_eq!(g.take().lib, Some(LibAct::ApplyAt("transitions/zoom".into(), "end")));
    }

    /// Every card but a skill is a drag source, carrying the payload its timeline target understands.
    #[test]
    fn cards_drag_their_own_payloads() {
        let look = crate::engine::presets::builtin_looks()[0].name.clone();
        let caption = crate::engine::presets::builtin_caption_styles()[0].text.clone();
        let block = crate::model::builtin_blocks()[0].name.clone();
        let title = crate::engine::presets::builtin_titles()[0].name.clone();
        let ramp = crate::engine::presets::builtin_speed_ramps()[0].name.clone();
        let cases: Vec<(GalleryTab, String, Option<DragPayload>)> = vec![
            (GalleryTab::Looks, look.clone(), Some(DragPayload::GalleryCard(GalleryTab::Looks, look))),
            (GalleryTab::Captions, caption.clone(), Some(DragPayload::GalleryCard(GalleryTab::Captions, caption))),
            (GalleryTab::SpeedRamps, ramp.clone(), Some(DragPayload::GalleryCard(GalleryTab::SpeedRamps, ramp))),
            (GalleryTab::Animate, block.clone(), Some(DragPayload::KeyBlock(block))),
            (GalleryTab::Titles, title.clone(), Some(DragPayload::GalleryCard(GalleryTab::Titles, title))),
            (GalleryTab::Effects, "Blur".into(), Some(DragPayload::Effect(crate::model::EffectKind::Blur))),
            (GalleryTab::Transitions, "Wipe".into(), Some(DragPayload::Transition(crate::model::TransitionKind::Wipe))),
        ];
        for (tab, name, want) in cases {
            let mut g = Gx::new(tab);
            g.drag(&name, 30.0);
            let got = g.payload().map(|p| format!("{p:?}"));
            assert_eq!(got, want.map(|p| format!("{p:?}")), "{tab:?} card '{name}'");
        }
        // a saved template, a library item, a skill
        let mut g = Gx::new(GalleryTab::Templates);
        g.settings.templates.push(crate::settings::Template { name: "Lower third".into(), json: "{}".into() });
        g.drag("Lower third", 30.0);
        assert!(matches!(g.payload(), Some(DragPayload::Template(n)) if n == "Lower third"));
        let mut g = Gx::new(GalleryTab::Looks);
        g.state.lib = Some(Kind::Html);
        g.cells = vec![("html/card", "Card", Kind::Html, true), ("skills/tidy", "Tidy", Kind::Skill, true)];
        g.drag("Card", 30.0);
        assert!(matches!(g.payload(), Some(DragPayload::LibItem(id)) if id == "html/card"));
        let mut g = Gx::new(GalleryTab::Looks);
        g.state.lib = Some(Kind::Skill);
        g.cells = vec![("skills/tidy", "Tidy", Kind::Skill, true)];
        g.drag("Tidy", 30.0);
        assert!(g.payload().is_none(), "a skill makes no sense on a timeline: not draggable");
    }

    /// A press that barely moves is a click, never a drag.
    #[test]
    fn a_click_is_not_a_drag() {
        let mut g = Gx::new(GalleryTab::Effects);
        g.selection = vec![101];
        g.frame(vec![]);
        let at = g.rect("Blur").center();
        g.frame(vec![Event::PointerMoved(at)]);
        g.mouse(at, PointerButton::Primary, true);
        g.frame(vec![Event::PointerMoved(at + egui::vec2(3.0, 2.0))]);
        assert!(g.payload().is_none(), "3 points is under the drag threshold");
        g.mouse(at + egui::vec2(3.0, 2.0), PointerButton::Primary, false);
        g.frame(vec![]);
        assert!(g.payload().is_none());
        assert_eq!(g.take().apply.map(|a| a.1), Some("Blur".into()), "it was a click");
    }

    /// A drag past the threshold is not a click when the button comes up, and Esc cancels it for good.
    #[test]
    fn a_drag_does_not_click_and_escape_cancels_it() {
        let mut g = Gx::new(GalleryTab::Effects);
        g.selection = vec![101];
        let to = g.drag("Blur", 30.0);
        assert!(g.payload().is_some(), "dragging");
        g.frame(vec![Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
        g.frame(vec![Event::PointerMoved(to + egui::vec2(5.0, 5.0))]);
        g.frame(vec![Event::PointerMoved(to + egui::vec2(9.0, 9.0))]);
        assert!(g.payload().is_none(), "Esc dropped the payload and the held button does not bring it back");
        g.mouse(to, PointerButton::Primary, false);
        g.frame(vec![]);
        assert_eq!(g.take().apply, None, "a cancelled drag applies nothing");
        // a plain drag that is released on the pane does not count as a click either
        let mut g = Gx::new(GalleryTab::Effects);
        g.selection = vec![101];
        let to = g.drag("Blur", 30.0);
        g.mouse(to, PointerButton::Primary, false);
        g.frame(vec![]);
        assert_eq!(g.take().apply, None);
    }

    #[test]
    fn assert_no_idle_repaint_gallery_effects_and_transitions_tabs() {
        for tab in [GalleryTab::Effects, GalleryTab::Transitions] {
            let mut g = Gx::new(tab);
            g.cells = vec![("effects/glow", "Glow", Kind::Effect, true)];
            for _ in 0..30 {
                g.frame(vec![]);
            }
            assert!(!g.ctx.has_requested_repaint(), "an idle {tab:?} tab requested a repaint");
        }
    }

    /// The same wrap / scroll guarantee for the merged tabs (built-in cards and library cells).
    #[test]
    fn effects_and_transitions_tabs_wrap_and_scroll() {
        for tab in [GalleryTab::Effects, GalleryTab::Transitions] {
            for w in [205.0, 350.0, 1000.0] {
                let ctx = ctx();
                let mut state = GalleryState { tab, ..Default::default() };
                let mut settings = Settings::default();
                let mut t = 0.0;
                for _ in 0..3 {
                    frame(&ctx, &mut state, &mut settings, w, 800.0, 0.0, t);
                    t += 0.016;
                }
                let at = format!("{tab:?} at {w} pt");
                let last = card_rect(&ctx, &format!("Item {}", MANY - 1));
                for i in 0..MANY {
                    let r = card_rect(&ctx, &format!("Item {i}"));
                    assert!(r.left() >= -0.5 && r.right() <= w + 0.5, "{at}: card {i} leaves the pane: {r:?}");
                }
                let (view, content) = scroll(&ctx);
                ctx.data_mut(|d| {
                    d.insert_temp(egui::Id::new("gallery_scroll_to"), (content.y - view.height()).max(0.0))
                });
                for _ in 0..3 {
                    frame(&ctx, &mut state, &mut settings, w, 800.0, 0.0, t);
                    t += 0.016;
                }
                let last2 = card_rect(&ctx, &format!("Item {}", MANY - 1));
                assert!(last2.bottom() <= view.bottom() + 1.0 && last2.top() >= view.top() - 1.0, "{at}: {last:?}");
            }
        }
    }

    // ---- your own Gallery: Mine badge / filter / menu, the Sounds tab ----
    fn own_root(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("se-gallery-ui-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        store::set_test_root(Some(d.clone()));
        forget_mine();
        d
    }

    fn wav_in(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let f = dir.join(name);
        std::fs::write(&f, b"RIFF....WAVEfmt ").unwrap();
        f
    }

    fn esc() -> Event {
        Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }

    #[test]
    fn your_saved_cards_are_marked_filterable_and_have_the_mine_menu() {
        let root = own_root("mine-menu");
        let mut g = Gx::new(GalleryTab::Looks);
        g.settings.effect_presets.push(EffectPreset { name: "MyLook".into(), json: "[]".into() });
        g.frame(vec![]);
        let builtin = crate::engine::presets::builtin_looks()[0].name.clone();
        assert!(is_mine(GalleryTab::Looks, "MyLook", &g.settings, 0.0));
        assert!(!is_mine(GalleryTab::Looks, &builtin, &g.settings, 0.0));
        // the built-in has no Mine rows; yours has all of them
        g.open_menu(&builtin);
        assert!(!g.has_text("Rename…") && !g.has_text("Delete"), "{:?}", g.texts);
        g.frame(vec![esc()]);
        g.open_menu("MyLook");
        for row in ["Rename…", "Edit tags…", "Duplicate", "Replace with current", "Open file location", "Delete"] {
            assert!(g.has_text(row), "'{row}' in {:?}", g.texts);
        }
        g.pick("Delete");
        assert_eq!(g.take().mine, Some((MineAct::Delete, MineRef { kind: GKind::Look, name: "MyLook".into() })));
        // "Mine" filter: only yours
        let mut g = Gx::new(GalleryTab::Looks);
        g.settings.effect_presets.push(EffectPreset { name: "MyLook".into(), json: "[]".into() });
        g.state.f.src = Src::Mine;
        g.frame(vec![]);
        let _ = g.rect("MyLook");
        assert!(!g.has_text(&builtin), "built-ins are filtered out");
        let _ = std::fs::remove_dir_all(root);
        store::set_test_root(None);
    }

    #[test]
    fn the_header_menu_offers_the_folder_and_import() {
        let root = own_root("bar");
        let mut g = Gx::new(GalleryTab::Looks);
        g.frame(vec![]);
        for (label, folder) in
            [("Open the gallery folder", true), ("Import to Gallery…", false), ("Import a folder…", false)]
        {
            g.click_text("⋯");
            g.pick(label);
            let out = g.take();
            match label {
                "Open the gallery folder" => assert_eq!(out.open_folder, Some(Some(GKind::Look)), "{folder}"),
                "Import to Gallery…" => assert_eq!(out.import, Some(false)),
                _ => assert_eq!(out.import, Some(true)),
            }
        }
        // Luts have no saved kind: no folder / import rows
        let mut g = Gx::new(GalleryTab::Luts);
        g.frame(vec![]);
        g.click_text("⋯");
        assert!(!g.has_text("Import to Gallery…"));
        let _ = std::fs::remove_dir_all(root);
        store::set_test_root(None);
    }

    #[test]
    fn empty_space_menu_opens_save_to_gallery() {
        let root = own_root("bgmenu");
        let mut g = Gx::new(GalleryTab::Sounds);
        g.frame(vec![]);
        let at = Pos2::new(200.0, 600.0);
        g.frame(vec![Event::PointerMoved(at)]);
        g.mouse(at, PointerButton::Secondary, true);
        g.mouse(at, PointerButton::Secondary, false);
        g.frame(vec![]);
        assert!(g.has_text("Save to Gallery…"), "{:?}", g.texts);
        g.pick("Save to Gallery…");
        assert!(g.take().save_dialog);
        let _ = std::fs::remove_dir_all(root);
        store::set_test_root(None);
    }

    #[test]
    fn sounds_tab_lists_files_previews_on_hover_and_clicks_add() {
        let root = own_root("sounds");
        let f = wav_in(&root.join("src"), "Door slam.wav");
        let item = store::import_file(&store::root(), &f, &store::parse_tags("foley"), Some(1.5)).unwrap();
        forget_mine();
        let file = item.file.clone().unwrap();
        let mut g = Gx::new(GalleryTab::Sounds);
        g.frame(vec![]);
        let r = g.rect("Door slam");
        assert!(g.has_text("Door slam"));
        // nothing is decoded up front: the card only ASKS for its waveform
        assert_eq!(take_sound_wants(), vec![file.clone()]);
        // the pointer resting on it previews it (after a moment); leaving stops it
        g.frame(vec![Event::PointerMoved(r.center())]);
        assert_eq!(take_sound_wish(), None, "not yet");
        for _ in 0..10 {
            g.frame(vec![]);
        }
        assert_eq!(take_sound_wish(), Some(file.clone()), "hovered long enough");
        g.state.no_hover_preview = true;
        for _ in 0..3 {
            g.frame(vec![]);
        }
        assert_eq!(take_sound_wish(), None, "preview on hover can be turned off");
        // a click adds it at the playhead; a drag carries it
        g.click("Door slam");
        assert_eq!(g.take().sound, Some("Door slam".into()));
        g.drag("Door slam", 30.0);
        let carried = g.payload();
        assert!(
            matches!(carried, Some(crate::ui::DragPayload::GalleryCard(GalleryTab::Sounds, ref n)) if n == "Door slam")
        );
        // search by tag; the menu
        let mut g = Gx::new(GalleryTab::Sounds);
        g.state.f.q = "water".into();
        g.frame(vec![]);
        assert!(g.has_text("No sound matches."));
        g.state.f.q = "foley".into();
        g.frame(vec![]);
        let _ = g.rect("Door slam");
        g.open_menu("Door slam");
        for row in ["Add at playhead", "Rename…", "Open file location", "Delete"] {
            assert!(g.has_text(row), "'{row}' in {:?}", g.texts);
        }
        assert!(!g.has_text("Replace with current"), "a sound has no payload to replace");
        let _ = std::fs::remove_dir_all(root);
        store::set_test_root(None);
    }

    #[test]
    fn assert_no_idle_repaint_gallery_sounds_tab() {
        let root = own_root("idle");
        let f = wav_in(&root.join("src"), "tick.wav");
        let it = store::import_file(&store::root(), &f, &[], None).unwrap();
        forget_mine();
        // its waveform has arrived: an idle tab asks for nothing more
        let peaks = crate::media::waveform::Peaks { min: vec![-0.5; 100], max: vec![0.5; 100] };
        set_sound_peaks(it.file.unwrap(), std::sync::Arc::new(peaks));
        let mut g = Gx::new(GalleryTab::Sounds);
        for _ in 0..30 {
            g.frame(vec![]);
        }
        assert!(!g.ctx.has_requested_repaint(), "an idle Sounds tab requested a repaint");
        assert!(take_sound_wants().is_empty());
        let _ = std::fs::remove_dir_all(root);
        store::set_test_root(None);
    }

    #[test]
    fn assert_no_idle_repaint_gallery_library() {
        let ctx = ctx();
        let mut state = GalleryState { lib: Some(crate::engine::library::Kind::Effect), ..Default::default() };
        let mut settings = Settings::default();
        let mut asked = Vec::new();
        for _ in 0..30 {
            run_lib(&ctx, &mut state, &mut settings, &mut asked);
        }
        assert!(!ctx.has_requested_repaint(), "an idle library tab requested a repaint");
        let per_frame = asked.len() / 30;
        assert!(per_frame > 0 && per_frame < 40, "thumbnails asked for visible cells only ({per_frame} of 40)");
    }

    #[test]
    fn library_lut_listing_is_cached_until_a_folder_changes() {
        let dir = std::env::temp_dir().join(format!("se-gallery-luts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.cube"), "").unwrap();
        let mut settings = Settings::default();
        settings.lut_dirs = vec![dir.to_string_lossy().into_owned()];
        forget_lut_listing();
        let n = |s: &Settings| lut_files(s).iter().filter(|p| p.contains("se-gallery-luts")).count();
        assert_eq!(n(&settings), 1);
        // same stamps: the cached list (a file written behind its back with the stamp forced equal)
        LUT_LIST.with(|l| l.borrow_mut().as_mut().unwrap().1.push("se-gallery-luts-fake".into()));
        assert_eq!(n(&settings), 2, "no rescan while the folders are unchanged");
        forget_lut_listing();
        assert_eq!(n(&settings), 1, "forget = rescan");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn animate_tab_lists_builtin_blocks_then_saved_ones() {
        let mut settings = Settings::default();
        let n = crate::model::builtin_blocks().len();
        settings.key_blocks.push(crate::model::KeyBlock { name: "Mine".into(), dur: 1.0, tracks: vec![] });
        let names = card_names(GalleryTab::Animate, &settings);
        assert_eq!(names.len(), n + 1);
        assert_eq!(names[0], "Fade In");
        assert_eq!(names.last().unwrap(), "Mine");
    }

    #[test]
    fn opening_animate_tab_requests_no_idle_repaint() {
        let ctx = ctx();
        let mut state = GalleryState { tab: GalleryTab::Animate, ..Default::default() };
        let mut settings = Settings::default();
        for _ in 0..30 {
            run(&ctx, &mut state, &mut settings);
        }
        assert!(!ctx.has_requested_repaint(), "idle Animate tab requested a repaint");
    }

    #[test]
    fn gallery_tab_name_roundtrips() {
        for t in GalleryTab::ALL {
            assert_eq!(GalleryTab::from_name(t.name()), Some(t));
        }
    }

    #[test]
    fn looks_tab_excludes_graph_presets() {
        let mut settings = Settings::default();
        settings.effect_presets.push(EffectPreset { name: "MyLook".into(), json: "[]".into() });
        settings.effect_presets.push(EffectPreset { name: "MyGraph".into(), json: "{}".into() });
        let names = card_names(GalleryTab::Looks, &settings);
        assert!(names.contains(&"MyLook".to_string()));
        assert!(!names.contains(&"MyGraph".to_string()), "a saved node-graph preset is not a Look card");
        assert_eq!(names.len(), 12 + 1, "12 builtins + the one saved non-graph preset");
    }

    #[test]
    fn clicking_a_look_card_sets_apply_not_place() {
        let ctx = ctx();
        let mut state = GalleryState::default();
        let mut settings = Settings::default();
        let resp = run(&ctx, &mut state, &mut settings);
        // no click simulated yet - this just proves show() runs headlessly without panicking and lists
        // the Looks tab by default
        assert!(resp.apply.is_none() && resp.place.is_none());
    }

    #[test]
    fn templates_tab_lists_saved_templates() {
        let mut settings = Settings::default();
        settings.templates.push(crate::settings::Template { name: "Intro".into(), json: String::new() });
        let names = card_names(GalleryTab::Templates, &settings);
        assert_eq!(names, vec!["Intro".to_string()]);
    }

    #[test]
    fn speed_ramps_tab_lists_the_five_builtins() {
        let settings = Settings::default();
        let names = card_names(GalleryTab::SpeedRamps, &settings);
        assert_eq!(names, vec!["Montage", "Hero", "Bullet", "Jump", "Flash"]);
    }

    // ---- ws:text-titles ----

    #[test]
    fn titles_tab_lists_builtins_plus_user_text_templates() {
        let mut settings = Settings::default();
        let mut p = crate::model::Project::new();
        let text_id = p.add_text_clip(0.0, 2.0);
        settings.templates.push(crate::engine::presets::capture_template("My Title", &p, &[text_id]));
        let names = card_names(GalleryTab::Titles, &settings);
        assert_eq!(names.len(), 4 + 1, "4 builtins (ws:browser-layer added Browser Window) + the one saved text template");
        assert!(names.contains(&"My Title".to_string()));
        assert!(names.contains(&"Lower Third".to_string()));
    }

    #[test]
    fn clicking_a_titles_card_sets_place_title_not_place() {
        let ctx = ctx();
        let mut state = GalleryState { tab: GalleryTab::Titles, ..Default::default() };
        let mut settings = Settings::default();
        let resp = run(&ctx, &mut state, &mut settings);
        // no click simulated yet - proves show() runs headlessly on the Titles tab without panicking
        assert!(resp.place.is_none() && resp.place_title.is_none());
    }

    #[test]
    fn opening_titles_tab_requests_no_idle_repaint() {
        let ctx = ctx();
        let mut state = GalleryState { tab: GalleryTab::Titles, ..Default::default() };
        let mut settings = Settings::default();
        for _ in 0..30 {
            run(&ctx, &mut state, &mut settings);
        }
        assert!(!ctx.has_requested_repaint(), "idle Titles tab requested a repaint");
    }

    #[test]
    fn template_field_widget_edits_the_right_field() {
        let ctx = ctx();
        let mut clip = crate::model::Clip::new(1, crate::model::ClipKind::Text, "t", 0.0, 2.0);
        clip.text.as_mut().unwrap().text = "before".into();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                template_field_widget(ui, "text.text", &mut clip);
            });
        });
        // no simulated edit this frame - proves the widget renders for a real field without panicking
        assert_eq!(clip.text.unwrap().text, "before");
    }

    // ---- search, chips, category, sort, count; the online switch ----
    #[test]
    fn chips_category_and_count_filter_the_effects_grid() {
        use crate::model::EffectKind;
        let mut g = Gx::new(GalleryTab::Effects);
        g.cells = vec![("effects/glow", "Glow", Kind::Effect, true), ("effects/rain", "Rain", Kind::Effect, false)];
        g.frame(vec![]);
        let built = EffectKind::ALL.len();
        assert_eq!(g.state.count, (built + 2, built + 2), "everything, unfiltered");
        for (chip, src, shown) in [
            ("Online", Src::Online, 1),
            ("Installed", Src::Installed, 1),
            ("Built-in", Src::BuiltIn, built),
            ("Mine", Src::Mine, 0),
        ] {
            g.click_text(chip);
            assert_eq!(g.state.f.src, src, "{chip}");
            assert_eq!(g.state.count, (shown, built + 2), "{chip}: {:?}", g.state.count);
        }
        g.click_text("All");
        let cat = crate::ui::effects_ui::categories()[0];
        g.state.f.cat = cat.into();
        g.frame(vec![]);
        let in_cat = EffectKind::ALL.iter().filter(|k| k.category() == cat).count();
        assert_eq!(g.state.count.0, in_cat, "one category: its built-ins only");
        // a search on top; the unknown category of another tab is dropped when the tab changes
        g.state.f.cat.clear();
        g.state.f.q = "rain".into();
        g.frame(vec![]);
        assert_eq!(g.state.count.0, 1, "{:?}", g.state.count);
        g.state.tab = GalleryTab::Transitions;
        g.frame(vec![]);
        assert!(g.state.f.q.is_empty() && g.state.f.src == Src::All, "a new tab starts unfiltered");
    }

    #[test]
    fn sort_by_most_used_moves_a_card_to_the_front() {
        let mut g = Gx::new(GalleryTab::Effects);
        g.cells = vec![("effects/rain", "Rain", Kind::Effect, true)];
        g.frame(vec![]);
        let (rain, blur) = (g.rect("Rain"), g.rect("Blur"));
        let before =
            |a: Rect, b: Rect| a.top() < b.top() - 1.0 || ((a.top() - b.top()).abs() < 1.0 && a.left() < b.left());
        assert!(before(blur, rain), "default order: the built-ins, then the library");
        g.settings.gallery_used.insert(used_key("lib", "effects/rain"), 3);
        g.state.f.sort = Sort::Used;
        g.frame(vec![]);
        assert!(before(g.rect("Rain"), g.rect("Blur")), "most used first");
        g.state.f.sort = Sort::Name;
        g.frame(vec![]);
        assert!(before(g.rect("Blur"), g.rect("Rain")) && before(g.rect("Blur"), g.rect("Vignette")), "A to Z");
    }

    #[test]
    fn esc_clears_the_search_and_ctrl_f_focuses_it() {
        let mut g = Gx::new(GalleryTab::Effects);
        g.state.f.q = "blur".into();
        g.frame(vec![Event::PointerMoved(Pos2::new(200.0, 300.0))]);
        g.frame(vec![Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
        assert!(g.state.f.q.is_empty(), "Esc clears the search");
        focus_search();
        g.frame(vec![]);
        g.frame(vec![]);
        assert!(
            g.ctx.memory(|m| m.has_focus(egui::Id::new("gallery_search"))),
            "Ctrl+F put the keyboard in the search box"
        );
    }

    #[test]
    fn search_looks_in_tags_and_descriptions_of_library_items() {
        let mut g = Gx::new(GalleryTab::Luts);
        g.cells = vec![("luts/warm", "Warm", Kind::Lut, true)];
        g.frame(vec![]);
        let all = g.state.count.1;
        g.state.f.q = "warm".into();
        g.frame(vec![]);
        assert_eq!(g.state.count, (1, all), "the LUT's name; the others are searched by file name");
    }

    /// Not connected: the cells are only what is installed, a small banner offers the library, and clicking
    /// it asks to connect (nothing else). Connected: the banner and the wait are gone, "Online" is a chip.
    #[test]
    fn browse_online_library_is_a_banner_not_a_gate() {
        let mut g = Gx::new(GalleryTab::Effects);
        g.connected = false;
        g.cells = vec![("effects/glow", "Glow", Kind::Effect, true)];
        g.frame(vec![]);
        assert!(g.has_text("Browse online library"), "{:?}", g.texts);
        assert!(!g.has_text("Online"), "no Online chip before connecting");
        let _ = g.rect("Blur");
        let _ = g.rect("Glow"); // the built-ins and what is installed are all there, usable at once
        g.click_text("Browse online library");
        assert_eq!(g.take().lib, Some(LibAct::Connect));
        let mut g = Gx::new(GalleryTab::Effects);
        g.cells = vec![("effects/rain", "Rain", Kind::Effect, false)];
        g.frame(vec![]);
        assert!(!g.has_text("Browse online library") && g.has_text("Online"));
        // no "use online library or a folder" choice anywhere in the main flow
        assert!(!g.texts.iter().any(|t| t.contains("Use a folder") && !t.contains("…")), "{:?}", g.texts);
    }

    /// Clicking an online (cloud) card is one action: the caller downloads just that item and applies it.
    #[test]
    fn clicking_an_online_card_asks_to_use_it() {
        let mut g = Gx::new(GalleryTab::Effects);
        g.cells = vec![("effects/rain", "Rain", Kind::Effect, false)];
        g.state.f.q = "rain".into(); // (the library cards sit after the long list of built-ins)
        g.click("Rain");
        assert_eq!(g.take().lib, Some(LibAct::Apply("effects/rain".into())));
        let mut g = Gx::new(GalleryTab::Transitions);
        g.selection = vec![101];
        g.cells = vec![("transitions/tear", "Tear", Kind::Transition, false)];
        g.open_menu("Tear");
        g.pick("Apply to End");
        assert_eq!(g.take().lib, Some(LibAct::ApplyAt("transitions/tear".into(), "end")));
    }

    #[test]
    fn the_header_menu_holds_use_a_folder_and_turning_the_library_off() {
        let mut g = Gx::new(GalleryTab::Effects);
        g.click_text("⋯");
        assert!(
            g.has_text("Use a folder as the library…") && g.has_text("Turn the online library off"),
            "{:?}",
            g.texts
        );
        g.pick("Turn the online library off");
        assert_eq!(g.take().lib, Some(LibAct::Disconnect));
    }

    /// `gallery.list`'s filters are the header's: source chip, search, category, sort - over built-in,
    /// saved and library items alike.
    #[test]
    fn card_infos_filter_like_the_header() {
        use crate::engine::library::Row;
        let row = |id: &str, name: &str, installed: bool, tags: &[&str]| Row {
            id: id.into(),
            kind: if id.starts_with("effects/") { Kind::Effect } else { Kind::Transition },
            name: name.into(),
            desc: String::new(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            installed,
            yours: false,
            thumb: String::new(),
        };
        let rows = [
            row("effects/glow", "Glow", true, &["light"]),
            row("effects/rain", "Rain", false, &["weather"]),
            row("transitions/tear", "Paper Tear", false, &["paper"]),
        ];
        let settings = Settings::default();
        let names = |tab, f: &Filters| {
            card_infos(tab, &settings, &rows, f, 0.0).into_iter().map(|c| c.name).collect::<Vec<_>>()
        };
        let all = names(GalleryTab::Effects, &Filters::default());
        assert!(
            all.contains(&"Blur".to_string()) && all.ends_with(&["Glow".to_string(), "Rain".to_string()]),
            "{all:?}"
        );
        let f = |src| Filters { src, ..Default::default() };
        assert_eq!(names(GalleryTab::Effects, &f(Src::Online)), ["Rain"]);
        assert_eq!(names(GalleryTab::Effects, &f(Src::Installed)), ["Glow"]);
        assert!(names(GalleryTab::Effects, &f(Src::BuiltIn)).len() == crate::model::EffectKind::ALL.len());
        let q = Filters { q: "weather".into(), ..Default::default() };
        assert_eq!(names(GalleryTab::Effects, &q), ["Rain"], "the search reads tags");
        let cat = Filters { cat: "Shader".into(), ..Default::default() };
        assert_eq!(names(GalleryTab::Transitions, &cat), ["Paper Tear"], "library transitions are Shader transitions");
        let fades = Filters { cat: "Fades".into(), ..Default::default() };
        assert_eq!(names(GalleryTab::Transitions, &fades), ["Cross Fade", "Fade to Color"]);
        let by_name = Filters { sort: Sort::Name, src: Src::Online, ..Default::default() };
        assert_eq!(names(GalleryTab::Effects, &by_name), ["Rain"]);
        let info = card_infos(GalleryTab::Effects, &settings, &rows, &f(Src::Online), 0.0);
        assert_eq!(info[0].id.as_deref(), Some("effects/rain"));
    }
}
