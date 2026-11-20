//! ---- ws:forum-preview ----
//! Session D's bench clips (plans/shotcut-forum-insights.md, problem 4): a 3-hour H.264 file, a 4K60
//! 10-bit HEVC phone clip and a VFR screen recording, made with ffmpeg in the temp folder. For each, on
//! the original and on its proxy: open time, 20 random seeks, a 10-frame backwards scrub and the first
//! frame of a "next clip" (start-of-clip lag). Budgets, on the proxies (what playback uses once the
//! proposal is accepted): worst seek and worst scrub step under 50 ms, first frame under 150 ms.
//! The originals are printed, not gated (that is the point of the proxy).
//!
//! `cargo test bench_forum_clips -- --ignored --nocapture` (dev build; generating the 3-hour file takes a
//! few minutes the first time, and it is kept in the temp folder for the next run).

use crate::media::{ffpipe, open_video, Backend, Frame};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn make(ff: &Path, out: &Path, input: &str, dur: f64, enc: &[&str]) -> bool {
    if out.exists() {
        return true;
    }
    let tmp = out.with_extension("part.mp4");
    let ok = ffpipe::command(ff)
        .args(["-y", "-v", "error", "-f", "lavfi", "-i", input, "-t", &format!("{dur}")])
        .args(enc)
        .arg(&tmp)
        .status()
        .is_ok_and(|s| s.success());
    ok && std::fs::rename(&tmp, out).is_ok()
}

fn proxy_of(ff: &Path, src: &Path) -> Option<PathBuf> {
    let out = src.with_extension("proxy.mp4");
    if out.exists() {
        return Some(out);
    }
    let s = src.to_string_lossy();
    let ok = ffpipe::command(ff)
        .args(["-y", "-v", "error", "-i"])
        .arg(src)
        .args(["-vf", "scale=-2:720"])
        .args(crate::media::proxy::ENCODE)
        .args(crate::media::proxy::cfr_args(&s))
        .arg(&out)
        .status()
        .is_ok_and(|st| st.success());
    ok.then_some(out)
}

#[derive(Debug, Default)]
struct Times {
    open: Duration,
    seek_worst: Duration,
    seek_avg: Duration,
    scrub_worst: Duration,
    first_frame: Duration,
}

fn measure(path: &Path, dur: f64, fps: f64) -> Option<Times> {
    let p = path.to_string_lossy().into_owned();
    let t0 = Instant::now();
    let mut v = open_video(&p, Backend::Auto).ok()?;
    let mut t = Times { open: t0.elapsed(), ..Default::default() };
    let (w, h) = (640, 360);
    let mut f = Frame::default();
    // start-of-clip: a fresh decoder's first frame somewhere inside (a clip that starts mid-file)
    let t1 = Instant::now();
    v.frame_at(dur * 0.37, w, h, &mut f);
    t.first_frame = t.open + t1.elapsed();
    let mut seed = 0x9e37_79b9_u64;
    let mut total = Duration::ZERO;
    for _ in 0..20 {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let at = (seed % 10_000) as f64 / 10_000.0 * (dur - 1.0).max(0.0);
        let s = Instant::now();
        v.frame_at(at, w, h, &mut f);
        let d = s.elapsed();
        total += d;
        t.seek_worst = t.seek_worst.max(d);
    }
    t.seek_avg = total / 20;
    let mut at = dur * 0.5;
    for _ in 0..10 {
        at -= 1.0 / fps;
        let s = Instant::now();
        v.frame_at(at, w, h, &mut f);
        t.scrub_worst = t.scrub_worst.max(s.elapsed());
    }
    Some(t)
}

#[test]
#[ignore]
fn bench_forum_clips() {
    let (Some(ff), Some(_)) = (ffpipe::ffmpeg_exe(), ffpipe::ffprobe_exe()) else {
        return eprintln!("skipped: no ffmpeg / ffprobe");
    };
    let dir = std::env::temp_dir().join("se-forum-bench");
    std::fs::create_dir_all(&dir).unwrap();
    let x264 = ["-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p"];
    let clips: Vec<(&str, PathBuf, f64, f64, bool)> = vec![
        (
            "3-hour H.264",
            dir.join("three-hours.mp4"),
            3.0 * 3600.0,
            30.0,
            make(&ff, &dir.join("three-hours.mp4"), "testsrc2=s=640x360:r=30", 3.0 * 3600.0, &[&x264[..], &["-g", "250"]].concat()),
        ),
        (
            "4K60 HEVC phone",
            dir.join("phone-4k60.mp4"),
            10.0,
            60.0,
            make(
                &ff,
                &dir.join("phone-4k60.mp4"),
                "testsrc2=s=3840x2160:r=60",
                10.0,
                &["-c:v", "libx265", "-preset", "ultrafast", "-pix_fmt", "yuv420p10le", "-tag:v", "hvc1"],
            ),
        ),
        (
            "VFR screen recording",
            dir.join("screen-vfr.mp4"),
            60.0,
            60.0,
            make(
                &ff,
                &dir.join("screen-vfr.mp4"),
                "testsrc2=s=1920x1080:r=60,select='not(eq(mod(n\\,5)\\,2))*not(eq(mod(n\\,7)\\,3))'",
                60.0,
                &[&x264[..], &["-fps_mode", "passthrough"]].concat(),
            ),
        ),
    ];
    let mut failed = Vec::new();
    for (name, path, dur, fps, made) in clips {
        if !made {
            eprintln!("{name}: could not be made with this ffmpeg (no libx265?) - skipped");
            continue;
        }
        let s = path.to_string_lossy().into_owned();
        crate::media::playcheck::remember(&s, crate::media::playcheck::probe(&s));
        let orig = measure(&path, dur, fps);
        eprintln!("{name} original: {orig:?}");
        let Some(proxy) = proxy_of(&ff, &path) else {
            failed.push(format!("{name}: the proxy encode failed"));
            continue;
        };
        let Some(p) = measure(&proxy, dur, fps) else {
            failed.push(format!("{name}: the proxy did not open"));
            continue;
        };
        eprintln!("{name} proxy:    {p:?}");
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        if ms(p.seek_worst) > 50.0 || ms(p.scrub_worst) > 50.0 || ms(p.first_frame) > 150.0 {
            failed.push(format!("{name} proxy over budget: {p:?}"));
        }
    }
    assert!(failed.is_empty(), "{failed:#?}");
}
