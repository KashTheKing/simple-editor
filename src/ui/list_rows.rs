//! ---- ws:thread-viewers ----
//! Shared list rows: a vertical list is a TABLE, not a line of widgets. Every row has the same columns at
//! the same x - [grip] [arrow] [label: fills the rest and truncates with an ellipsis] [fixed value
//! columns, right-aligned] [fixed controls, right-aligned] - so nothing floats after variable-length text.
//! Reordering is built in: drag a row's grip (a ghost follows the pointer, a line shows where it lands,
//! the enclosing scroll area follows near its edges, a horizontal move picks the nesting of a nested list,
//! Esc cancels), or select a row and press Alt+Up / Alt+Down, or Alt+Shift+Up / Down (Alt+Left / Alt+Right nest). The helper only
//! reports what happened; the caller applies it as one undo step.
//!
//! ```ignore
//! let mut rows = Rows::new(ui, "my_list", CTRLS, &[60.0], false);
//! for (i, it) in items.iter().enumerate() {
//!     let r = rows.row(ui, i, &RowSpec { key: it.id as u64, label: &it.name, ..Default::default() });
//!     // r.ctrl == Some(k): control k was clicked; r.toggled: the arrow was
//!     // (draw the row's body here if it is open)
//! }
//! if let Some(m) = rows.finish(ui) { /* Moved::Slot / Step / Indent */ }
//! ```

use eframe::egui::{self, vec2, Align2, Color32, FontId, Id, Key, Modifiers, Pos2, Rect, Sense, Stroke, Ui};

pub const ROW_H: f32 = 22.0;
const GRIP_W: f32 = 16.0;
const ARROW_W: f32 = 16.0;
const CTRL_W: f32 = 24.0;
const INDENT: f32 = 18.0;

/// A control button at the right of every row: its glyph and its tooltip.
pub struct Ctrl {
    pub glyph: &'static str,
    pub hint: &'static str,
}

#[derive(Default)]
pub struct RowSpec<'a> {
    /// Identifies the row across moves (selection, drag).
    pub key: u64,
    pub label: &'a str,
    /// Indentation levels (nested lists).
    pub depth: u8,
    /// Some = an arrow that folds / opens the row's body.
    pub open: Option<bool>,
    /// Greyed (a row hidden under a folded parent).
    pub dim: bool,
    /// One text per fixed value column.
    pub values: &'a [&'a str],
}

pub struct RowResp {
    pub toggled: bool,
    /// The control (index into the list's controls) that was clicked.
    pub ctrl: Option<usize>,
    /// Where the controls start, for tests: the same for every row.
    pub ctrl_x: f32,
    /// The label's rectangle.
    pub label: Rect,
    /// The whole row (click / double-click / right-click).
    pub body: egui::Response,
}

/// What a drag or a key asked for (rows are numbered as they were drawn).
#[derive(Clone, Debug, PartialEq)]
pub enum Moved {
    /// Dropped before row `slot` (`rows.len()` = at the end); `depth` = the nesting the pointer chose.
    Slot { from: usize, slot: usize, depth: Option<u8> },
    /// Alt+Up / Alt+Down on the selected row.
    Step { from: usize, delta: i32 },
    /// Alt+Left / Alt+Right on the selected row.
    Indent { from: usize, delta: i32 },
}

pub struct Rows {
    id: Id,
    ctrls: &'static [Ctrl],
    widths: Vec<f32>,
    nested: bool,
    rects: Vec<Rect>,
    keys: Vec<u64>,
    labels: Vec<String>,
    /// No grip and no moves: a list whose order is not the user's (a chat list sorted by date).
    fixed: bool,
}

impl Rows {
    pub fn new(ui: &Ui, id: impl std::hash::Hash, ctrls: &'static [Ctrl], value_widths: &[f32], nested: bool) -> Rows {
        Rows { id: ui.make_persistent_id(id), ctrls, widths: value_widths.to_vec(), nested, rects: Vec::new(), keys: Vec::new(), labels: Vec::new(), fixed: false }
    }

    /// A list the user doesn't reorder: no grip, no drag, no Alt+arrow moves.
    pub fn fixed_order(mut self) -> Rows {
        self.fixed = true;
        self
    }

    fn drag_id(&self) -> Id {
        self.id.with("drag")
    }
    fn sel_id(&self) -> Id {
        self.id.with("sel")
    }

    /// Draw row `i`. Its body (if open) is drawn by the caller after this returns.
    pub fn row(&mut self, ui: &mut Ui, i: usize, spec: &RowSpec) -> RowResp {
        let (rect, body) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
        let selected = ui.data(|d| d.get_temp::<u64>(self.sel_id())) == Some(spec.key);
        let dragging = ui.data(|d| d.get_temp::<u64>(self.drag_id())) == Some(spec.key);
        let v = ui.visuals();
        let painter = ui.painter_at(rect);
        if selected {
            painter.rect_filled(rect, 3.0, v.selection.bg_fill.gamma_multiply(0.5));
        } else if body.hovered() {
            painter.rect_filled(rect, 3.0, v.widgets.hovered.weak_bg_fill);
        }
        let ink = if spec.dim { v.weak_text_color() } else { v.text_color() };
        let dot = v.weak_text_color();
        let font = FontId::proportional(13.0);
        let mut out = RowResp { toggled: false, ctrl: None, ctrl_x: 0.0, label: Rect::NOTHING, body: body.clone() };
        // grip: a drag handle
        let grip = Rect::from_min_size(rect.min, vec2(GRIP_W, ROW_H));
        let g = ui.interact(grip, self.id.with(("grip", spec.key)), if self.fixed { Sense::hover() } else { Sense::drag() });
        if g.hovered() || g.dragged() {
            ui.ctx().set_cursor_icon(if g.dragged() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
        }
        if !self.fixed {
            g.clone().on_hover_text("Drag to reorder. Select a row, then Alt+Shift+Up / Down (or Alt+Up / Down) moves it, Alt+Left / Right nests it.");
            for (dx, dy) in [(-2.0, -5.0), (2.0, -5.0), (-2.0, 0.0), (2.0, 0.0), (-2.0, 5.0), (2.0, 5.0)] {
                painter.circle_filled(grip.center() + vec2(dx, dy), 1.3, dot);
            }
        }
        if g.drag_started() {
            ui.data_mut(|d| {
                d.insert_temp(self.drag_id(), spec.key);
                d.insert_temp(self.sel_id(), spec.key);
            });
        }
        // arrow
        let arrow = Rect::from_min_size(rect.min + vec2(GRIP_W, 0.0), vec2(ARROW_W, ROW_H));
        if let Some(open) = spec.open {
            let a = ui.interact(arrow, self.id.with(("arrow", spec.key)), Sense::click());
            let c = arrow.center();
            let pts = if open { vec![c + vec2(-4.0, -2.5), c + vec2(4.0, -2.5), c + vec2(0.0, 3.5)] } else { vec![c + vec2(-2.5, -4.0), c + vec2(-2.5, 4.0), c + vec2(3.5, 0.0)] };
            painter.add(egui::Shape::convex_polygon(pts, if a.hovered() { v.text_color() } else { dot }, Stroke::NONE));
            out.toggled = a.clicked();
        }
        // controls: the same columns on every row
        let ctrl_x0 = rect.right() - CTRL_W * self.ctrls.len() as f32;
        out.ctrl_x = ctrl_x0;
        for (k, c) in self.ctrls.iter().enumerate() {
            let r = Rect::from_min_size(Pos2::new(ctrl_x0 + CTRL_W * k as f32, rect.top()), vec2(CTRL_W, ROW_H));
            let resp = ui.interact(r, self.id.with(("ctrl", spec.key, k)), Sense::click()).on_hover_text(c.hint);
            if resp.hovered() {
                painter.rect_filled(r.shrink(1.0), 3.0, v.widgets.hovered.bg_fill);
            }
            painter.text(r.center(), Align2::CENTER_CENTER, c.glyph, font.clone(), if resp.hovered() { v.strong_text_color() } else { ink });
            if resp.clicked() {
                out.ctrl = Some(k);
            }
        }
        // fixed value columns, right-aligned, just left of the controls
        let mut x = ctrl_x0 - 4.0;
        for (k, w) in self.widths.iter().enumerate().rev() {
            if let Some(t) = spec.values.get(k) {
                painter.text(Pos2::new(x, rect.center().y), Align2::RIGHT_CENTER, *t, font.clone(), v.weak_text_color());
            }
            x -= *w;
        }
        // the label takes what is left and ends in an ellipsis
        // a fixed list without arrows needs neither column: its label starts at the row's edge
        let lead = if self.fixed && spec.open.is_none() { rect.left() + 6.0 } else { arrow.right() };
        let left = lead + INDENT * spec.depth as f32;
        out.label = Rect::from_min_max(Pos2::new(left, rect.top()), Pos2::new((x - 4.0).max(left + 20.0), rect.bottom()));
        let mut job = egui::text::LayoutJob::simple_singleline(spec.label.to_string(), font, ink);
        job.wrap = egui::text::TextWrapping::truncate_at_width(out.label.width());
        let galley = ui.painter().layout_job(job);
        painter.galley(Pos2::new(left, rect.center().y - galley.size().y / 2.0), galley, ink);
        if body.clicked() || g.clicked() {
            ui.data_mut(|d| d.insert_temp(self.sel_id(), spec.key));
        }
        if dragging {
            painter.rect_filled(rect, 3.0, Color32::from_black_alpha(60));
        }
        self.rects.push(rect);
        self.keys.push(spec.key);
        self.labels.push(spec.label.to_string());
        out
    }

    /// After the last row: the drag in progress (ghost, insertion line, scrolling), what it dropped, and
    /// the keyboard moves of the selected row.
    pub fn finish(self, ui: &mut Ui) -> Option<Moved> {
        let n = self.rects.len();
        if n == 0 || self.fixed {
            return None;
        }
        if let Some(key) = ui.data(|d| d.get_temp::<u64>(self.drag_id())) {
            let from = self.keys.iter().position(|k| *k == key);
            let Some(p) = ui.ctx().pointer_interact_pos() else {
                ui.data_mut(|d| d.remove::<u64>(self.drag_id()));
                return None;
            };
            if ui.input(|i| i.key_pressed(Key::Escape)) {
                ui.data_mut(|d| d.remove::<u64>(self.drag_id()));
                return None;
            }
            // the slot under the pointer: before the first row whose middle is below it
            let slot = self.rects.iter().position(|r| p.y < r.center().y).unwrap_or(n);
            let first = self.rects[0];
            let depth = self.nested.then(|| (((p.x - (first.left() + GRIP_W + ARROW_W)) / INDENT).floor().max(0.0) as u8).min(8));
            let y = if slot < n { self.rects[slot].top() } else { self.rects[n - 1].bottom() };
            let layer = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, self.id.with("ghost")));
            let accent = ui.visuals().selection.stroke.color;
            let lx = first.left() + GRIP_W + ARROW_W + INDENT * depth.unwrap_or(0) as f32;
            layer.line_segment([Pos2::new(lx, y), Pos2::new(first.right(), y)], Stroke::new(2.0, accent));
            layer.circle_filled(Pos2::new(lx, y), 3.5, accent);
            if let Some(f) = from {
                let w = (first.width() * 0.8).min(320.0);
                let r = Rect::from_min_size(Pos2::new(p.x - 12.0, p.y - ROW_H / 2.0), vec2(w, ROW_H));
                layer.rect_filled(r, 4.0, ui.visuals().window_fill.gamma_multiply(0.92));
                layer.rect_stroke(r, 4.0, Stroke::new(1.0, accent), egui::StrokeKind::Inside);
                layer.text(r.left_center() + vec2(8.0, 0.0), Align2::LEFT_CENTER, &self.labels[f], FontId::proportional(13.0), ui.visuals().text_color());
            }
            // near the edge of the scroll area it scrolls
            let clip = ui.clip_rect();
            if p.y < clip.top() + 28.0 {
                ui.scroll_with_delta(vec2(0.0, 14.0));
            } else if p.y > clip.bottom() - 28.0 {
                ui.scroll_with_delta(vec2(0.0, -14.0));
            }
            ui.ctx().request_repaint();
            if ui.input(|i| i.pointer.any_released()) {
                ui.data_mut(|d| d.remove::<u64>(self.drag_id()));
                return from.map(|from| Moved::Slot { from, slot, depth });
            }
            return None;
        }
        // keys on the selected row (not while a text box is being typed in)
        let sel = ui.data(|d| d.get_temp::<u64>(self.sel_id()))?;
        let from = self.keys.iter().position(|k| *k == sel)?;
        if ui.ctx().wants_keyboard_input() {
            return None;
        }
        // Alt+arrow, or Alt+Shift+arrow where an app shortcut already owns Alt+Up (Back to Parent Timeline)
        let key = |ui: &mut Ui, k: Key| ui.input_mut(|i| i.consume_key(Modifiers::ALT, k) | i.consume_key(Modifiers::ALT | Modifiers::SHIFT, k));
        if key(ui, Key::ArrowUp) {
            return Some(Moved::Step { from, delta: -1 });
        }
        if key(ui, Key::ArrowDown) {
            return Some(Moved::Step { from, delta: 1 });
        }
        if self.nested && key(ui, Key::ArrowLeft) {
            return Some(Moved::Indent { from, delta: -1 });
        }
        if self.nested && key(ui, Key::ArrowRight) {
            return Some(Moved::Indent { from, delta: 1 });
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CTRLS: &[Ctrl] = &[Ctrl { glyph: "a", hint: "A" }, Ctrl { glyph: "b", hint: "B" }, Ctrl { glyph: "c", hint: "C" }];

    fn draw(ctx: &egui::Context, labels: &[&str], width: f32) -> Vec<RowResp> {
        let mut out = Vec::new();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.set_width(width);
                let mut rows = Rows::new(ui, "t", CTRLS, &[50.0], true);
                for (i, l) in labels.iter().enumerate() {
                    out.push(rows.row(ui, i, &RowSpec { key: i as u64, label: l, depth: (i % 2) as u8, open: Some(i == 1), values: &["4:32 PM"], ..Default::default() }));
                }
                assert!(rows.finish(ui).is_none());
            });
        });
        out
    }

    /// The controls are at the same x on every row whatever the text, a long text is cut short of them,
    /// and drawing the list asks for no frames.
    #[test]
    fn assert_no_idle_repaint_list_rows_and_aligned_columns() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let labels = ["x", "a short one", "a message that is far, far too long to ever fit in the row of a narrow inspector panel, so it must be cut", "ii"];
        for _ in 0..10 {
            draw(&ctx, &labels, 320.0);
        }
        let rows = draw(&ctx, &labels, 320.0);
        assert!(rows.iter().all(|r| r.ctrl_x == rows[0].ctrl_x && r.ctrl_x > 150.0), "controls are one column: {:?}", rows.iter().map(|r| r.ctrl_x).collect::<Vec<_>>());
        assert!(rows.iter().all(|r| r.label.right() <= rows[0].ctrl_x), "no label runs under the controls");
        assert!(rows[2].label.width() > 60.0 && rows[2].label.right() <= rows[0].ctrl_x);
        // a different width moves the column for every row together
        let wide = draw(&ctx, &labels, 500.0);
        assert!(wide.iter().all(|r| r.ctrl_x == wide[0].ctrl_x), "{:?}", wide.iter().map(|r| r.ctrl_x).collect::<Vec<_>>());
        assert!(!ctx.has_requested_repaint(), "idle list rows requested a repaint");
    }

    #[test]
    fn a_drop_slot_is_the_row_whose_middle_is_below_the_pointer() {
        let rects = [Rect::from_min_size(Pos2::new(0.0, 0.0), vec2(100.0, 22.0)), Rect::from_min_size(Pos2::new(0.0, 22.0), vec2(100.0, 22.0))];
        let slot = |y: f32| rects.iter().position(|r| y < r.center().y).unwrap_or(2);
        assert_eq!((slot(3.0), slot(15.0), slot(30.0), slot(60.0)), (0, 1, 1, 2));
    }
}
