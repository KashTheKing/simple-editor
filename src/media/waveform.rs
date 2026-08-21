//! Audio waveform peaks per (file, audio stream): computed once on a background thread (decoding the
//! whole stream through an AudioSource), cached in memory and on disk under Settings::cache_dir()
//! (key = hash of path + file size + mtime + stream). `PEAKS_PER_SEC` buckets of mono (min, max).

use super::{AudioSource, Backend, SAMPLE_RATE};
use crate::settings::Settings;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

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

#[derive(Default)]
struct State {
    /// Finished (or failed → empty) peaks, keyed by hash of (path, stream).
    ready: HashMap<u64, Arc<Peaks>>,
    pending: HashSet<u64>,
}

pub struct WaveformCache {
    backend: Backend,
    ctx: eframe::egui::Context,
    state: Arc<Mutex<State>>,
}

/// ponytail: 64-bit hash as the map key so the per-frame lookup allocates nothing — collisions are astronomically unlikely.
fn mem_key(path: &str, stream: usize) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (path, stream).hash(&mut h);
    h.finish()
}

impl WaveformCache {
    pub fn new(ctx: eframe::egui::Context, backend: Backend) -> Self {
        Self { backend, ctx, state: Arc::default() }
    }
    pub fn set_backend(&mut self, b: Backend) {
        self.backend = b;
    }
    /// Forget all in-memory peaks (a file at a known path was rewritten, e.g. Save over the original).
    /// Swaps the state so an in-flight computation finishes into the orphaned map instead of re-inserting
    /// stale peaks; unchanged files just reload from the len+mtime-keyed disk cache.
    pub fn clear(&mut self) {
        self.state = Arc::default();
    }
    /// Non-blocking. Returns the peaks if available; otherwise starts computing them in the background
    /// (once) and returns None. Calls `ctx.request_repaint()` when a computation finishes.
    pub fn get(&mut self, path: &str, stream: usize) -> Option<Arc<Peaks>> {
        let key = mem_key(path, stream);
        let mut st = self.state.lock().ok()?;
        if let Some(p) = st.ready.get(&key) {
            return Some(p.clone());
        }
        if st.pending.insert(key) {
            let (state, ctx, backend, path) = (self.state.clone(), self.ctx.clone(), self.backend, path.to_string());
            std::thread::spawn(move || {
                // ponytail: a decoder panic must leave the key resolved (as empty peaks, same as a file
                // that won't open) — otherwise it stays pending forever and callers wait for good.
                let peaks =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| load_or_compute(&path, stream, backend)))
                        .unwrap_or_else(|_| Peaks { min: Vec::new(), max: Vec::new() });
                if let Ok(mut st) = state.lock() {
                    st.pending.remove(&key);
                    st.ready.insert(key, Arc::new(peaks));
                }
                ctx.request_repaint();
            });
        }
        None
    }
}

fn cache_file(path: &str, stream: usize) -> Option<std::path::PathBuf> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (path, meta.len(), mtime, stream).hash(&mut h);
    Some(Settings::cache_dir().join(format!("{:016x}.peaks", h.finish())))
}

/// Disk cache first; else decode the whole stream (empty Peaks when the stream can't be opened).
fn load_or_compute(path: &str, stream: usize, backend: Backend) -> Peaks {
    let file = cache_file(path, stream);
    if let Some(bytes) = file.as_ref().and_then(|f| std::fs::read(f).ok()) {
        if !bytes.is_empty() && bytes.len() % 8 == 0 {
            let mut p = Peaks { min: Vec::with_capacity(bytes.len() / 8), max: Vec::with_capacity(bytes.len() / 8) };
            for c in bytes.chunks_exact(8) {
                p.min.push(f32::from_le_bytes([c[0], c[1], c[2], c[3]]));
                p.max.push(f32::from_le_bytes([c[4], c[5], c[6], c[7]]));
            }
            return p;
        }
    }
    let Ok(mut src) = super::open_audio(path, stream, backend) else {
        return Peaks { min: Vec::new(), max: Vec::new() };
    };
    let duration = src.duration();
    let peaks = compute_peaks(&mut *src, duration);
    if let Some(f) = file {
        if !peaks.is_empty() {
            let mut bytes = Vec::with_capacity(peaks.len() * 8);
            for (lo, hi) in peaks.min.iter().zip(&peaks.max) {
                bytes.extend_from_slice(&lo.to_le_bytes());
                bytes.extend_from_slice(&hi.to_le_bytes());
            }
            let tmp = f.with_extension("tmp");
            let _ = std::fs::create_dir_all(Settings::cache_dir());
            if std::fs::write(&tmp, bytes).is_ok() {
                let _ = std::fs::rename(&tmp, &f);
            }
        }
    }
    peaks
}

/// Sequentially decode `source` in 4800-frame blocks into 10 ms mono (min, max) buckets. Stops at the
/// first all-zero block at/after `duration` (so an unknown duration of 0 stops at the first silent block).
pub(crate) fn compute_peaks(source: &mut dyn AudioSource, duration: f64) -> Peaks {
    const BLOCK: usize = 4800;
    const BUCKET: usize = (SAMPLE_RATE / PEAKS_PER_SEC) as usize; // 480 frames
    let mut p = Peaks { min: Vec::new(), max: Vec::new() };
    let mut buf = vec![0f32; BLOCK * 2];
    let mut frame: u64 = 0;
    loop {
        let t = frame as f64 / SAMPLE_RATE as f64;
        if t > 24.0 * 3600.0 {
            break; // ponytail: hard cap — never spin forever on a broken source
        }
        source.read_at(t, &mut buf);
        if t >= duration && buf.iter().all(|&x| x == 0.0) {
            break;
        }
        for b in buf.chunks_exact(BUCKET * 2) {
            let (mut lo, mut hi) = (f32::MAX, f32::MIN);
            for s in b.chunks_exact(2) {
                let m = (s[0] + s[1]) * 0.5;
                lo = lo.min(m);
                hi = hi.max(m);
            }
            p.min.push(lo);
            p.max.push(hi);
        }
        frame += BLOCK as u64;
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::ffpipe;

    #[test]
    fn peaks_from_test_clip() {
        let mut src = ffpipe::open_audio(&ffpipe::tests::test_mp4(), 0).unwrap();
        let d = src.duration();
        let p = compute_peaks(&mut *src, d);
        let expect = (d * PEAKS_PER_SEC as f64) as usize;
        assert!(p.len().abs_diff(expect) <= 10, "len {} vs {expect}", p.len());
        let hi = p.max.iter().cloned().fold(f32::MIN, f32::max);
        let lo = p.min.iter().cloned().fold(f32::MAX, f32::min);
        assert!(hi > 0.1 && lo < -0.1, "{lo}..{hi}");
        let (a, b) = p.range(0.5, 0.6);
        assert!(a < -0.1 && b > 0.1);
    }

    /// get() is non-blocking, completes in the background, and the second instance hits the disk cache.
    #[test]
    fn cache_get_async_and_disk() {
        let path = ffpipe::tests::test_mp4();
        let poll = |c: &mut WaveformCache, path: &str| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
            assert!(c.get(path, 1).is_none(), "first get must not block");
            loop {
                if let Some(p) = c.get(path, 1) {
                    return p;
                }
                assert!(std::time::Instant::now() < deadline, "waveform never finished");
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        };
        let mut c = WaveformCache::new(eframe::egui::Context::default(), Backend::Ffmpeg);
        let first = poll(&mut c, &path);
        assert!(first.len() > 350 && first.max.iter().any(|&x| x > 0.1));
        let mut c2 = WaveformCache::new(eframe::egui::Context::default(), Backend::Ffmpeg);
        let second = poll(&mut c2, &path);
        assert_eq!(first.len(), second.len());
        assert_eq!(first.max, second.max);
        // failure is remembered as empty peaks (no respawn loop)
        let mut c3 = WaveformCache::new(eframe::egui::Context::default(), Backend::Ffmpeg);
        assert!(poll(&mut c3, "Z:/definitely/missing.mp4").is_empty());
    }

    /// After the file at a path is rewritten, clear() makes get() recompute instead of serving old peaks.
    #[test]
    fn clear_forgets_rewritten_file() {
        let poll = |c: &mut WaveformCache, path: &str| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
            loop {
                if let Some(p) = c.get(path, 0) {
                    return p;
                }
                assert!(std::time::Instant::now() < deadline, "waveform never finished");
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        };
        let path = std::path::Path::new(&ffpipe::tests::test_mp4())
            .with_file_name(format!("{}-rewrite.mp4", std::process::id()));
        std::fs::copy(ffpipe::tests::test_mp4(), &path).unwrap();
        let path = path.to_string_lossy().into_owned();
        let mut c = WaveformCache::new(eframe::egui::Context::default(), Backend::Ffmpeg);
        let old = poll(&mut c, &path);
        assert!(old.len() > 350, "{}", old.len());
        let st = std::process::Command::new("ffmpeg")
            .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i", "sine=frequency=440:duration=1", "-c:a", "aac"])
            .arg(&path)
            .status()
            .expect("ffmpeg on PATH");
        assert!(st.success());
        assert_eq!(poll(&mut c, &path).len(), old.len(), "stale until cleared");
        c.clear();
        let new = poll(&mut c, &path);
        assert!(new.len() < 150, "{} (old {})", new.len(), old.len());
    }

    #[test]
    fn peaks_range_bounds() {
        let p = Peaks { min: vec![-0.5, -0.2], max: vec![0.5, 0.2] };
        assert_eq!(p.range(0.0, 0.01), (-0.5, 0.5));
        assert_eq!(p.range(0.01, 0.02), (-0.2, 0.2));
        assert_eq!(p.range(5.0, 6.0), (0.0, 0.0));
    }

    #[test]
    fn mem_key_differs_per_stream() {
        assert_ne!(mem_key("a.mp4", 0), mem_key("a.mp4", 1));
        assert_eq!(mem_key("a.mp4", 0), mem_key("a.mp4", 0));
    }
}
