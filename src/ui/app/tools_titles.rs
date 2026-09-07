//! ---- ws:text-titles ----
//! 4 MCP tools for this workstream: `titles.list`/`titles.place` (the Gallery Titles tab's tool
//! surface - `App::place_title_template`, actions.rs, is the UI-path twin that additionally does its
//! own push_undo/selection/after_edit, since `run_tool_undoable` only wraps the tool path), `text.animate`
//! (apply/merge a motion preset onto a clip, the Text inspector's Animation row's tool twin -
//! `presets::apply_motion`/`merge_motion` directly, same as `inspector_text::section`'s Apply button),
//! `templates.expose` (rewrite a saved user template's `Clip.exposed`).
//!
//! DEVIATION from the plan's MCP table (see PR body): `templates.expose` is `ToolKind::Ui`, not
//! `Mutate`. `run_tool_undoable`'s snapshot/undo is `self.project.to_json()`-diff based (mcp_exec.rs
//! `run_snapshot_if_mutate`/`run_rollback`) - it can only ever detect and undo a PROJECT change.
//! `templates.expose` rewrites `Settings.templates`, not `Project`, so a `Mutate` kind would silently
//! push zero undo entries (project JSON never changes) and never roll back a failed partial write
//! either. `tools_gallery.rs`'s `inspector.folds` - also a `Settings`-only write - is the exact
//! precedent this follows: `ToolKind::Ui`, `settings.save()` inline, no undo.

use super::tools_args::Args;
use super::tools_helpers::*;
use super::*;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};

fn done(v: Value) -> Result<ToolOutcome, String> {
    Ok(ToolOutcome::Done(v))
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "titles.list",
        desc: "List placeable title/text templates: builtin_titles() + Settings.templates filtered by is_text_template. Returns [{name, clip_count, exposed: [string]}].",
        args: &[],
        kind: ToolKind::Read,
        run: |app, _args| {
            let list: Vec<Value> = crate::engine::presets::builtin_titles()
                .into_iter()
                .chain(
                    app.settings
                        .templates
                        .iter()
                        .filter(|t| crate::engine::presets::is_text_template(t))
                        .cloned(),
                )
                .filter_map(|t| {
                    let (clips, _) = crate::engine::presets::decode_template(&t)?;
                    let exposed: Vec<String> = clips.iter().flat_map(|c| c.exposed.iter().cloned()).collect();
                    Some(json!({"name": t.name, "clip_count": clips.len(), "exposed": exposed}))
                })
                .collect();
            done(json!(list))
        },
    },
    ToolDef {
        name: "titles.place",
        desc: "Decode the named title template (builtin or Settings.templates) and place it at `at` (or the playhead). Returns {clip_ids: [Id], exposed: [{clip_id, field}]}.",
        args: &["name:string:true:template name from titles.list", "at:number:false:seconds, defaults to playhead"],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let name = req(Args(args).str("name"), "name")?;
            let at = Args(args).t_or_playhead("at", app);
            let tpl = crate::engine::presets::builtin_titles()
                .into_iter()
                .chain(app.settings.templates.iter().cloned())
                .find(|t| t.name == name)
                .ok_or_else(|| format!("no title template '{name}'"))?;
            let (clips, assets) =
                crate::engine::presets::decode_template(&tpl).ok_or("title template is corrupted")?;
            // positional zip against the PRE-place clips' `.exposed` - verified 1:1 only for
            // Text/Shape/Adjustment templates (builtin_titles() and is_text_template both guarantee
            // that kind set), length-checked defensively rather than assumed.
            let exposed_by_index: Vec<Vec<String>> = clips.iter().map(|c| c.exposed.clone()).collect();
            let ids = app.project.place_clips(clips, assets, at);
            let exposed: Vec<Value> = if ids.len() == exposed_by_index.len() {
                ids.iter()
                    .zip(exposed_by_index.iter())
                    .flat_map(|(id, fields)| fields.iter().map(move |f| json!({"clip_id": id, "field": f})))
                    .collect()
            } else {
                Vec::new()
            };
            done(json!({"clip_ids": ids, "exposed": exposed}))
        },
    },
    ToolDef {
        name: "text.animate",
        desc: "Apply a motion preset's keyframes to a clip's Position/Scale/Rotation/Opacity - apply_motion (replace, stretched to the clip's length) or merge_motion (merge, layered on from the playhead) when merge is true. Same call `inspector_text::section`'s Animation-row Apply button makes.",
        args: &[
            "clip_id:integer:true:target clip",
            "preset:string:true:a builtin_motions() or Settings.motion_presets name",
            "merge:boolean:false:merge instead of replace (default false)",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let id = req(Args(args).id("clip_id"), "clip_id")?;
            let preset_name = req(Args(args).str("preset"), "preset")?;
            let merge = Args(args).bool("merge").unwrap_or(false);
            let preset = crate::engine::presets::builtin_motions()
                .into_iter()
                .chain(app.settings.motion_presets.iter().cloned())
                .find(|m| m.name.eq_ignore_ascii_case(preset_name))
                .ok_or_else(|| format!("no motion preset '{preset_name}'"))?;
            let playhead = app.playhead;
            let c = app.project.clip_mut(id).ok_or("no such clip")?;
            if merge {
                let off = c.local(playhead).clamp(0.0, c.duration);
                crate::engine::presets::merge_motion(&preset, c, off);
            } else {
                crate::engine::presets::apply_motion(&preset, c, true);
            }
            done(json!({"ok": true}))
        },
    },
    ToolDef {
        name: "templates.expose",
        desc: "Rewrite a saved user template's captured clips, setting Clip.exposed on each addressed clip_index (position within the template's own clip list, not a live id). Settings-level (Settings.templates) - see this file's DEVIATION doc comment for why this is ToolKind::Ui, not Mutate.",
        args: &["name:string:true:a Settings.templates entry", "fields:array:true:[{clip_index:integer, field:string}] to mark exposed"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let name = req(arg_str(args, "name"), "name")?;
            let raw = args.get("fields").and_then(|v| v.as_array()).ok_or("fields: array")?;
            let mut fields = Vec::with_capacity(raw.len());
            for f in raw {
                let idx = f.get("clip_index").and_then(|v| v.as_u64()).ok_or("fields[].clip_index: integer")?;
                let field = f.get("field").and_then(|v| v.as_str()).ok_or("fields[].field: string")?;
                fields.push((idx as usize, field.to_string()));
            }
            let i = app
                .settings
                .templates
                .iter()
                .position(|t| t.name == name)
                .ok_or_else(|| format!("no template '{name}'"))?;
            let rewritten = crate::engine::presets::expose_fields(&app.settings.templates[i], &fields)
                .ok_or("template is corrupted, or a clip_index is out of range")?;
            app.settings.templates[i] = rewritten;
            app.settings.save();
            done(json!({"ok": true}))
        },
    },
];

#[cfg(test)]
mod tests {
    use crate::model::Project;
    use crate::settings::Template;

    fn two_clip_template(exposed_on_second: &str) -> Template {
        let mut p = Project::new();
        let a_id = p.add_text_clip(0.0, 2.0);
        let b_id = p.add_text_clip(3.0, 2.0);
        p.clip_mut(b_id).unwrap().exposed = vec![exposed_on_second.to_string()];
        crate::engine::presets::capture_template("Fixture", &p, &[a_id, b_id])
    }

    /// `titles.place` on a 2-clip template with `exposed` on the SECOND clip must resolve to the
    /// second RETURNED id, not the first - the whole point of the positional zip.
    #[test]
    fn titles_place_resolves_exposed_to_live_ids() {
        let mut app_project = Project::new();
        let tpl = two_clip_template("text.text");
        let (clips, assets) = crate::engine::presets::decode_template(&tpl).unwrap();
        let exposed_by_index: Vec<Vec<String>> = clips.iter().map(|c| c.exposed.clone()).collect();
        let ids = app_project.place_clips(clips, assets, 0.0);
        assert_eq!(ids.len(), 2);
        let exposed: Vec<(u64, String)> = ids
            .iter()
            .zip(exposed_by_index.iter())
            .flat_map(|(id, fields)| fields.iter().map(move |f| (*id, f.clone())))
            .collect();
        assert_eq!(exposed, vec![(ids[1], "text.text".to_string())], "exposed must land on the 2nd id, not the 1st");
    }

    /// `templates.expose`'s pure body (`presets::expose_fields`): only the addressed `clip_index` gains
    /// the field, the other clip is untouched, and the result re-encodes to valid JSON.
    #[test]
    fn templates_expose_rewrites_only_addressed_clip() {
        let tpl = two_clip_template("text.text"); // clip 1 already exposes text.text
        let rewritten = crate::engine::presets::expose_fields(&tpl, &[(1, "text.color".into())]).unwrap();
        let (clips, _) = crate::engine::presets::decode_template(&rewritten).unwrap();
        assert!(clips[0].exposed.is_empty(), "clip_index 0 must stay untouched");
        assert_eq!(clips[1].exposed, vec!["text.text".to_string(), "text.color".to_string()]);
        // re-encodes to valid, re-decodable JSON
        assert!(serde_json::from_str::<serde_json::Value>(&rewritten.json).is_ok());
    }

    #[test]
    fn expose_fields_out_of_range_index_is_none() {
        let tpl = two_clip_template("text.text");
        assert!(crate::engine::presets::expose_fields(&tpl, &[(99, "text.text".into())]).is_none());
    }
}
