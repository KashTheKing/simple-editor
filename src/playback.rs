//! Player: owns the render thread (Compositor + DecoderPool → latest Frame) and the audio thread
//! (Mixer + DecoderPool → ring buffer → cpal WASAPI output). The wall clock is the master when playing:
//! time = base_time + elapsed. Video frames are rendered on the project fps grid (t quantized to
//! frame index / fps, exactly like export) and published via `take_frame`; `ctx.request_repaint()`
//! is called whenever a new frame is ready.
//!
//! Finished work is cached in RAM (`Cache`, byte-budgeted LRU — `Settings::cache_mb`, auto-scaled
//! to installed RAM by default): composited frames on the CPU path, decoded layer sets on the GPU
//! path, keyed by frame index. Replays and scrub-backs over recently seen footage are served
//! without touching a decoder. Invalidation is selective where possible: an edit (SetProject)
//! evicts only its dirty spans, a finished proxy (Proxies) only the spans using that source; a
//! canvas resize, backend or GPU-mode switch still clears everything — entries can never go stale.
//! While playing, the pacing gap until the next frame is due is spent pre-rendering upcoming frames
//! into the cache instead of sleeping, so a decode hiccup lands in the prefetch window and not on a
//! visible frame.
//!
//! Commands (mpsc from the UI thread): SetProject, Seek, Play, Pause, Canvas, Backend, ClearDecoders(ack),
//! Quit. The payload (project, clock, canvas) lives in `Shared`, so draining the queue and reading the
//! latest state gives "latest request wins" for free. While paused, every Seek/SetProject/Canvas
//! re-renders the current time; the render thread blocks on the channel when idle (zero CPU).
//! Audio thread mirrors the clock: when playing it keeps ~120 ms of mixed audio ahead in the ring;
//! seek/pause flush the ring. Reaching the project end pauses (the UI sees is_playing() flip).
//!
//! GPU mode (`set_gpu(true)`, settings.gpu + a working `engine::gpu::GpuRenderer`): the render thread
//! stops compositing and instead publishes the **layers** for the current instant (`take_layers` →
//! `engine::gpu::LayerSet`); the UI thread renders them with the GL context it owns.
//! Everything else (clock, pacing, decoder pool, audio) is identical, so falling back to the CPU
//! compositor is a single `set_gpu(false)`.

use crate::engine::compose::{placement, Compositor};
use crate::engine::gpu::LayerSet;
use crate::engine::mixer::Mixer;
use crate::engine::mixer_fx::METER_FEED;
use crate::engine::shapes::ShapeRasterizer;
use crate::engine::text::TextRasterizer;
use crate::media::{Backend, DecoderPool, Frame, SAMPLE_RATE};
use crate::model::{Clip, ClipKind, Project, TrackKind};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Audio mixed per block (frames) and how far ahead of the clock the ring is kept.
const BLOCK: usize = 1024;
const LEAD_SECS: f64 = 0.12;
/// RAM the render thread's finished-work cache may hold until the app pushes the real budget
/// (`Cmd::CacheBudget`, from `Settings::cache_mb` via `cache_budget_bytes`).
const DEFAULT_CACHE_BYTES: usize = 512 << 20;

/// Installed physical RAM, or `None` if the query fails (then the default budget stands).
fn total_ram() -> Option<u64> {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut m = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    unsafe { GlobalMemoryStatusEx(&mut m) }.ok()?;
    Some(m.ullTotalPhys)
}

/// `Settings::cache_mb` → the byte budget the caches run with. `0` = automatic: a quarter of
/// installed RAM, clamped to 512 MB..=4 GB (4K frames are ~33 MB each — a real read-ahead needs
/// gigabytes, but the cache must never crowd out the OS on small machines).
pub fn cache_budget_bytes(cache_mb: u32) -> usize {
    if cache_mb > 0 {
        return (cache_mb as usize) << 20;
    }
    match total_ram() {
        Some(total) => ((total / 4).clamp(512 << 20, 4 << 30)) as usize,
        None => DEFAULT_CACHE_BYTES,
    }
}
/// How far past the playhead the prefetcher keeps frames ready (seconds).
const READ_AHEAD_SECS: f64 = 1.5;
/// Recently shown frames kept eviction-protected behind the playhead (seconds), so a short
/// scrub-back replays from the cache.
const TRAIL_SECS: f64 = 0.5;
/// How far the clock may run past the newest published frame before playback is declared
/// buffering (the UI pauses the clock and shows a spinner until the cache refills).
const STALL_BEHIND: f64 = 0.15;
/// Recycled frame buffers kept for the CPU compositor (cache evictions feed it).
const FPOOL_KEEP: usize = 4;

/// Byte-budgeted LRU keyed by frame index on the project fps grid. The whole cache is cleared on
/// any change that could affect the picture, so entries never go stale.
struct Cache<T> {
    /// frame index -> (last-use tick, payload bytes, payload)
    map: HashMap<i64, (u64, usize, Arc<T>)>,
    bytes: usize,
    tick: u64,
    /// Inclusive index range evicted last (playhead trail + prefetch horizon).
    protect: (i64, i64),
    /// Byte budget evictions keep `bytes` under (`Settings::cache_mb` → `Cmd::CacheBudget`).
    budget: usize,
}

impl<T> Cache<T> {
    fn new(budget: usize) -> Self {
        Cache { map: HashMap::new(), bytes: 0, tick: 0, protect: (i64::MAX, i64::MIN), budget }
    }
    /// Change the byte budget, evicting down immediately (lowering the setting must return RAM now,
    /// not on the next insert).
    fn set_budget(&mut self, budget: usize) -> Vec<Arc<T>> {
        self.budget = budget;
        self.evict_to_budget()
    }
    /// Mean payload size of the current entries — what one cached index really costs, which for a
    /// GPU `LayerSet` is one bitmap PER VISIBLE CLIP, not one canvas frame. `None` while empty or
    /// when every entry is zero-byte (an empty timeline's LayerSets), so callers can fall back to
    /// the canvas-frame estimate without dividing by zero.
    fn avg_entry_bytes(&self) -> Option<usize> {
        if self.map.is_empty() || self.bytes == 0 {
            return None;
        }
        Some(self.bytes / self.map.len())
    }
    /// Frames in [lo, hi] are evicted only when nothing else is left to evict.
    fn set_protect(&mut self, lo: i64, hi: i64) {
        self.protect = (lo, hi);
    }
    fn get(&mut self, idx: i64) -> Option<Arc<T>> {
        self.tick += 1;
        let (t, _, v) = self.map.get_mut(&idx)?;
        *t = self.tick;
        Some(v.clone())
    }
    fn contains(&self, idx: i64) -> bool {
        self.map.contains_key(&idx)
    }
    /// Insert and evict LRU entries down to the budget; evicted payloads come back for buffer recycling.
    fn insert(&mut self, idx: i64, bytes: usize, v: Arc<T>) -> Vec<Arc<T>> {
        self.tick += 1;
        let mut out = Vec::new();
        if let Some((_, b, old)) = self.map.insert(idx, (self.tick, bytes, v)) {
            self.bytes -= b;
            out.push(old);
        }
        self.bytes += bytes;
        out.extend(self.evict_to_budget());
        out
    }
    /// Evict LRU entries (protected range last) until `bytes <= budget`; shared by `insert` and
    /// `set_budget`.
    fn evict_to_budget(&mut self) -> Vec<Arc<T>> {
        let mut out = Vec::new();
        while self.bytes > self.budget {
            // ponytail: O(n) min scan per eviction; n is a few hundred at most. A heap if it ever shows up.
            let (lo, hi) = self.protect;
            let pick = self
                .map
                .iter()
                .filter(|(i, _)| !(lo..=hi).contains(i))
                .min_by_key(|(_, (t, _, _))| *t)
                .or_else(|| self.map.iter().min_by_key(|(_, (t, _, _))| *t));
            let Some((&i, _)) = pick else { break };
            let (_, b, old) = self.map.remove(&i).expect("key from iter");
            self.bytes -= b;
            out.push(old);
        }
        out
    }
    /// Drop every entry whose index lies in [lo, hi], handing payloads back for recycling.
    fn evict_range(&mut self, lo: i64, hi: i64) -> Vec<Arc<T>> {
        let keys: Vec<i64> = self.map.keys().copied().filter(|i| (lo..=hi).contains(i)).collect();
        let mut out = Vec::new();
        for i in keys {
            let (_, b, v) = self.map.remove(&i).expect("key from keys");
            self.bytes -= b;
            out.push(v);
        }
        out
    }
    /// Empty the cache, handing every payload back for buffer recycling.
    fn drain(&mut self) -> Vec<Arc<T>> {
        self.bytes = 0;
        self.map.drain().map(|(_, (_, _, v))| v).collect()
    }
}

/// Wall clock + canvas shared by the UI and both threads. `now()` flips `playing` off at the end.
struct Clock {
    playing: bool,
    base_t: f64,
    base_at: Instant,
    duration: f64,
    /// Render size in pixels (already clamped to the preview max width); (0,0) = nothing to render.
    canvas: (u32, u32),
    // ---- ws:player-rate-loop ----
    /// Shuttle rate: `now() = base_t + elapsed * rate`. 1.0 = normal forward. `set_rate` rejects 0.0
    /// (use `pause` to stop); `play()` forces this back to 1.0 so Stop/Play always resumes forward.
    rate: f64,
    /// `Some((in, out))` loops playback between those timeline seconds instead of stopping at 0/duration.
    loop_range: Option<(f64, f64)>,
    /// Bumped every time `now()` wraps a loop — the audio thread watches this to flush its ring at the
    /// discontinuity instead of playing a stale block across the seam.
    wraps: u64,
}

impl Clock {
    /// Rate- and loop-aware wall-clock read. While playing: `raw = base_t + elapsed * rate`; inside a
    /// loop range it wraps (rebasing `base_t`/`base_at` and bumping `wraps`) instead of stopping;
    /// otherwise it stops at `duration` going forward (`rate >= 0`) or at `0.0` going backward
    /// (`rate < 0`), mirroring the forward-only stop this replaces.
    fn now(&mut self) -> f64 {
        if !self.playing {
            return self.base_t;
        }
        let raw = self.base_t + self.base_at.elapsed().as_secs_f64() * self.rate;
        if let Some((a, b)) = self.loop_range {
            let len = (b - a).max(1e-6);
            if self.rate >= 0.0 {
                if raw < b {
                    return raw;
                }
                let wrapped = a + (raw - b) % len;
                self.base_t = wrapped;
                self.base_at = Instant::now();
                self.wraps += 1;
                return wrapped;
            } else {
                if raw > a {
                    return raw;
                }
                let wrapped = b - (a - raw) % len;
                self.base_t = wrapped;
                self.base_at = Instant::now();
                self.wraps += 1;
                return wrapped;
            }
        }
        if self.rate >= 0.0 {
            if raw < self.duration {
                return raw;
            }
            self.playing = false;
            self.base_t = self.duration;
            self.duration
        } else {
            if raw > 0.0 {
                return raw;
            }
            self.playing = false;
            self.base_t = 0.0;
            0.0
        }
    }
}

struct Shared {
    clock: Mutex<Clock>,
    /// Newest rendered frame not yet taken by the UI.
    frame: Mutex<Option<Arc<Frame>>>,
    /// GPU mode: newest decoded layer set not yet taken by the UI (no CPU compositing happened).
    layers: Mutex<Option<Arc<LayerSet>>>,
    project: Mutex<Arc<Project>>,
    /// Frames/layer sets served from the RAM cache instead of being rendered (observability + tests).
    cache_hits: AtomicU64,
    /// Playback fell behind decode: the UI should pause the clock and show a spinner until cleared.
    buffering: AtomicBool,
    // ---- ws:player-rate-loop ----
    /// Latest async layer request from `Player::request_layers`: (request id, time, max width).
    req: Mutex<Option<(u64, f64, u32)>>,
    /// The reply to the newest-processed `req`, polled (and consumed) by `Player::take_layers_reply`.
    reply: Mutex<Option<(u64, Arc<LayerSet>)>>,
    /// Cumulative genuinely-dropped frames (direction-normalized; see `dropped_delta`).
    dropped: AtomicU64,
    /// Allocator for `request_layers`' request ids.
    next_req: AtomicU64,
}

/// Poison-tolerant lock: a panicking worker must never take the UI down with it.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Clone)]
enum Cmd {
    SetProject,
    Seek,
    Play,
    Pause,
    Canvas,
    Backend(Backend),
    /// Publish decoded layers instead of a composited frame (the UI renders them on the GPU).
    Gpu(bool),
    /// Preview decode reads these proxy files instead of the originals (source path -> proxy path).
    Proxies(HashMap<String, String>),
    /// New byte budget for the finished-work caches (and, at a quarter of it, the decoded-source
    /// cache on the preview pool). From `Settings::cache_mb` via `cache_budget_bytes`.
    CacheBudget(usize),
    ClearDecoders(SyncSender<()>),
    /// One-shot render at time t, at most max_w px wide (project aspect), reply on the channel.
    RenderOnce(f64, u32, SyncSender<Arc<Frame>>),
    /// One-shot layer decode at time t (GPU path: the UI thread renders them), reply on the channel.
    LayersOnce(f64, u32, SyncSender<Arc<LayerSet>>),
    // ---- ws:player-rate-loop ----
    /// The shuttle rate changed (render-thread: marks dirty so the next pass republishes at the new
    /// pace). No payload — both threads re-read `Clock::rate` fresh; this is only a wake-up signal for
    /// a thread that was idle-blocked on the channel.
    Rate,
    /// The loop range changed (render/audio threads re-read `Clock::loop_range` fresh each pass); same
    /// wake-up-only shape as `Rate`.
    Loop,
    /// Audio-only: mix one BLOCK at this timeline time into the ring without touching the clock.
    Scrub(f64),
    /// Render-thread only: decode the layers for `Shared::req` and publish the reply, non-blocking.
    LayersAsync,
    Quit,
}

pub struct Player {
    shared: Arc<Shared>,
    render: Sender<Cmd>,
    audio: Sender<Cmd>,
}

impl Player {
    pub fn new(ctx: eframe::egui::Context, backend: Backend, text: Arc<Mutex<TextRasterizer>>) -> Self {
        let shared = Arc::new(Shared {
            clock: Mutex::new(Clock {
                playing: false,
                base_t: 0.0,
                base_at: Instant::now(),
                duration: 0.0,
                canvas: (0, 0),
                rate: 1.0,
                loop_range: None,
                wraps: 0,
            }),
            frame: Mutex::new(None),
            layers: Mutex::new(None),
            project: Mutex::new(Arc::new(Project::new())),
            cache_hits: AtomicU64::new(0),
            buffering: AtomicBool::new(false),
            req: Mutex::new(None),
            reply: Mutex::new(None),
            dropped: AtomicU64::new(0),
            next_req: AtomicU64::new(0),
        });
        let (render, rx) = mpsc::channel();
        let s = shared.clone();
        let _ =
            std::thread::Builder::new().name("render".into()).spawn(move || render_thread(s, rx, ctx, backend, text));
        let (audio, rx) = mpsc::channel();
        let s = shared.clone();
        let _ = std::thread::Builder::new().name("audio".into()).spawn(move || audio_thread(s, rx, backend));
        Self { shared, render, audio }
    }
    fn both(&self, c: Cmd) {
        let _ = self.audio.send(c.clone());
        let _ = self.render.send(c);
    }
    /// Replace the project (cheap clone; called after every edit). Re-renders the current frame.
    pub fn set_project(&mut self, project: &Project) {
        *lock(&self.shared.project) = Arc::new(project.clone());
        lock(&self.shared.clock).duration = project.duration();
        self.both(Cmd::SetProject);
    }
    pub fn set_backend(&mut self, b: Backend) {
        self.both(Cmd::Backend(b));
    }
    /// GPU preview: the render thread publishes decoded layers (`take_layers`) instead of a composited
    /// frame. Switching back to false resumes the CPU compositor on the next render.
    pub fn set_gpu(&mut self, on: bool) {
        let _ = self.render.send(Cmd::Gpu(on));
    }
    /// Byte budget for the render thread's caches (see `cache_budget_bytes`). Render-only, like
    /// `set_gpu`/`set_proxies` — the audio thread caches nothing.
    pub fn set_cache_bytes(&mut self, bytes: usize) {
        let _ = self.render.send(Cmd::CacheBudget(bytes));
    }
    /// Preview canvas size in pixels (the Player clamps width to `max_width`, keeping aspect).
    pub fn set_canvas(&mut self, w: u32, h: u32, max_width: u32) {
        let (w, h) = if max_width > 0 && w > max_width {
            (max_width, ((h as u64 * max_width as u64) / w as u64).max(1) as u32)
        } else {
            (w, h)
        };
        let mut c = lock(&self.shared.clock);
        if c.canvas != (w, h) {
            c.canvas = (w, h);
            drop(c);
            let _ = self.render.send(Cmd::Canvas);
        }
    }
    pub fn play(&mut self) {
        {
            let mut c = lock(&self.shared.clock);
            let t = c.now();
            c.base_t = if t >= c.duration - 1e-6 { 0.0 } else { t };
            c.base_at = Instant::now();
            c.playing = true;
            // ws:player-rate-loop: Play always resumes forward at 1x, regardless of a prior shuttle
            // rate — Stop (pause()) leaves `rate` alone (now() ignores it while !playing), so without
            // this a bare Space after J J J would silently resume playing backward at -4x.
            c.rate = 1.0;
        }
        self.both(Cmd::Play);
    }
    pub fn pause(&mut self) {
        {
            let mut c = lock(&self.shared.clock);
            c.base_t = c.now();
            c.playing = false;
        }
        self.both(Cmd::Pause);
    }
    pub fn toggle(&mut self) {
        if self.is_playing() {
            self.pause()
        } else {
            self.play()
        }
    }
    pub fn is_playing(&self) -> bool {
        let mut c = lock(&self.shared.clock);
        c.now();
        c.playing
    }
    /// Current timeline time (advances while playing).
    pub fn time(&self) -> f64 {
        lock(&self.shared.clock).now()
    }
    /// Clamp to [0, duration] and re-render if paused (keeps playing from `t` if playing).
    pub fn seek(&mut self, t: f64) {
        {
            let mut c = lock(&self.shared.clock);
            c.base_t = t.clamp(0.0, c.duration.max(0.0));
            c.base_at = Instant::now();
        }
        self.both(Cmd::Seek);
    }
    /// Newest rendered frame not yet taken (UI uploads it to a texture).
    pub fn take_frame(&mut self) -> Option<Arc<Frame>> {
        lock(&self.shared.frame).take()
    }
    /// GPU mode only: the newest decoded layers not yet taken (the UI renders them with `engine::gpu`).
    /// Always None while the CPU compositor is running.
    pub fn take_layers(&mut self) -> Option<Arc<LayerSet>> {
        lock(&self.shared.layers).take()
    }
    /// True while the render thread has fallen behind decode and is refilling its read-ahead.
    /// The UI pauses the clock (audio too) and shows a spinner until this clears.
    pub fn is_buffering(&self) -> bool {
        self.shared.buffering.load(Ordering::Relaxed)
    }
    /// How many frames/layer sets have been served from the RAM cache instead of rendered.
    pub fn cache_hits(&self) -> u64 {
        self.shared.cache_hits.load(Ordering::Relaxed)
    }
    /// Synchronously render the timeline at `t`, at most `max_w` px wide (project aspect kept), on the
    /// render thread (its decoders stay warm). None if the thread is gone or takes longer than 3 s.
    /// Not a hot path (MCP `render.frame` tool): allocates a fresh frame per call.
    pub fn render_once(&self, t: f64, max_w: u32) -> Option<Arc<Frame>> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.render.send(Cmd::RenderOnce(t, max_w, tx)).ok()?;
        rx.recv_timeout(Duration::from_secs(3)).ok()
    }
    /// Synchronously decode the layers at `t` (at most `max_w` px wide) on the render thread, for a
    /// one-off GPU render on the UI thread (export-frame, MCP tools). None if it takes longer than 3 s.
    pub fn layers_once(&self, t: f64, max_w: u32) -> Option<Arc<LayerSet>> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.render.send(Cmd::LayersOnce(t, max_w, tx)).ok()?;
        rx.recv_timeout(Duration::from_secs(3)).ok()
    }
    /// Swap the preview proxy map (source path -> proxy file). Video only: the render thread's pool
    /// re-resolves every decode through it; audio always reads the originals.
    pub fn set_proxies(&mut self, map: HashMap<String, String>) {
        let _ = self.render.send(Cmd::Proxies(map));
    }
    /// Synchronously drop every decoder on both threads (before overwriting a source file).
    pub fn release_files(&mut self) {
        let (tx, rx) = mpsc::sync_channel(2);
        self.both(Cmd::ClearDecoders(tx));
        let deadline = Instant::now() + Duration::from_secs(2);
        for _ in 0..2 {
            // a dead thread drops its sender → Disconnected → we stop waiting early
            if rx.recv_timeout(deadline.saturating_duration_since(Instant::now())).is_err() {
                break;
            }
        }
    }

    // ---- ws:player-rate-loop ----

    /// Rebase the clock at the current time with a new shuttle rate. Rejects (no-ops) exactly 0.0 —
    /// use `pause` to stop. Negative = reverse; magnitude != 1.0 = fast/slow forward or reverse.
    pub fn set_rate(&mut self, rate: f64) {
        if rate == 0.0 {
            return;
        }
        {
            let mut c = lock(&self.shared.clock);
            let t = c.now();
            c.base_t = t;
            c.base_at = Instant::now();
            c.rate = rate;
        }
        self.both(Cmd::Rate);
    }
    /// Current shuttle rate (1.0 = normal forward).
    pub fn rate(&self) -> f64 {
        lock(&self.shared.clock).rate
    }
    /// Enable (`Some((in, out))`) or disable (`None`) Loop In->Out. If the playhead is currently outside
    /// the new range, seeks to `in` first — otherwise `Clock::now`'s wrap math would land the next tick
    /// at an unpredictable point inside the range (a modulo of however far past `out` playback had
    /// drifted) instead of a clean loop start.
    pub fn set_loop(&mut self, range: Option<(f64, f64)>) {
        if let Some((a, b)) = range {
            let t = self.time();
            if t < a || t >= b {
                self.seek(a);
            }
        }
        {
            lock(&self.shared.clock).loop_range = range;
        }
        self.both(Cmd::Loop);
    }
    /// The current loop range, if looping.
    pub fn loop_range(&self) -> Option<(f64, f64)> {
        lock(&self.shared.clock).loop_range
    }
    /// Frame-accurate relative seek: pauses, then moves by `frames` (negative = back) at `fps`.
    pub fn step(&mut self, frames: i64, fps: f64) {
        self.pause();
        let t = self.time() + frames as f64 / fps.max(1.0);
        self.seek(t);
    }
    /// Mix exactly one BLOCK (~21 ms @ 48 kHz/1024) of audio at `t` into the ring without moving the
    /// clock — audio-only, for a paused playhead scrub. Audio-thread command only (not `both`).
    pub fn scrub(&mut self, t: f64) {
        let _ = self.audio.send(Cmd::Scrub(t));
    }
    /// Cumulative genuinely-dropped frames (direction-normalized: intentional rate-driven index
    /// skipping never counts), for the wave-2 rate/dropped-frame badge.
    pub fn dropped_frames(&self) -> u64 {
        self.shared.dropped.load(Ordering::Relaxed)
    }
    /// Queue a non-blocking one-shot layer decode at `t` (at most `max_w` px wide); returns a request
    /// id. Newest request always wins — an in-flight older request's reply is never surfaced once a
    /// newer one has been queued. Never blocks the caller (unlike `layers_once`).
    pub fn request_layers(&self, t: f64, max_w: u32) -> u64 {
        let id = self.shared.next_req.fetch_add(1, Ordering::Relaxed) + 1;
        *lock(&self.shared.req) = Some((id, t, max_w));
        let _ = self.render.send(Cmd::LayersAsync);
        id
    }
    /// Poll for `request_layers`' async reply (consumes it). `None` until ready, or forever for a
    /// request superseded by a newer one before it was decoded.
    pub fn take_layers_reply(&self) -> Option<(u64, Arc<LayerSet>)> {
        lock(&self.shared.reply).take()
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.both(Cmd::Quit);
    }
}

/// Run one decode+compose/mix under `catch_unwind`: a panicking decoder must not kill the render or audio
/// thread for the rest of the session (the UI would freeze on the last frame with no error). On a panic the
/// decoders are dropped so the next call starts clean. False = the call panicked and its output is garbage.
fn guarded(pool: &mut DecoderPool, f: impl FnOnce(&mut DecoderPool)) -> bool {
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(pool))).is_ok() {
        return true;
    }
    pool.clear();
    false
}

fn render_thread(
    shared: Arc<Shared>,
    rx: Receiver<Cmd>,
    ctx: eframe::egui::Context,
    backend: Backend,
    text: Arc<Mutex<TextRasterizer>>,
) {
    let mut pool = DecoderPool::new(backend);
    let mut comp = Compositor::new();
    let mut shapes = ShapeRasterizer::new();
    let mut project = lock(&shared.project).clone();
    let mut dirty = false;
    let mut pending: Option<Cmd> = None;
    // GPU mode state + recycled layer buffers
    let mut gpu = false;
    let mut spare_layers: Vec<Frame> = Vec::new();
    // finished-work caches, keyed by frame index on the fps grid (see module docs), plus a small pool
    // of recycled Frame buffers for the CPU compositor (fed by cache evictions)
    let mut budget = DEFAULT_CACHE_BYTES; // replaced by Cmd::CacheBudget right after startup
    let mut fcache: Cache<Frame> = Cache::new(budget);
    let mut lcache: Cache<LayerSet> = Cache::new(budget);
    let mut fpool: Vec<Frame> = Vec::new();
    let mut last_pub: i64 = -1;
    // buffering: the clock outran decode; free-run the prefetcher (even while the UI pauses the
    // clock) until the read-ahead is back, instead of live-rendering straight into every stall
    let mut stall = false;
    // clear both caches, recycling their buffers (`drain` so a per-edit clear does not thrash the allocator)
    macro_rules! clear_caches {
        () => {
            for old in fcache.drain() {
                reclaim(old, &mut fpool);
            }
            for old in lcache.drain() {
                recycle(Some(old), &mut spare_layers);
            }
            last_pub = -1;
        };
    }
    // evict only the frame indices covering the given `(start, end)` second spans (±1 frame slack,
    // matching the span math's float fuzz), recycling the buffers — the selective alternative to
    // clear_caches! for edits (SetProject) and proxy swaps (Proxies)
    macro_rules! evict_spans {
        ($spans:expr, $fps:expr) => {
            for (a, b) in $spans {
                let (lo, hi) = ((a * $fps).floor() as i64 - 1, (b * $fps).ceil() as i64 + 1);
                for old in fcache.evict_range(lo, hi) {
                    reclaim(old, &mut fpool);
                }
                for old in lcache.evict_range(lo, hi) {
                    recycle(Some(old), &mut spare_layers);
                }
            }
            last_pub = -1; // the current frame may sit in an evicted span: republish
        };
    }
    loop {
        let playing = {
            let mut c = lock(&shared.clock);
            c.now();
            c.playing
        };
        if pending.is_none() && !dirty && !playing && !stall {
            let Ok(c) = rx.recv() else { return }; // idle: block, zero CPU
            pending = Some(c);
        }
        // drain everything queued; the shared state already holds the latest values
        loop {
            let c = match pending.take() {
                Some(c) => c,
                None => match rx.try_recv() {
                    Ok(c) => c,
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return,
                },
            };
            match c {
                Cmd::SetProject => {
                    let new = lock(&shared.project).clone();
                    dirty = true;
                    // evict only the frames the edit can have changed; None = anything could differ
                    match video_dirty_spans(&project, &new) {
                        Some(spans) => {
                            evict_spans!(spans, new.fps.max(1.0));
                        }
                        None => {
                            clear_caches!();
                        }
                    }
                    project = new;
                }
                Cmd::Seek => {
                    dirty = true;
                    // a seek can jump the timeline index by any amount, including while still playing
                    // (Player::seek keeps playing from `t`) — without this, dropped_delta(last_pub, idx,
                    // rate) reads the whole jump as decode falling behind, not an intentional skip.
                    last_pub = -1;
                }
                Cmd::Play | Cmd::Pause => dirty = true,
                Cmd::Canvas => {
                    dirty = true;
                    clear_caches!(); // cached entries are the old canvas size
                }
                Cmd::Backend(b) => {
                    pool.set_backend(b);
                    dirty = true;
                    clear_caches!();
                }
                Cmd::Gpu(on) => {
                    if gpu != on {
                        gpu = on;
                        *lock(&shared.layers) = None;
                        dirty = true;
                        clear_caches!();
                    }
                }
                Cmd::Proxies(map) => {
                    // only the sources whose mapping actually changed re-decode — a proxy finishing
                    // for clip B must not throw away clip A's read-ahead. Ordering is safe: Proxies
                    // and SetProject share this FIFO, so a map built against a newer project than
                    // `project` is healed by the SetProject queued right behind it (whose dirty
                    // spans / full clear cover whatever moved).
                    let changed = pool.set_proxies(map);
                    if !changed.is_empty() {
                        let fps = project.fps.max(1.0);
                        let spans: Vec<(f64, f64)> =
                            changed.iter().flat_map(|src| spans_using_source(&project, src)).collect();
                        evict_spans!(spans, fps);
                        dirty = true;
                    }
                }
                Cmd::CacheBudget(n) => {
                    budget = n;
                    for old in fcache.set_budget(n) {
                        reclaim(old, &mut fpool);
                    }
                    for old in lcache.set_budget(n) {
                        recycle(Some(old), &mut spare_layers);
                    }
                    // a quarter of the finished-work budget for decoded source frames (fix for
                    // "reloading the footage" on scrubs — see DecoderPool::frame_at)
                    pool.set_source_cache_bytes(n / 4);
                }
                Cmd::ClearDecoders(ack) => {
                    pool.clear();
                    clear_caches!(); // the caller is about to overwrite a source file
                    let _ = ack.send(());
                }
                Cmd::LayersOnce(t, max_w, reply) => {
                    let (pw, ph) = (project.width.max(1), project.height.max(1));
                    let w = pw.min(max_w.max(16));
                    let h = ((ph as u64 * w as u64) / pw as u64).max(1) as u32;
                    let mut set = LayerSet::default();
                    let text = &mut lock(&text);
                    guarded(&mut pool, |pool| {
                        set = decode_layers(&project, t, w, h, pool, &mut spare_layers, text, &mut shapes, &mut comp)
                    });
                    let _ = reply.send(Arc::new(set));
                }
                Cmd::RenderOnce(t, max_w, reply) => {
                    let (pw, ph) = (project.width.max(1), project.height.max(1));
                    let w = pw.min(max_w.max(16));
                    let h = ((ph as u64 * w as u64) / pw as u64).max(1) as u32;
                    let mut f = Frame::default();
                    guarded(&mut pool, |pool| comp.render(&project, t, w, h, pool, &mut lock(&text), &mut f));
                    let _ = reply.send(Arc::new(f));
                }
                // ---- ws:player-rate-loop ----
                Cmd::Rate => dirty = true,
                Cmd::Loop => {} // loop_range is read fresh from Clock each pass
                Cmd::LayersAsync => {
                    // non-blocking mirror of LayersOnce: decode the latest request and publish the reply
                    if let Some((id, t, max_w)) = *lock(&shared.req) {
                        let (pw, ph) = (project.width.max(1), project.height.max(1));
                        let w = pw.min(max_w.max(16));
                        let h = ((ph as u64 * w as u64) / pw as u64).max(1) as u32;
                        let mut set = LayerSet::default();
                        let text = &mut lock(&text);
                        guarded(&mut pool, |pool| {
                            set =
                                decode_layers(&project, t, w, h, pool, &mut spare_layers, text, &mut shapes, &mut comp)
                        });
                        *lock(&shared.reply) = Some((id, Arc::new(set)));
                    }
                }
                Cmd::Scrub(_) => {} // audio-thread only
                Cmd::Quit => return,
            }
        }
        let (playing, t, (w, h), duration, rate) = {
            let mut c = lock(&shared.clock);
            let t = c.now();
            (c.playing, t, c.canvas, c.duration, c.rate)
        };
        if !playing && !dirty && !stall {
            continue;
        }
        let force = std::mem::take(&mut dirty);
        let fps = project.fps.max(1.0);
        if w == 0 || h == 0 {
            stall = false;
            shared.buffering.store(false, Ordering::Relaxed);
            if playing {
                std::thread::sleep(Duration::from_secs_f64(1.0 / fps)); // nothing to render, keep pace
            }
            continue;
        }
        // quantize to the fps grid (same formula as ffpipe): one render per frame index, cache-keyable
        let idx = (t * fps + 1e-6).floor() as i64;
        // prefetch horizon: READ_AHEAD_SECS of frames, capped so the window can't blow the cache
        // budget on its own. One index is costed at the ACTIVE cache's measured average — a GPU
        // LayerSet is one bitmap per visible clip, so the old one-canvas-frame estimate oversized
        // the window on multi-layer timelines and the inserts then evicted inside their own
        // protected range (churn). Canvas size is the fallback while the cache is empty.
        let frame_bytes = (w as usize * h as usize * 4).max(1);
        let avg = if gpu { lcache.avg_entry_bytes() } else { fcache.avg_entry_bytes() };
        let per_idx = avg.unwrap_or(frame_bytes).max(1);
        let read_ahead = ((fps * READ_AHEAD_SECS).ceil() as i64).clamp(8, (budget / 2 / per_idx).max(8) as i64);
        let last_idx = (((duration * fps).ceil() as i64) - 1).max(idx);
        // keep the trail + horizon around the playhead eviction-protected: a scrub-back replays free
        // (direction-aware: reverse shuttling protects what's already been shown behind idx instead)
        let trail = (fps * TRAIL_SECS).ceil() as i64;
        let (protect_lo, protect_hi) = protect_bounds(idx, trail, read_ahead, rate);
        fcache.set_protect(protect_lo, protect_hi);
        lcache.set_protect(protect_lo, protect_hi);
        macro_rules! cached {
            ($i:expr) => {
                if gpu {
                    lcache.contains($i)
                } else {
                    fcache.contains($i)
                }
            };
        }
        // buffering: the clock ran STALL_BEHIND past the newest published frame and the due frame
        // still isn't cached — declare a stall instead of grinding out one late frame at a time.
        // The UI polls is_buffering(), pauses the clock (audio flushes with it) and shows a spinner.
        if playing && !stall && last_pub >= 0 && idx > last_pub {
            let behind = t - (last_pub + 1) as f64 / fps;
            if behind > STALL_BEHIND && !cached!(idx) {
                stall = true;
                shared.buffering.store(true, Ordering::Relaxed);
                ctx.request_repaint();
            }
        }
        if stall {
            // free-run the prefetcher: one frame per loop pass so commands stay responsive
            // (direction-aware: nearest-to-idx first, stride-sampled at high |rate|)
            let (rlo, rhi) = read_ahead_window(idx, read_ahead, last_idx, rate);
            if let Some(i) = prefetch_order(rlo, rhi, rate).into_iter().find(|i| !cached!(*i)) {
                let ok = if gpu {
                    gpu_cached(
                        &mut lcache,
                        &mut spare_layers,
                        &project,
                        i,
                        fps,
                        w,
                        h,
                        &mut pool,
                        &text,
                        &mut shapes,
                        &mut comp,
                        &shared.cache_hits,
                    )
                    .1
                } else {
                    cpu_cached(
                        &mut fcache,
                        &mut fpool,
                        &project,
                        i,
                        fps,
                        w,
                        h,
                        &mut pool,
                        &text,
                        &mut comp,
                        &shared.cache_hits,
                    )
                    .1
                };
                if !ok {
                    stall = false; // a decode panicked and the pool was cleared: give up on this stall
                    shared.buffering.store(false, Ordering::Relaxed);
                    ctx.request_repaint();
                }
            }
            // enough contiguous frames ready (hysteresis: a third of the horizon) -> resume
            let (glo, ghi) = read_ahead_window(idx, (read_ahead / 3).max(2), last_idx, rate);
            if stall && (glo..=ghi).all(|i| cached!(i)) {
                stall = false;
                shared.buffering.store(false, Ordering::Relaxed);
                dirty = true; // republish the (now cached) due frame on the next pass
                ctx.request_repaint();
            }
            continue; // re-drain commands between decodes; never live-render while buffering
        }
        if force || idx != last_pub {
            if gpu {
                // the UI thread owns the GL context: decode/rasterise here, render there
                let (set, _) = gpu_cached(
                    &mut lcache,
                    &mut spare_layers,
                    &project,
                    idx,
                    fps,
                    w,
                    h,
                    &mut pool,
                    &text,
                    &mut shapes,
                    &mut comp,
                    &shared.cache_hits,
                );
                *lock(&shared.layers) = Some(set);
            } else {
                let (frame, _) = cpu_cached(
                    &mut fcache,
                    &mut fpool,
                    &project,
                    idx,
                    fps,
                    w,
                    h,
                    &mut pool,
                    &text,
                    &mut comp,
                    &shared.cache_hits,
                );
                *lock(&shared.frame) = Some(frame); // replaces an untaken (stale) frame: latest wins
            }
            if playing && last_pub >= 0 {
                shared.dropped.fetch_add(dropped_delta(last_pub, idx, rate), Ordering::Relaxed);
            }
            last_pub = idx;
            ctx.request_repaint();
        }
        if playing {
            // Pacing window until frame idx+1 is due on the grid: pre-render upcoming frames into the
            // cache, then sleep out whatever is left. A slow prefetch can overrun the window by one
            // render — the same stall the live path would have hit at that frame's due time, just earlier.
            // If we are behind, remain <= 0 and we render the current clock frame next (never queue up).
            // ponytail: thread::sleep uses Windows' high-resolution waitable timer; recv_timeout rounds to
            // the 15.6 ms scheduler tick (30 fps -> ~21 fps). timeBeginPeriod(1) would also work but costs power.
            loop {
                // next frame due in the direction of travel; dividing by the signed rate keeps this
                // identical to the old `(idx+1)/fps - now()` at rate=1.0 while flipping sign for reverse
                let target_idx = if rate >= 0.0 { idx + 1 } else { idx - 1 };
                let denom = if rate >= 0.0 { rate.max(1e-6) } else { rate.min(-1e-6) };
                let remain = (target_idx as f64 / fps - lock(&shared.clock).now()) / denom;
                if remain <= 0.0 {
                    break;
                }
                let (rlo, rhi) = read_ahead_window(idx, read_ahead, last_idx, rate);
                let missing = prefetch_order(rlo, rhi, rate)
                    .into_iter()
                    .skip(1) // idx itself is already rendered this pass
                    .find(|i| if gpu { !lcache.contains(*i) } else { !fcache.contains(*i) });
                let Some(i) = missing else {
                    std::thread::sleep(Duration::from_secs_f64(remain));
                    break;
                };
                let ok = if gpu {
                    gpu_cached(
                        &mut lcache,
                        &mut spare_layers,
                        &project,
                        i,
                        fps,
                        w,
                        h,
                        &mut pool,
                        &text,
                        &mut shapes,
                        &mut comp,
                        &shared.cache_hits,
                    )
                    .1
                } else {
                    cpu_cached(
                        &mut fcache,
                        &mut fpool,
                        &project,
                        i,
                        fps,
                        w,
                        h,
                        &mut pool,
                        &text,
                        &mut comp,
                        &shared.cache_hits,
                    )
                    .1
                };
                if !ok {
                    break; // a decode panicked and the pool was cleared: stop prefetching this window
                }
            }
        }
    }
}

// ---- ws:player-rate-loop ----

/// Cache eviction-protect window around `idx`, direction-aware: forward keeps `trail` frames behind
/// and `read_ahead` ahead (unchanged from the old forward-only formula); reverse swaps which side is
/// "ahead" — `read_ahead` frames toward 0 and `trail` frames back toward where playback came from.
fn protect_bounds(idx: i64, trail: i64, read_ahead: i64, rate: f64) -> (i64, i64) {
    if rate >= 0.0 {
        (idx - trail, idx + read_ahead)
    } else {
        (idx - read_ahead, idx + trail)
    }
}

/// Prefetch bounds, direction-aware: `read_ahead` frames beyond `idx` in the direction of travel,
/// clamped to `[0, last_idx]`. One of the two returned bounds always equals `idx`.
fn read_ahead_window(idx: i64, read_ahead: i64, last_idx: i64, rate: f64) -> (i64, i64) {
    if rate >= 0.0 {
        (idx, (idx + read_ahead).min(last_idx))
    } else {
        ((idx - read_ahead).max(0), idx)
    }
}

/// Nearest-first, stride-sampled frame indices covering `[lo, hi]` — the order the prefetcher fills
/// them in. Stride is `|rate|` rounded (minimum 1, so |rate| <= 1 samples every frame); forward scans
/// from `lo` upward, reverse (`rate < 0`) scans from `hi` downward (nearest to the playhead first, since
/// `read_ahead_window` puts `idx` at `hi` for a reverse window).
fn prefetch_order(lo: i64, hi: i64, rate: f64) -> Vec<i64> {
    let stride = rate.abs().round().max(1.0) as i64;
    let mut out = Vec::new();
    if rate >= 0.0 {
        let mut i = lo;
        while i <= hi {
            out.push(i);
            i += stride;
        }
    } else {
        let mut i = hi;
        while i >= lo {
            out.push(i);
            i -= stride;
        }
    }
    out
}

/// Genuinely-dropped frames between two published indices, direction-normalized so a reverse-shuttle
/// stall is counted instead of silently zeroed by a raw negative jump: `jump` is the published-index
/// delta rotated into "frames of intended travel" (multiplying by `rate`'s sign), `stride` is how many
/// frames one publish is expected to advance at this rate (intentional skipping, never a drop). Only
/// the excess over `stride` counts as dropped.
fn dropped_delta(prev_idx: i64, idx: i64, rate: f64) -> u64 {
    if prev_idx < 0 {
        return 0; // no prior publish to measure a jump from
    }
    let sign = if rate == 0.0 { 1.0 } else { rate.signum() };
    let jump = (idx - prev_idx) as f64 * sign;
    let stride = rate.abs().round().max(1.0);
    (jump.round() as i64 - stride as i64).max(0) as u64
}

/// Timeline spans (seconds) whose cached frames the edit `old` -> `new` can have changed.
/// None = the change can affect any frame (or is too entangled to bound): clear everything.
/// Edits that cannot change pixels (markers, in/out points, audio tracks, buses, planner, notes,
/// labels, folders) produce no spans at all — the whole cache survives them.
fn video_dirty_spans(old: &Project, new: &Project) -> Option<Vec<(f64, f64)>> {
    // serde_json string equality instead of a PartialEq derive cascade across the whole model
    fn json<T: serde::Serialize>(v: &T) -> String {
        serde_json::to_string(v).unwrap_or_default()
    }
    // trivially frame-global state: any difference clears everything
    if old.width != new.width
        || old.height != new.height
        || old.fps != new.fps
        || old.editing != new.editing
        || json(&old.scaler) != json(&new.scaler)
        || old.show_subtitles != new.show_subtitles
        || old.tracks.len() != new.tracks.len()
        || json(&old.assets) != json(&new.assets)
        || json(&old.sequences) != json(&new.sequences)
    {
        return None;
    }
    let mut spans: Vec<(f64, f64)> = Vec::new();
    let clip_span = |c: &Clip| (c.start, c.start + c.duration.max(0.0));
    // subtitle style/margin or cue edits dirty every cue window from both sides (cues are few)
    if json(&old.subtitle_style) != json(&new.subtitle_style)
        || old.subtitle_margin != new.subtitle_margin
        || json(&old.subtitles) != json(&new.subtitles)
    {
        for c in old.subtitles.iter().chain(&new.subtitles) {
            spans.push((c.start, c.end));
        }
    }
    for (ot, nt) in old.tracks.iter().zip(&new.tracks) {
        // ws:player-rate-loop: a reorder (same ids, different index) can flip z-order between two
        // video tracks — bound spans computed against the OLD index assignment would be wrong for the
        // new one, so treat any id mismatch at an index as unbounded (pro-timeline, wave 3, will allow
        // track reordering and depends on this invariant already existing).
        if ot.id != nt.id {
            return None;
        }
        if ot.kind != nt.kind {
            return None;
        }
        if ot.kind != TrackKind::Video {
            continue; // audio-only edits never touch a rendered frame
        }
        // mute/solo flips change visibility rules across tracks: not worth bounding
        if ot.muted != nt.muted || ot.solo != nt.solo {
            return None;
        }
        // a transition edit repaints its window; the exact window needs the clip pair, so take the
        // track's whole span (transition edits are rare; clip edits below stay tightly bounded)
        if json(&ot.transitions) != json(&nt.transitions) {
            spans.push((0.0, ot.end().max(nt.end())));
            continue;
        }
        let olds: HashMap<crate::model::Id, &Clip> = ot.clips.iter().map(|c| (c.id, c)).collect();
        let news: HashMap<crate::model::Id, &Clip> = nt.clips.iter().map(|c| (c.id, c)).collect();
        for c in &ot.clips {
            match news.get(&c.id) {
                None => spans.push(clip_span(c)), // removed
                Some(n) => {
                    if json(*n) != json(c) {
                        spans.push(clip_span(c));
                        spans.push(clip_span(n));
                    }
                }
            }
        }
        for c in nt.clips.iter().filter(|c| !olds.contains_key(&c.id)) {
            spans.push(clip_span(c)); // added
        }
    }
    Some(spans)
}

/// Timeline spans (seconds) whose rendered pixels can involve the source file at `path` — the
/// eviction set for a proxy swap (`Cmd::Proxies`): clips using a matching asset directly, clips
/// whose node graph samples it (`NodeKind::Asset`, mirrored from `layer_for`), and any sequence
/// clip whose nested content uses it — that clip's WHOLE span, since inverting the nested retime
/// buys nothing over a slightly wider evict. Every span is widened by the longest transition in
/// the project (a transition draws both neighbours past their own bounds). Path match is
/// ASCII-case-insensitive, like `Project::asset_by_path`.
fn spans_using_source(project: &Project, path: &str) -> Vec<(f64, f64)> {
    use crate::model::{Id, NodeKind};
    let ids: Vec<Id> = project.assets.iter().filter(|a| a.path.eq_ignore_ascii_case(path)).map(|a| a.id).collect();
    if ids.is_empty() {
        return Vec::new();
    }
    let uses = |c: &Clip| {
        (c.uses_asset() && ids.contains(&c.asset))
            || c.graph
                .iter()
                .flat_map(|g| g.nodes.iter())
                .any(|n| matches!(n.kind, NodeKind::Asset(a) if ids.contains(&a)))
    };
    // does this sequence, recursively, contain a clip that uses the asset? (shape + depth guard
    // mirror Project::sequence_contains; sequence_tracks honours the open-for-editing swap)
    fn seq_uses(project: &Project, seq: Id, uses: &dyn Fn(&Clip) -> bool, depth: usize) -> bool {
        if depth > 32 {
            return false;
        }
        let Some(tracks) = project.sequence_tracks(seq) else { return false };
        tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .any(|c| uses(c) || (c.kind == ClipKind::Sequence && seq_uses(project, c.sequence, uses, depth + 1)))
    }
    let widen = project.tracks.iter().flat_map(|t| t.transitions.iter()).map(|tr| tr.duration).fold(0.0f64, f64::max);
    let mut spans = Vec::new();
    for (_, c) in project.all_clips() {
        if uses(c) || (c.kind == ClipKind::Sequence && seq_uses(project, c.sequence, &uses, 0)) {
            spans.push(((c.start - widen).max(0.0), c.end() + widen));
        }
    }
    spans
}

/// Take a frame buffer nobody uses any more back into the compositor's pool.
fn reclaim(old: Arc<Frame>, fpool: &mut Vec<Frame>) {
    if fpool.len() < FPOOL_KEEP {
        if let Ok(f) = Arc::try_unwrap(old) {
            fpool.push(f);
        }
    }
}

/// Composited frame for grid index `idx` — from the cache when possible, else rendered and cached.
/// `false` = the render panicked (nothing was cached; the returned frame is stale/blank).
#[allow(clippy::too_many_arguments)]
fn cpu_cached(
    cache: &mut Cache<Frame>,
    fpool: &mut Vec<Frame>,
    project: &Project,
    idx: i64,
    fps: f64,
    w: u32,
    h: u32,
    pool: &mut DecoderPool,
    text: &Mutex<TextRasterizer>,
    comp: &mut Compositor,
    hits: &AtomicU64,
) -> (Arc<Frame>, bool) {
    if let Some(f) = cache.get(idx) {
        hits.fetch_add(1, Ordering::Relaxed);
        return (f, true);
    }
    let t = idx as f64 / fps;
    let mut frame = fpool.pop().unwrap_or_default();
    let ok = guarded(pool, |pool| comp.render(project, t, w, h, pool, &mut lock(text), &mut frame));
    let frame = Arc::new(frame);
    if ok {
        for old in cache.insert(idx, frame.rgba.len(), frame.clone()) {
            reclaim(old, fpool);
        }
    }
    (frame, ok)
}

/// Decoded layer set for grid index `idx` — from the cache when possible, else decoded and cached.
#[allow(clippy::too_many_arguments)]
fn gpu_cached(
    cache: &mut Cache<LayerSet>,
    spare: &mut Vec<Frame>,
    project: &Project,
    idx: i64,
    fps: f64,
    w: u32,
    h: u32,
    pool: &mut DecoderPool,
    text: &Mutex<TextRasterizer>,
    shapes: &mut ShapeRasterizer,
    comp: &mut Compositor,
    hits: &AtomicU64,
) -> (Arc<LayerSet>, bool) {
    if let Some(s) = cache.get(idx) {
        hits.fetch_add(1, Ordering::Relaxed);
        return (s, true);
    }
    let t = idx as f64 / fps;
    let mut set = LayerSet::default();
    let ok = {
        let text = &mut lock(text);
        guarded(pool, |pool| set = decode_layers(project, t, w, h, pool, spare, text, shapes, comp))
    };
    let set = Arc::new(set);
    if ok {
        let bytes: usize = set.layers.iter().map(|(_, f)| f.rgba.len()).sum::<usize>()
            + set.motion.iter().map(|(_, _, f)| f.rgba.len()).sum::<usize>();
        for old in cache.insert(idx, bytes, set.clone()) {
            recycle(Some(old), spare);
        }
    }
    (set, ok)
}

/// Everything `engine::gpu` needs for one timeline instant (its `LayerSet` contract): one bitmap per
/// visual clip on screen at `t` — **both** clips of a transition, which the renderer draws virtually
/// extended past their own bounds — plus the subtitle bitmap under `LayerSet::SUBTITLES`. Media is
/// decoded at the size the compositor would have used (placement, capped at the source's native size);
/// text/shape clips are rasterised and nested sequences composited on the CPU, all at (w, h) — which is
/// already the quality-scaled render size, so `gpu::render_size` must not be applied again. Adjustment
/// layers need no bitmap (they re-process the canvas). Effects that `needs_motion()` also get neighbours.
#[allow(clippy::too_many_arguments)]
pub(crate) fn decode_layers(
    project: &Project,
    t: f64,
    w: u32,
    h: u32,
    pool: &mut DecoderPool,
    spare: &mut Vec<Frame>,
    text: &mut TextRasterizer,
    shapes: &mut ShapeRasterizer,
    comp: &mut Compositor,
) -> LayerSet {
    let mut set = LayerSet::default();
    for (ti, track) in project.tracks.iter().enumerate() {
        if track.kind != TrackKind::Video || !project.active(ti) {
            continue;
        }
        // mirrors gpu::render_canvas / compose::render_tracks: inside a transition window both clips are
        // drawn past their own bounds, so filtering by `contains(t)` here would hard-cut every transition
        if let Some((_, left, right)) = track.transition_at(t) {
            for clip in [left, right].into_iter().flatten() {
                layer_for(project, clip, t, w, h, pool, spare, text, shapes, comp, &mut set);
            }
            continue;
        }
        for clip in &track.clips {
            if clip.enabled && clip.contains(t) {
                layer_for(project, clip, t, w, h, pool, spare, text, shapes, comp, &mut set);
            }
        }
    }
    if let Some((cue_text, base_style)) = crate::engine::subtitles::cue_layer_at(project, t) {
        // ws:transcript-captions: `cue_layer_at` returns Cows (owned only on the karaoke path)
        let mut style = base_style.into_owned();
        style.text = cue_text.into_owned();
        // ---- ws:text-titles ----: `t` here is decode_layers' own timeline time (subtitles have no
        // clip-local time of their own — see compose.rs's identical call for the same reasoning).
        let img = text.render(&style, w as f32 / project.width.max(1) as f32, t);
        if img.width > 1 || img.height > 1 {
            set.layers.push((LayerSet::SUBTITLES, img));
        }
    }
    set
}

/// One clip's layer: video/images decode, text/shapes rasterise, a nested sequence is composited on the
/// CPU. Kinds the renderer draws without a bitmap (Adjustment) or not at all (Audio) push nothing.
#[allow(clippy::too_many_arguments)]
fn layer_for(
    project: &Project,
    clip: &Clip,
    t: f64,
    w: u32,
    h: u32,
    pool: &mut DecoderPool,
    spare: &mut Vec<Frame>,
    text: &mut TextRasterizer,
    shapes: &mut ShapeRasterizer,
    comp: &mut Compositor,
    set: &mut LayerSet,
) {
    // Asset nodes sample footage that need not be on the timeline, so nothing else decodes it: one
    // frame per asset, keyed by the asset id (`gpu::eval_graph_on` looks it up there). Before the
    // `draws()` gate, since an adjustment layer can hold them too.
    // ponytail: sampled at the clip's own local time, at canvas size — no per-node time offset yet.
    for aid in clip.graph.iter().flat_map(|g| g.nodes.iter()).filter_map(|n| match n.kind {
        crate::model::NodeKind::Asset(a) => Some(a),
        _ => None,
    }) {
        let Some(asset) = project.asset(aid).filter(|a| a.width > 0 && a.height > 0) else { continue };
        if set.get(aid).is_some() {
            continue;
        }
        let (dw, dh) = (w.min(asset.width), h.min(asset.height));
        if let Some(f) = decode_one(pool, &asset.path, clip.local(t), asset.duration, dw, dh, spare) {
            set.layers.push((aid, f));
        }
    }
    if !clip.draws() {
        return;
    }
    let s = w as f32 / project.width.max(1) as f32;
    match clip.kind {
        ClipKind::Video | ClipKind::Image => {
            if clip.is_empty_container() {
                let label_text = if !clip.container_label.is_empty() {
                    format!("Container Slot\n[{}]", clip.container_label)
                } else {
                    "Container Slot\n(Empty)".to_string()
                };
                let style = crate::model::TextStyle {
                    text: label_text,
                    size: crate::model::Animated::new(32.0),
                    color: [220, 220, 220, 255],
                    box_color: [40, 40, 40, 220],
                    box_padding: 16.0,
                    align: 1,
                    ..Default::default()
                };
                let img = text.render(&style, s, clip.local(t));
                if img.width > 1 || img.height > 1 {
                    set.layers.push((clip.id, img));
                }
                return;
            }
            let Some(asset) = project.asset(clip.asset) else { return };
            let native = (asset.width, asset.height);
            if native.0 == 0 || native.1 == 0 {
                return;
            }
            let p = placement(project, clip, t, native, w, h, true);
            if !(p.w.is_finite() && p.h.is_finite()) {
                return;
            }
            let dw = (p.w.round().max(1.0) as u32).clamp(1, native.0);
            let dh = (p.h.round().max(1.0) as u32).clamp(1, native.1);
            if let Some(f) = decode_one(pool, &asset.path, clip.src_time(t), asset.duration, dw, dh, spare) {
                set.layers.push((clip.id, f));
            }
            // ponytail: two extra samples half a frame apart feed shutter-based effects; a shutter-angle
            // driven sample count is the upgrade if motion blur ever needs to be exact.
            if clip.effects.iter().any(|e| e.enabled && e.kind.needs_motion()) {
                let fd = project.frame_dur();
                for k in [-1.0, -0.5] {
                    let st = clip.src_time(t + k * fd);
                    if let Some(f) = decode_one(pool, &asset.path, st, asset.duration, dw, dh, spare) {
                        set.motion.push((clip.id, k * fd, f));
                    }
                }
            }
        }
        ClipKind::Text => {
            let Some(style) = &clip.text else { return };
            let img = text.render(style, s, clip.local(t));
            if img.width > 1 || img.height > 1 {
                set.layers.push((clip.id, img));
            }
        }
        ClipKind::Shape => {
            let Some(style) = &clip.shape else { return };
            let img = shapes.render(style, s, clip.local(t));
            if !img.is_empty() {
                set.layers.push((clip.id, img));
            }
        }
        ClipKind::Sequence => {
            let editing = project.editing == Some(clip.sequence);
            let (qw, qh) = if editing {
                (project.width, project.height) // its live size is swapped into the project
            } else {
                match project.sequence(clip.sequence) {
                    Some(sq) => (sq.width, sq.height),
                    None => return,
                }
            };
            if qw == 0 || qh == 0 {
                return;
            }
            let p = placement(project, clip, t, (qw, qh), w, h, true);
            if !(p.w.is_finite() && p.h.is_finite() && p.w > 0.0 && p.h > 0.0) {
                return;
            }
            let dw = (p.w.round() as u32).clamp(1, qw);
            let dh = (p.h.round() as u32).clamp(1, qh);
            let dur = project.sequence_duration(clip.sequence);
            let st = clip.src_time(t).clamp(0.0, (dur - 1e-6).max(0.0));
            // ponytail: gpu.rs wants a nested sequence as a ready bitmap, so the CPU compositor renders
            // it — through a shallow project view (one Project clone per frame while a sequence clip is
            // on screen). Making `Compositor::render_tracks` pub would drop both the clone and this.
            let mut sub = project.clone();
            if !editing {
                let Some(sq) = project.sequence(clip.sequence) else { return };
                sub.tracks.clone_from(&sq.tracks);
                (sub.width, sub.height) = (qw, qh);
                sub.editing = Some(clip.sequence);
            }
            sub.show_subtitles = false; // subtitles belong to the outer canvas
            let mut frame = spare.pop().unwrap_or_default();
            comp.render(&sub, st, dw, dh, pool, text, &mut frame);
            set.layers.push((clip.id, Arc::new(frame)));
        }
        ClipKind::Audio | ClipKind::Adjustment => {}
    }
}

/// One decode into a recycled buffer. None when the decoder or the frame is unavailable.
fn decode_one(
    pool: &mut DecoderPool,
    path: &str,
    src_t: f64,
    src_duration: f64,
    dw: u32,
    dh: u32,
    spare: &mut Vec<Frame>,
) -> Option<Arc<Frame>> {
    let st = if src_duration > 0.0 { src_t.clamp(0.0, (src_duration - 1e-4).max(0.0)) } else { src_t.max(0.0) };
    let mut frame = spare.pop().unwrap_or_default();
    // through the pool's source-frame cache: a replay/scrub-back is a memcpy, not a decoder seek
    if !pool.frame_at(path, st, dw, dh, &mut frame) {
        spare.push(frame);
        return None;
    }
    Some(Arc::new(frame))
}

/// Take the buffers of a layer set nobody uses any more back into the spare pool.
fn recycle(old: Option<Arc<LayerSet>>, spare: &mut Vec<Frame>) {
    const KEEP: usize = 8;
    let Some(Ok(set)) = old.map(Arc::try_unwrap) else { return };
    let frames = set.layers.into_iter().map(|(_, f)| f).chain(set.motion.into_iter().map(|(_, _, f)| f));
    for f in frames {
        if spare.len() >= KEEP {
            return;
        }
        if let Ok(f) = Arc::try_unwrap(f) {
            spare.push(f);
        }
    }
}

fn audio_thread(shared: Arc<Shared>, rx: Receiver<Cmd>, backend: Backend) {
    use cpal::traits::StreamTrait;
    let mut pool = DecoderPool::new(backend);
    let mut mixer = Mixer::new();
    let ring: Arc<Mutex<VecDeque<f32>>> = Arc::new(Mutex::new(VecDeque::new()));
    let dead = Arc::new(AtomicBool::new(false)); // set by the stream's error callback (device gone/changed)
    let mut stream: Option<cpal::Stream> = None; // opened on the first Play, re-opened once `dead`
    let mut running = false; // device stream started (kept stopped while idle: no wakeups, no audiodg load)
    let mut project = lock(&shared.project).clone();
    let mut block = vec![0f32; BLOCK * 2];
    let mut mixed_until = 0.0;
    // true from a Play/Seek reset until the ring has been pre-filled to LEAD_SECS once: suppresses the
    // underrun catch-up below so a slow first block (cold device/decoder open) cannot skip mixed_until
    // past a fade-in sitting at the new start time — it only ever plays audio mixed from that exact time.
    let mut filling = false;
    let mut pending: Option<Cmd> = None;
    // ws:player-rate-loop: scratch mix window for 0<rate<=2 (sized for the widest case, rate=2 -> 2*BLOCK
    // frames), decimated/duplicated down to exactly BLOCK output frames; last_wraps detects a loop seam.
    let mut scratch = vec![0f32; BLOCK * 2 * 2];
    let mut last_wraps: u64 = 0;
    let is_playing = || {
        let mut c = lock(&shared.clock);
        c.now();
        c.playing
    };
    loop {
        if pending.is_none() && !running && !(stream.is_some() && is_playing()) {
            let Ok(c) = rx.recv() else { return }; // idle (or no device): block, zero CPU
            pending = Some(c);
        }
        loop {
            let c = match pending.take() {
                Some(c) => c,
                None => match rx.try_recv() {
                    Ok(c) => c,
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return,
                },
            };
            match c {
                Cmd::SetProject => project = lock(&shared.project).clone(),
                Cmd::Play | Cmd::Seek => {
                    lock(&ring).clear();
                    mixed_until = lock(&shared.clock).now();
                    filling = true;
                }
                Cmd::Pause => lock(&ring).clear(),
                Cmd::Canvas => {}
                Cmd::Backend(b) => pool.set_backend(b),
                Cmd::Gpu(_) | Cmd::Proxies(_) | Cmd::CacheBudget(_) => {} // render-thread only
                Cmd::ClearDecoders(ack) => {
                    pool.clear();
                    let _ = ack.send(());
                }
                Cmd::RenderOnce(..) | Cmd::LayersOnce(..) => {} // render-thread only
                // ---- ws:player-rate-loop ----
                Cmd::Rate | Cmd::Loop | Cmd::LayersAsync => {} // render-thread only
                Cmd::Scrub(t) => {
                    // mixes exactly one BLOCK at `t` into the ring without touching mixed_until — a
                    // paused playhead scrub. Generalizes the lazy open/play path above so a scrub can
                    // start the device even while !playing; the existing `!playing && running &&
                    // ring.is_empty()` pause condition (below, after this drain loop) tears the stream
                    // back down once this one block has drained, regardless of why it started.
                    if stream.is_none() || dead.swap(false, Ordering::Relaxed) {
                        stream = open_output(ring.clone(), dead.clone());
                    }
                    if let Some(s) = &stream {
                        if !running {
                            running = s.play().is_ok();
                        }
                        if running && guarded(&mut pool, |pool| mixer.mix(&project, t, pool, &mut block)) {
                            lock(&ring).extend(block.iter().copied());
                            mixer.graph().publish(&METER_FEED); // ws:audio-dsp-automation
                        }
                    }
                }
                Cmd::Quit => return,
            }
        }
        let playing = is_playing();
        if playing && (stream.is_none() || dead.swap(false, Ordering::Relaxed)) {
            // lazy/late open: no device at startup, or it went away / the default changed mid-session.
            // If it fails we fall back to the idle recv above (retried on the next command).
            stream = open_output(ring.clone(), dead.clone());
            running = false;
        }
        let Some(stream) = &stream else { continue };
        if playing && !running {
            running = stream.play().is_ok();
        } else if !playing && running && lock(&ring).is_empty() {
            // let the tail play out after the clock auto-stops at the end, then stop the device
            let _ = stream.pause();
            running = false;
        }
        let mut queued = lock(&ring).len() as f64 / (2 * SAMPLE_RATE) as f64;
        if playing && queued < LEAD_SECS {
            // ws:player-rate-loop: a loop wrap is a clock discontinuity the ring hasn't seen — flush it
            // and refill from scratch, same as a fresh Play/Seek (whose own Cmd arm above already
            // flushes; this only fires for an in-progress loop wrap between commands).
            let cur_wraps = lock(&shared.clock).wraps;
            if cur_wraps != last_wraps {
                last_wraps = cur_wraps;
                lock(&ring).clear();
                mixed_until = lock(&shared.clock).now(); // mix from the post-wrap time, not the stale one
                filling = true;
                queued = 0.0;
            }
            // ponytail: wall clock is the master; the ring is refilled only as the device drains it, so it
            // can't grow past LEAD + one block. The ring head plays at timeline time `mixed_until - queued`;
            // if that is already >50 ms in the past (decoder respawn, mid-playback underrun) snap the next
            // block to land on time instead of carrying the lag forever. Not while still `filling`, though:
            // a cold device/decoder open makes this same block "late" on every fresh Play/Seek, and snapping
            // then would skip mixed_until straight past a fade-in sitting at the new start time. Resample if
            // drift ever matters.
            let now = lock(&shared.clock).now();
            if !filling && now - (mixed_until - queued) > 0.05 {
                mixed_until = now + queued;
            }
            let rate = lock(&shared.clock).rate;
            if (rate - 1.0).abs() < 1e-9 {
                // unchanged fast path: byte-identical to the pre-rate-loop code at the default rate
                if guarded(&mut pool, |pool| mixer.mix(&project, mixed_until, pool, &mut block)) {
                    lock(&ring).extend(block.iter().copied()); // a panicked block: underrun instead
                                                               // ---- ws:audio-dsp-automation ----
                                                               // hand the post-fader bus blocks to the UI thread's meters/LUFS (App::sync_buses)
                    mixer.graph().publish(&METER_FEED);
                }
                mixed_until += BLOCK as f64 / SAMPLE_RATE as f64;
            } else if rate > 0.0 && rate <= 2.0 {
                // fixed-ratio resample: mix `win` real frames of source (spanning win/SAMPLE_RATE of
                // TIMELINE time), then decimate/duplicate down to exactly BLOCK OUTPUT frames — that
                // many frames of source content plays back in one BLOCK of real output time.
                let win = ((BLOCK as f64 * rate).round().max(1.0) as usize).min(scratch.len() / 2);
                let buf = &mut scratch[..win * 2];
                if guarded(&mut pool, |pool| mixer.mix(&project, mixed_until, pool, buf)) {
                    for i in 0..BLOCK {
                        let si = (i * win / BLOCK).min(win - 1);
                        block[i * 2] = buf[si * 2];
                        block[i * 2 + 1] = buf[si * 2 + 1];
                    }
                    lock(&ring).extend(block.iter().copied());
                    mixer.graph().publish(&METER_FEED); // ws:audio-dsp-automation
                }
                mixed_until += win as f64 / SAMPLE_RATE as f64;
            } else {
                // reverse or >2x: not resampled/pitch-shifted — muted outright (still extends the ring
                // so pacing/underrun logic sees normal progress; no drift tracking needed while muted).
                block.fill(0.0);
                lock(&ring).extend(block.iter().copied());
                mixed_until = lock(&shared.clock).now();
            }
        } else {
            filling = false; // ring reached LEAD_SECS (or we're paused): back to normal underrun detection
            match rx.recv_timeout(Duration::from_millis(4)) {
                Ok(c) => pending = Some(c),
                Err(RecvTimeoutError::Disconnected) => return,
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
    }
}

/// Default output device as 48 kHz stereo f32; None = no audio (playback still runs off the wall clock).
fn open_output(ring: Arc<Mutex<VecDeque<f32>>>, dead: Arc<AtomicBool>) -> Option<cpal::Stream> {
    use cpal::traits::{DeviceTrait, HostTrait};
    let device = cpal::default_host().default_output_device()?;
    // ponytail: WASAPI shared mode converts any PCM rate/channel count for output (cpal sets
    // AUTOCONVERTPCM), so we ask for exactly what the mixer produces. Add a resampler if a device refuses.
    let config = cpal::StreamConfig { channels: 2, sample_rate: SAMPLE_RATE, buffer_size: cpal::BufferSize::Default };
    let stream = device
        .build_output_stream(
            config,
            move |out: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let mut r = lock(&ring);
                for s in out.iter_mut() {
                    *s = r.pop_front().unwrap_or(0.0); // zero-fill on underrun
                }
            },
            move |e| {
                eprintln!("audio: {e}");
                // cpal keeps the stream on the old endpoint after a default-device change and only reports
                // it; any non-xrun error means "rebuild the stream" (done lazily by audio_thread).
                if e.kind() != cpal::ErrorKind::Xrun {
                    dead.store(true, Ordering::Relaxed);
                }
            },
            None,
        )
        .map_err(|e| eprintln!("audio: no output stream ({e}); playing silently"))
        .ok()?;
    Some(stream)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media;
    use std::thread::sleep;

    /// Newest frame rendered for exactly `t` at `size` (earlier renders may still be in flight).
    fn wait_frame(p: &mut Player, t: f64, size: (u32, u32)) -> Arc<Frame> {
        for _ in 0..300 {
            if let Some(f) = p.take_frame() {
                if (f.pts - t).abs() < 1e-6 && (f.width, f.height) == size {
                    return f;
                }
            }
            sleep(Duration::from_millis(10));
        }
        panic!("no {size:?} frame for t={t} within 3 s");
    }
    fn centre(f: &Frame) -> [u8; 4] {
        let i = ((f.height / 2 * f.width + f.width / 2) * 4) as usize;
        [f.rgba[i], f.rgba[i + 1], f.rgba[i + 2], f.rgba[i + 3]]
    }

    #[test]
    fn player_seek_play_pause_release() {
        use cpal::traits::HostTrait;
        if cpal::default_host().default_output_device().is_none() {
            println!("playback test: no audio device, running silently");
        }
        let path = media::ffpipe::tests::test_mp4(); // red 0–2 s, lime 2–4 s, 320x240, sine audio
        let asset = media::probe(&path, Backend::Auto).unwrap();
        let project = Project::from_media(asset);
        let mut p =
            Player::new(eframe::egui::Context::default(), Backend::Auto, Arc::new(Mutex::new(TextRasterizer::new())));
        p.set_project(&project);
        p.set_canvas(320, 240, 1280);
        let dur = project.duration();
        assert!((dur - 4.0).abs() < 0.2, "duration {dur}");

        p.seek(0.5);
        let c = centre(&wait_frame(&mut p, 0.5, (320, 240)));
        assert!(c[0] > 200 && c[1] < 70 && c[2] < 70, "expected red at 0.5 s, got {c:?}");

        p.seek(2.5);
        let c = centre(&wait_frame(&mut p, 2.5, (320, 240)));
        assert!(c[0] < 70 && c[1] > 200 && c[2] < 70, "expected green at 2.5 s, got {c:?}");

        p.play();
        assert!(p.is_playing());
        // frames arrive while playing, paced at the project fps (the old recv_timeout pacing rounded up to
        // the 15.6 ms scheduler tick: every gap ~47 ms at 30 fps)
        let (start, mut pts) = (Instant::now(), Vec::new());
        while start.elapsed() < Duration::from_millis(600) {
            if let Some(f) = p.take_frame() {
                let now = p.time();
                // pts are on the fps grid now: the first frame after play() from 2.5 is exactly 2.5
                assert!(f.pts >= 2.5 && f.pts <= now, "pts {} vs time {now}", f.pts);
                pts.push(f.pts);
            }
            sleep(Duration::from_millis(1));
        }
        let t = p.time();
        assert!((3.05..3.45).contains(&t), "time after 600 ms of play: {t}");
        let mut gaps: Vec<f64> = pts.windows(2).map(|w| w[1] - w[0]).collect();
        gaps.sort_by(f64::total_cmp);
        assert!(gaps.len() >= 8, "only {} frames in 600 ms", pts.len());
        let q1 = gaps[gaps.len() / 4]; // lower quartile: robust to a loaded test machine inflating some gaps
        assert!(q1 < 1.25 / project.fps, "frame gap {q1:.4} s at {} fps", project.fps);

        p.pause();
        let t1 = p.time();
        sleep(Duration::from_millis(100));
        assert_eq!(p.time(), t1);
        assert!(!p.is_playing());

        // play from the end rewinds
        p.seek(dur);
        p.play();
        let t = p.time();
        assert!(t < 0.2, "rewound: {t}");
        p.pause();

        let start = Instant::now();
        p.release_files();
        assert!(start.elapsed() < Duration::from_secs(2), "release_files took {:?}", start.elapsed());
        p.seek(1.0); // published pts sit on the fps grid, so ask for a grid time
        p.set_canvas(160, 120, 100); // clamp keeps aspect, and the preview re-renders after release
        wait_frame(&mut p, 1.0, (100, 75));

        // render_once: synchronous frame for tools (MCP render.frame), independent of the preview canvas
        let f = p.render_once(2.5, 160).expect("render_once");
        assert_eq!((f.width, f.height), (160, 120));
        let c = centre(&f);
        assert!(c[0] < 70 && c[1] > 200, "expected green from render_once at 2.5 s, got {c:?}");
    }

    /// Replays and scrub-backs are served from the RAM cache: no re-decode, identical frames.
    #[test]
    fn replay_serves_cached_frames() {
        let path = media::ffpipe::tests::test_mp4(); // red 0–2 s, 30 fps
        let asset = media::probe(&path, Backend::Auto).unwrap();
        let project = Project::from_media(asset);
        let mut p =
            Player::new(eframe::egui::Context::default(), Backend::Auto, Arc::new(Mutex::new(TextRasterizer::new())));
        p.set_project(&project);
        p.set_canvas(320, 240, 1280);
        p.seek(0.5);
        wait_frame(&mut p, 0.5, (320, 240));
        let h0 = p.cache_hits();
        p.seek(0.5); // same frame index -> must come from the cache
        wait_frame(&mut p, 0.5, (320, 240));
        assert!(p.cache_hits() > h0, "re-seek to the same frame must hit the cache");

        // play a stretch, then jump back into it: the replayed frame must be a cache hit too
        p.play();
        sleep(Duration::from_millis(400));
        p.pause();
        let h1 = p.cache_hits();
        p.seek(0.6); // 3 frames past 0.5: rendered (or prefetched) during the stretch above
        let f = wait_frame(&mut p, 0.6, (320, 240));
        assert!(p.cache_hits() > h1, "replay after playback must be served from cache");
        let c = centre(&f);
        assert!(c[0] > 200 && c[1] < 70, "cached frame content must be right: {c:?}");

        // a no-op edit (identical project) keeps the cache: selective invalidation finds no dirty span
        let h2 = p.cache_hits();
        p.set_project(&project);
        p.seek(0.6);
        wait_frame(&mut p, 0.6, (320, 240));
        assert!(p.cache_hits() > h2, "an identical project must keep the cache");

        // an edit that touches the clip evicts its span: the same seek renders fresh
        let mut edited = project.clone();
        let (_, clip) = edited.all_clips().next().expect("one clip");
        let id = clip.id;
        edited.clip_mut(id).unwrap().speed = 2.0;
        let h3 = p.cache_hits();
        p.set_project(&edited);
        p.seek(0.6);
        wait_frame(&mut p, 0.6, (320, 240));
        assert_eq!(p.cache_hits(), h3, "an edit inside the clip's span must evict its frames");
    }

    /// `spans_using_source`: direct users (case-insensitive), transition widening, nested
    /// sequences, and graph-sampled assets all resolve to spans; unrelated paths to none.
    #[test]
    fn spans_using_source_covers_every_user_kind() {
        use crate::model::{Node, NodeGraph, NodeKind, ShapeKind, Transition, TransitionKind};
        let path = media::ffpipe::tests::test_mp4();
        let asset = media::probe(&path, Backend::Auto).unwrap();
        let project = Project::from_media(asset.clone());

        assert!(spans_using_source(&project, "Z:\\other.mp4").is_empty(), "unrelated path: no spans");
        let spans = spans_using_source(&project, &path.to_ascii_uppercase());
        assert!(!spans.is_empty(), "case-insensitive path match");
        let dur = project.duration();
        let end = spans.iter().fold(f64::MIN, |m, &(_, b)| m.max(b));
        assert!(end >= dur - 1e-6, "the using clip's span is covered: {end} vs {dur}");

        // a transition on the track widens every span by its duration (it draws neighbours past
        // their own bounds)
        let mut tp = project.clone();
        let vt = tp.tracks.iter().position(|t| !t.clips.is_empty()).unwrap();
        let cid = tp.tracks[vt].clips[0].id;
        tp.tracks[vt].transitions.push(Transition {
            id: 9999,
            right: cid,
            kind: TransitionKind::ALL[0],
            duration: 1.5,
            color: [0, 0, 0, 255],
            direction: 0,
            ease: Default::default(),
            edge: Default::default(),
        });
        let wend = spans_using_source(&tp, &path).iter().fold(f64::MIN, |m, &(_, b)| m.max(b));
        assert!((wend - end - 1.5).abs() < 1e-6, "widened by the longest transition: {wend} vs {end}");

        // nesting the clips into a sequence: the outer Sequence clip is found recursively
        let mut np = project.clone();
        let ids: Vec<crate::model::Id> = np.all_clips().map(|(_, c)| c.id).collect();
        np.nest_selection(&ids, "seq").expect("nested");
        assert!(!spans_using_source(&np, &path).is_empty(), "found through the nested sequence clip");

        // a clip that only SAMPLES the asset through its node graph counts too
        let mut gp = Project::new();
        let aid = gp.add_asset(asset);
        let sid = gp.add_shape_clip(ShapeKind::Rect, 0.0, 2.0);
        let mut n = 100;
        let mut next = move || {
            n += 1;
            n
        };
        let mut g = NodeGraph::new(&mut next);
        g.nodes.push(Node { id: 999, kind: NodeKind::Asset(aid), x: 0.0, y: 0.0, enabled: true });
        gp.clip_mut(sid).unwrap().graph = Some(g);
        let gspans = spans_using_source(&gp, &path);
        assert!(gspans.iter().any(|&(a, b)| a <= 0.0 && b >= 2.0), "graph-sampled asset: {gspans:?}");
    }

    /// The cache budget is enforced on insert AND on set_budget (lowering the setting returns RAM
    /// immediately), and `avg_entry_bytes` refuses to divide by nothing.
    #[test]
    fn cache_budget_evicts_down_and_avg_guards() {
        let mut c: Cache<u8> = Cache::new(100);
        assert!(c.avg_entry_bytes().is_none(), "empty cache: no average");
        c.insert(0, 0, Arc::new(0u8));
        assert!(c.avg_entry_bytes().is_none(), "zero-byte entries must not produce a 0 average");
        c.insert(1, 60, Arc::new(1u8));
        c.insert(2, 60, Arc::new(2u8));
        assert!(c.bytes <= 100, "insert evicts to budget, had {}", c.bytes);
        assert_eq!(c.avg_entry_bytes(), Some(c.bytes / c.map.len()));
        let out = c.set_budget(50);
        assert!(!out.is_empty() && c.bytes <= 50, "set_budget evicts immediately ({} bytes left)", c.bytes);
    }

    #[test]
    fn cache_budget_bytes_auto_and_explicit() {
        assert_eq!(cache_budget_bytes(1024), 1024usize << 20, "explicit MB passes through");
        let auto = cache_budget_bytes(0);
        assert!((512usize << 20..=4usize << 30).contains(&auto), "auto in [512 MB, 4 GB]: {auto}");
    }

    /// A finished proxy evicts only the spans using that source — an unrelated mapping keeps the
    /// whole cache (this used to clear everything, restarting the read-ahead on every proxy).
    #[test]
    fn proxy_swap_evicts_only_that_source() {
        let path = media::ffpipe::tests::test_mp4();
        let asset = media::probe(&path, Backend::Auto).unwrap();
        let project = Project::from_media(asset);
        let mut p =
            Player::new(eframe::egui::Context::default(), Backend::Auto, Arc::new(Mutex::new(TextRasterizer::new())));
        p.set_project(&project);
        p.set_canvas(320, 240, 1280);
        p.seek(0.5);
        wait_frame(&mut p, 0.5, (320, 240));

        let h0 = p.cache_hits();
        let mut unrelated = HashMap::new();
        unrelated.insert("Z:\\other.mp4".to_string(), "Z:\\other-proxy.mp4".to_string());
        p.set_proxies(unrelated);
        p.seek(0.5);
        wait_frame(&mut p, 0.5, (320, 240));
        assert!(p.cache_hits() > h0, "a proxy for another source must keep this source's cache");

        let mut real = HashMap::new();
        real.insert(path.clone(), "Z:\\nope\\proxy.mp4".to_string());
        let h1 = p.cache_hits();
        p.set_proxies(real);
        p.seek(0.5);
        wait_frame(&mut p, 0.5, (320, 240));
        assert_eq!(p.cache_hits(), h1, "mapping THIS source must evict its span (fresh render)");
    }

    /// Frame-level invalidation: only the spans an edit can change are evicted.
    #[test]
    fn dirty_spans_bound_the_edit() {
        let path = media::ffpipe::tests::test_mp4();
        let asset = media::probe(&path, Backend::Auto).unwrap();
        let project = Project::from_media(asset);
        // identical projects: nothing dirty
        assert_eq!(video_dirty_spans(&project, &project.clone()), Some(vec![]));
        // marker/in-out edits: still nothing dirty
        let mut m = project.clone();
        m.in_point = Some(1.0);
        assert_eq!(video_dirty_spans(&project, &m), Some(vec![]));
        // clip edit: exactly that clip's span (old + new)
        let mut e = project.clone();
        let id = e.all_clips().next().unwrap().1.id;
        e.clip_mut(id).unwrap().speed = 2.0;
        let spans = video_dirty_spans(&project, &e).expect("bounded");
        assert!(!spans.is_empty());
        // canvas-size change: unbounded
        let mut g = project.clone();
        g.width += 2;
        assert_eq!(video_dirty_spans(&project, &g), None);
    }

    /// ws:registries-schema-hooks — the new Track flags/fields never widen to a full clear: nothing
    /// reads them yet (mixer volume-sampling and the trim primitives land in later waves), so a diff
    /// on them alone must still return a bounded (possibly empty) span list, never `None`.
    #[test]
    fn flags_do_not_dirty_video() {
        let path = media::ffpipe::tests::test_mp4();
        let asset = media::probe(&path, Backend::Auto).unwrap();
        let project = Project::from_media(asset);
        let mut f = project.clone();
        let vt = f.tracks.iter_mut().find(|t| t.kind == TrackKind::Video).unwrap();
        vt.locked = true;
        vt.ripple = Some(false);
        vt.magnetic = true;
        vt.color = Some([200, 60, 60]);
        vt.volume = crate::model::Animated::new(0.5);
        assert!(video_dirty_spans(&project, &f).is_some(), "track flags/volume must not force a full clear");
    }

    /// ws:player-rate-loop — a track REORDER (same ids, swapped index) can flip which video draws on
    /// top; a bounded diff computed against the old index assignment would be wrong for the new one, so
    /// it must force a full clear (needed before pro-timeline, wave 3, allows track reordering).
    #[test]
    fn track_id_reorder_full_clears() {
        let path = media::ffpipe::tests::test_mp4();
        let asset = media::probe(&path, Backend::Auto).unwrap();
        let mut project = Project::from_media(asset);
        project.add_track(TrackKind::Video);
        let mut reordered = project.clone();
        let (id0, id1) = (reordered.tracks[0].id, reordered.tracks[1].id);
        reordered.tracks[0].id = id1;
        reordered.tracks[1].id = id0;
        assert_eq!(
            video_dirty_spans(&project, &reordered),
            None,
            "a Track.id reorder (same index, different id) must force a full clear"
        );
    }

    /// ws:pro-monitor review gap — `multicam_switch` (model/ops/multicam.rs) mutates the nested
    /// sequence's tracks via `Project.sequences`, never via `Project.tracks` directly. `video_dirty_spans`
    /// already treats any `old.sequences != new.sequences` diff as unbounded (`None`, full clear) —
    /// checked up front, before the per-track bounded-span logic even runs — so an angle switch already
    /// forces a full cache clear today. Pinned here (extending this file's existing `video_dirty_spans`
    /// tests) so that behavior doesn't silently regress if the sequences check above is ever narrowed.
    #[test]
    fn multicam_switch_forces_full_cache_clear() {
        use crate::model::Asset;

        fn asset(id: crate::model::Id, dur: f64) -> Asset {
            Asset {
                id,
                path: format!("C:/cam{id}.mp4"),
                kind: ClipKind::Video,
                duration: dur,
                width: 1920,
                height: 1080,
                fps: 30.0,
                audio_streams: Vec::new(),
                codec: String::new(),
                folder: String::new(),
                tags: Vec::new(),
                label: 0,
                description: String::new(),
                rel_path: None,
                parent: None,
                range: None,
                effects: Vec::new(),
            }
        }

        let mut p = Project::new();
        let ids: Vec<crate::model::Id> = (0..2)
            .map(|i| {
                let aid = p.add_asset(asset(200 + i, 20.0));
                let cid = p.new_id();
                let mut c = Clip::new(cid, ClipKind::Video, format!("cam{i}"), 0.0, 10.0);
                c.asset = aid;
                let ti = if i == 0 { 0 } else { p.add_track(TrackKind::Video) };
                p.tracks[ti].clips.push(c);
                cid
            })
            .collect();
        let seq_id = p.multicam_make(&ids, &[0.0, 0.0], "Multicam").unwrap();
        let clip_id = p.insert_sequence_clip(seq_id, 0.0, None).unwrap();

        let before = p.clone();
        assert!(p.multicam_switch(clip_id, 5.0, 1), "the switch itself must succeed");
        assert_ne!(
            serde_json::to_string(&before.sequences).unwrap(),
            serde_json::to_string(&p.sequences).unwrap(),
            "sanity: the switch actually changed the nested sequence"
        );
        assert_eq!(
            video_dirty_spans(&before, &p),
            None,
            "an angle switch changes Project.sequences, which video_dirty_spans already treats as unbounded"
        );
    }

    #[test]
    fn clock_rate_and_loop_wrap() {
        // rate=2.0, no loop: advances at ~2x wall-clock
        let mut c = Clock {
            playing: true,
            base_t: 0.0,
            base_at: Instant::now(),
            duration: 100.0,
            canvas: (0, 0),
            rate: 2.0,
            loop_range: None,
            wraps: 0,
        };
        sleep(Duration::from_millis(100));
        let t = c.now();
        assert!((0.12..0.40).contains(&t), "rate=2 should advance ~2x wall clock, got {t}");

        // loop_range=(1.0,3.0), rate=1.0: wraps back near 1.0, never past 3.0, and counts the wrap
        let mut c = Clock {
            playing: true,
            base_t: 2.9,
            base_at: Instant::now(),
            duration: 100.0,
            canvas: (0, 0),
            rate: 1.0,
            loop_range: Some((1.0, 3.0)),
            wraps: 0,
        };
        sleep(Duration::from_millis(200)); // 2.9 + ~0.2 = ~3.1, past b=3.0
        let t = c.now();
        assert!((1.0..2.0).contains(&t), "must wrap back inside [1,3), got {t}");
        assert_eq!(c.wraps, 1);

        // rate=-1.0 with the same loop range: wraps at the low end, symmetrically
        let mut c = Clock {
            playing: true,
            base_t: 1.1,
            base_at: Instant::now(),
            duration: 100.0,
            canvas: (0, 0),
            rate: -1.0,
            loop_range: Some((1.0, 3.0)),
            wraps: 0,
        };
        sleep(Duration::from_millis(200)); // 1.1 - ~0.2 = ~0.9, past a=1.0 going backward
        let t = c.now();
        assert!((2.0..3.0).contains(&t), "reverse must wrap back inside [1,3), got {t}");
        assert_eq!(c.wraps, 1);
    }

    #[test]
    fn reverse_stops_at_zero() {
        let mut c = Clock {
            playing: true,
            base_t: 0.3,
            base_at: Instant::now(),
            duration: 100.0,
            canvas: (0, 0),
            rate: -2.0,
            loop_range: None,
            wraps: 0,
        };
        sleep(Duration::from_millis(250)); // 0.3 - 2*0.25 = -0.2 -> clamps to 0, mirrors the duration-stop
        let t = c.now();
        assert_eq!(t, 0.0);
        assert!(!c.playing, "reverse must stop playback at 0.0, mirroring the forward stop at duration");
    }

    /// A shuttle rate never survives past the next Play — Stop leaves `rate` alone (irrelevant while
    /// paused), but `play()` always forces it back to 1.0 first, so Space after J J J resumes forward.
    #[test]
    fn play_forces_rate_1() {
        let path = media::ffpipe::tests::test_mp4();
        let asset = media::probe(&path, Backend::Auto).unwrap();
        let project = Project::from_media(asset);
        let mut p =
            Player::new(eframe::egui::Context::default(), Backend::Auto, Arc::new(Mutex::new(TextRasterizer::new())));
        p.set_project(&project);
        p.set_rate(-4.0);
        p.pause();
        p.play();
        assert_eq!(p.rate(), 1.0, "play() must force rate back to 1.0 regardless of the last shuttle rate");
    }

    /// `protect_bounds`/`read_ahead_window` swap which side of `idx` is "ahead" for a reverse shuttle.
    #[test]
    fn protect_and_prefetch_flip_for_reverse() {
        for (idx, rate) in [(50i64, 1.0), (50, 4.0), (50, -1.0), (50, -4.0)] {
            let (plo, phi) = protect_bounds(idx, 10, 20, rate);
            let (rlo, rhi) = read_ahead_window(idx, 20, 1000, rate);
            if rate >= 0.0 {
                assert_eq!((plo, phi), (idx - 10, idx + 20), "forward protect: rate {rate}");
                assert_eq!((rlo, rhi), (idx, idx + 20), "forward read-ahead: rate {rate}");
            } else {
                assert_eq!((plo, phi), (idx - 20, idx + 10), "reverse protect: rate {rate}");
                assert_eq!((rlo, rhi), (idx - 20, idx), "reverse read-ahead: rate {rate}");
            }
        }
    }

    #[test]
    fn prefetch_order_strides_and_orders_nearest_first() {
        assert_eq!(prefetch_order(2, 10, 4.0), vec![2, 6, 10]);
        // reverse, |rate|<=1: dense hi,hi-1,...,lo — nearest-to-idx (hi) first, no stride skip
        assert_eq!(prefetch_order(5, 9, -1.0), vec![9, 8, 7, 6, 5]);
    }

    #[test]
    fn dropped_delta_ignores_intentional_stride() {
        assert_eq!(dropped_delta(10, 11, 1.0), 0, "exact stride at rate=1");
        // note: the plan's own prose example used 15 here, inconsistent with its own formula (15-10=5 !=
        // stride 4); 14 is the value actually consistent with `stride = rate.abs().round()` at rate=4.
        assert_eq!(dropped_delta(10, 14, 4.0), 0, "exact stride at rate=4");
        assert!(dropped_delta(10, 20, 1.0) > 0, "real stall at rate=1");
        assert_eq!(dropped_delta(-1, 5, 1.0), 0, "no prior publish");
        assert_eq!(dropped_delta(10, 9, -1.0), 0, "exact reverse stride");
        assert!(dropped_delta(10, 3, -1.0) > 0, "real reverse stall");
    }

    /// A scrub-while-playing (a `seek()` call arriving mid-playback, same as dragging the playhead
    /// without pausing) must not be miscounted as dropped frames: `Cmd::Seek` has to reset `last_pub`
    /// just like `clear_caches!`/`evict_spans!` do, or the huge intentional index jump reads as a
    /// decode stall.
    #[test]
    fn seek_while_playing_does_not_inflate_dropped_frames() {
        let path = media::ffpipe::tests::test_mp4();
        let asset = media::probe(&path, Backend::Auto).unwrap();
        let project = Project::from_media(asset);
        let mut p =
            Player::new(eframe::egui::Context::default(), Backend::Auto, Arc::new(Mutex::new(TextRasterizer::new())));
        p.set_project(&project);
        p.set_canvas(320, 240, 1280);
        p.seek(0.0);
        p.play();
        sleep(Duration::from_millis(150)); // let a few frames publish near t=0 so last_pub advances
        p.seek(3.5); // big forward jump while still playing — an intentional scrub, not a stall
        sleep(Duration::from_millis(300)); // let post-seek frames publish
        eprintln!("DEBUG dropped_frames={} time={} playing={}", p.dropped_frames(), p.time(), p.is_playing());
        assert!(
            p.dropped_frames() < 5,
            "seek-while-playing must not inflate dropped_frames, got {}",
            p.dropped_frames()
        );
    }

    /// Arming Loop In->Out while the playhead sits outside the range must seek to `a` first — otherwise
    /// `Clock::now`'s wrap math lands the next tick at an unpredictable point inside the range instead of
    /// a clean loop start. Already being inside the range must leave the playhead alone.
    #[test]
    fn set_loop_seeks_into_range_only_when_outside() {
        let path = media::ffpipe::tests::test_mp4();
        let asset = media::probe(&path, Backend::Auto).unwrap();
        let project = Project::from_media(asset);
        let mut p =
            Player::new(eframe::egui::Context::default(), Backend::Auto, Arc::new(Mutex::new(TextRasterizer::new())));
        p.set_project(&project);

        p.seek(3.9); // outside [0.5, 1.5)
        p.set_loop(Some((0.5, 1.5)));
        assert_eq!(p.time(), 0.5, "arming a loop while outside its range must seek to `in`");

        p.set_loop(None);
        p.seek(1.0); // inside [0.5, 1.5)
        p.set_loop(Some((0.5, 1.5)));
        assert_eq!(p.time(), 1.0, "arming a loop while already inside its range must not move the playhead");
    }

    /// While paused, `scrub` mixes audio without ever moving the clock.
    #[test]
    fn scrub_mixes_without_moving_clock() {
        let path = media::ffpipe::tests::test_mp4();
        let asset = media::probe(&path, Backend::Auto).unwrap();
        let project = Project::from_media(asset);
        let mut p =
            Player::new(eframe::egui::Context::default(), Backend::Auto, Arc::new(Mutex::new(TextRasterizer::new())));
        p.set_project(&project);
        p.set_canvas(320, 240, 1280);
        p.seek(1.0);
        let t0 = p.time();
        p.scrub(1.0);
        sleep(Duration::from_millis(150)); // let the one-BLOCK mix settle (no audio device on CI: harmless)
        assert_eq!(p.time(), t0, "scrub must never move the clock");
        assert!(!p.is_playing());
    }

    /// Two `request_layers` calls in quick succession before either resolves: `take_layers_reply`
    /// eventually surfaces only the second call's id, never the first's stale one.
    #[test]
    fn request_layers_newest_wins() {
        let path = media::ffpipe::tests::test_mp4();
        let asset = media::probe(&path, Backend::Auto).unwrap();
        let project = Project::from_media(asset);
        let mut p =
            Player::new(eframe::egui::Context::default(), Backend::Auto, Arc::new(Mutex::new(TextRasterizer::new())));
        p.set_project(&project);
        let _id1 = p.request_layers(0.5, 160);
        let id2 = p.request_layers(1.5, 160);
        let mut last = None;
        for _ in 0..300 {
            if let Some((rid, _set)) = p.take_layers_reply() {
                last = Some(rid);
            }
            sleep(Duration::from_millis(10));
        }
        assert_eq!(last, Some(id2), "only the newest request's id must ever surface");
    }

    /// After a scrub settles and nothing is outstanding, the render/audio threads have gone back to
    /// their zero-CPU idle block: no further cache/dropped-frame activity or stray frame happens on its
    /// own. (No hook into egui's own repaint-request count exists from a bare `Context::default()`, so
    /// this checks the observable proxies the selftest idle step ultimately cares about: nothing keeps
    /// working once nothing changed.)
    #[test]
    fn idle_no_repaint_while_paused_with_pending_scrub_state() {
        let path = media::ffpipe::tests::test_mp4();
        let asset = media::probe(&path, Backend::Auto).unwrap();
        let project = Project::from_media(asset);
        let mut p =
            Player::new(eframe::egui::Context::default(), Backend::Auto, Arc::new(Mutex::new(TextRasterizer::new())));
        p.set_project(&project);
        p.set_canvas(320, 240, 1280);
        p.seek(0.5);
        wait_frame(&mut p, 0.5, (320, 240));
        p.scrub(0.5);
        sleep(Duration::from_millis(150));
        let (h0, d0) = (p.cache_hits(), p.dropped_frames());
        sleep(Duration::from_millis(200)); // idle window: paused, nothing requested
        assert_eq!(p.cache_hits(), h0, "no further cache activity once idle");
        assert_eq!(p.dropped_frames(), d0, "no further dropped-frame accounting once idle");
        assert!(p.take_frame().is_none(), "no stray frame published while idle");
    }

    /// GPU mode: the render thread publishes decoded layers instead of a composited frame, and going
    /// back to the CPU path resumes composited frames. No GL context is involved.
    #[test]
    fn gpu_mode_publishes_layers() {
        let path = media::ffpipe::tests::test_mp4();
        let asset = media::probe(&path, Backend::Auto).unwrap();
        let project = Project::from_media(asset);
        let mut p =
            Player::new(eframe::egui::Context::default(), Backend::Auto, Arc::new(Mutex::new(TextRasterizer::new())));
        p.set_project(&project);
        p.set_canvas(320, 240, 1280);
        p.set_gpu(true);
        p.seek(0.5);
        let mut set = None;
        for _ in 0..300 {
            if let Some(s) = p.take_layers() {
                if s.layers.first().map(|(_, f)| (f.pts - 0.5).abs() < 0.2).unwrap_or(false) {
                    set = Some(s);
                    break;
                }
            }
            sleep(Duration::from_millis(10));
        }
        let set = set.expect("no layers within 3 s");
        assert_eq!(set.layers.len(), 1, "one video clip → one layer");
        let (id, frame) = &set.layers[0];
        assert_eq!(*id, project.all_clips().find(|(_, c)| c.is_visual()).unwrap().1.id);
        assert!(set.get(*id).is_some());
        let c = centre(frame);
        assert!(c[0] > 200 && c[1] < 70, "expected red at 0.5 s, got {c:?}");
        assert!(set.motion.is_empty(), "no motion-blur effect → no extra samples");

        // switching back to the CPU compositor resumes composited frames and stops layer publishing
        p.set_gpu(false);
        p.seek(2.5);
        let f = wait_frame(&mut p, 2.5, (320, 240));
        let c = centre(&f);
        assert!(c[1] > 200 && c[0] < 70, "expected green from the CPU path, got {c:?}");
        assert!(p.take_layers().is_none(), "layers must stop when the GPU path is off");
    }

    /// `decode_layers` owes the GPU renderer a bitmap for every visual clip, not just decoded media:
    /// text, shapes, nested sequences and the subtitle overlay used to silently vanish on the GPU path.
    #[test]
    fn layers_cover_text_shape_sequence_and_subtitles() {
        use crate::model::{ShapeKind, ShapeStyle};
        let mut project = Project::new();
        (project.width, project.height) = (320, 240);
        let txt = project.add_text_clip(0.0, 4.0);
        let shp = project.add_shape_clip(ShapeKind::Star, 0.0, 4.0);
        let sid = project.new_sequence("seq", 320, 240, 30.0);
        let mut inner = Clip::new(project.new_id(), ClipKind::Shape, "inner", 0.0, 4.0);
        inner.shape = Some(ShapeStyle::new(ShapeKind::Rect)); // an empty sequence lasts 0 s
        project.sequence_mut(sid).unwrap().tracks[0].clips.push(inner);
        let seq = project.insert_sequence_clip(sid, 0.0, None).expect("sequence clip");
        project.add_cue(0.0, 2.0, "HELLO");
        assert!(project.show_subtitles);

        let (mut pool, mut spare) = (DecoderPool::new(Backend::Ffmpeg), Vec::new());
        let (mut text, mut shapes, mut comp) = (TextRasterizer::new(), ShapeRasterizer::new(), Compositor::new());
        let set = decode_layers(&project, 1.0, 320, 240, &mut pool, &mut spare, &mut text, &mut shapes, &mut comp);
        assert!(set.get(shp).is_some(), "shape clip has no layer");
        assert!(set.get(seq).is_some(), "nested sequence has no layer");
        if text.families().is_empty() {
            eprintln!("no system fonts - skipping the text assertions");
            return;
        }
        assert!(set.get(txt).is_some(), "text clip has no layer");
        assert!(set.get(LayerSet::SUBTITLES).is_some(), "no subtitle layer");
        // ...and none after the cue ends / with subtitles off
        let set = decode_layers(&project, 3.0, 320, 240, &mut pool, &mut spare, &mut text, &mut shapes, &mut comp);
        assert!(set.get(LayerSet::SUBTITLES).is_none(), "subtitle layer outside the cue");
    }

    /// Inside a transition window both clips are drawn extended past their own bounds, so both need a
    /// layer — filtering by `contains(t)` made every GPU transition a hard cut.
    #[test]
    fn transition_window_yields_both_clips() {
        use crate::model::{Ease, ShapeKind, Transition, TransitionKind};
        let mut project = Project::new();
        (project.width, project.height) = (320, 240);
        let a = project.add_shape_clip(ShapeKind::Rect, 0.0, 2.0);
        let b = project.add_shape_clip(ShapeKind::Ellipse, 2.0, 2.0);
        let vt = project.video_tracks()[0];
        assert!(project.tracks[vt].clips.len() == 2, "the two clips must abut on one track");
        let id = project.new_id();
        project.tracks[vt].transitions.push(Transition {
            id,
            right: b,
            kind: TransitionKind::CrossFade,
            duration: 1.0,
            color: [0, 0, 0, 255],
            direction: 0,
            ease: Ease::Linear,
            edge: Default::default(),
        });
        let (mut pool, mut spare) = (DecoderPool::new(Backend::Ffmpeg), Vec::new());
        let (mut text, mut shapes, mut comp) = (TextRasterizer::new(), ShapeRasterizer::new(), Compositor::new());
        // 1.7 s = 20% through the 1.5–2.5 s window: only `a` contains(t), yet both must be handed over
        for t in [1.7, 2.3] {
            let set = decode_layers(&project, t, 320, 240, &mut pool, &mut spare, &mut text, &mut shapes, &mut comp);
            assert!(set.get(a).is_some(), "outgoing clip missing at t={t}");
            assert!(set.get(b).is_some(), "incoming clip missing at t={t}");
        }
        // outside the window only the clip under the playhead
        let set = decode_layers(&project, 0.5, 320, 240, &mut pool, &mut spare, &mut text, &mut shapes, &mut comp);
        assert!(set.get(a).is_some() && set.get(b).is_none());
    }

    /// A panicking decoder is contained: the caller lives on and the next call still runs.
    #[test]
    fn guarded_contains_a_panicking_decode() {
        let mut pool = DecoderPool::new(Backend::Ffmpeg);
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {})); // the panic below is expected: keep the log clean
        assert!(!guarded(&mut pool, |_| panic!("decoder exploded")));
        std::panic::set_hook(hook);
        let mut ran = false;
        assert!(guarded(&mut pool, |_| ran = true));
        assert!(ran);
    }
}
