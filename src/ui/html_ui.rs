//! ---- ws:html-layer ----
//! The HTML editor window (same shape as the shader editor): the whole file on the left, a live
//! preview on the right (the page itself running in Edge, `html::live_show`: every keystroke, field and
//! param shows at once; it plays in real time, or holds the scrub bar's moment), a scrub bar, the library rule findings and any render error, Apply,
//! and an "Ask the assistant" box. Non-modal. Also the Inspector's "Fields" section of an HTML clip.
//! Hand-offs to the app (`html_ctl` drains them each frame): which clip to open in the editor, and
//! that a field value changed (re-render).

use crate::engine::html;
use crate::model::{Id, Project};
use eframe::egui;
use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

thread_local! {
    static EDIT: RefCell<Option<Id>> = const { RefCell::new(None) };
    static EDITED: Cell<bool> = const { Cell::new(false) };
    static OPS: RefCell<Vec<Op>> = const { RefCell::new(Vec::new()) };
}

/// What the Inspector's HTML section asks the app for (dialogs, files and other programs are the
/// app's business; `html_ctl::tick` drains these every frame).
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// Write the source to its file (made on first use) and open it in a code editor.
    OpenExternal(Id),
    /// Pick an existing `.html` file: its text becomes the source, and the two stay in sync.
    LinkPick(Id),
    /// Choose where to save the source as a `.html` file; the two stay in sync.
    SaveAs(Id),
    Unlink(Id),
    Reveal(Id),
    /// Keyframe Position Y so a page taller than the frame scrolls from its top to its bottom.
    Scroll(Id),
    Ask(Id, String),
}

fn push_op(ctx: &egui::Context, op: Op) {
    OPS.with(|o| o.borrow_mut().push(op));
    ctx.request_repaint();
}

pub fn take_ops() -> Vec<Op> {
    OPS.with(|o| std::mem::take(&mut *o.borrow_mut()))
}

/// Open the editor on `clip` (a timeline double-click, the Inspector's "Edit HTML"): a window of its own.
pub fn request_edit(ctx: &egui::Context, clip: Id) {
    EDIT.with(|e| *e.borrow_mut() = Some(clip));
    ctx.request_repaint();
}

pub fn take_edit_request() -> Option<Id> {
    EDIT.with(|e| e.borrow_mut().take())
}

/// ws:thread-viewers: a thread edit wants the layer rendered again.
pub fn mark_edited() {
    EDITED.with(|e| e.set(true));
}

/// A field value was changed in the Inspector since the last call.
pub fn take_fields_edited() -> bool {
    EDITED.with(|e| e.replace(false))
}

/// What is wrong with a clip's source. The library's own rules (a `<title>`, `se:desc`, `se:tags` ...)
/// only matter for a library item, so a plain imported page is not nagged about them. Files it names
/// that the project does not have ("missing: image.png") are always listed. `base`: a linked file's folder.
pub fn problems(src: &str, base: &str) -> Vec<String> {
    // a captured web page is a picture of a page: forms, inputs and network links in it are dropped or
    // left alone when it renders, so listing them would only be noise
    let page = html::parse(src).fullpage;
    html::lint(src)
        .into_iter()
        .filter(|l| !l.starts_with("missing ") && !l.starts_with("contains <link>"))
        .filter(|l| !(page && (l.starts_with("contains a network URL") || l.ends_with("(not allowed)"))))
        .chain(html::media_warnings(src, &[], base))
        .collect()
}

/// The preview renders with its longer side at this many pixels.
const PREVIEW_PX: f64 = 480.0;

/// The live picture of a page in an egui texture, with a small player of its own: a time bar,
/// play / pause, frame steps, start / end, loop, "follow the playhead", and a timecode. Held (the
/// default) it shows exactly the moment the frame-exact renderer gives; playing, the page runs in real
/// time from there. The request is built only when its inputs change (a web page can be a few hundred
/// KB to prepare), the texture only when a new picture is in. Keys while it has focus: Space, Left /
/// Right (a frame), Home / End.
pub struct LivePreview {
    /// Which live tab (`html::LIVE_INSPECTOR` / `LIVE_EDITOR`).
    pub slot: usize,
    key: u64,
    page: Option<(String, (u32, u32), f64)>,
    shown: Option<std::sync::Arc<crate::media::Frame>>,
    tex: Option<egui::TextureHandle>,
    /// The held moment (seconds of the layer), or where playing started.
    pub t: f64,
    pub playing: bool,
    /// When playing started (wall clock).
    started: Option<std::time::Instant>,
    pub looping: bool,
    /// Follow the timeline playhead (when it is over the clip).
    pub follow: bool,
    epoch: u64,
    /// The picture shows the current request (false while a change is on its way).
    pub current: bool,
}

impl Default for LivePreview {
    fn default() -> Self {
        LivePreview {
            slot: html::LIVE_INSPECTOR,
            key: 0,
            page: None,
            shown: None,
            tex: None,
            t: 0.0,
            playing: false,
            started: None,
            looping: true,
            follow: false,
            epoch: 0,
            current: false,
        }
    }
}

/// What a live preview shows.
pub struct LiveInput<'a> {
    pub src: &'a str,
    pub fields: &'a [(String, String)],
    pub base: &'a str,
    pub width: u32,
    pub params: &'a [(String, crate::model::Animated)],
    pub duration: f64,
    /// The project's frame rate (frame steps, timecode).
    pub fps: f64,
    /// The playhead in the layer's own time when it is over the clip (for "follow playhead"); None in
    /// the editor window or when the playhead is elsewhere.
    pub playhead: Option<f64>,
}

/// `t` seconds as HH:MM:SS:FF and the frame number, at `fps`.
pub fn timecode(t: f64, fps: f64) -> (String, i64) {
    let fps = if fps > 0.0 { fps } else { 30.0 };
    let f = (t.max(0.0) * fps).round() as i64;
    let per = fps.round().max(1.0) as i64;
    let (s, ff) = (f / per, f % per);
    (format!("{:02}:{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60, ff), f)
}

impl LivePreview {
    pub fn for_slot(slot: usize) -> Self {
        LivePreview { slot, ..Default::default() }
    }

    /// The moment on screen now (the held one, or where real-time playback has got to).
    pub fn now(&self, duration: f64) -> f64 {
        let d = duration.max(0.01);
        match (self.playing, self.started) {
            (true, Some(at)) => {
                let e = self.t + at.elapsed().as_secs_f64();
                if !self.looping {
                    e.min(d)
                } else if e < d {
                    e
                } else {
                    (e - d) % d
                }
            }
            _ => self.t.clamp(0.0, d),
        }
    }

    /// Hold `t` (a seek, a step, the bar): paused, as the user expects after picking a moment.
    pub fn hold(&mut self, t: f64, duration: f64) {
        self.t = t.clamp(0.0, duration.max(0.0));
        self.playing = false;
        self.started = None;
        self.epoch += 1;
    }

    /// Play from the moment on screen.
    pub fn play(&mut self, duration: f64) {
        let mut t = self.now(duration);
        if t >= duration - 1e-6 {
            t = 0.0; // at the end: from the start
        }
        self.t = t;
        self.playing = true;
        self.started = Some(std::time::Instant::now());
        self.epoch += 1;
    }

    pub fn toggle(&mut self, duration: f64) {
        if self.playing {
            let t = self.now(duration);
            self.hold(t, duration);
        } else {
            self.play(duration);
        }
    }

    /// Picture, time bar and buttons. Never blocks; repaints come from new pictures (and the clock
    /// while playing) only.
    pub fn show(&mut self, ui: &mut egui::Ui, height: f32, inp: &LiveInput) {
        use std::hash::{Hash, Hasher};
        let d = inp.duration.max(0.01);
        let step = 1.0 / if inp.fps > 0.0 { inp.fps } else { 30.0 };
        if self.follow {
            if let Some(p) = inp.playhead {
                if self.playing || (p - self.t).abs() > 1e-9 {
                    self.hold(p, d);
                }
            }
        }
        // a non-looping run that reached the end holds there
        if self.playing && !self.looping && self.now(d) >= d {
            self.hold(d, d);
        }
        let mut h = std::hash::DefaultHasher::new();
        (inp.src, inp.fields, inp.base, inp.width).hash(&mut h);
        let key = h.finish();
        if self.key != key || self.page.is_none() {
            let (tpl, page) = html::prepare_in(inp.src, inp.fields, inp.base);
            let size = if inp.width > 0 { (inp.width.clamp(2, 3840), tpl.size.1) } else { tpl.size };
            let scale = (PREVIEW_PX / size.0.max(size.1) as f64).min(1.0);
            if self.page.is_some() && self.playing {
                self.play(d); // an edit while playing goes on from where it is, not from the start
            }
            self.page = Some((page, size, scale));
            self.key = key;
        }
        // the picture
        let w = ui.available_width();
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, height), egui::Sense::click());
        if resp.clicked() {
            resp.request_focus();
        }
        if resp.double_clicked() {
            self.toggle(d);
        }
        if resp.has_focus() {
            let (space, left, right, home, end) = ui.input_mut(|i| {
                let k = |i: &mut egui::InputState, key| i.consume_key(egui::Modifiers::NONE, key);
                (k(i, egui::Key::Space), k(i, egui::Key::ArrowLeft), k(i, egui::Key::ArrowRight), k(i, egui::Key::Home), k(i, egui::Key::End))
            });
            let now = self.now(d);
            if space {
                self.toggle(d);
            } else if left {
                self.hold(now - step, d);
            } else if right {
                self.hold(now + step, d);
            } else if home {
                self.hold(0.0, d);
            } else if end {
                self.hold(d - step, d);
            }
        }
        if let Some((page, size, scale)) = &self.page {
            let params = html::param_values(inp.src, inp.params, self.now(d));
            let req = html::LiveReq {
                page: page.clone(),
                size: *size,
                scale: *scale,
                // playing, params hold their value at the start (a keyframed one moves when held / stepped)
                params,
                at: self.t,
                playing: self.playing,
                looping: self.looping,
                epoch: self.epoch,
                duration: d,
            };
            let ctx = ui.ctx().clone();
            if let Some((f, current)) = html::live_show(self.slot, req, move || ctx.request_repaint()) {
                self.current = current;
                if !self.shown.as_ref().is_some_and(|s| std::sync::Arc::ptr_eq(s, &f)) {
                    let img = egui::ColorImage::from_rgba_unmultiplied([f.width as usize, f.height as usize], &f.rgba);
                    match &mut self.tex {
                        Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                        None => self.tex = Some(ui.ctx().load_texture("html_live", img, egui::TextureOptions::LINEAR)),
                    }
                    self.shown = Some(f);
                }
            }
        }
        checker(ui.painter(), rect);
        if let Some(t) = &self.tex {
            let [tw, th] = t.size().map(|v| v as f32);
            let k = (rect.width() / tw).min(rect.height() / th);
            let r = egui::Rect::from_center_size(rect.center(), egui::vec2(tw * k, th * k));
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            ui.painter().image(t.id(), r, uv, egui::Color32::WHITE);
        }
        if resp.has_focus() {
            ui.painter().rect_stroke(rect, 2.0, ui.visuals().selection.stroke, egui::StrokeKind::Inside);
        }
        // the time bar: click or drag to hold a moment
        let now = self.now(d);
        let (bar, br) = ui.allocate_exact_size(egui::vec2(w, 12.0), egui::Sense::click_and_drag());
        let pal = ui.visuals();
        ui.painter().rect_filled(bar.shrink2(egui::vec2(0.0, 3.0)), 3.0, pal.widgets.inactive.bg_fill);
        let x = bar.left() + bar.width() * (now / d) as f32;
        let filled = egui::Rect::from_min_max(bar.left_top() + egui::vec2(0.0, 3.0), egui::pos2(x, bar.bottom() - 3.0));
        ui.painter().rect_filled(filled, 3.0, pal.selection.bg_fill);
        ui.painter().circle_filled(egui::pos2(x, bar.center().y), 5.0, pal.strong_text_color());
        if let Some(p) = br.interact_pointer_pos().filter(|_| br.clicked() || br.dragged()) {
            let f = ((p.x - bar.left()) / bar.width().max(1.0)).clamp(0.0, 1.0) as f64;
            let t = (f * d / step).round() * step; // on a frame
            self.hold(t, d);
            self.follow = false;
        }
        // the buttons and the readout
        ui.horizontal(|ui| {
            let b = |ui: &mut egui::Ui, text: &str, tip: &str| ui.small_button(text).on_hover_text(tip).clicked();
            if b(ui, "⏮", "Start (Home)") {
                self.hold(0.0, d);
            }
            if b(ui, "◀", "Previous frame (Left)") {
                self.hold(now - step, d);
            }
            if b(ui, if self.playing { "⏸" } else { "▶" }, "Play / pause (Space)") {
                self.toggle(d);
            }
            if b(ui, "▶|", "Next frame (Right)") {
                self.hold(now + step, d);
            }
            if b(ui, "⏭", "End (End)") {
                self.hold(d - step, d);
            }
            if ui.selectable_label(self.looping, "⟲").on_hover_text("Loop").clicked() {
                self.looping = !self.looping;
                self.epoch += 1;
            }
            if inp.playhead.is_some() || self.follow {
                ui.checkbox(&mut self.follow, "Follow playhead")
                    .on_hover_text("Show the moment under the timeline playhead");
            }
            let (tc, f) = timecode(now, inp.fps);
            ui.weak(format!("{tc} · frame {f}"));
        });
        if self.playing {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(33)); // the clock, the bar
        }
    }
}

/// A grey checkerboard (what "transparent" looks like in an editor).
fn checker(p: &egui::Painter, rect: egui::Rect) {
    p.rect_filled(rect, 2.0, egui::Color32::from_rgb(0x2b, 0x2f, 0x3a));
    let s = 12.0;
    let (nx, ny) = ((rect.width() / s).ceil() as i32, (rect.height() / s).ceil() as i32);
    for y in 0..ny {
        for x in (y % 2..nx).step_by(2) {
            let min = rect.min + egui::vec2(x as f32 * s, y as f32 * s);
            let r = egui::Rect::from_min_size(min, egui::vec2(s, s)).intersect(rect);
            p.rect_filled(r, 0.0, egui::Color32::from_rgb(0x36, 0x3b, 0x48));
        }
    }
}

#[derive(Default)]
pub struct HtmlUi {
    pub open: bool,
    /// The OS window has been placed: its size is only set the frame it opens (a builder that changes
    /// later is a resize command, and would fight the user's own drag).
    placed: bool,
    pub name: String,
    /// The folder of the clip's linked file (relative links in the page resolve there).
    pub base: String,
    /// The layout width (0 = the template's).
    pub width: u32,
    pub mouse: Option<crate::model::Mouse>,
    /// The page's param tracks (the preview shows them at the scrub time).
    pub params: crate::engine::html::Params,
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
    /// The project's frame rate (the preview's frame steps).
    pub fps: f64,
    pub(crate) live: LivePreview,
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
        if self.live.slot != html::LIVE_EDITOR {
            self.live = LivePreview::for_slot(html::LIVE_EDITOR);
        }
        let t = self.t;
        self.live.hold(t, self.duration);
        self.lint = problems(src, &self.base);
        self.error.clear();
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
        st.placed = false;
        return resp;
    }
    // a real window of its own: resize it, maximise it, drag it to another monitor, F11 for full screen
    let mut builder = egui::ViewportBuilder::default()
        .with_title(format!("HTML Editor - {}", if st.name.is_empty() { "layer" } else { &st.name }))
        .with_min_inner_size([640.0, 420.0]);
    if !st.placed {
        builder = builder.with_inner_size([1040.0, 660.0]);
        st.placed = true;
    }
    let mut close = false;
    ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("html_editor_window"), builder, |ctx, _class| {
        close = ctx.input(|i| i.viewport().close_requested());
        egui::CentralPanel::default().show(ctx, |ui| body(ui, st, &mut resp));
    });
    if close {
        st.open = false;
    }
    resp
}

/// What is inside the editor, in the window or the OS window.
fn body(ui: &mut egui::Ui, st: &mut HtmlUi, resp: &mut EditorResponse) {
    ui.horizontal(|ui| {
        let ctx = ui.ctx().clone();
        let full = ctx.input(|i| i.viewport().fullscreen).unwrap_or(false);
        let key = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F11));
        if ui.button(if full { "Exit full screen" } else { "Full screen" }).on_hover_text("F11").clicked() || key {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!full));
        }
        ui.weak("Ctrl+Space: suggestions · Tab: accept · the window can be maximised or dragged to another monitor");
    });
    ui.columns(2, |cols| {
        let ui = &mut cols[0];
        ui.weak("HTML with its <style>. CSS animations only - no JavaScript. Fields: {{name}}.");
        let rows = ((ui.available_height() / 14.0) as usize).clamp(12, 60);
        if st.src.len() > BIG_SOURCE {
            ui.label(format!(
                "{} KB of HTML (a saved web page) is too big for a text box. Edit the linked file in your code editor; \
                 this window's preview follows it.",
                st.src.len() / 1024
            ));
        } else {
            egui::ScrollArea::both().id_salt("html_src").show(ui, |ui| {
                let r = crate::ui::html_code::code_edit(ui, egui::Id::new("html_editor_source"), &mut st.src, rows);
                if r.changed() {
                    st.lint = problems(&st.src, &st.base);
                }
            });
        }

        let ui = &mut cols[1];
        ui.horizontal(|ui| {
            resp.apply = ui.button("Apply").on_hover_text("Use this source for the clip and render it").clicked();
        });
        let w = ui.available_width();
        let src = st.src.clone();
        let inp = LiveInput {
            src: &src,
            fields: &st.fields,
            base: &st.base,
            width: st.width,
            params: &st.params,
            duration: st.duration,
            fps: st.fps,
            playhead: None,
        };
        st.live.show(ui, w * 9.0 / 16.0, &inp);
        st.t = st.live.now(st.duration);
        if !st.error.is_empty() {
            ui.colored_label(ui.visuals().error_fg_color, &st.error);
        }
        egui::ScrollArea::vertical().id_salt("html_lint").max_height(110.0).show(ui, |ui| {
            if st.lint.is_empty() {
                ui.weak("No problems found.");
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
}

// ---------------------------------------------------------------------------------------------------
// the Inspector's Fields section

pub(crate) fn hex_rgba(s: &str) -> [u8; 4] {
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

pub(crate) fn rgba_hex(c: [u8; 4]) -> String {
    match c[3] {
        255 => format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]),
        a => format!("#{:02x}{:02x}{:02x}{a:02x}", c[0], c[1], c[2]),
    }
}

/// The template of an HTML asset's source, parsed once per change of the text (it is read every frame
/// the clip is selected, and a saved web page can be megabytes).
fn template_of(aid: Id, src: &str) -> std::rc::Rc<html::Template> {
    use std::hash::{Hash, Hasher};
    thread_local! {
        static CACHE: RefCell<Option<(Id, u64, std::rc::Rc<html::Template>)>> = const { RefCell::new(None) };
    }
    let mut h = std::hash::DefaultHasher::new();
    src.hash(&mut h);
    let key = h.finish();
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if let Some((a, k, t)) = c.as_ref() {
            if *a == aid && *k == key {
                return t.clone();
            }
        }
        let t = std::rc::Rc::new(html::parse(src));
        *c = Some((aid, key, t.clone()));
        t
    })
}

/// Does this clip's page declare `se:param`s?
pub fn has_params(project: &Project, clip: Id) -> bool {
    let Some(a) = project.clip(clip).and_then(|c| project.asset(c.asset)) else { return false };
    a.html.as_ref().is_some_and(|h| !template_of(a.id, &h.src).params.is_empty())
}

/// A source longer than this is not drawn in a text box (colouring and editing it every frame would
/// make the app crawl): a saved web page. It is still linked, rendered and editable in a code editor.
const BIG_SOURCE: usize = 250_000;

// ---------------------------------------------------------------------------------------------------
// the Web Page Snapshot window

/// The window that brings a real web page in: from an address (Edge opens it as a visitor sees it), or
/// from the user's own browser (an extension saves what is on screen into Downloads, and Simple Editor
/// picks the file up).
#[derive(Default)]
pub struct WebUi {
    pub open: bool,
    /// 0 = from a web address, 1 = from my browser.
    pub tab: u8,
    pub url: String,
    pub width: u32,
    pub status: String,
    /// 0..1 while a snapshot is being taken.
    pub busy: Option<f32>,
}

#[derive(Default)]
pub struct WebResponse {
    /// Take a snapshot of (address, width).
    pub capture: Option<(String, u32)>,
    pub import_file: bool,
    pub save_extension: bool,
}

pub fn show_web(ctx: &egui::Context, st: &mut WebUi) -> WebResponse {
    let mut resp = WebResponse::default();
    if !st.open {
        return resp;
    }
    let mut open = true;
    egui::Window::new("Web Page Snapshot").open(&mut open).default_width(460.0).show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut st.tab, 0, "From a web address");
            ui.selectable_value(&mut st.tab, 1, "From my browser");
        });
        ui.separator();
        if st.tab == 0 {
            ui.label("Simple Editor opens the page itself and keeps the real HTML and CSS (no scripts), as a visitor who is not signed in sees it.");
            let r = ui.add(egui::TextEdit::singleline(&mut st.url).hint_text("roblox.com or https://www.youtube.com/…").desired_width(f32::INFINITY));
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.horizontal(|ui| {
                ui.label("Page width");
                ui.add(egui::DragValue::new(&mut st.width).range(320..=3840).speed(4.0).suffix(" px"));
                for (name, px) in [("Phone", 390), ("Short", 1080), ("Desktop", 1920)] {
                    if ui.small_button(name).on_hover_text(format!("{px} px")).clicked() {
                        st.width = px;
                    }
                }
            });
            ui.horizontal(|ui| {
                let busy = st.busy.is_some();
                let ready = !busy && !st.url.trim().is_empty();
                if ui.add_enabled(ready, egui::Button::new("Take snapshot")).clicked() || (enter && ready) {
                    resp.capture = Some((st.url.trim().to_string(), st.width));
                }
                if let Some(p) = st.busy {
                    ui.add(egui::ProgressBar::new(p).desired_width(160.0));
                }
            });
        } else {
            ui.label("Snapshot the page exactly as it is on your screen (signed in, scrolled, menus open):");
            ui.label("1. Install the extension once (it is a folder, 30 seconds).");
            if ui.button("Save the extension to a folder…").on_hover_text("Writes the three extension files and opens the folder").clicked() {
                resp.save_extension = true;
            }
            ui.label("2. In your browser (Brave, Chrome, Edge) open brave://extensions, turn on Developer mode, Load unpacked, pick that folder.");
            ui.label("3. On any page click the extension's button (or Alt+Shift+S). The page appears here.");
            ui.weak("Waiting for a page from your browser (your Downloads folder is watched)…");
            ui.separator();
            ui.label("Or save a page yourself (Ctrl+S: Webpage, Single File or Complete) and import the file:");
            if ui.button("Import a saved page…").clicked() {
                resp.import_file = true;
            }
        }
        if !st.status.is_empty() {
            ui.separator();
            ui.label(&st.status);
        }
    });
    st.open = open;
    resp
}

/// The violet of the `</>` mark (the timeline paints an HTML clip in the same colour family).
const MARK: egui::Color32 = egui::Color32::from_rgb(104, 82, 184);

/// The Inspector's HTML section: what this clip is (code, not footage), its file, the source itself
/// and a box to ask the assistant. The source box is bound to the project, not a copy: when an agent or
/// the linked file changes the source, it changes here as you watch; typing is one undo step and a
/// re-render (a moment after you stop). True when the project changed.
pub fn source_section(ui: &mut egui::Ui, project: &mut Project, clip: Id, undo: &mut dyn FnMut(&Project)) -> bool {
    let Some(aid) = project.clip(clip).map(|c| c.asset) else { return false };
    let Some(h) = project.asset(aid).and_then(|a| a.html.clone()) else { return false };
    let linked = !h.file.is_empty();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(" </> ").monospace().strong().color(egui::Color32::WHITE).background_color(MARK));
        if linked {
            let name = std::path::Path::new(&h.file).file_name().map(|n| n.to_string_lossy().into_owned());
            ui.add(egui::Label::new(format!("synced with {}", name.unwrap_or_default())).truncate())
                .on_hover_text(format!("{}
Save in your code editor and the clip updates; edit here and the file follows.", h.file));
        } else {
            ui.weak("Stored in this project. Link a file to edit it elsewhere.");
        }
    });
    ui.horizontal_wrapped(|ui| {
        if ui
            .button("Open in code editor")
            .on_hover_text("Write the source to a .html file and open it in your code editor (VS Code, else Notepad); it stays linked")
            .clicked()
        {
            push_op(ui.ctx(), Op::OpenExternal(clip));
        }
        if linked {
            if ui.button("Show file").on_hover_text(&h.file).clicked() {
                push_op(ui.ctx(), Op::Reveal(clip));
            }
            if ui.button("Unlink").on_hover_text("Keep the source in the project only").clicked() {
                push_op(ui.ctx(), Op::Unlink(clip));
            }
        } else {
            if ui.button("Link .html file…").on_hover_text("Use an existing file as the source and keep them in sync").clicked() {
                push_op(ui.ctx(), Op::LinkPick(clip));
            }
            if ui.button("Save as .html…").on_hover_text("Write the source to a file and keep them in sync").clicked() {
                push_op(ui.ctx(), Op::SaveAs(clip));
            }
        }
        let tall = project.asset(aid).is_some_and(|a| a.height > project.height);
        if tall
            && ui
                .button("Scroll the page")
                .on_hover_text("The page is taller than the frame: keyframe it to scroll from its top to its bottom (then adjust the keys like any others)")
                .clicked()
        {
            push_op(ui.ctx(), Op::Scroll(clip));
        }
        if ui
            .button("Edit HTML")
            .on_hover_text("The editor in a window of its own: code, live preview, scrub bar; maximise it or drag it to another monitor")
            .clicked()
        {
            request_edit(ui.ctx(), clip);
        }
    });

    // a web page (no se:size): the width it is laid out at, since a site has one layout per width
    let mut width_changed = None;
    if template_of(aid, &h.src).fullpage {
        ui.horizontal(|ui| {
            ui.label("Page width");
            let mut w = if h.width == 0 { template_of(aid, &h.src).size.0 } else { h.width };
            let r = ui.add(egui::DragValue::new(&mut w).range(320..=3840).speed(4.0).suffix(" px"));
            let mut set = r.changed().then_some(w);
            for (name, px) in [("Phone", 390), ("Short", 1080), ("Desktop", 1920)] {
                if ui.small_button(name).on_hover_text(format!("{px} px")).clicked() {
                    set = Some(px);
                }
            }
            width_changed = set;
        });
    }
    // the source: bound to the project each frame, so an agent's edit shows up while you watch
    let mut src = h.src.clone();
    let mut typed = false;
    if src.len() > BIG_SOURCE {
        ui.weak(format!("{} KB of HTML (a saved web page): too big for this box - use Open in code editor.", src.len() / 1024));
    } else {
        egui::ScrollArea::both().id_salt(("html_source", clip)).max_height(260.0).show(ui, |ui| {
            let id = ui.make_persistent_id(("html_source_edit", clip));
            typed = crate::ui::html_code::code_edit(ui, id, &mut src, 10).changed();
        });
    }
    // what breaks the library rules, parsed once per change of the text
    let key = {
        use std::hash::{Hash, Hasher};
        let mut s = std::hash::DefaultHasher::new();
        src.hash(&mut s);
        s.finish()
    };
    let lid = ui.make_persistent_id(("html_lint", clip));
    let lint = match ui.data(|d| d.get_temp::<(u64, Vec<String>)>(lid)) {
        Some((k, l)) if k == key => l,
        _ => {
            let l = problems(&src, &h.file.rsplit_once(['\\', '/']).map(|(d, _)| d.to_string()).unwrap_or_default());
            ui.data_mut(|d| d.insert_temp(lid, (key, l.clone())));
            l
        }
    };
    for l in lint.iter().take(3) {
        ui.colored_label(ui.visuals().warn_fg_color, l);
    }

    // ask the assistant about this clip
    let aid_ask = ui.make_persistent_id(("html_ask", clip));
    let mut ask: String = ui.data(|d| d.get_temp(aid_ask)).unwrap_or_default();
    ui.horizontal(|ui| {
        let r = ui.add(
            egui::TextEdit::singleline(&mut ask).hint_text("Ask the assistant to change it…").desired_width(ui.available_width() - 48.0),
        );
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (ui.button("Ask").clicked() || enter) && !ask.trim().is_empty() {
            push_op(ui.ctx(), Op::Ask(clip, std::mem::take(&mut ask)));
        }
    });
    ui.data_mut(|d| d.insert_temp(aid_ask, ask));

    if let Some(w) = width_changed.filter(|w| *w != h.width) {
        undo(project);
        if let Some(x) = project.asset_mut(aid).and_then(|a| a.html.as_mut()) {
            x.width = w;
        }
        EDITED.with(|e| e.set(true));
        return true;
    }
    if !typed || src == h.src {
        return false;
    }
    // one undo step per burst of typing: a pause of 1.5 s starts a new one
    let (now, tid) = (ui.input(|i| i.time), ui.make_persistent_id(("html_undo_at", clip)));
    if ui.data(|d| d.get_temp::<f64>(tid)).is_none_or(|last| now - last > 1.5) {
        undo(project);
    }
    ui.data_mut(|d| d.insert_temp(tid, now));
    let tpl = html::parse(&src);
    if let Some(a) = project.asset_mut(aid).and_then(|a| a.html.as_mut()) {
        a.src = src;
        a.fields.retain(|k, _| tpl.fields.iter().any(|f| f.name == *k)); // a field you deleted loses its value
    }
    EDITED.with(|e| e.set(true));
    true
}

/// Keys for one click at `at`: the button goes down and, 0.18 s later, up (a step: it holds each value).
fn add_click(down: &mut crate::model::Animated, at: f64) {
    use crate::model::{Ease, Keyframe};
    let mut keys: Vec<Keyframe> = down.keys.iter().filter(|k| k.t < at - 0.05 || k.t > at + 0.3).cloned().collect();
    if keys.is_empty() {
        keys.push(Keyframe { t: 0.0, v: 0.0, ease: Ease::Hold });
    }
    keys.extend([
        Keyframe { t: at, v: 1.0, ease: Ease::Hold },
        Keyframe { t: at + 0.18, v: 0.0, ease: Ease::Hold },
    ]);
    keys.sort_by(|a, b| a.t.total_cmp(&b.t));
    keys.dedup_by(|b, a| (a.t - b.t).abs() < 1e-6);
    down.keys = keys;
}

/// The Inspector's Mouse section: a fake mouse over the page. In the render a real mouse is moved over
/// it and pressed, so `:hover`, `:active` and the CSS transitions they start run exactly as in a browser,
/// and a cursor is drawn. Position (CSS pixels of the page's viewport) and the button are keyframed like
/// any property; "Click here" adds a press and release at the playhead. True when the project changed.
pub fn mouse_section(
    ui: &mut egui::Ui,
    project: &mut Project,
    clip: Id,
    at: f64,
    palette: &crate::theme::Palette,
    undo: &mut dyn FnMut(&Project),
) -> bool {
    let Some(aid) = project.clip(clip).map(|c| c.asset) else { return false };
    let Some(h) = project.asset(aid).and_then(|a| a.html.as_ref()) else { return false };
    let tpl = template_of(aid, &h.src);
    let (vw, vh) = (if h.width > 0 { h.width } else { tpl.size.0 }, tpl.size.1);
    let mut mouse = h.mouse.clone();
    let mut g = crate::ui::Gesture::default();
    let mut on = mouse.is_some();
    if ui.checkbox(&mut on, "Fake mouse").on_hover_text("Move a mouse over the page: hover, press, click").changed() {
        g.click();
        mouse = on.then(|| crate::model::Mouse::at(vw as f64 * 0.5, vh as f64 * 0.35));
    }
    if let Some(m) = mouse.as_mut() {
        ui.weak("Keyframe where it is and when it is pressed. Hover styles and their transitions play for real; links go nowhere.");
        egui::Grid::new("html_mouse").num_columns(2).show(ui, |ui| {
            for (label, a, max) in [("Mouse X", &mut m.x, vw as f64), ("Mouse Y", &mut m.y, vh as f64), ("Pressed", &mut m.down, 1.0)] {
                ui.label(label);
                ui.horizontal(|ui| {
                    let mut v = a.at(at);
                    let dv = egui::DragValue::new(&mut v).range(0.0..=max).speed(if max > 1.0 { 2.0 } else { 1.0 });
                    let r = ui.add(if max > 1.0 { dv } else { dv.fixed_decimals(0) });
                    if r.changed() {
                        a.set_at(at, v);
                    }
                    g.note(&r);
                    crate::ui::key_buttons(ui, a, at, palette, &mut g, label, &[]);
                });
                ui.end_row();
            }
        });
        if ui.button("Click here").on_hover_text("Press the button down and up at the playhead (0.18 s)").clicked() {
            add_click(&mut m.down, at);
            g.click();
        }
    }
    if !g.changed || mouse == h.mouse {
        return false;
    }
    if g.start {
        undo(project);
    }
    if let Some(x) = project.asset_mut(aid).and_then(|a| a.html.as_mut()) {
        x.mouse = mouse;
    }
    EDITED.with(|e| e.set(true));
    true
}

/// The Inspector's Parameters section: one row per `se:param` of the page - a number you can keyframe
/// like any other property (the ◆ at the playhead; right-click for easing and the rest). `at` is the
/// playhead in the layer's own time. True when the project changed.
pub fn params_section(
    ui: &mut egui::Ui,
    project: &mut Project,
    clip: Id,
    at: f64,
    palette: &crate::theme::Palette,
    undo: &mut dyn FnMut(&Project),
) -> bool {
    let Some(aid) = project.clip(clip).map(|c| c.asset) else { return false };
    let Some(h) = project.asset(aid).and_then(|a| a.html.as_ref()) else { return false };
    let tpl = template_of(aid, &h.src);
    if tpl.params.is_empty() {
        return false;
    }
    let mut tracks = h.params.clone();
    let mut g = crate::ui::Gesture::default();
    ui.weak("Each is a CSS variable: var(--name). Keyframe it to animate the layer.");
    egui::Grid::new("html_params").num_columns(2).show(ui, |ui| {
        for p in &tpl.params {
            ui.label(crate::engine::shaderlib::label(&p.name));
            let default = p.default.trim().parse::<f64>().unwrap_or(0.0);
            let a = tracks.entry(p.name.clone()).or_insert_with(|| crate::model::Animated::new(default));
            ui.horizontal(|ui| {
                let mut v = a.at(at);
                let mut dv = egui::DragValue::new(&mut v);
                if let (Some(lo), Some(hi)) = (p.min, p.max) {
                    dv = dv.range(lo..=hi).speed(((hi - lo) / 300.0).max(0.001));
                }
                let r = ui.add(dv);
                if r.changed() {
                    a.set_at(at, v);
                }
                g.note(&r);
                crate::ui::key_buttons(ui, a, at, palette, &mut g, &p.name, &[]);
            });
            ui.end_row();
        }
    });
    // a param left at its default with no keys is not stored
    tracks.retain(|n, a| a.is_animated() || tpl.params.iter().any(|p| p.name == *n && (a.value - p.default.trim().parse::<f64>().unwrap_or(0.0)).abs() > 1e-9));
    if !g.changed || tracks == h.params {
        return false;
    }
    if g.start {
        undo(project);
    }
    if let Some(x) = project.asset_mut(aid).and_then(|a| a.html.as_mut()) {
        x.params = tracks;
    }
    EDITED.with(|e| e.set(true));
    true
}

/// The font under the pointer in an open font list: (clip, field, family). The Inspector preview shows
/// it until the pointer leaves the list.
struct HoverFont(Cell<Option<(Id, String, String)>>);

impl HoverFont {
    fn set(&self, v: Option<(Id, String, String)>) {
        let old = self.0.take();
        self.0.set(v.or(old));
    }
    fn get(&self) -> Option<(Id, String, String)> {
        let v = self.0.take();
        self.0.set(v.clone());
        v
    }
    fn clear_field(&self, clip: Id, field: &str) {
        let v = self.0.take();
        self.0.set(v.filter(|(c, f, _)| !(*c == clip && f == field)));
    }
    /// Another clip's Fields are drawn: its hover is gone.
    fn set_if_some_differs(&self, clip: Id) {
        let v = self.0.take();
        self.0.set(v.filter(|(c, _, _)| *c == clip));
    }
}

thread_local! {
    static HOVER_FONT: HoverFont = const { HoverFont(Cell::new(None)) };
    static INSPECTOR_LIVE: RefCell<LivePreview> = RefCell::new(LivePreview::for_slot(html::LIVE_INSPECTOR));
    /// `html.preview` asks: (clip, hold this second, play / pause).
    static PREVIEW_ASK: RefCell<Option<(Id, Option<f64>, Option<bool>)>> = const { RefCell::new(None) };
    /// The Inspector preview's state as last drawn: (clip, time, playing).
    static PREVIEW_STATE: Cell<Option<(Id, f64, bool)>> = const { Cell::new(None) };
}

/// `html.preview`: set the Inspector preview of `clip` (shown when the clip is selected).
pub fn ask_preview(clip: Id, t: Option<f64>, playing: Option<bool>) {
    PREVIEW_ASK.with(|a| *a.borrow_mut() = Some((clip, t, playing)));
}

/// The Inspector preview's (clip, time, playing) as last drawn.
pub fn preview_state() -> Option<(Id, f64, bool)> {
    PREVIEW_STATE.with(Cell::get)
}

/// The Inspector's live preview of an HTML clip: the page itself, so a field or param edit shows at
/// once while the frame-exact render (what the timeline and export use) catches up. `at`: the clip's own
/// time under the playhead (held there, the same moment the timeline shows); None = play live.
pub fn live_section(ui: &mut egui::Ui, project: &Project, clip: Id, at: Option<f64>) {
    let fps = if project.fps > 0.0 { project.fps } else { 30.0 };
    let Some(a) = project.clip(clip).and_then(|c| project.asset(c.asset)) else { return };
    let Some(h) = a.html.as_ref() else { return };
    let tpl = template_of(a.id, &h.src);
    let base = std::path::Path::new(&h.file).parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
    let (mut fields, params) = (h.field_list(), html::params_of(&h.src, &h.params));
    if let Some((_, f, fam)) = HOVER_FONT.with(HoverFont::get).filter(|(c, _, _)| *c == clip) {
        fields.retain(|(k, _)| *k != f);
        fields.push((f, fam));
    }
    let w = ui.available_width();
    let (sw, sh) = (tpl.size.0.max(1) as f32, tpl.size.1.max(1) as f32);
    let hgt = (w * sh / sw).clamp(60.0, 240.0);
    let d = h.duration;
    let inp = LiveInput { src: &h.src, fields: &fields, base: &base, width: h.width, params: &params, duration: d, fps, playhead: at };
    INSPECTOR_LIVE.with(|l| {
        let mut l = l.borrow_mut();
        if PREVIEW_STATE.with(Cell::get).map(|s| s.0) != Some(clip) {
            // another clip: start on its representative moment, held
            l.hold(tpl.thumb_at().min(d), d);
            l.follow = false;
        }
        if let Some((c, t, play)) = PREVIEW_ASK.with(|a| a.borrow_mut().take_if(|x| x.0 == clip)) {
            debug_assert_eq!(c, clip);
            if let Some(t) = t {
                l.hold(t, d);
                l.follow = false;
            }
            match play {
                Some(true) if !l.playing => l.play(d),
                Some(false) if l.playing => {
                    let now = l.now(d);
                    l.hold(now, d);
                }
                _ => {}
            }
        }
        l.show(ui, hgt, &inp);
        PREVIEW_STATE.with(|s| s.set(Some((clip, l.now(d), l.playing))));
    });
}

/// One control per `se:field` of the clip's template, plus "Edit HTML…". One undo step per committed
/// change; the re-render is the app's (`take_fields_edited`). True when the project changed.
pub fn fields_section(
    ui: &mut egui::Ui,
    project: &mut Project,
    clip: Id,
    fonts: &[String],
    undo: &mut dyn FnMut(&Project),
) -> bool {
    HOVER_FONT.with(|h| h.set_if_some_differs(clip));
    let Some(aid) = project.clip(clip).map(|c| c.asset) else { return false };
    let Some(h) = project.asset(aid).and_then(|a| a.html.clone()) else { return false };
    let tpl = template_of(aid, &h.src);
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
                html::FieldType::Number | html::FieldType::Range | html::FieldType::Radius | html::FieldType::Size => {
                    let mut n = cur.trim().trim_end_matches(|c: char| c.is_ascii_alphabetic() || c == '%').parse::<f64>().unwrap_or(0.0);
                    let unit = f.unit();
                    let r = match (f.ty, f.bounds()) {
                        // a range kind with bounds: a slider (with its own number box)
                        (t, (Some(a), Some(b))) if t != html::FieldType::Number && a < b => {
                            let mut sl = egui::Slider::new(&mut n, a..=b).suffix(unit.clone());
                            if f.step() > 0.0 {
                                sl = sl.step_by(f.step());
                            }
                            ui.add(sl)
                        }
                        (_, (lo, hi)) => {
                            let mut dv = egui::DragValue::new(&mut n).suffix(unit.clone());
                            if let (Some(a), Some(b)) = (lo, hi) {
                                dv = dv.range(a..=b).speed(((b - a) / 300.0).max(0.01));
                            }
                            ui.add(dv)
                        }
                    };
                    g.note(&r);
                    if r.changed() {
                        // stored as the number; the unit is the template's
                        let v = html::field_value(f, &[(f.name.clone(), n.to_string())]);
                        vals.insert(name, v.trim_end_matches(unit.as_str()).to_string());
                    }
                }
                html::FieldType::Font => {
                    let mut pick: Option<String> = None;
                    let r = egui::ComboBox::from_id_salt(("html_font", clip, &f.name)).selected_text(cur.clone()).height(320.0).show_ui(ui, |ui| {
                        if fonts.is_empty() {
                            ui.weak("Loading fonts…");
                        }
                        if !fonts.iter().any(|x| *x == cur) {
                            let _ = ui.selectable_label(true, cur.as_str());
                        }
                        for fam in fonts {
                            let r = ui.selectable_label(*fam == cur, fam);
                            if r.hovered() {
                                // the live preview shows the layer in the font under the pointer
                                HOVER_FONT.with(|h| h.set(Some((clip, f.name.clone(), fam.clone()))));
                            }
                            if r.clicked() {
                                pick = Some(fam.clone());
                            }
                        }
                    });
                    if r.inner.is_none() {
                        HOVER_FONT.with(|h| h.clear_field(clip, &f.name));
                    }
                    if let Some(p) = pick {
                        g.click();
                        vals.insert(name, p);
                    }
                }
                html::FieldType::Select => {
                    let opts = f.options();
                    let mut pick: Option<String> = None;
                    egui::ComboBox::from_id_salt(("html_select", clip, &f.name)).selected_text(cur.clone()).show_ui(ui, |ui| {
                        for (label, _) in &opts {
                            if ui.selectable_label(label.eq_ignore_ascii_case(&cur), label).clicked() {
                                pick = Some(label.clone());
                            }
                        }
                    });
                    if let Some(p) = pick {
                        g.click();
                        vals.insert(name, p);
                    }
                }
                html::FieldType::Toggle => {
                    let mut on = html::toggle_on(&cur);
                    if ui.checkbox(&mut on, "").changed() {
                        g.click();
                        vals.insert(name, if on { "on" } else { "off" }.into());
                    }
                }
                html::FieldType::Shadow | html::FieldType::Outline => {
                    let (mut size, colour) = html::size_colour(&cur).unwrap_or((0.0, "#000000".into()));
                    let mut c = hex_rgba(&colour);
                    ui.horizontal(|ui| {
                        let max = if f.ty == html::FieldType::Shadow { 120.0 } else { 40.0 };
                        let r = ui.add(egui::Slider::new(&mut size, 0.0..=max).suffix("px"));
                        g.note(&r);
                        let rc = ui.color_edit_button_srgba_unmultiplied(&mut c);
                        g.note(&rc);
                        if r.changed() || rc.changed() {
                            vals.insert(name.clone(), format!("{} {}", (size * 10.0).round() / 10.0, rgba_hex(c)));
                        }
                    });
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
    if ui.button("Edit HTML").on_hover_text("Open the source in the HTML editor window").clicked() {
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
        assert!(!ctx.has_requested_repaint(), "idle HTML editor requested a repaint");
    }

    /// A held live preview drawn again and again with nothing changed: the same request every frame
    /// (no seek, no swap - the blink), and no frames asked for; playing asks for its clock.
    #[test]
    fn assert_no_idle_repaint_html_live_preview() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let src = html::icon_page("x", "<svg></svg>");
        let mut lp = LivePreview::for_slot(html::LIVE_EDITOR);
        lp.hold(1.25, 4.0);
        let epoch = lp.epoch;
        let inp = LiveInput { src: &src, fields: &[], base: "", width: 0, params: &[], duration: 4.0, fps: 60.0, playhead: None };
        for _ in 0..30 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| lp.show(ui, 120.0, &inp));
            });
        }
        assert_eq!((lp.epoch, lp.t, lp.playing), (epoch, 1.25, false), "held: nothing moved");
        assert!(!ctx.has_requested_repaint(), "a held live preview requested a repaint");
        lp.play(4.0);
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| lp.show(ui, 120.0, &inp));
        });
        assert!(ctx.has_requested_repaint(), "playing, the clock moves");
    }

    #[test]
    fn the_preview_player_steps_frames_loops_and_reads_out_timecode() {
        let mut lp = LivePreview::default();
        lp.hold(2.0, 4.0);
        assert_eq!(lp.now(4.0), 2.0);
        lp.hold(9.0, 4.0);
        assert_eq!(lp.now(4.0), 4.0, "held inside the layer");
        assert_eq!(timecode(1.5, 60.0), ("00:00:01:30".to_string(), 90));
        assert_eq!(timecode(61.0 + 1.0 / 30.0, 30.0).0, "00:01:01:01");
        lp.hold(4.0, 4.0);
        lp.play(4.0);
        assert!(lp.playing && lp.t == 0.0, "play at the end starts again from the start");
    }

    /// The Inspector's HTML section: drawing it is not an edit and asks for no frames; the source it
    /// shows is the project's (an agent's change is visible at once).
    #[test]
    fn assert_no_idle_repaint_html_source_section() {
        use crate::model::{Asset, ClipKind, HtmlSource};
        let mut p = Project::new();
        let src = html::icon_page("x", "<svg></svg>");
        let aid = p.add_asset(Asset {
            kind: ClipKind::Image,
            html: Some(HtmlSource { name: "x".into(), src: src.clone(), duration: 4.0, ..Default::default() }),
            ..crate::engine::import::placeholder("C:/x/pending.png")
        });
        let cid = p.insert_asset_clips(aid, 0.0, None)[0];
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let mut undos = 0;
        for _ in 0..30 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    assert!(!source_section(ui, &mut p, cid, &mut |_| undos += 1));
                });
            });
        }
        assert_eq!(undos, 0);
        assert!(!take_fields_edited() && take_ops().is_empty());
        assert!(!ctx.has_requested_repaint(), "idle HTML source section requested a repaint");
        // an agent rewrites the source: the next frame shows it (and still asks for nothing)
        p.asset_mut(aid).unwrap().html.as_mut().unwrap().src = "<p>changed</p>".into();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                assert!(!source_section(ui, &mut p, cid, &mut |_| undos += 1));
            });
        });
        assert_eq!(p.asset(aid).unwrap().html.as_ref().unwrap().src, "<p>changed</p>");
    }

    /// The snapshot window and the Mouse section: drawing them is not an edit and asks for no frames.
    #[test]
    fn assert_no_idle_repaint_web_snapshot_and_mouse() {
        use crate::model::{Asset, ClipKind, HtmlSource};
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let mut web = WebUi { open: true, width: 1080, ..Default::default() };
        for tab in [0, 1] {
            web.tab = tab;
            for _ in 0..30 {
                let _ = ctx.run(egui::RawInput::default(), |ctx| {
                    let r = show_web(ctx, &mut web);
                    assert!(r.capture.is_none() && !r.import_file && !r.save_extension);
                });
            }
        }
        assert!(!ctx.has_requested_repaint(), "idle snapshot window requested a repaint");
        let mut p = Project::new();
        let aid = p.add_asset(Asset {
            kind: ClipKind::Image,
            html: Some(HtmlSource { name: "x".into(), src: html::icon_page("x", "<svg></svg>"), duration: 4.0, ..Default::default() }),
            ..crate::engine::import::placeholder("C:/x/pending.png")
        });
        let cid = p.insert_asset_clips(aid, 0.0, None)[0];
        let pal = crate::theme::palette(&ctx);
        for _ in 0..30 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    assert!(!mouse_section(ui, &mut p, cid, 0.0, &pal, &mut |_| {}));
                });
            });
        }
        assert!(!ctx.has_requested_repaint(), "idle mouse section requested a repaint");
    }

    #[test]
    fn a_click_is_a_press_and_a_release_and_clicks_do_not_pile_up() {
        let mut a = crate::model::Animated::new(0.0);
        add_click(&mut a, 2.0);
        let on = |a: &crate::model::Animated, t: f64| a.at(t) >= 0.5;
        assert!(!on(&a, 1.9) && on(&a, 2.0) && on(&a, 2.1) && !on(&a, 2.2));
        add_click(&mut a, 2.05); // the same click again (a double press on the same spot): replaced
        assert_eq!(a.keys.iter().filter(|k| k.v == 1.0).count(), 1);
        add_click(&mut a, 5.0);
        assert!(on(&a, 5.1) && !on(&a, 4.0) && on(&a, 2.05));
    }

    #[test]
    fn a_plain_page_is_not_nagged_about_library_metadata() {
        assert!(problems("<html><body><p>hi</p></body></html>", "").is_empty());
        assert!(!problems("<html><body><script>x()</script></body></html>", "").is_empty(), "script is still a problem");
    }

    /// Fields: drawing them is not an edit (no undo, no re-render); a stored value reaches the page.
    #[test]
    fn fields_section_is_quiet_until_edited() {
        use crate::model::{Asset, ClipKind, HtmlSource};
        let mut p = Project::new();
        let src = html::icon_page("x", "<svg></svg>");
        let aid = p.add_asset(Asset {
            kind: ClipKind::Image,
            html: Some(HtmlSource { name: "x".into(), src, fields: Default::default(), duration: 4.0, ..Default::default() }),
            ..crate::engine::import::placeholder("C:/x/pending.png")
        });
        let cid = p.insert_asset_clips(aid, 0.0, None)[0];
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let mut undos = 0;
        // nothing touched: no edit
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                assert!(!fields_section(ui, &mut p, cid, &[], &mut |_| undos += 1));
            });
        });
        assert_eq!(undos, 0);
        assert!(!take_fields_edited());
        let a = p.asset_mut(aid).unwrap().html.as_mut().unwrap();
        a.fields.insert("color".into(), "#ff0000".into());
        assert!(html::prepare(&a.src, &a.field_list()).1.contains("color: #ff0000;"));
    }
}
