//! ---- ws:online-library ----
//! The `library.*` MCP tools (list / install / apply / save / remove). `library.*` is the CONTENT
//! library (shader transitions and effects, LUTs, HTML templates, icons, skills), not the media
//! library panel (`library.select` / `library.columns`).

use super::library_ctl;
use super::tools_helpers::*;
use super::*;
use crate::engine::library::{self, Kind};
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};

fn id_arg(args: &Value) -> Result<(String, Kind, String), String> {
    let id = req(arg_str(args, "id"), "id")?;
    let (kind, short) = library::split_id(id)
        .ok_or_else(|| format!("no such library item '{id}' (ids look like transitions/paper-tear)"))?;
    Ok((id.to_string(), kind, short))
}

/// What is wrong with `text` as a library item of `kind` (empty = fine). Shaders: `shaderlib::lint`;
/// LUTs: `lut::parse_cube`; HTML / icons / skills: a basic check of the header the format requires.
/// ponytail: switch HTML to `engine::html::lint` and skills to the skills parser once they exist.
pub(super) fn check(kind: Kind, text: &str) -> Vec<String> {
    let lower = text.to_ascii_lowercase();
    let (name, desc, _, _) = library::meta_of(kind, text);
    let mut out = match kind {
        Kind::Effect | Kind::Transition => crate::engine::shaderlib::lint(text, kind == Kind::Transition),
        Kind::Lut => crate::engine::lut::parse_cube(text).err().into_iter().collect(),
        Kind::Html => crate::engine::html::lint(text),
        Kind::Icon | Kind::Skill => Vec::new(),
    };
    match kind {
        Kind::Html | Kind::Icon if lower.contains("<script") || lower.contains("javascript:") => {
            out.push("no JavaScript: remove <script> / javascript: URLs".into())
        }
        Kind::Icon if !lower.contains("<svg") => out.push("not an <svg> file".into()),
        Kind::Html if name.is_empty() || desc.is_empty() => out.push("needs a <title> and a se:desc meta tag".into()),
        Kind::Skill if name.is_empty() || desc.is_empty() => {
            out.push("needs a --- header with `name:` and `description:` lines".into())
        }
        _ => {}
    }
    out
}

fn kind_arg(args: &Value) -> Result<Option<Kind>, String> {
    match arg_str(args, "kind") {
        None | Some("") => Ok(None),
        Some(k) => Kind::from_name(k)
            .map(Some)
            .ok_or_else(|| format!("unknown kind '{k}' (effect, transition, lut, html, icon, skill)")),
    }
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "library.list",
        desc: "Items of the CONTENT library (shader transitions and effects, LUTs, HTML templates, icons, skills - not the media library panel): installed ones and the online catalogue, each with `installed`. The first call starts loading the catalogue: `catalogue` is \"loading\" until it arrives (call again).",
        args: &["kind:string:false:effect|transition|lut|html|icon|skill", "query:string:false:words matched in name / tags / description"],
        kind: ToolKind::Read,
        run: |app, args| {
            let kind = kind_arg(args)?;
            library_ctl::ensure(app, false);
            if app.online.cat.is_none() && app.online.installed.is_empty() {
                app.online.installed = library::installed_ids(); // until the first load arrives
            }
            let rows = app.online.rows(kind, arg_str(args, "query").unwrap_or(""));
            let items: Vec<Value> = rows
                .iter()
                .map(|r| {
                    json!({"id": r.id, "kind": r.kind.name(), "name": r.name, "desc": r.desc, "tags": r.tags,
                           "installed": r.installed, "yours": r.yours})
                })
                .collect();
            let state = match (&app.online.cat, app.online.loading()) {
                (_, true) => "loading".to_string(),
                (Some(_), _) if app.online.note.is_empty() => "ok".to_string(),
                _ => app.online.note.clone(),
            };
            Ok(ToolOutcome::Done(json!({"items": items, "catalogue": state})))
        },
    },
    ToolDef {
        name: "library.install",
        desc: "Download one CONTENT library item (not the media library) onto this PC, by id (`transitions/paper-tear`, from library.list). Replies when the file is installed.",
        args: &["id:string:true:<kind dir>/<file id>"],
        kind: ToolKind::Job,
        run: |app, args| {
            let (id, kind, short) = id_arg(args)?;
            let path = library::path_of(kind, &short);
            if path.is_file() {
                return Ok(ToolOutcome::Done(json!({"ok": true, "path": path, "status": "already installed"})));
            }
            let prog = library_ctl::start_download(app, &id, library_ctl::Then::Install);
            Ok(ToolOutcome::Job(prog, path))
        },
    },
    ToolDef {
        name: "library.apply",
        desc: "Use a CONTENT library item (not the media library): a transition goes on the clip's cut, an effect or LUT on the clip (default: the selection), an HTML template or icon is added as an HTML layer at `at` (default: the playhead), a skill is installed for the Assistant. An online item must be installed first (library.install). One undo step.",
        args: &[
            "id:string:true:<kind dir>/<file id>",
            "clip:integer:false:clip id (default: the selection)",
            "at:number:false:timeline seconds for HTML / icons (default: the playhead)",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let (id, _, _) = id_arg(args)?;
            let msg = library_ctl::apply(app, &id, arg_u64(args, "clip"), arg_f64(args, "at"))?;
            Ok(ToolOutcome::Done(json!({"ok": true, "message": msg})))
        },
    },
    ToolDef {
        name: "library.save",
        desc: "Save text as a CONTENT library item on this PC (not the media library): a shader effect / transition (checked like shader.check's header rules), a .cube LUT (must parse), an HTML template, an icon SVG or a skill (format: library/README.md). Replaces an item of the same name.",
        args: &[
            "kind:string:true:effect|transition|lut|html|icon|skill",
            "name:string:true:display name; the id is its kebab-case form",
            "text:string:true:the whole file",
        ],
        kind: ToolKind::Ui,
        run: |app, args| {
            let kind = kind_arg(args)?.ok_or("missing argument 'kind'")?;
            let name = req(arg_str(args, "name"), "name")?;
            let text = req(arg_str(args, "text"), "text")?;
            let bad = check(kind, text);
            if !bad.is_empty() {
                return Err(format!("not saved:\n{}", bad.join("\n")));
            }
            let it = library::save(kind, name, text)?;
            let id = library::full_id(kind, &it.id);
            app.online.installed.insert(id.clone());
            crate::ui::gallery::forget_lut_listing();
            Ok(ToolOutcome::Done(json!({"ok": true, "id": id, "path": it.path})))
        },
    },
    ToolDef {
        name: "library.remove",
        desc: "Delete an installed CONTENT library item from this PC (not the media library; online items can be installed again).",
        args: &["id:string:true:<kind dir>/<file id>"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let (id, kind, short) = id_arg(args)?;
            if !library::remove(kind, &short) {
                return Err(format!("{id} is not installed"));
            }
            app.online.installed.remove(&id);
            crate::ui::gallery::forget_lut_listing();
            Ok(ToolOutcome::Done(json!({"ok": true})))
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_save_checks_each_kind() {
        let shader = "// @name Glow\n// @desc Glows.\n// @tags light\nvec4 effect(vec4 src, vec2 uv) { return src; }\n";
        assert!(check(Kind::Effect, shader).is_empty(), "{:?}", check(Kind::Effect, shader));
        assert!(!check(Kind::Transition, shader).is_empty(), "an effect is not a transition");
        assert!(!check(Kind::Lut, "LUT_3D_SIZE 2\n0 0 0\n").is_empty());
        let html = "<title>Card</title><meta name=\"se:desc\" content=\"A card.\"><meta name=\"se:tags\" content=\"card\">                    <meta name=\"se:duration\" content=\"4s\"><body>Card</body>";
        assert!(check(Kind::Html, html).is_empty(), "{:?}", check(Kind::Html, html));
        assert!(!check(Kind::Html, &format!("{html}<script>x()</script>")).is_empty());
        assert!(!check(Kind::Html, "<div>no header</div>").is_empty());
        assert!(check(Kind::Icon, "<svg><title>A</title></svg>").is_empty());
        assert!(!check(Kind::Icon, "<div/>").is_empty());
        assert!(check(Kind::Skill, "---\nname: x\ndescription: Does x.\n---\nbody").is_empty());
        assert!(!check(Kind::Skill, "just text").is_empty());
    }

    #[test]
    fn library_tool_descriptions_say_content_library() {
        for t in TOOLS {
            assert!(t.desc.contains("CONTENT library") && t.desc.contains("media library"), "{}", t.name);
        }
    }
}
