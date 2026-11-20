//! ---- ws:gallery-stock ----
//! Gallery ▸ Stock: search free stock video, pictures, music and sound effects online, download one into the
//! project and keep its credit. Only official, documented APIs: four that need no key (Wikimedia Commons, the
//! Internet Archive, the NASA Image and Video Library, Openverse) and three the user can switch on with a free
//! key of their own (Pexels, Pixabay, Freesound; the key stays in Windows Credential Manager on this PC). Sites
//! without an API (Mixkit, Coverr, Videvo...) are only "open the site" links: nothing here reads their pages.
//! The user-facing description of every source and its licence rules is `website/docs/stock.mdx`.
//!
//! Network: `curl.exe` (the HTTP client the online library already uses), a generic User-Agent with no
//! personal data, background threads only. Every response is parsed by a pure function (`parse`) that the
//! tests run against saved sample responses in `fixtures/`.

pub mod keys;
pub mod sources;

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub use sources::parse;

/// What the app tells every stock site it is: the app and its version, nothing about the user or the PC.
pub fn user_agent() -> String {
    format!("SimpleEditor/{} (desktop video editor; stock search)", env!("CARGO_PKG_VERSION"))
}

/// A stock site with an official API.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Commons,
    Archive,
    Nasa,
    Openverse,
    Pexels,
    Pixabay,
    Freesound,
}

impl Source {
    pub const ALL: [Source; 7] = [
        Source::Commons,
        Source::Archive,
        Source::Nasa,
        Source::Openverse,
        Source::Pexels,
        Source::Pixabay,
        Source::Freesound,
    ];
    pub fn id(self) -> &'static str {
        match self {
            Source::Commons => "commons",
            Source::Archive => "archive",
            Source::Nasa => "nasa",
            Source::Openverse => "openverse",
            Source::Pexels => "pexels",
            Source::Pixabay => "pixabay",
            Source::Freesound => "freesound",
        }
    }
    pub fn from_id(s: &str) -> Option<Source> {
        Source::ALL.into_iter().find(|x| x.id().eq_ignore_ascii_case(s.trim()))
    }
    pub fn name(self) -> &'static str {
        match self {
            Source::Commons => "Wikimedia Commons",
            Source::Archive => "Internet Archive",
            Source::Nasa => "NASA",
            Source::Openverse => "Openverse",
            Source::Pexels => "Pexels",
            Source::Pixabay => "Pixabay",
            Source::Freesound => "Freesound",
        }
    }
    /// Needs the user's own free API key.
    pub fn needs_key(self) -> bool {
        matches!(self, Source::Pexels | Source::Pixabay | Source::Freesound)
    }
    /// Where the user gets a key (keyed sources) or the site's home page.
    pub fn site(self) -> &'static str {
        match self {
            Source::Commons => "https://commons.wikimedia.org",
            Source::Archive => "https://archive.org/details/prelinger",
            Source::Nasa => "https://images.nasa.gov",
            Source::Openverse => "https://openverse.org",
            Source::Pexels => "https://www.pexels.com/api/",
            Source::Pixabay => "https://pixabay.com/api/docs/",
            Source::Freesound => "https://freesound.org/apiv2/apply",
        }
    }
    /// Which kinds the source can search.
    pub fn kinds(self) -> &'static [Kind] {
        match self {
            Source::Commons => &[Kind::Video, Kind::Image, Kind::Music, Kind::Sfx],
            Source::Archive => &[Kind::Video, Kind::Music],
            Source::Nasa => &[Kind::Video, Kind::Image, Kind::Sfx],
            Source::Openverse => &[Kind::Image, Kind::Music, Kind::Sfx],
            Source::Pexels | Source::Pixabay => &[Kind::Video, Kind::Image],
            Source::Freesound => &[Kind::Sfx, Kind::Music],
        }
    }
}

/// Stock sites with no API (or terms that forbid automated access): the Gallery only offers to open them.
pub const LINK_ONLY: [(&str, &str); 8] = [
    ("Mixkit", "https://mixkit.co"),
    ("Coverr", "https://coverr.co"),
    ("Videvo", "https://www.videvo.net"),
    ("Videezy", "https://www.videezy.com"),
    ("Free Music Archive", "https://freemusicarchive.org"),
    ("Incompetech", "https://incompetech.com/music/royalty-free/"),
    ("YouTube Audio Library", "https://www.youtube.com/audiolibrary"),
    ("BBC Sound Effects", "https://sound-effects.bbcrewind.co.uk"),
];

/// What to search for.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Video,
    Image,
    Music,
    Sfx,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Video, Kind::Image, Kind::Music, Kind::Sfx];
    pub fn id(self) -> &'static str {
        match self {
            Kind::Video => "video",
            Kind::Image => "image",
            Kind::Music => "music",
            Kind::Sfx => "sfx",
        }
    }
    pub fn from_id(s: &str) -> Option<Kind> {
        let s = s.trim().to_ascii_lowercase();
        match s.as_str() {
            "photo" | "picture" | "images" => Some(Kind::Image),
            "audio" | "song" => Some(Kind::Music),
            "sound" | "sounds" | "sound_effect" | "effect" => Some(Kind::Sfx),
            _ => Kind::ALL.into_iter().find(|k| k.id() == s),
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Kind::Video => "Video",
            Kind::Image => "Images",
            Kind::Music => "Music",
            Kind::Sfx => "SFX",
        }
    }
    pub fn is_audio(self) -> bool {
        matches!(self, Kind::Music | Kind::Sfx)
    }
}

/// The licence groups the filter knows. Anything else (NC, ND, "all rights reserved", unknown) is never
/// shown: a video edit is a derivative work, and it may be monetised.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Licence {
    /// CC0, the Public Domain Mark, public domain, US government works (NASA).
    Pd,
    /// A site's own free licence that needs no credit (Pexels License, Pixabay Content License).
    Free,
    /// CC BY: credit the author.
    By,
    /// CC BY-SA: credit, and share what you make under the same licence.
    BySa,
    /// Not usable here (NC / ND / unknown).
    Other,
}

impl Licence {
    /// From a licence URL or a short name ("CC BY-SA 4.0", "cc0", "Public domain", "by-nc", ...).
    pub fn parse(s: &str) -> Licence {
        let l = s.trim().to_ascii_lowercase().replace(['_', ' '], "-");
        if l.is_empty() {
            return Licence::Other;
        }
        if l.contains("-nc") || l.contains("/nc") || l.contains("noncommercial") || l.contains("-nd") || l.contains("/nd") {
            return Licence::Other;
        }
        if l.contains("publicdomain")
            || l.contains("public-domain")
            || l == "pd"
            || l.starts_with("pd-")
            || l.contains("cc0")
            || l == "pdm"
            || l.contains("zero")
        {
            return Licence::Pd;
        }
        if l.contains("by-sa") || l.contains("/by-sa/") || l.contains("attribution-sharealike") {
            return Licence::BySa;
        }
        if l.contains("cc-by") || l == "by" || l.starts_with("by-") || l.contains("/by/") || l.contains("attribution") {
            return Licence::By;
        }
        Licence::Other
    }
    pub fn needs_credit(self) -> bool {
        matches!(self, Licence::By | Licence::BySa)
    }
}

/// How far the licence filter goes (each level includes the ones before it). The default is the most
/// permissive set of rights for the user: nothing that needs a credit.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LicenceLevel {
    #[default]
    Pd,
    By,
    BySa,
}

impl LicenceLevel {
    pub const ALL: [LicenceLevel; 3] = [LicenceLevel::Pd, LicenceLevel::By, LicenceLevel::BySa];
    pub fn id(self) -> &'static str {
        match self {
            LicenceLevel::Pd => "pd",
            LicenceLevel::By => "by",
            LicenceLevel::BySa => "by-sa",
        }
    }
    pub fn from_id(s: &str) -> Option<LicenceLevel> {
        let s = s.trim().to_ascii_lowercase().replace([' ', '_'], "-");
        match s.as_str() {
            "pd" | "cc0" | "public-domain" | "cc0/pd" => Some(LicenceLevel::Pd),
            "by" | "cc-by" => Some(LicenceLevel::By),
            "by-sa" | "cc-by-sa" => Some(LicenceLevel::BySa),
            _ => None,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            LicenceLevel::Pd => "CC0 / PD",
            LicenceLevel::By => "CC BY",
            LicenceLevel::BySa => "CC BY-SA",
        }
    }
    pub fn allows(self, l: Licence) -> bool {
        match l {
            Licence::Pd | Licence::Free => true,
            Licence::By => self >= LicenceLevel::By,
            Licence::BySa => self >= LicenceLevel::BySa,
            Licence::Other => false,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Orientation {
    #[default]
    Any,
    Landscape,
    Portrait,
    Square,
}

impl Orientation {
    pub const ALL: [Orientation; 4] = [Orientation::Any, Orientation::Landscape, Orientation::Portrait, Orientation::Square];
    pub fn id(self) -> &'static str {
        match self {
            Orientation::Any => "any",
            Orientation::Landscape => "landscape",
            Orientation::Portrait => "portrait",
            Orientation::Square => "square",
        }
    }
    pub fn from_id(s: &str) -> Option<Orientation> {
        Orientation::ALL.into_iter().find(|o| o.id().eq_ignore_ascii_case(s.trim()))
    }
    pub fn label(self) -> &'static str {
        match self {
            Orientation::Any => "Any shape",
            Orientation::Landscape => "Landscape",
            Orientation::Portrait => "Portrait",
            Orientation::Square => "Square",
        }
    }
    /// Does a `w` x `h` picture have this shape? (Square: within 10 %.)
    pub fn fits(self, w: u32, h: u32) -> bool {
        if self == Orientation::Any {
            return true;
        }
        if w == 0 || h == 0 {
            return false;
        }
        let r = w as f64 / h as f64;
        match self {
            Orientation::Any => true,
            Orientation::Landscape => r > 1.1,
            Orientation::Portrait => r < 0.9,
            Orientation::Square => (0.9..=1.1).contains(&r),
        }
    }
}

/// The search box and its filters.
#[derive(Clone, PartialEq, Eq, Debug, Hash, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Query {
    pub text: String,
    pub kind: Kind,
    pub orientation: Orientation,
    /// The shorter side must be at least this many pixels (0 = any). 720, 1080, 2160 in the UI.
    pub min_res: u32,
    pub licence: LicenceLevel,
    /// 1-based page.
    pub page: u32,
}

impl Query {
    pub fn page(&self) -> u32 {
        self.page.max(1)
    }
}

/// One search result.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Hit {
    /// `<source>:<the site's own id>`, e.g. `nasa:PIA01973`: what `stock.add` takes.
    pub id: String,
    pub source: Option<Source>,
    pub kind: Kind,
    pub title: String,
    pub author: String,
    pub author_url: String,
    /// The page of the item on its site (the credit links here).
    pub page_url: String,
    pub licence: Option<Licence>,
    /// As the site names it ("CC BY-SA 4.0").
    pub licence_name: String,
    pub licence_url: String,
    pub thumb_url: String,
    /// A small file for the hover preview (video / audio); empty = use `file_url`.
    pub preview_url: String,
    /// The file to download; empty = resolved at download time (`sources::resolve`).
    pub file_url: String,
    pub width: u32,
    pub height: u32,
    /// Seconds (0 = a still, or not known).
    pub duration: f64,
    /// A NASA item's `collection.json`, an Archive item's identifier: what `resolve` reads.
    pub detail: String,
}

impl Hit {
    pub fn licence(&self) -> Licence {
        self.licence.unwrap_or(Licence::Other)
    }
    /// Size known (videos and pictures; some sites only say after a download).
    pub fn has_size(&self) -> bool {
        self.width > 0 && self.height > 0
    }
}

/// Does `hit` pass the filters? Unknown sizes pass only while no size or shape is asked for.
pub fn passes(hit: &Hit, q: &Query) -> bool {
    if !q.licence.allows(hit.licence()) {
        return false;
    }
    if !hit.kind.is_audio() {
        if q.min_res > 0 && (!hit.has_size() || hit.width.min(hit.height) < q.min_res) {
            return false;
        }
        if !q.orientation.fits(hit.width, hit.height) {
            return false;
        }
    }
    true
}

/// The credit line for `hit` (title, author, source, licence: "TASL"), e.g.
/// `"Ocean in Sommaroy" by Netha Hussain, CC0, via Wikimedia Commons - https://commons.wikimedia.org/...`.
pub fn attribution(hit: &Hit) -> String {
    let title = if hit.title.trim().is_empty() { "Untitled".to_string() } else { hit.title.trim().to_string() };
    let mut s = format!("\"{title}\"");
    if !hit.author.trim().is_empty() {
        s.push_str(&format!(" by {}", hit.author.trim()));
    }
    let lic = if hit.licence_name.trim().is_empty() {
        match hit.licence() {
            Licence::Pd => "Public domain".to_string(),
            Licence::Free => "free licence".to_string(),
            Licence::By => "CC BY".to_string(),
            Licence::BySa => "CC BY-SA".to_string(),
            Licence::Other => "licence unknown".to_string(),
        }
    } else {
        hit.licence_name.trim().to_string()
    };
    s.push_str(&format!(", {lic}"));
    if !hit.licence_url.trim().is_empty() && hit.licence().needs_credit() {
        s.push_str(&format!(" ({})", hit.licence_url.trim()));
    }
    if let Some(src) = hit.source {
        s.push_str(&format!(", via {}", src.name()));
    }
    if !hit.page_url.trim().is_empty() {
        s.push_str(&format!(" - {}", hit.page_url.trim()));
    }
    s
}

/// What a downloaded stock file keeps beside it (`<file>.stock.json`): enough to credit it later.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Credit {
    pub id: String,
    pub source: String,
    pub title: String,
    pub author: String,
    pub page_url: String,
    pub file_url: String,
    pub licence: String,
    pub licence_url: String,
    pub needs_credit: bool,
    pub attribution: String,
}

impl Credit {
    pub fn of(hit: &Hit) -> Credit {
        Credit {
            id: hit.id.clone(),
            source: hit.source.map(|s| s.name().to_string()).unwrap_or_default(),
            title: hit.title.clone(),
            author: hit.author.clone(),
            page_url: hit.page_url.clone(),
            file_url: hit.file_url.clone(),
            licence: if hit.licence_name.is_empty() { format!("{:?}", hit.licence()) } else { hit.licence_name.clone() },
            licence_url: hit.licence_url.clone(),
            needs_credit: hit.licence().needs_credit(),
            attribution: attribution(hit),
        }
    }
}

/// `<file>.stock.json`.
pub fn credit_path(file: &Path) -> PathBuf {
    let mut s = file.as_os_str().to_os_string();
    s.push(".stock.json");
    PathBuf::from(s)
}

pub fn read_credit(file: &Path) -> Option<Credit> {
    serde_json::from_str(&std::fs::read_to_string(credit_path(file)).ok()?).ok()
}

/// The tag a stock asset carries in the Library.
pub const TAG: &str = "stock";

/// The Library asset's description for a stock file: the credit, the licence and where it came from.
pub fn asset_description(c: &Credit) -> String {
    format!("Stock: {}\nLicence: {} {}\nSource: {}", c.attribution, c.licence, c.licence_url, c.page_url)
        .trim_end()
        .to_string()
}

/// The project's credits as text: one block per stock file (credited ones first), deduplicated by id.
/// `credits` are (file name, credit) pairs in the order the assets are in the project.
pub fn credits_text(credits: &[(String, Credit)], project: &str) -> String {
    let mut seen = std::collections::HashSet::new();
    let mut list: Vec<&(String, Credit)> = credits.iter().filter(|(_, c)| seen.insert(c.id.clone())).collect();
    list.sort_by_key(|(_, c)| !c.needs_credit);
    let mut s = String::new();
    s.push_str(&format!("Credits{}\n\n", if project.is_empty() { String::new() } else { format!(" - {project}") }));
    if list.is_empty() {
        s.push_str("No stock media in this project.\n");
        return s;
    }
    for (file, c) in list {
        s.push_str(&c.attribution);
        s.push('\n');
        if !c.needs_credit {
            s.push_str("  (no credit required)\n");
        }
        s.push_str(&format!("  File: {file}\n\n"));
    }
    s
}

// ------------------------------------------------------------------------------------------- network

/// One GET: url plus headers (a keyed source's key header).
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub url: String,
    pub headers: Vec<String>,
}

/// Why a fetch failed, as the Gallery says it.
#[derive(Clone, Debug, PartialEq)]
pub enum NetError {
    Offline,
    /// HTTP error / bad key / bad response.
    Failed(String),
}

impl std::fmt::Display for NetError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            NetError::Offline => write!(f, "You're offline"),
            NetError::Failed(s) => write!(f, "{s}"),
        }
    }
}

fn curl_exe() -> Result<PathBuf, NetError> {
    crate::media::ffpipe::find_exe("curl.exe", "").ok_or(NetError::Failed("curl.exe not found".into()))
}

/// curl's exit codes for "no network": 5 (resolve proxy), 6 (resolve host), 7 (connect).
fn is_offline(code: Option<i32>) -> bool {
    matches!(code, Some(5) | Some(6) | Some(7))
}

/// GET `req` and return the body. Blocking: background threads only.
pub fn get(req: &Request, max_secs: u32) -> Result<Vec<u8>, NetError> {
    let curl = curl_exe()?;
    let mut c = crate::media::ffpipe::command(&curl);
    c.args(["--fail", "--location", "--silent", "--show-error", "--max-time", &max_secs.to_string(), "-A"])
        .arg(user_agent());
    for h in &req.headers {
        c.arg("-H").arg(h);
    }
    let out = c
        .arg(&req.url)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| NetError::Failed(format!("curl: {e}")))?;
    if !out.status.success() {
        if is_offline(out.status.code()) {
            return Err(NetError::Offline);
        }
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(NetError::Failed(if msg.is_empty() { format!("request failed ({})", out.status) } else { msg }));
    }
    Ok(out.stdout)
}

/// Download `req` into `dest` (through `<dest>.part` + rename) with progress: the size of the part file
/// against `expect` bytes (0 = unknown). Cancelling `prog` stops it and removes the part file.
pub fn download(req: &Request, dest: &Path, expect: u64, prog: &crate::engine::export::Progress) -> Result<(), String> {
    let curl = curl_exe().map_err(|e| e.to_string())?;
    if let Some(d) = dest.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let mut tmp = dest.as_os_str().to_os_string();
    tmp.push(".part");
    let tmp = PathBuf::from(tmp);
    let _ = std::fs::remove_file(&tmp);
    let mut c = crate::media::ffpipe::command(&curl);
    c.args(["--fail", "--location", "--silent", "--show-error", "--max-time", "3600", "-A"]).arg(user_agent());
    for h in &req.headers {
        c.arg("-H").arg(h);
    }
    let mut child = c
        .arg("-o")
        .arg(&tmp)
        .arg(&req.url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("curl: {e}"))?;
    let tail = crate::engine::export::stderr_tail(&mut child);
    let status = loop {
        if prog.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(&tmp);
            return Err(crate::engine::export::CANCELLED.into());
        }
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) => {}
            Err(e) => return Err(format!("curl: {e}")),
        }
        let got = std::fs::metadata(&tmp).map(|m| m.len()).unwrap_or(0);
        let f = if expect > 0 { got as f32 / expect as f32 } else { 0.0 };
        prog.set(f.clamp(0.0, 0.99), format!("Downloading… {:.1} MB", got as f64 / 1e6));
        std::thread::sleep(std::time::Duration::from_millis(150));
    };
    let msg = tail.and_then(|t| t.join().ok()).unwrap_or_default();
    if !status.success() {
        let _ = std::fs::remove_file(&tmp);
        if is_offline(status.code()) {
            return Err(NetError::Offline.to_string());
        }
        return Err(if msg.trim().is_empty() { format!("download failed ({status})") } else { msg.trim().to_string() });
    }
    std::fs::rename(&tmp, dest).map_err(|e| format!("{}: {e}", dest.display()))
}

/// The user's keys: `key(source)` returns the stored key for a keyed source.
pub type KeyFn<'a> = &'a (dyn Fn(Source) -> Option<String> + Sync);

/// Search every `sources` entry that can look for `q.kind` (in parallel), filter by `q` and interleave the
/// results source by source. Returns the hits and one line per source that failed. Blocking.
pub fn search(q: &Query, sources: &[Source], key: KeyFn) -> (Vec<Hit>, Vec<String>) {
    let wanted: Vec<Source> = sources.iter().copied().filter(|s| s.kinds().contains(&q.kind)).collect();
    let results: Vec<(Source, Result<Vec<Hit>, NetError>)> = std::thread::scope(|sc| {
        let hs: Vec<_> = wanted
            .iter()
            .map(|&s| {
                let k = if s.needs_key() { key(s) } else { None };
                sc.spawn(move || {
                    let r = match sources::request(s, q, k.as_deref()) {
                        Some(req) => get(&req, 20).and_then(|b| {
                            let text = String::from_utf8_lossy(&b);
                            parse(s, q.kind, &text).map_err(NetError::Failed)
                        }),
                        None => Ok(Vec::new()),
                    };
                    (s, r)
                })
            })
            .collect();
        hs.into_iter().filter_map(|h| h.join().ok()).collect()
    });
    let mut errors = Vec::new();
    let mut lists: Vec<Vec<Hit>> = Vec::new();
    let mut offline = 0;
    for (s, r) in results {
        match r {
            Ok(v) => lists.push(v.into_iter().filter(|h| passes(h, q)).collect()),
            Err(NetError::Offline) => offline += 1,
            Err(NetError::Failed(e)) => errors.push(format!("{}: {}", s.name(), short_err(&e))),
        }
    }
    if offline > 0 && lists.iter().all(|l| l.is_empty()) && errors.is_empty() {
        errors.push(NetError::Offline.to_string());
    } else if offline > 0 {
        errors.push(format!("{offline} source(s) unreachable"));
    }
    (interleave(lists), errors)
}

fn short_err(e: &str) -> String {
    let e = e.trim().trim_start_matches("curl: ");
    if e.contains("401") || e.contains("403") {
        "the key was refused".into()
    } else if e.contains("429") {
        "too many searches, try again in a minute".into()
    } else {
        e.chars().take(120).collect()
    }
}

/// a1 b1 c1 a2 b2 c2 ...
pub fn interleave(lists: Vec<Vec<Hit>>) -> Vec<Hit> {
    let mut its: Vec<std::vec::IntoIter<Hit>> = lists.into_iter().map(|l| l.into_iter()).collect();
    let mut out = Vec::new();
    loop {
        let mut any = false;
        for it in its.iter_mut() {
            if let Some(h) = it.next() {
                out.push(h);
                any = true;
            }
        }
        if !any {
            return out;
        }
    }
}

/// A file name for `hit` downloaded as `ext`: `<source>-<slug of the title>-<short id>.<ext>`.
pub fn file_name(hit: &Hit, ext: &str) -> String {
    let slug = crate::engine::library::slug(&hit.title);
    let slug: String = slug.chars().take(48).collect();
    let short: String = hit
        .id
        .split_once(':')
        .map_or(hit.id.as_str(), |(_, b)| b)
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(16)
        .collect();
    let src = hit.source.map_or("stock", Source::id);
    let ext = ext.trim_start_matches('.').to_ascii_lowercase();
    let ext = if ext.is_empty() || ext.len() > 5 || !ext.chars().all(|c| c.is_ascii_alphanumeric()) {
        default_ext(hit.kind).to_string()
    } else {
        ext
    };
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        format!("{src}-{short}.{ext}")
    } else {
        format!("{src}-{}-{short}.{ext}", slug.trim_end_matches('-'))
    }
}

fn default_ext(k: Kind) -> &'static str {
    match k {
        Kind::Video => "mp4",
        Kind::Image => "jpg",
        Kind::Music | Kind::Sfx => "mp3",
    }
}

/// The extension of a URL's file (`.../a.webm?x=1` -> "webm").
pub fn url_ext(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or("");
    let last = path.rsplit('/').next().unwrap_or("");
    match last.rsplit_once('.') {
        Some((_, e)) if (1..=5).contains(&e.len()) && e.chars().all(|c| c.is_ascii_alphanumeric()) => e.to_ascii_lowercase(),
        _ => String::new(),
    }
}

/// Download `hit` into `dir` (resolving its file first when the site needs a second request), write its
/// `<file>.stock.json` credit beside it and return the file. A file already there is reused. Blocking.
pub fn fetch(hit: &Hit, dir: &Path, key: Option<&str>, prog: &crate::engine::export::Progress) -> Result<PathBuf, String> {
    prog.set(0.0, "Finding the file…");
    let mut hit = hit.clone();
    let mut expect = 0;
    if hit.file_url.is_empty() {
        let (url, size) = sources::resolve(&hit).map_err(|e| e.to_string())?;
        hit.file_url = url;
        expect = size;
    }
    let ext = url_ext(&hit.file_url);
    let dest = dir.join(file_name(&hit, &ext));
    if !dest.is_file() {
        let req = sources::file_request(&hit, key);
        download(&req, &dest, expect, prog)?;
    }
    let credit = Credit::of(&hit);
    let json = serde_json::to_string_pretty(&credit).map_err(|e| e.to_string())?;
    std::fs::write(credit_path(&dest), json).map_err(|e| format!("{}: {e}", dest.display()))?;
    prog.set(1.0, "Done");
    Ok(dest)
}

#[cfg(test)]
mod tests;
