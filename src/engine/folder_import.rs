//! Import a folder of media as a library bin (`folder` on assets / `Project::folders`).
//!
//! Two halves, both free of the UI: `scan` walks the disk (runs on a worker - a media drive can be
//! slow) and `apply` folds the result into a `Project` (UI thread, one undo step around it). Sub-folders
//! become nested bins ("SFX/Impacts"), files sort naturally ("take2" before "take10"), hidden/system
//! entries and unsupported files are skipped and counted, and a file already in the library (same
//! path) is reused rather than added twice. The app side lives in `ui/app/folder_import.rs`.

use super::import::{detect_sequence, placeholder};
use crate::model::{Id, Project};
use serde_json::Value;
use std::cmp::Ordering;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Every extension the library imports (lower-case, no dot).
pub const MEDIA_EXTS: &[&str] = &[
    "mp4", "mov", "mkv", "webm", "avi", "m4v", "wmv", "ts", "m2ts", "mts", "flv", "3gp", "mpg", "mpeg", "gif", "mp3",
    "wav", "m4a", "aac", "flac", "ogg", "opus", "wma", "png", "jpg", "jpeg", "bmp", "webp", "tif", "tiff",
    // ---- ws:parity-formats ---- (.svg / Lottie are converted first, so only the Import dialog offers
    // them: `formats_ctl::CONVERTED_EXTS`; a folder or a linked bin never probes one as media)
    "avif",
];

/// Folders nested deeper than this below the picked one are skipped (and counted).
pub const MAX_DEPTH: usize = 8;
/// More files than this and the UI asks before importing.
pub const CONFIRM_ABOVE: usize = 500;

#[derive(Clone, Debug)]
pub struct Options {
    /// Walk sub-folders (as nested bins). Off = only the files directly in the folder.
    pub recursive: bool,
    /// Name of the top bin; the folder's own name when `None`.
    pub bin_name: Option<String>,
    /// Lower-case extensions without the dot; empty = every supported one.
    pub extensions: Vec<String>,
    /// Link the folder as a bin (`engine::linked_bins`) instead of importing every file: instant, the
    /// default. `recursive` / `extensions` only apply to a full import.
    pub linked: bool,
    /// A full import that replaces this linked bin ("Convert to regular bin"): its bins keep that name,
    /// unabridged, and the link is dropped in the same undo step.
    pub into_bin: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        Self { recursive: true, bin_name: None, extensions: Vec::new(), linked: true, into_bin: None }
    }
}

impl Options {
    /// The MCP arguments `recursive` / `bin_name` / `extensions` (".wav" and "WAV" both work).
    pub fn from_args(args: &Value) -> Self {
        let mut o = Self::default();
        if let Some(r) = args.get("recursive").and_then(|v| v.as_bool()) {
            o.recursive = r;
        }
        if let Some(l) = args.get("linked").and_then(|v| v.as_bool()) {
            o.linked = l;
        }
        o.bin_name = args.get("bin_name").and_then(|v| v.as_str()).map(str::to_string).filter(|s| !s.trim().is_empty());
        if let Some(a) = args.get("extensions").and_then(|v| v.as_array()) {
            o.extensions = a
                .iter()
                .filter_map(|v| v.as_str())
                .map(|e| e.trim().trim_start_matches(['*', '.']).to_ascii_lowercase())
                .filter(|e| !e.is_empty())
                .collect();
        }
        o
    }
}

/// One bin: its '/'-nested name and the files that go straight into it, in natural name order.
#[derive(Debug, Default, PartialEq)]
pub struct Bin {
    pub name: String,
    pub files: Vec<PathBuf>,
}

/// What `scan` found. `bins` are parents-first; a bin with no media anywhere below it is not listed.
#[derive(Debug, Default)]
pub struct Scan {
    pub bins: Vec<Bin>,
    /// Numbered still runs (bin, frames): each bakes into one video like a dropped sequence does.
    pub runs: Vec<(String, Vec<PathBuf>)>,
    /// Unsupported files, unreadable folders and folders below `MAX_DEPTH`. Hidden/system entries
    /// (`desktop.ini`, `.git`, `$RECYCLE.BIN`) are passed over without counting - they are not "media
    /// the user expected".
    pub skipped: usize,
    /// The linked bin this import converts (`Options::into_bin`).
    pub unlink: Option<String>,
}

impl Scan {
    /// Files that will import (a run's frames count: they are files the user pointed at).
    pub fn file_count(&self) -> usize {
        self.bins.iter().map(|b| b.files.len()).sum::<usize>() + self.runs.iter().map(|(_, f)| f.len()).sum::<usize>()
    }

    /// The top bin's name (what the toast quotes).
    pub fn top(&self) -> &str {
        self.bins.first().map_or_else(|| self.runs.first().map_or("", |r| r.0.as_str()), |b| b.name.as_str())
    }

    /// Import every run's frames as plain stills instead (no ffmpeg to bake them with).
    pub fn fold_runs(&mut self) {
        for (bin, frames) in std::mem::take(&mut self.runs) {
            if let Some(b) = self.bins.iter_mut().find(|b| b.name == bin) {
                b.files.extend(frames);
                b.files.sort_by(|a, b| natural_cmp(&name_of(a), &name_of(b)));
            }
        }
    }
}

fn name_of(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Case-insensitive, digit runs compared as numbers: "take2" < "take10", "a01" ~ "a1" (then plain order).
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(p), Some(q)) if p.is_ascii_digit() && q.is_ascii_digit() => {
                let (n, m) = (digits(&mut x), digits(&mut y));
                let (n, m) = (n.trim_start_matches('0'), m.trim_start_matches('0'));
                // a longer run of significant digits is the bigger number
                let o = n.len().cmp(&m.len()).then_with(|| n.cmp(m));
                if o != Ordering::Equal {
                    return o;
                }
            }
            (Some(p), Some(q)) => {
                let o = p.to_lowercase().cmp(q.to_lowercase());
                if o != Ordering::Equal {
                    return o;
                }
                x.next();
                y.next();
            }
        }
    }
}

fn digits(it: &mut std::iter::Peekable<std::str::Chars>) -> String {
    let mut s = String::new();
    while let Some(c) = it.next_if(char::is_ascii_digit) {
        s.push(c);
    }
    s
}

/// Hidden or system (Windows attributes), or a dot-name: skipped without being counted.
fn hidden(e: &std::fs::DirEntry, name: &str) -> bool {
    use std::os::windows::fs::MetadataExt;
    const HIDDEN_OR_SYSTEM: u32 = 0x2 | 0x4;
    name.starts_with('.') || e.metadata().is_ok_and(|m| m.file_attributes() & HIDDEN_OR_SYSTEM != 0)
}

/// A bin name from a user/folder name: no path separators, no edge whitespace.
fn clean(name: &str) -> String {
    name.replace(['/', '\\'], "-").trim().to_string()
}

/// Walk `root`. `exts` = every extension the app can import (lower-case, no dot).
pub fn scan(root: &Path, o: &Options, exts: &[&str]) -> Scan {
    if let Some(bin) = &o.into_bin {
        let mut s = Scan { unlink: Some(bin.clone()), ..Default::default() };
        walk(root, bin, 0, o, exts, &mut s);
        return s;
    }
    let top = o.bin_name.as_deref().map(clean).filter(|n| !n.is_empty()).unwrap_or_else(|| {
        let n = root.file_name().map(|n| clean(&n.to_string_lossy())).unwrap_or_default();
        // "D:\" has no name: the drive stays recognisable
        if n.is_empty() { root.to_string_lossy().trim_end_matches(['\\', '/']).replace(':', "") } else { n }
    });
    let top = if top.is_empty() { "Imported".to_string() } else { top };
    let mut s = Scan::default();
    walk(root, &top, 0, o, exts, &mut s);
    s
}

/// True when the bin (or anything below it) kept something.
fn walk(dir: &Path, bin: &str, depth: usize, o: &Options, exts: &[&str], out: &mut Scan) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else {
        out.skipped += 1;
        return false;
    };
    let (mut files, mut dirs) = (Vec::new(), Vec::new());
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if hidden(&e, &name) {
            continue;
        }
        let path = e.path();
        // a link is followed only when it points at a file: a folder link could loop back up the tree
        let (is_dir, is_file) = match e.file_type() {
            Ok(t) if t.is_symlink() => (false, std::fs::metadata(&path).is_ok_and(|m| m.is_file())),
            Ok(t) => (t.is_dir(), t.is_file()),
            Err(_) => (false, false),
        };
        if is_dir {
            if o.recursive {
                dirs.push((name, path));
            }
        } else if is_file {
            let ext = path.extension().map(|x| x.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
            if exts.contains(&ext.as_str()) && (o.extensions.is_empty() || o.extensions.contains(&ext)) {
                files.push(path);
            } else {
                out.skipped += 1;
            }
        }
    }
    files.sort_by(|a, b| natural_cmp(&name_of(a), &name_of(b)));
    dirs.sort_by(|a, b| natural_cmp(&a.0, &b.0));

    // numbered still runs bake into one video each, like a dropped sequence; their frames leave `files`
    let mut runs = Vec::new();
    let mut i = 0;
    while i < files.len() {
        match detect_sequence(&files[i]) {
            Some(seq) => {
                let frames: HashSet<&PathBuf> = seq.frames.iter().collect();
                files.retain(|f| !frames.contains(f));
                runs.push((bin.to_string(), seq.frames.clone())); // numeric order, as detected
            }
            None => i += 1,
        }
    }

    let idx = out.bins.len();
    out.bins.push(Bin { name: bin.to_string(), files });
    let runs_before = out.runs.len();
    out.runs.extend(runs);
    let mut kept = !out.bins[idx].files.is_empty() || out.runs.len() > runs_before;
    for (name, path) in dirs {
        if depth + 1 > MAX_DEPTH {
            out.skipped += 1;
            continue;
        }
        kept |= walk(&path, &format!("{bin}/{}", clean(&name)), depth + 1, o, exts, out);
    }
    if !kept {
        out.bins.truncate(idx);
    }
    kept
}

/// What `apply` did to the project.
#[derive(Debug, Default)]
pub struct Applied {
    /// New placeholder assets (id, path): the caller starts their probes.
    pub fresh: Vec<(Id, String)>,
    /// Files whose path was already an asset: left exactly where they were.
    pub reused: usize,
}

/// Put every bin and file of `scan` into `project`: bins via `add_folder`, each new file as a
/// placeholder asset filed under its bin. Asset paths are compared the way `asset_by_path` does
/// (ASCII case-insensitive), through one set - a 5 000-file folder must not be a 5 000 x 5 000 scan.
pub fn apply(project: &mut Project, scan: &Scan) -> Applied {
    let mut known: HashSet<String> = project.assets.iter().map(|a| a.path.to_ascii_lowercase()).collect();
    let mut done = Applied::default();
    for bin in &scan.bins {
        project.add_folder(&bin.name);
        for f in &bin.files {
            let path = f.to_string_lossy().into_owned();
            if !known.insert(path.to_ascii_lowercase()) {
                done.reused += 1;
                continue;
            }
            let mut a = placeholder(&path);
            a.folder = bin.name.clone();
            a.id = project.new_id();
            done.fresh.push((a.id, path));
            project.assets.push(a);
        }
    }
    done
}

#[cfg(test)]
mod tests;
