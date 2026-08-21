//! Timeline widget (custom painted). Header column (track name, M/S buttons, height drag handle at the
//! bottom edge), ruler (ticks, in/out markers, click/drag scrubs), track lanes (video tracks top→bottom =
//! Vn..V1, then A1..An), clips (name, waveform for audio via WaveformCache, keyframe diamonds, disabled
//! = dimmed, selected = accent outline), playhead (drag by ruler or line).
//!
//! Interaction: click select (Ctrl toggles), drag clip body = move (linked clips follow; Project::move_clips
//! all-or-nothing; up/down across tracks of the same kind), drag clip edges = trim (Clip::trim_start /
//! trim_end with Project::max_clip_duration), snapping (playhead, clip edges, in/out, 0; ~8 px) when
//! `snap`, right-click context menu (Split, Delete, Ripple Delete, Link/Unlink, Enable/Disable, Add
//! Video/Audio Track, Remove Track (on headers), Mute/Solo). Ctrl+Scroll = zoom around the cursor,
//! Shift+Scroll = horizontal pan, Alt+Scroll = height of the track under the cursor, plain scroll =
//! vertical pan. Accepts egui dnd payloads `DragPayload` (Asset → Project::insert_asset_clips at the drop
//! time/track; Path → returned in `dropped_files`). Every mutation calls `c.undo(&project)` with the project as
//! it was before the gesture (once per gesture, only if it changed something) and sets `edited`. While `playing`,
//! auto-scroll keeps the playhead visible unless the user panned away (resumes once the playhead re-enters the
//! view or playback pauses).
//!
//! Round 2: horizontal/vertical scrollbars (thumb drag + click-to-page), edge auto-scroll during drags,
//! linked selection (click = link group, Alt+click = single, linked clips get a thin outline), audio
//! volume line (dB mapped, draggable) + fade handles, video track V/S headers, full-width track resize
//! handles, draggable keyframe diamonds with an easing context menu, transition bands (edge drag = duration,
//! right-click = remove), colour labels / sequence tint, speed badges, thumbnail filmstrips (ThumbCache),
//! auto-cut keep-range shading and Sequence open-on-double-click.

use crate::media::thumbs::ThumbCache;
use crate::media::waveform::{Peaks, WaveformCache};
use crate::model::{Asset, Clip, ClipKind, Ease, Id, Project, TrackKind, LABEL_COLORS};
use crate::theme::Palette;
use crate::ui::DragPayload;
use eframe::egui::{
    self, pos2, vec2, Align2, Color32, CornerRadius, CursorIcon, FontId, Pos2, Rangef, Rect, Sense, Shape, Stroke,
    StrokeKind,
};
use std::path::PathBuf;

const RULER_H: f32 = 22.0;
const EDGE_W: f32 = 6.0;
const HANDLE_H: f32 = 4.0;
const SNAP_PX: f32 = 8.0;
/// Scrollbar strip thickness (points).
const HBAR_H: f32 = 10.0;
const VBAR_W: f32 = 10.0;
/// Edge auto-scroll kicks in within this many points of the lanes edge.
const SCROLL_MARGIN: f32 = 16.0;
/// Volume line dB range: +12 dB at the top of the clip, -60 dB at the bottom, 0 dB at 70 % height.
const DB_TOP: f32 = 12.0;
const DB_BOT: f32 = -60.0;
const MIN_TRACK_H: f32 = 24.0;
const MAX_TRACK_H: f32 = 300.0;
/// (major tick, minor tick) seconds; the first major that spans >= 80 px wins.
const TICKS: [(f64, f64); 16] = [
    (0.05, 0.01),
    (0.1, 0.02),
    (0.2, 0.05),
    (0.5, 0.1),
    (1.0, 0.2),
    (2.0, 0.5),
    (5.0, 1.0),
    (10.0, 2.0),
    (15.0, 5.0),
    (30.0, 10.0),
    (60.0, 15.0),
    (120.0, 30.0),
    (300.0, 60.0),
    (600.0, 120.0),
    (1800.0, 300.0),
    (3600.0, 600.0),
];

pub struct TimelineState {
    /// Pixels per second.
    pub zoom: f32,
    /// Horizontal scroll in seconds at the left edge of the lanes.
    pub scroll_x: f64,
    /// Vertical scroll in points.
    pub scroll_y: f32,
    /// Width of the header column in points.
    pub header_w: f32,
    /// Content rect (lanes area, excluding header & ruler) from the last frame — used by the app for file drops.
    pub lanes_rect: egui::Rect,
    /// Active move/trim gesture.
    drag: Option<Drag>,
    /// The user panned/zoomed while playing: stop following the playhead until it is back in view.
    user_panned: bool,
}

impl Default for TimelineState {
    fn default() -> Self {
        Self {
            zoom: 40.0,
            scroll_x: 0.0,
            scroll_y: 0.0,
            header_w: 150.0,
            lanes_rect: egui::Rect::NOTHING,
            drag: None,
            user_panned: false,
        }
    }
}

impl TimelineState {
    /// Timeline time at an absolute x position (points).
    pub fn time_at(&self, x: f32) -> f64 {
        self.scroll_x + ((x - self.lanes_rect.left()) / self.zoom) as f64
    }
    pub fn x_at(&self, t: f64) -> f32 {
        self.lanes_rect.left() + ((t - self.scroll_x) as f32) * self.zoom
    }
    /// Track index under an absolute y position (points), given the project (for track heights).
    pub fn track_at(&self, y: f32, project: &Project) -> Option<usize> {
        if y < self.lanes_rect.top() {
            return None;
        }
        let mut top = self.lanes_rect.top() - self.scroll_y;
        for i in row_order(project) {
            let h = project.tracks[i].height;
            if y >= top && y < top + h {
                return Some(i);
            }
            top += h;
        }
        None
    }
    /// Fit [0, duration] into `avail_w` points.
    pub fn zoom_to_fit(&mut self, duration: f64, avail_w: f32) {
        let d = duration.max(1.0) as f32;
        self.zoom = ((avail_w - 20.0).max(50.0) / d).clamp(0.5, 2000.0);
        self.scroll_x = 0.0;
    }
    /// Multiply zoom, keeping the time under `anchor_x` (absolute points) fixed if given.
    pub fn zoom_by(&mut self, factor: f32, anchor_x: Option<f32>) {
        let anchor_t = anchor_x.map(|x| self.time_at(x));
        self.zoom = (self.zoom * factor).clamp(0.5, 2000.0);
        if let (Some(t), Some(x)) = (anchor_t, anchor_x) {
            self.scroll_x = (t - ((x - self.lanes_rect.left()) / self.zoom) as f64).max(0.0);
        }
    }
    /// Scroll so `t` is visible (used when the playhead runs off-screen while playing). Suspended after a
    /// user pan until `t` is back in view.
    pub fn ensure_visible(&mut self, t: f64) {
        let w = (self.lanes_rect.width() / self.zoom) as f64;
        if w <= 0.0 {
            return;
        }
        if t >= self.scroll_x && t <= self.scroll_x + w {
            self.user_panned = false;
        } else if !self.user_panned {
            self.scroll_x = (t - w * 0.1).max(0.0);
        }
    }
}

pub struct TimelineCtx<'a> {
    pub project: &'a mut Project,
    pub selection: &'a mut Vec<Id>,
    pub playhead: &'a mut f64,
    /// Call with the project *before* mutating it (once per gesture) — pushes an undo snapshot.
    pub undo: &'a mut dyn FnMut(&Project),
    pub waveforms: &'a mut WaveformCache,
    pub palette: &'a Palette,
    pub snap: bool,
    pub playing: bool,
    /// Thumbnail cache for video/image filmstrips. None = no filmstrips (e.g. headless tests).
    pub thumbs: Option<&'a mut ThumbCache>,
    /// Timeline ranges shaded in `palette.selection` at 15 % alpha (auto-cut "keep" preview). Empty = nothing.
    pub keep_ranges: &'a [(f64, f64)],
}

#[derive(Default)]
pub struct TimelineResponse {
    /// The project was mutated (app marks dirty & re-renders).
    pub edited: bool,
    /// The playhead was moved by the user.
    pub seeked: bool,
    /// Files dropped via dnd `DragPayload::Path` — (path, timeline time, track index).
    pub dropped_files: Vec<(PathBuf, f64, Option<usize>)>,
    /// Other dnd payloads (Sequence / Template) dropped on the lanes — the app places them.
    pub dropped_other: Vec<(DragPayload, f64, Option<usize>)>,
    /// Actions requested from the timeline's context menus (Retime, AddTransition, FreezeFrame, AutoCut, …).
    pub actions: Vec<crate::hotkeys::Action>,
    /// A Sequence clip was double-clicked — the app calls `Project::open_sequence` with this sequence id.
    pub open_sequence: Option<Id>,
}

struct Drag {
    /// Pointer position where the gesture started.
    origin: Pos2,
    /// Project at gesture start — pushed as the undo snapshot on release, only if something changed.
    before: Project,
    g: Gesture,
}

enum Gesture {
    /// Move `ids` (selection + links). `orig` = start time of each id at drag start; `tr` = track of the pressed
    /// clip; `dt`/`dtrack` = applied so far.
    Move { ids: Vec<Id>, orig: Vec<f64>, kind: TrackKind, tr: usize, dt: f64, dtrack: i32 },
    /// Trim the start (`start`) or end edge of `ids`; `edge` = edge time at drag start.
    Trim { ids: Vec<Id>, start: bool, edge: f64, changed: bool },
    /// Drag the volume line of an audio clip vertically.
    Volume { id: Id, changed: bool },
    /// Drag a fade handle horizontally (`out` = fade-out, else fade-in).
    Fade { id: Id, out: bool, changed: bool },
    /// Drag the keyframe diamond(s) at clip-local time `t` horizontally.
    Keys { id: Id, t: f64, changed: bool },
    /// Drag a transition band edge (duration changes symmetrically around the cut).
    TransDur { track: usize, id: Id, changed: bool },
}

/// Deferred project mutation (collected while the project is borrowed for drawing).
enum Act {
    Split,
    Delete(bool),
    Link,
    Enable(bool),
    AddTrack(TrackKind),
    RemoveTrack(usize),
    Mute(usize),
    Solo(usize),
    DropAsset(Id, f64, Option<usize>),
    /// Colour label for the selection (0 = none / inherit asset).
    Label(u8),
    /// Copy the selection's effective labels onto their assets.
    LabelToAsset,
    /// Set the easing of every property keyed at clip-local time `t`.
    SetEase(Id, f64, Ease),
    /// Delete every keyframe at clip-local time `t`.
    DelKeys(Id, f64),
    RemoveTransition(Id),
}

/// Display order: video tracks reversed (Vn on top, V1 just above audio), then A1..An.
fn row_order(p: &Project) -> impl Iterator<Item = usize> + '_ {
    let n = p.tracks.len();
    (0..n)
        .rev()
        .filter(move |&i| p.tracks[i].kind == TrackKind::Video)
        .chain((0..n).filter(move |&i| p.tracks[i].kind == TrackKind::Audio))
}

/// Top y of track `ti` in display order (absolute points).
fn row_top(state: &TimelineState, p: &Project, ti: usize) -> Option<f32> {
    let mut top = state.lanes_rect.top() - state.scroll_y;
    for i in row_order(p) {
        if i == ti {
            return Some(top);
        }
        top += p.tracks[i].height;
    }
    None
}

/// Nearest candidate within `thr` of `t`.
fn nearest(t: f64, thr: f64, candidates: impl Iterator<Item = f64>) -> Option<f64> {
    let mut best: Option<f64> = None;
    for x in candidates {
        let d = (x - t).abs();
        if d <= thr && best.map_or(true, |b| d < (b - t).abs()) {
            best = Some(x);
        }
    }
    best
}

/// Snap target for `t`: 0, playhead, in/out, edges of clips not in `exclude` — within `thr` seconds.
fn snap_target(t: f64, thr: f64, p: &Project, playhead: f64, exclude: &[Id]) -> Option<f64> {
    let edges = p.all_clips().filter(|(_, c)| !exclude.contains(&c.id)).flat_map(|(_, c)| [c.start, c.end()]);
    nearest(t, thr, [0.0, playhead].into_iter().chain(p.in_point).chain(p.out_point).chain(edges))
}

/// (major, minor) tick spacing in seconds for a zoom (px/s).
fn tick_step(zoom: f32) -> (f64, f64) {
    TICKS.iter().copied().find(|(major, _)| *major * zoom as f64 >= 80.0).unwrap_or(TICKS[TICKS.len() - 1])
}

fn tick_label(t: f64, major: f64) -> String {
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

fn toggle_button(
    ui: &egui::Ui,
    p: &egui::Painter,
    rect: Rect,
    id: egui::Id,
    label: &str,
    on: bool,
    pal: &Palette,
    font: &FontId,
) -> bool {
    let r = ui.interact(rect, id, Sense::click());
    let fill = if on { pal.accent } else { pal.panel };
    let stroke = if r.hovered() { pal.accent } else { pal.border };
    p.rect(rect, CornerRadius::same(2), fill, Stroke::new(1.0, stroke), StrokeKind::Inside);
    p.text(rect.center(), Align2::CENTER_CENTER, label, font.clone(), pal.text);
    r.clicked()
}

/// One vertical min..max line per point column over the visible part of an audio clip.
fn draw_waveform(p: &egui::Painter, peaks: &Peaks, clip: &Clip, vis: Rect, state: &TimelineState, color: Color32) {
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

fn diamond(c: Pos2, r: f32, color: Color32) -> Shape {
    Shape::convex_polygon(
        vec![pos2(c.x, c.y - r), pos2(c.x + r, c.y), pos2(c.x, c.y + r), pos2(c.x - r, c.y)],
        color,
        Stroke::NONE,
    )
}

/// dB → fraction of the clip height measured from the bottom. Piecewise linear: -60 dB = 0, 0 dB = 0.7,
/// +12 dB = 1 (so unity gain sits at 70 % height and the usable attenuation range gets most of the clip).
fn db_frac(db: f32) -> f32 {
    if db >= 0.0 {
        0.7 + (db / DB_TOP) * 0.3
    } else {
        0.7 * (1.0 - db / DB_BOT)
    }
}

fn frac_db(f: f32) -> f32 {
    if f >= 0.7 {
        (f - 0.7) / 0.3 * DB_TOP
    } else {
        (1.0 - f / 0.7) * DB_BOT
    }
}

fn gain_db(g: f32) -> f32 {
    (20.0 * g.max(1e-4).log10()).clamp(DB_BOT, DB_TOP)
}

/// Filmstrip of source-time thumbnails across a video/image clip. Non-blocking: misses are queued in the
/// ThumbCache worker and painted on a later frame.
fn draw_filmstrip(
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
    let aspect = if asset.height > 0 { asset.width as f32 / asset.height as f32 } else { 16.0 / 9.0 };
    let step = (h * aspect).max(80.0);
    let pc = p.with_clip_rect(vis);
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    let mut n = ((vis.left() - rect.left()) / step).floor().max(0.0);
    loop {
        let x = rect.left() + n * step;
        if x >= vis.right() || x >= rect.right() {
            break;
        }
        let t = clip.src_time(state.time_at(x)).max(0.0);
        if let Some((tex, [tw, th])) = thumbs.texture(ectx, &asset.path, t, h as u32) {
            let w = h * tw as f32 / th.max(1) as f32;
            pc.image(tex, Rect::from_min_size(pos2(x, rect.top() + band), vec2(w.min(step), h)), uv, Color32::WHITE);
        }
        n += 1.0;
    }
}

fn clip_menu(
    ui: &mut egui::Ui,
    linked: bool,
    enabled: bool,
    act: &mut Option<Act>,
    actions: &mut Vec<crate::hotkeys::Action>,
) {
    use crate::hotkeys::Action;
    if ui.button("Split at Playhead").clicked() {
        *act = Some(Act::Split);
    }
    if ui.button("Delete").clicked() {
        *act = Some(Act::Delete(false));
    }
    if ui.button("Ripple Delete").clicked() {
        *act = Some(Act::Delete(true));
    }
    ui.separator();
    if ui.button("Retime…").clicked() {
        actions.push(Action::Retime);
    }
    if ui.button("Freeze Frame at Playhead").clicked() {
        actions.push(Action::FreezeFrame);
    }
    if ui.button("Add Transition at Start").clicked() {
        actions.push(Action::AddTransition);
    }
    if ui.button("Add Transition at End").clicked() {
        actions.push(Action::AddTransition);
    }
    if ui.button("Auto-cut…").clicked() {
        actions.push(Action::AutoCut);
    }
    ui.separator();
    if ui.button("Nest into Sequence").clicked() {
        actions.push(Action::NestSequence);
    }
    if ui.button("Save as Template…").clicked() {
        actions.push(Action::SaveTemplate);
    }
    ui.menu_button("Color Label", |ui| {
        if ui.button("None").clicked() {
            *act = Some(Act::Label(0));
        }
        for (i, (name, _)) in LABEL_COLORS.iter().enumerate() {
            if ui.button(*name).clicked() {
                *act = Some(Act::Label(i as u8 + 1));
            }
        }
        ui.separator();
        if ui.button("Apply label to asset").clicked() {
            *act = Some(Act::LabelToAsset);
        }
    });
    ui.separator();
    if ui.button(if linked { "Unlink" } else { "Link" }).clicked() {
        *act = Some(Act::Link);
    }
    if ui.button(if enabled { "Disable" } else { "Enable" }).clicked() {
        *act = Some(Act::Enable(!enabled));
    }
}

pub fn show(ui: &mut egui::Ui, state: &mut TimelineState, mut c: TimelineCtx<'_>) -> TimelineResponse {
    let mut out = TimelineResponse::default();
    let pal = *c.palette;
    let full = ui.available_rect_before_wrap();
    ui.allocate_rect(full, Sense::hover());
    let id = ui.id().with("timeline");
    let content_h: f32 = c.project.tracks.iter().map(|t| t.height).sum();
    let vbar_w = if content_h > full.height() - RULER_H - HBAR_H { VBAR_W } else { 0.0 };
    let ruler =
        Rect::from_min_max(pos2(full.left() + state.header_w, full.top()), pos2(full.right(), full.top() + RULER_H));
    let lanes =
        Rect::from_min_max(pos2(ruler.left(), ruler.bottom()), pos2(full.right() - vbar_w, full.bottom() - HBAR_H));
    let header = Rect::from_min_max(pos2(full.left(), lanes.top()), pos2(lanes.left(), full.bottom()));
    let body = Rect::from_min_max(pos2(full.left(), lanes.top()), lanes.max);
    state.lanes_rect = lanes;
    if c.playing {
        state.ensure_visible(*c.playhead);
    } else {
        state.user_panned = false;
    }
    let (mods, pointer, primary_down) = ui.input(|i| (i.modifiers, i.pointer.latest_pos(), i.pointer.primary_down()));

    // ---- scroll / zoom (only when hovered) ----
    if ui.rect_contains_pointer(full) {
        if let Some(pos) = pointer {
            let sx = state.scroll_x;
            let (delta, zoom) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta()));
            if zoom != 1.0 {
                state.zoom_by(zoom.clamp(0.5, 2.0), Some(pos.x));
            }
            if delta != egui::Vec2::ZERO {
                if mods.alt {
                    if let Some(ti) = state.track_at(pos.y, c.project) {
                        let t = &mut c.project.tracks[ti];
                        t.height = (t.height + delta.y * 0.25).clamp(MIN_TRACK_H, MAX_TRACK_H);
                    }
                } else {
                    state.scroll_x = (state.scroll_x - (delta.x / state.zoom) as f64).max(0.0);
                    state.scroll_y -= delta.y;
                }
            }
            if state.scroll_x != sx {
                state.user_panned = true;
            }
        }
    }
    state.scroll_y = state.scroll_y.clamp(0.0, (content_h - lanes.height()).max(0.0));

    let small = egui::TextStyle::Small.resolve(ui.style());
    let font = egui::TextStyle::Body.resolve(ui.style());
    let painter = ui.painter().clone();
    let bp = painter.with_clip_rect(body);
    let lp = painter.with_clip_rect(lanes);
    let rp = painter.with_clip_rect(ruler);
    painter.rect_filled(full, 0, pal.bg);
    painter.rect_filled(Rect::from_min_max(full.min, pos2(header.right(), full.bottom())), 0, pal.header);
    painter.rect_filled(ruler, 0, pal.header);

    // in/out shading (ruler + lanes)
    let (ip, op) = (c.project.in_point, c.project.out_point);
    if ip.is_some() || op.is_some() {
        let xa = ip.map(|t| state.x_at(t)).unwrap_or(lanes.left()).max(lanes.left());
        let xb = op.map(|t| state.x_at(t)).unwrap_or(lanes.right()).min(lanes.right());
        if xb > xa {
            painter.rect_filled(
                Rect::from_min_max(pos2(xa, ruler.top()), pos2(xb, lanes.bottom())),
                0,
                pal.in_out.gamma_multiply(0.12),
            );
        }
        for t in ip.into_iter().chain(op) {
            rp.vline(state.x_at(t), ruler.y_range(), Stroke::new(1.0, pal.in_out));
        }
    }

    // lanes background response first so clips (added later) win hit-testing
    let lanes_resp = ui.interact(lanes, id.with("lanes"), Sense::click_and_drag());
    let ruler_resp = ui.interact(ruler, id.with("ruler"), Sense::click_and_drag());
    // playhead line: registered before the clips so egui's thin-widget tie-break lets the 6 px edge handles
    // beat the 8 px playhead, which still beats clip bodies
    let px = state.x_at(*c.playhead);
    let ph_resp = ui
        .interact(
            Rect::from_x_y_ranges(px - 4.0..=px + 4.0, lanes.y_range()).intersect(lanes),
            id.with("ph"),
            Sense::click_and_drag(),
        )
        .on_hover_cursor(CursorIcon::ResizeHorizontal);

    let mut act: Option<Act> = None;
    let mut click: Option<Id> = None;
    let mut start_move: Option<Id> = None;
    let mut start_trim: Option<(Id, bool)> = None;
    let mut start_vol: Option<Id> = None;
    let mut start_fade: Option<(Id, bool)> = None;
    let mut start_key: Option<(Id, f64)> = None;
    let mut start_trans: Option<(usize, Id)> = None;
    let mut resize: Option<(usize, f32)> = None;
    let mut divider_y: Option<f32> = None;
    let mut key_hits: Vec<(Id, f64, Pos2)> = Vec::new();
    let linked_sel = c.project.expand_links(c.selection);
    let thin = Stroke::new(1.0, pal.border);

    // ---- rows ----
    let mut y = lanes.top() - state.scroll_y;
    for ti in row_order(c.project) {
        let track = &c.project.tracks[ti];
        let row = Rect::from_min_max(pos2(lanes.left(), y), pos2(lanes.right(), y + track.height));
        y += track.height;
        if track.kind == TrackKind::Video {
            divider_y = Some(row.bottom());
        }
        if row.bottom() < lanes.top() || row.top() > lanes.bottom() {
            continue;
        }
        bp.hline(body.x_range(), row.bottom() - 0.5, thin);

        // header cell
        let hr = Rect::from_min_max(pos2(header.left(), row.top()), pos2(header.right(), row.bottom()));
        let tid = id.with(track.id);
        let hresp = ui.interact(hr, tid.with("hdr"), Sense::click());
        let active = c.project.active(ti);
        let bw = vec2(18.0, 16.0);
        let sb = Rect::from_center_size(pos2(hr.right() - 4.0 - bw.x * 0.5, hr.center().y), bw);
        let mb = sb.translate(vec2(-(bw.x + 3.0), 0.0));
        bp.with_clip_rect(Rect::from_min_max(hr.min, pos2(mb.left() - 2.0, hr.bottom()))).text(
            pos2(hr.left() + 6.0, hr.center().y),
            Align2::LEFT_CENTER,
            &track.name,
            font.clone(),
            if active { pal.text } else { pal.text_dim },
        );
        // audio: M = muted; video: V = visible (= !muted)
        let is_video = track.kind == TrackKind::Video;
        let (m_label, m_on) = if is_video { ("V", !track.muted) } else { ("M", track.muted) };
        if toggle_button(ui, &bp, mb, tid.with("m"), m_label, m_on, &pal, &small) {
            act = Some(Act::Mute(ti));
        }
        if toggle_button(ui, &bp, sb, tid.with("s"), "S", track.solo, &pal, &small) {
            act = Some(Act::Solo(ti));
        }
        let (empty, muted, solo) = (track.clips.is_empty(), track.muted, track.solo);
        hresp.context_menu(|ui| {
            if ui.button("Add Video Track").clicked() {
                act = Some(Act::AddTrack(TrackKind::Video));
            }
            if ui.button("Add Audio Track").clicked() {
                act = Some(Act::AddTrack(TrackKind::Audio));
            }
            if ui.add_enabled(empty, egui::Button::new("Remove Track")).clicked() {
                act = Some(Act::RemoveTrack(ti));
            }
            ui.separator();
            if ui.button(if muted { "Unmute" } else { "Mute" }).clicked() {
                act = Some(Act::Mute(ti));
            }
            if ui.button(if solo { "Unsolo" } else { "Solo" }).clicked() {
                act = Some(Act::Solo(ti));
            }
        });

        // clips
        for clip in &track.clips {
            let (x0, x1) = (state.x_at(clip.start), state.x_at(clip.end()));
            if x1 < lanes.left() || x0 > lanes.right() {
                continue;
            }
            let rect = Rect::from_min_max(pos2(x0, row.top() + 1.0), pos2(x1, row.bottom() - 1.0));
            let vis = rect.intersect(lanes);
            let label = c.project.clip_label(clip);
            let mut color = if label != 0 {
                let [r, g, b] = LABEL_COLORS[(label as usize - 1).min(LABEL_COLORS.len() - 1)].1;
                Color32::from_rgb(r, g, b)
            } else {
                pal.clip_color(clip.kind)
            };
            if !clip.enabled || !active {
                color = color.gamma_multiply(0.5);
            }
            lp.rect_filled(rect, 0, color);
            lp.rect_stroke(rect, 0, thin, StrokeKind::Inside);
            if clip.kind == ClipKind::Audio {
                if let Some(asset) = c.project.asset(clip.asset) {
                    if let Some(peaks) = c.waveforms.get(&asset.path, clip.audio_stream) {
                        draw_waveform(&lp, &peaks, clip, vis.shrink(1.0), state, pal.waveform);
                    }
                }
            } else if matches!(clip.kind, ClipKind::Video | ClipKind::Image) {
                if let (Some(th), Some(asset)) = (c.thumbs.as_deref_mut(), c.project.asset(clip.asset)) {
                    draw_filmstrip(&lp, ui.ctx(), state, clip, asset, rect, vis.shrink(1.0), th);
                }
            }
            let name_pc = lp.with_clip_rect(vis.shrink(1.0));
            let name_pos = pos2(vis.left() + 4.0, rect.top() + 2.0);
            if clip.kind == ClipKind::Sequence {
                name_pc.text(name_pos, Align2::LEFT_TOP, format!("⧉ {}", clip.name), font.clone(), pal.text);
            } else {
                name_pc.text(name_pos, Align2::LEFT_TOP, &clip.name, font.clone(), pal.text);
            }
            if clip.is_retimed() {
                let badge = if clip.freeze.is_some() {
                    "❄".to_string()
                } else {
                    let mut s = String::new();
                    if (clip.speed - 1.0).abs() > 1e-9 {
                        s = if clip.speed < 1.0 {
                            format!("{:.0} %", clip.speed * 100.0)
                        } else if clip.speed.fract().abs() < 1e-6 {
                            format!("{}x", clip.speed as i64)
                        } else {
                            format!("{:.2}x", clip.speed)
                        };
                    }
                    if clip.reverse {
                        if !s.is_empty() {
                            s.push(' ');
                        }
                        s.push('⟲');
                    }
                    s
                };
                name_pc.text(
                    pos2(rect.right() - 3.0, rect.top() + 2.0),
                    Align2::RIGHT_TOP,
                    badge,
                    small.clone(),
                    pal.text,
                );
            }
            if clip.kind == ClipKind::Audio {
                // volume line (dB mapped) + fade ramps/handles over the waveform
                let vs = Stroke::new(1.0, pal.accent);
                if clip.volume.is_animated() {
                    let mut last: Option<Pos2> = None;
                    let mut x = vis.left();
                    while x <= vis.right() + 4.0 {
                        let lt = (state.time_at(x) - clip.start).clamp(0.0, clip.duration);
                        let y = rect.bottom() - db_frac(gain_db(clip.volume.at(lt) as f32)) * rect.height();
                        let pnt = pos2(x.min(vis.right()), y);
                        if let Some(l) = last {
                            name_pc.line_segment([l, pnt], vs);
                        }
                        last = Some(pnt);
                        x += 4.0;
                    }
                } else {
                    let y = rect.bottom() - db_frac(gain_db(clip.volume.value as f32)) * rect.height();
                    name_pc.hline(vis.x_range(), y, vs);
                }
                let fs = Stroke::new(1.0, pal.text);
                let fi_x = (rect.left() + clip.fade_in as f32 * state.zoom).min(rect.right());
                let fo_x = (rect.right() - clip.fade_out as f32 * state.zoom).max(rect.left());
                if clip.fade_in > 0.0 {
                    name_pc.line_segment([pos2(rect.left(), rect.bottom()), pos2(fi_x, rect.top())], fs);
                }
                if clip.fade_out > 0.0 {
                    name_pc.line_segment([pos2(fo_x, rect.top()), pos2(rect.right(), rect.bottom())], fs);
                }
                for hx in [fi_x, fo_x] {
                    lp.rect_filled(Rect::from_center_size(pos2(hx, rect.top() + 3.0), vec2(6.0, 6.0)), 0, pal.text);
                }
            }
            for t in clip.key_times() {
                let kx = state.x_at(clip.start + t);
                if kx < lanes.left() - 6.0 || kx > lanes.right() + 6.0 {
                    continue;
                }
                let kp = pos2(kx, rect.bottom() - 5.0);
                lp.add(diamond(kp, 4.0, pal.keyframe));
                key_hits.push((clip.id, t, kp));
            }
            let selected = c.selection.contains(&clip.id);
            if selected {
                lp.rect_stroke(rect, 0, Stroke::new(2.0, pal.selection), StrokeKind::Inside);
            } else if linked_sel.contains(&clip.id) {
                lp.rect_stroke(rect, 0, Stroke::new(1.0, pal.selection), StrokeKind::Inside);
            }

            // interaction: body, then volume line, then edges, then fade handles on top
            let cid = id.with(clip.id);
            let br = ui.interact(vis, cid, Sense::click_and_drag());
            if br.clicked() {
                click = Some(clip.id);
            }
            if br.double_clicked() {
                c.selection.clear();
                c.selection.push(clip.id);
                if clip.kind == ClipKind::Sequence {
                    out.open_sequence = Some(clip.sequence);
                } else if let Some(p) = br.interact_pointer_pos() {
                    *c.playhead = c.project.snap_frame(state.time_at(p.x).max(0.0));
                    out.seeked = true;
                }
            }
            if br.drag_started_by(egui::PointerButton::Primary) {
                start_move = Some(clip.id);
            }
            let (linked, enabled) = (clip.link != 0, clip.enabled);
            let mut rclick = br.secondary_clicked();
            br.context_menu(|ui| clip_menu(ui, linked, enabled, &mut act, &mut out.actions));
            if clip.kind == ClipKind::Audio {
                if let Some(pos) = pointer {
                    if pos.x >= vis.left() && pos.x <= vis.right() {
                        let lt = (state.time_at(pos.x) - clip.start).clamp(0.0, clip.duration);
                        let y = rect.bottom() - db_frac(gain_db(clip.volume.at(lt) as f32)) * rect.height();
                        let vr = Rect::from_x_y_ranges(vis.x_range(), (y - 4.0)..=(y + 4.0)).intersect(vis);
                        if vr.is_positive() {
                            let r = ui
                                .interact(vr, cid.with("vol"), Sense::drag())
                                .on_hover_cursor(CursorIcon::ResizeVertical);
                            if r.drag_started_by(egui::PointerButton::Primary) {
                                start_vol = Some(clip.id);
                            }
                        }
                    }
                }
            }
            if rect.width() >= 3.0 * EDGE_W {
                let le = Rect::from_min_max(rect.min, pos2(rect.left() + EDGE_W, rect.bottom())).intersect(lanes);
                let re = Rect::from_min_max(pos2(rect.right() - EDGE_W, rect.top()), rect.max).intersect(lanes);
                for (er, is_start, salt) in [(le, true, "l"), (re, false, "r")] {
                    if !er.is_positive() {
                        continue;
                    }
                    let r = ui
                        .interact(er, cid.with(salt), Sense::click_and_drag())
                        .on_hover_cursor(CursorIcon::ResizeHorizontal);
                    if r.clicked() {
                        click = Some(clip.id);
                    }
                    if r.drag_started_by(egui::PointerButton::Primary) {
                        start_trim = Some((clip.id, is_start));
                    }
                    rclick |= r.secondary_clicked();
                    r.context_menu(|ui| clip_menu(ui, linked, enabled, &mut act, &mut out.actions));
                }
            }
            if clip.kind == ClipKind::Audio {
                let fi_x = (rect.left() + clip.fade_in as f32 * state.zoom).min(rect.right());
                let fo_x = (rect.right() - clip.fade_out as f32 * state.zoom).max(rect.left());
                for (hx, is_out, salt) in [(fi_x, false, "fi"), (fo_x, true, "fo")] {
                    let fr = Rect::from_center_size(pos2(hx, rect.top() + 3.0), vec2(10.0, 10.0)).intersect(lanes);
                    if !fr.is_positive() {
                        continue;
                    }
                    let r =
                        ui.interact(fr, cid.with(salt), Sense::drag()).on_hover_cursor(CursorIcon::ResizeHorizontal);
                    if r.drag_started_by(egui::PointerButton::Primary) {
                        start_fade = Some((clip.id, is_out));
                    }
                }
            }
            if rclick && !selected {
                c.selection.clear();
                c.selection.push(clip.id);
            }
        }

        // transitions: an overlapping band centred on each valid cut
        for tr in &track.transitions {
            let Some((_, right)) = track.transition_pair(tr) else { continue };
            let (wa, wb) = tr.window(right.start);
            let (xa, xb) = (state.x_at(wa), state.x_at(wb));
            if xb < lanes.left() || xa > lanes.right() {
                continue;
            }
            let band = Rect::from_min_max(pos2(xa, row.top() + 1.0), pos2(xb, row.bottom() - 1.0));
            let bvis = band.intersect(lanes);
            lp.rect_filled(band, 0, pal.bg.gamma_multiply(0.5));
            lp.rect_stroke(band, 0, Stroke::new(1.0, pal.accent), StrokeKind::Inside);
            if bvis.is_positive() {
                lp.with_clip_rect(bvis).text(
                    band.center(),
                    Align2::CENTER_CENTER,
                    tr.kind.name(),
                    small.clone(),
                    pal.text,
                );
                let br = ui.interact(bvis, id.with(tr.id), Sense::click());
                br.context_menu(|ui| {
                    if ui.button("Remove Transition").clicked() {
                        act = Some(Act::RemoveTransition(tr.id));
                    }
                });
            }
            for (er, salt) in [
                (Rect::from_min_max(band.min, pos2(band.left() + EDGE_W, band.bottom())), "a"),
                (Rect::from_min_max(pos2(band.right() - EDGE_W, band.top()), band.max), "b"),
            ] {
                let er = er.intersect(lanes);
                if !er.is_positive() {
                    continue;
                }
                let r = ui
                    .interact(er, id.with(tr.id).with(salt), Sense::click_and_drag())
                    .on_hover_cursor(CursorIcon::ResizeHorizontal);
                if r.drag_started_by(egui::PointerButton::Primary) {
                    start_trans = Some((ti, tr.id));
                }
                r.context_menu(|ui| {
                    if ui.button("Remove Transition").clicked() {
                        act = Some(Act::RemoveTransition(tr.id));
                    }
                });
            }
        }

        // track resize handle: spans the full width (header + lanes), registered after the clips so it wins
        let handle = Rect::from_min_max(pos2(body.left(), row.bottom() - HANDLE_H), pos2(body.right(), row.bottom()));
        let hd = ui.interact(handle, tid.with("h"), Sense::drag()).on_hover_cursor(CursorIcon::ResizeVertical);
        if hd.dragged() {
            resize = Some((ti, hd.drag_delta().y));
        }
    }

    // keyframe diamonds: registered after everything in the rows so the small targets win hit-testing
    for (i, &(kcid, kt, kp)) in key_hits.iter().enumerate() {
        let r = ui
            .interact(Rect::from_center_size(kp, vec2(10.0, 10.0)), id.with(("key", i)), Sense::click_and_drag())
            .on_hover_cursor(CursorIcon::ResizeHorizontal);
        if r.drag_started_by(egui::PointerButton::Primary) {
            start_key = Some((kcid, kt));
        }
        r.context_menu(|ui| {
            for e in Ease::ALL {
                if ui.button(e.name()).clicked() {
                    act = Some(Act::SetEase(kcid, kt, e));
                }
            }
            for (name, e) in Ease::PRESETS {
                if ui.button(name).clicked() {
                    act = Some(Act::SetEase(kcid, kt, e));
                }
            }
            ui.separator();
            if ui.button("Delete keyframe(s)").clicked() {
                act = Some(Act::DelKeys(kcid, kt));
            }
        });
    }

    // auto-cut preview: shade the kept ranges over the lanes
    for &(ka, kb) in c.keep_ranges {
        let xa = state.x_at(ka).max(lanes.left());
        let xb = state.x_at(kb).min(lanes.right());
        if xb > xa {
            lp.rect_filled(
                Rect::from_min_max(pos2(xa, lanes.top()), pos2(xb, lanes.bottom())),
                0,
                pal.selection.gamma_multiply(0.15),
            );
        }
    }
    if let Some(dy) = divider_y {
        if c.project.tracks.iter().any(|t| t.kind == TrackKind::Audio) {
            bp.hline(body.x_range(), dy - 1.0, Stroke::new(2.0, pal.border));
        }
    }

    // ---- ruler ticks ----
    let (major, minor) = tick_step(state.zoom);
    let ratio = (major / minor).round() as i64;
    let t_end = state.time_at(ruler.right());
    let mut i = (state.scroll_x / minor).floor() as i64;
    loop {
        let t = i as f64 * minor;
        if t > t_end {
            break;
        }
        let x = state.x_at(t);
        if i % ratio == 0 {
            rp.vline(x, Rangef::new(ruler.bottom() - 8.0, ruler.bottom()), Stroke::new(1.0, pal.text_dim));
            rp.text(
                pos2(x + 3.0, ruler.top() + 1.0),
                Align2::LEFT_TOP,
                tick_label(t, major),
                small.clone(),
                pal.text_dim,
            );
        } else {
            rp.vline(x, Rangef::new(ruler.bottom() - 4.0, ruler.bottom()), thin);
        }
        i += 1;
    }
    painter.hline(ruler.x_range(), ruler.bottom() - 0.5, thin);
    painter.vline(header.right() - 0.5, full.y_range(), thin);

    // ---- playhead (recomputed: a double-click seek above moves it this frame) ----
    let px = state.x_at(*c.playhead);
    if px >= lanes.left() - 1.0 && px <= lanes.right() + 1.0 {
        let pp = painter.with_clip_rect(Rect::from_min_max(ruler.min, lanes.max));
        pp.vline(px, Rangef::new(ruler.top(), lanes.bottom()), Stroke::new(1.5, pal.playhead));
        pp.add(Shape::convex_polygon(
            vec![pos2(px - 5.0, ruler.top()), pos2(px + 5.0, ruler.top()), pos2(px, ruler.top() + 7.0)],
            pal.playhead,
            Stroke::NONE,
        ));
    }
    // scrub: ruler press/drag or playhead line drag
    let scrub_x = if primary_down && (ruler_resp.is_pointer_button_down_on() || ruler_resp.dragged()) {
        ruler_resp.interact_pointer_pos()
    } else if ph_resp.dragged_by(egui::PointerButton::Primary) {
        ph_resp.interact_pointer_pos()
    } else {
        None
    };
    if let Some(p) = scrub_x {
        *c.playhead = c.project.snap_frame(state.time_at(p.x).max(0.0));
        out.seeked = true;
    }

    // ---- lanes background: deselect / context menu / dnd ----
    if lanes_resp.clicked() && !mods.ctrl {
        c.selection.clear();
    }
    lanes_resp.context_menu(|ui| {
        if ui.button("Add Video Track").clicked() {
            act = Some(Act::AddTrack(TrackKind::Video));
        }
        if ui.button("Add Audio Track").clicked() {
            act = Some(Act::AddTrack(TrackKind::Audio));
        }
    });
    if let Some(pos) = pointer {
        let drop_t = |state: &TimelineState, p: &Project| {
            let t = state.time_at(pos.x).max(0.0);
            if c.snap {
                p.snap_frame(t)
            } else {
                t
            }
        };
        if let Some(payload) = lanes_resp.dnd_hover_payload::<DragPayload>() {
            let t = drop_t(state, c.project);
            let dur = match &*payload {
                DragPayload::Asset(aid) => {
                    c.project.asset(*aid).map(|a| if a.kind == ClipKind::Image { 5.0 } else { a.duration })
                }
                DragPayload::Path(_) | DragPayload::Template(_) => None,
                DragPayload::Sequence(sid) => Some(c.project.sequence_duration(*sid)),
            }
            .unwrap_or(2.0);
            let ti = state.track_at(pos.y, c.project).or_else(|| c.project.video_tracks().first().copied());
            if let Some((ti, top)) = ti.and_then(|ti| Some((ti, row_top(state, c.project, ti)?))) {
                let h = c.project.tracks[ti].height;
                let ghost =
                    Rect::from_min_max(pos2(state.x_at(t), top + 1.0), pos2(state.x_at(t + dur), top + h - 1.0));
                lp.rect_filled(ghost, 0, pal.selection.gamma_multiply(0.3));
                lp.rect_stroke(ghost, 0, Stroke::new(1.0, pal.selection), StrokeKind::Inside);
            }
        }
        if let Some(payload) = lanes_resp.dnd_release_payload::<DragPayload>() {
            let t = drop_t(state, c.project);
            let ti = state.track_at(pos.y, c.project);
            match &*payload {
                DragPayload::Asset(aid) => act = Some(Act::DropAsset(*aid, t, ti)),
                DragPayload::Path(p) => out.dropped_files.push((PathBuf::from(p), t, ti)),
                other => out.dropped_other.push((other.clone(), t, ti)),
            }
        }
    }

    // ---- scrollbars ----
    {
        let hbar = Rect::from_min_max(pos2(lanes.left(), lanes.bottom()), pos2(full.right(), full.bottom()));
        painter.rect_filled(hbar, 0, pal.panel);
        let vis_w = (lanes.width() / state.zoom) as f64;
        let total = (c.project.duration() * 1.1).max(state.scroll_x + vis_w).max(1e-6);
        let max_sx = (total - vis_w).max(0.0);
        let bw = lanes.width().max(1.0);
        let tw = ((vis_w / total) as f32 * bw).clamp(20.0f32.min(bw), bw);
        let tx = lanes.left() + ((state.scroll_x / total) as f32 * bw).min(bw - tw).max(0.0);
        let thumb = Rect::from_min_max(pos2(tx, hbar.top() + 2.0), pos2(tx + tw, full.bottom() - 2.0));
        let hb = ui.interact(
            Rect::from_min_max(hbar.min, pos2(lanes.right(), full.bottom())),
            id.with("hbar"),
            Sense::click(),
        );
        let ht = ui.interact(thumb, id.with("hthumb"), Sense::drag());
        if ht.dragged() {
            state.scroll_x = (state.scroll_x + (ht.drag_delta().x / bw) as f64 * total).clamp(0.0, max_sx);
            state.user_panned = true;
        }
        if hb.clicked() {
            if let Some(p) = hb.interact_pointer_pos() {
                let dir = if p.x < thumb.left() {
                    -1.0
                } else if p.x > thumb.right() {
                    1.0
                } else {
                    0.0
                };
                state.scroll_x = (state.scroll_x + dir * vis_w).clamp(0.0, max_sx);
                state.user_panned = true;
            }
        }
        let fill = if ht.dragged() {
            pal.accent
        } else if ht.hovered() {
            pal.text_dim
        } else {
            pal.border
        };
        painter.rect_filled(thumb, CornerRadius::same(3), fill);
        if vbar_w > 0.0 {
            let vbar = Rect::from_min_max(pos2(lanes.right(), lanes.top()), pos2(full.right(), lanes.bottom()));
            painter.rect_filled(vbar, 0, pal.panel);
            let bh = vbar.height().max(1.0);
            let max_sy = (content_h - lanes.height()).max(0.0);
            let th = (lanes.height() / content_h * bh).clamp(20.0f32.min(bh), bh);
            let ty = vbar.top() + (state.scroll_y / content_h * bh).min(bh - th).max(0.0);
            let vthumb = Rect::from_min_max(pos2(vbar.left() + 2.0, ty), pos2(vbar.right() - 2.0, ty + th));
            let vb = ui.interact(vbar, id.with("vbar"), Sense::click());
            let vt = ui.interact(vthumb, id.with("vthumb"), Sense::drag());
            if vt.dragged() {
                state.scroll_y = (state.scroll_y + vt.drag_delta().y / bh * content_h).clamp(0.0, max_sy);
            }
            if vb.clicked() {
                if let Some(p) = vb.interact_pointer_pos() {
                    let dir = if p.y < vthumb.top() {
                        -1.0
                    } else if p.y > vthumb.bottom() {
                        1.0
                    } else {
                        0.0
                    };
                    state.scroll_y = (state.scroll_y + dir * lanes.height()).clamp(0.0, max_sy);
                }
            }
            let vfill = if vt.dragged() {
                pal.accent
            } else if vt.hovered() {
                pal.text_dim
            } else {
                pal.border
            };
            painter.rect_filled(vthumb, CornerRadius::same(3), vfill);
        }
    }

    // ---- apply deferred bits ----
    if let Some((ti, dy)) = resize {
        // ponytail: track height is cosmetic — no undo snapshot, not marked as an edit
        let t = &mut c.project.tracks[ti];
        t.height = (t.height + dy).clamp(MIN_TRACK_H, MAX_TRACK_H);
    }
    if let Some(cid) = click {
        // click = whole link group, Alt+click = just this clip, Ctrl toggles the group
        let group = if mods.alt { vec![cid] } else { c.project.expand_links(&[cid]) };
        if mods.ctrl {
            if c.selection.contains(&cid) {
                c.selection.retain(|x| !group.contains(x));
            } else {
                for g in group {
                    if !c.selection.contains(&g) {
                        c.selection.push(g);
                    }
                }
            }
        } else {
            *c.selection = group;
        }
    }
    if let Some(a) = act {
        let p = &mut *c.project;
        (c.undo)(p);
        let ids = p.expand_links(c.selection);
        match a {
            Act::Split => {
                p.split_at(*c.playhead, Some(&ids));
            }
            Act::Delete(ripple) => {
                p.delete_clips(&ids, ripple);
                c.selection.clear();
            }
            Act::Link => p.toggle_link(&ids),
            Act::Enable(on) => p.set_enabled(c.selection, on),
            Act::AddTrack(kind) => {
                p.add_track(kind);
            }
            Act::RemoveTrack(ti) => p.remove_track(ti),
            Act::Mute(ti) => p.tracks[ti].muted = !p.tracks[ti].muted,
            Act::Solo(ti) => p.tracks[ti].solo = !p.tracks[ti].solo,
            Act::DropAsset(aid, t, ti) => {
                let vt = ti.filter(|&i| p.tracks[i].kind == TrackKind::Video);
                p.insert_asset_clips(aid, t, vt);
            }
            Act::Label(l) => {
                for id in &ids {
                    if let Some(cl) = p.clip_mut(*id) {
                        cl.label = l;
                    }
                }
            }
            Act::LabelToAsset => {
                let pairs: Vec<(Id, u8)> = ids
                    .iter()
                    .filter_map(|&id| p.clip(id).filter(|cl| cl.uses_asset()).map(|cl| (cl.asset, p.clip_label(cl))))
                    .collect();
                for (aid, l) in pairs {
                    if let Some(a) = p.asset_mut(aid) {
                        a.label = l;
                    }
                }
            }
            Act::SetEase(cid, t, e) => {
                if let Some(cl) = p.clip_mut(cid) {
                    for a in cl.all_animated_mut() {
                        a.set_ease_at(t, e);
                    }
                }
            }
            Act::DelKeys(cid, t) => {
                if let Some(cl) = p.clip_mut(cid) {
                    for a in cl.all_animated_mut() {
                        if a.has_key_at(t) {
                            a.toggle_key(t);
                        }
                    }
                }
            }
            Act::RemoveTransition(tid) => p.remove_transition(tid),
        }
        out.edited = true;
    }

    // ---- gestures ----
    let origin = ui.input(|i| i.pointer.press_origin()).or(pointer).unwrap_or(Pos2::ZERO);
    if let Some(cid) = start_move.or(start_trim.map(|(id, _)| id)) {
        if !c.selection.contains(&cid) {
            if !mods.ctrl {
                c.selection.clear();
            }
            c.selection.push(cid);
        }
    }
    if let Some(cid) = start_move {
        if let Some(tr) = c.project.track_of(cid) {
            let ids = c.project.expand_links(c.selection);
            let orig = ids.iter().map(|&id| c.project.clip(id).map(|cl| cl.start).unwrap_or(0.0)).collect();
            let kind = c.project.tracks[tr].kind;
            let g = Gesture::Move { ids, orig, kind, tr, dt: 0.0, dtrack: 0 };
            state.drag = Some(Drag { origin, before: c.project.clone(), g });
        }
    } else if let Some((cid, start)) = start_trim {
        if let Some(clip) = c.project.clip(cid) {
            let edge = if start { clip.start } else { clip.end() };
            // linked clips trim together when their edge coincides
            let ids: Vec<Id> = c
                .project
                .linked(cid)
                .into_iter()
                .filter(|&id| {
                    c.project
                        .clip(id)
                        .map_or(false, |cl| ((if start { cl.start } else { cl.end() }) - edge).abs() < 1e-6)
                })
                .collect();
            let g = Gesture::Trim { ids, start, edge, changed: false };
            state.drag = Some(Drag { origin, before: c.project.clone(), g });
        }
    } else if let Some(cid) = start_vol {
        state.drag = Some(Drag { origin, before: c.project.clone(), g: Gesture::Volume { id: cid, changed: false } });
    } else if let Some((cid, fout)) = start_fade {
        state.drag =
            Some(Drag { origin, before: c.project.clone(), g: Gesture::Fade { id: cid, out: fout, changed: false } });
    } else if let Some((cid, kt)) = start_key {
        state.drag =
            Some(Drag { origin, before: c.project.clone(), g: Gesture::Keys { id: cid, t: kt, changed: false } });
    } else if let Some((tri, tid)) = start_trans {
        state.drag = Some(Drag {
            origin,
            before: c.project.clone(),
            g: Gesture::TransDur { track: tri, id: tid, changed: false },
        });
    }
    let hover_tr = pointer.and_then(|pos| state.track_at(pos.y, c.project));
    // scalar copies + closures so the gesture arms can use geometry while `state.drag` is mutably borrowed
    let (lx, sx, sy, ltop) = (lanes.left(), state.scroll_x, state.scroll_y, lanes.top());
    let zoom0 = state.zoom;
    let t_at = move |x: f32| sx + ((x - lx) / zoom0) as f64;
    let row_top_of = move |p: &Project, ti: usize| -> Option<f32> {
        let mut top = ltop - sy;
        for i in row_order(p) {
            if i == ti {
                return Some(top);
            }
            top += p.tracks[i].height;
        }
        None
    };
    if let (Some(drag), Some(pos)) = (state.drag.as_mut(), pointer) {
        let zoom = zoom0;
        let dx = (pos.x - drag.origin.x) as f64 / zoom as f64;
        let thr = (SNAP_PX / zoom) as f64;
        let p = &mut *c.project;
        match &mut drag.g {
            Gesture::Move { ids, orig, kind, tr, dt, dtrack } => {
                let mut want = dx;
                if c.snap {
                    let s0 = orig.first().copied().unwrap_or(0.0);
                    want = p.snap_frame(s0 + want) - s0;
                    let mut best: Option<f64> = None;
                    for (&id, &s) in ids.iter().zip(orig.iter()) {
                        let Some(cl) = p.clip(id) else { continue };
                        for edge in [s + want, s + want + cl.duration] {
                            if let Some(tgt) = snap_target(edge, thr, p, *c.playhead, ids) {
                                let adj = tgt - edge;
                                if best.map_or(true, |b: f64| adj.abs() < b.abs()) {
                                    best = Some(adj);
                                }
                            }
                        }
                    }
                    want += best.unwrap_or(0.0);
                }
                let min_start = orig.iter().copied().fold(f64::INFINITY, f64::min);
                want = want.max(-min_start);
                // destination row = same-kind track under the pointer (keeps the last one while outside)
                let list = if *kind == TrackKind::Video { p.video_tracks() } else { p.audio_tracks() };
                let pos_of = |t: usize| list.iter().position(|&x| x == t);
                let want_tr = match (hover_tr.and_then(pos_of), pos_of(*tr)) {
                    (Some(h), Some(o)) => h as i32 - o as i32,
                    _ => *dtrack,
                };
                let (ddt, ddtr) = (want - *dt, want_tr - *dtrack);
                if ddt.abs() > 1e-9 || ddtr != 0 {
                    if p.move_clips(ids, ddt, ddtr, Some(*kind)) {
                        *dt = want;
                        *dtrack = want_tr;
                    } else if ddtr != 0 && ddt.abs() > 1e-9 {
                        if p.move_clips(ids, ddt, 0, Some(*kind)) {
                            *dt = want;
                        } else if p.move_clips(ids, 0.0, ddtr, Some(*kind)) {
                            *dtrack = want_tr;
                        }
                    }
                }
            }
            Gesture::Trim { ids, start, edge, changed } => {
                let mut want = *edge + dx;
                if c.snap {
                    want = p.snap_frame(want);
                    if let Some(t) = snap_target(want, thr, p, *c.playhead, ids) {
                        want = t;
                    }
                }
                // all-or-nothing (like move_clips): linked clips keep identical extents when one is blocked
                let mut upd = Vec::with_capacity(ids.len());
                for &id in ids.iter() {
                    let Some((ti, ci)) = p.find(id) else { continue };
                    let mut tmp = p.tracks[ti].clips[ci].clone();
                    if *start {
                        let hr = p.head_room(&tmp);
                        tmp.trim_start(want, hr);
                    } else {
                        let md = p.max_clip_duration(&tmp);
                        tmp.trim_end(want, md);
                    }
                    if !p.tracks[ti].fits(tmp.start, tmp.duration, &[id]) {
                        upd.clear();
                        break;
                    }
                    upd.push((ti, ci, tmp));
                }
                for (ti, ci, tmp) in upd {
                    let cl = &p.tracks[ti].clips[ci];
                    if cl.start != tmp.start || cl.duration != tmp.duration {
                        p.tracks[ti].clips[ci] = tmp;
                        *changed = true;
                    }
                }
            }
            Gesture::Volume { id, changed } => {
                if let Some((ti, ci)) = p.find(*id) {
                    if let Some(top) = row_top_of(p, ti) {
                        let h = (p.tracks[ti].height - 2.0).max(1.0);
                        let frac = ((top + p.tracks[ti].height - 1.0 - pos.y) / h).clamp(0.0, 1.0);
                        let db = frac_db(frac);
                        let gain = if db <= DB_BOT + 0.25 { 0.0 } else { 10f64.powf(db as f64 / 20.0) };
                        let cl = &mut p.tracks[ti].clips[ci];
                        if cl.volume.is_animated() {
                            let lt = (t_at(pos.x) - cl.start).clamp(0.0, cl.duration);
                            cl.volume.set_at(lt, gain);
                            *changed = true;
                        } else if (cl.volume.value - gain).abs() > 1e-9 {
                            cl.volume.value = gain;
                            *changed = true;
                        }
                    }
                }
            }
            Gesture::Fade { id, out: fout, changed } => {
                if let Some(cl) = p.clip_mut(*id) {
                    let t = t_at(pos.x);
                    let v = if *fout {
                        (cl.end() - t).clamp(0.0, cl.duration)
                    } else {
                        (t - cl.start).clamp(0.0, cl.duration)
                    };
                    let dst = if *fout { &mut cl.fade_out } else { &mut cl.fade_in };
                    if (*dst - v).abs() > 1e-9 {
                        *dst = v;
                        *changed = true;
                    }
                }
            }
            Gesture::Keys { id, t, changed } => {
                let nt = p.snap_frame(t_at(pos.x));
                if let Some(cl) = p.clip_mut(*id) {
                    let lt = (nt - cl.start).clamp(0.0, cl.duration);
                    if (lt - *t).abs() > 1e-9 {
                        cl.move_keys(*t, lt);
                        *t = lt;
                        *changed = true;
                    }
                }
            }
            Gesture::TransDur { track, id, changed } => {
                let tr = &p.tracks[*track];
                let cut = tr
                    .transitions
                    .iter()
                    .find(|t| t.id == *id)
                    .and_then(|t| tr.transition_pair(t))
                    .map(|(_, r)| r.start);
                if let Some(cut) = cut {
                    let d = ((t_at(pos.x) - cut).abs() * 2.0).max(0.1);
                    if let Some(tr) = p.tracks[*track].transitions.iter_mut().find(|t| t.id == *id) {
                        if (tr.duration - d).abs() > 1e-9 {
                            tr.duration = d;
                            *changed = true;
                        }
                    }
                }
            }
        }
    }
    // ---- edge auto-scroll while dragging (playhead scrub, move/trim, keyframes, …) ----
    if state.drag.is_some() || scrub_x.is_some() {
        if let Some(pos) = pointer {
            let over = if pos.x < lanes.left() + SCROLL_MARGIN {
                pos.x - (lanes.left() + SCROLL_MARGIN)
            } else if pos.x > lanes.right() - SCROLL_MARGIN {
                pos.x - (lanes.right() - SCROLL_MARGIN)
            } else {
                0.0
            };
            if over != 0.0 {
                let ds = (over as f64 * 0.2) / state.zoom as f64; // ~proportional to the overshoot, per frame
                let ns = (state.scroll_x + ds).max(0.0);
                let applied = ns - state.scroll_x;
                if applied != 0.0 {
                    state.scroll_x = ns;
                    state.user_panned = true;
                    // keep pixel-based gestures (move/trim use pointer - origin) tracking the scrolled content
                    if let Some(d) = state.drag.as_mut() {
                        d.origin.x -= (applied * state.zoom as f64) as f32;
                    }
                    ui.ctx().request_repaint();
                }
            }
        }
    }
    if !primary_down {
        if let Some(d) = state.drag.take() {
            let edited = match d.g {
                Gesture::Move { dt, dtrack, .. } => dt != 0.0 || dtrack != 0,
                Gesture::Trim { changed, .. }
                | Gesture::Volume { changed, .. }
                | Gesture::Fade { changed, .. }
                | Gesture::Keys { changed, .. }
                | Gesture::TransDur { changed, .. } => changed,
            };
            if edited {
                (c.undo)(&d.before);
                out.edited = true;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Track;

    #[test]
    fn nearest_within_threshold() {
        let cands = [0.0, 1.0, 2.5, 10.0];
        assert_eq!(nearest(1.1, 0.2, cands.iter().copied()), Some(1.0));
        assert_eq!(nearest(1.8, 0.5, cands.iter().copied()), None);
        assert_eq!(nearest(1.8, 0.8, cands.iter().copied()), Some(2.5));
        assert_eq!(nearest(0.04, 0.05, cands.iter().copied()), Some(0.0));
        assert_eq!(nearest(5.0, 100.0, [].into_iter()), None);
    }

    #[test]
    fn snap_targets_exclude_moving_clips() {
        let mut p = Project::new();
        p.tracks[0].clips.push(Clip::new(7, ClipKind::Video, "a", 2.0, 3.0));
        p.tracks[0].clips.push(Clip::new(8, ClipKind::Video, "b", 6.0, 1.0));
        // end of clip 7 = 5.0 is the nearest target
        assert_eq!(snap_target(5.1, 0.2, &p, 20.0, &[]), Some(5.0));
        // excluded → falls back to nothing in range
        assert_eq!(snap_target(5.1, 0.2, &p, 20.0, &[7]), None);
        // playhead and 0 count too
        assert_eq!(snap_target(19.9, 0.2, &p, 20.0, &[]), Some(20.0));
        assert_eq!(snap_target(0.1, 0.2, &p, 20.0, &[]), Some(0.0));
    }

    #[test]
    fn tick_spacing_scales_with_zoom() {
        assert_eq!(tick_step(40.0), (2.0, 0.5));
        assert_eq!(tick_step(2000.0), (0.05, 0.01));
        assert_eq!(tick_step(0.5), (300.0, 60.0));
        let mut last = 0.0;
        for z in [0.5, 1.0, 5.0, 40.0, 200.0, 2000.0] {
            let (major, minor) = tick_step(z);
            assert!(major * z as f64 >= 80.0 || major == 3600.0);
            assert!(major > minor && ((major / minor) - (major / minor).round()).abs() < 1e-9);
            assert!(major <= last || last == 0.0);
            last = major;
        }
        assert_eq!(tick_label(65.0, 5.0), "1:05");
        assert_eq!(tick_label(2.5, 0.5), "0:02.5");
        assert_eq!(tick_label(0.25, 0.05), "0:00.25");
    }

    #[test]
    fn track_at_maps_rows() {
        let mut p = Project::new(); // V1 A1
        p.add_track(TrackKind::Video); // V2 at index 1
        p.add_track(TrackKind::Audio); // A2 at index 3
        assert_eq!(
            p.tracks.iter().map(|t| t.kind).collect::<Vec<_>>(),
            [TrackKind::Video, TrackKind::Video, TrackKind::Audio, TrackKind::Audio]
        );
        let heights = [50.0, 70.0, 40.0, 60.0];
        for (t, h) in p.tracks.iter_mut().zip(heights) {
            t.height = h;
        }
        let mut s = TimelineState {
            lanes_rect: Rect::from_min_max(pos2(100.0, 200.0), pos2(900.0, 500.0)),
            ..Default::default()
        };
        // display order: V2 (70) V1 (50) A1 (40) A2 (60)
        assert_eq!(row_order(&p).collect::<Vec<_>>(), [1, 0, 2, 3]);
        assert_eq!(s.track_at(210.0, &p), Some(1));
        assert_eq!(s.track_at(269.9, &p), Some(1));
        assert_eq!(s.track_at(270.0, &p), Some(0));
        assert_eq!(s.track_at(330.0, &p), Some(2));
        assert_eq!(s.track_at(375.0, &p), Some(3));
        assert_eq!(s.track_at(420.0, &p), None);
        assert_eq!(s.track_at(150.0, &p), None); // above the lanes (ruler)
        s.scroll_y = 100.0;
        assert_eq!(s.track_at(210.0, &p), Some(0));
        assert_eq!(row_top(&s, &p, 2), Some(220.0));
        let _ = Track::new(1, TrackKind::Video, "x");
    }

    #[test]
    fn time_x_roundtrip() {
        let mut s = TimelineState {
            lanes_rect: Rect::from_min_max(pos2(100.0, 0.0), pos2(900.0, 100.0)),
            ..Default::default()
        };
        s.zoom = 50.0;
        s.scroll_x = 2.0;
        assert!((s.time_at(s.x_at(7.25)) - 7.25).abs() < 1e-4);
        assert_eq!(s.x_at(2.0), 100.0);
    }

    #[test]
    fn ensure_visible_follows_unless_user_panned() {
        let mut s = TimelineState {
            lanes_rect: Rect::from_min_max(pos2(100.0, 0.0), pos2(900.0, 100.0)),
            zoom: 40.0, // 20 s visible
            scroll_x: 30.0,
            ..Default::default()
        };
        s.ensure_visible(2.0);
        assert_eq!(s.scroll_x, 0.0);
        s.scroll_x = 30.0;
        s.user_panned = true;
        s.ensure_visible(2.0);
        assert_eq!(s.scroll_x, 30.0, "panned away: no snap back");
        s.ensure_visible(35.0);
        assert!(!s.user_panned, "playhead back in view resumes following");
        s.ensure_visible(60.0);
        assert_eq!(s.scroll_x, 58.0);
    }

    // ---- headless egui harness: real layout + hit-testing + gestures ----
    use crate::media::Backend;
    use crate::model::{Asset, AudioStreamInfo};
    use egui::{Event, Modifiers, PointerButton, RawInput};

    struct Harness {
        ctx: egui::Context,
        state: TimelineState,
        project: Project,
        selection: Vec<Id>,
        playhead: f64,
        undos: usize,
        waves: WaveformCache,
        time: f64,
    }

    impl Harness {
        fn new() -> Self {
            let ctx = egui::Context::default();
            let mut project = Project::new();
            let aid = project.add_asset(Asset {
                id: 0,
                path: "C:/x.mp4".into(),
                kind: ClipKind::Video,
                duration: 10.0,
                width: 1280,
                height: 720,
                fps: 30.0,
                audio_streams: vec![AudioStreamInfo { channels: 2, sample_rate: 48000, ..Default::default() }],
                codec: "h264".into(),
                folder: String::new(),
                tags: Vec::new(),
                label: 0,
                description: String::new(),
            });
            project.insert_asset_clips(aid, 0.0, None);
            let waves = WaveformCache::new(ctx.clone(), Backend::Ffmpeg);
            let mut h = Self {
                ctx,
                state: TimelineState::default(),
                project,
                selection: Vec::new(),
                playhead: 0.0,
                undos: 0,
                waves,
                time: 0.0,
            };
            h.frame(vec![]); // layout pass: sets lanes_rect
            h
        }
        fn frame(&mut self, events: Vec<Event>) -> TimelineResponse {
            self.frame_m(events, Modifiers::NONE)
        }
        fn frame_m(&mut self, events: Vec<Event>, mods: Modifiers) -> TimelineResponse {
            self.time += 0.05;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 400.0))),
                time: Some(self.time),
                modifiers: mods,
                events,
                ..Default::default()
            };
            let pal = Palette::new(true, Color32::from_rgb(0, 120, 212));
            let Harness { ctx, state, project, selection, playhead, undos, waves, .. } = self;
            let mut resp = None;
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut undo = |_: &Project| *undos += 1;
                    resp = Some(show(
                        ui,
                        state,
                        TimelineCtx {
                            project,
                            selection,
                            playhead,
                            undo: &mut undo,
                            waveforms: waves,
                            palette: &pal,
                            snap: false,
                            playing: false,
                            thumbs: None,
                            keep_ranges: &[],
                        },
                    ));
                });
            });
            resp.unwrap()
        }
        fn press(&mut self, pos: Pos2) -> TimelineResponse {
            self.press_m(pos, Modifiers::NONE)
        }
        fn press_m(&mut self, pos: Pos2, mods: Modifiers) -> TimelineResponse {
            self.frame_m(vec![Event::PointerMoved(pos)], mods);
            self.frame_m(
                vec![Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: mods }],
                mods,
            )
        }
        fn release(&mut self, pos: Pos2) -> TimelineResponse {
            self.release_m(pos, Modifiers::NONE)
        }
        fn release_m(&mut self, pos: Pos2, mods: Modifiers) -> TimelineResponse {
            self.frame_m(
                vec![Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers: mods }],
                mods,
            )
        }
        /// press at `from`, move in steps to `to`, release; returns the OR of `edited` over the gesture.
        fn drag(&mut self, from: Pos2, to: Pos2) -> bool {
            self.press(from);
            let mut edited = false;
            for i in 1..=4 {
                let p = from + (to - from) * (i as f32 / 4.0);
                edited |= self.frame(vec![Event::PointerMoved(p)]).edited;
            }
            edited |= self.release(to).edited;
            edited |= self.frame(vec![]).edited;
            edited
        }
        fn video_clip(&self) -> &Clip {
            &self.project.tracks[0].clips[0]
        }
        fn audio_clip(&self) -> &Clip {
            &self.project.tracks[1].clips[0]
        }
    }

    #[test]
    fn headless_click_selects_and_drag_moves_linked() {
        let mut h = Harness::new();
        let lanes = h.state.lanes_rect;
        assert!(lanes.width() > 300.0, "lanes rect not laid out: {lanes:?}");
        let vid = h.video_clip().id;
        let aud = h.audio_clip().id;
        // V1 is the top row (video tracks above audio): click inside the clip selects the link group
        let p = pos2(lanes.left() + 100.0, lanes.top() + 30.0);
        h.press(p);
        h.release(p);
        h.frame(vec![]);
        assert_eq!(h.selection, vec![vid, aud]);
        // click on empty lane → deselect
        let empty = pos2(lanes.left() + 600.0, lanes.top() + 30.0);
        h.press(empty);
        h.release(empty);
        h.frame(vec![]);
        assert!(h.selection.is_empty());
        // drag the clip right by 120 px = 3 s at zoom 40
        let edited = h.drag(p, p + vec2(120.0, 0.0));
        assert!(edited);
        assert_eq!(h.undos, 1);
        assert!((h.video_clip().start - 3.0).abs() < 0.05, "video start {}", h.video_clip().start);
        assert!((h.audio_clip().start - 3.0).abs() < 0.05, "audio (linked) start {}", h.audio_clip().start);
        assert!(h.state.drag.is_none());
        // add V2 (displayed above V1) and drag the clip up one row → it lands on V2, audio stays on A1
        h.project.add_track(TrackKind::Video);
        h.frame(vec![]);
        let v1_h = h.project.tracks[0].height;
        let v2_h = h.project.tracks[1].height;
        let from = pos2(h.state.x_at(3.0) + 50.0, lanes.top() + v2_h + v1_h * 0.5);
        assert!(h.drag(from, from - vec2(0.0, v2_h)));
        assert_eq!(h.project.tracks[1].clips.len(), 1, "clip should be on V2");
        assert!((h.project.tracks[1].clips[0].start - 3.0).abs() < 0.05);
        assert_eq!(h.project.tracks[2].clips.len(), 1, "audio stays on A1");
        assert_eq!(h.undos, 2);
    }

    #[test]
    fn headless_trim_end_and_ruler_scrub() {
        let mut h = Harness::new();
        let lanes = h.state.lanes_rect;
        let x_end = h.state.x_at(10.0);
        // drag the right edge of the video clip left by 80 px = 2 s
        let from = pos2(x_end - 2.0, lanes.top() + 30.0);
        let edited = h.drag(from, from - vec2(80.0, 0.0));
        assert!(edited);
        assert_eq!(h.undos, 1);
        assert!((h.video_clip().duration - 8.0).abs() < 0.05, "duration {}", h.video_clip().duration);
        assert!((h.audio_clip().duration - 8.0).abs() < 0.05, "linked audio duration {}", h.audio_clip().duration);
        // ruler press scrubs the playhead (frame-snapped)
        let rx = h.state.x_at(4.0) + 1.0;
        let r = h.press(pos2(rx, lanes.top() - RULER_H * 0.5));
        assert!(r.seeked);
        assert!((h.playhead - 4.0).abs() < 0.05, "playhead {}", h.playhead);
        h.release(pos2(rx, lanes.top() - RULER_H * 0.5));
    }

    #[test]
    fn headless_edge_handle_beats_playhead_after_split() {
        let mut h = Harness::new();
        let lanes = h.state.lanes_rect;
        h.playhead = 5.0;
        h.project.split_at(5.0, None);
        h.frame(vec![]);
        assert_eq!(h.project.tracks[0].clips.len(), 2);
        // press inside the right clip's left edge zone, which overlaps the playhead hit-rect, drag 1 s right
        let from = pos2(h.state.x_at(5.0) + 2.0, lanes.top() + 30.0);
        assert!(h.drag(from, from + vec2(40.0, 0.0)));
        assert_eq!(h.undos, 1);
        let right = &h.project.tracks[0].clips[1];
        assert!((right.start - 6.0).abs() < 0.05, "trimmed start {}", right.start);
        assert!((h.project.tracks[1].clips[1].start - 6.0).abs() < 0.05, "linked audio trims too");
        assert_eq!(h.playhead, 5.0, "playhead not scrubbed");
    }

    #[test]
    fn headless_linked_trim_is_all_or_nothing_and_no_dead_undo() {
        let mut h = Harness::new();
        let lanes = h.state.lanes_rect;
        // press-and-hold / blocked move (clip at 0 dragged left): nothing changes, no undo entry
        let p = pos2(lanes.left() + 100.0, lanes.top() + 30.0);
        h.press(p);
        h.frame(vec![]);
        h.release(p);
        assert!(!h.drag(p, p - vec2(80.0, 0.0)));
        assert_eq!(h.undos, 0);
        assert_eq!(h.video_clip().start, 0.0);
        // V1 2-10 linked with A1 2-10; unlinked audio on A1 at 0-1.5 blocks the audio's left edge
        h.project.tracks[0].clips[0].trim_start(2.0, f64::INFINITY);
        h.project.tracks[1].clips[0].trim_start(2.0, f64::INFINITY);
        h.project.tracks[1].clips.insert(0, Clip::new(99, ClipKind::Audio, "blk", 0.0, 1.5));
        h.frame(vec![]);
        let from = pos2(h.state.x_at(2.0) + 2.0, lanes.top() + 30.0);
        assert!(h.drag(from, from - vec2(80.0, 0.0))); // to t = 0 in 0.5 s steps
        assert_eq!(h.undos, 1);
        let (v, a) = (&h.project.tracks[0].clips[0], &h.project.tracks[1].clips[1]);
        assert!((v.start - 1.5).abs() < 0.05, "video start {}", v.start);
        assert_eq!(v.start, a.start, "linked clips keep identical extents");
        assert_eq!(v.end(), a.end());
    }

    #[test]
    fn headless_cross_track_move_uses_track_under_pointer() {
        let mut h = Harness::new();
        let lanes = h.state.lanes_rect;
        h.project.add_track(TrackKind::Video); // V2 (index 1), displayed above V1
        h.project.tracks[1].height = 200.0;
        h.frame(vec![]);
        // drag the V1 clip up: pointer ends 150 px into the tall V2 row -> lands on V2 (dy/row_h would overshoot)
        let from = pos2(h.state.x_at(0.0) + 50.0, lanes.top() + 200.0 + 30.0);
        assert!(h.drag(from, pos2(from.x, lanes.top() + 150.0)));
        assert_eq!(h.project.tracks[1].clips.len(), 1, "clip on V2");
        assert_eq!(h.project.tracks[0].clips.len(), 0);
        // drag it down 150 px: still inside V2 -> no change, no undo
        let from = pos2(from.x, lanes.top() + 10.0);
        assert!(!h.drag(from, pos2(from.x, lanes.top() + 160.0)));
        assert_eq!(h.project.tracks[1].clips.len(), 1, "still on V2");
        assert_eq!(h.undos, 1);
    }

    #[test]
    fn headless_dnd_drops_asset_and_ctrl_wheel_zooms() {
        let mut h = Harness::new();
        let lanes = h.state.lanes_rect;
        // dnd: press somewhere else, set the payload, hover the lanes past the clip, release → inserted there
        let aid = h.project.assets[0].id;
        h.press(pos2(10.0, 10.0));
        egui::DragAndDrop::set_payload(&h.ctx, DragPayload::Asset(aid));
        let drop = pos2(lanes.left() + 480.0, lanes.top() + 30.0); // t = 12 s, after the 10 s clip
        h.frame(vec![Event::PointerMoved(drop)]);
        let r = h.release(drop);
        assert!(r.edited);
        assert_eq!(h.undos, 1);
        assert_eq!(h.project.tracks[0].clips.len(), 2);
        let start = h.project.tracks[0].clips[1].start;
        assert!((start - 12.0).abs() < 0.05, "start {start}");
        assert_eq!(h.project.tracks[1].clips.len(), 2);
        // ctrl+wheel over the lanes zooms
        let over = pos2(lanes.left() + 200.0, lanes.top() + 30.0);
        h.frame(vec![Event::PointerMoved(over)]);
        for _ in 0..6 {
            h.frame(vec![Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, 40.0),
                modifiers: Modifiers::CTRL,
            }]);
        }
        assert!(h.state.zoom > 40.0, "zoom {}", h.state.zoom);
    }

    #[test]
    fn volume_db_mapping_roundtrip() {
        assert!((db_frac(0.0) - 0.7).abs() < 1e-6, "0 dB sits at 70 % height");
        assert_eq!(db_frac(DB_TOP), 1.0);
        assert_eq!(db_frac(DB_BOT), 0.0);
        for f in [0.0, 0.2, 0.5, 0.7, 0.9, 1.0] {
            assert!((db_frac(frac_db(f)) - f).abs() < 1e-4, "roundtrip at {f}");
        }
        assert!(gain_db(1.0).abs() < 1e-6);
        assert!((gain_db(2.0) - 6.02).abs() < 0.01);
        assert_eq!(gain_db(0.0), DB_BOT);
    }

    #[test]
    fn headless_volume_line_drag_changes_volume() {
        let mut h = Harness::new();
        let lanes = h.state.lanes_rect;
        // A1 row sits under V1 (height 64); audio clip rect is inset 1 pt; line at 70 % height (unity gain)
        let row_top = lanes.top() + h.project.tracks[0].height;
        let rect_h = h.project.tracks[1].height - 2.0;
        let line_y = (row_top + h.project.tracks[1].height - 1.0) - 0.7 * rect_h;
        let from = pos2(lanes.left() + 100.0, line_y);
        assert!(h.drag(from, from + vec2(0.0, 20.0)), "volume drag edits");
        assert_eq!(h.undos, 1);
        let v = &h.audio_clip().volume;
        assert!(!v.is_animated(), "constant volume stays constant");
        assert!(v.value > 0.0 && v.value < 0.5, "gain lowered, got {}", v.value);
        // clip untouched otherwise
        assert_eq!(h.audio_clip().start, 0.0);
        assert!((h.audio_clip().duration - 10.0).abs() < 1e-6);
    }

    #[test]
    fn headless_keyframe_drag_moves_key() {
        let mut h = Harness::new();
        let lanes = h.state.lanes_rect;
        h.project.tracks[0].clips[0].opacity.toggle_key(2.0);
        h.frame(vec![]);
        let row_bottom = lanes.top() + h.project.tracks[0].height;
        let kp = pos2(h.state.x_at(2.0), row_bottom - 1.0 - 5.0);
        assert!(h.drag(kp, kp + vec2(40.0, 0.0)), "keyframe drag edits");
        assert_eq!(h.undos, 1);
        let keys = &h.project.tracks[0].clips[0].opacity.keys;
        assert_eq!(keys.len(), 1);
        assert!((keys[0].t - 3.0).abs() < 1.0 / 30.0 + 1e-6, "key moved to {}", keys[0].t);
        assert_eq!(h.project.tracks[0].clips[0].start, 0.0, "clip not moved");
    }

    #[test]
    fn headless_transition_edge_drag_changes_duration() {
        use crate::model::TransitionKind;
        let mut h = Harness::new();
        let lanes = h.state.lanes_rect;
        h.project.split_at(5.0, None);
        let right_id = h.project.tracks[0].clips[1].id;
        h.project.add_transition(right_id, TransitionKind::CrossFade, 1.0).unwrap();
        h.frame(vec![]);
        // band spans 4.5..5.5 on V1; drag the right band edge +40 px (1 s at zoom 40)
        let from = pos2(h.state.x_at(5.5) - 2.0, lanes.top() + 30.0);
        assert!(h.drag(from, from + vec2(40.0, 0.0)), "transition drag edits");
        assert_eq!(h.undos, 1);
        let tr = h.project.tracks[0].transitions.iter().find(|t| t.right == right_id).unwrap();
        // pointer ends at t = 6.45 → half = 1.45 → duration 2.9
        assert!((tr.duration - 2.9).abs() < 0.06, "duration {}", tr.duration);
        // clips untouched
        assert!((h.project.tracks[0].clips[1].start - 5.0).abs() < 1e-6);
    }

    #[test]
    fn headless_scrollbar_thumb_drag_and_page() {
        let mut h = Harness::new();
        h.state.zoom = 200.0; // 10 s project, ~3 s visible → thumb smaller than the bar
        h.frame(vec![]);
        let lanes = h.state.lanes_rect;
        let bar_y = lanes.bottom() + HBAR_H * 0.5;
        // thumb starts at the far left: drag it right
        let from = pos2(lanes.left() + 30.0, bar_y);
        h.drag(from, from + vec2(100.0, 0.0));
        let vis_w = (lanes.width() / 200.0) as f64;
        let expect = (100.0 / lanes.width()) as f64 * 11.0; // total = duration * 1.1 = 11 s
        assert!((h.state.scroll_x - expect).abs() < expect * 0.3, "scroll_x {} vs {expect}", h.state.scroll_x);
        // click right of the thumb pages forward by one visible width
        let before = h.state.scroll_x;
        let pg = pos2(lanes.left() + lanes.width() - 20.0, bar_y);
        h.press(pg);
        h.release(pg);
        h.frame(vec![]);
        assert!(h.state.scroll_x > before + vis_w * 0.9, "paged from {before} to {}", h.state.scroll_x);
    }

    #[test]
    fn headless_alt_click_selects_single_clip() {
        let mut h = Harness::new();
        let lanes = h.state.lanes_rect;
        let (vid, aud) = (h.video_clip().id, h.audio_clip().id);
        let p = pos2(lanes.left() + 100.0, lanes.top() + 30.0);
        h.press(p);
        h.release(p);
        h.frame(vec![]);
        assert_eq!(h.selection, vec![vid, aud], "plain click selects the link group");
        h.press_m(p, Modifiers::ALT);
        h.release_m(p, Modifiers::ALT);
        h.frame(vec![]);
        assert_eq!(h.selection, vec![vid], "Alt+click selects only the clicked clip");
    }

    #[test]
    fn headless_video_track_v_toggle_flips_muted() {
        let mut h = Harness::new();
        let lanes = h.state.lanes_rect;
        // V1 header row: S button right-aligned (18 wide, 4 in), V button 21 pt left of it
        let vb = pos2(lanes.left() - 34.0, lanes.top() + h.project.tracks[0].height * 0.5);
        assert!(!h.project.tracks[0].muted);
        h.press(vb);
        let r = h.release(vb);
        assert!(r.edited || h.frame(vec![]).edited, "V toggle marks edited");
        assert!(h.project.tracks[0].muted, "V toggle flips muted (visibility off)");
        assert_eq!(h.undos, 1);
    }
}
