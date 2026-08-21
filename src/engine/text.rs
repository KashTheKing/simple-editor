//! Text rasterizer for Text clips: system fonts (fontdb) + ab_glyph. Produces a tight, straight-alpha
//! RGBA image with fill, outline (dilated coverage), shadow (offset + box blur) and optional background box.
//! Output is cached by (style.cache_key(), scale bits). `scale` = canvas px per project px, so a 72 px
//! style at a 960-wide preview of a 1920 project renders at 36 px.

use crate::media::Frame;
use crate::model::TextStyle;
use std::collections::HashMap;
use std::sync::Arc;

pub struct TextRasterizer {
    #[allow(dead_code)]
    cache: HashMap<(u64, u32), Arc<Frame>>,
    families: Vec<String>,
    loaded: bool,
}

impl Default for TextRasterizer {
    fn default() -> Self {
        Self::new()
    }
}

impl TextRasterizer {
    /// Cheap; fonts load lazily (or via `load_system_fonts` from a background thread).
    pub fn new() -> Self {
        Self { cache: HashMap::new(), families: Vec::new(), loaded: false }
    }
    /// Scan system fonts (≈100 ms). Idempotent.
    pub fn load_system_fonts(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        todo!("engine::text::TextRasterizer::load_system_fonts")
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
    pub fn render(&mut self, _style: &TextStyle, _scale: f32) -> Arc<Frame> {
        todo!("engine::text::TextRasterizer::render")
    }
}
