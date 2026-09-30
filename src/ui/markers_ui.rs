//! Markers pane: every marker in timeline order (project markers and clip markers together, via
//! `rows`), scoped to the sequence currently open (`Project.editing`) - a project-level marker only
//! shows on the sequence it was made on; clip markers scope naturally through their clip and are
//! unaffected. One header row: the label filter and a search over names and notes.
//!
//! One `CollapsingState` row per marker (collapsed by default): the icon glyph, its label colour, a dim
//! timecode and the name - clicking the row selects it AND seeks there; expanding it reveals the editable
//! name/time/duration/note and a small icon picker. Everything else is one right-click away:
//! - a row: Rename, Label ▸, Snap to nearest clip, Link to closest clip, Delete - on every selected
//!   marker when the row is part of the selection (one undo for the batch), else on that row alone;
//! - empty space: Add at playhead (M), Add on selected clip, Copy as list (markdown, every marker in the
//!   project - handy for notes and the AI tools), Export ▸ (CSV, YouTube chapters), Import….
//!
//! Multi-select (`state.selected: Vec<Id>`): click a row to select (replacing), Ctrl/Shift+click to
//! toggle it in/out - same pattern as clip and subtitle-cue selection elsewhere. A mini time strip
//! above the list plots every marker as a tick; dragging over empty strip space rubber-bands a time
//! range (Shift adds to the selection).

use crate::hotkeys::Action;
use crate::model::{Id, Project};
use crate::theme::Palette;
use crate::ui::tools::Glyph;
use crate::ui::{edit_start, menu, timecode};
use eframe::egui::{self, DragValue, Response, RichText};

/// Small, non-exhaustive set of glyphs relevant to a marker (the full picker lives in Settings ▸
/// Appearance ▸ Icons if someone wants an exotic one - this is just the quick picks).
const ICON_CHOICES: &[Glyph] = &[
    Glyph::Flag,
    Glyph::Bookmark,
    Glyph::Star,
    Glyph::Diamond,
    Glyph::Dot,
    Glyph::Camera,
    Glyph::MusicNote,
    Glyph::Target,
];

fn marker_glyph(name: &str) -> Glyph {
    Glyph::from_name(name).unwrap_or(Glyph::Flag)
}

#[derive(Default)]
pub struct MarkersState {
    pub selected: Vec<Id>,
    /// 0 = every label.
    pub filter_label: u8,
    /// Header search: case-insensitive, over names and notes.
    pub query: String,
    /// Drag-select band in progress on the mini time strip: (press time in seconds, "Shift held").
    band: Option<(f64, bool)>,
    /// Right-click ▸ Rename: open this row and focus its name field (cleared once focused).
    rename: Option<Id>,
}

/// A row's right-click verbs, applied to the row's targets after the list is drawn (one undo).
#[derive(Clone, Copy, PartialEq)]
enum RowOp {
    Label(u8),
    Snap,
    Link,
    Delete,
}

#[derive(Default)]
pub struct MarkersResponse {
    pub edited: bool,
    pub seek: Option<f64>,
}

/// The delete button used everywhere (planner, inspector, effects, mixer, …): a painted cross that
/// turns red on hover.
pub(crate) fn x_button(ui: &mut egui::Ui) -> Response {
    let r = crate::ui::tools::glyph_text_button(ui, Glyph::Cross, "");
    if r.hovered() {
        let c = egui::Color32::from_rgb(220, 70, 70);
        ui.painter().rect_stroke(r.rect, 2.0, egui::Stroke::new(1.0, c), egui::StrokeKind::Inside);
        crate::ui::tools::draw_glyph(ui.painter(), r.rect, Glyph::Cross, c);
    }
    r
}

/// A side pane's empty space as its right-click target: registered before anything is drawn, so every
/// row and widget sits on top of it and only truly empty space reaches it. Labels stop being
/// text-selectable in the pane too - a selectable label under the pointer would take the right-click
/// that should open its row's (or the pane's) menu.
pub(crate) fn menu_area(ui: &mut egui::Ui) -> Response {
    ui.style_mut().interaction.selectable_labels = false;
    ui.interact(ui.max_rect(), ui.id().with("pane_menu_area"), egui::Sense::click())
}

/// Test-only: remember a widget rect so headless tests can click the real button.
#[cfg(test)]
fn mark(ui: &egui::Ui, name: impl std::fmt::Display, r: &Response) {
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("mk", name.to_string())), r.rect));
}
#[cfg(not(test))]
fn mark(_ui: &egui::Ui, _name: impl std::fmt::Display, _r: &Response) {}

/// Where a marker lives: the project timeline, or a clip (with its start offset).
struct Row {
    id: Id,
    /// Timeline seconds.
    abs_t: f64,
    /// 0 for project markers.
    offset: f64,
    clip: Option<Id>,
}

/// Every marker in timeline order, project markers filtered to the sequence currently being edited
/// (clip markers are unaffected - they already scope through their clip), then by label (0 = any) and
/// by `query` (case-insensitive, name or note; empty = all).
fn rows(project: &Project, filter: u8, query: &str) -> Vec<Row> {
    let q = query.trim().to_lowercase();
    let keep = |m: &crate::model::Marker| {
        (filter == 0 || m.label == filter)
            && (q.is_empty() || m.name.to_lowercase().contains(&q) || m.note.to_lowercase().contains(&q))
    };
    let mut v: Vec<Row> = project
        .markers
        .iter()
        .filter(|m| keep(m) && m.sequence == project.editing)
        .map(|m| Row { id: m.id, abs_t: m.t, offset: 0.0, clip: None })
        .collect();
    for (_, c) in project.all_clips() {
        for m in c.markers.iter().filter(|m| keep(m)) {
            v.push(Row { id: m.id, abs_t: c.start + m.t, offset: c.start, clip: Some(c.id) });
        }
    }
    v.sort_by(|a, b| a.abs_t.total_cmp(&b.abs_t));
    v
}

/// Markdown list of every marker in timeline order (for notes / the AI tools) - every sequence, not
/// just the one currently open; this is a project-wide export, not the pane's scoped display.
fn as_markdown(project: &Project, fps: f64) -> String {
    let mut s = String::new();
    for (_, t, dur, name, _) in project.markers_in_timeline() {
        let name = if name.is_empty() { "(marker)".to_string() } else { name };
        if dur > 0.0 {
            s.push_str(&format!("- {} → {}  {name}\n", timecode(t, fps), timecode(t + dur, fps)));
        } else {
            s.push_str(&format!("- {}  {name}\n", timecode(t, fps)));
        }
    }
    s
}

// ---- ws:export-deliver ----
/// Text formats `export_markers` writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkerFmt {
    /// `time,duration,name,note,label` with a header row; `import_markers_csv` reads it back.
    Csv,
    /// `HH:MM:SS Name` per line - paste into a YouTube description. YouTube insists the list starts
    /// at 00:00:00, so an "Intro" line is prepended when the first marker doesn't.
    YoutubeChapters,
}

impl MarkerFmt {
    pub fn parse(s: &str) -> Option<MarkerFmt> {
        match s.to_ascii_lowercase().as_str() {
            "csv" => Some(MarkerFmt::Csv),
            "youtube_chapters" | "youtube" | "chapters" => Some(MarkerFmt::YoutubeChapters),
            _ => None,
        }
    }
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Split CSV text into logical records - like `.lines()`, but a `\n`/`\r\n` inside a quoted field
/// (RFC 4180 allows a literal newline there - `csv_field` emits one for a multi-line note) does not
/// end the record.
fn csv_records(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                cur.push(c);
                if quoted && chars.peek() == Some(&'"') {
                    cur.push(chars.next().unwrap());
                } else {
                    quoted = !quoted;
                }
            }
            '\r' if !quoted => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push(std::mem::take(&mut cur));
            }
            '\n' if !quoted => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// One CSV record → fields (RFC 4180 quoting: `""` inside quotes is a literal quote).
fn csv_split(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

/// Every marker (project + clip, every sequence) in timeline order as text. `fps` is unused by both
/// formats today (chapters are whole seconds, CSV keeps float seconds) but kept so a frame-based
/// format can slot in without changing callers.
pub fn export_markers(project: &Project, _fps: f64, fmt: MarkerFmt) -> String {
    let rows = project.markers_in_timeline();
    let mut s = String::new();
    match fmt {
        MarkerFmt::Csv => {
            s.push_str("time,duration,name,note,label\n");
            for (id, t, dur, name, label) in rows {
                let note = project
                    .markers
                    .iter()
                    .chain(project.all_clips().flat_map(|(_, c)| c.markers.iter()))
                    .find(|m| m.id == id)
                    .map(|m| m.note.clone())
                    .unwrap_or_default();
                s.push_str(&format!("{t:.3},{dur:.3},{},{},{label}\n", csv_field(&name), csv_field(&note)));
            }
        }
        MarkerFmt::YoutubeChapters => {
            let hms = |t: f64| {
                let secs = t.max(0.0).round() as u64;
                format!("{:02}:{:02}:{:02}", secs / 3600, (secs / 60) % 60, secs % 60)
            };
            if rows.first().is_none_or(|r| r.1 >= 1.0) {
                s.push_str("00:00:00 Intro\n");
            }
            for (_, t, _, name, _) in rows {
                let name = if name.is_empty() { "Chapter".to_string() } else { name };
                s.push_str(&format!("{} {name}\n", hms(t)));
            }
        }
    }
    s
}

/// Add project markers from `export_markers(Csv)` text (or any `time,name[,note,label]` /
/// `time,duration,name[,note,label]` CSV - the header row decides which). Returns the count added;
/// lines whose first field isn't a number (the header, blanks) are skipped.
pub fn import_markers_csv(project: &mut Project, csv: &str) -> usize {
    let records = csv_records(csv);
    let mut lines = records.iter().map(|s| s.trim()).filter(|l| !l.is_empty());
    let Some(first) = lines.next() else { return 0 };
    let header: Vec<String> = csv_split(first).iter().map(|f| f.trim().to_ascii_lowercase()).collect();
    let has_header = header.first().is_some_and(|h| h.parse::<f64>().is_err());
    let col = |name: &str, fallback: usize| header.iter().position(|h| h == name).unwrap_or(fallback);
    let (ti, di, ni, oi, li) = if has_header {
        (col("time", 0), col("duration", usize::MAX), col("name", 1), col("note", 2), col("label", 3))
    } else {
        (0, usize::MAX, 1, 2, 3)
    };
    let rows: Vec<&str> = if has_header { lines.collect() } else { std::iter::once(first).chain(lines).collect() };
    let mut n = 0;
    for line in rows {
        let f = csv_split(line);
        let Some(t) = f.get(ti).and_then(|s| s.trim().parse::<f64>().ok()) else { continue };
        let name = f.get(ni).map(|s| s.trim().to_string()).unwrap_or_default();
        let id = project.add_marker(t, name);
        if let Some(m) = project.marker_mut(id) {
            m.duration = f.get(di).and_then(|s| s.trim().parse::<f64>().ok()).unwrap_or(0.0).max(0.0);
            m.note = f.get(oi).map(|s| s.trim().to_string()).unwrap_or_default();
            m.label = f.get(li).and_then(|s| s.trim().parse::<u8>().ok()).unwrap_or(0);
        }
        n += 1;
    }
    n
}

/// Save dialog + write for the Markers toolbar's "Export…" and `Action::ExportMarkers`. Returns the
/// path written (the caller toasts / opens the folder).
pub fn export_markers_dialog(project: &Project, fmt: MarkerFmt) -> Result<Option<std::path::PathBuf>, String> {
    let (filter, ext) = match fmt {
        MarkerFmt::Csv => ("CSV", "csv"),
        MarkerFmt::YoutubeChapters => ("Text", "txt"),
    };
    let Some(out) = rfd::FileDialog::new()
        .add_filter(filter, &[ext])
        .set_file_name(format!("{}_markers.{ext}", project.name))
        .save_file()
    else {
        return Ok(None);
    };
    std::fs::write(&out, export_markers(project, project.fps, fmt)).map_err(|e| e.to_string())?;
    Ok(Some(out))
}

pub fn show(
    ui: &mut egui::Ui,
    state: &mut MarkersState,
    project: &mut Project,
    selection: &[Id],
    playhead: f64,
    palette: &Palette,
    undo: &mut dyn FnMut(&Project),
) -> MarkersResponse {
    let mut out = MarkersResponse::default();
    let fps = project.fps;
    let labels: Vec<(String, [u8; 3])> = project.labels.iter().map(|l| (l.name.clone(), l.color)).collect();
    let (mods, pointer, primary_down) = ui.input(|i| (i.modifiers, i.pointer.latest_pos(), i.pointer.primary_down()));
    let bg = menu_area(ui);

    let rows = rows(project, state.filter_label, &state.query);
    state.selected.retain(|id| rows.iter().any(|r| r.id == *id));
    let sel_clip = selection.iter().copied().find(|&id| project.clip(id).is_some());

    // ---- the one header row: label filter · search ----
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("marker_filter")
            .selected_text(if state.filter_label == 0 {
                "All labels".to_string()
            } else {
                project.label_name(state.filter_label).to_string()
            })
            .width(110.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut state.filter_label, 0, "All labels");
                for (i, (name, [r, g, b])) in labels.iter().enumerate() {
                    // the name is tinted with its own colour - no swatch character needed
                    let t = RichText::new(name.clone()).color(egui::Color32::from_rgb(*r, *g, *b));
                    ui.selectable_value(&mut state.filter_label, i as u8 + 1, t);
                }
            });
        let w = ui.available_width();
        ui.add(egui::TextEdit::singleline(&mut state.query).hint_text("Search markers").desired_width(w));
    });
    ui.separator();

    let mut row_op: Option<(RowOp, Vec<Id>)> = None;
    if rows.is_empty() {
        let key = menu::shortcut(Action::AddMarker);
        let none_at_all = project.markers.is_empty() && state.query.is_empty() && state.filter_label == 0;
        ui.weak(match (none_at_all, key.is_empty()) {
            (false, _) => "No markers match".to_string(),
            (true, true) => "No markers - right-click to add one at the playhead".to_string(),
            (true, false) => format!("No markers - press {key} to add one at the playhead"),
        });
    } else {
        // ---- mini time strip: a tick per marker, drag to rubber-band a time range ----
        let strip_h = 20.0;
        let span = rows.iter().map(|r| r.abs_t).fold(project.duration().max(1.0), f64::max);
        let (strip_rect, strip_resp) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), strip_h), egui::Sense::click_and_drag());
        let x_at = |t: f64| strip_rect.left() + ((t / span).clamp(0.0, 1.0) as f32) * strip_rect.width();
        let t_at = |x: f32| ((x - strip_rect.left()) / strip_rect.width()).clamp(0.0, 1.0) as f64 * span;
        let painter = ui.painter().clone();
        painter.rect_filled(strip_rect, 2.0, palette.header);

        if strip_resp.drag_started_by(egui::PointerButton::Primary) {
            if let Some(o) = strip_resp.interact_pointer_pos() {
                state.band = Some((t_at(o.x), mods.shift));
            }
        }
        if strip_resp.clicked() {
            state.selected.clear();
        }
        let band_range = state.band.and_then(|(t0, _)| {
            let t1 = t_at(pointer?.x);
            Some((t0.min(t1), t0.max(t1)))
        });
        if let Some((a, b)) = band_range {
            painter.rect_filled(
                egui::Rect::from_x_y_ranges(x_at(a)..=x_at(b), strip_rect.y_range()),
                0.0,
                palette.selection.gamma_multiply(0.25),
            );
        }
        if state.band.is_some() && !primary_down {
            let (_, add) = state.band.take().expect("checked above");
            if let Some((a, b)) = band_range {
                let hit: Vec<Id> = rows.iter().filter(|r| r.abs_t >= a && r.abs_t <= b).map(|r| r.id).collect();
                if !add {
                    state.selected.clear();
                }
                for h in hit {
                    if !state.selected.contains(&h) {
                        state.selected.push(h);
                    }
                }
            }
        }
        for row in &rows {
            let x = x_at(row.abs_t);
            let sel = state.selected.contains(&row.id);
            let c = if sel { palette.accent } else { palette.text_dim };
            painter.circle_filled(egui::pos2(x, strip_rect.center().y), if sel { 3.5 } else { 2.5 }, c);
        }
        ui.add_space(2.0);

        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            for row in &rows {
                // ponytail: edit a clone of the one marker and write it back - lets `undo` snapshot the
                // untouched project without holding a mutable borrow across the widgets.
                let Some(mut m) = project.marker_mut(row.id).map(|m| m.clone()) else { continue };
                let selected = state.selected.contains(&row.id);
                let (mut start, mut changed) = (false, false);
                let mut hit: Option<bool> = None; // Some(ctrl/shift toggle) when the row was clicked

                let header_id = ui.id().with(("marker_row", row.id));
                let mut cs =
                    egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), header_id, false);
                if state.rename == Some(row.id) {
                    cs.set_open(true);
                }
                cs.show_header(ui, |ui| {
                    // the whole row senses clicks under its widgets: click = select + seek, right-click = menu
                    let row_r = ui.scope_builder(egui::UiBuilder::new().sense(egui::Sense::click()), |ui| {
                        ui.horizontal(|ui| {
                            let (icon, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
                            crate::ui::tools::draw_glyph(ui.painter(), icon, marker_glyph(&m.icon), palette.text);
                            let color = match project.label_color(m.label) {
                                Some([r, g, b]) => egui::Color32::from_rgb(r, g, b),
                                None => palette.text_dim,
                            };
                            let (dot, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                            ui.painter().circle_filled(dot.center(), 5.0, color);
                            ui.weak(timecode(row.abs_t, fps));
                            let text = if m.name.is_empty() { "(marker)".to_string() } else { m.name.clone() };
                            ui.selectable_label(selected, text)
                        })
                        .inner
                    });
                    let name_r = row_r.inner;
                    mark(ui, format_args!("name{}", row.id), &name_r);
                    let r = row_r.response.union(name_r);
                    if r.clicked() {
                        hit = Some(mods.ctrl || mods.shift);
                    }
                    if r.secondary_clicked() && !selected {
                        state.selected = vec![row.id];
                    }
                    r.context_menu(|ui| {
                        let targets =
                            if state.selected.contains(&row.id) { state.selected.clone() } else { vec![row.id] };
                        let n = targets.len();
                        if n == 1 && menu::row(ui, None, "Rename", "").clicked() {
                            state.rename = Some(row.id);
                        }
                        menu::sub(ui, None, "Label", |ui| {
                            if menu::check(ui, m.label == 0, "None", "").clicked() {
                                row_op = Some((RowOp::Label(0), targets.clone()));
                            }
                            for (i, (name, [r, g, b])) in labels.iter().enumerate() {
                                let t = RichText::new(name.clone()).color(egui::Color32::from_rgb(*r, *g, *b));
                                if ui.selectable_label(m.label == i as u8 + 1, t).clicked() {
                                    row_op = Some((RowOp::Label(i as u8 + 1), targets.clone()));
                                    ui.close();
                                }
                            }
                        });
                        let r = menu::row(ui, None, "Snap to nearest clip", "")
                            .on_hover_text("Move each timeline marker to its nearest clip edge (start or end)");
                        if r.clicked() {
                            row_op = Some((RowOp::Snap, targets.clone()));
                        }
                        let r = menu::row(ui, None, "Link to closest clip", "")
                            .on_hover_text("Turn each timeline marker into a marker on its nearest clip");
                        if r.clicked() {
                            row_op = Some((RowOp::Link, targets.clone()));
                        }
                        ui.separator();
                        let label = if n > 1 { format!("Delete {n} markers") } else { "Delete".into() };
                        if menu::row(ui, Some(Glyph::Cross), &label, "").clicked() {
                            row_op = Some((RowOp::Delete, targets.clone()));
                        }
                    });
                })
                .body(|ui| {
                    let r = ui.add(egui::TextEdit::singleline(&mut m.name).hint_text("marker"));
                    if state.rename == Some(row.id) {
                        r.request_focus();
                        state.rename = None;
                    }
                    start |= r.gained_focus();
                    changed |= r.changed();
                    ui.horizontal(|ui| {
                        let mut t = m.t;
                        let r =
                            ui.add(DragValue::new(&mut t).speed(0.05).range(0.0..=1e6).suffix(" s").fixed_decimals(2));
                        if r.changed() {
                            m.t = t;
                        }
                        start |= edit_start(&r);
                        changed |= r.changed();
                        if m.duration > 0.0 {
                            let r = ui.add(DragValue::new(&mut m.duration).speed(0.05).range(0.0..=1e6).prefix("+"));
                            start |= edit_start(&r);
                            changed |= r.changed();
                        }
                    });
                    let r = ui.add(
                        egui::TextEdit::multiline(&mut m.note)
                            .desired_rows(2)
                            .desired_width(f32::INFINITY)
                            .hint_text("note"),
                    );
                    start |= r.gained_focus();
                    changed |= r.changed();
                    ui.horizontal_wrapped(|ui| {
                        for g in ICON_CHOICES {
                            let picked = m.icon == g.name();
                            let btn = crate::ui::tools::glyph_text_button(ui, *g, "");
                            if picked {
                                ui.painter().rect_stroke(
                                    btn.rect,
                                    2.0,
                                    egui::Stroke::new(1.5, palette.accent),
                                    egui::StrokeKind::Inside,
                                );
                            }
                            if btn.on_hover_text(g.name()).clicked() {
                                m.icon = g.name().to_string();
                                start = true;
                                changed = true;
                            }
                        }
                    });
                });

                match hit {
                    Some(true) if selected => state.selected.retain(|id| *id != row.id),
                    Some(true) => state.selected.push(row.id),
                    Some(false) => {
                        // "when I click on a marker, it should reposition my playhead"
                        state.selected = vec![row.id];
                        out.seek = Some(m.t + row.offset);
                    }
                    None => {}
                }

                if start {
                    undo(project);
                }
                if changed {
                    if row.clip.is_some() {
                        // clip markers stay inside their clip
                        if let Some(c) = row.clip.and_then(|cid| project.clip(cid)) {
                            m.t = m.t.clamp(0.0, c.duration);
                        }
                    }
                    if let Some(dst) = project.marker_mut(row.id) {
                        *dst = m;
                    }
                    out.edited = true;
                }
            }
        });
    }

    // ---- empty space: the pane's verbs ----
    bg.context_menu(|ui| {
        menu::action_item(ui, Action::AddMarker);
        let r = ui.add_enabled_ui(sel_clip.is_some(), |ui| menu::row(ui, None, "Add on selected clip", "")).inner;
        if r.on_disabled_hover_text("Select a clip first").clicked() {
            if let Some(cid) = sel_clip {
                let local = project.clip(cid).map(|c| (playhead - c.start).clamp(0.0, c.duration)).unwrap_or(0.0);
                undo(project);
                if let Some(id) = project.add_clip_marker(cid, local, "") {
                    state.selected = vec![id];
                }
                out.edited = true;
            }
        }
        ui.separator();
        if menu::row(ui, None, "Copy as list", "").on_hover_text("Markdown, for notes and the AI tools").clicked() {
            ui.ctx().copy_text(as_markdown(project, fps));
        }
        // ---- ws:export-deliver ----
        menu::sub(ui, Some(Glyph::ExportArrow), "Export", |ui| {
            for (label, fmt) in [("CSV…", MarkerFmt::Csv), ("YouTube chapters…", MarkerFmt::YoutubeChapters)] {
                if menu::row(ui, None, label, "").clicked() {
                    // ponytail: the toast lives with the Action (App::act_export_markers); this inline
                    // path just writes, since a leaf pane has no toast handle
                    let _ = export_markers_dialog(project, fmt);
                }
            }
        });
        let r = menu::row(ui, Some(Glyph::ImportArrow), "Import…", "")
            .on_hover_text("Add markers from a CSV (time,name[,note,label])");
        if r.clicked() {
            if let Some(p) = rfd::FileDialog::new().add_filter("CSV", &["csv"]).pick_file() {
                if let Ok(text) = std::fs::read_to_string(&p) {
                    undo(project);
                    if import_markers_csv(project, &text) > 0 {
                        out.edited = true;
                    }
                }
            }
        }
    });

    if let Some((op, ids)) = row_op {
        undo(project);
        for &id in &ids {
            match op {
                RowOp::Label(l) => {
                    if let Some(m) = project.marker_mut(id) {
                        m.label = l;
                    }
                }
                RowOp::Snap => project.snap_marker_to_nearest_clip(id),
                RowOp::Link => {
                    project.link_marker_to_closest_clip(id);
                }
                RowOp::Delete => project.remove_marker(id),
            }
        }
        if op == RowOp::Delete {
            state.selected.retain(|s| !ids.contains(s));
        }
        out.edited = true;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Clip, ClipKind};
    use eframe::egui::{Color32, Event, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2};

    struct H {
        ctx: egui::Context,
        project: Project,
        state: MarkersState,
        selection: Vec<Id>,
        undos: usize,
        time: f64,
        shapes: Vec<egui::epaint::ClippedShape>,
    }

    impl H {
        fn new() -> Self {
            let mut project = Project::new();
            project.tracks[0].clips.push(Clip::new(9, ClipKind::Video, "v", 2.0, 4.0));
            let ctx = egui::Context::default();
            ctx.set_fonts(crate::theme::test_fonts()); // real glyph sizes, so popup entries land where drawn
            Self {
                ctx,
                project,
                state: MarkersState::default(),
                selection: vec![9],
                undos: 0,
                time: 0.0,
                shapes: Vec::new(),
            }
        }
        fn frame(&mut self, events: Vec<Event>) -> MarkersResponse {
            self.frame_mod(events, Modifiers::NONE)
        }
        /// `show()` reads the CURRENT modifier state via `ui.input(|i| i.modifiers)` (the RawInput-level
        /// field), not each event's own `modifiers` - a click event's embedded modifiers alone (as
        /// `frame` alone would send) is not enough to make Ctrl/Shift-click register.
        fn frame_mod(&mut self, events: Vec<Event>, modifiers: Modifiers) -> MarkersResponse {
            self.time += 0.05;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(520.0, 600.0))),
                time: Some(self.time),
                modifiers,
                events,
                ..Default::default()
            };
            let pal = Palette::new(true, Color32::WHITE);
            let H { ctx, project, state, selection, undos, shapes, .. } = self;
            let mut out = MarkersResponse::default();
            let full = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut undo = |_: &Project| *undos += 1;
                    out = show(ui, state, project, selection, 3.0, &pal, &mut undo);
                });
            });
            *shapes = full.shapes;
            out
        }
        /// Centre of the first painted text equal to `label` - how a popup's entries are found.
        fn text_at(&self, label: &str) -> Option<Pos2> {
            self.shapes.iter().find_map(|c| match &c.shape {
                egui::epaint::Shape::Text(t) if t.galley.text() == label => Some(t.visual_bounding_rect().center()),
                _ => None,
            })
        }
        /// Right-click `at`, then click the menu entry `label`.
        fn menu(&mut self, at: Pos2, label: &str) -> MarkersResponse {
            self.frame(vec![Event::PointerMoved(at)]);
            for pressed in [true, false] {
                self.frame(vec![Event::PointerButton {
                    pos: at,
                    button: PointerButton::Secondary,
                    pressed,
                    modifiers: Modifiers::NONE,
                }]);
            }
            self.frame(vec![]);
            let item = self.text_at(label).unwrap_or_else(|| panic!("no '{label}' in the menu"));
            self.time += 1.0; // a separate gesture, not a double-click
            self.click(item)
        }
        fn rect(&self, name: &str) -> Rect {
            self.ctx
                .data(|d| d.get_temp::<Rect>(egui::Id::new(("mk", name.to_string()))))
                .unwrap_or_else(|| panic!("no widget rect recorded for {name}"))
        }
        fn click_mod(&mut self, pos: Pos2, modifiers: Modifiers) -> MarkersResponse {
            self.frame_mod(vec![Event::PointerMoved(pos)], modifiers);
            self.frame_mod(
                vec![Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers }],
                modifiers,
            );
            let out = self.frame_mod(
                vec![Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers }],
                modifiers,
            );
            self.frame(vec![]);
            out
        }
        fn click(&mut self, pos: Pos2) -> MarkersResponse {
            self.click_mod(pos, Modifiers::NONE)
        }
        fn ctrl_click(&mut self, pos: Pos2) -> MarkersResponse {
            self.click_mod(pos, Modifiers::CTRL)
        }
    }

    #[test]
    fn click_seeks_and_row_menu_deletes() {
        let mut h = H::new();
        let id = h.project.add_marker(1.25, "a");
        h.frame(vec![]);
        let out = h.click(h.rect(&format!("name{id}")).center());
        assert_eq!(out.seek, Some(1.25), "clicking a row seeks to the marker (no Go button needed)");
        assert_eq!(h.state.selected, vec![id]);
        assert_eq!(h.undos, 0, "selecting is not an edit");

        h.time += 1.0;
        let at = h.rect(&format!("name{id}")).center();
        let out = h.menu(at, "Delete");
        assert!(out.edited && h.project.markers.is_empty(), "right-click ▸ Delete removes it");
        assert_eq!(h.undos, 1, "delete is one undo");
        assert!(h.state.selected.is_empty());
    }

    /// Empty space ▸ "Add on selected clip" drops a clip marker at the playhead; "Add at playhead" is
    /// the M Action itself, queued through the shared menu helper.
    #[test]
    fn empty_space_menu_adds_markers() {
        let mut h = H::new(); // clip 9 at 2..6 is selected, the playhead is at 3
        h.frame(vec![]);
        let empty = Pos2::new(260.0, 500.0);
        let out = h.menu(empty, "Add on selected clip");
        assert!(out.edited);
        let c = h.project.clip(9).unwrap();
        assert_eq!(c.markers.len(), 1);
        assert!((c.markers[0].t - 1.0).abs() < 1e-9, "clip-local 1 s = timeline 3 s");
        assert_eq!(h.undos, 1);
        let _ = menu::take_queued();
        h.time += 1.0;
        h.menu(empty, Action::AddMarker.label());
        assert_eq!(menu::take_queued(), vec![Action::AddMarker], "Add at playhead runs the M Action");
    }

    #[test]
    fn search_filters_by_name_and_note() {
        let mut p = Project::new();
        p.add_marker(1.0, "Intro");
        let b = p.add_marker(2.0, "Hook");
        p.marker_mut(b).unwrap().note = "the big reveal".into();
        assert_eq!(rows(&p, 0, "intro").len(), 1, "case-insensitive name match");
        assert_eq!(rows(&p, 0, "REVEAL").iter().map(|r| r.id).collect::<Vec<_>>(), vec![b], "notes count");
        assert!(rows(&p, 0, "nothing like it").is_empty());
        assert_eq!(rows(&p, 0, "  ").len(), 2, "blank = everything");
    }

    #[test]
    fn clip_markers_are_offset_and_filtered() {
        let mut p = Project::new();
        p.tracks[0].clips.push(Clip::new(9, ClipKind::Video, "v", 2.0, 4.0));
        p.add_marker(3.0, "project");
        p.add_clip_marker(9, 1.0, "on clip");
        let r = rows(&p, 0, "");
        assert_eq!(r.len(), 2);
        assert!(r.iter().all(|x| x.abs_t == 3.0), "both land at 3 s");
        let clip_row = r.iter().find(|x| x.clip.is_some()).expect("clip marker");
        assert_eq!(clip_row.offset, 2.0, "clip markers carry their clip start");
        p.markers[0].label = 2;
        assert_eq!(rows(&p, 2, "").len(), 1);
        assert_eq!(rows(&p, 1, "").len(), 0);
        assert_eq!(rows(&p, 0, "").len(), 2);
    }

    #[test]
    fn project_markers_scope_to_the_current_sequence() {
        let mut p = Project::new();
        p.add_marker(1.0, "main"); // editing == None
        p.editing = Some(42);
        p.add_marker(2.0, "other seq");
        assert_eq!(rows(&p, 0, "").len(), 1, "only the marker made on the current sequence shows");
        p.editing = None;
        assert_eq!(rows(&p, 0, "").len(), 1, "back on main, the other sequence's marker is hidden");
    }

    #[test]
    fn markdown_list_has_a_line_per_marker() {
        let mut p = Project::new();
        p.add_marker(1.0, "a");
        p.add_marker(2.5, "");
        let md = as_markdown(&p, 30.0);
        assert_eq!(md.lines().count(), 2);
        assert!(md.contains(" a"), "{md}");
        assert!(md.contains("(marker)"), "{md}");
    }

    // ---- ws:export-deliver ----
    #[test]
    fn markers_csv_round_trip() {
        let mut p = Project::new();
        p.tracks[0].clips.push(Clip::new(9, ClipKind::Video, "v", 2.0, 4.0));
        let a = p.add_marker(1.5, "Intro, part \"one\"");
        p.marker_mut(a).unwrap().note = "line\nbreak".into();
        p.marker_mut(a).unwrap().label = 2;
        let b = p.add_marker(4.0, "");
        p.marker_mut(b).unwrap().duration = 0.5;
        p.add_clip_marker(9, 1.0, "on clip"); // lands at 3.0
        let csv = export_markers(&p, 30.0, MarkerFmt::Csv);
        assert!(csv.starts_with("time,duration,name,note,label\n"), "{csv}");
        // record count, not `.lines()`: one note is a quoted multi-line field, so it spans 2 physical
        // lines on its own - `.lines()` would overcount, which is exactly the bug `csv_records` fixes
        assert_eq!(csv_records(&csv).len(), 4, "{csv}");
        let mut q = Project::new();
        assert_eq!(import_markers_csv(&mut q, &csv), 3);
        let got: Vec<(f64, f64, String, u8)> =
            q.markers.iter().map(|m| (m.t, m.duration, m.name.clone(), m.label)).collect();
        assert_eq!(
            got,
            vec![
                (1.5, 0.0, "Intro, part \"one\"".to_string(), 2),
                (3.0, 0.0, "on clip".to_string(), 0),
                (4.0, 0.5, String::new(), 0),
            ]
        );
        assert_eq!(q.markers[0].note, "line\nbreak");
        // a bare `time,name` file (no header) imports too; junk lines are skipped, not fatal
        let mut r = Project::new();
        assert_eq!(import_markers_csv(&mut r, "7.25,seven\n\nnot a time,x\n9,nine"), 2);
        assert_eq!((r.markers[0].t, r.markers[0].name.as_str()), (7.25, "seven"));
        // chapters: every line starts with HH:MM:SS, the list starts at zero
        let yt = export_markers(&p, 30.0, MarkerFmt::YoutubeChapters);
        for l in yt.lines() {
            let ts = l.split(' ').next().unwrap();
            assert_eq!(ts.len(), 8, "{l}");
            assert!(
                ts.chars().enumerate().all(|(i, c)| if i == 2 || i == 5 { c == ':' } else { c.is_ascii_digit() }),
                "{l}"
            );
        }
        assert!(yt.starts_with("00:00:00 Intro\n"), "{yt}");
        assert!(yt.contains("00:00:03 on clip"), "{yt}");
        assert!(yt.contains("00:00:04 Chapter"), "unnamed markers still get a chapter title: {yt}");
        assert_eq!(MarkerFmt::parse("youtube_chapters"), Some(MarkerFmt::YoutubeChapters));
        assert_eq!(MarkerFmt::parse("nope"), None);
    }

    #[test]
    fn empty_pane_is_harmless() {
        let mut h = H::new();
        let out = h.frame(vec![]);
        assert!(!out.edited && out.seek.is_none());
        assert_eq!(h.undos, 0);
    }

    #[test]
    fn multi_select_ctrl_toggle() {
        let mut h = H::new();
        let a = h.project.add_marker(1.0, "a");
        let b = h.project.add_marker(2.0, "b");

        h.frame(vec![]);
        let ra = h.rect(&format!("name{a}"));
        h.click(ra.center());
        assert_eq!(h.state.selected, vec![a], "plain click selects, replacing");

        h.frame(vec![]);
        let rb = h.rect(&format!("name{b}"));
        h.ctrl_click(rb.center());
        assert_eq!(h.state.selected, vec![a, b], "ctrl+click adds to the selection");

        h.frame(vec![]);
        let ra2 = h.rect(&format!("name{a}"));
        h.ctrl_click(ra2.center());
        assert_eq!(h.state.selected, vec![b], "ctrl+click again toggles it back off");
    }

    #[test]
    fn row_menu_deletes_the_whole_selection_with_one_undo() {
        let mut h = H::new();
        let a = h.project.add_marker(1.0, "a");
        let b = h.project.add_marker(2.0, "b");
        h.state.selected = vec![a, b];
        h.frame(vec![]);
        let at = h.rect(&format!("name{b}")).center();
        h.menu(at, "Delete 2 markers");
        assert!(h.project.markers.is_empty(), "both selected markers are removed");
        assert_eq!(h.undos, 1, "one undo for the whole batch");
        assert!(h.state.selected.is_empty());
    }

    #[test]
    fn row_menu_snaps_every_selected_marker_with_one_undo() {
        let mut h = H::new();
        h.project.tracks[0].clips.push(Clip::new(10, ClipKind::Video, "c2", 20.0, 2.0));
        let a = h.project.add_marker(1.0, "a"); // nearer clip 9 (start 2.0)
        let b = h.project.add_marker(19.0, "b"); // nearer clip 10 (start 20.0)
        let lone = h.project.add_marker(12.0, "lone");
        h.state.selected = vec![a, b];
        h.frame(vec![]);
        let at = h.rect(&format!("name{a}")).center();
        h.menu(at, "Snap to nearest clip");
        assert_eq!(h.project.marker_mut(a).unwrap().t, 2.0);
        assert_eq!(h.project.marker_mut(b).unwrap().t, 20.0);
        assert_eq!(h.project.marker_mut(lone).unwrap().t, 12.0, "an unselected marker stays put");
        assert_eq!(h.undos, 1, "one undo for the whole batch");
    }

    /// Right-clicking a row outside the selection acts on that row alone (and selects it).
    #[test]
    fn row_menu_on_an_unselected_row_targets_just_it() {
        let mut h = H::new();
        let a = h.project.add_marker(1.0, "a");
        let b = h.project.add_marker(2.0, "b");
        h.state.selected = vec![a];
        h.frame(vec![]);
        let at = h.rect(&format!("name{b}")).center();
        h.menu(at, "Delete");
        assert_eq!(h.project.markers.iter().map(|m| m.id).collect::<Vec<_>>(), vec![a]);
    }
}
