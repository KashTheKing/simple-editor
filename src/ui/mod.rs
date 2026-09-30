//! egui UI. Dockable layout (ui/layout.rs) of panes: Library, Preview, Inspector, Effects, Transitions,
//! Subtitles, Timeline, Curves, Planner, Moodboard, Auto-cut, Tracking - DaVinci-like by default, bare
//! Windows-forms styling. Windows (Settings, Retime, Export) never block the editor.

pub mod app;
pub mod autocut_ui;
pub mod capture_ui;
pub mod cheatsheet;
// ---- ws:inspector-gallery ----
pub mod color_ui;
pub mod confirm;
pub mod curves;
pub mod effects_ui;
pub mod export_ui;
// ---- ws:pro-timeline ----
pub mod find_ui;
pub mod frame_ui;
// ---- ws:inspector-gallery ----
pub mod gallery;
pub mod guides;
pub mod spiky_ball;
pub mod history_ui;
// ---- ws:layout-modes-onboarding ----
pub mod home;
pub mod import_ui;
pub mod inspector;
pub mod inspector_audio;
pub mod inspector_text;
pub mod layout;
pub mod library;
pub mod markdown;
pub mod markers_ui;
// ---- ws:ui-kit ----
pub mod menu;
pub mod mixer_ui;
pub mod moodboard_ui;
// ---- ws:pro-monitor ----
pub mod multicam_ui;
pub mod nodes;
// ---- ws:layout-modes-onboarding ----
pub mod onboarding;
pub mod palette;
pub mod paste_ui;
pub mod planner;
pub mod preview;
pub mod retime;
// ---- ws:pro-monitor ----
pub mod scopes_ui;
pub mod settings_ui;
pub mod shader_ui;
// ---- ws:source-monitor ----
pub mod source_ui;
pub mod subtitles_ui;
pub mod timeline;
pub mod tools;
pub mod tracking_ui;
// ---- ws:transcript-captions ----
pub mod transcript_ui;
pub mod transitions_ui;
// ---- ws:jobs-panel ----
pub mod jobs_ui;

use crate::model::{AnimLink, Animated, Id, Mask, MaskShape, Project, LABEL_COLORS};
use crate::theme::Palette;
use eframe::egui::{self, DragValue, Grid, Response};

/// True on the first frame of an edit gesture (drag start, or a non-drag change such as typing/clicking).
pub(crate) fn edit_start(r: &Response) -> bool {
    r.drag_started() || (r.changed() && !r.dragged())
}

/// Accumulates widget responses over one frame: `start` → push undo once, `changed` → write back.
/// Shared by every panel so the gesture rule lives in one place.
#[derive(Default)]
pub(crate) struct Gesture {
    pub start: bool,
    pub changed: bool,
}

impl Gesture {
    pub fn note(&mut self, r: &Response) {
        self.start |= edit_start(r);
        self.changed |= r.changed();
    }
    /// Text fields: the gesture starts when the field is entered, so typing a paragraph is one undo
    /// entry instead of one per character (which used to evict the whole undo stack).
    pub fn note_text(&mut self, r: &Response) {
        self.start |= r.gained_focus();
        self.changed |= r.changed();
    }
    pub fn click(&mut self) {
        self.start = true;
        self.changed = true;
    }
}

/// Push undo at most once per frame.
pub(crate) fn once(flag: &mut bool, undo: &mut dyn FnMut(&Project), p: &Project) {
    if !*flag {
        undo(p);
        *flag = true;
    }
}

thread_local! {
    /// Previous / Next key picked from a keyframe menu: the clip-local time to jump to. Only the caller
    /// knows the clip, so the inspector turns it into a timeline seek (`take_key_seek`).
    static KEY_SEEK: std::cell::Cell<Option<f64>> = const { std::cell::Cell::new(None) };
}

/// Clip-local time a keyframe menu's Previous / Next key asked to jump to, since the last call.
pub(crate) fn take_key_seek() -> Option<f64> {
    KEY_SEEK.with(|s| s.take())
}

/// The one keyframe control of every property row (Inspector, Effects stack, masks, shapes): a ◆ that
/// toggles a key at the clip-local playhead `lt` - filled when a key sits here, outlined when the
/// property is animated elsewhere, faint when it isn't; a ∿ while a live link drives it. Right-click it
/// (or the value - see `key_menu`) for everything else. `label` names the property, and is its id.
pub(crate) fn key_buttons(
    ui: &mut egui::Ui,
    a: &mut Animated,
    lt: f64,
    palette: &Palette,
    g: &mut Gesture,
    label: &str,
    paths: &[(Id, String)],
) -> Response {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
    let r = ui.interact(rect, ui.id().with(("kf", label)), egui::Sense::click());
    let here = a.has_key_at(lt);
    let color = if a.is_animated() || !a.link.is_none() { palette.accent } else { palette.text_dim };
    let color = if r.hovered() { palette.text } else { color };
    let p = ui.painter();
    if a.link.is_none() {
        let (c, h) = (rect.center(), 5.0);
        let pts = vec![c - egui::vec2(0.0, h), c + egui::vec2(h, 0.0), c + egui::vec2(0.0, h), c - egui::vec2(h, 0.0)];
        let fill = if here { color } else { egui::Color32::TRANSPARENT };
        p.add(egui::Shape::convex_polygon(pts, fill, egui::Stroke::new(1.3, color)));
    } else {
        p.text(rect.center(), egui::Align2::CENTER_CENTER, "∿", egui::FontId::proportional(15.0), color);
    }
    let tip = match &a.link {
        AnimLink::PathX(_) | AnimLink::PathY(_) => "Following a path - right-click to unlink",
        AnimLink::Expr(_) => "Driven by an expression - right-click to unlink",
        AnimLink::None if here => "Remove the keyframe at the playhead (right-click: more)",
        AnimLink::None => "Add a keyframe at the playhead (right-click: more)",
    };
    let r = r.on_hover_text(tip);
    if r.clicked() {
        a.toggle_key(lt);
        g.click();
    }
    menu::context(&r, |ui| key_menu(ui, a, lt, g, label, paths));
    r
}

/// The keyframe right-click menu, shared by the ◆ and the value next to it: Previous / Next key (a seek,
/// see `take_key_seek`), Clear keyframes, and the live links - a saved path (Position X/Y only) or a
/// Luau expression, AE-style. A manual edit elsewhere breaks a link (`Animated::set_at`).
pub(crate) fn key_menu(
    ui: &mut egui::Ui,
    a: &mut Animated,
    lt: f64,
    g: &mut Gesture,
    label: &str,
    paths: &[(Id, String)],
) {
    use crate::ui::menu;
    use crate::ui::tools::{Dir, Glyph};
    const EPS: f64 = 1e-4;
    let prev = a.keys.iter().rev().find(|k| k.t < lt - EPS).map(|k| k.t);
    let next = a.keys.iter().find(|k| k.t > lt + EPS).map(|k| k.t);
    for (t, glyph, text) in
        [(prev, Glyph::Jump(Dir::Left), "Previous key"), (next, Glyph::Jump(Dir::Right), "Next key")]
    {
        if ui.add_enabled_ui(t.is_some(), |ui| menu::row(ui, Some(glyph), text, "")).inner.clicked() {
            KEY_SEEK.with(|s| s.set(t));
            ui.ctx().request_repaint();
        }
    }
    let animated = a.is_animated();
    if ui.add_enabled_ui(animated, |ui| menu::row(ui, Some(Glyph::Cross), "Clear keyframes", "")).inner.clicked() {
        a.clear_keys(lt);
        g.click();
    }
    ui.separator();
    let axis_x = label == "Position X";
    if axis_x || label == "Position Y" {
        menu::sub(ui, Some(Glyph::Link), "Link to path", |ui| {
            if paths.is_empty() {
                ui.weak("No saved paths - save one from a shape's Path section");
            }
            for (pid, name) in paths {
                if menu::row(ui, None, name, "").clicked() {
                    a.unlink();
                    a.link = if axis_x { AnimLink::PathX(*pid) } else { AnimLink::PathY(*pid) };
                    g.click();
                }
            }
        });
    }
    if !matches!(a.link, AnimLink::Expr(_)) && menu::row(ui, None, "Link to expression…", "").clicked() {
        a.unlink();
        a.link = AnimLink::Expr("return value".into());
        g.click();
    }
    if !a.link.is_none() && menu::row(ui, None, "Unlink", "").clicked() {
        a.unlink();
        g.click();
    }
}

/// A Polygon/Path mask is drawn from `points`; with fewer than 3 vertices the rasteriser reports
/// "outside everywhere" and the masked clip disappears with no way back. Seed the vertices from the
/// mask's own rect so every shape switch leaves something visible and editable.
pub(crate) fn seed_mask_points(m: &mut Mask) {
    if !matches!(m.shape, MaskShape::Polygon | MaskShape::Path) || m.points.len() >= 3 {
        return;
    }
    let (cx, cy) = (m.cx.value as f32, m.cy.value as f32);
    let (rx, ry) = ((m.rx.value as f32).abs().max(1.0), (m.ry.value as f32).abs().max(1.0));
    m.points = vec![(cx - rx, cy - ry), (cx + rx, cy - ry), (cx + rx, cy + ry), (cx - rx, cy + ry)];
}

/// Mask parameter grid, shared by the Inspector (the clip mask) and the Effects panel (a per-effect
/// mask): shape, enabled, invert, position/size/rotation, feather, expand, opacity - each keyframable.
pub(crate) fn mask_grid(ui: &mut egui::Ui, m: &mut Mask, lt: f64, palette: &Palette, g: &mut Gesture, salt: egui::Id) {
    Grid::new(salt).num_columns(2).show(ui, |ui| {
        ui.label("Shape");
        egui::ComboBox::from_id_salt(salt.with("shape")).selected_text(m.shape.name()).show_ui(ui, |ui| {
            for sh in MaskShape::ALL {
                g.note(&ui.selectable_value(&mut m.shape, sh, sh.name()));
            }
        });
        seed_mask_points(m);
        ui.end_row();
        ui.label("Enabled");
        g.note(&ui.checkbox(&mut m.enabled, ""));
        ui.end_row();
        ui.label("Invert");
        g.note(&ui.checkbox(&mut m.invert, ""));
        ui.end_row();
        // X/Y/radius are here too: a mask added from a panel has no viewport drag behind it and would
        // otherwise be stuck at its default rect in the middle of the layer.
        let rows: [(&str, f64, f64, f64); 8] = [
            ("X", -10000.0, 10000.0, 0.5),
            ("Y", -10000.0, 10000.0, 0.5),
            ("Radius X", 0.0, 10000.0, 0.5),
            ("Radius Y", 0.0, 10000.0, 0.5),
            ("Rotation", -360.0, 360.0, 0.5),
            ("Feather", 0.0, 500.0, 0.5),
            ("Expand", -500.0, 500.0, 0.5),
            ("Opacity", 0.0, 1.0, 0.01),
        ];
        for (label, lo, hi, speed) in rows {
            ui.label(label);
            ui.horizontal(|ui| {
                let a: &mut Animated = match label {
                    "X" => &mut m.cx,
                    "Y" => &mut m.cy,
                    "Radius X" => &mut m.rx,
                    "Radius Y" => &mut m.ry,
                    "Rotation" => &mut m.rotation,
                    "Feather" => &mut m.feather,
                    "Expand" => &mut m.expand,
                    _ => &mut m.opacity,
                };
                let mut v = a.at(lt);
                let r = ui.add(DragValue::new(&mut v).range(lo..=hi).speed(speed).clamp_existing_to_range(false));
                if r.changed() {
                    a.set_at(lt, v);
                }
                g.note(&r);
                menu::context(&r, |ui| key_menu(ui, a, lt, g, label, &[]));
                key_buttons(ui, a, lt, palette, g, label, &[]);
            });
            ui.end_row();
        }
    });
}

/// Drag-and-drop payload from the library / recent / planner panels to the timeline (egui dnd API).
#[derive(Clone, Debug)]
pub enum DragPayload {
    /// A library asset id.
    Asset(Id),
    /// A file path (recent panel, linked folders) - the timeline reports it back as a dropped file.
    Path(String),
    /// A nested timeline (Project.sequences) id.
    Sequence(Id),
    /// A saved template (Settings.templates) by name.
    Template(String),
    /// An effect from the effects panel (dropped on a clip or the node canvas).
    Effect(crate::model::EffectKind),
    /// A transition from the transitions panel (dropped on a cut or the node canvas).
    Transition(crate::model::TransitionKind),
}

/// How far the pointer must travel while held before a click turns into a drag.
const DRAG_SLOP: f32 = 6.0;

/// "The user really means to drag this": the primary button is down and the pointer has moved past
/// `DRAG_SLOP`. egui also promotes a *stationary* long press to a drag (`is_decidedly_dragging`), and
/// its `Sense::drag` widgets start dragging on the press of ANY button - which is how a right-click
/// used to pluck a card out of a panel instead of opening its menu.
pub(crate) fn drag_intent(ui: &egui::Ui) -> bool {
    ui.input(|i| {
        i.pointer.button_down(egui::PointerButton::Primary)
            && match (i.pointer.press_origin(), i.pointer.interact_pos()) {
                (Some(a), Some(b)) => a.distance(b) > DRAG_SLOP,
                _ => false,
            }
    })
}

// ---- ws:inspector-gallery ----
/// True once the pointer has sat over `response` for at least `ms` milliseconds (false, and the timer
/// reset, the instant it leaves). Shared by the effects/transitions catalogues and the Gallery cards so
/// a quick mouse pass-over doesn't spam `App.alt_render` with a decode it'll immediately discard.
/// Requests a repaint for the remaining time so the threshold fires even if the pointer stays still.
pub(crate) fn hover_after(ui: &egui::Ui, id: egui::Id, response: &Response, ms: f64) -> bool {
    let now = ui.input(|i| i.time);
    // `contains_pointer`, not `hovered`: same reasoning egui's own `dnd_hover_payload` gives (`hovered`
    // is false while ANY widget is being dragged, and can also miss a plain container `Response`).
    if !response.contains_pointer() {
        ui.ctx().data_mut(|d| d.remove::<f64>(id));
        return false;
    }
    let started = ui.ctx().data_mut(|d| *d.get_temp_mut_or_insert_with(id, || now));
    let elapsed_ms = (now - started) * 1000.0;
    if elapsed_ms >= ms {
        true
    } else {
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(((ms - elapsed_ms) / 1000.0).max(0.0)));
        false
    }
}

/// An item that is clickable and right-clickable first and a drag-and-drop source second: the payload
/// is only handed to egui once `drag_intent` holds, and the body is lifted under the cursor from that
/// moment (the one thing `Ui::dnd_drag_source` is good for). Use this instead of `dnd_drag_source`,
/// which senses drag only - grab cursor on hover, and a drag on any press.
pub(crate) fn drag_source<P: std::any::Any + Send + Sync>(
    ui: &mut egui::Ui,
    id: egui::Id,
    payload: P,
    contents: impl FnOnce(&mut egui::Ui),
) -> Response {
    let dragging = ui.ctx().is_being_dragged(id) && drag_intent(ui);
    let rect = if dragging {
        // paint the body into its own tooltip-order layer, then move that layer under the cursor
        let layer = egui::LayerId::new(egui::Order::Tooltip, id);
        let r = ui.scope_builder(egui::UiBuilder::new().layer_id(layer), contents).response;
        // translate by how far the pointer has travelled, NOT by (pointer - centre): snapping the body's
        // centre under the cursor makes anything grabbed off-centre jump the moment the drag starts.
        let (now, origin) = ui.ctx().input(|i| (i.pointer.interact_pos(), i.pointer.press_origin()));
        if let (Some(p), Some(o)) = (now, origin) {
            ui.ctx().transform_layer_shapes(layer, egui::emath::TSTransform::from_translation(p - o));
        }
        r.rect
    } else {
        ui.scope(contents).response.rect
    };
    let r = ui.interact(rect, id, egui::Sense::click_and_drag());
    if dragging {
        // re-set every frame of the drag: egui drops the payload on release, never mid-gesture
        egui::DragAndDrop::set_payload(ui.ctx(), payload);
    }
    r
}

/// Where a just-opened popup should appear: the click location if a click happened this frame
/// (button/menu item that opened it), otherwise the screen center (opened automatically - hotkey,
/// recovery, etc). Pair with `.pivot(egui::Align2::CENTER_CENTER)` so the popup centers on the point.
pub fn popup_open_pos(ctx: &egui::Context) -> egui::Pos2 {
    let clicked_at = ctx.input(|i| i.pointer.any_click().then(|| i.pointer.interact_pos()).flatten());
    clicked_at.unwrap_or_else(|| ctx.content_rect().center())
}

/// HH:MM:SS:FF timecode.
pub fn timecode(t: f64, fps: f64) -> String {
    let t = t.max(0.0);
    let fps = fps.max(1.0);
    let total_frames = (t * fps).round() as u64;
    let fpsr = fps.round() as u64;
    let f = total_frames % fpsr;
    let s = total_frames / fpsr;
    format!("{:02}:{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60, f)
}

// ---- ws:canvas-handles-monitor ----
/// The transport label's click-to-edit parser (and `playhead.set_timecode`): `hh:mm:ss:ff` /
/// `hh:mm:ss` / `mm:ss` / a bare number of seconds are absolute; `+N` / `-N` are frames from `cur`,
/// `+1.5s` / `-2s` seconds from `cur`. `None` for anything else - the edit is dropped, the playhead
/// stays put. Never negative (a relative step past the start clamps to 0).
pub(crate) fn parse_timecode(s: &str, fps: f64, cur: f64) -> Option<f64> {
    let s = s.trim();
    let fps = fps.max(1.0);
    if let Some(rest) = s.strip_prefix('+').or_else(|| s.strip_prefix('-')) {
        let sign = if s.starts_with('-') { -1.0 } else { 1.0 };
        let secs = match rest.trim().strip_suffix('s') {
            Some(secs) => secs.trim().parse::<f64>().ok()?,
            None => rest.trim().parse::<f64>().ok()? / fps,
        };
        return Some((cur + sign * secs).max(0.0)).filter(|t| t.is_finite());
    }
    let n: Vec<f64> = s.split(':').map(|p| p.trim().parse::<f64>().ok()).collect::<Option<_>>()?;
    let t = match n[..] {
        [secs] => secs,
        [m, s] => m * 60.0 + s,
        [h, m, s] => h * 3600.0 + m * 60.0 + s,
        [h, m, s, f] => h * 3600.0 + m * 60.0 + s + f / fps,
        _ => return None,
    };
    (t.is_finite() && t >= 0.0).then_some(t)
}

/// Short duration text "1:23.4".
pub fn duration_text(t: f64) -> String {
    let m = (t / 60.0).floor() as u64;
    let s = t - m as f64 * 60.0;
    format!("{m}:{s:04.1}")
}

/// Colour of a clip/asset/recent label index (0 or out of range = "no label" dim).
pub fn label_color(idx: u8, palette: &Palette) -> egui::Color32 {
    if idx == 0 || idx as usize > LABEL_COLORS.len() {
        palette.text_dim
    } else {
        let [r, g, b] = LABEL_COLORS[idx as usize - 1].1;
        egui::Color32::from_rgb(r, g, b)
    }
}

/// Name of a label index ("None" for 0 / out of range).
pub fn label_name(idx: u8) -> &'static str {
    if idx == 0 || idx as usize > LABEL_COLORS.len() {
        "None"
    } else {
        LABEL_COLORS[idx as usize - 1].0
    }
}

/// ComboBox over (value, label) pairs writing into a String setting. Returns true if the value changed.
pub fn combo(ui: &mut egui::Ui, id: &str, value: &mut String, options: &[(&str, &str)], width: Option<f32>) -> bool {
    let mut changed = false;
    let current = options.iter().find(|(v, _)| *v == value.as_str()).map(|(_, l)| *l).unwrap_or(value.as_str());
    let mut cb = egui::ComboBox::from_id_salt(id).selected_text(current);
    if let Some(w) = width {
        cb = cb.width(w);
    }
    cb.show_ui(ui, |ui| {
        for (v, label) in options {
            if ui.selectable_label(value.as_str() == *v, *label).clicked() && value.as_str() != *v {
                *value = (*v).to_string();
                changed = true;
            }
        }
    });
    changed
}

/// x264-style speed presets offered by the settings and export windows.
pub const ENCODER_PRESETS: [&str; 7] = ["ultrafast", "superfast", "veryfast", "faster", "fast", "medium", "slow"];

/// Encoder names worth offering: the h264 / hevc / vp9 / av1 families (software + nvenc/qsv/amf).
pub fn encoder_options(encoders: &[String]) -> Vec<&str> {
    const KEYS: [&str; 9] = ["264", "265", "hevc", "vp9", "av1", "x264", "x265", "nvenc", "qsv"];
    encoders.iter().map(String::as_str).filter(|e| KEYS.iter().any(|k| e.contains(k)) || e.contains("amf")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polygon_masks_never_stay_empty() {
        let mut m = Mask::default();
        seed_mask_points(&mut m);
        assert!(m.points.is_empty(), "a rect mask is drawn from cx/cy/rx/ry, not points");
        m.shape = MaskShape::Polygon;
        seed_mask_points(&mut m);
        // < 3 points rasterises as "outside everywhere": the masked clip would vanish
        assert_eq!(m.points.len(), 4, "{:?}", m.points);
        assert!(m.points.iter().any(|p| p.0 < 0.0) && m.points.iter().any(|p| p.0 > 0.0));
        m.points.push((5.0, 5.0));
        let kept = m.points.clone();
        seed_mask_points(&mut m);
        assert_eq!(m.points, kept, "an existing outline is never overwritten");
    }

    // ---- ws:canvas-handles-monitor ----
    #[test]
    fn parse_timecode_parses_every_form() {
        let near = |a: Option<f64>, b: f64| a.is_some_and(|a| (a - b).abs() < 1e-6);
        let fps = 30.0;
        assert!(near(parse_timecode("00:01:02:15", fps, 0.0), 62.5), "hh:mm:ss:ff");
        assert!(near(parse_timecode("01:02:03", fps, 0.0), 3723.0), "hh:mm:ss");
        assert!(near(parse_timecode("01:02", fps, 0.0), 62.0), "mm:ss");
        assert!(near(parse_timecode("7.5", fps, 0.0), 7.5), "bare seconds");
        assert!(near(parse_timecode("+48", fps, 10.0), 11.6), "+N frames");
        assert!(near(parse_timecode("-15", fps, 10.0), 9.5), "-N frames");
        assert!(near(parse_timecode("+1.5s", fps, 10.0), 11.5), "+N.Ns seconds");
        assert!(near(parse_timecode("-2s", fps, 10.0), 8.0), "-Ns seconds");
        assert!(near(parse_timecode("-2s", fps, 1.0), 0.0), "never negative");
        assert!(near(parse_timecode(" 01:02 ", fps, 0.0), 62.0), "whitespace tolerated");
        for junk in ["", "abc", "1:2:3:4:5", "+", "1:x", "-1", "+inf"] {
            let got = parse_timecode(junk, fps, 10.0);
            // "-1" is a legal "one frame back"; everything else is garbage
            if junk == "-1" {
                assert!(near(got, 10.0 - 1.0 / 30.0));
            } else {
                assert_eq!(got, None, "{junk:?} must not parse");
            }
        }
    }
}
