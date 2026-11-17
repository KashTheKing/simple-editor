//! The bitmap side: replays the same `Op`s into an RGBA `Frame` (anti-aliased shapes from signed
//! distances, text through the engine's own `TextRasterizer`), so `render.frame` / `render.contact_sheet`
//! can carry the guide and `guides.preview` can hand back the device picture as a PNG. Deterministic: no
//! clock, no randomness.

use super::device::{self, Sheet};
use super::paint::GuideView;
use super::scene::Op;
use super::*;
use crate::engine::text::TextRasterizer;
use crate::media::Frame;
use crate::model::{Animated, TextStyle};
use eframe::egui::{Color32, Pos2, Rect};
use std::sync::Mutex;

pub struct Canvas {
    pub f: Frame,
    clip: (i32, i32, i32, i32),
}

fn straight(c: Color32) -> [u8; 4] {
    c.to_srgba_unmultiplied()
}

impl Canvas {
    /// Wrap `f` (any frame; converted to RGBA with opaque alpha when it came as BGRX).
    pub fn new(mut f: Frame) -> Canvas {
        if f.bgrx {
            for px in f.rgba.chunks_exact_mut(4) {
                px.swap(0, 2);
                px[3] = 255;
            }
            f.bgrx = false;
        }
        let clip = (0, 0, f.width as i32, f.height as i32);
        Canvas { f, clip }
    }
    fn blend(&mut self, x: i32, y: i32, c: [u8; 4], cov: f32) {
        let a = cov * c[3] as f32 / 255.0;
        if a <= 0.002 || x < self.clip.0 || y < self.clip.1 || x >= self.clip.2 || y >= self.clip.3 {
            return;
        }
        let i = (y as usize * self.f.width as usize + x as usize) * 4;
        let d = &mut self.f.rgba[i..i + 4];
        for k in 0..3 {
            d[k] = (d[k] as f32 + (c[k] as f32 - d[k] as f32) * a).round() as u8;
        }
        d[3] = d[3].max((a * 255.0) as u8);
    }
    /// Per-pixel coverage `0.5 - sd(x, y)` over `bb`.
    fn fill_sd(&mut self, bb: Rect, col: [u8; 4], sd: impl Fn(f32, f32) -> f32) {
        if col[3] == 0 {
            return;
        }
        let (x0, y0) = ((bb.min.x.floor() as i32 - 1).max(self.clip.0), (bb.min.y.floor() as i32 - 1).max(self.clip.1));
        let (x1, y1) = ((bb.max.x.ceil() as i32 + 1).min(self.clip.2), (bb.max.y.ceil() as i32 + 1).min(self.clip.3));
        for y in y0..y1 {
            for x in x0..x1 {
                let cov = (0.5 - sd(x as f32 + 0.5, y as f32 + 0.5)).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.blend(x, y, col, cov);
                }
            }
        }
    }
    pub fn run(&mut self, ops: &[Op], av: Option<&Frame>) {
        let full = (0, 0, self.f.width as i32, self.f.height as i32);
        for op in ops {
            match op {
                Op::Clip(r) => {
                    self.clip = match r {
                        Some(r) => (
                            (r.min.x.floor() as i32).max(full.0),
                            (r.min.y.floor() as i32).max(full.1),
                            (r.max.x.ceil() as i32).min(full.2),
                            (r.max.y.ceil() as i32).min(full.3),
                        ),
                        None => full,
                    }
                }
                Op::Rect { r, rad, top, fill, line, lc } => {
                    let (c, h) = (r.center(), r.size() * 0.5);
                    let rad = rad.min(h.x).min(h.y).max(0.0);
                    let top = *top;
                    let sd = move |x: f32, y: f32| {
                        let rr = if top && y > c.y { 0.0 } else { rad };
                        let (qx, qy) = ((x - c.x).abs() - (h.x - rr), (y - c.y).abs() - (h.y - rr));
                        (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - rr
                    };
                    self.fill_sd(*r, straight(*fill), sd);
                    if *line > 0.0 {
                        let w = *line * 0.5;
                        self.fill_sd(r.expand(w), straight(*lc), move |x, y| sd(x, y).abs() - w);
                    }
                }
                Op::Circle { c, r, fill, line, lc } => {
                    let (c, r) = (*c, *r);
                    let sd = move |x: f32, y: f32| ((x - c.x).powi(2) + (y - c.y).powi(2)).sqrt() - r;
                    let bb = Rect::from_center_size(c, egui::vec2(2.0 * r, 2.0 * r));
                    self.fill_sd(bb, straight(*fill), sd);
                    if *line > 0.0 {
                        let w = *line * 0.5;
                        self.fill_sd(bb.expand(w), straight(*lc), move |x, y| sd(x, y).abs() - w);
                    }
                }
                Op::Line { a, b, w, col } => {
                    let (a, b, hw) = (*a, *b, *w * 0.5);
                    let ab = b - a;
                    let l2 = ab.length_sq().max(1e-6);
                    let sd = move |x: f32, y: f32| {
                        let t = (((x - a.x) * ab.x + (y - a.y) * ab.y) / l2).clamp(0.0, 1.0);
                        let (px, py) = (a.x + ab.x * t, a.y + ab.y * t);
                        ((x - px).powi(2) + (y - py).powi(2)).sqrt() - hw
                    };
                    self.fill_sd(Rect::from_two_pos(a, b).expand(hw), straight(*col), sd);
                }
                Op::Poly { p, fill } => {
                    let col = straight(*fill);
                    let bb = p.iter().fold(Rect::NOTHING, |r, q| r.union(Rect::from_min_max(*q, *q)));
                    if bb.is_negative() {
                        continue;
                    }
                    let inside = |x: f32, y: f32| {
                        let mut c = false;
                        for i in 0..p.len() {
                            let (a, b) = (p[i], p[(i + 1) % p.len()]);
                            if (a.y > y) != (b.y > y) && x < (b.x - a.x) * (y - a.y) / (b.y - a.y) + a.x {
                                c = !c;
                            }
                        }
                        c
                    };
                    let (x0, y0) = ((bb.min.x.floor() as i32).max(self.clip.0), (bb.min.y.floor() as i32).max(self.clip.1));
                    let (x1, y1) = ((bb.max.x.ceil() as i32).min(self.clip.2), (bb.max.y.ceil() as i32).min(self.clip.3));
                    for y in y0..y1 {
                        for x in x0..x1 {
                            let n = [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)]
                                .iter()
                                .filter(|(dx, dy)| inside(x as f32 + dx, y as f32 + dy))
                                .count();
                            if n > 0 {
                                self.blend(x, y, col, n as f32 / 4.0);
                            }
                        }
                    }
                }
                Op::Grad { r, up, a } => {
                    let (y0, y1) = (r.min.y.floor() as i32, r.max.y.ceil() as i32);
                    for y in y0.max(self.clip.1)..y1.min(self.clip.3) {
                        let t = ((y as f32 + 0.5 - r.min.y) / r.height().max(1.0)).clamp(0.0, 1.0);
                        let t = if *up { t } else { 1.0 - t };
                        for x in (r.min.x.floor() as i32).max(self.clip.0)..(r.max.x.ceil() as i32).min(self.clip.2) {
                            self.blend(x, y, [0, 0, 0, *a], t);
                        }
                    }
                }
                Op::Avatar { c, r } => {
                    let (c, r) = (*c, *r);
                    let bb = Rect::from_center_size(c, egui::vec2(2.0 * r, 2.0 * r));
                    let (x0, y0) = ((bb.min.x.floor() as i32 - 1).max(self.clip.0), (bb.min.y.floor() as i32 - 1).max(self.clip.1));
                    let (x1, y1) = ((bb.max.x.ceil() as i32 + 1).min(self.clip.2), (bb.max.y.ceil() as i32 + 1).min(self.clip.3));
                    for y in y0..y1 {
                        for x in x0..x1 {
                            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                            let d = ((px - c.x).powi(2) + (py - c.y).powi(2)).sqrt();
                            let cov = (0.5 - (d - r)).clamp(0.0, 1.0);
                            if cov <= 0.0 {
                                continue;
                            }
                            let col = match av {
                                Some(f) => sample(f, (px - c.x) / (2.0 * r) + 0.5, (py - c.y) / (2.0 * r) + 0.5),
                                None => {
                                    let head = ((px - c.x).powi(2) + (py - (c.y - r * 0.22)).powi(2)).sqrt() < r * 0.32;
                                    let sh = ((px - c.x).powi(2) + (py - (c.y + r * 0.95)).powi(2)).sqrt() < r * 0.62;
                                    if head || sh { [190, 194, 205, 255] } else { [92, 96, 108, 255] }
                                }
                            };
                            self.blend(x, y, col, cov);
                        }
                    }
                }
                Op::Text { p, s, col, bold, t, al } => self.text(*p, *s, straight(*col), *bold, t, *al),
            }
        }
        self.clip = full;
    }
    fn text(&mut self, p: Pos2, s: f32, col: [u8; 4], bold: bool, t: &str, al: u8) {
        static R: Mutex<Option<TextRasterizer>> = Mutex::new(None);
        let style = TextStyle { text: t.to_string(), size: Animated::new(s as f64), bold, color: col, ..TextStyle::default() };
        let img = match R.lock() {
            Ok(mut g) => g.get_or_insert_with(TextRasterizer::new).render(&style, 1.0, 0.0),
            Err(_) => return,
        };
        let w = img.width as f32 - 2.0;
        let x0 = match al {
            1 => p.x - w / 2.0,
            2 => p.x - w,
            _ => p.x,
        } - 1.0;
        let (ox, oy) = (x0.round() as i32, (p.y - 1.0).round() as i32);
        for y in 0..img.height as i32 {
            for x in 0..img.width as i32 {
                let i = (y as usize * img.width as usize + x as usize) * 4;
                let px = &img.rgba[i..i + 4];
                if px[3] > 0 {
                    self.blend(ox + x, oy + y, [px[0], px[1], px[2], 255], px[3] as f32 / 255.0);
                }
            }
        }
    }
    /// `src` into `dst` (points), showing the `uv` part of it; bilinear, 2x2 averaged when shrinking.
    pub fn blit(&mut self, src: &Frame, dst: Rect, uv: Rect) {
        let bgr = src.bgrx;
        let (x0, y0) = ((dst.min.x.round() as i32).max(self.clip.0), (dst.min.y.round() as i32).max(self.clip.1));
        let (x1, y1) = ((dst.max.x.round() as i32).min(self.clip.2), (dst.max.y.round() as i32).min(self.clip.3));
        let shrink = src.width as f32 * uv.width() / dst.width().max(1.0) > 1.5;
        let taps: &[(f32, f32)] = if shrink { &[(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] } else { &[(0.5, 0.5)] };
        for y in y0..y1 {
            for x in x0..x1 {
                let mut acc = [0.0f32; 3];
                for (dx, dy) in taps {
                    let u = uv.min.x + uv.width() * ((x as f32 + dx - dst.min.x) / dst.width());
                    let v = uv.min.y + uv.height() * ((y as f32 + dy - dst.min.y) / dst.height());
                    let c = sample(src, u, v);
                    for k in 0..3 {
                        acc[k] += c[if bgr { 2 - k } else { k }] as f32;
                    }
                }
                let n = taps.len() as f32;
                self.blend(x, y, [(acc[0] / n) as u8, (acc[1] / n) as u8, (acc[2] / n) as u8, 255], 1.0);
            }
        }
    }
}

/// Bilinear sample of `f` at normalised (u, v).
fn sample(f: &Frame, u: f32, v: f32) -> [u8; 4] {
    let (w, h) = (f.width as i32, f.height as i32);
    let (fx, fy) = ((u * w as f32 - 0.5).clamp(0.0, (w - 1) as f32), (v * h as f32 - 0.5).clamp(0.0, (h - 1) as f32));
    let (x0, y0) = (fx.floor() as i32, fy.floor() as i32);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
    let px = |x: i32, y: i32, k: usize| f.rgba[(y as usize * w as usize + x as usize) * 4 + k] as f32;
    let mut out = [255u8; 4];
    for (k, o) in out.iter_mut().enumerate().take(4) {
        let top = px(x0, y0, k) + (px(x1, y0, k) - px(x0, y0, k)) * tx;
        let bot = px(x0, y1, k) + (px(x1, y1, k) - px(x0, y1, k)) * tx;
        *o = (top + (bot - top) * ty) as u8;
    }
    out
}

/// `frame` with the guide (safe zone or UI preview) drawn over it.
pub fn render_overlay(frame: Frame, g: &GuideView) -> Frame {
    let (w, h) = (frame.width as f32, frame.height as f32);
    let ops = g.overlay_ops(Rect::from_min_size(Pos2::ZERO, egui::vec2(w, h)));
    let av = super::profile::avatar(&g.profile, true);
    let mut c = Canvas::new(frame);
    c.run(&ops, av.as_deref());
    c.f
}

/// The device picture `width` px wide (height follows the device); `frame` = the video (None = a stand-in).
pub fn render_device(frame: Option<&Frame>, g: &GuideView, open: Option<Sheet>, width: u32) -> Frame {
    let dev = g.dev();
    let w = width.clamp(160, 2400) as f32;
    let (bz, bar) = if dev.k == "desktop" { (0.0, 40.0) } else { (if dev.k == "tablet" { 12.0 } else { 7.0 }, 0.0) };
    let m = 0.03 * w;
    let k = (w - 2.0 * m) / (dev.w + 2.0 * bz);
    let h = ((dev.h + 2.0 * bz + bar) * k + 2.0 * m).round();
    let area = Rect::from_min_size(Pos2::ZERO, egui::vec2(w, h));
    let scene = device::build(&g.device_view(open, if open.is_some() { 1.0 } else { 0.0 }), area);
    let mut c = Canvas::new(Frame::new(w as u32, h as u32));
    let av = super::profile::avatar(&g.profile, true);
    c.run(&scene.under, av.as_deref());
    match frame {
        Some(f) => c.blit(f, scene.video, scene.uv),
        None => c.run(&[Op::Rect { r: scene.video, rad: 0.0, top: false, fill: Color32::from_rgb(38, 64, 98), line: 0.0, lc: Color32::TRANSPARENT }], None),
    }
    c.run(&scene.over, av.as_deref());
    c.f
}

/// The burnt-in guide for an export (None when no guide is chosen): the overlay of `s`'s guide on each
/// frame, playing, its bar and time those of the exported video - `range` (In/Out) when it is one, else
/// the timeline. A Device preview burns as its UI overlay (an export keeps the project's frame size).
pub fn export_overlay(s: &crate::settings::Settings, project: &crate::model::Project, range: Option<(f64, f64)>) -> Option<crate::engine::export::FrameOverlay> {
    let mut s = s.clone();
    if s.guide.is_none() {
        return None;
    }
    if s.guide_mode == super::Mode::Device.id() {
        s.guide_mode = super::Mode::Ui.id().into();
    }
    let (a, b) = range.unwrap_or((0.0, project.duration()));
    let title = project.name.clone();
    let aspect = project.width.max(1) as f32 / project.height.max(1) as f32;
    Some(crate::engine::export::FrameOverlay(std::sync::Arc::new(move |f: Frame, t: f64| match GuideView::new(&s, &title, aspect, t - a, b - a) {
        Some(mut g) => {
            g.playing = true;
            g.export = true;
            render_overlay(f, &g)
        }
        None => f,
    })))
}
