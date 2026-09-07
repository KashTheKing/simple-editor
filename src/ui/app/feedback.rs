//! ---- ws:forgiveness ----
//! Toast notifications: bottom-right, non-blocking, auto-expiring egui::Area (unchanged from the
//! pre-wave-0a behaviour, just extracted out of `mod.rs` per the split-god-files plan). Extends the
//! original `msg`/`at`/`open_path` shape with a `kind` (colour), an optional `(label, Action)` button
//! (Undo etc.) and an optional progress bar - `Toast::new`/`with_folder` keep their old signatures, so
//! every existing `App::toast`/`toast_with_folder` call site compiles unchanged.

use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ToastKind {
    Info,
    Success,
    Warn,
    Error,
}

/// One toast notification. `open_path` offers an "Open Folder" button for a file (or folder) it just
/// finished writing; `action` offers a second button (label, Action) - used for Undo today.
pub(super) struct Toast {
    pub(super) msg: String,
    pub(super) kind: ToastKind,
    pub(super) at: Instant,
    pub(super) open_path: Option<PathBuf>,
    pub(super) action: Option<(String, Action)>,
    pub(super) progress: Option<Arc<Progress>>,
}

impl Toast {
    pub(super) fn new(msg: impl Into<String>) -> Self {
        Toast {
            msg: msg.into(),
            kind: ToastKind::Info,
            at: Instant::now(),
            open_path: None,
            action: None,
            progress: None,
        }
    }

    pub(super) fn with_folder(msg: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Toast {
            msg: msg.into(),
            kind: ToastKind::Info,
            at: Instant::now(),
            open_path: Some(path.into()),
            action: None,
            progress: None,
        }
    }

    pub(super) fn kind(mut self, kind: ToastKind) -> Self {
        self.kind = kind;
        self
    }

    pub(super) fn undo(mut self, action: Action) -> Self {
        self.action = Some(("Undo".into(), action));
        self
    }
}

impl App {
    /// 122 existing call sites, unchanged.
    pub(super) fn toast(&mut self, msg: impl Into<String>) {
        self.push_toast(Toast::new(msg));
    }

    /// Like `toast`, but offers an "Open Folder" button for a file (or folder) just written to disk.
    pub(super) fn toast_with_folder(&mut self, msg: impl Into<String>, path: impl Into<PathBuf>) {
        self.push_toast(Toast::with_folder(msg, path));
    }

    /// An Undo-actioned toast (Delete, Remove unused, Clear subtitles, …) - clicking its button appends
    /// `Action::Undo` (or whichever undo-style action the caller passes) to `pending_actions`, reusing
    /// the existing undo stack verbatim.
    /// ponytail: no per-op revert logic - every caller passes `Action::Undo`, which pops the same undo
    /// stack a Ctrl+Z would. A per-op-specific undo action isn't needed until something can't be undone
    /// that way.
    pub(crate) fn toast_undo(&mut self, msg: impl Into<String>, undo: Action) {
        self.push_toast(Toast::new(msg).undo(undo));
    }

    /// Push a toast, deduping by (msg, kind) via `dedupe_push` below.
    pub(super) fn push_toast(&mut self, t: Toast) {
        dedupe_push(&mut self.toasts, t);
    }
}

/// A repeat of the same (msg, kind) within its lifetime just refreshes `at` (and adopts the new one's
/// open_path/action/progress if it has one) instead of stacking a duplicate - useful for a
/// fast-repeating source like a progress toast or a hammered hotkey. Free function (not a method) so
/// the dedupe rule itself is testable without a live `App`.
fn dedupe_push(toasts: &mut Vec<Toast>, mut t: Toast) {
    if let Some(existing) = toasts.iter_mut().find(|e| e.msg == t.msg && e.kind == t.kind) {
        existing.at = Instant::now();
        if t.open_path.is_some() {
            existing.open_path = t.open_path.take();
        }
        if t.action.is_some() {
            existing.action = t.action.take();
        }
        if t.progress.is_some() {
            existing.progress = t.progress.take();
        }
        return;
    }
    toasts.push(t);
}

/// WINDOW_DRAWER: renders every live toast, oldest first, bottom-right - verbatim behaviour from the
/// pre-extraction inline block, plus the kind colour / Undo button / progress bar this workstream adds.
pub(super) fn draw(app: &mut App, ctx: &egui::Context) {
    app.toasts.retain(|t| {
        t.at.elapsed().as_secs_f32() < if t.open_path.is_some() || t.action.is_some() { 10.0 } else { 5.0 }
    });
    if app.toasts.is_empty() {
        return;
    }
    let palette = app.palette.clone();
    let mut clicked_undo: Option<Action> = None;
    egui::Area::new(egui::Id::new("toasts"))
        .anchor(egui::Align2::RIGHT_BOTTOM, [-12.0, -12.0])
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            for t in &app.toasts {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    let color = match t.kind {
                        ToastKind::Info => ui.visuals().text_color(),
                        ToastKind::Success => palette.selection,
                        ToastKind::Warn => ui.visuals().warn_fg_color,
                        ToastKind::Error => ui.visuals().error_fg_color,
                    };
                    ui.colored_label(color, &t.msg);
                    if let Some(p) = &t.progress {
                        ui.add(egui::ProgressBar::new(p.fraction()).show_percentage());
                    }
                    ui.horizontal(|ui| {
                        if let Some(p) = &t.open_path {
                            if ui.small_button("Open Folder").clicked() {
                                let mut cmd = std::process::Command::new("explorer");
                                if p.is_dir() {
                                    cmd.arg(p);
                                } else {
                                    cmd.arg("/select,").arg(p);
                                }
                                let _ = cmd.spawn();
                            }
                        }
                        if let Some((label, action)) = &t.action {
                            if ui.small_button(label).clicked() {
                                clicked_undo = Some(*action);
                            }
                        }
                    });
                });
            }
        });
    if let Some(a) = clicked_undo {
        app.pending_actions.push(a);
    }
    ctx.request_repaint_after(std::time::Duration::from_millis(500));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toast_dedupes_by_message() {
        let mut toasts: Vec<Toast> = Vec::new();
        dedupe_push(&mut toasts, Toast::new("Saved"));
        let first_at = toasts[0].at;
        std::thread::sleep(std::time::Duration::from_millis(5));
        dedupe_push(&mut toasts, Toast::new("Saved"));
        assert_eq!(toasts.len(), 1, "same message must refresh, not append");
        assert!(toasts[0].at > first_at, "the refresh must bump `at`");
        // a different kind is a different toast, even with the same text
        dedupe_push(&mut toasts, Toast::new("Saved").kind(ToastKind::Error));
        assert_eq!(toasts.len(), 2);
    }

    #[test]
    fn toast_undo_button_carries_the_action() {
        let t = Toast::new("Deleted 3 clips").undo(Action::Undo);
        assert_eq!(t.action, Some(("Undo".to_string(), Action::Undo)));
    }
}
