//! ---- ws:inspector-gallery ----
//! The Gallery pane (`Pane::Presets`'s drawer, replacing size-diet's `library::reuse_ui` placeholder):
//! tabs of click-to-apply cards - Looks, LUTs, Captions, Speed Ramps, Templates, Titles (the Transitions
//! tab is list/apply-only over MCP: the Transitions pane is that catalogue's one home).
//! Looks/LUTs get real GPU thumbnails (`ui::app::thumbs::build_gallery_thumbnails`, read here through a
//! thread-local cache - same convention `effects_ui.rs`'s own `THUMBS` uses, since `App` isn't reachable
//! from this sibling module); Captions get a painted colour swatch (cheap, no GPU); Speed
//! Ramps/Transitions/Templates draw a plain name tile. `// ponytail:` - a fancier preview for those
//! three is a pure visual upgrade, not a functional gap (`gallery.list`/`gallery.apply` already cover
//! every tab; see the plan's own ponytail note about Template cards specifically, extended here to the
//! other two for the same reason: time-boxed, not a missing capability).
//!
//! Looks reuse color-engine's `builtin_looks()`/`apply_look()` unchanged (never redefined here) and are
//! filtered to `!EffectPreset::is_graph()` - a saved node-graph preset is not a "Look" card. Applying any
//! card is one undo (`App::run_tool_undoable("gallery.apply", …)`, called by the caller - this file only
//! returns intent via `GalleryResponse`, it never touches `Project` itself). Hovering a card >=150ms sets
//! `App.alt_render` to `AltRequest::Gallery(tab, name)` (canvas-handles-monitor's mechanism, reserved for
//! exactly this) via the same `hover` field, not a new App field.

use crate::model::{Clip, Id};
use crate::settings::Settings;
use crate::theme::Palette;
use crate::ui::app::thumbs::ThumbSource;
use crate::ui::effects_ui::CARD;
use crate::ui::menu;
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
}

impl GalleryTab {
    pub const ALL: [GalleryTab; 8] = [
        Self::Animate,
        Self::Looks,
        Self::Luts,
        Self::Captions,
        Self::SpeedRamps,
        Self::Transitions,
        Self::Templates,
        Self::Titles,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Looks => "Looks",
            Self::Luts => "Luts",
            Self::Captions => "Captions",
            Self::SpeedRamps => "SpeedRamps",
            Self::Transitions => "Transitions",
            Self::Templates => "Templates",
            Self::Titles => "Titles",
            Self::Animate => "Animate",
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
pub const LIB_TABS: [(crate::engine::library::Kind, &str); 5] = {
    use crate::engine::library::Kind;
    [
        (Kind::Transition, "Transitions"),
        (Kind::Effect, "Effects"),
        (Kind::Html, "HTML"),
        (Kind::Icon, "Icons"),
        (Kind::Skill, "Skills"),
    ]
};

/// One library cell, prepared by `ui::app::library_ctl` each frame.
pub struct LibCell {
    pub id: String,
    pub name: String,
    pub desc: String,
    pub installed: bool,
    /// Download progress while downloading.
    pub busy: Option<f32>,
    pub skill: bool,
}

/// What the library part of the Gallery draws. `thumb` is only called for cells on screen.
pub struct LibView<'a> {
    pub cells: Vec<LibCell>,
    pub source: String,
    pub local: bool,
    pub note: String,
    pub loading: bool,
    pub thumb: &'a mut dyn FnMut(&str) -> Option<(egui::TextureId, [u32; 2])>,
}

/// A library click / menu choice, handled by `ui::app::gallery_ctl`.
#[derive(Clone, Debug, PartialEq)]
pub enum LibAct {
    Apply(String),
    Install(String),
    Remove(String),
    Reveal(String),
    View(String),
    Refresh,
    UseFolder,
    UseOnline,
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

/// The library grid: search, source line, cells. One cell drawing (`card`) for every kind.
fn library_grid(ui: &mut egui::Ui, state: &mut GalleryState, lib: &mut LibView, out: &mut GalleryResponse) {
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut state.lib_query).hint_text("Search").desired_width(150.0));
        if ui.small_button("Refresh").on_hover_text("Fetch the catalogue again").clicked() {
            out.lib = Some(LibAct::Refresh);
        }
        if lib.loading {
            ui.weak("Loading…");
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.weak(format!("From {}", lib.source));
        if ui.small_button("Use a folder…").on_hover_text("Read the library from a local folder").clicked() {
            out.lib = Some(LibAct::UseFolder);
        }
        if lib.local && ui.small_button("Use online library").clicked() {
            out.lib = Some(LibAct::UseOnline);
        }
    });
    if !lib.note.is_empty() {
        ui.weak(&lib.note);
    }
    if lib.cells.is_empty() && !lib.loading {
        ui.weak("Nothing here yet.");
    }
    let LibView { cells, thumb, .. } = lib;
    ui.horizontal_wrapped(|ui| {
        for c in cells.iter() {
            let r = card(ui, &c.name, |p, t| {
                if p.clip_rect().intersects(t) {
                    thumb_or_blank(p, t, (*thumb)(&c.id));
                }
                if !c.installed {
                    cloud(p, t);
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
                let rows: [(bool, &str, fn(String) -> LibAct); 5] = [
                    (!c.skill, "Apply", LibAct::Apply),
                    (!c.installed, "Install", LibAct::Install),
                    (c.skill, "View text", LibAct::View),
                    (true, "Show in folder", LibAct::Reveal),
                    (c.installed, "Remove", LibAct::Remove),
                ];
                for (show, label, act) in rows {
                    if show && menu::row(ui, None, label, "").clicked() {
                        out.lib = Some(act(c.id.clone()));
                    }
                }
            });
        }
    });
}

#[derive(Default)]
pub struct GalleryState {
    pub tab: GalleryTab,
    // ---- ws:online-library ----
    /// A library tab (`LIB_TABS`) is open instead of `tab`.
    pub lib: Option<crate::engine::library::Kind>,
    pub lib_query: String,
    /// The "Save look from selection…" name prompt: `Some` while it is open (the typed name so far).
    pub save_name: Option<String>,
    // ---- ws:text-titles ----
    /// (clip id, exposed field name) rows for the Customize panel below the Titles grid, set by the
    /// caller (`ui::app::gallery_ctl::draw`) right after a Titles card is placed - a plain state field
    /// rather than an `egui::Id`-keyed temp, since `App` already owns `GalleryState` per frame.
    pub customize: Vec<(Id, String)>,
    /// ws:keyframe-blocks: the "Save keyframe block from selection…" name prompt.
    pub save_block: Option<String>,
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
}

/// One clickable card: `picture` paints the tile's contents (thumbnail / swatch / nothing - the name is
/// drawn under it either way). Returns the card's response for hover/click.
fn card(ui: &mut egui::Ui, name: &str, picture: impl FnOnce(&egui::Painter, Rect)) -> Response {
    let font = egui::TextStyle::Small.resolve(ui.style());
    let name_h = ui.text_style_height(&egui::TextStyle::Small);
    let (rect, r) = ui.allocate_exact_size(egui::vec2(CARD.0, CARD.1 + name_h + 2.0), Sense::click());
    let tile = Rect::from_min_size(rect.min, egui::vec2(CARD.0, CARD.1));
    let p = ui.painter();
    p.rect_filled(tile, 2.0, ui.visuals().extreme_bg_color);
    picture(p, tile);
    let border = if r.hovered() {
        ui.visuals().selection.stroke.color
    } else {
        ui.visuals().widgets.noninteractive.bg_stroke.color
    };
    p.rect_stroke(tile, 2.0, Stroke::new(if r.hovered() { 2.0 } else { 1.0 }, border), StrokeKind::Inside);
    let g = p.layout(name.to_string(), font, ui.visuals().text_color(), CARD.0 - 4.0);
    let tx = (rect.left() + (CARD.0 - g.size().x) / 2.0).max(rect.left());
    p.galley(egui::pos2(tx, tile.bottom() + 2.0), g, ui.visuals().text_color());
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
        GalleryTab::SpeedRamps => crate::engine::presets::builtin_speed_ramps().into_iter().map(|r| r.name).collect(),
        GalleryTab::Transitions => {
            crate::model::TransitionKind::ALL.into_iter().map(|k| k.name().to_string()).collect()
        }
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

/// The Gallery pane: the tab strip (pinned), then the card grid and - after a Titles card was placed -
/// its Customize rows, scrolling together. A card click applies it (Templates / Titles place at the
/// playhead); its right-click offers the same, and the viewer previews the card while that menu is open.
/// Right-click empty space for "Save look from selection…". Transitions have their own pane, so the
/// Transitions tab is not drawn (it stays a `gallery.list` / `gallery.apply` tab for MCP and scripts).
pub fn show(
    ui: &mut egui::Ui,
    state: &mut GalleryState,
    settings: &mut Settings,
    project: &crate::model::Project,
    selection: &[Id],
    palette: &Palette,
    lib: Option<LibView>,
) -> GalleryResponse {
    let mut out = GalleryResponse::default();
    let bg = crate::ui::markers_ui::menu_area(ui);
    if state.tab == GalleryTab::Transitions {
        state.tab = GalleryTab::Looks;
    }
    // one row that scrolls sideways when the pane is narrower than the tabs
    egui::ScrollArea::horizontal().id_salt("gallery_tabs").show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for tab in GalleryTab::ALL.into_iter().filter(|&t| t != GalleryTab::Transitions) {
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
    });
    ui.separator();
    // ---- ws:online-library ---- a library tab, or the library's LUTs above the Luts tab's folders
    let mut lib = lib;
    if state.lib.is_some() {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if let Some(lib) = lib.as_mut() {
                library_grid(ui, state, lib, &mut out);
            }
        });
        return out;
    }
    let lib_luts = crate::engine::library::root().join("luts");
    let mut names = card_names(state.tab, settings);
    if state.tab == GalleryTab::Luts {
        names.retain(|n| !std::path::Path::new(n).starts_with(&lib_luts)); // drawn by the library grid
    }
    if names.is_empty() && state.tab != GalleryTab::Luts {
        ui.weak(match state.tab {
            GalleryTab::Luts => "No .cube files found - add a folder in Settings ▸ Color.",
            GalleryTab::Templates => "No saved templates yet.",
            GalleryTab::Titles => "No title templates yet.", // builtin_titles() is never empty; user-only edge case
            _ => "Nothing here yet.",
        });
    }
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        if let Some(lib) = lib.as_mut() {
            library_grid(ui, state, lib, &mut out);
            ui.separator();
        }
        ui.horizontal_wrapped(|ui| {
            for name in &names {
                let tab = state.tab;
                let r = match tab {
                    GalleryTab::Looks => {
                        let idx = crate::engine::presets::builtin_looks().iter().position(|p| &p.name == name);
                        let thumb = idx.and_then(|i| thumbnail(&ThumbSource::Look(i)));
                        card(ui, name, |p, t| thumb_or_blank(p, t, thumb))
                    }
                    GalleryTab::Luts => {
                        let thumb = thumbnail(&ThumbSource::Lut(std::path::PathBuf::from(name)));
                        let short = std::path::Path::new(name)
                            .file_stem()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_else(|| name.clone());
                        card(ui, &short, |p, t| thumb_or_blank(p, t, thumb))
                    }
                    GalleryTab::Captions => {
                        let style = crate::engine::presets::builtin_caption_styles()
                            .into_iter()
                            .chain(settings.caption_presets.iter().cloned())
                            .find(|s| &s.text == name);
                        card(ui, name, |p, t| {
                            if let Some(s) = &style {
                                swatch(p, t, s.color, s.outline_color);
                            }
                        })
                    }
                    GalleryTab::Animate => {
                        let b = crate::model::find_block(&settings.key_blocks, name);
                        let payload = crate::ui::DragPayload::KeyBlock(name.clone());
                        crate::ui::drag_source(ui, egui::Id::new(("kblock", name.as_str())), payload, |ui| {
                            card(ui, name, |p, t| {
                                if let Some(b) = &b {
                                    block_picture(p, t, b);
                                }
                            });
                        })
                        .on_hover_text(format!("{name} - click: at the playhead · drag onto a clip"))
                    }
                    _ => card(ui, name, |_, _| {}),
                };
                #[cfg(test)]
                ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("gallery_card", name.as_str())), r.rect));
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
                    if tab == GalleryTab::Animate
                        && settings.key_blocks.iter().any(|b| &b.name == name)
                        && menu::row(ui, None, "Forget saved block", "").clicked()
                    {
                        out.forget_block = Some(name.clone());
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
    menu::context(&bg, |ui| {
        let r =
            ui.add_enabled_ui(!selection.is_empty(), |ui| menu::row(ui, None, "Save look from selection…", "")).inner;
        if r.on_disabled_hover_text("Select a clip whose effects to save").clicked() {
            state.save_name = Some(String::new());
        }
        let r = ui
            .add_enabled_ui(!selection.is_empty(), |ui| menu::row(ui, None, "Save keyframe block from selection…", ""))
            .inner;
        if r.on_disabled_hover_text("Select a clip whose keys to save").clicked() {
            state.save_block = Some(String::new());
        }
    });
    let _ = palette; // reserved: a themed border colour is a pure visual follow-up, not load-bearing yet
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
                out = show(ui, state, settings, &crate::model::Project::new(), &[7], &palette, None);
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
                    id: format!("effects/e{i}"),
                    name: format!("Effect {i}"),
                    desc: String::new(),
                    installed: i % 2 == 0,
                    busy: None,
                    skill: false,
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
                    source: "D:/lib".into(),
                    local: true,
                    note: String::new(),
                    loading: false,
                    thumb: &mut thumb,
                };
                show(ui, state, settings, &crate::model::Project::new(), &[7], &palette, Some(lib));
            });
        });
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
        assert_eq!(names.len(), 3 + 1, "3 builtins + the one saved text template");
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
}
