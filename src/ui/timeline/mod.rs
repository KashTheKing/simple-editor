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
//! time/track; Path → returned in `dropped_files`; Effect / Transition → the clip under the pointer is
//! outlined while dragging and reported in `dropped_other`, so a card lands straight on that clip).
//! Every mutation calls `c.undo(&project)` with the project as
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
//!
//! Round 4: right-click quick-changes, additive to the menus above — selected transitions get "Change
//! Type" / "Change Easing" submenus on their band's context menu (absolute-overwrite every selected
//! transition, one undo for the whole bulk pick), and 2+ selected clips sharing an effect kind get an
//! "Effects" submenu on the clip context menu to toggle that shared effect on/off across the selection.
//! Any clip with a native size (video/image/sequence) also gets a "Transform" submenu — "Stretch to
//! Screen" / "Fit to Screen" (`Project::fit_clip_to_screen`), applied to every such clip in the
//! selection as one undo step.

use crate::media::thumbs::ThumbCache;
use crate::media::waveform::{Peaks, WaveformCache};
use crate::model::{Asset, Clip, ClipKind, Ease, EffectKind, Id, Label, Project, TrackKind, TransitionKind};
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
/// Height of the subtitle lane pinned under the ruler (shown only when the project has cues).
const SUB_LANE_H: f32 = 20.0;
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
/// On-screen clip width (same measure as `detailed` below) above which the keyframe mini-graph toggle
/// appears: wide enough for the corner icon plus a graph worth looking at, not just barely visible.
const MINI_GRAPH_MIN_W: f32 = 120.0;
/// Mini-graph toggle: a square icon inset this far from the clip's top-right corner, this many px across.
const MINI_GRAPH_PAD: f32 = 3.0;
const MINI_GRAPH_BTN: f32 = 15.0;
/// Height of the inline mini-graph panel dropped below a clip whose toggle is on.
const MINI_GRAPH_H: f32 = 48.0;
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
    /// Subtitle lane height (Alt+scroll over the lane resizes it, like tracks).
    pub sub_h: f32,
    /// Selected cues on the subtitle lane (band drag / Ctrl+click) — bulk convert/delete targets.
    pub sub_sel: Vec<Id>,
    /// Band-select in progress on the subtitle lane: press-origin time and "Shift held" (add to selection).
    sub_band: Option<(f64, bool)>,
    /// Cue edge being trimmed on the subtitle lane: (cue id, right edge?).
    sub_trim: Option<(Id, bool)>,
    /// Clip ids whose inline keyframe mini-graph (toggled by the corner icon) is open.
    mini_graph_open: Vec<Id>,
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
            sub_h: SUB_LANE_H,
            sub_sel: Vec::new(),
            sub_band: None,
            sub_trim: None,
            mini_graph_open: Vec::new(),
        }
    }
}

/// Paste a copied group (relative times, the way `presets::capture_template` stores one) at `at`, with
/// fresh clip / link / marker ids. `Project::place_clips` picks the first track of each kind with room
/// (adding one when nothing is free); `target` then pulls the group onto the row the user last clicked,
/// if the whole group fits there.
/// The paste flavours, shared by every timeline context menu. The app decides whether the clipboard
/// actually holds anything — a menu that hid itself when empty would just look broken.
fn paste_menu(ui: &mut egui::Ui, actions: &mut Vec<crate::hotkeys::Action>) {
    use crate::hotkeys::Action;
    for (label, a) in [
        ("Paste", Action::PasteClips),
        ("Paste Insert", Action::PasteInsert),
        ("Paste At Top", Action::PasteAtTop),
        ("Paste In Place", Action::PasteInPlace),
    ] {
        if ui.button(label).clicked() {
            actions.push(a);
            ui.close();
        }
    }
}

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
    /// Selected transition ids (timeline bands) — separate from the clip selection: clicking one
    /// kind of thing deselects the other, Ctrl adds within its own kind.
    pub sel_transitions: &'a mut Vec<Id>,
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
    /// Other dnd payloads (Sequence / Template / Effect / Transition) dropped on the lanes — the app
    /// places them; an Effect or a Transition goes onto the clip that contains the reported time.
    pub dropped_other: Vec<(DragPayload, f64, Option<usize>)>,
    /// Actions requested from the timeline's context menus (Retime, AddTransition, FreezeFrame, AutoCut, …).
    pub actions: Vec<crate::hotkeys::Action>,
    /// Container media replacement request — (clip id, pair mode).
    pub replace_container: Option<(Id, bool)>,
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
    /// Right-click quick-change: toggle a shared effect kind's enabled state across the selection.
    /// The new state is the opposite of the first selected clip carrying that kind (mirrors the
    /// Inspector's "first sets the baseline" convention, just with nothing to diff since the menu
    /// only offers one action — toggle — rather than a value picker).
    ToggleEffect(EffectKind),
    /// Right-click quick-change: "Stretch to Screen" on every selected clip with a native size —
    /// `Project::fit_clip_to_screen(id, true)`, applied per clip (each against its own native size).
    StretchToScreen,
    /// Right-click quick-change: "Fit to Screen" on every selected clip with a native size —
    /// `Project::fit_clip_to_screen(id, false)`.
    FitToScreen,
    /// Set the easing of every property keyed at clip-local time `t`.
    SetEase(Id, f64, Ease),
    /// Delete every keyframe at clip-local time `t`.
    DelKeys(Id, f64),
    RemoveTransition(Id),
    RemoveTransitions(Vec<Id>),
    /// Right-click quick-change: absolute-overwrite every listed transition's kind. Unlike
    /// `inspector.rs`'s `transition_section` (diff-against-first, only fields the user actually
    /// touched get written), a menu click IS the new value, so there's nothing to diff against.
    SetTransitionsKind(Vec<Id>, TransitionKind),
    /// Right-click quick-change: absolute-overwrite every listed transition's ease (see
    /// `SetTransitionsKind`).
    SetTransitionsEase(Vec<Id>, Ease),
    /// Add a project marker at this timeline time.
    AddMarker(f64),
    /// Razor tool: split every clip crossing this timeline time.
    SplitAt(f64),
    RenameMarker(Id, String),
    DelMarker(Id),
    MarkerLabel(Id, u8),
    /// Route the selected audio clips to a bus (0 = inherit the track's).
    Bus(Id),
    /// Replace media of a container clip.
    ReplaceContainerMedia(Id),
    /// Replace media of a container clip and linked audio.
    ReplaceContainerPair(Id),
    /// Convert selection to containers.
    MakeContainer,
    /// Remove container flag from selection.
    UnmakeContainer,
    /// Rename a container's slot label.
    RenameContainer(Id, String),
}

mod cue_lane;
mod gestures;
mod header;
mod menus;
mod paint;
#[cfg(test)]
mod tests;

use menus::{clip_menu, label_menu, shared_effect_kinds, transition_ease_menu, transition_kind_menu};
use paint::*;

pub fn show(ui: &mut egui::Ui, state: &mut TimelineState, mut c: TimelineCtx<'_>) -> TimelineResponse {
    let mut out = TimelineResponse::default();
    let pal = *c.palette;
    let full = ui.available_rect_before_wrap();
    ui.allocate_rect(full, Sense::hover());
    let id = ui.id().with("timeline");
    let content_h: f32 = c.project.tracks.iter().map(|t| t.height).sum();
    let sub_h = if c.project.subtitles.is_empty() { 0.0 } else { state.sub_h };
    let vbar_w = if content_h > full.height() - RULER_H - sub_h - HBAR_H { VBAR_W } else { 0.0 };
    let ruler =
        Rect::from_min_max(pos2(full.left() + state.header_w, full.top()), pos2(full.right(), full.top() + RULER_H));
    let subs_lane =
        Rect::from_min_max(pos2(ruler.left(), ruler.bottom()), pos2(full.right() - vbar_w, ruler.bottom() + sub_h));
    let lanes =
        Rect::from_min_max(pos2(ruler.left(), subs_lane.bottom()), pos2(full.right() - vbar_w, full.bottom() - HBAR_H));
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
                    if sub_h > 0.0 && subs_lane.contains(pos) {
                        state.sub_h = (state.sub_h + delta.y * 0.25).clamp(SUB_LANE_H, 80.0);
                    } else if let Some(ti) = state.track_at(pos.y, c.project) {
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

    // the row under the press is where Ctrl+V pastes — right-click counts, so the context menu's
    // Paste lands on the row you opened it over
    if ui.input(|i| i.pointer.primary_pressed() || i.pointer.secondary_pressed()) {
        if let Some(ti) = pointer.filter(|p| lanes.contains(*p)).and_then(|p| state.track_at(p.y, c.project)) {
            state.last_track = Some(ti);
        }
    }

    let mut act: Option<Act> = None;
    let mut click: Option<Id> = None;
    let mut trans_click: Option<Id> = None;
    // drop selected transitions that no longer exist (removed, pruned, project swapped)
    c.sel_transitions.retain(|id| c.project.tracks.iter().any(|t| t.transitions.iter().any(|x| x.id == *id)));
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
    // track-resize handle rects, collected as they're laid out below so the rubber-band-start check
    // (near the end of this function) can tell a resize press from an empty-lane press
    let mut handle_rects: Vec<Rect> = Vec::new();
    let mut key_hits: Vec<(Id, f64, Pos2, Option<usize>)> = Vec::new();
    let mut marker_hits: Vec<(Id, Id, Rect)> = Vec::new();
    let linked_sel = c.project.expand_links(c.selection);
    let thin = Stroke::new(1.0, pal.border);
    // rubber band: hit-tested while the rows are painted, applied on release
    let band_rect = state.band.map(|(o, _)| Rect::from_two_pos(o, pointer.unwrap_or(o)).intersect(lanes));
    let mut band_ids: Vec<Id> = Vec::new();
    let mut band_trans: Vec<Id> = Vec::new();
    // marker rename buffer, moved out of `state` so the menu closures can borrow it while `state` paints
    let mut rename = state.rename.take();
    let mut seek_marker: Option<f64> = None;
    let labels: &[Label] = &c.project.labels;
    let buses: &[crate::model::Bus] = &c.project.buses;
    // multi-clip right-click "Effects" quick-toggle: only offered when 2+ selected clips share a kind
    let shared_effects = shared_effect_kinds(c.project, c.selection);

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

        let tid = id.with(track.id);
        let active = c.project.active(ti);
        if let Some(a) = header::draw_header(ui, &bp, header, row, id, &pal, &font, &small, track, ti, active) {
            act = Some(a);
        }

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
            let cr = CornerRadius::same(pal.clip_rounding as u8);
            lp.rect_filled(rect, cr, color);
            lp.rect_stroke(rect, cr, thin, StrokeKind::Inside);
            if clip.container {
                lp.rect_stroke(rect, cr, Stroke::new(1.5, pal.accent), StrokeKind::Inside);
            }
            // sub-pixel clips (zoomed way out): fill only — no text, waveform, diamonds or hit-testing.
            // ponytail: 4 pt is well under the ~18 pt a clip needs for its trim handles, and the rubber
            // band still catches them; give them their own interact if that ever bites.
            let detailed = vis.width() >= 4.0;
            if clip.kind == ClipKind::Adjustment && detailed {
                hatch(&lp.with_clip_rect(vis), vis, pal.text.gamma_multiply(0.25));
            } else if clip.is_empty_container() && detailed {
                hatch(&lp.with_clip_rect(vis), vis, pal.accent.gamma_multiply(0.35));
            }
            if !detailed {
                if c.selection.contains(&clip.id) {
                    lp.rect_stroke(rect, cr, Stroke::new(2.0, pal.selection), StrokeKind::Inside);
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
            } else if clip.container {
                let badge = Rect::from_min_size(name_pos, vec2(15.0, 15.0));
                draw_glyph(&name_pc, badge, Glyph::Container, pal.accent);
                let label = if clip.is_empty_container() {
                    if !clip.container_label.is_empty() {
                        format!("[{}] (Empty)", clip.container_label)
                    } else {
                        "Container (Empty)".to_string()
                    }
                } else if !clip.container_label.is_empty()
                    && !clip.name.contains(&format!("[{}]", clip.container_label))
                {
                    format!("{} [{}]", clip.name, clip.container_label)
                } else {
                    clip.name.clone()
                };
                name_pc.text(pos2(badge.right() + 2.0, name_pos.y), Align2::LEFT_TOP, &label, font.clone(), pal.text);
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
            }
            if clip.kind != ClipKind::Adjustment {
                // fade ramps + handles: audio gain fades, opacity fades on visual clips
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
                lp.rect_stroke(rect, cr, Stroke::new(2.0, pal.selection), StrokeKind::Inside);
            } else if linked_sel.contains(&clip.id) {
                lp.rect_stroke(rect, cr, Stroke::new(1.0, pal.selection), StrokeKind::Inside);
            }

            // interaction: body, then volume line, then edges, then fade handles on top
            let cid = id.with(clip.id);
            let br = ui.interact(vis, cid, Sense::click_and_drag());
            if br.clicked() {
                // razor / marker tools act where the pointer is instead of selecting
                let (snap_on, zoom, ph) = (c.snap, state.zoom, *c.playhead);
                match c.tool {
                    Tool::Cut => {
                        let x = ui.input(|i| i.pointer.latest_pos()).unwrap_or(vis.center()).x;
                        let t = snap_time(state.time_at(x), snap_on, zoom, c.project, ph, &[]);
                        act = Some(Act::SplitAt(t));
                    }
                    Tool::Marker => {
                        let x = ui.input(|i| i.pointer.latest_pos()).unwrap_or(vis.center()).x;
                        let t = snap_time(state.time_at(x), snap_on, zoom, c.project, ph, &[]);
                        act = Some(Act::AddMarker(t.max(0.0)));
                    }
                    _ => click = Some(clip.id),
                }
            }
            if br.double_clicked() {
                c.selection.clear();
                c.selection.push(clip.id);
                if clip.kind == ClipKind::Sequence {
                    out.open_sequence = Some(clip.sequence);
                } else if let Some(p) = br.interact_pointer_pos() {
                    *c.playhead = snap_playhead(state.time_at(p.x).max(0.0), c.snap, state.zoom, c.project);
                    out.seeked = true;
                }
            }
            if br.drag_started_by(egui::PointerButton::Primary) {
                start_move = Some(clip.id);
            }
            let (linked, enabled, aud, is_cont) =
                (clip.link != 0, clip.enabled, clip.kind == ClipKind::Audio, clip.container);
            let has_native = c.project.clip_native_size(clip).is_some();
            let graph_open = has_curve_keys(clip).then(|| state.mini_graph_open.contains(&clip.id));
            let mut toggle_graph = false;
            let mut rclick = br.secondary_clicked();
            br.context_menu(|ui| {
                clip_menu(
                    ui,
                    clip.id,
                    is_cont,
                    linked,
                    enabled,
                    aud,
                    has_native,
                    graph_open,
                    &mut toggle_graph,
                    labels,
                    buses,
                    &shared_effects,
                    &mut act,
                    &mut out.actions,
                    &mut out.edit_labels,
                )
            });
            if toggle_graph {
                match state.mini_graph_open.iter().position(|&x| x == clip.id) {
                    Some(i) => {
                        state.mini_graph_open.remove(i);
                    }
                    None => state.mini_graph_open.push(clip.id),
                }
            }
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
                            clip.id,
                            is_cont,
                            linked,
                            enabled,
                            aud,
                            has_native,
                            None, // edge-handle menu: skip the mini-graph entry, body right-click has it
                            &mut false,
                            labels,
                            buses,
                            &shared_effects,
                            &mut act,
                            &mut out.actions,
                            &mut out.edit_labels,
                        )
                    });
                }
            }
            if clip.kind != ClipKind::Adjustment {
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
            // keyframe mini-graph: once the clip is wide enough on screen to be worth it (same measure
            // as `detailed` above, just a higher bar), a small toggle icon sits in its top-right corner.
            // Anchored to the VISIBLE right edge (`vis`), not the clip's own — a zoomed-in clip whose
            // right edge is off-screen used to lose the button entirely, the opposite of "show it when
            // zoomed in". Registered last so it wins hit-testing over the body underneath it.
            if has_curve_keys(clip) && vis.width() >= MINI_GRAPH_MIN_W {
                let btn = Rect::from_min_size(
                    pos2(vis.right() - MINI_GRAPH_PAD - MINI_GRAPH_BTN, rect.top() + MINI_GRAPH_PAD),
                    vec2(MINI_GRAPH_BTN, MINI_GRAPH_BTN),
                )
                .intersect(lanes);
                if btn.is_positive() {
                    let open = state.mini_graph_open.contains(&clip.id);
                    let clicked =
                        toggle_button(ui, &lp, btn, cid.with("mg"), Cap::Icon(Glyph::Diamond), open, &pal, &small);
                    if clicked {
                        if open {
                            state.mini_graph_open.retain(|&x| x != clip.id);
                        } else {
                            state.mini_graph_open.push(clip.id);
                        }
                    }
                }
                if state.mini_graph_open.contains(&clip.id) {
                    draw_mini_graph(&lp, clip, rect, lanes, &pal);
                }
            }
        }

        // transitions: an overlapping band on each valid cut / clip edge
        for tr in &track.transitions {
            let Some((left, right)) = track.transition_clips(tr) else { continue };
            // the played window is clamped to the clip(s) (an over-long transition can't reach past them)
            let Some((cut, half)) = tr.cut_half(left, right) else { continue };
            let (wa, wb) = (cut - half, cut + half);
            let (xa, xb) = (state.x_at(wa), state.x_at(wb));
            if xb < lanes.left() || xa > lanes.right() {
                continue;
            }
            let band = Rect::from_min_max(pos2(xa, row.top() + 1.0), pos2(xb, row.bottom() - 1.0));
            let bvis = band.intersect(lanes);
            if let Some(br) = band_rect {
                if br.intersects(band) {
                    band_trans.push(tr.id);
                }
            }
            let selected = c.sel_transitions.contains(&tr.id);
            lp.rect_filled(band, 0, pal.bg.gamma_multiply(0.5));
            if selected {
                lp.rect_filled(band, 0, pal.selection.gamma_multiply(0.25));
            }
            let (bs, bc) = if selected { (2.0, pal.selection) } else { (1.0, pal.accent) };
            lp.rect_stroke(band, 0, Stroke::new(bs, bc), StrokeKind::Inside);
            if bvis.is_positive() {
                lp.with_clip_rect(bvis).text(
                    band.center(),
                    Align2::CENTER_CENTER,
                    tr.kind.name(),
                    small.clone(),
                    pal.text,
                );
                let br = ui.interact(bvis, id.with(tr.id), Sense::click());
                if br.clicked() {
                    trans_click = Some(tr.id);
                }
                if br.secondary_clicked() && !selected {
                    c.sel_transitions.clear();
                    c.sel_transitions.push(tr.id);
                    c.selection.clear();
                }
                br.context_menu(|ui| {
                    let many = selected && c.sel_transitions.len() > 1;
                    let label = if many {
                        format!("Remove {} Selected Transitions", c.sel_transitions.len())
                    } else {
                        "Remove Transition".into()
                    };
                    if ui.button(label).clicked() {
                        act = Some(if many {
                            Act::RemoveTransitions(c.sel_transitions.clone())
                        } else {
                            Act::RemoveTransition(tr.id)
                        });
                    }
                    ui.separator();
                    // right-click quick-change: bulk-edits every selected transition (or just this
                    // one if it wasn't already part of the selection — `sel_transitions` was reset
                    // to just `tr.id` above in that case), same absolute-overwrite as picking a new
                    // value in the Inspector's transition kind/ease combo boxes.
                    ui.menu_button("Change Type", |ui| transition_kind_menu(ui, c.sel_transitions, &mut act));
                    ui.menu_button("Change Easing", |ui| transition_ease_menu(ui, c.sel_transitions, &mut act));
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
        handle_rects.push(handle);
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
                c.sel_transitions.clear();
            }
            for cid in band_ids.drain(..) {
                if !c.selection.contains(&cid) {
                    c.selection.push(cid);
                }
            }
            for tid in band_trans.drain(..) {
                if !c.sel_transitions.contains(&tid) {
                    c.sel_transitions.push(tid);
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

    // ---- project markers on the ruler (only the open sequence's — same filter as markers_ui::rows;
    // an unfiltered ruler let another timeline's markers be dragged/deleted from the wrong context) ----
    for m in &c.project.markers {
        if m.sequence != c.project.editing {
            continue;
        }
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
        ui.separator();
        paste_menu(ui, &mut out.actions);
    });

    cue_lane::draw(
        ui,
        &mut c,
        state,
        &mut out,
        &painter,
        sub_h,
        subs_lane,
        &pal,
        &small,
        thin,
        full,
        id,
        mods,
        pointer,
        primary_down,
    );

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
        *c.playhead = snap_playhead(state.time_at(p.x).max(0.0), c.snap, state.zoom, c.project);
        out.seeked = true;
    }

    // ---- lanes background: deselect / rubber band / context menu / dnd ----
    if lanes_resp.clicked() && !mods.ctrl {
        // the marker tool also works on empty lanes; everything else just deselects
        match (c.tool, lanes_resp.interact_pointer_pos()) {
            (Tool::Marker, Some(pp)) => {
                let t = snap_time(state.time_at(pp.x), c.snap, state.zoom, c.project, *c.playhead, &[]);
                act = Some(Act::AddMarker(t.max(0.0)));
            }
            _ => {
                c.selection.clear();
                c.sel_transitions.clear();
            }
        }
    }
    // a press that missed every clip starts a rubber band (Shift adds to the selection)
    if state.drag.is_none() && lanes_resp.drag_started_by(egui::PointerButton::Primary) {
        // a vertical resize drag can cross HANDLE_H's few px before egui recognizes the drag as
        // started, so lanes_resp can still see this as a drag-start too; use where the press actually
        // began (not the possibly-drifted current pointer pos) to tell a resize press from an
        // empty-lane one, and let the handle keep the gesture instead of also opening a rubber band.
        let press_origin = ui.input(|i| i.pointer.press_origin()).or(pointer);
        let on_resize_handle = press_origin.is_some_and(|p| handle_rects.iter().any(|r| r.contains(p)));
        if !on_resize_handle {
            if c.tool == Tool::Spacer {
                start_spacer = true;
            } else if let Some(o) = lanes_resp.interact_pointer_pos().or(pointer) {
                state.band = Some((o, mods.shift));
            }
        }
    }
    lanes_resp.context_menu(|ui| {
        paste_menu(ui, &mut out.actions);
        ui.separator();
        if ui.button("Add Video Track").clicked() {
            act = Some(Act::AddTrack(TrackKind::Video));
        }
        if ui.button("Add Audio Track").clicked() {
            act = Some(Act::AddTrack(TrackKind::Audio));
        }
    });
    if let Some(pos) = pointer {
        let (snap_on, ph) = (c.snap, *c.playhead);
        let drop_t = move |state: &TimelineState, p: &Project| {
            snap_time(state.time_at(pos.x), snap_on, state.zoom, p, ph, &[]).max(0.0)
        };
        if let Some(payload) = lanes_resp.dnd_hover_payload::<DragPayload>() {
            let t = drop_t(state, c.project);
            // an effect lands on ONE clip and a transition on ONE cut, so they highlight what is under
            // the pointer instead of a ghost clip that would lie about a duration
            let target = match &*payload {
                DragPayload::Effect(k) => drop_on_clip(state, c.project, pos, t)
                    .filter(|(_, cl)| (cl.kind == ClipKind::Audio) == k.applies_to_audio())
                    .map(|(r, _)| r),
                DragPayload::Transition(_) => drop_on_clip(state, c.project, pos, t).map(|(r, cl)| {
                    // the half you are over picks the cut (same rule the app applies on release)
                    let x = if crate::ui::transitions_ui::drop_at_end(cl, t) { r.right() } else { r.left() };
                    Rect::from_min_max(pos2(x - 6.0, r.top()), pos2(x + 6.0, r.bottom()))
                }),
                _ => None,
            };
            if let Some(hit) = target {
                lp.rect_filled(hit, 0, pal.selection.gamma_multiply(0.35));
                lp.rect_stroke(hit, 0, Stroke::new(2.0, pal.selection), StrokeKind::Inside);
            } else if !matches!(&*payload, DragPayload::Effect(_) | DragPayload::Transition(_)) {
                let dur = match &*payload {
                    DragPayload::Asset(aid) => {
                        c.project.asset(*aid).map(|a| if a.kind == ClipKind::Image { 5.0 } else { a.duration })
                    }
                    DragPayload::Sequence(sid) => Some(c.project.sequence_duration(*sid)),
                    _ => None,
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
    if let Some(tid) = trans_click {
        // transitions select like clips: click replaces, Ctrl toggles; the clip selection makes way
        if mods.ctrl {
            match c.sel_transitions.iter().position(|&x| x == tid) {
                Some(i) => {
                    c.sel_transitions.remove(i);
                }
                None => c.sel_transitions.push(tid),
            }
        } else {
            *c.sel_transitions = vec![tid];
            c.selection.clear();
        }
    }
    if let Some(cid) = click {
        if !mods.ctrl {
            c.sel_transitions.clear();
        }
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
            Act::ToggleEffect(k) => {
                // absolute overwrite: everyone gets the opposite of what the first selected clip
                // carrying this effect kind currently has (there's nothing to diff, it's a toggle)
                let target = !ids
                    .iter()
                    .find_map(|&id| p.clip(id)?.effects.iter().find(|e| e.kind == k).map(|e| e.enabled))
                    .unwrap_or(true);
                for &id in &ids {
                    if let Some(cl) = p.clip_mut(id) {
                        if let Some(e) = cl.effects.iter_mut().find(|e| e.kind == k) {
                            e.enabled = target;
                        }
                    }
                }
            }
            Act::StretchToScreen => {
                for &id in &ids {
                    p.fit_clip_to_screen(id, true);
                }
            }
            Act::FitToScreen => {
                for &id in &ids {
                    p.fit_clip_to_screen(id, false);
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
            Act::RemoveTransitions(tids) => {
                for tid in tids {
                    p.remove_transition(tid);
                }
                c.sel_transitions.clear();
            }
            Act::SetTransitionsKind(tids, k) => {
                for tid in tids {
                    if let Some(t) = p.transition_mut(tid) {
                        t.kind = k;
                    }
                }
            }
            Act::SetTransitionsEase(tids, e) => {
                for tid in tids {
                    if let Some(t) = p.transition_mut(tid) {
                        t.ease = e;
                    }
                }
            }
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
            Act::ReplaceContainerMedia(cid) => {
                out.replace_container = Some((cid, false));
            }
            Act::ReplaceContainerPair(cid) => {
                out.replace_container = Some((cid, true));
            }
            Act::MakeContainer => {
                p.make_container(&ids);
            }
            Act::UnmakeContainer => {
                p.unmake_container(&ids);
            }
            Act::RenameContainer(cid, name) => {
                if let Some(cl) = p.clip_mut(cid) {
                    cl.container_label = name;
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

    gestures::handle(
        ui,
        state,
        &mut c,
        pointer,
        mods,
        lanes,
        start_spacer,
        start_move,
        start_trim,
        start_vol,
        start_fade,
        start_key,
        start_trans,
        start_marker,
    );
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
