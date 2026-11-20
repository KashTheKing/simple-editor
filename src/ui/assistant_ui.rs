//! ---- ws:assistant-pane ----
//! The Assistant pane widget (pure: no `App`), laid out like a chat app:
//! - a **sidebar** of chats (New chat, search over every chat, the list with pinned ones first and a date
//!   column; right-click or double-click to rename, pin, save or delete; collapsible);
//! - the **message column**: user bubbles on the right, Markdown replies on the left with each reply's tool
//!   calls folded into one "N steps" line; right-click a message to copy it (plain text or Markdown), copy
//!   its attachments, retry, edit and resend, or delete it; text selected in a message copies with Ctrl+C;
//! - the **composer** at the bottom: attachment thumbnails / chips (click to preview, right-click to copy or
//!   save), a prompt box that grows with the text, the paperclip, the capture menu (current frame, a panel,
//!   an area of the window, the timeline range), dictation, and Send / Stop with the model and effort
//!   pickers kept small beside it. The agent, Tasks, Skills and import / export live in the `⋯` menu.
//!
//! Opening the pane starts a new chat (an empty one is reused) with the cursor in the composer. It edits
//! what is pure state (conversations, tasks, chips) itself and returns what needs the app (`Out`);
//! `ui/app/assistant_ctl.rs` acts on it. The data model is `ui::assistant_data`.

pub use super::assistant_data::*;
use super::layout::Pane;
use super::list_rows::{RowSpec, Rows};
use super::menu;
use super::tools::{draw_glyph, glyph_text_button, glyph_text_button_w, icon_button, on_accent, Glyph};
use crate::engine::skills::{self, Skill, Source};
use crate::mcp::perms::Mode;
use crate::theme::Palette;
use eframe::egui;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Chat,
    /// Scripts the assistant wrote for the user to run (the `⋯` menu).
    Tasks,
    /// ws:skills: every skill, with Run / Edit / Duplicate / Delete and New skill (the `⋯` menu).
    Skills,
    /// feat/styles-context: the Editing Styles library (`ui::styles_ui::styles_tab`).
    Styles,
}

/// The composer's prompt box (fixed, so the app can tell when it has the keyboard: image paste).
pub fn prompt_id() -> egui::Id {
    egui::Id::new("assistant_prompt")
}

/// ws:skills: the New / Edit skill window's fields.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SkillDraft {
    pub name: String,
    /// "What should it do?" (Write it for me).
    pub what: String,
    pub description: String,
    pub args: String,
    pub body: String,
    /// The plain editor is open (always, when editing).
    pub plain: bool,
    /// The user skill being edited (a rename removes the old file).
    pub editing: Option<String>,
    pub error: String,
}

impl SkillDraft {
    pub fn edit(s: &Skill) -> Self {
        Self {
            name: s.name.clone(),
            description: s.description.clone(),
            args: s.args.clone(),
            body: s.body.clone(),
            plain: true,
            editing: Some(s.name.clone()),
            ..Default::default()
        }
    }
    /// A copy under a free name, as a new user skill.
    pub fn copy(s: &Skill, all: &[Skill]) -> Self {
        let mut name = format!("{}-copy", s.name);
        let mut n = 2;
        while all.iter().any(|o| o.name == name) {
            name = format!("{}-copy-{n}", s.name);
            n += 1;
        }
        Self { name, editing: None, ..Self::edit(s) }
    }
}

/// What `skill_window` / the Skills tab ask of the app.
#[derive(Debug, PartialEq, Eq)]
pub enum SkillOut {
    /// "Write it for me": run `/create-skill` with the draft's name and "what".
    Draft,
    /// Save the draft as a user skill.
    Save,
    /// Delete this user skill (after the app's confirm).
    Delete(String),
}

/// Shared state of the Assistant pane (`App.assistant`). `mcp-context` reads `pending`
/// (`assistant.wait`); `voice-input` drives `mic_wanted` / `mic_status`, appends to `prompt` and sets
/// `send_now`.
#[derive(Default)]
pub struct AssistantState {
    /// The text in the chat prompt box (voice dictation appends here).
    pub prompt: String,
    /// Requests sent with the "external" backend, waiting for an MCP client to take them
    /// through `assistant.wait` (oldest first).
    pub pending: std::collections::VecDeque<String>,
    /// Set when dictation or a quick action wants the prompt sent on the next frame.
    pub send_now: bool,
    /// The mic button: while true the voice code records; when it goes false it stops and transcribes.
    pub mic_wanted: bool,
    /// Written by the voice code: "" when idle, else "Listening…", "Transcribing…" or an error.
    pub mic_status: String,
    /// Live context chips (keys) the user removed for the next request; cleared after each send.
    pub chips_off: Vec<&'static str>,
    /// This project's conversations; `cur` is the one shown. Never empty once `conv_mut` ran.
    pub convs: Vec<Conversation>,
    pub cur: usize,
    /// The conversation the running request writes to (the user may open another one meanwhile).
    pub run_conv: Option<u64>,
    pub tab: Tab,
    /// Context the user added on purpose ("Add to Assistant", a timeline range); sent with the next message.
    pub pinned: Vec<Chip>,
    /// Images and files waiting in the composer.
    pub images: Vec<Attachment>,
    /// The Tasks view's "Describe a task…" box.
    pub task_prompt: String,
    /// The task whose code is open for editing, and the edit buffer.
    pub editing: Option<(u64, String)>,
    /// "Handle my comments": new timeline comments go to the agent in the background.
    pub comments: bool,
    /// Focus the chat prompt on the next draw (a pinned chip was just added).
    pub focus_prompt: bool,
    /// The sidebar's search box.
    pub search: String,
    /// The sidebar: Some(shown) once the user toggled it, None = shown when the pane is wide enough.
    pub sidebar: Option<bool>,
    /// The chat being renamed in the sidebar, and the edit buffer.
    pub renaming: Option<(u64, String)>,
    /// The attachment open in the preview window.
    pub preview: Option<Attachment>,
    /// "Select an area": the overlay is up (`area_overlay`), and where the drag began.
    pub area: bool,
    area_from: Option<egui::Pos2>,
    /// The frame the pane was last drawn: a gap means it was just opened (a new chat is made ready).
    seen_frame: Option<u64>,
    /// Set by a caller that already prepared the chat for this opening (`assistant_ctl::ask`).
    pub keep_on_open: bool,
    /// The conversations changed: the app saves them (off the UI thread) and clears this.
    pub dirty: bool,
    /// The model picker's "Other…" text field is open.
    pub model_other: bool,
    /// ws:skills: the highlighted row of the `/` list.
    pub slash_sel: usize,
    /// ws:skills: put the prompt's cursor at its end on the next draw (after a completion).
    pub cursor_end: bool,
    /// ws:skills: the New / Edit skill window (open while Some).
    pub skill_draft: Option<SkillDraft>,
    /// Attachment thumbnails by file (None = unreadable), loaded once; the preview's full image.
    thumbs: HashMap<std::path::PathBuf, Option<egui::TextureHandle>>,
    preview_tex: Option<(std::path::PathBuf, Option<egui::TextureHandle>)>,
    /// feat/styles-context: the Editing Styles tab and the Learn a style window.
    pub styles: crate::ui::styles_ui::StylesState,
}

impl AssistantState {
    /// The conversation shown (a fresh one is made when there is none).
    pub fn conv_mut(&mut self) -> &mut Conversation {
        if self.convs.is_empty() {
            self.convs.push(Conversation::new(unique_id()));
            self.cur = 0;
        }
        self.cur = self.cur.min(self.convs.len() - 1);
        &mut self.convs[self.cur]
    }
    pub fn conv(&self) -> Option<&Conversation> {
        self.convs.get(self.cur.min(self.convs.len().saturating_sub(1)))
    }
    /// The conversation the running request belongs to (else the one shown).
    pub fn target_mut(&mut self) -> &mut Conversation {
        match self.run_conv.and_then(|id| self.convs.iter().position(|c| c.id == id)) {
            Some(i) => &mut self.convs[i],
            None => self.conv_mut(),
        }
    }
    /// The shown conversation's lines (what `assistant.status` reports).
    pub fn log(&self) -> &[Line] {
        self.conv().map_or(&[], |c| c.lines.as_slice())
    }
    /// Add a line to the running request's conversation (the shown one when nothing runs).
    pub fn push(&mut self, line: Line) {
        let c = self.target_mut();
        c.lines.push(line);
        c.updated = now_secs();
    }
    pub fn tasks_mut(&mut self) -> &mut Vec<Task> {
        &mut self.conv_mut().tasks
    }
    /// Start a new chat (reusing the shown one when it is still empty).
    pub fn new_chat(&mut self) {
        if !self.conv().is_some_and(Conversation::is_empty) {
            self.convs.push(Conversation::new(unique_id()));
            self.cur = self.convs.len() - 1;
        }
        self.tab = Tab::Chat;
        self.focus_prompt = true;
        self.dirty = true;
    }
    pub fn open(&mut self, id: u64) {
        if let Some(i) = self.convs.iter().position(|c| c.id == id) {
            self.cur = i;
            // a chat that only has tasks opens on them
            let chatty = !self.convs[i].lines.is_empty() || self.convs[i].tasks.is_empty();
            self.tab = if chatty { Tab::Chat } else { Tab::Tasks };
        }
    }
    /// Delete a conversation (a fresh one takes its place when it was the last).
    pub fn delete(&mut self, id: u64) -> Option<Conversation> {
        let i = self.convs.iter().position(|c| c.id == id)?;
        let gone = self.convs.remove(i);
        if self.cur >= i && self.cur > 0 {
            self.cur -= 1;
        }
        self.conv_mut();
        self.dirty = true;
        Some(gone)
    }
    pub fn rename(&mut self, id: u64, title: &str) {
        if let Some(c) = self.convs.iter_mut().find(|c| c.id == id) {
            c.title = short(title, 80);
            self.dirty = true;
        }
    }
    pub fn toggle_pin(&mut self, id: u64) {
        if let Some(c) = self.convs.iter_mut().find(|c| c.id == id) {
            c.pinned = !c.pinned;
            self.dirty = true;
        }
    }
    /// Remove one message (its lines) from the shown chat.
    pub fn delete_msg(&mut self, m: &Msg) {
        let c = self.conv_mut();
        if m.lines.end <= c.lines.len() {
            c.lines.drain(m.lines.clone());
            self.dirty = true;
        }
    }
    /// Edit and resend: the user message `m` goes back into the composer (text and attachments) and the
    /// chat is cut back to before it.
    /// ponytail: the agent's session still remembers the cut turns; start a new chat to forget them.
    pub fn edit_msg(&mut self, m: &Msg) {
        let c = self.conv_mut();
        if !m.user || m.lines.end > c.lines.len() {
            return;
        }
        let (text, images) = (msg_markdown(&c.lines, m), msg_attachments(&c.lines, m));
        c.lines.truncate(m.lines.start);
        self.prompt = text;
        self.images = images;
        (self.focus_prompt, self.cursor_end, self.dirty) = (true, true, true);
    }
    /// Add chats from an exported file as new conversations (fresh ids; their agent sessions are not ours).
    pub fn import(&mut self, chats: Vec<Conversation>) -> usize {
        let n = chats.len();
        for mut c in chats.into_iter().filter(|c| !c.is_empty()) {
            c.id = unique_id();
            c.session = None;
            self.convs.push(c);
        }
        self.dirty = true;
        n
    }
    /// Show `convs` (a project's store): the most recently used one, or a fresh chat.
    pub fn load(&mut self, convs: Vec<Conversation>) {
        self.convs = convs;
        self.cur = self.convs.iter().enumerate().max_by(|a, b| a.1.updated.total_cmp(&b.1.updated)).map_or(0, |(i, _)| i);
        self.conv_mut();
        self.editing = None;
    }
    /// Called once per draw with the frame number: a gap since the last draw means the pane was just
    /// opened - a new chat, the cursor in the composer.
    fn seen(&mut self, frame: u64) {
        if self.seen_frame.is_none_or(|f| frame > f + 1) && !std::mem::take(&mut self.keep_on_open) {
            self.new_chat();
        }
        self.keep_on_open = false;
        self.seen_frame = Some(frame);
    }
}

/// What the pane shows that it doesn't own.
pub struct View<'a> {
    pub running: bool,
    /// (key, label) for each piece of live context that would be sent (`assistant_ctl::chips_of`);
    /// empty while "Assistant follows the selection" is off.
    pub chips: &'a [(&'static str, String)],
    /// ws:skills: every skill (`engine::skills::all`), and the names the user ran last (newest first).
    pub skills: &'a [Skill],
    pub recent: &'a [String],
    /// (undo-stack length, label of its top entry) - decides whether a request's Undo still applies.
    pub undo_top: (usize, Option<&'a str>),
    /// `Settings.assistant_backend`: "claude" | "codex" | "custom" | "external".
    pub backend: &'a str,
    pub model: &'a str,
    /// `Settings.assistant_effort`: "" (the model's default) or one of `EFFORTS`.
    pub effort: &'a str,
    /// Comments waiting for the agent ("Handle my comments").
    pub queued: usize,
    /// The docked panes on screen (the capture menu's "Screenshot a panel").
    pub panes: &'a [Pane],
    /// ws:mcp-permissions: every MCP call runs (`mcp::perms::full_access`): the header shows a badge.
    pub full_access: bool,
    /// The running chat's access questions: (held call index, question, reason).
    pub asks: &'a [(usize, String, String)],
    // ---- feat/styles-context ----
    /// Every editing style (`engine::styles::all`), the one applied to this project, and whether the
    /// Project Context pane is on screen (its button is lit).
    pub styles: &'a [crate::engine::styles::Style],
    pub applied_style: Option<&'a str>,
    pub context_open: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Out {
    None,
    /// Send `state.prompt` (with the chips and images).
    Send,
    Stop,
    /// Undo a finished request (it is the top of the undo stack).
    Undo,
    /// Send the user message starting at this line of the shown chat again.
    Retry(usize),
    Backend(&'static str),
    Model(String),
    Effort(String),
    /// Send `state.task_prompt` as a script-writing task.
    AddTask,
    /// Run one task / every Ready task in order.
    Run(Option<u64>),
    /// Write a task into the Scripts folder.
    SaveScript(u64),
    Attach(Attach),
    /// "Handle my comments" switched on (true) or off.
    Comments(bool),
    Skill(SkillOut),
    /// ws:mcp-permissions: held call `.0` answered: 0 Allow once, 1 Always allow, 2 Deny.
    Answer(usize, u8),
    /// The shown chat's permission mode.
    Mode(Mode),
    /// feat/styles-context: the Editing Styles tab / Project Context button.
    Styles(crate::ui::styles_ui::Out),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attach {
    /// The timeline at the playhead, as an image.
    Frame,
    /// Files from disk (images are attached as pictures, anything else by path).
    File,
    /// A screenshot of one pane.
    Panel(Pane),
    /// The timeline's In/Out (else the selected clips' span): a picture plus its clips and tracks.
    Timeline,
    Pin(Pin),
}

/// Whether a finished request's Undo applies: Ok, or why not.
pub fn undo_state(undo: &Option<(usize, String)>, top: (usize, Option<&str>)) -> Result<(), &'static str> {
    match undo {
        None => Err("This request didn't change anything"),
        Some((len, label)) if *len == top.0 && top.1 == Some(label.as_str()) => Ok(()),
        Some(_) => Err("Other edits came after it - use History to step back further"),
    }
}

/// A right-click menu row "Add to Assistant": queues `pin` for the Assistant's next tick.
pub fn add_to_assistant_row(ui: &mut egui::Ui, pin: Pin) {
    if menu::row(ui, Some(Glyph::Assistant), "Add to Assistant", "").clicked() {
        request_pin(ui.ctx(), pin);
    }
}

/// The pane is wider than this: the sidebar shows unless the user hid it.
const SIDEBAR_FROM: f32 = 460.0;
const SIDEBAR_W: f32 = 190.0;

pub fn draw(ui: &mut egui::Ui, state: &mut AssistantState, v: &View) -> Out {
    let pal = crate::ui::titlebar::palette(ui.ctx());
    let mut out = Out::None;
    state.seen(ui.ctx().cumulative_frame_nr());
    state.conv_mut();
    // the composer has the cursor whenever the pane opens: a blinking caret would repaint twice a second
    // forever (the palette does the same)
    if ui.style().visuals.text_cursor.blink {
        ui.ctx().style_mut(|s| s.visuals.text_cursor.blink = false);
    }
    let wide = ui.available_width() >= SIDEBAR_FROM;
    if state.sidebar.unwrap_or(wide) {
        egui::SidePanel::left(ui.id().with("assistant_sidebar"))
            .exact_width(SIDEBAR_W.min(ui.available_width() * 0.5))
            .resizable(false)
            .frame(egui::Frame::NONE.fill(ui.visuals().panel_fill).inner_margin(egui::Margin::symmetric(6, 6)))
            .show_inside(ui, |ui| sidebar(ui, state, v));
    }
    // a little air at the pane's edges: right-aligned controls (Send) sat on the pane border and lost
    // their last pixels to its clip rect
    egui::Frame::NONE.inner_margin(egui::Margin { left: 6, right: 8, top: 2, bottom: 2 }).show(ui, |ui| {
        top_bar(ui, state, v, wide, &mut out);
        match state.tab {
            Tab::Chat => chat_tab(ui, state, v, &pal, &mut out),
            Tab::Tasks => tasks_tab(ui, state, v, &pal, &mut out),
            Tab::Skills => skills_tab(ui, state, v, &mut out),
            Tab::Styles => {
                let sv = crate::ui::styles_ui::View { styles: v.styles, applied: v.applied_style, context_open: v.context_open };
                match crate::ui::styles_ui::styles_tab(ui, &mut state.styles, &sv) {
                    crate::ui::styles_ui::Out::None => {}
                    o => out = Out::Styles(o),
                }
            }
        }
    });
    out
}

// ---------------------------------------------------------------------------------------------------
// sidebar

/// New chat, search, then the chats: pinned first, then the most recent, each with its date.
fn sidebar(ui: &mut egui::Ui, state: &mut AssistantState, v: &View) {
    let new = glyph_text_button_w(ui, Glyph::Letter('+'), "New chat", ui.available_width());
    if new.on_hover_text("Start a new chat").clicked() {
        state.new_chat();
    }
    ui.add_space(4.0);
    ui.add(egui::TextEdit::singleline(&mut state.search).hint_text("Search chats").desired_width(f32::INFINITY));
    ui.add_space(4.0);
    let order = sidebar_order(&state.convs, &state.search);
    let shown = state.conv().map(|c| c.id);
    let now = now_secs();
    let (mut open, mut delete, mut save) = (None, None, None);
    egui::ScrollArea::vertical().id_salt("assistant_chats").auto_shrink([false, false]).show(ui, |ui| {
        if order.is_empty() {
            ui.add_space(6.0);
            ui.weak(if state.search.is_empty() { "No chats yet." } else { "No chat matches." });
            return;
        }
        // a table: [label fills and truncates][date column]; no grip, the order is by date
        let mut rows = Rows::new(ui, "assistant_chat_rows", &[], &[34.0], false).fixed_order();
        // the row lit with the accent is the chat on screen, whatever was clicked last
        rows.select(ui, shown);
        let mut pinned_head = false;
        for (n, &i) in order.iter().enumerate() {
            let c = &state.convs[i];
            if c.pinned != pinned_head || n == 0 {
                pinned_head = c.pinned;
                if n > 0 {
                    ui.add_space(6.0);
                }
                // a section title, as the Library's
                let head = egui::RichText::new(if c.pinned { "Pinned" } else { "Recent" }).small().strong();
                ui.label(head.color(ui.visuals().weak_text_color()));
            }
            let (id, date) = (c.id, short_date(c.updated, now));
            let title = c.title().to_string();
            let r = rows.row(ui, n, &RowSpec { key: id, label: &title, values: &[&date], ..Default::default() });
            if state.renaming.as_ref().is_some_and(|(r, _)| *r == id) {
                rename_field(ui, state, r.label);
                continue;
            }
            if r.body.double_clicked() {
                state.renaming = Some((id, title.clone()));
            } else if r.body.clicked() {
                open = Some(id);
            }
            let r = r.body.on_hover_text(format!("{}
Double-click to rename, right-click for more", c.title()));
            menu::context(&r, |ui| {
                if menu::row(ui, None, "Rename", "").clicked() {
                    state.renaming = Some((id, title.clone()));
                }
                let pinned = state.convs.get(i).is_some_and(|c| c.pinned);
                if menu::row(ui, Some(Glyph::Pin), if pinned { "Unpin" } else { "Pin" }, "").clicked() {
                    state.toggle_pin(id);
                }
                if menu::row(ui, Some(Glyph::FloppyDisk), "Save chat…", "").clicked() {
                    save = Some(id);
                }
                ui.separator();
                let del = ui.add_enabled_ui(!(v.running && state.run_conv == Some(id)), |ui| menu::row(ui, Some(Glyph::Cross), "Delete", ""));
                if del.inner.on_disabled_hover_text("Wait for the assistant to finish").clicked() {
                    delete = Some(id);
                }
            });
        }
        let _ = rows.finish(ui);
    });
    if let Some(id) = open {
        state.open(id);
    }
    if let Some(id) = delete {
        state.delete(id);
    }
    if let Some(c) = save.and_then(|id| state.convs.iter().find(|c| c.id == id)) {
        save_chats_dialog(&c.title().replace(|ch: char| "/\\:*?\"<>|".contains(ch), "_"), &[c]);
    }
}

/// The rename box over a chat row: Enter or a click away keeps it, Esc cancels.
fn rename_field(ui: &mut egui::Ui, state: &mut AssistantState, at: egui::Rect) {
    let Some((id, buf)) = state.renaming.as_mut() else { return };
    let id = *id;
    let fid = egui::Id::new(("assistant_rename", id));
    let r = ui.put(at, egui::TextEdit::singleline(buf).id(fid).desired_width(at.width()));
    if !r.has_focus() && !r.lost_focus() {
        r.request_focus();
    }
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        state.renaming = None;
    } else if r.lost_focus() {
        let title = buf.trim().to_string();
        if !title.is_empty() {
            state.rename(id, &title);
        }
        state.renaming = None;
    }
}

// ---------------------------------------------------------------------------------------------------
// top bar

/// [sidebar toggle] the chat's title … [new chat] [⋯]: everything advanced is in the `⋯` menu.
fn top_bar(ui: &mut egui::Ui, state: &mut AssistantState, v: &View, wide: bool, out: &mut Out) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let pal = crate::ui::titlebar::palette(ui.ctx());
        let open = state.sidebar.unwrap_or(wide);
        let tip = if open { "Hide the chat list" } else { "Show the chat list" };
        if icon_button(ui, &pal, ui.id().with("assistant_list_toggle"), Glyph::ListIcon, tip, open).clicked() {
            state.sidebar = Some(!open);
        }
        if state.tab != Tab::Chat {
            if ui.button("← Chat").clicked() {
                state.tab = Tab::Chat;
            }
            ui.strong(match state.tab {
                Tab::Tasks => "Tasks",
                Tab::Styles => "Editing Styles",
                _ => "Skills",
            });
        }
        if state.tab == Tab::Chat {
            // the title takes what the two buttons on the right leave, and truncates
            let title = state.conv().map_or("New chat", Conversation::title).to_string();
            // (+ and ⋯, and the Full access badge when it shows: it overlapped a long title)
            let w = (ui.available_width() - if v.full_access { 150.0 } else { 64.0 }).max(20.0);
            ui.allocate_ui_with_layout(egui::vec2(w, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add(egui::Label::new(egui::RichText::new(title).strong()).truncate());
            });
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            more_menu(ui, state, v, out);
            if v.full_access {
                full_access_badge(ui);
            }
            if icon_button(ui, &pal, ui.id().with("assistant_new_chat"), Glyph::Letter('+'), "New chat", false).clicked() {
                state.new_chat();
            }
            // feat/styles-context: the Project Context pane, lit while it is open
            if crate::ui::styles_ui::context_button(ui, &pal, v.context_open) {
                *out = Out::Styles(crate::ui::styles_ui::Out::ToggleContext);
            }
        });
    });
    ui.add_space(2.0);
}

/// ws:mcp-permissions: "Full access" while every MCP call runs without asking.
pub fn full_access_badge(ui: &mut egui::Ui) {
    let c = ui.visuals().warn_fg_color;
    egui::Frame::new().stroke(egui::Stroke::new(1.0, c)).corner_radius(8u8).inner_margin(egui::Margin::symmetric(6, 1)).show(ui, |ui| {
        ui.label(egui::RichText::new("Full access").small().color(c));
    })
    .response
    .on_hover_text("Every MCP call runs without asking, blocked ones too (a development build, --mcp-full-access or                     SE_MCP_FULL_ACCESS=1). SE_MCP_FULL_ACCESS=0 turns it off.");
}

/// `⋯`: the agent, Tasks, Skills, "Handle my comments", import / export.
fn more_menu(ui: &mut egui::Ui, state: &mut AssistantState, v: &View, out: &mut Out) {
    menu::button(ui, " ⋯ ", |ui| {
        menu::sub(ui, Some(Glyph::Assistant), &format!("Agent: {}", agent_label(v.backend)), |ui| {
            for &(value, label) in AGENTS {
                if menu::check(ui, v.backend == value, label, "").clicked() && v.backend != value {
                    *out = Out::Backend(value);
                }
            }
        });
        ui.separator();
        let n = state.conv().map_or(0, |c| c.tasks.len());
        let tasks = if n > 0 { format!("Tasks ({n})") } else { "Tasks".into() };
        if menu::row(ui, Some(Glyph::Notepad), &tasks, "").on_hover_text("Scripts the assistant writes for you to run").clicked() {
            state.tab = Tab::Tasks;
        }
        if menu::row(ui, Some(Glyph::Bolt), "Skills", "").on_hover_text("Saved instructions you run in chat as /name").clicked() {
            state.tab = Tab::Skills;
        }
        // feat/styles-context
        let styles_tip = "Editing styles you set for a project with /style name, or learn from a video";
        if menu::row(ui, Some(Glyph::Star), "Editing Styles", "").on_hover_text(styles_tip).clicked() {
            state.tab = Tab::Styles;
        }
        let ctx_tip = "The project's notes, rules and style guide the assistant reads with every request";
        if menu::check(ui, v.context_open, "Project Context", "").on_hover_text(ctx_tip).clicked() {
            *out = Out::Styles(crate::ui::styles_ui::Out::ToggleContext);
        }
        let external = v.backend == "external";
        let label = if v.queued > 0 { format!("Handle my comments ({} waiting)", v.queued) } else { "Handle my comments".into() };
        let r = ui.add_enabled_ui(!external, |ui| menu::check(ui, state.comments, &label, "")).inner;
        let r = r
            .on_hover_text(
                "Every comment you leave on the timeline goes to the assistant, which works on it in the background \
                 and ticks it off when done. One at a time; later ones wait.",
            )
            .on_disabled_hover_text("An external agent gets comments through assistant.wait already");
        if r.clicked() {
            state.comments = !state.comments;
            *out = Out::Comments(state.comments);
        }
        ui.separator();
        if menu::row(ui, Some(Glyph::ImportArrow), "Import chats…", "").on_hover_text("Add chats from a file saved with Export").clicked() {
            if let Some(p) = rfd::FileDialog::new().add_filter("Chats", &["json"]).pick_file() {
                if let Ok(chats) = std::fs::read_to_string(&p).map_err(|e| e.to_string()).and_then(|s| parse_import(&s)) {
                    state.import(chats);
                }
            }
        }
        if menu::row(ui, Some(Glyph::ExportArrow), "Export chats…", "").on_hover_text("Save every chat of this project to a file").clicked() {
            let all: Vec<&Conversation> = state.convs.iter().filter(|c| !c.is_empty()).collect();
            save_chats_dialog("chats", &all);
        }
    })
    .response
    .on_hover_text("More: agent, tasks, skills, import and export");
}

/// Ask where, then write `chats` there (the download).
fn save_chats_dialog(name: &str, chats: &[&Conversation]) {
    if let Some(p) = rfd::FileDialog::new().add_filter("Chats", &["json"]).set_file_name(format!("{name}.json")).save_file() {
        let _ = std::fs::write(p, export_json(chats));
    }
}

/// The model menu under the composer: the latest models (keys 1-4 pick one while it is open, a tick on
/// the current one), Effort, More models (every listed version, the agent's default, Other…).
fn model_picker(ui: &mut egui::Ui, state: &mut AssistantState, v: &View, out: &mut Out) {
    match v.backend {
        "external" => {}
        "claude" if !state.model_other && (v.model.is_empty() || CLAUDE_MODELS.iter().any(|(id, _)| *id == v.model)) => {
            let shown = if v.model.is_empty() { "Default model" } else { model_label(v.model).trim_end_matches(" (latest)") };
            menu::button(ui, egui::RichText::new(format!("{shown} ▾")).small(), |ui| {
                const KEYS: [egui::Key; 4] = [egui::Key::Num1, egui::Key::Num2, egui::Key::Num3, egui::Key::Num4];
                for (i, &(id, label)) in CLAUDE_MODELS[..CLAUDE_ALIASES].iter().enumerate() {
                    let num = (i + 1).to_string();
                    let label = label.trim_end_matches(" (latest)");
                    let hit = ui.input(|inp| inp.key_pressed(KEYS[i]));
                    if menu::row(ui, None, label, if v.model == id { "✓" } else { num.as_str() }).clicked() || hit {
                        *out = Out::Model(id.to_string());
                        ui.close();
                    }
                }
                ui.separator();
                menu::sub(ui, None, "Effort", |ui| {
                    if menu::check(ui, v.effort.is_empty(), "Default", "").clicked() {
                        *out = Out::Effort(String::new());
                    }
                    for &(id, label) in EFFORTS {
                        if menu::check(ui, v.effort == id, label, "").clicked() {
                            *out = Out::Effort(id.to_string());
                        }
                    }
                })
                ;
                menu::sub(ui, None, "More models", |ui| {
                    if menu::check(ui, v.model.is_empty(), "Default model", "").clicked() {
                        *out = Out::Model(String::new());
                    }
                    ui.separator();
                    for &(id, label) in &CLAUDE_MODELS[CLAUDE_ALIASES..] {
                        if menu::check(ui, v.model == id, label, "").clicked() {
                            *out = Out::Model(id.to_string());
                        }
                    }
                    ui.separator();
                    if menu::row(ui, None, "Other…", "").clicked() {
                        state.model_other = true;
                    }
                });
            })
            .response
            .on_hover_text("Model and effort");
        }
        _ => {
            let mut m = v.model.to_string();
            let r = ui.add(egui::TextEdit::singleline(&mut m).hint_text("model").desired_width(80.0).font(egui::TextStyle::Small));
            if r.changed() {
                *out = Out::Model(m.trim().to_string());
            }
            if r.lost_focus() && m.trim().is_empty() {
                state.model_other = false; // back to the list
            }
            r.on_hover_text("Model name passed to the agent (empty = its default)");
        }
    }
}

/// A small × that removes something.
fn x_button(ui: &mut egui::Ui, salt: impl std::hash::Hash) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
    let r = ui.interact(rect, ui.id().with(salt), egui::Sense::click());
    let color = if r.hovered() { ui.visuals().strong_text_color() } else { ui.visuals().weak_text_color() };
    draw_glyph(ui.painter(), rect, Glyph::Cross, color);
    r
}

// ---------------------------------------------------------------------------------------------------
// tasks

fn tasks_tab(ui: &mut egui::Ui, state: &mut AssistantState, v: &View, pal: &Palette, out: &mut Out) {
    let external = v.backend == "external";
    // ws:skills: `/name` works here too
    let tid = ui.id().with("assistant_task_prompt");
    let (slash, done) = slash_list(ui, &mut state.task_prompt, &mut state.slash_sel, v.skills, &[], tid);
    ui.horizontal(|ui| {
        let w = (ui.available_width() - 48.0).max(80.0);
        let r = ui.add(
            egui::TextEdit::singleline(&mut state.task_prompt)
                .id(tid)
                .hint_text("Describe a task, or / for a skill…")
                .desired_width(w)
                .lock_focus(slash),
        );
        if done {
            r.request_focus();
            cursor_to_end(ui.ctx(), tid, &state.task_prompt);
        }
        let can = !state.task_prompt.trim().is_empty() && !external;
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let add = ui.add_enabled(can, egui::Button::new("Add"));
        let add = if external {
            add.on_disabled_hover_text("Tasks need an agent the app runs - pick one in the header")
        } else {
            add.on_hover_text("The assistant writes a script for it; nothing changes until you run it")
        };
        if can && (enter || add.clicked()) {
            *out = Out::AddTask;
        }
    });
    let tasks = state.conv().map_or(&[][..], |c| c.tasks.as_slice());
    let ready = tasks.iter().filter(|t| t.status == TaskStatus::Ready && t.runnable()).count();
    ui.horizontal(|ui| {
        let r = ui.add_enabled(ready > 0, egui::Button::new(format!("Run all ({ready})")).small());
        if r.on_hover_text("Run the Ready tasks top to bottom, each one undo step; a failure stops the queue").clicked() {
            *out = Out::Run(None);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if v.queued > 0 {
                ui.weak(format!("{} waiting", v.queued));
            }
            let mut on = state.comments;
            let r = ui.add_enabled(!external, egui::Checkbox::new(&mut on, "Handle my comments"));
            let r = r
                .on_hover_text(
                    "Every comment you leave on the timeline goes to the assistant, which works on it in the \
                     background and ticks it off when done. One at a time; later ones wait.",
                )
                .on_disabled_hover_text("An external agent gets comments through assistant.wait already");
            if r.changed() {
                state.comments = on;
                *out = Out::Comments(on);
            }
        });
    });
    ui.add_space(2.0);
    egui::ScrollArea::vertical().id_salt("assistant_tasks").auto_shrink([false, false]).show(ui, |ui| {
        if state.conv().is_none_or(|c| c.tasks.is_empty()) {
            ui.add_space(12.0);
            ui.vertical_centered(|ui| {
                ui.weak(
                    "Describe a change and the assistant writes it as a script you can review, edit and run. \
                     Each run is one undo step.",
                );
            });
            return;
        }
        let ids: Vec<u64> = state.conv().map(|c| c.tasks.iter().map(|t| t.id).collect()).unwrap_or_default();
        for id in ids {
            task_card(ui, state, id, pal, out);
            ui.add_space(4.0);
        }
    });
}

fn task_card(ui: &mut egui::Ui, state: &mut AssistantState, id: u64, pal: &Palette, out: &mut Out) {
    let Some(t) = state.conv().and_then(|c| c.tasks.iter().find(|t| t.id == id)).cloned() else { return };
    let error = ui.visuals().error_fg_color;
    egui::Frame::new()
        .fill(pal.panel)
        .stroke(egui::Stroke::new(1.0, pal.border))
        .corner_radius(pal.rounding as u8 + 2)
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let color = match t.status {
                    TaskStatus::Failed => error,
                    TaskStatus::Done => pal.text_dim,
                    TaskStatus::Ready => pal.accent,
                    _ => pal.text_dim,
                };
                ui.label(egui::RichText::new(t.status.label()).small().color(color));
                ui.label(egui::RichText::new(&t.title).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if x_button(ui, ("task_rm", id)).on_hover_text("Remove").clicked() {
                        task_remove(state.tasks_mut(), id);
                        state.dirty = true;
                    }
                    if ui.small_button("↓").on_hover_text("Later in the queue").clicked() {
                        task_move(state.tasks_mut(), id, false);
                        state.dirty = true;
                    }
                    if ui.small_button("↑").on_hover_text("Earlier in the queue").clicked() {
                        task_move(state.tasks_mut(), id, true);
                        state.dirty = true;
                    }
                });
            });
            if !t.note.is_empty() {
                ui.add(egui::Label::new(egui::RichText::new(&t.note).weak()).truncate());
            }
            if t.status == TaskStatus::Failed && !t.error.is_empty() {
                ui.colored_label(error, short(&t.error, 300));
            }
            let editing = state.editing.as_ref().is_some_and(|(e, _)| *e == id);
            if editing {
                let mut save = None;
                if let Some((_, buf)) = state.editing.as_mut() {
                    ui.add(
                        egui::TextEdit::multiline(buf)
                            .code_editor()
                            .desired_rows(8)
                            .desired_width(f32::INFINITY),
                    );
                    ui.horizontal(|ui| {
                        if ui.button("Save").clicked() {
                            save = Some(Some(buf.clone()));
                        }
                        if ui.button("Cancel").clicked() {
                            save = Some(None);
                        }
                    });
                }
                if let Some(code) = save {
                    if let Some(code) = code {
                        let _ = task_set(state.tasks_mut(), id, None, Some(&code));
                        state.dirty = true;
                    }
                    state.editing = None;
                }
                return;
            }
            ui.horizontal(|ui| {
                let run = ui.add_enabled(t.runnable(), egui::Button::new("Run").small());
                if run.on_hover_text("Run it now, as one undo step").clicked() {
                    *out = Out::Run(Some(id));
                }
                let edit = ui.add_enabled(t.status != TaskStatus::Writing, egui::Button::new("Edit").small());
                if edit.on_hover_text("Read or change the script").clicked() {
                    state.editing = Some((id, t.code.clone()));
                }
                let save = ui.add_enabled(!t.code.trim().is_empty(), egui::Button::new("Save to Scripts").small());
                if save.on_hover_text("Keep it in the Scripts menu for reuse").clicked() {
                    *out = Out::SaveScript(id);
                }
            });
        });
}

// ---------------------------------------------------------------------------------------------------
// chat

fn chat_tab(ui: &mut egui::Ui, state: &mut AssistantState, v: &View, pal: &Palette, out: &mut Out) {
    // the composer is laid out first, as a bottom panel sized by its own content (the chips wrap to as
    // many rows as the width needs); the conversation takes what is left
    egui::TopBottomPanel::bottom(ui.id().with("assistant_composer"))
        .frame(egui::Frame::NONE)
        .resizable(false)
        .show_inside(ui, |ui| composer(ui, state, v, pal, out));
    egui::ScrollArea::vertical()
        .id_salt("assistant_log")
        .stick_to_bottom(true)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if state.log().is_empty() {
                empty_chat(ui, state, v, out);
            } else {
                conversation(ui, state, v, pal, out);
            }
            if v.running && state.run_conv.is_none_or(|id| state.conv().is_some_and(|c| c.id == id)) {
                ui.weak("Working…");
            }
        });
}

fn empty_chat(ui: &mut egui::Ui, state: &mut AssistantState, v: &View, out: &mut Out) {
    ui.add_space((ui.available_height() * 0.25).clamp(12.0, 120.0));
    ui.vertical_centered(|ui| {
        ui.label(egui::RichText::new("What should we work on?").size(18.0).strong());
        ui.add_space(4.0);
        ui.weak(match v.backend {
            "external" => "Requests wait for a connected agent (assistant.wait).",
            "custom" => "Requests run your custom command (Settings ▸ Assistant).",
            _ => "Ask for an edit, attach a frame, a panel or a file. Every request is one undo step.",
        });
        ui.add_space(10.0);
        // ws:skills: the skills the user ran last, then the first ones; each chip is one Button so the
        // row wraps instead of widening the pane
        ui.horizontal_wrapped(|ui| {
            for s in chip_skills(v.skills, v.recent, CHIPS) {
                let r = ui.add_enabled(!v.running, egui::Button::new(format!("/{}", s.name)).small().corner_radius(8u8));
                if r.on_hover_text(s.description.as_str()).clicked() {
                    run_skill(state, s, out);
                }
            }
        });
    });
}

/// How many skill chips an empty chat shows.
const CHIPS: usize = 5;

/// The chip row: skills from `recent` that still exist, then the rest in list order; at most `n`.
pub fn chip_skills<'a>(all: &'a [Skill], recent: &[String], n: usize) -> Vec<&'a Skill> {
    let mut out: Vec<&Skill> = recent.iter().filter_map(|r| all.iter().find(|s| s.name == *r)).collect();
    out.extend(all.iter().filter(|s| !recent.contains(&s.name)));
    out.truncate(n);
    out
}

/// Run a skill from a chip or the Skills tab: one without arguments goes at once; one that takes
/// arguments is typed into the chat prompt for the user to finish.
pub fn run_skill(state: &mut AssistantState, s: &Skill, out: &mut Out) {
    state.tab = Tab::Chat;
    if s.args.is_empty() {
        state.prompt = format!("/{}", s.name);
        *out = Out::Send;
    } else {
        state.prompt = format!("/{} ", s.name);
        state.focus_prompt = true;
        state.cursor_end = true;
    }
}

/// The text after a leading `/` while a skill name is still being typed (no space yet).
fn slash_query(prompt: &str) -> Option<&str> {
    prompt.strip_prefix('/').filter(|q| !q.contains(char::is_whitespace))
}

// ---------------------------------------------------------------------------------------------------
// skills (ws:skills)

fn skills_tab(ui: &mut egui::Ui, state: &mut AssistantState, v: &View, out: &mut Out) {
    ui.horizontal_wrapped(|ui| {
        let r = ui.button("New skill").on_hover_text("Describe it and let the assistant write it, or write it yourself");
        if r.clicked() {
            state.skill_draft = Some(SkillDraft::default());
        }
        if ui.button("Open skills folder").on_hover_text("Your skills are Markdown files in this folder").clicked() {
            skills::open_folder();
        }
    });
    ui.add_space(2.0);
    egui::ScrollArea::vertical().id_salt("assistant_skills").auto_shrink([false, false]).show(ui, |ui| {
        if v.skills.is_empty() {
            ui.weak("No skills yet.");
            return;
        }
        for s in v.skills {
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new(format!("/{}", s.name)).strong());
                if !s.args.is_empty() {
                    ui.label(egui::RichText::new(&s.args).small().weak());
                }
                ui.label(egui::RichText::new(s.source.label()).small().weak());
            });
            if !s.description.is_empty() {
                ui.add(egui::Label::new(egui::RichText::new(&s.description).weak()).wrap());
            }
            ui.horizontal_wrapped(|ui| {
                let run = ui.add_enabled(!v.running, egui::Button::new("Run").small());
                let tip = if s.args.is_empty() { "Run it now" } else { "Type what it should work on in Chat, then send" };
                if run.on_hover_text(tip).on_disabled_hover_text("Wait for the assistant to finish").clicked() {
                    run_skill(state, s, out);
                }
                if s.source == Source::User {
                    if ui.small_button("Edit").clicked() {
                        state.skill_draft = Some(SkillDraft::edit(s));
                    }
                    if ui.small_button("Duplicate").clicked() {
                        state.skill_draft = Some(SkillDraft::copy(s, v.skills));
                    }
                    if ui.small_button("Delete").clicked() {
                        *out = Out::Skill(SkillOut::Delete(s.name.clone()));
                    }
                } else if ui.small_button("Duplicate to edit").on_hover_text("Make your own copy to change").clicked() {
                    state.skill_draft = Some(SkillDraft::copy(s, v.skills));
                }
            });
            ui.add_space(6.0);
        }
    });
}

/// The New / Edit skill window (non-modal; WINDOW_DRAWERS through `assistant_ctl`). Closes itself on
/// Cancel; the app closes it after a Save or "Write it for me".
pub fn skill_window(ctx: &egui::Context, state: &mut AssistantState, running: bool) -> Out {
    let mut out = Out::None;
    let Some(d) = state.skill_draft.as_mut() else { return out };
    let (mut open, mut cancel) = (true, false);
    let title = if d.editing.is_some() { "Edit skill" } else { "New skill" };
    egui::Window::new(title)
        .id(egui::Id::new("assistant_skill_window"))
        .open(&mut open)
        .collapsible(false)
        .default_width(440.0)
        .show(ctx, |ui| {
            ui.label("Name");
            ui.add(egui::TextEdit::singleline(&mut d.name).hint_text("vlog-intro").desired_width(f32::INFINITY));
            let name_ok = skills::valid_name(&d.name);
            if !d.name.is_empty() && !name_ok {
                ui.colored_label(ui.visuals().warn_fg_color, "Lowercase letters, digits and - only; you run it as /name");
            }
            if !d.plain {
                ui.label("What should it do?");
                ui.add(
                    egui::TextEdit::multiline(&mut d.what)
                        .hint_text("Add an intro title with my channel name, then a whoosh sound")
                        .desired_rows(4)
                        .desired_width(f32::INFINITY),
                );
                ui.horizontal(|ui| {
                    let can = !d.what.trim().is_empty() && !running && (d.name.is_empty() || name_ok);
                    let r = ui.add_enabled(can, egui::Button::new("Write it for me"));
                    let r = r
                        .on_hover_text("The assistant writes the skill and saves it; it then shows in the Skills list")
                        .on_disabled_hover_text("Say what it should do first (and wait for the assistant to finish)");
                    if r.clicked() {
                        out = Out::Skill(SkillOut::Draft);
                    }
                    if ui.button("Write it myself").clicked() {
                        d.plain = true;
                        if d.description.is_empty() {
                            d.description = short(&d.what, 80);
                        }
                        if d.body.is_empty() {
                            d.body = d.what.clone();
                        }
                    }
                });
                return;
            }
            ui.label("Description");
            ui.add(egui::TextEdit::singleline(&mut d.description).hint_text("One line, shown in the / list").desired_width(f32::INFINITY));
            ui.label("Arguments hint (optional)");
            ui.add(egui::TextEdit::singleline(&mut d.args).hint_text("[style]").desired_width(f32::INFINITY));
            ui.label("Instructions");
            ui.weak("$ARGS is replaced by what is typed after /name.");
            egui::ScrollArea::vertical().id_salt("skill_body").max_height(320.0).show(ui, |ui| {
                ui.add(egui::TextEdit::multiline(&mut d.body).code_editor().desired_rows(12).desired_width(f32::INFINITY));
            });
            if !d.error.is_empty() {
                ui.colored_label(ui.visuals().error_fg_color, d.error.as_str());
            }
            ui.horizontal(|ui| {
                let can = name_ok && !d.description.trim().is_empty() && !d.body.trim().is_empty();
                if ui.add_enabled(can, egui::Button::new("Save")).clicked() {
                    out = Out::Skill(SkillOut::Save);
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
    if !open || cancel {
        state.skill_draft = None;
    }
    out
}

/// A library skill's first run: its whole text, then Run / Cancel. Some(true) = run it.
pub fn read_first_window(ctx: &egui::Context, s: &Skill) -> Option<bool> {
    let mut answer = None;
    egui::Window::new(format!("Run /{}?", s.name))
        .id(egui::Id::new("assistant_skill_read_first"))
        .collapsible(false)
        .default_width(480.0)
        .show(ctx, |ui| {
            ui.label(
                "This skill comes from the library. A skill is instructions the assistant follows, so read it \
                 before it runs the first time.",
            );
            ui.strong(s.description.as_str());
            egui::ScrollArea::vertical().id_salt("skill_read_first").max_height(340.0).show(ui, |ui| {
                ui.add(egui::Label::new(egui::RichText::new(&s.body).monospace().small()).wrap());
            });
            ui.horizontal(|ui| {
                if ui.button("Run").clicked() {
                    answer = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    answer = Some(false);
                }
            });
        });
    answer
}

/// What a message's right-click menu asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MsgAct {
    CopyText,
    CopyMarkdown,
    CopyAttachments,
    Retry,
    Edit,
    Delete,
}

/// The rows of a message's right-click menu (shared with the tests): what each one is and whether it
/// applies to `m` while `running`.
pub fn msg_menu_rows(lines: &[Line], m: &Msg, running: bool) -> Vec<(MsgAct, &'static str, bool)> {
    let atts = !msg_attachments(lines, m).is_empty();
    let user = m.user || lines[..m.lines.start].iter().any(|l| matches!(l, Line::User(_)));
    let mut rows = vec![(MsgAct::CopyText, "Copy text", true), (MsgAct::CopyMarkdown, "Copy as Markdown", true)];
    if m.user {
        rows.push((MsgAct::CopyAttachments, "Copy attachments", atts));
        rows.push((MsgAct::Edit, "Edit and resend", !running));
    }
    rows.push((MsgAct::Retry, "Retry", !running && user));
    rows.push((MsgAct::Delete, "Delete", !running));
    rows
}

/// Carry out a message menu row (the copies go to the clipboard through `ctx`).
pub fn msg_act(ctx: &egui::Context, state: &mut AssistantState, m: &Msg, act: MsgAct, out: &mut Out) {
    let lines = state.log().to_vec();
    match act {
        MsgAct::CopyText => ctx.copy_text(msg_plain(&lines, m)),
        MsgAct::CopyMarkdown => ctx.copy_text(msg_markdown(&lines, m)),
        MsgAct::CopyAttachments => copy_attachments(ctx, &msg_attachments(&lines, m)),
        MsgAct::Edit => state.edit_msg(m),
        MsgAct::Delete => state.delete_msg(m),
        MsgAct::Retry => {
            // a reply retries the user message before it
            let at = if m.user { Some(m.lines.start) } else { lines[..m.lines.start].iter().rposition(|l| matches!(l, Line::User(_))) };
            if let Some(i) = at {
                *out = Out::Retry(i);
            }
        }
    }
}

fn conversation(ui: &mut egui::Ui, state: &mut AssistantState, v: &View, pal: &Palette, out: &mut Out) {
    let lines = state.log().to_vec();
    let last_done = lines.iter().rposition(|l| matches!(l, Line::Done { .. }));
    let conv = state.conv().map_or(0, |c| c.id);
    let mut acted = None;
    for (k, m) in messages(&lines).into_iter().enumerate() {
        if k > 0 {
            ui.add_space(10.0);
        }
        let r = ui.scope(|ui| {
            if m.user {
                user_msg(ui, state, &lines, &m, pal);
            } else {
                reply(ui, state, v, &lines, &m, last_done, conv, out);
            }
        });
        let id = egui::Id::new(("assistant_msg", conv, m.lines.start));
        menu::context_area(ui, id, r.response.rect, |ui| {
            for (act, label, on) in msg_menu_rows(&lines, &m, v.running) {
                if matches!(act, MsgAct::Edit | MsgAct::Delete) {
                    ui.separator();
                }
                let glyph = match act {
                    MsgAct::CopyText | MsgAct::CopyMarkdown | MsgAct::CopyAttachments => Some(Glyph::Copy),
                    MsgAct::Retry => Some(Glyph::UndoArrow(super::tools::Dir::Right)),
                    MsgAct::Delete => Some(Glyph::Cross),
                    MsgAct::Edit => Some(Glyph::Pencil),
                };
                if ui.add_enabled_ui(on, |ui| menu::row(ui, glyph, label, "")).inner.clicked() {
                    acted = Some((m.clone(), act));
                }
            }
        });
    }
    if let Some((m, act)) = acted {
        msg_act(ui.ctx(), state, &m, act, out);
    }
}

/// The user's message: a right-aligned rounded bubble (at most ~85 % of the width), its attachments under it.
fn user_msg(ui: &mut egui::Ui, state: &mut AssistantState, lines: &[Line], m: &Msg, pal: &Palette) {
    let text = msg_markdown(lines, m);
    let font = egui::TextStyle::Body.resolve(ui.style());
    let natural = ui.painter().layout_no_wrap(text.clone(), font, pal.text).size().x;
    let margin = egui::Margin::symmetric(12, 8);
    let max = (ui.available_width() * 0.85).max(60.0);
    let w = natural.min(max - margin.sum().x);
    ui.horizontal(|ui| {
        ui.add_space((ui.available_width() - w - margin.sum().x).max(0.0));
        let fill = ui.visuals().widgets.inactive.weak_bg_fill;
        egui::Frame::new().fill(fill).corner_radius(bubble_radius(ui)).inner_margin(margin).show(ui, |ui| {
            ui.set_max_width(w);
            ui.add(egui::Label::new(egui::RichText::new(&text).color(ui.visuals().strong_text_color())).wrap());
        });
    });
    let chips: Vec<String> = lines[m.lines.clone()]
        .iter()
        .flat_map(|l| match l {
            Line::Attached { chips, .. } => chips.clone(),
            _ => Vec::new(),
        })
        .collect();
    let atts = msg_attachments(lines, m);
    if atts.is_empty() && chips.is_empty() {
        return;
    }
    ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
        ui.horizontal_wrapped(|ui| {
            for (i, a) in atts.iter().enumerate() {
                attachment(ui, state, a, 0.8, ("sent", m.lines.start, i), false);
            }
            for c in &chips {
                ui.label(egui::RichText::new(c).small().weak());
            }
        });
    });
}

/// The assistant's reply: its text as Markdown, each run of tool calls folded into one "N steps" line,
/// errors and notes, and Undo / Copy / Retry under it once it finished.
#[allow(clippy::too_many_arguments)]
fn reply(ui: &mut egui::Ui, state: &mut AssistantState, v: &View, lines: &[Line], m: &Msg, last_done: Option<usize>, conv: u64, out: &mut Out) {
    let mut i = m.lines.start;
    while i < m.lines.end {
        match &lines[i] {
            Line::Text(t) => crate::ui::markdown::show(ui, t, &crate::ui::titlebar::palette(ui.ctx())),
            Line::Tool(_) => {
                let n = lines[i..m.lines.end].iter().take_while(|l| matches!(l, Line::Tool(_))).count();
                let label = egui::RichText::new(format!("{n} step{}", if n == 1 { "" } else { "s" })).small();
                let vis = ui.visuals().clone();
                egui::Frame::new()
                    .fill(vis.faint_bg_color)
                    .stroke(vis.widgets.noninteractive.bg_stroke)
                    .corner_radius(bubble_radius(ui))
                    .inner_margin(egui::Margin::symmetric(6, 2))
                    .show(ui, |ui| {
                        let head = egui::CollapsingHeader::new(label).id_salt(("assistant_tools", conv, i)).default_open(false);
                        head.show(ui, |ui| {
                            for l in &lines[i..i + n] {
                                if let Line::Tool(t) = l {
                                    let t = short(t, 300);
                                    let (name, args) = t.split_once(' ').unwrap_or((t.as_str(), ""));
                                    ui.horizontal_wrapped(|ui| {
                                        ui.spacing_mut().item_spacing.x = 6.0;
                                        ui.label(egui::RichText::new(name).monospace().small().color(vis.text_color()));
                                        if !args.is_empty() {
                                            ui.add(egui::Label::new(egui::RichText::new(args).monospace().small().weak()).wrap());
                                        }
                                    });
                                }
                            }
                        })
                        .header_response
                        .on_hover_text("The edits the assistant made");
                    });
                i += n;
                continue;
            }
            Line::Error(t) => {
                ui.colored_label(ui.visuals().error_fg_color, t);
            }
            Line::Info(t) => {
                ui.weak(t);
            }
            Line::Done { undo } => {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    let ok = undo_state(undo, v.undo_top);
                    // only the newest request can still be the top entry; older ones just show Copy
                    if Some(i) == last_done || ok.is_ok() {
                        let r = ui.add_enabled(ok.is_ok() && !v.running, egui::Button::new(egui::RichText::new("Undo").small()).frame_when_inactive(false));
                        let r = match ok {
                            Ok(()) => r.on_hover_text("Undo everything this request did, in one step"),
                            Err(why) => r.on_disabled_hover_text(why),
                        };
                        if r.clicked() {
                            *out = Out::Undo;
                        }
                    }
                    let text = msg_plain(lines, m);
                    let copy = ui.add_enabled(!text.is_empty(), egui::Button::new(egui::RichText::new("Copy").small()).frame_when_inactive(false));
                    if copy.on_hover_text("Copy the reply as plain text (right-click for Markdown)").clicked() {
                        ui.ctx().copy_text(text);
                    }
                    if Some(i) == last_done {
                        let retry = ui.add_enabled(!v.running, egui::Button::new(egui::RichText::new("Retry").small()).frame_when_inactive(false));
                        if retry.on_hover_text("Send the request again").clicked() {
                            msg_act(ui.ctx(), state, m, MsgAct::Retry, out);
                        }
                    }
                });
            }
            Line::User(_) | Line::Attached { .. } => {}
        }
        i += 1;
    }
}

// ---------------------------------------------------------------------------------------------------
// attachments

/// An image as egui pixels (None when the file is missing or not a PNG).
pub fn load_color_image(png: &std::path::Path) -> Option<egui::ColorImage> {
    let f = crate::engine::html::decode_png(&std::fs::read(png).ok()?).ok()?;
    Some(egui::ColorImage::from_rgba_unmultiplied([f.width as usize, f.height as usize], &f.rgba))
}

/// Copy one attachment: a picture as an image (it pastes into any app), any other file as its path.
pub fn copy_attachment(ctx: &egui::Context, a: &Attachment) {
    match a.is_image().then(|| load_color_image(&a.png)).flatten() {
        Some(img) => ctx.copy_image(img),
        None => ctx.copy_text(a.path().display().to_string()),
    }
}

/// Copy a message's attachments: one picture as an image, else their paths, one per line.
pub fn copy_attachments(ctx: &egui::Context, list: &[Attachment]) {
    match list {
        [a] => copy_attachment(ctx, a),
        _ => ctx.copy_text(list.iter().map(|a| a.path().display().to_string()).collect::<Vec<_>>().join("\n")),
    }
}

/// The file name "Save as…" suggests: the attachment's name, with `.png` for a picture.
pub fn save_name(a: &Attachment) -> String {
    let name: String = a.name.chars().map(|c| if "/\\:*?\"<>|".contains(c) { '_' } else { c }).collect();
    if a.is_image() && !name.to_lowercase().ends_with(".png") {
        format!("{name}.png")
    } else {
        name
    }
}

/// Write the attachment (the picture's PNG, or the attached file) to `to`.
pub fn save_attachment(a: &Attachment, to: &std::path::Path) -> Result<(), String> {
    let from = if a.is_image() { a.png.as_path() } else { a.path() };
    std::fs::copy(from, to).map(|_| ()).map_err(|e| format!("Couldn't save {}: {e}", a.name))
}

fn save_attachment_dialog(a: &Attachment) {
    if let Some(to) = rfd::FileDialog::new().set_file_name(save_name(a)).save_file() {
        let _ = save_attachment(a, &to);
    }
}

/// An attachment: a picture's thumbnail (`scale` of `THUMB`) or a file chip. A click previews a picture
/// (opens a file); right-click: Copy, Copy path, Save as… (and Remove in the composer). True = remove it.
fn attachment(ui: &mut egui::Ui, state: &mut AssistantState, a: &Attachment, scale: f32, salt: impl std::hash::Hash, removable: bool) -> bool {
    let r = if a.is_image() {
        thumb(ui, state, a, scale)
    } else {
        let r = ui.add(egui::Button::new(egui::RichText::new(format!("▤ {}", short(&a.name, 28))).small()).corner_radius(6u8));
        r.on_hover_text(a.path().display().to_string())
    };
    let r = ui.interact(r.rect, ui.id().with(("att", salt)), egui::Sense::click());
    if r.clicked() {
        if a.is_image() {
            state.preview = Some(a.clone());
        } else {
            crate::engine::browsers::open_url(&a.path().display().to_string());
        }
    }
    let mut remove = false;
    menu::context(&r, |ui| {
        let what = if a.is_image() { "Copy image" } else { "Copy path" };
        if menu::row(ui, Some(Glyph::Copy), what, "").clicked() {
            copy_attachment(ui.ctx(), a);
        }
        if a.is_image() && menu::row(ui, None, "Copy path", "").clicked() {
            ui.ctx().copy_text(a.path().display().to_string());
        }
        if menu::row(ui, Some(Glyph::FloppyDisk), "Save as…", "").clicked() {
            save_attachment_dialog(a);
        }
        if removable {
            ui.separator();
            if menu::row(ui, Some(Glyph::Cross), "Remove", "").clicked() {
                remove = true;
            }
        }
    });
    remove
}

/// An attachment's thumbnail (loaded from its file once), `scale` of `THUMB`.
fn thumb(ui: &mut egui::Ui, state: &mut AssistantState, a: &Attachment, scale: f32) -> egui::Response {
    let ctx = ui.ctx().clone();
    let tex = state
        .thumbs
        .entry(a.thumb.clone())
        .or_insert_with(|| {
            let rgba = read_thumb(&a.thumb)?;
            let img = egui::ColorImage::from_rgba_unmultiplied([THUMB[0] as usize, THUMB[1] as usize], &rgba);
            Some(ctx.load_texture(a.thumb.to_string_lossy(), img, egui::TextureOptions::LINEAR))
        })
        .clone();
    let size = egui::vec2(THUMB[0] as f32, THUMB[1] as f32) * scale;
    match tex {
        Some(t) => ui
            .add(egui::Image::new(egui::load::SizedTexture::new(t.id(), size)).corner_radius(4u8))
            .on_hover_text(format!("{} · {}×{} - click to preview, right-click to copy or save", a.name, a.width, a.height)),
        None => ui.weak(format!("[{}]", a.name)),
    }
}

/// The attachment preview (WINDOW_DRAWERS through `assistant_ctl`): the picture at up to its own size,
/// with Copy and Save as…; closes with its ×.
pub fn preview_window(ctx: &egui::Context, state: &mut AssistantState) {
    let Some(a) = state.preview.clone() else { return };
    if state.preview_tex.as_ref().is_none_or(|(p, _)| *p != a.png) {
        let tex = load_color_image(&a.png).map(|img| ctx.load_texture(format!("assistant_preview {}", a.png.display()), img, egui::TextureOptions::LINEAR));
        state.preview_tex = Some((a.png.clone(), tex));
    }
    let tex = state.preview_tex.as_ref().and_then(|(_, t)| t.clone());
    let mut open = true;
    egui::Window::new(short(&a.name, 60))
        .id(egui::Id::new("assistant_preview"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size([640.0, 420.0])
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.weak(format!("{}×{}", a.width, a.height));
                if ui.small_button("Copy image").clicked() {
                    copy_attachment(ui.ctx(), &a);
                }
                if ui.small_button("Save as…").clicked() {
                    save_attachment_dialog(&a);
                }
            });
            match tex {
                Some(t) => {
                    let s = t.size_vec2();
                    let fit = (ui.available_width() / s.x).min(ui.available_height().max(120.0) / s.y).min(1.0);
                    ui.add(egui::Image::new(egui::load::SizedTexture::new(t.id(), s * fit)));
                }
                None => {
                    ui.weak("The picture's file is gone.");
                }
            }
        });
    if !open {
        state.preview = None;
        state.preview_tex = None;
    }
}

/// "Select an area" (WINDOW_DRAWERS through `assistant_ctl`): while `state.area`, the window dims and a
/// rectangle dragged anywhere is returned (points) for the app to capture; Esc or a right-click cancels.
pub fn area_overlay(ctx: &egui::Context, state: &mut AssistantState) -> Option<egui::Rect> {
    if !state.area {
        return None;
    }
    let screen = ctx.content_rect();
    let mut picked = None;
    egui::Area::new(egui::Id::new("assistant_area_pick"))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            let r = ui.allocate_rect(screen, egui::Sense::drag());
            ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
            let p = ui.painter();
            let dim = egui::Color32::from_black_alpha(110);
            let sel = match (state.area_from, ctx.pointer_interact_pos()) {
                (Some(a), Some(b)) => Some(egui::Rect::from_two_pos(a, b)),
                _ => None,
            };
            match sel {
                // dim everything but the selection
                Some(s) => {
                    for band in [
                        egui::Rect::from_x_y_ranges(screen.x_range(), screen.top()..=s.top()),
                        egui::Rect::from_x_y_ranges(screen.x_range(), s.bottom()..=screen.bottom()),
                        egui::Rect::from_x_y_ranges(screen.left()..=s.left(), s.y_range()),
                        egui::Rect::from_x_y_ranges(s.right()..=screen.right(), s.y_range()),
                    ] {
                        p.rect_filled(band, 0.0, dim);
                    }
                    p.rect_stroke(s, 0.0, egui::Stroke::new(1.5, egui::Color32::WHITE), egui::StrokeKind::Outside);
                    let size = format!("{:.0} × {:.0}", s.width() * ctx.pixels_per_point(), s.height() * ctx.pixels_per_point());
                    p.text(s.left_bottom() + egui::vec2(0.0, 4.0), egui::Align2::LEFT_TOP, size, egui::FontId::proportional(12.0), egui::Color32::WHITE);
                }
                None => {
                    p.rect_filled(screen, 0.0, dim);
                    p.text(
                        screen.center_top() + egui::vec2(0.0, 40.0),
                        egui::Align2::CENTER_TOP,
                        "Drag to select the area to attach  ·  Esc cancels",
                        egui::FontId::proportional(15.0),
                        egui::Color32::WHITE,
                    );
                }
            }
            if r.drag_started() {
                state.area_from = r.interact_pointer_pos();
            }
            if r.drag_stopped() {
                picked = sel.filter(|s| s.width() >= 4.0 && s.height() >= 4.0);
                state.area_from = None;
                state.area = false;
            }
        });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape) || i.pointer.secondary_clicked()) {
        (state.area, state.area_from) = (false, None);
    }
    picked
}

/// A chip: a label and an ×, one widget so a row of them wraps. True when it was clicked (= remove).
fn chip(ui: &mut egui::Ui, pal: &Palette, label: &str, pinned: bool) -> bool {
    let text = egui::RichText::new(format!("{label}  ×")).small();
    // pinned: the accent's selection tint; live: a plain widget fill
    let vis = ui.visuals();
    let button = egui::Button::new(text)
        .fill(if pinned { vis.selection.bg_fill.gamma_multiply(0.35) } else { vis.widgets.inactive.weak_bg_fill })
        .stroke(egui::Stroke::new(1.0, if pinned { pal.accent } else { vis.widgets.noninteractive.bg_stroke.color }))
        .corner_radius(bubble_radius(ui));
    ui.add(button)
        .on_hover_text(if pinned {
            "Added on purpose: sent with the next message. Click to remove."
        } else {
            "Live: follows the selection. Click to leave it out of the next message."
        })
        .clicked()
}

/// The prompt box grows with the text up to this many rows, then scrolls.
const PROMPT_ROWS: f32 = 8.0;

/// Bubbles, the composer, chips and step rows: the theme's widget corners (Settings ▸ Appearance), as the
/// Library and Inspector.
fn bubble_radius(ui: &egui::Ui) -> egui::CornerRadius {
    ui.visuals().widgets.inactive.corner_radius
}

fn composer(ui: &mut egui::Ui, state: &mut AssistantState, v: &View, pal: &Palette, out: &mut Out) {
    ui.add_space(4.0);
    // ws:mcp-permissions: the running chat's questions ("Allow media.import for this chat?")
    for (i, q, detail) in v.asks {
        egui::Frame::new()
            .fill(ui.visuals().faint_bg_color)
            .stroke(egui::Stroke::new(1.0, pal.accent))
            .corner_radius(bubble_radius(ui))
            .inner_margin(egui::Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(egui::RichText::new(q).strong());
                if !detail.is_empty() {
                    ui.weak(detail.as_str());
                }
                if let Some(c) = answer_buttons(ui) {
                    *out = Out::Answer(*i, c);
                }
            });
        ui.add_space(4.0);
    }
    let mode = state.conv().map_or(Mode::default(), |c| c.mode);
    let warn = ui.visuals().warn_fg_color;
    let typing = ui.memory(|m| m.has_focus(prompt_id()));
    let border = match () {
        _ if mode == Mode::Bypass => warn,
        _ if typing => pal.accent,
        _ => ui.visuals().widgets.noninteractive.bg_stroke.color,
    };
    egui::Frame::new()
        .fill(ui.visuals().extreme_bg_color)
        .stroke(egui::Stroke::new(1.0, border))
        .corner_radius(bubble_radius(ui))
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            // a live chip that only repeats a pinned one is left out
            let live: Vec<&(&'static str, String)> = v
                .chips
                .iter()
                .filter(|(k, label)| !state.chips_off.contains(k) && !state.pinned.iter().any(|p| p.label == *label))
                .collect();
            if !(state.pinned.is_empty() && live.is_empty() && state.images.is_empty()) {
                ui.horizontal_wrapped(|ui| {
                    let mut drop_img = None;
                    for (i, a) in state.images.clone().iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 1.0;
                            let mut gone = attachment(ui, state, a, 0.5, ("draft", i), true);
                            gone |= x_button(ui, ("img", i)).on_hover_text("Remove").clicked();
                            if gone {
                                drop_img = Some(i);
                            }
                        });
                    }
                    if let Some(i) = drop_img {
                        state.images.remove(i);
                    }
                    let mut drop_pin = None;
                    for (i, c) in state.pinned.iter().enumerate() {
                        if chip(ui, pal, &c.label, true) {
                            drop_pin = Some(i);
                        }
                    }
                    if let Some(i) = drop_pin {
                        state.pinned.remove(i);
                    }
                    for &(key, ref label) in live {
                        if chip(ui, pal, label, false) {
                            state.chips_off.push(key);
                        }
                    }
                });
                ui.add_space(2.0);
            }
            let id = prompt_id();
            let (slash, done) = slash_list(ui, &mut state.prompt, &mut state.slash_sel, v.skills, v.styles, id);
            if done {
                (state.cursor_end, state.focus_prompt) = (true, true);
            }
            let row_h = ui.text_style_height(&egui::TextStyle::Body);
            let r = egui::ScrollArea::vertical()
                .id_salt("assistant_prompt_scroll")
                .max_height(row_h * PROMPT_ROWS)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut state.prompt)
                            .id(id)
                            .frame(false)
                            .desired_rows(2)
                            .desired_width(f32::INFINITY)
                            .hint_text("Ask anything…  (/ for skills)")
                            .return_key(egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, egui::Key::Enter))
                            // while the `/` list is open Tab completes instead of moving the focus on
                            .lock_focus(slash),
                    )
                })
                .inner;
            if std::mem::take(&mut state.focus_prompt) {
                r.request_focus();
            }
            if std::mem::take(&mut state.cursor_end) {
                cursor_to_end(ui.ctx(), id, &state.prompt);
            }
            let can_send = !v.running && !state.prompt.trim().is_empty();
            if r.has_focus() && can_send && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift) {
                *out = Out::Send;
            }
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                attach_menu(ui, state, v, out);
                if !state.mic_status.is_empty() {
                    ui.weak(state.mic_status.as_str());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if v.running {
                        if glyph_text_button(ui, Glyph::Stop, "Stop").on_hover_text("Stop the assistant").clicked() {
                            *out = Out::Stop;
                        }
                    } else {
                        let accent = pal.accent;
                        let text = egui::RichText::new("↵").strong();
                        let send = match can_send {
                            true => egui::Button::new(text.color(on_accent(accent))).fill(accent),
                            false => egui::Button::new(text),
                        };
                        let send = send.corner_radius(bubble_radius(ui)).min_size(egui::vec2(28.0, 24.0));
                        let r = ui.add_enabled(can_send, send).on_hover_text("Send (Enter)").on_disabled_hover_text("Type a request first");
                        if r.clicked() {
                            *out = Out::Send;
                        }
                    }
                    let mic = glyph_text_button(ui, Glyph::Mic, if state.mic_wanted { "Stop" } else { "" });
                    let mic = mic.on_hover_text(if state.mic_wanted { "Stop dictating" } else { "Dictate the request" });
                    if mic.clicked() {
                        state.mic_wanted = !state.mic_wanted;
                    }
                });
            });
        });
    // under the box, on the right: model, effort, permission mode
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            mode_picker(ui, mode, out);
            if v.backend == "claude" {
                let e = EFFORTS.iter().find(|(id, _)| *id == v.effort).map_or("", |(_, l)| l);
                if !e.is_empty() {
                    ui.weak(egui::RichText::new(e).small()).on_hover_text("Effort: pick it in the model menu");
                }
            }
            model_picker(ui, state, v, out);
        });
    });
    ui.add_space(2.0);
}

/// ws:mcp-permissions: Allow once / Always allow / Deny (the chat's questions and the approval window).
pub fn answer_buttons(ui: &mut egui::Ui) -> Option<u8> {
    let mut pick = None;
    ui.horizontal(|ui| {
        if ui.button("Allow once").clicked() {
            pick = Some(0);
        }
        if ui.button("Always allow").on_hover_text("Saved in Settings ▸ Permissions").clicked() {
            pick = Some(1);
        }
        if ui.button("Deny").clicked() {
            pick = Some(2);
        }
    });
    pick
}

/// The chat's permission mode: Ask / Accept edits / Bypass. Bypass shows in the warning colour.
fn mode_picker(ui: &mut egui::Ui, mode: Mode, out: &mut Out) {
    let text = egui::RichText::new(format!("{} ▾", mode.label())).small();
    let text = if mode == Mode::Bypass { text.color(ui.visuals().warn_fg_color) } else { text };
    menu::button(ui, text, |ui| {
        for m in Mode::ALL {
            if menu::check(ui, m == mode, m.label(), "").on_hover_text(m.hint()).clicked() {
                *out = Out::Mode(m);
            }
        }
    })
    .response
    .on_hover_text(format!("{}. Blocked tools never run; set them in Settings ▸ Permissions.", mode.hint()));
}

/// The `+` menu: files, the frame at the playhead, a screenshot of a panel, an area of the window, the
/// timeline range, and the selection / In-Out as context.
fn attach_menu(ui: &mut egui::Ui, state: &mut AssistantState, v: &View, out: &mut Out) {
    let r = menu::button(ui, egui::RichText::new(" + ").strong(), |ui| {
        if menu::row(ui, Some(Glyph::Link), "Files…", "").on_hover_text("Or drop them here, or paste a picture").clicked() {
            *out = Out::Attach(Attach::File);
        }
        if menu::row(ui, Some(Glyph::FilmStrip), "Current frame", "").clicked() {
            *out = Out::Attach(Attach::Frame);
        }
        menu::sub(ui, Some(Glyph::GridIcon), "Screenshot a panel", |ui| {
            for &p in v.panes {
                if menu::row(ui, None, p.title(), "").clicked() {
                    *out = Out::Attach(Attach::Panel(p));
                }
            }
        });
        if menu::row(ui, Some(Glyph::Crop), "Select an area", "").on_hover_text("Drag a rectangle anywhere in the window").clicked() {
            state.area = true;
        }
        if menu::row(ui, Some(Glyph::Clock), "Timeline range", "").on_hover_text("In/Out (else the selected clips): a picture plus its clips and tracks").clicked() {
            *out = Out::Attach(Attach::Timeline);
        }
        ui.separator();
        if menu::row(ui, Some(Glyph::Cursor), "Selection", "").clicked() {
            *out = Out::Attach(Attach::Pin(Pin::Smart));
        }
        if menu::row(ui, None, "In/Out range", "").clicked() {
            *out = Out::Attach(Attach::Pin(Pin::Range));
        }
    });
    r.response.on_hover_text("Attach files or capture the editor: the frame, a panel, an area, the timeline range");
}

/// ws:skills: the `/` list above a prompt box (`text`, widget `id`) while a skill name is typed:
/// filtered as the user types; Up / Down pick (`sel`), Enter / Tab / a click complete to `/name `.
/// Returns (open, completed this frame); the caller focuses the box and moves its cursor on a completion.
pub fn slash_list(
    ui: &mut egui::Ui,
    text: &mut String,
    sel: &mut usize,
    all: &[Skill],
    styles: &[crate::engine::styles::Style],
    id: egui::Id,
) -> (bool, bool) {
    // (what a pick puts in the box, the row's text, its tooltip)
    let hits: Vec<(String, String, String)> = if let Some(q) = style_query(text, styles) {
        // feat/styles-context: `/style ` then a style's name
        crate::engine::styles::filter(styles, q, 8)
            .into_iter()
            .map(|s| (format!("/style {}", s.name), format!("/style {}  ·  {}", s.name, s.title), s.description.clone()))
            .collect()
    } else if let Some(q) = slash_query(text) {
        let mut v = Vec::new();
        if !styles.is_empty() && !q.is_empty() && "style".starts_with(q) {
            v.push(("/style ".to_string(), "/style [name]  ·  Set this project's editing style".to_string(), STYLE_TIP.to_string()));
        }
        v.extend(skills::filter(all, q, 8 - v.len()).into_iter().map(|s| {
            let mut text = format!("/{}", s.name);
            if !s.args.is_empty() {
                text += &format!(" {}", s.args);
            }
            if !s.description.is_empty() {
                text += &format!("  ·  {}", s.description);
            }
            (format!("/{} ", s.name), text, s.description.clone())
        }));
        v
    } else {
        *sel = 0;
        return (false, false);
    };
    if hits.is_empty() {
        return (false, false);
    }
    *sel = (*sel).min(hits.len() - 1);
    let mut pick = None;
    if ui.memory(|m| m.has_focus(id)) {
        let none = egui::Modifiers::NONE;
        let (up, down, enter, tab) = ui.input_mut(|i| {
            (
                i.consume_key(none, egui::Key::ArrowUp),
                i.consume_key(none, egui::Key::ArrowDown),
                i.consume_key(none, egui::Key::Enter),
                i.consume_key(none, egui::Key::Tab),
            )
        });
        if up {
            *sel = sel.checked_sub(1).unwrap_or(hits.len() - 1);
        }
        if down {
            *sel = (*sel + 1) % hits.len();
        }
        if enter || tab {
            pick = Some(*sel);
        }
    }
    for (i, (_, label, tip)) in hits.iter().enumerate() {
        let r = ui.add(egui::Button::selectable(i == *sel, label.as_str()).truncate());
        if r.on_hover_text(tip.as_str()).clicked() {
            pick = Some(i);
        }
    }
    ui.separator();
    if let Some(i) = pick {
        *text = hits[i].0.clone();
        *sel = 0;
    }
    (true, pick.is_some())
}

/// feat/styles-context: what `/style` does (its row's tooltip).
const STYLE_TIP: &str = "Writes a style from your Editing Styles into this project's Style guide and Rules; /style off takes it out";

/// The name being typed after `/style ` (None once it is a whole style's name, so Enter sends).
fn style_query<'a>(prompt: &'a str, styles: &[crate::engine::styles::Style]) -> Option<&'a str> {
    let q = prompt.strip_prefix("/style ")?;
    (!q.contains(char::is_whitespace) && !styles.iter().any(|s| s.name == q)).then_some(q)
}

/// Put a TextEdit's cursor at the end of `text`.
fn cursor_to_end(ctx: &egui::Context, id: egui::Id, text: &str) {
    let mut s = egui::text_edit::TextEditState::load(ctx, id).unwrap_or_default();
    let end = egui::text::CCursor::new(text.chars().count());
    s.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
    s.store(ctx, id);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_frames(ctx: &egui::Context, state: &mut AssistantState, v: &View, n: usize) {
        for _ in 0..n {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| assert_eq!(draw(ui, state, v), Out::None));
            });
        }
    }

    /// An idle Assistant pane - on both tabs, with a finished request, a task, chips and an attachment
    /// showing - requests no repaint.
    #[test]
    fn assert_no_idle_repaint_assistant() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let mut state = AssistantState { keep_on_open: true, ..Default::default() };
        state.push(Line::User("Tighten the intro".into()));
        state.push(Line::Attached { chips: vec!["2 clips".into()], images: vec![] });
        state.push(Line::Tool("editor.context".into()));
        state.push(Line::Tool("timeline.cut {\"t\":1}".into()));
        state.push(Line::Text("Trimmed **3** pauses.".into()));
        state.push(Line::Done { undo: Some((4, "Assistant: Tighten the intro".into())) });
        let id = task_writing(state.tasks_mut(), "Cut the ums");
        task_hand_over(state.tasks_mut(), Some(id), "Cut ums", "editor.log('x')", "cuts them");
        state.pinned.push(Chip { label: "In/Out 1.0s–2.0s".into(), data: serde_json::json!({"kind": "range"}) });
        let chips = [("selection", "2 clips".to_string()), ("playhead", "Playhead 0:12".to_string())];
        let all = test_skills();
        let v = View {
            running: false,
            chips: &chips,
            skills: &all,
            recent: &[],
            undo_top: (4, Some("Assistant: Tighten the intro")),
            backend: "claude",
            model: "sonnet",
            effort: "high",
            queued: 0,
            panes: &[],
            full_access: false,
            asks: &[],
            styles: &[],
            applied_style: None,
            context_open: false,
        };
        state.images.push(Attachment::file(std::path::Path::new("C:\\notes.txt")));
        for tab in [Tab::Tasks, Tab::Chat, Tab::Skills] {
            state.tab = tab;
            run_frames(&ctx, &mut state, &v, 30);
            assert!(!ctx.has_requested_repaint(), "an idle Assistant pane ({tab:?}) must request no repaint");
        }
        // the sidebar hidden, and shown in a wide pane
        state.sidebar = Some(false);
        run_frames(&ctx, &mut state, &v, 30);
        assert!(!ctx.has_requested_repaint(), "an idle Assistant pane without its sidebar must request no repaint");
        state.sidebar = Some(true);
        // an empty conversation (the centred hint and quick prompts) and a free-text model field too
        state.new_chat();
        let v = View { backend: "codex", model: "", ..v };
        run_frames(&ctx, &mut state, &v, 30);
        assert!(!ctx.has_requested_repaint(), "an empty chat must request no repaint");
    }

    /// Enter in the focused prompt sends; Shift+Enter does not.
    #[test]
    fn enter_sends_from_the_prompt() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let mut state = AssistantState::default();
        state.tab = Tab::Chat;
        state.prompt = "tighten this".into();
        state.focus_prompt = true;
        let all = test_skills();
        let v = view(&all);
        // (the pressed modifiers are read from the frame's input state, not from the key event)
        let mut frame = |state: &mut AssistantState, events: Vec<egui::Event>| {
            let mut out = Out::None;
            let modifiers = match events.first() {
                Some(egui::Event::Key { modifiers, .. }) => *modifiers,
                _ => egui::Modifiers::NONE,
            };
            let _ = ctx.run(egui::RawInput { events, modifiers, ..Default::default() }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| out = draw(ui, state, &v));
            });
            out
        };
        let key = |modifiers| egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers };
        assert_eq!(frame(&mut state, vec![]), Out::None); // the prompt takes focus
        assert_eq!(frame(&mut state, vec![]), Out::None);
        assert_eq!(frame(&mut state, vec![key(egui::Modifiers::SHIFT)]), Out::None, "Shift+Enter is a new line");
        assert_eq!(frame(&mut state, vec![key(egui::Modifiers::NONE)]), Out::Send, "Enter sends");
    }

    // ---- ws:skills ----

    fn skill(name: &str, args: &str, source: Source) -> Skill {
        Skill {
            name: name.into(),
            description: format!("{name} description"),
            args: args.into(),
            body: "Do it. $ARGS".into(),
            source,
            path: None,
            thumb: String::new(),
        }
    }

    fn test_skills() -> Vec<Skill> {
        vec![
            skill("captions", "[style]", Source::BuiltIn),
            skill("cut-silences", "", Source::BuiltIn),
            skill("my-intro", "", Source::User),
            skill("shorts", "", Source::Library),
        ]
    }

    fn view(all: &[Skill]) -> View<'_> {
        View {
            running: false,
            chips: &[],
            skills: all,
            recent: &[],
            undo_top: (0, None),
            backend: "claude",
            model: "",
            effort: "",
            queued: 0,
            panes: &[],
            full_access: false,
            asks: &[],
            styles: &[],
            applied_style: None,
            context_open: false,
        }
    }

    /// Typing `/c` lists the matching skills above the prompt; Down + Enter completes to `/name `
    /// without sending, Tab completes too, and Enter then sends. (Headless: the window has focus here,
    /// unlike an SE_BACKGROUND test window - see notes.md.)
    #[test]
    fn slash_list_filters_and_completes() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let all = test_skills();
        let v = view(&all);
        let mut state = AssistantState { tab: Tab::Chat, prompt: "/c".into(), focus_prompt: true, ..Default::default() };
        let frame = |state: &mut AssistantState, keys: &[egui::Key]| {
            let mut out = Out::None;
            let events = keys
                .iter()
                .map(|&key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE })
                .collect();
            let _ = ctx.run(egui::RawInput { events, ..Default::default() }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| out = draw(ui, state, &v));
            });
            out
        };
        assert_eq!(frame(&mut state, &[]), Out::None); // the prompt takes focus
        assert_eq!(frame(&mut state, &[]), Out::None);
        assert_eq!(frame(&mut state, &[egui::Key::ArrowDown]), Out::None);
        assert_eq!(state.slash_sel, 1, "captions, cut-silences: Down picks the second");
        assert_eq!(frame(&mut state, &[egui::Key::Enter]), Out::None, "Enter completes while the list is open");
        assert_eq!(state.prompt, "/cut-silences ");
        assert_eq!(frame(&mut state, &[]), Out::None);
        assert_eq!(frame(&mut state, &[egui::Key::Enter]), Out::Send, "with the name typed, Enter sends");
        state.prompt = "/sh".into();
        assert_eq!(frame(&mut state, &[]), Out::None);
        assert_eq!(frame(&mut state, &[egui::Key::Tab]), Out::None);
        assert_eq!(state.prompt, "/shorts ", "Tab completes");
        assert_eq!(frame(&mut state, &[]), Out::None);
        assert_eq!(frame(&mut state, &[egui::Key::Enter]), Out::Send, "the prompt kept the focus through Tab");
        state.prompt = "/zz".into();
        assert_eq!(frame(&mut state, &[]), Out::None);
        assert_eq!(frame(&mut state, &[egui::Key::Enter]), Out::Send, "no match: no list, Enter sends as text");
    }

    /// Chips: the skills run last first; Run of a skill without arguments sends, one with arguments is
    /// typed into the prompt.
    #[test]
    fn skill_chips_and_run() {
        let all = test_skills();
        let names = |v: Vec<&Skill>| v.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(",");
        assert_eq!(names(chip_skills(&all, &["shorts".into(), "gone".into()], 3)), "shorts,captions,cut-silences");
        let mut state = AssistantState::default();
        let mut out = Out::None;
        run_skill(&mut state, &all[1], &mut out);
        assert_eq!((state.prompt.as_str(), &out, state.tab), ("/cut-silences", &Out::Send, Tab::Chat));
        let mut out = Out::None;
        run_skill(&mut state, &all[0], &mut out);
        assert_eq!((state.prompt.as_str(), &out, state.focus_prompt), ("/captions ", &Out::None, true));
        let copy = SkillDraft::copy(&all[2], &all);
        assert_eq!((copy.name.as_str(), copy.editing.is_none(), copy.plain), ("my-intro-copy", true, true));
        assert_eq!(SkillDraft::edit(&all[2]).editing.as_deref(), Some("my-intro"));
    }

    #[test]
    fn assert_no_idle_repaint_assistant_skills_tab() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let all = test_skills();
        let v = view(&all);
        let mut state = AssistantState { tab: Tab::Skills, keep_on_open: true, ..Default::default() };
        run_frames(&ctx, &mut state, &v, 30);
        assert!(!ctx.has_requested_repaint(), "an idle Skills tab must request no repaint");
        // the `/` list open in the chat composer
        state.tab = Tab::Chat;
        state.prompt = "/".into();
        run_frames(&ctx, &mut state, &v, 30);
        assert!(!ctx.has_requested_repaint(), "an open / list must request no repaint");
    }

    #[test]
    fn assert_no_idle_repaint_assistant_new_skill_window() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let all = test_skills();
        for draft in [SkillDraft::default(), SkillDraft::edit(&all[2])] {
            let mut state = AssistantState { skill_draft: Some(draft), ..Default::default() };
            for _ in 0..30 {
                let _ = ctx.run(egui::RawInput::default(), |ctx| assert_eq!(skill_window(ctx, &mut state, false), Out::None));
            }
            assert!(state.skill_draft.is_some(), "it stays open");
            assert!(!ctx.has_requested_repaint(), "an idle skill window must request no repaint");
        }
    }

    #[test]
    fn assert_no_idle_repaint_assistant_skill_read_first() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let s = skill("shorts", "", Source::Library);
        for _ in 0..30 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| assert_eq!(read_first_window(ctx, &s), None));
        }
        assert!(!ctx.has_requested_repaint(), "an idle read-first window must request no repaint");
    }

    #[test]
    fn undo_only_while_top_of_stack() {
        let u = Some((4, "Assistant: x".to_string()));
        assert_eq!(undo_state(&u, (4, Some("Assistant: x"))), Ok(()));
        assert!(undo_state(&u, (5, Some("Delete"))).is_err(), "an edit came after it");
        assert!(undo_state(&u, (3, Some("Assistant: x"))).is_err(), "already undone");
        assert!(undo_state(&None, (4, None)).is_err(), "nothing to undo");
    }

    /// New chat reuses an empty one; delete keeps something to show; load shows the most recent one.
    #[test]
    fn conversations_switch_and_delete() {
        let mut s = AssistantState::default();
        s.new_chat();
        s.new_chat();
        assert_eq!(s.convs.len(), 1, "an empty chat is reused");
        s.push(Line::User("a".into()));
        s.new_chat();
        assert_eq!((s.convs.len(), s.cur), (2, 1));
        let first = s.convs[0].id;
        s.open(first);
        assert_eq!(s.log(), &[Line::User("a".into())]);
        s.delete(first);
        assert_eq!(s.convs.len(), 1);
        s.delete(s.convs[0].id);
        assert_eq!(s.convs.len(), 1, "deleting the last leaves a fresh chat");
        let mut old = Conversation::new(1);
        old.updated = 5.0;
        let mut recent = Conversation::new(2);
        recent.updated = 9.0;
        s.load(vec![recent.clone(), old]);
        assert_eq!(s.conv().unwrap().id, 2);
    }


    /// Frames with a fixed screen size (the sidebar shows from `SIDEBAR_FROM` wide).
    fn frame_at(ctx: &egui::Context, state: &mut AssistantState, v: &View, w: f32, events: Vec<egui::Event>) -> Out {
        let mut out = Out::None;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, 600.0))),
            events,
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| out = draw(ui, state, v));
        });
        out
    }

    /// Opening the pane makes a new chat ready (an empty one is reused) with the cursor in the composer;
    /// staying open keeps the chat; a caller that prepared the chat (`keep_on_open`) keeps its own.
    #[test]
    fn a_new_chat_is_ready_when_the_pane_opens() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let all = test_skills();
        let v = view(&all);
        let mut old = Conversation::new(1);
        old.lines.push(Line::User("earlier".into()));
        let mut state = AssistantState::default();
        state.load(vec![old]);
        frame_at(&ctx, &mut state, &v, 400.0, vec![]);
        assert_eq!(state.convs.len(), 2, "a new chat next to the old one");
        assert!(state.conv().unwrap().is_empty(), "the new, empty chat is shown");
        frame_at(&ctx, &mut state, &v, 400.0, vec![]);
        assert!(ctx.memory(|m| m.has_focus(prompt_id())), "the cursor is in the composer");
        // still open: typing and the next frames keep this chat
        state.push(Line::User("now".into()));
        frame_at(&ctx, &mut state, &v, 400.0, vec![]);
        assert_eq!(state.log().len(), 1);
        // closed for a while (frames without a draw), opened again: a fresh chat
        for _ in 0..3 {
            let _ = ctx.run(egui::RawInput::default(), |_| {});
        }
        frame_at(&ctx, &mut state, &v, 400.0, vec![]);
        assert_eq!((state.convs.len(), state.log().len()), (3, 0));
        // reopened while the shown chat is still empty: it is reused, not doubled
        for _ in 0..3 {
            let _ = ctx.run(egui::RawInput::default(), |_| {});
        }
        frame_at(&ctx, &mut state, &v, 400.0, vec![]);
        assert_eq!(state.convs.len(), 3);
        // a caller that already opened a chat for its request keeps it
        state.push(Line::User("from the palette".into()));
        state.keep_on_open = true;
        for _ in 0..3 {
            let _ = ctx.run(egui::RawInput::default(), |_| {});
        }
        frame_at(&ctx, &mut state, &v, 400.0, vec![]);
        assert_eq!(state.log(), &[Line::User("from the palette".into())]);
    }

    fn chat(id: u64, title: &str, updated: f64, text: &str) -> Conversation {
        let mut c = Conversation::new(id);
        (c.title, c.updated) = (title.into(), updated);
        c.lines.push(Line::User(text.into()));
        c
    }

    /// The sidebar lists the chats (pinned first, then newest), searches their titles and messages, and
    /// renames one in place: Enter keeps the new name.
    #[test]
    fn sidebar_lists_searches_and_renames() {
        let mut convs = vec![chat(1, "Old intro", 10.0, "cut the intro"), chat(2, "Music", 30.0, "louder music"), chat(3, "Colour", 20.0, "warmer")];
        convs.push(Conversation::new(4)); // empty: never listed
        let ids = |order: Vec<usize>, convs: &[Conversation]| order.into_iter().map(|i| convs[i].id).collect::<Vec<_>>();
        assert_eq!(ids(sidebar_order(&convs, ""), &convs), vec![2, 3, 1], "newest first");
        convs[0].pinned = true;
        assert_eq!(ids(sidebar_order(&convs, ""), &convs), vec![1, 2, 3], "pinned on top");
        assert_eq!(ids(sidebar_order(&convs, "LOUDER"), &convs), vec![2], "messages are searched");
        assert_eq!(ids(sidebar_order(&convs, "colour"), &convs), vec![3], "titles are searched");
        assert!(sidebar_order(&convs, "zebra").is_empty());
        assert_eq!(short_date(1000.0, 1030.0), "now");
        assert_eq!(short_date(0.0, 7200.0), "2h");
        assert_eq!(short_date(1_763_337_600.0, 1_763_337_600.0 + 30.0 * 86_400.0), "Nov 17", "2025-11-17 UTC");

        // drawn: a wide pane shows the sidebar; a search narrows it; rename through the in-place box
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let all = test_skills();
        let v = view(&all);
        let mut state = AssistantState { keep_on_open: true, ..Default::default() };
        state.load(convs);
        frame_at(&ctx, &mut state, &v, 900.0, vec![]);
        state.search = "music".into();
        frame_at(&ctx, &mut state, &v, 900.0, vec![]);
        state.renaming = Some((2, String::new()));
        frame_at(&ctx, &mut state, &v, 900.0, vec![]); // the box takes the focus
        frame_at(&ctx, &mut state, &v, 900.0, vec![egui::Event::Text("Soundtrack".into())]);
        let enter = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        frame_at(&ctx, &mut state, &v, 900.0, vec![enter]);
        assert_eq!(state.renaming, None);
        assert_eq!(state.convs.iter().find(|c| c.id == 2).unwrap().title, "Soundtrack");
        assert!(state.dirty);
        state.toggle_pin(2);
        assert!(state.convs.iter().find(|c| c.id == 2).unwrap().pinned);
    }

    /// The text a clipboard command carries, from one frame's output.
    fn copied(out: &egui::FullOutput) -> Vec<String> {
        out.platform_output
            .commands
            .iter()
            .filter_map(|c| match c {
                egui::OutputCommand::CopyText(t) => Some(t.clone()),
                egui::OutputCommand::CopyImage(i) => Some(format!("image {}x{}", i.size[0], i.size[1])),
                _ => None,
            })
            .collect()
    }

    /// A reply full of markup and tool calls (an HTML layer's source among them).
    fn marked_up_chat() -> Vec<Line> {
        vec![
            Line::User("Add a **title**".into()),
            Line::Attached { chips: vec!["2 clips".into()], images: vec![Attachment::file(std::path::Path::new("C:\\brief.txt"))] },
            Line::Tool("html.add {\"html\":\"<div class='title'><h1>Hi</h1></div>\"}".into()),
            Line::Text("Added the title:\n\n- **font**: `Montserrat`\n- size _72_\n\n```\n<h1>Hi</h1>\n```".into()),
            Line::Done { undo: None },
        ]
    }

    /// Copy text gives plain words: no Markdown, no tool calls, no HTML from the agent's tool arguments;
    /// Copy as Markdown keeps the Markdown; both come from the message alone.
    #[test]
    fn copy_gives_plain_text_without_markup() {
        let lines = marked_up_chat();
        let msgs = messages(&lines);
        assert_eq!(msgs, vec![Msg { user: true, lines: 0..2 }, Msg { user: false, lines: 2..5 }]);
        assert_eq!(msg_plain(&lines, &msgs[0]), "Add a title");
        let reply = msg_plain(&lines, &msgs[1]);
        assert_eq!(reply, "Added the title:\n\n- font: Montserrat\n- size 72\n\n<h1>Hi</h1>");
        assert!(!reply.contains("html.add") && !reply.contains("class=") && !reply.contains("**"), "{reply}");
        assert_eq!(msg_markdown(&lines, &msgs[1]), "Added the title:\n\n- **font**: `Montserrat`\n- size _72_\n\n```\n<h1>Hi</h1>\n```");
        // the reply's Copy button and its menu row put exactly that on the clipboard
        let ctx = egui::Context::default();
        let mut state = AssistantState { keep_on_open: true, ..Default::default() };
        state.conv_mut().lines = lines;
        let out = ctx.run(egui::RawInput::default(), |ctx| {
            let mut o = Out::None;
            msg_act(ctx, &mut state, &msgs[1], MsgAct::CopyText, &mut o);
        });
        assert_eq!(copied(&out), vec![reply]);
    }

    /// Every row of a message's right-click menu does what it says.
    #[test]
    fn message_menu_actions() {
        let lines = marked_up_chat();
        let msgs = messages(&lines);
        let names = |rows: Vec<(MsgAct, &str, bool)>| rows.into_iter().map(|r| (r.0, r.2)).collect::<Vec<_>>();
        use MsgAct::*;
        assert_eq!(
            names(msg_menu_rows(&lines, &msgs[0], false)),
            vec![(CopyText, true), (CopyMarkdown, true), (CopyAttachments, true), (Edit, true), (Retry, true), (Delete, true)]
        );
        assert_eq!(names(msg_menu_rows(&lines, &msgs[1], false)), vec![(CopyText, true), (CopyMarkdown, true), (Retry, true), (Delete, true)]);
        assert!(msg_menu_rows(&lines, &msgs[0], true).iter().filter(|r| matches!(r.0, Edit | Retry | Delete)).all(|r| !r.2), "not while running");

        let ctx = egui::Context::default();
        let fresh = || {
            let mut s = AssistantState { keep_on_open: true, ..Default::default() };
            s.conv_mut().lines = marked_up_chat();
            s
        };
        let act = |state: &mut AssistantState, m: &Msg, a: MsgAct| {
            let mut o = Out::None;
            let full = ctx.run(egui::RawInput::default(), |ctx| msg_act(ctx, state, m, a, &mut o));
            (o, copied(&full))
        };
        let mut s = fresh();
        assert_eq!(act(&mut s, &msgs[0], CopyMarkdown).1, vec!["Add a **title**".to_string()]);
        assert_eq!(act(&mut s, &msgs[0], CopyAttachments).1, vec!["C:\\brief.txt".to_string()], "a file copies as its path");
        assert_eq!(act(&mut s, &msgs[0], Retry).0, Out::Retry(0));
        assert_eq!(act(&mut s, &msgs[1], Retry).0, Out::Retry(0), "a reply retries the message it answered");
        act(&mut s, &msgs[1], Delete);
        assert_eq!(s.log().len(), 2, "the reply is gone, the question stays");
        let mut s = fresh();
        act(&mut s, &msgs[0], Edit);
        assert_eq!((s.prompt.as_str(), s.images.len(), s.log().len()), ("Add a **title**", 1, 0), "back in the composer, the chat cut before it");

        // the menu opens on a right-click anywhere in the message's area (over many labels), at the pointer
        let rect = egui::Rect::from_min_size(egui::pos2(10.0, 10.0), egui::vec2(200.0, 80.0));
        let at = egui::pos2(60.0, 40.0);
        let mut rows = 0;
        let mut frame = |events: Vec<egui::Event>| {
            let _ = ctx.run(egui::RawInput { events, ..Default::default() }, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.label("selectable text inside the message");
                    if menu::context_area(ui, egui::Id::new("t_msg"), rect, |ui| menu::row(ui, None, "Copy text", "")).is_some() {
                        rows += 1;
                    }
                });
            });
        };
        frame(vec![egui::Event::PointerMoved(at)]);
        let b = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: egui::Modifiers::NONE };
        frame(vec![b(true)]);
        frame(vec![b(false)]);
        frame(vec![]);
        assert!(rows > 0, "the message menu opened");
    }

    /// Attachments: a picture (PNG + thumbnail) and a file by path; Copy puts the picture on the clipboard
    /// as an image (a file as its path), several go as their paths; Save as writes the same bytes.
    #[test]
    fn attachments_add_copy_and_save() {
        let dir = std::env::temp_dir().join(format!("se-assistant-att-ui-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let rgba: Vec<u8> = (0..64 * 32).flat_map(|i| [(i % 200) as u8, 40, 90, 255]).collect();
        let pic = write_attachment(&dir, "Frame at 1.00s", &rgba, 64, 32).unwrap();
        assert!(pic.is_image() && pic.width == 64);
        let doc = dir.join("notes.txt");
        std::fs::write(&doc, "hello").unwrap();
        let file = Attachment::file(&doc);
        assert!(!file.is_image() && file.name == "notes.txt" && file.path() == doc);

        let ctx = egui::Context::default();
        let run = |f: &dyn Fn(&egui::Context)| copied(&ctx.run(egui::RawInput::default(), |ctx| f(ctx)));
        assert_eq!(run(&|ctx| copy_attachment(ctx, &pic)), vec!["image 64x32".to_string()]);
        assert_eq!(run(&|ctx| copy_attachment(ctx, &file)), vec![doc.display().to_string()]);
        let both = [pic.clone(), file.clone()];
        assert_eq!(run(&|ctx| copy_attachments(ctx, &both)), vec![format!("{}\n{}", pic.png.display(), doc.display())]);

        assert_eq!(save_name(&pic), "Frame at 1.00s.png");
        assert_eq!(save_name(&file), "notes.txt");
        let to = dir.join("saved.png");
        save_attachment(&pic, &to).unwrap();
        assert_eq!(std::fs::read(&to).unwrap(), std::fs::read(&pic.png).unwrap());
        save_attachment(&file, &dir.join("copy.txt")).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("copy.txt")).unwrap(), "hello");
        // old chats (no `file` field) still load
        let old: Attachment = serde_json::from_str(r#"{"name":"f","png":"a.png","thumb":"a.thumb","width":2,"height":1}"#).unwrap();
        assert!(old.is_image() && old.file.is_none());
        let old: Conversation = serde_json::from_str(r#"{"id":1,"title":"t","created":0,"updated":0,"session":null,"lines":[],"tasks":[]}"#).unwrap();
        assert!(!old.pinned);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn assert_no_idle_repaint_assistant_preview_and_area() {
        let dir = std::env::temp_dir().join(format!("se-assistant-preview-{}", std::process::id()));
        let rgba = vec![200u8; 32 * 16 * 4];
        let pic = write_attachment(&dir, "shot", &rgba, 32, 16).unwrap();
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let mut state = AssistantState { preview: Some(pic), ..Default::default() };
        for _ in 0..30 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| preview_window(ctx, &mut state));
        }
        assert!(state.preview.is_some(), "it stays open");
        assert!(!ctx.has_requested_repaint(), "an idle attachment preview must request no repaint");
        state.area = true;
        for _ in 0..30 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| assert_eq!(area_overlay(ctx, &mut state), None));
        }
        assert!(state.area, "waits for a drag");
        assert!(!ctx.has_requested_repaint(), "an idle area picker must request no repaint");
        // a drag picks the rectangle and closes it
        let p = |x, y| egui::Event::PointerMoved(egui::pos2(x, y));
        let b = |x, y, pressed| egui::Event::PointerButton { pos: egui::pos2(x, y), button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        let mut got = None;
        for events in [vec![p(10.0, 20.0)], vec![b(10.0, 20.0, true)], vec![p(60.0, 50.0)], vec![p(110.0, 70.0)], vec![b(110.0, 70.0, false)]] {
            let _ = ctx.run(egui::RawInput { events, ..Default::default() }, |ctx| got = got.or(area_overlay(ctx, &mut state)));
        }
        let r = got.expect("a rectangle");
        assert!((r.width() - 100.0).abs() < 8.0 && (r.height() - 50.0).abs() < 8.0, "{r:?}");
        assert!(!state.area);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The product's own wording never names a vendor: every string literal in the Assistant's files,
    /// outside the agent / model option tables (`AGENTS`, `CLAUDE_MODELS`) and tests, is vendor-free.
    #[test]
    fn no_vendor_name_in_assistant_strings() {
        let files = [
            ("assistant_ui.rs", include_str!("assistant_ui.rs")),
            ("assistant_data.rs", include_str!("assistant_data.rs")),
            ("app/assistant_ctl.rs", include_str!("app/assistant_ctl.rs")),
            ("app/tools_context.rs", include_str!("app/tools_context.rs")),
            ("settings_ui/assistant.rs", include_str!("settings_ui/assistant.rs")),
            ("engine/agent.rs", include_str!("../engine/agent.rs")),
            ("mcp/mod.rs INSTRUCTIONS", include_str!("../mcp/mod.rs")),
        ];
        let vendor = ["Clau", "de"].concat(); // spelled apart so this test's own source passes
        for (name, src) in files {
            let code = src.split("#[cfg(test)]").next().unwrap_or(src);
            for (n, line) in code.lines().enumerate() {
                let t = line.trim_start();
                if t.starts_with("//") || t.contains("pub const AGENTS") || t.starts_with("&[(\"claude\"") {
                    continue;
                }
                // only what is inside string literals counts
                let strings: String = t.split('"').skip(1).step_by(2).collect();
                assert!(!strings.contains(&vendor), "{name}:{}: product wording names a vendor: {t}", n + 1);
            }
        }
    }
}
