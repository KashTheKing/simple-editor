//! A cut between two clips that touch, with edges off the frame grid (a ripple/trim, or a cut taken
//! from a transcript, puts them anywhere): the Preview must never go black there. The player decodes
//! a `LayerSet` for the frame-grid instant `idx / fps`, and the preview used to draw it at the clock's
//! own time - between the cut and the next grid frame the drawer then looked for the incoming clip's
//! layer in a set that held only the outgoing one, and drew nothing.

use super::driver_tests::gl;
use super::*;
use crate::engine::compose::Compositor;
use crate::engine::shapes::ShapeRasterizer;
use crate::engine::text::TextRasterizer;
use crate::media::{Backend, DecoderPool, VideoSource};
use crate::model::BackgroundMode;

const RED: [u8; 4] = [230, 20, 20, 255];
const BLUE: [u8; 4] = [20, 20, 230, 255];

/// A solid-colour source; `end` > 0 = no frame at all from a time within 0.1 ms of `end` on (a
/// decoder asked past its last frame).
struct Solid {
    rgba: [u8; 4],
    end: f64,
}
impl VideoSource for Solid {
    fn size(&self) -> (u32, u32) {
        (160, 90)
    }
    fn is_still(&self) -> bool {
        false
    }
    fn frame_at(&mut self, t: f64, w: u32, h: u32, out: &mut Frame) -> bool {
        if self.end > 0.0 && t >= self.end - 1e-4 {
            return false;
        }
        out.resize(w, h);
        out.pts = t;
        for px in out.rgba.chunks_exact_mut(4) {
            px.copy_from_slice(&self.rgba);
        }
        true
    }
}

/// Clip A (red) on [a_start, cut), clip B (blue) on [cut, cut + 3): full-frame H.264 footage, B's
/// source starting at 5 s. `a_src_runs_out`: A's file ends exactly where the clip does.
fn seam(fps: f64, cut: f64, a_src_runs_out: bool) -> (Project, DecoderPool, Id, Id) {
    let (w, h) = (160u32, 90u32);
    let mut p = Project::new();
    (p.width, p.height, p.fps, p.preview_bg) = (w, h, fps, BackgroundMode::Black);
    p.show_subtitles = false;
    let a_start = cut - 3.0;
    let a_dur = if a_src_runs_out { 3.0 } else { 20.0 };
    let mut pool = DecoderPool::new(Backend::Ffmpeg);
    let mut ids = Vec::new();
    for (path, dur, rgba, start, src_in) in [("seam-a", a_dur, RED, a_start, 0.0), ("seam-b", 20.0, BLUE, cut, 5.0)] {
        let v = serde_json::json!({"id": 0, "path": path, "kind": "Video", "duration": dur, "width": w, "height": h, "fps": fps});
        let mut a: crate::model::Asset = serde_json::from_value(v).expect("asset");
        a.codec = "h264".into(); // opaque: `covers` may cull what is under it
        let aid = p.add_asset(a);
        let id = p.new_id();
        let mut c = crate::model::Clip::new(id, ClipKind::Video, path, start, 3.0);
        (c.asset, c.src_in) = (aid, src_in);
        p.tracks[0].clips.push(c);
        ids.push(id);
        pool.insert_video(path, Box::new(Solid { rgba, end: if a_src_runs_out && path == "seam-a" { dur } else { 0.0 } }));
    }
    assert_eq!(p.tracks[0].kind, TrackKind::Video);
    assert!((p.clip(ids[0]).unwrap().end() - p.clip(ids[1]).unwrap().start).abs() < 1e-12, "the clips touch");
    (p, pool, ids[0], ids[1])
}

fn centre(f: &Frame) -> [u8; 4] {
    let i = ((f.height / 2 * f.width + f.width / 2) * 4) as usize;
    [f.rgba[i], f.rgba[i + 1], f.rgba[i + 2], f.rgba[i + 3]]
}

fn near(c: [u8; 4], want: [u8; 4]) -> bool {
    c.iter().zip(want).take(3).all(|(x, y)| (*x as i32 - y as i32).abs() <= 6)
}

/// Every clock time around an off-grid cut - paused anywhere there, or played through it in 0.5 ms
/// steps - shows the clip that is on screen at that frame's grid time: the outgoing one up to the
/// cut's frame, the incoming one from it on. Never black, and the very picture the CPU compositor
/// (and so export, which renders the grid times) makes for that frame.
#[test]
fn an_off_grid_cut_is_never_black_on_the_gpu() {
    let Some(gl) = gl() else {
        eprintln!("SKIPPED: no OpenGL driver for a hidden WGL context");
        return;
    };
    let mut r = GpuRenderer::new(Arc::new(gl)).expect("renderer");
    let (w, h) = (160u32, 90u32);
    // 13.766 is the user's cut: off both grids. 13.75 is on the 60 fps grid; 13.7666... is a third.
    for (fps, cut, runs_out) in [
        (60.0, 13.766, false),
        (30.0, 13.766, false),
        (60.0, 13.766, true),
        (30.0, 13.766, true),
        (60.0, 13.75, false),
        (60.0, 13.75, true),
        (60.0, 13.0 + 23.0 / 30.0, false),
        (24.0, 13.766, true),
    ] {
        let (p, mut pool, a, b) = seam(fps, cut, runs_out);
        let (mut text, mut shapes, mut comp) = (TextRasterizer::new(), ShapeRasterizer::new(), Compositor::new());
        let fd = 1.0 / fps;
        let mut clock: Vec<f64> = (-40..=40).map(|k| cut + k as f64 * 0.0005).collect();
        clock.extend([cut - fd, cut - 1e-9, cut, cut + 1e-9, cut + fd, ((cut * fps).floor() + 1.0) / fps - 1e-7]);
        for t in clock {
            // what the player does for the clock time `t`: the grid frame's set (`gpu_cached`)...
            let idx = (t * fps + 1e-6).floor() as i64;
            let tg = idx as f64 / fps;
            let set = crate::playback::decode_layers(&p, tg, w, h, &mut pool, &mut Vec::new(), &mut text, &mut shapes, &mut comp);
            let want = if p.clip(b).unwrap().contains(tg) { (b, BLUE) } else { (a, RED) };
            assert!(set.get(want.0).is_some(), "fps {fps} cut {cut} runs_out {runs_out}: no layer for the clip on screen at {tg}");
            // ...drawn by the preview at the clock's time (`App::update` passes `player.time()`)
            let mut gpu = Frame::default();
            r.render_frame(&p, t, w, h, &set, &mut gpu);
            let c = centre(&gpu);
            assert!(
                near(c, want.1),
                "fps {fps} cut {cut} runs_out {runs_out}: clock {t:.6} (frame {idx} at {tg:.6}) shows {c:?}, want {:?}",
                want.1
            );
            // export renders frame `idx` at `tg`: the same picture
            let mut cpu = Frame::default();
            comp.render(&p, tg, w, h, &mut pool, &mut text, &mut cpu);
            assert!(near(centre(&cpu), want.1), "CPU/export at {tg:.6} shows {:?}", centre(&cpu));
        }
    }
}
