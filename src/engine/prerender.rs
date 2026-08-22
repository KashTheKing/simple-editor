//! Pre-render ("movie mode"): render ranges at full quality into a cache so playback shows exactly what
//! an export would, without re-running the effect chain every frame.
//!
//! Cache: `Settings::cache_dir()/prerender/<hash>.rgba`, one file per second of timeline at project
//! resolution, keyed by a hash of everything affecting the picture in that second. A worker thread
//! renders whole seconds with the CPU compositor; the UI thread only hands out seconds and collects
//! finished ones, so a slow frame lands on the worker and never on a repaint.
//!
//! File layout: `SEPR` + version + width + height + frame count (u32 LE each), then that many
//! `width * height * 4` RGBA frames. `frame()` seeks to the one it needs, so a served frame costs one
//! read and never a whole second of RAM.
//!
//! ponytail: rendering runs on `engine::compose::Compositor` — the GL renderer is UI-thread-only and a
//! worker cannot touch it. Movie mode wanting the GPU shaders means routing layers back to the UI thread
//! the way `export::GpuScratch` does (the keys and files do not move).

use crate::engine::compose::Compositor;
use crate::engine::text::TextRasterizer;
use crate::media::{Backend, DecoderPool, Frame};
use crate::model::{ClipKind, Project, TrackKind};
use crate::settings::Settings;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

const MAGIC: &[u8; 4] = b"SEPR";
const VERSION: u32 = 1;
const HEADER: u64 = 4 + 4 * 4;
/// Stop growing the cache past this (raw RGBA is big); the oldest files go first.
const CACHE_BUDGET: u64 = 8 << 30;

#[derive(Default)]
pub struct PreRender {
    /// Requested ranges, seconds, merged and clamped to whole seconds.
    ranges: Vec<(f64, f64)>,
    /// Whole seconds still to render, in order. A second stays here while a worker has it.
    queue: Vec<i64>,
    /// Seconds rendered this session with the key they were rendered under.
    done: Vec<(i64, u64)>,
    /// Seconds invalidated since they were rendered (never served, always re-rendered).
    dirty: Vec<i64>,
    /// Seconds handed to the worker, with the key each is rendering under.
    inflight: Vec<(i64, u64)>,
    /// Bumped by every invalidation; a job behind it is dropped mid-second.
    generation: Arc<AtomicU64>,
    /// Spawned on the first queued second, dropped when the queue drains.
    worker: Option<Worker>,
}

/// Seconds handed over at once: one rendering plus one waiting, so the worker never idles between
/// frames of the UI but an edit still gets picked up within a second of work.
const DEPTH: usize = 2;

/// The render worker. Dropping it closes the job channel, so the thread finishes its second and exits.
struct Worker {
    jobs: Sender<Job>,
    results: Receiver<(i64, u64, Res)>,
}

/// One second of timeline for one worker, against a snapshot of the project it was queued from.
struct Job {
    project: Arc<Project>,
    sec: i64,
    key: u64,
    w: u32,
    h: u32,
    n: u32,
    fps: f64,
    generation: u64,
}

/// How a worker's second ended.
enum Res {
    /// Written and published.
    Done,
    /// An edit overtook it: still queued, re-rendered under the new key.
    Stale,
    /// Out of disk / no cache dir: give up on pre-rendering.
    Failed,
}

/// Per-worker scratch: a compositor with its own decoders and font cache.
struct Work {
    comp: Compositor,
    pool: DecoderPool,
    text: TextRasterizer,
    frame: Frame,
}

/// A half-written second; dropping it without `finish()` throws the temp file away.
struct Open {
    tmp: PathBuf,
    path: PathBuf,
    f: Option<std::io::BufWriter<std::fs::File>>,
}

impl Work {
    fn new() -> Self {
        Work {
            comp: Compositor::new(),
            pool: DecoderPool::new(Backend::Auto),
            text: TextRasterizer::new(),
            frame: Frame::default(),
        }
    }

    /// Render one whole second into the cache, streaming frames out as they are made (a second of 1080p
    /// RGBA is 250 MB, far too much to buffer). Decoding stays sequential, so the decoder never re-seeks.
    fn render_sec(&mut self, job: &Job, generation: &AtomicU64) -> Res {
        let Ok(mut o) = open_sec(job.key, job.w, job.h, job.n) else { return Res::Failed };
        for i in 0..job.n {
            if generation.load(Ordering::Relaxed) != job.generation {
                return Res::Stale; // an edit landed: dropping `o` deletes the half-written file
            }
            let t = job.sec as f64 + i as f64 / job.fps;
            self.comp.render(&job.project, t, job.w, job.h, &mut self.pool, &mut self.text, &mut self.frame);
            if !o.write(&self.frame.rgba) {
                return Res::Failed;
            }
        }
        if o.finish() {
            Res::Done
        } else {
            Res::Failed
        }
    }
}

impl Worker {
    fn new(generation: Arc<AtomicU64>) -> Worker {
        let (jobs, rx) = channel::<Job>();
        let (tx, results) = channel();
        // ponytail: one thread, measured, not guessed. A pool renders seconds concurrently, which means
        // one decoder per worker seeking into the same file: on 1080p H.264 two Media Foundation readers
        // bought nothing and four were 6x SLOWER than one (they each spawn a core's worth of internal
        // threads). Split by *source file* if a multi-track project ever needs the cores.
        std::thread::spawn(move || {
            let mut work = Work::new();
            while let Ok(job) = rx.recv() {
                let r = work.render_sec(&job, &generation);
                if tx.send((job.sec, job.key, r)).is_err() {
                    return;
                }
            }
        });
        Worker { jobs, results }
    }
}

impl Open {
    fn write(&mut self, b: &[u8]) -> bool {
        self.f.as_mut().is_some_and(|f| f.write_all(b).is_ok())
    }

    /// Flush the second and publish it (rename over the real name).
    fn finish(mut self) -> bool {
        let Some(f) = self.f.take() else { return false };
        let ok = f.into_inner().map_err(|e| e.into_error()).and_then(|_| std::fs::rename(&self.tmp, &self.path));
        if ok.is_err() {
            let _ = std::fs::remove_file(&self.tmp);
        }
        ok.is_ok()
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        if self.f.take().is_some() {
            let _ = std::fs::remove_file(&self.tmp);
        }
    }
}

/// Create the temp file for one second and write its header.
fn open_sec(key: u64, w: u32, h: u32, n: u32) -> std::io::Result<Open> {
    let path = path_for(key);
    let tmp = path.with_extension("tmp");
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let mut f = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
    f.write_all(MAGIC)?;
    for v in [VERSION, w, h, n] {
        f.write_all(&v.to_le_bytes())?;
    }
    Ok(Open { tmp, path, f: Some(f) })
}

impl PreRender {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark a range dirty (an edit touched it).
    pub fn invalidate(&mut self, from: f64, to: f64) {
        // unconditional: the app invalidates the whole timeline per edit, so anything in flight is dead
        self.generation.fetch_add(1, Ordering::Relaxed);
        let (a, b) = (from.min(to), from.max(to));
        for s in sec_range(a, b) {
            self.done.retain(|(x, _)| *x != s);
            if !self.dirty.contains(&s) {
                self.dirty.push(s);
            }
            // a dirty second inside a requested range goes back on the queue
            if self.ranges.iter().any(|(x, y)| (s as f64) < *y && (s + 1) as f64 > *x) && !self.queue.contains(&s) {
                self.queue.push(s);
            }
        }
        self.queue.sort_unstable();
    }

    /// Queue [a, b) for rendering.
    pub fn request(&mut self, project: &Project, a: f64, b: f64) {
        let (a, b) = (a.min(b).max(0.0), b.max(a));
        if !(b > a) {
            return;
        }
        // merged, not appended: `after_edit` re-requests the whole timeline on every single edit, and an
        // unmerged list makes `segments()` and `progress()` — both drawn every frame — grow without end
        self.ranges.push((a, b));
        self.ranges.sort_by(|x, y| x.0.total_cmp(&y.0));
        let mut merged: Vec<(f64, f64)> = Vec::with_capacity(self.ranges.len());
        for (x, y) in self.ranges.drain(..) {
            match merged.last_mut() {
                Some(l) if x <= l.1 => l.1 = l.1.max(y),
                _ => merged.push((x, y)),
            }
        }
        self.ranges = merged;
        for s in sec_range(a, b) {
            let key = key_for(project, s);
            if self.done.contains(&(s, key)) || self.queue.contains(&s) {
                continue;
            }
            if path_for(key).exists() {
                self.done.push((s, key));
                self.dirty.retain(|d| *d != s);
                continue;
            }
            self.queue.push(s);
        }
        self.queue.sort_unstable();
    }

    /// Collect finished seconds and keep the worker fed (call once per frame). Returns true while work
    /// is outstanding. The budget is ignored: rendering left the UI thread, so there is nothing to slice.
    pub fn tick(&mut self, project: &Project, _budget_ms: f32) -> bool {
        if self.queue.is_empty() {
            self.worker = None; // idle: closing the job channel lets the thread exit
            self.inflight.clear();
            return false;
        }
        let (w, h) = (project.width.max(1), project.height.max(1));
        let fps = if project.fps > 1.0 { project.fps } else { 30.0 };
        let n = fps.round().max(1.0) as u32;
        let generation = self.generation.clone();
        let worker = self.worker.take().unwrap_or_else(|| Worker::new(generation.clone()));
        let PreRender { queue, done, dirty, inflight, .. } = self;
        let mut landed = false;
        while let Ok((sec, key, r)) = worker.results.try_recv() {
            inflight.retain(|(s, _)| *s != sec);
            match r {
                // out of disk / no cache dir: stop trying, playback falls back to live render
                Res::Failed => {
                    queue.clear();
                    inflight.clear();
                    return false;
                }
                Res::Stale => {} // still queued, re-dispatched below under the new key
                Res::Done => {
                    queue.retain(|s| *s != sec);
                    dirty.retain(|d| *d != sec);
                    if !done.contains(&(sec, key)) {
                        done.push((sec, key));
                    }
                    landed = true;
                }
            }
        }
        if landed {
            // an evicted second is no longer ready: drop it so the next request re-renders it
            let evicted = prune(&dir(), CACHE_BUDGET);
            done.retain(|(_, k)| !evicted.contains(k));
        }
        // keep the worker fed, off a snapshot of the project as it is right now
        let g = generation.load(Ordering::Relaxed);
        let mut snap: Option<Arc<Project>> = None;
        while inflight.len() < DEPTH {
            let Some(sec) = queue.iter().copied().find(|s| !inflight.iter().any(|(x, _)| x == s)) else { break };
            let key = key_for(project, sec);
            if path_for(key).exists() {
                queue.retain(|s| *s != sec);
                dirty.retain(|d| *d != sec);
                if !done.contains(&(sec, key)) {
                    done.push((sec, key));
                }
                continue;
            }
            let p = snap.get_or_insert_with(|| Arc::new(project.clone())).clone();
            if worker.jobs.send(Job { project: p, sec, key, w, h, n, fps, generation: g }).is_err() {
                queue.clear();
                return false;
            }
            inflight.push((sec, key));
        }
        self.worker = Some(worker);
        !self.queue.is_empty()
    }

    /// A pre-rendered frame for time t, when the cache holds a valid one.
    pub fn frame(&self, project: &Project, t: f64) -> Option<Arc<Frame>> {
        if t < 0.0 {
            return None;
        }
        let sec = t.floor() as i64;
        if self.dirty.contains(&sec) {
            return None;
        }
        let key = key_for(project, sec);
        if !self.done.contains(&(sec, key)) {
            return None;
        }
        let fps = if project.fps > 1.0 { project.fps } else { 30.0 };
        let i = ((t - sec as f64) * fps).floor().max(0.0) as u32;
        read_frame(&path_for(key), i, t).map(Arc::new)
    }

    /// Fraction of the requested range that is ready.
    pub fn progress(&self) -> f32 {
        let total: i64 = self.ranges.iter().map(|(a, b)| sec_range(*a, *b).count() as i64).sum();
        if total <= 0 {
            return 1.0;
        }
        let left = self.queue.len() as f32;
        (1.0 - left / total as f32).clamp(0.0, 1.0)
    }

    /// Requested ranges as merged runs of whole seconds with their state: `(from, to, ready)`.
    /// Drawn as the pre-render bar at the top of the timeline.
    pub fn segments(&self) -> Vec<(f64, f64, bool)> {
        let mut secs: Vec<(i64, bool)> = Vec::new();
        for (a, b) in &self.ranges {
            for s in sec_range(*a, *b) {
                let ready = !self.dirty.contains(&s) && self.done.iter().any(|(x, _)| *x == s);
                match secs.iter_mut().find(|(x, _)| *x == s) {
                    Some(e) => e.1 |= ready,
                    None => secs.push((s, ready)),
                }
            }
        }
        secs.sort_by_key(|(s, _)| *s);
        let mut out: Vec<(f64, f64, bool)> = Vec::new();
        for (s, ready) in secs {
            match out.last_mut() {
                Some(l) if l.2 == ready && (l.1 - s as f64).abs() < 1e-9 => l.1 = (s + 1) as f64,
                _ => out.push((s as f64, (s + 1) as f64, ready)),
            }
        }
        out
    }

    pub fn clear(&mut self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.ranges.clear();
        self.queue.clear();
        self.done.clear();
        self.dirty.clear();
        self.inflight.clear();
        self.worker = None;
    }
}

/// Whole seconds covered by [a, b).
fn sec_range(a: f64, b: f64) -> std::ops::Range<i64> {
    if !(b > a) || !a.is_finite() || !b.is_finite() {
        return 0..0;
    }
    a.max(0.0).floor() as i64..b.max(0.0).ceil() as i64
}

fn dir() -> PathBuf {
    Settings::cache_dir().join("prerender")
}

fn path_for(key: u64) -> PathBuf {
    dir().join(format!("{key:016x}.rgba"))
}

/// Hash of everything that changes the picture during second `sec`: format, and every clip (with its
/// effects, graph, mask and asset) visible in that second on an active video track.
pub fn key_for(project: &Project, sec: i64) -> u64 {
    let (a, b) = (sec as f64, sec as f64 + 1.0);
    let mut h = DefaultHasher::new();
    VERSION.hash(&mut h);
    project.width.hash(&mut h);
    project.height.hash(&mut h);
    project.fps.to_bits().hash(&mut h);
    project.scaler.hash(&mut h);
    sec.hash(&mut h);
    let mut any_sequence = false;
    for (ti, track) in project.tracks.iter().enumerate() {
        if track.kind != TrackKind::Video || !project.active(ti) {
            continue;
        }
        for tr in &track.transitions {
            if let Some((l, r)) = track.transition_pair(tr) {
                if l.start < b && r.end() > a {
                    hash_json(&mut h, tr);
                }
            }
        }
        for clip in &track.clips {
            if clip.start >= b || clip.end() <= a || !clip.enabled {
                continue;
            }
            hash_json(&mut h, clip);
            if let Some(asset) = project.asset(clip.asset) {
                hash_json(&mut h, asset);
            }
            any_sequence |= clip.kind == ClipKind::Sequence;
        }
    }
    if any_sequence {
        // ponytail: a nested sequence rehashes wholesale — cheap enough, and always correct.
        hash_json(&mut h, &project.sequences);
    }
    if project.show_subtitles {
        project.subtitle_margin.to_bits().hash(&mut h);
        hash_json(&mut h, &project.subtitle_style);
        for cue in project.subtitles.iter().filter(|c| c.start < b && c.end > a) {
            hash_json(&mut h, cue);
        }
    }
    h.finish()
}

fn hash_json<T: serde::Serialize>(h: &mut DefaultHasher, v: &T) {
    match serde_json::to_string(v) {
        Ok(s) => s.hash(h),
        // unserialisable => treat as always-changed rather than silently equal
        Err(_) => std::time::SystemTime::now().hash(h),
    }
}

/// Frame `i` of a cache file, or None when the file is missing/short/foreign.
fn read_frame(path: &PathBuf, i: u32, pts: f64) -> Option<Frame> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut head = [0u8; HEADER as usize];
    f.read_exact(&mut head).ok()?;
    if &head[..4] != MAGIC {
        return None;
    }
    let u = |o: usize| u32::from_le_bytes([head[o], head[o + 1], head[o + 2], head[o + 3]]);
    if u(4) != VERSION {
        return None;
    }
    let (w, h, n) = (u(8), u(12), u(16));
    if w == 0 || h == 0 || n == 0 {
        return None;
    }
    let i = i.min(n - 1);
    let len = w as usize * h as usize * 4;
    f.seek(SeekFrom::Start(HEADER + i as u64 * len as u64)).ok()?;
    let mut out = Frame::new(w, h);
    f.read_exact(&mut out.rgba).ok()?;
    out.pts = pts;
    let _ = len;
    Some(out)
}

/// The cache key a file name encodes (`path_for`'s inverse); None for anything else in the folder.
fn key_of(p: &Path) -> Option<u64> {
    u64::from_str_radix(p.file_name()?.to_str()?.strip_suffix(".rgba")?, 16).ok()
}

/// Keep the cache under `budget` bytes, oldest files first. Returns the keys it deleted so the caller
/// can forget the seconds they held.
fn prune(d: &Path, budget: u64) -> Vec<u64> {
    let mut evicted = Vec::new();
    let Ok(rd) = std::fs::read_dir(d) else { return evicted };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            m.is_file().then(|| (m.modified().unwrap_or(std::time::UNIX_EPOCH), m.len(), e.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|(_, l, _)| *l).sum();
    if total <= budget {
        return evicted;
    }
    files.sort_by_key(|(t, _, _)| *t);
    for (_, len, p) in files {
        if total <= budget {
            break;
        }
        if std::fs::remove_file(&p).is_ok() {
            total = total.saturating_sub(len);
            evicted.extend(key_of(&p));
        }
    }
    evicted
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Clip, EffectKind, Project, Track};

    /// The timeline bar: rendered seconds merge into ready runs, everything else stays pending.
    #[test]
    fn segments_merge_by_state() {
        let mut pr = PreRender::new();
        pr.ranges = vec![(0.0, 5.0)];
        pr.done = vec![(0, 1), (1, 1), (3, 1)];
        pr.dirty = vec![1];
        // 0 ready, 1 dirty, 2 pending, 3 ready, 4 pending
        assert_eq!(pr.segments(), vec![(0.0, 1.0, true), (1.0, 3.0, false), (3.0, 4.0, true), (4.0, 5.0, false)]);
        assert!(PreRender::new().segments().is_empty());
    }

    /// Write a whole cache file at once (the renderer streams it; the tests do not need to).
    fn write_file(path: &PathBuf, data: &[u8]) -> std::io::Result<()> {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::File::create(path)?.write_all(data)
    }

    fn project_with_clip() -> Project {
        let mut p = Project::new();
        p.width = 32;
        p.height = 16;
        p.fps = 10.0;
        if p.tracks.is_empty() {
            let id = p.new_id();
            p.tracks.push(Track::new(id, TrackKind::Video, "V1"));
        }
        let id = p.new_id();
        let mut c = Clip::new(id, ClipKind::Text, "t", 0.0, 4.0);
        c.text = Some(crate::model::TextStyle::default());
        let v = p.tracks.iter().position(|t| t.kind == TrackKind::Video).unwrap();
        p.tracks[v].clips.push(c);
        p
    }

    #[test]
    fn key_tracks_the_picture_only() {
        let mut p = project_with_clip();
        let k0 = key_for(&p, 0);
        assert_eq!(k0, key_for(&p, 0), "stable");
        assert_ne!(k0, key_for(&p, 1), "different second, different key");

        // irrelevant edits keep the key
        p.name = "renamed".into();
        p.notes = "hello".into();
        assert_eq!(k0, key_for(&p, 0));

        // a clip move changes it
        let v = p.tracks.iter().position(|t| t.kind == TrackKind::Video).unwrap();
        p.tracks[v].clips[0].x = crate::model::Animated::new(5.0);
        let k1 = key_for(&p, 0);
        assert_ne!(k0, k1);

        // so does adding an effect, and changing one of its parameters
        p.tracks[v].clips[0].effects.push(crate::model::Effect::new(EffectKind::Blur));
        let k2 = key_for(&p, 0);
        assert_ne!(k1, k2);
        p.tracks[v].clips[0].effects[0].params[0] = crate::model::Animated::new(3.0);
        assert_ne!(k2, key_for(&p, 0));

        // a clip outside the second does not
        let before = key_for(&p, 0);
        let id = p.new_id();
        p.tracks[v].clips.push(Clip::new(id, ClipKind::Text, "later", 10.0, 1.0));
        assert_eq!(before, key_for(&p, 0));
        assert_ne!(before, key_for(&p, 10));
    }

    #[test]
    fn frame_is_none_for_dirty_and_unrendered_ranges() {
        let p = project_with_clip();
        let mut pr = PreRender::new();
        assert!(pr.frame(&p, 0.0).is_none(), "nothing rendered yet");
        // pretend second 0 is in the cache
        pr.done.push((0, key_for(&p, 0)));
        pr.dirty.push(0);
        assert!(pr.frame(&p, 0.5).is_none(), "dirty range must never serve a frame");
        pr.dirty.clear();
        // the file is not there, so it still declines (but for the other reason)
        assert!(pr.frame(&p, 0.5).is_none());
        assert!(pr.frame(&p, -1.0).is_none());
    }

    #[test]
    fn invalidate_requeues_only_requested_seconds() {
        let p = project_with_clip();
        let mut pr = PreRender::new();
        pr.ranges.push((0.0, 2.0));
        pr.done.push((0, key_for(&p, 0)));
        pr.done.push((5, key_for(&p, 5)));
        pr.invalidate(0.2, 0.8);
        assert!(pr.dirty.contains(&0));
        assert!(pr.queue.contains(&0), "inside the requested range -> re-render");
        pr.invalidate(5.0, 5.5);
        assert!(pr.dirty.contains(&5));
        assert!(!pr.queue.contains(&5), "outside every request -> just dropped");
        assert!(pr.done.iter().all(|(s, _)| *s != 0 && *s != 5), "invalidated seconds are not ready");
    }

    #[test]
    fn progress_counts_the_queue() {
        let mut pr = PreRender::new();
        assert_eq!(pr.progress(), 1.0, "nothing requested is nothing to wait for");
        pr.ranges.push((0.0, 4.0));
        pr.queue = vec![0, 1, 2, 3];
        assert_eq!(pr.progress(), 0.0);
        pr.queue = vec![3];
        assert!((pr.progress() - 0.75).abs() < 1e-6);
        pr.queue.clear();
        assert_eq!(pr.progress(), 1.0);
    }

    /// Every edit re-requests the whole timeline. Unmerged, `ranges` grows with the edit count and
    /// takes `segments()` and `progress()` — drawn every frame — with it, until the UI stalls.
    #[test]
    fn repeated_requests_merge_into_one_range() {
        let p = project_with_clip();
        let mut pr = PreRender::new();
        for _ in 0..50 {
            pr.request(&p, 0.0, 4.0);
        }
        assert_eq!(pr.ranges, vec![(0.0, 4.0)], "re-requesting the same range must not grow it");
        pr.request(&p, 10.0, 12.0);
        assert_eq!(pr.ranges, vec![(0.0, 4.0), (10.0, 12.0)], "a disjoint request stays its own run");
        pr.request(&p, 3.0, 11.0);
        assert_eq!(pr.ranges, vec![(0.0, 12.0)], "one that bridges them merges all three");
    }

    #[test]
    fn seconds_cover_the_range() {
        assert_eq!(sec_range(0.0, 1.0).collect::<Vec<_>>(), vec![0]);
        assert_eq!(sec_range(0.5, 2.1).collect::<Vec<_>>(), vec![0, 1, 2]);
        assert_eq!(sec_range(2.0, 2.0).count(), 0);
        assert_eq!(sec_range(f64::NAN, 1.0).count(), 0);
    }

    #[test]
    fn tick_hands_seconds_to_the_worker() {
        let mut p = project_with_clip(); // 33×17 @ 10 fps, one text clip: no decoder needed
                                         // an odd size gives this test a cache key of its own, so the tests that assert "no file
                                         // for this second" never race with the one this writes
        p.width = 33;
        p.height = 17;
        let paths: Vec<PathBuf> = (0..3).map(|s| path_for(key_for(&p, s))).collect();
        for path in &paths {
            let _ = std::fs::remove_file(path);
        }
        let mut pr = PreRender::new();
        pr.request(&p, 0.0, 3.0);
        assert_eq!(pr.queue, vec![0, 1, 2]);
        let start = std::time::Instant::now();
        while pr.tick(&p, 4.0) {
            assert!(pr.inflight.len() <= DEPTH, "{} seconds in flight at once", pr.inflight.len());
            assert!(start.elapsed() < std::time::Duration::from_secs(30), "not converging");
        }
        assert!(paths.iter().all(|path| path.exists()), "every finished second is published");
        assert!(pr.frame(&p, 2.05).is_some(), "and served from the cache");
        for path in &paths {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn prune_reports_the_keys_it_deleted() {
        let d = std::env::temp_dir().join(format!("se-prune-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let keys = [0x11u64, 0x22, 0x33];
        for k in keys {
            write_file(&d.join(format!("{k:016x}.rgba")), &[0u8; 64]).unwrap();
        }
        assert!(prune(&d, 1 << 20).is_empty(), "inside the budget nothing is touched");
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 3);
        let mut got = prune(&d, 0);
        got.sort_unstable();
        assert_eq!(got, keys, "every deleted second must be reported");
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 0);
        // a stray file is still pruned, it just has no key to forget
        write_file(&d.join("half.tmp"), b"x").unwrap();
        assert!(prune(&d, 0).is_empty());
        assert_eq!(key_of(&path_for(0xdead_beef)), Some(0xdead_beef));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn cache_file_round_trip() {
        let dir = std::env::temp_dir().join(format!("se-prerender-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("x.rgba");
        let (w, h, n) = (2u32, 2u32, 3u32);
        let mut buf = Vec::new();
        buf.extend_from_slice(MAGIC);
        for v in [VERSION, w, h, n] {
            buf.extend_from_slice(&v.to_le_bytes());
        }
        for i in 0..n {
            buf.extend(std::iter::repeat(i as u8 + 1).take((w * h * 4) as usize));
        }
        write_file(&path, &buf).unwrap();
        let f = read_frame(&path, 1, 1.25).unwrap();
        assert_eq!((f.width, f.height), (2, 2));
        assert!(f.rgba.iter().all(|b| *b == 2), "frame 1 must be the second block");
        assert!((f.pts - 1.25).abs() < 1e-9);
        // past the end clamps to the last frame, a foreign file is refused
        assert!(read_frame(&path, 99, 0.0).unwrap().rgba.iter().all(|b| *b == 3));
        write_file(&path, b"nope").unwrap();
        assert!(read_frame(&path, 0, 0.0).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
