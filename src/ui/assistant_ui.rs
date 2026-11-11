//! ---- ws:assistant-pane ----
//! The Assistant pane widget (pure: no `App`): the request log, context chips, quick prompts, the
//! prompt box (Enter sends, Shift+Enter is a new line), Send / Stop and the mic button. It returns what
//! the user asked for (`Out`); `ui/app/assistant_ctl.rs` acts on it.

use super::tools::{glyph_text_button, Glyph};
use eframe::egui;

/// One row of the pane's log.
#[derive(Clone, Debug, PartialEq)]
pub enum Line {
    User(String),
    Text(String),
    /// A tool call: "name {args}".
    Tool(String),
    Error(String),
    Info(String),
    /// A request finished. `undo` = (undo-stack length right after its entry was pushed, that entry's
    /// label) when it changed anything.
    Done { undo: Option<(usize, String)> },
}

/// Shared state of the Assistant pane (`App.assistant`). `mcp-context` reads `pending`
/// (`assistant.wait`); `voice-input` drives `mic_wanted` / `mic_status` and appends to `prompt`.
#[derive(Default)]
pub struct AssistantState {
    /// The text in the prompt box (voice dictation appends here).
    pub prompt: String,
    /// Requests sent with the "external" backend, waiting for an MCP client to take them
    /// through `assistant.wait` (oldest first).
    pub pending: std::collections::VecDeque<String>,
    /// The pane's log, oldest first.
    pub log: Vec<Line>,
    /// Set when dictation or a quick action wants the prompt sent on the next frame.
    pub send_now: bool,
    /// The mic button: while true the voice code records; when it goes false it stops and transcribes.
    pub mic_wanted: bool,
    /// Written by the voice code: "" when idle, else "Listening…", "Transcribing…" or an error.
    pub mic_status: String,
    /// Context chips (keys) the user clicked off for the next request; cleared after each send.
    pub chips_off: Vec<&'static str>,
}

/// What the pane shows that it doesn't own.
pub struct View<'a> {
    pub running: bool,
    /// (key, label) for each piece of context that would be sent (`assistant_ctl::chips_of`).
    pub chips: &'a [(&'static str, String)],
    /// (label, prompt) quick-prompt buttons.
    pub quick: &'a [(String, String)],
    /// (undo-stack length, label of its top entry) - decides whether a request's "Undo all" still applies.
    pub undo_top: (usize, Option<&'a str>),
    /// "claude" | "custom" | "external", for the empty-log hint.
    pub backend: &'a str,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Out {
    None,
    /// Send `state.prompt`.
    Send,
    Stop,
    /// "Undo all" on a finished request (it is the top of the undo stack).
    Undo,
}

/// Whether a finished request's "Undo all" applies: Ok, or why not.
pub fn undo_state(undo: &Option<(usize, String)>, top: (usize, Option<&str>)) -> Result<(), &'static str> {
    match undo {
        None => Err("This request didn't change anything"),
        Some((len, label)) if *len == top.0 && top.1 == Some(label.as_str()) => Ok(()),
        Some(_) => Err("Other edits came after it - use History to step back further"),
    }
}

pub fn draw(ui: &mut egui::Ui, state: &mut AssistantState, v: &View) -> Out {
    let mut out = Out::None;
    let reserve = 150.0;
    egui::ScrollArea::vertical()
        .id_salt("assistant_log")
        .stick_to_bottom(true)
        .auto_shrink([false, false])
        .max_height((ui.available_height() - reserve).max(60.0))
        .show(ui, |ui| {
            if state.log.is_empty() {
                ui.weak(match v.backend {
                    "external" => "Requests wait for a connected agent (assistant.wait).",
                    "custom" => "Requests run your custom command (Settings ▸ Assistant).",
                    _ => "Ask Claude to edit: it sees your selection, In/Out and playhead, and works through \
                          the editor's own tools. Every request is one undo step.",
                });
            }
            let last = state.log.len().saturating_sub(1);
            for (i, line) in state.log.iter().enumerate() {
                match line {
                    Line::User(t) => {
                        if i > 0 {
                            ui.separator();
                        }
                        ui.label(egui::RichText::new(t).strong());
                    }
                    Line::Text(t) => {
                        ui.label(t);
                    }
                    Line::Tool(t) => {
                        ui.label(egui::RichText::new(format!("› {t}")).monospace().small().weak());
                    }
                    Line::Error(t) => {
                        ui.colored_label(ui.visuals().error_fg_color, t);
                    }
                    Line::Info(t) => {
                        ui.weak(t);
                    }
                    Line::Done { undo } => {
                        ui.horizontal(|ui| {
                            ui.weak("Finished");
                            let ok = undo_state(undo, v.undo_top);
                            // only the newest request can still be the top entry; older ones just say done
                            if i == last || ok.is_ok() {
                                let r = ui.add_enabled(ok.is_ok() && !v.running, egui::Button::new("Undo all").small());
                                let r = match ok {
                                    Ok(()) => r.on_hover_text("Undo everything this request did, in one step"),
                                    Err(why) => r.on_disabled_hover_text(why),
                                };
                                if r.clicked() {
                                    out = Out::Undo;
                                }
                            }
                        });
                    }
                }
            }
            if v.running {
                ui.weak("Working…");
            }
        });
    ui.separator();
    if !v.chips.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.weak("Context:");
            for (key, label) in v.chips {
                let on = !state.chips_off.contains(key);
                let tip = if on { "Sent with the request - click to leave it out" } else { "Left out - click to send it" };
                if ui.selectable_label(on, label.as_str()).on_hover_text(tip).clicked() {
                    if on {
                        state.chips_off.push(*key);
                    } else {
                        state.chips_off.retain(|k| k != key);
                    }
                }
            }
        });
    }
    if !v.quick.is_empty() {
        ui.horizontal_wrapped(|ui| {
            for (label, prompt) in v.quick {
                let r = ui.add_enabled(!v.running, egui::Button::new(label.as_str()).small());
                if r.on_hover_text(prompt.as_str()).clicked() {
                    state.prompt = prompt.clone();
                    out = Out::Send;
                }
            }
        });
    }
    let id = ui.id().with("assistant_prompt");
    let r = ui.add(
        egui::TextEdit::multiline(&mut state.prompt)
            .id(id)
            .desired_rows(3)
            .desired_width(f32::INFINITY)
            .hint_text("Ask the assistant…  (Enter sends, Shift+Enter for a new line)")
            .return_key(egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, egui::Key::Enter)),
    );
    let can_send = !v.running && !state.prompt.trim().is_empty();
    if r.has_focus() && can_send && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift) {
        out = Out::Send;
    }
    ui.horizontal(|ui| {
        if ui.add_enabled(can_send, egui::Button::new("Send")).clicked() {
            out = Out::Send;
        }
        if ui.add_enabled(v.running, egui::Button::new("Stop")).on_hover_text("Stop the agent").clicked() {
            out = Out::Stop;
        }
        let mic = glyph_text_button(ui, Glyph::Mic, if state.mic_wanted { "Stop" } else { "" });
        let mic = mic.on_hover_text(if state.mic_wanted { "Stop dictating" } else { "Dictate the request" });
        if mic.clicked() {
            state.mic_wanted = !state.mic_wanted;
        }
        if !state.mic_status.is_empty() {
            ui.weak(state.mic_status.as_str());
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An idle Assistant pane - a finished request in the log, chips and quick prompts showing -
    /// requests no repaint.
    #[test]
    fn assert_no_idle_repaint_assistant() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let mut state = AssistantState::default();
        state.log = vec![
            Line::User("Tighten the intro".into()),
            Line::Tool("editor.context".into()),
            Line::Text("Trimmed 3 pauses.".into()),
            Line::Done { undo: Some((4, "Claude: Tighten the intro".into())) },
        ];
        let chips = [("selection", "2 clips".to_string()), ("playhead", "Playhead 0:12".to_string())];
        let quick = crate::settings::default_assistant_quick();
        let v = View {
            running: false,
            chips: &chips,
            quick: &quick,
            undo_top: (4, Some("Claude: Tighten the intro")),
            backend: "claude",
        };
        for _ in 0..30 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| assert_eq!(draw(ui, &mut state, &v), Out::None));
            });
        }
        assert!(!ctx.has_requested_repaint(), "an idle Assistant pane must request no repaint");
    }

    #[test]
    fn undo_all_only_while_top_of_stack() {
        let u = Some((4, "Claude: x".to_string()));
        assert_eq!(undo_state(&u, (4, Some("Claude: x"))), Ok(()));
        assert!(undo_state(&u, (5, Some("Delete"))).is_err(), "an edit came after it");
        assert!(undo_state(&u, (3, Some("Claude: x"))).is_err(), "already undone");
        assert!(undo_state(&None, (4, None)).is_err(), "nothing to undo");
    }
}
