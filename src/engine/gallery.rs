//! Your own Gallery, on disk: `%APPDATA%\SimpleEditor\gallery\<kind>\<name>.json` (a sound is the audio file
//! itself in `sounds\`, with an optional `<name>.json` beside it). Plain files - copy or zip a folder to share
//! it; the receiver drops it into their gallery folder or uses "Import to Gallery...".
//!
//! One item = one file in the envelope format below (documented in `website/docs/my-gallery.mdx`):
//! ```json
//! {"format": "simple-editor-gallery", "version": 1, "kind": "look", "name": "Warm film",
//!  "tags": ["grade"], "created": 1731500000, "data": { ... the kind's payload ... }}
//! ```
//! `index.json` beside the kind folders lists every item (kind, file, name, tags); it is only a convenience
//! and is rebuilt from the files whenever it is missing or unreadable (`ensure_index`). Every function takes
//! the gallery `root` so tests run in a temp folder. Deleting moves a file to `.trash\` (nothing is lost);
//! a name that is taken gets a numbered suffix (`Name (2)`), nothing is overwritten unless asked (`Replace`).

use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const FORMAT: &str = "simple-editor-gallery";
pub const VERSION: u64 = 1;

/// What an item is. The folder names are part of the on-disk format.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Kind {
    /// An effect stack (Gallery > Looks): replaces a clip's effects.
    Look,
    /// One or more effects with their settings (Gallery > Effects): added to a clip's stack.
    Effect,
    /// A transition with its duration / colour / direction / shader and controls (Gallery > Transitions).
    Transition,
    /// A subtitle style (Gallery > Captions).
    Caption,
    /// A speed curve (Gallery > Speed).
    Speed,
    /// A keyframe block (Gallery > Animate).
    Animate,
    /// A group of clips, titles included (Gallery > Templates / Titles).
    Template,
    /// An audio file (Gallery > Sounds).
    Sound,
}

impl Kind {
    pub const ALL: [Kind; 8] = [
        Kind::Look,
        Kind::Effect,
        Kind::Transition,
        Kind::Caption,
        Kind::Speed,
        Kind::Animate,
        Kind::Template,
        Kind::Sound,
    ];
    /// The `kind` string inside a file, and `gallery.save`'s `kind` argument.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Look => "look",
            Kind::Effect => "effect",
            Kind::Transition => "transition",
            Kind::Caption => "caption",
            Kind::Speed => "speed",
            Kind::Animate => "animate",
            Kind::Template => "template",
            Kind::Sound => "sound",
        }
    }
    pub fn from_name(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.name().eq_ignore_ascii_case(s.trim()))
    }
    pub fn dir(self) -> &'static str {
        match self {
            Kind::Look => "looks",
            Kind::Effect => "effects",
            Kind::Transition => "transitions",
            Kind::Caption => "captions",
            Kind::Speed => "speed",
            Kind::Animate => "animate",
            Kind::Template => "templates",
            Kind::Sound => "sounds",
        }
    }
}

/// `%APPDATA%\SimpleEditor\gallery`. (A test never sees the real one: its own folder, or `set_test_root`.)
pub fn root() -> PathBuf {
    #[cfg(test)]
    {
        return TEST_ROOT
            .with(|r| r.borrow().clone())
            .unwrap_or_else(|| std::env::temp_dir().join(format!("se-gallery-test-root-{}", std::process::id())));
    }
    #[cfg(not(test))]
    crate::settings::Settings::dir().join("gallery")
}

#[cfg(test)]
thread_local! {
    static TEST_ROOT: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// Tests: this thread's gallery folder.
#[cfg(test)]
pub fn set_test_root(p: Option<PathBuf>) {
    TEST_ROOT.with(|r| *r.borrow_mut() = p);
}

/// One saved item.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub kind: Kind,
    /// The file stem (`warm-film`): what the item is called on disk.
    pub slug: String,
    pub name: String,
    pub tags: Vec<String>,
    pub created: u64,
    /// The `.json` file (a sound: its sidecar, which may not exist) or the audio file for `Sound`.
    pub path: PathBuf,
    /// The payload (`Null` for a sound).
    pub data: Value,
    /// A sound's audio file.
    pub file: Option<PathBuf>,
}

/// What to do when the name is taken.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Collide {
    /// `Name (2)`, `Name (3)`...: nothing is overwritten.
    Number,
    /// Overwrite the item of that name (only where the user asked for it).
    Replace,
}

pub fn is_audio(p: &Path) -> bool {
    const EXT: [&str; 10] = ["wav", "mp3", "ogg", "flac", "m4a", "aac", "opus", "aif", "aiff", "wma"];
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| EXT.contains(&e.to_ascii_lowercase().as_str()))
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn slug(name: &str) -> String {
    let s = crate::engine::library::slug(name);
    if s.is_empty() {
        "item".into()
    } else {
        s
    }
}

fn dir_of(root: &Path, kind: Kind) -> PathBuf {
    root.join(kind.dir())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    let tmp = path.with_extension("part");
    std::fs::write(&tmp, bytes).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

fn envelope(kind: Kind, name: &str, tags: &[String], created: u64, data: &Value) -> Value {
    json!({"format": FORMAT, "version": VERSION, "kind": kind.name(), "name": name, "tags": tags,
           "created": created, "data": data})
}

fn parse_item(kind: Kind, path: &Path) -> Option<Item> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    if v.get("format").and_then(Value::as_str) != Some(FORMAT) {
        return None;
    }
    let k = Kind::from_name(v.get("kind")?.as_str()?)?;
    if k != kind {
        return None;
    }
    let slug = path.file_stem()?.to_string_lossy().into_owned();
    Some(Item {
        kind,
        name: v.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).unwrap_or(&slug).to_string(),
        slug,
        tags: tags_of(&v),
        created: v.get("created").and_then(Value::as_u64).unwrap_or(0),
        path: path.to_path_buf(),
        data: v.get("data").cloned().unwrap_or(Value::Null),
        file: None,
    })
}

fn tags_of(v: &Value) -> Vec<String> {
    v.get("tags").and_then(Value::as_array).map_or_else(Vec::new, |a| {
        a.iter().filter_map(|t| t.as_str()).map(str::trim).filter(|t| !t.is_empty()).map(String::from).collect()
    })
}

/// "a, b ,c" -> ["a","b","c"] (lower-cased, no empties, no duplicates).
pub fn parse_tags(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in s.split([',', ';']) {
        let t = t.trim().to_lowercase();
        if !t.is_empty() && !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

/// Every item of `kind`, by name. A file that is not an item of that kind is ignored, never an error.
pub fn list(root: &Path, kind: Kind) -> Vec<Item> {
    let Ok(rd) = std::fs::read_dir(dir_of(root, kind)) else { return Vec::new() };
    let mut out: Vec<Item> = Vec::new();
    for e in rd.flatten() {
        let p = e.path();
        if kind == Kind::Sound {
            if !is_audio(&p) {
                continue;
            }
            let slug = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let side = p.with_extension("json");
            let meta: Value =
                std::fs::read_to_string(&side).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
            out.push(Item {
                kind,
                name: meta.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).unwrap_or(&slug).to_string(),
                slug,
                tags: tags_of(&meta),
                created: meta.get("created").and_then(Value::as_u64).unwrap_or(0),
                path: side,
                data: meta.get("data").cloned().unwrap_or(Value::Null),
                file: Some(p),
            });
        } else if p.extension().is_some_and(|e| e == "json") {
            out.extend(parse_item(kind, &p));
        }
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.slug.cmp(&b.slug)));
    out
}

pub fn find(root: &Path, kind: Kind, name: &str) -> Option<Item> {
    let items = list(root, kind);
    items
        .iter()
        .find(|i| i.name.eq_ignore_ascii_case(name.trim()))
        .or_else(|| items.iter().find(|i| i.slug == slug(name)))
        .cloned()
}

/// A name and file stem not used by any item of `kind` yet: `Name`, then `Name (2)`...
fn free_name(root: &Path, kind: Kind, name: &str, ext: &str) -> (String, String) {
    let taken = |n: &str| {
        let s = slug(n);
        dir_of(root, kind).join(format!("{s}.{ext}")).exists() || dir_of(root, kind).join(format!("{s}.json")).exists()
    };
    let base = name.trim();
    if !taken(base) {
        return (base.to_string(), slug(base));
    }
    (2..).map(|i| format!("{base} ({i})")).find(|n| !taken(n)).map(|n| (n.clone(), slug(&n))).expect("endless")
}

/// Save a JSON item. `Number`: a taken name becomes `Name (2)`. `Replace`: the item of that name is
/// overwritten (its tags and creation time are kept unless `tags` is given).
pub fn save(root: &Path, kind: Kind, name: &str, tags: &[String], data: Value, how: Collide) -> Result<Item, String> {
    if kind == Kind::Sound {
        return Err("a sound is a file: use import_file".into());
    }
    let name = name.trim();
    if name.is_empty() {
        return Err("a Gallery item needs a name".into());
    }
    let (name, stem, created, tags) = match (how, find(root, kind, name)) {
        (Collide::Replace, Some(old)) => {
            (old.name.clone(), old.slug.clone(), old.created, if tags.is_empty() { old.tags } else { tags.to_vec() })
        }
        _ => {
            let (n, s) = free_name(root, kind, name, "json");
            (n, s, now(), tags.to_vec())
        }
    };
    let path = dir_of(root, kind).join(format!("{stem}.json"));
    let text =
        serde_json::to_string_pretty(&envelope(kind, &name, &tags, created, &data)).map_err(|e| e.to_string())?;
    write_atomic(&path, text.as_bytes())?;
    let _ = write_index(root);
    Ok(Item { kind, slug: stem, name, tags, created, path, data, file: None })
}

/// Copy an audio file into `sounds\` (a taken file name is numbered) with a sidecar for its name / tags.
pub fn import_file(root: &Path, src: &Path, tags: &[String], duration: Option<f64>) -> Result<Item, String> {
    import_file_as(root, src, None, tags, duration)
}

/// `import_file` under a given display name (a shared sidecar's), numbered if taken.
pub fn import_file_as(
    root: &Path,
    src: &Path,
    name: Option<&str>,
    tags: &[String],
    duration: Option<f64>,
) -> Result<Item, String> {
    if !is_audio(src) {
        return Err(format!("{}: not an audio file", src.display()));
    }
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("wav").to_ascii_lowercase();
    let name = name
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(String::from)
        .or_else(|| src.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "sound".into());
    let (name, stem) = free_name(root, Kind::Sound, &name, &ext);
    let dest = dir_of(root, Kind::Sound).join(format!("{stem}.{ext}"));
    std::fs::create_dir_all(dest.parent().unwrap_or(root)).map_err(|e| e.to_string())?;
    std::fs::copy(src, &dest).map_err(|e| format!("{}: {e}", src.display()))?;
    let side = dest.with_extension("json");
    let data = duration.map_or(Value::Null, |d| json!({"duration": d}));
    let text =
        serde_json::to_string_pretty(&envelope(Kind::Sound, &name, tags, now(), &data)).map_err(|e| e.to_string())?;
    write_atomic(&side, text.as_bytes())?;
    let _ = write_index(root);
    Ok(Item {
        kind: Kind::Sound,
        slug: stem,
        name,
        tags: tags.to_vec(),
        created: now(),
        path: side,
        data,
        file: Some(dest),
    })
}

fn rewrite(item: &Item, name: &str, tags: &[String], data: &Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(&envelope(item.kind, name, tags, item.created, data))
        .map_err(|e| e.to_string())?;
    write_atomic(&item.path, text.as_bytes())
}

/// Rename: the display name and the file name both change (numbered if the new name is taken).
pub fn rename(root: &Path, item: &Item, new_name: &str) -> Result<Item, String> {
    let new_name = new_name.trim();
    if new_name.is_empty() {
        return Err("a Gallery item needs a name".into());
    }
    if new_name == item.name {
        return Ok(item.clone());
    }
    let ext = item.file.as_ref().and_then(|f| f.extension()).and_then(|e| e.to_str()).unwrap_or("json").to_string();
    let same_slug = slug(new_name) == item.slug;
    let (name, stem) =
        if same_slug { (new_name.to_string(), item.slug.clone()) } else { free_name(root, item.kind, new_name, &ext) };
    let dir = dir_of(root, item.kind);
    let mut out = item.clone();
    out.name = name.clone();
    if item.kind == Kind::Sound {
        let old = item.file.clone().ok_or("no file")?;
        let new = dir.join(format!("{stem}.{ext}"));
        if new != old {
            std::fs::rename(&old, &new).map_err(|e| e.to_string())?;
            let _ = std::fs::remove_file(&item.path);
        }
        out.slug = stem.clone();
        out.file = Some(new.clone());
        out.path = new.with_extension("json");
    } else {
        out.slug = stem.clone();
        out.path = dir.join(format!("{stem}.json"));
    }
    rewrite(&out, &name, &out.tags, &out.data)?;
    if item.kind != Kind::Sound && out.path != item.path {
        let _ = std::fs::remove_file(&item.path);
    }
    let _ = write_index(root);
    Ok(out)
}

pub fn set_tags(root: &Path, item: &Item, tags: &[String]) -> Result<Item, String> {
    let mut out = item.clone();
    out.tags = tags.to_vec();
    rewrite(&out, &out.name, tags, &out.data)?;
    let _ = write_index(root);
    Ok(out)
}

/// "Replace with current": new payload, same name / tags / file.
pub fn replace_data(root: &Path, item: &Item, data: Value) -> Result<Item, String> {
    if item.kind == Kind::Sound {
        return Err("a sound has no payload to replace".into());
    }
    let mut out = item.clone();
    out.data = data;
    rewrite(&out, &out.name, &out.tags, &out.data)?;
    let _ = write_index(root);
    Ok(out)
}

pub fn duplicate(root: &Path, item: &Item) -> Result<Item, String> {
    let name = format!("{} copy", item.name);
    if item.kind == Kind::Sound {
        let src = item.file.clone().ok_or("no file")?;
        let mut copy = import_file(root, &src, &item.tags, item.data.get("duration").and_then(Value::as_f64))?;
        // import names a copy after its file; give it the wanted name
        copy = rename(root, &copy, &name)?;
        return Ok(copy);
    }
    save(root, item.kind, &name, &item.tags, item.data.clone(), Collide::Number)
}

/// Delete = move to `.trash\<kind>\` (a file of that name there is numbered), so nothing is lost.
pub fn delete(root: &Path, item: &Item) -> Result<(), String> {
    let trash = root.join(".trash").join(item.kind.dir());
    std::fs::create_dir_all(&trash).map_err(|e| e.to_string())?;
    let stamp = now();
    for p in [item.file.as_ref(), Some(&item.path)].into_iter().flatten() {
        if p.exists() {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            std::fs::rename(p, trash.join(format!("{stamp}-{name}"))).map_err(|e| e.to_string())?;
        }
    }
    let _ = write_index(root);
    Ok(())
}

// ------------------------------------------------------------------------------------------ index

/// Every item of every kind, as the `index.json` entries.
pub fn scan_index(root: &Path) -> Value {
    let mut items = Vec::new();
    for kind in Kind::ALL {
        for i in list(root, kind) {
            let file = i.file.as_ref().unwrap_or(&i.path);
            let rel = file.strip_prefix(root).map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_default();
            items.push(json!({"kind": kind.name(), "name": i.name, "tags": i.tags, "file": rel}));
        }
    }
    json!({"format": FORMAT, "version": VERSION, "items": items})
}

pub fn write_index(root: &Path) -> Result<(), String> {
    if !root.exists() {
        return Ok(());
    }
    write_atomic(
        &root.join("index.json"),
        serde_json::to_string_pretty(&scan_index(root)).map_err(|e| e.to_string())?.as_bytes(),
    )
}

/// The index, rebuilt from the files (and written back) when it is missing or unreadable.
pub fn ensure_index(root: &Path) -> Value {
    let ok = std::fs::read_to_string(root.join("index.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .filter(|v| v.get("format").and_then(Value::as_str) == Some(FORMAT) && v.get("items").is_some());
    match ok {
        Some(v) => v,
        None => {
            let _ = write_index(root);
            scan_index(root)
        }
    }
}

// ------------------------------------------------------------------------------------------ import

/// What an import did.
#[derive(Default, Debug)]
pub struct Imported {
    pub added: Vec<(Kind, String)>,
    pub skipped: Vec<(PathBuf, String)>,
}

/// "Import to Gallery...": item files, audio files, folders (searched through) and `.zip`s. A name that is
/// already taken is numbered, never overwritten.
pub fn import(root: &Path, paths: &[PathBuf]) -> Imported {
    let mut out = Imported::default();
    for p in paths {
        import_one(root, p, &mut out, 0);
    }
    out
}

fn import_one(root: &Path, p: &Path, out: &mut Imported, depth: usize) {
    if depth > 8 {
        return;
    }
    if p.is_dir() {
        let Ok(rd) = std::fs::read_dir(p) else { return };
        let mut es: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        es.sort();
        // (a sound's sidecar rides with its audio file; the trash and the index are not items)
        for e in es {
            let n = e.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if n.starts_with('.') || n == "index.json" {
                continue;
            }
            import_one(root, &e, out, depth + 1);
        }
        return;
    }
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "zip" => match unzip(p) {
            Ok(dir) => {
                import_one(root, &dir, out, depth + 1);
                let _ = std::fs::remove_dir_all(&dir);
            }
            Err(e) => out.skipped.push((p.to_path_buf(), e)),
        },
        "json" => {
            let Ok(text) = std::fs::read_to_string(p) else { return };
            let Ok(v) = serde_json::from_str::<Value>(&text) else {
                out.skipped.push((p.to_path_buf(), "not JSON".into()));
                return;
            };
            if v.get("format").and_then(Value::as_str) != Some(FORMAT) {
                out.skipped.push((p.to_path_buf(), "not a Gallery item".into()));
                return;
            }
            let Some(kind) = v.get("kind").and_then(Value::as_str).and_then(Kind::from_name) else {
                out.skipped.push((p.to_path_buf(), "unknown kind".into()));
                return;
            };
            if kind == Kind::Sound {
                return; // the sidecar of an audio file: taken with the file
            }
            let name = v.get("name").and_then(Value::as_str).unwrap_or("item").to_string();
            match save(root, kind, &name, &tags_of(&v), v.get("data").cloned().unwrap_or(Value::Null), Collide::Number)
            {
                Ok(i) => out.added.push((kind, i.name)),
                Err(e) => out.skipped.push((p.to_path_buf(), e)),
            }
        }
        _ if is_audio(p) => {
            let side: Value = std::fs::read_to_string(p.with_extension("json"))
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok())
                .unwrap_or(Value::Null);
            let dur = side.get("data").and_then(|d| d.get("duration")).and_then(Value::as_f64);
            let shared = side.get("name").and_then(Value::as_str);
            match import_file_as(root, p, shared, &tags_of(&side), dur) {
                Ok(i) => out.added.push((Kind::Sound, i.name)),
                Err(e) => out.skipped.push((p.to_path_buf(), e)),
            }
        }
        _ => out.skipped.push((p.to_path_buf(), "not a Gallery item".into())),
    }
}

/// Extract a `.zip` into a fresh temp folder with the PowerShell that ships with Windows (no new crate).
fn unzip(zip: &Path) -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join(format!("simple-editor-gallery-import-{}-{}", std::process::id(), now()));
    let _ = std::fs::remove_dir_all(&dir);
    let cmd = format!(
        "Expand-Archive -LiteralPath '{}' -DestinationPath '{}' -Force",
        zip.display().to_string().replace('\'', "''"),
        dir.display().to_string().replace('\'', "''")
    );
    let mut c = std::process::Command::new("powershell");
    c.args(["-NoProfile", "-NonInteractive", "-Command", &cmd]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let st = c.output().map_err(|e| format!("could not run PowerShell: {e}"))?;
    if st.status.success() && dir.exists() {
        Ok(dir)
    } else {
        Err(format!("could not open the zip: {}", String::from_utf8_lossy(&st.stderr).lines().next().unwrap_or("")))
    }
}

// ------------------------------------------------------------------------------------------ search

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("se-gallery-{tag}-{}-{}", std::process::id(), now()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn save_then_list_round_trips_with_tags() {
        let root = tmp("save");
        let tags = parse_tags(" Grade, film ,grade");
        assert_eq!(tags, ["grade", "film"]);
        let i = save(&root, Kind::Look, "Warm film", &tags, json!([{"kind": "Blur"}]), Collide::Number).unwrap();
        assert_eq!((i.name.as_str(), i.slug.as_str()), ("Warm film", "warm-film"));
        let got = list(&root, Kind::Look);
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].tags.clone(), got[0].data.clone()), (tags, json!([{"kind": "Blur"}])));
        assert!(list(&root, Kind::Effect).is_empty(), "other kinds are separate folders");
        assert!(root.join("looks/warm-film.json").is_file(), "plain file per item");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_taken_name_is_numbered_and_nothing_is_overwritten() {
        let root = tmp("collide");
        let a = save(&root, Kind::Look, "Glow", &[], json!(1), Collide::Number).unwrap();
        let b = save(&root, Kind::Look, "Glow", &[], json!(2), Collide::Number).unwrap();
        let c = save(&root, Kind::Look, "glow", &[], json!(3), Collide::Number).unwrap();
        assert_eq!((a.name.as_str(), b.name.as_str(), c.name.as_str()), ("Glow", "Glow (2)", "glow (3)"));
        assert_eq!(list(&root, Kind::Look).len(), 3);
        assert_eq!(find(&root, Kind::Look, "GLOW").unwrap().data, json!(1), "the first one is untouched");
        let r = save(&root, Kind::Look, "Glow", &[], json!(9), Collide::Replace).unwrap();
        assert_eq!(r.name, "Glow");
        assert_eq!(list(&root, Kind::Look).len(), 3, "Replace overwrites that one item only");
        assert_eq!(find(&root, Kind::Look, "Glow").unwrap().data, json!(9));
        assert!(save(&root, Kind::Look, "  ", &[], json!(0), Collide::Number).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn rename_tags_replace_duplicate_delete() {
        let root = tmp("edit");
        let i = save(&root, Kind::Animate, "Pop", &[], json!({"dur": 1}), Collide::Number).unwrap();
        let r = rename(&root, &i, "Big pop").unwrap();
        assert_eq!((r.name.as_str(), r.slug.as_str()), ("Big pop", "big-pop"));
        assert!(!root.join("animate/pop.json").exists() && root.join("animate/big-pop.json").is_file());
        let t = set_tags(&root, &r, &parse_tags("fast, ui")).unwrap();
        assert_eq!(find(&root, Kind::Animate, "Big pop").unwrap().tags, t.tags);
        let rp = replace_data(&root, &t, json!({"dur": 2})).unwrap();
        let again = find(&root, Kind::Animate, "Big pop").unwrap();
        assert_eq!(
            (again.data.clone(), again.tags.clone(), again.created),
            (json!({"dur": 2}), t.tags.clone(), rp.created)
        );
        let d = duplicate(&root, &again).unwrap();
        assert_eq!(d.name, "Big pop copy");
        assert_eq!(d.tags, again.tags);
        // renaming onto a taken name is numbered
        let n = rename(&root, &d, "Big pop").unwrap();
        assert_eq!(n.name, "Big pop (2)");
        delete(&root, &n).unwrap();
        assert_eq!(list(&root, Kind::Animate).len(), 1);
        assert!(std::fs::read_dir(root.join(".trash/animate")).unwrap().count() >= 1, "deleted = moved to .trash");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_index_is_rebuilt_from_the_files() {
        let root = tmp("index");
        save(&root, Kind::Look, "A", &[], json!([]), Collide::Number).unwrap();
        save(&root, Kind::Transition, "B", &parse_tags("x"), json!({}), Collide::Number).unwrap();
        let idx = ensure_index(&root);
        assert_eq!(idx["items"].as_array().unwrap().len(), 2);
        std::fs::remove_file(root.join("index.json")).unwrap();
        let rebuilt = ensure_index(&root);
        assert_eq!(rebuilt, idx, "same index from the files alone");
        assert!(root.join("index.json").is_file(), "and written back");
        std::fs::write(root.join("index.json"), "{ not json").unwrap();
        assert_eq!(ensure_index(&root)["items"].as_array().unwrap().len(), 2, "a broken index is rebuilt too");
        // an item that is not valid is ignored, not fatal
        std::fs::write(root.join("looks/junk.json"), "{}").unwrap();
        assert_eq!(list(&root, Kind::Look).len(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn import_takes_files_folders_and_numbers_collisions() {
        let (src, root) = (tmp("imp-src"), tmp("imp-root"));
        save(&src, Kind::Look, "Shared", &parse_tags("team"), json!([1]), Collide::Number).unwrap();
        save(&src, Kind::Caption, "Bold", &[], json!({"x": 1}), Collide::Number).unwrap();
        std::fs::create_dir_all(src.join("sounds")).unwrap();
        std::fs::write(src.join("sounds/boom.wav"), b"RIFF....WAVE").unwrap();
        let side = envelope(Kind::Sound, "Boom", &[], 1, &Value::Null);
        std::fs::write(src.join("sounds/boom.json"), side.to_string()).unwrap();
        std::fs::write(src.join("readme.txt"), "hi").unwrap();
        // someone already has a "Shared" look
        save(&root, Kind::Look, "Shared", &[], json!([0]), Collide::Number).unwrap();
        let r = import(&root, &[src.clone()]);
        let kinds: Vec<Kind> = r.added.iter().map(|(k, _)| *k).collect();
        assert!(kinds.contains(&Kind::Look) && kinds.contains(&Kind::Caption) && kinds.contains(&Kind::Sound), "{r:?}");
        assert_eq!(r.skipped.len(), 1, "the readme is skipped, listed");
        let looks = list(&root, Kind::Look);
        assert_eq!(looks.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["Shared", "Shared (2)"]);
        assert_eq!(find(&root, Kind::Look, "Shared").unwrap().data, json!([0]), "the receiver's own is untouched");
        assert_eq!(find(&root, Kind::Look, "Shared (2)").unwrap().tags, ["team"]);
        let s = list(&root, Kind::Sound);
        assert_eq!(s.len(), 1);
        assert!(s[0].file.as_ref().unwrap().is_file());
        // importing the same folder again numbers every item, adds no overwrite
        let r2 = import(&root, &[src.clone()]);
        assert_eq!(r2.added.len(), 3);
        assert_eq!(list(&root, Kind::Look).len(), 3);
        let names: Vec<String> = list(&root, Kind::Sound).into_iter().map(|i| i.name).collect();
        assert_eq!(
            names.iter().map(|n| n.to_lowercase()).collect::<Vec<_>>(),
            ["boom", "boom (2)"],
            "the second copy is (2), not (3)"
        );
        assert!(import(&root, &[src.join("looks/shared.json")]).added.len() == 1, "a single file works");
        assert_eq!(list(&root, Kind::Look).len(), 4);
        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn sounds_are_files_with_a_sidecar_and_search_by_tag() {
        let (src, root) = (tmp("snd-src"), tmp("snd-root"));
        std::fs::create_dir_all(&src).unwrap();
        let f = src.join("Door slam.wav");
        std::fs::write(&f, b"RIFF").unwrap();
        let i = import_file(&root, &f, &parse_tags("foley, door"), Some(1.5)).unwrap();
        assert_eq!(i.name, "Door slam");
        let j = import_file(&root, &f, &[], None).unwrap();
        assert_eq!(j.name, "Door slam (2)", "a taken file name is numbered");
        let l = list(&root, Kind::Sound);
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].data["duration"], 1.5);
        let r = rename(&root, &l[0], "Slam").unwrap();
        assert!(r.file.as_ref().unwrap().ends_with("slam.wav") && r.file.as_ref().unwrap().is_file());
        assert_eq!(find(&root, Kind::Sound, "Slam").unwrap().tags, ["foley", "door"]);
        delete(&root, &r).unwrap();
        assert_eq!(list(&root, Kind::Sound).len(), 1);
        assert!(import_file(&root, &src.join("x.txt"), &[], None).is_err());
        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn kind_names_round_trip() {
        for k in Kind::ALL {
            assert_eq!(Kind::from_name(k.name()), Some(k));
        }
        assert_eq!(Kind::from_name("nope"), None);
    }
}
