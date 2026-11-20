//! Ctrl+P palette, one box with three modes chosen by the FIRST character only (`mode`):
//! - plain text: go to things (`goto_rows` over an `Item` index built once per project change by
//!   `index`): clips on every timeline (nested sequences too), timelines, Library media and bins,
//!   markers / comments, pages, panels and Gallery cards. Enter goes there (`plan`), Alt+Enter runs the
//!   row's secondary action (add an asset at the playhead, apply a Gallery card).
//! - `>`: commands - every `Action`, the panes no Action toggles ("Show X"), arg-free tools that change
//!   something, Luau scripts and the on/off settings (`rows`). `:` (kept for MCP users) lists EVERY tool
//!   by its raw name and builds a tiny arg form for one that takes args.
//! - `/`: the assistant, and the only way to it from here: `/ask <message>`, or `/<skill> [args]`
//!   (`assistant_rows`).
//!
//! `rows`/`tool_rows` take plain data (a `MenuSnapshot`, a pane->Action fn, a script-metadata slice) -
//! not `&App` - so they're unit-testable directly: there is no headless `App` harness in this crate
//! (`App::new` needs a real `eframe::CreationContext`/GL context). `ui::app::palette_ctl` is the thin,
//! App-owning glue that calls these from a live `App` and dispatches the `Command` they return.
//! ---- ws:command-palette ----

use crate::engine::skills::Skill;
use crate::hotkeys::{Action, Hotkeys};
use crate::mcp::tools::ToolKind;
use crate::model::{ClipKind, Id, Project, Track};
use crate::scripting::ScriptMeta;
use crate::ui::gallery::GalleryTab;
use crate::ui::layout::{pane_icon, Pane};
use crate::ui::menu::{self, MenuSnapshot};
use crate::ui::tools::{self, Glyph};
use eframe::egui;
use std::path::PathBuf;

/// What the first character of the query selects (only the first: a `>` or `/` later in the text is
/// plain text).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode<'a> {
    /// Plain text: go to clips, timelines, media, markers, pages, panels, Gallery cards.
    GoTo(&'a str),
    /// `>`: run a command.
    Commands(&'a str),
    /// `:`: any tool by its raw name (an arg form for one that takes args).
    Tools(&'a str),
    /// `/`: the assistant - the text after the `/` (`ask hi`, `captions bold`).
    Assistant(&'a str),
}

pub fn mode(query: &str) -> Mode<'_> {
    match query.chars().next() {
        Some('>') => Mode::Commands(query[1..].trim_start()),
        Some(':') => Mode::Tools(query[1..].trim_start()),
        Some('/') => Mode::Assistant(&query[1..]),
        _ => Mode::GoTo(query.trim()),
    }
}

/// Where a go-to row leads.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// A clip on timeline `seq` (None = the main one), starting at `t` there.
    Clip { id: Id, seq: Option<Id>, t: f64 },
    /// Open a timeline (None = back to the main one).
    Timeline(Option<Id>),
    Asset(Id),
    /// A Library folder ("Footage/Day 1").
    Bin(String),
    Marker { id: Id, seq: Option<Id>, t: f64 },
    Page(String),
    Gallery(GalleryTab, String),
}

/// One thing the palette can run.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Action(Action),
    Pane(Pane),
    Tool(&'static str),
    Script(PathBuf),
    /// Set an on/off setting (`settings.set`) to this value.
    Setting(&'static str, bool),
    GoTo(Target),
    /// Send this text to the assistant (a `/skill args` text runs that skill).
    Ask(String),
    /// Put this text in the box and keep the palette open (a skill that needs arguments).
    Complete(String),
    /// Export with the Export pane's current settings: the save dialog, then the job starts.
    QuickExport,
    // ---- ws:forum-ui ----
    /// A plain-phrase task (`ui::tasks::TASKS[i]`).
    Task(usize),
}

impl Default for Command {
    fn default() -> Self {
        Command::Action(Action::CommandPalette)
    }
}

#[derive(Default)]
pub struct Row {
    pub cmd: Command,
    pub label: String,
    pub shortcut: String,
    pub glyph: Option<Glyph>,
    pub enabled: bool,
    /// Why a disabled row is disabled (shown on the row), or what a tool does (on hover).
    pub reason: Option<&'static str>,
    /// Grey text after the label: a clip's track and time, an asset's folder, a skill's description.
    pub sub: String,
    /// The type heading the row sits under (go-to mode; "" = no heading).
    pub group: &'static str,
    /// What Alt+Enter does on this row, if anything ("Add at playhead").
    pub alt: Option<&'static str>,
}

/// "timeline.close_gap" -> "Close gap (timeline)".
pub fn tool_label(name: &str) -> String {
    let (group, verb) = name.split_once('.').unwrap_or(("", name));
    let mut verb = verb.replace('_', " ");
    if let Some(c) = verb.get(..1) {
        verb.replace_range(..1, &c.to_uppercase());
    }
    if group.is_empty() {
        verb
    } else {
        format!("{verb} ({group})")
    }
}

/// Allocation-free, case-insensitive subsequence scorer: every character of `query` must appear in
/// `text`, in order (not necessarily contiguous). Higher is a better match; matching consecutively
/// scores a bonus, and the FIRST query character landing right at a word boundary scores a bigger one
/// (deliberately not every character - see `fuzzy_score_prefers_word_starts`'s regression case: without
/// this restriction a long label with several word starts, e.g. "Ripple Delete In / Out" against query
/// "redo", could out-score the near-exact match "Redo" purely by accumulating more word-start bonuses
/// than a short label has room for). `None` = `query` is not a subsequence of `text` at all (an empty
/// query matches everything with score 0).
pub fn fuzzy_score(query: &str, text: &str) -> Option<u32> {
    if query.is_empty() {
        return Some(0);
    }
    let mut score: u32 = 0;
    let mut chars = text.chars().enumerate();
    let mut prev_idx: Option<usize> = None;
    let mut at_word_start = true;
    for (qi, qc) in query.chars().enumerate() {
        let qc = qc.to_ascii_lowercase();
        loop {
            let (i, tc) = chars.next()?; // ran out of text before this query char matched anything
            let word_start_here = at_word_start;
            at_word_start = !tc.is_alphanumeric();
            if tc.to_ascii_lowercase() == qc {
                score += 1;
                if qi == 0 && word_start_here {
                    score += 8;
                }
                if prev_idx == Some(i.wrapping_sub(1)) {
                    score += 4; // consecutive with the previous match (e.g. "spl" inside "Split")
                }
                prev_idx = Some(i);
                break;
            }
        }
    }
    Some(score)
}

fn push_scored(rows: &mut Vec<(u32, Row)>, query: &str, label: &str, row: Row) {
    if let Some(s) = fuzzy_score(query, label) {
        rows.push((s, row));
    }
}

fn finish(mut rows: Vec<(u32, Row)>) -> Vec<Row> {
    rows.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.label.cmp(&b.1.label)));
    rows.into_iter().map(|(_, r)| r).collect()
}

/// The `>` row that exports with the Export pane's settings (`Command::QuickExport`).
pub const QUICK_EXPORT: &str = "Quick export";

/// The `>` row set: `Action::ALL` + a "Show X" row for each pane `toggle_of` has no Action for +
/// arg-free tools that act (a `Read` tool's reply would be dropped here; `:` mode lists those) +
/// scripts + the on/off `settings` (key, label, current value), fuzzy-filtered by `query` (empty query = everything, most-recently-used actions first -
/// `recent`, capped to `Settings.palette_recent`). Shortcut text, icons (overrides included) and
/// disabled reasons come from `snap`, the same `ui::menu` snapshot every menu row reads.
pub fn rows(
    snap: &MenuSnapshot,
    toggle_of: impl Fn(Pane) -> Option<Action>,
    scripts: &[ScriptMeta],
    settings: &[(&'static str, &'static str, bool)],
    recent: &[String],
    query: &str,
) -> Vec<Row> {
    let mut rows: Vec<(u32, Row)> = Vec::new();
    let boost = |id: &str| -> u32 {
        if !query.is_empty() {
            return 0;
        }
        match recent.iter().position(|r| r == id) {
            Some(i) => 1000u32.saturating_sub(i as u32),
            None => 0,
        }
    };
    for &a in Action::ALL {
        let label = a.label();
        let Some(mut s) = fuzzy_score(query, label) else { continue };
        s += boost(a.id());
        let reason = snap.disabled.get(&a).copied();
        // a pane's Show / Hide row wears the pane's icon, unless the user picked one for the action
        let pane = Pane::ALL.iter().copied().find(|&p| toggle_of(p) == Some(a));
        let glyph = match (menu::glyph_for(&snap.icons, a), pane) {
            (None, Some(p)) if !snap.icons.contains_key(&format!("action.{}", a.id())) => pane_icon(&snap.icons, p),
            (g, _) => g,
        };
        rows.push((
            s,
            Row {
                cmd: Command::Action(a),
                label: label.to_string(),
                shortcut: snap.shortcuts.get(&a).map(Hotkeys::format).unwrap_or_default(),
                glyph,
                enabled: reason.is_none(),
                reason,
                ..Default::default()
            },
        ));
    }
    // ">Export" lists this first: the Export page's settings, straight to a file
    if let Some(s) = fuzzy_score(query, QUICK_EXPORT).or_else(|| fuzzy_score(query, "Export quick")) {
        let lead = if query.is_empty() { 0 } else { 40 };
        rows.push((
            s + lead + boost("export.quick_pane"),
            Row {
                cmd: Command::QuickExport,
                label: QUICK_EXPORT.into(),
                glyph: Some(Glyph::Upload),
                enabled: true,
                reason: Some("Pick the file, then export with the Export page's current settings."),
                ..Default::default()
            },
        ));
    }
    for &p in Pane::ALL.iter().filter(|&&p| toggle_of(p).is_none()) {
        let label = format!("Show {}", p.title());
        push_scored(
            &mut rows,
            query,
            &label,
            Row {
                cmd: Command::Pane(p),
                label: label.clone(),
                shortcut: String::new(),
                glyph: pane_icon(&snap.icons, p),
                enabled: true,
                reason: None,
                ..Default::default()
            },
        );
    }
    for t in crate::mcp::tools::all().filter(|t| t.args.is_empty() && matches!(t.kind, ToolKind::Mutate | ToolKind::Ui))
    {
        let label = tool_label(t.name);
        // the raw name still matches, for someone who knows it from MCP / Luau
        let Some(s) = fuzzy_score(query, &label).or_else(|| fuzzy_score(query, t.name)) else { continue };
        rows.push((
            s,
            Row {
                cmd: Command::Tool(t.name),
                label,
                shortcut: String::new(),
                glyph: Some(Glyph::Terminal),
                enabled: true,
                reason: Some(t.desc),
                ..Default::default()
            },
        ));
    }
    for &(key, label, on) in settings {
        let label = format!("{label}: turn {}", if on { "off" } else { "on" });
        push_scored(
            &mut rows,
            query,
            &label,
            Row {
                cmd: Command::Setting(key, !on),
                label: label.clone(),
                shortcut: if on { "on" } else { "off" }.into(),
                glyph: Some(Glyph::Gear),
                enabled: true,
                ..Default::default()
            },
        );
    }
    for sm in scripts {
        push_scored(
            &mut rows,
            query,
            &sm.name,
            Row {
                cmd: Command::Script(sm.path.clone()),
                label: sm.name.clone(),
                shortcut: sm.hotkey.clone().unwrap_or_default(),
                glyph: sm.icon.and_then(Glyph::from_name).or(Some(Glyph::Terminal)),
                enabled: true,
                reason: None,
                ..Default::default()
            },
        );
    }
    finish(rows)
}

/// `:`-mode rows: every registered tool (not just arg-free ones - see `rows` above), fuzzy-filtered by
/// `filter` (the text after the `:`). Picking one builds an arg form (`show`, below) instead of running
/// it immediately - needs no `App` at all, since `mcp::tools::all()` is a free fn over the static
/// `TOOL_TABLES` registry.
pub fn tool_rows(filter: &str) -> Vec<Row> {
    let mut rows: Vec<(u32, Row)> = Vec::new();
    for t in crate::mcp::tools::all() {
        push_scored(
            &mut rows,
            filter,
            t.name,
            Row {
                cmd: Command::Tool(t.name),
                label: t.name.to_string(),
                shortcut: String::new(),
                glyph: Some(Glyph::Terminal),
                enabled: true,
                reason: Some(t.desc),
                ..Default::default()
            },
        );
    }
    finish(rows)
}

// ---------------------------------------------------------------------------------------------------
// go to (plain text)

/// One go-to row, before scoring: built once per project change (`index`), scored per keystroke.
#[derive(Clone, Debug)]
pub struct Item {
    pub cmd: Command,
    pub label: String,
    pub sub: String,
    pub glyph: Option<Glyph>,
    /// One of `GROUPS`.
    pub group: &'static str,
}

/// The go-to headings, in the order a tie between their best matches is broken.
pub const GROUPS: [&str; 8] = ["Clips", "Timelines", "Media", "Bins", "Markers", "Pages", "Panels", "Gallery"];

impl Target {
    /// What Alt+Enter does on it.
    pub fn alt(&self) -> Option<&'static str> {
        match self {
            Target::Asset(_) => Some("Add at playhead"),
            Target::Gallery(..) => Some("Apply"),
            _ => None,
        }
    }
}

/// The id `Settings.palette_recent` keeps for a command (Action ids reuse `Action::id()`; the others get
/// a small namespaced prefix so they never collide with one).
pub fn recent_id(cmd: &Command) -> Option<String> {
    Some(match cmd {
        Command::Action(a) => a.id().to_string(),
        Command::Pane(p) => format!("pane.{}", p.title()),
        Command::Tool(name) => format!("tool.{name}"),
        Command::Script(path) => format!("script.{}", path.to_string_lossy()),
        Command::Setting(key, _) => format!("setting.{key}"),
        Command::QuickExport => "export.quick_pane".into(),
        Command::Task(i) => format!("task.{i}"),
        Command::GoTo(t) => match t {
            Target::Clip { id, .. } => format!("clip.{id}"),
            Target::Timeline(s) => format!("seq.{}", s.unwrap_or(0)),
            Target::Asset(id) => format!("asset.{id}"),
            Target::Bin(b) => format!("bin.{b}"),
            Target::Marker { id, .. } => format!("marker.{id}"),
            Target::Page(p) => format!("page.{p}"),
            Target::Gallery(t, n) => format!("gallery.{}.{n}", t.name()),
        },
        Command::Ask(_) | Command::Complete(_) => return None,
    })
}

/// Every timeline with its tracks wherever they are right now (the open one is swapped into
/// `Project.tracks`, the main one then waits in `main_stash`): (sequence id or None for main, name, tracks).
pub fn timelines(p: &Project) -> Vec<(Option<Id>, &str, &[Track])> {
    let main: &[Track] = match (&p.editing, &p.main_stash) {
        (Some(_), Some(st)) => &st.tracks,
        _ => &p.tracks,
    };
    let mut out = vec![(None, "Main timeline", main)];
    for s in &p.sequences {
        let tracks: &[Track] = if p.editing == Some(s.id) { &p.tracks } else { &s.tracks };
        out.push((Some(s.id), s.name.as_str(), tracks));
    }
    out
}

/// The go-to index: clips on every timeline, timelines, media and bins, markers, `pages`, panels and
/// `gallery` cards ((tab, names)). Pane icons honour `icons` (Settings ▸ Appearance overrides).
pub fn index(
    p: &Project,
    pages: &[String],
    gallery: &[(GalleryTab, Vec<String>)],
    icons: &std::collections::BTreeMap<String, String>,
) -> Vec<Item> {
    let tc = |t: f64| crate::ui::timecode(t, p.fps);
    let go = |t: Target| Command::GoTo(t);
    let mut out = Vec::new();
    let lines = timelines(p);
    for &(seq, name, tracks) in &lines {
        for (ti, tr) in tracks.iter().enumerate() {
            let tname = if tr.name.is_empty() { format!("Track {}", ti + 1) } else { tr.name.clone() };
            for c in &tr.clips {
                let glyph = match c.kind {
                    ClipKind::Audio => Glyph::Waveform,
                    ClipKind::Sequence => Glyph::Sequence,
                    ClipKind::Text => Glyph::Titles,
                    _ => Glyph::FilmStrip,
                };
                let mut sub = format!("{tname} · {}", tc(c.start));
                if seq.is_some() {
                    sub = format!("{sub} · in {name}");
                }
                out.push(Item {
                    cmd: go(Target::Clip { id: c.id, seq, t: c.start }),
                    label: c.name.clone(),
                    sub,
                    glyph: Some(glyph),
                    group: "Clips",
                });
            }
        }
    }
    for &(seq, name, tracks) in &lines {
        let end = tracks.iter().map(|t| t.end()).fold(0.0, f64::max);
        let open = if p.editing == seq { " · open" } else { "" };
        out.push(Item {
            cmd: go(Target::Timeline(seq)),
            label: name.to_string(),
            sub: format!("{} · {}{open}", if seq.is_some() { "Sequence" } else { "Timeline" }, tc(end)),
            glyph: Some(Glyph::Sequence),
            group: "Timelines",
        });
    }
    for a in &p.assets {
        let glyph = match a.kind {
            ClipKind::Audio => Glyph::MusicNote,
            ClipKind::Image => Glyph::Landscape,
            _ => Glyph::FilmReel,
        };
        let sub = if a.folder.is_empty() { "Library".to_string() } else { format!("Library ▸ {}", a.folder) };
        out.push(Item { cmd: go(Target::Asset(a.id)), label: a.name(), sub, glyph: Some(glyph), group: "Media" });
    }
    for f in &p.folders {
        let (parent, leaf) = f.rsplit_once('/').unwrap_or(("", f));
        let sub = if parent.is_empty() { "Library".to_string() } else { format!("Library ▸ {parent}") };
        out.push(Item {
            cmd: go(Target::Bin(f.clone())),
            label: leaf.to_string(),
            sub,
            glyph: Some(Glyph::Folder),
            group: "Bins",
        });
    }
    let seq_name = |s: Option<Id>| lines.iter().find(|l| l.0 == s).map_or("", |l| l.1);
    for m in &p.markers {
        let comment = !m.note.trim().is_empty();
        // a comment reads as its text (its name is usually just "Comment"), a marker as its name
        let label = match (m.name.trim(), m.note.trim()) {
            ("", "") => format!("Marker at {}", tc(m.t)),
            (name, "") => name.to_string(),
            (_, note) => note.lines().next().unwrap_or("").to_string(),
        };
        let who = if m.author.is_empty() { String::new() } else { format!(" by {}", m.author) };
        let mut sub = format!("{}{who} · {}", if comment { "Comment" } else { "Marker" }, tc(m.t));
        if m.sequence.is_some() {
            sub = format!("{sub} · in {}", seq_name(m.sequence));
        }
        out.push(Item {
            cmd: go(Target::Marker { id: m.id, seq: m.sequence, t: m.t }),
            label,
            sub,
            glyph: Some(if comment { Glyph::Notepad } else { Glyph::Bookmark }),
            group: "Markers",
        });
    }
    for pg in pages {
        out.push(Item {
            cmd: go(Target::Page(pg.clone())),
            label: format!("{pg} page"),
            sub: "Page".into(),
            glyph: Some(Glyph::GridIcon),
            group: "Pages",
        });
    }
    for &pane in Pane::ALL {
        out.push(Item {
            cmd: Command::Pane(pane),
            label: pane.title().to_string(),
            sub: "Panel".into(),
            glyph: pane_icon(icons, pane),
            group: "Panels",
        });
    }
    for (tab, names) in gallery {
        for n in names {
            out.push(Item {
                cmd: go(Target::Gallery(*tab, n.clone())),
                label: n.clone(),
                sub: format!("Gallery ▸ {}", tab.name()),
                glyph: Some(Glyph::Sliders),
                group: "Gallery",
            });
        }
    }
    out
}

fn item_row(it: &Item) -> Row {
    let alt = match &it.cmd {
        Command::GoTo(t) => t.alt(),
        _ => None,
    };
    Row {
        cmd: it.cmd.clone(),
        label: it.label.clone(),
        glyph: it.glyph,
        enabled: true,
        sub: it.sub.clone(),
        group: it.group,
        alt,
        ..Default::default()
    }
}

/// Go-to rows for `query`: grouped by type, the group with the best match first, best matches first
/// inside a group, at most `PER_GROUP` a group. An empty query lists the `recent` ones (most recent
/// first), then the pages and panels.
pub fn goto_rows(items: &[Item], recent: &[String], query: &str) -> Vec<Row> {
    const PER_GROUP: usize = 8;
    if query.is_empty() {
        let mut out: Vec<Row> = recent
            .iter()
            .filter_map(|r| items.iter().find(|it| recent_id(&it.cmd).as_deref() == Some(r.as_str())))
            .map(|it| Row { group: "Recent", ..item_row(it) })
            .take(PER_GROUP)
            .collect();
        out.extend(items.iter().filter(|it| it.group == "Pages" || it.group == "Panels").map(item_row));
        return out;
    }
    // (group index, score, item): a label match counts fully, a match in the grey text at half weight
    let mut hits: Vec<(usize, u32, &Item)> = items
        .iter()
        .filter_map(|it| {
            let s = fuzzy_score(query, &it.label).or_else(|| fuzzy_score(query, &it.sub).map(|s| s / 2))?;
            Some((GROUPS.iter().position(|g| *g == it.group).unwrap_or(GROUPS.len()), s, it))
        })
        .collect();
    let mut best = [0u32; GROUPS.len() + 1];
    for &(g, s, _) in &hits {
        best[g] = best[g].max(s);
    }
    hits.sort_by(|a, b| {
        best[b.0].cmp(&best[a.0]).then(a.0.cmp(&b.0)).then(b.1.cmp(&a.1)).then_with(|| a.2.label.cmp(&b.2.label))
    });
    let mut out = Vec::new();
    let mut per = [0usize; GROUPS.len() + 1];
    for (g, _, it) in hits {
        per[g] += 1;
        if per[g] <= PER_GROUP {
            out.push(item_row(it));
        }
    }
    out
}

/// What going to `target` does to the editor (`alt`: its secondary action instead). Pure, so tests can
/// check it without an `App`; `palette_ctl::go` applies it.
#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    /// Switch to this timeline first (None inside = the main one).
    pub timeline: Option<Option<Id>>,
    pub seek: Option<f64>,
    pub select_clip: Option<Id>,
    pub select_marker: Option<Id>,
    /// Select this asset in the Library (None = no asset) and show this folder.
    pub library: Option<(Option<Id>, String)>,
    pub page: Option<String>,
    pub gallery: Option<(GalleryTab, String)>,
    pub pane: Option<Pane>,
    /// Run this tool as the user's own undoable edit.
    pub tool: Option<(&'static str, serde_json::Value)>,
}

pub fn plan(target: &Target, alt: bool, p: &Project, playhead: f64) -> Plan {
    use serde_json::json;
    let switch = |seq: Option<Id>| (p.editing != seq).then_some(seq);
    match target {
        Target::Clip { id, seq, t } => Plan {
            timeline: switch(*seq),
            seek: Some(*t),
            select_clip: Some(*id),
            pane: Some(Pane::Timeline),
            ..Default::default()
        },
        Target::Timeline(seq) => Plan { timeline: switch(*seq), pane: Some(Pane::Timeline), ..Default::default() },
        Target::Marker { id, seq, t } => Plan {
            timeline: switch(*seq),
            seek: Some(*t),
            select_marker: Some(*id),
            pane: Some(Pane::Timeline),
            ..Default::default()
        },
        Target::Asset(id) if alt => {
            Plan { tool: Some(("timeline.place", json!({"asset_id": id, "at": playhead}))), ..Default::default() }
        }
        Target::Asset(id) => Plan {
            library: Some((Some(*id), p.asset(*id).map(|a| a.folder.clone()).unwrap_or_default())),
            pane: Some(Pane::Library),
            ..Default::default()
        },
        Target::Bin(b) => Plan { library: Some((None, b.clone())), pane: Some(Pane::Library), ..Default::default() },
        Target::Page(pg) => Plan { page: Some(pg.clone()), ..Default::default() },
        Target::Gallery(tab, name) if alt => {
            Plan { tool: Some(("gallery.apply", json!({"tab": tab.name(), "name": name}))), ..Default::default() }
        }
        Target::Gallery(tab, name) => {
            Plan { gallery: Some((*tab, name.clone())), pane: Some(Pane::Presets), ..Default::default() }
        }
    }
}

// ---------------------------------------------------------------------------------------------------
// the assistant (`/`)

/// `/` rows for `rest` (the text after the `/`): `/ask <message>` sends it; `/<skill> [args]` runs that
/// skill; while the name is being typed, `/ask` and the matching `skills` are listed with their
/// descriptions. `ask_row` is the assistant's own "Ask the assistant: …" row (`assistant_ctl::palette_row`).
pub fn assistant_rows(rest: &str, skills: &[Skill], ask_row: impl Fn(&str) -> Row) -> Vec<Row> {
    let (name, args) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    if name == "ask" && rest.len() > 3 {
        let r = ask_row(args);
        return vec![Row { cmd: Command::Ask(args.trim().to_string()), ..r }];
    }
    if let Some(s) = skills.iter().find(|s| s.name == name).filter(|_| rest.len() > name.len()) {
        return vec![skill_row(s, args.trim(), true)]; // "/name " (+ args): run it
    }
    let mut out = Vec::new();
    if "ask".starts_with(name) {
        out.push(Row {
            cmd: Command::Complete("/ask ".into()),
            label: "/ask".into(),
            sub: "Send a message to the assistant".into(),
            glyph: Some(Glyph::Assistant),
            enabled: true,
            ..Default::default()
        });
    }
    out.extend(crate::engine::skills::filter(skills, name, 50).into_iter().map(|s| skill_row(s, "", false)));
    if out.is_empty() {
        out.push(Row {
            label: format!("No skill called /{name}"),
            sub: "/ask <message> talks to the assistant".into(),
            glyph: Some(Glyph::Assistant),
            ..Default::default()
        });
    }
    out
}

fn skill_row(s: &Skill, args: &str, typed: bool) -> Row {
    let text = if args.is_empty() { format!("/{}", s.name) } else { format!("/{} {args}", s.name) };
    // a skill that takes arguments, picked from the list before any were typed: fill them in first
    let cmd = if !typed && !s.args.is_empty() {
        Command::Complete(format!("/{} ", s.name))
    } else {
        Command::Ask(text.clone())
    };
    let label = if s.args.is_empty() || typed { text } else { format!("{text} {}", s.args) };
    Row { cmd, label, sub: s.description.clone(), glyph: Some(Glyph::Assistant), enabled: true, ..Default::default() }
}

/// Ctrl+P palette state.
#[derive(Default)]
pub struct PaletteState {
    pub open: bool,
    pub query: String,
    pub sel: usize,
    /// `Some((tool name, [(arg name, typed-in value), ...]))` while the `:` mode's arg form is showing.
    pub arg_form: Option<(&'static str, Vec<(String, String)>)>,
    /// The go-to index and the project signature it was built for (`palette_ctl::index_for`): rebuilt
    /// when the project changes or the palette opens, never per keystroke.
    pub index: Vec<Item>,
    pub index_sig: Option<u64>,
    /// The rows of the last (query, index signature): typing or moving the selection redraws without
    /// scoring the index again.
    pub rows: Vec<Row>,
    pub rows_key: Option<(String, u64)>,
}

/// Draws the palette window if `state.open`. `rows` is the already-filtered/sorted list for the
/// current query (built by `palette_ctl::windows` for the query's `mode`). Returns the chosen `Command`
/// and whether it was Alt+Enter (the row's secondary action) on Enter/click and closes the window -
/// EXCEPT two picks that keep it open: a `Command::Complete` (its text replaces the query) and a tool
/// that takes args, which switches to an arg form; submitting that form (Run / Enter) is what finally
/// returns `Some(Command::Tool(name))`, and leaves `state.arg_form` populated with the typed values so
/// the caller can read them (`state.arg_form.take()`) right after this call returns, before anything
/// else touches `state` - every other way of closing (Escape, the window's own X, Cancel) clears
/// `arg_form` itself. Tab / Shift+Tab cycle the rows (the box keeps focus). Disables the text-cursor
/// blink while open so an idle palette costs no repaints (`assert_no_idle_repaint_palette_closed_and_open`).
pub fn show(ctx: &egui::Context, state: &mut PaletteState, rows: &[Row]) -> Option<(Command, bool)> {
    if !state.open {
        return None;
    }
    ctx.style_mut(|s| s.visuals.text_cursor.blink = false);
    let mut result = None;
    let mut open = true;
    let mut close_palette = false; // fully close (Escape at the top level, or the window's own X)
    let mut back_to_search = false; // leave arg-form mode, palette stays open on the search box
    let mut complete = None;
    egui::Window::new("Go to")
        .id(egui::Id::new("command_palette_window"))
        .order(egui::Order::Foreground)
        .collapsible(false)
        .resizable(false)
        .title_bar(false)
        .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 80.0))
        .default_width(560.0)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.set_width(560.0);
            if let Some((tool_name, fields)) = state.arg_form.clone() {
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    back_to_search = true;
                    return;
                }
                ui.strong(format!(": {tool_name}"));
                let mut fields = fields;
                for (k, v) in fields.iter_mut() {
                    ui.horizontal(|ui| {
                        ui.label(k.as_str());
                        ui.text_edit_singleline(v);
                    });
                }
                let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                let mut run = false;
                ui.horizontal(|ui| {
                    if ui.button("Run").clicked() || enter {
                        run = true;
                    }
                    if ui.button("Cancel").clicked() {
                        back_to_search = true;
                    }
                });
                state.arg_form = Some((tool_name, fields)); // persist this frame's edits either way
                if run {
                    result = Some((Command::Tool(tool_name), false));
                    close_palette = true;
                }
                return;
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                close_palette = true;
                return;
            }
            let id = egui::Id::new("command_palette_query");
            let first_frame = ctx.read_response(id).is_none();
            let r = ui.add(
                egui::TextEdit::singleline(&mut state.query)
                    .id(id)
                    .lock_focus(true) // Tab cycles the rows instead of leaving the box
                    .hint_text("Go to a clip, media, timeline, marker, page or panel…")
                    .desired_width(f32::INFINITY),
            );
            // the box keeps the keyboard while the palette is open (a click on a row, or a Tab before the
            // focus lock exists, would otherwise leave it)
            if first_frame || !r.has_focus() {
                r.request_focus();
            }
            if r.changed() {
                state.sel = 0;
            }
            let n = rows.len();
            state.sel = state.sel.min(n.saturating_sub(1));
            let (enter, alt) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.modifiers.alt));
            let moved = ui.input(|i| {
                let before = state.sel;
                let tab = i.key_pressed(egui::Key::Tab);
                if n > 0 && (i.key_pressed(egui::Key::ArrowDown) || (tab && !i.modifiers.shift)) {
                    state.sel = if tab { (state.sel + 1) % n } else { (state.sel + 1).min(n - 1) };
                }
                if n > 0 && (i.key_pressed(egui::Key::ArrowUp) || (tab && i.modifiers.shift)) {
                    state.sel = if tab { (state.sel + n - 1) % n } else { state.sel.saturating_sub(1) };
                }
                state.sel != before
            });
            egui::ScrollArea::vertical().max_height(380.0).auto_shrink([false, true]).show(ui, |ui| {
                let mut group = "";
                for (i, row) in rows.iter().enumerate() {
                    if !row.group.is_empty() && row.group != group {
                        group = row.group;
                        ui.add_space(if i == 0 { 0.0 } else { 4.0 });
                        ui.label(egui::RichText::new(group).small().weak());
                    }
                    // the menu rows' shape: icon gutter, label, grey detail, shortcut on the right; a
                    // disabled row says why right there (a hover tip never shows for keyboard use)
                    let right = match (row.enabled, row.reason) {
                        (false, Some(why)) if row.shortcut.is_empty() => why.to_string(),
                        (false, Some(why)) => format!("{why}  ·  {}", row.shortcut),
                        _ if i == state.sel && row.alt.is_some() => format!("Alt+Enter: {}", row.alt.unwrap_or("")),
                        _ => row.shortcut.clone(),
                    };
                    let icon_id = ui.id().with(("palette-icon", i));
                    let gutter = egui::Atom::custom(icon_id, egui::vec2(18.0, 16.0));
                    let sub = egui::RichText::new(format!("  {}", row.sub)).weak();
                    let atoms = egui::Atoms::new((gutter, row.label.as_str(), sub));
                    let button = egui::Button::selectable(i == state.sel, atoms)
                        .truncate()
                        .shortcut_text(right)
                        .min_size(egui::vec2(ui.available_width(), 0.0));
                    let resp = ui
                        .add_enabled_ui(row.enabled, |ui| {
                            let r = button.atom_ui(ui);
                            for (id, rect) in r.custom_rects() {
                                if id == icon_id {
                                    if let Some(g) = row.glyph {
                                        let fg = ui.style().interact(&r.response).text_color();
                                        tools::draw_glyph(ui.painter(), rect, g, fg);
                                    }
                                }
                            }
                            r.response
                        })
                        .inner;
                    if moved && i == state.sel {
                        resp.scroll_to_me(None);
                    }
                    let resp = match row.reason {
                        Some(r) if row.enabled => resp.on_hover_text(r),
                        _ => resp,
                    };
                    let picked = resp.clicked() || (enter && i == state.sel);
                    if picked && row.enabled {
                        if let Command::Complete(text) = &row.cmd {
                            complete = Some(text.clone());
                            return;
                        }
                        if let Command::Tool(name) = &row.cmd {
                            if let Some(def) = crate::mcp::tools::find(name) {
                                if !def.args.is_empty() {
                                    let fields: Vec<(String, String)> = def
                                        .args
                                        .iter()
                                        .map(|a| (a.split(':').next().unwrap_or("").to_string(), String::new()))
                                        .collect();
                                    state.arg_form = Some((name, fields));
                                    return;
                                }
                            }
                        }
                        result = Some((row.cmd.clone(), alt && row.alt.is_some()));
                        close_palette = true;
                    }
                }
                if rows.is_empty() && !state.query.trim().is_empty() {
                    ui.weak("Nothing found");
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                let hint = |ui: &mut egui::Ui, key: &str, what: &str| {
                    ui.label(egui::RichText::new(key).small().strong());
                    ui.label(egui::RichText::new(what).small().weak());
                };
                hint(ui, "text", "go to");
                hint(ui, ">", "commands");
                hint(ui, "/", "assistant");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new("↑↓ Tab  ·  Enter  ·  Alt+Enter  ·  Esc").small().weak());
                });
            });
        });
    if let Some(text) = complete {
        state.query = text;
        state.sel = 0;
        // the caret to the end of the new text
        let id = egui::Id::new("command_palette_query");
        if let Some(mut st) = egui::TextEdit::load_state(ctx, id) {
            let end = egui::text::CCursor::new(state.query.chars().count());
            st.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
            st.store(ctx, id);
        }
    }
    if back_to_search {
        state.arg_form = None;
    }
    if close_palette || !open {
        state.open = false;
        state.query.clear();
        state.sel = 0;
        state.index_sig = None; // the next open re-indexes (Gallery cards and pages live outside the project)
        state.rows_key = None;
        if result.is_none() {
            // a real close/cancel - a successful arg-form submit (`result` = Some(Tool(..))) leaves
            // `arg_form` for the caller to read once, right after this call returns (see doc comment)
            state.arg_form = None;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scripting::ScriptMeta;
    use std::time::Duration;

    #[test]
    fn fuzzy_score_prefers_word_starts() {
        // word-start bonus: "sp" as the start of "Split at Playhead" beats "sp" buried mid-word
        let word_start = fuzzy_score("sp", "Split at Playhead").unwrap();
        let mid_word = fuzzy_score("sp", "Grasp").unwrap();
        assert!(word_start > mid_word, "{word_start} should beat {mid_word}");
        // consecutive-char bonus: "spl" scores higher than the same 3 letters scattered apart
        let consecutive = fuzzy_score("spl", "Split").unwrap();
        let scattered = fuzzy_score("spl", "Sample Playhead List").unwrap();
        assert!(consecutive > scattered, "{consecutive} should beat {scattered}");
        // not a subsequence at all
        assert_eq!(fuzzy_score("xyz", "Split"), None);
        // empty query matches everything with score 0
        assert_eq!(fuzzy_score("", "anything"), Some(0));
        // case-insensitive
        assert!(fuzzy_score("SPLIT", "split at playhead").is_some());
    }

    fn dummy_script(name: &str) -> ScriptMeta {
        ScriptMeta {
            path: PathBuf::from(format!("{name}.luau")),
            name: name.to_string(),
            desc: String::new(),
            icon: None,
            hotkey: None,
            on: Vec::new(),
            budget: Duration::from_millis(250),
        }
    }

    fn snap(enabled: impl Fn(Action) -> Result<(), &'static str>) -> MenuSnapshot {
        MenuSnapshot::new(&Hotkeys::defaults(), &Default::default(), enabled)
    }

    /// The test's own pane -> toggle Action map (the app's is `menus::toggle_action`, pinned in
    /// `palette_ctl`'s tests).
    fn toggles(p: Pane) -> Option<Action> {
        (p == Pane::Library).then_some(Action::ToggleLibrary)
    }

    #[test]
    fn palette_lists_every_action_and_one_row_per_pane() {
        let scripts = vec![dummy_script("my_script")];
        let all = rows(&snap(|_| Ok(())), toggles, &scripts, &[], &[], "");
        for &a in Action::ALL {
            assert!(all.iter().any(|r| matches!(&r.cmd, Command::Action(x) if *x == a)), "missing Action {a:?}");
        }
        // Library is reached through its Show / Hide Action (wearing the pane's icon), not a second row
        assert!(!all.iter().any(|r| r.cmd == Command::Pane(Pane::Library)));
        let lib = all.iter().find(|r| r.cmd == Command::Action(Action::ToggleLibrary)).unwrap();
        assert_eq!(lib.glyph, Some(Pane::Library.glyph()));
        assert!(all.iter().any(|r| r.cmd == Command::Pane(Pane::Timeline)), "a pane with no Action keeps Show X");
        // the pages are their Actions, once each
        let pages: Vec<_> = all.iter().filter(|r| r.label.ends_with(" page")).map(|r| r.label.as_str()).collect();
        // one Action per built-in page (Alt+1..6, then the unbound Export page action); "Page 8/9" are spare slots
        assert_eq!(pages.len(), crate::ui::layout::PAGES.len(), "one page Action per built-in page: {pages:?}");
        // arg-free tools that act are listed under a readable name; a Read tool (its reply would be
        // dropped) and an arg-taking tool only show up in ':' mode
        for t in crate::mcp::tools::all().filter(|t| t.args.is_empty()) {
            let listed = all.iter().any(|r| r.cmd == Command::Tool(t.name));
            assert_eq!(listed, matches!(t.kind, ToolKind::Mutate | ToolKind::Ui), "{}", t.name);
        }
        assert!(!all.iter().any(|r| matches!(&r.cmd, Command::Tool(_)) && r.label.contains('.')), "raw tool name");
        let has_args = crate::mcp::tools::all().find(|t| !t.args.is_empty()).expect("at least one tool takes args");
        assert!(!all.iter().any(|r| r.cmd == Command::Tool(has_args.name)));
        assert!(all.iter().any(|r| matches!(&r.cmd, Command::Script(p) if p.ends_with("my_script.luau"))));
    }

    /// `>Export` (and its variants) lists "Quick export" first, with the Export page commands after it.
    #[test]
    fn export_query_lists_quick_export_first() {
        let s = snap(|_| Ok(()));
        for q in ["Export", "export", "exp", "quick", "quick export", "Export quick"] {
            let typed = format!(">{q}");
            let Mode::Commands(q) = mode(&typed) else { panic!(">{q} is command mode") };
            let rs = rows(&s, toggles, &[], &[], &[], q);
            assert_eq!(rs.first().map(|r| &r.cmd), Some(&Command::QuickExport), ">{q}");
            assert_eq!(rs[0].label, QUICK_EXPORT);
        }
        let rs = rows(&s, toggles, &[], &[], &[], "Export");
        for a in [Action::ExportVideo, Action::QuickExportAs] {
            assert!(rs.iter().any(|r| r.cmd == Command::Action(a)), "{a:?} still listed");
        }
        assert_eq!(recent_id(&Command::QuickExport).as_deref(), Some("export.quick_pane"));
    }

    #[test]
    fn tool_names_read_as_words() {
        assert_eq!(tool_label("timeline.close_gap"), "Close gap (timeline)");
        assert_eq!(tool_label("playback.play"), "Play (playback)");
        assert_eq!(tool_label("undo"), "Undo");
        // the raw name still finds it
        let s = snap(|_| Ok(()));
        let t = crate::mcp::tools::all()
            .find(|t| t.args.is_empty() && t.kind == ToolKind::Ui)
            .expect("an arg-free Ui tool");
        assert!(rows(&s, toggles, &[], &[], &[], t.name).iter().any(|r| r.cmd == Command::Tool(t.name)), "{}", t.name);
    }

    /// Settings ▸ Appearance ▸ Icons applies here too, exactly as in the menus.
    #[test]
    fn icon_overrides_are_honoured() {
        let mut icons = std::collections::BTreeMap::new();
        icons.insert("action.undo".to_string(), "none".to_string());
        icons.insert("action.toggle_library".to_string(), "none".to_string());
        icons.insert("action.redo".to_string(), Glyph::Flag.name().to_string());
        let s = MenuSnapshot::new(&Hotkeys::defaults(), &icons, |_| Ok(()));
        let all = rows(&s, toggles, &[], &[], &[], "");
        let glyph = |a: Action| all.iter().find(|r| r.cmd == Command::Action(a)).unwrap().glyph;
        assert_eq!(glyph(Action::Undo), None);
        assert_eq!(glyph(Action::ToggleLibrary), None, "'none' beats the pane-icon fallback");
        assert_eq!(glyph(Action::Redo), Some(Glyph::Flag));
        assert_eq!(glyph(Action::Save), tools::action_glyph(Action::Save));
    }

    #[test]
    fn tool_rows_covers_every_tool_including_ones_with_args() {
        let all = tool_rows("");
        assert_eq!(all.len(), crate::mcp::tools::all().count());
    }

    #[test]
    fn disabled_action_carries_its_reason() {
        let s = snap(|a| if a == Action::Undo { Err("Nothing to undo") } else { Ok(()) });
        let rs = rows(&s, toggles, &[], &[], &[], "undo");
        let row = rs.iter().find(|r| matches!(&r.cmd, Command::Action(Action::Undo))).unwrap();
        assert!(!row.enabled);
        assert_eq!(row.reason, Some("Nothing to undo"));
    }

    #[test]
    fn recent_boosts_ordering_only_when_query_empty() {
        let s = snap(|_| Ok(()));
        let recent = vec![Action::Undo.id().to_string()];
        let rs = rows(&s, toggles, &[], &[], &recent, "");
        assert!(matches!(&rs[0].cmd, Command::Action(Action::Undo)), "recent action should sort first");
        // once there's a query, recency stops mattering - plain fuzzy order applies
        let rs2 = rows(&s, toggles, &[], &[], &recent, "redo");
        assert!(matches!(&rs2[0].cmd, Command::Action(Action::Redo)));
    }

    /// Enter on the selected row returns exactly that command and closes the window.
    #[test]
    fn palette_enter_pushes_one_command_and_closes() {
        let ctx = egui::Context::default();
        let rs = rows(&snap(|_| Ok(())), toggles, &[], &[], &[], "undo");
        let undo_i = rs.iter().position(|r| matches!(&r.cmd, Command::Action(Action::Undo))).expect("Undo row present");
        let mut state = PaletteState { open: true, query: "undo".into(), sel: undo_i, ..Default::default() };
        let mut result = None;
        // settle the first-frame focus request, then drive the accept path with a synthetic Enter
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            result = show(ctx, &mut state, &rs);
        });
        let mut raw = egui::RawInput::default();
        raw.events.push(egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        let _ = ctx.run(raw, |ctx| {
            result = show(ctx, &mut state, &rs);
        });
        assert_eq!(result, Some((Command::Action(Action::Undo), false)));
        assert!(!state.open, "the window must close on accept");
        assert!(state.query.is_empty(), "the query resets on close");
    }

    /// 30 idle frames (no input), palette closed then open: neither state asks for a repaint (the
    /// text-cursor blink is disabled while open - see `show`'s doc comment).
    #[test]
    fn assert_no_idle_repaint_palette_closed_and_open() {
        let s = snap(|_| Ok(()));
        for open in [false, true] {
            let ctx = egui::Context::default();
            let mut state = PaletteState { open, ..Default::default() };
            for _ in 0..30 {
                let rs = rows(&s, toggles, &[], &[], &[], &state.query);
                let _ = ctx.run(egui::RawInput::default(), |ctx| {
                    let _ = show(ctx, &mut state, &rs);
                });
            }
            assert!(!ctx.has_requested_repaint(), "idle palette (open={open}) requested a repaint");
        }
    }

    // ---- Ctrl+P: modes, go to, the assistant ----

    #[test]
    fn mode_is_chosen_by_the_first_character_only() {
        assert_eq!(mode("intro"), Mode::GoTo("intro"));
        assert_eq!(mode("  intro "), Mode::GoTo("intro"));
        assert_eq!(mode(">split"), Mode::Commands("split"));
        assert_eq!(mode("> split"), Mode::Commands("split"));
        // a '>' or '/' later in the text is plain text
        assert_eq!(mode("a > b"), Mode::GoTo("a > b"));
        assert_eq!(mode("intro/outro"), Mode::GoTo("intro/outro"));
        assert_eq!(mode(" >x"), Mode::GoTo(">x"));
        assert_eq!(mode("/ask cut the ums"), Mode::Assistant("ask cut the ums"));
        assert_eq!(mode("/captions bold"), Mode::Assistant("captions bold"));
        assert_eq!(mode(":timeline.find"), Mode::Tools("timeline.find"));
        assert_eq!(mode(""), Mode::GoTo(""));
    }

    fn skill(name: &str, args: &str, desc: &str) -> Skill {
        Skill {
            name: name.into(),
            description: desc.into(),
            args: args.into(),
            body: "do it".into(),
            source: crate::engine::skills::Source::User,
            path: None,
            thumb: String::new(),
        }
    }

    fn ask_row(text: &str) -> Row {
        Row { label: format!("Ask the assistant: {}", text.trim()), enabled: !text.trim().is_empty(), ..Default::default() }
    }

    #[test]
    fn slash_ask_and_slash_skill_reach_the_assistant() {
        let skills = vec![skill("captions", "[style]", "Add captions"), skill("cleanup", "", "Remove silences")];
        // /ask <message> sends just the message
        let r = assistant_rows("ask cut the ums", &skills, ask_row);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].cmd, Command::Ask("cut the ums".into()));
        assert!(r[0].enabled);
        assert!(!assistant_rows("ask ", &skills, ask_row)[0].enabled, "an empty /ask sends nothing");
        // /skill args runs the skill (the assistant resolves the /name text)
        let r = assistant_rows("captions bold yellow", &skills, ask_row);
        assert_eq!(r[0].cmd, Command::Ask("/captions bold yellow".into()));
        assert_eq!(r[0].sub, "Add captions");
        // while the name is typed: /ask and the matching skills, with descriptions
        let r = assistant_rows("c", &skills, ask_row);
        let names: Vec<_> = r.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(names, ["/captions [style]", "/cleanup"]);
        assert_eq!(r[0].cmd, Command::Complete("/captions ".into()), "a skill with args fills the box first");
        assert_eq!(r[1].cmd, Command::Ask("/cleanup".into()), "a skill without args runs");
        assert_eq!(assistant_rows("", &skills, ask_row)[0].cmd, Command::Complete("/ask ".into()));
        let none = assistant_rows("zzz", &skills, ask_row);
        assert!(none.len() == 1 && !none[0].enabled);
    }

    /// No other mode reaches the assistant: plain text and '>' never produce a `Command::Ask`.
    #[test]
    fn only_slash_reaches_the_assistant() {
        let p = seeded();
        let idx = index(&p, &["Edit".into()], &[], &Default::default());
        for q in ["ask cut the ums", "cut", "assistant"] {
            assert!(!goto_rows(&idx, &[], q).iter().any(|r| matches!(r.cmd, Command::Ask(_))), "{q}");
            let rs = rows(&snap(|_| Ok(())), toggles, &[], &[], &[], q);
            assert!(!rs.iter().any(|r| matches!(r.cmd, Command::Ask(_))), "> {q}");
        }
    }

    /// A project with a clip on the main timeline, a sequence holding another clip, an asset in a bin,
    /// a comment marker inside the sequence.
    fn seeded() -> Project {
        use crate::model::{Clip, Marker, Sequence, TrackKind};
        let mut p = Project::new();
        p.tracks[0].clips.push(Clip::new(10, ClipKind::Video, "Intro Shot", 2.0, 3.0));
        let mut t = Track::new(20, TrackKind::Video, "V1");
        t.clips.push(Clip::new(21, ClipKind::Video, "Drone Flyover", 4.5, 2.0));
        p.sequences.push(Sequence { id: 30, name: "B-Roll Nest".into(), width: 1920, height: 1080, fps: 30.0, tracks: vec![t], max_duration: None });
        let a: crate::model::Asset = serde_json::from_value(serde_json::json!({
            "id": 40, "path": "C:/media/sunset_beach.mp4", "kind": "Video", "duration": 5.0,
            "width": 1920, "height": 1080, "fps": 30.0, "folder": "Footage/Day 1"
        }))
        .unwrap();
        p.assets.push(a);
        p.folders.push("Footage/Day 1".into());
        p.markers.push(Marker { id: 50, t: 6.0, note: "Fix the colour here".into(), author: "Sam".into(), sequence: Some(30), ..Default::default() });
        p
    }

    fn first(rows: &[Row], group: &str) -> Command {
        rows.iter().find(|r| r.group == group).unwrap_or_else(|| panic!("no {group} row")).cmd.clone()
    }

    #[test]
    fn each_kind_is_found_with_type_and_detail() {
        let p = seeded();
        let gallery = vec![(GalleryTab::Looks, vec!["Teal Orange".to_string()])];
        let idx = index(&p, &["Edit".into(), "Color".into()], &gallery, &Default::default());
        // a clip inside a nested sequence: its track, time and timeline in the grey text
        let r = goto_rows(&idx, &[], "drone");
        assert_eq!(r[0].cmd, Command::GoTo(Target::Clip { id: 21, seq: Some(30), t: 4.5 }));
        assert_eq!(r[0].group, "Clips");
        assert!(r[0].sub.starts_with("V1 · 00:00:04:15") && r[0].sub.ends_with("in B-Roll Nest"), "{}", r[0].sub);
        assert_eq!(first(&goto_rows(&idx, &[], "intro"), "Clips"), Command::GoTo(Target::Clip { id: 10, seq: None, t: 2.0 }));
        assert_eq!(first(&goto_rows(&idx, &[], "nest"), "Timelines"), Command::GoTo(Target::Timeline(Some(30))));
        let r = goto_rows(&idx, &[], "sunset");
        assert_eq!(r[0].cmd, Command::GoTo(Target::Asset(40)));
        assert_eq!(r[0].sub, "Library ▸ Footage/Day 1");
        assert_eq!(r[0].alt, Some("Add at playhead"));
        assert_eq!(first(&goto_rows(&idx, &[], "day 1"), "Bins"), Command::GoTo(Target::Bin("Footage/Day 1".into())));
        let r = goto_rows(&idx, &[], "fix the colour");
        assert_eq!(r[0].cmd, Command::GoTo(Target::Marker { id: 50, seq: Some(30), t: 6.0 }));
        assert!(r[0].sub.starts_with("Comment by Sam"), "{}", r[0].sub);
        assert_eq!(first(&goto_rows(&idx, &[], "color page"), "Pages"), Command::GoTo(Target::Page("Color".into())));
        assert_eq!(first(&goto_rows(&idx, &[], "inspector"), "Panels"), Command::Pane(Pane::Inspector));
        let r = goto_rows(&idx, &[], "teal");
        assert_eq!(r[0].cmd, Command::GoTo(Target::Gallery(GalleryTab::Looks, "Teal Orange".into())));
        assert_eq!(r[0].alt, Some("Apply"));
        // grouped: rows of one type sit together
        let r = goto_rows(&idx, &[], "e");
        let mut seen: Vec<&str> = Vec::new();
        for row in &r {
            if seen.last() != Some(&row.group) {
                assert!(!seen.contains(&row.group), "group {} split up", row.group);
                seen.push(row.group);
            }
        }
        // empty query: recent first, then pages and panels
        let recent = vec!["asset.40".to_string(), "clip.21".to_string()];
        let r = goto_rows(&idx, &recent, "");
        assert_eq!(r[0].cmd, Command::GoTo(Target::Asset(40)));
        assert_eq!(r[1].cmd, Command::GoTo(Target::Clip { id: 21, seq: Some(30), t: 4.5 }));
        assert_eq!(r[0].group, "Recent");
        assert!(r.iter().any(|r| r.group == "Pages") && r.iter().any(|r| r.group == "Panels"));
        // the ids remembered are the ones read back
        assert_eq!(recent_id(&r[0].cmd).as_deref(), Some("asset.40"));
    }

    /// What Enter / Alt+Enter does for each kind, applied to the project where it touches it.
    #[test]
    fn each_kind_is_acted_on() {
        let mut p = seeded();
        // a clip in the nested sequence: open that sequence, seek to the clip, select it
        let pl = plan(&Target::Clip { id: 21, seq: Some(30), t: 4.5 }, false, &p, 0.0);
        assert_eq!((pl.timeline, pl.seek, pl.select_clip, pl.pane), (Some(Some(30)), Some(4.5), Some(21), Some(Pane::Timeline)));
        assert!(p.open_sequence(30));
        assert!(p.clip(21).is_some(), "the clip is on the open timeline");
        // already there: no switch; the main timeline's clip switches back to main
        assert_eq!(plan(&Target::Clip { id: 21, seq: Some(30), t: 4.5 }, false, &p, 0.0).timeline, None);
        assert_eq!(plan(&Target::Clip { id: 10, seq: None, t: 2.0 }, false, &p, 0.0).timeline, Some(None));
        // the index follows the swap: both clips still found, each on its own timeline
        let idx = index(&p, &[], &[], &Default::default());
        assert_eq!(first(&goto_rows(&idx, &[], "intro"), "Clips"), Command::GoTo(Target::Clip { id: 10, seq: None, t: 2.0 }));
        assert_eq!(first(&goto_rows(&idx, &[], "drone"), "Clips"), Command::GoTo(Target::Clip { id: 21, seq: Some(30), t: 4.5 }));
        p.close_sequence();
        // a timeline opens
        let pl = plan(&Target::Timeline(Some(30)), false, &p, 0.0);
        assert_eq!((pl.timeline, pl.pane), (Some(Some(30)), Some(Pane::Timeline)));
        // a marker: its timeline, its time, selected
        let pl = plan(&Target::Marker { id: 50, seq: Some(30), t: 6.0 }, false, &p, 0.0);
        assert_eq!((pl.timeline, pl.seek, pl.select_marker), (Some(Some(30)), Some(6.0), Some(50)));
        // an asset: revealed and selected in its bin; Alt+Enter adds it at the playhead
        let pl = plan(&Target::Asset(40), false, &p, 0.0);
        assert_eq!((pl.library, pl.pane), (Some((Some(40), "Footage/Day 1".into())), Some(Pane::Library)));
        let pl = plan(&Target::Asset(40), true, &p, 7.5);
        assert_eq!(pl.tool, Some(("timeline.place", serde_json::json!({"asset_id": 40, "at": 7.5}))));
        assert!(crate::mcp::tools::find("timeline.place").is_some());
        // a bin shows its folder
        assert_eq!(plan(&Target::Bin("Footage/Day 1".into()), false, &p, 0.0).library, Some((None, "Footage/Day 1".into())));
        // a page switches
        assert_eq!(plan(&Target::Page("Color".into()), false, &p, 0.0).page, Some("Color".into()));
        // a Gallery card is revealed on its tab; Alt+Enter applies it
        let g = Target::Gallery(GalleryTab::Looks, "Teal Orange".into());
        assert_eq!(plan(&g, false, &p, 0.0).gallery, Some((GalleryTab::Looks, "Teal Orange".into())));
        assert_eq!(plan(&g, true, &p, 0.0).tool.map(|t| t.0), Some("gallery.apply"));
        assert!(crate::mcp::tools::find("gallery.apply").is_some());
    }

    #[test]
    fn settings_toggles_are_commands() {
        let rs = rows(&snap(|_| Ok(())), toggles, &[], &[("snap", "Snapping", true)], &[], "snapping");
        let r = rs.iter().find(|r| matches!(r.cmd, Command::Setting(..))).expect("a setting row");
        assert_eq!(r.cmd, Command::Setting("snap", false));
        assert_eq!(r.label, "Snapping: turn off");
    }

    /// Tab cycles the rows (wrapping) and keeps the box focused; Alt+Enter returns the secondary action.
    #[test]
    fn tab_cycles_and_alt_enter_is_the_secondary_action() {
        let ctx = egui::Context::default();
        let p = seeded();
        let idx = index(&p, &[], &[], &Default::default());
        let rs = goto_rows(&idx, &[], "sunset");
        let mut state = PaletteState { open: true, query: "sunset".into(), ..Default::default() };
        let key = |key, modifiers| {
            let mut raw = egui::RawInput::default();
            raw.modifiers = modifiers;
            raw.events.push(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
            raw
        };
        let _ = ctx.run(egui::RawInput::default(), |ctx| assert!(show(ctx, &mut state, &rs).is_none()));
        let n = rs.len();
        let _ = ctx.run(key(egui::Key::Tab, egui::Modifiers::NONE), |ctx| assert!(show(ctx, &mut state, &rs).is_none()));
        assert_eq!(state.sel, 1 % n);
        let _ = ctx.run(key(egui::Key::Tab, egui::Modifiers::SHIFT), |ctx| assert!(show(ctx, &mut state, &rs).is_none()));
        assert_eq!(state.sel, 0);
        assert!(ctx.memory(|m| m.has_focus(egui::Id::new("command_palette_query"))), "Tab must not leave the box");
        let mut got = None;
        let _ = ctx.run(key(egui::Key::Enter, egui::Modifiers::ALT), |ctx| got = show(ctx, &mut state, &rs));
        assert_eq!(got, Some((Command::GoTo(Target::Asset(40)), true)));
    }

    /// Ctrl+P opens the palette by default; Ctrl+K stays free for its alias.
    #[test]
    fn ctrl_p_is_bound_by_default() {
        let hk = Hotkeys::defaults();
        let ctrl_p = egui::KeyboardShortcut::new(egui::Modifiers::CTRL, egui::Key::P);
        assert_eq!(hk.conflict(ctrl_p), Some(Action::CommandPalette));
        assert_eq!(hk.text(Action::CommandPalette), "Ctrl+P");
        assert_eq!(hk.conflict(egui::KeyboardShortcut::new(egui::Modifiers::CTRL, egui::Key::K)), None);
    }

    /// A user who never rebound the palette gets Ctrl+P; one who bound Ctrl+P to something else keeps
    /// it, and the palette stays on Ctrl+K.
    #[test]
    fn ctrl_p_migration_respects_the_users_own_bindings() {
        let s = crate::settings::Settings::default();
        assert_eq!(Hotkeys::from_settings(&s).text(Action::CommandPalette), "Ctrl+P");
        let mut s = crate::settings::Settings::default();
        s.hotkeys.insert(Action::Undo.id().to_string(), "Ctrl+P".into());
        let hk = Hotkeys::from_settings(&s);
        assert_eq!(hk.text(Action::Undo), "Ctrl+P");
        assert_eq!(hk.text(Action::CommandPalette), "Ctrl+K");
        // an explicit choice for the palette itself is kept as is
        s.hotkeys.insert(Action::CommandPalette.id().to_string(), String::new());
        assert_eq!(Hotkeys::from_settings(&s).text(Action::CommandPalette), "");
    }

    /// Idle frames with the palette open in each mode (and rows from a real index) ask for no repaint.
    #[test]
    fn assert_no_idle_repaint_palette_each_mode() {
        let p = seeded();
        let idx = index(&p, &["Edit".into()], &[], &Default::default());
        let skills = vec![skill("captions", "[style]", "Add captions")];
        for q in ["", "intro", ">undo", "/", "/ask hi", "/captions"] {
            let ctx = egui::Context::default();
            let mut state = PaletteState { open: true, query: q.into(), ..Default::default() };
            let rs = match mode(q) {
                Mode::GoTo(t) => goto_rows(&idx, &[], t),
                Mode::Commands(t) => rows(&snap(|_| Ok(())), toggles, &[], &[], &[], t),
                Mode::Tools(t) => tool_rows(t),
                Mode::Assistant(t) => assistant_rows(t, &skills, ask_row),
            };
            for _ in 0..30 {
                let _ = ctx.run(egui::RawInput::default(), |ctx| {
                    let _ = show(ctx, &mut state, &rs);
                });
            }
            assert!(!ctx.has_requested_repaint(), "idle palette ({q:?}) requested a repaint");
        }
    }
}
