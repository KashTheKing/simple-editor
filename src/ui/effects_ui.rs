//! Effects panel. Top: the CATALOGUE as a thumbnail grid — one ~96x54 card per `EffectKind` showing the
//! effect applied to a stock image with its name underneath, grouped by `EffectKind::category()` under
//! collapsible headers, with a search box. Thumbnails arrive through `set_thumbnail(kind, id, size)`,
//! which the app calls after rendering them on the GPU; without a GL context a card still shows its name
//! on a neutral tile (never blank) — an effect with `EffectKind::applies_to_audio()` instead gets a
//! solid green tile with a music-note glyph, since it has no picture to render. The catalogue is filtered
//! to what the first selected clip can use (audio clip -> audio effects only, and vice versa); with
//! nothing selected it shows everything. Every card is also a dnd drag source (`DragPayload::Effect`,
//! same mechanism as `ui::library`'s asset tiles) so it can be dropped onto a clip, alongside its
//! existing click-to-add. Clicking a card adds the effect to every eligible selected clip.
//! Below: the FIRST selected clip's effect stack, in order: for each effect a header row (enabled
//! checkbox, name, mask button, "Edit shader…" for a custom shader, copy/paste params, ▲ ▼ reorder,
//! ✕ remove) and its parameters from
//! `effect.specs()`: label + DragValue (range from ParamSpec, speed ≈ (max-min)/200) — or a CHECKBOX when
//! `EffectKind::is_bool_param` — + "◆" keyframe toggle at clip-local playhead time (Animated::toggle_key /
//! set_at, highlighted when a key exists) + "✕ keys". Tint shows a colour button bound to R/G/B as well.
//! An effect with a mask gets the inspector's mask grid inline (shape / position / radius / feather /
//! invert …) plus a ✕ to drop it. A clip that renders from a node graph greys the stack out: the graph
//! is what the renderer evaluates, so stack edits would be invisible.
//! Every change → undo once per gesture (same `edit_start` rule as the inspector).

use crate::model::{Animated, ClipKind, Effect, EffectKind, Id, Mask, Project};
use crate::settings::MotionPreset;
use crate::theme::Palette;
use crate::ui::{mask_grid, DragPayload, Gesture};
use eframe::egui::{self, Button, DragValue, Grid, Response, StrokeKind};
use std::cell::RefCell;
use std::collections::HashMap;

/// Card size of one catalogue thumbnail (the name is drawn under it).
pub const CARD: (f32, f32) = (96.0, 54.0);

thread_local! {
    // ponytail: thread_local hand-off because show() can't reach Settings — the app polls
    // take_pending_motion() each frame and stores the preset. Upgrade: pass an EffectsState if the
    // signature is ever allowed to grow.
    static PENDING_MOTION: RefCell<Option<MotionPreset>> = const { RefCell::new(None) };
    /// GPU-rendered catalogue thumbnails, uploaded by the app (empty without a GL context).
    static THUMBS: RefCell<HashMap<EffectKind, (egui::TextureId, [u32; 2])>> = RefCell::new(HashMap::new());
    /// One effect's parameters on the panel's private clipboard (Copy / Paste on the stack rows).
    static PARAM_CLIP: RefCell<Option<Effect>> = const { RefCell::new(None) };
}

/// A motion preset captured via "Save as motion preset…" waiting for the app to store it in Settings.
pub fn take_pending_motion() -> Option<MotionPreset> {
    PENDING_MOTION.with(|p| p.borrow_mut().take())
}

/// The app hands a rendered catalogue thumbnail to the panel (texture must stay alive while shown).
pub fn set_thumbnail(kind: EffectKind, texture: egui::TextureId, size: [u32; 2]) {
    THUMBS.with(|t| t.borrow_mut().insert(kind, (texture, size)));
}

/// Drop every catalogue thumbnail (stock image changed / GL context lost).
pub fn clear_thumbnails() {
    THUMBS.with(|t| t.borrow_mut().clear());
}

/// How many thumbnails the panel currently holds.
pub fn thumbnail_count() -> usize {
    THUMBS.with(|t| t.borrow().len())
}

pub(crate) fn thumbnail(kind: EffectKind) -> Option<(egui::TextureId, [u32; 2])> {
    THUMBS.with(|t| t.borrow().get(&kind).copied())
}

/// Catalogue categories in `EffectKind::ALL` order (first appearance wins), so the grid is stable.
fn categories() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = Vec::new();
    for k in EffectKind::ALL {
        if !v.contains(&k.category()) {
            v.push(k.category());
        }
    }
    v
}

fn matches(kind: EffectKind, q: &str) -> bool {
    if q.is_empty() {
        return true;
    }
    let q = q.to_lowercase();
    kind.name().to_lowercase().contains(&q) || kind.category().to_lowercase().contains(&q)
}

/// Whether `kind` belongs in the catalogue for the selected clip: `audio` is the first selected clip's
/// "is it an audio clip" (None = nothing selected, so nothing is filtered out).
fn fits_selection(kind: EffectKind, audio: Option<bool>) -> bool {
    audio.map_or(true, |a| kind.applies_to_audio() == a)
}

/// One catalogue card: the thumbnail (or a neutral named tile, or a green tile + note for an audio
/// effect) with the effect name underneath. Also a dnd drag source for `DragPayload::Effect`, same
/// pattern as `ui::library`'s `tile()` — the click-sensing `interact` is registered after the drag
/// source so a plain click still reaches it (egui hands the click to the topmost click-sensing widget).
fn effect_card(ui: &mut egui::Ui, kind: EffectKind, palette: &Palette) -> Response {
    let font = egui::TextStyle::Small.resolve(ui.style());
    let name_h = ui.text_style_height(&egui::TextStyle::Small);
    let id = ui.id().with(("fx_card", kind));
    let src = ui.dnd_drag_source(id, DragPayload::Effect(kind), |ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(CARD.0, CARD.1 + name_h + 2.0), egui::Sense::hover());
        let tile = egui::Rect::from_min_size(rect.min, egui::vec2(CARD.0, CARD.1));
        let p = ui.painter();
        if kind.applies_to_audio() {
            // no picture to render for an audio effect: a solid tile + note reads at a glance
            p.rect_filled(tile, 2.0, palette.clip_audio);
            crate::ui::tools::draw_glyph(p, tile, crate::ui::tools::Glyph::MusicNote, egui::Color32::WHITE);
        } else {
            match thumbnail(kind) {
                Some((tex, _)) => {
                    p.image(
                        tex,
                        tile,
                        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                }
                None => {
                    // no GL / not rendered yet: a neutral tile that still names the effect
                    p.rect_filled(tile, 2.0, palette.header);
                    let g = p.layout(kind.name().to_string(), font.clone(), palette.text_dim, CARD.0 - 8.0);
                    p.galley(tile.center() - g.size() / 2.0, g, palette.text_dim);
                }
            }
        }
        let border = if ui.rect_contains_pointer(tile) { palette.accent } else { palette.border };
        p.rect_stroke(tile, 2.0, egui::Stroke::new(1.0, border), StrokeKind::Inside);
        let label = p.layout_no_wrap(kind.name().to_string(), font, palette.text);
        let lx = (rect.left() + (CARD.0 - label.size().x) / 2.0).max(rect.left());
        p.with_clip_rect(egui::Rect::from_min_max(egui::pos2(rect.left(), tile.bottom()), rect.max)).galley(
            egui::pos2(lx, tile.bottom() + 2.0),
            label,
            palette.text,
        );
    });
    let r = ui.interact(src.response.rect, id.with("click"), egui::Sense::click());
    r.on_hover_text(format!("{} · {}", kind.name(), kind.category()))
}

/// Test-only registry of widget rects so headless tests can click real widgets without pixel-guessing.
#[cfg(test)]
pub(crate) mod test_rects {
    use eframe::egui::Rect;
    use std::cell::RefCell;
    thread_local! {
        static RECTS: RefCell<Vec<(String, Rect)>> = const { RefCell::new(Vec::new()) };
    }
    pub fn clear() {
        RECTS.with(|r| r.borrow_mut().clear());
    }
    pub fn push(name: String, rect: Rect) {
        RECTS.with(|r| r.borrow_mut().push((name, rect)));
    }
    pub fn get(name: &str) -> Option<Rect> {
        RECTS.with(|r| r.borrow().iter().rev().find(|(n, _)| n == name).map(|(_, rect)| *rect))
    }
}

fn key_buttons(ui: &mut egui::Ui, a: &mut Animated, lt: f64, palette: &Palette, g: &mut Gesture, _at: (usize, usize)) {
    let r = crate::ui::tools::icon_button(
        ui,
        palette,
        ui.id().with(("kf", _at)),
        crate::ui::tools::Glyph::Diamond,
        "Toggle keyframe at playhead",
        a.has_key_at(lt),
    );
    #[cfg(test)]
    test_rects::push(format!("kf{}_{}", _at.0, _at.1), r.rect);
    if r.clicked() {
        a.toggle_key(lt);
        g.click();
    }
    if a.is_animated() && ui.small_button("✕ keys").on_hover_text("Remove all keyframes").clicked() {
        a.clear_keys(lt);
        g.click();
    }
}

/// Unit suffix for a wobble parameter.
fn wobble_suffix(name: &str) -> &'static str {
    match name {
        "Amplitude X" | "Amplitude Y" => " px",
        "Roll" | "Yaw" | "Pitch" => "°",
        "Frequency" => " Hz",
        _ => "",
    }
}

#[derive(Default)]
pub struct EffectsResponse {
    pub edited: bool,
    /// Index (into the first selected clip's stack) of the effect whose mask the user wants to draw —
    /// the app switches to the mask tool and points the viewport at it.
    pub mask_for: Option<usize>,
    /// "Node editor" was clicked: the app opens the node pane for the selected clip.
    pub open_nodes: bool,
    /// Index of the `EffectKind::Shader` effect whose GLSL the user wants to edit (app opens the window).
    pub edit_shader: Option<usize>,
}

/// The catalogue grid: search box + collapsible category sections of thumbnail cards, filtered to what
/// the selected clip (`audio`) can use. Returns the effect a card click asked for.
fn catalogue(ui: &mut egui::Ui, palette: &Palette, audio: Option<bool>) -> Option<EffectKind> {
    let mut add = None;
    let search_id = ui.id().with("fx_search");
    let mut q: String = ui.ctx().data_mut(|d| d.get_temp(search_id).unwrap_or_default());
    ui.horizontal(|ui| {
        ui.strong("Effects");
        ui.add(egui::TextEdit::singleline(&mut q).hint_text("🔍 search").desired_width(f32::INFINITY));
    });
    let hits: Vec<EffectKind> =
        EffectKind::ALL.into_iter().filter(|&k| matches(k, &q) && fits_selection(k, audio)).collect();
    if hits.is_empty() {
        ui.weak(if audio == Some(true) { "No audio effects yet" } else { "No effect matches" });
    }
    for cat in categories() {
        let in_cat: Vec<EffectKind> = hits.iter().copied().filter(|k| k.category() == cat).collect();
        if in_cat.is_empty() {
            continue;
        }
        egui::CollapsingHeader::new(cat).id_salt(("fx_cat", cat)).default_open(true).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for kind in in_cat {
                    let r = effect_card(ui, kind, palette);
                    #[cfg(test)]
                    test_rects::push(format!("card_{}", kind.name()), r.rect);
                    if r.clicked() {
                        add = Some(kind);
                    }
                }
            });
        });
    }
    ui.ctx().data_mut(|d| d.insert_temp(search_id, q));
    add
}

pub fn show(
    ui: &mut egui::Ui,
    project: &mut Project,
    selection: &[Id],
    playhead: f64,
    palette: &Palette,
    undo: &mut dyn FnMut(&Project),
) -> EffectsResponse {
    let mut out = EffectsResponse::default();
    #[cfg(test)]
    test_rects::clear();

    // first selected clip that still exists — drives both the catalogue's audio-aware filter and the
    // stack panel below
    let first_id = selection.iter().find(|&&id| project.clip(id).is_some()).copied();
    let audio_sel = first_id.and_then(|id| project.clip(id)).map(|c| c.kind == ClipKind::Audio);

    // ---- catalogue ----
    if let Some(kind) = catalogue(ui, palette, audio_sel) {
        // a clip with a node graph renders from the graph, so anything pushed on its linear stack would
        // be invisible — those clips are skipped (the stack below is greyed out for the same reason).
        // each selected clip is checked on its own kind, not just the first (a mixed video+audio
        // selection can still get an eligible effect on every clip it applies to).
        let targets: Vec<Id> = selection
            .iter()
            .copied()
            .filter(|&id| {
                let Some(c) = project.clip(id) else { return false };
                (c.kind == ClipKind::Audio) == kind.applies_to_audio() && c.graph.is_none()
            })
            .collect();
        if !targets.is_empty() {
            undo(project);
            for id in targets {
                if let Some(c) = project.clip_mut(id) {
                    c.effects.push(Effect::new(kind));
                }
            }
            out.edited = true;
        }
    }

    // ---- stack of the first selected clip ----
    let Some(id) = first_id else {
        ui.separator();
        ui.label("Select a clip");
        return out;
    };
    // ponytail: edit a per-frame clone and write back (inspector pattern) so `undo` can snapshot first.
    let orig = project.clip(id).cloned();
    let Some(mut clip) = orig else { return out };
    // clamped: the playhead can sit off the clip, and keys written outside [0, duration] are invisible
    // in every editor yet still make the param "animated" forever (mixer.rs clamps the same way).
    let lt = clip.local(playhead).clamp(0.0, clip.duration);
    let mut g = Gesture::default();

    ui.separator();
    ui.horizontal(|ui| {
        ui.strong(clip.name.clone());
        let nodes = ui.small_button("Node editor").on_hover_text("Edit this clip's chain as a graph");
        #[cfg(test)]
        test_rects::push("nodes".into(), nodes.rect);
        if nodes.clicked() {
            out.open_nodes = true;
        }
    });
    if clip.graph.is_some() {
        // gpu::run_chain evaluates the graph and never looks at clip.effects — show the stack, but do
        // not let the user edit into the void
        ui.colored_label(palette.text_dim, "This clip renders from its node graph — edit it there.");
        ui.disable();
    }
    let n = clip.effects.len();
    let mut remove: Option<usize> = None;
    let mut swap: Option<(usize, usize)> = None;
    let mut copy: Option<usize> = None;
    let mut paste: Option<usize> = None;
    let copied_kind = PARAM_CLIP.with(|c| c.borrow().as_ref().map(|e| e.kind));
    for (i, fx) in clip.effects.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            g.note(&ui.checkbox(&mut fx.enabled, ""));
            ui.label(fx.kind.name());
            let masked = fx.mask.is_some();
            let mask = ui
                .add(Button::new(if masked { "Edit mask" } else { "Add mask" }).small())
                .on_hover_text("Limit this effect to a shape (drawn in the viewport)");
            #[cfg(test)]
            test_rects::push(format!("mask{i}"), mask.rect);
            if mask.clicked() {
                if !masked {
                    fx.mask = Some(Mask::default());
                    g.click();
                }
                out.mask_for = Some(i);
            }
            if masked {
                let rm = crate::ui::markers_ui::x_button(ui).on_hover_text("Remove this mask");
                #[cfg(test)]
                test_rects::push(format!("maskx{i}"), rm.rect);
                if rm.clicked() {
                    fx.mask = None;
                    g.click();
                }
            }
            if fx.kind == EffectKind::Shader {
                let ed = ui.add(Button::new("Edit shader…").small()).on_hover_text("Edit this effect's GLSL");
                #[cfg(test)]
                test_rects::push(format!("shader{i}"), ed.rect);
                if ed.clicked() {
                    out.edit_shader = Some(i);
                }
            }
            let cp = ui.add(Button::new("⧉").small()).on_hover_text("Copy these parameters");
            #[cfg(test)]
            test_rects::push(format!("copy{i}"), cp.rect);
            if cp.clicked() {
                copy = Some(i);
            }
            let can_paste = copied_kind == Some(fx.kind);
            let pt = ui
                .add_enabled(can_paste, Button::new("⤵").small())
                .on_hover_text("Paste copied parameters")
                .on_disabled_hover_text("Copy the same kind of effect first");
            #[cfg(test)]
            test_rects::push(format!("paste{i}"), pt.rect);
            if pt.clicked() {
                paste = Some(i);
            }
            let up = ui.add_enabled(i > 0, Button::new("▲").small());
            if up.clicked() {
                swap = Some((i, i - 1));
            }
            let down = ui.add_enabled(i + 1 < n, Button::new("▼").small());
            if down.clicked() {
                swap = Some((i, i + 1));
            }
            let del = ui.add(Button::new("✕").small());
            if del.clicked() {
                remove = Some(i);
            }
            #[cfg(test)]
            {
                test_rects::push(format!("up{i}"), up.rect);
                test_rects::push(format!("down{i}"), down.rect);
                test_rects::push(format!("del{i}"), del.rect);
            }
        });
        if fx.kind == EffectKind::Tint && fx.params.len() >= 3 {
            ui.horizontal(|ui| {
                ui.label("Colour");
                let mut rgb = [fx.params[0].at(lt) as u8, fx.params[1].at(lt) as u8, fx.params[2].at(lt) as u8];
                let r = ui.color_edit_button_srgb(&mut rgb);
                if r.changed() {
                    for (a, v) in fx.params.iter_mut().zip(rgb) {
                        a.set_at(lt, v as f64);
                    }
                }
                g.note(&r);
            });
        }
        // the effect's own mask: same grid as the inspector's clip mask, so it can be shaped without
        // the viewport (the mask tool only ever edits clip.mask)
        if let Some(m) = &mut fx.mask {
            let _r = ui.scope(|ui| mask_grid(ui, m, lt, palette, &mut g, egui::Id::new(("fx_mask", i)))).response;
            #[cfg(test)]
            test_rects::push(format!("maskgrid{i}"), _r.rect);
        }
        let kind = fx.kind;
        Grid::new(("fx_params", i)).num_columns(2).show(ui, |ui| {
            for (j, spec) in kind.params().iter().enumerate() {
                let Some(a) = fx.params.get_mut(j) else { continue };
                ui.label(spec.name);
                ui.horizontal(|ui| {
                    let mut v = a.at(lt);
                    let r = if kind == EffectKind::Wobble && spec.name == "Motion" {
                        // a named waveform reads better than 0..4 (Sine / Layered / Cubic / …)
                        let names = crate::model::WOBBLE_MOTIONS;
                        let cur = (v.round().clamp(0.0, (names.len() - 1) as f64)) as usize;
                        let mut sel = cur;
                        let inner = egui::ComboBox::from_id_salt(("wobble_motion", i))
                            .selected_text(names[cur])
                            .width(96.0)
                            .show_ui(ui, |ui| {
                                for (k, n) in names.iter().enumerate() {
                                    ui.selectable_value(&mut sel, k, *n);
                                }
                            });
                        let mut r = inner.response;
                        if sel != cur {
                            v = sel as f64;
                            r.mark_changed();
                        }
                        r
                    } else if kind.is_bool_param(j) {
                        // stored as 0/1 — a checkbox is the honest widget (Flip H/V, "Show mask", …)
                        let mut on = v >= 0.5;
                        let r = ui.checkbox(&mut on, "");
                        if r.changed() {
                            v = if on { 1.0 } else { 0.0 };
                        }
                        r
                    } else {
                        // clamp_existing_to_range(false): clamping an out-of-range stored value reports
                        // `changed()`, which would fake an edit just by drawing the panel.
                        let mut dv = DragValue::new(&mut v)
                            .range(spec.min..=spec.max)
                            .clamp_existing_to_range(false)
                            .speed((spec.max - spec.min) / 200.0);
                        if kind == EffectKind::Wobble {
                            dv = dv.suffix(wobble_suffix(spec.name));
                        }
                        ui.add(dv)
                    };
                    #[cfg(test)]
                    test_rects::push(format!("param{i}_{j}"), r.rect);
                    if r.changed() {
                        a.set_at(lt, v);
                    }
                    g.note(&r);
                    key_buttons(ui, a, lt, palette, &mut g, (i, j));
                });
                ui.end_row();
            }
        });
    }
    if let Some(i) = copy {
        PARAM_CLIP.with(|c| *c.borrow_mut() = clip.effects.get(i).cloned());
    }
    if let Some(i) = paste {
        let src = PARAM_CLIP.with(|c| c.borrow().clone());
        if let (Some(src), Some(dst)) = (src, clip.effects.get_mut(i)) {
            if src.kind == dst.kind {
                dst.params = src.params.clone();
                dst.shader = src.shader.clone();
                dst.mask = src.mask.clone();
                g.click();
            }
        }
    }
    if let Some((a, b)) = swap {
        clip.effects.swap(a, b);
        g.click();
    }
    if let Some(i) = remove {
        clip.effects.remove(i);
        if out.mask_for == Some(i) {
            out.mask_for = None;
        }
        g.click();
    }
    if clip.effects.is_empty() {
        ui.label("No effects — click a card above to add one");
    }

    // ---- save the clip's animation as a motion preset ----
    if clip.all_animated().iter().any(|a| a.is_animated()) {
        ui.separator();
        ui.horizontal(|ui| {
            let name_id = ui.id().with("motion_name");
            let mut name: String = ui.ctx().data_mut(|d| d.get_temp(name_id).unwrap_or_default());
            ui.add(egui::TextEdit::singleline(&mut name).hint_text("Preset name").desired_width(120.0));
            if ui.add_enabled(!name.trim().is_empty(), Button::new("Save as motion preset")).clicked() {
                let p = crate::engine::presets::capture_motion(name.trim(), &clip);
                PENDING_MOTION.with(|s| *s.borrow_mut() = Some(p));
                name.clear();
            }
            ui.ctx().data_mut(|d| d.insert_temp(name_id, name));
        });
    }

    if g.start {
        undo(project);
    }
    if g.changed {
        if let Some(c) = project.clip_mut(id) {
            *c = clip;
        }
    }
    out.edited |= g.changed;
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Clip, ClipKind};
    use eframe::egui::{vec2, Color32, Event, Modifiers, PointerButton, Pos2, RawInput, Rect};

    struct Harness {
        ctx: egui::Context,
        project: Project,
        selection: Vec<Id>,
        undos: usize,
        time: f64,
        playhead: f64,
        last: EffectsResponse,
    }

    impl Harness {
        fn new() -> Self {
            let mut project = Project::new();
            project.tracks[0].clips.push(Clip::new(7, ClipKind::Video, "v", 0.0, 4.0));
            Self {
                ctx: egui::Context::default(),
                project,
                selection: vec![7],
                undos: 0,
                time: 0.0,
                playhead: 1.0,
                last: EffectsResponse::default(),
            }
        }
        /// One update. egui may run the ui closure twice (multi-pass layout, e.g. the first frame a
        /// collapsing header appears), so the responses of every pass are folded together — a click
        /// landing in a discarded pass still mutated the project.
        fn frame(&mut self, events: Vec<Event>) -> bool {
            self.time += 0.05;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(500.0, 900.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let pal = Palette::new(true, Color32::WHITE);
            let Harness { ctx, project, selection, undos, playhead, last, .. } = self;
            let playhead = *playhead;
            let mut out = EffectsResponse::default();
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut undo = |_: &Project| *undos += 1;
                    let r = show(ui, project, selection, playhead, &pal, &mut undo);
                    out.edited |= r.edited;
                    out.open_nodes |= r.open_nodes;
                    out.mask_for = r.mask_for.or(out.mask_for);
                });
            });
            let edited = out.edited;
            *last = out;
            edited
        }
        fn click(&mut self, pos: Pos2) -> bool {
            let mut e = self.frame(vec![Event::PointerMoved(pos)]);
            e |= self.frame(vec![Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            }]);
            e |= self.frame(vec![Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            }]);
            e
        }
        fn clip(&self) -> &Clip {
            &self.project.tracks[0].clips[0]
        }
        fn rect(&self, name: &str) -> Rect {
            test_rects::get(name).unwrap_or_else(|| panic!("no widget rect for {name}"))
        }
    }

    #[test]
    fn catalogue_is_grouped_and_searchable() {
        let cats = categories();
        assert!(cats.contains(&"Adjustments") && cats.contains(&"Stylize") && cats.contains(&"Custom"));
        assert_eq!(cats.len(), cats.iter().collect::<std::collections::HashSet<_>>().len(), "no duplicates");
        assert!(matches(EffectKind::Blur, ""));
        assert!(matches(EffectKind::Blur, "blu"));
        assert!(matches(EffectKind::Blur, "STYL"), "category matches too");
        assert!(!matches(EffectKind::Blur, "chroma"));
    }

    /// The grid renders without any thumbnail (no GL) and with one, and stays clickable both ways.
    #[test]
    fn grid_renders_with_and_without_thumbnails() {
        clear_thumbnails();
        let mut h = Harness::new();
        h.frame(vec![]);
        let bare = h.rect("card_Blur");
        assert!(bare.width() >= CARD.0 - 1.0 && bare.height() >= CARD.1, "card is card-sized: {bare:?}");
        assert_eq!(thumbnail_count(), 0);

        set_thumbnail(EffectKind::Blur, egui::TextureId::Managed(1), [96, 54]);
        assert_eq!(thumbnail_count(), 1);
        h.frame(vec![]);
        let with = h.rect("card_Blur");
        assert_eq!((with.width(), with.height()), (bare.width(), bare.height()), "same layout either way");
        clear_thumbnails();
    }

    #[test]
    fn card_click_adds_effect_with_one_undo() {
        clear_thumbnails();
        let mut h = Harness::new();
        h.frame(vec![]);
        let r = h.rect("card_Blur");
        let edited = h.click(r.center());
        assert!(edited, "clicking the Blur card adds an effect");
        assert_eq!(h.clip().effects.len(), 1);
        assert_eq!(h.clip().effects[0].kind, EffectKind::Blur);
        assert_eq!(h.undos, 1);
    }

    #[test]
    fn stack_reorder_with_one_undo() {
        let mut h = Harness::new();
        {
            let c = &mut h.project.tracks[0].clips[0];
            c.effects.push(Effect::new(EffectKind::Blur));
            c.effects.push(Effect::new(EffectKind::Invert));
        }
        h.frame(vec![]);
        let r = h.rect("down0");
        assert!(h.click(r.center()));
        assert_eq!(h.clip().effects[0].kind, EffectKind::Invert);
        assert_eq!(h.clip().effects[1].kind, EffectKind::Blur);
        assert_eq!(h.undos, 1);
    }

    #[test]
    fn stack_remove_with_one_undo() {
        let mut h = Harness::new();
        h.project.tracks[0].clips[0].effects.push(Effect::new(EffectKind::Blur));
        h.frame(vec![]);
        let r = h.rect("del0");
        assert!(h.click(r.center()));
        assert!(h.clip().effects.is_empty());
        assert_eq!(h.undos, 1);
    }

    #[test]
    fn param_drag_edits_with_one_undo() {
        let mut h = Harness::new();
        h.project.tracks[0].clips[0].effects.push(Effect::new(EffectKind::Blur));
        h.frame(vec![]);
        let r = h.rect("param0_0");
        let from = r.center();
        h.frame(vec![Event::PointerMoved(from)]);
        h.frame(vec![Event::PointerButton {
            pos: from,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        }]);
        let mut edited = false;
        for i in 1..=4 {
            let p = from + vec2(10.0 * i as f32, 0.0);
            edited |= h.frame(vec![Event::PointerMoved(p)]);
        }
        edited |= h.frame(vec![Event::PointerButton {
            pos: from + vec2(40.0, 0.0),
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }]);
        edited |= h.frame(vec![]);
        assert!(edited, "dragging the Blur radius should edit the project");
        let radius = h.clip().effects[0].params[0].value;
        assert!(radius > 8.0, "radius should have grown from the default: {radius}");
        assert_eq!(h.undos, 1, "one drag gesture = one undo");
    }

    /// Flip's two parameters are checkboxes, and toggling one writes 0/1.
    #[test]
    fn bool_params_are_checkboxes() {
        assert!(EffectKind::Flip.is_bool_param(0) && EffectKind::Flip.is_bool_param(1));
        let mut h = Harness::new();
        h.project.tracks[0].clips[0].effects.push(Effect::new(EffectKind::Flip));
        h.frame(vec![]);
        let r = h.rect("param0_0");
        assert!(r.width() < 30.0, "a checkbox, not a drag value: {r:?}");
        assert_eq!(h.clip().effects[0].params[0].value, 1.0);
        assert!(h.click(r.center()));
        assert_eq!(h.clip().effects[0].params[0].value, 0.0, "toggled off");
        assert_eq!(h.undos, 1);
    }

    #[test]
    fn mask_button_creates_a_mask_and_reports_the_index() {
        let mut h = Harness::new();
        h.project.tracks[0].clips[0].effects.push(Effect::new(EffectKind::Blur));
        h.frame(vec![]);
        let r = h.rect("mask0");
        h.click(r.center());
        assert_eq!(h.last.mask_for, Some(0), "the app is told which effect to mask");
        assert!(h.clip().effects[0].mask.is_some(), "a mask was created");
        assert_eq!(h.undos, 1);
    }

    #[test]
    fn a_masked_effect_gets_the_mask_grid_and_a_remove_button() {
        let mut h = Harness::new();
        let mut fx = Effect::new(EffectKind::Blur);
        fx.mask = Some(Mask::default());
        h.project.tracks[0].clips[0].effects.push(fx);
        h.frame(vec![]);
        assert!(h.rect("maskgrid0").height() > 0.0, "the mask parameters are on the panel, not viewport-only");
        let r = h.rect("maskx0");
        assert!(h.click(r.center()), "✕ drops the mask");
        assert!(h.clip().effects[0].mask.is_none());
        assert_eq!(h.undos, 1);
    }

    #[test]
    fn a_clip_with_a_node_graph_takes_no_stack_edits() {
        clear_thumbnails();
        let mut h = Harness::new();
        h.project.ensure_graph(7);
        h.frame(vec![]);
        let r = h.rect("card_Blur");
        // gpu::run_chain evaluates the graph and never reads clip.effects — the add would be invisible
        assert!(!h.click(r.center()));
        assert!(h.clip().effects.is_empty());
        assert_eq!(h.undos, 0);
    }

    #[test]
    fn node_editor_button_sets_the_flag() {
        let mut h = Harness::new();
        h.frame(vec![]);
        let r = h.rect("nodes");
        h.click(r.center());
        assert!(h.last.open_nodes);
        assert_eq!(h.undos, 0, "opening the node editor is not an edit");
    }

    #[test]
    fn copy_paste_one_parameter_set() {
        PARAM_CLIP.with(|c| *c.borrow_mut() = None);
        let mut h = Harness::new();
        {
            let c = &mut h.project.tracks[0].clips[0];
            let mut a = Effect::new(EffectKind::Blur);
            a.params[0].value = 42.0;
            c.effects.push(a);
            c.effects.push(Effect::new(EffectKind::Blur));
        }
        h.frame(vec![]);
        let cp = h.rect("copy0");
        h.click(cp.center());
        assert_eq!(h.undos, 0, "copying is not an edit");
        h.frame(vec![]);
        let pt = h.rect("paste1");
        h.click(pt.center());
        assert_eq!(h.clip().effects[1].params[0].value, 42.0, "parameters pasted");
        assert_eq!(h.undos, 1, "pasting is one undo");
        PARAM_CLIP.with(|c| *c.borrow_mut() = None);
    }

    #[test]
    fn keyframe_button_off_clip_keys_inside_the_clip() {
        let mut h = Harness::new();
        h.project.tracks[0].clips[0].start = 10.0; // clip lives at 10..14
        h.project.tracks[0].clips[0].effects.push(Effect::new(EffectKind::Blur));
        h.playhead = 0.0; // playhead well before the clip → local time -10
        h.frame(vec![]);
        let r = h.rect("kf0_0");
        assert!(h.click(r.center()));
        let keys = &h.clip().effects[0].params[0].keys;
        assert_eq!(keys.len(), 1);
        assert!(keys[0].t >= 0.0 && keys[0].t <= 4.0, "key must land inside the clip, got t = {}", keys[0].t);
    }

    #[test]
    fn no_selection_is_harmless() {
        let mut h = Harness::new();
        h.selection.clear();
        assert!(!h.frame(vec![]));
        assert_eq!(h.undos, 0);
    }

    /// No `EffectKind` applies to audio yet (see `EffectKind::applies_to_audio`), so selecting an audio
    /// clip must hide every pixel-effect card rather than leave them there to silently no-op on click.
    #[test]
    fn audio_clip_filters_the_catalogue() {
        clear_thumbnails();
        let mut h = Harness::new();
        h.project.tracks[0].clips[0].kind = ClipKind::Audio;
        h.frame(vec![]);
        assert!(test_rects::get("card_Blur").is_none(), "a pixel effect must not be offered for an audio clip");
    }

    #[test]
    fn pending_motion_handoff() {
        assert!(take_pending_motion().is_none());
        PENDING_MOTION.with(|s| *s.borrow_mut() = Some(MotionPreset { name: "m".into(), props: Vec::new() }));
        assert_eq!(take_pending_motion().map(|m| m.name), Some("m".into()));
        assert!(take_pending_motion().is_none());
    }
}
