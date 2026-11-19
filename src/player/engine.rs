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
//! - **Reverse** (`set_reverse`): the video thread decodes chunks backwards - up to `REV_FRAMES`,
//!   starting on the keyframe before them when it is that near (each frame decoded once), else a seek's
//!   worth before where the last chunk began - and queues each newest first, keeping two chunks ahead.
//!   The audio thread decodes `REV_AUDIO` seconds at a time backwards and puts them in the ring
//!   time-mirrored, so the device plays the sound reversed (at the rate, pitch following speed as it
//!   does forwards) and still drives the clock, now running down.
//! - **Audio tracks** (`set_audio_streams`): every ticked stream is decoded and mixed into the ring.
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
/// Frames decoded per backwards step at most. The next chunk is decoded while fewer than two are queued,
/// so up to three are held at once - under the zero-copy source's `d3d::MAX_LIVE` surfaces.
// ponytail: a frame count; a byte budget if reverse of big CPU-decoded frames ever runs short of RAM
pub const REV_FRAMES: usize = 36;
/// Seconds of audio decoded per backwards step (one seek each).
const REV_AUDIO: f64 = 0.5;
/// While reversing the picture is decoded at most this wide: a chunk of frames is held at once.
// ponytail: fixed cap; size it from free VRAM if reverse at 4K ever looks soft
const REV_MAX_W: u32 = 1920;
/// Above this rate the sound is muted (no pitch-keeping time stretch here), as in the editor.
pub const MAX_AUDIBLE_RATE: f64 = 2.0;

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
    /// A frame went on screen since the last seek (or the start): paused, nothing more is taken.
    shown: bool,
    /// Decoding backwards: the queue runs newest first.
    reverse: bool,
    /// Seconds per frame (from the file's rate; reverse needs it to know when a frame is due).
    frame_dur: f64,
}

#[derive(Default)]
struct Audio {
    ring: VecDeque<f32>,
    /// Source time of `ring[0]`.
    head_t: f64,
    gen: u64,
    seek_to: Option<f64>,
    eof: bool,
    /// The streams mixed together; None = all of the file's (the default).
    streams: Option<Vec<usize>>,
    /// Fractional read position (source frames) into `ring`, for rates other than 1.
    frac: f64,
    /// Decoding backwards: the ring runs newest first (`head_t` is its top, time goes down along it).
    reverse: bool,
    /// Last few hundred output samples (mono), for the audio-only visualizer.
    tap: VecDeque<f32>,
}

struct Clock {
    playing: bool,
    rate: f64,
    /// Running backwards (down to 0).
    reverse: bool,
    /// Source time `t` is heard at instant `at`; `limit` caps it (the end of what is queued to play) -
    /// from below when reversing.
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
        if self.reverse {
            return (self.t - dt * self.rate).max(self.limit).max(0.0);
        }
        (self.t + dt * self.rate).min(self.limit)
    }
    fn set(&mut self, t: f64) {
        self.t = t;
        self.at = Instant::now();
        self.limit = if self.reverse { 0.0 } else { f64::INFINITY };
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
    /// The audio thread is running (no audio tracks ticked: it is not, and the wall clock runs).
    audio_running: AtomicBool,
    /// Audio Only: no video is decoded.
    audio_only: AtomicBool,
    /// Frames the video decoder delivered (tests: Audio Only decodes none).
    decoded: AtomicU64,
}

impl Shared {
    /// A picture is wanted but none is queued yet (after a seek, or reversing, before the first chunk
    /// is decoded). Locks the video: never call with the clock locked.
    fn picture_pending(&self) -> bool {
        let has_video = matches!(&*lock(&self.info), Some(Ok(i)) if i.has_video);
        let v = lock(&self.video);
        has_video && !self.audio_only.load(Ordering::Relaxed) && v.queue.is_empty() && !v.eof
    }

    /// The sound moves the clock (the device is open, the audio thread runs, not too fast to hear).
    fn audio_drives(&self, c: &Clock) -> bool {
        self.audio_clock.load(Ordering::Relaxed) && self.audio_running.load(Ordering::Relaxed) && c.rate <= MAX_AUDIBLE_RATE
    }

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
            clock: Mutex::new(Clock { playing: play, rate: 1.0, reverse: false, t: at, at: Instant::now(), limit: f64::INFINITY }),
            audio_clock: AtomicBool::new(false),
            quit: AtomicBool::new(false),
            volume: AtomicU32::new(1.0f32.to_bits()),
            muted: AtomicBool::new(false),
            dropped: AtomicU64::new(0),
            wake: Mutex::new(None),
            gpu,
            audio_running: AtomicBool::new(false),
            audio_only: AtomicBool::new(false),
            decoded: AtomicU64::new(0),
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
        let pending = self.s.picture_pending();
        let mut c = lock(&self.s.clock);
        if c.playing == on {
            return;
        }
        let t = c.now(Instant::now());
        c.playing = on;
        c.set(t);
        self.hold(&mut c, pending);
        drop(c);
        self.s.audio_cv.notify_all();
    }

    /// With the device driving the clock, time waits at `t` until the first buffer from there is
    /// queued to play (the callback re-anchors it) - the picture never runs ahead of the sound. Not
    /// too fast to hear: the wall clock runs then. Reversing, it also waits for the first chunk of
    /// pictures (`pending`): the sound waits for it too, and the video thread lets the clock go when
    /// nothing else will - so the start of a reverse shows every frame.
    fn hold(&self, c: &mut Clock, pending: bool) {
        if self.s.audio_clock.load(Ordering::Relaxed) && c.rate <= MAX_AUDIBLE_RATE || c.reverse && pending {
            c.limit = c.t;
        }
    }

    pub fn rate(&self) -> f64 {
        lock(&self.s.clock).rate
    }

    pub fn set_rate(&self, rate: f64) {
        let pending = self.s.picture_pending();
        let mut c = lock(&self.s.clock);
        let t = c.now(Instant::now());
        let was_quiet = c.rate > MAX_AUDIBLE_RATE;
        c.rate = rate.clamp(0.25, 4.0);
        c.set(t);
        self.hold(&mut c, pending);
        let quiet = c.rate > MAX_AUDIBLE_RATE;
        drop(c);
        if was_quiet != quiet {
            self.reset_audio(t); // the sound stopped (or starts) here: decode from here
        }
    }

    pub fn is_reverse(&self) -> bool {
        lock(&self.s.clock).reverse
    }

    /// Play backwards from the current time (on), or forwards again (off). The decoders restart from
    /// here in the new direction.
    pub fn set_reverse(&self, on: bool) {
        let t = {
            let mut c = lock(&self.s.clock);
            if c.reverse == on {
                return;
            }
            let t = c.now(Instant::now());
            c.reverse = on;
            t
        };
        lock(&self.s.video).reverse = on;
        lock(&self.s.audio).reverse = on;
        self.seek(t);
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
            let mut v = lock(&self.s.video);
            v.gen += 1;
            v.seek_to = Some(t);
            v.queue.clear();
            v.eof = false;
            v.shown = false;
        }
        let pending = self.s.picture_pending();
        {
            let mut c = lock(&self.s.clock);
            c.set(t);
            self.hold(&mut c, pending);
        }
        self.s.video_cv.notify_all();
        self.reset_audio(t);
    }

    /// The audio decoder starts over at `t`.
    fn reset_audio(&self, t: f64) {
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

    /// Mix these audio streams (0-based among the file's audio streams) from the current time on;
    /// None = all of them. None ticked (`Some(vec![])`): no audio is decoded, mixed or output at all -
    /// the audio thread ends, the device closes and the clock runs off the system timer.
    pub fn set_audio_streams(&self, streams: Option<Vec<usize>>) {
        let t = self.time();
        lock(&self.s.audio).streams = streams;
        self.seek(t);
        spawn_audio(&self.s);
    }

    /// The audio thread is running (decoding / playing sound).
    pub fn audio_running(&self) -> bool {
        self.s.audio_running.load(Ordering::Relaxed)
    }

    /// Audio Only: the video decoder stops (nothing decoded); off: it starts again from here.
    pub fn set_audio_only(&self, on: bool) {
        if self.s.audio_only.swap(on, Ordering::Relaxed) == on {
            return;
        }
        if on {
            lock(&self.s.video).queue.clear();
            self.s.video_cv.notify_all();
        } else {
            self.seek(self.time());
        }
    }

    pub fn audio_only(&self) -> bool {
        self.s.audio_only.load(Ordering::Relaxed)
    }

    /// Frames the video decoder has delivered so far.
    pub fn frames_decoded(&self) -> u64 {
        self.s.decoded.load(Ordering::Relaxed)
    }

    /// The streams being mixed; None = all.
    pub fn audio_streams(&self) -> Option<Vec<usize>> {
        lock(&self.s.audio).streams.clone()
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
        // forwards a frame is due once the clock reaches it; backwards (newest first) once the clock
        // has come down into it
        let (rev, d) = (v.reverse, v.frame_dur);
        while v.queue.front().is_some_and(|f| if rev { f.pts + d > t + 0.002 } else { f.pts <= t + 0.002 }) {
            if out.is_some() {
                skipped += 1;
            }
            out = v.queue.pop_front();
        }
        // paused after a seek (or at the start): the first frame at/after that point, once. Every UI
        // update calls this - a pointer move over a paused picture must not step it on a frame.
        if out.is_none() && !v.shown && !self.is_playing() {
            out = v.queue.pop_front();
        }
        v.shown |= out.is_some();
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

    /// Clock time the next queued frame is due at (None: nothing queued yet); reversing, that is the
    /// end of its frame interval.
    pub fn next_due(&self) -> Option<f64> {
        let v = lock(&self.s.video);
        v.queue.front().map(|f| if v.reverse { f.pts + v.frame_dur } else { f.pts })
    }

    /// The video ran out and everything decoded has been shown (audio: the clock passed the end).
    pub fn at_end(&self) -> bool {
        let dur = self.duration();
        let v = lock(&self.s.video);
        let video_done = v.eof && v.queue.is_empty();
        drop(v);
        let has_video = matches!(&*lock(&self.s.info), Some(Ok(i)) if i.has_video) && !self.audio_only();
        if has_video {
            video_done
        } else if self.is_reverse() {
            self.time() <= 0.0
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
/// Reversing, at once: every chunk seeks anyway.
fn take_upgrade(up: &Option<Upgrade>, pending: &mut Option<(Box<dyn VideoSource>, f64)>, s: &Shared, next_t: f64, dur: f64, reverse: bool) -> Option<Box<dyn VideoSource>> {
    if pending.is_none() {
        *pending = up.as_ref()?.try_recv().ok();
    }
    let at = pending.as_ref()?.1;
    let playing = lock(&s.clock).playing;
    let swap = !playing || reverse || next_t >= at - dur * 0.5 || next_t < at - UPGRADE_LEAD - 1.0;
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
    let fps = info.fps;
    *lock(&s.info) = Some(Ok(info.clone()));
    spawn_audio(&s);
    s.wake();
    let Some(mut src) = src else {
        lock(&s.video).eof = true;
        s.video_cv.notify_all();
        return;
    };
    crate::media::bg::raise_thread();
    // the probed rate can be off (MF reads some MKVs at half their rate): frames are asked for half a
    // frame on (a repeat of the same frame steps on again), and reversing learns the real spacing
    let mut dur = 1.0 / fps;
    lock(&s.video).frame_dur = dur;
    let mut next_t = 0.0;
    // reversing: where the chunk decoded last began (the next one ends there), and the keyframe the
    // decoder last started from (NAN: not known)
    let mut rev_hi = f64::NAN;
    let mut rev_key = f64::NAN;
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
                    rev_hi = t + dur * 0.5; // the frame showing at t is in the first chunk
                    rev_key = f64::NAN;
                }
                // Audio Only: nothing is decoded until it is switched off (a seek wakes this).
                // Reversing, the next chunk is decoded while fewer than two are queued.
                let ahead = if v.reverse { 2 * REV_FRAMES } else { QUEUE };
                if !v.eof && v.queue.len() < ahead && !s.audio_only.load(Ordering::Relaxed) {
                    break;
                }
                v = s.video_cv.wait(v).unwrap_or_else(|e| e.into_inner());
            }
            (v.gen, v.want)
        };
        let reverse = lock(&s.video).reverse;
        if let Some(v) = take_upgrade(&up, &mut pending, &s, next_t, dur, reverse) {
            src = v;
            up = None;
            if let Some(Ok(i)) = lock(&s.info).as_mut() {
                i.decoder = "Hardware, zero-copy";
            }
            crate::perf::startup_mark("player: on zero-copy");
        }
        if reverse {
            rev_hi = reverse_chunk(&s, src.as_mut(), gen, want, info.size, rev_hi, &mut rev_key, &mut dur, &mut spare);
            continue;
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
                next_t = f.pts + dur * 0.5;
                let first = v.queue.is_empty();
                if !marked {
                    marked = true;
                    crate::perf::startup_mark("player: frame decoded");
                }
                v.queue.push_back(f);
                drop(v);
                s.decoded.fetch_add(1, Ordering::Relaxed);
                s.video_cv.notify_all();
                if first {
                    s.wake();
                }
            }
        }
    }
}

/// Where the reverse chunk ending at `hi` starts: the keyframe `key` when it is at most `REV_FRAMES`
/// back (decoding starts right on it, no frame wasted), else `REV_FRAMES` back (the seek decodes from
/// the keyframe before that, the frames ahead of the chunk thrown away).
fn reverse_chunk_start(hi: f64, key: f64, dur: f64) -> f64 {
    let lo = (hi - REV_FRAMES as f64 * dur).max(0.0);
    if key.is_finite() && key < hi - dur * 0.5 && key >= lo - 1e-6 {
        key
    } else {
        lo
    }
}

/// Reversing: decode the chunk before `hi` (`reverse_chunk_start`) forwards and queue it newest first.
/// Returns where the next chunk ends (this one's first frame); at the start of the file, marks the end.
#[allow(clippy::too_many_arguments)]
fn reverse_chunk(s: &Shared, src: &mut dyn VideoSource, gen: u64, want: (u32, u32), native: (u32, u32), hi: f64, key: &mut f64, dur: &mut f64, spare: &mut Vec<Frame>) -> f64 {
    // never above the source's own size (the picture is scaled up where it is drawn): a chunk is held
    let (mut w, mut h) = if want.0 > 0 && want.1 > 0 && want.0 < native.0 { want } else { native };
    if w > REV_MAX_W {
        h = ((h as u64 * REV_MAX_W as u64 / w as u64) as u32).max(2) & !1;
        w = REV_MAX_W;
    }
    let lo = reverse_chunk_start(hi, *key, *dur);
    let mut frames: Vec<Arc<Frame>> = Vec::new();
    // A seek to a keyframe's own time lands MF on the keyframe before it (by the B-frame reorder
    // delay: measured ~2 frames on a 1080p60 H.264 mp4) - a whole GOP decoded for nothing. Starting on
    // a keyframe, ask for a few frames past it (not at 0: no chunk follows); the chunk after this one (decoding from the keyframe
    // before anyway) runs on through those frames. Else a quarter frame in: the frame at `lo`.
    let mut t = if lo == *key && lo > 0.0 { lo + *dur * 3.0 } else { lo + *dur * 0.25 };
    while t < hi && hi > 0.0 {
        if lock(&s.video).gen != gen || s.quit.load(Ordering::Relaxed) {
            return hi; // a seek or a direction change: the video thread starts over
        }
        let Some(f) = src.frame_gpu(t, w, h).or_else(|| src.frame_shared(t, w, h, spare)) else { break };
        if frames.is_empty() {
            // the decoder seeked here and started on this keyframe; at the chunk start: unknown again
            *key = src.seek_landed().filter(|k| *k < lo - *dur * 0.5).unwrap_or(f64::NAN);
        }
        if f.pts >= hi - 1e-6 {
            break;
        }
        t = f.pts.max(t) + *dur * 0.5;
        if frames.last().is_none_or(|l| f.pts > l.pts + 1e-6) {
            frames.push(f);
        }
    }
    let mut v = lock(&s.video);
    if v.gen != gen {
        return hi;
    }
    // frames closer than the probed rate says: that is the real spacing (when a frame is due)
    if let Some(d) = frames.windows(2).map(|w| w[1].pts - w[0].pts).min_by(f64::total_cmp).filter(|d| *d < *dur * 0.9) {
        *dur = d;
        v.frame_dur = d;
    }
    let first = frames.first().map(|f| f.pts);
    let was_empty = v.queue.is_empty();
    v.queue.extend(frames.into_iter().rev());
    if lo <= 0.0 || first.is_none() {
        v.eof = true; // this chunk reached the start of the file
    }
    drop(v);
    s.video_cv.notify_all();
    if was_empty {
        // the first pictures are in: the sound starts (and moves the clock), or else the clock goes now
        s.audio_cv.notify_all();
        let mut c = lock(&s.clock);
        if !s.audio_drives(&c) && c.limit > 0.0 && c.limit >= c.t {
            let t = c.t;
            c.set(t);
        }
        drop(c);
        s.wake();
    }
    first.unwrap_or(0.0)
}

/// Start the audio thread unless it runs, the file has no audio, or no track is ticked.
fn spawn_audio(s: &Arc<Shared>) {
    let has_audio = matches!(&*lock(&s.info), Some(Ok(i)) if !i.audio_streams.is_empty());
    let none = lock(&s.audio).streams.as_ref().is_some_and(|v| v.is_empty());
    if !has_audio || none || s.quit.load(Ordering::Relaxed) {
        return;
    }
    if s.audio_running.compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed).is_err() {
        return;
    }
    let a = s.clone();
    let _ = std::thread::Builder::new().name("player-audio".into()).spawn(move || audio_thread(a));
}

/// The audio thread ended: the wall clock takes over from where the sound was.
struct AudioGone(Arc<Shared>);
impl Drop for AudioGone {
    fn drop(&mut self) {
        self.0.audio_clock.store(false, Ordering::Relaxed);
        let mut c = lock(&self.0.clock);
        let t = c.now(Instant::now());
        c.set(t);
        drop(c);
        self.0.audio_running.store(false, Ordering::Release);
        spawn_audio(&self.0); // a track was ticked again while this one was ending
    }
}

fn audio_thread(s: Arc<Shared>) {
    use cpal::traits::StreamTrait;
    let _gone = AudioGone(s.clone());
    crate::media::mf::init_thread();
    let p = s.path.to_string_lossy().into_owned();
    let stream = open_output(s.clone());
    s.audio_clock.store(stream.is_some(), Ordering::Relaxed);
    let mut stream_playing = false;
    // the open sources, one per stream mixed
    let mut srcs: Vec<(usize, Box<dyn AudioSource>)> = Vec::new();
    let mut opened: Option<Vec<usize>> = None;
    let mut mixed = Vec::new();
    let n_streams = match &*lock(&s.info) {
        Some(Ok(i)) => i.audio_streams.len().max(1),
        _ => 1,
    };
    let mut next_t = 0.0;
    let mut gen = u64::MAX;
    let mut block = Vec::new();
    let ahead = (RING_SECS * SAMPLE_RATE as f64) as usize * 2;
    loop {
        if s.quit.load(Ordering::Relaxed) {
            return;
        }
        let (playing, quiet, rev) = {
            let c = lock(&s.clock);
            (c.playing, c.rate > MAX_AUDIBLE_RATE, c.reverse)
        };
        // reversing, the sound starts with the first chunk of pictures
        let wait_picture = rev && s.picture_pending();
        if let Some(st) = &stream {
            // a paused player keeps no audio device busy
            if playing != stream_playing {
                stream_playing = playing;
                let _ = if playing { st.play() } else { st.pause() };
            }
        }
        let (want, rev) = {
            let mut a = lock(&s.audio);
            if let Some(t) = a.seek_to.take() {
                next_t = t;
                gen = a.gen;
            }
            // too fast to hear: nothing to decode (the callback plays silence)
            // reversing, a whole chunk more: one is decoded at a time, so it must not run dry meanwhile
            let ahead = if a.reverse { ahead + (REV_AUDIO * SAMPLE_RATE as f64) as usize * 2 } else { ahead };
            if a.ring.len() >= ahead || a.eof || quiet || wait_picture {
                let _ = s.audio_cv.wait_timeout(a, Duration::from_millis(100));
                continue;
            }
            (a.streams.clone().unwrap_or_else(|| (0..n_streams).collect()), a.reverse)
        };
        if want.is_empty() {
            return; // no track ticked: no decode, no device (`AudioGone` hands the clock back)
        }
        if opened.as_ref() != Some(&want) {
            srcs = want.iter().filter_map(|&n| media::open_audio(&p, n, Backend::Auto).ok().map(|a| (n, a))).collect();
            opened = Some(want);
        }
        // forwards a block from `next_t`; backwards the `REV_AUDIO` before it (one seek per chunk)
        let n = if rev { (REV_AUDIO.min(next_t) * SAMPLE_RATE as f64).round() as usize } else { BLOCK };
        let t0 = if rev { next_t - n as f64 / SAMPLE_RATE as f64 } else { next_t };
        mixed.clear();
        mixed.resize(n * 2, 0.0);
        block.resize(n * 2, 0.0);
        // the ticked streams, summed (none ticked: silence, and time still runs)
        let mut duration = 0.0f64;
        for (_, a_src) in srcs.iter_mut() {
            duration = duration.max(a_src.duration());
            a_src.read_at(t0, &mut block);
            for (m, x) in mixed.iter_mut().zip(&block) {
                *m += x;
            }
        }
        if srcs.is_empty() {
            duration = lock(&s.info).as_ref().and_then(|i| i.as_ref().ok()).map_or(0.0, |i| i.duration);
        }
        let mut a = lock(&s.audio);
        if a.gen != gen {
            continue;
        }
        if rev {
            // time-mirrored: the ring runs newest first, so the device plays it backwards
            a.ring.extend(mirrored(&mixed));
            next_t = t0;
            a.eof = t0 <= 0.0;
            continue;
        }
        a.ring.extend(mixed.iter().copied());
        next_t += BLOCK as f64 / SAMPLE_RATE as f64;
        if duration > 0.0 && next_t >= duration {
            a.eof = true;
        }
    }
}

/// Interleaved stereo `x`, last frame first.
fn mirrored(x: &[f32]) -> impl Iterator<Item = f32> + '_ {
    x.chunks_exact(2).rev().flatten().copied()
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
    let (playing, rate, quiet) = {
        let c = lock(&s.clock);
        (c.playing, c.rate, c.rate > MAX_AUDIBLE_RATE)
    };
    let gain = if s.muted.load(Ordering::Relaxed) { 0.0 } else { f32::from_bits(s.volume.load(Ordering::Relaxed)) };
    let mut a = lock(&s.audio);
    let frames = out.len() / 2;
    if !playing || quiet {
        out.fill(0.0);
        return;
    }
    // reversing, time runs down along the ring
    let dir = if a.reverse { -1.0 } else { 1.0 };
    let start_t = a.head_t + dir * a.frac / SAMPLE_RATE as f64;
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
            // several tracks summed can pass full scale
            out[i * 2 + c] = ((x0 + (x1 - x0) * k) * gain).clamp(-1.0, 1.0);
        }
        a.frac += rate;
        played += 1;
    }
    let consumed = (a.frac.floor() as usize).min(a.ring.len() / 2);
    a.ring.drain(..consumed * 2);
    a.frac -= consumed as f64;
    a.head_t += dir * consumed as f64 / SAMPLE_RATE as f64;
    let end_t = a.head_t + dir * a.frac / SAMPLE_RATE as f64;
    let tap = out.chunks_exact(2).map(|c| (c[0] + c[1]) * 0.5).collect::<Vec<_>>();
    a.tap.extend(tap);
    let over = a.tap.len().saturating_sub(1024);
    a.tap.drain(..over);
    drop(a);
    s.audio_cv.notify_all();
    if s.audio_clock.load(Ordering::Relaxed) && played > 0 {
        let mut c = lock(&s.clock);
        if c.playing {
            if c.limit == c.t {
                // the first buffer from here: time waits until it is heard
                c.t = start_t;
                c.at = heard_at;
            } else {
                // what is being heard right now: `latency` before this buffer. (Anchoring at `heard_at`
                // stood the clock still until then - with the latency longer than a device period, time
                // moved in period-sized steps, a frame skipped at 2x.) Never backwards.
                let now = Instant::now();
                let cur = c.now(now);
                let heard = start_t - dir * latency.as_secs_f64() * rate;
                c.t = if dir > 0.0 { heard.max(cur) } else { heard.min(cur) };
                c.at = now;
            }
            c.limit = end_t;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Amplitude of the `f` Hz component of interleaved stereo `x` (Goertzel on the mono mix).
    fn tone(x: &[f32], f: f32) -> f32 {
        let mono: Vec<f32> = x.chunks_exact(2).map(|c| (c[0] + c[1]) * 0.5).collect();
        let w = std::f32::consts::TAU * f / SAMPLE_RATE as f32;
        let (mut s1, mut s2) = (0.0f32, 0.0f32);
        for v in &mono {
            let s0 = v + 2.0 * w.cos() * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        let power = s1 * s1 + s2 * s2 - 2.0 * w.cos() * s1 * s2;
        2.0 * power.max(0.0).sqrt() / mono.len() as f32
    }

    /// The audio heard is exactly the ticked streams mixed: the test file's stream 0 is a 440 Hz tone,
    /// stream 1 an 880 Hz one.
    #[test]
    fn the_audio_selection_mixes_exactly_the_ticked_streams() {
        let Some(f) = crate::player::tracks::tests::two_audio_one_sub() else { return };
        let ring = |sel: Option<Vec<usize>>| -> Vec<f32> {
            let e = Engine::start(&f, 1.0, false, false);
            if sel.is_some() {
                e.set_audio_streams(sel);
            }
            // paused, the audio thread still decodes its ring ahead once
            let end = Instant::now() + Duration::from_secs(8);
            while lock(&e.s.audio).ring.len() < 16384 && Instant::now() < end {
                std::thread::sleep(Duration::from_millis(10));
            }
            let x: Vec<f32> = lock(&e.s.audio).ring.iter().copied().take(16384).collect();
            x
        };
        for (sel, a440, a880) in [(None, true, true), (Some(vec![0]), true, false), (Some(vec![1]), false, true), (Some(vec![0, 1]), true, true)] {
            let x = ring(sel.clone());
            assert_eq!(x.len(), 16384, "{sel:?}: no audio decoded");
            let (m440, m880) = (tone(&x, 440.0), tone(&x, 880.0));
            assert_eq!(m440 > 0.03, a440, "{sel:?}: 440 Hz at {m440}");
            assert_eq!(m880 > 0.03, a880, "{sel:?}: 880 Hz at {m880}");
        }
    }

    /// No track ticked: no audio thread, no decode, no device - and the clock still runs (system timer).
    /// Ticking one again brings the sound back.
    #[test]
    fn no_audio_tracks_decode_and_output_nothing() {
        let Some(f) = crate::player::tracks::tests::two_audio_one_sub() else { return };
        let e = Engine::start(&f, 1.0, false, false);
        e.set_audio_streams(Some(vec![]));
        let end = Instant::now() + Duration::from_secs(8);
        while e.info().is_none() && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(10));
        }
        std::thread::sleep(Duration::from_millis(300));
        assert!(!e.audio_running(), "no audio thread with no track ticked");
        assert!(lock(&e.s.audio).ring.is_empty(), "nothing decoded");
        assert!(!e.s.audio_clock.load(Ordering::Relaxed), "the device does not drive the clock");
        e.set_playing(true);
        std::thread::sleep(Duration::from_millis(300));
        assert!(e.time() > 1.2, "the clock runs on the system timer: {}", e.time());
        e.set_audio_streams(Some(vec![1]));
        let end = Instant::now() + Duration::from_secs(8);
        while lock(&e.s.audio).ring.is_empty() && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(e.audio_running() && !lock(&e.s.audio).ring.is_empty(), "ticking a track starts the sound again");
    }

    /// Audio Only: the video decoder delivers nothing (and the sound still decodes); off again, the
    /// picture comes back.
    #[test]
    fn audio_only_decodes_no_video() {
        let Some(f) = crate::player::tracks::tests::two_audio_one_sub() else { return };
        let e = Engine::start(&f, 0.0, false, false);
        e.set_audio_only(true);
        e.set_playing(true);
        let end = Instant::now() + Duration::from_secs(8);
        while lock(&e.s.audio).ring.is_empty() && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(10));
        }
        std::thread::sleep(Duration::from_millis(400));
        // a frame may have been in flight when it was switched on: none after that
        assert!(e.frames_decoded() <= 1, "{} frames decoded in Audio Only", e.frames_decoded());
        assert!(!lock(&e.s.audio).ring.is_empty(), "the sound plays");
        let before = e.frames_decoded();
        e.set_audio_only(false);
        let end = Instant::now() + Duration::from_secs(8);
        while e.frames_decoded() == before && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(e.frames_decoded() > before, "the picture decodes again");
    }

    /// Reversing, the clock runs down at the rate (1x and 2x), holds where the sound has not been
    /// queued yet, and stops at 0.
    #[test]
    fn reverse_clock_runs_down_at_speed() {
        let at = Instant::now();
        let mut c = Clock { playing: true, rate: 1.0, reverse: true, t: 0.0, at, limit: 0.0 };
        c.set(4.0);
        let at = c.at;
        assert_eq!(c.limit, 0.0, "reversing, the floor is the start of the file");
        assert!((c.now(at + Duration::from_millis(500)) - 3.5).abs() < 1e-9);
        c.rate = 2.0;
        assert!((c.now(at + Duration::from_millis(500)) - 3.0).abs() < 1e-9);
        assert_eq!(c.now(at + Duration::from_secs(10)), 0.0, "never below the start");
        c.limit = 3.6;
        assert_eq!(c.now(at + Duration::from_secs(1)), 3.6, "never runs past what is queued to be heard");
    }

    /// Waits up to 8 s for `f`.
    fn until(mut f: impl FnMut() -> bool) -> bool {
        let end = Instant::now() + Duration::from_secs(8);
        while !f() {
            if Instant::now() > end {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        true
    }

    /// The engine's clock in reverse, with the sound driving it: 1x and 2x backwards in real time.
    #[test]
    fn reverse_plays_backwards_at_the_speed() {
        let Some(f) = crate::player::tracks::tests::two_audio_one_sub() else { return };
        let e = Engine::start(&f, 3.8, false, false);
        assert!(until(|| e.info().is_some()));
        e.set_reverse(true);
        e.set_playing(true);
        for rate in [1.0, 2.0] {
            e.set_rate(rate);
            std::thread::sleep(Duration::from_millis(100)); // the sound starts (the clock waits for it)
            let (t0, w0) = (e.time(), Instant::now());
            std::thread::sleep(Duration::from_millis(600));
            let ran = (t0 - e.time()) / w0.elapsed().as_secs_f64();
            assert!((ran - rate).abs() < 0.2 * rate, "{rate}x backwards ran at {ran}x");
        }
    }

    /// Reversing, every frame comes out once, newest first, with no gap where one chunk meets the next.
    #[test]
    fn reverse_frames_descend_without_gaps_across_chunks() {
        let Some(f) = crate::player::tracks::tests::two_audio_one_sub() else { return };
        let e = Engine::start(&f, 3.5, false, false);
        assert!(until(|| e.info().is_some()));
        e.set_reverse(true);
        let mut pts = Vec::new();
        loop {
            assert!(until(|| {
                let v = lock(&e.s.video);
                !v.queue.is_empty() || v.eof
            }));
            let mut v = lock(&e.s.video);
            let eof = v.eof;
            pts.extend(v.queue.drain(..).map(|f| f.pts));
            drop(v);
            e.s.video_cv.notify_all();
            if eof {
                break;
            }
        }
        let fd = 1.0 / 30.0;
        assert!(pts.len() > 2 * REV_FRAMES, "{} frames: crosses chunks: {pts:?} fps {:?}", pts.len(), e.info().unwrap().unwrap().fps);
        assert!((pts[0] - 3.5).abs() < fd, "starts at the playhead: {}", pts[0]);
        assert!(*pts.last().unwrap() < 1e-3, "ends on the first frame: {:?}", pts.last());
        for w in pts.windows(2) {
            assert!(w[1] < w[0] && w[0] - w[1] < fd * 1.5, "gap or step {} -> {}: {pts:?}", w[0], w[1]);
        }
    }

    /// Reversing, the sound in the ring is the source mirrored in time, sample for sample - also where
    /// one decoded chunk meets the next.
    #[test]
    fn reversed_audio_is_the_time_mirrored_source() {
        let Some(f) = crate::player::tracks::tests::two_audio_one_sub() else { return };
        let e = Engine::start(&f, 3.0, false, false);
        e.set_audio_streams(Some(vec![0]));
        e.set_reverse(true);
        let want = (1.2 * SAMPLE_RATE as f64) as usize * 2; // over two chunks
        let mut got = Vec::new();
        while got.len() < want {
            assert!(until(|| !lock(&e.s.audio).ring.is_empty()), "no audio decoded");
            got.extend(lock(&e.s.audio).ring.drain(..));
            e.s.audio_cv.notify_all();
        }
        got.truncate(want);
        let mut src = media::open_audio(&f.to_string_lossy(), 0, Backend::Auto).expect("audio");
        let mut fwd = vec![0f32; want];
        src.read_at(3.0 - 1.2, &mut fwd);
        let mirror: Vec<f32> = mirrored(&fwd).collect();
        assert!(tone(&got, 440.0) > 0.03, "the tone is there");
        let worst = got.iter().zip(&mirror).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        assert!(worst < 2e-3, "differs from the mirrored source by up to {worst}");
    }

    /// Reverse on a real file, zero-copy as the player runs it: 10 s at 1x and at 2x, frames taken as
    /// they fall due (polled every ms). Prints frames shown, dropped (skipped unseen), frame-interval
    /// stats and late frames. `SE_REVERSE_BENCH=<file> cargo test bench_reverse -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bench_reverse() {
        let Ok(path) = std::env::var("SE_REVERSE_BENCH") else { return };
        let e = Engine::start(Path::new(&path), 25.0, false, true);
        e.wait_first(Duration::from_secs(10));
        let info = e.info().expect("opened").expect("opened");
        e.set_output_size(1280, 720);
        let fd = 1.0 / info.fps;
        println!("{path}: {}x{} @ {} fps, {}", info.size.0, info.size.1, info.fps, info.decoder);
        for rate in [1.0, 2.0] {
            e.set_playing(false);
            e.set_reverse(true);
            e.seek(25.0);
            e.set_rate(rate);
            let d0 = e.dropped();
            e.set_playing(true);
            let start = Instant::now();
            let mut shown: Vec<(f64, f64)> = Vec::new(); // (wall s, pts)
            let mut late = 0;
            let mut starved = 0; // polls with nothing decoded ahead (the decoder fell behind)
            let mut was_empty = false;
            while start.elapsed() < Duration::from_secs(10) {
                let t = e.time();
                let empty = !shown.is_empty() && e.next_due().is_none();
                starved += (empty && !was_empty) as u32;
                was_empty = empty;
                if let Some(f) = e.frame_due(t) {
                    if (f.pts - t).abs() > fd * 1.5 {
                        late += 1;
                    }
                    shown.push((start.elapsed().as_secs_f64(), f.pts));
                }
                std::thread::yield_now(); // a sleep's ~1-2 ms granularity would itself skip frames at 2x
            }
            let gaps: Vec<f64> = shown.windows(2).map(|w| (w[1].0 - w[0].0) * 1000.0).collect();
            let mut sorted = gaps.clone();
            sorted.sort_by(f64::total_cmp);
            let pct = |p: f64| sorted.get(((sorted.len() as f64 - 1.0) * p) as usize).copied().unwrap_or(0.0);
            let steps_fwd = shown.windows(2).filter(|w| w[1].1 > w[0].1).count();
            let first = shown.first().map_or(0.0, |s| s.0 * 1000.0);
            println!(
                "reverse {rate}x: {} frames shown in 10 s (expected {:.0}), {} dropped, {late} late, {starved} times run dry, {steps_fwd} forward steps; \
                 first frame after {first:.0} ms; interval mean {:.2} ms, p50 {:.2}, p99 {:.2}, max {:.2} (frame = {:.2} ms); clock {:.2} -> {:.2}",
                shown.len(),
                10.0 * info.fps * rate,
                e.dropped() - d0,
                gaps.iter().sum::<f64>() / gaps.len().max(1) as f64,
                pct(0.5),
                pct(0.99),
                pct(1.0),
                fd * 1000.0 / rate,
                25.0,
                e.time()
            );
        }
    }

    /// Reversing, the sound plays (reversed), so the Audio Only visualizer, fed from what the device
    /// plays, keeps moving: its input is the tone, not silence, and changes over time.
    #[test]
    fn the_visualizer_input_moves_in_reverse() {
        let Some(f) = crate::player::tracks::tests::two_audio_one_sub() else { return };
        let e = Engine::start(&f, 3.5, false, false);
        e.set_audio_streams(Some(vec![0]));
        e.set_reverse(true);
        e.set_playing(true);
        std::thread::sleep(Duration::from_millis(500));
        if !e.s.audio_clock.load(Ordering::Relaxed) {
            return; // no output device on this machine
        }
        let a = e.tap();
        std::thread::sleep(Duration::from_millis(200));
        let b = e.tap();
        let rms = |x: &[f32]| (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt();
        assert!(rms(&a) > 0.02 && rms(&b) > 0.02, "the visualizer sees silence: {} {}", rms(&a), rms(&b));
        assert_ne!(a, b, "the visualizer input is frozen");
    }

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
        let mut c = Clock { playing: false, rate: 1.0, reverse: false, t: 4.0, at, limit: f64::INFINITY };
        assert_eq!(c.now(at + Duration::from_secs(3)), 4.0);
        c.playing = true;
        c.rate = 2.0;
        assert!((c.now(at + Duration::from_millis(500)) - 5.0).abs() < 1e-9);
        c.limit = 4.2;
        assert_eq!(c.now(at + Duration::from_secs(1)), 4.2, "never runs past what is queued to be heard");
    }
}
