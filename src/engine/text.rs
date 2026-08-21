//! Text rasterizer for Text clips: system fonts (fontdb) + ab_glyph. Produces a tight, straight-alpha
//! RGBA image with fill, outline (dilated coverage), shadow (offset + box blur) and optional background box.
//! Output is cached by (style.cache_key(), scale bits). `scale` = canvas px per project px, so a 72 px
//! style at a 960-wide preview of a 1920 project renders at 36 px.

use crate::media::Frame;
use crate::model::TextStyle;
use ab_glyph::{point, Font, FontArc, FontVec, Glyph, GlyphId, OutlinedGlyph, PxScale, ScaleFont};
use fontdb::{Database, Family, Query, Style, Weight, ID};
use std::collections::HashMap;
use std::sync::Arc;

/// Largest rendered text image side — keeps silly sizes from allocating gigabytes.
const MAX_SIDE: u32 = 8192;

pub struct TextRasterizer {
    cache: HashMap<(u64, u32), Arc<Frame>>,
    families: Vec<String>,
    loaded: bool,
    db: Database,
    fonts: HashMap<ID, Option<FontArc>>,
}

impl Default for TextRasterizer {
    fn default() -> Self {
        Self::new()
    }
}

impl TextRasterizer {
    /// Cheap; fonts load lazily (or via `load_system_fonts` from a background thread).
    pub fn new() -> Self {
        Self { cache: HashMap::new(), families: Vec::new(), loaded: false, db: Database::new(), fonts: HashMap::new() }
    }
    /// Scan system fonts (≈100 ms). Idempotent.
    pub fn load_system_fonts(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        self.db.load_system_fonts();
        let mut fams: Vec<String> =
            self.db.faces().filter_map(|f| f.families.first().map(|(n, _)| n.clone())).collect();
        fams.sort();
        fams.dedup();
        self.families = fams;
    }
    pub fn is_loaded(&self) -> bool {
        self.loaded
    }
    /// Sorted unique family names (empty until loaded).
    pub fn families(&self) -> &[String] {
        &self.families
    }
    /// Render `style` at `scale`. Never fails: unknown fonts fall back to any sans font; empty text
    /// yields a 1×1 transparent frame.
    pub fn render(&mut self, style: &TextStyle, scale: f32) -> Arc<Frame> {
        let key = (style.cache_key(), scale.to_bits());
        if let Some(f) = self.cache.get(&key) {
            return f.clone();
        }
        self.load_system_fonts();
        let frame = Arc::new(self.rasterize(style, scale).unwrap_or_else(|| Frame::new(1, 1)));
        if self.cache.len() >= 64 {
            // ponytail: drop-all cache — LRU if text-heavy projects thrash
            self.cache.clear();
        }
        self.cache.insert(key, frame.clone());
        frame
    }

    fn font_for(&mut self, style: &TextStyle) -> Option<FontArc> {
        let q = Query {
            families: &[Family::Name(&style.font), Family::SansSerif],
            weight: if style.bold { Weight::BOLD } else { Weight::NORMAL },
            style: if style.italic { Style::Italic } else { Style::Normal },
            stretch: Default::default(),
        };
        let id = self.db.query(&q).or_else(|| self.db.faces().next().map(|f| f.id))?;
        let db = &self.db;
        self.fonts
            .entry(id)
            .or_insert_with(|| {
                db.with_face_data(id, |data, idx| FontVec::try_from_vec_and_index(data.to_vec(), idx).ok())
                    .flatten()
                    .map(FontArc::new)
            })
            .clone()
    }

    fn rasterize(&mut self, style: &TextStyle, scale: f32) -> Option<Frame> {
        let px = style.size * scale;
        if style.text.trim().is_empty() || !(px >= 1.0) || !px.is_finite() {
            return None;
        }
        let font = self.font_for(style)?;
        // `size` is the em size in px; ab_glyph's PxScale is the ascent-descent height.
        let upm = font.units_per_em().unwrap_or(font.height_unscaled());
        let sf = font.as_scaled(PxScale::from(px * font.height_unscaled() / upm));
        let ps = sf.scale();
        let ls = style.letter_spacing * scale;
        let lh = (sf.ascent() - sf.descent() + sf.line_gap()) * style.line_spacing.max(0.1);

        // ---- layout: glyph positions per line, then align ----
        let mut glyphs: Vec<Glyph> = Vec::new();
        let mut lines: Vec<(usize, usize, f32)> = Vec::new();
        for line in style.text.split('\n') {
            let start = glyphs.len();
            let mut x = 0.0f32;
            let mut prev: Option<GlyphId> = None;
            for c in line.chars().filter(|c| *c != '\r') {
                let id = sf.glyph_id(c);
                if let Some(p) = prev {
                    x += sf.kern(p, id);
                }
                glyphs.push(id.with_scale_and_position(ps, point(x, 0.0)));
                x += sf.h_advance(id) + ls;
                prev = Some(id);
            }
            if prev.is_some() {
                x -= ls;
            }
            lines.push((start, glyphs.len(), x.max(0.0)));
        }
        let block_w = lines.iter().map(|l| l.2).fold(0.0, f32::max);
        let block_h = (lines.len() - 1) as f32 * lh + sf.ascent() - sf.descent();
        let align = match style.align {
            0 => 0.0,
            2 => 1.0,
            _ => 0.5,
        };
        for (i, (s, e, lw)) in lines.iter().enumerate() {
            let ox = (block_w - lw) * align;
            let oy = sf.ascent() + i as f32 * lh;
            for g in &mut glyphs[*s..*e] {
                g.position.x += ox;
                g.position.y += oy;
            }
        }
        let outlined: Vec<OutlinedGlyph> = glyphs.iter().filter_map(|g| font.outline_glyph(g.clone())).collect();

        // ---- image size: layout block ∪ glyph pixel bounds, plus a uniform margin for effects ----
        let (mut bx0, mut by0, mut bx1, mut by1) = (0.0f32, 0.0f32, block_w.ceil(), block_h.ceil());
        for og in &outlined {
            let b = og.px_bounds();
            bx0 = bx0.min(b.min.x);
            by0 = by0.min(b.min.y);
            bx1 = bx1.max(b.max.x);
            by1 = by1.max(b.max.y);
        }
        let r = style.outline_width.max(0.0) * scale;
        let r = if r < 0.5 { 0.0 } else { r };
        let (shx, shy, blur) = if style.shadow {
            let s = |v: f32| (v * scale).round();
            (s(style.shadow_x), s(style.shadow_y), s(style.shadow_blur.max(0.0)))
        } else {
            (0.0, 0.0, 0.0)
        };
        let has_box = style.box_color[3] > 0;
        let pad = if has_box { style.box_padding.max(0.0) * scale } else { 0.0 };
        let m = (r + shx.abs().max(shy.abs()) + blur * 2.0).max(pad).ceil() as i32 + 1;
        // ponytail: hard cap on the image size — bigger text just gets clipped
        let w = ((bx1 - bx0).ceil() as i32 + 2 * m).clamp(1, MAX_SIDE as i32) as u32;
        let h = ((by1 - by0).ceil() as i32 + 2 * m).clamp(1, MAX_SIDE as i32) as u32;
        let (ox, oy) = (m as f32 - bx0, m as f32 - by0);
        let (wu, n) = (w as usize, (w * h) as usize);

        // ---- masks ----
        let mut fill = vec![0u8; n];
        for og in &outlined {
            let b = og.px_bounds();
            let (gx, gy) = ((b.min.x + ox) as i32, (b.min.y + oy) as i32);
            og.draw(|x, y, c| {
                let (px, py) = (gx + x as i32, gy + y as i32);
                if px >= 0 && py >= 0 && (px as u32) < w && (py as u32) < h {
                    let i = py as usize * wu + px as usize;
                    fill[i] = fill[i].max((c.min(1.0) * 255.0 + 0.5) as u8);
                }
            });
        }
        let outline = if r > 0.0 { dilate(&fill, w, h, r) } else { Vec::new() };
        let shadow = if style.shadow && style.shadow_color[3] > 0 {
            let mut s = vec![0u8; n];
            let (dx, dy) = (shx as i32, shy as i32);
            for y in 0..h as i32 {
                let sy = y - dy;
                if sy < 0 || sy >= h as i32 {
                    continue;
                }
                for x in 0..w as i32 {
                    let sx = x - dx;
                    if sx < 0 || sx >= w as i32 {
                        continue;
                    }
                    let j = sy as usize * wu + sx as usize;
                    let v = if outline.is_empty() { fill[j] } else { fill[j].max(outline[j]) };
                    s[y as usize * wu + x as usize] = v;
                }
            }
            box_blur(&mut s, w, h, blur as usize);
            s
        } else {
            Vec::new()
        };

        // ---- compose: box, shadow, outline, fill ----
        let mut out = Frame::new(w, h);
        if has_box {
            let x0 = ((ox - pad).floor().max(0.0) as u32).min(w);
            let y0 = ((oy - pad).floor().max(0.0) as u32).min(h);
            let x1 = ((block_w + ox + pad).ceil().max(0.0) as u32).min(w);
            let y1 = ((block_h + oy + pad).ceil().max(0.0) as u32).min(h);
            for y in y0..y1 {
                for x in x0..x1 {
                    over(&mut out.rgba[(y as usize * wu + x as usize) * 4..][..4], style.box_color, 255);
                }
            }
        }
        for (mask, color) in [(&shadow, style.shadow_color), (&outline, style.outline_color), (&fill, style.color)] {
            if mask.is_empty() || color[3] == 0 {
                continue;
            }
            for (px, &m) in out.rgba.chunks_exact_mut(4).zip(mask.iter()) {
                if m > 0 {
                    over(px, color, m);
                }
            }
        }
        Some(out)
    }
}

/// Straight-alpha "over": `c` with coverage `cov` (0..255) onto the straight-alpha pixel `d`.
#[inline]
fn over(d: &mut [u8], c: [u8; 4], cov: u8) {
    let sa = c[3] as f32 / 255.0 * cov as f32 / 255.0;
    if sa <= 0.0 {
        return;
    }
    let da = d[3] as f32 / 255.0 * (1.0 - sa);
    let oa = sa + da;
    for i in 0..3 {
        d[i] = ((c[i] as f32 * sa + d[i] as f32 * da) / oa + 0.5) as u8;
    }
    d[3] = (oa * 255.0 + 0.5) as u8;
}

/// Disc dilation of a coverage mask by radius `r` (antialiased rim).
fn dilate(src: &[u8], w: u32, h: u32, r: f32) -> Vec<u8> {
    // ponytail: O(filled px × r²) splat — swap for a distance transform if big outlines hitch
    let rr = r.ceil() as i32;
    let mut offs: Vec<(i32, i32, u32)> = Vec::new();
    for dy in -rr..=rr {
        for dx in -rr..=rr {
            let d = ((dx * dx + dy * dy) as f32).sqrt();
            let wgt = (r + 0.5 - d).clamp(0.0, 1.0);
            if wgt > 0.0 {
                offs.push((dx, dy, (wgt * 255.0 + 0.5) as u32));
            }
        }
    }
    let (wi, hi) = (w as i32, h as i32);
    let mut out = vec![0u8; src.len()];
    for y in 0..hi {
        for x in 0..wi {
            let v = src[(y * wi + x) as usize] as u32;
            if v == 0 {
                continue;
            }
            for &(dx, dy, wgt) in &offs {
                let (px, py) = (x + dx, y + dy);
                if px >= 0 && py >= 0 && px < wi && py < hi {
                    let i = (py * wi + px) as usize;
                    let nv = ((v * wgt + 127) / 255) as u8;
                    if nv > out[i] {
                        out[i] = nv;
                    }
                }
            }
        }
    }
    out
}

/// Two iterations of a separable box blur with radius `r` (≈ Gaussian of radius 2r).
fn box_blur(buf: &mut [u8], w: u32, h: u32, r: usize) {
    if r == 0 || buf.is_empty() {
        return;
    }
    let (w, h) = (w as usize, h as usize);
    let mut tmp = vec![0u8; buf.len()];
    for _ in 0..2 {
        blur_lines(buf, &mut tmp, h, w, w, 1, r);
        blur_lines(&tmp, buf, w, h, 1, w, r);
    }
}

/// 1-D box blur of `lines` lines of `len` elements; element j of line i is at i*lstride + j*estride.
fn blur_lines(src: &[u8], dst: &mut [u8], lines: usize, len: usize, lstride: usize, estride: usize, r: usize) {
    let norm = (2 * r + 1) as u32;
    for l in 0..lines {
        let base = l * lstride;
        let at = |j: usize| base + j * estride;
        let mut sum: u32 = (0..=r.min(len - 1)).map(|j| src[at(j)] as u32).sum();
        for j in 0..len {
            dst[at(j)] = ((sum + norm / 2) / norm) as u8;
            if j + r + 1 < len {
                sum += src[at(j + r + 1)] as u32;
            }
            if j >= r {
                sum -= src[at(j - r)] as u32;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha_sum(f: &Frame) -> u64 {
        f.rgba.chunks_exact(4).map(|p| p[3] as u64).sum()
    }

    #[test]
    fn renders_default_style_and_caches() {
        let mut tr = TextRasterizer::new();
        let style = TextStyle::default();
        let a = tr.render(&style, 0.5);
        if tr.families().is_empty() {
            eprintln!("no system fonts — skipping");
            assert_eq!((a.width, a.height), (1, 1));
            return;
        }
        assert!(a.width > 4 && a.height > 4, "{}x{}", a.width, a.height);
        assert!(alpha_sum(&a) > 0);
        // white fill: every covered pixel is white
        assert!(a.rgba.chunks_exact(4).filter(|p| p[3] > 0).all(|p| p[0] == 255 && p[1] == 255 && p[2] == 255));
        let b = tr.render(&style, 0.5);
        assert!(Arc::ptr_eq(&a, &b), "cache hit expected");
        let c = tr.render(&style, 1.0);
        assert!(!Arc::ptr_eq(&a, &c));
        assert!(c.width > a.width);

        // unknown font falls back, never panics
        let mut s2 = style.clone();
        s2.font = "No Such Font 123".into();
        s2.bold = true;
        s2.italic = true;
        assert!(alpha_sum(&tr.render(&s2, 0.5)) > 0);

        // outline + shadow + box make the image bigger and add non-white pixels
        let mut s3 = style.clone();
        s3.outline_width = 4.0;
        s3.shadow = true;
        s3.box_color = [0, 0, 255, 255];
        let d = tr.render(&s3, 0.5);
        assert!(d.width > a.width && d.height > a.height);
        assert!(d.rgba.chunks_exact(4).any(|p| p[3] > 0 && p[2] == 255 && p[0] == 0));
        assert!(d.rgba.chunks_exact(4).any(|p| p[3] > 0 && p[0] == 0 && p[2] == 0));

        // multi-line, left aligned, is taller
        let mut s4 = style.clone();
        s4.text = "One\nTwo\nThree".into();
        s4.align = 0;
        let e = tr.render(&s4, 0.5);
        assert!(e.height > a.height * 2);
    }

    #[test]
    fn empty_text_is_1x1() {
        let mut tr = TextRasterizer::new();
        let mut style = TextStyle::default();
        style.text = "  \n ".into();
        let f = tr.render(&style, 1.0);
        assert_eq!((f.width, f.height, f.rgba[3]), (1, 1, 0));
        style.text = "x".into();
        assert_eq!(tr.render(&style, 0.0).width, 1);
    }

    #[test]
    fn blur_and_dilate_basics() {
        let mut m = vec![0u8; 25];
        m[12] = 255;
        let d = dilate(&m, 5, 5, 1.5);
        assert_eq!(d[12], 255);
        assert_eq!(d[11], 255);
        assert_eq!(d[7], 255);
        assert_eq!(d[0], 0);
        assert_eq!(d[10], 0);
        // two passes of radius 1 spread 2 px: centre dims, neighbours light up, corners (3 px away) stay 0
        let mut b = vec![0u8; 49];
        b[24] = 255;
        box_blur(&mut b, 7, 7, 1);
        assert!(b[24] < 255 && b[24] > 0);
        assert!(b[23] > 0 && b[16] > 0 && b[10] > 0);
        assert_eq!(b[0], 0);
        assert_eq!(b[3], 0);
    }
}
