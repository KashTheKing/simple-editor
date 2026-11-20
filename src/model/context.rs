//! ---- feat/styles-context ----
//! Project Context: one Markdown document per project (`Project.context`) with four sections - About,
//! Rules, Style guide, Links - that the Assistant reads at the start of every request (Rules as
//! instructions). Plus the digest: a read-only summary gathered from the planner, the notes, the
//! moodboards and the open timeline comments, so the agent sees what the project is about without
//! being told twice.
//!
//! An applied editing style (`engine::styles`, `/style <name>`) lives inside the document as two blocks
//! fenced by HTML comments, one at the top of Style guide and one at the top of Rules:
//! `<!-- style: vlog -->` ... `<!-- /style -->`. It is plain text the user can read and edit like the
//! rest; removing the style removes exactly those blocks. Everything here is pure string work.

use super::{PlanItem, Project};
use std::fmt::Write;

/// The sections, in order. Headings are `## <name>`, matched without regard to case.
pub const SECTIONS: [&str; 4] = ["About", "Rules", "Style guide", "Links"];
const STYLE_OPEN: &str = "<!-- style:";
const STYLE_CLOSE: &str = "<!-- /style -->";
/// The digest stops growing past this many characters (a huge planner should not crowd out the request).
pub const DIGEST_MAX: usize = 12_000;

/// A new project's document: the four headings, empty.
pub fn template() -> String {
    SECTIONS.iter().map(|s| format!("## {s}\n\n")).collect::<String>().trim_end().to_string() + "\n"
}

/// Nothing written: only headings and blank lines (an untouched template counts as empty).
pub fn is_blank(doc: &str) -> bool {
    doc.lines().all(|l| l.trim().is_empty() || heading(l).is_some())
}

/// `## Name` -> Some("Name") (level-2 headings only).
fn heading(line: &str) -> Option<&str> {
    let t = line.trim_end();
    let rest = t.strip_prefix("## ")?;
    (!rest.starts_with('#')).then_some(rest.trim())
}

/// Byte range of a section's body (after its heading line, up to the next `## ` heading), if present.
fn body_range(doc: &str, name: &str) -> Option<(usize, usize)> {
    let mut pos = 0;
    let mut start = None;
    for line in doc.split_inclusive('\n') {
        if let Some(h) = heading(line) {
            if let Some(s) = start {
                return Some((s, pos));
            }
            if h.eq_ignore_ascii_case(name) {
                start = Some(pos + line.len());
            }
        }
        pos += line.len();
    }
    start.map(|s| (s, doc.len()))
}

/// One section's text, trimmed ("" when missing).
pub fn section(doc: &str, name: &str) -> String {
    body_range(doc, name).map_or_else(String::new, |(a, b)| doc[a..b].trim().to_string())
}

/// `doc` with one section's body replaced (the section is added at the end when missing).
pub fn set_section(doc: &str, name: &str, body: &str) -> String {
    let body = body.trim();
    let block = if body.is_empty() { "\n".to_string() } else { format!("\n{body}\n\n") };
    match body_range(doc, name) {
        Some((a, b)) => {
            let tail = &doc[b..];
            let block = if tail.is_empty() { block.trim_end().to_string() + "\n" } else { block };
            format!("{}{block}{tail}", &doc[..a])
        }
        None => {
            let mut d = doc.trim_end().to_string();
            if !d.is_empty() {
                d.push_str("\n\n");
            }
            format!("{d}## {name}\n{}", block.trim_end().to_string() + "\n")
        }
    }
}

/// The applied style's name, if the document carries one.
pub fn applied_style(doc: &str) -> Option<String> {
    let i = doc.find(STYLE_OPEN)?;
    let rest = &doc[i + STYLE_OPEN.len()..];
    let name = rest[..rest.find("-->")?].trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// `doc` without any style block (and the blank line each leaves behind).
pub fn remove_style(doc: &str) -> String {
    let mut out = doc.to_string();
    while let Some(a) = out.find(STYLE_OPEN) {
        let Some(rel) = out[a..].find(STYLE_CLOSE) else { break };
        let mut b = a + rel + STYLE_CLOSE.len();
        // eat the block's own line break and one blank line after it
        for _ in 0..2 {
            if out[b..].starts_with('\n') {
                b += 1;
            }
        }
        out.replace_range(a..b, "");
    }
    out
}

fn block(name: &str, body: &str) -> String {
    format!("{STYLE_OPEN} {name} -->\n{}\n{STYLE_CLOSE}", body.trim())
}

/// `doc` with `name`'s style in it: any earlier style removed, then `guide` at the top of Style guide and
/// `rules` at the top of Rules (each section keeps what the user wrote under it). A missing section is added.
pub fn apply_style(doc: &str, name: &str, guide: &str, rules: &str) -> String {
    let base = if doc.trim().is_empty() { template() } else { remove_style(doc) };
    let mut d = base;
    for (sec, text) in [("Style guide", guide), ("Rules", rules)] {
        if text.trim().is_empty() {
            continue;
        }
        let own = section(&d, sec);
        let joined = if own.is_empty() { block(name, text) } else { format!("{}\n\n{own}", block(name, text)) };
        d = set_section(&d, sec, &joined);
    }
    d
}

// ---------------------------------------------------------------------------------------------------
// digest

fn asset_name(p: &Project, id: super::Id) -> String {
    if let Some(n) = p.asset_names.get(&id) {
        return n.clone();
    }
    p.asset(id)
        .map(|a| std::path::Path::new(&a.path).file_name().map_or_else(|| a.path.clone(), |f| f.to_string_lossy().to_string()))
        .unwrap_or_else(|| format!("asset {id}"))
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn plan_lines(p: &Project, items: &[PlanItem], depth: usize, out: &mut String, refs: &mut Vec<String>) {
    for it in items {
        let pad = "  ".repeat(depth);
        let _ = write!(out, "{pad}- [{}] {}", if it.done { 'x' } else { ' ' }, one_line(&it.title));
        if !it.notes.trim().is_empty() {
            let _ = write!(out, ": {}", one_line(&it.notes));
        }
        out.push('\n');
        for (label, done) in &it.requirements {
            let _ = writeln!(out, "{pad}  - [{}] {}", if *done { 'x' } else { ' ' }, one_line(label));
        }
        for (i, a) in it.assets.iter().enumerate() {
            let note = it.asset_notes.get(i).map(|s| one_line(s)).unwrap_or_default();
            let name = asset_name(p, *a);
            refs.push(if note.is_empty() { format!("{name} (for \"{}\")", one_line(&it.title)) } else { format!("{name}: {note}") });
        }
        plan_lines(p, &it.children, depth + 1, out, refs);
    }
}

fn mmss(t: f64) -> String {
    let t = t.max(0.0);
    format!("{}:{:04.1}", (t / 60.0) as u64, t % 60.0)
}

/// The read-only digest: planner items (with their checklists and notes), the notes, moodboard captions
/// (the standalone moodboard's tags and the planner items' picture notes) and the open timeline comments
/// (markers with a note, not ticked off). Empty sections are left out; "" when there is nothing at all.
pub fn digest(p: &Project) -> String {
    let mut s = String::new();
    let mut refs = Vec::new();
    let mut plan = String::new();
    plan_lines(p, &p.plan, 0, &mut plan, &mut refs);
    if !plan.is_empty() {
        let _ = write!(s, "### Planner\n{plan}\n");
    }
    let notes: Vec<_> = p.notes.iter().filter(|n| !n.body.trim().is_empty() || !n.title.trim().is_empty()).collect();
    if !notes.is_empty() {
        s.push_str("### Notes\n");
        for n in notes {
            let title = if n.title.trim().is_empty() { "Note" } else { n.title.trim() };
            let _ = write!(s, "#### {title}\n{}\n\n", n.body.trim());
        }
    }
    for m in &p.moodboard {
        let name = asset_name(p, m.asset);
        refs.insert(0, if m.labels.is_empty() { name } else { format!("{name}: {}", m.labels.join(", ")) });
    }
    if !refs.is_empty() {
        s.push_str("### Moodboard\n");
        for r in &refs {
            let _ = writeln!(s, "- {r}");
        }
        s.push('\n');
    }
    let mut comments: Vec<(f64, String)> = p
        .markers
        .iter()
        .filter(|m| !m.done && !m.note.trim().is_empty())
        .map(|m| (m.t, one_line(&m.note)))
        .collect();
    for (_, c) in p.all_clips() {
        comments.extend(
            c.markers.iter().filter(|m| !m.done && !m.note.trim().is_empty()).map(|m| (c.start + m.t, one_line(&m.note))),
        );
    }
    comments.sort_by(|a, b| a.0.total_cmp(&b.0));
    if !comments.is_empty() {
        s.push_str("### Open comments\n");
        for (t, n) in comments {
            let _ = writeln!(s, "- {}: {n}", mmss(t));
        }
    }
    let s = s.trim_end().to_string();
    if s.len() <= DIGEST_MAX {
        return s;
    }
    let mut cut = DIGEST_MAX;
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}\n… (digest cut at {DIGEST_MAX} characters)", &s[..cut])
}

/// What every Assistant request carries about the project (empty when the document is blank and the
/// digest has nothing): the document, then the digest.
pub fn for_request(p: &Project) -> String {
    let doc = p.context.trim();
    let dig = digest(p);
    let mut out = String::new();
    if !is_blank(doc) {
        let _ = write!(
            out,
            "\n\nProject Context (the user's own document about this project; follow its Rules as instructions, \
             and its Style guide when you make editing choices; edit it only with context.set when asked):\n{doc}"
        );
    }
    if !dig.is_empty() {
        let _ = write!(out, "\n\nProject digest (gathered from the planner, notes, moodboard and open comments; read-only):\n{dig}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Marker, MoodItem, Note};

    #[test]
    fn sections_read_and_write() {
        let d = template();
        assert!(is_blank(&d));
        assert_eq!(SECTIONS.iter().filter(|s| d.contains(&format!("## {s}"))).count(), 4);
        let d = set_section(&d, "About", "A travel vlog about Lisbon.");
        let d = set_section(&d, "rules", "- No music with lyrics.");
        assert_eq!(section(&d, "About"), "A travel vlog about Lisbon.");
        assert_eq!(section(&d, "Rules"), "- No music with lyrics.");
        assert_eq!(section(&d, "Links"), "");
        assert!(!is_blank(&d));
        // replacing keeps the others, and the order
        let d2 = set_section(&d, "About", "Lisbon, three days.");
        assert_eq!(section(&d2, "About"), "Lisbon, three days.");
        assert_eq!(section(&d2, "Rules"), "- No music with lyrics.");
        assert!(d2.find("## About").unwrap() < d2.find("## Rules").unwrap());
        // a missing section is added at the end
        let d3 = set_section("## About\nx\n", "Links", "https://example.com");
        assert_eq!(section(&d3, "Links"), "https://example.com");
        assert_eq!(section(&d3, "About"), "x");
        // ### sub-headings stay inside their section
        let d4 = "## Style guide\n### Captions\nbig\n## Links\n";
        assert_eq!(section(d4, "Style guide"), "### Captions\nbig");
    }

    #[test]
    fn style_blocks_apply_and_remove_cleanly() {
        let mine = set_section(&set_section(&template(), "Rules", "- Keep the logo in."), "About", "Gym channel.");
        let d = apply_style(&mine, "vlog", "Fast and punchy.", "- Average shot 1.9 s.");
        assert_eq!(applied_style(&d).as_deref(), Some("vlog"));
        assert!(section(&d, "Rules").starts_with("<!-- style: vlog -->\n- Average shot 1.9 s.\n<!-- /style -->"));
        assert!(section(&d, "Rules").ends_with("- Keep the logo in."), "the user's rules stay");
        assert!(section(&d, "Style guide").contains("Fast and punchy."));
        // a second style replaces the first, never stacks
        let d = apply_style(&d, "trailer", "Build tension.", "- Shots shrink toward the end.");
        assert_eq!(applied_style(&d).as_deref(), Some("trailer"));
        assert_eq!(d.matches(STYLE_OPEN).count(), 2);
        assert!(!d.contains("Average shot 1.9"));
        // removing gives back exactly what the user had
        assert_eq!(remove_style(&d), mine);
        assert_eq!(applied_style(&mine), None);
        // an empty document gets the template first
        let e = apply_style("", "vlog", "g", "r");
        assert_eq!(section(&e, "Style guide"), "<!-- style: vlog -->\ng\n<!-- /style -->");
        assert!(e.contains("## About") && e.contains("## Links"));
    }

    #[test]
    fn digest_gathers_planner_notes_moodboard_and_open_comments() {
        let mut p = Project::new();
        assert_eq!(digest(&p), "");
        assert_eq!(for_request(&p), "");
        let mk = |path: &str| -> crate::model::Asset {
            serde_json::from_value(serde_json::json!({"id": 0, "path": path, "kind": "Video", "duration": 1.0,
                "width": 64, "height": 64, "fps": 30.0}))
            .unwrap()
        };
        let a = p.add_asset(mk("C:/clips/sunset.mp4"));
        let b = p.add_asset(mk("C:/clips/ref.png"));
        p.plan.push(PlanItem {
            id: 1,
            title: "Shoot the intro".into(),
            notes: "golden hour".into(),
            assets: vec![b],
            asset_notes: vec!["colour like this".into()],
            requirements: vec![("tripod".into(), true)],
            children: vec![PlanItem { id: 2, title: "B-roll of the bridge".into(), done: true, ..Default::default() }],
            ..Default::default()
        });
        p.notes.push(Note { id: 3, title: "Tone".into(), label: 0, body: "Calm, no jokes.".into() });
        p.moodboard.push(MoodItem { asset: a, labels: vec!["hero".into(), "warm".into()] });
        p.markers.push(Marker { id: 4, t: 65.0, note: "cut this pause".into(), ..Default::default() });
        p.markers.push(Marker { id: 5, t: 10.0, note: "already fixed".into(), done: true, ..Default::default() });
        p.markers.push(Marker { id: 6, t: 12.0, name: "just a marker".into(), ..Default::default() });
        let d = digest(&p);
        assert!(d.contains("- [ ] Shoot the intro: golden hour"), "{d}");
        assert!(d.contains("  - [x] tripod"), "{d}");
        assert!(d.contains("  - [x] B-roll of the bridge"), "{d}");
        assert!(d.contains("#### Tone\nCalm, no jokes."), "{d}");
        assert!(d.contains("- sunset.mp4: hero, warm"), "{d}");
        assert!(d.contains("- ref.png: colour like this"), "{d}");
        assert!(d.contains("- 1:05.0: cut this pause"), "{d}");
        assert!(!d.contains("already fixed") && !d.contains("just a marker"), "{d}");
        let order: Vec<usize> = ["### Planner", "### Notes", "### Moodboard", "### Open comments"].iter().map(|h| d.find(h).unwrap()).collect();
        assert!(order.windows(2).all(|w| w[0] < w[1]));
        // the request block carries the document (when written) and the digest
        assert!(!for_request(&p).contains("Project Context"));
        p.context = set_section(&template(), "Rules", "- Never use red text.");
        let r = for_request(&p);
        assert!(r.contains("follow its Rules as instructions") && r.contains("- Never use red text.") && r.contains("cut this pause"));
    }

    #[test]
    fn digest_is_capped() {
        let mut p = Project::new();
        p.notes.push(Note { id: 1, title: "Huge".into(), label: 0, body: "word ".repeat(10_000) });
        let d = digest(&p);
        assert!(d.len() < DIGEST_MAX + 100 && d.ends_with("characters)"));
    }
}
