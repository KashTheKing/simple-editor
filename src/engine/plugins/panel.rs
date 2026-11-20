//! ---- feat/plugins ----
//! Plugin panels: a Luau script draws a dockable pane through a small immediate-mode API -
//!
//! ```lua
//! local count = 0
//! function draw()
//!   ui.heading("Hello")
//!   if ui.button("Add a marker") then editor.tool("markers.add", {}) end
//!   count = ui.slider("Count", count, 0, 10)
//! end
//! ```
//!
//! Each run of `draw()` records a tree of widgets (`Node`); the app then draws that tree with egui (form
//! rows in two aligned columns, like every other panel). What the user did to a widget (a click, a new
//! slider value, typed text) is handed to the next run as that widget's return value, so the script reads
//! like immediate-mode code while the Lua never touches egui. `draw()` runs only when something could have
//! changed (an interaction, an edit to the project, a download landing, a reload), never on idle frames,
//! and under a strict budget: `IDLE_BUDGET` for a redraw, `ACTION_BUDGET` when it answers a click. Over
//! budget, the panel stops and shows the error with a Retry button.

use super::runtime::{Ctx, Env, Vm};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// A redraw that answers no interaction (the project changed, a download landed).
pub const IDLE_BUDGET: Duration = Duration::from_millis(4);
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
    pub key: String,
    pub title: String,
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
}

impl Panel {
    pub fn new(key: String, title: String, ctx: Ctx, script: PathBuf) -> Panel {
        Panel { key, title, ctx, script, vm: None, nodes: Vec::new(), changes: HashMap::new(), error: None, log: Vec::new(), stale: true, drawn_for: u64::MAX, last_ms: 0.0, images: HashMap::new() }
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

    /// Should `draw()` run now? Only when something it shows could have changed.
    pub fn needs_run(&self, revision: u64) -> bool {
        self.error.is_none() && (self.stale || !self.changes.is_empty() || revision != self.drawn_for)
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
        self.stale = false;
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

// ------------------------------------------------------------------ drawing

use eframe::egui;

/// Width of the label column of a panel's form rows.
const LABEL_W: f32 = 110.0;

fn is_form(n: &Node) -> bool {
    matches!(n, Node::Slider { .. } | Node::Text { .. } | Node::Check { .. } | Node::Dropdown { .. })
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
                    let label = form_label(n);
                    ui.add_sized([LABEL_W, 18.0], egui::Label::new(label).truncate()).on_hover_text(label);
                    form_input(ui, n, changes, false);
                    ui.end_row();
                }
            });
            continue;
        }
        let n = &nodes[i];
        i += 1;
        match n {
            Node::Label { text, heading: false } => {
                ui.label(text);
            }
            Node::Label { text, heading: true } => {
                ui.heading(text);
            }
            Node::Button { id, label } => {
                if ui.button(label).clicked() {
                    changes.insert(id.clone(), Change::Click);
                }
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
            form => {
                // a form widget inside a row: its label in front of it
                ui.label(form_label(form));
                form_input(ui, form, changes, true);
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

fn form_input(ui: &mut egui::Ui, n: &Node, changes: &mut HashMap<String, Change>, compact: bool) {
    match n {
        Node::Slider { id, value, min, max, .. } => {
            let mut v = *value;
            if ui.add(egui::Slider::new(&mut v, *min..=*max)).changed() {
                changes.insert(id.clone(), Change::Num(v));
            }
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
        }
        Node::Check { id, value, .. } => {
            let mut v = *value;
            if ui.checkbox(&mut v, "").changed() {
                changes.insert(id.clone(), Change::Bool(v));
            }
        }
        Node::Dropdown { id, value, options, .. } => {
            let mut pick = None;
            egui::ComboBox::from_id_salt(("plugin_dd", id.as_str())).selected_text(value.as_str()).show_ui(ui, |ui| {
                for o in options {
                    if ui.selectable_label(o == value, o).clicked() {
                        pick = Some(o.clone());
                    }
                }
            });
            if let Some(o) = pick.filter(|o| o != value) {
                changes.insert(id.clone(), Change::Text(o));
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::super::runtime::tests::ctx;
    use super::super::Permissions;
    use super::*;
    use serde_json::Value;

    fn panel(src: &str) -> Panel {
        let dir = std::env::temp_dir().join(format!("se-panel-{}-{}", std::process::id(), src.len()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("p.luau");
        std::fs::write(&script, src).unwrap();
        let mut c = ctx(Permissions::default(), Permissions::default());
        c.dir = dir;
        Panel::new("t/main".into(), "T".into(), c, script)
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
            if ui.button("Marker") then editor.tool("markers.add", {}) end
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
        assert!(p.needs_run(0));
        run(&mut p, 0);
        assert!(p.error.is_none(), "{:?}", p.error);
        assert!(!p.needs_run(0), "nothing changed: no rerun on idle frames");
        assert!(p.needs_run(1), "an edit to the project reruns it");
        assert_eq!(p.nodes[0], Node::Label { text: "Counter".into(), heading: true });
        assert_eq!(p.nodes[1], Node::Label { text: "n = 0".into(), heading: false });
        let Node::Row(kids) = &p.nodes[2] else { panic!("{:?}", p.nodes[2]) };
        assert_eq!(kids[0], Node::Button { id: "Add".into(), label: "Add".into() });
        assert!(matches!(&p.nodes[3], Node::Text { value, .. } if value == "Ann"));
        // the user clicks Add and types a name: the next run sees both
        p.changes.insert("Add".into(), Change::Click);
        p.changes.insert("Name".into(), Change::Text("Bo".into()));
        assert!(p.needs_run(0));
        run(&mut p, 0);
        assert_eq!(p.nodes[1], Node::Label { text: "n = 1".into(), heading: false });
        assert!(matches!(&p.nodes[3], Node::Text { value, .. } if value == "Bo"));
        // a click on Marker runs the tool, once
        p.changes.insert("Marker".into(), Change::Click);
        assert_eq!(run(&mut p, 0), vec!["markers.add".to_string()]);
        assert_eq!(run(&mut p, 1), Vec::<String>::new());
    }

    #[test]
    fn a_slow_or_broken_panel_stops_with_an_error_until_retry() {
        let mut p = panel("function draw() while true do end end");
        run(&mut p, 0);
        let e = p.error.clone().unwrap();
        assert!(e.contains("budget"), "{e}");
        assert!(p.nodes.is_empty() && !p.needs_run(5), "a stopped panel does not rerun on its own");
        p.reload();
        assert!(p.needs_run(5));
        let mut p = panel("ui.label('no draw')");
        run(&mut p, 0);
        assert!(p.error.unwrap().contains("draw()"));
        let mut p = panel("function draw() editor.tool('export.video', {}) end");
        run(&mut p, 0);
        assert!(p.error.unwrap().contains("mcp_tools"), "panels get the same permission checks");
        let mut p = panel("function draw() for i = 1, 5000 do ui.label(i) end end");
        run(&mut p, 0);
        assert!(p.error.unwrap().contains("widgets"));
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
            if p.needs_run(0) {
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
