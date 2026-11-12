//! ---- ws:skills ----
//! Assistant skills: one Markdown file each (`library/README.md`, "Skills"), run from chat as
//! `/name arguments`. Three sources, earlier ones shadowing later ones of the same name: the user's
//! `%APPDATA%\SimpleEditor\skills\*.md`, installed library skills, and a few built-ins compiled in.
//! The listing is cached and only re-read when a folder's modified time changes (checked at most every
//! `RECHECK`) or after `save` / `delete`: never a disk scan per frame.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    User,
    Library,
    BuiltIn,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::User => "Yours",
            Source::Library => "Library",
            Source::BuiltIn => "Built-in",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    /// The hint shown after the name ("[style]"); empty when the skill takes no arguments.
    pub args: String,
    pub body: String,
    pub source: Source,
    /// The file it was read from (None for a built-in).
    pub path: Option<PathBuf>,
}

/// The skills compiled into the exe (the rest of `library/skills/` comes from the online library).
pub const BUILT_IN: &[(&str, &str)] = &[
    ("captions", include_str!("../../library/skills/captions.md")),
    ("cut-silences", include_str!("../../library/skills/cut-silences.md")),
    ("shorts", include_str!("../../library/skills/shorts.md")),
    ("lower-third", include_str!("../../library/skills/lower-third.md")),
    ("title-card", include_str!("../../library/skills/title-card.md")),
    ("create-skill", include_str!("../../library/skills/create-skill.md")),
];

/// The skill that writes skills (`/create-skill`, the New skill window's "Write it for me").
pub const CREATE: &str = "create-skill";

/// Lowercase letters, digits and `-`, not empty.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
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

/// Parse a skill file. Tolerant: a BOM, CRLF, quoted values and unknown keys are fine; a file without
/// a `---` header is a skill named after its file with an empty description.
pub fn parse(stem: &str, text: &str, source: Source, path: Option<PathBuf>) -> Skill {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text).replace("\r\n", "\n");
    let mut s = Skill {
        name: stem.to_lowercase(),
        description: String::new(),
        args: String::new(),
        body: String::new(),
        source,
        path,
    };
    let mut body = text.as_str();
    if let Some(rest) = text.trim_start().strip_prefix("---") {
        if let Some((head, after)) = rest.split_once("\n---") {
            for line in head.lines() {
                let Some((k, v)) = line.split_once(':') else { continue };
                let v = unquote(v);
                match k.trim() {
                    "name" if valid_name(v) => s.name = v.to_string(),
                    "description" => s.description = v.to_string(),
                    "args" => s.args = v.to_string(),
                    _ => {}
                }
            }
            // the rest of the closing `---` line, then the body
            body = after.split_once('\n').map_or("", |(_, b)| b);
        }
    }
    s.body = body.trim_matches('\n').to_string();
    s
}

/// The request a skill run sends: `$ARGS` replaced everywhere, or the arguments appended as a last
/// line when the body has no `$ARGS`.
pub fn expand(skill: &Skill, args: &str) -> String {
    let args = args.trim();
    if skill.body.contains("$ARGS") {
        skill.body.replace("$ARGS", args)
    } else if args.is_empty() {
        skill.body.clone()
    } else {
        format!("{}\n{args}", skill.body.trim_end())
    }
}

/// `"/captions bold yellow"` -> `("captions", "bold yellow")`. None when the text does not start with
/// `/` followed by a valid name (a path such as `/x/y` is plain text).
pub fn parse_invocation(text: &str) -> Option<(&str, &str)> {
    let rest = text.trim_start().strip_prefix('/')?;
    let (name, args) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    valid_name(name).then(|| (name, args.trim()))
}

/// The file text `save` writes.
pub fn to_text(name: &str, description: &str, args: &str, body: &str) -> String {
    let one_line = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut t = format!("---\nname: {name}\ndescription: {}\n", one_line(description));
    let args = one_line(args);
    if !args.is_empty() {
        t += &format!("args: \"{args}\"\n");
    }
    t += "---\n";
    t += body.replace("\r\n", "\n").trim_matches('\n');
    t.push('\n');
    t
}

/// A stable hash of what a skill would make the assistant do (the Library first-run check).
pub fn content_hash(s: &Skill) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in [&s.description, &s.args, &s.body].iter().flat_map(|t| t.bytes().chain([0])) {
        h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// `%APPDATA%\SimpleEditor\skills`.
pub fn user_dir() -> PathBuf {
    crate::settings::Settings::dir().join("skills")
}

fn library_dir() -> PathBuf {
    use crate::engine::library::{root, Kind};
    root().join(Kind::Skill.dir())
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

fn read(path: &Path, source: Source) -> Option<Skill> {
    let text = std::fs::read_to_string(path).ok()?;
    Some(parse(&path.file_stem()?.to_string_lossy(), &text, source, Some(path.to_path_buf())))
}

/// Every skill, by name: the user's folder, then `library` (installed library files), then the
/// built-ins; a later source never shadows an earlier one with the same name.
pub fn list_in(user: &Path, library: &[PathBuf]) -> Vec<Skill> {
    let mut out: Vec<Skill> = Vec::new();
    let user_files = md_files(user);
    let found = user_files
        .iter()
        .filter_map(|p| read(p, Source::User))
        .chain(library.iter().filter_map(|p| read(p, Source::Library)))
        .chain(BUILT_IN.iter().map(|(stem, text)| parse(stem, text, Source::BuiltIn, None)));
    for s in found {
        if !out.iter().any(|o| o.name == s.name) {
            out.push(s);
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

const RECHECK: Duration = Duration::from_secs(2);

struct Cache {
    checked: Instant,
    stamp: [Option<SystemTime>; 2],
    skills: Arc<Vec<Skill>>,
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

fn stamp() -> [Option<SystemTime>; 2] {
    let m = |p: PathBuf| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    [m(user_dir()), m(library_dir())]
}

/// Every skill (cached). ponytail: a folder's modified time changes when a file is added, removed or
/// replaced by a save-and-rename, not always when one is edited in place; `invalidate` or a restart
/// picks that up - watch each file's time if that bites.
pub fn all() -> Arc<Vec<Skill>> {
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(c) = c.as_mut() {
        if c.checked.elapsed() < RECHECK {
            return c.skills.clone();
        }
        c.checked = Instant::now();
        if stamp() == c.stamp {
            return c.skills.clone();
        }
    }
    let lib: Vec<PathBuf> =
        crate::engine::library::installed(crate::engine::library::Kind::Skill).into_iter().map(|i| i.path).collect();
    let skills = Arc::new(list_in(&user_dir(), &lib));
    *c = Some(Cache { checked: Instant::now(), stamp: stamp(), skills: skills.clone() });
    skills
}

/// Forget the cached listing (after a save / delete, or a migration on another thread).
pub fn invalidate() {
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

pub fn get(name: &str) -> Option<Skill> {
    all().iter().find(|s| s.name == name).cloned()
}

/// The whole file text of a skill (the embedded text for a built-in).
pub fn file_text(s: &Skill) -> String {
    match &s.path {
        Some(p) => std::fs::read_to_string(p).unwrap_or_default(),
        None => BUILT_IN.iter().find(|(n, _)| *n == s.name).map_or_else(String::new, |(_, t)| t.to_string()),
    }
}

/// Write a user skill into `dir` (temp file + rename). The name must be lowercase letters, digits, `-`.
pub fn save_in(dir: &Path, name: &str, description: &str, args: &str, body: &str) -> Result<PathBuf, String> {
    if !valid_name(name) {
        return Err(format!("'{name}': a skill name is lowercase letters, digits and '-' only"));
    }
    if description.trim().is_empty() {
        return Err("a skill needs a one-line description".into());
    }
    if body.trim().is_empty() {
        return Err("a skill needs instructions (the body)".into());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join(format!("{name}.md"));
    let tmp = dir.join(format!("{name}.md.part"));
    std::fs::write(&tmp, to_text(name, description, args, body)).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

pub fn save(name: &str, description: &str, args: &str, body: &str) -> Result<PathBuf, String> {
    let r = save_in(&user_dir(), name, description, args, body);
    invalidate();
    r
}

/// Delete a user skill from `dir`. A built-in or library skill cannot be deleted (`others` says which
/// skills exist beyond the folder, to tell the user why).
pub fn delete_in(dir: &Path, name: &str, others: &[Skill]) -> Result<(), String> {
    let mine = md_files(dir).into_iter().find(|p| read(p, Source::User).is_some_and(|s| s.name == name));
    match (mine, others.iter().find(|s| s.name == name)) {
        (Some(p), _) => std::fs::remove_file(&p).map_err(|e| format!("{}: {e}", p.display())),
        (None, Some(s)) => Err(format!(
            "'{name}' is a {} skill and can't be deleted; duplicate it to make your own version",
            s.source.label().to_lowercase()
        )),
        (None, None) => Err(format!("no skill named '{name}'")),
    }
}

pub fn delete(name: &str) -> Result<(), String> {
    let r = delete_in(&user_dir(), name, &all());
    invalidate();
    r
}

/// Quick prompts become user skills: each (label, prompt) whose slugged name is not taken and has no
/// file yet is written once. Returns how many were written.
pub fn migrate_in(dir: &Path, quick: &[(String, String)], taken: &[String]) -> usize {
    quick
        .iter()
        .filter(|(label, prompt)| {
            let name = crate::engine::library::slug(label);
            valid_name(&name)
                && !prompt.trim().is_empty()
                && !taken.contains(&name)
                && !dir.join(format!("{name}.md")).exists()
                && save_in(dir, &name, label, "", prompt).is_ok()
        })
        .count()
}

/// Skills whose name matches what follows the `/` being typed: name prefix first, then the name
/// containing it, then the description; at most `max`.
pub fn filter<'a>(skills: &'a [Skill], query: &str, max: usize) -> Vec<&'a Skill> {
    let q = query.to_lowercase();
    let mut out: Vec<&Skill> = skills.iter().filter(|s| s.name.starts_with(&q)).collect();
    out.extend(skills.iter().filter(|s| !s.name.starts_with(&q) && s.name.contains(&q)));
    if !q.is_empty() {
        out.extend(skills.iter().filter(|s| !s.name.contains(&q) && s.description.to_lowercase().contains(&q)));
    }
    out.truncate(max);
    out
}

/// Open the user's skills folder in Explorer (created first).
pub fn open_folder() {
    let dir = user_dir();
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::process::Command::new("explorer").arg(dir).spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("se-skills-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn parses_header_tolerantly() {
        let s = parse(
            "x",
            "\u{feff}---\r\nname: captions\r\ndescription: Add captions.\r\nargs: \"[style]\"\r\nmood: ignored\r\n---\r\nDo it.\r\n\r\nStyle: $ARGS\r\n",
            Source::User,
            None,
        );
        assert_eq!(
            (s.name.as_str(), s.description.as_str(), s.args.as_str()),
            ("captions", "Add captions.", "[style]")
        );
        assert_eq!(s.body, "Do it.\n\nStyle: $ARGS");
        let bare = parse("Tighten", "Just do this.", Source::User, None);
        assert_eq!(
            (bare.name.as_str(), bare.description.as_str(), bare.body.as_str()),
            ("tighten", "", "Just do this.")
        );
        let single = parse("q", "---\nname: 'quoted'\ndescription: has: colons\n---\nbody", Source::User, None);
        assert_eq!((single.name.as_str(), single.description.as_str()), ("quoted", "has: colons"));
    }

    #[test]
    fn expands_args() {
        let mut s = parse("c", "---\nname: c\ndescription: d\n---\nA $ARGS B $ARGS", Source::User, None);
        assert_eq!(expand(&s, " bold "), "A bold B bold");
        assert_eq!(expand(&s, ""), "A  B ");
        s.body = "No placeholder.\n".into();
        assert_eq!(expand(&s, "yellow"), "No placeholder.\nyellow");
        assert_eq!(expand(&s, "  "), "No placeholder.\n");
    }

    #[test]
    fn invocations() {
        assert_eq!(parse_invocation("/captions bold yellow"), Some(("captions", "bold yellow")));
        assert_eq!(parse_invocation("  /shorts"), Some(("shorts", "")));
        assert_eq!(parse_invocation("/cut-silences\n  keep 0.3s "), Some(("cut-silences", "keep 0.3s")));
        assert_eq!(parse_invocation("/Path/to/file"), None);
        assert_eq!(parse_invocation("/"), None);
        assert_eq!(parse_invocation("captions"), None);
    }

    /// Every library skill parses, its header name equals its file name, it has a description; the
    /// built-ins are exactly the six embedded ones.
    #[test]
    fn every_library_skill_parses() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("library").join("skills");
        let files = md_files(&dir);
        assert!(files.len() >= 6);
        for p in &files {
            let stem = p.file_stem().unwrap().to_string_lossy().to_string();
            let s = read(p, Source::Library).unwrap();
            assert_eq!(s.name, stem, "{}", p.display());
            assert!(!s.description.is_empty() && !s.body.is_empty(), "{stem}");
        }
        assert_eq!(BUILT_IN.len(), 6);
        for (stem, text) in BUILT_IN {
            assert_eq!(parse(stem, text, Source::BuiltIn, None).name, *stem);
        }
    }

    /// User skills shadow library ones, which shadow built-ins; save / get / delete round trip; a
    /// built-in or library skill cannot be deleted; a saved file reads back the same.
    #[test]
    fn sources_save_and_delete() {
        let user = temp("user");
        let lib = temp("lib");
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::write(lib.join("captions.md"), "---\nname: captions\ndescription: lib version\n---\nlib").unwrap();
        std::fs::write(lib.join("extra.md"), "---\nname: extra\ndescription: e\n---\nx").unwrap();
        let libs = md_files(&lib);
        let all = list_in(&user, &libs);
        let cap = all.iter().find(|s| s.name == "captions").unwrap();
        assert_eq!((cap.source, cap.description.as_str()), (Source::Library, "lib version"));
        assert_eq!(all.iter().find(|s| s.name == "shorts").unwrap().source, Source::BuiltIn);
        assert!(save_in(&user, "Bad Name", "d", "", "b").is_err());
        assert!(save_in(&user, "ok", "", "", "b").is_err());
        save_in(&user, "captions", "mine", "[style]", "My $ARGS\r\n").unwrap();
        save_in(&user, "vlog-intro", "Make an intro.", "", "Steps.").unwrap();
        let all = list_in(&user, &libs);
        let cap = all.iter().find(|s| s.name == "captions").unwrap();
        assert_eq!((cap.source, cap.args.as_str(), cap.body.as_str()), (Source::User, "[style]", "My $ARGS"));
        assert_eq!(all.iter().filter(|s| s.name == "captions").count(), 1, "one per name");
        assert_eq!(file_text(cap), "---\nname: captions\ndescription: mine\nargs: \"[style]\"\n---\nMy $ARGS\n");
        assert!(!user.join("captions.md.part").exists());
        assert!(delete_in(&user, "shorts", &all).unwrap_err().contains("built-in"));
        assert!(delete_in(&user, "extra", &all).unwrap_err().contains("library"));
        assert!(delete_in(&user, "nope", &all).is_err());
        delete_in(&user, "captions", &all).unwrap();
        let all = list_in(&user, &libs);
        assert_eq!(
            all.iter().find(|s| s.name == "captions").unwrap().source,
            Source::Library,
            "the library one shows again"
        );
        assert!(file_text(all.iter().find(|s| s.name == "create-skill").unwrap()).contains("skills.save"));
        let _ = std::fs::remove_dir_all(&user);
        let _ = std::fs::remove_dir_all(&lib);
    }

    /// Quick prompts are written once as user skills (not twice, not over a taken name).
    #[test]
    fn quick_prompts_migrate_once() {
        let dir = temp("migrate");
        let quick = vec![
            ("Remove silences".to_string(), "Remove the silent parts.".to_string()),
            ("Captions".to_string(), "taken by a built-in".to_string()),
            ("Empty".to_string(), "  ".to_string()),
        ];
        let taken = vec!["captions".to_string()];
        assert_eq!(migrate_in(&dir, &quick, &taken), 1);
        assert_eq!(migrate_in(&dir, &quick, &taken), 0, "a second run writes nothing");
        let s = list_in(&dir, &[]).into_iter().find(|s| s.name == "remove-silences").unwrap();
        assert_eq!(
            (s.description.as_str(), s.body.as_str(), s.source),
            ("Remove silences", "Remove the silent parts.", Source::User)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn filter_orders_prefix_first() {
        let mk = |n: &str, d: &str| Skill {
            name: n.into(),
            description: d.into(),
            args: String::new(),
            body: String::new(),
            source: Source::BuiltIn,
            path: None,
        };
        let all = [mk("b-roll", "cover cuts"), mk("captions", ""), mk("cut-silences", ""), mk("shorts", "make clips")];
        let names = |v: Vec<&Skill>| v.into_iter().map(|s| s.name.clone()).collect::<Vec<_>>();
        // name prefix, then the descriptions that mention it
        assert_eq!(names(filter(&all, "c", 9)), ["captions", "cut-silences", "b-roll", "shorts"].map(String::from));
        assert_eq!(names(filter(&all, "", 2)).len(), 2);
        assert_eq!(names(filter(&all, "ROLL", 9)), ["b-roll"]);
        let a = mk("x", "same");
        let mut b = a.clone();
        b.source = Source::User;
        assert_eq!(content_hash(&a), content_hash(&b));
        b.body = "changed".into();
        assert_ne!(content_hash(&a), content_hash(&b));
    }
}
