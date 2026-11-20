//! ---- ws:home-page ----
//! The Home page's glue (`ui::home_page` draws it): when it shows, what its clicks do, the Home button,
//! File ▸ Quit to Project List / New, the rename dialog, and the `home.show` / `project.recent` tools.
//!
//! The page shows while `HomeState.open` and `Settings.home_page` are on and nothing is loaded (no file,
//! no clips, no media): it takes the panes' place, so anything that loads a project or media ends it
//! (`set_project` closes it too). Going Home closes the project through the usual save prompt.
//! Project formats and thumbnail sources are read off the UI thread, thumbnails come from `ThumbCache`.

use super::tools_helpers::*;
use super::*;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::ui::home_page::{self, Card, PageAction, RecentInfo, RowView};
use std::collections::{HashMap, HashSet};

/// Thumbnail height asked of `ThumbCache` (it rounds to 16 px).
const THUMB_H: u32 = 64;

#[derive(Default)]
pub(crate) struct HomeState {
    pub(crate) open: bool,
    /// What `home_page::scan` read, per recent path (filled by a worker thread).
    infos: Arc<Mutex<HashMap<String, RecentInfo>>>,
    /// Paths handed to a worker already (so each is read once per visit).
    asked: HashSet<String>,
    rename: Option<RenameDlg>,
}

struct RenameDlg {
    path: String,
    name: String,
    error: Option<String>,
    focus: bool,
}

/// Launch: open on Home when the page is on and nothing was opened (agents' quiet test windows only with
/// `SE_HOME=1`, so their scripted runs keep starting in the editor).
pub(super) fn boot(app: &mut App) {
    let opened = app.project_path.is_some() || !app.project.is_empty() || !app.project.assets.is_empty();
    let quiet = crate::winpos::quiet() && std::env::var("SE_HOME").map_or(true, |v| v != "1");
    if app.settings.home_page && !opened && !quiet {
        app.home.open = true;
    }
}

/// Nothing loaded: the state Home stands in for.
fn nothing_loaded(app: &App) -> bool {
    app.project_path.is_none() && app.project.is_empty() && app.project.assets.is_empty()
}

/// Is the Home page on screen this frame? Something loaded since ends it.
pub(super) fn showing(app: &mut App) -> bool {
    if app.home.open && !nothing_loaded(app) {
        app.home.open = false;
    }
    app.home.open && app.settings.home_page && !app.fullscreen
}

/// The project this window leaves: drop its `.lock` when it is this window's (another window's stays).
pub(super) fn release_lock(old: Option<&Path>, new: Option<&Path>) {
    let Some(old) = old.filter(|o| Some(*o) != new) else { return };
    let lock = home_page::lock_path(old);
    if std::fs::read_to_string(&lock).is_ok_and(|s| s.trim() == std::process::id().to_string()) {
        let _ = std::fs::remove_file(lock);
    }
}

/// The old start-screen card (`ui::home`) only when the Home page is switched off.
pub(super) fn popup_allowed(app: &App) -> bool {
    !app.settings.home_page
}

/// Draw the page in place of the panes. False = not showing (draw the layout).
pub(super) fn draw(app: &mut App, ui: &mut egui::Ui) -> bool {
    if !showing(app) {
        return false;
    }
    let ctx = ui.ctx().clone();
    read_infos(app, &ctx);
    let recents = app.settings.recent_projects.clone();
    let infos = app.home.infos.lock().map(|m| m.clone()).unwrap_or_default();
    let thumbs: Vec<_> = recents
        .iter()
        .map(|r| {
            let (p, t) = infos.get(r)?.thumb.as_ref()?;
            app.thumbs.texture(&ctx, p, *t, THUMB_H)
        })
        .collect();
    let rows: Vec<RowView> =
        recents.iter().zip(&thumbs).map(|(r, th)| RowView { path: r, info: infos.get(r), thumb: *th }).collect();
    let cards = home_page::cards(&app.settings);
    let action = home_page::show(ui, &cards, &rows);
    rename_window(app, &ctx);
    if let Some(a) = action {
        handle(app, a);
    }
    true
}

/// Hand the recents not read yet to one worker thread.
fn read_infos(app: &mut App, ctx: &egui::Context) {
    let todo: Vec<String> = app.settings.recent_projects.iter().filter(|r| !app.home.asked.contains(*r)).cloned().collect();
    if todo.is_empty() {
        return;
    }
    app.home.asked.extend(todo.iter().cloned());
    let (infos, ctx) = (app.home.infos.clone(), ctx.clone());
    std::thread::spawn(move || {
        crate::media::bg::lower_thread();
        for r in todo {
            let i = home_page::scan(Path::new(&r));
            if let Ok(mut m) = infos.lock() {
                m.insert(r, i);
            }
            ctx.request_repaint();
        }
    });
}

/// Forget what was read, so the next frame reads the list again (files may have moved meanwhile).
fn refresh(app: &mut App) {
    app.home.asked.clear();
    if let Ok(mut m) = app.home.infos.lock() {
        m.clear();
    }
}

/// Close the project (through the save prompt) and show Home.
pub(super) fn go_home(app: &mut App) {
    app.confirm_discard_then(|app| {
        if !nothing_loaded(app) {
            app.set_project(Project::new(), None);
        }
        app.home.open = true;
        app.home_dismissed = true; // the old card never comes back over an emptied project
        refresh(app);
    });
}

/// The Home button: Home, or back to the (blank) editor when Home is already showing.
fn toggle(app: &mut App) {
    if !app.settings.home_page {
        app.push_toast(feedback::Toast::new("The Home page is off").action("Settings", Action::Settings));
    } else if showing(app) {
        app.home.open = false;
    } else {
        go_home(app);
    }
}

pub(super) fn act(app: &mut App, a: Action) -> bool {
    match a {
        // File ▸ Quit to Project List / the app icon: Home (through the save prompt); already there, stay
        Action::GoHome if !app.settings.home_page => toggle(app),
        Action::GoHome => {
            if !showing(app) {
                go_home(app);
            }
        }
        // File ▸ New / Ctrl+N: the New cards are where a project is made
        Action::NewProject if app.settings.home_page => go_home(app),
        _ => return false,
    }
    true
}

/// The top bar's Home button: lit while Home is on screen.
pub(super) fn button(app: &mut App, ui: &mut egui::Ui) {
    if !app.settings.home_page {
        return;
    }
    let open = showing(app);
    let id = egui::Id::new("se-home-button");
    if crate::ui::titlebar::panel_toggle_button(ui, &app.palette, id, tools::Glyph::Home, "Home", open).clicked() {
        toggle(app);
    }
}

/// Make the project a card asks for and go straight into the editor.
fn create(app: &mut App, card: Card) {
    match card {
        Card::Blank => app.new_project(None),
        Card::Preset(i) => app.new_project(Some(i)),
        Card::Saved(i) => app.confirm_discard_then(move |app| {
            if let Some(p) = home_page::saved_project(&app.settings, i) {
                app.set_project(p, None);
                app.home_dismissed = true;
            }
        }),
    }
    app.home.open = false;
}

fn handle(app: &mut App, a: PageAction) {
    match a {
        PageAction::Create(card) => create(app, card),
        PageAction::Open(path) => {
            let path = PathBuf::from(path);
            app.confirm_discard_then(move |app| app.open_project(&path));
        }
        PageAction::Rename(path) => {
            let name = home_page::title_of(&path);
            app.home.rename = Some(RenameDlg { path, name, error: None, focus: true });
        }
        PageAction::OpenFolder(path) => {
            let p = Path::new(&path);
            let mut cmd = std::process::Command::new("explorer");
            if p.is_file() {
                cmd.arg("/select,").arg(p);
            } else if let Some(d) = p.parent() {
                cmd.arg(d);
            }
            let _ = cmd.spawn();
        }
        PageAction::Remove(path) => {
            remove(app, &path);
        }
        PageAction::Browse => app.pending_actions.push(Action::OpenProject),
    }
}

fn remove(app: &mut App, path: &str) -> bool {
    let gone = home_page::remove_from_recents(&mut app.settings.recent_projects, path);
    if gone {
        app.settings.save();
    }
    gone
}

/// Rename a recent project's file (see `home_page::rename_project`) and its place in the list; the
/// project open in this window follows it.
fn rename(app: &mut App, old: &str, name: &str) -> Result<PathBuf, String> {
    let new = home_page::rename_project(Path::new(old), name, std::process::id())?;
    let new_s = new.to_string_lossy().into_owned();
    home_page::rename_in_recents(&mut app.settings.recent_projects, old, &new_s);
    if app.project_path.as_deref().is_some_and(|p| p.to_string_lossy().eq_ignore_ascii_case(old)) {
        app.project_path = Some(new.clone());
        let _ = std::fs::write(home_page::lock_path(&new), std::process::id().to_string());
        crate::engine::lut::set_project(Some(&new));
    }
    app.settings.save();
    if let Ok(mut m) = app.home.infos.lock() {
        if let Some(i) = m.remove(old) {
            m.insert(new_s.clone(), i);
        }
    }
    app.home.asked.insert(new_s);
    Ok(new)
}

/// Rename Project: the new name, a Rename button (Enter) and Cancel (Esc); a refusal stays in the dialog.
fn rename_window(app: &mut App, ctx: &egui::Context) {
    let Some(mut d) = app.home.rename.take() else { return };
    let (mut go, mut close) = (false, false);
    let resp = egui::Modal::new(egui::Id::new("home_rename")).show(ctx, |ui| {
        ui.set_width(340.0);
        ui.heading("Rename Project");
        ui.add_space(6.0);
        let t = ui.add(egui::TextEdit::singleline(&mut d.name).desired_width(f32::INFINITY));
        if std::mem::take(&mut d.focus) {
            t.request_focus();
        }
        if t.changed() {
            d.error = None;
        }
        if t.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            go = true;
        }
        if let Some(e) = &d.error {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.button("Rename").on_hover_text("Renames the .sedit file in its folder").clicked() {
                go = true;
            }
            if ui.button("Cancel").clicked() {
                close = true;
            }
        });
    });
    if resp.should_close() {
        close = true;
    }
    if go {
        match rename(app, &d.path, &d.name) {
            Ok(new) => {
                app.toast(format!("Renamed to {}", home_page::title_of(&new.to_string_lossy())));
                return;
            }
            Err(e) => d.error = Some(e),
        }
    }
    if !close {
        app.home.rename = Some(d);
    }
}

/// `project.recent` list rows.
fn recent_json(app: &App) -> Value {
    let rows: Vec<Value> = app
        .settings
        .recent_projects
        .iter()
        .map(|r| {
            let i = home_page::scan(Path::new(r));
            let (w, h, fps) = i.size.map_or((None, None, None), |(w, h, f)| (Some(w), Some(h), Some(f)));
            json!({"path": r, "title": home_page::title_of(r), "exists": i.exists, "width": w, "height": h, "fps": fps})
        })
        .collect();
    json!(rows)
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "home.show",
        desc: "Shows the Home page (recent projects and New cards), closing the project through the usual save prompt; open:false returns to the editor. Needs Settings home_page on.",
        args: &["open:boolean:false:true (default) shows Home, false hides it"],
        kind: ToolKind::Ui,
        run: |app, args| {
            if !app.settings.home_page {
                return Err("the Home page is off (Settings home_page)".into());
            }
            if arg_bool(args, "open").unwrap_or(true) {
                let prompt = app.dirty;
                go_home(app);
                Ok(ToolOutcome::Done(json!({"ok": true, "showing": app.home.open, "save_prompt": prompt})))
            } else {
                app.home.open = false;
                Ok(ToolOutcome::Done(json!({"ok": true, "showing": false})))
            }
        },
    },
    ToolDef {
        name: "project.recent",
        desc: "The recent projects list. action list (default): path, title, exists, width, height, fps per row; rename: renames the .sedit file at path to name (refuses a taken name, a missing file, or a project open in another window); remove: drops path from the list (the file stays).",
        args: &[
            "action:string:false:list | rename | remove",
            "path:string:false:the project's path as listed (rename / remove)",
            "name:string:false:the new file name without .sedit (rename)",
        ],
        kind: ToolKind::Ui,
        run: |app, args| match arg_str(args, "action").unwrap_or("list") {
            "list" => Ok(ToolOutcome::Done(recent_json(app))),
            "rename" => {
                let path = req(arg_str(args, "path"), "path")?.to_string();
                let name = req(arg_str(args, "name"), "name")?.to_string();
                let new = rename(app, &path, &name)?;
                Ok(ToolOutcome::Done(json!({"ok": true, "path": new.to_string_lossy()})))
            }
            "remove" => {
                let path = req(arg_str(args, "path"), "path")?;
                if !remove(app, path) {
                    return Err(format!("not in the list: {path}"));
                }
                Ok(ToolOutcome::Done(json!({"ok": true})))
            }
            other => Err(format!("unknown action {other:?} (list | rename | remove)")),
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    /// A headless App whose settings live in a scratch folder for the whole test.
    fn app(name: &str) -> (App, PathBuf) {
        let scratch = std::env::temp_dir().join(format!("se-home-ctl-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).unwrap();
        *crate::settings::TEST_DIR.lock().unwrap_or_else(|e| e.into_inner()) = Some(scratch.clone());
        let mut app = App::build(&egui::Context::default(), None, None);
        app.settings.recent_projects.clear();
        app.settings.home_page = true;
        (app, scratch)
    }

    #[test]
    fn preset_cards_make_the_right_format_and_go_to_the_editor() {
        let (mut app, _d) = app("cards");
        app.home.open = true;
        assert!(showing(&mut app));
        let shorts = PRESETS_SHORT;
        create(&mut app, Card::Preset(shorts));
        let p = &crate::ui::guides::PRESETS[shorts];
        assert_eq!((app.project.width, app.project.height), (p.w, p.h));
        assert_eq!(app.project.max_duration, Some(60.0), "short form gets Max length 1:00");
        assert!(!app.home.open, "straight into the editor");
        let four_k = crate::ui::guides::PRESETS.iter().position(|p| p.w == 3840).unwrap();
        create(&mut app, Card::Preset(four_k));
        assert_eq!((app.project.width, app.project.height, app.project.max_duration), (3840, 2160, None));
        app.settings.project_templates.push(crate::settings::ProjectTemplate { name: "Mine".into(), width: 1280, height: 720, fps: 24.0 });
        create(&mut app, Card::Saved(0));
        assert_eq!((app.project.width, app.project.height, app.project.fps), (1280, 720, 24.0));
        create(&mut app, Card::Blank);
        let blank = Project::new();
        assert_eq!((app.project.width, app.project.height, app.project.fps), (blank.width, blank.height, blank.fps));
    }

    /// The built-in 9:16 preset with no social guide (so the test never writes the guide setting).
    const PRESETS_SHORT: usize = 1;

    #[test]
    fn new_and_home_go_home_and_the_setting_off_keeps_the_old_popup() {
        let (mut app, _d) = app("setting");
        assert!(crate::ui::guides::PRESETS[PRESETS_SHORT].guide.is_none());
        // on: File ▸ New goes Home (no project made), the card popup never shows
        assert!(act(&mut app, Action::NewProject));
        assert!(app.home.open && showing(&mut app));
        assert!(!popup_allowed(&app));
        // something loaded ends Home
        app.project_path = Some(PathBuf::from("C:/x.sedit"));
        assert!(!showing(&mut app) && !app.home.open);
        app.project_path = None;
        // Quit to Project List goes Home and stays; the house button toggles back to the editor
        app.home.open = false;
        act(&mut app, Action::GoHome);
        assert!(showing(&mut app));
        act(&mut app, Action::GoHome);
        assert!(showing(&mut app), "Quit to Project List on Home stays on Home");
        toggle(&mut app);
        assert!(!showing(&mut app));
        toggle(&mut app);
        assert!(showing(&mut app));
        // off: New is the old New again (not handled here), Home never shows, the popup is back
        app.settings.home_page = false;
        assert!(!act(&mut app, Action::NewProject), "File ▸ New falls through to the old blank project");
        app.home.open = true;
        assert!(!showing(&mut app));
        assert!(popup_allowed(&app));
        assert!(crate::ui::home::visible(&app.settings, true, false), "the old start card shows as before");
        // and launch doesn't open it
        app.home.open = false;
        boot(&mut app);
        assert!(!app.home.open);
    }

    #[test]
    fn recent_tool_lists_renames_and_removes() {
        let (mut app, d) = app("tool");
        let a = d.join("One.sedit");
        std::fs::write(&a, Project::new().to_json()).unwrap();
        let gone = d.join("Gone.sedit");
        app.settings.recent_projects = vec![a.to_string_lossy().into(), gone.to_string_lossy().into()];
        let run = |app: &mut App, args: Value| match (TOOLS[1].run)(app, &args) {
            Ok(ToolOutcome::Done(v)) => Ok(v),
            Ok(_) => panic!("not a plain reply"),
            Err(e) => Err(e),
        };
        let l = run(&mut app, json!({})).unwrap();
        assert_eq!(l[0]["title"], "One");
        assert_eq!((l[0]["exists"].clone(), l[1]["exists"].clone()), (json!(true), json!(false)));
        assert_eq!(l[0]["width"], 1920);
        let r = run(&mut app, json!({"action": "rename", "path": a.to_string_lossy(), "name": "Two"})).unwrap();
        let two = d.join("Two.sedit");
        assert_eq!(r["path"], two.to_string_lossy().as_ref());
        assert!(two.is_file() && !a.exists());
        assert_eq!(app.settings.recent_projects[0], two.to_string_lossy());
        assert!(run(&mut app, json!({"action": "rename", "path": gone.to_string_lossy(), "name": "X"})).is_err());
        run(&mut app, json!({"action": "remove", "path": gone.to_string_lossy()})).unwrap();
        assert_eq!(app.settings.recent_projects.len(), 1);
        assert!(run(&mut app, json!({"action": "remove", "path": gone.to_string_lossy()})).is_err());
        *crate::settings::TEST_DIR.lock().unwrap_or_else(|e| e.into_inner()) = None;
        let _ = std::fs::remove_dir_all(&d);
    }
}
