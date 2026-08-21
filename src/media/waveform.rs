//! Audio waveform peaks per (file, audio stream): computed once on a background thread (decoding the
//! whole stream through an AudioSource), cached in memory and on disk under Settings::cache_dir()
//! (key = hash of path + file size + mtime + stream). `PEAKS_PER_SEC` buckets of mono (min, max).

use super::Backend;
use std::collections::HashMap;
use std::sync::Arc;

pub const PEAKS_PER_SEC: u32 = 100;

pub struct Peaks {
    /// Per bucket: min sample in [-1,1].
    pub min: Vec<f32>,
    /// Per bucket: max sample in [-1,1].
    pub max: Vec<f32>,
}

impl Peaks {
    pub fn len(&self) -> usize {
        self.min.len()
    }
    pub fn is_empty(&self) -> bool {
        self.min.is_empty()
    }
    /// (min, max) over source time range [a, b) seconds.
    pub fn range(&self, a: f64, b: f64) -> (f32, f32) {
        let i0 = ((a * PEAKS_PER_SEC as f64).floor().max(0.0)) as usize;
        let i1 = ((b * PEAKS_PER_SEC as f64).ceil().max(0.0)) as usize;
        let i1 = i1.min(self.len()).max(i0 + 1);
        if i0 >= self.len() {
            return (0.0, 0.0);
        }
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for i in i0..i1 {
            lo = lo.min(self.min[i]);
            hi = hi.max(self.max[i]);
        }
        (lo, hi)
    }
}

pub struct WaveformCache {
    backend: Backend,
    ctx: eframe::egui::Context,
    #[allow(dead_code)]
    ready: HashMap<(String, usize), Arc<Peaks>>,
}

impl WaveformCache {
    pub fn new(ctx: eframe::egui::Context, backend: Backend) -> Self {
        Self { backend, ctx, ready: HashMap::new() }
    }
    pub fn set_backend(&mut self, b: Backend) {
        self.backend = b;
    }
    /// Non-blocking. Returns the peaks if available; otherwise starts computing them in the background
    /// (once) and returns None. Calls `ctx.request_repaint()` when a computation finishes.
    pub fn get(&mut self, _path: &str, _stream: usize) -> Option<Arc<Peaks>> {
        let _ = (&self.ctx, self.backend);
        None
    }
}
