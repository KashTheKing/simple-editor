//! Settings window (egui::Window, closable). Tabs:
//!  * General: theme (system/dark/light), decoder (auto/mf/ffmpeg), ffmpeg folder (text + Browse…, shows
//!    whether ffmpeg/ffprobe were found), preview max width, snapping, confirm overwrite,
//!    "Edit with Simple Editor" context menu checkbox (install/uninstall via crate::contextmenu).
//!  * Performance: GPU preview toggle + the detected OpenGL renderer, preview render quality (%),
//!    movie mode (pre-rendered playback) and the stock image the effect thumbnails are rendered from.
//!  * Capture: screen recording (fps, bitrate, microphone, desktop audio, cursor, record-on-blur, output
//!    folder) and voiceover (input device, channels). Device lists come from `engine::capture`.
//!  * Export: encoder combo (auto + detected encoders filtered to h264/hevc/vp9/av1 families), CRF, preset.
//!  * Hotkeys: table of every Action (label, current binding, "Rebind" → waits for the next key press with
//!    modifiers (Escape cancels, Backspace/Delete unbinds), "Reset"), plus "Reset all". Conflicts are
//!    resolved by unbinding the other action (Hotkeys::set). Note the fixed mouse modifiers.
//! Returns true when settings changed (caller saves + re-applies theme/backend/hotkeys).

use crate::hotkeys::{Action, Hotkeys};
use crate::settings::Settings;
use crate::theme::Palette;
use crate::ui::tools::Glyph;
use crate::ui::{combo, encoder_options, ENCODER_PRESETS};
use eframe::egui::{self, Event, Key, KeyboardShortcut, Modifiers};

#[derive(Default)]
pub struct SettingsUi {
    pub open: bool,
    pub tab: usize,
    pub rebinding: Option<Action>,
    /// Cached ffmpeg / context-menu status (filesystem + registry lookups are not per-frame).
    status: Option<Status>,
    /// Last rebind note ("Unbound X") shown under the hotkey table.
    note: String,
    /// MCP port while it is being dragged / typed; committed to the settings when the gesture ends
    /// (the app restarts the server on every value it sees, and a busy one in between switches it off).
    port_edit: Option<u16>,
}

struct Status {
    /// Inputs the status was computed from; recomputed at the start of a frame when they differ
    /// (the app applies ffmpeg_dir / context_menu changes after `show` returns).
    ffmpeg_dir: String,
    ytdlp_dir: String,
    context_menu: bool,
    ffmpeg: String,
    ytdlp: String,
    ctxmenu: &'static str,
}

impl Status {
    fn compute(s: &Settings) -> Self {
        let exe = |p: Option<std::path::PathBuf>| {
            p.map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| "not found".into())
        };
        Self {
            ffmpeg_dir: s.ffmpeg_dir.clone(),
            ytdlp_dir: s.ytdlp_dir.clone(),
            context_menu: s.context_menu,
            ffmpeg: format!(
                "ffmpeg: {}\nffprobe: {}",
                exe(crate::media::ffpipe::ffmpeg_exe()),
                exe(crate::media::ffpipe::ffprobe_exe())
            ),
            ytdlp: match crate::media::ytdlp::exe() {
                Some(p) => format!("yt-dlp: {}", p.display()),
                None => "yt-dlp: not found — the Library's \"Import URL…\" button is hidden".into(),
            },
            ctxmenu: if crate::contextmenu::is_installed() { "(installed)" } else { "(not installed)" },
        }
    }
}

const THEMES: [(&str, &str); 3] = [("system", "System"), ("dark", "Dark"), ("light", "Light")];
const DECODERS: [(&str, &str); 3] =
    [("auto", "Auto (Media Foundation, ffmpeg fallback)"), ("mf", "Media Foundation"), ("ffmpeg", "ffmpeg")];
#[allow(clippy::too_many_arguments)]
pub fn show(
    ctx: &egui::Context,
    state: &mut SettingsUi,
    settings: &mut Settings,
    hotkeys: &mut Hotkeys,
    encoders: &[String],
    _palette: &Palette,
    mcp_status: &str,
    // OpenGL renderer string ("no OpenGL context" when eframe runs without one)
    gpu_name: &str,
    // dshow audio inputs (`engine::capture::audio_devices`); the bool marks loopback/desktop devices
    audio_inputs: &[(String, bool)],
) -> bool {
    let mut changed = false;
    let stale = state.status.as_ref().is_none_or(|s| {
        s.ffmpeg_dir != settings.ffmpeg_dir
            || s.ytdlp_dir != settings.ytdlp_dir
            || s.context_menu != settings.context_menu
    });
    if stale {
        state.status = Some(Status::compute(settings));
    }
    let mut open = state.open;
    egui::Window::new("Settings")
        .open(&mut open)
        // "when I open the settings tab, it should be really big" — sized to most of the screen,
        // still resizable/movable like any other non-blocking window
        .default_size([900.0, 700.0])
        .collapsible(false)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut state.tab, 0, "General");
                ui.selectable_value(&mut state.tab, 1, "Performance");
                ui.selectable_value(&mut state.tab, 2, "Capture");
                ui.selectable_value(&mut state.tab, 3, "Export");
                ui.selectable_value(&mut state.tab, 4, "Hotkeys");
                ui.selectable_value(&mut state.tab, 5, "Appearance");
            });
            ui.separator();
            changed = match state.tab {
                0 => general(ui, state, settings, mcp_status),
                1 => performance(ui, settings, gpu_name),
                2 => capture_tab(ui, settings, audio_inputs),
                3 => export(ui, settings, encoders),
                4 => hotkeys_tab(ui, state, hotkeys),
                _ => appearance(ui, settings),
            };
        });
    state.open = open;
    if !open {
        state.rebinding = None;
        state.status = None;
        state.note.clear();
        state.port_edit = None;
    }
    changed
}

fn general(ui: &mut egui::Ui, state: &mut SettingsUi, s: &mut Settings, mcp_status: &str) -> bool {
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
fn port_field(ui: &mut egui::Ui, state: &mut SettingsUi, s: &mut Settings) -> bool {
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

fn performance(ui: &mut egui::Ui, s: &mut Settings, gpu_name: &str) -> bool {
    let mut changed = false;
    egui::Grid::new("perf").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
        ui.label("GPU preview");
        ui.vertical(|ui| {
            changed |= ui.checkbox(&mut s.gpu, "Render the preview with OpenGL shaders").changed();
            ui.weak(format!("Renderer: {gpu_name}"));
            ui.weak("Off (or when the driver refuses the shaders) the CPU compositor is used.");
        });
        ui.end_row();

        ui.label("Preview quality");
        ui.horizontal(|ui| {
            let mut q = s.preview_quality.clamp(25, 100);
            changed |= ui.add(egui::DragValue::new(&mut q).range(25..=100).suffix(" %").speed(1)).changed();
            s.preview_quality = q;
            ui.weak("of the preview size — lower is faster, the export is unaffected");
        });
        ui.end_row();

        ui.label("Proxy media");
        ui.vertical(|ui| {
            changed |= ui.checkbox(&mut s.use_proxies, "Play low-res all-intra proxies in the preview").changed();
            ui.horizontal(|ui| {
                let mut h = s.proxy_height.clamp(120, 2160);
                changed |= ui.add(egui::DragValue::new(&mut h).range(120..=2160).suffix(" px").speed(10)).changed();
                s.proxy_height = h;
                ui.weak("proxy height — built in the background, exports always use the originals");
            });
        });
        ui.end_row();

        ui.label("Movie mode");
        ui.vertical(|ui| {
            changed |= ui.checkbox(&mut s.movie_mode, "Play back pre-rendered frames").changed();
            ui.weak("Renders the in/out range (or the whole timeline) at full quality in the background.");
        });
        ui.end_row();

        ui.label("Effect thumbnails");
        ui.horizontal(|ui| {
            changed |= ui
                .add(
                    egui::TextEdit::singleline(&mut s.effect_thumb_image)
                        .desired_width(200.0)
                        .hint_text("built-in image"),
                )
                .changed();
            if ui.button("Browse…").clicked() {
                if let Some(p) =
                    rfd::FileDialog::new().add_filter("Images", &["png", "jpg", "jpeg", "bmp", "webp"]).pick_file()
                {
                    s.effect_thumb_image = p.to_string_lossy().into_owned();
                    changed = true;
                }
            }
            if ui
                .add_enabled(!s.effect_thumb_image.is_empty(), egui::Button::new("Reset to default"))
                .on_hover_text("Render the catalogue over the image built into the app")
                .clicked()
            {
                s.effect_thumb_image.clear();
                changed = true;
            }
        });
        ui.end_row();
    });
    changed
}

fn capture_tab(ui: &mut egui::Ui, s: &mut Settings, audio_inputs: &[(String, bool)]) -> bool {
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
    changed |= ui.checkbox(&mut s.capture_desktop_audio, "Record desktop audio (needs a loopback device)").changed();
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

fn export(ui: &mut egui::Ui, s: &mut Settings, encoders: &[String]) -> bool {
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
        let opts: Vec<(&str, &str)> = ENCODER_PRESETS.iter().map(|p| (*p, *p)).collect();
        changed |= combo(ui, "preset", &mut s.preset, &opts, Some(260.0));
        ui.end_row();
    });
    changed
}

fn hotkeys_tab(ui: &mut egui::Ui, state: &mut SettingsUi, hotkeys: &mut Hotkeys) -> bool {
    let mut changed = false;
    if let Some(a) = state.rebinding {
        changed |= capture(ui.ctx(), state, a, hotkeys);
    }
    if ui.button("Reset all").clicked() {
        hotkeys.reset_all();
        state.rebinding = None;
        state.note.clear();
        changed = true;
    }
    ui.add_space(4.0);
    egui::ScrollArea::vertical().max_height((ui.available_height() - 40.0).max(0.0)).auto_shrink([false, true]).show(
        ui,
        |ui| {
            egui::Grid::new("hotkeys").num_columns(4).striped(true).spacing([12.0, 4.0]).show(ui, |ui| {
                for &a in Action::ALL {
                    ui.label(a.label());
                    let text = hotkeys.text(a);
                    if state.rebinding == Some(a) {
                        ui.strong("…");
                    } else if text.is_empty() {
                        ui.weak("—");
                    } else {
                        ui.label(text);
                    }
                    if ui.small_button("Rebind").clicked() {
                        state.rebinding = Some(a);
                        state.note.clear();
                    }
                    if ui.small_button("Reset").clicked() {
                        let before = hotkeys.get(a);
                        hotkeys.reset(a);
                        changed |= hotkeys.get(a) != before;
                    }
                    ui.end_row();
                }
            });
        },
    );
    ui.add_space(4.0);
    if let Some(a) = state.rebinding {
        let r = ui.strong(format!("Press keys for \"{}\"… (Esc cancels, Backspace/Delete unbinds)", a.label()));
        // Hold keyboard focus so the app's hotkey polling (which skips while a widget has focus) stays quiet.
        r.request_focus();
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                r.id,
                egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true },
            )
        });
    } else if !state.note.is_empty() {
        ui.label(&state.note);
    }
    ui.weak("Mouse: Ctrl+Scroll zoom, Shift+Scroll pan, Alt+Scroll track height (fixed).");
    changed
}

/// While rebinding: take the first key press this frame, bind/unbind/cancel, and swallow all key/text
/// events so nothing else reacts to them. Returns true if a binding changed.
fn capture(ctx: &egui::Context, state: &mut SettingsUi, a: Action, hotkeys: &mut Hotkeys) -> bool {
    let mut changed = false;
    ctx.input_mut(|i| {
        let pressed = i.events.iter().find_map(|e| match e {
            Event::Key { key, pressed: true, modifiers, .. } => Some((*key, *modifiers)),
            _ => None,
        });
        if let Some((key, m)) = pressed {
            state.rebinding = None;
            match key {
                Key::Escape => {}
                Key::Backspace | Key::Delete => {
                    changed = hotkeys.get(a).is_some();
                    hotkeys.set(a, None);
                }
                key => {
                    let ks = KeyboardShortcut::new(
                        Modifiers {
                            alt: m.alt,
                            ctrl: m.ctrl || m.command,
                            shift: m.shift,
                            mac_cmd: false,
                            command: false,
                        },
                        key,
                    );
                    if let Some(other) = hotkeys.conflict(ks).filter(|&o| o != a) {
                        state.note =
                            format!("{} was using {} and is now unbound.", other.label(), Hotkeys::format(&ks));
                    }
                    hotkeys.set(a, Some(ks));
                    changed = true;
                }
            }
        }
        i.events.retain(|e| !matches!(e, Event::Key { .. } | Event::Text(_)));
    });
    changed
}

/// Appearance: mode (Follow Windows / Light / Dark / Custom) + a colour override per `Palette` field
/// the app actually paints with. Mutates `s.palette` directly, so `theme::palette_with` (recomputed
/// every frame in `App::update`) picks it up immediately — no restart, no extra plumbing.
fn appearance(ui: &mut egui::Ui, s: &mut Settings) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label("Look");
        changed |= combo(ui, "ui_look", &mut s.ui_look, &[("cozy", "Cozy (rounded)"), ("sharp", "Sharp")], Some(200.0));
    });
    ui.add_space(4.0);
    let has_overrides =
        s.palette != crate::theme::PaletteOverride { mode: s.palette.mode.clone(), ..Default::default() };
    ui.horizontal(|ui| {
        ui.label("Theme");
        // one list: the three base modes, every built-in preset, and "Custom" once colours are tweaked
        let cur = if !s.palette.preset.is_empty() {
            s.palette.preset.clone()
        } else if has_overrides {
            "Custom".to_string()
        } else {
            match s.theme.as_str() {
                "dark" => "Dark",
                "light" => "Light",
                _ => "Follow System",
            }
            .to_string()
        };
        egui::ComboBox::from_id_salt("theme_preset").selected_text(cur.clone()).width(200.0).show_ui(ui, |ui| {
            for (label, theme) in [("Follow System", "system"), ("Dark", "dark"), ("Light", "light")] {
                if ui.selectable_label(cur == label, label).clicked() {
                    s.theme = theme.into();
                    s.palette = crate::theme::PaletteOverride {
                        mode: if theme == "system" { String::new() } else { theme.into() },
                        ..Default::default()
                    };
                    changed = true;
                }
            }
            ui.separator();
            for name in crate::theme::PRESETS {
                if ui.selectable_label(s.palette.preset == *name, *name).clicked() {
                    if let Some(ov) = crate::theme::preset(name) {
                        s.theme = ov.mode.clone();
                        s.palette = ov;
                        changed = true;
                    }
                }
            }
        });
        if ui.button("Export theme…").clicked() {
            let tf = crate::settings::ThemeFile {
                name: if s.palette.preset.is_empty() { "My theme".into() } else { s.palette.preset.clone() },
                theme: s.theme.clone(),
                ui_look: s.ui_look.clone(),
                palette: s.palette.clone(),
                bg_image: s.bg_image.clone(),
                bg_tint: s.bg_tint,
                bg_blur: s.bg_blur,
                panel_opacity: s.panel_opacity,
            };
            if let Some(out) = rfd::FileDialog::new()
                .add_filter("Simple Editor theme", &["sedit-theme"])
                .set_file_name("theme.sedit-theme")
                .save_file()
            {
                let _ = std::fs::write(&out, serde_json::to_string_pretty(&tf).unwrap_or_default());
            }
        }
        if ui.button("Import theme…").clicked() {
            if let Some(p) =
                rfd::FileDialog::new().add_filter("Simple Editor theme", &["sedit-theme", "json"]).pick_file()
            {
                if let Some(tf) = std::fs::read_to_string(&p)
                    .ok()
                    .and_then(|t| serde_json::from_str::<crate::settings::ThemeFile>(&t).ok())
                {
                    s.theme = tf.theme;
                    s.ui_look = tf.ui_look;
                    s.palette = tf.palette;
                    s.bg_image = tf.bg_image;
                    s.bg_tint = tf.bg_tint;
                    s.bg_blur = tf.bg_blur;
                    s.panel_opacity = tf.panel_opacity;
                    s.save(); // belt and braces: persist right away, whatever happens to this frame's flow
                    changed = true;
                }
            }
        }
    });
    ui.weak("Tweak any colour below to start a Custom theme from the one selected above.");
    ui.add_space(6.0);

    let base = crate::theme::palette_with(ui.ctx(), &s.palette);
    ui.horizontal(|ui| {
        ui.label("Preview");
        for c in [
            base.bg,
            base.panel,
            base.header,
            base.border,
            base.text,
            base.text_dim,
            base.accent,
            base.selection,
            base.keyframe,
            base.waveform,
        ] {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 2.0, c);
            ui.painter().rect_stroke(rect, 2.0, (1.0, base.border), egui::StrokeKind::Outside);
        }
    });
    ui.add_space(6.0);

    let mut tweaked = false;
    egui::Grid::new("appearance").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
        tweaked |= color_row(ui, "Background", &mut s.palette.background, base.bg);
        tweaked |= color_row(ui, "Panel", &mut s.palette.panel, base.panel);
        tweaked |= color_row(ui, "Header", &mut s.palette.header, base.header);
        tweaked |= color_row(ui, "Border", &mut s.palette.border, base.border);
        tweaked |= color_row(ui, "Text", &mut s.palette.text, base.text);
        tweaked |= color_row(ui, "Text (dim)", &mut s.palette.text_dim, base.text_dim);
        tweaked |= color_row(ui, "Accent", &mut s.palette.accent, base.accent);
        tweaked |= color_row(ui, "Selection", &mut s.palette.selection, base.selection);
        tweaked |= color_row(ui, "Keyframe", &mut s.palette.keyframe, base.keyframe);
        tweaked |= color_row(ui, "Waveform", &mut s.palette.waveform, base.waveform);
    });
    ui.add_space(4.0);
    ui.collapsing("Clip colours", |ui| {
        egui::Grid::new("clip_colours").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            tweaked |= color_row(ui, "Video clips", &mut s.palette.clip_video, base.clip_video);
            tweaked |= color_row(ui, "Audio clips", &mut s.palette.clip_audio, base.clip_audio);
            tweaked |= color_row(ui, "Image clips", &mut s.palette.clip_image, base.clip_image);
            tweaked |= color_row(ui, "Text clips", &mut s.palette.clip_text, base.clip_text);
            tweaked |= color_row(ui, "Sequence clips", &mut s.palette.clip_sequence, base.clip_sequence);
            tweaked |= color_row(ui, "Shape clips", &mut s.palette.clip_shape, base.clip_shape);
            tweaked |= color_row(ui, "Adjustment clips", &mut s.palette.clip_adjust, base.clip_adjust);
        });
        ui.weak("A clip's custom label colour (right-click a clip) still wins over these.");
    });
    if tweaked {
        // editing a colour forks the current theme into "Custom" — the colours stay as a starting point
        s.palette.preset = String::new();
        if s.palette.mode.is_empty() || s.palette.mode == "system" {
            s.palette.mode = "custom".into(); // keep following Windows dark/light underneath
        }
        changed = true;
    }
    ui.add_space(8.0);
    ui.collapsing("Background image", |ui| {
        ui.weak("A picture behind the whole editor; lower the panel opacity to see it through the panes.");
        ui.horizontal(|ui| {
            ui.label("Image");
            let name = if s.bg_image.is_empty() {
                "(none)".to_string()
            } else {
                std::path::Path::new(&s.bg_image)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| s.bg_image.clone())
            };
            ui.label(name).on_hover_text(&s.bg_image);
            if ui.button("Browse…").clicked() {
                if let Some(p) = rfd::FileDialog::new()
                    .add_filter("Images", &["png", "jpg", "jpeg", "bmp", "webp", "gif"])
                    .pick_file()
                {
                    s.bg_image = p.to_string_lossy().into_owned();
                    changed = true;
                }
            }
            if !s.bg_image.is_empty() && ui.button("Clear").clicked() {
                s.bg_image.clear();
                changed = true;
            }
        });
        ui.add_enabled_ui(!s.bg_image.is_empty(), |ui| {
            ui.horizontal(|ui| {
                ui.label("Tint");
                let mut c =
                    egui::Color32::from_rgba_unmultiplied(s.bg_tint[0], s.bg_tint[1], s.bg_tint[2], s.bg_tint[3]);
                if egui::color_picker::color_edit_button_srgba(ui, &mut c, egui::color_picker::Alpha::OnlyBlend)
                    .changed()
                {
                    let [r, g, b, a] = c.to_srgba_unmultiplied();
                    s.bg_tint = [r, g, b, a];
                    changed = true;
                }
                ui.label("Blur");
                changed |= ui.add(egui::DragValue::new(&mut s.bg_blur).range(0..=20)).changed();
            });
            ui.horizontal(|ui| {
                ui.label("Panel opacity");
                changed |= ui.add(egui::Slider::new(&mut s.panel_opacity, 60..=255)).changed();
            });
        });
    });
    ui.add_space(8.0);
    ui.collapsing("Icons", |ui| {
        ui.weak("Pick the glyph shown for each pane and menu action ('None' removes it, 'Default' restores).");
        egui::ScrollArea::vertical().max_height((ui.available_height() - 40.0).max(0.0)).show(ui, |ui| {
            egui::Grid::new("icons_panes").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
                for p in crate::ui::layout::Pane::ALL {
                    changed |= icon_row(ui, s, format!("pane.{}", p.title()), p.title(), Some(p.glyph()));
                }
                for a in crate::hotkeys::Action::ALL {
                    changed |=
                        icon_row(ui, s, format!("action.{}", a.id()), a.label(), crate::ui::tools::action_glyph(*a));
                }
            });
        });
    });
    changed
}

/// One icon assignment: current glyph (or blank), and a picker with every glyph, None and Default.
fn icon_row(ui: &mut egui::Ui, s: &mut Settings, key: String, label: &str, default: Option<Glyph>) -> bool {
    use crate::ui::tools::{glyph_label, glyph_text_button};
    let mut changed = false;
    ui.label(label);
    let cur = match s.icon_overrides.get(&key) {
        Some(n) if n == "none" => None,
        Some(n) => Glyph::from_name(n).or(default),
        None => default,
    };
    ui.horizontal(|ui| {
        match cur {
            Some(g) => {
                glyph_label(ui, g, ui.visuals().text_color());
            }
            None => ui.add_space(18.0),
        }
        ui.menu_button("Change", |ui| {
            ui.set_max_width(260.0);
            ui.horizontal_wrapped(|ui| {
                for g in Glyph::ALL {
                    if glyph_text_button(ui, *g, "").on_hover_text(g.name()).clicked() {
                        s.icon_overrides.insert(key.clone(), g.name().to_string());
                        changed = true;
                        ui.close();
                    }
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("None").clicked() {
                    s.icon_overrides.insert(key.clone(), "none".into());
                    changed = true;
                    ui.close();
                }
                if ui.button("Default").clicked() {
                    s.icon_overrides.remove(&key);
                    changed = true;
                    ui.close();
                }
            });
        });
    });
    ui.end_row();
    changed
}

/// One overridable colour: checkbox turns the override on/off, the button edits it while on (shown
/// at `fallback` — today's derived colour — while off, so turning it on starts from what's in effect).
fn color_row(ui: &mut egui::Ui, label: &str, ov: &mut Option<[u8; 3]>, fallback: egui::Color32) -> bool {
    let mut changed = false;
    ui.label(label);
    let mut on = ov.is_some();
    let mut rgb = ov.unwrap_or([fallback.r(), fallback.g(), fallback.b()]);
    ui.horizontal(|ui| {
        changed |= ui.checkbox(&mut on, "").changed();
        ui.add_enabled_ui(on, |ui| {
            changed |= egui::color_picker::color_edit_button_srgb(ui, &mut rgb).changed();
        });
    });
    ui.end_row();
    *ov = on.then_some(rgb);
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoder_filter() {
        let all: Vec<String> =
            ["libx264", "h264_nvenc", "hevc_qsv", "libvpx-vp9", "libaom-av1", "h264_amf", "mpeg4", "gif", "aac"]
                .iter()
                .map(|s| s.to_string())
                .collect();
        assert_eq!(
            encoder_options(&all),
            vec!["libx264", "h264_nvenc", "hevc_qsv", "libvpx-vp9", "libaom-av1", "h264_amf"]
        );
        assert!(encoder_options(&[]).is_empty());
    }

    #[test]
    fn capture_binds_and_swallows() {
        let ctx = egui::Context::default();
        let mut hk = Hotkeys::defaults();
        let mut st = SettingsUi { rebinding: Some(Action::Split), ..Default::default() };
        let mut input = egui::RawInput::default();
        input.events.push(Event::Key {
            key: Key::B,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::CTRL | Modifiers::SHIFT,
        });
        input.events.push(Event::Text("B".into()));
        ctx.begin_pass(input);
        // Ctrl+Shift+B is free: binds Split, swallows the events, stops rebinding.
        assert!(capture(&ctx, &mut st, Action::Split, &mut hk));
        assert_eq!(hk.text(Action::Split), "Ctrl+Shift+B");
        assert!(st.rebinding.is_none());
        assert!(ctx.input(|i| i.events.is_empty()));
        let _ = ctx.end_pass();

        // Ctrl+Z conflicts with Undo: Undo gets unbound and a note is written.
        let mut input = egui::RawInput::default();
        input.events.push(Event::Key {
            key: Key::Z,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::COMMAND,
        });
        ctx.begin_pass(input);
        st.rebinding = Some(Action::Split);
        assert!(capture(&ctx, &mut st, Action::Split, &mut hk));
        assert_eq!(hk.text(Action::Split), "Ctrl+Z");
        assert_eq!(hk.text(Action::Undo), "");
        assert!(st.note.contains("Undo"));
        let _ = ctx.end_pass();

        // Escape cancels without changes; Delete unbinds.
        for (key, expect_changed, expect_text) in [(Key::Escape, false, "Ctrl+Z"), (Key::Delete, true, "")] {
            let mut input = egui::RawInput::default();
            input.events.push(Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            });
            ctx.begin_pass(input);
            st.rebinding = Some(Action::Split);
            assert_eq!(capture(&ctx, &mut st, Action::Split, &mut hk), expect_changed);
            assert_eq!(hk.text(Action::Split), expect_text);
            assert!(st.rebinding.is_none());
            let _ = ctx.end_pass();
        }
    }

    /// Headless: every tab lays out without panicking and reports no change without input.
    #[test]
    fn show_headless_no_change() {
        let ctx = egui::Context::default();
        let mut settings = Settings::default();
        let mut hk = Hotkeys::defaults();
        let encoders = vec!["libx264".to_string(), "aac".to_string()];
        let palette = Palette::new(false, egui::Color32::BLACK);
        let mut st = SettingsUi { open: true, ..Default::default() };
        let inputs = vec![("Microphone (USB)".to_string(), false), ("Stereo Mix".to_string(), true)];
        for tab in [0, 1, 2, 3, 4, 5] {
            st.tab = tab;
            for _ in 0..2 {
                let _ = ctx.run(egui::RawInput::default(), |ctx| {
                    assert!(!show(
                        ctx,
                        &mut st,
                        &mut settings,
                        &mut hk,
                        &encoders,
                        &palette,
                        "stopped",
                        "Test GL",
                        &inputs
                    ));
                });
            }
            assert!(st.open && st.status.is_some());
        }
        // the new tabs left the settings alone
        assert_eq!(settings.preview_quality, Settings::default().preview_quality);
        assert_eq!(settings.capture_fps, Settings::default().capture_fps);
        assert_eq!(settings.palette, crate::theme::PaletteOverride::default());
    }

    /// The Appearance tab's per-colour toggle: off stays unset, and an already-on value round-trips
    /// through a redraw unchanged (no spurious `changed` from just laying the row out).
    #[test]
    fn color_row_toggle_persists() {
        let ctx = egui::Context::default();
        let mut ov: Option<[u8; 3]> = None;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                assert!(!color_row(ui, "Test", &mut ov, egui::Color32::from_rgb(1, 2, 3)));
            });
        });
        assert!(ov.is_none(), "left alone: stays unset");

        ov = Some([1, 2, 3]);
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                assert!(!color_row(ui, "Test", &mut ov, egui::Color32::from_rgb(9, 9, 9)));
            });
        });
        assert_eq!(ov, Some([1, 2, 3]));
    }
}
