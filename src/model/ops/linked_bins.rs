//! Linked bins on the project: link / unlink / relink / rename, and the mapping between a bin path
//! ("Video Assets/SFX/Impacts") and the folder on disk it shows. No disk access here except where
//! a name says so - linking a folder must stay instant whatever is in it.

use crate::model::*;
use std::path::{Path, PathBuf};

/// Windows paths compare without case; a trailing separator is not part of the name.
fn same_dir(a: &str, b: &str) -> bool {
    a.trim_end_matches(['\\', '/']).eq_ignore_ascii_case(b.trim_end_matches(['\\', '/']))
}

/// `path` is `dir` or below it (on a separator boundary - "C:\media" does not own "C:\mediaXYZ").
fn under<'a>(path: &'a str, dir: &str) -> Option<&'a str> {
    let dir = dir.trim_end_matches(['\\', '/']);
    if path.len() < dir.len() || !path.is_char_boundary(dir.len()) || !path[..dir.len()].eq_ignore_ascii_case(dir) {
        return None;
    }
    let rest = &path[dir.len()..];
    if rest.is_empty() {
        return Some("");
    }
    rest.strip_prefix(['\\', '/'])
}

impl Project {
    /// Link `path` as a bin named `name` (the folder's own name when None); returns the bin's name. A
    /// folder that is already linked returns its existing bin. A name taken by another bin gets " 2", " 3"…
    /// Instant: no disk access at all - the bin's contents are listed when it is opened.
    pub fn link_bin(&mut self, path: &str, name: Option<&str>) -> String {
        let path = path.trim().trim_end_matches(['\\', '/']).to_string();
        // "D:" alone is the drive's current directory, "D:\" its root
        let path = if path.ends_with(':') { format!("{path}\\") } else { path };
        if let Some(b) = self.linked_bins.iter().find(|b| same_dir(&b.path, &path)) {
            return b.bin.clone();
        }
        let own = Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let own = if own.is_empty() { path.trim_end_matches(['\\', '/']).replace(':', "") } else { own };
        let base = name.unwrap_or(&own).replace(['/', '\\'], "-").trim().to_string();
        let base = if base.is_empty() { "Linked".to_string() } else { base };
        let names = self.folder_names();
        let mut bin = base.clone();
        let mut n = 2;
        while names.iter().any(|f| f == &bin) {
            bin = format!("{base} {n}");
            n += 1;
        }
        self.add_folder(&bin);
        self.linked_bins.push(LinkedBin { bin: bin.clone(), path });
        bin
    }

    /// Stop linking `bin`. Files already used from it stay assets, filed under the bin, which then
    /// remains as a plain bin holding just them; with none, the bin goes too. False if it isn't linked.
    pub fn unlink_bin(&mut self, bin: &str) -> bool {
        let before = self.linked_bins.len();
        self.linked_bins.retain(|b| b.bin != bin);
        if self.linked_bins.len() == before {
            return false;
        }
        let prefix = format!("{bin}/");
        if !self.assets.iter().any(|a| a.folder == bin || a.folder.starts_with(&prefix)) {
            self.remove_folder(bin);
        }
        true
    }

    /// Point `bin` at another folder (the drive letter changed, the folder moved). Assets used from the old
    /// folder follow when the same file exists under the new one. False if `bin` isn't linked.
    pub fn relink_bin(&mut self, bin: &str, path: &str) -> bool {
        let new = path.trim().trim_end_matches(['\\', '/']).to_string();
        let Some(b) = self.linked_bins.iter_mut().find(|b| b.bin == bin) else { return false };
        let old = std::mem::replace(&mut b.path, new.clone());
        for a in &mut self.assets {
            if let Some(rest) = under(&a.path, &old).filter(|r| !r.is_empty()) {
                let moved = Path::new(&new).join(rest);
                if moved.exists() {
                    a.path = moved.to_string_lossy().into_owned();
                }
            }
        }
        true
    }

    /// The linked bin `bin` itself, or the one whose disk tree `bin` is a sub-bin of.
    pub fn linked_bin_of(&self, bin: &str) -> Option<&LinkedBin> {
        self.linked_bins.iter().find(|b| bin == b.bin || bin.strip_prefix(&b.bin).is_some_and(|r| r.starts_with('/')))
    }

    /// The folder on disk that `bin` (a linked bin or a sub-bin of one) shows.
    pub fn linked_bin_dir(&self, bin: &str) -> Option<PathBuf> {
        let b = self.linked_bin_of(bin)?;
        let rest = bin[b.bin.len()..].trim_start_matches('/');
        let mut p = PathBuf::from(&b.path);
        for seg in rest.split('/').filter(|s| !s.is_empty()) {
            p.push(seg);
        }
        Some(p)
    }

    /// The bin a file on disk belongs to, through the deepest linked folder that holds it.
    pub fn linked_bin_for_file(&self, file: &str) -> Option<String> {
        let parent = Path::new(file).parent()?.to_string_lossy().into_owned();
        let (b, rest) = self
            .linked_bins
            .iter()
            .filter_map(|b| under(&parent, &b.path).map(|r| (b, r)))
            .max_by_key(|(b, _)| b.path.len())?;
        let rest = rest.replace('\\', "/");
        Some(if rest.is_empty() { b.bin.clone() } else { format!("{}/{rest}", b.bin) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_is_named_after_the_folder_and_unique() {
        let mut p = Project::new();
        assert_eq!(p.link_bin(r"C:\Media\Video Assets\", None), "Video Assets");
        assert_eq!(p.link_bin(r"c:\media\video assets", None), "Video Assets", "same folder, same bin");
        assert_eq!(p.link_bin(r"D:\Other\Video Assets", None), "Video Assets 2");
        assert_eq!(p.link_bin(r"E:\", None), "E");
        assert!(p.folders.iter().any(|f| f == "Video Assets 2"));
    }

    #[test]
    fn sub_bins_map_to_sub_folders_and_back() {
        let mut p = Project::new();
        p.link_bin(r"C:\Media\Assets", Some("A"));
        assert_eq!(p.linked_bin_dir("A/SFX/Hits"), Some(PathBuf::from(r"C:\Media\Assets\SFX\Hits")));
        assert_eq!(p.linked_bin_dir("AB"), None);
        assert_eq!(p.linked_bin_for_file(r"C:\Media\Assets\SFX\Hits\boom.wav").as_deref(), Some("A/SFX/Hits"));
        assert_eq!(p.linked_bin_for_file(r"C:\Media\Assets\x.mp4").as_deref(), Some("A"));
        assert_eq!(p.linked_bin_for_file(r"C:\Media\AssetsX\x.mp4"), None);
    }

    #[test]
    fn unlink_keeps_a_bin_that_holds_used_files() {
        let mut p = Project::new();
        p.link_bin(r"C:\M\Empty", None);
        p.link_bin(r"C:\M\Used", None);
        let id = p.add_asset(crate::engine::import::placeholder(r"C:\M\Used\a.mp4"));
        assert_eq!(p.asset(id).unwrap().folder, "Used", "filed under its linked bin when it became an asset");
        assert!(p.unlink_bin("Empty") && p.unlink_bin("Used") && !p.unlink_bin("Used"));
        assert!(!p.folders.iter().any(|f| f == "Empty"));
        assert!(p.folder_names().iter().any(|f| f == "Used"));
        assert!(p.linked_bins.is_empty());
    }
}
