//! The "Fortnite Short" regressions on the real driver: a see-through HTML render (H.264, colour
//! stacked over alpha) over footage must not black out the export, and the exported 1080x1920 clip
//! alone must play at 60 fps on a busy PC.

use super::driver_tests::gl;
use super::*;
use crate::engine::compose::Compositor;
use crate::engine::export::tests::{gen_media, temp_dir};
use crate::engine::shapes::ShapeRasterizer;
use crate::engine::text::TextRasterizer;
use crate::media::{ffpipe, Backend, DecoderPool};
use crate::model::{Asset, Track};

/// A fully transparent 320x240 page render, encoded the way `html.rs` writes `.stacked.mp4`.
fn transparent_stacked(dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let exe = ffpipe::ffmpeg_exe()?;
    let out = dir.join("flash.stacked.mp4");
    let st = ffpipe::command(&exe)
        .args([
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=black@0.0:s=320x240:d=4,format=rgba",
        ])
        .args(["-vf", &crate::media::proxy::stacked_filter_to("320:240")])
        .args(["-r", "30", "-pix_fmt", "yuv420p", "-c:v", "libx264"])
        .arg(&out)
        .status()
        .ok()?;
    st.success().then_some(out)
}

/// A 4 s 320x240 H.264 asset (the codec `compose::covers` takes for opaque).
fn h264(path: &std::path::Path) -> Asset {
    Asset {
        id: 0,
        path: path.to_string_lossy().into(),
        kind: ClipKind::Video,
        duration: 4.0,
        width: 320,
        height: 240,
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
        html: None,
        proxy: None,
    }
}

#[test]
fn a_see_through_html_render_does_not_black_out_the_export() {
    let dir = temp_dir("stacked-cover");
    let (Some(red), Some(flash)) = (gen_media(&dir), transparent_stacked(&dir)) else {
        eprintln!("SKIPPED: no ffmpeg");
        return;
    };
    // red footage on V1, the full-frame transparent HTML render on a track above it
    let mut p = Project::from_media(h264(&red));
    let aid = p.add_asset(h264(&flash));
    let (tid, cid) = (p.new_id(), p.new_id());
    let mut over = Clip::new(cid, ClipKind::Video, "Impact flashes", 0.0, 4.0);
    over.asset = aid;
    let mut track = Track::new(tid, TrackKind::Video, "V2");
    track.clips.push(over);
    p.tracks.push(track);
    p.show_subtitles = false;
    let (w, h) = (p.width, p.height);
    assert_eq!((w, h), (320, 240));

    // the export's decode: the stacked layer is not a cover, so V1 is still decoded
    let mut pool = DecoderPool::new(Backend::Ffmpeg);
    let (mut text, mut shapes, mut comp) = (TextRasterizer::new(), ShapeRasterizer::new(), Compositor::new());
    let set =
        crate::playback::decode_layers(&p, 1.0, w, h, &mut pool, &mut Vec::new(), &mut text, &mut shapes, &mut comp);
    assert!(set.get(cid).is_some(), "the HTML render decodes");
    assert_eq!(set.base, 0, "a see-through render must not cull the tracks below it");
    assert!(set.get(p.tracks[0].clips[0].id).is_some(), "the footage under it is decoded");

    // and the export frame (GPU path, full size) shows the footage, not black
    let Some(gl) = gl() else {
        eprintln!("SKIPPED: no OpenGL driver for a hidden WGL context");
        return;
    };
    let mut r = GpuRenderer::new(Arc::new(gl)).expect("renderer");
    let mut out = Frame::default();
    assert!(r.render_export_sync(&p, 1.0, w, h, &set, Readout::Rgba, &mut out));
    let c = &out.rgba[((h / 2 * w + w / 2) * 4) as usize..][..4];
    assert!(c[0] > 200 && c[1] < 60 && c[2] < 60, "the red footage shows through: {c:?}");
}

/// The exported Short (1080x1920 60 fps H.264) alone on the timeline, played for 10 s the way the
/// preview plays it: GPU layer mode, zero-copy decode, each layer set composited on this GL context.
/// `SE_BURN=n` adds n busy threads per core at normal priority (a parallel build) - before the
/// playback threads joined MMCSS (`media::bg::raise_thread`) that dropped frames. Timing-only:
/// `cargo test bench_vertical_clip_plays -- --ignored --nocapture`
#[test]
#[ignore]
fn bench_vertical_clip_plays_without_drops() {
    use crate::playback::Player;
    let dir = std::env::temp_dir().join("se-bench");
    let _ = std::fs::create_dir_all(&dir);
    let mp4 = dir.join("1080x1920p60.mp4");
    let ok = mp4.exists()
        || ffpipe::ffmpeg_exe().is_some_and(|exe| {
            ffpipe::command(&exe)
                .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc2=size=1080x1920:rate=60", "-t", "12"])
                .args(["-c:v", "libx264", "-preset", "veryfast", "-pix_fmt", "yuv420p"])
                .arg(&mp4)
                .status()
                .is_ok_and(|s| s.success())
        });
    let Some(gl) = gl().filter(|_| ok) else {
        eprintln!("SKIPPED: no ffmpeg or no OpenGL driver");
        return;
    };
    let mut r = GpuRenderer::new(Arc::new(gl)).expect("renderer"); // opens the interop: zero-copy on
    crate::media::bg::inherit(i32::MAX); // this thread stands in for the UI thread (`main`)
    let asset = crate::media::probe(&mp4.to_string_lossy(), Backend::Auto).expect("probe");
    let project = Project::from_media(asset);
    let mut p = Player::new(
        eframe::egui::Context::default(),
        Backend::Auto,
        Arc::new(std::sync::Mutex::new(TextRasterizer::new())),
    );
    p.set_gpu(true);
    p.set_project(&project);
    p.set_canvas(360, 640, 0); // about the preview pane's size for a vertical clip
    p.seek(0.0);
    let opened = std::time::Instant::now();
    while p.take_layers().is_none() && opened.elapsed() < std::time::Duration::from_secs(5) {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let per_core = std::env::var("SE_BURN").ok().and_then(|v| v.parse().ok()).unwrap_or(0usize);
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let cores = std::thread::available_parallelism().map_or(8, |n| n.get());
    let burn: Vec<_> = (0..cores * per_core)
        .map(|_| {
            let stop = stop.clone();
            std::thread::spawn(move || {
                let mut x = 0u64;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    x = std::hint::black_box(x.wrapping_mul(31).wrapping_add(1));
                }
            })
        })
        .collect();
    let (published0, _) = p.publish_stats();
    let dropped0 = p.dropped_frames();
    let start = std::time::Instant::now();
    p.play();
    while start.elapsed() < std::time::Duration::from_secs(10) {
        if let Some(layers) = p.take_layers() {
            let _ = r.render_preview_texture(&project, p.time(), 360, 640, &layers);
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    p.pause();
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    burn.into_iter().for_each(|h| h.join().unwrap());
    let (published, worst_gap_ms) = p.publish_stats();
    let dropped = p.dropped_frames() - dropped0;
    eprintln!(
        "1080x1920p60, {per_core} busy threads a core: {} published, dropped {dropped}, worst gap {worst_gap_ms:.1} ms",
        published - published0
    );
    assert_eq!(dropped, 0, "the exported Short alone drops frames");
}

/// ws:gap-alpha-export: a transparent export frame on the real driver, read back as RGBA: clear
/// corners, an opaque shape, soft edges that keep their colour (straight alpha, no dark fringe) - and
/// the CPU compositor draws the same frame.
#[test]
fn a_transparent_export_frame_keeps_alpha_on_the_gpu() {
    let Some(gl) = gl() else {
        eprintln!("SKIPPED: no OpenGL driver for a hidden WGL context");
        return;
    };
    let mut r = GpuRenderer::new(Arc::new(gl)).expect("renderer");
    let mut p = crate::engine::export::tests::shape_over_nothing();
    p.preview_bg = crate::model::BackgroundMode::Custom([0, 0, 0, 0]);
    let (w, h) = (p.width, p.height);
    let mut pool = DecoderPool::new(Backend::Ffmpeg);
    let (mut text, mut shapes, mut comp) = (TextRasterizer::new(), ShapeRasterizer::new(), Compositor::new());
    let set =
        crate::playback::decode_layers(&p, 0.5, w, h, &mut pool, &mut Vec::new(), &mut text, &mut shapes, &mut comp);
    // the export sends the clear background as an override: the live project keeps its own
    p.preview_bg = crate::model::BackgroundMode::Black;
    r.bg_override = Some(crate::model::BackgroundMode::Custom([0, 0, 0, 0]));
    let mut gpu = Frame::default();
    assert!(r.render_export_sync(&p, 0.5, w, h, &set, crate::engine::gpu::Readout::Rgba, &mut gpu));
    crate::engine::export::tests::assert_overlay(&gpu.rgba, "gpu");

    p.preview_bg = crate::model::BackgroundMode::Custom([0, 0, 0, 0]);
    let mut cpu = Frame::default();
    comp.render(&p, 0.5, w, h, &mut pool, &mut text, &mut cpu);
    crate::engine::export::tests::assert_overlay(&cpu.rgba, "cpu");
    let alpha = |f: &Frame| f.rgba.chunks_exact(4).map(|p| p[3] as i32).collect::<Vec<_>>();
    let worst = alpha(&gpu).iter().zip(alpha(&cpu)).map(|(x, y)| (x - y).abs()).max().unwrap_or(0);
    // the two rasterisers anti-alias the shape's edge differently (GPU coverage vs the CPU scanline): one
    // edge pixel can differ by ~18/255; the corners-clear / shape-opaque checks above are the real gate
    assert!(worst <= 24, "gpu and cpu coverage differ by {worst}");
}
