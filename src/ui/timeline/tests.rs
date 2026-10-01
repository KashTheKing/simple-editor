//! Timeline widget tests, moved verbatim from the old monolithic timeline.rs.
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
fn markers_and_transitions_are_snap_candidates() {
    use crate::model::TransitionKind;
    let mut p = Project::new();
    p.tracks[0].clips.push(Clip::new(7, ClipKind::Video, "a", 0.0, 3.0)); // [0,3)
    p.tracks[0].clips.push(Clip::new(8, ClipKind::Video, "b", 3.0, 3.0)); // [3,6) abuts clip 7
    p.add_marker(10.0, "M");
    p.add_transition(8, TransitionKind::CrossFade, 1.0).expect("abutting clips make a transition");
    // marker hit: far from playhead/edges, close to the marker at 10.0
    let mhit = target(&p, 10.1, 0.3, 999.0, &[], &[], None, true);
    assert_eq!(mhit.map(|(_, k)| k), Some(SnapKind::Marker), "marker must be a candidate");
    // transition hit: cut at 3.0, half=0.5s -> played-window edges at 2.5 / 3.5; exclude both
    // clips so their own edges (0, 3, 6) don't shadow the transition-edge tier at 2.55
    let thit = target(&p, 2.55, 0.3, 999.0, &[7, 8], &[], None, true);
    assert_eq!(thit.map(|(_, k)| k), Some(SnapKind::TransitionEdge), "transition edge must be a candidate");
    // snap_markers=false removes the marker hit but not the transition one
    assert_eq!(target(&p, 10.1, 0.3, 999.0, &[], &[], None, false), None, "marker hit must disappear");
    let thit2 = target(&p, 2.55, 0.3, 999.0, &[7, 8], &[], None, false);
    assert_eq!(thit2.map(|(_, k)| k), Some(SnapKind::TransitionEdge), "transition hit must survive");
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
    let mut s =
        TimelineState { lanes_rect: Rect::from_min_max(pos2(100.0, 200.0), pos2(900.0, 500.0)), ..Default::default() };
    // display order: V2 (70) V1 (50) A1 (40) A2 (60)
    assert_eq!(row_order(&p, (true, true)).collect::<Vec<_>>(), [1, 0, 2, 3]);
    assert_eq!(row_order(&p, (false, true)).collect::<Vec<_>>(), [2, 3], "hidden video rows drop out");
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
    let mut s =
        TimelineState { lanes_rect: Rect::from_min_max(pos2(100.0, 0.0), pos2(900.0, 100.0)), ..Default::default() };
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
use crate::settings::TimelineView;
use egui::{Event, Modifiers, PointerButton, RawInput};

/// ws:pro-timeline: "everything on" view preset for the harness, so every pre-existing test's
/// waveform/filmstrip/keyframe/clip-text assertions keep passing unchanged.
const DETAILED_VIEW: TimelineView =
    TimelineView { name: String::new(), waves: true, thumbs: true, keys: true, clip_text: true, row_h: 64.0 };

struct Harness {
    ctx: egui::Context,
    state: TimelineState,
    project: Project,
    selection: Vec<Id>,
    sel_transitions: Vec<Id>,
    playhead: f64,
    undos: usize,
    waves: WaveformCache,
    tool: Tool,
    snap: bool,
    snap_markers: bool,
    time: f64,
    /// Paint list of the last frame (asserting on what was actually drawn).
    shapes: Vec<egui::epaint::ClippedShape>,
    // ---- ws:pro-timeline ----
    view: TimelineView,
    overview: bool,
    boring_thr: (f32, f32),
    realtime: Vec<(f64, f64, bool, bool)>,
    // ---- ws:timeline-surface ----
    views: Vec<TimelineView>,
}

impl Harness {
    fn new() -> Self {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts()); // size-diet: no default_fonts feature anymore
        ctx.set_fonts(crate::theme::test_fonts()); // size-diet: no default_fonts feature anymore
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
            rel_path: None,
            parent: None,
            range: None,
            effects: Vec::new(),
        });
        project.insert_asset_clips(aid, 0.0, None);
        let waves = WaveformCache::new(ctx.clone(), Backend::Ffmpeg);
        let mut h = Self {
            ctx,
            state: TimelineState::default(),
            project,
            selection: Vec::new(),
            sel_transitions: Vec::new(),
            playhead: 0.0,
            undos: 0,
            waves,
            tool: Tool::Select,
            snap: false,
            snap_markers: true,
            time: 0.0,
            shapes: Vec::new(),
            view: DETAILED_VIEW,
            overview: false,
            boring_thr: (1.5, 20.0),
            realtime: Vec::new(),
            views: vec![
                TimelineView { name: "Detailed".into(), ..DETAILED_VIEW },
                TimelineView { name: "Compact".into(), waves: false, thumbs: false, keys: false, ..DETAILED_VIEW },
            ],
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
        let Harness {
            ctx,
            state,
            project,
            selection,
            sel_transitions,
            playhead,
            undos,
            waves,
            tool,
            snap,
            snap_markers,
            view,
            overview,
            boring_thr,
            realtime,
            views,
            ..
        } = self;
        let mut resp = None;
        let full = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let mut undo = |_: &Project, _: &'static str| *undos += 1;
                resp = Some(show(
                    ui,
                    state,
                    TimelineCtx {
                        project,
                        selection,
                        sel_transitions,
                        playhead,
                        undo: &mut undo,
                        waveforms: waves,
                        palette: &pal,
                        snap: *snap,
                        snap_markers: *snap_markers,
                        playing: false,
                        thumbs: None,
                        keep_ranges: &[],
                        prerender: &[],
                        tool: *tool,
                        library_selected: None,
                        view,
                        overview: *overview,
                        boring_thr: *boring_thr,
                        realtime,
                        views,
                    },
                ));
            });
        });
        self.shapes = full.shapes;
        resp.unwrap()
    }
    /// Advance the synthetic clock well past `max_double_click_delay` (0.3 s) so the next click
    /// isn't merged into a double-click with the previous one - egui's double-click timer is a
    /// single global `last_click_time`, not scoped to a widget/position, so two unrelated clicks
    /// (even on different clips) count as a double-click if the harness's virtual clock hasn't
    /// moved far enough between them (see agents.md's "double-click timing in synthetic clocks").
    fn settle(&mut self) {
        self.time += 1.0;
        self.frame(vec![]);
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
    /// Is any line segment (e.g. `hatch()`'s diagonals) painted in this stroke colour?
    fn has_line(&self, color: Color32) -> bool {
        self.shapes.iter().any(|cs| matches!(&cs.shape, Shape::LineSegment { stroke, .. } if stroke.color == color))
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
    /// Like `drag`, but every event (press, each move step, release) carries `mods`.
    fn drag_m(&mut self, from: Pos2, to: Pos2, mods: Modifiers) -> bool {
        self.press_m(from, mods);
        let mut edited = false;
        for i in 1..=4 {
            let p = from + (to - from) * (i as f32 / 4.0);
            edited |= self.frame_m(vec![Event::PointerMoved(p)], mods).edited;
        }
        edited |= self.release_m(to, mods).edited;
        edited |= self.frame(vec![]).edited;
        edited
    }
    fn video_clip(&self) -> &Clip {
        &self.project.tracks[0].clips[0]
    }
    fn audio_clip(&self) -> &Clip {
        &self.project.tracks[1].clips[0]
    }
    /// Right-click at `pos` and let the context menu lay out (ws:timeline-surface).
    fn rclick(&mut self, pos: Pos2) -> TimelineResponse {
        let sec = |pressed| Event::PointerButton {
            pos,
            button: PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        self.frame(vec![Event::PointerMoved(pos)]);
        self.frame(vec![sec(true)]);
        let r = self.frame(vec![sec(false)]);
        self.frame(vec![]);
        r
    }
    /// Click the painted text `label` (a menu row, or a submenu to open), then settle a frame. Returns
    /// the release frame's response (a menu row's Act / Action fires there), `edited` OR-ed with the next.
    fn click_text(&mut self, label: &str) -> TimelineResponse {
        let p = self.painted_text(label).unwrap_or_else(|| panic!("'{label}' not painted: {:?}", self.texts()))
            + vec2(4.0, 4.0);
        self.press(p);
        let mut r = self.release(p);
        r.edited |= self.frame(vec![]).edited;
        r
    }
}

/// `draw` in a tall bare panel, with a click-by-label helper - for menu bodies called directly.
struct MenuProbe {
    ctx: egui::Context,
    time: f64,
    shapes: Vec<egui::epaint::ClippedShape>,
}

impl MenuProbe {
    fn new() -> Self {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        Self { ctx, time: 0.0, shapes: Vec::new() }
    }
    fn run(&mut self, events: Vec<Event>, draw: &mut dyn FnMut(&mut egui::Ui)) {
        self.time += 0.05;
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 1400.0))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        self.shapes = self
            .ctx
            .run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| draw(ui));
            })
            .shapes;
    }
    fn texts(&self) -> Vec<String> {
        self.shapes
            .iter()
            .filter_map(|cs| match &cs.shape {
                Shape::Text(t) if !t.galley.text().is_empty() => Some(t.galley.text().to_string()),
                _ => None,
            })
            .collect()
    }
    fn has(&self, label: &str) -> bool {
        self.texts().iter().any(|t| t == label)
    }
    /// Press + release on the painted `label` (opens a submenu, or fires a row).
    fn click(&mut self, label: &str, draw: &mut dyn FnMut(&mut egui::Ui)) {
        let pos = self
            .shapes
            .iter()
            .find_map(|cs| match &cs.shape {
                Shape::Text(t) if t.galley.text() == label => Some(t.pos + vec2(4.0, 4.0)),
                _ => None,
            })
            .unwrap_or_else(|| panic!("'{label}' not painted: {:?}", self.texts()));
        let btn =
            |pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        self.run(vec![Event::PointerMoved(pos)], draw);
        self.run(vec![btn(true)], draw);
        self.run(vec![btn(false)], draw);
        self.run(vec![], draw);
        self.run(vec![], draw);
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

/// Snapping is not just for move/trim: the razor, the marker tool and marker drags land on the same
/// candidates as everything else. In/out points are used as the candidates here - the playhead has a
/// grab zone that would eat the clicks.
#[test]
fn every_tool_snaps() {
    let mut h = Harness::new();
    h.snap = true;
    h.project.in_point = Some(3.0);
    h.project.out_point = Some(5.0);
    let lanes = h.state.lanes_rect;
    let y = lanes.top() + 30.0;
    let off = lanes.left() + 3.0 * h.state.zoom + 4.0; // 4 px past the in point, inside the 8 px threshold

    // marker tool first: the razor's cut would put a trim handle over the spot we click
    h.tool = Tool::Marker;
    let p = pos2(off, y);
    h.press(p);
    h.release(p);
    h.frame(vec![]);
    assert_eq!(h.project.markers.len(), 1, "marker tool dropped nothing");
    assert!((h.project.markers[0].t - 3.0).abs() < 1e-9, "marker tool must snap: {}", h.project.markers[0].t);

    h.tool = Tool::Cut;
    h.press(p);
    h.release(p);
    h.frame(vec![]);
    assert_eq!(h.project.tracks[0].clips.len(), 2, "razor did not split");
    let cut = h.project.tracks[0].clips[1].start;
    assert!((cut - 3.0).abs() < 1e-9, "razor must snap to the in point: {cut}");

    // and dragging the ruler flag snaps too - over to the out point, the only candidate near the drop
    h.tool = Tool::Select;
    let y = lanes.top() - RULER_H + 4.0;
    assert!(h.drag(pos2(h.state.x_at(3.0) + 1.0, y), pos2(h.state.x_at(5.0) + 5.0, y)), "marker drag edits");
    assert!((h.project.markers[0].t - 5.0).abs() < 1e-9, "marker drag must snap: {}", h.project.markers[0].t);
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

/// An effect card dragged out of the Effects panel is reported against the clip under the pointer
/// (the app pushes it on that clip's stack); over empty lane space it is reported with no clip so
/// the app can say so instead of swallowing the gesture.
#[test]
fn headless_dnd_effect_targets_the_clip_under_the_pointer() {
    use crate::model::EffectKind;
    let mut h = Harness::new();
    let lanes = h.state.lanes_rect;
    let on_clip = pos2(lanes.left() + 200.0, lanes.top() + 30.0); // t = 5 s, inside the 0..10 clip
    h.press(pos2(10.0, 10.0));
    egui::DragAndDrop::set_payload(&h.ctx, DragPayload::Effect(EffectKind::Blur));
    h.frame(vec![Event::PointerMoved(on_clip)]);
    let hit = drop_on_clip(&h.state, &h.project, on_clip, 5.0).map(|(_, c)| c.id);
    assert_eq!(hit, Some(h.project.tracks[0].clips[0].id), "the drag highlights that clip");
    let r = h.release(on_clip);
    assert!(!r.edited, "the timeline changes nothing itself - the app adds the effect");
    assert_eq!(r.dropped_other.len(), 1);
    let (payload, t, ti) = &r.dropped_other[0];
    assert!(matches!(payload, DragPayload::Effect(EffectKind::Blur)));
    assert_eq!(*ti, Some(0));
    assert!(h.project.tracks[0].clips[0].contains(*t), "reported time is inside the clip: {t}");

    // past the end of the clip: still reported, but on nothing
    let empty = pos2(lanes.left() + 480.0, lanes.top() + 30.0); // t = 12 s
    h.press(pos2(10.0, 10.0));
    egui::DragAndDrop::set_payload(&h.ctx, DragPayload::Effect(EffectKind::Blur));
    h.frame(vec![Event::PointerMoved(empty)]);
    assert!(drop_on_clip(&h.state, &h.project, empty, 12.0).is_none(), "nothing to highlight");
    let r = h.release(empty);
    let (_, t, _) = &r.dropped_other[0];
    assert!(!h.project.tracks[0].clips[0].contains(*t));
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
    // one key at the grab time, edited in place for the rest of the gesture - not one per frame
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
fn headless_mini_graph_toggle_needs_keys_and_width() {
    let mut h = Harness::new();
    let lanes = h.state.lanes_rect;
    let vid = h.video_clip().id;
    // default zoom (40 px/s) makes the 10 s clip 400 px wide - well past MINI_GRAPH_MIN_W (120)
    let btn_center = |h: &Harness| {
        let right = h.state.x_at(h.video_clip().end());
        let top = lanes.top() + 1.0;
        pos2(right - MINI_GRAPH_PAD - MINI_GRAPH_BTN * 0.5, top + MINI_GRAPH_PAD + MINI_GRAPH_BTN * 0.5)
    };

    // no keyframes yet: a click where the icon would sit just selects the clip like anywhere else
    // on its body - proves nothing is drawn/interactive there regardless of zoom
    let p = btn_center(&h);
    h.press(p);
    h.release(p);
    h.frame(vec![]);
    assert!(h.state.mini_graph_open.is_empty(), "no icon without keyframes");
    // the harness's default video clip is linked to an audio clip, so a plain click selects the
    // whole link group (same as clicking anywhere else on its body) - not just `vid` alone.
    assert!(h.selection.contains(&vid), "click without keys falls through to the clip body");
    h.selection.clear();

    // key it: the same spot now toggles the mini graph instead of selecting
    h.project.tracks[0].clips[0].opacity.toggle_key(2.0);
    h.project.tracks[0].clips[0].opacity.toggle_key(5.0);
    h.frame(vec![]);
    let p = btn_center(&h);
    h.press(p);
    h.release(p);
    h.frame(vec![]);
    assert_eq!(h.state.mini_graph_open, vec![vid], "click opened the mini graph");
    assert!(h.selection.is_empty(), "the icon must win hit-testing over the clip body under it");

    // click again: closes it
    h.press(p);
    h.release(p);
    h.frame(vec![]);
    assert!(h.state.mini_graph_open.is_empty(), "second click closed it");
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
fn headless_resize_handle_drag_does_not_start_a_rubber_band() {
    let mut h = Harness::new();
    let lanes = h.state.lanes_rect;
    let before_h = h.project.tracks[0].height;
    let row_bottom = lanes.top() + before_h;
    // press inside track 0's HANDLE_H-tall resize strip, then drag down well past it: the
    // in-progress drag can leave the strip within a step or two, which used to also arm a rubber
    // band from the lane background underneath
    let from = pos2(h.state.x_at(2.0), row_bottom - 2.0);
    h.drag(from, from + vec2(0.0, 30.0));
    assert!(h.project.tracks[0].height > before_h, "the handle drag actually resized the track");
    assert!(h.state.band.is_none(), "a resize press must not leave a rubber band armed");
    assert!(h.selection.is_empty(), "a resize drag must not select clips underneath it");
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
    // ws:pro-timeline: bumped from 80 to 100 -- the sequence tab strip now reserves 20 px above the
    // ruler unconditionally, and the scrolled audio row needs that much more headroom to stay within
    // `lanes.bottom()` (still comfortably "past RULER_H", the scenario this test is pinning).
    h.state.scroll_y = 100.0;
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
    assert_eq!(prop_range(h.video_clip(), 3), Some((0.01, 20.0)), "Scale X");
    assert_eq!(prop_range(h.video_clip(), 7), Some((0.01, 100.0)), "Speed");
    assert_eq!(prop_range(h.audio_clip(), 2), Some((0.01, 100.0)), "Speed (audio)");
    h.project.tracks[0].clips[0].effects.push(Effect::new(EffectKind::Blur));
    let spec = h.video_clip().effects[0].specs()[0];
    assert_eq!(prop_range(h.video_clip(), 8), Some((spec.min, spec.max)), "first Blur param");
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

/// A small marker flag still wins a click over the clip body it sits on (registered after the body),
/// even with the Blade active; a Blade click elsewhere on the body - low on a tall row too - still cuts.
#[test]
fn headless_marker_click_wins_over_the_blade_body() {
    let mut h = Harness::new();
    h.project.tracks[0].height = 2.0 * MIN_TRACK_H + 8.0;
    h.tool = Tool::Cut;
    h.frame(vec![]);
    let rt = row_top(&h.state, &h.project, 0).unwrap();
    let th = h.project.tracks[0].height;
    let cid = h.video_clip().id;
    let mid = h.project.add_clip_marker(cid, 1.0, "beat").unwrap();
    h.frame(vec![]);
    let mp = pos2(h.state.x_at(1.0) + 1.0, rt + 4.0);
    h.press(mp);
    h.release(mp);
    h.frame(vec![]);
    assert_eq!(h.state.selected_marker, Some(mid), "the marker flag wins the click");
    assert_eq!(h.project.tracks[0].clips.len(), 1, "the marker click must not also cut");
    h.settle();
    let sp = pos2(h.state.x_at(3.0) + 1.0, rt + th * 0.7);
    h.press(sp);
    h.release(sp);
    h.frame(vec![]);
    assert_eq!(h.project.tracks[0].clips.len(), 2, "a Blade click elsewhere on the body cuts");
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

/// Transition bands select like clips: click replaces (and clears the clip selection), Ctrl+click
/// adds, clicking a clip clears them again, and the rubber band picks up the bands it crosses.
#[test]
fn transition_bands_select_like_clips() {
    use crate::model::TransitionKind;
    let mut h = Harness::new();
    let lanes = h.state.lanes_rect;
    h.project.split_at(4.0, None);
    h.project.split_at(7.0, None);
    let c2 = h.project.tracks[0].clips[1].id;
    let c3 = h.project.tracks[0].clips[2].id;
    let t1 = h.project.add_transition(c2, TransitionKind::CrossFade, 1.0).unwrap();
    let t2 = h.project.add_transition(c3, TransitionKind::CrossFade, 1.0).unwrap();
    h.selection = vec![c2];
    h.frame(vec![]);
    let band1 = pos2(h.state.x_at(4.0), lanes.top() + 30.0);
    let band2 = pos2(h.state.x_at(7.0), lanes.top() + 30.0);
    h.press(band1);
    h.release(band1);
    assert_eq!(h.sel_transitions, vec![t1]);
    assert!(h.selection.is_empty(), "selecting a transition clears the clip selection");
    // Ctrl+click adds the second, Ctrl+click again toggles it off
    h.press_m(band2, Modifiers::CTRL);
    h.release_m(band2, Modifiers::CTRL);
    assert_eq!(h.sel_transitions, vec![t1, t2]);
    h.press_m(band2, Modifiers::CTRL);
    h.release_m(band2, Modifiers::CTRL);
    assert_eq!(h.sel_transitions, vec![t1]);
    // clicking a clip clears the transition selection
    let clip = pos2(h.state.x_at(2.0), lanes.top() + 30.0);
    h.press(clip);
    h.release(clip);
    assert!(h.sel_transitions.is_empty(), "selecting a clip clears the transition selection");
    assert!(h.selection.contains(&h.project.tracks[0].clips[0].id));
    // a rubber band from empty space across both cuts picks up both transitions
    let from = pos2(h.state.x_at(11.0), lanes.top() + 30.0);
    h.drag(from, pos2(h.state.x_at(3.4), lanes.top() + 30.0));
    assert!(h.sel_transitions.contains(&t1) && h.sel_transitions.contains(&t2), "{:?}", h.sel_transitions);
    // deleting a transition drops it from the selection on the next frame
    h.project.remove_transition(t1);
    h.frame(vec![]);
    assert!(!h.sel_transitions.contains(&t1));
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

/// Right-clicking 2+ selected transitions offers "Change Type"/"Change Easing" quick-changes -
/// the timeline's fast path to what the Inspector's `transition_section` already bulk-edits.
/// Absolute-overwrite (a menu click IS the new value): every selected transition gets the picked
/// kind, in exactly one undo step for the whole bulk operation.
#[test]
fn transition_menu_bulk_changes_kind_with_one_undo() {
    let mut h = Harness::new();
    let lanes = h.state.lanes_rect;
    h.project.split_at(4.0, None);
    h.project.split_at(7.0, None);
    let c2 = h.project.tracks[0].clips[1].id;
    let c3 = h.project.tracks[0].clips[2].id;
    let t1 = h.project.add_transition(c2, TransitionKind::CrossFade, 1.0).unwrap();
    let t2 = h.project.add_transition(c3, TransitionKind::CrossFade, 1.0).unwrap();
    h.frame(vec![]);
    let band1 = pos2(h.state.x_at(4.0), lanes.top() + 30.0);
    let band2 = pos2(h.state.x_at(7.0), lanes.top() + 30.0);
    h.press(band1);
    h.release(band1);
    h.press_m(band2, Modifiers::CTRL);
    h.release_m(band2, Modifiers::CTRL);
    assert_eq!(h.sel_transitions, vec![t1, t2], "both bands selected before the right-click");

    let undos0 = h.undos;
    // right-click the already-selected band: opens the bulk menu, targeting both (sel_transitions
    // is only reset to a single id when the right-clicked band wasn't already part of it)
    let secondary = |pos: Pos2, pressed: bool| Event::PointerButton {
        pos,
        button: PointerButton::Secondary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    h.frame(vec![secondary(band1, true)]);
    h.frame(vec![]);
    h.frame(vec![secondary(band1, false)]);
    h.frame(vec![]);
    let change_type = h.painted_text("Change Type").expect("Change Type submenu button painted") + vec2(4.0, 4.0);
    h.press(change_type);
    h.release(change_type);
    let push = h.painted_text("Push").expect("Push kind listed in the Change Type submenu") + vec2(4.0, 4.0);
    h.press(push);
    let r = h.release(push);
    assert!(r.edited, "picking a kind from the menu edits the project");
    assert_eq!(h.undos - undos0, 1, "one undo for the whole bulk operation, not one per transition");
    let kind_of = |id: Id| h.project.tracks[0].transitions.iter().find(|t| t.id == id).unwrap().kind;
    assert_eq!(kind_of(t1), TransitionKind::Push);
    assert_eq!(kind_of(t2), TransitionKind::Push);
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

/// AUDIT FIX regression test: the modifier table's Body/Shift row ("click = add link group to
/// selection") had no matching code before this workstream - Shift+click was indistinguishable
/// from a plain click. Confirms the fix and that Ctrl/plain click paths are unaffected.
#[test]
fn shift_click_adds_link_group_without_clearing() {
    let mut h = Harness::new();
    h.project.tracks[1].clips.clear();
    h.project.tracks[0].clips.clear();
    h.project.tracks[0].clips.push(Clip::new(101, ClipKind::Video, "a", 0.0, 2.0));
    h.project.tracks[0].clips.push(Clip::new(102, ClipKind::Video, "b", 3.0, 2.0));
    h.frame(vec![]);
    let lanes = h.state.lanes_rect;
    let pa = pos2(h.state.x_at(1.0), lanes.top() + 30.0);
    let pb = pos2(h.state.x_at(4.0), lanes.top() + 30.0);
    h.press(pa);
    h.release(pa);
    h.frame(vec![]);
    assert_eq!(h.selection, vec![101], "plain click selects A alone");
    h.settle(); // clicks on different clips within 0.3s would merge into a double-click
    h.press_m(pb, Modifiers::SHIFT);
    h.release_m(pb, Modifiers::SHIFT);
    h.frame(vec![]);
    assert_eq!(h.selection, vec![101, 102], "Shift-click adds B without clearing A");
    h.settle();
    h.press_m(pb, Modifiers::CTRL);
    h.release_m(pb, Modifiers::CTRL);
    h.frame(vec![]);
    assert_eq!(h.selection, vec![101], "Ctrl-click still toggles B out, unchanged");
    h.settle();
    h.press_m(pb, Modifiers::SHIFT);
    h.release_m(pb, Modifiers::SHIFT);
    h.frame(vec![]);
    h.settle();
    h.press(pa);
    h.release(pa);
    h.frame(vec![]);
    assert_eq!(h.selection, vec![101], "plain click still collapses selection to the clicked clip");
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
    let texts = |audio: bool, open: &str| -> Vec<String> {
        let m = ClipMenu { audio, ..test_clip_menu(&p.labels, &p.buses, &[]) };
        let (mut act, mut tg, mut el) = (None, false, false);
        let mut draw = |ui: &mut egui::Ui| clip_menu(ui, &m, &mut act, &mut tg, &mut el);
        let mut probe = MenuProbe::new();
        probe.run(vec![], &mut draw);
        probe.click(open, &mut draw);
        probe.texts()
    };
    assert!(texts(false, "Add").iter().any(|s| s == "Add Mask to Selection"), "video clips keep Add Mask");
    assert!(!texts(true, "Add").iter().any(|s| s == "Add Mask to Selection"), "a mask means nothing on audio");
    assert!(texts(true, "Audio").iter().any(|s| s == "Bus"), "audio clips get the bus submenu");
    assert!(!texts(false, "Audio").iter().any(|s| s == "Bus"), "video clips get no bus routing");
}

/// A `ClipMenu` for a plain video clip (id 1) - tests override the fields they care about.
fn test_clip_menu<'a>(labels: &'a [Label], buses: &'a [crate::model::Bus], shared: &'a [EffectKind]) -> ClipMenu<'a> {
    ClipMenu {
        id: 1,
        container: false,
        audio: false,
        native_size: true,
        sequence: false,
        library_selected: None,
        graph_open: None,
        labels,
        buses,
        shared_effects: shared,
    }
}

/// Only an effect kind carried by 2+ of the given clips counts as "shared" (a clip's own duplicate
/// effects count once, and a clip missing entirely just doesn't contribute).
#[test]
fn shared_effect_kinds_needs_two_clips_with_the_same_kind() {
    let mut p = Project::new();
    let mut a = Clip::new(1, ClipKind::Video, "a", 0.0, 2.0);
    a.effects.push(Effect::new(EffectKind::Blur));
    a.effects.push(Effect::new(EffectKind::Blur)); // duplicate on one clip must still count once
    let mut b = Clip::new(2, ClipKind::Video, "b", 2.0, 2.0);
    b.effects.push(Effect::new(EffectKind::Blur));
    b.effects.push(Effect::new(EffectKind::Vignette)); // only on b - not shared
    let c = Clip::new(3, ClipKind::Video, "c", 4.0, 2.0); // no effects at all
    p.tracks[0].clips = vec![a, b, c];
    assert_eq!(shared_effect_kinds(&p, &[1, 2]), vec![EffectKind::Blur]);
    assert_eq!(shared_effect_kinds(&p, &[1, 2, 3]), vec![EffectKind::Blur], "clip 3 contributes nothing");
    assert!(shared_effect_kinds(&p, &[1]).is_empty(), "needs 2+ clips");
    assert!(shared_effect_kinds(&p, &[2, 3]).is_empty(), "Blur is only on one of these two");
}

/// The timeline clip menu's "Effects" quick-toggle only appears when the caller found a shared
/// kind, and lists a "Toggle <name>" entry per kind passed in.
#[test]
fn clip_menu_effects_submenu_only_shows_shared_kinds() {
    let p = Project::new();
    let texts = |shared: &[EffectKind]| -> Vec<String> {
        let m = test_clip_menu(&p.labels, &p.buses, shared);
        let (mut act, mut tg, mut el) = (None, false, false);
        let mut draw = |ui: &mut egui::Ui| clip_menu(ui, &m, &mut act, &mut tg, &mut el);
        let mut probe = MenuProbe::new();
        probe.run(vec![], &mut draw);
        probe.texts()
    };
    let none = texts(&[]);
    assert!(!none.iter().any(|s| s == "Effects"), "no shared kind: no Effects submenu: {none:?}");
    let shared = texts(&[EffectKind::Blur]);
    assert!(shared.iter().any(|s| s == "Effects"), "a shared kind offers the Effects submenu: {shared:?}");
}

/// Right-clicking 2+ selected video clips offers a "Transform" submenu; "Stretch to Screen" fits
/// every selected clip with a native size to the project canvas - each against its OWN asset's
/// native size, not a shared one - in exactly one undo step for the whole bulk operation.
#[test]
fn transform_menu_bulk_stretches_selection_with_one_undo() {
    let mut h = Harness::new();
    // second clip on the same row, a different (portrait) native size from the harness's own
    // 1280x720 asset, so the two clips must land on different scale_x/scale_y
    let aid2 = h.project.add_asset(Asset {
        id: 0,
        path: "C:/y.mp4".into(),
        kind: ClipKind::Video,
        duration: 10.0,
        width: 720,
        height: 1280,
        fps: 30.0,
        audio_streams: Vec::new(),
        codec: "h264".into(),
        folder: String::new(),
        tags: Vec::new(),
        label: 0,
        description: String::new(),
        rel_path: None,
        parent: None,
        range: None,
        effects: Vec::new(),
    });
    let id2 = h.project.insert_asset_clips(aid2, 11.0, Some(0))[0];
    let id1 = h.video_clip().id;
    h.project.clip_mut(id1).unwrap().x.value = 30.0; // pre-existing, different transforms
    h.project.clip_mut(id2).unwrap().scale.value = 2.0;
    h.frame(vec![]);

    let lanes = h.state.lanes_rect;
    let p1 = pos2(lanes.left() + 100.0, lanes.top() + 30.0); // inside clip 1 (0..10 s)
    let p2 = pos2(h.state.x_at(11.2), lanes.top() + 30.0); // inside clip 2 (11..21 s)
    h.press(p1);
    h.release(p1);
    // clear egui's double-click window before the second click: two single-clicks on different
    // clips shortly after one another (in wall-clock time, which the harness's `time` field drives)
    // must not be misread as one double-click on the second clip - that fires the double-click
    // handler's unconditional `selection = [clip]`, stomping the ctrl-click's additive result below.
    h.time += 1.0;
    h.press_m(p2, Modifiers::CTRL);
    h.release_m(p2, Modifiers::CTRL);
    assert!(h.selection.contains(&id1) && h.selection.contains(&id2), "both clips selected: {:?}", h.selection);

    let undos0 = h.undos;
    let secondary = |pos: Pos2, pressed: bool| Event::PointerButton {
        pos,
        button: PointerButton::Secondary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    h.frame(vec![secondary(p1, true)]);
    h.frame(vec![]);
    h.frame(vec![secondary(p1, false)]);
    h.frame(vec![]);
    let xf = h.painted_text("Transform").expect("Transform submenu button painted") + vec2(4.0, 4.0);
    h.press(xf);
    h.release(xf);
    let stretch = h.painted_text("Stretch to Screen").expect("Stretch to Screen listed") + vec2(4.0, 4.0);
    h.press(stretch);
    let r = h.release(stretch);
    assert!(r.edited, "picking Stretch to Screen edits the project");
    assert_eq!(h.undos - undos0, 1, "one undo for the whole bulk operation, not one per clip");

    let c1 = h.project.clip(id1).unwrap();
    let c2 = h.project.clip(id2).unwrap();
    assert_eq!(c1.x.value, 0.0, "transform reset along with the stretch");
    // 1280x720 (16:9) into the project's own 1920x1080 (16:9) canvas: same aspect, no stretch needed
    assert_eq!((c1.scale_x.value, c1.scale_y.value), (1.0, 1.0));
    // 720x1280 (9:16 portrait) into a 16:9 canvas is height-constrained
    assert_eq!(c2.scale.value, 1.0, "scale reset too");
    assert_eq!(c2.scale_y.value, 1.0);
    let expected_sx = (1920.0_f64 / 1080.0) / (720.0 / 1280.0);
    assert!((c2.scale_x.value - expected_sx).abs() < 1e-9, "scale_x {} vs {expected_sx}", c2.scale_x.value);
}

#[test]
fn headless_video_track_v_toggle_flips_muted() {
    let mut h = Harness::new();
    let lanes = h.state.lanes_rect;
    // V1 header row: the eye is the right-most button (18 wide, 4 in)
    let vb = pos2(lanes.left() - 13.0, lanes.top() + h.project.tracks[0].height * 0.5);
    assert!(!h.project.tracks[0].muted);
    h.press(vb);
    let r = h.release(vb);
    assert!(r.edited || h.frame(vec![]).edited, "V toggle marks edited");
    assert!(h.project.tracks[0].muted, "V toggle flips muted (visibility off)");
    assert_eq!(h.undos, 1);
}

// ---- ws:snap-engine ----

/// ws:timeline-surface: the clip body is one zone. With Select a click low on a tall row selects (no cut,
/// no split hairline) and a right-click there opens the clip menu (the old bottom-half split zone had no
/// menu at all); the Blade cuts anywhere on the body and shows its hairline first.
#[test]
fn clip_body_is_one_zone_blade_cuts_anywhere() {
    let mut h = Harness::new();
    h.project.tracks[0].height = 3.0 * MIN_TRACK_H;
    h.project.tracks[1].clips.clear(); // no audio volume line (also accent) in the paint list
    h.frame(vec![]);
    let pal = Palette::new(true, Color32::from_rgb(0, 120, 212));
    let top = h.state.lanes_rect.top();
    let th = h.project.tracks[0].height;
    let (mid, id) = ((h.video_clip().start + h.video_clip().end()) / 2.0, h.video_clip().id);
    let low = pos2(h.state.x_at(mid), top + th * 0.75);
    h.frame(vec![Event::PointerMoved(low)]);
    assert!(!h.has_line(pal.accent), "Select: no split hairline over the body");
    h.press(low);
    h.release(low);
    h.frame(vec![]);
    assert_eq!(h.project.tracks[0].clips.len(), 1, "Select never cuts");
    assert!(h.selection.contains(&id), "a click low on the body selects");
    h.settle();
    h.rclick(low);
    assert!(h.painted_text("Split at Playhead").is_some(), "a right-click low on a tall clip opens its menu");
    let esc = Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    };
    h.frame(vec![esc]);
    h.settle();
    h.tool = Tool::Cut;
    let high = pos2(h.state.x_at(mid), top + th * 0.25);
    h.frame(vec![Event::PointerMoved(high)]);
    h.frame(vec![]);
    assert!(h.has_line(pal.accent), "the Blade shows where it will cut");
    h.press(high);
    h.release(high);
    h.frame(vec![]);
    assert_eq!(h.project.tracks[0].clips.len(), 2, "the Blade cuts high on the body too");
}

/// Clicking a seam (two abutting clips' shared edge) selects an `EditPoint`: plain=Both,
/// Ctrl=Left (outgoing), Alt=Right (incoming). Nothing else on the timeline changes selection.
#[test]
fn seam_click_selects_edit_point_sides() {
    let mut h = Harness::new();
    h.project.tracks[1].clips.clear();
    h.project.tracks[0].clips.clear();
    h.project.tracks[0].clips.push(Clip::new(201, ClipKind::Video, "a", 0.0, 2.0));
    h.project.tracks[0].clips.push(Clip::new(202, ClipKind::Video, "b", 2.0, 2.0)); // abuts at t=2.0
    h.frame(vec![]);
    let lanes = h.state.lanes_rect;
    let sx = h.state.x_at(2.0);
    let sy = lanes.top() + 30.0;
    h.press(pos2(sx, sy));
    h.release(pos2(sx, sy));
    h.frame(vec![]);
    assert_eq!(h.state.edit_point.map(|e| e.side), Some(Side::Both), "plain click selects Both");
    h.press_m(pos2(sx, sy), Modifiers::CTRL);
    h.release_m(pos2(sx, sy), Modifiers::CTRL);
    h.frame(vec![]);
    assert_eq!(h.state.edit_point.map(|e| e.side), Some(Side::Left), "Ctrl-click selects Left");
    h.press_m(pos2(sx, sy), Modifiers::ALT);
    h.release_m(pos2(sx, sy), Modifiers::ALT);
    h.frame(vec![]);
    assert_eq!(h.state.edit_point.map(|e| e.side), Some(Side::Right), "Alt-click selects Right");
}

/// Ruler in/out handles are draggable (snapped), clamp in<=out, and push exactly one undo per
/// gesture only if the value actually changed.
#[test]
fn inout_handles_drag_and_clamp() {
    let mut h = Harness::new();
    h.project.in_point = Some(1.0);
    h.project.out_point = Some(8.0);
    h.frame(vec![]);
    let lanes = h.state.lanes_rect;
    let y = lanes.top() - RULER_H + (RULER_H - 3.0);
    // drag the in handle past the out point: clamps to out (8.0)
    let from = pos2(h.state.x_at(1.0), y);
    let to = pos2(h.state.x_at(20.0), y);
    let edited = h.drag(from, to);
    assert!(edited, "in-handle drag edits");
    assert_eq!(h.undos, 1, "exactly one undo for the whole gesture");
    assert_eq!(h.project.in_point, Some(8.0), "in clamps to out: {:?}", h.project.in_point);
    // releasing at the same spot (no movement) pushes no undo
    let from2 = pos2(h.state.x_at(8.0), y);
    let edited2 = h.drag(from2, from2);
    assert!(!edited2, "a released-in-place drag must not edit");
    assert_eq!(h.undos, 1, "no undo for an unchanged drag");
}

/// A middle-button drag on the lanes pans scroll_x/scroll_y without starting any Move/band
/// gesture and without touching the selection.
#[test]
fn middle_mouse_pans_without_selecting() {
    let mut h = Harness::new();
    let lanes = h.state.lanes_rect;
    // empty lane space (past the default 10 s clip at zoom 40, i.e. past x=400) so the lanes
    // background - not a clip body registered on top of it - wins hit-testing here
    let from = pos2(lanes.left() + 480.0, lanes.top() + 30.0);
    let to = from - vec2(80.0, 20.0);
    h.frame_m(vec![Event::PointerMoved(from)], Modifiers::NONE);
    h.frame_m(
        vec![Event::PointerButton {
            pos: from,
            button: PointerButton::Middle,
            pressed: true,
            modifiers: Modifiers::NONE,
        }],
        Modifiers::NONE,
    );
    let sx0 = h.state.scroll_x;
    h.frame_m(vec![Event::PointerMoved(to)], Modifiers::NONE);
    h.frame_m(
        vec![Event::PointerButton {
            pos: to,
            button: PointerButton::Middle,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
        Modifiers::NONE,
    );
    assert!(h.state.scroll_x > sx0, "middle drag must pan scroll_x: {sx0} -> {}", h.state.scroll_x);
    assert!(h.state.drag.is_none(), "middle drag must not start a Move/trim gesture");
    assert!(h.selection.is_empty(), "middle drag must not select anything");
}

/// An empty project (zero clips on every track) shows the dashed drop hint; it disappears once
/// any clip exists.
#[test]
fn empty_timeline_shows_hint() {
    let mut h = Harness::new();
    h.project.tracks[0].clips.clear();
    h.project.tracks[1].clips.clear();
    h.frame(vec![]);
    assert!(h.painted_text("Drop video").is_some(), "empty timeline must paint the drop hint");
    h.project.tracks[0].clips.push(Clip::new(301, ClipKind::Video, "a", 0.0, 2.0));
    h.frame(vec![]);
    assert!(h.painted_text("Drop video").is_none(), "hint must disappear once a clip exists");
}

/// Dragging a clip within threshold of a candidate paints exactly one accent-colour guide line at
/// the candidate's x; dragging with nothing in range paints none.
#[test]
fn snap_guide_line_paints_mid_drag() {
    let mut h = Harness::new();
    h.snap = true;
    h.project.tracks[1].clips.clear();
    h.project.tracks[0].clips.clear();
    h.project.tracks[0].clips.push(Clip::new(302, ClipKind::Video, "a", 0.0, 2.0)); // reference edge at t=2.0
    h.project.tracks[0].clips.push(Clip::new(303, ClipKind::Video, "b", 8.0, 2.0)); // clip being dragged
    h.frame(vec![]);
    let lanes = h.state.lanes_rect;
    let y = lanes.top() + 30.0;
    let from = pos2(h.state.x_at(8.5), y); // press 0.5s into clip 303 (both points stay on the 800pt-wide test screen at zoom 40)
                                           // drag left so clip 303's start (8 + dx) lands within threshold of clip 302's end (2.0)
    let to = pos2(h.state.x_at(2.55), y);
    h.press(from);
    for i in 1..=3 {
        let p = from + (to - from) * (i as f32 / 3.0);
        h.frame(vec![Event::PointerMoved(p)]);
    }
    // the guide reflects the PREVIOUS frame's snap result (paint runs before gestures::handle in
    // show()), so one more frame at the same pointer position lets it catch up
    h.frame(vec![]);
    let accent = Palette::new(true, Color32::from_rgb(0, 120, 212)).accent;
    let has_guide =
        h.shapes.iter().any(|cs| matches!(&cs.shape, Shape::LineSegment { stroke, .. } if stroke.color == accent));
    assert!(has_guide, "an accent guide line must paint while snapped mid-drag");
    h.release(to);
    h.frame(vec![]);

    // dragging with nothing in range paints no guide line
    let mut h2 = Harness::new();
    h2.snap = true;
    h2.project.tracks[1].clips.clear();
    h2.project.tracks[0].clips.clear();
    h2.project.tracks[0].clips.push(Clip::new(304, ClipKind::Video, "c", 0.0, 2.0));
    h2.project.tracks[0].clips.push(Clip::new(305, ClipKind::Video, "d", 8.0, 2.0));
    h2.frame(vec![]);
    let from2 = pos2(h2.state.x_at(8.5), y);
    let to2 = pos2(h2.state.x_at(5.0), y); // far from every candidate, still on-screen
    h2.press(from2);
    for i in 1..=3 {
        let p = from2 + (to2 - from2) * (i as f32 / 3.0);
        h2.frame(vec![Event::PointerMoved(p)]);
    }
    h2.frame(vec![]);
    let no_guide =
        !h2.shapes.iter().any(|cs| matches!(&cs.shape, Shape::LineSegment { stroke, .. } if stroke.color == accent));
    assert!(no_guide, "no guide line when nothing is within snap threshold");
    h2.release(to2);
}

/// Dragging a subtitle cue body and releasing at the press point pushes 0 undos; actually moving
/// it pushes exactly 1 and the new position is snapped.
#[test]
fn cue_body_drag_one_undo_only_if_moved() {
    use crate::model::Cue;
    let mut h = Harness::new();
    h.project.subtitles.push(Cue { id: 401, start: 2.0, end: 4.0, text: "hi".into() });
    h.frame(vec![]); // relayout: the subtitle lane now has height
    let lanes = h.state.lanes_rect;
    let y = lanes.top() - h.state.sub_h * 0.5;
    let cx = h.state.x_at(3.0); // inside the cue body [2, 4)

    // press and release with no movement at all: click_and_drag() widgets need actual movement to
    // recognize a drag at all, so this never even opens a CueDrag - the simplest "nothing changed"
    // case, and it must not edit or undo either.
    let r = h.press(pos2(cx, y));
    let r2 = h.release(pos2(cx, y));
    let r3 = h.frame(vec![]);
    assert!(!r.edited && !r2.edited && !r3.edited, "a non-moving cue press+release must not edit");
    assert_eq!(h.undos, 0, "no undo when the cue never moved");

    // now actually move it: exactly one undo
    h.press(pos2(cx, y));
    h.frame(vec![Event::PointerMoved(pos2(cx + 80.0, y))]); // +2s at zoom 40
    let r3 = h.release(pos2(cx + 80.0, y));
    let r4 = h.frame(vec![]);
    assert!(r3.edited || r4.edited, "moving the cue body must edit");
    assert_eq!(h.undos, 1, "exactly one undo for the move gesture");
    let cue = h.project.subtitles.iter().find(|q| q.id == 401).unwrap();
    assert!((cue.start - 4.0).abs() < 0.1, "cue moved to ~4.0: {}", cue.start);
}

/// Trimming a cue edge and releasing without moving it pushes 0 undos (previously pushed 1 at
/// drag start regardless of whether anything changed).
#[test]
fn cue_trim_undo_on_release_not_press() {
    use crate::model::Cue;
    let mut h = Harness::new();
    h.project.subtitles.push(Cue { id: 402, start: 2.0, end: 4.0, text: "hi".into() });
    h.frame(vec![]);
    let lanes = h.state.lanes_rect;
    let y = lanes.top() - h.state.sub_h * 0.5;
    let ex = h.state.x_at(4.0); // right edge, within the 6 pt trim handle
                                // the trim handle senses drag-only, so press-then-release-in-place still counts as a drag
                                // gesture with zero net movement - exactly the case the old code pushed a dead undo for
    h.press(pos2(ex, y));
    let r = h.release(pos2(ex, y));
    let r2 = h.frame(vec![]);
    assert!(!r.edited && !r2.edited, "an in-place trim release must not edit");
    assert_eq!(h.undos, 0, "no undo for a released-in-place trim (was 1, pushed at drag start)");

    // an actual trim still pushes exactly one undo, on release
    h.press(pos2(ex, y));
    h.frame(vec![Event::PointerMoved(pos2(ex + 40.0, y))]); // +1s
    let r3 = h.release(pos2(ex + 40.0, y));
    let r4 = h.frame(vec![]);
    assert!(r3.edited || r4.edited, "an actual trim must edit");
    assert_eq!(h.undos, 1);
}

// ---- ws:timeline-trim-gestures ----

/// Ctrl+edge arms RippleTrim: the live drag only updates the ghost delta (the project is untouched),
/// and release makes exactly one `ripple_trim` call plus one undo, shifting the downstream clip on the
/// ripple track by the same amount the edge moved.
#[test]
fn ripple_trim_ghosts_during_drag_applies_once_on_release() {
    let mut h = Harness::new();
    h.project.tracks[1].clips.clear();
    h.project.tracks[0].clips.clear();
    h.project.tracks[0].clips.push(Clip::new(101, ClipKind::Video, "a", 0.0, 5.0));
    h.project.tracks[0].clips.push(Clip::new(102, ClipKind::Video, "b", 7.0, 3.0)); // gap [5,7)
    h.frame(vec![]);
    let lanes = h.state.lanes_rect;
    let x_end = h.state.x_at(5.0);
    let from = pos2(x_end - 2.0, lanes.top() + 30.0); // clip a's end-edge zone; no seam nearby
    h.press_m(from, Modifiers::CTRL);
    let before = h.project.to_json();
    h.frame_m(vec![Event::PointerMoved(from + vec2(40.0, 0.0))], Modifiers::CTRL); // +1s
    assert_eq!(h.project.to_json(), before, "RippleTrim must not touch the model until release");
    assert!(
        matches!(&h.state.drag, Some(Drag { g: Gesture::RippleTrim { dt, .. }, .. }) if dt.abs() > 0.5),
        "the ghost delta must track the drag"
    );
    h.release_m(from + vec2(40.0, 0.0), Modifiers::CTRL);
    h.frame(vec![]);
    assert_eq!(h.undos, 1, "exactly one undo on release");
    let a = h.project.clip(101).unwrap();
    assert!((a.duration - 6.0).abs() < 0.1, "trimmed end by ~1s: duration {}", a.duration);
    let b = h.project.clip(102).unwrap();
    assert!((b.start - 8.0).abs() < 0.1, "downstream clip shifted by the same delta, ids/tracks stable: {}", b.start);
}

/// Alt+edge arms Roll: the shared cut moves, the two clips' combined duration is unchanged, and the
/// whole gesture is one undo.
#[test]
fn roll_moves_cut_keeps_length_one_undo() {
    let mut h = Harness::new();
    h.project.tracks[1].clips.clear();
    h.project.tracks[0].clips.clear();
    h.project.tracks[0].clips.push(Clip::new(101, ClipKind::Video, "a", 0.0, 5.0));
    h.project.tracks[0].clips.push(Clip::new(102, ClipKind::Video, "b", 5.0, 5.0)); // abuts at 5.0
    h.frame(vec![]);
    let lanes = h.state.lanes_rect;
    let x_end = h.state.x_at(5.0);
    // 5px inside clip a's end-edge zone (EDGE_W=6) but outside the seam's own +/-3px click strip
    let from = pos2(x_end - 5.0, lanes.top() + 30.0);
    let total0 = h.project.clip(101).unwrap().duration + h.project.clip(102).unwrap().duration;
    assert!(h.drag_m(from, from + vec2(40.0, 0.0), Modifiers::ALT), "Alt+edge drag must edit");
    assert_eq!(h.undos, 1, "one undo for the whole roll");
    let (a, b) = (h.project.clip(101).unwrap(), h.project.clip(102).unwrap());
    assert!((a.end() - 6.0).abs() < 0.1, "cut moved to ~6.0: {}", a.end());
    assert_eq!(a.end(), b.start, "clips stay abutting");
    assert!((a.duration + b.duration - total0).abs() < 1e-6, "combined length unchanged");
}

/// Alt+body-drag arms Slip: only `src_in` changes, the on-timeline start/duration are fixed.
#[test]
fn slip_shifts_src_in_only_rect_fixed() {
    let mut h = Harness::new();
    let aid = h.project.assets[0].id; // 10s source
    h.project.tracks[1].clips.clear();
    h.project.tracks[0].clips.clear();
    let mut c = Clip::new(101, ClipKind::Video, "a", 2.0, 4.0);
    c.asset = aid;
    c.src_in = 2.0; // room to slip either way: max_in = 10 - 4 = 6
    h.project.tracks[0].clips.push(c);
    h.frame(vec![]);
    let lanes = h.state.lanes_rect;
    let body = pos2(h.state.x_at(3.0), lanes.top() + 30.0); // inside the body, clear of both edges
    let (start0, dur0) = (h.project.clip(101).unwrap().start, h.project.clip(101).unwrap().duration);
    assert!(h.drag_m(body, body + vec2(40.0, 0.0), Modifiers::ALT), "Alt+body drag must edit");
    assert_eq!(h.undos, 1);
    let c = h.project.clip(101).unwrap();
    assert_eq!(c.start, start0, "on-timeline start is fixed");
    assert_eq!(c.duration, dur0, "on-timeline duration is fixed");
    assert!((c.src_in - 2.0).abs() > 0.1, "src_in actually moved: {}", c.src_in);
}

/// A bare Alt+click (no movement) selects only the pressed clip and arms no gesture; an Alt+press
/// that then crosses the drag threshold arms Slip.
#[test]
fn alt_click_selects_single_alt_drag_slips() {
    let mut h = Harness::new();
    let lanes = h.state.lanes_rect;
    let (vid, aud) = (h.video_clip().id, h.audio_clip().id);
    let p = pos2(lanes.left() + 100.0, lanes.top() + 30.0);
    h.press_m(p, Modifiers::ALT);
    assert!(h.state.drag.is_none(), "a bare Alt+click must not arm a gesture before the drag threshold");
    h.release_m(p, Modifiers::ALT);
    h.frame(vec![]);
    assert_eq!(h.selection, vec![vid], "Alt+click selects only the clicked clip, not its link group");
    assert!(!h.selection.contains(&aud));

    h.press_m(p, Modifiers::ALT);
    h.frame_m(vec![Event::PointerMoved(p + vec2(20.0, 0.0))], Modifiers::ALT);
    assert!(
        matches!(&h.state.drag, Some(Drag { g: Gesture::Slip { .. }, .. })),
        "Alt+drag past the threshold must arm Slip"
    );
    h.release_m(p + vec2(20.0, 0.0), Modifiers::ALT);
    h.frame(vec![]);
}

/// AUDIT FIX regression test: the drag-start pre-arm auto-select step ("an unselected clip becomes the
/// selection, Ctrl adds it") must not replay for the new gesture kinds whose Ctrl bit means something
/// else now (Slide=Ctrl+Alt, Segment=Ctrl+Shift, RippleTrim=Ctrl+edge) - only for gestures that still
/// mean plain ctrl-toggle Move/Trim.
#[test]
fn pre_arm_ctrl_toggle_skipped_for_new_gesture_kinds() {
    // each check gets a fresh Harness: Slide/Segment/plain-Move actually commit a live edit on
    // release (not Esc'd here), so reusing one project across checks would corrupt the geometry the
    // next check's press position depends on
    let fresh = || {
        let mut h = Harness::new();
        h.project.tracks[1].clips.clear();
        h.project.tracks[0].clips.clear();
        h.project.tracks[0].clips.push(Clip::new(101, ClipKind::Video, "a", 0.0, 5.0));
        h.project.tracks[0].clips.push(Clip::new(102, ClipKind::Video, "b", 5.0, 5.0)); // abuts, for the edge case
        h.frame(vec![]);
        h
    };
    let ctrl_alt = Modifiers { ctrl: true, alt: true, ..Modifiers::NONE };
    let ctrl_shift = Modifiers { ctrl: true, shift: true, ..Modifiers::NONE };
    for (label, mods) in [("Ctrl+Alt (Slide)", ctrl_alt), ("Ctrl+Shift (Segment)", ctrl_shift)] {
        let mut h = fresh();
        let body = pos2(h.state.x_at(2.0), h.state.lanes_rect.top() + 30.0);
        h.press_m(body, mods);
        h.frame_m(vec![Event::PointerMoved(body + vec2(20.0, 0.0))], mods);
        assert!(h.selection.is_empty(), "{label}: the old ctrl-toggle-select must not fire on an unselected clip");
        h.release_m(body + vec2(20.0, 0.0), mods);
        h.frame(vec![]);
    }
    // Ctrl+edge (RippleTrim): same gate, on an edge press this time
    let mut h = fresh();
    let lanes = h.state.lanes_rect;
    let x_end = h.state.x_at(5.0);
    let edge = pos2(x_end - 5.0, lanes.top() + 30.0);
    h.press_m(edge, Modifiers::CTRL);
    h.frame_m(vec![Event::PointerMoved(edge + vec2(20.0, 0.0))], Modifiers::CTRL);
    assert!(h.selection.is_empty(), "Ctrl+edge (RippleTrim): the old ctrl-toggle-select must not fire either");
    h.release_m(edge + vec2(20.0, 0.0), Modifiers::CTRL);
    h.frame(vec![]);

    // sanity: plain Ctrl+body (still means Move-with-ctrl-toggle) DOES still replay the old step
    let mut h = fresh();
    let body = pos2(h.state.x_at(2.0), h.state.lanes_rect.top() + 30.0);
    h.press_m(body, Modifiers::CTRL);
    h.frame_m(vec![Event::PointerMoved(body + vec2(20.0, 0.0))], Modifiers::CTRL);
    assert_eq!(h.selection, vec![101], "plain Ctrl (Move) must still replay the old toggle-select step");
    h.release_m(body + vec2(20.0, 0.0), Modifiers::CTRL);
    h.frame(vec![]);
}

/// Ctrl+Shift+body arms Segment: the live drag only updates the ghost delta (the project is
/// untouched), and release extracts the clip from its old span and splices it in at the new one,
/// atomically, as one undo.
#[test]
fn segment_drag_extracts_and_splices_ghost_then_release() {
    let mut h = Harness::new();
    h.project.tracks[1].clips.clear();
    h.project.tracks[0].clips.clear();
    h.project.tracks[0].clips.push(Clip::new(101, ClipKind::Video, "a", 0.0, 5.0));
    h.frame(vec![]);
    let lanes = h.state.lanes_rect;
    let body = pos2(h.state.x_at(2.0), lanes.top() + 30.0);
    let ctrl_shift = Modifiers { ctrl: true, shift: true, ..Modifiers::NONE };
    h.press_m(body, ctrl_shift);
    let before = h.project.to_json();
    h.frame_m(vec![Event::PointerMoved(body + vec2(80.0, 0.0))], ctrl_shift); // +2s
    assert_eq!(h.project.to_json(), before, "Segment must not touch the model until release");
    assert!(
        matches!(&h.state.drag, Some(Drag { g: Gesture::Segment { dt, .. }, .. }) if dt.abs() > 1.5),
        "the ghost delta tracks the drag"
    );
    h.release_m(body + vec2(80.0, 0.0), ctrl_shift);
    h.frame(vec![]);
    assert_eq!(h.undos, 1, "one undo for the whole segment move");
    let c = h.project.clip(101).unwrap();
    assert!((c.start - 2.0).abs() < 0.1, "clip moved to ~2.0s: {}", c.start);
    assert!((c.duration - 5.0).abs() < 1e-6, "duration preserved by the extract+splice");
}

/// If the destination becomes invalid before release (here: the track gets locked mid-drag), the
/// project matches `before` exactly and no undo is pushed - a half-applied extract must never survive.
#[test]
fn segment_drag_invalid_destination_rolls_back() {
    let mut h = Harness::new();
    h.project.tracks[1].clips.clear();
    h.project.tracks[0].clips.clear();
    h.project.tracks[0].clips.push(Clip::new(101, ClipKind::Video, "a", 0.0, 5.0));
    h.frame(vec![]);
    let lanes = h.state.lanes_rect;
    let body = pos2(h.state.x_at(2.0), lanes.top() + 30.0);
    let ctrl_shift = Modifiers { ctrl: true, shift: true, ..Modifiers::NONE };
    h.press_m(body, ctrl_shift);
    h.frame_m(vec![Event::PointerMoved(body + vec2(80.0, 0.0))], ctrl_shift);
    let before = h.project.to_json();
    h.project.tracks[0].locked = true; // the destination track becomes invalid mid-drag
    h.release_m(body + vec2(80.0, 0.0), ctrl_shift);
    h.frame(vec![]);
    assert_eq!(h.project.to_json(), before, "a refused segment move leaves the project exactly as `before`");
    assert_eq!(h.undos, 0, "a refused segment move pushes no undo");
}

/// Every one of this workstream's new model ops refuses (no-ops, project untouched) on a locked
/// track, matching the existing edge/body Trim/Move refusal.
#[test]
fn locked_track_refuses_every_new_gesture() {
    let mut h = Harness::new();
    h.project.tracks[1].clips.clear();
    h.project.tracks[0].clips.clear();
    h.project.tracks[0].locked = true;
    let aid = h.project.assets[0].id;
    let mut a = Clip::new(101, ClipKind::Video, "a", 0.0, 4.0);
    a.asset = aid;
    a.src_in = 2.0;
    h.project.tracks[0].clips.push(a);
    h.project.tracks[0].clips.push(Clip::new(102, ClipKind::Video, "b", 4.0, 4.0)); // abuts, for roll
    let before = h.project.to_json();
    assert!(!h.project.roll_edit(102, 5.0), "roll refuses on a locked track");
    assert!(!h.project.slip(&[101], 1.0), "slip refuses on a locked track");
    assert!(!h.project.slide(101, 1.0), "slide refuses on a locked track");
    assert!(!h.project.ripple_trim(101, false, 5.0, true), "ripple trim refuses on a locked track");
    assert!(
        !gestures::segment_move(&mut h.project, &[101], &[(0, 0.0, 4.0)], 2.0),
        "segment move refuses on a locked track"
    );
    assert!(!h.project.magnetic_move(&[101], 1.0, 0), "magnetic_move refuses on a locked track");
    assert_eq!(h.project.to_json(), before, "every refusal leaves the project untouched");
}

/// On a magnetic track, deleting a clip pulls the rest of that track left to close its own gap
/// (`gestures::delete_clips_magnetic`, wired to the keyboard Delete action in `ui::app::actions`); a
/// non-magnetic track keeps the plain leave-a-gap delete. A move blocked by an overlap on a magnetic
/// track shoves the neighbour out of the way (`Project::magnetic_move`) instead of refusing.
#[test]
fn magnetic_track_delete_closes_gap_move_shoves() {
    let mut h = Harness::new();
    h.project.tracks[1].clips.clear();
    h.project.tracks[0].clips.clear();
    h.project.tracks[0].magnetic = true;
    h.project.tracks[0].clips.push(Clip::new(301, ClipKind::Video, "a", 0.0, 2.0));
    h.project.tracks[0].clips.push(Clip::new(302, ClipKind::Video, "b", 2.0, 2.0)); // abuts a
    gestures::delete_clips_magnetic(&mut h.project, &[301], false);
    assert_eq!(h.project.tracks[0].clips.len(), 1, "a is gone");
    assert!(
        (h.project.tracks[0].clips[0].start).abs() < 1e-6,
        "b shifted left to close the gap a's removal made: {}",
        h.project.tracks[0].clips[0].start
    );

    h.project.tracks[0].clips.clear();
    h.project.tracks[0].magnetic = false;
    h.project.tracks[0].clips.push(Clip::new(303, ClipKind::Video, "a", 0.0, 2.0));
    h.project.tracks[0].clips.push(Clip::new(304, ClipKind::Video, "b", 2.0, 2.0)); // abuts a
    gestures::delete_clips_magnetic(&mut h.project, &[303], false);
    assert_eq!(h.project.tracks[0].clips.len(), 1);
    assert_eq!(h.project.tracks[0].clips[0].start, 2.0, "non-magnetic delete leaves the gap where a used to be");

    h.project.tracks[0].clips.clear();
    h.project.tracks[0].magnetic = true;
    h.project.tracks[0].clips.push(Clip::new(305, ClipKind::Video, "a", 0.0, 2.0));
    h.project.tracks[0].clips.push(Clip::new(306, ClipKind::Video, "b", 2.0, 2.0)); // abuts a
    assert!(!h.project.move_clips(&[305], 1.9, 0, None), "a plain move refuses the overlap");
    assert!(h.project.magnetic_move(&[305], 1.9, 0), "magnetic_move shoves b out of the way instead of refusing");
    assert!((h.project.clip(305).unwrap().start - 1.9).abs() < 1e-6);
    assert!(h.project.clip(306).unwrap().start >= 3.9, "b was shoved clear: {}", h.project.clip(306).unwrap().start);
}

/// A plain click on empty lane space selects the gap under it (hatched); Delete closes it via
/// `close_gap_at`, which only shifts clips on ripple-flagged tracks - a non-ripple track's own gap is
/// left exactly where it was.
#[test]
fn gap_click_selects_and_delete_closes_on_ripple_tracks() {
    let mut h = Harness::new();
    h.project.tracks[1].clips.clear();
    h.project.tracks[0].clips.clear();
    h.project.tracks[0].ripple = Some(true);
    h.project.tracks[0].clips.push(Clip::new(101, ClipKind::Video, "a", 0.0, 2.0));
    h.project.tracks[0].clips.push(Clip::new(102, ClipKind::Video, "b", 5.0, 2.0)); // gap [2,5)
    h.frame(vec![]);
    let lanes = h.state.lanes_rect;
    let gap_pt = pos2(h.state.x_at(3.5), lanes.top() + 30.0);
    let del = || Event::Key {
        key: egui::Key::Delete,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    };
    h.press(gap_pt);
    h.release(gap_pt);
    h.frame(vec![]);
    assert_eq!(h.state.gap_sel, Some((0, 2.0, 5.0)), "a plain click on empty lane space selects the gap");
    h.frame(vec![del()]);
    assert_eq!(h.project.tracks[0].clips.len(), 2, "close_gap_at shifts clips, it doesn't remove any");
    assert!(
        (h.project.tracks[0].clips[1].start - 2.0).abs() < 1e-6,
        "Delete closed the gap on the ripple track: b shifted to {}",
        h.project.tracks[0].clips[1].start
    );
    assert!(h.state.gap_sel.is_none(), "the gap selection clears once it's closed");

    h.project.tracks[0].ripple = Some(false);
    h.project.tracks[0].clips.clear();
    h.project.tracks[0].clips.push(Clip::new(103, ClipKind::Video, "a", 0.0, 2.0));
    h.project.tracks[0].clips.push(Clip::new(104, ClipKind::Video, "b", 5.0, 2.0));
    h.frame(vec![]);
    h.press(gap_pt);
    h.release(gap_pt);
    h.frame(vec![]);
    assert_eq!(h.state.gap_sel, Some((0, 2.0, 5.0)));
    h.frame(vec![del()]);
    assert_eq!(h.project.tracks[0].clips[1].start, 5.0, "a non-ripple track's own gap is left untouched");
}

/// `drop_mode_for`'s modifier mapping (arm()'s Drop zone) actually reaches the drop: Ctrl splices
/// (ripple-opens room), Alt overwrites, Shift places on a fresh track on top.
#[test]
fn ctrl_drop_inserts_alt_drop_overwrites_shift_places_on_top() {
    let mut h = Harness::new();
    let lanes = h.state.lanes_rect;
    let aid = h.project.assets[0].id;
    let drop = pos2(lanes.left() + 480.0, lanes.top() + 30.0); // t = 12s, past the 10s clip

    let before_len = h.project.tracks[0].clips.len();
    h.press(pos2(10.0, 10.0));
    egui::DragAndDrop::set_payload(&h.ctx, DragPayload::Asset(aid));
    h.frame(vec![Event::PointerMoved(drop)]);
    let r = h.release_m(drop, Modifiers::CTRL);
    assert!(r.edited, "Ctrl-drop must edit");
    assert_eq!(h.project.tracks[0].clips.len(), before_len + 1, "Ctrl-drop spliced a new clip in");

    h.press(pos2(10.0, 10.0));
    egui::DragAndDrop::set_payload(&h.ctx, DragPayload::Asset(aid));
    h.frame(vec![Event::PointerMoved(drop)]);
    let r = h.release_m(drop, Modifiers::ALT);
    assert!(r.edited, "Alt-drop (overwrite) must edit");

    let before_tracks = h.project.tracks.len();
    h.press(pos2(10.0, 10.0));
    egui::DragAndDrop::set_payload(&h.ctx, DragPayload::Asset(aid));
    h.frame(vec![Event::PointerMoved(drop)]);
    let r = h.release_m(drop, Modifiers::SHIFT);
    assert!(r.edited, "Shift-drop (place on top) must edit");
    assert!(h.project.tracks.len() > before_tracks, "Shift-drop adds a fresh track");
}

/// Dropping an audio-only asset over an audio track row lands the clip on that row (was: only a
/// Video-kind row under the pointer was ever passed through; other rows always got `None`).
#[test]
fn audio_drop_infers_pointer_audio_row() {
    let mut h = Harness::new();
    let lanes = h.state.lanes_rect;
    let aid = h.project.add_asset(Asset {
        id: 0,
        path: "C:/x.mp3".into(),
        kind: ClipKind::Audio,
        duration: 4.0,
        width: 0,
        height: 0,
        fps: 0.0,
        audio_streams: vec![AudioStreamInfo { channels: 2, sample_rate: 48000, ..Default::default() }],
        codec: "aac".into(),
        folder: String::new(),
        tags: Vec::new(),
        label: 0,
        description: String::new(),
        rel_path: None,
        parent: None,
        range: None,
        effects: Vec::new(),
    });
    let a1_y = lanes.top() + h.project.tracks[0].height + 20.0; // inside the A1 row
    let drop = pos2(lanes.left() + 480.0, a1_y);
    h.press(pos2(10.0, 10.0));
    egui::DragAndDrop::set_payload(&h.ctx, DragPayload::Asset(aid));
    h.frame(vec![Event::PointerMoved(drop)]);
    let r = h.release(drop);
    assert!(r.edited);
    assert_eq!(h.project.tracks[1].clips.len(), 2, "the audio asset landed on the audio row under the pointer");
}

/// Ctrl-splice and Alt-overwrite must honour the audio row under the pointer too, not just the
/// no-modifier default drop above (was: `Act::DropAsset`'s Splice/Overwrite arms called
/// `splice_in`/`overwrite_asset` with only a video-track hint, so an audio-only asset always landed
/// on the model's default audio track regardless of which row it was dropped on).
#[test]
fn ctrl_splice_and_alt_overwrite_drop_onto_pointer_audio_row() {
    let mut h = Harness::new();
    let lanes = h.state.lanes_rect;
    let a2 = h.project.add_track(TrackKind::Audio); // non-default audio track, A1 already exists
    let aid = h.project.add_asset(Asset {
        id: 0,
        path: "C:/y.mp3".into(),
        kind: ClipKind::Audio,
        duration: 4.0,
        width: 0,
        height: 0,
        fps: 0.0,
        audio_streams: vec![AudioStreamInfo { channels: 2, sample_rate: 48000, ..Default::default() }],
        codec: "aac".into(),
        folder: String::new(),
        tags: Vec::new(),
        label: 0,
        description: String::new(),
        rel_path: None,
        parent: None,
        range: None,
        effects: Vec::new(),
    });
    let a2_top = row_top(&h.state, &h.project, a2).unwrap();
    let drop = pos2(lanes.left() + 480.0, a2_top + 20.0); // inside the A2 row

    h.press(pos2(10.0, 10.0));
    egui::DragAndDrop::set_payload(&h.ctx, DragPayload::Asset(aid));
    h.frame(vec![Event::PointerMoved(drop)]);
    let r = h.release_m(drop, Modifiers::CTRL);
    assert!(r.edited, "Ctrl-drop (splice) must edit");
    assert_eq!(h.project.tracks[a2].clips.len(), 1, "splice landed on A2, the row under the pointer");
    assert_eq!(h.project.tracks[1].clips.len(), 1, "A1 (the model's default audio track) is untouched");

    h.project.tracks[a2].clips.clear();
    h.press(pos2(10.0, 10.0));
    egui::DragAndDrop::set_payload(&h.ctx, DragPayload::Asset(aid));
    h.frame(vec![Event::PointerMoved(drop)]);
    let r = h.release_m(drop, Modifiers::ALT);
    assert!(r.edited, "Alt-drop (overwrite) must edit");
    assert_eq!(h.project.tracks[a2].clips.len(), 1, "overwrite landed on A2, the row under the pointer");
    assert_eq!(h.project.tracks[1].clips.len(), 1, "A1 (the model's default audio track) is untouched");
}

/// Header Lock (its glyph) and Ripple (the header's right-click) are undo-free track state via the
/// `track_toggle` deferred field - zero undo growth, unlike every `Act` variant (e.g. Mute/Solo), which
/// pushes one unconditionally. A locked lane also paints the hatch treatment.
#[test]
fn header_lock_ripple_toggle_zero_undo_and_hatch() {
    let mut h = Harness::new();
    let lanes = h.state.lanes_rect;
    let row_center_y = lanes.top() + h.project.tracks[0].height * 0.5;
    // lock sits left of the eye (see header.rs: mb, lb)
    let lock_btn = pos2(lanes.left() - 34.0, row_center_y);

    assert!(!h.project.tracks[0].locked);
    h.press(lock_btn);
    h.release(lock_btn);
    h.frame(vec![]);
    assert!(h.project.tracks[0].locked, "Lock toggled on");
    assert_eq!(h.undos, 0, "the header Lock toggle must push no undo");
    let pal = Palette::new(true, Color32::from_rgb(0, 120, 212));
    assert!(h.has_line(pal.text_dim.gamma_multiply(0.25)), "a locked lane paints the hatch");

    h.settle();
    h.press(lock_btn);
    h.release(lock_btn);
    h.frame(vec![]);
    assert!(!h.project.tracks[0].locked, "Lock toggled back off");
    assert_eq!(h.undos, 0);
    assert!(!h.has_line(pal.text_dim.gamma_multiply(0.25)), "an unlocked lane paints no hatch");

    let ripple0 = h.project.tracks[0].ripple;
    h.settle();
    h.rclick(pos2(lanes.left() - 100.0, row_center_y));
    h.click_text("Ripple (Sync)");
    assert_ne!(h.project.tracks[0].ripple, ripple0, "Ripple toggled from the header menu");
    assert_eq!(h.undos, 0, "the header Ripple toggle must push no undo");
}

/// Esc during any of this workstream's new gestures - the live-mutating ones (Roll) and the
/// ghost-only, release-applied ones (RippleTrim, Segment) - restores the project exactly and pushes
/// no undo, same as the pre-existing gestures.
#[test]
fn esc_restores_before_zero_undos_for_new_gestures() {
    let esc = || Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    };

    // Roll (live-mutating): Alt+edge
    let mut h = Harness::new();
    h.project.tracks[1].clips.clear();
    h.project.tracks[0].clips.clear();
    h.project.tracks[0].clips.push(Clip::new(101, ClipKind::Video, "a", 0.0, 5.0));
    h.project.tracks[0].clips.push(Clip::new(102, ClipKind::Video, "b", 5.0, 5.0));
    h.frame(vec![]);
    let lanes = h.state.lanes_rect;
    let from = pos2(h.state.x_at(5.0) - 5.0, lanes.top() + 30.0);
    h.press_m(from, Modifiers::ALT);
    h.frame_m(vec![Event::PointerMoved(from + vec2(40.0, 0.0))], Modifiers::ALT);
    assert!((h.project.clip(101).unwrap().duration - 5.0).abs() > 0.1, "mid-drag: the roll already moved the cut");
    h.frame(vec![esc()]);
    h.release_m(from + vec2(40.0, 0.0), Modifiers::ALT);
    h.frame(vec![]);
    assert_eq!(h.project.clip(101).unwrap().duration, 5.0, "Esc restored the pre-roll project");
    assert!(h.state.drag.is_none());
    assert_eq!(h.undos, 0);

    // RippleTrim (ghost-only): Ctrl+edge
    let mut h = Harness::new();
    h.project.tracks[1].clips.clear();
    h.project.tracks[0].clips.clear();
    h.project.tracks[0].clips.push(Clip::new(201, ClipKind::Video, "a", 0.0, 5.0));
    h.project.tracks[0].clips.push(Clip::new(202, ClipKind::Video, "b", 7.0, 3.0));
    h.frame(vec![]);
    let lanes = h.state.lanes_rect;
    let from = pos2(h.state.x_at(5.0) - 2.0, lanes.top() + 30.0);
    h.press_m(from, Modifiers::CTRL);
    h.frame_m(vec![Event::PointerMoved(from + vec2(40.0, 0.0))], Modifiers::CTRL);
    h.frame(vec![esc()]);
    h.release_m(from + vec2(40.0, 0.0), Modifiers::CTRL);
    h.frame(vec![]);
    assert_eq!(h.project.clip(201).unwrap().duration, 5.0, "Esc restored the pre-trim project");
    assert_eq!(h.project.clip(202).unwrap().start, 7.0);
    assert!(h.state.drag.is_none());
    assert_eq!(h.undos, 0);

    // Segment (ghost-only): Ctrl+Shift+body
    let mut h = Harness::new();
    h.project.tracks[1].clips.clear();
    h.project.tracks[0].clips.clear();
    h.project.tracks[0].clips.push(Clip::new(301, ClipKind::Video, "a", 0.0, 5.0));
    h.frame(vec![]);
    let lanes = h.state.lanes_rect;
    let body = pos2(h.state.x_at(2.0), lanes.top() + 30.0);
    let ctrl_shift = Modifiers { ctrl: true, shift: true, ..Modifiers::NONE };
    h.press_m(body, ctrl_shift);
    h.frame_m(vec![Event::PointerMoved(body + vec2(80.0, 0.0))], ctrl_shift);
    h.frame(vec![esc()]);
    h.release_m(body + vec2(80.0, 0.0), ctrl_shift);
    h.frame(vec![]);
    assert_eq!(h.project.clip(301).unwrap().start, 0.0, "Esc restored the pre-segment project");
    assert!(h.state.drag.is_none());
    assert_eq!(h.undos, 0);
}

/// 30 idle frames (no input) with the pointer resting over the header Lock button of a locked (thus
/// hatched) track must request no repaint.
#[test]
fn assert_no_idle_repaint_timeline_header() {
    let mut h = Harness::new();
    h.project.tracks[0].locked = true;
    let lanes = h.state.lanes_rect;
    let row_center_y = lanes.top() + h.project.tracks[0].height * 0.5;
    let lock_btn = pos2(lanes.left() - 34.0, row_center_y);
    h.frame(vec![Event::PointerMoved(lock_btn)]);
    for _ in 0..30 {
        h.frame(vec![]);
    }
    assert!(
        !h.ctx.has_requested_repaint(),
        "idle frame over the header Lock button / hatched lane requested a repaint"
    );
}

/// The clip menu's rows dispatch what they say: an Action row queues its Action (the menu bar's path -
/// Split / Delete are those Actions now, not bespoke `Act`s); "Un-nest" and "Replace with Library
/// Selection" keep the clicked clip's `Act`, and are greyed (no dispatch) when they can't apply.
#[test]
fn clip_menu_rows_dispatch_actions_and_acts() {
    use crate::hotkeys::Action;
    let p = Project::new();
    let run = |m: &ClipMenu, path: &[&str]| -> (bool, Option<Id>, Vec<Action>) {
        let (mut act, mut tg, mut el) = (None, false, false);
        {
            let mut draw = |ui: &mut egui::Ui| clip_menu(ui, m, &mut act, &mut tg, &mut el);
            let mut probe = MenuProbe::new();
            probe.run(vec![], &mut draw);
            for l in path {
                probe.click(l, &mut draw);
            }
        }
        let fired = act.is_some();
        let target = match act {
            Some(Act::ReplaceClip(id)) | Some(Act::Unnest(id)) => Some(id),
            _ => None,
        };
        (fired, target, crate::ui::menu::take_queued())
    };
    crate::ui::menu::take_queued();
    let base = test_clip_menu(&p.labels, &p.buses, &[]);
    assert_eq!(run(&base, &["Join Through Edit"]).2, vec![Action::JoinThroughEdit]);
    assert_eq!(run(&base, &["Split at Playhead"]).2, vec![Action::Split]);
    assert_eq!(run(&base, &["Delete (Leave Gap)"]).2, vec![Action::Delete]);
    assert_eq!(run(&base, &["Replace with Library Selection"]), (false, None, vec![]), "greyed: no Library pick");
    let lib = ClipMenu { library_selected: Some(7), ..test_clip_menu(&p.labels, &p.buses, &[]) };
    assert_eq!(run(&lib, &["Replace with Library Selection"]), (true, Some(1), vec![]), "the clicked clip's Act");
    let seq = ClipMenu { sequence: true, ..test_clip_menu(&p.labels, &p.buses, &[]) };
    assert_eq!(run(&seq, &["Nest", "Un-nest Sequence Clip"]), (true, Some(1), vec![]));
    assert_eq!(run(&base, &["Nest", "Un-nest Sequence Clip"]), (false, None, vec![]), "only a Sequence clip");
}

/// Extends `headless_1000_clips_stays_fast`: a live RippleTrim drag (ghost-paint only, zero model
/// calls per frame) over a 1000-clip timeline stays within the same per-frame budget.
#[test]
fn headless_1000_clip_ripple_trim_ghost_stays_fast() {
    let mut h = Harness::new();
    h.project = Project::new();
    for i in 0..1000usize {
        let mut c = Clip::new(i as Id + 1000, ClipKind::Video, "clip", i as f64 * 2.0, 1.8);
        c.label = (i % 8) as u8 + 1;
        h.project.tracks[0].clips.push(c);
    }
    h.project.tidy();
    h.frame(vec![]);
    let lanes = h.state.lanes_rect;
    h.state.zoom = 40.0;
    h.state.scroll_x = 0.0;
    let x_end = h.state.x_at(1.8); // end edge of the first clip
    let from = pos2(x_end - 2.0, lanes.top() + 30.0);
    h.press_m(from, Modifiers::CTRL);
    h.frame_m(vec![Event::PointerMoved(from + vec2(8.0, 0.0))], Modifiers::CTRL); // crosses the drag threshold
    assert!(matches!(&h.state.drag, Some(Drag { g: Gesture::RippleTrim { .. }, .. })), "RippleTrim armed");
    let t0 = std::time::Instant::now();
    for i in 1..=20 {
        let p = from + vec2(8.0 + i as f32, 0.0);
        h.frame_m(vec![Event::PointerMoved(p)], Modifiers::CTRL);
    }
    let ms = t0.elapsed().as_secs_f64() * 1000.0 / 20.0;
    println!("timeline: 1000 clips, live RippleTrim ghost: {ms:.2} ms/frame");
    assert!(ms < 10.0, "1000-clip RippleTrim ghost frame took {ms:.2} ms");
    h.release_m(from + vec2(28.0, 0.0), Modifiers::CTRL);
    h.frame(vec![]);
}

// ==================================================================================
// ws:pro-timeline
// ==================================================================================

#[test]
fn header_rename_commits_on_enter() {
    let mut h = Harness::new();
    h.state.track_rename = Some((0, "New Name".into()));
    h.frame(vec![Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }]);
    assert_eq!(h.project.tracks[0].name, "New Name");
    assert_eq!(h.undos, 1, "exactly one undo for the rename");
    assert!(h.state.track_rename.is_none(), "rename mode closes after commit");
}

/// The track colour is a stripe down the header's left edge (the old swatch box read as a checkbox),
/// picked from the header's right-click ▸ Colour.
#[test]
fn header_colour_is_a_left_stripe_picked_from_the_menu() {
    let mut h = Harness::new();
    let lanes = h.state.lanes_rect;
    h.rclick(pos2(lanes.left() - 100.0, lanes.top() + 20.0));
    h.click_text("Colour");
    let (name, color) = crate::model::LABEL_COLORS[2];
    h.click_text(name);
    h.frame(vec![]);
    assert_eq!(h.project.tracks[0].color, Some(color));
    let [r, g, b] = color;
    assert!(h.has_fill(Color32::from_rgb(r, g, b)), "the stripe is painted in the track colour");
    assert_eq!(h.undos, 1);
}

#[test]
fn header_drag_reorder_swaps_same_kind_tracks() {
    let mut h = Harness::new();
    h.project.add_track(TrackKind::Video); // V2 at index 1; row_order shows V2 above V1
    h.frame(vec![]);
    let (v1_id, v2_id) = (h.project.tracks[0].id, h.project.tracks[1].id);
    let lanes = h.state.lanes_rect;
    let header_left = lanes.left() - h.state.header_w;
    let v2_h = h.project.tracks[1].height;
    // V1 is the SECOND displayed row (V2 draws above it, per row_order's video reversal)
    let v1_row_top = lanes.top() + v2_h;
    let grip = pos2(header_left + 4.0, v1_row_top + h.project.tracks[0].height * 0.5);
    h.press(grip);
    h.frame(vec![Event::PointerMoved(pos2(header_left + 4.0, lanes.top() - 5.0))]); // dragged up, above V2's row
    let resp = h.release(pos2(header_left + 4.0, lanes.top() - 5.0));
    assert!(resp.edited);
    h.frame(vec![]);
    assert_eq!(h.project.tracks[0].id, v2_id, "V2 swapped into slot 0");
    assert_eq!(h.project.tracks[1].id, v1_id, "V1 (dragged up, past V2) swapped into slot 1");
    assert_eq!(h.undos, 1);
}

#[test]
fn view_preset_toggles_paint_calls() {
    let mut h = Harness::new();
    let name = h.video_clip().name.clone();
    h.frame(vec![]);
    let geom_before = (h.state.x_at(h.video_clip().start), h.state.x_at(h.video_clip().end()));
    assert!(h.painted_text(&name).is_some(), "clip_text=true (the harness default) paints the clip name");
    h.view.clip_text = false;
    h.frame(vec![]);
    assert!(h.painted_text(&name).is_none(), "clip_text=false suppresses the name");
    let geom_after = (h.state.x_at(h.video_clip().start), h.state.x_at(h.video_clip().end()));
    assert_eq!(geom_before, geom_after, "toggling a view flag must not move the clip");
    h.view.clip_text = true;
    h.frame(vec![]);
    assert!(h.painted_text(&name).is_some(), "clip_text=true again re-paints the name");
}

#[test]
fn overview_strip_paints_only_when_enabled() {
    let mut h = Harness::new();
    h.overview = false;
    h.frame(vec![]);
    let lanes_off = h.state.lanes_rect;
    h.overview = true;
    h.frame(vec![]);
    let lanes_on = h.state.lanes_rect;
    assert!(lanes_on.top() > lanes_off.top(), "the overview strip eats vertical space above the ruler when on");
    // clicking inside the overview strip (still on, above the now-lower ruler) seeks
    let pos = pos2(h.state.header_w + 50.0, lanes_off.top() + 4.0);
    h.press(pos);
    let resp = h.release(pos);
    assert!(resp.seeked, "clicking the overview strip seeks the playhead");
}

/// The inner "Main" strip is gone (the Timeline pane's own tab carries the sequence), so the ruler sits
/// at the very top of the widget.
#[test]
fn no_inner_sequence_strip() {
    let mut h = Harness::new();
    let seq_id = h.project.new_sequence("Seq A", 1920, 1080, 30.0);
    assert!(h.project.open_sequence(seq_id));
    h.frame(vec![]);
    assert!(h.painted_text("Main").is_none() && h.painted_text("Seq A").is_none(), "{:?}", h.texts());
    let inset = h.state.lanes_rect.left() - h.state.header_w; // CentralPanel's margin, same on every side
    assert_eq!(h.state.lanes_rect.top(), inset + RULER_H, "nothing above the ruler");
}

/// Match Frame / Reveal in Library are on every clip menu; the menu is grouped into submenus rather than
/// ~32 flat rows; and the mislabeled "Convert to Adjustment Layer" (it ADDED a new layer) is gone.
#[test]
fn clip_menu_is_grouped_without_the_adjustment_row() {
    let p = Project::new();
    let m = test_clip_menu(&p.labels, &p.buses, &[]);
    let (mut act, mut tg, mut el) = (None, false, false);
    let mut draw = |ui: &mut egui::Ui| clip_menu(ui, &m, &mut act, &mut tg, &mut el);
    let mut probe = MenuProbe::new();
    probe.run(vec![], &mut draw);
    assert!(probe.has("Match Frame") && probe.has("Reveal in Library"), "{:?}", probe.texts());
    let rows = probe.texts().iter().filter(|t| t.chars().count() > 1).count(); // not the icon glyphs
    assert!(rows <= 24, "grouped into submenus, not ~32 flat rows: {rows}");
    assert!(!probe.texts().iter().any(|t| t.contains("Adjustment")));
    probe.click("Add", &mut draw);
    assert!(probe.has("Add Transition at Clip End"));
    assert!(!probe.texts().iter().any(|t| t.contains("Adjustment")), "not under Add ▸ either");
}

/// The track under the pointer (lane or header) is what the track Actions target; nothing is under the
/// pointer over the ruler.
#[test]
fn hover_track_follows_the_pointer() {
    let mut h = Harness::new();
    let lanes = h.state.lanes_rect;
    let a1_y = lanes.top() + h.project.tracks[0].height + 10.0;
    h.frame(vec![Event::PointerMoved(pos2(lanes.left() + 50.0, lanes.top() + 10.0))]);
    assert_eq!(h.state.hover_track, Some(0), "over V1's lane");
    h.frame(vec![Event::PointerMoved(pos2(lanes.left() - 60.0, a1_y))]);
    assert_eq!(h.state.hover_track, Some(1), "over A1's header");
    h.frame(vec![Event::PointerMoved(pos2(lanes.left() + 50.0, lanes.top() - 5.0))]);
    assert_eq!(h.state.hover_track, None, "the ruler is no track");
}

/// A header shows Solo only while hovered - or while that track is soloed, so the state never hides.
#[test]
fn solo_shows_on_hover_or_while_on() {
    let mut h = Harness::new();
    let lanes = h.state.lanes_rect;
    let solos = |h: &Harness| h.texts().iter().filter(|(t, _)| t == "S").count();
    let lane = pos2(lanes.right() - 20.0, lanes.top() + 10.0);
    h.frame(vec![Event::PointerMoved(lane)]);
    assert_eq!(solos(&h), 0, "no Solo buttons at rest");
    h.frame(vec![Event::PointerMoved(pos2(lanes.left() - 100.0, lanes.top() + 10.0))]);
    assert_eq!(solos(&h), 1, "hovering a header shows its Solo");
    h.project.tracks[1].solo = true;
    h.frame(vec![Event::PointerMoved(lane)]);
    assert_eq!(solos(&h), 1, "a soloed track keeps its S without hover");
}

/// Right-clicking a gap selects it (hatched, like a click) and its menu's Close Gap runs the same
/// `close_gap_at` Delete does.
#[test]
fn gap_right_click_selects_it_and_close_gap_closes_it() {
    let mut h = Harness::new();
    h.project.tracks[1].clips.clear();
    h.project.tracks[0].clips.clear();
    h.project.tracks[0].ripple = Some(true);
    h.project.tracks[0].clips.push(Clip::new(401, ClipKind::Video, "a", 0.0, 2.0));
    h.project.tracks[0].clips.push(Clip::new(402, ClipKind::Video, "b", 5.0, 2.0));
    h.frame(vec![]);
    h.rclick(pos2(h.state.x_at(3.5), h.state.lanes_rect.top() + 30.0));
    assert_eq!(h.state.gap_sel, Some((0, 2.0, 5.0)), "a right-click selects the gap");
    assert!(h.click_text("Close Gap").edited);
    assert!((h.project.clip(402).unwrap().start - 2.0).abs() < 1e-6, "the next clip pulled left");
}

/// A seam's right-click arms that edit point (both sides) and selects its incoming clip, which is what
/// its two rows act on: Roll to Playhead (ExtendEdit) and Add Transition at Selected Cut.
#[test]
fn seam_right_click_arms_the_edit_point_and_offers_roll_and_transition() {
    use crate::hotkeys::Action;
    let mut h = Harness::new();
    h.project.tracks[1].clips.clear();
    h.project.tracks[0].clips.clear();
    h.project.tracks[0].clips.push(Clip::new(201, ClipKind::Video, "a", 0.0, 2.0));
    h.project.tracks[0].clips.push(Clip::new(202, ClipKind::Video, "b", 2.0, 2.0));
    h.frame(vec![]);
    let seam = pos2(h.state.x_at(2.0), h.state.lanes_rect.top() + 30.0);
    h.rclick(seam);
    assert_eq!(h.state.edit_point.map(|e| (e.t, e.side)), Some((2.0, Side::Both)));
    assert_eq!(h.selection, vec![202], "the incoming clip: AddTransition's cut is on its left");
    assert!(h.click_text("Roll Edit to Playhead").actions.contains(&Action::ExtendEdit));
    h.settle();
    h.rclick(seam);
    crate::ui::menu::take_queued();
    h.click_text("Add Transition at Selected Cut");
    assert_eq!(crate::ui::menu::take_queued(), vec![Action::AddTransition]);
}

/// The ruler's View ▸ holds what used to be a toolbar row: the view presets (radio) and the overview.
#[test]
fn ruler_view_menu_picks_a_preset_and_toggles_overview() {
    use crate::hotkeys::Action;
    let mut h = Harness::new();
    let ruler = pos2(h.state.lanes_rect.left() + 200.0, h.state.lanes_rect.top() - 10.0);
    h.rclick(ruler);
    h.click_text("View");
    h.click_text("Compact");
    assert_eq!(h.state.view_idx, 1, "a preset pick switches the view");
    h.settle();
    h.rclick(ruler);
    h.click_text("View");
    assert!(h.click_text("Overview Strip").actions.contains(&Action::ToggleOverview));
}

/// The subtitle lane's empty area has its own menu: add a cue at the playhead, or import a file.
#[test]
fn subtitle_lane_menu_offers_add_and_import() {
    let mut h = Harness::new();
    h.project.add_cue(5.0, 6.0, "hi");
    h.frame(vec![]);
    h.rclick(pos2(h.state.x_at(1.0), h.state.lanes_rect.top() - SUB_LANE_H * 0.5));
    assert!(h.painted_text("Add Subtitle at Playhead").is_some(), "{:?}", h.texts());
    assert!(h.click_text("Import Subtitles…").import_subtitles);
}

#[test]
fn dupes_group_by_asset_and_src_range() {
    let mut p = Project::new();
    let aid = 42;
    let mut a = Clip::new(1, ClipKind::Video, "a", 0.0, 2.0);
    (a.asset, a.src_in) = (aid, 1.0); // source range [1,3)
    let mut b = Clip::new(2, ClipKind::Video, "b", 5.0, 2.0);
    (b.asset, b.src_in) = (aid, 1.0); // same range [1,3) -- dupe of a
    let mut c = Clip::new(3, ClipKind::Video, "c", 8.0, 2.0);
    (c.asset, c.src_in) = (aid, 9.0); // range [9,11) -- singleton, excluded
    p.tracks[0].clips = vec![a, b, c];
    let groups = dupe_groups(&p);
    assert_eq!(groups.len(), 1, "exactly one dupe group (the singleton is excluded)");
    let mut g = groups[0].clone();
    g.sort();
    assert_eq!(g, vec![1, 2]);
}

#[test]
fn pacing_bands_match_thresholds() {
    let mut p = Project::new();
    let short = Clip::new(1, ClipKind::Video, "s", 0.0, 0.5); // shorter than 1.5
    let ok = Clip::new(2, ClipKind::Video, "ok", 1.0, 5.0); // inside [1.5, 20.0]
    let long = Clip::new(3, ClipKind::Video, "l", 10.0, 25.0); // longer than 20.0
    p.tracks[0].clips = vec![short, ok, long];
    let flagged: Vec<Id> = pacing_spans(&p, (1.5, 20.0)).into_iter().map(|(id, ..)| id).collect();
    assert!(flagged.contains(&1) && flagged.contains(&3), "too-short and too-long clips are flagged");
    assert!(!flagged.contains(&2), "a clip inside the range is not flagged");
}

#[test]
fn realtime_bar_skips_effect_free_seconds() {
    let mut h = Harness::new();
    let pal = Palette::new(true, Color32::from_rgb(0, 120, 212));
    h.realtime = vec![(0.0, 2.0, true, true)]; // heavy but already ready -- no tint
    h.frame(vec![]);
    assert!(!h.has_fill(pal.playhead), "a ready segment gets no danger tint");
    h.realtime = vec![(0.0, 2.0, false, false)]; // effect-free -- no tint
    h.frame(vec![]);
    assert!(!h.has_fill(pal.playhead), "an effect-free segment gets no danger tint");
    h.realtime = vec![(0.0, 2.0, false, true)]; // heavy AND not ready -- tint
    h.frame(vec![]);
    assert!(h.has_fill(pal.playhead), "a heavy, not-yet-ready segment gets the danger tint");
}

/// Shift-click arms a seam onto the roller set (a plain click, never a drag, so it claims no chord
/// from Shift+DRAG's existing RateStretch binding); the next plain edge-drag folds it in and moves
/// both cuts by the SAME delta, each from its own press-time edge, while an untouched third seam on
/// the same track (E) stays put.
#[test]
fn asymmetric_multi_roller_trim_moves_only_shift_clicked_seams() {
    let mut h = Harness::new();
    h.project.tracks[0].clips =
        vec![Clip::new(101, ClipKind::Video, "A", 0.0, 3.0), Clip::new(102, ClipKind::Video, "E", 5.0, 3.0)];
    h.project.add_track(TrackKind::Video);
    let v2 = h.project.video_tracks()[1];
    h.project.tracks[v2].clips.push(Clip::new(103, ClipKind::Video, "D", 1.0, 3.0));
    h.frame(vec![]);

    let lanes = h.state.lanes_rect;
    let v1_row_top = lanes.top() + h.project.tracks[v2].height; // V2 draws above V1
    let a_end_x = h.state.x_at(3.0);
    h.press_m(pos2(a_end_x - 2.0, v1_row_top + 30.0), Modifiers::SHIFT);
    h.release_m(pos2(a_end_x - 2.0, v1_row_top + 30.0), Modifiers::SHIFT);
    h.frame(vec![]);
    assert_eq!(h.state.rollers, vec![(101, false)], "Shift-click armed A's end edge");

    let d_start_x = h.state.x_at(1.0);
    let from = pos2(d_start_x + 2.0, lanes.top() + 30.0);
    let to = from - vec2(20.0, 0.0); // 0.5 s left at zoom 40
    assert!(h.drag(from, to));

    assert!((h.project.tracks[v2].clips[0].start - 0.5).abs() < 0.05, "D's start moved by the drag delta");
    let a_after = h.project.tracks[0].clips.iter().find(|c| c.id == 101).unwrap();
    assert!((a_after.end() - 2.5).abs() < 0.05, "A's end moved by the SAME delta, from its own edge");
    let e_after = h.project.tracks[0].clips.iter().find(|c| c.id == 102).unwrap();
    assert_eq!(e_after.start, 5.0, "an untouched seam on the same track stays put");
    assert!(h.state.rollers.is_empty(), "the roller set is consumed once the drag starts");
}

#[test]
fn pacing_spans_boundary_is_not_flagged() {
    let mut p = Project::new();
    // exactly at the thresholds -- the condition is strict (< / >), so neither is flagged
    let at_short = Clip::new(1, ClipKind::Video, "s", 0.0, 1.5);
    let at_long = Clip::new(2, ClipKind::Video, "l", 2.0, 20.0);
    p.tracks[0].clips = vec![at_short, at_long];
    assert!(pacing_spans(&p, (1.5, 20.0)).is_empty(), "clips exactly at the boundary are not flagged");
}

#[test]
fn header_rename_blank_name_is_refused() {
    let mut h = Harness::new();
    h.state.track_rename = Some((0, "   ".into())); // blank after trim -- rename_track refuses it
    h.frame(vec![Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }]);
    assert_eq!(h.project.tracks[0].name, "V1", "refused rename leaves the name untouched");
    assert_eq!(h.undos, 0, "a refused op pushes no undo");
}

#[test]
fn asymmetric_roller_shift_click_toggles_off() {
    let mut h = Harness::new();
    let lanes = h.state.lanes_rect;
    let x_end = h.state.x_at(10.0);
    let pos = pos2(x_end - 2.0, lanes.top() + 30.0);
    h.press_m(pos, Modifiers::SHIFT);
    h.release_m(pos, Modifiers::SHIFT);
    h.frame(vec![]);
    assert_eq!(h.state.rollers.len(), 1, "first Shift-click arms the seam");
    h.press_m(pos, Modifiers::SHIFT);
    h.release_m(pos, Modifiers::SHIFT);
    h.frame(vec![]);
    assert!(h.state.rollers.is_empty(), "a second Shift-click on the same seam removes it");
}

/// Unlike `asymmetric_multi_roller_trim_moves_only_shift_clicked_seams` above (A/E have a 2-second gap
/// specifically so the click lands on a lone edge handle), A and B here actually TOUCH -- so the seam's
/// own 6px hit-strip (registered after every clip in the row, winning hit-testing at an exact cut) is
/// what receives the click. Before the fix, that seam handler only understood Ctrl/Alt (EditPoint
/// Left/Right) and ignored Shift entirely, so a Shift-click at an abutting seam -- the classic roll-edit
/// / L-cut scenario this feature exists for -- silently fell through to a plain EditPoint select and
/// never touched `state.rollers`.
#[test]
fn asymmetric_multi_roller_trim_arms_an_abutting_seam() {
    let mut h = Harness::new();
    h.project.tracks[0].clips = vec![
        Clip::new(101, ClipKind::Video, "A", 0.0, 3.0),  // ends 3.0
        Clip::new(102, ClipKind::Video, "B", 3.0, 2.0),  // starts 3.0 -- abuts A: the seam under test
        Clip::new(103, ClipKind::Video, "C", 10.0, 2.0), // ends 12.0
        Clip::new(104, ClipKind::Video, "D", 12.0, 2.0), // starts 12.0 -- an untouched abutting seam
    ];
    h.project.add_track(TrackKind::Video);
    let v2 = h.project.video_tracks()[1];
    h.project.tracks[v2].clips.push(Clip::new(105, ClipKind::Video, "X", 1.0, 3.0));
    h.frame(vec![]);

    let lanes = h.state.lanes_rect;
    let v1_row_top = lanes.top() + h.project.tracks[v2].height; // V2 draws above V1

    let seam_x = h.state.x_at(3.0);
    let pos = pos2(seam_x, v1_row_top + 30.0);
    h.press_m(pos, Modifiers::SHIFT);
    h.release_m(pos, Modifiers::SHIFT);
    h.frame(vec![]);
    assert_eq!(h.state.rollers.len(), 2, "Shift-click on an abutting seam arms both sides of the cut");
    assert!(h.state.rollers.contains(&(101, false)), "A's end edge armed");
    assert!(h.state.rollers.contains(&(102, true)), "B's start edge armed");
    assert!(h.state.edit_point.is_none(), "a Shift-click on the seam must not also set a plain EditPoint");

    let x_start_x = h.state.x_at(1.0);
    let from = pos2(x_start_x + 2.0, lanes.top() + 30.0);
    let to = from - vec2(20.0, 0.0); // 0.5 s left at zoom 40
    assert!(h.drag(from, to));

    assert!((h.project.tracks[v2].clips[0].start - 0.5).abs() < 0.05, "X's start moved by the drag delta");
    let a_after = h.project.tracks[0].clips.iter().find(|c| c.id == 101).unwrap();
    assert!((a_after.end() - 2.5).abs() < 0.05, "A's end rolled by the SAME delta, from its own edge");
    let b_after = h.project.tracks[0].clips.iter().find(|c| c.id == 102).unwrap();
    assert!((b_after.start - 2.5).abs() < 0.05, "B's start rolled by the SAME delta -- still abutting A");
    let c_after = h.project.tracks[0].clips.iter().find(|c| c.id == 103).unwrap();
    assert_eq!(c_after.end(), 12.0, "an untouched seam elsewhere on the same track (C/D) stayed put");
    let d_after = h.project.tracks[0].clips.iter().find(|c| c.id == 104).unwrap();
    assert_eq!(d_after.start, 12.0, "an untouched seam elsewhere on the same track (C/D) stayed put");
    assert!(h.state.rollers.is_empty(), "the roller set is consumed once the drag starts");
}

/// "Move To" re-anchors a transition around its clip and keeps its settings; a position with no cut
/// to sit on changes nothing. (The Transitions pane's Position combo, before it went catalogue-only.)
#[test]
fn move_transition_keeps_settings_and_refuses_missing_cuts() {
    use super::menus::{move_transition, TransPos};
    use crate::model::{TransitionEdge, TransitionKind};
    let mut h = Harness::new();
    h.project.split_at(4.0, None);
    h.project.split_at(7.0, None);
    let (c1, c2) = (h.project.tracks[0].clips[0].id, h.project.tracks[0].clips[1].id);
    let tid = h.project.add_transition(c2, TransitionKind::Wipe, 1.0).unwrap();
    if let Some(t) = h.project.transition_mut(tid) {
        (t.color, t.direction) = ([9, 9, 9, 255], 2);
    }
    let only = |p: &Project| p.tracks[0].transitions.clone();
    // cut before c2 -> end of c2 (fade out), settings carried over
    assert!(move_transition(&mut h.project, tid, TransPos::End));
    let t = &only(&h.project)[0];
    assert_eq!(
        (t.right, t.edge, t.kind, t.color, t.direction),
        (c2, TransitionEdge::Out, TransitionKind::Wipe, [9, 9, 9, 255], 2)
    );
    // -> the cut with the next clip belongs to that next clip
    let tid = t.id;
    assert!(move_transition(&mut h.project, tid, TransPos::Next));
    let t = &only(&h.project)[0];
    assert_eq!((t.edge, t.right), (TransitionEdge::Cut, h.project.tracks[0].clips[2].id));
    assert_eq!(only(&h.project).len(), 1, "moved, not copied");
    // c1 has no previous clip: refused, nothing removed
    let tid = h.project.add_edge_transition(c1, TransitionKind::CrossFade, 1.0, false).unwrap();
    assert!(!move_transition(&mut h.project, tid, TransPos::Previous));
    assert!(only(&h.project).iter().any(|t| t.id == tid && t.edge == TransitionEdge::In));
}

/// The transition band's right-click has "Move To" for a single transition; picking a position moves
/// it (one edit), and the bulk menu for several doesn't offer it.
#[test]
fn transition_menu_move_to_moves_one_transition() {
    use crate::model::TransitionEdge;
    let mut h = Harness::new();
    let lanes = h.state.lanes_rect;
    h.project.split_at(4.0, None);
    let c2 = h.project.tracks[0].clips[1].id;
    h.project.add_transition(c2, TransitionKind::CrossFade, 1.0).unwrap();
    h.frame(vec![]);
    let band = pos2(h.state.x_at(4.0), lanes.top() + 30.0);
    h.rclick(band);
    h.frame(vec![]);
    let move_to = h.painted_text("Move To").expect("Move To submenu painted") + vec2(4.0, 4.0);
    h.press(move_to);
    h.release(move_to);
    let end = h.painted_text("End of Clip (fade out)").expect("positions listed") + vec2(4.0, 4.0);
    h.press(end);
    let r = h.release(end);
    assert!(r.edited, "picking a position edits the project");
    let t = &h.project.tracks[0].transitions[0];
    assert_eq!((t.right, t.edge), (c2, TransitionEdge::Out));
}

/// Resolve-style delete: RippleDelete (the Delete key) closes the gap, Delete (Backspace) leaves it; and
/// "Auto close gaps" packs every unlocked track left so a move or lift never leaves a hole.
#[test]
fn ripple_delete_closes_gap_lift_leaves_it_and_auto_close_packs() {
    let mut h = Harness::new();
    let v = |p: &mut Project| {
        p.tracks[1].clips.clear();
        p.tracks[0].clips.clear();
        p.tracks[0].clips.push(Clip::new(401, ClipKind::Video, "a", 0.0, 2.0));
        p.tracks[0].clips.push(Clip::new(402, ClipKind::Video, "b", 2.0, 2.0));
    };
    v(&mut h.project);
    gestures::delete_clips_magnetic(&mut h.project, &[401], true);
    assert_eq!(h.project.clip(402).unwrap().start, 0.0, "ripple delete pulls b left");
    v(&mut h.project);
    gestures::delete_clips_magnetic(&mut h.project, &[401], false);
    assert_eq!(h.project.clip(402).unwrap().start, 2.0, "lift leaves the gap");
    // auto close gaps: a lifted hole and a moved-away clip both pack back to 0
    gestures::close_all_gaps(&mut h.project);
    assert_eq!(h.project.clip(402).unwrap().start, 0.0);
    h.project.tracks[0].clips.push(Clip::new(403, ClipKind::Video, "c", 7.0, 1.0));
    gestures::close_all_gaps(&mut h.project);
    assert_eq!(h.project.clip(403).unwrap().start, 2.0, "c packs up against b");
    h.project.tracks[0].locked = true;
    h.project.tracks[0].clips[1].start = 5.0;
    gestures::close_all_gaps(&mut h.project);
    assert_eq!(h.project.clip(403).unwrap().start, 5.0, "a locked track is left alone");
}

/// Hiding a track kind is view-only: the audio rows move up into the lanes, a click there hits the
/// audio clip (not the hidden video one), and the project itself is untouched.
#[test]
fn hidden_video_rows_are_view_only() {
    let mut h = Harness::new();
    h.frame(vec![]);
    let before = h.project.to_json();
    h.state.show_kinds = (false, true);
    h.frame(vec![]);
    let top = h.state.lanes_rect.top() + 5.0;
    let ti = h.state.track_at(top, &h.project).expect("a row at the top");
    assert_eq!(h.project.tracks[ti].kind, TrackKind::Audio, "first visible row is audio");
    assert_eq!(h.project.to_json(), before, "hiding rows never edits the project");
    let p = pos2(h.state.lanes_rect.left() + 100.0, top + 25.0);
    h.press(p);
    h.release(p);
    h.frame(vec![]);
    let hit: Vec<_> =
        h.selection.iter().filter_map(|&id| h.project.find(id)).map(|(t, _)| h.project.tracks[t].kind).collect();
    assert!(hit.contains(&TrackKind::Audio), "clicked the audio clip (linked video comes along): {hit:?}");
}

/// tools-panel: each timeline tool the toolbar adds does its job on a click or a drag.
#[test]
fn toolbar_tools_act_on_the_timeline() {
    let mut h = Harness::new();
    let lanes = h.state.lanes_rect;
    let y = lanes.top() + 30.0;
    let x = |h: &Harness, t: f64| h.state.x_at(t);

    // Marker: a drag lays down a range marker (a click still drops a point, see tools_cut_mark_and_stretch)
    h.tool = Tool::Marker;
    h.drag(pos2(x(&h, 2.0), y), pos2(x(&h, 4.0), y));
    assert_eq!(h.project.markers.len(), 1, "marker drag drops one marker");
    let m = &h.project.markers[0];
    assert!((m.t - 2.0).abs() < 0.05 && (m.duration - 2.0).abs() < 0.05, "a 2..4 s range: {} + {}", m.t, m.duration);
    assert_eq!(h.video_clip().start, 0.0, "and never moves the clip under it");

    // Pen: a click keys the video clip's opacity / the audio clip's volume there
    h.tool = Tool::Pen;
    let p = pos2(x(&h, 5.0), y);
    h.time += 1.0;
    h.press(p);
    h.release(p);
    h.frame(vec![]);
    assert!(h.video_clip().opacity.has_key_at(5.0), "pen keys opacity at the click");
    let ay = lanes.top() + h.project.tracks[0].height + h.project.tracks[1].height * 0.9; // clear of the volume line
    let p = pos2(x(&h, 6.0), ay);
    h.time += 1.0;
    h.press(p);
    h.release(p);
    h.frame(vec![]);
    assert!(h.audio_clip().volume.has_key_at(6.0), "pen keys volume on an audio clip");

    // Track Select Forward from 3 s: both linked clips (they end after 3 s); Backward from 0.5 s: they
    // start before it; Shift = that track only
    h.tool = Tool::TrackForward;
    h.selection.clear();
    let p = pos2(x(&h, 3.0), y);
    h.time += 1.0;
    h.press(p);
    h.release(p);
    h.frame(vec![]);
    assert_eq!(h.selection.len(), 2, "forward selects every track");
    h.tool = Tool::TrackBackward;
    h.selection.clear();
    h.time += 1.0;
    h.press_m(p, Modifiers::SHIFT);
    h.release_m(p, Modifiers::SHIFT);
    h.frame(vec![]);
    assert_eq!(h.selection, vec![h.video_clip().id], "Shift: the clicked track only");

    // Zoom: click in, Alt-click out
    h.tool = Tool::Zoom;
    let z0 = h.state.zoom;
    h.time += 1.0;
    h.press(p);
    h.release(p);
    h.frame(vec![]);
    assert!((h.state.zoom - z0 * 2.0).abs() < 1e-3, "click zooms in: {z0} -> {}", h.state.zoom);
    h.time += 1.0;
    h.press_m(p, Modifiers::ALT);
    h.release_m(p, Modifiers::ALT);
    h.frame(vec![]);
    assert!((h.state.zoom - z0).abs() < 1e-3, "Alt-click zooms out: {}", h.state.zoom);

    // Hand: a drag scrolls the lanes and edits nothing
    h.tool = Tool::Hand;
    h.state.scroll_x = 5.0;
    let from = pos2(x(&h, 6.0), y);
    assert!(!h.drag(from, from + vec2(80.0, 0.0)), "hand edits nothing");
    assert!(h.state.scroll_x < 5.0, "hand drag scrolls: {}", h.state.scroll_x);
    assert_eq!(h.video_clip().start, 0.0);
}

/// tools-panel: the trim tools run the Select tool's modifier gestures on a plain drag.
#[test]
fn toolbar_trim_tools_reuse_the_trim_gestures() {
    // Ripple Edit: dragging the end in pulls the next clip (split at 5 s) along with it
    let mut h = Harness::new();
    h.project.split_at(5.0, None);
    let lanes = h.state.lanes_rect;
    let y = lanes.top() + 30.0;
    h.tool = Tool::Ripple;
    let edge = h.state.x_at(5.0) - 2.0;
    assert!(h.drag(pos2(edge, y), pos2(edge - 40.0, y)), "ripple trims");
    let next = &h.project.tracks[0].clips[1];
    assert!(next.start < 5.0 - 0.5, "ripple pulled the next clip left: {}", next.start);

    // Slip: the body drag moves the source window, not the clip
    let mut h = Harness::new();
    h.project.split_at(5.0, None);
    h.tool = Tool::Slip;
    let id = h.project.tracks[0].clips[1].id;
    let src0 = h.project.clip(id).unwrap().src_in;
    let at = pos2(h.state.x_at(7.0), y);
    assert!(h.drag(at, at + vec2(40.0, 0.0)), "slip edits");
    let c = h.project.clip(id).unwrap();
    assert!(
        (c.start - 5.0).abs() < 1e-6 && (c.src_in - src0).abs() > 0.1,
        "slipped in place: {} {}",
        c.start,
        c.src_in
    );
}

/// ws:keyframe-blocks: an applied block is a bar above the key strip - drag its body to move it (keys
/// follow), drag its right end to stretch it; each drag is one undo.
#[test]
fn headless_key_block_bar_moves_and_stretches() {
    let mut h = Harness::new();
    let lanes = h.state.lanes_rect;
    let id = h.project.tracks[0].clips[0].id;
    let spin = crate::model::find_block(&[], "Spin").unwrap();
    h.project.apply_key_block(id, &spin, Some(1.0), None);
    h.frame(vec![]);
    let bar_y = lanes.top() + h.project.tracks[0].height - 1.0 - 14.0;
    let from = pos2(h.state.x_at(1.3), bar_y);
    assert!(h.drag(from, from + vec2(40.0, 0.0)), "bar drag edits");
    assert_eq!(h.undos, 1);
    let c = &h.project.tracks[0].clips[0];
    assert!((c.blocks[0].t - 2.0).abs() < 1.0 / 30.0 + 1e-6, "moved to {}", c.blocks[0].t);
    assert!((c.rotation.keys[0].t - c.blocks[0].t).abs() < 1e-9, "keys ride along");
    let end_t = c.blocks[0].end();
    h.frame(vec![]);
    let end = pos2(h.state.x_at(end_t) - 3.0, bar_y);
    assert!(h.drag(end, end + vec2(40.0, 0.0)), "edge drag edits");
    let c = &h.project.tracks[0].clips[0];
    assert!((c.blocks[0].dur - 1.8).abs() < 1.0 / 30.0 + 1e-6, "stretched to {}", c.blocks[0].dur);
    assert!((c.rotation.at(c.blocks[0].end()) - 360.0).abs() < 1e-6);
    assert_eq!(h.undos, 2);
}
