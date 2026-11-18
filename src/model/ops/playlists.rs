//! ---- ws:music-burn ----
//! Playlist edits (`playlist.*` tools, the Playlists pane) and the playlist -> timeline mix.

use crate::model::*;
use std::path::Path;

impl Project {
    pub fn playlist(&self, id: Id) -> Option<&Playlist> {
        self.playlists.iter().find(|p| p.id == id)
    }

    fn playlist_mut(&mut self, id: Id) -> Option<&mut Playlist> {
        self.playlists.iter_mut().find(|p| p.id == id)
    }

    /// A new, empty playlist ("Playlist N" when `name` is blank). Returns its id.
    pub fn add_playlist(&mut self, name: &str) -> Id {
        let id = self.new_id();
        let name = if name.trim().is_empty() { format!("Playlist {}", self.playlists.len() + 1) } else { name.trim().to_string() };
        self.playlists.push(Playlist { id, name, ..Default::default() });
        id
    }

    pub fn rename_playlist(&mut self, id: Id, name: &str) -> bool {
        match self.playlist_mut(id) {
            Some(p) if !name.trim().is_empty() => {
                p.name = name.trim().to_string();
                true
            }
            _ => false,
        }
    }

    pub fn delete_playlist(&mut self, id: Id) -> bool {
        let n = self.playlists.len();
        self.playlists.retain(|p| p.id != id);
        self.playlists.len() != n
    }

    /// Add library assets (sound or video only; others are skipped) at `at` (None = the end). Returns
    /// how many were added.
    pub fn playlist_add(&mut self, id: Id, assets: &[Id], at: Option<usize>) -> usize {
        let ok: Vec<Id> = assets
            .iter()
            .copied()
            .filter(|a| self.asset(*a).is_some_and(|x| matches!(x.kind, ClipKind::Audio | ClipKind::Video) && x.html.is_none()))
            .collect();
        let Some(p) = self.playlist_mut(id) else { return 0 };
        let at = at.unwrap_or(p.items.len()).min(p.items.len());
        let n = ok.len();
        p.items.splice(at..at, ok);
        n
    }

    /// Remove the items at `indices` (any order; out-of-range ignored). Returns how many went.
    pub fn playlist_remove(&mut self, id: Id, indices: &[usize]) -> usize {
        let Some(p) = self.playlist_mut(id) else { return 0 };
        let n = p.items.len();
        let mut i = 0;
        p.items.retain(|_| {
            let keep = !indices.contains(&i);
            i += 1;
            keep
        });
        n - p.items.len()
    }

    /// Move the item at `from` so it ends up at index `to` (clamped).
    pub fn playlist_move(&mut self, id: Id, from: usize, to: usize) -> bool {
        let Some(p) = self.playlist_mut(id) else { return false };
        if from >= p.items.len() {
            return false;
        }
        let it = p.items.remove(from);
        let to = to.min(p.items.len());
        p.items.insert(to, it);
        from != to
    }

    /// Gap and crossfade (seconds, clamped to 0..30).
    pub fn playlist_set_spacing(&mut self, id: Id, gap: Option<f64>, crossfade: Option<f64>) -> bool {
        let Some(p) = self.playlist_mut(id) else { return false };
        if let Some(g) = gap {
            p.gap = g.clamp(0.0, 30.0);
        }
        if let Some(c) = crossfade {
            p.crossfade = c.clamp(0.0, 30.0);
        }
        true
    }

    /// Seconds the playlist plays for (items back to back, gaps added, crossfades taken off).
    pub fn playlist_duration(&self, id: Id) -> f64 {
        let Some(p) = self.playlist(id) else { return 0.0 };
        let lens: Vec<f64> = p.items.iter().filter_map(|a| self.asset(*a)).map(|a| a.duration).collect();
        spaced_length(&lens, p.gap, p.crossfade)
    }

    /// The playlist's items laid on the timeline from `at`: back to back with its gap, or overlapping by
    /// its crossfade (each song fades out under the next fading in; overlapping songs land on the next
    /// free track). Returns every clip made.
    pub fn playlist_to_timeline(&mut self, id: Id, at: f64) -> Vec<Id> {
        let Some(p) = self.playlist(id).cloned() else { return Vec::new() };
        let mut t = at.max(0.0);
        let mut all = Vec::new();
        let mut prev_audio: Vec<Id> = Vec::new();
        for aid in &p.items {
            let Some(len) = self.asset(*aid).map(|a| a.duration) else { continue };
            let xf = p.crossfade.min(len / 2.0);
            let ids = self.insert_asset_clips(*aid, t, None);
            if ids.is_empty() {
                continue;
            }
            if xf > 0.0 {
                for &c in &prev_audio {
                    if let Some(c) = self.clip_mut(c) {
                        c.fade_out = xf;
                    }
                }
                if !prev_audio.is_empty() {
                    for &c in &ids {
                        if let Some(c) = self.clip_mut(c) {
                            if c.kind == ClipKind::Audio {
                                c.fade_in = xf;
                            } else {
                                // the picture dissolves in over the same span
                                c.opacity = Animated::new(1.0);
                                c.opacity.key_at(0.0, 0.0);
                                c.opacity.key_at(xf, 1.0);
                            }
                        }
                    }
                }
            }
            prev_audio = ids.iter().copied().filter(|&c| self.clip(c).is_some_and(|c| c.kind == ClipKind::Audio)).collect();
            let end = ids.iter().filter_map(|&c| self.clip(c)).map(|c| c.end()).fold(t, f64::max);
            t = if p.crossfade > 0.0 { end - xf } else { end + p.gap };
            all.extend(ids);
        }
        self.tidy();
        all
    }

    /// The playlist as an `.m3u8` (UTF-8): `#EXTINF` lines with the length and "Artist - Title" (from
    /// `titles`, else the library name), paths relative to `dir` when they are under it, else absolute.
    pub fn playlist_m3u8(&self, id: Id, dir: Option<&Path>, titles: &dyn Fn(Id) -> Option<String>) -> Option<String> {
        let p = self.playlist(id)?;
        let mut s = String::from("#EXTM3U\n");
        s += &format!("#PLAYLIST:{}\n", p.name);
        for aid in &p.items {
            let Some(a) = self.asset(*aid) else { continue };
            // no tag title: the user's rename, else the file name without its extension
            let title = titles(*aid).unwrap_or_else(|| {
                self.asset_names.get(&a.id).cloned().unwrap_or_else(|| {
                    Path::new(&a.path).file_stem().map_or_else(|| a.name(), |s| s.to_string_lossy().into_owned())
                })
            });
            s += &format!("#EXTINF:{},{}\n", a.duration.round() as i64, title.replace(['\r', '\n'], " "));
            let path = Path::new(&a.path);
            let rel = dir.and_then(|d| path.strip_prefix(d).ok()).map(|r| r.to_string_lossy().replace('\\', "/"));
            s += &rel.unwrap_or_else(|| a.path.clone());
            s.push('\n');
        }
        Some(s)
    }

    /// The library's name for an asset (the user's rename, else the file name).
    pub fn asset_display_name(&self, a: &Asset) -> String {
        self.asset_names.get(&a.id).cloned().unwrap_or_else(|| a.name())
    }
}

/// Length of items played back to back with `gap` between them, or overlapping by `crossfade`.
pub fn spaced_length(lens: &[f64], gap: f64, crossfade: f64) -> f64 {
    let mut t = 0.0;
    let mut end: f64 = 0.0;
    for &l in lens {
        let xf = crossfade.min(l / 2.0);
        end = t + l;
        t = if crossfade > 0.0 { end - xf } else { end + gap };
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(p: &mut Project, path: &str, secs: f64) -> Id {
        let a = Asset {
            id: 0,
            path: path.into(),
            kind: ClipKind::Audio,
            duration: secs,
            width: 0,
            height: 0,
            fps: 0.0,
            audio_streams: vec![AudioStreamInfo { index: 0, channels: 2, sample_rate: 48000, ..Default::default() }],
            codec: String::new(),
            folder: String::new(),
            tags: Vec::new(),
            label: 0,
            description: String::new(),
            rel_path: None,
            parent: None,
            range: None,
            effects: Vec::new(),
            html: None,
            proxy: None,
        };
        p.add_asset(a)
    }

    #[test]
    fn create_rename_delete_and_edit_items() {
        let mut p = Project::new();
        let a = song(&mut p, "C:/m/a.mp3", 10.0);
        let b = song(&mut p, "C:/m/b.mp3", 20.0);
        let c = song(&mut p, "C:/m/c.mp3", 30.0);
        let pl = p.add_playlist("");
        assert_eq!(p.playlist(pl).unwrap().name, "Playlist 1");
        assert!(p.rename_playlist(pl, " Road trip "));
        assert!(!p.rename_playlist(pl, "  "), "a blank name is refused");
        assert_eq!(p.playlist(pl).unwrap().name, "Road trip");
        assert_eq!(p.playlist_add(pl, &[a, b, 999], None), 2, "unknown ids are skipped");
        assert_eq!(p.playlist_add(pl, &[c], Some(0)), 1);
        assert_eq!(p.playlist(pl).unwrap().items, vec![c, a, b]);
        assert!(p.playlist_move(pl, 0, 2));
        assert_eq!(p.playlist(pl).unwrap().items, vec![a, b, c]);
        assert_eq!(p.playlist_duration(pl), 60.0);
        p.playlist_set_spacing(pl, Some(2.0), None);
        assert_eq!(p.playlist_duration(pl), 64.0);
        p.playlist_set_spacing(pl, None, Some(4.0));
        assert_eq!(p.playlist_duration(pl), 52.0, "crossfades overlap, gaps ignored");
        assert_eq!(p.playlist_remove(pl, &[1, 7]), 1);
        assert_eq!(p.playlist(pl).unwrap().items, vec![a, c]);
        // saved with the project
        let back = Project::from_json(&p.to_json()).unwrap();
        assert_eq!(back.playlists, p.playlists);
        assert!(p.delete_playlist(pl));
        assert!(p.playlists.is_empty());
        assert!(!Project::new().to_json().contains("playlists"), "no key in projects without playlists");
    }

    #[test]
    fn m3u8_export_has_lengths_titles_and_relative_paths() {
        let mut p = Project::new();
        let a = song(&mut p, "C:/Music/x/a.mp3", 61.4);
        let b = song(&mut p, "D:/Other/b.flac", 5.0);
        let pl = p.add_playlist("Mix");
        p.playlist_add(pl, &[a, b], None);
        let titles = |id: Id| (id == a).then(|| "Artist - Song A".to_string());
        let m = p.playlist_m3u8(pl, Some(Path::new("C:/Music")), &titles).unwrap();
        assert_eq!(m, "#EXTM3U\n#PLAYLIST:Mix\n#EXTINF:61,Artist - Song A\nx/a.mp3\n#EXTINF:5,b\nD:/Other/b.flac\n");
    }

    #[test]
    fn to_timeline_back_to_back_with_gaps_and_crossfades() {
        let mut p = Project::new();
        let a = song(&mut p, "C:/m/a.mp3", 10.0);
        let b = song(&mut p, "C:/m/b.mp3", 20.0);
        let pl = p.add_playlist("x");
        p.playlist_add(pl, &[a, b], None);
        p.playlist_set_spacing(pl, Some(1.5), None);
        let ids = p.playlist_to_timeline(pl, 2.0);
        let starts: Vec<f64> = ids.iter().map(|&c| p.clip(c).unwrap().start).collect();
        assert_eq!(starts, vec![2.0, 13.5]);
        // crossfade: overlap, fades, the second song on another track
        let mut q = Project::new();
        let a = song(&mut q, "C:/m/a.mp3", 10.0);
        let b = song(&mut q, "C:/m/b.mp3", 20.0);
        let pl = q.add_playlist("x");
        q.playlist_add(pl, &[a, b], None);
        q.playlist_set_spacing(pl, None, Some(3.0));
        let ids = q.playlist_to_timeline(pl, 0.0);
        let (c1, c2) = (q.clip(ids[0]).unwrap().clone(), q.clip(ids[1]).unwrap().clone());
        assert_eq!((c1.start, c2.start), (0.0, 7.0));
        assert_eq!((c1.fade_out, c2.fade_in), (3.0, 3.0));
        let track = |id: Id| q.tracks.iter().position(|t| t.clips.iter().any(|c| c.id == id)).unwrap();
        assert_ne!(track(c1.id), track(c2.id));
        assert_eq!(spaced_length(&[10.0, 20.0], 0.0, 3.0), 27.0);
    }
}
