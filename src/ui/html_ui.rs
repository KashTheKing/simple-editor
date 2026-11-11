//! ---- ws:html-layer ----
//! The HTML editor window (same shape as the shader editor): the whole file on the left, a live
//! preview on the right (rendered off the UI thread ~400 ms after typing stops; the old picture stays
//! until the new one arrives), a scrub bar, the library rule findings and any render error, Apply,
//! and an "Ask the assistant" box. Non-modal. Also the Inspector's "Fields" section of an HTML clip.
//! Hand-offs to the app (`html_ctl` drains them each frame): which clip to open in the editor, and
//! that a field value changed (re-render).

use crate::engine::html;
use crate::model::{Id, Project};
use eframe::egui;
use std::cell::{Cell, RefCell};
use std::sync::mpsc;
use std::time::{Duration, Instant};

thread_local! {
    static EDIT: RefCell<Option<Id>> = const { RefCell::new(None) };
    static EDITED: Cell<bool> = const { Cell::new(false) };
}

/// Open the editor on `clip` (a timeline double-click, the Inspector's "Edit HTML…").
pub fn request_edit(ctx: &egui::Context, clip: Id) {
    EDIT.with(|e| *e.borrow_mut() = Some(clip));
    ctx.request_repaint();
}

pub fn take_edit_request() -> Option<Id> {
    EDIT.with(|e| e.borrow_mut().take())
}

/// A field value was changed in the Inspector since the last call.
pub fn take_fields_edited() -> bool {
    EDITED.with(|e| e.replace(false))
}

const DEBOUNCE: Duration = Duration::from_millis(400);
/// The preview renders with its longer side at this many pixels.
const PREVIEW_PX: f64 = 480.0;

#[derive(Default)]
pub struct HtmlUi {
    pub open: bool,
    /// The clip being edited.
    pub clip: Option<Id>,
    pub src: String,
    pub fields: Vec<(String, String)>,
    pub duration: f64,
    /// Scrub position, seconds.
    pub t: f64,
    pub lint: Vec<String>,
    pub error: String,
    pub ask: String,
    /// When the preview should be rendered next; None = up to date.
    due: Option<Instant>,
    job: Option<mpsc::Receiver<Result<crate::media::Frame, String>>>,
    tex: Option<egui::TextureHandle>,
}

impl HtmlUi {
    /// Point the window at an HTML clip.
    pub fn edit(&mut self, clip: Id, src: &str, fields: Vec<(String, String)>, duration: f64) {
        let tpl = html::parse(src);
        self.open = true;
        self.clip = Some(clip);
        self.src = src.to_string();
        self.fields = fields;
        self.duration = if duration > 0.0 { duration } else { tpl.duration };
        self.t = tpl.thumb_at().min(self.duration);
        self.lint = html::lint(src);
        self.error.clear();
        self.due = Some(Instant::now());
    }

    /// Start the preview render if one is due and none is running.
    fn kick(&mut self, ctx: &egui::Context) {
        if self.job.is_some() || self.due.is_none_or(|d| d > Instant::now()) {
            return;
        }
        self.due = None;
        let (tx, rx) = mpsc::channel();
        let (src, fields, t, ctx) = (self.src.clone(), self.fields.clone(), self.t, ctx.clone());
        let size = html::parse(&src).size;
        let scale = (PREVIEW_PX / size.0.max(size.1) as f64).min(1.0);
        let spawned = std::thread::Builder::new().name("html-preview".into()).spawn(move || {
            let _ = tx.send(html::render_frame(&src, &fields, scale, t));
            ctx.request_repaint();
        });
        match spawned {
            Ok(_) => self.job = Some(rx),
            Err(e) => self.error = e.to_string(),
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.job else { return };
        match rx.try_recv() {
            Ok(Ok(f)) => {
                let img = egui::ColorImage::from_rgba_unmultiplied([f.width as usize, f.height as usize], &f.rgba);
                match &mut self.tex {
                    Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                    None => self.tex = Some(ctx.load_texture("html_preview", img, egui::TextureOptions::LINEAR)),
                }
                self.error.clear();
                self.job = None;
            }
            Ok(Err(e)) => {
                self.error = e;
                self.job = None;
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => self.job = None,
        }
    }
}

/// What the window asks the app to do this frame.
#[derive(Default)]
pub struct EditorResponse {
    /// Write the source to the clip's asset (one undo step) and re-render.
    pub apply: bool,
    /// Send this to the Assistant, with the clip pinned.
    pub ask: Option<String>,
}

pub fn show(ctx: &egui::Context, st: &mut HtmlUi) -> EditorResponse {
    let mut resp = EditorResponse::default();
    if !st.open {
        return resp;
    }
    st.poll(ctx);
    st.kick(ctx);
    let mut open = st.open;
    egui::Window::new("HTML Editor").open(&mut open).default_size([920.0, 560.0]).show(ctx, |ui| {
        ui.columns(2, |cols| {
            let ui = &mut cols[0];
            ui.weak("HTML with its <style>. CSS animations only - no JavaScript. Fields: {{name}}.");
            egui::ScrollArea::both().id_salt("html_src").max_height(420.0).show(ui, |ui| {
                let r = ui.add(
                    egui::TextEdit::multiline(&mut st.src).code_editor().desired_rows(24).desired_width(f32::INFINITY),
                );
                if r.changed() {
                    st.due = Some(Instant::now() + DEBOUNCE);
                    st.lint = html::lint(&st.src);
                }
            });
            ui.horizontal(|ui| {
                resp.apply = ui.button("Apply").on_hover_text("Use this source for the clip and render it").clicked();
                if st.job.is_some() || st.due.is_some() {
                    ui.spinner();
                }
            });

            let ui = &mut cols[1];
            let w = ui.available_width();
            let (rect, _) = ui.allocate_exact_size(egui::vec2(w, w * 9.0 / 16.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 2.0, egui::Color32::from_rgb(0x2b, 0x2f, 0x3a));
            if let Some(t) = &st.tex {
                let [tw, th] = t.size().map(|v| v as f32);
                let k = (rect.width() / tw).min(rect.height() / th);
                let r = egui::Rect::from_center_size(rect.center(), egui::vec2(tw * k, th * k));
                let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                ui.painter().image(t.id(), r, uv, egui::Color32::WHITE);
            }
            let max = st.duration.max(0.1);
            let r = ui.add(egui::Slider::new(&mut st.t, 0.0..=max).suffix(" s").text("time"));
            if r.changed() {
                st.due = Some(Instant::now()); // as soon as the running render is done
            }
            if !st.error.is_empty() {
                ui.colored_label(ui.visuals().error_fg_color, &st.error);
            }
            egui::ScrollArea::vertical().id_salt("html_lint").max_height(110.0).show(ui, |ui| {
                if st.lint.is_empty() {
                    ui.weak("Follows the library rules.");
                }
                for l in &st.lint {
                    ui.colored_label(ui.visuals().warn_fg_color, l);
                }
            });
            ui.separator();
            ui.label("Ask the assistant");
            ui.horizontal(|ui| {
                let r = ui.add(
                    egui::TextEdit::singleline(&mut st.ask)
                        .hint_text("make the button bounce twice")
                        .desired_width(ui.available_width() - 50.0),
                );
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (ui.button("Ask").clicked() || enter) && !st.ask.trim().is_empty() {
                    resp.ask = Some(std::mem::take(&mut st.ask));
                }
            });
        });
    });
    st.open = open;
    if !st.open {
        st.due = None; // a closed window renders nothing more (a running render just finishes)
    } else if st.job.is_none() {
        if let Some(d) = st.due {
            ctx.request_repaint_after(d.saturating_duration_since(Instant::now()));
        }
    }
    resp
}

// ---------------------------------------------------------------------------------------------------
// the Inspector's Fields section

fn hex_rgba(s: &str) -> [u8; 4] {
    let h = s.trim().trim_start_matches('#');
    let d = |i: usize, n: usize| u8::from_str_radix(h.get(i..i + n).unwrap_or("0"), 16).unwrap_or(0);
    match h.len() {
        3 | 4 => {
            let c = |i: usize| d(i, 1) * 17;
            [c(0), c(1), c(2), if h.len() == 4 { c(3) } else { 255 }]
        }
        8 => [d(0, 2), d(2, 2), d(4, 2), d(6, 2)],
        _ => [d(0, 2), d(2, 2), d(4, 2), 255],
    }
}

fn rgba_hex(c: [u8; 4]) -> String {
    match c[3] {
        255 => format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]),
        a => format!("#{:02x}{:02x}{:02x}{a:02x}", c[0], c[1], c[2]),
    }
}

/// One control per `se:field` of the clip's template, plus "Edit HTML…". One undo step per committed
/// change; the re-render is the app's (`take_fields_edited`). True when the project changed.
pub fn fields_section(ui: &mut egui::Ui, project: &mut Project, clip: Id, undo: &mut dyn FnMut(&Project)) -> bool {
    let Some(aid) = project.clip(clip).map(|c| c.asset) else { return false };
    let Some(h) = project.asset(aid).and_then(|a| a.html.clone()) else { return false };
    let tpl = html::parse(&h.src);
    let mut vals = h.fields.clone();
    let mut g = crate::ui::Gesture::default();
    if tpl.fields.is_empty() {
        ui.weak("This template has no fields.");
    }
    egui::Grid::new("html_fields").num_columns(2).show(ui, |ui| {
        for f in &tpl.fields {
            ui.label(crate::engine::shaderlib::label(&f.name));
            let cur = vals.get(&f.name).cloned().unwrap_or_else(|| f.default.clone());
            let name = f.name.clone();
            match f.ty {
                html::FieldType::Text => {
                    let mut s = cur;
                    let r = ui.text_edit_singleline(&mut s);
                    g.note_text(&r);
                    if r.changed() {
                        vals.insert(name, s);
                    }
                }
                html::FieldType::Color => {
                    let mut c = hex_rgba(&cur);
                    let r = ui.color_edit_button_srgba_unmultiplied(&mut c);
                    g.note(&r);
                    if r.changed() {
                        vals.insert(name, rgba_hex(c));
                    }
                }
                html::FieldType::Number => {
                    let mut n = cur.trim().parse::<f64>().unwrap_or(0.0);
                    let mut dv = egui::DragValue::new(&mut n);
                    if let (Some(a), Some(b)) = (f.min, f.max) {
                        dv = dv.range(a..=b).speed(((b - a) / 300.0).max(0.01));
                    }
                    let r = ui.add(dv);
                    g.note(&r);
                    if r.changed() {
                        vals.insert(name, html::field_value(f, &[(f.name.clone(), n.to_string())]));
                    }
                }
                html::FieldType::Image => {
                    ui.horizontal(|ui| {
                        let file = std::path::Path::new(&cur).file_name().map(|n| n.to_string_lossy().into_owned());
                        ui.weak(file.as_deref().unwrap_or("(none)")).on_hover_text(&cur);
                        if ui.small_button("Pick…").clicked() {
                            let pick = rfd::FileDialog::new()
                                .add_filter("Images", &["png", "jpg", "jpeg", "gif", "webp", "svg", "bmp"])
                                .pick_file();
                            if let Some(p) = pick {
                                g.click();
                                vals.insert(name.clone(), p.to_string_lossy().into_owned());
                            }
                        }
                        if !cur.is_empty() && ui.small_button("×").on_hover_text("No picture").clicked() {
                            g.click();
                            vals.insert(name, String::new());
                        }
                    });
                }
            }
            ui.end_row();
        }
    });
    if ui.button("Edit HTML…").on_hover_text("Open the source in the HTML editor").clicked() {
        request_edit(ui.ctx(), clip);
    }
    if !g.changed || vals == h.fields {
        return false;
    }
    if g.start {
        undo(project);
    }
    if let Some(a) = project.asset_mut(aid).and_then(|a| a.html.as_mut()) {
        a.fields = vals;
    }
    EDITED.with(|e| e.set(true));
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_round_trip_through_the_picker() {
        assert_eq!(hex_rgba("#e62117"), [0xe6, 0x21, 0x17, 255]);
        assert_eq!(hex_rgba("#00ff0080"), [0, 255, 0, 0x80]);
        assert_eq!(hex_rgba("#fff"), [255, 255, 255, 255]);
        assert_eq!(rgba_hex([0xe6, 0x21, 0x17, 255]), "#e62117");
        assert_eq!(rgba_hex([0, 255, 0, 0x80]), "#00ff0080");
    }

    /// An open, settled editor (nothing typed, no render running) asks for no frames of its own.
    #[test]
    fn assert_no_idle_repaint_html_editor() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let mut st = HtmlUi {
            open: true,
            clip: Some(1),
            src: html::icon_page("x", "<svg></svg>"),
            duration: 4.0,
            ..Default::default()
        };
        for _ in 0..30 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                let r = show(ctx, &mut st);
                assert!(!r.apply && r.ask.is_none());
            });
        }
        assert!(st.job.is_none(), "nothing was due, so nothing rendered");
        assert!(!ctx.has_requested_repaint(), "idle HTML editor requested a repaint");
    }

    /// Fields: drawing them is not an edit (no undo, no re-render); a stored value reaches the page.
    #[test]
    fn fields_section_is_quiet_until_edited() {
        use crate::model::{Asset, ClipKind, HtmlSource};
        let mut p = Project::new();
        let src = html::icon_page("x", "<svg></svg>");
        let aid = p.add_asset(Asset {
            kind: ClipKind::Image,
            html: Some(HtmlSource { name: "x".into(), src, fields: Default::default(), duration: 4.0 }),
            ..crate::engine::import::placeholder("C:/x/pending.png")
        });
        let cid = p.insert_asset_clips(aid, 0.0, None)[0];
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let mut undos = 0;
        // nothing touched: no edit
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                assert!(!fields_section(ui, &mut p, cid, &mut |_| undos += 1));
            });
        });
        assert_eq!(undos, 0);
        assert!(!take_fields_edited());
        let a = p.asset_mut(aid).unwrap().html.as_mut().unwrap();
        a.fields.insert("color".into(), "#ff0000".into());
        assert!(html::prepare(&a.src, &a.field_list()).1.contains("color: #ff0000;"));
    }
}
