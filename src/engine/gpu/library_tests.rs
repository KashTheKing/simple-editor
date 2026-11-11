//! ws:shader-library: every shader in `library/effects` and `library/transitions` is linted, compiled
//! by the real driver and drawn once (`cargo test library_shaders`). Also the tool content authors
//! look at their work with:
//!   SE_LIBRARY=<dir>   the library folder (default: `<repo>/library`)
//!   SE_ONLY=<text>     only files whose path contains this (e.g. `transitions/paper` or `ghost`)
//!   SE_SHOTS=<dir>     also write `<kind>-<id>.png` there: an effect as [input | t=0.5 s | t=1.7 s],
//!                      a transition as five frames at progress 0, 0.25, 0.5, 0.75, 1
//! Files are read at run time, so the built test binary can be re-run after editing a shader.

use super::driver_tests::gl;
use super::*;
use crate::engine::shaderlib::{lint, parse_meta, ShaderMeta};

const W: i32 = 480;
const H: i32 = 270;

fn lib_dir() -> std::path::PathBuf {
    std::env::var_os("SE_LIBRARY")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("library"))
}

fn mixc(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// Three stand-in pictures, top-down RGBA: 0 = a bright day scene with a "person", 1 = a night
/// skyline, 2 = the person alone on transparency (for cut-out effects). `shift` slides the picture
/// sideways in pixels: the "earlier frame" of a `@needs previous` effect.
fn picture(which: u8, w: i32, h: i32, shift: f32) -> Vec<u8> {
    let mut px = vec![0u8; (w * h * 4) as usize];
    let asp = w as f32 / h as f32;
    for y in 0..h {
        for x in 0..w {
            let (u, v) = ((x as f32 + shift) / w as f32, y as f32 / h as f32);
            let disc = |cx: f32, cy: f32, r: f32| ((u - cx) * asp).hypot(v - cy) < r;
            let head = disc(0.32, 0.40, 0.085);
            let body = (u - 0.32).abs() * asp < 0.14 && v > 0.50 && v < 0.95 && !head;
            let person = head || body;
            let mut a = 1.0;
            let c = if which == 1 {
                let mut c = mixc([0.04, 0.05, 0.14], [0.22, 0.10, 0.30], v);
                if disc(0.78, 0.2, 0.07) {
                    c = [0.92, 0.92, 0.85];
                }
                let col = (u * 9.0).floor();
                let top = 0.35 + 0.3 * ((col * 12.9898).sin() * 43758.547).fract().abs();
                if v > top {
                    c = [0.07, 0.08, 0.12];
                    let (fu, fv) = ((u * 9.0).fract(), ((v - top) * 14.0).fract());
                    let lit = ((col * 7.0 + ((v - top) * 14.0).floor() * 3.0).sin() * 9.0).fract().abs() > 0.45;
                    if lit && fu > 0.2 && fu < 0.8 && fv > 0.25 && fv < 0.75 {
                        c = [1.0, 0.82, 0.35];
                    }
                }
                if (v - 0.9).abs() < 0.012 {
                    c = [1.0, 0.15, 0.7];
                }
                c
            } else {
                let mut c = mixc([0.22, 0.52, 0.95], [0.78, 0.9, 1.0], v / 0.62);
                if disc(0.78, 0.2, 0.08) {
                    c = [1.0, 0.93, 0.55];
                }
                let ridge = 0.62 - 0.13 * (1.0 - ((u * 3.0).fract() * 2.0 - 1.0).abs());
                if v > ridge {
                    c = [0.36, 0.43, 0.52];
                }
                if v > 0.64 {
                    let stripe = if ((u * 24.0 + v * 9.0).fract()) < 0.5 { 0.9 } else { 1.0 };
                    c = [0.30 * stripe, 0.62 * stripe, 0.28 * stripe];
                }
                if u > 0.56 && u < 0.92 && ((v - 0.74).abs() < 0.018 || (v - 0.81).abs() < 0.012) {
                    c = [1.0, 1.0, 1.0];
                }
                if head {
                    c = [0.87, 0.66, 0.53];
                }
                if body {
                    c = [0.86, 0.2, 0.2];
                }
                if which == 2 && !person {
                    a = 0.0;
                }
                c
            };
            let i = ((y * w + x) * 4) as usize;
            px[i] = (c[0] * 255.0) as u8;
            px[i + 1] = (c[1] * 255.0) as u8;
            px[i + 2] = (c[2] * 255.0) as u8;
            px[i + 3] = (a * 255.0) as u8;
        }
    }
    px
}

unsafe fn texture(gl: &glow::Context, px: Option<&[u8]>, w: i32, h: i32) -> glow::Texture {
    let t = gl.create_texture().unwrap();
    gl.bind_texture(glow::TEXTURE_2D, Some(t));
    tex_params(gl);
    let data = glow::PixelUnpackData::Slice(px);
    gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA8 as i32, w, h, 0, glow::RGBA, glow::UNSIGNED_BYTE, data);
    t
}

struct Rig {
    gl: glow::Context,
    vert: glow::Shader,
    pics: [glow::Texture; 3],
    prev: glow::Texture,
    next: glow::Texture,
    out: glow::Texture,
}

impl Rig {
    unsafe fn new(gl: glow::Context) -> Rig {
        let vert = compile(&gl, glow::VERTEX_SHADER, shaders::VERT).unwrap();
        let pics = [0u8, 1, 2].map(|i| texture(&gl, Some(&picture(i, W, H, 0.0)), W, H));
        let prev = texture(&gl, Some(&picture(0, W, H, 36.0)), W, H);
        let next = texture(&gl, Some(&picture(0, W, H, 18.0)), W, H);
        let out = texture(&gl, None, W, H);
        let fbo = gl.create_framebuffer().unwrap();
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(out), 0);
        gl.viewport(0, 0, W, H);
        let vao = gl.create_vertex_array().unwrap();
        gl.bind_vertex_array(Some(vao));
        Rig { gl, vert, pics, prev, next, out }
    }

    /// Draw `prog` with the header's default knobs; `set` binds what differs per kind.
    unsafe fn draw(&self, prog: glow::Program, meta: &ShaderMeta, set: impl Fn(&dyn Fn(&str, f32))) -> Vec<u8> {
        let gl = &self.gl;
        gl.use_program(Some(prog));
        let f = |name: &str, v: f32| gl.uniform_1_f32(gl.get_uniform_location(prog, name).as_ref(), v);
        let tex = |name: &str, unit: u32, t: glow::Texture| {
            gl.active_texture(glow::TEXTURE0 + unit);
            gl.bind_texture(glow::TEXTURE_2D, Some(t));
            gl.uniform_1_i32(gl.get_uniform_location(prog, name).as_ref(), unit as i32);
        };
        let layer = self.pics[if meta.tags.iter().any(|t| t == "alpha" || t == "cutout") { 2 } else { 0 }];
        tex("tex", 0, layer);
        tex("u_from", 1, self.pics[0]);
        tex("u_to", 2, self.pics[1]);
        tex("u_prev", 3, self.prev);
        tex("u_next", 4, self.next);
        gl.uniform_2_f32(gl.get_uniform_location(prog, "u_res").as_ref(), W as f32, H as f32);
        f("u_scale", W as f32 / 1920.0);
        f("ratio", W as f32 / H as f32);
        f("u_frames", 2.0);
        gl.uniform_1_i32(gl.get_uniform_location(prog, "u_has_mask").as_ref(), 0);
        for i in 0..8 {
            f(&format!("p{i}"), meta.params.get(i).map_or(0.0, |p| p.default as f32));
        }
        set(&f);
        gl.clear_color(0.0, 0.0, 0.0, 0.0);
        gl.clear(glow::COLOR_BUFFER_BIT);
        gl.draw_arrays(glow::TRIANGLES, 0, 3);
        let mut pix = vec![0u8; (W * H * 4) as usize];
        gl.read_pixels(0, 0, W, H, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelPackData::Slice(Some(&mut pix)));
        let _ = self.out;
        pix
    }
}

/// Mean absolute difference per channel, 0..255.
fn diff(a: &[u8], b: &[u8]) -> f32 {
    a.iter().zip(b).map(|(x, y)| (*x as f32 - *y as f32).abs()).sum::<f32>() / a.len() as f32
}

/// `tiles` side by side over a checkerboard (so transparency shows), as a PNG.
fn strip(tiles: &[&[u8]]) -> Vec<u8> {
    let mut f = crate::media::Frame::new((W as usize * tiles.len()) as u32, H as u32);
    for (n, t) in tiles.iter().enumerate() {
        for y in 0..H as usize {
            for x in 0..W as usize {
                let s = (y * W as usize + x) * 4;
                let d = (y * W as usize * tiles.len() + n * W as usize + x) * 4;
                let a = t[s + 3] as u32;
                let bg = if (x / 12 + y / 12) % 2 == 0 { 70 } else { 100 };
                for c in 0..3 {
                    f.rgba[d + c] = ((t[s + c] as u32 * a + bg * (255 - a)) / 255) as u8;
                }
                f.rgba[d + 3] = 255;
            }
        }
    }
    crate::mcp::png_encode(&f)
}

#[test]
fn library_shaders_compile_and_draw_on_the_real_driver() {
    let Some(gl) = gl() else {
        eprintln!("SKIPPED: no OpenGL driver for a hidden WGL context");
        return;
    };
    let rig = unsafe { Rig::new(gl) };
    let (only, shots) = (std::env::var("SE_ONLY").unwrap_or_default(), std::env::var_os("SE_SHOTS"));
    if let Some(d) = &shots {
        let _ = std::fs::create_dir_all(d);
    }
    let from = picture(0, W, H, 0.0);
    let to = picture(1, W, H, 0.0);
    let (mut bad, mut seen) = (Vec::new(), 0);
    for (kind, transition) in [("effects", false), ("transitions", true)] {
        let Ok(dir) = std::fs::read_dir(lib_dir().join(kind)) else { continue };
        let mut files: Vec<_> = dir.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "glsl")).collect();
        files.sort();
        for path in files {
            let id = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
            let rel = format!("{kind}/{id}");
            if !only.is_empty() && !rel.contains(&only) {
                continue;
            }
            seen += 1;
            let Ok(src) = std::fs::read_to_string(&path) else {
                bad.push(format!("{rel}: not UTF-8 text"));
                continue;
            };
            let mut problems = lint(&src, transition);
            if id.chars().any(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')) {
                problems.push("file name must be lowercase kebab-case".into());
            }
            let meta = parse_meta(&src);
            let full = if transition { shaders::transition_shader(&src) } else { shaders::user_shader(&src) };
            match unsafe { build(&rig.gl, rig.vert, &full) } {
                Err(log) => problems.push(format!("does not compile:\n{}", log.trim())),
                Ok(prog) => unsafe {
                    let tiles: Vec<Vec<u8>> = if transition {
                        [0.0f32, 0.25, 0.5, 0.75, 1.0].iter().map(|p| rig.draw(prog.p, &meta, |f| f("progress", *p))).collect()
                    } else {
                        let layer = picture(if meta.tags.iter().any(|t| t == "alpha" || t == "cutout") { 2 } else { 0 }, W, H, 0.0);
                        let mut v = vec![layer];
                        v.extend([0.5f32, 1.7].iter().map(|t| rig.draw(prog.p, &meta, |f| f("u_time", *t))));
                        v
                    };
                    if transition {
                        let (d0, d1) = (diff(&tiles[0], &from), diff(&tiles[4], &to));
                        if d0 > 1.0 {
                            problems.push(format!("progress 0 is not the outgoing picture (mean difference {d0:.1}/255)"));
                        }
                        if d1 > 1.0 {
                            problems.push(format!("progress 1 is not the incoming picture (mean difference {d1:.1}/255)"));
                        }
                        if diff(&tiles[2], &from) < 1.0 || diff(&tiles[2], &to) < 1.0 {
                            problems.push("progress 0.5 looks like one of the two pictures: nothing happens".into());
                        }
                    } else {
                        if tiles[1].chunks(4).all(|p| p[3] == 0) {
                            problems.push("draws nothing (alpha 0 everywhere) with the default params".into());
                        }
                        if diff(&tiles[1], &tiles[0]) < 0.5 && diff(&tiles[2], &tiles[0]) < 0.5 {
                            problems.push("no visible change with the default params".into());
                        }
                    }
                    if let Some(d) = &shots {
                        let refs: Vec<&[u8]> = tiles.iter().map(|t| t.as_slice()).collect();
                        let _ = std::fs::write(std::path::Path::new(d).join(format!("{kind}-{id}.png")), strip(&refs));
                    }
                    rig.gl.delete_program(prog.p);
                },
            }
            if problems.is_empty() {
                println!("ok    {rel}  ({})", meta.name);
            } else {
                println!("FAIL  {rel}");
                for p in &problems {
                    println!("      - {}", p.replace('\n', "\n        "));
                }
                bad.push(rel);
            }
        }
    }
    println!("{seen} library shaders checked, {} failed", bad.len());
    assert!(bad.is_empty(), "library shaders with problems: {bad:?}");
}
