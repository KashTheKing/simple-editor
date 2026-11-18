//! The player's media pipeline: one file, read strictly front to back.
//!
//! - **Video thread**: opens the file zero-copy where it can (`mf::open_video_gpu`: hardware decode into
//!   D3D11, the video processor converts NV12 -> RGB and scales to the window on the GPU), else the
//!   usual MF / ffmpeg source. It decodes ahead into a small queue (`QUEUE` frames) and sleeps while it is
//!   full. Seeks only when asked to (`Engine::seek`, i.e. a scrub or a skip).
//! - **Audio thread**: decodes ahead into a ring (`RING_SECS`) and owns the output stream (cpal / WASAPI).
//!   The device callback is the clock: each buffer it plays moves the anchor `(source time, instant it
//!   is heard)`, so the picture follows what is actually coming out of the speakers.
//! - No audio (or no output device): a wall clock.
//!
//! The UI thread only asks `time()`, takes the frame due (`frame_due`) and sleeps until the next one
//! is (`next_due`). Nothing here runs between frames or while paused, except the decode-ahead filling
//! up once.
//!
//! Started from `main` before the window exists (`Engine::start`), so opening the file and decoding the
//! first frame overlap window and GL creation.

use crate::media::{self, AudioSource, Backend, Frame, VideoSource, SAMPLE_RATE};
use crate::model::{AudioStreamInfo, ClipKind};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Frames decoded ahead. Small: playback is linear, so a few frames cover any scheduling hiccup, and
/// a seek throws away little.
pub const QUEUE: usize = 6;
/// Seconds of audio decoded ahead.
const RING_SECS: f64 = 0.35;
/// Audio is decoded in blocks of this many frames.
const BLOCK: usize = 1024;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// What the file is, known once the video thread has opened it.
#[derive(Clone, Debug, Default)]
pub struct Info {
    pub path: PathBuf,
    pub duration: f64,
    pub fps: f64,
    /// Native (display) size; (0, 0) for audio.
    pub size: (u32, u32),
    pub has_video: bool,
    pub audio_streams: Vec<AudioStreamInfo>,
    pub codec: String,
    /// "Hardware, zero-copy", "Hardware" or "Software" - for Video info.
    pub decoder: &'static str,
}

#[derive(Default)]
struct Video {
    queue: VecDeque<Arc<Frame>>,
    /// Bumped by every seek: a frame decoded for an older one is thrown away.
    gen: u64,
    seek_to: Option<f64>,
    eof: bool,
    /// Physical pixels the picture is drawn at (the video processor scales to it); (0, 0) = native.
    want: (u32, u32),
}

#[derive(Default)]
struct Audio {
    ring: VecDeque<f32>,
    /// Source time of `ring[0]`.
    head_t: f64,
    gen: u64,
    seek_to: Option<f64>,
    eof: bool,
    stream: usize,
    /// Fractional read position (source frames) into `ring`, for rates other than 1.
    frac: f64,
    /// Last few hundred output samples (mono), for the audio-only visualizer.
    tap: VecDeque<f32>,
}

struct Clock {
    playing: bool,
    rate: f64,
    /// Source time `t` is heard at instant `at`; `limit` caps it (the end of what is queued to play).
    t: f64,
    at: Instant,
    limit: f64,
}

impl Clock {
    fn now(&self, now: Instant) -> f64 {
        if !self.playing {
            return self.t;
        }
        let dt = now.saturating_duration_since(self.at).as_secs_f64();
        (self.t + dt * self.rate).min(self.limit)
    }
    fn set(&mut self, t: f64) {
        self.t = t;
        self.at = Instant::now();
        self.limit = f64::INFINITY;
    }
}

struct Shared {
    path: PathBuf,
    info: Mutex<Option<Result<Info, String>>>,
    video: Mutex<Video>,
    video_cv: Condvar,
    audio: Mutex<Audio>,
    audio_cv: Condvar,
    clock: Mutex<Clock>,
    /// The output device is open and drives the clock.
    audio_clock: AtomicBool,
    quit: AtomicBool,
    volume: AtomicU32,
    muted: AtomicBool,
    dropped: AtomicU64,
    /// Called when something the UI shows arrives (a frame into an empty queue, the file info).
    wake: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    /// Hardware zero-copy decode allowed (off in tests that want CPU frames).
    gpu: bool,
}

impl Shared {
    fn wake(&self) {
        if let Some(w) = lock(&self.wake).clone() {
            w();
        }
    }
}

pub struct Engine {
    s: Arc<Shared>,
    threads: Vec<std::thread::JoinHandle<()>>,
    pub started: Instant,
}

impl Engine {
    /// Open `path` and start decoding at `at` (paused when `!play`). Returns at once; `info()` is None
    /// until the file is open.
    pub fn start(path: &Path, at: f64, play: bool, gpu: bool) -> Engine {
        let s = Arc::new(Shared {
            path: path.to_path_buf(),
            info: Mutex::new(None),
            video: Mutex::new(Video { seek_to: Some(at), ..Default::default() }),
            video_cv: Condvar::new(),
            audio: Mutex::new(Audio { head_t: at, seek_to: Some(at), ..Default::default() }),
            audio_cv: Condvar::new(),
            clock: Mutex::new(Clock { playing: play, rate: 1.0, t: at, at: Instant::now(), limit: f64::INFINITY }),
            audio_clock: AtomicBool::new(false),
            quit: AtomicBool::new(false),
            volume: AtomicU32::new(1.0f32.to_bits()),
            muted: AtomicBool::new(false),
            dropped: AtomicU64::new(0),
            wake: Mutex::new(None),
            gpu,
        });
        if gpu {
            // the D3D11 device takes ~0.25 s: build it while the file is probed and the window opens
            let _ = std::thread::Builder::new().name("player-d3d".into()).spawn(|| {
                crate::media::mf::init_thread();
                let _ = crate::media::mf::d3d_device();
            });
        }
        let v = s.clone();
        let threads = vec![std::thread::Builder::new()
            .name("player-video".into())
            .spawn(move || video_thread(v))
            .expect("spawn player video thread")];
        Engine { s, threads, started: Instant::now() }
    }

    /// Repaint hook: called from the decode threads when the UI has something new to show.
    pub fn set_wake(&self, f: Arc<dyn Fn() + Send + Sync>) {
        *lock(&self.s.wake) = Some(f);
        if lock(&self.s.info).is_some() || !lock(&self.s.video).queue.is_empty() {
            self.s.wake();
        }
    }

    pub fn path(&self) -> &Path {
        &self.s.path
    }

    /// None while the file is still opening.
    pub fn info(&self) -> Option<Result<Info, String>> {
        lock(&self.s.info).clone()
    }

    /// Wait (up to `max`) for the first frame or for the file to turn out to have none.
    pub fn wait_first(&self, max: Duration) {
        let end = Instant::now() + max;
        let mut v = lock(&self.s.video);
        while v.queue.is_empty() && !v.eof {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() || matches!(*lock(&self.s.info), Some(Err(_))) {
                break;
            }
            if matches!(&*lock(&self.s.info), Some(Ok(i)) if !i.has_video) {
                break;
            }
            v = self.s.video_cv.wait_timeout(v, left.min(Duration::from_millis(5))).map(|r| r.0).unwrap_or_else(|e| e.into_inner().0);
        }
    }

    pub fn time(&self) -> f64 {
        lock(&self.s.clock).now(Instant::now())
    }

    pub fn is_playing(&self) -> bool {
        lock(&self.s.clock).playing
    }

    pub fn duration(&self) -> f64 {
        match &*lock(&self.s.info) {
            Some(Ok(i)) => i.duration,
            _ => 0.0,
        }
    }

    pub fn set_playing(&self, on: bool) {
        let mut c = lock(&self.s.clock);
        if c.playing == on {
            return;
        }
        let t = c.now(Instant::now());
        c.playing = on;
        c.set(t);
        self.hold_for_audio(&mut c);
        drop(c);
        self.s.audio_cv.notify_all();
    }

    /// With the device driving the clock, time waits at `t` until the first buffer from there is
    /// queued to play (the callback re-anchors it) - the picture never runs ahead of the sound.
    fn hold_for_audio(&self, c: &mut Clock) {
        if self.s.audio_clock.load(Ordering::Relaxed) {
            c.limit = c.t;
        }
    }

    pub fn rate(&self) -> f64 {
        lock(&self.s.clock).rate
    }

    pub fn set_rate(&self, rate: f64) {
        let mut c = lock(&self.s.clock);
        let t = c.now(Instant::now());
        c.rate = rate.clamp(0.25, 4.0);
        c.set(t);
    }

    pub fn volume(&self) -> f32 {
        f32::from_bits(self.s.volume.load(Ordering::Relaxed))
    }
    pub fn set_volume(&self, v: f32) {
        self.s.volume.store(v.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }
    pub fn muted(&self) -> bool {
        self.s.muted.load(Ordering::Relaxed)
    }
    pub fn set_muted(&self, m: bool) {
        self.s.muted.store(m, Ordering::Relaxed);
    }

    /// Frames shown late enough that a newer one replaced them unseen.
    pub fn dropped(&self) -> u64 {
        self.s.dropped.load(Ordering::Relaxed)
    }

    /// Jump to `t` (clamped to the file). The only non-linear access: the decoders seek here.
    pub fn seek(&self, t: f64) {
        let dur = self.duration();
        let t = if dur > 0.0 { t.clamp(0.0, (dur - 0.001).max(0.0)) } else { t.max(0.0) };
        {
            let mut c = lock(&self.s.clock);
            c.set(t);
            self.hold_for_audio(&mut c);
        }
        {
            let mut v = lock(&self.s.video);
            v.gen += 1;
            v.seek_to = Some(t);
            v.queue.clear();
            v.eof = false;
        }
        self.s.video_cv.notify_all();
        {
            let mut a = lock(&self.s.audio);
            a.gen += 1;
            a.seek_to = Some(t);
            a.ring.clear();
            a.head_t = t;
            a.frac = 0.0;
            a.eof = false;
        }
        self.s.audio_cv.notify_all();
    }

    /// `seek(time() + by)`.
    pub fn skip(&self, by: f64) {
        self.seek(skip_target(self.time(), by, self.duration()));
    }

    /// Play another audio stream of the file from the current time.
    pub fn set_audio_stream(&self, n: usize) {
        let t = self.time();
        {
            let mut a = lock(&self.s.audio);
            a.stream = n;
        }
        self.seek(t);
    }

    pub fn audio_stream(&self) -> usize {
        lock(&self.s.audio).stream
    }

    /// The size (physical pixels) the picture is drawn at: the GPU video processor scales to it.
    pub fn set_output_size(&self, w: u32, h: u32) {
        let mut v = lock(&self.s.video);
        if v.want != (w, h) {
            v.want = (w, h);
        }
    }

    /// The frame to show at time `t` - the newest one due, dropping older ones (counted as dropped
    /// when more than one is skipped over during playback). None: keep what is on screen.
    pub fn frame_due(&self, t: f64) -> Option<Arc<Frame>> {
        let mut v = lock(&self.s.video);
        let mut out = None;
        let mut skipped = 0u64;
        while v.queue.front().is_some_and(|f| f.pts <= t + 0.002) {
            if out.is_some() {
                skipped += 1;
            }
            out = v.queue.pop_front();
        }
        // paused after a seek: the first frame at/after the seek point is the one to show
        if out.is_none() && !self.is_playing() {
            out = v.queue.pop_front();
        }
        let freed = out.is_some();
        drop(v);
        if freed {
            self.s.video_cv.notify_all();
        }
        if skipped > 0 && self.is_playing() {
            self.s.dropped.fetch_add(skipped, Ordering::Relaxed);
        }
        out
    }

    /// Source time of the next queued frame (None: nothing queued yet).
    pub fn next_due(&self) -> Option<f64> {
        lock(&self.s.video).queue.front().map(|f| f.pts)
    }

    /// The video ran out and everything decoded has been shown (audio: the clock passed the end).
    pub fn at_end(&self) -> bool {
        let dur = self.duration();
        let v = lock(&self.s.video);
        let video_done = v.eof && v.queue.is_empty();
        drop(v);
        let has_video = matches!(&*lock(&self.s.info), Some(Ok(i)) if i.has_video);
        if has_video {
            video_done
        } else {
            dur > 0.0 && self.time() >= dur - 0.01
        }
    }

    /// Recent output samples (mono), oldest first - the audio-only visualizer reads these.
    pub fn tap(&self) -> Vec<f32> {
        lock(&self.s.audio).tap.iter().copied().collect()
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.s.quit.store(true, Ordering::Relaxed);
        self.s.video_cv.notify_all();
        self.s.audio_cv.notify_all();
        *lock(&self.s.wake) = None;
        // the threads see `quit` within one wait; never block the UI on a decoder stuck in a read
        for t in self.threads.drain(..) {
            drop(t);
        }
    }
}

/// `t + by`, kept inside the file (and off its very last frame, so a skip at the end still shows one).
pub fn skip_target(t: f64, by: f64, duration: f64) -> f64 {
    let end = if duration > 0.0 { (duration - 0.001).max(0.0) } else { f64::INFINITY };
    (t + by).clamp(0.0, end)
}

/// A zero-copy source being brought up beside the software one the picture starts on, and the time
/// it was decoded up to (`upgrade`).
type Upgrade = std::sync::mpsc::Receiver<(Box<dyn VideoSource>, f64)>;

/// How far ahead of the clock `upgrade` positions the zero-copy source, so the hand-over is a plain
/// next frame (no seek) on the video thread.
const UPGRADE_LEAD: f64 = 0.5;

/// Up to this size the software reader (`mf::open_video`, no D3D) starts the picture: its first frame
/// is out in ~0.25 s, where the hardware decoder's setup took 1-2.5 s (measured on an RTX 2060,
/// 1080p60 H.264). Bigger sources go straight to zero-copy - software decode would not keep up.
const SOFTWARE_FIRST_PIXELS: u64 = 2560 * 1440;

/// Open the video and probe the file. Up to `SOFTWARE_FIRST_PIXELS` the software reader comes back at
/// once and `upgrade` brings up zero-copy behind it; bigger, it is zero-copy (else the usual source).
fn open(s: &Arc<Shared>) -> Result<(Info, Option<Box<dyn VideoSource>>, Option<Upgrade>), String> {
    let p = s.path.to_string_lossy().into_owned();
    crate::media::mf::init_thread();
    crate::perf::startup_mark("player: mf init");
    let asset = media::mf::probe(&p).or_else(|e| media::ffpipe::probe(&p).map_err(|e2| format!("{e}; {e2}")))?;
    let has_video = asset.kind == ClipKind::Video && asset.width > 0;
    let mut info = Info {
        path: s.path.clone(),
        duration: asset.duration,
        fps: if asset.fps > 1.0 { asset.fps } else { 30.0 },
        size: (asset.width, asset.height),
        has_video,
        audio_streams: asset.audio_streams.clone(),
        codec: asset.codec.clone(),
        decoder: "Software",
    };
    if !has_video {
        return Ok((info, None, None));
    }
    crate::perf::startup_mark("player: probed");
    let gpu = s.gpu && !media::is_image_path(&p);
    let small = (asset.width as u64) * (asset.height as u64) < SOFTWARE_FIRST_PIXELS;
    let (src, up) = if gpu && small {
        let up = upgrade(s.clone(), p.clone());
        match media::open_video(&p, Backend::Auto) {
            Ok(v) => (v, Some(up)),
            // no software reader for it: wait for the zero-copy one instead
            Err(e) => match up.recv() {
                Ok((v, _)) => {
                    info.decoder = "Hardware, zero-copy";
                    (v, None)
                }
                Err(_) => return Err(e),
            },
        }
    } else if gpu {
        match media::mf::open_video_gpu(&p) {
            Ok(v) => {
                info.decoder = "Hardware, zero-copy";
                (v, None)
            }
            Err(_) => (media::open_video(&p, Backend::Auto)?, None),
        }
    } else {
        (media::open_video(&p, Backend::Auto)?, None)
    };
    if info.size.0 == 0 {
        info.size = src.size();
    }
    Ok((info, Some(src), up))
}

/// Bring up the zero-copy source on a thread of its own (D3D device, hardware decoder, first frame)
/// and pre-roll it to `UPGRADE_LEAD` ahead of the clock. Nothing arrives when it can't (no D3D video
/// device, a software-only codec) - the software source simply carries on.
fn upgrade(s: Arc<Shared>, p: String) -> Upgrade {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let _ = std::thread::Builder::new().name("player-gpu-open".into()).spawn(move || {
        crate::media::mf::init_thread();
        let Ok(mut src) = media::mf::open_video_gpu(&p) else { return };
        crate::perf::startup_mark("player: zero-copy ready");
        if s.quit.load(Ordering::Relaxed) {
            return;
        }
        let at = lock(&s.clock).now(Instant::now()) + UPGRADE_LEAD;
        let (mut w, mut h) = lock(&s.video).want;
        if w == 0 || h == 0 {
            (w, h) = src.size();
        }
        let Some(f) = src.frame_gpu(at, w, h) else { return };
        let _ = tx.send((src, f.pts));
    });
    rx
}

/// Hand the picture to the zero-copy source once the software one reaches where it was pre-rolled
/// to (`at`), or at once when nothing plays or a seek moved far from it.
fn take_upgrade(up: &Option<Upgrade>, pending: &mut Option<(Box<dyn VideoSource>, f64)>, s: &Shared, next_t: f64, dur: f64) -> Option<Box<dyn VideoSource>> {
    if pending.is_none() {
        *pending = up.as_ref()?.try_recv().ok();
    }
    let at = pending.as_ref()?.1;
    let playing = lock(&s.clock).playing;
    let swap = !playing || next_t >= at - dur * 0.5 || next_t < at - UPGRADE_LEAD - 1.0;
    swap.then(|| pending.take().map(|(v, _)| v)).flatten()
}

fn video_thread(s: Arc<Shared>) {
    crate::perf::startup_mark("player: video thread");
    let opened = open(&s);
    crate::perf::startup_mark("player: file open");
    let (info, src, up) = match opened {
        Ok((i, v, u)) => (i, v, u),
        Err(e) => {
            *lock(&s.info) = Some(Err(e));
            lock(&s.video).eof = true;
            s.video_cv.notify_all();
            s.wake();
            return;
        }
    };
    let audio = !info.audio_streams.is_empty();
    let fps = info.fps;
    *lock(&s.info) = Some(Ok(info.clone()));
    if audio {
        let a = s.clone();
        let _ = std::thread::Builder::new().name("player-audio".into()).spawn(move || audio_thread(a));
    }
    s.wake();
    let Some(mut src) = src else {
        lock(&s.video).eof = true;
        s.video_cv.notify_all();
        return;
    };
    crate::media::bg::raise_thread();
    let dur = 1.0 / fps;
    let mut next_t = 0.0;
    let mut last_pts = f64::NEG_INFINITY;
    let mut spare: Vec<Frame> = Vec::new();
    let mut pending = None;
    let mut up = up;
    let mut marked = false;
    // a seek at (or a hair before) the end asks for a time past the last frame's start: one retry there
    let mut retried = false;
    loop {
        let (gen, want) = {
            let mut v = lock(&s.video);
            loop {
                if s.quit.load(Ordering::Relaxed) {
                    return;
                }
                if let Some(t) = v.seek_to.take() {
                    next_t = t;
                    last_pts = f64::NEG_INFINITY;
                    retried = false;
                }
                if !v.eof && v.queue.len() < QUEUE {
                    break;
                }
                v = s.video_cv.wait(v).unwrap_or_else(|e| e.into_inner());
            }
            (v.gen, v.want)
        };
        if let Some(v) = take_upgrade(&up, &mut pending, &s, next_t, dur) {
            src = v;
            up = None;
            if let Some(Ok(i)) = lock(&s.info).as_mut() {
                i.decoder = "Hardware, zero-copy";
            }
            crate::perf::startup_mark("player: on zero-copy");
        }
        let (w, h) = if want.0 > 0 && want.1 > 0 { want } else { info.size };
        let f = src.frame_gpu(next_t, w, h).or_else(|| src.frame_shared(next_t, w, h, &mut spare));
        let mut v = lock(&s.video);
        if v.gen != gen {
            continue; // a seek came in while decoding: the frame is for the old position
        }
        match f {
            None if last_pts == f64::NEG_INFINITY && !retried && next_t > dur => {
                // nothing at the seek point: it is past the last frame's start - show that frame
                retried = true;
                next_t = (info.duration - dur).min(next_t - dur).max(0.0);
            }
            None => {
                v.eof = true;
                drop(v);
                s.video_cv.notify_all();
                s.wake();
            }
            Some(f) if f.pts <= last_pts + 1e-6 => {
                // the decoder held the same frame (a VFR gap): look a little further on
                next_t += dur * 0.5;
            }
            Some(f) => {
                last_pts = f.pts;
                next_t = f.pts + dur;
                let first = v.queue.is_empty();
                if !marked {
                    marked = true;
                    crate::perf::startup_mark("player: frame decoded");
                }
                v.queue.push_back(f);
                drop(v);
                s.video_cv.notify_all();
                if first {
                    s.wake();
                }
            }
        }
    }
}

fn audio_thread(s: Arc<Shared>) {
    use cpal::traits::StreamTrait;
    crate::media::mf::init_thread();
    let p = s.path.to_string_lossy().into_owned();
    let stream = open_output(s.clone());
    s.audio_clock.store(stream.is_some(), Ordering::Relaxed);
    let mut stream_playing = false;
    let mut src: Option<(usize, Box<dyn AudioSource>)> = None;
    let mut next_t = 0.0;
    let mut gen = u64::MAX;
    let mut block = vec![0f32; BLOCK * 2];
    let ahead = (RING_SECS * SAMPLE_RATE as f64) as usize * 2;
    loop {
        if s.quit.load(Ordering::Relaxed) {
            return;
        }
        let playing = lock(&s.clock).playing;
        if let Some(st) = &stream {
            // a paused player keeps no audio device busy
            if playing != stream_playing {
                stream_playing = playing;
                let _ = if playing { st.play() } else { st.pause() };
            }
        }
        let stream_n = {
            let mut a = lock(&s.audio);
            if let Some(t) = a.seek_to.take() {
                next_t = t;
                gen = a.gen;
            }
            if a.ring.len() >= ahead || a.eof {
                let _ = s.audio_cv.wait_timeout(a, Duration::from_millis(100));
                continue;
            }
            a.stream
        };
        if src.as_ref().is_none_or(|(n, _)| *n != stream_n) {
            src = media::open_audio(&p, stream_n, Backend::Auto).ok().map(|a| (stream_n, a));
        }
        let Some((_, a_src)) = src.as_mut() else {
            lock(&s.audio).eof = true;
            continue;
        };
        let duration = a_src.duration();
        a_src.read_at(next_t, &mut block);
        let mut a = lock(&s.audio);
        if a.gen != gen {
            continue;
        }
        a.ring.extend(block.iter().copied());
        next_t += BLOCK as f64 / SAMPLE_RATE as f64;
        if duration > 0.0 && next_t >= duration {
            a.eof = true;
        }
    }
}

/// The output stream; its callback plays the ring and moves the clock. None: no device - the wall
/// clock runs the show.
fn open_output(s: Arc<Shared>) -> Option<cpal::Stream> {
    use cpal::traits::{DeviceTrait, HostTrait};
    let device = cpal::default_host().default_output_device()?;
    let config = cpal::StreamConfig { channels: 2, sample_rate: SAMPLE_RATE, buffer_size: cpal::BufferSize::Default };
    let cb = s.clone();
    device
        .build_output_stream(
            config,
            move |out: &mut [f32], info: &cpal::OutputCallbackInfo| fill(&cb, out, info),
            |e| eprintln!("player audio: {e}"),
            None,
        )
        .ok()
}

/// One device buffer: resample the ring at the playback rate, apply volume, and re-anchor the clock
/// on when this buffer will be heard.
fn fill(s: &Shared, out: &mut [f32], info: &cpal::OutputCallbackInfo) {
    let ts = info.timestamp();
    let latency = ts.playback.duration_since(ts.callback);
    let heard_at = Instant::now() + latency;
    let (playing, rate) = {
        let c = lock(&s.clock);
        (c.playing, c.rate)
    };
    let gain = if s.muted.load(Ordering::Relaxed) { 0.0 } else { f32::from_bits(s.volume.load(Ordering::Relaxed)) };
    let mut a = lock(&s.audio);
    let frames = out.len() / 2;
    if !playing {
        out.fill(0.0);
        return;
    }
    let start_t = a.head_t + a.frac / SAMPLE_RATE as f64;
    let mut played = 0usize;
    for i in 0..frames {
        let idx = a.frac.floor() as usize;
        let k = (a.frac - idx as f64) as f32;
        if (idx + 1) * 2 + 1 >= a.ring.len() {
            if a.eof {
                // past the end of the audio: silence, but time goes on (video may be longer)
                out[i * 2] = 0.0;
                out[i * 2 + 1] = 0.0;
                a.frac += rate;
                played += 1;
                continue;
            }
            out[i * 2..].fill(0.0); // underrun: the clock waits for the decoder
            break;
        }
        for c in 0..2 {
            let x0 = a.ring[idx * 2 + c];
            let x1 = a.ring[(idx + 1) * 2 + c];
            out[i * 2 + c] = (x0 + (x1 - x0) * k) * gain;
        }
        a.frac += rate;
        played += 1;
    }
    let consumed = (a.frac.floor() as usize).min(a.ring.len() / 2);
    a.ring.drain(..consumed * 2);
    a.frac -= consumed as f64;
    a.head_t += consumed as f64 / SAMPLE_RATE as f64;
    let end_t = a.head_t + a.frac / SAMPLE_RATE as f64;
    let tap = out.chunks_exact(2).map(|c| (c[0] + c[1]) * 0.5).collect::<Vec<_>>();
    a.tap.extend(tap);
    let over = a.tap.len().saturating_sub(1024);
    a.tap.drain(..over);
    drop(a);
    s.audio_cv.notify_all();
    if s.audio_clock.load(Ordering::Relaxed) && played > 0 {
        let mut c = lock(&s.clock);
        if c.playing {
            c.t = start_t;
            c.at = heard_at;
            c.limit = end_t;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skip_stays_inside_the_file() {
        assert_eq!(skip_target(5.0, -10.0, 60.0), 0.0);
        assert_eq!(skip_target(20.0, -10.0, 60.0), 10.0);
        assert_eq!(skip_target(20.0, 30.0, 60.0), 50.0);
        assert!((skip_target(45.0, 30.0, 60.0) - 59.999).abs() < 1e-9, "not past the last frame");
    }

    #[test]
    fn paused_clock_holds_and_playing_clock_runs_at_rate() {
        let at = Instant::now();
        let mut c = Clock { playing: false, rate: 1.0, t: 4.0, at, limit: f64::INFINITY };
        assert_eq!(c.now(at + Duration::from_secs(3)), 4.0);
        c.playing = true;
        c.rate = 2.0;
        assert!((c.now(at + Duration::from_millis(500)) - 5.0).abs() < 1e-9);
        c.limit = 4.2;
        assert_eq!(c.now(at + Duration::from_secs(1)), 4.2, "never runs past what is queued to be heard");
    }
}
