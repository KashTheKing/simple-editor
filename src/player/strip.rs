//! The seek bar's hover thumbnails: a strip of small (`W` px wide) frames built in the background as
//! soon as a file opens, by ffmpeg child processes, never the playback decoder.
//!
//! The file is cut into segments, worked from the END toward the start. Pass 1 decodes only the
//! keyframes of each segment (`-skip_frame nokey`: a few ms a frame, so the whole bar has a picture
//! within seconds); pass 2 then decodes each segment fully and keeps one frame every `step` s, the exact
//! moments. Hovering a part of the bar that has nothing within `step` yet moves that segment to the
//! front of the queue. Hover shows the nearest thumbnail made so far, never a blank.
//!
//! Playback first: workers run as below-normal priority processes; while playing only one runs, software
//! decode with 2 threads (the GPU decoder stays the player's); paused, `WORKERS` run with the hardware
//! decoder. A finished strip goes to the disk cache (MJPEG + the times as JSON), keyed by the file's
//! path, size and mtime, so reopening the file is instant.
//! ponytail: the cache is never pruned; prune by age with the other caches if it grows.

use eframe::egui;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

/// Thumbnail width in pixels (the height keeps the picture's aspect).
pub const W: u32 = 160;
const WORKERS: usize = 3;

pub struct Thumb {
    pub t: f64,
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Pass {
    Keys,
    Fine,
}

#[derive(Default)]
struct State {
    /// Sorted by `t`.
    thumbs: Vec<Thumb>,
    /// Jobs not started yet, front first: (pass, segment).
    queue: VecDeque<(Pass, usize)>,
    running: usize,
    quit: bool,
    /// Every job done (or loaded from the cache).
    done: bool,
}

struct Shared {
    st: Mutex<State>,
    cv: Condvar,
    playing: AtomicBool,
    path: PathBuf,
    dur: f64,
    seg: f64,
    step: f64,
}

pub struct Strip {
    s: Arc<Shared>,
    /// The texture of the thumbnail last shown and its time.
    tex: Option<(egui::TextureHandle, f64)>,
}

/// One thumbnail every `step` seconds (pass 2): a dense strip, at most ~600 frames.
pub fn step_for(dur: f64) -> f64 {
    (dur / 600.0).max(if dur < 30.0 { 0.25 } else { 1.0 })
}

/// Segment length: ~20 s (the unit of priority), fewer for long files.
fn seg_for(dur: f64) -> f64 {
    (dur / 64.0).max(20.0).min(dur.max(0.1))
}

/// The jobs in the order they run: every segment's keyframes from the last segment to the first, then
/// the full pass the same way.
fn job_order(n: usize) -> VecDeque<(Pass, usize)> {
    let keys = (0..n).rev().map(|i| (Pass::Keys, i));
    keys.chain((0..n).rev().map(|i| (Pass::Fine, i))).collect()
}

/// Index of the thumbnail nearest `t` (None when there are none). `thumbs` sorted by time.
pub fn nearest(thumbs: &[Thumb], t: f64) -> Option<usize> {
    let i = thumbs.partition_point(|x| x.t < t);
    [i.checked_sub(1), (i < thumbs.len()).then_some(i)]
        .into_iter()
        .flatten()
        .min_by(|&a, &b| (thumbs[a].t - t).abs().total_cmp(&(thumbs[b].t - t).abs()))
}

/// The disk cache's name for `path`: changes when the file is replaced or edited (size, mtime).
pub fn cache_key(path: &Path) -> Option<String> {
    use std::hash::{Hash, Hasher};
    let m = std::fs::metadata(path).ok()?;
    let mtime = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (path.to_string_lossy().to_lowercase(), m.len(), mtime, W).hash(&mut h);
    Some(format!("{:016x}", h.finish()))
}

fn cache_paths(path: &Path) -> Option<(PathBuf, PathBuf)> {
    let k = cache_key(path)?;
    let d = crate::settings::Settings::cache_dir().join("player-thumbs");
    Some((d.join(format!("{k}.mjpeg")), d.join(format!("{k}.json"))))
}

impl Strip {
    /// Start building the strip of `path` (`dur` s long). Nothing repaints when a thumbnail lands: the
    /// hover card repaints itself while the strip is filling.
    pub fn start(path: &Path, dur: f64) -> Strip {
        Self::start_with(path, dur, WORKERS, true)
    }

    fn start_with(path: &Path, dur: f64, workers: usize, cache: bool) -> Strip {
        let seg = seg_for(dur);
        let n = (dur / seg).ceil().max(1.0) as usize;
        let s = Arc::new(Shared {
            st: Mutex::new(State::default()),
            cv: Condvar::new(),
            playing: AtomicBool::new(false),
            path: path.to_path_buf(),
            dur,
            seg,
            step: step_for(dur),
        });
        // off the UI thread: reading the cache back is an ffmpeg run too
        let s2 = s.clone();
        let _ = std::thread::Builder::new().name("player-thumbs".into()).spawn(move || {
            if cache && load_cache(&s2) {
                return;
            }
            if let Ok(mut st) = s2.st.lock() {
                st.queue = job_order(n);
            }
            for i in 1..workers {
                let s = s2.clone();
                let _ = std::thread::Builder::new().name(format!("player-thumbs{i}")).spawn(move || worker(s, i, cache));
            }
            worker(s2, 0, cache);
        });
        Strip { s, tex: None }
    }

    /// The player calls this every frame: only one light worker runs while playing.
    pub fn set_playing(&self, on: bool) {
        if self.s.playing.swap(on, Ordering::Relaxed) != on && !on {
            self.s.cv.notify_all();
        }
    }

    pub fn len(&self) -> usize {
        self.s.st.lock().map_or(0, |st| st.thumbs.len())
    }

    pub fn done(&self) -> bool {
        self.s.st.lock().is_ok_and(|st| st.done)
    }

    /// Decoded bytes held.
    pub fn bytes(&self) -> usize {
        self.s.st.lock().map_or(0, |st| st.thumbs.iter().map(|t| t.rgba.len()).sum())
    }

    /// The thumbnail nearest `t` as a texture (uploaded only when it changes), with its pixel size.
    /// Hovering a part with nothing within a step of `t` moves its segment to the front of the queue.
    pub fn texture(&mut self, ctx: &egui::Context, t: f64) -> Option<(egui::TextureId, egui::Vec2)> {
        let Ok(mut st) = self.s.st.lock() else { return None };
        let i = nearest(&st.thumbs, t);
        if i.is_none_or(|i| (st.thumbs[i].t - t).abs() > self.s.step) {
            let seg = ((t / self.s.seg) as usize).min(((self.s.dur / self.s.seg).ceil() as usize).saturating_sub(1));
            let mut first = Vec::new();
            st.queue.retain(|&(p, i)| {
                let hit = i == seg;
                if hit {
                    first.push((p, i));
                }
                !hit
            });
            for j in first.into_iter().rev() {
                st.queue.push_front(j);
            }
        }
        let th = &st.thumbs[i?];
        let size = egui::vec2(th.w as f32, th.h as f32);
        match &mut self.tex {
            Some((tex, at)) if *at == th.t => Some((tex.id(), size)),
            slot => {
                let img = egui::ColorImage::from_rgba_unmultiplied([th.w as usize, th.h as usize], &th.rgba);
                let at = th.t;
                drop(st);
                match slot {
                    Some((tex, t0)) => {
                        tex.set(img, egui::TextureOptions::LINEAR);
                        *t0 = at;
                    }
                    None => *slot = Some((ctx.load_texture("player-hover-thumb", img, egui::TextureOptions::LINEAR), at)),
                }
                slot.as_ref().map(|(tex, _)| (tex.id(), size))
            }
        }
    }
}

impl Drop for Strip {
    fn drop(&mut self) {
        if let Ok(mut st) = self.s.st.lock() {
            st.quit = true;
        }
        self.s.cv.notify_all();
    }
}

fn insert(s: &Shared, th: Thumb) {
    let Ok(mut st) = s.st.lock() else { return };
    let i = st.thumbs.partition_point(|x| x.t < th.t);
    if st.thumbs.get(i).is_some_and(|x| (x.t - th.t).abs() < 1e-6) {
        return; // a keyframe the full pass decoded again
    }
    st.thumbs.insert(i, th);
}

fn worker(s: Arc<Shared>, id: usize, cache: bool) {
    loop {
        let job = {
            let Ok(mut st) = s.st.lock() else { return };
            loop {
                if st.quit {
                    return;
                }
                if st.queue.is_empty() && st.running == 0 {
                    if !st.done {
                        st.done = true;
                        drop(st);
                        if cache {
                            save_cache(&s);
                        }
                    }
                    return;
                }
                // playing: worker 0 alone
                if !(s.playing.load(Ordering::Relaxed) && id > 0) {
                    if let Some(j) = st.queue.pop_front() {
                        st.running += 1;
                        break j;
                    }
                }
                st = match s.cv.wait_timeout(st, std::time::Duration::from_millis(250)) {
                    Ok((g, _)) => g,
                    Err(_) => return,
                };
            }
        };
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_job(&s, job)));
        if let Ok(mut st) = s.st.lock() {
            st.running -= 1;
        }
        s.cv.notify_all();
    }
}

/// Parse a showinfo line: (pts_time, width, height).
fn showinfo(line: &str) -> Option<(f64, u32, u32)> {
    if !line.contains("Parsed_showinfo") {
        return None;
    }
    let t = line.split("pts_time:").nth(1)?.split_whitespace().next()?.parse().ok()?;
    let s = line.split(" s:").nth(1)?.split_whitespace().next()?;
    let (w, h) = s.split_once('x')?;
    Some((t, w.parse().ok()?, h.parse().ok()?))
}

/// Read frames from an ffmpeg child that writes rawvideo RGBA to stdout and `showinfo` to stderr,
/// calling `each` per frame until it returns false or the child ends.
fn read_frames(mut c: std::process::Child, quit: impl Fn() -> bool, mut each: impl FnMut(Thumb) -> bool) {
    let (Some(mut out), Some(err)) = (c.stdout.take(), c.stderr.take()) else { return };
    let (tx, rx) = std::sync::mpsc::channel();
    let _ = std::thread::Builder::new().name("player-thumbs-log".into()).spawn(move || {
        for line in BufReader::new(err).lines().map_while(Result::ok) {
            if let Some(f) = showinfo(&line) {
                if tx.send(f).is_err() {
                    return;
                }
            }
        }
    });
    while let Ok((t, w, h)) = rx.recv() {
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        if out.read_exact(&mut rgba).is_err() || quit() || !each(Thumb { t, w, h, rgba }) {
            break;
        }
    }
    let _ = c.kill();
    let _ = c.wait();
}

fn run_job(s: &Shared, (pass, i): (Pass, usize)) {
    let Some(exe) = crate::media::ffpipe::ffmpeg_exe() else { return };
    let a = i as f64 * s.seg;
    let len = s.seg.min(s.dur - a + 0.5);
    let playing = s.playing.load(Ordering::Relaxed);
    // below normal (`bg::command`), not idle: an idle-priority ffmpeg starved behind a busy PC (a build:
    // the first thumbnail took 53 s instead of 0.5 s, measured)
    let mut c = crate::media::bg::command(&exe);
    c.args(["-hide_banner", "-nostats", "-v", "info", "-copyts"]);
    if playing {
        c.args(["-threads", "2"]);
    } else {
        c.args(["-hwaccel", "auto"]);
    }
    if pass == Pass::Keys {
        c.args(["-skip_frame", "nokey"]);
    }
    c.arg("-ss").arg(format!("{a:.3}")).arg("-t").arg(format!("{len:.3}")).arg("-i").arg(&s.path);
    let vf = match pass {
        Pass::Keys => format!("scale={W}:-2,showinfo"),
        Pass::Fine => format!("select='isnan(prev_selected_t)+gte(t-prev_selected_t\\,{})',scale={W}:-2,showinfo", s.step - 0.001),
    };
    c.args(["-an", "-sn", "-dn", "-vf", &vf, "-fps_mode", "passthrough", "-f", "rawvideo", "-pix_fmt", "rgba", "-"]);
    c.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let Ok(child) = c.spawn() else { return };
    let quit = || s.st.lock().map_or(true, |st| st.quit);
    // -ss before -i lands on the keyframe before `a` for the keys pass: drop what the previous segment has
    read_frames(child, quit, |th| {
        if pass == Pass::Keys && i > 0 && th.t < a - 0.001 {
            return true;
        }
        insert(s, th);
        true
    });
}

fn save_cache(s: &Shared) {
    let Some((mj, js)) = cache_paths(&s.path) else { return };
    let Some(exe) = crate::media::ffpipe::ffmpeg_exe() else { return };
    let Ok(st) = s.st.lock() else { return };
    let Some(f) = st.thumbs.first() else { return };
    let (w, h) = (f.w, f.h);
    if st.thumbs.iter().any(|t| (t.w, t.h) != (w, h)) {
        return;
    }
    let times: Vec<f64> = st.thumbs.iter().map(|t| t.t).collect();
    let frames: Vec<u8> = st.thumbs.iter().flat_map(|t| t.rgba.iter().copied()).collect();
    drop(st);
    let _ = std::fs::create_dir_all(mj.parent().unwrap_or(Path::new(".")));
    let mut c = crate::media::bg::command(&exe);
    c.args(["-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgba", "-s", &format!("{w}x{h}"), "-i", "-"]);
    c.args(["-q:v", "3", "-f", "mjpeg"]).arg(&mj).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null());
    let Ok(mut child) = c.spawn() else { return };
    let ok = child.stdin.take().is_some_and(|mut i| i.write_all(&frames).is_ok());
    if ok && child.wait().is_ok_and(|e| e.success()) {
        let _ = std::fs::write(js, serde_json::to_vec(&(w, h, times)).unwrap_or_default());
    }
}

/// Fill the strip from the disk cache. False when there is none (or it does not read back whole).
fn load_cache(s: &Arc<Shared>) -> bool {
    let Some((mj, js)) = cache_paths(&s.path) else { return false };
    let Some((w, h, times)) = std::fs::read(&js).ok().and_then(|b| serde_json::from_slice::<(u32, u32, Vec<f64>)>(&b).ok())
    else {
        return false;
    };
    let Some(exe) = crate::media::ffpipe::ffmpeg_exe() else { return false };
    let mut c = crate::media::ffpipe::command(&exe);
    c.args(["-hide_banner", "-nostats", "-v", "info", "-f", "mjpeg", "-i"]).arg(&mj);
    c.args(["-vf", "showinfo", "-fps_mode", "passthrough", "-f", "rawvideo", "-pix_fmt", "rgba", "-"]);
    c.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let Ok(child) = c.spawn() else { return false };
    let mut thumbs = Vec::with_capacity(times.len());
    read_frames(child, || false, |mut th| {
        th.t = times.get(thumbs.len()).copied().unwrap_or(f64::NAN);
        thumbs.push(th);
        true
    });
    if thumbs.len() != times.len() || thumbs.iter().any(|t| (t.w, t.h) != (w, h)) {
        return false;
    }
    if let Ok(mut st) = s.st.lock() {
        st.thumbs = thumbs;
        st.done = true;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn th(t: f64) -> Thumb {
        Thumb { t, w: 1, h: 1, rgba: vec![0; 4] }
    }

    #[test]
    fn hover_returns_the_nearest_thumbnail() {
        assert_eq!(nearest(&[], 3.0), None);
        let v: Vec<Thumb> = [1.0, 4.0, 10.0].into_iter().map(th).collect();
        assert_eq!(nearest(&v, 0.0), Some(0));
        assert_eq!(nearest(&v, 2.4), Some(0));
        assert_eq!(nearest(&v, 2.6), Some(1));
        assert_eq!(nearest(&v, 8.0), Some(2));
        assert_eq!(nearest(&v, 99.0), Some(2));
    }

    #[test]
    fn jobs_run_end_first_keyframes_before_the_full_pass() {
        let q: Vec<_> = job_order(3).into_iter().collect();
        use Pass::*;
        assert_eq!(q, [(Keys, 2), (Keys, 1), (Keys, 0), (Fine, 2), (Fine, 1), (Fine, 0)]);
        assert_eq!(step_for(10.0), 0.25);
        assert_eq!(step_for(180.0), 1.0);
        assert_eq!(step_for(7200.0), 12.0);
    }

    #[test]
    fn the_cache_key_changes_when_the_file_changes() {
        let p = std::env::temp_dir().join(format!("se-strip-key-{}.mp4", std::process::id()));
        std::fs::write(&p, b"one").unwrap();
        let a = cache_key(&p).unwrap();
        assert_eq!(cache_key(&p).unwrap(), a, "stable");
        std::fs::write(&p, b"other").unwrap();
        assert_ne!(cache_key(&p).unwrap(), a, "size and mtime are in the key");
        let _ = std::fs::remove_file(&p);
        assert!(cache_key(&p).is_none());
    }

    #[test]
    fn showinfo_lines_parse() {
        let l = "[Parsed_showinfo_1 @ 0000] n:   0 pts: 1234 pts_time:12.5    duration:1 fmt:rgba cl:left sar:1/1 s:160x90 i:P iskey:1";
        assert_eq!(showinfo(l), Some((12.5, 160, 90)));
        assert_eq!(showinfo("frame=  1 fps=0"), None);
    }

    /// `SE_STRIP_FILE=<video> cargo test strip_bench -- --ignored --nocapture`: time to the first
    /// thumbnail, to the whole strip, its memory, and how fast the disk cache reads it back.
    #[test]
    #[ignore]
    fn strip_bench() {
        let p = PathBuf::from(std::env::var("SE_STRIP_FILE").expect("SE_STRIP_FILE"));
        let dur: f64 = String::from_utf8_lossy(
            &std::process::Command::new("ffprobe")
                .args(["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0"])
                .arg(&p)
                .output()
                .unwrap()
                .stdout,
        )
        .trim()
        .parse()
        .unwrap();
        let t0 = Instant::now();
        let strip = Strip::start_with(&p, dur, WORKERS, true);
        let (mut first, mut keys_done) = (None, None);
        let seg = seg_for(dur);
        let n = (dur / seg).ceil() as usize;
        while !strip.done() {
            let st = strip.s.st.lock().unwrap();
            if first.is_none() && !st.thumbs.is_empty() {
                first = Some((t0.elapsed(), st.thumbs[0].t));
            }
            if keys_done.is_none() && !st.queue.is_empty() && !st.queue.iter().any(|j| j.0 == Pass::Keys) {
                keys_done = Some((t0.elapsed(), st.thumbs.len()));
            }
            drop(st);
            std::thread::sleep(Duration::from_millis(5));
        }
        let full = t0.elapsed();
        println!("file {:.1} s, {} segments of {seg:.1} s, step {} s", dur, n, step_for(dur));
        println!("first thumbnail after {:?} (at t = {:.1} s)", first.map(|f| f.0), first.map_or(0.0, |f| f.1));
        println!("last keyframe job started after {:?} ({} thumbs then)", keys_done.map(|k| k.0), keys_done.map_or(0, |k| k.1));
        println!("full strip after {full:?}: {} thumbs, {:.1} MB in memory", strip.len(), strip.bytes() as f64 / 1e6);
        drop(strip);
        let (_, js) = cache_paths(&p).unwrap();
        let t_save = Instant::now();
        while !js.is_file() && t_save.elapsed() < Duration::from_secs(20) {
            std::thread::sleep(Duration::from_millis(10)); // the cache is written just after `done`
        }
        println!("disk cache written {:?} after the strip was done", t_save.elapsed());
        let t1 = Instant::now();
        let again = Strip::start_with(&p, dur, WORKERS, true);
        while !again.done() {
            std::thread::sleep(Duration::from_millis(2));
        }
        let (mj, _) = cache_paths(&p).unwrap();
        println!(
            "reopened from the disk cache in {:?}: {} thumbs ({:.1} MB on disk)",
            t1.elapsed(),
            again.len(),
            std::fs::metadata(mj).map_or(0, |m| m.len()) as f64 / 1e6
        );
    }

    /// A 40 s clip (keyframe every 2 s, segments of 20 s), one worker, no cache: the first thumbnail is
    /// from the last segment, and the finished strip has one frame per second at least.
    #[test]
    fn the_strip_fills_end_first() {
        let p = std::env::temp_dir().join(format!("se-strip-{}.mp4", std::process::id()));
        let st = std::process::Command::new("ffmpeg")
            .args(["-y", "-v", "error", "-f", "lavfi", "-i", "testsrc=s=320x180:r=30:d=40", "-g", "60", "-pix_fmt", "yuv420p"])
            .arg(&p)
            .status()
            .expect("ffmpeg on PATH");
        assert!(st.success());
        let strip = Strip::start_with(&p, 40.0, 1, false);
        let t0 = Instant::now();
        let first = loop {
            if let Some(t) = strip.s.st.lock().unwrap().thumbs.first().map(|t| t.t) {
                break t;
            }
            assert!(t0.elapsed() < Duration::from_secs(20), "no thumbnail");
            std::thread::sleep(Duration::from_millis(5));
        };
        assert!(first >= 20.0, "first thumbnail from the end, got {first}");
        while !strip.done() {
            assert!(t0.elapsed() < Duration::from_secs(60), "never finished");
            std::thread::sleep(Duration::from_millis(20));
        }
        let st = strip.s.st.lock().unwrap();
        assert!(st.thumbs.windows(2).all(|w| w[0].t < w[1].t), "sorted");
        assert!(st.thumbs.len() >= 40, "one per second at least: {}", st.thumbs.len());
        assert!(st.thumbs.iter().all(|t| t.w == W && t.h == 90));
        let _ = std::fs::remove_file(&p);
    }
}
