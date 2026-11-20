//! ---- ws:assistant-luau ----
//! `scripts.eval`: the Assistant runs a Luau script straight away against the live project and library,
//! as ONE undo step labelled with the script's purpose. The script reaches the project and the library
//! through `tool` and the `library` / `project` / `query` API (`scripting::PRELUDE`) - and nothing
//! else: `script_may_call` admits project / library tools only (no files, exports, settings, network,
//! other scripts or UI), and the VM itself has no io / os / require (`scripting::run_api`). Because it
//! cannot leave the project, the Assistant guard does not hold it for Allow (`mcp::tools::leaves_project`
//! does not list it) - the user asked for exactly that; `tasks.run` / `scripts.run`, which run with full
//! access, stay held.

use super::tools_helpers::*;
use super::*;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::model::Asset;
use crate::scripting::{self, Limits};

/// Tool groups a script never reaches: files, exports, settings, the network, other scripts, the
/// assistant, background jobs, app UI and the content library's downloads.
const DENY_PREFIX: &[&str] = &[
    "export.", "scripts.", "tasks.", "skills.", "assistant.", "settings.", "hotkeys.", "profile.", "enhance.",
    "voice.", "jobs.", "caches.", "perf.", "ui.", "onboarding.", "layout.", "gallery.", "transcribe.",
    "cursor.", "tts.", "playback.", "edit.", "history.", "frame.", "render.layers", "render.poll",
];
/// Single tools a script never reaches (they read or write files, or reach the network).
const DENY: &[&str] = &[
    "project.open", "project.new", "project.save", "project.recover", "project.autosave",
    "library.list", "library.install", "library.apply", "library.save", "library.remove", "library.select",
    "library.columns", "media.convert", "media.batch_convert", "media.consolidate", "media.relink",
    "media.transcribe", "markers.export", "markers.import", "beats.export", "beats.import", "transcript.export",
    "subtitles.import", "templates.export", "templates.import", "timeline.import", "html.import", "html.snapshot",
    "browser.import", "keyblocks.save", "keyblocks.forget", "stills.save", "html.renderer",
];
/// An argument with one of these names points outside the project (a file, a folder on disk, a URL).
const PATH_ARGS: &[&str] = &["path", "paths", "dir", "file", "files", "out", "output", "url", "folder_path"];
/// Mutate tools that touch the library only: no timeline refresh needed after them.
const LIBRARY_ONLY: &[&str] = &["media.bulk_set", "library.bins", "media.set", "media.smart_bin"];

/// Whether a script may call `name`. Default-deny for anything new that is not plainly a project read
/// or edit: only Read / Mutate tools, none that leave the project or take a path.
pub(super) fn script_may_call(name: &str) -> bool {
    let Some(def) = mcp::tools::find(name) else { return false };
    if !matches!(def.kind, ToolKind::Read | ToolKind::Mutate) && name != "selection.set" && name != "selection.get" {
        return false;
    }
    if assistant_ctl::leaves_project(name, &Value::Null) || DENY.contains(&name) || DENY_PREFIX.iter().any(|p| name.starts_with(p)) {
        return false;
    }
    !def.args.iter().any(|a| PATH_ARGS.contains(&a.split(':').next().unwrap_or("")))
}

/// What a script changed, for its reply: assets re-filed / re-tagged, bins made or removed, clips edited.
fn diff(old: &Project, new: &Project) -> Value {
    let olds: std::collections::HashMap<Id, &Asset> = old.assets.iter().map(|a| (a.id, a)).collect();
    let mut moved = 0;
    let mut retagged = 0;
    let mut relabelled = 0;
    for a in &new.assets {
        if let Some(o) = olds.get(&a.id) {
            moved += (o.folder != a.folder) as usize;
            retagged += (o.tags != a.tags) as usize;
            relabelled += (o.label != a.label || o.description != a.description) as usize;
        }
    }
    let (of, nf) = (old.folder_names(), new.folder_names());
    let added: Vec<&String> = nf.iter().filter(|f| !of.contains(f)).collect();
    let removed: Vec<&String> = of.iter().filter(|f| !nf.contains(f)).collect();
    json!({
        "assets_moved": moved, "assets_retagged": retagged, "assets_relabelled": relabelled,
        "assets_added": new.assets.len().saturating_sub(old.assets.len()),
        "assets_removed": old.assets.len().saturating_sub(new.assets.len()),
        "bins_added": added, "bins_removed": removed,
        "clips_changed": tools_context::changed_clips(old, new).len(),
    })
}

/// The undo group a script's edits close into (`App::end_group` pushes it as one labelled entry).
pub(super) fn script_group(label: &str, before: String, undo: &[UndoEntry]) -> tools_context::UndoGroup {
    tools_context::UndoGroup {
        label: label.to_string(),
        before,
        undo_len: undo.len(),
        top_at: undo.last().map(|e| e.at),
        last_call: Instant::now(),
        idle: tools_context::GROUP_IDLE,
    }
}

/// Run `code` (see the tool's description). Ok = the reply; Err = the error plus the output so far, and
/// the project is exactly as it was.
pub(super) fn eval(app: &mut App, args: &Value) -> Result<Value, String> {
    let code = req(arg_str(args, "code"), "code")?;
    let purpose = arg_str(args, "purpose").map(str::trim).filter(|p| !p.is_empty()).unwrap_or("Script").to_string();
    let dry = arg_bool(args, "dry_run").unwrap_or(false);
    let d = Limits::default();
    let limits = Limits {
        wall: arg_u64(args, "budget_ms").map(|ms| Duration::from_millis(ms.clamp(100, 60_000))).unwrap_or(d.wall),
        output: arg_u64(args, "max_output").map(|n| (n as usize).clamp(1_000, 60_000)).unwrap_or(d.output),
        ..d
    };
    if assistant_ctl::running_task(app) {
        return Err("not from inside a running task".into());
    }
    let before = app.project.to_json();
    let timeline_edits = std::cell::Cell::new(false);
    let run = {
        let cell = std::cell::RefCell::new(&mut *app);
        let mut call = |tool: &str, a: &Value| -> Result<Value, String> {
            let mut app = cell.borrow_mut();
            let def = mcp::tools::find(tool).ok_or_else(|| format!("unknown tool '{tool}'"))?;
            // no per-call snapshot: the whole script is undone on failure (a 5 MB project's to_json per
            // call would eat the budget)
            match (def.run)(&mut app, a)? {
                ToolOutcome::Done(v) => {
                    if def.kind == ToolKind::Mutate && !LIBRARY_ONLY.contains(&tool) {
                        timeline_edits.set(true);
                        app.after_edit(); // gap closing / span refresh between calls, as a hand edit would
                    }
                    Ok(v)
                }
                ToolOutcome::Image { png_b64, meta } => Ok(mcp::tools::image_json(&png_b64, meta)),
                ToolOutcome::Job(..) | ToolOutcome::Wait(_) => Err(format!("'{tool}' starts a background job")),
            }
        };
        scripting::run_api(code, "script", &purpose, &limits, &script_may_call, &mut call)
    };
    let now = app.project.to_json();
    let changed = now != before;
    let old = Project::from_json(&before).map_err(|e| format!("internal: {e}"))?;
    let what = changed.then(|| diff(&old, &app.project));
    let restore = |app: &mut App, old: Project| {
        app.project = old;
        app.after_edit();
    };
    if let Some(err) = &run.error {
        if changed {
            restore(app, old);
        }
        let mut msg = format!("{err}\n(the script's edits were undone - the project is as it was)");
        if !run.output.is_empty() {
            msg.push_str(&format!("\n--- output ---\n{}", run.output));
        }
        return Err(msg);
    }
    let mut undo = Value::Null;
    if changed && dry {
        restore(app, old);
    } else if changed {
        if !timeline_edits.get() {
            app.after_edit();
        }
        let label = format!("Script: {purpose}");
        if app.mcp_ctx.group.is_some() {
            // inside edit.begin / an Assistant request: the script joins that one undo step
            undo = json!(app.mcp_ctx.group.as_ref().map(|g| g.label.clone()));
        } else {
            app.mcp_ctx.group = Some(script_group(&label, before, &app.undo));
            app.end_group(); // ONE labelled entry, changed clips glow, Undo toast
            undo = json!(label);
        }
    }
    let mut reply = json!({
        "ok": true,
        "purpose": purpose,
        "changed": changed,
        "dry_run": dry,
        "undo": undo,
        "diff": what,
        "output": run.output,
        "result": run.result,
        "stats": {"ms": run.ms, "steps": run.steps, "calls": run.calls},
    });
    if run.truncated {
        reply["truncated"] = json!(scripting::TRUNCATED.trim());
    }
    Ok(reply)
}

pub const TOOLS: &[ToolDef] = &[ToolDef {
    name: "scripts.eval",
    desc: "Run a Luau script now, against the live project and its library, as ONE undo step labelled \
           'Script: <purpose>' (inside an Assistant request it joins the request's step). If it errors, all its \
           edits are undone. dry_run: run it, report, then undo everything. Use it for library-wide or bulk \
           work - organising, tagging, renaming, auditing, many clip edits. Sandboxed: project and library \
           only - no files, folders, exports, settings, network, require or other scripts, so it needs no \
           Allow. Budget: budget_ms wall-clock (default 10000), an instruction cap and 256 MB of memory. \
           print(...) (tables print as JSON), warn(...) (a 'warning: ' line) and the chunk's return value come \
           back, capped at max_output characters (default 16000) with a 'truncated, refine your query' note; \
           error(...) fails the run.\n\
           API: tool(name, args) calls a project / library tool (same names and args as your tools; tools() \
           lists the ones a script may call). library.assets(filter) -> every asset with id, \
           name, path, ext, kind (video|audio|image|html), duration, width, height, fps, codec, size, tags, \
           label, folder, description, used, uses (filter: kind, folder, nested, used, query, tags, label, \
           ext, untagged, ids); library.get(id); library.summary(); library.set(ids, {folder, tags, add_tags, \
           remove_tags, label, description}); library.update(filter, fields); library.move(ids, bin); \
           library.tag(ids, tags); library.bins(); library.create_bin(bin); library.rename_bin(bin, name); \
           library.move_bin(bin, parent); library.delete_bin(bin) (assets go to the root, never deleted); \
           library.linked() / library.linked_files(bin, sub) (read-only listing of linked folders); \
           project.get() / summary() / timeline() / clips() / tracks() / markers() / subtitles() / \
           sequences() / spans() / blocks(clip_id); query.filter(list, fn | {field = value}) / find / map / \
           pluck / ids / sort(list, key, desc) / group_by(list, key) / count_by / sum / keys / \
           has(text, words). Bins nest with '/' (\"Audio/SFX\").",
    args: &[
        "code:string:true:Luau source",
        "purpose:string:true:what it does, a few words - the undo label",
        "dry_run:boolean:false:report what it would change, then undo it",
        "budget_ms:integer:false:wall-clock budget, 100..60000 (default 10000)",
        "max_output:integer:false:characters of output, 1000..60000 (default 16000)",
    ],
    kind: ToolKind::Ui,
    run: |app, args| Ok(ToolOutcome::Done(eval(app, args)?)),
}];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_reach_the_project_and_library_only() {
        for ok in [
            "media.list", "media.summary", "media.bulk_set", "library.bins", "media.linked", "media.set",
            "timeline.list", "timeline.move", "clip.set", "markers.add", "subtitles.set", "project.get",
            "sequence.list", "keyblocks.list", "track.list",
        ] {
            assert!(script_may_call(ok), "{ok} should be callable");
        }
        for no in [
            "export.video", "project.open", "project.save", "settings.set", "scripts.run", "scripts.eval",
            "tasks.run", "media.import", "media.import_folder", "media.convert", "library.install", "profile.fetch",
            "skills.save", "ui.action", "assistant.attachment", "frame.export", "markers.export", "html.import",
            "no.such.tool",
        ] {
            assert!(!script_may_call(no), "{no} must not be callable");
        }
        // nothing a script can call takes a path / url argument
        for def in mcp::tools::all().filter(|d| script_may_call(d.name)) {
            for a in def.args {
                let n = a.split(':').next().unwrap_or("");
                assert!(!PATH_ARGS.contains(&n), "{} takes {n}", def.name);
            }
        }
    }

    #[test]
    fn eval_is_not_held_but_full_access_runs_are() {
        let none = json!({});
        assert!(!assistant_ctl::leaves_project("scripts.eval", &none));
        assert!(assistant_ctl::leaves_project("tasks.run", &none));
        assert!(assistant_ctl::leaves_project("scripts.run", &none));
    }

    /// A real script against a real project: it creates bins and files assets through the library tools,
    /// and the whole run becomes ONE labelled undo entry that restores the project exactly.
    #[test]
    fn script_creates_bins_and_moves_assets_as_one_undo_step() {
        use super::super::media_api;
        let mut p = media_api::tests::sample();
        let before = p.to_json();
        let code = r#"
            library.create_bin("Audio/SFX")
            local audio = library.assets({kind = "audio"})
            library.move(query.ids(query.filter(audio, function(a) return query.has(a.name, "whoosh") end)), "Audio/SFX")
            library.update({kind = "video"}, {folder = "Footage", add_tags = {"footage"}})
            library.rename_bin("Old", "Archive")
            print(#library.bins())
            return library.summary().bins_total
        "#;
        let run = {
            let mut call = |tool: &str, a: &Value| -> Result<Value, String> {
                assert!(script_may_call(tool), "{tool}");
                match tool {
                    "media.list" => media_api::list(&p, a),
                    "media.summary" => Ok(media_api::summary(&p)),
                    "media.bulk_set" => media_api::bulk_set(&mut p, a),
                    "library.bins" => media_api::bins(&mut p, a),
                    t => Err(format!("unexpected {t}")),
                }
            };
            scripting::run_api(code, "t", "Sort audio", &Limits::default(), &script_may_call, &mut call)
        };
        assert_eq!(run.error, None, "{}", run.output);
        assert_eq!(p.asset(2).unwrap().folder, "Audio/SFX");
        assert_eq!((p.asset(1).unwrap().folder.as_str(), p.asset(5).unwrap().folder.as_str()), ("Footage", "Footage"));
        assert_eq!(p.asset(4).unwrap().folder, "Archive/Nested");
        assert_eq!(run.result, Some(json!(5))); // Archive, Archive/Nested, Audio, Audio/SFX, Footage
        let (mut undo, mut redo) = (Vec::new(), Vec::new());
        let g = script_group("Script: Sort audio", before.clone(), &undo);
        assert!(tools_context::push_group_entry(&mut undo, &mut redo, &g, &p.to_json()));
        assert_eq!(undo.len(), 1);
        assert_eq!(undo[0].label, "Script: Sort audio");
        assert_eq!(undo[0].json, before); // undoing it restores the project as it was
    }

    #[test]
    fn diff_counts_moves_and_bins() {
        let old = super::super::media_api::tests::sample();
        let mut new = old.clone();
        new.assets[0].folder = "Footage".into();
        new.assets[1].tags.push("x".into());
        let d = diff(&old, &new);
        assert_eq!(d["assets_moved"], 1);
        assert_eq!(d["assets_retagged"], 1);
        assert_eq!(d["bins_added"], json!(["Footage"]));
    }
}
