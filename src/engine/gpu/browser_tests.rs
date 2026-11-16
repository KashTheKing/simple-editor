//! ws:browser-layer: the browser clip, and the polygon mask it relies on, drawn by the real driver
//! through `GpuRenderer::render_frame` and compared with what the CPU compositor promises.

use super::driver_tests::gl;
use super::*;
use crate::model::{Animated, BackgroundMode, Clip, ClipKind, Mask, MaskShape, Project, TrackKind};

fn px(out: &Frame, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * out.width + x) * 4) as usize;
    [out.rgba[i], out.rgba[i + 1], out.rgba[i + 2], out.rgba[i + 3]]
}

fn near(a: [u8; 4], b: [u8; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (*x as i32 - y as i32).abs() <= 3)
}

/// A polygon clip mask sits where its points say, around the layer's centre - as on the CPU
/// (`compose::mask_coverage`). It used to be drawn a whole layer-centre (plus the mask's own cx / cy)
/// away, so on the GPU a polygon-masked clip showed the wrong part or nothing at all.
#[test]
fn a_polygon_mask_is_where_the_cpu_puts_it() {
    let Some(gl) = gl() else {
        eprintln!("SKIPPED: no OpenGL driver for a hidden WGL context");
        return;
    };
    let mut r = GpuRenderer::new(Arc::new(gl)).expect("renderer");
    let (w, h) = (64u32, 64u32);
    let mut p = Project::new();
    (p.width, p.height, p.preview_bg) = (w, h, BackgroundMode::Black);
    let mut c = Clip::new(101, ClipKind::Video, "a", 0.0, 2.0);
    let mut m = Mask::new(MaskShape::Polygon);
    // a 16 px square in the middle; cx / cy are what the mask tool leaves behind and mean nothing here
    m.points = vec![(-8.0, -8.0), (8.0, -8.0), (8.0, 8.0), (-8.0, 8.0)];
    (m.cx, m.cy) = (Animated::new(20.0), Animated::new(-12.0));
    c.mask = Some(m);
    p.tracks[0].clips.push(c);
    let mut red = Frame::new(w, h);
    red.fill([255, 0, 0, 255]);
    let layers = LayerSet { layers: vec![(101, Arc::new(red))], ..Default::default() };
    let mut out = Frame::default();
    r.render_frame(&p, 1.0, w, h, &layers, &mut out);
    assert!(near(px(&out, 32, 32), [255, 0, 0, 255]), "the middle is inside the mask: {:?}", px(&out, 32, 32));
    assert!(near(px(&out, 26, 38), [255, 0, 0, 255]));
    for (x, y) in [(20, 32), (44, 32), (32, 20), (32, 44), (52, 20), (60, 60)] {
        assert!(near(px(&out, x, y), [0, 0, 0, 255]), "({x}, {y}) is outside the mask: {:?}", px(&out, x, y));
    }
}

/// The browser clip on the GPU: the layer below shows inside the window's page area only, and zoom
/// enlarges that layer about its point instead of a picture of the frame.
#[test]
fn the_browser_shows_the_layer_below_and_zooms_it() {
    let Some(gl) = gl() else {
        eprintln!("SKIPPED: no OpenGL driver for a hidden WGL context");
        return;
    };
    let mut r = GpuRenderer::new(Arc::new(gl)).expect("renderer");
    let (w, h) = (640u32, 360u32);
    let mut p = Project::new();
    (p.width, p.height, p.preview_bg) = (w, h, BackgroundMode::Black);
    p.tracks[0].clips.push(Clip::new(101, ClipKind::Video, "page", 0.0, 4.0));
    let ti = p.add_track(TrackKind::Video);
    let mut c = Clip::new(102, ClipKind::Browser, "Browser", 0.0, 4.0);
    {
        let b = c.browser.as_mut().unwrap();
        (b.w, b.h, b.shadow, b.corner) = (400.0, 311.0, 0.0, 0.0); // page area 400x225, window x 120..520
        (b.bg, b.bg2, b.cursor) = ([200, 10, 10, 255], [200, 10, 10, 255], None);
    }
    p.tracks[ti].clips.push(c);
    // the page: blue with a green column over its rightmost tenth
    let mut page = Frame::new(w, h);
    page.fill([0, 0, 255, 255]);
    for y in 0..h {
        for x in 576..w {
            page.rgba[((y * w + x) * 4) as usize..][..4].copy_from_slice(&[0, 255, 0, 255]);
        }
    }
    let page = Arc::new(page);
    let mut art = crate::engine::browser::BrowserRasterizer::default();
    let mut text = crate::engine::text::TextRasterizer::new();
    let mut frame = |p: &Project, r: &mut GpuRenderer| {
        let b = p.clip(102).unwrap().browser.as_ref().unwrap();
        let l = art.layers(b, 1.0, 1.0, 1.0, &mut text, &mut |_: &str, _: u32, _: u32| None);
        let key = crate::engine::browser::key(102, crate::engine::browser::WINDOW);
        let layers = LayerSet { layers: vec![(101, page.clone()), (key, l.window.unwrap())], ..Default::default() };
        let mut out = Frame::default();
        r.render_frame(p, 1.0, w, h, &layers, &mut out);
        out
    };
    let out = frame(&p, &mut r);
    assert!(near(px(&out, 10, 10), [200, 10, 10, 255]), "background: {:?}", px(&out, 10, 10));
    assert!(near(px(&out, 500, 28), [218, 222, 228, 255]), "tab strip: {:?}", px(&out, 500, 28));
    assert!(near(px(&out, 130, 200), [0, 0, 255, 255]), "page at the window's left edge: {:?}", px(&out, 130, 200));
    assert!(near(px(&out, 510, 200), [0, 255, 0, 255]), "the page's right tenth: {:?}", px(&out, 510, 200));
    assert!(near(px(&out, 110, 200), [200, 10, 10, 255]), "nothing of the page outside the window");
    // zoom 4 about the page's left edge: the green column is pushed out of the window
    let b = p.clip_mut(102).unwrap().browser.as_mut().unwrap();
    (b.zoom, b.zoom_x) = (Animated::new(4.0), Animated::new(-320.0));
    let out = frame(&p, &mut r);
    assert!(near(px(&out, 510, 200), [0, 0, 255, 255]), "zoomed: {:?}", px(&out, 510, 200));
    assert!(near(px(&out, 110, 200), [200, 10, 10, 255]), "the zoomed page still stops at the window");
    assert!(near(px(&out, 500, 28), [218, 222, 228, 255]), "the window itself is not zoomed");
}

/// Mean absolute difference between horizontal neighbours: fine detail gives a big number, a blurred or
/// enlarged picture a small one.
fn sharpness(f: &Frame) -> f64 {
    let (w, h) = (f.width as usize, f.height as usize);
    let mut sum = 0u64;
    for y in 0..h {
        for x in 1..w {
            let (a, b) = (f.rgba[(y * w + x) * 4] as i32, f.rgba[(y * w + x - 1) * 4] as i32);
            sum += (a - b).unsigned_abs() as u64;
        }
    }
    sum as f64 / ((w - 1) * h) as f64
}

/// A zoomed browser shows the layer below from its own pixels: the frame is the one a direct render of
/// that layer at that scale gives (within rounding), at rest and in the middle of a zoom move, and it
/// is as sharp - where a frame-sized picture enlarged afterwards would be a grey blur.
#[test]
fn a_zoomed_frame_is_a_direct_render_at_that_scale_not_an_enlarged_picture() {
    let Some(gl) = gl() else {
        eprintln!("SKIPPED: no OpenGL driver for a hidden WGL context");
        return;
    };
    let mut r = GpuRenderer::new(Arc::new(gl)).expect("renderer");
    let (w, h) = (640u32, 360u32);
    // the layer: twice the frame's size, one-pixel black and white columns - detail that only
    // survives when it is shown from its own pixels
    let mut src = Frame::new(w * 2, h * 2);
    for (i, px) in src.rgba.chunks_exact_mut(4).enumerate() {
        let v = if (i as u32 % (w * 2)) % 2 == 0 { 255 } else { 0 };
        px.copy_from_slice(&[v, v, v, 255]);
    }
    let src = Arc::new(src);
    let project = |scale: f64, zoom: Option<f64>| {
        let mut p = Project::new();
        (p.width, p.height, p.preview_bg) = (w, h, BackgroundMode::Black);
        let mut page = Clip::new(101, ClipKind::Video, "page", 0.0, 4.0);
        page.scale = Animated::new(scale);
        p.tracks[0].clips.push(page);
        if let Some(z) = zoom {
            let ti = p.add_track(TrackKind::Video);
            let mut c = Clip::new(102, ClipKind::Browser, "Browser", 0.0, 4.0);
            let b = c.browser.as_mut().unwrap();
            (b.display, b.cursor) = (crate::model::BrowserDisplay::Page, None);
            // a move: 1 at 0 s to `z` at 2 s (frames are taken at 1 s, mid-move, and at 3 s, at rest)
            b.window_zoom_to(0.0, 2.0, z, Some((0.0, 0.0)));
            p.tracks[ti].clips.push(c);
        }
        p
    };
    let mut frame = |p: &Project, t: f64| {
        let layers = LayerSet { layers: vec![(101, src.clone())], ..Default::default() };
        let mut out = Frame::default();
        r.render_frame(p, t, w, h, &layers, &mut out);
        out
    };
    let zoomed = project(1.0, Some(2.0));
    for t in [3.0, 1.0] {
        let z = zoomed.clip(102).unwrap().browser.as_ref().unwrap().wzoom.at(t);
        assert!(if t == 3.0 { z == 2.0 } else { z > 1.2 && z < 1.8 }, "zoom at {t}: {z}");
        let (got, want) = (frame(&zoomed, t), frame(&project(z, None), t));
        let worst = got.rgba.iter().zip(&want.rgba).map(|(a, b)| (*a as i32 - *b as i32).abs()).max().unwrap();
        assert!(worst <= 3, "at {t} (zoom {z}): the zoomed frame differs from a direct render at that scale by {worst}");
        // an enlarged frame-sized picture: what the layer looks like at 1x, which is flat grey
        let flat = sharpness(&frame(&project(1.0, None), t));
        let (a, b) = (sharpness(&got), sharpness(&want));
        assert!(a >= b * 0.98, "at {t}: sharpness {a} under the direct render's {b}");
        if t == 3.0 {
            assert!(a > 200.0 && flat < 20.0, "at rest every column is resolved ({a}); at 1x they are a blur ({flat})");
        }
    }
}

/// Page only: the load bar runs along the top edge of the frame on the GPU too (preview and export both
/// draw through here). Its bitmap is the slot `browser::BG`; it used to reach the CPU compositor only.
#[test]
fn the_page_only_load_bar_is_drawn_on_the_gpu() {
    let Some(gl) = gl() else {
        eprintln!("SKIPPED: no OpenGL driver for a hidden WGL context");
        return;
    };
    let mut r = GpuRenderer::new(Arc::new(gl)).expect("renderer");
    let (w, h) = (640u32, 360u32);
    let mut p = Project::new();
    (p.width, p.height, p.preview_bg) = (w, h, BackgroundMode::Black);
    p.tracks[0].clips.push(Clip::new(101, ClipKind::Video, "page", 0.0, 4.0));
    let ti = p.add_track(TrackKind::Video);
    let mut c = Clip::new(102, ClipKind::Browser, "Browser", 0.0, 4.0);
    {
        let b = c.browser.as_mut().unwrap();
        (b.display, b.cursor) = (crate::model::BrowserDisplay::Page, None);
        b.load = Animated::new(0.5);
    }
    p.tracks[ti].clips.push(c);
    let mut page = Frame::new(w, h);
    page.fill([0, 0, 255, 255]);
    let mut art = crate::engine::browser::BrowserRasterizer::default();
    let mut text = crate::engine::text::TextRasterizer::new();
    let accent = [40, 120, 230, 255];
    let mut frame = |p: &Project, r: &mut GpuRenderer| {
        let b = p.clip(102).unwrap().browser.as_ref().unwrap();
        let l = art.layers(b, 1.0, 1.0, 1.0, &mut text, &mut |_: &str, _: u32, _: u32| None);
        let mut layers = LayerSet { layers: vec![(101, Arc::new(page.clone()))], ..Default::default() };
        layers.layers.extend(l.bg.map(|f| (crate::engine::browser::key(102, crate::engine::browser::BG), f)));
        let mut out = Frame::default();
        r.render_frame(p, 1.0, w, h, &layers, &mut out);
        out
    };
    let out = frame(&p, &mut r);
    assert!(near(px(&out, 100, 1), accent), "the bar, in the left half: {:?}", px(&out, 100, 1));
    assert!(near(px(&out, 500, 1), [0, 0, 255, 255]), "not past where the load is: {:?}", px(&out, 500, 1));
    assert!(near(px(&out, 100, 30), [0, 0, 255, 255]), "a thin strip: {:?}", px(&out, 100, 30));
    // finished (1) or not started (0): no bar
    for v in [0.0, 1.0] {
        p.clip_mut(102).unwrap().browser.as_mut().unwrap().load = Animated::new(v);
        let out = frame(&p, &mut r);
        assert!(near(px(&out, 100, 1), [0, 0, 255, 255]), "load {v}: {:?}", px(&out, 100, 1));
    }
}

/// Window zoom with the chrome ON (tabs, address bar, the mouse): zooming the whole browser is the same
/// picture as making the window that much bigger by the clip's own Scale - at rest and in the middle of a
/// move - and the zoomed art is as sharp (it is drawn again at that size, not enlarged).
#[test]
fn a_window_zoom_with_the_chrome_on_is_the_window_drawn_at_that_scale() {
    let Some(gl) = gl() else {
        eprintln!("SKIPPED: no OpenGL driver for a hidden WGL context");
        return;
    };
    use crate::engine::browser as br;
    let mut r = GpuRenderer::new(Arc::new(gl)).expect("renderer");
    let (w, h) = (640u32, 360u32);
    let mut page = Frame::new(w, h);
    page.fill([250, 250, 250, 255]);
    for y in (0..h).step_by(8) {
        for x in 0..w {
            page.rgba[((y * w + x) * 4) as usize..][..4].copy_from_slice(&[30, 30, 30, 255]);
        }
    }
    let page = Arc::new(page);
    let project = |scale: f64, zoom: Option<f64>| {
        let mut p = Project::new();
        (p.width, p.height, p.preview_bg) = (w, h, BackgroundMode::Black);
        p.tracks[0].clips.push(Clip::new(101, ClipKind::Video, "page", 0.0, 4.0));
        let ti = p.add_track(TrackKind::Video);
        let mut c = Clip::new(102, ClipKind::Browser, "Browser", 0.0, 4.0);
        c.scale = Animated::new(scale);
        let b = c.browser.as_mut().unwrap();
        (b.w, b.h, b.shadow, b.corner) = (400.0, 311.0, 0.0, 8.0);
        b.url = "https://example.com/a/page".into();
        b.cursor.as_mut().unwrap().x = Animated::new(60.0);
        if let Some(z) = zoom {
            b.window_zoom_to(0.0, 2.0, z, Some((0.0, 0.0)));
        }
        p.tracks[ti].clips.push(c);
        p
    };
    let mut art = br::BrowserRasterizer::default();
    let mut text = crate::engine::text::TextRasterizer::new();
    let mut frame = |p: &Project, t: f64, r: &mut GpuRenderer| {
        let c = p.clip(102).unwrap();
        let (lt, s) = (c.local(t), 1.0);
        let l = art.layers(c.browser.as_ref().unwrap(), lt, s, br::raster_scale(c, s, lt), &mut text, &mut |_: &str, _: u32, _: u32| None);
        let mut layers = LayerSet { layers: vec![(101, page.clone())], ..Default::default() };
        for (slot, f) in [(br::BG, l.bg), (br::WINDOW, l.window), (br::CURSOR, l.cursor)] {
            layers.layers.extend(f.map(|f| (br::key(102, slot), f)));
        }
        let mut out = Frame::default();
        r.render_frame(p, t, w, h, &layers, &mut out);
        out
    };
    let zoomed = project(1.0, Some(2.0));
    for t in [3.0, 1.0] {
        let z = zoomed.clip(102).unwrap().browser.as_ref().unwrap().wzoom.at(t);
        assert!(if t == 3.0 { z == 2.0 } else { z > 1.2 && z < 1.8 }, "zoom at {t}: {z}");
        let (got, want) = (frame(&zoomed, t, &mut r), frame(&project(z, None), t, &mut r));
        let off = got.rgba.chunks_exact(4).zip(want.rgba.chunks_exact(4)).filter(|(a, b)| a.iter().zip(*b).any(|(x, y)| (*x as i32 - *y as i32).abs() > 24)).count();
        assert!(off * 200 < (w * h) as usize, "at {t} (zoom {z}): {off} of {} pixels differ from the window drawn at that scale", w * h);
        let (a, b) = (sharpness(&got), sharpness(&want));
        assert!(a >= b * 0.97, "at {t}: sharpness {a} under the direct render's {b}");
    }
}
