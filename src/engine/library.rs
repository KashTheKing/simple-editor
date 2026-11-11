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

#[cfg(test)]
mod tests {
    use super::*;

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
