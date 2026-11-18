//! ---- ws:panels-multiscreen ----
//! The Rendering pane: the running export (title, file, progress, status, elapsed / ETA, how many more
//! are queued, Cancel) and the Export Frame form (`frame_ui`). Both were floating windows. An export
//! that starts brings the pane up once (`reveal_on_start`), like the window used to appear.

use super::*;

/// What one running export shows.
pub(super) struct Running {
    pub title: &'static str,
    pub file: String,
    pub fraction: f32,
    pub status: String,
    pub time: String,
}

#[derive(Default, PartialEq, Debug)]
pub(super) struct Asked {
    pub cancel: bool,
    pub export_video: bool,
}

/// The export part of the pane, App-free so it can be drawn headless.
pub(super) fn body(ui: &mut egui::Ui, running: Option<&Running>, queued: usize) -> Asked {
    let mut asked = Asked::default();
    let Some(r) = running else {
        ui.horizontal(|ui| {
            ui.weak("No export running");
            if ui.button("Export Video…").clicked() {
                asked.export_video = true;
            }
        });
        return asked;
    };
    ui.strong(r.title);
    if !r.file.is_empty() {
        ui.add(egui::Label::new(egui::RichText::new(&r.file).weak()).truncate());
    }
    ui.add(egui::ProgressBar::new(r.fraction).show_percentage());
    ui.label(&r.status);
    ui.weak(&r.time);
    if queued > 0 {
        ui.weak(format!("{queued} more queued"));
    }
    asked.cancel = ui.button(if queued > 0 { "Cancel all" } else { "Cancel" }).clicked();
    asked
}

pub(super) fn draw(app: &mut App, ui: &mut egui::Ui, pane: Pane) -> bool {
    if pane != Pane::Rendering {
        return false;
    }
    let running = app.export.as_ref().map(|(prog, kind)| {
        let (title, file) = match kind {
            ExportKind::File { path } => ("Exporting", path.display().to_string()),
            ExportKind::Overwrite { original, .. } => ("Saving over the original", original.display().to_string()),
        };
        let elapsed = crate::ui::duration_text(prog.elapsed().as_secs_f64());
        let time = match prog.eta() {
            Some(eta) => format!("{elapsed} elapsed · ETA {}", crate::ui::duration_text(eta.as_secs_f64())),
            None => format!("{elapsed} elapsed"),
        };
        Running { title, file, fraction: prog.fraction(), status: prog.status(), time }
    });
    let asked = body(ui, running.as_ref(), app.export_queue.len());
    if asked.cancel {
        if let Some((prog, _)) = &app.export {
            prog.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        app.export_queue.clear();
    }
    if asked.export_video {
        app.pending_actions.push(Action::ExportVideo);
    }
    if app.frame_ui.open {
        ui.separator();
        let choice = {
            let App { frame_ui: st, project, settings, .. } = app;
            frame_ui::show(ui, st, project, settings)
        };
        if let Some(c) = choice {
            app.settings.save();
            app.export_frame(c);
        }
    }
    true
}

/// Per frame (from `windows()`): the first frame of an export (or Export Frame) brings the pane up.
pub(super) fn reveal_on_start(app: &mut App) {
    let busy = app.export.is_some() || app.frame_ui.open;
    if busy && !app.render_revealed {
        app.layout.reveal(Pane::Rendering);
        app.layout_dirty = true;
    }
    app.render_revealed = busy;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Idle and mid-export, the pane lays out, asks for nothing and requests no repaint (the app's
    /// export tick drives the progress, not the pane).
    #[test]
    fn assert_no_idle_repaint_rendering_pane() {
        let r = Running {
            title: "Exporting",
            file: "C:/out/video.mp4".into(),
            fraction: 0.4,
            status: "Encoding 40%".into(),
            time: "0:10 elapsed".into(),
        };
        for running in [None, Some(&r)] {
            for queued in [0, 2] {
                let ctx = egui::Context::default();
                for _ in 0..3 {
                    let _ = ctx.run(egui::RawInput::default(), |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            assert_eq!(body(ui, running, queued), Asked::default());
                        });
                    });
                }
                assert!(!ctx.has_requested_repaint(), "Rendering pane repainted while idle");
            }
        }
    }
}
