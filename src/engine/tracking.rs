//! Point / area tracking (the Tracking pane): zero-mean normalised cross-correlation of a template
//! patch over a search window, frame by frame, producing exactly the `(x, y, t)` point list a
//! `PathAsset` holds — so a track can be saved as a project path and replayed onto a clip's X/Y
//! keyframes (`Project::apply_path`).
//!
//! The job owns a thread and its own `DecoderPool` (same shape as every other worker here) and streams
//! one point plus a progress fraction per frame; dropping the job hangs up the channel and the worker
//! stops on its next send. The UI never blocks.
//!
//! ponytail: the clip is tracked in its source frame decoded at project resolution, so the clip's own
//! transform is ignored — right for the usual full-frame clip. Render through `engine::compose` per
//! frame if a scaled / rotated / nested clip ever has to track.

use crate::media::{Backend, DecoderPool, Frame};
use crate::model::{Id, Project};
use std::sync::mpsc::{channel, Receiver, TryRecvError};

/// Luma at (x, y), edge-clamped (the search window is allowed to hang off the frame).
fn luma(f: &Frame, x: i32, y: i32) -> f32 {
    let x = x.clamp(0, f.width as i32 - 1) as usize;
    let y = y.clamp(0, f.height as i32 - 1) as usize;
    let p = &f.rgba[y * f.stride() + x * 4..];
    p[0] as f32 * 0.299 + p[1] as f32 * 0.587 + p[2] as f32 * 0.114
}

/// The `(2*hw+1) x (2*hh+1)` luma patch centred on (cx, cy).
pub fn patch(f: &Frame, cx: i32, cy: i32, hw: i32, hh: i32) -> Vec<f32> {
    let (tw, th) = (2 * hw + 1, 2 * hh + 1);
    let mut out = Vec::with_capacity((tw * th).max(0) as usize);
    if f.is_empty() {
        return out;
    }
    for j in 0..th {
        for i in 0..tw {
            out.push(luma(f, cx - hw + i, cy - hh + j));
        }
    }
    out
}

/// Best zero-mean NCC match for `tpl` (a `patch` of the same half-sizes) within `search` px of
/// (cx, cy): the matched centre and its score in -1..=1. None when the template or every candidate
/// window is flat — there is nothing to lock onto, and the caller keeps the previous position.
///
/// ponytail: brute-force scan, O(search² · patch) per frame — fine at the sizes the pane offers.
/// Go to a coarse-to-fine image pyramid if a 4K search radius ever needs to be interactive.
pub fn best_match(f: &Frame, tpl: &[f32], hw: i32, hh: i32, cx: i32, cy: i32, search: i32) -> Option<(i32, i32, f32)> {
    let (tw, th) = (2 * hw + 1, 2 * hh + 1);
    let n = (tw * th) as f32;
    if f.is_empty() || tpl.len() != (tw * th) as usize {
        return None;
    }
    let tmean = tpl.iter().sum::<f32>() / n;
    let tvar: f32 = tpl.iter().map(|v| (v - tmean) * (v - tmean)).sum();
    if tvar <= 1e-3 {
        return None;
    }
    let mut best: Option<(i32, i32, f32)> = None;
    for dy in -search..=search {
        for dx in -search..=search {
            let (ox, oy) = (cx + dx, cy + dy);
            let (mut s, mut ss, mut dot) = (0.0f32, 0.0f32, 0.0f32);
            for j in 0..th {
                for i in 0..tw {
                    let v = luma(f, ox - hw + i, oy - hh + j);
                    s += v;
                    ss += v * v;
                    dot += v * tpl[(j * tw + i) as usize];
                }
            }
            let wmean = s / n;
            let wvar = ss - s * wmean;
            if wvar <= 1e-3 {
                continue;
            }
            let score = (dot - n * tmean * wmean) / (tvar * wvar).sqrt();
            if best.is_none_or(|(_, _, b)| score > b) {
                best = Some((ox, oy, score));
            }
        }
    }
    best
}

/// One tracked frame: its point (canvas px relative to the centre, clip-local seconds) and how far
/// through the clip the worker is.
type Msg = ((f32, f32, f32), f32);

pub struct TrackJob {
    rx: Receiver<Msg>,
    /// Points so far, in `PathAsset::points` form and always in time order once `poll` reports done.
    pub points: Vec<(f32, f32, f32)>,
    pub progress: f32,
}

impl TrackJob {
    /// Track `clip` from its head (or its tail, `backward`) with the box centred on (cx, cy) and
    /// half-sized (hw, hh), all in canvas px relative to the centre. `refresh` re-grabs the template
    /// every N frames (0 = never: rigid features drift less without it). Err = nothing to track.
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        project: &Project,
        clip: Id,
        (cx, cy, hw, hh): (f32, f32, f32, f32),
        search: f32,
        refresh: u32,
        backward: bool,
        backend: Backend,
    ) -> Result<Self, String> {
        let c = project.clip(clip).ok_or("pick a clip to track")?;
        let path = project.asset(c.asset).map(|a| a.path.clone()).ok_or("that clip has no footage to track")?;
        let (w, h) = (project.width.max(2), project.height.max(2));
        let fps = project.fps.max(1.0);
        let n = ((c.duration * fps).round() as usize).clamp(1, 200_000);
        // clip-local time -> source time up front, so the worker never touches the Project
        let mut times: Vec<(f32, f64)> =
            (0..n).map(|i| (i as f64 / fps)).map(|lt| (lt as f32, c.src_time(c.start + lt))).collect();
        if backward {
            times.reverse();
        }
        let (hw, hh) = (hw.max(2.0) as i32, hh.max(2.0) as i32);
        let search = search.clamp(1.0, 512.0) as i32;
        let (x0, y0) = ((cx + w as f32 / 2.0) as i32, (cy + h as f32 / 2.0) as i32);
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let mut pool = DecoderPool::new(backend);
            let mut frame = Frame::default();
            let mut tpl: Vec<f32> = Vec::new();
            let (mut px, mut py) = (x0, y0);
            let total = times.len() as f32;
            for (i, (lt, st)) in times.iter().enumerate() {
                let Some(dec) = pool.video(&path) else { break };
                if !dec.frame_at(*st, w, h, &mut frame) {
                    break;
                }
                if tpl.is_empty() {
                    tpl = patch(&frame, px, py, hw, hh);
                } else {
                    if let Some((nx, ny, _)) = best_match(&frame, &tpl, hw, hh, px, py, search) {
                        (px, py) = (nx, ny);
                    }
                    if refresh > 0 && i % refresh as usize == 0 {
                        tpl = patch(&frame, px, py, hw, hh);
                    }
                }
                let pt = (px as f32 - w as f32 / 2.0, py as f32 - h as f32 / 2.0, *lt);
                if tx.send((pt, (i + 1) as f32 / total)).is_err() {
                    break; // the pane went away (cancel / project closed)
                }
            }
        });
        Ok(Self { rx, points: Vec::new(), progress: 0.0 })
    }

    /// Drain what the worker produced. True while it is still running (the caller keeps repainting).
    pub fn poll(&mut self) -> bool {
        loop {
            match self.rx.try_recv() {
                Ok((pt, p)) => {
                    self.points.push(pt);
                    self.progress = p;
                }
                Err(TryRecvError::Empty) => return true,
                Err(TryRecvError::Disconnected) => {
                    // backward tracking walks the clip in reverse; a path is always in time order
                    self.points.sort_by(|a, b| a.2.total_cmp(&b.2));
                    return false;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mid-grey frame with one bright square, so the patch is a real feature and not the only light.
    fn square(w: u32, h: u32, cx: i32, cy: i32, r: i32) -> Frame {
        let mut f = Frame::new(w, h);
        f.fill([40, 40, 40, 255]);
        for y in (cy - r).max(0)..=(cy + r).min(h as i32 - 1) {
            for x in (cx - r).max(0)..=(cx + r).min(w as i32 - 1) {
                let i = y as usize * f.stride() + x as usize * 4;
                f.rgba[i..i + 4].copy_from_slice(&[230, 220, 200, 255]);
            }
        }
        f
    }

    #[test]
    fn ncc_reports_the_offset_the_square_moved_by() {
        let a = square(96, 72, 40, 30, 6);
        let b = square(96, 72, 49, 25, 6);
        let tpl = patch(&a, 40, 30, 10, 10);
        let (x, y, score) = best_match(&b, &tpl, 10, 10, 40, 30, 16).expect("a bright square is not flat");
        assert_eq!((x - 40, y - 30), (9, -5), "score {score}");
        assert!(score > 0.9, "an exact patch should correlate ~1, got {score}");
        // a flat frame has nothing to lock onto: the caller keeps the previous position
        let flat = Frame::new(96, 72);
        assert!(best_match(&flat, &patch(&flat, 40, 30, 10, 10), 10, 10, 40, 30, 4).is_none());
    }
}
