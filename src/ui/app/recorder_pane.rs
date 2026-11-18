//! ---- fix/screen-record ----
//! PANE_DRAWERS glue for the Screen Recording pane (`ui::capture_ui::screen_pane`): starts / stops the
//! recording through `start_screen_capture` / `stop_screen_capture` (jobs.rs), feeds the level meters
//! (the recording's own audio taps, or meter-only taps for the chosen audio while idle) and keeps the
//! live preview ticking - 4 frames a second idle, 2 while recording - only while the pane is on screen
//! (`App::update` puts it to sleep otherwise).

use super::*;
use crate::engine::audio_tap::{self, AudioTap, TapSource};

pub(super) fn draw(app: &mut App, ui: &mut egui::Ui, pane: Pane) -> bool {
    if pane != Pane::ScreenRecording {
        return false;
    }
    if app.capture_ui.mics.is_none() {
        app.capture_ui.mics = Some(guarded(audio_tap::input_names).unwrap_or_default());
    }
    app.capture_ui.live = true;
    let recording = app.screen_rec.is_some();
    feed_meters(app, recording);
    let mics = app.capture_ui.mics.clone().unwrap_or_default();
    let monitors = app.screens.monitors.clone();
    let r = {
        let App { capture_ui, settings, palette, .. } = app;
        capture_ui::screen_pane(ui, capture_ui, settings, &mics, &monitors, recording, palette)
    };
    if r.stop_screen {
        app.stop_screen_capture();
    }
    if let Some(o) = r.screen {
        app.start_screen_capture(o);
    }
    if r.open_folder {
        let dir = capture_ui::capture_dir(&app.settings);
        let _ = std::process::Command::new("explorer").arg(dir).spawn();
    }
    ui.ctx().request_repaint_after(Duration::from_millis(if recording { 500 } else { 250 }));
    true
}

/// Show the pane in its own window; the first time it gets a size that fits the preview and every row.
pub(super) fn open(app: &mut App) {
    if !app.layout.popped_rects.iter().any(|(p, _)| *p == Pane::ScreenRecording) {
        app.layout.set_popped_rect(Pane::ScreenRecording, [160.0, 120.0, 460.0, 700.0]);
    }
    app.layout.popout(Pane::ScreenRecording);
    app.layout_dirty = true;
}

/// Levels for the pane's meters: the running recording's taps, else meter-only taps opened for the
/// chosen audio mode and microphone (reopened when either changes).
fn feed_meters(app: &mut App, recording: bool) {
    let st = &mut app.capture_ui;
    if let Some((c, _)) = &app.screen_rec {
        st.levels = c.levels();
        st.audio_error = c.tap_error();
        return;
    }
    if recording || st.finishing {
        st.levels = (None, None);
        return;
    }
    let mode = app.settings.capture_audio_mode();
    let key = format!("{}:{}", mode.key(), app.settings.capture_mic);
    if st.meter_key != key {
        st.meter_taps.clear();
        if mode.system() {
            st.meter_taps.push(AudioTap::start(TapSource::System, None));
        }
        if mode.mic() {
            st.meter_taps.push(AudioTap::start(TapSource::Mic(app.settings.capture_mic.clone()), None));
        }
        st.meter_key = key;
    }
    let mut it = st.meter_taps.iter().map(|t| t.state.peak());
    let sys = mode.system().then(|| it.next()).flatten();
    let mic = mode.mic().then(|| it.next()).flatten();
    st.levels = (sys, mic);
    st.audio_error = st.meter_taps.iter().find_map(|t| t.state.error());
}

use super::tools_helpers::*;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use serde_json::{json, Value};

/// What `capture.screen` reports: recording / finishing / idle, the audio choice, the last file.
fn status_json(app: &App) -> Value {
    let state = if app.screen_rec.is_some() {
        "recording"
    } else if app.capture_ui.finishing {
        "finishing"
    } else {
        "idle"
    };
    json!({
        "state": state,
        "elapsed": app.capture_ui.elapsed,
        "audio": app.settings.capture_audio_mode().key(),
        "file": app.screen_rec.as_ref().map(|(_, p)| p.to_string_lossy().into_owned()),
        "folder": capture_ui::capture_dir(&app.settings).to_string_lossy(),
    })
}

pub const TOOLS: &[ToolDef] = &[ToolDef {
    name: "capture.screen",
    desc: "Screen recording (the Screen Recording pane). action 'open' shows the pane, 'start' records with the \
           pane's source and audio choice, 'stop' finishes it (the mp4 is checked, then imported into the \
           library; poll with 'status' until state is 'idle'). 'audio' sets the remembered choice first.",
    args: &[
        "action:string:true:'open' | 'start' | 'stop' | 'status'",
        "audio:string:false:'system' | 'mic' | 'both' | 'none'",
        "source:string:false:'screen:<n>' (0 = main) | 'desktop' | 'window' | 'region'",
        "window:string:false:window title, for source 'window'",
    ],
    kind: ToolKind::Ui,
    run: |app, args| {
        if let Some(a) = arg_str(args, "audio") {
            let m = crate::engine::capture::AudioMode::from_key(a).ok_or("audio: system | mic | both | none")?;
            app.settings.capture_audio = m.key().into();
            app.settings.save();
        }
        if let Some(s) = arg_str(args, "source") {
            app.capture_ui.area = s.into();
        }
        if let Some(w) = arg_str(args, "window") {
            app.capture_ui.window = w.into();
        }
        match req(arg_str(args, "action"), "action")? {
            "open" => {
                if !app.layout.is_visible(Pane::ScreenRecording) {
                    open(app);
                }
            }
            "start" => {
                if app.screen_rec.is_some() {
                    return Err("already recording".into());
                }
                let o = app.blur_capture_options();
                app.start_screen_capture(o);
                if app.screen_rec.is_none() {
                    return Err("the recording didn't start (see the toast)".into());
                }
            }
            "stop" => {
                if app.screen_rec.is_none() {
                    return Err("not recording".into());
                }
                app.stop_screen_capture();
                app.capture_ui.finishing = true;
            }
            "status" => {}
            other => return Err(format!("unknown action '{other}'")),
        }
        Ok(ToolOutcome::Done(status_json(app)))
    },
}];
