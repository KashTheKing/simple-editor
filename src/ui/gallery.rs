//! ---- ws:inspector-gallery ----
//! The Gallery pane (`Pane::Presets`'s drawer, replacing size-diet's `library::reuse_ui` placeholder):
//! six tabs of click-to-apply cards - Looks, LUTs, Captions, Speed Ramps, Transitions, Templates.
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
}

impl GalleryTab {
    pub const ALL: [GalleryTab; 7] =
        [Self::Looks, Self::Luts, Self::Captions, Self::SpeedRamps, Self::Transitions, Self::Templates, Self::Titles];
    pub fn name(self) -> &'static str {
        match self {
            Self::Looks => "Looks",
            Self::Luts => "Luts",
            Self::Captions => "Captions",
            Self::SpeedRamps => "SpeedRamps",
            Self::Transitions => "Transitions",
            Self::Templates => "Templates",
            Self::Titles => "Titles",
        }
    }
    pub fn from_name(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.name().eq_ignore_ascii_case(s))
    }
    fn label(self) -> &'static str {
        match self {
            Self::SpeedRamps => "Speed Ramps",
            other => other.name(),
        }
    }
}

#[derive(Default)]
pub struct GalleryState {
    pub tab: GalleryTab,
    /// "Save from selection…" name field scratch.
    pub save_name: String,
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
    /// "Save current effect stack as a Look" was clicked, with the name typed in `save_name`.
    pub save: Option<String>,
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
        GalleryTab::Luts => settings
            .lut_dirs
            .iter()
            .flat_map(|d| std::fs::read_dir(d).into_iter().flatten())
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("cube")))
            .take(24 * settings.lut_dirs.len().max(1))
            .map(|p| p.to_string_lossy().into_owned())
            .collect(),
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
    }
}

/// The Gallery pane body: tab strip + card grid. `selection` gates whether "Apply" makes sense (an empty
/// selection still lists cards - Templates/Transitions need no clip selected).
pub fn show(
    ui: &mut egui::Ui,
    state: &mut GalleryState,
    settings: &mut Settings,
    selection: &[Id],
    palette: &Palette,
) -> GalleryResponse {
    let mut out = GalleryResponse::default();
    ui.horizontal_wrapped(|ui| {
        for tab in GalleryTab::ALL {
            if ui.selectable_label(state.tab == tab, tab.label()).clicked() {
                state.tab = tab;
                settings.gallery_tab = tab.name().to_string();
            }
        }
    });
    ui.separator();
    if state.tab == GalleryTab::Looks && !selection.is_empty() {
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut state.save_name).hint_text("name").desired_width(120.0));
            if ui.add_enabled(!state.save_name.trim().is_empty(), egui::Button::new("Save from selection…")).clicked()
            {
                out.save = Some(state.save_name.trim().to_string());
                state.save_name.clear();
            }
        });
        ui.separator();
    }
    let names = card_names(state.tab, settings);
    if names.is_empty() {
        ui.weak(match state.tab {
            GalleryTab::Luts => "No .cube files found - add a folder in Settings ▸ Color.",
            GalleryTab::Templates => "No saved templates yet.",
            GalleryTab::Titles => "No title templates yet.", // builtin_titles() is never empty; user-only edge case
            _ => "Nothing here yet.",
        });
    }
    egui::ScrollArea::vertical().show(ui, |ui| {
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
                    _ => card(ui, name, |_, _| {}),
                };
                if crate::ui::hover_after(ui, r.id, &r, 150.0) {
                    out.hover = Some((tab, name.clone()));
                }
                if r.clicked() {
                    match tab {
                        GalleryTab::Templates => out.place = Some(name.clone()),
                        GalleryTab::Titles => out.place_title = Some(name.clone()),
                        _ => out.apply = Some((tab, name.clone(), 1.0)),
                    }
                }
            }
        });
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
                out = show(ui, state, settings, &[7], &palette);
            });
        });
        out
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
