//! A see-through HTML render (H.264, colour stacked over alpha) over footage: export must not take it
//! for an opaque full-frame layer and drop the tracks below (the "Fortnite Short" black export).

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
    assert!(r.render_export_sync(&p, 1.0, w, h, &set, false, &mut out));
    let c = &out.rgba[((h / 2 * w + w / 2) * 4) as usize..][..4];
    assert!(c[0] > 200 && c[1] < 60 && c[2] < 60, "the red footage shows through: {c:?}");
}
