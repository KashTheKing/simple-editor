//! ---- ws:skills ----
//! The `skills.*` MCP tools (list / get / save / delete). They change files in the user's skills
//! folder, not the project: `ToolKind::Read` / `Ui`, no undo. `skills.save` / `skills.delete` count as
//! leaving the project for the Assistant's guard (`assistant_ctl::needs_approval`), except a save
//! during a `/create-skill` run the user started.

use super::tools_helpers::*;
use crate::engine::skills::{self, Skill};
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use serde_json::{json, Value};

fn row(s: &Skill) -> Value {
    json!({"name": s.name, "description": s.description, "args": s.args, "source": s.source.label().to_lowercase()})
}

/// `skills.get`: the whole file text plus the parsed parts.
pub(super) fn get_json(s: &Skill) -> Value {
    let mut v = row(s);
    v["body"] = json!(s.body);
    v["text"] = json!(skills::file_text(s));
    v
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "skills.list",
        desc: "Assistant skills (run in chat as /name arguments): [{name, description, args, source}]; source is \
               yours, library or built-in. skills.get{name} returns one's instructions.",
        args: &[],
        kind: ToolKind::Read,
        run: |_app, _args| Ok(ToolOutcome::Done(json!(skills::all().iter().map(row).collect::<Vec<_>>()))),
    },
    ToolDef {
        name: "skills.get",
        desc: "One assistant skill: {name, description, args, source, body, text} (text = the whole file). A \
               skill's body is a procedure to follow; $ARGS stands for the arguments.",
        args: &["name:string:true:"],
        kind: ToolKind::Read,
        run: |_app, args| {
            let name = req(arg_str(args, "name"), "name")?;
            let s = skills::get(name.trim_start_matches('/')).ok_or(format!("no skill named '{name}'"))?;
            Ok(ToolOutcome::Done(get_json(&s)))
        },
    },
    ToolDef {
        name: "skills.save",
        desc: "Save a user skill to the skills folder (replaces the user's own skill of that name). name: \
               lowercase letters, digits, '-'; description: one line; args: optional hint such as \"[style]\"; \
               body: Markdown instructions, $ARGS = what the user types after /name.",
        args: &["name:string:true:", "description:string:true:", "args:string:false:", "body:string:true:"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let name = req(arg_str(args, "name"), "name")?;
            let path = skills::save(
                name,
                req(arg_str(args, "description"), "description")?,
                arg_str(args, "args").unwrap_or(""),
                req(arg_str(args, "body"), "body")?,
            )?;
            app.toast(format!("Saved skill /{name}"));
            Ok(ToolOutcome::Done(json!({"ok": true, "name": name, "path": path.to_string_lossy()})))
        },
    },
    ToolDef {
        name: "skills.delete",
        desc: "Delete one of the user's own skills (built-in and library skills can't be deleted).",
        args: &["name:string:true:"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let name = req(arg_str(args, "name"), "name")?;
            skills::delete(name)?;
            app.toast(format!("Deleted skill /{name}"));
            Ok(ToolOutcome::Done(json!({"ok": true})))
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::skills::{delete_in, list_in, save_in, Source};

    /// What the tools return, through the same functions on a temp folder (never the real %APPDATA%):
    /// save -> list -> get -> delete.
    #[test]
    fn skills_tools_round_trip() {
        let dir = std::env::temp_dir().join(format!("se-skills-tools-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        save_in(&dir, "vlog-intro", "Make a vlog intro.", "[title]", "Add a title: $ARGS").unwrap();
        let all = list_in(&dir, &[]);
        let s = all.iter().find(|s| s.name == "vlog-intro").unwrap();
        assert_eq!(
            row(s),
            json!({"name": "vlog-intro", "description": "Make a vlog intro.", "args": "[title]", "source": "yours"})
        );
        let g = get_json(s);
        assert_eq!(g["body"], "Add a title: $ARGS");
        assert!(g["text"].as_str().unwrap().starts_with("---\nname: vlog-intro\n"));
        let b = all.iter().find(|s| s.source == Source::BuiltIn).unwrap();
        assert!(get_json(b)["text"].as_str().unwrap().starts_with("---"));
        delete_in(&dir, "vlog-intro", &all).unwrap();
        assert!(list_in(&dir, &[]).iter().all(|s| s.name != "vlog-intro"));
        let _ = std::fs::remove_dir_all(&dir);
        for (name, kind) in [
            ("skills.list", ToolKind::Read),
            ("skills.get", ToolKind::Read),
            ("skills.save", ToolKind::Ui),
            ("skills.delete", ToolKind::Ui),
        ] {
            assert_eq!(crate::mcp::tools::find(name).map(|d| d.kind), Some(kind), "{name}");
        }
    }
}
