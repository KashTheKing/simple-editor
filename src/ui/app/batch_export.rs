//! ---- ws:parity-editing ----
//! Library ▸ Export Each As: every selected asset written to one folder in one format (Shotcut's batch
//! export from the playlist). One background job runs the files one after another
//! (`convert::start_batch`); it shows in the Jobs pane, and a toast with the folder says when it is done.
//! Unlike Convert To, nothing is imported back into the Library: this is delivery, not editing.

use super::*;
use crate::engine::convert::{self, ConvertOptions};
use std::sync::{Arc, Mutex};

struct Batch {
    prog: Arc<Progress>,
    dir: PathBuf,
    count: usize,
}

static JOBS: Mutex<Vec<Batch>> = Mutex::new(Vec::new());

fn jobs() -> std::sync::MutexGuard<'static, Vec<Batch>> {
    JOBS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Rows for the Jobs pane.
pub(super) fn progress_jobs() -> Vec<jobs_pane::ProgJob> {
    jobs()
        .iter()
        .map(|b| (crate::ui::jobs_ui::JobKind::Convert, format!("Batch export · {} files", b.count), b.prog.clone()))
        .collect()
}

/// The conversions for `assets` into `dir` as `ext`, with the export settings' encoder and quality.
/// Assets without a file (HTML layers, titles) are skipped.
pub(super) fn plan(project: &Project, settings: &Settings, assets: &[Id], dir: &Path, ext: &str) -> Vec<ConvertOptions> {
    let mut taken: Vec<PathBuf> = Vec::new();
    assets
        .iter()
        .filter_map(|&id| project.asset(id))
        .filter(|a| !a.path.is_empty() && a.html.is_none())
        .map(|a| {
            let src = PathBuf::from(&a.path);
            let mut out = convert::batch_out_path(dir, &src, ext);
            // two sources with one name in a batch: number the second one too
            let mut n = 2;
            while taken.contains(&out) {
                let stem = src.file_stem().unwrap_or_default().to_string_lossy();
                out = dir.join(format!("{stem} ({n}).{ext}"));
                n += 1;
            }
            taken.push(out.clone());
            ConvertOptions {
                audio_streams: None,
                src,
                out,
                encoder: settings.encoder.clone(),
                crf: settings.crf,
                preset: settings.preset.clone(),
                out_size: None,
                scaler: settings.export_scaler.clone(),
                gif_fps: 15,
                target_bytes: None,
                vf_extra: None,
                af_extra: None,
                range: a.range, // a subclip exports just its part
            }
        })
        .collect()
}

/// Ask for a folder, then start the batch.
pub(super) fn start(app: &mut App, assets: &[Id], ext: &str) {
    if app.ffmpeg_missing() {
        return;
    }
    let Some(dir) = rfd::FileDialog::new().set_title("Export each selected file to…").pick_folder() else { return };
    let list = plan(&app.project, &app.settings, assets, &dir, ext);
    if list.is_empty() {
        app.toast("Nothing to export: the selection has no media files");
        return;
    }
    let count = list.len();
    app.toast(format!("Exporting {count} file{} as {ext}…", if count == 1 { "" } else { "s" }));
    jobs().push(Batch { prog: convert::start_batch(list), dir, count });
}

/// Toast the batches that finished (called with the other job polls).
pub(super) fn poll(app: &mut App) {
    let done: Vec<Batch> = {
        let mut j = jobs();
        let (done, keep): (Vec<Batch>, Vec<Batch>) = std::mem::take(&mut *j).into_iter().partition(|b| b.prog.is_done());
        *j = keep;
        done
    };
    for b in done {
        match b.prog.error() {
            Some(e) if e == "cancelled" => app.toast("Batch export cancelled"),
            Some(e) => app.toast(format!("Batch export: {e}")),
            None => app.toast_with_folder(format!("Exported {} files", b.count), b.dir),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_export_plans_one_file_each_with_unique_names() {
        let mut p = Project::default();
        let mk = |id: Id, path: &str| -> crate::model::Asset {
            serde_json::from_value(serde_json::json!({
                "id": id, "path": path, "kind": "Video", "duration": 5.0, "width": 640, "height": 360, "fps": 30.0
            }))
            .unwrap()
        };
        p.assets.push(mk(1, "C:/a/take.mov"));
        p.assets.push(mk(2, "C:/b/take.mp4"));
        let mut sub = mk(3, "C:/a/long.mov");
        sub.range = Some((2.0, 4.0));
        p.assets.push(sub);
        let dir = std::env::temp_dir().join(format!("se-batch-{}", std::process::id()));
        let list = plan(&p, &Settings::default(), &[1, 2, 3, 99], &dir, "mp4");
        let names: Vec<String> = list.iter().map(|o| o.out.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["take.mp4", "take (2).mp4", "long.mp4"], "a missing id is skipped, a clash is numbered");
        assert_eq!(list[2].range, Some((2.0, 4.0)), "a subclip exports only its range");
        assert!(list.iter().all(|o| o.out.parent() == Some(dir.as_path())));
    }
}
