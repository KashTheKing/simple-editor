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

    /// `SE_REPAIR_FILE=<path> cargo test real_file` - run a real project through the repair.
    #[test]
    fn real_file() {
        let Ok(p) = std::env::var("SE_REPAIR_FILE") else { return };
        let r = Project::load_repaired(std::path::Path::new(&p)).unwrap();
        eprintln!("{} tracks, notes: {:?}", r.project.tracks.len(), r.notes);
        Project::load(std::path::Path::new(&p)).unwrap();
    }
}
