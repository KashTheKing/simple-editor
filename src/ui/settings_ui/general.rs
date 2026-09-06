//! The General tab: theme/decoder/ffmpeg-and-yt-dlp folders/downloads/preview width/snapping/confirm
//! overwrite/context-menu, imported fonts, and the MCP server section. Extracted verbatim from
//! settings_ui.rs (see mod.rs's module doc for the whole settings window).

use super::SettingsUi;
use crate::settings::Settings;
use crate::ui::combo;
use eframe::egui;

const THEMES: [(&str, &str); 3] = [("system", "System"), ("dark", "Dark"), ("light", "Light")];
const DECODERS: [(&str, &str); 3] =
    [("auto", "Auto (Media Foundation, ffmpeg fallback)"), ("mf", "Media Foundation"), ("ffmpeg", "ffmpeg")];

pub(super) fn general(ui: &mut egui::Ui, state: &mut SettingsUi, s: &mut Settings, mcp_status: &str) -> bool {
    let mut changed = false;
    egui::Grid::new("general").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
        ui.label("Theme");
        changed |= combo(ui, "theme", &mut s.theme, &THEMES, Some(260.0));
        ui.end_row();

        ui.label("Decoder");
        changed |= combo(ui, "decoder", &mut s.decoder, &DECODERS, Some(260.0));
        ui.end_row();

        ui.label("ffmpeg folder");
        ui.horizontal(|ui| {
            changed |=
                ui.add(egui::TextEdit::singleline(&mut s.ffmpeg_dir).desired_width(200.0).hint_text("auto")).changed();
            if ui.button("Browse…").clicked() {
                if let Some(p) = rfd::FileDialog::new().pick_folder() {
                    s.ffmpeg_dir = p.to_string_lossy().into_owned();
                    changed = true;
                }
            }
        });
        ui.end_row();

        ui.label("");
        if let Some(st) = &state.status {
            ui.weak(&st.ffmpeg);
        }
        ui.end_row();

        ui.label("yt-dlp folder");
        ui.horizontal(|ui| {
            changed |=
                ui.add(egui::TextEdit::singleline(&mut s.ytdlp_dir).desired_width(200.0).hint_text("auto")).changed();
            if ui.button("Browse…").clicked() {
                if let Some(p) = rfd::FileDialog::new().pick_folder() {
                    s.ytdlp_dir = p.to_string_lossy().into_owned();
                    changed = true;
                }
            }
        });
        ui.end_row();

        ui.label("");
        if let Some(st) = &state.status {
            ui.weak(&st.ytdlp);
        }
        ui.end_row();

        ui.label("Downloads folder");
        ui.horizontal(|ui| {
            changed |= ui
                .add(egui::TextEdit::singleline(&mut s.download_dir).desired_width(200.0).hint_text("Videos"))
                .changed();
            if ui.button("Browse…").clicked() {
                if let Some(p) = rfd::FileDialog::new().pick_folder() {
                    s.download_dir = p.to_string_lossy().into_owned();
                    changed = true;
                }
            }
        });
        ui.end_row();

        ui.label("Preview max width");
        ui.horizontal(|ui| {
            changed |= ui.add(egui::DragValue::new(&mut s.preview_max_width).range(320..=3840).speed(8)).changed();
            ui.weak("px");
        });
        ui.end_row();
    });
    ui.add_space(6.0);
    changed |= ui.checkbox(&mut s.snap, "Snapping in the timeline").changed();
    changed |= ui.checkbox(&mut s.confirm_overwrite, "Confirm before overwriting files").changed();
    changed |= ui
        .checkbox(
            &mut s.lossless_save,
            "Save (Ctrl+S) uses the instant lossless cut when the project is a plain cut (cuts snap to keyframes)",
        )
        .changed();
    ui.horizontal(|ui| {
        changed |= ui
            .checkbox(&mut s.context_menu, "Add 'Edit with Simple Editor' to the right-click menu of video files")
            .changed();
        if let Some(st) = &state.status {
            ui.weak(st.ctxmenu);
        }
    });
    ui.add_space(6.0);
    ui.separator();
    // ---- imported fonts ----
    ui.horizontal(|ui| {
        ui.label("Fonts");
        if ui.button("Import font…").clicked() {
            if let Some(paths) = rfd::FileDialog::new().add_filter("Fonts", &["ttf", "otf", "ttc"]).pick_files() {
                for p in paths {
                    let p = p.to_string_lossy().into_owned();
                    if !s.user_fonts.iter().any(|f| f.eq_ignore_ascii_case(&p)) {
                        s.user_fonts.push(p);
                        changed = true;
                    }
                }
            }
        }
        ui.weak("(.ttf / .otf, usable in text clips and subtitles)");
    });
    let mut remove = None;
    for (i, f) in s.user_fonts.iter().enumerate() {
        ui.horizontal(|ui| {
            if crate::ui::markers_ui::x_button(ui).on_hover_text("Remove this font").clicked() {
                remove = Some(i);
            }
            let name = std::path::Path::new(f).file_name().map(|n| n.to_string_lossy().into_owned());
            ui.label(name.unwrap_or_else(|| f.clone())).on_hover_text(f);
        });
    }
    if let Some(i) = remove {
        s.user_fonts.remove(i);
        changed = true;
    }
    ui.add_space(6.0);
    ui.separator();
    // ---- MCP server ----
    changed |= ui.checkbox(&mut s.mcp_enabled, "MCP server (AI co-editing)").changed();
    ui.horizontal(|ui| {
        ui.label("Port");
        changed |= port_field(ui, state, s);
        ui.weak(format!("http://127.0.0.1:{}/mcp", s.mcp_port));
    });
    ui.horizontal(|ui| {
        if ui.button("Copy Claude Code command").clicked() {
            ui.ctx().copy_text(crate::mcp::claude_code_command(s.mcp_port));
        }
        ui.weak(mcp_status);
    });
    changed
}

/// MCP port DragValue. The app restarts the server whenever `settings.mcp_port` changes, so the value is
/// held in `state.port_edit` while the user drags or types and only written when the gesture ends —
/// otherwise a drag from 7337 to 7400 walks through ~60 ports and one busy port in between turns the
/// server off. Returns true when the setting changed.
pub(super) fn port_field(ui: &mut egui::Ui, state: &mut SettingsUi, s: &mut Settings) -> bool {
    let mut port = state.port_edit.unwrap_or(s.mcp_port);
    let r = ui.add(egui::DragValue::new(&mut port).range(1024..=65535));
    if r.dragged() || r.has_focus() {
        state.port_edit = Some(port);
        return false;
    }
    state.port_edit = None;
    if port != s.mcp_port {
        s.mcp_port = port;
        return true;
    }
    false
}
