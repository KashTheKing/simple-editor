//! ---- ws:online-library ----
//! The library on this PC: `%APPDATA%\SimpleEditor\library\<kind>\<id>.<ext>` holds every installed
//! item (downloaded from the online catalogue, or saved by the user / an agent). The file formats are
//! specified in the repo's `library/README.md`. This file is the shared core every creator-kit
//! workstream reads and writes installed items through; the catalogue (index.json, downloads) is
//! built on top of it.

use std::path::{Path, PathBuf};

/// What a library item is. The folder / extension pairs match the repo's `library/` layout.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Kind {
    Effect,
    Transition,
    Lut,
    Html,
    Icon,
    Skill,
}

impl Kind {
    pub const ALL: [Kind; 6] = [Kind::Effect, Kind::Transition, Kind::Lut, Kind::Html, Kind::Icon, Kind::Skill];
    /// The `kind` string of `index.json` and of the `library.*` tools.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Effect => "effect",
            Kind::Transition => "transition",
            Kind::Lut => "lut",
            Kind::Html => "html",
            Kind::Icon => "icon",
            Kind::Skill => "skill",
        }
    }
    pub fn from_name(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.name() == s)
    }
    pub fn dir(self) -> &'static str {
        match self {
            Kind::Effect => "effects",
            Kind::Transition => "transitions",
            Kind::Lut => "luts",
            Kind::Html => "html",
            Kind::Icon => "icons",
            Kind::Skill => "skills",
        }
    }
    pub fn ext(self) -> &'static str {
        match self {
            Kind::Effect | Kind::Transition => "glsl",
            Kind::Lut => "cube",
            Kind::Html => "html",
            Kind::Icon => "svg",
            Kind::Skill => "md",
        }
    }
}

/// One installed file.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub kind: Kind,
    /// The file name without its extension (`paper-tear`).
    pub id: String,
    pub path: PathBuf,
}

/// `%APPDATA%\SimpleEditor\library`.
pub fn root() -> PathBuf {
    crate::settings::Settings::dir().join("library")
}

/// "Paper Tear!" -> "paper-tear": the id / file name a display name gets.
pub fn slug(name: &str) -> String {
    let mut s = String::new();
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c.to_ascii_lowercase());
        } else if !s.is_empty() && !s.ends_with('-') {
            s.push('-');
        }
    }
    s.trim_end_matches('-').to_string()
}

/// Where `kind`/`id` lives (whether or not it is installed).
pub fn path_of(kind: Kind, id: &str) -> PathBuf {
    path_in(&root(), kind, id)
}

fn path_in(root: &Path, kind: Kind, id: &str) -> PathBuf {
    root.join(kind.dir()).join(format!("{}.{}", slug(id), kind.ext()))
}

/// Every installed item of `kind`, by id.
pub fn installed(kind: Kind) -> Vec<Item> {
    installed_in(&root(), kind)
}

fn installed_in(root: &Path, kind: Kind) -> Vec<Item> {
    let Ok(dir) = std::fs::read_dir(root.join(kind.dir())) else { return Vec::new() };
    let mut out: Vec<Item> = dir
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case(kind.ext())))
        .filter_map(|p| Some(Item { kind, id: p.file_stem()?.to_string_lossy().into_owned(), path: p }))
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// The text of an installed item.
pub fn read(kind: Kind, id: &str) -> Result<String, String> {
    let p = path_of(kind, id);
    std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))
}

/// Install (or replace) an item: `id` is slugged, the file is written beside its final name and
/// renamed, so a reader never sees half a file.
pub fn save(kind: Kind, id: &str, text: &str) -> Result<Item, String> {
    save_in(&root(), kind, id, text.as_bytes())
}

fn save_in(root: &Path, kind: Kind, id: &str, bytes: &[u8]) -> Result<Item, String> {
    let id = slug(id);
    if id.is_empty() {
        return Err("a library item needs a name".into());
    }
    let path = path_in(root, kind, &id);
    let dir = path.parent().unwrap_or(root);
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let tmp = path.with_extension(format!("{}.part", kind.ext()));
    std::fs::write(&tmp, bytes).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Item { kind, id, path })
}

/// Remove an installed item (true when a file was deleted).
pub fn remove(kind: Kind, id: &str) -> bool {
    std::fs::remove_file(path_of(kind, id)).is_ok()
}

// ---------------------------------------------------------------- the online catalogue

impl Kind {
    pub fn from_dir(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.dir() == s)
    }
}

/// One `index.json` item (format: the repo's `library/README.md`). Field order = the file's key order.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct Entry {
    /// `<kind dir>/<file id>`, e.g. `transitions/paper-tear`.
    pub id: String,
    pub kind: String,
    pub name: String,
    pub desc: String,
    pub tags: Vec<String>,
    /// Relative to the library root, e.g. `transitions/paper-tear.glsl`.
    pub file: String,
    pub size: u64,
    pub credit: String,
    /// Relative to the library root (`thumbs/transitions-paper-tear.jpg`); empty for skills.
    pub thumb: String,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct Catalogue {
    pub version: u32,
    pub items: Vec<Entry>,
}

/// `kind` + file id -> the id every tool and the Gallery use.
pub fn full_id(kind: Kind, id: &str) -> String {
    format!("{}/{}", kind.dir(), id)
}

/// `transitions/paper-tear` -> (Transition, "paper-tear"); None for anything else (unknown kind dir, an
/// id that is not already a slug).
pub fn split_id(full: &str) -> Option<(Kind, String)> {
    let (dir, id) = full.split_once('/')?;
    let kind = Kind::from_dir(dir)?;
    (!id.is_empty() && slug(id) == id).then(|| (kind, id.to_string()))
}

/// `transitions/paper-tear` -> `transitions/paper-tear.glsl` (the file, relative to the library root).
pub fn file_of(full: &str) -> Option<String> {
    let (kind, id) = split_id(full)?;
    Some(format!("{}/{id}.{}", kind.dir(), kind.ext()))
}

/// The thumbnail an item has under the library root (`thumbs/<kind dir>-<id>.jpg`; skills: none).
pub fn thumb_of(kind: Kind, id: &str) -> String {
    if kind == Kind::Skill {
        String::new()
    } else {
        format!("thumbs/{}-{id}.jpg", kind.dir())
    }
}

/// A catalogue-relative path the app may read: lowercase ASCII, no `..`, no root, no drive. Guards the
/// fetchers against a hostile `index.json`.
fn safe_rel(rel: &str) -> bool {
    !rel.is_empty()
        && !rel.starts_with('/')
        && !rel.contains("..")
        && rel.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-_./".contains(&b))
}

/// Where the catalogue comes from: `Settings.library_dir` when set (files read straight from it), else
/// `Settings.library_url` (files fetched with `curl.exe` into `cache_root()`).
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    Dir(PathBuf),
    Url(String),
}

impl Source {
    pub fn of(settings: &crate::settings::Settings) -> Source {
        let dir = settings.library_dir.trim();
        if dir.is_empty() {
            Source::Url(settings.library_url.trim().trim_end_matches('/').to_string())
        } else {
            Source::Dir(PathBuf::from(dir))
        }
    }
    pub fn label(&self) -> String {
        match self {
            Source::Dir(d) => d.display().to_string(),
            Source::Url(u) => u.clone(),
        }
    }
    /// Where `rel` is (or will be) on this PC: in the folder, or its cached copy.
    pub fn local(&self, rel: &str) -> PathBuf {
        self.local_in(&cache_root(), rel)
    }
    fn local_in(&self, cache: &Path, rel: &str) -> PathBuf {
        match self {
            Source::Dir(d) => d.join(rel),
            Source::Url(_) => cache.join(rel),
        }
    }
    pub fn url(&self, rel: &str) -> Option<String> {
        match self {
            Source::Dir(_) => None,
            Source::Url(u) => Some(format!("{u}/{rel}")),
        }
    }
}

/// `%LOCALAPPDATA%\SimpleEditor\cache\library`: the cached `index.json`, thumbnails and item files.
pub fn cache_root() -> PathBuf {
    crate::settings::Settings::cache_dir().join("library")
}

/// `curl.exe` `url` into `dest` through `<dest>.part` + rename, so a reader never sees half a file.
/// Blocking: background threads only.
fn curl_get(url: &str, dest: &Path) -> Result<(), String> {
    let curl = crate::media::ffpipe::find_exe("curl.exe", "").ok_or("curl.exe not found")?;
    if let Some(d) = dest.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let mut tmp = dest.as_os_str().to_os_string();
    tmp.push(".part");
    let tmp = PathBuf::from(tmp);
    let out = crate::media::ffpipe::command(&curl)
        .args(["--fail", "--location", "--silent", "--show-error", "--max-time", "60", "-o"])
        .arg(&tmp)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("curl: {e}"))?;
    if !out.status.success() {
        let _ = std::fs::remove_file(&tmp);
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if msg.is_empty() { format!("download failed ({})", out.status) } else { msg });
    }
    std::fs::rename(&tmp, dest).map_err(|e| format!("{}: {e}", dest.display()))
}

/// The bytes of catalogue file `rel`: read from the folder, or downloaded into the cache first (an
/// existing cached copy is reused when `reuse`).
fn fetch(src: &Source, rel: &str, reuse: bool) -> Result<Vec<u8>, String> {
    if !safe_rel(rel) {
        return Err(format!("bad library path '{rel}'"));
    }
    let p = src.local(rel);
    if let Some(url) = src.url(rel) {
        if !(reuse && p.is_file()) {
            curl_get(&url, &p)?;
        }
    }
    std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))
}

/// Parse `index.json`, dropping items whose id / file is not one the app would read.
pub fn parse_catalogue(text: &str) -> Result<Catalogue, String> {
    let mut c: Catalogue = serde_json::from_str(text).map_err(|e| format!("index.json: {e}"))?;
    c.items.retain(|e| file_of(&e.id).is_some_and(|f| f == e.file) && (e.thumb.is_empty() || safe_rel(&e.thumb)));
    Ok(c)
}

/// The catalogue already on this PC (the folder's, or the cached download), without any network.
pub fn cached_catalogue(src: &Source) -> Option<Catalogue> {
    parse_catalogue(&std::fs::read_to_string(src.local("index.json")).ok()?).ok()
}

/// True when the cached `index.json` should be fetched again (missing, or older than a day). A local
/// folder is always re-read (it is cheap).
pub fn catalogue_stale(src: &Source) -> bool {
    let age = std::fs::metadata(src.local("index.json")).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok());
    matches!(src, Source::Dir(_)) || age.is_none_or(|a| a.as_secs() > 24 * 3600)
}

/// Fetch (or re-read) `index.json`. Blocking: background threads only.
pub fn fetch_catalogue(src: &Source) -> Result<Catalogue, String> {
    let bytes = fetch(src, "index.json", false)?;
    parse_catalogue(&String::from_utf8_lossy(&bytes))
}

/// Download (or copy) item `full` (`<kind dir>/<id>`) and install it. A LUT must parse. Blocking.
pub fn fetch_item(src: &Source, full: &str) -> Result<Item, String> {
    fetch_item_into(src, &root(), full)
}

fn fetch_item_into(src: &Source, root: &Path, full: &str) -> Result<Item, String> {
    let (kind, id) = split_id(full).ok_or_else(|| format!("no such library item '{full}'"))?;
    let bytes = fetch(src, &file_of(full).unwrap_or_default(), false)?;
    let text = String::from_utf8(bytes).map_err(|_| format!("{full}: not UTF-8 text"))?;
    if kind == Kind::Lut {
        crate::engine::lut::parse_cube(&text).map_err(|e| format!("{full}: {e}"))?;
    }
    save_in(root, kind, &id, text.as_bytes())
}

/// The thumbnail file for `rel` on this PC (downloaded into the cache when needed). Blocking.
pub fn fetch_thumb(src: &Source, rel: &str) -> Result<PathBuf, String> {
    fetch(src, rel, true).map(|_| src.local(rel))
}

/// Every installed item, as full ids.
pub fn installed_ids() -> std::collections::HashSet<String> {
    installed_ids_in(&root())
}

fn installed_ids_in(root: &Path) -> std::collections::HashSet<String> {
    Kind::ALL.into_iter().flat_map(|k| installed_in(root, k)).map(|i| full_id(i.kind, &i.id)).collect()
}

/// One Gallery cell / `library.list` row: a catalogue item (installed or not) or an installed item
/// the catalogue does not have ("yours": saved by the user or an agent).
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: String,
    pub kind: Kind,
    pub name: String,
    pub desc: String,
    pub tags: Vec<String>,
    pub installed: bool,
    pub yours: bool,
    pub thumb: String,
}

impl Row {
    /// Every whitespace-separated word of `query` appears in the name, id, description or tags.
    pub fn matches(&self, query: &str) -> bool {
        let hay = format!("{} {} {} {}", self.name, self.id, self.desc, self.tags.join(" ")).to_lowercase();
        query.to_lowercase().split_whitespace().all(|w| hay.contains(w))
    }
}

/// Catalogue items in catalogue order, then installed items it does not list ("yours"), by id.
pub fn merge(cat: &[Entry], installed: &std::collections::HashSet<String>) -> Vec<Row> {
    let mut out: Vec<Row> = cat
        .iter()
        .filter_map(|e| {
            let (kind, _) = split_id(&e.id)?;
            Some(Row {
                id: e.id.clone(),
                kind,
                name: e.name.clone(),
                desc: e.desc.clone(),
                tags: e.tags.clone(),
                installed: installed.contains(&e.id),
                yours: false,
                thumb: e.thumb.clone(),
            })
        })
        .collect();
    let mut yours: Vec<&String> = installed.iter().filter(|id| !cat.iter().any(|e| &e.id == *id)).collect();
    yours.sort();
    for id in yours {
        let Some((kind, short)) = split_id(id) else { continue };
        out.push(Row {
            id: id.clone(),
            kind,
            name: short.replace('-', " "),
            desc: String::new(),
            tags: Vec::new(),
            installed: true,
            yours: true,
            thumb: String::new(),
        });
    }
    out
}

/// (name, desc, tags, credit) from a library file's own header (`library/README.md`): `@name` lines
/// for shaders, `# @name` comments for `.cube`, `<title>` / `se:desc` / `se:tags` for HTML, `<title>`
/// / `<desc>` for icons (the desc is also the tags), the `---` header for skills.
pub fn meta_of(kind: Kind, text: &str) -> (String, String, Vec<String>, String) {
    let tags = |s: &str| -> Vec<String> {
        s.split(',').map(|t| t.trim().to_ascii_lowercase()).filter(|t| !t.is_empty()).collect()
    };
    let between = |open: &str, close: &str| -> String {
        text.split_once(open)
            .and_then(|(_, r)| r.split_once(close))
            .map(|(v, _)| v.trim().to_string())
            .unwrap_or_default()
    };
    match kind {
        Kind::Effect | Kind::Transition => {
            let m = crate::engine::shaderlib::parse_meta(text);
            (m.name, m.desc, m.tags, m.credit)
        }
        Kind::Lut => {
            let key = |k: &str| -> String {
                text.lines()
                    .map_while(|l| l.trim().strip_prefix('#'))
                    .find_map(|l| l.trim().strip_prefix(k).filter(|v| v.starts_with(char::is_whitespace)))
                    .map(|v| v.trim().to_string())
                    .unwrap_or_default()
            };
            (key("@name"), key("@desc"), tags(&key("@tags")), key("@credit"))
        }
        Kind::Html => {
            let meta = |name: &str| -> String {
                let needle = format!("name=\"{name}\"");
                text.split("<meta")
                    .skip(1)
                    .filter_map(|m| m.split_once('>').map(|(a, _)| a))
                    .find(|a| a.contains(&needle))
                    .and_then(|a| a.split_once("content=\"")?.1.split_once('"').map(|(v, _)| v.trim().to_string()))
                    .unwrap_or_default()
            };
            (between("<title>", "</title>"), meta("se:desc"), tags(&meta("se:tags")), String::new())
        }
        Kind::Icon => {
            let d = between("<desc>", "</desc>");
            (between("<title>", "</title>"), d.clone(), tags(&d), String::new())
        }
        Kind::Skill => {
            let head = text.trim_start_matches('\u{feff}').strip_prefix("---").and_then(|r| r.split_once("\n---"));
            let key = |k: &str| -> String {
                head.into_iter()
                    .flat_map(|(h, _)| h.lines())
                    .find_map(|l| l.split_once(':').filter(|(a, _)| a.trim() == k).map(|(_, v)| v.trim()))
                    .map(|v| v.trim_matches('"').to_string())
                    .unwrap_or_default()
            };
            (key("name"), key("description"), Vec::new(), String::new())
        }
    }
}

/// The catalogue entry a file makes (what `scripts/library-index.ps1` writes into `index.json`).
pub fn entry_of(kind: Kind, id: &str, text: &str, size: u64) -> Entry {
    let (name, desc, tags, credit) = meta_of(kind, text);
    Entry {
        id: full_id(kind, id),
        kind: kind.name().into(),
        name,
        desc,
        tags,
        file: format!("{}/{id}.{}", kind.dir(), kind.ext()),
        size,
        credit,
        thumb: thumb_of(kind, id),
    }
}

#[cfg(test)]
mod index_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("se-lib-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn library_ids_paths_and_urls_map_both_ways() {
        assert_eq!(split_id("transitions/paper-tear"), Some((Kind::Transition, "paper-tear".into())));
        assert_eq!(file_of("luts/teal-orange").as_deref(), Some("luts/teal-orange.cube"));
        assert_eq!(full_id(Kind::Icon, "bell"), "icons/bell");
        for bad in ["paper-tear", "nope/x", "effects/", "effects/Bad Name", "effects/../x"] {
            assert_eq!(split_id(bad), None, "{bad}");
        }
        assert_eq!(thumb_of(Kind::Html, "bell-ring"), "thumbs/html-bell-ring.jpg");
        assert_eq!(thumb_of(Kind::Skill, "captions"), "");
        let u = Source::Url("https://x.test/lib".into());
        assert_eq!(u.url("index.json").as_deref(), Some("https://x.test/lib/index.json"));
        assert_eq!(u.local_in(Path::new("C:/c"), "thumbs/a.jpg"), Path::new("C:/c").join("thumbs/a.jpg"));
        let d = Source::Dir(PathBuf::from("D:/lib"));
        assert_eq!((d.url("index.json"), d.local("x/y.md")), (None, PathBuf::from("D:/lib").join("x/y.md")));
        let mut s = crate::settings::Settings::default();
        s.library_url = "https://x.test/lib/".into();
        assert_eq!(Source::of(&s), Source::Url("https://x.test/lib".into()));
        s.library_dir = "D:/lib".into();
        assert_eq!(Source::of(&s), d);
        for bad in ["../x", "/abs", "a/../../b", "C:/x", "a\\b", ""] {
            assert!(!safe_rel(bad), "{bad}");
        }
    }

    #[test]
    fn library_catalogue_parses_and_merges_with_installed() {
        let json = r#"{"version":1,"items":[
            {"id":"transitions/paper-tear","kind":"transition","name":"Paper Tear","desc":"Rips","tags":["paper"],
             "file":"transitions/paper-tear.glsl","size":10,"credit":"","thumb":"thumbs/transitions-paper-tear.jpg"},
            {"id":"luts/warm","kind":"lut","name":"Warm","file":"luts/warm.cube"},
            {"id":"luts/evil","kind":"lut","name":"Evil","file":"../../evil.cube"}]}"#;
        let c = parse_catalogue(json).unwrap();
        assert_eq!(c.items.len(), 2, "an item whose file does not match its id is dropped");
        let installed: std::collections::HashSet<String> =
            ["luts/warm", "skills/mine"].into_iter().map(String::from).collect();
        let rows = merge(&c.items, &installed);
        let got: Vec<(&str, bool, bool)> = rows.iter().map(|r| (r.id.as_str(), r.installed, r.yours)).collect();
        assert_eq!(
            got,
            [("transitions/paper-tear", false, false), ("luts/warm", true, false), ("skills/mine", true, true)]
        );
        assert_eq!(rows[2].name, "mine");
        assert!(rows[0].matches("PAPER rip") && !rows[0].matches("paper fire"));
        assert!(parse_catalogue("not json").is_err());
    }

    #[test]
    fn library_fetch_from_a_local_folder_installs_items() {
        let lib = temp("src");
        let root = temp("root");
        std::fs::create_dir_all(lib.join("luts")).unwrap();
        std::fs::create_dir_all(lib.join("skills")).unwrap();
        let cube = "# @name Id\nLUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";
        std::fs::write(lib.join("luts/id.cube"), cube).unwrap();
        std::fs::write(lib.join("luts/bad.cube"), "LUT_3D_SIZE 2\n0 0 0\n").unwrap();
        std::fs::write(lib.join("skills/s.md"), "---\nname: s\n---\nbody").unwrap();
        std::fs::write(lib.join("index.json"), r#"{"version":1,"items":[]}"#).unwrap();
        let src = Source::Dir(lib.clone());
        assert!(catalogue_stale(&src), "a folder is always re-read");
        assert_eq!(cached_catalogue(&src), Some(Catalogue { version: 1, items: vec![] }));
        let it = fetch_item_into(&src, &root, "luts/id").unwrap();
        assert_eq!(std::fs::read_to_string(&it.path).unwrap(), cube);
        assert!(fetch_item_into(&src, &root, "luts/bad").is_err(), "a LUT that does not parse is not installed");
        assert!(fetch_item_into(&src, &root, "luts/missing").is_err());
        fetch_item_into(&src, &root, "skills/s").unwrap();
        let ids = installed_ids_in(&root);
        assert!(ids.contains("luts/id") && ids.contains("skills/s") && ids.len() == 2, "{ids:?}");
        assert!(fetch_thumb(&src, "thumbs/none.jpg").is_err());
        let _ = (std::fs::remove_dir_all(&lib), std::fs::remove_dir_all(&root));
    }

    #[test]
    fn library_meta_reads_each_kind_header() {
        let html = "<title>Bell</title>\n<meta name=\"se:kind\" content=\"layer\">\n\
                    <meta name=\"se:desc\" content=\"Rings.\">\n<meta name=\"se:tags\" content=\"Bell, alert\">";
        assert_eq!(
            meta_of(Kind::Html, html),
            ("Bell".into(), "Rings.".into(), vec!["bell".into(), "alert".into()], "".into())
        );
        let svg = "<svg><title>Like</title><desc>thumbs up, like</desc></svg>";
        assert_eq!(meta_of(Kind::Icon, svg).2, vec!["thumbs up".to_string(), "like".into()]);
        let md = "---\nname: captions\ndescription: Add captions.\nargs: \"[x]\"\n---\nbody";
        assert_eq!(meta_of(Kind::Skill, md).0, "captions");
        assert_eq!(meta_of(Kind::Skill, md).1, "Add captions.");
        let cube = "# @name Teal\n# @desc Warm skin.\n# @tags a, B\nTITLE \"Teal\"\nLUT_3D_SIZE 2\n";
        assert_eq!(
            meta_of(Kind::Lut, cube),
            ("Teal".into(), "Warm skin.".into(), vec!["a".into(), "b".into()], "".into())
        );
        let e = entry_of(Kind::Lut, "teal", cube, 5);
        assert_eq!(
            (e.id.as_str(), e.file.as_str(), e.thumb.as_str(), e.kind.as_str()),
            ("luts/teal", "luts/teal.cube", "thumbs/luts-teal.jpg", "lut")
        );
    }

    #[test]
    fn save_list_read_remove_round_trip() {
        let root = std::env::temp_dir().join(format!("se-library-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        assert!(installed_in(&root, Kind::Transition).is_empty(), "a missing folder is an empty library");
        let a = save_in(&root, Kind::Transition, "Paper Tear!", b"// @name Paper Tear\n").unwrap();
        assert_eq!((a.id.as_str(), a.path.file_name().unwrap().to_str().unwrap()), ("paper-tear", "paper-tear.glsl"));
        save_in(&root, Kind::Transition, "blinds", b"x").unwrap();
        save_in(&root, Kind::Skill, "captions", b"y").unwrap();
        let ids: Vec<String> = installed_in(&root, Kind::Transition).into_iter().map(|i| i.id).collect();
        assert_eq!(ids, ["blinds", "paper-tear"], "sorted, one kind only, no .part leftovers");
        assert!(save_in(&root, Kind::Lut, " !! ", b"").is_err());
        assert_eq!(slug("  Teal & Orange 2 "), "teal-orange-2");
        for k in Kind::ALL {
            assert_eq!(Kind::from_name(k.name()), Some(k));
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
