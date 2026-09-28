//! ---- ws:jobs-panel ----
//! The four `jobs.*` MCP tools over the Jobs pane's snapshot. `Ui`, not `Mutate`, for the same
//! documented reason as `export.queue` in tools_export.rs: every holder here is `App` state, not the
//! `Project` — `Mutate`'s snapshot/undo would dirty a clean project for no project change.

use super::tools_helpers::*;
use super::*;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::ui::jobs_ui::{JobRow, JobState};

/// Pure: the same rows the pane shows, as JSON.
pub(super) fn rows_json(rows: &[JobRow]) -> Value {
    Value::Array(
        rows.iter()
            .map(|r| {
                let (state, error) = match &r.state {
                    JobState::Queued => ("queued", None),
                    JobState::Running => ("running", None),
                    JobState::Done => ("done", None),
                    JobState::Cancelled => ("cancelled", None),
                    JobState::Failed(e) => ("failed", Some(e.clone())),
                };
                json!({
                    "id": r.id,
                    "kind": r.kind.name(),
                    "label": r.label,
                    "state": state,
                    "error": error,
                    "fraction": r.fraction,
                    "status": r.status,
                    "eta_s": r.eta.map(|d| d.as_secs_f64()),
                    "elapsed_s": r.elapsed.as_secs_f64(),
                    "can_cancel": r.can_cancel,
                    "can_reorder": r.can_reorder,
                })
            })
            .collect(),
    )
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "jobs.list",
        desc: "Every background job the Jobs pane shows (running / queued), with progress, ETA and whether it can be cancelled or reordered.",
        args: &[],
        kind: ToolKind::Read,
        run: |app, _| Ok(ToolOutcome::Done(json!({"jobs": rows_json(&app.jobs.last), "running": app.jobs.running()}))),
    },
    ToolDef {
        name: "jobs.cancel",
        desc: "Cancel a job by its jobs.list id (sets its cancel flag / drops a queued export / stops a recording); errors for ids that cannot be cancelled.",
        args: &["id:string:true:"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let id = req(arg_str(args, "id"), "id")?;
            jobs_pane::cancel(app, id)?;
            Ok(ToolOutcome::Done(json!({"ok": true})))
        },
    },
    ToolDef {
        name: "jobs.reorder",
        desc: "Move a queued export (id 'queue:<i>') by delta positions in the render queue (only the export queue has an order).",
        args: &["id:string:true:", "delta:integer:true:e.g. -1 = one place earlier"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let id = req(arg_str(args, "id"), "id")?;
            let delta = req(args.get("delta").and_then(Value::as_i64), "delta")? as isize;
            if !jobs_pane::reorder_queue(&mut app.export_queue, id, delta) {
                return Err(format!("'{id}' is not a queued export that can move by {delta}"));
            }
            Ok(ToolOutcome::Done(json!({"ok": true})))
        },
    },
    ToolDef {
        name: "jobs.proxy_next",
        desc: "Build this source's proxy next (path = the asset path); it must be a video above proxy size without a proxy yet.",
        args: &["path:string:true:"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let path = req(arg_str(args, "path"), "path")?;
            jobs_pane::proxy_next(app, path)?;
            Ok(ToolOutcome::Done(json!({"ok": true})))
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::jobs_ui::JobKind;

    #[test]
    fn jobs_list_matches_pane_rows() {
        let mut a = JobRow::new(JobKind::Convert, 0, "a");
        a.fraction = Some(0.25);
        let mut b = JobRow::new(JobKind::QueuedExport, 0, "b");
        b.state = JobState::Queued;
        let rows = vec![a, b];
        let v = rows_json(&rows);
        let arr = v.as_array().unwrap();
        assert_eq!(arr.len(), rows.len());
        for (j, r) in arr.iter().zip(&rows) {
            assert_eq!(j["id"], r.id);
            assert_eq!(j["kind"], r.kind.name());
            assert_eq!(j["can_cancel"], r.can_cancel);
            assert_eq!(j["can_reorder"], r.can_reorder);
        }
        assert_eq!(arr[0]["state"], "running");
        assert_eq!(arr[0]["fraction"], 0.25);
        assert_eq!(arr[1]["state"], "queued");
        for t in TOOLS {
            assert!(t.name.starts_with("jobs."));
            assert!(matches!(t.kind, ToolKind::Read | ToolKind::Ui), "{} never touches the Project", t.name);
        }
    }

    #[test]
    fn jobs_cancel_unknown_id_errors() {
        let jobs = vec![(JobKind::Convert, "c".to_string(), Progress::new())];
        let mut q: std::collections::VecDeque<u8> = std::collections::VecDeque::new();
        assert!(jobs_pane::cancel_core(&jobs, &mut q, "convert:3").is_err());
        assert!(jobs_pane::cancel_core(&jobs, &mut q, "queue:0").is_err());
        assert!(jobs_pane::cancel_core(&jobs, &mut q, "").is_err());
        assert!(!jobs[0].2.is_cancelled());
    }
}
