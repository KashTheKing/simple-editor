//! Pre-render ("movie mode"): render ranges at full quality into a cache so playback shows exactly what
//! an export would, without re-running the effect chain every frame.
//!
//! Cache: `Settings::cache_dir()/prerender/<hash>.rgba`, one file per second of timeline at project
//! resolution, keyed by a hash of everything affecting the picture in that second. Work happens in small
//! slices on the UI thread (the GPU renderer owns the GL context) or on the CPU compositor without GL.

use crate::media::Frame;
use crate::model::Project;
use std::sync::Arc;

#[derive(Default)]
pub struct PreRender {
    #[allow(dead_code)]
    ranges: Vec<(f64, f64)>,
}

impl PreRender {
    pub fn new() -> Self {
        Self::default()
    }
    /// Mark a range dirty (an edit touched it).
    pub fn invalidate(&mut self, _from: f64, _to: f64) {
        todo!("engine::prerender::PreRender::invalidate")
    }
    /// Queue [a, b) for rendering.
    pub fn request(&mut self, _project: &Project, _a: f64, _b: f64) {
        todo!("engine::prerender::PreRender::request")
    }
    /// Do a slice of work (call once per frame with a time budget). Returns true while busy.
    pub fn tick(&mut self, _project: &Project, _budget_ms: f32) -> bool {
        todo!("engine::prerender::PreRender::tick")
    }
    /// A pre-rendered frame for time t, when the cache holds a valid one.
    pub fn frame(&self, _project: &Project, _t: f64) -> Option<Arc<Frame>> {
        todo!("engine::prerender::PreRender::frame")
    }
    /// Fraction of the requested range that is ready.
    pub fn progress(&self) -> f32 {
        todo!("engine::prerender::PreRender::progress")
    }
    pub fn clear(&mut self) {
        todo!("engine::prerender::PreRender::clear")
    }
}
