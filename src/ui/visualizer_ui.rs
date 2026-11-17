//! ---- ws:audio-visualizer ----
//! The Inspector's "Visualizer" section for a clip carrying a visualizer effect (`engine::visualizer`):
//! pick the style (built-ins + the user's audio-reading library effects), bake the audio under the clip
//! again after moving or trimming it, and the band mixer (which bands drive it and how). Commits
//! straight to the project, with one undo per click or drag; a mix change re-bakes from the song's
//! cached analysis (`visualizer::analysis`), so it shows at once and costs playback nothing.

use crate::engine::visualizer;
use crate::media::Backend;
use crate::model::{AudioMix, Id, Project};
use eframe::egui;

/// The clip's current style name, if it is a visualizer clip.
pub fn style(project: &Project, id: Id) -> Option<String> {
    project.clip(id)?.effects.iter().find_map(visualizer::style_of)
}

/// Draw the section; true when the project changed.
/// `last` is `Settings::visualizer_style`: a pick becomes the style the next visualizer gets.
pub fn section(
    ui: &mut egui::Ui,
    project: &mut Project,
    id: Id,
    backend: Backend,
    last: &mut String,
    undo: &mut dyn FnMut(&Project),
) -> bool {
    let Some(current) = style(project, id) else { return false };
    let err_id = egui::Id::new(("visualizer_err", id));
    let mut pick: Option<visualizer::Style> = None;
    let mut rebake = false;
    let source = visualizer::source_of(project, id);
    egui::Grid::new("inspector_visualizer").num_columns(2).show(ui, |ui| {
        ui.label("Style");
        egui::ComboBox::from_id_salt("visualizer_style").selected_text(&current).show_ui(ui, |ui| {
            // ponytail: lists the library folder each frame the popup is open (a few small files)
            for s in visualizer::styles() {
                let label = if s.custom { format!("{} (custom)", s.name) } else { s.name.clone() };
                if ui.selectable_label(s.name == current, label).clicked() && s.name != current {
                    pick = Some(s);
                }
            }
        });
        ui.end_row();
        ui.label("Audio");
        ui.horizontal(|ui| {
            match source.and_then(|a| project.clip(a)) {
                Some(a) => {
                    ui.label(&a.name);
                }
                None => {
                    ui.weak("no audio under this clip");
                }
            }
            if ui
                .add_enabled(source.is_some(), egui::Button::new("Re-bake").small())
                .on_hover_text("Read the audio under the clip again (after moving or trimming it)")
                .clicked()
            {
                rebake = true;
            }
        });
        ui.end_row();
    });
    // ---- the band mixer
    let mix_now = project
        .clip(id)
        .and_then(|c| c.effects.iter().find(|e| visualizer::style_of(e).is_some()))
        .and_then(|e| e.audio_mix.clone())
        .unwrap_or_default();
    let spectrum = source.and_then(|a| {
        let (c, v) = (project.clip(a)?, project.clip(id)?);
        let path = project.asset(c.asset)?.path.clone();
        let ctx = ui.ctx().clone();
        let song = visualizer::analysis_async(&path, c.audio_stream, backend, move || ctx.request_repaint())?;
        let (s0, s1) = (c.src_time(v.start), c.src_time(v.start + v.duration));
        Some(song.spectrum(s0.min(s1), s0.max(s1)))
    });
    let (mix, mix_started) = mixer(ui, id, &mix_now, spectrum);
    if let Some(e) = ui.ctx().data(|d| d.get_temp::<String>(err_id)) {
        ui.colored_label(ui.visuals().warn_fg_color, e);
    }
    if mix != mix_now {
        if mix_started {
            undo(project);
        }
        if let Some(e) = project
            .clip_mut(id)
            .and_then(|c| c.effects.iter_mut().find(|e| visualizer::style_of(e).is_some()))
        {
            e.audio_mix = (mix != AudioMix::default()).then_some(mix);
        }
        if pick.is_none() && source.is_some() {
            rebake = true;
        }
    }
    let mix_changed = project
        .clip(id)
        .and_then(|c| c.effects.iter().find(|e| visualizer::style_of(e).is_some()))
        .map(|e| e.audio_mix.clone().unwrap_or_default())
        != Some(mix_now);
    let r = match (pick, rebake, source) {
        (Some(s), _, _) => {
            undo(project);
            *last = s.name.clone();
            Some(visualizer::restyle(project, id, &s, backend))
        }
        (None, true, Some(a)) => {
            if !mix_changed {
                undo(project); // the Re-bake button (a mix edit pushed its own undo above)
            }
            Some(visualizer::rebake(project, id, a, backend).map(|_| ()))
        }
        _ => None,
    };
    match r {
        Some(Ok(())) => {
            ui.ctx().data_mut(|d| d.remove::<String>(err_id));
            true
        }
        Some(Err(e)) => {
            ui.ctx().data_mut(|d| d.insert_temp(err_id, e));
            true
        }
        None => mix_changed,
    }
}

/// The band mixer's widgets: a preset, the spectrum graph, per band a gain and a beat-source tick, then
/// sensitivity and smoothing. Returns the edited mix and whether this frame STARTED an interaction
/// (a click, a typed value, a drag's first move) - the caller's one undo per edit.
fn mixer(ui: &mut egui::Ui, id: Id, cur: &AudioMix, spectrum: Option<[f32; 6]>) -> (AudioMix, bool) {
    let mut m = cur.clone();
    let mut started = false;
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label("Listen like");
        let shown = if m.preset.is_empty() { "Custom".to_string() } else { m.preset.clone() };
        egui::ComboBox::from_id_salt(("visualizer_preset", id)).selected_text(shown).show_ui(ui, |ui| {
            for p in AudioMix::PRESETS {
                if ui.selectable_label(m.preset == p, p).clicked() {
                    if let Some(next) = AudioMix::preset(p) {
                        m = next;
                        started = true;
                    }
                }
            }
        });
        if ui.small_button("Reset").on_hover_text("Every band equal, the default smoothing").clicked() {
            m = AudioMix::default();
            started = true;
        }
    });
    // the song's spectrum under the clip (dim) and what the mix makes of it (accent)
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width().min(260.0), 56.0), egui::Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 3.0, ui.visuals().extreme_bg_color);
    let w = rect.width() / 6.0;
    for b in 0..6 {
        let x0 = rect.left() + w * b as f32 + 3.0;
        let bar = |v: f32, c: egui::Color32| {
            let h = (rect.height() - 14.0) * v.clamp(0.0, 1.0);
            p.rect_filled(egui::Rect::from_min_max(egui::pos2(x0, rect.bottom() - 12.0 - h), egui::pos2(x0 + w - 6.0, rect.bottom() - 12.0)), 1.0, c);
        };
        let have = spectrum.map_or(0.0, |s| s[b].sqrt());
        bar(have, ui.visuals().weak_text_color().gamma_multiply(0.5));
        bar(have * m.gains[b] / 3.0_f32.max(1.0) * m.sensitivity.min(3.0), ui.visuals().selection.bg_fill);
        p.text(
            egui::pos2(x0 + (w - 6.0) / 2.0, rect.bottom() - 1.0),
            egui::Align2::CENTER_BOTTOM,
            AudioMix::BANDS[b].0,
            egui::FontId::proportional(9.0),
            ui.visuals().text_color(),
        );
    }
    if spectrum.is_none() {
        p.text(rect.center(), egui::Align2::CENTER_CENTER, "reading the song…", egui::FontId::proportional(11.0), ui.visuals().weak_text_color());
    }
    // a drag owes ONE undo, paid by its first frame that actually changes a value
    let owed_id = egui::Id::new(("visualizer_mix_owed", id));
    let mut owed = ui.ctx().data(|d| d.get_temp::<bool>(owed_id)).unwrap_or(false);
    let mut note = |r: &egui::Response| {
        if r.drag_started() {
            owed = true;
        }
        if r.changed() && (owed || !r.dragged()) {
            started = true;
            owed = false;
        }
    };
    let before = m.clone();
    egui::Grid::new(("visualizer_bands", id)).num_columns(3).show(ui, |ui| {
        ui.weak("Band");
        ui.weak("Intensity");
        ui.weak("Beat").on_hover_text("Which bands the beat detection listens to (none = the whole sound)");
        ui.end_row();
        for (b, (name, lo, hi)) in AudioMix::BANDS.iter().enumerate() {
            ui.label(*name).on_hover_text(format!("{lo:.0} - {hi:.0} Hz"));
            note(&ui.add(egui::Slider::new(&mut m.gains[b], 0.0..=3.0).fixed_decimals(2)));
            note(&ui.checkbox(&mut m.beat_bands[b], ""));
            ui.end_row();
        }
        ui.label("Sensitivity");
        note(&ui.add(egui::Slider::new(&mut m.sensitivity, 0.25..=4.0).logarithmic(true).fixed_decimals(2)));
        ui.end_row();
        ui.label("Attack");
        note(&ui.add(egui::Slider::new(&mut m.attack, 0.0..=0.5).suffix(" s").fixed_decimals(2)));
        ui.end_row();
        ui.label("Release");
        note(&ui.add(egui::Slider::new(&mut m.release, 0.02..=2.0).logarithmic(true).suffix(" s").fixed_decimals(2)));
        ui.end_row();
    });
    ui.ctx().data_mut(|d| d.insert_temp(owed_id, owed));
    if m != before {
        m.preset.clear(); // edited by hand: no longer that preset
    }
    (m.clamped(), started)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draw(p: &mut Project, id: Id) -> (bool, usize, String) {
        let ctx = egui::Context::default();
        let (mut changed, mut undos) = (false, 0);
        let out = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                changed = section(ui, p, id, Backend::Auto, &mut String::new(), &mut |_| undos += 1);
            });
        });
        let text: String = out.shapes.iter().map(|s| format!("{:?}", s.shape)).collect();
        (changed, undos, text)
    }

    /// A visualizer clip gets the Style picker showing its style; a plain shape gets nothing; drawing
    /// alone never edits or pushes an undo.
    #[test]
    fn section_shows_the_style_and_is_idle() {
        let mut p = Project::new();
        let plain = p.add_shape_clip(crate::model::ShapeKind::Rect, 0.0, 1.0);
        let vis = p.add_shape_clip(crate::model::ShapeKind::Rect, 2.0, 1.0);
        p.clip_mut(vis).unwrap().effects.push(crate::model::Effect::from_shader(visualizer::BUILTIN[1].1));
        assert_eq!(style(&p, vis).as_deref(), Some("Spectrum Bars"));
        assert_eq!(style(&p, plain), None);
        let before = p.to_json();
        let (changed, undos, text) = draw(&mut p, vis);
        assert!(!changed && undos == 0 && p.to_json() == before);
        assert!(text.contains("Spectrum Bars") && text.contains("no audio under this clip"), "{text}");
        for w in ["Listen like", "Custom", "Sub", "High-mid", "Air", "Intensity", "Sensitivity", "Release"] {
            assert!(text.contains(w), "the band mixer shows {w}");
        }
        assert!(!draw(&mut p, plain).0);
    }
}
