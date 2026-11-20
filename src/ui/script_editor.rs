//! ---- feat/scripts-editor ----
//! The Script Editor pane (`Pane::ScriptEditor`): one Luau script from the scripts folder in a code editor
//! with Luau colours (the HTML editor's palette, `html_code::colour`), Save (Ctrl+S while the editor has
//! focus), Run, Open Externally and the last run's result line. An unsaved change shows a dot by the name;
//! a file changed on disk reloads by itself, or offers Reload when there are unsaved edits here.
//! Also the Window ▸ Scripts menu (`scripts_menu`): per script Run / Open / Open Folder / Rename… /
//! Delete…, then Add Script and Open Scripts Folder. Nothing here touches the App: the caller
//! (`ui::app::scripts_ctl`) does the work each `Act` / `MenuAct` names.

use crate::scripting::ScriptMeta;
use crate::scripting_api::Ctx;
use crate::ui::html_code::{colour, Kind};
use crate::ui::script_intel::{self as intel, Analysis, Completion, Diag, SymKind};
use std::sync::{Arc, Mutex};
use crate::ui::{menu, tools};
use eframe::egui::{self, text::LayoutJob, text::TextFormat, Color32, FontId, Key, Modifiers};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

// ---------------------------------------------------------------------------------------------------
// Luau colours

const KEYWORDS: &[&str] = &[
    "and", "break", "continue", "do", "else", "elseif", "end", "export", "false", "for", "function", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "type", "until", "while",
];
/// `[[`, `[=[`, `[==[`... at `i`: the number of `=` (the level), or `None`.
pub(crate) fn long_open(b: &[u8], i: usize) -> Option<usize> {
    if b.get(i) != Some(&b'[') {
        return None;
    }
    let mut j = i + 1;
    while b.get(j) == Some(&b'=') {
        j += 1;
    }
    (b.get(j) == Some(&b'[')).then_some(j - i - 1)
}

/// The end (exclusive) of a long bracket of `level` whose body starts at `from` (the whole rest when unclosed).
pub(crate) fn long_close(text: &str, from: usize, level: usize) -> usize {
    let close = format!("]{}]", "=".repeat(level));
    text.get(from..).and_then(|t| t.find(&close)).map_or(text.len(), |k| from + k + close.len())
}

/// Every coloured stretch of a Luau source, in order (what is not listed is plain text).
pub fn tokens(text: &str) -> Vec<(Range<usize>, Kind)> {
    let b = text.as_bytes();
    let n = b.len();
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        let c = b[i];
        // comments: --[[ long ]] or to the end of the line; a header's `@key` stands out
        if c == b'-' && b.get(i + 1) == Some(&b'-') {
            let s = i;
            if let Some(level) = long_open(b, i + 2) {
                i = long_close(text, i + 2 + level + 2, level);
                out.push((s..i, Kind::Comment));
                continue;
            }
            i = text[i..].find('\n').map_or(n, |k| i + k);
            let line = &text[s..i];
            match line[2..].trim_start().strip_prefix('@').map(|r| r.find(char::is_whitespace).unwrap_or(r.len())) {
                Some(len) => {
                    let at = s + line.find('@').unwrap_or(2);
                    out.push((s..at, Kind::Comment));
                    out.push((at..at + 1 + len, Kind::AtRule));
                    out.push((at + 1 + len..i, Kind::Comment));
                }
                None => out.push((s..i, Kind::Comment)),
            }
            continue;
        }
        // strings: "…" '…' `…` with escapes, [[ long ]]
        if c == b'"' || c == b'\'' || c == b'`' {
            let s = i;
            i += 1;
            while i < n && b[i] != c && b[i] != b'\n' {
                i += if b[i] == b'\\' { 2 } else { 1 };
            }
            i = (i + 1).min(n);
            out.push((s..i, Kind::Str));
            continue;
        }
        if let Some(level) = long_open(b, i) {
            let s = i;
            i = long_close(text, i + level + 2, level);
            out.push((s..i, Kind::Str));
            continue;
        }
        // numbers: 12, 1.5e3, 0xFF, 1_000, .5
        if c.is_ascii_digit() || (c == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit)) {
            let s = i;
            i += 1;
            while i < n && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'.') {
                // an exponent's sign belongs to the number
                if (b[i] == b'e' || b[i] == b'E')
                    && matches!(b.get(i + 1), Some(b'+' | b'-'))
                    && !text[s..i].starts_with("0x")
                {
                    i += 1;
                }
                i += 1;
            }
            out.push((s..i, Kind::Num));
            continue;
        }
        // names: keywords, the API's globals, calls
        if c.is_ascii_alphabetic() || c == b'_' {
            let s = i;
            while i < n && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            let word = &text[s..i];
            let after = text[i..].trim_start_matches([' ', '\t']).as_bytes().first().copied();
            let before = text[..s].trim_end_matches([' ', '\t']).as_bytes().last().copied();
            let member = matches!(before, Some(b'.' | b':'));
            let kind = if KEYWORDS.contains(&word) && !member {
                Some(Kind::Keyword)
            } else if !member && crate::scripting_api::globals(crate::scripting_api::ALL).any(|g| g.name == word) {
                Some(Kind::Tag)
            } else if matches!(after, Some(b'(' | b'{' | b'"' | b'\'')) {
                Some(Kind::Func)
            } else if member {
                Some(Kind::Prop)
            } else {
                None
            };
            if let Some(k) = kind {
                out.push((s..i, k));
            }
            continue;
        }
        if c.is_ascii_punctuation() {
            out.push((i..i + 1, Kind::Punct));
        }
        // step over a whole character (a multi-byte one is plain text)
        i += text[i..].chars().next().map_or(1, char::len_utf8);
    }
    out
}

/// The coloured layout of a Luau source (every byte of it is in exactly one section, in order).
pub fn highlight(text: &str, font: &FontId, dark: bool, base: Color32) -> LayoutJob {
    let mut kinds = vec![Kind::Plain; text.len()];
    for (r, k) in tokens(text) {
        for x in &mut kinds[r.start.min(text.len())..r.end.min(text.len())] {
            *x = k;
        }
    }
    let mut job = LayoutJob::default();
    let mut s = 0;
    for i in 1..=text.len() {
        if i == text.len() || (kinds[i] != kinds[s] && text.is_char_boundary(i)) {
            job.append(&text[s..i], 0.0, TextFormat::simple(font.clone(), colour(kinds[s], dark, base)));
            s = i;
        }
    }
    job
}

// ---------------------------------------------------------------------------------------------------
// the pane

/// What the pane holds: the open file, its text, the text as last saved, and the last run's result.
#[derive(Default)]
pub struct ScriptEditor {
    pub path: Option<PathBuf>,
    pub text: String,
    /// The file's text as on disk (the unsaved dot is `text != saved`).
    pub saved: String,
    /// The file's modified time when it was loaded / saved: a different one on disk means another program
    /// changed it.
    pub disk_time: Option<SystemTime>,
    /// When the disk was last looked at (at most once a second while the pane is drawn).
    pub checked: Option<Instant>,
    /// The file changed on disk while there were unsaved edits here: Reload offers the disk's version.
    pub conflict: bool,
    /// The last run's result line (empty until something ran) and whether it is an error.
    pub output: String,
    pub output_err: bool,
    /// The menu's Rename… box, while it is open.
    pub rename: Option<Rename>,
    // ---- feat/script-intellisense ----
    /// The open file belongs to a plugin (completion and checks use the plugin API).
    pub plugin: bool,
    /// Completion, signature help, hover and diagnostics state.
    pub intel: Intel,
}

/// The floating name field Rename… opens beside the menu.
pub struct Rename {
    pub path: PathBuf,
    pub name: String,
    pub error: String,
    pub at: egui::Pos2,
    /// Focus the field on its first frame.
    pub focus: bool,
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

impl ScriptEditor {
    pub fn dirty(&self) -> bool {
        self.path.is_some() && self.text != self.saved
    }

    /// Open `path` (its text replaces whatever was open; the caller asks before dropping unsaved edits).
    pub fn load(&mut self, path: &Path) -> std::io::Result<()> {
        let text = std::fs::read_to_string(path)?;
        self.path = Some(path.to_path_buf());
        self.saved = text.clone();
        self.text = text;
        self.disk_time = modified(path);
        self.checked = Some(Instant::now());
        self.conflict = false;
        self.output.clear();
        self.output_err = false;
        self.intel = Intel::default();
        self.intel.changed(None);
        Ok(())
    }

    pub fn save(&mut self) -> Result<(), String> {
        let path = self.path.clone().ok_or("No script is open")?;
        std::fs::write(&path, &self.text).map_err(|e| format!("Couldn't save {}: {e}", path.display()))?;
        self.saved = self.text.clone();
        self.disk_time = modified(&path);
        self.conflict = false;
        Ok(())
    }

    /// The open file moved (renamed from the menu or a tool): follow it.
    pub fn moved(&mut self, from: &Path, to: &Path) {
        if self.path.as_deref() == Some(from) {
            self.path = Some(to.to_path_buf());
            self.disk_time = modified(to);
        }
    }

    /// The open file was deleted: close it (unsaved edits go with it - Delete… asked first).
    pub fn deleted(&mut self, path: &Path) {
        if self.path.as_deref() == Some(path) {
            *self = ScriptEditor { rename: self.rename.take(), ..Default::default() };
        }
    }

    /// Look at the disk (at most once a second): a newer file reloads, or raises `conflict` when there
    /// are unsaved edits here. True when the text was reloaded.
    pub fn poll_disk(&mut self, now: Instant) -> bool {
        let Some(path) = self.path.clone() else { return false };
        if self.checked.is_some_and(|t| now.duration_since(t).as_secs_f32() < 1.0) {
            return false;
        }
        self.checked = Some(now);
        let t = modified(&path);
        if t.is_none() || t == self.disk_time {
            return false;
        }
        if self.dirty() {
            self.conflict = true;
            return false;
        }
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                self.saved = text.clone();
                self.text = text;
                self.disk_time = t;
                self.intel.changed(None);
                true
            }
            Err(_) => false,
        }
    }

    /// Reload the disk's version over the edits here (the conflict bar's Reload).
    pub fn reload(&mut self) {
        if let Some(p) = self.path.clone() {
            let (out, err) = (std::mem::take(&mut self.output), self.output_err);
            if self.load(&p).is_ok() {
                self.output = out;
                self.output_err = err;
            }
        }
    }

    pub fn file_name(&self) -> String {
        self.path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }
}

/// What the user asked the pane for; the app does it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Act {
    Save,
    Run,
    OpenExternally,
    OpenFolder,
    AddScript,
}

/// Draw the pane. Saving happens here only as an `Act` (the app shows the result).
pub fn show(ui: &mut egui::Ui, st: &mut ScriptEditor) -> Vec<Act> {
    let mut acts = Vec::new();
    if st.poll_disk(Instant::now()) {
        st.output = "Reloaded: the file changed on disk".into();
        st.output_err = false;
    }
    if st.path.is_none() {
        ui.vertical_centered(|ui| {
            ui.add_space(24.0);
            ui.weak("No script open");
            ui.add_space(6.0);
            if ui.button("Add Script").on_hover_text("Make a new script from a short template and open it").clicked() {
                acts.push(Act::AddScript);
            }
            ui.add_space(4.0);
            ui.weak("Or pick one in Window ▸ Scripts ▸ <script> ▸ Open");
        });
        return acts;
    }
    let editor_id = egui::Id::new("se-script-editor-code");
    // Ctrl+S while the editor has focus: hotkeys stand down while a text field has focus, so the
    // project's Save never sees this press
    let focused = ui.memory(|m| m.has_focus(editor_id));
    if focused && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::S)) {
        acts.push(Act::Save);
    }
    ui.horizontal(|ui| {
        let name = st.file_name();
        ui.strong(&name).on_hover_text(st.path.as_ref().map(|p| p.display().to_string()).unwrap_or_default());
        if st.dirty() {
            ui.label(egui::RichText::new("●").color(ui.visuals().warn_fg_color)).on_hover_text("Unsaved changes");
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .button("Open Externally")
                .on_hover_text("Open the file in the app Windows uses for it, or Notepad")
                .clicked()
            {
                acts.push(Act::OpenExternally);
            }
            if ui.button("Open Folder").on_hover_text("Show the file in File Explorer").clicked() {
                acts.push(Act::OpenFolder);
            }
            if ui.button("Run").on_hover_text("Save, then run it against the project as one undo step").clicked() {
                acts.push(Act::Run);
            }
            let save = ui.add_enabled(st.dirty(), egui::Button::new("Save"));
            if save.on_hover_text("Save (Ctrl+S while editing)").clicked() {
                acts.push(Act::Save);
            }
        });
    });
    if st.conflict {
        ui.horizontal(|ui| {
            ui.colored_label(ui.visuals().warn_fg_color, "Changed on disk");
            if ui.button("Reload").on_hover_text("Drop the edits here and load the file from disk").clicked() {
                st.reload();
            }
            if ui.button("Keep Mine").on_hover_text("Keep the edits here; Save writes them over the disk's").clicked() {
                st.conflict = false;
                st.disk_time = st.path.as_deref().and_then(modified);
            }
        });
    }
    // the code fills the pane above the result line
    let line_h = ui.text_style_height(&egui::TextStyle::Body) + ui.spacing().item_spacing.y * 2.0;
    let code_h = (ui.available_height() - line_h).max(60.0);
    egui::ScrollArea::both().id_salt("se-script-editor-scroll").max_height(code_h).auto_shrink([false, false]).show(
        ui,
        |ui| code(ui, st, editor_id, code_h),
    );
    status_line(ui, st);
    acts
}

// ---------------------------------------------------------------------------------------------------
// ---- feat/script-intellisense ----
// completion, signature help, hover and diagnostics (the analysis is `ui::script_intel`)

/// How long after the last keystroke the syntax / unknown-global check runs.
const CHECK_DELAY: std::time::Duration = std::time::Duration::from_millis(300);
/// Rows the completion list shows at once.
const LIST_ROWS: usize = 8;

/// The pane's IntelliSense state. The check runs on its own thread; the list and signature help are
/// worked out on the UI thread only when the text or the cursor changes.
#[derive(Default)]
pub struct Intel {
    /// Bumped by every edit.
    rev: u64,
    /// When the last edit happened (`None` = check straight away, e.g. a file just opened).
    edited: Option<Instant>,
    /// An edit has not been checked yet.
    pending: bool,
    /// The check in flight: its result lands here.
    job: Option<Arc<Mutex<Option<Vec<Diag>>>>>,
    /// The last check's findings.
    pub diags: Vec<Diag>,
    /// The completion list and its selected row.
    list: Option<(Completion, usize)>,
    /// Signature help for (rev, cursor), and the call whose help Esc closed.
    sig: Option<(intel::Signature, usize, usize)>,
    sig_key: Option<(u64, usize)>,
    sig_closed: Option<usize>,
    /// Hover text for (rev, char).
    hover: Option<((u64, usize), Option<(String, String)>)>,
    /// The editor had focus last frame (egui drops focus on Esc before the pane sees the key).
    had_focus: bool,
}

impl Intel {
    /// The text changed (at `now`; `None` checks without waiting).
    pub fn changed(&mut self, now: Option<Instant>) {
        self.rev += 1;
        self.pending = true;
        self.edited = now;
    }

    /// A check is waiting or running.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn busy(&self) -> bool {
        self.pending || self.job.is_some()
    }

    /// Collect a finished check, start a due one, and ask for a frame when the delay ends.
    fn poll(&mut self, ctx: &egui::Context, text: &str, lang: Ctx) {
        if let Some(job) = &self.job {
            let done = job.lock().ok().and_then(|mut g| g.take());
            if let Some(d) = done {
                self.diags = d;
                self.job = None;
            }
        }
        if !self.pending || self.job.is_some() {
            return;
        }
        let wait = self.edited.map_or(std::time::Duration::ZERO, |t| CHECK_DELAY.saturating_sub(t.elapsed()));
        if !wait.is_zero() {
            ctx.request_repaint_after(wait);
            return;
        }
        self.pending = false;
        let slot = Arc::new(Mutex::new(None));
        self.job = Some(slot.clone());
        let (text, ctx) = (text.to_string(), ctx.clone());
        let spawned = std::thread::Builder::new().name("script-check".into()).spawn(move || {
            let d = intel::check(&text, lang);
            if let Ok(mut g) = slot.lock() {
                *g = Some(d);
            }
            ctx.request_repaint();
        });
        if spawned.is_err() {
            self.job = None;
        }
    }
}

fn char_to_byte(s: &str, chars: usize) -> usize {
    s.char_indices().nth(chars).map_or(s.len(), |(i, _)| i)
}

fn byte_to_char(s: &str, byte: usize) -> usize {
    let mut b = byte.min(s.len());
    while !s.is_char_boundary(b) {
        b -= 1;
    }
    s[..b].chars().count()
}

/// The icon colour of a completion row's kind (the editor's own syntax colours).
fn kind_colour(k: SymKind, dark: bool, base: Color32) -> Color32 {
    let k = match k {
        SymKind::Keyword => Kind::Keyword,
        SymKind::Function | SymKind::LocalFunction => Kind::Func,
        SymKind::Table => Kind::Tag,
        SymKind::Type => Kind::AtRule,
        SymKind::Tool => Kind::Str,
        SymKind::Value | SymKind::Local | SymKind::Param | SymKind::Global => Kind::Prop,
    };
    colour(k, dark, base)
}

/// Put `label` in place of `start..cursor` and the cursor after it.
fn accept(ctx: &egui::Context, id: egui::Id, text: &mut String, start: usize, label: &str) -> bool {
    let Some(mut state) = egui::TextEdit::load_state(ctx, id) else { return false };
    let Some(range) = state.cursor.char_range() else { return false };
    let end = char_to_byte(text, range.primary.index).max(start);
    if start > text.len() || !text.is_char_boundary(start) {
        return false;
    }
    text.replace_range(start..end, label);
    let at = byte_to_char(text, start + label.len());
    state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(at))));
    state.store(ctx, id);
    true
}

/// A wavy underline from x0 to x1 at y.
fn squiggle(p: &egui::Painter, x0: f32, x1: f32, y: f32, c: Color32) {
    let mut pts = Vec::new();
    let mut x = x0;
    let mut up = false;
    while x < x1 {
        pts.push(egui::pos2(x, if up { y - 1.5 } else { y + 0.5 }));
        x += 2.0;
        up = !up;
    }
    pts.push(egui::pos2(x1, y));
    p.add(egui::Shape::line(pts, egui::Stroke::new(1.0_f32, c)));
}

/// The code field with the list, signature help, hover and underlines.
fn code(ui: &mut egui::Ui, st: &mut ScriptEditor, id: egui::Id, code_h: f32) {
    let ctx = ui.ctx().clone();
    let (dark, base) = (ui.visuals().dark_mode, ui.visuals().text_color());
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let lang = intel::context_of(&st.text, st.plugin);
    let focused = ui.memory(|m| m.has_focus(id)) || st.intel.had_focus;

    // keys that belong to an open list (or signature help) are taken before the text field sees them
    let mut take: Option<usize> = None;
    if focused {
        if let Some((c, sel)) = st.intel.list.as_mut() {
            let n = c.items.len().max(1);
            let (down, up, tab, enter, esc) = ui.input_mut(|i| {
                (
                    i.consume_key(Modifiers::NONE, Key::ArrowDown),
                    i.consume_key(Modifiers::NONE, Key::ArrowUp),
                    i.consume_key(Modifiers::NONE, Key::Tab),
                    i.consume_key(Modifiers::NONE, Key::Enter),
                    i.consume_key(Modifiers::NONE, Key::Escape),
                )
            });
            if down {
                *sel = (*sel + 1) % n;
            }
            if up {
                *sel = (*sel + n - 1) % n;
            }
            if tab || enter {
                take = Some(*sel);
            }
            if esc {
                st.intel.list = None;
                ctx.memory_mut(|m| m.request_focus(id));
            }
        } else if let Some((_, _, open)) = &st.intel.sig {
            let open = *open;
            if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
                st.intel.sig_closed = Some(open);
                st.intel.sig = None;
                ctx.memory_mut(|m| m.request_focus(id));
            }
        }
    }
    let ask = focused && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Space));
    let mut taken = false;
    if let Some(k) = take {
        if let Some((c, _)) = st.intel.list.take() {
            if let Some(it) = c.items.get(k) {
                taken = accept(&ctx, id, &mut st.text, c.start, &it.label);
            }
        }
    }

    let len_before = st.text.len();
    let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
        let mut job = highlight(buf.as_str(), &font, dark, base);
        job.wrap.max_width = wrap;
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let rows = ((code_h / ui.text_style_height(&egui::TextStyle::Monospace)) as usize).max(4);
    let out = egui::TextEdit::multiline(&mut st.text)
        .id(id)
        .code_editor()
        .lock_focus(true)
        .desired_rows(rows)
        .desired_width(f32::INFINITY)
        .layouter(&mut layouter)
        .show(ui);
    let changed = out.response.changed() || taken;
    if taken {
        ctx.memory_mut(|m| m.request_focus(id));
    }
    if changed {
        st.intel.changed(Some(Instant::now()));
    }
    st.intel.poll(&ctx, &st.text, lang);
    let text = st.text.as_str();
    let cursor = out.cursor_range.map(|r| char_to_byte(text, r.primary.index));
    let has_focus = out.response.has_focus() || taken || ctx.memory(|m| m.has_focus(id));
    st.intel.had_focus = has_focus;
    if !has_focus && !ctx.input(|i| i.pointer.any_down()) {
        st.intel.list = None;
    }

    // the list: opened by typing a word, `.` or `:`, or Ctrl+Space; it follows the word being typed
    if !taken && (changed || ask) {
        let typed = text.len() > len_before;
        let prev = cursor.and_then(|c| text[..c].chars().next_back());
        let wordy = prev.is_some_and(|c| c.is_alphanumeric() || c == '_');
        let opens = ask || (typed && (wordy || matches!(prev, Some('.' | ':')))) || (st.intel.list.is_some() && wordy);
        st.intel.list = match (opens, cursor) {
            (true, Some(c)) => intel::complete(&Analysis::new(text), c, ask, lang).map(|comp| (comp, 0)),
            _ => None,
        };
    } else if let (Some((comp, _)), Some(c)) = (&st.intel.list, cursor) {
        let word_ok = c >= comp.start && !text.get(comp.start..c).is_some_and(|w| w.contains(char::is_whitespace));
        if !word_ok || (!has_focus && !ctx.input(|i| i.pointer.any_down())) {
            st.intel.list = None;
        }
    }
    if !has_focus {
        st.intel.sig = None;
    }

    // signature help: worked out when the text or the cursor moved
    if let (true, Some(c)) = (has_focus, cursor) {
        let key = (st.intel.rev, c);
        if st.intel.sig_key != Some(key) {
            st.intel.sig_key = Some(key);
            let an = Analysis::new(text);
            st.intel.sig = intel::call_at(&an, c).and_then(|call| {
                if st.intel.sig_closed == Some(call.open) {
                    return None;
                }
                intel::signature(&an, &call, lang).map(|s| (s, call.arg, call.open))
            });
            if st.intel.sig.is_none() && intel::call_at(&an, c).map(|c| c.open) != st.intel.sig_closed {
                st.intel.sig_closed = None;
            }
        }
    }

    let galley_pos = out.galley_pos.to_vec2();
    let rect_at = |byte: usize| out.galley.pos_from_cursor(egui::text::CCursor::new(byte_to_char(text, byte))).translate(galley_pos);
    let painter = ui.painter_at(out.response.rect.intersect(ui.clip_rect()));
    let pointer = ctx.input(|i| i.pointer.hover_pos()).filter(|p| out.response.rect.contains(*p));

    // underlines: errors red, unknown globals in the warning colour; their message on hover
    let mut hover_diag: Option<&Diag> = None;
    let char_w = ui.fonts_mut(|f| f.glyph_width(&egui::TextStyle::Monospace.resolve(ui.style()), '0')).max(4.0);
    for d in st.intel.diags.iter().take(200) {
        let (r0, r1) = (d.range.start.min(text.len()), d.range.end.min(text.len()));
        let a = rect_at(r0);
        let b = rect_at(r1);
        let x1 = if (a.top() - b.top()).abs() < 1.0 { b.left() } else { a.left() + char_w * 3.0 };
        let x1 = x1.max(a.left() + char_w);
        let c = if d.error { ui.visuals().error_fg_color } else { ui.visuals().warn_fg_color };
        squiggle(&painter, a.left(), x1, a.bottom() - 1.0, c);
        let zone = egui::Rect::from_min_max(a.left_top(), egui::pos2(x1, a.bottom()));
        if pointer.is_some_and(|p| zone.contains(p)) {
            hover_diag = Some(d);
        }
    }

    // hover docs for the API name under the pointer
    let hover = pointer.filter(|_| !ctx.input(|i| i.pointer.any_down())).and_then(|p| {
        let cc = out.galley.cursor_from_pos(p - out.galley_pos).index;
        let key = (st.intel.rev, cc);
        if st.intel.hover.as_ref().map(|(k, _)| *k) != Some(key) {
            let an = Analysis::new(text);
            st.intel.hover = Some((key, intel::hover(&an, char_to_byte(text, cc), lang)));
        }
        st.intel.hover.as_ref().and_then(|(_, h)| h.clone())
    });
    if hover_diag.is_some() || hover.is_some() {
        let diag = hover_diag.map(|d| (d.msg.clone(), d.error));
        out.response.clone().on_hover_ui_at_pointer(|ui| {
            ui.set_max_width(420.0);
            if let Some((msg, err)) = &diag {
                let c = if *err { ui.visuals().error_fg_color } else { ui.visuals().warn_fg_color };
                ui.colored_label(c, msg);
            }
            if let Some((sig, doc)) = &hover {
                ui.label(egui::RichText::new(sig).monospace());
                if !doc.is_empty() {
                    ui.weak(doc);
                }
            }
        });
    }

    let Some(range) = out.cursor_range else { return };
    let caret = out.galley.pos_from_cursor(range.primary).translate(galley_pos);

    // signature help, above the line: the current parameter stands out
    if let Some((sig, arg, _)) = st.intel.sig.clone() {
        egui::Area::new(id.with("signature"))
            .order(egui::Order::Foreground)
            .pivot(egui::Align2::LEFT_BOTTOM)
            .fixed_pos(caret.left_top() - egui::vec2(0.0, 2.0))
            .interactable(false)
            .show(&ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_max_width(480.0);
                    let accent = ui.visuals().selection.stroke.color;
                    let weak = ui.visuals().weak_text_color();
                    let mono = egui::TextStyle::Monospace.resolve(ui.style());
                    let mut job = LayoutJob::default();
                    let mut put = |s: &str, c: Color32, strong: bool| {
                        let mut f = TextFormat::simple(mono.clone(), c);
                        if strong {
                            f.underline = egui::Stroke::new(1.0_f32, c);
                        }
                        job.append(s, 0.0, f);
                    };
                    put(&sig.name, base, false);
                    put("(", weak, false);
                    let last = sig.params.len().saturating_sub(1);
                    for (k, p) in sig.params.iter().enumerate() {
                        let on = k == arg || (k == last && p.starts_with("...") && arg > last);
                        put(p, if on { accent } else { base }, on);
                        if k < last {
                            put(", ", weak, false);
                        }
                    }
                    put(")", weak, false);
                    if !sig.ret.is_empty() {
                        put(&format!(" -> {}", sig.ret), weak, false);
                    }
                    ui.label(job);
                    if !sig.doc.is_empty() {
                        ui.weak(&sig.doc);
                    }
                });
            });
    }

    // the completion list, under the cursor: icon, name, signature; the selected row's doc below
    let Some((comp, sel)) = st.intel.list.clone() else { return };
    let mut clicked = None;
    egui::Area::new(id.with("completion"))
        .order(egui::Order::Foreground)
        .fixed_pos(caret.left_bottom() + egui::vec2(0.0, 2.0))
        .show(&ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                let w = 420.0;
                ui.set_width(w);
                let v = ui.visuals().clone();
                let mono = egui::TextStyle::Monospace.resolve(ui.style());
                let row_h = ui.text_style_height(&egui::TextStyle::Monospace) + 6.0;
                let first = sel.saturating_sub(LIST_ROWS - 1).min(comp.items.len().saturating_sub(LIST_ROWS));
                for (k, it) in comp.items.iter().enumerate().skip(first).take(LIST_ROWS) {
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, row_h), egui::Sense::click());
                    let on = k == sel;
                    let p = ui.painter_at(rect);
                    let (fg, weak) = if on {
                        let fill = v.selection.bg_fill;
                        p.rect_filled(rect, 3.0, fill);
                        let c = tools::on_accent_over(fill, v.panel_fill);
                        (c, c.gamma_multiply(0.75))
                    } else {
                        if resp.hovered() {
                            p.rect_filled(rect, 3.0, v.widgets.hovered.weak_bg_fill);
                        }
                        (v.text_color(), v.weak_text_color())
                    };
                    // the kind icon: its letter on a tint of the kind's syntax colour
                    let kc = kind_colour(it.kind, v.dark_mode, v.text_color());
                    let icon = egui::Rect::from_center_size(egui::pos2(rect.left() + 12.0, rect.center().y), egui::vec2(18.0, 16.0));
                    p.rect_filled(icon, 3.0, kc.gamma_multiply(0.25));
                    p.text(icon.center(), egui::Align2::CENTER_CENTER, it.kind.letter(), FontId::monospace(11.0), kc);
                    let name = p.text(egui::pos2(rect.left() + 26.0, rect.center().y), egui::Align2::LEFT_CENTER, &it.label, mono.clone(), fg);
                    if it.detail != it.label {
                        let sig_font = FontId::new(mono.size * 0.9, mono.family.clone());
                        p.text(egui::pos2(name.right() + 10.0, rect.center().y), egui::Align2::LEFT_CENTER, &it.detail, sig_font, weak);
                    }
                    if resp.clicked() {
                        clicked = Some(k);
                    }
                }
                if let Some(it) = comp.items.get(sel) {
                    let doc = if it.doc.is_empty() { it.kind.name().to_string() } else { it.doc.clone() };
                    ui.separator();
                    ui.add(egui::Label::new(egui::RichText::new(doc).color(v.weak_text_color())).wrap());
                }
            });
        });
    if let Some(k) = clicked {
        if let Some(it) = comp.items.get(k) {
            if accept(&ctx, id, &mut st.text, comp.start, &it.label) {
                st.intel.changed(Some(Instant::now()));
                ctx.memory_mut(|m| m.request_focus(id));
            }
        }
        st.intel.list = None;
    }
}

/// The line under the code: the first problem (its line and message, counted), then the last run's result.
fn status_line(ui: &mut egui::Ui, st: &ScriptEditor) {
    ui.horizontal(|ui| {
        let d = &st.intel.diags;
        let first = d.iter().find(|d| d.error).or(d.first());
        if let Some(f) = first {
            let c = if f.error { ui.visuals().error_fg_color } else { ui.visuals().warn_fg_color };
            let more = if d.len() > 1 { format!("  (+{} more)", d.len() - 1) } else { String::new() };
            let tip: Vec<String> = d.iter().take(20).map(|d| format!("Line {}: {}", d.line, d.msg)).collect();
            ui.add(egui::Label::new(egui::RichText::new(format!("Line {}: {}{more}", f.line, f.msg)).color(c)).truncate())
                .on_hover_text(tip.join("\n"));
            ui.separator();
        }
        // the last run's result: selectable, so an error can be copied
        let colour = if st.output_err { ui.visuals().error_fg_color } else { ui.visuals().weak_text_color() };
        let text = if st.output.is_empty() { "Run shows the script's result here".to_string() } else { st.output.clone() };
        ui.add(egui::Label::new(egui::RichText::new(text).color(colour)).truncate().selectable(true))
            .on_hover_text(st.output.as_str());
    });
}

// ---------------------------------------------------------------------------------------------------
// the Scripts menu

/// One row of Window ▸ Scripts, in order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Entry {
    /// The submenu of `metas[i]`.
    Script(usize),
    Separator,
    AddScript,
    OpenFolder,
}

/// The rows of the Scripts menu for `n` scripts: one submenu each, then Add Script and Open Scripts Folder.
/// With no scripts it is just the two (so Add Script is always there).
pub fn entries(n: usize) -> Vec<Entry> {
    let mut v: Vec<Entry> = (0..n).map(Entry::Script).collect();
    if n > 0 {
        v.push(Entry::Separator);
    }
    v.extend([Entry::AddScript, Entry::OpenFolder]);
    v
}

/// What a Scripts menu row asked for.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum MenuAct {
    Run(PathBuf),
    Open(PathBuf),
    OpenFolder(PathBuf),
    /// Rename… was clicked at this screen position (the name box opens there).
    Rename(PathBuf, egui::Pos2),
    Delete(PathBuf),
    AddScript,
    OpenScriptsFolder,
}

/// The Scripts submenu's body. `external` names where Open goes ("Open" vs "Open Externally" in the tip).
pub fn scripts_menu(ui: &mut egui::Ui, metas: &[ScriptMeta], external: bool) -> Option<MenuAct> {
    let mut act = None;
    for e in entries(metas.len()) {
        match e {
            Entry::Script(i) => {
                let m = &metas[i];
                let glyph = m.icon.and_then(tools::Glyph::from_name).unwrap_or(tools::Glyph::Terminal);
                let r = menu::sub(ui, Some(glyph), &m.name, |ui| script_rows(ui, m, external)).flatten();
                act = act.or(r);
            }
            Entry::Separator => {
                ui.separator();
            }
            Entry::AddScript => {
                let tip = if external {
                    "Make a new script and open it in your editor"
                } else {
                    "Make a new script and open it in the Script Editor"
                };
                if menu::row(ui, Some(tools::Glyph::Letter('+')), "Add Script", "").on_hover_text(tip).clicked() {
                    act = Some(MenuAct::AddScript);
                }
            }
            Entry::OpenFolder => {
                if menu::row(ui, Some(tools::Glyph::Folder), "Open Scripts Folder", "").clicked() {
                    act = Some(MenuAct::OpenScriptsFolder);
                }
            }
        }
    }
    act
}

fn script_rows(ui: &mut egui::Ui, m: &ScriptMeta, external: bool) -> Option<MenuAct> {
    let p = || m.path.clone();
    let run = menu::row(ui, Some(tools::Glyph::Play), "Run", m.hotkey.as_deref().unwrap_or(""));
    let run = if m.desc.is_empty() { run } else { run.on_hover_text(&m.desc) };
    if run.clicked() {
        return Some(MenuAct::Run(p()));
    }
    let tip = if external { "Open it in your editor (Settings ▸ General)" } else { "Open it in the Script Editor" };
    if menu::row(ui, None, "Open", "").on_hover_text(tip).clicked() {
        return Some(MenuAct::Open(p()));
    }
    if menu::row(ui, Some(tools::Glyph::Folder), "Open Folder", "").on_hover_text("Show it in File Explorer").clicked()
    {
        return Some(MenuAct::OpenFolder(p()));
    }
    let r = menu::row(ui, None, "Rename…", "");
    if r.clicked() {
        return Some(MenuAct::Rename(p(), r.rect.left_top()));
    }
    if menu::row(ui, None, "Delete…", "").on_hover_text("Move it to the Recycle Bin").clicked() {
        return Some(MenuAct::Delete(p()));
    }
    None
}

/// The Rename… name box: a small window at the menu's spot. `Some(name)` when the user pressed Enter or
/// Rename; the box closes itself on Esc / Cancel (`st.rename` becomes `None`).
pub fn rename_box(ctx: &egui::Context, st: &mut ScriptEditor) -> Option<(PathBuf, String)> {
    let r = st.rename.as_mut()?;
    let mut out = None;
    let mut close = false;
    egui::Window::new("Rename Script")
        .id(egui::Id::new("se-script-rename"))
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .fixed_pos(r.at)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Name");
                let field = ui.add(egui::TextEdit::singleline(&mut r.name).desired_width(200.0));
                if std::mem::take(&mut r.focus) {
                    field.request_focus();
                }
                if field.changed() {
                    r.error.clear();
                }
                ui.weak(r.path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default());
                let enter = field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                if ui.button("Rename").clicked() || enter {
                    out = Some((r.path.clone(), r.name.clone()));
                }
                if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                    close = true;
                }
            });
            if !r.error.is_empty() {
                ui.colored_label(ui.visuals().error_fg_color, &r.error);
            }
        });
    if close {
        st.rename = None;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<(&str, Kind)> {
        tokens(src).into_iter().map(|(r, k)| (&src[r], k)).collect()
    }

    #[test]
    fn luau_tokens() {
        let k = kinds("local x = editor.tool(\"a\", {n = 0x1F}) -- hi\n");
        assert!(k.contains(&("local", Kind::Keyword)));
        assert!(k.contains(&("editor", Kind::Tag)));
        assert!(k.contains(&("tool", Kind::Func)));
        assert!(k.contains(&("\"a\"", Kind::Str)));
        assert!(k.contains(&("0x1F", Kind::Num)));
        assert!(k.contains(&("-- hi", Kind::Comment)));
        let k = kinds("--[==[ long\n ]] still ]==] x = [[s]] .. 1.5e-3");
        assert_eq!(k[0], ("--[==[ long\n ]] still ]==]", Kind::Comment));
        assert!(k.contains(&("[[s]]", Kind::Str)) && k.contains(&("1.5e-3", Kind::Num)));
        let k = kinds("-- @desc Says hi\nt.end_time = 'it''s'");
        assert!(k.contains(&("@desc", Kind::AtRule)));
        assert!(k.contains(&("end_time", Kind::Prop)), "a member named like a keyword is not one");
        // an unclosed string or comment never panics and stays inside the text
        for s in ["\"abc", "--[[ open", "[==[", "x = 'a\\", "é = 1 -- ü"] {
            let _ = highlight(s, &FontId::monospace(12.0), true, Color32::WHITE);
            assert!(tokens(s).iter().all(|(r, _)| r.end <= s.len()), "{s}");
        }
    }

    #[test]
    fn highlight_covers_the_whole_text() {
        let src = crate::scripting::TEMPLATE;
        let job = highlight(src, &FontId::monospace(12.0), true, Color32::WHITE);
        assert_eq!(job.text, src);
        assert!(job.sections.len() > 10, "the template is coloured");
    }

    #[test]
    fn menu_shows_add_script_with_zero_scripts() {
        assert_eq!(entries(0), vec![Entry::AddScript, Entry::OpenFolder]);
        assert_eq!(
            entries(2),
            vec![Entry::Script(0), Entry::Script(1), Entry::Separator, Entry::AddScript, Entry::OpenFolder]
        );
        // and the Window menu draws the Scripts submenu whatever the count
        let menus = include_str!("app/menus.rs");
        assert!(menus.contains("scripts_ctl::menu(self, ui)"));
        assert!(!menus.contains("if !self.script_metas().is_empty()"), "the Scripts submenu is never hidden");
        // the empty menu really draws its two rows
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                assert_eq!(scripts_menu(ui, &[], false), None);
            });
        });
    }

    #[test]
    fn edits_save_and_follow_the_disk() {
        let d = std::env::temp_dir().join(format!("se-script-editor-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let path = crate::scripting::create_in(&d).unwrap();
        let mut st = ScriptEditor::default();
        st.load(&path).unwrap();
        assert!(!st.dirty());
        st.text.push_str("-- more\n");
        assert!(st.dirty());
        st.save().unwrap();
        assert!(!st.dirty() && std::fs::read_to_string(&path).unwrap().ends_with("-- more\n"));
        // another program writes the file: it reloads (a second later)
        std::fs::write(&path, "-- from outside\n").unwrap();
        st.disk_time = Some(SystemTime::UNIX_EPOCH); // stand-in for a newer mtime on a coarse clock
        assert!(st.poll_disk(Instant::now() + std::time::Duration::from_secs(2)));
        assert_eq!(st.text, "-- from outside\n");
        // with unsaved edits here it asks instead
        st.text.push('x');
        st.disk_time = Some(SystemTime::UNIX_EPOCH);
        assert!(!st.poll_disk(Instant::now() + std::time::Duration::from_secs(4)));
        assert!(st.conflict && st.text.ends_with('x'));
        st.reload();
        assert!(!st.conflict && st.text == "-- from outside\n");
        // rename / delete follow the file
        let to = crate::scripting::rename(&path, "Other").unwrap();
        st.moved(&path, &to);
        assert_eq!(st.file_name(), "Other.luau");
        st.deleted(&to);
        assert!(st.path.is_none() && st.text.is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    fn frame(ctx: &egui::Context, f: &mut dyn FnMut(&egui::Context)) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 700.0))),
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| f(ctx));
    }

    #[test]
    fn assert_no_idle_repaint_script_editor() {
        let d = std::env::temp_dir().join(format!("se-script-idle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let path = crate::scripting::create_in(&d).unwrap();
        let ctx = egui::Context::default();
        for open in [false, true] {
            let mut st = ScriptEditor::default();
            if open {
                st.load(&path).unwrap();
                st.output = "script failed: line 3".into();
                st.output_err = true;
            }
            // the check a newly opened file gets runs on its own thread: idle means after it landed
            for _ in 0..2000 {
                frame(&ctx, &mut |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        assert!(show(ui, &mut st).is_empty());
                    });
                });
                if !st.intel.busy() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            assert!(!st.intel.busy(), "the check finished");
            for _ in 0..30 {
                frame(&ctx, &mut |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        assert!(show(ui, &mut st).is_empty());
                    });
                });
            }
            assert!(!ctx.has_requested_repaint(), "an idle Script Editor must not repaint (open: {open})");
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    // ---- feat/script-intellisense ----
    fn keys(ctx: &egui::Context, st: &mut ScriptEditor, events: Vec<egui::Event>) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 700.0))),
            events,
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let _ = show(ui, st);
            });
        });
    }

    fn key(k: Key) -> egui::Event {
        egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }
    }

    #[test]
    fn typing_a_member_opens_the_list_and_enter_takes_it() {
        let d = std::env::temp_dir().join(format!("se-script-intel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let path = crate::scripting::create_in(&d).unwrap();
        std::fs::write(&path, "").unwrap();
        let mut st = ScriptEditor::default();
        st.load(&path).unwrap();
        let ctx = egui::Context::default();
        let id = egui::Id::new("se-script-editor-code");
        keys(&ctx, &mut st, vec![]);
        ctx.memory_mut(|m| m.request_focus(id));
        keys(&ctx, &mut st, vec![]);
        keys(&ctx, &mut st, vec![egui::Event::Text("editor.".into())]);
        let labels: Vec<String> = st.intel.list.as_ref().map(|(c, _)| c.items.iter().map(|i| i.label.clone()).collect()).unwrap_or_default();
        assert_eq!(labels, ["log", "tool", "tools"], "the list after `editor.`");
        // Down, Enter: the second row goes in and the list closes
        keys(&ctx, &mut st, vec![key(Key::ArrowDown)]);
        keys(&ctx, &mut st, vec![key(Key::Enter)]);
        assert_eq!(st.text, "editor.tool");
        assert!(st.intel.list.is_none());
        // `(` opens signature help on the first parameter
        keys(&ctx, &mut st, vec![egui::Event::Text("(".into())]);
        let (sig, arg, _) = st.intel.sig.clone().expect("signature help");
        assert_eq!((sig.name.as_str(), arg), ("editor.tool", 0));
        // Esc closes the list; a later word opens it again
        keys(&ctx, &mut st, vec![egui::Event::Text("\"x\", pa".into())]);
        assert!(st.intel.list.as_ref().is_some_and(|(c, _)| c.items[0].label == "pairs"));
        keys(&ctx, &mut st, vec![key(Key::Escape)]);
        assert!(st.intel.list.is_none() && st.text.ends_with("pa"), "Esc closes the list and keeps the text: {:?} {:?}", st.text, st.intel.list.as_ref().map(|(c, _)| c.items.len()));
        // the check runs after typing stops and finds the unfinished call
        for _ in 0..2000 {
            keys(&ctx, &mut st, vec![]);
            if !st.intel.busy() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(st.intel.diags.iter().any(|d| d.error && d.line == 1), "{:?}", st.intel.diags);
        let _ = std::fs::remove_dir_all(&d);
    }
}
