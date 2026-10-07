//! Proxy media: background all-intra low-res transcodes of imported video, played instead of the
//! originals in the preview. Every proxy frame is a keyframe (`-g 1`), so seeks, reverse scrubs and
//! clip-boundary cold opens decode without reference chains - the reason big NLEs feel instant.
//! Proxies live in the cache dir, named by a hash of (source path, mtime, height): a re-exported
//! source gets a fresh proxy automatically and stale files are just never referenced again.
//! Export and full-quality one-shot renders never see proxies (their DecoderPools carry no map).
//!
//! A source with an alpha channel (a PNG/qtrle/ProRes 4444 `.mov` used as a wipe or overlay) gets a
//! STACKED proxy, named `<hash>-a.mp4`: H.264 has no alpha, so the frame is twice as tall - colour
//! (premultiplied) on top, the alpha plane as grey below - and `StackedAlpha` folds the two halves
//! back into one straight-alpha frame at decode. It stays an all-intra H.264 file Media Foundation
//! reads, so it seeks and plays like any other proxy (decoding the source itself, PNG frames through
//! an ffmpeg pipe, is ~36 ms a 1080p frame).
//! ponytail: a stacked proxy is twice the proxy height - past a 1080 setting that is taller than
//! Media Foundation's H.264 decoder takes, and the proxy plays through the ffmpeg pipe instead
//! (correct, slow). Cap the height of stacked proxies if anyone previews alpha footage that big.

use crate::engine::export::{self, Progress};
use crate::media::{ffpipe, Frame, VideoSource};
use std::collections::BTreeMap;
use std::io::BufRead;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex, MutexGuard};

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The proxy build currently running: (source path, fraction 0..1). Owned entirely by the build job
/// (set on entry, fraction updated from ffmpeg progress, cleared by a drop guard so an error or
/// panic can never leave a stuck "building" badge). Read from UI leaf code - library rows, the
/// inspector's Asset block, the preview badge - which has no channel to `App` state; same shape as
/// `engine::import::is_probing`.
static BUILDING: Mutex<Option<(String, f32)>> = Mutex::new(None);
/// Source paths (lowercased) whose proxy is built and in use, as last pushed by `App::sync_proxies`
/// - pushed in the same breath as the player's proxy map, so badges and playback agree.
static READY: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// What the proxy pipeline is doing for one asset - drives the per-asset badges.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ProxyStatus {
    /// Not video, already at/below proxy height, zero-length, or proxies are off.
    NotNeeded,
    /// Eligible, waiting its turn (builds run one at a time).
    Queued,
    /// The transcode running right now (fraction 0..1).
    Building(f32),
    Ready,
}

/// The build in flight, if any: (source path, fraction 0..1).
pub fn building() -> Option<(String, f32)> {
    lock(&BUILDING).clone()
}

/// `App::sync_proxies` publishes which sources currently play from a proxy (call sites: the normal
/// scan push and the regenerate path - both, or badges lie for up to one 2 s scan).
pub fn set_ready(sources: Vec<String>) {
    *lock(&READY) = sources.into_iter().map(|s| s.to_ascii_lowercase()).collect();
}

/// Per-asset proxy state. The eligibility gate mirrors `App::sync_proxies` verbatim so the badge
/// and the builder can't drift.
pub fn status(a: &crate::model::Asset, use_proxies: bool, proxy_height: u32) -> ProxyStatus {
    let h = proxy_height.max(120);
    if !use_proxies || a.kind != crate::model::ClipKind::Video || a.height <= h || a.duration <= 0.0 {
        return ProxyStatus::NotNeeded;
    }
    if let Some((p, f)) = lock(&BUILDING).as_ref() {
        if p.eq_ignore_ascii_case(&a.path) {
            return ProxyStatus::Building(*f);
        }
    }
    if lock(&READY).iter().any(|r| r == &a.path.to_ascii_lowercase()) {
        return ProxyStatus::Ready;
    }
    ProxyStatus::Queued
}

/// Clears the BUILDING badge when the job ends, however it ends (ok / ffmpeg error / panic).
struct BuildingGuard(String);
impl BuildingGuard {
    fn set(src: &str) -> Self {
        *lock(&BUILDING) = Some((src.to_string(), 0.0));
        BuildingGuard(src.to_string())
    }
}
impl Drop for BuildingGuard {
    fn drop(&mut self) {
        let mut b = lock(&BUILDING);
        if b.as_ref().is_some_and(|(p, _)| *p == self.0) {
            *b = None;
        }
    }
}

fn set_building_fraction(src: &str, f: f32) {
    if let Some((p, frac)) = lock(&BUILDING).as_mut() {
        if p == src {
            *frac = f;
        }
    }
}

pub fn dir() -> PathBuf {
    crate::settings::Settings::cache_dir().join("proxies")
}

fn mtime_secs(src: &str) -> u64 {
    std::fs::metadata(src)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Pixel formats that carry alpha. `pal8` counts: a palette may hold transparent entries (GIF,
/// qtrle), and guessing wrong only costs a taller proxy.
fn pix_fmt_has_alpha(pix_fmt: &str) -> bool {
    let p = pix_fmt.trim();
    p.starts_with("yuva")
        || p.starts_with("gbrap")
        || p.starts_with("ya")
        || p == "pal8"
        || ["rgba", "bgra", "argb", "abgr"].iter().any(|f| p.starts_with(f))
}

/// One ffprobe run: does the first video stream of `src` have an alpha channel?
fn probe_alpha(src: &str) -> bool {
    let Some(exe) = ffpipe::ffprobe_exe() else { return false };
    let args = ["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=pix_fmt", "-of", "csv=p=0"];
    let out = ffpipe::command(&exe).args(args).arg(src).stdin(Stdio::null()).output();
    out.is_ok_and(|o| String::from_utf8_lossy(&o.stdout).lines().any(pix_fmt_has_alpha))
}

/// Does `src` have an alpha channel? Probed once per (path, mtime) on a thread of its own - this is
/// asked from the UI thread's proxy scan, and an ffprobe run is ~50 ms a file: `None` until the
/// answer is in (the scan skips the asset and asks again 2 s later).
pub fn has_alpha(src: &str) -> Option<bool> {
    // containers that only hold codecs without alpha: no probe
    let ext = crate::media::ext(src);
    if matches!(ext.as_str(), "mp4" | "m4v" | "ts" | "mts" | "m2ts" | "mpg" | "mpeg" | "wmv" | "flv" | "3gp") {
        return Some(false);
    }
    static KNOWN: Mutex<BTreeMap<(String, u64), Option<bool>>> = Mutex::new(BTreeMap::new());
    static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());
    let key = (src.to_ascii_lowercase(), mtime_secs(src));
    let mut known = lock(&KNOWN);
    if let Some(known) = known.get(&key) {
        return *known;
    }
    known.insert(key.clone(), None);
    drop(known);
    let src = src.to_string();
    std::thread::spawn(move || {
        let _one = lock(&ONE_AT_A_TIME); // a library of 200 clips must not be 200 ffprobe.exe at once
        let alpha = probe_alpha(&src);
        lock(&KNOWN).insert(key, Some(alpha));
    });
    None
}

/// Is `path` a stacked colour-over-alpha proxy (see the module docs)? By name and folder, so a
/// user's own `take-a.mp4` is never mistaken for one.
pub fn is_stacked(path: &std::path::Path) -> bool {
    path.file_name().is_some_and(|n| n.to_string_lossy().ends_with("-a.mp4"))
        && path.parent().and_then(|d| d.file_name()).is_some_and(|d| d == "proxies")
}

/// Where the proxy for `src` at `height` lives (whether or not it has been built yet). A source
/// with alpha gets the `-a` (stacked) name, so a plain proxy built for it before proxies kept alpha
/// is simply never referenced again - `run` deletes it when the stacked one is built.
pub fn proxy_path(src: &str, height: u32) -> PathBuf {
    stacked_or_plain(src, height, has_alpha(src) == Some(true))
}

fn stacked_or_plain(src: &str, height: u32, stacked: bool) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (src.to_ascii_lowercase(), mtime_secs(src), height).hash(&mut h);
    dir().join(format!("{:016x}{}.mp4", h.finish(), if stacked { "-a" } else { "" }))
}

/// 255 * 65536 / a, rounded: un-premultiplying is a multiply and a shift per channel.
const RECIP: [u32; 256] = {
    let mut t = [0u32; 256];
    let mut a = 1;
    while a < 256 {
        t[a] = (255 * 65536 + a as u32 / 2) / a as u32;
        a += 1;
    }
    t
};

/// Decodes a stacked proxy (premultiplied colour over grey alpha, see the module docs) as the
/// straight-alpha frame the compositors expect.
pub struct StackedAlpha {
    inner: Box<dyn VideoSource>,
    both: Frame,
}

impl StackedAlpha {
    pub fn new(inner: Box<dyn VideoSource>) -> Self {
        Self { inner, both: Frame::default() }
    }
}

impl VideoSource for StackedAlpha {
    fn size(&self) -> (u32, u32) {
        let (w, h) = self.inner.size();
        (w, h / 2)
    }
    fn frame_at(&mut self, t: f64, w: u32, h: u32, out: &mut Frame) -> bool {
        // always the whole stacked frame: a decoder asked for a smaller one scales it, and its scaler
        // blends the rows either side of the seam - the top of the picture picks up alpha from the
        // bottom's brightness, a line across the preview
        let (sw, sh) = self.size();
        if w == 0 || h == 0 || !self.inner.frame_at(t, sw, sh * 2, &mut self.both) {
            return false;
        }
        out.resize(w, h);
        out.pts = self.both.pts;
        let (sw, sh) = (sw as usize, sh as usize);
        let (colour, alpha) = self.both.rgba.split_at(sw * sh * 4);
        // ponytail: nearest neighbour below the proxy's own size (a small preview pane) - colour and
        // alpha stay paired, so no fringes; a box filter over both halves if the shimmer ever shows
        let xs: Vec<usize> = (0..w as usize).map(|x| x * sw / w as usize * 4).collect();
        for (y, row) in out.rgba.chunks_exact_mut(w as usize * 4).enumerate() {
            let at = (y * sh / h as usize) * sw * 4;
            let (colour, alpha) = (&colour[at..at + sw * 4], &alpha[at..at + sw * 4]);
            // indexed on purpose: this crate is built for size, and the iterator version of this
            // loop cost as much as decoding the frame
            for i in 0..xs.len() {
                let (x, d) = (xs[i], &mut row[i * 4..i * 4 + 4]);
                // the ends snap: H.264 and the limited-range round trip leave a flat 0 or 255 a few
                // levels off, and an "opaque" 252 would let the layer below glow through
                let a = alpha[x + 1];
                if a >= 250 {
                    (d[0], d[1], d[2], d[3]) = (colour[x], colour[x + 1], colour[x + 2], 255);
                } else if a <= 5 {
                    (d[0], d[1], d[2], d[3]) = (0, 0, 0, 0);
                } else {
                    let k = RECIP[a as usize];
                    for j in 0..3 {
                        d[j] = ((colour[x + j] as u32 * k) >> 16).min(255) as u8;
                    }
                    d[3] = a;
                }
            }
        }
        true
    }
}

/// Transcode `src` into its proxy file on a background thread (temp + rename; the destination never
/// exists half-written). Video only - audio always plays from the original.
pub fn generate(src: String, dst: PathBuf, height: u32) -> Arc<Progress> {
    export::spawn_job("proxy", move |prog| {
        let _badge = BuildingGuard::set(&src); // set + cleared inside the job: no startup race
        run(&src, &dst, height, prog)
    })
}

fn run(src: &str, dst: &PathBuf, height: u32, prog: &Progress) -> Result<(), String> {
    let ffmpeg = ffpipe::ffmpeg_exe().ok_or("ffmpeg.exe not found")?;
    std::fs::create_dir_all(dir()).map_err(|e| format!("proxies dir: {e}"))?;
    let dur = crate::engine::convert::probe_seconds(std::path::Path::new(src)).unwrap_or(0.0);
    let tmp = export::temp_output(dst);
    let mut cmd = ffpipe::command(&ffmpeg);
    cmd.args(["-y", "-hide_banner", "-loglevel", "error", "-progress", "pipe:1"]);
    cmd.arg("-i").arg(src);
    // -2 keeps aspect at an even width; -g 1 = all-intra; no audio (the original supplies it)
    let mut vf = format!("scale=-2:{}", height.max(120));
    if is_stacked(dst) {
        // premultiplied BEFORE the downscale (scaling straight alpha drags the colour hidden under
        // transparent pixels - usually black - into every edge: dark fringes), then colour over alpha
        vf = format!(
            "format=gbrap,premultiply=inplace=1,{vf},format=gbrap,split[c][a];\
             [a]alphaextract,format=yuv420p[m];[c]format=yuv420p[k];[k][m]vstack"
        );
        // the proxy this source got before proxies kept alpha: same name without the `-a`
        let _ = std::fs::remove_file(stacked_or_plain(src, height, false));
    }
    cmd.args(["-vf", &vf, "-c:v", "libx264", "-preset", "veryfast", "-g", "1", "-crf", "20"]);
    cmd.args(["-pix_fmt", "yuv420p", "-an", "-movflags", "+faststart"]);
    cmd.arg(&tmp.0);
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("ffmpeg: {e}"))?;
    let tail = export::stderr_tail(&mut child);
    if let Some(out) = child.stdout.take() {
        prog.set(0.0, "Building proxy…");
        for line in std::io::BufReader::new(out).lines() {
            let Ok(line) = line else { break };
            if prog.is_cancelled() {
                let _ = child.kill();
                break;
            }
            if let Some(us) = line.strip_prefix("out_time_us=").and_then(|v| v.trim().parse::<f64>().ok()) {
                let f = if dur > 0.0 { (us / 1e6 / dur).clamp(0.0, 1.0).min(0.99) as f32 } else { 0.0 };
                prog.set(f, "Building proxy…");
                set_building_fraction(src, f);
            }
        }
    }
    export::wait_ffmpeg(&mut child, tail, prog)?;
    tmp.commit(dst.as_path())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One test for the whole ladder (BUILDING/READY are process-global statics - keeping every
    /// assertion in one test avoids parallel-test interference on them).
    #[test]
    fn status_classifies_the_pipeline() {
        use crate::model::{Asset, ClipKind};
        let a = Asset {
            id: 1,
            path: "Z:\\proxy-status-test.mp4".into(),
            kind: ClipKind::Video,
            duration: 10.0,
            width: 3840,
            height: 2160,
            fps: 30.0,
            audio_streams: Vec::new(),
            codec: "hevc".into(),
            folder: String::new(),
            tags: Vec::new(),
            label: 0,
            description: String::new(),
            rel_path: None,
            parent: None,
            range: None,
            effects: Vec::new(),
        };
        assert_eq!(status(&a, false, 720), ProxyStatus::NotNeeded, "proxies off");
        let small = Asset { height: 720, ..a.clone() };
        assert_eq!(status(&small, true, 720), ProxyStatus::NotNeeded, "at/below proxy height");
        let audio = Asset { kind: ClipKind::Audio, ..a.clone() };
        assert_eq!(status(&audio, true, 720), ProxyStatus::NotNeeded, "audio never proxies");
        assert_eq!(status(&a, true, 720), ProxyStatus::Queued, "eligible, not building, not ready");

        {
            let _g = BuildingGuard::set(&a.path);
            set_building_fraction(&a.path, 0.25);
            assert_eq!(status(&a, true, 720), ProxyStatus::Building(0.25));
        }
        assert_eq!(status(&a, true, 720), ProxyStatus::Queued, "the drop guard cleared the badge");

        set_ready(vec![a.path.to_uppercase()]);
        assert_eq!(status(&a, true, 720), ProxyStatus::Ready, "ready is case-insensitive");
        set_ready(Vec::new());
        assert_eq!(status(&a, true, 720), ProxyStatus::Queued);
    }

    #[test]
    fn alpha_pix_fmts() {
        for p in ["rgba", "argb", "bgra", "abgr", "yuva444p10le", "gbrap", "gbrap12le", "ya8", "pal8", "rgba64be\r"] {
            assert!(pix_fmt_has_alpha(p), "{p}");
        }
        for p in ["yuv420p", "yuv422p10le", "rgb24", "gbrp", "gray", "nv12", ""] {
            assert!(!pix_fmt_has_alpha(p), "{p}");
        }
        assert_eq!(has_alpha("C:\\missing\\clip.mp4"), Some(false), "mp4 is answered without a probe");
    }

    /// The reported bug, end to end: a `.mov` with an alpha channel previewed through its proxy lost
    /// its transparency (the proxy was plain yuv420p) and showed on black. Its proxy is now the
    /// stacked one, the plain one it had before is deleted, and the preview decode composites it.
    #[test]
    fn alpha_source_keeps_its_transparency_through_the_proxy() {
        use crate::media::{Backend, DecoderPool};
        let dir = std::env::temp_dir().join(format!("simple-editor-alpha-proxy-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        // left half transparent (over white - what a careless decode would show), right half blue
        let mov = dir.join("wipe.mov");
        let geq = "format=rgba,geq=r=255*lt(X\\,W/2):g=255*lt(X\\,W/2):b=255:a=255*gte(X\\,W/2)";
        let made = std::process::Command::new("ffmpeg")
            .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i", "color=white:s=640x480:d=1:r=10"])
            .args(["-vf", geq, "-c:v", "png"])
            .arg(&mov)
            .status()
            .expect("ffmpeg on PATH");
        assert!(made.success(), "ffmpeg failed to generate the alpha clip");
        let src = mov.to_string_lossy().into_owned();

        // detection: unknown while the probe runs, then alpha
        let t0 = std::time::Instant::now();
        while has_alpha(&src).is_none() {
            assert!(t0.elapsed().as_secs() < 20, "alpha probe never finished");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(has_alpha(&src), Some(true));

        // invalidation: the stacked proxy has its own name, and building it deletes the old one
        let (old, dst) = (stacked_or_plain(&src, 240, false), proxy_path(&src, 240));
        assert!(is_stacked(&dst) && dst != old, "{dst:?}");
        assert!(!is_stacked(&old) && !is_stacked(&mov));
        std::fs::create_dir_all(super::dir()).unwrap();
        std::fs::write(&old, b"a proxy from before proxies kept alpha").unwrap();
        run(&src, &dst, 240, &Progress::new()).expect("build the proxy");
        assert!(dst.exists() && !old.exists(), "the stale plain proxy is removed");

        // over a red clip, red shows through the left half - in the preview (a pool with the proxy
        // map) and in an export (a pool without one, reading the source) alike
        let red = crate::media::ffpipe::tests::test_mp4(); // red 0-2 s, 320x240
        let mut project = crate::model::Project::from_media(crate::media::probe(&red, Backend::Auto).unwrap());
        let aid = project.add_asset(crate::media::probe(&src, Backend::Auto).unwrap());
        project.insert_asset_clips(aid, 0.0, None);
        let mut pool = DecoderPool::new(Backend::Auto);
        pool.set_proxies([(src.clone(), dst.to_string_lossy().into_owned())].into());
        assert_eq!(pool.video(&src).map(|v| v.size()), Some((320, 240)), "half of the stacked 320x480");
        for (what, pool) in [("preview", &mut pool), ("export", &mut DecoderPool::new(Backend::Auto))] {
            let mut out = Frame::default();
            let mut text = crate::engine::text::TextRasterizer::new();
            crate::engine::compose::Compositor::new().render(&project, 0.5, 320, 240, pool, &mut text, &mut out);
            let px = |x: usize, y: usize| <[u8; 3]>::try_from(&out.rgba[(y * 320 + x) * 4..][..3]).unwrap();
            for (x, y) in [(40, 30), (80, 120), (150, 230)] {
                let [r, g, b] = px(x, y);
                assert!(r > 200 && g < 50 && b < 50, "{what}: the red clip below shows at {x},{y}: {:?}", [r, g, b]);
            }
            for (x, y) in [(170, 10), (240, 120), (300, 230)] {
                let [r, g, b] = px(x, y);
                assert!(r < 50 && g < 50 && b > 200, "{what}: the clip itself shows at {x},{y}: {:?}", [r, g, b]);
            }
            // across the edge red turns into blue and nothing else: no dark (or white) fringe
            for x in 150..170 {
                let [r, g, b] = px(x, 120);
                assert!(r as u32 + b as u32 > 200 && g < 60, "{what}: fringe at {x}: {:?}", [r, g, b]);
            }
        }
        drop(pool); // the decoder holds the proxy open
        let _ = std::fs::remove_file(&dst);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Decode cost of a stacked 720p proxy against a plain one, at full size and at the 5/8 a small
    /// preview asks for; and no bleed across the seam between the halves when the decoder scales.
    /// `cargo test bench_stacked_alpha_proxy -- --ignored --nocapture` (SE_ALPHA_SRC = your own clip)
    #[test]
    #[ignore]
    fn bench_stacked_alpha_proxy() {
        use crate::media::{open_video, Backend};
        let dir = std::env::temp_dir().join("se-bench").join("proxies");
        let _ = std::fs::create_dir_all(&dir);
        let own = std::env::var("SE_ALPHA_SRC").ok();
        let mov = dir.join("alpha1080.mov");
        if own.is_none() && !mov.exists() {
            // transparent top half over an opaque white bottom half: the worst case at the seam
            let geq = "format=rgba,geq=r=255:g=255:b=255:a=255*gte(Y\\,H/2)";
            let st = std::process::Command::new("ffmpeg")
                .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i", "color=white:s=1920x1080:d=2:r=60"])
                .args(["-vf", geq, "-c:v", "png"])
                .arg(&mov)
                .status();
            assert!(st.is_ok_and(|s| s.success()), "ffmpeg failed");
        }
        let src = own.clone().unwrap_or(mov.to_string_lossy().into_owned());
        let (plain, stacked) = (dir.join("bench.mp4"), dir.join("bench-a.mp4"));
        for dst in [&plain, &stacked] {
            run(&src, dst, 720, &Progress::new()).expect("proxy");
        }
        for (name, path) in [("plain", &plain), ("stacked", &stacked), ("stacked, undivided", &stacked)] {
            let mut v = open_video(&path.to_string_lossy(), Backend::Auto).expect("open");
            if name.ends_with("undivided") {
                v = crate::media::mf::open_video(&path.to_string_lossy()).expect("open");
            }
            let (w, h) = v.size();
            for (w, h) in [(w, h), (w * 5 / 8, h * 5 / 8)] {
                let mut f = Frame::default();
                let mut ms: Vec<f64> = (0..120)
                    .map(|i| {
                        let t0 = std::time::Instant::now();
                        assert!(v.frame_at(i as f64 / 60.0 % 1.0, w, h, &mut f));
                        t0.elapsed().as_secs_f64() * 1e3
                    })
                    .collect();
                ms.drain(..5); // the first frames pay for a seek and a size negotiation
                ms.sort_by(|a, b| a.total_cmp(b));
                eprintln!("{name} {w}x{h}: median {:.2} ms, max {:.2} ms a frame", ms[ms.len() / 2], ms[ms.len() - 1]);
                if name == "stacked" && own.is_none() {
                    let row = |y: u32| f.rgba[(y * w * 4) as usize..][..(w * 4) as usize].to_vec();
                    assert!(row(0).chunks(4).all(|p| p[3] == 0), "top row stays transparent at {w}x{h}");
                    let last = row(h - 1);
                    assert!(
                        last.chunks(4).all(|p| p[3] == 255 && p[0] > 240),
                        "bottom row stays white: {:?}",
                        &last[..4]
                    );
                }
            }
        }
    }

    /// The proxy name is stable for the same (path, mtime, height) and changes with any of them.
    #[test]
    fn proxy_path_is_deterministic() {
        let a = proxy_path("C:\\missing\\clip.mp4", 720);
        assert_eq!(a, proxy_path("C:\\missing\\clip.mp4", 720));
        assert_eq!(a, proxy_path("C:\\MISSING\\CLIP.mp4", 720), "case-insensitive paths hash alike");
        assert_ne!(a, proxy_path("C:\\missing\\clip.mp4", 540));
        assert_ne!(a, proxy_path("C:\\missing\\other.mp4", 720));
        assert!(a.extension().is_some_and(|e| e == "mp4"));
    }
}
