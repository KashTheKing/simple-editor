use super::*;

fn asset(id: Id, dur: f64, streams: usize) -> Asset {
    Asset {
        id,
        path: format!("C:/v{id}.mp4"),
        kind: ClipKind::Video,
        duration: dur,
        width: 1280,
        height: 720,
        fps: 30.0,
        audio_streams: (0..streams)
            .map(|i| AudioStreamInfo { index: i, channels: 2, sample_rate: 48000, ..Default::default() })
            .collect(),
        codec: "h264".into(),
        folder: String::new(),
        tags: Vec::new(),
        label: 0,
        description: String::new(),
        rel_path: None,
        parent: None,
        range: None,
        effects: Vec::new(),
    }
}

/// `paste_attributes` with only `text_style` set must NOT touch the destination clip's own
/// wording — the bug the user reported (pasting text attributes used to overwrite the text too,
/// since `AttrSet::text` copied the whole `TextStyle` including its content).
#[test]
fn paste_text_style_keeps_destination_wording_content_only_keeps_destination_style() {
    let mut p = Project::new();
    let mut src = Clip::new(1, ClipKind::Text, "src", 0.0, 4.0);
    src.text = Some(TextStyle { text: "Hello".into(), size: 40.0, bold: true, ..Default::default() });
    let mut dst = Clip::new(2, ClipKind::Text, "dst", 0.0, 4.0);
    dst.text = Some(TextStyle { text: "World".into(), size: 72.0, bold: false, ..Default::default() });
    let dst_id = dst.id;
    p.tracks[0].clips.push(dst);

    // style only: destination keeps its own wording, gains the source's look
    let n = p.paste_attributes(&src, &[dst_id], AttrSet { text_style: true, ..AttrSet::NONE });
    assert_eq!(n, 1);
    let t = p.clip(dst_id).unwrap().text.as_ref().unwrap();
    assert_eq!(t.text, "World", "style-only paste must not touch the wording");
    assert_eq!((t.size, t.bold), (40.0, true), "style fields copy from the source");

    // content only: destination keeps its (now-updated) style, gains the source's wording
    p.paste_attributes(&src, &[dst_id], AttrSet { text_content: true, ..AttrSet::NONE });
    let t = p.clip(dst_id).unwrap().text.as_ref().unwrap();
    assert_eq!(t.text, "Hello", "content-only paste copies the wording");
    assert_eq!((t.size, t.bold), (40.0, true), "content-only paste must not touch the style");
}

/// Style-only paste copies the source's spans, whose char ranges index the SOURCE wording — they
/// must be clamped to the destination's (shorter) text instead of dangling in the saved file.
#[test]
fn paste_text_style_clamps_spans_to_destination_wording() {
    let mut p = Project::new();
    let mut src = Clip::new(1, ClipKind::Text, "src", 0.0, 4.0);
    src.text = Some(TextStyle {
        text: "Hello beautiful world".into(),
        spans: vec![
            TextSpan { start: 6, end: 15, color: Some([255, 0, 0, 255]), ..Default::default() },
            TextSpan { start: 0, end: 4, bold: Some(true), ..Default::default() },
        ],
        ..Default::default()
    });
    let mut dst = Clip::new(2, ClipKind::Text, "dst", 0.0, 4.0);
    dst.text = Some(TextStyle { text: "Hi there".into(), ..Default::default() }); // 8 chars
    p.tracks[0].clips.push(dst);
    p.paste_attributes(&src, &[2], AttrSet { text_style: true, ..AttrSet::NONE });
    let t = p.clip(2).unwrap().text.as_ref().unwrap();
    assert_eq!(t.text, "Hi there");
    assert_eq!(t.spans.len(), 2);
    assert!(t.spans.iter().all(|s| s.end <= 8 && s.start < s.end), "{:?}", t.spans);
}

/// Span ranges follow the characters they styled across a text edit (type before / inside /
/// replace over), instead of silently pointing at whatever now sits at the old offsets.
#[test]
fn remap_spans_follows_the_styled_characters() {
    let styled = |text: &str| TextStyle {
        text: text.into(),
        spans: vec![TextSpan { start: 6, end: 11, bold: Some(true), ..Default::default() }], // "World"
        ..Default::default()
    };

    // insert before the span: it shifts right
    let mut t = styled("Hello World");
    t.text = "Hey, Hello World".into();
    t.remap_spans("Hello World");
    assert_eq!((t.spans[0].start, t.spans[0].end), (11, 16));

    // type inside the span: it grows around the insertion
    let mut t = styled("Hello World");
    t.text = "Hello WoXrld".into();
    t.remap_spans("Hello World");
    assert_eq!((t.spans[0].start, t.spans[0].end), (6, 12));

    // delete the whole styled word: the span disappears
    let mut t = styled("Hello World");
    t.text = "Hello ".into();
    t.remap_spans("Hello World");
    assert!(t.spans.is_empty());

    // replace overlapping the span's head: the surviving tail stays styled
    let mut t = styled("Hello World");
    t.text = "HelZrld".into(); // replaced chars 3..8 ("lo Wo") with "Z"
    t.remap_spans("Hello World");
    assert_eq!((t.spans[0].start, t.spans[0].end), (4, 7)); // "rld"

    // non-ASCII: char indices, not bytes
    let mut t = TextStyle {
        text: "héllo wörld".into(),
        spans: vec![TextSpan { start: 6, end: 11, bold: Some(true), ..Default::default() }],
        ..Default::default()
    };
    t.text = "ah héllo wörld".into();
    t.remap_spans("héllo wörld");
    assert_eq!((t.spans[0].start, t.spans[0].end), (9, 14));
}

/// "Clear Style on Selection": trims, drops, or splits spans overlapping the cleared range.
#[test]
fn clear_span_range_trims_drops_and_splits() {
    let mut t = TextStyle {
        text: "abcdefghij".into(),
        spans: vec![
            TextSpan { start: 0, end: 10, bold: Some(true), ..Default::default() }, // contains → split
            TextSpan { start: 4, end: 6, italic: Some(true), ..Default::default() }, // inside → dropped
            TextSpan { start: 0, end: 5, color: Some([1, 2, 3, 255]), ..Default::default() }, // head survives
            TextSpan { start: 5, end: 10, size: Some(9.0), ..Default::default() },  // tail survives
        ],
        ..Default::default()
    };
    t.clear_span_range(4, 6);
    let mut ranges: Vec<(usize, usize)> = t.spans.iter().map(|s| (s.start, s.end)).collect();
    ranges.sort();
    assert_eq!(ranges, vec![(0, 4), (0, 4), (6, 10), (6, 10)]);
    assert!(t.spans.iter().all(|s| s.end <= 4 || s.start >= 6));
}

#[test]
fn animated_interp() {
    let mut a = Animated::new(5.0);
    assert_eq!(a.at(3.0), 5.0);
    a.toggle_key(0.0);
    a.set_at(2.0, 15.0);
    assert_eq!(a.keys.len(), 2);
    assert!((a.at(1.0) - 10.0).abs() < 1e-9);
    assert_eq!(a.at(-1.0), 5.0);
    assert_eq!(a.at(9.0), 15.0);
    a.toggle_key(2.0);
    a.toggle_key(0.0);
    assert!(!a.is_animated());
    assert_eq!(a.value, 5.0);
}

#[test]
fn easing_and_key_moves() {
    let mut a = Animated::new(0.0);
    a.toggle_key(0.0);
    a.set_at(2.0, 10.0);
    a.set_ease_at(0.0, Ease::Hold);
    assert_eq!(a.at(1.0), 0.0);
    a.set_ease_at(0.0, Ease::EaseInOut);
    assert!((a.at(1.0) - 5.0).abs() < 1e-9);
    assert!(a.at(0.5) < 2.5);
    a.set_ease_at(0.0, Ease::EaseIn);
    assert!((a.at(1.0) - 2.5).abs() < 1e-9);
    let j = a.move_key(1, -1.0); // moves before the first key and stays sorted
    assert_eq!(j, 0);
    assert_eq!(a.keys[0].v, 10.0);
}

#[test]
fn from_media_layout() {
    let p = Project::from_media(asset(0, 10.0, 2));
    assert_eq!(p.width, 1280);
    assert_eq!(p.tracks.len(), 3); // V1 A1 A2
    assert_eq!(p.tracks[0].clips.len(), 1);
    assert_eq!(p.tracks[2].clips[0].audio_stream, 1);
    let v = &p.tracks[0].clips[0];
    assert_eq!(p.linked(v.id).len(), 3);
    assert_eq!(p.duration(), 10.0);
    assert!(p.source_video.is_some());
}

#[test]
fn has_video_at_and_audio_clips_at() {
    let mut p = Project::new();
    let at = p.tracks.iter().position(|t| t.kind == TrackKind::Audio).unwrap();
    let mut c = Clip::new(1, ClipKind::Audio, "a", 0.0, 4.0);
    c.asset = 7;
    c.audio_stream = 2;
    p.tracks[at].clips.push(c);

    assert!(!p.has_video_at(1.0), "audio-only region should report no video");
    assert_eq!(p.audio_clips_at(1.0), vec![(7, 2)]);
    assert!(p.audio_clips_at(5.0).is_empty(), "outside the clip's range");

    p.tracks[at].muted = true;
    assert!(p.audio_clips_at(1.0).is_empty(), "a muted track contributes nothing");
    p.tracks[at].muted = false;

    let vt = p.tracks.iter().position(|t| t.kind == TrackKind::Video).unwrap();
    p.tracks[vt].clips.push(Clip::new(2, ClipKind::Video, "v", 0.0, 4.0));
    assert!(p.has_video_at(1.0), "a video clip now covers the same time");
}

#[test]
fn split_delete_ripple() {
    let mut p = Project::from_media(asset(0, 10.0, 1));
    let new = p.split_at(4.0, None);
    assert_eq!(new.len(), 2);
    assert_eq!(p.tracks[0].clips.len(), 2);
    assert!((p.tracks[0].clips[1].src_in - 4.0).abs() < 1e-9);
    // right halves linked to each other but not to left halves
    assert_eq!(p.linked(new[0]).len(), 2);
    assert!(p.linked(new[0]).contains(&new[1]));
    let left = p.tracks[0].clips[0].id;
    assert!(!p.linked(left).contains(&new[0]));
    // ripple delete the left halves
    let ids = p.linked(left);
    p.delete_clips(&ids, true);
    assert_eq!(p.tracks[0].clips.len(), 1);
    assert!((p.tracks[0].clips[0].start).abs() < 1e-9);
    assert!((p.tracks[1].clips[0].start).abs() < 1e-9);
    assert!((p.duration() - 6.0).abs() < 1e-9);
}

#[test]
fn trim_to_range() {
    let mut p = Project::from_media(asset(0, 10.0, 1));
    p.trim_to_range(2.0, 5.0);
    assert!((p.duration() - 3.0).abs() < 1e-9);
    let c = &p.tracks[0].clips[0];
    assert!((c.src_in - 2.0).abs() < 1e-9);
    assert!(c.start.abs() < 1e-9);
}

#[test]
fn move_and_fit() {
    let mut p = Project::from_media(asset(0, 10.0, 1));
    let a2 = asset(1, 3.0, 0);
    let aid = p.add_asset(a2);
    let ids = p.insert_asset_clips(aid, 10.0, Some(0));
    assert_eq!(ids.len(), 1);
    assert_eq!(p.track_of(ids[0]), Some(0));
    // can't move onto the first clip
    assert!(!p.move_clips(&ids, -5.0, 0, None));
    // can move right
    assert!(p.move_clips(&ids, 5.0, 0, None));
    assert!((p.clip(ids[0]).unwrap().start - 15.0).abs() < 1e-9);
    // move to a new video track: none exists -> false
    assert!(!p.move_clips(&ids, 0.0, 1, Some(TrackKind::Video)));
    p.add_track(TrackKind::Video);
    assert!(p.move_clips(&ids, -15.0, 1, Some(TrackKind::Video)));
    assert_eq!(p.track_of(ids[0]), Some(1));
}

#[test]
fn json_roundtrip() {
    let mut p = Project::from_media(asset(0, 10.0, 2));
    p.add_text_clip(1.0, 2.0);
    p.add_cue(0.5, 1.5, "hi");
    let s = p.to_json();
    let q = Project::from_json(&s).unwrap();
    assert_eq!(q.tracks.len(), p.tracks.len());
    assert_eq!(q.to_json(), s);
    let mut q = q;
    let id = q.new_id();
    assert!(id > p.next_id);
}

#[test]
fn polygon_points_round_trip() {
    let mut p = Project::new();
    let id = p.add_shape_clip(ShapeKind::Polygon, 0.0, 2.0);
    let s = p.clip_mut(id).unwrap().shape.as_mut().unwrap();
    assert!(s.poly_points().is_none(), "no points = the regular n-gon");
    s.points = vec![(0.0, -50.0), (60.0, 40.0), (-60.0, 40.0)];
    let key = s.cache_key();
    let t = Project::from_json(&p.to_json()).unwrap().clip(id).unwrap().shape.clone().unwrap();
    assert_eq!(t.points, vec![(0.0, -50.0), (60.0, 40.0), (-60.0, 40.0)]);
    assert_eq!(t.poly_points().map(|v| v.len()), Some(3));
    assert_eq!(t.cache_key(), key, "a round trip is the same rasterised layer");
    // a dragged vertex must not reuse the cached layer
    let mut moved = t.clone();
    moved.points[1].0 += 5.0;
    assert_ne!(moved.cache_key(), key);
    // a project written before points existed still loads, as the regular n-gon
    let old = p.to_json().replace("\"points\"", "\"was_not_a_field\"");
    let o = Project::from_json(&old).unwrap();
    assert!(o.clip(id).unwrap().shape.as_ref().unwrap().poly_points().is_none());
}

#[test]
fn fit_clip_to_screen_noop_without_native_size() {
    let mut p = Project::new();
    let id = p.add_shape_clip(ShapeKind::Rect, 0.0, 2.0);
    assert!(!p.fit_clip_to_screen(id, false));
    assert!(!p.fit_clip_to_screen(id, true));
}

#[test]
fn fit_clip_to_screen_resets_defaults_regardless_of_prior_values() {
    let mut p = Project::new();
    let aid = p.add_asset(asset(1, 5.0, 0));
    let vt = p.tracks.iter().position(|t| t.kind == TrackKind::Video).unwrap();
    let mut c = Clip::new(101, ClipKind::Video, "v", 0.0, 5.0);
    c.asset = aid;
    c.x.value = 123.0;
    c.y.value = -45.0;
    c.scale.value = 3.0;
    c.rotation.value = 30.0;
    c.scale_x.value = 5.0;
    c.scale_y.value = 0.2;
    p.tracks[vt].clips.push(c);

    assert!(p.fit_clip_to_screen(101, false));
    let clip = p.clip(101).unwrap();
    assert_eq!(clip.x.value, 0.0);
    assert_eq!(clip.y.value, 0.0);
    assert_eq!(clip.scale.value, 1.0);
    assert_eq!(clip.rotation.value, 0.0, "a rotated quad can't fill the canvas");
    assert_eq!(clip.scale_x.value, 1.0);
    assert_eq!(clip.scale_y.value, 1.0);
}

/// Round-trips "Stretch to Screen" through the real placement formula: native 1280x720 (16:9,
/// same ratio the `asset()` helper always uses) placed on a 1080x1920 (9:16) canvas is
/// width-constrained, so the hand-derived scale_y is `native_aspect / canvas_aspect` — and the
/// resulting placement must fill the canvas exactly, both axes.
#[test]
fn fit_clip_to_screen_stretch_fills_canvas_exactly() {
    let mut p = Project::new();
    p.width = 1080;
    p.height = 1920;
    let aid = p.add_asset(asset(1, 5.0, 0)); // 1280x720, 16:9
    let vt = p.tracks.iter().position(|t| t.kind == TrackKind::Video).unwrap();
    let mut c = Clip::new(100, ClipKind::Video, "v", 0.0, 5.0);
    c.asset = aid;
    c.x.value = 40.0; // pre-existing transform the reset must clear
    c.scale.value = 2.0;
    p.tracks[vt].clips.push(c);

    assert!(p.fit_clip_to_screen(100, true));
    let clip = p.clip(100).unwrap();
    assert_eq!(clip.x.value, 0.0);
    assert_eq!(clip.y.value, 0.0);
    assert_eq!(clip.scale.value, 1.0);
    let expected_sy = (1280.0_f64 / 720.0) / (1080.0 / 1920.0);
    assert_eq!(clip.scale_x.value, 1.0);
    assert!((clip.scale_y.value - expected_sy).abs() < 1e-12);

    let placement = crate::engine::compose::placement(&p, clip, 0.0, (1280, 720), p.width, p.height, true);
    assert!((placement.w - p.width as f32).abs() < 1e-3, "w = {} vs canvas {}", placement.w, p.width);
    assert!((placement.h - p.height as f32).abs() < 1e-3, "h = {} vs canvas {}", placement.h, p.height);
}

#[test]
fn a_recorded_stroke_becomes_a_motion_path() {
    let mut p = Project::new();
    let id = p.add_shape_clip(ShapeKind::Draw, 0.0, 4.0);
    let s = p.clip_mut(id).unwrap().shape.as_mut().unwrap();
    // two takes, the second recorded 2 s in, with a still-mouse pause in the middle of the first
    s.strokes = vec![
        Stroke { color: [255; 4], width: 6.0, points: vec![(-100.0, 0.0, 0.0), (0.0, 50.0, 0.5), (0.0, 50.0, 0.5)] },
        Stroke { color: [255; 4], width: 6.0, points: vec![(60.0, 20.0, 2.0), (120.0, -30.0, 2.5)] },
    ];
    p.clip_mut(id).unwrap().x.value = 10.0; // the sketch's own position is part of the path
    let pts = p.path_from_clip(id);
    assert_eq!(pts.len(), 5);
    let mover = p.add_shape_clip(ShapeKind::Rect, 0.0, 8.0);
    assert!(p.apply_path(mover, &pts));
    let c = p.clip(mover).unwrap();
    assert_eq!(c.x.keys.len(), 5);
    for w in c.x.keys.windows(2) {
        assert!(w[1].t > w[0].t, "key times must increase: {:?} -> {:?}", w[0].t, w[1].t);
    }
    assert!(c.x.keys.iter().all(|k| k.t <= 8.0 + 1e-9), "the path fits the clip: {:?}", c.x.keys);
    // starts on the first point and ends on the last, both at the clip's own ends
    assert!((c.x.at(0.0) - -90.0).abs() < 1e-6 && (c.y.at(0.0) - 0.0).abs() < 1e-6);
    assert!((c.x.at(8.0) - 130.0).abs() < 1e-6 && (c.y.at(8.0) - -30.0).abs() < 1e-6);
    // a path with no clock (a polygon outline) spreads its points evenly over the clip
    let poly = path_to_keys(&[(0.0, -50.0, 0.0), (60.0, 40.0, 0.0), (-60.0, 40.0, 0.0)], 2.0);
    assert_eq!(poly.0.keys.iter().map(|k| k.t).collect::<Vec<_>>(), vec![0.0, 1.0, 2.0]);
    assert!(path_to_keys(&[], 3.0).0.keys.is_empty(), "an empty path animates nothing");
}

#[test]
fn live_links_follow_edits_and_detach_on_manual_change() {
    let mut p = Project::new();
    let pid = p.add_path("walk".into(), vec![(0.0, 0.0, 0.0), (100.0, 50.0, 1.0)]);
    let id = p.add_shape_clip(ShapeKind::Rect, 0.0, 4.0);
    assert!(p.link_path(id, pid));
    p.refresh_links();
    let c = p.clip(id).unwrap();
    assert!((c.x.at(4.0) - 100.0).abs() < 1e-6 && (c.y.at(4.0) - 50.0).abs() < 1e-6);
    // editing the saved path moves the linked clip too — that's the point of a live link
    p.paths[0].points[1].0 = 200.0;
    p.refresh_links();
    assert!((p.clip(id).unwrap().x.at(4.0) - 200.0).abs() < 1e-6);
    // an expression sees t and the property's own base value v
    let c = p.clip_mut(id).unwrap();
    c.opacity.value = 0.5;
    c.opacity.link = AnimLink::Expr("return value + t / 8".into());
    p.refresh_links();
    let c = p.clip(id).unwrap();
    assert!(c.opacity.link_err.is_none(), "{:?}", c.opacity.link_err);
    assert!((c.opacity.at(0.0) - 0.5).abs() < 1e-6 && (c.opacity.at(4.0) - 1.0).abs() < 1e-6);
    // a broken expression reports instead of animating, and never panics
    let c = p.clip_mut(id).unwrap();
    c.opacity.link = AnimLink::Expr("nonsense(".into());
    p.refresh_links();
    let c = p.clip(id).unwrap();
    assert!(c.opacity.link_err.is_some() && (c.opacity.at(2.0) - 0.5).abs() < 1e-6);
    // a manual edit breaks the link (AE-style), links survive a save/load round trip
    let c = p.clip_mut(id).unwrap();
    c.x.set_at(0.0, 7.0);
    assert!(c.x.link.is_none() && c.x.baked.is_empty());
    let mut o = Project::from_json(&p.to_json()).unwrap();
    assert_eq!(o.clip(id).unwrap().y.link, AnimLink::PathY(pid));
    o.refresh_links();
    assert!((o.clip(id).unwrap().y.at(4.0) - 50.0).abs() < 1e-6);
}

#[test]
fn trims() {
    let mut c = Clip::new(1, ClipKind::Video, "c", 2.0, 5.0);
    c.src_in = 1.0;
    c.opacity.toggle_key(1.0);
    c.trim_start(0.0, 1.0); // headroom 1 s → clamped to start - 1 = 1.0
    assert!((c.start - 1.0).abs() < 1e-9);
    assert!((c.src_in).abs() < 1e-9);
    assert!((c.duration - 6.0).abs() < 1e-9);
    assert!((c.opacity.keys[0].t - 2.0).abs() < 1e-9);
    c.trim_end(100.0, 10.0 - c.src_in);
    assert!((c.duration - 10.0).abs() < 1e-9);
    // images/text are unbounded on both sides
    let mut img = Clip::new(2, ClipKind::Image, "i", 5.0, 5.0);
    img.trim_start(3.0, f64::INFINITY);
    assert!((img.start - 3.0).abs() < 1e-9);
    assert!((img.duration - 7.0).abs() < 1e-9);
}

#[test]
fn speed_reverse_freeze() {
    let mut p = Project::from_media(asset(0, 10.0, 1));
    let v = p.tracks[0].clips[0].id;
    // 2x: source window stays [0,10), duration halves, audio follows
    assert!(p.set_speed(&[v], 2.0, false));
    let c = p.clip(v).unwrap().clone();
    assert!((c.duration - 5.0).abs() < 1e-9);
    assert!((c.src_time(1.0) - 2.0).abs() < 1e-9);
    assert!((p.tracks[1].clips[0].duration - 5.0).abs() < 1e-9);
    assert!((p.max_clip_duration(&c) - 5.0).abs() < 1e-9);
    // split at 2 s: right half starts at source 4 s
    p.split_at(2.0, None);
    assert!((p.tracks[0].clips[1].src_in - 4.0).abs() < 1e-9);
    assert!((p.tracks[0].clips[1].src_time(3.0) - 6.0).abs() < 1e-9);
    // reverse the right half: t=2 shows source 10, t=5 shows source 4
    let r = p.tracks[0].clips[1].id;
    assert!(p.set_speed(&[r], 2.0, true));
    let c = p.clip(r).unwrap().clone();
    assert!((c.src_time(2.0) - 10.0).abs() < 1e-9);
    assert!((c.src_time(5.0) - 4.0).abs() < 1e-9);
    // reversed: head room is what is left after src_end (nothing), max duration adds src_in/speed
    assert!((p.head_room(&c)).abs() < 1e-9);
    assert!((p.max_clip_duration(&c) - 5.0).abs() < 1e-9);
    // extend the right edge by 1 s → earliest source time moves 2 s earlier
    let mut c2 = c.clone();
    c2.trim_end(6.0, p.max_clip_duration(&c));
    assert!((c2.src_in - 2.0).abs() < 1e-9);
    assert!((c2.src_time(6.0) - 2.0).abs() < 1e-9);
    // freeze at t=3 splits and holds source 8 (2x reversed clip: 10 - 1*2)
    let frozen = p.freeze_at(3.0, &[r]);
    assert_eq!(frozen.len(), 2); // video + linked audio
    let f = p.clip(frozen[0]).unwrap();
    assert_eq!(f.freeze, Some(8.0));
    assert!((f.src_time(4.0) - 8.0).abs() < 1e-9);
    assert!(p.max_clip_duration(f).is_infinite());
}

#[test]
fn keyframed_speed_ramps_src_time() {
    let mut c = Clip::new(1, ClipKind::Video, "v", 2.0, 4.0);
    c.speed_curve.keys =
        vec![Keyframe { t: 0.0, v: 1.0, ease: Ease::Linear }, Keyframe { t: 4.0, v: 3.0, ease: Ease::Linear }];
    assert!(c.is_retimed());
    // local 2 s at rate 2 → source 4 s; local 3 s at rate 2.5 → source 7.5 s
    assert!((c.src_time(4.0) - 4.0).abs() < 1e-9, "{}", c.src_time(4.0));
    assert!((c.src_time(5.0) - 7.5).abs() < 1e-9, "{}", c.src_time(5.0));
    // no keys → the constant speed still rules
    c.speed_curve.keys.clear();
    c.set_speed(2.0);
    assert!((c.src_time(4.0) - 4.0).abs() < 1e-9);
    assert!((c.speed_curve.value - 2.0).abs() < 1e-9);
}

#[test]
fn transitions_add_and_prune() {
    let mut p = Project::from_media(asset(0, 10.0, 1));
    let right = p.split_at(4.0, None)[0];
    let id = p.add_transition(right, TransitionKind::CrossFade, 1.0).unwrap();
    assert_eq!(p.tracks[0].transitions.len(), 1);
    assert_eq!(p.tracks[1].transitions.len(), 1); // mirrored on the linked audio cut
    let (tr, l, r) = p.tracks[0].transition_at(3.9).unwrap();
    let (l, r) = (l.unwrap(), r.unwrap());
    assert_eq!(tr.id, id);
    assert!(l.end() == r.start);
    let (cut, half) = tr.cut_half(Some(l), Some(r)).unwrap();
    assert!((tr.progress_at(cut, half, 3.5) - 0.0).abs() < 1e-9);
    assert!((tr.progress_at(cut, half, 4.5) - 1.0).abs() < 1e-9);
    assert!(p.tracks[0].transition_at(4.6).is_none());
    // no left neighbour → None
    let first = p.tracks[0].clips[0].id;
    assert!(p.add_transition(first, TransitionKind::Push, 1.0).is_none());
    // moving the right clip away invalidates the transition
    assert!(p.move_clips(&p.expand_links(&[right]), 2.0, 0, None));
    assert!(p.tracks[0].transitions.is_empty());
    assert!(p.tracks[1].transitions.is_empty());
}

#[test]
fn edge_transitions_need_no_neighbour() {
    let mut p = Project::from_media(asset(0, 10.0, 1));
    let lone = p.tracks[0].clips[0].id;
    let id = p.add_edge_transition(lone, TransitionKind::CrossFade, 2.0, false).unwrap();
    assert_eq!(p.tracks[1].transitions.len(), 1, "mirrored fade on the linked audio clip");
    assert_eq!(p.tracks[1].transitions[0].kind, TransitionKind::CrossFade);
    let (tr, l, r) = p.tracks[0].transition_at(0.5).unwrap();
    assert_eq!((tr.id, tr.edge), (id, TransitionEdge::In));
    assert!(l.is_none() && r.is_some_and(|c| c.id == lone));
    let (cut, h) = tr.cut_half(l, r).unwrap();
    assert!((cut - 1.0).abs() < 1e-9 && (h - 1.0).abs() < 1e-9, "window = first 2 s of the clip");
    assert!((tr.progress_at(cut, h, 0.0) - 0.0).abs() < 1e-9);
    assert!((tr.progress_at(cut, h, 2.0) - 1.0).abs() < 1e-9);
    assert!(p.tracks[0].transition_at(2.5).is_none());
    // Out edge at the clip end, and the window is clamped to the clip
    let out = p.add_edge_transition(lone, TransitionKind::FadeToColor, 30.0, true).unwrap();
    let (tr, l, r) = p.tracks[0].transition_at(9.9).unwrap();
    assert_eq!(tr.id, out);
    assert!(l.is_some_and(|c| c.id == lone) && r.is_none());
    let (cut, h) = tr.cut_half(l, r).unwrap();
    assert!((cut - 5.0).abs() < 1e-9 && (h - 5.0).abs() < 1e-9, "clamped to the 10 s clip");
    // both edges live on one clip; pruning keeps them while the clip exists
    assert_eq!(p.tracks[0].transitions.len(), 2);
    p.tracks[0].prune_transitions();
    assert_eq!(p.tracks[0].transitions.len(), 2);
}

#[test]
fn a_cut_and_an_in_share_the_start_slot() {
    let mut p = Project::from_media(asset(0, 10.0, 1));
    let right = p.split_at(4.0, None)[0];
    let left = p.tracks[0].clips[0].id;
    p.add_edge_transition(right, TransitionKind::CrossFade, 1.0, false).unwrap();
    // a Cut on the same clip's start replaces the In (same spot on the track)
    p.add_transition(right, TransitionKind::Wipe, 1.0).unwrap();
    assert_eq!(p.tracks[0].transitions.len(), 1);
    assert_eq!(p.tracks[0].transitions[0].edge, TransitionEdge::Cut);
    // ... and the previous clip's Out replaces that Cut in turn (same spot again)
    p.add_edge_transition(left, TransitionKind::CrossFade, 1.0, true).unwrap();
    assert_eq!(p.tracks[0].transitions.len(), 1);
    assert_eq!(p.tracks[0].transitions[0].edge, TransitionEdge::Out);
}

#[test]
fn fade_mult_ramps_and_clamps() {
    let mut c = Clip::new(1, ClipKind::Video, "v", 0.0, 10.0);
    c.fade_in = 2.0;
    c.fade_out = 4.0;
    assert!((c.fade_mult(-1.0) - 0.0).abs() < 1e-9, "virtual extension holds the edge value");
    assert!((c.fade_mult(1.0) - 0.5).abs() < 1e-9);
    assert!((c.fade_mult(5.0) - 1.0).abs() < 1e-9);
    assert!((c.fade_mult(8.0) - 0.5).abs() < 1e-9);
    assert!((c.fade_mult(11.0) - 0.0).abs() < 1e-9);
}

#[test]
fn effects_params_and_keys() {
    let mut c = Clip::new(1, ClipKind::Video, "c", 0.0, 4.0);
    c.effects.push(Effect::new(EffectKind::Blur));
    assert_eq!(c.effects[0].at(0, 1.0), 8.0);
    c.effects[0].params[0].toggle_key(1.0);
    c.effects[0].params[0].set_at(3.0, 20.0);
    assert_eq!(c.key_times(), vec![1.0, 3.0]);
    c.move_keys(3.0, 2.0);
    assert_eq!(c.key_times(), vec![1.0, 2.0]);
    assert!(c.has_effects());
    let json = serde_json::to_string(&c).unwrap();
    let d: Clip = serde_json::from_str(&json).unwrap();
    assert_eq!(d.effects[0].kind, EffectKind::Blur);
}

/// An old project file, saved before `scale_x`/`scale_y` existed, has no such keys in its JSON.
/// They must deserialize as the no-op multiplier (1.0) so old projects render unchanged.
#[test]
fn scale_x_y_default_to_noop_for_old_projects() {
    let json = r#"{"id":1,"kind":"Video","name":"c","start":0.0,"duration":4.0,"scale":{"value":2.0}}"#;
    let c: Clip = serde_json::from_str(json).unwrap();
    assert_eq!(c.scale.value, 2.0, "old field is untouched");
    assert_eq!(c.scale_x.value, 1.0);
    assert_eq!(c.scale_y.value, 1.0);
}

/// An old project file, saved before `icon`/`sequence` existed, has no such keys. They must
/// deserialize as the flag glyph / main timeline, and roundtrip once set.
#[test]
fn marker_icon_and_sequence_default_for_old_projects() {
    let json = r#"{"id":1,"t":3.0,"name":"m"}"#;
    let m: Marker = serde_json::from_str(json).unwrap();
    assert_eq!(m.icon, "flag");
    assert_eq!(m.sequence, None);

    let m2 = Marker { sequence: Some(7), icon: "star".into(), ..m };
    let json = serde_json::to_string(&m2).unwrap();
    assert_eq!(serde_json::from_str::<Marker>(&json).unwrap(), m2);
}

#[test]
fn add_marker_stamps_current_editing_sequence() {
    let mut p = Project::new();
    let main = p.add_marker(1.0, "main");
    assert_eq!(p.marker_mut(main).unwrap().sequence, None);
    p.editing = Some(99);
    let seq = p.add_marker(2.0, "seq");
    assert_eq!(p.marker_mut(seq).unwrap().sequence, Some(99));
}

#[test]
fn snap_marker_to_nearest_clip_moves_to_clip_start() {
    let mut p = Project::new();
    p.tracks[0].clips.push(Clip::new(1, ClipKind::Video, "a", 0.0, 2.0));
    p.tracks[0].clips.push(Clip::new(2, ClipKind::Video, "b", 5.0, 2.0));
    let mid = p.add_marker(4.0, "m");
    p.snap_marker_to_nearest_clip(mid);
    assert_eq!(p.marker_mut(mid).unwrap().t, 5.0, "closer to clip b's start than clip a's");
}

#[test]
fn link_marker_to_closest_clip_converts_to_clip_local() {
    let mut p = Project::new();
    p.tracks[0].clips.push(Clip::new(1, ClipKind::Video, "a", 10.0, 4.0));
    let mid = p.add_marker(11.5, "beat");
    p.marker_mut(mid).unwrap().note = "hit".into();
    assert!(p.link_marker_to_closest_clip(mid));
    assert!(p.markers.is_empty(), "removed from the project list");
    let c = p.clip(1).unwrap();
    assert_eq!(c.markers.len(), 1);
    assert_eq!(c.markers[0].id, mid, "same id, just re-homed");
    assert_eq!(c.markers[0].t, 1.5, "converted to clip-local time");
    assert_eq!(c.markers[0].note, "hit", "note is preserved");
}

#[test]
fn link_marker_to_closest_clip_noop_without_clips() {
    let mut p = Project::new(); // default tracks have no clips
    let mid = p.add_marker(1.0, "m");
    assert!(!p.link_marker_to_closest_clip(mid));
    assert_eq!(p.markers.len(), 1, "left alone");
}

/// An old project file, saved before `preview_bg` existed, has no such key. It must deserialize as
/// `Black` so old projects (and a brand-new one) render exactly as before this setting existed.
#[test]
fn preview_bg_defaults_to_black_for_old_projects() {
    let p = Project::from_json("{}").unwrap();
    assert_eq!(p.preview_bg, BackgroundMode::Black);
    assert_eq!(Project::new().preview_bg, BackgroundMode::Black);
}

#[test]
fn background_mode_roundtrips_through_json() {
    for m in [
        BackgroundMode::Checkerboard,
        BackgroundMode::Black,
        BackgroundMode::White,
        BackgroundMode::Custom([10, 20, 30, 255]),
    ] {
        let json = serde_json::to_string(&m).unwrap();
        assert_eq!(serde_json::from_str::<BackgroundMode>(&json).unwrap(), m);
    }
}

/// Every current kind is a pixel/GLSL effect — none apply to an audio clip yet (see the doc comment
/// on `applies_to_audio`). This pins that so the effects panel's audio filter stays correct rather
/// than silently drifting if a kind's classification is ever meant to change.
#[test]
fn no_effect_kind_applies_to_audio_yet() {
    assert!(EffectKind::ALL.iter().all(|k| !k.applies_to_audio()));
}

// ---- ws:color-engine ----
// deviation (see PR body): the plan named `src/model/effect.rs (tests mod)` as the home for these —
// post split-god-files, model-level tests are centralized in this file (`#[cfg(test)] mod tests;` in
// model/mod.rs, `use super::*;`), and effect.rs itself has no `#[cfg(test)] mod tests` of its own (see
// `no_effect_kind_applies_to_audio_yet` just above, already living here for the same reason).

/// `EffectKind::ALL` grows to 29 with the 4 new kinds inserted before `Shader`; every dispatch fn stays
/// total (compiles) for every kind; `needs_motion()` is true for exactly `MotionBlur`/`FrameBlend`.
#[test]
fn effect_kind_all_has_29_entries_in_declared_order() {
    assert_eq!(EffectKind::ALL.len(), 29);
    let shader_i = EffectKind::ALL.iter().position(|&k| k == EffectKind::Shader).unwrap();
    for k in [EffectKind::Primaries, EffectKind::Qualifier, EffectKind::Lut, EffectKind::FrameBlend] {
        let i = EffectKind::ALL.iter().position(|&x| x == k).unwrap();
        assert!(i < shader_i, "{k:?} must sit before Shader in ALL");
    }
    for k in EffectKind::ALL {
        let _ = (k.category(), k.applies_to_audio(), k.params(), k.name()); // total (compiles + runs)
    }
    for k in EffectKind::ALL {
        let want = matches!(k, EffectKind::MotionBlur | EffectKind::FrameBlend);
        assert_eq!(k.needs_motion(), want, "{k:?}");
    }
}

/// Documents the PARAM_NAMES budget explicitly (gpu.rs's fixed p0..p11 uniform slots): Primaries'
/// natural 12 knobs drop a redundant per-channel Offset to fit in 11, with one to spare.
#[test]
fn primaries_param_count_fits_param_names() {
    assert!(EffectKind::Primaries.params().len() <= 12);
}

/// Regression pin for the `needs_motion()` fix: without `FrameBlend` here, `gpu.rs`'s run_effect/
/// run_chain/eval_graph_on and playback.rs's motion-sample decode all silently stay MotionBlur-only and
/// FrameBlend renders through the shader's always-zero-`u_frames` fallback.
#[test]
fn needs_motion_gates_frame_blend() {
    assert!(EffectKind::FrameBlend.needs_motion());
    assert!(EffectKind::MotionBlur.needs_motion());
    assert!(!EffectKind::Primaries.needs_motion());
}

#[test]
fn subtitles_and_folders() {
    let mut p = Project::new();
    p.add_cue(2.0, 3.0, "b");
    p.add_cue(0.0, 1.0, "a");
    assert_eq!(p.subtitles[0].text, "a");
    assert_eq!(p.cue_at(2.5).unwrap().text, "b");
    assert!(p.cue_at(1.5).is_none());
    assert!(p.add_folder("Footage/Day 1"));
    assert!(!p.add_folder("  "));
    let aid = p.add_asset(asset(0, 1.0, 0));
    p.asset_mut(aid).unwrap().folder = "Footage/Day 1".into();
    assert_eq!(p.folder_names(), vec!["Footage/Day 1".to_string()]);
    p.remove_folder("Footage");
    assert!(p.folders.is_empty());
    assert_eq!(p.asset(aid).unwrap().folder, "");
}

#[test]
fn move_many() {
    let mut p = Project::new();
    let ids: Vec<Id> = (0..4).map(|i| p.add_text_clip(i as f64 * 2.0, 2.0)).collect(); // V1: [0,2)[2,4)[4,6)[6,8)
    let blocker = p.add_text_clip(10.0, 1.0);
    assert!(p.move_clips(&ids, 1.0, 0, None)); // adjacent moved clips don't block each other
    assert!(!p.move_clips(&ids, 2.0, 0, None)); // last one would hit the blocker
    assert!(!p.move_clips(&[ids[1]], 1.0, 0, None)); // onto a stationary clip
    assert!((p.clip(ids[0]).unwrap().start - 1.0).abs() < 1e-9);
    assert!((p.clip(blocker).unwrap().start - 10.0).abs() < 1e-9);
}

#[test]
fn from_json_sanitizes() {
    let mut p = Project::from_media(asset(0, 10.0, 1));
    p.add_text_clip(12.0, 1.0);
    let s = p.to_json().replace("\"fps\": 30.0", "\"fps\": 0.0").replace("\"width\": 1280", "\"width\": 0");
    let q = Project::from_json(&s).unwrap();
    assert_eq!(q.fps, 30.0);
    assert!(q.width >= 16);
    assert!(q.snap_frame(1.0).is_finite());
    let mut bad = Project::new();
    bad.tracks[0].clips.push(Clip::new(99, ClipKind::Text, "t", 0.0, -1.0));
    assert!(Project::from_json(&bad.to_json()).unwrap().is_empty());
}

#[test]
fn sequences_open_close_nest() {
    let mut p = Project::from_media(asset(0, 10.0, 1));
    let v = p.tracks[0].clips[0].id;
    // nest the whole clip (+ audio) into a sequence
    let seq = p.nest_selection(&[v], "Intro").unwrap();
    assert_eq!(p.sequences.len(), 1);
    assert_eq!(p.sequence(seq).unwrap().tracks.len(), 2);
    assert_eq!(p.tracks[0].clips.len(), 1);
    assert_eq!(p.tracks[0].clips[0].kind, ClipKind::Sequence);
    assert!((p.sequence_duration(seq) - 10.0).abs() < 1e-9);
    assert!((p.duration() - 10.0).abs() < 1e-9);
    assert!(p.used_assets().len() == 1); // the asset is used inside the sequence
                                         // open the sequence for editing: tracks swap, main is stashed
    assert!(p.open_sequence(seq));
    assert_eq!(p.editing, Some(seq));
    assert_eq!(p.tracks[0].clips[0].kind, ClipKind::Video);
    assert!(p.main_stash.is_some());
    // can't place itself inside itself
    assert!(p.insert_sequence_clip(seq, 0.0, None).is_none());
    p.close_sequence();
    assert_eq!(p.editing, None);
    assert_eq!(p.tracks[0].clips[0].kind, ClipKind::Sequence);
    // round trip keeps everything
    let q = Project::from_json(&p.to_json()).unwrap();
    assert_eq!(q.sequences.len(), 1);
    assert!((q.sequence_duration(seq) - 10.0).abs() < 1e-9);
}

#[test]
fn planner_and_unused() {
    let mut p = Project::from_media(asset(0, 10.0, 1));
    let extra = p.add_asset(asset(1, 3.0, 0));
    let third = p.add_asset(asset(2, 3.0, 0));
    let a = p.plan_add(None, "Intro");
    let b = p.plan_add(Some(a), "Hook shot");
    p.plan_item_mut(b).unwrap().assets.push(extra);
    p.plan_item_mut(b).unwrap().done = true;
    assert_eq!(p.plan[0].children[0].title, "Hook shot");
    assert!(p.plan_assets().contains(&extra));
    assert_eq!(p.remove_unused_assets(), 1); // `third` gone, `extra` kept by the moodboard
    assert!(p.asset(extra).is_some() && p.asset(third).is_none());
    p.plan_remove(a);
    assert!(p.plan.is_empty());
}

/// The standalone Moodboard pane's assets are protected from cleanup the same way the planner's
/// per-task moodboards already are.
#[test]
fn moodboard_assets_protected_from_cleanup() {
    let mut p = Project::from_media(asset(0, 10.0, 1));
    let kept = p.add_asset(asset(1, 3.0, 0));
    let unused = p.add_asset(asset(2, 3.0, 0));
    p.moodboard.push(MoodItem { asset: kept, labels: vec!["hero".into()] });
    assert!(p.plan_assets().contains(&kept));
    assert_eq!(p.remove_unused_assets(), 1);
    assert!(p.asset(kept).is_some() && p.asset(unused).is_none());
}

/// An old project file (`notes` was one flat string) migrates a non-empty value into a single
/// untitled note with a real id; an old empty string, or a missing key, migrates to no notes at all.
#[test]
fn notes_migrate_from_old_string_format() {
    let p = Project::from_json(r#"{"notes":"describe the process here"}"#).unwrap();
    assert_eq!(p.notes.len(), 1);
    assert_eq!(p.notes[0].title, "");
    assert_eq!(p.notes[0].body, "describe the process here");
    assert_ne!(p.notes[0].id, 0, "the placeholder id must be fixed up to a real one");

    let p = Project::from_json(r#"{"notes":""}"#).unwrap();
    assert!(p.notes.is_empty());
    let p = Project::from_json("{}").unwrap();
    assert!(p.notes.is_empty());
}

/// The current list-of-notes format round-trips (and a fresh project starts with none).
#[test]
fn notes_add_edit_roundtrip() {
    let mut p = Project::new();
    assert!(p.notes.is_empty());
    let id = p.add_note("Style");
    p.note_mut(id).unwrap().body = "**bold** ideas".into();
    p.note_mut(id).unwrap().label = 2;
    let mut q = Project::from_json(&p.to_json()).unwrap();
    assert_eq!(q.notes.len(), 1);
    assert_eq!(q.notes[0].title, "Style");
    assert_eq!(q.notes[0].body, "**bold** ideas");
    assert_eq!(q.notes[0].label, 2);
    q.remove_note(id);
    assert!(q.notes.is_empty());
}

/// `requirements` / `tracked_seconds` (added for the Timer tab and the compact checklist) round-trip
/// and default to empty/zero for a plan item written before either field existed.
#[test]
fn plan_item_requirements_and_tracked_seconds_roundtrip() {
    let mut p = Project::new();
    let id = p.plan_add(None, "Edit intro");
    {
        let it = p.plan_item_mut(id).unwrap();
        it.requirements.push(("Colour graded".into(), false));
        it.requirements.push(("Music licensed".into(), true));
        it.tracked_seconds = 125.5;
    }
    let q = Project::from_json(&p.to_json()).unwrap();
    assert_eq!(
        q.plan[0].requirements,
        vec![("Colour graded".to_string(), false), ("Music licensed".to_string(), true)]
    );
    assert_eq!(q.plan[0].tracked_seconds, 125.5);

    // an old plan item JSON without either field defaults to empty/zero
    let old = Project::from_json(
            r#"{"plan":[{"id":1,"title":"Old task","done":false,"notes":"","color":0,"assets":[],"asset_notes":[],"children":[]}]}"#,
        )
        .unwrap();
    assert!(old.plan[0].requirements.is_empty());
    assert_eq!(old.plan[0].tracked_seconds, 0.0);
}

#[test]
fn auto_cut_removes_quiet_parts() {
    let mut p = Project::from_media(asset(0, 10.0, 1));
    let a = p.tracks[1].clips[0].id;
    // cuts at 2,4,6,8; remove [0,2) and [4,6) and [8,10) → keeps [2,4) and [6,8), rippled together
    let n = p.auto_cut(&[a], &[2.0, 4.0, 6.0, 8.0], &[(0.0, 2.0), (4.0, 6.0), (8.0, 10.0)], true);
    assert_eq!(n, 6); // 3 audio + 3 linked video pieces
    assert_eq!(p.tracks[1].clips.len(), 2);
    assert_eq!(p.tracks[0].clips.len(), 2);
    assert!((p.duration() - 4.0).abs() < 1e-9);
    assert!((p.tracks[0].clips[1].src_in - 6.0).abs() < 1e-9);
}

#[test]
fn flow_and_place() {
    let mut p = Project::new();
    let a = p.add_text_clip(0.0, 2.0);
    let b = p.add_text_clip(2.0, 2.0);
    p.clip_mut(a).unwrap().x.toggle_key(0.0);
    p.clip_mut(a).unwrap().x.set_at(2.0, 100.0);
    p.clip_mut(b).unwrap().x.toggle_key(0.0);
    p.clip_mut(b).unwrap().x.set_at(2.0, -100.0); // b starts at 0 → after flow both meet at 50
    assert!(p.flow_clips(a, b));
    assert!((p.clip(a).unwrap().x.at(2.0) - 50.0).abs() < 1e-9);
    assert!((p.clip(b).unwrap().x.at(0.0) - 50.0).abs() < 1e-9);
    assert_eq!(p.clip(b).unwrap().x.keys[0].ease, Ease::EaseOut);
    // place the two clips again as a template at t=10
    let clips: Vec<Clip> = [a, b].iter().map(|&id| p.clip(id).unwrap().clone()).collect();
    let ids = p.place_clips(clips, Vec::new(), 10.0);
    assert_eq!(ids.len(), 2);
    assert!((p.clip(ids[0]).unwrap().start - 10.0).abs() < 1e-9);
    assert_eq!(p.tracks[0].clips.len(), 4);
}

#[test]
fn bezier_ease() {
    let e = Ease::Bezier { x1: 0.42, y1: 0.0, x2: 0.58, y2: 1.0 };
    assert!((e.apply(0.0)).abs() < 1e-6 && (e.apply(1.0) - 1.0).abs() < 1e-6);
    assert!((e.apply(0.5) - 0.5).abs() < 1e-3);
    assert!(e.apply(0.25) < 0.25); // ease-in start
    let s = serde_json::to_string(&e).unwrap();
    assert_eq!(serde_json::from_str::<Ease>(&s).unwrap(), e);
}

#[test]
fn save_is_atomic() {
    let dir = std::env::temp_dir().join("simple-editor-model-test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("p.sedit");
    let p = Project::from_media(asset(0, 10.0, 1));
    p.save(&path).unwrap();
    p.save(&path).unwrap(); // overwrites
    assert!(!dir.join("p.sedit.tmp").exists());
    assert_eq!(Project::load(&path).unwrap().to_json(), p.to_json());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unlink_graph_round_trips_the_effect_stack() {
    let mut p = Project::from_media(asset(0, 10.0, 1));
    let id = p.tracks[0].clips[0].id;
    let mut blur = Effect::new(EffectKind::Blur);
    blur.params[0].value = 7.0;
    let mut off = Effect::new(EffectKind::Sharpen);
    off.enabled = false;
    p.clip_mut(id).unwrap().effects = vec![blur, off];
    p.ensure_graph(id);
    assert_eq!(p.unlink_graph(id), Ok(2));
    let c = p.clip(id).unwrap();
    assert!(c.graph.is_none(), "the graph is gone");
    // order, parameters and the disabled flag survive the round trip
    assert_eq!(c.effects.iter().map(|e| e.kind).collect::<Vec<_>>(), vec![EffectKind::Blur, EffectKind::Sharpen]);
    assert_eq!(c.effects[0].at(0, 0.0), 7.0);
    assert!(c.effects[0].enabled && !c.effects[1].enabled);

    // a branch (Blend pulling in a second source) has no flat equivalent, but unlink still means the
    // graph is gone: the clip keeps the effects it already had rather than staying on the nodes
    p.ensure_graph(id);
    let g = p.clip(id).unwrap().graph.clone().unwrap();
    let (input, out) = (g.nodes[0].id, g.output().unwrap());
    let blend = p.add_node(id, NodeKind::Blend { mode: BlendMode::Normal, opacity: Animated::new(1.0) }, 0.0, 0.0);
    let color = p.add_node(id, NodeKind::Color([255, 0, 0, 255]), 0.0, 0.0).unwrap();
    let blend = blend.unwrap();
    let g = p.clip_mut(id).unwrap().graph.as_mut().unwrap();
    assert!(g.connect(input, blend, 0) && g.connect(color, blend, 1) && g.connect(blend, out, 0));
    let kept: Vec<EffectKind> = p.clip(id).unwrap().effects.iter().map(|e| e.kind).collect();
    assert_eq!(p.unlink_graph(id), Ok(0), "nothing of a branch converts");
    assert!(p.clip(id).unwrap().graph.is_none(), "but the graph is gone either way");
    assert_eq!(
        p.clip(id).unwrap().effects.iter().map(|e| e.kind).collect::<Vec<_>>(),
        kept,
        "and the clip keeps the effect stack it already had"
    );
    // unlinking a clip that never had a graph is still an error
    assert!(p.unlink_graph(id).is_err(), "no graph, nothing to unlink");
}

/// Undo restores the project by parsing a JSON snapshot, and silently does nothing when that parse
/// fails — so every shape the tools can make must survive a round trip, signs and all.
#[test]
fn every_shape_kind_round_trips_through_json() {
    let mut p = Project::new();
    for kind in [
        ShapeKind::Rect,
        ShapeKind::Ellipse,
        ShapeKind::Triangle,
        ShapeKind::Polygon,
        ShapeKind::Star,
        ShapeKind::Line,
        ShapeKind::Arrow,
        ShapeKind::Draw,
    ] {
        let id = p.add_shape_clip(kind, 0.0, 4.0);
        // a line dragged up-and-right: the extents carry the direction, so they are negative
        if let Some(sh) = p.clip_mut(id).and_then(|c| c.shape.as_mut()) {
            sh.w.value = 60.0;
            sh.h.value = -40.0;
        }
    }
    let json = p.to_json();
    let back = Project::from_json(&json).expect("a project full of shapes must parse back");
    let kinds: Vec<ShapeKind> = back.all_clips().filter_map(|(_, c)| c.shape.as_ref().map(|s| s.kind)).collect();
    assert_eq!(kinds.len(), 8, "every shape came back: {kinds:?}");
    let h: Vec<f64> = back.all_clips().filter_map(|(_, c)| c.shape.as_ref().map(|s| s.h.value)).collect();
    assert!(h.iter().all(|v| *v == -40.0), "the negative extent survived: {h:?}");
}

/// An effect covers the whole clip until it is given a window, and a file written before the window
/// existed deserialises to exactly that.
#[test]
fn effect_window_defaults_to_the_whole_clip() {
    let mut e = Effect::new(EffectKind::Blur);
    assert!(e.on_at(0.0) && e.on_at(1000.0));
    e.start = 1.0;
    e.len = 2.0;
    assert!(!e.on_at(0.5) && e.on_at(1.0) && e.on_at(3.0) && !e.on_at(3.5));
    e.enabled = false;
    assert!(!e.on_at(2.0), "disabled beats the window");

    let old: Effect = serde_json::from_str(r#"{"kind":"Blur","params":[]}"#).unwrap();
    assert!(old.on_at(0.0) && old.on_at(1e6), "an old project keeps whole-clip effects");
}

/// The node editor is opt-in: a bare Input→Output graph must not shadow the clip's effect list.
#[test]
fn a_bare_graph_does_not_take_over_the_effect_stack() {
    let mut p = Project::from_media(asset(0, 10.0, 1));
    let id = p.tracks[0].clips[0].id;
    p.ensure_graph(id);
    assert_eq!(p.clip(id).unwrap().graph.as_ref().unwrap().nodes.len(), 2);
    assert!(!p.clip(id).unwrap().uses_graph(), "Input→Output says nothing the stack does not");
    assert!(p.add_node(id, NodeKind::Effect(Effect::new(EffectKind::Blur)), 0.0, 0.0).is_some());
    assert!(p.clip(id).unwrap().uses_graph(), "a real node makes the graph the renderer's truth");
}

#[test]
fn text_node_format_expands() {
    // 25 fps: t = 2 s is timeline frame 50, 1 s into the clip is counter 25
    assert_eq!(expand_text("{n} / {frame}", 2.0, 1.0, 25.0), "25 / 50");
    assert_eq!(expand_text("{time}", 61.5, 0.0, 25.0), "01:01.50");
    assert_eq!(expand_text("{time}", 3661.0, 0.0, 25.0), "1:01:01.00");
    // nonsense fps falls back, unknown braces and negative times are left alone
    assert_eq!(expand_text("{frame} {x}", 1.0, 0.0, 0.0), "30 {x}");
    assert_eq!(expand_text("{n}", 0.0, -5.0, 30.0), "0");
}

/// A bare node straight into a graph (the editor goes through `Project::add_node`).
fn push(g: &mut NodeGraph, id: Id, kind: NodeKind) -> Id {
    g.nodes.push(Node { id, kind, x: 0.0, y: 0.0, enabled: true });
    id
}

#[test]
fn the_logic_nodes_evaluate_as_a_chain() {
    let mut next = 100;
    let mut nid = || {
        next += 1;
        next
    };
    let mut g = NodeGraph::new(&mut nid);
    let out = g.output().unwrap();
    let a = push(&mut g, nid(), NodeKind::Number(Animated::new(3.0)));
    let b = push(&mut g, nid(), NodeKind::Number(Animated::new(4.0)));
    let sum = push(&mut g, nid(), NodeKind::Math(MathOp::Add));
    let gt = push(&mut g, nid(), NodeKind::Compare(CmpOp::Gt));
    let not = push(&mut g, nid(), NodeKind::Logic(LogicOp::Not));
    let sel = push(&mut g, nid(), NodeKind::Select);
    assert!(g.connect(a, sum, 0) && g.connect(b, sum, 1));
    assert!(g.connect(sum, gt, 0) && g.connect(b, gt, 1)); // 7 > 4
    assert!(g.connect(gt, not, 0));
    assert!(g.connect(not, sel, 0) && g.connect(a, sel, 1) && g.connect(b, sel, 2));
    assert!(g.connect(sel, out, 0));
    let v = g.eval_values(0.0, 30.0);
    assert_eq!(v[&sum], 7.0);
    assert_eq!(v[&gt], 1.0);
    assert_eq!(v[&not], 0.0);
    assert_eq!(v[&sel], 4.0, "the condition was negated, so Select takes b");
    // flip the comparison and the switch follows
    g.node_mut(gt).unwrap().kind = NodeKind::Compare(CmpOp::Lt);
    let v = g.eval_values(0.0, 30.0);
    assert_eq!(v[&sel], 3.0);
    // only what the Output reads is evaluated, exactly like the picture side
    let loose = push(&mut g, nid(), NodeKind::Number(Animated::new(9.0)));
    assert!(!g.eval_values(0.0, 30.0).contains_key(&loose));
    // a division by zero is 0, not a NaN loose in the render
    g.node_mut(sum).unwrap().kind = NodeKind::Math(MathOp::Div);
    g.disconnect(sum, 1);
    assert_eq!(g.eval_values(0.0, 30.0)[&sum], 0.0);
}

#[test]
fn random_is_seeded_so_a_render_repeats() {
    let mut next = 200;
    let mut nid = || {
        next += 1;
        next
    };
    let mut g = NodeGraph::new(&mut nid);
    let out = g.output().unwrap();
    let r = push(&mut g, nid(), NodeKind::Random { seed: 7, min: -1.0, max: 1.0 });
    assert!(g.connect(r, out, 0));
    let first = g.eval_values(0.5, 30.0)[&r];
    assert_eq!(first, g.eval_values(0.5, 30.0)[&r], "same seed, same frame, same number");
    assert!((-1.0..=1.0).contains(&first), "{first} is outside min..max");
    assert_ne!(first, g.eval_values(1.5, 30.0)[&r], "a different frame draws again");
    g.node_mut(r).unwrap().kind = NodeKind::Random { seed: 8, min: -1.0, max: 1.0 };
    assert_ne!(first, g.eval_values(0.5, 30.0)[&r], "a different seed is a different sequence");
}

#[test]
fn an_old_text_node_loads_as_a_string_node() {
    let k = NodeKind::String(TextStyle { text: "hello".into(), ..Default::default() });
    let json = serde_json::to_string(&k).unwrap();
    assert!(json.starts_with("{\"String\""), "{json}");
    let old = json.replacen("\"String\"", "\"Text\"", 1);
    assert_eq!(serde_json::from_str::<NodeKind>(&old).unwrap(), k, "projects from before the rename still load");
}

#[test]
fn an_effect_node_takes_one_value_port_per_parameter() {
    let fx = NodeKind::Effect(Effect::new(EffectKind::Tint));
    assert_eq!(fx.inputs(), 1 + EffectKind::Tint.params().len());
    assert_eq!(fx.port_label(0), "in");
    assert_eq!(fx.port_label(1), EffectKind::Tint.params()[0].name);
    // Not reads one input, the other logic nodes two
    assert_eq!(NodeKind::Logic(LogicOp::Not).inputs(), 1);
    assert_eq!(NodeKind::Logic(LogicOp::And).inputs(), 2);
    assert_eq!(NodeKind::Select.port_label(0), "cond");
}
