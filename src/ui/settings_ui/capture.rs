//! The Capture tab (screen recording + voiceover device settings) and the Export tab (encoder/CRF/
//! preset). Extracted verbatim from settings_ui.rs (see mod.rs's module doc for the whole settings
//! window).

use crate::settings::Settings;
use crate::ui::{combo, encoder_options, ENCODER_PRESETS};
use eframe::egui;

pub(super) fn capture_tab(ui: &mut egui::Ui, s: &mut Settings, audio_inputs: &[(String, bool)]) -> bool {
    let mut changed = false;
    let devices = |ui: &mut egui::Ui, id: &str, value: &mut String, want_loopback: bool| -> bool {
        let mut opts: Vec<(&str, &str)> = vec![("", "(none)")];
        opts.extend(audio_inputs.iter().filter(|(_, lb)| !want_loopback || *lb).map(|(n, _)| (n.as_str(), n.as_str())));
        combo(ui, id, value, &opts, Some(260.0))
    };
    ui.strong("Screen recording");
    egui::Grid::new("capture").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
        ui.label("Frame rate");
        ui.horizontal(|ui| {
            changed |= ui.add(egui::DragValue::new(&mut s.capture_fps).range(5..=120)).changed();
            ui.weak("fps");
        });
        ui.end_row();

        ui.label("Bitrate");
        ui.horizontal(|ui| {
            changed |=
                ui.add(egui::DragValue::new(&mut s.capture_bitrate_kbps).range(0..=100_000).speed(100)).changed();
            ui.weak("kbit/s (0 = use the export quality / CRF)");
        });
        ui.end_row();

        ui.label("Audio");
        let mut key = s.capture_audio_mode().key().to_string();
        let modes: Vec<(&str, &str)> =
            crate::engine::capture::AudioMode::ALL.iter().map(|m| (m.key(), m.label())).collect();
        if combo(ui, "cap_audio", &mut key, &modes, Some(260.0)) {
            s.capture_audio = key;
            changed = true;
        }
        ui.end_row();

        ui.label("Microphone");
        changed |= devices(ui, "cap_mic", &mut s.capture_mic, false);
        ui.end_row();

        ui.label("Output folder");
        ui.horizontal(|ui| {
            changed |= ui
                .add(egui::TextEdit::singleline(&mut s.capture_dir).desired_width(200.0).hint_text("temp folder"))
                .changed();
            if ui.button("Browse…").clicked() {
                if let Some(p) = rfd::FileDialog::new().pick_folder() {
                    s.capture_dir = p.to_string_lossy().into_owned();
                    changed = true;
                }
            }
        });
        ui.end_row();
    });
    changed |= ui.checkbox(&mut s.capture_cursor, "Record the mouse cursor").changed();
    changed |= ui
        .checkbox(&mut s.capture_on_blur, "Record while the editor is in the background (starts when it loses focus)")
        .changed();
    if audio_inputs.is_empty() {
        ui.weak("No audio input devices found (ffmpeg -list_devices).");
    }
    ui.add_space(6.0);
    ui.separator();
    ui.strong("Voiceover");
    egui::Grid::new("voice").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
        ui.label("Input device");
        changed |= devices(ui, "voice_dev", &mut s.voice_device, false);
        ui.end_row();
        ui.label("Channels");
        ui.horizontal(|ui| {
            changed |= ui.add(egui::DragValue::new(&mut s.voice_channels).range(1..=2)).changed();
            ui.weak("1 = mono, 2 = stereo");
        });
        ui.end_row();
    });
    changed
}

pub(super) fn export(ui: &mut egui::Ui, s: &mut Settings, encoders: &[String]) -> bool {
    let mut changed = false;
    egui::Grid::new("export").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
        ui.label("Encoder");
        ui.horizontal(|ui| {
            let opts: Vec<(&str, &str)> = std::iter::once(("auto", "auto"))
                .chain(encoder_options(encoders).into_iter().map(|e| (e, e)))
                .collect();
            changed |= combo(ui, "encoder", &mut s.encoder, &opts, Some(260.0));
            if encoders.is_empty() {
                ui.weak("(ffmpeg not found)");
            }
        });
        ui.end_row();

        ui.label("Quality (CRF)");
        ui.horizontal(|ui| {
            changed |= ui.add(egui::DragValue::new(&mut s.crf).range(0..=51)).changed();
            ui.weak("18 ≈ visually lossless, 23 default; lower = better / larger");
        });
        ui.end_row();

        ui.label("Preset");
        // "auto" follows the Export pane's Speed choice; a named preset wins over it
        let opts: Vec<(&str, &str)> =
            std::iter::once(("auto", "auto (Speed)")).chain(ENCODER_PRESETS.iter().map(|p| (*p, *p))).collect();
        changed |= combo(ui, "preset", &mut s.preset, &opts, Some(260.0));
        ui.end_row();

        // ---- ws:parity-io ----
        ui.label("Levels");
        changed |= ui
            .checkbox(&mut s.export_full_range, "Full range")
            .on_hover_text(
                "SDR H.264 / HEVC exports use 0-255 levels, tagged full range, instead of the broadcast 16-235. \
                 For screen recordings and players that expect full range; most platforms want it off.",
            )
            .changed();
        ui.end_row();
    });
    changed
}
