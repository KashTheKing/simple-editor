//! Media decoding: a small trait layer over two backends.
//!  * `mf`     — Windows Media Foundation (native software decode, no external deps, instant seeks). Primary.
//!  * `ffpipe` — ffmpeg.exe / ffprobe.exe child processes. Universal fallback, images, probing, export.
//! Everything produces top-down RGBA8 frames and interleaved stereo f32 audio at SAMPLE_RATE.

pub mod ffpipe;
pub mod mf;
pub mod proxy;
pub mod thumbs;
pub mod waveform;
pub mod ytdlp;

use crate::model::Asset;
use std::collections::HashMap;
use std::sync::Arc;

pub const SAMPLE_RATE: u32 = 48000;
pub const CHANNELS: usize = 2;

/// Top-down RGBA8 image. `rgba.len() == width*height*4`.
#[derive(Clone, Default)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// Source/timeline time this frame represents (informational).
    pub pts: f64,
    pub rgba: Vec<u8>,
}

impl Frame {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height, pts: 0.0, rgba: vec![0; (width * height * 4) as usize] }
    }
    /// Resize (contents undefined/zero when the size changes).
    pub fn resize(&mut self, width: u32, height: u32) {
        if self.width != width || self.height != height {
            self.width = width;
            self.height = height;
            self.rgba.clear();
            self.rgba.resize((width * height * 4) as usize, 0);
        }
    }
    pub fn fill(&mut self, rgba: [u8; 4]) {
        for px in self.rgba.chunks_exact_mut(4) {
            px.copy_from_slice(&rgba);
        }
    }
    pub fn stride(&self) -> usize {
        self.width as usize * 4
    }
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }
}

pub trait VideoSource: Send {
    /// Native (coded) size.
    fn size(&self) -> (u32, u32);
    /// Decode the frame displayed at source time `t`, scaled (aspect-ignorant, caller picks w/h; the
    /// compositor never asks for more than native size, so upscaling need only be correct) into `out`.
    /// Returns false at/after EOF or on error (then `out` is untouched). Must be cheap for sequential
    /// increasing `t` (playback: keep decoding forward); may seek for other jumps.
    fn frame_at(&mut self, t: f64, w: u32, h: u32, out: &mut Frame) -> bool;
}

pub trait AudioSource: Send {
    fn duration(&self) -> f64;
    /// Fill `out` (interleaved stereo f32 @ SAMPLE_RATE, frames = out.len()/2) starting at source time `t`.
    /// Zero-fill past the end. Must be cheap for sequential calls (t advancing by exactly the previous block).
    fn read_at(&mut self, t: f64, out: &mut [f32]);
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Backend {
    Auto,
    Mf,
    Ffmpeg,
}

impl Backend {
    pub fn parse(s: &str) -> Self {
        match s {
            "mf" => Backend::Mf,
            "ffmpeg" => Backend::Ffmpeg,
            _ => Backend::Auto,
        }
    }
}

/// Lower-case file extension ("" when none).
pub fn ext(path: &str) -> String {
    std::path::Path::new(path).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

pub fn is_image_path(path: &str) -> bool {
    matches!(ext(path).as_str(), "png" | "jpg" | "jpeg" | "bmp" | "gif" | "webp" | "tif" | "tiff" | "tga" | "psd")
}

/// Probe a media file into an Asset (id = 0). ffprobe gives the richest stream metadata, so it is
/// preferred when present; Media Foundation otherwise.
pub fn probe(path: &str, backend: Backend) -> Result<Asset, String> {
    match backend {
        Backend::Ffmpeg => ffpipe::probe(path),
        Backend::Mf => mf::probe(path),
        Backend::Auto => {
            if ffpipe::ffprobe_exe().is_some() {
                ffpipe::probe(path).or_else(|e| mf::probe(path).map_err(|e2| format!("{e}; {e2}")))
            } else {
                mf::probe(path).or_else(|e| ffpipe::probe(path).map_err(|e2| format!("{e}; {e2}")))
            }
        }
    }
}

pub fn open_video(path: &str, backend: Backend) -> Result<Box<dyn VideoSource>, String> {
    if is_image_path(path) {
        return ffpipe::open_video(path);
    }
    match backend {
        Backend::Ffmpeg => ffpipe::open_video(path),
        Backend::Mf => mf::open_video(path),
        Backend::Auto => mf::open_video(path).or_else(|e| ffpipe::open_video(path).map_err(|e2| format!("{e}; {e2}"))),
    }
}

pub fn open_audio(path: &str, stream: usize, backend: Backend) -> Result<Box<dyn AudioSource>, String> {
    match backend {
        Backend::Ffmpeg => ffpipe::open_audio(path, stream),
        Backend::Mf => mf::open_audio(path, stream),
        Backend::Auto => mf::open_audio(path, stream)
            .or_else(|e| ffpipe::open_audio(path, stream).map_err(|e2| format!("{e}; {e2}"))),
    }
}

/// Lazily opened decoders, one per (path) / (path, audio stream). Failed opens are remembered
/// (None) so a missing file doesn't re-spawn work every frame.
pub struct DecoderPool {
    backend: Backend,
    videos: HashMap<String, (u64, Option<Box<dyn VideoSource>>)>,
    audios: HashMap<(String, usize), (u64, Option<Box<dyn AudioSource>>)>,
    /// Use counter for LRU eviction: a long timeline must not accumulate one live MF reader (or
    /// ffmpeg.exe child) per distinct file forever.
    tick: u64,
    /// source path -> proxy file: preview decode opens the proxy instead of the original. Empty for
    /// export / one-shot pools, which must always read the real footage. Video only.
    proxies: std::collections::HashMap<String, String>,
    /// Decoded-source-frame LRU used by `frame_at` — see `SourceCache`. Budget 0 (the default)
    /// disables it entirely, so export / thumbnail / prerender / audio pools stay byte-identical
    /// to a pool without one; only the preview render thread turns it on (`Cmd::CacheBudget`).
    source_cache: SourceCache,
}

/// Byte-budgeted LRU of decoded source frames, keyed by (SOURCE path, request time in µs, w, h).
/// Callers re-derive the same `f64` time for the same timeline frame (fps grid → src_time → clamp),
/// so keys recur bit-exactly on replays; keying by the source path (pre-proxy-resolution) lets a
/// proxy swap invalidate exactly the entries whose pixels changed. This is what turns a backwards
/// scrub past the composited cache from an ffmpeg respawn / MF GOP re-decode into a memcpy.
/// ponytail: no protect window, plain LRU — the composited caches handle read-ahead protection;
/// this one only needs "recently decoded stays".
#[derive(Default)]
struct SourceCache {
    map: HashMap<(String, i64, u32, u32), (u64, Arc<Frame>)>,
    bytes: usize,
    budget: usize,
    tick: u64,
}

impl SourceCache {
    fn key(path: &str, t: f64, w: u32, h: u32) -> (String, i64, u32, u32) {
        (path.to_ascii_lowercase(), (t * 1e6).round() as i64, w, h)
    }
    fn get(&mut self, path: &str, t: f64, w: u32, h: u32) -> Option<Arc<Frame>> {
        if self.budget == 0 {
            return None;
        }
        self.tick += 1;
        let e = self.map.get_mut(&Self::key(path, t, w, h))?;
        e.0 = self.tick;
        Some(e.1.clone())
    }
    fn insert(&mut self, path: &str, t: f64, w: u32, h: u32, f: Arc<Frame>) {
        if self.budget == 0 {
            return;
        }
        self.tick += 1;
        let bytes = f.rgba.len();
        if bytes > self.budget {
            return; // a frame bigger than the whole budget would just evict everything for nothing
        }
        if let Some((_, old)) = self.map.insert(Self::key(path, t, w, h), (self.tick, f)) {
            self.bytes -= old.rgba.len();
        }
        self.bytes += bytes;
        while self.bytes > self.budget {
            // ponytail: O(n) min scan per eviction, like playback::Cache — n stays small.
            let Some(k) = self.map.iter().min_by_key(|(_, (t, _))| *t).map(|(k, _)| k.clone()) else { break };
            if let Some((_, old)) = self.map.remove(&k) {
                self.bytes -= old.rgba.len();
            }
        }
    }
    /// Drop every entry for one source path (case-insensitive) — a proxy for it appeared/vanished.
    fn evict_path(&mut self, path: &str) {
        let p = path.to_ascii_lowercase();
        self.map.retain(|(k, _, _, _), (_, f)| {
            let keep = *k != p;
            if !keep {
                self.bytes -= f.rgba.len();
            }
            keep
        });
    }
    fn clear(&mut self) {
        self.map.clear();
        self.bytes = 0;
    }
}

/// Live decoders kept per pool (LRU past this). Failed opens (None) are cheap and never counted.
const POOL_VIDEOS: usize = 16;
const POOL_AUDIOS: usize = 32;

impl DecoderPool {
    pub fn new(backend: Backend) -> Self {
        Self {
            backend,
            videos: HashMap::new(),
            audios: HashMap::new(),
            tick: 0,
            proxies: HashMap::new(),
            source_cache: SourceCache::default(),
        }
    }
    /// Swap the proxy map. Returns the SOURCE paths whose mapping actually changed (added, removed
    /// or re-pointed), and drops only THEIR decoders and cached frames — a proxy finishing for one
    /// file must not cost every other file its warm decoder. Audio decoders are untouched: audio
    /// always reads the originals (see `proxies` above).
    pub fn set_proxies(&mut self, map: HashMap<String, String>) -> Vec<String> {
        if map == self.proxies {
            return Vec::new();
        }
        let changed: Vec<String> = self
            .proxies
            .keys()
            .chain(map.keys())
            .filter(|k| self.proxies.get(*k) != map.get(*k))
            .cloned()
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        for src in &changed {
            // the pool is keyed by RESOLVED path: the source itself, its old proxy, or its new one
            // may each hold a live decoder (ASCII-case-insensitive, like Project::asset_by_path)
            let mut victims: Vec<String> = vec![src.clone()];
            victims.extend(self.proxies.get(src).cloned());
            victims.extend(map.get(src).cloned());
            self.videos.retain(|k, _| !victims.iter().any(|v| v.eq_ignore_ascii_case(k)));
            self.source_cache.evict_path(src);
        }
        self.proxies = map;
        changed
    }
    /// Byte budget for the decoded-source-frame cache (0 = off, the default for every pool except
    /// the preview render thread's).
    pub fn set_source_cache_bytes(&mut self, bytes: usize) {
        self.source_cache.budget = bytes;
        if bytes == 0 {
            self.source_cache.clear();
        } else {
            while self.source_cache.bytes > bytes {
                let sc = &mut self.source_cache;
                let Some(k) = sc.map.iter().min_by_key(|(_, (t, _))| *t).map(|(k, _)| k.clone()) else { break };
                if let Some((_, old)) = sc.map.remove(&k) {
                    sc.bytes -= old.rgba.len();
                }
            }
        }
    }
    /// Decode a frame through the source-frame cache: a hit is one memcpy instead of a seek /
    /// ffmpeg respawn. Same contract as `VideoSource::frame_at` (`out` untouched on `false`).
    pub fn frame_at(&mut self, path: &str, t: f64, w: u32, h: u32, out: &mut Frame) -> bool {
        if let Some(f) = self.source_cache.get(path, t, w, h) {
            out.resize(f.width, f.height);
            out.rgba.copy_from_slice(&f.rgba);
            out.pts = f.pts;
            return true;
        }
        let Some(dec) = self.video(path) else { return false };
        if !dec.frame_at(t, w, h, out) {
            return false;
        }
        self.source_cache.insert(path, t, w, h, Arc::new(out.clone()));
        true
    }
    pub fn set_backend(&mut self, b: Backend) {
        if b != self.backend {
            self.backend = b;
            self.clear();
        }
    }
    pub fn video(&mut self, path: &str) -> Option<&mut (dyn VideoSource + 'static)> {
        // preview pools decode the proxy when one exists (export pools carry an empty map)
        let path = self.proxies.get(path).cloned().unwrap_or_else(|| path.to_string());
        let path = path.as_str();
        let b = self.backend;
        self.tick += 1;
        let tick = self.tick;
        let e = self.videos.entry(path.to_string()).or_insert_with(|| (tick, open_video(path, b).ok()));
        e.0 = tick;
        let hit = e.1.is_some();
        if hit && self.videos.values().filter(|(_, v)| v.is_some()).count() > POOL_VIDEOS {
            let evict = self
                .videos
                .iter()
                .filter(|(p, (_, v))| v.is_some() && p.as_str() != path)
                .min_by_key(|(_, (t, _))| *t)
                .map(|(p, _)| p.clone());
            if let Some(p) = evict {
                self.videos.remove(&p);
            }
        }
        self.videos.get_mut(path).and_then(|(_, v)| v.as_deref_mut())
    }
    pub fn audio(&mut self, path: &str, stream: usize) -> Option<&mut (dyn AudioSource + 'static)> {
        let b = self.backend;
        self.tick += 1;
        let tick = self.tick;
        let key = (path.to_string(), stream);
        let e = self.audios.entry(key.clone()).or_insert_with(|| (tick, open_audio(path, stream, b).ok()));
        e.0 = tick;
        let hit = e.1.is_some();
        if hit && self.audios.values().filter(|(_, v)| v.is_some()).count() > POOL_AUDIOS {
            let evict = self
                .audios
                .iter()
                .filter(|(k, (_, v))| v.is_some() && **k != key)
                .min_by_key(|(_, (t, _))| *t)
                .map(|(k, _)| k.clone());
            if let Some(k) = evict {
                self.audios.remove(&k);
            }
        }
        self.audios.get_mut(&key).and_then(|(_, v)| v.as_deref_mut())
    }
    /// Inject a ready-made source (tests / synthetic media).
    #[cfg(test)]
    pub fn insert_video(&mut self, path: &str, v: Box<dyn VideoSource>) {
        self.videos.insert(path.to_string(), (self.tick, Some(v)));
    }
    #[cfg(test)]
    pub fn insert_audio(&mut self, path: &str, stream: usize, a: Box<dyn AudioSource>) {
        self.audios.insert((path.to_string(), stream), (self.tick, Some(a)));
    }
    /// Drop every decoder (releases file handles — required before overwriting a source file). Also
    /// forgets failed opens, so they are retried next time.
    pub fn clear(&mut self) {
        self.videos.clear();
        self.audios.clear();
        self.source_cache.clear(); // a caller may be about to overwrite a source file
    }
    /// Test-only: is a live decoder open for exactly this (resolved) path?
    #[cfg(test)]
    pub fn has_video(&self, path: &str) -> bool {
        self.videos.get(path).is_some_and(|(_, v)| v.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Counts real decodes, so a cache hit is provable as "no new call".
    struct Counting(Arc<AtomicUsize>);
    impl VideoSource for Counting {
        fn size(&self) -> (u32, u32) {
            (32, 32)
        }
        fn frame_at(&mut self, t: f64, w: u32, h: u32, out: &mut Frame) -> bool {
            self.0.fetch_add(1, Ordering::SeqCst);
            out.resize(w, h);
            out.pts = t;
            true
        }
    }
    struct Silence;
    impl AudioSource for Silence {
        fn duration(&self) -> f64 {
            1.0
        }
        fn read_at(&mut self, _t: f64, out: &mut [f32]) {
            out.fill(0.0);
        }
    }
    fn fake(c: &Arc<AtomicUsize>) -> Box<dyn VideoSource> {
        Box::new(Counting(c.clone()))
    }

    /// `set_proxies` reports exactly the changed sources and drops only their decoders — audio and
    /// unrelated video decoders survive a proxy landing for some other file.
    #[test]
    fn set_proxies_returns_changed_and_drops_targeted() {
        let c = Arc::new(AtomicUsize::new(0));
        let mut pool = DecoderPool::new(Backend::Ffmpeg);
        pool.insert_video("C:\\a.mp4", fake(&c));
        pool.insert_video("C:\\b.mp4", fake(&c));
        pool.insert_audio("C:\\a.mp4", 0, Box::new(Silence));

        let mut m = HashMap::new();
        m.insert("C:\\a.mp4".to_string(), "C:\\a-proxy.mp4".to_string());
        assert_eq!(pool.set_proxies(m.clone()), vec!["C:\\a.mp4".to_string()]);
        assert!(!pool.has_video("C:\\a.mp4"), "the remapped source's decoder is dropped");
        assert!(pool.has_video("C:\\b.mp4"), "an unrelated decoder survives");
        assert!(pool.audios.contains_key(&("C:\\a.mp4".to_string(), 0)), "audio always reads originals");
        assert!(pool.set_proxies(m).is_empty(), "an identical map changes nothing");

        // re-pointing drops the source, its OLD proxy and its NEW proxy (all possible pool keys)
        pool.insert_video("C:\\a-proxy.mp4", fake(&c));
        pool.insert_video("C:\\a-proxy2.mp4", fake(&c));
        let mut m2 = HashMap::new();
        m2.insert("C:\\a.mp4".to_string(), "C:\\a-proxy2.mp4".to_string());
        assert_eq!(pool.set_proxies(m2), vec!["C:\\a.mp4".to_string()]);
        assert!(!pool.has_video("C:\\a-proxy.mp4") && !pool.has_video("C:\\a-proxy2.mp4"));
        assert!(pool.has_video("C:\\b.mp4"));
    }

    /// The decoded-source-frame cache: identical requests hit (no second decode), budget 0 disables,
    /// LRU eviction works, and a proxy change for the source drops its entries.
    #[test]
    fn source_frame_cache_hits_evicts_and_invalidates() {
        let c = Arc::new(AtomicUsize::new(0));
        let mut pool = DecoderPool::new(Backend::Ffmpeg);
        pool.insert_video("C:\\v.mp4", fake(&c));
        let mut out = Frame::default();

        // budget 0 (the default): every request decodes — export/thumb pools stay untouched
        assert!(pool.frame_at("C:\\v.mp4", 1.0, 8, 8, &mut out));
        assert!(pool.frame_at("C:\\v.mp4", 1.0, 8, 8, &mut out));
        assert_eq!(c.load(Ordering::SeqCst), 2, "disabled cache never intercepts");

        pool.set_source_cache_bytes(10 << 20);
        assert!(pool.frame_at("C:\\v.mp4", 1.0, 8, 8, &mut out));
        assert!(pool.frame_at("C:\\v.mp4", 1.0, 8, 8, &mut out));
        assert_eq!(c.load(Ordering::SeqCst), 3, "bit-identical (t, w, h) replay is a hit");
        assert_eq!((out.width, out.height, out.pts), (8, 8, 1.0), "the hit fills out like a decode");
        assert!(pool.frame_at("C:\\v.mp4", 2.0, 8, 8, &mut out));
        assert_eq!(c.load(Ordering::SeqCst), 4, "a new time decodes");

        // an 8x8 RGBA frame is 256 bytes: budget 600 holds two — the third insert evicts the LRU
        pool.set_source_cache_bytes(600);
        for t in [1.0, 2.0, 3.0] {
            pool.frame_at("C:\\v.mp4", t, 8, 8, &mut out);
        }
        let before = c.load(Ordering::SeqCst);
        pool.frame_at("C:\\v.mp4", 1.0, 8, 8, &mut out); // oldest: evicted, decodes again
        assert_eq!(c.load(Ordering::SeqCst), before + 1, "LRU under budget evicted the oldest");
        pool.frame_at("C:\\v.mp4", 3.0, 8, 8, &mut out);

        // a proxy landing for the source invalidates its cached frames (content changed)
        let n = c.load(Ordering::SeqCst);
        let mut m = HashMap::new();
        m.insert("C:\\v.mp4".to_string(), "C:\\v-proxy.mp4".to_string());
        pool.set_proxies(m);
        pool.insert_video("C:\\v-proxy.mp4", fake(&c));
        pool.frame_at("C:\\v.mp4", 3.0, 8, 8, &mut out);
        assert_eq!(c.load(Ordering::SeqCst), n + 1, "no stale pre-proxy pixels served after the swap");
    }
}
