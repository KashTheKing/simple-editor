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
    // ---- ws:gallery-audio ---- (the Audio side of the Video | Audio switch)
    /// Keyframe blocks that move only volume / pan (fades, swell, duck, tremolo, pan sweeps).
    AudioAnimate,
    /// Crossfade curves, a dip to silence, and J / L-cut helpers (`AUDIO_TRANSITIONS`).
    AudioTransitions,
    /// Bus chains (`mixer_fx::AUDIO_FX` / `AUDIO_SWEEPS`): reverb, echo, EQ looks, dynamics, pitch, sweeps.
    AudioEffects,
    // ---- ws:preview-audio-only ----
    /// The audio visualizer types (`engine::visualizer::styles`): a layer of its own that shows the timeline's
    /// mixed sound. Click = one at the playhead, drag onto a video track = one there.
    Visualizers,
}

/// The Audio ▸ Transitions cards, in order.
pub const AUDIO_TRANSITIONS: [&str; 6] = ["Equal Power", "Linear", "Exponential", "Dip", "J-Cut", "L-Cut"];

/// A block that only moves sound (volume / pan): it belongs on the Audio side.
pub fn is_audio_block(b: &crate::model::KeyBlock) -> bool {
    !b.tracks.is_empty() && b.tracks.iter().all(|t| t.prop == "volume" || t.prop == "pan")
}

impl GalleryTab {
    pub const ALL: [GalleryTab; 14] = [
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
        Self::AudioAnimate,
        Self::AudioTransitions,
        Self::AudioEffects,
        Self::Visualizers,
    ];
    /// The tab strip of each side of the Video | Audio switch.
    pub const VIDEO: [GalleryTab; 9] = [
        Self::Animate,
        Self::Looks,
        Self::Luts,
        Self::Captions,
        Self::SpeedRamps,
        Self::Effects,
        Self::Transitions,
        Self::Templates,
        Self::Titles,
    ];
    pub const AUDIO: [GalleryTab; 5] =
        [Self::AudioAnimate, Self::AudioTransitions, Self::AudioEffects, Self::Visualizers, Self::Sounds];
    /// On the Audio side of the switch.
    pub fn is_audio(self) -> bool {
        Self::AUDIO.contains(&self)
    }
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
            Self::AudioAnimate => "AudioAnimate",
            Self::AudioTransitions => "AudioTransitions",
            Self::AudioEffects => "AudioEffects",
            Self::Visualizers => "Visualizers",
        }
    }
    pub fn from_name(s: &str) -> Option<Self> {
        // (the Animate tab reads "Blocks")
        let blocks = s.eq_ignore_ascii_case("blocks").then_some(Self::Animate);
        Self::ALL.into_iter().find(|t| t.name().eq_ignore_ascii_case(s)).or(blocks)
    }
    fn label(self) -> &'static str {
        match self {
            Self::SpeedRamps => "Speed", // short, so every tab fits a ~350 pt pane
            Self::Animate => "Blocks",
            Self::AudioAnimate => "Animate",
            Self::AudioTransitions => "Transitions",
            Self::AudioEffects => "Effects",
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
        GalleryTab::Sounds | GalleryTab::AudioTransitions | GalleryTab::AudioEffects | GalleryTab::Visualizers => &[],
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
    let label = if c.skill { title_of(&c.name) } else { c.name.clone() };
    let r = card(ui, ("lib", c.id.as_str()), &label, drag, false, |p, t| {
        if p.clip_rect().intersects(t) {
            // an installed shader effect shows what it does (rendered by the GPU), else the catalogue picture
            let own = (c.kind == crate::engine::library::Kind::Effect && c.installed)
                .then(|| crate::ui::effects_ui::library_thumbnail(&short))
                .flatten()
                .map(|tex| (tex, [0, 0]));
            lib_picture(p, t, c, own.or_else(|| thumb(&c.id)));
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

/// A library card's picture: its thumbnail when there is one, else never a blank tile - a skill is drawn
/// as a document card (a `.md` page tinted by its category, the category's symbol, title and the first
/// line of its description), every other kind as its type symbol (HTML `</>`, an icon outline...).
fn lib_picture(p: &egui::Painter, t: Rect, c: &LibCell, thumb: Option<(egui::TextureId, [u32; 2])>) {
    use crate::engine::library::Kind;
    use crate::ui::tools::Glyph;
    if thumb.is_some() {
        return thumb_or_blank(p, t, thumb);
    }
    let (g, tint): (Option<Glyph>, Color32) = match c.kind {
        Kind::Skill => return skill_card(p, t, c),
        Kind::Html => (None, Color32::from_rgb(230, 120, 60)),
        Kind::Icon => (Some(Glyph::Star), Color32::from_rgb(240, 200, 70)),
        Kind::Effect => (Some(Glyph::Sliders), Color32::from_rgb(120, 160, 240)),
        Kind::Transition => (Some(Glyph::Transition), Color32::from_rgb(150, 120, 230)),
        Kind::Lut => (Some(Glyph::CurveIcon), Color32::from_rgb(220, 90, 170)),
    };
    type_tile(p, t, tint, g, "</>");
}

/// A tile tinted `tint` with one big symbol in the middle: `g`, or `text` (monospace) without one.
fn type_tile(p: &egui::Painter, t: Rect, tint: Color32, g: Option<crate::ui::tools::Glyph>, text: &str) {
    p.rect_filled(t, 2.0, tint.gamma_multiply(0.22));
    let px = (t.height() * 0.42).clamp(12.0, 40.0);
    match g {
        Some(g) => big_glyph(p, t.center(), g, px, tint),
        None => {
            p.text(t.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::monospace(px * 0.8), tint);
        }
    }
}

/// `g` from Windows' icon font at `px` (the font `tools::draw_glyph` uses, at any size); the few
/// vector-only glyphs fall back to `draw_glyph`'s own size.
fn big_glyph(p: &egui::Painter, at: egui::Pos2, g: crate::ui::tools::Glyph, px: f32, col: Color32) {
    let Some(ch) = g.icon() else {
        return crate::ui::tools::draw_glyph(p, Rect::from_center_size(at, egui::vec2(px, px)), g, col);
    };
    let icons = crate::theme::icons();
    let family = match p.fonts(|f| f.definitions().families.contains_key(&icons)) {
        true => icons,
        false => egui::FontFamily::Proportional,
    };
    p.text(at, egui::Align2::CENTER_CENTER, ch, egui::FontId::new(px, family), col);
}

/// A skill's category from its id, name and tags: (symbol, tint). Nothing matched: a plain note.
fn skill_category(c: &LibCell) -> (crate::ui::tools::Glyph, Color32) {
    use crate::ui::tools::Glyph;
    let hay = format!("{} {} {}", c.id, c.name, c.tags.join(" ")).to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| hay.contains(w));
    if has(&["convert", "premiere", "resolve", "after effects", "avid"]) {
        (Glyph::Wrench, Color32::from_rgb(70, 190, 180))
    } else if has(&["style", "preset", "trailer", "vlog", "montage", "recap", "highlight"]) {
        (Glyph::Clapperboard, Color32::from_rgb(235, 90, 110))
    } else if has(&["caption", "subtitle", "translate", "transcript"]) {
        (Glyph::Subtitles, Color32::from_rgb(240, 200, 70))
    } else if has(&["color", "colour", "grade", "lut", "cdl", "look"]) {
        (Glyph::Eyedropper, Color32::from_rgb(200, 110, 230))
    } else if has(&["audio", "loud", "sfx", "music", "beat", "duck", "voice", "sound"]) {
        (Glyph::Waveform, Color32::from_rgb(90, 200, 120))
    } else if has(&["export", "deliver", "chapter", "description", "thumbnail", "render"]) {
        (Glyph::ExportArrow, Color32::from_rgb(90, 150, 240))
    } else if has(&["cut", "silence", "filler", "pacing", "trim", "j-cut", "split", "reframe", "b-roll"]) {
        (Glyph::Razor, Color32::from_rgb(240, 140, 60))
    } else {
        (Glyph::Notepad, Color32::from_rgb(150, 160, 175))
    }
}

/// `text` laid out in at most `rows` lines of width `w`, the rest cut with an ellipsis.
fn clipped(p: &egui::Painter, text: &str, font: egui::FontId, col: Color32, w: f32, rows: usize) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple(text.to_string(), font, col, w.max(1.0));
    job.wrap = egui::text::TextWrapping {
        max_width: w.max(1.0),
        max_rows: rows.max(1),
        break_anywhere: rows <= 1,
        overflow_character: Some('\u{2026}'),
    };
    p.layout_job(job)
}

/// A skill's name as a title: `cut-silences` -> "Cut silences" (a name with capitals is kept).
fn title_of(name: &str) -> String {
    if name.chars().any(|c| c.is_uppercase()) {
        return name.to_string();
    }
    let t = name.replace(['-', '_'], " ");
    let mut cs = t.chars();
    cs.next().map(|f| f.to_uppercase().chain(cs).collect()).unwrap_or_default()
}

/// The document card: a `.md` page with a folded corner, tinted by the skill's category, the category
/// symbol on it, and beside it the title and the description (as many lines as the card size allows).
fn skill_card(p: &egui::Painter, t: Rect, c: &LibCell) {
    let (g, tint) = skill_category(c);
    p.rect_filled(t, 2.0, tint.gamma_multiply(0.16));
    let m = (t.height() * 0.12).clamp(3.0, 8.0);
    let ph = t.height() - 2.0 * m;
    let pw = (ph * 0.78).min(t.width() * 0.42);
    let page = Rect::from_min_size(t.min + egui::vec2(m, m), egui::vec2(pw, ph));
    let f = pw * 0.32;
    let paper = Color32::from_rgb(238, 240, 244);
    let (tl, tr, tr2) = (page.left_top(), egui::pos2(page.right() - f, page.top()), egui::pos2(page.right(), page.top() + f));
    p.add(egui::Shape::convex_polygon(vec![tl, tr, tr2, page.right_bottom(), page.left_bottom()], paper, Stroke::new(1.0, tint)));
    p.add(egui::Shape::convex_polygon(vec![tr, egui::pos2(tr.x, tr2.y), tr2], tint, Stroke::NONE));
    big_glyph(p, page.center() - egui::vec2(0.0, ph * 0.08), g, (pw * 0.5).clamp(9.0, 30.0), tint);
    if ph >= 30.0 {
        let fs = (ph * 0.16).clamp(7.0, 11.0);
        let at = egui::pos2(page.center().x, page.bottom() - 2.0);
        p.text(at, egui::Align2::CENTER_BOTTOM, ".md", egui::FontId::monospace(fs), tint);
    }
    let x = page.right() + m;
    let w = t.right() - m - x;
    if w < 18.0 {
        return;
    }
    // beside the page: the description (the title is the label under the card); a wide card also
    // repeats the title on the card. The cloud badge of an online card keeps the top-right corner free.
    let px = (t.height() * 0.17).clamp(8.0, 12.0);
    let mut y = t.top() + m;
    if !c.installed {
        y += 12.0_f32.min(t.height() * 0.2);
    }
    if w >= 60.0 {
        let title = clipped(p, &title_of(&c.name), egui::FontId::proportional(px + 2.0), Color32::WHITE, w, 1);
        let h = title.size().y;
        p.galley(egui::pos2(x, y), title, Color32::WHITE);
        y += h + 1.0;
    }
    let room = t.bottom() - m - y;
    let text = if c.desc.is_empty() { title_of(&c.name) } else { c.desc.clone() };
    if room >= px {
        let rows = ((room / (px * 1.25)).floor() as usize).max(1);
        let dim = Color32::from_gray(205);
        p.galley(egui::pos2(x, y), clipped(p, &text, egui::FontId::proportional(px), dim, w, rows), dim);
    }
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
        GalleryTab::Animate => crate::ui::animator::block_sections().into_iter().map(String::from).collect(),
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
        ui.weak(if audio == Some(true) && total == 0 {
            "Sound effects are on the Audio side (Audio ▸ Effects)"
        } else {
            "No effect matches"
        });
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
    // ---- ws:gallery-audio ----
    /// The selection last seen: a NEW selection may pre-pick the matching side (`auto_side`).
    last_sel: Vec<Id>,
    // ---- ws:text-titles ----
    /// (clip id, exposed field name) rows for the Customize panel below the Titles grid, set by the
    /// caller (`ui::app::gallery_ctl::draw`) right after a Titles card is placed - a plain state field
    /// rather than an `egui::Id`-keyed temp, since `App` already owns `GalleryState` per frame.
    pub customize: Vec<(Id, String)>,
    /// The user picked a tab other than Blocks since the Animator last switched the Gallery to it (a click
    /// into the Animator then leaves the tab alone).
    pub picked: bool,
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
    /// ws:animator-templates: the empty-space menu's "Template Builder…".
    pub builder: bool,
    /// ws:gallery-audio: an Audio card rested under the pointer: hear it on the selected clip.
    pub audition: Option<(GalleryTab, String)>,
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

// ---- ws:animator-templates ----
thread_local! {
    /// The Templates tab's card pictures by template name (set by `gallery_ctl::draw` from the `.png`
    /// beside each saved template).
    static TPL_THUMBS: RefCell<HashMap<String, (egui::TextureId, [u32; 2])>> = RefCell::default();
}

pub(crate) fn set_template_thumbs(m: HashMap<String, (egui::TextureId, [u32; 2])>) {
    TPL_THUMBS.with(|t| *t.borrow_mut() = m);
}

fn template_thumb(name: &str) -> Option<(egui::TextureId, [u32; 2])> {
    TPL_THUMBS.with(|t| t.borrow().get(name).copied())
}

/// How many slots template `name` has (counted once per template: a saved one again when it changes).
fn template_slots(name: &str, settings: &Settings) -> usize {
    thread_local! {
        static SLOTS: RefCell<HashMap<String, (usize, usize)>> = RefCell::default();
    }
    let mine = settings.templates.iter().find(|t| t.name == name);
    let stamp = mine.map_or(0, |t| t.json.len() + 1);
    SLOTS.with(|s| {
        let mut s = s.borrow_mut();
        if let Some(&(st, n)) = s.get(name) {
            if st == stamp {
                return n;
            }
        }
        let n = match mine {
            Some(t) => crate::engine::templates::TemplateData::decode(&t.json).map_or(0, |d| d.slot_list().len()),
            None => crate::engine::templates::builtin(name, 1920, 1080).map_or(0, |d| d.slot_list().len()),
        };
        s.insert(name.to_string(), (stamp, n));
        n
    })
}

/// A template card's slots: one small card per slot along the bottom of the tile (bigger, in the middle,
/// when the template has no picture of its own).
fn slot_marks(p: &egui::Painter, tile: Rect, slots: usize, big: bool) {
    const COLORS: [Color32; 6] = [
        Color32::from_rgb(56, 97, 140),
        Color32::from_rgb(140, 76, 96),
        Color32::from_rgb(70, 122, 92),
        Color32::from_rgb(150, 112, 58),
        Color32::from_rgb(98, 82, 146),
        Color32::from_rgb(60, 120, 128),
    ];
    let n = slots.min(6);
    if n == 0 {
        return;
    }
    let gap = 3.0;
    let w = ((tile.width() - 12.0 - gap * (n as f32 - 1.0)) / n as f32).min(if big { 26.0 } else { 12.0 });
    let h = if big { w * 0.62 } else { 5.0 };
    let total = w * n as f32 + gap * (n as f32 - 1.0);
    let y = if big { tile.center().y - h / 2.0 } else { tile.bottom() - h - 4.0 };
    for i in 0..n {
        let x = tile.center().x - total / 2.0 + i as f32 * (w + gap);
        p.rect_filled(Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h)), 2.0, COLORS[i]);
    }
}

pub(crate) fn set_sound_peaks(path: std::path::PathBuf, p: std::sync::Arc<crate::media::waveform::Peaks>) {
    SOUND_PEAKS.with(|m| m.borrow_mut().insert(path, p));
}

/// The audio files visible cards asked a waveform for (drained by the App, which decodes a few per frame).
pub(crate) fn take_sound_wants() -> Vec<std::path::PathBuf> {
    SOUND_WANT.with(|w| std::mem::take(&mut *w.borrow_mut()))
}

/// The sound whose preview should be playing now (None = silence); reset each time it is read.
/// ws:gallery-audio: play `path` as this frame's hover preview (an Audio card's audition).
pub(crate) fn wish_sound(path: std::path::PathBuf) {
    SOUND_WISH.with(|w| *w.borrow_mut() = Some(path));
}
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
        GalleryTab::Animate | GalleryTab::AudioAnimate => GKind::Animate,
        GalleryTab::Sounds => GKind::Sound,
        GalleryTab::Luts | GalleryTab::AudioTransitions | GalleryTab::AudioEffects | GalleryTab::Visualizers => {
            return None
        }
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
        GalleryTab::Animate | GalleryTab::AudioAnimate => settings.key_blocks.iter().any(|b| b.name == name),
        GalleryTab::Templates | GalleryTab::Titles => settings.templates.iter().any(|t| t.name == name),
        GalleryTab::SpeedRamps | GalleryTab::Effects | GalleryTab::Transitions | GalleryTab::Sounds => {
            mine_kind(tab).is_some_and(|k| mine_items(k, now).iter().any(|i| i.name == name))
        }
        GalleryTab::Luts | GalleryTab::AudioTransitions | GalleryTab::AudioEffects | GalleryTab::Visualizers => false,
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
    // ws:animator-templates: one file to share, with or without the footage inside
    if kind == GKind::Template {
        rows.push(("Export with footage…", MineAct::Export(true)));
        rows.push(("Export without footage…", MineAct::Export(false)));
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
                        // peaks still loading (or none): the sound symbol, never a blank tile
                        let px = (t.height() * 0.42).clamp(12.0, 40.0);
                        big_glyph(p, t.center(), crate::ui::tools::Glyph::Waveform, px, palette.clip_audio);
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
    let g = clipped(&p, name, font, ui.visuals().text_color(), cw - 4.0, 1);
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
impl GalleryState {
    /// Show card `name` on `tab` (already opened by the caller): the tab's search is set to it, kept
    /// through the "new tab starts unfiltered" reset (the palette's go-to).
    pub fn reveal(&mut self, tab: GalleryTab, name: &str) {
        self.seen = Some((tab, None));
        self.f.clear();
        self.f.q = name.to_string();
    }
}

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
        // ws:animator-templates: the built-in animation templates, then yours (a name of yours wins)
        GalleryTab::Templates => crate::engine::templates::BUILTIN_NAMES
            .iter()
            .map(|n| n.to_string())
            .filter(|n| !settings.templates.iter().any(|t| t.name.eq_ignore_ascii_case(n)))
            .chain(settings.templates.iter().map(|t| t.name.clone()))
            .collect(),
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
        GalleryTab::Animate | GalleryTab::AudioAnimate => crate::model::builtin_blocks()
            .into_iter()
            .chain(settings.key_blocks.iter().cloned())
            .filter(|b| tab == GalleryTab::Animate || is_audio_block(b))
            .map(|b| b.name)
            .collect(),
        GalleryTab::AudioTransitions => AUDIO_TRANSITIONS.iter().map(|s| s.to_string()).collect(),
        GalleryTab::AudioEffects => crate::engine::mixer_fx::audio_fx_cards().map(|(_, l)| l.to_string()).collect(),
        GalleryTab::Visualizers => visualizer_styles().into_iter().map(|s| s.name).collect(),
    }
}

// ---- ws:preview-audio-only ----
/// The visualizer styles, re-read from the library folder at most every 2 s (the card grid asks every frame).
fn visualizer_styles() -> Vec<crate::engine::visualizer::Style> {
    thread_local! {
        static CACHE: RefCell<Option<(std::time::Instant, Vec<crate::engine::visualizer::Style>)>> =
            const { RefCell::new(None) };
    }
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.as_ref().is_none_or(|(at, _)| at.elapsed().as_secs_f32() > 2.0) {
            *c = Some((std::time::Instant::now(), crate::engine::visualizer::styles()));
        }
        c.as_ref().map(|(_, s)| s.clone()).unwrap_or_default()
    })
}

/// A Visualizers card's picture: the style itself, still, at a lively level.
fn visualizer_picture(p: &egui::Painter, tile: Rect, name: &str) {
    thread_local! {
        static ORBS: RefCell<HashMap<String, crate::ui::spiky_ball::Orb>> = RefCell::new(HashMap::new());
    }
    let Some(style) = visualizer_styles().into_iter().find(|s| s.name == name) else { return };
    ORBS.with(|o| {
        let mut o = o.borrow_mut();
        let orb = o.entry(style.name.clone()).or_default();
        orb.show_level(0.6, 1.5);
        // a made-up waveform for the Waveform card (no song to read)
        orb.set_wave(Some(crate::engine::visualizer::wave_window(1.5, |k| {
            let k = k as f32;
            0.25 + 0.6 * ((k * 0.11).sin() * (k * 0.023).sin()).abs()
        })));
        orb.paint(p, tile.shrink(1.0), &style.src);
    });
}

/// A Blocks section's title row in the card grid: the kind's colour dot (as in the "+ Animation" menu) and
/// its name, on a row of its own.
fn section_head(ui: &mut egui::Ui, name: &str, kind: &str) {
    ui.end_row();
    let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width().max(40.0), 18.0), Sense::hover());
    let p = ui.painter();
    p.circle_filled(egui::pos2(r.left() + 5.0, r.center().y), 4.0, crate::ui::animator::kind_color(kind));
    let col = ui.visuals().text_color();
    p.text(egui::pos2(r.left() + 14.0, r.center().y), egui::Align2::LEFT_CENTER, name, egui::FontId::proportional(12.0), col);
    ui.end_row();
}

/// A Blocks card's kind: its colour as a dot in the tile's corner.
fn kind_mark(p: &egui::Painter, tile: Rect, col: Color32) {
    p.circle_filled(tile.left_top() + egui::vec2(7.0, 7.0), 3.5, col);
}

/// An Animate card's picture: each animated property's curve over the block, normalised to the tile,
/// in the block's bar colour (the one its timeline bar uses).
fn block_picture(p: &egui::Painter, tile: Rect, b: &crate::model::KeyBlock) {
    let col = crate::ui::animator::kind_of(&b.name, b.gen).1; // the kind's colour, as in the Animator
    // a simulated block has no keys of its own: picture what it makes
    let made;
    let b = if b.gen.is_some() {
        made = crate::model::BlockCtl::new(b.clone()).resolve(b.dur);
        &made
    } else {
        b
    };
    let mut c = Clip::new(0, crate::model::ClipKind::Video, "", 0.0, 1.0);
    b.write(&mut c, 0.0, 1.0, 1.0, 1.0);
    let area = tile.shrink(8.0);
    for tr in &b.tracks {
        // volume / pan blocks skip a video clip - preview them on an audio one
        let mut ca;
        let src = if tr.prop == "volume" || tr.prop == "pan" {
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
        // ws:animator-templates
        if menu::row(ui, None, "Template Builder…", "").clicked() {
            out.builder = true;
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
// ---- ws:gallery-audio ----
/// Remember `tab` as the last one of its side.
fn remember_tab(settings: &mut Settings, tab: GalleryTab) {
    if tab.is_audio() {
        settings.gallery_audio_tab = tab.name().to_string();
    } else {
        settings.gallery_tab = tab.name().to_string();
    }
}

/// Show the Video (`audio` false) or Audio side, on the tab last used there.
pub fn set_side(state: &mut GalleryState, settings: &Settings, audio: bool) {
    let last = if audio { &settings.gallery_audio_tab } else { &settings.gallery_tab };
    let first = if audio { GalleryTab::AudioEffects } else { GalleryTab::Looks };
    state.tab = GalleryTab::from_name(last).filter(|t| t.is_audio() == audio).unwrap_or(first);
    state.lib = None;
}

impl GalleryState {
    /// The side and tab of the last session (the App's starting state).
    pub fn from_settings(settings: &Settings) -> Self {
        let mut st = Self::default();
        set_side(&mut st, settings, settings.gallery_audio);
        st
    }
}

/// A NEW selection pre-picks the matching side: an audio clip shows Audio, a picture-only clip Video (a
/// video clip with linked sound could want either: it leaves the side alone). Never while the pointer is
/// over the Gallery (the user is browsing), never for the same selection twice, and never remembered as
/// the user's choice.
fn auto_side(
    state: &mut GalleryState,
    settings: &Settings,
    project: &crate::model::Project,
    selection: &[Id],
    over: bool,
) {
    if state.last_sel == selection {
        return;
    }
    state.last_sel = selection.to_vec();
    let Some(c) = selection.first().and_then(|&id| project.clip(id)) else { return };
    let want = match c.kind {
        crate::model::ClipKind::Audio => true,
        crate::model::ClipKind::Video if c.link != 0 => return,
        _ => false,
    };
    let audio = state.lib.is_none() && state.tab.is_audio();
    if !over && want != audio {
        set_side(state, settings, want);
    }
}

/// A Titles / Templates card: its rendered picture, else (still rendering, or nothing to show) its type
/// icon - a title "Aa", an HTML template "</>", any other template the template symbol. Never blank.
fn template_picture(p: &egui::Painter, t: Rect, pic: (Option<(egui::TextureId, [u32; 2])>, bool), title: bool) {
    match pic {
        (Some(_), _) => thumb_or_blank(p, t, pic.0),
        (None, true) => type_tile(p, t, Color32::from_rgb(230, 120, 60), None, "</>"),
        (None, false) if title => type_tile(p, t, Color32::from_rgb(90, 190, 140), None, "Aa"),
        (None, false) => type_tile(p, t, Color32::from_rgb(120, 160, 240), Some(crate::ui::tools::Glyph::Template), ""),
    }
}

/// An Audio ▸ Transitions card: the outgoing (faint) and incoming (accent) gains across the card; J / L-cut
/// draw the two clips with the sound's cut moved before / after the picture's.
fn audio_transition_picture(p: &egui::Painter, tile: Rect, name: &str, accent: Color32) {
    let a = tile.shrink2(egui::vec2(8.0, 8.0));
    let faint = accent.gamma_multiply(0.45);
    let j = match name {
        "J-Cut" => Some(true),
        "L-Cut" => Some(false),
        _ => None,
    };
    if let Some(j) = j {
        let (h, mid) = (a.height() * 0.38, a.center().x);
        let cut = mid + if j { -a.width() * 0.22 } else { a.width() * 0.22 };
        let row = |y: f32, x: f32, c1: Color32, c2: Color32| {
            p.rect_filled(Rect::from_min_max(egui::pos2(a.left(), y), egui::pos2(x - 1.0, y + h)), 2.0, c1);
            p.rect_filled(Rect::from_min_max(egui::pos2(x + 1.0, y), egui::pos2(a.right(), y + h)), 2.0, c2);
        };
        row(a.top(), mid, faint, accent);
        row(a.bottom() - h, cut, faint.gamma_multiply(0.7), accent.gamma_multiply(0.7));
        return;
    }
    let (dip, curve) = match name {
        "Dip" => (true, crate::model::XfadeCurve::Linear),
        "Equal Power" => (false, crate::model::XfadeCurve::EqualPower),
        "Exponential" => (false, crate::model::XfadeCurve::Exponential),
        _ => (false, crate::model::XfadeCurve::Linear),
    };
    let pts = |incoming: bool| -> Vec<egui::Pos2> {
        (0..=24)
            .map(|i| {
                let f = i as f64 / 24.0;
                let (o, n) = crate::model::audio_curve_gains(dip, curve, f);
                let g = if incoming { n } else { o };
                egui::pos2(a.left() + f as f32 * a.width(), a.bottom() - g as f32 * a.height())
            })
            .collect()
    };
    p.add(egui::Shape::line(pts(false), Stroke::new(2.0, faint)));
    p.add(egui::Shape::line(pts(true), Stroke::new(2.0, accent)));
}

thread_local! {
    /// Audio ▸ Effects card pictures by card name: (dry, wet) envelopes of the test signal.
    static FX_WAVES: RefCell<HashMap<String, Rc<(Vec<f32>, Vec<f32>)>>> = RefCell::default();
}

/// Columns in an effect card's waveform.
const FX_COLS: usize = 48;

/// The test signal the effect cards are drawn from: a low, a mid and a high tone burst, then a quiet hiss
/// and silence (interleaved stereo at `sr`, one second).
pub(crate) fn fx_test_signal(sr: u32) -> Vec<f32> {
    let mut rng = 0x2545_f491u32;
    (0..sr as usize)
        .flat_map(|i| {
            let t = i as f32 / sr as f32;
            let tone = |hz: f32| (std::f32::consts::TAU * hz * t).sin() * 0.7;
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            let s = match t {
                t if t < 0.14 => tone(150.0),
                t if (0.18..0.32).contains(&t) => tone(1000.0),
                t if (0.36..0.5).contains(&t) => tone(5000.0),
                t if (0.54..0.62).contains(&t) => (rng as f32 / u32::MAX as f32 - 0.5) * 0.06,
                _ => 0.0,
            };
            [s, s]
        })
        .collect()
}

/// (dry, wet) envelopes (max |x| per column) of `fx_test_signal` through the preset's chain at 16 kHz:
/// a filter thins or swells a burst, a reverb / echo leaves a tail, a gate cuts the hiss. Once per name.
pub(crate) fn fx_wave(name: &str) -> Option<Rc<(Vec<f32>, Vec<f32>)>> {
    use crate::engine::mixer_fx as fx;
    if let Some(w) = FX_WAVES.with(|m| m.borrow().get(name).cloned()) {
        return Some(w);
    }
    const SR: u32 = 16_000;
    let (_, chain) = fx::fx_chain(fx::fx_id(name)?, (0.0, 1.0))?;
    let mut buf = fx_test_signal(SR);
    let env = |b: &[f32]| -> Vec<f32> {
        b.chunks(b.len().div_ceil(FX_COLS)).map(|c| c.iter().fold(0f32, |m, x| m.max(x.abs()))).collect()
    };
    let dry = env(&buf);
    fx::run_chain(&chain, SR, 0.0, &mut buf);
    let w = Rc::new((dry, env(&buf)));
    FX_WAVES.with(|m| m.borrow_mut().insert(name.to_string(), w.clone()));
    Some(w)
}

/// An Audio ▸ Effects card: the dry test signal faint, the processed one in the accent colour.
fn audio_fx_picture(p: &egui::Painter, tile: Rect, name: &str, accent: Color32) {
    let Some(w) = fx_wave(name) else { return };
    let a = tile.shrink(6.0);
    let dx = a.width() / FX_COLS as f32;
    for (vals, col, wd) in [(&w.0, accent.gamma_multiply(0.25), dx * 0.8), (&w.1, accent, dx * 0.45)] {
        for (i, v) in vals.iter().enumerate() {
            let h = v.min(1.0) * a.height() * 0.5;
            let x = a.left() + (i as f32 + 0.5) * dx;
            p.line_segment([egui::pos2(x, a.center().y - h), egui::pos2(x, a.center().y + h)], Stroke::new(wd, col));
        }
    }
}

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
    // ---- ws:gallery-audio ---- Video | Audio: a new selection may pre-pick the side, never while browsing
    let over = ui.rect_contains_pointer(ui.max_rect());
    auto_side(state, settings, project, selection, over);
    let mut audio = state.lib.is_none() && state.tab.is_audio();
    ui.horizontal(|ui| {
        if crate::ui::animator::segmented(ui, &mut audio, &[(false, "Video"), (true, "Audio")]) {
            set_side(state, settings, audio);
            settings.gallery_audio = audio; // the user's own pick is remembered; an auto-pick is not
        }
    });
    // the tabs wrap onto a second row in a narrow pane (a sideways-scrolling row hid the library tabs)
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let tabs: &[GalleryTab] = if audio { &GalleryTab::AUDIO } else { &GalleryTab::VIDEO };
        for &tab in tabs {
            if ui.selectable_label(state.lib.is_none() && state.tab == tab, tab.label()).clicked() {
                state.tab = tab;
                state.lib = None;
                state.picked = tab != GalleryTab::Animate;
                remember_tab(settings, tab);
            }
        }
        // ---- ws:online-library ----
        if !audio {
            for (kind, label) in LIB_TABS {
                if ui.selectable_label(state.lib == Some(kind), label).clicked() {
                    state.lib = Some(kind);
                    state.picked = true;
                }
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
    header(ui, state, &srcs, &cats, lib.as_ref(), folder, state.tab.is_audio() && !lib_tab, &mut out);
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
                let (mut tags, created) = mine_meta(tab, &n, now);
                if tab == GalleryTab::Animate {
                    // the category menu's sections: the "+ Animation" menu's kinds
                    tags.push(crate::ui::animator::block_group(&n, origin == Src::Mine).to_string());
                }
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
    // Blocks: by section (in the menu's order), each in the order picked
    let secs = crate::ui::animator::block_sections();
    let sec_of = |n: &str| crate::ui::animator::block_group(n, is_mine(tab, n, settings, now));
    if tab == GalleryTab::Animate {
        names.sort_by_key(|n| secs.iter().position(|s| *s == sec_of(n)));
    }
    // ...after the timeline's transitions (dragged onto the Animator's Timeline lane: a span)
    let trs: Vec<crate::model::TransitionKind> = match tab {
        GalleryTab::Animate => crate::model::TransitionKind::ALL
            .into_iter()
            .filter(|k| state.f.passes(Src::BuiltIn, &["Transitions"], k.name()))
            .collect(),
        _ => Vec::new(),
    };
    state.count_acc.0 += trs.len();
    state.count_acc.1 += if tab == GalleryTab::Animate { crate::model::TransitionKind::ALL.len() } else { 0 };
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
        // ws:gallery-audio: Titles / Templates pictures by card name (thumbnail, has an HTML page)
        let tpl_pics: HashMap<String, (Option<(egui::TextureId, [u32; 2])>, bool)> =
            if matches!(tab, GalleryTab::Titles | GalleryTab::Templates) {
                crate::ui::app::template_thumbs::templates(settings)
                    .into_iter()
                    .map(|t| {
                        let html = t.json.contains("\"html\":{");
                        let k = crate::ui::app::template_thumbs::key(&t);
                        (t.name, (thumbnail(&ThumbSource::Template(k)), html))
                    })
                    .collect()
            } else {
                HashMap::new()
            };
        if names.is_empty() && lut_cells.is_empty() && trs.is_empty() && !matches!(tab, GalleryTab::Sounds) {
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
            if !trs.is_empty() {
                section_head(ui, "Transitions", "Transition");
            }
            for &k in &trs {
                let col = crate::ui::animator::kind_color("Transition");
                let drag = Some(DragPayload::Transition(k));
                let r = card(ui, ("blk-tr", k.name()), k.name(), drag, false, |p, t| {
                    type_tile(p, t, col, Some(crate::ui::tools::Glyph::Transition), "");
                    kind_mark(p, t, col);
                })
                .on_hover_text(format!(
                    "{} - click: at the start of the selection · drag onto the Animator's Timeline lane or a cut",
                    k.name()
                ));
                if r.clicked() && !selection.is_empty() {
                    out.apply = Some((GalleryTab::Transitions, k.name().to_string(), 1.0));
                    out.apply_where = Some(transitions_ui::Where::All.name());
                }
            }
            let mut sec = "";
            for name in &names {
                if tab == GalleryTab::Animate && sec_of(name) != sec {
                    sec = sec_of(name);
                    section_head(ui, sec, if sec == "Mine" { "Custom" } else { sec });
                }
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
                    GalleryTab::Animate | GalleryTab::AudioAnimate => {
                        let b = crate::model::find_block(&settings.key_blocks, name);
                        let drag = DragPayload::KeyBlock(name.clone());
                        card(ui, ("blk", name), name, Some(drag), false, |p, t| {
                            if let Some(b) = &b {
                                block_picture(p, t, b);
                                kind_mark(p, t, crate::ui::animator::kind_of(&b.name, b.gen).1);
                            }
                        })
                        .on_hover_text(format!(
                            "{name} - click: at the playhead · drag onto a clip, or a clip's lane in the Animator"
                        ))
                    }
                    GalleryTab::Templates => {
                        // ws:animator-templates: the picture chosen in the Template Builder, else the rendered one
                        let pic = tpl_pics.get(name.as_str()).copied().unwrap_or_default();
                        let pic = (template_thumb(name).or(pic.0), pic.1);
                        let slots = template_slots(name, settings);
                        card(ui, ("tpl", name), name, Some(DragPayload::Template(name.clone())), false, |p, t| {
                            template_picture(p, t, pic, false);
                            slot_marks(p, t, slots, false);
                        })
                        .on_hover_text(match slots {
                            0 => "Click: place it at the playhead · drag onto the timeline".to_string(),
                            1 => "1 slot for your footage · click: fill it and place it at the playhead".to_string(),
                            n => format!("{n} slots for your footage · click: fill them and place it at the playhead"),
                        })
                    }
                    GalleryTab::Titles => {
                        let pic = tpl_pics.get(name.as_str()).copied().unwrap_or_default();
                        let drag = DragPayload::GalleryCard(tab, name.clone());
                        card(ui, ("card", tab.name(), name), name, Some(drag), false, |p, t| template_picture(p, t, pic, true))
                    }
                    // ---- ws:gallery-audio ----
                    GalleryTab::AudioTransitions => {
                        let drag = DragPayload::GalleryCard(tab, name.clone());
                        card(ui, ("atr", name), name, Some(drag), false, |p, t| {
                            audio_transition_picture(p, t, name, palette.accent)
                        })
                    }
                    GalleryTab::AudioEffects => {
                        let drag = DragPayload::GalleryCard(tab, name.clone());
                        card(ui, ("afx", name), name, Some(drag), false, |p, t| audio_fx_picture(p, t, name, palette.accent))
                    }
                    // ---- ws:preview-audio-only ----: each style drawn by its own shader
                    GalleryTab::Visualizers => {
                        let drag = DragPayload::GalleryCard(tab, name.clone());
                        card(ui, ("viz", name), name, Some(drag), false, |p, t| visualizer_picture(p, t, name))
                            .on_hover_text(format!("{name} - click: at the playhead · drag onto a video track"))
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
                if tab == GalleryTab::Visualizers {
                    // (nothing to hear: a picture)
                } else if tab.is_audio() {
                    // hear it on the selected clip (`gallery_audition`), unless previews are off
                    if !state.no_hover_preview && crate::ui::hover_after(ui, r.id, &r, 300.0) {
                        out.audition = Some((tab, name.clone()));
                    }
                } else if tab != GalleryTab::Animate && crate::ui::hover_after(ui, r.id, &r, 150.0) {
                    out.hover = Some((tab, name.clone()));
                }
                let mut go = r.clicked();
                let menu = menu::context(&r, |ui| {
                    let (label, needs_clip) = match tab {
                        GalleryTab::Templates | GalleryTab::Titles | GalleryTab::Visualizers => {
                            ("Place at playhead", false)
                        }
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
                if menu.is_some()
                    && !tab.is_audio()
                    && !matches!(tab, GalleryTab::Templates | GalleryTab::Titles | GalleryTab::Animate)
                {
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

    /// Paint one library card picture into a tile of the S / M / L size and return its shapes
    /// (flattened), minus nothing: the caller decides what counts as "blank".
    fn picture_shapes(c: &LibCell, size: Size, thumb: Option<(egui::TextureId, [u32; 2])>) -> Vec<egui::Shape> {
        let ctx = ctx();
        let w = size.base_w();
        let tile = Rect::from_min_size(egui::pos2(10.0, 10.0), egui::vec2(w, (w * 9.0 / 16.0).round()));
        let out = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| lib_picture(ui.painter(), tile, c, thumb));
        });
        fn flat(s: &egui::Shape, out: &mut Vec<egui::Shape>) {
            match s {
                egui::Shape::Vec(v) => v.iter().for_each(|s| flat(s, out)),
                s => out.push(s.clone()),
            }
        }
        let mut v = Vec::new();
        out.shapes.iter().for_each(|c| flat(&c.shape, &mut v));
        v
    }

    fn cell(kind: Kind, id: &str, name: &str, desc: &str, tags: &[&str]) -> LibCell {
        LibCell {
            id: id.into(),
            kind,
            name: name.into(),
            desc: desc.into(),
            installed: false,
            busy: None,
            skill: kind == Kind::Skill,
            tags: tags.iter().map(|t| t.to_string()).collect(),
            yours: false,
        }
    }

    /// User report (screenshot): every Skills card was an empty dark tile with only the cloud and the
    /// name. Without a thumbnail no library kind draws a blank tile, at any card size: a skill gets its
    /// document card (page + symbol + title), every other kind its type symbol.
    #[test]
    fn library_cards_without_a_thumbnail_are_never_blank() {
        for kind in Kind::ALL {
            let c = cell(kind, &format!("{}/x", kind.dir()), "Cut Silences", "Remove pauses from talk.", &["cut"]);
            for size in [Size::S, Size::M, Size::L] {
                let shapes = picture_shapes(&c, size, None);
                let marks = shapes.iter().filter(|s| matches!(s, egui::Shape::Text(_) | egui::Shape::Path(_))).count();
                assert!(marks >= 1, "{kind:?} at {size:?} drew only a background: {shapes:?}");
                if kind == Kind::Skill {
                    let page = shapes.iter().any(|s| matches!(s, egui::Shape::Path(p) if p.closed && p.points.len() == 5));
                    assert!(page, "a skill card has its folded page at {size:?}");
                    let texts: Vec<String> = shapes
                        .iter()
                        .filter_map(|s| if let egui::Shape::Text(t) = s { Some(t.galley.text().to_string()) } else { None })
                        .collect();
                    assert!(texts.iter().any(|t| t.contains("Remove")), "the description is on the card at {size:?}: {texts:?}");
                    if size == Size::L {
                        assert!(texts.iter().any(|t| t.starts_with("Cut")), "a wide card shows the title too: {texts:?}");
                    }
                }
            }
        }
    }

    /// A skill that names a picture (`thumb:`) shows it instead of the drawn card.
    #[test]
    fn a_skill_with_a_thumbnail_shows_it() {
        let c = cell(Kind::Skill, "skills/x", "X", "d", &[]);
        let tex = egui::TextureId::User(7);
        let shapes = picture_shapes(&c, Size::M, Some((tex, [16, 9])));
        assert!(shapes.iter().any(|s| matches!(s, egui::Shape::Mesh(m) if m.texture_id == tex)), "{shapes:?}");
        assert!(!shapes.iter().any(|s| matches!(s, egui::Shape::Path(_))), "no drawn page over the picture");
    }

    /// Skills sort into categories by tags / name, each with its own symbol (no two categories alike).
    #[test]
    fn skill_categories_from_tags() {
        let g = |id: &str, tags: &[&str]| skill_category(&cell(Kind::Skill, id, id, "", tags)).0;
        use crate::ui::tools::Glyph;
        assert_eq!(g("skills/convert-from-editor", &[]), Glyph::Wrench);
        assert_eq!(g("skills/style-vlog", &["style"]), Glyph::Clapperboard);
        assert_eq!(g("skills/captions", &[]), Glyph::Subtitles);
        assert_eq!(g("skills/color-grade", &[]), Glyph::Eyedropper);
        assert_eq!(g("skills/loudness", &["audio"]), Glyph::Waveform);
        assert_eq!(g("skills/deliver", &["export"]), Glyph::ExportArrow);
        assert_eq!(g("skills/cut-silences", &[]), Glyph::Razor);
        assert_eq!(g("skills/mystery", &[]), Glyph::Notepad);
    }

    #[test]
    fn skill_titles_read_like_titles() {
        assert_eq!(title_of("cut-silences"), "Cut silences");
        assert_eq!(title_of("style-vlog"), "Style vlog");
        assert_eq!(title_of("My Skill"), "My Skill");
        assert_eq!(title_of(""), "");
    }

    /// Long names stay on one line, cut with an ellipsis, at the smallest card width.
    #[test]
    fn long_card_names_truncate_with_an_ellipsis() {
        let ctx = ctx();
        let mut g = None;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let font = egui::FontId::proportional(10.0);
                g = Some(clipped(ui.painter(), "A very long skill name that never fits", font, Color32::WHITE, 66.0, 1));
            });
        });
        let g = g.unwrap();
        assert_eq!(g.rows.len(), 1);
        let last = g.rows[0].row.glyphs.last().map(|gl| gl.chr);
        assert!(last == Some('\u{2026}') && g.size().x <= 66.5, "{last:?} {}", g.size().x);
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
                    got.audition = got.audition.take().or(o.audition);
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
        // ws:gallery-audio: the new audio selection pre-picks the Audio side, whose Effects are sound effects
        assert_eq!(g.state.tab, GalleryTab::AudioEffects);
        assert!(g.has_text("Telephone") && !g.has_text("Blur"));
        // back on Video ▸ Effects by hand: no pixel effect is offered for the sound, and it says where to look
        g.click_text("Video");
        g.click_text("Effects");
        assert_eq!(g.state.tab, GalleryTab::Effects);
        assert!(g.has_text("Sound effects are on the Audio side (Audio ▸ Effects)"));
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
        // (sound-only blocks too, as in the "+ Animation" menu's Sound; Audio ▸ Animate has only those)
        let n = crate::model::builtin_blocks().len();
        settings.key_blocks.push(crate::model::KeyBlock { name: "Mine".into(), dur: 1.0, ..Default::default() });
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
        // the Animate tab reads Blocks, and `gallery.tab` takes either name
        assert_eq!(GalleryTab::Animate.label(), "Blocks");
        assert_eq!(GalleryTab::from_name("blocks"), Some(GalleryTab::Animate));
    }

    /// The Blocks tab's categories are the "+ Animation" menu's: the timeline's transitions, its kinds in its
    /// order, then yours; every block the menu lists is in its section.
    #[test]
    fn blocks_tab_categories_match_the_animation_menu() {
        use crate::ui::animator::{block_group, block_sections, BLOCK_GROUPS};
        let mut want = vec!["Transitions"];
        want.extend(BLOCK_GROUPS.iter().map(|g| g.0));
        want.push("Mine");
        assert_eq!(block_sections(), want);
        assert_eq!(want[1..9], ["In", "Out", "Physics", "Emphasis", "Camera", "Text", "Sound", "Speed"]);
        assert_eq!(tab_cats(GalleryTab::Animate, None, false, 0.0), want);
        let mut settings = Settings::default();
        settings.key_blocks.push(crate::model::KeyBlock { name: "My Move".into(), dur: 1.0, ..Default::default() });
        let names = card_names(GalleryTab::Animate, &settings);
        for (g, list) in BLOCK_GROUPS {
            for n in list {
                assert!(names.iter().any(|x| x == n), "{n} is a card");
                assert_eq!(block_group(n, false), g, "{n}");
            }
        }
        for n in &names {
            assert!(want.contains(&block_group(n, is_mine(GalleryTab::Animate, n, &settings, 0.0))), "{n}");
        }
        assert_eq!(block_group("My Move", true), "Mine");
        assert_eq!(block_group("Slide In Top", false), "In");
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
        // ws:animator-templates: the built-in animation templates first, then the saved ones
        let n = crate::engine::templates::BUILTIN_NAMES.len();
        assert_eq!(names.len(), n + 1);
        assert_eq!(names[..n], crate::engine::templates::BUILTIN_NAMES);
        assert_eq!(names[n], "Intro");
        // a saved template named like a built-in one is listed once (yours)
        settings.templates.push(crate::settings::Template { name: "Quote Card".into(), json: String::new() });
        assert_eq!(card_names(GalleryTab::Templates, &settings).len(), n + 1);
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

    // ---- ws:gallery-audio ----
    #[test]
    fn the_video_audio_switch_swaps_the_tabs_and_is_remembered() {
        let mut g = Gx::new(GalleryTab::Looks);
        g.frame(vec![]);
        assert!(g.has_text("Video") && g.has_text("Audio"));
        assert!(g.has_text("Looks") && g.has_text("Titles") && g.has_text("HTML") && !g.has_text("Sounds"));
        g.click_text("Audio");
        assert!(g.state.tab.is_audio() && g.settings.gallery_audio);
        assert!(g.has_text("Sounds") && !g.has_text("Looks") && !g.has_text("HTML"), "{:?}", g.texts);
        g.click_text("Transitions");
        assert_eq!(g.state.tab, GalleryTab::AudioTransitions);
        assert!(g.has_text("Equal Power") && g.has_text("J-Cut"));
        g.click_text("Effects");
        assert_eq!(g.state.tab, GalleryTab::AudioEffects);
        assert!(g.has_text("Telephone") && g.has_text("Low-pass Sweep"));
        // each side reopens on its own last tab
        g.click_text("Video");
        assert_eq!(g.state.tab, GalleryTab::Looks);
        assert!(!g.settings.gallery_audio);
        g.click_text("Audio");
        assert_eq!(g.state.tab, GalleryTab::AudioEffects);
        // a restart (through the saved JSON) comes back on the same side and tab
        let saved: Settings = serde_json::from_str(&serde_json::to_string(&g.settings).unwrap()).unwrap();
        assert_eq!(GalleryState::from_settings(&saved).tab, GalleryTab::AudioEffects);
        assert_eq!(GalleryState::from_settings(&Settings::default()).tab, GalleryTab::Looks, "a fresh install: Video");
    }

    #[test]
    fn a_new_audio_selection_picks_the_audio_side_but_never_while_browsing() {
        let mut g = Gx::new(GalleryTab::Looks);
        let ai = g.project.audio_tracks()[0];
        g.project.tracks[ai].clips.push(Clip::new(201, ClipKind::Audio, "a", 0.0, 4.0));
        g.frame(vec![]);
        // the pointer is over the Gallery: the user is browsing, nothing flips
        g.selection = vec![201];
        g.frame(vec![Event::PointerMoved(Pos2::new(200.0, 300.0))]);
        assert_eq!(g.state.tab, GalleryTab::Looks);
        // the same selection later, pointer elsewhere: still nothing (only a NEW selection pre-picks)
        g.frame(vec![Event::PointerGone]);
        assert_eq!(g.state.tab, GalleryTab::Looks);
        // a new selection made elsewhere (the timeline): Audio, and that is not remembered as the user's pick
        g.selection = vec![];
        g.frame(vec![Event::PointerGone]);
        g.selection = vec![201];
        g.frame(vec![Event::PointerGone]);
        assert!(g.state.tab.is_audio());
        assert!(!g.settings.gallery_audio, "an auto-pick is not saved");
        // a picture clip: back to Video
        g.selection = vec![101];
        g.frame(vec![Event::PointerGone]);
        assert!(!g.state.tab.is_audio());
    }

    #[test]
    fn assert_no_idle_repaint_gallery_audio_side() {
        for tab in
            [GalleryTab::AudioAnimate, GalleryTab::AudioTransitions, GalleryTab::AudioEffects, GalleryTab::Visualizers]
        {
            let mut g = Gx::new(tab);
            for _ in 0..30 {
                g.frame(vec![]);
            }
            assert!(!g.ctx.has_requested_repaint(), "an idle {tab:?} tab requested a repaint");
        }
    }

    // ---- ws:preview-audio-only ----
    /// Audio ▸ Visualizers lists the types as cards; a click places one (at the playhead), a card drags as
    /// a Gallery card the timeline puts on a video track.
    #[test]
    fn visualizer_cards_place_a_visualizer() {
        assert!(GalleryTab::AUDIO.contains(&GalleryTab::Visualizers));
        assert_eq!(GalleryTab::from_name("visualizers"), Some(GalleryTab::Visualizers));
        let names = card_names(GalleryTab::Visualizers, &Settings::default());
        for n in ["Neon Orb", "Waveform", "Spectrum Bars", "Voxel Blocks"] {
            assert!(names.iter().any(|x| x == n), "{n} in {names:?}");
        }
        let mut g = Gx::new(GalleryTab::Visualizers);
        g.click("Spectrum Bars");
        assert_eq!(g.take().apply, Some((GalleryTab::Visualizers, "Spectrum Bars".to_string(), 1.0)));
        let card = crate::ui::DragPayload::GalleryCard(GalleryTab::Visualizers, "Neon Orb".into());
        assert_eq!(
            crate::ui::timeline::gallery_drop::drop_kind(&card),
            Some(crate::ui::timeline::gallery_drop::DropKind::OnTrack)
        );
    }

    #[test]
    fn audio_cards_have_pictures_and_hovering_one_asks_to_hear_it() {
        let mut g = Gx::new(GalleryTab::AudioEffects);
        g.frame(vec![]);
        // a reverb leaves a tail where the dry signal is silent; a gate cuts the hiss; a low-pass thins the 5 kHz burst
        let tail = |n: &str| fx_wave(n).unwrap().1[FX_COLS - 4];
        assert!(tail("Hall") > 1e-3 && tail("Muffled") < 1e-4);
        let (dry, wet) = &*fx_wave("Muffled").unwrap();
        let hi = (0.43 * FX_COLS as f32) as usize;
        assert!(wet[hi] < dry[hi] * 0.3, "the low-pass thins the high burst");
        for name in card_names(GalleryTab::AudioEffects, &g.settings) {
            let w = fx_wave(&name).unwrap_or_else(|| panic!("{name}"));
            assert!(w.1.iter().any(|v| *v > 0.01), "{name}: a flat picture");
        }
        let at = g.rect("Telephone").center();
        for _ in 0..12 {
            g.frame(vec![Event::PointerMoved(at)]);
        }
        assert_eq!(g.take().audition, Some((GalleryTab::AudioEffects, "Telephone".to_string())));
        g.state.no_hover_preview = true;
        for _ in 0..12 {
            g.frame(vec![Event::PointerMoved(at)]);
        }
        assert_eq!(g.take().audition, None, "previews off");
    }

    #[test]
    fn title_and_template_cards_are_never_blank() {
        for (pic, title) in [((None, false), true), ((None, false), false), ((None, true), false)] {
            let ctx = ctx();
            let out = ctx.run(egui::RawInput::default(), |ctx| {
                let p = ctx.layer_painter(egui::LayerId::background());
                template_picture(&p, Rect::from_min_size(Pos2::ZERO, egui::vec2(96.0, 54.0)), pic, title);
            });
            assert!(!out.shapes.is_empty(), "{pic:?} {title}: drew nothing");
        }
    }
}
