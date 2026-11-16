//! Effects: the applied STACK the Inspector's Effects section draws, plus the catalogue helpers the
//! Gallery's Effects tab (`ui::gallery`) uses - category / search / audio-fit filters and the GPU
//! thumbnails handed over through `set_thumbnail` / `set_library_thumbnail` (without a GL context a card
//! still shows its name on a neutral tile; an effect with `EffectKind::applies_to_audio()` gets a solid
//! green tile with a music-note glyph, since it has no picture to render). The catalogue is filtered to
//! what the first selected clip can use (audio clip -> audio effects only, and vice versa); with nothing
//! selected it shows everything.
//! Stack (`stack`, the Inspector's Effects section - the one place it is edited): the FIRST selected
//! clip's effects in order, one row each - drag handle · enable · name (click folds its parameters) -
//! with Add/Edit mask, Remove mask, Edit shader…, Copy / Paste parameters, Move up / down, Open in Node
//! editor and Remove on the row's right-click. A row's body holds a "From … for …" window inside the
//! clip (0 length = to the end of it), then label + DragValue (range from ParamSpec, speed ≈
//! (max-min)/200) - or a CHECKBOX when `EffectKind::is_bool_param` - + the shared ◆ keyframe control
//! (`ui::key_buttons`, right-click for keys and links). Tint shows a colour button bound to R/G/B too.
//! An effect with a mask gets the inspector's mask grid inline (shape / position / radius / feather /
//! invert …). A clip that renders from a REAL node graph (`Clip::uses_graph`) greys the stack out - the
//! graph is what the renderer evaluates, so stack edits would be invisible.
//! Every change → undo once per gesture (same `edit_start` rule as the inspector).

use crate::model::{Effect, EffectKind, Id, Mask, Project};
use crate::theme::Palette;
use crate::ui::{key_buttons, key_menu, mask_grid, menu, Gesture};
use eframe::egui::{self, DragValue, Grid};
use std::cell::RefCell;
use std::collections::HashMap;

// ---- ws:inspector-gallery ----
/// Local (non-`DragPayload`) drag identity for one effect-stack row: the dragged row's
/// `stable_effect_key` at drag-start. Scoped to this file's own stack UI - the shared
/// `crate::ui::DragPayload` enum (verified `src/ui/mod.rs`) has no slot-identifying variant, which would
/// be ambiguous when a clip has two same-kind effects.
struct EffectDragId(String);

/// Synthesized fold/drag identity for the effect at `effects[i]`, since `struct Effect` has no `id`
/// field (verified, and no upstream workstream adds one - see the plan's risk table): `(kind name,
/// same-kind rank among the effects BEFORE it in the current stack order)`. Stable across everything
/// that doesn't change same-kind relative order (enable/disable, param edits, adding/removing a
/// DIFFERENT kind, reordering this kind relative to other kinds) - the one thing it can't track is two
/// same-kind effects swapping places with each other, a documented `// ponytail:` ceiling (upgrade path:
/// a real `Effect.id` in one line, if a later workstream adds the field).
fn stable_effect_key(effects: &[Effect], i: usize) -> String {
    let kind = effects[i].kind;
    let rank = effects[..i].iter().filter(|e| e.kind == kind).count();
    format!("{kind:?}#{rank}")
}

thread_local! {
    /// GPU-rendered catalogue thumbnails, uploaded by the app (empty without a GL context).
    static THUMBS: RefCell<HashMap<EffectKind, (egui::TextureId, [u32; 2])>> = RefCell::new(HashMap::new());
    /// One effect's parameters on the panel's private clipboard (Copy / Paste on the stack rows).
    static PARAM_CLIP: RefCell<Option<Effect>> = const { RefCell::new(None) };
}

/// The app hands a rendered catalogue thumbnail to the panel (texture must stay alive while shown).
pub fn set_thumbnail(kind: EffectKind, texture: egui::TextureId, size: [u32; 2]) {
    THUMBS.with(|t| t.borrow_mut().insert(kind, (texture, size)));
}

/// Drop every catalogue thumbnail (stock image changed / GL context lost).
pub fn clear_thumbnails() {
    THUMBS.with(|t| t.borrow_mut().clear());
}

pub(crate) fn thumbnail(kind: EffectKind) -> Option<(egui::TextureId, [u32; 2])> {
    THUMBS.with(|t| t.borrow().get(&kind).copied())
}

/// Catalogue categories in `EffectKind::ALL` order (first appearance wins), so the grid is stable.
pub(crate) fn categories() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = Vec::new();
    for k in EffectKind::ALL {
        if !v.contains(&k.category()) {
            v.push(k.category());
        }
    }
    v
}

/// Whether `kind` belongs in the catalogue for the selected clip: `audio` is the first selected clip's
/// "is it an audio clip" (None = nothing selected, so nothing is filtered out).
pub(crate) fn fits_selection(kind: EffectKind, audio: Option<bool>) -> bool {
    audio.map_or(true, |a| kind.applies_to_audio() == a)
}

/// Test-only registry of widget rects so headless tests can click real widgets without pixel-guessing.
#[cfg(test)]
pub(crate) mod test_rects {
    use eframe::egui::Rect;
    use std::cell::RefCell;
    thread_local! {
        static RECTS: RefCell<Vec<(String, Rect)>> = const { RefCell::new(Vec::new()) };
    }
    pub fn push(name: String, rect: Rect) {
        RECTS.with(|r| r.borrow_mut().push((name, rect)));
    }
    pub fn get(name: &str) -> Option<Rect> {
        RECTS.with(|r| r.borrow().iter().rev().find(|(n, _)| n == name).map(|(_, rect)| *rect))
    }
}

// ---- ws:shader-library ----
/// Plain (not keyframed) rows for a shader's header-declared controls - label + a DragValue over the
/// header's range - inside a 2-column `Grid`. `vals` is grown to `specs.len()` with the defaults.
/// Shader transitions use it in the Inspector and the Gallery's Transitions tab.
pub(crate) fn plain_param_rows(
    ui: &mut egui::Ui,
    specs: &[crate::model::ParamSpec],
    vals: &mut Vec<f64>,
    g: &mut Gesture,
) {
    for (j, s) in specs.iter().enumerate() {
        if vals.len() <= j {
            vals.push(s.default);
        }
        ui.label(s.name);
        let dv = DragValue::new(&mut vals[j]).range(s.min..=s.max).clamp_existing_to_range(false);
        g.note(&ui.add(dv.speed((s.max - s.min) / 200.0)));
        ui.end_row();
    }
}

thread_local! {
    /// GPU thumbnails of installed library effects, by library id (`App::build_library_effect_thumbs`).
    static LIB_THUMBS: RefCell<HashMap<String, egui::TextureId>> = RefCell::new(HashMap::new());
}

/// The app hands over a rendered library-effect thumbnail.
pub fn set_library_thumbnail(id: &str, texture: egui::TextureId) {
    LIB_THUMBS.with(|t| t.borrow_mut().insert(id.to_string(), texture));
}

/// The rendered thumbnail of an installed library effect (by file id), if the GPU has made one yet.
pub(crate) fn library_thumbnail(id: &str) -> Option<egui::TextureId> {
    LIB_THUMBS.with(|t| t.borrow().get(id).copied())
}

pub(crate) fn has_library_thumbnail(id: &str) -> bool {
    LIB_THUMBS.with(|t| t.borrow().contains_key(id))
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
    /// Index (into the first selected clip's stack) of the effect whose mask the user wants to draw -
    /// the app switches to the mask tool and points the viewport at it.
    pub mask_for: Option<usize>,
    /// "Save to Gallery…" was picked on this stack row.
    pub save_effect: Option<usize>,
    /// "Open in Node editor" was picked: the app opens the node pane for the selected clip.
    pub open_nodes: bool,
    /// Index of the `EffectKind::Shader` effect whose GLSL the user wants to edit (app opens the window).
    pub edit_shader: Option<usize>,
    // ---- ws:registries-schema-hooks ----
    /// Index of the stack row the pointer is hovering, for a future async GPU hover preview
    /// (ws:inspector-gallery, wave 2, via `App::alt_render`). Unread this wave.
    #[allow(dead_code)]
    pub hover: Option<usize>,
}

/// A stack row's right-click pick, applied once the rows are drawn.
#[derive(Clone, Copy, PartialEq)]
enum RowCmd {
    AddMask,
    RemoveMask,
    Copy,
    Paste,
    Move(usize),
    Remove,
}

/// The applied effect stack of the first selected clip - drawn inside the Inspector's Effects section,
/// the one place it is edited. One row per effect: drag handle · enable · name (click to fold the
/// parameters); everything else is on the row's right-click. Parameter edits bulk-apply to the other
/// selected clips' same-index, same-kind effect.
pub fn stack(
    ui: &mut egui::Ui,
    project: &mut Project,
    selection: &[Id],
    playhead: f64,
    palette: &Palette,
    undo: &mut dyn FnMut(&Project),
) -> EffectsResponse {
    let mut out = EffectsResponse::default();
    let Some(id) = selection.iter().find(|&&id| project.clip(id).is_some()).copied() else { return out };
    // ponytail: edit a per-frame clone and write back (inspector pattern) so `undo` can snapshot first.
    let Some(mut clip) = project.clip(id).cloned() else { return out };
    // clamped: the playhead can sit off the clip, and keys written outside [0, duration] are invisible
    // in every editor yet still make the param "animated" forever (mixer.rs clamps the same way).
    let lt = clip.local(playhead).clamp(0.0, clip.duration);
    let mut g = Gesture::default();

    if clip.uses_graph() {
        // gpu::run_chain evaluates the graph and never looks at clip.effects - show the stack, but do
        // not let the user edit into the void; the section's right-click unlinks it in one click
        ui.colored_label(palette.text_dim, "Renders from its node graph - right-click Effects to unlink.");
        ui.disable();
    }
    let n = clip.effects.len();
    let dur = clip.duration;
    // a mask shapes pixels - an audio clip's filters get no mask controls at all
    let maskable = clip.is_visual();
    let mut cmd: Option<(usize, RowCmd)> = None;
    let mut drag_move: Option<(usize, usize)> = None;
    // (stack index, param name, new absolute value) - propagated to same-kind/same-index siblings when
    // more than one clip is selected (bulk edit; absolute overwrite, not a relative delta).
    let mut param_edits: Vec<(usize, String, f64)> = Vec::new();
    let copied_kind = PARAM_CLIP.with(|c| c.borrow().as_ref().map(|e| e.kind));
    // stable per-row identity (see `stable_effect_key`'s doc comment) computed once for the whole stack,
    // so the drag payload and the drop target agree on the same keys within one frame.
    let keys: Vec<String> = (0..clip.effects.len()).map(|i| stable_effect_key(&clip.effects, i)).collect();
    for (i, fx) in clip.effects.iter_mut().enumerate() {
        // one CollapsingState per effect, keyed by its STABLE key (not stack index `i`) so fold state
        // (and, via `Settings`-free egui memory, nothing else) survives a reorder - see `stable_effect_key`.
        let header_id = ui.id().with(("fx_stack", &keys[i]));
        let header = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), header_id, true)
            .show_header(ui, |ui| {
                // drag handle: a small grip, its own Sense::drag widget (not the whole row - that would
                // fight the checkbox and the name for clicks). Local payload (EffectDragId), never the
                // shared `DragPayload` enum - see this file's module doc comment on why.
                let (handle_rect, handle) =
                    ui.allocate_exact_size(egui::vec2(12.0, ui.spacing().interact_size.y), egui::Sense::drag());
                ui.painter().text(
                    handle_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "⠿",
                    egui::FontId::proportional(13.0),
                    ui.visuals().weak_text_color(),
                );
                handle.dnd_set_drag_payload(EffectDragId(keys[i].clone()));
                let _handle = handle.on_hover_cursor(egui::CursorIcon::Grab).on_hover_text("Drag to reorder");
                #[cfg(test)]
                test_rects::push(format!("draghandle{i}"), _handle.rect);
                g.note(&ui.checkbox(&mut fx.enabled, "").on_hover_text("Enabled"));
                // the name fills the rest of the row: a click folds the parameters, a right-click has the rest
                let h = ui.spacing().interact_size.y;
                let (rect, r) = ui.allocate_exact_size(egui::vec2(ui.available_width(), h), egui::Sense::click());
                let mut name = fx.name();
                if fx.mask.is_some() && maskable {
                    name.push_str("  · masked");
                }
                let color = if fx.enabled { ui.visuals().text_color() } else { ui.visuals().weak_text_color() };
                let font = egui::TextStyle::Body.resolve(ui.style());
                ui.painter().text(rect.left_center(), egui::Align2::LEFT_CENTER, name, font, color);
                r
            });
        let (_, header_r, _) = header.body(|ui| {
            // when it runs inside the clip - 0 length means "to the end", which is what every effect
            // that predates this row already says
            ui.horizontal(|ui| {
                ui.label("From");
                let r = ui.add(
                    DragValue::new(&mut fx.start)
                        .range(0.0..=dur)
                        .clamp_existing_to_range(false)
                        .speed(dur / 200.0)
                        .suffix(" s"),
                );
                #[cfg(test)]
                test_rects::push(format!("fxstart{i}"), r.rect);
                g.note(&r);
                ui.label("for");
                let r = ui
                    .add(
                        DragValue::new(&mut fx.len)
                            .range(0.0..=dur)
                            .clamp_existing_to_range(false)
                            .speed(dur / 200.0)
                            .suffix(" s"),
                    )
                    .on_hover_text("0 = to the end of the clip");
                #[cfg(test)]
                test_rects::push(format!("fxlen{i}"), r.rect);
                g.note(&r);
                if fx.len <= 0.0 {
                    ui.weak("(rest of the clip)");
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
            // the effect's own mask: same grid as the inspector's clip mask, so it can be shaped
            // without the viewport (the mask tool only ever edits clip.mask)
            if let Some(m) = fx.mask.as_mut().filter(|_| maskable) {
                let _r = ui.scope(|ui| mask_grid(ui, m, lt, palette, &mut g, egui::Id::new(("fx_mask", i)))).response;
                #[cfg(test)]
                test_rects::push(format!("maskgrid{i}"), _r.rect);
            }
            let kind = fx.kind;
            let specs = fx.specs();
            Grid::new(("fx_params", i)).num_columns(2).show(ui, |ui| {
                for (j, spec) in specs.iter().enumerate() {
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
                            // stored as 0/1 - a checkbox is the honest widget (Flip H/V, "Show mask", …)
                            let mut on = v >= 0.5;
                            let r = ui.checkbox(&mut on, "");
                            if r.changed() {
                                v = if on { 1.0 } else { 0.0 };
                            }
                            r
                        } else {
                            // clamp_existing_to_range(false): clamping an out-of-range stored value
                            // reports `changed()`, which would fake an edit just by drawing the panel.
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
                            param_edits.push((i, spec.name.to_string(), v));
                        }
                        g.note(&r);
                        menu::context(&r, |ui| key_menu(ui, a, lt, &mut g, spec.name, &[]));
                        let _kf = key_buttons(ui, a, lt, palette, &mut g, spec.name, &[]);
                        #[cfg(test)]
                        test_rects::push(format!("kf{i}_{j}"), _kf.rect);
                    });
                    ui.end_row();
                }
            });
        });
        let (row_r, name_r) = (header_r.response, header_r.inner);
        #[cfg(test)]
        test_rects::push(format!("row{i}"), name_r.rect);
        if name_r.clicked() {
            if let Some(mut st) = egui::collapsing_header::CollapsingState::load(ui.ctx(), header_id) {
                st.toggle(ui);
                st.store(ui.ctx());
            }
        }
        let masked = fx.mask.is_some();
        let kind = fx.kind;
        menu::context(&name_r, |ui| {
            use crate::ui::tools::{Dir, Glyph};
            let mut pick = |ui: &mut egui::Ui, on: bool, glyph: Option<Glyph>, label: &str, c: RowCmd| {
                if ui.add_enabled_ui(on, |ui| menu::row(ui, glyph, label, "")).inner.clicked() {
                    cmd = Some((i, c));
                }
            };
            if maskable {
                pick(ui, true, Some(Glyph::Mask), if masked { "Edit mask" } else { "Add mask" }, RowCmd::AddMask);
                pick(ui, masked, None, "Remove mask", RowCmd::RemoveMask);
            }
            if kind == EffectKind::Shader && menu::row(ui, Some(Glyph::Pencil), "Edit shader…", "").clicked() {
                out.edit_shader = Some(i);
            }
            if menu::row(ui, None, "Save to Gallery…", "").clicked() {
                out.save_effect = Some(i);
            }
            ui.separator();
            pick(ui, true, Some(Glyph::Copy), "Copy parameters", RowCmd::Copy);
            pick(ui, copied_kind == Some(kind), Some(Glyph::Paste), "Paste parameters", RowCmd::Paste);
            ui.separator();
            pick(ui, i > 0, Some(Glyph::Tri(Dir::Up)), "Move up", RowCmd::Move(i.wrapping_sub(1)));
            pick(ui, i + 1 < n, Some(Glyph::Tri(Dir::Down)), "Move down", RowCmd::Move(i + 1));
            if menu::row(ui, Some(Glyph::Nodes), "Open in Node editor", "").clicked() {
                out.open_nodes = true;
            }
            ui.separator();
            pick(ui, true, Some(Glyph::Cross), "Remove", RowCmd::Remove);
        });
        // >=150ms hover on the row wires the wave-0 `EffectsResponse.hover` stub (real async GPU
        // preview via `App.alt_render` lands where the caller wires `EffectsResponse`, not here).
        if crate::ui::hover_after(ui, header_id, &row_r, 150.0) {
            out.hover = Some(i);
        }
        // drop target: did a drag payload (this same stack's grip handle) land on this row this frame?
        if let Some(payload) = row_r.dnd_release_payload::<EffectDragId>() {
            if let Some(from) = keys.iter().position(|k| *k == payload.0) {
                if from != i {
                    drag_move = Some((from, i));
                }
            }
        }
    }
    match cmd {
        Some((i, RowCmd::AddMask)) => {
            if let Some(fx) = clip.effects.get_mut(i) {
                if fx.mask.is_none() {
                    fx.mask = Some(Mask::default());
                    g.click();
                }
                out.mask_for = Some(i);
            }
        }
        Some((i, RowCmd::RemoveMask)) => {
            if let Some(fx) = clip.effects.get_mut(i) {
                fx.mask = None;
                g.click();
            }
        }
        Some((i, RowCmd::Copy)) => PARAM_CLIP.with(|c| *c.borrow_mut() = clip.effects.get(i).cloned()),
        Some((i, RowCmd::Paste)) => {
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
        Some((i, RowCmd::Move(j))) if j < n => {
            clip.effects.swap(i, j);
            g.click();
        }
        Some((i, RowCmd::Remove)) if i < n => {
            clip.effects.remove(i);
            g.click();
        }
        _ => {}
    }
    if let Some((from, to)) = drag_move {
        let e = clip.effects.remove(from);
        clip.effects.insert(to, e);
        g.click();
    }
    if clip.effects.is_empty() {
        ui.weak("No effects - pick one in the Gallery's Effects tab.");
    }

    if g.start {
        undo(project);
    }
    if g.changed {
        if let Some(c) = project.clip_mut(id) {
            c.effects = clip.effects;
        }
    }
    // ---- ws:inspector-gallery ----
    // Bulk propagation: a param edit at stack index `i` on the representative clip (`id`, first) is
    // pushed as an absolute value (not a relative delta) onto every OTHER selected clip whose effect at
    // that same index shares its kind - `Project::bulk_set_effect_params` already skips a mismatch.
    if !param_edits.is_empty() && selection.len() > 1 {
        let ids: Vec<Id> = std::iter::once(id).chain(selection.iter().copied().filter(|&s| s != id)).collect();
        for (index, name, value) in param_edits {
            let mut params = std::collections::HashMap::new();
            params.insert(name, value);
            project.bulk_set_effect_params(&ids, index, &params);
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
        shapes: Vec<egui::epaint::ClippedShape>,
        /// Screen height (a long catalogue needs more room to stay clickable).
        screen_h: f32,
    }

    impl Harness {
        fn new() -> Self {
            let mut project = Project::new();
            project.tracks[0].clips.push(Clip::new(7, ClipKind::Video, "v", 0.0, 4.0));
            let ctx = egui::Context::default();
            ctx.set_fonts(crate::theme::test_fonts()); // size-diet: no default_fonts feature anymore
            Self {
                ctx,
                project,
                selection: vec![7],
                undos: 0,
                time: 0.0,
                playhead: 1.0,
                last: EffectsResponse::default(),
                shapes: Vec::new(),
                screen_h: 900.0,
            }
        }
        /// One update. egui may run the ui closure twice (multi-pass layout, e.g. the first frame a
        /// collapsing header appears), so the responses of every pass are folded together - a click
        /// landing in a discarded pass still mutated the project.
        fn frame(&mut self, events: Vec<Event>) -> bool {
            self.time += 0.05;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, self.screen_h))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let pal = Palette::new(true, Color32::WHITE);
            let Harness { ctx, project, selection, undos, playhead, last, shapes, .. } = self;
            let playhead = *playhead;
            let mut out = EffectsResponse::default();
            let full = ctx.run(input, |ctx| {
                let mut undo = |_: &Project| *undos += 1;
                egui::CentralPanel::default().show(ctx, |ui| {
                    let s = stack(ui, project, selection, playhead, &pal, &mut undo);
                    out.edited |= s.edited;
                    out.open_nodes |= s.open_nodes;
                    out.mask_for = s.mask_for.or(out.mask_for);
                    out.hover = s.hover.or(out.hover);
                });
            });
            *shapes = full.shapes;
            let edited = out.edited;
            *last = out;
            edited
        }
        /// Centre of the painted text that reads exactly `label`.
        fn text_eq(&self, label: &str) -> Option<Pos2> {
            self.shapes.iter().rev().find_map(|c| match &c.shape {
                egui::epaint::Shape::Text(t) if t.galley.text() == label => Some(t.visual_bounding_rect().center()),
                _ => None,
            })
        }
        /// Right-click stack row `i`, let its menu draw, and click `item`. True if anything was edited.
        fn row_menu(&mut self, i: usize, item: &str) -> bool {
            let at = self.rect(&format!("row{i}")).center();
            let mut e = self.frame(vec![Event::PointerMoved(at)]);
            e |= self.button(at, PointerButton::Secondary, true);
            e |= self.button(at, PointerButton::Secondary, false);
            e |= self.frame(vec![]);
            let pos = self.text_eq(item).unwrap_or_else(|| panic!("no menu row '{item}'"));
            self.time += 1.0;
            e | self.click(pos)
        }
        fn button(&mut self, pos: Pos2, button: PointerButton, pressed: bool) -> bool {
            self.frame(vec![Event::PointerButton { pos, button, pressed, modifiers: Modifiers::NONE }])
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
    fn stack_reorder_with_one_undo() {
        let mut h = Harness::new();
        {
            let c = &mut h.project.tracks[0].clips[0];
            c.effects.push(Effect::new(EffectKind::Blur));
            c.effects.push(Effect::new(EffectKind::Invert));
        }
        h.frame(vec![]);
        assert!(h.row_menu(0, "Move down"));
        assert_eq!(h.clip().effects[0].kind, EffectKind::Invert);
        assert_eq!(h.clip().effects[1].kind, EffectKind::Blur);
        assert_eq!(h.undos, 1);
    }

    #[test]
    fn stack_remove_with_one_undo() {
        let mut h = Harness::new();
        h.project.tracks[0].clips[0].effects.push(Effect::new(EffectKind::Blur));
        h.frame(vec![]);
        assert!(h.row_menu(0, "Remove"));
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
        h.row_menu(0, "Add mask");
        assert_eq!(h.last.mask_for, Some(0), "the app is told which effect to mask");
        assert!(h.clip().effects[0].mask.is_some(), "a mask was created");
        assert_eq!(h.undos, 1);
    }

    #[test]
    fn a_masked_effect_gets_the_mask_grid_and_a_remove_item() {
        let mut h = Harness::new();
        let mut fx = Effect::new(EffectKind::Blur);
        fx.mask = Some(Mask::default());
        h.project.tracks[0].clips[0].effects.push(fx);
        h.frame(vec![]);
        assert!(h.rect("maskgrid0").height() > 0.0, "the mask parameters are on the panel, not viewport-only");
        assert!(h.row_menu(0, "Remove mask"), "the row menu drops the mask");
        assert!(h.clip().effects[0].mask.is_none());
        assert_eq!(h.undos, 1);
    }

    /// A mask shapes pixels: an audio clip's stack gets no mask menu rows and no mask grid, even when a
    /// mask reached it (Paste Attributes, a hand-edited project).
    #[test]
    fn audio_clips_get_no_effect_mask_controls() {
        let mut h = Harness::new();
        {
            let c = &mut h.project.tracks[0].clips[0];
            c.kind = ClipKind::Audio;
            let mut fx = Effect::new(EffectKind::Blur);
            fx.mask = Some(Mask::default());
            c.effects.push(fx);
        }
        h.frame(vec![]);
        assert!(test_rects::get("row0").is_some(), "the row itself still renders");
        assert!(test_rects::get("maskgrid0").is_none(), "no mask parameters");
        let at = h.rect("row0").center();
        h.frame(vec![Event::PointerMoved(at)]);
        h.button(at, PointerButton::Secondary, true);
        h.button(at, PointerButton::Secondary, false);
        h.frame(vec![]);
        assert!(h.text_eq("Copy parameters").is_some(), "the row menu is open");
        assert!(h.text_eq("Edit mask").is_none() && h.text_eq("Remove mask").is_none(), "no mask rows");
        assert_eq!(h.last.mask_for, None);
    }

    /// Two numbers per effect: when it starts inside the clip and how long it lasts (0 = to the end).
    #[test]
    fn effect_window_is_editable_with_one_undo() {
        let mut h = Harness::new();
        h.project.tracks[0].clips[0].effects.push(Effect::new(EffectKind::Blur));
        h.frame(vec![]);
        assert!(h.clip().effects[0].on_at(0.0), "covers the whole clip by default");
        let from = h.rect("fxstart0").center();
        h.frame(vec![Event::PointerMoved(from)]);
        h.frame(vec![Event::PointerButton {
            pos: from,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        }]);
        let mut edited = false;
        for i in 1..=4 {
            edited |= h.frame(vec![Event::PointerMoved(from + vec2(10.0 * i as f32, 0.0))]);
        }
        edited |= h.frame(vec![Event::PointerButton {
            pos: from + vec2(40.0, 0.0),
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }]);
        edited |= h.frame(vec![]);
        assert!(edited);
        let fx = &h.clip().effects[0];
        assert!(fx.start > 0.0, "the effect now starts later in the clip: {}", fx.start);
        assert!(!fx.on_at(0.0) && fx.on_at(4.0), "off before its start, still on to the end");
        assert_eq!(h.undos, 1, "one drag gesture = one undo");
    }

    #[test]
    fn node_editor_row_sets_the_flag() {
        let mut h = Harness::new();
        h.project.tracks[0].clips[0].effects.push(Effect::new(EffectKind::Blur));
        h.frame(vec![]);
        h.row_menu(0, "Open in Node editor");
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
        h.row_menu(0, "Copy parameters");
        assert_eq!(h.undos, 0, "copying is not an edit");
        h.frame(vec![]);
        h.row_menu(1, "Paste parameters");
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

    // ---- ws:inspector-gallery ----
    #[test]
    fn stable_effect_key_is_kind_and_same_kind_rank() {
        let fx = vec![Effect::new(EffectKind::Blur), Effect::new(EffectKind::Vignette), Effect::new(EffectKind::Blur)];
        assert_eq!(stable_effect_key(&fx, 0), "Blur#0");
        assert_eq!(stable_effect_key(&fx, 1), "Vignette#0");
        assert_eq!(stable_effect_key(&fx, 2), "Blur#1", "second Blur in the stack ranks #1 among same-kind");
    }

    #[test]
    fn fold_state_keyed_by_kind_survives_a_reorder() {
        // a single Blur's key never depends on its position, so folding it and moving OTHER effects
        // around it must not lose its own fold association.
        let before = vec![Effect::new(EffectKind::Blur), Effect::new(EffectKind::Vignette)];
        let after = vec![Effect::new(EffectKind::Vignette), Effect::new(EffectKind::Blur)];
        assert_eq!(stable_effect_key(&before, 0), stable_effect_key(&after, 1));
    }

    #[test]
    fn stack_reorder_via_local_drag_payload_moves_and_is_one_undo() {
        let mut h = Harness::new();
        h.project.tracks[0].clips[0].effects = vec![Effect::new(EffectKind::Blur), Effect::new(EffectKind::Vignette)];
        h.frame(vec![]); // lay out once so the drop-target row's rect is captured
        let target = h.rect("draghandle1").center();
        // bypass simulating the exact press+move `drag_started` gesture on the source handle (finicky in
        // a synthetic harness) - inject the payload directly, the same state `dnd_set_drag_payload` would
        // have set, then release over the target row.
        egui::DragAndDrop::set_payload(
            &h.ctx,
            EffectDragId(stable_effect_key(&h.project.tracks[0].clips[0].effects, 0)),
        );
        h.frame(vec![
            Event::PointerMoved(target),
            Event::PointerButton {
                pos: target,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            },
        ]);
        let kinds: Vec<EffectKind> = h.clip().effects.iter().map(|e| e.kind).collect();
        assert_eq!(kinds, [EffectKind::Vignette, EffectKind::Blur], "dropping row 0 onto row 1 moves it there");
        assert_eq!(h.undos, 1);
    }

    #[test]
    fn hover_after_150ms_wires_the_hover_stub() {
        let mut h = Harness::new();
        h.project.tracks[0].clips[0].effects = vec![Effect::new(EffectKind::Blur)];
        h.frame(vec![]);
        let over = h.rect("draghandle0").center();
        // first hovered frame: elapsed time is ~0, below the 150ms threshold
        h.frame(vec![Event::PointerMoved(over)]);
        assert_eq!(h.last.hover, None);
        // advance real time past the threshold (Harness::frame bumps `self.time` by 50ms/frame)
        for _ in 0..4 {
            h.frame(vec![Event::PointerMoved(over)]);
        }
        assert_eq!(h.last.hover, Some(0));
    }
}
