//! Text/typography editing extracted from `clip_section` (see inspector.rs's module doc for the whole
//! inspector): the text body, font/size/colour/outline/shadow/align/spacing/box grid, and per-selection
//! style overrides (`TextSpan`) via the "Style Selection…"/"Clear Style" buttons and the "Set Text
//! Style" popup.
//!
//! ---- ws:text-titles ----
//! `size`/`letter_spacing`/`outline_width` are now `Animated` (schema promotion, this workstream): each
//! gets the same DragValue + `key_buttons` (the ◆, right-click for keys and path/expression links) row
//! shape the clip transform grid uses for Position X (`inspector_audio::section`), plus new Reveal
//! (typewriter, 0..100%) and Wave (per-glyph bob) rows over the like-named `Animated` fields, and an
//! Animation row (builtin/saved motion preset combo + Apply, calling `presets::apply_motion` directly -
//! mirrors curves.rs:744's inline call, not curves.rs:762's unrelated PENDING_MOTION "Save motion"
//! plumbing). `inspector.rs::clip_section` now calls this fn BEFORE the generic transform grid for a
//! Text clip (primary-first ordering), which is why `section` takes `playhead` - the transform grid used
//! to be the thing computing `clip.local(playhead)` first.
//!
//! The "Text style presets" sub-panel (save/apply/delete/import/export of `Settings::text_presets`)
//! stays in inspector.rs's `clip_section`, since this fn's signature has no `&mut Settings` param -
//! `span_draft_at`/`set_span` below are `pub(super)` so that sub-panel can still reach them.

use crate::model::{AnimLink, Animated, Id, Project, TextSpan, TextStyle};
use crate::settings::TextPreset;
use crate::theme::Palette;
use crate::ui::inspector::luau_highlight;
use crate::ui::Gesture;
use crate::ui::{fill, prop_field, prop_field_val, prop_label, prop_value, PROP_VAL_W};
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
        // ws:text-titles: size/letter_spacing are now Animated - a span override is a plain f32 (spans
        // don't animate, see the ponytail note on TextSpan promotion), so this seeds from the CURRENT
        // (base, unkeyed) value only, same as before the promotion for a non-animated style.
        size: style.size.value as f32,
        bold: style.bold,
        italic: style.italic,
        color: style.color,
        letter_spacing: style.letter_spacing.value as f32,
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
/// editor exposes, not a sparse per-field one - simpler than a per-field "inherit" toggle in the popup,
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

/// The editable fields of a `TextPreset` - shared by the "Set Text Style" popup and (implicitly, same
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

/// The editable expression box shown right under a row when its `Animated` field is linked to
/// `AnimLink::Expr` - same widget/pattern as `inspector_audio::prop_line` (search `link_err`/`AnimLink::Expr`
/// there), indented to the value column.
fn expr_edit_row(ui: &mut egui::Ui, a: &mut Animated, g: &mut Gesture) {
    if let Animated { link: AnimLink::Expr(src), link_err, .. } = a {
        ui.horizontal(|ui| {
            ui.add_space(crate::ui::PROP_LABEL_W + ui.spacing().item_spacing.x);
            let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
                let mut job = luau_highlight(ui, buf.as_str());
                job.wrap.max_width = wrap_width;
                ui.fonts_mut(|f| f.layout_job(job))
            };
            g.note(
                &ui.add(
                    egui::TextEdit::singleline(src)
                        .desired_width(150.0)
                        .hint_text("return value + math.sin(t*4) * 20")
                        .layouter(&mut layouter),
                ),
            );
            if let Some(e) = link_err {
                ui.colored_label(ui.visuals().warn_fg_color, "!").on_hover_text(e.clone());
            }
        });
    }
}

/// A keyframable text row's value box (`ui::prop_value`): a DragValue over `a` in its stored unit, "Mixed"
/// while the selected clips differ (until it is changed).
#[allow(clippy::too_many_arguments)]
fn anim_value(
    ui: &mut egui::Ui,
    a: &mut Animated,
    lt: f64,
    palette: &Palette,
    g: &mut Gesture,
    label: &str,
    mixed: bool,
    dv: impl FnOnce(&mut f64) -> DragValue<'_>,
) -> egui::Response {
    prop_value(ui, a, lt, palette, g, label, &[], |ui, a| {
        let mut v = a.at(lt);
        let mut d = dv(&mut v).clamp_existing_to_range(false);
        if mixed {
            d = d.custom_formatter(|_, _| MIXED.to_string());
        }
        let r = fill(ui, d);
        if r.changed() {
            a.set_at(lt, v);
        }
        r
    })
    .0
}

/// A plain DragValue, "Mixed" while the selected clips differ.
fn mixed_dv<'a>(dv: DragValue<'a>, mixed: bool) -> DragValue<'a> {
    if mixed {
        dv.custom_formatter(|_, _| MIXED.to_string())
    } else {
        dv
    }
}

const MIXED: &str = "Mixed";

/// The Text/typography block: multiline body, font/size/colour/outline/shadow/align/spacing/box rows,
/// and per-selection style overrides. Every field edits every Text clip in `ids` at once (one undo step);
/// a field the clips disagree on reads "Mixed" until it is changed. Returns true if anything changed.
/// No-op (returns false) unless the representative clip (`ids.first()`) is a Text clip.
#[allow(clippy::too_many_arguments)]
pub(super) fn section(
    ui: &mut egui::Ui,
    project: &mut Project,
    ids: &[Id],
    playhead: f64,
    fonts: &[String],
    palette: &Palette,
    undo: &mut dyn FnMut(&Project),
) -> bool {
    let Some(&id) = ids.first() else {
        return false;
    };
    let Some(orig) = project.clip(id).cloned() else {
        return false;
    };
    if orig.kind != crate::model::ClipKind::Text {
        return false;
    }
    // ws:text-titles: size/letter_spacing/outline_width/reveal/wave are Animated now, keyed at the
    // clip-local playhead exactly like the Transform rows (inspector_audio::section).
    let lt = orig.local(playhead);
    let mut clip = orig.clone();
    let mut g = Gesture::default();
    let style = clip.text.get_or_insert_with(Default::default);
    let base = style.clone();
    // the other selected text clips' styles: a field any of them disagrees on reads "Mixed"
    let others: Vec<TextStyle> =
        ids[1..].iter().filter_map(|&i| project.clip(i).and_then(|c| c.text.clone())).collect();
    macro_rules! mixed {
        ($($f:ident),+) => { others.iter().any(|o| $(o.$f != base.$f)||+) };
    }

    // snapshot the wording so per-word spans can follow the characters they style across this
    // frame's edit (typing before a styled word used to shift the styling onto the wrong chars)
    let text_before = (!style.spans.is_empty()).then(|| style.text.clone());
    let live_sel: Option<(usize, usize)> = if mixed!(text) {
        // different wording on the selected clips: an empty box, and what is typed becomes every clip's text
        let mut buf = String::new();
        let r = ui.add(egui::TextEdit::multiline(&mut buf).id_salt("inspector_text_body").hint_text(MIXED));
        g.note_text(&r);
        if r.changed() {
            let before = style.text.clone();
            style.text = buf;
            style.remap_spans(&before);
        }
        None
    } else {
        let text_out = egui::TextEdit::multiline(&mut style.text).id_salt("inspector_text_body").show(ui);
        g.note_text(&text_out.response);
        if text_out.response.changed() {
            if let Some(before) = text_before {
                style.remap_spans(&before);
            }
        }
        // char range of the current selection (empty/collapsed = no selection) - used by "Style
        // Selection…" below to know what a new/edited TextSpan should cover.
        text_out.cursor_range.and_then(|r| {
            let (a, b) = (r.primary.index, r.secondary.index);
            (a != b).then(|| (a.min(b), a.max(b)))
        })
    };
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
    let mixed_note = |ui: &mut egui::Ui, m: bool| {
        if m {
            ui.weak(MIXED);
        }
    };
    prop_field(ui, "Font", |ui| {
        let shown = if mixed!(font) { MIXED.to_string() } else { style.font.clone() };
        egui::ComboBox::from_id_salt("font").selected_text(shown).width(PROP_VAL_W + 64.0).show_ui(ui, |ui| {
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
    prop_field(ui, "Size", |ui| {
        let r = anim_value(ui, &mut style.size, lt, palette, &mut g, "Text Size", mixed!(size), |v| {
            DragValue::new(v).range(1.0..=1000.0)
        });
        mark(ui, "text_size", &r);
        g.note(&ui.add(egui::Checkbox::new(&mut style.bold, "Bold").indeterminate(mixed!(bold))));
        g.note(&ui.add(egui::Checkbox::new(&mut style.italic, "Italic").indeterminate(mixed!(italic))));
    });
    expr_edit_row(ui, &mut style.size, &mut g);
    prop_field(ui, "Fill", |ui| {
        g.note(&ui.color_edit_button_srgba_unmultiplied(&mut style.color));
        mixed_note(ui, mixed!(color));
    });
    prop_field(ui, "Outline", |ui| {
        anim_value(ui, &mut style.outline_width, lt, palette, &mut g, "Outline Width", mixed!(outline_width), |v| {
            DragValue::new(v).range(0.0..=50.0).speed(0.1)
        });
        g.note(&ui.color_edit_button_srgba_unmultiplied(&mut style.outline_color));
        mixed_note(ui, mixed!(outline_color));
    });
    expr_edit_row(ui, &mut style.outline_width, &mut g);
    prop_field(ui, "Drop shadow", |ui| {
        g.note(&ui.add(egui::Checkbox::new(&mut style.shadow, "").indeterminate(mixed!(shadow))));
        g.note(&ui.color_edit_button_srgba_unmultiplied(&mut style.shadow_color));
        mixed_note(ui, mixed!(shadow_color));
    });
    prop_field(ui, "Shadow x/y/blur", |ui| {
        g.note(&ui.add(mixed_dv(DragValue::new(&mut style.shadow_x).range(-200.0..=200.0), mixed!(shadow_x))));
        g.note(&ui.add(mixed_dv(DragValue::new(&mut style.shadow_y).range(-200.0..=200.0), mixed!(shadow_y))));
        let blur = DragValue::new(&mut style.shadow_blur).range(0.0..=50.0).speed(0.1);
        g.note(&ui.add(mixed_dv(blur, mixed!(shadow_blur))));
    });
    let mut align_set = false;
    prop_field(ui, "Align", |ui| {
        let m = mixed!(align);
        for (i, name) in ["Left", "Center", "Right"].iter().enumerate() {
            let r = ui.selectable_label(!m && style.align == i as u8, *name);
            if r.clicked() && (m || style.align != i as u8) {
                style.align = i as u8;
                align_set = true;
                g.click();
            }
        }
    });
    let r = prop_field_val(
        ui,
        "Line spacing",
        mixed_dv(DragValue::new(&mut style.line_spacing).range(0.5..=3.0).speed(0.01), mixed!(line_spacing)),
    );
    g.note(&r);
    prop_field(ui, "Letter spacing", |ui| {
        anim_value(ui, &mut style.letter_spacing, lt, palette, &mut g, "Letter Spacing", mixed!(letter_spacing), |v| {
            DragValue::new(v).range(-10.0..=50.0).speed(0.1)
        });
    });
    expr_edit_row(ui, &mut style.letter_spacing, &mut g);
    prop_field(ui, "Box", |ui| {
        let pad = DragValue::new(&mut style.box_padding).range(0.0..=100.0).speed(0.5);
        let r = ui.add_sized([PROP_VAL_W, ui.spacing().interact_size.y], mixed_dv(pad, mixed!(box_padding)));
        g.note(&r);
        g.note(&ui.color_edit_button_srgba_unmultiplied(&mut style.box_color));
        mixed_note(ui, mixed!(box_color));
    });
    // ---- ws:social-text: box shape (one block / a box per line / a full-width bar) + corners ----
    prop_field(ui, "Box shape", |ui| {
        let m = mixed!(box_mode);
        for (v, name, tip) in [
            (crate::model::BOX_BLOCK, "Block", "One box around the text"),
            (crate::model::BOX_LINES, "Lines", "A box per line that hugs its width; the boxes merge into one shape"),
            (crate::model::BOX_BAR, "Bar", "A band the full width of the frame; the clip moves up and down only"),
        ] {
            let r = ui.selectable_label(!m && style.box_mode == v, name).on_hover_text(tip);
            if r.clicked() && (m || style.box_mode != v) {
                style.box_mode = v;
                g.click();
            }
        }
    });
    let r = prop_field_val(
        ui,
        "Corner radius",
        mixed_dv(DragValue::new(&mut style.box_radius).range(0.0..=200.0).speed(0.5), mixed!(box_radius)),
    );
    g.note(&r);
    // ---- ws:text-titles: reveal (typewriter) + wave (per-glyph bob) ----
    ui.horizontal(|ui| {
        prop_label(ui, "Reveal").on_hover_text("Typewriter progress: 0% hides all glyphs, 100% shows them all");
        anim_value(ui, &mut style.reveal, lt, palette, &mut g, "Reveal", mixed!(reveal), |v| {
            DragValue::new(v)
                .range(0.0..=1.0)
                .speed(0.005)
                .custom_formatter(|v, _| format!("{:.0} %", v * 100.0))
                .custom_parser(|s| s.trim().trim_end_matches('%').trim().parse::<f64>().ok().map(|p| p / 100.0))
        });
    });
    expr_edit_row(ui, &mut style.reveal, &mut g);
    ui.horizontal(|ui| {
        prop_label(ui, "Wave").on_hover_text("Per-glyph vertical bob amount (project px); 0 = none");
        anim_value(ui, &mut style.wave, lt, palette, &mut g, "Wave", mixed!(wave), |v| {
            DragValue::new(v).range(0.0..=200.0).speed(0.5)
        });
    });
    expr_edit_row(ui, &mut style.wave, &mut g);

    // ---- ws:text-titles: Animation row - apply a builtin/saved motion preset to the WHOLE clip's
    // transform (Position/Scale/Rotation/Opacity), mirrors curves.rs:744's inline apply_motion() call
    // (not curves.rs:762's PENDING_MOTION, which belongs to the unrelated "Save motion" button). No
    // ACT_HANDLERS entry - this panel already owns `&mut Project` via `undo`/the write-back below.
    let motions = crate::ui::curves::available_motions();
    if !motions.is_empty() {
        let motion_sel_id = egui::Id::new(("inspector_text_motion_sel", id));
        let mut sel: usize = ui.ctx().data(|d| d.get_temp(motion_sel_id).unwrap_or(0)).min(motions.len() - 1);
        prop_field(ui, "Animation", |ui| {
            egui::ComboBox::from_id_salt("text_motion_preset")
                .selected_text(motions[sel].name.clone())
                .width(PROP_VAL_W + 32.0)
                .show_ui(ui, |ui| {
                    for (i, m) in motions.iter().enumerate() {
                        ui.selectable_value(&mut sel, i, &m.name);
                    }
                });
            if ui.small_button("Apply").on_hover_text("Stretch the preset to each clip's length").clicked() {
                // Applies to the LIVE clips directly (not the local `clip` clone this fn writes back only
                // `.text` from) - a motion preset keys Position/Scale/Rotation/Opacity, none of which
                // `section`'s write-back at the bottom touches. `g.changed = true` below is what makes
                // this fn (and clip_section/inspector::show above it) return true so the caller still
                // runs its usual after-edit refresh, even though this specific write bypassed `clip`.
                undo(project);
                for &i in ids {
                    if let Some(c) = project.clip_mut(i) {
                        crate::engine::presets::apply_motion(&motions[sel], c, true);
                    }
                }
                g.changed = true;
            }
        });
        ui.ctx().data_mut(|d| d.insert_temp(motion_sel_id, sel));
    }

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
        let new = clip.text.clone().unwrap_or_default();
        if let Some(c) = project.clip_mut(id) {
            c.text = clip.text.clone();
        }
        // the other selected text clips: every field changed here, the rest stay their own
        for &sid in &ids[1..] {
            let Some(st) = project.clip_mut(sid).and_then(|c| c.text.as_mut()) else { continue };
            macro_rules! sync {
                ($($f:ident),+) => { $( if new.$f != base.$f { st.$f = new.$f.clone(); } )+ };
            }
            sync!(
                font,
                size,
                bold,
                italic,
                color,
                outline_width,
                outline_color,
                shadow,
                shadow_color,
                shadow_x,
                shadow_y
            );
            sync!(shadow_blur, line_spacing, letter_spacing, box_color, box_padding, reveal, wave);
            sync!(box_mode, box_radius);
            if new.align != base.align || align_set {
                st.align = new.align;
            }
            if new.text != base.text {
                let before = std::mem::replace(&mut st.text, new.text.clone());
                st.remap_spans(&before);
            }
        }
    }
    g.changed
}
