//! Left panel: tabs "Library" (project assets: name, kind, duration, size; select, double-click or "Add"
//! → timeline at the playhead; right-click → Remove; drag source with `DragPayload::Asset`) and "Recent"
//! (settings.recent_assets across all projects: double-click / "Open" → `open_paths`; drag source with
//! `DragPayload::Path`; "Clear"). An "Import…" button at the top of Library.

use crate::model::{ClipKind, Id, Project};
use crate::settings::Settings;
use crate::theme::Palette;
use crate::ui::{duration_text, DragPayload};
use eframe::egui;
use std::path::PathBuf;

#[derive(Default)]
pub struct LibraryState {
    /// 0 = Library, 1 = Recent
    pub tab: usize,
    pub selected: Option<Id>,
}

#[derive(Default)]
pub struct LibraryResponse {
    pub import: bool,
    pub add_to_timeline: Vec<Id>,
    /// Recent files to import into the library (and select).
    pub open_paths: Vec<PathBuf>,
    pub remove: Vec<Id>,
    pub clear_recent: bool,
    /// The project changed (folders / tags / labels edited) — app pushes undo via `undo` first.
    pub edited: bool,
    /// settings.recent_assets changed (tags / labels / pins / removals) — app saves settings.
    pub settings_changed: bool,
}

pub fn show(
    ui: &mut egui::Ui,
    state: &mut LibraryState,
    project: &mut Project,
    settings: &mut Settings,
    _palette: &Palette,
    _undo: &mut dyn FnMut(&Project),
) -> LibraryResponse {
    let mut resp = LibraryResponse::default();
    ui.horizontal(|ui| {
        ui.selectable_value(&mut state.tab, 0, "Library");
        ui.selectable_value(&mut state.tab, 1, "Recent");
    });
    ui.separator();
    if state.tab == 0 {
        library_tab(ui, state, project, &mut resp);
    } else {
        recent_tab(ui, settings, &mut resp);
    }
    resp
}

fn kind_tag(k: ClipKind) -> &'static str {
    match k {
        ClipKind::Video => "V",
        ClipKind::Audio => "A",
        ClipKind::Image => "I",
        ClipKind::Text => "T",
        ClipKind::Sequence => "S",
    }
}

/// One list row: a drag source with a click-sensing overlay (select / double-click / context menu) and
/// an optional action button on the right. Returns (row response, button clicked).
fn row(
    ui: &mut egui::Ui,
    id: egui::Id,
    payload: DragPayload,
    selected: bool,
    button: Option<&str>,
    contents: impl FnOnce(&mut egui::Ui),
) -> (egui::Response, bool) {
    ui.horizontal(|ui| {
        let reserve = if button.is_some() { 40.0 } else { 0.0 };
        let src = ui.dnd_drag_source(id, payload, |ui| {
            let bg = ui.painter().add(egui::Shape::Noop);
            ui.horizontal(|ui| {
                contents(ui);
                ui.add_space((ui.available_width() - reserve).max(0.0));
            });
            let rect = ui.min_rect();
            let v = ui.visuals();
            let fill = if selected {
                v.selection.bg_fill.gamma_multiply(0.35)
            } else if ui.rect_contains_pointer(rect) {
                v.widgets.hovered.weak_bg_fill
            } else {
                egui::Color32::TRANSPARENT
            };
            ui.painter().set(bg, egui::Shape::rect_filled(rect, 2.0, fill));
        });
        // Registered after the drag-only widget so clicks reach it (egui gives the topmost click-sensing widget
        // the click and the drag-sensing one the drag).
        let r = ui.interact(src.response.rect, id.with("click"), egui::Sense::click());
        let clicked = button.is_some_and(|b| ui.small_button(b).clicked());
        (r, clicked)
    })
    .inner
}

fn library_tab(ui: &mut egui::Ui, state: &mut LibraryState, project: &Project, resp: &mut LibraryResponse) {
    if ui.button("Import…").clicked() {
        resp.import = true;
    }
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        for a in &project.assets {
            let selected = state.selected == Some(a.id);
            let (r, add) = row(
                ui,
                egui::Id::new(("asset", a.id)),
                DragPayload::Asset(a.id),
                selected,
                selected.then_some("Add"),
                |ui| {
                    let name = egui::RichText::new(a.name());
                    ui.label(if selected { name.strong() } else { name });
                    ui.weak(kind_tag(a.kind));
                    ui.weak(duration_text(a.duration));
                },
            );
            if r.clicked() {
                state.selected = Some(a.id);
            }
            if add || r.double_clicked() {
                state.selected = Some(a.id);
                resp.add_to_timeline.push(a.id);
            }
            r.context_menu(|ui| {
                if ui.button("Add to timeline at playhead").clicked() {
                    resp.add_to_timeline.push(a.id);
                    ui.close();
                }
                if ui.button("Remove from project").clicked() {
                    resp.remove.push(a.id);
                    ui.close();
                }
            });
        }
    });
}

fn recent_tab(ui: &mut egui::Ui, settings: &Settings, resp: &mut LibraryResponse) {
    if ui.button("Clear").clicked() {
        resp.clear_recent = true;
    }
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        // ponytail: no existence check here (would stat 200 files per frame) — the open path reports errors.
        for (i, rec) in settings.recent_assets.iter().enumerate() {
            let (name, dir) = split_path(&rec.path);
            let (r, open) =
                row(ui, egui::Id::new(("recent", i)), DragPayload::Path(rec.path.clone()), false, Some("Open"), |ui| {
                    ui.label(name);
                    ui.add(egui::Label::new(egui::RichText::new(dir).weak()).truncate());
                });
            if open || r.double_clicked() {
                resp.open_paths.push(PathBuf::from(&rec.path));
            }
            r.on_hover_text(&rec.path);
        }
    });
}

/// ("file.mp4", "C:\dir") — pure string split on the last path separator.
fn split_path(p: &str) -> (&str, &str) {
    match p.rfind(['\\', '/']) {
        Some(i) => (&p[i + 1..], &p[..i]),
        None => (p, ""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_path_cases() {
        assert_eq!(split_path(r"C:\Videos\clip.mp4"), ("clip.mp4", r"C:\Videos"));
        assert_eq!(split_path("/home/u/a.mov"), ("a.mov", "/home/u"));
        assert_eq!(split_path("a.mp4"), ("a.mp4", ""));
        assert_eq!(split_path(r"C:\x\"), ("", r"C:\x"));
    }

    #[test]
    fn kind_tags() {
        assert_eq!(kind_tag(ClipKind::Video), "V");
        assert_eq!(kind_tag(ClipKind::Audio), "A");
        assert_eq!(kind_tag(ClipKind::Image), "I");
    }

    /// Headless: both tabs lay out without panicking and report nothing when nothing was clicked.
    #[test]
    fn show_headless() {
        use crate::model::{Asset, Project};
        let mut project = Project::new();
        for (i, kind) in [ClipKind::Video, ClipKind::Audio, ClipKind::Image].iter().enumerate() {
            project.add_asset(Asset {
                id: 0,
                path: format!(r"C:\media\file{i}.mp4"),
                kind: *kind,
                duration: 12.5 * i as f64,
                width: 320,
                height: 240,
                fps: 30.0,
                audio_streams: Vec::new(),
                codec: String::new(),
                folder: String::new(),
                tags: Vec::new(),
                label: 0,
                description: String::new(),
            });
        }
        let mut settings = Settings::default();
        settings.touch_recent(r"C:\media\old.mp4");
        settings.touch_recent(r"D:\clips\new.mov");
        let palette = Palette::new(true, egui::Color32::WHITE);
        let ctx = egui::Context::default();
        for tab in [0, 1] {
            let mut state = LibraryState { tab, selected: Some(project.assets[1].id) };
            for _ in 0..2 {
                let _ = ctx.run(egui::RawInput::default(), |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let mut undo = |_: &Project| {};
                        let r = show(ui, &mut state, &mut project, &mut settings, &palette, &mut undo);
                        assert!(!r.import && !r.clear_recent);
                        assert!(r.add_to_timeline.is_empty() && r.open_paths.is_empty() && r.remove.is_empty());
                    });
                });
            }
            assert_eq!(state.tab, tab);
        }
    }
}
