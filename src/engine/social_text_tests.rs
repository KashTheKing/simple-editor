//! ---- ws:social-text ----
//! The TikTok / Snapchat text looks: per-line box geometry, the full-width caption bar, the presets'
//! save / load round trip, a rendered frame of each preset against what it should look like, and the
//! emulator's safe zones.

use crate::engine::compose::Compositor;
use crate::engine::presets::{
    builtin_caption_styles, builtin_titles, decode_template, social_clip, social_style, social_title_names,
};
use crate::engine::text::{box_mask, line_boxes, BoxRect, TextRasterizer};
use crate::media::{Backend, DecoderPool, Frame};
use crate::model::{Project, TextStyle, BOX_BAR, BOX_BLOCK, BOX_LINES};

fn fonts_or_skip(tr: &mut TextRasterizer) -> bool {
    tr.load_system_fonts();
    // the bundled fonts are always there, system fonts or not
    tr.families().iter().any(|f| f == "TikTok Sans") && tr.families().iter().any(|f| f == "Inter")
}

fn px(f: &Frame, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * f.width + x) * 4) as usize;
    [f.rgba[i], f.rgba[i + 1], f.rgba[i + 2], f.rgba[i + 3]]
}

/// The boxes (not fillets) of a `line_boxes` result.
fn plain(b: &[BoxRect]) -> Vec<BoxRect> {
    b.iter().copied().filter(|r| r.r.iter().all(|&v| v >= 0.0)).collect()
}

fn fillets(b: &[BoxRect]) -> Vec<BoxRect> {
    b.iter().copied().filter(|r| r.r.iter().any(|&v| v < 0.0)).collect()
}

// ---- per-line box geometry ----

#[test]
fn one_line_box_hugs_the_line_with_round_corners() {
    let b = line_boxes(&[Some((100.0, 300.0))], 50.0, 80.0, 70.0, 10.0, 12.0);
    assert_eq!(b.len(), 1, "one box, no fillets");
    let r = b[0];
    assert_eq!((r.x0, r.x1), (90.0, 310.0), "the line's width plus the padding each side");
    assert_eq!((r.y0, r.y1), (40.0, 130.0), "the glyph band plus the padding");
    assert_eq!(r.r, [12.0; 4], "a lone line is rounded at every corner");
}

#[test]
fn two_uneven_lines_hug_each_width_and_merge() {
    // a short line over a long one (centred text)
    let spans = [Some((150.0, 250.0)), Some((100.0, 300.0))];
    let b = line_boxes(&spans, 50.0, 80.0, 70.0, 10.0, 12.0);
    let p = plain(&b);
    assert_eq!(p.len(), 2);
    assert_eq!((p[0].x0, p[0].x1), (140.0, 260.0), "top box hugs the short line");
    assert_eq!((p[1].x0, p[1].x1), (90.0, 310.0), "bottom box hugs the long line");
    assert_eq!(p[0].y1, p[1].y0, "the boxes meet on one row");
    assert_eq!(p[0].y1, p[0].y1.round(), "on a whole pixel, so no seam shows");
    // the short line's bottom corners are square (they sit on the long box), its top ones round;
    // the long line's top corners stick out, so they stay round
    assert_eq!(p[0].r, [12.0, 12.0, 0.0, 0.0]);
    assert_eq!(p[1].r, [12.0, 12.0, 12.0, 12.0]);
    // a concave fillet each side, above the shared row, against the short box
    let f = fillets(&b);
    assert_eq!(f.len(), 2, "{f:?}");
    for fl in &f {
        assert_eq!(fl.y1, p[0].y1, "fillets end on the shared row");
        assert!((fl.y1 - fl.y0 - 12.0).abs() < 1e-4, "fillet radius = corner radius");
    }
    assert!(f.iter().any(|fl| fl.x1 == p[0].x0), "left fillet against the short box");
    assert!(f.iter().any(|fl| fl.x0 == p[0].x1), "right fillet against the short box");
}

#[test]
fn three_lines_narrow_wide_narrow() {
    // left-aligned: every line starts at x 100, widths 120 / 300 / 60
    let spans = [Some((100.0, 220.0)), Some((100.0, 400.0)), Some((100.0, 160.0))];
    let b = line_boxes(&spans, 40.0, 70.0, 60.0, 8.0, 10.0);
    let p = plain(&b);
    assert_eq!(p.len(), 3);
    for (i, s) in spans.iter().enumerate() {
        let (x0, x1) = s.unwrap();
        assert_eq!((p[i].x0, p[i].x1), (x0 - 8.0, x1 + 8.0), "line {i} hugs its own width");
    }
    assert_eq!(p[0].y1, p[1].y0);
    assert_eq!(p[1].y1, p[2].y0);
    // left edges line up: the left corners between lines are square, the outer ones round
    assert_eq!(p[0].r[0], 10.0);
    assert_eq!((p[0].r[3], p[1].r[0], p[1].r[3], p[2].r[0]), (0.0, 0.0, 0.0, 0.0));
    assert_eq!(p[2].r[3], 10.0);
    // right side: the middle line sticks out past both neighbours - round there, fillets on them
    assert_eq!((p[1].r[1], p[1].r[2]), (10.0, 10.0));
    assert_eq!((p[0].r[2], p[2].r[1]), (0.0, 0.0));
    let f = fillets(&b);
    assert_eq!(f.len(), 2, "one fillet under line 1's right end, one over line 3's: {f:?}");
    assert!(f.iter().any(|fl| fl.x0 == p[0].x1 && fl.y1 == p[0].y1));
    assert!(f.iter().any(|fl| fl.x0 == p[2].x1 && fl.y0 == p[2].y0));
}

#[test]
fn a_small_width_difference_shrinks_the_corner_to_fit() {
    // lines 6 px apart in width at each end: radius and fillet share those 6 px
    let b = line_boxes(&[Some((100.0, 200.0)), Some((94.0, 206.0))], 0.0, 60.0, 50.0, 5.0, 20.0);
    let p = plain(&b);
    assert_eq!(p[1].r[0], 3.0, "convex corner = half the step");
    assert!(fillets(&b).iter().all(|f| (f.x1 - f.x0 - 3.0).abs() < 1e-4), "fillet = the other half");
}

#[test]
fn an_empty_line_breaks_the_chain() {
    let b = line_boxes(&[Some((0.0, 100.0)), None, Some((0.0, 50.0))], 10.0, 60.0, 50.0, 4.0, 8.0);
    assert_eq!(plain(&b).len(), 2);
    assert!(fillets(&b).is_empty(), "no neighbours, no fillets");
    assert_eq!(plain(&b)[0].r, [8.0; 4]);
}

#[test]
fn merged_boxes_cover_the_shared_row_with_no_seam_and_fill_the_fillet() {
    let spans = [Some((60.0, 140.0)), Some((20.0, 180.0))];
    let b = line_boxes(&spans, 20.0, 40.0, 34.0, 6.0, 10.0);
    let m = box_mask(&b, 200, 120);
    let at = |x: usize, y: usize| m[y * 200 + x];
    let yb = plain(&b)[0].y1 as usize;
    for y in yb - 3..yb + 3 {
        assert_eq!(at(100, y), 255, "solid across the join at row {y}");
    }
    // just outside the short box, on the row above the join: filled by the fillet
    let short_left = plain(&b)[0].x0 as usize;
    assert!(at(short_left - 1, yb - 1) > 200, "fillet fills the inside corner");
    // well above the join, outside the short box: empty
    assert_eq!(at(short_left - 5, yb - 9), 0);
    // the long box's outer corner is rounded away
    let long = plain(&b)[1];
    assert_eq!(at(long.x0 as usize, long.y1 as usize - 1), 0, "rounded bottom-left corner");
}

// ---- the caption bar ----

/// A project of the given size with one Snapchat Bar clip, moved, scaled and rotated (the bar ignores
/// all but its height), its box made opaque red so it shows on the black canvas.
fn bar_project(w: u32, h: u32) -> (Project, u64) {
    let mut p = Project::new();
    (p.width, p.height) = (w, h);
    let mut c = social_clip("Snapchat Bar").unwrap();
    c.x.value = 300.0;
    c.y.value = 40.0;
    c.scale.value = 1.3;
    c.rotation.value = 20.0;
    c.text.as_mut().unwrap().box_color = [255, 0, 0, 255];
    let ids = p.place_clips(vec![c], Vec::new(), 0.0);
    (p, ids[0])
}

#[test]
fn snapchat_bar_spans_the_frame_at_any_aspect_ratio() {
    let mut text = TextRasterizer::new();
    if !fonts_or_skip(&mut text) {
        return;
    }
    for (w, h) in [(1080u32, 1920u32), (1920, 1080), (1080, 1080), (1080, 1350), (2560, 1080)] {
        let (project, _) = bar_project(w, h);
        let (cw, ch) = (w / 4, h / 4);
        let mut comp = Compositor::new();
        let mut pool = DecoderPool::new(Backend::Ffmpeg);
        let mut out = Frame::default();
        comp.render(&project, 1.0, cw, ch, &mut pool, &mut text, &mut out);
        // the bar's middle row: half the canvas plus y (40 project px)
        let row = ch / 2 + (40 * ch / h);
        let red = |x: u32| {
            let p = px(&out, x, row);
            p[0] > 200 && p[1] < 60
        };
        assert!(red(0) && red(cw - 1), "{w}x{h}: bar reaches both edges");
        assert!(red(cw / 16) && red(cw - cw / 16), "{w}x{h}: and in between");
        // unbroken: every column is the bar or its white text
        let bar_or_text = |x: u32| px(&out, x, row)[0] > 200; // red, white or the pink of an antialiased edge
        let red_cols = (0..cw).filter(|&x| bar_or_text(x)).count() as u32;
        assert!(red_cols + 2 >= cw, "{w}x{h}: bar is unbroken ({red_cols}/{cw})");
        // straight: the same rows are red at the left edge and the right edge (no rotation)
        let rows = |x: u32| (0..ch).filter(|&y| px(&out, x, y)[0] > 200 && px(&out, x, y)[1] < 60).collect::<Vec<_>>();
        assert_eq!(rows(1), rows(cw - 2), "{w}x{h}: the bar is level");
        // it is roughly one line tall: well under a fifth of the frame
        assert!((rows(1).len() as u32) < ch / 5, "{w}x{h}: one line tall, not {}", rows(1).len());
    }
}

#[test]
fn bar_bitmap_is_the_frame_width_and_other_styles_ignore_it() {
    let mut tr = TextRasterizer::new();
    if !fonts_or_skip(&mut tr) {
        return;
    }
    let bar = social_style("Snapchat Bar").unwrap();
    for fw in [270.0f32, 480.0, 1080.0] {
        let img = tr.render_in(&bar, 0.25, 0.0, fw);
        assert_eq!(img.width, fw as u32, "bar image is {fw} wide");
        // the band touches both edges
        let mid = img.height / 2;
        assert!(px(&img, 0, mid)[3] > 100 && px(&img, img.width - 1, mid)[3] > 100);
    }
    let classic = social_style("TikTok Classic").unwrap();
    let a = tr.render_in(&classic, 0.25, 0.0, 270.0);
    let b = tr.render_in(&classic, 0.25, 0.0, 1080.0);
    assert!(std::sync::Arc::ptr_eq(&a, &b), "not a bar: one cache entry whatever the frame");
}

#[test]
fn subtitles_with_the_snapchat_bar_span_the_frame() {
    let mut text = TextRasterizer::new();
    if !fonts_or_skip(&mut text) {
        return;
    }
    let mut p = Project::new();
    (p.width, p.height) = (1080, 1920);
    p.add_cue(0.0, 2.0, "hello there");
    p.subtitle_style = builtin_caption_styles().into_iter().find(|s| s.text == "Snapchat Bar").unwrap();
    p.subtitle_style.box_color = [0, 0, 255, 255];
    let mut comp = Compositor::new();
    let mut pool = DecoderPool::new(Backend::Ffmpeg);
    let mut out = Frame::default();
    comp.render(&p, 1.0, 270, 480, &mut pool, &mut text, &mut out);
    let blue_row = (0..480).find(|&y| px(&out, 0, y)[2] > 200).expect("a blue bar at the left edge");
    assert!(px(&out, 269, blue_row)[2] > 200, "and at the right edge");
}

// ---- save / load ----

#[test]
fn social_presets_round_trip_through_save_and_load() {
    let mut p = Project::new();
    (p.width, p.height) = (1080, 1920);
    let mut at = 0.0;
    for t in builtin_titles().iter().filter(|t| social_style(&t.name).is_some()) {
        let (clips, assets) = decode_template(t).unwrap();
        p.place_clips(clips, assets, at);
        at += 5.0;
    }
    p.subtitle_style = builtin_caption_styles().into_iter().find(|s| s.text == "TikTok Background").unwrap();
    let path = std::env::temp_dir().join(format!("se-social-{}.sedit", std::process::id()));
    p.save(&path).unwrap();
    let q = Project::load(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    let styles = |p: &Project| -> Vec<TextStyle> { p.all_clips().filter_map(|(_, c)| c.text.clone()).collect() };
    assert_eq!(styles(&p).len(), social_title_names().len());
    assert_eq!(styles(&p), styles(&q), "every social style survives save / load");
    assert_eq!(p.subtitle_style, q.subtitle_style);
    assert_eq!(q.subtitle_style.box_mode, BOX_LINES);
    let modes: Vec<u8> = styles(&q).iter().map(|s| s.box_mode).collect();
    assert!(modes.contains(&BOX_BAR) && modes.contains(&BOX_LINES) && modes.contains(&BOX_BLOCK));
    // and a file from before box_mode / box_radius existed loads as one square block, as it drew then
    let old: TextStyle = serde_json::from_str(r#"{"text":"Hi","box_color":[0,0,0,200],"box_padding":8.0}"#).unwrap();
    assert_eq!((old.box_mode, old.box_radius), (BOX_BLOCK, 0.0));
}

// ---- a rendered frame of each preset ----

/// Pixel tallies of a rendered text bitmap.
struct Tally {
    opaque: usize,
    near: Box<dyn Fn([u8; 3], u8) -> usize>,
    w: u32,
    h: u32,
}

fn tally(img: &Frame) -> Tally {
    let px: Vec<[u8; 4]> = img.rgba.chunks_exact(4).map(|p| [p[0], p[1], p[2], p[3]]).collect();
    let opaque = px.iter().filter(|p| p[3] > 240).count();
    let near = move |c: [u8; 3], tol: u8| {
        px.iter().filter(|p| p[3] > 240 && (0..3).all(|i| p[i].abs_diff(c[i]) <= tol)).count()
    };
    Tally { opaque, near: Box::new(near), w: img.width, h: img.height }
}

#[test]
fn each_social_preset_renders_its_look() {
    let mut tr = TextRasterizer::new();
    if !fonts_or_skip(&mut tr) {
        return;
    }
    let shots = std::env::var("SE_SOCIAL_SHOTS").ok();
    for name in social_title_names() {
        let style = social_style(&name).unwrap();
        // a typewriter is checked once revealed
        let img = tr.render_in(&style, 0.5, 5.0, 540.0);
        if let Some(dir) = &shots {
            let mut ppm = format!("P6\n{} {}\n255\n", img.width, img.height).into_bytes();
            for p in img.rgba.chunks_exact(4) {
                // over a mid grey, so boxes and white text both show
                let a = p[3] as u32;
                ppm.extend((0..3).map(|i| ((p[i] as u32 * a + 90 * (255 - a)) / 255) as u8));
            }
            let _ = std::fs::write(std::path::Path::new(dir).join(format!("{name}.ppm")), ppm);
        }
        let t = tally(&img);
        let area = (t.w * t.h) as usize;
        assert!(t.opaque > 0 || name == "Snapchat Bar" || name == "TikTok Semi-Transparent", "{name}: drew nothing");
        let white = (t.near)([255, 255, 255], 10);
        let black = (t.near)([0, 0, 0], 10);
        match name.as_str() {
            "TikTok Background" => {
                assert!(white > area / 3, "{name}: a white box behind the text ({white}/{area})");
                assert!(black > 50, "{name}: black text on it");
            }
            "TikTok Background Dark" => {
                assert!(black > area / 3, "{name}: a black box");
                assert!(white > 50, "{name}: white text");
            }
            "TikTok Background Red" => assert!((t.near)([254, 44, 85], 6) > area / 3, "{name}: a red box"),
            "TikTok Background Yellow" => assert!((t.near)([242, 205, 70], 6) > area / 3, "{name}: a yellow box"),
            "TikTok Background Blue" => assert!((t.near)([52, 150, 240], 6) > area / 3, "{name}: a blue box"),
            "TikTok Semi-Transparent" => {
                let half = img.rgba.chunks_exact(4).filter(|p| (120..=160).contains(&p[3])).count();
                assert!(half > area / 3, "{name}: a see-through box ({half}/{area})");
                assert!(white > 50, "{name}: solid white text");
            }
            "TikTok Classic" | "TikTok Typewriter" => {
                assert!(white > 100, "{name}: white text");
                assert!(t.opaque < area / 2, "{name}: no box");
            }
            "TikTok Neon" => {
                let pink =
                    img.rgba.chunks_exact(4).filter(|p| p[3] > 100 && p[0] > 200 && p[1] < 120 && p[2] > 150).count();
                assert!(pink > 100, "{name}: a pink glow round the letters");
            }
            "Snapchat Bar" => {
                assert_eq!(t.w, 540, "{name}: the frame's width");
                let band = img.rgba.chunks_exact(4).filter(|p| (130..=150).contains(&p[3])).count();
                assert!(band > area / 3, "{name}: a ~55% black band ({band}/{area})");
                assert!(white > 30, "{name}: white text");
            }
            "Snapchat Big Text" => {
                assert!(white > 500, "{name}: big white letters");
                assert!(t.opaque < area / 2, "{name}: no box");
                let soft = img.rgba.chunks_exact(4).filter(|p| p[3] > 0 && p[3] < 200 && p[0] < 60).count();
                assert!(soft > 50, "{name}: a soft dark shadow");
            }
            "Snapchat Highlight" => {
                let y = (t.near)([255, 252, 0], 6);
                assert!(y > area / 3, "{name}: a yellow pill");
                // a pill: the box's corner pixels are rounded away
                let first = (0..t.h).find(|&r| (0..t.w).any(|x| px(&img, x, r)[3] > 240)).unwrap();
                let row: Vec<u32> = (0..t.w).filter(|&x| px(&img, x, first)[3] > 240).collect();
                let mid = (0..t.w).filter(|&x| px(&img, x, t.h / 2)[3] > 240).count();
                assert!(row.len() * 4 < mid * 3, "{name}: round ends ({} vs {mid})", row.len());
            }
            other => panic!("no expectation for {other}"),
        }
    }
}

#[test]
fn typewriter_reveals_then_stops_re_rendering() {
    let mut tr = TextRasterizer::new();
    if !fonts_or_skip(&mut tr) {
        return;
    }
    let style = social_style("TikTok Typewriter").unwrap();
    let start = tr.render(&style, 0.25, 0.0);
    let mid = tr.render(&style, 0.25, 0.6);
    let done = tr.render(&style, 0.25, 1.5);
    let later = tr.render(&style, 0.25, 3.0);
    let ink = |f: &Frame| f.rgba.chunks_exact(4).filter(|p| p[3] > 128).count();
    assert!(ink(&start) < ink(&mid) && ink(&mid) < ink(&done), "letters appear over time");
    assert!(std::sync::Arc::ptr_eq(&done, &later), "after the last key the same cached picture serves");
}

// ---- safe zones ----

/// The safe rect (project px of a 1080x1920 frame) of a platform in the emulator's guides.
fn safe_zone(platform: &str) -> (f32, f32, f32, f32) {
    let v: serde_json::Value = serde_json::from_str(include_str!("../../assets/guides.json")).unwrap();
    let p = v["platforms"].as_array().unwrap().iter().find(|p| p["id"] == platform).unwrap();
    let lay = &p["lay"]["phone"];
    let (rw, rh) = (lay["ref"][0].as_f64().unwrap() as f32, lay["ref"][1].as_f64().unwrap() as f32);
    let z = lay["zones"].as_array().unwrap().iter().find(|z| z["k"] == "s").unwrap();
    let r: Vec<f32> = z["r"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect();
    let (kx, ky) = (1080.0 / rw, 1920.0 / rh);
    (r[0] * kx, r[1] * ky, r[2] * kx, r[3] * ky)
}

#[test]
fn social_presets_sit_inside_the_platform_safe_zones() {
    let mut text = TextRasterizer::new();
    if !fonts_or_skip(&mut text) {
        return;
    }
    for name in social_title_names() {
        let platform = if name.starts_with("TikTok") { "tiktok" } else { "snap_spotlight" };
        let (sx0, sy0, sx1, sy1) = safe_zone(platform);
        let mut p = Project::new();
        (p.width, p.height) = (1080, 1920);
        p.place_clips(vec![social_clip(&name).unwrap()], Vec::new(), 0.0);
        let mut comp = Compositor::new();
        let mut pool = DecoderPool::new(Backend::Ffmpeg);
        let mut out = Frame::default();
        comp.render(&p, 2.0, 540, 960, &mut pool, &mut text, &mut out);
        // anything drawn (the canvas is black; a see-through box still lifts it, or the text does)
        let lit: Vec<(u32, u32)> = (0..960)
            .flat_map(|y| (0..540).map(move |x| (x, y)))
            .filter(|&(x, y)| px(&out, x, y)[..3] != [0, 0, 0])
            .collect();
        assert!(!lit.is_empty(), "{name}: nothing on screen");
        let (x0, x1) = (lit.iter().map(|p| p.0).min().unwrap() * 2, lit.iter().map(|p| p.0).max().unwrap() * 2);
        let (y0, y1) = (lit.iter().map(|p| p.1).min().unwrap() * 2, lit.iter().map(|p| p.1).max().unwrap() * 2);
        assert!(y0 as f32 >= sy0 && y1 as f32 <= sy1, "{name}: rows {y0}..{y1} outside {platform}'s {sy0}..{sy1}");
        if social_style(&name).is_some_and(|s| s.is_bar()) {
            continue; // a bar is the frame's width on purpose; only its height must be safe
        }
        assert!(x0 as f32 >= sx0 && x1 as f32 <= sx1, "{name}: columns {x0}..{x1} outside {platform}'s {sx0}..{sx1}");
    }
}

// ---- names ----

#[test]
fn mcp_names_find_the_social_presets() {
    use crate::engine::presets::preset_name_eq;
    let captions = builtin_caption_styles();
    for (asked, real) in [("tiktok_background", "TikTok Background"), ("snapchat_bar", "Snapchat Bar")] {
        let hit = captions.iter().find(|s| preset_name_eq(&s.text, asked)).map(|s| s.text.as_str());
        assert_eq!(hit, Some(real));
    }
    assert!(captions.iter().find(|s| s.text == "Snapchat Bar").unwrap().is_bar());
    for n in social_title_names() {
        assert!(builtin_titles().iter().any(|t| t.name == n), "{n} is a placeable title");
        assert_eq!(crate::engine::presets::title_category(&n), "Social");
    }
    assert_eq!(crate::engine::presets::title_category("Lower Third"), "Basic");
}
