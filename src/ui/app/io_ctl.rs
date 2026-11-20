//! ws:parity-io: I/O and monitoring - the App side of
//!  * Media Properties (Library item ▸ Properties…, `Action::MediaProperties`): `engine::media_info` on a
//!    worker thread, the bitrate graph on demand, rotation / range overrides as one undo step each;
//!  * the decode overrides themselves: `Project::media_io` is mirrored into `media::io_override` every
//!    frame it changes, and the sources it touched are re-decoded;
//!  * Open Network Stream (File ▸ Open Network Stream…): probe, live preview, Record to Library;
//!  * full-range export (the static `engine::export` reads);
//!  * GPU choice (Windows' per-app preference, written when the setting changes);
//!  * the Contour ShuttlePRO reader (`engine::shuttle`): shuttle = rate, jog = frames, buttons = actions.
//!
//! Drawing is `ui::io_ui`; this file only owns state, threads and edits.

use super::tools_helpers::{arg_bool, arg_f64, arg_str, arg_u64, req};
use super::*;
use crate::engine::media_info::{self, Bitrate, MediaInfo};
use crate::engine::{gpu_pick, netstream, shuttle};
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::media::io_override::{self, MediaIo, Range};
use crate::ui::io_ui::{self, PropsAction, PropsUi, PropsView, StreamAction, StreamUi, StreamView};
use std::collections::{BTreeMap, HashMap};

/// A worker's answer: None until it is done.
type Slot<T> = Arc<Mutex<Option<Result<T, String>>>>;

/// Run `f` on a worker thread; the slot fills (and the UI wakes) when it returns.
fn spawn_slot<T: Send + 'static>(
    ctx: Option<egui::Context>,
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Slot<T> {
    let slot: Slot<T> = Arc::new(Mutex::new(None));
    let s = slot.clone();
    std::thread::spawn(move || {
        crate::media::bg::lower_thread();
        let r = f();
        *s.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
        if let Some(c) = ctx {
            c.request_repaint();
        }
    });
    slot
}

fn peek<T: Clone>(slot: &Slot<T>) -> Option<Result<T, String>> {
    slot.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

struct Props {
    path: String,
    title: String,
    /// The project asset it was opened for (overrides are offered only then).
    asset: Option<Id>,
    info: Slot<MediaInfo>,
    /// The graph shown: (stream selector, its data).
    graph: Option<(String, Slot<Bitrate>)>,
}

#[derive(Default)]
pub(crate) struct IoState {
    props: Option<Props>,
    props_ui: PropsUi,
    stream_open: bool,
    stream_ui: StreamUi,
    probe: Option<Slot<MediaInfo>>,
    preview: Option<netstream::Preview>,
    preview_tex: Option<egui::TextureHandle>,
    recording: Option<netstream::Recording>,
    /// `media.bitrate` results by (path, selector).
    bitrates: HashMap<(String, String), Slot<Bitrate>>,
    /// The override map the decoders follow now (`io_override::sync`).
    synced: Option<BTreeMap<String, MediaIo>>,
    /// The GPU setting already written to Windows (None = not looked at yet: the value at start-up).
    gpu_applied: Option<String>,
    shuttle: Option<(shuttle::Reader, Receiver<shuttle::Msg>)>,
    /// The UI context (from the frame hook) for workers started by a tool call.
    ctx: Option<egui::Context>,
}

// ------------------------------------------------------------------------------------------ frame hook

pub(super) fn tick(app: &mut App, ctx: &egui::Context) {
    if app.io.ctx.is_none() {
        app.io.ctx = Some(ctx.clone());
    }
    sync_overrides(app);
    crate::engine::export::set_full_range(app.settings.export_full_range);
    gpu_tick(app);
    shuttle_tick(app, ctx);
    stream_tick(app, ctx);
}

/// Mirror `Project::media_io` into the decoders, and re-decode the sources it changed.
fn sync_overrides(app: &mut App) {
    if app.io.synced.as_ref() == Some(&app.project.media_io) {
        return;
    }
    let changed = io_override::sync(&app.project.media_io);
    app.io.synced = Some(app.project.media_io.clone());
    if changed.is_empty() {
        return;
    }
    // a proxy-map round trip with these sources pointed at themselves is exactly "their decode changed":
    // their decoders are dropped and only their spans of the cache are evicted (`DecoderPool::set_proxies`)
    let mut bumped = app.proxy_map.clone();
    for p in &changed {
        let same = bumped.get(p) == Some(p);
        bumped.insert(p.clone(), if same { format!("{p}?") } else { p.clone() });
    }
    for player in std::iter::once(&mut app.player).chain(app.source.as_mut().map(|s| &mut s.player)) {
        player.set_proxies(bumped.clone());
        player.set_proxies(app.proxy_map.clone());
    }
}

fn gpu_tick(app: &mut App) {
    let want = &app.settings.gpu_adapter;
    match &app.io.gpu_applied {
        None => app.io.gpu_applied = Some(want.clone()),
        Some(done) if done == want => {}
        Some(_) => {
            let want = want.clone();
            app.io.gpu_applied = Some(want.clone());
            let choice = gpu_pick::Choice::parse(&want);
            match gpu_pick::apply_windows_pref(&choice) {
                Ok(()) => app.toast(format!("GPU: {} - restart Simple Editor to switch", choice.label())),
                Err(e) => app.toast(format!("Couldn't set the GPU preference, Windows said: {e}")),
            }
        }
    }
}

fn shuttle_tick(app: &mut App, ctx: &egui::Context) {
    match (app.settings.shuttle, app.io.shuttle.is_some()) {
        (true, false) => {
            let (tx, rx) = std::sync::mpsc::channel();
            let wake = ctx.clone();
            let reader = shuttle::Reader::start(move |m| {
                let _ = tx.send(m);
                wake.request_repaint();
            });
            app.io.shuttle = Some((reader, rx));
        }
        (false, true) => {
            app.io.shuttle = None; // the reader stops on drop
            shuttle::set_status(None);
        }
        _ => {}
    }
    let msgs: Vec<shuttle::Msg> = match &app.io.shuttle {
        Some((_, rx)) => rx.try_iter().collect(),
        None => return,
    };
    for m in msgs {
        match m {
            shuttle::Msg::Connected(model) => {
                shuttle::set_status(Some(model));
                app.toast(format!("{model} connected"));
            }
            shuttle::Msg::Disconnected => shuttle::set_status(None),
            shuttle::Msg::Event(e) => shuttle_event(app, e),
        }
    }
}

/// One controller event: the ring sets the play rate (centre = pause), the wheel steps frames, a button
/// runs its action.
fn shuttle_event(app: &mut App, e: shuttle::Event) {
    match e {
        shuttle::Event::Shuttle(pos) => match shuttle::rate(pos) {
            0.0 => app.act(Action::Stop),
            r => app.play_gated(r),
        },
        shuttle::Event::Jog(n) => {
            let a = if n > 0 { Action::StepForward } else { Action::StepBack };
            for _ in 0..n.unsigned_abs().min(30) {
                app.act(a);
            }
        }
        shuttle::Event::Button(i, true) => {
            let id = app.settings.shuttle_buttons.get(i as usize - 1).cloned().unwrap_or_default();
            if let Some(a) = Action::from_id(&id) {
                app.act(a);
            }
        }
        shuttle::Event::Button(_, false) => {}
    }
}

fn stream_tick(app: &mut App, ctx: &egui::Context) {
    if let Some(p) = &app.io.preview {
        if let Some(f) = p.take() {
            let img = egui::ColorImage::from_rgba_unmultiplied([f.width as usize, f.height as usize], &f.rgba);
            match app.io.preview_tex.as_mut() {
                Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                None => {
                    app.io.preview_tex = Some(ctx.load_texture("stream_preview", img, egui::TextureOptions::LINEAR))
                }
            }
        }
    }
    let Some(rec) = &app.io.recording else { return };
    match rec.finished() {
        None => ctx.request_repaint_after(Duration::from_secs(1)), // the time and size count up
        Some(r) => {
            let rec = app.io.recording.take().expect("checked above");
            match r {
                Ok(()) => {
                    app.toast_with_folder("Stream recorded - adding it to the Library", rec.out.clone());
                    app.import_recording(rec.out.clone(), None, None);
                }
                Err(e) => app.toast(format!("The stream recording stopped without a file: {e}")),
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------- acts

pub(super) fn act(app: &mut App, a: Action) -> bool {
    match a {
        Action::OpenStream => {
            app.io.stream_open = !app.io.stream_open;
            if !app.io.stream_open {
                close_stream(app);
            }
            true
        }
        Action::MediaProperties => {
            match selected_asset(app) {
                Some(id) => open_props_for_asset(app, id),
                None => app.toast("Select a clip or a Library item to see its properties"),
            }
            true
        }
        _ => false,
    }
}

/// The Library's anchor, else the first selected clip's asset.
fn selected_asset(app: &App) -> Option<Id> {
    app.library.selected.filter(|id| app.project.asset(*id).is_some()).or_else(|| {
        app.selection
            .iter()
            .filter_map(|c| app.project.clip(*c))
            .map(|c| c.asset)
            .find(|a| app.project.asset(*a).is_some())
    })
}

/// Open (or re-point) the Properties window at a project asset.
pub(super) fn open_props_for_asset(app: &mut App, id: Id) {
    let Some(a) = app.project.asset(id) else { return };
    let path = a.path.clone();
    open_props(app, path.clone(), file_name(&path), Some(id));
}

/// Open the Properties window at a file (an asset or not).
pub(super) fn open_props(app: &mut App, path: String, title: String, asset: Option<Id>) {
    if app.io.props.as_ref().is_some_and(|p| p.path == path) {
        app.io.props.as_mut().expect("checked").asset = asset;
        return;
    }
    let p = path.clone();
    let info = spawn_slot(app.io.ctx.clone(), move || media_info::properties(&p));
    app.io.props = Some(Props { path, title, asset, info, graph: None });
}

/// A path's file name (a URL's last part), for a window title.
fn file_name(path: &str) -> String {
    let t = path.trim_end_matches(['/', '\\']);
    t.rsplit(['/', '\\']).next().filter(|s| !s.is_empty()).unwrap_or(t).to_string()
}

fn close_stream(app: &mut App) {
    app.io.preview = None;
    app.io.preview_tex = None;
    app.io.probe = None;
    app.io.stream_ui.error = None;
}

// --------------------------------------------------------------------------------------------- windows

pub(super) fn windows(app: &mut App, ctx: &egui::Context) {
    props_window(app, ctx);
    stream_window(app, ctx);
}

fn props_window(app: &mut App, ctx: &egui::Context) {
    let Some(p) = &app.io.props else { return };
    let info = peek(&p.info);
    let graph = p.graph.as_ref().map(|(_, s)| peek(s));
    let io = p.asset.filter(|id| app.project.asset(*id).is_some()).map(|_| app.project.media_io_of(&p.path));
    let view = PropsView { title: &p.title, info: info.as_ref(), bitrate: graph.as_ref().map(|g| g.as_ref()), io };
    let mut open = true;
    let act = io_ui::props_window(ctx, &mut open, &mut app.io.props_ui, &view);
    if !open {
        app.io.props = None;
        return;
    }
    match act {
        Some(PropsAction::Bitrate(sel)) => {
            let path = app.io.props.as_ref().map(|p| p.path.clone()).unwrap_or_default();
            let slot = bitrate_slot(app, &path, &sel);
            if let Some(p) = app.io.props.as_mut() {
                p.graph = Some((sel, slot));
            }
        }
        Some(PropsAction::SetIo(io)) => {
            let (path, coded, rot) = match info.as_ref().and_then(|i| i.as_ref().ok()) {
                Some(i) => (i.path.clone(), coded_of(i), file_rotation(i)),
                None => return,
            };
            app.push_undo();
            app.project.set_media_io(&path, io, coded, rot);
            app.after_edit();
        }
        None => {}
    }
}

/// The bitrate of `sel` in `path`, measured once per session.
fn bitrate_slot(app: &mut App, path: &str, sel: &str) -> Slot<Bitrate> {
    let ctx = app.io.ctx.clone();
    app.io
        .bitrates
        .entry((path.to_string(), sel.to_string()))
        .or_insert_with(|| {
            let (p, s) = (path.to_string(), sel.to_string());
            spawn_slot(ctx, move || media_info::bitrate(&p, &s, 1.0))
        })
        .clone()
}

/// The first video stream's stored size.
fn coded_of(i: &MediaInfo) -> Option<(u32, u32)> {
    i.streams.iter().find(|s| s.kind == "video").map(|s| (s.width, s.height))
}
fn file_rotation(i: &MediaInfo) -> i32 {
    i.streams.iter().find(|s| s.kind == "video").map_or(0, |s| s.rotation)
}

fn stream_window(app: &mut App, ctx: &egui::Context) {
    if !app.io.stream_open {
        return;
    }
    let probe = app.io.probe.as_ref().map(peek);
    let preview = app.io.preview_tex.as_ref().map(|t| (t.id(), t.size_vec2()));
    let perr = app.io.preview.as_ref().and_then(|p| p.error());
    let recording = app.io.recording.as_ref().map(|r| (r.started.elapsed().as_secs_f64(), r.bytes(), r.stopping()));
    let recent = app.settings.stream_urls.clone();
    let view = StreamView {
        recent: &recent,
        probe: probe.as_ref().map(|p| p.as_ref()),
        preview,
        previewing: app.io.preview.is_some() && perr.is_none(),
        preview_error: perr.as_deref(),
        recording,
    };
    let mut open = true;
    let act = io_ui::stream_window(ctx, &mut open, &mut app.io.stream_ui, &view);
    if !open {
        app.io.stream_open = false;
        if app.io.recording.is_none() {
            close_stream(app);
        } else {
            app.io.preview = None; // the recording carries on; its toast says when it is in the Library
        }
        return;
    }
    let result = match act {
        Some(StreamAction::Open) => {
            let url = app.io.stream_ui.url.clone();
            open_stream(app, &url)
        }
        Some(StreamAction::StopPreview) => {
            app.io.preview = None;
            Ok(())
        }
        Some(StreamAction::Record(limit)) => {
            let url = app.io.stream_ui.url.clone();
            start_recording(app, &url, limit).map(|_| ())
        }
        Some(StreamAction::StopRecording) => {
            if let Some(r) = app.io.recording.as_mut() {
                r.stop();
            }
            Ok(())
        }
        None => Ok(()),
    };
    if let Err(e) = result {
        app.io.stream_ui.error = Some(e);
    }
}

/// Look at a stream and start its preview. Remembers the address.
fn open_stream(app: &mut App, url: &str) -> Result<(), String> {
    let url = netstream::validate(url)?;
    if crate::media::ffpipe::ffmpeg_exe().is_none() {
        return Err("ffmpeg.exe not found - set its folder in Settings ▸ General".into());
    }
    app.io.stream_ui.error = None;
    app.io.stream_ui.url = url.clone();
    app.io.stream_open = true;
    let ctx = app.io.ctx.clone();
    let u = url.clone();
    app.io.probe = Some(spawn_slot(ctx.clone(), move || media_info::properties(&u)));
    app.io.preview_tex = None;
    app.io.preview = netstream::Preview::start(&url, move || {
        if let Some(c) = &ctx {
            c.request_repaint();
        }
    })
    .ok();
    remember_url(app, &url);
    Ok(())
}

fn remember_url(app: &mut App, url: &str) {
    let l = &mut app.settings.stream_urls;
    l.retain(|u| u != url);
    l.insert(0, url.to_string());
    l.truncate(10);
    app.settings.save_async();
}

/// Start recording `url` into the capture folder's Streams folder.
fn start_recording(app: &mut App, url: &str, limit: Option<f64>) -> Result<PathBuf, String> {
    if app.io.recording.is_some() {
        return Err("A stream is already being recorded".into());
    }
    let url = netstream::validate(url)?;
    let dir = crate::ui::capture_ui::capture_dir(&app.settings).join("Streams");
    let out = netstream::record_path(&dir, &url, &format!("{}", crate::settings::Settings::now()));
    let rec = netstream::Recording::start(&url, out.clone(), limit)?;
    app.io.recording = Some(rec);
    remember_url(app, &url);
    Ok(out)
}

// ------------------------------------------------------------------------------------------------ tools

/// The file a tool is about: `asset_id`, else `path` (any file, or a stream URL for `media.properties`).
fn target(app: &App, args: &Value) -> Result<(String, Option<Id>), String> {
    if let Some(id) = arg_u64(args, "asset_id") {
        let a = app.project.asset(id).ok_or_else(|| format!("no asset {id}"))?;
        return Ok((a.path.clone(), Some(id)));
    }
    match arg_str(args, "path").map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => Ok((
            app.project.asset_by_path(p).map_or(p.to_string(), |a| a.path.clone()),
            app.project.asset_by_path(p).map(|a| a.id),
        )),
        None => Err("give asset_id or path".into()),
    }
}

fn rotate_arg(args: &Value, cur: Option<u16>) -> Result<Option<u16>, String> {
    match args.get("rotate") {
        None | Some(Value::Null) => Ok(cur),
        Some(Value::String(s)) if s.eq_ignore_ascii_case("auto") || s.eq_ignore_ascii_case("file") => Ok(None),
        Some(v) => {
            let deg = v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().trim_end_matches('°').parse().ok()));
            match deg {
                Some(d) if (d / 90.0).fract() == 0.0 => Ok(Some(MediaIo::quarter(d as i32))),
                _ => Err("rotate is auto, 0, 90, 180 or 270".into()),
            }
        }
    }
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "media.properties",
        desc: "Detailed properties of a media file or stream: container format, duration, size, overall bitrate, tags, and every stream (codec and profile, resolution, frame rate, pixel format, colour range / primaries / transfer / matrix, rotation, field order, sample rate, channels, language, bitrate), plus the decode override set with media.set_io. Also opens the Properties window when show is true.",
        args: &[
            "asset_id:integer:false:a library asset",
            "path:string:false:any media file or a stream URL (instead of asset_id)",
            "show:boolean:false:also open the Media Properties window",
        ],
        kind: ToolKind::Read,
        run: |app, args| {
            let (path, asset) = target(app, args)?;
            let info = media_info::properties(&path)?;
            if arg_bool(args, "show").unwrap_or(false) {
                open_props(app, path.clone(), file_name(&path), asset);
            }
            let mut v = serde_json::to_value(&info).map_err(|e| e.to_string())?;
            v["override"] = serde_json::to_value(app.project.media_io_of(&path)).unwrap_or(Value::Null);
            Ok(ToolOutcome::Done(v))
        },
    },
    ToolDef {
        name: "media.bitrate",
        desc: "Per-second bitrate of a file's video or audio stream (kbps per bucket, average, peak, keyframe times). Reads the whole file once on a worker thread: the first call returns pending:true, call again until it returns the data.",
        args: &[
            "asset_id:integer:false:a library asset",
            "path:string:false:any media file (instead of asset_id)",
            "stream:string:false:video (default) | audio",
        ],
        kind: ToolKind::Read,
        run: |app, args| {
            let (path, _) = target(app, args)?;
            let sel = match arg_str(args, "stream").unwrap_or("video") {
                "audio" => "a:0",
                _ => "v:0",
            };
            let slot = bitrate_slot(app, &path, sel);
            Ok(ToolOutcome::Done(match peek(&slot) {
                None => json!({"pending": true}),
                Some(Ok(b)) => serde_json::to_value(b).map_err(|e| e.to_string())?,
                Some(Err(e)) => return Err(e),
            }))
        },
    },
    ToolDef {
        name: "media.set_io",
        desc: "Override how a library video is decoded, for every clip of it: rotate (auto = the file's own rotation, or 0 / 90 / 180 / 270 clockwise, ignoring the file's) and range (auto = as tagged, limited = 16-235, full = 0-255; for footage tagged wrong). A quarter turn swaps the asset's width and height. One undo step.",
        args: &[
            "asset_id:integer:false:the library asset",
            "path:string:false:its file (instead of asset_id)",
            "rotate:string:false:auto | 0 | 90 | 180 | 270 (unchanged when omitted)",
            "range:string:false:auto | limited | full (unchanged when omitted)",
        ],
        kind: ToolKind::Mutate,
        run: |app, args| {
            let (path, asset) = target(app, args)?;
            if asset.is_none() {
                return Err(format!("{path} is not in the Library"));
            }
            let cur = app.project.media_io_of(&path);
            let range = match arg_str(args, "range") {
                None => cur.range,
                Some(r) => Range::parse(r).ok_or("range is auto, limited or full")?,
            };
            let io = MediaIo { rotate: rotate_arg(args, cur.rotate)?, range };
            let info = media_info::properties(&path).ok();
            let coded = info.as_ref().and_then(coded_of);
            let rot = info.as_ref().map_or(0, file_rotation);
            let n = app.project.set_media_io(&path, io, coded, rot);
            let a = asset.and_then(|id| app.project.asset(id));
            Ok(ToolOutcome::Done(json!({
                "ok": true, "assets": n, "override": io,
                "width": a.map(|a| a.width), "height": a.map(|a| a.height),
            })))
        },
    },
    ToolDef {
        name: "stream.open",
        desc: "Open a network stream (http(s) / HLS / DASH, rtmp, rtsp, srt, udp, rtp) in the Open Network Stream window: probes it and starts a live preview. Read what was found with media.properties path=<url>.",
        args: &["url:string:true:the stream address"],
        kind: ToolKind::Ui,
        run: |app, args| {
            let url = req(arg_str(args, "url"), "url")?.to_string();
            open_stream(app, &url)?;
            Ok(ToolOutcome::Done(json!({"ok": true, "url": app.io.stream_ui.url})))
        },
    },
    ToolDef {
        name: "stream.record",
        desc: "Record a network stream to a file (stream copy, no re-encode) and add it to the Library when it stops. action start (url, optional seconds to stop after) | stop | status.",
        args: &[
            "action:string:false:start (default) | stop | status",
            "url:string:false:the stream address (start)",
            "seconds:number:false:stop on its own after this long (start)",
        ],
        kind: ToolKind::Ui,
        run: |app, args| match arg_str(args, "action").unwrap_or("start") {
            "start" => {
                let url = req(arg_str(args, "url"), "url")?.to_string();
                let out = start_recording(app, &url, arg_f64(args, "seconds"))?;
                app.io.stream_open = true;
                Ok(ToolOutcome::Done(json!({"ok": true, "file": out.to_string_lossy()})))
            }
            "stop" => match app.io.recording.as_mut() {
                Some(r) => {
                    r.stop();
                    Ok(ToolOutcome::Done(json!({"ok": true, "file": r.out.to_string_lossy()})))
                }
                None => Err("nothing is recording".into()),
            },
            _ => Ok(ToolOutcome::Done(match &app.io.recording {
                Some(r) => json!({
                    "recording": true, "url": r.url, "file": r.out.to_string_lossy(),
                    "seconds": r.started.elapsed().as_secs_f64(), "bytes": r.bytes(), "stopping": r.stopping(), "limit": r.limit,
                }),
                None => json!({"recording": false}),
            })),
        },
    },
    ToolDef {
        name: "gpu.list",
        desc: "The graphics adapters (name, vendor / device id, video memory, Windows' high-performance and power-saving order) and the GPU chosen in Settings (gpu_adapter: \"\" automatic, power, performance or an adapter name; set it with settings.set, it applies after a restart).",
        args: &[],
        kind: ToolKind::Read,
        run: |app, _| {
            let list: Vec<Value> = gpu_pick::adapters()
                .iter()
                .map(|a| json!({"name": a.name, "vendor": a.vendor, "device": a.device, "vram_mb": a.vram_mb, "perf_rank": a.perf_rank, "power_rank": a.power_rank}))
                .collect();
            let choice = gpu_pick::Choice::parse(&app.settings.gpu_adapter);
            Ok(ToolOutcome::Done(json!({"adapters": list, "gpu_adapter": app.settings.gpu_adapter, "choice": choice.label()})))
        },
    },
    ToolDef {
        name: "shuttle.status",
        desc: "The Contour ShuttlePRO / ShuttleXpress: whether it is on (Settings shuttle), the connected model, and what each button runs (shuttle_buttons: Action ids, button 1 first).",
        args: &[],
        kind: ToolKind::Read,
        run: |app, _| {
            Ok(ToolOutcome::Done(json!({
                "enabled": app.settings.shuttle,
                "connected": shuttle::status(),
                "buttons": app.settings.shuttle_buttons,
            })))
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotate_argument_forms() {
        assert_eq!(rotate_arg(&json!({}), Some(90)), Ok(Some(90)), "unchanged when omitted");
        assert_eq!(rotate_arg(&json!({"rotate": "auto"}), Some(90)), Ok(None));
        assert_eq!(rotate_arg(&json!({"rotate": 270}), None), Ok(Some(270)));
        assert_eq!(rotate_arg(&json!({"rotate": "-90"}), None), Ok(Some(270)));
        assert_eq!(rotate_arg(&json!({"rotate": "180°"}), None), Ok(Some(180)));
        assert!(rotate_arg(&json!({"rotate": 45}), None).is_err());
    }

    #[test]
    fn tools_are_registered() {
        for t in [
            "media.properties",
            "media.bitrate",
            "media.set_io",
            "stream.open",
            "stream.record",
            "gpu.list",
            "shuttle.status",
        ] {
            assert!(crate::mcp::tools::find(t).is_some(), "{t}");
        }
        assert!(crate::mcp::tools::leaves_project("stream.open", &json!({})), "network access waits for Allow");
        assert!(crate::mcp::tools::leaves_project("stream.record", &json!({"action": "start"})));
        assert!(!crate::mcp::tools::leaves_project("stream.record", &json!({"action": "status"})));
    }
}
