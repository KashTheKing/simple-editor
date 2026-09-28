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

fn find_value(clip: &Clip, kind: EffectKind, i: usize, default: f64) -> f64 {
    clip.effects.iter().find(|e| e.kind == kind).map(|e| e.params[i].value).unwrap_or(default)
}

fn set_value(clip: &mut Clip, kind: EffectKind, i: usize, v: f64) {
    match clip.effects.iter_mut().find(|e| e.kind == kind) {
        Some(e) => e.params[i].value = v,
        None => {
            let mut e = Effect::new(kind);
            e.params[i].value = v;
            clip.effects.push(e);
        }
    }
}

/// One `Grid` of DragValue rows for `kind`'s params, lazily materialising the effect on first edit.
fn param_grid(ui: &mut egui::Ui, clip: &mut Clip, kind: EffectKind, g: &mut Gesture) {
    Grid::new(("color_params", kind)).num_columns(2).show(ui, |ui| {
        for (i, spec) in kind.params().iter().enumerate() {
            ui.label(spec.name);
            let mut v = find_value(clip, kind, i, spec.default);
            let speed = ((spec.max - spec.min) / 200.0).max(0.001);
            let r = ui.add(DragValue::new(&mut v).range(spec.min..=spec.max).speed(speed));
            g.note(&r); // one undo per gesture, same rule the rest of the inspector uses
            if r.changed() {
                set_value(clip, kind, i, v);
            }
            ui.end_row();
        }
    });
}

/// Draws the Color section body: Primaries/Curves/Levels/HueShift/Vignette + Auto/Match/LUT/Eyedropper.
/// `others`: other selected clips (id, name) offered as the Colour Match reference.
pub fn show(
    ui: &mut egui::Ui,
    clip: &mut Clip,
    others: &[(Id, String)],
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
        let r = glyph_text_button(ui, Glyph::Zoom, "Eyedropper")
            .on_hover_text("Pick a colour from the preview (arms the sample - click a point on the canvas)");
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
        param_grid(ui, clip, kind, g);
    }
    out
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
                out = show(ui, clip, others, &palette, g);
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
        set_value(&mut clip, EffectKind::Vignette, 0, 0.9);
        assert_eq!(clip.effects.len(), 1);
        assert_eq!(clip.effects[0].kind, EffectKind::Vignette);
    }

    #[test]
    fn primaries_added_ahead_of_a_later_curves_touch() {
        let mut clip = Clip::new(1, ClipKind::Video, "c", 0.0, 4.0);
        set_value(&mut clip, EffectKind::Primaries, 0, 0.1);
        set_value(&mut clip, EffectKind::Curves, 0, 0.2);
        let kinds: Vec<EffectKind> = clip.effects.iter().map(|e| e.kind).collect();
        assert_eq!(kinds, [EffectKind::Primaries, EffectKind::Curves], "insertion order, not ORDER's order");
    }

    #[test]
    fn find_value_falls_back_to_the_param_default() {
        let clip = Clip::new(1, ClipKind::Video, "c", 0.0, 4.0);
        let default = EffectKind::Vignette.params()[0].default;
        assert_eq!(find_value(&clip, EffectKind::Vignette, 0, default), default);
    }
}
