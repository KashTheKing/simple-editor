//! Inspector panel. Nothing selected → Project panel (name, width, height, fps, duration, Save / Export /
//! Export Frame buttons, export settings summary). One clip selected →
//! clip name, enabled, colour label, timing (start / duration / source in), retime readout (Ctrl+R), and:
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
use crate::model::{Animated, BlendMode, ClipKind, Id, Label, Mask, Project, ShapeKind, ShapeStyle};
use crate::settings::Settings;
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
    playhead: f64,
    fonts: &[String],
    palette: &Palette,
    settings: &mut Settings,
    undo: &mut dyn FnMut(&Project),
) -> bool {
    match selection.iter().find(|&&id| project.clip(id).is_some()) {
        None => project_section(ui, project, settings, undo),
        Some(&id) => clip_section(ui, project, id, selection.len(), playhead, fonts, palette, undo),
    }
}

fn project_section(
    ui: &mut egui::Ui,
    project: &mut Project,
    settings: &mut Settings,
    undo: &mut dyn FnMut(&Project),
) -> bool {
    let mut edited = false;
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
        ui.weak(format!(
            "{} · CRF {} · {} · {} · {}",
            settings.encoder, settings.crf, settings.preset, res, scaler_name
        ));
    });
    if ui.small_button("Edit export settings…").clicked() {
        PENDING_ACTION.with(|p| *p.borrow_mut() = Some(Action::ExportVideo));
    }
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
    // snapshots: the widgets edit a clone of the clip, so the project must not stay borrowed
    let labels: Vec<Label> = project.labels.clone();
    let buses: Vec<(Id, String)> = project.buses.iter().map(|b| (b.id, b.name.clone())).collect();
    let labels_open_id = egui::Id::new("inspector_labels_open");
    let mut edit_labels: bool = ui.ctx().data(|d| d.get_temp(labels_open_id).unwrap_or(false));
    let mut label_ops: Vec<LabelOp> = Vec::new();
    let mut path_op: Option<PathOp> = None;

    if n_selected > 1 {
        ui.label(format!("{n_selected} clips selected"));
    }
    if clip.container {
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
        ui.separator();
    }
    ui.strong("Clip");
    Grid::new("inspector_clip").num_columns(2).show(ui, |ui| {
        ui.label("Name");
        let r = ui.text_edit_singleline(&mut clip.name);
        #[cfg(test)]
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("test_name_field"), r.id));
        g.note_text(&r);
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
                    "Pan" => {
                        let r = ui.add(Slider::new(&mut v, -1.0..=1.0).fixed_decimals(2));
                        #[cfg(test)]
                        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("test_pan_slider"), r.id));
                        r
                    }
                    "Scale" => ui.add(DragValue::new(&mut v).speed(0.01).range(0.01..=20.0)),
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
                };
                if r.changed() {
                    a.set_at(lt, v);
                }
                g.note(&r);
                key_buttons(ui, a, lt, palette, &mut g);
            });
            ui.end_row();
        }
        if clip.kind == ClipKind::Audio {
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

    let transitions: Vec<String> =
        project.transitions_of(id).iter().map(|(_, tr)| format!("{} · {:.2} s", tr.kind.name(), tr.duration)).collect();
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
    let mut ga = Gesture::default();
    let mut asset_desc: Option<String> = None;
    let mut asset_tags: Option<Vec<String>> = None;
    if clip.uses_asset() {
        if let Some(a) = project.asset(clip.asset) {
            ui.separator();
            ui.strong("Asset");
            ui.add(egui::Label::new(RichText::new(a.name()).weak()).truncate()).on_hover_text(&a.path);
            ui.weak(format!("Used in: {} clips", asset_use_count(project, a.id)));
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
        g.note_text(&ui.text_edit_multiline(&mut style.text));
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
                let r = ui.small_button("Save").on_hover_text("Keep this outline in the project as a reusable path");
                mark(ui, "save_path", &r);
                if r.clicked() {
                    path_op = Some(PathOp::Save);
                }
            }
            if !paths.is_empty() {
                egui::ComboBox::from_id_salt("clip_path").selected_text("Animate X/Y").width(130.0).show_ui(ui, |ui| {
                    for (pid, name) in &paths {
                        if ui.selectable_label(false, name).clicked() {
                            path_op = Some(PathOp::Apply(*pid));
                        }
                    }
                });
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

    // compact labels editor (labels live on the project, not the clip)
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
            let pts = project.path(pid).map(|p| p.points.clone()).unwrap_or_default();
            project.apply_path(id, &pts);
        }
        None => {}
    }
    g.changed || ga.changed || labels_changed || path_op.is_some()
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
                    show(ui, p, &[id], 1.0, &fonts, &palette, &mut Settings::default(), &mut undo);
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
                    show(ui, p, &[id], 1.0, &fonts, &palette, &mut Settings::default(), &mut undo);
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
                        show(ui, p, &[], 1.0, &fonts, &palette, &mut settings, &mut undo);
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
                    edited = show(ui, p, &[id], 1.0, &[], &palette, &mut Settings::default(), &mut undo);
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
        undos: usize,
        time: f64,
    }

    impl H {
        fn shape_clip() -> Self {
            let mut project = Project::new();
            let id = project.add_shape_clip(crate::model::ShapeKind::Star, 0.0, 3.0);
            Self { ctx: egui::Context::default(), project, id, undos: 0, time: 0.0 }
        }
        fn audio_clip() -> Self {
            let mut project = Project::new();
            let id = project.new_id();
            project.tracks[1].clips.push(Clip::new(id, ClipKind::Audio, "a", 0.0, 3.0));
            Self { ctx: egui::Context::default(), project, id, undos: 0, time: 0.0 }
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
            let H { ctx, project, id, undos, .. } = self;
            let id = *id;
            let mut changed = false;
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut undo = |_: &Project| *undos += 1;
                    changed |= show(ui, project, &[id], 1.0, &[], &pal, &mut Settings::default(), &mut undo);
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
                        assert!(!show(ui, &mut p, &[sel], 1.0, &fonts, &palette, &mut Settings::default(), &mut undo));
                    });
                });
            }
        }
    }
}
