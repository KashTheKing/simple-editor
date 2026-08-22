//! Tool bar: a thin, movable strip that sits between the viewport and the timeline (its own dockable
//! pane, so it can also be popped out). One row of icon buttons:
//!   Select (V) · Text (T) · Rectangle · Ellipse · Triangle · Polygon · Star · Line · Arrow · Draw (D) ·
//!   Mask (rect/ellipse/polygon/path) · Zoom
//! plus, for shape tools, fill and stroke colour buttons and a stroke-width DragValue; for Draw, a
//! play/record button, the brush colour/width, the playback speed (0.5x / 1x / 2x) and a page toggle.
//!
//! The active tool changes what a click-drag in the Preview does (see `ui::preview`): Select edits the
//! selected clip, a shape tool drags out a new Shape clip at the playhead, Draw records strokes while the
//! mouse is down (timed, so the sketch replays), Mask edits the selected effect's / clip's mask.

use crate::model::{MaskShape, ShapeKind};
use crate::theme::Palette;
use eframe::egui;
use egui::{Align2, Color32, CornerRadius, FontId, Key, Modifiers, Sense, Stroke, StrokeKind};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Tool {
    #[default]
    Select,
    Text,
    Shape(ShapeKind),
    Draw,
    Mask(MaskShape),
    Zoom,
    /// Razor: click a clip in the timeline to split it there.
    Cut,
    /// Click the timeline to drop a project marker.
    Marker,
    /// Drag a clip edge to change its speed instead of trimming it.
    Stretch,
    /// Drag the timeline lanes to open (or close) a gap from the press time onward.
    Spacer,
}

pub struct ToolsState {
    pub tool: Tool,
    /// Style applied to the next shape drawn.
    pub fill: [u8; 4],
    pub stroke: [u8; 4],
    pub stroke_width: f32,
    pub sides: u32,
    pub corner: f32,
    /// Draw tool: brush and recording rate.
    pub brush: [u8; 4],
    pub brush_width: f32,
    pub draw_rate: f32,
    pub page: [u8; 4],
    /// Draw tool: a take is running — the app plays the video and drops every stroke into one drawing
    /// until this goes back off (see `App::toggle_draw_recording`).
    pub recording: bool,
}

impl Default for ToolsState {
    fn default() -> Self {
        Self {
            tool: Tool::Select,
            fill: [255, 255, 255, 255],
            stroke: [0, 0, 0, 0],
            stroke_width: 4.0,
            sides: 5,
            corner: 0.0,
            brush: [255, 80, 80, 255],
            brush_width: 6.0,
            draw_rate: 1.0,
            page: [0, 0, 0, 0],
            recording: false,
        }
    }
}

/// The strip, left to right: (tool, icon, name). `Tool::Mask` stands for whichever mask shape is
/// selected (the combo next to it picks one), the shape tools each get their own button.
/// What `icon_button` draws. Painted with the painter, not typed: several of the obvious characters
/// (polygon, pencil, half-disc, magnifier) are missing from Segoe UI and egui's bundled fonts and came
/// out as tofu boxes, which made half the strip look identical.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Glyph {
    Cursor,
    Letter(char),
    Rect,
    Ellipse,
    Poly(u32),
    Star,
    Line,
    Arrow,
    Pencil,
    Mask,
    Zoom,
    Razor,
    Flag,
    Hourglass,
    Eye,
    EyeOff,
    Diamond,
    Record,
    Mic,
    Headphone,
    SpeakerOn,
    SpeakerOff,
    Camera,
    FilmStrip,
    /// Two patch boxes joined by a cable — a node graph.
    Nodes,
    /// A stack of sheets — an adjustment layer.
    Layers,
    /// Two posts with a double-headed arrow between them — a gap being widened.
    Spacer,
}

const STRIP: [(Tool, Glyph, &str); 16] = [
    (Tool::Select, Glyph::Cursor, "Select"),
    (Tool::Cut, Glyph::Razor, "Cut"),
    (Tool::Marker, Glyph::Flag, "Marker"),
    (Tool::Stretch, Glyph::Hourglass, "Stretch"),
    (Tool::Spacer, Glyph::Spacer, "Spacer"),
    (Tool::Text, Glyph::Letter('T'), "Text"),
    (Tool::Shape(ShapeKind::Rect), Glyph::Rect, "Rectangle"),
    (Tool::Shape(ShapeKind::Ellipse), Glyph::Ellipse, "Ellipse"),
    (Tool::Shape(ShapeKind::Triangle), Glyph::Poly(3), "Triangle"),
    (Tool::Shape(ShapeKind::Polygon), Glyph::Poly(5), "Polygon"),
    (Tool::Shape(ShapeKind::Star), Glyph::Star, "Star"),
    (Tool::Shape(ShapeKind::Line), Glyph::Line, "Line"),
    (Tool::Shape(ShapeKind::Arrow), Glyph::Arrow, "Arrow"),
    (Tool::Draw, Glyph::Pencil, "Draw"),
    (Tool::Mask(MaskShape::Rect), Glyph::Mask, "Mask"),
    (Tool::Zoom, Glyph::Zoom, "Zoom"),
];

/// Single-key shortcut of a tool (used for the tooltips and by `handle_hotkeys`).
pub fn tool_hotkey(tool: Tool) -> Option<Key> {
    match tool {
        Tool::Select => Some(Key::V),
        Tool::Text => Some(Key::T),
        Tool::Draw => Some(Key::D),
        Tool::Shape(_) => Some(Key::S),
        Tool::Mask(_) => Some(Key::M),
        Tool::Cut => Some(Key::C),
        Tool::Stretch => Some(Key::R),
        Tool::Zoom | Tool::Marker | Tool::Spacer => None,
    }
}

/// V / T / S / D / M, ignored while a text field has focus or any modifier is held. Pressing S or M
/// again steps through the shape / mask variants. Returns the new tool when it changed; the key is
/// consumed, so calling this twice in a frame is harmless.
pub fn handle_hotkeys(ctx: &egui::Context, state: &mut ToolsState) -> Option<Tool> {
    if ctx.wants_keyboard_input() {
        return None;
    }
    let next = ctx.input_mut(|i| {
        if !i.modifiers.is_none() {
            return None;
        }
        if i.consume_key(Modifiers::NONE, Key::V) {
            Some(Tool::Select)
        } else if i.consume_key(Modifiers::NONE, Key::T) {
            Some(Tool::Text)
        } else if i.consume_key(Modifiers::NONE, Key::D) {
            Some(Tool::Draw)
        } else if i.consume_key(Modifiers::NONE, Key::S) {
            Some(Tool::Shape(next_shape(state.tool)))
        } else if i.consume_key(Modifiers::NONE, Key::M) {
            Some(Tool::Mask(next_mask(state.tool)))
        } else if i.consume_key(Modifiers::NONE, Key::C) {
            Some(Tool::Cut)
        } else if i.consume_key(Modifiers::NONE, Key::R) {
            Some(Tool::Stretch)
        } else {
            None
        }
    })?;
    (next != state.tool).then(|| {
        state.tool = next;
        next
    })
}

/// The shape tools in strip order (Draw has its own key, so it is not part of the S cycle).
const SHAPE_CYCLE: [ShapeKind; 7] = [
    ShapeKind::Rect,
    ShapeKind::Ellipse,
    ShapeKind::Triangle,
    ShapeKind::Polygon,
    ShapeKind::Star,
    ShapeKind::Line,
    ShapeKind::Arrow,
];

fn next_shape(cur: Tool) -> ShapeKind {
    match cur {
        Tool::Shape(k) => match SHAPE_CYCLE.iter().position(|c| *c == k) {
            Some(i) => SHAPE_CYCLE[(i + 1) % SHAPE_CYCLE.len()],
            None => SHAPE_CYCLE[0],
        },
        _ => SHAPE_CYCLE[0],
    }
}

fn next_mask(cur: Tool) -> MaskShape {
    match cur {
        Tool::Mask(m) => match MaskShape::ALL.iter().position(|c| *c == m) {
            Some(i) => MaskShape::ALL[(i + 1) % MaskShape::ALL.len()],
            None => MaskShape::ALL[0],
        },
        _ => MaskShape::ALL[0],
    }
}

/// Returns true when the tool or its style changed (the app may want to repaint the preview overlay).
pub fn show(ui: &mut egui::Ui, state: &mut ToolsState, palette: &Palette) -> bool {
    let mut changed = handle_hotkeys(ui.ctx(), state).is_some();
    let base = ui.id();
    // wrapped: at a small pane width the strip must fold onto a second row, not clip its last buttons
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (tool, icon, name) in STRIP {
            let active = same_tool(tool, state.tool);
            let tip = match tool_hotkey(tool) {
                Some(k) => format!("{name} ({})", k.name()),
                // the spacer's gesture is not readable from its picture
                None if tool == Tool::Spacer => format!("{name} — drag the lanes to open or close a gap"),
                None => name.to_string(),
            };
            if icon_button(ui, palette, base.with(("tool", name)), icon, &tip, active).clicked() && !active {
                state.tool = tool;
                changed = true;
            }
        }
        ui.separator();
        changed |= style_controls(ui, state, palette);
    });
    changed
}

/// The Mask button lights up for every mask shape (the combo beside it picks one); everything else is
/// an exact match.
fn same_tool(entry: Tool, cur: Tool) -> bool {
    matches!((entry, cur), (Tool::Mask(_), Tool::Mask(_))) || entry == cur
}

/// Style controls for the active tool. Returns true when anything changed.
fn style_controls(ui: &mut egui::Ui, state: &mut ToolsState, palette: &Palette) -> bool {
    let mut changed = false;
    match state.tool {
        Tool::Shape(kind) => {
            let outline_only = matches!(kind, ShapeKind::Line | ShapeKind::Arrow);
            if !outline_only {
                ui.label("Fill");
                changed |= ui.color_edit_button_srgba_unmultiplied(&mut state.fill).changed();
            }
            ui.label("Stroke");
            changed |= ui.color_edit_button_srgba_unmultiplied(&mut state.stroke).changed();
            changed |= ui
                .add(egui::DragValue::new(&mut state.stroke_width).speed(0.2).range(0.0..=200.0))
                .on_hover_text("Stroke width (px)")
                .changed();
            match kind {
                ShapeKind::Polygon | ShapeKind::Star => {
                    ui.label("Sides");
                    changed |= ui.add(egui::DragValue::new(&mut state.sides).range(3..=64)).changed();
                }
                ShapeKind::Rect => {
                    ui.label("Corner");
                    changed |= ui.add(egui::DragValue::new(&mut state.corner).speed(0.5).range(0.0..=2000.0)).changed();
                }
                ShapeKind::Arrow => {
                    ui.label("Head");
                    changed |= ui
                        .add(egui::DragValue::new(&mut state.corner).speed(0.5).range(0.0..=2000.0))
                        .on_hover_text("Arrow head size (px, 0 = follow the stroke width)")
                        .changed();
                }
                _ => {}
            }
        }
        Tool::Draw => {
            // play + record: the app starts the video and keeps every stroke of the take in one drawing
            let rec = state.recording;
            let tip = "Play and record: every stroke joins one drawing until the video stops";
            if icon_button(ui, palette, ui.id().with("draw-rec"), Glyph::Record, tip, rec).clicked() {
                state.recording = !rec;
                changed = true;
            }
            ui.label("Brush");
            changed |= ui.color_edit_button_srgba_unmultiplied(&mut state.brush).changed();
            changed |= ui
                .add(egui::DragValue::new(&mut state.brush_width).speed(0.2).range(0.5..=200.0))
                .on_hover_text("Brush width (px)")
                .changed();
            ui.label("Speed");
            for (rate, label) in [(0.5, "0.5x"), (1.0, "1x"), (2.0, "2x"), (0.0, "all")] {
                let on = (state.draw_rate - rate).abs() < 1e-3;
                if ui
                    .selectable_label(on, label)
                    .on_hover_text(if rate == 0.0 {
                        "Show the whole sketch at once"
                    } else {
                        "Playback speed of the recorded drawing"
                    })
                    .clicked()
                    && !on
                {
                    state.draw_rate = rate;
                    changed = true;
                }
            }
            let mut page = state.page[3] > 0;
            if ui.checkbox(&mut page, "Page").changed() {
                state.page[3] = if page { 255 } else { 0 };
                if page && state.page[..3] == [0, 0, 0] {
                    state.page = [255, 255, 255, 255];
                }
                changed = true;
            }
            if page {
                changed |= ui.color_edit_button_srgba_unmultiplied(&mut state.page).changed();
            }
        }
        Tool::Mask(shape) => {
            ui.label("Mask");
            let mut pick = shape;
            egui::ComboBox::from_id_salt("tool-mask-shape").selected_text(shape.name()).width(90.0).show_ui(ui, |ui| {
                for m in MaskShape::ALL {
                    ui.selectable_value(&mut pick, m, m.name());
                }
            });
            if pick != shape {
                state.tool = Tool::Mask(pick);
                changed = true;
            }
        }
        Tool::Text | Tool::Select | Tool::Zoom | Tool::Cut | Tool::Marker | Tool::Stretch | Tool::Spacer => {}
    }
    changed
}

/// A button showing `icon` and then `text`. Used wherever a control needs a real-world picture rather
/// than a symbol character (half of those render as tofu boxes in Segoe UI).
pub(crate) fn glyph_text_button(ui: &mut egui::Ui, icon: Glyph, text: &str) -> egui::Response {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, Color32::PLACEHOLDER);
    let pad = ui.spacing().button_padding;
    let icon_w = 18.0;
    let size = egui::vec2(icon_w + galley.size().x + pad.x * 2.0, galley.size().y.max(20.0) + pad.y * 2.0);
    let (rect, r) = ui.allocate_exact_size(size, Sense::click());
    let v = ui.style().interact(&r);
    ui.painter().rect(rect, v.corner_radius, v.weak_bg_fill, v.bg_stroke, StrokeKind::Inside);
    let icon_rect =
        egui::Rect::from_min_size(egui::pos2(rect.left() + pad.x, rect.top()), egui::vec2(icon_w, rect.height()));
    draw_glyph(ui.painter(), icon_rect, icon, v.text_color());
    let tp = egui::pos2(icon_rect.right(), rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(tp, galley, v.text_color());
    r
}

/// Bare square icon button: accent fill when active, a hover outline otherwise.
pub(crate) fn icon_button(
    ui: &mut egui::Ui,
    palette: &Palette,
    id: egui::Id,
    icon: Glyph,
    tip: &str,
    active: bool,
) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(24.0, 22.0), Sense::hover());
    let r = ui.interact(rect, id, Sense::click());
    let p = ui.painter();
    if active {
        p.rect_filled(rect, CornerRadius::same(2), palette.accent);
    } else if r.hovered() {
        p.rect_filled(rect, CornerRadius::same(2), palette.header);
        p.rect_stroke(rect, CornerRadius::same(2), Stroke::new(1.0, palette.border), StrokeKind::Inside);
    }
    let fg = if active { on_accent(palette.accent) } else { palette.text };
    draw_glyph(p, rect, icon, fg);
    r.on_hover_text(tip)
}

/// Paint one tool glyph inside `rect` (a 24x22 button) in `fg`.
pub(crate) fn draw_glyph(p: &egui::Painter, rect: egui::Rect, g: Glyph, fg: Color32) {
    let c = rect.center();
    let r = 6.0; // half-extent of the drawn icon
    let stroke = Stroke::new(1.4, fg);
    let poly = |n: u32, rot: f32, rad: f32| -> Vec<egui::Pos2> {
        (0..n)
            .map(|i| {
                let a = rot + std::f32::consts::TAU * i as f32 / n as f32;
                c + egui::vec2(a.cos() * rad, a.sin() * rad)
            })
            .collect()
    };
    match g {
        Glyph::Letter(ch) => {
            p.text(c, Align2::CENTER_CENTER, ch, FontId::proportional(13.0), fg);
        }
        // a classic arrow cursor
        Glyph::Cursor => {
            let pts = vec![
                c + egui::vec2(-3.5, -6.5),
                c + egui::vec2(-3.5, 5.5),
                c + egui::vec2(-0.5, 2.5),
                c + egui::vec2(1.5, 6.5),
                c + egui::vec2(3.5, 5.5),
                c + egui::vec2(1.5, 1.8),
                c + egui::vec2(5.0, 1.5),
            ];
            p.add(egui::Shape::convex_polygon(pts, fg, Stroke::NONE));
        }
        Glyph::Rect => {
            p.rect_stroke(
                egui::Rect::from_center_size(c, egui::vec2(12.0, 9.0)),
                CornerRadius::ZERO,
                stroke,
                StrokeKind::Inside,
            );
        }
        Glyph::Ellipse => {
            p.circle_stroke(c, r, stroke);
        }
        Glyph::Poly(n) => {
            let pts = poly(n, -std::f32::consts::FRAC_PI_2, r);
            p.add(egui::Shape::closed_line(pts, stroke));
        }
        Glyph::Star => {
            let outer = poly(5, -std::f32::consts::FRAC_PI_2, r);
            let inner = poly(5, -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU / 10.0, r * 0.45);
            let mut pts = Vec::with_capacity(10);
            for i in 0..5 {
                pts.push(outer[i]);
                pts.push(inner[i]);
            }
            p.add(egui::Shape::closed_line(pts, stroke));
        }
        Glyph::Line => {
            p.line_segment([c + egui::vec2(-6.0, 5.0), c + egui::vec2(6.0, -5.0)], stroke);
        }
        Glyph::Arrow => {
            p.line_segment([c + egui::vec2(-6.0, 0.0), c + egui::vec2(4.0, 0.0)], stroke);
            let head = vec![c + egui::vec2(6.5, 0.0), c + egui::vec2(2.5, -3.2), c + egui::vec2(2.5, 3.2)];
            p.add(egui::Shape::convex_polygon(head, fg, Stroke::NONE));
        }
        // pencil: a slanted body with a tip at the bottom-left
        Glyph::Pencil => {
            let body = vec![
                c + egui::vec2(1.0, -6.0),
                c + egui::vec2(5.5, -1.5),
                c + egui::vec2(-2.0, 6.0),
                c + egui::vec2(-6.0, 6.5),
                c + egui::vec2(-5.5, 2.5),
            ];
            p.add(egui::Shape::convex_polygon(body, fg, Stroke::NONE));
        }
        // mask: a circle whose left half is filled (matte in / matte out)
        Glyph::Mask => {
            p.circle_stroke(c, r, stroke);
            let half: Vec<egui::Pos2> = (0..=12)
                .map(|i| {
                    let a = std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * i as f32 / 12.0;
                    c + egui::vec2(a.cos() * r, a.sin() * r)
                })
                .collect();
            p.add(egui::Shape::convex_polygon(half, fg, Stroke::NONE));
        }
        Glyph::Zoom => {
            p.circle_stroke(c + egui::vec2(-1.0, -1.0), 4.5, stroke);
            p.line_segment([c + egui::vec2(2.2, 2.2), c + egui::vec2(6.0, 6.0)], Stroke::new(1.8, fg));
        }
        // razor blade: a rectangular blade with a toothed cutting edge along the bottom
        Glyph::Razor => {
            let body = egui::Rect::from_center_size(c + egui::vec2(0.0, -2.0), egui::vec2(12.0, 6.0));
            p.rect_stroke(body, CornerRadius::same(1), stroke, StrokeKind::Inside);
            p.line_segment([c + egui::vec2(-6.0, 1.5), c + egui::vec2(6.0, 1.5)], Stroke::new(2.0, fg));
            for i in 0..4 {
                let x = -4.5 + i as f32 * 3.0;
                p.line_segment([c + egui::vec2(x, 1.5), c + egui::vec2(x, 5.0)], Stroke::new(1.0, fg));
            }
        }
        // spacer: two posts with a double-headed arrow pushing them apart
        Glyph::Spacer => {
            for x in [-6.5, 6.5] {
                p.line_segment([c + egui::vec2(x, -6.0), c + egui::vec2(x, 6.0)], stroke);
            }
            p.line_segment([c + egui::vec2(-4.5, 0.0), c + egui::vec2(4.5, 0.0)], stroke);
            for (tip, back) in [(-5.5f32, -2.0f32), (5.5, 2.0)] {
                let head = vec![c + egui::vec2(tip, 0.0), c + egui::vec2(back, -3.0), c + egui::vec2(back, 3.0)];
                p.add(egui::Shape::convex_polygon(head, fg, Stroke::NONE));
            }
        }
        // pennant on a pole
        Glyph::Flag => {
            p.line_segment([c + egui::vec2(-4.0, -6.5), c + egui::vec2(-4.0, 6.5)], Stroke::new(1.6, fg));
            let cloth = vec![c + egui::vec2(-4.0, -6.0), c + egui::vec2(5.5, -3.0), c + egui::vec2(-4.0, 0.0)];
            p.add(egui::Shape::convex_polygon(cloth, fg, Stroke::NONE));
        }
        // hourglass: two triangles meeting at the waist, between two plates
        Glyph::Hourglass => {
            p.line_segment([c + egui::vec2(-5.0, -6.5), c + egui::vec2(5.0, -6.5)], stroke);
            p.line_segment([c + egui::vec2(-5.0, 6.5), c + egui::vec2(5.0, 6.5)], stroke);
            p.add(egui::Shape::convex_polygon(
                vec![c + egui::vec2(-4.5, -6.0), c + egui::vec2(4.5, -6.0), c],
                fg,
                Stroke::NONE,
            ));
            p.add(egui::Shape::convex_polygon(
                vec![c + egui::vec2(-4.5, 6.0), c + egui::vec2(4.5, 6.0), c],
                fg,
                Stroke::NONE,
            ));
        }
        // eye: two lids and a pupil (crossed out when hidden)
        Glyph::Eye | Glyph::EyeOff => {
            let lid = |up: f32| -> Vec<egui::Pos2> {
                (0..=16)
                    .map(|i| {
                        let x = -7.0 + i as f32 * 14.0 / 16.0;
                        let k: f32 = 1.0 - (x / 7.0) * (x / 7.0);
                        c + egui::vec2(x, up * 4.2 * k.max(0.0))
                    })
                    .collect()
            };
            p.add(egui::Shape::line(lid(-1.0), stroke));
            p.add(egui::Shape::line(lid(1.0), stroke));
            p.circle_filled(c, 2.2, fg);
            if g == Glyph::EyeOff {
                p.line_segment([c + egui::vec2(-6.5, -6.0), c + egui::vec2(6.5, 6.0)], Stroke::new(1.8, fg));
            }
        }
        Glyph::Diamond => {
            let pts = vec![
                c + egui::vec2(0.0, -5.5),
                c + egui::vec2(4.5, 0.0),
                c + egui::vec2(0.0, 5.5),
                c + egui::vec2(-4.5, 0.0),
            ];
            p.add(egui::Shape::convex_polygon(pts, fg, Stroke::NONE));
        }
        Glyph::Record => {
            p.circle_filled(c, 5.5, Color32::from_rgb(220, 60, 60));
        }
        // microphone: a capsule on a stand
        Glyph::Mic => {
            p.rect_filled(
                egui::Rect::from_center_size(c + egui::vec2(0.0, -2.5), egui::vec2(5.0, 8.0)),
                CornerRadius::same(2),
                fg,
            );
            p.line_segment([c + egui::vec2(0.0, 3.0), c + egui::vec2(0.0, 6.0)], Stroke::new(1.6, fg));
            p.line_segment([c + egui::vec2(-3.5, 6.0), c + egui::vec2(3.5, 6.0)], Stroke::new(1.6, fg));
        }
        // headphones: a headband over two ear cups
        Glyph::Headphone => {
            let band: Vec<egui::Pos2> = (0..=16)
                .map(|i| {
                    let a = std::f32::consts::PI + std::f32::consts::PI * i as f32 / 16.0;
                    c + egui::vec2(a.cos() * 6.0, a.sin() * 6.0 + 1.0)
                })
                .collect();
            p.add(egui::Shape::line(band, stroke));
            for x in [-6.0, 6.0] {
                p.rect_filled(
                    egui::Rect::from_center_size(c + egui::vec2(x, 3.0), egui::vec2(3.5, 6.0)),
                    CornerRadius::same(1),
                    fg,
                );
            }
        }
        // speaker cone, with sound waves or crossed out
        Glyph::SpeakerOn | Glyph::SpeakerOff => {
            let cone = vec![
                c + egui::vec2(-6.0, -2.0),
                c + egui::vec2(-3.0, -2.0),
                c + egui::vec2(0.5, -5.5),
                c + egui::vec2(0.5, 5.5),
                c + egui::vec2(-3.0, 2.0),
                c + egui::vec2(-6.0, 2.0),
            ];
            p.add(egui::Shape::convex_polygon(cone, fg, Stroke::NONE));
            if g == Glyph::SpeakerOn {
                for (i, rad) in [3.0_f32, 5.5].iter().enumerate() {
                    let arc: Vec<egui::Pos2> = (0..=10)
                        .map(|k| {
                            let a = -std::f32::consts::FRAC_PI_3 + 2.0 * std::f32::consts::FRAC_PI_3 * k as f32 / 10.0;
                            c + egui::vec2(1.5 + a.cos() * rad, a.sin() * rad)
                        })
                        .collect();
                    p.add(egui::Shape::line(arc, Stroke::new(1.3 - i as f32 * 0.2, fg)));
                }
            } else {
                p.line_segment([c + egui::vec2(2.5, -3.5), c + egui::vec2(6.5, 3.5)], Stroke::new(1.6, fg));
                p.line_segment([c + egui::vec2(6.5, -3.5), c + egui::vec2(2.5, 3.5)], Stroke::new(1.6, fg));
            }
        }
        // stills camera: body, lens and a viewfinder bump
        Glyph::Camera => {
            let body = egui::Rect::from_center_size(c + egui::vec2(0.0, 1.0), egui::vec2(13.0, 9.0));
            p.rect_stroke(body, CornerRadius::same(1), stroke, StrokeKind::Inside);
            p.rect_filled(
                egui::Rect::from_center_size(c + egui::vec2(-2.0, -4.5), egui::vec2(5.0, 2.5)),
                CornerRadius::same(1),
                fg,
            );
            p.circle_stroke(c + egui::vec2(0.0, 1.0), 3.0, stroke);
        }
        // film strip: a frame with sprocket holes down both edges
        Glyph::FilmStrip => {
            let body = egui::Rect::from_center_size(c, egui::vec2(13.0, 11.0));
            p.rect_stroke(body, CornerRadius::same(1), stroke, StrokeKind::Inside);
            for i in 0..3 {
                let y = -3.5 + i as f32 * 3.5;
                for x in [-4.8_f32, 4.8] {
                    p.rect_filled(
                        egui::Rect::from_center_size(c + egui::vec2(x, y), egui::vec2(2.2, 2.0)),
                        CornerRadius::ZERO,
                        fg,
                    );
                }
            }
        }
        // node graph: two patch boxes with a cable between them
        Glyph::Nodes => {
            let a = egui::Rect::from_center_size(c + egui::vec2(-3.5, -3.5), egui::vec2(7.0, 5.0));
            let b = egui::Rect::from_center_size(c + egui::vec2(3.5, 3.5), egui::vec2(7.0, 5.0));
            p.line_segment([a.right_center(), b.left_center()], stroke);
            p.rect_stroke(a, CornerRadius::same(1), stroke, StrokeKind::Inside);
            p.rect_stroke(b, CornerRadius::same(1), stroke, StrokeKind::Inside);
        }
        // adjustment layer: a stack of sheets
        Glyph::Layers => {
            for dy in [-4.0_f32, 0.0, 4.0] {
                let sheet = egui::Rect::from_center_size(c + egui::vec2(0.0, dy), egui::vec2(12.0, 3.0));
                p.rect_stroke(sheet, CornerRadius::same(1), stroke, StrokeKind::Inside);
            }
        }
    }
}

/// Readable text colour on top of the accent fill.
fn on_accent(c: Color32) -> Color32 {
    let l = 0.299 * c.r() as f32 + 0.587 * c.g() as f32 + 0.114 * c.b() as f32;
    if l > 140.0 {
        Color32::BLACK
    } else {
        Color32::WHITE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Pos2, Rect, Vec2};

    struct Harness {
        ctx: egui::Context,
        state: ToolsState,
        base: egui::Id,
        time: f64,
        changed: bool,
    }

    impl Harness {
        fn new() -> Self {
            let mut h = Self {
                ctx: egui::Context::default(),
                state: ToolsState::default(),
                base: egui::Id::NULL,
                time: 0.0,
                changed: false,
            };
            h.frame(vec![]);
            h
        }
        fn frame(&mut self, events: Vec<Event>) {
            self.time += 0.05;
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 60.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let pal = Palette::new(true, Color32::from_rgb(0, 120, 212));
            let Harness { ctx, state, base, changed, .. } = self;
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    *base = ui.id();
                    *changed = show(ui, state, &pal);
                });
            });
        }
        /// Centre of a strip button by its name, from the previous frame's layout.
        fn button(&self, name: &str) -> Pos2 {
            self.ctx
                .read_response(self.base.with(("tool", name)))
                .unwrap_or_else(|| panic!("no button {name}"))
                .rect
                .center()
        }
        fn click(&mut self, name: &str) {
            let pos = self.button(name);
            self.frame(vec![Event::PointerMoved(pos)]);
            self.frame(vec![Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            }]);
            self.frame(vec![Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            }]);
        }
        fn key(&mut self, key: Key) {
            self.frame(vec![
                Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE },
                Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE },
            ]);
        }
    }

    #[test]
    fn strip_lays_out_every_tool() {
        let h = Harness::new();
        for (_, _, name) in STRIP {
            let c = h.button(name);
            assert!(c.x > 0.0 && c.x < 900.0, "{name} off-strip at {c:?}");
        }
    }

    #[test]
    fn clicking_switches_the_tool() {
        let mut h = Harness::new();
        assert_eq!(h.state.tool, Tool::Select);
        h.click("Ellipse");
        assert_eq!(h.state.tool, Tool::Shape(ShapeKind::Ellipse));
        assert!(h.changed, "a switch is reported as a change");
        h.click("Draw");
        assert_eq!(h.state.tool, Tool::Draw);
        h.click("Zoom");
        assert_eq!(h.state.tool, Tool::Zoom);
        h.click("Select");
        assert_eq!(h.state.tool, Tool::Select);
    }

    #[test]
    fn clicking_the_active_tool_reports_nothing() {
        let mut h = Harness::new();
        h.click("Star");
        assert_eq!(h.state.tool, Tool::Shape(ShapeKind::Star));
        h.click("Star");
        assert!(!h.changed, "re-clicking the active tool is not a change");
        assert_eq!(h.state.tool, Tool::Shape(ShapeKind::Star));
    }

    #[test]
    fn mask_button_lights_up_for_every_mask_shape() {
        assert!(same_tool(Tool::Mask(MaskShape::Rect), Tool::Mask(MaskShape::Path)));
        assert!(!same_tool(Tool::Mask(MaskShape::Rect), Tool::Select));
        assert!(!same_tool(Tool::Shape(ShapeKind::Rect), Tool::Shape(ShapeKind::Star)));
        let mut h = Harness::new();
        h.state.tool = Tool::Mask(MaskShape::Path);
        h.frame(vec![]);
        h.click("Mask");
        assert_eq!(h.state.tool, Tool::Mask(MaskShape::Path), "the active mask shape survives a re-click");
        h.click("Rectangle");
        assert_eq!(h.state.tool, Tool::Shape(ShapeKind::Rect));
        h.click("Mask");
        assert_eq!(h.state.tool, Tool::Mask(MaskShape::Rect));
    }

    #[test]
    fn single_key_shortcuts_switch_tools() {
        let mut h = Harness::new();
        h.key(Key::T);
        assert_eq!(h.state.tool, Tool::Text);
        h.key(Key::D);
        assert_eq!(h.state.tool, Tool::Draw);
        h.key(Key::V);
        assert_eq!(h.state.tool, Tool::Select);
        h.key(Key::S);
        assert_eq!(h.state.tool, Tool::Shape(ShapeKind::Rect));
        h.key(Key::S);
        assert_eq!(h.state.tool, Tool::Shape(ShapeKind::Ellipse), "S steps through the shapes");
        h.key(Key::M);
        assert_eq!(h.state.tool, Tool::Mask(MaskShape::Rect));
        h.key(Key::M);
        assert_eq!(h.state.tool, Tool::Mask(MaskShape::Ellipse));
    }

    #[test]
    fn modified_keys_are_left_alone() {
        let mut h = Harness::new();
        h.frame(vec![Event::Key {
            key: Key::V,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::CTRL,
        }]);
        assert_eq!(h.state.tool, Tool::Select);
        h.state.tool = Tool::Draw;
        h.frame(vec![Event::Key {
            key: Key::S,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::CTRL,
        }]);
        assert_eq!(h.state.tool, Tool::Draw, "Ctrl+S must stay the save shortcut");
    }

    #[test]
    fn hotkeys_are_consumed_once() {
        // `show` handles the keys itself, so a second call in the same frame must be a no-op
        let mut state = ToolsState::default();
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 60.0))),
            events: vec![Event::Key {
                key: Key::S,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
            ..Default::default()
        };
        let pal = Palette::new(true, Color32::from_rgb(0, 120, 212));
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                show(ui, &mut state, &pal);
                assert!(handle_hotkeys(ui.ctx(), &mut state).is_none(), "key already consumed");
            });
        });
        assert_eq!(state.tool, Tool::Shape(ShapeKind::Rect));
    }

    #[test]
    fn tool_hotkey_covers_the_documented_keys() {
        assert_eq!(tool_hotkey(Tool::Select), Some(Key::V));
        assert_eq!(tool_hotkey(Tool::Text), Some(Key::T));
        assert_eq!(tool_hotkey(Tool::Shape(ShapeKind::Star)), Some(Key::S));
        assert_eq!(tool_hotkey(Tool::Draw), Some(Key::D));
        assert_eq!(tool_hotkey(Tool::Mask(MaskShape::Path)), Some(Key::M));
        assert_eq!(tool_hotkey(Tool::Zoom), None);
    }

    #[test]
    fn every_tool_renders_its_style_controls() {
        let mut h = Harness::new();
        let mut tools: Vec<Tool> = STRIP.iter().map(|(t, ..)| *t).collect();
        tools.extend(MaskShape::ALL.map(Tool::Mask));
        tools.push(Tool::Shape(ShapeKind::Draw));
        for t in tools {
            h.state.tool = t;
            h.frame(vec![]);
            assert_eq!(h.state.tool, t, "{t:?} controls must not change the tool on their own");
            assert!(!h.changed, "{t:?} reports no change when nothing is touched");
        }
    }
}
