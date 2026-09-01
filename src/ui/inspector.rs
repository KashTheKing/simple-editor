//! Inspector panel. Nothing selected → Project panel (name, width, height, fps, duration, Save / Export /
//! Export Frame buttons, export settings summary). One or more clips selected →
//! clip name, enabled, colour label, timing (start / duration / source in), retime readout (Ctrl+R), and:
//!  * multi-selection: Enabled, Label, transform/opacity (or volume/pan), fades and Blend edit every
//!    selected clip at once (absolute overwrite of whatever changed, diff-against-original — mirrors
//!    `transition_section`'s bulk-edit rule); everything else in this file (Name included) is disabled
//!    and needs a single-clip selection.
//!  * visual clips: Position X/Y, Scale, Rotation (DragValue), Opacity (0-100% slider) — each row + a
//!    diamond keyframe toggle (`Animated::toggle_key(clip.local(playhead))`, highlighted when a key exists at the
//!    playhead) + "clear keys"; edits go through `Animated::set_at(local_t, v)` so keyframed props get
//!    keys; Blend mode combo.
//!  * audio clips: Volume (dB slider mapped to linear gain) and Pan (-1..1 slider) with the same keyframe
//!    controls, plus fade in/out lengths.
//!  * effects summary (enable toggles, remove) and transitions touching the clip.
//!  * the clip's asset: "Used in: N clips", editable description and tags.
//!  * text clips: multiline text, font family combo (`fonts`) + "Import font…" (handed to the app via
//!    `take_pending_font_import`), size, bold/italic, colour pickers (fill, outline, drop shadow, box),
//!    outline width, drop shadow on/off + x/y/blur, alignment, line/letter spacing, box padding.
//!  * sequence clips: the sequence's name + "Open sequence" (handed to the app via `take_open_sequence`).
//!  * round 3: the clip's label from `Project.labels` (plus a compact "Edit labels…" editor), its mask
//!    (shape, position, radius, rotation, feather, expand, opacity, invert, "Edit in viewport" — the same
//!    grid the Effects panel shows under a per-effect mask), "Open node editor", the shape style
//!    of Shape clips (kind, fill/stroke, width, sides, corner, draw rate, page), clip markers (add /
//!    remove), an audio bus override, and a note on Adjustment clips.
//! Call `undo(project)` once per gesture (on `drag_started()` / first `changed()` of a widget) before mutating.
//! Returns true if the project changed.

use crate::hotkeys::Action;
use crate::model::{
    AnimLink, Animated, BlendMode, ClipKind, Id, Label, Mask, Project, ShapeKind, ShapeStyle, TextSpan, TextStyle,
};
use crate::settings::{Settings, TextPreset};
use crate::theme::Palette;
use crate::ui::markers_ui::x_button;
use crate::ui::{edit_start, key_buttons, mask_grid, timecode, Gesture};
use eframe::egui::{self, DragValue, Grid, Response, RichText, Slider};
use std::cell::RefCell;

/// Test-only: remember a widget rect so headless tests can click the real button.
#[cfg(test)]
fn mark(ui: &egui::Ui, name: &str, r: &Response) {
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("insp", name.to_string())), r.rect));
}
#[cfg(not(test))]
fn mark(_ui: &egui::Ui, _name: &str, _r: &Response) {}

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

/// Seed a "Set Text Style" popup draft for the char range [a, b): an existing span exactly covering
/// that range wins (edit it in place), else every field starts from the clip's base style.
fn span_draft_at(style: &TextStyle, a: usize, b: usize) -> TextPreset {
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
fn set_span(style: &mut TextStyle, a: usize, b: usize, p: &TextPreset) {
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
/// shape) the saved-preset list.
fn text_preset_fields(ui: &mut egui::Ui, p: &mut TextPreset, fonts: &[String]) {
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

// Hand-offs to the app (the show() signature has no room for these; the app polls them each frame).
thread_local! {
    static PENDING_FONT: RefCell<Option<String>> = const { RefCell::new(None) };
    static OPEN_SEQUENCE: RefCell<Option<Id>> = const { RefCell::new(None) };
    static EDIT_MASK: RefCell<Option<Id>> = const { RefCell::new(None) };
    static OPEN_NODES: RefCell<Option<Id>> = const { RefCell::new(None) };
    static UNLINK_NODES: RefCell<Option<Id>> = const { RefCell::new(None) };
    /// Project-panel button (Save / Export… / Export Frame… / edit export settings) — the app runs it
    /// through the same Action dispatch the menus use.
    static PENDING_ACTION: RefCell<Option<Action>> = const { RefCell::new(None) };
}

/// Clip the user asked to turn back into a plain effect stack (`Project::unlink_graph`).
pub fn take_unlink_nodes() -> Option<Id> {
    UNLINK_NODES.with(|p| p.borrow_mut().take())
}

/// Ask for it from elsewhere — the effects panel offers the same escape hatch.
pub fn ask_unlink_nodes(id: Id) {
    UNLINK_NODES.with(|p| *p.borrow_mut() = Some(id));
}

/// Clip whose mask the user wants to draw in the viewport ("Edit in viewport") — the app switches the
/// active tool to a mask tool and points the preview at this clip.
pub fn take_edit_mask() -> Option<Id> {
    EDIT_MASK.with(|p| p.borrow_mut().take())
}

/// Clip the user asked to open in the node editor.
pub fn take_open_nodes() -> Option<Id> {
    OPEN_NODES.with(|p| p.borrow_mut().take())
}

/// Font file (.ttf/.otf) the user picked with "Import font…" — the app adds it to settings.user_fonts
/// and reloads the text rasterizer.
pub fn take_pending_font_import() -> Option<String> {
    PENDING_FONT.with(|p| p.borrow_mut().take())
}

/// Sequence the user asked to open from the inspector.
pub fn take_open_sequence() -> Option<Id> {
    OPEN_SEQUENCE.with(|p| p.borrow_mut().take())
}

/// Action a project-panel button asked to run (Save / Export… / Export Frame…) — the app pushes it
/// through the same `Action` dispatch the menus and hotkeys use.
pub fn take_pending_action() -> Option<Action> {
    PENDING_ACTION.with(|p| p.borrow_mut().take())
}

#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    project: &mut Project,
    selection: &[Id],
    sel_transitions: &[Id],
    playhead: f64,
    fonts: &[String],
    palette: &Palette,
    settings: &mut Settings,
    undo: &mut dyn FnMut(&Project),
) -> bool {
    let edited = match selection.iter().find(|&&id| project.clip(id).is_some()) {
        None if !sel_transitions.is_empty() => transition_section(ui, project, sel_transitions, undo),
        None => project_section(ui, project, settings, undo),
        Some(_) => clip_section(ui, project, selection, playhead, fonts, palette, settings, undo),
    };
    // drawn from here (not project_section) so the open prompt survives selection changes
    rescale_prompt(ui, project, undo) || edited
}

/// Selected transitions (timeline bands): one set of editors; each field you change is written to
/// every selected transition, fields you leave alone keep their per-transition values.
fn transition_section(ui: &mut egui::Ui, project: &mut Project, ids: &[Id], undo: &mut dyn FnMut(&Project)) -> bool {
    use crate::model::{Ease, TransitionKind};
    let list: Vec<crate::model::Transition> = ids
        .iter()
        .filter_map(|&id| project.tracks.iter().flat_map(|t| &t.transitions).find(|t| t.id == id).cloned())
        .collect();
    let Some(first) = list.first().cloned() else {
        ui.label("Select a clip or a transition");
        return false;
    };
    let mut g = Gesture::default();
    if list.len() == 1 {
        ui.strong("Transition");
    } else {
        ui.strong(format!("Transitions ({} selected)", list.len()));
    }
    let mut e = first.clone();
    Grid::new("insp_transition").num_columns(2).show(ui, |ui| {
        ui.label("Kind");
        egui::ComboBox::from_id_salt("insp_tr_kind").selected_text(e.kind.name()).show_ui(ui, |ui| {
            for k in TransitionKind::ALL {
                g.note(&ui.selectable_value(&mut e.kind, k, k.name()));
            }
        });
        ui.end_row();
        ui.label("Duration");
        // clamp_existing_to_range(false): merely drawing an out-of-range value must not fake an edit
        g.note(&ui.add(
            DragValue::new(&mut e.duration).range(0.1..=5.0).clamp_existing_to_range(false).speed(0.02).suffix(" s"),
        ));
        ui.end_row();
        if e.kind == TransitionKind::FadeToColor {
            ui.label("Color");
            g.note(&ui.color_edit_button_srgba_unmultiplied(&mut e.color));
            ui.end_row();
        }
        if e.kind.has_direction() {
            ui.label("Direction");
            ui.horizontal(|ui| {
                for (i, name) in ["Left", "Right", "Up", "Down"].iter().enumerate() {
                    g.note(&ui.selectable_value(&mut e.direction, i as u8, *name));
                }
            });
            ui.end_row();
        }
        ui.label("Ease");
        egui::ComboBox::from_id_salt("insp_tr_ease").selected_text(e.ease.name()).show_ui(ui, |ui| {
            for ea in Ease::ALL {
                g.note(&ui.selectable_value(&mut e.ease, ea, ea.name()));
            }
        });
        ui.end_row();
    });
    let mut removed = false;
    let label = if list.len() == 1 { "Remove transition".into() } else { format!("Remove {} transitions", list.len()) };
    let r = ui.button(label);
    mark(ui, "tr_remove", &r);
    if r.clicked() {
        g.click();
        removed = true;
    }
    if g.start {
        undo(project);
    }
    if !g.changed {
        return false;
    }
    if removed {
        for &id in ids {
            project.remove_transition(id);
        }
        return true;
    }
    for &id in ids {
        if let Some(t) = project.transition_mut(id) {
            if e.kind != first.kind {
                t.kind = e.kind;
            }
            if e.duration != first.duration {
                t.duration = e.duration;
            }
            if e.color != first.color {
                t.color = e.color;
            }
            if e.direction != first.direction {
                t.direction = e.direction;
            }
            if e.ease != first.ease {
                t.ease = e.ease;
            }
        }
    }
    true
}

fn project_section(
    ui: &mut egui::Ui,
    project: &mut Project,
    settings: &mut Settings,
    undo: &mut dyn FnMut(&Project),
) -> bool {
    let mut edited = false;
    // resolution before any of this frame's Width/Height/preset/quality-tier/template widgets can
    // touch it — diffed at the bottom to offer the rescale prompt once, regardless of which control
    // changed it.
    let size_before = (project.width, project.height);
    ui.strong("Project");
    Grid::new("inspector_project").num_columns(2).show(ui, |ui| {
        ui.label("Name");
        let mut name = project.name.clone(); // ponytail: per-frame clone so undo can snapshot before the write
        let r = ui.text_edit_singleline(&mut name);
        if r.gained_focus() {
            undo(project); // once per visit to the field, not per keystroke
        }
        if r.changed() {
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
            mark(ui, &format!("project_{}", label.to_lowercase()), &r);
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
        ui.label("Duration");
        ui.monospace(timecode(project.duration(), project.fps));
        ui.end_row();
        // same setting as the preview's right-click Background submenu, editable from project settings
        // per the original ask ("this should save in project settings, and I should also be able to
        // edit it in the project settings tab")
        ui.label("Background");
        ui.horizontal(|ui| {
            use crate::model::BackgroundMode;
            let current = project.preview_bg;
            egui::ComboBox::from_id_salt("project_bg").selected_text(current.name()).show_ui(ui, |ui| {
                for mode in BackgroundMode::ALL {
                    if ui.selectable_label(current == mode, mode.name()).clicked() && current != mode {
                        undo(project);
                        project.preview_bg = mode;
                        edited = true;
                    }
                }
                let custom = matches!(current, BackgroundMode::Custom(_));
                if ui.selectable_label(custom, "Custom").clicked() && !custom {
                    undo(project);
                    project.preview_bg = BackgroundMode::Custom([0, 0, 0, 255]);
                    edited = true;
                }
            });
            if let BackgroundMode::Custom(mut rgba) = project.preview_bg {
                let cr = ui.color_edit_button_srgba_unmultiplied(&mut rgba);
                if edit_start(&cr) {
                    undo(project);
                }
                if cr.changed() {
                    project.preview_bg = BackgroundMode::Custom(rgba);
                    edited = true;
                }
            }
        });
        ui.end_row();
    });

    ui.separator();
    ui.strong("Presets");
    ui.horizontal_wrapped(|ui| {
        for p in crate::ui::guides::PRESETS {
            let active = project.width == p.w && project.height == p.h && (project.fps - p.fps).abs() < 0.001;
            let r = crate::ui::tools::glyph_text_button(ui, p.glyph, p.name).on_hover_text(format!(
                "{}×{} @ {:.0} fps{}",
                p.w,
                p.h,
                p.fps,
                if p.guide.is_some() { " · shows the platform guide overlay" } else { "" }
            ));
            if active {
                ui.painter().rect_stroke(
                    r.rect,
                    2.0,
                    egui::Stroke::new(1.0, ui.visuals().selection.stroke.color),
                    egui::StrokeKind::Inside,
                );
            }
            if r.clicked() {
                undo(project);
                project.width = p.w;
                project.height = p.h;
                project.fps = p.fps;
                edited = true;
                if settings.guide != p.guide {
                    settings.guide = p.guide;
                    settings.save();
                }
            }
        }
    });

    // quality / frame-rate quick buttons: the tier keeps the current aspect (shorter edge = tier)
    ui.horizontal_wrapped(|ui| {
        ui.label("Quality");
        for &(name, tier) in crate::ui::guides::RES_TIERS {
            let (w, h) = crate::ui::guides::scale_to_tier(project.width, project.height, tier);
            let r = ui
                .selectable_label(project.width == w && project.height == h, name)
                .on_hover_text(format!("{w}×{h} (keeps the current aspect)"));
            if r.clicked() {
                undo(project);
                (project.width, project.height) = (w, h);
                edited = true;
            }
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.label("FPS");
        for &fps in crate::ui::guides::FPS_PRESETS {
            let label = if fps.fract() == 0.0 { format!("{fps:.0}") } else { format!("{fps}") };
            if ui.selectable_label((project.fps - fps).abs() < 0.001, label).clicked() {
                undo(project);
                project.fps = fps;
                edited = true;
            }
        }
    });

    // user templates: apply / delete, plus "save current as template" with an inline name field
    if !settings.project_templates.is_empty() {
        ui.add_space(4.0);
        ui.strong("Templates");
        let mut delete = None;
        for (i, t) in settings.project_templates.iter().enumerate() {
            ui.horizontal(|ui| {
                let r = crate::ui::tools::glyph_text_button(ui, crate::ui::tools::Glyph::Template, &t.name)
                    .on_hover_text(format!("{}×{} @ {} fps", t.width, t.height, t.fps));
                if r.clicked() {
                    undo(project);
                    project.width = t.width;
                    project.height = t.height;
                    project.fps = t.fps;
                    edited = true;
                }
                if ui.small_button("✕").on_hover_text("Delete template").clicked() {
                    delete = Some(i);
                }
            });
        }
        if let Some(i) = delete {
            settings.project_templates.remove(i);
            settings.save();
        }
    }
    ui.horizontal(|ui| {
        let id = ui.id().with("tmpl_name");
        let mut name = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_default();
        let hint = format!("{}×{}", project.width, project.height);
        ui.add(egui::TextEdit::singleline(&mut name).hint_text(&hint).desired_width(120.0));
        if ui.button("Save as template").on_hover_text("Save the current size and FPS as a reusable template").clicked()
        {
            let name = if name.trim().is_empty() { hint } else { name.trim().to_string() };
            settings.project_templates.retain(|t| t.name != name);
            settings.project_templates.push(crate::settings::ProjectTemplate {
                name,
                width: project.width,
                height: project.height,
                fps: project.fps,
            });
            settings.save();
            ui.data_mut(|d| d.remove::<String>(id));
        } else {
            ui.data_mut(|d| d.insert_temp(id, name));
        }
    });

    ui.separator();
    ui.horizontal(|ui| {
        let r = ui.button("Save");
        mark(ui, "project_save", &r);
        if r.clicked() {
            PENDING_ACTION.with(|p| *p.borrow_mut() = Some(Action::Save));
        }
        let r = crate::ui::tools::glyph_text_button(ui, crate::ui::tools::Glyph::ExportArrow, "Export…");
        mark(ui, "project_export", &r);
        if r.clicked() {
            PENDING_ACTION.with(|p| *p.borrow_mut() = Some(Action::ExportVideo));
        }
        let r = ui.button("Export Frame…");
        mark(ui, "project_export_frame", &r);
        if r.clicked() {
            PENDING_ACTION.with(|p| *p.borrow_mut() = Some(Action::ExportFrame));
        }
    });

    ui.separator();
    ui.strong("Export settings");
    let scaler_name = crate::ui::export_ui::SCALERS
        .iter()
        .find(|(k, _)| *k == settings.export_scaler.as_str())
        .map(|(_, v)| *v)
        .unwrap_or(settings.export_scaler.as_str());
    let res = if settings.export_resolution.is_empty() || settings.export_resolution == "project" {
        "Project size".to_string()
    } else {
        settings.export_resolution.clone()
    };
    ui.horizontal_wrapped(|ui| {
        // same quality unit as the Export window's slider — the raw CRF is what the user asked to
        // stop having to decode ("a value that I don't know if it's compressing it or not")
        ui.weak(format!(
            "{} · quality {} % · {} · {} · {}",
            settings.encoder,
            crate::engine::export::quality_percent_from_crf(settings.crf),
            settings.preset,
            res,
            scaler_name
        ));
    });
    if ui.small_button("Edit export settings…").clicked() {
        PENDING_ACTION.with(|p| *p.borrow_mut() = Some(Action::ExportVideo));
    }

    // arm the prompt; the window itself is drawn by `rescale_prompt` from `show()` on EVERY frame, so
    // clicking a clip mid-decision (which switches the inspector to clip_section) can't vanish it
    if (project.width, project.height) != size_before
        && project.all_clips().any(|(_, c)| project.clip_native_size(c).is_some())
    {
        ui.ctx().data_mut(|d| d.insert_temp(rescale_prompt_id(), true));
    }

    edited
}

fn rescale_prompt_id() -> egui::Id {
    egui::Id::new("inspector_rescale_prompt")
}

/// "Resize existing media?" — non-blocking (plain egui::Window, never Modal). Armed by
/// `project_section` when a control changed the resolution and footage exists; stays open across
/// frames and selection changes (egui::Id-keyed temp) until answered or closed.
fn rescale_prompt(ui: &mut egui::Ui, project: &mut Project, undo: &mut dyn FnMut(&Project)) -> bool {
    let mut edited = false;
    let mut rescale_open: bool = ui.ctx().data(|d| d.get_temp(rescale_prompt_id()).unwrap_or(false));
    if rescale_open {
        let mut dismissed = false;
        egui::Window::new("Resize Media?").resizable(false).collapsible(false).open(&mut rescale_open).show(
            ui.ctx(),
            |ui| {
                ui.label("Resize existing media to fit the new project size?");
                ui.horizontal(|ui| {
                    let r = ui.button("Rescale");
                    mark(ui, "rescale_prompt_rescale", &r);
                    if r.clicked() {
                        undo(project);
                        let ids: Vec<Id> = project.all_clips().map(|(_, c)| c.id).collect();
                        for id in ids {
                            project.fit_clip_to_screen(id, false);
                        }
                        edited = true;
                        dismissed = true;
                    }
                    let r = ui.button("Keep As Is");
                    mark(ui, "rescale_prompt_keep", &r);
                    if r.clicked() {
                        dismissed = true;
                    }
                });
            },
        );
        if dismissed {
            rescale_open = false;
        }
    }
    ui.ctx().data_mut(|d| d.insert_temp(rescale_prompt_id(), rescale_open));
    edited
}

#[allow(clippy::too_many_arguments)]
fn clip_section(
    ui: &mut egui::Ui,
    project: &mut Project,
    selection: &[Id],
    playhead: f64,
    fonts: &[String],
    palette: &Palette,
    settings: &mut Settings,
    undo: &mut dyn FnMut(&Project),
) -> bool {
    // Every selected id that is still a live clip. The first is the "representative" — its widgets
    // drive the section — and edits to the bulk-editable fields below (zone 1) propagate to the rest
    // with diff-against-original, absolute-overwrite semantics (mirrors `transition_section`).
    let clip_ids: Vec<Id> = selection.iter().copied().filter(|&i| project.clip(i).is_some()).collect();
    let n_selected = clip_ids.len();
    let Some(&id) = clip_ids.first() else {
        return false;
    };
    // ponytail: edit a per-frame clone of the clip and write it back at the end — lets `undo` snapshot the
    // untouched project first without borrow gymnastics. Upgrade: per-field scratch copies if it ever shows.
    // Owned (not borrowed) so it survives the later `project.clip_mut` write-backs — needed to diff the
    // bulk-editable fields against their pre-edit values for the sibling-propagation pass.
    let Some(orig) = project.clip(id).cloned() else {
        return false;
    };
    let mut clip = orig.clone();
    let track = project.track_of(id).map(|t| project.tracks[t].name.clone()).unwrap_or_default();
    let fps = project.fps;
    let lt = clip.local(playhead);
    let mut g = Gesture::default();
    // snapshots: the widgets edit a clone of the clip, so the project must not stay borrowed
    let labels: Vec<Label> = project.labels.clone();
    let buses: Vec<(Id, String)> = project.buses.iter().map(|b| (b.id, b.name.clone())).collect();
    let labels_open_id = egui::Id::new("inspector_labels_open");
    let mut edit_labels: bool = ui.ctx().data(|d| d.get_temp(labels_open_id).unwrap_or(false));
    let mut label_ops: Vec<LabelOp> = Vec::new();
    let mut path_op: Option<PathOp> = None;
    let path_list: Vec<(Id, String)> = project.paths.iter().map(|p| (p.id, p.name.clone())).collect();
    // asset-details gesture/scratch (zone 2, but read back at commit time outside any wrap)
    let mut ga = Gesture::default();
    let mut asset_desc: Option<String> = None;
    let mut asset_tags: Option<Vec<String>> = None;
    let multi = n_selected > 1;

    if multi {
        ui.label(format!("{n_selected} clips selected"));
        // an honest header for mixed selections: props propagate by matching label, and audio clips
        // expose Volume/Pan where visual ones expose transform — so a mixed selection only bulk-edits
        // the clips matching the representative's kind, and the header must say so, not claim "all"
        let same_kind = |i: &Id| project.clip(*i).map(|c| c.is_visual()) == Some(orig.is_visual());
        let matching = clip_ids.iter().filter(|i| same_kind(i)).count();
        if matching < n_selected {
            let (this, other) = if orig.is_visual() { ("video", "audio") } else { ("audio", "video") };
            ui.weak(format!(
                "Mixed selection — property edits apply to the {matching} {this} clip{}; the {} {other} \
                 clip{} keep their own (enabled, label and fades still edit all of them).",
                if matching == 1 { "" } else { "s" },
                n_selected - matching,
                if n_selected - matching == 1 { "" } else { "s" },
            ));
        } else {
            ui.weak(
                "Transform, opacity, enabled, label, fades and blend edit all of them; everything else needs one clip.",
            );
        }
    }
    if clip.container {
        ui.add_enabled_ui(!multi, |ui| {
            ui.strong("Container Slot");
            Grid::new("inspector_container").num_columns(2).show(ui, |ui| {
                ui.label("Slot label");
                let r = ui.text_edit_singleline(&mut clip.container_label);
                g.note_text(&r);
                ui.end_row();

                ui.label("Media");
                ui.horizontal(|ui| {
                    if clip.asset == 0 {
                        ui.weak("(Empty Slot)");
                    } else if let Some(a) = project.asset(clip.asset) {
                        ui.label(a.name());
                    } else {
                        ui.weak("Missing asset");
                    }
                    if ui.small_button("Replace…").clicked() {
                        PENDING_ACTION.with(|p| *p.borrow_mut() = Some(Action::ReplaceContainerMedia));
                    }
                });
                ui.end_row();
            });
        });
        ui.separator();
    }
    ui.strong("Clip");
    Grid::new("inspector_clip").num_columns(2).show(ui, |ui| {
        ui.label("Name");
        ui.add_enabled_ui(!multi, |ui| {
            let r = ui.text_edit_singleline(&mut clip.name);
            #[cfg(test)]
            ui.ctx().data_mut(|d| {
                d.insert_temp(egui::Id::new("test_name_field"), r.id);
                d.insert_temp(egui::Id::new("test_name_field_enabled"), r.enabled());
            });
            g.note_text(&r);
        });
        ui.end_row();
        ui.label("Enabled");
        g.note(&ui.checkbox(&mut clip.enabled, ""));
        ui.end_row();
        ui.label("Label");
        ui.horizontal(|ui| {
            let name = if clip.label == 0 {
                "None".to_string()
            } else {
                labels.get(clip.label as usize - 1).map(|l| l.name.clone()).unwrap_or_else(|| "None".into())
            };
            egui::ComboBox::from_id_salt("clip_label").selected_text(name).show_ui(ui, |ui| {
                g.note(&ui.selectable_value(&mut clip.label, 0, "None"));
                for (i, l) in labels.iter().enumerate() {
                    let [r, gc, b] = l.color;
                    let t = RichText::new(l.name.clone()).color(egui::Color32::from_rgb(r, gc, b));
                    g.note(&ui.selectable_value(&mut clip.label, i as u8 + 1, t));
                }
                ui.separator();
                if ui.selectable_label(false, "Edit labels…").clicked() {
                    edit_labels = true;
                    ui.close();
                }
            });
            if clip.label != 0 {
                if let Some(l) = labels.get(clip.label as usize - 1) {
                    let [r, gc, b] = l.color;
                    crate::ui::tools::glyph_label(ui, crate::ui::tools::Glyph::Dot, egui::Color32::from_rgb(r, gc, b));
                }
            }
        });
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
            ui.label("Speed");
            ui.horizontal(|ui| {
                let mut s = format!("{:.0} %", clip.speed * 100.0);
                if clip.reverse {
                    s.push_str(", reversed");
                }
                if clip.freeze.is_some() {
                    s.push_str(", freeze frame");
                }
                ui.label(s);
                ui.weak("Retime… Ctrl+R");
            });
            ui.end_row();
        }
        for (label, a) in clip.props_mut() {
            ui.label(label);
            let linked = !a.link.is_none();
            ui.horizontal(|ui| {
                let mut v = a.at(lt);
                let r = ui
                    .add_enabled_ui(!linked, |ui| match label {
                        "Volume" => {
                            let mut db = gain_to_db(v);
                            let r = ui.add(Slider::new(&mut db, -60.0..=12.0).suffix(" dB").fixed_decimals(1));
                            if r.changed() {
                                v = db_to_gain(db);
                            }
                            r
                        }
                        "Pan" => {
                            let r = ui.add(Slider::new(&mut v, -1.0..=1.0).fixed_decimals(2));
                            #[cfg(test)]
                            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("test_pan_slider"), r.id));
                            r
                        }
                        "Scale" | "Scale X" | "Scale Y" => {
                            ui.add(DragValue::new(&mut v).speed(0.01).range(0.01..=20.0))
                        }
                        "Opacity" => {
                            let mut pct = v * 100.0;
                            let r = ui.add(Slider::new(&mut pct, 0.0..=100.0).suffix(" %").fixed_decimals(0));
                            if r.changed() {
                                v = pct / 100.0;
                            }
                            #[cfg(test)]
                            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("test_opacity_slider"), r.id));
                            r
                        }
                        _ => ui.add(DragValue::new(&mut v).speed(1.0)),
                    })
                    .inner;
                if r.changed() {
                    a.set_at(lt, v);
                }
                g.note(&r);
                key_buttons(ui, a, lt, palette, &mut g);
                link_menu(ui, label, a, &path_list, &mut g);
            });
            ui.end_row();
            // a linked expression is edited right under its property
            if let Animated { link: AnimLink::Expr(src), link_err, .. } = a {
                ui.label("");
                ui.horizontal(|ui| {
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
                ui.end_row();
            }
        }
        // gain fades on audio, opacity fades on visual clips (same fields, same ramps)
        if clip.kind != ClipKind::Adjustment {
            let dur = clip.duration;
            ui.label("Fade in");
            g.note(&ui.add(DragValue::new(&mut clip.fade_in).range(0.0..=dur).speed(0.05).suffix(" s")));
            ui.end_row();
            ui.label("Fade out");
            g.note(&ui.add(DragValue::new(&mut clip.fade_out).range(0.0..=dur).speed(0.05).suffix(" s")));
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

    // Zone 2: everything below is per-clip data that does not bulk-edit — greyed out and non-interactive
    // while more than one clip is selected, exactly like the Name field above (the labels editor at the
    // very bottom is the one exception: it edits `Project.labels`, not this clip, so it stays live).
    ui.add_enabled_ui(!multi, |ui| {
        if !clip.effects.is_empty() {
            ui.separator();
            ui.strong("Effects");
            let mut rm: Option<usize> = None;
            for (i, e) in clip.effects.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    g.note(&ui.checkbox(&mut e.enabled, e.kind.name()));
                    if crate::ui::markers_ui::x_button(ui).on_hover_text("Remove this effect").clicked() {
                        rm = Some(i);
                    }
                });
            }
            if let Some(i) = rm {
                clip.effects.remove(i);
                g.click();
            }
        }

        let transitions: Vec<String> = project
            .transitions_of(id)
            .iter()
            .map(|(_, tr)| format!("{} · {:.2} s", tr.kind.name(), tr.duration))
            .collect();
        if !transitions.is_empty() {
            ui.separator();
            ui.strong("Transitions");
            for t in &transitions {
                ui.weak(t);
            }
        }

        if clip.kind == ClipKind::Sequence {
            ui.separator();
            ui.strong("Sequence");
            let name = project.sequence(clip.sequence).map(|s| s.name.clone()).unwrap_or_else(|| "(missing)".into());
            ui.horizontal(|ui| {
                ui.label(name);
                if ui.button("Open sequence").clicked() {
                    OPEN_SEQUENCE.with(|p| *p.borrow_mut() = Some(clip.sequence));
                }
            });
        }

        // asset details (description / tags live on the asset, not the clip)
        if clip.uses_asset() {
            if let Some(a) = project.asset(clip.asset) {
                ui.separator();
                ui.strong("Asset");
                ui.add(egui::Label::new(RichText::new(a.name()).weak()).truncate()).on_hover_text(&a.path);
                ui.weak(format!("Used in: {} clips", asset_use_count(project, a.id)));
                // where this asset sits in the proxy pipeline (playback smoothness at 4K depends on it)
                match crate::media::proxy::status(a, settings.use_proxies, settings.proxy_height) {
                    crate::media::proxy::ProxyStatus::Ready => {
                        ui.weak("Proxy: ready (preview plays the low-res proxy)");
                    }
                    crate::media::proxy::ProxyStatus::Building(f) => {
                        ui.weak(format!("Proxy: building — {:.0} %", f * 100.0));
                    }
                    crate::media::proxy::ProxyStatus::Queued => {
                        ui.weak("Proxy: queued (builds run one at a time)");
                    }
                    crate::media::proxy::ProxyStatus::NotNeeded => {}
                }
                let mut desc = a.description.clone();
                let r = ui.add(egui::TextEdit::multiline(&mut desc).desired_rows(2).hint_text("description"));
                if r.changed() {
                    asset_desc = Some(desc);
                }
                ga.note_text(&r);
                // The raw text is kept so a comma survives typing; it is stored with the tags it produced and
                // dropped again as soon as the asset's tags were changed elsewhere (library details box).
                let tags_id = egui::Id::new(("asset_tags", a.id));
                let mut buf = ui
                    .ctx()
                    .data_mut(|d| d.get_temp::<(String, Vec<String>)>(tags_id))
                    .filter(|(_, src)| *src == a.tags)
                    .map(|(b, _)| b)
                    .unwrap_or_else(|| a.tags.join(", "));
                let r = ui.add(egui::TextEdit::singleline(&mut buf).hint_text("tags, comma, separated"));
                if r.changed() {
                    let tags: Vec<String> =
                        buf.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect();
                    ui.ctx().data_mut(|d| d.insert_temp(tags_id, (buf, tags.clone())));
                    asset_tags = Some(tags);
                }
                ga.note_text(&r);
            }
        }

        if clip.kind == ClipKind::Text {
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
                            PENDING_FONT.with(|f| *f.borrow_mut() = Some(p.to_string_lossy().into_owned()));
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

            // ---- per-selection style override (TextSpan) + saved text-style presets ----
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
                egui::Window::new("Set Text Style").resizable(false).collapsible(false).open(&mut open).show(
                    ui.ctx(),
                    |ui| {
                        text_preset_fields(ui, &mut preset, fonts);
                        ui.horizontal(|ui| {
                            if ui.button("Apply").clicked() {
                                apply = true;
                            }
                            if ui.button("Cancel").clicked() {
                                cancel = true;
                            }
                        });
                    },
                );
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

            let presets_open_id = egui::Id::new("inspector_text_presets_open");
            let mut presets_open: bool = ui.ctx().data(|d| d.get_temp(presets_open_id).unwrap_or(false));
            ui.checkbox(&mut presets_open, "Text style presets");
            ui.ctx().data_mut(|d| d.insert_temp(presets_open_id, presets_open));
            if presets_open {
                ui.horizontal(|ui| {
                    let has_sel = matches!(text_sel, Some((a, b)) if b > a);
                    let label =
                        if has_sel { "Save selection's style as preset" } else { "Save current style as preset" };
                    if ui.button(label).on_hover_text("Rename it in the list below").clicked() {
                        let name = format!("Text style {}", settings.text_presets.len() + 1);
                        // with a selection, capture its EFFECTIVE style (span overrides included) — the
                        // "I styled this word, save that look" workflow; otherwise the clip style
                        let mut p = match text_sel {
                            Some((a, b)) if b > a => span_draft_at(style, a, b),
                            _ => span_draft_at(style, 0, 0),
                        };
                        p.name = name;
                        settings.text_presets.push(p);
                        settings.save();
                    }
                });
                let mut delete: Option<usize> = None;
                let mut renamed = false;
                let mut apply_preset: Option<(TextPreset, bool)> = None; // (preset, to the whole clip)
                let armed_id = egui::Id::new("inspector_text_preset_delete_armed");
                let mut armed: Option<usize> = ui.ctx().data(|d| d.get_temp(armed_id)).flatten();
                for (i, p) in settings.text_presets.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        let r = ui.add(egui::TextEdit::singleline(&mut p.name).desired_width(110.0));
                        // renames used to vanish on quit: nothing saved settings after editing the name
                        if r.lost_focus() {
                            renamed = true;
                        }
                        let can = matches!(text_sel, Some((a, b)) if b > a);
                        if ui.add_enabled(can, egui::Button::new("Apply to Selection")).clicked() {
                            apply_preset = Some((p.clone(), false));
                        }
                        if ui.button("Apply to Clip").clicked() {
                            apply_preset = Some((p.clone(), true));
                        }
                        if ui.small_button("Export…").clicked() {
                            if let Some(out) = rfd::FileDialog::new()
                                .add_filter("Simple Editor text style", &["sedit-textstyle"])
                                .set_file_name(format!("{}.sedit-textstyle", p.name))
                                .save_file()
                            {
                                let _ = std::fs::write(&out, serde_json::to_string_pretty(p).unwrap_or_default());
                            }
                        }
                        // presets live outside undo — two-click delete (Shift+click skips the confirm)
                        if armed == Some(i) {
                            if ui.small_button("Sure?").clicked() {
                                delete = Some(i);
                                armed = None;
                            }
                        } else if x_button(ui).on_hover_text("Delete preset (Shift+click: no confirm)").clicked() {
                            if ui.input(|inp| inp.modifiers.shift) {
                                delete = Some(i);
                            } else {
                                armed = Some(i);
                            }
                        }
                    });
                }
                ui.ctx().data_mut(|d| d.insert_temp(armed_id, armed));
                if ui.button("Import…").clicked() {
                    if let Some(p) = rfd::FileDialog::new()
                        .add_filter("Simple Editor text style", &["sedit-textstyle", "json"])
                        .pick_file()
                    {
                        if let Ok(json) = std::fs::read_to_string(&p) {
                            if let Ok(preset) = serde_json::from_str::<TextPreset>(&json) {
                                settings.text_presets.retain(|x| x.name != preset.name);
                                settings.text_presets.push(preset);
                                settings.save();
                            }
                        }
                    }
                }
                if let Some(i) = delete {
                    settings.text_presets.remove(i);
                    settings.save();
                }
                if renamed {
                    settings.save();
                }
                match apply_preset {
                    Some((p, true)) => {
                        undo(project);
                        style.font = p.font.clone();
                        style.size = p.size;
                        style.bold = p.bold;
                        style.italic = p.italic;
                        style.color = p.color;
                        style.letter_spacing = p.letter_spacing;
                        g.changed = true;
                    }
                    Some((p, false)) => {
                        if let Some((a, b)) = text_sel {
                            undo(project);
                            set_span(style, a, b, &p);
                            g.changed = true;
                        }
                    }
                    None => {}
                }
            }
        }

        // ---------- round 3 sections ----------
        if clip.kind == ClipKind::Adjustment {
            ui.separator();
            ui.weak("Adjustment layer — its effects apply to everything below it on the timeline.");
        }

        // node graph
        ui.separator();
        ui.horizontal(|ui| {
            ui.strong("Nodes");
            let has = clip.graph.is_some();
            let r = ui.button("Open node editor").on_hover_text(if has {
                "Edit the node graph"
            } else {
                "Convert this clip's effect stack to nodes"
            });
            mark(ui, "open_nodes", &r);
            if r.clicked() {
                OPEN_NODES.with(|p| *p.borrow_mut() = Some(id));
            }
            if has {
                let n = clip.graph.as_ref().map(|gr| gr.nodes.len()).unwrap_or(0);
                ui.weak(format!("{n} nodes"));
                let r = ui.button("Unlink").on_hover_text("Back to a plain effect list (a simple chain only)");
                mark(ui, "unlink_nodes", &r);
                if r.clicked() {
                    UNLINK_NODES.with(|p| *p.borrow_mut() = Some(id));
                }
            }
        });

        // mask — a mask shapes pixels, so an audio clip gets no mask UI at all (not even a dead button)
        if clip.is_visual() {
            ui.separator();
            ui.horizontal(|ui| {
                ui.strong("Mask");
                if clip.mask.is_none() {
                    let r = ui.button("Add mask");
                    mark(ui, "add_mask", &r);
                    if r.clicked() {
                        clip.mask = Some(Mask::default());
                        g.click();
                    }
                } else {
                    let r = ui.button("Edit in viewport").on_hover_text("Drag the mask over the preview");
                    mark(ui, "edit_mask", &r);
                    if r.clicked() {
                        EDIT_MASK.with(|p| *p.borrow_mut() = Some(id));
                    }
                    if x_button(ui).on_hover_text("Remove mask").clicked() {
                        clip.mask = None;
                        g.click();
                    }
                }
            });
            if let Some(m) = &mut clip.mask {
                mask_grid(ui, m, lt, palette, &mut g, egui::Id::new("inspector_mask"));
            }
        }

        // shape style
        if let Some(sh) = &mut clip.shape {
            ui.separator();
            ui.strong("Shape");
            Grid::new("inspector_shape").num_columns(2).show(ui, |ui| {
                ui.label("Kind");
                egui::ComboBox::from_id_salt("shape_kind").selected_text(sh.kind.name()).show_ui(ui, |ui| {
                    for k in ShapeKind::ALL {
                        g.note(&ui.selectable_value(&mut sh.kind, k, k.name()));
                    }
                });
                ui.end_row();
                ui.label("Fill");
                g.note(&ui.color_edit_button_srgba_unmultiplied(&mut sh.fill));
                ui.end_row();
                ui.label("Stroke");
                ui.horizontal(|ui| {
                    g.note(&ui.color_edit_button_srgba_unmultiplied(&mut sh.stroke));
                    g.note(&ui.add(DragValue::new(&mut sh.stroke_width).range(0.0..=200.0).speed(0.2)));
                });
                ui.end_row();
                ui.label("Sides");
                g.note(&ui.add(DragValue::new(&mut sh.sides).range(3..=64)));
                ui.end_row();
                ui.label("Corner");
                g.note(&ui.add(DragValue::new(&mut sh.corner).range(0.0..=500.0).speed(0.5)));
                ui.end_row();
                for label in ["Width", "Height"] {
                    ui.label(label);
                    ui.horizontal(|ui| {
                        let a: &mut Animated = if label == "Width" { &mut sh.w } else { &mut sh.h };
                        let mut v = a.at(lt);
                        let r = ui.add(DragValue::new(&mut v).range(1.0..=20000.0).speed(1.0));
                        if r.changed() {
                            a.set_at(lt, v);
                        }
                        g.note(&r);
                        key_buttons(ui, a, lt, palette, &mut g);
                    });
                    ui.end_row();
                }
                if sh.kind == ShapeKind::Draw {
                    ui.label("Draw rate");
                    ui.horizontal(|ui| {
                        g.note(&ui.add(DragValue::new(&mut sh.draw_rate).range(0.0..=8.0).speed(0.05)));
                        ui.weak(format!("{} strokes · {:.1} s", sh.strokes.len(), sh.draw_duration()));
                    });
                    ui.end_row();
                    ui.label("Page");
                    g.note(&ui.color_edit_button_srgba_unmultiplied(&mut sh.page));
                    ui.end_row();
                }
            });
        } else if clip.kind == ClipKind::Shape {
            ui.separator();
            if ui.button("Add shape style").clicked() {
                clip.shape = Some(ShapeStyle::default());
                g.click();
            }
        }

        // reusable paths: a drawing or a polygon outline is saved on the project, and any clip can then
        // travel along one (its X/Y become keyframes over the clip's own length)
        // ponytail: only the cheap "is there an outline" test runs per frame; the points are copied when
        // Save is actually clicked
        let outline = clip.shape.as_ref().is_some_and(|s| !s.strokes.is_empty() || s.points.len() >= 2);
        let paths: Vec<(Id, String)> = project.paths.iter().map(|p| (p.id, p.name.clone())).collect();
        if outline || !paths.is_empty() {
            ui.separator();
            ui.horizontal(|ui| {
                ui.strong("Path");
                if outline {
                    let r =
                        ui.small_button("Save").on_hover_text("Keep this outline in the project as a reusable path");
                    mark(ui, "save_path", &r);
                    if r.clicked() {
                        path_op = Some(PathOp::Save);
                    }
                }
                if !paths.is_empty() {
                    egui::ComboBox::from_id_salt("clip_path").selected_text("Animate X/Y").width(130.0).show_ui(
                        ui,
                        |ui| {
                            for (pid, name) in &paths {
                                if ui.selectable_label(false, name).clicked() {
                                    path_op = Some(PathOp::Apply(*pid));
                                }
                            }
                        },
                    );
                }
            });
        }

        // audio bus override
        if clip.kind == ClipKind::Audio || clip.kind == ClipKind::Video {
            ui.separator();
            ui.horizontal(|ui| {
                ui.strong("Bus");
                let name = buses
                    .iter()
                    .find(|(bid, _)| *bid == clip.bus)
                    .map(|(_, n)| n.clone())
                    .unwrap_or_else(|| "Track default".into());
                egui::ComboBox::from_id_salt("clip_bus").selected_text(name).width(140.0).show_ui(ui, |ui| {
                    g.note(&ui.selectable_value(&mut clip.bus, 0, "Track default"));
                    for (bid, n) in &buses {
                        g.note(&ui.selectable_value(&mut clip.bus, *bid, n));
                    }
                });
            });
        }

        // clip markers
        ui.separator();
        ui.horizontal(|ui| {
            ui.strong("Markers");
            let r = ui.small_button("+ at playhead");
            mark(ui, "add_marker", &r);
            if r.clicked() {
                let t = lt.clamp(0.0, clip.duration);
                let mid = project.new_id();
                clip.markers.push(crate::model::Marker { id: mid, t, ..Default::default() });
                clip.markers.sort_by(|a, b| a.t.total_cmp(&b.t));
                g.click();
            }
        });
        let mut rm_marker: Option<usize> = None;
        for (i, m) in clip.markers.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                let r = ui.add(DragValue::new(&mut m.t).range(0.0..=1e6).speed(0.05).suffix(" s").fixed_decimals(2));
                g.note(&r);
                let w = (ui.available_width() - 30.0).max(50.0);
                let r = ui.add(egui::TextEdit::singleline(&mut m.name).desired_width(w).hint_text("marker"));
                g.note_text(&r);
                if x_button(ui).on_hover_text("Delete marker").clicked() {
                    rm_marker = Some(i);
                }
            });
        }
        if let Some(i) = rm_marker {
            clip.markers.remove(i);
            g.click();
        }
        if clip.markers.is_empty() {
            ui.weak("No clip markers");
        }
    }); // end zone 2 (add_enabled_ui)

    // compact labels editor (labels live on the project, not the clip) — project-wide, so it stays
    // interactive regardless of how many clips are selected (see the doc comment above zone 2).
    if edit_labels {
        ui.separator();
        ui.horizontal(|ui| {
            ui.strong("Labels");
            if ui.small_button("Add").clicked() {
                label_ops.push(LabelOp::Add);
            }
            if ui.small_button("Done").clicked() {
                edit_labels = false;
            }
        });
        for (i, l) in labels.iter().enumerate() {
            ui.horizontal(|ui| {
                let mut color = l.color;
                if ui.color_edit_button_srgb(&mut color).changed() {
                    label_ops.push(LabelOp::Color(i, color));
                }
                let mut name = l.name.clone();
                let w = (ui.available_width() - 30.0).max(50.0);
                if ui.add(egui::TextEdit::singleline(&mut name).desired_width(w)).changed() {
                    label_ops.push(LabelOp::Rename(i, name));
                }
                if x_button(ui).on_hover_text("Remove label").clicked() {
                    label_ops.push(LabelOp::Remove(i));
                }
            });
        }
    }
    ui.ctx().data_mut(|d| d.insert_temp(labels_open_id, edit_labels));

    if g.start || ga.start || !label_ops.is_empty() || path_op.is_some() {
        undo(project);
    }
    if g.changed {
        if let Some(c) = project.clip_mut(id) {
            *c = clip.clone();
        }
        // Bulk propagation: only the zone-1 fields, only when a field actually changed from `orig`, and
        // always an absolute overwrite of the sibling's own value — never a relative delta (the same rule
        // `transition_section` uses). `orig`/`clip`/`clip_ids` are owned values by this point, so fetching
        // `project.clip_mut(sibling)` in the loop below borrows nothing that is still borrowed.
        if multi {
            // Diff each props_mut() field position-wise: `clip` and `orig` share a kind (it never changes
            // in this section), so `props_mut()` yields the same labels in the same order for both.
            let mut orig_probe = orig.clone();
            let mut changed_props: Vec<(&'static str, f64)> = Vec::new();
            for ((label, a), (_, oa)) in clip.props_mut().into_iter().zip(orig_probe.props_mut()) {
                let v = a.at(lt);
                if v != oa.at(lt) {
                    changed_props.push((label, v));
                }
            }
            for &sid in &clip_ids {
                if sid == id {
                    continue;
                }
                let Some(s) = project.clip_mut(sid) else { continue };
                if clip.enabled != orig.enabled {
                    s.enabled = clip.enabled;
                }
                if clip.label != orig.label {
                    s.label = clip.label;
                }
                if clip.blend != orig.blend && s.is_visual() {
                    s.blend = clip.blend;
                }
                if clip.fade_in != orig.fade_in {
                    s.fade_in = clip.fade_in.clamp(0.0, s.duration);
                }
                if clip.fade_out != orig.fade_out {
                    s.fade_out = clip.fade_out.clamp(0.0, s.duration);
                }
                if !changed_props.is_empty() {
                    // clamp into the sibling: an unclamped local time wrote keyframes OUTSIDE the
                    // sibling's 0..duration (invisible in every editor, but steering its value by
                    // extrapolation) whenever the playhead wasn't over that sibling
                    let slt = s.local(playhead).clamp(0.0, s.duration);
                    for (label, v) in &changed_props {
                        for (slabel, sa) in s.props_mut() {
                            // a linked property (expression / path) is skipped, mirroring how the
                            // representative's own row is disabled while linked — set_at would
                            // silently sever the sibling's link otherwise
                            if slabel == *label && sa.link.is_none() {
                                sa.set_at(slt, *v);
                            }
                        }
                    }
                }
            }
        }
    }
    if ga.changed {
        if let Some(a) = project.asset_mut(clip.asset) {
            if let Some(d) = asset_desc {
                a.description = d;
            }
            if let Some(t) = asset_tags {
                a.tags = t;
            }
        }
    }
    let labels_changed = !label_ops.is_empty();
    for op in label_ops {
        match op {
            LabelOp::Add => {
                project.add_label(format!("Label {}", project.labels.len() + 1), [160, 160, 160]);
            }
            LabelOp::Rename(i, name) => {
                if let Some(l) = project.labels.get_mut(i) {
                    l.name = name;
                }
            }
            LabelOp::Color(i, c) => {
                if let Some(l) = project.labels.get_mut(i) {
                    l.color = c;
                }
            }
            // remove_label() also re-points every clip / asset / marker using it
            LabelOp::Remove(i) => project.remove_label(i as u8 + 1),
        }
    }
    // after the write-back: applying a path overwrites the X/Y the clone still held
    match path_op {
        Some(PathOp::Save) => {
            let pts = project.path_from_clip(id);
            project.add_path(clip.name.clone(), pts);
        }
        Some(PathOp::Apply(pid)) => {
            project.link_path(id, pid);
        }
        None => {}
    }
    g.changed || ga.changed || labels_changed || path_op.is_some()
}

const LUAU_KEYWORDS: &[&str] = &[
    "and", "break", "continue", "do", "else", "elseif", "end", "export", "false", "for", "function", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "type", "until", "while",
];

/// Minimal hand-rolled Luau tokenizer for the expression field — keywords, strings, numbers and
/// comments get a colour, everything else stays the default text colour. One expression at a time,
/// so a full syntax-highlighting crate would be a lot of dependency for one line of text.
fn luau_highlight(ui: &egui::Ui, text: &str) -> egui::text::LayoutJob {
    use egui::text::{LayoutJob, TextFormat};
    use egui::{Color32, FontId};

    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let base = ui.visuals().text_color();
    let (keyword, string, number, comment) = (
        ui.visuals().hyperlink_color,
        Color32::from_rgb(0x9a, 0xc5, 0x6b),
        ui.visuals().warn_fg_color,
        base.gamma_multiply(0.6),
    );

    let mut job = LayoutJob::default();
    let mut push = |s: &str, color: Color32, font: &FontId| {
        job.append(s, 0.0, TextFormat { font_id: font.clone(), color, ..Default::default() });
    };

    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '-' && chars.get(i + 1) == Some(&'-') {
            let s: String = chars[i..].iter().collect();
            push(&s, comment, &font);
            break;
        } else if c == '"' || c == '\'' {
            let start = i;
            i += 1;
            while i < chars.len() && chars[i] != c {
                i += if chars[i] == '\\' { 2 } else { 1 };
            }
            i = (i + 1).min(chars.len());
            let s: String = chars[start..i].iter().collect();
            push(&s, string, &font);
            continue;
        } else if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '.') {
                i += 1;
            }
            let s: String = chars[start..i].iter().collect();
            push(&s, number, &font);
            continue;
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let s: String = chars[start..i].iter().collect();
            push(&s, if LUAU_KEYWORDS.contains(&s.as_str()) { keyword } else { base }, &font);
            continue;
        } else {
            push(&c.to_string(), base, &font);
            i += 1;
        }
    }
    job
}

/// The "∿" menu on a property row: link the value to a saved path (Position X/Y) or a Luau
/// expression, AE-style. Manual edits elsewhere break the link (`Animated::set_at`).
fn link_menu(ui: &mut egui::Ui, label: &str, a: &mut Animated, paths: &[(Id, String)], g: &mut Gesture) {
    let lit = !a.link.is_none();
    let r = ui.menu_button(if lit { RichText::new("∿").strong() } else { RichText::new("∿").weak() }, |ui| {
        if lit {
            if ui.button("Unlink").clicked() {
                a.unlink();
                g.click();
                ui.close();
            }
            ui.separator();
        }
        let axis_x = label == "Position X";
        if axis_x || label == "Position Y" {
            for (pid, name) in paths {
                if ui.button(format!("Path: {name}")).clicked() {
                    a.unlink();
                    a.link = if axis_x { AnimLink::PathX(*pid) } else { AnimLink::PathY(*pid) };
                    g.click();
                    ui.close();
                }
            }
            if !paths.is_empty() {
                ui.separator();
            }
        }
        if !matches!(a.link, AnimLink::Expr(_)) && ui.button("Expression…").clicked() {
            a.unlink();
            a.link = AnimLink::Expr("return value".into());
            g.click();
            ui.close();
        }
    });
    r.response.on_hover_text(match &a.link {
        AnimLink::None => "Link to a path or expression".to_string(),
        AnimLink::PathX(_) => "Following a path (X)".to_string(),
        AnimLink::PathY(_) => "Following a path (Y)".to_string(),
        AnimLink::Expr(_) => "Driven by an expression".to_string(),
    });
}

/// Save the clip's outline as a project path, or animate it along one that was saved.
enum PathOp {
    Save,
    Apply(Id),
}

/// Edits to `Project.labels` collected during the frame (applied after the clip write-back).
enum LabelOp {
    Add,
    Rename(usize, String),
    Color(usize, [u8; 3]),
    Remove(usize),
}

/// How many clips (main timeline, stash, every sequence) use the asset.
fn asset_use_count(project: &Project, aid: Id) -> usize {
    let count = |tracks: &[crate::model::Track]| {
        tracks.iter().flat_map(|t| t.clips.iter()).filter(|c| c.uses_asset() && c.asset == aid).count()
    };
    let mut n = count(&project.tracks);
    if let Some(st) = &project.main_stash {
        n += count(&st.tracks);
    }
    for s in &project.sequences {
        n += count(&s.tracks);
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Asset, Clip};

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

    #[test]
    fn use_count_and_labels() {
        let a = Asset {
            id: 0,
            path: r"C:\m\a.mp4".into(),
            kind: ClipKind::Video,
            duration: 10.0,
            width: 1280,
            height: 720,
            fps: 30.0,
            audio_streams: Vec::new(),
            codec: String::new(),
            folder: String::new(),
            tags: Vec::new(),
            label: 0,
            description: String::new(),
        };
        let mut p = Project::from_media(a);
        let aid = p.assets[0].id;
        assert_eq!(asset_use_count(&p, aid), 1);
        p.split_at(4.0, None);
        assert_eq!(asset_use_count(&p, aid), 2);
        assert_eq!(crate::ui::label_name(0), "None");
        assert_eq!(crate::ui::label_name(1), "Red");
        assert_eq!(crate::ui::label_name(200), "None");
    }

    /// A keyboard nudge on the Pan slider of an audio clip pushes exactly one undo and moves the pan.
    #[test]
    fn pan_edit_pushes_one_undo() {
        let mut p = Project::new();
        let ai = p.tracks.iter().position(|t| t.kind == crate::model::TrackKind::Audio).unwrap();
        let c = Clip::new(500, ClipKind::Audio, "a", 0.0, 5.0);
        let id = c.id;
        p.tracks[ai].clips.push(c);
        let palette = Palette::new(true, egui::Color32::WHITE);
        let fonts: Vec<String> = Vec::new();
        let ctx = egui::Context::default();
        let mut undos = 0;
        let mut run = |ctx: &egui::Context, input: egui::RawInput, p: &mut Project, undos: &mut usize| {
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut undo = |pre: &Project| {
                        *undos += 1;
                        assert_eq!(pre.clip(id).unwrap().pan.value, 0.0, "undo sees the pre-edit project");
                    };
                    show(ui, p, &[id], &[], 1.0, &fonts, &palette, &mut Settings::default(), &mut undo);
                });
            });
        };
        run(&ctx, egui::RawInput::default(), &mut p, &mut undos); // layout, records the pan slider id
        let slider = ctx.data_mut(|d| d.get_temp::<egui::Id>(egui::Id::new("test_pan_slider"))).expect("pan id");
        ctx.memory_mut(|m| m.request_focus(slider));
        run(&ctx, egui::RawInput::default(), &mut p, &mut undos); // focus settles
        assert_eq!(undos, 0);
        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Key {
            key: egui::Key::ArrowRight,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        });
        run(&ctx, input, &mut p, &mut undos);
        assert_eq!(undos, 1, "exactly one undo per edit gesture");
        assert!(p.clip(id).unwrap().pan.value > 0.0, "pan moved right");
        run(&ctx, egui::RawInput::default(), &mut p, &mut undos);
        assert_eq!(undos, 1, "no undo without an edit");
    }

    /// The Opacity control is a 0-100% slider that writes straight into `clip.opacity` — the same
    /// `Animated` field the renderer reads via `props_mut()` — not a second, disconnected property.
    #[test]
    fn opacity_slider_drives_the_animated_field() {
        let mut p = Project::new();
        let vi = p.tracks.iter().position(|t| t.kind == crate::model::TrackKind::Video).unwrap();
        let c = Clip::new(500, ClipKind::Video, "v", 0.0, 5.0);
        let id = c.id;
        p.tracks[vi].clips.push(c);
        assert_eq!(p.clip(id).unwrap().opacity.value, 1.0, "clips start fully opaque");
        let palette = Palette::new(true, egui::Color32::WHITE);
        let fonts: Vec<String> = Vec::new();
        let ctx = egui::Context::default();
        let mut run = |ctx: &egui::Context, input: egui::RawInput, p: &mut Project| {
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut undo = |_: &Project| {};
                    show(ui, p, &[id], &[], 1.0, &fonts, &palette, &mut Settings::default(), &mut undo);
                });
            });
        };
        run(&ctx, egui::RawInput::default(), &mut p); // layout, records the opacity slider id
        let slider =
            ctx.data_mut(|d| d.get_temp::<egui::Id>(egui::Id::new("test_opacity_slider"))).expect("opacity id");
        ctx.memory_mut(|m| m.request_focus(slider));
        run(&ctx, egui::RawInput::default(), &mut p); // focus settles
        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Key {
            key: egui::Key::ArrowLeft,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        });
        run(&ctx, input, &mut p);
        let o = p.clip(id).unwrap().opacity.value;
        assert!(o < 1.0 && o >= 0.0, "opacity moved down off its default 1.0: {o}");
    }

    /// With 2+ clips selected, dragging the representative clip's Opacity slider propagates the new
    /// ABSOLUTE value to every selected clip — even one whose opacity started at a different value than
    /// the representative's (diff-against-original, absolute-overwrite: the same rule `transition_section`
    /// uses for bulk-editing transitions, not a relative delta).
    #[test]
    fn opacity_bulk_edit_propagates_absolute_value_to_every_selected_clip() {
        let mut p = Project::new();
        let vi = p.tracks.iter().position(|t| t.kind == crate::model::TrackKind::Video).unwrap();
        let a = Clip::new(500, ClipKind::Video, "a", 0.0, 5.0);
        let id_a = a.id;
        p.tracks[vi].clips.push(a);
        let mut b = Clip::new(501, ClipKind::Video, "b", 0.0, 5.0);
        b.opacity.value = 0.4; // starts at a different value than `a`'s default 1.0
        let id_b = b.id;
        p.tracks[vi].clips.push(b);
        let palette = Palette::new(true, egui::Color32::WHITE);
        let fonts: Vec<String> = Vec::new();
        let ctx = egui::Context::default();
        let mut run = |ctx: &egui::Context, input: egui::RawInput, p: &mut Project| {
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut undo = |_: &Project| {};
                    show(ui, p, &[id_a, id_b], &[], 1.0, &fonts, &palette, &mut Settings::default(), &mut undo);
                });
            });
        };
        run(&ctx, egui::RawInput::default(), &mut p); // layout, records `a`'s opacity slider id
        let slider =
            ctx.data_mut(|d| d.get_temp::<egui::Id>(egui::Id::new("test_opacity_slider"))).expect("opacity id");
        ctx.memory_mut(|m| m.request_focus(slider));
        run(&ctx, egui::RawInput::default(), &mut p); // focus settles
        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Key {
            key: egui::Key::ArrowLeft,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        });
        run(&ctx, input, &mut p);
        let oa = p.clip(id_a).unwrap().opacity.value;
        let ob = p.clip(id_b).unwrap().opacity.value;
        assert!(oa < 1.0 && oa >= 0.0, "the representative clip's opacity moved down: {oa}");
        assert_eq!(oa, ob, "the new absolute value propagated to the other selected clip, not a relative delta");
    }

    /// With 2+ clips selected, the Name field (zone 1, but not bulk-editable) is disabled — it needs a
    /// single-clip selection, unlike the transform/opacity/enabled/label/fade/blend fields around it.
    #[test]
    fn name_field_disabled_when_multiple_clips_selected() {
        let mut p = Project::new();
        let a = Clip::new(500, ClipKind::Text, "a", 0.0, 5.0);
        let id_a = a.id;
        p.tracks[0].clips.push(a);
        let b = Clip::new(501, ClipKind::Text, "b", 0.0, 5.0);
        let id_b = b.id;
        p.tracks[0].clips.push(b);
        let palette = Palette::new(true, egui::Color32::WHITE);
        let ctx = egui::Context::default();
        let mut settings = Settings::default();
        let mut undo = |_: &Project| {};
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                show(ui, &mut p, &[id_a, id_b], &[], 1.0, &[], &palette, &mut settings, &mut undo);
            });
        });
        let enabled = ctx
            .data_mut(|d| d.get_temp::<bool>(egui::Id::new("test_name_field_enabled")))
            .expect("name field enabled-flag not recorded");
        assert!(!enabled, "Name is disabled while multiple clips are selected");
        // sanity: narrowing back to one clip re-enables it
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                show(ui, &mut p, &[id_a], &[], 1.0, &[], &palette, &mut settings, &mut undo);
            });
        });
        let enabled = ctx
            .data_mut(|d| d.get_temp::<bool>(egui::Id::new("test_name_field_enabled")))
            .expect("name field enabled-flag not recorded");
        assert!(enabled, "Name is enabled again once only one clip is selected");
    }

    /// With 2+ clips selected, a zone-2 control (here: "Add mask", per-clip data that does not bulk-edit)
    /// is disabled — the click lands on the widget rect but registers no edit at all.
    #[test]
    fn zone2_controls_disabled_when_multiple_clips_selected() {
        let mut p = Project::new();
        let a = p.add_shape_clip(crate::model::ShapeKind::Star, 0.0, 3.0);
        let b = p.add_shape_clip(crate::model::ShapeKind::Star, 3.0, 3.0);
        let palette = Palette::new(true, egui::Color32::WHITE);
        let ctx = egui::Context::default();
        let mut settings = Settings::default();
        let mut undos = 0;
        let mut frame = |events: Vec<egui::Event>, p: &mut Project, undos: &mut usize| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(420.0, 1400.0))),
                events,
                ..Default::default()
            };
            let mut changed = false;
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut undo = |_: &Project| *undos += 1;
                    changed |= show(ui, p, &[a, b], &[], 1.0, &[], &palette, &mut settings, &mut undo);
                });
            });
            changed
        };
        frame(vec![], &mut p, &mut undos); // layout, records the (disabled) "Add mask" rect
        let r = ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("insp", "add_mask".to_string()))))
            .expect("add_mask rect recorded even while disabled");
        let pos = r.center();
        let mut changed = frame(vec![egui::Event::PointerMoved(pos)], &mut p, &mut undos);
        changed |= frame(
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
            &mut p,
            &mut undos,
        );
        changed |= frame(
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
            &mut p,
            &mut undos,
        );
        assert!(!changed, "a disabled control must not register a click");
        assert!(p.clip(a).unwrap().mask.is_none(), "Add mask did not fire while multiple clips were selected");
        assert_eq!(undos, 0);
    }

    /// The project panel's Save / Export… / Export Frame… buttons push the same `Action`s the menus and
    /// hotkeys use, through the `take_pending_action` hand-off.
    #[test]
    fn project_buttons_push_the_menu_actions() {
        let cases = [
            ("project_save", Action::Save),
            ("project_export", Action::ExportVideo),
            ("project_export_frame", Action::ExportFrame),
        ];
        for (button, expect) in cases {
            let mut p = Project::new();
            let palette = Palette::new(true, egui::Color32::WHITE);
            let fonts: Vec<String> = Vec::new();
            let mut settings = Settings::default();
            let ctx = egui::Context::default();
            let mut frame = |events: Vec<egui::Event>, p: &mut Project| {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(420.0, 900.0))),
                    events,
                    ..Default::default()
                };
                let mut undo = |_: &Project| {};
                let _ = ctx.run(input, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        show(ui, p, &[], &[], 1.0, &fonts, &palette, &mut settings, &mut undo);
                    });
                });
            };
            let _ = take_pending_action(); // clear any leftover from a previous case
            frame(vec![], &mut p); // layout, records the button rect
            let r = ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("insp", button.to_string()))))
                .unwrap_or_else(|| panic!("no widget rect for {button}"));
            let pos = r.center();
            frame(vec![egui::Event::PointerMoved(pos)], &mut p);
            frame(
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                }],
                &mut p,
            );
            frame(
                vec![egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                &mut p,
            );
            assert_eq!(take_pending_action(), Some(expect), "{button}");
        }
    }

    /// Changing the project's Width via its DragValue opens the non-blocking "Resize existing media?"
    /// prompt whenever the project has a clip with a native size; "Rescale" applies Fit to Screen to it,
    /// "Keep As Is" leaves its transform untouched.
    #[test]
    fn resolution_change_offers_rescale_prompt() {
        fn project_with_video_clip() -> (Project, Id) {
            let mut p = Project::new(); // 1920x1080
            let aid = p.add_asset(Asset {
                id: 0,
                path: "C:/v.mp4".into(),
                kind: ClipKind::Video,
                duration: 5.0,
                width: 1280,
                height: 720,
                fps: 30.0,
                audio_streams: Vec::new(),
                codec: String::new(),
                folder: String::new(),
                tags: Vec::new(),
                label: 0,
                description: String::new(),
            });
            let vi = p.tracks.iter().position(|t| t.kind == crate::model::TrackKind::Video).unwrap();
            let mut c = Clip::new(500, ClipKind::Video, "v", 0.0, 5.0);
            c.asset = aid;
            c.x.value = 42.0; // pre-existing transform: "Keep As Is" must leave this alone
            let id = c.id;
            p.tracks[vi].clips.push(c);
            (p, id)
        }

        // drives the Width DragValue, then clicks whichever prompt button `rescale` names; returns the
        // project afterwards.
        let run = |rescale: bool| -> Project {
            let (mut p, _id) = project_with_video_clip();
            let palette = Palette::new(true, egui::Color32::WHITE);
            let fonts: Vec<String> = Vec::new();
            let ctx = egui::Context::default();
            let mut settings = Settings::default();
            let mut frame = |events: Vec<egui::Event>, p: &mut Project| {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(420.0, 900.0))),
                    events,
                    ..Default::default()
                };
                let mut undo = |_: &Project| {};
                let _ = ctx.run(input, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        show(ui, p, &[], &[], 1.0, &fonts, &palette, &mut settings, &mut undo);
                    });
                });
            };
            frame(vec![], &mut p); // layout, records the Width DragValue rect
            let wr = ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("insp", "project_width".to_string()))))
                .expect("width rect recorded");
            let (from, to) = (wr.center(), wr.center() + egui::vec2(60.0, 0.0));
            // drag the DragValue right, in a few steps like a real pointer move
            frame(vec![egui::Event::PointerMoved(from)], &mut p);
            frame(
                vec![egui::Event::PointerButton {
                    pos: from,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                }],
                &mut p,
            );
            for i in 1..=4 {
                let pos = from + (to - from) * (i as f32 / 4.0);
                frame(vec![egui::Event::PointerMoved(pos)], &mut p);
            }
            frame(
                vec![egui::Event::PointerButton {
                    pos: to,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                &mut p,
            );
            frame(vec![], &mut p);
            assert_ne!(p.width, 1920, "dragging the Width control must change it");
            let prompt_open =
                ctx.data(|d| d.get_temp::<bool>(egui::Id::new("inspector_rescale_prompt"))).unwrap_or(false);
            assert!(prompt_open, "a resolution change with native-size media must open the rescale prompt");

            let btn = if rescale { "rescale_prompt_rescale" } else { "rescale_prompt_keep" };
            let br = ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("insp", btn.to_string()))))
                .unwrap_or_else(|| panic!("no rect recorded for {btn}"));
            let bp = br.center();
            frame(vec![egui::Event::PointerMoved(bp)], &mut p);
            frame(
                vec![egui::Event::PointerButton {
                    pos: bp,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                }],
                &mut p,
            );
            frame(
                vec![egui::Event::PointerButton {
                    pos: bp,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                &mut p,
            );
            p
        };

        let rescaled = run(true);
        let c = rescaled.clip(500).unwrap();
        assert_eq!(c.x.value, 0.0, "Rescale resets the transform (Fit to Screen)");
        assert_eq!((c.scale_x.value, c.scale_y.value), (1.0, 1.0));

        let kept = run(false);
        let c = kept.clip(500).unwrap();
        assert_eq!(c.x.value, 42.0, "Keep As Is must leave the clip's transform untouched");
    }

    /// Typing in a text field snapshots once when the field is entered, not once per character.
    #[test]
    fn name_edit_pushes_one_undo_per_visit() {
        let mut p = Project::new();
        let c = Clip::new(0, ClipKind::Text, "a", 0.0, 5.0);
        let id = c.id;
        p.tracks[0].clips.push(c);
        let palette = Palette::new(true, egui::Color32::WHITE);
        let ctx = egui::Context::default();
        let mut undos = 0;
        // focus is requested inside the pass: done between passes it counts as "had focus last frame"
        let mut run = |input: egui::RawInput, focus: Option<egui::Id>, p: &mut Project, undos: &mut usize| {
            let mut edited = false;
            let _ = ctx.run(input, |ctx| {
                if let Some(f) = focus {
                    ctx.memory_mut(|m| m.request_focus(f));
                }
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut undo = |_: &Project| *undos += 1;
                    edited = show(ui, p, &[id], &[], 1.0, &[], &palette, &mut Settings::default(), &mut undo);
                });
            });
            edited
        };
        run(egui::RawInput::default(), None, &mut p, &mut undos); // layout, records the name field id
        assert_eq!(undos, 0);
        let field = ctx.data_mut(|d| d.get_temp::<egui::Id>(egui::Id::new("test_name_field"))).expect("name id");
        assert!(!run(egui::RawInput::default(), Some(field), &mut p, &mut undos), "entering a field is not an edit");
        assert_eq!(undos, 1, "one snapshot when the field is entered");
        for ch in ["h", "i"] {
            let mut input = egui::RawInput::default();
            input.events.push(egui::Event::Text(ch.into()));
            assert!(run(input, None, &mut p, &mut undos));
        }
        assert_eq!(undos, 1, "no extra snapshot per keystroke");
        assert!(p.clip(id).unwrap().name.contains("hi"), "{}", p.clip(id).unwrap().name);
    }

    struct H {
        ctx: egui::Context,
        project: Project,
        id: Id,
        /// Selected transitions handed to show(); when non-empty the clip selection is left empty.
        sel_trans: Vec<Id>,
        undos: usize,
        time: f64,
    }

    impl H {
        fn shape_clip() -> Self {
            let mut project = Project::new();
            let id = project.add_shape_clip(crate::model::ShapeKind::Star, 0.0, 3.0);
            Self { ctx: egui::Context::default(), project, id, sel_trans: Vec::new(), undos: 0, time: 0.0 }
        }
        fn audio_clip() -> Self {
            let mut project = Project::new();
            let id = project.new_id();
            project.tracks[1].clips.push(Clip::new(id, ClipKind::Audio, "a", 0.0, 3.0));
            Self { ctx: egui::Context::default(), project, id, sel_trans: Vec::new(), undos: 0, time: 0.0 }
        }
        /// Two abutting video clips with a cut transition and an edge transition, both selected.
        fn transitions() -> Self {
            use crate::model::TransitionKind;
            let mut project = Project::new();
            let a = project.new_id();
            project.tracks[0].clips.push(Clip::new(a, ClipKind::Video, "a", 0.0, 2.0));
            let b = project.new_id();
            project.tracks[0].clips.push(Clip::new(b, ClipKind::Video, "b", 2.0, 2.0));
            let t1 = project.add_transition(b, TransitionKind::CrossFade, 1.0).unwrap();
            let t2 = project.add_edge_transition(a, TransitionKind::CrossFade, 1.0, false).unwrap();
            Self { ctx: egui::Context::default(), project, id: a, sel_trans: vec![t1, t2], undos: 0, time: 0.0 }
        }
        fn frame(&mut self, events: Vec<egui::Event>) -> bool {
            self.time += 0.05;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(420.0, 1400.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let pal = Palette::new(true, egui::Color32::WHITE);
            let H { ctx, project, id, sel_trans, undos, .. } = self;
            let sel: &[Id] = if sel_trans.is_empty() { &[*id] } else { &[] };
            let mut changed = false;
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut undo = |_: &Project| *undos += 1;
                    changed |= show(ui, project, sel, sel_trans, 1.0, &[], &pal, &mut Settings::default(), &mut undo);
                });
            });
            changed
        }
        fn maybe_rect(&self, name: &str) -> Option<egui::Rect> {
            self.ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("insp", name.to_string()))))
        }
        fn rect(&self, name: &str) -> egui::Rect {
            self.maybe_rect(name).unwrap_or_else(|| panic!("no widget rect for {name}"))
        }
        fn click(&mut self, pos: egui::Pos2) -> bool {
            let mut e = self.frame(vec![egui::Event::PointerMoved(pos)]);
            e |= self.frame(vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }]);
            e |= self.frame(vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }]);
            e
        }
    }

    /// Selected transitions get their own inspector section, and "Remove" deletes them all in one undo.
    #[test]
    fn transition_section_shows_and_removes_all_selected() {
        let mut h = H::transitions();
        h.frame(vec![]);
        let r = h.rect("tr_remove");
        assert!(h.click(r.center()), "removing transitions edits the project");
        assert!(h.project.tracks[0].transitions.is_empty(), "both selected transitions removed");
        assert_eq!(h.undos, 1, "one undo for the whole removal");
        // redrawing with the stale selection is not an edit (the section shows a hint instead)
        assert!(!h.frame(vec![]));
        assert_eq!(h.undos, 1);
    }

    /// The mask section appears, "Add mask" creates one with a single undo, and the follow-up
    /// "Edit in viewport" hands the clip to the app.
    #[test]
    fn mask_section_adds_and_hands_off() {
        let mut h = H::shape_clip();
        h.frame(vec![]);
        let r = h.rect("add_mask");
        assert!(h.click(r.center()), "adding a mask edits the project");
        assert!(h.project.clip(h.id).unwrap().mask.is_some());
        assert_eq!(h.undos, 1, "one undo per gesture");
        h.frame(vec![]);
        let r = h.rect("edit_mask");
        let _ = take_edit_mask();
        h.click(r.center());
        assert_eq!(take_edit_mask(), Some(h.id), "the app is told which clip to mask");
        assert_eq!(h.undos, 1, "asking to edit in the viewport is not an edit");
    }

    /// A mask shapes pixels, so an audio clip shows no mask section at all — not even a dead button,
    /// and not even for a mask a hand-edited project smuggled in.
    #[test]
    fn audio_clips_get_no_mask_section() {
        let mut h = H::audio_clip();
        h.frame(vec![]);
        assert!(h.maybe_rect("add_mask").is_none(), "no Add mask on an audio clip");
        h.project.clip_mut(h.id).unwrap().mask = Some(Mask::default());
        h.frame(vec![]);
        assert!(h.maybe_rect("edit_mask").is_none(), "and no editor for one that got in anyway");
    }

    /// Shape clips get the style section, and a Shape clip's markers can be added from the inspector.
    #[test]
    fn shape_and_marker_sections() {
        let mut h = H::shape_clip();
        h.frame(vec![]);
        assert!(h.project.clip(h.id).unwrap().shape.is_some(), "the shape style exists");
        let r = h.rect("add_marker");
        assert!(h.click(r.center()), "adding a marker edits the project");
        let markers = &h.project.clip(h.id).unwrap().markers;
        assert_eq!(markers.len(), 1);
        assert!(markers[0].t >= 0.0 && markers[0].t <= 3.0, "inside the clip: {}", markers[0].t);
        assert_eq!(h.undos, 1);
    }

    #[test]
    fn node_button_hands_the_clip_off() {
        let mut h = H::shape_clip();
        h.frame(vec![]);
        let r = h.rect("open_nodes");
        let _ = take_open_nodes();
        h.click(r.center());
        assert_eq!(take_open_nodes(), Some(h.id));
        assert_eq!(h.undos, 0);
    }

    /// The label combo lists the project's own labels, and removing one re-points the clips using it.
    #[test]
    fn labels_come_from_the_project() {
        let mut p = Project::new();
        assert!(!p.labels.is_empty(), "a new project ships the default labels");
        let n = p.labels.len();
        let idx = p.add_label("Hero", [10, 20, 30]);
        assert_eq!(idx as usize, n + 1);
        assert_eq!(p.label_name(idx), "Hero");
        assert_eq!(p.label_color(idx), Some([10, 20, 30]));
        let cid = p.add_shape_clip(crate::model::ShapeKind::Rect, 0.0, 1.0);
        p.clip_mut(cid).unwrap().label = idx;
        p.remove_label(idx);
        assert_eq!(p.clip(cid).unwrap().label, 0, "the clip falls back to no label");
    }

    /// Headless: sections for effects / retime / audio fades lay out without panicking.
    #[test]
    fn show_headless_sections() {
        let a = Asset {
            id: 0,
            path: r"C:\m\a.mp4".into(),
            kind: ClipKind::Video,
            duration: 10.0,
            width: 1280,
            height: 720,
            fps: 30.0,
            audio_streams: vec![Default::default()],
            codec: String::new(),
            folder: String::new(),
            tags: vec!["x".into()],
            label: 2,
            description: "d".into(),
        };
        let mut p = Project::from_media(a);
        let vid = p.tracks[0].clips[0].id;
        p.clip_mut(vid).unwrap().effects.push(crate::model::Effect::new(crate::model::EffectKind::Blur));
        p.clip_mut(vid).unwrap().speed = 2.0;
        let right = p.split_at(4.0, None)[0];
        p.add_transition(right, crate::model::TransitionKind::CrossFade, 0.5);
        let aud = p.tracks[1].clips[0].id;
        let palette = Palette::new(true, egui::Color32::WHITE);
        let fonts: Vec<String> = Vec::new();
        let ctx = egui::Context::default();
        for sel in [vid, right, aud] {
            for _ in 0..2 {
                let _ = ctx.run(egui::RawInput::default(), |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let mut undo = |_: &Project| panic!("no undo without edits");
                        assert!(!show(
                            ui,
                            &mut p,
                            &[sel],
                            &[],
                            1.0,
                            &fonts,
                            &palette,
                            &mut Settings::default(),
                            &mut undo
                        ));
                    });
                });
            }
        }
    }
}
