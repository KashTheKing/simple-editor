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
//!
//! Round 3: rubber-band select on empty lane space (Shift adds; dragging any selected clip moves the whole
//! selection as one undo step), drag past the top video / bottom audio row to drop onto a freshly created
//! track (gutter preview, Esc cancels the gesture), keyframe diamonds in a value lane on tall clips (y =
//! value, x = time, tooltip, one undo per gesture), project markers on the ruler and clip markers inside
//! clips (click selects, double-click seeks, drag moves, right-click = Rename / Delete / Set label),
//! clip colours from `Project.labels`, hatched Adjustment layers with an "adj" badge, and the Add Marker /
//! Copy & Paste Attributes / Add Mask / Nest / Convert to Adjustment Layer context-menu entries.

use crate::media::thumbs::ThumbCache;
use crate::media::waveform::{Peaks, WaveformCache};
use crate::model::{Asset, Clip, ClipKind, Ease, Id, Label, Project, TrackKind};
use crate::theme::Palette;
use crate::ui::tools::{draw_glyph, Glyph, Tool};

/// What a header toggle paints: a picture, or a plain character.
#[derive(Clone, Copy)]
enum Cap {
    Icon(Glyph),
    Text(&'static str),
}
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
/// Clip height from which keyframe diamonds move into a value lane (y = value) instead of the bottom strip.
const KEY_LANE_MIN: f32 = 28.0;
/// Inset of the value lane inside the clip rect (points), top and bottom.
const KEY_PAD: f32 = 3.0;
/// Insertion gutter shown when clips are dragged past the first/last row.
const GUTTER_H: f32 = 5.0;
/// Marker flag size on the ruler / inside clips.
const FLAG_W: f32 = 7.0;
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
    /// Rubber band in progress: (press origin, add to the selection instead of replacing it).
    band: Option<(Pos2, bool)>,
    /// Marker picked by the last click (drawn highlighted).
    pub selected_marker: Option<Id>,
    /// Rename buffer for the marker context menu.
    rename: Option<(Id, String)>,
    /// Track under the last press on the lanes — where Ctrl+V pastes.
    /// ponytail: an index, not an id, so removing a track just makes the next paste land on its neighbour.
    pub last_track: Option<usize>,
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
            band: None,
            selected_marker: None,
            rename: None,
            last_track: None,
        }
    }
}

/// Paste a copied group (relative times, the way `presets::capture_template` stores one) at `at`, with
/// fresh clip / link / marker ids. `Project::place_clips` picks the first track of each kind with room
/// (adding one when nothing is free); `target` then pulls the group onto the row the user last clicked,
/// if the whole group fits there.
pub fn paste_clips(p: &mut Project, clips: Vec<Clip>, assets: Vec<Asset>, at: f64, target: Option<usize>) -> Vec<Id> {
    let ids = p.place_clips(clips, assets, at);
    if let Some(ti) = target.filter(|&i| i < p.tracks.len()) {
        let kind = p.tracks[ti].kind;
        let list = if kind == TrackKind::Video { p.video_tracks() } else { p.audio_tracks() };
        let row = |t: usize| list.iter().position(|&x| x == t);
        let from = ids.iter().filter_map(|&id| p.track_of(id)).filter_map(row).min();
        if let (Some(to), Some(from)) = (row(ti), from) {
            p.move_clips(&ids, 0.0, to as i32 - from as i32, Some(kind));
        }
    }
    ids
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
    /// Scroll `t` into view unconditionally (an explicit seek: Home/End, prev/next cut, a subtitle
    /// click, an MCP `playback.seek`). Unlike `ensure_visible` a previous user pan does not suppress it.
    pub fn follow_playhead(&mut self, t: f64) {
        self.user_panned = false;
        self.ensure_visible(t);
    }
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
    /// Movie-mode pre-render coverage: merged `(from, to, ready)` runs, drawn as a bar along the top
    /// of the ruler. Empty = movie mode is off / nothing requested.
    pub prerender: &'a [(f64, f64, bool)],
    /// Active tool strip tool. Cut splits a clicked clip, Marker drops a marker, Stretch retimes an
    /// edge drag instead of trimming it; everything else behaves as Select.
    pub tool: crate::ui::tools::Tool,
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
    /// "Edit labels…" was picked in a clip's colour submenu — the app opens its label editor.
    pub edit_labels: bool,
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
    /// clip; `dt`/`dtrack` = applied so far. `new_track` = the pointer is past the first/last row of `kind`,
    /// so releasing here adds a track and drops the clips on it.
    Move { ids: Vec<Id>, orig: Vec<f64>, kind: TrackKind, tr: usize, dt: f64, dtrack: i32, new_track: bool },
    /// Trim the start (`start`) or end edge of `ids`; `edge` = edge time at drag start.
    Trim { ids: Vec<Id>, start: bool, edge: f64, changed: bool },
    /// Rate-stretch one clip by dragging an edge: the source window (`src_len` source seconds) is kept
    /// and the speed follows the new duration.
    Stretch { id: Id, start: bool, edge: f64, src_len: f64, changed: bool },
    /// Drag the volume line of an audio clip vertically.
    Volume { id: Id, changed: bool },
    /// Drag a fade handle horizontally (`out` = fade-out, else fade-in).
    Fade { id: Id, out: bool, changed: bool },
    /// Drag the keyframe diamond(s) at clip-local time `t`: horizontally = time (every property keyed there),
    /// vertically = the value of curve-editor property `prop` (value-lane mode only), mapped through the
    /// value `range` captured at press so the diamond does not chase the pointer as the range grows.
    Keys { id: Id, t: f64, prop: Option<usize>, range: (f64, f64), changed: bool },
    /// Drag a transition band edge (duration changes symmetrically around the cut).
    TransDur { track: usize, id: Id, changed: bool },
    /// Drag a marker: project marker (`clip` = None) or clip-local marker.
    Marker { id: Id, clip: Option<Id>, changed: bool },
    /// Spacer tool: shift every clip starting at or after the press time. `room` = how far left the
    /// group can go before it hits the clip in front of it (or 0).
    Spacer { ids: Vec<Id>, dt: f64, room: f64 },
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
    /// Add a project marker at this timeline time.
    AddMarker(f64),
    /// Razor tool: split every clip crossing this timeline time.
    SplitAt(f64),
    RenameMarker(Id, String),
    DelMarker(Id),
    MarkerLabel(Id, u8),
    /// Route the selected audio clips to a bus (0 = inherit the track's).
    Bus(Id),
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

/// Does the clip have any keyframe at all? Cheap (no allocation), unlike `Clip::key_times`, so the 1000-clip
/// case never touches the keyframe path.
fn has_keys(c: &Clip) -> bool {
    c.animated().iter().any(|a| a.is_animated())
        || c.effects.iter().any(|e| e.params.iter().any(|a| a.is_animated()) || e.mask.is_some())
        || c.mask.is_some()
        || c.graph.is_some()
        || c.shape.is_some()
}

/// Value range the diamonds of `prop` are mapped through: the live auto-range, or the one frozen at the
/// start of the drag so the dragged key does not push its own scale around.
fn key_range(state: &TimelineState, clip: Id, prop: usize, a: &crate::model::Animated) -> (f64, f64) {
    if let Some(Drag { g: Gesture::Keys { id, prop: Some(p), range, .. }, .. }) = &state.drag {
        if *id == clip && *p == prop {
            return *range;
        }
    }
    crate::ui::curves::y_range(a)
}

/// The range the rest of the UI enforces for property `i` (inspector sliders for the base props,
/// `ParamSpec` for effect params), so a value-lane drag cannot write past what a `DragValue` allows.
/// `None` where nothing enforces one: Position X/Y and Rotation are unbounded everywhere, and Volume is
/// a gain behind a nonlinear dB slider.
/// ponytail: same property order as curves.rs — move it next to `prop_ref` if a second caller shows up.
fn prop_range(c: &Clip, i: usize) -> Option<(f64, f64)> {
    let nb = if c.is_visual() { 6 } else { 3 };
    if i < nb {
        return match (c.is_visual(), i) {
            (true, 2) => Some((0.01, 20.0)),              // Scale
            (true, 4) => Some((0.0, 1.0)),                // Opacity
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
fn hatch(p: &egui::Painter, vis: Rect, color: Color32) {
    let (step, stroke, h) = (8.0f32, Stroke::new(1.0, color), vis.height());
    let mut x = (vis.left() / step).floor() * step - h;
    while x < vis.right() {
        p.line_segment([pos2(x, vis.bottom()), pos2(x + h, vis.top())], stroke);
        x += step;
    }
}

/// Marker flag: a stem down from `top` with a pennant to the right.
fn flag(p: &egui::Painter, x: f32, top: f32, bottom: f32, color: Color32, wide: bool) {
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
fn marker_hit(
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
fn wave_color(body: Color32, label: u8, pal: &Palette) -> Color32 {
    if label == 0 {
        return pal.waveform;
    }
    let target = if body.intensity() > 0.5 { Color32::BLACK } else { Color32::WHITE };
    body.lerp_to_gamma(target, 0.55)
}

fn label_color(p: &Project, idx: u8, fallback: Color32) -> Color32 {
    match p.label_color(idx) {
        Some([r, g, b]) => Color32::from_rgb(r, g, b),
        None => fallback,
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

/// Colour submenu: the project's own labels (name + colour), not the built-in defaults.
fn label_menu(ui: &mut egui::Ui, labels: &[Label], act: &mut Option<Act>, edit_labels: &mut bool) {
    if ui.button("None").clicked() {
        *act = Some(Act::Label(0));
    }
    for (i, l) in labels.iter().enumerate() {
        let color = Color32::from_rgb(l.color[0], l.color[1], l.color[2]);
        if ui.button(egui::RichText::new(l.name.clone()).color(color)).clicked() {
            *act = Some(Act::Label(i as u8 + 1));
        }
    }
    ui.separator();
    if ui.button("Apply label to asset").clicked() {
        *act = Some(Act::LabelToAsset);
    }
    if ui.button("Edit labels…").clicked() {
        *edit_labels = true;
    }
}

#[allow(clippy::too_many_arguments)]
fn clip_menu(
    ui: &mut egui::Ui,
    linked: bool,
    enabled: bool,
    audio: bool,
    labels: &[Label],
    buses: &[crate::model::Bus],
    act: &mut Option<Act>,
    actions: &mut Vec<crate::hotkeys::Action>,
    edit_labels: &mut bool,
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
    // ponytail: one entry — Action::AddTransition always targets the cut on the selected clip's left.
    // "at End" pushed the same action, so it either duplicated this or toasted "no left neighbour".
    // The Transitions pane covers the right-hand cut (transitions_ui::right_neighbor).
    if ui.button("Add Transition at Start").clicked() {
        actions.push(Action::AddTransition);
    }

    if ui.button("Add Transition at End").clicked() {
        actions.push(Action::AddTransitionEnd);
    }
    if ui.button("Auto-cut…").clicked() {
        actions.push(Action::AutoCut);
    }
    ui.separator();
    if ui.button("Add Marker").clicked() {
        actions.push(Action::AddMarker);
    }
    if ui.button("Copy Attributes").clicked() {
        actions.push(Action::CopyAttributes);
    }
    if ui.button("Paste Attributes…").clicked() {
        actions.push(Action::PasteAttributes);
    }
    // a mask means nothing on audio; that clip wants its mixer routing instead
    if audio {
        ui.menu_button("Bus", |ui| {
            if ui.button("(track)").clicked() {
                *act = Some(Act::Bus(0));
            }
            for b in buses {
                if ui.button(&b.name).clicked() {
                    *act = Some(Act::Bus(b.id));
                }
            }
        });
    } else if ui.button("Add Mask").clicked() {
        actions.push(Action::AddMask);
    }
    ui.separator();
    if ui.button("Nest into Sequence…").clicked() {
        actions.push(Action::NestSequence);
    }
    if ui.button("Convert to Adjustment Layer").clicked() {
        actions.push(Action::AddAdjustment);
    }
    if ui.button("Save as Template…").clicked() {
        actions.push(Action::SaveTemplate);
    }
    ui.menu_button("Color Label", |ui| label_menu(ui, labels, act, edit_labels));
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
    let (mods, pointer, primary_down, escape) =
        ui.input(|i| (i.modifiers, i.pointer.latest_pos(), i.pointer.primary_down(), i.key_pressed(egui::Key::Escape)));
    // Esc aborts the gesture: put the project back as it was at press time and drop the band.
    if escape {
        state.band = None;
        if let Some(d) = state.drag.take() {
            *c.project = d.before;
        }
    }

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

    // the row under the press is where Ctrl+V pastes
    if ui.input(|i| i.pointer.primary_pressed()) {
        if let Some(ti) = pointer.filter(|p| lanes.contains(*p)).and_then(|p| state.track_at(p.y, c.project)) {
            state.last_track = Some(ti);
        }
    }

    let mut act: Option<Act> = None;
    let mut click: Option<Id> = None;
    // spacer tool: a press on empty lane space opens a gap instead of rubber-banding
    let mut start_spacer = false;
    let mut start_move: Option<Id> = None;
    let mut start_trim: Option<(Id, bool)> = None;
    let mut start_vol: Option<Id> = None;
    let mut start_fade: Option<(Id, bool)> = None;
    let mut start_key: Option<(Id, f64, Option<usize>)> = None;
    let mut start_trans: Option<(usize, Id)> = None;
    let mut start_marker: Option<(Id, Option<Id>)> = None;
    let mut resize: Option<(usize, f32)> = None;
    let mut divider_y: Option<f32> = None;
    let mut key_hits: Vec<(Id, f64, Pos2, Option<usize>)> = Vec::new();
    let mut marker_hits: Vec<(Id, Id, Rect)> = Vec::new();
    let linked_sel = c.project.expand_links(c.selection);
    let thin = Stroke::new(1.0, pal.border);
    // rubber band: hit-tested while the rows are painted, applied on release
    let band_rect = state.band.map(|(o, _)| Rect::from_two_pos(o, pointer.unwrap_or(o)).intersect(lanes));
    let mut band_ids: Vec<Id> = Vec::new();
    // marker rename buffer, moved out of `state` so the menu closures can borrow it while `state` paints
    let mut rename = state.rename.take();
    let mut seek_marker: Option<f64> = None;
    let labels: &[Label] = &c.project.labels;
    let buses: &[crate::model::Bus] = &c.project.buses;

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
        // real-world icons: an eye for video visibility, a speaker for audio mute
        let (m_label, m_on) = if is_video {
            (Cap::Icon(if track.muted { Glyph::EyeOff } else { Glyph::Eye }), !track.muted)
        } else {
            (Cap::Icon(if track.muted { Glyph::SpeakerOff } else { Glyph::SpeakerOn }), track.muted)
        };
        if toggle_button(ui, &bp, mb, tid.with("m"), m_label, m_on, &pal, &small) {
            act = Some(Act::Mute(ti));
        }
        if toggle_button(ui, &bp, sb, tid.with("s"), Cap::Text("S"), track.solo, &pal, &small) {
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
            if let Some(br) = band_rect {
                if br.intersects(rect) {
                    band_ids.push(clip.id);
                }
            }
            // custom labels win over the per-kind fill (Shape / Adjustment have their own)
            let clip_label = c.project.clip_label(clip);
            let mut color = label_color(c.project, clip_label, pal.clip_color(clip.kind));
            if !clip.enabled || !active {
                color = color.gamma_multiply(0.5);
            }
            lp.rect_filled(rect, 0, color);
            lp.rect_stroke(rect, 0, thin, StrokeKind::Inside);
            // sub-pixel clips (zoomed way out): fill only — no text, waveform, diamonds or hit-testing.
            // ponytail: 4 pt is well under the ~18 pt a clip needs for its trim handles, and the rubber
            // band still catches them; give them their own interact if that ever bites.
            let detailed = vis.width() >= 4.0;
            if clip.kind == ClipKind::Adjustment && detailed {
                hatch(&lp.with_clip_rect(vis), vis, pal.text.gamma_multiply(0.25));
            }
            if !detailed {
                if c.selection.contains(&clip.id) {
                    lp.rect_stroke(rect, 0, Stroke::new(2.0, pal.selection), StrokeKind::Inside);
                }
                continue;
            }
            if clip.kind == ClipKind::Audio {
                if let Some(asset) = c.project.asset(clip.asset) {
                    if let Some(peaks) = c.waveforms.get(&asset.path, clip.audio_stream) {
                        draw_waveform(&lp, &peaks, clip, vis.shrink(1.0), state, wave_color(color, clip_label, &pal));
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
                let badge = Rect::from_min_size(name_pos, vec2(15.0, 15.0));
                draw_glyph(&name_pc, badge, Glyph::Sequence, pal.text);
                name_pc.text(pos2(badge.right(), name_pos.y), Align2::LEFT_TOP, &clip.name, font.clone(), pal.text);
            } else {
                name_pc.text(name_pos, Align2::LEFT_TOP, &clip.name, font.clone(), pal.text);
            }
            if clip.is_retimed() {
                if clip.freeze.is_some() {
                    // a snowflake, painted: the frozen clip has no rate to print
                    let f = Rect::from_min_size(pos2(rect.right() - 17.0, rect.top() + 1.0), vec2(16.0, 16.0));
                    draw_glyph(&name_pc, f, Glyph::Snowflake, pal.text);
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
                        s.push_str("rev");
                    }
                    name_pc.text(
                        pos2(rect.right() - 3.0, rect.top() + 2.0),
                        Align2::RIGHT_TOP,
                        s,
                        small.clone(),
                        pal.text,
                    );
                }
            }
            // retime preview: while the Stretch tool drags this clip's edge, show the rate it is landing on
            // big in the middle — the corner badge is easy to miss mid-drag
            if matches!(&state.drag, Some(Drag { g: Gesture::Stretch { id, .. }, .. }) if *id == clip.id) {
                name_pc.text(
                    vis.center(),
                    Align2::CENTER_CENTER,
                    format!("{:.2}x", clip.speed),
                    font.clone(),
                    pal.accent,
                );
            }
            if clip.kind == ClipKind::Adjustment {
                name_pc.text(
                    pos2(rect.right() - 3.0, rect.bottom() - 2.0),
                    Align2::RIGHT_BOTTOM,
                    "adj",
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
            // keyframe diamonds: on a tall clip in a value lane — 0 % at the bottom, 100 % at the top of the
            // range of the first property keyed at that time (the curve editor's auto-range, so both panes
            // agree) — otherwise in the bottom strip
            if has_keys(clip) {
                let lane = rect.height() >= KEY_LANE_MIN;
                let props = crate::ui::curves::prop_count(clip);
                for t in clip.key_times() {
                    let kx = state.x_at(clip.start + t);
                    if kx < lanes.left() - 6.0 || kx > lanes.right() + 6.0 {
                        continue;
                    }
                    // ponytail: the lane follows the curve editor's property list (transform + effect
                    // params). Mask / graph / shape keys keep the bottom strip — extend curves::prop_ref
                    // to them if they ever need a value lane too. Audio volume (prop 0) too: it is drawn
                    // and dragged on the dB scale above, which the linear lane would contradict.
                    let first = if clip.is_visual() { 0 } else { 1 };
                    let prop = lane
                        .then(|| {
                            (first..props)
                                .find(|&i| crate::ui::curves::prop_ref(clip, i).is_some_and(|a| a.has_key_at(t)))
                        })
                        .flatten();
                    let ky = match prop.and_then(|i| crate::ui::curves::prop_ref(clip, i).map(|a| (i, a))) {
                        Some((i, a)) => {
                            let (lo, hi) = key_range(state, clip.id, i, a);
                            let f = ((a.at(t) - lo) / (hi - lo).max(1e-9)).clamp(0.0, 1.0) as f32;
                            rect.bottom() - KEY_PAD - f * (rect.height() - 2.0 * KEY_PAD)
                        }
                        None => rect.bottom() - 5.0,
                    };
                    let kp = pos2(kx, ky);
                    lp.add(diamond(kp, 4.0, pal.keyframe));
                    key_hits.push((clip.id, t, kp, prop));
                }
            }
            // clip markers (clip-local time), drawn from the top of the clip down
            for m in &clip.markers {
                let mx = state.x_at(clip.start + m.t);
                if mx < vis.left() - FLAG_W || mx > vis.right() {
                    continue;
                }
                let sel = state.selected_marker == Some(m.id);
                flag(
                    &lp.with_clip_rect(vis),
                    mx,
                    rect.top() + 1.0,
                    rect.bottom(),
                    label_color(c.project, m.label, pal.text),
                    sel,
                );
                // interaction is registered after the clip bodies (below) so the small flags win hit-testing
                let mr = Rect::from_min_size(pos2(mx - 2.0, rect.top()), vec2(FLAG_W + 4.0, 10.0)).intersect(lanes);
                if mr.is_positive() {
                    marker_hits.push((clip.id, m.id, mr));
                }
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
                // razor / marker tools act where the pointer is instead of selecting
                match (c.tool, br.interact_pointer_pos()) {
                    (Tool::Cut, Some(pp)) => act = Some(Act::SplitAt(state.time_at(pp.x))),
                    (Tool::Marker, Some(pp)) => act = Some(Act::AddMarker(state.time_at(pp.x).max(0.0))),
                    _ => click = Some(clip.id),
                }
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
            let (linked, enabled, aud) = (clip.link != 0, clip.enabled, clip.kind == ClipKind::Audio);
            let mut rclick = br.secondary_clicked();
            br.context_menu(|ui| {
                clip_menu(ui, linked, enabled, aud, labels, buses, &mut act, &mut out.actions, &mut out.edit_labels)
            });
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
                    r.context_menu(|ui| {
                        clip_menu(
                            ui,
                            linked,
                            enabled,
                            aud,
                            labels,
                            buses,
                            &mut act,
                            &mut out.actions,
                            &mut out.edit_labels,
                        )
                    });
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
            let Some((left, right)) = track.transition_pair(tr) else { continue };
            // the played window is clamped to the two clips (an over-long transition can't reach past them)
            let (wa, wb) = tr.window(left, right);
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
    for (i, &(kcid, kt, kp, kprop)) in key_hits.iter().enumerate() {
        let cursor = if kprop.is_some() { CursorIcon::Move } else { CursorIcon::ResizeHorizontal };
        let r = ui
            .interact(Rect::from_center_size(kp, vec2(10.0, 10.0)), id.with(("key", i)), Sense::click_and_drag())
            .on_hover_cursor(cursor);
        if r.drag_started_by(egui::PointerButton::Primary) {
            start_key = Some((kcid, kt, kprop));
        }
        let clip = c.project.clip(kcid);
        let r = r.on_hover_ui(|ui| {
            // built only while hovered — a per-frame format! per diamond is not worth it
            match (clip, kprop) {
                (Some(cl), Some(pi)) => {
                    let v = crate::ui::curves::prop_ref(cl, pi).map(|a| a.at(kt)).unwrap_or(0.0);
                    ui.label(format!("{} {:.3}\n{:.2} s", crate::ui::curves::prop_label(cl, pi), v, cl.start + kt));
                }
                (Some(cl), None) => {
                    ui.label(format!("{:.2} s", cl.start + kt));
                }
                _ => {}
            }
        });
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

    // clip markers, on top of the clip bodies
    for &(cid, mid, mr) in &marker_hits {
        let Some((cl, m)) = c.project.clip(cid).and_then(|cl| Some((cl, cl.markers.iter().find(|m| m.id == mid)?)))
        else {
            continue;
        };
        let r = ui.interact(mr, id.with(("cmk", mid)), Sense::click_and_drag());
        marker_hit(
            r,
            m,
            Some(cid),
            cl.start,
            &mut state.selected_marker,
            &mut start_marker,
            &mut act,
            &mut rename,
            &mut seek_marker,
            labels,
        );
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

    // ---- rubber band ----
    if let (Some(br), Some((_, add))) = (band_rect, state.band) {
        lp.rect_filled(br, 0, pal.accent.gamma_multiply(0.15));
        lp.rect_stroke(br, 0, Stroke::new(1.0, pal.accent), StrokeKind::Inside);
        if !primary_down {
            if !add {
                c.selection.clear();
            }
            for cid in band_ids.drain(..) {
                if !c.selection.contains(&cid) {
                    c.selection.push(cid);
                }
            }
            state.band = None;
        }
    }
    // ---- insertion gutter: dragging clips past the first/last row of their kind ----
    if let Some(Drag { g: Gesture::Move { kind, new_track: true, .. }, .. }) = &state.drag {
        let g = if *kind == TrackKind::Video {
            Rect::from_min_max(lanes.min, pos2(lanes.right(), lanes.top() + GUTTER_H))
        } else {
            Rect::from_min_max(pos2(lanes.left(), lanes.bottom() - GUTTER_H), lanes.max)
        };
        lp.rect_filled(g, 0, pal.accent);
    }

    // ---- pre-render bar: a thin strip along the very top of the ruler ----
    for &(a, b, ready) in c.prerender {
        let (xa, xb) = (state.x_at(a), state.x_at(b));
        let seg = Rect::from_min_max(pos2(xa.max(ruler.left()), ruler.top()), pos2(xb, ruler.top() + 3.0));
        if seg.is_positive() {
            let col = if ready { pal.selection } else { pal.text_dim };
            rp.rect_filled(seg, 0, col);
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

    // ---- project markers on the ruler ----
    for m in &c.project.markers {
        let mx = state.x_at(m.t);
        if mx < ruler.left() - FLAG_W || mx > ruler.right() {
            continue;
        }
        let sel = state.selected_marker == Some(m.id);
        flag(&rp, mx, ruler.top() + 1.0, ruler.bottom(), label_color(c.project, m.label, pal.accent), sel);
        let mr = Rect::from_min_size(pos2(mx - 2.0, ruler.top()), vec2(FLAG_W + 4.0, RULER_H)).intersect(ruler);
        if mr.is_positive() {
            let r = ui.interact(mr, id.with(("mk", m.id)), Sense::click_and_drag());
            marker_hit(
                r,
                m,
                None,
                0.0,
                &mut state.selected_marker,
                &mut start_marker,
                &mut act,
                &mut rename,
                &mut seek_marker,
                labels,
            );
        }
    }
    let ph_now = *c.playhead;
    ruler_resp.context_menu(|ui| {
        if ui.button("Add Marker at Playhead").clicked() {
            act = Some(Act::AddMarker(ph_now));
        }
    });

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

    // ---- lanes background: deselect / rubber band / context menu / dnd ----
    if lanes_resp.clicked() && !mods.ctrl {
        // the marker tool also works on empty lanes; everything else just deselects
        match (c.tool, lanes_resp.interact_pointer_pos()) {
            (Tool::Marker, Some(pp)) => act = Some(Act::AddMarker(state.time_at(pp.x).max(0.0))),
            _ => c.selection.clear(),
        }
    }
    // a press that missed every clip starts a rubber band (Shift adds to the selection)
    if state.drag.is_none() && lanes_resp.drag_started_by(egui::PointerButton::Primary) {
        if c.tool == Tool::Spacer {
            start_spacer = true;
        } else if let Some(o) = lanes_resp.interact_pointer_pos().or(pointer) {
            state.band = Some((o, mods.shift));
        }
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
                DragPayload::Path(_) | DragPayload::Template(_) | DragPayload::Effect(_) => None,
                DragPayload::Transition(_) => None,
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
            Act::SplitAt(t) => {
                p.split_at(t, None);
            }
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
            Act::AddMarker(t) => {
                let mid = p.add_marker(t, "Marker");
                state.selected_marker = Some(mid);
            }
            Act::RenameMarker(mid, name) => {
                if let Some(m) = p.marker_mut(mid) {
                    m.name = name;
                }
            }
            Act::DelMarker(mid) => {
                p.remove_marker(mid);
                state.selected_marker = None;
            }
            Act::MarkerLabel(mid, l) => {
                if let Some(m) = p.marker_mut(mid) {
                    m.label = l;
                }
            }
            Act::Bus(b) => {
                for id in &ids {
                    if let Some(cl) = p.clip_mut(*id).filter(|cl| cl.kind == ClipKind::Audio) {
                        cl.bus = b;
                    }
                }
            }
        }
        out.edited = true;
    }
    state.rename = rename;
    if let Some(t) = seek_marker {
        *c.playhead = c.project.snap_frame(t.max(0.0));
        out.seeked = true;
    }

    // ---- gestures ----
    let origin = ui.input(|i| i.pointer.press_origin()).or(pointer).unwrap_or(Pos2::ZERO);
    // with the spacer tool every lane press is a gap gesture, clip bodies and edges included
    let spacer = c.tool == Tool::Spacer && (start_spacer || start_move.is_some() || start_trim.is_some());
    if let Some(cid) = start_move.or(start_trim.map(|(id, _)| id)).filter(|_| !spacer) {
        if !c.selection.contains(&cid) {
            if !mods.ctrl {
                c.selection.clear();
            }
            c.selection.push(cid);
        }
    }
    if spacer {
        let t0 = state.time_at(origin.x).max(0.0);
        let ids: Vec<Id> = c.project.all_clips().filter(|(_, cl)| cl.start >= t0).map(|(_, cl)| cl.id).collect();
        // how far left the whole group can go: the tightest gap in front of it, per track
        let room = c
            .project
            .tracks
            .iter()
            .filter_map(|t| {
                let first = t.clips.iter().map(|cl| cl.start).filter(|&s| s >= t0).fold(f64::INFINITY, f64::min);
                let prev = t.clips.iter().filter(|cl| cl.start < t0).map(|cl| cl.end()).fold(0.0, f64::max);
                first.is_finite().then_some(first - prev)
            })
            .fold(f64::INFINITY, f64::min);
        let room = if room.is_finite() { room.max(0.0) } else { 0.0 };
        state.drag = Some(Drag { origin, before: c.project.clone(), g: Gesture::Spacer { ids, dt: 0.0, room } });
    } else if let Some(cid) = start_move {
        if let Some(tr) = c.project.track_of(cid) {
            let ids = c.project.expand_links(c.selection);
            let orig = ids.iter().map(|&id| c.project.clip(id).map(|cl| cl.start).unwrap_or(0.0)).collect();
            let kind = c.project.tracks[tr].kind;
            let g = Gesture::Move { ids, orig, kind, tr, dt: 0.0, dtrack: 0, new_track: false };
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
            let g = if c.tool == Tool::Stretch {
                Gesture::Stretch { id: cid, start, edge, src_len: clip.src_len(), changed: false }
            } else {
                Gesture::Trim { ids, start, edge, changed: false }
            };
            state.drag = Some(Drag { origin, before: c.project.clone(), g });
        }
    } else if let Some(cid) = start_vol {
        state.drag = Some(Drag { origin, before: c.project.clone(), g: Gesture::Volume { id: cid, changed: false } });
    } else if let Some((cid, fout)) = start_fade {
        state.drag =
            Some(Drag { origin, before: c.project.clone(), g: Gesture::Fade { id: cid, out: fout, changed: false } });
    } else if let Some((cid, kt, prop)) = start_key {
        // freeze the value scale at press time: dragging a key must not move the scale under itself
        let range = prop
            .and_then(|pi| c.project.clip(cid).and_then(|cl| crate::ui::curves::prop_ref(cl, pi)))
            .map(crate::ui::curves::y_range)
            .unwrap_or((0.0, 1.0));
        state.drag = Some(Drag {
            origin,
            before: c.project.clone(),
            g: Gesture::Keys { id: cid, t: kt, prop, range, changed: false },
        });
    } else if let Some((tri, tid)) = start_trans {
        state.drag = Some(Drag {
            origin,
            before: c.project.clone(),
            g: Gesture::TransDur { track: tri, id: tid, changed: false },
        });
    } else if let Some((mid, mclip)) = start_marker {
        state.drag = Some(Drag {
            origin,
            before: c.project.clone(),
            g: Gesture::Marker { id: mid, clip: mclip, changed: false },
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
        let ox = drag.origin.x; // grab point (auto-scroll compensates it), for gestures that edit at one time
        let thr = (SNAP_PX / zoom) as f64;
        let p = &mut *c.project;
        match &mut drag.g {
            Gesture::Move { ids, orig, kind, tr, dt, dtrack, new_track } => {
                // past the first video row (up) or the last audio row (down): offer a fresh track.
                // Armed off the painted gutter bands, not the first/last row — those are scrolled away
                // once the lanes scroll, which used to make the gesture unreachable.
                *new_track = match kind {
                    TrackKind::Video => pos.y < lanes.top() + GUTTER_H,
                    TrackKind::Audio => pos.y > lanes.bottom() - GUTTER_H,
                };
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
            Gesture::Stretch { id, start, edge, src_len, changed } => {
                let mut want = *edge + dx;
                if c.snap {
                    want = p.snap_frame(want);
                }
                if let Some((ti, ci)) = p.find(*id) {
                    let cl = &p.tracks[ti].clips[ci];
                    let (new_start, dur) = if *start {
                        let w = want.clamp(0.0, cl.end() - crate::model::MIN_CLIP);
                        (w, cl.end() - w)
                    } else {
                        (cl.start, want - cl.start)
                    };
                    let dur = dur.max(crate::model::MIN_CLIP);
                    if p.tracks[ti].fits(new_start, dur, &[*id]) {
                        let cl = &mut p.tracks[ti].clips[ci];
                        if (cl.duration - dur).abs() > 1e-9 || (cl.start - new_start).abs() > 1e-9 {
                            // speed = source seconds per timeline second: the window is untouched
                            cl.set_speed(*src_len / dur);
                            cl.start = new_start;
                            *changed = true;
                        }
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
                            // latch to the grab time: editing at the live x inserts a key per frame of the drag
                            let lt = (t_at(ox) - cl.start).clamp(0.0, cl.duration);
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
            Gesture::Keys { id, t, prop, range, changed } => {
                let nt = p.snap_frame(t_at(pos.x));
                if let Some(cl) = p.clip_mut(*id) {
                    let lt = (nt - cl.start).clamp(0.0, cl.duration);
                    if (lt - *t).abs() > 1e-9 {
                        cl.move_keys(*t, lt);
                        *t = lt;
                        *changed = true;
                    }
                }
                // value lane: y inside the clip rect → value, through the range frozen at press time
                if let (Some(pi), Some((ti, ci))) = (*prop, p.find(*id)) {
                    if let Some(top) = row_top_of(p, ti) {
                        let h = p.tracks[ti].height;
                        let inner = (h - 2.0 - 2.0 * KEY_PAD).max(1.0);
                        let f = (((top + h - 1.0 - KEY_PAD) - pos.y) / inner).clamp(0.0, 1.0) as f64;
                        let v = range.0 + f * (range.1 - range.0);
                        // the lane auto-scales past the property's own bounds (y_range pads by 10 %) —
                        // clamp the written value the way every DragValue for it does
                        let v = prop_range(&p.tracks[ti].clips[ci], pi).map_or(v, |(lo, hi)| v.clamp(lo, hi));
                        if let Some(a) = crate::ui::curves::prop_mut(&mut p.tracks[ti].clips[ci], pi) {
                            if let Some(ki) = a.key_index_at(*t) {
                                if (a.keys[ki].v - v).abs() > 1e-9 {
                                    a.keys[ki].v = v;
                                    *changed = true;
                                }
                            }
                        }
                    }
                }
            }
            Gesture::TransDur { track, id, changed } => {
                // the project can be replaced mid-drag (undo, MCP project.open/sequence.open): index may be stale
                if let Some(tr) = p.tracks.get_mut(*track) {
                    let lim = tr
                        .transitions
                        .iter()
                        .find(|t| t.id == *id)
                        .and_then(|t| tr.transition_pair(t))
                        .map(|(l, r)| (r.start, 2.0 * l.duration.min(r.duration)));
                    if let Some((cut, max)) = lim {
                        // cap: the Transitions panel's 0.1..5 s range, and what the two clips can actually supply
                        let d = ((t_at(pos.x) - cut).abs() * 2.0).min(max).min(5.0).max(0.1);
                        if let Some(t) = tr.transitions.iter_mut().find(|t| t.id == *id) {
                            if (t.duration - d).abs() > 1e-9 {
                                t.duration = d;
                                *changed = true;
                            }
                        }
                    }
                }
            }
            Gesture::Marker { id, clip, changed } => {
                let want = p.snap_frame(t_at(pos.x).max(0.0));
                // clip markers are clip-local and stay inside their clip
                let nt = match clip.and_then(|cid| p.clip(cid)) {
                    Some(cl) => (want - cl.start).clamp(0.0, cl.duration),
                    None => want,
                };
                if let Some(m) = p.marker_mut(*id) {
                    if (m.t - nt).abs() > 1e-9 {
                        m.t = nt;
                        *changed = true;
                    }
                }
                if clip.is_none() && *changed {
                    p.sort_markers();
                }
            }
            Gesture::Spacer { ids, dt, room } => {
                // move_clips is all-or-nothing, so the clip in front of the group is never overrun
                let want = if c.snap { p.snap_frame(dx) } else { dx }.max(-*room);
                if (want - *dt).abs() > 1e-9 && p.move_clips(ids, want - *dt, 0, None) {
                    *dt = want;
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
            let mut edited = match &d.g {
                Gesture::Move { dt, dtrack, .. } => *dt != 0.0 || *dtrack != 0,
                Gesture::Spacer { dt, .. } => *dt != 0.0,
                Gesture::Trim { changed, .. }
                | Gesture::Stretch { changed, .. }
                | Gesture::Volume { changed, .. }
                | Gesture::Fade { changed, .. }
                | Gesture::Keys { changed, .. }
                | Gesture::TransDur { changed, .. }
                | Gesture::Marker { changed, .. } => *changed,
            };
            // released over the gutter: add a track at the far end and drop the clips of that kind on it
            if let Gesture::Move { ids, kind, new_track: true, .. } = &d.g {
                let p = &mut *c.project;
                let ti = p.add_track(*kind);
                let list = if *kind == TrackKind::Video { p.video_tracks() } else { p.audio_tracks() };
                let pos_of = |t: usize| list.iter().position(|&x| x == t).map(|i| i as i32);
                let from =
                    ids.iter().filter_map(|&cid| p.track_of(cid)).find(|&t| p.tracks[t].kind == *kind).and_then(pos_of);
                match from.zip(pos_of(ti)).filter(|(f, to)| f != to) {
                    Some((f, to)) if p.move_clips(ids, 0.0, to - f, Some(*kind)) => edited = true,
                    _ => p.remove_track(ti),
                }
            }
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
    use crate::model::{Asset, AudioStreamInfo, Effect, EffectKind};
    use egui::{Event, Modifiers, PointerButton, RawInput};

    struct Harness {
        ctx: egui::Context,
        state: TimelineState,
        project: Project,
        selection: Vec<Id>,
        playhead: f64,
        undos: usize,
        waves: WaveformCache,
        tool: Tool,
        time: f64,
        /// Paint list of the last frame (asserting on what was actually drawn).
        shapes: Vec<egui::epaint::ClippedShape>,
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
                tool: Tool::Select,
                time: 0.0,
                shapes: Vec::new(),
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
            let Harness { ctx, state, project, selection, playhead, undos, waves, tool, .. } = self;
            let mut resp = None;
            let full = ctx.run(input, |ctx| {
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
                            prerender: &[],
                            tool: *tool,
                        },
                    ));
                });
            });
            self.shapes = full.shapes;
            resp.unwrap()
        }
        /// Every text painted last frame, with its top-left position.
        fn texts(&self) -> Vec<(String, Pos2)> {
            self.shapes
                .iter()
                .filter_map(|cs| match &cs.shape {
                    Shape::Text(t) => Some((t.galley.text().to_string(), t.pos)),
                    _ => None,
                })
                .collect()
        }
        fn painted_text(&self, needle: &str) -> Option<Pos2> {
            self.texts().into_iter().find(|(s, _)| s.contains(needle)).map(|(_, p)| p)
        }
        /// Is any filled rect painted in this colour?
        fn has_fill(&self, color: Color32) -> bool {
            self.shapes.iter().any(|cs| matches!(&cs.shape, Shape::Rect(r) if r.fill == color))
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

    /// The tool strip drives the timeline: razor splits where you click, the marker tool drops a
    /// marker there, and stretch retimes an edge drag instead of trimming it.
    #[test]
    fn tools_cut_mark_and_stretch() {
        let mut h = Harness::new();
        let lanes = h.state.lanes_rect;
        let p = pos2(lanes.left() + 100.0, lanes.top() + 30.0);

        h.tool = Tool::Cut;
        let before = h.project.tracks[0].clips.len();
        h.press(p);
        h.release(p);
        h.frame(vec![]);
        assert_eq!(h.project.tracks[0].clips.len(), before + 1, "razor click must split the clip");

        h.tool = Tool::Marker;
        assert!(h.project.markers.is_empty());
        let p2 = pos2(lanes.left() + 220.0, lanes.top() + 30.0);
        h.press(p2);
        h.release(p2);
        h.frame(vec![]);
        assert_eq!(h.project.markers.len(), 1, "marker tool click must drop a marker");

        // fresh timeline: the razor above left a clip butting up against the edge we want to stretch
        let mut h = Harness::new();
        let lanes = h.state.lanes_rect;
        h.tool = Tool::Stretch;
        let id = h.video_clip().id;
        let (dur0, src0) = (h.video_clip().duration, h.video_clip().src_len());
        let edge = lanes.left() + (h.video_clip().end() as f32) * h.state.zoom;
        assert!(h.drag(pos2(edge, lanes.top() + 30.0), pos2(edge + 80.0, lanes.top() + 30.0)));
        let c = h.project.clip(id).unwrap();
        assert!(c.duration > dur0 + 0.5, "stretch must lengthen the clip: {} -> {}", dur0, c.duration);
        assert!((c.src_len() - src0).abs() < 1e-6, "stretch must keep the source window: {} -> {}", src0, c.src_len());
        assert!(c.speed < 1.0, "a longer clip over the same source must slow down: {}", c.speed);
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
    fn headless_volume_line_drag_keys_once_on_animated_clip() {
        let mut h = Harness::new();
        let lanes = h.state.lanes_rect;
        // keyframed volume, unity at both ends → the line still sits at 70 % height
        h.project.tracks[1].clips[0].volume.toggle_key(0.0);
        h.project.tracks[1].clips[0].volume.toggle_key(9.0);
        h.frame(vec![]);
        let row_top = lanes.top() + h.project.tracks[0].height;
        let rect_h = h.project.tracks[1].height - 2.0;
        let line_y = (row_top + h.project.tracks[1].height - 1.0) - 0.7 * rect_h;
        let from = pos2(lanes.left() + 40.0, line_y); // t = 1 s at zoom 40
        assert!(h.drag(from, from + vec2(160.0, 12.0)), "volume drag edits");
        let v = &h.audio_clip().volume;
        // one key at the grab time, edited in place for the rest of the gesture — not one per frame
        assert_eq!(v.keys.len(), 3, "keys {:?}", v.keys);
        assert!((v.keys[1].t - 1.0).abs() < 1e-6, "key at the grab time, got {:?}", v.keys);
        assert!(v.keys[1].v < 1.0, "grabbed key lowered, got {:?}", v.keys);
    }

    #[test]
    fn headless_keyframe_drag_moves_key() {
        let mut h = Harness::new();
        let lanes = h.state.lanes_rect;
        h.project.tracks[0].height = MIN_TRACK_H; // < KEY_LANE_MIN → bottom strip, time only
        h.project.tracks[0].clips[0].opacity.toggle_key(2.0);
        h.frame(vec![]);
        let row_bottom = lanes.top() + h.project.tracks[0].height;
        let kp = pos2(h.state.x_at(2.0), row_bottom - 1.0 - 5.0);
        assert!(h.drag(kp, kp + vec2(40.0, 20.0)), "keyframe drag edits");
        assert_eq!(h.undos, 1);
        let keys = &h.project.tracks[0].clips[0].opacity.keys;
        assert_eq!(keys.len(), 1);
        assert!((keys[0].t - 3.0).abs() < 1.0 / 30.0 + 1e-6, "key moved to {}", keys[0].t);
        assert_eq!(keys[0].v, 1.0, "short clip: vertical drag does not touch the value");
        assert_eq!(h.project.tracks[0].clips[0].start, 0.0, "clip not moved");
    }

    #[test]
    fn headless_keyframe_value_lane_drag_changes_time_and_value() {
        let mut h = Harness::new();
        let lanes = h.state.lanes_rect;
        h.project.tracks[0].clips[0].opacity.toggle_key(2.0);
        h.frame(vec![]);
        // 64 pt row → value lane. One key at 1.0 auto-ranges to 0.5..1.5, so it sits mid-lane.
        let th = h.project.tracks[0].height;
        let inner = th - 2.0 - 2.0 * KEY_PAD;
        let kp = pos2(h.state.x_at(2.0), lanes.top() + th - 1.0 - KEY_PAD - 0.5 * inner);
        assert!(h.drag(kp, kp + vec2(40.0, 0.25 * inner)), "value-lane drag edits");
        assert_eq!(h.undos, 1, "one undo for the whole gesture");
        let keys = &h.project.tracks[0].clips[0].opacity.keys;
        assert_eq!(keys.len(), 1);
        assert!((keys[0].t - 3.0).abs() < 1.0 / 30.0 + 1e-6, "time moved to {}", keys[0].t);
        assert!((keys[0].v - 0.75).abs() < 0.05, "value dragged down to {}", keys[0].v);
        assert_eq!(h.project.tracks[0].clips[0].start, 0.0, "clip not moved");
    }

    #[test]
    fn headless_rubber_band_selects_and_moves_together() {
        let mut h = Harness::new();
        let lanes = h.state.lanes_rect;
        h.project.tracks[1].clips.clear(); // drop the linked audio: two video clips only
        h.project.split_at(5.0, None);
        h.frame(vec![]);
        let (a, b) = (h.project.tracks[0].clips[0].id, h.project.tracks[0].clips[1].id);
        // press on empty lane space below the tracks, drag up across both clips
        let from = pos2(h.state.x_at(9.0), lanes.top() + 200.0);
        h.drag(from, pos2(h.state.x_at(4.0), lanes.top() + 5.0));
        assert_eq!(h.selection, vec![a, b], "band selected both clips");
        assert!(h.state.band.is_none(), "band cleared on release");
        // dragging one of them moves the whole selection, as one undo step
        let grab = pos2(h.state.x_at(1.0), lanes.top() + 30.0);
        assert!(h.drag(grab, grab + vec2(40.0, 0.0)));
        assert_eq!(h.undos, 1);
        assert!((h.project.tracks[0].clips[0].start - 1.0).abs() < 0.05);
        assert!((h.project.tracks[0].clips[1].start - 6.0).abs() < 0.05);
    }

    #[test]
    fn headless_rubber_band_shift_adds_and_esc_cancels() {
        let mut h = Harness::new();
        let lanes = h.state.lanes_rect;
        h.project.tracks[1].clips.clear();
        h.project.split_at(5.0, None);
        h.frame(vec![]);
        let (a, b) = (h.project.tracks[0].clips[0].id, h.project.tracks[0].clips[1].id);
        let empty_y = lanes.top() + 200.0;
        // band over the left clip only
        h.drag(pos2(h.state.x_at(0.5), empty_y), pos2(h.state.x_at(2.0), lanes.top() + 5.0));
        assert_eq!(h.selection, vec![a]);
        // Shift-band over the right clip adds to it
        let (from, to) = (pos2(h.state.x_at(7.0), empty_y), pos2(h.state.x_at(9.0), lanes.top() + 5.0));
        h.press_m(from, Modifiers::SHIFT);
        for i in 1..=4 {
            let p = from + (to - from) * (i as f32 / 4.0);
            h.frame_m(vec![Event::PointerMoved(p)], Modifiers::SHIFT);
        }
        h.release_m(to, Modifiers::SHIFT);
        h.frame(vec![]);
        assert_eq!(h.selection, vec![a, b], "Shift added to the selection");
        // Esc during a clip move puts everything back and leaves no undo entry
        let grab = pos2(h.state.x_at(1.0), lanes.top() + 30.0);
        h.press(grab);
        h.frame(vec![Event::PointerMoved(grab + vec2(80.0, 0.0))]);
        assert!(h.project.tracks[0].clips[0].start > 0.5, "moved mid-drag");
        h.frame(vec![Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
        h.release(grab + vec2(80.0, 0.0));
        h.frame(vec![]);
        assert_eq!(h.project.tracks[0].clips[0].start, 0.0, "Esc restored the pre-drag project");
        assert_eq!(h.project.tracks[0].clips.len(), 2);
        assert_eq!(h.undos, 0, "cancelled gesture pushes no undo");
    }

    #[test]
    fn headless_drag_past_top_row_creates_a_video_track() {
        let mut h = Harness::new();
        let lanes = h.state.lanes_rect;
        let vid = h.video_clip().id;
        let from = pos2(h.state.x_at(2.0), lanes.top() + 30.0);
        // drag up into the ruler: the gutter appears, release drops the clip on a fresh V2
        h.press(from);
        for y in [lanes.top() + 10.0, lanes.top() - 5.0, lanes.top() - 12.0] {
            h.frame(vec![Event::PointerMoved(pos2(from.x, y))]);
        }
        assert!(matches!(h.state.drag, Some(Drag { g: Gesture::Move { new_track: true, .. }, .. })), "gutter armed");
        h.release(pos2(from.x, lanes.top() - 12.0));
        h.frame(vec![]);
        assert_eq!(h.project.video_tracks().len(), 2, "a video track was created");
        assert_eq!(h.project.tracks[1].clips.len(), 1, "clip on the new top track");
        assert_eq!(h.project.tracks[1].clips[0].id, vid);
        assert!(h.project.tracks[0].clips.is_empty(), "V1 is empty now");
        assert_eq!(h.project.tracks[2].clips.len(), 1, "linked audio stayed on A1");
        assert_eq!(h.undos, 1);
        // and the same past the bottom for audio: A2 appears below A1
        let a1_top = lanes.top() + h.project.tracks[1].height + h.project.tracks[0].height;
        let from = pos2(h.state.x_at(2.0), a1_top + 30.0);
        let to = pos2(from.x, lanes.bottom() - 2.0);
        h.press(from);
        for y in [a1_top + 80.0, lanes.bottom() - 40.0, to.y] {
            h.frame(vec![Event::PointerMoved(pos2(from.x, y))]);
        }
        h.release(to);
        h.frame(vec![]);
        assert_eq!(h.project.audio_tracks().len(), 2, "an audio track was created");
        assert_eq!(h.project.tracks[3].clips.len(), 1, "audio clip moved to A2");
        assert!(h.project.tracks[2].clips.is_empty(), "A1 is empty now");
        assert_eq!(h.undos, 2);
    }

    #[test]
    fn headless_track_gutters_arm_while_the_lanes_scroll() {
        let mut h = Harness::new();
        let lanes = h.state.lanes_rect;
        let (vid, aud) = (h.video_clip().id, h.audio_clip().id);
        for _ in 0..5 {
            h.project.add_track(TrackKind::Video);
        }
        h.state.scroll_y = 80.0; // past RULER_H, where the old row-relative arm zones were empty
        h.frame(vec![]);
        assert!(h.state.scroll_y > RULER_H, "lanes did not scroll: {}", h.state.scroll_y);
        let armed = |h: &Harness| matches!(h.state.drag, Some(Drag { g: Gesture::Move { new_track: true, .. }, .. }));
        let clip_at = |h: &Harness, id| {
            let ti = h.project.track_of(id).unwrap();
            pos2(h.state.x_at(2.0), row_top(&h.state, &h.project, ti).unwrap() + 30.0)
        };
        // audio first (the bottom gutter): drag A1's clip onto the bottom edge of the lanes
        let (from, to) = (clip_at(&h, aud), pos2(h.state.x_at(2.0), lanes.bottom() - 2.0));
        h.press(from);
        h.frame(vec![Event::PointerMoved(to)]);
        assert!(armed(&h), "audio gutter armed while scrolled");
        h.release(to);
        h.frame(vec![]);
        assert_eq!(h.project.audio_tracks().len(), 2, "an audio track was created");
        // then video (the top gutter): drag V1's clip onto the top edge
        let (from, to) = (clip_at(&h, vid), pos2(h.state.x_at(2.0), lanes.top() + 2.0));
        h.press(from);
        h.frame(vec![Event::PointerMoved(to)]);
        assert!(armed(&h), "video gutter armed while scrolled");
        h.release(to);
        h.frame(vec![]);
        assert_eq!(h.project.video_tracks().len(), 7, "a video track was created");
    }

    #[test]
    fn headless_value_lane_clamps_to_the_property_range() {
        let mut h = Harness::new();
        let lanes = h.state.lanes_rect;
        h.project.tracks[0].clips[0].opacity.toggle_key(0.0);
        h.project.tracks[0].clips[0].opacity.toggle_key(2.0);
        h.project.tracks[0].clips[0].opacity.keys[1].v = 0.0;
        h.frame(vec![]);
        // keys at 1.0 and 0.0 auto-range to -0.5..1.5, so the low key sits a quarter up the lane and
        // dragging it to the top of the lane asks for 1.5
        let th = h.project.tracks[0].height;
        let inner = th - 2.0 - 2.0 * KEY_PAD;
        let kp = pos2(h.state.x_at(2.0), lanes.top() + th - 1.0 - KEY_PAD - 0.25 * inner);
        assert!(h.drag(kp, pos2(kp.x, lanes.top() + 2.0)), "value-lane drag edits");
        let keys = &h.project.tracks[0].clips[0].opacity.keys;
        assert_eq!(keys[1].v, 1.0, "opacity clamped to its 0..1 range, got {keys:?}");
        // effect params clamp to their ParamSpec
        assert_eq!(prop_range(h.video_clip(), 2), Some((0.01, 20.0)), "Scale");
        assert_eq!(prop_range(h.video_clip(), 0), None, "Position X is unbounded");
        assert_eq!(prop_range(h.audio_clip(), 0), None, "Volume is dB-scaled");
        assert_eq!(prop_range(h.audio_clip(), 1), Some((-1.0, 1.0)), "Pan");
        assert_eq!(prop_range(h.video_clip(), 5), Some((0.01, 100.0)), "Speed");
        assert_eq!(prop_range(h.audio_clip(), 2), Some((0.01, 100.0)), "Speed (audio)");
        h.project.tracks[0].clips[0].effects.push(Effect::new(EffectKind::Blur));
        let spec = h.video_clip().effects[0].specs()[0];
        assert_eq!(prop_range(h.video_clip(), 6), Some((spec.min, spec.max)), "first Blur param");
    }

    #[test]
    fn headless_audio_volume_keys_stay_in_the_bottom_strip() {
        let mut h = Harness::new();
        let lanes = h.state.lanes_rect;
        h.project.tracks[1].clips[0].volume.toggle_key(2.0);
        h.frame(vec![]);
        // the volume line is dB-mapped, so its keys keep the time-only strip: dragging one is time-only
        let row_bottom = lanes.top() + h.project.tracks[0].height + h.project.tracks[1].height;
        let kp = pos2(h.state.x_at(2.0), row_bottom - 1.0 - 5.0);
        assert!(h.drag(kp, kp + vec2(40.0, -20.0)), "keyframe drag edits");
        let keys = &h.audio_clip().volume.keys;
        assert_eq!(keys.len(), 1);
        assert!((keys[0].t - 3.0).abs() < 1.0 / 30.0 + 1e-6, "key moved to {}", keys[0].t);
        assert_eq!(keys[0].v, 1.0, "vertical drag does not touch a volume key's value");
    }

    #[test]
    fn headless_marker_click_seek_and_drag() {
        let mut h = Harness::new();
        let lanes = h.state.lanes_rect;
        let mid = h.project.add_marker(2.0, "cut here");
        h.frame(vec![]);
        let mp = pos2(h.state.x_at(2.0) + 1.0, lanes.top() - RULER_H + 4.0);
        h.press(mp);
        h.release(mp);
        h.frame(vec![]);
        assert_eq!(h.state.selected_marker, Some(mid), "click selects the marker");
        assert_eq!(h.playhead, 0.0, "a marker click does not scrub the ruler");
        // drag it 80 px right = 2 s
        assert!(h.drag(mp, mp + vec2(80.0, 0.0)), "marker drag edits");
        assert_eq!(h.undos, 1, "one undo per marker drag");
        assert!((h.project.markers[0].t - 4.0).abs() < 0.05, "marker at {}", h.project.markers[0].t);
        // and a clip marker moves inside its clip
        let cid = h.video_clip().id;
        let cm = h.project.add_clip_marker(cid, 1.0, "beat").unwrap();
        h.frame(vec![]);
        let cp = pos2(h.state.x_at(1.0) + 1.0, lanes.top() + 4.0);
        assert!(h.drag(cp, cp + vec2(40.0, 0.0)), "clip marker drag edits");
        assert_eq!(h.undos, 2);
        let m = h.video_clip().markers.iter().find(|m| m.id == cm).unwrap();
        assert!((m.t - 2.0).abs() < 0.05, "clip marker local t {}", m.t);
    }

    #[test]
    fn headless_clip_colour_and_menu_come_from_project_labels() {
        let mut h = Harness::new();
        h.project.labels.clear();
        let idx = h.project.add_label("Neon", [10, 200, 30]);
        h.project.tracks[0].clips[0].label = idx;
        h.frame(vec![]);
        assert!(h.has_fill(Color32::from_rgb(10, 200, 30)), "clip painted in the project label's colour");
        // the colour submenu lists the project's labels plus "Edit labels…"
        let (mut act, mut edit) = (None, false);
        let labels = h.project.labels.clone();
        let mut pos = None;
        let full = h.ctx.run(
            RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 400.0))), ..Default::default() },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| label_menu(ui, &labels, &mut act, &mut edit));
            },
        );
        for cs in &full.shapes {
            if let Shape::Text(t) = &cs.shape {
                if t.galley.text().contains("Neon") {
                    pos = Some(t.pos);
                }
                assert!(!t.galley.text().contains("Orange"), "built-in labels must not leak in");
            }
        }
        let pos = pos.expect("the project label is listed");
        // click it → Act::Label(1)
        let click = pos + vec2(4.0, 4.0);
        for pressed in [true, false] {
            let _ = h.ctx.run(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 400.0))),
                    events: vec![Event::PointerButton {
                        pos: click,
                        button: PointerButton::Primary,
                        pressed,
                        modifiers: Modifiers::NONE,
                    }],
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| label_menu(ui, &labels, &mut act, &mut edit));
                },
            );
        }
        assert!(matches!(act, Some(Act::Label(1))), "clicking the label picks index 1, got {:?}", act.is_some());
    }

    #[test]
    fn headless_adjustment_and_shape_clips_paint() {
        use crate::model::ShapeKind;
        let mut h = Harness::new();
        h.project.add_adjustment_clip(11.0, 3.0);
        h.project.add_shape_clip(ShapeKind::Rect, 15.0, 3.0);
        h.state.zoom = 20.0;
        h.frame(vec![]);
        let pal = Palette::new(true, Color32::from_rgb(0, 120, 212));
        assert!(h.has_fill(pal.clip_adjust), "adjustment clip uses its own fill");
        assert!(h.has_fill(pal.clip_shape), "shape clip uses its own fill");
        assert!(h.painted_text("adj").is_some(), "adjustment badge painted");
        assert!(h.painted_text("Rect").is_some(), "shape clip name painted");
    }

    #[test]
    fn headless_1000_clips_stays_fast() {
        let mut h = Harness::new();
        h.project = Project::new();
        for _ in 0..3 {
            h.project.add_track(TrackKind::Video);
        }
        let tracks = h.project.video_tracks();
        for i in 0..1000usize {
            let (ti, n) = (tracks[i % tracks.len()], (i / tracks.len()) as f64);
            let mut c = Clip::new(i as Id + 1000, ClipKind::Video, "clip", n * 2.0, 1.8);
            c.label = (i % 8) as u8 + 1;
            h.project.tracks[ti].clips.push(c);
        }
        h.project.tidy();
        h.frame(vec![]);
        let lanes = h.state.lanes_rect;
        h.state.zoom_to_fit(h.project.duration(), lanes.width());
        h.frame(vec![]);
        let t0 = std::time::Instant::now();
        for i in 0..20 {
            h.frame(vec![Event::PointerMoved(pos2(lanes.left() + 5.0 * i as f32, lanes.top() + 20.0))]);
        }
        let ms = t0.elapsed().as_secs_f64() * 1000.0 / 20.0;
        println!("timeline: 1000 clips, zoom-to-fit: {ms:.2} ms/frame");
        assert_eq!(h.project.all_clips().count(), 1000);
        // ~0.5 ms on this machine (dev profile, opt-level 1); the budget is 3 ms, the ceiling catches
        // O(n²) or per-clip-allocation regressions without being flaky on a loaded box
        assert!(ms < 10.0, "1000-clip frame took {ms:.2} ms");
        // and the same again zoomed in, where clips get their full detail treatment
        h.state.zoom = 40.0;
        h.state.scroll_x = 0.0;
        let t0 = std::time::Instant::now();
        for _ in 0..20 {
            h.frame(vec![]);
        }
        let ms = t0.elapsed().as_secs_f64() * 1000.0 / 20.0;
        println!("timeline: 1000 clips, zoom 40: {ms:.2} ms/frame");
        assert!(ms < 10.0, "zoomed-in frame took {ms:.2} ms");
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
    fn headless_transition_edge_drag_clamps_duration() {
        use crate::model::TransitionKind;
        let mut h = Harness::new();
        let lanes = h.state.lanes_rect;
        h.project.split_at(5.0, None);
        let right_id = h.project.tracks[0].clips[1].id;
        h.project.add_transition(right_id, TransitionKind::CrossFade, 1.0).unwrap();
        h.frame(vec![]);
        let dur = |h: &Harness| h.project.tracks[0].transitions.iter().find(|t| t.right == right_id).unwrap().duration;
        // two 5 s clips: the Transitions panel's 5 s cap binds
        let from = pos2(h.state.x_at(5.5) - 2.0, lanes.top() + 30.0);
        assert!(h.drag(from, from + vec2(300.0, 0.0)), "transition drag edits");
        assert!((dur(&h) - 5.0).abs() < 1e-6, "duration {}", dur(&h));
        // shrink the left clip to 1 s: the played window clamps to ±1 s, and the clips cap the drag at
        // 2 × the shorter one
        h.project.tracks[0].clips[0].trim_start(4.0, f64::INFINITY);
        h.frame(vec![]);
        let from = pos2(h.state.x_at(6.0) - 2.0, lanes.top() + 30.0);
        assert!(h.drag(from, from + vec2(300.0, 0.0)), "second transition drag edits");
        assert!((dur(&h) - 2.0).abs() < 1e-6, "duration {}", dur(&h));
    }

    #[test]
    fn headless_transition_drag_survives_project_swap() {
        use crate::model::TransitionKind;
        let mut h = Harness::new();
        let lanes = h.state.lanes_rect;
        h.project.split_at(5.0, None);
        let right_id = h.project.tracks[0].clips[1].id;
        h.project.add_transition(right_id, TransitionKind::CrossFade, 1.0).unwrap();
        h.frame(vec![]);
        let from = pos2(h.state.x_at(5.5) - 2.0, lanes.top() + 30.0);
        h.press(from);
        h.frame(vec![Event::PointerMoved(from + vec2(10.0, 0.0))]);
        assert!(matches!(h.state.drag, Some(Drag { g: Gesture::TransDur { .. }, .. })), "edge drag started");
        // undo / MCP project.open can replace the project mid-drag: the captured track index goes stale
        if let Some(Drag { g: Gesture::TransDur { track, .. }, .. }) = h.state.drag.as_mut() {
            *track = 9;
        }
        h.project = Project::new();
        h.frame(vec![Event::PointerMoved(from + vec2(40.0, 0.0))]); // used to panic: index out of bounds
        h.release(from + vec2(40.0, 0.0));
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

    /// Spacer: a lane drag shifts everything starting at or after the press, forward without limit and
    /// backward only as far as the clip in front of the group.
    #[test]
    fn headless_spacer_opens_and_closes_a_gap() {
        let mut h = Harness::new();
        let lanes = h.state.lanes_rect;
        h.project.tracks[1].clips.clear(); // video only
        h.project.tracks[0].clips.clear();
        h.project.tracks[0].clips.push(Clip::new(101, ClipKind::Video, "a", 0.0, 5.0));
        h.project.tracks[0].clips.push(Clip::new(102, ClipKind::Video, "b", 6.0, 5.0));
        h.tool = Tool::Spacer;
        h.frame(vec![]);
        let start = |h: &Harness, i: usize| h.project.tracks[0].clips[i].start;
        // press in the gap at t = 5.5 and drag +80 px (= +2 s at zoom 40)
        let from = pos2(h.state.x_at(5.5), lanes.top() + 30.0);
        assert!(h.drag(from, from + vec2(80.0, 0.0)), "spacer drag edits");
        assert_eq!(h.undos, 1, "one undo per gesture");
        assert_eq!(start(&h, 0), 0.0, "clips before the press stay put");
        assert!((start(&h, 1) - 8.0).abs() < 0.05, "gap opened to {}", start(&h, 1));
        // drag far left: the group stops on the clip in front of it instead of overlapping it
        assert!(h.drag(from, from - vec2(400.0, 0.0)), "closing drag edits");
        assert_eq!(h.undos, 2);
        assert_eq!(start(&h, 0), 0.0);
        assert!((start(&h, 1) - 5.0).abs() < 0.05, "gap closed to {}, not past the left clip", start(&h, 1));
        // pressing on a clip body spaces from there too, instead of moving that clip
        let body = pos2(h.state.x_at(2.0), lanes.top() + 30.0);
        assert!(h.drag(body, body + vec2(40.0, 0.0)), "body drag edits");
        assert_eq!(start(&h, 0), 0.0, "the pressed clip is not the one that moves");
        assert!((start(&h, 1) - 6.0).abs() < 0.05, "clips after the press moved to {}", start(&h, 1));
    }

    #[test]
    fn paste_clips_keeps_offsets_and_takes_fresh_ids() {
        use crate::engine::presets::{capture_template, decode_template};
        let mut h = Harness::new();
        h.project.tracks[1].clips.clear();
        h.project.split_at(4.0, None);
        let ids: Vec<Id> = h.project.tracks[0].clips.iter().map(|c| c.id).collect();
        let links: Vec<Id> = h.project.tracks[0].clips.iter().map(|c| c.link).collect();
        h.project.add_track(TrackKind::Video); // V2 = track index 1
        let tpl = capture_template("c", &h.project, &ids);

        let (clips, assets) = decode_template(&tpl).unwrap();
        let new = paste_clips(&mut h.project, clips, assets, 20.0, Some(1));
        assert_eq!(new.len(), 2);
        assert!(new.iter().all(|id| !ids.contains(id)), "fresh clip ids");
        let starts: Vec<f64> = new.iter().map(|&id| h.project.clip(id).unwrap().start).collect();
        assert_eq!(starts, vec![20.0, 24.0], "relative offsets survive the paste");
        assert!(new.iter().all(|&id| h.project.track_of(id) == Some(1)), "pasted onto the clicked track");
        let nl: Vec<Id> = new.iter().map(|&id| h.project.clip(id).unwrap().link).collect();
        assert!(nl.iter().all(|l| *l != 0 && !links.contains(l)), "fresh link ids: {nl:?} vs {links:?}");
        // no target (Paste In Place): the first track of the kind with room takes it
        let (clips, assets) = decode_template(&tpl).unwrap();
        let free = paste_clips(&mut h.project, clips, assets, 40.0, None);
        assert!(free.iter().all(|&id| h.project.track_of(id) == Some(0)), "V1 is free at 40 s");
    }

    #[test]
    fn waveform_takes_the_clip_label_colour() {
        let pal = Palette::new(true, Color32::from_rgb(0, 120, 212));
        assert_eq!(wave_color(pal.clip_audio, 0, &pal), pal.waveform, "unlabelled clips keep the palette wave");
        let bright = Color32::from_rgb(240, 220, 60);
        let w = wave_color(bright, 2, &pal);
        assert!(w.intensity() < bright.intensity(), "a bright label darkens its wave so it still reads");
        assert!(w.r() > w.b(), "the label's hue survives: {w:?}");
        let dark = Color32::from_rgb(30, 40, 120);
        assert!(wave_color(dark, 2, &pal).intensity() > dark.intensity(), "a dark label lightens its wave");
    }

    #[test]
    fn audio_clip_menu_swaps_add_mask_for_a_bus() {
        let mut p = Project::new();
        p.add_bus("Music");
        let ctx = egui::Context::default();
        let mut texts = |audio: bool| -> Vec<String> {
            let (mut act, mut acts, mut edit) = (None, Vec::new(), false);
            let full = ctx.run(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 1400.0))),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        clip_menu(ui, false, true, audio, &p.labels, &p.buses, &mut act, &mut acts, &mut edit)
                    });
                },
            );
            full.shapes
                .iter()
                .filter_map(|cs| match &cs.shape {
                    Shape::Text(t) => Some(t.galley.text().to_string()),
                    _ => None,
                })
                .collect()
        };
        let v = texts(false);
        assert!(v.iter().any(|s| s == "Add Mask"), "video clips keep Add Mask: {v:?}");
        assert!(!v.iter().any(|s| s == "Bus"), "video clips get no bus routing");
        let a = texts(true);
        assert!(!a.iter().any(|s| s == "Add Mask"), "a mask means nothing on audio: {a:?}");
        assert!(a.iter().any(|s| s == "Bus"), "audio clips get the bus submenu: {a:?}");
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
