//! The Screen Recording pane (a dockable pane like the others; Record ▸ Screen Recording opens it in
//! its own window) and the voiceover state shared with the Voiceover pane (`ui::voiceover_ui`).
//!
//! Screen recorder, top to bottom: a live preview of exactly what will be recorded (a small, low-rate
//! GDI grab, `engine::screen_grab`), the source (a screen, all screens, a window or a typed region),
//! the audio choice (System audio only / Microphone only / Both / None, remembered in settings), the
//! microphone with its level meter, the system-audio meter, the output folder, frame rate, bitrate,
//! cursor and "record when the editor loses focus", then a big Record / Stop button with the timer.
//! The pane only *asks*: the app owns the running recording (`ui::app::recorder_pane`) and imports
//! the file once it is finished and verified.
//!
//! Voiceover: input device, channels, a level meter, "Record from the playhead" (playback rolls while
//! recording, so the take lines up), stop → the WAV is imported and placed on an audio track at the start
//! time, with a "Retake" that deletes the last take and records again.

use crate::engine::capture::{AudioMode, CaptureSource, ScreenCaptureOptions, VoiceoverOptions};
use crate::engine::screen_grab::Preview;
use crate::settings::Settings;
use crate::theme::Palette;
use crate::ui::combo;
use crate::ui::screens::Monitor;
use eframe::egui;
use std::path::PathBuf;

#[derive(Default)]
pub struct CaptureUi {
    pub elapsed: f64,
    /// Region in desktop pixels (x, y, w, h) - typed in the pane, used in "region" mode.
    pub region: Option<(i32, i32, u32, u32)>,
    /// "screen:<n>" (a monitor) | "desktop" (all of them) | "window" | "region"
    pub area: String,
    /// The window to record in "window" mode (its title) and the titles last listed.
    pub window: String,
    pub windows: Vec<String>,
    /// Input level 0..1 for the voiceover meter (fed by the app while recording).
    pub level: f32,
    /// Screen recording levels (system, mic), fed by the app from the recording or the meter taps.
    pub levels: (Option<f32>, Option<f32>),
    /// Why an audio device can't be heard (no output device, microphone unplugged...).
    pub audio_error: Option<String>,
    /// Run the live preview (the app sets it while the pane is on screen; headless tests leave it off).
    pub live: bool,
    pub preview: Option<Preview>,
    pub preview_tex: Option<(egui::TextureHandle, u64)>,
    /// Microphone names (cpal / WASAPI), listed once when the pane first shows.
    pub mics: Option<Vec<String>>,
    /// Meter-only audio taps while not recording, and the "mode:mic" they were opened for.
    pub meter_taps: Vec<crate::engine::audio_tap::AudioTap>,
    pub meter_key: String,
    /// The screen recording is being finished (muxed and checked) after Stop.
    pub finishing: bool,
    /// Roll playback while recording the voiceover, so the take lines up with the timeline.
    pub from_playhead: bool,
    /// The last voiceover take: the timeline time it was started from, and its file. Retake removes its
    /// clips and records again from there.
    pub last_take: Option<(f64, PathBuf)>,
}

impl CaptureUi {
    /// The pane went off screen: stop grabbing the screen and listening to the devices.
    pub fn sleep(&mut self) {
        if self.live || self.preview.is_some() || !self.meter_taps.is_empty() {
            self.live = false;
            self.preview = None;
            self.preview_tex = None;
            self.meter_taps.clear();
            self.meter_key.clear();
            self.levels = (None, None);
            self.audio_error = None;
        }
    }
}

/// What the app should do this frame.
#[derive(Default)]
pub struct CaptureResponse {
    /// Filled when Record was pressed - the options the pane built.
    pub screen: Option<ScreenCaptureOptions>,
    pub stop_screen: bool,
    pub open_folder: bool,
}

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

/// The source the pane's pick stands for. An unknown monitor index falls back to the first monitor.
pub fn source_of(state: &CaptureUi, monitors: &[Monitor]) -> CaptureSource {
    let screen = |i: usize| {
        monitors.get(i).or(monitors.first()).map(|m| {
            let [x, y, w, h] = m.rect;
            CaptureSource::Region(x, y, w.max(2) as u32, h.max(2) as u32)
        })
    };
    match state.area.as_str() {
        "desktop" => CaptureSource::Desktop,
        "window" if !state.window.is_empty() => CaptureSource::Window(state.window.clone()),
        "region" => {
            let (x, y, w, h) = state.region.unwrap_or((0, 0, 1280, 720));
            CaptureSource::Region(x, y, w, h)
        }
        a => screen(a.strip_prefix("screen:").and_then(|n| n.parse().ok()).unwrap_or(0))
            .unwrap_or(CaptureSource::Desktop),
    }
}

/// The options the recorder should run with.
pub(crate) fn screen_options(settings: &Settings, state: &CaptureUi, monitors: &[Monitor], out: PathBuf) -> ScreenCaptureOptions {
    ScreenCaptureOptions {
        out,
        fps: settings.capture_fps.clamp(1, 120),
        bitrate_kbps: settings.capture_bitrate_kbps,
        crf: settings.crf,
        source: source_of(state, monitors),
        audio: settings.capture_audio_mode(),
        mic: settings.capture_mic.clone(),
        cursor: settings.capture_cursor,
    }
}

/// A new recording's options with a fresh `screen-<time>.mp4` in the capture folder.
pub(crate) fn new_screen_options(settings: &Settings, state: &CaptureUi, monitors: &[Monitor]) -> ScreenCaptureOptions {
    screen_options(settings, state, monitors, out_path(settings, "screen", "mp4"))
}

/// The Screen Recording pane's body.
#[allow(clippy::too_many_arguments)]
pub fn screen_pane(
    ui: &mut egui::Ui,
    state: &mut CaptureUi,
    settings: &mut Settings,
    mics: &[String],
    monitors: &[Monitor],
    recording: bool,
    palette: &Palette,
) -> CaptureResponse {
    let mut r = CaptureResponse::default();
    if state.area.is_empty() {
        state.area = "screen:0".into();
    }
    // the controls' height, so the preview shrinks before anything has to scroll
    let mode = settings.capture_audio_mode();
    let rows = 8
        + usize::from(matches!(state.area.as_str(), "window" | "region"))
        + 2 * usize::from(mode.mic())
        + usize::from(mode.system())
        + usize::from(state.audio_error.is_some());
    let max_h = (ui.available_height() - rows as f32 * 27.0 - 100.0).max(110.0);
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        preview(ui, state, monitors, recording, palette, max_h);
        ui.add_space(6.0);
        let busy = recording || state.finishing;
        let mut save = false;
        egui::Grid::new("screen_rec").num_columns(2).spacing([12.0, 6.0]).striped(false).show(ui, |ui| {
            ui.label("Source");
            ui.add_enabled_ui(!busy, |ui| {
                let names: Vec<(String, String)> = monitors
                    .iter()
                    .enumerate()
                    .map(|(i, m)| {
                        let [_, _, w, h] = m.rect;
                        (format!("screen:{i}"), format!("Screen {} ({w}×{h})", i + 1))
                    })
                    .chain([("desktop".into(), "All screens".into()), ("window".into(), "Window".into()), ("region".into(), "Region".into())])
                    .collect();
                let opts: Vec<(&str, &str)> = names.iter().map(|(k, l)| (k.as_str(), l.as_str())).collect();
                if combo(ui, "rec_source", &mut state.area, &opts, Some(200.0)) && state.area == "window" {
                    state.windows = crate::engine::screen_grab::windows();
                }
            });
            ui.end_row();

            if state.area == "window" {
                ui.label("Window");
                ui.add_enabled_ui(!busy, |ui| {
                    ui.horizontal(|ui| {
                        if state.windows.is_empty() {
                            state.windows = crate::engine::screen_grab::windows();
                        }
                        let opts: Vec<(&str, &str)> = state.windows.iter().map(|t| (t.as_str(), t.as_str())).collect();
                        combo(ui, "rec_window", &mut state.window, &opts, Some(200.0));
                        if ui.button("Refresh").on_hover_text("List the open windows again").clicked() {
                            state.windows = crate::engine::screen_grab::windows();
                        }
                    });
                });
                ui.end_row();
            }

            if state.area == "region" {
                // ponytail: typed coordinates, not a drag-to-pick desktop overlay - a fullscreen
                // transparent viewport is a lot of machinery for four numbers (the preview shows the result).
                let (mut x, mut y, mut w, mut h) = state.region.unwrap_or((0, 0, 1280, 720));
                ui.label("Region");
                ui.add_enabled_ui(!busy, |ui| {
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut x).prefix("x ").speed(2.0));
                        ui.add(egui::DragValue::new(&mut y).prefix("y ").speed(2.0));
                        ui.add(egui::DragValue::new(&mut w).prefix("w ").range(16..=16384).speed(2.0));
                        ui.add(egui::DragValue::new(&mut h).prefix("h ").range(16..=16384).speed(2.0));
                    });
                });
                state.region = Some((x, y, w, h));
                ui.end_row();
            }

            ui.label("Audio");
            ui.add_enabled_ui(!busy, |ui| {
                let mut key = mode.key().to_string();
                let opts: Vec<(&str, &str)> = AudioMode::ALL.iter().map(|m| (m.key(), m.label())).collect();
                if combo(ui, "rec_audio", &mut key, &opts, Some(200.0)) {
                    settings.capture_audio = key;
                    save = true;
                }
            });
            ui.end_row();

            if mode.mic() {
                ui.label("Microphone");
                ui.add_enabled_ui(!busy, |ui| {
                    let mut opts: Vec<(&str, &str)> = vec![("", "Default microphone")];
                    opts.extend(mics.iter().map(|n| (n.as_str(), n.as_str())));
                    save |= combo(ui, "rec_mic", &mut settings.capture_mic, &opts, Some(200.0));
                });
                ui.end_row();
                ui.label("Mic level");
                meter(ui, state.levels.1.unwrap_or(0.0), palette);
                ui.end_row();
            }
            if mode.system() {
                ui.label("System level");
                meter(ui, state.levels.0.unwrap_or(0.0), palette);
                ui.end_row();
            }
            if let Some(e) = state.audio_error.as_deref().filter(|_| mode != AudioMode::None) {
                ui.label("");
                ui.colored_label(ui.visuals().error_fg_color, e);
                ui.end_row();
            }

            ui.label("Folder");
            ui.horizontal(|ui| {
                let mut dir = capture_dir(settings).to_string_lossy().into_owned();
                ui.add_enabled_ui(!busy, |ui| {
                    if ui.add(egui::TextEdit::singleline(&mut dir).desired_width(150.0)).changed() {
                        settings.capture_dir = dir.clone();
                        save = true;
                    }
                    if ui.button("Browse…").clicked() {
                        if let Some(p) = rfd::FileDialog::new().set_directory(&dir).pick_folder() {
                            settings.capture_dir = p.to_string_lossy().into_owned();
                            save = true;
                        }
                    }
                });
                if ui.button("Open").on_hover_text("Show the folder in Explorer").clicked() {
                    r.open_folder = true;
                }
            });
            ui.end_row();

            ui.label("Frame rate");
            ui.add_enabled_ui(!busy, |ui| {
                save |= ui.add(egui::DragValue::new(&mut settings.capture_fps).range(1..=120).suffix(" fps")).changed();
            });
            ui.end_row();

            ui.label("Bitrate");
            ui.add_enabled_ui(!busy, |ui| {
                save |= ui
                    .add(egui::DragValue::new(&mut settings.capture_bitrate_kbps).range(0..=200_000).suffix(" kbit/s"))
                    .on_hover_text("0 uses the export quality (CRF) instead")
                    .changed();
            });
            ui.end_row();

            ui.label("Cursor");
            ui.add_enabled_ui(!busy, |ui| save |= ui.checkbox(&mut settings.capture_cursor, "Record the cursor").changed());
            ui.end_row();

            ui.label("Background");
            save |= ui
                .checkbox(&mut settings.capture_on_blur, "Record when the editor loses focus")
                .on_hover_text("Starts when you switch to another app and stops when you come back")
                .changed();
            ui.end_row();
        });
        if save {
            settings.save();
        }

        ui.add_space(10.0);
        let w = ui.available_width().min(360.0);
        let big = |text: String, fill: egui::Color32| {
            egui::Button::new(egui::RichText::new(text).size(18.0).strong().color(egui::Color32::WHITE))
                .fill(fill)
                .min_size(egui::vec2(w, 44.0))
        };
        if recording {
            if ui.add(big(format!("Stop   {}", clock(state.elapsed)), palette.playhead)).clicked() {
                r.stop_screen = true;
            }
        } else if state.finishing {
            ui.add_enabled(false, big("Finishing…".into(), palette.border));
        } else if ui.add(big("Record".into(), egui::Color32::from_rgb(0xD9, 0x2D, 0x2D))).clicked() {
            r.screen = Some(new_screen_options(settings, state, monitors));
        }
        ui.add_space(4.0);
        ui.weak(if state.finishing {
            "Checking the file, then it goes into the library."
        } else {
            "The file is added to the library when the recording stops."
        });
    });
    r
}

/// The live preview: the latest small grab, letterboxed into the pane's width (16:9 box, 240 px max,
/// `max_h` when the pane is short).
fn preview(ui: &mut egui::Ui, state: &mut CaptureUi, monitors: &[Monitor], recording: bool, palette: &Palette, max_h: f32) {
    let w = ui.available_width().max(160.0);
    let h = (w * 9.0 / 16.0).min(240.0).min(max_h);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::hover());
    ui.painter().rect_filled(rect, 4, egui::Color32::BLACK);
    if state.live {
        let src = source_of(state, monitors);
        match &state.preview {
            Some(p) => p.set_source(src),
            None => state.preview = Some(Preview::start(src)),
        }
        if let Some(p) = &state.preview {
            // half the rate while recording: the encoder always comes first
            p.set_interval(if recording { 500 } else { 250 });
            let gen = p.generation();
            if state.preview_tex.as_ref().map(|(_, g)| *g) != Some(gen) {
                if let Some(f) = p.frame() {
                    let img = egui::ColorImage::from_rgba_unmultiplied(f.size, &f.rgba);
                    match state.preview_tex.as_mut() {
                        Some((t, g)) => {
                            t.set(img, egui::TextureOptions::LINEAR);
                            *g = gen;
                        }
                        None => state.preview_tex = Some((ui.ctx().load_texture("rec-preview", img, egui::TextureOptions::LINEAR), gen)),
                    }
                }
            }
        }
    }
    match &state.preview_tex {
        Some((t, _)) if state.live => {
            let s = t.size_vec2();
            let k = (rect.width() / s.x).min(rect.height() / s.y);
            let img = egui::Rect::from_center_size(rect.center(), s * k);
            ui.painter().image(t.id(), img, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
        }
        _ => {
            let msg = if state.area == "window" && state.window.is_empty() { "Pick a window" } else { "Preview" };
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, msg, egui::FontId::proportional(14.0), palette.border);
        }
    }
    if recording {
        let at = rect.left_top() + egui::vec2(10.0, 10.0);
        ui.painter().circle_filled(at + egui::vec2(6.0, 8.0), 6.0, egui::Color32::from_rgb(0xE5, 0x30, 0x30));
        ui.painter().text(at + egui::vec2(18.0, 8.0), egui::Align2::LEFT_CENTER, format!("REC {}", clock(state.elapsed)), egui::FontId::proportional(14.0), egui::Color32::WHITE);
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

/// Flat input-level bar (green → the accent colour as it approaches clipping).
pub(crate) fn meter(ui: &mut egui::Ui, level: f32, palette: &Palette) {
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

/// "M:SS" - recordings are minutes long, not hours.
pub(crate) fn clock(secs: f64) -> String {
    let s = secs.max(0.0);
    format!("{}:{:04.1}", (s / 60.0).floor() as u64, s % 60.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitors() -> Vec<Monitor> {
        let m = |x, primary| Monitor { name: format!("D{x}"), rect: [x, 0, 1920, 1080], work: [x, 0, 1920, 1040], primary };
        vec![m(0, true), m(1920, false)]
    }

    #[test]
    fn options_follow_the_settings_and_the_source() {
        let mut s = Settings::default();
        s.capture_fps = 240; // clamped
        s.capture_mic = "Mic".into();
        s.capture_audio = "both".into();
        let mut st = CaptureUi { area: "screen:1".into(), ..Default::default() };
        let o = screen_options(&s, &st, &monitors(), PathBuf::from("a.mp4"));
        assert_eq!(o.fps, 120);
        assert_eq!(o.source, CaptureSource::Region(1920, 0, 1920, 1080), "screen 2 is its monitor's rectangle");
        assert_eq!((o.audio, o.mic.as_str()), (AudioMode::Both, "Mic"));
        st.area = "screen:9".into();
        assert_eq!(source_of(&st, &monitors()), CaptureSource::Region(0, 0, 1920, 1080), "a gone monitor → the first");
        st.area = "region".into();
        st.region = Some((10, 20, 640, 480));
        assert_eq!(source_of(&st, &monitors()), CaptureSource::Region(10, 20, 640, 480));
        st.area = "window".into();
        assert_eq!(source_of(&st, &monitors()), CaptureSource::Region(0, 0, 1920, 1080), "no window picked yet");
        st.window = "Notepad".into();
        assert_eq!(source_of(&st, &monitors()), CaptureSource::Window("Notepad".into()));
        st.area = "desktop".into();
        assert_eq!(source_of(&st, &monitors()), CaptureSource::Desktop);
        // the audio choice is remembered as its key; anything unknown is system audio
        s.capture_audio = "?".into();
        assert_eq!(s.capture_audio_mode(), AudioMode::System);
    }

    fn run(st: &mut CaptureUi, s: &mut Settings, rec: bool) -> (CaptureResponse, bool) {
        let ctx = egui::Context::default();
        let palette = Palette::new(false, egui::Color32::BLUE);
        let mics = vec!["Mic".to_string()];
        let mut r = CaptureResponse::default();
        let mut repaint = false;
        for _ in 0..3 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| r = screen_pane(ui, st, s, &mics, &monitors(), rec, &palette));
            });
            repaint = ctx.has_requested_repaint();
        }
        (r, repaint)
    }

    /// Headless (no live preview): every source and audio mode lays out, idle and recording, asks for
    /// nothing on its own and never asks to repaint (the app drives the preview's refresh).
    #[test]
    fn assert_no_idle_repaint_screen_recording_pane() {
        let mut s = Settings::default();
        for area in ["screen:0", "desktop", "window", "region"] {
            for mode in AudioMode::ALL {
                s.capture_audio = mode.key().into();
                for rec in [false, true] {
                    let mut st = CaptureUi { area: area.into(), elapsed: 3.5, windows: vec!["A".into()], ..Default::default() };
                    let (r, repaint) = run(&mut st, &mut s, rec);
                    assert!(r.screen.is_none() && !r.stop_screen && !r.open_folder);
                    assert!(!repaint, "{area} / {mode:?} / recording {rec} requested a repaint");
                }
            }
        }
        // region mode seeds an editable region instead of silently recording the whole desktop
        let mut st = CaptureUi { area: "region".into(), ..Default::default() };
        run(&mut st, &mut s, false);
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
