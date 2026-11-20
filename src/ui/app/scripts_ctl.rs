//! ---- feat/scripts-editor ----
//! The App side of the scripts folder: Window ▸ Scripts (per script Run / Open / Open Folder / Rename… /
//! Delete…, then Add Script and Open Scripts Folder), the Script Editor pane (`ui::script_editor`), its
//! top-bar toggle button, the Rename… box, and the `scripts.create` / `rename` / `delete` / `open` tools
//! (beside `scripts.list` / `scripts.run` in `tools_commands`). Settings ▸ General ▸ "Open scripts
//! externally" sends Open and Add Script to the program Windows uses for `.luau` (Notepad when none).
//! Delete moves the file to the Recycle Bin after a confirm; nothing here ever hard-deletes a script.

use super::tools_helpers::*;
use super::*;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::scripting;
use crate::ui::script_editor::{self, Act, MenuAct, Rename};
use std::path::{Path, PathBuf};

/// Make the next `script_metas()` re-read the folder (a file was just made, renamed or binned).
fn invalidate(app: &mut App) {
    if let Some(old) = Instant::now().checked_sub(Duration::from_secs(5)) {
        app.script_meta_cache.0 = old;
    }
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Open `path`: in the program Windows uses for it when `external`, else in the Script Editor pane (asking
/// first when the pane holds unsaved edits to another file).
pub(super) fn open(app: &mut App, path: &Path, external: bool) {
    if external {
        match scripting::open_external(path) {
            Ok(prog) => app.toast(format!("Opened {} in {prog}", file_name(path))),
            Err(e) => app.toast(e),
        }
        return;
    }
    let ed = &app.script_editor;
    if ed.dirty() && ed.path.as_deref() != Some(path) {
        let (path, open_name) = (path.to_path_buf(), ed.file_name());
        crate::ui::confirm::ask_app(
            "Unsaved script",
            format!("{open_name} has unsaved changes. Open {} and lose them?", file_name(&path)),
            move |app| load(app, &path),
        );
        return;
    }
    if ed.path.as_deref() == Some(path) {
        app.surface(Pane::ScriptEditor); // already open: keep its edits, just show it
        return;
    }
    load(app, path);
}

fn load(app: &mut App, path: &Path) {
    match app.script_editor.load(path) {
        Ok(()) => app.surface(Pane::ScriptEditor),
        Err(e) => app.toast(format!("Couldn't open {}: {e}", file_name(path))),
    }
}

/// Add Script: `New Script.luau` from the template in the scripts folder, then opened (where the setting says).
fn add(app: &mut App) -> Option<PathBuf> {
    match scripting::create_in(&scripting::scripts_dir()) {
        Ok(p) => {
            invalidate(app);
            let external = app.settings.scripts_external;
            open(app, &p, external);
            Some(p)
        }
        Err(e) => {
            app.toast(format!("Couldn't add a script: {e}"));
            None
        }
    }
}

fn rename(app: &mut App, path: &Path, name: &str) -> Result<PathBuf, String> {
    let to = scripting::rename(path, name)?;
    app.script_editor.moved(path, &to);
    invalidate(app);
    Ok(to)
}

fn delete_now(app: &mut App, path: &Path) {
    match scripting::delete(path) {
        Ok(()) => {
            app.script_editor.deleted(path);
            invalidate(app);
            app.toast(format!("Moved {} to the Recycle Bin", file_name(path)));
        }
        Err(e) => app.toast(e),
    }
}

/// Window ▸ Scripts ▸ (the submenu's body). Drawn even with no scripts, so Add Script is always there.
pub(super) fn menu(app: &mut App, ui: &mut egui::Ui) {
    let metas = app.script_metas().to_vec();
    let Some(act) = script_editor::scripts_menu(ui, &metas, app.settings.scripts_external) else { return };
    match act {
        MenuAct::Run(p) => app.run_script_path = Some(p),
        MenuAct::Open(p) => {
            let external = app.settings.scripts_external;
            open(app, &p, external);
        }
        MenuAct::OpenFolder(p) => crate::contextmenu::reveal(&p),
        MenuAct::Rename(p, at) => {
            let name = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            app.script_editor.rename = Some(Rename { path: p, name, error: String::new(), at, focus: true });
        }
        MenuAct::Delete(p) => {
            let unsaved = app.script_editor.dirty() && app.script_editor.path.as_deref() == Some(p.as_path());
            let more = if unsaved { " Its unsaved changes in the Script Editor go too." } else { "" };
            crate::ui::confirm::ask_app(
                format!("Delete {}?", file_name(&p)),
                format!("It moves to the Recycle Bin, where you can restore it.{more}"),
                move |app| delete_now(app, &p),
            );
        }
        MenuAct::AddScript => {
            add(app);
        }
        MenuAct::OpenScriptsFolder => {
            let dir = scripting::scripts_dir();
            let _ = std::fs::create_dir_all(&dir);
            crate::contextmenu::reveal(&dir);
        }
    }
}

/// PANE_DRAWERS entry.
pub(super) fn draw(app: &mut App, ui: &mut egui::Ui, pane: Pane) -> bool {
    if pane != Pane::ScriptEditor {
        return false;
    }
    for act in script_editor::show(ui, &mut app.script_editor) {
        match act {
            Act::Save => {
                save(app);
            }
            Act::Run => {
                if !app.script_editor.dirty() || save(app) {
                    // run on the next frame, outside the pane draw (like the Scripts menu's Run)
                    app.script_editor_run = true;
                    ui.ctx().request_repaint();
                }
            }
            Act::OpenExternally => {
                if let Some(p) = app.script_editor.path.clone() {
                    if app.script_editor.dirty() {
                        save(app);
                    }
                    open(app, &p, true);
                }
            }
            Act::OpenFolder => {
                if let Some(p) = &app.script_editor.path {
                    crate::contextmenu::reveal(p);
                }
            }
            Act::AddScript => {
                add(app);
            }
        }
    }
    true
}

/// Save the open script; the result goes to the pane's result line. True when it saved.
fn save(app: &mut App) -> bool {
    let ed = &mut app.script_editor;
    match ed.save() {
        Ok(()) => {
            invalidate(app); // its header (@desc, @hotkey…) may have changed
            true
        }
        Err(e) => {
            ed.output = e;
            ed.output_err = true;
            false
        }
    }
}

/// FRAME_HOOKS entry: the pane's Run, outside the pane draw. The result (or the first error line, with
/// its line number) lands on the pane's result line; the logs are toasts as for every script run.
pub(super) fn tick(app: &mut App, _ctx: &egui::Context) {
    if !std::mem::take(&mut app.script_editor_run) {
        return;
    }
    let Some(path) = app.script_editor.path.clone() else { return };
    let name = file_name(&path);
    let src = app.script_editor.text.clone();
    let result = match scripting::check(&src, &name) {
        Err(e) => Err(e.lines().next().unwrap_or("failed").to_string()),
        Ok(()) => app.run_script_src(&src, &name, None),
    };
    let ed = &mut app.script_editor;
    (ed.output, ed.output_err) = match result {
        Ok(()) => (format!("{name} ran without errors"), false),
        Err(e) => (e, true),
    };
}

/// WINDOW_DRAWERS entry: the Rename… box.
pub(super) fn windows(app: &mut App, ctx: &egui::Context) {
    if let Some((path, name)) = script_editor::rename_box(ctx, &mut app.script_editor) {
        match rename(app, &path, &name) {
            Ok(_) => app.script_editor.rename = None,
            Err(e) => {
                if let Some(r) = app.script_editor.rename.as_mut() {
                    r.error = e;
                }
            }
        }
    }
}

/// ACT_HANDLERS entry.
pub(super) fn act(app: &mut App, a: Action) -> bool {
    if a != Action::ToggleScriptEditor {
        return false;
    }
    app.toggle_panel(Pane::ScriptEditor);
    true
}

/// The top bar's Scripts button: toggles the Script Editor pane, lit while it is on screen.
pub(super) fn button(app: &mut App, ui: &mut egui::Ui) {
    let id = egui::Id::new("se-script-editor-button");
    let open = app.pane_drawn(Pane::ScriptEditor);
    if crate::ui::titlebar::panel_toggle_button(ui, &app.palette, id, tools::Glyph::Terminal, "Script Editor", open)
        .clicked()
    {
        app.toggle_panel(Pane::ScriptEditor);
    }
}

/// A script by its menu name (`@name`, else the file stem) or its file stem.
fn find(app: &mut App, name: &str) -> Result<PathBuf, String> {
    invalidate(app);
    app.script_metas()
        .iter()
        .find(|m| m.name == name || m.path.file_stem().is_some_and(|s| s.to_string_lossy() == name))
        .map(|m| m.path.clone())
        .ok_or_else(|| format!("no script named '{name}'"))
}

fn info(p: &Path) -> Value {
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    json!({"ok": true, "name": stem, "path": p.to_string_lossy()})
}

pub(super) const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "scripts.create",
        desc: "Make a new Luau script in the scripts folder: 'New Script.luau' from the commented template, or \
               'name' (refused if taken) with 'content' instead of the template. 'open' shows it in the Script \
               Editor pane. Writes a file, so the guard asks first.",
        args: &[
            "name:string:false:file name without .luau (default New Script, numbered if taken)",
            "content:string:false:the script's text (default: the template)",
            "open:boolean:false:show it in the Script Editor",
        ],
        kind: ToolKind::Ui,
        run: |app, args| {
            let dir = scripting::scripts_dir();
            let name = match arg_str(args, "name") {
                Some(n) => {
                    let stem = scripting::valid_stem(n)?;
                    if dir.join(format!("{stem}.luau")).exists() {
                        return Err(format!("A script named {stem} already exists"));
                    }
                    Some(stem)
                }
                None => None,
            };
            let mut path = scripting::create_in(&dir).map_err(|e| e.to_string())?;
            if let Some(stem) = name {
                path = scripting::rename(&path, &stem).inspect_err(|_| {
                    let _ = std::fs::remove_file(&path);
                })?;
            }
            if let Some(c) = arg_str(args, "content") {
                std::fs::write(&path, c).map_err(|e| e.to_string())?;
            }
            invalidate(app);
            if arg_bool(args, "open") == Some(true) {
                open(app, &path, false);
            }
            Ok(ToolOutcome::Done(info(&path)))
        },
    },
    ToolDef {
        name: "scripts.rename",
        desc: "Rename a script (by its name from scripts.list or its file stem) to 'to'; the extension is kept and \
               a name another script has is refused.",
        args: &["name:string:true:the script", "to:string:true:the new name, without the extension"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let path = find(app, req(arg_str(args, "name"), "name")?)?;
            let to = rename(app, &path, req(arg_str(args, "to"), "to")?)?;
            Ok(ToolOutcome::Done(info(&to)))
        },
    },
    ToolDef {
        name: "scripts.delete",
        desc: "Move a script (by name) to the Recycle Bin. The guard asks first.",
        args: &["name:string:true:the script"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let path = find(app, req(arg_str(args, "name"), "name")?)?;
            scripting::delete(&path)?;
            app.script_editor.deleted(&path);
            invalidate(app);
            Ok(ToolOutcome::Done(json!({"ok": true, "recycled": path.to_string_lossy()})))
        },
    },
    ToolDef {
        name: "scripts.open",
        desc: "Open a script (by name) in the Script Editor pane, or with 'external' in the program Windows uses \
               for .luau files (Notepad when none; the guard asks first). The 'Open scripts externally' setting \
               does not apply here: omit 'external' for the pane.",
        args: &["name:string:true:the script", "external:boolean:false:open it in another program"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let path = find(app, req(arg_str(args, "name"), "name")?)?;
            let external = arg_bool(args, "external") == Some(true);
            if external {
                let prog = scripting::open_external(&path)?;
                return Ok(ToolOutcome::Done(json!({"ok": true, "opened_in": prog})));
            }
            open(app, &path, false);
            let ed = &app.script_editor;
            Ok(ToolOutcome::Done(
                json!({"ok": true, "open": ed.path.as_deref() == Some(path.as_path()), "unsaved": ed.dirty()}),
            ))
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_tools_are_registered_and_file_changes_are_guarded() {
        let none = json!({});
        for t in ["scripts.create", "scripts.rename", "scripts.delete", "scripts.open"] {
            assert!(mcp::tools::find(t).is_some(), "{t} is in the catalogue");
        }
        for t in ["scripts.create", "scripts.rename", "scripts.delete"] {
            assert!(assistant_ctl::leaves_project(t, &none), "{t} waits for Allow");
        }
        assert!(!assistant_ctl::leaves_project("scripts.open", &none), "the pane is in the app");
        assert!(assistant_ctl::leaves_project("scripts.open", &json!({"external": true})), "another program is not");
        assert!(!script_ctl::script_may_call("scripts.create"), "a sandboxed script can't make files");
    }

    #[test]
    fn the_pane_is_wired() {
        assert_eq!(Pane::by_title("Script Editor"), Some(Pane::ScriptEditor));
        assert!(Pane::ALL.contains(&Pane::ScriptEditor));
        assert_eq!(menus::toggle_action(Pane::ScriptEditor), Some(Action::ToggleScriptEditor));
        let menus = include_str!("menus.rs");
        assert!(menus.contains("scripts_ctl::button(self, ui)"), "the top bar has the toggle button");
        let reg = include_str!("mod.rs");
        for entry in [
            "scripts_ctl::draw,",
            "scripts_ctl::act,",
            "scripts_ctl::windows,",
            "scripts_ctl::tick,",
            "scripts_ctl::TOOLS,",
        ] {
            assert!(reg.contains(entry), "{entry} is registered");
        }
    }
}
