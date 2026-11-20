//! The General tab: decoder/ffmpeg-and-yt-dlp folders/downloads/preview width/snapping/confirm
//! overwrite/context-menu, imported fonts, and the MCP server section. Extracted verbatim from
//! settings_ui.rs (see mod.rs's module doc for the whole settings window).

use super::SettingsUi;
use crate::settings::Settings;
use crate::ui::combo;
use eframe::egui;

const DECODERS: [(&str, &str); 3] =
    [("auto", "Auto (Media Foundation, ffmpeg fallback)"), ("mf", "Media Foundation"), ("ffmpeg", "ffmpeg")];

pub(super) fn general(ui: &mut egui::Ui, state: &mut SettingsUi, s: &mut Settings, mcp_status: &str) -> bool {
    let mut changed = false;
    egui::Grid::new("general").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
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

        // ---- ws:ai-enhancer ----
        ui.label("AI enhancer");
        ui.horizontal(enhancer_row);
        ui.end_row();

        // ---- ws:addon-deepfilternet ----
        ui.label("Speech denoise");
        ui.horizontal(denoise_row);
        ui.end_row();

        // ---- ws:addon-faster-whisper ----
        ui.label("Transcription");
        changed |= transcription_row(ui, s);
        ui.end_row();

        // ---- ws:addon-demucs ----
        ui.label("Stem Separation");
        ui.horizontal(demucs_row);
        ui.end_row();

        // ---- ws:addon-sam2 ----
        ui.label("Object masks");
        ui.horizontal(object_masks_row);
        ui.end_row();

        // ---- ws:addon-bg-removal ----
        ui.label("Background removal");
        ui.horizontal(bgremove_row);
        ui.end_row();

        // ---- ws:addon-voice ----
        ui.label("Kokoro voices");
        ui.horizontal(|ui| crate::ui::app::speech_addon_row(ui, crate::engine::tts_addon::Addon::Kokoro));
        ui.end_row();
        ui.label("Chatterbox voices");
        ui.horizontal(|ui| crate::ui::app::speech_addon_row(ui, crate::engine::tts_addon::Addon::Chatterbox));
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

        // ---- ws:forgiveness ----
        ui.label("Autosave interval");
        ui.horizontal(|ui| {
            changed |= ui.add(egui::DragValue::new(&mut s.autosave_secs).range(0..=3600).speed(1)).changed();
            ui.weak(if s.autosave_secs == 0 { "off" } else { "s" });
        });
        ui.end_row();

        // ---- ws:cache-budget ----
        ui.label("Cache disk budget");
        ui.horizontal(|ui| {
            changed |= ui.add(egui::DragValue::new(&mut s.cache_disk_gb).range(0..=2000).speed(1)).changed();
            ui.weak(if s.cache_disk_gb == 0 { "unlimited" } else { "GB (HTML, pre-render, proxies, peaks; oldest first)" });
        });
        ui.end_row();

        // ---- ws:neutral-web ----
        ui.label("Cache folder");
        cache_folder_row(ui, state, s);
        ui.end_row();

        ui.label("Web browser");
        changed |= browser_row(ui, state, s);
        ui.end_row();
    });
    ui.add_space(6.0);
    // ---- ws:pages ---- (was the Dynamic / Granular layout mode; same field)
    let mut follow = s.layout_mode != "granular";
    let r = ui.checkbox(&mut follow, "Switch panel tabs to follow the selection").on_hover_text(
        "Selecting a clip brings the tab that edits it to the front. Off: tabs stay put and only glow. \
         Right-click a tab ▸ Stay on this tab to keep one in front either way.",
    );
    if r.changed() {
        s.layout_mode = if follow { "dynamic" } else { "granular" }.into();
        changed = true;
    }
    // ---- ws:panels-multiscreen ----
    ui.horizontal(|ui| {
        ui.label("Layout");
        let n = s.screen_mode.max(1);
        ui.weak(if n == 1 { "one screen".to_string() } else { format!("{n} screens") });
        if ui.button("Screens…").on_hover_text("One window, or the pages spread over 2 or 3 monitors").clicked() {
            state.open_screens = true;
        }
    });
    // ---- ws:layout-modes-onboarding ----
    // ---- ws:home-page ----
    changed |= ui
        .checkbox(&mut s.home_page, "Home page")
        .on_hover_text("Open on a page of recent projects and New cards; File ▸ New and File ▸ Quit to Project List go there too. Off: the start screen card below")
        .changed();
    changed |= ui
        .add_enabled(!s.home_page, egui::Checkbox::new(&mut s.home_screen, "Start screen"))
        .on_hover_text("The Open / Import / formats / recent projects card over an empty project")
        .on_disabled_hover_text("Used when the Home page is off")
        .changed();
    changed |= ui.checkbox(&mut s.snap, "Snapping in the timeline").changed();
    let mut detailed = s.timeline_waveform == "detailed";
    if ui
        .checkbox(&mut detailed, "Detailed waveforms in the timeline")
        .on_hover_text(
            "Colour each moment of audio by its bass / mids / highs, mark the hits, and show a spectrogram on tall              tracks. Off: a simple loudness waveform. A track's right-click menu can override this.",
        )
        .changed()
    {
        s.timeline_waveform = if detailed { "detailed" } else { "simple" }.into();
        changed = true;
    }
    changed |= ui.checkbox(&mut s.auto_close_gaps, "Auto close gaps (magnetic timeline)").changed();
    changed |= ui
        .checkbox(&mut s.match_import, "Ask to match the project to imported videos")
        .on_hover_text("When a video's size, frame rate or HDR differs from the project's")
        .changed();
    changed |= ui.checkbox(&mut s.confirm_overwrite, "Confirm before overwriting files").changed();
    let save = match crate::ui::menu::shortcut(crate::hotkeys::Action::Save) {
        k if k.is_empty() => "Save".to_string(),
        k => format!("Save ({k})"),
    };
    let lossless =
        format!("{save} uses the instant lossless cut when the project is a plain cut (cuts snap to keyframes)");
    changed |= ui.checkbox(&mut s.lossless_save, lossless).changed();
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
/// held in `state.port_edit` while the user drags or types and only written when the gesture ends -
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

// ---- ws:ai-enhancer ----
/// Install / uninstall the optional AI enhancer add-on (Real-ESRGAN, used by clip ▸ AI Upscale).
fn enhancer_row(ui: &mut egui::Ui) {
    use crate::engine::enhance;
    if let Some(p) = enhance::installing() {
        ui.add(egui::Spinner::new());
        ui.weak(format!("{:.0}% · {}", p.fraction() * 100.0, p.status()));
    } else if enhance::available() {
        ui.weak(match enhance::version() {
            Some(v) => format!("installed ({v})"),
            None => "installed".into(),
        });
        let id = egui::Id::new("enhancer_uninstall_error");
        if ui.button("Uninstall").on_hover_text("Removes the add-on; files it already upscaled stay").clicked() {
            let r = enhance::uninstall().err();
            ui.data_mut(|d| d.insert_temp(id, r));
        }
        if let Some(e) = ui.data(|d| d.get_temp::<Option<String>>(id)).flatten() {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
    } else {
        if ui
            .button("Install")
            .on_hover_text("Real-ESRGAN AI upscaling on the graphics card: right-click a clip ▸ AI Upscale. About 45 MB, downloaded once.")
            .clicked()
        {
            enhance::start_install();
        }
        match enhance::install_error() {
            Some(e) => ui.colored_label(ui.visuals().error_fg_color, format!("install failed: {e}")),
            None => ui.weak("optional, about 45 MB"),
        };
    }
}

// ---- ws:addon-deepfilternet ----
/// Install / remove the optional speech-denoise add-on (DeepFilterNet, used by Clean Audio and Denoise).
fn denoise_row(ui: &mut egui::Ui) {
    use crate::engine::deepfilter;
    if let Some(p) = deepfilter::installing() {
        ui.add(egui::Spinner::new());
        ui.weak(format!("{:.0}% · {}", p.fraction() * 100.0, p.status()));
    } else if deepfilter::available() {
        ui.weak(match deepfilter::version() {
            Some(v) => format!("installed ({v})"),
            None => "installed".into(),
        });
        let id = egui::Id::new("denoise_uninstall_error");
        if ui.button("Remove").on_hover_text("Removes the add-on; Clean Audio then uses ffmpeg afftdn").clicked() {
            let r = deepfilter::uninstall().err();
            ui.data_mut(|d| d.insert_temp(id, r));
        }
        if let Some(e) = ui.data(|d| d.get_temp::<Option<String>>(id)).flatten() {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
    } else {
        if ui
            .button("Install")
            .on_hover_text(
                "Optional. DeepFilterNet removes fans, traffic and room noise from speech: Inspector ▸ Audio ▸ \
                 Clean Audio, and the Denoise bake. Without it they use ffmpeg afftdn. About 27 MB, downloaded once.",
            )
            .clicked()
        {
            deepfilter::start_install();
        }
        if let Some(e) = deepfilter::install_error() {
            ui.colored_label(ui.visuals().error_fg_color, "Install failed").on_hover_text(e);
        }
    }
}

// ---- ws:addon-faster-whisper ----
/// The Transcription engine (whisper.cpp built in, faster-whisper the add-on), faster-whisper's Precise
/// word timing, and the add-on itself: Install with its size, the running install, or its size and Remove.
fn transcription_row(ui: &mut egui::Ui, s: &mut Settings) -> bool {
    use crate::engine::fast_whisper::{self, Engine};
    let mut changed = false;
    let installed = fast_whisper::available();
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            let (resolved, _) = fast_whisper::resolve(None, &s.transcribe_engine, installed);
            let mut pick = Engine::parse(&s.transcribe_engine).unwrap_or(resolved);
            let before = pick;
            egui::ComboBox::from_id_salt("settings_transcribe_engine").selected_text(pick.id()).show_ui(ui, |ui| {
                for e in Engine::ALL {
                    ui.selectable_value(&mut pick, e, e.id()).on_hover_text(e.tip());
                }
            });
            if pick != before {
                s.transcribe_engine = pick.id().to_string();
                changed = true;
            }
            ui.add_enabled_ui(installed, |ui| {
                changed |= ui
                    .checkbox(&mut s.transcribe_align, "Precise word timing")
                    .on_hover_text("faster-whisper: pins every word to the audio with a second model (English), so captions and word cuts land exactly on the speech")
                    .changed();
            });
        });
        ui.horizontal(|ui| {
            if installed && fast_whisper::installing().is_none() {
                let v = fast_whisper::version().map(|v| format!(" {v}")).unwrap_or_default();
                ui.weak(format!("faster-whisper{v} installed, {:.0} MB", fast_whisper::disk_size() as f64 / 1e6));
                let id = egui::Id::new("faster_whisper_remove_error");
                if ui.button("Remove").on_hover_text("Removes the add-on and its models; transcripts already made stay").clicked() {
                    let r = fast_whisper::uninstall().err();
                    ui.data_mut(|d| d.insert_temp(id, r));
                }
                if let Some(e) = ui.data(|d| d.get_temp::<Option<String>>(id)).flatten() {
                    ui.colored_label(ui.visuals().error_fg_color, e);
                }
            } else {
                // the model Settings names (never `default_model()`: it reads settings.json, per frame here)
                let models = &crate::engine::transcribe::MODELS;
                let want = s.transcribe_model.trim();
                let file = models
                    .iter()
                    .find(|(n, f, _)| !want.is_empty() && (*f == want || n.starts_with(want)))
                    .unwrap_or(&models[1])
                    .1;
                crate::ui::subtitles_ui::addon_offer(ui, file);
            }
        });
    });
    changed
}

// ---- ws:addon-demucs ----
/// Install / uninstall the optional stem separation add-on (Demucs, used by clip ▸ Audio ▸ Stems).
fn demucs_row(ui: &mut egui::Ui) {
    use crate::engine::demucs;
    if let Some(p) = demucs::installing() {
        ui.add(egui::Spinner::new());
        ui.weak(format!("{:.0}% · {}", p.fraction() * 100.0, p.status()));
    } else if demucs::available() {
        ui.weak(match demucs::version() {
            Some(v) => format!("installed ({v})"),
            None => "installed".into(),
        });
        let id = egui::Id::new("demucs_uninstall_error");
        if ui.button("Uninstall").on_hover_text("Removes the add-on; stems it already separated stay in the cache").clicked() {
            let r = demucs::uninstall().err();
            ui.data_mut(|d| d.insert_temp(id, r));
        }
        if let Some(e) = ui.data(|d| d.get_temp::<Option<String>>(id)).flatten() {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
    } else {
        if ui
            .button("Install")
            .on_hover_text("Demucs (htdemucs) splits a song into drums, bass, other and vocals on the processor: right-click a clip ▸ Audio ▸ Stems. About 85 MB, downloaded once.")
            .clicked()
        {
            demucs::start_install();
        }
        match demucs::install_error() {
            Some(e) => ui.colored_label(ui.visuals().error_fg_color, format!("install failed: {e}")),
            None => ui.weak("optional, about 85 MB"),
        };
    }
}

// ---- ws:addon-sam2 ----
/// Install / uninstall the optional object-mask add-on (SAM 2, used by Object Mask).
fn object_masks_row(ui: &mut egui::Ui) {
    use crate::engine::sam2;
    if let Some(p) = sam2::installing() {
        ui.add(egui::Spinner::new());
        ui.weak(format!("{:.0}% · {}", p.fraction() * 100.0, p.status()));
    } else if sam2::available() {
        ui.weak(match sam2::version() {
            Some(v) => format!("installed ({v})"),
            None => "installed".into(),
        });
        let id = egui::Id::new("sam2_uninstall_error");
        if ui.button("Uninstall").on_hover_text("Removes the add-on; masks it already tracked stay in the cache").clicked() {
            let r = sam2::uninstall().err();
            ui.data_mut(|d| d.insert_temp(id, r));
        }
        if let Some(e) = ui.data(|d| d.get_temp::<Option<String>>(id)).flatten() {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
    } else {
        if ui
            .button("Install")
            .on_hover_text(
                "SAM 2 object masks: click an object in the Preview and its outline is followed through the clip (Inspector ▸ Mask ▸ Object Mask). Runs on the graphics card through DirectML, or on the processor. About 400 MB, downloaded once.",
            )
            .clicked()
        {
            sam2::start_install();
        }
        match sam2::install_error() {
            Some(e) => ui.colored_label(ui.visuals().error_fg_color, format!("install failed: {e}")),
            None => ui.weak("optional, about 400 MB"),
        };
    }
}

// ---- ws:addon-bg-removal ----
/// Install / uninstall the optional background-removal add-on (clip ▸ Remove Background).
fn bgremove_row(ui: &mut egui::Ui) {
    use crate::engine::bgremove;
    use crate::model::BgQuality;
    if let Some(p) = bgremove::installing() {
        ui.add(egui::Spinner::new());
        ui.weak(format!("{:.0}% · {}", p.fraction() * 100.0, p.status()));
        return;
    }
    let i = bgremove::installed();
    let tip = |q: BgQuality| match q {
        BgQuality::Fast => "MODNet: fast, made for people. About 45 MB with the runtime.",
        BgQuality::Best => "BiRefNet: slower, any subject, finer edges. About 240 MB with the runtime.",
    };
    for (q, have) in [(BgQuality::Fast, i.helper && i.runtime && i.fast), (BgQuality::Best, i.helper && i.runtime && i.best)] {
        if have {
            ui.weak(format!("{} installed", q.label())).on_hover_text(tip(q));
        } else if ui.button(format!("Install {}", q.label())).on_hover_text(tip(q)).clicked() {
            bgremove::start_install(q);
        }
    }
    if i.helper || i.runtime || i.fast || i.best {
        let id = egui::Id::new("bgremove_uninstall_error");
        if ui.button("Uninstall").on_hover_text("Removes the add-on; mattes already computed stay in the cache").clicked() {
            let r = bgremove::uninstall().err();
            ui.data_mut(|d| d.insert_temp(id, r));
        }
        if let Some(e) = ui.data(|d| d.get_temp::<Option<String>>(id)).flatten() {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
    }
    if let Some(e) = bgremove::install_error() {
        ui.colored_label(ui.visuals().error_fg_color, format!("install failed: {e}"));
    }
}

// ---- ws:neutral-web ----
/// A cache folder picked in Settings, and how to get there.
#[derive(Clone, Debug, Default)]
pub struct CachePick {
    /// The new folder ("" = the default).
    pub path: String,
    /// Move the existing cache there (else start fresh).
    pub mv: bool,
    /// Starting fresh: delete the old folder.
    pub delete_old: bool,
    /// The folder is the top of a drive and the user said that is fine.
    pub root_ok: bool,
}

fn gb(b: u64) -> String {
    if b >= 1_000_000_000 {
        format!("{:.1} GB", b as f64 / 1e9)
    } else {
        format!("{:.0} MB", b as f64 / 1e6)
    }
}

/// Cache folder: where it is, its size and the drive's free space, Browse… / Open folder / Reset, and
/// (once a folder is picked) Move existing cache / Start fresh. The change itself is the app's
/// (`caches::change_folder`, via `state.cache_change`).
fn cache_folder_row(ui: &mut egui::Ui, state: &mut SettingsUi, s: &Settings) {
    use crate::engine::cache_move;
    let dir = Settings::cache_dir();
    ui.vertical(|ui| {
        ui.add(egui::Label::new(dir.to_string_lossy()).truncate()).on_hover_text(dir.to_string_lossy());
        ui.horizontal(|ui| {
            let moving = cache_move::running();
            if let Some(p) = &moving {
                ui.add(egui::ProgressBar::new(p.fraction()).desired_width(160.0).text("Moving the cache…"));
                if ui.small_button("Cancel").on_hover_text("Stop the move: what is not moved yet stays in the old folder").clicked() {
                    p.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
                }
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
            }
            let idle = moving.is_none() && state.cache_pick.is_none();
            if ui.add_enabled(idle, egui::Button::new("Browse…")).clicked() {
                if let Some(p) = rfd::FileDialog::new().pick_folder() {
                    state.cache_pick = Some(CachePick { path: p.to_string_lossy().into_owned(), ..Default::default() });
                    state.cache_error = None;
                }
            }
            if ui.button("Open folder").clicked() {
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::process::Command::new("explorer").arg(&dir).spawn();
            }
            if ui
                .add_enabled(idle && !s.cache_folder.is_empty(), egui::Button::new("Reset to default"))
                .on_hover_text(Settings::default_cache_dir().to_string_lossy())
                .clicked()
            {
                state.cache_pick = Some(CachePick::default());
                state.cache_error = None;
            }
        });
        // size and free space: walking the folder is not free, so at most every 3 s
        let id = egui::Id::new("cache_folder_stats");
        let now = ui.input(|i| i.time);
        let stats = ui.data(|d| d.get_temp::<(f64, u64, Option<u64>)>(id)).filter(|st| now - st.0 < 3.0);
        let (_, used, free) = stats.unwrap_or_else(|| {
            let st = (now, crate::ui::app::caches_bytes_for_ui(), crate::engine::cache_lru::free_bytes(&dir));
            ui.data_mut(|d| d.insert_temp(id, st));
            st
        });
        let drive = dir.components().next().map(|c| c.as_os_str().to_string_lossy().into_owned()).unwrap_or_default();
        ui.weak(match free {
            Some(f) => format!("{} used · {} free on {drive}", gb(used), gb(f)),
            None => format!("{} used", gb(used)),
        });
        let Some(pick) = state.cache_pick.as_mut() else { return };
        let target = if pick.path.is_empty() { Settings::default_cache_dir() } else { std::path::PathBuf::from(&pick.path) };
        let (mut choice, mut cancel) = (None, false);
        ui.group(|ui| {
            ui.label(format!("Use {} for the cache:", target.display()));
            if cache_move::is_drive_root(&target) {
                ui.checkbox(&mut pick.root_ok, "Use the top of the drive (the cache's files go straight in it)");
            }
            if let Some(e) = &state.cache_error {
                ui.colored_label(ui.visuals().error_fg_color, e);
            }
            ui.horizontal(|ui| {
                if ui
                    .button("Move existing cache")
                    .on_hover_text("Moves the files in the background (Jobs), at low priority; files in use stay where they are")
                    .clicked()
                {
                    choice = Some(true);
                }
                if ui.button("Start fresh").on_hover_text("Renders, proxies and peaks are made again in the new folder when needed").clicked() {
                    choice = Some(false);
                }
                ui.checkbox(&mut pick.delete_old, "and delete the old folder");
                cancel = ui.button("Cancel").clicked();
            });
        });
        if cancel {
            state.cache_pick = None;
            state.cache_error = None;
        } else if let Some(mv) = choice {
            let mut c = state.cache_pick.take().unwrap_or_default();
            c.mv = mv;
            state.cache_change = Some(c);
        }
    });
}

/// Web browser: "Default (<name>)", every installed browser, or "Other…" (pick an exe). Used for links,
/// the snapshot extension's steps and the folder its snapshots are taken from.
fn browser_row(ui: &mut egui::Ui, state: &mut SettingsUi, s: &mut Settings) -> bool {
    use crate::engine::browsers::{self, Browser};
    let (default, installed) = state.browsers.get_or_insert_with(|| (browsers::default_browser(), browsers::installed()));
    let default_label = match default {
        Some(b) => format!("Default ({})", b.name),
        None => "Default (Windows)".into(),
    };
    let current = if s.web_browser.is_empty() {
        default_label.clone()
    } else {
        Browser::from_exe(std::path::Path::new(&s.web_browser)).name
    };
    let (mut changed, mut other) = (false, false);
    egui::ComboBox::from_id_salt("web_browser").selected_text(current).width(260.0).show_ui(ui, |ui| {
        changed |= ui.selectable_value(&mut s.web_browser, String::new(), default_label).changed();
        for b in installed.iter() {
            let exe = b.exe.to_string_lossy().into_owned();
            changed |= ui.selectable_value(&mut s.web_browser, exe.clone(), &b.name).on_hover_text(exe).changed();
        }
        other = ui.selectable_label(false, "Other…").clicked();
    });
    if other {
        if let Some(p) = rfd::FileDialog::new().add_filter("Program", &["exe"]).pick_file() {
            s.web_browser = p.to_string_lossy().into_owned();
            changed = true;
        }
    }
    changed
}
