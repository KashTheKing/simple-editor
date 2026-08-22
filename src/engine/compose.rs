//! Compositor: renders the timeline at time t into an RGBA canvas. Used by preview (small canvas)
//! and export (project-size canvas) — identical output, so preview == export.
//!
//! Layer order: video tracks bottom→top in `project.tracks` order (V1 first, then V2 drawn over it…).
//! For each active (`project.active(track)`), enabled, visual clip containing t:
//!   * Video/Image: `pool.video(asset.path).frame_at(clip.src_time(t), dw, dh, ..)` where (dw,dh) is the
//!     placement size (clamped to native size, min 1) — decoders scale for us.
//!   * Text: `text.render(style, canvas_w / project.width)` (already at canvas scale) — contain=false.
//!   * Sequence: the nested timeline is rendered recursively at `clip.src_time(t)` into its own canvas
//!     (the sequence's size, fit like a video layer), depth ≤ 8.
//!   * `clip.effects` run in stack order on the decoded layer (Wobble moves the placement instead:
//!     dx/dy/roll, and yaw/pitch render through a perspective homography).
//!   * Transform the layer into canvas space per `placement()` (sampling per `project.scaler`;
//!     straight row copies for the common axis-aligned case), then `blend::composite_row` /
//!     `blend::composite_rect`.
//! Transitions render both clips of the cut — extended virtually into the window, which is clamped to
//! the two clips (`trans_window`) so an over-long one cannot hide the rest of the track — and blend per
//! kind. Subtitles (`Project::cue_at`) draw last, bottom-centre.
//! Background is opaque black. Out-of-view layers are skipped.

use crate::engine::blend;
use crate::engine::effects;
use crate::engine::text::TextRasterizer;
use crate::media::{DecoderPool, Frame};
use crate::model::{
    BlendMode, Clip, ClipKind, Project, Scaler, TextStyle, Track, TrackKind, Transition, TransitionKind,
};

/// Recursion limit for nested sequences.
const MAX_DEPTH: usize = 8;

/// Where a layer lands on a canvas of (cw, ch) pixels: centre, size and rotation (degrees, clockwise).
/// `yaw`/`pitch` (degrees) tilt the layer about the vertical/horizontal axis (camera shake) — rendered
/// through a perspective homography with focal length ≈ canvas width; 0 = flat.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub cx: f32,
    pub cy: f32,
    pub w: f32,
    pub h: f32,
    pub rot: f32,
    pub yaw: f32,
    pub pitch: f32,
}

impl Placement {
    /// Axis-aligned bounding box (x0, y0, x1, y1) of the flat (yaw/pitch = 0) rect.
    pub fn bounds(&self) -> (f32, f32, f32, f32) {
        let (s, c) = self.rot.to_radians().sin_cos();
        let hw = self.w / 2.0;
        let hh = self.h / 2.0;
        let ex = (hw * c).abs() + (hh * s).abs();
        let ey = (hw * s).abs() + (hh * c).abs();
        (self.cx - ex, self.cy - ey, self.cx + ex, self.cy + ey)
    }
}

/// Placement of a clip's layer on a (cw, ch) canvas at timeline time t.
/// `native` = the layer image size. With `contain` (video/images) the image is first fitted inside the
/// canvas preserving aspect; text is already rendered at canvas scale so `contain = false`.
/// Then: scale about the centre by `clip.scale`, offset by (clip.x, clip.y) project pixels (scaled to
/// canvas), rotate by `clip.rotation`.
pub fn placement(
    project: &Project,
    clip: &Clip,
    t: f64,
    native: (u32, u32),
    cw: u32,
    ch: u32,
    contain: bool,
) -> Placement {
    placement_w(project.width, clip, t, native, cw, ch, contain)
}

/// `placement` with an explicit project width (nested sequences use the sequence's own size).
fn placement_w(pw: u32, clip: &Clip, t: f64, native: (u32, u32), cw: u32, ch: u32, contain: bool) -> Placement {
    let lt = clip.local(t);
    let s = cw as f32 / pw.max(1) as f32;
    let (nw, nh) = (native.0.max(1) as f32, native.1.max(1) as f32);
    let fit = if contain { (cw as f32 / nw).min(ch as f32 / nh) } else { 1.0 };
    let k = clip.scale.at(lt) as f32;
    Placement {
        cx: cw as f32 / 2.0 + clip.x.at(lt) as f32 * s,
        cy: ch as f32 / 2.0 + clip.y.at(lt) as f32 * s,
        w: nw * fit * k,
        h: nh * fit * k,
        rot: clip.rotation.at(lt) as f32,
        yaw: 0.0,
        pitch: 0.0,
    }
}

/// Per-clip render tweaks used by transitions (opacity fade, push offset, fade-to-colour).
#[derive(Clone, Copy)]
struct Extra {
    opacity: f32,
    dx: f32,
    dy: f32,
    /// Mix the layer's RGB towards `color` by this amount (FadeToColor).
    fade: f32,
    color: [u8; 4],
}

impl Extra {
    const NONE: Extra = Extra { opacity: 1.0, dx: 0.0, dy: 0.0, fade: 0.0, color: [0, 0, 0, 255] };
}

/// Half-width of a transition window, clamped so it never reaches past either clip of the cut.
/// `Transition::duration` is not clamped when it is set, and an over-long window would otherwise hide
/// every other clip on the track (and keep both clips playing past their own ends).
pub fn trans_half(tr: &Transition, left: &Clip, right: &Clip) -> f64 {
    tr.half(left, right)
}

/// Eased progress 0..1 across a transition window of `cut ± half`.
pub fn trans_progress_at(tr: &Transition, cut: f64, half: f64, t: f64) -> f64 {
    tr.progress_at(cut, half, t)
}

/// Eased progress 0..1 over the clamped window.
pub fn trans_progress(tr: &Transition, left: &Clip, right: &Clip, t: f64) -> f64 {
    tr.progress(left, right, t)
}

/// Mute/solo resolution over an arbitrary track list (mirrors `Project::active`).
fn track_active(tracks: &[Track], ti: usize) -> bool {
    let t = &tracks[ti];
    let any_solo = tracks.iter().any(|o| o.kind == t.kind && o.solo);
    if any_solo {
        t.solo
    } else {
        !t.muted
    }
}

/// Holds scratch buffers so rendering does not allocate per frame.
pub struct Compositor {
    layer: Frame,
    src: Frame,
    /// Effects scratch (blur passes).
    fx: Frame,
    /// Wipe-transition scratch canvas.
    wipe: Frame,
    /// Nested sequence canvases, one per recursion depth.
    seq: Vec<Frame>,
    /// Cached subtitle style (subtitle_style + current cue text) to avoid per-frame clones.
    sub_style: TextStyle,
    sub_key: u64,
}

impl Default for Compositor {
    fn default() -> Self {
        Self::new()
    }
}

impl Compositor {
    pub fn new() -> Self {
        Self {
            layer: Frame::default(),
            src: Frame::default(),
            fx: Frame::default(),
            wipe: Frame::default(),
            seq: Vec::new(),
            sub_style: TextStyle::subtitle_default(),
            sub_key: 0,
        }
    }

    /// Render timeline time `t` into `out` at w×h (out is resized). Never panics on missing media:
    /// a clip whose decoder fails is simply skipped.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        project: &Project,
        t: f64,
        w: u32,
        h: u32,
        pool: &mut DecoderPool,
        text: &mut TextRasterizer,
        out: &mut Frame,
    ) {
        self.render_tracks(project, &project.tracks, project.width, t, w, h, pool, text, out, 0);
        out.pts = t;
        if w == 0 || h == 0 {
            return;
        }
        if project.show_subtitles {
            if let Some(cue) = project.cue_at(t) {
                if !cue.text.trim().is_empty() {
                    let sk = project.subtitle_style.cache_key();
                    if self.sub_key != sk || self.sub_style.text != cue.text {
                        self.sub_style.clone_from(&project.subtitle_style);
                        self.sub_style.text.clone_from(&cue.text);
                        self.sub_key = sk;
                    }
                    let s = w as f32 / project.width.max(1) as f32;
                    let img = text.render(&self.sub_style, s);
                    if img.width > 1 || img.height > 1 {
                        let sv = h as f32 / project.height.max(1) as f32;
                        let p = Placement {
                            cx: w as f32 / 2.0,
                            cy: h as f32 - project.subtitle_margin * sv - img.height as f32 / 2.0,
                            w: img.width as f32,
                            h: img.height as f32,
                            rot: 0.0,
                            yaw: 0.0,
                            pitch: 0.0,
                        };
                        draw_layer(out, &img, p, BlendMode::Normal, 1.0, Scaler::Bilinear, &mut self.layer);
                    }
                }
            }
        }
    }

    /// Render a track list onto a fresh opaque-black canvas (recursion entry for nested sequences).
    /// `pw` = the "project width" the tracks' clips are placed against (project or sequence width).
    #[allow(clippy::too_many_arguments)]
    fn render_tracks(
        &mut self,
        project: &Project,
        tracks: &[Track],
        pw: u32,
        t: f64,
        w: u32,
        h: u32,
        pool: &mut DecoderPool,
        text: &mut TextRasterizer,
        out: &mut Frame,
        depth: usize,
    ) {
        out.resize(w, h);
        out.fill([0, 0, 0, 255]);
        if w == 0 || h == 0 {
            return;
        }
        for (ti, track) in tracks.iter().enumerate() {
            if track.kind != TrackKind::Video || !track_active(tracks, ti) {
                continue;
            }
            if let Some((tr, left, right)) = track.transition_at(t) {
                self.render_transition(project, pw, tr, left, right, t, w, h, pool, text, out, depth);
                continue;
            }
            for clip in &track.clips {
                if !clip.enabled || !clip.contains(t) {
                    continue;
                }
                self.render_clip(project, pw, clip, t, w, h, pool, text, out, depth, Extra::NONE);
            }
        }
    }

    /// Blend the two clips of a transition per kind at progress `tr.progress`.
    #[allow(clippy::too_many_arguments)]
    fn render_transition(
        &mut self,
        project: &Project,
        pw: u32,
        tr: &Transition,
        left: &Clip,
        right: &Clip,
        t: f64,
        w: u32,
        h: u32,
        pool: &mut DecoderPool,
        text: &mut TextRasterizer,
        out: &mut Frame,
        depth: usize,
    ) {
        let p = trans_progress(tr, left, right, t) as f32;
        match tr.kind {
            TransitionKind::CrossFade => {
                self.render_clip(project, pw, left, t, w, h, pool, text, out, depth, Extra::NONE);
                self.render_clip(
                    project,
                    pw,
                    right,
                    t,
                    w,
                    h,
                    pool,
                    text,
                    out,
                    depth,
                    Extra { opacity: p, ..Extra::NONE },
                );
            }
            TransitionKind::FadeToColor => {
                // A→colour for p < 0.5, colour→B after
                let (clip, fade) = if p < 0.5 { (left, 2.0 * p) } else { (right, 2.0 * (1.0 - p)) };
                let extra = Extra { fade: fade.clamp(0.0, 1.0), color: tr.color, ..Extra::NONE };
                self.render_clip(project, pw, clip, t, w, h, pool, text, out, depth, extra);
            }
            TransitionKind::Push => {
                let (dx, dy) = dir_vec(tr.direction, w, h);
                let a = Extra { dx: -p * dx, dy: -p * dy, ..Extra::NONE };
                let b = Extra { dx: (1.0 - p) * dx, dy: (1.0 - p) * dy, ..Extra::NONE };
                self.render_clip(project, pw, left, t, w, h, pool, text, out, depth, a);
                self.render_clip(project, pw, right, t, w, h, pool, text, out, depth, b);
            }
            TransitionKind::Wipe => {
                self.render_clip(project, pw, left, t, w, h, pool, text, out, depth, Extra::NONE);
                let mut tmp = std::mem::take(&mut self.wipe);
                tmp.resize(w, h);
                tmp.rgba.copy_from_slice(&out.rgba);
                tmp.pts = out.pts;
                self.render_clip(project, pw, right, t, w, h, pool, text, &mut tmp, depth, Extra::NONE);
                // copy the wiped region (where B has arrived) from tmp back into out
                let (x0, x1, y0, y1) = match tr.direction {
                    0 => (0, (w as f32 * p) as u32, 0, h),         // from the left
                    1 => ((w as f32 * (1.0 - p)) as u32, w, 0, h), // from the right
                    2 => (0, w, 0, (h as f32 * p) as u32),         // from the top
                    _ => (0, w, (h as f32 * (1.0 - p)) as u32, h), // from the bottom
                };
                let stride = out.stride();
                for y in y0..y1.min(h) {
                    let row = y as usize * stride;
                    let (a, b) = (row + x0.min(w) as usize * 4, row + x1.min(w) as usize * 4);
                    if a < b {
                        out.rgba[a..b].copy_from_slice(&tmp.rgba[a..b]);
                    }
                }
                self.wipe = tmp;
            }
        }
    }

    /// Render one clip's layer onto `out`. Does NOT check `contains` — transitions render clips
    /// virtually extended past their window (source times are clamped to the asset/sequence).
    #[allow(clippy::too_many_arguments)]
    fn render_clip(
        &mut self,
        project: &Project,
        pw: u32,
        clip: &Clip,
        t: f64,
        w: u32,
        h: u32,
        pool: &mut DecoderPool,
        text: &mut TextRasterizer,
        out: &mut Frame,
        depth: usize,
        extra: Extra,
    ) {
        let lt = clip.local(t);
        let opacity = clip.opacity.at(lt).clamp(0.0, 1.0) as f32 * extra.opacity;
        if opacity <= 0.0 {
            return;
        }
        let s = w as f32 / pw.max(1) as f32;
        match clip.kind {
            // TODO(engine-gpu/shapes): Shape draws its vector/drawing layer; Adjustment re-processes
            // everything already on the canvas (both land in round 3's renderer)
            ClipKind::Shape | ClipKind::Adjustment => {}
            ClipKind::Video | ClipKind::Image => {
                let Some(asset) = project.asset(clip.asset) else {
                    return;
                };
                let Some(dec) = pool.video(&asset.path) else {
                    return;
                };
                let mut native = (asset.width, asset.height);
                if native.0 == 0 || native.1 == 0 {
                    native = dec.size();
                }
                if native.0 == 0 || native.1 == 0 {
                    return;
                }
                let mut p = placement_w(pw, clip, t, native, w, h, true);
                if !(p.w.is_finite() && p.h.is_finite()) {
                    return;
                }
                // Keyframed scale on an image: decode once at the size the largest key needs (images
                // always go through ffmpeg, which re-decodes per requested size); draw_layer resizes.
                let pd = match clip.scale.keys.iter().max_by(|a, b| a.v.total_cmp(&b.v)) {
                    Some(k) if clip.kind == ClipKind::Image => {
                        placement_w(pw, clip, clip.start + k.t, native, w, h, true)
                    }
                    _ => p,
                };
                let dw = (pd.w.round() as u32).clamp(1, native.0);
                let dh = (pd.h.round() as u32).clamp(1, native.1);
                let st = if asset.duration > 0.0 {
                    clip.src_time(t).clamp(0.0, (asset.duration - 1e-4).max(0.0))
                } else {
                    clip.src_time(t).max(0.0)
                };
                if !dec.frame_at(st, dw, dh, &mut self.src) {
                    return;
                }
                let img_scale = s * (dw as f32 / p.w.max(1e-3));
                apply_effects(clip, lt, s, img_scale, &mut self.src, &mut self.fx, &mut p);
                fade_to(&mut self.src, extra.color, extra.fade);
                p.cx += extra.dx;
                p.cy += extra.dy;
                draw_layer(out, &self.src, p, clip.blend, opacity, project.scaler, &mut self.layer);
            }
            ClipKind::Text => {
                let Some(style) = &clip.text else { return };
                let img = text.render(style, s);
                let mut p = placement_w(pw, clip, t, (img.width, img.height), w, h, false);
                // a fade also needs the copy path — fade_to writes into the layer, and the cached text
                // bitmap is shared
                if clip.effects.iter().any(|e| e.enabled) || extra.fade > 0.0 {
                    self.src.resize(img.width, img.height);
                    self.src.rgba.copy_from_slice(&img.rgba);
                    apply_effects(clip, lt, s, s, &mut self.src, &mut self.fx, &mut p);
                    fade_to(&mut self.src, extra.color, extra.fade);
                    p.cx += extra.dx;
                    p.cy += extra.dy;
                    draw_layer(out, &self.src, p, clip.blend, opacity, project.scaler, &mut self.layer);
                } else {
                    p.cx += extra.dx;
                    p.cy += extra.dy;
                    draw_layer(out, &img, p, clip.blend, opacity, project.scaler, &mut self.layer);
                }
            }
            ClipKind::Sequence => {
                if depth >= MAX_DEPTH {
                    return;
                }
                let (qw, qh) = if project.editing == Some(clip.sequence) {
                    (project.width, project.height) // its live size is swapped into the project
                } else {
                    match project.sequence(clip.sequence) {
                        Some(sq) => (sq.width, sq.height),
                        None => return,
                    }
                };
                let Some(seq_tracks) = project.sequence_tracks(clip.sequence) else {
                    return;
                };
                if qw == 0 || qh == 0 {
                    return;
                }
                let mut p = placement_w(pw, clip, t, (qw, qh), w, h, true);
                if !(p.w.is_finite() && p.h.is_finite() && p.w > 0.0 && p.h > 0.0) {
                    return;
                }
                // render the nested timeline at the placed size (capped at the sequence's own size,
                // like a decode request) — draw_layer scales the rest
                let dw = (p.w.round() as u32).clamp(1, qw);
                let dh = (p.h.round() as u32).clamp(1, qh);
                let dur = project.sequence_duration(clip.sequence);
                let st = clip.src_time(t).clamp(0.0, (dur - 1e-6).max(0.0));
                while self.seq.len() <= depth {
                    self.seq.push(Frame::default());
                }
                let mut nested = std::mem::take(&mut self.seq[depth]);
                self.render_tracks(project, seq_tracks, qw, st, dw, dh, pool, text, &mut nested, depth + 1);
                let img_scale = s * (dw as f32 / p.w.max(1e-3));
                apply_effects(clip, lt, s, img_scale, &mut nested, &mut self.fx, &mut p);
                fade_to(&mut nested, extra.color, extra.fade);
                p.cx += extra.dx;
                p.cy += extra.dy;
                draw_layer(out, &nested, p, clip.blend, opacity, project.scaler, &mut self.layer);
                self.seq[depth] = nested;
            }
            ClipKind::Audio => {}
        }
    }
}

/// Direction unit vector scaled to canvas size: 0 = left, 1 = right, 2 = up, 3 = down.
fn dir_vec(direction: u8, w: u32, h: u32) -> (f32, f32) {
    match direction {
        0 => (-(w as f32), 0.0),
        1 => (w as f32, 0.0),
        2 => (0.0, -(h as f32)),
        _ => (0.0, h as f32),
    }
}

/// Run the clip's effect stack on the layer image; geometric effects (Wobble) move `p` instead.
/// `s` = canvas px per project px; `img_scale` = layer-image px per project px.
fn apply_effects(clip: &Clip, lt: f64, s: f32, img_scale: f32, img: &mut Frame, fx: &mut Frame, p: &mut Placement) {
    for e in &clip.effects {
        if !e.enabled {
            continue;
        }
        if effects::is_geometric(e.kind) {
            let (dx, dy, roll, yaw, pitch) = effects::wobble(e, lt);
            p.cx += dx as f32 * s;
            p.cy += dy as f32 * s;
            p.rot += roll as f32;
            p.yaw += yaw as f32;
            p.pitch += pitch as f32;
        } else {
            effects::apply(e, lt, img_scale, img, fx);
        }
    }
}

/// Mix the layer's RGB towards `color` by `f` (FadeToColor transitions).
fn fade_to(img: &mut Frame, color: [u8; 4], f: f32) {
    if f <= 0.0 {
        return;
    }
    let f = f.min(1.0);
    let mut lut = [[0u8; 256]; 3];
    for (c, l) in lut.iter_mut().enumerate() {
        for (v, o) in l.iter_mut().enumerate() {
            *o = (v as f32 + (color[c] as f32 - v as f32) * f + 0.5) as u8;
        }
    }
    for px in img.rgba.chunks_exact_mut(4) {
        px[0] = lut[0][px[0] as usize];
        px[1] = lut[1][px[1] as usize];
        px[2] = lut[2][px[2] as usize];
    }
}

/// Sample `src` at (sx, sy) — already clamped to [0, sw-1]/[0, sh-1] — returning straight RGBA
/// (r, g, b in 0..255, a in 0..255).
#[inline]
fn sample(src: &Frame, sx: f32, sy: f32, scaler: Scaler) -> [f32; 4] {
    let (sw, sh) = (src.width as usize, src.height as usize);
    let sstride = src.stride();
    match scaler {
        Scaler::Nearest => {
            let ix = (sx + 0.5) as usize;
            let iy = (sy + 0.5) as usize;
            let o = iy.min(sh - 1) * sstride + ix.min(sw - 1) * 4;
            let p = &src.rgba[o..o + 4];
            [p[0] as f32, p[1] as f32, p[2] as f32, p[3] as f32]
        }
        Scaler::Bilinear => {
            let (fx, fy) = (sx.floor(), sy.floor());
            let (tx, ty) = (sx - fx, sy - fy);
            let ix0 = fx as usize;
            let iy0 = fy as usize;
            let ix1 = (ix0 + 1).min(sw - 1);
            let iy1 = (iy0 + 1).min(sh - 1);
            let taps = [
                (iy0 * sstride + ix0 * 4, (1.0 - tx) * (1.0 - ty)),
                (iy0 * sstride + ix1 * 4, tx * (1.0 - ty)),
                (iy1 * sstride + ix0 * 4, (1.0 - tx) * ty),
                (iy1 * sstride + ix1 * 4, tx * ty),
            ];
            let (mut r, mut g, mut b, mut a) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
            for (off, wgt) in taps {
                let s = &src.rgba[off..off + 4];
                let wa = wgt * s[3] as f32;
                if wa > 0.0 {
                    r += wa * s[0] as f32;
                    g += wa * s[1] as f32;
                    b += wa * s[2] as f32;
                    a += wa;
                }
            }
            if a <= 0.0 {
                return [0.0; 4];
            }
            [r / a, g / a, b / a, a]
        }
        Scaler::Bicubic => {
            // 16-tap Catmull-Rom, premultiplied accumulation, clamped edges
            let (fx, fy) = (sx.floor(), sy.floor());
            let (tx, ty) = (sx - fx, sy - fy);
            let wx = catmull(tx);
            let wy = catmull(ty);
            let (ix, iy) = (fx as isize, fy as isize);
            let (mut r, mut g, mut b, mut a) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
            for (j, wyj) in wy.iter().enumerate() {
                let y = (iy + j as isize - 1).clamp(0, sh as isize - 1) as usize;
                let row = y * sstride;
                for (i, wxi) in wx.iter().enumerate() {
                    let x = (ix + i as isize - 1).clamp(0, sw as isize - 1) as usize;
                    let wgt = wyj * wxi;
                    let s = &src.rgba[row + x * 4..row + x * 4 + 4];
                    let wa = wgt * s[3] as f32;
                    r += wa * s[0] as f32;
                    g += wa * s[1] as f32;
                    b += wa * s[2] as f32;
                    a += wa;
                }
            }
            if a <= 0.0 {
                return [0.0; 4];
            }
            [(r / a).clamp(0.0, 255.0), (g / a).clamp(0.0, 255.0), (b / a).clamp(0.0, 255.0), a.clamp(0.0, 255.0)]
        }
    }
}

/// Catmull-Rom weights for taps at offsets -1, 0, 1, 2 for fractional position `t`.
#[inline]
fn catmull(t: f32) -> [f32; 4] {
    let t2 = t * t;
    let t3 = t2 * t;
    [0.5 * (-t3 + 2.0 * t2 - t), 0.5 * (3.0 * t3 - 5.0 * t2 + 2.0), 0.5 * (-3.0 * t3 + 4.0 * t2 + t), 0.5 * (t3 - t2)]
}

/// Draw `src` into `dst` (canvas) at `p` with blend mode & opacity, using `scratch` as the canvas-sized
/// layer buffer. Sampling per `scaler`; pixels outside `src` are transparent.
pub fn draw_layer(
    dst: &mut Frame,
    src: &Frame,
    p: Placement,
    mode: BlendMode,
    opacity: f32,
    scaler: Scaler,
    scratch: &mut Frame,
) {
    if dst.is_empty() || src.is_empty() || !(opacity > 0.0) {
        return;
    }
    if !(p.cx.is_finite() && p.cy.is_finite() && p.w > 0.0 && p.h > 0.0 && p.w.is_finite() && p.h.is_finite()) {
        return;
    }
    if p.yaw != 0.0 || p.pitch != 0.0 {
        draw_layer_perspective(dst, src, p, mode, opacity, scaler, scratch);
        return;
    }
    let (cw, ch) = (dst.width as i64, dst.height as i64);
    let (sw, sh) = (src.width as i64, src.height as i64);

    // (a) axis-aligned, 1:1 — composite src rows straight onto the canvas at an integer offset.
    if p.rot == 0.0 && (p.w - sw as f32).abs() < 0.5 && (p.h - sh as f32).abs() < 0.5 {
        let ox = (p.cx - sw as f32 / 2.0).round() as i64;
        let oy = (p.cy - sh as f32 / 2.0).round() as i64;
        let (x0, x1) = (ox.max(0), (ox + sw).min(cw));
        let (y0, y1) = (oy.max(0), (oy + sh).min(ch));
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let (ds, ss) = (dst.stride(), src.stride());
        for y in y0..y1 {
            let d = y as usize * ds;
            let s = (y - oy) as usize * ss;
            let (dx0, dx1) = (x0 as usize * 4, x1 as usize * 4);
            let (sx0, sx1) = ((x0 - ox) as usize * 4, (x1 - ox) as usize * 4);
            blend::composite_row(&mut dst.rgba[d + dx0..d + dx1], &src.rgba[s + sx0..s + sx1], mode, opacity);
        }
        return;
    }

    // (b) general: inverse-map the pixels inside bounds ∩ canvas, sampling per `scaler`.
    let (bx0, by0, bx1, by1) = p.bounds();
    let x0 = (bx0.floor() as i64).clamp(0, cw) as u32;
    let y0 = (by0.floor() as i64).clamp(0, ch) as u32;
    let x1 = (bx1.ceil() as i64).clamp(0, cw) as u32;
    let y1 = (by1.ceil() as i64).clamp(0, ch) as u32;
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    scratch.resize(dst.width, dst.height);
    let (sin, cos) = p.rot.to_radians().sin_cos();
    let (hw, hh) = (p.w / 2.0, p.h / 2.0);
    let (kx, ky) = (sw as f32 / p.w, sh as f32 / p.h);
    let (swm, shm) = ((sw - 1) as f32, (sh - 1) as f32);
    let stride = scratch.stride();
    for y in y0..y1 {
        let row = y as usize * stride;
        let out = &mut scratch.rgba[row + x0 as usize * 4..row + x1 as usize * 4];
        let dy = y as f32 + 0.5 - p.cy;
        for (i, o) in out.chunks_exact_mut(4).enumerate() {
            let dx = (x0 + i as u32) as f32 + 0.5 - p.cx;
            // canvas → layer space (undo the clockwise rotation)
            let u = dx * cos + dy * sin;
            let v = -dx * sin + dy * cos;
            // 1px feathered rectangle coverage
            let cov = ((hw - u.abs()).min(hh - v.abs()) + 0.5).clamp(0.0, 1.0);
            if cov <= 0.0 {
                o.copy_from_slice(&[0, 0, 0, 0]);
                continue;
            }
            let sx = ((u + hw) * kx - 0.5).clamp(0.0, swm);
            let sy = ((v + hh) * ky - 0.5).clamp(0.0, shm);
            let c = sample(src, sx, sy, scaler);
            if c[3] <= 0.0 {
                o.copy_from_slice(&[0, 0, 0, 0]);
                continue;
            }
            o[0] = (c[0] + 0.5) as u8;
            o[1] = (c[1] + 0.5) as u8;
            o[2] = (c[2] + 0.5) as u8;
            o[3] = (c[3] * cov + 0.5) as u8;
        }
    }
    blend::composite_rect(dst, scratch, mode, opacity, x0, y0, x1, y1);
}

/// Perspective path: project the placed rect's corners tilted by yaw (about Y) and pitch (about X)
/// with focal length = canvas width, then inverse-map each canvas pixel through the homography.
#[allow(clippy::too_many_arguments)]
fn draw_layer_perspective(
    dst: &mut Frame,
    src: &Frame,
    p: Placement,
    mode: BlendMode,
    opacity: f32,
    scaler: Scaler,
    scratch: &mut Frame,
) {
    let f = dst.width as f32;
    let (hw, hh) = (p.w / 2.0, p.h / 2.0);
    let (sy_, cy_) = p.yaw.to_radians().sin_cos();
    let (sp, cp) = p.pitch.to_radians().sin_cos();
    let (sr, cr) = p.rot.to_radians().sin_cos();
    // rect corners TL, TR, BR, BL ↔ unit square (0,0) (1,0) (1,1) (0,1)
    let mut quad = [[0.0f32; 2]; 4];
    for (k, (x, y)) in [(-hw, -hh), (hw, -hh), (hw, hh), (-hw, hh)].into_iter().enumerate() {
        let x1 = x * cy_;
        let z1 = -x * sy_;
        let y2 = y * cp - z1 * sp;
        let z2 = y * sp + z1 * cp;
        let d = f + z2;
        if d < f * 0.05 {
            return; // corner (nearly) behind the camera — skip the layer
        }
        let px = f * x1 / d;
        let py = f * y2 / d;
        quad[k] = [p.cx + px * cr - py * sr, p.cy + px * sr + py * cr];
    }
    let hm = square_to_quad(&quad);
    let Some(inv) = invert3(&hm) else { return };
    let (cw, ch) = (dst.width as i64, dst.height as i64);
    let (mut bx0, mut by0, mut bx1, mut by1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for q in &quad {
        bx0 = bx0.min(q[0]);
        by0 = by0.min(q[1]);
        bx1 = bx1.max(q[0]);
        by1 = by1.max(q[1]);
    }
    let x0 = (bx0.floor() as i64).clamp(0, cw) as u32;
    let y0 = (by0.floor() as i64).clamp(0, ch) as u32;
    let x1 = (bx1.ceil() as i64).clamp(0, cw) as u32;
    let y1 = (by1.ceil() as i64).clamp(0, ch) as u32;
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    scratch.resize(dst.width, dst.height);
    let (sw, sh) = (src.width as f32, src.height as f32);
    let (swm, shm) = (sw - 1.0, sh - 1.0);
    let stride = scratch.stride();
    for y in y0..y1 {
        let row = y as usize * stride;
        let out = &mut scratch.rgba[row + x0 as usize * 4..row + x1 as usize * 4];
        let py = y as f32 + 0.5;
        for (i, o) in out.chunks_exact_mut(4).enumerate() {
            let px = (x0 + i as u32) as f32 + 0.5;
            let ww = inv[2][0] * px + inv[2][1] * py + inv[2][2];
            if ww.abs() < 1e-9 {
                o.copy_from_slice(&[0, 0, 0, 0]);
                continue;
            }
            let u = (inv[0][0] * px + inv[0][1] * py + inv[0][2]) / ww;
            let v = (inv[1][0] * px + inv[1][1] * py + inv[1][2]) / ww;
            // feathered coverage ≈ distance to the unit-square edge in source pixels
            let cov = ((u.min(1.0 - u) * sw).min(v.min(1.0 - v) * sh) + 0.5).clamp(0.0, 1.0);
            if cov <= 0.0 || ww < 0.0 {
                o.copy_from_slice(&[0, 0, 0, 0]);
                continue;
            }
            let sx = (u * sw - 0.5).clamp(0.0, swm);
            let sy = (v * sh - 0.5).clamp(0.0, shm);
            let c = sample(src, sx, sy, scaler);
            if c[3] <= 0.0 {
                o.copy_from_slice(&[0, 0, 0, 0]);
                continue;
            }
            o[0] = (c[0] + 0.5) as u8;
            o[1] = (c[1] + 0.5) as u8;
            o[2] = (c[2] + 0.5) as u8;
            o[3] = (c[3] * cov + 0.5) as u8;
        }
    }
    blend::composite_rect(dst, scratch, mode, opacity, x0, y0, x1, y1);
}

/// Homography mapping the unit square (0,0)(1,0)(1,1)(0,1) onto `q` (Heckbert).
fn square_to_quad(q: &[[f32; 2]; 4]) -> [[f32; 3]; 3] {
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
fn invert3(m: &[[f32; 3]; 3]) -> Option<[[f32; 3]; 3]> {
    let a = m[0][0] as f64;
    let b = m[0][1] as f64;
    let c = m[0][2] as f64;
    let d = m[1][0] as f64;
    let e = m[1][1] as f64;
    let f = m[1][2] as f64;
    let g = m[2][0] as f64;
    let h = m[2][1] as f64;
    let i = m[2][2] as f64;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    Some([
        [((e * i - f * h) * inv) as f32, ((c * h - b * i) * inv) as f32, ((b * f - c * e) * inv) as f32],
        [((f * g - d * i) * inv) as f32, ((a * i - c * g) * inv) as f32, ((c * d - a * f) * inv) as f32],
        [((d * h - e * g) * inv) as f32, ((b * g - a * h) * inv) as f32, ((a * e - b * d) * inv) as f32],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::{Backend, VideoSource};
    use crate::model::{Asset, ClipKind, Ease, Keyframe, Project, TransitionKind};

    fn px(f: &Frame, x: u32, y: u32) -> [u8; 4] {
        let i = (y * f.width + x) as usize * 4;
        [f.rgba[i], f.rgba[i + 1], f.rgba[i + 2], f.rgba[i + 3]]
    }

    fn black(w: u32, h: u32) -> Frame {
        let mut f = Frame::new(w, h);
        f.fill([0, 0, 0, 255]);
        f
    }

    fn pl(cx: f32, cy: f32, w: f32, h: f32, rot: f32) -> Placement {
        Placement { cx, cy, w, h, rot, yaw: 0.0, pitch: 0.0 }
    }

    #[test]
    fn draw_layer_scale_2_centred() {
        let mut dst = black(8, 8);
        let mut src = Frame::new(2, 2);
        src.fill([255, 0, 0, 255]);
        let mut scratch = Frame::default();
        let p = pl(4.0, 4.0, 4.0, 4.0, 0.0);
        draw_layer(&mut dst, &src, p, BlendMode::Normal, 1.0, Scaler::Bilinear, &mut scratch);
        for y in 0..8 {
            for x in 0..8 {
                let inside = (2..6).contains(&x) && (2..6).contains(&y);
                let want = if inside { [255, 0, 0, 255] } else { [0, 0, 0, 255] };
                assert_eq!(px(&dst, x, y), want, "at {x},{y}");
            }
        }
    }

    #[test]
    fn draw_layer_fast_path_offset_and_clip() {
        let mut dst = black(4, 4);
        let mut src = Frame::new(2, 2);
        src.fill([0, 255, 0, 255]);
        let mut scratch = Frame::default();
        // native size at the top-left corner, 50% opacity
        let p = pl(1.0, 1.0, 2.0, 2.0, 0.0);
        draw_layer(&mut dst, &src, p, BlendMode::Normal, 0.5, Scaler::Bilinear, &mut scratch);
        assert_eq!(px(&dst, 0, 0), [0, 128, 0, 255]);
        assert_eq!(px(&dst, 1, 1), [0, 128, 0, 255]);
        assert_eq!(px(&dst, 2, 2), [0, 0, 0, 255]);
        // partially off-canvas
        let mut dst = black(4, 4);
        let p = pl(0.0, 0.0, 2.0, 2.0, 0.0);
        draw_layer(&mut dst, &src, p, BlendMode::Normal, 1.0, Scaler::Bilinear, &mut scratch);
        assert_eq!(px(&dst, 0, 0), [0, 255, 0, 255]);
        assert_eq!(px(&dst, 1, 0), [0, 0, 0, 255]);
        // fully off-canvas: no-op, no panic
        let p = pl(100.0, 100.0, 2.0, 2.0, 45.0);
        draw_layer(&mut dst, &src, p, BlendMode::Normal, 1.0, Scaler::Bilinear, &mut scratch);
    }

    #[test]
    fn draw_layer_rotated_90() {
        // 4x2 source, left half red, right half blue; rotated 90 deg cw -> top red, bottom blue.
        let mut dst = black(8, 8);
        let mut src = Frame::new(4, 2);
        for y in 0..2 {
            for x in 0..4 {
                let i = (y * 4 + x) * 4;
                let c = if x < 2 { [255, 0, 0, 255] } else { [0, 0, 255, 255] };
                src.rgba[i..i + 4].copy_from_slice(&c);
            }
        }
        let mut scratch = Frame::default();
        let p = pl(4.0, 4.0, 4.0, 2.0, 90.0);
        draw_layer(&mut dst, &src, p, BlendMode::Normal, 1.0, Scaler::Bilinear, &mut scratch);
        assert_eq!(px(&dst, 3, 2), [255, 0, 0, 255]);
        assert_eq!(px(&dst, 4, 2), [255, 0, 0, 255]);
        assert_eq!(px(&dst, 3, 5), [0, 0, 255, 255]);
        assert_eq!(px(&dst, 4, 5), [0, 0, 255, 255]);
        assert_eq!(px(&dst, 0, 0), [0, 0, 0, 255]);
        assert_eq!(px(&dst, 1, 4), [0, 0, 0, 255]);
    }

    #[test]
    fn scaler_nearest_vs_bilinear_differ() {
        // 2x2 checkerboard upscaled 4x: nearest keeps pure colours, bilinear interpolates
        let mut src = Frame::new(2, 2);
        for (i, c) in [[255u8, 255, 255, 255], [0, 0, 0, 255], [0, 0, 0, 255], [255, 255, 255, 255]].iter().enumerate()
        {
            src.rgba[i * 4..i * 4 + 4].copy_from_slice(c);
        }
        let mut scratch = Frame::default();
        let p = pl(4.0, 4.0, 8.0, 8.0, 0.0);
        let mut near = black(8, 8);
        draw_layer(&mut near, &src, p, BlendMode::Normal, 1.0, Scaler::Nearest, &mut scratch);
        let mut bil = black(8, 8);
        draw_layer(&mut bil, &src, p, BlendMode::Normal, 1.0, Scaler::Bilinear, &mut scratch);
        let mut cub = black(8, 8);
        draw_layer(&mut cub, &src, p, BlendMode::Normal, 1.0, Scaler::Bicubic, &mut scratch);
        assert_ne!(near.rgba, bil.rgba);
        // nearest: every pixel is a pure source colour
        for p in near.rgba.chunks_exact(4) {
            assert!(p[0] == 0 || p[0] == 255, "{p:?}");
        }
        // bilinear: intermediate values appear
        assert!(bil.rgba.chunks_exact(4).any(|p| p[0] > 20 && p[0] < 235));
        // bicubic resolves too and differs from nearest
        assert_ne!(near.rgba, cub.rgba);
        assert_eq!(px(&cub, 1, 1)[0], 255);
    }

    #[test]
    fn perspective_yaw_draws_and_shrinks() {
        let mut src = Frame::new(4, 4);
        src.fill([255, 255, 255, 255]);
        let mut scratch = Frame::default();
        let lit = |f: &Frame| f.rgba.chunks_exact(4).filter(|p| p[0] > 64).count();
        let mut flat = black(16, 16);
        draw_layer(
            &mut flat,
            &src,
            pl(8.0, 8.0, 10.0, 10.0, 0.0),
            BlendMode::Normal,
            1.0,
            Scaler::Bilinear,
            &mut scratch,
        );
        let mut tilted = black(16, 16);
        let p = Placement { cx: 8.0, cy: 8.0, w: 10.0, h: 10.0, rot: 0.0, yaw: 50.0, pitch: 10.0 };
        draw_layer(&mut tilted, &src, p, BlendMode::Normal, 1.0, Scaler::Bilinear, &mut scratch);
        assert!(lit(&tilted) > 0, "tilted layer vanished");
        assert!(lit(&tilted) < lit(&flat), "yaw should foreshorten: {} vs {}", lit(&tilted), lit(&flat));
    }

    struct FakeVideo([u8; 4]);
    impl VideoSource for FakeVideo {
        fn size(&self) -> (u32, u32) {
            (320, 240)
        }
        fn frame_at(&mut self, t: f64, w: u32, h: u32, out: &mut Frame) -> bool {
            if t >= 10.0 {
                return false;
            }
            out.resize(w, h);
            out.fill(self.0);
            out.pts = t;
            true
        }
    }

    fn fake_asset() -> Asset {
        Asset {
            id: 0,
            path: "Z:\\nope\\fake.mp4".into(),
            kind: ClipKind::Video,
            duration: 10.0,
            width: 320,
            height: 240,
            fps: 30.0,
            audio_streams: Vec::new(),
            codec: "h264".into(),
            folder: String::new(),
            tags: Vec::new(),
            label: 0,
            description: String::new(),
        }
    }

    #[test]
    fn compositor_renders_fake_video() {
        let project = Project::from_media(fake_asset());
        let mut pool = DecoderPool::new(Backend::Ffmpeg);
        pool.insert_video(&project.assets[0].path, Box::new(FakeVideo([255, 0, 0, 255])));
        let mut comp = Compositor::new();
        let mut text = TextRasterizer::new();
        let mut out = Frame::default();
        comp.render(&project, 1.0, 64, 48, &mut pool, &mut text, &mut out);
        assert_eq!((out.width, out.height), (64, 48));
        assert_eq!(px(&out, 32, 24), [255, 0, 0, 255]);
        assert_eq!(px(&out, 0, 0), [255, 0, 0, 255]);
        // outside the clip -> black
        comp.render(&project, 20.0, 64, 48, &mut pool, &mut text, &mut out);
        assert_eq!(px(&out, 32, 24), [0, 0, 0, 255]);
        // scale 0.5 + opacity 0.5 -> centred half-size block at half brightness, corners black
        let mut p2 = project.clone();
        let c = &mut p2.tracks[0].clips[0];
        c.scale.value = 0.5;
        c.opacity.value = 0.5;
        comp.render(&p2, 1.0, 64, 48, &mut pool, &mut text, &mut out);
        assert_eq!(px(&out, 32, 24), [128, 0, 0, 255]);
        assert_eq!(px(&out, 0, 0), [0, 0, 0, 255]);
        // muted (disabled) video track -> black
        let mut p3 = project.clone();
        p3.tracks[0].muted = true;
        comp.render(&p3, 1.0, 64, 48, &mut pool, &mut text, &mut out);
        assert_eq!(px(&out, 32, 24), [0, 0, 0, 255]);
        // missing decoder -> skipped, no panic
        let mut empty = DecoderPool::new(Backend::Ffmpeg);
        empty.insert_video("other", Box::new(FakeVideo([0, 0, 0, 255])));
        comp.render(&project, 1.0, 16, 16, &mut empty, &mut text, &mut out);
        assert_eq!(px(&out, 8, 8), [0, 0, 0, 255]);
    }

    /// Records every (w, h) it is asked for.
    struct SizeSpy(std::sync::Arc<std::sync::Mutex<Vec<(u32, u32)>>>);
    impl VideoSource for SizeSpy {
        fn size(&self) -> (u32, u32) {
            (320, 240)
        }
        fn frame_at(&mut self, _t: f64, w: u32, h: u32, out: &mut Frame) -> bool {
            self.0.lock().unwrap().push((w, h));
            out.resize(w, h);
            out.fill([0, 0, 255, 255]);
            true
        }
    }

    #[test]
    fn keyframed_image_scale_decodes_one_size() {
        let mut project = Project::new();
        (project.width, project.height, project.fps) = (320, 240, 30.0);
        let mut a = fake_asset();
        a.kind = ClipKind::Image;
        a.path = "Z:\\nope\\pic.png".into();
        let id = project.add_asset(a);
        project.insert_asset_clips(id, 0.0, None);
        let clip = &mut project.tracks[0].clips[0];
        assert_eq!(clip.kind, ClipKind::Image);
        clip.scale.keys =
            vec![Keyframe { t: 0.0, v: 0.25, ease: Ease::Linear }, Keyframe { t: 4.0, v: 0.5, ease: Ease::Linear }];
        let sizes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut pool = DecoderPool::new(Backend::Ffmpeg);
        pool.insert_video(&project.assets[0].path, Box::new(SizeSpy(sizes.clone())));
        let mut comp = Compositor::new();
        let mut text = TextRasterizer::new();
        let mut out = Frame::default();
        for i in 0..5 {
            comp.render(&project, i as f64, 320, 240, &mut pool, &mut text, &mut out);
        }
        // one decode size for the whole clip (the largest key), yet drawn at the animated size
        let s = sizes.lock().unwrap().clone();
        assert_eq!(s.len(), 5);
        assert!(s.iter().all(|&x| x == (160, 120)), "{s:?}");
        comp.render(&project, 0.0, 320, 240, &mut pool, &mut text, &mut out);
        assert_eq!(px(&out, 160, 120), [0, 0, 255, 255]);
        assert_eq!(px(&out, 100, 120), [0, 0, 0, 255]); // scale 0.25 -> 80 px wide, centred
    }

    /// Two abutting full-frame clips (red then blue) with a transition on the cut at t=2.
    fn transition_project(kind: TransitionKind, duration: f64) -> (Project, DecoderPool) {
        let mut project = Project::new();
        (project.width, project.height) = (320, 240);
        let a1 = fake_asset();
        let mut a2 = fake_asset();
        a2.path = "Z:\\nope\\fake2.mp4".into();
        let id1 = project.add_asset(a1);
        let id2 = project.add_asset(a2);
        let c1 = {
            let mut c = crate::model::Clip::new(project.new_id(), ClipKind::Video, "a", 0.0, 2.0);
            c.asset = id1;
            c
        };
        let mut c2 = crate::model::Clip::new(project.new_id(), ClipKind::Video, "b", 2.0, 2.0);
        c2.asset = id2;
        let right = c2.id;
        project.tracks[0].clips.push(c1);
        project.tracks[0].clips.push(c2);
        let tid = project.new_id();
        project.tracks[0].transitions.push(crate::model::Transition {
            id: tid,
            right,
            kind,
            duration,
            color: [255, 255, 255, 255],
            direction: 0,
            ease: Ease::Linear,
        });
        let mut pool = DecoderPool::new(Backend::Ffmpeg);
        pool.insert_video("Z:\\nope\\fake.mp4", Box::new(FakeVideo([255, 0, 0, 255])));
        pool.insert_video("Z:\\nope\\fake2.mp4", Box::new(FakeVideo([0, 0, 255, 255])));
        (project, pool)
    }

    #[test]
    fn transition_crossfade_midpoint_mixes() {
        let (project, mut pool) = transition_project(TransitionKind::CrossFade, 1.0);
        let mut comp = Compositor::new();
        let mut text = TextRasterizer::new();
        let mut out = Frame::default();
        // t = cut -> progress 0.5 -> 50/50 red/blue
        comp.render(&project, 2.0, 64, 48, &mut pool, &mut text, &mut out);
        let c = px(&out, 32, 24);
        assert!((120..=135).contains(&c[0]) && (120..=135).contains(&c[2]), "{c:?}");
        // before the window -> pure red; after -> pure blue
        comp.render(&project, 1.0, 64, 48, &mut pool, &mut text, &mut out);
        assert_eq!(px(&out, 32, 24), [255, 0, 0, 255]);
        comp.render(&project, 3.0, 64, 48, &mut pool, &mut text, &mut out);
        assert_eq!(px(&out, 32, 24), [0, 0, 255, 255]);
    }

    #[test]
    fn transition_push_moves_content() {
        let (project, mut pool) = transition_project(TransitionKind::Push, 2.0);
        let mut comp = Compositor::new();
        let mut text = TextRasterizer::new();
        let mut out = Frame::default();
        // t = cut -> progress 0.5: A pushed half off to the right, B occupies the left half
        comp.render(&project, 2.0, 64, 48, &mut pool, &mut text, &mut out);
        assert_eq!(px(&out, 8, 24), [0, 0, 255, 255], "left half is B");
        assert_eq!(px(&out, 56, 24), [255, 0, 0, 255], "right half is A");
    }

    #[test]
    fn transition_wipe_reveals_region() {
        let (project, mut pool) = transition_project(TransitionKind::Wipe, 2.0);
        let mut comp = Compositor::new();
        let mut text = TextRasterizer::new();
        let mut out = Frame::default();
        // progress 0.5, direction "from the left": left half B, right half A (both full-frame)
        comp.render(&project, 2.0, 64, 48, &mut pool, &mut text, &mut out);
        assert_eq!(px(&out, 8, 24), [0, 0, 255, 255]);
        assert_eq!(px(&out, 56, 24), [255, 0, 0, 255]);
    }

    #[test]
    fn transition_fade_to_color() {
        let (project, mut pool) = transition_project(TransitionKind::FadeToColor, 2.0);
        let mut comp = Compositor::new();
        let mut text = TextRasterizer::new();
        let mut out = Frame::default();
        // t = cut -> progress 0.5 -> fully the transition colour (white)
        comp.render(&project, 2.0, 64, 48, &mut pool, &mut text, &mut out);
        assert_eq!(px(&out, 32, 24), [255, 255, 255, 255]);
        // early in the window: red fading towards white
        comp.render(&project, 1.5, 64, 48, &mut pool, &mut text, &mut out);
        let c = px(&out, 32, 24);
        assert_eq!(c[0], 255);
        assert!(c[1] > 100 && c[1] < 155, "{c:?}");
    }

    #[test]
    fn transition_window_clamped_to_clips() {
        // A [0,2) red, B [2,3) green, C [3,5) blue with a 4 s transition on the A|B cut: the window is
        // clamped to [1,3) by the 1 s clip B, so C renders normally at t=3.5.
        let (mut project, mut pool) = transition_project(TransitionKind::CrossFade, 4.0);
        project.tracks[0].clips[1].duration = 1.0;
        let mut c3 = crate::model::Clip::new(project.new_id(), ClipKind::Video, "c", 3.0, 2.0);
        let mut a3 = fake_asset();
        a3.path = "Z:\\nope\\fake3.mp4".into();
        c3.asset = project.add_asset(a3);
        project.tracks[0].clips.push(c3);
        pool.insert_video("Z:\\nope\\fake3.mp4", Box::new(FakeVideo([0, 255, 255, 255])));
        let mut comp = Compositor::new();
        let mut text = TextRasterizer::new();
        let mut out = Frame::default();
        comp.render(&project, 3.5, 64, 48, &mut pool, &mut text, &mut out);
        assert_eq!(px(&out, 32, 24), [0, 255, 255, 255], "clip C hidden by an over-long transition");
        // inside the clamped window the cut still dissolves (t = 2 is the cut → 50/50)
        comp.render(&project, 2.0, 64, 48, &mut pool, &mut text, &mut out);
        let c = px(&out, 32, 24);
        assert!((120..=135).contains(&c[0]) && (120..=135).contains(&c[2]), "{c:?}");
    }

    #[test]
    fn text_clip_fades_to_colour() {
        // Two abutting text clips with an opaque box, FadeToColor to white: the cut is fully white even
        // though neither clip has an effect.
        let mut project = Project::new();
        (project.width, project.height) = (320, 240);
        let a = project.add_text_clip(0.0, 2.0);
        let b = project.add_text_clip(2.0, 2.0);
        for id in [a, b] {
            let st = project.clip_mut(id).unwrap().text.as_mut().expect("text style");
            st.box_color = [255, 0, 0, 255];
            st.color = [0, 255, 0, 255];
        }
        let tid = project.new_id();
        let vt = project.video_tracks()[0];
        project.tracks[vt].transitions.push(crate::model::Transition {
            id: tid,
            right: b,
            kind: TransitionKind::FadeToColor,
            duration: 2.0,
            color: [255, 255, 255, 255],
            direction: 0,
            ease: Ease::Linear,
        });
        let mut pool = DecoderPool::new(Backend::Ffmpeg);
        let mut comp = Compositor::new();
        let mut text = TextRasterizer::new();
        let mut out = Frame::default();
        let count = |f: &Frame, c: [u8; 4]| f.rgba.chunks_exact(4).filter(|p| *p == c).count();
        comp.render(&project, 1.0, 64, 48, &mut pool, &mut text, &mut out);
        if text.families().is_empty() {
            eprintln!("no system fonts - skipping");
            return;
        }
        assert!(count(&out, [255, 0, 0, 255]) > 0, "no text box before the window");
        // at the cut the fade is full: the layer is the transition colour, no source colours left
        comp.render(&project, 2.0, 64, 48, &mut pool, &mut text, &mut out);
        assert_eq!(count(&out, [255, 0, 0, 255]), 0, "text clip ignored the fade");
        assert_eq!(count(&out, [0, 255, 0, 255]), 0);
        assert!(count(&out, [255, 255, 255, 255]) > 0, "no fade colour drawn");
    }

    #[test]
    fn sequence_clip_renders_nested() {
        let mut project = Project::new();
        (project.width, project.height) = (320, 240);
        let id = project.add_asset(fake_asset());
        let sid = project.new_sequence("seq", 320, 240, 30.0);
        let cid = project.new_id();
        let mut c = crate::model::Clip::new(cid, ClipKind::Video, "v", 0.0, 5.0);
        c.asset = id;
        project.sequence_mut(sid).unwrap().tracks[0].clips.push(c);
        project.insert_sequence_clip(sid, 0.0, None).expect("sequence clip");
        let mut pool = DecoderPool::new(Backend::Ffmpeg);
        pool.insert_video("Z:\\nope\\fake.mp4", Box::new(FakeVideo([0, 255, 0, 255])));
        let mut comp = Compositor::new();
        let mut text = TextRasterizer::new();
        let mut out = Frame::default();
        comp.render(&project, 1.0, 64, 48, &mut pool, &mut text, &mut out);
        assert_eq!(px(&out, 32, 24), [0, 255, 0, 255]);
        // past the sequence clip -> black
        comp.render(&project, 9.0, 64, 48, &mut pool, &mut text, &mut out);
        assert_eq!(px(&out, 32, 24), [0, 0, 0, 255]);
    }

    #[test]
    fn subtitle_renders_near_bottom() {
        let mut project = Project::from_media(fake_asset());
        project.add_cue(0.0, 2.0, "HELLO");
        assert!(project.show_subtitles);
        let mut pool = DecoderPool::new(Backend::Ffmpeg);
        pool.insert_video(&project.assets[0].path, Box::new(FakeVideo([255, 0, 0, 255])));
        let mut comp = Compositor::new();
        let mut text = TextRasterizer::new();
        let mut out = Frame::default();
        comp.render(&project, 1.0, 128, 96, &mut pool, &mut text, &mut out);
        if text.families().is_empty() {
            eprintln!("no system fonts - skipping");
            return;
        }
        // white subtitle pixels in the bottom half, none in the top half (background is red)
        let white = |out: &Frame, y0: u32, y1: u32| {
            (y0..y1)
                .flat_map(|y| (0..128).map(move |x| (x, y)))
                .filter(|&(x, y)| px(out, x, y) == [255, 255, 255, 255])
                .count()
        };
        assert!(white(&out, 48, 96) > 0, "no subtitle pixels");
        assert_eq!(white(&out, 0, 48), 0);
        // hidden when show_subtitles is off
        project.show_subtitles = false;
        comp.render(&project, 1.0, 128, 96, &mut pool, &mut text, &mut out);
        assert_eq!(white(&out, 48, 96), 0);
    }

    #[test]
    fn wobble_effect_moves_placement() {
        let mut project = Project::from_media(fake_asset());
        let clip = &mut project.tracks[0].clips[0];
        let mut e = crate::model::Effect::new(crate::model::EffectKind::Wobble);
        e.params[0].value = 100.0; // Amplitude X
        e.params[1].value = 100.0;
        clip.effects.push(e);
        let mut pool = DecoderPool::new(Backend::Ffmpeg);
        pool.insert_video(&project.assets[0].path, Box::new(FakeVideo([255, 0, 0, 255])));
        let mut comp = Compositor::new();
        let mut text = TextRasterizer::new();
        let mut out = Frame::default();
        // full-frame red shifted by wobble -> some border pixels are black
        comp.render(&project, 0.37, 64, 48, &mut pool, &mut text, &mut out);
        let corners = [px(&out, 0, 0), px(&out, 63, 0), px(&out, 0, 47), px(&out, 63, 47)];
        assert!(corners.iter().any(|c| *c == [0, 0, 0, 255]), "{corners:?}");
    }
}
