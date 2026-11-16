//! ws:nested-gpu: a nested sequence drawn on the real driver as a layer tree of its own
//! (`LayerSet::nested`) matches what the CPU compositor draws for it.

use super::driver_tests::gl;
use super::*;
use crate::engine::compose::Compositor;
use crate::engine::shapes::ShapeRasterizer;
use crate::engine::text::TextRasterizer;
use crate::media::{Backend, DecoderPool};
use crate::model::{Animated, BackgroundMode, Clip, ShapeKind, ShapeStyle};

fn rect(id: Id, fill: [u8; 4], half: (f64, f64)) -> Clip {
    let mut c = Clip::new(id, ClipKind::Shape, "rect", 0.0, 4.0);
    let mut s = ShapeStyle::new(ShapeKind::Rect);
    (s.fill, s.stroke_width, s.corner) = (fill, 0.0, 0.0);
    (s.w, s.h) = (Animated::new(half.0), Animated::new(half.1));
    c.shape = Some(s);
    c
}

/// A sequence (a green rect over a half-transparent red one) placed at 60 % opacity: the GPU draws it
/// from its own layers, never from a CPU-composited bitmap, and lands within 2 grey levels of the CPU.
#[test]
fn a_nested_sequence_on_the_gpu_matches_the_cpu() {
    let Some(gl) = gl() else {
        eprintln!("SKIPPED: no OpenGL driver for a hidden WGL context");
        return;
    };
    let mut r = GpuRenderer::new(Arc::new(gl)).expect("renderer");
    let (w, h) = (160u32, 90u32);
    let mut p = Project::new();
    (p.width, p.height, p.preview_bg) = (w, h, BackgroundMode::Black);
    let sid = p.new_sequence("seq", w, h, 30.0);
    let (a, b) = (p.new_id(), p.new_id());
    let mut under = rect(a, [255, 0, 0, 255], (60.0, 30.0));
    under.opacity = Animated::new(0.5);
    let over = rect(b, [0, 255, 0, 255], (20.0, 10.0));
    let sq = p.sequence_mut(sid).unwrap();
    sq.tracks[0].clips.push(under);
    let ti = sq.tracks.len();
    sq.tracks.push(crate::model::Track::new(999_999, TrackKind::Video, "V2"));
    sq.tracks[ti].clips.push(over);
    let seq = p.insert_sequence_clip(sid, 0.0, None).expect("sequence clip");
    p.clip_mut(seq).unwrap().opacity = Animated::new(0.6);
    p.show_subtitles = false;

    let mut pool = DecoderPool::new(Backend::Ffmpeg);
    let (mut text, mut shapes, mut comp) = (TextRasterizer::new(), ShapeRasterizer::new(), Compositor::new());
    let set =
        crate::playback::decode_layers(&p, 1.0, w, h, &mut pool, &mut Vec::new(), &mut text, &mut shapes, &mut comp);
    assert!(set.get(seq).is_none(), "no CPU-composited bitmap for the sequence clip");
    let n = set.nested.iter().find(|n| n.id == seq).expect("a nested layer tree");
    assert!(n.set.get(a).is_some() && n.set.get(b).is_some(), "the sequence's own clips are its layers");

    let mut gpu = Frame::default();
    r.render_frame(&p, 1.0, w, h, &set, &mut gpu);
    let mut cpu = Frame::default();
    comp.render(&p, 1.0, w, h, &mut pool, &mut text, &mut cpu);
    assert_eq!((gpu.width, gpu.height), (cpu.width, cpu.height));
    let worst = gpu.rgba.iter().zip(&cpu.rgba).map(|(x, y)| (*x as i32 - *y as i32).abs()).max().unwrap_or(0);
    assert!(worst <= 2, "GPU nested differs from CPU nested by {worst} grey levels");
    // and it is really there: green at the centre, red-ish beside it
    let px = |f: &Frame, x: u32, y: u32| f.rgba[((y * w + x) * 4) as usize..][..4].to_vec();
    let c = px(&gpu, 80, 45);
    assert!(c[1] > 140 && c[0] < 10, "the green rect at 60 %: {c:?}");
    let s = px(&gpu, 30, 45);
    assert!(s[0] > 60 && s[1] < 10, "the red rect at 50 % x 60 %: {s:?}");
}
