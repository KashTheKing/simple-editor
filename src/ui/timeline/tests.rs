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
use egui::{Event, Modifiers, PointerButton, RawInput};

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
    time: f64,
    /// Paint list of the last frame (asserting on what was actually drawn).
    shapes: Vec<egui::epaint::ClippedShape>,
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
        let Harness { ctx, state, project, selection, sel_transitions, playhead, undos, waves, tool, snap, .. } = self;
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
                        sel_transitions,
                        playhead,
                        undo: &mut undo,
                        waveforms: waves,
                        palette: &pal,
                        snap: *snap,
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

/// Snapping is not just for move/trim: the razor, the marker tool and marker drags land on the same
/// candidates as everything else. In/out points are used as the candidates here — the playhead has a
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

    // and dragging the ruler flag snaps too — over to the out point, the only candidate near the drop
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
    assert!(!r.edited, "the timeline changes nothing itself — the app adds the effect");
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
fn headless_mini_graph_toggle_needs_keys_and_width() {
    let mut h = Harness::new();
    let lanes = h.state.lanes_rect;
    let vid = h.video_clip().id;
    // default zoom (40 px/s) makes the 10 s clip 400 px wide — well past MINI_GRAPH_MIN_W (120)
    let btn_center = |h: &Harness| {
        let right = h.state.x_at(h.video_clip().end());
        let top = lanes.top() + 1.0;
        pos2(right - MINI_GRAPH_PAD - MINI_GRAPH_BTN * 0.5, top + MINI_GRAPH_PAD + MINI_GRAPH_BTN * 0.5)
    };

    // no keyframes yet: a click where the icon would sit just selects the clip like anywhere else
    // on its body — proves nothing is drawn/interactive there regardless of zoom
    let p = btn_center(&h);
    h.press(p);
    h.release(p);
    h.frame(vec![]);
    assert!(h.state.mini_graph_open.is_empty(), "no icon without keyframes");
    // the harness's default video clip is linked to an audio clip, so a plain click selects the
    // whole link group (same as clicking anywhere else on its body) — not just `vid` alone.
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

/// Right-clicking 2+ selected transitions offers "Change Type"/"Change Easing" quick-changes —
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
    ctx.set_fonts(crate::theme::test_fonts()); // size-diet: no default_fonts feature anymore
    let mut texts = |audio: bool| -> Vec<String> {
        let (mut act, mut acts, mut edit) = (None, Vec::new(), false);
        let full = ctx.run(
            RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 1400.0))), ..Default::default() },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    clip_menu(
                        ui,
                        1,
                        false,
                        false,
                        true,
                        audio,
                        false,
                        None,
                        &mut false,
                        &p.labels,
                        &p.buses,
                        &[],
                        &mut act,
                        &mut acts,
                        &mut edit,
                    )
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
    b.effects.push(Effect::new(EffectKind::Vignette)); // only on b — not shared
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
    let ctx = egui::Context::default();
    ctx.set_fonts(crate::theme::test_fonts()); // size-diet: no default_fonts feature anymore
    let mut texts = |shared: &[EffectKind]| -> Vec<String> {
        let (mut act, mut acts, mut edit) = (None, Vec::new(), false);
        let full = ctx.run(
            RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 1400.0))), ..Default::default() },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    clip_menu(
                        ui, 1, false, false, true, false, false, None, &mut false, &p.labels, &p.buses, shared,
                        &mut act, &mut acts, &mut edit,
                    )
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
    let none = texts(&[]);
    assert!(!none.iter().any(|s| s == "Effects"), "no shared kind: no Effects submenu: {none:?}");
    let shared = texts(&[EffectKind::Blur]);
    assert!(shared.iter().any(|s| s == "Effects"), "a shared kind offers the Effects submenu: {shared:?}");
}

/// Right-clicking 2+ selected video clips offers a "Transform" submenu; "Stretch to Screen" fits
/// every selected clip with a native size to the project canvas — each against its OWN asset's
/// native size, not a shared one — in exactly one undo step for the whole bulk operation.
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
    // must not be misread as one double-click on the second clip — that fires the double-click
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
    // V1 header row: S button right-aligned (18 wide, 4 in), V button 21 pt left of it
    let vb = pos2(lanes.left() - 34.0, lanes.top() + h.project.tracks[0].height * 0.5);
    assert!(!h.project.tracks[0].muted);
    h.press(vb);
    let r = h.release(vb);
    assert!(r.edited || h.frame(vec![]).edited, "V toggle marks edited");
    assert!(h.project.tracks[0].muted, "V toggle flips muted (visibility off)");
    assert_eq!(h.undos, 1);
}
