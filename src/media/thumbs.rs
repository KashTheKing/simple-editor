//! Thumbnail cache for timeline filmstrips and library previews. Non-blocking: `get`/`texture` return
//! immediately; misses are queued to ONE worker thread (newest request first, so scrubbing/zooming stays
//! responsive) that owns a DecoderPool (backend from settings) and decodes `h`-pixel-tall RGBA thumbnails
//! (aspect kept; images ignore `t`; Sequence clips are not handled here — the timeline draws a label).
//! Results are kept in an LRU (≈512 entries, ~96 px tall ≈ 10 MB) and uploaded lazily as egui textures
//! (`texture`) so panels can paint them directly. `ctx.request_repaint()` when a thumbnail lands.
//! Key = (path, t rounded to 0.5 s buckets, h). `clear()` forgets everything (e.g. after save-over-original).

use crate::media::{Backend, Frame};
use eframe::egui;
use std::sync::Arc;

pub struct ThumbCache {
    #[allow(dead_code)]
    ctx: egui::Context,
    #[allow(dead_code)]
    backend: Backend,
}

impl ThumbCache {
    pub fn new(ctx: egui::Context, backend: Backend) -> Self {
        Self { ctx, backend }
    }
    pub fn set_backend(&mut self, _b: Backend) {
        todo!("media::thumbs::ThumbCache::set_backend")
    }
    /// Thumbnail frame of `path` at source time `t`, `h` px tall. None while computing (request queued).
    pub fn get(&mut self, _path: &str, _t: f64, _h: u32) -> Option<Arc<Frame>> {
        todo!("media::thumbs::ThumbCache::get")
    }
    /// Same as `get` but as an egui texture (created/cached here), with its size in pixels.
    pub fn texture(&mut self, _ctx: &egui::Context, _path: &str, _t: f64, _h: u32) -> Option<(egui::TextureId, [u32; 2])> {
        todo!("media::thumbs::ThumbCache::texture")
    }
    pub fn clear(&mut self) {
        todo!("media::thumbs::ThumbCache::clear")
    }
}
