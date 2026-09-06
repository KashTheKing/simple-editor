use super::tools_helpers::*;
use super::*;

pub(super) fn dispatch(app: &mut App, name: &str, args: &Value) -> Option<Result<Value, String>> {
    let prefix = name.split('.').next().unwrap_or("");
    if !matches!(prefix, "media") {
        return None;
    }
    pub(super) fn run(app: &mut App, name: &str, args: &Value) -> Result<Value, String> {
        match name {
            "media.import" => {
                let paths: Vec<PathBuf> = req(args.get("paths").and_then(|v| v.as_array()), "paths")?
                    .iter()
                    .filter_map(|v| v.as_str().map(PathBuf::from))
                    .collect();
                let ids = app.import_files(&paths);
                Ok(json!({"ok": true, "asset_ids": ids}))
            }
            "media.list" => {
                let used = app.project.used_assets();
                let list: Vec<Value> = app
                    .project
                    .assets
                    .iter()
                    .map(|a| {
                        json!({
                            "id": a.id, "path": a.path, "kind": format!("{:?}", a.kind),
                            "duration": a.duration, "width": a.width, "height": a.height,
                            "tags": a.tags, "label": a.label, "folder": a.folder,
                            "description": a.description, "used": used.contains(&a.id),
                        })
                    })
                    .collect();
                Ok(json!(list))
            }
            "media.set" => {
                let id = req(arg_u64(args, "id"), "id")?;
                let a = app.project.asset_mut(id).ok_or("no such asset")?;
                if let Some(d) = arg_str(args, "description") {
                    a.description = d.to_string();
                }
                if let Some(tags) = args.get("tags").and_then(|v| v.as_array()) {
                    a.tags = tags.iter().filter_map(|t| t.as_str().map(String::from)).collect();
                }
                if let Some(l) = arg_u64(args, "label") {
                    a.label = l.min(8) as u8;
                }
                if let Some(f) = arg_str(args, "folder") {
                    a.folder = f.to_string();
                }
                Ok(json!({"ok": true}))
            }
            _ => unreachable!(),
        }
    }
    Some(run(app, name, args))
}
