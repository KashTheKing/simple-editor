//! FFmpeg and the optional add-ons: where each comes from, where the app looks for it, and installing one.
//! Every download is checksum-verified: a pinned SHA-256 from the library manifest, or the digest GitHub's
//! release API reports for the asset.

use crate::json;
use crate::net::{self, Net};
use crate::paths::Paths;
use crate::report::{mb, Report};
use crate::zip;
use std::path::{Path, PathBuf};

pub enum Src {
    /// The latest release asset of a GitHub repo (verified against the API's digest).
    GitHub { repo: &'static str, asset: &'static str },
    /// A library manifest (`library/bin/*.json`: url, size, sha256), compiled in.
    Manifest(&'static str),
}

pub enum Unpack {
    /// The download is the program itself.
    File(&'static str),
    /// A zip: the folder holding `main` becomes the add-on's folder (less `junk`).
    Folder { main: &'static str, junk: &'static [&'static str] },
    /// A zip: just these files, wherever they are inside it.
    Pick(&'static [&'static str]),
}

pub struct AddOn {
    pub id: &'static str,
    /// The checklist line: "name - what it is for".
    pub label: &'static str,
    pub src: Src,
    pub unpack: Unpack,
    /// Where it goes.
    pub dest: fn(&Paths) -> PathBuf,
    /// Write the manifest beside it under this name (the app reads the installed version from it).
    pub keep_manifest: Option<&'static str>,
}

pub const FFMPEG_JSON: &str = include_str!("../../../library/bin/ffmpeg.json");
const SE_HTML_JSON: &str = include_str!("../../../library/bin/se-html.json");
const ENHANCER_JSON: &str = include_str!("../../../library/bin/enhancer.json");

/// FFmpeg: ffmpeg.exe + ffprobe.exe into `<install>\ffmpeg`, the folder the app checks after its own.
pub const FFMPEG: AddOn = AddOn {
    id: "ffmpeg",
    label: "Install FFmpeg",
    src: Src::Manifest(FFMPEG_JSON),
    unpack: Unpack::Pick(&["ffmpeg.exe", "ffprobe.exe"]),
    dest: |p| p.install.join("ffmpeg"),
    keep_manifest: None,
};

/// The optional add-ons, in checklist order.
pub const ADDONS: &[AddOn] = &[
    AddOn {
        id: "yt-dlp",
        label: "yt-dlp - Download videos from a URL",
        src: Src::GitHub { repo: "yt-dlp/yt-dlp", asset: "yt-dlp.exe" },
        unpack: Unpack::File("yt-dlp.exe"),
        dest: |p| p.install.clone(),
        keep_manifest: None,
    },
    AddOn {
        id: "whisper",
        label: "whisper.cpp - Transcribe speech into captions",
        src: Src::GitHub { repo: "ggml-org/whisper.cpp", asset: "whisper-bin-x64.zip" },
        unpack: Unpack::Folder { main: "whisper-cli.exe", junk: &[] },
        dest: |p| p.cache().join("whisper"),
        keep_manifest: None,
    },
    AddOn {
        id: "se-html",
        label: "Fast HTML renderer - Draws HTML and CSS layers quickly",
        src: Src::Manifest(SE_HTML_JSON),
        unpack: Unpack::File("se-html.exe"),
        dest: |p| p.bin(),
        keep_manifest: Some("se-html.json"),
    },
    AddOn {
        id: "enhancer",
        label: "AI enhancer - Upscale clips on the GPU",
        src: Src::Manifest(ENHANCER_JSON),
        unpack: Unpack::Folder { main: "realesrgan-ncnn-vulkan.exe", junk: &["onepiece_demo.mp4", "input.jpg", "input2.jpg"] },
        dest: |p| p.bin().join("enhancer"),
        keep_manifest: Some("enhancer.json"),
    },
];

#[cfg(test)]
pub fn by_id(id: &str) -> Option<&'static AddOn> {
    ADDONS.iter().chain(std::iter::once(&FFMPEG)).find(|a| a.id == id)
}

/// Is it already on this PC, where the app would find it?
pub fn installed(a: &AddOn, p: &Paths) -> bool {
    match a.id {
        "ffmpeg" => p.find_tool("ffmpeg.exe", "ffmpeg_dir").is_some() && p.find_tool("ffprobe.exe", "ffmpeg_dir").is_some(),
        "yt-dlp" => p.find_tool("yt-dlp.exe", "ytdlp_dir").is_some(),
        "whisper" => {
            let dirs: Vec<PathBuf> = p.setting_dir("whisper_dir").into_iter().chain(Some(p.cache().join("whisper"))).collect();
            ["whisper-cli.exe", "whisper.exe"].iter().any(|n| dirs.iter().any(|d| d.join(n).is_file()) || p.on_path(n).is_some())
        }
        "se-html" => p.bin().join("se-html.exe").is_file(),
        "enhancer" => p.bin().join("enhancer").join("realesrgan-ncnn-vulkan.exe").is_file(),
        _ => false,
    }
}

/// The two exes out of an FFmpeg build zip (`<build>/bin/ffmpeg.exe`), flattened.
#[cfg(test)]
pub fn ffmpeg_map(e: &zip::Entry) -> Option<PathBuf> {
    let n = e.file_name();
    ["ffmpeg.exe", "ffprobe.exe"].iter().any(|w| n.eq_ignore_ascii_case(w)).then(|| PathBuf::from(n))
}

struct Pinned {
    url: String,
    sha256: String,
    size: Option<u64>,
}

fn resolve(a: &AddOn, net: &dyn Net) -> Result<(Pinned, Option<&'static str>), String> {
    match &a.src {
        Src::Manifest(text) => {
            let j = json::parse(text)?;
            let url = j.str("url").ok_or("manifest has no url")?.to_string();
            let sha256 = j.str("sha256").ok_or("manifest has no sha256")?.to_string();
            Ok((Pinned { url, sha256, size: j.num("size").map(|n| n as u64) }, Some(text)))
        }
        Src::GitHub { repo, asset } => {
            let r = net::latest_release(net, repo)?;
            let x = r.asset(asset).ok_or_else(|| format!("{repo}'s latest release has no {asset}"))?;
            // never install an unverified program: GitHub's digest is the checksum
            let sha256 = x.sha256.clone().ok_or_else(|| format!("{asset} has no checksum on GitHub"))?;
            Ok((Pinned { url: x.url.clone(), sha256, size: Some(x.size).filter(|&s| s > 0) }, None))
        }
    }
}

/// Download, verify and unpack one add-on. `lo..hi` is its share of the whole job's progress bar.
/// Returns the folder or file it now lives in.
pub fn install(a: &AddOn, p: &Paths, net: &dyn Net, rep: &Report, lo: f32, hi: f32) -> Result<PathBuf, String> {
    let name = a.label.split(" - ").next().unwrap_or(a.label).trim_start_matches("Install ");
    rep.say(lo, &format!("Looking up {name}…"));
    let (pin, manifest) = resolve(a, net)?;
    rep.check()?;
    let dest = (a.dest)(p);
    std::fs::create_dir_all(&p.temp).map_err(|e| format!("{}: {e}", p.temp.display()))?;
    let file_name = pin.url.rsplit('/').next().unwrap_or("download").to_string();
    let dl = p.temp.join(format!("se-setup-{}-{file_name}", a.id));
    let span = (hi - lo) * 0.8;
    net::download(net, &pin.url, &dl, Some(&pin.sha256), pin.size, &mut |got, total| {
        let f = total.map(|t| got as f32 / t.max(1) as f32).unwrap_or(0.0);
        rep.say(lo + span * f, &format!("Downloading {name}… {} of {}", mb(got), total.map(mb).unwrap_or_else(|| "?".into())));
        !rep.cancelled()
    })?;
    let unpack_at = lo + span;
    let r = unpack(a, &dl, &dest, rep, unpack_at, hi, name);
    let _ = std::fs::remove_file(&dl);
    let out = r?;
    if let (Some(fname), Some(text)) = (a.keep_manifest, manifest) {
        let dir = if out.is_dir() { out.clone() } else { dest.clone() };
        let _ = std::fs::write(dir.join(fname), text);
    }
    rep.say(hi, &format!("{name} installed"));
    Ok(out)
}

fn unpack(a: &AddOn, dl: &Path, dest: &Path, rep: &Report, lo: f32, hi: f32, name: &str) -> Result<PathBuf, String> {
    let mut prog = |done: u64, total: u64| {
        rep.say(lo + (hi - lo) * done as f32 / total.max(1) as f32, &format!("Unpacking {name}…"));
        !rep.cancelled()
    };
    match &a.unpack {
        Unpack::File(fname) => {
            std::fs::create_dir_all(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
            let to = dest.join(fname);
            let _ = std::fs::remove_file(&to);
            // a rename across volumes (a moved TEMP) fails: copy then
            if std::fs::rename(dl, &to).is_err() {
                std::fs::copy(dl, &to).map_err(|e| format!("{}: {e}", to.display()))?;
            }
            Ok(to)
        }
        Unpack::Pick(names) => {
            let got = zip::extract_where(
                dl,
                dest,
                &|e| {
                    let n = e.file_name();
                    names.iter().any(|w| n.eq_ignore_ascii_case(w)).then(|| PathBuf::from(n))
                },
                &mut prog,
            )?;
            if got.len() < names.len() {
                return Err(format!("the {name} download is missing files"));
            }
            Ok(dest.to_path_buf())
        }
        Unpack::Folder { main, junk } => {
            let mut f = std::fs::File::open(dl).map_err(|e| e.to_string())?;
            let list = zip::entries(&mut f)?;
            drop(f);
            let at = list.iter().find(|e| e.file_name().eq_ignore_ascii_case(main)).ok_or_else(|| format!("the download has no {main}"))?;
            let prefix = at.name[..at.name.len() - at.file_name().len()].to_string();
            // unpack beside the old copy, then swap, so a failure leaves the old one working
            let fresh = PathBuf::from(format!("{}.new", dest.display()));
            let _ = std::fs::remove_dir_all(&fresh);
            let r = zip::extract_where(
                dl,
                &fresh,
                &|e| {
                    let rest = e.name.strip_prefix(prefix.as_str())?;
                    (!junk.iter().any(|j| e.file_name().eq_ignore_ascii_case(j))).then(|| PathBuf::from(rest))
                },
                &mut prog,
            );
            if let Err(e) = r {
                let _ = std::fs::remove_dir_all(&fresh);
                return Err(e);
            }
            let _ = std::fs::remove_dir_all(dest);
            std::fs::rename(&fresh, dest).map_err(|e| format!("{}: {e}", dest.display()))?;
            Ok(dest.to_path_buf())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::FakeNet;
    use crate::report::quiet;
    use crate::sha256::Sha256;
    use crate::testutil::TempDir;

    fn sha(b: &[u8]) -> String {
        let mut h = Sha256::new();
        h.update(b);
        h.hex()
    }

    #[test]
    fn manifests_compiled_in_are_complete() {
        for a in ADDONS.iter().chain(Some(&FFMPEG)) {
            if let Src::Manifest(t) = a.src {
                let j = json::parse(t).unwrap();
                assert!(j.str("url").unwrap().starts_with("https://"), "{}", a.id);
                assert_eq!(j.str("sha256").unwrap().len(), 64, "{}", a.id);
                assert!(j.num("size").unwrap() > 0.0, "{}", a.id);
            }
            assert!(a.id == "ffmpeg" || a.label.contains(" - "), "{}: a one-line use", a.id);
        }
    }

    #[test]
    fn detection_follows_the_apps_lookup() {
        let t = TempDir::new("detect");
        let p = Paths::under(t.path());
        assert!(!installed(&FFMPEG, &p));
        std::fs::create_dir_all(p.install.join("ffmpeg")).unwrap();
        std::fs::write(p.install.join("ffmpeg").join("ffmpeg.exe"), b"x").unwrap();
        assert!(!installed(&FFMPEG, &p), "ffprobe too");
        std::fs::write(p.install.join("ffmpeg").join("ffprobe.exe"), b"x").unwrap();
        assert!(installed(&FFMPEG, &p));
        // a folder named in the app's settings counts
        let mine = t.path().join("tools");
        std::fs::create_dir_all(&mine).unwrap();
        std::fs::write(mine.join("yt-dlp.exe"), b"x").unwrap();
        let ytdlp = by_id("yt-dlp").unwrap();
        assert!(!installed(ytdlp, &p));
        std::fs::create_dir_all(&p.app_data).unwrap();
        std::fs::write(p.app_data.join("settings.json"), format!("{{\"ytdlp_dir\":{}}}", json::Json::Str(mine.display().to_string()).dump())).unwrap();
        assert!(installed(ytdlp, &p));
        // PATH counts
        let mut p2 = p.clone();
        let w = t.path().join("onpath");
        std::fs::create_dir_all(&w).unwrap();
        std::fs::write(w.join("whisper-cli.exe"), b"x").unwrap();
        assert!(!installed(by_id("whisper").unwrap(), &p2));
        p2.path_dirs.push(w);
        assert!(installed(by_id("whisper").unwrap(), &p2));
    }

    #[test]
    fn a_github_add_on_is_verified_against_the_api_digest() {
        let t = TempDir::new("gh");
        let p = Paths::under(t.path());
        let exe = b"MZ fake yt-dlp";
        let rel = format!(
            r#"{{"tag_name":"2026.11.01","assets":[{{"name":"yt-dlp.exe","size":{},"digest":"sha256:{}","browser_download_url":"https://dl/yt-dlp.exe"}}]}}"#,
            exe.len(),
            sha(exe)
        );
        let net = FakeNet::new(&[("https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest", rel.as_bytes()), ("https://dl/yt-dlp.exe", exe)]);
        let out = quiet(|r| install(by_id("yt-dlp").unwrap(), &p, &net, r, 0.0, 1.0)).unwrap();
        assert_eq!(out, p.install.join("yt-dlp.exe"));
        assert_eq!(std::fs::read(&out).unwrap(), exe);
        assert!(installed(by_id("yt-dlp").unwrap(), &p));
        // a tampered download is refused and nothing is left behind
        let net = FakeNet::new(&[("https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest", rel.as_bytes()), ("https://dl/yt-dlp.exe", b"MZ evil yt-dlp")]);
        std::fs::remove_file(&out).unwrap();
        assert!(quiet(|r| install(by_id("yt-dlp").unwrap(), &p, &net, r, 0.0, 1.0)).is_err());
        assert!(!out.exists());
        // no digest, no install
        let nodigest = rel.replace(&format!("\"digest\":\"sha256:{}\",", sha(exe)), "");
        let net = FakeNet::new(&[("https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest", nodigest.as_bytes()), ("https://dl/yt-dlp.exe", exe)]);
        assert!(quiet(|r| install(by_id("yt-dlp").unwrap(), &p, &net, r, 0.0, 1.0)).unwrap_err().contains("checksum"));
    }

    #[test]
    fn a_zip_add_on_unpacks_the_folder_holding_its_program() {
        let t = TempDir::new("wz");
        let p = Paths::under(t.path());
        let z = t.path().join("w.zip");
        zip::write_zip(&z, &[("Release/whisper-cli.exe", b"cli", true), ("Release/ggml.dll", b"dll", true), ("README.md", b"r", false)]);
        let bytes = std::fs::read(&z).unwrap();
        let rel = format!(
            r#"{{"tag_name":"v1.8.0","assets":[{{"name":"whisper-bin-x64.zip","size":{},"digest":"sha256:{}","browser_download_url":"https://dl/w.zip"}}]}}"#,
            bytes.len(),
            sha(&bytes)
        );
        let net = FakeNet::new(&[("https://api.github.com/repos/ggml-org/whisper.cpp/releases/latest", rel.as_bytes()), ("https://dl/w.zip", &bytes)]);
        let out = quiet(|r| install(by_id("whisper").unwrap(), &p, &net, r, 0.0, 1.0)).unwrap();
        assert_eq!(out, p.local_data.join("cache").join("whisper"));
        assert_eq!(std::fs::read(out.join("whisper-cli.exe")).unwrap(), b"cli");
        assert!(out.join("ggml.dll").is_file() && !out.join("README.md").exists());
        assert!(installed(by_id("whisper").unwrap(), &p));
    }
}
