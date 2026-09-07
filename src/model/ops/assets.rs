use crate::model::*;
use std::path::{Path, PathBuf};

impl Project {
    pub fn new() -> Self {
        let mut p = Self {
            version: Project::VERSION,
            name: "Untitled".into(),
            width: 1920,
            height: 1080,
            fps: 30.0,
            assets: Vec::new(),
            folders: Vec::new(),
            linked_folders: Vec::new(),
            tracks: Vec::new(),
            in_point: None,
            out_point: None,
            source_video: None,
            subtitles: Vec::new(),
            subtitle_style: TextStyle::subtitle_default(),
            subtitle_margin: 60.0,
            show_subtitles: true,
            subtitle_cont_prefix: String::new(),
            subtitle_cont_suffix: String::new(),
            scaler: Scaler::Bilinear,
            preview_bg: BackgroundMode::Black,
            labels: default_labels(),
            markers: Vec::new(),
            buses: Vec::new(),
            sequences: Vec::new(),
            editing: None,
            main_stash: None,
            plan: Vec::new(),
            notes: Vec::new(),
            moodboard: Vec::new(),
            paths: Vec::new(),
            next_id: 0,
            // ---- ws:registries-schema-hooks ----
            transcripts: Vec::new(),
            subtitle_anim: SubtitleAnim::default(),
            smart_bins: Vec::new(),
            // ---- ws:size-diet ----
            // ---- ws:split-god-files ----
            // ---- ws:audio-analysis ----
            // ---- ws:audio-dsp-automation ----
            // ---- ws:color-engine ----
            // ---- ws:command-palette ----
            // ---- ws:forgiveness ----
            // ---- ws:player-rate-loop ----
            // ---- ws:snap-engine ----
            // ---- ws:trim-model ----
            // ---- ws:canvas-handles-monitor ----
            // ---- ws:export-deliver ----
            // ---- ws:inspector-gallery ----
            // ---- ws:layout-modes-onboarding ----
            // ---- ws:media-library ----
            // ---- ws:source-monitor ----
            // ---- ws:timeline-trim-gestures ----
            // ---- ws:transcript-captions ----
            // ---- ws:pro-monitor ----
            // ---- ws:pro-timeline ----
            // ---- ws:text-titles ----
            // ---- ws:docs-refresh ----
        };
        p.add_track(TrackKind::Video);
        p.add_track(TrackKind::Audio);
        p
    }

    /// New project sized from a media asset, with the asset laid out at t=0 (video + one audio track per stream).
    pub fn from_media(asset: Asset) -> Self {
        let mut p = Self::new();
        if asset.has_video() && asset.width > 0 && asset.height > 0 {
            p.width = asset.width;
            p.height = asset.height;
            if asset.kind == ClipKind::Video && asset.fps > 1.0 {
                p.fps = asset.fps;
            }
        }
        p.name = Path::new(&asset.path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".into());
        if asset.kind == ClipKind::Video {
            p.source_video = Some(asset.path.clone());
        }
        let aid = p.add_asset(asset);
        p.insert_asset_clips(aid, 0.0, None);
        p
    }

    pub fn new_id(&mut self) -> Id {
        self.next_id += 1;
        self.next_id
    }

    // ---------- assets ----------
    pub fn asset(&self, id: Id) -> Option<&Asset> {
        self.assets.iter().find(|a| a.id == id)
    }
    pub fn asset_mut(&mut self, id: Id) -> Option<&mut Asset> {
        self.assets.iter_mut().find(|a| a.id == id)
    }
    pub fn asset_by_path(&self, path: &str) -> Option<&Asset> {
        self.assets.iter().find(|a| a.path.eq_ignore_ascii_case(path))
    }
    // ---- ws:source-monitor ----
    /// Resolve an asset for the Source monitor / Match Frame: prefers an exact `id` when the caller
    /// already knows it, else the first row matching `path`. Path-only lookup always returns the
    /// FIRST asset row with that path — the parent, when the real target is a subclip, since
    /// `add_subclip` copies the parent's `path` verbatim and appends the subclip after it — so an id
    /// must win whenever one is known; path-matching is only the fallback for a file with no known
    /// id (e.g. drag-and-drop from outside the project).
    pub fn asset_for_source(&self, id: Option<Id>, path: &str) -> Option<&Asset> {
        id.and_then(|i| self.asset(i)).or_else(|| self.assets.iter().find(|a| a.path == path))
    }
    /// Adds an asset (de-duplicated by path) and returns its id.
    pub fn add_asset(&mut self, mut a: Asset) -> Id {
        if let Some(e) = self.asset_by_path(&a.path) {
            return e.id;
        }
        a.id = self.new_id();
        let id = a.id;
        self.assets.push(a);
        id
    }
    // ---- ws:registries-schema-hooks ----
    /// Creates a subclip: a new library asset covering `[in_t, out_t)` of `parent`'s source. Copies
    /// `kind`/`path`/`width`/`height`/`fps` from the parent so it decodes like any other asset; `range`
    /// records the window it was cut from. None if `parent` doesn't exist or the range is empty/invalid.
    ///
    /// `in_t`/`out_t` are always relative to `parent`'s own window (0..`parent.duration`), same as for
    /// a master asset. When `parent` is itself a subclip, its stored `range` is already an absolute
    /// offset into the shared `path` (this same rule, applied one level up), so composing onto that
    /// base — rather than storing `in_t`/`out_t` verbatim — is what makes a subclip-of-a-subclip play
    /// the right footage instead of re-reading `path` from its own start.
    pub fn add_subclip(&mut self, parent: Id, in_t: f64, out_t: f64, name: Option<String>) -> Option<Id> {
        if !(in_t.is_finite() && out_t.is_finite() && out_t > in_t) {
            return None;
        }
        let p = self.asset(parent)?.clone(); // owned copy: `new_id` below needs `&mut self`
        let (in_t, out_t) = match p.range {
            Some((base, _)) if p.parent.is_some() => (base + in_t, base + out_t),
            _ => (in_t, out_t),
        };
        let id = self.new_id();
        self.assets.push(Asset {
            id,
            path: p.path,
            kind: p.kind,
            duration: out_t - in_t,
            width: p.width,
            height: p.height,
            fps: p.fps,
            audio_streams: p.audio_streams,
            codec: p.codec,
            folder: p.folder,
            tags: Vec::new(),
            label: 0,
            // subclips are never de-duplicated by path (add_asset's usual rule), so the name lives in
            // `description` — the library has no separate display-name field for assets
            description: name.unwrap_or_default(),
            rel_path: None,
            parent: Some(parent),
            range: Some((in_t, out_t)),
            effects: Vec::new(),
        });
        Some(id)
    }
    // ---- ws:media-library ----
    /// Foreground half of Consolidate Media: repoint `Asset.path` for every `Ok` copy result (an `Err`
    /// entry — copy failed — leaves that asset where it was). Returns how many were repointed. The
    /// caller pushes ONE undo snapshot before calling (`media_sync::tick` / the `media.consolidate`
    /// tool's Mutate wrapper), so a whole consolidate is a single Ctrl+Z.
    pub fn apply_consolidate(&mut self, results: &[(Id, PathBuf, Result<(), String>)]) -> usize {
        let mut n = 0;
        for (id, dst, r) in results {
            if r.is_err() {
                continue;
            }
            let dst = dst.to_string_lossy().into_owned();
            // subclips share their parent's path: repoint every asset on the old path, not just `id`
            let Some(old) = self.asset(*id).map(|a| a.path.clone()) else { continue };
            for a in self.assets.iter_mut().filter(|a| a.path == old) {
                a.path = dst.clone();
                n += 1;
            }
        }
        n
    }
    // ---- ws:source-monitor ----
    /// Subclip from the Source monitor's in/out marks: `add_subclip` with a default name of
    /// "<file> [in–out]" when none is given, so an unnamed subclip is still tellable apart from its
    /// parent in the library.
    pub fn subclip_from_marks(&mut self, asset: Id, in_t: f64, out_t: f64, name: Option<String>) -> Option<Id> {
        let name = name.filter(|n| !n.trim().is_empty()).or_else(|| {
            let stem = self.asset(asset)?.name();
            Some(format!("{stem} [{in_t:.2}–{out_t:.2}]"))
        });
        self.add_subclip(asset, in_t, out_t, name)
    }
    /// Removes an asset and every clip using it — in the live timeline, the stashed main timeline and
    /// every nested sequence (a leftover clip would render black / silent).
    pub fn remove_asset(&mut self, id: Id) {
        self.assets.retain(|a| a.id != id);
        let drop_clips = |tracks: &mut Vec<Track>| {
            for t in tracks.iter_mut() {
                t.clips.retain(|c| !(c.uses_asset() && c.asset == id));
                t.prune_transitions();
            }
        };
        drop_clips(&mut self.tracks);
        if let Some(st) = &mut self.main_stash {
            drop_clips(&mut st.tracks);
        }
        for seq in &mut self.sequences {
            drop_clips(&mut seq.tracks);
        }
        self.tidy();
    }
    /// All folder names (explicit + used by assets), sorted.
    pub fn folder_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.folders.clone();
        v.extend(self.assets.iter().filter(|a| !a.folder.is_empty()).map(|a| a.folder.clone()));
        v.sort();
        v.dedup();
        v
    }
    /// Create (or ensure) a folder; returns false if the name is empty.
    pub fn add_folder(&mut self, name: &str) -> bool {
        let name = name.trim().trim_matches('/').to_string();
        if name.is_empty() {
            return false;
        }
        if !self.folders.contains(&name) {
            self.folders.push(name);
            self.folders.sort();
        }
        true
    }
    /// Remove a folder (and sub-folders); assets inside move to the root.
    pub fn remove_folder(&mut self, name: &str) {
        let prefix = format!("{name}/");
        self.folders.retain(|f| f != name && !f.starts_with(&prefix));
        for a in &mut self.assets {
            if a.folder == name || a.folder.starts_with(&prefix) {
                a.folder.clear();
            }
        }
    }
}

// ---- ws:media-library ----
// Associated fns (no `self`): `ops::assets` is a private module, and these need no Project at all.
impl Project {
    /// Background half of Consolidate Media: copy every `(id, path)` into `dir` (keeping the file
    /// name, uniquified if a different file already holds it) — file I/O only, no `Project` access,
    /// so it satisfies `engine::export::spawn_job`'s `Send + 'static` bound. Paths already under `dir`
    /// are reported `Ok` at their existing location without a copy. `apply_consolidate` consumes the
    /// result on the UI thread.
    pub fn consolidate_assets_copy(dir: &Path, assets: &[(Id, String)]) -> Vec<(Id, PathBuf, Result<(), String>)> {
        assets
            .iter()
            .map(|(id, src)| {
                let src = Path::new(src);
                if Self::path_is_under(src, dir) {
                    return (*id, src.to_path_buf(), Ok(()));
                }
                let Some(name) = src.file_name() else {
                    return (*id, src.to_path_buf(), Err("no file name".into()));
                };
                let mut dst = dir.join(name);
                // ponytail: same-name collision = a different file of that name is already there;
                // suffix rather than overwrite (a same-content check would cost a full read of both)
                let stem = src.file_stem().unwrap_or_default().to_string_lossy().into_owned();
                let ext = src.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
                let mut n = 2;
                while dst.exists() {
                    dst = dir.join(format!("{stem}_{n}{ext}"));
                    n += 1;
                }
                let r = std::fs::copy(src, &dst).map(|_| ()).map_err(|e| format!("{}: {e}", src.display()));
                (*id, dst, r)
            })
            .collect()
    }

    /// Is `path` directly inside `dir` (case-insensitive, either separator)? Compared textually on
    /// the parent — a missing file can't be canonicalized, and an offline asset must still be
    /// reported honestly.
    pub fn path_is_under(path: &Path, dir: &Path) -> bool {
        let norm = |p: &Path| p.to_string_lossy().replace('\\', "/").trim_end_matches('/').to_ascii_lowercase();
        match path.parent() {
            Some(parent) => norm(parent) == norm(dir),
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- ws:media-library ----
    #[test]
    fn add_subclip_gets_its_own_id_not_the_parents() {
        let mut p = Project::new();
        let parent = p.add_asset(asset("C:/clip.mp4"));
        let a = p.add_subclip(parent, 1.0, 2.0, None).unwrap();
        let b = p.add_subclip(parent, 3.0, 4.0, None).unwrap();
        assert_ne!(a, b, "two subclips of one parent must not collapse onto one id");
        assert!(a != parent && b != parent);
        assert!(p.asset(a).is_some() && p.asset(b).is_some());
        assert_eq!(p.assets.len(), 3, "the parent stays, both subclips are rows of their own");
    }

    #[test]
    fn insert_asset_clips_honours_asset_range() {
        let mut p = Project::new();
        let parent = p.add_asset(asset("C:/clip.mp4"));
        let sub = p.add_subclip(parent, 2.0, 5.0, None).unwrap();
        let ids = p.insert_asset_clips(sub, 0.0, None);
        let c = p.clip(ids[0]).unwrap();
        assert_eq!(c.src_in, 2.0, "a subclip starts where its window starts, not at source 0");
        assert_eq!(c.duration, 3.0);
        // the parent itself is unaffected: whole file, from 0
        let ids = p.insert_asset_clips(parent, 10.0, None);
        let c = p.clip(ids[0]).unwrap();
        assert_eq!((c.src_in, c.duration), (0.0, 10.0));
    }

    #[test]
    fn consolidate_assets_copy_skips_files_already_under_dir() {
        let dir = std::env::temp_dir().join(format!("se-consolidate-{}", std::process::id()));
        let outside = dir.join("outside");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(dir.join("inside.mp4"), b"in").unwrap();
        std::fs::write(outside.join("far.mp4"), b"far").unwrap();
        let list = vec![
            (1, dir.join("inside.mp4").to_string_lossy().into_owned()),
            (2, outside.join("far.mp4").to_string_lossy().into_owned()),
            (3, outside.join("gone.mp4").to_string_lossy().into_owned()),
        ];
        let r = Project::consolidate_assets_copy(&dir, &list);
        assert_eq!(r[0].1, dir.join("inside.mp4"), "already inside: reported at its own path");
        assert!(r[0].2.is_ok());
        assert_eq!(r[1].1, dir.join("far.mp4"), "copied in under its own name");
        assert!(r[1].2.is_ok() && dir.join("far.mp4").exists());
        assert_eq!(std::fs::read(dir.join("far.mp4")).unwrap(), b"far");
        assert!(r[2].2.is_err(), "a missing source is an Err, not a panic");
        // a second copy of a different file with the same name gets a suffix, never overwrites
        let other = dir.join("other");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("far.mp4"), b"different").unwrap();
        let r = Project::consolidate_assets_copy(&dir, &[(4, other.join("far.mp4").to_string_lossy().into_owned())]);
        assert_eq!(r[0].1, dir.join("far_2.mp4"));
        assert_eq!(std::fs::read(dir.join("far.mp4")).unwrap(), b"far", "the first copy is untouched");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn apply_consolidate_repoints_only_ok_results() {
        let mut p = Project::new();
        let a = p.add_asset(asset("Z:/a.mp4"));
        let b = p.add_asset(asset("Z:/b.mp4"));
        let sub = p.add_subclip(a, 0.0, 1.0, None).unwrap(); // shares a's path
        let results = vec![
            (a, PathBuf::from("C:/proj/a.mp4"), Ok(())),
            (b, PathBuf::from("C:/proj/b.mp4"), Err("disk full".to_string())),
        ];
        // a + its subclip = 2 rows repointed; b untouched
        assert_eq!(p.apply_consolidate(&results), 2);
        assert_eq!(p.asset(a).unwrap().path, "C:/proj/a.mp4");
        assert_eq!(p.asset(sub).unwrap().path, "C:/proj/a.mp4", "a subclip follows its parent's file");
        assert_eq!(p.asset(b).unwrap().path, "Z:/b.mp4");
        assert!(
            Project::path_is_under(Path::new("C:\\Proj\\a.mp4"), Path::new("c:/proj/")),
            "case/separator-insensitive"
        );
        assert!(
            !Project::path_is_under(Path::new("C:/proj/sub/a.mp4"), Path::new("C:/proj")),
            "a subfolder is not 'under'"
        );
    }

    fn asset(path: &str) -> Asset {
        Asset {
            id: 0,
            path: path.into(),
            kind: ClipKind::Video,
            duration: 10.0,
            width: 1280,
            height: 720,
            fps: 30.0,
            audio_streams: Vec::new(),
            codec: "h264".into(),
            folder: String::new(),
            tags: Vec::new(),
            label: 0,
            description: String::new(),
            rel_path: None,
            parent: None,
            range: None,
            effects: Vec::new(),
        }
    }

    // deviation (see PR body): this only exercises `Project::add_subclip` directly. The issue's Tests
    // table for this row also names "the media.subclip MCP tool round-trips to the same result", but
    // that half is untested here — same App-construction limitation as the other App-dependent tests
    // (see tools_registry_tests.rs), just not called out there as a deviation until now.
    #[test]
    fn add_subclip_creates_ranged_asset() {
        let mut p = Project::new();
        let parent = p.add_asset(asset("C:/clip.mp4"));
        let sub = p.add_subclip(parent, 2.0, 5.0, Some("Best take".into())).unwrap();
        let a = p.asset(sub).unwrap();
        assert_eq!(a.parent, Some(parent));
        assert_eq!(a.range, Some((2.0, 5.0)));
        assert_eq!(a.duration, 3.0);
        assert_eq!(a.description, "Best take");
        assert_eq!(a.path, "C:/clip.mp4"); // decodes like any other asset
                                           // an out-of-order/degenerate range is refused
        assert!(p.add_subclip(parent, 5.0, 2.0, None).is_none());
        assert!(p.add_subclip(999, 0.0, 1.0, None).is_none(), "unknown parent");
    }

    // ---- ws:media-library review fix ----
    /// A subclip of a subclip must compose onto the parent's own absolute window, not read `in_t`/
    /// `out_t` as if they were offsets into the root file: master 0..100, `s1` = 10..20, and a subclip
    /// of `s1` over its own local 2..5 must land on the root's 12..15 — the exact scenario from the
    /// PR #55 review (marks (2,5) on a subclip of 10..20 must play 12..15, not 2..5).
    #[test]
    fn add_subclip_of_a_subclip_composes_absolute_range() {
        let mut p = Project::new();
        let master = p.add_asset(asset("C:/master.mp4"));
        let s1 = p.add_subclip(master, 10.0, 20.0, None).unwrap();
        assert_eq!(p.asset(s1).unwrap().range, Some((10.0, 20.0)));

        let s2 = p.add_subclip(s1, 2.0, 5.0, None).unwrap();
        let a = p.asset(s2).unwrap();
        assert_eq!(a.range, Some((12.0, 15.0)), "local marks (2,5) on s1 (10..20) compose to 12..15");
        assert_eq!(a.duration, 3.0);
        assert_eq!(a.parent, Some(s1), "lineage still points at the immediate parent, not the root");
        assert_eq!(a.path, "C:/master.mp4");

        // one more level: the same rule applies again, using s2's own (already-absolute) base
        let s3 = p.add_subclip(s2, 1.0, 2.0, None).unwrap();
        assert_eq!(p.asset(s3).unwrap().range, Some((13.0, 14.0)));
    }

    // ---- ws:source-monitor ----
    /// The Source monitor's subclip records the parent + [in,out) with duration out−in, and names an
    /// unnamed one after its file and range so it never looks identical to the parent row.
    #[test]
    fn subclip_from_marks_creates_ranged_asset() {
        let mut p = Project::new();
        let parent = p.add_asset(asset("C:/clip.mp4"));
        let sub = p.subclip_from_marks(parent, 1.5, 4.0, None).unwrap();
        let a = p.asset(sub).unwrap();
        assert_eq!(a.parent, Some(parent));
        assert_eq!(a.range, Some((1.5, 4.0)));
        assert_eq!(a.duration, 2.5);
        assert_eq!(a.description, "clip.mp4 [1.50–4.00]", "default name from file + range");
        let named = p.subclip_from_marks(parent, 0.0, 1.0, Some("Take 2".into())).unwrap();
        assert_eq!(p.asset(named).unwrap().description, "Take 2");
        assert!(p.subclip_from_marks(parent, 4.0, 1.5, None).is_none(), "inverted marks refused");
        assert!(p.subclip_from_marks(999, 0.0, 1.0, None).is_none(), "unknown parent");
    }

    // ---- ws:source-monitor ----
    // deviation (see PR body): the real bug lived in `App::source_open_now` (Match Frame / `source.open`
    // resolving the wrong asset for a subclip), but `App` needs a real `eframe::CreationContext` and has
    // no headless test harness (same App-construction limitation as `tools_registry_tests.rs` and the
    // `add_subclip` test above), so this exercises `asset_for_source` directly — the exact lookup
    // `source_open_now` now calls to resolve `SourceState.asset`.
    /// A subclip shares its parent's `path` and is appended after it in `Project::assets`, so a
    /// path-only lookup (id unknown) always finds the parent — opening the subclip by its own asset id
    /// must resolve to the subclip itself, not the parent sharing its path.
    #[test]
    fn asset_for_source_prefers_id_over_first_path_match() {
        let mut p = Project::new();
        let parent = p.add_asset(asset("C:/clip.mp4"));
        let sub = p.add_subclip(parent, 2.0, 5.0, None).unwrap();
        let path = p.asset(parent).unwrap().path.clone();
        assert_eq!(p.asset(sub).unwrap().path, path, "subclip shares the parent's path");
        // no id known (e.g. a bare path with no library match yet) -> the first row with that path
        assert_eq!(p.asset_for_source(None, &path).map(|a| a.id), Some(parent));
        // the subclip's own id -> the subclip itself, not the parent
        assert_eq!(p.asset_for_source(Some(sub), &path).map(|a| a.id), Some(sub));
        // the parent's own id still resolves to the parent
        assert_eq!(p.asset_for_source(Some(parent), &path).map(|a| a.id), Some(parent));
        // an unknown id falls back to the path match rather than resolving to nothing
        assert_eq!(p.asset_for_source(Some(999), &path).map(|a| a.id), Some(parent));
    }
}
