//! Pre-render ("movie mode"): render ranges at full quality into a cache so playback shows exactly what
//! an export would, without re-running the effect chain every frame.
//!
//! Cache: `Settings::cache_dir()/prerender/<hash>.rgba`, one file per second of timeline at project
//! resolution, keyed by a hash of everything affecting the picture in that second. Work happens in small
//! slices on the UI thread (the GPU renderer owns the GL context) or on the CPU compositor without GL.
//!
//! File layout: `SEPR` + version + width + height + frame count (u32 LE each), then that many
//! `width * height * 4` RGBA frames. `frame()` seeks to the one it needs, so a second costs one 8 MB
//! read at 1080p and never a full second of RAM.
//!
//! ponytail: rendering runs on `engine::compose::Compositor` — `tick` has no room for the GL renderer,
//! and the cache is identical either way. Hand a `&mut GpuRenderer` in when movie mode needs the GPU
//! shaders (that is the only change required; the keys and files do not move).

use crate::engine::compose::Compositor;
use crate::engine::text::TextRasterizer;
use crate::media::{Backend, DecoderPool, Frame};
use crate::model::{ClipKind, Project, TrackKind};
use crate::settings::Settings;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
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
    /// Whole seconds still to render, in order.
    queue: Vec<i64>,
    /// Seconds rendered this session with the key they were rendered under.
    done: Vec<(i64, u64)>,
    /// Seconds invalidated since they were rendered (never served, always re-rendered).
    dirty: Vec<i64>,
    /// Scratch, so a tick does not allocate a compositor per slice.
    work: Option<Box<Work>>,
}

/// Everything a slice of rendering needs (lazily created: an idle PreRender costs nothing).
struct Work {
    comp: Compositor,
    pool: DecoderPool,
    text: TextRasterizer,
    frame: Frame,
}

impl PreRender {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark a range dirty (an edit touched it).
    pub fn invalidate(&mut self, from: f64, to: f64) {
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
        self.ranges.push((a, b));
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

    /// Do a slice of work (call once per frame with a time budget). Returns true while busy.
    pub fn tick(&mut self, project: &Project, budget_ms: f32) -> bool {
        if self.queue.is_empty() {
            self.work = None;
            return false;
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs_f32(budget_ms.max(0.0) / 1000.0);
        let (w, h) = (project.width.max(1), project.height.max(1));
        let fps = if project.fps > 1.0 { project.fps } else { 30.0 };
        let n = fps.round().max(1.0) as u32;
        // taken out of `self` for the loop: the slice needs both the scratch and the queue
        let mut work = self.work.take().unwrap_or_else(|| {
            Box::new(Work {
                comp: Compositor::new(),
                pool: DecoderPool::new(Backend::Auto),
                text: TextRasterizer::new(),
                frame: Frame::default(),
            })
        });
        // ponytail: the budget is checked between seconds, so one slice is one second of frames —
        // fine at preview sizes, and a partial-second resume is the upgrade if 4K ever stutters.
        while let Some(&sec) = self.queue.first() {
            let key = key_for(project, sec);
            let path = path_for(key);
            if !path.exists() {
                // streamed frame by frame: a second of 1080p RGBA is 250 MB, far too much to buffer
                let tmp = path.with_extension("tmp");
                let write = |work: &mut Work| -> std::io::Result<()> {
                    if let Some(d) = path.parent() {
                        std::fs::create_dir_all(d)?;
                    }
                    let mut f = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
                    f.write_all(MAGIC)?;
                    for v in [VERSION, w, h, n] {
                        f.write_all(&v.to_le_bytes())?;
                    }
                    for i in 0..n {
                        let t = sec as f64 + i as f64 / fps;
                        work.comp.render(project, t, w, h, &mut work.pool, &mut work.text, &mut work.frame);
                        f.write_all(&work.frame.rgba)?;
                    }
                    drop(f.into_inner().map_err(|e| e.into_error())?);
                    std::fs::rename(&tmp, &path)
                };
                if write(&mut work).is_err() {
                    // out of disk / no cache dir: stop trying, playback just falls back to live render
                    let _ = std::fs::remove_file(&tmp);
                    self.queue.clear();
                    return false;
                }
                prune(CACHE_BUDGET);
            }
            self.queue.remove(0);
            self.dirty.retain(|d| *d != sec);
            if !self.done.contains(&(sec, key)) {
                self.done.push((sec, key));
            }
            if std::time::Instant::now() >= deadline {
                break;
            }
        }
        self.work = Some(work);
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

    pub fn clear(&mut self) {
        self.ranges.clear();
        self.queue.clear();
        self.done.clear();
        self.dirty.clear();
        self.work = None;
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

/// Keep the cache under `budget` bytes, oldest files first.
fn prune(budget: u64) {
    let Ok(rd) = std::fs::read_dir(dir()) else { return };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            m.is_file().then(|| (m.modified().unwrap_or(std::time::UNIX_EPOCH), m.len(), e.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|(_, l, _)| *l).sum();
    if total <= budget {
        return;
    }
    files.sort_by_key(|(t, _, _)| *t);
    for (_, len, p) in files {
        if total <= budget {
            break;
        }
        if std::fs::remove_file(&p).is_ok() {
            total = total.saturating_sub(len);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Clip, EffectKind, Project, Track};

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

    #[test]
    fn seconds_cover_the_range() {
        assert_eq!(sec_range(0.0, 1.0).collect::<Vec<_>>(), vec![0]);
        assert_eq!(sec_range(0.5, 2.1).collect::<Vec<_>>(), vec![0, 1, 2]);
        assert_eq!(sec_range(2.0, 2.0).count(), 0);
        assert_eq!(sec_range(f64::NAN, 1.0).count(), 0);
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
