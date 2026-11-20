//! ---- fix/screen-record, feat/recorder-2 ----
//! PANE_DRAWERS glue for the Screen Recording pane (`ui::capture_ui::screen_pane`): starts / stops the
//! recording through `start_screen_capture` / `stop_screen_capture` (jobs.rs), feeds the level meters
//! (the recording's own audio taps, or meter-only taps for the chosen audio while idle) and runs the
//! live preview: the recording's own frames while it runs, else a preview-only producer
//! (`screen_grab::Preview`). The preview runs only while it is looked at - the pane on screen
//! (`App::update` puts it to sleep otherwise), its window not minimised, and not paused from its
//! right-click menu - and then repaints at its 30 fps.
//!
//! parity-webcam: the cameras and the picked camera's formats are probed in the background (`sync_cam`),
//! Snapshot runs `capture::snapshot` on its own thread and the picture is imported like a recording.

use super::*;
use crate::engine::audio_tap::{self, AudioTap, TapSource};
use crate::engine::capture::{CaptureSource, TrackKind};
use std::sync::atomic::Ordering as AtomicOrdering;

pub(super) fn draw(app: &mut App, ui: &mut egui::Ui, pane: Pane) -> bool {
    if pane != Pane::ScreenRecording {
        return false;
    }
    if app.capture_ui.mics.is_none() {
        app.capture_ui.mics = Some(guarded(audio_tap::input_names).unwrap_or_default());
    }
    app.capture_ui.live = true;
    sync_cam(app);
    let recording = app.screen_rec.is_some();
    feed_meters(app, recording);
    let minimized = ui.ctx().input(|i| i.viewport().minimized.unwrap_or(false));
    let showing = feed_preview(app, recording, minimized);
    let mics = app.capture_ui.mics.clone().unwrap_or_default();
    let monitors = app.screens.monitors.clone();
    let bins = app.project.folder_names();
    let r = {
        let App { capture_ui, settings, palette, .. } = app;
        let c = capture_ui::PaneCtx { mics: &mics, monitors: &monitors, bins: &bins, recording, palette };
        capture_ui::screen_pane(ui, capture_ui, settings, &c)
    };
    if r.stop_screen {
        app.stop_screen_capture();
    }
    if let Some(name) = r.new_bin {
        app.push_undo();
        app.project.add_folder(&name);
        app.after_edit();
    }
    if let Some(o) = r.screen {
        app.start_screen_capture(o);
    }
    if r.snapshot {
        app.take_snapshot();
    }
    if r.open_folder {
        let dir = capture_ui::capture_dir(&app.settings);
        let _ = std::process::Command::new("explorer").arg(dir).spawn();
    }
    let meters = !app.capture_ui.meter_taps.is_empty() || recording;
    if showing {
        ui.ctx().request_repaint_after(Duration::from_millis(1000 / crate::engine::screen_grab::PREVIEW_FPS as u64));
    } else if meters {
        ui.ctx().request_repaint_after(Duration::from_millis(100));
    }
    true
}

/// The preview's frames: the recording's own, else a preview-only producer for the picked source.
/// Nothing runs while it isn't looked at. True when frames are coming.
fn feed_preview(app: &mut App, recording: bool, minimized: bool) -> bool {
    let st = &mut app.capture_ui;
    // a webcam opens for one program at a time: the preview lets go of it while a photo is taken
    st.snapping = app.pending_recordings.iter().any(|r| r.snapshot && r.done.as_ref().is_some_and(|p| !p.is_done()));
    let wanted = app.settings.capture_preview && !minimized && !(st.snapping && st.area == "webcam");
    if let Some((c, _)) = &app.screen_rec {
        st.preview = None; // one capture at a time: the recording's frames replace the idle preview
        st.slot = c.preview();
    } else if !wanted || recording {
        st.preview = None;
        st.slot = None;
    } else {
        let src = capture_ui::source_of(st, &app.screens.monitors);
        let pickable = match st.area.as_str() {
            "window" => !st.window.is_empty(),
            "app" => st.app.is_some(),
            "webcam" => !st.cam.device.is_empty(),
            _ => true,
        };
        let cursor = app.settings.capture_cursor;
        if !pickable {
            st.preview = None;
        } else if st.preview.as_ref().is_none_or(|p| p.source != src || p.cursor != cursor) {
            st.preview = None; // stop the old one first: two grabs of one screen cost twice
            st.preview = crate::engine::screen_grab::Preview::start(src, cursor);
        }
        st.slot = st.preview.as_ref().map(|p| p.slot.clone());
    }
    if let Some(s) = &st.slot {
        s.wanted.store(wanted, AtomicOrdering::Relaxed);
        let g = s.generation();
        let now = std::time::Instant::now();
        st.preview_rate = match st.preview_rate {
            Some((t, g0, _)) if now.duration_since(t).as_secs_f32() >= 1.0 => {
                Some((now, g, g.saturating_sub(g0) as f32 / now.duration_since(t).as_secs_f32()))
            }
            Some(r) => Some(r),
            None => Some((now, g, 0.0)),
        };
    } else {
        st.preview_rate = None;
    }
    wanted && st.slot.is_some()
}

/// The cameras (probed once in the background), the picked camera's formats, and the camera the
/// settings resolve to, into the pane's state.
pub(super) fn sync_cam(app: &mut App) {
    use crate::engine::webcam;
    let st = &mut app.capture_ui;
    if st.cams.is_none() {
        st.cams = webcam::devices_nonblocking(super::panes::wake_ui);
    }
    st.cam = capture_ui::cam_spec(&app.settings, st.cams.as_deref());
    if st.cam_formats.is_none() && !st.cam.device.is_empty() {
        st.cam_formats = webcam::formats_nonblocking(&st.cam.device, super::panes::wake_ui);
    }
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
/// chosen audio, microphone and app (reopened when any of them changes).
fn feed_meters(app: &mut App, recording: bool) {
    let st = &mut app.capture_ui;
    if let Some((c, _)) = &app.screen_rec {
        st.levels = c.levels();
        st.audio_error = c.tap_error();
        return;
    }
    if recording || st.finishing {
        st.levels = [None; 3];
        return;
    }
    let mode = app.settings.capture_audio_mode();
    let tracks = &app.settings.capture_tracks;
    let has = |k: TrackKind| tracks.iter().any(|t| t.on && t.kind == k);
    let mix = has(TrackKind::Mix);
    let sys = has(TrackKind::System) || (mix && mode.system());
    let mic = has(TrackKind::Mic) || (mix && mode.mic());
    let app_pid = match capture_ui::source_of(st, &app.screens.monitors) {
        CaptureSource::App { pid, .. } if has(TrackKind::App) => Some(pid),
        _ => None,
    };
    let key = format!("{sys}:{mic}:{}:{app_pid:?}", app.settings.capture_mic);
    if st.meter_key != key {
        st.meter_taps.clear();
        let mut taps = Vec::new();
        if sys {
            taps.push((0, AudioTap::start(TapSource::System, None)));
        }
        if mic {
            taps.push((1, AudioTap::start(TapSource::Mic(app.settings.capture_mic.clone()), None)));
        }
        if let Some(pid) = app_pid {
            taps.push((2, AudioTap::start(TapSource::App(pid), None)));
        }
        st.meter_slots = taps.iter().map(|(i, _)| *i).collect();
        st.meter_taps = taps.into_iter().map(|(_, t)| t).collect();
        st.meter_key = key;
    }
    let mut levels = [None; 3];
    for (slot, t) in st.meter_slots.iter().zip(&st.meter_taps) {
        levels[*slot] = Some(t.state.peak());
    }
    st.levels = levels;
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
        "source": app.capture_ui.area,
        "camera": app.capture_ui.cam.device,
        "bubble": app.settings.cam_bubble,
        "snapping": app.capture_ui.snapping,
        "preview_fps": app.capture_ui.preview_rate.map(|r| r.2),
        "hidden": app.capture_ui.pill.as_ref().is_some_and(|_| app.settings.capture_hide),
        "file": app.screen_rec.as_ref().map(|(_, p)| p.to_string_lossy().into_owned()),
        "folder": capture_ui::capture_dir(&app.settings).to_string_lossy(),
    })
}

pub const TOOLS: &[ToolDef] = &[ToolDef {
    name: "capture.screen",
    desc: "Screen recording (the Screen Recording pane). action 'open' shows the pane, 'start' records with the \
           pane's source and audio choice, 'stop' finishes it (the mp4 is checked, then imported into the \
           library; poll with 'status' until state is 'idle'). 'snapshot' takes a still of the source into the \
           library; 'devices' lists the webcams and the picked one's formats. 'audio' sets the remembered choice first.",
    args: &[
        "action:string:true:'open' | 'start' | 'stop' | 'status' | 'snapshot' | 'devices'",
        "audio:string:false:'system' | 'mic' | 'both' | 'none'",
        "source:string:false:'screen:<n>' (0 = main) | 'desktop' | 'window' | 'region' | 'app' | 'webcam'",
        "camera:string:false:webcam name for source 'webcam' and the bubble ('' = the first found), remembered",
        "format:string:false:webcam format key from 'devices' (e.g. 'mjpeg 1280x720 30'; '' = camera default), remembered",
        "bubble:boolean:false:put a round webcam picture in a corner of screen recordings, remembered",
        "preview:boolean:false:show (true) or pause (false) the live preview, remembered",
        "dock:boolean:false:with 'open': true docks the pane into the current page, false pops it out",
        "window:string:false:window title for source 'window'; for 'app' part of the title, the program name (RobloxStudioBeta.exe) or pid:<process id>",
    ],
    kind: ToolKind::Ui,
    run: |app, args| {
        if let Some(c) = arg_str(args, "camera") {
            app.settings.cam_device = c.into();
            app.settings.cam_format.clear();
            app.capture_ui.cam_formats = None;
            app.settings.save();
        }
        if let Some(f) = arg_str(args, "format") {
            if !f.is_empty() && crate::engine::webcam::CamFormat::from_key(f).is_none() {
                return Err("format: a key from action 'devices', like 'mjpeg 1280x720 30'".into());
            }
            app.settings.cam_format = f.into();
            app.settings.save();
        }
        if let Some(on) = args.get("bubble").and_then(Value::as_bool) {
            app.settings.cam_bubble = on;
            app.settings.save();
        }
        if let Some(a) = arg_str(args, "audio") {
            let m = crate::engine::capture::AudioMode::from_key(a).ok_or("audio: system | mic | both | none")?;
            app.settings.capture_audio = m.key().into();
            app.settings.save();
        }
        if let Some(on) = args.get("preview").and_then(Value::as_bool) {
            app.settings.capture_preview = on;
            app.settings.save();
        }
        if let Some(s) = arg_str(args, "source") {
            app.capture_ui.area = s.into();
        }
        if let Some(w) = arg_str(args, "window") {
            if app.capture_ui.area == "app" {
                let want = w.to_ascii_lowercase();
                let pid = w.strip_prefix("pid:").and_then(|p| p.trim().parse::<u32>().ok());
                let found = crate::engine::wgc::app_windows().into_iter().find(|a| match pid {
                    Some(pid) => a.pid == pid,
                    None => a.title.to_ascii_lowercase().contains(&want) || a.exe.eq_ignore_ascii_case(w),
                });
                app.capture_ui.app = Some(found.ok_or_else(|| format!("no application window matches '{w}'"))?);
            } else {
                app.capture_ui.window = w.into();
            }
        }
        match req(arg_str(args, "action"), "action")? {
            "open" => match args.get("dock").and_then(Value::as_bool) {
                Some(true) => {
                    app.layout.dock(Pane::ScreenRecording);
                    app.layout_dirty = true;
                }
                Some(false) => open(app),
                None if !app.layout.is_visible(Pane::ScreenRecording) => open(app),
                None => {}
            },
            "devices" => {
                // blocking here is fine: an MCP call, not a frame; the pane's lists fill from the same probes
                let cams = crate::engine::webcam::devices();
                app.capture_ui.cams = Some(cams.clone());
                sync_cam(app);
                let formats = if app.capture_ui.cam.device.is_empty() {
                    Vec::new()
                } else {
                    crate::engine::webcam::formats(&app.capture_ui.cam.device)
                };
                let list: Vec<Value> = formats.iter().map(|f| json!({ "key": f.key(), "label": f.label() })).collect();
                app.capture_ui.cam_formats = Some(formats);
                return Ok(ToolOutcome::Done(json!({ "webcams": cams, "camera": app.capture_ui.cam.device, "formats": list })));
            }
            "snapshot" => {
                sync_cam(app);
                if app.capture_ui.snapping {
                    return Err("a snapshot is already being taken".into());
                }
                if !app.take_snapshot() {
                    return Err("the snapshot didn't start (see the toast)".into());
                }
            }
            "start" => {
                sync_cam(app);
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
