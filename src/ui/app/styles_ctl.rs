//! ---- feat/styles-context ----
//! Project Context and Editing Styles in the app: the Project Context pane, `/style <name>` from the
//! Assistant's composer, the Editing Styles tab's actions, the Learn a style window, and the MCP tools
//! (`context.get` / `context.set`, `styles.list` / `get` / `save` / `apply` / `remove` / `delete`,
//! `styles.learn` + `styles.learn_status`, `styles.describer` / `styles.install_describer`).
//!
//! Applying or removing a style is one labelled undo step; typing in the pane is one step per burst
//! (from the first key to the box losing the keyboard).

use super::tools_helpers::*;
use super::*;
use crate::engine::style_learn;
use crate::engine::styles::{self, Style};
use crate::engine::vlm;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::model::context;
use crate::ui::styles_ui::{self, CtxOut, CtxView, Learn, LearnOut, LearnView};
use serde_json::{json, Value};
use std::sync::atomic::Ordering;

/// The style written into the open project's context, if any.
pub(super) fn applied(app: &App) -> Option<String> {
    context::applied_style(&app.project.context)
}

/// Apply `name` from the library to the project: one undo step ("Apply style").
pub(super) fn apply(app: &mut App, name: &str) -> Result<(), String> {
    let s = styles::get(name).ok_or_else(|| format!("no style named '{name}' (styles.list has them)"))?;
    let before = app.project.to_json();
    if app.project.apply_context_style(&s.name, &styles::guide_markdown(&s), &styles::rules_markdown(&s)) {
        app.push_undo_labeled(before, "Apply style");
        app.dirty = true;
    }
    Ok(())
}

/// Take the applied style out again: one undo step ("Remove style"). False = there was none.
pub(super) fn remove(app: &mut App) -> bool {
    let before = app.project.to_json();
    if app.project.remove_context_style() {
        app.push_undo_labeled(before, "Remove style");
        app.dirty = true;
        return true;
    }
    false
}

/// `/style …` typed in the Assistant's composer (handled here, never sent to the agent). `arg` is what
/// follows `/style`: a name, `off`, or nothing (opens the library).
pub(super) fn slash(app: &mut App, arg: &str) {
    use crate::ui::assistant_ui::{Line, Tab};
    let arg = arg.trim();
    if arg.is_empty() {
        app.assistant.tab = Tab::Styles;
        return;
    }
    let line = if styles::is_off(arg) {
        match applied(app) {
            Some(n) if remove(app) => format!("Took the **{n}** style out of Project Context."),
            _ => "No style is applied to this project.".to_string(),
        }
    } else {
        let name = styles::slug(arg);
        match apply(app, &name) {
            Ok(()) => format!(
                "Applied the **{name}** style: its guidance is in Project Context's Style guide and its rules in Rules. \
                 Undo takes it back out, or `/style off`."
            ),
            Err(e) => {
                app.toast(e);
                app.assistant.prompt = format!("/style {arg}");
                return;
            }
        }
    };
    let st = &mut app.assistant;
    st.push(Line::User(format!("/style {arg}")));
    st.push(Line::Info(line));
    st.dirty = true;
}

/// The Editing Styles tab's and the Project Context button's asks.
pub(super) fn styles_out(app: &mut App, o: styles_ui::Out) {
    use styles_ui::Out as O;
    let r: Result<String, String> = match o {
        O::None => return,
        O::ToggleContext => {
            app.toggle_panel(Pane::Context);
            return;
        }
        O::Apply(n) => apply(app, &n).map(|_| format!("Applied the {n} style")),
        O::Remove => Ok(if remove(app) { "Removed the style".into() } else { "No style is applied".into() }),
        O::Duplicate(n) => styles::duplicate(&n).map(|c| {
            app.assistant.styles.selected = Some(c.clone());
            format!("Made {c}")
        }),
        O::Rename(a, b) => styles::rename(&a, &b).map(|_| {
            app.assistant.styles.selected = Some(b.clone());
            format!("Renamed to {b}")
        }),
        O::Delete(n) => {
            crate::ui::confirm::ask_app("Delete style?", format!("Delete the {n} style? Its file is removed from your styles folder."), move |app| {
                match styles::delete(&n) {
                    Ok(()) => app.toast(format!("Deleted {n}")),
                    Err(e) => app.toast(e),
                }
            });
            return;
        }
        O::Import => match rfd::FileDialog::new().add_filter("Style", &["md"]).pick_file() {
            Some(p) => styles::import(&p).map(|n| {
                app.assistant.styles.selected = Some(n.clone());
                format!("Imported {n}")
            }),
            None => return,
        },
        O::Export(n) => {
            let Some(s) = styles::get(&n) else { return };
            match rfd::FileDialog::new().add_filter("Style", &["md"]).set_file_name(format!("{n}.md")).save_file() {
                Some(p) => std::fs::write(&p, styles::file_text(&s)).map(|_| format!("Saved {}", p.display())).map_err(|e| e.to_string()),
                None => return,
            }
        }
        O::OpenFolder => {
            let d = styles::user_dir();
            let _ = std::fs::create_dir_all(&d);
            let _ = std::process::Command::new("explorer").arg(d).spawn();
            return;
        }
        O::OpenLearn => {
            if app.assistant.styles.learn.is_none() {
                let mut l = Learn::new(vlm::available());
                l.speech = false;
                app.assistant.styles.learn = Some(l);
                app.styles_ctx_can_transcribe = crate::engine::transcribe::exe().is_some();
            }
            return;
        }
    };
    match r {
        Ok(m) => app.toast(m),
        Err(e) => app.toast(e),
    }
}

// ---------------------------------------------------------------------------------------------------
// the pane

pub(super) fn draw(app: &mut App, ui: &mut egui::Ui, pane: Pane) -> bool {
    if pane != Pane::Context {
        return false;
    }
    let digest = app.styles_ctx.digest_open.then(|| context::digest(&app.project));
    let open = [Pane::Planner, Pane::Moodboard, Pane::Markers].map(|p| app.pane_drawn(p));
    let v = CtxView { doc: &app.project.context, applied: applied(app), digest, open };
    match styles_ui::context_pane(ui, &mut app.styles_ctx, &v) {
        CtxOut::None => {}
        CtxOut::Edited(text) => {
            if app.styles_ctx.before.is_none() {
                app.styles_ctx.before = Some(app.project.to_json());
            }
            app.project.set_context(&text);
            app.dirty = true;
        }
        CtxOut::Commit => commit_typing(app),
        CtxOut::RemoveStyle => {
            commit_typing(app);
            remove(app);
        }
        CtxOut::OpenStyles => {
            app.assistant.tab = crate::ui::assistant_ui::Tab::Styles;
            app.surface(Pane::Assistant);
        }
        CtxOut::Open(p) => app.toggle_panel(p),
    }
    true
}

/// Close the typing burst: one undo entry for it (when it changed anything).
fn commit_typing(app: &mut App) {
    if let Some(before) = app.styles_ctx.before.take() {
        if before != app.project.to_json() {
            app.push_undo_labeled(before, "Edit Project Context");
        }
    }
}

// ---------------------------------------------------------------------------------------------------
// the Learn a style window (WINDOW_DRAWERS)

fn job_view(j: &style_learn::LearnJob) -> (f32, String, bool, Option<String>) {
    let p = &j.progress;
    (p.fraction(), p.status(), p.is_done(), p.error())
}

pub(super) fn windows(app: &mut App, ctx: &egui::Context) {
    let Some(mut l) = app.assistant.styles.learn.take() else { return };
    let job = l.job.and_then(style_learn::job);
    if let Some(j) = &job {
        if j.progress.is_done() && l.draft.is_empty() {
            if let Some(s) = j.result.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
                l.draft = styles::to_text(s);
            }
        }
    }
    let installing = vlm::installing();
    let v = LearnView {
        job: job.as_deref().map(job_view),
        models: vlm::installed_models(),
        installing: installing.as_ref().map(|p| (p.fraction(), p.status())),
        install_error: vlm::install_error(),
        size_default: vlm::install_size(vlm::DEFAULT),
        size_upgrade: vlm::install_size("2.2b"),
        can_transcribe: app.styles_ctx_can_transcribe,
    };
    if v.models.is_empty() && l.describe {
        l.describe = false;
    }
    let busy = v.job.as_ref().is_some_and(|j| !j.2) || installing.is_some();
    let out = styles_ui::learn_window(ctx, &mut l, &v);
    if busy {
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
    match out {
        LearnOut::None => {}
        LearnOut::Pick => {
            let exts = ["mp4", "mov", "mkv", "webm", "avi", "m4v", "wmv", "mts"];
            if let Some(p) = rfd::FileDialog::new().add_filter("Video", &exts).pick_file() {
                if l.name.trim().is_empty() {
                    l.name = styles::slug(&p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default());
                }
                l.path = p.to_string_lossy().to_string();
            }
        }
        LearnOut::Start => match start_learn(&l) {
            Ok(j) => {
                l.job = Some(j.id);
                l.draft.clear();
                l.error.clear();
            }
            Err(e) => l.error = e,
        },
        LearnOut::Cancel => {
            if let Some(j) = &job {
                j.progress.cancel.store(true, Ordering::SeqCst);
            }
        }
        LearnOut::Install(id) => {
            vlm::start_install(&id);
            l.describe = true;
            l.model = id;
        }
        LearnOut::Save(and_apply) => match save_draft(&l.draft) {
            Ok(name) => {
                app.toast(format!("Saved the {name} style"));
                app.assistant.styles.selected = Some(name.clone());
                app.assistant.tab = crate::ui::assistant_ui::Tab::Styles;
                if and_apply {
                    if let Err(e) = apply(app, &name) {
                        app.toast(e);
                    }
                }
                return; // the window closes
            }
            Err(e) => l.error = e,
        },
        LearnOut::Close => {
            if let Some(j) = job.filter(|j| !j.progress.is_done()) {
                j.progress.cancel.store(true, Ordering::SeqCst);
            }
            return;
        }
    }
    app.assistant.styles.learn = Some(l);
}

fn start_learn(l: &Learn) -> Result<std::sync::Arc<style_learn::LearnJob>, String> {
    let path = std::path::PathBuf::from(l.path.trim());
    if !path.is_file() {
        return Err(format!("{} is not a file", path.display()));
    }
    let model = (l.describe && vlm::has_model(&l.model)).then(|| l.model.clone());
    let name = if l.name.trim().is_empty() { path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default() } else { l.name.clone() };
    Ok(style_learn::start(style_learn::Options { path, name, describe: model, max_shots: l.max_shots.max(1), speech: l.speech }))
}

/// Save a reviewed draft (its front matter names it) as a user style; returns the name.
fn save_draft(text: &str) -> Result<String, String> {
    let s: Style = styles::parse("learned", text, styles::Source::User, None);
    if !styles::valid_name(&s.name) {
        return Err("the name in the draft's front matter is lowercase letters, digits and '-' only".into());
    }
    styles::save(&s)?;
    Ok(s.name)
}

// ---------------------------------------------------------------------------------------------------
// MCP tools

fn row(s: &Style) -> Value {
    json!({"name": s.name, "title": s.title, "description": s.description, "source": s.source.label().to_lowercase(), "skill": s.skill})
}

fn rules_json(s: &Style) -> Value {
    let r = &s.rules;
    json!({"avg_shot": r.avg_shot, "cut_on_beat": r.cut_on_beat, "transitions": r.transitions, "captions": r.captions,
           "loudness": r.loudness, "grade": r.grade, "pacing": r.pacing, "words_per_minute": r.wpm})
}

/// `context.get`: the document, its sections, the applied style and the digest.
pub(super) fn context_json(p: &crate::model::Project) -> Value {
    let doc = &p.context;
    let sections: serde_json::Map<String, Value> = context::SECTIONS.iter().map(|s| (s.to_string(), json!(context::section(doc, s)))).collect();
    json!({"text": doc, "sections": sections, "style": context::applied_style(doc), "digest": context::digest(p)})
}

fn learn_json(j: &style_learn::LearnJob) -> Value {
    let (f, status, done, err) = job_view(j);
    let draft = j.result.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(styles::to_text);
    let state = match (done, &err) {
        (false, _) => "running",
        (true, Some(e)) if e == crate::engine::export::CANCELLED => "cancelled",
        (true, Some(_)) => "failed",
        (true, None) => "done",
    };
    json!({"job": j.id, "state": state, "progress": f, "status": status, "error": err, "file": j.opts.path.to_string_lossy(), "draft": draft})
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "context.get",
        desc: "Project Context: the user's Markdown document about this project (text, and its sections About, Rules, \
               Style guide, Links), the editing style applied to it (style), and the digest gathered from the planner, \
               notes, moodboard and open comments. Rules are the user's instructions: follow them.",
        args: &[],
        kind: ToolKind::Read,
        run: |app, _| Ok(ToolOutcome::Done(context_json(&app.project))),
    },
    ToolDef {
        name: "context.set",
        desc: "Change Project Context (one undo step): text replaces the whole document, or section (About, Rules, \
               Style guide, Links) + body replaces one section's text. Only when the user asks.",
        args: &["text:string:false:the whole Markdown document", "section:string:false:About, Rules, Style guide or Links", "body:string:false:the section's new text"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let changed = match (arg_str(args, "text"), arg_str(args, "section")) {
                (Some(t), _) => app.project.set_context(t),
                (None, Some(sec)) => app.project.set_context_section(sec, arg_str(args, "body").unwrap_or("")),
                (None, None) => return Err("give text, or section and body".into()),
            };
            Ok(ToolOutcome::Done(json!({"ok": true, "changed": changed})))
        },
    },
    ToolDef {
        name: "styles.list",
        desc: "Editing Styles in the library: [{name, title, description, source (yours / built-in), skill}]. \
               /style <name> (or styles.apply) writes one into Project Context.",
        args: &[],
        kind: ToolKind::Read,
        run: |_app, _| Ok(ToolOutcome::Done(json!(styles::all().iter().map(row).collect::<Vec<_>>()))),
    },
    ToolDef {
        name: "styles.get",
        desc: "One editing style: {name, title, description, source, skill, rules {avg_shot s, cut_on_beat 0..1, \
               transitions, captions, loudness LUFS, grade, pacing [avg shot per quarter], words_per_minute}, body, text}.",
        args: &["name:string:true:"],
        kind: ToolKind::Read,
        run: |_app, args| {
            let name = req(arg_str(args, "name"), "name")?;
            let s = styles::get(name).ok_or(format!("no style named '{name}'"))?;
            let mut v = row(&s);
            v["rules"] = rules_json(&s);
            v["body"] = json!(s.body);
            v["text"] = json!(styles::file_text(&s));
            Ok(ToolOutcome::Done(v))
        },
    },
    ToolDef {
        name: "styles.save",
        desc: "Save a style file to the user's library: text = Markdown with `---` front matter (name, title, \
               description, avg_shot, cut_on_beat, transitions [..], captions, loudness, grade, pacing [..], \
               words_per_minute) and prose guidance below. Replaces the user's own style of that name.",
        args: &["text:string:true:", "name:string:false:overrides the front matter's name"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let mut s = styles::parse("style", req(arg_str(args, "text"), "text")?, styles::Source::User, None);
            if let Some(n) = arg_str(args, "name") {
                s.name = n.to_string();
            }
            let path = styles::save(&s)?;
            app.toast(format!("Saved the {} style", s.name));
            Ok(ToolOutcome::Done(json!({"ok": true, "name": s.name, "path": path.to_string_lossy()})))
        },
    },
    ToolDef {
        name: "styles.delete",
        desc: "Delete one of the user's styles (built-ins can't be deleted).",
        args: &["name:string:true:"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let name = req(arg_str(args, "name"), "name")?;
            styles::delete(name)?;
            app.toast(format!("Deleted {name}"));
            Ok(ToolOutcome::Done(json!({"ok": true})))
        },
    },
    ToolDef {
        name: "styles.apply",
        desc: "Apply a style to this project (what /style <name> does): its guidance goes into Project Context's Style \
               guide, its rules into Rules, replacing any style applied before. One undo step.",
        args: &["name:string:true:"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let name = req(arg_str(args, "name"), "name")?;
            let s = styles::get(name).ok_or(format!("no style named '{name}'"))?;
            let changed = app.project.apply_context_style(&s.name, &styles::guide_markdown(&s), &styles::rules_markdown(&s));
            Ok(ToolOutcome::Done(json!({"ok": true, "changed": changed, "style": s.name})))
        },
    },
    ToolDef {
        name: "styles.remove",
        desc: "Take the applied style back out of Project Context (what the user wrote stays). One undo step.",
        args: &[],
        kind: ToolKind::Mutate,
        run: |app, _| Ok(ToolOutcome::Done(json!({"ok": true, "removed": app.project.remove_context_style()}))),
    },
    ToolDef {
        name: "styles.learn",
        desc: "Learn a style from a reference video (read-only): measures shot lengths, pacing, cuts on the beat, \
               transitions, loudness, colour and (speech: true, needs speech-to-text) words per minute; describe: true \
               also has the video describer write each shot's framing, subject, camera move and on-screen text \
               (styles.describer says whether it is installed). A background job: poll styles.learn_status {job}; its \
               draft is a style file to show the user, then styles.save once they approve.",
        args: &[
            "path:string:true:the video file",
            "name:string:false:the draft's name",
            "describe:boolean:false:",
            "model:string:false:256m (default) or 2.2b",
            "max_shots:number:false:most shots described (default 24)",
            "speech:boolean:false:",
        ],
        kind: ToolKind::Ui,
        run: |_app, args| {
            let path = std::path::PathBuf::from(req(arg_str(args, "path"), "path")?);
            if !path.is_file() {
                return Err(format!("{} is not a file", path.display()));
            }
            let describe = match arg_bool(args, "describe").unwrap_or(false) {
                false => None,
                true => {
                    let m = arg_str(args, "model").map(str::to_string).or_else(|| vlm::installed_models().last().cloned());
                    let m = m.filter(|m| vlm::has_model(m) && vlm::exe().is_some());
                    Some(m.ok_or("the video describer is not installed: ask the user, then styles.install_describer")?)
                }
            };
            let name = arg_str(args, "name").map(str::to_string).unwrap_or_else(|| path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default());
            let max_shots = arg_u64(args, "max_shots").unwrap_or(24).clamp(1, 200) as usize;
            let speech = arg_bool(args, "speech").unwrap_or(false);
            let j = style_learn::start(style_learn::Options { path, name, describe, max_shots, speech });
            Ok(ToolOutcome::Done(json!({"ok": true, "job": j.id})))
        },
    },
    ToolDef {
        name: "styles.learn_status",
        desc: "A styles.learn job: {job, state (running / done / failed / cancelled), progress 0..1, status, error, draft}. \
               Without job: the latest one.",
        args: &["job:number:false:"],
        kind: ToolKind::Read,
        run: |_app, args| {
            let j = match arg_u64(args, "job") {
                Some(id) => style_learn::job(id).ok_or(format!("no learn job {id}"))?,
                None => style_learn::latest().ok_or("no learn job yet")?,
            };
            Ok(ToolOutcome::Done(learn_json(&j)))
        },
    },
    ToolDef {
        name: "styles.describer",
        desc: "The video describer add-on (SmolVLM2 on llama.cpp): {installed, models (installed ids), installing \
               {progress, status}, error, sizes {256m, 2.2b} in bytes}.",
        args: &[],
        kind: ToolKind::Read,
        run: |_app, _| {
            let inst = vlm::installing().map(|p| json!({"progress": p.fraction(), "status": p.status()}));
            Ok(ToolOutcome::Done(json!({
                "installed": vlm::available(), "models": vlm::installed_models(), "installing": inst, "error": vlm::install_error(),
                "sizes": {"256m": vlm::install_size("256m"), "2.2b": vlm::install_size("2.2b")},
            })))
        },
    },
    ToolDef {
        name: "styles.install_describer",
        desc: "Download and install the video describer (a download of about 300 MB for 256m, 1.7 GB for 2.2b; ask the \
               user first). Returns at once; styles.describer shows the progress.",
        args: &["model:string:false:256m (default) or 2.2b"],
        kind: ToolKind::Ui,
        run: |_app, args| {
            let m = arg_str(args, "model").unwrap_or(vlm::DEFAULT);
            vlm::model_info(m).ok_or(format!("no describer model '{m}'"))?;
            vlm::start_install(m);
            Ok(ToolOutcome::Done(json!({"ok": true, "model": m})))
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    fn test_app() -> App {
        let scratch = std::env::temp_dir().join(format!("se-styles-ctl-test-{}", std::process::id()));
        *crate::settings::TEST_DIR.lock().unwrap_or_else(|e| e.into_inner()) = Some(scratch);
        let app = App::build(&egui::Context::default(), None, None);
        *crate::settings::TEST_DIR.lock().unwrap_or_else(|e| e.into_inner()) = None;
        app
    }

    fn run(app: &mut App, name: &str, args: Value) -> Result<Value, String> {
        let t = crate::mcp::tools::find(name).unwrap();
        let before = crate::ui::app::mcp_exec::snapshot_if_mutate(&app.project, t.kind);
        let r = (t.run)(app, &args)?;
        if let Some(b) = before {
            app.commit_tool_edit(b);
        }
        match r {
            ToolOutcome::Done(v) => Ok(v),
            _ => Err("not a plain reply".into()),
        }
    }

    /// Context set/get through the tools, and the document survives a save / load of the project.
    #[test]
    fn context_round_trips_through_tools_and_the_project_file() {
        let mut app = test_app();
        let v = run(&mut app, "context.get", json!({})).unwrap();
        assert_eq!((v["text"].as_str(), v["style"].is_null()), (Some(""), true));
        run(&mut app, "context.set", json!({"section": "About", "body": "A cooking channel."})).unwrap();
        run(&mut app, "context.set", json!({"section": "Rules", "body": "- No swearing."})).unwrap();
        let v = run(&mut app, "context.get", json!({})).unwrap();
        assert_eq!(v["sections"]["About"], "A cooking channel.");
        assert_eq!(v["sections"]["Rules"], "- No swearing.");
        assert_eq!(app.undo.len(), 2, "one undo step per call");
        let json = app.project.to_json();
        let back = crate::model::Project::from_json(&json).unwrap();
        assert_eq!(back.context, app.project.context);
        assert!(json.contains("A cooking channel."));
        // an untouched project does not write the field at all
        assert!(!crate::model::Project::new().to_json().contains("\"context\""));
    }

    /// `/style vlog` writes the style into Style guide + Rules as ONE undo step; Undo restores the user's
    /// text exactly; `/style off` removes it again (also one step); an unknown name changes nothing.
    #[test]
    fn slash_style_applies_and_undoes() {
        let mut app = test_app();
        app.project.set_context_section("Rules", "- Keep the logo in the corner.");
        let mine = app.project.context.clone();
        let undo0 = app.undo.len();
        slash(&mut app, "vlog");
        assert_eq!(applied(&app).as_deref(), Some("vlog"));
        let rules = context::section(&app.project.context, "Rules");
        assert!(rules.contains("Average shot length about 1.9 s") && rules.ends_with("- Keep the logo in the corner."), "{rules}");
        assert!(context::section(&app.project.context, "Style guide").contains("Fast-cut vlog"));
        assert_eq!(app.undo.len(), undo0 + 1, "one undo step");
        assert_eq!(app.undo.last().unwrap().label, "Apply style");
        app.act(Action::Undo);
        assert_eq!(app.project.context, mine, "undo restores the document exactly");
        app.act(Action::Redo);
        assert_eq!(applied(&app).as_deref(), Some("vlog"));
        slash(&mut app, "trailer");
        assert_eq!(applied(&app).as_deref(), Some("trailer"));
        assert_eq!(app.project.context.matches("<!-- style:").count(), 2, "replaced, not stacked");
        slash(&mut app, "off");
        assert_eq!(app.project.context, mine);
        assert_eq!(app.undo.last().unwrap().label, "Remove style");
        let n = app.undo.len();
        slash(&mut app, "no-such-style");
        assert_eq!(app.undo.len(), n);
        assert_eq!(app.assistant.prompt, "/style no-such-style", "kept for fixing");
        // the tools do the same
        run(&mut app, "styles.apply", json!({"name": "documentary"})).unwrap();
        assert_eq!(applied(&app).as_deref(), Some("documentary"));
        assert_eq!(run(&mut app, "styles.remove", json!({})).unwrap()["removed"], true);
        assert_eq!(app.project.context, mine);
    }

    #[test]
    fn styles_tools_list_get_and_learn_status() {
        let mut app = test_app();
        let list = run(&mut app, "styles.list", json!({})).unwrap();
        assert!(list.as_array().unwrap().iter().any(|s| s["name"] == "vlog" && s["source"] == "built-in"));
        let v = run(&mut app, "styles.get", json!({"name": "trailer"})).unwrap();
        assert_eq!(v["rules"]["pacing"].as_array().unwrap().len(), 4);
        assert!(v["text"].as_str().unwrap().starts_with("---\nname: trailer"));
        assert!(run(&mut app, "styles.learn", json!({"path": "Z:/nope.mp4"})).is_err());
        let d = run(&mut app, "styles.describer", json!({})).unwrap();
        assert!(d["sizes"]["256m"].as_u64().unwrap() > 200_000_000);
    }
}
