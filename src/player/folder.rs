//! Previous / next, shuffle and repeat over the media files in the playing file's folder - Media
//! Player's queue for a file opened from Explorer. The folder is read once, on the first Previous /
//! Next / end of file, never at startup.

use crate::engine::folder_import::natural_cmp;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Repeat {
    #[default]
    Off,
    /// Wrap from the last file to the first.
    All,
    /// Play the same file again.
    One,
}

impl Repeat {
    /// Media Player's button cycles Off -> All -> One -> Off.
    pub fn next(self) -> Repeat {
        match self {
            Repeat::Off => Repeat::All,
            Repeat::All => Repeat::One,
            Repeat::One => Repeat::Off,
        }
    }
}

/// A file the player opens: a video the decoders take, or a sound file.
pub fn is_playable(p: &Path) -> bool {
    let ext = crate::media::ext(&p.to_string_lossy());
    crate::contextmenu::VIDEO_EXTS.contains(&ext.as_str())
        || crate::engine::export::AUDIO_EXTS.contains(&ext.as_str())
        || matches!(ext.as_str(), "wma" | "aiff" | "aif" | "m4b" | "mka")
}

#[derive(Debug, Default)]
pub struct Playlist {
    /// The folder's playable files in name order (natural: "2" before "10").
    files: Vec<PathBuf>,
    /// The play order: indices into `files` (shuffled, or 0..n).
    order: Vec<usize>,
    /// Position in `order` of the playing file.
    pos: usize,
    pub shuffle: bool,
    pub repeat: Repeat,
    scanned: bool,
    seed: u64,
}

impl Playlist {
    pub fn new(shuffle: bool, repeat: Repeat) -> Playlist {
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64);
        Playlist { shuffle, repeat, seed: seed | 1, ..Default::default() }
    }

    /// Read `current`'s folder (once) and find `current` in it.
    pub fn scan(&mut self, current: &Path) {
        if self.scanned && self.files.get(self.order.get(self.pos).copied().unwrap_or(usize::MAX)).is_some_and(|f| f == current) {
            return;
        }
        let dir = current.parent().unwrap_or(Path::new("."));
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
            .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_file() && is_playable(p)).collect())
            .unwrap_or_default();
        if !files.iter().any(|f| f == current) {
            files.push(current.to_path_buf());
        }
        files.sort_by(|a, b| natural_cmp(&name(a), &name(b)));
        self.files = files;
        self.scanned = true;
        self.reorder(current);
    }

    /// Turn shuffle on or off, keeping the playing file where it is.
    pub fn set_shuffle(&mut self, on: bool, current: &Path) {
        self.shuffle = on;
        if self.scanned {
            self.reorder(current);
        }
    }

    fn reorder(&mut self, current: &Path) {
        let cur = self.files.iter().position(|f| f == current).unwrap_or(0);
        self.order = (0..self.files.len()).collect();
        if self.shuffle {
            // the playing file first, the rest in a random order (xorshift, Fisher-Yates)
            self.order.retain(|&i| i != cur);
            for i in (1..self.order.len()).rev() {
                self.seed ^= self.seed << 13;
                self.seed ^= self.seed >> 7;
                self.seed ^= self.seed << 17;
                self.order.swap(i, (self.seed % (i as u64 + 1)) as usize);
            }
            self.order.insert(0, cur);
        }
        self.pos = self.order.iter().position(|&i| i == cur).unwrap_or(0);
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// The file after the playing one (wrapping only with Repeat All / shuffle); None at the end.
    pub fn next(&mut self, current: &Path) -> Option<PathBuf> {
        self.step(current, 1, matches!(self.repeat, Repeat::All))
    }

    /// The file before the playing one (Previous always wraps, like Media Player's queue).
    pub fn prev(&mut self, current: &Path) -> Option<PathBuf> {
        self.step(current, -1, true)
    }

    fn step(&mut self, current: &Path, by: isize, wrap: bool) -> Option<PathBuf> {
        self.scan(current);
        let n = self.order.len() as isize;
        if n < 2 {
            return None;
        }
        let mut p = self.pos as isize + by;
        if p < 0 || p >= n {
            if !wrap {
                return None;
            }
            p = p.rem_euclid(n);
        }
        self.pos = p as usize;
        Some(self.files[self.order[self.pos]].clone())
    }

    /// What to do when the playing file ends: Some(path) to open next (the same path = play it again),
    /// None = stop on the last frame.
    pub fn after_end(&mut self, current: &Path) -> Option<PathBuf> {
        match self.repeat {
            Repeat::One => Some(current.to_path_buf()),
            Repeat::All => self.next(current).or_else(|| Some(current.to_path_buf())),
            Repeat::Off if self.shuffle => self.next(current),
            Repeat::Off => None,
        }
    }
}

fn name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(names: &[&str]) -> (PathBuf, Vec<PathBuf>) {
        let dir = std::env::temp_dir().join(format!("se-player-folder-{}-{}", std::process::id(), names.len()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let paths = names.iter().map(|n| dir.join(n)).collect::<Vec<_>>();
        for p in &paths {
            std::fs::write(p, b"x").unwrap();
        }
        (dir, paths)
    }

    #[test]
    fn folder_prev_next_in_natural_order_skipping_other_files() {
        let (dir, p) = folder(&["clip10.mp4", "clip2.mkv", "notes.txt", "song.mp3", "clip1.mp4"]);
        let at = |n: &str| dir.join(n);
        let mut pl = Playlist::new(false, Repeat::Off);
        assert_eq!(pl.next(&at("clip1.mp4")), Some(at("clip2.mkv")));
        assert_eq!(pl.next(&at("clip2.mkv")), Some(at("clip10.mp4")));
        assert_eq!(pl.next(&at("clip10.mp4")), Some(at("song.mp3")), "sound files are in the queue too");
        assert_eq!(pl.len(), 4, "notes.txt is not playable");
        assert_eq!(pl.next(&at("song.mp3")), None, "Repeat off: Next stops at the end");
        assert_eq!(pl.prev(&at("song.mp3")), Some(at("clip10.mp4")));
        assert_eq!(pl.prev(&at("clip1.mp4")), Some(at("song.mp3")), "Previous wraps");
        pl.repeat = Repeat::All;
        assert_eq!(pl.next(&at("song.mp3")), Some(at("clip1.mp4")), "Repeat all wraps");
        assert_eq!(pl.after_end(&at("clip1.mp4")), Some(at("clip2.mkv")));
        pl.repeat = Repeat::One;
        assert_eq!(pl.after_end(&at("clip2.mkv")), Some(at("clip2.mkv")));
        pl.repeat = Repeat::Off;
        assert_eq!(pl.after_end(&at("clip2.mkv")), None);
        drop(p);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn shuffle_visits_every_file_once_starting_from_the_playing_one() {
        let names: Vec<String> = (1..=9).map(|i| format!("v{i}.mp4")).collect();
        let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        let (dir, _) = folder(&refs);
        let mut pl = Playlist::new(true, Repeat::Off);
        let mut cur = dir.join("v5.mp4");
        let mut seen = vec![cur.clone()];
        while let Some(n) = pl.next(&cur) {
            assert!(!seen.contains(&n), "{n:?} twice");
            seen.push(n.clone());
            cur = n;
        }
        assert_eq!(seen.len(), 9);
        assert_eq!(Repeat::Off.next().next().next(), Repeat::Off);
        let _ = std::fs::remove_dir_all(dir);
    }
}
