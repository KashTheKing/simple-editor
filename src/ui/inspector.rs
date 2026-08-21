//! Inspector panel. Nothing selected → Project settings (name, width, height, fps). One clip selected →
//! clip name, enabled, timing (start / duration / source in, read-only or DragValue), and:
//!  * visual clips: Position X/Y, Scale, Rotation, Opacity — each row = DragValue + "◆" keyframe toggle
//!    (`Animated::toggle_key(clip.local(playhead))`, highlighted when a key exists at the playhead) +
//!    "clear keys"; edits go through `Animated::set_at(local_t, v)` so keyframed props get keys; Blend mode combo.
//!  * audio clips: Volume (dB slider mapped to linear gain) with the same keyframe controls.
//!  * text clips: multiline text, font family combo (`fonts`), size, bold/italic, colour pickers (fill,
//!    outline, shadow, box), outline width, shadow on/off + x/y/blur, alignment, line/letter spacing, box padding.
//! Call `undo(project)` once per gesture (on `drag_started()` / first `changed()` of a widget) before mutating.
//! Returns true if the project changed.

use crate::model::{Animated, BlendMode, ClipKind, Id, Project};
use crate::theme::Palette;
use crate::ui::timecode;
use eframe::egui::{self, Button, DragValue, Grid, Response, Slider};

/// True on the first frame of an edit gesture (drag start, or a non-drag change such as typing/clicking).
fn edit_start(r: &Response) -> bool {
    r.drag_started() || (r.changed() && !r.dragged())
}

/// Accumulates widget responses over one frame: `start` → push undo, `changed` → write back.
#[derive(Default)]
struct Gesture {
    start: bool,
    changed: bool,
}

impl Gesture {
    fn note(&mut self, r: &Response) {
        self.start |= edit_start(r);
        self.changed |= r.changed();
    }
    fn click(&mut self) {
        self.start = true;
        self.changed = true;
    }
}

fn gain_to_db(g: f64) -> f64 {
    if g <= 0.0 {
        -60.0
    } else {
        (20.0 * g.log10()).max(-60.0)
    }
}

fn db_to_gain(db: f64) -> f64 {
    if db <= -60.0 {
        0.0
    } else {
        10f64.powf(db / 20.0)
    }
}

pub fn show(
    ui: &mut egui::Ui,
    project: &mut Project,
    selection: &[Id],
    playhead: f64,
    fonts: &[String],
    palette: &Palette,
    undo: &mut dyn FnMut(&Project),
) -> bool {
    match selection.iter().find(|&&id| project.clip(id).is_some()) {
        None => project_section(ui, project, undo),
        Some(&id) => clip_section(ui, project, id, selection.len(), playhead, fonts, palette, undo),
    }
}

fn project_section(ui: &mut egui::Ui, project: &mut Project, undo: &mut dyn FnMut(&Project)) -> bool {
    let mut edited = false;
    ui.strong("Project");
    Grid::new("inspector_project").num_columns(2).show(ui, |ui| {
        ui.label("Name");
        let mut name = project.name.clone(); // ponytail: per-frame clone so undo can snapshot before the write
        let r = ui.text_edit_singleline(&mut name);
        if r.changed() {
            if edit_start(&r) {
                undo(project);
            }
            project.name = name;
            edited = true;
        }
        ui.end_row();
        let mut num = |ui: &mut egui::Ui,
                       project: &mut Project,
                       label: &str,
                       get: fn(&Project) -> f64,
                       set: fn(&mut Project, f64),
                       range: std::ops::RangeInclusive<f64>,
                       speed: f64| {
            ui.label(label);
            let mut v = get(project);
            let r = ui.add(DragValue::new(&mut v).range(range).speed(speed));
            if edit_start(&r) {
                undo(project);
            }
            if r.changed() {
                set(project, v);
                edited = true;
            }
            ui.end_row();
        };
        num(ui, project, "Width", |p| p.width as f64, |p, v| p.width = v as u32, 16.0..=8192.0, 1.0);
        num(ui, project, "Height", |p| p.height as f64, |p, v| p.height = v as u32, 16.0..=8192.0, 1.0);
        num(ui, project, "FPS", |p| p.fps, |p, v| p.fps = v, 1.0..=240.0, 0.1);
    });
    edited
}

#[allow(clippy::too_many_arguments)]
fn clip_section(
    ui: &mut egui::Ui,
    project: &mut Project,
    id: Id,
    n_selected: usize,
    playhead: f64,
    fonts: &[String],
    palette: &Palette,
    undo: &mut dyn FnMut(&Project),
) -> bool {
    // ponytail: edit a per-frame clone of the clip and write it back at the end — lets `undo` snapshot the
    // untouched project first without borrow gymnastics. Upgrade: per-field scratch copies if it ever shows.
    let Some(orig) = project.clip(id) else {
        return false;
    };
    let mut clip = orig.clone();
    let track = project.track_of(id).map(|t| project.tracks[t].name.clone()).unwrap_or_default();
    let fps = project.fps;
    let lt = clip.local(playhead);
    let mut g = Gesture::default();

    if n_selected > 1 {
        ui.label(format!("{n_selected} clips selected"));
    }
    ui.strong("Clip");
    Grid::new("inspector_clip").num_columns(2).show(ui, |ui| {
        ui.label("Name");
        g.note(&ui.text_edit_singleline(&mut clip.name));
        ui.end_row();
        ui.label("Enabled");
        g.note(&ui.checkbox(&mut clip.enabled, ""));
        ui.end_row();
        ui.label("Track");
        ui.label(&track);
        ui.end_row();
        ui.label("Start");
        ui.monospace(timecode(clip.start, fps));
        ui.end_row();
        ui.label("Duration");
        ui.monospace(timecode(clip.duration, fps));
        ui.end_row();
        if clip.uses_asset() {
            ui.label("Source in");
            ui.monospace(timecode(clip.src_in, fps));
            ui.end_row();
        }
        for (label, a) in clip.props_mut() {
            ui.label(label);
            ui.horizontal(|ui| {
                let mut v = a.at(lt);
                let r = match label {
                    "Volume" => {
                        let mut db = gain_to_db(v);
                        let r = ui.add(Slider::new(&mut db, -60.0..=12.0).suffix(" dB").fixed_decimals(1));
                        if r.changed() {
                            v = db_to_gain(db);
                        }
                        r
                    }
                    "Scale" => ui.add(DragValue::new(&mut v).speed(0.01).range(0.01..=20.0)),
                    "Opacity" => ui.add(DragValue::new(&mut v).speed(0.01).range(0.0..=1.0)),
                    _ => ui.add(DragValue::new(&mut v).speed(1.0)),
                };
                if r.changed() {
                    a.set_at(lt, v);
                }
                g.note(&r);
                key_buttons(ui, a, lt, palette, &mut g);
            });
            ui.end_row();
        }
        if clip.is_visual() {
            ui.label("Blend");
            egui::ComboBox::from_id_salt("blend").selected_text(clip.blend.name()).show_ui(ui, |ui| {
                for b in BlendMode::ALL {
                    g.note(&ui.selectable_value(&mut clip.blend, b, b.name()));
                }
            });
            ui.end_row();
        }
    });

    if clip.kind == ClipKind::Text {
        let style = clip.text.get_or_insert_with(Default::default);
        ui.separator();
        ui.strong("Text");
        g.note(&ui.text_edit_multiline(&mut style.text));
        Grid::new("inspector_text").num_columns(2).show(ui, |ui| {
            ui.label("Font");
            egui::ComboBox::from_id_salt("font").selected_text(style.font.clone()).show_ui(ui, |ui| {
                if !fonts.iter().any(|f| *f == style.font) {
                    let _ = ui.selectable_label(true, style.font.as_str());
                }
                for f in fonts {
                    g.note(&ui.selectable_value(&mut style.font, f.clone(), f));
                }
            });
            ui.end_row();
            ui.label("Size");
            ui.horizontal(|ui| {
                g.note(&ui.add(DragValue::new(&mut style.size).range(1.0..=1000.0)));
                g.note(&ui.checkbox(&mut style.bold, "Bold"));
                g.note(&ui.checkbox(&mut style.italic, "Italic"));
            });
            ui.end_row();
            ui.label("Fill");
            g.note(&ui.color_edit_button_srgba_unmultiplied(&mut style.color));
            ui.end_row();
            ui.label("Outline");
            ui.horizontal(|ui| {
                g.note(&ui.color_edit_button_srgba_unmultiplied(&mut style.outline_color));
                g.note(&ui.add(DragValue::new(&mut style.outline_width).range(0.0..=50.0).speed(0.1)));
            });
            ui.end_row();
            ui.label("Shadow");
            ui.horizontal(|ui| {
                g.note(&ui.checkbox(&mut style.shadow, ""));
                g.note(&ui.color_edit_button_srgba_unmultiplied(&mut style.shadow_color));
            });
            ui.end_row();
            ui.label("Shadow x/y/blur");
            ui.horizontal(|ui| {
                g.note(&ui.add(DragValue::new(&mut style.shadow_x).range(-200.0..=200.0)));
                g.note(&ui.add(DragValue::new(&mut style.shadow_y).range(-200.0..=200.0)));
                g.note(&ui.add(DragValue::new(&mut style.shadow_blur).range(0.0..=50.0).speed(0.1)));
            });
            ui.end_row();
            ui.label("Align");
            ui.horizontal(|ui| {
                for (i, name) in ["Left", "Center", "Right"].iter().enumerate() {
                    g.note(&ui.selectable_value(&mut style.align, i as u8, *name));
                }
            });
            ui.end_row();
            ui.label("Line spacing");
            g.note(&ui.add(DragValue::new(&mut style.line_spacing).range(0.5..=3.0).speed(0.01)));
            ui.end_row();
            ui.label("Letter spacing");
            g.note(&ui.add(DragValue::new(&mut style.letter_spacing).range(-10.0..=50.0).speed(0.1)));
            ui.end_row();
            ui.label("Box");
            ui.horizontal(|ui| {
                g.note(&ui.color_edit_button_srgba_unmultiplied(&mut style.box_color));
                g.note(&ui.add(DragValue::new(&mut style.box_padding).range(0.0..=100.0).speed(0.5)));
            });
            ui.end_row();
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
    g.changed
}

fn key_buttons(ui: &mut egui::Ui, a: &mut Animated, lt: f64, palette: &Palette, g: &mut Gesture) {
    let mut b = Button::new("◆").small();
    if a.has_key_at(lt) {
        b = b.fill(palette.keyframe);
    }
    if ui.add(b).on_hover_text("Toggle keyframe at playhead").clicked() {
        a.toggle_key(lt);
        g.click();
    }
    if a.is_animated() {
        if ui.small_button("✕ keys").clicked() {
            a.clear_keys(lt);
            g.click();
        }
        ui.label(format!("{} keys", a.keys.len()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_gain_roundtrip() {
        assert_eq!(gain_to_db(1.0), 0.0);
        assert!((db_to_gain(6.0) - 1.9953).abs() < 1e-3);
        assert_eq!(db_to_gain(-60.0), 0.0);
        assert_eq!(gain_to_db(0.0), -60.0);
        for db in [-59.0, -20.0, -3.0, 0.0, 6.0, 12.0] {
            assert!((gain_to_db(db_to_gain(db)) - db).abs() < 1e-9, "{db}");
        }
        for g in [0.002, 0.1, 0.5, 1.0, 2.0, 3.98] {
            assert!((db_to_gain(gain_to_db(g)) - g).abs() < 1e-9, "{g}");
        }
    }
}
