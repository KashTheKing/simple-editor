//! ---- ws:assistant-pane ----
//! The Assistant tab: which agent the Assistant pane drives (`assistant_data::AGENTS`), its model, the
//! API key (Windows Credential Manager, never settings.json), the guard on tools that leave the
//! project, and the quick-prompt buttons. The pane's header has the same agent / model pickers.

use crate::engine::agent;
use crate::settings::Settings;
use eframe::egui;
use std::sync::atomic::{AtomicU8, Ordering};

/// Whether a key is stored: 0 = not checked yet, 1 = no, 2 = yes (a Credential Manager read per frame
/// is not free).
static KEY_STORED: AtomicU8 = AtomicU8::new(0);

pub(super) fn assistant_tab(ui: &mut egui::Ui, s: &mut Settings) -> bool {
    let mut changed = false;
    egui::Grid::new("assistant_settings").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
        ui.label("Agent");
        ui.vertical(|ui| {
            for &(value, label) in crate::ui::assistant_data::AGENTS {
                let hint = match value {
                    "claude" => "Runs the `claude` CLI with this editor's tools only (no shell, no files).",
                    "codex" => "Runs `codex exec` with this editor's MCP server (check `codex --help` if it fails).",
                    "custom" => "Runs your own command line - any agent CLI that speaks MCP.",
                    _ => "Requests wait for an MCP client looping on assistant.wait.",
                };
                changed |= ui.radio_value(&mut s.assistant_backend, value.to_string(), label).on_hover_text(hint).changed();
            }
        });
        ui.end_row();

        match s.assistant_backend.as_str() {
            "claude" => {
                ui.label("Model");
                ui.vertical(|ui| {
                    changed |= ui
                        .add(egui::TextEdit::singleline(&mut s.assistant_model).hint_text("default").desired_width(200.0))
                        .on_hover_text("default, fable, opus, sonnet, haiku or a full model name")
                        .changed();
                    // the PATH scan runs when the tab opens (no draw last frame) or on "Check again", not per frame
                    let id = ui.id().with("assistant_claude_exe");
                    let frame = ui.ctx().cumulative_frame_nr();
                    let cached: Option<(u64, Option<std::path::PathBuf>)> = ui.data(|d| d.get_temp(id));
                    let mut exe = match cached {
                        Some((last, exe)) if last + 1 >= frame => exe,
                        _ => agent::claude_exe(),
                    };
                    ui.horizontal(|ui| {
                        match &exe {
                            Some(p) => ui.weak(format!("Found {}", p.display())),
                            None => ui.colored_label(ui.visuals().warn_fg_color, agent::NOT_FOUND),
                        };
                        if ui.small_button("Check again").clicked() {
                            exe = agent::claude_exe();
                        }
                    });
                    ui.data_mut(|d| d.insert_temp(id, (frame, exe)));
                });
                ui.end_row();
            }
            "custom" => {
                ui.label("Command");
                ui.vertical(|ui| {
                    changed |= ui
                        .add(
                            egui::TextEdit::singleline(&mut s.assistant_command)
                                .hint_text("my-agent --mcp {mcp_url} \"{prompt}\"")
                                .desired_width(f32::INFINITY),
                        )
                        .changed();
                    ui.weak(
                        "{prompt}, {mcp_url} and {model} are filled in; \"double quotes\" group words. No shell \
                         runs it.",
                    );
                });
                ui.end_row();
                ui.label("Key variable");
                changed |= ui
                    .add(egui::TextEdit::singleline(&mut s.assistant_key_env).hint_text("OPENAI_API_KEY").desired_width(200.0))
                    .on_hover_text("The stored API key is passed to your command in this environment variable")
                    .changed();
                ui.end_row();
            }
            "codex" => {
                ui.label("Model");
                changed |= ui
                    .add(egui::TextEdit::singleline(&mut s.assistant_model).hint_text("default").desired_width(200.0))
                    .changed();
                ui.end_row();
            }
            _ => {}
        }

        if s.assistant_backend != "external" {
            ui.label("API key");
            key_row(ui, &s.assistant_backend);
            ui.end_row();
        }

        ui.label("Guard");
        changed |= ui
            .checkbox(&mut s.assistant_guard, "Ask before an agent exports, opens a project, changes settings or runs scripts")
            .on_hover_text("Applies to every MCP client, including UI actions that do the same")
            .changed();
        ui.end_row();

        ui.label("Voice");
        changed |= ui.checkbox(&mut s.voice_autosend, "Send as soon as I stop talking").changed();
        ui.end_row();
    });

    ui.add_space(8.0);
    ui.strong("Quick prompts");
    ui.weak("Buttons in the Assistant pane: a label and the request it sends.");
    let mut remove = None;
    for (i, (label, prompt)) in s.assistant_quick.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            changed |= ui.add(egui::TextEdit::singleline(label).desired_width(140.0)).changed();
            changed |= ui.add(egui::TextEdit::singleline(prompt).desired_width(ui.available_width() - 40.0)).changed();
            if ui.small_button("Remove").clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        s.assistant_quick.remove(i);
        changed = true;
    }
    ui.horizontal(|ui| {
        if ui.button("Add").clicked() {
            s.assistant_quick.push(("New prompt".into(), String::new()));
            changed = true;
        }
        if ui.button("Reset to built-ins").clicked() {
            s.assistant_quick = crate::settings::default_assistant_quick();
            changed = true;
        }
    });
    changed
}

/// Save / Remove the key. The typed key sits in egui's temp memory until saved, then is cleared.
fn key_row(ui: &mut egui::Ui, backend: &str) {
    if KEY_STORED.load(Ordering::Relaxed) == 0 {
        KEY_STORED.store(if agent::key_load().is_some() { 2 } else { 1 }, Ordering::Relaxed);
    }
    let stored = KEY_STORED.load(Ordering::Relaxed) == 2;
    let id = ui.id().with("assistant_key_buf");
    let mut buf: String = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut buf)
                    .password(true)
                    .hint_text(if stored { "a key is stored" } else { "optional" })
                    .desired_width(260.0),
            );
            if ui.add_enabled(!buf.trim().is_empty(), egui::Button::new("Save")).clicked() {
                match agent::key_save(&buf) {
                    Ok(()) => KEY_STORED.store(2, Ordering::Relaxed),
                    Err(e) => eprintln!("assistant key: {e}"),
                }
                buf.clear();
            }
            if ui.add_enabled(stored, egui::Button::new("Remove")).clicked() {
                agent::key_delete();
                KEY_STORED.store(1, Ordering::Relaxed);
            }
        });
        let var = match backend {
            "claude" => "ANTHROPIC_API_KEY",
            "codex" => "OPENAI_API_KEY",
            _ => "the key variable",
        };
        ui.weak(format!(
            "Kept in Windows Credential Manager (\"SimpleEditor/assistant\"), passed to the agent as {var}. \
             Leave empty to use the CLI's own sign-in."
        ));
    });
    ui.data_mut(|d| d.insert_temp(id, buf));
}
