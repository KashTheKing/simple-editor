//! Thumbnail cache for timeline filmstrips and library previews. Non-blocking: `get`/`texture` return
//! immediately; misses are queued to ONE worker thread (newest request first, so scrubbing/zooming stays
//! responsive) that owns a DecoderPool (backend from settings) and decodes `h`-pixel-tall RGBA thumbnails
//! (aspect kept; images ignore `t`; Sequence clips are not handled here — the timeline draws a label).
//! Results are kept in an LRU bounded by both entry count (≤512) and decoded bytes (≤48 MB) and uploaded
//! lazily as egui textures (`texture`) so panels can paint them directly. `ctx.request_repaint()` when a
//! thumbnail lands. Key = (path, t rounded to 0.5 s buckets, h rounded up to 16 px); `clear()` forgets
//! everything (e.g. after save-over-original).

use crate::media::{is_image_path, Backend, DecoderPool, Frame};
use eframe::egui;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Condvar, Mutex};

/// Max decoded thumbnails kept (failed lookups are memoised separately and never evicted).
const CAP: usize = 512;
/// Max decoded bytes kept — a 1080p source 300 px tall is 640 KB per entry, so CAP alone is not a bound.
const BUDGET: usize = 48 << 20;

struct Entry {
    /// None = decode failed; memoised so a bad file/time is never retried.
    frame: Option<Arc<Frame>>,
    /// Last-used tick for LRU eviction.
    tick: u64,
}

struct Shared {
    ready: HashMap<u64, Entry>,
    /// LIFO request stack (worker pops from the back → newest first).
    queue: Vec<(u64, String, f64, u32)>,
    pending: HashSet<u64>,
    /// Keys evicted by the worker; the UI thread drains this to drop their textures.
    evicted: Vec<u64>,
    tick: u64,
    backend: Backend,
    /// Bumped by `clear()`; in-flight decodes from an older epoch are discarded.
    epoch: u64,
    /// Ask the worker to drop its decoders (releases file handles).
    clear_pool: bool,
    quit: bool,
}

/// ponytail: 64-bit hash as the map key so per-frame lookups allocate nothing — collisions are astronomically unlikely.
fn key_of(path: &str, t: f64, h: u32) -> u64 {
    let mut hs = std::collections::hash_map::DefaultHasher::new();
    (path, (t * 2.0).round() as i64, h).hash(&mut hs);
    hs.finish()
}

/// ponytail: 16 px steps so a track-height drag reuses cached thumbs instead of queueing a decode per
/// pixel of height; rounding up means the texture is never upscaled on draw.
fn qh(h: u32) -> u32 {
    h.max(1).next_multiple_of(16)
}

/// The half-second bucket time actually decoded for a request at `t` (0 for images).
fn bucket_time(path: &str, t: f64) -> f64 {
    if is_image_path(path) {
        0.0
    } else {
        (t * 2.0).round().max(0.0) / 2.0
    }
}

pub struct ThumbCache {
    shared: Arc<(Mutex<Shared>, Condvar)>,
    /// egui textures for ready thumbnails, created lazily by `texture()` (UI thread only).
    textures: HashMap<u64, (egui::TextureHandle, [u32; 2])>,
}

impl ThumbCache {
    pub fn new(ctx: egui::Context, backend: Backend) -> Self {
        let shared = Arc::new((
            Mutex::new(Shared {
                ready: HashMap::new(),
                queue: Vec::new(),
                pending: HashSet::new(),
                evicted: Vec::new(),
                tick: 0,
                backend,
                epoch: 0,
                clear_pool: false,
                quit: false,
            }),
            Condvar::new(),
        ));
        let s = shared.clone();
        let _ = std::thread::Builder::new().name("thumbs".into()).spawn(move || worker(s, ctx));
        Self { shared, textures: HashMap::new() }
    }

    pub fn set_backend(&mut self, b: Backend) {
        {
            let Ok(mut st) = self.shared.0.lock() else { return };
            if st.backend == b {
                return;
            }
            st.backend = b;
        }
        // new backend may decode what the old one couldn't → forget everything (incl. failed memos)
        self.clear();
    }

    /// Thumbnail frame of `path` at source time `t`, `h` px tall. None while computing (request queued).
    pub fn get(&mut self, path: &str, t: f64, h: u32) -> Option<Arc<Frame>> {
        let (t, h) = (bucket_time(path, t), qh(h));
        let key = key_of(path, t, h);
        let (result, evicted) = {
            let mut st = self.shared.0.lock().ok()?;
            st.tick += 1;
            let tick = st.tick;
            let result = match st.ready.get_mut(&key) {
                Some(e) => {
                    e.tick = tick;
                    e.frame.clone() // None here = memoised failure: do not re-queue
                }
                None => {
                    if st.pending.insert(key) {
                        st.queue.push((key, path.to_string(), t, h));
                        self.shared.1.notify_one();
                    }
                    None
                }
            };
            (result, std::mem::take(&mut st.evicted))
        };
        for k in evicted {
            self.textures.remove(&k);
        }
        result
    }

    /// Same as `get` but as an egui texture (created/cached here), with its size in pixels.
    pub fn texture(&mut self, ctx: &egui::Context, path: &str, t: f64, h: u32) -> Option<(egui::TextureId, [u32; 2])> {
        let frame = self.get(path, t, h)?;
        let key = key_of(path, bucket_time(path, t), qh(h));
        if let Some((th, size)) = self.textures.get(&key) {
            return Some((th.id(), *size));
        }
        let (w, hh) = (frame.width as usize, frame.height as usize);
        if w == 0 || hh == 0 || frame.rgba.len() != w * hh * 4 {
            return None;
        }
        let img = egui::ColorImage::from_rgba_premultiplied([w, hh], &frame.rgba);
        let th = ctx.load_texture(format!("thumb{key:016x}"), img, egui::TextureOptions::LINEAR);
        let (id, size) = (th.id(), [frame.width, frame.height]);
        self.textures.insert(key, (th, size));
        Some((id, size))
    }

    pub fn clear(&mut self) {
        self.textures.clear();
        if let Ok(mut st) = self.shared.0.lock() {
            st.ready.clear();
            st.queue.clear();
            st.pending.clear();
            st.evicted.clear();
            st.epoch += 1;
            st.clear_pool = true;
        }
        self.shared.1.notify_one();
    }
}

impl Drop for ThumbCache {
    fn drop(&mut self) {
        if let Ok(mut st) = self.shared.0.lock() {
            st.quit = true;
        }
        self.shared.1.notify_all();
        // ponytail: no join — a mid-decode worker exits at its next loop; joining could stall the UI.
    }
}

enum Job {
    Quit,
    Clear,
    Decode(u64, String, f64, u32, Backend, u64),
}

fn worker(shared: Arc<(Mutex<Shared>, Condvar)>, ctx: egui::Context) {
    let mut pool: Option<DecoderPool> = None;
    loop {
        let job = {
            let Ok(mut st) = shared.0.lock() else { return };
            loop {
                if st.quit {
                    break Job::Quit;
                }
                if st.clear_pool {
                    st.clear_pool = false;
                    break Job::Clear;
                }
                if let Some((k, p, t, h)) = st.queue.pop() {
                    break Job::Decode(k, p, t, h, st.backend, st.epoch);
                }
                match shared.1.wait(st) {
                    Ok(g) => st = g,
                    Err(_) => return,
                }
            }
        };
        match job {
            Job::Quit => return,
            Job::Clear => pool = None, // drop decoders → release file handles
            Job::Decode(key, path, t, h, backend, epoch) => {
                let p = pool.get_or_insert_with(|| DecoderPool::new(backend));
                p.set_backend(backend); // no-op when unchanged
                                        // ponytail: a decoder panic must not kill the only thumb worker — memoise it as a failed
                                        // entry (so it is never retried) and drop the pool so the next job starts on fresh decoders.
                let frame =
                    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| decode_thumb(p, &path, t, h))) {
                        Ok(f) => f,
                        Err(_) => {
                            pool = None;
                            None
                        }
                    };
                let Ok(mut st) = shared.0.lock() else { return };
                if st.epoch != epoch {
                    continue; // cleared while decoding: discard the stale result
                }
                st.pending.remove(&key);
                st.tick += 1;
                let tick = st.tick;
                st.ready.insert(key, Entry { frame: frame.map(Arc::new), tick });
                evict(&mut st);
                drop(st);
                ctx.request_repaint();
            }
        }
    }
}

/// Decode `path` at `t` scaled to `h` px tall, aspect kept. None on any failure.
fn decode_thumb(pool: &mut DecoderPool, path: &str, t: f64, h: u32) -> Option<Frame> {
    let v = pool.video(path)?;
    let (w0, h0) = v.size();
    if w0 == 0 || h0 == 0 {
        return None;
    }
    let h = h.clamp(1, 2160);
    let w = ((w0 as f64 * h as f64 / h0 as f64).round() as u32).max(1);
    let mut out = Frame::new(w, h);
    out.pts = t;
    if v.frame_at(t, w, h, &mut out) {
        Some(out)
    } else {
        None
    }
}

/// Drop least-recently-used decoded thumbnails beyond CAP entries or BUDGET bytes (failed memos are
/// exempt: they cost nothing).
/// ponytail: O(n) min-scan per insert over ≤512 entries on the worker thread — a real LRU list if CAP grows.
fn evict(st: &mut Shared) {
    let live = |e: &Entry| e.frame.as_ref().map(|f| f.rgba.len());
    let mut bytes: usize = st.ready.values().filter_map(live).sum();
    let mut count = st.ready.values().filter(|e| e.frame.is_some()).count();
    while bytes > BUDGET || count > CAP {
        let Some((&k, _)) = st.ready.iter().filter(|(_, e)| e.frame.is_some()).min_by_key(|(_, e)| e.tick) else {
            return;
        };
        bytes -= st.ready.remove(&k).as_ref().and_then(live).unwrap_or(0);
        count -= 1;
        st.evicted.push(k);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn poll(c: &mut ThumbCache, path: &str, t: f64, h: u32, secs: u64) -> Option<Arc<Frame>> {
        let deadline = Instant::now() + Duration::from_secs(secs);
        loop {
            if let Some(f) = c.get(path, t, h) {
                return Some(f);
            }
            if Instant::now() >= deadline {
                return None;
            }
            let done = {
                let st = c.shared.0.lock().unwrap();
                st.pending.is_empty() && st.queue.is_empty()
            };
            if done {
                // worker finished: either the frame just landed or the failure was memoised
                return c.get(path, t, h);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn centre(f: &Frame) -> [u8; 3] {
        let i = ((f.height / 2 * f.width + f.width / 2) * 4) as usize;
        [f.rgba[i], f.rgba[i + 1], f.rgba[i + 2]]
    }

    #[test]
    fn video_thumbs_colours_aspect_and_cache() {
        let path = crate::media::ffpipe::tests::test_mp4(); // 320x240: red 0–2 s, green 2–4 s
        let ctx = egui::Context::default();
        let mut c = ThumbCache::new(ctx.clone(), Backend::Ffmpeg);
        assert!(c.get(&path, 0.5, 48).is_none(), "first get must not block");
        let red = poll(&mut c, &path, 0.5, 48, 3).expect("thumb within 3 s");
        assert_eq!((red.width, red.height), (64, 48), "aspect kept");
        let [r, g, _] = centre(&red);
        assert!(r > 180 && g < 90, "red at 0.5 s, got {:?}", centre(&red));
        let green = poll(&mut c, &path, 2.5, 48, 3).expect("second thumb");
        let [r, g, _] = centre(&green);
        assert!(g > 180 && r < 90, "green at 2.5 s, got {:?}", centre(&green));
        // repeated calls hit the cache: immediate, and nothing queued
        assert!(c.get(&path, 0.5, 48).is_some());
        assert!(c.get(&path, 0.6, 48).is_some(), "same 0.5 s bucket");
        {
            let st = c.shared.0.lock().unwrap();
            assert!(st.queue.is_empty() && st.pending.is_empty());
        }
        // textures: created once, stable id, right size
        let (id, size) = c.texture(&ctx, &path, 0.5, 48).unwrap();
        assert_eq!(size, [64, 48]);
        let (id2, _) = c.texture(&ctx, &path, 0.5, 48).unwrap();
        assert_eq!(id, id2);
        // clear forgets everything
        c.clear();
        assert!(c.textures.is_empty());
        assert!(c.get(&path, 0.5, 48).is_none());
        assert!(poll(&mut c, &path, 0.5, 48, 5).is_some(), "recomputes after clear");
    }

    #[test]
    fn failed_decode_memoised() {
        let mut c = ThumbCache::new(egui::Context::default(), Backend::Ffmpeg);
        assert!(poll(&mut c, "Z:/definitely/missing.mp4", 0.0, 48, 10).is_none());
        // wait until the worker resolved it
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let st = c.shared.0.lock().unwrap();
            if st.pending.is_empty() {
                break;
            }
            drop(st);
            assert!(Instant::now() < deadline, "failure never resolved");
            std::thread::sleep(Duration::from_millis(10));
        }
        // still None, and never re-queued
        assert!(c.get("Z:/definitely/missing.mp4", 0.0, 48).is_none());
        let st = c.shared.0.lock().unwrap();
        assert!(st.queue.is_empty() && st.pending.is_empty(), "no respawn loop");
        assert_eq!(st.ready.len(), 1, "failure memoised");
    }

    #[test]
    fn image_ignores_t() {
        let png = std::path::Path::new(&crate::media::ffpipe::tests::test_mp4())
            .with_file_name(format!("{}-thumb.png", std::process::id()));
        let st = std::process::Command::new("ffmpeg")
            .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i", "color=blue:s=64x48", "-frames:v", "1"])
            .arg(&png)
            .status()
            .expect("ffmpeg on PATH");
        assert!(st.success());
        let png = png.to_string_lossy().into_owned();
        let mut c = ThumbCache::new(egui::Context::default(), Backend::Ffmpeg);
        let f = poll(&mut c, &png, 7.0, 24, 10).expect("image thumb");
        assert_eq!((f.width, f.height), (43, 32), "64x48 source at qh(24) = 32 px tall");
        let [r, _, b] = centre(&f);
        assert!(b > 180 && r < 90, "blue, got {:?}", centre(&f));
        // any t maps to the same cached entry
        assert!(c.get(&png, 123.0, 24).is_some());
        assert!(c.get(&png, 0.0, 24).is_some());
        let stt = c.shared.0.lock().unwrap();
        assert_eq!(stt.ready.len(), 1);
    }

    fn empty_shared() -> Shared {
        Shared {
            ready: HashMap::new(),
            queue: Vec::new(),
            pending: HashSet::new(),
            evicted: Vec::new(),
            tick: 0,
            backend: Backend::Ffmpeg,
            epoch: 0,
            clear_pool: false,
            quit: false,
        }
    }

    #[test]
    fn lru_evicts_and_drops_textures() {
        // shrink the cap indirectly: fill ready by hand and let evict() trim it
        let mut st = empty_shared();
        for i in 0..(CAP as u64 + 3) {
            st.ready.insert(i, Entry { frame: Some(Arc::new(Frame::new(1, 1))), tick: i });
        }
        st.ready.insert(9999, Entry { frame: None, tick: 0 }); // failed memo: never evicted
        evict(&mut st);
        assert_eq!(st.ready.values().filter(|e| e.frame.is_some()).count(), CAP);
        assert_eq!(st.evicted.len(), 3);
        // the oldest ticks went first
        assert!(st.evicted.contains(&0) && st.evicted.contains(&1) && st.evicted.contains(&2));
        assert!(st.ready.contains_key(&9999));
    }

    /// Big thumbnails are bounded by bytes long before the entry count cap is reached.
    #[test]
    fn lru_evicts_on_byte_budget() {
        let mut st = empty_shared();
        let big = Arc::new(Frame::new(512, 512)); // 1 MiB each
        for i in 0..64u64 {
            st.ready.insert(i, Entry { frame: Some(big.clone()), tick: i });
        }
        evict(&mut st);
        let bytes: usize = st.ready.values().filter_map(|e| e.frame.as_ref()).map(|f| f.rgba.len()).sum();
        assert!(bytes <= BUDGET, "{bytes} > {BUDGET}");
        assert_eq!(st.ready.len(), BUDGET >> 20, "kept the 48 newest, count cap never fired");
        assert!(st.evicted.contains(&0) && !st.evicted.contains(&63), "oldest first");
    }

    /// A track-height drag must reuse cached entries instead of queueing a decode per pixel of height.
    #[test]
    fn height_quantised() {
        assert_eq!([qh(0), qh(1), qh(16), qh(17), qh(40)], [16, 16, 16, 32, 48]);
        assert!((1..1000).all(|h| qh(h) >= h), "never upscaled on draw");
        // MIN_TRACK_H..MAX_TRACK_H in the timeline: 277 pixel heights → 18 distinct decodes
        let keys: HashSet<u64> = (24..=300).map(|h| key_of("a.mp4", 0.5, qh(h))).collect();
        assert_eq!(keys.len(), 18);
        // get() and texture() must agree on the key, else every texture lookup misses
        assert_eq!(key_of("a.mp4", 0.5, qh(41)), key_of("a.mp4", 0.5, qh(48)));
    }
}
