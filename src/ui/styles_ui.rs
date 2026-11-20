//! ---- feat/styles-context ----
//! The Assistant's Editing Styles tab (browse, preview, apply, duplicate, rename, delete, import, export),
//! the Learn a style window, and the Project Context pane (the project's own Markdown document plus the
//! read-only digest). Drawing only: every change is returned as an `Out` for `ui::app::styles_ctl`.

use super::list_rows::{Ctrl, RowSpec, Rows};
use super::tools::{icon_button, Glyph};
use crate::engine::styles::{self, Source, Style};
use crate::theme::Palette;
use eframe::egui;

#[derive(Default)]
pub struct StylesState {
    /// The style shown in the preview (None = the applied one, else the first).
    pub selected: Option<String>,
    /// The style being renamed and the edit buffer.
    pub rename: Option<(String, String)>,
    /// The Learn a style window (open while Some).
    pub learn: Option<Learn>,
}

/// The Learn a style window's fields.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Learn {
    pub path: String,
    pub name: String,
    pub describe: bool,
    /// The describer model id (`vlm::DEFAULT` or the upgrade).
    pub model: String,
    pub speech: bool,
    pub max_shots: usize,
    /// The running / finished learn job (`style_learn::job`).
    pub job: Option<u64>,
    /// The draft style file, editable once the job is done.
    pub draft: String,
    pub error: String,
    /// The user chose "Measure only" over installing the describer.
    pub declined: bool,
}

impl Learn {
    pub fn new(describer_ready: bool) -> Self {
        Learn { describe: describer_ready, model: crate::engine::vlm::DEFAULT.into(), max_shots: 24, ..Default::default() }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Out {
    None,
    Apply(String),
    Remove,
    Duplicate(String),
    Rename(String, String),
    Delete(String),
    Import,
    Export(String),
    OpenFolder,
    /// Open the Learn a style window.
    OpenLearn,
    /// Toggle the Project Context pane.
    ToggleContext,
}

/// What the Styles tab needs from the app.
pub struct View<'a> {
    pub styles: &'a [Style],
    /// The style written into this project's context, if any.
    pub applied: Option<&'a str>,
    /// The Project Context pane is on screen (its button is lit).
    pub context_open: bool,
}

const CTRLS: &[Ctrl] = &[
    Ctrl { glyph: "✓", hint: "Apply to this project" },
    Ctrl { glyph: "⧉", hint: "Duplicate" },
    Ctrl { glyph: "✕", hint: "Delete" },
];

/// The Project Context pane's toggle (lit while it is open), shared by the tab and the Assistant's top bar.
pub fn context_button(ui: &mut egui::Ui, pal: &Palette, open: bool) -> bool {
    let tip = if open { "Close Project Context" } else { "Project Context: what the assistant knows about this project" };
    icon_button(ui, pal, ui.id().with("ctx_toggle"), Glyph::Notepad, tip, open).clicked()
}

pub fn styles_tab(ui: &mut egui::Ui, st: &mut StylesState, v: &View) -> Out {
    let pal = crate::ui::titlebar::palette(ui.ctx());
    let mut out = Out::None;
    ui.horizontal_wrapped(|ui| {
        let r = ui.button("Learn this style…").on_hover_text(
            "Measure a reference video (shot lengths, pacing, cuts on the beat, transitions, loudness, colour) and write a draft style",
        );
        if r.clicked() {
            out = Out::OpenLearn;
        }
        if ui.button("Import…").on_hover_text("Add a style file (.md) to your library").clicked() {
            out = Out::Import;
        }
        if ui.button("Open folder").on_hover_text("Your styles are Markdown files in this folder").clicked() {
            out = Out::OpenFolder;
        }
        if context_button(ui, &pal, v.context_open) {
            out = Out::ToggleContext;
        }
    });
    ui.add_space(2.0);
    egui::ScrollArea::vertical().id_salt("assistant_styles").auto_shrink([false, false]).show(ui, |ui| {
        let mut rows = Rows::new(ui, "style_rows", CTRLS, &[64.0], false).fixed_order();
        for (i, s) in v.styles.iter().enumerate() {
            let applied = v.applied == Some(s.name.as_str());
            let tag = if applied { "Applied" } else { s.source.label() };
            let blank = if s.source == Source::User { 0 } else { 0b100 };
            let r = rows.row(ui, i, &RowSpec { key: i as u64 + 1, label: &s.title, values: &[tag], blank, ..Default::default() });
            let r_body = r.body.on_hover_text(format!("/style {}  ·  {}", s.name, s.description));
            if r_body.clicked() {
                st.selected = Some(s.name.clone());
            }
            match r.ctrl {
                Some(0) => out = Out::Apply(s.name.clone()),
                Some(1) => out = Out::Duplicate(s.name.clone()),
                Some(2) if s.source == Source::User => out = Out::Delete(s.name.clone()),
                _ => {}
            }
        }
        let _ = rows.finish(ui);
        ui.separator();
        let shown = st.selected.as_deref().or(v.applied).and_then(|n| v.styles.iter().find(|s| s.name == n)).or(v.styles.first());
        if let Some(s) = shown {
            preview(ui, st, s, v, &pal, &mut out);
        }
    });
    out
}

fn preview(ui: &mut egui::Ui, st: &mut StylesState, s: &Style, v: &View, pal: &Palette, out: &mut Out) {
    ui.horizontal_wrapped(|ui| {
        ui.heading(&s.title);
        ui.label(egui::RichText::new(format!("/style {}", s.name)).monospace().weak());
    });
    if !s.description.is_empty() {
        ui.add(egui::Label::new(egui::RichText::new(&s.description).weak()).wrap());
    }
    ui.horizontal_wrapped(|ui| {
        if v.applied == Some(s.name.as_str()) {
            if ui.button("Remove").on_hover_text("Take this style back out of Project Context (what you wrote stays)").clicked() {
                *out = Out::Remove;
            }
        } else if ui.button("Apply").on_hover_text("Write it into this project's Style guide and Rules (one undo step)").clicked() {
            *out = Out::Apply(s.name.clone());
        }
        if ui.button("Duplicate").on_hover_text("Make your own copy to change").clicked() {
            *out = Out::Duplicate(s.name.clone());
        }
        if s.source == Source::User && ui.button("Rename").clicked() {
            st.rename = Some((s.name.clone(), s.name.clone()));
        }
        if ui.button("Export…").on_hover_text("Save the style file somewhere else").clicked() {
            *out = Out::Export(s.name.clone());
        }
        if s.source == Source::User && ui.button("Delete").clicked() {
            *out = Out::Delete(s.name.clone());
        }
    });
    if let Some((old, buf)) = st.rename.as_mut().filter(|(o, _)| *o == s.name) {
        let mut done = None;
        ui.horizontal(|ui| {
            let r = ui.add(egui::TextEdit::singleline(buf).desired_width(160.0).hint_text("new-name"));
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.button("OK").clicked() || enter {
                done = Some(Some((old.clone(), styles::slug(buf))));
            }
            if ui.button("Cancel").clicked() {
                done = Some(None);
            }
        });
        if let Some(d) = done {
            st.rename = None;
            if let Some((a, b)) = d {
                *out = Out::Rename(a, b);
            }
        }
    }
    ui.add_space(4.0);
    let rules = styles::rules_markdown(s);
    if !rules.is_empty() {
        ui.strong("Rules");
        crate::ui::markdown::show(ui, &rules, pal);
        ui.add_space(4.0);
    }
    if !s.body.trim().is_empty() {
        ui.strong("Style guide");
        crate::ui::markdown::show(ui, &s.body, pal);
    }
}

// ---------------------------------------------------------------------------------------------------
// Learn a style

#[derive(Debug, PartialEq, Eq)]
pub enum LearnOut {
    None,
    Pick,
    Start,
    Cancel,
    /// Save the draft to the library (and apply it to the project when true).
    Save(bool),
    Install(String),
    Close,
}

/// What the window shows of the job and the describer.
#[derive(Default)]
pub struct LearnView {
    /// The job: (fraction, status, finished, error).
    pub job: Option<(f32, String, bool, Option<String>)>,
    /// Installed describer model ids.
    pub models: Vec<String>,
    /// A describer install running: (fraction, status).
    pub installing: Option<(f32, String)>,
    pub install_error: Option<String>,
    /// Download size of the default model (plus the runtime when missing), bytes; and of the upgrade.
    pub size_default: u64,
    pub size_upgrade: u64,
    /// Speech-to-text is installed (words per minute).
    pub can_transcribe: bool,
}

fn mb(b: u64) -> String {
    format!("{:.0} MB", b as f64 / 1_048_576.0)
}

pub fn learn_window(ctx: &egui::Context, l: &mut Learn, v: &LearnView) -> LearnOut {
    let mut out = LearnOut::None;
    let mut open = true;
    egui::Window::new("Learn a style").id(egui::Id::new("learn_style_window")).open(&mut open).default_width(520.0).resizable(true).show(ctx, |ui| {
        let running = v.job.as_ref().is_some_and(|j| !j.2);
        let finished = v.job.as_ref().is_some_and(|j| j.2 && j.3.is_none());
        egui::Grid::new("learn_grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            ui.label("Video");
            ui.horizontal(|ui| {
                if ui.add_enabled(!running, egui::Button::new("Choose…")).on_hover_text("The file is only read, never changed").clicked() {
                    out = LearnOut::Pick;
                }
                let name = std::path::Path::new(&l.path).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
                ui.add(egui::Label::new(if name.is_empty() { "None".to_string() } else { name }).truncate()).on_hover_text(&l.path);
            });
            ui.end_row();
            ui.label("Name");
            ui.add_enabled(!running, egui::TextEdit::singleline(&mut l.name).hint_text("my-style").desired_width(200.0));
            ui.end_row();
            ui.label("Shots");
            ui.horizontal(|ui| {
                if v.models.is_empty() {
                    ui.weak("Measured");
                } else {
                    ui.add_enabled(!running, egui::Checkbox::new(&mut l.describe, "Describe"))
                        .on_hover_text("SmolVLM2 writes each shot's framing, subject, camera move and on-screen text. The numbers always come from the measurements.");
                    if l.describe && v.models.len() > 1 {
                        egui::ComboBox::from_id_salt("learn_model").selected_text(model_label(&l.model)).show_ui(ui, |ui| {
                            for m in &v.models {
                                ui.selectable_value(&mut l.model, m.clone(), model_label(m));
                            }
                        });
                    }
                    if l.describe {
                        ui.add(egui::DragValue::new(&mut l.max_shots).range(1..=200).suffix(" at most"))
                            .on_hover_text("Describing takes a few seconds a shot; longer videos are sampled evenly");
                    }
                }
            });
            ui.end_row();
            ui.label("Speech");
            ui.add_enabled(v.can_transcribe && !running, egui::Checkbox::new(&mut l.speech, "Words per minute"))
                .on_hover_text("Transcribe the sound to measure how fast people talk")
                .on_disabled_hover_text("Needs speech-to-text (Transcript ▸ Get captions installs it)");
            ui.end_row();
        });
        // the describer: offered the first time, installed on a click
        if v.models.is_empty() && !l.declined {
            ui.add_space(4.0);
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.strong("Describe each shot too?");
                ui.add(
                    egui::Label::new(
                        "The video describer (SmolVLM2 on llama.cpp, an optional add-on) looks at every shot and writes its framing, subject, \
                         camera move and on-screen text into the draft. It runs on this computer.",
                    )
                    .wrap(),
                );
                if let Some((f, s)) = &v.installing {
                    ui.add(egui::ProgressBar::new(*f).text(s.as_str()));
                } else {
                    ui.horizontal_wrapped(|ui| {
                        if ui.button(format!("Install ({})", mb(v.size_default))).on_hover_text("SmolVLM2 256M Video Instruct (Apache-2.0) and llama.cpp (MIT)").clicked() {
                            out = LearnOut::Install(crate::engine::vlm::DEFAULT.into());
                        }
                        if ui.button(format!("Larger model ({})", mb(v.size_upgrade))).on_hover_text("SmolVLM2 2.2B Instruct: better descriptions, slower").clicked() {
                            out = LearnOut::Install("2.2b".into());
                        }
                        if ui.button("Measure only").clicked() {
                            l.declined = true;
                        }
                    });
                    if let Some(e) = &v.install_error {
                        ui.colored_label(ui.visuals().error_fg_color, e);
                    }
                }
            });
        } else if !v.models.is_empty() && !v.models.iter().any(|m| m == "2.2b") && v.installing.is_none() {
            ui.horizontal(|ui| {
                if ui.small_button(format!("Get the larger model ({})", mb(v.size_upgrade))).on_hover_text("SmolVLM2 2.2B Instruct: better descriptions, slower").clicked() {
                    out = LearnOut::Install("2.2b".into());
                }
            });
        } else if let Some((f, s)) = &v.installing {
            ui.add(egui::ProgressBar::new(*f).text(s.as_str()));
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if running {
                if ui.button("Cancel").clicked() {
                    out = LearnOut::Cancel;
                }
            } else {
                let can = !l.path.trim().is_empty() && v.installing.is_none();
                let label = if finished { "Learn again" } else { "Learn" };
                if ui.add_enabled(can, egui::Button::new(label)).on_disabled_hover_text("Choose a video first").clicked() {
                    out = LearnOut::Start;
                }
            }
            if let Some((f, s, done, _)) = &v.job {
                if !done {
                    ui.add(egui::ProgressBar::new(*f).text(s.as_str()).desired_width(300.0));
                }
            }
        });
        if let Some((_, _, true, Some(e))) = &v.job {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
        if finished {
            ui.add_space(4.0);
            ui.strong("Draft");
            ui.weak("Review and edit it, then save it to your styles.");
            egui::ScrollArea::vertical().id_salt("learn_draft").max_height(320.0).show(ui, |ui| {
                ui.add(egui::TextEdit::multiline(&mut l.draft).code_editor().desired_width(f32::INFINITY).desired_rows(14));
            });
            ui.horizontal(|ui| {
                if ui.button("Save to styles").clicked() {
                    out = LearnOut::Save(false);
                }
                if ui.button("Save and apply").on_hover_text("Save it, then write it into this project's Project Context").clicked() {
                    out = LearnOut::Save(true);
                }
            });
            if !l.error.is_empty() {
                ui.colored_label(ui.visuals().error_fg_color, &l.error);
            }
        }
    });
    if !open {
        out = LearnOut::Close;
    }
    out
}

fn model_label(id: &str) -> &'static str {
    match id {
        "2.2b" => "SmolVLM2 2.2B",
        _ => "SmolVLM2 256M",
    }
}

// ---------------------------------------------------------------------------------------------------
// the Project Context pane

#[derive(Default)]
pub struct CtxState {
    /// The edit buffer and the project text it was last synced from.
    pub buf: String,
    pub synced: Option<String>,
    pub preview: bool,
    pub digest_open: bool,
    /// The project as it was when the current burst of typing began (one undo step per burst).
    pub before: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CtxOut {
    None,
    /// The text changed (live): write it to the project.
    Edited(String),
    /// The text box lost the keyboard: close the undo step.
    Commit,
    RemoveStyle,
    OpenStyles,
    Open(super::layout::Pane),
}

/// What the pane shows: the project's document, its applied style, and the digest (only built while open).
pub struct CtxView<'a> {
    pub doc: &'a str,
    pub applied: Option<String>,
    pub digest: Option<String>,
}

pub fn context_pane(ui: &mut egui::Ui, st: &mut CtxState, v: &CtxView) -> CtxOut {
    use super::layout::Pane;
    let pal = crate::ui::titlebar::palette(ui.ctx());
    let mut out = CtxOut::None;
    let id = egui::Id::new("project_context_text");
    let focused = ui.memory(|m| m.has_focus(id));
    // the project's text wins whenever the box is not being typed in (undo, an agent's context.set)
    if !focused && st.synced.as_deref() != Some(v.doc) {
        st.buf = if v.doc.trim().is_empty() { crate::model::context::template() } else { v.doc.to_string() };
        st.synced = Some(v.doc.to_string());
    }
    ui.horizontal_wrapped(|ui| {
        if let Some(name) = &v.applied {
            ui.label(egui::RichText::new(format!("Style: {name}")).strong());
            if ui.small_button("Remove").on_hover_text("Take the style back out (what you wrote stays)").clicked() {
                out = CtxOut::RemoveStyle;
            }
        }
        if ui.small_button("Styles").on_hover_text("Open Editing Styles in the Assistant").clicked() {
            out = CtxOut::OpenStyles;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icon_button(ui, &pal, ui.id().with("ctx_preview"), Glyph::Eye, "Preview the Markdown", st.preview).clicked() {
                st.preview = !st.preview;
            }
        });
    });
    egui::ScrollArea::vertical().id_salt("project_context_scroll").auto_shrink([false, false]).show(ui, |ui| {
        if st.preview {
            crate::ui::markdown::show(ui, &st.buf, &pal);
        } else {
            let r = ui.add(
                egui::TextEdit::multiline(&mut st.buf)
                    .id(id)
                    .code_editor()
                    .desired_width(f32::INFINITY)
                    .desired_rows(16)
                    .hint_text("About, Rules, Style guide, Links"),
            );
            let r = r.on_hover_text("The assistant reads this at the start of every request; Rules are followed as instructions");
            if r.changed() {
                let text = if crate::model::context::is_blank(&st.buf) { String::new() } else { st.buf.clone() };
                st.synced = Some(text.clone());
                out = CtxOut::Edited(text);
            }
            if r.lost_focus() {
                out = CtxOut::Commit;
            }
        }
        ui.add_space(6.0);
        let h = egui::CollapsingHeader::new("Digest").id_salt("ctx_digest").default_open(false).open(Some(st.digest_open)).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (p, label) in [(Pane::Planner, "Planner"), (Pane::Moodboard, "Moodboard"), (Pane::Markers, "Markers")] {
                    if ui.small_button(label).on_hover_text(format!("Open the {label}")).clicked() {
                        out = CtxOut::Open(p);
                    }
                }
            });
            match v.digest.as_deref() {
                Some("") | None => {
                    ui.weak("Nothing yet: planner items, notes, moodboard tags and open comments show here.");
                }
                Some(d) => crate::ui::markdown::show(ui, d, &pal),
            }
        });
        if h.header_response.clicked() {
            st.digest_open = !st.digest_open;
        }
        h.header_response.on_hover_text("Gathered from the planner, notes, moodboard and open comments; the assistant reads it too");
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(ctx: &egui::Context, n: usize, mut f: impl FnMut(&mut egui::Ui)) {
        for _ in 0..n {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| f(ui));
            });
        }
    }

    #[test]
    fn assert_no_idle_repaint_project_context() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let mut st = CtxState::default();
        let doc = crate::model::context::apply_style(&crate::model::context::template(), "vlog", "Fast.", "- Short shots.");
        let v = CtxView { doc: &doc, applied: Some("vlog".into()), digest: Some("### Notes\n#### Tone\nCalm.".into()) };
        run(&ctx, 30, |ui| {
            let _ = context_pane(ui, &mut st, &v);
        });
        assert!(!ctx.has_requested_repaint(), "an idle Project Context pane must request no repaint");
        assert!(st.buf.contains("<!-- style: vlog -->"), "the buffer follows the project");
        st.digest_open = true;
        st.preview = true;
        run(&ctx, 30, |ui| {
            let _ = context_pane(ui, &mut st, &v);
        });
        assert!(!ctx.has_requested_repaint(), "the open digest and the preview must request no repaint");
    }

    #[test]
    fn assert_no_idle_repaint_editing_styles() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let all = styles::list_in(std::path::Path::new("Z:/no-such-styles-dir"));
        let v = View { styles: &all, applied: Some("vlog"), context_open: true };
        let mut st = StylesState::default();
        run(&ctx, 30, |ui| {
            let _ = styles_tab(ui, &mut st, &v);
        });
        assert!(!ctx.has_requested_repaint(), "an idle Editing Styles tab must request no repaint");
        let mut l = Learn::new(false);
        l.path = "C:/ref.mp4".into();
        let lv = LearnView { size_default: 300 << 20, size_upgrade: 1700 << 20, ..Default::default() };
        for _ in 0..30 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                let _ = learn_window(ctx, &mut l, &lv);
            });
        }
        assert!(!ctx.has_requested_repaint(), "an idle Learn a style window must request no repaint");
    }

    /// The table's controls line up whatever the title length, and a built-in has no Delete.
    #[test]
    fn style_rows_click_through() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let all = styles::list_in(std::path::Path::new("Z:/no-such-styles-dir"));
        let v = View { styles: &all, applied: None, context_open: false };
        let mut st = StylesState { selected: Some("trailer".into()), ..Default::default() };
        let mut out = Out::None;
        run(&ctx, 2, |ui| out = styles_tab(ui, &mut st, &v));
        assert_eq!(out, Out::None);
    }
}
