//! URL import: download media from a link with `yt-dlp.exe`, the same child-process pattern
//! `ffpipe.rs` uses for ffmpeg. Entirely optional — `exe()` returns None when yt-dlp is not installed
//! and the Library then hides its "Import URL…" button.
//!
//! yt-dlp is driven with `--print after_move:filepath` (prints the final file, and because a later
//! stage is named it does not imply `--simulate`) plus `--progress --newline --progress-template` so
//! progress and the resulting path both arrive as lines on stdout.

use crate::engine::export::{self, Progress};
use crate::media::ffpipe;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};

static DIR: Mutex<String> = Mutex::new(String::new());
// checking every candidate's --version (below) is a handful of process spawns, so the result is cached
// until set_dir invalidates it — cheap enough at start-up, too slow to redo on every download.
static CACHE: Mutex<Option<Option<PathBuf>>> = Mutex::new(None);

/// Set the user-configured yt-dlp directory ("" = app dir, then PATH).
pub fn set_dir(dir: &str) {
    *DIR.lock().unwrap_or_else(|e| e.into_inner()) = dir.to_string();
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// The best working `yt-dlp.exe`, or None when none is installed (the URL import is hidden then).
///
/// Each candidate is verified with `--version`, because a PATH entry can hold a broken pip shim (one
/// left behind by an uninstalled Python exits 1 and prints nothing). Machines with more than one Python
/// install commonly have several *working* copies at very different ages — yt-dlp ships near-weekly to
/// keep up with site changes, so picking merely the first one found can land on a copy too stale to get
/// past current extractor/bot-check logic ("Sign in to confirm you're not a bot" and friends). The
/// version string sorts as "YYYY.MM.DD[.N]", so the newest wins.
pub fn exe() -> Option<PathBuf> {
    if let Some(cached) = CACHE.lock().unwrap_or_else(|e| e.into_inner()).clone() {
        return cached;
    }
    let dir = DIR.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let found = ffpipe::find_all_exe("yt-dlp.exe", &dir)
        .into_iter()
        .filter_map(|p| version(&p).map(|v| (p, v)))
        .max_by(|a, b| a.1.cmp(&b.1))
        .map(|(p, _)| p);
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = Some(found.clone());
    found
}

/// This exe's version string ("2026.08.19"), or None if it does not actually run (a broken shim exits
/// non-zero and prints nothing).
fn version(exe: &std::path::Path) -> Option<String> {
    let out = ffpipe::command(exe).arg("--version").stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !v.is_empty()).then_some(v)
}

/// Where downloads go by default: the user's Videos folder, else the temp dir.
pub fn default_dir() -> PathBuf {
    let videos = std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join("Videos"));
    match videos {
        Some(v) if v.is_dir() => v,
        _ => std::env::temp_dir(),
    }
}

#[derive(Clone, Debug)]
pub struct DownloadOptions {
    pub url: String,
    pub dir: PathBuf,
    /// Extract the audio only (needs ffmpeg, which yt-dlp is pointed at when we have it).
    pub audio_only: bool,
}

/// A running download. `progress` drives the UI (and `progress.cancel` kills yt-dlp); `path()` is the
/// downloaded file once `progress.is_done()` with no error.
pub struct Download {
    pub progress: Arc<Progress>,
    pub url: String,
    path: Arc<Mutex<Option<PathBuf>>>,
}

impl Download {
    pub fn path(&self) -> Option<PathBuf> {
        self.path.lock().ok().and_then(|p| p.clone())
    }
}

/// Percent from a `dl:` progress line ("dl:  45.2%" → 0.452). None for any other line.
fn parse_percent(line: &str) -> Option<f32> {
    let v = line.strip_prefix("dl:")?.trim().trim_end_matches('%').trim();
    // yt-dlp paints "  N/A%" before the size is known
    v.parse::<f32>().ok().map(|p| (p / 100.0).clamp(0.0, 1.0))
}

/// yt-dlp's own stderr, tacking on an update hint when the message is one it (or YouTube's bot-check)
/// prints for a stale extractor — these are real yt-dlp errors, not ours, but "install a newer yt-dlp"
/// is the actual fix often enough that it is worth saying plainly instead of leaving the user to guess.
fn annotate_if_outdated(err: &str) -> String {
    if err == export::CANCELLED {
        return err.to_string(); // not a failure — the caller reports it as a cancel, not an error
    }
    let lower = err.to_ascii_lowercase();
    const SIGNS: [&str; 4] =
        ["yt-dlp -u", "confirm you are on the latest version", "sign in to confirm", "please reload"];
    if SIGNS.iter().any(|s| lower.contains(s)) {
        let hint = "your yt-dlp looks outdated — update it and try again: \
            yt-dlp -U (or `pip install -U yt-dlp` if it came from pip)";
        format!("{err}\n\n{hint}")
    } else {
        err.to_string()
    }
}

pub fn start_download(opts: DownloadOptions) -> Download {
    let path: Arc<Mutex<Option<PathBuf>>> = Arc::new(Mutex::new(None));
    let url = opts.url.clone();
    let out = path.clone();
    let progress = export::spawn_job("ytdlp", move |prog| run(&opts, prog, &out));
    Download { progress, url, path }
}

fn run(opts: &DownloadOptions, prog: &Progress, out: &Mutex<Option<PathBuf>>) -> Result<(), String> {
    let exe = exe().ok_or("yt-dlp.exe not found")?;
    if opts.url.trim().is_empty() {
        return Err("No URL".into());
    }
    std::fs::create_dir_all(&opts.dir).map_err(|e| format!("{}: {e}", opts.dir.display()))?;
    prog.set(0.0, "Starting…");

    let mut cmd = ffpipe::command(&exe);
    #[rustfmt::skip]
    cmd.args([
        "--no-playlist",          // a link inside a playlist must not pull the whole playlist
        "--newline",              // one progress update per line
        "--progress",             // --print implies --quiet; keep the progress lines
        "--no-warnings",
        "--progress-template", "dl:%(progress._percent_str)s",
        "--print", "after_move:filepath",
        "-o", "%(title).100B [%(id)s].%(ext)s",
    ]);
    cmd.arg("-P").arg(&opts.dir);
    // let yt-dlp merge/extract with the ffmpeg we already located, even when it is not on PATH
    if let Some(dir) = ffpipe::ffmpeg_exe().and_then(|p| p.parent().map(PathBuf::from)) {
        cmd.arg("--ffmpeg-location").arg(dir);
    }
    if opts.audio_only {
        cmd.arg("-x");
    }
    cmd.arg(&opts.url);
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| format!("yt-dlp: {e}"))?;
    let tail = export::stderr_tail(&mut child);
    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if prog.is_cancelled() {
                break; // wait_ffmpeg kills the child and reports CANCELLED
            }
            let line = line.trim().to_string();
            match parse_percent(&line) {
                Some(f) => prog.set(f, format!("Downloading… {:.0}%", f * 100.0)),
                // any other line is the `--print` filepath (the last one wins: audio extraction
                // prints the container first, then the extracted file)
                None if !line.is_empty() => {
                    prog.set(prog.fraction().max(0.99), "Finishing…");
                    *out.lock().unwrap_or_else(|e| e.into_inner()) = Some(PathBuf::from(line));
                }
                None => {}
            }
        }
    }
    export::wait_ffmpeg(&mut child, tail, prog).map_err(|e| annotate_if_outdated(&e))?;

    let file = out.lock().unwrap_or_else(|e| e.into_inner()).clone();
    match file {
        Some(f) if f.is_file() => {
            prog.set(1.0, "Done");
            Ok(())
        }
        // yt-dlp exited 0 but printed no usable path (e.g. the file was already there under another name)
        _ => Err("yt-dlp finished but reported no output file".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_lines_parse() {
        assert!((parse_percent("dl:  45.2%").unwrap() - 0.452).abs() < 1e-6);
        assert!((parse_percent("dl:100.0%").unwrap() - 1.0).abs() < 1e-6);
        assert_eq!(parse_percent("dl: N/A%"), None);
        // a printed path is not progress
        assert_eq!(parse_percent(r"C:\Users\me\Videos\clip [abc].mp4"), None);
        assert_eq!(parse_percent(""), None);
    }

    #[test]
    fn outdated_yt_dlp_gets_an_update_hint() {
        let bot_check = "ERROR: [youtube] abc123: Sign in to confirm you're not a bot.";
        assert!(annotate_if_outdated(bot_check).contains("yt-dlp -U"));
        let stale_extractor =
            "ERROR: [youtube] abc123: some error; Confirm you are on the latest version using  yt-dlp -U";
        assert!(annotate_if_outdated(stale_extractor).contains("pip install -U yt-dlp"));
        // unrelated failures are passed through untouched
        let unrelated = "ERROR: [youtube] abc123: Video unavailable";
        assert_eq!(annotate_if_outdated(unrelated), unrelated);
        assert_eq!(annotate_if_outdated(export::CANCELLED), export::CANCELLED);
    }

    /// The whole download path against a LOCAL http server — no third-party site is contacted.
    /// Skips itself when yt-dlp, python or the test media are unavailable, so it is never flaky.
    #[test]
    fn local_http_download() {
        let Some(_) = exe() else { return };
        let media = std::env::temp_dir().join("simple-editor-selftest");
        if !media.join("test.mp4").is_file() {
            return; // `--selftest` has not generated the media on this machine
        }
        let port = 8700 + (std::process::id() % 200) as u16;
        let Ok(mut server) = std::process::Command::new("python")
            .args(["-m", "http.server", &port.to_string(), "--bind", "127.0.0.1"])
            .current_dir(&media)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            return; // no python on this machine
        };
        // wait for the port to accept connections (and give up rather than fail if it never does)
        let addr = format!("127.0.0.1:{port}");
        let mut up = false;
        for _ in 0..50 {
            if std::net::TcpStream::connect(&addr).is_ok() {
                up = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        if !up {
            let _ = server.kill();
            return;
        }
        let dir = std::env::temp_dir().join("se-ytdlp-test");
        let _ = std::fs::remove_dir_all(&dir);
        let d = start_download(DownloadOptions {
            url: format!("http://{addr}/test.mp4"),
            dir: dir.clone(),
            audio_only: false,
        });
        let mut seen_progress = false;
        for _ in 0..600 {
            if d.progress.fraction() > 0.0 {
                seen_progress = true;
            }
            if d.progress.is_done() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let _ = server.kill();
        let err = d.progress.error();
        let path = d.path();
        println!("done={} err={:?} path={:?} progress={}", d.progress.is_done(), err, path, seen_progress);
        assert!(d.progress.is_done() && err.is_none(), "err {err:?}");
        let p = path.expect("output path");
        assert!(p.is_file(), "{} missing", p.display());
        assert!(p.starts_with(&dir), "wrote outside the chosen folder: {}", p.display());
        println!("downloaded {} bytes to {}", std::fs::metadata(&p).unwrap().len(), p.display());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn broken_shim_is_rejected() {
        // an .exe that is not a program at all must not count as "installed"
        let dir = std::env::temp_dir().join(format!("se-ytdlp-broken-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("yt-dlp.exe");
        std::fs::write(&fake, b"not an executable").unwrap();
        assert!(version(&fake).is_none());
        set_dir(&dir.to_string_lossy());
        assert!(exe().is_none_or(|p| p != fake), "a broken shim was accepted");
        set_dir("");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_dir_exists() {
        assert!(default_dir().is_dir());
    }

    #[test]
    fn missing_url_fails_without_spawning() {
        if exe().is_none() {
            return; // yt-dlp not installed on this machine: nothing to check
        }
        let opts = DownloadOptions { url: "  ".into(), dir: std::env::temp_dir(), audio_only: false };
        let d = start_download(opts);
        for _ in 0..200 {
            if d.progress.is_done() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(d.progress.is_done());
        assert_eq!(d.progress.error().as_deref(), Some("No URL"));
        assert!(d.path().is_none());
    }
}
