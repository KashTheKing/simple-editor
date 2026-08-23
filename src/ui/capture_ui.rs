//! Screen recording + voiceover windows (both non-modal, both keep the editor usable).
//!
//! Screen recorder: output folder, fps, bitrate (or CRF), area (Whole desktop / a Region typed in
//! desktop pixels), microphone combo (`engine::capture::audio_devices`), desktop audio toggle, cursor
//! toggle, and "Record when the editor loses focus" (auto start/stop) — the app watches focus and drives
//! `engine::capture`. While recording: elapsed time, a stop button and a note that the file is imported
//! into the library when it finishes. Both windows only *ask*: the app owns the running recording (and
//! its output path) and imports the file itself when it stops.
//!
//! Voiceover: input device, channels, a level meter, "Record from the playhead" (playback rolls while
//! recording, so the take lines up), stop → the WAV is imported and placed on an audio track at the start
//! time, with a "Retake" that deletes the last take and records again.

use crate::engine::capture::{audio_devices, ScreenCaptureOptions, VoiceoverOptions};
use crate::settings::Settings;
use crate::theme::Palette;
use crate::ui::combo;
use crate::ui::tools::{glyph_text_button, Glyph};
use eframe::egui;
use std::path::PathBuf;

#[derive(Default)]
pub struct CaptureUi {
    pub screen_open: bool,
    pub voice_open: bool,
    pub elapsed: f64,
    /// Region in desktop pixels (x, y, w, h) — typed in the window, used in "region" mode.
    pub region: Option<(i32, i32, u32, u32)>,
    /// "desktop" | "region"
    pub area: String,
    /// Input level 0..1 for the voiceover meter (fed by the app while recording).
    pub level: f32,
    /// Roll playback while recording the voiceover, so the take lines up with the timeline.
    pub from_playhead: bool,
    /// Timeline time the current voiceover take started at.
    pub voice_start: Option<f64>,
}

/// What the app should do this frame.
#[derive(Default)]
pub struct CaptureResponse {
    pub start_screen: bool,
    pub stop_screen: bool,
    pub start_voice: bool,
    pub stop_voice: bool,
    /// Filled together with `start_screen` / `start_voice` — the options the window built.
    pub screen: Option<ScreenCaptureOptions>,
    pub voice: Option<VoiceoverOptions>,
    /// Current "Record when the editor loses focus" state (the app watches focus and drives capture).
    pub auto_on_blur: bool,
    /// Throw the last voiceover take away and record again.
    pub retake: bool,
}

const AREAS: [(&str, &str); 2] = [("desktop", "Whole desktop"), ("region", "Region")];

/// Where recordings go: the configured folder, else the user's Videos folder, else the temp dir.
pub fn capture_dir(settings: &Settings) -> PathBuf {
    if !settings.capture_dir.trim().is_empty() {
        return PathBuf::from(settings.capture_dir.trim());
    }
    let videos = std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join("Videos"));
    match videos {
        Some(v) if v.is_dir() => v,
        _ => std::env::temp_dir(),
    }
}

fn out_path(settings: &Settings, prefix: &str, ext: &str) -> PathBuf {
    capture_dir(settings).join(format!("{prefix}-{}.{ext}", Settings::now()))
}

pub fn show(
    ctx: &egui::Context,
    state: &mut CaptureUi,
    settings: &mut Settings,
    recording_screen: bool,
    recording_voice: bool,
    palette: &Palette,
) -> CaptureResponse {
    // the app owns the output path of a running recording and imports it itself when it stops
    let mut r = CaptureResponse { auto_on_blur: settings.capture_on_blur, ..Default::default() };
    if state.area.is_empty() {
        state.area = "desktop".into();
    }
    if state.screen_open {
        screen_window(ctx, state, settings, recording_screen, &mut r);
    }
    if state.voice_open {
        voice_window(ctx, state, settings, recording_voice, palette, &mut r);
    }
    r
}

fn screen_window(
    ctx: &egui::Context,
    state: &mut CaptureUi,
    settings: &mut Settings,
    recording: bool,
    r: &mut CaptureResponse,
) {
    let devices = audio_devices();
    let has_loopback = devices.iter().any(|(_, lb)| *lb);
    let mut open = state.screen_open;
    egui::Window::new("Screen Recording").open(&mut open).resizable(false).default_width(320.0).show(ctx, |ui| {
        ui.add_enabled_ui(!recording, |ui| {
            egui::Grid::new("cap_opts").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                ui.label("Folder");
                ui.horizontal(|ui| {
                    let mut dir = capture_dir(settings).to_string_lossy().into_owned();
                    if ui.add(egui::TextEdit::singleline(&mut dir).desired_width(160.0)).changed() {
                        settings.capture_dir = dir.clone();
                    }
                    if ui.small_button("…").clicked() {
                        if let Some(p) = rfd::FileDialog::new().set_directory(&dir).pick_folder() {
                            settings.capture_dir = p.to_string_lossy().into_owned();
                        }
                    }
                });
                ui.end_row();

                ui.label("Frame rate");
                ui.add(egui::DragValue::new(&mut settings.capture_fps).range(1..=120).suffix(" fps"));
                ui.end_row();

                ui.label("Bitrate");
                ui.horizontal(|ui| {
                    ui.add(
                        egui::DragValue::new(&mut settings.capture_bitrate_kbps).range(0..=200_000).suffix(" kbit/s"),
                    );
                    if settings.capture_bitrate_kbps == 0 {
                        ui.weak(format!("quality CRF {}", settings.crf));
                    }
                });
                ui.end_row();

                ui.label("Capture");
                combo(ui, "cap_area", &mut state.area, &AREAS, Some(110.0));
                ui.end_row();

                if state.area == "region" {
                    // ponytail: typed coordinates, not a drag-to-pick desktop overlay — a fullscreen
                    // transparent viewport is a lot of machinery for four numbers.
                    let (mut x, mut y, mut w, mut h) = state.region.unwrap_or((0, 0, 1280, 720));
                    ui.label("Region");
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut x).prefix("x ").speed(2.0));
                        ui.add(egui::DragValue::new(&mut y).prefix("y ").speed(2.0));
                        ui.add(egui::DragValue::new(&mut w).prefix("w ").range(16..=16384).speed(2.0));
                        ui.add(egui::DragValue::new(&mut h).prefix("h ").range(16..=16384).speed(2.0));
                    });
                    state.region = Some((x, y, w, h));
                    ui.end_row();
                }

                ui.label("Microphone");
                let mut opts: Vec<(&str, &str)> = vec![("", "None")];
                opts.extend(devices.iter().map(|(n, _)| (n.as_str(), n.as_str())));
                combo(ui, "cap_mic", &mut settings.capture_mic, &opts, Some(180.0));
                ui.end_row();
            });
            ui.add_enabled_ui(has_loopback, |ui| {
                ui.checkbox(&mut settings.capture_desktop_audio, "Desktop audio").on_disabled_hover_text(
                    "No loopback input found — enable \"Stereo Mix\" in Windows sound settings.",
                );
            });
            ui.checkbox(&mut settings.capture_cursor, "Record the mouse cursor");
            if ui.checkbox(&mut settings.capture_on_blur, "Record when the editor loses focus").changed() {
                settings.save();
            }
            r.auto_on_blur = settings.capture_on_blur;
        });
        ui.separator();
        ui.horizontal(|ui| {
            if recording {
                crate::ui::tools::glyph_label(ui, Glyph::Record, ui.visuals().error_fg_color);
                ui.colored_label(ui.visuals().error_fg_color, "REC");
                ui.label(clock(state.elapsed));
                if ui.button("Stop").clicked() {
                    r.stop_screen = true;
                }
            } else if glyph_text_button(ui, Glyph::Record, "Record").clicked() {
                r.screen = Some(screen_options(settings, state, out_path(settings, "screen", "mp4"), has_loopback));
                r.start_screen = true;
                settings.save();
            }
        });
        ui.weak("The file is added to the library when the recording stops.");
    });
    state.screen_open = open;
}

/// The options the recorder should run with (the region only applies in "region" mode).
pub(crate) fn screen_options(
    settings: &Settings,
    state: &CaptureUi,
    out: PathBuf,
    has_loopback: bool,
) -> ScreenCaptureOptions {
    ScreenCaptureOptions {
        out,
        fps: settings.capture_fps.clamp(1, 120),
        bitrate_kbps: settings.capture_bitrate_kbps,
        crf: settings.crf,
        region: if state.area == "region" { state.region } else { None },
        mic: settings.capture_mic.clone(),
        desktop_audio: settings.capture_desktop_audio && has_loopback,
        cursor: settings.capture_cursor,
    }
}

/// The options a voiceover take should run with.
pub(crate) fn voice_options(settings: &Settings) -> VoiceoverOptions {
    VoiceoverOptions {
        out: out_path(settings, "voice", "wav"),
        device: settings.voice_device.clone(),
        sample_rate: 48_000,
        channels: settings.voice_channels.clamp(1, 2),
    }
}

fn voice_window(
    ctx: &egui::Context,
    state: &mut CaptureUi,
    settings: &mut Settings,
    recording: bool,
    palette: &Palette,
    r: &mut CaptureResponse,
) {
    let devices = audio_devices();
    let mut open = state.voice_open;
    egui::Window::new("Voiceover").open(&mut open).resizable(false).default_width(280.0).show(ctx, |ui| {
        ui.add_enabled_ui(!recording, |ui| {
            egui::Grid::new("vo_opts").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                ui.label("Input");
                let opts: Vec<(&str, &str)> = devices.iter().map(|(n, _)| (n.as_str(), n.as_str())).collect();
                combo(ui, "vo_dev", &mut settings.voice_device, &opts, Some(170.0));
                ui.end_row();
                ui.label("Channels");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut settings.voice_channels, 1, "Mono");
                    ui.selectable_value(&mut settings.voice_channels, 2, "Stereo");
                });
                ui.end_row();
            });
            ui.checkbox(&mut state.from_playhead, "Record from the playhead");
        });
        meter(ui, state.level, palette);
        ui.horizontal(|ui| {
            if recording {
                crate::ui::tools::glyph_label(ui, Glyph::Record, ui.visuals().error_fg_color);
                ui.colored_label(ui.visuals().error_fg_color, "REC");
                ui.label(clock(state.elapsed));
                if ui.button("Stop").clicked() {
                    r.stop_voice = true;
                }
            } else {
                let can = !settings.voice_device.is_empty();
                if ui
                    .add_enabled_ui(can, |ui| glyph_text_button(ui, Glyph::Mic, "Record"))
                    .inner
                    .on_disabled_hover_text("Pick an input.")
                    .clicked()
                {
                    r.voice = Some(voice_options(settings));
                    r.start_voice = true;
                    settings.save();
                }
                if state.voice_start.is_some() && ui.button("Retake").clicked() {
                    r.retake = true;
                    r.start_voice = true;
                    r.voice = Some(voice_options(settings));
                }
            }
        });
        if state.from_playhead {
            ui.weak("Playback rolls while recording; the take lands at the playhead.");
        }
    });
    state.voice_open = open;
}

/// Flat input-level bar (green → the accent colour as it approaches clipping).
fn meter(ui: &mut egui::Ui, level: f32, palette: &Palette) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width().min(240.0), 8.0), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 0, palette.panel);
    let l = level.clamp(0.0, 1.0);
    if l > 0.0 {
        let mut filled = rect;
        filled.set_width(rect.width() * l);
        p.rect_filled(filled, 0, if l > 0.95 { palette.playhead } else { palette.waveform });
    }
    p.rect_stroke(rect, 0, egui::Stroke::new(1.0, palette.border), egui::StrokeKind::Inside);
}

/// "M:SS" — recordings are minutes long, not hours.
fn clock(secs: f64) -> String {
    let s = secs.max(0.0);
    format!("{}:{:04.1}", (s / 60.0).floor() as u64, s % 60.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> CaptureUi {
        CaptureUi { screen_open: true, voice_open: true, area: "desktop".into(), ..Default::default() }
    }

    #[test]
    fn options_follow_the_settings() {
        let mut s = Settings::default();
        s.capture_fps = 240; // clamped
        s.capture_mic = "Mic".into();
        let mut st = state();
        st.region = Some((10, 20, 640, 480));
        let o = screen_options(&s, &st, PathBuf::from("a.mp4"), false);
        assert_eq!(o.fps, 120);
        assert_eq!(o.region, None, "the region only applies in region mode");
        assert_eq!(o.mic, "Mic");
        assert!(!o.desktop_audio, "no loopback device → no desktop audio, whatever the setting says");
        st.area = "region".into();
        let o = screen_options(&s, &st, PathBuf::from("a.mp4"), true);
        assert_eq!(o.region, Some((10, 20, 640, 480)));
        assert!(o.desktop_audio);
    }

    /// Headless: both windows lay out, idle and recording, and ask for nothing on their own.
    #[test]
    fn show_headless() {
        let ctx = egui::Context::default();
        let palette = Palette::new(false, egui::Color32::BLUE);
        let mut s = Settings::default();
        for area in ["desktop", "region"] {
            for rec in [false, true] {
                let mut st = CaptureUi { area: area.into(), elapsed: 3.5, ..state() };
                let mut r = CaptureResponse::default();
                let _ = ctx.run(egui::RawInput::default(), |ctx| r = show(ctx, &mut st, &mut s, rec, rec, &palette));
                assert!(!r.start_screen && !r.stop_screen && !r.start_voice && !r.stop_voice);
                assert!(r.screen.is_none() && r.voice.is_none() && !r.retake);
                assert_eq!(r.auto_on_blur, s.capture_on_blur);
                assert!(st.screen_open && st.voice_open);
            }
        }
        // region mode seeds an editable region instead of silently recording the whole desktop
        let mut st = CaptureUi { area: "region".into(), ..state() };
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            show(ctx, &mut st, &mut s, false, false, &palette);
        });
        assert!(st.region.is_some(), "region mode always has a concrete region");
    }

    #[test]
    fn clock_text() {
        assert_eq!(clock(0.0), "0:00.0");
        assert_eq!(clock(65.25), "1:05.2");
        assert_eq!(clock(-1.0), "0:00.0");
    }

    #[test]
    fn capture_dir_falls_back() {
        let mut s = Settings::default();
        s.capture_dir = r"C:\shots ".into();
        assert_eq!(capture_dir(&s), PathBuf::from(r"C:\shots"));
        s.capture_dir = "  ".into();
        assert!(capture_dir(&s).is_dir(), "the fallback is always a real folder");
    }
}
