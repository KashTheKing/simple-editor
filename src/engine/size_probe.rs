//! ---- ws:export-size ----
//! The Export pane's estimated file size, measured instead of guessed. A constant-quality encode's
//! size depends on the footage far more than on any formula (a slide deck and a voxel visualizer
//! differ 20x at the same CRF), so this encodes `SAMPLES` two-second pieces spread over the export
//! with the real settings - each one GOP long, like the final file's - and scales their total up.
//! One measurement runs at a time in the background; a new key cancels the old one, and it waits
//! while anything plays (`bg::pause_point`).

use crate::engine::export::{self, ExportOptions, Progress, CANCELLED};
use crate::engine::text::TextRasterizer;
use crate::model::Project;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const SAMPLES: usize = 5;
const SAMPLE_SECS: f64 = 2.0;

struct State {
    key: String,
    rev: String,
    bytes: Option<u64>,
    job: Option<Arc<Progress>>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn lock() -> std::sync::MutexGuard<'static, Option<State>> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// The encode settings a measurement depends on (the project's revision is passed apart, so the
/// Export pane can look a result up without knowing it).
pub fn key(o: &ExportOptions) -> String {
    format!(
        "{}|{}|{}|{}|{:?}|{:?}|{:?}|{:?}|{:?}",
        export::ext_of(&o.out_path),
        o.encoder,
        o.crf,
        o.preset,
        o.out_size,
        o.range,
        o.audio,
        o.stem_by,
        o.fps
    )
}

/// True when `key` at `rev` is measured or being measured.
pub fn current(key: &str, rev: &str) -> bool {
    lock().as_ref().is_some_and(|s| s.key == key && s.rev == rev)
}

/// The measured size for `key`, once its samples are encoded.
pub fn measured(key: &str) -> Option<u64> {
    lock().as_ref().filter(|s| s.key == key).and_then(|s| s.bytes)
}

/// Measure `opts` on `project` (at revision `rev`) in the background unless that is already measured
/// or measuring. Cheap to call every frame.
pub fn request(rev: String, project: &Project, opts: &ExportOptions, text: &Arc<Mutex<TextRasterizer>>) {
    let key = key(opts);
    if current(&key, &rev) {
        return;
    }
    let mut st = lock();
    if let Some(job) = st.as_ref().and_then(|s| s.job.as_ref()) {
        job.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    let project = crate::engine::zones::drawn(project); // ws:zones-markers: + browser::render_view
    let (opts, text, k, r) = (opts.clone(), text.clone(), key.clone(), rev.clone());
    let job = export::spawn_job("size-probe", move |prog| {
        // settle first: dragging the quality slider asks for a new key every frame
        for _ in 0..12 {
            if prog.is_cancelled() {
                return Err(CANCELLED.into());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let bytes = sample(&project, &opts, &text, prog)?;
        if let Some(s) = lock().as_mut().filter(|s| s.key == k && s.rev == r) {
            s.bytes = Some(bytes);
        }
        Ok(())
    });
    *st = Some(State { key, rev, bytes: None, job: Some(job) });
}

/// The sample windows `[a, b)` over `span`: the whole span when it is short, else `SAMPLES` evenly
/// spread pieces of `SAMPLE_SECS`.
pub fn windows(span: (f64, f64)) -> Vec<(f64, f64)> {
    let (a, b) = span;
    let dur = b - a;
    if dur <= SAMPLES as f64 * SAMPLE_SECS * 1.5 {
        return vec![(a, b)];
    }
    (0..SAMPLES)
        .map(|i| {
            let mid = a + dur * (i as f64 + 0.5) / SAMPLES as f64;
            (mid - SAMPLE_SECS / 2.0, mid + SAMPLE_SECS / 2.0)
        })
        .collect()
}

/// Encode the sample windows of `opts` to temp files and scale their total to the whole export.
pub fn sample(
    project: &Project,
    opts: &ExportOptions,
    text: &Arc<Mutex<TextRasterizer>>,
    prog: &Progress,
) -> Result<u64, String> {
    let span = opts.range.unwrap_or((0.0, project.duration()));
    let dir = std::env::temp_dir().join(format!("simple-editor-size-probe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let ext = export::ext_of(&opts.out_path);
    let (mut bytes, mut secs) = (0u64, 0f64);
    let mut r = Ok(());
    for (i, (a, b)) in windows(span).into_iter().enumerate() {
        crate::media::bg::pause_point(|| prog.is_cancelled());
        let out = dir.join(format!("s{i}.{ext}"));
        let o = ExportOptions {
            out_path: out.clone(),
            range: Some((a, b)),
            loudnorm: false, // no effect on size, and a second audio pass
            metadata: Vec::new(),
            target_bytes: None,
            alpha: false,
            audio: if opts.audio == export::AudioLayout::Stems { export::AudioLayout::Mix } else { opts.audio },
            ..opts.clone()
        };
        r = export::run_export(project, &o, text, prog);
        if r.is_err() {
            break;
        }
        bytes += std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
        secs += b - a;
    }
    let _ = std::fs::remove_dir_all(&dir);
    r?;
    Ok((bytes as f64 * (span.1 - span.0) / secs.max(0.001)) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_spread_over_the_span() {
        assert_eq!(windows((0.0, 10.0)), vec![(0.0, 10.0)], "short exports are measured whole");
        let w = windows((0.0, 100.0));
        assert_eq!(w.len(), SAMPLES);
        assert_eq!(w[0], (9.0, 11.0));
        assert_eq!(w[4], (89.0, 91.0));
        assert!(w.iter().all(|(a, b)| (b - a - SAMPLE_SECS).abs() < 1e-9));
    }
}
