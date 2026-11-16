//! One loaded page: parse + style + layout once, then any number of frames at any time `t`.
//!
//! Time: Blitz hands `resolve(now)` straight to Stylo's animation machinery, so any `t` can be asked for,
//! but Stylo advances an animation by ONE iteration per `resolve` however far `now` jumped. `seek`
//! therefore walks the clock forward in steps no longer than half the shortest animation iteration; an
//! earlier time than the clock reloads the page (the iteration counters only count up).

use anyrender::render_to_buffer;
use anyrender_vello_cpu::VelloCpuImageRenderer;
use blitz_dom::DocumentConfig;
use blitz_html::HtmlDocument;
use blitz_paint::paint_scene;
use blitz_traits::net::{Bytes, NetHandler, NetProvider, Request};
use blitz_traits::shell::{ColorScheme, Viewport};
use serde_json::{json, Value};
use std::sync::Arc;

/// Decodes `data:` URLs and reads `file:` URLs, synchronously. Nothing leaves the machine: anything else
/// is answered with an empty body (a dropped request would leave a "critical" resource pending forever).
struct Local;

fn b64(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for &c in s {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => continue,
        } as u32;
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

fn pct(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let (mut out, mut i) = (Vec::with_capacity(b.len()), 0);
    while i < b.len() {
        if b[i] == b'%' && b.len() >= i + 3 {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

impl NetProvider for Local {
    fn fetch(&self, _doc: usize, req: Request, handler: Box<dyn NetHandler>) {
        let url = req.url.to_string();
        match req.url.scheme() {
            "data" => match url[5..].split_once(',') {
                Some((head, body)) => {
                    let bytes = if head.ends_with(";base64") { b64(&pct(body)) } else { pct(body) };
                    handler.bytes(url, Bytes::from(bytes));
                }
                None => handler.bytes(url, Bytes::new()),
            },
            "file" => match req.url.to_file_path().ok().and_then(|p| std::fs::read(p).ok()) {
                Some(b) => handler.bytes(url, Bytes::from(b)),
                None => handler.bytes(url, Bytes::new()),
            },
            _ => handler.bytes(url, Bytes::new()),
        }
    }
}

/// Stylo ends an iteration when `now > end`, a browser's timeline when `now >= end`: nudge the clock so an
/// exact boundary (t = 1.0 for a 0.5 s x 2 animation) lands on the browser's side.
const EPS: f64 = 1e-5;

/// Families Windows names after their weight ("Arial Black", "Segoe UI Black"): the font stack matches the
/// typographic family ("Arial") instead, so a page asking for the name gets the regular face. Each one found
/// installed and mentioned by the page gets an `@font-face` pointing at its file.
const WEIGHT_FAMILIES: [(&str, &str); 6] = [
    ("Arial Black", "ariblk.ttf"),
    ("Segoe UI Black", "seguibl.ttf"),
    ("Segoe UI Semibold", "seguisb.ttf"),
    ("Segoe UI Semilight", "segoeuisl.ttf"),
    ("Segoe UI Light", "segoeuil.ttf"),
    ("Bahnschrift", "bahnschrift.ttf"),
];

fn with_weight_families(html: &str) -> std::borrow::Cow<'_, str> {
    let lower = html.to_ascii_lowercase();
    let dir = std::env::var_os("WINDIR").map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows")).join("Fonts");
    let mut css = String::new();
    for (family, file) in WEIGHT_FAMILIES {
        let p = dir.join(file);
        if lower.contains(&family.to_ascii_lowercase()) && p.is_file() {
            let url = p.display().to_string().replace('\\', "/");
            css.push_str(&format!("@font-face{{font-family:\"{family}\";src:url(\"file:///{url}\")}}"));
        }
    }
    if css.is_empty() {
        return html.into();
    }
    // inside <head> when there is one (anything before the doctype would put the page in quirks mode)
    let at = lower.find("<head").and_then(|i| lower[i..].find('>').map(|j| i + j + 1)).unwrap_or(0);
    format!("{}<style>{css}</style>{}", &html[..at], &html[at..]).into()
}

/// Blitz lays text out at the device scale (hinted advances), so below scale 1 a page breaks its lines
/// differently from a browser's, and painting a scale-1 layout smaller does not scale the text. Pictures
/// below scale 1 are therefore drawn at 1 and averaged down (`box_down`): the same layout as a browser.
fn eff_scale(scale: f64) -> f64 {
    scale.max(1.0)
}

fn build(html: &str, base_dir: &str, css: (u32, u32), eff: f64) -> HtmlDocument {
    let epx = (((css.0 as f64 * eff).round() as u32).max(1), ((css.1 as f64 * eff).round() as u32).max(1));
    let html = &*with_weight_families(html);
    let base_url = if base_dir.is_empty() {
        // a data: base cannot resolve a relative link and Blitz panics on one: use a folder that is not there
        Some("file:///se-html-no-base/".to_string())
    } else {
        Some(format!("file:///{}/", base_dir.replace('\\', "/").trim_matches('/')))
    };
    let mut doc = HtmlDocument::from_html(
        html,
        DocumentConfig {
            base_url,
            net_provider: Some(Arc::new(Local)),
            viewport: Some(Viewport::new(epx.0, epx.1, eff as f32, ColorScheme::Light)),
            ..Default::default()
        },
    );
    doc.resolve(0.0);
    doc
}

pub struct Page {
    doc: HtmlDocument,
    src: String,
    base_dir: String,
    pub css: (u32, u32),
    px: (u32, u32),
    scale: f64,
    clock: f64,
    /// Shortest animation iteration (s) at load; 0 = no animation.
    period: f64,
    /// Seconds after which no finite animation is running (INFINITY if one repeats forever), 0 = none.
    pub ends: f64,
    pub animated: bool,
    /// The pointer so far (CSS px), for replay after a reload.
    pointer: Option<(f32, f32, bool)>,
    scroll: f64,
    vars: Vec<(String, String)>,
}

fn px_of(css: (u32, u32), scale: f64) -> (u32, u32) {
    (((css.0 as f64 * scale).round() as u32).max(1), ((css.1 as f64 * scale).round() as u32).max(1))
}

impl Page {
    pub fn load(html: &str, base_dir: &str, css_w: u32, css_h: u32, scale: f64) -> Page {
        let css = (css_w.max(1), css_h.max(1));
        let px = px_of(css, scale);
        let doc = build(html, base_dir, css, eff_scale(scale));
        let mut p = Page {
            doc,
            src: html.to_string(),
            base_dir: base_dir.to_string(),
            css,
            px,
            scale,
            clock: 0.0,
            period: 0.0,
            ends: 0.0,
            animated: false,
            pointer: None,
            scroll: 0.0,
            vars: Vec::new(),
        };
        p.scan_animations();
        p
    }

    /// Shortest `animation-duration`, and when the last finite animation is over, over every element.
    fn scan_animations(&mut self) {
        let (mut min, mut end) = (f64::MAX, 0.0f64);
        for (_, n) in self.doc.tree().iter() {
            if let Some(cv) = n.primary_styles() {
                let b = cv.get_ui();
                for (i, name) in b.animation_name_iter().enumerate() {
                    if name.is_none() {
                        continue;
                    }
                    let d = b.animation_duration_mod(i).seconds() as f64;
                    if d <= 0.0 {
                        continue;
                    }
                    let delay = (b.animation_delay_mod(i).seconds() as f64).max(0.0);
                    let count = b.animation_iteration_count_mod(i).0 as f64;
                    min = min.min(d);
                    end = end.max(if count.is_infinite() { f64::INFINITY } else { delay + d * count.max(0.0) });
                }
            }
        }
        self.animated = min != f64::MAX;
        self.period = if self.animated { min } else { 0.0 };
        self.ends = if self.animated { end } else { 0.0 };
    }

    pub fn content_height(&self) -> f32 {
        self.doc.root_element().final_layout().size.height / eff_scale(self.scale) as f32
    }

    pub fn set_var(&mut self, name: &str, value: &str) {
        match self.vars.iter_mut().find(|(n, _)| n == name) {
            Some(v) => v.1 = value.to_string(),
            None => self.vars.push((name.to_string(), value.to_string())),
        }
        let root = self.doc.root_element().id;
        self.doc.set_style_property(root, name, value);
    }

    pub fn scroll_to(&mut self, y: f64) {
        self.scroll = y;
        self.doc.set_viewport_scroll(blitz_dom::Point { x: 0.0, y: y * eff_scale(self.scale) });
    }

    /// The pointer at (x, y) CSS px, button down or up: `:hover` and `:active` (and the transitions they
    /// start) apply from the current clock on.
    pub fn pointer(&mut self, x: f32, y: f32, down: bool) {
        let was_down = self.pointer.is_some_and(|p| p.2);
        let e = eff_scale(self.scale) as f32;
        self.doc.set_hover_to(x * e, y * e);
        if down && !was_down {
            self.doc.active_node();
        } else if !down && was_down {
            self.doc.unactive_node();
        }
        self.pointer = Some((x, y, down));
    }

    fn reload(&mut self) {
        self.doc = build(&self.src, &self.base_dir, self.css, eff_scale(self.scale));
        self.clock = 0.0;
        let vars = self.vars.clone();
        for (n, v) in &vars {
            let root = self.doc.root_element().id;
            self.doc.set_style_property(root, n, v);
        }
        if self.scroll != 0.0 {
            self.doc.set_viewport_scroll(blitz_dom::Point { x: 0.0, y: self.scroll * eff_scale(self.scale) });
        }
    }

    /// Move the animation clock to `t` (>= 0) and restyle. Going back reloads (and loses the pointer:
    /// the caller replays it from its own history).
    pub fn seek(&mut self, t: f64) {
        let t = t.max(0.0);
        if t < self.clock - 1e-9 {
            self.reload();
            self.pointer = None;
        }
        let step = if self.period > 0.0 { (self.period * 0.5).max(1.0 / 240.0) } else { f64::MAX };
        let mut guard = 0;
        while self.clock < t && guard < 100_000 {
            self.clock = (self.clock + step).min(t);
            self.doc.resolve(self.clock + EPS);
            guard += 1;
        }
        if guard == 0 {
            self.doc.resolve(t + EPS);
        }
    }

    /// Straight-alpha RGBA of the current state (call `seek` first). `(width, height, bytes)`.
    pub fn paint(&mut self) -> (u32, u32, Vec<u8>) {
        let eff = eff_scale(self.scale);
        let (ew, eh) = (((self.css.0 as f64 * eff).round() as u32).max(1), ((self.css.1 as f64 * eff).round() as u32).max(1));
        let doc = &mut self.doc;
        let mut buf = render_to_buffer::<VelloCpuImageRenderer, _>(|scene| paint_scene(scene, doc.as_mut(), eff, ew, eh, 0, 0), ew, eh);
        let (w, h) = self.px;
        if (ew, eh) != (w, h) {
            buf = box_down(&buf, ew, eh, w, h);
        }
        unpremultiply(&mut buf);
        (w, h, buf)
    }

    /// The part `rect` (x, y, w, h CSS px, anywhere on the page) drawn at `scale` picture px per CSS px:
    /// text and vector art are sharp at any zoom. A page of its own is laid out for it.
    pub fn region(&mut self, rect: [f64; 4], scale: f64, t: f64) -> (u32, u32, Vec<u8>) {
        let (cw, _) = self.css;
        let (pw, ph) = ((rect[2] * scale).round().max(1.0) as u32, (rect[3] * scale).round().max(1.0) as u32);
        let eff = eff_scale(scale);
        // laid out like the page (same width) at the paint scale, from the part's corner
        let mut doc = build(&self.src, &self.base_dir, (cw, (rect[1] + rect[3]).max(1.0) as u32), eff);
        let root = doc.root_element().id;
        for (n, v) in &self.vars {
            doc.set_style_property(root, n, v);
        }
        doc.resolve(t.max(0.0) + EPS);
        // the part's corner goes to (0, 0) by scrolling the viewport there: paint_scene's x/y offsets are
        // ADDED to the page's position (they move the page right / down), so they cannot do this
        doc.set_viewport_scroll(blitz_dom::Point { x: rect[0].max(0.0), y: rect[1].max(0.0) });
        let (ew, eh) = (((rect[2] * eff).round().max(1.0)) as u32, ((rect[3] * eff).round().max(1.0)) as u32);
        let mut buf = render_to_buffer::<VelloCpuImageRenderer, _>(|scene| paint_scene(scene, doc.as_mut(), eff, ew, eh, 0, 0), ew, eh);
        if (ew, eh) != (pw, ph) {
            buf = box_down(&buf, ew, eh, pw, ph);
        }
        unpremultiply(&mut buf);
        (pw, ph, buf)
    }

    /// Everything on the page a mouse can go to: links, buttons, fields, role=button..., plus headings and
    /// short figures as landmarks. CSS px from the page's top-left.
    pub fn elements(&self) -> Vec<Value> {
        let s = eff_scale(self.scale) as f32;
        let mut out = Vec::new();
        let attr = |n: &blitz_dom::Node, k: &str| -> String {
            n.element_data()
                .and_then(|e| e.attrs().iter().find(|a| &*a.name.local == k).map(|a| a.value.to_string()))
                .unwrap_or_default()
        };
        let clean = |s: String, max: usize| -> String { s.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(max).collect() };
        for (_, n) in self.doc.tree().iter() {
            let Some(e) = n.element_data() else { continue };
            let tag = e.name.local.as_ref();
            let role = attr(n, "role");
            let interactive = (tag == "a" && !attr(n, "href").is_empty())
                || matches!(tag, "button" | "input" | "select" | "textarea")
                || matches!(role.as_str(), "button" | "link" | "tab" | "menuitem");
            let landmark = matches!(tag, "h1" | "h2" | "h3" | "h4");
            if !interactive && !landmark {
                continue;
            }
            let lay = n.final_layout();
            let p = n.absolute_position(0.0, 0.0);
            let (x, y, w, h) = (p.x / s, p.y / s, lay.size.width / s, lay.size.height / s);
            if w < 4.0 || h < 4.0 {
                continue;
            }
            if let Some(cv) = n.primary_styles() {
                if cv.get_effects().opacity == 0.0 {
                    continue;
                }
            }
            if landmark {
                if n.children.iter().any(|&c| self.doc.get_node(c).is_some_and(|c| c.element_data().is_some())) {
                    continue;
                }
                let t = clean(n.text_content(), 120);
                if t.is_empty() {
                    continue;
                }
                out.push(json!({"tag": tag, "role": "text", "text": t, "href": "", "x": x, "y": y, "w": w, "h": h}));
                continue;
            }
            let mut t = clean(n.text_content(), 120);
            for k in ["value", "aria-label", "title", "placeholder"] {
                if t.is_empty() {
                    t = clean(attr(n, k), 120);
                }
            }
            if t.is_empty() {
                t = clean(attr(n, "data-testid"), 120);
            }
            if t.is_empty() {
                t = clean(attr(n, "id"), 120);
            }
            if t.is_empty() {
                t = clean(attr(n, "class").split(' ').next().unwrap_or("").replace(['-', '_'], " "), 120);
            }
            out.push(json!({"tag": tag, "role": role, "text": t, "href": attr(n, "href"), "x": x, "y": y, "w": w, "h": h}));
            if out.len() >= 4400 {
                break;
            }
        }
        out
    }
}

/// Average a premultiplied RGBA8 picture down to `dw` x `dh` (area weights): what a browser's smaller device
/// scale looks like once the layout is the same.
fn box_down(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    let (sw, sh, dw, dh) = (sw as usize, sh as usize, dw as usize, dh as usize);
    let mut out = vec![0u8; dw * dh * 4];
    let (kx, ky) = (sw as f32 / dw as f32, sh as f32 / dh as f32);
    for y in 0..dh {
        let (y0, y1) = (y as f32 * ky, (y as f32 + 1.0) * ky);
        for x in 0..dw {
            let (x0, x1) = (x as f32 * kx, (x as f32 + 1.0) * kx);
            let mut acc = [0f32; 4];
            let mut wsum = 0f32;
            for sy in (y0.floor() as usize)..(y1.ceil() as usize).min(sh) {
                let wy = (y1.min(sy as f32 + 1.0) - y0.max(sy as f32)).max(0.0);
                for sx in (x0.floor() as usize)..(x1.ceil() as usize).min(sw) {
                    let w = wy * (x1.min(sx as f32 + 1.0) - x0.max(sx as f32)).max(0.0);
                    let p = (sy * sw + sx) * 4;
                    for c in 0..4 {
                        acc[c] += src[p + c] as f32 * w;
                    }
                    wsum += w;
                }
            }
            let o = (y * dw + x) * 4;
            if wsum > 0.0 {
                for c in 0..4 {
                    out[o + c] = (acc[c] / wsum + 0.5).min(255.0) as u8;
                }
            }
        }
    }
    out
}

/// vello_cpu hands back premultiplied RGBA8; the app (and PNG) want straight alpha.
pub fn unpremultiply(buf: &mut [u8]) {
    for p in buf.chunks_exact_mut(4) {
        let a = p[3] as u32;
        if a != 0 && a != 255 {
            for c in &mut p[..3] {
                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
}

/// A fast PNG (straight alpha, RGBA8): what the app's PNG-in-MOV writer and decoder expect.
pub fn encode_png(w: u32, h: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(rgba.len() / 4);
    let mut enc = png::Encoder::new(&mut out, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::Fast);
    enc.set_filter(png::Filter::Sub);
    let mut wr = enc.write_header().map_err(|e| e.to_string())?;
    wr.write_image_data(rgba).map_err(|e| e.to_string())?;
    wr.finish().map_err(|e| e.to_string())?;
    Ok(out)
}
