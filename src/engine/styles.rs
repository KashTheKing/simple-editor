//! ---- feat/styles-context ----
//! Editing Styles: a library of Markdown files, one style each, in `%APPDATA%\SimpleEditor\styles`, plus the
//! built-ins compiled in from `library/styles/`. The `---` front matter holds the measurable rules (average
//! shot length, cuts on the beat, transitions, caption look, loudness target, grade, pacing curve, words
//! per minute); the body is prose guidance. `/style <name>` in the Assistant writes one into the project's
//! Project Context (`model::context::apply_style`): the prose into Style guide, the rules into Rules.
//! A user file shadows a built-in of the same name; built-ins can be duplicated, not changed.

use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    User,
    BuiltIn,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::User => "Yours",
            Source::BuiltIn => "Built-in",
        }
    }
}

/// The measurable part (front matter). Every field is optional: a hand-written style may give only some.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rules {
    /// Average shot length, seconds.
    pub avg_shot: Option<f64>,
    /// Share of cuts that land on a beat of the music, 0..1.
    pub cut_on_beat: Option<f64>,
    /// "cut", "dissolve 0.5", "flash 0.15" (kind, then a typical duration in seconds).
    pub transitions: Vec<String>,
    pub captions: String,
    /// Integrated loudness target, LUFS.
    pub loudness: Option<f64>,
    /// Grade / LUT / look.
    pub grade: String,
    /// Average shot length per quarter of the video (seconds), start to end.
    pub pacing: Vec<f64>,
    /// Speech rate, words per minute.
    pub wpm: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    pub name: String,
    /// The display title ("Fast-cut vlog"); the name when the file has none.
    pub title: String,
    pub description: String,
    pub rules: Rules,
    pub body: String,
    /// The skill that does the whole pass (`/style-vlog`), when there is one.
    pub skill: String,
    /// Front-matter keys this module does not know, kept so a save never drops them.
    pub extra: Vec<(String, String)>,
    pub source: Source,
    pub path: Option<PathBuf>,
}

/// The styles compiled into the exe.
pub const BUILT_IN: &[(&str, &str)] = &[
    ("documentary", include_str!("../../library/styles/documentary.md")),
    ("music-video", include_str!("../../library/styles/music-video.md")),
    ("podcast-clips", include_str!("../../library/styles/podcast-clips.md")),
    ("shorts", include_str!("../../library/styles/shorts.md")),
    ("trailer", include_str!("../../library/styles/trailer.md")),
    ("vlog", include_str!("../../library/styles/vlog.md")),
];

/// Lowercase letters, digits and `-`, not empty (the same rule as skills).
pub fn valid_name(name: &str) -> bool {
    crate::engine::skills::valid_name(name)
}

/// A free name made from anything: "My Lisbon Vlog!" -> "my-lisbon-vlog".
pub fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.trim().to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "style".into()
    } else {
        out
    }
}

fn unquote(v: &str) -> &str {
    let v = v.trim();
    for q in ['"', '\''] {
        if v.len() >= 2 && v.starts_with(q) && v.ends_with(q) {
            return &v[1..v.len() - 1];
        }
    }
    v
}

/// `[a, b, c]` or `a, b, c` -> items (quotes trimmed, empties dropped).
fn list(v: &str) -> Vec<String> {
    let v = v.trim();
    let v = v.strip_prefix('[').and_then(|v| v.strip_suffix(']')).unwrap_or(v);
    v.split(',').map(|s| unquote(s).to_string()).filter(|s| !s.is_empty()).collect()
}

/// A number with optional units after it: "1.9", "1.9 s", "-14 LUFS", "60%" (-> 0.6 when `pct`).
fn number(v: &str, pct: bool) -> Option<f64> {
    let v = unquote(v);
    let end = v.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+')).unwrap_or(v.len());
    let n: f64 = v[..end].parse().ok()?;
    let is_pct = v[end..].trim_start().starts_with('%');
    Some(if pct && (is_pct || n > 1.0) { n / 100.0 } else { n }).filter(|n| n.is_finite())
}

/// Parse a style file. Tolerant: a BOM, CRLF, quoted values, units after numbers and unknown keys are
/// fine; a file without front matter is a style named after its file with only a body.
pub fn parse(stem: &str, text: &str, source: Source, path: Option<PathBuf>) -> Style {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text).replace("\r\n", "\n");
    let mut s = Style {
        name: slug(stem),
        title: String::new(),
        description: String::new(),
        rules: Rules::default(),
        body: String::new(),
        skill: String::new(),
        extra: Vec::new(),
        source,
        path,
    };
    let mut body = text.as_str();
    if let Some(rest) = text.trim_start().strip_prefix("---") {
        if let Some((head, after)) = rest.split_once("\n---") {
            for line in head.lines() {
                let Some((k, v)) = line.split_once(':') else { continue };
                let (k, raw) = (k.trim().to_ascii_lowercase(), v.trim());
                let v = unquote(raw);
                let r = &mut s.rules;
                match k.as_str() {
                    "name" if valid_name(v) => s.name = v.to_string(),
                    "title" => s.title = v.to_string(),
                    "description" => s.description = v.to_string(),
                    "skill" => s.skill = v.trim_start_matches('/').to_string(),
                    "avg_shot" | "average_shot" | "asl" => r.avg_shot = number(v, false),
                    "cut_on_beat" => r.cut_on_beat = number(v, true).map(|n| n.clamp(0.0, 1.0)),
                    "transitions" => r.transitions = list(raw),
                    "captions" => r.captions = v.to_string(),
                    "loudness" | "lufs" => r.loudness = number(v, false),
                    "grade" | "lut" | "look" => r.grade = v.to_string(),
                    "pacing" => r.pacing = list(raw).iter().filter_map(|x| number(x, false)).collect(),
                    "words_per_minute" | "wpm" => r.wpm = number(v, false),
                    "" => {}
                    _ => s.extra.push((k, raw.to_string())),
                }
            }
            body = after.split_once('\n').map_or("", |(_, b)| b);
        }
    }
    s.body = body.trim_matches('\n').to_string();
    if s.title.is_empty() {
        s.title = s.name.clone();
    }
    s
}

fn num(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// The file text (front matter + body); `parse(to_text(s))` gives `s` back.
pub fn to_text(s: &Style) -> String {
    let one = |v: &str| v.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut t = format!("---\nname: {}\n", s.name);
    if !s.title.is_empty() && s.title != s.name {
        let _ = writeln!(t, "title: {}", one(&s.title));
    }
    if !s.description.is_empty() {
        let _ = writeln!(t, "description: {}", one(&s.description));
    }
    let r = &s.rules;
    if let Some(v) = r.avg_shot {
        let _ = writeln!(t, "avg_shot: {}", num(v));
    }
    if let Some(v) = r.cut_on_beat {
        let _ = writeln!(t, "cut_on_beat: {}", num(v));
    }
    if !r.transitions.is_empty() {
        let _ = writeln!(t, "transitions: [{}]", r.transitions.iter().map(|x| one(x)).collect::<Vec<_>>().join(", "));
    }
    if !r.captions.is_empty() {
        let _ = writeln!(t, "captions: {}", one(&r.captions));
    }
    if let Some(v) = r.loudness {
        let _ = writeln!(t, "loudness: {}", num(v));
    }
    if !r.grade.is_empty() {
        let _ = writeln!(t, "grade: {}", one(&r.grade));
    }
    if !r.pacing.is_empty() {
        let _ = writeln!(t, "pacing: [{}]", r.pacing.iter().map(|v| num(*v)).collect::<Vec<_>>().join(", "));
    }
    if let Some(v) = r.wpm {
        let _ = writeln!(t, "words_per_minute: {}", num(v));
    }
    if !s.skill.is_empty() {
        let _ = writeln!(t, "skill: {}", s.skill);
    }
    for (k, v) in &s.extra {
        let _ = writeln!(t, "{k}: {v}");
    }
    t.push_str("---\n");
    t.push_str(s.body.replace("\r\n", "\n").trim_matches('\n'));
    t.push('\n');
    t
}

/// The pacing curve in words: "even", "speeds up", "slows down", "fast in the middle", "slow in the middle".
pub fn pacing_words(p: &[f64]) -> &'static str {
    if p.len() < 2 {
        return "";
    }
    let (first, last) = (p[0], p[p.len() - 1]);
    let mid = p[1..p.len() - 1].iter().copied().fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.min(v))));
    let mid_max = p[1..p.len() - 1].iter().copied().fold(f64::MIN, f64::max);
    let lo = p.iter().copied().fold(f64::MAX, f64::min);
    let hi = p.iter().copied().fold(f64::MIN, f64::max);
    if hi <= lo * 1.2 {
        return "even";
    }
    if let Some(m) = mid {
        if m < first.min(last) * 0.85 {
            return "fast in the middle";
        }
        if mid_max > first.max(last) * 1.15 {
            return "slow in the middle";
        }
    }
    if last < first {
        "speeds up toward the end"
    } else {
        "slows down toward the end"
    }
}

/// The Rules lines a style puts into Project Context: one instruction per measurable rule.
pub fn rules_markdown(s: &Style) -> String {
    let r = &s.rules;
    let mut out = String::new();
    if let Some(v) = r.avg_shot {
        let _ = writeln!(out, "- Average shot length about {} s (most shots {}-{} s).", num(v), num(v * 0.6), num(v * 1.6));
    }
    if !r.pacing.is_empty() {
        let p: Vec<String> = r.pacing.iter().map(|v| format!("{} s", num(*v))).collect();
        let _ = writeln!(out, "- Pacing {}: average shot by quarter {}.", pacing_words(&r.pacing), p.join(", "));
    }
    if let Some(v) = r.cut_on_beat {
        if v >= 0.05 {
            let _ = writeln!(out, "- About {:.0}% of cuts land on a beat of the music.", v * 100.0);
        } else {
            let _ = writeln!(out, "- Cuts follow the story, not the music's beat.");
        }
    }
    if !r.transitions.is_empty() {
        let _ = writeln!(out, "- Transitions: {} (seconds after a name are a typical length).", r.transitions.join(", "));
    }
    if !r.captions.is_empty() {
        let _ = writeln!(out, "- Captions: {}.", r.captions.trim_end_matches('.'));
    }
    if let Some(v) = r.loudness {
        let _ = writeln!(out, "- Mix to {} LUFS integrated.", num(v));
    }
    if !r.grade.is_empty() {
        let _ = writeln!(out, "- Grade: {}.", r.grade.trim_end_matches('.'));
    }
    if let Some(v) = r.wpm {
        let _ = writeln!(out, "- Speech runs about {:.0} words per minute; cut pauses to keep that pace.", v);
    }
    out.trim_end().to_string()
}

/// The Style guide text a style puts into Project Context: its title and description, the prose, and the
/// skill that does the whole pass.
pub fn guide_markdown(s: &Style) -> String {
    let mut out = format!("**{}**", s.title);
    if !s.description.is_empty() {
        let _ = write!(out, " - {}", s.description);
    }
    if !s.body.trim().is_empty() {
        let _ = write!(out, "\n\n{}", s.body.trim());
    }
    out
}

// ---------------------------------------------------------------------------------------------------
// the library on disk

/// `%APPDATA%\SimpleEditor\styles`.
pub fn user_dir() -> PathBuf {
    crate::settings::Settings::dir().join("styles")
}

fn md_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("md")))
        .collect();
    v.sort();
    v
}

fn read(path: &Path) -> Option<Style> {
    let text = std::fs::read_to_string(path).ok()?;
    Some(parse(&path.file_stem()?.to_string_lossy(), &text, Source::User, Some(path.to_path_buf())))
}

/// Every style in `user` plus the built-ins (a user file shadows a built-in of its name), by name.
pub fn list_in(user: &Path) -> Vec<Style> {
    let mut out: Vec<Style> = Vec::new();
    let found = md_files(user)
        .iter()
        .filter_map(|p| read(p))
        .chain(BUILT_IN.iter().map(|(stem, text)| parse(stem, text, Source::BuiltIn, None)))
        .collect::<Vec<_>>();
    for s in found {
        if !out.iter().any(|o| o.name == s.name) {
            out.push(s);
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

pub fn get_in(user: &Path, name: &str) -> Option<Style> {
    list_in(user).into_iter().find(|s| s.name == name)
}

/// Write `s` as a user style in `dir` (temp file + rename). Replaces the user's own file of that name.
pub fn save_in(dir: &Path, s: &Style) -> Result<PathBuf, String> {
    if !valid_name(&s.name) {
        return Err(format!("'{}': a style name is lowercase letters, digits and '-' only", s.name));
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join(format!("{}.md", s.name));
    let tmp = dir.join(format!("{}.md.part", s.name));
    std::fs::write(&tmp, to_text(s)).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// A name not taken in `all`: `base`, else `base-2`, `base-3`...
pub fn free_name(all: &[Style], base: &str) -> String {
    let base = slug(base);
    if !all.iter().any(|s| s.name == base) {
        return base;
    }
    (2..).map(|n| format!("{base}-{n}")).find(|n| !all.iter().any(|s| &s.name == n)).unwrap_or(base)
}

/// Copy any style (a built-in too) to a new user style; returns its name.
pub fn duplicate_in(dir: &Path, name: &str) -> Result<String, String> {
    let all = list_in(dir);
    let mut s = all.iter().find(|s| s.name == name).cloned().ok_or(format!("no style named '{name}'"))?;
    s.name = free_name(&all, &format!("{name}-copy"));
    s.title = format!("{} (copy)", s.title);
    save_in(dir, &s)?;
    Ok(s.name)
}

fn user_file(dir: &Path, name: &str) -> Option<PathBuf> {
    md_files(dir).into_iter().find(|p| read(p).is_some_and(|s| s.name == name))
}

/// Rename one of the user's styles (a built-in can't be renamed: duplicate it first).
pub fn rename_in(dir: &Path, old: &str, new: &str) -> Result<(), String> {
    if !valid_name(new) {
        return Err(format!("'{new}': a style name is lowercase letters, digits and '-' only"));
    }
    if old == new {
        return Ok(());
    }
    let all = list_in(dir);
    if all.iter().any(|s| s.name == new && s.source == Source::User) {
        return Err(format!("there is already a style named '{new}'"));
    }
    let file = user_file(dir, old).ok_or_else(|| match all.iter().find(|s| s.name == old) {
        Some(_) => format!("'{old}' is built in and can't be renamed; duplicate it to make your own"),
        None => format!("no style named '{old}'"),
    })?;
    let mut s = read(&file).ok_or("could not read the style")?;
    s.name = new.to_string();
    save_in(dir, &s)?;
    std::fs::remove_file(&file).map_err(|e| format!("{}: {e}", file.display()))
}

/// Delete one of the user's styles (a built-in that it shadowed shows again).
pub fn delete_in(dir: &Path, name: &str) -> Result<(), String> {
    match user_file(dir, name) {
        Some(p) => std::fs::remove_file(&p).map_err(|e| format!("{}: {e}", p.display())),
        None if BUILT_IN.iter().any(|(n, _)| *n == name) => {
            Err(format!("'{name}' is built in and can't be deleted; duplicate it to make your own"))
        }
        None => Err(format!("no style named '{name}'")),
    }
}

/// Import a `.md` file as a user style (a free name when it would shadow another of the user's styles).
pub fn import_in(dir: &Path, file: &Path) -> Result<String, String> {
    let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let stem = file.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let mut s = parse(&stem, &text, Source::User, None);
    if s.body.trim().is_empty() && s.rules == Rules::default() {
        return Err(format!("{} has no style in it (no rules and no text)", file.display()));
    }
    let mine: Vec<Style> = list_in(dir).into_iter().filter(|x| x.source == Source::User).collect();
    s.name = free_name(&mine, &s.name);
    save_in(dir, &s)?;
    Ok(s.name)
}

/// The whole file text of a style (also what Export writes).
pub fn file_text(s: &Style) -> String {
    match &s.path {
        Some(p) => std::fs::read_to_string(p).unwrap_or_else(|_| to_text(s)),
        None => BUILT_IN.iter().find(|(n, _)| *n == s.name).map_or_else(|| to_text(s), |(_, t)| t.to_string()),
    }
}

const RECHECK: Duration = Duration::from_secs(2);
static CACHE: Mutex<Option<(Instant, Option<SystemTime>, Arc<Vec<Style>>)>> = Mutex::new(None);

/// Every style (cached; re-read when the folder's time changes, checked at most every 2 s).
pub fn all() -> Arc<Vec<Style>> {
    let stamp = || std::fs::metadata(user_dir()).and_then(|m| m.modified()).ok();
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((checked, st, list)) = c.as_mut() {
        if checked.elapsed() < RECHECK {
            return list.clone();
        }
        *checked = Instant::now();
        if stamp() == *st {
            return list.clone();
        }
    }
    let list = Arc::new(list_in(&user_dir()));
    *c = Some((Instant::now(), stamp(), list.clone()));
    list
}

pub fn invalidate() {
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

pub fn get(name: &str) -> Option<Style> {
    all().iter().find(|s| s.name == name).cloned()
}

fn done<T>(r: Result<T, String>) -> Result<T, String> {
    invalidate();
    r
}

pub fn save(s: &Style) -> Result<PathBuf, String> {
    done(save_in(&user_dir(), s))
}
pub fn duplicate(name: &str) -> Result<String, String> {
    done(duplicate_in(&user_dir(), name))
}
pub fn rename(old: &str, new: &str) -> Result<(), String> {
    done(rename_in(&user_dir(), old, new))
}
pub fn delete(name: &str) -> Result<(), String> {
    done(delete_in(&user_dir(), name))
}
pub fn import(file: &Path) -> Result<String, String> {
    done(import_in(&user_dir(), file))
}

/// Styles whose name or title matches what follows `/style `: name prefix first; at most `max`.
pub fn filter<'a>(all: &'a [Style], query: &str, max: usize) -> Vec<&'a Style> {
    let q = query.trim().to_lowercase();
    let mut out: Vec<&Style> = all.iter().filter(|s| s.name.starts_with(&q)).collect();
    out.extend(all.iter().filter(|s| !s.name.starts_with(&q) && (s.name.contains(&q) || s.title.to_lowercase().contains(&q))));
    out.truncate(max);
    out
}

/// `/style vlog` -> Some("vlog"); `/style` alone -> Some(""); anything else (`/style-vlog`, a skill) -> None.
pub fn parse_command(text: &str) -> Option<&str> {
    let rest = text.trim_start().strip_prefix("/style")?;
    if rest.is_empty() {
        return Some("");
    }
    rest.starts_with(char::is_whitespace).then(|| rest.trim())
}

/// The words that take the applied style out again (`/style off`).
pub fn is_off(arg: &str) -> bool {
    matches!(arg.trim().to_ascii_lowercase().as_str(), "off" | "none" | "remove" | "clear")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let base = std::env::var_os("SE_TEST_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let d = base.join(format!("se-styles-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn front_matter_parses_tolerantly() {
        let s = parse(
            "x",
            "\u{feff}---\r\nname: lisbon\r\ntitle: \"Lisbon vlog\"\r\ndescription: Warm and fast.\r\navg_shot: 1.9 s\r\n\
             cut_on_beat: 40%\r\ntransitions: [cut, \"dissolve 0.5\", whip 0.2]\r\ncaptions: Bold Outline, highlight\r\n\
             loudness: -14 LUFS\r\ngrade: luts/warm-film at 0.5\r\npacing: [2.4, 1.8 , 1.2,0.9]\r\nwpm: 172\r\n\
             mood: sunny\r\n---\r\nKeep it moving.\r\n",
            Source::User,
            None,
        );
        assert_eq!((s.name.as_str(), s.title.as_str(), s.description.as_str()), ("lisbon", "Lisbon vlog", "Warm and fast."));
        let r = &s.rules;
        assert_eq!(r.avg_shot, Some(1.9));
        assert_eq!(r.cut_on_beat, Some(0.4));
        assert_eq!(r.transitions, ["cut", "dissolve 0.5", "whip 0.2"]);
        assert_eq!(r.captions, "Bold Outline, highlight");
        assert_eq!(r.loudness, Some(-14.0));
        assert_eq!(r.grade, "luts/warm-film at 0.5");
        assert_eq!(r.pacing, [2.4, 1.8, 1.2, 0.9]);
        assert_eq!(r.wpm, Some(172.0));
        assert_eq!(s.extra, [("mood".to_string(), "sunny".to_string())]);
        assert_eq!(s.body, "Keep it moving.");
        // the text round-trips, unknown keys included
        let again = parse("y", &to_text(&s), Source::User, None);
        assert_eq!(again, s);
        // a fraction is kept, a percentage without % above 1 is read as one
        assert_eq!(parse("a", "---\ncut_on_beat: 0.25\n---\n", Source::User, None).rules.cut_on_beat, Some(0.25));
        assert_eq!(parse("a", "---\ncut_on_beat: 60\n---\n", Source::User, None).rules.cut_on_beat, Some(0.6));
        // no front matter: named after the file, the whole text is the body
        let bare = parse("My Style", "Just prose.", Source::User, None);
        assert_eq!((bare.name.as_str(), bare.body.as_str(), bare.title.as_str()), ("my-style", "Just prose.", "my-style"));
        assert_eq!(bare.rules, Rules::default());
        // garbage numbers are skipped, not fatal
        assert_eq!(parse("a", "---\navg_shot: fast\n---\n", Source::User, None).rules.avg_shot, None);
    }

    #[test]
    fn built_ins_parse_with_rules() {
        assert_eq!(BUILT_IN.len(), 6);
        for (stem, text) in BUILT_IN {
            let s = parse(stem, text, Source::BuiltIn, None);
            assert_eq!(s.name, *stem);
            assert!(s.rules.avg_shot.is_some() && !s.rules.transitions.is_empty() && s.rules.loudness.is_some(), "{stem}");
            assert_eq!(s.rules.pacing.len(), 4, "{stem}");
            assert!(!s.body.is_empty() && !s.description.is_empty(), "{stem}");
            assert!(crate::engine::skills::BUILT_IN.iter().all(|(n, _)| *n != s.name), "no clash with a skill name");
            let skill = Path::new(env!("CARGO_MANIFEST_DIR")).join("library").join("skills").join(format!("{}.md", s.skill));
            assert!(skill.is_file(), "{stem}: skill {} exists", s.skill);
        }
    }

    #[test]
    fn rules_and_guide_text() {
        let s = parse("vlog", BUILT_IN.iter().find(|(n, _)| *n == "vlog").unwrap().1, Source::BuiltIn, None);
        let r = rules_markdown(&s);
        assert!(r.starts_with("- Average shot length about 1.9 s"), "{r}");
        assert!(r.contains("- Mix to -14 LUFS integrated.") && r.contains("- Captions: Bold Outline"), "{r}");
        assert!(r.lines().all(|l| l.starts_with("- ")), "{r}");
        let g = guide_markdown(&s);
        assert!(g.starts_with("**Fast-cut vlog** - Talking-head") && g.contains("/style-vlog"), "{g}");
        assert_eq!(pacing_words(&[4.0, 2.5, 1.0, 0.8]), "speeds up toward the end");
        assert_eq!(pacing_words(&[2.0, 2.1, 1.9, 2.0]), "even");
        assert_eq!(pacing_words(&[1.6, 1.0, 0.8, 1.2]), "fast in the middle");
    }

    #[test]
    fn library_save_duplicate_rename_import_delete() {
        let dir = temp("lib");
        let all = list_in(&dir);
        assert_eq!(all.iter().filter(|s| s.source == Source::BuiltIn).count(), 6);
        // a built-in can't be changed in place
        assert!(delete_in(&dir, "vlog").unwrap_err().contains("built in"));
        assert!(rename_in(&dir, "vlog", "mine").unwrap_err().contains("built in"));
        // duplicate -> a user copy with a free name
        let copy = duplicate_in(&dir, "vlog").unwrap();
        assert_eq!(copy, "vlog-copy");
        assert_eq!(duplicate_in(&dir, "vlog").unwrap(), "vlog-copy-2");
        let c = get_in(&dir, "vlog-copy").unwrap();
        assert_eq!((c.source, c.rules.avg_shot), (Source::User, Some(1.9)));
        // rename keeps the content, refuses a taken or bad name
        assert!(rename_in(&dir, "vlog-copy", "Bad Name").is_err());
        assert!(rename_in(&dir, "vlog-copy", "vlog-copy-2").unwrap_err().contains("already"));
        rename_in(&dir, "vlog-copy", "gym-vlog").unwrap();
        assert!(get_in(&dir, "vlog-copy").is_none());
        assert_eq!(get_in(&dir, "gym-vlog").unwrap().rules.avg_shot, Some(1.9));
        assert!(!dir.join("gym-vlog.md.part").exists());
        // a user file named like a built-in shadows it; deleting it brings the built-in back
        let mut mine = get_in(&dir, "trailer").unwrap();
        mine.rules.avg_shot = Some(3.3);
        save_in(&dir, &mine).unwrap();
        assert_eq!(get_in(&dir, "trailer").unwrap().source, Source::User);
        assert_eq!(list_in(&dir).iter().filter(|s| s.name == "trailer").count(), 1);
        delete_in(&dir, "trailer").unwrap();
        assert_eq!(get_in(&dir, "trailer").unwrap().source, Source::BuiltIn);
        // import / export
        let ext = dir.join("outside");
        std::fs::create_dir_all(&ext).unwrap();
        let f = ext.join("Gym Vlog.md");
        std::fs::write(&f, file_text(&get_in(&dir, "gym-vlog").unwrap())).unwrap();
        let got = import_in(&dir, &f).unwrap();
        assert_eq!(got, "gym-vlog-2", "the name was taken");
        std::fs::write(ext.join("empty.md"), "---\nname: e\n---\n").unwrap();
        assert!(import_in(&dir, &ext.join("empty.md")).is_err());
        assert!(delete_in(&dir, "nope").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn style_command() {
        assert_eq!(parse_command("/style vlog"), Some("vlog"));
        assert_eq!(parse_command("  /style   trailer "), Some("trailer"));
        assert_eq!(parse_command("/style"), Some(""));
        assert_eq!(parse_command("/style-vlog chill"), None, "a skill");
        assert_eq!(parse_command("/styles"), None);
        assert_eq!(parse_command("style vlog"), None);
        assert!(is_off("off") && is_off(" None ") && !is_off("vlog"));
        let all = list_in(&temp("filter"));
        let names: Vec<&str> = filter(&all, "t", 9).iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names[0], "trailer");
        assert!(names.contains(&"shorts"), "title / name containing it");
        assert_eq!(slug("My Lisbon Vlog!"), "my-lisbon-vlog");
        assert_eq!(slug("!!!"), "style");
    }
}
