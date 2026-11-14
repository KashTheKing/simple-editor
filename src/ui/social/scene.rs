//! Draw lists. Everything the guides draw - zones, silhouettes, the device and its sheets - is built here
//! as plain `Op`s (screen points), so the viewer (`paint`) and the bitmap renderer (`raster`) show exactly
//! the same picture. Generic vector shapes only: rounded rectangles, circles, lines, simple glyphs; no
//! logos, no brand assets, no copyrighted icons.

use super::profile::SocialProfile;
use super::*;
use eframe::egui::{Color32, Pos2, Rect};

#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// `top`: only the two top corners are rounded (a sheet).
    Rect { r: Rect, rad: f32, top: bool, fill: Color32, line: f32, lc: Color32 },
    Circle { c: Pos2, r: f32, fill: Color32, line: f32, lc: Color32 },
    Line { a: Pos2, b: Pos2, w: f32, col: Color32 },
    /// Filled polygon, star-shaped from its first point.
    Poly { p: Vec<Pos2>, fill: Color32 },
    /// `p` = top-left (al 0), top-centre (1) or top-right (2) of the line box; `s` = font size in points.
    Text { p: Pos2, s: f32, col: Color32, bold: bool, t: String, al: u8 },
    /// The profile picture (or its placeholder) in a circle.
    Avatar { c: Pos2, r: f32 },
    /// Vertical black gradient: `up` = darkest at the bottom.
    Grad { r: Rect, up: bool, a: u8 },
    /// Clip the following ops to this rectangle (None = no clip).
    Clip(Option<Rect>),
}

pub fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(r, g, b, a)
}
pub fn white(a: f32) -> Color32 {
    Color32::from_white_alpha((a.clamp(0.0, 1.0) * 255.0) as u8)
}
pub const RED: Color32 = Color32::from_rgb(230, 40, 40);

/// Op builder with a global opacity (the UI fades while a sheet opens).
pub struct B {
    pub ops: Vec<Op>,
    pub a: f32,
}

impl B {
    pub fn new() -> B {
        B { ops: Vec::new(), a: 1.0 }
    }
    fn col(&self, c: Color32) -> Color32 {
        if self.a >= 0.999 { c } else { c.gamma_multiply(self.a.max(0.0)) }
    }
    pub fn rect(&mut self, r: Rect, rad: f32, fill: Color32) {
        let fill = self.col(fill);
        self.ops.push(Op::Rect { r, rad, top: false, fill, line: 0.0, lc: Color32::TRANSPARENT });
    }
    pub fn rect_top(&mut self, r: Rect, rad: f32, fill: Color32) {
        let fill = self.col(fill);
        self.ops.push(Op::Rect { r, rad, top: true, fill, line: 0.0, lc: Color32::TRANSPARENT });
    }
    pub fn frame(&mut self, r: Rect, rad: f32, w: f32, lc: Color32) {
        let lc = self.col(lc);
        self.ops.push(Op::Rect { r, rad, top: false, fill: Color32::TRANSPARENT, line: w, lc });
    }
    pub fn circle(&mut self, c: Pos2, r: f32, fill: Color32) {
        let fill = self.col(fill);
        self.ops.push(Op::Circle { c, r, fill, line: 0.0, lc: Color32::TRANSPARENT });
    }
    pub fn ring(&mut self, c: Pos2, r: f32, w: f32, lc: Color32) {
        let lc = self.col(lc);
        self.ops.push(Op::Circle { c, r, fill: Color32::TRANSPARENT, line: w, lc });
    }
    pub fn line(&mut self, a: Pos2, b: Pos2, w: f32, col: Color32) {
        let col = self.col(col);
        self.ops.push(Op::Line { a, b, w, col });
    }
    pub fn poly(&mut self, p: Vec<Pos2>, fill: Color32) {
        let fill = self.col(fill);
        self.ops.push(Op::Poly { p, fill });
    }
    pub fn text(&mut self, p: Pos2, s: f32, col: Color32, bold: bool, t: &str, al: u8) {
        if t.is_empty() || s < 1.0 {
            return;
        }
        let col = self.col(col);
        self.ops.push(Op::Text { p, s, col, bold, t: t.to_string(), al });
    }
    pub fn avatar(&mut self, c: Pos2, r: f32) {
        if self.a > 0.02 {
            self.ops.push(Op::Avatar { c, r });
        }
    }
    pub fn grad(&mut self, r: Rect, up: bool, a: f32) {
        let a = (a * self.a * 255.0).clamp(0.0, 255.0) as u8;
        self.ops.push(Op::Grad { r, up, a });
    }
    pub fn clip(&mut self, r: Option<Rect>) {
        self.ops.push(Op::Clip(r));
    }
}

impl Default for B {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------- text measurement and wrapping

/// Helvetica advance widths (per mille of the em) for ASCII 32..=126: font-independent, so the same
/// text breaks at the same place in the viewer, the bitmap renderer and the tests.
const HELV: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556, 556, 556,
    556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556,
    833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500,
    556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500,
    334, 260, 334, 584,
];

pub fn text_w(s: &str, size: f32, bold: bool) -> f32 {
    let em: f32 = s
        .chars()
        .map(|c| match c as u32 {
            32..=126 => HELV[c as usize - 32] as f32 / 1000.0,
            0..=31 => 0.0,
            0x2E80.. => 1.0,
            _ => 0.6,
        })
        .sum();
    em * size * if bold { 1.05 } else { 1.0 }
}

/// Greedy word wrap to `max_w`; `\n` breaks; a word wider than the line is cut.
pub fn wrap(s: &str, size: f32, bold: bool, max_w: f32) -> Vec<String> {
    let mut out = Vec::new();
    for para in s.split('\n') {
        let mut line = String::new();
        for word in para.split_whitespace() {
            let joined = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if text_w(&joined, size, bold) <= max_w || line.is_empty() && text_w(word, size, bold) <= max_w {
                line = joined;
                continue;
            }
            if !line.is_empty() {
                out.push(std::mem::take(&mut line));
            }
            let mut piece = String::new();
            for ch in word.chars() {
                if text_w(&format!("{piece}{ch}"), size, bold) > max_w && !piece.is_empty() {
                    out.push(std::mem::take(&mut piece));
                }
                piece.push(ch);
            }
            line = piece;
        }
        out.push(line);
    }
    out
}

/// Cut `lines` to `max` lines. When something was cut the last line is shortened to leave room for the
/// "... more" tail (`tail_w` wide) and `true` comes back.
pub fn collapse(mut lines: Vec<String>, max: usize, size: f32, bold: bool, max_w: f32, tail_w: f32) -> (Vec<String>, bool) {
    if max == 0 || lines.len() <= max {
        return (lines, false);
    }
    lines.truncate(max);
    let last = lines.last_mut().expect("max >= 1");
    while !last.is_empty() && text_w(last, size, bold) > max_w - tail_w {
        match last.rfind(' ') {
            Some(i) => last.truncate(i),
            None => {
                last.pop();
            }
        }
    }
    (lines, true)
}

/// One line cut with an ellipsis to fit `max_w`.
pub fn ellipsize(s: &str, size: f32, bold: bool, max_w: f32) -> String {
    if text_w(s, size, bold) <= max_w {
        return s.to_string();
    }
    let mut t: String = s.to_string();
    while !t.is_empty() && text_w(&format!("{t}…"), size, bold) > max_w {
        t.pop();
    }
    format!("{}…", t.trim_end())
}

// ---------------------------------------------------------------- glyphs

/// A generic icon centred on `c`, `h` = half its size.
pub fn glyph(b: &mut B, name: &str, c: Pos2, h: f32, col: Color32) {
    let p = |x: f32, y: f32| pos2(c.x + x * h, c.y + y * h);
    let w = (h * 0.16).max(1.0);
    let dark = Color32::from_black_alpha(150);
    match name {
        "like" => {
            // heart
            b.circle(p(-0.5, -0.3), h * 0.52, col);
            b.circle(p(0.5, -0.3), h * 0.52, col);
            b.poly(vec![p(-0.98, -0.1), p(0.98, -0.1), p(0.0, 0.95)], col);
        }
        "thumb" | "thumb_down" => {
            // thumbs-up: cuff + hand (thumb up, fingers right); thumbs-down is the same turned 180 degrees
            let f = if name == "thumb" { 1.0 } else { -1.0 };
            let q = |x: f32, y: f32| p(x * f, y * f);
            b.poly(
                vec![q(-0.40, 0.85), q(-0.40, -0.05), q(-0.05, -0.40), q(0.10, -0.95), q(0.40, -0.95), q(0.45, -0.45), q(0.95, -0.45), q(0.95, 0.65), q(0.70, 0.85)],
                col,
            );
            b.rect(Rect::from_two_pos(q(-0.95, -0.05), q(-0.55, 0.85)), h * 0.1, col);
        }
        "plane" => {
            // paper plane
            b.poly(vec![p(-0.95, -0.15), p(0.95, -0.9), p(0.15, 0.9), p(-0.1, 0.15)], col);
            b.line(p(-0.1, 0.15), p(0.95, -0.9), w, Color32::from_black_alpha(140));
        }
        "comment" => {
            b.rect(Rect::from_min_max(p(-1.0, -0.8), p(1.0, 0.55)), h * 0.45, col);
            b.poly(vec![p(-0.5, 0.45), p(0.1, 0.45), p(-0.65, 0.98)], col);
            for x in [-0.45, 0.0, 0.45] {
                b.circle(p(x, -0.12), h * 0.1, dark);
            }
        }
        "share" | "repost" => {
            b.line(p(-0.8, 0.7), p(0.5, -0.5), w * 1.6, col);
            b.poly(vec![p(0.98, -0.98), p(0.05, -0.82), p(0.82, -0.05)], col);
        }
        "more" => {
            for y in [-0.62, 0.0, 0.62] {
                b.circle(p(0.0, y), h * 0.15, col);
            }
        }
        "search" => {
            b.ring(p(-0.15, -0.15), h * 0.62, w * 1.4, col);
            b.line(p(0.3, 0.3), p(0.9, 0.9), w * 1.6, col);
        }
        "camera" => {
            b.frame(Rect::from_min_max(p(-0.95, -0.55), p(0.95, 0.7)), h * 0.2, w * 1.2, col);
            b.ring(p(0.0, 0.08), h * 0.36, w, col);
            b.line(p(-0.3, -0.7), p(0.3, -0.7), w * 1.4, col);
        }
        "disc" => {
            b.circle(c, h, rgba(20, 20, 20, 230));
            b.avatar(c, h * 0.62);
            b.ring(c, h * 0.95, w * 1.2, white(0.55));
        }
        "note" => {
            b.circle(p(-0.4, 0.6), h * 0.38, col);
            b.line(p(-0.05, 0.6), p(-0.05, -0.9), w * 1.3, col);
            b.line(p(-0.05, -0.9), p(0.7, -0.45), w * 1.3, col);
        }
        "play" => b.poly(vec![p(-0.5, -0.8), p(-0.5, 0.8), p(0.85, 0.0)], col),
        "mute" => {
            b.poly(vec![p(-0.95, -0.3), p(-0.4, -0.3), p(0.1, -0.8), p(0.1, 0.8), p(-0.4, 0.3), p(-0.95, 0.3)], col);
            b.line(p(0.45, -0.4), p(0.45, 0.4), w, col);
            b.line(p(0.8, -0.65), p(0.8, 0.65), w, col);
        }
        "cc" => {
            b.frame(Rect::from_min_max(p(-1.0, -0.7), p(1.0, 0.7)), h * 0.22, w * 1.2, col);
            b.line(p(-0.6, 0.0), p(-0.15, 0.0), w * 1.2, col);
            b.line(p(0.15, 0.0), p(0.6, 0.0), w * 1.2, col);
        }
        "gear" => {
            b.ring(c, h * 0.55, w * 1.5, col);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::FRAC_PI_4;
                b.line(p(a.cos() * 0.75, a.sin() * 0.75), p(a.cos() * 1.0, a.sin() * 1.0), w * 1.5, col);
            }
        }
        "full" => {
            for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                b.line(p(sx * 0.85, sy * 0.85), p(sx * 0.4, sy * 0.85), w * 1.4, col);
                b.line(p(sx * 0.85, sy * 0.85), p(sx * 0.85, sy * 0.4), w * 1.4, col);
            }
        }
        "theater" => b.frame(Rect::from_min_max(p(-1.0, -0.55), p(1.0, 0.55)), h * 0.1, w * 1.3, col),
        "clock" => {
            b.ring(c, h * 0.9, w * 1.2, col);
            b.line(c, p(0.0, -0.55), w, col);
            b.line(c, p(0.4, 0.2), w, col);
        }
        "save" => {
            let pts = vec![p(-0.6, -0.9), p(0.6, -0.9), p(0.6, 0.9), p(0.0, 0.35), p(-0.6, 0.9), p(-0.6, -0.9)];
            for k in 0..5 {
                b.line(pts[k], pts[k + 1], w * 1.3, col);
            }
        }
        "remix" => {
            b.ring(p(-0.38, 0.0), h * 0.6, w * 1.3, col);
            b.ring(p(0.38, 0.0), h * 0.6, w * 1.3, col);
        }
        "plus" => {
            b.rect(Rect::from_min_max(p(-1.0, -0.62), p(1.0, 0.62)), h * 0.3, white(0.95));
            b.line(p(-0.3, 0.0), p(0.3, 0.0), w * 1.4, dark);
            b.line(p(0.0, -0.3), p(0.0, 0.3), w * 1.4, dark);
        }
        "home" => {
            b.poly(vec![p(-0.95, -0.05), p(0.0, -0.9), p(0.95, -0.05)], col);
            b.rect(Rect::from_min_max(p(-0.65, -0.05), p(0.65, 0.85)), 0.0, col);
        }
        "friends" => {
            b.ring(p(-0.4, -0.3), h * 0.34, w * 1.2, col);
            b.ring(p(0.4, -0.3), h * 0.34, w * 1.2, col);
            b.line(p(-0.9, 0.7), p(-0.4, 0.2), w * 1.2, col);
            b.line(p(-0.4, 0.2), p(0.1, 0.7), w * 1.2, col);
            b.line(p(0.1, 0.7), p(0.4, 0.2), w * 1.2, col);
            b.line(p(0.4, 0.2), p(0.9, 0.7), w * 1.2, col);
        }
        "inbox" => {
            b.frame(Rect::from_min_max(p(-0.95, -0.6), p(0.95, 0.65)), h * 0.18, w * 1.2, col);
            b.line(p(-0.95, 0.1), p(-0.3, 0.1), w, col);
            b.line(p(-0.3, 0.1), p(-0.15, 0.4), w, col);
            b.line(p(-0.15, 0.4), p(0.15, 0.4), w, col);
            b.line(p(0.15, 0.4), p(0.3, 0.1), w, col);
            b.line(p(0.3, 0.1), p(0.95, 0.1), w, col);
        }
        "user" => {
            b.ring(p(0.0, -0.4), h * 0.4, w * 1.3, col);
            b.line(p(-0.75, 0.85), p(0.0, 0.3), w * 1.3, col);
            b.line(p(0.0, 0.3), p(0.75, 0.85), w * 1.3, col);
        }
        "menu" => {
            for y in [-0.55, 0.0, 0.55] {
                b.line(p(-0.8, y), p(0.8, y), w * 1.4, col);
            }
        }
        "tag" => {
            b.ring(p(0.0, 0.0), h * 0.85, w * 1.2, col);
            b.circle(p(0.0, -0.1), h * 0.2, col);
            b.line(p(-0.4, 0.5), p(0.4, 0.5), w, col);
        }
        "badge" => {
            b.frame(Rect::from_min_max(p(-0.4, -0.95), p(0.95, 0.4)), h * 0.18, w * 1.2, col);
            b.frame(Rect::from_min_max(p(-0.95, -0.4), p(0.4, 0.95)), h * 0.18, w * 1.2, col);
        }
        _ => b.ring(c, h * 0.7, w, col),
    }
}

// ---------------------------------------------------------------- silhouettes

/// What a silhouette text needs.
pub struct Ctx<'a> {
    pub prof: &'a SocialProfile,
    /// The project's name: a long-form title.
    pub title: &'a str,
    /// Playhead / duration, 0..1 (the progress bar fills with it).
    pub progress: f32,
    /// "0:11 / 8:58".
    pub clock: &'a str,
}

/// Draw `lay.ui` through `map`. Elements flagged `o` (outside the frame) only draw when `outside`.
/// Returns the tap targets (rectangle, tap name) of the interactive ones.
pub fn draw_ui(b: &mut B, lay: &Layout, map: &Map, cx: &Ctx, outside: bool) -> Vec<(Rect, String)> {
    let k = map.k;
    let mut hits = Vec::new();
    for e in &lay.ui {
        if e.o != 0 && !outside {
            continue;
        }
        let r = if matches!(e.t.as_str(), "scr" | "bar") { map.rect(e.r) } else { map.rect_el(e.r) };
        if !matches!(e.tap.as_str(), "" | "none") {
            hits.push((r, e.tap.clone()));
        }
        let (cxp, w, h) = (r.center().x, r.width(), r.height());
        let ink = if e.c == "r" { RED } else { Color32::WHITE };
        match e.t.as_str() {
            "scr" => b.grad(r, e.d == "u", e.a),
            "bar" => b.rect(r, 0.0, Color32::from_black_alpha((e.a * 255.0) as u8)),
            "ico" => {
                let has_t = !e.n.is_empty() || !e.lbl.is_empty();
                let half = if has_t { (w * 0.31).min(h * 0.25) } else { w.min(h) * 0.28 };
                let cy = if has_t { r.top() + h * 0.34 } else { r.center().y };
                glyph(b, &e.i, pos2(cxp, cy), half, ink);
                let t = if e.lbl.is_empty() { cx.prof.count(&e.n).to_string() } else { e.lbl.clone() };
                b.text(pos2(cxp, r.top() + h * 0.62), 12.0 * k, ink, false, &t, 1);
            }
            "ava" => {
                let rad = w.min(h) * 0.5;
                let c = pos2(cxp, r.center().y);
                b.circle(c, rad, rgba(60, 60, 64, 255));
                b.avatar(c, rad);
                b.ring(c, rad, (1.2 * k).max(1.0), white(0.85));
                if e.plus != 0 {
                    glyph(b, "plus", pos2(c.x, c.y + rad), rad * 0.34, Color32::WHITE);
                }
            }
            "txt" => draw_text(b, e, r, k, cx),
            "pil" => {
                let label = if e.x.is_empty() { cx.prof.field(&e.f, cx.title, cx.clock) } else { e.x.clone() };
                let (fill, tc) = if e.fill != 0 { (Color32::WHITE, Color32::BLACK) } else { (white(0.0), Color32::WHITE) };
                b.rect(r, h * 0.5, fill);
                if e.fill == 0 {
                    b.frame(r, h * 0.5, (1.2 * k).max(1.0), white(0.9));
                }
                let s = if e.s > 0.0 { e.s } else { 13.0 } * k;
                b.text(pos2(cxp, r.center().y - s * 0.6), s, tc, true, &label, 1);
            }
            "prg" => {
                let th = h.max(1.5);
                b.rect(r, th * 0.5, white(0.35));
                let mut fr = r;
                fr.max.x = r.min.x + w * cx.progress.clamp(0.0, 1.0);
                b.rect(fr, th * 0.5, ink);
                if e.k != 0 {
                    b.circle(pos2(fr.max.x, r.center().y), th * 1.4, ink);
                }
            }
            "chp" => {
                b.rect(r, h * 0.2, rgba(255, 255, 255, 70));
                glyph(b, "play", r.center(), h * 0.28, white(0.9));
            }
            _ => {}
        }
    }
    hits
}

/// A text element: single line (ellipsised) or a wrapped caption with a "... more" tail.
fn draw_text(b: &mut B, e: &El, r: Rect, k: f32, cx: &Ctx) {
    let s = (if e.s > 0.0 { e.s } else { 14.0 }) * k;
    let text = if e.x.is_empty() { cx.prof.field(&e.f, cx.title, cx.clock) } else { e.x.clone() };
    if text.is_empty() {
        return;
    }
    let bold = e.b != 0;
    let lh = s * 1.3;
    let x = if e.al == "c" { r.center().x } else { r.left() };
    let al = u8::from(e.al == "c");
    if e.l <= 1 && e.more == 0 {
        b.text(pos2(x, r.top()), s, Color32::WHITE, bold, &ellipsize(&text, s, bold, r.width()), al);
        return;
    }
    let (lines, cut) = collapse(wrap(&text, s, bold, r.width()), e.l.max(1) as usize, s, bold, r.width(), text_w("… more", s, bold));
    let n = lines.len();
    for (i, line) in lines.iter().enumerate() {
        b.text(pos2(x, r.top() + i as f32 * lh), s, Color32::WHITE, bold, line, al);
        if cut && i + 1 == n && e.more != 0 {
            b.text(pos2(x + text_w(line, s, bold), r.top() + i as f32 * lh), s, white(0.7), bold, "… more", 0);
        }
    }
}

// ---------------------------------------------------------------- the overlay over the editor's video

/// Percent of the frame a zone covers, as text: the band's thickness when it spans the frame, else w x h.
pub fn zone_pct(z: [f32; 4], refsz: [f32; 2]) -> String {
    let (w, h) = ((z[2] - z[0]) / refsz[0] * 100.0, (z[3] - z[1]) / refsz[1] * 100.0);
    if w >= 90.0 {
        format!("{h:.0}% tall")
    } else if h >= 90.0 {
        format!("{w:.0}% wide")
    } else {
        format!("{w:.0}% x {h:.0}%")
    }
}

/// Intersection of every `pg == "full"` platform's safe rectangle with this platform's aspect, as
/// fractions of the frame, plus the largest device insets: the "worst case" safe area.
pub fn worst_safe(d: &Data, plat: &Platform) -> ([f32; 4], Vec<String>) {
    let mut r = [0.0f32, 0.0, 1.0, 1.0];
    let mut names = Vec::new();
    for p in d.platforms.iter().filter(|p| p.a == plat.a && p.pg == plat.pg) {
        let l = p.lay.get("phone").or_else(|| p.lay.values().next());
        if let Some((l, s)) = l.and_then(|l| l.safe.map(|s| (l, s))) {
            r = [r[0].max(s[0] / l.refsz[0]), r[1].max(s[1] / l.refsz[1]), r[2].min(s[2] / l.refsz[0]), r[3].min(s[3] / l.refsz[1])];
            names.push(p.n.clone());
        }
    }
    if plat.pg == "full" {
        for dev in d.devices.iter().filter(|v| v.k == "phone") {
            r[1] = r[1].max(dev.st / dev.h);
            r[3] = r[3].min(1.0 - (dev.home + dev.nav) / dev.h);
        }
    }
    (r, names)
}

/// Safe-zone or UI-preview overlay for `plat` in `frame` (the platform's own aspect rectangle).
/// `video` is the whole video rectangle; what the platform crops away is dimmed.
pub fn overlay(d: &Data, plat: &Platform, dev: &Device, mode: Mode, worst: bool, video: Rect, frame: Rect, cx: &Ctx) -> Vec<Op> {
    let lay = plat.layout(dev);
    let k = frame.width() / lay.refsz[0];
    let size = vec2(frame.width() / k, frame.height() / k);
    let map = Map::new(lay.refsz, size, frame.min, k);
    let mut b = B::new();
    // dim what the platform crops away
    for r in [
        Rect::from_min_max(video.min, pos2(video.max.x, frame.min.y)),
        Rect::from_min_max(pos2(video.min.x, frame.max.y), video.max),
        Rect::from_min_max(pos2(video.min.x, frame.min.y), pos2(frame.min.x, frame.max.y)),
        Rect::from_min_max(pos2(frame.max.x, frame.min.y), pos2(video.max.x, frame.max.y)),
    ] {
        if r.width() > 0.5 && r.height() > 0.5 {
            b.rect(r, 0.0, Color32::from_black_alpha(150));
        }
    }
    let font = (10.0 * k.max(0.6)).max(9.0);
    let mut title = format!("{} · {}", plat.n, mode.name());
    if mode == Mode::Safe && plat.pg == "full" && dev.k == "phone" {
        title = format!("{title} · {} (status bar, {})", dev.n, if dev.cut == "none" { "no cut-out" } else { &dev.cut });
    }
    match mode {
        Mode::Ui | Mode::Device => {
            draw_ui(&mut b, lay, &map, cx, false);
        }
        Mode::Safe => {
            let (red, amber, green) = (rgba(235, 60, 60, 60), rgba(255, 180, 40, 55), rgba(80, 220, 120, 255));
            if worst {
                let (s, names) = worst_safe(d, plat);
                let sr = Rect::from_min_max(
                    pos2(frame.min.x + s[0] * frame.width(), frame.min.y + s[1] * frame.height()),
                    pos2(frame.min.x + s[2] * frame.width(), frame.min.y + s[3] * frame.height()),
                );
                for r in [
                    Rect::from_min_max(frame.min, pos2(frame.max.x, sr.min.y)),
                    Rect::from_min_max(pos2(frame.min.x, sr.max.y), frame.max),
                    Rect::from_min_max(pos2(frame.min.x, sr.min.y), pos2(sr.min.x, sr.max.y)),
                    Rect::from_min_max(pos2(sr.max.x, sr.min.y), pos2(frame.max.x, sr.max.y)),
                ] {
                    b.rect(r, 0.0, red);
                }
                b.frame(sr, 0.0, 1.6, green);
                let pct = format!("Worst case of {}: top {:.0}%, bottom {:.0}%, left {:.0}%, right {:.0}%", names.len(), s[1] * 100.0, (1.0 - s[3]) * 100.0, s[0] * 100.0, (1.0 - s[2]) * 100.0);
                b.clip(Some(frame));
                b.text(pos2(sr.min.x + 4.0, sr.min.y + 3.0), font, green, true, &pct, 0);
                b.clip(None);
                title = format!("Worst case · {}", plat.n);
            } else {
                for z in &lay.zones {
                    let r = map.rect(z.r);
                    match z.k.as_str() {
                        "d" => {
                            b.rect(r, 0.0, red);
                            b.frame(r, 0.0, 1.0, rgba(235, 60, 60, 140));
                        }
                        "c" => {
                            b.rect(r, 0.0, amber);
                            b.frame(r, 0.0, 1.0, rgba(255, 180, 40, 130));
                        }
                        "s" => b.frame(r, 0.0, 1.6, green),
                        _ => dashed(&mut b, r, green.gamma_multiply(0.8)),
                    }
                    if z.k != "s" && r.height() > font * 1.2 && r.width() > font * 4.0 {
                        b.clip(Some(r));
                        let t = format!("{} · {}", z.l, zone_pct(z.r, lay.refsz));
                        b.text(pos2(r.min.x + 3.0, r.min.y + 2.0), font, white(0.9), false, &t, 0);
                        b.clip(None);
                    }
                }
            }
            // the phone's own bars
            if plat.pg == "full" && dev.k == "phone" {
                let amber2 = rgba(255, 180, 40, 70);
                let top = dev.st / dev.h * frame.height();
                let bot = (dev.home + dev.nav) / dev.h * frame.height();
                let band = Rect::from_min_max(frame.min, pos2(frame.max.x, frame.min.y + top));
                b.rect(band, 0.0, amber2);
                if bot > 0.5 {
                    b.rect(Rect::from_min_max(pos2(frame.min.x, frame.max.y - bot), frame.max), 0.0, amber2);
                }
                b.clip(Some(band));
                let _ = band;
                b.clip(None);
            }
        }
    }
    b.text(frame.min + vec2(5.0, 5.0 + if mode == Mode::Safe { (dev.st / dev.h * frame.height()).max(0.0) } else { 0.0 }), font, white(0.95), false, &title, 0);
    // where the numbers come from, and how to refresh them
    let note = format!("Measured {}{}. Outdated? Check again: Settings > Social", plat.ver, if plat.st.comments.src == "est" { " (some parts estimated)" } else { "" });
    b.clip(Some(frame));
    let note = ellipsize(&note, font * 0.85, false, frame.width() * 0.55);
    b.text(frame.min + vec2(5.0, 5.0 + font * 1.4 + if mode == Mode::Safe { (dev.st / dev.h * frame.height()).max(0.0) } else { 0.0 }), font * 0.85, white(0.7), false, &note, 0);
    b.clip(None);
    b.ops
}

fn dashed(b: &mut B, r: Rect, col: Color32) {
    let (d, g) = (6.0, 5.0);
    for (a, c) in [(r.left_top(), r.right_top()), (r.right_top(), r.right_bottom()), (r.right_bottom(), r.left_bottom()), (r.left_bottom(), r.left_top())] {
        let len = (c - a).length();
        let dir = (c - a) / len.max(0.001);
        let mut t = 0.0;
        while t < len {
            b.line(a + dir * t, a + dir * (t + d).min(len), 1.2, col);
            t += d + g;
        }
    }
}
