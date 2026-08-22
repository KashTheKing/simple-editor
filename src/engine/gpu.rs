//! GPU renderer: the effect chain as OpenGL shaders, using eframe's existing glow context.
//!
//! Why: the CPU compositor is fine for cuts but blur/VHS/motion-blur at 1080p+ are not. Every round-3
//! effect is a fragment shader here; the CPU path (engine/effects.rs) keeps the older, cheap effects so
//! the app still runs (degraded) when no GL context exists.
//!
//! Threading: GL belongs to the thread that owns the context — the UI thread. So:
//!   * preview  → `render_to_texture` during `App::update`, painted straight into the preview pane
//!                (no CPU readback at all).
//!   * export   → the export thread posts `RenderRequest`s; the UI thread serves them with `render_frame`
//!                (readback via `glReadPixels`) so preview and export run the SAME shaders.
//!   * headless (`--selftest`, no GL) → callers fall back to `engine::compose::Compositor`.
//!
//! Pipeline per frame: decode (CPU, player threads) → upload layer textures (`upload`) → for each visual
//! clip: build its chain (node graph when present, else the linear effect stack) → ping-pong FBOs →
//! transform+blend into the canvas FBO → adjustment layers re-process the canvas → subtitles/markers.
//! All textures are RGBA8 with straight alpha, matching `media::Frame`.
//!
//! What the caller owes us (`LayerSet`): one decoded frame per visual clip that is on screen at `t`,
//! keyed by clip id — including both clips of a transition (they are rendered virtually extended past
//! their own bounds), the rendered bitmap of a `Text`/`Shape` clip, the rendered canvas of a nested
//! `Sequence` clip, and, under `LayerSet::SUBTITLES`, the subtitle bitmap. Anything missing is skipped.
//! Footage is placed from its asset's native size, so any decode size works; text/shape/subtitle
//! bitmaps are placed 1:1 and must be rasterised for `render_size(w, h, quality)`, not for `w`×`h`.
//!
//! Textures returned by `render_to_texture` / `apply_effect` / `mask_texture` / `eval_graph` stay valid
//! until the next `render_to_texture` / `render_frame` call, which recycles them.

use crate::engine::shaders;
use crate::media::Frame;
use crate::model::{BlendMode, Clip, ClipKind, Effect, EffectKind, Id, Mask, MaskShape, NodeGraph, NodeKind};
use crate::model::{Project, Scaler, TrackKind};
use eframe::glow;
use eframe::glow::HasContext;
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::Arc;

/// One reusable off-screen target (texture + FBO).
pub struct Target {
    pub tex: glow::Texture,
    pub fbo: glow::Framebuffer,
    pub w: u32,
    pub h: u32,
}

/// Compiled shader programs, cached by effect kind (and by source hash for `EffectKind::Shader`).
#[derive(Default)]
pub struct Programs {
    map: HashMap<u64, Prog>,
    /// Keys whose GLSL failed — never retried (and the first log is kept for the UI).
    failed: HashMap<u64, String>,
}

struct Prog {
    p: glow::Program,
    uni: HashMap<&'static str, glow::UniformLocation>,
}

/// Every uniform any of our programs can declare; looked up once per link.
const UNIFORMS: &[&str] = &[
    "tex",
    "u_res",
    "u_time",
    "u_scale",
    "p0",
    "p1",
    "p2",
    "p3",
    "p4",
    "p5",
    "p6",
    "p7",
    // Curves declares p8..p11 itself; a program that does not is simply missing the location.
    "p8",
    "p9",
    "p10",
    "p11",
    "u_mask",
    "u_has_mask",
    "u_dst",
    "u_layer",
    "u_inv",
    "u_scaler",
    "u_mode",
    "u_opacity",
    "u_b",
    "u_matte_alpha",
    "u_matte_invert",
    "u_prev",
    "u_next",
    "u_frames",
    "u_motion",
    "m_shape",
    "m_center",
    "m_radius",
    "m_rot",
    "m_feather",
    "m_expand",
    "m_opacity",
    "m_invert",
    "m_count",
];

// Program cache keys. Effects live at EFFECT_BASE + kind index; user shaders at their source hash
// (with the top bit set so they can never collide with the small ids).
const K_COMPOSITE: u64 = 1;
const K_MASK: u64 = 2;
const K_COPY: u64 = 3;
const K_MATTE: u64 = 4;
const EFFECT_BASE: u64 = 16;
const USER_BIT: u64 = 1 << 63;

/// `vec4 effect(..)` bodies that are not per-effect: a pass-through and the two-input matte.
const COPY_BODY: &str = "vec4 effect(vec4 src, vec2 uv) { return src; }";
const MATTE_BODY: &str = r#"
uniform sampler2D u_b;
uniform int u_matte_alpha;
uniform int u_matte_invert;
vec4 effect(vec4 src, vec2 uv) {
    vec4 m = texture(u_b, uv);
    float k = (u_matte_alpha != 0) ? m.a : luma(m.rgb) * m.a;
    if (u_matte_invert != 0) { k = 1.0 - k; }
    return vec4(src.rgb, src.a * clamp(k, 0.0, 1.0));
}
"#;

/// Texture units.
const U_TEX: u32 = 0;
const U_MASK: u32 = 1;
const U_DST: u32 = 2;
const U_B: u32 = 3;
const U_PREV: u32 = 4;
const U_NEXT: u32 = 5;

/// Texture-cache keys for the neighbouring frames of a motion-blurred clip. Clip ids are small
/// counters (`Project::new_id`), so the top bits are free to tag a variant of the same clip.
const MOTION_PREV: u64 = 1 << 62;
const MOTION_NEXT: u64 = 1 << 61;

/// A layer texture and the revision of the frame currently in it.
struct LayerTex {
    tex: glow::Texture,
    w: u32,
    h: u32,
    rev: (usize, u64),
}

/// Size-keyed target pool. GL objects are created by the renderer when `take` finds nothing reusable
/// and deleted here when the free list outgrows its budget. Ping-pong falls out of it: `composite`
/// takes a second canvas-sized target, draws into it and returns the old canvas here, so the two keep
/// swapping.
#[derive(Default)]
struct Pool {
    /// The context, so `put` can actually free what it evicts. `None` in the pure-bookkeeping tests.
    gl: Option<Arc<glow::Context>>,
    free: Vec<Target>,
    /// Targets handed out and not yet returned.
    live: usize,
}

impl Pool {
    /// How many RGBA8 pixels the free list may hold (~256 MB: 8 × 4K, 30 × 1080p). Without a cap a
    /// preview resize walks through one target per pixel width and never frees any of them.
    const MAX_FREE_PX: usize = 64 << 20;

    /// A free target of exactly this size, if there is one (callers create one otherwise).
    fn take(&mut self, w: u32, h: u32) -> Option<Target> {
        let i = self.free.iter().position(|t| t.w == w && t.h == h)?;
        self.live += 1;
        Some(self.free.swap_remove(i))
    }
    /// Count a target the caller just created.
    fn created(&mut self) {
        self.live += 1;
    }
    fn put(&mut self, t: Target) {
        self.live = self.live.saturating_sub(1);
        self.free.push(t);
        // ponytail: FIFO eviction, not true LRU — the oldest entry is the stale size, and resize
        // churn is the only thing that grows this. Sort by last use if a real workload thrashes.
        let px = |t: &Target| t.w as usize * t.h as usize;
        let mut total: usize = self.free.iter().map(px).sum();
        while total > Self::MAX_FREE_PX && self.free.len() > 1 {
            let old = self.free.remove(0);
            total -= px(&old);
            if let Some(gl) = &self.gl {
                unsafe {
                    gl.delete_framebuffer(old.fbo);
                    gl.delete_texture(old.tex);
                }
            }
        }
    }
}

/// The renderer. One per app; lives on the UI thread.
pub struct GpuRenderer {
    gl: Arc<glow::Context>,
    vao: glow::VertexArray,
    vert: glow::Shader,
    programs: Programs,
    pool: Pool,
    /// Uploaded layer textures by caller key.
    textures: HashMap<u64, LayerTex>,
    /// 1×1 white texture, bound where a sampler is unused.
    blank: glow::Texture,
    /// Targets handed to the caller; recycled at the start of the next frame.
    loaned: Vec<Target>,
}

impl GpuRenderer {
    /// Compile the shader set. Err (with the GLSL log) when the driver rejects something — the caller
    /// falls back to the CPU compositor and shows the message once.
    pub fn new(gl: Arc<glow::Context>) -> Result<Self, String> {
        unsafe {
            let vao = gl.create_vertex_array().map_err(|e| format!("vertex array: {e}"))?;
            let vert = compile(&gl, glow::VERTEX_SHADER, shaders::VERT)?;
            let blank = gl.create_texture().map_err(|e| format!("texture: {e}"))?;
            gl.bind_texture(glow::TEXTURE_2D, Some(blank));
            tex_params(&gl);
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                1,
                1,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&[255, 255, 255, 255])),
            );
            let mut r = Self {
                pool: Pool { gl: Some(gl.clone()), ..Pool::default() },
                gl,
                vao,
                vert,
                programs: Programs::default(),
                textures: HashMap::new(),
                blank,
                loaned: Vec::new(),
            };
            // The composite program is the one nothing works without: fail loudly here instead of
            // silently rendering black later.
            r.ensure(K_COMPOSITE)?;
            Ok(r)
        }
    }

    /// Upload/refresh a decoded layer as a texture keyed by (clip id, source time bucket). Returns the
    /// texture to use; re-uploads only when the frame actually changed (`Frame` pointer/pts).
    pub fn upload(&mut self, key: u64, frame: &Frame) -> glow::Texture {
        if frame.is_empty() {
            return self.blank;
        }
        let gl = self.gl.clone();
        let rev = (frame.rgba.as_ptr() as usize, frame.pts.to_bits());
        let (w, h) = (frame.width, frame.height);
        unsafe {
            match self.textures.get_mut(&key) {
                Some(lt) if lt.w == w && lt.h == h => {
                    if lt.rev == rev {
                        return lt.tex;
                    }
                    lt.rev = rev;
                    gl.bind_texture(glow::TEXTURE_2D, Some(lt.tex));
                    gl.tex_sub_image_2d(
                        glow::TEXTURE_2D,
                        0,
                        0,
                        0,
                        w as i32,
                        h as i32,
                        glow::RGBA,
                        glow::UNSIGNED_BYTE,
                        glow::PixelUnpackData::Slice(Some(&frame.rgba)),
                    );
                    return lt.tex;
                }
                _ => {}
            }
            if let Some(old) = self.textures.remove(&key) {
                gl.delete_texture(old.tex);
            }
            let Ok(tex) = gl.create_texture() else { return self.blank };
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            tex_params(&gl);
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                w as i32,
                h as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&frame.rgba)),
            );
            self.textures.insert(key, LayerTex { tex, w, h, rev });
            tex
        }
    }

    /// Render the whole timeline at time `t` into an off-screen target and return it, so the preview can
    /// paint it with `eframe::Frame::register_native_glow_texture`. `quality` (0.25..1.0) scales the
    /// internal resolution (see `render_size`); the texture is always presented at `w`×`h`.
    pub fn render_to_texture(
        &mut self,
        project: &Project,
        t: f64,
        w: u32,
        h: u32,
        quality: f32,
        layers: &LayerSet,
    ) -> Option<glow::Texture> {
        self.reclaim();
        let host = self.host_fbo();
        let (rw, rh) = render_size(w, h, quality);
        let canvas = self.render_canvas(project, t, rw, rh, layers);
        self.restore(host);
        let canvas = canvas?;
        let tex = canvas.tex;
        self.loaned.push(canvas);
        Some(tex)
    }

    /// Same, but read back into `out` (export / export-frame / MCP `render.frame`).
    pub fn render_frame(&mut self, project: &Project, t: f64, w: u32, h: u32, layers: &LayerSet, out: &mut Frame) {
        self.reclaim();
        let host = self.host_fbo();
        out.resize(w, h);
        out.pts = t;
        if w == 0 || h == 0 {
            return;
        }
        let Some(canvas) = self.render_canvas(project, t, w, h, layers) else {
            self.restore(host);
            return;
        };
        let gl = self.gl.clone();
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(canvas.fbo));
            gl.pixel_store_i32(glow::PACK_ALIGNMENT, 1);
            gl.read_pixels(
                0,
                0,
                w as i32,
                h as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut out.rgba)),
            );
        }
        self.pool.put(canvas);
        self.restore(host);
    }

    /// Apply one effect to a texture (used by the effect-thumbnail grid and the node editor previews).
    /// Render the timeline into an off-screen texture the caller can paint directly (zero-copy preview).
    /// The texture stays owned by the renderer and is valid until the next call; `reclaim()` recycles it
    /// on the following frame, so the caller must paint it in the same frame it asked for it.
    pub fn render_preview_texture(
        &mut self,
        project: &Project,
        t: f64,
        w: u32,
        h: u32,
        layers: &LayerSet,
    ) -> Option<(glow::Texture, u32, u32)> {
        if w == 0 || h == 0 {
            return None;
        }
        self.reclaim();
        let host = self.host_fbo();
        let canvas = self.render_canvas(project, t, w, h, layers);
        self.restore(host);
        let canvas = canvas?;
        let out = (canvas.tex, canvas.w, canvas.h);
        // hand it out for this frame; `loaned` returns it to the pool on the next `reclaim`
        self.loaned.push(canvas);
        Some(out)
    }

    /// Render `effect` over `src` and read the result back into `out` (catalogue thumbnails).
    /// `t` is the clip-local time to sample animated parameters at. False when the effect has no GPU body.
    pub fn effect_preview(&mut self, src: &Frame, effect: &Effect, t: f64, out: &mut Frame) -> bool {
        if src.is_empty() {
            return false;
        }
        let (w, h) = (src.width, src.height);
        let host = self.host_fbo();
        let tex = self.upload(u64::MAX, src);
        let blob = (effect.kind == EffectKind::BlobTrack)
            .then(|| crate::engine::effects::track(src, &param_values(effect, t)))
            .flatten();
        let dst = self.run_effect(tex, (w, h), effect, t, 1.0, None, None, blob);
        let ok = match dst {
            Some(target) => {
                out.resize(w, h);
                let gl = self.gl.clone();
                unsafe {
                    gl.bind_framebuffer(glow::FRAMEBUFFER, Some(target.fbo));
                    gl.pixel_store_i32(glow::PACK_ALIGNMENT, 1);
                    gl.read_pixels(
                        0,
                        0,
                        w as i32,
                        h as i32,
                        glow::RGBA,
                        glow::UNSIGNED_BYTE,
                        glow::PixelPackData::Slice(Some(&mut out.rgba)),
                    );
                }
                self.pool.put(target);
                true
            }
            None => false,
        };
        self.restore(host);
        ok
    }

    pub fn apply_effect(
        &mut self,
        src: glow::Texture,
        size: (u32, u32),
        effect: &Effect,
        t: f64,
    ) -> Option<glow::Texture> {
        let host = self.host_fbo();
        let dst = self.run_effect(src, size, effect, t, 1.0, None, None, None);
        self.restore(host);
        let dst = dst?;
        let tex = dst.tex;
        self.loaned.push(dst);
        Some(tex)
    }

    /// Rasterise a mask into a single-channel texture (also used by the mask editor overlay).
    pub fn mask_texture(&mut self, mask: &Mask, size: (u32, u32), t: f64, project_w: u32) -> Option<glow::Texture> {
        let scale = size.0 as f32 / project_w.max(1) as f32;
        let centre = (size.0 as f32 / 2.0, size.1 as f32 / 2.0);
        let host = self.host_fbo();
        let target = self.render_mask(mask, size, t, scale, centre);
        self.restore(host);
        let target = target?;
        let tex = target.tex;
        self.loaned.push(target);
        Some(tex)
    }

    /// Evaluate a node graph, returning the output texture.
    pub fn eval_graph(&mut self, graph: &NodeGraph, clip: &Clip, t: f64, layers: &LayerSet) -> Option<glow::Texture> {
        let input = layers.get(clip.id).map(|f| (self.upload(clip.id, f), f.width, f.height));
        let size = input.map(|(_, w, h)| (w, h)).filter(|s| s.0 > 0 && s.1 > 0)?;
        let host = self.host_fbo();
        let out = self.eval_graph_on(graph, clip, t, layers, input.map(|(tex, ..)| tex), size, 1.0);
        self.restore(host);
        let out = out?;
        let tex = out.tex;
        self.loaned.push(out);
        Some(tex)
    }

    /// Drop cached textures for a clip/asset (source replaced, project closed).
    pub fn invalidate(&mut self, key: Option<u64>) {
        let gl = self.gl.clone();
        unsafe {
            match key {
                Some(k) => {
                    if let Some(t) = self.textures.remove(&k) {
                        gl.delete_texture(t.tex);
                    }
                }
                None => {
                    for (_, t) in self.textures.drain() {
                        gl.delete_texture(t.tex);
                    }
                }
            }
        }
    }

    pub fn scaler_supported(&self, _s: Scaler) -> bool {
        true
    }

    /// The GLSL log of the last program that failed to compile, if any.
    pub fn last_error(&self) -> Option<&str> {
        self.programs.failed.values().next().map(|s| s.as_str())
    }

    /// The GLSL log this user shader body was rejected with, if it ever was.
    pub fn shader_error(&self, src: &str) -> Option<&str> {
        self.programs.failed.get(&user_key(src)).map(|s| s.as_str())
    }

    /// Compile + link a user shader body without rendering anything, so the editor can show the log
    /// before the effect is applied. The program lands in the cache the renderer reads.
    pub fn check_shader(&mut self, src: &str) -> Result<(), String> {
        let key = user_key(src);
        if self.programs.map.contains_key(&key) {
            return Ok(());
        }
        if let Some(e) = self.programs.failed.get(&key) {
            return Err(e.clone());
        }
        self.link(key, &shaders::user_shader(src))
    }

    // ---------------- internals ----------------

    /// The framebuffer eframe/egui had bound when it called us.
    fn host_fbo(&self) -> Option<glow::Framebuffer> {
        unsafe {
            NonZeroU32::new(self.gl.get_parameter_i32(glow::DRAW_FRAMEBUFFER_BINDING) as u32)
                .map(glow::NativeFramebuffer)
        }
    }

    /// Put back the GL state egui assumes (its framebuffer, unit 0, no program bound).
    fn restore(&self, host: Option<glow::Framebuffer>) {
        unsafe {
            self.gl.bind_framebuffer(glow::FRAMEBUFFER, host);
            self.gl.active_texture(glow::TEXTURE0);
            self.gl.bind_texture(glow::TEXTURE_2D, None);
            self.gl.use_program(None);
        }
    }

    /// Take back everything handed out during the previous frame.
    fn reclaim(&mut self) {
        // ponytail: one frame of lag is enough because the UI paints within the frame it asked;
        // a fence/refcount would be needed if a texture ever outlived its frame.
        for t in std::mem::take(&mut self.loaned) {
            self.pool.put(t);
        }
    }

    fn acquire(&mut self, w: u32, h: u32) -> Option<Target> {
        if w == 0 || h == 0 {
            return None;
        }
        if let Some(t) = self.pool.take(w, h) {
            return Some(t);
        }
        let gl = self.gl.clone();
        unsafe {
            let tex = gl.create_texture().ok()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            tex_params(&gl);
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                w as i32,
                h as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            let fbo = gl.create_framebuffer().ok()?;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(tex), 0);
            if gl.check_framebuffer_status(glow::FRAMEBUFFER) != glow::FRAMEBUFFER_COMPLETE {
                gl.delete_framebuffer(fbo);
                gl.delete_texture(tex);
                gl.bind_framebuffer(glow::FRAMEBUFFER, None);
                return None;
            }
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            self.pool.created();
            Some(Target { tex, fbo, w, h })
        }
    }

    fn clear(&self, t: &Target, c: [f32; 4]) {
        let gl = &self.gl;
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(t.fbo));
            gl.disable(glow::SCISSOR_TEST);
            gl.disable(glow::BLEND);
            gl.clear_color(c[0], c[1], c[2], c[3]);
            gl.clear(glow::COLOR_BUFFER_BIT);
        }
    }

    /// Ensure a program is compiled and cached. `Err` carries the GLSL log.
    fn ensure(&mut self, key: u64) -> Result<(), String> {
        if self.programs.map.contains_key(&key) {
            return Ok(());
        }
        if let Some(e) = self.programs.failed.get(&key) {
            return Err(e.clone());
        }
        let src = match key {
            K_COMPOSITE => format!("{}{}{}", shaders::PRELUDE, shaders::BLEND, shaders::COMPOSITE),
            K_MASK => format!("{}{}", shaders::PRELUDE, shaders::MASK),
            K_COPY => format!("{}\n{}\n{}", shaders::PRELUDE, COPY_BODY, shaders::MAIN),
            K_MATTE => format!("{}\n{}\n{}", shaders::PRELUDE, MATTE_BODY, shaders::MAIN),
            _ => return Err("no source for this program".into()),
        };
        self.link(key, &src)
    }

    fn link(&mut self, key: u64, src: &str) -> Result<(), String> {
        let gl = self.gl.clone();
        let r = unsafe { build(&gl, self.vert, src) };
        match r {
            Ok(p) => {
                self.programs.map.insert(key, p);
                Ok(())
            }
            Err(e) => {
                // A failed effect program is otherwise a silent no-op: the chain just skips it.
                // `last_error()` is what the UI should toast; this keeps a dev run from guessing.
                if cfg!(debug_assertions) {
                    eprintln!("GLSL program {key:#x} failed to build:\n{e}");
                }
                self.programs.failed.insert(key, e.clone());
                Err(e)
            }
        }
    }

    /// Compile (once) the program for an effect; None when the kind has no GPU body.
    fn effect_program(&mut self, effect: &Effect) -> Option<u64> {
        let key =
            if effect.kind == EffectKind::Shader { user_key(&effect.shader) } else { EFFECT_BASE + effect.kind as u64 };
        if self.programs.map.contains_key(&key) {
            return Some(key);
        }
        if self.programs.failed.contains_key(&key) {
            return None;
        }
        let src = shaders::fragment(effect.kind, &effect.shader)?;
        self.link(key, &src).ok().map(|_| key)
    }

    /// Bind `prog`, point the fixed sampler units at `tex`/`mask`, size uniforms, and draw the triangle.
    fn draw(&self, key: u64, dst: &Target, tex: glow::Texture, mask: Option<glow::Texture>, set: impl Fn(&Prog)) {
        let Some(prog) = self.programs.map.get(&key) else { return };
        let gl = &self.gl;
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(dst.fbo));
            gl.viewport(0, 0, dst.w as i32, dst.h as i32);
            gl.disable(glow::BLEND);
            gl.disable(glow::SCISSOR_TEST);
            gl.disable(glow::DEPTH_TEST);
            gl.use_program(Some(prog.p));
            gl.active_texture(glow::TEXTURE0 + U_TEX);
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            gl.uniform_1_i32(prog.uni.get("tex"), U_TEX as i32);
            gl.active_texture(glow::TEXTURE0 + U_MASK);
            gl.bind_texture(glow::TEXTURE_2D, Some(mask.unwrap_or(self.blank)));
            gl.uniform_1_i32(prog.uni.get("u_mask"), U_MASK as i32);
            gl.uniform_1_i32(prog.uni.get("u_has_mask"), i32::from(mask.is_some()));
            set(prog);
            gl.bind_vertex_array(Some(self.vao));
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            gl.bind_vertex_array(None);
        }
    }

    /// Copy `src` (stretched to the target) into a fresh target.
    fn copy_to(&mut self, src: glow::Texture, size: (u32, u32)) -> Option<Target> {
        self.ensure(K_COPY).ok()?;
        let dst = self.acquire(size.0, size.1)?;
        let gl = self.gl.clone();
        self.draw(K_COPY, &dst, src, None, |p| unsafe {
            gl.uniform_2_f32(p.uni.get("u_res"), size.0 as f32, size.1 as f32);
        });
        Some(dst)
    }

    /// One effect pass: `src` -> a new target of the same size. None when the effect has no GPU body.
    /// `motion` is the (prev, next, count) sampler set a `needs_motion` kind wants; anything else
    /// passes None.
    #[allow(clippy::too_many_arguments)]
    fn run_effect(
        &mut self,
        src: glow::Texture,
        size: (u32, u32),
        effect: &Effect,
        t: f64,
        scale: f32,
        mask: Option<glow::Texture>,
        motion: Option<(glow::Texture, glow::Texture, f32)>,
        // BlobTrack only: (cx, cy, area) from engine::effects::track, in 0..1 layer coordinates
        blob: Option<(f64, f64, f64)>,
    ) -> Option<Target> {
        let key = self.effect_program(effect)?;
        let dst = self.acquire(size.0, size.1)?;
        let gl = self.gl.clone();
        let mut params = [0.0f32; PARAM_NAMES.len()];
        for (i, p) in params.iter_mut().enumerate() {
            *p = effect.at(i, t) as f32;
        }
        // uniforms persist on a cached program, so a motion kind must set these every draw
        let motion = effect.kind.needs_motion().then(|| motion.unwrap_or((self.blank, self.blank, 0.0)));
        self.draw(key, &dst, src, mask, |prog| unsafe {
            gl.uniform_2_f32(prog.uni.get("u_res"), size.0 as f32, size.1 as f32);
            gl.uniform_1_f32(prog.uni.get("u_time"), t as f32);
            gl.uniform_1_f32(prog.uni.get("u_scale"), scale);
            for (i, v) in params.iter().enumerate() {
                gl.uniform_1_f32(prog.uni.get(PARAM_NAMES[i]), *v);
            }
            if let Some((prev, next, frames)) = motion {
                gl.active_texture(glow::TEXTURE0 + U_PREV);
                gl.bind_texture(glow::TEXTURE_2D, Some(prev));
                gl.uniform_1_i32(prog.uni.get("u_prev"), U_PREV as i32);
                gl.active_texture(glow::TEXTURE0 + U_NEXT);
                gl.bind_texture(glow::TEXTURE_2D, Some(next));
                gl.uniform_1_i32(prog.uni.get("u_next"), U_NEXT as i32);
                gl.uniform_1_f32(prog.uni.get("u_frames"), frames);
                // ponytail: no per-frame placement delta yet, so the 0-sample fallback (clip edge)
                // blurs by nothing rather than guessing a direction.
                gl.uniform_2_f32(prog.uni.get("u_motion"), 0.0, 0.0);
            }
            if effect.kind == EffectKind::BlobTrack {
                // zero area = "nothing matched", which the shader reads as "draw no crosshair"
                let (cx, cy, area) = blob.unwrap_or((0.5, 0.5, 0.0));
                gl.uniform_3_f32(prog.uni.get("u_blob"), cx as f32, cy as f32, area as f32);
            }
        });
        Some(dst)
    }

    /// `BlobTrack`: find the target colour's centroid on the clip's decoded frame. The tracking runs on
    /// the CPU copy the decoder already produced, so no readback is needed; the shader only draws it.
    fn track_blob(&self, effect: &Effect, clip: Id, t: f64, layers: &LayerSet) -> Option<(f64, f64, f64)> {
        if effect.kind != EffectKind::BlobTrack {
            return None;
        }
        let frame = layers.get(clip)?;
        crate::engine::effects::track(frame, &param_values(effect, t))
    }

    /// The neighbouring frames a `needs_motion` effect wants: (prev, next, how many are real).
    /// `LayerSet::motion` carries whatever the decoder managed to produce — 0, 1 or 2 samples.
    fn motion_texs(&mut self, id: Id, layers: &LayerSet) -> (glow::Texture, glow::Texture, f32) {
        let (mut before, mut after) = (None, None);
        let (mut bo, mut ao) = (0.0f64, 0.0f64);
        for (cid, off, f) in &layers.motion {
            if *cid != id || f.is_empty() {
                continue;
            }
            if *off < bo {
                bo = *off;
                before = Some(f);
            } else if *off > ao {
                ao = *off;
                after = Some(f);
            }
        }
        match (before, after) {
            (None, None) => (self.blank, self.blank, 0.0),
            (Some(f), None) => {
                let tex = self.upload(id | MOTION_PREV, f);
                (tex, tex, 1.0)
            }
            (None, Some(f)) => {
                let tex = self.upload(id | MOTION_NEXT, f);
                (tex, tex, 1.0)
            }
            (Some(p), Some(n)) => {
                let prev = self.upload(id | MOTION_PREV, p);
                let next = self.upload(id | MOTION_NEXT, n);
                (prev, next, 2.0)
            }
        }
    }

    /// Rasterise `mask` at `size`; `scale` is canvas px per project px and `centre` the shape origin.
    fn render_mask(&mut self, mask: &Mask, size: (u32, u32), t: f64, scale: f32, centre: (f32, f32)) -> Option<Target> {
        self.ensure(K_MASK).ok()?;
        let dst = self.acquire(size.0, size.1)?;
        let gl = self.gl.clone();
        let mut pts = Vec::with_capacity(mask.points.len().min(MAX_MASK_POINTS) * 2);
        for (x, y) in mask.points.iter().take(MAX_MASK_POINTS) {
            pts.push(centre.0 + x * scale);
            pts.push(centre.1 + y * scale);
        }
        let n = (pts.len() / 2) as i32;
        let shape = match mask.shape {
            MaskShape::Rect => 0,
            MaskShape::Ellipse => 1,
            // ponytail: Path is rasterised as a polygon through its points — the editor densifies
            // curves, so a real bezier SDF only matters for very sparse paths.
            MaskShape::Polygon | MaskShape::Path => 2,
        };
        let (cx, cy) = (centre.0 + mask.cx.at(t) as f32 * scale, centre.1 + mask.cy.at(t) as f32 * scale);
        let (rx, ry) = (mask.rx.at(t) as f32 * scale, mask.ry.at(t) as f32 * scale);
        self.draw(K_MASK, &dst, self.blank, None, |p| unsafe {
            gl.uniform_2_f32(p.uni.get("u_res"), size.0 as f32, size.1 as f32);
            gl.uniform_1_i32(p.uni.get("m_shape"), shape);
            gl.uniform_2_f32(p.uni.get("m_center"), cx, cy);
            gl.uniform_2_f32(p.uni.get("m_radius"), rx.abs(), ry.abs());
            gl.uniform_1_f32(p.uni.get("m_rot"), mask.rotation.at(t) as f32);
            gl.uniform_1_f32(p.uni.get("m_feather"), (mask.feather.at(t) as f32 * scale).max(0.0));
            gl.uniform_1_f32(p.uni.get("m_expand"), mask.expand.at(t) as f32 * scale);
            gl.uniform_1_f32(p.uni.get("m_opacity"), mask.opacity.at(t).clamp(0.0, 1.0) as f32);
            gl.uniform_1_i32(p.uni.get("m_invert"), i32::from(mask.invert));
            gl.uniform_1_i32(p.uni.get("m_count"), n);
            if !pts.is_empty() {
                gl.uniform_2_f32_slice(p.uni.get("m_points"), &pts);
            }
        });
        Some(dst)
    }

    /// Composite `layer` onto `canvas` through `inv` (canvas px -> layer uv). Consumes and returns the
    /// canvas (ping-pong: you cannot sample the framebuffer you are drawing into).
    #[allow(clippy::too_many_arguments)]
    fn composite(
        &mut self,
        canvas: Target,
        layer: glow::Texture,
        layer_size: (u32, u32),
        inv: [[f32; 3]; 3],
        mode: BlendMode,
        opacity: f32,
        scaler: Scaler,
        mask: Option<glow::Texture>,
    ) -> Target {
        if self.ensure(K_COMPOSITE).is_err() {
            return canvas;
        }
        let Some(dst) = self.acquire(canvas.w, canvas.h) else { return canvas };
        let gl = self.gl.clone();
        let canvas_tex = canvas.tex;
        let m = column_major(&inv);
        let (cw, ch) = (canvas.w as f32, canvas.h as f32);
        let mode_i = blend_index(mode);
        let scaler_i = scaler_index(scaler);
        self.draw(K_COMPOSITE, &dst, layer, mask, |p| unsafe {
            gl.uniform_2_f32(p.uni.get("u_res"), cw, ch);
            gl.uniform_2_f32(p.uni.get("u_layer"), layer_size.0 as f32, layer_size.1 as f32);
            gl.uniform_matrix_3_f32_slice(p.uni.get("u_inv"), false, &m);
            gl.uniform_1_i32(p.uni.get("u_scaler"), scaler_i);
            gl.uniform_1_i32(p.uni.get("u_mode"), mode_i);
            gl.uniform_1_f32(p.uni.get("u_opacity"), opacity);
            gl.active_texture(glow::TEXTURE0 + U_DST);
            gl.bind_texture(glow::TEXTURE_2D, Some(canvas_tex));
            gl.uniform_1_i32(p.uni.get("u_dst"), U_DST as i32);
        });
        self.pool.put(canvas);
        dst
    }

    /// The whole timeline at `t` into a fresh canvas target.
    fn render_canvas(&mut self, project: &Project, t: f64, w: u32, h: u32, layers: &LayerSet) -> Option<Target> {
        let mut canvas = self.acquire(w, h)?;
        self.clear(&canvas, [0.0, 0.0, 0.0, 1.0]);
        for (ti, track) in project.tracks.iter().enumerate() {
            if track.kind != TrackKind::Video || !project.active(ti) {
                continue;
            }
            if let Some((tr, left, right)) = track.transition_at(t) {
                let p = crate::engine::compose::trans_progress(tr, left, right, t) as f32;
                canvas = self.draw_transition(project, tr, left, right, p, t, canvas, layers);
                continue;
            }
            for clip in &track.clips {
                if !clip.enabled || !clip.contains(t) {
                    continue;
                }
                canvas = self.draw_clip(project, clip, t, canvas, layers, Extra::NONE);
            }
        }
        if project.show_subtitles {
            if let Some(sub) = layers.get(LayerSet::SUBTITLES) {
                canvas = self.draw_subtitles(project, sub, canvas);
            }
        }
        Some(canvas)
    }

    /// Both clips of a transition, per kind (mirrors `compose::render_transition`).
    #[allow(clippy::too_many_arguments)]
    fn draw_transition(
        &mut self,
        project: &Project,
        tr: &crate::model::Transition,
        left: &Clip,
        right: &Clip,
        p: f32,
        t: f64,
        mut canvas: Target,
        layers: &LayerSet,
    ) -> Target {
        use crate::model::TransitionKind::*;
        let (w, h) = (canvas.w as f32, canvas.h as f32);
        match tr.kind {
            CrossFade => {
                canvas = self.draw_clip(project, left, t, canvas, layers, Extra::NONE);
                self.draw_clip(project, right, t, canvas, layers, Extra { opacity: p, ..Extra::NONE })
            }
            FadeToColor => {
                let (clip, fade) = if p < 0.5 { (left, 2.0 * p) } else { (right, 2.0 * (1.0 - p)) };
                canvas = self.draw_clip(project, clip, t, canvas, layers, Extra::NONE);
                self.fill_canvas(canvas, tr.color, fade.clamp(0.0, 1.0))
            }
            Push => {
                let (dx, dy) = match tr.direction {
                    0 => (-w, 0.0),
                    1 => (w, 0.0),
                    2 => (0.0, -h),
                    _ => (0.0, h),
                };
                let a = Extra { dx: -p * dx, dy: -p * dy, ..Extra::NONE };
                let b = Extra { dx: (1.0 - p) * dx, dy: (1.0 - p) * dy, ..Extra::NONE };
                canvas = self.draw_clip(project, left, t, canvas, layers, a);
                self.draw_clip(project, right, t, canvas, layers, b)
            }
            Wipe => {
                canvas = self.draw_clip(project, left, t, canvas, layers, Extra::NONE);
                // the incoming clip is limited to the wiped rectangle by a rect mask
                let (cx, cy, rx, ry) = match tr.direction {
                    0 => (w * p / 2.0, h / 2.0, w * p / 2.0, h / 2.0),
                    1 => (w - w * p / 2.0, h / 2.0, w * p / 2.0, h / 2.0),
                    2 => (w / 2.0, h * p / 2.0, w / 2.0, h * p / 2.0),
                    _ => (w / 2.0, h - h * p / 2.0, w / 2.0, h * p / 2.0),
                };
                let mut rect = Mask::new(MaskShape::Rect);
                rect.rx = crate::model::Animated::new(rx as f64);
                rect.ry = crate::model::Animated::new(ry as f64);
                let size = (canvas.w, canvas.h);
                let Some(mask) = self.render_mask(&rect, size, 0.0, 1.0, (cx, cy)) else { return canvas };
                let extra = Extra { mask: Some(mask.tex), ..Extra::NONE };
                let out = self.draw_clip(project, right, t, canvas, layers, extra);
                self.pool.put(mask);
                out
            }
        }
    }

    /// Mix the whole canvas towards `color` by `f` (FadeToColor).
    fn fill_canvas(&mut self, canvas: Target, color: [u8; 4], f: f32) -> Target {
        if f <= 0.0 {
            return canvas;
        }
        let Some(fill) = self.acquire(1, 1) else { return canvas };
        self.clear(&fill, [color[0] as f32 / 255.0, color[1] as f32 / 255.0, color[2] as f32 / 255.0, 1.0]);
        let inv = [[1.0 / canvas.w as f32, 0.0, 0.0], [0.0, 1.0 / canvas.h as f32, 0.0], [0.0, 0.0, 1.0]];
        let out = self.composite(canvas, fill.tex, (1, 1), inv, BlendMode::Normal, f.min(1.0), Scaler::Nearest, None);
        self.pool.put(fill);
        out
    }

    /// The subtitle bitmap, bottom-centre (mirrors `compose::render`).
    fn draw_subtitles(&mut self, project: &Project, sub: &Arc<Frame>, canvas: Target) -> Target {
        if sub.is_empty() {
            return canvas;
        }
        let (w, h) = (canvas.w, canvas.h);
        let sv = h as f32 / project.height.max(1) as f32;
        let p = crate::engine::compose::Placement {
            cx: w as f32 / 2.0,
            cy: h as f32 - project.subtitle_margin * sv - sub.height as f32 / 2.0,
            w: sub.width as f32,
            h: sub.height as f32,
            rot: 0.0,
            yaw: 0.0,
            pitch: 0.0,
        };
        let Some(inv) = inverse_placement(&p, w as f32, 0.0) else { return canvas };
        let tex = self.upload(LayerSet::SUBTITLES, sub);
        let size = (sub.width, sub.height);
        self.composite(canvas, tex, size, inv, BlendMode::Normal, 1.0, project.scaler, None)
    }

    /// One clip: layer -> chain -> composite. Returns the (possibly new) canvas.
    fn draw_clip(
        &mut self,
        project: &Project,
        clip: &Clip,
        t: f64,
        canvas: Target,
        layers: &LayerSet,
        extra: Extra,
    ) -> Target {
        let lt = clip.local(t);
        let opacity = clip.opacity.at(lt).clamp(0.0, 1.0) as f32 * extra.opacity;
        if opacity <= 0.0 || clip.kind == ClipKind::Audio {
            return canvas;
        }
        let (cw, ch) = (canvas.w, canvas.h);
        let s = cw as f32 / project.width.max(1) as f32;

        if clip.kind == ClipKind::Adjustment {
            // re-process everything already on the canvas
            let Some(out) = self.run_chain(clip, t, layers, canvas.tex, (cw, ch), s) else { return canvas };
            self.pool.put(canvas);
            return out;
        }

        let Some(frame) = layers.get(clip.id) else { return canvas };
        if frame.is_empty() {
            return canvas;
        }
        let contain = !matches!(clip.kind, ClipKind::Text | ClipKind::Shape);
        let native = self.native_size(project, clip, frame);
        let mut p = crate::engine::compose::placement(project, clip, t, native, cw, ch, contain);
        if !(p.w.is_finite() && p.h.is_finite() && p.w > 0.0 && p.h > 0.0) {
            return canvas;
        }
        // geometric effects move the layer instead of touching pixels
        let (mut focal, mut z0) = (cw as f32, 0.0f32);
        for e in clip.effects.iter().filter(|e| e.enabled && e.kind.is_geometric()) {
            match e.kind {
                EffectKind::Wobble => {
                    let (dx, dy, roll, yaw, pitch) = crate::engine::effects::wobble(e, lt);
                    p.cx += dx as f32 * s;
                    p.cy += dy as f32 * s;
                    p.rot += roll as f32;
                    p.yaw += yaw as f32;
                    p.pitch += pitch as f32;
                }
                EffectKind::Plane3d => {
                    p.yaw += e.at(0, lt) as f32;
                    p.pitch += e.at(1, lt) as f32;
                    p.rot += e.at(2, lt) as f32;
                    let fov = (e.at(4, lt) as f32).clamp(1.0, 179.0).to_radians();
                    focal = (cw as f32 / 2.0) / (fov / 2.0).tan() * (e.at(3, lt) as f32 / 2.0).max(0.01);
                    z0 = e.at(5, lt) as f32 * cw as f32;
                }
                _ => {}
            }
        }
        p.cx += extra.dx;
        p.cy += extra.dy;
        let Some(inv) = inverse_placement(&p, focal, z0) else { return canvas };

        let tex = self.upload(clip.id, frame);
        let lsize = (frame.width, frame.height);
        // effect scale: layer px per project px (matches compose::apply_effects' img_scale — footage
        // is decoded at roughly the placed size, text/shape bitmaps arrive at canvas scale already)
        let img_scale = if contain { s * (lsize.0 as f32 / p.w.max(1e-3)) } else { s };
        let processed = self.run_chain(clip, t, layers, tex, lsize, img_scale);
        let (src, size, own) = match &processed {
            Some(target) => (target.tex, (target.w, target.h), true),
            None => (tex, lsize, false),
        };
        let mask = match (extra.mask, &clip.mask) {
            (Some(m), _) => Some(MaskTex::Borrowed(m)),
            (None, Some(m)) if m.enabled => self.render_mask(m, (cw, ch), lt, s, (p.cx, p.cy)).map(MaskTex::Owned),
            _ => None,
        };
        let mask_tex = mask.as_ref().map(|m| m.tex());
        let out = self.composite(canvas, src, size, inv, clip.blend, opacity, project.scaler, mask_tex);
        if let Some(MaskTex::Owned(t)) = mask {
            self.pool.put(t);
        }
        if own {
            if let Some(t) = processed {
                self.pool.put(t);
            }
        }
        out
    }

    /// The clip's effect chain (node graph when it has one, else the linear stack). None = unchanged.
    fn run_chain(
        &mut self,
        clip: &Clip,
        t: f64,
        layers: &LayerSet,
        src: glow::Texture,
        size: (u32, u32),
        scale: f32,
    ) -> Option<Target> {
        let lt = clip.local(t);
        if let Some(g) = &clip.graph {
            return self.eval_graph_on(g, clip, t, layers, Some(src), size, scale);
        }
        let mut cur: Option<Target> = None;
        for e in &clip.effects {
            if !e.enabled || e.kind.is_geometric() {
                continue;
            }
            let input = cur.as_ref().map(|t| t.tex).unwrap_or(src);
            let mask = e
                .mask
                .as_ref()
                .filter(|m| m.enabled)
                .and_then(|m| self.render_mask(m, size, lt, scale, (size.0 as f32 / 2.0, size.1 as f32 / 2.0)));
            let motion = e.kind.needs_motion().then(|| self.motion_texs(clip.id, layers));
            let blob = self.track_blob(e, clip.id, lt, layers);
            let next = self.run_effect(input, size, e, lt, scale, mask.as_ref().map(|m| m.tex), motion, blob);
            if let Some(m) = mask {
                self.pool.put(m);
            }
            if let Some(n) = next {
                if let Some(old) = cur.replace(n) {
                    self.pool.put(old);
                }
            }
        }
        cur
    }

    /// Walk `graph` in evaluation order, one target per node. Missing inputs are transparent.
    #[allow(clippy::too_many_arguments)]
    fn eval_graph_on(
        &mut self,
        graph: &NodeGraph,
        clip: &Clip,
        t: f64,
        layers: &LayerSet,
        input: Option<glow::Texture>,
        size: (u32, u32),
        scale: f32,
    ) -> Option<Target> {
        let lt = clip.local(t);
        let order = graph.eval_order();
        let mut done: Vec<(Id, Target)> = Vec::with_capacity(order.len());
        let out_id = graph.output()?;
        for id in order {
            let Some(node) = graph.node(id) else { continue };
            let port = |p: usize| graph.input_of(id, p);
            let tex_of = |done: &Vec<(Id, Target)>, nid: Option<Id>| -> Option<glow::Texture> {
                let nid = nid?;
                done.iter().find(|(i, _)| *i == nid).map(|(_, t)| t.tex)
            };
            let a = tex_of(&done, port(0));
            let b = tex_of(&done, port(1));
            let result = if !node.enabled && node.kind.inputs() > 0 {
                a.and_then(|tex| self.copy_to(tex, size))
            } else {
                match &node.kind {
                    NodeKind::Input => match input {
                        Some(tex) => self.copy_to(tex, size),
                        None => self.transparent(size),
                    },
                    NodeKind::Color(c) => {
                        let target = self.acquire(size.0, size.1);
                        if let Some(target) = &target {
                            let a = c[3] as f32 / 255.0;
                            self.clear(target, [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, a]);
                        }
                        target
                    }
                    NodeKind::Clip(other) => match layers.get(*other) {
                        Some(f) if !f.is_empty() => {
                            let tex = self.upload(*other, f);
                            self.copy_to(tex, size)
                        }
                        _ => self.transparent(size),
                    },
                    NodeKind::Effect(_) if a.is_none() => self.transparent(size),
                    NodeKind::Effect(e) => {
                        let src = a.unwrap_or(self.blank);
                        let mask = e.mask.as_ref().filter(|m| m.enabled).and_then(|m| {
                            self.render_mask(m, size, lt, scale, (size.0 as f32 / 2.0, size.1 as f32 / 2.0))
                        });
                        let motion = e.kind.needs_motion().then(|| self.motion_texs(clip.id, layers));
                        let blob = self.track_blob(e, clip.id, lt, layers);
                        let r = self.run_effect(src, size, e, lt, scale, mask.as_ref().map(|m| m.tex), motion, blob);
                        if let Some(m) = mask {
                            self.pool.put(m);
                        }
                        // an effect with no GPU body passes its input through
                        match r {
                            Some(target) => Some(target),
                            None => self.copy_to(src, size),
                        }
                    }
                    NodeKind::Blend { mode, opacity } => {
                        let under = match a {
                            Some(tex) => self.copy_to(tex, size),
                            None => self.transparent(size),
                        };
                        match (under, b) {
                            (Some(under), Some(over)) => {
                                let inv =
                                    [[1.0 / size.0 as f32, 0.0, 0.0], [0.0, 1.0 / size.1 as f32, 0.0], [0.0, 0.0, 1.0]];
                                let op = opacity.at(lt).clamp(0.0, 1.0) as f32;
                                Some(self.composite(under, over, size, inv, *mode, op, Scaler::Bilinear, None))
                            }
                            (u, _) => u,
                        }
                    }
                    NodeKind::Matte { invert, use_alpha } => match (a, b) {
                        (Some(a), Some(b)) => self.matte(a, b, size, *invert, *use_alpha),
                        (Some(a), None) => self.copy_to(a, size),
                        _ => self.transparent(size),
                    },
                    NodeKind::Mask(m) => {
                        self.render_mask(m, size, lt, scale, (size.0 as f32 / 2.0, size.1 as f32 / 2.0))
                    }
                    NodeKind::Output => match a {
                        Some(tex) => self.copy_to(tex, size),
                        None => self.transparent(size),
                    },
                }
            };
            if let Some(r) = result {
                done.push((id, r));
            }
        }
        self.finish_graph(done, out_id)
    }

    /// Keep the output node's target, recycle the rest.
    fn finish_graph(&mut self, mut done: Vec<(Id, Target)>, out_id: Id) -> Option<Target> {
        let idx = done.iter().position(|(i, _)| *i == out_id);
        let out = idx.map(|i| done.swap_remove(i).1);
        for (_, t) in done {
            self.pool.put(t);
        }
        out
    }

    fn transparent(&mut self, size: (u32, u32)) -> Option<Target> {
        let t = self.acquire(size.0, size.1)?;
        self.clear(&t, [0.0; 4]);
        Some(t)
    }

    fn matte(
        &mut self,
        a: glow::Texture,
        b: glow::Texture,
        size: (u32, u32),
        invert: bool,
        use_alpha: bool,
    ) -> Option<Target> {
        self.ensure(K_MATTE).ok()?;
        let dst = self.acquire(size.0, size.1)?;
        let gl = self.gl.clone();
        self.draw(K_MATTE, &dst, a, None, |p| unsafe {
            gl.uniform_2_f32(p.uni.get("u_res"), size.0 as f32, size.1 as f32);
            gl.active_texture(glow::TEXTURE0 + U_B);
            gl.bind_texture(glow::TEXTURE_2D, Some(b));
            gl.uniform_1_i32(p.uni.get("u_b"), U_B as i32);
            gl.uniform_1_i32(p.uni.get("u_matte_alpha"), i32::from(use_alpha));
            gl.uniform_1_i32(p.uni.get("u_matte_invert"), i32::from(invert));
        });
        Some(dst)
    }

    /// The layer's "native" size for placement: the asset/sequence size for footage, the bitmap size
    /// for anything the caller rendered for us.
    fn native_size(&self, project: &Project, clip: &Clip, frame: &Frame) -> (u32, u32) {
        let wh = match clip.kind {
            ClipKind::Video | ClipKind::Image => project.asset(clip.asset).map(|a| (a.width, a.height)),
            ClipKind::Sequence => project.sequence(clip.sequence).map(|s| (s.width, s.height)),
            _ => None,
        };
        match wh {
            Some((w, h)) if w > 0 && h > 0 => (w, h),
            _ => (frame.width.max(1), frame.height.max(1)),
        }
    }
}

impl Drop for GpuRenderer {
    fn drop(&mut self) {
        let gl = self.gl.clone();
        unsafe {
            for (_, t) in self.textures.drain() {
                gl.delete_texture(t.tex);
            }
            for t in self.loaned.drain(..).chain(self.pool.free.drain(..)) {
                gl.delete_framebuffer(t.fbo);
                gl.delete_texture(t.tex);
            }
            for (_, p) in self.programs.map.drain() {
                gl.delete_program(p.p);
            }
            gl.delete_shader(self.vert);
            gl.delete_texture(self.blank);
            gl.delete_vertex_array(self.vao);
        }
    }
}

/// Either a mask we rendered (and must recycle) or one the caller owns.
enum MaskTex {
    Owned(Target),
    Borrowed(glow::Texture),
}

impl MaskTex {
    fn tex(&self) -> glow::Texture {
        match self {
            MaskTex::Owned(t) => t.tex,
            MaskTex::Borrowed(t) => *t,
        }
    }
}

/// Per-clip render tweaks used by transitions (mirrors `compose::Extra`).
#[derive(Clone, Copy)]
struct Extra {
    opacity: f32,
    dx: f32,
    dy: f32,
    mask: Option<glow::Texture>,
}

impl Extra {
    const NONE: Extra = Extra { opacity: 1.0, dx: 0.0, dy: 0.0, mask: None };
}

/// One name per parameter any effect can have (`Curves` has 12, everything else 8 or fewer).
const PARAM_NAMES: [&str; 12] = ["p0", "p1", "p2", "p3", "p4", "p5", "p6", "p7", "p8", "p9", "p10", "p11"];

/// `MASK`'s `m_points` array size.
pub const MAX_MASK_POINTS: usize = 64;

/// An effect's parameters at time `t`, in `EffectKind::params()` order.
fn param_values(effect: &Effect, t: f64) -> Vec<f64> {
    (0..effect.kind.params().len()).map(|i| effect.at(i, t)).collect()
}

/// Decoded layers for one timeline instant, produced by the player/export thread on the CPU and handed
/// to the renderer: (clip id, decoded frame). Motion-blur effects also get the neighbouring frames.
#[derive(Default)]
pub struct LayerSet {
    pub layers: Vec<(crate::model::Id, Arc<Frame>)>,
    /// Extra samples for shutter-based effects: (clip id, offset seconds, frame). `run_effect` binds
    /// the earliest as `u_prev` and the latest as `u_next` (both the same one when only one arrived);
    /// with none at all `u_frames` is 0 and the body falls back to the current frame.
    pub motion: Vec<(crate::model::Id, f64, Arc<Frame>)>,
}

impl LayerSet {
    /// Reserved id for the subtitle overlay (clip ids are never 0).
    pub const SUBTITLES: crate::model::Id = 0;

    pub fn get(&self, id: crate::model::Id) -> Option<&Arc<Frame>> {
        self.layers.iter().find(|(i, _)| *i == id).map(|(_, f)| f)
    }
}

// ---------------- pure helpers (tested without a GL context) ----------------

/// The canvas size `render_to_texture` actually renders at for a given preview quality. Callers must
/// rasterise text/shape/subtitle layers at *this* size (they are placed 1:1, not fitted).
pub fn render_size(w: u32, h: u32, quality: f32) -> (u32, u32) {
    let q = quality.clamp(0.1, 1.0);
    (((w as f32 * q).round() as u32).max(1), ((h as f32 * q).round() as u32).max(1))
}

/// The GLSL `int mode` for a blend mode: its index in `BlendMode::ALL`, which is the order
/// `engine::blend::blend_channel` matches.
pub fn blend_index(mode: BlendMode) -> i32 {
    BlendMode::ALL.iter().position(|m| *m == mode).unwrap_or(0) as i32
}

pub fn scaler_index(s: Scaler) -> i32 {
    match s {
        Scaler::Nearest => 0,
        Scaler::Bilinear => 1,
        Scaler::Bicubic => 2,
    }
}

/// The placed layer's four corners (TL, TR, BR, BL) in canvas pixels, projected through the
/// yaw/pitch tilt with focal length `f` and depth offset `z0`. `None` when a corner is behind the
/// camera. Mirrors `compose::draw_layer_perspective` (which uses f = canvas width, z0 = 0).
pub fn placement_quad(p: &crate::engine::compose::Placement, f: f32, z0: f32) -> Option<[[f32; 2]; 4]> {
    let (hw, hh) = (p.w / 2.0, p.h / 2.0);
    let (sy, cy) = p.yaw.to_radians().sin_cos();
    let (sp, cp) = p.pitch.to_radians().sin_cos();
    let (sr, cr) = p.rot.to_radians().sin_cos();
    let f = if f.is_finite() && f > 1.0 { f } else { 1.0 };
    let mut quad = [[0.0f32; 2]; 4];
    for (k, (x, y)) in [(-hw, -hh), (hw, -hh), (hw, hh), (-hw, hh)].into_iter().enumerate() {
        let x1 = x * cy;
        let z1 = -x * sy;
        let y2 = y * cp - z1 * sp;
        let z2 = y * sp + z1 * cp + z0;
        let d = f + z2;
        if d < f * 0.05 {
            return None; // corner (nearly) behind the camera
        }
        let px = f * x1 / d;
        let py = f * y2 / d;
        quad[k] = [p.cx + px * cr - py * sr, p.cy + px * sr + py * cr];
    }
    Some(quad)
}

/// Homography mapping the unit square (0,0)(1,0)(1,1)(0,1) onto `q` (Heckbert).
/// ponytail: copied from `compose::square_to_quad` (private there) so the GPU geometry cannot drift
/// from the CPU one — make that pair `pub(crate)` and this goes away.
pub fn square_to_quad(q: &[[f32; 2]; 4]) -> [[f32; 3]; 3] {
    let [p0, p1, p2, p3] = *q;
    let (dx1, dy1) = (p1[0] - p2[0], p1[1] - p2[1]);
    let (dx2, dy2) = (p3[0] - p2[0], p3[1] - p2[1]);
    let (sx, sy) = (p0[0] - p1[0] + p2[0] - p3[0], p0[1] - p1[1] + p2[1] - p3[1]);
    let (g, h) = if sx.abs() < 1e-6 && sy.abs() < 1e-6 {
        (0.0, 0.0)
    } else {
        let den = dx1 * dy2 - dx2 * dy1;
        if den.abs() < 1e-9 {
            (0.0, 0.0)
        } else {
            ((sx * dy2 - dx2 * sy) / den, (dx1 * sy - sx * dy1) / den)
        }
    };
    [
        [p1[0] - p0[0] + g * p1[0], p3[0] - p0[0] + h * p3[0], p0[0]],
        [p1[1] - p0[1] + g * p1[1], p3[1] - p0[1] + h * p3[1], p0[1]],
        [g, h, 1.0],
    ]
}

/// Inverse of a 3×3 (adjugate / det); None when singular.
pub fn invert3(m: &[[f32; 3]; 3]) -> Option<[[f32; 3]; 3]> {
    let [a, b, c] = [m[0][0] as f64, m[0][1] as f64, m[0][2] as f64];
    let [d, e, f] = [m[1][0] as f64, m[1][1] as f64, m[1][2] as f64];
    let [g, h, i] = [m[2][0] as f64, m[2][1] as f64, m[2][2] as f64];
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    if det.abs() < 1e-12 {
        return None;
    }
    let k = 1.0 / det;
    Some([
        [((e * i - f * h) * k) as f32, ((c * h - b * i) * k) as f32, ((b * f - c * e) * k) as f32],
        [((f * g - d * i) * k) as f32, ((a * i - c * g) * k) as f32, ((c * d - a * f) * k) as f32],
        [((d * h - e * g) * k) as f32, ((b * g - a * h) * k) as f32, ((a * e - b * d) * k) as f32],
    ])
}

/// Canvas pixels -> layer uv for a placement (what `COMPOSITE`'s `u_inv` wants).
pub fn inverse_placement(p: &crate::engine::compose::Placement, f: f32, z0: f32) -> Option<[[f32; 3]; 3]> {
    invert3(&square_to_quad(&placement_quad(p, f, z0)?))
}

/// Row-major 3×3 -> the column-major order `glUniformMatrix3fv` expects.
fn column_major(m: &[[f32; 3]; 3]) -> [f32; 9] {
    [m[0][0], m[1][0], m[2][0], m[0][1], m[1][1], m[2][1], m[0][2], m[1][2], m[2][2]]
}

/// Cache key of a user shader body — shared by `effect_program`, `check_shader` and `shader_error`
/// so the editor reads the very program the renderer would build.
fn user_key(src: &str) -> u64 {
    USER_BIT | hash_str(src)
}

fn hash_str(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish() & !USER_BIT
}

unsafe fn tex_params(gl: &glow::Context) {
    // Sampling is done in the shaders (alpha-weighted, matching the CPU compositor), so the fixed
    // function filter must not interpolate for us.
    gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
    gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
    gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
    gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
}

unsafe fn compile(gl: &glow::Context, kind: u32, src: &str) -> Result<glow::Shader, String> {
    let sh = gl.create_shader(kind).map_err(|e| format!("create_shader: {e}"))?;
    gl.shader_source(sh, src);
    gl.compile_shader(sh);
    if !gl.get_shader_compile_status(sh) {
        let log = gl.get_shader_info_log(sh);
        gl.delete_shader(sh);
        return Err(log);
    }
    Ok(sh)
}

unsafe fn build(gl: &glow::Context, vert: glow::Shader, frag_src: &str) -> Result<Prog, String> {
    let frag = compile(gl, glow::FRAGMENT_SHADER, frag_src)?;
    let p = gl.create_program().map_err(|e| format!("create_program: {e}"))?;
    gl.attach_shader(p, vert);
    gl.attach_shader(p, frag);
    gl.link_program(p);
    gl.detach_shader(p, vert);
    gl.detach_shader(p, frag);
    gl.delete_shader(frag);
    if !gl.get_program_link_status(p) {
        let log = gl.get_program_info_log(p);
        gl.delete_program(p);
        return Err(log);
    }
    let mut uni = HashMap::new();
    for name in UNIFORMS {
        if let Some(l) = gl.get_uniform_location(p, name) {
            uni.insert(*name, l);
        }
    }
    // arrays are reported as "name[0]" by some drivers
    if let Some(l) = gl.get_uniform_location(p, "m_points[0]") {
        uni.entry("m_points").or_insert(l);
    }
    Ok(Prog { p, uni })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::compose::Placement;
    use std::num::NonZeroU32;

    fn fake_target(w: u32, h: u32, id: u32) -> Target {
        // GL handles are never touched by the pool itself.
        Target {
            tex: glow::NativeTexture(NonZeroU32::new(id).unwrap()),
            fbo: glow::NativeFramebuffer(NonZeroU32::new(id).unwrap()),
            w,
            h,
        }
    }

    #[test]
    fn pool_reuses_only_matching_sizes() {
        let mut p = Pool::default();
        assert!(p.take(16, 16).is_none());
        p.created();
        assert_eq!(p.live, 1);
        p.put(fake_target(16, 16, 1));
        assert_eq!(p.live, 0);
        assert_eq!(p.free.len(), 1);
        // wrong size -> caller must create a new one
        assert!(p.take(32, 16).is_none());
        assert_eq!(p.free.len(), 1);
        let t = p.take(16, 16).expect("reuse");
        assert_eq!((t.w, t.h), (16, 16));
        assert_eq!(p.live, 1);
        assert!(p.free.is_empty());
        p.put(t);
        p.put(fake_target(8, 8, 2));
        assert_eq!(p.free.len(), 2);
        // live never goes negative
        assert_eq!(p.live, 0);
    }

    #[test]
    fn pool_evicts_the_oldest_when_the_free_list_is_over_budget() {
        let mut p = Pool::default();
        // one target of exactly the budget, then a second one: the first must be dropped
        let side = (Pool::MAX_FREE_PX as f64).sqrt() as u32;
        p.put(fake_target(side, side, 1));
        p.put(fake_target(64, 64, 2));
        assert_eq!(p.free.len(), 1, "the oversized free list must shrink");
        assert_eq!((p.free[0].w, p.free[0].h), (64, 64), "the newest target survives");
        // a normal working set is untouched
        let mut p = Pool::default();
        for i in 0..8 {
            p.put(fake_target(1920, 1080, i + 1));
        }
        assert_eq!(p.free.len(), 8, "8 × 1080p is well inside the budget");
    }

    #[test]
    fn every_effect_parameter_reaches_a_uniform() {
        let most = crate::model::EffectKind::ALL.iter().map(|k| k.params().len()).max().unwrap_or(0);
        assert!(most <= PARAM_NAMES.len(), "{most} params but only {} are uploaded", PARAM_NAMES.len());
        for n in PARAM_NAMES {
            assert!(UNIFORMS.contains(&n), "{n} is never located at link time");
        }
    }

    #[test]
    fn blend_int_mapping_matches_engine_blend() {
        assert_eq!(blend_index(BlendMode::Normal), 0);
        for (i, m) in BlendMode::ALL.iter().enumerate() {
            assert_eq!(blend_index(*m), i as i32, "{}", m.name());
        }
        // the GLSL switch must list the same modes in the same order
        let mut seen = Vec::new();
        for line in shaders::BLEND.lines() {
            let Some(rest) = line.split("mode == ").nth(1) else { continue };
            let Some(n) = rest.split(')').next().and_then(|s| s.trim().parse::<usize>().ok()) else {
                continue;
            };
            let Some(name) = line.split("// ").nth(1) else { continue };
            seen.push((n, name.trim().to_string()));
        }
        seen.dedup();
        assert_eq!(seen.len(), 12, "expected one branch per non-Normal mode: {seen:?}");
        for (n, name) in seen {
            let want = BlendMode::ALL[n].name().replace(' ', "");
            assert_eq!(name, want, "GLSL branch {n} is {name}, enum says {want}");
        }
    }

    #[test]
    fn render_size_scales_and_never_collapses() {
        assert_eq!(render_size(1920, 1080, 1.0), (1920, 1080));
        assert_eq!(render_size(1920, 1080, 0.5), (960, 540));
        assert_eq!(render_size(1, 1, 0.0), (1, 1), "quality is clamped, never zero-sized");
        assert_eq!(render_size(640, 480, 2.0), (640, 480), "never upscales past 1.0");
    }

    #[test]
    fn scaler_mapping() {
        assert_eq!(scaler_index(Scaler::Nearest), 0);
        assert_eq!(scaler_index(Scaler::Bilinear), 1);
        assert_eq!(scaler_index(Scaler::Bicubic), 2);
    }

    fn map(m: &[[f32; 3]; 3], x: f32, y: f32) -> (f32, f32) {
        let w = m[2][0] * x + m[2][1] * y + m[2][2];
        ((m[0][0] * x + m[0][1] * y + m[0][2]) / w, (m[1][0] * x + m[1][1] * y + m[1][2]) / w)
    }

    #[test]
    fn flat_quad_matches_placement_bounds() {
        let p = Placement { cx: 100.0, cy: 60.0, w: 80.0, h: 40.0, rot: 0.0, yaw: 0.0, pitch: 0.0 };
        let q = placement_quad(&p, 640.0, 0.0).unwrap();
        let (x0, y0, x1, y1) = p.bounds();
        assert_eq!(q, [[x0, y0], [x1, y0], [x1, y1], [x0, y1]]);
        // rotated: the quad's bbox is the placement's bbox
        let r = Placement { rot: 30.0, ..p };
        let q = placement_quad(&r, 640.0, 0.0).unwrap();
        let (bx0, by0, bx1, by1) = r.bounds();
        let xs: Vec<f32> = q.iter().map(|c| c[0]).collect();
        let ys: Vec<f32> = q.iter().map(|c| c[1]).collect();
        let mn = |v: &[f32]| v.iter().cloned().fold(f32::MAX, f32::min);
        let mx = |v: &[f32]| v.iter().cloned().fold(f32::MIN, f32::max);
        for (a, b) in [(mn(&xs), bx0), (mx(&xs), bx1), (mn(&ys), by0), (mx(&ys), by1)] {
            assert!((a - b).abs() < 1e-3, "{a} vs {b}");
        }
    }

    #[test]
    fn yaw_zero_is_the_affine_inverse() {
        let p = Placement { cx: 160.0, cy: 90.0, w: 320.0, h: 180.0, rot: 0.0, yaw: 0.0, pitch: 0.0 };
        let inv = inverse_placement(&p, 320.0, 0.0).unwrap();
        // the placement's corners map to the layer's uv corners
        for ((x, y), (u, v)) in [
            ((0.0f32, 0.0f32), (0.0f32, 0.0f32)),
            ((320.0, 0.0), (1.0, 0.0)),
            ((320.0, 180.0), (1.0, 1.0)),
            ((160.0, 90.0), (0.5, 0.5)),
        ] {
            let (mu, mv) = map(&inv, x, y);
            assert!((mu - u).abs() < 1e-4 && (mv - v).abs() < 1e-4, "({x},{y}) -> ({mu},{mv})");
        }
        // no perspective term
        assert!(inv[2][0].abs() < 1e-6 && inv[2][1].abs() < 1e-6);
    }

    #[test]
    fn yaw_tilts_and_stays_invertible() {
        let p = Placement { cx: 160.0, cy: 90.0, w: 200.0, h: 100.0, rot: 0.0, yaw: 40.0, pitch: 0.0 };
        let q = placement_quad(&p, 320.0, 0.0).unwrap();
        // positive yaw swings the right edge towards the camera, so it projects taller
        let left_h = (q[3][1] - q[0][1]).abs();
        let right_h = (q[2][1] - q[1][1]).abs();
        assert!(right_h > left_h, "{right_h} !> {left_h}");
        // ...and negative yaw does the opposite
        let q2 = placement_quad(&Placement { yaw: -40.0, ..p }, 320.0, 0.0).unwrap();
        assert!((q2[2][1] - q2[1][1]).abs() < (q2[3][1] - q2[0][1]).abs());
        let inv = inverse_placement(&p, 320.0, 0.0).unwrap();
        let (u, v) = map(&inv, p.cx, p.cy);
        assert!((0.0..=1.0).contains(&u) && (v - 0.5).abs() < 1e-3, "centre -> ({u},{v})");
        // a corner behind the camera is refused rather than rendered inside out
        let bad = Placement { w: 4000.0, yaw: 89.0, ..p };
        assert!(placement_quad(&bad, 320.0, 0.0).is_none());
    }

    #[test]
    fn matrix_uploads_column_major() {
        let m = [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0], [7.0, 8.0, 9.0]];
        assert_eq!(column_major(&m), [1.0, 4.0, 7.0, 2.0, 5.0, 8.0, 3.0, 6.0, 9.0]);
    }

    #[test]
    fn user_shader_keys_differ_per_source() {
        assert_ne!(hash_str("a"), hash_str("b"));
        assert_eq!(hash_str("a"), hash_str("a"));
        assert_eq!(hash_str("a") & USER_BIT, 0, "the user bit must be free for tagging");
        assert!((USER_BIT | hash_str("a")) > EFFECT_BASE + 64);
        // check_shader / shader_error look the program up by this key: a fixed source must not read
        // back the broken one's log (that was the whole point of the editor)
        assert_ne!(user_key("a"), user_key("b"));
        assert_eq!(user_key("a"), USER_BIT | hash_str("a"));
    }

    /// The GLSL text of every program the renderer can build (the assembly `ensure`/`effect_program`
    /// do, without a context). Only a driver can answer whether they link — run that from a windowed
    /// harness — but a duplicate `main()` or a missing `effect()` is visible from here.
    #[test]
    fn every_program_source_has_exactly_one_entry_point() {
        let mut sources = vec![
            format!("{}{}{}", shaders::PRELUDE, shaders::BLEND, shaders::COMPOSITE),
            format!("{}{}", shaders::PRELUDE, shaders::MASK),
            format!(
                "{}
{}
{}",
                shaders::PRELUDE,
                COPY_BODY,
                shaders::MAIN
            ),
            format!(
                "{}
{}
{}",
                shaders::PRELUDE,
                MATTE_BODY,
                shaders::MAIN
            ),
        ];
        sources.extend(
            crate::model::EffectKind::ALL.iter().filter_map(|k| shaders::fragment(*k, crate::model::DEFAULT_SHADER)),
        );
        for src in &sources {
            let n = src.matches("void main()").count();
            assert_eq!(n, 1, "{n} entry points in {}", &src[..80.min(src.len())]);
            assert!(src.starts_with("#version 330 core"), "{}", &src[..40.min(src.len())]);
            // `effect()` is only ever called by MAIN, and only bodies that define it get MAIN
            assert_eq!(
                src.contains("effect(src, uv)"),
                src.contains("vec4 effect(vec4 src, vec2 uv)"),
                "{}",
                &src[..80.min(src.len())]
            );
        }
    }
}
