//! Inspector panel. Nothing selected → Project settings (name, width, height, fps). One clip selected →
//! clip name, enabled, colour label, timing (start / duration / source in), retime readout (Ctrl+R), and:
//!  * visual clips: Position X/Y, Scale, Rotation, Opacity — each row = DragValue + "◆" keyframe toggle
//!    (`Animated::toggle_key(clip.local(playhead))`, highlighted when a key exists at the playhead) +
//!    "clear keys"; edits go through `Animated::set_at(local_t, v)` so keyframed props get keys; Blend mode combo.
//!  * audio clips: Volume (dB slider mapped to linear gain) and Pan (-1..1 slider) with the same keyframe
//!    controls, plus fade in/out lengths.
//!  * effects summary (enable toggles, remove) and transitions touching the clip.
//!  * the clip's asset: "Used in: N clips", editable description and tags.
//!  * text clips: multiline text, font family combo (`fonts`) + "Import font…" (handed to the app via
//!    `take_pending_font_import`), size, bold/italic, colour pickers (fill, outline, drop shadow, box),
//!    outline width, drop shadow on/off + x/y/blur, alignment, line/letter spacing, box padding.
//!  * sequence clips: the sequence's name + "Open sequence" (handed to the app via `take_open_sequence`).
//! Call `undo(project)` once per gesture (on `drag_started()` / first `changed()` of a widget) before mutating.
//! Returns true if the project changed.

use crate::model::{Animated, BlendMode, ClipKind, Id, Project, LABEL_COLORS};
use crate::theme::Palette;
use crate::ui::{label_name, timecode};
use eframe::egui::{self, Button, DragValue, Grid, Response, RichText, Slider};
use std::cell::RefCell;

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
    /// Text fields: the gesture starts when the field is entered, so typing a paragraph is one undo
    /// entry instead of one per character (which used to evict the whole undo stack).
    fn note_text(&mut self, r: &Response) {
        self.start |= r.gained_focus();
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

// Hand-offs to the app (the show() signature has no room for these; the app polls them each frame).
thread_local! {
    static PENDING_FONT: RefCell<Option<String>> = const { RefCell::new(None) };
    static OPEN_SEQUENCE: RefCell<Option<Id>> = const { RefCell::new(None) };
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
        let r = ui.text_edit_singleline(&mut clip.name);
        #[cfg(test)]
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("test_name_field"), r.id));
        g.note_text(&r);
        ui.end_row();
        ui.label("Enabled");
        g.note(&ui.checkbox(&mut clip.enabled, ""));
        ui.end_row();
        ui.label("Label");
        egui::ComboBox::from_id_salt("clip_label").selected_text(label_name(clip.label)).show_ui(ui, |ui| {
            g.note(&ui.selectable_value(&mut clip.label, 0, "None"));
            for (i, (name, [r, gc, b])) in LABEL_COLORS.iter().enumerate() {
                let t = RichText::new(format!("● {name}")).color(egui::Color32::from_rgb(*r, *gc, *b));
                g.note(&ui.selectable_value(&mut clip.label, i as u8 + 1, t));
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
                if ui.small_button("✕").clicked() {
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

    if g.start || ga.start {
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
    g.changed || ga.changed
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
        assert_eq!(label_name(0), "None");
        assert_eq!(label_name(1), "Red");
        assert_eq!(label_name(200), "None");
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
                    show(ui, p, &[id], 1.0, &fonts, &palette, &mut undo);
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
                    edited = show(ui, p, &[id], 1.0, &[], &palette, &mut undo);
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
                        assert!(!show(ui, &mut p, &[sel], 1.0, &fonts, &palette, &mut undo));
                    });
                });
            }
        }
    }
}
