//! Text rasterizer for Text clips: system fonts (fontdb) + ab_glyph. Produces a tight, straight-alpha
//! RGBA image with fill, outline (dilated coverage), shadow (offset + box blur) and optional background box.
//! Output is cached by (style.cache_key(), scale bits, reveal bucket). `scale` = canvas px per project px,
//! so a 72 px style at a 960-wide preview of a 1920 project renders at 36 px.
//!
//! ---- ws:text-titles ----
//! `render`/`rasterize` take a clip-local `t: f64` (seconds) so `TextStyle.size`/`letter_spacing`/
//! `outline_width` (all `Animated` since this workstream) and the reveal (typewriter) / wave (per-glyph
//! bob) effects can sample it, mirroring `ShapeRasterizer::render(style, scale, t)` (engine/shapes.rs).
//! `reveal_bucket` keeps the cache from growing per-frame the way `engine::shapes::reveal_bucket` does.

use crate::media::Frame;
use crate::model::{TextStyle, BOX_BAR, BOX_LINES};
use ab_glyph::{point, Font, FontArc, FontVec, Glyph, GlyphId, OutlinedGlyph, PxScale, ScaleFont};
use fontdb::{Database, Family, Query, Style, Weight, ID};
use std::collections::HashMap;
use std::sync::Arc;

/// Fonts shipped inside the exe (SIL Open Font License 1.1, see `assets/fonts/OFL-*.txt`): static
/// Bold Latin subsets made from the upstream variable fonts. TikTok Sans for the TikTok text styles,
/// Inter for the Snapchat ones, Comic Neue for the Classic 2000s titles when Comic Sans MS is not
/// installed (`engine::retro::FONT`; Comic Sans itself is never bundled). Registered before the system
/// fonts, so they work on any PC.
pub const BUNDLED_FONTS: [&[u8]; 3] = [
    include_bytes!("../../assets/fonts/TikTokSans-Bold.ttf"),
    include_bytes!("../../assets/fonts/Inter-Bold.ttf"),
    include_bytes!("../../assets/fonts/ComicNeue-Bold.ttf"),
];

/// Largest rendered text image side - keeps silly sizes from allocating gigabytes.
const MAX_SIDE: u32 = 8192;
/// Largest side a zoomed text bitmap (`render_zoomed`) may reach.
const ZOOM_MAX_SIDE: f32 = 8192.0;
/// Most pixels a zoomed text bitmap may hold (64 MB of RGBA), as `shapes::MAX_PIXELS`.
const ZOOM_MAX_PIXELS: f32 = 16.0e6;
/// Reveal/wave time quantisation for the cache (mirrors engine::shapes::REVEAL_HZ).
const REVEAL_HZ: f64 = 30.0;
/// Per-glyph wave oscillation rate (full cycles per second) and phase step between neighbouring glyphs.
const WAVE_HZ: f64 = 2.0;
const WAVE_PHASE: f64 = 0.5;

/// Cache-key time quantizer, mirroring `engine::shapes::reveal_bucket`. Returns a constant `0` when
/// neither `reveal` nor `wave` can change the rendered pixels over time (the common, non-animated case -
/// the cache behaves exactly as it did before this field existed), otherwise a bounded Hz-quantized
/// bucket. `wave` is time-dependent even with a plain (non-keyframed) nonzero `value` - the oscillation
/// itself is `sin(t * WAVE_HZ + …)` in `rasterize`, not something `Animated::at` captures - so `wave` is
/// "static" only when its value is exactly 0, unlike `reveal` (whose non-animated value is a genuine
/// constant across every `t`).
fn reveal_bucket(style: &TextStyle, t: f64) -> u32 {
    let wave_varies = style.wave.is_animated() || style.wave.value != 0.0;
    if !style.reveal.is_animated() && !wave_varies {
        return 0;
    }
    let mut r = if t.is_finite() { t.max(0.0) } else { 0.0 };
    // ws:social-text: a keyframed reveal alone (a typewriter) only changes between its first and
    // last key - outside them the picture is fixed, so it needs no re-render per frame there
    if !wave_varies && style.reveal.link.is_none() && style.reveal.baked.is_empty() {
        if let (Some(a), Some(b)) = (style.reveal.keys.first(), style.reveal.keys.last()) {
            r = r.clamp(a.t.max(0.0), b.t.max(0.0));
        }
    }
    (r * REVEAL_HZ).min(u32::MAX as f64) as u32
}

pub struct TextRasterizer {
    cache: HashMap<(u64, u32, u32, u32), Arc<Frame>>,
    families: Vec<String>,
    loaded: bool,
    db: Database,
    fonts: HashMap<ID, Option<FontArc>>,
    /// User font file paths already loaded (or attempted), so repeat calls are cheap.
    user_fonts: Vec<String>,
}

impl Default for TextRasterizer {
    fn default() -> Self {
        Self::new()
    }
}

impl TextRasterizer {
    /// Cheap; fonts load lazily (or via `load_system_fonts` from a background thread).
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
            families: Vec::new(),
            loaded: false,
            db: Database::new(),
            fonts: HashMap::new(),
            user_fonts: Vec::new(),
        }
    }
    /// Scan system fonts (≈100 ms). Idempotent.
    pub fn load_system_fonts(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        for f in BUNDLED_FONTS {
            self.db.load_font_data(f.to_vec());
        }
        self.db.load_system_fonts();
        self.refresh_families();
    }
    /// Load user font files (`Settings.user_fonts`) into the database. Idempotent: paths already
    /// loaded (or that failed) are skipped on repeat calls.
    pub fn load_user_fonts(&mut self, paths: &[String]) {
        let mut added = false;
        for p in paths {
            if self.user_fonts.iter().any(|q| q == p) {
                continue;
            }
            // ponytail: failed paths are remembered and never retried - restart to retry
            self.user_fonts.push(p.clone());
            if self.db.load_font_file(p).is_ok() {
                added = true;
            }
        }
        if added {
            self.refresh_families();
            // styles that fell back to another font before this one existed must re-render
            self.cache.clear();
        }
    }
    fn refresh_families(&mut self) {
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
    /// Render `style` at `scale` for clip-local time `t` (seconds). Never fails: unknown fonts fall
    /// back to any sans font; empty text yields a 1×1 transparent frame.
    pub fn render(&mut self, style: &TextStyle, scale: f32, t: f64) -> Arc<Frame> {
        self.render_in(style, scale, t, 0.0)
    }
    /// `render` for a layer on a frame `frame_w` px wide (in the bitmap's own px, before the clip's
    /// scale): a caption bar (`TextStyle::is_bar`) draws its band exactly that wide. Other styles
    /// ignore it and share one cache entry whatever the frame.
    pub fn render_in(&mut self, style: &TextStyle, scale: f32, t: f64, frame_w: f32) -> Arc<Frame> {
        let bar_w =
            if style.is_bar() && frame_w.is_finite() { frame_w.clamp(0.0, MAX_SIDE as f32).round() } else { 0.0 };
        let key = (style.cache_key(), scale.to_bits(), reveal_bucket(style, t), bar_w.to_bits());
        if let Some(f) = self.cache.get(&key) {
            return f.clone();
        }
        self.load_system_fonts();
        let frame = Arc::new(self.rasterize(style, scale, t, bar_w).unwrap_or_else(|| Frame::new(1, 1)));
        if self.cache.len() >= 64 {
            // ponytail: drop-all cache - LRU if text-heavy projects thrash
            self.cache.clear();
        }
        self.cache.insert(key, frame.clone());
        frame
    }

    /// `render_in` drawn `zoom` times bigger, for a clip shown at that magnification
    /// (`compose::raster_zoom`, a half-octave step), so a scaled-up title is rasterised at its
    /// on-screen size instead of stretched. The zoom steps down by half octaves while the bitmap would
    /// pass `ZOOM_MAX_SIDE` px a side or `ZOOM_MAX_PIXELS` (judged from the zoom-1 bitmap, which stays
    /// cached beside it). `frame_w` is at zoom 1. Returns the bitmap and the zoom it was drawn at: its
    /// on-screen size is its own size divided by that zoom.
    pub fn render_zoomed(&mut self, style: &TextStyle, scale: f32, zoom: f32, t: f64, frame_w: f32) -> (Arc<Frame>, f32) {
        let base = self.render_in(style, scale, t, frame_w);
        if !(zoom > 1.0 && zoom.is_finite()) || (base.width <= 1 && base.height <= 1) {
            return (base, 1.0);
        }
        let (bw, bh) = (base.width.max(1) as f32, base.height.max(1) as f32);
        // margins and outlines grow with the scale too, so the bitmap is about `z` times the base
        let fit = (ZOOM_MAX_SIDE / bw.max(bh)).min((ZOOM_MAX_PIXELS / (bw * bh)).sqrt()) * 0.97;
        let mut z = crate::engine::compose::half_octave(zoom);
        while z > 1.0 && z > fit {
            z = crate::engine::compose::half_octave(z / std::f32::consts::SQRT_2);
        }
        if z <= 1.0 {
            return (base, 1.0);
        }
        (self.render_in(style, scale * z, t, frame_w * z), z)
    }

    /// Resolve a font by family name + weight/style (used for the base style and for any span that
    /// overrides font/bold/italic). Unknown family falls back to the first loaded face, same as before.
    fn font_for(&mut self, family: &str, bold: bool, italic: bool) -> Option<FontArc> {
        let id = self.face_for(family, bold, italic)?;
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

    /// The face a family list resolves to: "Pricedown, Impact" is the first one installed (a font a
    /// template may not ship, then a stand-in), then any sans font, then the first face loaded.
    fn face_for(&self, family: &str, bold: bool, italic: bool) -> Option<ID> {
        let mut families: Vec<Family> = family.split(',').map(str::trim).filter(|f| !f.is_empty()).map(Family::Name).collect();
        families.push(Family::SansSerif);
        let q = Query {
            families: &families,
            weight: if bold { Weight::BOLD } else { Weight::NORMAL },
            style: if italic { Style::Italic } else { Style::Normal },
            stretch: Default::default(),
        };
        self.db.query(&q).or_else(|| self.db.faces().next().map(|f| f.id))
    }

    /// The family name a style's font list draws with on this PC (what the fallback picked).
    pub fn resolved_family(&self, family: &str, bold: bool) -> Option<String> {
        let id = self.face_for(family, bold, false)?;
        self.db.face(id)?.families.first().map(|(n, _)| n.clone())
    }

    fn rasterize(&mut self, style: &TextStyle, scale: f32, t: f64, bar_w: f32) -> Option<Frame> {
        // ---- ws:text-titles: sample the now-Animated fields once at this clip-local `t` ----
        let sz = style.size.at(t) as f32;
        let ls = style.letter_spacing.at(t) as f32;
        let ow = style.outline_width.at(t) as f32;
        let px = sz * scale;
        if style.text.trim().is_empty() || !(px >= 1.0) || !px.is_finite() {
            return None;
        }
        let font = self.font_for(&style.font, style.bold, style.italic)?;
        // `size` is the em size in px; ab_glyph's PxScale is the ascent-descent height.
        let upm = font.units_per_em().unwrap_or(font.height_unscaled());
        let sf = font.as_scaled(PxScale::from(px * font.height_unscaled() / upm));
        let ps = sf.scale();
        let lh = (sf.ascent() - sf.descent() + sf.line_gap()) * style.line_spacing.max(0.1);
        let n_chars = style.text.chars().count();
        // typewriter reveal: only glyphs with char_idx < cutoff get outlined below; layout above still
        // positions the FULL string so already-revealed glyphs never shift as more of it appears.
        let cutoff = (n_chars as f64 * style.reveal.at(t).clamp(0.0, 1.0)).round() as usize;
        // per-glyph wave/bob: 0 is the overwhelmingly common case (no oscillation, no per-frame cost).
        let wv = style.wave.at(t) as f32;

        // Spans that change glyph SHAPE (font/size/bold/italic), resolved once per span rather than
        // per char - a text clip has a handful of spans, not hundreds.
        // ponytail: layout below still advances the cursor using the BASE font's metrics for every
        // char, even one covered by a shape-changing span - mixing genuinely different advance widths
        // into the existing single-pass line layout (kerning, line height, alignment) is a real rewrite
        // of this function, not a proportionate one for how far the mouse-selects-a-word feature reaches.
        // A span's glyph is outlined with its own font/size and drawn at the position the base layout
        // already gave that character, so it reads in the right font/weight/size but a much bigger
        // override can visually overlap its neighbours instead of pushing them aside. Upgrade path: give
        // this pass a per-run advance/kerning loop keyed by resolved style instead of the single `sf`.
        struct SpanShape {
            font: FontArc,
            ps: PxScale,
        }
        let mut span_shapes: Vec<Option<SpanShape>> = Vec::with_capacity(style.spans.len());
        for s in &style.spans {
            if s.font.is_none() && s.size.is_none() && s.bold.is_none() && s.italic.is_none() {
                span_shapes.push(None);
                continue;
            }
            let shape = (|| {
                let fam = s.font.as_deref().unwrap_or(&style.font);
                let bold = s.bold.unwrap_or(style.bold);
                let italic = s.italic.unwrap_or(style.italic);
                let size = s.size.unwrap_or(sz).max(0.0);
                let opx = (size * scale).max(1.0);
                let f = self.font_for(fam, bold, italic)?;
                let upm = f.units_per_em().unwrap_or(f.height_unscaled());
                let ps = PxScale::from(opx * f.height_unscaled() / upm);
                Some(SpanShape { font: f, ps })
            })();
            span_shapes.push(shape);
        }
        let span_at = |char_idx: usize| -> Option<usize> {
            let mut found = None;
            for (i, s) in style.spans.iter().enumerate() {
                let end = s.end.min(n_chars);
                if s.start < end && char_idx >= s.start && char_idx < end {
                    found = Some(i);
                }
            }
            found
        };

        // ---- layout: glyph positions (base font/size/kerning throughout), then align ----
        let mut glyphs: Vec<Glyph> = Vec::new();
        let mut glyph_chars: Vec<char> = Vec::new();
        let mut glyph_char_idx: Vec<usize> = Vec::new();
        let mut lines: Vec<(usize, usize, f32)> = Vec::new();
        {
            let mut x = 0.0f32;
            let mut prev: Option<GlyphId> = None;
            let mut last_ls = 0.0f32;
            let mut line_start = 0usize;
            for (char_idx, c) in style.text.chars().enumerate() {
                if c == '\r' {
                    continue;
                }
                if c == '\n' {
                    if prev.is_some() {
                        x -= last_ls;
                    }
                    lines.push((line_start, glyphs.len(), x.max(0.0)));
                    line_start = glyphs.len();
                    x = 0.0;
                    prev = None;
                    continue;
                }
                let id = sf.glyph_id(c);
                if let Some(p) = prev {
                    x += sf.kern(p, id);
                }
                glyphs.push(id.with_scale_and_position(ps, point(x, 0.0)));
                glyph_chars.push(c);
                glyph_char_idx.push(char_idx);
                let cls = span_at(char_idx).and_then(|i| style.spans[i].letter_spacing).unwrap_or(ls) * scale;
                x += sf.h_advance(id) + cls;
                last_ls = cls;
                prev = Some(id);
            }
            if prev.is_some() {
                x -= last_ls;
            }
            lines.push((line_start, glyphs.len(), x.max(0.0)));
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
        // per-glyph wave/bob, after line alignment so it offsets the FINAL position.
        if wv != 0.0 {
            for (gi, g) in glyphs.iter_mut().enumerate() {
                let ci = glyph_char_idx[gi];
                g.position.y += wv * ((t * WAVE_HZ + ci as f64 * WAVE_PHASE).sin() as f32);
            }
        }
        // ---- outline glyphs: base shape, unless a span overrides font/size/bold/italic ----
        let mut outlined: Vec<OutlinedGlyph> = Vec::with_capacity(glyphs.len());
        let mut outlined_char: Vec<usize> = Vec::with_capacity(glyphs.len());
        for (gi, g) in glyphs.iter().enumerate() {
            let ci = glyph_char_idx[gi];
            // typewriter reveal: characters at/after the cutoff are laid out but never outlined, so
            // they take no space in the compose pass below and the image doesn't reflow as more reveals.
            if ci >= cutoff {
                continue;
            }
            let shape = span_at(ci).and_then(|i| span_shapes[i].as_ref());
            let og = if let Some(sh) = shape {
                let id = sh.font.glyph_id(glyph_chars[gi]);
                let g2 = id.with_scale_and_position(sh.ps, g.position);
                sh.font.outline_glyph(g2).or_else(|| font.outline_glyph(g.clone()))
            } else {
                font.outline_glyph(g.clone())
            };
            if let Some(og) = og {
                outlined.push(og);
                outlined_char.push(ci);
            }
        }

        // ---- image size: layout block ∪ glyph pixel bounds, plus a uniform margin for effects ----
        let (mut bx0, mut by0, mut bx1, mut by1) = (0.0f32, 0.0f32, block_w.ceil(), block_h.ceil());
        for og in &outlined {
            let b = og.px_bounds();
            bx0 = bx0.min(b.min.x);
            by0 = by0.min(b.min.y);
            bx1 = bx1.max(b.max.x);
            by1 = by1.max(b.max.y);
        }
        let r = ow.max(0.0) * scale;
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
        // ponytail: hard cap on the image size - bigger text just gets clipped
        let cw = (bx1 - bx0).ceil() as i32 + 2 * m;
        // a caption bar: the image is the frame's width (or the text's, if wider), the text centred
        let bar = has_box && style.box_mode == BOX_BAR && bar_w >= 1.0;
        let w = (if bar { cw.max(bar_w as i32) } else { cw }).clamp(1, MAX_SIDE as i32) as u32;
        let h = ((by1 - by0).ceil() as i32 + 2 * m).clamp(1, MAX_SIDE as i32) as u32;
        let (ox, oy) = (m as f32 - bx0 + ((w as i32 - cw).max(0) / 2) as f32, m as f32 - by0);
        let (wu, n) = (w as usize, (w * h) as usize);

        // ---- masks ----
        // `fill_owner[i]` is the char index (+1; 0 = none) of whichever glyph currently holds the
        // highest coverage at pixel i, so the compose pass below can look up that char's span color.
        // Two glyphs rarely overlap enough for the tie-break (last-drawn-wins-at-equal-coverage) to
        // matter visually.
        let mut fill = vec![0u8; n];
        let mut fill_owner = vec![0u32; n];
        for (og, &ci) in outlined.iter().zip(outlined_char.iter()) {
            let b = og.px_bounds();
            let (gx, gy) = ((b.min.x + ox) as i32, (b.min.y + oy) as i32);
            og.draw(|x, y, c| {
                let (px, py) = (gx + x as i32, gy + y as i32);
                if px >= 0 && py >= 0 && (px as u32) < w && (py as u32) < h {
                    let i = py as usize * wu + px as usize;
                    let cov = (c.min(1.0) * 255.0 + 0.5) as u8;
                    if cov > fill[i] {
                        fill[i] = cov;
                        fill_owner[i] = ci as u32 + 1;
                    }
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
            let rad = style.box_radius.max(0.0) * scale;
            let boxes = if bar {
                let (y0, y1) = ((oy - pad).round(), (oy + block_h + pad).round());
                vec![BoxRect { x0: 0.0, y0, x1: w as f32, y1, r: [rad.min((y1 - y0) / 2.0); 4] }]
            } else if style.box_mode == BOX_LINES {
                let spans: Vec<Option<(f32, f32)>> = lines
                    .iter()
                    .map(|&(_, _, lw)| {
                        let x0 = ox + (block_w - lw) * align;
                        (lw > 0.0).then_some((x0, x0 + lw))
                    })
                    .collect();
                line_boxes(&spans, oy, lh, sf.ascent() - sf.descent(), pad, rad)
            } else {
                let (y0, y1) = ((oy - pad).round(), (oy + block_h + pad).round());
                let r = rad.min(block_w / 2.0 + pad).min((y1 - y0) / 2.0);
                vec![BoxRect { x0: ox - pad, y0, x1: ox + block_w + pad, y1, r: [r; 4] }]
            };
            let mask = box_mask(&boxes, w, h);
            for (px, &m) in out.rgba.chunks_exact_mut(4).zip(mask.iter()) {
                if m > 0 {
                    over(px, style.box_color, m);
                }
            }
        }
        for (mask, color) in [(&shadow, style.shadow_color), (&outline, style.outline_color)] {
            if mask.is_empty() || color[3] == 0 {
                continue;
            }
            for (px, &m) in out.rgba.chunks_exact_mut(4).zip(mask.iter()) {
                if m > 0 {
                    over(px, color, m);
                }
            }
        }
        // Fill is colored per pixel: a span covering the owning glyph's char overrides `style.color`
        // for just those pixels, everything else uses the base color exactly as before spans existed.
        if !fill.is_empty() {
            for i in 0..n {
                let m = fill[i];
                if m == 0 {
                    continue;
                }
                let color = if fill_owner[i] > 0 {
                    let ci = (fill_owner[i] - 1) as usize;
                    span_at(ci).and_then(|si| style.spans[si].color).unwrap_or(style.color)
                } else {
                    style.color
                };
                if color[3] == 0 {
                    continue;
                }
                over(&mut out.rgba[i * 4..(i + 1) * 4], color, m);
            }
        }
        Some(out)
    }
}

/// One background box in image px: its edges, then the corner radii [top-left, top-right,
/// bottom-right, bottom-left]. A box with a NEGATIVE radius is a fillet instead (see `line_boxes`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct BoxRect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub r: [f32; 4],
}

/// Per-line hugging boxes (`BOX_LINES`). `spans[i]` is line i's horizontal extent in image px
/// (`None` for an empty line: no box there, and the chain breaks). Line i's glyphs start at
/// `top + i * lh` and are `glyph_h` tall; each box adds `pad` round them. Neighbouring boxes meet
/// on one whole-pixel row, so no seam shows. Where one line sticks out past its neighbour, its
/// corner is rounded and the inside corner gets a concave fillet of the same radius, so the stack
/// reads as one shape. A fillet is returned as a `BoxRect` quarter square whose single negative
/// radius marks the corner holding the circle's centre: the square is filled outside that circle.
pub(crate) fn line_boxes(
    spans: &[Option<(f32, f32)>],
    top: f32,
    lh: f32,
    glyph_h: f32,
    pad: f32,
    rad: f32,
) -> Vec<BoxRect> {
    let n = spans.len();
    let mut ys: Vec<(f32, f32)> =
        (0..n).map(|i| (top + i as f32 * lh - pad, top + i as f32 * lh + glyph_h + pad)).collect();
    for i in 0..n.saturating_sub(1) {
        if spans[i].is_some() && spans[i + 1].is_some() {
            let yb = (ys[i].1 + ys[i + 1].0) / 2.0;
            ys[i].1 = yb;
            ys[i + 1].0 = yb;
        }
    }
    let rad = rad.max(0.0);
    let mut out: Vec<BoxRect> = Vec::with_capacity(n * 2);
    let mut idx: Vec<Option<usize>> = vec![None; n];
    for i in 0..n {
        let Some((x0, x1)) = spans[i] else { continue };
        let (y0, y1) = (ys[i].0.round(), ys[i].1.round());
        let r = rad.min((x1 - x0) / 2.0 + pad).min((y1 - y0) / 2.0);
        idx[i] = Some(out.len());
        out.push(BoxRect { x0: x0 - pad, y0, x1: x1 + pad, y1, r: [r; 4] });
    }
    let mut fillets = Vec::new();
    for i in 0..n.saturating_sub(1) {
        let (Some(a), Some(b)) = (idx[i], idx[i + 1]) else { continue };
        let (ra, rb) = (out[a], out[b]);
        let yb = ra.y1;
        let cap = |x: f32| x.min(rad).min((ra.y1 - ra.y0) / 2.0).min((rb.y1 - rb.y0) / 2.0);
        // left: d > 0 when the lower box sticks out further
        let d = ra.x0 - rb.x0;
        if d.abs() < 0.5 {
            (out[a].r[3], out[b].r[0]) = (0.0, 0.0);
        } else if d > 0.0 {
            let f = cap(d / 2.0);
            out[a].r[3] = 0.0;
            out[b].r[0] = out[b].r[0].min(f);
            fillets.push(BoxRect { x0: ra.x0 - f, y0: yb - f, x1: ra.x0, y1: yb, r: [-f, 0.0, 0.0, 0.0] });
        } else {
            let f = cap(-d / 2.0);
            out[b].r[0] = 0.0;
            out[a].r[3] = out[a].r[3].min(f);
            fillets.push(BoxRect { x0: rb.x0 - f, y0: yb, x1: rb.x0, y1: yb + f, r: [0.0, 0.0, 0.0, -f] });
        }
        // right: d > 0 when the lower box sticks out further
        let d = rb.x1 - ra.x1;
        if d.abs() < 0.5 {
            (out[a].r[2], out[b].r[1]) = (0.0, 0.0);
        } else if d > 0.0 {
            let f = cap(d / 2.0);
            out[a].r[2] = 0.0;
            out[b].r[1] = out[b].r[1].min(f);
            fillets.push(BoxRect { x0: ra.x1, y0: yb - f, x1: ra.x1 + f, y1: yb, r: [0.0, -f, 0.0, 0.0] });
        } else {
            let f = cap(-d / 2.0);
            out[b].r[1] = 0.0;
            out[a].r[2] = out[a].r[2].min(f);
            fillets.push(BoxRect { x0: rb.x1, y0: yb, x1: rb.x1 + f, y1: yb + f, r: [0.0, 0.0, -f, 0.0] });
        }
    }
    out.extend(fillets.into_iter().filter(|f| f.x1 - f.x0 >= 0.5));
    out
}

/// Antialiased coverage of the union of `boxes` on a w×h image (rounded rectangles, and fillets:
/// see `line_boxes`). Top and bottom edges sit on whole pixels, so boxes sharing a row leave no seam.
pub(crate) fn box_mask(boxes: &[BoxRect], w: u32, h: u32) -> Vec<u8> {
    let (wu, hu) = (w as usize, h as usize);
    let mut m = vec![0u8; wu * hu];
    let span = |a: f32, b: f32, n: usize| ((a.floor().max(0.0) as usize).min(n), (b.ceil().max(0.0) as usize).min(n));
    for b in boxes {
        let fillet = b.r.iter().position(|&r| r < 0.0);
        // a fillet's neighbouring boxes cover its inner sides: reach a pixel past them for the AA
        let (gx0, gx1) = if fillet.is_some() { span(b.x0 - 1.0, b.x1 + 1.0, wu) } else { span(b.x0, b.x1, wu) };
        let (gy0, gy1) = span(b.y0, b.y1, hu);
        for y in gy0..gy1 {
            let py = y as f32 + 0.5;
            for x in gx0..gx1 {
                let px = x as f32 + 0.5;
                let cov = match fillet {
                    Some(k) => {
                        let f = -b.r[k];
                        let (cx, cy) = match k {
                            0 => (b.x0, b.y0),
                            1 => (b.x1, b.y0),
                            2 => (b.x1, b.y1),
                            _ => (b.x0, b.y1),
                        };
                        let d = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
                        // clipped on the circle's side only: the other sides meet the boxes
                        let ox = if k == 0 || k == 3 { px - b.x0 } else { b.x1 - px };
                        let oy = if k < 2 { py - b.y0 } else { b.y1 - py };
                        (d - f + 0.5).clamp(0.0, 1.0).min((ox.min(oy) + 0.5).clamp(0.0, 1.0))
                    }
                    None => {
                        let (cx, cy) = ((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0);
                        let (hx, hy) = ((b.x1 - b.x0) / 2.0, (b.y1 - b.y0) / 2.0);
                        let r = match (px < cx, py < cy) {
                            (true, true) => b.r[0],
                            (false, true) => b.r[1],
                            (false, false) => b.r[2],
                            (true, false) => b.r[3],
                        };
                        let (qx, qy) = ((px - cx).abs() - (hx - r), (py - cy).abs() - (hy - r));
                        let d = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - r;
                        (0.5 - d).clamp(0.0, 1.0)
                    }
                };
                let v = (cov * 255.0 + 0.5) as u8;
                let o = &mut m[y * wu + x];
                *o = (*o).max(v);
            }
        }
    }
    m
}

/// Straight-alpha "over":`c` with coverage `cov` (0..255) onto the straight-alpha pixel `d`.
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

/// Disc dilation of a coverage mask by radius `r` (antialiased rim): exact Euclidean distance transform
/// (Felzenszwalb–Huttenlocher, columns then rows), O(px) whatever the radius.
fn dilate(src: &[u8], w: u32, h: u32, r: f32) -> Vec<u8> {
    const INF: f32 = 1e20;
    let (w, h) = (w as usize, h as usize);
    // squared distance to the nearest covered (≥ 50 %) pixel
    let mut d: Vec<f32> = src.iter().map(|&v| if v >= 128 { 0.0 } else { INF }).collect();
    let n = w.max(h);
    // line scratch in f64: q² exceeds f32's exact integer range on lines longer than 4096 px
    let (mut f, mut g, mut v, mut z) = (vec![0f64; n], vec![0f64; n], vec![0usize; n], vec![0f64; n + 1]);
    // (lines, len, line stride, element stride): columns, then rows
    for (lines, len, ls, es) in [(w, h, 1, w), (h, w, w, 1)] {
        for l in 0..lines {
            for j in 0..len {
                f[j] = d[l * ls + j * es] as f64;
            }
            dt1d(&f[..len], &mut g, &mut v, &mut z);
            for j in 0..len {
                d[l * ls + j * es] = g[j] as f32;
            }
        }
    }
    d.iter().map(|&d2| ((r + 0.5 - d2.sqrt()).clamp(0.0, 1.0) * 255.0 + 0.5) as u8).collect()
}

/// 1-D squared-distance transform of sampled function `f` into `d` (lower envelope of parabolas).
/// `v`/`z` are scratch of len ≥ f.len() / f.len()+1.
fn dt1d(f: &[f64], d: &mut [f64], v: &mut [usize], z: &mut [f64]) {
    let n = f.len();
    let mut k = 0usize;
    v[0] = 0;
    z[0] = f64::NEG_INFINITY;
    z[1] = f64::INFINITY;
    let s_at = |q: usize, p: usize| ((f[q] + (q * q) as f64) - (f[p] + (p * p) as f64)) / (2.0 * (q - p) as f64);
    for q in 1..n {
        let mut s = s_at(q, v[k]);
        while s <= z[k] {
            k -= 1;
            s = s_at(q, v[k]);
        }
        k += 1;
        v[k] = q;
        z[k] = s;
        z[k + 1] = f64::INFINITY;
    }
    k = 0;
    for (q, dq) in d[..n].iter_mut().enumerate() {
        while z[k + 1] < q as f64 {
            k += 1;
        }
        let p = v[k];
        *dq = (q as f64 - p as f64).powi(2) + f[p];
    }
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
    use crate::model::{Animated, Ease, Keyframe, TextSpan};

    fn alpha_sum(f: &Frame) -> u64 {
        f.rgba.chunks_exact(4).map(|p| p[3] as u64).sum()
    }

    /// A style with wide, left-aligned text so 10 distinct glyphs land at 10 distinct char indices -
    /// shared fixture for the reveal/wave tests below.
    fn ten_char_style() -> TextStyle {
        let mut s = TextStyle::default();
        s.text = "0123456789".into();
        s.align = 0;
        s.size = Animated::new(48.0);
        s
    }

    #[test]
    fn zoomed_text_reuses_half_octave_steps() {
        let mut tr = TextRasterizer::new();
        let style = ten_char_style();
        let (base, z1) = tr.render_zoomed(&style, 0.5, 1.0, 0.0, 0.0);
        if tr.families().is_empty() {
            eprintln!("no system fonts - skipping");
            return;
        }
        assert_eq!(z1, 1.0);
        // a scale animating 1 -> 3 over 2 s at 60 fps: a handful of rasters, not 120
        for i in 0..=120 {
            let k = 1.0 + 2.0 * i as f32 / 120.0;
            let (img, z) = tr.render_zoomed(&style, 0.5, crate::engine::compose::half_octave(k), 0.0, 0.0);
            assert!(z >= k / 1.1 && z <= k * 1.5, "scale {k} drawn at {z}");
            // the bitmap is z times the zoom-1 one, so its on-screen size (its size / z) is unchanged
            let shown = img.width as f32 / z;
            assert!((shown - base.width as f32).abs() <= 2.0 + 0.03 * base.width as f32, "{shown} vs {}", base.width);
        }
        assert!(tr.cache.len() <= 4, "half-octave steps: {} rasters for a 1 -> 3 zoom", tr.cache.len());
    }

    #[test]
    fn zoomed_text_stays_under_the_size_cap() {
        let mut tr = TextRasterizer::new();
        let mut style = ten_char_style();
        style.size = Animated::new(400.0);
        let (base, _) = tr.render_zoomed(&style, 1.0, 1.0, 0.0, 0.0);
        if tr.families().is_empty() {
            return;
        }
        let (img, z) = tr.render_zoomed(&style, 1.0, 64.0, 0.0, 0.0);
        assert!(z < 64.0 && z >= 1.0, "stepped down: {z}");
        assert!(img.width < MAX_SIDE && img.height < MAX_SIDE, "{}x{}", img.width, img.height);
        assert!((img.width as f32) * (img.height as f32) <= ZOOM_MAX_PIXELS * 1.1);
        assert!(img.width > base.width, "still drawn bigger than zoom 1");
    }

    #[test]
    fn renders_default_style_and_caches() {
        let mut tr = TextRasterizer::new();
        let style = TextStyle::default();
        let a = tr.render(&style, 0.5, 0.0);
        if tr.families().is_empty() {
            eprintln!("no system fonts - skipping");
            assert_eq!((a.width, a.height), (1, 1));
            return;
        }
        assert!(a.width > 4 && a.height > 4, "{}x{}", a.width, a.height);
        assert!(alpha_sum(&a) > 0);
        // white fill: every covered pixel is white
        assert!(a.rgba.chunks_exact(4).filter(|p| p[3] > 0).all(|p| p[0] == 255 && p[1] == 255 && p[2] == 255));
        let b = tr.render(&style, 0.5, 0.0);
        assert!(Arc::ptr_eq(&a, &b), "cache hit expected");
        let c = tr.render(&style, 1.0, 0.0);
        assert!(!Arc::ptr_eq(&a, &c));
        assert!(c.width > a.width);

        // unknown font falls back, never panics
        let mut s2 = style.clone();
        s2.font = "No Such Font 123".into();
        s2.bold = true;
        s2.italic = true;
        assert!(alpha_sum(&tr.render(&s2, 0.5, 0.0)) > 0);
        // a list takes the first installed font: a missing one, then Impact = Impact
        if std::path::Path::new("C:/Windows/Fonts/impact.ttf").is_file() {
            let mut one = style.clone();
            one.font = "Impact".into();
            let mut list = style.clone();
            list.font = "No Such Font 123, Impact".into();
            let (x, y) = (tr.render(&one, 0.5, 0.0), tr.render(&list, 0.5, 0.0));
            assert_eq!((x.width, x.height, &x.rgba), (y.width, y.height, &y.rgba));
        }

        // outline + shadow + box make the image bigger and add non-white pixels
        let mut s3 = style.clone();
        s3.outline_width.value = 4.0;
        s3.shadow = true;
        s3.box_color = [0, 0, 255, 255];
        let d = tr.render(&s3, 0.5, 0.0);
        assert!(d.width > a.width && d.height > a.height);
        assert!(d.rgba.chunks_exact(4).any(|p| p[3] > 0 && p[2] == 255 && p[0] == 0));
        assert!(d.rgba.chunks_exact(4).any(|p| p[3] > 0 && p[0] == 0 && p[2] == 0));

        // multi-line, left aligned, is taller
        let mut s4 = style.clone();
        s4.text = "One\nTwo\nThree".into();
        s4.align = 0;
        let e = tr.render(&s4, 0.5, 0.0);
        assert!(e.height > a.height * 2);
    }

    /// CRITICAL regression guard: `spans: vec![]` is what every project ever saved before spans
    /// existed loads as (`#[serde(default)]`), so it must render byte-identical to how this file
    /// rendered before spans existed. `TextStyle::default()` already has `spans: vec![]` and is
    /// exactly what `renders_default_style_and_caches` above pins (dimensions grow with scale, white
    /// fill, outline/shadow/box growth, multi-line height) - this test adds the direct check: pushing
    /// a span onto a style and then clearing it back to empty must not perturb the cache key or a
    /// single output byte, proving the new span bookkeeping is a true no-op when spans is empty.
    #[test]
    fn spans_empty_matches_unspanned_baseline_byte_for_byte() {
        let mut tr = TextRasterizer::new();
        let base = TextStyle::default();
        if tr.families().is_empty() {
            eprintln!("no system fonts - skipping");
            return;
        }
        let a = tr.render(&base, 0.6, 0.0);

        let mut touched = base.clone();
        touched.spans.push(TextSpan { start: 0, end: 3, color: Some([255, 0, 0, 255]), ..Default::default() });
        touched.spans.clear();
        assert_eq!(touched.cache_key(), base.cache_key(), "empty spans must not change the cache key");
        let b = tr.render(&touched, 0.6, 0.0);
        assert_eq!((a.width, a.height), (b.width, b.height));
        assert_eq!(a.rgba, b.rgba, "unspanned render regressed");
    }

    /// A span with an out-of-range / inverted / zero-width char range covers no character, so it must
    /// render identically to having no span at all - and must never panic (no unwrap on span data).
    #[test]
    fn spans_out_of_range_are_ignored_gracefully() {
        let mut tr = TextRasterizer::new();
        let base = TextStyle::default(); // text: "Text" - 4 chars
        if tr.families().is_empty() {
            eprintln!("no system fonts - skipping");
            return;
        }
        let a = tr.render(&base, 0.6, 0.0);

        let mut s = base.clone();
        s.spans = vec![
            TextSpan { start: 100, end: 200, color: Some([255, 0, 0, 255]), ..Default::default() }, // past the end
            TextSpan { start: 3, end: 1, color: Some([0, 255, 0, 255]), ..Default::default() },     // inverted
            TextSpan { start: 2, end: 2, bold: Some(true), ..Default::default() },                  // zero-width
        ];
        let b = tr.render(&s, 0.6, 0.0); // must not panic
        assert_eq!((a.width, a.height), (b.width, b.height));
        assert_eq!(a.rgba, b.rgba, "an out-of-range span must be a no-op");
    }

    /// A span covering part of the text renders visibly different pixels in that range (here: color),
    /// while the base render stays the baseline - this is the actual per-run override feature.
    #[test]
    fn span_color_override_is_visible_only_in_its_range() {
        let mut tr = TextRasterizer::new();
        let mut style = TextStyle::default();
        style.text = "Hello World".into();
        style.align = 0;
        if tr.families().is_empty() {
            eprintln!("no system fonts - skipping");
            return;
        }
        let baseline = tr.render(&style, 1.0, 0.0);
        let all_white =
            baseline.rgba.chunks_exact(4).filter(|p| p[3] > 0).all(|p| p[0] == 255 && p[1] == 255 && p[2] == 255);
        assert!(all_white);

        // "World" is chars 6..11 of "Hello World".
        style.spans.push(TextSpan {
            start: 6,
            end: 11,
            color: Some([255, 0, 0, 255]),
            bold: Some(true),
            ..Default::default()
        });
        let spanned = tr.render(&style, 1.0, 0.0);
        assert_ne!(baseline.rgba, spanned.rgba, "spanned render must differ from the baseline");
        assert!(
            spanned.rgba.chunks_exact(4).any(|p| p[3] > 0 && p[0] > 200 && p[1] < 80 && p[2] < 80),
            "expected red-tinted pixels from the span's color override"
        );
        // "Hello " (unspanned) is still plain white.
        assert!(
            spanned.rgba.chunks_exact(4).filter(|p| p[3] > 0).any(|p| p[0] == 255 && p[1] == 255 && p[2] == 255),
            "unspanned text must keep the base color"
        );
    }

    #[test]
    fn user_font_load_is_idempotent_and_listed() {
        let src = std::path::Path::new("C:\\Windows\\Fonts\\arial.ttf");
        if !src.exists() {
            eprintln!("no arial.ttf - skipping");
            return;
        }
        let dst = std::env::temp_dir().join(format!("se-userfont-{}.ttf", std::process::id()));
        std::fs::copy(src, &dst).expect("copy font");
        let mut tr = TextRasterizer::new();
        let paths = vec![dst.to_string_lossy().into_owned()];
        tr.load_user_fonts(&paths);
        assert!(tr.families().iter().any(|f| f == "Arial"), "{:?}", tr.families());
        let n = tr.families().len();
        tr.load_user_fonts(&paths); // idempotent
        assert_eq!(tr.families().len(), n);
        // renders with the user family (before system fonts are scanned the db has only this file,
        // but render() also pulls in system fonts - either way it must not panic and must cover pixels)
        let mut style = TextStyle::default();
        style.font = "Arial".into();
        assert!(alpha_sum(&tr.render(&style, 0.5, 0.0)) > 0);
        // a missing path is remembered without error
        tr.load_user_fonts(&["Z:\\nope\\missing-font.ttf".into()]);
        let _ = std::fs::remove_file(&dst);
    }

    #[test]
    fn empty_text_is_1x1() {
        let mut tr = TextRasterizer::new();
        let mut style = TextStyle::default();
        style.text = "  \n ".into();
        let f = tr.render(&style, 1.0, 0.0);
        assert_eq!((f.width, f.height, f.rgba[3]), (1, 1, 0));
        style.text = "x".into();
        assert_eq!(tr.render(&style, 0.0, 0.0).width, 1);
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

    #[test]
    fn dilate_matches_brute_force_and_is_fast() {
        // sparse seeds on a 40×30 mask, r = 3.7: every pixel equals the brute-force nearest-seed disc
        let (w, h) = (40usize, 30usize);
        let mut m = vec![0u8; w * h];
        for &(x, y) in &[(3usize, 4usize), (20, 15), (21, 15), (38, 28), (0, 0)] {
            m[y * w + x] = 255;
        }
        m[10 * w + 10] = 100; // below 50 % coverage: not a seed
        let r = 3.7f32;
        let d = dilate(&m, w as u32, h as u32, r);
        for y in 0..h {
            for x in 0..w {
                let mut best = f32::INFINITY;
                for (i, &v) in m.iter().enumerate() {
                    if v >= 128 {
                        let (xx, yy) = ((i % w) as f32, (i / w) as f32);
                        best = best.min(((x as f32 - xx).powi(2) + (y as f32 - yy).powi(2)).sqrt());
                    }
                }
                let want = ((r + 0.5 - best).clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                assert_eq!(d[y * w + x], want, "at {x},{y}");
            }
        }
        // size-1000 / outline-50 class (2000×600, r = 100): O(px), so well under a second
        let (w, h) = (2000u32, 600u32);
        let mut big = vec![0u8; (w * h) as usize];
        for y in 200..400 {
            big[(y * w + 500) as usize..(y * w + 1500) as usize].fill(255);
        }
        let t = std::time::Instant::now();
        let d = dilate(&big, w, h, 100.0);
        assert!(t.elapsed() < std::time::Duration::from_secs(2), "{:?}", t.elapsed());
        assert_eq!(d[(300 * w + 1000) as usize], 255);
        assert_eq!(d[(300 * w + 1550) as usize], 255); // 51 px out
        assert_eq!(d[(300 * w + 1600) as usize], 0); // 101 px out
    }

    // ---- ws:text-titles ----

    #[test]
    fn render_cache_stable_for_static_style() {
        let mut tr = TextRasterizer::new();
        tr.load_system_fonts();
        if tr.families().is_empty() {
            eprintln!("no system fonts - skipping");
            return;
        }
        let style = ten_char_style();
        let a = tr.render(&style, 0.5, 0.0);
        let b = tr.render(&style, 0.5, 5.0);
        assert!(Arc::ptr_eq(&a, &b), "a non-animated style must cache once regardless of t");
        assert_eq!(tr.cache.len(), 1);
    }

    #[test]
    fn render_cache_buckets_animated_reveal() {
        let mut tr = TextRasterizer::new();
        tr.load_system_fonts();
        if tr.families().is_empty() {
            eprintln!("no system fonts - skipping");
            return;
        }
        let mut style = ten_char_style();
        style.reveal = Animated {
            keys: vec![
                Keyframe { t: 0.0, v: 0.0, ease: Ease::Linear },
                Keyframe { t: 1.0, v: 1.0, ease: Ease::Linear },
            ],
            ..Animated::new(0.0)
        };
        let a = tr.render(&style, 1.0, 0.0);
        let b = tr.render(&style, 1.0, 1.0);
        assert_ne!(a.rgba, b.rgba, "reveal 0.0 vs 1.0 must render visibly different frames");
        assert!(tr.cache.len() > 1, "an animated reveal must not collapse to one cache entry");
        // 100 near-identical t values (same 1/30s bucket) must not each add a cache entry.
        for i in 0..100 {
            tr.render(&style, 1.0, 0.5 + i as f64 * 1e-5);
        }
        assert!(
            tr.cache.len() < 10,
            "near-identical t must share buckets, not grow one entry per call: {}",
            tr.cache.len()
        );
    }

    #[test]
    fn reveal_cutoff_hides_trailing_chars() {
        let mut tr = TextRasterizer::new();
        tr.load_system_fonts();
        if tr.families().is_empty() {
            eprintln!("no system fonts - skipping");
            return;
        }
        let mut style = ten_char_style();
        style.reveal = Animated::new(0.5);
        let half = tr.rasterize(&style, 1.0, 0.0, 0.0).expect("renders");
        style.reveal = Animated::new(1.0);
        let full = tr.rasterize(&style, 1.0, 0.0, 0.0).expect("renders");
        assert!(alpha_sum(&half) > 0, "the first 5 chars must still draw something");
        assert!(alpha_sum(&half) < alpha_sum(&full), "half-revealed must draw strictly less than fully revealed");
    }

    #[test]
    fn wave_offsets_glyph_y_without_changing_glyph_count() {
        let mut tr = TextRasterizer::new();
        tr.load_system_fonts();
        if tr.families().is_empty() {
            eprintln!("no system fonts - skipping");
            return;
        }
        let mut style = ten_char_style();
        let flat = tr.rasterize(&style, 1.0, 0.3, 0.0).expect("renders");
        style.wave = Animated::new(20.0);
        let waved = tr.rasterize(&style, 1.0, 0.3, 0.0).expect("renders");
        // same glyph "ink" (alpha mass), just moved - not clipped away - so the two frames differ but
        // carry a comparable amount of coverage (loosely: within 15%, allowing for edge clipping).
        let (fa, wa) = (alpha_sum(&flat) as f64, alpha_sum(&waved) as f64);
        assert!(wa > 0.0);
        assert!((fa - wa).abs() / fa < 0.15, "flat={fa} waved={wa}");
        assert_ne!(flat.rgba, waved.rgba, "a nonzero wave must move pixels");
    }

    /// feat-retro-2000s-pack: the Classic 2000s font list falls back to the bundled Comic Neue on a PC
    /// without Comic Sans MS, and uses Comic Sans MS when it is there.
    #[test]
    fn classic_font_falls_back_to_comic_neue_without_comic_sans() {
        // only the bundled fonts: a PC with no Comic Sans MS
        let mut r = TextRasterizer::new();
        for f in BUNDLED_FONTS {
            r.db.load_font_data(f.to_vec());
        }
        r.refresh_families();
        assert!(r.families().iter().any(|f| f == "Comic Neue"), "{:?}", r.families());
        assert_eq!(r.resolved_family(crate::engine::retro::FONT, true).as_deref(), Some("Comic Neue"));
        assert_eq!(r.resolved_family(crate::engine::retro::FONT, false).as_deref(), Some("Comic Neue"));
        // and it draws: a classic title renders ink
        let style = crate::engine::retro::text_style("My Movie", 64.0);
        let f = r.render(&style, 1.0, 0.0);
        assert!(f.width > 50 && alpha_sum(&f) > 0);
        // with the system fonts: Comic Sans MS wins when installed, else still Comic Neue
        let mut sys = TextRasterizer::new();
        sys.load_system_fonts();
        let want = if sys.families().iter().any(|f| f == "Comic Sans MS") { "Comic Sans MS" } else { "Comic Neue" };
        assert_eq!(sys.resolved_family(crate::engine::retro::FONT, true).as_deref(), Some(want));
    }
}
