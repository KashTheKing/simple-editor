//! ---- ws:interchange ----
//! EDL export (`Action::ExportEdl`, File ▸ Export ▸ "Export Avid EDL…") and the `export.edl` /
//! `export.xml` MCP tools. `Read`: writing a file changes nothing in the Project.

use super::tools_helpers::*;
use super::*;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use std::path::{Path, PathBuf};

/// Write one EDL per video track: the first at `path`, the others beside it as `<stem>_V2.edl` ….
pub(super) fn write_edls(project: &Project, path: &Path) -> Result<Vec<PathBuf>, String> {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "timeline".into());
    let mut written = Vec::new();
    for (i, (tag, text)) in crate::engine::edl::export_edl(project).into_iter().enumerate() {
        let out = if i == 0 { path.to_path_buf() } else { path.with_file_name(format!("{stem}_{tag}.edl")) };
        std::fs::write(&out, text).map_err(|e| format!("{}: {e}", out.display()))?;
        written.push(out);
    }
    Ok(written)
}

pub(super) fn act(app: &mut App, a: Action) -> bool {
    if !matches!(a, Action::ExportEdl) {
        return false;
    }
    let Some(out) = rfd::FileDialog::new()
        .add_filter("CMX 3600 EDL (Avid, Premiere, Resolve)", &["edl"])
        .set_file_name(format!("{}.edl", app.project.name))
        .save_file()
    else {
        return true;
    };
    match write_edls(&app.export_project(), &out) {
        Ok(files) => app.toast_with_folder(
            format!("{} EDL file(s) exported - in Avid use File > Import, in Premiere File > Import", files.len()),
            out,
        ),
        Err(e) => app.toast(format!("EDL export failed: {e}")),
    }
    true
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "export.edl",
        desc: "Write the main timeline as CMX 3600 EDL (for Avid): one file per video track (the first at path, \
               others as <stem>_V2.edl …), the first also carrying every audio track; dissolves as D events.",
        args: &["path:string:true:.edl file to write"],
        kind: ToolKind::Read,
        run: |app, args| {
            let path = PathBuf::from(req(arg_str(args, "path"), "path")?);
            let files = write_edls(&app.export_project(), &path)?;
            let files: Vec<String> = files.iter().map(|p| p.to_string_lossy().into_owned()).collect();
            Ok(ToolOutcome::Done(json!({"ok": true, "files": files})))
        },
    },
    ToolDef {
        name: "export.xml",
        desc: "Write the main timeline as Final Cut Pro 7 XML (xmeml v5) for Premiere / Resolve: clips, audio \
               level/pan, A/V links, cross dissolves and sequence + clip markers (track names are not part of xmeml).",
        args: &["path:string:true:.xml file to write"],
        kind: ToolKind::Read,
        run: |app, args| {
            let path = PathBuf::from(req(arg_str(args, "path"), "path")?);
            std::fs::write(&path, crate::engine::xmeml::export_xmeml(&app.export_project()))
                .map_err(|e| e.to_string())?;
            Ok(ToolOutcome::Done(json!({"ok": true, "path": path.to_string_lossy()})))
        },
    },
];
