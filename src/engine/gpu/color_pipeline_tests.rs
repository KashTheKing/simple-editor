//! ws:parity-color-pipeline: the processing modes on the real driver (`GpuRenderer::set_mode`). A white
//! clip at 50 % opacity over black is the simplest picture where the light model shows: half the
//! encoded value when mixing encoded values, the encoded value of half the light when mixing linear light.

use super::driver_tests::gl;
use super::*;
use crate::model::{Animated, BackgroundMode, Clip, ClipKind, Project};
use crate::settings::ProcessingMode;

fn half_white(mode: ProcessingMode) -> Option<[u8; 4]> {
    let gl = gl()?;
    let mut r = GpuRenderer::new(Arc::new(gl)).expect("renderer");
    r.set_mode(mode);
    let (w, h) = (32u32, 32u32);
    let mut p = Project::new();
    (p.width, p.height, p.preview_bg) = (w, h, BackgroundMode::Black);
    let mut c = Clip::new(101, ClipKind::Video, "a", 0.0, 2.0);
    c.opacity = Animated::new(0.5);
    p.tracks[0].clips.push(c);
    let mut white = Frame::new(w, h);
    white.fill([255, 255, 255, 255]);
    let layers = LayerSet { layers: vec![(101, Arc::new(white))], ..Default::default() };
    let mut out = Frame::default();
    r.render_frame(&p, 1.0, w, h, &layers, &mut out);
    let i = ((16 * w + 16) * 4) as usize;
    Some([out.rgba[i], out.rgba[i + 1], out.rgba[i + 2], out.rgba[i + 3]])
}

#[test]
fn opacity_mixes_encoded_values_in_8_bit_and_16_bit_float() {
    for mode in [ProcessingMode::Fast8, ProcessingMode::Float16] {
        let Some(px) = half_white(mode) else {
            eprintln!("SKIPPED: no OpenGL driver for a hidden WGL context");
            return;
        };
        assert!((px[0] as i32 - 128).abs() <= 2, "{mode:?}: {px:?}");
        assert_eq!(px[3], 255);
    }
}

#[test]
fn opacity_mixes_light_in_the_linear_mode() {
    let Some(px) = half_white(ProcessingMode::Linear16) else {
        eprintln!("SKIPPED: no OpenGL driver for a hidden WGL context");
        return;
    };
    let want = (to_encoded(0.5) * 255.0).round() as i32; // 188
    assert!((px[0] as i32 - want).abs() <= 2, "{px:?}, want {want}");
}

#[test]
fn switching_modes_back_and_forth_keeps_rendering() {
    let Some(gl) = gl() else {
        eprintln!("SKIPPED: no OpenGL driver for a hidden WGL context");
        return;
    };
    let mut r = GpuRenderer::new(Arc::new(gl)).expect("renderer");
    let mut p = Project::new();
    (p.width, p.height, p.preview_bg) = (16, 16, BackgroundMode::Black);
    let mut c = Clip::new(7, ClipKind::Video, "a", 0.0, 2.0);
    c.opacity = Animated::new(1.0);
    p.tracks[0].clips.push(c);
    let mut red = Frame::new(16, 16);
    red.fill([200, 10, 10, 255]);
    let layers = LayerSet { layers: vec![(7, Arc::new(red))], ..Default::default() };
    for mode in [ProcessingMode::Float16, ProcessingMode::Fast8, ProcessingMode::Linear16, ProcessingMode::Fast8] {
        r.set_mode(mode);
        let mut out = Frame::default();
        r.render_frame(&p, 1.0, 16, 16, &layers, &mut out);
        // an opaque layer comes out the same in every mode (the linear round trip is exact enough)
        assert!((out.rgba[0] as i32 - 200).abs() <= 1 && (out.rgba[1] as i32 - 10).abs() <= 1, "{mode:?}: {:?}", &out.rgba[..4]);
    }
}
