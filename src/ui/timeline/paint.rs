//! Timeline paint helpers: free functions for row layout, snapping, and drawing clip internals.
#![allow(dead_code)]
use super::*;

pub(super) fn row_order(p: &Project) -> impl Iterator<Item = usize> + '_ {
    let n = p.tracks.len();
    (0..n)
        .rev()
        .filter(move |&i| p.tracks[i].kind == TrackKind::Video)
        .chain((0..n).filter(move |&i| p.tracks[i].kind == TrackKind::Audio))
}

/// Top y of track `ti` in display order (absolute points).
pub(crate) fn row_top(state: &TimelineState, p: &Project, ti: usize) -> Option<f32> {
    let mut top = state.lanes_rect.top() - state.scroll_y;
    for i in row_order(p) {
        if i == ti {
            return Some(top);
        }
        top += p.tracks[i].height;
    }
    None
}

/// The clip under a drop at screen `pos` / timeline time `t`, with its lane rect — what an effect or a
/// transition dragged out of its panel is aimed at (`app.rs` resolves the same clip from the reported
/// time + track when the drop actually happens).
pub(super) fn drop_on_clip<'a>(state: &TimelineState, p: &'a Project, pos: Pos2, t: f64) -> Option<(Rect, &'a Clip)> {
    let ti = state.track_at(pos.y, p)?;
    let top = row_top(state, p, ti)?;
    let clip = p.tracks.get(ti)?.clips.iter().find(|c| c.contains(t))?;
    let rect = Rect::from_min_max(
        pos2(state.x_at(clip.start), top + 1.0),
        pos2(state.x_at(clip.end()), top + p.tracks[ti].height - 1.0),
    );
    Some((rect, clip))
}

/// (major, minor) tick spacing in seconds for a zoom (px/s).
pub(super) fn tick_step(zoom: f32) -> (f64, f64) {
    TICKS.iter().copied().find(|(major, _)| *major * zoom as f64 >= 80.0).unwrap_or(TICKS[TICKS.len() - 1])
}

pub(super) fn tick_label(t: f64, major: f64) -> String {
    let m = (t / 60.0).floor();
    let s = t - m * 60.0;
    if major >= 1.0 {
        format!("{}:{:02}", m, s.round())
    } else if major >= 0.1 {
        format!("{}:{:04.1}", m, s)
    } else {
        format!("{}:{:05.2}", m, s)
    }
}

pub(super) fn toggle_button(
    ui: &egui::Ui,
    p: &egui::Painter,
    rect: Rect,
    id: egui::Id,
    label: Cap,
    on: bool,
    pal: &Palette,
    font: &FontId,
) -> bool {
    let r = ui.interact(rect, id, Sense::click());
    let fill = if on { pal.accent } else { pal.panel };
    let stroke = if r.hovered() { pal.accent } else { pal.border };
    p.rect(rect, CornerRadius::same(2), fill, Stroke::new(1.0, stroke), StrokeKind::Inside);
    match label {
        Cap::Icon(g) => draw_glyph(p, rect, g, pal.text),
        Cap::Text(t) => {
            p.text(rect.center(), Align2::CENTER_CENTER, t, font.clone(), pal.text);
        }
    }
    r.clicked()
}

/// One vertical min..max line per point column over the visible part of an audio clip.
pub(super) fn draw_waveform(
    p: &egui::Painter,
    peaks: &Peaks,
    clip: &Clip,
    vis: Rect,
    state: &TimelineState,
    color: Color32,
) {
    let mid = vis.center().y;
    let half = (vis.height() * 0.5 - 1.0).max(0.0);
    let stroke = Stroke::new(1.0, color);
    let mut x = vis.left().floor();
    while x < vis.right() {
        let t0 = clip.src_time(state.time_at(x));
        let t1 = clip.src_time(state.time_at(x + 1.0));
        let (lo, hi) = peaks.range(t0, t1);
        if hi > lo {
            p.vline(x + 0.5, Rangef::new(mid - hi * half, mid - lo * half), stroke);
        }
        x += 1.0;
    }
}

/// Would the inline mini graph have anything to plot? It draws properties with 2+ keys through the
/// curve editor's plumbing, so this gates the toggle on exactly that — `has_keys` below is true for a
/// bare mask/shape too and used to open an empty panel.
pub(super) fn has_curve_keys(c: &Clip) -> bool {
    (0..crate::ui::curves::prop_count(c)).any(|i| crate::ui::curves::prop_ref(c, i).is_some_and(|a| a.keys.len() >= 2))
}

/// Does the clip have any keyframe at all? Cheap (no allocation), unlike `Clip::key_times`, so the 1000-clip
/// case never touches the keyframe path.
pub(super) fn has_keys(c: &Clip) -> bool {
    c.animated().iter().any(|a| a.is_animated())
        || c.effects.iter().any(|e| e.params.iter().any(|a| a.is_animated()) || e.mask.is_some())
        || c.mask.is_some()
        || c.graph.is_some()
        || c.shape.is_some()
}

/// Value range the diamonds of `prop` are mapped through: the live auto-range, or the one frozen at the
/// start of the drag so the dragged key does not push its own scale around.
pub(super) fn key_range(state: &TimelineState, clip: Id, prop: usize, a: &crate::model::Animated) -> (f64, f64) {
    if let Some(Drag { g: Gesture::Keys { id, prop: Some(p), range, .. }, .. }) = &state.drag {
        if *id == clip && *p == prop {
            return *range;
        }
    }
    crate::ui::curves::y_range(a)
}

/// Same colour cycle as the Curve Editor's per-property lines (`curves::prop_color`, private to that
/// file) — duplicated here so this view stays self-contained rather than reaching into curves.rs.
pub(super) fn mini_prop_color(pal: &Palette, i: usize) -> Color32 {
    let cycle =
        [pal.clip_video, pal.clip_audio, pal.clip_image, pal.clip_text, pal.clip_sequence, pal.waveform, pal.keyframe];
    cycle[i % cycle.len()]
}

/// Compact view-only keyframe graph for one clip: one polyline per animated property with 2+ keys, each
/// normalized to its own auto-range (`curves::y_range`, the same range the value-lane diamonds use),
/// dots at each key, dropped in a small panel below the clip. Not editable — that's the Curve Editor
/// pane (`ui/curves.rs`); this is a lightweight glance, not a replacement for it.
/// ponytail: doesn't negotiate space with the row(s) below — it just paints on top, clipped to the lanes.
pub(super) fn draw_mini_graph(p: &egui::Painter, clip: &Clip, rect: Rect, lanes: Rect, pal: &Palette) {
    let panel = Rect::from_min_max(
        pos2(rect.left(), rect.bottom() + 2.0),
        pos2(rect.right(), rect.bottom() + 2.0 + MINI_GRAPH_H),
    );
    if !lanes.intersects(panel) {
        return;
    }
    let cr = CornerRadius::same(3);
    p.rect_filled(panel, cr, pal.panel);
    p.rect_stroke(panel, cr, Stroke::new(1.0, pal.border), StrokeKind::Inside);
    let inner = panel.shrink(4.0);
    let dur = clip.duration.max(1e-6);
    for i in 0..crate::ui::curves::prop_count(clip) {
        let Some(a) = crate::ui::curves::prop_ref(clip, i) else { continue };
        if a.keys.len() < 2 {
            continue;
        }
        let (lo, hi) = crate::ui::curves::y_range(a);
        let span = (hi - lo).max(1e-9);
        let color = mini_prop_color(pal, i);
        let mut last: Option<Pos2> = None;
        for k in &a.keys {
            let fx = (k.t / dur).clamp(0.0, 1.0) as f32;
            let fy = ((k.v - lo) / span).clamp(0.0, 1.0) as f32;
            let pt = pos2(inner.left() + fx * inner.width(), inner.bottom() - fy * inner.height());
            if let Some(l) = last {
                p.line_segment([l, pt], Stroke::new(1.2, color));
            }
            p.circle_filled(pt, 2.0, color);
            last = Some(pt);
        }
    }
}

/// The range the rest of the UI enforces for property `i` (inspector sliders for the base props,
/// `ParamSpec` for effect params), so a value-lane drag cannot write past what a `DragValue` allows.
/// `None` where nothing enforces one: Position X/Y and Rotation are unbounded everywhere, and Volume is
/// a gain behind a nonlinear dB slider.
/// ponytail: same property order as curves.rs — move it next to `prop_ref` if a second caller shows up.
pub(super) fn prop_range(c: &Clip, i: usize) -> Option<(f64, f64)> {
    let nb = if c.is_visual() { 8 } else { 3 };
    if i < nb {
        return match (c.is_visual(), i) {
            (true, 2) => Some((0.01, 20.0)),              // Scale
            (true, 3) | (true, 4) => Some((0.01, 20.0)),  // Scale X / Scale Y — same clamp as Scale
            (true, 6) => Some((0.0, 1.0)),                // Opacity
            (false, 1) => Some((-1.0, 1.0)),              // Pan
            (_, _) if i == nb - 1 => Some((0.01, 100.0)), // Speed — same clamp as Clip::set_speed
            _ => None,
        };
    }
    let mut i = i - nb;
    for e in &c.effects {
        if i < e.params.len() {
            return e.specs().get(i).map(|s| (s.min, s.max));
        }
        i -= e.params.len();
    }
    None
}

/// Diagonal hatching (adjustment layers read as "applies to everything below").
pub(super) fn hatch(p: &egui::Painter, vis: Rect, color: Color32) {
    let (step, stroke, h) = (8.0f32, Stroke::new(1.0, color), vis.height());
    let mut x = (vis.left() / step).floor() * step - h;
    while x < vis.right() {
        p.line_segment([pos2(x, vis.bottom()), pos2(x + h, vis.top())], stroke);
        x += step;
    }
}

/// Marker flag: a stem down from `top` with a pennant to the right.
pub(super) fn flag(p: &egui::Painter, x: f32, top: f32, bottom: f32, color: Color32, wide: bool) {
    p.vline(x, Rangef::new(top, bottom), Stroke::new(if wide { 2.0 } else { 1.0 }, color));
    p.add(Shape::convex_polygon(
        vec![pos2(x, top), pos2(x + FLAG_W, top + 3.0), pos2(x, top + 6.0)],
        color,
        Stroke::NONE,
    ));
}

/// Shared marker interaction (ruler flags and clip flags): click selects, double-click seeks, drag moves,
/// right-click = Rename / Delete / Set label.
#[allow(clippy::too_many_arguments)]
pub(super) fn marker_hit(
    r: egui::Response,
    m: &crate::model::Marker,
    clip: Option<Id>,
    clip_start: f64,
    selected: &mut Option<Id>,
    start: &mut Option<(Id, Option<Id>)>,
    act: &mut Option<Act>,
    rename: &mut Option<(Id, String)>,
    seek: &mut Option<f64>,
    labels: &[Label],
) {
    if r.clicked() {
        *selected = Some(m.id);
    }
    if r.double_clicked() {
        *seek = Some(clip_start + m.t);
    }
    if r.drag_started_by(egui::PointerButton::Primary) {
        *selected = Some(m.id);
        *start = Some((m.id, clip));
    }
    if r.secondary_clicked() {
        *selected = Some(m.id);
        *rename = Some((m.id, m.name.clone()));
    }
    let r = r.on_hover_ui(|ui| {
        ui.label(if m.name.is_empty() { "(unnamed marker)" } else { m.name.as_str() });
    });
    r.context_menu(|ui| {
        ui.horizontal(|ui| {
            ui.label("Name");
            if let Some((_, buf)) = rename.as_mut().filter(|(rid, _)| *rid == m.id) {
                let te = ui.add(egui::TextEdit::singleline(buf).desired_width(140.0));
                if te.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    *act = Some(Act::RenameMarker(m.id, buf.clone()));
                    ui.close();
                }
            }
        });
        if ui.button("Delete").clicked() {
            *act = Some(Act::DelMarker(m.id));
        }
        ui.menu_button("Set label", |ui| {
            if ui.button("None").clicked() {
                *act = Some(Act::MarkerLabel(m.id, 0));
            }
            for (i, l) in labels.iter().enumerate() {
                if ui.button(&l.name).clicked() {
                    *act = Some(Act::MarkerLabel(m.id, i as u8 + 1));
                }
            }
        });
    });
}

/// Wave colour of an audio clip: the plain palette green while it has no label, otherwise its own
/// (already computed) body colour pushed to the opposite end of the brightness scale — the body is
/// painted in that colour, so an unshifted wave would be invisible on it.
pub(super) fn wave_color(body: Color32, label: u8, pal: &Palette) -> Color32 {
    if label == 0 {
        return pal.waveform;
    }
    let target = if body.intensity() > 0.5 { Color32::BLACK } else { Color32::WHITE };
    body.lerp_to_gamma(target, 0.55)
}

pub(super) fn label_color(p: &Project, idx: u8, fallback: Color32) -> Color32 {
    match p.label_color(idx) {
        Some([r, g, b]) => Color32::from_rgb(r, g, b),
        None => fallback,
    }
}

pub(super) fn diamond(c: Pos2, r: f32, color: Color32) -> Shape {
    Shape::convex_polygon(
        vec![pos2(c.x, c.y - r), pos2(c.x + r, c.y), pos2(c.x, c.y + r), pos2(c.x - r, c.y)],
        color,
        Stroke::NONE,
    )
}

/// dB → fraction of the clip height measured from the bottom. Piecewise linear: -60 dB = 0, 0 dB = 0.7,
/// +12 dB = 1 (so unity gain sits at 70 % height and the usable attenuation range gets most of the clip).
pub(super) fn db_frac(db: f32) -> f32 {
    if db >= 0.0 {
        0.7 + (db / DB_TOP) * 0.3
    } else {
        0.7 * (1.0 - db / DB_BOT)
    }
}

pub(super) fn frac_db(f: f32) -> f32 {
    if f >= 0.7 {
        (f - 0.7) / 0.3 * DB_TOP
    } else {
        (1.0 - f / 0.7) * DB_BOT
    }
}

pub(super) fn gain_db(g: f32) -> f32 {
    (20.0 * g.max(1e-4).log10()).clamp(DB_BOT, DB_TOP)
}

/// Filmstrip of source-time thumbnails across a video/image clip. Non-blocking: misses are queued in the
/// ThumbCache worker and painted on a later frame.
pub(super) fn draw_filmstrip(
    p: &egui::Painter,
    ectx: &egui::Context,
    state: &TimelineState,
    clip: &Clip,
    asset: &Asset,
    rect: Rect,
    vis: Rect,
    thumbs: &mut ThumbCache,
) {
    let band = 16.0; // name row above the strip
    let h = rect.height() - band;
    if h < 12.0 || !vis.is_positive() {
        return;
    }
    // ponytail: 16 px decode buckets, so a track-height drag reuses thumbs instead of queueing a fresh
    // set (new cache key + new t grid) every frame. Ceiling: <5 % horizontal squash from the rounding.
    let hq = (((h as u32 + 8) / 16) * 16).max(16);
    let aspect = if asset.height > 0 { asset.width as f32 / asset.height as f32 } else { 16.0 / 9.0 };
    let step = (hq as f32 * aspect).max(80.0);
    let pc = p.with_clip_rect(vis);
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    let mut n = ((vis.left() - rect.left()) / step).floor().max(0.0);
    loop {
        let x = rect.left() + n * step;
        if x >= vis.right() || x >= rect.right() {
            break;
        }
        let t = clip.src_time(state.time_at(x)).max(0.0);
        if let Some((tex, [tw, th])) = thumbs.texture(ectx, &asset.path, t, hq) {
            let w = h * tw as f32 / th.max(1) as f32;
            pc.image(tex, Rect::from_min_size(pos2(x, rect.top() + band), vec2(w.min(step), h)), uv, Color32::WHITE);
        }
        n += 1.0;
    }
}
