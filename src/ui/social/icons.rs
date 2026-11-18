//! The platforms' real icons. Each one is the SVG the platform's own page draws (or Google's Material
//! Symbols for the Android status bar), kept in `library/icons/platforms/<platform>/<icon>.svg` with its
//! source in that folder's README, and rasterised ahead of time into one atlas, `assets/social_icons.png`
//! (`scripts/platform-icons.sh`: white on transparent, logos in colour), so the exe needs no SVG renderer.
//! `atlas.txt` is the atlas order: one "<platform>/<icon> [cells wide]" per line, 16 cells of 72 px a row,
//! the icon drawn 64 px inside its cell. The viewer draws the atlas texture, `raster` samples the decoded
//! atlas: both show the same picture. Names are looked up as "yt/like", "tt/heart", ...

use crate::media::Frame;
use eframe::egui::{self, pos2, Rect};
use std::sync::OnceLock;

const LIST: &str = include_str!("../../../library/icons/platforms/atlas.txt");
const PNG: &[u8] = include_bytes!("../../../assets/social_icons.png");
const CELL: f32 = 72.0;
const PAD: f32 = 4.0;
const COLS: usize = 16;

pub struct Entry {
    pub name: &'static str,
    /// The icon's box in atlas pixels (x0, y0, x1, y1): 64 px tall, 64 px wide per cell it spans.
    pub px: [f32; 4],
}

struct Table {
    entries: Vec<Entry>,
    size: [f32; 2],
}

fn table() -> &'static Table {
    static T: OnceLock<Table> = OnceLock::new();
    T.get_or_init(|| {
        let (mut col, mut row) = (0usize, 0usize);
        let mut entries = Vec::new();
        for line in LIST.lines() {
            let mut it = line.split_whitespace();
            let Some(name) = it.next() else { continue };
            let span: usize = it.next().and_then(|s| s.parse().ok()).unwrap_or(1).clamp(1, COLS);
            if col + span > COLS {
                (col, row) = (0, row + 1);
            }
            let (x, y) = (col as f32 * CELL + PAD, row as f32 * CELL + PAD);
            entries.push(Entry { name, px: [x, y, x + span as f32 * CELL - 2.0 * PAD, y + CELL - 2.0 * PAD] });
            col += span;
        }
        Table { entries, size: [COLS as f32 * CELL, (row + 1) as f32 * CELL] }
    })
}

/// Every icon, in atlas order.
pub fn all() -> &'static [Entry] {
    &table().entries
}

pub fn index(name: &str) -> Option<u16> {
    table().entries.iter().position(|e| e.name == name).map(|i| i as u16)
}

/// Width / height of the icon's box (1 for a square icon, 3 for a three-cell wordmark).
pub fn aspect(i: u16) -> f32 {
    table().entries.get(i as usize).map_or(1.0, |e| (e.px[2] - e.px[0]) / (e.px[3] - e.px[1]))
}

/// The icon's part of the atlas as a 0..1 uv rectangle.
pub fn uv(i: u16) -> Rect {
    let t = table();
    let p = t.entries.get(i as usize).map_or([0.0; 4], |e| e.px);
    Rect::from_min_max(pos2(p[0] / t.size[0], p[1] / t.size[1]), pos2(p[2] / t.size[0], p[3] / t.size[1]))
}

/// The decoded atlas (straight RGBA), for the bitmap renderer.
pub fn atlas() -> &'static Frame {
    static F: OnceLock<Frame> = OnceLock::new();
    F.get_or_init(|| crate::engine::html::decode_png(PNG).unwrap_or_else(|_| Frame::new(1, 1)))
}

/// The atlas as a texture, uploaded once per context (with mipmaps: icons are drawn far smaller than 64 px).
pub fn texture(ctx: &egui::Context) -> egui::TextureId {
    let id = egui::Id::new("social_icon_atlas");
    if let Some(t) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return t.id();
    }
    let f = atlas();
    let img = egui::ColorImage::from_rgba_unmultiplied([f.width as usize, f.height as usize], &f.rgba);
    let opts = egui::TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..egui::TextureOptions::LINEAR };
    let t = ctx.load_texture("social-icons", img, opts);
    let tid = t.id();
    ctx.data_mut(|d| d.insert_temp(id, t));
    tid
}
