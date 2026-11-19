//! The Screen Recording pane (a dockable pane like the others; Record ▸ Screen Recording opens it in
//! its own window, and it docks into any page) and the voiceover state shared with the Voiceover pane
//! (`ui::voiceover_ui`).
//!
//! Always on top: the live preview (30 fps, the capture itself, cursor included; right-click to pause
//! it), the level meters and the big Record / Stop with the timer. Below, folds in the Inspector's
//! style (`inspector::section`, remembered in `Settings.capture_folds`):
//! * Sources: Screen / Window / Region / Application, the pick, the Audio choice (what "All audio"
//!   mixes), the microphone, and the audio tracks (each its own stream: name, source, on, meter).
//! * Quality: frame rate, bitrate, scale, encoder.
//! * Misc: cursor, Hide on Record and its stop hotkey, record-on-blur.
//! * Output: folder, Library bin, file name, add to the library.
//! The pane only *asks*: the app owns the recording, the preview producers and the meter taps
//! (`ui::app::recorder_pane`) and imports the file once it is finished and verified.
//!
//! Voiceover: input device, channels, a level meter, "Record from the playhead" (playback rolls while
//! recording, so the take lines up), stop → the WAV is imported and placed on an audio track at the start
//! time, with a "Retake" that deletes the last take and records again.

use crate::engine::capture::{AudioMode, CaptureSource, ScreenCaptureOptions, TrackKind, TrackSpec, VoiceoverOptions};
use crate::engine::screen_grab::{FrameSlot, Preview};
use crate::engine::wgc::AppWindow;
use crate::settings::Settings;
use crate::theme::Palette;
use crate::ui::combo;
use crate::ui::screens::Monitor;
use eframe::egui;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Default)]
pub struct CaptureUi {
    pub elapsed: f64,
    /// Region in desktop pixels (x, y, w, h) - typed in the pane, used in "region" mode.
    pub region: Option<(i32, i32, u32, u32)>,
    /// "screen:<n>" (a monitor) | "desktop" (all of them) | "window" | "region" | "app"
    pub area: String,
    /// The window to record in "window" mode (its title) and the titles last listed.
    pub window: String,
    pub windows: Vec<String>,
    /// The application to record in "app" mode, the ones last listed and their icons.
    pub app: Option<AppWindow>,
    pub apps: Vec<AppWindow>,
    pub app_icons: HashMap<isize, egui::TextureHandle>,
    /// Input level 0..1 for the voiceover meter (fed by the app while recording).
    pub level: f32,
    /// Screen recording levels [system, mic, app], fed by the app from the recording or the meter taps.
    pub levels: [Option<f32>; 3],
    /// Why an audio device can't be heard (no output device, microphone unplugged...).
    pub audio_error: Option<String>,
    /// The pane is on screen (the app sets it; headless tests leave it off).
    pub live: bool,
    /// The before-recording preview producer (owned here, started / stopped by the app).
    pub preview: Option<Preview>,
    /// The frames the preview shows now (the recording's own, or `preview`'s).
    pub slot: Option<Arc<FrameSlot>>,
    pub preview_tex: Option<(egui::TextureHandle, u64)>,
    /// Preview frames per second shown, measured each second (for `capture.screen` status): (since, frames, fps).
    pub preview_rate: Option<(std::time::Instant, u64, f32)>,
    /// Microphone names (cpal / WASAPI), listed once when the pane first shows.
    pub mics: Option<Vec<String>>,
    /// Meter-only audio taps while not recording, and the key they were opened for.
    pub meter_taps: Vec<crate::engine::audio_tap::AudioTap>,
    /// Which level (0 system, 1 mic, 2 app) each meter tap feeds.
    pub meter_slots: Vec<usize>,
    pub meter_key: String,
    /// Hide on Record's Stop pill and the stop hotkey, while a recording runs.
    pub pill: Option<crate::engine::rec_pill::Pill>,
    /// A clone of the UI context, so the pill / hotkey thread can wake the app.
    pub ctx: Option<egui::Context>,
    /// The screen recording is being finished (muxed and checked) after Stop.
    pub finishing: bool,
    /// The New bin box under the Library bin pick.
    pub new_bin: String,
    /// The stop hotkey couldn't be registered.
    pub hotkey_error: Option<String>,
    /// Roll playback while recording the voiceover, so the take lines up with the timeline.
    pub from_playhead: bool,
    /// The last voiceover take: the timeline time it was started from, and its file. Retake removes its
    /// clips and records again from there.
    pub last_take: Option<(f64, PathBuf)>,
}

impl CaptureUi {
    /// The pane went off screen: stop the preview and stop listening to the devices.
    pub fn sleep(&mut self) {
        if self.live || self.preview.is_some() || !self.meter_taps.is_empty() || self.slot.is_some() {
            self.live = false;
            self.preview = None;
            if let Some(s) = self.slot.take() {
                s.wanted.store(false, std::sync::atomic::Ordering::Relaxed);
            }
            self.preview_tex = None;
            self.meter_taps.clear();
            self.meter_key.clear();
            self.levels = [None; 3];
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
    /// Create this Library bin (the New bin box).
    pub new_bin: Option<String>,
}

/// What the pane shows that the app knows.
pub struct PaneCtx<'a> {
    pub mics: &'a [String],
    pub monitors: &'a [Monitor],
    /// The project's Library bins.
    pub bins: &'a [String],
    pub recording: bool,
    pub palette: &'a Palette,
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

/// The file name a recording gets from the pattern: {time} (seconds since 1970, unique per second),
/// {source} (the screen / window / app name). Never empty, never a character Windows refuses.
pub fn file_name(pattern: &str, source: &str, time: u64) -> String {
    let clean: String = source.chars().map(|c| if r#"<>:"/\|?*"#.contains(c) || c.is_control() { '_' } else { c }).collect();
    let name = pattern.replace("{time}", &time.to_string()).replace("{source}", clean.trim());
    let name: String = name.chars().map(|c| if r#"<>:"/\|?*"#.contains(c) || c.is_control() { '_' } else { c }).collect();
    let name = name.trim().trim_end_matches('.').to_string();
    if name.is_empty() {
        format!("screen-{time}")
    } else {
        name
    }
}

/// The source the pane's pick stands for. An unknown monitor index falls back to the first monitor;
/// "app" with nothing picked (or a window that closed) falls back the same way.
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
        "app" if state.app.is_some() => {
            let a = state.app.as_ref().expect("checked");
            CaptureSource::App { hwnd: a.hwnd, title: a.title.clone(), pid: a.pid }
        }
        "region" => {
            let (x, y, w, h) = state.region.unwrap_or((0, 0, 1280, 720));
            CaptureSource::Region(x, y, w, h)
        }
        a => screen(a.strip_prefix("screen:").and_then(|n| n.parse().ok()).unwrap_or(0))
            .unwrap_or(CaptureSource::Desktop),
    }
}

/// A short name for the source, for {source} in file names.
fn source_name(state: &CaptureUi, src: &CaptureSource) -> String {
    match src {
        CaptureSource::App { title, .. } | CaptureSource::Window(title) => title.chars().take(40).collect(),
        CaptureSource::Desktop => "All screens".into(),
        CaptureSource::Region(..) if state.area == "region" => "Region".into(),
        CaptureSource::Region(..) => {
            format!("Screen {}", state.area.strip_prefix("screen:").and_then(|n| n.parse::<usize>().ok()).unwrap_or(0) + 1)
        }
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
        tracks: settings.capture_tracks.clone(),
        scale: settings.capture_scale.clamp(10, 100),
        encoder: settings.capture_encoder.clone(),
        // the recording's own frames feed the preview while it runs (cheap: a 480x270 second output)
        preview: true,
    }
}

/// A new recording's options with a fresh file name in the capture folder.
pub(crate) fn new_screen_options(settings: &Settings, state: &CaptureUi, monitors: &[Monitor]) -> ScreenCaptureOptions {
    let src = source_of(state, monitors);
    let name = file_name(&settings.capture_name, &source_name(state, &src), Settings::now());
    screen_options(settings, state, monitors, capture_dir(settings).join(format!("{name}.mp4")))
}

/// A track's level from the source levels [system, mic, app].
fn track_level(kind: TrackKind, mode: AudioMode, l: [Option<f32>; 3]) -> f32 {
    let v = |o: Option<f32>| o.unwrap_or(0.0);
    match kind {
        TrackKind::Mix => {
            let s = if mode.system() { v(l[0]) } else { 0.0 };
            let m = if mode.mic() { v(l[1]) } else { 0.0 };
            s.max(m)
        }
        TrackKind::System => v(l[0]),
        TrackKind::Mic => v(l[1]),
        TrackKind::App => v(l[2]),
    }
}

/// The Screen Recording pane's body.
pub fn screen_pane(ui: &mut egui::Ui, state: &mut CaptureUi, settings: &mut Settings, c: &PaneCtx) -> CaptureResponse {
    let mut r = CaptureResponse::default();
    if state.area.is_empty() {
        state.area = "screen:0".into();
    }
    let busy = c.recording || state.finishing;
    let mode = settings.capture_audio_mode();
    let mut save = false;

    // ---- always visible: preview, meters, Record / Stop ----
    let max_h = (ui.available_height() * 0.45).clamp(110.0, 300.0);
    save |= preview(ui, state, settings, c, max_h);
    ui.add_space(4.0);
    let is_app = matches!(source_of(state, c.monitors), CaptureSource::App { .. });
    let (want_sys, want_mic, want_app) = {
        let o = ScreenCaptureOptions {
            audio: mode,
            tracks: settings.capture_tracks.clone(),
            source: source_of(state, c.monitors),
            ..Default::default()
        };
        let (s, m, a) = o.sources();
        (s, m, a.is_some() || (is_app && settings.capture_tracks.iter().any(|t| t.on && t.kind == TrackKind::App)))
    };
    egui::Grid::new("rec_meters").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
        for (on, label, i) in [(want_sys, "System", 0), (want_mic, "Microphone", 1), (want_app, "App", 2)] {
            if on {
                ui.label(label);
                meter(ui, state.levels[i].unwrap_or(0.0), c.palette);
                ui.end_row();
            }
        }
    });
    if let Some(e) = state.audio_error.as_deref().filter(|_| want_sys || want_mic || want_app) {
        ui.colored_label(ui.visuals().error_fg_color, e);
    }
    ui.add_space(6.0);
    let w = ui.available_width().min(420.0);
    let big = |text: String, fill: egui::Color32| {
        egui::Button::new(egui::RichText::new(text).size(18.0).strong().color(egui::Color32::WHITE)).fill(fill).min_size(egui::vec2(w, 44.0))
    };
    if c.recording {
        if ui.add(big(format!("Stop   {}", clock(state.elapsed)), c.palette.playhead)).clicked() {
            r.stop_screen = true;
        }
    } else if state.finishing {
        ui.add_enabled(false, big("Finishing…".into(), c.palette.border));
    } else {
        let ready = state.area != "app" || state.app.is_some();
        let b = ui.add_enabled(ready, big("Record".into(), egui::Color32::from_rgb(0xD9, 0x2D, 0x2D)));
        if b.on_disabled_hover_text("Pick an application under Sources").clicked() {
            r.screen = Some(new_screen_options(settings, state, c.monitors));
        }
    }
    ui.weak(if state.finishing {
        "Checking the file, then it goes into the library."
    } else if settings.capture_hide && !c.recording {
        "Simple Editor hides while recording; stop with the pill at the top of the screen or the hotkey."
    } else {
        "The file is added to the library when the recording stops."
    });
    ui.add_space(4.0);

    // ---- the settings, in folds ----
    let mut folds = std::mem::take(&mut settings.capture_folds);
    let before = folds.clone();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        crate::ui::inspector::section(ui, "rec_sources", "Sources", true, &mut folds, None, |ui| {
            ui.add_enabled_ui(!busy, |ui| save |= sources(ui, state, settings, c, mode));
        });
        crate::ui::inspector::section(ui, "rec_quality", "Quality", false, &mut folds, None, |ui| {
            ui.add_enabled_ui(!busy, |ui| save |= quality(ui, settings));
        });
        crate::ui::inspector::section(ui, "rec_misc", "Misc", false, &mut folds, None, |ui| {
            save |= misc(ui, state, settings, busy);
        });
        crate::ui::inspector::section(ui, "rec_output", "Output", false, &mut folds, None, |ui| {
            save |= output(ui, state, settings, c, busy, &mut r);
        });
    });
    save |= folds != before;
    settings.capture_folds = folds;
    if save {
        settings.save();
    }
    r
}

fn sources(ui: &mut egui::Ui, state: &mut CaptureUi, settings: &mut Settings, c: &PaneCtx, mode: AudioMode) -> bool {
    let mut save = false;
    egui::Grid::new("rec_src").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
        ui.label("Type");
        let mut kind = match state.area.as_str() {
            "window" | "region" | "app" => state.area.clone(),
            _ => "screen".to_string(),
        };
        let kinds = [("screen", "Screen"), ("window", "Window"), ("region", "Region"), ("app", "Application / Game")];
        if combo(ui, "rec_kind", &mut kind, &kinds, Some(200.0)) {
            state.area = if kind == "screen" { "screen:0".into() } else { kind.clone() };
            if kind == "window" {
                state.windows = crate::engine::screen_grab::windows();
            }
            if kind == "app" {
                state.apps = crate::engine::wgc::app_windows();
            }
        }
        ui.end_row();

        match kind.as_str() {
            "screen" => {
                ui.label("Screen");
                let names: Vec<(String, String)> = c
                    .monitors
                    .iter()
                    .enumerate()
                    .map(|(i, m)| {
                        let [_, _, w, h] = m.rect;
                        (format!("screen:{i}"), format!("Screen {} ({w}×{h})", i + 1))
                    })
                    .chain([("desktop".into(), "All screens".into())])
                    .collect();
                let opts: Vec<(&str, &str)> = names.iter().map(|(k, l)| (k.as_str(), l.as_str())).collect();
                combo(ui, "rec_screen", &mut state.area, &opts, Some(200.0));
                ui.end_row();
            }
            "window" => {
                ui.label("Window");
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
                ui.end_row();
            }
            "region" => {
                // ponytail: typed coordinates, not a drag-to-pick desktop overlay (the preview shows the result)
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
            _ => {
                ui.label("Application");
                ui.horizontal(|ui| {
                    app_pick(ui, state);
                    if ui.button("Refresh").on_hover_text("List the running applications again").clicked() {
                        state.apps = crate::engine::wgc::app_windows();
                    }
                });
                ui.end_row();
            }
        }

        ui.label("Audio");
        let mut key = mode.key().to_string();
        let opts: Vec<(&str, &str)> = AudioMode::ALL.iter().map(|m| (m.key(), m.label())).collect();
        if combo(ui, "rec_audio", &mut key, &opts, Some(200.0)) {
            settings.capture_audio = key;
            save = true;
        }
        ui.end_row();

        ui.label("Microphone");
        let mut opts: Vec<(&str, &str)> = vec![("", "Default microphone")];
        opts.extend(c.mics.iter().map(|n| (n.as_str(), n.as_str())));
        save |= combo(ui, "rec_mic", &mut settings.capture_mic, &opts, Some(200.0));
        ui.end_row();
    });

    ui.add_space(4.0);
    ui.label("Audio tracks");
    let mut remove = None;
    let is_app = state.area == "app";
    // one aligned row per track; the name box takes the width the fixed controls leave (a grid squeezed it
    // to a few letters in a narrow panel, so names could be neither read nor edited)
    let n = settings.capture_tracks.len();
    let kind_w = 110.0;
    let fixed = 22.0 + kind_w + 52.0 + 24.0 + 4.0 * 8.0; // tick, kind, meter, ✕, gaps
    let name_w = (ui.available_width() - fixed).clamp(90.0, 260.0);
    for (i, t) in settings.capture_tracks.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            save |= ui.checkbox(&mut t.on, "").on_hover_text("Record this track").changed();
            save |= ui
                .add(egui::TextEdit::singleline(&mut t.name).desired_width(name_w).hint_text("Track name"))
                .on_hover_text(t.name.clone())
                .changed();
            let mut k = t.kind.key().to_string();
            let kinds: Vec<(&str, &str)> = TrackKind::ALL.iter().map(|k| (k.key(), k.label())).collect();
            if combo(ui, &format!("rec_track_kind{i}"), &mut k, &kinds, Some(kind_w)) {
                t.kind = TrackKind::ALL.into_iter().find(|x| x.key() == k).unwrap_or_default();
                save = true;
            }
            let live = t.on && (t.kind != TrackKind::App || is_app);
            // a fixed-width meter: the shared one fills the row's free width and would push ✕ off the end
            let level = if live { track_level(t.kind, mode, state.levels) } else { 0.0 };
            ui.allocate_ui(egui::vec2(52.0, 16.0), |ui| meter(ui, level, c.palette));
            if ui.add_enabled(n > 1, egui::Button::new("✕").small()).on_hover_text("Remove this track").clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        settings.capture_tracks.remove(i);
        save = true;
    }
    ui.horizontal(|ui| {
        if settings.capture_tracks.len() < 6 && ui.button("Add track").clicked() {
            settings.capture_tracks.push(TrackSpec { name: format!("Track {}", settings.capture_tracks.len() + 1), kind: TrackKind::Mic, on: true });
            save = true;
        }
        if ui.button("Reset").on_hover_text("All audio, Microphone, App audio").clicked() {
            settings.capture_tracks = crate::engine::capture::default_tracks();
            save = true;
        }
    });
    if !is_app && settings.capture_tracks.iter().any(|t| t.on && t.kind == TrackKind::App) {
        ui.weak("App audio tracks record only with an Application source.");
    }
    save
}

/// The application combo: icon + title + program for each running app.
fn app_pick(ui: &mut egui::Ui, state: &mut CaptureUi) {
    if state.apps.is_empty() {
        state.apps = crate::engine::wgc::app_windows();
    }
    let label = state.app.as_ref().map(|a| a.title.chars().take(32).collect::<String>()).unwrap_or_else(|| "Pick an application".into());
    let mut picked = None;
    egui::ComboBox::from_id_salt("rec_app").width(200.0).selected_text(label).show_ui(ui, |ui| {
        for a in &state.apps {
            let tex = state.app_icons.entry(a.hwnd).or_insert_with(|| {
                let img = crate::engine::wgc::window_icon(a.hwnd)
                    .map(|f| egui::ColorImage::from_rgba_unmultiplied(f.size, &f.rgba))
                    .unwrap_or_else(|| egui::ColorImage::new([1, 1], vec![egui::Color32::TRANSPARENT]));
                ui.ctx().load_texture(format!("app-icon-{}", a.hwnd), img, egui::TextureOptions::LINEAR)
            });
            let text = if a.exe.is_empty() { a.title.clone() } else { format!("{}  ·  {}", a.title, a.exe) };
            let sel = state.app.as_ref().is_some_and(|x| x.hwnd == a.hwnd);
            let r = ui.horizontal(|ui| {
                ui.image((tex.id(), egui::vec2(16.0, 16.0)));
                ui.selectable_label(sel, text)
            });
            if r.inner.clicked() {
                picked = Some(a.clone());
            }
        }
        if state.apps.is_empty() {
            ui.weak("No applications with a window");
        }
    });
    if let Some(a) = picked {
        state.app = Some(a);
    }
    // a closed app's window is gone: forget it
    if state.app.as_ref().is_some_and(|a| !crate::engine::wgc::window_alive(a.hwnd)) {
        state.app = None;
    }
}

fn quality(ui: &mut egui::Ui, settings: &mut Settings) -> bool {
    let mut save = false;
    egui::Grid::new("rec_quality").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
        ui.label("Frame rate");
        save |= ui.add(egui::DragValue::new(&mut settings.capture_fps).range(1..=120).suffix(" fps")).changed();
        ui.end_row();
        ui.label("Bitrate");
        save |= ui
            .add(egui::DragValue::new(&mut settings.capture_bitrate_kbps).range(0..=200_000).suffix(" kbit/s"))
            .on_hover_text("0 uses the export quality (CRF) instead")
            .changed();
        ui.end_row();
        ui.label("Scale");
        let mut k = settings.capture_scale.clamp(10, 100).to_string();
        let opts = [("100", "100 %"), ("75", "75 %"), ("50", "50 %"), ("25", "25 %")];
        if combo(ui, "rec_scale", &mut k, &opts, Some(120.0)) {
            settings.capture_scale = k.parse().unwrap_or(100);
            save = true;
        }
        ui.end_row();
        ui.label("Encoder");
        let opts = [("x264", "CPU (x264)"), ("gpu", "Graphics card")];
        save |= combo(ui, "rec_encoder", &mut settings.capture_encoder, &opts, Some(160.0));
        ui.end_row();
    });
    if settings.capture_encoder == "gpu" && crate::engine::encoders::best_for("mkv").is_none() {
        ui.weak("No working graphics-card encoder found yet; x264 is used.");
    }
    save
}

fn misc(ui: &mut egui::Ui, state: &mut CaptureUi, settings: &mut Settings, busy: bool) -> bool {
    let mut save = false;
    egui::Grid::new("rec_misc").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
        ui.label("Cursor");
        ui.add_enabled_ui(!busy, |ui| save |= ui.checkbox(&mut settings.capture_cursor, "Show the cursor").on_hover_text("In the recording and the preview").changed());
        ui.end_row();
        ui.label("Hide on Record");
        ui.add_enabled_ui(!busy, |ui| {
            save |= ui
                .checkbox(&mut settings.capture_hide, "Hide Simple Editor while recording")
                .on_hover_text("A small Stop pill stays at the top of the screen; it never shows in the recording")
                .changed()
        });
        ui.end_row();
        ui.label("Stop hotkey");
        ui.add_enabled_ui(!busy, |ui| {
            let ok = crate::engine::rec_pill::parse_hotkey(&settings.capture_stop_hotkey).is_some();
            let te = egui::TextEdit::singleline(&mut settings.capture_stop_hotkey).desired_width(140.0);
            let te = if ok { te } else { te.text_color(ui.visuals().error_fg_color) };
            save |= ui.add(te).on_hover_text("Works in any app while recording, e.g. Ctrl+Shift+F9").changed();
        });
        ui.end_row();
        if let Some(e) = &state.hotkey_error {
            ui.label("");
            ui.colored_label(ui.visuals().error_fg_color, e);
            ui.end_row();
        }
        ui.label("Background");
        save |= ui
            .checkbox(&mut settings.capture_on_blur, "Start recording when the editor loses focus")
            .on_hover_text("Starts when you switch to another app and stops when you come back")
            .changed();
        ui.end_row();
    });
    save
}

fn output(ui: &mut egui::Ui, state: &mut CaptureUi, settings: &mut Settings, c: &PaneCtx, busy: bool, r: &mut CaptureResponse) -> bool {
    let mut save = false;
    egui::Grid::new("rec_output").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
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

        ui.label("Library bin");
        ui.horizontal(|ui| {
            let mut names: Vec<String> = vec![String::new()];
            names.extend(c.bins.iter().cloned());
            if !settings.capture_bin.is_empty() && !names.contains(&settings.capture_bin) {
                names.push(settings.capture_bin.clone()); // made when the first recording lands
            }
            let opts: Vec<(&str, &str)> = names.iter().map(|n| (n.as_str(), if n.is_empty() { "Top level" } else { n.as_str() })).collect();
            save |= combo(ui, "rec_bin", &mut settings.capture_bin, &opts, Some(150.0));
            ui.add(egui::TextEdit::singleline(&mut state.new_bin).hint_text("New bin").desired_width(90.0));
            if ui.add_enabled(!state.new_bin.trim().is_empty(), egui::Button::new("Add")).clicked() {
                let name = state.new_bin.trim().trim_matches('/').to_string();
                settings.capture_bin = name.clone();
                r.new_bin = Some(name);
                state.new_bin.clear();
                save = true;
            }
        });
        ui.end_row();

        ui.label("File name");
        save |= ui
            .add(egui::TextEdit::singleline(&mut settings.capture_name).desired_width(200.0))
            .on_hover_text("{time} = when it was recorded, {source} = the screen, window or app")
            .changed();
        ui.end_row();

        ui.label("Import");
        save |= ui.checkbox(&mut settings.capture_auto_import, "Add to the library when it stops").changed();
        ui.end_row();
    });
    save
}

/// The live preview: the newest frame in one reused texture, letterboxed into the pane's width.
/// Right-click: Pause preview / Show preview (remembered). Returns true when the setting changed.
fn preview(ui: &mut egui::Ui, state: &mut CaptureUi, settings: &mut Settings, c: &PaneCtx, max_h: f32) -> bool {
    let w = ui.available_width().max(160.0);
    let h = (w * 9.0 / 16.0).min(max_h);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::click());
    ui.painter().rect_filled(rect, 4, egui::Color32::BLACK);
    let on = settings.capture_preview;
    if on && state.live {
        if let Some(slot) = &state.slot {
            let gen = slot.generation();
            if state.preview_tex.as_ref().map(|(_, g)| *g) != Some(gen) {
                if let Some((f, _)) = slot.frame() {
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
        Some((t, _)) if on && state.live && state.slot.is_some() => {
            let s = t.size_vec2();
            let k = (rect.width() / s.x).min(rect.height() / s.y);
            let img = egui::Rect::from_center_size(rect.center(), s * k);
            ui.painter().image(t.id(), img, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
        }
        _ => {
            let msg = if !on {
                "Preview paused (right-click to show)"
            } else if state.area == "window" && state.window.is_empty() {
                "Pick a window"
            } else if state.area == "app" && state.app.is_none() {
                "Pick an application"
            } else {
                "Preview"
            };
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, msg, egui::FontId::proportional(14.0), c.palette.border);
        }
    }
    if c.recording {
        let at = rect.left_top() + egui::vec2(10.0, 10.0);
        ui.painter().circle_filled(at + egui::vec2(6.0, 8.0), 6.0, egui::Color32::from_rgb(0xE5, 0x30, 0x30));
        ui.painter().text(at + egui::vec2(18.0, 8.0), egui::Align2::LEFT_CENTER, format!("REC {}", clock(state.elapsed)), egui::FontId::proportional(14.0), egui::Color32::WHITE);
    }
    let mut changed = false;
    crate::ui::menu::context(&resp, |ui| {
        let label = if settings.capture_preview { "Pause preview" } else { "Show preview" };
        if ui.button(label).clicked() {
            settings.capture_preview = !settings.capture_preview;
            changed = true;
            ui.close();
        }
    });
    changed
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
        assert!(o.preview, "the recording feeds the live preview");
        assert_eq!(o.tracks, crate::engine::capture::default_tracks());
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

    /// Application / game source: the picked app becomes a WGC source with its process (for App audio),
    /// and the file is named after it.
    #[test]
    fn app_source_selection() {
        let s = Settings::default();
        let app = AppWindow { hwnd: 0x1234, title: "Roblox Studio".into(), exe: "RobloxStudioBeta.exe".into(), pid: 77 };
        let mut st = CaptureUi { area: "app".into(), ..Default::default() };
        assert_eq!(source_of(&st, &monitors()), CaptureSource::Region(0, 0, 1920, 1080), "nothing picked yet");
        st.app = Some(app);
        let o = new_screen_options(&s, &st, &monitors());
        assert_eq!(o.source, CaptureSource::App { hwnd: 0x1234, title: "Roblox Studio".into(), pid: 77 });
        assert_eq!(o.sources(), (true, true, Some(77)), "All audio (system), Microphone, App audio of pid 77");
        let name = o.out.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("screen-") && name.ends_with(".mp4"), "{name}");
        assert_eq!(file_name("{source} {time}", "Roblox: Studio?", 5), "Roblox_ Studio_ 5");
        assert_eq!(file_name("", "x", 9), "screen-9", "an empty pattern still names the file");
        assert_eq!(file_name("  ...  ", "x", 9), "screen-9");
    }

    fn run(st: &mut CaptureUi, s: &mut Settings, rec: bool) -> (CaptureResponse, bool) {
        let ctx = egui::Context::default();
        let palette = Palette::new(false, egui::Color32::BLUE);
        let mics = vec!["Mic".to_string()];
        let mon = monitors();
        let bins = vec!["Recordings".to_string(), "B-roll".to_string()];
        let mut r = CaptureResponse::default();
        let mut repaint = false;
        for _ in 0..3 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let c = PaneCtx { mics: &mics, monitors: &mon, bins: &bins, recording: rec, palette: &palette };
                    r = screen_pane(ui, st, s, &c)
                });
            });
            repaint = ctx.has_requested_repaint();
        }
        (r, repaint)
    }

    /// Headless (no producers): every source and audio mode lays out with every section open, idle and
    /// recording, asks for nothing on its own and never asks to repaint (the app drives the preview).
    #[test]
    fn assert_no_idle_repaint_screen_recording_pane() {
        let mut s = Settings::default();
        for id in ["rec_sources", "rec_quality", "rec_misc", "rec_output"] {
            s.capture_folds.insert(id.into(), true);
        }
        for area in ["screen:0", "desktop", "window", "region", "app"] {
            for mode in AudioMode::ALL {
                s.capture_audio = mode.key().into();
                for rec in [false, true] {
                    let mut st = CaptureUi { area: area.into(), elapsed: 3.5, windows: vec!["A".into()], ..Default::default() };
                    st.apps = vec![AppWindow { hwnd: 1, title: "Game".into(), exe: "game.exe".into(), pid: 9 }];
                    let (r, repaint) = run(&mut st, &mut s, rec);
                    assert!(r.screen.is_none() && !r.stop_screen && !r.open_folder && r.new_bin.is_none());
                    assert!(!repaint, "{area} / {mode:?} / recording {rec} requested a repaint");
                }
            }
        }
        let mut st = CaptureUi { area: "region".into(), ..Default::default() };
        run(&mut st, &mut s, false);
        assert!(st.region.is_some(), "region mode always has a concrete region");
    }

    /// The fold state is the settings' (it survives a restart): an opened section stays open.
    #[test]
    fn fold_state_persists_in_settings() {
        let mut s = Settings::default();
        assert!(s.capture_folds.is_empty());
        s.capture_folds.insert("rec_quality".into(), true);
        let json = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.capture_folds.get("rec_quality"), Some(&true));
        assert_eq!(back.capture_tracks, crate::engine::capture::default_tracks(), "the track setup is remembered too");
    }

    /// Paused or hidden, the pane shows no frame even with frames waiting.
    #[test]
    fn paused_preview_draws_no_frame() {
        let mut s = Settings::default();
        s.capture_preview = false;
        let slot = FrameSlot::new(true);
        slot.put(crate::engine::screen_grab::Frame { size: [2, 2], rgba: vec![255; 16] });
        let mut st = CaptureUi { live: true, slot: Some(slot), ..Default::default() };
        run(&mut st, &mut s, true);
        assert!(st.preview_tex.is_none(), "paused: nothing uploaded");
        s.capture_preview = true;
        run(&mut st, &mut s, true);
        assert!(st.preview_tex.is_some(), "showing again: the frame is uploaded");
    }

    #[test]
    fn track_levels_follow_their_sources() {
        let l = [Some(0.5), Some(0.2), Some(0.9)];
        assert_eq!(track_level(TrackKind::Mix, AudioMode::Both, l), 0.5);
        assert_eq!(track_level(TrackKind::Mix, AudioMode::Mic, l), 0.2);
        assert_eq!(track_level(TrackKind::Mix, AudioMode::None, l), 0.0);
        assert_eq!(track_level(TrackKind::App, AudioMode::None, l), 0.9);
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
