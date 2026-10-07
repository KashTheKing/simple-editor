//! Media decoding: a small trait layer over two backends.
//!  * `mf` - Windows Media Foundation (native software decode, no external deps, instant seeks). Primary.
//!  * `ffpipe` - ffmpeg.exe / ffprobe.exe child processes. Universal fallback, images, probing, export.
//! Everything produces top-down RGBA8 frames and interleaved stereo f32 audio at SAMPLE_RATE.

pub mod ffpipe;
pub mod mf;
pub mod proxy;
pub mod thumbs;
pub mod waveform;
pub mod ytdlp;

use crate::model::Asset;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Instant;

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
    /// Become a copy of `f`, reusing this buffer.
    pub fn copy_from(&mut self, f: &Frame) {
        self.resize(f.width, f.height);
        self.rgba.copy_from_slice(&f.rgba);
        self.pts = f.pts;
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
    /// A still image: every `t` is the same picture, so callers may cache one frame per size.
    fn is_still(&self) -> bool {
        false
    }
    /// A still that keeps its picture at w x h hands it out as it is: one shared frame, no copy.
    fn still(&mut self, _w: u32, _h: u32) -> Option<Arc<Frame>> {
        None
    }
    /// Decoded pixels this source holds on to (a still); 0 for one that decodes as it goes.
    fn bytes(&self) -> usize {
        0
    }
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

/// A still the caller knows the use of: (the file's size, the largest size it is ever shown at).
pub type StillUse = ((u32, u32), (u32, u32));

/// `open_video`, with what a preview pool knows about a still (`DecoderPool::set_stills`).
fn open_known(path: &str, backend: Backend, still: Option<StillUse>) -> Result<Box<dyn VideoSource>, String> {
    match still {
        Some((size, top)) if is_image_path(path) => ffpipe::open_still(path, size, top),
        _ => open_video(path, backend),
    }
}

pub fn open_video(path: &str, backend: Backend) -> Result<Box<dyn VideoSource>, String> {
    if is_image_path(path) {
        return ffpipe::open_video(path);
    }
    let v = match backend {
        Backend::Ffmpeg => ffpipe::open_video(path),
        Backend::Mf => mf::open_video(path),
        Backend::Auto => mf::open_video(path).or_else(|e| ffpipe::open_video(path).map_err(|e2| format!("{e}; {e2}"))),
    }?;
    // an alpha source's proxy is colour stacked over alpha: fold it back into one RGBA frame
    Ok(if proxy::is_stacked(std::path::Path::new(path)) { Box::new(proxy::StackedAlpha::new(v)) } else { v })
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
    /// By (path, stream, voice) - see `audio`.
    audios: HashMap<(String, usize, usize), (u64, Option<Box<dyn AudioSource>>)>,
    /// Use counter for LRU eviction: a long timeline must not accumulate one live MF reader (or
    /// ffmpeg.exe child) per distinct file forever.
    tick: u64,
    /// source path -> proxy file: preview decode opens the proxy instead of the original. Empty for
    /// export / one-shot pools, which must always read the real footage. Video only.
    proxies: std::collections::HashMap<String, String>,
    /// Decoded-source-frame LRU used by `frame_at` - see `SourceCache`. Budget 0 (the default)
    /// disables it entirely, so export / thumbnail / prerender / audio pools stay byte-identical
    /// to a pool without one; only the preview render thread turns it on (`Cmd::CacheBudget`).
    source_cache: SourceCache,
    /// Sources being opened ahead of use on their own threads (`warm`), by resolved path.
    warming: HashMap<String, std::thread::JoinHandle<Option<Box<dyn VideoSource>>>>,
    /// The timeline's stills (`set_stills`), and those of them still waiting for an opening thread.
    stills: HashMap<String, StillUse>,
    queue: VecDeque<String>,
    /// How long `video` waits for a source that is not open yet. None (every pool but the
    /// preview's) = until it is; otherwise up to this instant, and then the frame goes without it
    /// (`take_missed`) - one slow open must not hold up every other layer.
    deadline: Option<Instant>,
    missed: bool,
    /// A source finished opening and `video` took it in itself: `collect` has yet to say so.
    arrived: bool,
}

/// Byte-budgeted LRU of decoded source frames, keyed by (SOURCE path, request time in µs, w, h) -
/// a still (`VideoSource::is_still`) by `STILL_T`, since its picture is the same at every time.
/// Callers re-derive the same `f64` time for the same timeline frame (fps grid → src_time → clamp),
/// so keys recur bit-exactly on replays; keying by the source path (pre-proxy-resolution) lets a
/// proxy swap invalidate exactly the entries whose pixels changed. This is what turns a backwards
/// scrub past the composited cache from an ffmpeg respawn / MF GOP re-decode into a memcpy.
/// ponytail: no protect window, plain LRU - the composited caches handle read-ahead protection;
/// this one only needs "recently decoded stays".
#[derive(Default)]
struct SourceCache {
    map: HashMap<(String, i64, u32, u32), (u64, Arc<Frame>)>,
    bytes: usize,
    budget: usize,
    tick: u64,
}

impl SourceCache {
    /// The time key of a still: no real request rounds to it.
    const STILL_T: i64 = i64::MIN;
    fn us(t: f64) -> i64 {
        (t * 1e6).round() as i64
    }
    /// The entry for `path` at `us` µs - or its still entry, whatever the time.
    fn get(&mut self, path: &str, us: i64, w: u32, h: u32) -> Option<Arc<Frame>> {
        if self.budget == 0 {
            return None;
        }
        self.tick += 1;
        let mut key = (path.to_ascii_lowercase(), Self::STILL_T, w, h);
        if !self.map.contains_key(&key) {
            key.1 = us;
        }
        let e = self.map.get_mut(&key)?;
        e.0 = self.tick;
        Some(e.1.clone())
    }
    fn insert(&mut self, path: &str, us: i64, w: u32, h: u32, f: Arc<Frame>) {
        if self.budget == 0 {
            return;
        }
        self.tick += 1;
        let bytes = f.rgba.len();
        if bytes > self.budget {
            return; // a frame bigger than the whole budget would just evict everything for nothing
        }
        if let Some((_, old)) = self.map.insert((path.to_ascii_lowercase(), us, w, h), (self.tick, f)) {
            self.bytes -= old.rgba.len();
        }
        self.bytes += bytes;
        while self.bytes > self.budget {
            // ponytail: O(n) min scan per eviction, like playback::Cache - n stays small.
            let Some(k) = self.map.iter().min_by_key(|(_, (t, _))| *t).map(|(k, _)| k.clone()) else { break };
            if let Some((_, old)) = self.map.remove(&k) {
                self.bytes -= old.rgba.len();
            }
        }
    }
    /// Drop every entry for one source path (case-insensitive) - a proxy for it appeared/vanished.
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
            warming: HashMap::new(),
            stills: HashMap::new(),
            queue: VecDeque::new(),
            deadline: None,
            missed: false,
            arrived: false,
        }
    }
    /// Start opening `path` on a thread of its own, unless it is open (or opening) already; `video`
    /// picks the result up, waiting only for whatever is left. Opening is the slow part of a source
    /// - ffprobe.exe plus, for a still, the ffmpeg.exe run that decodes it: ~250 ms per PNG, 1 s
    /// for a 9 MP one - and done on first use it stops every other layer for that long. Playback
    /// calls this for the clips coming up, and for all of one frame's clips at once.
    /// ponytail: at most 8 opening at once (each is a process or two); the rest open on first use.
    /// `first` = the source time the clip coming up starts at: its frame there is decoded on the
    /// opening thread too and thrown away, so the reader's start-up (codec set-up, the seek) is
    /// paid there and the cut itself finds a decoder already rolling.
    pub fn warm(&mut self, path: &str, first: Option<f64>) {
        let path = self.proxies.get(path).cloned().unwrap_or_else(|| path.to_string());
        if self.videos.contains_key(&path) || self.warming.len() >= 8 || self.warming.contains_key(&path) {
            return;
        }
        self.open_on_thread(path, first);
    }
    fn open_on_thread(&mut self, path: String, first: Option<f64>) {
        mf::init_thread(); // COM must stay up on the thread that will use the reader
        let (b, p, still) = (self.backend, path.clone(), self.stills.get(&path).copied());
        let opening = std::thread::spawn(move || {
            let mut v = open_known(&p, b, still).ok()?;
            if let Some(t) = first.filter(|_| !v.is_still()) {
                let (w, h) = v.size();
                v.frame_at(t, w, h, &mut Frame::default());
            }
            Some(v)
        });
        self.warming.insert(path, opening);
    }
    /// The stills the timeline shows, most urgent first, each with its `StillUse`: all of them are
    /// decoded now, in the background, as far as the still budget goes (preview pools only: the
    /// source-cache budget, `set_source_cache_bytes`) - so that playing or scrubbing onto one finds
    /// it in RAM. A still that is open at another size than it now needs is decoded again; the old
    /// one stays in use until the new one is there.
    /// ponytail: 4 at a time (each is an ffmpeg.exe); a thread pool if a 500-photo timeline wants more.
    pub fn set_stills(&mut self, stills: Vec<(String, StillUse)>) {
        self.queue.clear();
        let budget = self.source_cache.budget;
        let held: usize = self.videos.values().map(|(_, v)| v.as_ref().map_or(0, |v| v.bytes())).sum();
        let mut room = budget.saturating_sub(held);
        for (path, (_, top)) in &stills {
            let open = self.videos.get(path).map(|(_, v)| v.as_ref().map(|v| v.size()));
            // failed opens stay failed; a decode in flight is taken as it comes
            if open == Some(None) || open == Some(Some(*top)) || self.warming.contains_key(path) || budget == 0 {
                continue;
            }
            let bytes = top.0 as usize * top.1 as usize * 16 / 3; // RGBA plus its halvings
            if bytes <= room {
                room -= bytes;
                self.queue.push_back(path.clone());
            }
        }
        self.stills = stills.into_iter().collect();
        self.collect();
    }
    /// Take in whatever finished opening and start what is queued. True when a source arrived - a
    /// frame that went without it (`take_missed`) can now be had whole.
    pub fn collect(&mut self) -> bool {
        let done: Vec<String> = self.warming.iter().filter(|(_, h)| h.is_finished()).map(|(p, _)| p.clone()).collect();
        for p in &done {
            let v = self.warming.remove(p).and_then(|h| h.join().ok()).flatten();
            self.tick += 1;
            self.videos.insert(p.clone(), (self.tick, v));
        }
        while self.warming.len() < 4 {
            let Some(p) = self.queue.pop_front() else { break };
            if !self.warming.contains_key(&p) {
                self.open_on_thread(p, None);
            }
        }
        std::mem::take(&mut self.arrived) || !done.is_empty()
    }
    /// Something is opening or waiting to: `collect` has work coming.
    pub fn busy(&self) -> bool {
        !self.warming.is_empty() || !self.queue.is_empty()
    }
    /// See `deadline`. The render thread sets it around each preview frame and clears it again.
    pub fn set_deadline(&mut self, deadline: Option<Instant>) {
        self.deadline = deadline;
    }
    /// Did a frame since the last call go without a source that was not open in time?
    pub fn take_missed(&mut self) -> bool {
        std::mem::take(&mut self.missed)
    }
    /// Swap the proxy map. Returns the SOURCE paths whose mapping actually changed (added, removed
    /// or re-pointed), and drops only THEIR decoders and cached frames - a proxy finishing for one
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
            self.warming.retain(|k, _| !victims.iter().any(|v| v.eq_ignore_ascii_case(k)));
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
        if let Some(f) = self.source_cache.get(path, SourceCache::us(t), w, h) {
            out.copy_from(&f);
            return true;
        }
        let Some(dec) = self.video(path) else { return false };
        if let Some(f) = dec.still(w, h) {
            out.copy_from(&f); // kept by the decoder itself: nothing to cache
            return true;
        }
        if !dec.frame_at(t, w, h, out) {
            return false;
        }
        let us = if dec.is_still() { SourceCache::STILL_T } else { SourceCache::us(t) };
        self.source_cache.insert(path, us, w, h, Arc::new(out.clone()));
        true
    }
    /// `frame_at` that hands out the frame itself (in a buffer from `spare` when it has to decode).
    /// A cached STILL comes back as the cache's own `Arc`: the same pixels at the same address on
    /// every frame, so a still on screen costs no copy here and no texture upload in `engine::gpu`
    /// (which re-uploads a layer only when its pointer/pts change). Video frames stay private
    /// copies - their buffers are recycled, and nothing would reuse a shared one anyway.
    pub fn frame_arc(&mut self, path: &str, t: f64, w: u32, h: u32, spare: &mut Vec<Frame>) -> Option<Arc<Frame>> {
        if let Some(f) = self.source_cache.get(path, SourceCache::STILL_T, w, h) {
            return Some(f);
        }
        if let Some(f) = self.video(path).and_then(|d| d.still(w, h)) {
            return Some(f);
        }
        let mut frame = spare.pop().unwrap_or_default();
        if !self.frame_at(path, t, w, h, &mut frame) {
            spare.push(frame);
            return None;
        }
        Some(Arc::new(frame))
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
        if let (Some(deadline), false) = (self.deadline, self.videos.contains_key(path)) {
            // opening is the slow part (a 9 MP still: half a second of ffmpeg.exe): it happens on a
            // thread of its own, and past the deadline the frame is made without this source
            if !self.warming.contains_key(path) && self.warming.len() < 8 {
                self.open_on_thread(path.to_string(), None);
            }
            while !self.warming.get(path).is_some_and(|h| h.is_finished()) {
                if Instant::now() >= deadline {
                    self.missed = true;
                    return None;
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        let b = self.backend;
        self.tick += 1;
        let tick = self.tick;
        let (warming, stills, arrived) = (&mut self.warming, &self.stills, &mut self.arrived);
        let e = self.videos.entry(path.to_string()).or_insert_with(|| {
            let v = match warming.remove(path) {
                Some(opening) => {
                    *arrived = true;
                    opening.join().ok().flatten()
                }
                None => open_known(path, b, stills.get(path).copied()).ok(),
            };
            (tick, v)
        });
        e.0 = tick;
        if e.1.is_some() {
            // Stills that keep their pixels are budgeted by those (a preview pool, which has a
            // budget and decodes every still of the timeline ahead of time); everything else by
            // count - a live MF reader or ffmpeg.exe child each.
            let budget = self.source_cache.budget;
            let held = |v: &Option<Box<dyn VideoSource>>| v.as_ref().map_or(0, |v| v.bytes());
            let lru = |videos: &HashMap<String, (u64, Option<Box<dyn VideoSource>>)>, still: bool| {
                videos
                    .iter()
                    .filter(|(p, (_, v))| v.is_some() && p.as_str() != path && (budget > 0 && held(v) > 0) == still)
                    .min_by_key(|(_, (t, _))| *t)
                    .map(|(p, _)| p.clone())
            };
            let counted = self.videos.values().filter(|(_, v)| v.is_some() && (budget == 0 || held(v) == 0)).count();
            if counted > POOL_VIDEOS {
                if let Some(p) = lru(&self.videos, false) {
                    self.videos.remove(&p);
                }
            }
            while budget > 0 && self.videos.values().map(|(_, v)| held(v)).sum::<usize>() > budget {
                let Some(p) = lru(&self.videos, true) else { break };
                self.videos.remove(&p);
            }
        }
        self.videos.get_mut(path).and_then(|(_, v)| v.as_deref_mut())
    }
    /// `voice` tells apart readers of one stream that play at the same moment (the mixer numbers the
    /// clips of a block): each reads on sequentially through a decoder of its own. Sharing one, two
    /// overlapping clips of a file made it seek twice per block.
    pub fn audio(&mut self, path: &str, stream: usize, voice: usize) -> Option<&mut (dyn AudioSource + 'static)> {
        let b = self.backend;
        self.tick += 1;
        let tick = self.tick;
        let key = (path.to_string(), stream, voice);
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
        self.insert_audio_voice(path, stream, 0, a);
    }
    #[cfg(test)]
    pub fn insert_audio_voice(&mut self, path: &str, stream: usize, voice: usize, a: Box<dyn AudioSource>) {
        self.audios.insert((path.to_string(), stream, voice), (self.tick, Some(a)));
    }
    /// Drop every decoder (releases file handles - required before overwriting a source file). Also
    /// forgets failed opens, so they are retried next time.
    pub fn clear(&mut self) {
        self.videos.clear();
        self.warming.clear(); // detached: each finishes its open and drops the result
        self.queue.clear();
        self.audios.clear();
        self.source_cache.clear(); // a caller may be about to overwrite a source file
    }
    /// Test-only: a source whose opening is under way on `opening`.
    #[cfg(test)]
    pub fn insert_opening(&mut self, path: &str, opening: std::thread::JoinHandle<Option<Box<dyn VideoSource>>>) {
        self.warming.insert(path.to_string(), opening);
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

    /// `set_proxies` reports exactly the changed sources and drops only their decoders - audio and
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
        assert!(pool.audios.contains_key(&("C:\\a.mp4".to_string(), 0, 0)), "audio always reads originals");
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

        // budget 0 (the default): every request decodes - export/thumb pools stay untouched
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

        // an 8x8 RGBA frame is 256 bytes: budget 600 holds two - the third insert evicts the LRU
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

    /// A still is one cache entry per size whatever the time, and `frame_arc` hands that entry out:
    /// the same `Arc` on every frame (no copy, and `engine::gpu` skips the upload). Without a cache
    /// (export pools) nothing is shared and nothing changes.
    #[test]
    fn a_still_is_one_shared_frame_at_every_time() {
        struct Still(Arc<AtomicUsize>);
        impl VideoSource for Still {
            fn size(&self) -> (u32, u32) {
                (32, 32)
            }
            fn is_still(&self) -> bool {
                true
            }
            fn frame_at(&mut self, _t: f64, w: u32, h: u32, out: &mut Frame) -> bool {
                self.0.fetch_add(1, Ordering::SeqCst);
                out.resize(w, h);
                true
            }
        }
        let c = Arc::new(AtomicUsize::new(0));
        let mut pool = DecoderPool::new(Backend::Ffmpeg);
        pool.insert_video("C:\\s.png", Box::new(Still(c.clone())));
        let mut spare = Vec::new();
        let a = pool.frame_arc("C:\\s.png", 0.1, 8, 8, &mut spare).unwrap();
        let b = pool.frame_arc("C:\\s.png", 0.2, 8, 8, &mut spare).unwrap();
        assert!(!Arc::ptr_eq(&a, &b), "no cache: every call is its own decode");
        assert_eq!(c.load(Ordering::SeqCst), 2);

        pool.set_source_cache_bytes(1 << 20);
        let first = pool.frame_arc("C:\\s.png", 0.3, 8, 8, &mut spare).unwrap();
        let a = pool.frame_arc("C:\\s.png", 0.4, 8, 8, &mut spare).unwrap();
        let b = pool.frame_arc("C:\\s.png", 9.0, 8, 8, &mut spare).unwrap();
        assert!(Arc::ptr_eq(&a, &b), "one frame for every time");
        assert_eq!(Arc::strong_count(&first), 1, "the decode that filled the cache stays the caller's own");
        assert_eq!(c.load(Ordering::SeqCst), 3, "decoded once");
        // the CPU compositor's copying path reads the same entry
        let mut out = Frame::default();
        assert!(pool.frame_at("C:\\s.png", 5.0, 8, 8, &mut out));
        assert_eq!(c.load(Ordering::SeqCst), 3);
        // another size is another entry
        let d = pool.frame_arc("C:\\s.png", 0.4, 4, 4, &mut spare).unwrap();
        assert_eq!((d.width, c.load(Ordering::SeqCst)), (4, 4));
    }

    /// `warm` opens in the background and `video` collects the result - no second open.
    #[test]
    fn warm_opens_in_the_background_and_video_picks_it_up() {
        let mut pool = DecoderPool::new(Backend::Ffmpeg);
        pool.warm("C:\\no-such-file.mp4", None);
        assert!(pool.warming.contains_key("C:\\no-such-file.mp4"));
        pool.warm("C:\\no-such-file.mp4", None); // already opening: nothing more
        assert_eq!(pool.warming.len(), 1);
        assert!(pool.video("C:\\no-such-file.mp4").is_none(), "a failed open is still a failed open");
        assert!(pool.warming.is_empty() && pool.videos.contains_key("C:\\no-such-file.mp4"));
        pool.warm("C:\\no-such-file.mp4", None); // known (failed) already: not retried behind the pool's back
        assert!(pool.warming.is_empty());
    }

    fn opening_after(ms: u64, c: &Arc<AtomicUsize>) -> std::thread::JoinHandle<Option<Box<dyn VideoSource>>> {
        let c = c.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(ms));
            Some(fake(&c))
        })
    }

    /// With a deadline (the preview) `video` gives up on a source that is still opening and says
    /// so; `collect` takes it in when it is there. Without one (export) it waits, as it always did.
    #[test]
    fn video_waits_for_an_opening_source_only_until_the_deadline() {
        use std::time::Duration;
        let c = Arc::new(AtomicUsize::new(0));
        let mut pool = DecoderPool::new(Backend::Ffmpeg);
        pool.insert_opening("C:\\slow.png", opening_after(300, &c));
        let start = Instant::now();
        pool.set_deadline(Some(start + Duration::from_millis(20)));
        assert!(pool.video("C:\\slow.png").is_none());
        assert!(start.elapsed() < Duration::from_millis(200), "waited {:?}", start.elapsed());
        assert!(pool.take_missed() && !pool.take_missed() && pool.busy());
        while !pool.collect() {
            assert!(start.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(5));
        }
        pool.set_deadline(Some(Instant::now()));
        assert!(pool.video("C:\\slow.png").is_some() && !pool.take_missed() && !pool.busy());

        pool.set_deadline(None);
        pool.insert_opening("C:\\slow2.png", opening_after(100, &c));
        assert!(pool.video("C:\\slow2.png").is_some(), "no deadline: the open is waited for");
        // taken in by `video` itself, it still counts as arrived: frames that missed it are redone
        assert!(pool.collect() && !pool.collect());
    }

    /// `set_stills` decodes the timeline's stills ahead of use, each at the size it is shown at
    /// and below - and again when that size changes, the old one staying in use meanwhile.
    #[test]
    fn set_stills_decodes_ahead_at_the_size_shown() {
        let png = ffpipe::tests::test_png(); // 64x48
        let settle = |pool: &mut DecoderPool| {
            let start = Instant::now();
            while pool.busy() {
                pool.collect();
                assert!(start.elapsed() < std::time::Duration::from_secs(20));
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        };
        let mut pool = DecoderPool::new(Backend::Ffmpeg);
        pool.set_stills(vec![(png.clone(), ((64, 48), (32, 24)))]);
        assert!(!pool.busy(), "a pool without a budget (export) decodes nothing ahead");
        pool.set_source_cache_bytes(1000);
        pool.set_stills(vec![(png.clone(), ((64, 48), (32, 24)))]);
        assert!(!pool.busy(), "nor is a still that does not fit the budget");

        pool.set_source_cache_bytes(1 << 20);
        pool.set_stills(vec![(png.clone(), ((64, 48), (32, 24)))]);
        assert!(pool.busy());
        settle(&mut pool);
        pool.set_deadline(Some(Instant::now())); // open already: nothing to wait for
        assert_eq!(pool.video(&png).map(|v| v.size()), Some((32, 24)));
        let mut spare = Vec::new();
        let a = pool.frame_arc(&png, 0.0, 16, 12, &mut spare).unwrap();
        let b = pool.frame_arc(&png, 3.0, 16, 12, &mut spare).unwrap();
        assert!(Arc::ptr_eq(&a, &b) && !pool.take_missed(), "the decoder's own frame, every time");
        assert!(a.rgba[2] > 200 && a.rgba[0] < 60, "blue: {:?}", &a.rgba[..4]);

        pool.set_stills(vec![(png.clone(), ((64, 48), (32, 24)))]);
        assert!(!pool.busy(), "open at the right size: nothing to do");
        pool.set_stills(vec![(png.clone(), ((64, 48), (64, 48)))]);
        assert!(pool.busy() && pool.has_video(&png), "shown larger: decoded again, the old one still in use");
        settle(&mut pool);
        assert_eq!(pool.video(&png).map(|v| v.size()), Some((64, 48)));
    }

    /// Stills that hold their pixels are evicted by the bytes they hold, not by the decoder count
    /// (a timeline has more than 16 photos) - on a pool with a budget; elsewhere by count as before.
    #[test]
    fn stills_are_budgeted_by_bytes_on_a_preview_pool() {
        struct Held(usize);
        impl VideoSource for Held {
            fn size(&self) -> (u32, u32) {
                (1, 1)
            }
            fn frame_at(&mut self, _t: f64, _w: u32, _h: u32, _out: &mut Frame) -> bool {
                true
            }
            fn bytes(&self) -> usize {
                self.0
            }
        }
        let path = |i: usize| format!("C:\\s{i}.png");
        let live = |pool: &DecoderPool, n: usize| (0..n).filter(|i| pool.has_video(&path(*i))).count();
        let mut pool = DecoderPool::new(Backend::Ffmpeg);
        for i in 0..20 {
            pool.insert_video(&path(i), Box::new(Held(100)));
        }
        pool.video(&path(0));
        assert_eq!(live(&pool, 20), 19, "no budget: one over the count goes on every use");

        let mut pool = DecoderPool::new(Backend::Ffmpeg);
        pool.set_source_cache_bytes(2000);
        for i in 0..20 {
            pool.insert_video(&path(i), Box::new(Held(100)));
            pool.video(&path(i));
        }
        assert_eq!(live(&pool, 20), 20, "2000 bytes of stills fit, however many they are");
        pool.insert_video(&path(20), Box::new(Held(250)));
        pool.video(&path(20));
        assert_eq!(live(&pool, 21), 18, "250 more bytes: the three least recently used go");
        assert!(!pool.has_video(&path(0)) && !pool.has_video(&path(2)) && pool.has_video(&path(3)));
    }
}
