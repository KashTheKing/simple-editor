//! Corrupt-project repair: salvage whatever a damaged `.sedit` still holds.
//! `Project::load_repaired` is the one entry point (File > Open falls back to it when a normal load fails).
//! It fixes, in order: a BOM / NUL padding / trailing garbage, a file cut off mid-write (closes the open
//! brackets, backing up to the last whole value), then drops only the pieces that no longer deserialize
//! (a top-level field, a list entry, a single clip) and lets everything else through `from_json`'s
//! normal clean-up. Returns the project plus a plain-language list of what was lost.

use super::Project;
use serde_json::Value;

pub struct Repaired {
    pub project: Project,
    pub notes: Vec<String>,
}

impl Project {
    pub fn load_repaired(path: &std::path::Path) -> Result<Repaired, String> {
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        repair_text(&String::from_utf8_lossy(&bytes))
    }
}

/// Text -> JSON value, repairing byte-level damage. Notes say what was done.
fn parse_lenient(text: &str, notes: &mut Vec<String>) -> Result<Value, String> {
    let t = text.trim_start_matches('\u{feff}').trim_matches(|c: char| c == '\0' || c.is_whitespace());
    if t.len() != text.len() {
        notes.push("Removed stray bytes (BOM / NUL padding) around the file".into());
    }
    if let Ok(v) = serde_json::from_str::<Value>(t) {
        return Ok(v);
    }
    // trailing garbage after a complete value: the stream parser stops at the first full object
    let mut de = serde_json::Deserializer::from_str(t).into_iter::<Value>();
    if let Some(Ok(v)) = de.next() {
        notes.push("Dropped unreadable data after the end of the project".into());
        return Ok(v);
    }
    // cut off mid-write: close what is open, backing up to an earlier comma until it parses
    let mut stack: Vec<char> = Vec::new();
    let (mut in_str, mut esc) = (false, false);
    let mut cuts: Vec<(usize, String)> = Vec::new(); // (cut index, closers) at every comma outside a string
    for (i, c) in t.char_indices() {
        if in_str {
            match (esc, c) {
                (true, _) => esc = false,
                (_, '\\') => esc = true,
                (_, '"') => in_str = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' => stack.push('}'),
            '[' => stack.push(']'),
            '}' | ']' => {
                stack.pop();
            }
            ',' => cuts.push((i, stack.iter().rev().collect())),
            _ => {}
        }
    }
    let end: String = stack.iter().rev().collect();
    let mut tries = vec![format!("{t}{}{end}", if in_str { "\"" } else { "" })];
    tries.extend(cuts.iter().rev().take(400).map(|(i, closers)| format!("{}{closers}", &t[..*i])));
    for s in tries {
        if let Ok(v @ Value::Object(_)) = serde_json::from_str::<Value>(&s) {
            notes.push("The file was cut off (an interrupted save?) - the end of it is lost".into());
            return Ok(v);
        }
    }
    Err("not a recognisable project file (no readable JSON)".into())
}

fn ok(base: &Value) -> bool {
    serde_json::from_value::<Project>(base.clone()).is_ok()
}

/// Sets `base[key] = v` when the project still loads; otherwise restores and says false.
fn put(base: &mut Value, key: &str, v: Value) -> bool {
    let old = base.get(key).cloned().unwrap_or(Value::Null);
    base[key] = v;
    if ok(base) {
        return true;
    }
    base[key] = old;
    false
}

pub fn repair_text(text: &str) -> Result<Repaired, String> {
    let mut notes = Vec::new();
    let Value::Object(file) = parse_lenient(text, &mut notes)? else {
        return Err("not a project (the top level is not an object)".into());
    };
    let mut base = serde_json::to_value(Project::new()).map_err(|e| e.to_string())?;
    let whole = Value::Object(file.clone());
    if ok(&whole) {
        base = whole; // nothing structural is wrong
    } else {
        for (k, v) in file {
            if put(&mut base, &k, v.clone()) {
                continue;
            }
            let Value::Array(items) = v else {
                notes.push(format!("Dropped the damaged setting '{k}' (reset to its default)"));
                continue;
            };
            let (mut kept, mut lost) = (Vec::new(), 0);
            base[&k] = Value::Array(vec![]);
            for mut item in items {
                // a damaged clip must not cost the whole track
                if k == "tracks" {
                    salvage_track(&base, &mut item, &mut notes);
                }
                let mut probe = base.clone();
                probe[&k] = Value::Array(kept.iter().cloned().chain([item.clone()]).collect());
                if ok(&probe) {
                    kept.push(item);
                } else if k != "tracks" {
                    lost += 1;
                }
            }
            base[&k] = Value::Array(kept);
            if lost > 0 {
                notes.push(format!("Dropped {lost} damaged entr{} from '{k}'", if lost == 1 { "y" } else { "ies" }));
            }
        }
    }
    let project = Project::from_json(&base.to_string())?;
    Ok(Repaired { project, notes })
}

/// Removes the clips of `track` that do not deserialize.
fn salvage_track(base: &Value, track: &mut Value, notes: &mut Vec<String>) {
    let Some(clips) = track.get("clips").and_then(Value::as_array).cloned() else { return };
    let mut keep = Vec::new();
    for c in clips {
        let mut t = track.clone();
        t["clips"] = Value::Array(vec![c.clone()]);
        let mut probe = base.clone();
        probe["tracks"] = Value::Array(vec![t]);
        if ok(&probe) {
            keep.push(c);
        }
    }
    let lost = track["clips"].as_array().map_or(0, Vec::len) - keep.len();
    if lost > 0 {
        let name = track.get("name").and_then(Value::as_str).unwrap_or("?").to_string();
        notes.push(format!("Dropped {lost} damaged clip(s) from track {name}"));
        track["clips"] = Value::Array(keep);
    }
}

// ---- forum-crash: safe start ----

/// Every clip list in the project: the open timeline, the stashed main timeline, each sequence.
fn every_track_mut(p: &mut Project) -> impl Iterator<Item = &mut super::Track> {
    let stash = p.main_stash.iter_mut().flat_map(|s| s.tracks.iter_mut());
    let seqs = p.sequences.iter_mut().flat_map(|s| s.tracks.iter_mut());
    p.tracks.iter_mut().chain(stash).chain(seqs)
}

/// "Recover without effects": switch off every effect (clips and master/source effects) so a project
/// that crashes in a filter opens. Returns how many were switched off; the caller pushes one undo step
/// first, so Undo brings them all back. A free function on purpose: a recovery step, not an edit tool.
pub fn disable_all_effects(p: &mut Project) -> usize {
    let mut n = 0;
    let mut off = |fx: &mut Vec<super::Effect>| {
        for e in fx.iter_mut().filter(|e| e.enabled) {
            e.enabled = false;
            n += 1;
        }
    };
    for a in p.assets.iter_mut() {
        off(&mut a.effects);
    }
    for t in every_track_mut(p) {
        for c in t.clips.iter_mut() {
            off(&mut c.effects);
        }
    }
    n
}

/// The media files a project reads ("Recover without media" takes these offline for one run).
pub fn media_paths(p: &Project) -> Vec<String> {
    let mut v: Vec<String> = p.assets.iter().map(|a| a.path.clone()).filter(|s| !s.is_empty()).collect();
    v.sort();
    v.dedup();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> String {
        let mut p = Project::new();
        p.name = "keep me".into();
        p.to_json()
    }

    #[test]
    fn a_healthy_file_loads_with_no_notes() {
        let r = repair_text(&sample()).unwrap();
        assert_eq!(r.project.name, "keep me");
        assert!(r.notes.is_empty(), "{:?}", r.notes);
    }

    #[test]
    fn bom_nul_padding_and_trailing_garbage() {
        let s = format!("\u{feff}{}\0\0\0", sample());
        assert_eq!(repair_text(&s).unwrap().project.name, "keep me");
        let s = format!("{}}}}}garbage", sample());
        let r = repair_text(&s).unwrap();
        assert_eq!(r.project.name, "keep me");
        assert!(!r.notes.is_empty());
    }

    #[test]
    fn a_truncated_file_keeps_everything_before_the_cut() {
        let full = sample();
        for cut in [full.len() - 3, full.len() / 2, full.len() / 5] {
            let r = repair_text(&full[..cut]).unwrap_or_else(|e| panic!("cut {cut}: {e}"));
            assert!(r.notes.iter().any(|n| n.contains("cut off")), "{:?}", r.notes);
        }
        assert_eq!(repair_text(&full[..full.len() - 3]).unwrap().project.name, "keep me");
    }

    #[test]
    fn a_damaged_field_or_clip_costs_only_itself() {
        let mut v: Value = serde_json::from_str(&sample()).unwrap();
        v["fps"] = Value::String("fast".into());
        v["name"] = Value::String("still here".into());
        let r = repair_text(&v.to_string()).unwrap();
        assert_eq!(r.project.name, "still here");
        assert!(r.notes.iter().any(|n| n.contains("fps")), "{:?}", r.notes);
        // a clip with a wrong-typed field is dropped, its neighbour stays
        let mut p = Project::new();
        let id = p.add_asset(crate::engine::import::placeholder("x.mp4"));
        let _ = id;
        let mut v: Value = serde_json::from_str(&p.to_json()).unwrap();
        let clip = serde_json::json!({"id": 9, "start": "never"});
        v["tracks"][0]["clips"] = Value::Array(vec![clip]);
        let r = repair_text(&v.to_string()).unwrap();
        assert!(r.project.tracks[0].clips.is_empty());
        assert!(r.notes.iter().any(|n| n.contains("clip")), "{:?}", r.notes);
    }

    #[test]
    fn nonsense_is_an_error_not_a_panic() {
        assert!(repair_text("hello").is_err());
        assert!(repair_text("[1,2]").is_err());
        assert!(repair_text("").is_err());
    }

    #[test]
    fn safe_start_switches_off_every_effect_and_lists_the_media() {
        let mut p = Project::new();
        let id = p.add_asset(crate::engine::import::placeholder(r"C:\v\a.mp4"));
        p.add_asset(crate::engine::import::placeholder(r"C:\v\b.wav"));
        let fx = || super::super::Effect::new(super::super::EffectKind::Blur);
        p.assets[0].effects.push(fx());
        p.insert_asset_clips(id, 0.0, Some(0));
        let clip = p.tracks.iter_mut().flat_map(|t| t.clips.iter_mut()).next().expect("a clip");
        clip.effects.push(fx());
        clip.effects.push(fx());
        clip.effects[1].enabled = false; // already off: not counted
        assert_eq!(disable_all_effects(&mut p), 2);
        assert_eq!(disable_all_effects(&mut p), 0, "nothing left on");
        assert_eq!(media_paths(&p), vec![r"C:\v\a.mp4".to_string(), r"C:\v\b.wav".to_string()]);
    }

    /// Forum report: 100+ markers lost. 250 project markers (points, ranges, notes) survive a save and load.
    #[test]
    fn many_markers_survive_save_and_load() {
        let mut p = Project::new();
        for i in 0..250u32 {
            p.markers.push(super::super::Marker {
                id: 10_000 + i as u64,
                t: i as f64 * 1.5,
                duration: if i % 3 == 0 { 2.0 } else { 0.0 },
                name: format!("Marker {i}"),
                note: format!("note {i} \u{e9}\u{1f3ac}"),
                label: (i % 8) as u8,
                ..Default::default()
            });
        }
        let dir = std::env::temp_dir().join(format!("se-markers-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("markers.sedit");
        p.save(&f).unwrap();
        let back = Project::load(&f).unwrap();
        assert_eq!(back.markers.len(), 250);
        assert_eq!(back.markers, p.markers);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `SE_REPAIR_FILE=<path> cargo test real_file` - run a real project through the repair.
    #[test]
    fn real_file() {
        let Ok(p) = std::env::var("SE_REPAIR_FILE") else { return };
        let r = Project::load_repaired(std::path::Path::new(&p)).unwrap();
        eprintln!("{} tracks, notes: {:?}", r.project.tracks.len(), r.notes);
        Project::load(std::path::Path::new(&p)).unwrap();
    }
}
