//! ---- ws:addon-voice ----
//! The neural voice add-ons, never part of the exe: each is a set of pinned downloads named by an embedded
//! manifest (`library/bin/kokoro.json`, `library/bin/chatterbox.json`: url, size, SHA-256), installed on a
//! click into `%APPDATA%\SimpleEditor\bin\<engine>` like the AI enhancer (`enhance.rs`).
//!
//! - **Kokoro** (Kokoro-82M, Apache-2.0) runs in sherpa-onnx's own `sherpa-onnx-offline-tts.exe`
//!   (Apache-2.0, onnxruntime on the CPU). sherpa-onnx 2.x phonemises from the misaki lexicons alone (no
//!   espeak-ng, no Python); words the lexicon lacks are spelled from lexicon pieces here (`extra_lexicon`).
//! - **Chatterbox** (Resemble AI, MIT) runs through the onnx-community ONNX export in a downloaded
//!   embeddable Python with onnxruntime-directml: the encoder and decoder on the GPU through DirectML when
//!   there is one, the q4 language model on the CPU (`assets/tts/chatterbox_runner.py`).

use crate::engine::blitz;
use crate::engine::export::{self, Progress};
use crate::media::ffpipe;
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

const KOKORO_MANIFEST: &str = include_str!("../../library/bin/kokoro.json");
const CHATTERBOX_MANIFEST: &str = include_str!("../../library/bin/chatterbox.json");
const RUNNER: &str = include_str!("../../assets/tts/chatterbox_runner.py");
const SHERPA_EXE: &str = "sherpa-onnx-offline-tts.exe";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Addon {
    Kokoro,
    Chatterbox,
}

impl Addon {
    pub const ALL: [Addon; 2] = [Addon::Kokoro, Addon::Chatterbox];
    pub fn id(self) -> &'static str {
        match self {
            Addon::Kokoro => "kokoro",
            Addon::Chatterbox => "chatterbox",
        }
    }
    fn manifest_text(self) -> &'static str {
        match self {
            Addon::Kokoro => KOKORO_MANIFEST,
            Addon::Chatterbox => CHATTERBOX_MANIFEST,
        }
    }
    pub fn manifest(self) -> Manifest {
        parse_manifest(self.manifest_text()).expect("the embedded manifest parses (manifests_are_complete)")
    }
    /// `%APPDATA%\SimpleEditor\bin\<id>`; `SE_TTS_<ID>` (a folder) overrides it for tests and development.
    pub fn dir(self) -> PathBuf {
        let var = format!("SE_TTS_{}", self.id().to_ascii_uppercase());
        if let Some(p) = std::env::var_os(var).map(PathBuf::from).filter(|p| p.is_dir()) {
            return p;
        }
        blitz::bin_dir().join(self.id())
    }
    /// The file whose presence means "installed" (written last by `install`).
    fn marker(self) -> PathBuf {
        self.dir().join("installed.json")
    }
    pub fn available(self) -> bool {
        self.marker().is_file()
    }
    /// Bytes the add-on takes on disk (walks the folder: call off the UI thread or rarely).
    pub fn disk_size(self) -> u64 {
        fn walk(p: &Path) -> u64 {
            std::fs::read_dir(p)
                .map(|r| {
                    r.flatten()
                        .map(|e| match e.file_type() {
                            Ok(t) if t.is_dir() => walk(&e.path()),
                            _ => e.metadata().map(|m| m.len()).unwrap_or(0),
                        })
                        .sum()
                })
                .unwrap_or(0)
        }
        walk(&self.dir())
    }
}

// ---------------------------------------------------------------------------------------------------
// manifests + install

#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub url: String,
    pub sha256: String,
    pub size: u64,
    /// An archive (zip, wheel, tar.bz2) unpacked with Windows' tar.
    pub unpack: bool,
    /// For an archive: only these file names (found anywhere inside), flattened into `to`. Empty = all of it.
    pub pick: Vec<String>,
    /// Relative to the add-on folder: a folder for an archive, the file's path otherwise.
    pub to: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    pub version: String,
    pub license: String,
    pub parts: Vec<Part>,
}

impl Manifest {
    pub fn download_size(&self) -> u64 {
        self.parts.iter().map(|p| p.size).sum()
    }
}

pub fn parse_manifest(text: &str) -> Result<Manifest, String> {
    let v: serde_json::Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())?;
    let s = |v: &serde_json::Value, k: &str| v[k].as_str().unwrap_or("").trim().to_string();
    let mut parts = Vec::new();
    for p in v["files"].as_array().ok_or("no files")? {
        let part = Part {
            url: s(p, "url"),
            sha256: s(p, "sha256").to_ascii_lowercase(),
            size: p["size"].as_u64().unwrap_or(0),
            unpack: p["unpack"].as_bool().unwrap_or(false),
            pick: p["pick"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default(),
            to: s(p, "to"),
        };
        if !part.url.starts_with("https://") || part.sha256.len() != 64 || part.size == 0 || part.to.contains("..") {
            return Err(format!("incomplete entry: {}", part.url));
        }
        if !part.unpack && part.to.is_empty() {
            return Err(format!("no destination for {}", part.url));
        }
        parts.push(part);
    }
    if parts.is_empty() {
        return Err("no files".into());
    }
    Ok(Manifest { version: s(&v, "version"), license: s(&v, "license"), parts })
}

/// Last path segment of a URL: the name a download is cached under (`SE_TTS_DOWNLOADS`).
fn url_name(url: &str) -> &str {
    url.rsplit('/').next().unwrap_or(url)
}

/// curl as a child, so a cancel stops it at once and the progress follows the bytes on disk.
fn download(url: &str, dest: &Path, size: u64, prog: &Progress, at: (f32, f32), what: &str) -> Result<(), String> {
    let curl = ffpipe::find_exe("curl.exe", "").ok_or("curl.exe not found")?;
    let mut child = ffpipe::command(&curl)
        .args(["--fail", "--location", "--silent", "--show-error", "--retry", "2", "-o"])
        .arg(dest)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("curl: {e}"))?;
    let tail = export::stderr_tail(&mut child);
    loop {
        if prog.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(export::CANCELLED.into());
        }
        if let Some(st) = child.try_wait().map_err(|e| e.to_string())? {
            if st.success() {
                return Ok(());
            }
            let msg = tail.and_then(|t| t.join().ok()).unwrap_or_default();
            return Err(if msg.is_empty() { format!("download failed ({st})") } else { msg });
        }
        let got = std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
        let f = (got as f32 / size.max(1) as f32).min(1.0);
        prog.set(at.0 + (at.1 - at.0) * f, format!("Downloading {what} ({:.0} of {:.0} MB)…", got as f64 / 1048576.0, size as f64 / 1048576.0));
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

fn tar_exe() -> Result<PathBuf, String> {
    // Windows' own tar (bsdtar) reads zip, wheel and bz2; git's GNU tar on PATH does not
    std::env::var_os("SystemRoot")
        .map(|r| PathBuf::from(r).join("System32").join("tar.exe"))
        .filter(|p| p.is_file())
        .ok_or_else(|| "tar.exe not found".to_string())
}

fn find_named(root: &Path, name: &str) -> Option<PathBuf> {
    for e in std::fs::read_dir(root).ok()?.flatten() {
        let p = e.path();
        if p.is_dir() {
            if let Some(f) = find_named(&p, name) {
                return Some(f);
            }
        } else if e.file_name().to_string_lossy().eq_ignore_ascii_case(name) {
            return Some(p);
        }
    }
    None
}

/// Download every part (or copy it from `SE_TTS_DOWNLOADS`, a folder of already-fetched files), check its
/// size and SHA-256, unpack or place it, finish the folder, check it runs, then swap it in. Blocking: a
/// background job only, started by a click or the `tts.install` tool.
pub fn install(addon: Addon, prog: &Progress) -> Result<(), String> {
    let m = addon.manifest();
    let bin = blitz::bin_dir();
    let fresh = bin.join(format!("{}.new", addon.id()));
    let dl = bin.join(format!("{}.download", addon.id()));
    let _ = std::fs::remove_dir_all(&fresh);
    std::fs::create_dir_all(&fresh).map_err(|e| format!("{}: {e}", fresh.display()))?;
    std::fs::create_dir_all(&dl).map_err(|e| format!("{}: {e}", dl.display()))?;
    let r = install_parts(addon, &m, &fresh, &dl, prog).and_then(|_| {
        prog.set(0.97, "Checking it runs…");
        check_runs(addon, &fresh)?;
        std::fs::write(fresh.join("installed.json"), addon.manifest_text()).map_err(|e| e.to_string())?;
        let dest = blitz::bin_dir().join(addon.id());
        let _ = std::fs::remove_dir_all(&dest);
        std::fs::rename(&fresh, &dest).map_err(|e| format!("{}: {e}", dest.display()))
    });
    let _ = std::fs::remove_dir_all(&fresh);
    let _ = std::fs::remove_dir_all(&dl);
    r?;
    prog.set(1.0, "Installed");
    Ok(())
}

fn install_parts(addon: Addon, m: &Manifest, root: &Path, dl: &Path, prog: &Progress) -> Result<(), String> {
    let total = m.download_size().max(1) as f32;
    let mut done = 0u64;
    let cache = std::env::var_os("SE_TTS_DOWNLOADS").map(PathBuf::from);
    for (i, p) in m.parts.iter().enumerate() {
        if prog.is_cancelled() {
            return Err(export::CANCELLED.into());
        }
        let name = url_name(&p.url);
        let file = dl.join(format!("{i}-{name}"));
        let at = (0.9 * done as f32 / total, 0.9 * (done + p.size) as f32 / total);
        match cache.as_ref().map(|c| c.join(name)).filter(|c| c.is_file()) {
            Some(c) => {
                prog.set(at.0, format!("Copying {name}…"));
                std::fs::copy(&c, &file).map_err(|e| format!("{}: {e}", c.display()))?;
            }
            None => download(&p.url, &file, p.size, prog, at, name)?,
        }
        let size = std::fs::metadata(&file).map(|x| x.len()).unwrap_or(0);
        if size != p.size {
            return Err(format!("{name} is {size} bytes, expected {}", p.size));
        }
        prog.set(at.1, format!("Checking {name}…"));
        if blitz::sha256_of(&file)? != p.sha256 {
            return Err(format!("{name} does not match its checksum"));
        }
        let to = root.join(&p.to);
        if p.unpack {
            let into = if p.pick.is_empty() { to.clone() } else { dl.join(format!("{i}-x")) };
            std::fs::create_dir_all(&into).map_err(|e| e.to_string())?;
            let out = ffpipe::command(&tar_exe()?)
                .arg("-xf")
                .arg(&file)
                .arg("-C")
                .arg(&into)
                .stdin(Stdio::null())
                .output()
                .map_err(|e| format!("tar: {e}"))?;
            if !out.status.success() {
                return Err(format!("could not unpack {name}: {}", String::from_utf8_lossy(&out.stderr).trim()));
            }
            if !p.pick.is_empty() {
                std::fs::create_dir_all(&to).map_err(|e| e.to_string())?;
                for want in &p.pick {
                    let f = find_named(&into, want).ok_or_else(|| format!("{name} has no {want}"))?;
                    std::fs::rename(&f, to.join(want)).map_err(|e| e.to_string())?;
                }
                let _ = std::fs::remove_dir_all(&into);
            }
            let _ = std::fs::remove_file(&file);
        } else {
            if let Some(d) = to.parent() {
                std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
            }
            std::fs::rename(&file, &to).map_err(|e| e.to_string())?;
        }
        done += p.size;
    }
    if addon == Addon::Chatterbox {
        // the embeddable Python only sees what its ._pth names: add site-packages (and nothing from the
        // user's own Python installs: the runner also runs with -I)
        let pth = root.join("py").join("python312._pth");
        std::fs::write(&pth, "python312.zip\r\n.\r\nLib\\site-packages\r\n").map_err(|e| e.to_string())?;
        std::fs::write(root.join("runner.py"), RUNNER).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn check_runs(addon: Addon, root: &Path) -> Result<(), String> {
    let out = match addon {
        Addon::Kokoro => ffpipe::command(&root.join(SHERPA_EXE)).arg("--help").stdin(Stdio::null()).output(),
        Addon::Chatterbox => ffpipe::command(&root.join("py").join("python.exe"))
            .args(["-I", "-c", "import onnxruntime, numpy, tokenizers; print('voice-runtime-ok', onnxruntime.get_available_providers())"])
            .stdin(Stdio::null())
            .output(),
    }
    .map_err(|e| format!("it does not start: {e}"))?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let want = if addon == Addon::Kokoro { "kokoro" } else { "voice-runtime-ok" };
    if text.contains(want) {
        Ok(())
    } else {
        Err(format!("the installed {} does not run: {}", addon.id(), text.trim().chars().take(300).collect::<String>()))
    }
}

/// Remove the add-on (speech it already wrote stays where it is).
pub fn uninstall(addon: Addon) -> Result<(), String> {
    let d = blitz::bin_dir().join(addon.id());
    if d.exists() {
        std::fs::remove_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    Ok(())
}

static INSTALLING: Mutex<Vec<(Addon, Arc<Progress>)>> = Mutex::new(Vec::new());

/// The running install of `addon`, if any.
pub fn installing(addon: Addon) -> Option<Arc<Progress>> {
    let slot = INSTALLING.lock().unwrap_or_else(|e| e.into_inner());
    slot.iter().find(|(a, p)| *a == addon && !p.is_done()).map(|(_, p)| p.clone())
}

/// Why the last install of `addon` failed (shown beside its Install button until the next try).
pub fn install_error(addon: Addon) -> Option<String> {
    let slot = INSTALLING.lock().unwrap_or_else(|e| e.into_inner());
    slot.iter().find(|(a, p)| *a == addon && p.is_done() && !p.is_cancelled()).and_then(|(_, p)| p.error())
}

/// The click on Install: a background job (the Jobs pane shows it). One download at a time: a second
/// add-on waits for the first to finish before it starts fetching. A click while one runs returns it.
pub fn start_install(addon: Addon) -> Arc<Progress> {
    let mut slot = INSTALLING.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, p)) = slot.iter().find(|(a, p)| *a == addon && !p.is_done()) {
        return p.clone();
    }
    let before: Vec<Arc<Progress>> = slot.iter().filter(|(_, p)| !p.is_done()).map(|(_, p)| p.clone()).collect();
    let p = export::spawn_job("tts-install", move |prog| {
        while before.iter().any(|b| !b.is_done()) {
            if prog.is_cancelled() {
                return Err(export::CANCELLED.into());
            }
            prog.set(0.0, "Waiting for the other download…");
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
        install(addon, prog)
    });
    slot.retain(|(a, _)| *a != addon);
    slot.push((addon, p.clone()));
    p
}

// ---------------------------------------------------------------------------------------------------
// text

/// English words for a whole number (Kokoro's lexicon has none for digits; the exe skips them).
pub fn number_words(n: u64) -> String {
    const ONES: [&str; 20] = [
        "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve",
        "thirteen", "fourteen", "fifteen", "sixteen", "seventeen", "eighteen", "nineteen",
    ];
    const TENS: [&str; 10] = ["", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety"];
    fn under_1000(n: u64) -> String {
        let (h, r) = (n / 100, n % 100);
        let mut parts = Vec::new();
        if h > 0 {
            parts.push(format!("{} hundred", ONES[h as usize]));
        }
        if r >= 20 {
            parts.push(if r % 10 == 0 { TENS[(r / 10) as usize].to_string() } else { format!("{} {}", TENS[(r / 10) as usize], ONES[(r % 10) as usize]) });
        } else if r > 0 || h == 0 {
            parts.push(ONES[r as usize].to_string());
        }
        parts.join(" ")
    }
    if n < 1000 {
        return under_1000(n);
    }
    // years read as years: 1999 -> nineteen ninety nine, 2026 -> twenty twenty six
    if (1100..2100).contains(&n) && n % 100 != 0 && !(2000..2010).contains(&n) {
        let r = n % 100;
        let tail = if r < 10 { format!("oh {}", ONES[r as usize]) } else { under_1000(r) };
        return format!("{} {tail}", under_1000(n / 100));
    }
    let mut parts = Vec::new();
    let mut rest = n;
    for (scale, name) in [(1_000_000_000_000u64, "trillion"), (1_000_000_000, "billion"), (1_000_000, "million"), (1000, "thousand")] {
        if rest >= scale {
            parts.push(format!("{} {name}", number_words(rest / scale)));
            rest %= scale;
        }
    }
    if rest > 0 {
        parts.push(under_1000(rest));
    }
    parts.join(" ")
}

/// Text the Kokoro exe reads well: typographic punctuation as ASCII, accents dropped (it reads its
/// arguments in the ANSI code page), digits as words ("3.5" -> "three point five", "1,000" -> "one thousand").
pub fn plain_text(text: &str) -> String {
    let mut s = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\u{2018}' | '\u{2019}' | '\u{02BC}' => s.push('\''),
            '\u{201C}' | '\u{201D}' => s.push('"'),
            '\u{2013}' | '\u{2014}' => s.push_str(", "),
            '\u{2026}' => s.push_str("..."),
            '&' => s.push_str(" and "),
            '%' => s.push_str(" percent"),
            c if c.is_ascii() => s.push(c),
            c => {
                // é -> e and friends: the first char of the canonical decomposition, when ASCII
                let base = match c {
                    'à'..='å' => 'a',
                    'À'..='Å' => 'A',
                    'è'..='ë' => 'e',
                    'È'..='Ë' => 'E',
                    'ì'..='ï' => 'i',
                    'Ì'..='Ï' => 'I',
                    'ò'..='ö' => 'o',
                    'Ò'..='Ö' => 'O',
                    'ù'..='ü' => 'u',
                    'Ù'..='Ü' => 'U',
                    'ç' => 'c',
                    'Ç' => 'C',
                    'ñ' => 'n',
                    'Ñ' => 'N',
                    c if c.is_whitespace() => ' ',
                    _ => continue,
                };
                s.push(base);
            }
        }
    }
    // numbers: digit runs with , thousands separators and one decimal point
    let mut out = String::with_capacity(s.len());
    let b: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < b.len() {
        if !b[i].is_ascii_digit() {
            out.push(b[i]);
            i += 1;
            continue;
        }
        let mut int = String::new();
        while i < b.len() && (b[i].is_ascii_digit() || (b[i] == ',' && !int.is_empty() && b.get(i + 1).is_some_and(|c| c.is_ascii_digit()))) {
            if b[i] != ',' {
                int.push(b[i]);
            }
            i += 1;
        }
        let words = int.parse::<u64>().map(number_words).unwrap_or_else(|_| int.chars().map(|d| number_words(d as u64 - '0' as u64)).collect::<Vec<_>>().join(" "));
        out.push_str(&words);
        if i + 1 < b.len() && b[i] == '.' && b[i + 1].is_ascii_digit() {
            out.push_str(" point");
            i += 1;
            while i < b.len() && b[i].is_ascii_digit() {
                out.push(' ');
                out.push_str(&number_words(b[i] as u64 - '0' as u64));
                i += 1;
            }
        }
    }
    out
}

/// Lexicon entries ("word phonemes") for the words of `text` that `lexicon` lacks, each spelled from the
/// longest lexicon pieces it splits into ("zorblaxian" -> zor + blax + ian). Words that do not split stay
/// out (the exe skips them, as before). ponytail: a greedy split, not a letter-to-sound model; it reads
/// made-up names and brands roughly right, which is the common case in scripts.
pub fn extra_lexicon(text: &str, lexicon: &HashMap<String, String>) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for w in text.split(|c: char| !(c.is_ascii_alphabetic() || c == '\'')) {
        let w = w.trim_matches('\'').to_ascii_lowercase();
        if w.is_empty() || lexicon.contains_key(&w) || out.iter().any(|(x, _)| *x == w) {
            continue;
        }
        let mut phones = Vec::new();
        let mut rest = w.as_str();
        while !rest.is_empty() {
            // longest piece first; single letters only as a last resort (they read as letter names)
            let found = (1..=rest.len()).rev().find_map(|n| {
                let piece = &rest[..n];
                (n >= 2 || rest.len() == 1).then(|| lexicon.get(piece).map(|p| (n, p)))?
            });
            match found {
                Some((n, p)) => {
                    phones.push(p.clone());
                    rest = &rest[n..];
                }
                None => break,
            }
        }
        if rest.is_empty() && !phones.is_empty() {
            out.push((w, phones.join(" ")));
        }
    }
    out
}

pub fn read_lexicon(path: &Path) -> HashMap<String, String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_once(' ').map(|(w, p)| (w.to_string(), p.trim().to_string())))
        .collect()
}

// ---------------------------------------------------------------------------------------------------
// Kokoro

/// The English voices in Kokoro v1.0's voices.bin, in speaker-id order (the id is the index). The other
/// languages in the file need lexicons sherpa-onnx does not ship for this model.
pub const KOKORO_VOICES: [&str; 28] = [
    "af_alloy", "af_aoede", "af_bella", "af_heart", "af_jessica", "af_kore", "af_nicole", "af_nova", "af_river",
    "af_sarah", "af_sky", "am_adam", "am_echo", "am_eric", "am_fenrir", "am_liam", "am_michael", "am_onyx",
    "am_puck", "am_santa", "bf_alice", "bf_emma", "bf_isabella", "bf_lily", "bm_daniel", "bm_fable", "bm_george",
    "bm_lewis",
];
pub const KOKORO_DEFAULT: &str = "af_heart";

/// "af_heart" -> "Heart (US, female)".
pub fn kokoro_label(id: &str) -> String {
    let (pre, name) = id.split_once('_').unwrap_or(("", id));
    let mut n = name.to_string();
    if let Some(f) = n.get_mut(0..1) {
        f.make_ascii_uppercase();
    }
    let accent = if pre.starts_with('b') { "UK" } else { "US" };
    let sex = if pre.ends_with('f') { "female" } else { "male" };
    format!("{n} ({accent}, {sex})")
}

/// The `sherpa-onnx-offline-tts.exe` call for one chunk. `extra` = a lexicon file of spelled-out words,
/// read before the main one. Pure; see `kokoro_command_args`.
pub fn kokoro_command(dir: &Path, text: &str, voice: &str, speed: f32, extra: Option<&Path>, out: &Path) -> Command {
    let sid = KOKORO_VOICES.iter().position(|v| *v == voice).unwrap_or(3);
    let model = dir.join("model");
    let lex = model.join(if voice.starts_with('b') { "lexicon-gb-en.txt" } else { "lexicon-us-en.txt" });
    let lexicons = match extra {
        Some(e) => format!("{},{}", e.display(), lex.display()),
        None => lex.display().to_string(),
    };
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 8);
    let mut c = ffpipe::command(&dir.join(SHERPA_EXE));
    c.arg("--debug=0")
        .arg(format!("--kokoro-model={}", model.join("model.int8.onnx").display()))
        .arg(format!("--kokoro-voices={}", model.join("voices.bin").display()))
        .arg(format!("--kokoro-tokens={}", model.join("tokens.txt").display()))
        .arg(format!("--kokoro-lexicon={lexicons}"))
        .arg(format!("--kokoro-length-scale={:.3}", 1.0 / speed.clamp(0.5, 2.0)))
        .arg(format!("--num-threads={threads}"))
        .arg(format!("--sid={sid}"))
        .arg(format!("--output-filename={}", out.display()))
        .arg(text);
    c
}

/// Pieces of at most `max` chars, cut after sentence ends (a command line holds 32 K characters, and a
/// cancel lands between pieces).
pub fn chunks(text: &str, max: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for sentence in sentences(text) {
        match out.last_mut() {
            Some(last) if last.len() + 1 + sentence.len() <= max => {
                last.push(' ');
                last.push_str(&sentence);
            }
            _ => out.push(sentence),
        }
    }
    out
}

/// Sentences (cut after . ! ? and line breaks), each trimmed; empty ones dropped.
pub fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = text.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if c == '\n' || c == '\r' {
            out.push(std::mem::take(&mut cur));
            continue;
        }
        cur.push(c);
        if matches!(c, '.' | '!' | '?') && chars.get(i + 1).is_none_or(|n| n.is_whitespace()) {
            out.push(std::mem::take(&mut cur));
        }
    }
    out.push(cur);
    out.into_iter().map(|s| s.trim().to_string()).filter(|s| s.chars().any(|c| c.is_alphanumeric())).collect()
}

/// Run a child to the end (killed on cancel); Err carries its stderr tail.
fn run(mut c: Command, what: &str, prog: &Progress) -> Result<(), String> {
    c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
    let mut child = c.spawn().map_err(|e| format!("{what}: {e}"))?;
    let tail = export::stderr_tail(&mut child);
    export::wait_ffmpeg(&mut child, tail, prog).map_err(|e| if e == export::CANCELLED { e } else { format!("{what}: {e}") })
}

/// Kokoro: `text` into WAVs in `tmp`, one per chunk (returned in order).
pub fn kokoro(text: &str, voice: &str, speed: f32, tmp: &Path, prog: &Progress) -> Result<Vec<PathBuf>, String> {
    let dir = Addon::Kokoro.dir();
    if !Addon::Kokoro.available() {
        return Err("Kokoro voices are not installed".into());
    }
    let text = plain_text(text);
    let lex = read_lexicon(&dir.join("model").join(if voice.starts_with('b') { "lexicon-gb-en.txt" } else { "lexicon-us-en.txt" }));
    let extra = extra_lexicon(&text, &lex);
    let extra_file = tmp.join("extra-lexicon.txt");
    if !extra.is_empty() {
        let body: String = extra.iter().map(|(w, p)| format!("{w} {p}\n")).collect();
        std::fs::write(&extra_file, body).map_err(|e| e.to_string())?;
    }
    let parts = chunks(&text, 1500);
    let mut wavs = Vec::new();
    for (i, part) in parts.iter().enumerate() {
        prog.set(0.05 + 0.85 * i as f32 / parts.len() as f32, "Speaking (Kokoro)…");
        let out = tmp.join(format!("kokoro-{i}.wav"));
        run(kokoro_command(&dir, part, voice, speed, (!extra.is_empty()).then_some(extra_file.as_path()), &out), "Kokoro", prog)?;
        wavs.push(out);
    }
    Ok(wavs)
}

// ---------------------------------------------------------------------------------------------------
// Chatterbox

/// The JSON request the runner reads on stdin. Pure; see `chatterbox_request_fields`.
pub fn chatterbox_request(dir: &Path, lines: &[String], reference: &Path, intensity: f32, gpu: bool, out: &Path) -> String {
    serde_json::json!({
        "models": dir.join("models"),
        "ref": reference,
        "lines": lines,
        "exaggeration": intensity.clamp(0.25, 2.0),
        "provider": if gpu { "auto" } else { "cpu" },
        "lm": "language_model_q4",
        "out": out,
    })
    .to_string()
}

/// One stdout line from the runner: Some(fraction) for "progress 0.5".
pub fn runner_progress(line: &str) -> Option<f32> {
    line.strip_prefix("progress ")?.trim().parse().ok()
}

/// Chatterbox: `text` in the voice of `reference` (a 24 kHz mono 16-bit WAV) into `out`.
pub fn chatterbox(text: &str, reference: &Path, intensity: f32, out: &Path, prog: &Progress) -> Result<(), String> {
    let dir = Addon::Chatterbox.dir();
    if !Addon::Chatterbox.available() {
        return Err("Chatterbox voices are not installed".into());
    }
    let lines = sentences(text);
    if lines.is_empty() {
        return Err("nothing to say".into());
    }
    let gpu = std::env::var("SE_TTS_CPU").is_err();
    let req = chatterbox_request(&dir, &lines, reference, intensity, gpu, out);
    prog.set(0.03, "Loading Chatterbox…");
    let mut child = ffpipe::command(&dir.join("py").join("python.exe"))
        .arg("-I")
        .arg(dir.join("runner.py"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Chatterbox: {e}"))?;
    if let Some(mut i) = child.stdin.take() {
        i.write_all(req.as_bytes()).map_err(|e| format!("Chatterbox: {e}"))?;
    }
    let tail = export::stderr_tail(&mut child);
    let p = Arc::new(Mutex::new(None::<f32>));
    if let Some(o) = child.stdout.take() {
        let p = p.clone();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(o).lines().map_while(Result::ok) {
                if let Some(f) = runner_progress(&line) {
                    *p.lock().unwrap_or_else(|e| e.into_inner()) = Some(f);
                }
            }
        });
    }
    let n = lines.len();
    loop {
        if prog.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(export::CANCELLED.into());
        }
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            break;
        }
        let f = p.lock().unwrap_or_else(|e| e.into_inner()).unwrap_or(0.0);
        let line = ((f * n as f32).round() as usize + 1).min(n);
        prog.set(0.05 + 0.85 * f, format!("Speaking line {line} of {n} (Chatterbox)…"));
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    export::wait_ffmpeg(&mut child, tail, prog).map_err(|e| format!("Chatterbox: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifests_are_complete() {
        for a in Addon::ALL {
            let m = a.manifest();
            assert!(!m.license.is_empty() && !m.version.is_empty(), "{a:?}");
            assert!(m.parts.iter().all(|p| p.url.starts_with("https://")));
        }
        assert_eq!(Addon::Kokoro.manifest().download_size(), 28_075_706 + 132_353_410);
        let cb = Addon::Chatterbox.manifest().download_size();
        assert!((1_500_000_000..1_700_000_000).contains(&cb), "{cb}");
        assert!(parse_manifest(r#"{"files":[{"url":"http://x","sha256":"a","size":1,"to":"x"}]}"#).is_err());
        let ok = format!(r#"{{"files":[{{"url":"https://x/y.zip","sha256":"{}","size":1,"unpack":true,"to":""}}]}}"#, "a".repeat(64));
        assert!(parse_manifest(&ok).is_ok());
        assert!(parse_manifest(&ok.replace(r#""to":"""#, r#""to":"../x""#)).is_err(), "no escaping the folder");
        assert!(parse_manifest(&ok.replace(r#""unpack":true,"to":"""#, r#""to":"""#)).is_err(), "a plain file needs a path");
        assert_eq!(url_name("https://a/b/c.whl"), "c.whl");
    }

    #[test]
    fn numbers_read_as_words() {
        assert_eq!(number_words(0), "zero");
        assert_eq!(number_words(42), "forty two");
        assert_eq!(number_words(300), "three hundred");
        assert_eq!(number_words(2026), "twenty twenty six");
        assert_eq!(number_words(1905), "nineteen oh five");
        assert_eq!(number_words(2003), "two thousand three");
        assert_eq!(number_words(12_500), "twelve thousand five hundred");
        assert_eq!(plain_text("In 2026, 3.5% had 1,000 \u{201C}ships\u{201D} \u{2014} caf\u{e9}"), "In twenty twenty six, three point five percent had one thousand \"ships\" ,  cafe");
        assert_eq!(plain_text("It\u{2019}s 7."), "It's seven.");
    }

    #[test]
    fn unknown_words_are_spelled_from_lexicon_pieces() {
        let lex: HashMap<String, String> =
            [("zor", "z ɔ ɹ"), ("blax", "b l æ k s"), ("ian", "i ə n"), ("hello", "h ə l O"), ("a", "A")]
                .into_iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect();
        let e = extra_lexicon("Hello Zorblaxian, hello! qqq", &lex);
        assert_eq!(e, vec![("zorblaxian".to_string(), "z ɔ ɹ b l æ k s i ə n".to_string())]);
    }

    #[test]
    fn text_is_cut_into_sentences_and_chunks() {
        assert_eq!(sentences("One. Two!\nThree? 3.5 is fine...  "), vec!["One.", "Two!", "Three?", "3.5 is fine..."]);
        assert_eq!(sentences(" ... \n"), Vec::<String>::new());
        let c = chunks("Aa. Bb. Cc.", 7);
        assert_eq!(c, vec!["Aa. Bb.", "Cc."]);
    }

    #[test]
    fn kokoro_command_args() {
        let c = kokoro_command(Path::new(r"C:\k"), "Hi there", "bm_george", 1.25, Some(Path::new(r"C:\t\x.txt")), Path::new(r"C:\o.wav"));
        assert!(c.get_program().to_string_lossy().ends_with(SHERPA_EXE));
        let a: Vec<String> = c.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert!(a.contains(&r"--kokoro-model=C:\k\model\model.int8.onnx".to_string()), "{a:?}");
        assert!(a.contains(&r"--kokoro-lexicon=C:\t\x.txt,C:\k\model\lexicon-gb-en.txt".to_string()), "UK voice, extra first: {a:?}");
        assert!(a.contains(&"--sid=26".to_string()), "{a:?}");
        assert!(a.contains(&"--kokoro-length-scale=0.800".to_string()), "{a:?}");
        assert!(a.contains(&r"--output-filename=C:\o.wav".to_string()));
        assert_eq!(a.last().map(String::as_str), Some("Hi there"), "the text is the last argument");
        let us = kokoro_command(Path::new("k"), "x", "nope", 1.0, None, Path::new("o.wav"));
        let a: Vec<String> = us.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert!(a.contains(&"--sid=3".to_string()), "an unknown voice is the default (af_heart)");
        assert!(a.iter().any(|x| x.ends_with("lexicon-us-en.txt") && !x.contains(',')));
        assert_eq!(KOKORO_VOICES[3], KOKORO_DEFAULT);
        assert_eq!(kokoro_label("bm_george"), "George (UK, male)");
        assert_eq!(kokoro_label("af_heart"), "Heart (US, female)");
    }

    #[test]
    fn chatterbox_request_fields() {
        let r = chatterbox_request(Path::new(r"C:\cb"), &["One.".into(), "Two.".into()], Path::new(r"C:\v\me.wav"), 3.0, false, Path::new(r"C:\o.wav"));
        let v: serde_json::Value = serde_json::from_str(&r).unwrap();
        assert_eq!(v["models"], r"C:\cb\models");
        assert_eq!(v["ref"], r"C:\v\me.wav");
        assert_eq!(v["lines"], serde_json::json!(["One.", "Two."]));
        assert_eq!(v["exaggeration"], 2.0, "clamped");
        assert_eq!(v["provider"], "cpu");
        assert_eq!(v["out"], r"C:\o.wav");
        assert_eq!(runner_progress("progress 0.500"), Some(0.5));
        assert_eq!(runner_progress("provider directml"), None);
        // the runner's request keys are the ones it reads
        for k in ["\"models\"", "\"ref\"", "\"lines\"", "\"exaggeration\"", "\"out\"", "\"provider\"", "\"lm\""] {
            assert!(RUNNER.contains(&format!("req[{k}]")) || RUNNER.contains(&format!("req.get({k}")), "{k}");
        }
    }

    /// Real install (ignored): every add-on from its manifest into this profile's bin folder. With
    /// `SE_TTS_DOWNLOADS` = a folder of the already-fetched files it copies instead of downloading.
    #[test]
    #[ignore]
    fn real_install() {
        for a in Addon::ALL {
            let t = std::time::Instant::now();
            install(a, &Progress::new()).unwrap();
            assert!(a.available(), "{a:?}");
            eprintln!("{a:?}: installed in {:.0} s, {:.0} MB on disk", t.elapsed().as_secs_f64(), a.disk_size() as f64 / 1048576.0);
        }
    }
}
