//! Settings ▸ Hotkeys ▸ "Import from…": read another editor's preference files and turn whatever has an
//! equivalent here into a `Plan` the confirm dialog lists and `apply` executes.
//!  * Premiere Pro: `Documents\Adobe\Premiere Pro\<ver>\Profile-*\Win\*.kys` (plain XML: `<item.N>` with
//!    `<virtualkey>`, `<modifier.ctrl|alt|shift>`, `<commandname>`) and the sibling
//!    `Adobe Premiere Pro Prefs` (XML key/values; the autosave/snap/playback-resolution keys are matched
//!    by name - Adobe doesn't document them).
//!  * DaVinci Resolve: an exported keyboard preset `.txt` (`name := Ctrl+B | Alt+B` per line). The live
//!    `Preferences\keyboard.preset.xml` is undocumented, so it is reported, not guessed at.
//!  * UI scale: none of the three stores one in a readable file, so nothing maps to `ui_scale`.
//!  * CapCut: no documented preference/shortcut file exists - detected and reported as unreadable.
//!
//! No custom keyboard file → the built-in `keymaps::PRESETS` row for that app, when there is one.

use crate::hotkeys::{Action, Hotkeys};
use crate::settings::Settings;
use eframe::egui::{Key, KeyboardShortcut, Modifiers};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum App {
    Premiere,
    Resolve,
    CapCut,
}

impl App {
    pub const ALL: [App; 3] = [App::Premiere, App::Resolve, App::CapCut];
    pub fn name(self) -> &'static str {
        match self {
            App::Premiere => "Premiere Pro",
            App::Resolve => "DaVinci Resolve",
            App::CapCut => "CapCut",
        }
    }
    fn preset(self) -> Option<&'static str> {
        match self {
            App::Premiere => Some("Premiere"),
            App::Resolve => Some("Resolve"),
            App::CapCut => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Change {
    AutosaveSecs(u32),
    Snap(bool),
    PreviewQuality(u32),
}

impl Change {
    pub fn label(self) -> String {
        match self {
            Change::AutosaveSecs(s) => format!("Autosave interval → {s} s"),
            Change::Snap(b) => format!("Snapping → {}", if b { "on" } else { "off" }),
            Change::PreviewQuality(q) => format!("Preview render quality → {q}%"),
        }
    }
}

#[derive(Default, Debug)]
pub struct Plan {
    pub app: Option<App>,
    pub sources: Vec<PathBuf>,
    /// (our action, chord, their command name)
    pub keys: Vec<(Action, KeyboardShortcut, String)>,
    /// built-in preset applied instead of `keys` (no custom keyboard file found)
    pub preset: Option<&'static str>,
    pub settings: Vec<Change>,
    /// their bindings / settings with no equivalent here
    pub skipped: Vec<String>,
    /// honest "couldn't read X" lines
    pub notes: Vec<String>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty() && self.preset.is_none() && self.settings.is_empty()
    }
}

/// What `apply` overwrote - "Undo import" puts exactly these back.
pub struct Snapshot {
    hotkeys: std::collections::BTreeMap<String, String>,
    keymap_preset: String,
    autosave_secs: u32,
    snap: bool,
    preview_quality: u32,
}

impl Snapshot {
    pub fn restore(self, s: &mut Settings, hk: &mut Hotkeys) {
        s.hotkeys = self.hotkeys;
        *hk = Hotkeys::from_settings(s);
        s.keymap_preset = self.keymap_preset;
        s.autosave_secs = self.autosave_secs;
        s.snap = self.snap;
        s.preview_quality = self.preview_quality;
    }
}

pub fn apply(plan: &Plan, s: &mut Settings, hk: &mut Hotkeys) -> Snapshot {
    hk.to_settings(s);
    let snap = Snapshot {
        hotkeys: s.hotkeys.clone(),
        keymap_preset: s.keymap_preset.clone(),
        autosave_secs: s.autosave_secs,
        snap: s.snap,
        preview_quality: s.preview_quality,
    };
    if let Some(p) = plan.preset {
        if super::apply(p, hk).is_ok() {
            s.keymap_preset = p.to_string();
        }
    }
    for &(a, ks, _) in &plan.keys {
        hk.set(a, Some(ks));
    }
    for &c in &plan.settings {
        match c {
            Change::AutosaveSecs(v) => s.autosave_secs = v,
            Change::Snap(v) => s.snap = v,
            Change::PreviewQuality(v) => s.preview_quality = v,
        }
    }
    hk.to_settings(s);
    snap
}

// ---------------------------------------------------------------- detection

fn env(k: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(k).unwrap_or_default())
}

fn newest(paths: impl Iterator<Item = PathBuf>) -> Option<PathBuf> {
    paths.max_by_key(|p| p.metadata().and_then(|m| m.modified()).ok())
}

fn dir(p: &Path) -> impl Iterator<Item = PathBuf> {
    std::fs::read_dir(p).into_iter().flatten().flatten().map(|e| e.path())
}

/// Premiere: (newest .kys, newest prefs file). Walks `<ver>\Profile-*\` - two levels, no recursion.
fn premiere_files() -> (Option<PathBuf>, Option<PathBuf>) {
    let root = env("USERPROFILE").join("Documents").join("Adobe").join("Premiere Pro");
    let profiles: Vec<PathBuf> = dir(&root)
        .flat_map(|v| dir(&v))
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("Profile-")))
        .collect();
    let kys =
        newest(profiles.iter().flat_map(|p| dir(&p.join("Win"))).filter(|p| p.extension().is_some_and(|e| e == "kys")));
    let prefs = newest(profiles.iter().map(|p| p.join("Adobe Premiere Pro Prefs")).filter(|p| p.is_file()));
    (kys, prefs)
}

fn resolve_dir() -> PathBuf {
    env("APPDATA").join("Blackmagic Design").join("DaVinci Resolve").join("Preferences")
}

fn capcut_dir() -> PathBuf {
    env("LOCALAPPDATA").join("CapCut").join("User Data")
}

/// Is the app's preference folder present? (drives the "(found)" hint on the menu)
pub fn installed(app: App) -> bool {
    match app {
        App::Premiere => env("USERPROFILE").join("Documents").join("Adobe").join("Premiere Pro").is_dir(),
        App::Resolve => resolve_dir().is_dir(),
        App::CapCut => capcut_dir().is_dir(),
    }
}

/// Auto-detect `app`'s files and build a plan from them.
pub fn detect(app: App) -> Plan {
    let mut plan = Plan { app: Some(app), ..Default::default() };
    match app {
        App::Premiere => {
            let (kys, prefs) = premiere_files();
            if let Some(p) = &prefs {
                read_into(&mut plan, p, premiere_prefs);
            }
            match kys {
                Some(k) => read_into(&mut plan, &k, premiere_kys),
                None => plan.notes.push("No custom Premiere .kys found.".into()),
            }
        }
        App::Resolve => {
            let d = resolve_dir();
            let txt = newest(dir(&d).filter(|p| p.extension().is_some_and(|e| e == "txt")));
            match txt {
                Some(t) => read_into(&mut plan, &t, resolve_txt),
                None if d.join("keyboard.preset.xml").is_file() => plan.notes.push(
                    "Found keyboard.preset.xml, but its format is undocumented and isn't read. \
                     In Resolve use Keyboard Customization ▸ ⋯ ▸ Export Preset, then Choose file…"
                        .into(),
                ),
                None => plan.notes.push("No exported Resolve keyboard preset (.txt) found.".into()),
            }
            if d.is_dir() {
                plan.notes.push("Resolve's other preferences (config.dat etc.) are binary - not imported.".into());
            }
        }
        App::CapCut => plan.notes.push(capcut_note()),
    }
    finish(plan)
}

fn capcut_note() -> String {
    "CapCut keeps no documented preference or shortcut file (its User Data folder holds projects and \
     caches), so nothing can be imported from it."
        .into()
}

/// "Choose file…": dispatch on content, not only extension.
pub fn from_file(path: &Path) -> Plan {
    let mut plan = Plan::default();
    let text = match std::fs::read(path) {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(e) => {
            plan.notes.push(format!("Couldn't read {}: {e}", path.display()));
            return plan;
        }
    };
    plan.app = Some(if text.contains("<commandname>") {
        App::Premiere
    } else if text.contains(":=") {
        App::Resolve
    } else if text.contains("<PremiereData") {
        App::Premiere
    } else {
        plan.notes.push(format!("{} isn't a Premiere .kys/prefs or Resolve keyboard .txt file.", path.display()));
        return plan;
    });
    plan.sources.push(path.to_path_buf());
    if text.contains("<commandname>") {
        premiere_kys(&mut plan, &text);
    } else if text.contains(":=") {
        resolve_txt(&mut plan, &text);
    } else {
        premiere_prefs(&mut plan, &text);
    }
    plan
}

fn read_into(plan: &mut Plan, p: &Path, f: fn(&mut Plan, &str)) {
    match std::fs::read(p) {
        Ok(b) if b.contains(&0) => plan.notes.push(format!("{} is binary - not read.", p.display())),
        Ok(b) => {
            plan.sources.push(p.to_path_buf());
            f(plan, &String::from_utf8_lossy(&b));
        }
        Err(e) => plan.notes.push(format!("Couldn't read {}: {e}", p.display())),
    }
}

/// No keyboard bindings came out of the files → fall back to the built-in preset.
fn finish(mut plan: Plan) -> Plan {
    if plan.keys.is_empty() {
        plan.preset = plan.app.and_then(App::preset);
    }
    plan
}

// ---------------------------------------------------------------- Premiere

/// Premiere command name → our action id. ponytail: hand-picked common commands, not all ~900 -
/// extend the table when a user reports a missing one.
const PREMIERE: &[(&str, &str)] = &[
    ("cmd.file.new.project", "new_project"),
    ("cmd.file.open", "open_project"),
    ("cmd.file.save", "save"),
    ("cmd.file.saveas", "save_project_as"),
    ("cmd.file.import", "import"),
    ("cmd.file.export.media", "export"),
    ("cmd.edit.undo", "undo"),
    ("cmd.edit.redo", "redo"),
    ("cmd.edit.selectall", "select_all"),
    ("cmd.edit.deselectall", "deselect"),
    ("cmd.edit.clear", "delete"),
    ("cmd.edit.rippledelete", "ripple_delete"),
    ("cmd.clip.addedit", "split"),
    ("cmd.clip.link", "link"),
    ("cmd.clip.enable", "toggle_enabled"),
    ("cmd.transport.play.stop", "play_pause"),
    ("cmd.transport.shuttle.stop", "stop"),
    ("cmd.transport.step.back", "step_back"),
    ("cmd.transport.step.forward", "step_fwd"),
    ("cmd.transport.goto.start", "go_start"),
    ("cmd.transport.goto.end", "go_end"),
    ("cmd.transport.goto.prev.edit", "prev_cut"),
    ("cmd.transport.goto.next.edit", "next_cut"),
    ("cmd.marker.mark.in", "mark_in"),
    ("cmd.marker.mark.out", "mark_out"),
    ("cmd.marker.clear.inout", "clear_in_out"),
    ("cmd.marker.add", "add_marker"),
    ("cmd.sequence.snap", "snap"),
    ("cmd.sequence.apply.default.video.transition", "add_last_transition"),
    ("cmd.timeline.zoom.in", "zoom_in"),
    ("cmd.timeline.zoom.out", "zoom_out"),
    ("cmd.timeline.zoom.to.sequence", "zoom_fit"),
    ("cmd.clip.speed", "retime"),
    ("cmd.clip.nest", "nest"),
];

/// Text of every `<tag>text</tag>` leaf, in order. ponytail: flat scan, no XML crate - both Adobe files
/// are simple leaf-per-line XML.
fn leaves(xml: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(i) = rest.find('<') {
        rest = &rest[i + 1..];
        let Some(j) = rest.find('>') else { break };
        let tag = rest[..j].split_whitespace().next().unwrap_or("");
        let body = &rest[j + 1..];
        if tag.starts_with(['/', '?', '!']) {
            continue;
        }
        if let Some(k) = body.find('<') {
            if body[k..].starts_with(&format!("</{tag}>")) {
                out.push((tag, body[..k].trim()));
            }
        }
    }
    out
}

/// Premiere's `<virtualkey>`: a character code with the top bit set (0x80000047 = 'G') or a plain
/// Windows virtual-key code (0x25 = Left).
fn premiere_key(vk: u64) -> Option<Key> {
    let c = (vk & 0xFFFF) as u32;
    let named = match c {
        0x08 => "Backspace",
        0x09 => "Tab",
        0x0D => "Enter",
        0x1B => "Escape",
        0x20 => "Space",
        0x21 => "PageUp",
        0x22 => "PageDown",
        0x23 => "End",
        0x24 => "Home",
        0x25 => "ArrowLeft",
        0x26 => "ArrowUp",
        0x27 => "ArrowRight",
        0x28 => "ArrowDown",
        0x2D => "Insert",
        0x2E if vk & 0x8000_0000 == 0 => "Delete",
        0x70..=0x7B if vk & 0x8000_0000 == 0 => return Key::from_name(&format!("F{}", c - 0x6F)),
        0xBB => "=",
        0xBC => ",",
        0xBD => "-",
        0xBE => ".",
        _ => return char::from_u32(c).and_then(|ch| symbol_key(ch.to_ascii_uppercase())),
    };
    Key::from_name(named).or_else(|| named.chars().next().and_then(symbol_key))
}

fn symbol_key(ch: char) -> Option<Key> {
    Some(match ch {
        ',' => Key::Comma,
        '.' => Key::Period,
        '=' => Key::Equals,
        '-' => Key::Minus,
        '/' => Key::Slash,
        ';' => Key::Semicolon,
        '\\' => Key::Backslash,
        '[' => Key::OpenBracket,
        ']' => Key::CloseBracket,
        '`' => Key::Backtick,
        ' ' => Key::Space,
        _ => return Key::from_name(&ch.to_string()),
    })
}

fn premiere_kys(plan: &mut Plan, xml: &str) {
    let (mut vk, mut m) = (None, Modifiers::NONE);
    for (tag, v) in leaves(xml) {
        let on = v == "true";
        match tag {
            "virtualkey" => (vk, m) = (v.parse::<u64>().ok(), Modifiers::NONE),
            "modifier.ctrl" if on => m = m.plus(Modifiers::CTRL),
            "modifier.shift" if on => m = m.plus(Modifiers::SHIFT),
            "modifier.alt" if on => m = m.plus(Modifiers::ALT),
            "commandname" => {
                // an item without <virtualkey> is an explicit unbind - nothing to import
                if let Some(code) = vk.take() {
                    let chord = premiere_key(code).map(|k| KeyboardShortcut::new(m, k));
                    push_key(plan, v, chord, PREMIERE);
                }
                m = Modifiers::NONE;
            }
            _ => {}
        }
    }
}

fn premiere_prefs(plan: &mut Plan, xml: &str) {
    for (tag, v) in leaves(xml) {
        let t = tag.to_ascii_lowercase();
        if t.contains("autosave") && t.contains("interval") {
            if let Ok(min) = v.parse::<u32>() {
                plan.settings.push(Change::AutosaveSecs(min * 60));
            }
        } else if t.contains("snap") && (v == "true" || v == "false") {
            plan.settings.push(Change::Snap(v == "true"));
        } else if t.contains("playbackresolution") {
            // Full, 1/2, 1/4, 1/8, 1/16 - as text or as that list's index
            let q = match v {
                "Full" | "0" => 100,
                "1/2" | "1" => 50,
                "1/4" | "2" => 25,
                _ => continue,
            };
            plan.settings.push(Change::PreviewQuality(q));
        }
    }
    plan.skipped.push("Premiere default sequence size/fps: Simple Editor sizes a project from its first clip".into());
}

// ---------------------------------------------------------------- Resolve

const RESOLVE: &[(&str, &str)] = &[
    ("fileNewProject", "new_project"),
    ("fileSaveProject", "save"),
    ("fileImportMedia", "import"),
    ("editUndo", "undo"),
    ("editRedo", "redo"),
    ("editSelectAll", "select_all"),
    ("editDeselectAll", "deselect"),
    ("editBladeRazor", "split"),
    ("editDelete", "delete"),
    ("editDeleteGaps", "ripple_delete"),
    ("editRippleDelete", "ripple_delete"),
    ("editSnapping", "snap"),
    ("editLinkedSelection", "link"),
    ("editEnableClip", "toggle_enabled"),
    ("editAddTransition", "add_last_transition"),
    ("editInsertTitle", "add_text"),
    ("controlPlayForward", "play_pause"),
    ("controlStop", "stop"),
    ("controlPrevFrame", "step_back"),
    ("controlNextFrame", "step_fwd"),
    ("controlFirstFrame", "go_start"),
    ("controlLastFrame", "go_end"),
    ("controlPrevEdit", "prev_cut"),
    ("controlNextEdit", "next_cut"),
    ("markIn", "mark_in"),
    ("markOut", "mark_out"),
    ("markInOutClear", "clear_in_out"),
    ("markAddMarker", "add_marker"),
    ("viewZoomIn", "zoom_in"),
    ("viewZoomOut", "zoom_out"),
    ("viewZoomToFit", "zoom_fit"),
];

/// "Ctrl+Shift+." / "Del" / "PgUp" / "Num+8" → a chord (first alternative of `a | b`).
fn resolve_chord(text: &str) -> Option<KeyboardShortcut> {
    let first = text.split('|').next()?.trim();
    if first.is_empty() {
        return None;
    }
    // split on '+' but keep a trailing literal "+" key
    let (mods, key) = match first.rsplit_once('+') {
        Some((m, "")) => (m.trim_end_matches('+'), "+"),
        Some((m, k)) => (m, k),
        None => ("", first),
    };
    let mut m = Modifiers::NONE;
    for p in mods.split('+').filter(|p| !p.is_empty()) {
        match p {
            "Ctrl" | "Cmd" => m = m.plus(Modifiers::CTRL),
            "Shift" => m = m.plus(Modifiers::SHIFT),
            "Alt" | "Opt" => m = m.plus(Modifiers::ALT),
            "Num" => {} // keypad digit - egui doesn't tell them apart
            _ => return None,
        }
    }
    let k = match key {
        "Del" => Key::Delete,
        "PgUp" => Key::PageUp,
        "PgDown" | "PgDn" => Key::PageDown,
        "Left" => Key::ArrowLeft,
        "Right" => Key::ArrowRight,
        "Up" => Key::ArrowUp,
        "Down" => Key::ArrowDown,
        "Return" => Key::Enter,
        "Esc" => Key::Escape,
        k if k.chars().count() == 1 => symbol_key(k.chars().next()?.to_ascii_uppercase())?,
        k => Key::from_name(k)?,
    };
    Some(KeyboardShortcut::new(m, k))
}

fn resolve_txt(plan: &mut Plan, text: &str) {
    for line in text.lines() {
        let Some((name, chord)) = line.split_once(":=") else { continue };
        let (name, chord) = (name.trim(), chord.trim());
        if chord.is_empty() {
            continue; // unbound in their preset
        }
        push_key(plan, name, resolve_chord(chord), RESOLVE);
    }
}

// ---------------------------------------------------------------- shared

fn push_key(plan: &mut Plan, their: &str, chord: Option<KeyboardShortcut>, table: &[(&str, &str)]) {
    let ours = table.iter().find(|(t, _)| *t == their).and_then(|(_, id)| Action::from_id(id));
    let reserved = |k| matches!(Hotkeys::defaults().conflict_all(k), Some(crate::hotkeys::Claim::Fixed(_)));
    match (ours, chord) {
        (Some(a), Some(k)) if !reserved(k) => {
            // the same action bound twice (global + timeline context): first one wins
            if !plan.keys.iter().any(|(b, ..)| *b == a) {
                plan.keys.push((a, k, their.to_string()));
            }
        }
        (Some(_), Some(k)) => plan.skipped.push(format!("{their} ({}): reserved here", Hotkeys::format(&k))),
        (Some(_), None) => plan.skipped.push(format!("{their}: key not understood")),
        (None, _) => {
            let k = chord.map(|k| Hotkeys::format(&k)).unwrap_or_default();
            if !plan.skipped.iter().any(|s| s.starts_with(their)) {
                plan.skipped.push(format!("{their} {k}: no equivalent"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KYS: &str = r#"<?xml version="1.0" encoding="UTF-8" ?>
<PremiereData Version="3">
<context.global Version="1">
  <item.1 Version="1">
    <virtualkey>2147483723</virtualkey>
    <modifier.ctrl>true</modifier.ctrl>
    <modifier.alt>false</modifier.alt>
    <modifier.shift>false</modifier.shift>
    <commandname>cmd.clip.addedit</commandname>
  </item.1>
  <item.2 Version="1">
    <virtualkey>2147483719</virtualkey>
    <modifier.ctrl>false</modifier.ctrl>
    <modifier.alt>false</modifier.alt>
    <modifier.shift>false</modifier.shift>
    <commandname>cmd.timeline.show.direct.clip.manipulation</commandname>
  </item.2>
  <item.3 Version="1">
    <virtualkey>37</virtualkey>
    <modifier.ctrl>false</modifier.ctrl>
    <modifier.alt>false</modifier.alt>
    <modifier.shift>true</modifier.shift>
    <commandname>cmd.transport.goto.prev.edit</commandname>
  </item.3>
</context.global>
<context.timeline Version="1">
  <item.4 Version="1">
    <commandname>cmd.clip.addedit</commandname>
  </item.4>
</context.timeline>
</PremiereData>"#;

    const PREFS: &str = r#"<PremiereData Version="3">
  <BE.Prefs.AutoSave.Enabled>true</BE.Prefs.AutoSave.Enabled>
  <BE.Prefs.AutoSave.Interval>5</BE.Prefs.AutoSave.Interval>
  <BE.Prefs.Timeline.Snap>false</BE.Prefs.Timeline.Snap>
  <MZ.Prefs.PlaybackResolution>1/2</MZ.Prefs.PlaybackResolution>
</PremiereData>"#;

    const RESOLVE_TXT: &str = "editBladeRazor := Ctrl+K\neditBlade :=\nmarkIn := I\n\
        controlPrevEdit := Shift+Left | Up\nnodesAddPCW := Alt+B\neditNudge := Shift+.\n";

    #[test]
    fn premiere_kys_maps_split_and_arrows_and_reports_unmapped() {
        let mut p = Plan::default();
        premiere_kys(&mut p, KYS);
        let k = |a| p.keys.iter().find(|(b, ..)| *b == a).map(|(_, k, _)| Hotkeys::format(k));
        assert_eq!(k(Action::Split).as_deref(), Some("Ctrl+K"));
        assert_eq!(k(Action::PrevCut).as_deref(), Some("Shift+Left"));
        assert_eq!(p.keys.len(), 2, "the key-less timeline item is an unbind, not a second binding");
        assert!(p.skipped.iter().any(|s| s.contains("direct.clip.manipulation")));
    }

    #[test]
    fn premiere_prefs_read_autosave_snap_and_playback_res() {
        let mut p = Plan::default();
        premiere_prefs(&mut p, PREFS);
        assert!(p.settings.contains(&Change::AutosaveSecs(300)));
        assert!(p.settings.contains(&Change::Snap(false)));
        assert!(p.settings.contains(&Change::PreviewQuality(50)));
    }

    #[test]
    fn resolve_txt_maps_first_alternative_and_skips_blank() {
        let mut p = Plan::default();
        resolve_txt(&mut p, RESOLVE_TXT);
        let k = |a| p.keys.iter().find(|(b, ..)| *b == a).map(|(_, k, _)| Hotkeys::format(k));
        assert_eq!(k(Action::Split).as_deref(), Some("Ctrl+K"));
        assert_eq!(k(Action::MarkIn).as_deref(), Some("I"));
        assert_eq!(k(Action::PrevCut).as_deref(), Some("Shift+Left"));
        assert!(p.skipped.iter().any(|s| s.starts_with("nodesAddPCW")));
        assert!(!p.skipped.iter().any(|s| s.starts_with("editBlade ")), "unbound rows aren't reported");
    }

    #[test]
    fn apply_then_restore_round_trips() {
        let mut s = Settings::default();
        let mut hk = Hotkeys::from_settings(&s);
        let before = (hk.text(Action::Split), s.autosave_secs, s.snap);
        let mut plan = Plan::default();
        resolve_txt(&mut plan, RESOLVE_TXT);
        plan.settings.push(Change::AutosaveSecs(999));
        plan.settings.push(Change::Snap(!s.snap));
        let snap = apply(&plan, &mut s, &mut hk);
        assert_eq!(hk.text(Action::Split), "Ctrl+K");
        assert_eq!(s.autosave_secs, 999);
        snap.restore(&mut s, &mut hk);
        assert_eq!((hk.text(Action::Split), s.autosave_secs, s.snap), before);
    }

    #[test]
    fn from_file_sniffs_format_and_falls_back_to_preset() {
        let dir = std::env::temp_dir().join(format!("se-prefs-import-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("mine.kys");
        std::fs::write(&f, KYS).unwrap();
        let p = from_file(&f);
        assert_eq!(p.app, Some(App::Premiere));
        assert!(!p.keys.is_empty());
        let junk = dir.join("x.bin");
        std::fs::write(&junk, [0u8, 1, 2]).unwrap();
        assert!(from_file(&junk).is_empty());
        assert_eq!(finish(Plan { app: Some(App::Resolve), ..Default::default() }).preset, Some("Resolve"));
        assert_eq!(finish(Plan { app: Some(App::CapCut), ..Default::default() }).preset, None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
