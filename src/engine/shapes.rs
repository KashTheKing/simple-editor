//! Vector shapes and recorded drawings -> RGBA layers (CPU rasteriser; the GPU path samples the result).
//!
//! `render(style, scale, local_t)` draws the shape centred in its own tight layer:
//!  * Rect / Ellipse / Triangle / Polygon / Star: `fill` plus an optional `stroke` outline of
//!    `stroke_width` (project px x scale); rounded corners for Rect via `corner`.
//!  * Line / Arrow: from (-w, -h) to (+w, +h) in layer space, head sized by `corner`.
//!  * Draw: the recorded strokes revealed up to `local_t * draw_rate` (0 = all at once) on the optional
//!    `page` background.
//! Anti-aliased with a coverage rasteriser (no new deps). Cached by `ShapeStyle::cache_key()` + size +
//! reveal bucket.

use crate::media::Frame;
use crate::model::ShapeStyle;
use std::sync::Arc;

#[derive(Default)]
pub struct ShapeRasterizer {
    #[allow(dead_code)]
    cache: std::collections::HashMap<(u64, u32, u32, u32), Arc<Frame>>,
}

impl ShapeRasterizer {
    pub fn new() -> Self {
        Self::default()
    }
    /// Layer size in project px for a style at clip-local time `t` (before `scale`).
    pub fn size(_style: &ShapeStyle, _t: f64) -> (f32, f32) {
        todo!("engine::shapes::ShapeRasterizer::size")
    }
    /// Rasterise at `scale` (canvas px per project px) for clip-local time `t`.
    pub fn render(&mut self, _style: &ShapeStyle, _scale: f32, _t: f64) -> Arc<Frame> {
        todo!("engine::shapes::ShapeRasterizer::render")
    }
}
