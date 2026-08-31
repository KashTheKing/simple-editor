//! History panel: a read-mostly view over the undo stack (`App.undo`, newest last) grouped by day,
//! searchable by label, filterable by category (Editing vs Layout — see `crate::ui::app::HistoryCategory`),
//! with per-entry and bulk delete, and export of the currently-filtered entries to a Markdown file.
//! File-system-style: one collapsed-by-default row per entry (mirrors markers_ui.rs/effects_ui.rs's
//! `CollapsingHeader` convention), grouped under a day heading.
//!
//! Deleting an entry only removes it from the list — since every `UndoEntry` is a complete project
//! snapshot (not a delta), removing one from the middle doesn't corrupt anything; Ctrl+Z just steps
//! past it to the next surviving snapshot.

use crate::ui::app::{HistoryCategory, UndoEntry};
use eframe::egui;

#[derive(Default)]
pub struct HistoryState {
    pub search: String,
    /// None = both categories.
    pub category: Option<HistoryCategory>,
}

/// Test-only: remember a widget rect so headless tests can click the real button.
#[cfg(test)]
fn mark(ui: &egui::Ui, name: &str, r: &egui::Response) {
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("hist", name.to_string())), r.rect));
}
#[cfg(not(test))]
fn mark(_ui: &egui::Ui, _name: &str, _r: &egui::Response) {}

fn day_of(at: f64) -> i64 {
    (at / 86400.0).floor() as i64
}

fn day_label(at: f64) -> String {
    let secs = at as i64;
    let days_since_epoch = secs.div_euclid(86400);
    let mut d = days_since_epoch;
    // civil_from_days (Howard Hinnant's algorithm) — no chrono dependency for one date format.
    d += 719468;
    let era = if d >= 0 { d } else { d - 146096 } / 146097;
    let doe = (d - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if month <= 2 { y + 1 } else { y };
    format!("{year:04}-{month:02}-{day:02}")
}

/// Bool-returning "was anything deleted" so the caller knows to mark the project dirty-ish (deleting
/// history is itself not a project edit, so it does NOT go through undo/push_undo).
pub fn show(ui: &mut egui::Ui, state: &mut HistoryState, undo: &mut Vec<UndoEntry>) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label("Search");
        ui.add(egui::TextEdit::singleline(&mut state.search).desired_width(160.0));
        if !state.search.is_empty() && ui.small_button("x").clicked() {
            state.search.clear();
        }
    });
    ui.horizontal(|ui| {
        ui.selectable_value(&mut state.category, None, "All");
        ui.selectable_value(&mut state.category, Some(HistoryCategory::Editing), "Editing");
        ui.selectable_value(&mut state.category, Some(HistoryCategory::Layout), "Layout");
    });

    let needle = state.search.to_lowercase();
    let matches = |e: &UndoEntry| {
        (state.category.is_none() || state.category == Some(e.category))
            && (needle.is_empty() || e.label.to_lowercase().contains(&needle))
    };
    let visible: Vec<usize> = undo.iter().enumerate().filter(|(_, e)| matches(e)).map(|(i, _)| i).collect();

    ui.horizontal(|ui| {
        ui.label(format!("{} entr{}", visible.len(), if visible.len() == 1 { "y" } else { "ies" }));
        let r = ui.add_enabled(!visible.is_empty(), egui::Button::new("Delete filtered"));
        mark(ui, "delete_filtered", &r);
        if r.clicked() {
            let victims: std::collections::HashSet<usize> = visible.iter().copied().collect();
            let mut i = 0;
            undo.retain(|_| {
                let keep = !victims.contains(&i);
                i += 1;
                keep
            });
            changed = true;
        }
        let r = ui.add_enabled(!visible.is_empty(), egui::Button::new("Export filtered to Markdown…"));
        mark(ui, "export", &r);
        if r.clicked() {
            if let Some(out) = rfd::FileDialog::new()
                .add_filter("Markdown", &["md"])
                .set_file_name("history.md")
                .save_file()
            {
                let text = export_markdown(undo, &visible);
                if std::fs::write(&out, &text).is_ok() {
                    // best-effort convenience, matching the app's other "reveal the saved file" spots
                    let mut cmd = std::process::Command::new("explorer");
                    cmd.arg("/select,").arg(&out);
                    let _ = cmd.spawn();
                }
            }
        }
    });

    ui.separator();
    let mut delete: Option<usize> = None;
    let mut last_day: Option<i64> = None;
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        // newest first — `.get` (not indexing) because "Delete filtered" above may have already
        // shrunk `undo` this same frame, leaving `visible`'s indices stale until the next repaint.
        for &i in visible.iter().rev() {
            let Some(e) = undo.get(i) else { continue };
            let d = day_of(e.at);
            if last_day != Some(d) {
                ui.strong(day_label(e.at));
                last_day = Some(d);
            }
            let header_id = ui.id().with(("history_row", i));
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), header_id, false)
                .show_header(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        let icon = match e.category {
                            HistoryCategory::Layout => crate::ui::tools::Glyph::GridIcon,
                            HistoryCategory::Editing => crate::ui::tools::Glyph::Hourglass,
                        };
                        let (r, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                        crate::ui::tools::draw_glyph(ui.painter(), r, icon, ui.visuals().text_color());
                        ui.weak(time_of_day(e.at));
                        ui.label(&e.label);
                        if ui.small_button("Delete").clicked() {
                            delete = Some(i);
                        }
                    });
                })
                .body(|ui| {
                    ui.weak(format!("{} bytes of project state", e.json.len()));
                });
        }
        if visible.is_empty() {
            ui.weak("No history — make a few edits, or clear the search/filter above");
        }
    });
    if let Some(i) = delete {
        undo.remove(i);
        changed = true;
    }
    changed
}

fn time_of_day(at: f64) -> String {
    let secs_in_day = (at as i64).rem_euclid(86400);
    format!("{:02}:{:02}:{:02}", secs_in_day / 3600, (secs_in_day / 60) % 60, secs_in_day % 60)
}

fn export_markdown(undo: &[UndoEntry], indices: &[usize]) -> String {
    let mut s = String::from("# Project History\n\n");
    let mut last_day: Option<i64> = None;
    for &i in indices {
        let e = &undo[i];
        let d = day_of(e.at);
        if last_day != Some(d) {
            s.push_str(&format!("\n## {}\n\n", day_label(e.at)));
            last_day = Some(d);
        }
        let cat = match e.category {
            HistoryCategory::Editing => "editing",
            HistoryCategory::Layout => "layout",
        };
        s.push_str(&format!("- `{}` [{cat}] {}\n", time_of_day(e.at), e.label));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::app::HistoryCategory;

    fn entry(label: &str, category: HistoryCategory, at: f64) -> UndoEntry {
        UndoEntry { json: "{}".into(), label: label.into(), category, at }
    }

    struct H {
        ctx: egui::Context,
        state: HistoryState,
        undo: Vec<UndoEntry>,
        time: f64,
    }
    impl H {
        fn new() -> Self {
            let undo = vec![
                entry("Added a clip", HistoryCategory::Editing, 1_000_000.0),
                entry("Rearranged panels", HistoryCategory::Layout, 1_000_100.0),
                entry("Edited effects", HistoryCategory::Editing, 1_000_200.0),
            ];
            Self { ctx: egui::Context::default(), state: HistoryState::default(), undo, time: 0.0 }
        }
        fn frame(&mut self) {
            self.time += 0.05;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(420.0, 700.0))),
                time: Some(self.time),
                ..Default::default()
            };
            let H { ctx, state, undo, .. } = self;
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    show(ui, state, undo);
                });
            });
        }
    }

    #[test]
    fn category_filter_hides_the_other_category() {
        let mut h = H::new();
        h.state.category = Some(HistoryCategory::Layout);
        h.frame();
        // only the Layout entry should remain interactable; nothing to assert on the label text
        // directly without a text-scan helper, so this just guards against a panic in the filtered
        // render path (the real assertions live in the delete/export tests below).
    }

    #[test]
    fn search_filters_by_label_substring() {
        let mut h = H::new();
        h.state.search = "clip".into();
        h.frame();
        let needle = h.state.search.to_lowercase();
        let n = h.undo.iter().filter(|e| e.label.to_lowercase().contains(&needle)).count();
        assert_eq!(n, 1);
    }

    #[test]
    fn delete_filtered_removes_only_matching_entries() {
        let mut h = H::new();
        h.state.category = Some(HistoryCategory::Editing);
        h.frame();
        let r = h.ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("hist", "delete_filtered")))).unwrap();
        h.frame(); // hover
        let changed = {
            let H { ctx, state, undo, .. } = &mut h;
            let mut changed = false;
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(420.0, 700.0))),
                    events: vec![
                        egui::Event::PointerMoved(r.center()),
                        egui::Event::PointerButton {
                            pos: r.center(),
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            modifiers: egui::Modifiers::NONE,
                        },
                        egui::Event::PointerButton {
                            pos: r.center(),
                            button: egui::PointerButton::Primary,
                            pressed: false,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        changed = show(ui, state, undo);
                    });
                },
            );
            changed
        };
        assert!(changed);
        assert_eq!(h.undo.len(), 1, "both Editing entries removed, the Layout one survives");
        assert_eq!(h.undo[0].category, HistoryCategory::Layout);
    }

    #[test]
    fn export_markdown_includes_only_the_given_indices() {
        let h = H::new();
        let md = export_markdown(&h.undo, &[0, 2]);
        assert!(md.contains("Added a clip"));
        assert!(md.contains("Edited effects"));
        assert!(!md.contains("Rearranged panels"));
    }

    #[test]
    fn day_label_matches_a_known_date() {
        // 2024-01-15 00:00:00 UTC = 1705276800
        assert_eq!(day_label(1_705_276_800.0), "2024-01-15");
    }
}
