//! Player: owns the render thread (Compositor + DecoderPool → latest Frame) and the audio thread
//! (Mixer + DecoderPool → ring buffer → cpal WASAPI output). The wall clock is the master when playing:
//! time = base_time + elapsed. Video frames are rendered at the project fps and published via
//! `take_frame`; `ctx.request_repaint()` is called whenever a new frame is ready.
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
use crate::engine::shapes::ShapeRasterizer;
use crate::engine::text::TextRasterizer;
use crate::media::{Backend, DecoderPool, Frame, SAMPLE_RATE};
use crate::model::{Clip, ClipKind, Project, TrackKind};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Audio mixed per block (frames) and how far ahead of the clock the ring is kept.
const BLOCK: usize = 1024;
const LEAD_SECS: f64 = 0.12;

/// Wall clock + canvas shared by the UI and both threads. `now()` flips `playing` off at the end.
struct Clock {
    playing: bool,
    base_t: f64,
    base_at: Instant,
    duration: f64,
    /// Render size in pixels (already clamped to the preview max width); (0,0) = nothing to render.
    canvas: (u32, u32),
}

impl Clock {
    fn now(&mut self) -> f64 {
        if !self.playing {
            return self.base_t;
        }
        let t = self.base_t + self.base_at.elapsed().as_secs_f64();
        if t < self.duration {
            return t;
        }
        self.playing = false;
        self.base_t = self.duration;
        self.duration
    }
}

struct Shared {
    clock: Mutex<Clock>,
    /// Newest rendered frame not yet taken by the UI.
    frame: Mutex<Option<Arc<Frame>>>,
    /// GPU mode: newest decoded layer set not yet taken by the UI (no CPU compositing happened).
    layers: Mutex<Option<Arc<LayerSet>>>,
    project: Mutex<Arc<Project>>,
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
    ClearDecoders(SyncSender<()>),
    /// One-shot render at time t, at most max_w px wide (project aspect), reply on the channel.
    RenderOnce(f64, u32, SyncSender<Arc<Frame>>),
    /// One-shot layer decode at time t (GPU path: the UI thread renders them), reply on the channel.
    LayersOnce(f64, u32, SyncSender<Arc<LayerSet>>),
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
            }),
            frame: Mutex::new(None),
            layers: Mutex::new(None),
            project: Mutex::new(Arc::new(Project::new())),
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
    // double buffer: `held` = last published, `spare` = the one before (free again once the UI took it)
    let (mut held, mut spare): (Option<Arc<Frame>>, Option<Arc<Frame>>) = (None, None);
    let mut dirty = false;
    let mut pending: Option<Cmd> = None;
    // GPU mode state: published layer set + recycled layer buffers
    let mut gpu = false;
    let mut held_layers: Option<Arc<LayerSet>> = None;
    let mut spare_layers: Vec<Frame> = Vec::new();
    loop {
        let playing = {
            let mut c = lock(&shared.clock);
            c.now();
            c.playing
        };
        if pending.is_none() && !dirty && !playing {
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
                    project = lock(&shared.project).clone();
                    dirty = true;
                }
                Cmd::Seek | Cmd::Play | Cmd::Pause | Cmd::Canvas => dirty = true,
                Cmd::Backend(b) => {
                    pool.set_backend(b);
                    dirty = true;
                }
                Cmd::Gpu(on) => {
                    if gpu != on {
                        gpu = on;
                        *lock(&shared.layers) = None;
                        held_layers = None;
                        dirty = true;
                    }
                }
                Cmd::ClearDecoders(ack) => {
                    pool.clear();
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
                Cmd::Quit => return,
            }
        }
        let (playing, t, (w, h)) = {
            let mut c = lock(&shared.clock);
            let t = c.now();
            (c.playing, t, c.canvas)
        };
        if !playing && !dirty {
            continue;
        }
        dirty = false;
        let start = Instant::now();
        if gpu && w > 0 && h > 0 {
            // the UI thread owns the GL context: decode/rasterise here, render there
            let mut set = LayerSet::default();
            {
                let text = &mut lock(&text);
                guarded(&mut pool, |pool| {
                    set = decode_layers(&project, t, w, h, pool, &mut spare_layers, text, &mut shapes, &mut comp)
                });
            }
            let set = Arc::new(set);
            *lock(&shared.layers) = Some(set.clone());
            recycle(held_layers.replace(set), &mut spare_layers); // buffers of the set before that
            ctx.request_repaint();
        } else if w > 0 && h > 0 {
            let mut frame = match spare.take().map(Arc::try_unwrap) {
                Some(Ok(f)) => f, // UI is done with it: no allocation
                _ => Frame::default(),
            };
            guarded(&mut pool, |pool| comp.render(&project, t, w, h, pool, &mut lock(&text), &mut frame));
            let frame = Arc::new(frame);
            *lock(&shared.frame) = Some(frame.clone()); // replaces an untaken (stale) frame: latest wins
            spare = held.replace(frame);
            ctx.request_repaint();
        }
        if playing {
            // pace at the project fps from this render's start; if we are behind we simply render the
            // current clock time next (never queue up). Commands are picked up by the drain above.
            // ponytail: thread::sleep uses Windows' high-resolution waitable timer; recv_timeout rounds to
            // the 15.6 ms scheduler tick (30 fps -> ~21 fps). timeBeginPeriod(1) would also work but costs power.
            let due = start + Duration::from_secs_f64(1.0 / project.fps.max(1.0));
            std::thread::sleep(due.saturating_duration_since(Instant::now()));
        }
    }
}

/// Everything `engine::gpu` needs for one timeline instant (its `LayerSet` contract): one bitmap per
/// visual clip on screen at `t` — **both** clips of a transition, which the renderer draws virtually
/// extended past their own bounds — plus the subtitle bitmap under `LayerSet::SUBTITLES`. Media is
/// decoded at the size the compositor would have used (placement, capped at the source's native size);
/// text/shape clips are rasterised and nested sequences composited on the CPU, all at (w, h) — which is
/// already the quality-scaled render size, so `gpu::render_size` must not be applied again. Adjustment
/// layers need no bitmap (they re-process the canvas). Effects that `needs_motion()` also get neighbours.
#[allow(clippy::too_many_arguments)]
fn decode_layers(
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
            for clip in [left, right] {
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
    if project.show_subtitles {
        if let Some(cue) = project.cue_at(t).filter(|c| !c.text.trim().is_empty()) {
            let mut style = project.subtitle_style.clone();
            style.text.clone_from(&cue.text);
            let img = text.render(&style, w as f32 / project.width.max(1) as f32);
            if img.width > 1 || img.height > 1 {
                set.layers.push((LayerSet::SUBTITLES, img));
            }
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
    if !clip.draws() {
        return;
    }
    let s = w as f32 / project.width.max(1) as f32;
    match clip.kind {
        ClipKind::Video | ClipKind::Image => {
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
            let img = text.render(style, s);
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
    let dec = pool.video(path)?;
    if !dec.frame_at(st, dw, dh, &mut frame) {
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
    let mut pending: Option<Cmd> = None;
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
                }
                Cmd::Pause => lock(&ring).clear(),
                Cmd::Canvas => {}
                Cmd::Backend(b) => pool.set_backend(b),
                Cmd::Gpu(_) => {} // render-thread only
                Cmd::ClearDecoders(ack) => {
                    pool.clear();
                    let _ = ack.send(());
                }
                Cmd::RenderOnce(..) | Cmd::LayersOnce(..) => {} // render-thread only
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
        let queued = lock(&ring).len() as f64 / (2 * SAMPLE_RATE) as f64;
        if playing && queued < LEAD_SECS {
            // ponytail: wall clock is the master; the ring is refilled only as the device drains it, so it
            // can't grow past LEAD + one block. The ring head plays at timeline time `mixed_until - queued`;
            // if that is already >50 ms in the past (slow first block, decoder respawn, underrun) snap the
            // next block to land on time instead of carrying the lag forever. Resample if drift ever matters.
            let now = lock(&shared.clock).now();
            if now - (mixed_until - queued) > 0.05 {
                mixed_until = now + queued;
            }
            if guarded(&mut pool, |pool| mixer.mix(&project, mixed_until, pool, &mut block)) {
                lock(&ring).extend(block.iter().copied()); // a panicked block would be garbage: underrun instead
            }
            mixed_until += BLOCK as f64 / SAMPLE_RATE as f64;
        } else {
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
                assert!(f.pts > 2.5 && f.pts <= now, "pts {} vs time {now}", f.pts);
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
        p.set_canvas(160, 120, 100); // clamp keeps aspect, and the preview re-renders after release
        let t = p.time();
        wait_frame(&mut p, t, (100, 75));

        // render_once: synchronous frame for tools (MCP render.frame), independent of the preview canvas
        let f = p.render_once(2.5, 160).expect("render_once");
        assert_eq!((f.width, f.height), (160, 120));
        let c = centre(&f);
        assert!(c[0] < 70 && c[1] > 200, "expected green from render_once at 2.5 s, got {c:?}");
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
