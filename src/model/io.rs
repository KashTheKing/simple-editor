use crate::model::*;
use std::path::Path;

/// The `assistant_chats` array of a saved project, as JSON text (None = no such key / unreadable).
pub fn chats_json_in(path: &Path) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct F {
        assistant_chats: Option<serde_json::Value>,
    }
    let f: F = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    f.assistant_chats.map(|v| v.to_string())
}

#[cfg(test)]
thread_local! {
    /// Whole-project serialisations on this thread: the instant-trims regression test asserts an edit's
    /// commit makes none (`trim_commit_does_not_serialise_the_project`).
    pub(crate) static TO_JSON_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl Project {
    // ---------- persistence ----------
    pub fn to_json(&self) -> String {
        #[cfg(test)]
        TO_JSON_CALLS.with(|n| n.set(n.get() + 1));
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
    pub fn from_json(s: &str) -> Result<Self, String> {
        let mut p: Project = serde_json::from_str(s).map_err(|e| e.to_string())?;
        let max_id = p
            .assets
            .iter()
            .map(|a| a.id)
            .chain(p.tracks.iter().map(|t| t.id))
            .chain(p.tracks.iter().flat_map(|t| t.transitions.iter().map(|x| x.id)))
            .chain(p.subtitles.iter().map(|c| c.id))
            .chain(p.markers.iter().map(|m| m.id))
            .chain(p.buses.iter().map(|b| b.id))
            .chain(p.all_clips().flat_map(|(_, c)| c.markers.iter().map(|m| m.id)))
            .chain(p.all_clips().filter_map(|(_, c)| c.graph.as_ref()).flat_map(|g| g.nodes.iter().map(|n| n.id)))
            .chain(p.notes.iter().map(|n| n.id))
            .chain(p.sequences.iter().map(|s| s.id))
            .chain(p.sequences.iter().flat_map(|s| s.tracks.iter().map(|t| t.id)))
            .chain(
                p.sequences.iter().flat_map(|s| s.tracks.iter().flat_map(|t| t.clips.iter().map(|c| c.id.max(c.link)))),
            )
            .chain(
                p.main_stash
                    .iter()
                    .flat_map(|s| s.tracks.iter().flat_map(|t| t.clips.iter().map(|c| c.id.max(c.link)))),
            )
            .chain(p.all_clips().map(|(_, c)| c.id.max(c.link)))
            .max()
            .unwrap_or(0);
        fn plan_max(items: &[PlanItem]) -> Id {
            items.iter().map(|i| i.id.max(plan_max(&i.children))).max().unwrap_or(0)
        }
        p.next_id = p.next_id.max(max_id).max(plan_max(&p.plan));
        // a note migrated from the old flat-string format (or a hand-edited one) has no real id yet
        for i in 0..p.notes.len() {
            if p.notes[i].id == 0 {
                p.notes[i].id = p.new_id();
            }
        }
        if p.editing.is_some_and(|id| p.sequence(id).is_none()) {
            p.editing = None;
        }
        // hand-edited files: keep every value in the range the UI can produce (fps 0 would make NaN times)
        if !(p.fps >= 1.0 && p.fps <= 1000.0) {
            p.fps = 30.0;
        }
        // ws:project-color-fps: a raw measured rate (26.0221...) from older builds becomes a clean one
        p.fps = normalize_fps(p.fps);
        for s in &mut p.sequences {
            s.fps = normalize_fps(if s.fps >= 1.0 { s.fps } else { 30.0 });
        }
        if let Some(st) = p.main_stash.as_mut() {
            st.fps = normalize_fps(st.fps);
        }
        p.width = p.width.max(16);
        p.height = p.height.max(16);
        for t in &mut p.tracks {
            t.clips.retain(|c| c.start >= 0.0 && c.duration.is_finite() && c.duration > 0.0);
            for c in &mut t.clips {
                if !(c.speed.is_finite() && c.speed > 0.0) {
                    c.speed = 1.0;
                }
                c.speed_curve.keys.retain(|k| k.t.is_finite() && k.v.is_finite() && k.v > 0.0);
                if !c.speed_curve.is_animated() {
                    c.speed_curve.value = c.speed; // older files have no curve at all
                }
                for e in &mut c.effects {
                    // older files / edited files: make sure every parameter exists
                    let specs = e.specs();
                    while e.params.len() < specs.len() {
                        let d = specs[e.params.len()].default;
                        e.params.push(Animated::new(d));
                    }
                }
            }
        }
        p.subtitles.retain(|c| c.start.is_finite() && c.end.is_finite() && c.end > c.start);
        if p.labels.is_empty() {
            p.labels = default_labels();
        }
        p.markers.retain(|m| m.t.is_finite() && m.t >= 0.0);
        p.moodboard.retain(|m| p.assets.iter().any(|a| a.id == m.asset));
        p.sort_markers();
        p.tidy();
        p.sort_cues();
        // ---- ws:registries-schema-hooks ----
        // a file with no `ripple` field (or a hand-edited one) never keeps a track at None: resolve
        // through the same per-kind-index rule every construction site uses. Every independent track
        // list gets this (the main `tracks`, each nested sequence's own tracks, and `main_stash`'s
        // tracks when a sequence was open at save time) - each with its own V1/A1 counters, since a
        // sequence's V1 is not the main timeline's V1.
        fn resolve_ripple(tracks: &mut [Track]) {
            let (mut vi, mut ai) = (0usize, 0usize);
            for t in tracks {
                let kind = t.kind;
                let idx = if kind == TrackKind::Video {
                    vi += 1;
                    vi - 1
                } else {
                    ai += 1;
                    ai - 1
                };
                if t.ripple.is_none() {
                    t.ripple = Track::default_ripple(kind, idx);
                }
            }
        }
        resolve_ripple(&mut p.tracks);
        for s in &mut p.sequences {
            resolve_ripple(&mut s.tracks);
        }
        if let Some(st) = &mut p.main_stash {
            resolve_ripple(&mut st.tracks);
        }
        if p.version < 3 {
            p.migrate_sequence_audio();
        }
        // a Crop effect from before crop was built in becomes the clip's own crop where nothing changes
        for t in p.tracks.iter_mut().chain(p.sequences.iter_mut().flat_map(|s| s.tracks.iter_mut())) {
            t.clips.iter_mut().for_each(|c| _ = c.migrate_crop_effect());
        }
        if let Some(st) = &mut p.main_stash {
            st.tracks.iter_mut().flat_map(|t| t.clips.iter_mut()).for_each(|c| _ = c.migrate_crop_effect());
        }
        // an older (or missing/default) version is silently brought forward - every field it lacks
        // already resolved a default above; a NEWER version is left alone so `newer_than_app` can
        // still tell the caller (a hard downgrade would be the only real data-loss risk, and this
        // never removes fields, only adds them)
        if p.version < Self::VERSION {
            p.version = Self::VERSION;
        }
        Ok(p)
    }
    /// The `.sedit` schema version this build writes and reads without a migration. A file saved by a
    /// newer build (`version > VERSION`) still loads - unknown fields are simply dropped on the next
    /// save - but the caller should toast a warning (see `newer_than_app`); this is not itself an error.
    /// v3: a nested sequence's sound plays from a linked Sequence clip on an audio track, not from the
    /// video track (`migrate_sequence_audio`). Undo snapshots are v3, so a deleted twin stays deleted.
    pub const VERSION: u32 = 3;
    /// True when `self.version` is newer than this build understands (see `VERSION`). Unused outside
    /// tests this wave - the caller that toasts it (`App::open_project`) is outside this workstream's
    /// files; this only exposes the bool.
    #[allow(dead_code)]
    pub fn newer_than_app(&self) -> bool {
        self.version > Self::VERSION
    }
    /// Writes beside the target then renames, so a failed write can't destroy the previous save.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        self.save_with_chats(path, None)
    }
    /// `save` plus the Assistant's chats as a top-level `assistant_chats` key (JSON array text). They
    /// live beside the project, not in `Project` (undo snapshots would carry - and revert - them).
    /// `None` keeps whatever the file being replaced already holds, so a save that doesn't know the
    /// chats (an MCP save, a recovery) never wipes them.
    pub fn save_with_chats(&self, path: &Path, chats: Option<String>) -> std::io::Result<()> {
        let mut json = self.to_json();
        let chats = chats.or_else(|| chats_json_in(path));
        if let (Some(c), Some(head)) = (chats, json.trim_end().strip_suffix('}')) {
            json = format!("{head},
  \"assistant_chats\": {c}
}}");
        }
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, path)
    }
    pub fn load(path: &Path) -> Result<Self, String> {
        let s = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        Self::from_json(&s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One video clip on V1 (a `Project::new` plus a hand-made clip; nothing is decoded here).
    fn one_clip() -> Project {
        let mut p = Project::new();
        p.tracks[0].clips.push(crate::model::Clip::new(7, crate::model::ClipKind::Video, "v", 0.0, 4.0));
        p
    }

    /// The built-in crop survives save and load, keys and all, and a clip that never crops writes no
    /// crop fields (old builds read the file unchanged).
    #[test]
    fn built_in_crop_round_trips() {
        let mut p = one_clip();
        let plain = p.to_json();
        assert!(!plain.contains("crop_"), "an uncropped clip writes no crop fields");
        let c = &mut p.tracks[0].clips[0];
        c.crop_left.value = 0.1;
        c.crop_top.key_at(0.0, 0.0);
        c.crop_top.key_at(2.0, 0.4);
        let back = Project::from_json(&p.to_json()).unwrap();
        let c = &back.tracks[0].clips[0];
        assert_eq!(c.crop_left.value, 0.1);
        assert_eq!(c.crop_top.keys.len(), 2, "keyframes kept");
        let [l, r, t, b] = c.crop_at(1.0);
        assert_eq!((l, r, b), (0.1, 0.0, 0.0));
        assert!(t > 0.0 && t < 0.4, "keyed top in between: {t}");
        assert_eq!(c.crop_at(2.0)[2], 0.4);
        // driven like every other property: by name (Inspector / MCP / blocks) and in the key list
        let mut c = c.clone();
        assert!(crate::model::prop_mut(&mut c, "crop_bottom").is_some());
        assert!(c.props_mut().iter().any(|(l, _)| *l == "Crop Bottom"));
        assert!(c.key_times().contains(&2.0));
        c.shift_keys(1.0);
        assert_eq!(c.crop_top.keys[1].t, 3.0, "keys move with the clip's other keys");
    }

    /// An old project's Crop effect becomes the built-in crop when that is lossless, and stays an effect
    /// (still rendering) when it is not.
    #[test]
    fn crop_effect_migrates_on_load() {
        use crate::model::{Effect, EffectKind};
        let crop = |v: [f64; 5]| {
            let mut e = Effect::new(EffectKind::Crop);
            for (i, x) in v.iter().enumerate() {
                e.params[i].value = *x;
            }
            e
        };
        let mut p = one_clip();
        p.tracks[0].clips[0].effects = vec![Effect::new(EffectKind::Blur), crop([0.1, 0.2, 0.0, 0.3, 0.0])];
        p.tracks[0].clips[0].effects[1].params[0].key_at(1.0, 0.25);
        // a second clip whose crop feathers: not lossless
        let mut f = crate::model::Clip::new(8, crate::model::ClipKind::Video, "f", 4.0, 2.0);
        f.effects = vec![crop([0.1, 0.0, 0.0, 0.0, 0.2])];
        p.tracks[0].clips.push(f);
        // a third with a Blur after its crop (the blur would bleed over the cut edge): not lossless
        let mut g = crate::model::Clip::new(9, crate::model::ClipKind::Video, "g", 6.0, 2.0);
        g.effects = vec![crop([0.1, 0.0, 0.0, 0.0, 0.0]), Effect::new(EffectKind::Blur)];
        p.tracks[0].clips.push(g);
        let back = Project::from_json(&p.to_json()).unwrap();
        let c = &back.tracks[0].clips[0];
        assert_eq!(c.effects.iter().map(|e| e.kind).collect::<Vec<_>>(), vec![EffectKind::Blur], "Crop folded in");
        assert_eq!((c.crop_left.keys.len(), c.crop_right.value, c.crop_bottom.value), (1, 0.2, 0.3));
        assert_eq!(c.crop_left.keys[0].v, 0.25, "its keys came along");
        assert_eq!(back.tracks[0].clips[1].effects[0].kind, EffectKind::Crop, "feathered crop kept");
        assert_eq!(back.tracks[0].clips[1].crop_at(0.0), [0.0; 4]);
        assert_eq!(back.tracks[0].clips[2].effects.len(), 2, "crop under a blur kept");
        // loading again changes nothing
        let again = Project::from_json(&back.to_json()).unwrap();
        assert_eq!(again.to_json(), back.to_json());
    }

    /// A `.sedit` with no `ripple` field on any track (every project saved before this workstream)
    /// resolves through the same per-kind-index rule as a fresh construction, and its version is
    /// brought forward.
    #[test]
    fn ripple_resolves_on_load() {
        let mut p = Project::new(); // V1, A1
        p.add_track(TrackKind::Video); // V2
        let mut json: serde_json::Value = serde_json::from_str(&p.to_json()).unwrap();
        json["version"] = serde_json::json!(1);
        for t in json["tracks"].as_array_mut().unwrap() {
            t.as_object_mut().unwrap().remove("ripple");
        }
        let loaded = Project::from_json(&json.to_string()).unwrap();
        // video tracks stay before audio tracks: [V1, V2, A1], not insertion order
        assert_eq!(loaded.tracks[0].ripple, Some(true), "V1");
        assert_eq!(loaded.tracks[1].ripple, Some(false), "V2 (second video track)");
        assert_eq!(loaded.tracks[2].ripple, Some(true), "A1");
        assert_eq!(loaded.version, Project::VERSION, "brought forward to the current version");
    }

    /// The same resolution must also reach a nested sequence's own tracks and, when the file was saved
    /// while a sequence was open (so the real main timeline sits in `main_stash`), that stash's tracks
    /// too - each with its own independent V1/A1 counters. Regression test for a bug where the loop only
    /// walked `p.tracks`.
    #[test]
    fn ripple_resolves_on_load_for_sequences_and_main_stash() {
        // a closed sequence: its tracks live in `p.sequences[0].tracks`
        let mut p = Project::new(); // main V1, A1
        let seq_id = p.new_sequence("Seq", 1920, 1080, 30.0); // sequence's own V1, A1
        let mut json: serde_json::Value = serde_json::from_str(&p.to_json()).unwrap();
        json["version"] = serde_json::json!(1);
        for t in json["sequences"][0]["tracks"].as_array_mut().unwrap() {
            t.as_object_mut().unwrap().remove("ripple");
        }
        let loaded = Project::from_json(&json.to_string()).unwrap();
        let seq = loaded.sequence(seq_id).unwrap();
        assert_eq!(seq.tracks[0].ripple, Some(true), "sequence's own V1");
        assert_eq!(seq.tracks[1].ripple, Some(true), "sequence's own A1");
        // the sequence is open when saved: the real main timeline's tracks are stashed in `main_stash`
        let mut p2 = Project::new();
        let seq_id2 = p2.new_sequence("Seq2", 1920, 1080, 30.0);
        assert!(p2.open_sequence(seq_id2));
        let mut json2: serde_json::Value = serde_json::from_str(&p2.to_json()).unwrap();
        json2["version"] = serde_json::json!(1);
        for t in json2["main_stash"]["tracks"].as_array_mut().unwrap() {
            t.as_object_mut().unwrap().remove("ripple");
        }
        let loaded2 = Project::from_json(&json2.to_string()).unwrap();
        let stash = loaded2.main_stash.as_ref().expect("sequence was open, so main_stash must round-trip");
        assert_eq!(stash.tracks[0].ripple, Some(true), "stashed main V1");
        assert_eq!(stash.tracks[1].ripple, Some(true), "stashed main A1");
    }

    #[test]
    fn newer_project_warns() {
        let p = Project::new();
        let mut json: serde_json::Value = serde_json::from_str(&p.to_json()).unwrap();
        json["version"] = serde_json::json!(Project::VERSION + 1);
        let loaded = Project::from_json(&json.to_string()).unwrap();
        assert!(loaded.newer_than_app(), "a version newer than this build must be flagged");
        assert_eq!(loaded.version, Project::VERSION + 1, "the newer version itself is preserved, not clamped");
    }
}

#[cfg(test)]
mod big_open_bench {
    use super::*;
    /// The open path of a big real project, step by step (never commit numbers from it - it reads
    /// whatever file you point it at): `SE_BIG_PROJECT=<a COPY of a .sedit> cargo test
    /// big_project_open_steps -- --ignored --nocapture`. The HTML part is what `html_ctl::reconcile`
    /// does per layer per scan: the first scan prepares the pages, the next ones must be ~free.
    #[test]
    #[ignore]
    fn big_project_open_steps() {
        let Ok(path) = std::env::var("SE_BIG_PROJECT") else { return };
        let t = std::time::Instant::now();
        let p = Project::load(Path::new(&path)).unwrap();
        eprintln!("load {:?}", t.elapsed());
        let t = std::time::Instant::now();
        let st = crate::engine::style::style_summary(&p);
        eprintln!("style_summary {} chars {:?}", st.len(), t.elapsed());
        use crate::engine::html;
        let dir = Path::new(&path).parent().map(Path::to_path_buf);
        let media = p.assets.iter().filter(|a| a.html.is_none()).map(|a| std::path::PathBuf::from(&a.path)).collect();
        html::set_media(html::Media::new(dir, media));
        for pass in ["cold", "warm"] {
            let t = std::time::Instant::now();
            for h in p.assets.iter().filter_map(|a| a.html.as_ref()) {
                let spec = html::Spec {
                    html: h.src.clone(),
                    fields: h.field_list(),
                    scale: html::scale_for(p.width, p.height),
                    fps: p.fps,
                    duration: h.duration,
                    base: String::new(),
                    params: html::params_of(&h.src, &h.params),
                    width: h.width,
                    mouse: h.mouse.clone(),
                    focus: None,
                    renderer: h.renderer,
                };
                let _ = html::lookup(&spec);
            }
            eprintln!("html scan ({pass}) {:?}", t.elapsed());
        }
    }
}
