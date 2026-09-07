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
//!
//! Split into one file per tab (general/performance/capture/hotkeys/appearance); this file keeps the
//! shared `SettingsUi` state, the ffmpeg/context-menu `Status` cache and the tab dispatcher.

mod appearance;
mod capture;
mod general;
mod hotkeys;
mod performance;

use crate::hotkeys::{Action, Hotkeys};
use crate::settings::Settings;
use crate::theme::Palette;
use eframe::egui;

#[derive(Default)]
pub struct SettingsUi {
    pub open: bool,
    pub tab: usize,
    pub rebinding: Option<Action>,
    /// Cached ffmpeg / context-menu status (filesystem + registry lookups are not per-frame).
    pub(super) status: Option<Status>,
    /// Last rebind note ("Unbound X") shown under the hotkey table.
    pub(super) note: String,
    /// MCP port while it is being dragged / typed; committed to the settings when the gesture ends
    /// (the app restarts the server on every value it sees, and a busy one in between switches it off).
    pub(super) port_edit: Option<u16>,
    /// ---- ws:forgiveness ----
    /// Performance tab's "Clear Caches" button was clicked this frame - the app (windows.rs) reads and
    /// resets this after calling `show`, since `performance()` has no `&mut App` to act on directly.
    pub clear_caches: bool,
    // ---- ws:command-palette ----
    /// Hotkeys tab search box text.
    pub(super) hotkeys_search: String,
    /// A rebind collided with another action's chord: (the action being rebound, its new chord, the
    /// action that already has it) - drives the inline Reassign/Keep row until the user picks one.
    pub(super) pending_conflict: Option<(Action, egui::KeyboardShortcut, Action)>,
}

pub(super) struct Status {
    /// Inputs the status was computed from; recomputed at the start of a frame when they differ
    /// (the app applies ffmpeg_dir / context_menu changes after `show` returns).
    pub(super) ffmpeg_dir: String,
    pub(super) ytdlp_dir: String,
    pub(super) context_menu: bool,
    pub(super) ffmpeg: String,
    pub(super) ytdlp: String,
    pub(super) ctxmenu: &'static str,
}

impl Status {
    pub(super) fn compute(s: &Settings) -> Self {
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
                None => "yt-dlp: not found - the Library's \"Import URL…\" button is hidden".into(),
            },
            ctxmenu: if crate::contextmenu::is_installed() { "(installed)" } else { "(not installed)" },
        }
    }
}

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
        // "when I open the settings tab, it should be really big" - sized to most of the screen,
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
                0 => general::general(ui, state, settings, mcp_status),
                1 => performance::performance(ui, settings, gpu_name, &mut state.clear_caches),
                2 => capture::capture_tab(ui, settings, audio_inputs),
                3 => capture::export(ui, settings, encoders),
                4 => hotkeys::hotkeys_tab(ui, state, hotkeys, settings),
                _ => appearance::appearance(ui, settings),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkeys::Action;
    use eframe::egui::{Event, Key, Modifiers};

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
        assert!(hotkeys::capture_key(&ctx, &mut st, Action::Split, &mut hk));
        assert_eq!(hk.text(Action::Split), "Ctrl+Shift+B");
        assert!(st.rebinding.is_none());
        assert!(ctx.input(|i| i.events.is_empty()));
        let _ = ctx.end_pass();

        // ---- ws:command-palette ----
        // Ctrl+Z conflicts with Undo: this no longer silently steals it - it opens the inline
        // Reassign/Keep row (`pending_conflict`) and leaves both bindings untouched until the user
        // picks one.
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
        assert!(!hotkeys::capture_key(&ctx, &mut st, Action::Split, &mut hk), "a conflict must not apply yet");
        assert_eq!(hk.text(Action::Split), "Ctrl+Shift+B", "unchanged pending the Reassign/Keep decision");
        assert_eq!(hk.text(Action::Undo), "Ctrl+Z", "unchanged pending the Reassign/Keep decision");
        let (a, ks, other) = st.pending_conflict.clone().expect("a conflict must open the Reassign/Keep row");
        assert_eq!((a, other), (Action::Split, Action::Undo));
        let _ = ctx.end_pass();
        // "Reassign" (hotkeys_tab's button does exactly this): apply it, clearing the prompt
        hk.set(a, Some(ks));
        st.pending_conflict = None;
        assert_eq!(hk.text(Action::Split), "Ctrl+Z");
        assert_eq!(hk.text(Action::Undo), "");

        // a chord RESERVED hard-codes ahead of the Action table (bare S) is rejected outright, with a
        // note, not silently accepted
        let mut input = egui::RawInput::default();
        input.events.push(Event::Key {
            key: Key::S,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        });
        ctx.begin_pass(input);
        st.rebinding = Some(Action::Split);
        st.note.clear();
        assert!(!hotkeys::capture_key(&ctx, &mut st, Action::Split, &mut hk));
        assert_eq!(hk.text(Action::Split), "Ctrl+Z", "a reserved chord must not be bound");
        assert!(st.note.contains("reserved"), "{}", st.note);
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
            assert_eq!(hotkeys::capture_key(&ctx, &mut st, Action::Split, &mut hk), expect_changed);
            assert_eq!(hk.text(Action::Split), expect_text);
            assert!(st.rebinding.is_none());
            let _ = ctx.end_pass();
        }
    }

    #[test]
    fn encoder_filter() {
        let all: Vec<String> =
            ["libx264", "h264_nvenc", "hevc_qsv", "libvpx-vp9", "libaom-av1", "h264_amf", "mpeg4", "gif", "aac"]
                .iter()
                .map(|s| s.to_string())
                .collect();
        assert_eq!(
            crate::ui::encoder_options(&all),
            vec!["libx264", "h264_nvenc", "hevc_qsv", "libvpx-vp9", "libaom-av1", "h264_amf"]
        );
        assert!(crate::ui::encoder_options(&[]).is_empty());
    }

    /// The Appearance tab's per-colour toggle: off stays unset, and an already-on value round-trips
    /// through a redraw unchanged (no spurious `changed` from just laying the row out).
    #[test]
    fn color_row_toggle_persists() {
        let ctx = egui::Context::default();
        let mut ov: Option<[u8; 3]> = None;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                assert!(!appearance::color_row(ui, "Test", &mut ov, egui::Color32::from_rgb(1, 2, 3)));
            });
        });
        assert!(ov.is_none(), "left alone: stays unset");

        ov = Some([1, 2, 3]);
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                assert!(!appearance::color_row(ui, "Test", &mut ov, egui::Color32::from_rgb(9, 9, 9)));
            });
        });
        assert_eq!(ov, Some([1, 2, 3]));
    }
}
