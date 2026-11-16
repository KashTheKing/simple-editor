//! ---- ws:audio-visualizer ----
//! The Inspector's "Visualizer" section for a clip carrying a visualizer effect (`engine::visualizer`):
//! pick the style (built-ins + the user's audio-reading library effects) and bake the audio under the
//! clip again after moving or trimming it. Commits straight to the project, with one undo per click.

use crate::engine::visualizer;
use crate::media::Backend;
use crate::model::{Id, Project};
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
    if let Some(e) = ui.ctx().data(|d| d.get_temp::<String>(err_id)) {
        ui.colored_label(ui.visuals().warn_fg_color, e);
    }
    let r = match (pick, rebake, source) {
        (Some(s), _, _) => {
            undo(project);
            *last = s.name.clone();
            Some(visualizer::restyle(project, id, &s, backend))
        }
        (None, true, Some(a)) => {
            undo(project);
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
        None => false,
    }
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
        assert!(!draw(&mut p, plain).0);
    }
}
