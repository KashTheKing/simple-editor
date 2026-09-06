//! Text/typography editing extracted from `clip_section` (see inspector.rs's module doc for the whole
//! inspector): the text body, font/size/colour/outline/shadow/align/spacing/box grid, and per-selection
//! style overrides (`TextSpan`) via the "Style Selection…"/"Clear Style" buttons and the "Set Text
//! Style" popup. Zero behaviour change from the original inline code.
//!
//! The "Text style presets" sub-panel (save/apply/delete/import/export of `Settings::text_presets`)
//! stays in inspector.rs's `clip_section`, since this fn's signature has no `&mut Settings` param —
//! `span_draft_at`/`set_span` below are `pub(super)` so that sub-panel can still reach them.

use crate::model::{Id, Project, TextSpan, TextStyle};
use crate::settings::TextPreset;
use crate::theme::Palette;
use crate::ui::Gesture;
use eframe::egui::{self, DragValue, Grid};

/// Test-only: remember a widget rect so headless tests can click the real button (mirrors
/// inspector.rs's own `mark`, duplicated here to avoid cross-file plumbing for a test-only shim).
#[cfg(test)]
fn mark(ui: &egui::Ui, name: &str, r: &egui::Response) {
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("insp", name.to_string())), r.rect));
}
#[cfg(not(test))]
fn mark(_ui: &egui::Ui, _name: &str, _r: &egui::Response) {}

/// Seed a "Set Text Style" popup draft for the char range [a, b): an existing span exactly covering
/// that range wins (edit it in place), else every field starts from the clip's base style.
pub(super) fn span_draft_at(style: &TextStyle, a: usize, b: usize) -> TextPreset {
    let base = TextPreset {
        name: String::new(),
        font: style.font.clone(),
        size: style.size,
        bold: style.bold,
        italic: style.italic,
        color: style.color,
        letter_spacing: style.letter_spacing,
    };
    let Some(s) = style.spans.iter().find(|s| s.start == a && s.end == b) else { return base };
    TextPreset {
        name: String::new(),
        font: s.font.clone().unwrap_or(base.font),
        size: s.size.unwrap_or(base.size),
        bold: s.bold.unwrap_or(base.bold),
        italic: s.italic.unwrap_or(base.italic),
        color: s.color.unwrap_or(base.color),
        letter_spacing: s.letter_spacing.unwrap_or(base.letter_spacing),
    }
}

/// Push (or replace, if one already exists over the exact same range) a fully-overriding `TextSpan`
/// covering [a, b) with `p`'s fields. Ponytail: a span is always a full override of every field this
/// editor exposes, not a sparse per-field one — simpler than a per-field "inherit" toggle in the popup,
/// and still correct since it only ever writes the fields the UI let the user see/change.
pub(super) fn set_span(style: &mut TextStyle, a: usize, b: usize, p: &TextPreset) {
    style.spans.retain(|s| !(s.start == a && s.end == b));
    style.spans.push(TextSpan {
        start: a,
        end: b,
        font: Some(p.font.clone()),
        size: Some(p.size),
        bold: Some(p.bold),
        italic: Some(p.italic),
        color: Some(p.color),
        letter_spacing: Some(p.letter_spacing),
        ..Default::default()
    });
}

/// The editable fields of a `TextPreset` — shared by the "Set Text Style" popup and (implicitly, same
/// shape) the saved-preset list in clip_section's presets sub-panel.
pub(super) fn text_preset_fields(ui: &mut egui::Ui, p: &mut TextPreset, fonts: &[String]) {
    Grid::new("text_preset_fields").num_columns(2).show(ui, |ui| {
        ui.label("Font");
        egui::ComboBox::from_id_salt("span_font").selected_text(p.font.clone()).show_ui(ui, |ui| {
            if !fonts.iter().any(|f| *f == p.font) {
                let _ = ui.selectable_label(true, p.font.as_str());
            }
            for f in fonts {
                ui.selectable_value(&mut p.font, f.clone(), f);
            }
        });
        ui.end_row();
        ui.label("Size");
        ui.horizontal(|ui| {
            ui.add(DragValue::new(&mut p.size).range(1.0..=1000.0));
            ui.checkbox(&mut p.bold, "Bold");
            ui.checkbox(&mut p.italic, "Italic");
        });
        ui.end_row();
        ui.label("Colour");
        ui.color_edit_button_srgba_unmultiplied(&mut p.color);
        ui.end_row();
        ui.label("Letter spacing");
        ui.add(DragValue::new(&mut p.letter_spacing).range(-10.0..=50.0).speed(0.1));
        ui.end_row();
    });
}

/// The Text/typography block: multiline body, font/size/colour/outline/shadow/align/spacing/box grid,
/// and per-selection style overrides. Returns true if anything changed. No-op (returns false) unless
/// the representative clip (`ids.first()`) is a Text clip.
#[allow(clippy::too_many_arguments)]
pub(super) fn section(
    ui: &mut egui::Ui,
    project: &mut Project,
    ids: &[Id],
    fonts: &[String],
    palette: &Palette,
    undo: &mut dyn FnMut(&Project),
) -> bool {
    let _ = palette; // kept for signature parity with inspector_audio::section / future use
    let Some(&id) = ids.first() else {
        return false;
    };
    let Some(orig) = project.clip(id).cloned() else {
        return false;
    };
    if orig.kind != crate::model::ClipKind::Text {
        return false;
    }
    let mut clip = orig.clone();
    let mut g = Gesture::default();
    let style = clip.text.get_or_insert_with(Default::default);

    ui.separator();
    ui.strong("Text");
    // snapshot the wording so per-word spans can follow the characters they style across this
    // frame's edit (typing before a styled word used to shift the styling onto the wrong chars)
    let text_before = (!style.spans.is_empty()).then(|| style.text.clone());
    let text_out = egui::TextEdit::multiline(&mut style.text).id_salt("inspector_text_body").show(ui);
    g.note_text(&text_out.response);
    if text_out.response.changed() {
        if let Some(before) = text_before {
            style.remap_spans(&before);
        }
    }
    // char range of the current selection (empty/collapsed = no selection) — used by "Style
    // Selection…" below to know what a new/edited TextSpan should cover.
    let live_sel: Option<(usize, usize)> = text_out.cursor_range.and_then(|r| {
        let (a, b) = (r.primary.index, r.secondary.index);
        (a != b).then(|| (a.min(b), a.max(b)))
    });
    // the text field loses focus (cursor_range -> None) the moment a button elsewhere is clicked,
    // so "Style Selection…"/"Apply to Selection" need the LAST non-empty selection, not this
    // frame's live one, to still know what to target once actually clicked.
    // keyed by clip id: a selection made on one clip must not arm the span buttons on another
    let text_sel_id = egui::Id::new(("inspector_text_sel", id));
    if live_sel.is_some() {
        ui.ctx().data_mut(|d| d.insert_temp(text_sel_id, live_sel));
    }
    let text_sel: Option<(usize, usize)> =
        ui.ctx().data(|d| d.get_temp::<Option<(usize, usize)>>(text_sel_id)).flatten();
    Grid::new("inspector_text").num_columns(2).show(ui, |ui| {
        ui.label("Font");
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("font").selected_text(style.font.clone()).show_ui(ui, |ui| {
                if !fonts.iter().any(|f| *f == style.font) {
                    let _ = ui.selectable_label(true, style.font.as_str());
                }
                for f in fonts {
                    g.note(&ui.selectable_value(&mut style.font, f.clone(), f));
                }
            });
            if ui.small_button("Import font…").clicked() {
                if let Some(p) = rfd::FileDialog::new().add_filter("Fonts", &["ttf", "otf"]).pick_file() {
                    crate::ui::inspector::set_pending_font_import(p.to_string_lossy().into_owned());
                }
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
        ui.label("Drop shadow");
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

    // ---- per-selection style override (TextSpan) ----
    // Only the fields the rasterizer actually honours per-span today (see TextSpan's doc comment
    // in model.rs): font/size/bold/italic/colour/letter-spacing. Outline/shadow stay clip-wide.
    ui.separator();
    let span_draft_id = egui::Id::new(("inspector_text_span_draft", id)); // per clip, like text_sel
    ui.horizontal(|ui| {
        match text_sel {
            Some((a, b)) if b > a => {
                ui.label(format!("{} character{} selected", b - a, if b - a == 1 { "" } else { "s" }));
            }
            _ => {
                ui.weak("Select text above, then style just that range");
            }
        }
        let can = matches!(text_sel, Some((a, b)) if b > a);
        let r = ui.add_enabled(can, egui::Button::new("Style Selection…"));
        mark(ui, "style_selection", &r);
        if r.clicked() {
            if let Some((a, b)) = text_sel {
                let seed = span_draft_at(style, a, b);
                ui.ctx().data_mut(|d| d.insert_temp(span_draft_id, (a, b, seed)));
            }
        }
        // the undo for a styled word: drop the per-char overrides back to the clip style
        let overlaps = |a: usize, b: usize| style.spans.iter().any(|sp| sp.start < b && sp.end > a);
        let can_clear = matches!(text_sel, Some((a, b)) if b > a && overlaps(a, b));
        let r = ui.add_enabled(can_clear, egui::Button::new("Clear Style on Selection"));
        mark(ui, "clear_span", &r);
        if r.clicked() {
            if let Some((a, b)) = text_sel {
                undo(project);
                style.clear_span_range(a, b);
                g.changed = true;
            }
        }
    });
    let draft: Option<(usize, usize, TextPreset)> = ui.ctx().data(|d| d.get_temp(span_draft_id));
    if let Some((a, b, mut preset)) = draft {
        let mut open = true;
        let mut apply = false;
        let mut cancel = false;
        egui::Window::new("Set Text Style").resizable(false).collapsible(false).open(&mut open).show(ui.ctx(), |ui| {
            text_preset_fields(ui, &mut preset, fonts);
            ui.horizontal(|ui| {
                if ui.button("Apply").clicked() {
                    apply = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
        if apply {
            undo(project);
            set_span(style, a, b, &preset);
            g.changed = true;
        }
        if apply || cancel || !open {
            ui.ctx().data_mut(|d| d.remove::<(usize, usize, TextPreset)>(span_draft_id));
        } else {
            ui.ctx().data_mut(|d| d.insert_temp(span_draft_id, (a, b, preset)));
        }
    }

    if g.start {
        undo(project);
    }
    if g.changed {
        if let Some(c) = project.clip_mut(id) {
            c.text = clip.text.clone();
        }
    }
    g.changed
}
