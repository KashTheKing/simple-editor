//! The four kinds of saved Gallery things that used to live inside `settings.json` - looks (effect
//! presets), caption styles, templates / titles and keyframe blocks - now live as files in the gallery
//! folder (`engine::gallery`) like every other kind. The `Settings` vectors stay as the in-memory view the
//! rest of the app reads and writes (so no save site changes): `load_into` fills them from the folder at
//! start (importing what an older `settings.json` held, once), and `mirror` - called by `Settings::save` -
//! writes any change back to files. The vectors are no longer written to `settings.json`.
//!
//! An item the app did not put there itself (a file dropped into the folder by hand) is never deleted by
//! `mirror`: only items it loaded or wrote earlier, and that the user has since removed, go to `.trash`.

use super::gallery::{self, Collide, Kind};
use crate::settings::Settings;
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;
use std::sync::Mutex;

const KINDS: [Kind; 4] = [Kind::Look, Kind::Template, Kind::Caption, Kind::Animate];

struct Mirrored {
    hash: u64,
    known: HashSet<(Kind, String)>,
}

static STATE: Mutex<Option<Mirrored>> = Mutex::new(None);

/// The settings' items of the four kinds as (kind, name, payload).
fn collect(s: &Settings) -> Vec<(Kind, String, Value)> {
    let mut out = Vec::new();
    for p in &s.effect_presets {
        out.push((Kind::Look, p.name.clone(), serde_json::from_str(&p.json).unwrap_or(Value::Null)));
    }
    for t in &s.templates {
        out.push((Kind::Template, t.name.clone(), serde_json::from_str(&t.json).unwrap_or(Value::Null)));
    }
    for c in &s.caption_presets {
        out.push((Kind::Caption, c.text.clone(), serde_json::to_value(c).unwrap_or(Value::Null)));
    }
    for b in &s.key_blocks {
        out.push((Kind::Animate, b.name.clone(), serde_json::to_value(b).unwrap_or(Value::Null)));
    }
    out
}

/// Replace the four vectors with the folder's items.
fn fill(s: &mut Settings, root: &Path) {
    s.effect_presets = gallery::list(root, Kind::Look)
        .into_iter()
        .map(|i| crate::settings::EffectPreset { name: i.name, json: i.data.to_string() })
        .collect();
    s.templates = gallery::list(root, Kind::Template)
        .into_iter()
        .map(|i| crate::settings::Template { name: i.name, json: i.data.to_string() })
        .collect();
    s.caption_presets = gallery::list(root, Kind::Caption)
        .into_iter()
        .filter_map(|i| {
            serde_json::from_value::<crate::model::TextStyle>(i.data).ok().map(|mut st| {
                st.text = i.name;
                st
            })
        })
        .collect();
    s.key_blocks = gallery::list(root, Kind::Animate)
        .into_iter()
        .filter_map(|i| {
            serde_json::from_value::<crate::model::KeyBlock>(i.data).ok().map(|mut b| {
                b.name = i.name;
                b
            })
        })
        .collect();
}

fn hash_of(items: &[(Kind, String, Value)]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for (k, n, v) in items {
        k.name().hash(&mut h);
        n.hash(&mut h);
        v.to_string().hash(&mut h);
    }
    h.finish()
}

fn remember(s: &Settings) {
    let items = collect(s);
    let known = items.iter().map(|(k, n, _)| (*k, n.to_lowercase())).collect();
    *STATE.lock().unwrap() = Some(Mirrored { hash: hash_of(&items), known });
}

/// At start: import what an older `settings.json` held (once, marked by `.legacy-imported`), then fill the
/// vectors from the folder.
pub fn load_into(s: &mut Settings, root: &Path) {
    let marker = root.join(".legacy-imported");
    if !marker.exists() {
        for (kind, name, data) in collect(s) {
            let _ = gallery::save(root, kind, &name, &[], data, Collide::Number);
        }
        let _ = std::fs::create_dir_all(root);
        let _ = std::fs::write(&marker, "items from settings.json were moved into this folder\n");
    }
    fill(s, root);
    remember(s);
    let _ = gallery::ensure_index(root); // (rebuilt from the files when it is missing)
}

/// Re-read the folder (after an import, a rename or a delete done there).
pub fn reload(s: &mut Settings, root: &Path) {
    fill(s, root);
    remember(s);
}

/// Write the vectors' changes to files: new / changed items are saved, items the app knew that were removed
/// move to `.trash`.
pub fn mirror(s: &Settings, root: &Path) {
    let items = collect(s);
    let hash = hash_of(&items);
    let mut st = STATE.lock().unwrap();
    if st.as_ref().is_some_and(|m| m.hash == hash) {
        return;
    }
    let known = st.take().map(|m| m.known).unwrap_or_default();
    let mut now_known = HashSet::new();
    for kind in KINDS {
        let existing = gallery::list(root, kind);
        for (_, name, data) in items.iter().filter(|(k, ..)| *k == kind) {
            now_known.insert((kind, name.to_lowercase()));
            match existing.iter().find(|i| i.name.eq_ignore_ascii_case(name)) {
                Some(i) if i.data == *data => {}
                Some(i) => {
                    let _ = gallery::replace_data(root, i, data.clone());
                }
                None => {
                    let _ = gallery::save(root, kind, name, &[], data.clone(), Collide::Number);
                }
            }
        }
        for i in existing {
            let key = (kind, i.name.to_lowercase());
            if known.contains(&key) && !now_known.contains(&key) {
                let _ = gallery::delete(root, &i);
            }
        }
    }
    *st = Some(Mirrored { hash, known: now_known });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{EffectPreset, Template};

    fn tmp(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("se-gsync-{tag}-{}-{}", std::process::id(), Settings::now()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    /// The statics make these tests share state: one test, run in order.
    #[test]
    fn legacy_import_mirror_and_hand_dropped_files() {
        let root = tmp("a");
        let mut s = Settings::default();
        s.effect_presets.push(EffectPreset { name: "Warm".into(), json: "[]".into() });
        s.effect_presets.push(EffectPreset { name: "Graph".into(), json: "{\"nodes\":[]}".into() });
        s.templates.push(Template { name: "Intro".into(), json: "{\"clips\":[],\"assets\":[]}".into() });
        s.key_blocks.push(crate::model::KeyBlock { name: "Pop".into(), dur: 1.0, tracks: vec![] });
        let mut style = crate::engine::presets::builtin_caption_styles().remove(0);
        style.text = "Mine bold".into();
        s.caption_presets.push(style);
        // 1. an older settings.json's items move into files once
        load_into(&mut s, &root);
        assert!(root.join(".legacy-imported").is_file());
        for (k, n) in [
            (Kind::Look, "Warm"),
            (Kind::Look, "Graph"),
            (Kind::Template, "Intro"),
            (Kind::Animate, "Pop"),
            (Kind::Caption, "Mine bold"),
        ] {
            assert!(gallery::find(&root, k, n).is_some(), "{n} is a file now");
        }
        assert_eq!(s.effect_presets.len(), 2, "and the vectors are filled back from them");
        assert!(
            s.effect_presets.iter().any(|p| p.name == "Graph" && p.json.trim_start().starts_with('{')),
            "a graph stays a graph"
        );
        // a second start does not import again
        let mut again = Settings::default();
        again.effect_presets.push(EffectPreset { name: "Old".into(), json: "[]".into() });
        load_into(&mut again, &root);
        assert!(gallery::find(&root, Kind::Look, "Old").is_none() && again.effect_presets.len() == 2);
        // 2. a save in the app (a push + Settings::save) lands as a file; a removal goes to .trash
        s.effect_presets.push(EffectPreset { name: "New look".into(), json: "[1]".into() });
        s.effect_presets.retain(|p| p.name != "Warm");
        // a file dropped in by hand is not the app's to delete
        gallery::save(&root, Kind::Look, "Dropped in", &[], serde_json::json!([2]), Collide::Number).unwrap();
        mirror(&s, &root);
        assert!(gallery::find(&root, Kind::Look, "New look").is_some());
        assert!(gallery::find(&root, Kind::Look, "Warm").is_none(), "removed in the app -> trashed");
        assert!(gallery::find(&root, Kind::Look, "Dropped in").is_some(), "never deleted by the mirror");
        // 3. reload picks the dropped-in file up
        reload(&mut s, &root);
        assert!(s.effect_presets.iter().any(|p| p.name == "Dropped in"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
