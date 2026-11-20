//! ---- feat/plugins ----
//! Plugins in the app: the Plugins pane (the installed list with Enable / Update / Uninstall / Open
//! Folder / View on GitHub, permissions and picked folders), Install from URL and the install confirm,
//! the panes plugin panels draw into (`Pane::Plugin(n)`), Window ▸ Plugins, and the `plugins.*` tools.
//! The plugin format, the install pipeline and the sandbox live in `engine::plugins`.
//!
//! Nothing a plugin ships runs on install. A panel's script runs when its pane is on screen, only when
//! something it shows could have changed (`Panel::needs_run`), under `panel::IDLE_BUDGET`; the edits one
//! run makes are one undo step. Downloads, update checks and unpacking happen on background threads.

use super::tools_helpers::*;
use super::*;
use crate::engine::plugins::{self, panel::Panel, runtime, Permissions, Plugin, Roots, Staged};
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::ui::list_rows::{Ctrl, RowSpec, Rows};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

type Pending<T> = Arc<Mutex<Option<Result<T, String>>>>;

/// The install confirm: what will be installed, and the permissions the user is granting.
pub(crate) struct Confirm {
    pub staged: Staged,
    pub grant: Permissions,
}

#[derive(Default)]
pub(crate) struct PluginsUi {
    /// Tests point this at a scratch folder; None = `%APPDATA%\SimpleEditor`.
    pub roots: Option<Roots>,
    loaded: bool,
    pub list: Vec<Plugin>,
    /// Open panels by key ("plugin-id/panel-id").
    panels: HashMap<String, Panel>,
    pub url_open: bool,
    pub url: String,
    /// A download / unpack in flight, and what it is ("Downloading owner/repo…").
    job: Option<(String, Pending<Staged>)>,
    pub confirm: Option<Confirm>,
    /// Update checks: plugin id -> the newer tag (None = up to date).
    updates: HashMap<String, Pending<Option<String>>>,
    update_found: HashMap<String, Option<String>>,
    /// Rows opened in the Plugins pane.
    open: Vec<String>,
    /// Font files of enabled plugins handed to the text renderer.
    fonts_loaded: Vec<String>,
}

impl PluginsUi {
    pub fn roots(&self) -> Roots {
        self.roots.clone().unwrap_or_else(Roots::app)
    }
    pub fn plugin(&self, id: &str) -> Option<&Plugin> {
        self.list.iter().find(|p| p.manifest.id == id)
    }
}

/// Read the installed list again and bring the pane slots, open panels and fonts up to date.
pub(super) fn refresh(app: &mut App) {
    let roots = app.plugins.roots();
    app.plugins.list = plugins::list(&roots);
    app.plugins.loaded = true;
    let state = roots.load_state();
    for (n, key) in state.panel_slots.iter().enumerate() {
        let (pid, panel) = key.split_once('/').unwrap_or((key, ""));
        match app.plugins.plugin(pid).and_then(|p| p.manifest.contents.panels.iter().find(|d| d.id == panel).map(|d| (p, d))) {
            Some((p, d)) => {
                let icon = if d.icon.is_empty() { &p.manifest.icon } else { &d.icon };
                plugins::panel::set_slot(n as u16, key, &d.title, icon);
            }
            None => plugins::panel::set_slot(n as u16, key, panel, ""),
        }
    }
    // a panel whose plugin went away, was turned off or changed version starts over
    let list = &app.plugins.list;
    app.plugins.panels.retain(|key, p| {
        let pid = key.split('/').next().unwrap_or("");
        list.iter().any(|q| q.manifest.id == pid && q.runnable() && q.manifest.version == p.ctx.version && q.granted() == p.ctx.granted)
    });
}

fn ensure_loaded(app: &mut App) {
    if !app.plugins.loaded {
        refresh(app);
    }
}

/// A cheap fingerprint of "what a panel could show changed": edits (the undo stack) and the selection.
/// Not the playhead: playback must never make panels rerun every frame.
fn revision(app: &App) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    app.undo.len().hash(&mut h);
    app.redo.len().hash(&mut h);
    app.undo.last().map(|e| e.at.to_bits()).hash(&mut h);
    app.selection.hash(&mut h);
    app.project_path.hash(&mut h);
    h.finish()
}

// ------------------------------------------------------------------ running plugin code

/// Run `f` (a script or a panel draw) with the plugin API wired to the live project. Mutating tools take
/// ONE snapshot (on the first edit), so a run that only reads never serialises the project; a run that
/// edits becomes one undo step labelled `label`, and a run that fails puts the project back.
fn with_env<R>(app: &mut App, ctx: &runtime::Ctx, label: &str, f: impl FnOnce(&runtime::Env) -> (R, bool)) -> R {
    let project_dir = app.project_path.as_ref().and_then(|p| p.parent()).map(Path::to_path_buf);
    let before: std::cell::RefCell<Option<String>> = Default::default();
    let (r, ok) = {
        let cell = std::cell::RefCell::new(&mut *app);
        let mut call = |tool: &str, a: &Value| -> Result<Value, String> {
            let mut app = cell.borrow_mut();
            let def = mcp::tools::find(tool).ok_or_else(|| format!("unknown tool '{tool}'"))?;
            if def.kind == ToolKind::Mutate && before.borrow().is_none() {
                *before.borrow_mut() = Some(app.project.to_json());
            }
            match (def.run)(&mut app, a)? {
                ToolOutcome::Done(v) => {
                    if def.kind == ToolKind::Mutate {
                        app.after_edit();
                    }
                    Ok(v)
                }
                ToolOutcome::Image { png_b64, meta } => Ok(mcp::tools::image_json(&png_b64, meta)),
                ToolOutcome::Job(..) | ToolOutcome::Wait(_) => Err(format!("'{tool}' starts a background job and can't be called from a plugin")),
            }
        };
        let base = |n: &str| script_ctl::script_may_call(n);
        let env = runtime::Env::new(ctx, project_dir, &base, &mut call);
        f(&env)
    };
    if let Some(snap) = before.into_inner() {
        if !ok {
            if let Some(p) = mcp_exec::rollback_project(&snap) {
                app.project = p;
                app.after_edit();
            }
        } else if app.project.to_json() != snap {
            push_undo_json(&mut app.undo, &mut app.redo, snap);
            if let Some(e) = app.undo.last_mut() {
                e.label = label.to_string();
            }
        }
    }
    r
}

/// Run one of a plugin's scripts (Window ▸ Plugins ▸ <plugin> ▸ <script>): its prints and errors are toasts.
pub(super) fn run_script(app: &mut App, plugin_id: &str, rel: &str) {
    ensure_loaded(app);
    let Some(p) = app.plugins.plugin(plugin_id).cloned() else { return app.toast(format!("No plugin '{plugin_id}' is installed")) };
    if !p.runnable() {
        return app.toast(format!("{} is turned off", p.manifest.name));
    }
    let Some(path) = plugins::zip::safe_name(rel).map(|r| p.dir.join(r)) else { return };
    let src = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => return app.toast(format!("{}: {e}", path.display())),
    };
    let name = plugins::stem(rel);
    let ctx = runtime::Ctx::of(&p);
    let label = format!("{}: {name}", p.manifest.name);
    let r = with_env(app, &ctx, &label, |env| {
        let r = runtime::run_script(env, &src, &name);
        let ok = r.is_ok();
        (r, ok)
    });
    match r {
        Ok(lines) if lines.is_empty() => app.toast(format!("{name}: done")),
        Ok(lines) => {
            for l in lines.iter().take(5) {
                app.toast(format!("{name}: {l}"));
            }
        }
        Err(e) => app.toast(format!("{name}: {}", e.lines().next().unwrap_or("failed"))),
    }
}

/// Open (or close) a plugin panel: its pane slot is assigned the first time.
pub(super) fn toggle_panel(app: &mut App, plugin_id: &str, panel_id: &str) {
    let key = plugins::panel_key(plugin_id, panel_id);
    match plugins::slot_for(&app.plugins.roots(), &key) {
        Ok(n) => {
            refresh(app);
            app.toggle_panel(Pane::Plugin(n));
        }
        Err(e) => app.toast(e),
    }
}

fn panel_open(app: &App, plugin_id: &str, panel_id: &str) -> bool {
    let key = plugins::panel_key(plugin_id, panel_id);
    let roots = app.plugins.roots();
    roots.load_state().panel_slots.iter().position(|k| *k == key).is_some_and(|n| app.layout.is_visible(Pane::Plugin(n as u16)))
}

// ------------------------------------------------------------------ registries

pub(super) fn tick(app: &mut App, ctx: &egui::Context) {
    // the installed list is read on first use (a menu, the pane, a tool), never at start-up
    if let Some((what, job)) = app.plugins.job.take() {
        let done = job.lock().unwrap_or_else(|e| e.into_inner()).take();
        match done {
            None => {
                app.plugins.job = Some((what, job));
                ctx.request_repaint_after(std::time::Duration::from_millis(250));
            }
            Some(Ok(staged)) => {
                let grant = Permissions { run_programs: false, ..staged.manifest.permissions.clone() };
                app.plugins.url_open = false;
                app.plugins.confirm = Some(Confirm { staged, grant });
            }
            Some(Err(e)) => app.toast(format!("Plugin install: {}", e.lines().next().unwrap_or("failed"))),
        }
    }
    let mut finished = Vec::new();
    for (id, job) in &app.plugins.updates {
        if let Some(r) = job.lock().unwrap_or_else(|e| e.into_inner()).take() {
            finished.push((id.clone(), r));
        }
    }
    if !app.plugins.updates.is_empty() && finished.is_empty() {
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
    for (id, r) in finished {
        app.plugins.updates.remove(&id);
        let name = app.plugins.plugin(&id).map(|p| p.manifest.name.clone()).unwrap_or(id.clone());
        match r {
            Ok(Some(tag)) => {
                app.toast(format!("{name}: {tag} is available - Update to install it"));
                app.plugins.update_found.insert(id, Some(tag));
            }
            Ok(None) => {
                app.toast(format!("{name} is up to date"));
                app.plugins.update_found.insert(id, None);
            }
            Err(e) => app.toast(format!("{name}: {}", e.lines().next().unwrap_or("update check failed"))),
        }
    }
    // OFL fonts of enabled plugins, into the text renderer (idempotent, like Settings' user fonts)
    if app.plugins.loaded {
        let fonts: Vec<String> = app
            .plugins
            .list
            .iter()
            .filter(|p| p.runnable())
            .flat_map(|p| p.manifest.contents.fonts.iter().filter_map(|f| plugins::zip::safe_name(&f.file).map(|r| p.dir.join(r).to_string_lossy().into_owned())))
            .collect();
        if fonts != app.plugins.fonts_loaded {
            if let Ok(mut t) = app.text.try_lock() {
                t.load_user_fonts(&fonts);
                app.fonts = t.families().to_vec();
                app.plugins.fonts_loaded = fonts;
            }
        }
    }
}

/// Start downloading `url` (a GitHub link) for the install confirm.
pub(super) fn start_install(app: &mut App, url: &str, ctx: Option<&egui::Context>) -> Result<String, String> {
    let src = plugins::github::parse(url)?;
    if app.plugins.job.is_some() {
        return Err("another plugin is still downloading".into());
    }
    let what = format!("Downloading {}/{}…", src.owner, src.repo);
    let slot: Pending<Staged> = Arc::default();
    let (roots, url, s2, ctx) = (app.plugins.roots(), url.trim().to_string(), slot.clone(), ctx.cloned());
    std::thread::spawn(move || {
        let r = plugins::download(&roots, &url);
        *s2.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
        if let Some(c) = ctx {
            c.request_repaint();
        }
    });
    app.plugins.job = Some((what.clone(), slot));
    Ok(what)
}

/// Stage a local plugin folder or zip (an author testing their plugin) for the same confirm.
fn stage_local(app: &mut App, path: &Path) {
    let roots = app.plugins.roots();
    let r = if path.is_dir() {
        plugins::stage_folder(&roots, path)
    } else {
        std::fs::read(path).map_err(|e| format!("{}: {e}", path.display())).and_then(|b| plugins::stage_zip(&roots, &b, "", "", ""))
    };
    match r {
        Ok(staged) => {
            let grant = Permissions { run_programs: false, ..staged.manifest.permissions.clone() };
            app.plugins.confirm = Some(Confirm { staged, grant });
        }
        Err(e) => app.toast(format!("Plugin: {e}")),
    }
}

fn check_update(app: &mut App, id: &str) {
    let Some(p) = app.plugins.plugin(id).cloned() else { return };
    let Some(src) = plugins::update_source(&p) else {
        return app.toast(format!("{} wasn't installed from GitHub, so it can't check for updates", p.manifest.name));
    };
    let slot: Pending<Option<String>> = Arc::default();
    let s2 = slot.clone();
    let installed = if p.state.tag.is_empty() { p.manifest.version.clone() } else { p.state.tag.clone() };
    std::thread::spawn(move || {
        let r = match src.release_api() {
            Some(api) => plugins::github::get(&api, true)
                .and_then(|b| plugins::github::release_zip(&String::from_utf8_lossy(&b)))
                .map(|(tag, _)| plugins::github::newer(&tag, &installed).then_some(tag)),
            // a branch / folder link has no versions: offer to reinstall it
            None => Ok(Some("the latest commit".to_string())),
        };
        *s2.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
    });
    app.plugins.updates.insert(id.to_string(), slot);
}

fn uninstall(app: &mut App, id: &str) {
    match plugins::uninstall(&app.plugins.roots(), id) {
        Ok(()) => {
            app.toast("Plugin uninstalled");
            refresh(app);
        }
        Err(e) => app.toast(e),
    }
}

fn set_enabled(app: &mut App, id: &str, on: bool) {
    if let Err(e) = plugins::set_enabled(&app.plugins.roots(), id, on) {
        app.toast(e);
    }
    refresh(app);
}

fn ask_uninstall(app: &mut App, id: &str) {
    let name = app.plugins.plugin(id).map(|p| p.manifest.name.clone()).unwrap_or(id.to_string());
    let id = id.to_string();
    crate::ui::confirm::ask_app(
        format!("Uninstall {name}?"),
        "Removes the plugin's folder and everything it added to the library. Your projects are not touched.",
        move |app| uninstall(app, &id),
    );
}

/// Show `path` selected in Explorer.
fn reveal(path: &Path) {
    let _ = std::process::Command::new("explorer").arg(format!("/select,{}", path.display())).spawn();
}

pub(super) fn windows(app: &mut App, ctx: &egui::Context) {
    if app.plugins.url_open {
        let mut open = true;
        let mut go = false;
        egui::Window::new("Install Plugin").open(&mut open).collapsible(false).resizable(false).default_width(420.0).show(ctx, |ui| {
            ui.label("Paste a GitHub link to the plugin's repository, release or folder.");
            let r = ui.add(egui::TextEdit::singleline(&mut app.plugins.url).hint_text("https://github.com/owner/repo").desired_width(f32::INFINITY));
            go = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.weak("Only install plugins from people you trust.");
            ui.horizontal(|ui| {
                let busy = app.plugins.job.as_ref().map(|(w, _)| w.clone());
                if let Some(w) = &busy {
                    ui.spinner();
                    ui.label(w);
                } else if ui.add_enabled(!app.plugins.url.trim().is_empty(), egui::Button::new("Install…")).clicked() {
                    go = true;
                }
            });
        });
        if go && app.plugins.job.is_none() {
            let url = app.plugins.url.clone();
            if let Err(e) = start_install(app, &url, Some(ctx)) {
                app.toast(e);
            }
        }
        if !open {
            app.plugins.url_open = false;
        }
    }
    let Some(mut c) = app.plugins.confirm.take() else { return };
    let mut decided: Option<bool> = None;
    let m = c.staged.manifest.clone();
    egui::Window::new(format!("Install {}?", m.name)).collapsible(false).resizable(false).default_width(460.0).show(ctx, |ui| {
        egui::Grid::new("plugin_confirm").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
            let mut row = |k: &str, v: String| {
                ui.strong(k);
                ui.label(v);
                ui.end_row();
            };
            row("Name", m.name.clone());
            row("Author", m.author.clone());
            row("Version", match &c.staged.replaces {
                Some(old) => format!("{} (replaces {old})", m.version),
                None => m.version.clone(),
            });
            if !c.staged.source.is_empty() {
                row("From", c.staged.source.clone());
            }
            let contents = m.content_lines();
            row("Contents", if contents.is_empty() { "nothing".into() } else { contents.join("\n") });
        });
        if !m.description.is_empty() {
            ui.add_space(4.0);
            ui.label(&m.description);
        }
        ui.separator();
        let declared = &m.permissions;
        if declared.is_empty() {
            ui.label("It asks for no permissions: it can read and edit the open project only.");
        } else {
            ui.strong("It asks to:");
            for h in &declared.network {
                let mut on = c.grant.network.contains(h);
                if ui.checkbox(&mut on, format!("Download from {h}")).changed() {
                    c.grant.network.retain(|x| x != h);
                    if on {
                        c.grant.network.push(h.clone());
                    }
                }
            }
            for f in &declared.files {
                let what = if f == "project" { "Read and write files in the project's folder" } else { "Read and write files in folders you pick" };
                let mut on = c.grant.files.contains(f);
                if ui.checkbox(&mut on, what).changed() {
                    c.grant.files.retain(|x| x != f);
                    if on {
                        c.grant.files.push(f.clone());
                    }
                }
            }
            for t in &declared.mcp_tools {
                let mut on = c.grant.mcp_tools.contains(t);
                if ui.checkbox(&mut on, format!("Use the app tool {t}")).changed() {
                    c.grant.mcp_tools.retain(|x| x != t);
                    if on {
                        c.grant.mcp_tools.push(t.clone());
                    }
                }
            }
            if declared.run_programs {
                ui.checkbox(&mut c.grant.run_programs, "Run programs on this PC")
                    .on_hover_text("Off by default. A plugin that runs programs can do anything you can: only allow it for plugins you fully trust.");
                if c.grant.run_programs {
                    ui.colored_label(ui.visuals().warn_fg_color, "⚠ It will be able to run any program, with your access to this PC.");
                }
            }
        }
        for w in &c.staged.warnings {
            ui.weak(format!("Note: {w}"));
        }
        ui.separator();
        ui.label("Only install plugins from people you trust. Nothing runs until you open its panels or scripts.");
        ui.horizontal(|ui| {
            if ui.button("Install").clicked() {
                decided = Some(true);
            }
            if ui.button("Cancel").clicked() {
                decided = Some(false);
            }
        });
    });
    match decided {
        None => app.plugins.confirm = Some(c),
        Some(false) => c.staged.discard(),
        Some(true) => {
            match plugins::commit(&app.plugins.roots(), &c.staged, &c.grant, true) {
                Ok(p) => {
                    app.toast(format!("Installed {} {}", p.manifest.name, p.manifest.version));
                    app.plugins.update_found.remove(&p.manifest.id);
                    if !app.plugins.open.contains(&p.manifest.id) {
                        app.plugins.open.push(p.manifest.id.clone());
                    }
                }
                Err(e) => app.toast(format!("Plugin install: {e}")),
            }
            refresh(app);
        }
    }
}

const ROW_CTRLS: &[Ctrl] = &[
    Ctrl { glyph: "⏻", hint: "Enable / Disable" },
    Ctrl { glyph: "⟳", hint: "Update: check GitHub for a newer version" },
    Ctrl { glyph: "🗁", hint: "Open Folder" },
    Ctrl { glyph: "↗", hint: "View on GitHub" },
    Ctrl { glyph: "✕", hint: "Uninstall" },
];

pub(super) fn draw(app: &mut App, ui: &mut egui::Ui, pane: Pane) -> bool {
    match pane {
        Pane::Plugins => {
            ensure_loaded(app);
            manager(app, ui);
            true
        }
        Pane::Plugin(n) => {
            ensure_loaded(app);
            draw_panel(app, ui, n);
            true
        }
        _ => false,
    }
}

fn manager(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        if ui.add(egui::Button::new("Install from URL…").selected(app.plugins.url_open)).on_hover_text("Install a plugin from a GitHub link").clicked() {
            app.plugins.url_open = !app.plugins.url_open;
        }
        if ui.button("Install from Folder…").on_hover_text("Install a plugin folder from this PC (for plugin authors)").clicked() {
            if let Some(dir) = rfd::FileDialog::new().set_title("Plugin folder (with plugin.json)").pick_folder() {
                stage_local(app, &dir);
            }
        }
        if ui.button("Install from Zip…").on_hover_text("Install a plugin .zip from this PC").clicked() {
            if let Some(f) = rfd::FileDialog::new().add_filter("Plugin", &["zip"]).pick_file() {
                stage_local(app, &f);
            }
        }
        if ui.button("Reload").on_hover_text("Read the plugins folder again and restart open plugin panels").clicked() {
            app.plugins.panels.clear();
            refresh(app);
        }
    });
    if let Some((what, _)) = &app.plugins.job {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(what);
        });
    }
    ui.separator();
    if app.plugins.list.is_empty() {
        ui.weak("No plugins installed. A plugin adds panels, scripts, effects, LUTs and more; see Help ▸ Tutorial & Docs ▸ Plugins.");
        return;
    }
    let list = app.plugins.list.clone();
    let mut act: Option<(usize, usize)> = None;
    let mut toggle: Option<String> = None;
    egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
        let mut rows = Rows::new(ui, "plugins_list", ROW_CTRLS, &[70.0], false).fixed_order();
        for (i, p) in list.iter().enumerate() {
            let id = &p.manifest.id;
            let open = app.plugins.open.contains(id);
            let state = if p.error.is_some() { "broken" } else if p.state.enabled { "on" } else { "off" };
            let ver = format!("{} · {state}", p.manifest.version);
            let label = format!("{} - {}", p.manifest.name, p.manifest.author);
            let r = rows.row(ui, i, &RowSpec { key: i as u64, label: &label, open: Some(open), dim: !p.runnable(), values: &[ver.as_str()], ..Default::default() });
            if r.toggled {
                toggle = Some(id.clone());
            }
            if let Some(k) = r.ctrl {
                act = Some((i, k));
            }
            if open {
                body(app, ui, p);
            }
        }
        let _ = rows.finish(ui);
    });
    if let Some(id) = toggle {
        if let Some(i) = app.plugins.open.iter().position(|x| *x == id) {
            app.plugins.open.remove(i);
        } else {
            app.plugins.open.push(id);
        }
    }
    if let Some((i, k)) = act {
        let p = &list[i];
        let id = p.manifest.id.clone();
        match k {
            0 => set_enabled(app, &id, !p.state.enabled),
            1 => {
                if let Some(Some(_)) = app.plugins.update_found.get(&id) {
                    let url = plugins::update_source(p).map(|s| if p.state.source.is_empty() { s.repo_url() } else { p.state.source.clone() });
                    match url.map(|u| start_install(app, &u, Some(ui.ctx()))) {
                        Some(Ok(_)) => {}
                        Some(Err(e)) => app.toast(e),
                        None => {}
                    }
                } else {
                    check_update(app, &id);
                }
            }
            2 => reveal(&p.dir.join(plugins::MANIFEST)),
            3 => match plugins::update_source(p) {
                Some(s) => crate::engine::browsers::open_url(&s.repo_url()),
                None if p.manifest.homepage.starts_with("https://") => crate::engine::browsers::open_url(&p.manifest.homepage),
                None => app.toast(format!("{} has no GitHub page", p.manifest.name)),
            },
            _ => ask_uninstall(app, &id),
        }
    }
}

/// An opened row of the Plugins pane: description, its panels (toggle buttons) and scripts, what it is
/// allowed to do, and its picked folders.
fn body(app: &mut App, ui: &mut egui::Ui, p: &Plugin) {
    ui.indent(("plugin_body", &p.manifest.id), |ui| {
        if let Some(e) = &p.error {
            ui.colored_label(ui.visuals().warn_fg_color, e);
        }
        if !p.manifest.description.is_empty() {
            ui.label(&p.manifest.description);
        }
        if let Some(Some(tag)) = app.plugins.update_found.get(&p.manifest.id) {
            ui.colored_label(app.palette.accent, format!("Update available: {tag}"));
        }
        if app.plugins.updates.contains_key(&p.manifest.id) {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Checking for updates…");
            });
        }
        let id = p.manifest.id.clone();
        if p.runnable() {
            let panels = &p.manifest.contents.panels;
            let scripts = &p.manifest.contents.scripts;
            if !panels.is_empty() || !scripts.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    for d in panels {
                        let open = panel_open(app, &id, &d.id);
                        if ui.add(egui::Button::new(&d.title).selected(open)).on_hover_text("Show or hide this panel").clicked() {
                            toggle_panel(app, &id, &d.id);
                        }
                    }
                    for s in scripts {
                        if ui.button(format!("▶ {}", plugins::stem(s))).on_hover_text("Run this script (one undo step)").clicked() {
                            run_script(app, &id, s);
                        }
                    }
                });
            }
        }
        let lines = p.manifest.content_lines();
        if !lines.is_empty() {
            ui.weak(lines.join(" · "));
        }
        let granted = p.granted().lines();
        ui.weak(if granted.is_empty() { "Permissions: the open project only".to_string() } else { format!("Permissions: {}", granted.join(" · ")) });
        if p.granted().files.iter().any(|f| f == "pick") {
            for f in &p.state.folders {
                ui.weak(format!("Folder: {f}"));
            }
            if ui.button("Add Folder…").on_hover_text("Let this plugin read and write files in a folder you choose").clicked() {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    match plugins::add_folder(&app.plugins.roots(), &id, &dir) {
                        Ok(()) => refresh(app),
                        Err(e) => app.toast(e),
                    }
                }
            }
        }
    });
}

fn draw_panel(app: &mut App, ui: &mut egui::Ui, n: u16) {
    let Some(key) = plugins::panel::slot_key(n) else {
        ui.weak("This plugin panel is no longer available.");
        return;
    };
    let (pid, panel_id) = key.split_once('/').unwrap_or((&key, ""));
    let found = app.plugins.plugin(pid).cloned().and_then(|p| p.manifest.contents.panels.iter().find(|d| d.id == panel_id).cloned().map(|d| (p, d)));
    let Some((p, decl)) = found.filter(|(p, _)| p.runnable()) else {
        let why = match app.plugins.plugin(pid) {
            Some(p) if p.error.is_some() => format!("{} can't load: {}", p.manifest.name, p.error.clone().unwrap_or_default()),
            Some(p) => format!("{} is turned off. Turn it on in the Plugins pane.", p.manifest.name),
            None => "The plugin that drew this panel is not installed.".to_string(),
        };
        ui.weak(why);
        if ui.button("Close").clicked() {
            app.toggle_pane(Pane::Plugin(n));
        }
        return;
    };
    let mut panel = app.plugins.panels.remove(&key).unwrap_or_else(|| {
        let ctx = runtime::Ctx::of(&p);
        let script = plugins::zip::safe_name(&decl.script).map(|r| p.dir.join(r)).unwrap_or_default();
        Panel::new(ctx, script)
    });
    {
        let c = ui.ctx().clone();
        *panel.ctx.net.wake.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(move || c.request_repaint()));
    }
    let rev = revision(app);
    if panel.needs_run(rev) {
        let ctx = panel.ctx.clone();
        let label = format!("{}: {}", p.manifest.name, decl.title);
        with_env(app, &ctx, &label, |env| {
            panel.run(env, rev);
            ((), panel.error.is_none())
        });
        // its own edits changed the revision: don't run again for them
        panel.drawn_for = revision(app);
    }
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        if let Some(e) = panel.error.clone() {
            ui.colored_label(ui.visuals().warn_fg_color, format!("{} stopped: {e}", decl.title));
            ui.horizontal(|ui| {
                if ui.button("Retry").clicked() {
                    panel.reload();
                }
                if ui.button("Open Folder").on_hover_text("Show the plugin's files").clicked() {
                    reveal(&panel.script);
                }
            });
        } else {
            let Panel { nodes, changes, images, .. } = &mut panel;
            if plugins::panel::draw(ui, nodes, changes, images) {
                ui.ctx().request_repaint();
            }
        }
        if !panel.log.is_empty() {
            ui.separator();
            for l in &panel.log {
                ui.weak(l);
            }
        }
    });
    app.plugins.panels.insert(key, panel);
}

/// Window ▸ Plugins: the Plugins pane, Install from URL, then each enabled plugin's panels and scripts.
pub(super) fn window_menu(app: &mut App, ui: &mut egui::Ui) {
    use crate::ui::{menu, tools::Glyph};
    menu::sub(ui, Some(Glyph::Wrench), "Plugins", |ui| {
        ensure_loaded(app);
        if menu::check(ui, app.layout.is_visible(Pane::Plugins), "Plugins", "").clicked() {
            app.toggle_pane(Pane::Plugins);
        }
        if menu::row(ui, None, "Install from URL…", "").clicked() {
            app.plugins.url_open = true;
        }
        let list: Vec<Plugin> = app.plugins.list.iter().filter(|p| p.runnable()).cloned().collect();
        if !list.is_empty() {
            ui.separator();
        }
        for p in list {
            let glyph = Glyph::from_name(&p.manifest.icon);
            let id = p.manifest.id.clone();
            let c = &p.manifest.contents;
            if c.panels.len() == 1 && c.scripts.is_empty() {
                let d = &c.panels[0];
                if menu::check(ui, panel_open(app, &id, &d.id), &p.manifest.name, "").clicked() {
                    toggle_panel(app, &id, &d.id);
                }
                continue;
            }
            menu::sub(ui, glyph, &p.manifest.name, |ui| {
                for d in &c.panels {
                    if menu::check(ui, panel_open(app, &id, &d.id), &d.title, "").clicked() {
                        toggle_panel(app, &id, &d.id);
                    }
                }
                if !c.panels.is_empty() && !c.scripts.is_empty() {
                    ui.separator();
                }
                for s in &c.scripts {
                    if menu::row(ui, Some(Glyph::Terminal), &plugins::stem(s), "").clicked() {
                        run_script(app, &id, s);
                    }
                }
            });
        }
    });
}

// ------------------------------------------------------------------ tools

fn plugin_json(p: &Plugin, app: &App) -> Value {
    json!({
        "id": p.manifest.id, "name": p.manifest.name, "version": p.manifest.version, "author": p.manifest.author,
        "description": p.manifest.description, "enabled": p.state.enabled, "error": p.error,
        "source": p.state.source, "contents": p.manifest.content_lines(),
        "permissions": p.granted().lines(),
        "panels": p.manifest.contents.panels.iter().map(|d| json!({"id": d.id, "title": d.title, "open": panel_open(app, &p.manifest.id, &d.id)})).collect::<Vec<_>>(),
        "scripts": p.manifest.contents.scripts,
    })
}

fn arg_id(args: &Value) -> Result<String, String> {
    req(arg_str(args, "id"), "id").map(str::to_string)
}

pub(crate) const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "plugins.list",
        desc: "Installed plugins: id, name, version, author, enabled, contents, granted permissions, panels (open or not) and scripts.",
        args: &[],
        kind: ToolKind::Read,
        run: |app, _| {
            refresh(app);
            Ok(ToolOutcome::Done(json!({"plugins": app.plugins.list.iter().map(|p| plugin_json(p, app)).collect::<Vec<_>>()})))
        },
    },
    ToolDef {
        name: "plugins.install",
        desc: "Install a plugin from a GitHub link (repository, release, tag or folder). Downloads it, then shows the user the \
               install confirm (name, author, contents, permissions); nothing is installed until the user clicks Install.",
        args: &["url:string:true:https://github.com/owner/repo"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let url = req(arg_str(args, "url"), "url")?.to_string();
            let what = start_install(app, &url, None)?;
            Ok(ToolOutcome::Done(json!({"ok": true, "status": what, "note": "the user confirms the install in the app"})))
        },
    },
    ToolDef {
        name: "plugins.uninstall",
        desc: "Ask the user to uninstall a plugin (removes its folder and what it added to the library; projects are untouched).",
        args: &["id:string:true:"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let id = arg_id(args)?;
            refresh(app);
            if app.plugins.plugin(&id).is_none() {
                return Err(format!("no plugin '{id}' is installed"));
            }
            ask_uninstall(app, &id);
            Ok(ToolOutcome::Done(json!({"ok": true, "note": "the user confirms the uninstall in the app"})))
        },
    },
    ToolDef {
        name: "plugins.enable",
        desc: "Turn a plugin on (its panels, scripts and library content) or off.",
        args: &["id:string:true:", "on:boolean:false:default true"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let id = arg_id(args)?;
            let on = arg_bool(args, "on").unwrap_or(true);
            plugins::set_enabled(&app.plugins.roots(), &id, on)?;
            refresh(app);
            Ok(ToolOutcome::Done(json!({"ok": true, "id": id, "enabled": on})))
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_tools_are_registered() {
        for t in ["plugins.list", "plugins.install", "plugins.uninstall", "plugins.enable"] {
            assert!(mcp::tools::find(t).is_some(), "{t}");
        }
        // plugins can't install, remove or switch plugins themselves
        for t in ["plugins.install", "plugins.uninstall", "plugins.enable"] {
            assert!(!script_ctl::script_may_call(t), "{t}");
        }
    }

    #[test]
    fn plugin_panes_have_titles_from_their_slots() {
        plugins::panel::set_slot(40, "hello-panel/main", "Hello", "bolt");
        assert_eq!(Pane::Plugin(40).title(), "Hello");
        assert_eq!(Pane::Plugin(41).title(), "Plugin panel");
        assert!(Pane::ALL.contains(&Pane::Plugins) && !Pane::ALL.iter().any(|p| matches!(p, Pane::Plugin(_))));
        let l = crate::ui::layout::Layout::default_layout();
        let json = l.to_json();
        let mut l = crate::ui::layout::Layout::from_json(&json).unwrap();
        l.toggle(Pane::Plugin(40));
        assert!(l.is_visible(Pane::Plugin(40)));
        let back = crate::ui::layout::Layout::from_json(&l.to_json()).unwrap();
        assert!(back.is_visible(Pane::Plugin(40)), "a plugin pane survives a save of the layout");
    }
}
