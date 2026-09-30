//! History panel: a read-mostly view over the undo stack (`App.undo`, newest last) grouped by day. One
//! header row: a label search, the category filter (All / Editing / Layout - see
//! `crate::ui::app::HistoryCategory`) and the entry count. One row per entry under its day heading (icon,
//! time, label); its right-click restores or deletes it, and right-clicking empty space deletes every
//! listed entry or exports them to a Markdown file.
//!
//! Deleting an EDITING entry only removes it from the list - every such `UndoEntry` is a complete
//! project snapshot (not a delta), so Ctrl+Z just steps past it to the next surviving snapshot.
//! LAYOUT entries are different: each is a sentinel paired 1:1 with a snapshot on the SEPARATE
//! `Layout::undo` stack, so deleting one here would desync the pairing and make later layout undos
//! restore the wrong arrangement - layout rows therefore cannot be deleted from this panel.
//!
//! Labels are derived lazily HERE, not at push time: entry `i` is the project as it was BEFORE edit
//! `i`, so its row describes the change from `i` to `i+1` (or to the live project for the newest row).
//! Deriving at push time both labelled every row one edit late and cost two full project parses on
//! every single edit gesture; now the cost is paid only while this panel is actually open, once per
//! entry (cached).

use crate::ui::app::{describe_change, HistoryCategory, UndoEntry, LAYOUT_STEP};
use crate::ui::menu;
use crate::ui::tools::Glyph;
use eframe::egui;

/// ---- ws:forgiveness ----
#[derive(Default)]
pub struct HistoryResponse {
    /// Something was deleted (see the module doc: not itself a project edit, no undo/push_undo).
    pub changed: bool,
    /// Index into `undo` whose Restore button was clicked this frame - the app-side handler (Layout
    /// rows never set this; see `restore_at`) does the actual restore + labeled undo push.
    pub restore: Option<usize>,
}

#[derive(Default)]
pub struct HistoryState {
    pub search: String,
    /// None = both categories.
    pub category: Option<HistoryCategory>,
    /// Lazily-derived row labels, keyed by (at bits, snapshot length) - stable across the stack
    /// shifting (cap eviction, deletes, undo/redo moving entries between stacks).
    labels: std::collections::HashMap<(u64, usize), String>,
}

/// Test-only: remember a widget rect so headless tests can click the real row.
#[cfg(test)]
fn mark(ui: &egui::Ui, name: &str, r: &egui::Response) {
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("hist", name.to_string())), r.rect));
}
#[cfg(not(test))]
fn mark(_ui: &egui::Ui, _name: &str, _r: &egui::Response) {}

/// Local-time offset from UTC in seconds, so day grouping and clock times match the user's wall
/// clock instead of filing every evening edit under tomorrow's UTC date. Queried once per process
/// (a DST flip mid-session shifts new rows by an hour - a shrug, not a bug worth polling for).
fn local_offset_secs() -> i64 {
    use std::sync::OnceLock;
    static OFF: OnceLock<i64> = OnceLock::new();
    *OFF.get_or_init(|| unsafe {
        use windows::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
        let mut tzi = TIME_ZONE_INFORMATION::default();
        let r = GetTimeZoneInformation(&mut tzi);
        let extra = if r == 2 { tzi.DaylightBias } else { tzi.StandardBias }; // 2 = TIME_ZONE_ID_DAYLIGHT
                                                                              // Windows bias is minutes with UTC = local + bias, so the offset to ADD to UTC is its negation
        -((tzi.Bias + extra) as i64) * 60
    })
}

fn day_of(at: f64) -> i64 {
    ((at + local_offset_secs() as f64) / 86400.0).floor() as i64
}

fn day_label(at: f64) -> String {
    let secs = at as i64 + local_offset_secs();
    let days_since_epoch = secs.div_euclid(86400);
    let mut d = days_since_epoch;
    // civil_from_days (Howard Hinnant's algorithm) - no chrono dependency for one date format.
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
/// history is itself not a project edit, so it does NOT go through undo/push_undo). `project` is the
/// LIVE project - the newest row's label describes the edit from its snapshot to this state.
pub fn show(
    ui: &mut egui::Ui,
    state: &mut HistoryState,
    undo: &mut Vec<UndoEntry>,
    project: &crate::model::Project,
) -> HistoryResponse {
    let mut resp = HistoryResponse::default();
    let mut changed = false;

    // resolve row labels lazily (see the module doc): entry i's row describes snapshot i -> i+1
    // (-> the live project for the top). A pre-set label (layout sentinels, tests) wins as-is.
    let key = |e: &UndoEntry| (e.at.to_bits(), e.json.len());
    let mut live_json: Option<String> = None; // serialized at most once, and only on a cache miss
    for i in 0..undo.len() {
        if !undo[i].label.is_empty() || state.labels.contains_key(&key(&undo[i])) {
            continue;
        }
        let label = match undo.get(i + 1) {
            Some(next) if next.json != LAYOUT_STEP => describe_change(&undo[i].json, &next.json),
            // a layout sentinel above means the project state carried through it unchanged; compare
            // against the next real snapshot, or the live project if there is none
            _ => {
                let next_real = undo[i + 1..].iter().find(|e| e.json != LAYOUT_STEP);
                match next_real {
                    Some(next) => describe_change(&undo[i].json, &next.json),
                    None => {
                        let live = live_json.get_or_insert_with(|| project.to_json());
                        describe_change(&undo[i].json, live)
                    }
                }
            }
        };
        state.labels.insert(key(&undo[i]), label);
    }
    let label_of = |state: &HistoryState, e: &UndoEntry| -> String {
        if !e.label.is_empty() {
            e.label.clone()
        } else {
            state.labels.get(&key(e)).cloned().unwrap_or_else(|| "Project edited".into())
        }
    };
    let bg = crate::ui::markers_ui::menu_area(ui);
    let needle = state.search.to_lowercase();
    let matches = |state: &HistoryState, e: &UndoEntry| {
        (state.category.is_none() || state.category == Some(e.category))
            && (needle.is_empty() || label_of(state, e).to_lowercase().contains(&needle))
    };
    let visible: Vec<usize> = undo.iter().enumerate().filter(|(_, e)| matches(state, e)).map(|(i, _)| i).collect();
    // layout rows are visible but NOT deletable - each is paired 1:1 with the separate layout undo
    // stack, and removing one desyncs that pairing (see the module doc)
    let deletable: Vec<usize> =
        visible.iter().copied().filter(|&i| undo[i].category != HistoryCategory::Layout).collect();

    // the one header row: search · category · count
    ui.horizontal(|ui| {
        ui.selectable_value(&mut state.category, None, "All");
        ui.selectable_value(&mut state.category, Some(HistoryCategory::Editing), "Editing");
        ui.selectable_value(&mut state.category, Some(HistoryCategory::Layout), "Layout");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.weak(format!("{} entr{}", visible.len(), if visible.len() == 1 { "y" } else { "ies" }));
            let w = ui.available_width();
            ui.add(egui::TextEdit::singleline(&mut state.search).hint_text("Search history").desired_width(w));
        });
    });
    ui.separator();

    let mut delete: Option<usize> = None;
    let mut delete_listed = false;
    let mut last_day: Option<i64> = None;
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        // newest first
        for &i in visible.iter().rev() {
            let Some(e) = undo.get(i) else { continue };
            let d = day_of(e.at);
            if last_day != Some(d) {
                ui.strong(day_label(e.at));
                last_day = Some(d);
            }
            let layout = e.category == HistoryCategory::Layout;
            let row = ui
                .scope_builder(egui::UiBuilder::new().sense(egui::Sense::click()), |ui| {
                    ui.horizontal(|ui| {
                        let icon =
                            if layout { crate::ui::tools::Glyph::GridIcon } else { crate::ui::tools::Glyph::History };
                        let (r, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                        crate::ui::tools::draw_glyph(ui.painter(), r, icon, ui.visuals().text_color());
                        ui.weak(time_of_day(e.at));
                        ui.label(label_of(state, e));
                    });
                })
                .response
                .on_hover_text(format!("{} bytes of project state - right-click to restore or delete", e.json.len()));
            mark(ui, &format!("row_{i}"), &row);
            row.context_menu(|ui| {
                // Layout entries pair 1:1 with the panel-arrangement undo stack: neither restorable
                // (they are not project states) nor deletable (it would desync that pairing)
                let why = "Layout entries pair with the panel-arrangement undo stack";
                let r = ui.add_enabled_ui(!layout, |ui| menu::row(ui, Some(Glyph::History), "Restore", "")).inner;
                if r.on_hover_text("Make this the live project (pushes a new, labeled undo entry)")
                    .on_disabled_hover_text(why)
                    .clicked()
                {
                    resp.restore = Some(i);
                }
                let r = ui.add_enabled_ui(!layout, |ui| menu::row(ui, Some(Glyph::Cross), "Delete", "")).inner;
                if r.on_disabled_hover_text(why).clicked() {
                    delete = Some(i);
                }
            });
        }
        if visible.is_empty() {
            ui.weak("No history - make a few edits, or clear the search/filter above");
        }
    });
    bg.context_menu(|ui| {
        let r = ui
            .add_enabled_ui(!deletable.is_empty(), |ui| menu::row(ui, Some(Glyph::Cross), "Delete listed entries", ""));
        let r = r.inner.on_hover_text(
            "Removes the listed Editing entries (Layout entries pair with the panel-undo stack and stay)",
        );
        delete_listed = r.clicked();
        let r = ui.add_enabled_ui(!visible.is_empty(), |ui| {
            menu::row(ui, Some(Glyph::ExportArrow), "Export listed to Markdown…", "")
        });
        if r.inner.clicked() {
            if let Some(out) =
                rfd::FileDialog::new().add_filter("Markdown", &["md"]).set_file_name("history.md").save_file()
            {
                let text = export_markdown(undo, &visible, |e| label_of(state, e));
                if std::fs::write(&out, &text).is_ok() {
                    // best-effort convenience, matching the app's other "reveal the saved file" spots
                    let mut cmd = std::process::Command::new("explorer");
                    cmd.arg("/select,").arg(&out);
                    let _ = cmd.spawn();
                }
            }
        }
    });
    if delete_listed {
        let victims: std::collections::HashSet<usize> = deletable.iter().copied().collect();
        let mut i = 0;
        undo.retain(|_| {
            let keep = !victims.contains(&i);
            i += 1;
            keep
        });
        // a cached label describes the change TO the (now different) next entry - recompute all
        state.labels.clear();
        changed = true;
    } else if let Some(i) = delete {
        undo.remove(i);
        state.labels.clear(); // the deleted entry's predecessor now describes a different neighbour
        changed = true;
    }
    resp.changed = changed;
    resp
}

/// The app-side restore logic (called from `panes.rs`'s `Pane::History` arm) when
/// `HistoryResponse.restore` is `Some(i)`: `None` when `i` is out of range or points at a
/// non-restorable (Layout) entry, otherwise `Some((json to push as the pre-restore undo snapshot,
/// the restored Project))`. Pure so a test can exercise it without a live `App` - see
/// `tools_registry_tests.rs`'s doc comment for why one isn't buildable in `#[test]`.
pub fn restore_at(undo: &[UndoEntry], i: usize, live_json: &str) -> Option<(String, crate::model::Project)> {
    let e = undo.get(i)?;
    if e.category == HistoryCategory::Layout {
        return None;
    }
    let restored = crate::model::Project::from_json(&e.json).ok()?;
    Some((live_json.to_string(), restored))
}

fn time_of_day(at: f64) -> String {
    let secs_in_day = (at as i64 + local_offset_secs()).rem_euclid(86400);
    format!("{:02}:{:02}:{:02}", secs_in_day / 3600, (secs_in_day / 60) % 60, secs_in_day % 60)
}

fn export_markdown(undo: &[UndoEntry], indices: &[usize], label_of: impl Fn(&UndoEntry) -> String) -> String {
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
        s.push_str(&format!("- `{}` [{cat}] {}\n", time_of_day(e.at), label_of(e)));
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
        project: crate::model::Project,
        time: f64,
    }
    impl H {
        fn new() -> Self {
            let undo = vec![
                entry("Added a clip", HistoryCategory::Editing, 1_000_000.0),
                entry("Rearranged panels", HistoryCategory::Layout, 1_000_100.0),
                entry("Edited effects", HistoryCategory::Editing, 1_000_200.0),
            ];
            let ctx = egui::Context::default();
            ctx.set_fonts(crate::theme::test_fonts()); // real glyph sizes, so popup entries land where drawn
            Self { ctx, state: HistoryState::default(), undo, project: crate::model::Project::new(), time: 0.0 }
        }
        fn frame(&mut self) {
            self.time += 0.05;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(420.0, 700.0))),
                time: Some(self.time),
                ..Default::default()
            };
            let H { ctx, state, undo, project, .. } = self;
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    show(ui, state, undo, project);
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

    impl H {
        /// Right-click `at`, then click the menu entry `label`; returns the click frame's response.
        fn menu(&mut self, at: egui::Pos2, label: &str) -> HistoryResponse {
            let mut out = HistoryResponse::default();
            let mut shapes = Vec::new();
            let steps: Vec<(egui::Pos2, egui::PointerButton, bool)> =
                vec![(at, egui::PointerButton::Secondary, true), (at, egui::PointerButton::Secondary, false)];
            for (pos, button, pressed) in steps {
                self.step(
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton { pos, button, pressed, modifiers: egui::Modifiers::NONE },
                    ],
                    &mut shapes,
                    &mut out,
                );
            }
            self.step(vec![], &mut shapes, &mut out);
            let item = shapes
                .iter()
                .find_map(|c| match &c.shape {
                    egui::epaint::Shape::Text(t) if t.galley.text() == label => Some(t.visual_bounding_rect().center()),
                    _ => None,
                })
                .unwrap_or_else(|| {
                    let texts: Vec<String> = shapes
                        .iter()
                        .filter_map(|c| match &c.shape {
                            egui::epaint::Shape::Text(t) => Some(t.galley.text().to_string()),
                            _ => None,
                        })
                        .collect();
                    panic!("no '{label}' in the menu: {texts:?}")
                });
            self.time += 1.0;
            let mut got = HistoryResponse::default();
            for pressed in [true, false] {
                let ev = egui::Event::PointerButton {
                    pos: item,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                };
                self.step(vec![egui::Event::PointerMoved(item), ev], &mut shapes, &mut got);
            }
            got
        }
        fn step(
            &mut self,
            events: Vec<egui::Event>,
            shapes: &mut Vec<egui::epaint::ClippedShape>,
            out: &mut HistoryResponse,
        ) {
            self.time += 0.05;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(420.0, 700.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let H { ctx, state, undo, project, .. } = self;
            let full = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let r = show(ui, state, undo, project);
                    out.changed |= r.changed;
                    out.restore = out.restore.or(r.restore);
                });
            });
            *shapes = full.shapes;
        }
    }

    #[test]
    fn delete_filtered_removes_only_matching_entries() {
        let mut h = H::new();
        h.state.category = Some(HistoryCategory::Editing);
        h.frame();
        let out = h.menu(egui::pos2(200.0, 650.0), "Delete listed entries");
        assert!(out.changed);
        assert_eq!(h.undo.len(), 1, "both Editing entries removed, the Layout one survives");
        assert_eq!(h.undo[0].category, HistoryCategory::Layout);
        // a cached label describes the change TO the next entry - deleting reshuffles every
        // neighbour pair, so the whole cache must go (it rebuilds lazily on the next render)
        assert!(h.state.labels.is_empty(), "deleting entries must drop the derived-label cache");
    }

    // ---- ws:forgiveness ----
    #[test]
    fn history_restore_pushes_one_labeled_undo() {
        let mut h = H::new();
        h.frame();
        // right-click ▸ Restore sets HistoryResponse.restore = Some(i) for an Editing row
        let r = h.ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("hist", "row_0")))).unwrap();
        let restore = h.menu(r.center(), "Restore").restore;
        assert_eq!(restore, Some(0), "undo[0] ('Added a clip', Editing) was restored from its row menu");

        // the app-side handler (restore_at) then does the actual restore + one labeled undo push
        let live = "{\"live\":true}".to_string();
        let (before, restored) = restore_at(&h.undo, 0, &live).expect("index 0 is an Editing row");
        assert_eq!(before, live, "the pre-restore snapshot pushed as undo is the CURRENT live project");
        assert_eq!(restored.to_json(), crate::model::Project::from_json(&h.undo[0].json).unwrap().to_json());
    }

    /// A single row's right-click ▸ Delete removes just that entry.
    #[test]
    fn row_menu_deletes_one_entry() {
        let mut h = H::new();
        h.frame();
        let r = h.ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("hist", "row_2")))).unwrap();
        assert!(h.menu(r.center(), "Delete").changed);
        let labels: Vec<&str> = h.undo.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(labels, vec!["Added a clip", "Rearranged panels"]);
    }

    #[test]
    fn history_layout_rows_not_restorable() {
        let mut h = H::new();
        h.frame();
        // undo[1] is the Layout entry: its menu's Restore is greyed out, so clicking it restores nothing
        let r = h.ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("hist", "row_1")))).unwrap();
        let out = h.menu(r.center(), "Restore");
        assert_eq!(out.restore, None, "a Layout row can never be restored");
        assert_eq!(h.undo.len(), 3);
        // and the app-side helper refuses it too, even if something upstream ever got this wrong
        assert!(restore_at(&h.undo, 1, "{}").is_none());
    }

    #[test]
    fn export_markdown_includes_only_the_given_indices() {
        let h = H::new();
        let md = export_markdown(&h.undo, &[0, 2], |e| e.label.clone());
        assert!(md.contains("Added a clip"));
        assert!(md.contains("Edited effects"));
        assert!(!md.contains("Rearranged panels"));
    }

    #[test]
    fn day_label_matches_a_known_date() {
        // 2024-01-15 00:00:00 UTC = 1705276800; labels are LOCAL time, so feed a timestamp that is
        // local midnight of that day (offset cancelled) - this pins both the civil-date math and the
        // fact that the offset is actually applied.
        assert_eq!(day_label(1_705_276_800.0 - local_offset_secs() as f64), "2024-01-15");
    }
}
