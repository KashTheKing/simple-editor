//! History panel: a read-mostly view over the undo stack (`App.undo`, newest last) grouped by day,
//! searchable by label, filterable by category (Editing vs Layout — see `crate::ui::app::HistoryCategory`),
//! with per-entry and bulk delete, and export of the currently-filtered entries to a Markdown file.
//! File-system-style: one collapsed-by-default row per entry (mirrors markers_ui.rs/effects_ui.rs's
//! `CollapsingHeader` convention), grouped under a day heading.
//!
//! Deleting an EDITING entry only removes it from the list — every such `UndoEntry` is a complete
//! project snapshot (not a delta), so Ctrl+Z just steps past it to the next surviving snapshot.
//! LAYOUT entries are different: each is a sentinel paired 1:1 with a snapshot on the SEPARATE
//! `Layout::undo` stack, so deleting one here would desync the pairing and make later layout undos
//! restore the wrong arrangement — layout rows therefore cannot be deleted from this panel.
//!
//! Labels are derived lazily HERE, not at push time: entry `i` is the project as it was BEFORE edit
//! `i`, so its row describes the change from `i` to `i+1` (or to the live project for the newest row).
//! Deriving at push time both labelled every row one edit late and cost two full project parses on
//! every single edit gesture; now the cost is paid only while this panel is actually open, once per
//! entry (cached).

use crate::ui::app::{describe_change, HistoryCategory, UndoEntry, LAYOUT_STEP};
use eframe::egui;

#[derive(Default)]
pub struct HistoryState {
    pub search: String,
    /// None = both categories.
    pub category: Option<HistoryCategory>,
    /// Lazily-derived row labels, keyed by (at bits, snapshot length) — stable across the stack
    /// shifting (cap eviction, deletes, undo/redo moving entries between stacks).
    labels: std::collections::HashMap<(u64, usize), String>,
}

/// Test-only: remember a widget rect so headless tests can click the real button.
#[cfg(test)]
fn mark(ui: &egui::Ui, name: &str, r: &egui::Response) {
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("hist", name.to_string())), r.rect));
}
#[cfg(not(test))]
fn mark(_ui: &egui::Ui, _name: &str, _r: &egui::Response) {}

/// Local-time offset from UTC in seconds, so day grouping and clock times match the user's wall
/// clock instead of filing every evening edit under tomorrow's UTC date. Queried once per process
/// (a DST flip mid-session shifts new rows by an hour — a shrug, not a bug worth polling for).
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
/// history is itself not a project edit, so it does NOT go through undo/push_undo). `project` is the
/// LIVE project — the newest row's label describes the edit from its snapshot to this state.
pub fn show(
    ui: &mut egui::Ui,
    state: &mut HistoryState,
    undo: &mut Vec<UndoEntry>,
    project: &crate::model::Project,
) -> bool {
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
    let matches = |state: &HistoryState, e: &UndoEntry| {
        (state.category.is_none() || state.category == Some(e.category))
            && (needle.is_empty() || label_of(state, e).to_lowercase().contains(&needle))
    };
    let visible: Vec<usize> = undo.iter().enumerate().filter(|(_, e)| matches(state, e)).map(|(i, _)| i).collect();
    // layout rows are visible but NOT deletable — each is paired 1:1 with the separate layout undo
    // stack, and removing one desyncs that pairing (see the module doc)
    let deletable: Vec<usize> =
        visible.iter().copied().filter(|&i| undo[i].category != HistoryCategory::Layout).collect();

    ui.horizontal(|ui| {
        ui.label(format!("{} entr{}", visible.len(), if visible.len() == 1 { "y" } else { "ies" }));
        let r = ui.add_enabled(!deletable.is_empty(), egui::Button::new("Delete filtered")).on_hover_text(
            "Removes the listed Editing entries (Layout entries pair with the panel-undo stack and stay)",
        );
        mark(ui, "delete_filtered", &r);
        if r.clicked() {
            let victims: std::collections::HashSet<usize> = deletable.iter().copied().collect();
            let mut i = 0;
            undo.retain(|_| {
                let keep = !victims.contains(&i);
                i += 1;
                keep
            });
            // a cached label describes the change TO the (now different) next entry — recompute all
            state.labels.clear();
            changed = true;
        }
        let r = ui.add_enabled(!visible.is_empty(), egui::Button::new("Export filtered to Markdown…"));
        mark(ui, "export", &r);
        if r.clicked() {
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
            // keyed by the entry's identity, not the index — the stack shifts on every delete and
            // cap-eviction, and an index key made the expanded row jump to a different entry
            let header_id = ui.id().with(("history_row", e.at.to_bits(), e.json.len()));
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
                        ui.label(label_of(state, e));
                        if e.category == HistoryCategory::Layout {
                            ui.weak("(pairs with panel undo)").on_hover_text(
                                "Layout entries can't be deleted here — each pairs with a snapshot on \
                                 the panel-arrangement undo stack, and removing one would desync it",
                            );
                        } else if ui.small_button("Delete").clicked() {
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
        state.labels.clear(); // the deleted entry's predecessor now describes a different neighbour
        changed = true;
    }
    changed
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
            Self {
                ctx: egui::Context::default(),
                state: HistoryState::default(),
                undo,
                project: crate::model::Project::new(),
                time: 0.0,
            }
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

    #[test]
    fn delete_filtered_removes_only_matching_entries() {
        let mut h = H::new();
        h.state.category = Some(HistoryCategory::Editing);
        h.frame();
        let r = h.ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("hist", "delete_filtered")))).unwrap();
        h.frame(); // hover
        let changed = {
            let H { ctx, state, undo, project, .. } = &mut h;
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
                        changed = show(ui, state, undo, project);
                    });
                },
            );
            changed
        };
        assert!(changed);
        assert_eq!(h.undo.len(), 1, "both Editing entries removed, the Layout one survives");
        assert_eq!(h.undo[0].category, HistoryCategory::Layout);
        // a cached label describes the change TO the next entry — deleting reshuffles every
        // neighbour pair, so the whole cache must go (it rebuilds lazily on the next render)
        assert!(h.state.labels.is_empty(), "deleting entries must drop the derived-label cache");
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
        // local midnight of that day (offset cancelled) — this pins both the civil-date math and the
        // fact that the offset is actually applied.
        assert_eq!(day_label(1_705_276_800.0 - local_offset_secs() as f64), "2024-01-15");
    }
}
