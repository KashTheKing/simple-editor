//! ---- ws:browser-layer ----
//! Drawing a `ClipKind::Browser` clip. Both compositors do the same five steps with what they
//! already have (a stretched bitmap, a placed bitmap, a masked placed bitmap):
//!   1. take what is on the canvas so far as "the page", clear the canvas to the background;
//!   2. the background bitmap (a gradient or a picture), stretched;
//!   3. the window bitmap (shadow, body, tabs, address bar) at the clip's placement;
//!   4. the page, placed in the window's page area and masked to it;
//!   5. the cursor sprite and the keystroke overlay.
//! This file owns the geometry (`plan`) and the bitmaps (`BrowserRasterizer`); nothing here is
//! per-pixel work per frame: the window art is cached and only its address text / load bar are
//! redrawn while they animate, the cursor sprite is small. No new dependency, no web engine.
//!
//! Scroll and zoom are not done to the page picture: while a browser clip is on screen the
//! compositors move and enlarge every layer on the tracks below it (`LayerView`), so a page taller or
//! denser than the frame scrolls and zooms at its own resolution instead of as a picture of the frame.

use crate::engine::compose::Placement;
use crate::engine::shapes::ShapeRasterizer;
use crate::engine::text::TextRasterizer;
use crate::media::Frame;
use crate::model::{
    BrowserDisplay, BrowserStyle, BrowserTheme, Clip, ClipKind, CursorEventKind, CursorShape, Mask, MaskShape, ShapeKind, ShapeStyle,
    Stroke, TextStyle, Track, TrackKind,
};
use crate::model::Animated;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// `LayerSet` slots of one browser clip (`key`).
pub const BG: u64 = 1;
pub const WINDOW: u64 = 2;
pub const CURSOR: u64 = 3;
pub const KEYS: u64 = 4;
/// A 1x1 frame whose alpha is the keystroke overlay's opacity.
pub const KEYS_ALPHA: u64 = 5;

/// The `LayerSet` / texture key of a browser clip's bitmap. Clip ids count up from 1, so the top
/// byte is free (bit 62 is the Source monitor's tag).
pub fn key(clip: crate::model::Id, slot: u64) -> u64 {
    clip | slot << 56
}

/// The centre a `Plan::mask` is rasterised around (see there).
pub const MASK_ORIGIN: (f32, f32) = (0.0, 0.0);

/// Half-size of the cursor sprite in project px at cursor size 1 (room for the widest ripple).
const SPRITE: f32 = 44.0;

/// Where everything of one browser clip lands on a (cw, ch) canvas.
#[derive(Clone, Debug)]
pub struct Plan {
    pub window: Placement,
    /// The page is the canvas itself (see `LayerView`): this is always the whole canvas.
    pub page: Placement,
    /// The page area as a polygon in absolute canvas px (rounded where the window is): rasterise it
    /// around `MASK_ORIGIN`.
    pub mask: Mask,
    // window-local (project px from the window centre) -> canvas: c + R * (l * u)
    c: (f32, f32),
    u: (f32, f32),
    rot: f32,
    /// Page centre in window-local px and window px per page px, at zoom 1.
    page0: (f32, f32),
    f0: f32,
    /// Size of the window bitmap on the canvas before the clip's Scale and the Window zoom.
    unscaled: (f32, f32),
    /// Half the window's height, project px.
    half_h: f32,
    /// Page only: the page is the frame; only the cursor and overlays are drawn.
    pub page_only: bool,
}

impl Plan {
    fn to_canvas(&self, l: (f32, f32)) -> (f32, f32) {
        let (s, c) = self.rot.to_radians().sin_cos();
        let (x, y) = (l.0 * self.u.0, l.1 * self.u.1);
        (self.c.0 + x * c - y * s, self.c.1 + x * s + y * c)
    }
    /// Canvas px of a cursor position (page px from the page centre).
    pub fn page_to_canvas(&self, p: (f64, f64)) -> (f32, f32) {
        self.to_canvas((self.page0.0 + p.0 as f32 * self.f0, self.page0.1 + p.1 as f32 * self.f0))
    }
    /// The cursor position under a canvas pixel (what a recording stores).
    pub fn canvas_to_page(&self, p: (f32, f32)) -> (f64, f64) {
        let (s, c) = self.rot.to_radians().sin_cos();
        let (dx, dy) = (p.0 - self.c.0, p.1 - self.c.1);
        let (x, y) = ((dx * c + dy * s) / self.u.0, (-dx * s + dy * c) / self.u.1);
        (((x - self.page0.0) / self.f0) as f64, ((y - self.page0.1) / self.f0) as f64)
    }
    /// A bitmap of `size` canvas px (at window scale 1) centred on a window-local point.
    fn sprite(&self, l: (f32, f32), size: (f32, f32)) -> Placement {
        let (cx, cy) = self.to_canvas(l);
        let s = (self.window.w / self.unscaled.0, self.window.h / self.unscaled.1);
        Placement { cx, cy, w: size.0 * s.0, h: size.1 * s.1, rot: self.rot, yaw: 0.0, pitch: 0.0 }
    }
    pub fn shift(&mut self, dx: f32, dy: f32) {
        (self.window.cx, self.window.cy) = (self.window.cx + dx, self.window.cy + dy);
        for p in &mut self.mask.points {
            *p = (p.0 + dx, p.1 + dy);
        }
        self.c = (self.c.0 + dx, self.c.1 + dy);
    }
    /// The Window zoom: everything of the browser `z` times bigger about the canvas point `f`.
    fn zoom_about(&mut self, f: (f32, f32), z: f32) {
        let about = |p: (f32, f32)| (f.0 + (p.0 - f.0) * z, f.1 + (p.1 - f.1) * z);
        (self.window.cx, self.window.cy) = about((self.window.cx, self.window.cy));
        (self.window.w, self.window.h) = (self.window.w * z, self.window.h * z);
        for p in &mut self.mask.points {
            *p = about(*p);
        }
        self.c = about(self.c);
        self.u = (self.u.0 * z, self.u.1 * z);
    }
    /// Placement of the cursor sprite (`None` when there is no visible cursor).
    pub fn cursor(&self, b: &BrowserStyle, lt: f64, s: f32) -> Option<Placement> {
        let c = b.cursor.as_ref().filter(|c| c.visible)?;
        let (x, y) = c.pos(lt);
        let l = (self.page0.0 + x as f32 * self.f0, self.page0.1 + y as f32 * self.f0);
        let d = SPRITE * 2.0 * c.size.clamp(0.2, 8.0) * s;
        Some(self.sprite(l, (d, d)))
    }
    /// Placement of the keystroke overlay bitmap: bottom centre of the page area.
    pub fn keys(&self, b: &BrowserStyle, size: (u32, u32)) -> Placement {
        // (window-local: half the window's height below its middle, less the foot bar and a margin)
        let y = self.half_h - b.foot_h() - 56.0;
        self.sprite((0.0, y), (size.0 as f32, size.1 as f32))
    }
    /// Page only: where the thin load bar goes (the top edge, as wide as the load is far), if it runs.
    pub fn load_bar(&self, b: &BrowserStyle, lt: f64, s: f32) -> Option<Placement> {
        let v = b.load.at(lt).clamp(0.0, 1.0) as f32;
        if !self.page_only || v <= 0.0 || v >= 1.0 {
            return None;
        }
        let (w, h) = (self.page.w * v, (4.0 * s).max(2.0));
        Some(Placement { cx: w / 2.0, cy: h / 2.0, w, h, rot: 0.0, yaw: 0.0, pitch: 0.0 })
    }
}

/// Shadow margin around the window inside its bitmap, project px.
fn margin(b: &BrowserStyle) -> f32 {
    if b.shadow > 0.0 {
        (b.shadow.min(400.0) * 1.3).ceil() + 2.0
    } else {
        0.0
    }
}

/// A zoom rounded up to a power of √2: bitmaps are drawn at a few fixed scales, each at least as big as
/// it is shown (never a smaller one stretched), instead of at a new size on every frame of a move.
fn level(z: f32) -> f32 {
    2f32.powf(((z.max(1.0).log2() * 2.0) - 1e-3).ceil() / 2.0)
}

/// The window bitmap (and the cursor sprite) are rasterised at this many px per project px at
/// clip-local `lt`: the canvas scale, times the clip's Scale and the Window zoom, so an enlarged
/// window is drawn at its size - up to a bitmap about 4096 px wide.
pub fn raster_scale(clip: &Clip, s: f32, lt: f64) -> f32 {
    let Some(b) = &clip.browser else { return s };
    let k = if b.display == BrowserDisplay::Page { 1.0 } else { clip.scale.at(lt) as f32 };
    let cap = (4096.0 / ((b.w + 2.0 * margin(b)) * s).max(1.0)).max(1.0);
    s * level(k.max(0.05) * b.wzoom.at(lt) as f32).min(cap)
}

/// Geometry of `clip` (a Browser clip) at `t` on a (cw, ch) canvas; `pw` = the project (or nested
/// sequence) width its numbers are in.
pub fn plan(pw: u32, clip: &Clip, t: f64, cw: u32, ch: u32) -> Option<Plan> {
    let b = clip.browser.as_ref()?;
    let lt = clip.local(t);
    let s = cw as f32 / pw.max(1) as f32;
    let (pwf, phf) = (pw.max(1) as f32, ch as f32 / s);
    // Page only: the "window" is the frame itself - no margin, no bars, not moved by the clip
    let page_only = b.display == BrowserDisplay::Page;
    let (bw, bh) = if page_only { (pwf, phf) } else { (b.w, b.h) };
    let m = if page_only { 0.0 } else { margin(b) };
    let native = ((bw + 2.0 * m) * s, (bh + 2.0 * m) * s);
    // the clip's own transform, exactly as a shape clip of the window's size would be placed
    let k = clip.scale.at(lt) as f32;
    let (u, c, rot) = if page_only {
        ((s, s), (cw as f32 / 2.0, ch as f32 / 2.0), 0.0)
    } else {
        (
            (s * k * clip.scale_x.at(lt) as f32, s * k * clip.scale_y.at(lt) as f32),
            (cw as f32 / 2.0 + clip.x.at(lt) as f32 * s, ch as f32 / 2.0 + clip.y.at(lt) as f32 * s),
            clip.rotation.at(lt) as f32,
        )
    };
    let window = Placement { cx: c.0, cy: c.1, w: native.0 / s * u.0, h: native.1 / s * u.1, rot, yaw: 0.0, pitch: 0.0 };
    if !(window.w.is_finite() && window.h.is_finite() && window.w > 0.0 && window.h > 0.0) {
        return None;
    }
    let (vw, vh) = if page_only { (pwf, phf) } else { b.viewport() };
    let top = -bh / 2.0 + b.chrome_h();
    let vc = (0.0, top + vh / 2.0);
    // the page: fitted to the page area's width, its top at the area's top
    let f0 = vw / pwf;
    let page0 = (0.0, top + phf * f0 / 2.0);
    let (wf, hf) = (cw as f32, ch as f32);
    let mut plan = Plan {
        window,
        page: Placement { cx: wf / 2.0, cy: hf / 2.0, w: wf, h: hf, rot: 0.0, yaw: 0.0, pitch: 0.0 },
        mask: Mask::new(MaskShape::Polygon),
        c,
        u,
        rot,
        page0,
        f0,
        unscaled: native,
        half_h: bh / 2.0,
        page_only,
    };
    // rounded where the page area meets the window's own corners
    let r = if page_only { 0.0 } else { b.corner.clamp(0.0, vw.min(vh) / 2.0) };
    let (rt, rb) = (if b.chrome_h() > 0.0 { 0.0 } else { r }, if b.foot_h() > 0.0 { 0.0 } else { r });
    let (hw, hh) = (vw / 2.0, vh / 2.0);
    let mut pts = Vec::with_capacity(28);
    for (i, (x, y, r)) in [(-hw, -hh, rt), (hw, -hh, rt), (hw, hh, rb), (-hw, hh, rb)].into_iter().enumerate() {
        if r <= 0.01 {
            pts.push((x, y));
            continue;
        }
        let (ax, ay) = (x - x.signum() * r, y - y.signum() * r);
        let a0 = std::f32::consts::PI * (1.0 + 0.5 * i as f32);
        for j in 0..=6 {
            let a = a0 + std::f32::consts::FRAC_PI_2 * (j as f32 / 6.0);
            pts.push((ax + r * a.cos(), ay + r * a.sin()));
        }
    }
    plan.mask.points = pts.into_iter().map(|(x, y)| plan.to_canvas((vc.0 + x, vc.1 + y))).collect();
    plan.mask.feather = Animated::new(1.0);
    // the Window zoom: the whole browser - frame, page area, mouse - about its point
    let wz = (b.wzoom.at(lt) as f32).clamp(0.05, 64.0);
    if wz != 1.0 {
        plan.zoom_about((wf / 2.0 + b.wzoom_x.at(lt) as f32 * s, hf / 2.0 + b.wzoom_y.at(lt) as f32 * s), wz);
    }
    Some(plan)
}

/// What a browser clip above a track does to its layers: each is placed straight where it shows on
/// the screen - scrolled, zoomed (the page Zoom and the Window zoom), fitted into the window's page
/// area and turned with the window - instead of into a picture of the frame that is then scaled.
/// So a layer is decoded / drawn at the size it is seen at, and nothing is ever stretched.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayerView {
    /// Page canvas px (x, y) -> screen px: (a x + c y + tx, b x + d y + ty).
    m: [f32; 6],
    /// Screen px per page px, and the window's rotation in degrees.
    pub scale: f32,
    rot: f32,
}

impl LayerView {
    pub const NONE: LayerView = LayerView { m: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], scale: 1.0, rot: 0.0 };
    pub fn is_none(&self) -> bool {
        *self == Self::NONE
    }
    /// A canvas point of the unscrolled, unzoomed page -> where it is shown.
    pub fn point(&self, p: (f32, f32)) -> (f32, f32) {
        let m = &self.m;
        (m[0] * p.0 + m[2] * p.1 + m[4], m[1] * p.0 + m[3] * p.1 + m[5])
    }
    /// The page point shown at a screen point.
    pub fn page_point(&self, p: (f32, f32)) -> (f32, f32) {
        let m = &self.m;
        let det = m[0] * m[3] - m[1] * m[2];
        let (x, y) = (p.0 - m[4], p.1 - m[5]);
        ((m[3] * x - m[2] * y) / det, (m[0] * y - m[1] * x) / det)
    }
    pub fn apply(&self, p: &mut Placement) {
        if self.is_none() {
            return;
        }
        (p.cx, p.cy) = self.point((p.cx, p.cy));
        p.w *= self.scale;
        p.h *= self.scale;
        p.rot += self.rot;
    }
}

/// The view of one browser clip at timeline `t` on a (cw, ch) canvas showing `pw` project px.
pub fn view_of(clip: &Clip, t: f64, pw: u32, cw: u32, ch: u32) -> LayerView {
    let (Some(b), Some(plan)) = (&clip.browser, plan(pw, clip, t, cw, ch)) else { return LayerView::NONE };
    let s = cw as f32 / pw.max(1) as f32;
    let lt = clip.local(t);
    let (z, ax, ay) = b.zoom_at(lt);
    let (z, dy) = (z as f32, -(b.scroll.at(lt) as f32) * s);
    let at = (cw as f32 / 2.0 + ax as f32 * s, ch as f32 / 2.0 + ay as f32 * s);
    let mid = (cw as f32 / 2.0, ch as f32 / 2.0);
    // scroll and page Zoom (in the page's own canvas), then into the window's page area
    let map = |p: (f32, f32)| {
        let q = (at.0 + (p.0 - at.0) * z, at.1 + (p.1 + dy - at.1) * z);
        plan.page_to_canvas((((q.0 - mid.0) / s) as f64, ((q.1 - mid.1) / s) as f64))
    };
    let (o, x, y) = (map((0.0, 0.0)), map((1.0, 0.0)), map((0.0, 1.0)));
    let m = [x.0 - o.0, x.1 - o.1, y.0 - o.0, y.1 - o.1, o.0, o.1];
    let v = LayerView { m, scale: (m[0] * m[0] + m[1] * m[1]).sqrt(), rot: plan.rot };
    // (an untouched Page-only browser changes nothing: its layers take the ordinary, exact paths)
    let same = v.rot == 0.0 && m.iter().zip(LayerView::NONE.m).all(|(a, b)| (a - b).abs() < 1e-5);
    if same { LayerView::NONE } else { v }
}

/// The view of the layers of track `ti`: that of the browser clip on screen above it (the nearest
/// one when browsers are stacked). `NONE` for nearly every track of nearly every project - one pass
/// over the clips on screen on the tracks above.
pub fn layer_view(tracks: &[Track], ti: usize, t: f64, pw: u32, cw: u32, ch: u32) -> LayerView {
    tracks
        .iter()
        .skip(ti + 1)
        .filter(|tr| tr.kind == TrackKind::Video)
        .flat_map(|tr| tr.clips.iter())
        .find(|c| c.kind == ClipKind::Browser && c.enabled && c.contains(t))
        .map_or(LayerView::NONE, |c| view_of(c, t, pw, cw, ch))
}

/// How much bigger than its own placement a layer under a browser is shown at `t` (1 without one, and
/// never less): the layers there are decoded, and text and shapes drawn, that much bigger.
pub fn zoom_on_screen(tracks: &[Track], t: f64, pw: u32, ph: u32) -> f32 {
    let on = tracks.iter().flat_map(|tr| tr.clips.iter()).filter(|c| c.kind == ClipKind::Browser && c.contains(t));
    on.map(|c| view_of(c, t, pw, pw, ph).scale).fold(1.0, f32::max)
}

/// How much bigger text and shape layers are drawn at `t` (`zoom_on_screen`, in steps, at most 8).
pub fn draw_boost(tracks: &[Track], t: f64, pw: u32, ph: u32) -> f32 {
    level(zoom_on_screen(tracks, t, pw, ph)).min(8.0)
}

/// The largest zoom any browser clip reaches while it overlaps `start..end` (1 without one): how much
/// bigger than its own placement a still there is ever shown, so that much of it is kept decoded.
pub fn max_zoom_over(tracks: &[Track], start: f64, end: f64) -> f32 {
    let over = tracks.iter().flat_map(|tr| tr.clips.iter()).filter(|c| c.kind == ClipKind::Browser && c.start < end && start < c.end());
    let top = |a: &Animated| a.keys.iter().map(|k| k.v).fold(if a.is_animated() { 1.0 } else { a.value }, f64::max).max(1.0) as f32;
    over.filter_map(|c| c.browser.as_ref().map(|b| top(&b.zoom) * top(&b.wzoom) * top(&c.scale))).fold(1.0, f32::max)
}

/// A stretch of a browser clip where the page is shown bigger than `2^(n-1)` and at most `2^n` times
/// its own size: clip-local span, that `2^n`, and the part of the page seen meanwhile (project px
/// from the frame's middle, before scroll and zoom).
#[derive(Clone, Debug, PartialEq)]
pub struct ZoomLevel {
    pub from: f64,
    pub to: f64,
    pub scale: f64,
    pub rect: (f64, f64, f64, f64),
}

/// The zoom levels of browser `clip` over its whole length, moves included: a picture of each part
/// at its scale is at least as sharp as the screen at every moment, so nothing is ever enlarged -
/// while a zoom moves, the next level up is shown smaller. Spans do not overlap.
pub fn zoom_levels(clip: &Clip, pw: u32, ph: u32) -> Vec<ZoomLevel> {
    const STEP: f64 = 1.0 / 15.0;
    let mut out: Vec<ZoomLevel> = Vec::new();
    let n = (clip.duration / STEP).ceil() as usize;
    for i in 0..=n {
        let lt = (i as f64 * STEP).min(clip.duration);
        let t = clip.start + lt;
        let (v, Some(plan)) = (view_of(clip, t, pw, pw, ph), plan(pw, clip, t, pw, ph)) else { continue };
        let l = (v.scale.log2() - 0.03).ceil().max(0.0) as i32;
        // what is seen: the page area on the screen, back through the view
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for p in &plan.mask.points {
            (x0, y0, x1, y1) = (x0.min(p.0), y0.min(p.1), x1.max(p.0), y1.max(p.1));
        }
        let (x0, y0, x1, y1) = (x0.max(0.0), y0.max(0.0), x1.min(pw as f32), y1.min(ph as f32));
        let mut r = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for c in [(x0, y0), (x1, y0), (x1, y1), (x0, y1)] {
            let p = v.page_point(c);
            let (x, y) = (p.0 as f64 - pw as f64 / 2.0, p.1 as f64 - ph as f64 / 2.0);
            r = (r.0.min(x), r.1.min(y), r.2.max(x), r.3.max(y));
        }
        let scale = 2f64.powi(l);
        match out.last_mut().filter(|o| o.scale == scale && o.to >= lt - STEP * 1.5) {
            Some(o) => {
                o.to = lt + STEP;
                o.rect = (o.rect.0.min(r.0), o.rect.1.min(r.1), o.rect.2.max(r.2), o.rect.3.max(r.3));
            }
            None if l > 0 => {
                // a level begins a step early and the one before ends there: no frame falls between
                let from = (lt - STEP).max(0.0);
                if let Some(o) = out.last_mut() {
                    o.to = o.to.min(from);
                }
                out.push(ZoomLevel { from, to: lt + STEP, scale, rect: r });
            }
            None => {}
        }
    }
    for o in &mut out {
        o.to = o.to.min(clip.duration);
    }
    out.retain(|o| o.to > o.from + 1e-3);
    out
}

/// The bitmaps of one browser clip at one instant.
#[derive(Default, Clone)]
pub struct Layers {
    pub bg: Option<Arc<Frame>>,
    pub window: Option<Arc<Frame>>,
    pub cursor: Option<Arc<Frame>>,
    /// Keystroke overlay and its opacity.
    pub keys: Option<(Arc<Frame>, f32)>,
}

struct Pal {
    frame: [u8; 4],
    bar: [u8; 4],
    text: [u8; 4],
    dim: [u8; 4],
    pill: [u8; 4],
    accent: [u8; 4],
    page: [u8; 4],
}

fn pal(theme: BrowserTheme) -> Pal {
    match theme {
        BrowserTheme::Dark => Pal {
            frame: [30, 31, 34, 255],
            bar: [50, 52, 56, 255],
            text: [232, 234, 237, 255],
            dim: [150, 156, 164, 255],
            pill: [30, 31, 34, 255],
            accent: [120, 170, 250, 255],
            page: [22, 22, 25, 255],
        },
        BrowserTheme::Minimal | BrowserTheme::Mobile => Pal {
            frame: [244, 245, 247, 255],
            bar: [244, 245, 247, 255],
            text: [34, 36, 40, 255],
            dim: [120, 124, 132, 255],
            pill: [226, 228, 233, 255],
            accent: [40, 120, 230, 255],
            page: [255, 255, 255, 255],
        },
        BrowserTheme::Light => Pal {
            frame: [218, 222, 228, 255],
            bar: [255, 255, 255, 255],
            text: [34, 36, 40, 255],
            dim: [100, 104, 112, 255],
            pill: [238, 240, 243, 255],
            accent: [40, 120, 230, 255],
            page: [255, 255, 255, 255],
        },
    }
}

/// Straight-alpha source-over of `src` onto `dst` at (x, y), clipped to `dst` and to `max_w` columns.
fn blit(dst: &mut Frame, src: &Frame, x: i32, y: i32, max_w: u32) {
    let (dw, dh) = (dst.width as i32, dst.height as i32);
    let sw = (src.width.min(max_w)) as i32;
    for sy in 0..src.height as i32 {
        let ty = y + sy;
        if ty < 0 || ty >= dh {
            continue;
        }
        for sx in 0..sw {
            let tx = x + sx;
            if tx < 0 || tx >= dw {
                continue;
            }
            let si = (sy as usize * src.width as usize + sx as usize) * 4;
            let sa = src.rgba[si + 3] as u32;
            if sa == 0 {
                continue;
            }
            let d = &mut dst.rgba[(ty as usize * dw as usize + tx as usize) * 4..][..4];
            if sa == 255 || d[3] == 0 {
                d.copy_from_slice(&src.rgba[si..si + 4]);
                continue;
            }
            let da = d[3] as u32 * (255 - sa) / 255;
            let oa = sa + da;
            for i in 0..3 {
                d[i] = ((src.rgba[si + i] as u32 * sa + d[i] as u32 * da) / oa) as u8;
            }
            d[3] = oa as u8;
        }
    }
}

/// A canvas in window px (origin = the window's top-left, `m` px inside the bitmap) drawn with the
/// app's own shape and text rasterisers.
struct Art<'a> {
    out: Frame,
    rs: f32,
    m: f32,
    shapes: &'a mut ShapeRasterizer,
    text: &'a mut TextRasterizer,
    font: &'a str,
    strokes: Vec<Stroke>,
    /// Window centre in window px: strokes are collected relative to it and drawn in one go.
    centre: (f32, f32),
}

impl Art<'_> {
    fn px(&self, v: f32) -> i32 {
        ((v + self.m) * self.rs).round() as i32
    }
    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, color: [u8; 4]) {
        if w <= 0.0 || h <= 0.0 || color[3] == 0 {
            return;
        }
        let st = ShapeStyle {
            fill: color,
            corner: r,
            w: Animated::new(w as f64 / 2.0),
            h: Animated::new(h as f64 / 2.0),
            ..ShapeStyle::new(ShapeKind::Rect)
        };
        let img = self.shapes.render(&st, self.rs, 0.0);
        let (x, y) = (self.px(x), self.px(y));
        blit(&mut self.out, &img, x, y, u32::MAX);
    }
    fn line(&mut self, pts: &[(f32, f32)], width: f32, color: [u8; 4]) {
        let c = self.centre;
        self.strokes.push(Stroke { color, width, points: pts.iter().map(|p| (p.0 - c.0, p.1 - c.1, 0.0)).collect() });
    }
    /// A dot is a zero-length stroke's round cap.
    fn dot(&mut self, x: f32, y: f32, r: f32, color: [u8; 4]) {
        self.line(&[(x, y), (x + 0.01, y)], r * 2.0, color);
    }
    fn arc(&mut self, cx: f32, cy: f32, r: f32, a0: f32, a1: f32, width: f32, color: [u8; 4]) {
        let pts: Vec<(f32, f32)> = (0..=16)
            .map(|i| {
                let a = (a0 + (a1 - a0) * i as f32 / 16.0).to_radians();
                (cx + r * a.cos(), cy + r * a.sin())
            })
            .collect();
        self.line(&pts, width, color);
    }
    /// Draw the collected strokes (one window-sized drawing) and forget them.
    fn flush(&mut self) {
        if self.strokes.is_empty() {
            return;
        }
        // an invisible stroke from corner to corner pins the drawing's layer to the window, so the
        // layer's centre is the window's centre
        let c = self.centre;
        self.strokes.push(Stroke { color: [0; 4], width: 0.5, points: vec![(-c.0, -c.1, 0.0), (c.0, c.1, 0.0)] });
        let st = drawing(std::mem::take(&mut self.strokes));
        let img = self.shapes.render(&st, self.rs, 0.0);
        let (x, y) = (self.px(self.centre.0) - img.width as i32 / 2, self.px(self.centre.1) - img.height as i32 / 2);
        blit(&mut self.out, &img, x, y, u32::MAX);
    }
    /// Text with its left edge at `x`, centred on `cy`, cut off after `max_w`. Returns its width.
    fn label(&mut self, s: &str, x: f32, cy: f32, size: f32, color: [u8; 4], max_w: f32) -> f32 {
        if s.trim().is_empty() || max_w <= 0.0 {
            return 0.0;
        }
        let mut st = TextStyle { text: s.to_string(), size: Animated::new(size as f64), color, align: 0, ..Default::default() };
        if !self.font.is_empty() {
            st.font = self.font.to_string();
        }
        let img = self.text.render(&st, self.rs, 0.0);
        let y = self.px(cy) - img.height as i32 / 2;
        let (x, cut) = (self.px(x), (max_w * self.rs) as u32);
        blit(&mut self.out, &img, x, y, cut);
        (img.width as f32 / self.rs).min(max_w)
    }
    /// A small picture (a favicon) fitted into a square at (x, y).
    fn icon(&mut self, img: &Frame, x: f32, y: f32, size: f32) {
        let n = (size * self.rs).round().max(1.0) as u32;
        if img.is_empty() {
            return;
        }
        let mut small = Frame::new(n, n);
        for j in 0..n {
            for i in 0..n {
                // ponytail: nearest sample - the decoder is asked for about this size already
                let (sx, sy) = (i * img.width / n, j * img.height / n);
                let si = (sy * img.width + sx) as usize * 4;
                let px = &img.rgba[si..si + 4];
                let px = if img.bgrx { [px[2], px[1], px[0], 255] } else { [px[0], px[1], px[2], px[3]] };
                small.rgba[(j * n + i) as usize * 4..][..4].copy_from_slice(&px);
            }
        }
        let (x, y) = (self.px(x), self.px(y));
        blit(&mut self.out, &small, x, y, u32::MAX);
    }
}

fn hash_f(h: &mut impl Hasher, v: &[f32]) {
    for f in v {
        f.to_bits().hash(h);
    }
}

/// A drawing of `strokes`, all shown at once. The shape cache keys a drawing by its strokes' count,
/// colour and width but not their points, so the points' hash rides in `corner` (unused by drawings).
fn drawing(strokes: Vec<Stroke>) -> ShapeStyle {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for p in strokes.iter().flat_map(|s| s.points.iter()) {
        (p.0.to_bits(), p.1.to_bits()).hash(&mut h);
    }
    ShapeStyle {
        strokes,
        draw_rate: 0.0,
        corner: (h.finish() >> 40) as f32,
        ..ShapeStyle::new(ShapeKind::Draw)
    }
}

/// Loads a still for the rasteriser: (path, wanted width, wanted height).
pub type ImageLoader<'a> = &'a mut dyn FnMut(&str, u32, u32) -> Option<Arc<Frame>>;

/// Cached bitmaps of browser clips. One per compositor, like `ShapeRasterizer`.
#[derive(Default)]
pub struct BrowserRasterizer {
    shapes: ShapeRasterizer,
    /// Window art without the address text and load bar, by style hash.
    base: Vec<(u64, Arc<Frame>)>,
    /// The finished window, by (base, address, load) hash.
    full: Vec<(u64, Arc<Frame>)>,
    bg: Vec<(u64, Arc<Frame>)>,
    cursor: Vec<(u64, Arc<Frame>)>,
    /// Stamped into each new frame's `pts`: the GPU re-uploads a texture when (pointer, pts) changes,
    /// and a freed bitmap's address can be handed out again.
    stamp: u64,
}

fn cached(list: &[(u64, Arc<Frame>)], k: u64) -> Option<Arc<Frame>> {
    list.iter().find(|(key, _)| *key == k).map(|(_, f)| f.clone())
}

fn remember(list: &mut Vec<(u64, Arc<Frame>)>, k: u64, f: Arc<Frame>, cap: usize) {
    if list.len() >= cap {
        list.remove(0);
    }
    list.push((k, f));
}

impl BrowserRasterizer {
    fn stamped(&mut self, mut f: Frame) -> Arc<Frame> {
        self.stamp += 1;
        f.pts = self.stamp as f64;
        Arc::new(f)
    }

    /// Every bitmap of the clip's browser at clip-local `lt`. `s` = canvas px per project px, `rs` =
    /// the window bitmap's own scale (`raster_scale`).
    pub fn layers(
        &mut self,
        b: &BrowserStyle,
        lt: f64,
        s: f32,
        rs: f32,
        text: &mut TextRasterizer,
        load: ImageLoader,
    ) -> Layers {
        let window = b.display == BrowserDisplay::Window;
        Layers {
            // (Page only has no background: the slot carries the load bar's colour while it runs)
            bg: if window { self.background(b, s, load) } else { self.load_colour(b, lt) },
            window: window.then(|| self.window(b, lt, rs, text, load)),
            // drawn at the window's own scale, so a zoomed-in pointer is as sharp as the rest
            cursor: self.cursor_sprite(b, lt, rs.max(s)),
            keys: keys_overlay(b, lt, s, text),
        }
    }

    /// One pixel of the look's accent colour while the load bar runs (Page only stretches it into a bar).
    fn load_colour(&mut self, b: &BrowserStyle, lt: f64) -> Option<Arc<Frame>> {
        let v = b.load.at(lt);
        (v > 0.0 && v < 1.0).then(|| {
            let mut f = Frame::new(1, 1);
            f.rgba.copy_from_slice(&pal(b.theme).accent);
            self.stamped(f)
        })
    }

    /// A top-to-bottom gradient strip or the background picture; None for one flat colour (the
    /// compositors clear to `bg`).
    fn background(&mut self, b: &BrowserStyle, s: f32, load: ImageLoader) -> Option<Arc<Frame>> {
        if !b.bg_image.is_empty() {
            // ponytail: asked for at 1080 rows whatever the canvas - a background is soft anyway
            let n = (1080.0 * s.min(1.0)).max(2.0) as u32;
            if let Some(f) = load(&b.bg_image, n * 16 / 9, n) {
                return Some(f);
            }
        }
        if b.bg[..3] == b.bg2[..3] {
            return None;
        }
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (b.bg, b.bg2).hash(&mut h);
        let k = h.finish();
        if let Some(f) = cached(&self.bg, k) {
            return Some(f);
        }
        let mut f = Frame::new(1, 256);
        for (y, px) in f.rgba.chunks_exact_mut(4).enumerate() {
            for i in 0..3 {
                px[i] = (b.bg[i] as f32 + (b.bg2[i] as f32 - b.bg[i] as f32) * y as f32 / 255.0).round() as u8;
            }
            px[3] = 255;
        }
        let f = self.stamped(f);
        remember(&mut self.bg, k, f.clone(), 4);
        Some(f)
    }

    fn window(&mut self, b: &BrowserStyle, lt: f64, rs: f32, text: &mut TextRasterizer, load: ImageLoader) -> Arc<Frame> {
        let tab = b.tab_at(lt);
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (b.theme, tab, b.show_tabs, b.show_toolbar, b.show_controls, b.show_bookmarks).hash(&mut h);
        hash_f(&mut h, &[b.w, b.h, b.corner, b.shadow, rs]);
        (&b.font, &b.bookmarks).hash(&mut h);
        for (i, t) in b.tabs.iter().enumerate() {
            (b.title_at(i, lt), b.favicon_at(i, lt), &t.favicon).hash(&mut h);
        }
        let base_key = h.finish();
        let url = b.url_at(lt);
        let load_v = b.load.at(lt).clamp(0.0, 1.0) as f32;
        let load_px = if load_v > 0.0 && load_v < 1.0 { (load_v * b.w * rs).round() as u32 } else { 0 };
        (url, load_px).hash(&mut h);
        let full_key = h.finish();
        if let Some(f) = cached(&self.full, full_key) {
            return f;
        }
        let base = match cached(&self.base, base_key) {
            Some(f) => f,
            None => {
                let f = Arc::new(self.draw_base(b, tab, lt, rs, text, load));
                remember(&mut self.base, base_key, f.clone(), 4);
                f
            }
        };
        // the moving parts on a copy of the cached art
        let p = pal(b.theme);
        let m = margin(b);
        let mut art = Art {
            out: (*base).clone(),
            rs,
            m,
            shapes: &mut self.shapes,
            text,
            font: &b.font,
            strokes: Vec::new(),
            centre: (b.w / 2.0, b.h / 2.0),
        };
        if b.show_toolbar {
            let bar = toolbar(b);
            art.label(url, bar.pill.0 + 36.0, bar.y + bar.h / 2.0, 14.5, p.text, bar.pill.1 - 48.0);
            if load_px > 0 {
                art.rect(0.0, bar.y + bar.h - 3.0, load_v * b.w, 3.0, 1.5, p.accent);
            }
        }
        let out = art.out;
        let f = self.stamped(out);
        remember(&mut self.full, full_key, f.clone(), 6);
        f
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_base(&mut self, b: &BrowserStyle, active: usize, lt: f64, rs: f32, text: &mut TextRasterizer, load: ImageLoader) -> Frame {
        let p = pal(b.theme);
        let m = margin(b);
        let (w, h) = (b.w.max(16.0), b.h.max(16.0));
        let (bw, bh) = (((w + 2.0 * m) * rs).round().max(1.0) as u32, ((h + 2.0 * m) * rs).round().max(1.0) as u32);
        let mut art = Art {
            out: Frame::new(bw, bh),
            rs,
            m,
            shapes: &mut self.shapes,
            text,
            font: &b.font,
            strokes: Vec::new(),
            centre: (w / 2.0, h / 2.0),
        };
        let r = b.corner.clamp(0.0, w.min(h) / 2.0);
        // shadow: stacked soft rectangles, widest and faintest first, dropped a little below
        if b.shadow > 0.0 {
            const N: usize = 10;
            let sh = b.shadow.min(400.0);
            for i in (1..=N).rev() {
                let e = sh * i as f32 / N as f32;
                art.rect(-e, -e + sh * 0.25, w + 2.0 * e, h + 2.0 * e, r + e, [0, 0, 0, 14]);
            }
        }
        let (ch, fh) = (b.chrome_h(), b.foot_h());
        // body in the frame colour, then the page area in the page colour (leaving the rounded
        // bottom corners to the body when there is no foot bar to hold them)
        art.rect(0.0, 0.0, w, h, r, if ch + fh > 0.0 { p.frame } else { p.page });
        if fh > 0.0 {
            art.rect(0.0, ch, w, h - ch - fh, 0.0, p.page);
            art.rect(w / 2.0 - 60.0, h - fh / 2.0 - 2.5, 120.0, 5.0, 2.5, p.dim);
        } else if ch > 0.0 {
            art.rect(0.0, ch + r, w, h - ch - 2.0 * r, 0.0, p.page);
            art.rect(0.0, h - 3.0 * r.max(1.0), w, 3.0 * r.max(1.0), r, p.page);
            art.rect(0.0, ch, w, r + 1.0, 0.0, p.page);
        }
        let slim = matches!(b.theme, BrowserTheme::Minimal | BrowserTheme::Mobile);
        let dots = |art: &mut Art, cy: f32| {
            for i in 0..3 {
                art.dot(20.0 + 20.0 * i as f32, cy, 6.0, [p.dim[0], p.dim[1], p.dim[2], 120]);
            }
        };
        let mut y = 0.0;
        if b.show_tabs && !slim {
            let x0 = if b.show_controls {
                dots(&mut art, 20.0);
                84.0
            } else {
                10.0
            };
            let n = b.tabs.len().max(1) as f32;
            let tw = ((w - x0 - 56.0) / n).clamp(40.0, 240.0);
            for i in 0..b.tabs.len() {
                let x = x0 + tw * i as f32;
                let on = i == active;
                if on {
                    art.rect(x, 6.0, tw, 40.0, 9.0, p.bar);
                } else if i + 1 != active && i + 1 < b.tabs.len() {
                    art.line(&[(x + tw, 14.0), (x + tw, 28.0)], 1.0, [p.dim[0], p.dim[1], p.dim[2], 110]);
                }
                let mut tx = x + 12.0;
                let icon = b.favicon_at(i, lt);
                if let Some(f) = (!icon.is_empty()).then(|| load(icon, 32, 32)).flatten() {
                    art.icon(&f, tx, 14.0, 16.0);
                    tx += 24.0;
                }
                art.label(b.title_at(i, lt), tx, 23.0, 13.0, if on { p.text } else { p.dim }, x + tw - 30.0 - tx);
                let (cx, cy) = (x + tw - 16.0, 23.0);
                art.line(&[(cx - 3.5, cy - 3.5), (cx + 3.5, cy + 3.5)], 1.4, p.dim);
                art.line(&[(cx - 3.5, cy + 3.5), (cx + 3.5, cy - 3.5)], 1.4, p.dim);
            }
            let (cx, cy) = (x0 + tw * b.tabs.len() as f32 + 20.0, 23.0);
            art.line(&[(cx - 5.0, cy), (cx + 5.0, cy)], 1.6, p.dim);
            art.line(&[(cx, cy - 5.0), (cx, cy + 5.0)], 1.6, p.dim);
            y = 40.0;
        }
        if b.show_toolbar {
            let bar = toolbar(b);
            let cy = bar.y + bar.h / 2.0;
            if !slim {
                art.rect(0.0, bar.y, w, bar.h, if y == 0.0 { r } else { 0.0 }, p.bar);
                if y == 0.0 {
                    art.rect(0.0, bar.y + bar.h / 2.0, w, bar.h / 2.0, 0.0, p.bar);
                }
                // back, forward, reload
                art.line(&[(27.0, cy - 6.0), (21.0, cy), (27.0, cy + 6.0)], 2.0, p.dim);
                art.line(&[(55.0, cy - 6.0), (61.0, cy), (55.0, cy + 6.0)], 2.0, [p.dim[0], p.dim[1], p.dim[2], 120]);
                reload(&mut art, 92.0, cy, p.dim);
                for i in -1..=1 {
                    art.dot(w - 26.0, cy + 5.5 * i as f32, 1.7, p.dim);
                }
            } else {
                if b.theme == BrowserTheme::Minimal && b.show_controls {
                    dots(&mut art, cy);
                }
                reload(&mut art, bar.pill.0 + bar.pill.1 - 20.0, cy, p.dim);
            }
            art.rect(bar.pill.0, cy - 16.0, bar.pill.1, 32.0, 16.0, p.pill);
            // a padlock: body and shackle
            let lx = bar.pill.0 + 20.0;
            art.rect(lx - 5.0, cy - 1.5, 10.0, 8.0, 2.0, p.dim);
            art.arc(lx, cy - 1.5, 3.2, 180.0, 360.0, 1.6, p.dim);
            y = bar.y + bar.h;
        }
        if b.show_bookmarks && !slim {
            art.rect(0.0, y, w, 30.0, 0.0, p.bar);
            art.flush();
            let mut x = 16.0;
            for name in &b.bookmarks {
                if x > w - 60.0 {
                    break;
                }
                art.dot(x + 5.0, y + 15.0, 5.0, [p.dim[0], p.dim[1], p.dim[2], 90]);
                x += 16.0 + art.label(name, x + 16.0, y + 15.0, 12.5, p.dim, w - x - 32.0) + 22.0;
            }
        }
        // a hairline under the bars
        if ch > 0.0 {
            art.rect(0.0, ch - 1.0, w, 1.0, 0.0, [p.dim[0], p.dim[1], p.dim[2], 50]);
        }
        art.flush();
        art.out
    }

    /// The pointer with whatever click ripples / wheel marks are alive, hotspot in the middle.
    fn cursor_sprite(&mut self, b: &BrowserStyle, lt: f64, s: f32) -> Option<Arc<Frame>> {
        let c = b.cursor.as_ref().filter(|c| c.visible)?;
        let k = c.size.clamp(0.2, 8.0);
        let shape = c.shape_at(lt);
        let pressed = c.pressed(lt);
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (shape, pressed, c.color, c.outline, c.ripple).hash(&mut h);
        hash_f(&mut h, &[k, s]);
        let mut marks = Vec::new();
        for (age, e) in c.live(lt) {
            match e.kind {
                CursorEventKind::Click | CursorEventKind::Down => marks.push((age, 0.0, 0)),
                CursorEventKind::DoubleClick => marks.extend([(age, 0.0, 0), ((age - 0.3).max(0.0), 0.0, 0)]),
                CursorEventKind::Scroll(d) => marks.push((age, d, 1)),
                _ => {}
            }
        }
        for m in &marks {
            ((m.0 * 64.0) as u32, m.1.to_bits(), m.2).hash(&mut h);
        }
        let key = h.finish();
        if let Some(f) = cached(&self.cursor, key) {
            return Some(f);
        }
        let half = SPRITE * k;
        let n = (half * 2.0 * s).round().max(2.0) as u32;
        let mut art_out = Frame::new(n, n);
        let centre = |img: &Frame, out: &mut Frame| {
            blit(out, img, (n as i32 - img.width as i32) / 2, (n as i32 - img.height as i32) / 2, u32::MAX)
        };
        for &(age, dir, kind) in &marks {
            let a = ((1.0 - age) * c.ripple[3] as f32) as u8;
            let col = [c.ripple[0], c.ripple[1], c.ripple[2], a];
            if a == 0 || age <= 0.0 && kind == 0 && marks.len() > 1 {
                continue;
            }
            let st = if kind == 0 {
                let r = ((8.0 + 30.0 * age) * k) as f64;
                ShapeStyle {
                    fill: [col[0], col[1], col[2], a / 4],
                    stroke: col,
                    stroke_width: 3.0 * k,
                    w: Animated::new(r),
                    h: Animated::new(r),
                    ..ShapeStyle::new(ShapeKind::Ellipse)
                }
            } else {
                // wheel: a chevron sliding the way the page moves
                let y = dir.signum() * (16.0 + 14.0 * age) * k;
                let d = dir.signum() * 6.0 * k;
                drawing(vec![Stroke {
                    color: col,
                    width: 3.0 * k,
                    points: vec![(-7.0 * k, y - d, 0.0), (0.0, y, 0.0), (7.0 * k, y - d, 0.0)],
                }])
            };
            centre(&self.shapes.render(&st, s, 0.0), &mut art_out);
        }
        let g = k * if pressed { 0.86 } else { 1.0 };
        let st = match shape {
            CursorShape::Text => drawing(
                [(4.2, c.outline), (1.8, c.color)]
                    .into_iter()
                    .flat_map(|(wd, col)| {
                        [[(0.0, -10.0), (0.0, 10.0)], [(-3.5, -10.0), (3.5, -10.0)], [(-3.5, 10.0), (3.5, 10.0)]].map(|l| {
                            Stroke { color: col, width: wd * g, points: l.iter().map(|p| (p.0 * g, p.1 * g, 0.0)).collect() }
                        })
                    })
                    .collect(),
            ),
            _ => {
                // outlines with the hotspot (arrow tip / fingertip) at the origin
                const ARROW: [(f32, f32); 7] =
                    [(0.0, 0.0), (0.0, 19.0), (4.6, 14.8), (7.6, 21.6), (10.6, 20.3), (7.7, 13.6), (13.6, 13.6)];
                const HAND: [(f32, f32); 14] = [
                    (-2.4, 1.2),
                    (0.0, -0.4),
                    (2.4, 1.2),
                    (2.4, 9.0),
                    (10.0, 10.6),
                    (11.0, 13.0),
                    (10.4, 19.0),
                    (8.0, 23.0),
                    (-3.6, 23.0),
                    (-8.6, 16.6),
                    (-9.4, 13.2),
                    (-7.0, 12.2),
                    (-2.4, 15.0),
                    (-2.4, 1.2),
                ];
                let pts: &[(f32, f32)] = if shape == CursorShape::Hand { &HAND } else { &ARROW };
                ShapeStyle {
                    fill: c.color,
                    stroke: c.outline,
                    stroke_width: 1.6 * g,
                    points: pts.iter().map(|p| (p.0 * g, p.1 * g)).collect(),
                    ..ShapeStyle::new(ShapeKind::Polygon)
                }
            }
        };
        centre(&self.shapes.render(&st, s, 0.0), &mut art_out);
        let f = self.stamped(art_out);
        remember(&mut self.cursor, key, f.clone(), 8);
        Some(f)
    }
}

struct Toolbar {
    y: f32,
    h: f32,
    /// Address pill: (x, width).
    pill: (f32, f32),
}

fn toolbar(b: &BrowserStyle) -> Toolbar {
    match b.theme {
        BrowserTheme::Mobile => Toolbar { y: 0.0, h: 48.0, pill: (22.0, b.w - 44.0) },
        BrowserTheme::Minimal => Toolbar { y: 0.0, h: 48.0, pill: (b.w * 0.2, b.w * 0.6) },
        _ => Toolbar { y: if b.show_tabs { 40.0 } else { 0.0 }, h: 46.0, pill: (116.0, (b.w - 116.0 - 52.0).max(40.0)) },
    }
}

/// A circle with a gap and an arrow head.
fn reload(art: &mut Art, cx: f32, cy: f32, color: [u8; 4]) {
    art.arc(cx, cy, 6.5, -50.0, 250.0, 2.0, color);
    art.line(&[(cx + 1.0, cy - 9.5), (cx + 5.0, cy - 5.5), (cx + 0.5, cy - 2.5)], 2.0, color);
}

/// The keys typed lately in a rounded box, and how visible it is.
fn keys_overlay(b: &BrowserStyle, lt: f64, s: f32, text: &mut TextRasterizer) -> Option<(Arc<Frame>, f32)> {
    let c = b.cursor.as_ref().filter(|c| c.keys)?;
    let mut shown = String::new();
    let mut youngest = 1.0f32;
    for (age, e) in c.live(lt) {
        if let CursorEventKind::Key(k) = &e.kind {
            if !shown.is_empty() && (k.chars().count() > 1 || shown.contains('+')) {
                shown.push(' ');
            }
            shown.push_str(k);
            youngest = youngest.min(age);
        }
    }
    if shown.is_empty() {
        return None;
    }
    let st = TextStyle {
        text: shown,
        size: Animated::new(30.0),
        bold: true,
        color: [255, 255, 255, 255],
        box_color: [20, 22, 28, 215],
        box_padding: 14.0,
        ..Default::default()
    };
    let img = text.render(&st, s, 0.0);
    (img.width > 1).then(|| (img, ((1.0 - youngest) * 5.0).clamp(0.0, 1.0)))
}

// ---------------------------------------------------------------------------------------------------
// Zoom details: the parts of an HTML page a browser layer zooms into, drawn again by the HTML engine at
// the scale they are shown at (`webpage::render_region`, cached on disk by source + fields + region +
// scale). They are NOT part of the project: no clips, no assets, nothing on the timeline or in the
// Library. `render_view` adds them to the copy of the project the player / export / pre-render draw.

/// One zoomed part of the page under a browser clip: clip-local `from..to`, `rect` = where it goes
/// (project px from the frame's middle, before scroll and zoom), the picture and its size.
#[derive(Clone, Debug, PartialEq)]
pub struct ZoomDetail {
    pub from: f64,
    pub to: f64,
    pub rect: (f64, f64, f64, f64),
    pub path: String,
    pub size: (u32, u32),
}

static DETAILS: std::sync::Mutex<Vec<(crate::model::Id, Vec<ZoomDetail>)>> = std::sync::Mutex::new(Vec::new());

fn details() -> std::sync::MutexGuard<'static, Vec<(crate::model::Id, Vec<ZoomDetail>)>> {
    DETAILS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Replace browser clip `browser`'s zoom details.
pub fn set_zoom_details(browser: crate::model::Id, list: Vec<ZoomDetail>) {
    let mut s = details();
    s.retain(|(b, _)| *b != browser);
    if !list.is_empty() {
        s.push((browser, list));
    }
}

/// Forget every zoom detail (another project was opened).
pub fn clear_zoom_details() {
    details().clear();
}

/// Browser clip `browser`'s zoom details as they are now.
pub fn zoom_details(browser: crate::model::Id) -> Vec<ZoomDetail> {
    details().iter().find(|(b, _)| *b == browser).map(|(_, l)| l.clone()).unwrap_or_default()
}

/// Ids of what `render_view` adds start here, far above any id a project hands out.
const VIEW_IDS: crate::model::Id = 1 << 40;

/// The project as it is drawn: `p` plus the zoom details of its browser clips as picture layers over
/// their pages (a copy; `p` itself is untouched). Borrowed when there is nothing to add.
pub fn render_view(p: &crate::model::Project) -> std::borrow::Cow<'_, crate::model::Project> {
    let s = details();
    let mine: Vec<&(crate::model::Id, Vec<ZoomDetail>)> =
        s.iter().filter(|(b, _)| p.clip(*b).is_some_and(|c| c.kind == ClipKind::Browser)).collect();
    if mine.is_empty() {
        return std::borrow::Cow::Borrowed(p);
    }
    let mut v = p.clone();
    v.next_id = v.next_id.max(VIEW_IDS);
    for (b, list) in mine {
        for d in list {
            let json = serde_json::json!({"id": 0, "path": d.path, "kind": "Image", "duration": 0.0, "width": d.size.0, "height": d.size.1, "fps": 0.0});
            // through text: from_value would compile a second copy of Asset's deserialiser
            let Ok(a) = serde_json::from_str::<crate::model::Asset>(&json.to_string()) else { continue };
            let aid = v.add_asset(a);
            v.browser_add_detail(*b, aid, d.from, d.to, d.rect);
        }
    }
    std::borrow::Cow::Owned(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Cursor, CursorEvent, Ease, Keyframe};

    fn clip() -> Clip {
        let mut c = Clip::new(7, ClipKind::Browser, "Browser", 0.0, 10.0);
        let b = c.browser.as_mut().unwrap();
        (b.w, b.h, b.shadow) = (1600.0, 986.0, 0.0);
        c
    }

    #[test]
    fn layers_below_are_placed_straight_into_the_page_area() {
        // 1920x1080 project, canvas at half size; window 1600x986 project px, 86 px of bars
        let c = clip();
        let p = plan(1920, &c, 0.0, 960, 540).unwrap();
        assert_eq!(c.browser.as_ref().unwrap().chrome_h(), 86.0);
        assert!((p.window.w - 800.0).abs() < 0.01 && (p.window.h - 493.0).abs() < 0.01, "{:?}", p.window);
        assert_eq!((p.page.w, p.page.h), (960.0, 540.0), "the page is the canvas: nothing is scaled twice");
        // the page's top-left lands on the page area's top-left, its width on the window's width
        let v = view_of(&c, 0.0, 1920, 960, 540);
        let (a, z) = (v.point((0.0, 0.0)), v.point((960.0, 0.0)));
        assert!((a.0 - 80.0).abs() < 0.01 && (a.1 - (270.0 - 246.5 + 43.0)).abs() < 0.01, "{a:?}");
        assert!((z.0 - 880.0).abs() < 0.01 && (v.scale - 800.0 / 960.0).abs() < 1e-4, "{z:?} {}", v.scale);
        let back = v.page_point(v.point((123.0, 45.0)));
        assert!((back.0 - 123.0).abs() < 0.01 && (back.1 - 45.0).abs() < 0.01);
        // a layer placed over the whole frame is placed over the page area, at that size
        let mut full = Placement { cx: 480.0, cy: 270.0, w: 960.0, h: 540.0, rot: 0.0, yaw: 0.0, pitch: 0.0 };
        v.apply(&mut full);
        assert!((full.w - 800.0).abs() < 0.01 && (full.cx - 480.0).abs() < 0.01, "{full:?}");
        // move + scale + turn the window: the layers follow it
        let mut c2 = clip();
        (c2.x, c2.scale, c2.rotation) = (Animated::new(100.0), Animated::new(0.5), Animated::new(90.0));
        let v2 = view_of(&c2, 0.0, 1920, 960, 540);
        assert!((v2.scale - 400.0 / 960.0).abs() < 1e-4);
        let mut l = Placement { cx: 480.0, cy: 270.0, w: 960.0, h: 540.0, rot: 0.0, yaw: 0.0, pitch: 0.0 };
        v2.apply(&mut l);
        assert_eq!(l.rot, 90.0);
    }

    #[test]
    fn window_zoom_scales_the_whole_browser_about_its_point() {
        let mut c = clip();
        let b = c.browser.as_mut().unwrap();
        b.cursor.as_mut().unwrap().x = Animated::new(200.0);
        let base = plan(1920, &c, 0.0, 960, 540).unwrap();
        let (cur0, v0) = (base.cursor(c.browser.as_ref().unwrap(), 0.0, 0.5).unwrap(), view_of(&c, 0.0, 1920, 960, 540));
        // 3x about a point 100 project px right of the frame's middle
        let b = c.browser.as_mut().unwrap();
        (b.wzoom, b.wzoom_x) = (Animated::new(3.0), Animated::new(100.0));
        let z = plan(1920, &c, 0.0, 960, 540).unwrap();
        let f = (530.0, 270.0);
        let about = |p: (f32, f32)| (f.0 + (p.0 - f.0) * 3.0, f.1 + (p.1 - f.1) * 3.0);
        // the frame, the page area, the page's layers and the mouse: all three times bigger, about it
        assert!((z.window.w - base.window.w * 3.0).abs() < 0.01);
        let want = about((base.window.cx, base.window.cy));
        assert!((z.window.cx - want.0).abs() < 0.01 && (z.window.cy - want.1).abs() < 0.01);
        for (a, b) in z.mask.points.iter().zip(&base.mask.points) {
            let want = about(*b);
            assert!((a.0 - want.0).abs() < 0.01 && (a.1 - want.1).abs() < 0.01);
        }
        let v = view_of(&c, 0.0, 1920, 960, 540);
        assert!((v.scale - v0.scale * 3.0).abs() < 1e-4);
        let (got, want) = (v.point((300.0, 200.0)), about(v0.point((300.0, 200.0))));
        assert!((got.0 - want.0).abs() < 0.01 && (got.1 - want.1).abs() < 0.01, "{got:?} {want:?}");
        let cur = z.cursor(c.browser.as_ref().unwrap(), 0.0, 0.5).unwrap();
        assert!((cur.w - cur0.w * 3.0).abs() < 0.01, "the pointer is three times bigger too");
        let want = about((cur0.cx, cur0.cy));
        assert!((cur.cx - want.0).abs() < 0.01 && (cur.cy - want.1).abs() < 0.01);
        // and everything is drawn at that size, in steps: the window art at 3x rounds up to 4x (capped by
        // the widest bitmap), the layers below are asked for at 3x their placement
        assert_eq!(raster_scale(&clip(), 0.5, 0.0), 0.5);
        assert!(raster_scale(&c, 0.5, 0.0) > 1.5, "{}", raster_scale(&c, 0.5, 0.0));
        assert_eq!(level(1.0), 1.0);
        assert!(level(3.0) >= 3.0 && level(3.0) <= 4.01 && level(1.3) >= 1.3);
        let mut tracks = vec![Track::new(1, TrackKind::Video, "V1"), Track::new(2, TrackKind::Video, "V2")];
        tracks[1].clips.push(c);
        assert!((zoom_on_screen(&tracks, 1.0, 1920, 1080) - 2.5).abs() < 0.01, "3 x the window's 1600 / 1920");
        assert_eq!(zoom_on_screen(&tracks, 11.0, 1920, 1080), 1.0);
        assert_eq!(max_zoom_over(&tracks, 5.0, 20.0), 3.0);
    }

    #[test]
    fn scroll_and_page_zoom_move_only_the_tracks_below() {
        let mut tracks = vec![Track::new(1, TrackKind::Video, "V1"), Track::new(2, TrackKind::Video, "V2")];
        let mut c = clip();
        let b = c.browser.as_mut().unwrap();
        b.display = BrowserDisplay::Page; // the page is the frame: the numbers are plain
        b.scroll.keys = vec![
            Keyframe { t: 0.0, v: 0.0, ease: Ease::Linear },
            Keyframe { t: 2.0, v: 400.0, ease: Ease::Linear },
        ];
        tracks[1].clips.push(c);
        // 200 project px of scroll at 1 s = 100 canvas px up
        let v = layer_view(&tracks, 0, 1.0, 1920, 960, 540);
        assert_eq!((v.point((480.0, 270.0)), v.scale), ((480.0, 170.0), 1.0));
        assert!(layer_view(&tracks, 1, 1.0, 1920, 960, 540).is_none(), "the browser's own track stays");
        assert!(layer_view(&tracks, 0, 11.0, 1920, 960, 540).is_none(), "after the clip nothing moves");
        assert!(layer_view(&tracks, 0, 0.0, 1920, 960, 540).is_none(), "an untouched Page-only browser changes nothing");
        // page Zoom 4 about a point: that point stays (after the scroll), the rest moves away 4x
        let b = tracks[1].clips[0].browser.as_mut().unwrap();
        (b.zoom, b.zoom_x, b.zoom_y) = (Animated::new(4.0), Animated::new(100.0), Animated::new(-50.0));
        let v = layer_view(&tracks, 0, 1.0, 1920, 960, 540);
        assert_eq!(v.point((530.0, 345.0)), (530.0, 245.0));
        assert_eq!(v.point((540.0, 345.0)), (570.0, 245.0));
        assert_eq!(v.scale, 4.0);
    }

    #[test]
    fn zoom_levels_cover_every_moment_of_a_move_at_screen_sharpness() {
        // Page only, zoom 1 -> 3.6 from 2 s to 3 s about a point, held, back out by 7 s
        let mut c = clip();
        let b = c.browser.as_mut().unwrap();
        b.display = BrowserDisplay::Page;
        b.zoom_to(2.0, 1.0, 3.6, Some((200.0, -100.0)));
        b.glide(true, 6.0, 1.0, 1.0);
        let levels = zoom_levels(&c, 1920, 1080);
        // in: a 2x level, then a 4x level that also holds the rest; out: 2x again
        let scales: Vec<f64> = levels.iter().map(|l| l.scale).collect();
        assert_eq!(scales, vec![2.0, 4.0, 2.0], "{levels:?}");
        assert!(levels.windows(2).all(|w| w[0].to <= w[1].from + 1e-9), "spans do not overlap: {levels:?}");
        // at every moment the level on screen is at least as sharp as the zoom, and holds what is seen
        for i in 0..=150 {
            let lt = i as f64 / 15.0;
            let v = view_of(&c, lt, 1920, 1920, 1080);
            if v.scale <= 1.03 {
                continue;
            }
            let l = levels.iter().find(|l| l.from <= lt + 1e-9 && lt <= l.to + 1e-9).unwrap_or_else(|| panic!("no level at {lt}"));
            assert!(l.scale as f32 >= v.scale * 0.98, "at {lt}: level {} under zoom {}", l.scale, v.scale);
            let (a, z) = (v.page_point((0.0, 0.0)), v.page_point((1920.0, 1080.0)));
            assert!(l.rect.0 <= a.0 as f64 - 960.0 + 0.5 && l.rect.2 >= z.0 as f64 - 960.0 - 0.5, "at {lt}: {:?} misses {a:?}..{z:?}", l.rect);
        }
        // the 4x level is the small part seen at rest, the 2x levels the wide part seen on the way
        assert!(levels[1].rect.2 - levels[1].rect.0 < 1000.0 && levels[0].rect.2 - levels[0].rect.0 > 1000.0);
        // a Window zoom counts the same way
        let mut w = clip();
        w.browser.as_mut().unwrap().display = BrowserDisplay::Page;
        w.browser.as_mut().unwrap().wzoom = Animated::new(8.0);
        assert_eq!(zoom_levels(&w, 1920, 1080).iter().map(|l| l.scale).collect::<Vec<_>>(), vec![8.0]);
    }

    #[test]
    fn page_only_is_the_frame_itself() {
        // whatever the window's size and the clip's position say, Page only is the whole frame
        let mut c = clip();
        c.browser.as_mut().unwrap().display = BrowserDisplay::Page;
        (c.x, c.scale, c.rotation) = (Animated::new(300.0), Animated::new(0.5), Animated::new(20.0));
        let p = plan(1920, &c, 0.0, 960, 540).unwrap();
        assert!(p.page_only);
        let mut c = c;
        assert_eq!((p.page.cx, p.page.cy, p.page.w, p.page.h, p.page.rot), (480.0, 270.0, 960.0, 540.0, 0.0));
        assert!(view_of(&c, 0.0, 1920, 960, 540).is_none());
        // the load bar: a thin strip along the top while it runs, nothing otherwise
        assert!(p.load_bar(c.browser.as_ref().unwrap(), 0.0, 0.5).is_none());
        c.browser.as_mut().unwrap().load = Animated::new(0.5);
        let bar = p.load_bar(c.browser.as_ref().unwrap(), 0.0, 0.5).unwrap();
        assert_eq!((bar.w, bar.cx, bar.h), (480.0, 240.0, 2.0));
        c.browser.as_mut().unwrap().load = Animated::new(0.0);
        assert_eq!((p.window.w, p.window.h), (960.0, 540.0), "no shadow margin");
        // the mouse is in frame coordinates: the page's middle is the frame's middle
        assert_eq!(p.page_to_canvas((0.0, 0.0)), (480.0, 270.0));
        assert_eq!(p.canvas_to_page((960.0, 540.0)), (960.0, 540.0));
        // nothing is drawn but the mouse: no window art, no background
        let mut r = BrowserRasterizer::default();
        let mut text = TextRasterizer::new();
        let l = r.layers(c.browser.as_ref().unwrap(), 1.0, 0.5, 0.5, &mut text, &mut |_: &str, _: u32, _: u32| None);
        assert!(l.window.is_none() && l.bg.is_none() && l.cursor.is_some());
    }

    #[test]
    fn mask_is_the_page_area_with_round_bottom_corners() {
        let c = clip();
        let p = plan(1920, &c, 0.0, 1920, 1080).unwrap();
        let (xs, ys): (Vec<f32>, Vec<f32>) = p.mask.points.iter().copied().unzip();
        let (x0, x1) = (xs.iter().cloned().fold(f32::MAX, f32::min), xs.iter().cloned().fold(f32::MIN, f32::max));
        let (y0, y1) = (ys.iter().cloned().fold(f32::MAX, f32::min), ys.iter().cloned().fold(f32::MIN, f32::max));
        assert!((x1 - x0 - 1600.0).abs() < 0.01 && (y1 - y0 - 900.0).abs() < 0.01, "{x0} {x1} {y0} {y1}");
        assert_eq!(p.mask.points.len(), 2 + 14, "square top corners, 7 points per bottom corner");
        assert!(p.mask.points.len() <= 64);
    }

    #[test]
    fn cursor_coordinates_round_trip_through_a_rotated_scaled_window() {
        let mut c = clip();
        c.x = Animated::new(-120.0);
        c.y = Animated::new(40.0);
        c.scale = Animated::new(0.8);
        c.rotation = Animated::new(12.0);
        let p = plan(1920, &c, 0.0, 1280, 720).unwrap();
        for page in [(0.0, 0.0), (-900.0, -500.0), (412.5, 300.25)] {
            let back = p.canvas_to_page(p.page_to_canvas(page));
            assert!((back.0 - page.0).abs() < 0.01 && (back.1 - page.1).abs() < 0.01, "{page:?} -> {back:?}");
        }
        // the page centre is where the layers' own middle is shown
        let (mid, v) = (p.page_to_canvas((0.0, 0.0)), view_of(&c, 0.0, 1920, 1280, 720));
        let shown = v.point((640.0, 360.0));
        assert!((mid.0 - shown.0).abs() < 0.01 && (mid.1 - shown.1).abs() < 0.01);
    }

    /// The whole thing through the CPU compositor: a blue page under a browser ends up inside the
    /// window, the background around it, the bars above it - and scrolling moves the page only.
    #[test]
    fn the_page_below_shows_inside_the_window() {
        use crate::engine::compose::Compositor;
        use crate::media::{Backend, DecoderPool};
        use crate::model::Project;
        let mut p = Project::new();
        (p.width, p.height) = (640, 360);
        // the page: the top half blue, the bottom half (a second shape) green
        for (y, fill) in [(-90.0, [0, 0, 255, 255]), (90.0, [0, 255, 0, 255])] {
            let id = p.add_shape_clip(ShapeKind::Rect, 0.0, 4.0);
            let c = p.clip_mut(id).unwrap();
            c.y = Animated::new(y);
            let st = c.shape.as_mut().unwrap();
            (st.fill, st.w.value, st.h.value) = (fill, 320.0, 90.0);
        }
        let id = p.add_browser_clip(0.0, 2.0);
        let b = p.clip_mut(id).unwrap().browser.as_mut().unwrap();
        (b.w, b.h, b.shadow, b.corner) = (400.0, 311.0, 0.0, 0.0); // page area 400x225: 16:9
        (b.bg, b.bg2, b.cursor) = ([200, 10, 10, 255], [200, 10, 10, 255], None);
        let mut pool = DecoderPool::new(Backend::Ffmpeg);
        let (mut comp, mut text, mut out) = (Compositor::new(), TextRasterizer::new(), Frame::default());
        let px = |f: &Frame, x: u32, y: u32| -> [u8; 4] { f.rgba[((y * f.width + x) * 4) as usize..][..4].try_into().unwrap() };
        comp.render(&p, 1.0, 640, 360, &mut pool, &mut text, &mut out);
        assert_eq!(px(&out, 10, 10), [200, 10, 10, 255], "background around the window");
        // window: x 120..520, y 24.5..335.5; bars 86 px; page area y 110.5..335.5
        assert_eq!(px(&out, 320, 30), [218, 222, 228, 255], "tab strip");
        assert_eq!(px(&out, 320, 160), [0, 0, 255, 255], "top half of the page");
        assert_eq!(px(&out, 320, 300), [0, 255, 0, 255], "bottom half of the page");
        assert_eq!(px(&out, 130, 200), [0, 0, 255, 255], "the page reaches the window's left edge");
        assert_eq!(px(&out, 110, 200), [200, 10, 10, 255], "and stops there");
        // scrolled by half the page: the green half is at the top of the page area now
        p.clip_mut(id).unwrap().browser.as_mut().unwrap().scroll = Animated::new(180.0);
        comp.render(&p, 1.0, 640, 360, &mut pool, &mut text, &mut out);
        assert_eq!(px(&out, 320, 160), [0, 255, 0, 255], "scroll moves the page up");
        assert_eq!(px(&out, 320, 30), [218, 222, 228, 255], "not the window");
        // Page only: no window, no background - the page fills the frame, and still scrolls
        let b = p.clip_mut(id).unwrap().browser.as_mut().unwrap();
        b.display = BrowserDisplay::Page;
        comp.render(&p, 1.0, 640, 360, &mut pool, &mut text, &mut out);
        assert_eq!(px(&out, 10, 10), [0, 255, 0, 255], "the scrolled page, edge to edge");
        assert_eq!(px(&out, 320, 30), [0, 255, 0, 255], "no bars");
        p.clip_mut(id).unwrap().browser.as_mut().unwrap().scroll = Animated::new(0.0);
        comp.render(&p, 1.0, 640, 360, &mut pool, &mut text, &mut out);
        assert_eq!((px(&out, 10, 10), px(&out, 320, 300)), ([0, 0, 255, 255], [0, 255, 0, 255]));
        // the load bar: a thin accent strip along the top as far as the load has got (the GPU's twin of
        // this is `gpu::browser_tests::the_page_only_load_bar_is_drawn_on_the_gpu`)
        p.clip_mut(id).unwrap().browser.as_mut().unwrap().load = Animated::new(0.5);
        comp.render(&p, 1.0, 640, 360, &mut pool, &mut text, &mut out);
        assert_eq!((px(&out, 100, 1), px(&out, 500, 1), px(&out, 100, 30)), ([40, 120, 230, 255], [0, 0, 255, 255], [0, 0, 255, 255]));
        p.clip_mut(id).unwrap().browser.as_mut().unwrap().load = Animated::new(0.0);
        // after the browser clip the frame is the plain page again
        comp.render(&p, 3.0, 640, 360, &mut pool, &mut text, &mut out);
        assert_eq!(px(&out, 10, 10), [0, 0, 255, 255]);
    }

    #[test]
    fn bitmaps_are_cached_and_redrawn_only_for_what_changed() {
        let mut r = BrowserRasterizer::default();
        let mut text = TextRasterizer::new();
        let mut c = clip();
        let b = c.browser.as_mut().unwrap();
        b.shadow = 20.0;
        b.url_typed.keys =
            vec![Keyframe { t: 0.0, v: 0.0, ease: Ease::Linear }, Keyframe { t: 1.0, v: 1.0, ease: Ease::Linear }];
        b.cursor = Some(Cursor { events: vec![CursorEvent { t: 2.0, kind: CursorEventKind::Click }], ..Default::default() });
        let mut none = |_: &str, _: u32, _: u32| None;
        let a = r.layers(b, 5.0, 0.5, 0.5, &mut text, &mut none);
        let again = r.layers(b, 6.0, 0.5, 0.5, &mut text, &mut none);
        let win = a.window.unwrap();
        assert!(Arc::ptr_eq(&win, again.window.as_ref().unwrap()), "a still window is one bitmap");
        assert!(Arc::ptr_eq(a.cursor.as_ref().unwrap(), again.cursor.as_ref().unwrap()));
        // sized for the window plus its shadow margin, at the raster scale
        assert_eq!(win.width, ((1600.0 + 2.0 * margin(b)) * 0.5f32).round() as u32);
        // the window's own corner is clear, its middle is the page colour, the bar is not
        let px = |f: &Frame, x: u32, y: u32| -> [u8; 4] { f.rgba[((y * f.width + x) * 4) as usize..][..4].try_into().unwrap() };
        let m = (margin(b) * 0.5) as u32;
        assert!(px(&win, m, m)[3] < 200, "rounded corner");
        assert_eq!(px(&win, win.width / 2, win.height / 2), [255, 255, 255, 255]);
        assert_eq!(px(&win, win.width / 2, m + 5), [218, 222, 228, 255], "tab strip");
        // gradient strip, top colour first
        let bg = a.bg.unwrap();
        assert_eq!((bg.width, bg.height), (1, 256));
        assert_eq!(&bg.rgba[..4], &[36, 40, 52, 255]);
        // typing the address is a new bitmap; a click ripple is a new sprite
        let typing = r.layers(b, 0.5, 0.5, 0.5, &mut text, &mut none);
        assert!(!Arc::ptr_eq(&win, typing.window.as_ref().unwrap()));
        let ripple = r.layers(b, 2.1, 0.5, 0.5, &mut text, &mut none);
        assert!(!Arc::ptr_eq(a.cursor.as_ref().unwrap(), ripple.cursor.as_ref().unwrap()));
        assert!(ripple.cursor.unwrap().pts != a.cursor.unwrap().pts, "every bitmap has its own stamp");
        // no cursor, one colour: nothing extra to draw
        b.cursor = None;
        b.bg2 = b.bg;
        let bare = r.layers(b, 5.0, 0.5, 0.5, &mut text, &mut none);
        assert!(bare.cursor.is_none() && bare.bg.is_none() && bare.keys.is_none());
    }
}
