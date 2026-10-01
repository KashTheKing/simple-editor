//! ---- ws:inspector-gallery ----
//! Inspector Color section: `Primaries` (lift/gamma/gain/temp/tint - labelled "wheels" in the plan;
//! painted circle+drag wheels are a `// ponytail:` deferral, see below) + the existing
//! `Curves`/`Levels`/`HueShift`/`Vignette` effect kinds, in a fixed order, each added to `clip.effects`
//! lazily on first touch (never eagerly - a clip with no grading keeps an empty stack). Auto Colour /
//! Colour Match / add-LUT all reuse color-engine's own logic rather than redefining it: `clip.add_lut`'s
//! four-line body is inlined here (it needs no GPU stats, and this call site already holds `&mut Clip`
//! plus the same per-gesture `undo` this whole inspector uses - routing a plain local mutation through
//! `App::run_tool_undoable` would need a much larger plumbing detour for zero behavioural difference),
//! while `color.auto`/`color.match` genuinely need a live GPU-rendered frame (`FrameStats`), which only
//! `App` can produce - those two are armed via a thread-local hand-off (`inspector::take_pending_color_*`,
//! same idiom as `PENDING_FONT`) that `App::poll_panels` drains into `App::run_tool_undoable("color.auto"
//! | "color.match", …)`.

use crate::model::{Clip, Effect, EffectKind, Id};
use crate::theme::Palette;
use crate::ui::tools::{glyph_label, glyph_text_button, Glyph};
use crate::ui::Gesture;
use eframe::egui::{self, DragValue, Grid};

/// Fixed display order - touching one never creates the others.
const ORDER: [EffectKind; 5] =
    [EffectKind::Primaries, EffectKind::Curves, EffectKind::Levels, EffectKind::HueShift, EffectKind::Vignette];

#[derive(Default)]
pub struct ColorResponse {
    /// The user clicked "Eyedropper" for this clip - arms a pending sample request. ponytail: nothing
    /// consumes it into an actual pixel read yet (that needs a canvas click handler in `preview.rs`,
    /// owned by canvas-handles-monitor, not this workstream); `App::poll_panels` still drains it and
    /// toasts an honest "not wired yet" instead of silently swallowing the click.
    pub eyedrop: bool,
    pub auto: bool,
    /// The user picked a clip from the "Match to" combo and clicked "Match" - its id.
    pub match_ref: Option<Id>,
}

fn find_value(clip: &Clip, kind: EffectKind, i: usize, default: f64, lt: f64) -> f64 {
    clip.effects.iter().find(|e| e.kind == kind).map(|e| e.params[i].at(lt)).unwrap_or(default)
}

/// Find-or-create `kind` (appended, so touching one never creates the others).
fn effect_mut(clip: &mut Clip, kind: EffectKind) -> &mut Effect {
    match clip.effects.iter().position(|e| e.kind == kind) {
        Some(i) => &mut clip.effects[i],
        None => {
            clip.effects.push(Effect::new(kind));
            clip.effects.last_mut().unwrap()
        }
    }
}

/// `set_at`, not `.value =`: once a param is keyframed its constant is ignored, so a plain write made
/// every edit after the first ◆ silently do nothing.
fn set_value(clip: &mut Clip, kind: EffectKind, i: usize, v: f64, lt: f64) {
    effect_mut(clip, kind).params[i].set_at(lt, v);
}

/// One `Grid` of DragValue + ◆ rows for `kind`'s params, lazily materialising the effect on first edit.
fn param_grid(ui: &mut egui::Ui, clip: &mut Clip, kind: EffectKind, lt: f64, palette: &Palette, g: &mut Gesture) {
    Grid::new(("color_params", kind)).num_columns(3).show(ui, |ui| {
        for (i, spec) in kind.params().iter().enumerate() {
            ui.label(spec.name);
            let mut v = find_value(clip, kind, i, spec.default, lt);
            let speed = ((spec.max - spec.min) / 200.0).max(0.001);
            let r = ui.add(DragValue::new(&mut v).range(spec.min..=spec.max).speed(speed));
            g.note(&r); // one undo per gesture, same rule the rest of the inspector uses
            if r.changed() {
                set_value(clip, kind, i, v, lt);
            }
            match clip.effects.iter_mut().find(|e| e.kind == kind) {
                Some(e) => {
                    crate::ui::key_buttons(ui, &mut e.params[i], lt, palette, g, spec.name, &[]);
                }
                None => {
                    // nothing to key yet: a click materialises the effect with that key
                    let mut a = crate::model::Animated::new(spec.default);
                    let mut kg = Gesture::default();
                    crate::ui::key_buttons(ui, &mut a, lt, palette, &mut kg, spec.name, &[]);
                    if kg.changed {
                        g.click();
                        effect_mut(clip, kind).params[i] = a;
                    }
                }
            }
            ui.end_row();
        }
    });
}

/// The four Resolve wheels over `Primaries`: (name, first param index, neutral value, puck scale).
/// Lift is added before gamma, Offset after it (see the PRIMARIES shader).
pub const WHEELS: [(&str, usize, f64, f64); 4] =
    [("Lift", 0, 0.0, 0.25), ("Gamma", 3, 1.0, 0.5), ("Gain", 6, 1.0, 0.5), ("Offset", 11, 0.0, 0.25)];

/// RGB deltas (zero mean) <-> a point on the wheel: red at 0°, green at 120°, blue at 240°.
fn rgb_to_xy(d: [f64; 3]) -> (f64, f64) {
    ((2.0 * d[0] - d[1] - d[2]) / 3.0, (d[1] - d[2]) / 3f64.sqrt())
}
fn xy_to_rgb(x: f64, y: f64) -> [f64; 3] {
    let k = 3f64.sqrt() / 2.0;
    [x, -0.5 * x + k * y, -0.5 * x - k * y]
}

/// Toggle a key at `lt` on params `idx` together: removes them all when any has one here, else adds.
fn toggle_keys(clip: &mut Clip, kind: EffectKind, idx: std::ops::Range<usize>, lt: f64) {
    let e = effect_mut(clip, kind);
    let any = idx.clone().any(|i| e.params[i].has_key_at(lt));
    for i in idx {
        if e.params[i].has_key_at(lt) == any {
            e.params[i].toggle_key(lt);
        }
    }
}

fn diamond(ui: &egui::Ui, rect: egui::Rect, filled: bool, bright: bool, palette: &Palette) {
    let color = if bright { palette.accent } else { palette.text_dim };
    let (c, h) = (rect.center(), 5.0);
    let pts = vec![c - egui::vec2(0.0, h), c + egui::vec2(h, 0.0), c + egui::vec2(0.0, h), c - egui::vec2(h, 0.0)];
    let fill = if filled { color } else { egui::Color32::TRANSPARENT };
    ui.painter().add(egui::Shape::convex_polygon(pts, fill, egui::Stroke::new(1.3, color)));
}

/// A wheel's title row: its name and a ◆ that keys (or un-keys) all three channels at once.
fn wheel_title(ui: &mut egui::Ui, clip: &mut Clip, w: usize, lt: f64, palette: &Palette, g: &mut Gesture) {
    let (name, i0, _, _) = WHEELS[w];
    ui.horizontal(|ui| {
        ui.strong(name);
        let e = clip.effects.iter().find(|e| e.kind == EffectKind::Primaries);
        let here = e.is_some_and(|e| (i0..i0 + 3).any(|i| e.params[i].has_key_at(lt)));
        let animated = e.is_some_and(|e| (i0..i0 + 3).any(|i| e.params[i].is_animated()));
        let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
        let r = ui.interact(rect, ui.id().with(("wheel_kf", w)), egui::Sense::click());
        diamond(ui, rect, here, animated || r.hovered(), palette);
        if r.on_hover_text(format!("Keyframe {name} (all three channels) at the playhead")).clicked() {
            toggle_keys(clip, EffectKind::Primaries, i0..i0 + 3, lt);
            g.click();
        }
    });
}

/// One colour wheel: the puck is the channel balance (drag it; double-click resets the wheel), the
/// number under it the master (the channels' mean).
fn wheel(ui: &mut egui::Ui, clip: &mut Clip, w: usize, lt: f64, size: f32, palette: &Palette, g: &mut Gesture) {
    let (name, i0, neutral, scale) = WHEELS[w];
    let spec = &EffectKind::Primaries.params()[i0];
    let rgb: [f64; 3] = std::array::from_fn(|c| find_value(clip, EffectKind::Primaries, i0 + c, spec.default, lt));
    let master = (rgb[0] + rgb[1] + rgb[2]) / 3.0;
    let (x, y) = rgb_to_xy([rgb[0] - master, rgb[1] - master, rgb[2] - master]);
    let set = |clip: &mut Clip, m: f64, x: f64, y: f64| {
        let d = xy_to_rgb(x, y);
        for (c, d) in d.iter().enumerate() {
            set_value(clip, EffectKind::Primaries, i0 + c, (m + d).clamp(spec.min, spec.max), lt);
        }
    };
    ui.vertical(|ui| {
        wheel_title(ui, clip, w, lt, palette, g);
        let (rect, r) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::click_and_drag());
        let c = rect.center();
        let rad = size * 0.5 - 2.0;
        let p = ui.painter();
        let tau = std::f32::consts::TAU;
        for k in 0..36 {
            // a hue ring, red at 0° like the maths above
            let pt = |a: f32| c + egui::vec2(a.cos(), -a.sin()) * rad;
            let col = egui::ecolor::Hsva::new(k as f32 / 36.0, 0.8, 0.9, 1.0);
            p.line_segment([pt(k as f32 / 36.0 * tau), pt((k + 1) as f32 / 36.0 * tau)], egui::Stroke::new(3.0, col));
        }
        p.circle_filled(c, rad - 2.0, palette.bg);
        p.line_segment([c - egui::vec2(rad * 0.3, 0.0), c + egui::vec2(rad * 0.3, 0.0)], (1.0, palette.text_dim));
        p.line_segment([c - egui::vec2(0.0, rad * 0.3), c + egui::vec2(0.0, rad * 0.3)], (1.0, palette.text_dim));
        let puck = c + egui::vec2((x / scale) as f32, -(y / scale) as f32) * rad;
        p.circle_stroke(puck, 5.0, egui::Stroke::new(2.0, palette.text));
        if r.double_clicked() {
            set(clip, neutral, 0.0, 0.0);
            g.click();
        } else if let Some(pos) = r.interact_pointer_pos().filter(|_| r.dragged() || r.clicked()) {
            let mut v = (pos - c) / rad;
            if v.length() > 1.0 {
                v = v.normalized();
            }
            set(clip, master, v.x as f64 * scale, -v.y as f64 * scale);
            g.start |= r.drag_started() || r.clicked();
            g.changed = true;
        }
        r.on_hover_text(format!("{name}: drag to balance the colour, double-click to reset"));
        let mut m = master;
        let r = ui.add_sized([size, 18.0], DragValue::new(&mut m).range(spec.min..=spec.max).speed(0.005));
        g.note(&r);
        if r.changed() {
            set(clip, m, x, y);
        }
    });
}

/// Primaries bars: the same wheel's three channels as vertical sliders.
fn bar_group(ui: &mut egui::Ui, clip: &mut Clip, w: usize, lt: f64, size: f32, palette: &Palette, g: &mut Gesture) {
    let i0 = WHEELS[w].1;
    ui.vertical(|ui| {
        ui.set_width(size);
        wheel_title(ui, clip, w, lt, palette, g);
        ui.horizontal(|ui| {
            for c in 0..3 {
                let spec = &EffectKind::Primaries.params()[i0 + c];
                let mut v = find_value(clip, EffectKind::Primaries, i0 + c, spec.default, lt);
                let r = ui.add(egui::Slider::new(&mut v, spec.min..=spec.max).vertical().show_value(false));
                g.note(&r.on_hover_text(spec.name));
                if v != find_value(clip, EffectKind::Primaries, i0 + c, spec.default, lt) {
                    set_value(clip, EffectKind::Primaries, i0 + c, v, lt);
                }
            }
        });
    });
}

/// "Keyframe grade": a key at `lt` on every param of every grade effect the clip has (Primaries is
/// created first so a fresh clip still gets one). Params already keyed here are left alone.
pub fn key_grade(clip: &mut Clip, lt: f64) {
    effect_mut(clip, EffectKind::Primaries);
    for e in clip.effects.iter_mut().filter(|e| ORDER.contains(&e.kind)) {
        for a in &mut e.params {
            if !a.has_key_at(lt) {
                a.toggle_key(lt);
            }
        }
    }
}

/// The Color page's Grade pane: Lift/Gamma/Gain/Offset wheels (or the same values as Primaries bars),
/// Temp/Tint, and the whole-grade keyframe button. Edits `clip` in place; `g` says when to commit.
pub fn grade_panel(ui: &mut egui::Ui, clip: &mut Clip, lt: f64, palette: &Palette, g: &mut Gesture) {
    let bars_id = egui::Id::new("grade_bars");
    let mut bars: bool = ui.ctx().data(|d| d.get_temp(bars_id).unwrap_or(false));
    ui.horizontal(|ui| {
        ui.selectable_value(&mut bars, false, "Wheels");
        ui.selectable_value(&mut bars, true, "Bars");
        ui.separator();
        let r = glyph_text_button(ui, Glyph::Wheel, "Keyframe grade")
            .on_hover_text("Add a keyframe to every grade parameter at the playhead");
        if r.clicked() {
            key_grade(clip, lt);
            g.click();
        }
        for (i, name) in [(9usize, "Temp"), (10, "Tint")] {
            ui.label(name);
            let mut v = find_value(clip, EffectKind::Primaries, i, 0.0, lt);
            let r = ui.add(DragValue::new(&mut v).range(-100.0..=100.0).speed(0.5));
            g.note(&r);
            if r.changed() {
                set_value(clip, EffectKind::Primaries, i, v, lt);
            }
        }
    });
    ui.ctx().data_mut(|d| d.insert_temp(bars_id, bars));
    let size = ((ui.available_width() - 40.0) / 4.0).min(ui.available_height() - 50.0).clamp(60.0, 170.0);
    ui.horizontal_top(|ui| {
        for w in 0..4 {
            if bars {
                bar_group(ui, clip, w, lt, size, palette, g);
            } else {
                wheel(ui, clip, w, lt, size, palette, g);
            }
            ui.add_space(6.0);
        }
    });
}

/// Draws the Color section body: Primaries/Curves/Levels/HueShift/Vignette + Auto/Match/LUT/Eyedropper.
/// `others`: other selected clips (id, name) offered as the Colour Match reference.
pub fn show(
    ui: &mut egui::Ui,
    clip: &mut Clip,
    others: &[(Id, String)],
    lt: f64,
    palette: &Palette,
    g: &mut Gesture,
) -> ColorResponse {
    let mut out = ColorResponse::default();
    ui.horizontal(|ui| {
        if ui.small_button("Auto Colour").on_hover_text("Sample the frame at the playhead").clicked() {
            out.auto = true;
        }
        if ui.small_button("Add LUT…").on_hover_text("Load a .cube 3D LUT").clicked() {
            if let Some(path) = rfd::FileDialog::new().add_filter("3D LUT", &["cube"]).pick_file() {
                if crate::engine::lut::load(&path.to_string_lossy()).is_ok() {
                    g.click();
                    let mut e = Effect::new(EffectKind::Lut);
                    e.lut = path.to_string_lossy().to_string();
                    clip.effects.push(e);
                }
            }
        }
        let r = glyph_text_button(ui, Glyph::Eyedropper, "")
            .on_hover_text("Eyedropper: pick a colour from the preview (click a point on the canvas)");
        if r.clicked() {
            out.eyedrop = true;
        }
    });
    if !others.is_empty() {
        ui.horizontal(|ui| {
            let sel_id = ui.id().with("color_match_ref");
            let mut chosen: Option<Id> = ui.ctx().data(|d| d.get_temp(sel_id));
            let text = chosen.and_then(|c| others.iter().find(|(id, _)| *id == c)).map(|(_, n)| n.clone());
            egui::ComboBox::from_id_salt("color_match_combo")
                .selected_text(text.unwrap_or_else(|| "Match to…".into()))
                .show_ui(ui, |ui| {
                    for (id, name) in others {
                        if ui.selectable_label(chosen == Some(*id), name).clicked() {
                            chosen = Some(*id);
                        }
                    }
                });
            ui.ctx().data_mut(|d| d.insert_temp(sel_id, chosen));
            if ui.add_enabled(chosen.is_some(), egui::Button::new("Match")).clicked() {
                out.match_ref = chosen;
            }
        });
    }
    ui.separator();
    for kind in ORDER {
        ui.horizontal(|ui| {
            if kind == EffectKind::Primaries {
                glyph_label(ui, Glyph::Wheel, palette.text);
            }
            ui.strong(kind.name());
        });
        param_grid(ui, clip, kind, lt, palette, g);
    }
    out
}

/// The `Curves` effect's channels as the Color page's custom curves: (tab, first param, line colour).
/// Each has three movable points at 1/4, 2/4, 3/4 input; 0 and 1 stay pinned.
const CURVE_CHANNELS: [(&str, usize, [u8; 3]); 4] =
    [("Luma", 0, [220, 220, 220]), ("R", 3, [235, 80, 80]), ("G", 6, [90, 210, 110]), ("B", 9, [90, 140, 240])];

/// The output value a drag to `y` (0 = top of the graph, 1 = bottom) sets.
fn curve_y(y: f32) -> f64 {
    (1.0 - y as f64).clamp(0.0, 1.0)
}

/// Luma/R/G/B custom curves over the clip's `Curves` effect (created on first drag). Drag a point up or
/// down; double-click the graph to reset the channel; ◆ keys the channel's three points.
pub fn color_curves(ui: &mut egui::Ui, clip: &mut Clip, lt: f64, palette: &Palette, g: &mut Gesture) {
    let ch_id = egui::Id::new("color_curves_ch");
    let mut ch: usize = ui.ctx().data(|d| d.get_temp(ch_id).unwrap_or(0));
    let i0 = CURVE_CHANNELS[ch].1;
    ui.horizontal(|ui| {
        for (i, (name, ..)) in CURVE_CHANNELS.iter().enumerate() {
            ui.selectable_value(&mut ch, i, *name);
        }
        let e = clip.effects.iter().find(|e| e.kind == EffectKind::Curves);
        let here = e.is_some_and(|e| (i0..i0 + 3).any(|i| e.params[i].has_key_at(lt)));
        let animated = e.is_some_and(|e| (i0..i0 + 3).any(|i| e.params[i].is_animated()));
        let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
        let r = ui.interact(rect, ui.id().with("curve_kf"), egui::Sense::click());
        diamond(ui, rect, here, animated || r.hovered(), palette);
        if r.on_hover_text("Keyframe this curve at the playhead").clicked() {
            toggle_keys(clip, EffectKind::Curves, i0..i0 + 3, lt);
            g.click();
        }
    });
    ui.ctx().data_mut(|d| d.insert_temp(ch_id, ch));
    let (_, i0, rgb) = CURVE_CHANNELS[ch];
    let side = ui.available_width().min(ui.available_height()).max(60.0);
    let (rect, r) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::click_and_drag());
    let col = egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
    let defaults = [0.25, 0.5, 0.75];
    let ys: [f64; 3] = std::array::from_fn(|k| find_value(clip, EffectKind::Curves, i0 + k, defaults[k], lt));
    let at = |x: f64, y: f64| rect.left_bottom() + egui::vec2(x as f32 * rect.width(), -(y as f32) * rect.height());
    let p = ui.painter();
    p.rect_filled(rect, 0.0, palette.bg);
    for k in 1..4 {
        let f = k as f32 / 4.0;
        let x = rect.left() + f * rect.width();
        let y = rect.top() + f * rect.height();
        p.line_segment([egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())], (1.0, palette.border));
        p.line_segment([egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)], (1.0, palette.border));
    }
    let pts = [at(0.0, 0.0), at(0.25, ys[0]), at(0.5, ys[1]), at(0.75, ys[2]), at(1.0, 1.0)];
    p.add(egui::Shape::line(pts.to_vec(), egui::Stroke::new(2.0, col)));
    for q in &pts[1..4] {
        p.circle_filled(*q, 4.0, col);
    }
    if r.double_clicked() {
        for k in 0..3 {
            set_value(clip, EffectKind::Curves, i0 + k, defaults[k], lt);
        }
        g.click();
    } else if let Some(pos) = r.interact_pointer_pos().filter(|_| r.dragged() || r.drag_started()) {
        // the drag moves the point whose column it started nearest to
        let k_id = ui.id().with("curve_drag_k");
        if r.drag_started() {
            let fx = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
            let k = ((fx * 4.0 - 1.0).round() as i32).clamp(0, 2) as usize;
            ui.ctx().data_mut(|d| d.insert_temp(k_id, k));
            g.start = true;
        }
        let k: usize = ui.ctx().data(|d| d.get_temp(k_id).unwrap_or(1));
        set_value(clip, EffectKind::Curves, i0 + k, curve_y((pos.y - rect.top()) / rect.height()), lt);
        g.changed = true;
    }
    r.on_hover_text("Drag a point up/down; double-click to reset this curve");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ClipKind;

    fn ctx() -> egui::Context {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        ctx
    }

    fn run(ctx: &egui::Context, clip: &mut Clip, others: &[(Id, String)], g: &mut Gesture) -> ColorResponse {
        let palette = Palette::new(true, egui::Color32::WHITE);
        let mut out = ColorResponse::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                out = show(ui, clip, others, 0.0, &palette, g);
            });
        });
        out
    }

    #[test]
    fn touching_only_vignette_does_not_create_others() {
        let ctx = ctx();
        let mut clip = Clip::new(1, ClipKind::Video, "c", 0.0, 4.0);
        let mut g = Gesture::default();
        run(&ctx, &mut clip, &[], &mut g);
        assert!(clip.effects.is_empty(), "just showing the section adds nothing");
        // simulate a direct edit the way param_grid would: touch Vignette only
        set_value(&mut clip, EffectKind::Vignette, 0, 0.9, 0.0);
        assert_eq!(clip.effects.len(), 1);
        assert_eq!(clip.effects[0].kind, EffectKind::Vignette);
    }

    #[test]
    fn primaries_added_ahead_of_a_later_curves_touch() {
        let mut clip = Clip::new(1, ClipKind::Video, "c", 0.0, 4.0);
        set_value(&mut clip, EffectKind::Primaries, 0, 0.1, 0.0);
        set_value(&mut clip, EffectKind::Curves, 0, 0.2, 0.0);
        let kinds: Vec<EffectKind> = clip.effects.iter().map(|e| e.kind).collect();
        assert_eq!(kinds, [EffectKind::Primaries, EffectKind::Curves], "insertion order, not ORDER's order");
    }

    #[test]
    fn find_value_falls_back_to_the_param_default() {
        let clip = Clip::new(1, ClipKind::Video, "c", 0.0, 4.0);
        let default = EffectKind::Vignette.params()[0].default;
        assert_eq!(find_value(&clip, EffectKind::Vignette, 0, default, 0.0), default);
    }

    #[test]
    fn edits_after_a_keyframe_still_take() {
        let mut clip = Clip::new(1, ClipKind::Video, "c", 0.0, 4.0);
        key_grade(&mut clip, 1.0);
        set_value(&mut clip, EffectKind::Primaries, 6, 1.5, 2.0);
        assert_eq!(find_value(&clip, EffectKind::Primaries, 6, 1.0, 2.0), 1.5, "a keyed param must still edit");
        assert_eq!(find_value(&clip, EffectKind::Primaries, 6, 1.0, 1.0), 1.0, "the earlier key holds");
    }

    #[test]
    fn wheel_maths_round_trips_and_keys_whole_wheels() {
        let (x, y) = rgb_to_xy([0.2, -0.05, -0.15]);
        for (a, b) in xy_to_rgb(x, y).iter().zip([0.2, -0.05, -0.15]) {
            assert!((a - b).abs() < 1e-9);
        }
        let mut clip = Clip::new(1, ClipKind::Video, "c", 0.0, 4.0);
        toggle_keys(&mut clip, EffectKind::Primaries, 6..9, 0.5);
        let p = &clip.effects[0].params;
        assert!((6..9).all(|i| p[i].has_key_at(0.5)) && !p[0].is_animated());
        toggle_keys(&mut clip, EffectKind::Primaries, 6..9, 0.5);
        assert!((6..9).all(|i| !clip.effects[0].params[i].is_animated()), "a second click removes them");
    }

    #[test]
    fn assert_no_idle_repaint_grade_panel() {
        let ctx = ctx();
        let palette = Palette::new(true, egui::Color32::WHITE);
        let mut clip = Clip::new(1, ClipKind::Video, "c", 0.0, 4.0);
        for bars in [false, true] {
            ctx.data_mut(|d| d.insert_temp(egui::Id::new("grade_bars"), bars));
            for _ in 0..30 {
                let _ = ctx.run(egui::RawInput::default(), |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        grade_panel(ui, &mut clip, 0.0, &palette, &mut Gesture::default());
                    });
                });
            }
            assert!(!ctx.has_requested_repaint(), "the Grade pane must not repaint while idle");
        }
        assert!(clip.effects.is_empty(), "drawing the wheels adds nothing");
    }

    #[test]
    fn curve_drag_writes_the_curves_effect_at_the_playhead() {
        let mut clip = Clip::new(1, ClipKind::Video, "c", 0.0, 4.0);
        // the R channel's middle point, dragged a quarter of the way down from the top
        set_value(&mut clip, EffectKind::Curves, CURVE_CHANNELS[1].1 + 1, curve_y(0.25), 0.0);
        assert_eq!(clip.effects[0].kind, EffectKind::Curves);
        assert_eq!(clip.effects[0].params[4].value, 0.75, "R 2/4 lifted to 0.75");
        assert_eq!(clip.effects[0].params[1].value, 0.5, "Luma untouched");
        let ctx = ctx();
        let palette = Palette::new(true, egui::Color32::WHITE);
        let mut fresh = Clip::new(2, ClipKind::Video, "d", 0.0, 4.0);
        for _ in 0..30 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    color_curves(ui, &mut fresh, 0.0, &palette, &mut Gesture::default());
                });
            });
        }
        assert!(!ctx.has_requested_repaint(), "assert_no_idle_repaint: colour curves");
        assert!(fresh.effects.is_empty(), "drawing the curves adds nothing");
    }
}
