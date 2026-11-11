//! The Assistant pane widget. pro-schema declares the shared state and a placeholder body;
//! ws:assistant-pane fills in the real UI (prompt box, backend picker, log).

use eframe::egui;

/// Shared state of the Assistant pane. `pro-schema` declares it so `mcp-context` (the
/// `assistant.wait` tool) and `voice-input` (dictation into `prompt`) can use it before
/// `assistant-pane` fills in the UI.
#[derive(Default)]
#[allow(dead_code)] // pending / log / send_now are read once ws:assistant-pane and ws:mcp-context land
pub struct AssistantState {
    /// The text in the prompt box (voice dictation appends here).
    pub prompt: String,
    /// Requests sent with the "external" backend, waiting for an MCP client to take them
    /// through `assistant.wait` (oldest first).
    pub pending: std::collections::VecDeque<String>,
    /// Lines shown in the pane's log (what the agent said and did), oldest first.
    pub log: Vec<String>,
    /// Set when dictation or a quick action wants the prompt sent on the next frame.
    pub send_now: bool,
}

/// Placeholder body until ws:assistant-pane lands.
pub fn draw(ui: &mut egui::Ui, state: &mut AssistantState) {
    let _ = state;
    ui.weak("The assistant isn't wired up yet.");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An idle Assistant pane requests no repaint.
    #[test]
    fn assert_no_idle_repaint_assistant() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let mut state = AssistantState::default();
        for _ in 0..30 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| draw(ui, &mut state));
            });
        }
        assert!(!ctx.has_requested_repaint(), "an idle Assistant pane must request no repaint");
    }
}
