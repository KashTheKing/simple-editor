//! egui UI. Dockable layout (ui/layout.rs) of panes: Library, Preview, Inspector, Effects, Transitions,
//! Subtitles, Timeline, Curves, Planner, Auto-cut — DaVinci-like by default, bare Windows-forms styling.
//! Windows (Settings, Retime, Export) never block the editor.

pub mod app;
pub mod autocut_ui;
pub mod capture_ui;
pub mod curves;
pub mod effects_ui;
pub mod export_ui;
pub mod frame_ui;
pub mod import_ui;
pub mod inspector;
pub mod layout;
pub mod library;
pub mod markers_ui;
pub mod mixer_ui;
pub mod nodes;
pub mod paste_ui;
pub mod planner;
pub mod preview;
pub mod retime;
pub mod settings_ui;
pub mod subtitles_ui;
pub mod timeline;
pub mod tools;
pub mod transitions_ui;

use crate::model::{Animated, Id, Mask, MaskShape, Project, LABEL_COLORS};
use crate::theme::Palette;
use eframe::egui::{self, Button, DragValue, Grid, Response};

/// True on the first frame of an edit gesture (drag start, or a non-drag change such as typing/clicking).
pub(crate) fn edit_start(r: &Response) -> bool {
    r.drag_started() || (r.changed() && !r.dragged())
}

/// Accumulates widget responses over one frame: `start` → push undo once, `changed` → write back.
/// Shared by every panel so the gesture rule lives in one place.
#[derive(Default)]
pub(crate) struct Gesture {
    pub start: bool,
    pub changed: bool,
}

impl Gesture {
    pub fn note(&mut self, r: &Response) {
        self.start |= edit_start(r);
        self.changed |= r.changed();
    }
    /// Text fields: the gesture starts when the field is entered, so typing a paragraph is one undo
    /// entry instead of one per character (which used to evict the whole undo stack).
    pub fn note_text(&mut self, r: &Response) {
        self.start |= r.gained_focus();
        self.changed |= r.changed();
    }
    pub fn click(&mut self) {
        self.start = true;
        self.changed = true;
    }
}

/// Push undo at most once per frame.
pub(crate) fn once(flag: &mut bool, undo: &mut dyn FnMut(&Project), p: &Project) {
    if !*flag {
        undo(p);
        *flag = true;
    }
}

/// "◆" keyframe toggle at the clip-local playhead + "✕ keys" once the parameter is animated.
pub(crate) fn key_buttons(ui: &mut egui::Ui, a: &mut Animated, lt: f64, palette: &Palette, g: &mut Gesture) {
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

/// A Polygon/Path mask is drawn from `points`; with fewer than 3 vertices the rasteriser reports
/// "outside everywhere" and the masked clip disappears with no way back. Seed the vertices from the
/// mask's own rect so every shape switch leaves something visible and editable.
pub(crate) fn seed_mask_points(m: &mut Mask) {
    if !matches!(m.shape, MaskShape::Polygon | MaskShape::Path) || m.points.len() >= 3 {
        return;
    }
    let (cx, cy) = (m.cx.value as f32, m.cy.value as f32);
    let (rx, ry) = ((m.rx.value as f32).abs().max(1.0), (m.ry.value as f32).abs().max(1.0));
    m.points = vec![(cx - rx, cy - ry), (cx + rx, cy - ry), (cx + rx, cy + ry), (cx - rx, cy + ry)];
}

/// Mask parameter grid, shared by the Inspector (the clip mask) and the Effects panel (a per-effect
/// mask): shape, enabled, invert, position/size/rotation, feather, expand, opacity — each keyframable.
pub(crate) fn mask_grid(ui: &mut egui::Ui, m: &mut Mask, lt: f64, palette: &Palette, g: &mut Gesture, salt: egui::Id) {
    Grid::new(salt).num_columns(2).show(ui, |ui| {
        ui.label("Shape");
        egui::ComboBox::from_id_salt(salt.with("shape")).selected_text(m.shape.name()).show_ui(ui, |ui| {
            for sh in MaskShape::ALL {
                g.note(&ui.selectable_value(&mut m.shape, sh, sh.name()));
            }
        });
        seed_mask_points(m);
        ui.end_row();
        ui.label("Enabled");
        g.note(&ui.checkbox(&mut m.enabled, ""));
        ui.end_row();
        ui.label("Invert");
        g.note(&ui.checkbox(&mut m.invert, ""));
        ui.end_row();
        // X/Y/radius are here too: a mask added from a panel has no viewport drag behind it and would
        // otherwise be stuck at its default rect in the middle of the layer.
        let rows: [(&str, f64, f64, f64); 8] = [
            ("X", -10000.0, 10000.0, 0.5),
            ("Y", -10000.0, 10000.0, 0.5),
            ("Radius X", 0.0, 10000.0, 0.5),
            ("Radius Y", 0.0, 10000.0, 0.5),
            ("Rotation", -360.0, 360.0, 0.5),
            ("Feather", 0.0, 500.0, 0.5),
            ("Expand", -500.0, 500.0, 0.5),
            ("Opacity", 0.0, 1.0, 0.01),
        ];
        for (label, lo, hi, speed) in rows {
            ui.label(label);
            ui.horizontal(|ui| {
                let a: &mut Animated = match label {
                    "X" => &mut m.cx,
                    "Y" => &mut m.cy,
                    "Radius X" => &mut m.rx,
                    "Radius Y" => &mut m.ry,
                    "Rotation" => &mut m.rotation,
                    "Feather" => &mut m.feather,
                    "Expand" => &mut m.expand,
                    _ => &mut m.opacity,
                };
                let mut v = a.at(lt);
                let r = ui.add(DragValue::new(&mut v).range(lo..=hi).speed(speed).clamp_existing_to_range(false));
                if r.changed() {
                    a.set_at(lt, v);
                }
                g.note(&r);
                key_buttons(ui, a, lt, palette, g);
            });
            ui.end_row();
        }
    });
}

/// Drag-and-drop payload from the library / recent / planner panels to the timeline (egui dnd API).
#[derive(Clone, Debug)]
pub enum DragPayload {
    /// A library asset id.
    Asset(Id),
    /// A file path (recent panel, linked folders) — the timeline reports it back as a dropped file.
    Path(String),
    /// A nested timeline (Project.sequences) id.
    Sequence(Id),
    /// A saved template (Settings.templates) by name.
    Template(String),
}

/// HH:MM:SS:FF timecode.
pub fn timecode(t: f64, fps: f64) -> String {
    let t = t.max(0.0);
    let fps = fps.max(1.0);
    let total_frames = (t * fps).round() as u64;
    let fpsr = fps.round() as u64;
    let f = total_frames % fpsr;
    let s = total_frames / fpsr;
    format!("{:02}:{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60, f)
}

/// Short duration text "1:23.4".
pub fn duration_text(t: f64) -> String {
    let m = (t / 60.0).floor() as u64;
    let s = t - m as f64 * 60.0;
    format!("{m}:{s:04.1}")
}

/// Colour of a clip/asset/recent label index (0 or out of range = "no label" dim).
pub fn label_color(idx: u8, palette: &Palette) -> egui::Color32 {
    if idx == 0 || idx as usize > LABEL_COLORS.len() {
        palette.text_dim
    } else {
        let [r, g, b] = LABEL_COLORS[idx as usize - 1].1;
        egui::Color32::from_rgb(r, g, b)
    }
}

/// Name of a label index ("None" for 0 / out of range).
pub fn label_name(idx: u8) -> &'static str {
    if idx == 0 || idx as usize > LABEL_COLORS.len() {
        "None"
    } else {
        LABEL_COLORS[idx as usize - 1].0
    }
}

/// ComboBox over (value, label) pairs writing into a String setting. Returns true if the value changed.
pub fn combo(ui: &mut egui::Ui, id: &str, value: &mut String, options: &[(&str, &str)], width: Option<f32>) -> bool {
    let mut changed = false;
    let current = options.iter().find(|(v, _)| *v == value.as_str()).map(|(_, l)| *l).unwrap_or(value.as_str());
    let mut cb = egui::ComboBox::from_id_salt(id).selected_text(current);
    if let Some(w) = width {
        cb = cb.width(w);
    }
    cb.show_ui(ui, |ui| {
        for (v, label) in options {
            if ui.selectable_label(value.as_str() == *v, *label).clicked() && value.as_str() != *v {
                *value = (*v).to_string();
                changed = true;
            }
        }
    });
    changed
}

/// x264-style speed presets offered by the settings and export windows.
pub const ENCODER_PRESETS: [&str; 7] = ["ultrafast", "superfast", "veryfast", "faster", "fast", "medium", "slow"];

/// Encoder names worth offering: the h264 / hevc / vp9 / av1 families (software + nvenc/qsv/amf).
pub fn encoder_options(encoders: &[String]) -> Vec<&str> {
    const KEYS: [&str; 9] = ["264", "265", "hevc", "vp9", "av1", "x264", "x265", "nvenc", "qsv"];
    encoders.iter().map(String::as_str).filter(|e| KEYS.iter().any(|k| e.contains(k)) || e.contains("amf")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polygon_masks_never_stay_empty() {
        let mut m = Mask::default();
        seed_mask_points(&mut m);
        assert!(m.points.is_empty(), "a rect mask is drawn from cx/cy/rx/ry, not points");
        m.shape = MaskShape::Polygon;
        seed_mask_points(&mut m);
        // < 3 points rasterises as "outside everywhere": the masked clip would vanish
        assert_eq!(m.points.len(), 4, "{:?}", m.points);
        assert!(m.points.iter().any(|p| p.0 < 0.0) && m.points.iter().any(|p| p.0 > 0.0));
        m.points.push((5.0, 5.0));
        let kept = m.points.clone();
        seed_mask_points(&mut m);
        assert_eq!(m.points, kept, "an existing outline is never overwritten");
    }
}
