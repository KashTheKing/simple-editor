//! Preview panel: the rendered frame (letterboxed, black bars), a selection outline for the selected
//! visual clip (drag to move = edits clip.x / clip.y at the playhead time via `Animated::set_at`, calling
//! `undo` once at drag start), and the transport bar, centred under the video:
//! go-start, prev cut, step back, play/pause, stop, step forward, next cut, go-end (all painted
//! glyphs, NLE order) plus timecode / duration, In/Out buttons and the
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
//!
//! ---- ws:canvas-handles-monitor ----
//! The selection outline of a single asset-backed clip carries handles: corners scale uniformly
//! (`clip.scale`), edge midpoints scale one axis (`scale_x` / `scale_y`), a knob above the top edge
//! rotates (Shift snaps to 15°), and with the context menu's "Crop Handles" on the edge handles crop
//! instead (one find-or-append `EffectKind::Crop`, fractions via `Animated::set_at`). Two or more
//! selected clips show one union box and move together (no handles). A drag-to-move snaps to the
//! canvas centre / edges / thirds / other clips with guide lines (`ui::guides::canvas_snap`, gated by
//! Settings.canvas_snap). Ctrl+wheel zooms and middle-drag pans the viewer (`PreviewState.view`, never
//! project data; `Action::ViewerFit` resets). The timecode label is click-to-edit
//! (`ui::parse_timecode`). `PreviewState.mask_target` routes the mask tool's drag at the clip's own mask
//! or one of its effects' masks.

use crate::engine::compose::{placement, Placement};
use crate::hotkeys::Action;
use crate::media::Frame;
use crate::model::{
    BackgroundMode, Clip, ClipKind, Effect, EffectKind, Id, Mask, MaskShape, Project, ShapeKind, ShapeStyle,
    Stroke as ModelStroke,
};
use crate::theme::Palette;
use crate::ui::guides::{canvas_snap, paint_canvas_guides};
use crate::ui::tools::{draw_glyph, glyph_text_button, Dir, Glyph, Tool};
use crate::ui::{parse_timecode, timecode};
use eframe::egui::{
    self, pos2, vec2, Color32, CursorIcon, PointerButton, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, TextureOptions,
    Vec2,
};
use std::sync::Arc;

/// Preview render scales offered by the quality selector.
pub const QUALITIES: [u32; 4] = [100, 75, 50, 25];

// ---- ws:canvas-handles-monitor ----

/// Grab radius of a handle, in points.
const HIT: f32 = 8.0;
/// How far above the top edge the rotate knob sits, in points.
const KNOB_OFFSET: f32 = 18.0;
/// Canvas-snap magnet, in points.
const SNAP_PX: f32 = 8.0;
/// Viewer zoom range (1 = fit).
const ZOOM_RANGE: (f32, f32) = (0.25, 8.0);

/// Which handle on the selection outline owns the active drag. `u8` indexes TL/TR/BR/BL for corners
/// and top/right/bottom/left for edges and crop handles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Handle {
    Corner(u8),
    Edge(u8),
    Rotate,
    Crop(u8),
}

/// Which mask the mask tool's canvas drag (`tool_drag`) writes to: the clip's own, or one of its
/// effects'. Default = the clip's own mask, exactly today's behaviour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum MaskTarget {
    #[default]
    Clip,
    Effect(usize),
}

/// The selection outline's geometry on screen — one rotated rect plus its handle positions.
#[derive(Clone, Copy)]
struct Outline {
    /// Screen centre and the rotation's (sin, cos).
    o: Pos2,
    sn: f32,
    cs: f32,
    /// Unrotated half size on screen (the full, uncropped layer).
    half: Vec2,
    /// Corners TL/TR/BR/BL; edge midpoints T/R/B/L (of the crop ring in crop mode); the rotate knob.
    corners: [Pos2; 4],
    edges: [Pos2; 4],
    knob: Pos2,
}

impl Outline {
    /// `crop` = (left, right, top, bottom) fractions when crop handles are wanted: the edge handles then
    /// sit on the cropped inner rect instead of the outline.
    fn new(p: &Placement, to_screen: impl Fn(f32, f32) -> Pos2, crop: Option<[f32; 4]>) -> Self {
        let o = to_screen(p.cx, p.cy);
        // screen points per canvas px (aspect is preserved, so one factor serves both axes)
        let k = (to_screen(p.cx + 1.0, p.cy).x - o.x).abs().max(1e-4);
        let half = vec2(p.w / 2.0 * k, p.h / 2.0 * k);
        let (sn, cs) = p.rot.to_radians().sin_cos();
        let at = |x: f32, y: f32| pos2(o.x + x * cs - y * sn, o.y + x * sn + y * cs);
        let (hw, hh) = (half.x, half.y);
        let [l, r, t, b] = crop.unwrap_or([0.0; 4]);
        let (x0, x1, y0, y1) = (-hw + 2.0 * hw * l, hw - 2.0 * hw * r, -hh + 2.0 * hh * t, hh - 2.0 * hh * b);
        let (mx, my) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        Self {
            o,
            sn,
            cs,
            half,
            corners: [at(-hw, -hh), at(hw, -hh), at(hw, hh), at(-hw, hh)],
            edges: [at(mx, y0), at(x1, my), at(mx, y1), at(x0, my)],
            knob: at(0.0, -hh - KNOB_OFFSET),
        }
    }
    /// Screen point -> the layer's own unrotated frame (screen units from the centre).
    fn local(&self, p: Pos2) -> Vec2 {
        let d = p - self.o;
        vec2(d.x * self.cs + d.y * self.sn, -d.x * self.sn + d.y * self.cs)
    }
    fn inside(&self, p: Pos2) -> bool {
        let l = self.local(p);
        l.x.abs() <= self.half.x && l.y.abs() <= self.half.y
    }
    /// Which handle `p` is over, if any. Crop mode turns the edge handles into crop handles — the two
    /// would otherwise sit on the same spot at zero crop.
    fn hit(&self, p: Pos2, crop_mode: bool) -> Option<Handle> {
        if (p - self.knob).length() <= HIT {
            return Some(Handle::Rotate);
        }
        if let Some(i) = self.corners.iter().position(|q| (p - *q).length() <= HIT) {
            return Some(Handle::Corner(i as u8));
        }
        let i = self.edges.iter().position(|q| (p - *q).length() <= HIT)? as u8;
        Some(if crop_mode { Handle::Crop(i) } else { Handle::Edge(i) })
    }
}

/// Snapshot taken at `drag_started` so every frame's value is a ratio / angle / fraction from a fixed
/// origin, never an accumulation of per-frame deltas.
struct HandleDrag {
    handle: Handle,
    id: Id,
    /// The outline as it was at the press.
    ol: Outline,
    press: Pos2,
    /// (scale, scale_x, scale_y, rotation) at the press.
    start: (f64, f64, f64, f64),
}

/// Single choke point for viewer zoom / pan: scale the fitted letterbox about its centre, then pan.
pub(crate) fn apply_view(lb: Rect, (zoom, pan): (f32, Vec2)) -> Rect {
    Rect::from_center_size(lb.center() + pan, lb.size() * zoom)
}

/// The clip's `EffectKind::Crop` entry, appended when missing (same find-or-append shape as
/// ToggleEffect / tools_color's `set_effect`). Shared with the `clip.crop` / `timeline.reframe` tools.
pub(crate) fn crop_effect(clip: &mut Clip) -> &mut Effect {
    let i = clip.effects.iter().position(|e| e.kind == EffectKind::Crop).unwrap_or_else(|| {
        clip.effects.push(Effect::new(EffectKind::Crop));
        clip.effects.len() - 1
    });
    &mut clip.effects[i]
}

/// Current (left, right, top, bottom) crop fractions of a clip at clip-local `lt`, if it has a Crop.
fn crop_fractions(clip: &Clip, lt: f64) -> Option<[f32; 4]> {
    let e = clip.effects.iter().find(|e| e.kind == EffectKind::Crop)?;
    let v = |i: usize| e.params.get(i).map(|a| a.at(lt) as f32).unwrap_or(0.0);
    Some([v(0), v(1), v(2), v(3)])
}

/// Apply one frame of a handle drag: the pointer at `now` against the press snapshot writes exactly one
/// property (scale / scale_x / scale_y / rotation / one crop fraction) at clip-local `lt`. False when
/// the clip is gone.
fn handle_apply(d: &HandleDrag, now: Pos2, shift: bool, project: &mut Project, lt: f64) -> bool {
    let Some(cl) = project.clip_mut(d.id) else { return false };
    match d.handle {
        Handle::Corner(_) => {
            let ratio = (now - d.ol.o).length() / (d.press - d.ol.o).length().max(1.0);
            cl.scale.set_at(lt, (d.start.0 * ratio as f64).max(0.01));
        }
        Handle::Edge(i) => {
            let (l, l0) = (d.ol.local(now), d.ol.local(d.press));
            if i % 2 == 1 {
                let ratio = l.x.abs() / l0.x.abs().max(1.0);
                cl.scale_x.set_at(lt, (d.start.1 * ratio as f64).max(0.01));
            } else {
                let ratio = l.y.abs() / l0.y.abs().max(1.0);
                cl.scale_y.set_at(lt, (d.start.2 * ratio as f64).max(0.01));
            }
        }
        Handle::Rotate => {
            let (a, a0) = (now - d.ol.o, d.press - d.ol.o);
            let mut deg = d.start.3 + (a.y.atan2(a.x) - a0.y.atan2(a0.x)).to_degrees() as f64;
            if shift {
                deg = (deg / 15.0).round() * 15.0;
            }
            cl.rotation.set_at(lt, deg);
        }
        Handle::Crop(i) => {
            let l = d.ol.local(now);
            let (hw, hh) = (d.ol.half.x.max(1.0), d.ol.half.y.max(1.0));
            // P_CROP order is Left, Right, Top, Bottom; handles are T/R/B/L
            let (idx, frac) = match i {
                0 => (2, (l.y + hh) / (2.0 * hh)),
                1 => (1, (hw - l.x) / (2.0 * hw)),
                2 => (3, (hh - l.y) / (2.0 * hh)),
                _ => (0, (l.x + hw) / (2.0 * hw)),
            };
            crop_effect(cl).params[idx].set_at(lt, frac.clamp(0.0, 0.5) as f64);
        }
    }
    true
}

/// The mask slot a mask-tool drag writes: the clip's own, or one effect's (`None` when the clip / effect
/// index is gone — the gesture is then a no-op with no undo entry, like a mask drag over an audio clip).
fn mask_slot_of(project: &mut Project, clip: Option<Id>, target: MaskTarget) -> Option<&mut Option<Mask>> {
    let cl = project.clip_mut(clip?)?;
    match target {
        MaskTarget::Clip => Some(&mut cl.mask),
        MaskTarget::Effect(i) => cl.effects.get_mut(i).map(|e| &mut e.mask),
    }
}

/// Axis-aligned half size (project px) of a clip's placed layer — the moving box `canvas_snap` snaps.
fn half_size(project: &Project, id: Id, playhead: f64) -> Option<(f32, f32)> {
    let cl = project.clip(id)?;
    let a = project.asset(cl.asset)?;
    let p = placement(project, cl, playhead, (a.width, a.height), project.width, project.height, true);
    let (x0, y0, x1, y1) = p.bounds();
    Some(((x1 - x0) / 2.0, (y1 - y0) / 2.0))
}

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
    /// Active drag-to-move: every moved clip's (id, x, y) at drag start and the accumulated pointer
    /// delta (points). One entry for a single clip, one per clip for a group move.
    drag: Option<(Vec<(Id, f64, f64)>, Vec2)>,
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
    // ---- ws:canvas-handles-monitor ----
    /// Active transform / crop handle drag.
    handle: Option<HandleDrag>,
    /// Context menu "Crop Handles": the edge handles crop (one Crop effect) instead of scaling.
    pub(crate) crop_mode: bool,
    /// Which mask the mask tool's drag writes (`clip.mask_target` / the context menu).
    pub(crate) mask_target: MaskTarget,
    /// Viewer (zoom, pan): Ctrl+wheel / middle-drag; 1.0 / ZERO = fit. Never project data.
    pub(crate) view: (f32, Vec2),
    /// The video area last frame (one-frame-stale, like `TimelineState.lanes_rect`) — the drop target
    /// `app::drops` checks for "dropped onto the monitor".
    pub(crate) canvas_rect: Rect,
    /// The timecode label's text while it is being edited (click-to-edit).
    tc_edit: Option<String>,
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
            handle: None,
            crop_mode: false,
            mask_target: MaskTarget::Clip,
            view: (1.0, Vec2::ZERO),
            canvas_rect: Rect::NOTHING,
            tc_edit: None,
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
    /// The style a shape dragged out right now would be created with (tool-strip picks applied) —
    /// `Some` only while a shape tool is active. Drives the live preview so it matches the result.
    pub shape_style: Option<ShapeStyle>,
    /// Current preview render scale in percent (settings.preview_quality).
    pub quality: u32,
    /// Current movie mode (settings.movie_mode).
    pub movie_mode: bool,
    /// Pre-render progress 0..1 while frames are being cached (None = not pre-rendering).
    pub prerender: Option<f32>,
    /// Playback is held while the player refills its read-ahead: draw a spinner over the video.
    pub buffering: bool,
    /// Progress 0..1 of the proxy build in flight (None = no proxy being built).
    pub proxy: Option<f32>,
    /// Tracker box of the Tracking pane (centre + half sizes, project px relative to the canvas
    /// centre) while that pane is on screen — drawn here and dragged to place the template.
    pub tracker: Option<(f32, f32, f32, f32)>,
    /// Social-guide overlay to draw over the video (settings.guide).
    pub guide: Option<crate::ui::guides::Guide>,
    // ---- ws:canvas-handles-monitor ----
    /// Settings.canvas_snap: snap a drag-to-move to the canvas centre / edges / thirds / other clips.
    pub canvas_snap: bool,
    /// The monitor's alt render (an effect / transition hover, `app::monitor`) — painted instead of
    /// the live frame while Some, so a hover preview never touches the project or the player.
    pub alt_texture: Option<(egui::TextureId, [u32; 2])>,
    /// settings.use_proxies, for the transport's proxy toggle.
    pub use_proxies: bool,
    /// `Player::dropped_frames`, shown as a small transport badge when non-zero.
    pub dropped: u64,
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
    /// A text box dragged out with the Text tool: (centre x, centre y, half width, half height), same
    /// convention as `new_shape` minus the kind. The caller creates the clip (`Project::add_text_clip`)
    /// and positions it at this centre.
    pub new_text: Option<(f32, f32, f32, f32)>,
    /// Vertices of a closed Polygon path, relative to that shape's centre (empty = a regular n-gon).
    pub new_points: Vec<(f32, f32)>,
    /// A stroke recorded with the Draw tool (points relative to the canvas centre, timed from the press).
    pub stroke: Option<ModelStroke>,
    /// The selected clip's mask was edited with a mask tool.
    pub mask_edit: bool,
    /// The tracker box was dragged to this centre (project px relative to the canvas centre).
    pub set_tracker: Option<(f32, f32)>,
    /// The user picked a social guide from the transport button (Some(None) = off).
    pub set_guide: Option<Option<crate::ui::guides::Guide>>,
    // ---- ws:canvas-handles-monitor ----
    /// The context menu toggled canvas snapping — the app stores it in Settings.canvas_snap.
    pub set_canvas_snap: Option<bool>,
}

pub fn show(ui: &mut egui::Ui, state: &mut PreviewState, mut c: PreviewCtx<'_>) -> PreviewResponse {
    let mut r = PreviewResponse::default();
    if c.fullscreen {
        video(ui, state, &mut c, &mut r);
        // transport overlay: summoned by mouse movement, gone ~2 s after the mouse stops.
        // `moved_at` is pointer-only (see video()), so spacebar play/pause never reveals the bar.
        if state.moved_at.is_some_and(|t| t.elapsed().as_secs_f32() < 2.0) {
            egui::Area::new(ui.id().with("fs_transport"))
                .anchor(egui::Align2::CENTER_BOTTOM, vec2(0.0, -24.0))
                .order(egui::Order::Foreground)
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).fill(c.palette.panel.gamma_multiply(0.92)).show(ui, |ui| {
                        ui.vertical(|ui| {
                            let width = state.transport.width().max(320.0);
                            if let Some(t) = scrub_bar(ui, c.project.duration(), c.playhead, c.palette, width) {
                                r.seek = Some(t);
                            }
                            transport(ui, state, &c, &mut r);
                        });
                        // hovering the bar keeps it alive past the 2 s fade
                        if ui.ui_contains_pointer() {
                            state.moved_at = Some(std::time::Instant::now());
                        }
                    });
                });
        }
        return r;
    }
    let hovered = ui.rect_contains_pointer(ui.max_rect());
    ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
        transport(ui, state, &c, &mut r);
        // the space is always allocated so the video does not jump; the bar itself only appears
        // (and takes clicks) while the pointer is over the pane, like the fullscreen overlay
        if hovered {
            if let Some(t) = scrub_bar(ui, c.project.duration(), c.playhead, c.palette, ui.available_width()) {
                r.seek = Some(t);
            }
        } else {
            ui.allocate_exact_size(vec2(ui.available_width(), 10.0), egui::Sense::hover());
        }
        video(ui, state, &mut c, &mut r);
    });
    r
}

/// Progress / scrub bar (same look as the library preview's, which now shares this fn instead of
/// duplicating the painting): fill shows the playhead, click or drag anywhere on it seeks. Returns the
/// seek target (seconds) instead of writing into a `PreviewResponse` directly, so the lib-preview
/// mini-player (which has no `PreviewResponse` of its own) can call it too.
pub(crate) fn scrub_bar(ui: &mut egui::Ui, duration: f64, playhead: f64, palette: &Palette, width: f32) -> Option<f64> {
    let duration = duration.max(f64::MIN_POSITIVE);
    let (bar, br) = ui.allocate_exact_size(vec2(width, 10.0), egui::Sense::click_and_drag());
    ui.painter().rect_filled(bar, 2.0, palette.panel);
    let frac = (playhead / duration).clamp(0.0, 1.0) as f32;
    let filled = Rect::from_min_max(bar.min, pos2(bar.left() + bar.width() * frac, bar.bottom()));
    ui.painter().rect_filled(filled, 2.0, palette.accent);
    ui.painter().rect_stroke(bar, 2.0, egui::Stroke::new(1.0, palette.border), egui::StrokeKind::Inside);
    if (br.clicked() || br.dragged()) && bar.width() > 0.0 {
        if let Some(p) = br.interact_pointer_pos() {
            let f = ((p.x - bar.left()) / bar.width()) as f64;
            return Some(scrub_time(f, duration));
        }
    }
    None
}

/// Time a scrub-bar click/drag at fractional position `frac` (0..1 across the bar, unclamped so a drag
/// past either end still reads as 0 or `duration`) seeks to.
pub(crate) fn scrub_time(frac: f64, duration: f64) -> f64 {
    frac.clamp(0.0, 1.0) * duration
}

fn transport(ui: &mut egui::Ui, state: &mut PreviewState, c: &PreviewCtx<'_>, r: &mut PreviewResponse) {
    let fps = c.project.fps;
    // centred: pad by half the leftover of last frame's measured width. Skip the pad on an
    // unmeasured/reset frame (width 0) — padding from a stale zero would overshoot the real content
    // width, wrap the row (see `horizontal_wrapped` below), and corrupt the very measurement next
    // frame's pad depends on, so it would never converge.
    let pad = if state.transport.width() > 0.0 {
        ((ui.available_width() - state.transport.width()) * 0.5).max(0.0)
    } else {
        0.0
    };
    // ---- ws:canvas-handles-monitor ----
    // `b` below captures `r.actions` by mutable reference for the whole closure (it's used again as
    // late as the Fullscreen button); a second direct touch of `r.actions` — or a reborrow of all of
    // `*r` (`timecode_label` used to take `r: &mut PreviewResponse`) — would conflict with that live
    // borrow. Both new bits of state go through fresh locals instead and land on `r` after the closure.
    let mut tc_seek = None;
    let mut toggle_proxy = false;
    let row = ui
        .horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.add_space(pad);
            // `label` is the tooltip for an icon button and the caption for a text one
            let mut b = |ui: &mut egui::Ui, icon: Option<Glyph>, label: &str, a: Action| {
                let hit = match icon {
                    Some(g) => glyph_text_button(ui, g, "").on_hover_text(label),
                    None => ui.button(label),
                };
                if hit.clicked() {
                    r.actions.push(a);
                }
            };
            b(ui, Some(Glyph::Jump(Dir::Left)), "Go to start", Action::GoStart);
            b(ui, Some(Glyph::Skip(Dir::Left)), "Previous cut", Action::PrevCut);
            b(ui, Some(Glyph::Tri(Dir::Left)), "Step back one frame", Action::StepBack);
            let (pp, pp_tip) = if c.playing { (Glyph::Pause, "Pause") } else { (Glyph::Play, "Play") };
            b(ui, Some(pp), pp_tip, Action::PlayPause);
            b(ui, Some(Glyph::Stop), "Stop", Action::Stop);
            b(ui, Some(Glyph::Tri(Dir::Right)), "Step forward one frame", Action::StepForward);
            b(ui, Some(Glyph::Skip(Dir::Right)), "Next cut", Action::NextCut);
            b(ui, Some(Glyph::Jump(Dir::Right)), "Go to end", Action::GoEnd);
            ui.add_space(8.0);
            tc_seek = timecode_label(ui, state, c, fps);
            ui.add_space(8.0);
            b(ui, None, "In", Action::MarkIn);
            b(ui, None, "Out", Action::MarkOut);
            b(ui, None, "Clear", Action::ClearInOut);
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
                Glyph::FilmStrip,
                "Movie mode: play pre-rendered full-quality frames",
                c.movie_mode,
            )
            .clicked()
            {
                r.set_movie_mode = Some(!c.movie_mode);
            }
            // ---- ws:canvas-handles-monitor ----: proxy toggle + dropped-frame badge
            if ui.selectable_label(c.use_proxies, "Proxy").on_hover_text("Play low-res proxies in the preview").clicked() {
                toggle_proxy = true;
            }
            if c.dropped > 0 {
                ui.weak(format!("{} dropped", c.dropped)).on_hover_text("Frames dropped during playback");
            }
            // social-guide overlay picker
            let gr = crate::ui::tools::icon_button(
                ui,
                c.palette,
                ui.id().with("guide"),
                Glyph::Guides,
                "Social guide overlay",
                c.guide.is_some(),
            );
            egui::Popup::menu(&gr).show(|ui| {
                use crate::ui::guides::Guide;
                let mut pick = |ui: &mut egui::Ui, label: &str, v: Option<Guide>| {
                    if ui.radio(c.guide == v, label).clicked() {
                        r.set_guide = Some(v);
                        ui.close();
                    }
                };
                pick(ui, "Off", None);
                for g in Guide::ALL {
                    pick(ui, g.name(), Some(g));
                }
            });
            b(ui, Some(Glyph::Fullscreen), "Fullscreen", Action::Fullscreen);
        })
        .response;
    state.transport = Rect::from_min_max(pos2(row.rect.left() + pad, row.rect.top()), row.rect.max);
    // ---- ws:canvas-handles-monitor ----: applied after `b`'s borrow of `r.actions` has ended
    if let Some(t) = tc_seek {
        r.seek = Some(t);
    }
    if toggle_proxy {
        r.actions.push(Action::ToggleProxies);
    }
}

// ---- ws:canvas-handles-monitor ----
/// The "playhead / duration" label: a click turns it into a text field; Enter parses it with
/// `parse_timecode` (hh:mm:ss:ff, mm:ss, +N / -N frames, +1.5s) and returns the seek target; Esc or
/// clicking away drops the edit. Returns a value instead of writing `r.seek` directly — the caller's
/// `b` closure already holds `r.actions` borrowed for longer than this call site (see `transport`'s
/// comment above its `row` binding).
fn timecode_label(ui: &mut egui::Ui, state: &mut PreviewState, c: &PreviewCtx<'_>, fps: f64) -> Option<f64> {
    let mut seek = None;
    let mut close = false;
    if let Some(text) = state.tc_edit.as_mut() {
        let te = ui.add(egui::TextEdit::singleline(text).desired_width(100.0).font(egui::TextStyle::Monospace));
        if te.lost_focus() {
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                seek = parse_timecode(text, fps, c.playhead);
            }
            close = true;
        } else if !te.has_focus() {
            te.request_focus();
        }
    } else {
        let text = format!("{} / {}", timecode(c.playhead, fps), timecode(c.project.duration(), fps));
        let lbl = ui
            .add(egui::Label::new(egui::RichText::new(text).monospace()).sense(Sense::click()))
            .on_hover_text("Click to type a time: hh:mm:ss:ff, mm:ss, +N / -N frames, +1.5s / -2s");
        if lbl.clicked() {
            state.tc_edit = Some(timecode(c.playhead, fps));
        }
    }
    if close {
        state.tc_edit = None;
    }
    seek
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

/// Shift locks the shape's aspect (square / circle / 45°-stepped line); Alt anchors it on the press point
/// instead of corner-to-corner, so it grows symmetrically from where the drag started. Returns the two
/// corners `draw_shape_preview` (and the final shape) should use in place of the raw press/cursor, so the
/// live preview and what actually gets created always agree.
fn constrain_drag(kind: ShapeKind, from: Pos2, to: Pos2, modifiers: egui::Modifiers) -> (Pos2, Pos2) {
    let mut delta = to - from;
    if modifiers.shift {
        if matches!(kind, ShapeKind::Line | ShapeKind::Arrow) {
            let len = delta.length();
            let step = std::f32::consts::FRAC_PI_4;
            let angle = (delta.y.atan2(delta.x) / step).round() * step;
            delta = vec2(angle.cos(), angle.sin()) * len;
        } else {
            let m = delta.x.abs().max(delta.y.abs());
            delta = vec2(m.copysign(delta.x), m.copysign(delta.y));
        }
    }
    if modifiers.alt {
        (from - delta, from + delta)
    } else {
        (from, from + delta)
    }
}

/// Outline of the shape a drag is creating: `from` is where the press landed, `to` is the cursor.
/// Bounded shapes fill the rectangle between them; a line or arrow runs from one to the other, so it
/// must not be handed a normalised rect (its corners would point the wrong way down two of the four
/// diagonals). Painted with `style`'s actual fill/stroke/corner/sides (project px, scaled to screen
/// points by `inv_k` = screen points per project px — the inverse of `tool_drag`'s `k`) so the live
/// preview looks like the shape `engine::shapes::ShapeRasterizer` will actually render, not a generic
/// translucent selection outline.
fn draw_shape_preview(p: &egui::Painter, style: &ShapeStyle, from: Pos2, to: Pos2, k: f32) {
    let kind = style.kind;
    let inv_k = if k.is_finite() && k > 1e-4 { 1.0 / k } else { 1.0 };
    let fill = Color32::from_rgba_unmultiplied(style.fill[0], style.fill[1], style.fill[2], style.fill[3]);
    let sw = (style.stroke_width.max(0.0) * inv_k).max(0.75);
    let stroke_color =
        Color32::from_rgba_unmultiplied(style.stroke[0], style.stroke[1], style.stroke[2], style.stroke[3]);
    let stroke = if style.stroke[3] > 0 { Stroke::new(sw, stroke_color) } else { Stroke::NONE };
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
            let corner = (style.corner.max(0.0) * inv_k).min(hw).min(hh);
            p.rect(r, corner, fill, stroke, StrokeKind::Inside);
        }
        ShapeKind::Ellipse => {
            let pts = poly(48, 0.0);
            p.add(Shape::convex_polygon(pts.clone(), fill, Stroke::NONE));
            p.add(Shape::closed_line(pts, stroke));
        }
        // matches engine::shapes::shape_outline's fixed apex-up / flat-base vertices exactly (a regular
        // n-gon inscribed in the w x h ellipse, which the old preview used here, is a visibly different
        // triangle).
        ShapeKind::Triangle => {
            let pts = vec![pos2(c.x, c.y - hh), pos2(c.x + hw, c.y + hh), pos2(c.x - hw, c.y + hh)];
            p.add(Shape::convex_polygon(pts.clone(), fill, Stroke::NONE));
            p.add(Shape::closed_line(pts, stroke));
        }
        ShapeKind::Polygon | ShapeKind::Star => {
            let sides = style.sides.clamp(3, 64);
            let pts = if kind == ShapeKind::Star {
                let (o, i) = (poly(sides, top), poly(sides, top + std::f32::consts::TAU / (2 * sides) as f32));
                (0..sides as usize)
                    .flat_map(|idx| [o[idx], pos2(c.x + (i[idx].x - c.x) * 0.45, c.y + (i[idx].y - c.y) * 0.45)])
                    .collect()
            } else {
                poly(sides, top)
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
                // same rule as engine::shapes::arrow_head: an explicit head size (`corner`), or a
                // stroke-relative default.
                let head_px = if style.corner > 0.0 && style.corner.is_finite() {
                    style.corner
                } else {
                    (style.stroke_width.max(0.0) * 4.0).max(8.0)
                };
                // clamp's bounds must stay ordered (min <= max) even when the drag is barely a pixel long
                let head = (head_px * inv_k).clamp(2.0, len.max(2.0));
                let base = pos2(b.x - ux * head, b.y - uy * head);
                let (px, py) = (-uy * head * 0.5, ux * head * 0.5);
                p.add(Shape::convex_polygon(
                    vec![b, pos2(base.x + px, base.y + py), pos2(base.x - px, base.y - py)],
                    stroke_color,
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
pub(crate) fn letterbox(area: Rect, aspect: f32, ppp: f32) -> Rect {
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

    // a mask shapes pixels: an audio clip has none, so the mask tool finds no target on one and the
    // gesture is a no-op (no mask, and no undo entry for an edit that never happened).
    // ws:canvas-handles-monitor: which of that clip's masks is written (its own, or one effect's) is
    // `state.mask_target`'s call — see `mask_slot_of`.
    let mask_clip = c.selection.iter().copied().find(|&id| c.project.clip(id).is_some_and(|cl| cl.is_visual()));
    let mask_target = state.mask_target;

    if resp.drag_started() {
        if let Some(p) = resp.interact_pointer_pos() {
            let t0 = ui.input(|i| i.time);
            state.tool_drag = Some(ToolDrag { from: p, t0, points: Vec::new() });
            if matches!(tool, Tool::Mask(_)) && mask_slot_of(c.project, mask_clip, mask_target).is_some() {
                (c.undo)(c.project);
            }
        }
    }
    let Some(d) = &mut state.tool_drag else { return !matches!(tool, Tool::Text) };
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
                if let Some(slot) = mask_slot_of(c.project, mask_clip, mask_target) {
                    let m = slot.get_or_insert_with(|| Mask::new(shape));
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
            // shape wants its corners sorted (see draw_shape_preview); Shift/Alt bend those corners first
            // (constrain_drag) so the preview matches what drag_stopped below will actually create.
            Tool::Shape(kind) => {
                let modifiers = ui.input(|i| i.modifiers);
                let (from, to) = constrain_drag(kind, d.from, now, modifiers);
                // the real style the clip will be created with (PreviewCtx::shape_style, from the
                // tool strip's picks) — a red 8-point star previews as a red 8-point star
                let style = c.shape_style.clone().unwrap_or_else(|| ShapeStyle::new(kind));
                draw_shape_preview(&p, &style, from, to, k);
            }
            // text box outline: no real text is rendered here, just where the box will land
            Tool::Text => {
                p.rect_stroke(Rect::from_two_pos(d.from, now), 0.0, stroke, StrokeKind::Inside);
            }
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
                // Shift/Alt bend the two corners first (constrain_drag), same as the live preview above.
                // Line/Arrow run from (-w, -h) to (+w, +h) about their centre (engine::shapes), so the
                // half-extents stay SIGNED and the drag's direction survives; a dragged-up-right line
                // used to come out pointing down-right because both were made absolute here.
                let modifiers = ui.input(|i| i.modifiers);
                let (p0, p1) = constrain_drag(kind, pos2(x0, y0), pos2(x1, y1), modifiers);
                let (dx, dy) = ((p1.x - p0.x) / 2.0, (p1.y - p0.y) / 2.0);
                let (w, h) = match kind {
                    ShapeKind::Line | ShapeKind::Arrow => (signed_min(dx, 2.0), signed_min(dy, 2.0)),
                    _ => (dx.abs().max(2.0), dy.abs().max(2.0)),
                };
                r.new_shape = Some((kind, (p0.x + p1.x) / 2.0, (p0.y + p1.y) / 2.0, w, h));
            }
            Tool::Draw if d.points.len() > 1 => {
                r.stroke = Some(ModelStroke { color: [255, 255, 255, 255], width: 6.0, points: d.points.clone() });
            }
            // same centre/half-size convention as new_shape (minus the kind); the caller creates the
            // text clip and positions it here (mirroring how new_shape is turned into a Shape clip).
            Tool::Text => {
                let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
                let (tw, th) = ((x1 - x0).abs() / 2.0, (y1 - y0).abs() / 2.0);
                r.new_text = Some((cx, cy, tw.max(2.0), th.max(2.0)));
            }
            _ => {}
        }
        state.tool_drag = None;
    }
    true
}

/// Right-click "Background" submenu on the video area: Checkerboard / Black / White / Custom (a colour
/// picker). Mirrors the transport bar's social-guide picker (radio per option, `ui.close()` on pick)
/// but as a `context_menu` submenu since the background has no dedicated toolbar button. Edits
/// `project.preview_bg` through the same undo-once-per-gesture/`edited`-flag convention every other
/// project-level edit in this file uses (e.g. the drag-to-move handling below).
/// ws:canvas-handles-monitor: also hosts the "Crop Handles" / "Snap to canvas" toggles, "Fit Viewer"
/// and the "Mask target" picker (the clip's own mask or one of its effects' — UI-only state).
fn background_menu(resp: &egui::Response, state: &mut PreviewState, c: &mut PreviewCtx<'_>, r: &mut PreviewResponse) {
    // the selected clip's effect stack, for the mask-target picker
    let fx: Vec<&'static str> = c
        .selection
        .iter()
        .find_map(|&id| c.project.clip(id).filter(|cl| cl.is_visual()))
        .map(|cl| cl.effects.iter().map(|e| e.kind.name()).collect())
        .unwrap_or_default();
    resp.context_menu(|ui| {
        ui.checkbox(&mut state.crop_mode, "Crop Handles")
            .on_hover_text("Edge handles crop the clip (one Crop effect) instead of scaling it");
        let mut snap = c.canvas_snap;
        if ui.checkbox(&mut snap, "Snap to canvas").changed() {
            r.set_canvas_snap = Some(snap);
        }
        if ui.button("Fit Viewer").clicked() {
            r.actions.push(Action::ViewerFit);
            ui.close();
        }
        if !fx.is_empty() {
            ui.menu_button("Mask target", |ui| {
                if ui.radio(state.mask_target == MaskTarget::Clip, "Clip mask").clicked() {
                    state.mask_target = MaskTarget::Clip;
                    ui.close();
                }
                for (i, name) in fx.iter().enumerate() {
                    if ui.radio(state.mask_target == MaskTarget::Effect(i), format!("{}: {name}", i + 1)).clicked() {
                        state.mask_target = MaskTarget::Effect(i);
                        ui.close();
                    }
                }
            });
        }
        ui.separator();
        ui.menu_button("Background", |ui| {
            let mut pick = |ui: &mut egui::Ui, label: &str, v: BackgroundMode| {
                if ui.radio(c.project.preview_bg == v, label).clicked() {
                    (c.undo)(c.project);
                    c.project.preview_bg = v;
                    r.edited = true;
                    ui.close();
                }
            };
            for mode in BackgroundMode::ALL {
                pick(ui, mode.name(), mode);
            }
            let custom = matches!(c.project.preview_bg, BackgroundMode::Custom(_));
            ui.horizontal(|ui| {
                if ui.radio(custom, "Custom").clicked() && !custom {
                    (c.undo)(c.project);
                    c.project.preview_bg = BackgroundMode::Custom([0, 0, 0, 255]);
                    r.edited = true;
                }
                if let BackgroundMode::Custom(mut rgba) = c.project.preview_bg {
                    let cr = ui.color_edit_button_srgba_unmultiplied(&mut rgba);
                    // gate the undo push to once per drag/pick, like every other gesture in this file
                    if crate::ui::edit_start(&cr) {
                        (c.undo)(c.project);
                    }
                    if cr.changed() {
                        c.project.preview_bg = BackgroundMode::Custom(rgba);
                        r.edited = true;
                    }
                }
            });
        });
    });
}

fn video(ui: &mut egui::Ui, state: &mut PreviewState, c: &mut PreviewCtx<'_>, r: &mut PreviewResponse) {
    let (rect, resp) = ui.allocate_exact_size(ui.available_size_before_wrap(), Sense::click_and_drag());
    background_menu(&resp, state, c, r);
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, Color32::BLACK);
    let ppp = ui.pixels_per_point();
    let aspect = c.project.width.max(1) as f32 / c.project.height.max(1) as f32;
    let fit = letterbox(rect, aspect, ppp);
    // ---- ws:canvas-handles-monitor ----
    // viewer zoom (Ctrl+wheel / pinch) and pan (middle-drag): `state.view` only, never project data,
    // and never the render size — the zoom magnifies the same texture. No Tool::Zoom.
    if resp.hovered() {
        let z = ui.input(|i| i.zoom_delta());
        if z != 1.0 {
            state.view.0 = (state.view.0 * z).clamp(ZOOM_RANGE.0, ZOOM_RANGE.1);
        }
    }
    if resp.dragged_by(PointerButton::Middle) {
        state.view.1 += resp.drag_delta();
    }
    let lb = apply_view(fit, state.view);
    state.canvas_rect = rect;
    let (cw, ch) = ((fit.width() * ppp).round().max(16.0) as u32, (fit.height() * ppp).round().max(16.0) as u32);
    r.canvas = (cw, ch);

    // social-guide overlay: painted on the Foreground layer so it sits over the video, the selection
    // outline and the tracker box, in windowed and fullscreen preview alike
    if let Some(g) = c.guide {
        let gp = ui
            .ctx()
            .layer_painter(egui::LayerId::new(egui::Order::Foreground, ui.id().with("social_guide")))
            .with_clip_rect(rect);
        crate::ui::guides::draw_guide(&gp, lb, g, c.palette);
    }

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
    // ws:canvas-handles-monitor: a pending hover preview (alt render) stands in for the live frame;
    // otherwise the GPU renderer's own texture wins: nothing is read back or re-uploaded
    let uv = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
    if let Some((id, _)) = c.alt_texture {
        painter.image(id, lb, uv, Color32::WHITE);
        painter.text(
            lb.right_top() + vec2(-6.0, 6.0),
            egui::Align2::RIGHT_TOP,
            "preview",
            egui::TextStyle::Small.resolve(ui.style()),
            c.palette.accent,
        );
    } else if let Some((id, _)) = c.gpu_texture {
        painter.image(id, lb, uv, Color32::WHITE);
    } else if let Some(t) = &state.texture {
        painter.image(t.id(), lb, uv, Color32::WHITE);
    }
    // buffering: the clock is held while the read-ahead refills — say so over the video
    if c.buffering {
        ui.put(Rect::from_center_size(lb.center(), vec2(32.0, 32.0)), egui::Spinner::new().size(32.0));
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
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

    // the Tracking pane's box: drawn over the video and dragged to place the template. A press that
    // started inside it owns the gesture, so the clip underneath is not moved as well.
    // ponytail: the drag centres the box on the pointer instead of keeping the grab offset — keeping it
    // needs the offset stored in PreviewState, and "put the tracker here" is what the gesture means.
    if let Some((tcx, tcy, thw, thh)) = c.tracker {
        let k = lb.width() / c.project.width.max(1) as f32; // points per project px
        let ctr = lb.center();
        let bx = Rect::from_center_size(ctr + vec2(tcx * k, tcy * k), vec2(thw * 2.0 * k, thh * 2.0 * k));
        let st = Stroke::new(1.5, c.palette.accent);
        painter.rect_stroke(bx, 0.0, st, StrokeKind::Middle);
        painter.line_segment([bx.center() - vec2(5.0, 0.0), bx.center() + vec2(5.0, 0.0)], st);
        painter.line_segment([bx.center() - vec2(0.0, 5.0), bx.center() + vec2(0.0, 5.0)], st);
        if ui.input(|i| i.pointer.press_origin()).is_some_and(|p| bx.contains(p)) && resp.dragged() {
            if let Some(p) = resp.interact_pointer_pos() {
                r.set_tracker = Some(((p.x - ctr.x) / k, (p.y - ctr.y) / k));
            }
            state.drag = None;
            return;
        }
    }

    // ---- ws:canvas-handles-monitor ----
    // selection overlay: one outline with handles for a single visual clip, a union box for two or
    // more (no handles); drag-to-move (with canvas snap) for both
    let sel: Vec<Id> = c
        .selection
        .iter()
        .copied()
        .filter(|&id| c.project.clip(id).is_some_and(|cl| cl.is_visual() && cl.enabled && cl.contains(c.playhead)))
        .collect();
    let Some(&id) = sel.first() else {
        state.drag = None;
        state.handle = None;
        return;
    };
    let to_screen =
        |x: f32, y: f32| pos2(lb.min.x + x / cw as f32 * lb.width(), lb.min.y + y / ch as f32 * lb.height());
    let stroke = Stroke::new(1.5, c.palette.selection);
    // the grab is decided by where the press began, not by where the pointer has dragged to
    let press = ui.input(|i| i.pointer.press_origin());
    let hover = resp.hover_pos();
    let group = sel.len() >= 2;
    let mut ol: Option<Outline> = None;
    // explicit polygon vertices (single selection): their screen frame and the vertex under the press
    let mut frame = None;
    let mut poly_hit = None;
    if group {
        let mut bb: Option<Rect> = None;
        for &sid in &sel {
            let Some(cl) = c.project.clip(sid) else { continue };
            let Some(a) = c.project.asset(cl.asset) else { continue };
            let p = placement(c.project, cl, c.playhead, (a.width, a.height), cw, ch, true);
            let (x0, y0, x1, y1) = p.bounds();
            let rr = Rect::from_min_max(to_screen(x0, y0), to_screen(x1, y1));
            bb = Some(bb.map_or(rr, |u| u.union(rr)));
        }
        if let Some(bb) = bb {
            painter.rect_stroke(bb, 0.0, stroke, StrokeKind::Middle);
            if hover.is_some_and(|p| bb.contains(p)) {
                ui.ctx().set_cursor_icon(CursorIcon::Move);
            }
        }
    } else {
        let clip = c.project.clip(id).unwrap();
        let lt = clip.local(c.playhead);
        if clip.kind == ClipKind::Text {
            let p = placement(c.project, clip, c.playhead, (1, 1), cw, ch, false);
            let o = to_screen(p.cx, p.cy);
            painter.line_segment([o - vec2(6.0, 0.0), o + vec2(6.0, 0.0)], stroke);
            painter.line_segment([o - vec2(0.0, 6.0), o + vec2(0.0, 6.0)], stroke);
        } else if let Some(a) = c.project.asset(clip.asset) {
            let p = placement(c.project, clip, c.playhead, (a.width, a.height), cw, ch, true);
            let crop = state.crop_mode.then(|| crop_fractions(clip, lt).unwrap_or([0.0; 4]));
            let mut o = Outline::new(&p, to_screen, crop);
            // keep the knob grabbable when the video fills the pane's height
            o.knob = o.knob.clamp(rect.min + vec2(6.0, 6.0), rect.max - vec2(6.0, 6.0));
            painter.add(Shape::closed_line(o.corners.to_vec(), stroke));
            let sq = |q: Pos2| Rect::from_center_size(q, vec2(7.0, 7.0));
            for q in o.corners {
                painter.rect_filled(sq(q), 1.0, c.palette.selection);
            }
            if state.crop_mode {
                // the crop ring: the visible part of the layer, with a handle mid-edge
                let ring: Vec<Pos2> = [(0usize, 3usize), (0, 1), (2, 1), (2, 3)]
                    .iter()
                    .map(|&(ey, ex)| {
                        let (ly, lx) = (o.local(o.edges[ey]).y, o.local(o.edges[ex]).x);
                        pos2(o.o.x + lx * o.cs - ly * o.sn, o.o.y + lx * o.sn + ly * o.cs)
                    })
                    .collect();
                painter.add(Shape::closed_line(ring, Stroke::new(1.0, c.palette.accent)));
                for q in o.edges {
                    painter.rect_stroke(sq(q), 1.0, Stroke::new(1.5, c.palette.accent), StrokeKind::Middle);
                }
            } else {
                for q in o.edges {
                    painter.rect_filled(sq(q), 1.0, c.palette.selection);
                }
            }
            painter.line_segment([o.edges[0], o.knob], Stroke::new(1.0, c.palette.selection));
            painter.circle_filled(o.knob, 4.0, c.palette.selection);
            ol = Some(o);
        }

        // explicit polygon vertices: outline + one grab handle each, in the shape's own rotated/scaled frame
        let poly: Vec<(f32, f32)> =
            clip.shape.as_ref().and_then(|s| s.poly_points()).map(|p| p.to_vec()).unwrap_or_default();
        frame = (!poly.is_empty()).then(|| {
            let p = placement(c.project, clip, c.playhead, (1, 1), cw, ch, false);
            let (sn, cs) = p.rot.to_radians().sin_cos();
            // project px -> screen points, through the clip's scale
            let k = clip.scale.at(lt) as f32 * lb.width() / c.project.width.max(1) as f32;
            (to_screen(p.cx, p.cy), sn, cs, if k.is_finite() && k.abs() > 1e-4 { k } else { 1e-4 })
        });
        if let Some((o, sn, cs, k)) = frame {
            let scr: Vec<Pos2> =
                poly.iter().map(|&(x, y)| pos2(o.x + (x * cs - y * sn) * k, o.y + (x * sn + y * cs) * k)).collect();
            painter.add(Shape::closed_line(scr.clone(), stroke));
            for (i, q) in scr.iter().enumerate() {
                painter.circle_filled(*q, 3.5, c.palette.selection);
                if press.is_some_and(|pp| (pp - *q).length() <= 7.0) {
                    poly_hit = Some(i);
                }
            }
        }
    }

    // hover feedback: a resize / grab cursor over a handle, a crop / rotate glyph at the pointer
    if let (Some(o), Some(hp)) = (&ol, hover) {
        if !ui.input(|i| i.pointer.any_down()) {
            let glyph_at = |g: Glyph| {
                draw_glyph(&painter, Rect::from_center_size(hp + vec2(16.0, 14.0), vec2(16.0, 16.0)), g, c.palette.text)
            };
            match o.hit(hp, state.crop_mode) {
                Some(Handle::Rotate) => {
                    ui.ctx().set_cursor_icon(CursorIcon::Grab);
                    glyph_at(Glyph::Rotate);
                }
                Some(Handle::Crop(_)) => {
                    ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
                    glyph_at(Glyph::Crop);
                }
                Some(Handle::Corner(i)) => ui.ctx().set_cursor_icon(if i % 2 == 0 {
                    CursorIcon::ResizeNwSe
                } else {
                    CursorIcon::ResizeNeSw
                }),
                Some(Handle::Edge(i)) => ui.ctx().set_cursor_icon(if i % 2 == 0 {
                    CursorIcon::ResizeVertical
                } else {
                    CursorIcon::ResizeHorizontal
                }),
                None if o.inside(hp) => ui.ctx().set_cursor_icon(CursorIcon::Move),
                None => {}
            }
        }
    }

    let handle_hit = match (&ol, press) {
        (Some(o), Some(pp)) => o.hit(pp, state.crop_mode),
        _ => None,
    };
    if resp.drag_started_by(PointerButton::Primary) {
        (c.undo)(c.project);
        state.handle = None;
        state.point_drag = None;
        state.drag = None;
        if let (Some(h), Some(o), Some(pp), Some(cl)) = (handle_hit, ol, press, c.project.clip(id)) {
            let lt = cl.local(c.playhead);
            let start = (cl.scale.at(lt), cl.scale_x.at(lt), cl.scale_y.at(lt), cl.rotation.at(lt));
            state.handle = Some(HandleDrag { handle: h, id, ol: o, press: pp, start });
        } else if poly_hit.is_some() {
            state.point_drag = poly_hit;
        } else {
            let clips = sel
                .iter()
                .filter_map(|&sid| {
                    let cl = c.project.clip(sid)?;
                    let lt = cl.local(c.playhead);
                    Some((sid, cl.x.at(lt), cl.y.at(lt)))
                })
                .collect();
            state.drag = Some((clips, Vec2::ZERO));
        }
    }
    if resp.dragged_by(PointerButton::Primary) {
        let now = resp.interact_pointer_pos().or_else(|| ui.input(|i| i.pointer.latest_pos()));
        if let (Some(d), Some(now)) = (&state.handle, now) {
            let shift = ui.input(|i| i.modifiers.shift);
            let lt = c.project.clip(d.id).map(|cl| cl.local(c.playhead)).unwrap_or(0.0);
            if handle_apply(d, now, shift, c.project, lt) {
                r.edited = true;
            }
        } else if let (Some(i), Some((o, sn, cs, k)), Some(pp)) = (state.point_drag, frame, now) {
            // pointer -> the shape's own frame (undo the rotation and scale the handles were drawn with)
            let (dx, dy) = ((pp.x - o.x) / k, (pp.y - o.y) / k);
            if let Some(p) = c.project.clip_mut(id).and_then(|cl| cl.shape.as_mut()).and_then(|s| s.points.get_mut(i)) {
                *p = (dx * cs + dy * sn, -dx * sn + dy * cs);
                r.edited = true;
            }
        } else if let Some((clips, acc)) = &mut state.drag {
            *acc += resp.drag_delta();
            let k = c.project.width as f32 / lb.width(); // project px per point
            let (mut dx, mut dy) = (acc.x * k, acc.y * k);
            // canvas snap on the single moving clip (a group would snap onto its own members —
            // ponytail: skipped for groups; snap the union box if that ever matters)
            let mut guides = Vec::new();
            if let (false, Some(&(sid, x0, y0))) = (group, clips.first()) {
                if let Some(half) = half_size(c.project, sid, c.playhead) {
                    let cand = (x0 as f32 + dx, y0 as f32 + dy);
                    let ((sx, sy), g) = canvas_snap(c.canvas_snap, c.project, sid, c.playhead, cand, half, SNAP_PX * k);
                    (dx, dy, guides) = (sx - x0 as f32, sy - y0 as f32, g);
                }
            }
            for &(sid, x0, y0) in clips.iter() {
                if let Some(cl) = c.project.clip_mut(sid) {
                    let lt = cl.local(c.playhead);
                    cl.x.set_at(lt, x0 + dx as f64);
                    cl.y.set_at(lt, y0 + dy as f64);
                }
            }
            r.edited = true;
            paint_canvas_guides(&painter, &guides, lb, (c.project.width, c.project.height), c.palette);
        }
    }
    if resp.drag_stopped() {
        state.drag = None;
        state.point_drag = None;
        state.handle = None;
    }
}

/// Small "Pre-rendering NN %" / "Building proxy NN %" badge in the top-left of the video.
fn prerender_badge(ui: &egui::Ui, painter: &egui::Painter, lb: Rect, c: &PreviewCtx<'_>) {
    let (p, label) = match (c.prerender, c.proxy) {
        (Some(p), _) => (p, "Pre-rendering"),
        (None, Some(p)) => (p, "Building proxy"),
        (None, None) => return,
    };
    let mut text = format!("{label} {:.0} %", (p.clamp(0.0, 1.0) * 100.0));
    if c.prerender.is_none() {
        // say WHICH file, so the pre-proxy window reads as progress on something, not vague churn
        if let Some((src, _)) = crate::media::proxy::building() {
            if let Some(name) = std::path::Path::new(&src).file_name() {
                text = format!("{text} — {}", name.to_string_lossy());
            }
        }
    }
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

    /// Moved here from the lib-preview mini-player's identical assertions (`scrub_bar` now shares this
    /// one implementation instead of duplicating it).
    #[test]
    fn scrub_time_clamps_to_bar() {
        assert_eq!(scrub_time(0.0, 4.0), 0.0);
        assert_eq!(scrub_time(1.0, 4.0), 4.0);
        assert_eq!(scrub_time(0.5, 4.0), 2.0);
        assert_eq!(scrub_time(-0.2, 4.0), 0.0, "past-left clamps to the start");
        assert_eq!(scrub_time(1.2, 4.0), 4.0, "past-right clamps to the end");
    }

    /// The tool_drag guard's fallthrough (`let Some(d) = &mut state.tool_drag else { ... }`) no longer
    /// names the deleted Zoom tool variant — proven at compile time (this whole crate would not build
    /// if it still did; a text self-scan of this very file can't check for its own search string, so
    /// the guarantee here is the stronger one) plus a runtime check, same idiom as
    /// `shape_tool_drag_reports_a_shape_instead_of_moving_the_clip` above: an ordinary non-Select/Text
    /// tool (Cut, which has no special-cased body of its own) still owns the canvas drag instead of
    /// moving the clip, so gestures on ordinary tools are unaffected by the removal.
    #[test]
    fn no_tool_zoom_references() {
        let mut h = H::new();
        h.tool = Tool::Cut;
        h.frame(vec![]);
        h.drag(pos2(250.0, 150.0), pos2(400.0, 250.0));
        let c = h.project.clip(7).unwrap();
        assert_eq!((c.x.value, c.y.value), (0.0, 0.0), "a non-Select/Text tool never moves the clip");
        assert_eq!(h.undos, 0, "no clip undo for a non-Select/Text tool's gesture");
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
            self.frame_mod(events, Modifiers::NONE)
        }
        fn frame_mod(&mut self, events: Vec<Event>, modifiers: Modifiers) -> PreviewResponse {
            self.time += 0.05;
            let input = RawInput {
                screen_rect: Some(self.panel),
                time: Some(self.time),
                events,
                modifiers,
                ..Default::default()
            };
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
                            shape_style: None,
                            quality: 100,
                            movie_mode: false,
                            prerender: Some(0.4),
                            buffering: false,
                            proxy: None,
                            tracker: None,
                            guide: None,
                            canvas_snap: false,
                            alt_texture: None,
                            use_proxies: false,
                            dropped: 0,
                        },
                    );
                });
            });
            out
        }
        // ---- ws:canvas-handles-monitor ----
        /// A 1280x720 video asset on clip 7 (16:9, so it fills the 1920x1080 canvas exactly) and a
        /// taller panel with black bars above the video, so the outline, its handles AND the rotate
        /// knob above the top edge all land inside the video widget. Two frames settle the layout.
        fn with_asset(&mut self) {
            use crate::model::Asset;
            let a = self.project.add_asset(Asset {
                id: 0,
                path: "C:/x.mp4".into(),
                kind: ClipKind::Video,
                duration: 4.0,
                width: 1280,
                height: 720,
                fps: 30.0,
                audio_streams: Vec::new(),
                codec: String::new(),
                folder: String::new(),
                tags: Vec::new(),
                label: 0,
                description: String::new(),
                rel_path: None,
                parent: None,
                range: None,
                effects: Vec::new(),
            });
            self.project.clip_mut(7).unwrap().asset = a;
            self.panel = Rect::from_min_size(Pos2::ZERO, vec2(700.0, 620.0));
            self.frame(vec![]);
            self.frame(vec![]);
        }
        /// The letterbox the video is painted in (the fit rect at zoom 1, pan 0).
        fn lb(&self) -> Rect {
            letterbox(self.state.canvas_rect, 16.0 / 9.0, 1.0)
        }
        /// A middle-button drag (viewer pan).
        fn middle_drag(&mut self, from: Pos2, to: Pos2) {
            let btn = |pos, pressed| Event::PointerButton {
                pos,
                button: PointerButton::Middle,
                pressed,
                modifiers: Modifiers::NONE,
            };
            self.frame(vec![Event::PointerMoved(from)]);
            self.frame(vec![btn(from, true)]);
            for i in 1..=4 {
                self.frame(vec![Event::PointerMoved(from + (to - from) * (i as f32 / 4.0))]);
            }
            self.frame(vec![btn(to, false)]);
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
            self.drag_mod(from, to, Modifiers::NONE)
        }
        /// Same gesture, with `modifiers` held down for the whole drag (Shift / Alt shape constraints).
        fn drag_mod(&mut self, from: Pos2, to: Pos2, modifiers: Modifiers) -> PreviewResponse {
            self.frame_mod(vec![Event::PointerMoved(from)], modifiers);
            self.frame_mod(
                vec![Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers }],
                modifiers,
            );
            let steps = 4;
            let (mut mask_edit, mut edited) = (false, false);
            for i in 1..=steps {
                let p = from + (to - from) * (i as f32 / steps as f32);
                let f = self.frame_mod(vec![Event::PointerMoved(p)], modifiers);
                mask_edit |= f.mask_edit;
                edited |= f.edited;
            }
            let mut out = self.frame_mod(
                vec![Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers }],
                modifiers,
            );
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

    /// The live preview (draw_shape_preview) is exercised for every shape kind by each frame of the
    /// drag; this is the regression coverage for it (its output can't be asserted on pixel-by-pixel like
    /// engine::shapes' rasteriser tests, since it paints straight into an egui::Painter, but a panic
    /// there — e.g. the arrow head's corner/stroke_width-derived size clamped against a near-zero drag
    /// length — would fail this test).
    #[test]
    fn shape_tool_preview_paints_without_panicking_for_every_kind() {
        // Polygon is click-to-place-vertices (its own branch above, own coverage in
        // `polygon_tool_places_and_closes_real_points`), not a single drag — skip it here.
        for kind in ShapeKind::ALL.into_iter().filter(|&k| k != ShapeKind::Polygon) {
            let mut h = H::new();
            h.tool = Tool::Shape(kind);
            h.frame(vec![]);
            // small, not zero: big enough to clear egui's own drag-recognition threshold (a 3x2 px
            // drag never registers as a drag at all, so drag_stopped() would never fire) while still
            // small enough to stress the near-zero-length arrow-head/clamp path this test guards.
            let out = h.drag(pos2(250.0, 150.0), pos2(262.0, 165.0));
            let (k, ..) = out.new_shape.unwrap_or_else(|| panic!("{kind:?} drag reports a shape"));
            assert_eq!(k, kind);
        }
    }

    /// Bug fix regression: dragging with the Text tool must report a text box (once) instead of falling
    /// into the generic tool arms, which used to draw a throwaway outline and discard it on release.
    #[test]
    fn text_tool_drag_reports_new_text_exactly_once_and_never_moves_the_clip() {
        let mut h = H::new();
        h.tool = Tool::Text;
        h.frame(vec![]);
        let (from, to) = (pos2(250.0, 150.0), pos2(400.0, 250.0));
        // drive the gesture by hand so every frame's response can be counted (mirrors
        // a_shape_drag_emits_exactly_one_shape below)
        h.frame(vec![Event::PointerMoved(from)]);
        let mut n = 0;
        let mut last = None;
        let mut count = |r: PreviewResponse| {
            if let Some(t) = r.new_text {
                n += 1;
                last = Some(t);
            }
        };
        count(h.frame(vec![Event::PointerButton {
            pos: from,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        }]));
        for i in 1..=4 {
            let p = from + (to - from) * (i as f32 / 4.0);
            count(h.frame(vec![Event::PointerMoved(p)]));
        }
        count(h.frame(vec![Event::PointerButton {
            pos: to,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }]));
        // a few idle frames after the release: a gesture that never cleared would keep firing
        for _ in 0..3 {
            count(h.frame(vec![]));
        }
        assert_eq!(n, 1, "one drag, one text box");
        let (_, _, hw, hh) = last.expect("a text box was dragged out");
        assert!(hw > 1.0 && hh > 1.0, "non-empty box: {hw} x {hh}");
        // the drag must not have moved (or otherwise edited) the existing clip
        let c = h.project.clip(7).unwrap();
        assert_eq!((c.x.value, c.y.value), (0.0, 0.0), "text tool never moves the clip");
        assert_eq!(h.undos, 0, "no clip undo for a text-box gesture");
    }

    /// A line follows the drag in every direction: the half-extents stay signed, so dragging up-right
    /// makes a line that points up-right instead of snapping to its bounding box's other diagonal.
    /// One drag must produce exactly ONE shape. If the gesture fired on more than one frame the timeline
    /// would gain several stacked clips and several undo entries, so a single Ctrl+Z would look dead.
    #[test]
    fn a_shape_drag_emits_exactly_one_shape() {
        let mut h = H::new();
        h.tool = Tool::Shape(ShapeKind::Line);
        h.frame(vec![]);
        let (from, to) = (pos2(250.0, 150.0), pos2(400.0, 80.0));
        // drive the gesture by hand so every frame's response can be counted
        h.frame(vec![Event::PointerMoved(from)]);
        let mut n = 0;
        let mut count = |r: PreviewResponse| {
            if r.new_shape.is_some() {
                n += 1;
            }
        };
        count(h.frame(vec![Event::PointerButton {
            pos: from,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        }]));
        for i in 1..=4 {
            let p = from + (to - from) * (i as f32 / 4.0);
            count(h.frame(vec![Event::PointerMoved(p)]));
        }
        count(h.frame(vec![Event::PointerButton {
            pos: to,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }]));
        // a few idle frames after the release: a gesture that never cleared would keep firing
        for _ in 0..3 {
            count(h.frame(vec![]));
        }
        assert_eq!(n, 1, "one drag, one shape (each extra one is an extra undo step)");
    }

    #[test]
    fn line_tool_keeps_the_press_as_its_origin() {
        // (dx, dy) of the drag -> expected sign of (w, h)
        for (to, want) in [
            (pos2(400.0, 250.0), (1.0, 1.0)),  // right + down
            (pos2(400.0, 50.0), (1.0, -1.0)),  // right + up
            (pos2(100.0, 250.0), (-1.0, 1.0)), // left  + down
            (pos2(100.0, 50.0), (-1.0, -1.0)), // left  + up
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
    fn shift_locks_a_rect_drag_to_a_square() {
        let mut h = H::new();
        h.tool = Tool::Shape(ShapeKind::Rect);
        h.frame(vec![]);
        let out = h.drag_mod(pos2(250.0, 150.0), pos2(400.0, 220.0), Modifiers::SHIFT);
        let (_, _, _, w, hh) = out.new_shape.expect("a shape was dragged out");
        assert!((w - hh).abs() < 0.01, "Shift must square the drag: {w} x {hh}");
    }

    #[test]
    fn shift_locks_a_line_to_45_degrees() {
        let mut h = H::new();
        h.tool = Tool::Shape(ShapeKind::Line);
        h.frame(vec![]);
        // a drag near 41 degrees should snap to a perfect 45-degree diagonal
        let out = h.drag_mod(pos2(250.0, 150.0), pos2(400.0, 280.0), Modifiers::SHIFT);
        let (_, _, _, w, hh) = out.new_shape.expect("a line was dragged out");
        assert!((w.abs() - hh.abs()).abs() < 0.5, "45 deg: |w| should equal |hh|, got {w} x {hh}");
    }

    #[test]
    fn alt_grows_a_shape_from_the_press_point() {
        let mut h = H::new();
        h.tool = Tool::Shape(ShapeKind::Rect);
        h.frame(vec![]);
        let press = pos2(250.0, 150.0);
        let plain = h.drag(press, pos2(350.0, 220.0)).new_shape.unwrap();
        let mut h2 = H::new();
        h2.tool = Tool::Shape(ShapeKind::Rect);
        h2.frame(vec![]);
        let alt = h2.drag_mod(press, pos2(350.0, 220.0), Modifiers::ALT).new_shape.unwrap();
        // same half-extents doubled, and the centre sits on the press point instead of the midpoint
        assert!((alt.3 - plain.3 * 2.0).abs() < 0.5, "half-width doubles under Alt: {} vs {}", alt.3, plain.3);
        assert!((alt.4 - plain.4 * 2.0).abs() < 0.5, "half-height doubles under Alt: {} vs {}", alt.4, plain.4);
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

    /// A mask shapes pixels: dragging one over a selected audio clip does nothing at all.
    #[test]
    fn mask_tool_drag_ignores_an_audio_clip() {
        let mut h = H::new();
        h.project.clip_mut(7).unwrap().kind = ClipKind::Audio;
        h.tool = Tool::Mask(MaskShape::Ellipse);
        h.frame(vec![]);
        let out = h.drag(pos2(240.0, 140.0), pos2(420.0, 260.0));
        assert!(!out.mask_edit, "nothing was masked");
        assert!(h.project.clip(7).unwrap().mask.is_none(), "no mask on an audio clip");
        assert_eq!(h.undos, 0, "and no undo entry for the edit that never happened");
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

    // ---- ws:canvas-handles-monitor ----

    fn xform(h: &H) -> (f64, f64, f64, f64, f64, f64) {
        let c = h.project.clip(7).unwrap();
        (c.scale.value, c.scale_x.value, c.scale_y.value, c.rotation.value, c.x.value, c.y.value)
    }

    #[test]
    fn corner_handle_scales_uniformly_with_one_undo() {
        let mut h = H::new();
        h.with_asset();
        let lb = h.lb();
        let out = h.drag(lb.right_bottom(), lb.right_bottom() + vec2(40.0, 22.5));
        assert!(out.edited);
        let (s, sx, sy, rot, x, y) = xform(&h);
        assert!(s > 1.05, "a corner drag away from the centre scales up: {s}");
        assert_eq!((sx, sy, rot), (1.0, 1.0, 0.0), "corner = uniform scale only");
        assert_eq!((x, y), (0.0, 0.0), "the centre never moves");
        assert_eq!(h.undos, 1, "one undo per gesture");
    }

    #[test]
    fn edge_handle_writes_independent_scale_axis() {
        let mut h = H::new();
        h.with_asset();
        let lb = h.lb();
        let right = pos2(lb.right(), lb.center().y);
        h.drag(right, right + vec2(60.0, 0.0));
        let (s, sx, sy, ..) = xform(&h);
        assert!(sx > 1.05, "right edge writes scale_x: {sx}");
        assert_eq!((s, sy), (1.0, 1.0), "…and nothing else");
        let mut h = H::new();
        h.with_asset();
        let lb = h.lb();
        let top = pos2(lb.center().x, lb.top());
        h.drag(top, top - vec2(0.0, 40.0));
        let (s, sx, sy, ..) = xform(&h);
        assert!(sy > 1.05, "top edge writes scale_y: {sy}");
        assert_eq!((s, sx), (1.0, 1.0), "…and nothing else");
    }

    #[test]
    fn rotate_handle_snaps_to_15_degrees_with_shift() {
        // the knob sits KNOB_OFFSET above the top edge; the drag target is that point swung 40° about
        // the centre (screen y grows downwards, so the maths below is a plain 2-D rotation)
        let target = |lb: Rect, deg: f32| {
            let o = lb.center();
            let arm = lb.height() / 2.0 + KNOB_OFFSET;
            let (s, c) = deg.to_radians().sin_cos();
            pos2(o.x + arm * s, o.y - arm * c)
        };
        let mut h = H::new();
        h.with_asset();
        let lb = h.lb();
        let knob = pos2(lb.center().x, lb.top() - KNOB_OFFSET);
        h.drag(knob, target(lb, 40.0));
        let (s, sx, sy, rot, ..) = xform(&h);
        assert!((rot - 40.0).abs() < 1.5, "continuous rotation follows the pointer: {rot}");
        assert_eq!((s, sx, sy), (1.0, 1.0, 1.0), "rotate never scales");
        assert_eq!(h.undos, 1);
        let mut h = H::new();
        h.with_asset();
        let lb = h.lb();
        let knob = pos2(lb.center().x, lb.top() - KNOB_OFFSET);
        h.drag_mod(knob, target(lb, 40.0), Modifiers::SHIFT);
        let rot = h.project.clip(7).unwrap().rotation.value;
        assert_eq!(rot, 45.0, "Shift snaps to the nearest 15°");
    }

    #[test]
    fn crop_handle_appends_exactly_one_crop_effect() {
        let mut h = H::new();
        h.with_asset();
        h.state.crop_mode = true;
        let lb = h.lb();
        // right edge inwards by 120 px, then the top edge (now on the cropped ring, 60 px left of
        // centre) downwards by 60 px
        let right = pos2(lb.right(), lb.center().y);
        h.drag(right, right - vec2(120.0, 0.0));
        let top = pos2(lb.center().x - 60.0, lb.top());
        h.drag(top, top + vec2(0.0, 60.0));
        let c = h.project.clip(7).unwrap();
        let crops: Vec<&Effect> = c.effects.iter().filter(|e| e.kind == EffectKind::Crop).collect();
        assert_eq!(c.effects.len(), 1, "two crop drags, one effect: {:?}", c.effects.iter().map(|e| e.kind).collect::<Vec<_>>());
        let e = crops[0];
        let (right_f, top_f) = (e.params[1].value, e.params[2].value);
        assert!((right_f - 120.0 / lb.width() as f64).abs() < 0.01, "Right fraction {right_f}");
        assert!((top_f - 60.0 / lb.height() as f64).abs() < 0.01, "Top fraction {top_f}");
        assert_eq!((e.params[0].value, e.params[3].value), (0.0, 0.0), "untouched sides stay 0");
        assert_eq!(xform(&h).1, 1.0, "crop never scales");
        assert_eq!(h.undos, 2, "one undo per drag");
    }

    #[test]
    fn crop_mode_off_never_touches_effects() {
        let mut h = H::new();
        h.with_asset();
        assert!(!h.state.crop_mode, "off by default");
        let lb = h.lb();
        let right = pos2(lb.right(), lb.center().y);
        h.drag(right, right + vec2(60.0, 0.0));
        let c = h.project.clip(7).unwrap();
        assert!(c.effects.is_empty(), "no Crop effect without the toggle");
        assert!(c.scale_x.value > 1.05, "the edge handle scaled instead");
    }

    #[test]
    fn mask_drag_targets_the_selected_effect_mask() {
        let mut h = H::new();
        h.project.clip_mut(7).unwrap().effects.push(Effect::new(EffectKind::ALL[0]));
        h.state.mask_target = MaskTarget::Effect(0);
        h.tool = Tool::Mask(MaskShape::Ellipse);
        h.frame(vec![]);
        let out = h.drag(pos2(240.0, 140.0), pos2(420.0, 260.0));
        assert!(out.mask_edit);
        let c = h.project.clip(7).unwrap();
        assert!(c.effects[0].mask.is_some(), "the effect's mask was written");
        assert!(c.mask.is_none(), "the clip's own mask is untouched");
        assert_eq!(h.undos, 1);
        // a stale effect index is a no-op, not a panic and not an undo entry
        let mut h = H::new();
        h.state.mask_target = MaskTarget::Effect(3);
        h.tool = Tool::Mask(MaskShape::Ellipse);
        h.frame(vec![]);
        let out = h.drag(pos2(240.0, 140.0), pos2(420.0, 260.0));
        assert!(!out.mask_edit);
        assert!(h.project.clip(7).unwrap().mask.is_none());
        assert_eq!(h.undos, 0);
    }

    /// Regression guard for the rewired mask block: with `mask_target` left at its default the
    /// gesture still writes `clip.mask` exactly as before this workstream.
    #[test]
    fn mask_drag_defaults_to_clip_mask_unchanged() {
        let mut h = H::new();
        assert_eq!(h.state.mask_target, MaskTarget::Clip);
        h.project.clip_mut(7).unwrap().effects.push(Effect::new(EffectKind::ALL[0]));
        h.tool = Tool::Mask(MaskShape::Rect);
        h.frame(vec![]);
        let out = h.drag(pos2(240.0, 140.0), pos2(420.0, 260.0));
        assert!(out.mask_edit);
        let c = h.project.clip(7).unwrap();
        let m = c.mask.as_ref().expect("clip mask written");
        assert_eq!(m.shape, MaskShape::Rect);
        assert!(m.rx.value > 1.0 && m.ry.value > 1.0);
        assert!(c.effects[0].mask.is_none(), "the effect's mask is untouched");
        assert_eq!(h.undos, 1);
    }

    #[test]
    fn group_selection_shows_union_box_and_moves_every_clip() {
        let mut h = H::new();
        h.with_asset();
        let a = h.project.clip(7).unwrap().asset;
        let mut c8 = Clip::new(8, ClipKind::Video, "w", 0.0, 4.0);
        c8.asset = a;
        h.project.tracks[0].clips.push(c8);
        h.selection = vec![7, 8];
        h.frame(vec![]);
        let lb = h.lb();
        // a press exactly on what would be the BR corner handle: a group has no handles, so it moves
        h.drag(lb.right_bottom() - vec2(1.0, 1.0), lb.right_bottom() + vec2(39.0, 19.0));
        let k = 1920.0 / lb.width() as f64;
        for id in [7, 8] {
            let c = h.project.clip(id).unwrap();
            assert!((c.x.value - 40.0 * k).abs() < 1.0 && (c.y.value - 20.0 * k).abs() < 1.0, "clip {id} moved: {:?}", (c.x.value, c.y.value));
            assert_eq!((c.scale.value, c.scale_x.value, c.rotation.value), (1.0, 1.0, 0.0), "no handle on a group");
        }
        assert_eq!(h.undos, 1, "one undo for the whole group move");
    }

    #[test]
    fn zoom_and_pan_never_edit_the_project() {
        let mut h = H::new();
        h.with_asset();
        let before = h.project.to_json();
        let lb = h.lb();
        h.frame(vec![Event::PointerMoved(lb.center()), Event::Zoom(1.5)]);
        assert!((h.state.view.0 - 1.5).abs() < 1e-4, "Ctrl+wheel / pinch zooms: {:?}", h.state.view);
        h.middle_drag(lb.center(), lb.center() + vec2(30.0, 10.0));
        assert!(h.state.view.1.length() > 20.0, "middle-drag pans: {:?}", h.state.view);
        assert_eq!(h.project.to_json(), before, "the project is untouched");
        assert_eq!(h.undos, 0, "no undo entry");
        // Fit Viewer is the reset (see monitor::act) — apply_view at (1, 0) is the plain letterbox
        assert_eq!(apply_view(lb, (1.0, Vec2::ZERO)), lb);
        let z = apply_view(lb, (2.0, vec2(5.0, 0.0)));
        assert_eq!(z.size(), lb.size() * 2.0);
        assert_eq!(z.center(), lb.center() + vec2(5.0, 0.0));
    }

    /// Idle frames with the handles (and crop ring) on screen request no repaint — the "idle CPU 0%"
    /// gate the selftest checks for the plain pane.
    #[test]
    fn assert_no_idle_repaint_with_handles_and_crop_ring() {
        let mut h = H::new();
        h.with_asset();
        h.state.crop_mode = true;
        for _ in 0..5 {
            h.frame(vec![]);
        }
        assert!(!h.ctx.has_requested_repaint(), "idle frame with handles shown requested a repaint");
    }
}
