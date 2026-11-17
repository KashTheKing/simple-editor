//! ---- ws:gallery-audio ----
//! Titles / Templates card pictures. Each template is rendered off the UI thread (one low-priority
//! worker, `media::bg`): an HTML template through the HTML engine (`html::render_frame`, the fast Blitz
//! renderer first, Edge as its fallback), any other template placed in a scratch project and drawn by the
//! CPU compositor at its first meaningful frame. Pictures are cached on disk keyed by the template's
//! content (`<cache>/gallery-thumbs/<key>.rgba`), so the second open reads a file instead of rendering.
//! Until a picture arrives (or when one cannot be made) the card shows its type icon, never a blank tile.

use super::*;
use crate::ui::app::thumbs::ThumbSource;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;

/// Rendered picture size (twice the small card, so a large card stays sharp).
pub(crate) const TW: u32 = 192;
pub(crate) const TH: u32 = 108;
/// Bump when the picture changes for the same template (the key includes it).
const VERSION: u32 = 2;

#[derive(Default)]
struct State {
    /// Keys handed to a worker, not back yet.
    pending: HashSet<u64>,
    /// Keys that could not be drawn, and how often: tried twice, the type icon stays for this session.
    failed: HashMap<u64, u8>,
    rx: Vec<mpsc::Receiver<(u64, Option<Frame>)>>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::default();
}

/// The content key of a template's picture.
pub(crate) fn key(tpl: &crate::settings::Template) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::hash::DefaultHasher::new();
    (VERSION, TW, TH, &tpl.json).hash(&mut h);
    h.finish()
}

pub(crate) fn cache_dir() -> PathBuf {
    crate::settings::Settings::cache_dir().join("gallery-thumbs")
}

fn file(dir: &Path, k: u64) -> PathBuf {
    dir.join(format!("{k:016x}.rgba"))
}

/// The cached picture for key `k`: width, height (u32 LE) then RGBA.
pub(crate) fn read_cached(dir: &Path, k: u64) -> Option<Frame> {
    let b = std::fs::read(file(dir, k)).ok()?;
    let w = u32::from_le_bytes(b.get(0..4)?.try_into().ok()?);
    let h = u32::from_le_bytes(b.get(4..8)?.try_into().ok()?);
    let px = b.get(8..)?;
    (px.len() == (w * h * 4) as usize).then(|| {
        let mut f = Frame::new(w, h);
        f.rgba.copy_from_slice(px);
        f
    })
}

fn write_cached(dir: &Path, k: u64, f: &Frame) {
    let mut b = Vec::with_capacity(8 + f.rgba.len());
    b.extend_from_slice(&f.width.to_le_bytes());
    b.extend_from_slice(&f.height.to_le_bytes());
    b.extend_from_slice(&f.rgba);
    if std::fs::create_dir_all(dir).is_ok() {
        let _ = std::fs::write(file(dir, k), b);
    }
}

/// How much of a frame shows: the sum of each pixel's brightest channel times its alpha (a half-faded
/// title counts half).
fn coverage(f: &Frame) -> u64 {
    f.rgba.chunks_exact(4).map(|p| p[0].max(p[1]).max(p[2]) as u64 * p[3] as u64 / 255).sum()
}

/// The part `(x, y, w, h)` of `f` scaled (box-averaged) into TW x TH, letterboxed when its shape differs.
fn fit(f: &Frame, (cx, cy, cw, ch): (u32, u32, u32, u32)) -> Frame {
    let mut out = Frame::new(TW, TH);
    if cw == 0 || ch == 0 || f.rgba.len() != (f.width * f.height * 4) as usize {
        return out;
    }
    let s = (TW as f32 / cw as f32).min(TH as f32 / ch as f32);
    let (w, h) = (((cw as f32 * s) as u32).clamp(1, TW), ((ch as f32 * s) as u32).clamp(1, TH));
    let (ox, oy) = ((TW - w) / 2, (TH - h) / 2);
    for y in 0..h {
        let (y0, y1) = (cy + (y as f32 / s) as u32, cy + ((y + 1) as f32 / s).ceil() as u32);
        for x in 0..w {
            let (x0, x1) = (cx + (x as f32 / s) as u32, cx + ((x + 1) as f32 / s).ceil() as u32);
            let mut acc = [0u32; 4];
            let mut n = 0;
            for sy in y0..y1.min(f.height).max(y0 + 1).min(f.height) {
                for sx in x0..x1.min(f.width).max(x0 + 1).min(f.width) {
                    let i = ((sy * f.width + sx) * 4) as usize;
                    for c in 0..4 {
                        acc[c] += f.rgba[i + c] as u32;
                    }
                    n += 1;
                }
            }
            let di = (((oy + y) * TW + ox + x) * 4) as usize;
            for c in 0..4 {
                out.rgba[di + c] = (acc[c] / n.max(1)) as u8;
            }
        }
    }
    out
}

/// The card-shaped part of `f` around what is drawn: a title that is a small line at the bottom of the
/// frame is shown big enough to read, a full-frame design stays whole. At least a third of the frame wide.
fn framing(f: &Frame) -> (u32, u32, u32, u32) {
    let (fw, fh) = (f.width, f.height);
    let (mut x0, mut y0, mut x1, mut y1) = (fw, fh, 0, 0);
    for (i, p) in f.rgba.chunks_exact(4).enumerate() {
        if p[3] > 16 && p[0].max(p[1]).max(p[2]) > 16 {
            let (x, y) = (i as u32 % fw, i as u32 / fw);
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
        }
    }
    if x1 <= x0 || y1 <= y0 {
        return (0, 0, fw, fh);
    }
    let aspect = TW as f32 / TH as f32;
    let mut w = ((x1 - x0) as f32 * 1.25).max(fw as f32 / 3.0);
    let mut h = (y1 - y0) as f32 * 1.25;
    if w / h < aspect {
        w = h * aspect;
    } else {
        h = w / aspect;
    }
    let (w, h) = ((w as u32).min(fw), (h as u32).min(fh));
    let cx = ((x0 + x1) / 2).saturating_sub(w / 2).min(fw - w);
    let cy = ((y0 + y1) / 2).saturating_sub(h / 2).min(fh - h);
    (cx, cy, w, h)
}

/// Draw one template's picture (blocking: a worker thread or a test only). None = nothing visible.
pub(crate) fn render(tpl: &crate::settings::Template) -> Option<Frame> {
    let (clips, assets) = crate::engine::presets::decode_template(tpl)?;
    // an HTML template: its page through the HTML engine, at its own thumbnail moment
    if let Some(h) = assets.iter().find_map(|a| a.html.as_ref()) {
        let page = crate::engine::html::parse(&h.src);
        let (vw, _) = crate::engine::html::viewport(&h.src, h.width);
        let fields: Vec<(String, String)> = h.fields.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        let scale = (TW * 2) as f64 / vw.max(1) as f64;
        let f = crate::engine::html::render_frame(&h.src, &fields, scale, page.thumb_at()).ok()?;
        return (coverage(&f) > 0).then(|| fit(&f, framing(&f)));
    }
    // ws:animator-templates: with its track order and transitions (text stays over footage)
    let mut p = crate::model::Project::new();
    match crate::engine::templates::TemplateData::decode(&tpl.json) {
        Some(d) => drop(crate::engine::templates::apply(&mut p, d, 0.0, &[])),
        None => drop(p.place_clips(clips, assets, 0.0)),
    }
    let end = p.duration();
    if end <= 0.0 {
        return None;
    }
    // the first meaningful frame: the earliest of nine moments showing most of what the busiest one does
    // (a title that types or fades in is caught once it is (nearly) all there, not on its first letter),
    // drawn at 4x the card so a small title can be framed and still be sharp
    let (rw, rh) = (TW * 4, TH * 4);
    let (mut comp, mut text) = (crate::engine::compose::Compositor::new(), crate::engine::text::TextRasterizer::new());
    let mut pool = crate::media::DecoderPool::new(crate::media::Backend::Auto);
    let shots: Vec<(u64, Frame)> = (0..9)
        .map(|i| {
            let mut f = Frame::default();
            let t = (end * i as f64 / 8.0).min(end - 1e-3);
            comp.render(&p, t, rw, rh, &mut pool, &mut text, &mut f);
            (coverage(&f), f)
        })
        .collect();
    let best = shots.iter().map(|s| s.0).max().unwrap_or(0);
    if best == 0 {
        return None;
    }
    let f = shots.into_iter().find(|s| s.0 * 10 >= best * 8).map(|s| s.1)?;
    Some(fit(&f, framing(&f)))
}

/// The picture for `tpl`: from the disk cache (`true`), else rendered and cached (`false`).
pub(crate) fn picture(tpl: &crate::settings::Template, dir: &Path) -> Option<(Frame, bool)> {
    let k = key(tpl);
    if let Some(f) = read_cached(dir, k) {
        return Some((f, true));
    }
    let f = render(tpl)?;
    write_cached(dir, k, &f);
    Some((f, false))
}

/// Every Titles / Templates card's template by name, as the Gallery lists them.
pub(crate) fn templates(settings: &crate::settings::Settings) -> Vec<crate::settings::Template> {
    // ws:animator-templates: the built-in animation templates too (a saved one of the same name comes later and wins)
    crate::engine::presets::builtin_titles()
        .into_iter()
        .chain(crate::engine::templates::builtin_cards().iter().cloned())
        .chain(settings.templates.iter().cloned())
        .collect()
}

/// Called while the Titles or Templates tab is on screen: hand the cards without a picture to a worker,
/// take back what it finished, and upload those as textures.
pub(super) fn poll(app: &mut App, ctx: &egui::Context) {
    let todo: Vec<(u64, crate::settings::Template)> = STATE.with(|s| {
        let s = s.borrow();
        templates(&app.settings)
            .into_iter()
            .map(|t| (key(&t), t))
            .filter(|(k, _)| {
                !s.pending.contains(k)
                    && s.failed.get(k).is_none_or(|n| *n < 2)
                    && !crate::ui::gallery::has_thumbnail(&ThumbSource::Template(*k))
            })
            .collect()
    });
    if !todo.is_empty() {
        let (tx, rx) = mpsc::channel();
        let dir = cache_dir();
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.pending.extend(todo.iter().map(|(k, _)| *k));
            s.rx.push(rx);
        });
        std::thread::spawn(move || {
            crate::media::bg::lower_thread();
            for (k, t) in todo {
                crate::media::bg::pause_point(|| false);
                let f = std::panic::catch_unwind(|| picture(&t, &dir)).ok().flatten().map(|(f, _)| f);
                if tx.send((k, f)).is_err() {
                    return;
                }
            }
        });
    }
    let mut got = Vec::new();
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.rx.retain(|rx| loop {
            match rx.try_recv() {
                Ok(r) => got.push(r),
                Err(mpsc::TryRecvError::Empty) => return true,
                Err(mpsc::TryRecvError::Disconnected) => return false,
            }
        });
        for (k, f) in &got {
            s.pending.remove(k);
            if f.is_none() {
                *s.failed.entry(*k).or_default() += 1;
            }
        }
        if !s.pending.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(150));
        }
    });
    for (k, f) in got {
        let Some(f) = f else { continue };
        let img = egui::ColorImage::from_rgba_unmultiplied([f.width as usize, f.height as usize], &f.rgba);
        let tex = ctx.load_texture(format!("gallery_tpl_{k:016x}"), img, egui::TextureOptions::LINEAR);
        crate::ui::gallery::set_thumbnail(ThumbSource::Template(k), tex.id(), [f.width, f.height]);
        app.gallery_textures.push(tex);
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A saved (non-title) template: a red shape that fades in over its first second.
    fn shape_template() -> crate::settings::Template {
        let mut c = Clip::new(1, crate::model::ClipKind::Shape, "Box", 0.0, 3.0);
        if let Some(s) = c.shape.as_mut() {
            s.fill = [220, 40, 40, 255];
        }
        c.opacity.set_at(0.0, 0.0);
        c.opacity.set_at(1.0, 1.0);
        let json = serde_json::json!({"clips": [c], "assets": []}).to_string();
        crate::settings::Template { name: "Red box".into(), json }
    }

    #[test]
    fn every_title_and_template_card_gets_a_picture_and_the_second_open_is_a_cache_hit() {
        let dir = std::env::temp_dir().join(format!("se-tpl-thumbs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let all: Vec<_> = crate::engine::presets::builtin_titles().into_iter().chain([shape_template()]).collect();
        assert!(all.len() >= 2);
        for t in &all {
            let (f, hit) = picture(t, &dir).unwrap_or_else(|| panic!("{}: no picture", t.name));
            assert!(!hit, "{}: first open renders", t.name);
            assert_eq!((f.width, f.height), (TW, TH));
            assert!(coverage(&f) > 20 * 255, "{}: the picture is blank", t.name);
            let (again, hit) = picture(t, &dir).unwrap();
            assert!(hit, "{}: second open must read the cache", t.name);
            assert_eq!(again.rgba, f.rgba);
        }
        // the key follows the content: an edited template is a new picture
        let mut t = shape_template();
        t.json = t.json.replace("220", "20");
        assert_ne!(key(&t), key(&shape_template()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_first_meaningful_frame_skips_a_fade_in() {
        let f = render(&shape_template()).unwrap();
        // fully faded in (the fade ends at 1 s of 3): the shape is bright red, not a dim first frame
        let max_r = f.rgba.chunks_exact(4).map(|p| p[0]).max().unwrap();
        assert!(max_r > 180, "picked a faded frame: max red {max_r}");
    }
}
