//! ws:parity-io: per-file decode overrides - rotation (instead of the file's display matrix) and the YUV
//! range to read it as (for footage tagged wrong: a full-range phone or screen recording flagged limited
//! looks washed out, the reverse looks crushed).
//!
//! The project stores them (`Project::media_io`, by source path, so every clip and the Source monitor
//! agree); `sync` copies that map into a process global that the decoders read, the same way HDR sources
//! are found by path (`ffpipe::is_hdr`). A source with an override always decodes through the ffmpeg pipe
//! (Media Foundation rotates by the matrix and has no range switch) and never through its proxy, so the
//! preview, thumbnails and export all show the same corrected picture.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::RwLock;

/// How to read a source's YUV levels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Range {
    /// As tagged in the file (untagged = limited).
    #[default]
    Auto,
    /// 16-235: broadcast / most cameras.
    Limited,
    /// 0-255: many phones, screen recorders, JPEG-derived video.
    Full,
}

impl Range {
    pub fn parse(s: &str) -> Option<Range> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" | "" => Some(Range::Auto),
            "limited" | "tv" | "mpeg" => Some(Range::Limited),
            "full" | "pc" | "jpeg" => Some(Range::Full),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Range::Auto => "auto",
            Range::Limited => "limited",
            Range::Full => "full",
        }
    }
}

/// One source's overrides. The default (nothing overridden) is never stored.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MediaIo {
    /// Clockwise degrees to show the picture at, ignoring the file's own rotation: None = the file's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotate: Option<u16>,
    #[serde(skip_serializing_if = "is_auto")]
    pub range: Range,
}

fn is_auto(r: &Range) -> bool {
    *r == Range::Auto
}

impl MediaIo {
    pub fn is_default(&self) -> bool {
        *self == MediaIo::default()
    }
    /// 0 / 90 / 180 / 270 from any angle (rounded to the nearest quarter turn).
    pub fn quarter(deg: i32) -> u16 {
        ((((deg as f64) / 90.0).round() as i32).rem_euclid(4) * 90) as u16
    }
    /// ffmpeg input options (before `-i`): the file's rotation is ignored when ours replaces it.
    pub fn input_args(&self) -> Vec<String> {
        match self.rotate {
            Some(_) => vec!["-noautorotate".into()],
            None => Vec::new(),
        }
    }
    /// The `-vf` chain (None = nothing to do). Range first (on the YUV picture), then the turn; ends in RGB
    /// so the pipe's own `-pix_fmt rgba -s WxH` is a plain RGB scale.
    pub fn filters(&self) -> Option<String> {
        let mut f: Vec<&str> = Vec::new();
        match self.range {
            Range::Auto => {}
            Range::Limited => f.push("scale=in_range=tv:out_range=pc,format=rgba"),
            Range::Full => f.push("scale=in_range=pc:out_range=pc,format=rgba"),
        }
        match self.rotate {
            Some(90) => f.push("transpose=clock"),
            Some(180) => f.push("hflip,vflip"),
            Some(270) => f.push("transpose=cclock"),
            _ => {}
        }
        (!f.is_empty()).then(|| f.join(","))
    }
    /// The displayed size of a source whose decoded (coded) picture is `coded` and whose file says
    /// `file_rot` clockwise degrees.
    pub fn display_size(&self, coded: (u32, u32), file_rot: i32) -> (u32, u32) {
        let rot = self.rotate.map_or(file_rot, |r| r as i32);
        if MediaIo::quarter(rot) % 180 == 90 {
            (coded.1, coded.0)
        } else {
            coded
        }
    }
}

static MAP: RwLock<Option<HashMap<String, MediaIo>>> = RwLock::new(None);

/// Make the decoders follow `map` (the project's). Returns the paths whose override changed, so the caller
/// drops their decoded frames.
pub fn sync(map: &BTreeMap<String, MediaIo>) -> Vec<String> {
    let mut g = MAP.write().unwrap_or_else(|e| e.into_inner());
    let cur = g.get_or_insert_with(HashMap::new);
    let mut changed: Vec<String> = cur.keys().filter(|k| !map.contains_key(*k)).cloned().collect();
    changed.extend(map.iter().filter(|(k, v)| cur.get(*k) != Some(v)).map(|(k, _)| k.clone()));
    if !changed.is_empty() {
        *cur = map.iter().filter(|(_, v)| !v.is_default()).map(|(k, v)| (k.clone(), *v)).collect();
    }
    changed
}

/// The override for a source path, if any.
pub fn get(path: &str) -> Option<MediaIo> {
    MAP.read().unwrap_or_else(|e| e.into_inner()).as_ref()?.get(path).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_and_args() {
        assert_eq!(MediaIo::default().filters(), None);
        assert!(MediaIo::default().input_args().is_empty());
        let r = MediaIo { rotate: Some(90), range: Range::Full };
        assert_eq!(r.filters().unwrap(), "scale=in_range=pc:out_range=pc,format=rgba,transpose=clock");
        assert_eq!(r.input_args(), vec!["-noautorotate"]);
        // a forced 0 still ignores the file's matrix, with no filter of its own
        let upright = MediaIo { rotate: Some(0), ..Default::default() };
        assert_eq!((upright.filters(), upright.input_args().len()), (None, 1));
        assert_eq!(MediaIo { rotate: Some(270), ..Default::default() }.filters().unwrap(), "transpose=cclock");
    }

    #[test]
    fn quarter_turns_and_sizes() {
        assert_eq!((MediaIo::quarter(-90), MediaIo::quarter(450), MediaIo::quarter(179)), (270, 90, 180));
        let auto = MediaIo::default();
        assert_eq!(auto.display_size((1920, 1080), 90), (1080, 1920), "the file's turn");
        let flat = MediaIo { rotate: Some(0), ..Default::default() };
        assert_eq!(flat.display_size((1920, 1080), 90), (1920, 1080), "overridden upright");
        let turn = MediaIo { rotate: Some(270), ..Default::default() };
        assert_eq!(turn.display_size((1920, 1080), 0), (1080, 1920));
    }

    #[test]
    fn sync_reports_changes_and_drops_defaults() {
        let mut m = BTreeMap::new();
        m.insert("C:/sync_a.mp4".to_string(), MediaIo { range: Range::Full, ..Default::default() });
        assert_eq!(sync(&m), vec!["C:/sync_a.mp4".to_string()]);
        assert!(sync(&m).is_empty(), "same map, nothing changed");
        assert_eq!(get("C:/sync_a.mp4").unwrap().range, Range::Full);
        m.clear();
        assert_eq!(sync(&m), vec!["C:/sync_a.mp4".to_string()]);
        assert_eq!(get("C:/sync_a.mp4"), None);
    }

    #[test]
    fn serde_keeps_files_small() {
        let io = MediaIo { rotate: Some(180), range: Range::Auto };
        assert_eq!(serde_json::to_string(&io).unwrap(), r#"{"rotate":180}"#);
        let back: MediaIo = serde_json::from_str(r#"{"range":"full"}"#).unwrap();
        assert_eq!(back, MediaIo { rotate: None, range: Range::Full });
        assert_eq!(Range::parse("PC"), Some(Range::Full));
    }
}
