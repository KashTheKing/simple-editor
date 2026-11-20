//! ws:parity-io: per-source decode overrides (rotation, YUV range) - `Project::media_io`.

use crate::media::io_override::MediaIo;
use crate::model::*;

impl Project {
    /// Set (or, with the default, clear) the override for the source at `path`. `coded` is the picture's
    /// stored size and `file_rot` the file's own clockwise rotation (from `engine::media_info`): with them
    /// every asset of that file takes its new displayed size, so a quarter turn swaps width and height.
    /// Returns how many assets use the file.
    pub fn set_media_io(&mut self, path: &str, io: MediaIo, coded: Option<(u32, u32)>, file_rot: i32) -> usize {
        let key = self.assets.iter().find(|a| a.path.eq_ignore_ascii_case(path)).map_or(path, |a| a.path.as_str()).to_string();
        if io.is_default() {
            self.media_io.remove(&key);
        } else {
            self.media_io.insert(key.clone(), io);
        }
        let mut n = 0;
        for a in self.assets.iter_mut().filter(|a| a.path.eq_ignore_ascii_case(&key)) {
            n += 1;
            if let Some(coded) = coded.filter(|c| c.0 > 0 && c.1 > 0) {
                (a.width, a.height) = io.display_size(coded, file_rot);
            }
        }
        n
    }
    /// The override in effect for `path` (default = none).
    pub fn media_io_of(&self, path: &str) -> MediaIo {
        self.media_io.iter().find(|(k, _)| k.eq_ignore_ascii_case(path)).map(|(_, v)| *v).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::io_override::Range;

    #[test]
    fn override_turns_the_asset_and_clears_back() {
        let a: Asset = serde_json::from_value(serde_json::json!({
            "id": 0, "path": "C:/Phone.mp4", "kind": "Video", "duration": 3.0, "width": 1080, "height": 1920, "fps": 30.0
        }))
        .unwrap();
        let mut p = Project::from_media(a);
        // the file is stored 1920x1080 and flagged 90 degrees: force it upright (landscape)
        let io = MediaIo { rotate: Some(0), range: Range::Full };
        assert_eq!(p.set_media_io("c:/phone.MP4", io, Some((1920, 1080)), 90), 1);
        assert_eq!((p.assets[0].width, p.assets[0].height), (1920, 1080));
        assert_eq!(p.media_io_of("C:/PHONE.mp4"), io);
        assert!(p.media_io.contains_key("C:/Phone.mp4"), "keyed by the asset's own spelling");
        let json = p.to_json();
        assert!(json.contains("\"media_io\""), "{json}");
        // back to the file's own rotation and range: nothing stored, portrait again
        p.set_media_io("C:/Phone.mp4", MediaIo::default(), Some((1920, 1080)), 90);
        assert!(p.media_io.is_empty());
        assert_eq!((p.assets[0].width, p.assets[0].height), (1080, 1920));
        assert!(!p.to_json().contains("media_io"), "an empty map is not written");
    }
}
