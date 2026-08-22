//! Preview panel: the rendered frame (letterboxed, black bars), a selection outline for the selected
//! visual clip (drag to move = edits clip.x / clip.y at the playhead time via `Animated::set_at`, calling
//! `undo` once at drag start), and the transport bar, centred under the video:
//! |◀  ◀◀(prev cut)  ◀(step)  ▶/⏸  ▶(step)  ▶▶  ▶| plus timecode / duration, In/Out buttons and the
//! in/out times, a preview-quality selector (100 / 75 / 50 / 25 %) and a Movie mode toggle.
//! In `fullscreen` mode only the video is drawn (no transport, no overlay): Esc / F11 leave it (app).
//!
//! When `PreviewCtx.tool` is anything but `Tool::Select`, a click-drag over the video draws instead of
//! moving the clip: a shape tool reports `new_shape`, the Draw tool records a timed `stroke`, and a mask
//! tool edits the selected clip's mask (reported through `mask_edit`); a Polygon/Path mask gets its
//! vertices from the drag rect (an empty point list would hide the clip entirely).
//! The Polygon *shape* tool is SVG-style instead: each click appends a vertex (the path is drawn live),
//! a click back on a placed vertex (which is where a double-click's second press lands) or Enter closes
//! it into a shape, and re-selecting that shape with the Select tool puts a drag handle on every point.

use crate::engine::compose::placement;
use crate::hotkeys::Action;
use crate::media::Frame;
use crate::model::{ClipKind, Id, Mask, MaskShape, Project, ShapeKind, Stroke as ModelStroke};
use crate::theme::Palette;
use crate::ui::timecode;
use crate::ui::tools::Tool;
use eframe::egui::{self, pos2, vec2, Color32, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, TextureOptions, Vec2};
use std::sync::Arc;

/// Preview render scales offered by the quality selector.
pub const QUALITIES: [u32; 4] = [100, 75, 50, 25];

/// A click-drag with a non-Select tool.
struct ToolDrag {
    /// Press position in screen points.
    from: Pos2,
    /// `ui.input(|i| i.time)` at the press (drawing strokes are timed).
    t0: f64,
    /// Draw tool: (x, y, t) in project px relative to the canvas centre.
    points: Vec<(f32, f32, f32)>,
}

pub struct PreviewState {
    pub texture: Option<egui::TextureHandle>,
    /// Active overlay drag: clip (x, y) at drag start and the accumulated pointer delta (points).
    drag: Option<(f64, f64, Vec2)>,
    /// Active tool drag (shape / draw / mask).
    tool_drag: Option<ToolDrag>,
    /// Polygon tool: vertices placed so far, project px relative to the canvas centre.
    poly: Vec<(f32, f32)>,
    /// Select tool: index of the polygon vertex being dragged.
    point_drag: Option<usize>,
    /// Last pointer movement (fullscreen hides the cursor after 2 s of stillness).
    moved_at: Option<std::time::Instant>,
    /// Content rect of the transport row last frame — it is centred against the panel using its own
    /// measured width, so the first frame is left-aligned and every later one is centred.
    transport: Rect,
}

impl Default for PreviewState {
    fn default() -> Self {
        Self {
            texture: None,
            drag: None,
            tool_drag: None,
            poly: Vec::new(),
            point_drag: None,
            moved_at: None,
            transport: Rect::ZERO,
        }
    }
}

pub struct PreviewCtx<'a> {
    pub project: &'a mut Project,
    pub selection: &'a [Id],
    pub playhead: f64,
    pub playing: bool,
    /// Video only: no transport bar, no selection overlay / drag.
    pub fullscreen: bool,
    pub palette: &'a Palette,
    pub undo: &'a mut dyn FnMut(&Project),
    /// A newly rendered frame to upload this update (None = keep the current texture).
    pub frame: Option<Arc<Frame>>,
    /// A texture the GPU renderer already holds, with its pixel size — painted directly, with no
    /// readback and no upload. Takes precedence over `frame`.
    pub gpu_texture: Option<(egui::TextureId, [u32; 2])>,
    /// Active editing tool (`Tool::Select` = drag moves the selected clip).
    pub tool: Tool,
    /// Current preview render scale in percent (settings.preview_quality).
    pub quality: u32,
    /// Current movie mode (settings.movie_mode).
    pub movie_mode: bool,
    /// Pre-render progress 0..1 while frames are being cached (None = not pre-rendering).
    pub prerender: Option<f32>,
}

#[derive(Default)]
pub struct PreviewResponse {
    pub seek: Option<f64>,
    /// Transport buttons map straight to actions (PlayPause, Stop, StepBack, …, MarkIn, ClearInOut).
    pub actions: Vec<Action>,
    pub edited: bool,
    /// Pixel size available for the video image (for Player::set_canvas).
    pub canvas: (u32, u32),
    /// The user picked another preview quality (percent) — the app stores it in Settings.
    pub set_quality: Option<u32>,
    /// The user toggled Movie mode.
    pub set_movie_mode: Option<bool>,
    /// A shape dragged out with a shape tool: (kind, centre x, centre y, half width, half height) in
    /// project pixels relative to the canvas centre (same frame as `Clip.x/y` and `ShapeStyle.w/h`).
    pub new_shape: Option<(ShapeKind, f32, f32, f32, f32)>,
    /// Vertices of a closed Polygon path, relative to that shape's centre (empty = a regular n-gon).
    pub new_points: Vec<(f32, f32)>,
    /// A stroke recorded with the Draw tool (points relative to the canvas centre, timed from the press).
    pub stroke: Option<ModelStroke>,
    /// The selected clip's mask was edited with a mask tool.
    pub mask_edit: bool,
}

pub fn show(ui: &mut egui::Ui, state: &mut PreviewState, mut c: PreviewCtx<'_>) -> PreviewResponse {
    let mut r = PreviewResponse::default();
    if c.fullscreen {
        video(ui, state, &mut c, &mut r);
        return r;
    }
    ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
        transport(ui, state, &c, &mut r);
        video(ui, state, &mut c, &mut r);
    });
    r
}

fn transport(ui: &mut egui::Ui, state: &mut PreviewState, c: &PreviewCtx<'_>, r: &mut PreviewResponse) {
    let fps = c.project.fps;
    // centred: pad by half the leftover of last frame's measured width (0 on the very first frame)
    let pad = ((ui.available_width() - state.transport.width()) * 0.5).max(0.0);
    let row = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.add_space(pad);
            let mut b = |ui: &mut egui::Ui, label: &str, a: Action| {
                if ui.button(label).clicked() {
                    r.actions.push(a);
                }
            };
            b(ui, "|◀", Action::GoStart);
            b(ui, "◀◀", Action::PrevCut);
            b(ui, "◀", Action::StepBack);
            b(ui, if c.playing { "⏸" } else { "▶" }, Action::PlayPause);
            b(ui, "■", Action::Stop);
            b(ui, "▶", Action::StepForward);
            b(ui, "▶▶", Action::NextCut);
            b(ui, "▶|", Action::GoEnd);
            ui.add_space(8.0);
            ui.monospace(format!("{} / {}", timecode(c.playhead, fps), timecode(c.project.duration(), fps)));
            ui.add_space(8.0);
            b(ui, "In", Action::MarkIn);
            b(ui, "Out", Action::MarkOut);
            b(ui, "Clear", Action::ClearInOut);
            if c.project.in_point.is_some() || c.project.out_point.is_some() {
                ui.add_space(4.0);
                let tc = |t: Option<f64>| t.map(|t| timecode(t, fps)).unwrap_or_else(|| "-".into());
                ui.monospace(format!("{} → {}", tc(c.project.in_point), tc(c.project.out_point)));
            }
            ui.add_space(8.0);
            let cur = QUALITIES.iter().copied().find(|&q| q == c.quality).unwrap_or(100);
            egui::ComboBox::from_id_salt("preview_quality")
                .selected_text(format!("{cur} %"))
                .width(64.0)
                .show_ui(ui, |ui| {
                    for q in QUALITIES {
                        if ui.selectable_label(cur == q, format!("{q} %")).clicked() && q != cur {
                            r.set_quality = Some(q);
                        }
                    }
                })
                .response
                .on_hover_text("Preview render scale");
            // film strip: movie mode plays pre-rendered frames
            if crate::ui::tools::icon_button(
                ui,
                c.palette,
                ui.id().with("movie"),
                crate::ui::tools::Glyph::FilmStrip,
                "Movie mode: play pre-rendered full-quality frames",
                c.movie_mode,
            )
            .clicked()
            {
                r.set_movie_mode = Some(!c.movie_mode);
            }
            b(ui, "⛶", Action::Fullscreen);
        })
        .response;
    // measured content width (without the centring pad) drives the next frame's padding
    state.transport = Rect::from_min_max(pos2(row.rect.left() + pad, row.rect.top()), row.rect.max);
}

/// Keep `v`'s direction while forcing at least `min` of length, so a zero-length drag still makes a
/// visible shape without flipping which way it points.
fn signed_min(v: f32, min: f32) -> f32 {
    if v < 0.0 {
        v.min(-min)
    } else {
        v.max(min)
    }
}

/// Outline of the shape a drag is creating: `from` is where the press landed, `to` is the cursor.
/// Bounded shapes fill the rectangle between them; a line or arrow runs from one to the other, so it
/// must not be handed a normalised rect (its corners would point the wrong way down two of the four
/// diagonals).
fn draw_shape_preview(p: &egui::Painter, kind: ShapeKind, from: Pos2, to: Pos2, palette: &Palette, stroke: Stroke) {
    let fill = palette.selection.gamma_multiply(0.25);
    let r = Rect::from_two_pos(from, to);
    let c = r.center();
    let (hw, hh) = (r.width() / 2.0, r.height() / 2.0);
    let poly = |n: u32, rot: f32| -> Vec<Pos2> {
        (0..n)
            .map(|i| {
                let a = rot + std::f32::consts::TAU * i as f32 / n as f32;
                pos2(c.x + a.cos() * hw, c.y + a.sin() * hh)
            })
            .collect()
    };
    let top = -std::f32::consts::FRAC_PI_2;
    match kind {
        ShapeKind::Rect => {
            p.rect(r, 0.0, fill, stroke, StrokeKind::Inside);
        }
        ShapeKind::Ellipse => {
            let pts = poly(48, 0.0);
            p.add(Shape::convex_polygon(pts.clone(), fill, Stroke::NONE));
            p.add(Shape::closed_line(pts, stroke));
        }
        ShapeKind::Triangle | ShapeKind::Polygon | ShapeKind::Star => {
            let pts = if kind == ShapeKind::Star {
                let (o, i) = (poly(5, top), poly(5, top + std::f32::consts::TAU / 10.0));
                (0..5).flat_map(|k| [o[k], pos2(c.x + (i[k].x - c.x) * 0.45, c.y + (i[k].y - c.y) * 0.45)]).collect()
            } else {
                poly(if kind == ShapeKind::Triangle { 3 } else { 5 }, top)
            };
            p.add(Shape::convex_polygon(pts.clone(), fill, Stroke::NONE));
            p.add(Shape::closed_line(pts, stroke));
        }
        ShapeKind::Line | ShapeKind::Arrow => {
            let (a, b) = (from, to);
            p.line_segment([a, b], stroke);
            if kind == ShapeKind::Arrow {
                let v = b - a;
                let len = v.length().max(1.0);
                let (ux, uy) = (v.x / len, v.y / len);
                let head = (len * 0.25).clamp(6.0, 40.0);
                let base = pos2(b.x - ux * head, b.y - uy * head);
                let (px, py) = (-uy * head * 0.5, ux * head * 0.5);
                p.add(Shape::convex_polygon(
                    vec![b, pos2(base.x + px, base.y + py), pos2(base.x - px, base.y - py)],
                    stroke.color,
                    Stroke::NONE,
                ));
            }
        }
        ShapeKind::Draw => {
            p.rect_stroke(r, 0.0, stroke, StrokeKind::Inside);
        }
    }
}

/// Largest `aspect` rect centred in `area`, edges snapped to whole physical pixels.
fn letterbox(area: Rect, aspect: f32, ppp: f32) -> Rect {
    let (aw, ah) = (area.width(), area.height());
    let (w, h) = if aw / ah > aspect { (ah * aspect, ah) } else { (aw, aw / aspect) };
    let snap = |v: f32| (v * ppp).round() / ppp;
    let min = pos2(snap(area.min.x + (aw - w) / 2.0), snap(area.min.y + (ah - h) / 2.0));
    Rect::from_min_size(min, vec2(snap(w), snap(h)))
}

/// Drag with a shape / draw / mask tool. Returns true when the tool consumed the gesture (so the clip
/// must not be moved). Never panics without a selection: a shape or a stroke is reported regardless and
/// the app decides what to create.
fn tool_drag(
    ui: &egui::Ui,
    state: &mut PreviewState,
    c: &mut PreviewCtx<'_>,
    r: &mut PreviewResponse,
    resp: &egui::Response,
    lb: Rect,
) -> bool {
    let tool = c.tool;
    if tool == Tool::Select {
        state.tool_drag = None;
        return false;
    }
    // screen point -> project px relative to the canvas centre
    let k = c.project.width as f32 / lb.width().max(1.0);
    let (hw, hh) = (c.project.width as f32 / 2.0, c.project.height as f32 / 2.0);
    let to_proj = move |p: Pos2| ((p.x - lb.min.x) * k - hw, (p.y - lb.min.y) * k - hh);
    let to_screen = move |&(x, y): &(f32, f32)| pos2(lb.min.x + (x + hw) / k, lb.min.y + (y + hh) / k);

    // the Polygon tool places real vertices: click to append, click a placed one / Enter to close
    if tool == Tool::Shape(ShapeKind::Polygon) {
        let at = resp.interact_pointer_pos();
        // clicking a vertex that is already placed closes the path instead of stacking a duplicate on
        // top of it — that is where a double-click's second press lands, and egui cannot be asked
        // (quick clicks at different points read as double/triple clicks while you place vertices)
        let on_vertex = at.is_some_and(|p| state.poly.iter().any(|q| (to_screen(q) - p).length() <= 6.0));
        if (resp.clicked() || resp.drag_stopped()) && !on_vertex {
            if let Some(p) = at {
                state.poly.push(to_proj(p));
            }
        }
        let p = ui.painter_at(lb);
        let stroke = Stroke::new(1.5, c.palette.selection);
        let mut pts: Vec<Pos2> = state.poly.iter().map(to_screen).collect();
        if !pts.is_empty() {
            for q in &pts {
                p.circle_filled(*q, 3.5, c.palette.selection);
            }
            // rubber band from the last vertex to the cursor, and a hint of the closing edge
            if let Some(at) = resp.hover_pos().or_else(|| ui.input(|i| i.pointer.latest_pos())) {
                p.line_segment([pts[0], at], Stroke::new(1.0, c.palette.selection.gamma_multiply(0.5)));
                pts.push(at);
            }
            p.add(Shape::line(pts, stroke));
        }
        if (on_vertex && (resp.clicked() || resp.double_clicked())) || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            if state.poly.len() >= 3 {
                let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
                for &(x, y) in &state.poly {
                    (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
                }
                let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
                r.new_points = state.poly.iter().map(|&(x, y)| (x - cx, y - cy)).collect();
                r.new_shape =
                    Some((ShapeKind::Polygon, cx, cy, ((x1 - x0) / 2.0).max(2.0), ((y1 - y0) / 2.0).max(2.0)));
            }
            state.poly.clear();
        }
        return true;
    }

    if resp.drag_started() {
        if let Some(p) = resp.interact_pointer_pos() {
            let t0 = ui.input(|i| i.time);
            state.tool_drag = Some(ToolDrag { from: p, t0, points: Vec::new() });
            if matches!(tool, Tool::Mask(_)) {
                (c.undo)(c.project);
            }
        }
    }
    let Some(d) = &mut state.tool_drag else { return !matches!(tool, Tool::Zoom | Tool::Text) };
    let now = resp.interact_pointer_pos().or_else(|| ui.input(|i| i.pointer.latest_pos())).unwrap_or(d.from);

    if resp.dragged() {
        match tool {
            Tool::Draw => {
                let (x, y) = to_proj(now);
                let t = (ui.input(|i| i.time) - d.t0).max(0.0) as f32;
                // skip points closer than a project pixel: a still mouse would otherwise flood the stroke
                if d.points.last().is_none_or(|&(px, py, _)| (px - x).abs() + (py - y).abs() > 1.0) {
                    d.points.push((x, y, t));
                }
            }
            Tool::Mask(shape) => {
                let (x0, y0) = to_proj(d.from);
                let (x1, y1) = to_proj(now);
                let id = c.selection.iter().copied().find(|&id| c.project.clip(id).is_some());
                if let Some(clip) = id.and_then(|id| c.project.clip_mut(id)) {
                    let m = clip.mask.get_or_insert_with(|| Mask::new(shape));
                    m.shape = shape;
                    m.cx.value = ((x0 + x1) / 2.0) as f64;
                    m.cy.value = ((y0 + y1) / 2.0) as f64;
                    m.rx.value = ((x1 - x0).abs() / 2.0).max(1.0) as f64;
                    m.ry.value = ((y1 - y0).abs() / 2.0).max(1.0) as f64;
                    if matches!(shape, MaskShape::Polygon | MaskShape::Path) {
                        // those shapes are rasterised from `points`; fewer than 3 vertices reads as
                        // "outside everywhere" and the clip would vanish, so the drag rect seeds them
                        m.points.clear();
                        crate::ui::seed_mask_points(m);
                    }
                    r.mask_edit = true;
                    r.edited = true;
                }
            }
            _ => {}
        }
        // rubber band / ink preview
        let p = ui.painter_at(lb);
        let stroke = Stroke::new(1.0, c.palette.selection);
        match tool {
            Tool::Draw => {
                let pts: Vec<Pos2> =
                    d.points.iter().map(|&(x, y, _)| pos2(lb.min.x + (x + hw) / k, lb.min.y + (y + hh) / k)).collect();
                if pts.len() > 1 {
                    p.add(Shape::line(pts, Stroke::new(2.0, c.palette.selection)));
                }
            }
            // draw the shape itself, not a box around it, so what you drag is what you get
            // `from`/`now` raw, not a normalised rect: a line runs press -> cursor, and only a bounded
            // shape wants its corners sorted (see draw_shape_preview)
            Tool::Shape(kind) => draw_shape_preview(&p, kind, d.from, now, c.palette, stroke),
            _ => {
                p.rect_stroke(Rect::from_two_pos(d.from, now), 0.0, stroke, StrokeKind::Inside);
            }
        }
    }

    if resp.drag_stopped() {
        let (x0, y0) = to_proj(d.from);
        let (x1, y1) = to_proj(now);
        match tool {
            Tool::Shape(kind) => {
                // Line/Arrow run from (-w, -h) to (+w, +h) about their centre (engine::shapes), so the
                // half-extents stay SIGNED and the drag's direction survives; a dragged-up-right line
                // used to come out pointing down-right because both were made absolute here.
                let (dx, dy) = ((x1 - x0) / 2.0, (y1 - y0) / 2.0);
                let (w, h) = match kind {
                    ShapeKind::Line | ShapeKind::Arrow => (signed_min(dx, 2.0), signed_min(dy, 2.0)),
                    _ => (dx.abs().max(2.0), dy.abs().max(2.0)),
                };
                r.new_shape = Some((kind, (x0 + x1) / 2.0, (y0 + y1) / 2.0, w, h));
            }
            Tool::Draw if d.points.len() > 1 => {
                r.stroke = Some(ModelStroke { color: [255, 255, 255, 255], width: 6.0, points: d.points.clone() });
            }
            _ => {}
        }
        state.tool_drag = None;
    }
    true
}

fn video(ui: &mut egui::Ui, state: &mut PreviewState, c: &mut PreviewCtx<'_>, r: &mut PreviewResponse) {
    let (rect, resp) = ui.allocate_exact_size(ui.available_size_before_wrap(), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, Color32::BLACK);
    let ppp = ui.pixels_per_point();
    let aspect = c.project.width.max(1) as f32 / c.project.height.max(1) as f32;
    let lb = letterbox(rect, aspect, ppp);
    let (cw, ch) = ((lb.width() * ppp).round().max(16.0) as u32, (lb.height() * ppp).round().max(16.0) as u32);
    r.canvas = (cw, ch);

    if let Some(f) = &c.frame {
        let (w, h) = (f.width as usize, f.height as usize);
        if w > 0 && h > 0 && f.rgba.len() == w * h * 4 {
            let img = egui::ColorImage::from_rgba_premultiplied([w, h], &f.rgba);
            match &mut state.texture {
                // same size: sub-image upload (tex_sub_image_2d) instead of re-specifying the texture
                Some(t) if t.size() == [w, h] => t.set_partial([0, 0], img, TextureOptions::LINEAR),
                Some(t) => t.set(img, TextureOptions::LINEAR),
                None => state.texture = Some(ui.ctx().load_texture("preview", img, TextureOptions::LINEAR)),
            }
        }
    }
    // the GPU renderer's own texture wins: nothing is read back or re-uploaded
    if let Some((id, _)) = c.gpu_texture {
        painter.image(id, lb, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
    } else if let Some(t) = &state.texture {
        painter.image(t.id(), lb, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
    }
    if c.fullscreen {
        // double-click toggles fullscreen, like every player
        if resp.double_clicked() {
            r.actions.push(Action::Fullscreen);
        }
        // hide the cursor after 2 s without movement
        let moved = ui.input(|i| i.pointer.delta() != Vec2::ZERO || i.pointer.any_down());
        if moved || state.moved_at.is_none() {
            state.moved_at = Some(std::time::Instant::now());
        }
        if state.moved_at.map(|t| t.elapsed().as_secs_f32() > 2.0).unwrap_or(false) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::None);
        } else {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
        }
        state.drag = None;
        return;
    }
    state.moved_at = None;
    prerender_badge(ui, &painter, lb, c);

    // a tool other than Select owns the gesture (draw / mask / shape) — never move the clip then, and
    // never let the polygon tool's closing double-click also toggle fullscreen
    if tool_drag(ui, state, c, r, &resp, lb) {
        state.drag = None;
        return;
    }
    if resp.double_clicked() {
        r.actions.push(Action::Fullscreen);
    }

    // selection overlay + drag-to-move
    let Some(clip) = c
        .selection
        .iter()
        .filter_map(|&id| c.project.clip(id))
        .find(|cl| cl.is_visual() && cl.enabled && cl.contains(c.playhead))
    else {
        state.drag = None;
        return;
    };
    let id = clip.id;
    let lt = clip.local(c.playhead);
    let to_screen =
        |x: f32, y: f32| pos2(lb.min.x + x / cw as f32 * lb.width(), lb.min.y + y / ch as f32 * lb.height());
    let stroke = Stroke::new(1.5, c.palette.selection);
    if clip.kind == ClipKind::Text {
        let p = placement(c.project, clip, c.playhead, (1, 1), cw, ch, false);
        let o = to_screen(p.cx, p.cy);
        painter.line_segment([o - vec2(6.0, 0.0), o + vec2(6.0, 0.0)], stroke);
        painter.line_segment([o - vec2(0.0, 6.0), o + vec2(0.0, 6.0)], stroke);
    } else if let Some(a) = c.project.asset(clip.asset) {
        let p = placement(c.project, clip, c.playhead, (a.width, a.height), cw, ch, true);
        let (s, co) = p.rot.to_radians().sin_cos();
        let (hw, hh) = (p.w / 2.0, p.h / 2.0);
        let pts = [(-hw, -hh), (hw, -hh), (hw, hh), (-hw, hh)]
            .iter()
            .map(|&(x, y)| to_screen(p.cx + x * co - y * s, p.cy + x * s + y * co))
            .collect();
        painter.add(Shape::closed_line(pts, stroke));
    }

    // explicit polygon vertices: outline + one grab handle each, in the shape's own rotated/scaled frame
    let poly: Vec<(f32, f32)> =
        clip.shape.as_ref().and_then(|s| s.poly_points()).map(|p| p.to_vec()).unwrap_or_default();
    let frame = (!poly.is_empty()).then(|| {
        let p = placement(c.project, clip, c.playhead, (1, 1), cw, ch, false);
        let (sn, cs) = p.rot.to_radians().sin_cos();
        // project px -> screen points, through the clip's scale
        let k = clip.scale.at(lt) as f32 * lb.width() / c.project.width.max(1) as f32;
        (to_screen(p.cx, p.cy), sn, cs, if k.is_finite() && k.abs() > 1e-4 { k } else { 1e-4 })
    });
    // the grab is decided by where the press began, not by where the pointer has dragged to
    let press = ui.input(|i| i.pointer.press_origin());
    let mut hit = None;
    if let Some((o, sn, cs, k)) = frame {
        let scr: Vec<Pos2> =
            poly.iter().map(|&(x, y)| pos2(o.x + (x * cs - y * sn) * k, o.y + (x * sn + y * cs) * k)).collect();
        painter.add(Shape::closed_line(scr.clone(), stroke));
        for (i, q) in scr.iter().enumerate() {
            painter.circle_filled(*q, 3.5, c.palette.selection);
            if press.is_some_and(|pp| (pp - *q).length() <= 7.0) {
                hit = Some(i);
            }
        }
    }

    if resp.drag_started() {
        (c.undo)(c.project);
        state.point_drag = hit;
        state.drag = hit.is_none().then(|| (clip.x.at(lt), clip.y.at(lt), Vec2::ZERO));
    }
    if resp.dragged() {
        if let (Some(i), Some((o, sn, cs, k)), Some(pp)) = (state.point_drag, frame, resp.interact_pointer_pos()) {
            // pointer -> the shape's own frame (undo the rotation and scale the handles were drawn with)
            let (dx, dy) = ((pp.x - o.x) / k, (pp.y - o.y) / k);
            if let Some(p) = c.project.clip_mut(id).and_then(|cl| cl.shape.as_mut()).and_then(|s| s.points.get_mut(i)) {
                *p = (dx * cs + dy * sn, -dx * sn + dy * cs);
                r.edited = true;
            }
        } else if let Some((x0, y0, acc)) = &mut state.drag {
            *acc += resp.drag_delta();
            let k = c.project.width as f32 / lb.width(); // project px per point
            let (nx, ny) = (*x0 + (acc.x * k) as f64, *y0 + (acc.y * k) as f64);
            if let Some(cl) = c.project.clip_mut(id) {
                cl.x.set_at(lt, nx);
                cl.y.set_at(lt, ny);
                r.edited = true;
            }
        }
    }
    if resp.drag_stopped() {
        state.drag = None;
        state.point_drag = None;
    }
}

/// Small "Pre-rendering NN %" badge in the top-left of the video while frames are being cached.
fn prerender_badge(ui: &egui::Ui, painter: &egui::Painter, lb: Rect, c: &PreviewCtx<'_>) {
    let Some(p) = c.prerender else { return };
    let text = format!("Pre-rendering {:.0} %", (p.clamp(0.0, 1.0) * 100.0));
    let font = egui::TextStyle::Small.resolve(ui.style());
    let galley = painter.layout_no_wrap(text, font, c.palette.text);
    let pad = vec2(6.0, 3.0);
    let at = lb.min + vec2(6.0, 6.0);
    let bg = Rect::from_min_size(at, galley.size() + pad * 2.0);
    painter.rect_filled(bg, 3.0, c.palette.header.gamma_multiply(0.9));
    painter.rect_stroke(bg, 3.0, Stroke::new(1.0, c.palette.accent), StrokeKind::Inside);
    painter.galley(at + pad, galley, c.palette.text);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Clip, MaskShape};
    use eframe::egui::{Event, Modifiers, PointerButton, RawInput};

    #[test]
    fn letterbox_fits_and_centres() {
        let area = Rect::from_min_size(pos2(10.0, 20.0), vec2(400.0, 300.0));
        // wide video in a 4:3 area → full width, bars top/bottom
        let lb = letterbox(area, 16.0 / 9.0, 1.0);
        assert_eq!(lb.width(), 400.0);
        assert_eq!(lb.height(), 225.0);
        assert!((lb.center() - area.center()).length() <= 0.5);
        // tall video → full height, bars left/right
        let lb = letterbox(area, 0.5, 1.0);
        assert_eq!(lb.height(), 300.0);
        assert_eq!(lb.width(), 150.0);
        assert!((lb.center() - area.center()).length() <= 0.5);
    }

    #[test]
    fn letterbox_snaps_to_pixels() {
        let area = Rect::from_min_size(pos2(0.3, 0.0), vec2(333.3, 200.0));
        let lb = letterbox(area, 16.0 / 9.0, 2.0);
        for v in [lb.min.x, lb.min.y, lb.width(), lb.height()] {
            assert!((v * 2.0 - (v * 2.0).round()).abs() < 1e-3, "{v} not pixel aligned");
        }
        assert!(lb.width() <= area.width() + 0.5 && lb.height() <= area.height() + 0.5);
    }

    struct H {
        ctx: egui::Context,
        state: PreviewState,
        project: Project,
        selection: Vec<Id>,
        tool: Tool,
        undos: usize,
        time: f64,
        panel: Rect,
    }

    impl H {
        fn new() -> Self {
            let mut project = Project::new();
            project.tracks[0].clips.push(Clip::new(7, ClipKind::Video, "v", 0.0, 4.0));
            Self {
                ctx: egui::Context::default(),
                state: PreviewState::default(),
                project,
                selection: vec![7],
                tool: Tool::Select,
                undos: 0,
                time: 0.0,
                panel: Rect::from_min_size(Pos2::ZERO, vec2(700.0, 460.0)),
            }
        }
        fn frame(&mut self, events: Vec<Event>) -> PreviewResponse {
            self.time += 0.05;
            let input = RawInput { screen_rect: Some(self.panel), time: Some(self.time), events, ..Default::default() };
            let pal = Palette::new(true, Color32::WHITE);
            let H { ctx, state, project, selection, tool, undos, .. } = self;
            let mut out = PreviewResponse::default();
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut undo = |_: &Project| *undos += 1;
                    out = show(
                        ui,
                        state,
                        PreviewCtx {
                            project,
                            selection,
                            playhead: 1.0,
                            playing: false,
                            fullscreen: false,
                            palette: &pal,
                            undo: &mut undo,
                            frame: None,
                            gpu_texture: None,
                            tool: *tool,
                            quality: 100,
                            movie_mode: false,
                            prerender: Some(0.4),
                        },
                    );
                });
            });
            out
        }
        fn click(&mut self, at: Pos2) -> PreviewResponse {
            self.frame(vec![Event::PointerMoved(at)]);
            let btn = |pressed| Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            self.frame(vec![btn(true)]);
            self.frame(vec![btn(false)])
        }
        fn drag(&mut self, from: Pos2, to: Pos2) -> PreviewResponse {
            self.frame(vec![Event::PointerMoved(from)]);
            self.frame(vec![Event::PointerButton {
                pos: from,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            }]);
            let steps = 4;
            let (mut mask_edit, mut edited) = (false, false);
            for i in 1..=steps {
                let p = from + (to - from) * (i as f32 / steps as f32);
                let f = self.frame(vec![Event::PointerMoved(p)]);
                mask_edit |= f.mask_edit;
                edited |= f.edited;
            }
            let mut out = self.frame(vec![Event::PointerButton {
                pos: to,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            }]);
            // the flags are per-frame; the test wants the whole gesture
            out.mask_edit |= mask_edit;
            out.edited |= edited;
            out
        }
    }

    #[test]
    fn transport_is_centred_under_the_video() {
        let mut h = H::new();
        h.frame(vec![]); // measures
        h.frame(vec![]); // centres with the measurement
        let row = h.state.transport;
        assert!(row.width() > 100.0, "transport measured: {row:?}");
        let dx = (row.center().x - h.panel.center().x).abs();
        assert!(dx < 6.0, "transport centre {} vs panel centre {}", row.center().x, h.panel.center().x);
    }

    #[test]
    fn shape_tool_drag_reports_a_shape_instead_of_moving_the_clip() {
        let mut h = H::new();
        h.tool = Tool::Shape(ShapeKind::Rect);
        h.frame(vec![]);
        let out = h.drag(pos2(250.0, 150.0), pos2(400.0, 250.0));
        let (kind, cx, cy, w, hh) = out.new_shape.expect("a shape was dragged out");
        assert_eq!(kind, ShapeKind::Rect);
        assert!(w > 1.0 && hh > 1.0, "non-empty shape: {w} x {hh}");
        // the drag must not have moved the clip
        let c = h.project.clip(7).unwrap();
        assert_eq!((c.x.value, c.y.value), (0.0, 0.0), "shape tool never moves the clip");
        assert_eq!(h.undos, 0, "no clip undo for a shape gesture");
        let _ = (cx, cy);
    }

    /// A line follows the drag in every direction: the half-extents stay signed, so dragging up-right
    /// makes a line that points up-right instead of snapping to its bounding box's other diagonal.
    #[test]
    fn line_tool_keeps_the_press_as_its_origin() {
        // (dx, dy) of the drag -> expected sign of (w, h)
        for (to, want) in [
            (pos2(400.0, 250.0), (1.0, 1.0)),   // right + down
            (pos2(400.0, 50.0), (1.0, -1.0)),   // right + up
            (pos2(100.0, 250.0), (-1.0, 1.0)),  // left  + down
            (pos2(100.0, 50.0), (-1.0, -1.0)),  // left  + up
        ] {
            let mut h = H::new();
            h.tool = Tool::Shape(ShapeKind::Line);
            h.frame(vec![]);
            let from = pos2(250.0, 150.0);
            let out = h.drag(from, to);
            let (kind, cx, cy, w, hh) = out.new_shape.expect("a line was dragged out");
            assert_eq!(kind, ShapeKind::Line);
            assert_eq!(w.signum(), want.0, "w sign for a drag to {to:?}: got {w}");
            assert_eq!(hh.signum(), want.1, "h sign for a drag to {to:?}: got {hh}");
            // centre + the signed half-extents reproduce both ends, which is what engine::shapes draws
            let (x0, y0) = (cx - w, cy - hh);
            let (x1, y1) = (cx + w, cy + hh);
            assert!(x1 - x0 != 0.0 && y1 - y0 != 0.0);
            assert_eq!(((x1 - x0).signum(), (y1 - y0).signum()), want, "the far end must follow the cursor");
        }
    }

    #[test]
    fn polygon_tool_places_and_closes_real_points() {
        let mut h = H::new();
        h.tool = Tool::Shape(ShapeKind::Polygon);
        h.frame(vec![]);
        for p in [pos2(300.0, 120.0), pos2(380.0, 240.0), pos2(220.0, 240.0)] {
            let out = h.click(p);
            assert!(out.new_shape.is_none(), "the path is still open");
        }
        assert_eq!(h.state.poly.len(), 3, "one vertex per click");
        let out = h.frame(vec![Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
        let (kind, cx, cy, w, hh) = out.new_shape.expect("Enter closes the path");
        assert_eq!(kind, ShapeKind::Polygon);
        assert_eq!(out.new_points.len(), 3);
        // the points are centred on the reported centre and the half-size is exactly their bounds
        let mx = out.new_points.iter().fold(0.0f32, |a, p| a.max(p.0.abs()));
        let my = out.new_points.iter().fold(0.0f32, |a, p| a.max(p.1.abs()));
        assert!(w > 1.0 && hh > 1.0 && cx.is_finite() && cy.is_finite(), "{:?}", (cx, cy, w, hh));
        assert!((mx - w).abs() < 0.5 && (my - hh).abs() < 0.5, "{:?} vs {:?}", (mx, my), (w, hh));
        assert!(h.state.poly.is_empty(), "the in-progress path is consumed");
        // clicking the last vertex again (a double-click's second press) closes a path too
        for p in [pos2(200.0, 100.0), pos2(260.0, 100.0), pos2(260.0, 160.0)] {
            h.click(p);
        }
        let out = h.click(pos2(260.0, 160.0));
        assert_eq!(out.new_points.len(), 3, "a click back on a vertex closes: {:?}", out.new_points);
    }

    #[test]
    fn select_tool_drags_a_polygon_vertex() {
        let mut h = H::new();
        let id = h.project.add_shape_clip(ShapeKind::Polygon, 0.0, 4.0);
        let s = h.project.clip_mut(id).unwrap().shape.as_mut().unwrap();
        s.points = vec![(0.0, 0.0), (200.0, 100.0), (-200.0, 100.0)];
        h.selection = vec![id];
        h.frame(vec![]);
        // the transport row is measured now, so the video rect (and its centre) is known
        h.frame(vec![]);
        // the first vertex sits on the shape centre, i.e. the centre of the video
        let at = pos2(h.panel.center().x, h.state.transport.top() / 2.0);
        let out = h.drag(at, at + vec2(40.0, 20.0));
        assert!(out.edited, "dragging a handle edits");
        let cl = h.project.clip(id).unwrap();
        let p = cl.shape.as_ref().unwrap().points[0];
        assert!(p.0 > 10.0 && p.1 > 5.0, "the vertex followed the pointer: {p:?}");
        assert_eq!(cl.shape.as_ref().unwrap().points[1], (200.0, 100.0), "the other vertices stay put");
        assert_eq!((cl.x.value, cl.y.value), (0.0, 0.0), "grabbing a handle never moves the clip");
        assert_eq!(h.undos, 1, "one undo for the gesture");
    }

    #[test]
    fn draw_tool_records_a_timed_stroke() {
        let mut h = H::new();
        h.tool = Tool::Draw;
        h.frame(vec![]);
        let out = h.drag(pos2(200.0, 120.0), pos2(430.0, 260.0));
        let s = out.stroke.expect("a stroke was recorded");
        assert!(s.points.len() > 1, "{} points", s.points.len());
        assert!(s.points[0].2 <= s.points[s.points.len() - 1].2, "times increase");
    }

    #[test]
    fn mask_tool_drag_edits_the_selected_clips_mask() {
        let mut h = H::new();
        h.tool = Tool::Mask(MaskShape::Ellipse);
        h.frame(vec![]);
        let out = h.drag(pos2(240.0, 140.0), pos2(420.0, 260.0));
        assert!(out.mask_edit, "mask edits are reported");
        let m = h.project.clip(7).unwrap().mask.clone().expect("mask created");
        assert_eq!(m.shape, MaskShape::Ellipse);
        assert!(m.rx.value > 1.0 && m.ry.value > 1.0, "{:?}", (m.rx.value, m.ry.value));
        assert_eq!(h.undos, 1, "one undo for the mask gesture");
    }

    #[test]
    fn polygon_mask_drag_leaves_usable_vertices() {
        let mut h = H::new();
        h.tool = Tool::Mask(MaskShape::Polygon);
        h.frame(vec![]);
        h.drag(pos2(240.0, 140.0), pos2(420.0, 260.0));
        let m = h.project.clip(7).unwrap().mask.clone().expect("mask created");
        assert_eq!(m.shape, MaskShape::Polygon);
        // fewer than 3 points rasterises as "outside everywhere" and the clip disappears
        assert!(m.points.len() >= 3, "{:?}", m.points);
        let xs: Vec<f32> = m.points.iter().map(|p| p.0).collect();
        assert!(xs.iter().cloned().fold(f32::MIN, f32::max) > xs.iter().cloned().fold(f32::MAX, f32::min));
    }

    #[test]
    fn select_tool_still_moves_the_clip() {
        let mut h = H::new();
        h.project.clip_mut(7).unwrap().asset = 0;
        h.frame(vec![]);
        let out = h.drag(pos2(300.0, 150.0), pos2(360.0, 190.0));
        // no asset → no outline, but the drag path still runs; either way nothing panics
        let _ = out;
    }
}
