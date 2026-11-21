//! ---- feat/plugins ----
//! Plugin panels: a Luau script draws a dockable pane through a small immediate-mode API -
//!
//! ```lua
//! local count = 0
//! function draw()
//!   ui.heading("Hello")
//!   if ui.button("Add a marker") then tool("markers.add", {}) end
//!   count = ui.slider("Count", count, 0, 10)
//! end
//! ```
//!
//! Each run of `draw()` records a tree of widgets (`Node`); the app then draws that tree with egui (form
//! rows in two aligned columns, like every other panel). What the user did to a widget (a click, a new
//! slider value, typed text) is handed to the next run as that widget's return value, so the script reads
//! like immediate-mode code while the Lua never touches egui. `draw()` runs only when something could have
//! changed (an interaction, an edit to the project, a download landing, a reload), never on idle frames,
//! and under a strict budget: `IDLE_BUDGET` for a redraw, `ACTION_BUDGET` when it answers a click. While
//! playback runs only clicks are answered (the rest waits for it to stop). Over budget, the panel stops and
//! shows the error with a Retry button.

use super::runtime::{Ctx, Env, Vm};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// A redraw that answers no interaction (the project changed, a download landed): one 60 Hz frame. These
/// never run while playback runs (`needs_run`'s `playing`), so they can't cost a played frame.
pub const IDLE_BUDGET: Duration = Duration::from_millis(16);
/// A redraw that answers the user's click or edit (it may run tools).
pub const ACTION_BUDGET: Duration = Duration::from_millis(250);
/// Loading the script (its top level runs once).
pub const LOAD_BUDGET: Duration = Duration::from_millis(250);
/// Widgets one draw may make (a runaway loop of `ui.label` stops here, not in the frame).
const MAX_NODES: usize = 2_000;

/// One widget of a panel, as the script asked for it.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Label { text: String, heading: bool },
    Button { id: String, label: String },
    Slider { id: String, label: String, value: f64, min: f64, max: f64 },
    Text { id: String, label: String, value: String, multiline: bool },
    Check { id: String, label: String, value: bool },
    Dropdown { id: String, label: String, value: String, options: Vec<String> },
    Separator,
    Space(f32),
    /// A PNG inside the plugin folder.
    Image { path: PathBuf, w: f32, h: f32 },
    Row(Vec<Node>),
    Column(Vec<Node>),
    // ---- ws:visualizer-helper ----
    /// A widget with hover text (`ui.tip` after it): labels stay plain, the explanation goes here.
    Tip { tip: String, node: Box<Node> },
}

/// What the user did to a widget since the last run.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    Click,
    Num(f64),
    Text(String),
    Bool(bool),
}

/// Collects a run's widgets: `stack` holds the open rows / columns.
#[derive(Default)]
struct Recorder {
    stack: Vec<Vec<Node>>,
    seen: HashMap<String, usize>,
    count: usize,
}

impl Recorder {
    fn push(&mut self, n: Node) -> mlua::Result<()> {
        self.count += 1;
        if self.count > MAX_NODES {
            return Err(mlua::Error::runtime(format!("drew more than {MAX_NODES} widgets in one frame")));
        }
        self.stack.last_mut().expect("root").push(n);
        Ok(())
    }
    /// The widget's id: the label, then `label#2`, `label#3` for repeats; `"Text##key"` uses `key` and shows
    /// `Text` (as in Dear ImGui), so two widgets can share a label.
    fn id(&mut self, label: &str) -> (String, String) {
        let (shown, key) = match label.split_once("##") {
            Some((s, k)) => (s.to_string(), format!("##{k}")),
            None => (label.to_string(), label.to_string()),
        };
        let n = self.seen.entry(key.clone()).or_insert(0);
        *n += 1;
        let id = if *n == 1 { key } else { format!("{key}#{n}") };
        (id, shown)
    }
}

/// A plugin panel: its VM, the last tree it drew, and what the user has done since.
pub struct Panel {
    pub ctx: Ctx,
    pub script: PathBuf,
    vm: Option<Vm>,
    pub nodes: Vec<Node>,
    pub changes: HashMap<String, Change>,
    pub error: Option<String>,
    /// Lines the last run printed (shown under the panel, newest last).
    pub log: Vec<String>,
    /// Run `draw()` on the next frame it is shown.
    pub stale: bool,
    /// The `revision` the tree was drawn for (the app's edit counter).
    pub drawn_for: u64,
    pub last_ms: f32,
    /// The plugin's PNGs, decoded once.
    pub images: HashMap<PathBuf, Option<eframe::egui::TextureHandle>>,
    /// The script's modified time when loaded, and when it was last looked at (`poll_script`).
    mtime: Option<std::time::SystemTime>,
    polled: Option<Instant>,
}

impl Panel {
    pub fn new(ctx: Ctx, script: PathBuf) -> Panel {
        Panel { ctx, script, vm: None, nodes: Vec::new(), changes: HashMap::new(), error: None, log: Vec::new(), stale: true, drawn_for: u64::MAX, last_ms: 0.0, images: HashMap::new(), mtime: None, polled: None }
    }

    /// Throw the VM away: the next draw loads the script again (Reload, or Retry after an error).
    pub fn reload(&mut self) {
        self.vm = None;
        self.error = None;
        self.nodes.clear();
        self.changes.clear();
        self.log.clear();
        self.images.clear();
        self.ctx.net.clear();
        self.stale = true;
    }

    /// A saved change to the script (the Script Editor, another editor) reloads the panel. Looks at the
    /// file at most once a second, and only while the panel is drawn.
    pub fn poll_script(&mut self, now: Instant) {
        if self.polled.is_some_and(|t| now.duration_since(t) < Duration::from_secs(1)) {
            return;
        }
        self.polled = Some(now);
        let m = std::fs::metadata(&self.script).and_then(|m| m.modified()).ok();
        if self.mtime.is_some() && m != self.mtime {
            self.reload();
        }
        self.mtime = m;
    }

    /// Should `draw()` run now? Only when something it shows could have changed; while `playing`, only to
    /// answer the user's click or edit.
    pub fn needs_run(&self, revision: u64, playing: bool) -> bool {
        self.error.is_none() && (!self.changes.is_empty() || !playing && (self.stale || revision != self.drawn_for))
    }

    /// Run `draw()` (loading the script first if needed) and keep the tree it records. Errors stop the
    /// panel until Retry; the previous tree is dropped so nothing half-drawn is shown.
    pub fn run(&mut self, env: &Env, revision: u64) {
        let t0 = Instant::now();
        let acting = !self.changes.is_empty();
        let r = self.run_inner(env, acting);
        self.last_ms = t0.elapsed().as_secs_f32() * 1000.0;
        self.log = env.log.borrow().iter().rev().take(20).rev().cloned().collect();
        self.changes.clear();
        // a run that answered a click drew the widgets above that button before the click changed anything:
        // run once more (on the next frame, with nothing to answer) so the whole panel shows the new state
        self.stale = acting;
        self.drawn_for = revision;
        match r {
            Ok(nodes) => self.nodes = nodes,
            Err(e) => {
                self.nodes.clear();
                self.error = Some(e);
            }
        }
    }

    fn run_inner(&mut self, env: &Env, acting: bool) -> Result<Vec<Node>, String> {
        if self.vm.is_none() {
            let src = std::fs::read_to_string(&self.script).map_err(|e| format!("{}: {e}", self.script.display()))?;
            let vm = Vm::new(&self.ctx)?;
            let name = self.script.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            vm.run(env, LOAD_BUDGET, |_, _, _| Ok(()), |lua| lua.load(&src).set_name(&name).exec())?;
            let has_draw = vm.lua().globals().get::<mlua::Value>("draw").map(|v| v.is_function()).unwrap_or(false);
            if !has_draw {
                return Err("the panel script must define a function draw()".into());
            }
            self.vm = Some(vm);
        }
        let vm = self.vm.as_ref().expect("loaded");
        let rec = std::cell::RefCell::new(Recorder { stack: vec![Vec::new()], ..Default::default() });
        let changes = std::cell::RefCell::new(std::mem::take(&mut self.changes));
        let dir = self.ctx.dir.clone();
        let budget = if acting { super::panel::ACTION_BUDGET } else { IDLE_BUDGET };
        vm.run(
            env,
            budget,
            |scope, _lua, host| bind_ui(scope, host, &rec, &changes, &dir),
            |lua| lua.globals().get::<mlua::Function>("draw")?.call::<()>(()),
        )?;
        let mut rec = rec.into_inner();
        if rec.stack.len() != 1 {
            return Err("a ui.row / ui.column was left open".into());
        }
        Ok(rec.stack.pop().unwrap_or_default())
    }
}

fn bind_ui<'s, 'r>(
    scope: &'s mlua::Scope<'s, 'r>,
    host: &mlua::Table,
    rec: &'r std::cell::RefCell<Recorder>,
    changes: &'r std::cell::RefCell<HashMap<String, Change>>,
    dir: &'r std::path::Path,
) -> mlua::Result<()> {
    let take = move |id: &str| changes.borrow_mut().remove(id);
    host.set(
        "ui.label",
        scope.create_function(move |_, text: mlua::Value| {
            rec.borrow_mut().push(Node::Label { text: crate::scripting::show(&text), heading: false })
        })?,
    )?;
    host.set(
        "ui.heading",
        scope.create_function(move |_, text: mlua::Value| {
            rec.borrow_mut().push(Node::Label { text: crate::scripting::show(&text), heading: true })
        })?,
    )?;
    host.set(
        "ui.button",
        scope.create_function(move |_, label: String| {
            let mut r = rec.borrow_mut();
            let (id, label) = r.id(&label);
            let clicked = matches!(take(&id), Some(Change::Click));
            r.push(Node::Button { id, label })?;
            Ok(clicked)
        })?,
    )?;
    host.set(
        "ui.slider",
        scope.create_function(move |_, (label, value, min, max): (String, f64, Option<f64>, Option<f64>)| {
            let (min, max) = (min.unwrap_or(0.0), max.unwrap_or(1.0));
            let (min, max) = if max < min { (max, min) } else { (min, max) };
            let mut r = rec.borrow_mut();
            let (id, label) = r.id(&label);
            let value = match take(&id) {
                Some(Change::Num(v)) => v,
                _ => value,
            }
            .clamp(min, max);
            r.push(Node::Slider { id, label, value, min, max })?;
            Ok(value)
        })?,
    )?;
    host.set(
        "ui.text",
        scope.create_function(move |_, (label, value, multiline): (String, Option<String>, Option<bool>)| {
            let mut r = rec.borrow_mut();
            let (id, label) = r.id(&label);
            let value = match take(&id) {
                Some(Change::Text(v)) => v,
                _ => value.unwrap_or_default(),
            };
            r.push(Node::Text { id, label, value: value.clone(), multiline: multiline.unwrap_or(false) })?;
            Ok(value)
        })?,
    )?;
    host.set(
        "ui.checkbox",
        scope.create_function(move |_, (label, value): (String, Option<bool>)| {
            let mut r = rec.borrow_mut();
            let (id, label) = r.id(&label);
            let value = match take(&id) {
                Some(Change::Bool(v)) => v,
                _ => value.unwrap_or(false),
            };
            r.push(Node::Check { id, label, value })?;
            Ok(value)
        })?,
    )?;
    host.set(
        "ui.dropdown",
        scope.create_function(move |_, (label, value, options): (String, Option<String>, Vec<String>)| {
            let mut r = rec.borrow_mut();
            let (id, label) = r.id(&label);
            let value = match take(&id) {
                Some(Change::Text(v)) if options.contains(&v) => v,
                _ => value.filter(|v| options.contains(v)).or_else(|| options.first().cloned()).unwrap_or_default(),
            };
            r.push(Node::Dropdown { id, label, value: value.clone(), options })?;
            Ok(value)
        })?,
    )?;
    // ---- ws:visualizer-helper ----
    host.set(
        "ui.tip",
        scope.create_function(move |_, text: mlua::Value| {
            let mut r = rec.borrow_mut();
            let list = r.stack.last_mut().expect("root");
            let last = list.pop().ok_or_else(|| mlua::Error::runtime("ui.tip goes right after the widget it explains"))?;
            list.push(Node::Tip { tip: crate::scripting::show(&text), node: Box::new(last) });
            Ok(())
        })?,
    )?;
    host.set("ui.separator", scope.create_function(move |_, ()| rec.borrow_mut().push(Node::Separator))?)?;
    host.set(
        "ui.space",
        scope.create_function(move |_, px: Option<f32>| rec.borrow_mut().push(Node::Space(px.unwrap_or(8.0).clamp(0.0, 200.0))))?,
    )?;
    host.set(
        "ui.image",
        scope.create_function(move |_, (file, w, h): (String, Option<f32>, Option<f32>)| {
            let rel = super::zip::safe_name(&file).ok_or_else(|| mlua::Error::runtime(format!("ui.image: '{file}' must be a file inside the plugin")))?;
            if !rel.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")) {
                return Err(mlua::Error::runtime("ui.image takes a .png file"));
            }
            rec.borrow_mut().push(Node::Image { path: dir.join(rel), w: w.unwrap_or(0.0).clamp(0.0, 2000.0), h: h.unwrap_or(0.0).clamp(0.0, 2000.0) })
        })?,
    )?;
    for (name, row) in [("ui.row", true), ("ui.column", false)] {
        host.set(
            name,
            scope.create_function(move |_, f: mlua::Function| {
                rec.borrow_mut().stack.push(Vec::new());
                let r = f.call::<()>(());
                let kids = rec.borrow_mut().stack.pop().unwrap_or_default();
                r?;
                rec.borrow_mut().push(if row { Node::Row(kids) } else { Node::Column(kids) })
            })?,
        )?;
    }
    Ok(())
}

// ------------------------------------------------------------------ pane slots

/// What `Pane::Plugin(n)` shows: the panel key ("plugin-id/panel-id") and its tab title / icon. Titles are
/// `&'static str` because every pane title is; each distinct title is leaked once (a handful per plugin).
#[derive(Clone, Default)]
struct SlotInfo {
    key: String,
    title: &'static str,
    icon: Option<&'static str>,
}

static SLOTS: std::sync::Mutex<Vec<SlotInfo>> = std::sync::Mutex::new(Vec::new());

fn slots() -> std::sync::MutexGuard<'static, Vec<SlotInfo>> {
    SLOTS.lock().unwrap_or_else(|e| e.into_inner())
}

/// The tab title of plugin pane `n` ("Plugin panel" for a slot whose plugin is gone).
pub fn slot_title(n: u16) -> &'static str {
    slots().get(n as usize).map(|s| s.title).filter(|t| !t.is_empty()).unwrap_or("Plugin panel")
}

/// The icon name of plugin pane `n`.
pub fn slot_icon(n: u16) -> Option<&'static str> {
    slots().get(n as usize).and_then(|s| s.icon)
}

/// The panel key a slot shows.
pub fn slot_key(n: u16) -> Option<String> {
    slots().get(n as usize).map(|s| s.key.clone()).filter(|k| !k.is_empty())
}

/// Record slot `n`'s panel, title and icon (from `plugins/state.json`'s `panel_slots` and the manifests).
pub fn set_slot(n: u16, key: &str, title: &str, icon: &str) {
    let mut v = slots();
    if v.len() <= n as usize {
        v.resize(n as usize + 1, SlotInfo::default());
    }
    let s = &mut v[n as usize];
    if s.title != title {
        s.title = Box::leak(title.to_string().into_boxed_str());
    }
    let icon = (!icon.is_empty()).then_some(icon);
    if s.icon != icon {
        s.icon = icon.map(|i| &*Box::leak(i.to_string().into_boxed_str()));
    }
    s.key = key.to_string();
}

// ------------------------------------------------------------------ drawing

use eframe::egui;

/// Width of the label column of a panel's form rows.
const LABEL_W: f32 = 110.0;

fn is_form(n: &Node) -> bool {
    match n {
        Node::Tip { node, .. } => is_form(node),
        n => matches!(n, Node::Slider { .. } | Node::Text { .. } | Node::Check { .. } | Node::Dropdown { .. }),
    }
}

/// A node and its hover text (None without `ui.tip`).
fn untip(n: &Node) -> (&Node, Option<&str>) {
    match n {
        Node::Tip { tip, node } => (node, Some(tip.as_str())),
        n => (n, None),
    }
}

/// Draw a recorded tree. Interactions land in `changes` (the next run reads them); true = something was
/// interacted with, so the caller repaints once to run `draw()` with it.
pub fn draw(ui: &mut egui::Ui, nodes: &[Node], changes: &mut HashMap<String, Change>, images: &mut HashMap<PathBuf, Option<egui::TextureHandle>>) -> bool {
    let before = changes.len();
    draw_list(ui, nodes, changes, images, "plugin_panel", false);
    changes.len() != before
}

fn draw_list(ui: &mut egui::Ui, nodes: &[Node], changes: &mut HashMap<String, Change>, images: &mut HashMap<PathBuf, Option<egui::TextureHandle>>, salt: &str, in_row: bool) {
    let mut i = 0;
    while i < nodes.len() {
        if !in_row && is_form(&nodes[i]) {
            // a run of form rows: two aligned columns (labels at one x, inputs at one x)
            let start = i;
            while i < nodes.len() && is_form(&nodes[i]) {
                i += 1;
            }
            egui::Grid::new((salt, start)).num_columns(2).spacing([8.0, 4.0]).min_col_width(LABEL_W).show(ui, |ui| {
                for n in &nodes[start..i] {
                    let (n, tip) = untip(n);
                    let label = form_label(n);
                    ui.add_sized([LABEL_W, 18.0], egui::Label::new(label).truncate()).on_hover_text(tip.unwrap_or(label));
                    let r = form_input(ui, n, changes, false);
                    if let (Some(r), Some(t)) = (r, tip) {
                        r.on_hover_text(t);
                    }
                    ui.end_row();
                }
            });
            continue;
        }
        let (n, tip) = untip(&nodes[i]);
        i += 1;
        let hover = |r: egui::Response| {
            if let Some(t) = tip {
                r.on_hover_text(t);
            }
        };
        match n {
            Node::Label { text, heading: false } => hover(ui.label(text)),
            Node::Label { text, heading: true } => hover(ui.heading(text)),
            Node::Button { id, label } => {
                let r = ui.button(label);
                if r.clicked() {
                    changes.insert(id.clone(), Change::Click);
                }
                hover(r);
            }
            Node::Separator => {
                ui.separator();
            }
            Node::Space(px) => ui.add_space(*px),
            Node::Image { path, w, h } => {
                let tex = images.entry(path.clone()).or_insert_with(|| {
                    let bytes = std::fs::read(path).ok().filter(|b| b.len() <= 4 << 20)?;
                    let f = crate::engine::html::decode_png(&bytes).ok()?;
                    let img = egui::ColorImage::from_rgba_unmultiplied([f.width as usize, f.height as usize], &f.rgba);
                    Some(ui.ctx().load_texture(format!("plugin-img {}", path.display()), img, egui::TextureOptions::LINEAR))
                });
                match tex {
                    Some(t) => {
                        let s = t.size_vec2();
                        let size = match (*w, *h) {
                            (0.0, 0.0) => s,
                            (w, 0.0) => egui::vec2(w, s.y * w / s.x.max(1.0)),
                            (0.0, h) => egui::vec2(s.x * h / s.y.max(1.0), h),
                            (w, h) => egui::vec2(w, h),
                        };
                        ui.add(egui::Image::new((t.id(), size)));
                    }
                    None => {
                        ui.weak(format!("(image {} could not be read)", path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()));
                    }
                }
            }
            Node::Row(kids) => {
                ui.horizontal(|ui| draw_list(ui, kids, changes, images, &format!("{salt}/r{i}"), true));
            }
            Node::Column(kids) => {
                ui.vertical(|ui| draw_list(ui, kids, changes, images, &format!("{salt}/c{i}"), false));
            }
            Node::Tip { node, .. } => draw_list(ui, std::slice::from_ref(&**node), changes, images, &format!("{salt}/t{i}"), in_row),
            form => {
                // a form widget inside a row: its label in front of it
                hover(ui.label(form_label(form)));
                if let Some(r) = form_input(ui, form, changes, true) {
                    hover(r);
                }
            }
        }
    }
}

fn form_label(n: &Node) -> &str {
    match n {
        Node::Slider { label, .. } | Node::Text { label, .. } | Node::Check { label, .. } | Node::Dropdown { label, .. } => label,
        _ => "",
    }
}

/// Draw a form widget's input; its response (for hover text).
fn form_input(ui: &mut egui::Ui, n: &Node, changes: &mut HashMap<String, Change>, compact: bool) -> Option<egui::Response> {
    Some(match n {
        Node::Slider { id, value, min, max, .. } => {
            let mut v = *value;
            let r = ui.add(egui::Slider::new(&mut v, *min..=*max));
            if r.changed() {
                changes.insert(id.clone(), Change::Num(v));
            }
            r
        }
        Node::Text { id, value, multiline, .. } => {
            let mut v = value.clone();
            let w = if compact { 140.0 } else { ui.available_width().max(80.0) };
            let r = if *multiline {
                ui.add(egui::TextEdit::multiline(&mut v).desired_width(w).desired_rows(4).id_salt(id))
            } else {
                ui.add(egui::TextEdit::singleline(&mut v).desired_width(w).id_salt(id))
            };
            if r.changed() {
                changes.insert(id.clone(), Change::Text(v));
            }
            r
        }
        Node::Check { id, value, .. } => {
            let mut v = *value;
            let r = ui.checkbox(&mut v, "");
            if r.changed() {
                changes.insert(id.clone(), Change::Bool(v));
            }
            r
        }
        Node::Dropdown { id, value, options, .. } => {
            let mut pick = None;
            let r = egui::ComboBox::from_id_salt(("plugin_dd", id.as_str())).selected_text(value.as_str()).show_ui(ui, |ui| {
                for o in options {
                    if ui.selectable_label(o == value, o).clicked() {
                        pick = Some(o.clone());
                    }
                }
            });
            if let Some(o) = pick.filter(|o| o != value) {
                changes.insert(id.clone(), Change::Text(o));
            }
            r.response
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::super::runtime::tests::ctx;
    use super::super::Permissions;
    use super::*;
    use serde_json::Value;

    fn panel(src: &str) -> Panel {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("se-panel-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("p.luau");
        std::fs::write(&script, src).unwrap();
        let mut c = ctx(Permissions::default(), Permissions::default());
        c.dir = dir;
        Panel::new(c, script)
    }

    impl Panel {
        fn needs_run_t(&self, rev: u64) -> bool {
            self.needs_run(rev, false)
        }
    }

    #[test]
    fn playback_holds_redraws_but_not_clicks() {
        let mut p = panel(COUNTER);
        run(&mut p, 0);
        assert!(p.needs_run(1, false) && !p.needs_run(1, true), "an edit while playing waits for playback to stop");
        p.changes.insert("Add".into(), Change::Click);
        assert!(p.needs_run(0, true), "a click is answered while playing");
    }

    fn run(p: &mut Panel, rev: u64) -> Vec<String> {
        let mut made = Vec::new();
        {
            let mut call = |n: &str, _: &Value| -> Result<Value, String> {
                made.push(n.to_string());
                Ok(serde_json::json!({"clips": 3}))
            };
            let base = |n: &str| n.starts_with("markers.") || n == "project.summary";
            let ctx = p.ctx.clone();
            let env = Env::new(&ctx, None, &base, &mut call);
            p.run(&env, rev);
        }
        made
    }

    const COUNTER: &str = r#"
        local n = 0
        local name = "Ann"
        function draw()
          ui.heading("Counter")
          ui.label("n = " .. n)
          ui.row(function()
            if ui.button("Add") then n = n + 1 end
            if ui.button("Marker") then tool("markers.add", {}) end
          end)
          name = ui.text("Name", name)
          local v = ui.slider("Size", 5, 0, 10)
          ui.checkbox("On", true)
          ui.dropdown("Kind", "b", {"a", "b"})
        end
    "#;

    #[test]
    fn a_panel_records_widgets_and_answers_clicks_on_the_next_run() {
        let mut p = panel(COUNTER);
        assert!(p.needs_run_t(0));
        run(&mut p, 0);
        assert!(p.error.is_none(), "{:?}", p.error);
        assert!(!p.needs_run_t(0), "nothing changed: no rerun on idle frames");
        assert!(p.needs_run_t(1), "an edit to the project reruns it");
        assert_eq!(p.nodes[0], Node::Label { text: "Counter".into(), heading: true });
        assert_eq!(p.nodes[1], Node::Label { text: "n = 0".into(), heading: false });
        let Node::Row(kids) = &p.nodes[2] else { panic!("{:?}", p.nodes[2]) };
        assert_eq!(kids[0], Node::Button { id: "Add".into(), label: "Add".into() });
        assert!(matches!(&p.nodes[3], Node::Text { value, .. } if value == "Ann"));
        // the user clicks Add and types a name: the next run sees both, and the one after shows the label
        // above the button up to date (immediate-mode: it was drawn before the click was handled)
        p.changes.insert("Add".into(), Change::Click);
        p.changes.insert("Name".into(), Change::Text("Bo".into()));
        assert!(p.needs_run_t(0));
        run(&mut p, 0);
        assert!(p.needs_run_t(0), "one follow-up run after a click");
        run(&mut p, 0);
        assert!(!p.needs_run_t(0), "and only one");
        assert_eq!(p.nodes[1], Node::Label { text: "n = 1".into(), heading: false });
        assert!(matches!(&p.nodes[3], Node::Text { value, .. } if value == "Bo"));
        // a click on Marker runs the tool, once
        p.changes.insert("Marker".into(), Change::Click);
        assert_eq!(run(&mut p, 0), vec!["markers.add".to_string()]);
        assert_eq!(run(&mut p, 0), Vec::<String>::new(), "the follow-up run doesn't click again");
    }

    #[test]
    fn a_slow_or_broken_panel_stops_with_an_error_until_retry() {
        let mut p = panel("function draw() while true do end end");
        run(&mut p, 0);
        let e = p.error.clone().unwrap();
        assert!(e.contains("budget"), "{e}");
        assert!(p.nodes.is_empty() && !p.needs_run_t(5), "a stopped panel does not rerun on its own");
        p.reload();
        assert!(p.needs_run_t(5));
        let mut p = panel("local x = 1");
        run(&mut p, 0);
        assert!(p.error.unwrap().contains("draw()"));
        let mut p = panel("function draw() tool('export.video', {}) end");
        run(&mut p, 0);
        assert!(p.error.unwrap().contains("mcp_tools"), "panels get the same permission checks");
        let mut p = panel("function draw() for i = 1, 5000 do ui.label(i) end end");
        run(&mut p, 0);
        assert!(p.error.unwrap().contains("widgets"));
    }

    /// The example plugin shipped in `examples/plugins/hello-panel` draws, and its buttons call the tools
    /// with the arguments those tools take (the tool replies mimic the app's shapes).
    #[test]
    fn the_example_plugins_panel_draws_and_its_buttons_call_real_tools() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/plugins/hello-panel");
        let mut c = ctx(Permissions::default(), Permissions::default());
        c.dir = dir.clone();
        let mut p = Panel::new(c, dir.join("panels/hello.luau"));
        let go = |p: &mut Panel| -> Vec<(String, Value)> {
            let mut made = Vec::new();
            {
                let mut call = |n: &str, a: &Value| -> Result<Value, String> {
                    made.push((n.to_string(), a.clone()));
                    Ok(match n {
                        "timeline.list" => serde_json::json!({"tracks": [{"name": "V1", "clips": [{"id": 7, "start": 0.0}, {"id": 8, "start": 2.0}]}]}),
                        "editor.context" => serde_json::json!({"playhead": 1.5, "selected_clips": [{"id": 8, "name": "b", "start": 2.0}]}),
                        "markers.list" => serde_json::json!([{"id": 1, "t": 0.5}]),
                        _ => serde_json::json!({"ok": true}),
                    })
                };
                let base = |_: &str| true;
                let ctx = p.ctx.clone();
                let env = Env::new(&ctx, None, &base, &mut call);
                p.run(&env, 0);
            }
            made.retain(|(n, _)| !["timeline.list", "editor.context", "markers.list"].contains(&n.as_str()));
            made
        };
        assert!(go(&mut p).is_empty());
        assert!(p.error.is_none(), "{:?}", p.error);
        assert_eq!(p.nodes[0], Node::Label { text: "Hello from a plugin".into(), heading: true });
        assert_eq!(p.nodes[1], Node::Label { text: "2 clips on the timeline, 1 selected".into(), heading: false });
        assert_eq!(p.nodes[2], Node::Label { text: "1 markers".into(), heading: false });
        p.changes.insert("Marker at playhead".into(), Change::Click);
        let made = go(&mut p);
        assert_eq!(made, vec![("markers.add".to_string(), serde_json::json!({"t": 1.5, "name": "Plugin marker"}))]);
        go(&mut p);
        p.changes.insert("Nudge selection".into(), Change::Click);
        let made = go(&mut p);
        assert_eq!(made, vec![("timeline.move".to_string(), serde_json::json!({"clip_ids": [8], "dt": 0.5}))]);
        // and it draws
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let _ = draw(ui, &p.nodes, &mut p.changes, &mut p.images);
            });
        });
    }

    // ---- ws:visualizer-helper ----
    /// The Visualizer Helper panel the library ships draws its five steps, and each step's one button calls
    /// its tool with arguments that tool declares.
    #[test]
    fn the_visualizer_helper_panel_draws_five_steps_and_calls_its_tools() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("library/plugins/visualizer-helper");
        let mut c = ctx(Permissions::default(), Permissions::default());
        c.dir = dir.clone();
        let mut p = Panel::new(c, dir.join("panels/visualizer-helper.luau"));
        let status = serde_json::json!({
            "song": 4, "name": "olympus", "state": "done", "stems": {"installed": false, "separated": false},
            "analysis": {"bpm": 151.0, "hits": 483, "drops": [26.389, 52.68], "calm": 78.1, "first_vocal": 6.2, "from_stems": true},
            "made": {"visualizer": 9, "template": "Olympus Storm", "lyrics": 0, "fx": 0, "background": 0, "shatter": 0.0},
            "loops": [], "templates": ["Olympus Storm", "Chrome Waveform", "Storm Lightning"], "palettes": ["Silver to Gold", "Neon"],
        });
        let go = |p: &mut Panel| -> Vec<(String, Value)> {
            let mut made = Vec::new();
            {
                let mut call = |n: &str, a: &Value| -> Result<Value, String> {
                    made.push((n.to_string(), a.clone()));
                    Ok(match n {
                        "audio.analyze" => status.clone(),
                        "visualizer.template" => serde_json::json!({"ok": true, "template": "Olympus Storm"}),
                        "lyrics.karaoke" => serde_json::json!({"ok": true, "lines": [{}, {}], "transcript": false, "drop_after": 50.0}),
                        "visualizer.drop_fx" => serde_json::json!({"ok": true, "drops": [26.389, 52.68], "shatter": true}),
                        _ => serde_json::json!({"running": [], "clips": []}),
                    })
                };
                let base = |_: &str| true;
                let ctx = p.ctx.clone();
                let env = Env::new(&ctx, None, &base, &mut call);
                p.run(&env, 0);
            }
            made.retain(|(n, a)| !(n == "audio.analyze" && a.get("action").is_none()));
            made
        };
        assert!(go(&mut p).is_empty(), "an idle draw only reads the status");
        assert!(p.error.is_none(), "{:?}", p.error);
        let labels: Vec<String> = p.nodes.iter().filter_map(|n| if let (Node::Label { text, .. }, _) = untip(n) { Some(text.clone()) } else { None }).collect();
        for s in ["1. Analyse Song", "2. Visualizer Templates", "3. Lyrics Karaoke", "4. Drop FX", "5. Background Loop"] {
            assert!(labels.iter().any(|l| l == s), "{s} missing: {labels:?}");
        }
        assert!(labels.iter().any(|l| l.starts_with("151 BPM")), "{labels:?}");
        let check = |made: &[(String, Value)], tool: &str| {
            let (n, a) = made.iter().find(|(n, _)| n == tool).unwrap_or_else(|| panic!("{tool} not called: {made:?}"));
            let def = crate::mcp::tools::find(n).unwrap_or_else(|| panic!("{n} is not a tool"));
            for k in a.as_object().unwrap().keys() {
                assert!(def.args.iter().any(|d| d.split(':').next() == Some(k.as_str())), "{n} has no argument {k}");
            }
        };
        for (button, tool) in [("Analyse Song", "audio.analyze"), ("Apply Template", "visualizer.template"), ("Make Lyrics", "lyrics.karaoke"), ("Add Drop FX", "visualizer.drop_fx"), ("Make Loop", "media.loop")] {
            p.changes.insert(button.into(), Change::Click);
            let made = go(&mut p);
            assert!(p.error.is_none(), "{button}: {:?}", p.error);
            check(&made, tool);
            go(&mut p);
        }
        // the stems add-on is missing: a button takes you to it
        p.changes.insert("Get Stem Separation".into(), Change::Click);
        let made = go(&mut p);
        assert!(made.iter().any(|(n, a)| n == "audio.analyze" && a["action"] == "stems"), "{made:?}");
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let _ = draw(ui, &p.nodes, &mut p.changes, &mut p.images);
            });
        });
    }

    #[test]
    fn a_tip_explains_the_widget_before_it() {
        let mut p = panel(r#"function draw() ui.button("Go") ui.tip("Runs it") local v = ui.slider("S", 1, 0, 2) ui.tip("Size") end"#);
        run(&mut p, 0);
        assert!(p.error.is_none(), "{:?}", p.error);
        assert_eq!(p.nodes[0], Node::Tip { tip: "Runs it".into(), node: Box::new(Node::Button { id: "Go".into(), label: "Go".into() }) });
        assert!(is_form(&p.nodes[1]));
        let mut p = panel(r#"function draw() ui.tip("nothing") end"#);
        run(&mut p, 0);
        assert!(p.error.unwrap().contains("ui.tip"));
        let ctx = egui::Context::default();
        let mut p2 = panel(r#"function draw() ui.button("Go") ui.tip("Runs it") ui.row(function() ui.checkbox("On", true) ui.tip("x") end) end"#);
        run(&mut p2, 0);
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                assert!(!draw(ui, &p2.nodes, &mut p2.changes, &mut p2.images));
            });
        });
    }

    #[test]
    fn repeated_labels_get_their_own_ids() {
        let mut p = panel(r#"function draw() ui.button("Go") ui.button("Go") ui.button("Go##third") end"#);
        run(&mut p, 0);
        let ids: Vec<&str> = p.nodes.iter().filter_map(|n| if let Node::Button { id, .. } = n { Some(id.as_str()) } else { None }).collect();
        assert_eq!(ids, ["Go", "Go#2", "##third"]);
        assert_eq!(p.nodes[2], Node::Button { id: "##third".into(), label: "Go".into() });
    }

    #[test]
    fn the_drawn_tree_reports_interactions() {
        let ctx = egui::Context::default();
        let nodes = vec![Node::Button { id: "Go".into(), label: "Go".into() }, Node::Slider { id: "S".into(), label: "S".into(), value: 1.0, min: 0.0, max: 2.0 }];
        let (mut changes, mut images) = (HashMap::new(), HashMap::new());
        for _ in 0..3 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    assert!(!draw(ui, &nodes, &mut changes, &mut images), "no input, no change");
                });
            });
        }
        assert!(changes.is_empty());
    }

    #[test]
    fn assert_no_idle_repaint_plugin_panel() {
        let mut p = panel(COUNTER);
        run(&mut p, 0);
        let ctx = egui::Context::default();
        let mut runs = 0;
        for _ in 0..30 {
            if p.needs_run_t(0) {
                run(&mut p, 0);
                runs += 1;
            }
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let _ = draw(ui, &p.nodes, &mut p.changes, &mut p.images);
                });
            });
        }
        assert_eq!(runs, 0, "an idle panel never reruns its script");
        assert!(!ctx.has_requested_repaint(), "an idle plugin panel must not repaint");
    }
}
