//! ---- ws:assistant-pane ----
//! The Assistant pane widget (pure: no `App`). One header row (conversation menu, New chat, the agent
//! and model pickers), then two tabs:
//! - **Tasks**: describe a change, the agent writes it as a Luau script; each script is a card the user
//!   runs, edits, saves to the Scripts folder or removes. "Handle my comments" lets the agent answer
//!   timeline comments in the background.
//! - **Chat**: user bubbles on the right, markdown replies on the left, each reply's tool calls folded
//!   into one line, Undo / Copy under a finished reply; the composer (chips, prompt, `+`, mic, Send /
//!   Stop) is pinned to the bottom.
//!
//! It edits what is pure state (conversations, tasks, chips) itself and returns what needs the app
//! (`Out`); `ui/app/assistant_ctl.rs` acts on it. The data model is `ui::assistant_data`.

pub use super::assistant_data::*;
use super::menu;
use super::tools::{draw_glyph, glyph_text_button, Glyph};
use crate::engine::skills::{self, Skill, Source};
use crate::theme::Palette;
use eframe::egui;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Tasks,
    Chat,
    /// Every saved conversation of this project.
    History,
    /// ws:skills: every skill, with Run / Edit / Duplicate / Delete and New skill.
    Skills,
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
    pub tab: Tab,
    /// Context the user added on purpose ("Add to Assistant"); sent with the next message.
    pub pinned: Vec<Chip>,
    /// Images waiting in the composer.
    pub images: Vec<Attachment>,
    /// The Tasks tab's "Describe a task…" box.
    pub task_prompt: String,
    /// The task whose code is open for editing, and the edit buffer.
    pub editing: Option<(u64, String)>,
    /// "Handle my comments": new timeline comments go to the agent in the background.
    pub comments: bool,
    /// Focus the chat prompt on the next draw (a pinned chip was just added).
    pub focus_prompt: bool,
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
    /// Attachment thumbnails by file (None = unreadable), loaded once.
    thumbs: HashMap<std::path::PathBuf, Option<egui::TextureHandle>>,
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
    /// The shown conversation's lines (what `assistant.status` reports).
    pub fn log(&self) -> &[Line] {
        self.conv().map_or(&[], |c| c.lines.as_slice())
    }
    pub fn push(&mut self, line: Line) {
        let c = self.conv_mut();
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
        self.dirty = true;
    }
    pub fn open(&mut self, id: u64) {
        if let Some(i) = self.convs.iter().position(|c| c.id == id) {
            self.cur = i;
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
    /// Show `convs` (a project's store): the most recently used one, or a fresh chat.
    pub fn load(&mut self, convs: Vec<Conversation>) {
        self.convs = convs;
        self.cur = self.convs.iter().enumerate().max_by(|a, b| a.1.updated.total_cmp(&b.1.updated)).map_or(0, |(i, _)| i);
        self.conv_mut();
        self.editing = None;
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
}

#[derive(Debug, PartialEq, Eq)]
pub enum Out {
    None,
    /// Send `state.prompt` (with the chips and images).
    Send,
    Stop,
    /// Undo a finished request (it is the top of the undo stack).
    Undo,
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attach {
    /// The timeline at the playhead, as an image.
    Frame,
    /// A PNG / JPG from disk.
    File,
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

pub fn draw(ui: &mut egui::Ui, state: &mut AssistantState, v: &View) -> Out {
    let pal = crate::theme::palette(ui.ctx());
    let mut out = Out::None;
    state.conv_mut();
    // a little air at the pane's edges: right-aligned controls (Send, the comments switch) sat on the
    // pane border and lost their last pixels to its clip rect
    egui::Frame::NONE.inner_margin(egui::Margin { left: 4, right: 8, top: 2, bottom: 2 }).show(ui, |ui| {
        header(ui, state, v, &mut out);
        // wraps: four tabs must not widen a narrow pane
        ui.horizontal_wrapped(|ui| {
            let n = state.conv().map_or(0, |c| c.tasks.len());
            let tasks = if n > 0 { format!("Tasks ({n})") } else { "Tasks".into() };
            let tip = "Scripts the assistant writes for you to run";
            if ui.selectable_label(state.tab == Tab::Tasks, tasks).on_hover_text(tip).clicked() {
                state.tab = Tab::Tasks;
            }
            if ui.selectable_label(state.tab == Tab::Chat, "Chat").clicked() {
                state.tab = Tab::Chat;
            }
            let saved = state.convs.iter().filter(|c| !c.is_empty()).count();
            let history = if saved > 0 { format!("History ({saved})") } else { "History".into() };
            let tip = "Past chats in this project";
            if ui.selectable_label(state.tab == Tab::History, history).on_hover_text(tip).clicked() {
                state.tab = Tab::History;
            }
            let tip = "Saved instructions you run in chat as /name";
            if ui.selectable_label(state.tab == Tab::Skills, "Skills").on_hover_text(tip).clicked() {
                state.tab = Tab::Skills;
            }
        });
        ui.separator();
        match state.tab {
            Tab::Tasks => tasks_tab(ui, state, v, &pal, &mut out),
            Tab::Chat => chat_tab(ui, state, v, &pal, &mut out),
            Tab::History => history_tab(ui, state, v),
            Tab::Skills => skills_tab(ui, state, v, &mut out),
        }
    });
    out
}

// ---------------------------------------------------------------------------------------------------
// header

fn header(ui: &mut egui::Ui, state: &mut AssistantState, v: &View, out: &mut Out) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let title = short(state.conv().map_or("New chat", Conversation::title), 28);
        ui.add_enabled_ui(!v.running, |ui| {
            menu::button(ui, format!("{title} ▾"), |ui| {
                if menu::row(ui, Some(Glyph::Letter('+')), "New chat", "").clicked() {
                    state.new_chat();
                }
                ui.separator();
                let mut order: Vec<(f64, u64, String)> =
                    state.convs.iter().filter(|c| !c.is_empty()).map(|c| (c.updated, c.id, short(c.title(), 40))).collect();
                order.sort_by(|a, b| b.0.total_cmp(&a.0));
                if order.is_empty() {
                    ui.weak("No conversations yet");
                }
                let mut delete = None;
                for (_, id, title) in order {
                    ui.horizontal(|ui| {
                        let shown = state.conv().is_some_and(|c| c.id == id);
                        if menu::check(ui, shown, &title, "").clicked() {
                            state.open(id);
                        }
                        if x_button(ui, ("del_conv", id)).on_hover_text("Delete this conversation").clicked() {
                            delete = Some(id);
                        }
                    });
                }
                if let Some(id) = delete {
                    state.delete(id);
                }
            })
            .response
            .on_hover_text("Conversations for this project");
        })
        .response
        .on_disabled_hover_text("Wait for the assistant to finish");
        if ui.add_enabled(!v.running, egui::Button::new("+").small()).on_hover_text("New chat").clicked() {
            state.new_chat();
        }
    });
    // agent, model and effort on a row of their own: three pickers do not fit beside the title
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        menu::button(ui, format!("{} ▾", agent_label(v.backend)), |ui| {
            for &(value, label) in AGENTS {
                if menu::check(ui, v.backend == value, label, "").clicked() && v.backend != value {
                    *out = Out::Backend(value);
                }
            }
        })
        .response
        .on_hover_text("Which agent runs the requests (Settings ▸ Assistant)");
        model_picker(ui, state, v, out);
        effort_picker(ui, v, out);
    });
}

/// How hard the model thinks, for agents that take it.
fn effort_picker(ui: &mut egui::Ui, v: &View, out: &mut Out) {
    if v.backend != "claude" {
        return;
    }
    let shown = EFFORTS.iter().find(|(id, _)| *id == v.effort).map_or("Default effort", |(_, l)| l);
    menu::button(ui, format!("{shown} ▾"), |ui| {
        if menu::check(ui, v.effort.is_empty(), "Default effort", "").clicked() {
            *out = Out::Effort(String::new());
        }
        ui.separator();
        for &(id, label) in EFFORTS {
            if menu::check(ui, v.effort == id, label, "").clicked() {
                *out = Out::Effort(id.to_string());
            }
        }
    })
    .response
    .on_hover_text("Effort: how hard the model thinks. Higher is slower and more thorough.");
}

/// Past chats of this project, newest first: click one to open it.
fn history_tab(ui: &mut egui::Ui, state: &mut AssistantState, v: &View) {
    let now = now_secs();
    let current = state.conv().map(|c| c.id);
    let mut rows: Vec<(f64, u64, String, String)> = state
        .convs
        .iter()
        .filter(|c| !c.is_empty())
        .map(|c| {
            let asked = c.lines.iter().filter(|l| matches!(l, Line::User(_))).count();
            let mut sub = format!("{} · {asked} message{}", ago(c.updated, now), if asked == 1 { "" } else { "s" });
            if !c.tasks.is_empty() {
                sub += &format!(" · {} task{}", c.tasks.len(), if c.tasks.len() == 1 { "" } else { "s" });
            }
            (c.updated, c.id, short(c.title(), 60), sub)
        })
        .collect();
    rows.sort_by(|a, b| b.0.total_cmp(&a.0));
    egui::ScrollArea::vertical().id_salt("assistant_history").auto_shrink([false, false]).show(ui, |ui| {
        if rows.is_empty() {
            ui.add_space(8.0);
            ui.weak("No past chats in this project yet. Chats and tasks are saved as you go.");
            return;
        }
        let (mut open, mut delete) = (None, None);
        ui.add_enabled_ui(!v.running, |ui| {
            for (_, id, title, sub) in &rows {
                ui.horizontal(|ui| {
                    if ui.selectable_label(current == Some(*id), title.as_str()).clicked() {
                        open = Some(*id);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if x_button(ui, ("del_hist", *id)).on_hover_text("Delete this chat").clicked() {
                            delete = Some(*id);
                        }
                    });
                });
                ui.label(egui::RichText::new(sub.as_str()).small().weak());
                ui.add_space(4.0);
            }
        })
        .response
        .on_disabled_hover_text("Wait for the assistant to finish");
        if let Some(id) = delete {
            state.delete(id);
        }
        if let Some(id) = open {
            state.open(id);
            // a chat that only has tasks opens on them
            let chatty = state.conv().is_some_and(|c| !c.lines.is_empty());
            state.tab = if chatty { Tab::Chat } else { Tab::Tasks };
        }
    });
}

fn model_picker(ui: &mut egui::Ui, state: &mut AssistantState, v: &View, out: &mut Out) {
    match v.backend {
        "external" => {}
        "claude" if !state.model_other && (v.model.is_empty() || CLAUDE_MODELS.iter().any(|(id, _)| *id == v.model)) => {
            let shown = if v.model.is_empty() { "Default model" } else { model_label(v.model) };
            menu::button(ui, format!("{shown} ▾"), |ui| {
                if menu::check(ui, v.model.is_empty(), "Default model", "").clicked() {
                    *out = Out::Model(String::new());
                }
                for (i, &(id, label)) in CLAUDE_MODELS.iter().enumerate() {
                    if i == 0 || i == CLAUDE_ALIASES {
                        ui.separator();
                    }
                    if menu::check(ui, v.model == id, label, "").clicked() {
                        *out = Out::Model(id.to_string());
                    }
                }
                ui.separator();
                if menu::row(ui, None, "Other…", "").clicked() {
                    state.model_other = true;
                }
            })
            .response
            .on_hover_text("Model");
        }
        _ => {
            let mut m = v.model.to_string();
            let r = ui.add(egui::TextEdit::singleline(&mut m).hint_text("model id").desired_width(130.0));
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
    let (slash, done) = slash_list(ui, &mut state.task_prompt, &mut state.slash_sel, v.skills, tid);
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
            if v.running {
                ui.weak("Working…");
            }
        });
}

fn empty_chat(ui: &mut egui::Ui, state: &mut AssistantState, v: &View, out: &mut Out) {
    ui.add_space(16.0);
    ui.vertical_centered(|ui| {
        ui.weak(match v.backend {
            "external" => "Requests wait for a connected agent (assistant.wait).",
            "custom" => "Requests run your custom command (Settings ▸ Assistant).",
            _ => "Ask the assistant to edit. It sees what you attach - and, while it follows the selection, your \
                  selection, In/Out and playhead - and works through the editor's own tools. Every request is \
                  one undo step.",
        });
        ui.add_space(8.0);
        // ws:skills: the skills the user ran last, then the first ones; each chip is one Button so the
        // row wraps instead of widening the pane
        ui.horizontal_wrapped(|ui| {
            for s in chip_skills(v.skills, v.recent, CHIPS) {
                let r = ui.add_enabled(!v.running, egui::Button::new(format!("/{}", s.name)).small());
                if r.on_hover_text(s.description.as_str()).clicked() {
                    run_skill(state, s, out);
                }
            }
        });
        if !v.skills.is_empty() {
            ui.weak("Type / for every skill.");
        }
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

fn conversation(ui: &mut egui::Ui, state: &mut AssistantState, v: &View, pal: &Palette, out: &mut Out) {
    let lines = state.log().to_vec();
    let last_done = lines.iter().rposition(|l| matches!(l, Line::Done { .. }));
    let mut reply_from = 0; // first line of the current reply (after the last user message)
    let mut i = 0;
    while i < lines.len() {
        match &lines[i] {
            Line::User(t) => {
                if i > 0 {
                    ui.add_space(8.0);
                }
                bubble(ui, t, pal);
                reply_from = i + 1;
            }
            Line::Attached { chips, images } => {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for img in images {
                            thumb(ui, state, img, 0.6);
                        }
                        for c in chips {
                            ui.label(egui::RichText::new(c).small().weak());
                        }
                    });
                });
            }
            Line::Text(t) => crate::ui::markdown::show(ui, t, pal),
            Line::Tool(_) => {
                let n = lines[i..].iter().take_while(|l| matches!(l, Line::Tool(_))).count();
                let label = egui::RichText::new(format!("{n} action{}", if n == 1 { "" } else { "s" })).small().weak();
                egui::CollapsingHeader::new(label).id_salt(("assistant_tools", state.conv().map_or(0, |c| c.id), i)).show(
                    ui,
                    |ui| {
                        for l in &lines[i..i + n] {
                            if let Line::Tool(t) = l {
                                ui.label(egui::RichText::new(t).monospace().small().weak());
                            }
                        }
                    },
                );
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
                    let ok = undo_state(undo, v.undo_top);
                    // only the newest request can still be the top entry; older ones just show Copy
                    if Some(i) == last_done || ok.is_ok() {
                        let r = ui.add_enabled(ok.is_ok() && !v.running, egui::Button::new("Undo").small());
                        let r = match ok {
                            Ok(()) => r.on_hover_text("Undo everything this request did, in one step"),
                            Err(why) => r.on_disabled_hover_text(why),
                        };
                        if r.clicked() {
                            *out = Out::Undo;
                        }
                    }
                    let text: Vec<&str> = lines[reply_from..i]
                        .iter()
                        .filter_map(|l| if let Line::Text(t) = l { Some(t.as_str()) } else { None })
                        .collect();
                    if ui.add_enabled(!text.is_empty(), egui::Button::new("Copy").small()).clicked() {
                        ui.ctx().copy_text(text.join("\n\n"));
                    }
                });
            }
        }
        i += 1;
    }
}

/// The user's message: a right-aligned rounded bubble, at most ~85 % of the width.
fn bubble(ui: &mut egui::Ui, text: &str, pal: &Palette) {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let natural = ui.painter().layout_no_wrap(text.to_owned(), font, pal.text).size().x;
    let margin = egui::Margin::symmetric(10, 6);
    let max = (ui.available_width() * 0.85).max(60.0);
    let w = natural.min(max - margin.sum().x);
    ui.horizontal(|ui| {
        ui.add_space((ui.available_width() - w - margin.sum().x).max(0.0));
        egui::Frame::new().fill(pal.header).corner_radius(10u8).inner_margin(margin).show(ui, |ui| {
            ui.set_max_width(w);
            ui.add(egui::Label::new(egui::RichText::new(text).color(pal.text)).wrap());
        });
    });
}

/// An attachment's thumbnail (loaded from its file once), `scale` of `THUMB`; hover shows it bigger.
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
        Some(t) => {
            let r = ui.add(egui::Image::new(egui::load::SizedTexture::new(t.id(), size)).corner_radius(3u8));
            r.on_hover_ui(|ui| {
                ui.add(egui::Image::new(egui::load::SizedTexture::new(t.id(), size * 3.0 / scale)));
                ui.weak(format!("{} · {}×{}", a.name, a.width, a.height));
            })
        }
        None => ui.weak(format!("[{}]", a.name)),
    }
}

/// A chip: a label and an ×, one widget so a row of them wraps. True when it was clicked (= remove).
fn chip(ui: &mut egui::Ui, pal: &Palette, label: &str, pinned: bool) -> bool {
    let text = egui::RichText::new(format!("{label}  ×")).small();
    let button = egui::Button::new(text)
        .fill(if pinned { pal.header } else { pal.panel })
        .stroke(egui::Stroke::new(1.0, if pinned { pal.accent } else { pal.border }))
        .corner_radius(8u8);
    ui.add(button)
        .on_hover_text(if pinned {
            "Added on purpose: sent with the next message. Click to remove."
        } else {
            "Live: follows the selection. Click to leave it out of the next message."
        })
        .clicked()
}

fn composer(ui: &mut egui::Ui, state: &mut AssistantState, v: &View, pal: &Palette, out: &mut Out) {
    ui.add_space(4.0);
    egui::Frame::new()
        .fill(pal.bg)
        .stroke(egui::Stroke::new(1.0, pal.border))
        .corner_radius(10u8)
        .inner_margin(egui::Margin::symmetric(8, 6))
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
                            thumb(ui, state, a, 0.5);
                            if x_button(ui, ("img", i)).on_hover_text("Remove").clicked() {
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
            let id = ui.id().with("assistant_prompt");
            let (slash, done) = slash_list(ui, &mut state.prompt, &mut state.slash_sel, v.skills, id);
            if done {
                (state.cursor_end, state.focus_prompt) = (true, true);
            }
            let r = ui.add(
                egui::TextEdit::multiline(&mut state.prompt)
                    .id(id)
                    .frame(false)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY)
                    .hint_text("Ask the assistant…  (Enter sends, Shift+Enter for a new line, / for skills)")
                    .return_key(egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, egui::Key::Enter))
                    // while the `/` list is open Tab completes instead of moving the focus on
                    .lock_focus(slash),
            );
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
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                menu::button(ui, "+", |ui| {
                    if menu::row(ui, Some(Glyph::Camera), "Current frame", "").clicked() {
                        *out = Out::Attach(Attach::Frame);
                    }
                    if menu::row(ui, Some(Glyph::Cursor), "Selection", "").clicked() {
                        *out = Out::Attach(Attach::Pin(Pin::Smart));
                    }
                    if menu::row(ui, None, "In/Out range", "").clicked() {
                        *out = Out::Attach(Attach::Pin(Pin::Range));
                    }
                    if menu::row(ui, Some(Glyph::Folder), "Image file…", "").clicked() {
                        *out = Out::Attach(Attach::File);
                    }
                })
                .response
                .on_hover_text("Attach a frame, an image or context");
                let mic = glyph_text_button(ui, Glyph::Mic, if state.mic_wanted { "Stop" } else { "" });
                let mic = mic.on_hover_text(if state.mic_wanted { "Stop dictating" } else { "Dictate the request" });
                if mic.clicked() {
                    state.mic_wanted = !state.mic_wanted;
                }
                if !state.mic_status.is_empty() {
                    ui.weak(state.mic_status.as_str());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if v.running {
                        if glyph_text_button(ui, Glyph::Stop, "Stop").on_hover_text("Stop the assistant").clicked() {
                            *out = Out::Stop;
                        }
                    } else if ui.add_enabled(can_send, egui::Button::new("Send")).clicked() {
                        *out = Out::Send;
                    }
                });
            });
        });
    ui.add_space(2.0);
}

/// ws:skills: the `/` list above a prompt box (`text`, widget `id`) while a skill name is typed:
/// filtered as the user types; Up / Down pick (`sel`), Enter / Tab / a click complete to `/name `.
/// Returns (open, completed this frame); the caller focuses the box and moves its cursor on a completion.
pub fn slash_list(ui: &mut egui::Ui, text: &mut String, sel: &mut usize, all: &[Skill], id: egui::Id) -> (bool, bool) {
    let Some(q) = slash_query(text) else {
        *sel = 0;
        return (false, false);
    };
    let hits = skills::filter(all, q, 8);
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
    for (i, s) in hits.iter().enumerate() {
        let mut text = format!("/{}", s.name);
        if !s.args.is_empty() {
            text += &format!(" {}", s.args);
        }
        if !s.description.is_empty() {
            text += &format!("  ·  {}", s.description);
        }
        let r = ui.add(egui::Button::selectable(i == *sel, text).truncate());
        if r.on_hover_text(s.description.as_str()).clicked() {
            pick = Some(i);
        }
    }
    ui.separator();
    if let Some(i) = pick {
        *text = format!("/{} ", hits[i].name);
        *sel = 0;
    }
    (true, pick.is_some())
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
        let mut state = AssistantState::default();
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
        };
        for tab in [Tab::Tasks, Tab::Chat] {
            state.tab = tab;
            run_frames(&ctx, &mut state, &v, 30);
            assert!(!ctx.has_requested_repaint(), "an idle Assistant pane ({tab:?}) must request no repaint");
        }
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
        let mut state = AssistantState { tab: Tab::Skills, ..Default::default() };
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
