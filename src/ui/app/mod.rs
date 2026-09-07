//! The application: owns the project, undo stack, settings, player, dockable layout and wires the panels
//! together. Non-blocking windows (Settings, Retime, Export, Save Template, Save Profile, export/convert
//! progress) are plain `egui::Window`s - the editor stays usable while they are open. Also executes MCP
//! tool calls against the live project (one undo step per mutating call).

use crate::engine::export::{self, ExportOptions, Progress};
use crate::engine::gpu::GpuRenderer;
use crate::engine::mixer_fx::BusGraph;
use crate::engine::prerender::PreRender;
use crate::engine::text::TextRasterizer;
use crate::hotkeys::{Action, Hotkeys};
use crate::mcp;
use crate::media::thumbs::ThumbCache;
use crate::media::waveform::WaveformCache;
use crate::media::{self, Backend, Frame};
use crate::model::{
    BlendMode, Clip, ClipKind, Effect, EffectKind, FilterKind, Id, Mask, MaskShape, NodeKind, Project, Scaler,
    ShapeKind, TrackKind, TransitionKind, MIN_CLIP,
};
use crate::playback::Player;
use crate::scripting;
use crate::settings::Settings;
use crate::theme::{self, Palette};
use crate::ui::layout::{self, Layout, Pane};
use crate::ui::tools::Tool;
use crate::ui::{
    autocut_ui, capture_ui, confirm, curves, effects_ui, export_ui, frame_ui, history_ui, import_ui, inspector,
    library, markers_ui, mixer_ui, moodboard_ui, nodes, palette, paste_ui, planner, preview, retime, settings_ui,
    shader_ui, subtitles_ui, timeline, tools, tracking_ui, transitions_ui, DragPayload,
};
use eframe::egui;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// OLE drops deliver no pointer events (winit ignores the drop point), so handle_drops asks the OS for the cursor.
windows::core::link!("user32.dll" "system" fn GetCursorPos(p: *mut windows::Win32::Foundation::POINT) -> windows::core::BOOL);

const PROJECT_EXT: &str = "sedit";
const MEDIA_EXTS: &[&str] = &[
    "mp4", "mov", "mkv", "webm", "avi", "m4v", "wmv", "ts", "m2ts", "mts", "flv", "3gp", "mpg", "mpeg", "gif", "mp3",
    "wav", "m4a", "aac", "flac", "ogg", "opus", "wma", "png", "jpg", "jpeg", "bmp", "webp", "tif", "tiff",
];

mod actions;
mod audio_actions;
// ---- ws:forgiveness ----
mod autosave;
mod boot;
mod caches;
mod drops;
mod edit_ops;
mod feedback;
mod files;
// ---- ws:layout-modes-onboarding ----
mod frame;
// ---- ws:inspector-gallery ----
mod gallery_ctl;
mod gpu;
mod jobs;
// ---- ws:layout-modes-onboarding ----
mod layout_ctl;
// ---- ws:source-monitor ----
// `lib_preview` is removed here - its one caller (the Library pane's small in-panel preview) was
// replaced by the real Source monitor pane; see `source_pane.rs`.
mod library_pane;
mod mcp_exec;
mod media_sync;
mod menus;
// ---- ws:canvas-handles-monitor ----
mod monitor;
// ---- ws:command-palette ----
mod palette_ctl;
mod panes;
mod playback_ctl;
mod preview_pane;
// ---- ws:forgiveness ----
// pub(crate): main.rs calls recovery::install_panic_hook() before eframe::run_native.
pub(crate) mod recovery;
// ---- ws:source-monitor ----
mod source_ctl;
mod source_pane;
// every placement call site (drops, library add, recording import, panes) names a DropMode
pub(crate) use edit_ops::DropMode;
// ---- ws:inspector-gallery ----: `pub(crate)`, not private, so `ui::gallery` (a sibling of `ui::app`,
// not a descendant) can name `thumbs::ThumbSource` - gallery.rs owns the actual thumbnail cache
// (thread-local, mirroring effects_ui.rs's own `THUMBS`), this module only builds the textures.
pub(crate) mod thumbs;
mod timeline_pane;
mod tools_args;
mod tools_audio;
mod tools_clip;
mod tools_color;
// ---- ws:command-palette ----
mod tools_commands;
// ---- ws:export-deliver ----
mod tools_export;
// ---- ws:inspector-gallery ----
mod tools_gallery;
mod tools_helpers;
// ---- ws:layout-modes-onboarding ----
mod tools_layout;
mod tools_media;
mod tools_mixer;
// ---- ws:pro-monitor ----
mod tools_monitor;
mod tools_playback;
// ---- ws:canvas-handles-monitor ----
mod tools_preview;
// ---- ws:forgiveness ----
mod tools_project;
#[cfg(test)]
mod tools_registry_tests;
// ---- ws:source-monitor ----
mod tools_source;
mod tools_subtitles;
mod tools_timeline;
// ---- ws:pro-timeline ----
mod tools_timeline_pro;
// ---- ws:text-titles ----
mod tools_titles;
// ---- ws:transcript-captions ----
mod tools_transcript;
mod tools_trim;
mod tools_ui;
// ---- ws:transcript-captions ----
mod transcript_ctl;
mod trim_actions;
mod whatsnew;
#[path = "windows.rs"]
mod windows_dlg;

enum ExportKind {
    File { path: PathBuf },
    Overwrite { original: PathBuf, temp: PathBuf },
}

/// A blocking MCP tool job (export.video / media.convert): the reply is sent when the job finishes.
struct McpJob {
    prog: Arc<Progress>,
    reply: Sender<Result<Value, String>>,
    out: PathBuf,
}

pub struct App {
    project: Project,
    project_path: Option<PathBuf>,
    dirty: bool,
    undo: Vec<UndoEntry>,
    redo: Vec<UndoEntry>,
    settings: Settings,
    hotkeys: Hotkeys,
    text: Arc<Mutex<TextRasterizer>>,
    fonts: Vec<String>,
    player: Player,
    waveforms: WaveformCache,
    thumbs: ThumbCache,
    layout: Layout,
    /// Last layout JSON written to settings (persist only on change, debounced to gesture end).
    layout_json: String,
    layout_dirty: bool,
    timeline: timeline::TimelineState,
    preview: preview::PreviewState,
    library: library::LibraryState,
    settings_ui: settings_ui::SettingsUi,
    transitions_ui: transitions_ui::TransitionsState,
    curves: curves::CurvesState,
    subtitles_ui: subtitles_ui::SubtitlesState,
    planner: planner::PlannerState,
    moodboard: moodboard_ui::MoodboardState,
    history: history_ui::HistoryState,
    autocut: autocut_ui::AutoCutState,
    tracking: tracking_ui::TrackState,
    retime: retime::RetimeUi,
    export_ui: export_ui::ExportUi,
    /// Some = the "Save Template" / "Save Profile" name windows are open (the String is the name field).
    template_name: Option<String>,
    profile_name: Option<String>,
    fullscreen: bool,
    selection: Vec<Id>,
    /// Selected transitions (timeline bands) - separate from the clip selection.
    sel_transitions: Vec<Id>,
    playhead: f64,
    export: Option<(Arc<Progress>, ExportKind)>,
    encoders: Vec<String>,
    toasts: Vec<feedback::Toast>,
    screenshot: Option<PathBuf>,
    started: Instant,
    /// Window starts hidden (see main.rs); shown once the first frame has been painted.
    window_shown: bool,
    first_frame_at: Option<Instant>,
    screenshot_requested: bool,
    close_confirmed: bool,
    /// Close was requested during an export: cancel it, then re-request the close once it has finished.
    close_after_export: bool,
    was_playing: bool,
    /// Last title sent to the OS - `send_viewport_cmd` forces a repaint, so only send on change.
    last_title: String,
    palette: Palette,
    /// Newest rendered frame, handed to the preview pane when it draws.
    pending_frame: Option<Arc<Frame>>,
    /// Actions requested by panels this frame (transport, context menus, breadcrumb).
    pending_actions: Vec<Action>,
    mcp: Option<(mcp::Server, Receiver<mcp::ToolCall>)>,
    mcp_port_running: u16,
    mcp_jobs: Vec<McpJob>,
    /// Library "Convert To…" jobs: (progress, output path) - polled each frame, imported when done.
    convert_jobs: Vec<(Arc<Progress>, PathBuf)>,
    /// Asset id + target extension for the Convert To… options window.
    convert_dialog: Option<(Id, String)>,
    /// Compress… window state (None = closed).
    compress: Option<Compress>,
    /// A working yt-dlp was found - gates the Library's URL import. Detected on a background thread
    /// (it spawns `yt-dlp --version`) at start-up and again when the setting changes.
    ytdlp_available: Arc<std::sync::atomic::AtomicBool>,
    /// Import-URL window state: (url, audio only).
    url_dialog: Option<(String, bool)>,
    /// Running URL downloads - polled each frame, imported into the library when they finish.
    downloads: Vec<crate::media::ytdlp::Download>,
    /// One receiver per import batch: ffprobe runs on a worker, `poll_probes` adopts the results.
    probes: Vec<Receiver<crate::engine::import::Probed>>,
    /// Was the Auto-cut pane drawn last frame? (its keep-range shading is only valid while it is open).
    /// `autocut_drawing` accumulates this frame; the timeline reads `autocut_shown` so the shading does
    /// not depend on which pane the tile tree draws first.
    autocut_shown: bool,
    autocut_drawing: bool,
    /// Same trick for the Tracking pane: the preview only draws its box while the pane is on screen.
    tracking_shown: bool,
    tracking_drawing: bool,
    /// Fonts already handed to the rasterizer (so we only reload when the list grows).
    loaded_fonts: usize,
    // ---------------- round 3 ----------------
    /// The eframe glow context (None when eframe runs without one) and the renderer it reports.
    gl: Option<Arc<eframe::glow::Context>>,
    gpu_name: String,
    /// GPU renderer, built lazily from `gl` while `settings.gpu` is on; None = CPU compositor.
    gpu: Option<GpuRenderer>,
    /// Effect catalogue thumbnails: the egui textures (kept alive while the panel shows them) and the
    /// key set they were built from, so they are re-rendered only when the stock image or size changes.
    /// GPU frame requests from export threads and movie-mode prerender workers (they decode; we composite
    /// on the GL context) - shared, since both are served identically.
    gpu_export: (
        std::sync::mpsc::Sender<crate::engine::export::GpuFrameRequest>,
        std::sync::mpsc::Receiver<crate::engine::export::GpuFrameRequest>,
    ),
    /// The GPU canvas the preview paints (zero-copy): id + pixel size. Stays valid until the next GPU
    /// render, which is also when it is replaced.
    gpu_tex: Option<(egui::TextureId, [u32; 2])>,
    /// glow texture -> egui id. The renderer's pool reuses a handful of textures, so registering each one
    /// once keeps eframe's texture map small (registering per frame would grow it forever).
    gpu_tex_ids: std::collections::HashMap<eframe::glow::Texture, egui::TextureId>,
    effect_thumbs: Vec<egui::TextureHandle>,
    effect_thumbs_key: Option<(String, u32)>,
    /// Editor background image: (path, blur radius, texture) - reloaded when either key changes.
    bg_tex: Option<(String, u8, egui::TextureHandle)>,
    /// The GPU path failed once - do not retry until the setting is switched off and on again.
    gpu_failed: bool,
    /// The frame the GPU rendered last: its buffer is reused once the preview released it.
    gpu_prev: Option<Arc<Frame>>,
    tools: tools::ToolsState,
    nodes: nodes::NodesState,
    mixer: mixer_ui::MixerState,
    markers: markers_ui::MarkersState,
    buses: BusGraph,
    capture_ui: capture_ui::CaptureUi,
    frame_ui: frame_ui::FrameUi,
    shader_ui: shader_ui::ShaderUi,
    import_ui: import_ui::ImportUi,
    paste_ui: paste_ui::PasteUi,
    /// Running screen recording / voiceover (voiceover remembers the timeline time it started at).
    screen_rec: Option<(crate::engine::capture::Capture, PathBuf)>,
    voice_rec: Option<(crate::engine::capture::Capture, PathBuf, f64)>,
    /// Running Draw take: the drawing every stroke joins, and the timeline time it started at.
    draw_rec: Option<(Id, f64)>,
    /// Viewport focus last frame (record-on-blur watches this).
    was_focused: bool,
    /// Ctrl+Alt+C clipboard for Paste Attributes.
    attrs: Option<Clip>,
    /// Ctrl+C / Ctrl+X clip clipboard - a template (clips + the assets they use), so paste reuses
    /// `Project::place_clips` and its fresh clip / link ids.
    clipboard: Option<crate::settings::Template>,
    /// Text to hand the OS clipboard at the end of the frame. egui-winit only emits `Event::Paste` when
    /// the system clipboard holds text (egui-winit-0.33.3 src/lib.rs:823 returns without pushing the key
    /// event either way), so a Ctrl+V after an internal-only copy produced NO event at all and could
    /// never be bound. Copying clips therefore also writes them out as text.
    os_clipboard: Option<String>,
    // ---- ws:source-monitor ----
    /// The Source monitor (`Pane::Source`, replaces the old `lib_preview` Preview-pane takeover): a
    /// player of its own so it never disturbs the program monitor or the timeline playhead, and the
    /// texture the pane paints this frame.
    source: Option<crate::ui::source_ui::SourceState>,
    source_tex: Option<egui::TextureHandle>,
    /// This update's uploaded frame, computed once (`Player::take_frame` consumes the buffered frame, so
    /// pulling it twice in one update would starve whichever call came second). Both the library pane's
    /// own preview box and the Source pane read this same value.
    source_live: Option<library::PreviewFrame>,
    /// Transport focus: true = Space/JKL/I/O drive the Source monitor (last-clicked transport wins),
    /// false = the timeline, the fallback. See `source_ctl::act`.
    source_focus: bool,
    /// A queued Source-monitor open (needs the egui ctx a new `Player` takes) - see `source_pane::tick`.
    source_pending: Option<source_pane::Pending>,
    /// Movie mode pre-render cache.
    prerender: PreRender,
    /// Movie mode paused the clock because the frame under the playhead was not rendered yet.
    movie_stall: bool,
    /// True while playback is held because the player reported buffering (spinner shown).
    buffer_stall: bool,
    /// A script picked from the Scripts menu, run on the next update (outside menu layout).
    run_script_path: Option<std::path::PathBuf>,
    /// Proxy build in flight: (source path, proxy file, job). One transcode at a time.
    proxy_job: Option<(String, std::path::PathBuf, std::sync::Arc<crate::engine::export::Progress>)>,
    /// source path -> proxy file, as last pushed to the player.
    proxy_map: std::collections::HashMap<String, String>,
    /// Next time the asset list is rescanned for missing proxies.
    proxy_scan_at: Option<Instant>,
    /// Preview canvas size in px, as the pane last reported it (the GPU renders at this size).
    canvas: (u32, u32),
    /// dshow audio inputs, listed once when the Settings / capture windows first need them.
    audio_inputs: Option<Vec<(String, bool)>>,
    /// Panes whose draw panicked: shown as a message instead of taking the whole editor down.
    failed_panes: Vec<Pane>,
    // ---- ws:registries-schema-hooks ----
    // ---- ws:canvas-handles-monitor ----
    // deviation (see PR body): retyped from wave-0b's `Option<AltRenderKind>` no-op placeholder to the
    // real coalescing state (`monitor::AltRenderState`) this workstream builds - anticipated in the
    // plan's own risk table ("wave-0b's alt_render App-field stub type may not match ... First commit
    // retypes that one field if needed - isolated, called out in the PR description").
    /// The monitor's async alt-render pipeline (hover preview of an effect/transition/gallery item) -
    /// see `monitor.rs`'s doc comment.
    pub(crate) alt_render: monitor::AltRenderState,
    // ---- ws:size-diet ----
    /// The "What's New" window (whatsnew.rs) is open - set on a version bump, or by `Action::WhatsNew`.
    pub(crate) whatsnew_open: bool,
    /// winpos's window-rect debounce: (drag/move started at, the rect it saw) while unsettled, `None`
    /// once saved. Owned here so `whatsnew::tick` can thread it into `winpos::tick` every frame.
    pub(crate) winpos_pending: Option<(Instant, [i32; 4])>,
    // ---- ws:forgiveness ----
    // deviation: unlike Settings/Project, this struct had no pre-seeded per-workstream marker section
    // (only ws:registries-schema-hooks/ws:size-diet above) - adding one here, following the same
    // pattern, since a future workstream will need the same treatment this struct's other fields got.
    /// Single-slot Settings snapshot for `Action::UndoSettings` (taken by `settings_snapshot`) -
    /// intentionally one slot, not a stack: a second destructive Settings op before the first is undone
    /// silently drops the first offer (see the PR body's deliberate-simplifications note).
    settings_undo: Option<Settings>,
    /// Non-blocking confirm windows queued by `crate::ui::confirm::ask`/`ask_app`/`ask_discard`,
    /// drained from the thread-local staging queue and drawn by `confirm::draw` (a WINDOW_DRAWER).
    /// `pub(crate)`: `confirm::draw` lives in a SIBLING module (`crate::ui::confirm`, not a descendant
    /// of `app`), so it needs crate-wide access to reach this field directly.
    pub(crate) confirm_active: Vec<confirm::Pending>,
    /// Set by the close-handler's `confirm_discard_then` continuation; `update()`'s top re-sends
    /// `ViewportCommand::Close` once it sees this, since the original close was cancelled to let the
    /// (now non-blocking) confirm window run first.
    pending_close: bool,
    /// Debounced off-thread autosave state (see autosave.rs).
    autosave: autosave::AutosaveState,
    /// `Action::RestoreBackup` opened the "Restore Autosave…" window (recovery.rs's `restore_window`).
    restore_backup_open: bool,
    // ---- ws:player-rate-loop ----
    /// Timeline seconds a Play In->Out / Play Around / Play to Out should auto-pause at; cleared once
    /// reached (or if playback stops some other way). See `playback_ctl::tick`.
    play_stop_at: Option<f64>,
    /// `playhead` as of the last `playback_ctl::tick` - lets the paused-playhead-change scrub fire once
    /// per change instead of every frame.
    scrub_last_t: f64,
    // ---- ws:command-palette ----
    /// Ctrl+K palette state. Named `cmd_palette`, not `palette` - `App.palette` is already the live
    /// theme `Palette` (`self.palette` is read constantly for colours throughout `ui::app`), so reusing
    /// that name for the command palette would shadow/collide with it everywhere.
    cmd_palette: palette::PaletteState,
    /// F1 cheat-sheet overlay open/closed.
    cheat_sheet_open: bool,
    /// `scripting::list()` + `scripting::meta()` for every script, refreshed at 1 Hz by `palette_ctl::
    /// tick` (re-parsing every script's header on every frame would be silly - see `App::script_metas`).
    script_meta_cache: (Instant, Vec<scripting::ScriptMeta>),
    /// Re-entrancy guard for `App::fire_hook`: true while a hook is already running, so a hook that
    /// itself calls `editor.tool`/triggers another hook-firing event can't recurse.
    hook_running: bool,
    /// Scripts disabled for the session after their `@on` hook overran its budget once (one toast, then
    /// silently skipped by `fire_hook` for the rest of the session).
    disabled_hooks: Vec<PathBuf>,
    /// Selection signature last handed to `fire_hook("selection_changed", ...)` - `palette_ctl::tick`
    /// compares against `frame::SelSig::of(self)` each frame so the hook fires on any change (clips,
    /// transitions, subtitle cues OR the edit point - not just `self.selection`), exactly once.
    last_fired_selection: frame::SelSig,
    // ---- ws:layout-modes-onboarding ----
    /// The first-run welcome wizard while it is open - armed by `boot::run` on a fresh install (no
    /// file argument, no `--screenshot`), `Action::ShowWelcome` and the `onboarding.reset` tool.
    onboarding: Option<crate::ui::onboarding::Onboarding>,
    /// The home / empty-state cards were dismissed for this session (`ui::home`).
    home_dismissed: bool,
    /// The selection `frame::tick` last reacted to, so auto-surface / glow fire once per change.
    sel_sig: frame::SelSig,
    // ---- ws:export-deliver ----
    // (same per-workstream section shape ws:forgiveness added above - this struct's pre-seeded markers
    // stop at ws:size-diet, so each later workstream appends its own)
    /// Render queue: exports waiting for the single `export` slot, popped in order by
    /// `tools_export::frame_tick` once it is free (and no bake is running).
    export_queue: std::collections::VecDeque<export_ui::ExportChoice>,
    /// In-flight bakes (render in place / stabilize / denoise / slow-mo) - at most one, stepped by
    /// `tools_export::frame_tick`; drawn by `windows()`'s "Rendering in place" job window.
    bake_jobs: Vec<tools_export::BakeJob>,
    // ---- ws:inspector-gallery ----
    /// Textures backing the Gallery's Looks/LUTs cards (kept alive while the pane shows them) - the
    /// (source, key) -> id/size cache itself is `gallery.rs`'s own thread-local, same convention as
    /// `effects_ui.rs`'s `THUMBS`, so `ui::gallery::show` (a sibling module, not a descendant of
    /// `ui::app`) can read it without needing an `&App` reference.
    gallery_textures: Vec<egui::TextureHandle>,
    /// Gallery pane state (open tab, save-from-selection scratch).
    gallery: crate::ui::gallery::GalleryState,
    // ---- ws:media-library ----
    /// Image-sequence bakes and consolidate copies in flight, polled by `media_sync::tick`.
    media_jobs: Vec<media_sync::MediaJob>,
    /// Next time `media_sync::tick` rescans the assets for missing files (2 s cadence, like proxies).
    /// The set itself lives in `library.offline` - the one copy `App::asset_status` and the library
    /// rows both read.
    offline_scan_at: Option<Instant>,
    // ---- ws:transcript-captions ----
    /// Background whisper / tracking / TTS jobs started outside the Subtitles pane (clip menu, MCP),
    /// the clip-menu model download and the "View transcript" window - see transcript_ctl.rs.
    transcript: transcript_ctl::TranscriptState,
    // ---- ws:pro-monitor ----
    /// Dynamic-trim arming, the dual-frame trim view's decode slots, Scopes open/closed and the
    /// eyedropper's armed (clip, target) - see `monitor.rs`'s `MonitorState` doc comment.
    monitor: monitor::MonitorState,
    // ---- ws:pro-timeline ----
    /// Ctrl+F Find window state (open/closed, query buffer).
    find: crate::ui::find_ui::FindState,
}

// ---- ws:canvas-handles-monitor ----
// wave-0b's `AltRenderKind` placeholder enum (Hover/TrimView/Scopes/Wipe) is superseded by
// `monitor::AltRequest` (Effect/Transition/Gallery - pro-monitor, wave 3, adds TrimOut/TrimIn/
// Compare/Angle to that same enum per the plan) and removed here to avoid two parallel "what should
// the monitor render" types.

// ---- ws:forgiveness ----
/// The on-disk cache directory's size - `settings_ui::performance` (a sibling module, not a descendant
/// of `app`, so it can't reach `caches::cache_bytes` directly) reads this for its "Clear Caches" row.
pub fn caches_bytes_for_ui() -> u64 {
    caches::cache_bytes()
}

/// Non-blocking progress window for background jobs (conversions, downloads): one row per job with a
/// progress bar and Cancel. Draws nothing when there are no jobs.
fn job_window(ctx: &egui::Context, title: &str, jobs: &[(Arc<Progress>, String)]) {
    if jobs.is_empty() {
        return;
    }
    egui::Window::new(title).resizable(false).default_width(320.0).show(ctx, |ui| {
        for (prog, name) in jobs {
            ui.label(egui::RichText::new(name).small());
            ui.add(egui::ProgressBar::new(prog.fraction()).show_percentage().text(prog.status()));
            if ui.button("Cancel").clicked() {
                prog.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
    });
    ctx.request_repaint_after(Duration::from_millis(150));
}

/// Marker in the undo stack for "a pane was dragged somewhere else". The arrangement itself lives in
/// `Layout`'s own (much shorter) history - this only keeps Ctrl+Z stepping back in the right order.
/// ponytail: once the layout history has scrolled past its 20 entries the marker undoes nothing; deepen
/// the layout stack if that ever bites.
pub(crate) const LAYOUT_STEP: &str = "\u{0}layout";

/// History panel filter bucket. `Layout` is a pane rearrangement (`LAYOUT_STEP`); everything else -
/// clip/effect/marker/text/project edits - is `Editing`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HistoryCategory {
    Editing,
    Layout,
}

/// One entry in the undo/redo stack, doubling as a History panel row. `label` stays EMPTY for project
/// edits - the History panel derives one lazily from neighbouring snapshots (`describe_change`), which
/// keeps the per-gesture push free of JSON parses and labels each row with its own edit instead of the
/// previous one. Only sentinel entries (layout steps) carry a fixed label.
#[derive(Clone)]
pub(crate) struct UndoEntry {
    pub json: String,
    pub label: String,
    /// Seconds since Unix epoch (`SystemTime`, not `Instant` - a History panel needs a real clock to
    /// group by day and survive across app restarts... though the stack itself is session-only today;
    /// kept as a real timestamp anyway since "session-only" is the smaller, more surprising fact here).
    pub at: f64,
    pub category: HistoryCategory,
}

fn now_secs() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// A short, best-effort description of what changed between two project snapshots - compares a handful
/// of high-signal counts/fields rather than a full structural diff (this is a "quick glance" History
/// panel label, not a changelog). Falls back to "Project edited" when nothing tracked here differs.
/// Costs two full `Project::from_json` parses - only the History panel calls it (lazily, cached),
/// NEVER the per-gesture undo push.
pub(crate) fn describe_change(old_json: &str, new_json: &str) -> String {
    let (Ok(old), Ok(new)) = (Project::from_json(old_json), Project::from_json(new_json)) else {
        return "Project edited".into();
    };
    let clips = |p: &Project| p.tracks.iter().map(|t| t.clips.len()).sum::<usize>();
    let effects = |p: &Project| p.tracks.iter().flat_map(|t| &t.clips).map(|c| c.effects.len()).sum::<usize>();
    let (oc, nc) = (clips(&old), clips(&new));
    if oc != nc {
        return match nc.cmp(&oc) {
            std::cmp::Ordering::Greater if nc - oc == 1 => "Added a clip".into(),
            std::cmp::Ordering::Greater => format!("Added {} clips", nc - oc),
            std::cmp::Ordering::Less if oc - nc == 1 => "Removed a clip".into(),
            _ => format!("Removed {} clips", oc - nc),
        };
    }
    if old.width != new.width || old.height != new.height {
        return "Changed project resolution".into();
    }
    if (old.fps - new.fps).abs() > f64::EPSILON {
        return "Changed project frame rate".into();
    }
    if old.markers.len() != new.markers.len() {
        return "Edited markers".into();
    }
    if old.notes.len() != new.notes.len() {
        return "Edited notes".into();
    }
    if old.plan.len() != new.plan.len() {
        return "Edited the planner".into();
    }
    if old.moodboard.len() != new.moodboard.len() {
        return "Edited the moodboard".into();
    }
    let (oe, ne) = (effects(&old), effects(&new));
    if oe != ne {
        return "Edited effects".into();
    }
    if old.name != new.name {
        return "Renamed the project".into();
    }
    "Project edited".into()
}

/// Push an undo snapshot (capped) and clear the redo history. Labels are NOT derived here - that cost
/// (two project parses) belongs to the History panel, lazily; see `UndoEntry::label`.
fn push_undo_json(undo: &mut Vec<UndoEntry>, redo: &mut Vec<UndoEntry>, json: String) {
    let entry = if json == LAYOUT_STEP {
        UndoEntry { label: "Rearranged panels".into(), category: HistoryCategory::Layout, at: now_secs(), json }
    } else {
        UndoEntry { json, label: String::new(), category: HistoryCategory::Editing, at: now_secs() }
    };
    undo.push(entry);
    if undo.len() > 200 {
        undo.remove(0);
    }
    redo.clear();
}

/// Moved/renamed sources: re-point assets to `project_dir/<file name>` when that exists (a silent black
/// preview is the alternative); returns the paths that are still missing.
fn relocate_assets(project: &mut Project, project_dir: Option<&Path>) -> Vec<String> {
    let mut missing = Vec::new();
    for a in &mut project.assets {
        if Path::new(&a.path).exists() {
            continue;
        }
        let alt = Path::new(&a.path).file_name().and_then(|n| project_dir.map(|d| d.join(n)));
        match alt.filter(|p| p.exists()) {
            Some(p) => a.path = p.to_string_lossy().into_owned(),
            None => missing.push(a.path.clone()),
        }
    }
    missing
}

/// Run something that may panic (GPU driver, pre-render, a panel widget) without taking the editor
/// down - same policy as the decoder threads. None = it panicked.
/// ponytail: the panic message goes to the default hook (stderr); the caller toasts and degrades.
fn guarded<T>(f: impl FnOnce() -> T) -> Option<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).ok()
}

/// Give the clip's last effect a mask, or the clip itself when it has no effects. False = there is one
/// already, or the clip is audio (a mask shapes pixels, and audio has none).
fn preview_canvas(canvas: (u32, u32), quality: u32) -> (u32, u32) {
    if canvas.0 == 0 || canvas.1 == 0 {
        return (0, 0);
    }
    let q = quality.clamp(25, 100) as f32 / 100.0;
    (((canvas.0 as f32 * q) as u32).max(16), ((canvas.1 as f32 * q) as u32).max(16))
}

/// `preview_max_width` applied to a canvas size, keeping the aspect ratio. Must match
/// `Player::set_canvas`, or the GPU renders at a different shape than the player decodes at.
fn clamp_canvas(w: u32, h: u32, max_width: u32) -> (u32, u32) {
    if max_width > 0 && w > max_width {
        (max_width, ((h as u64 * max_width as u64) / w.max(1) as u64).max(1) as u32)
    } else {
        (w, h)
    }
}

/// Render size for an image export: downscales are rendered straight at the target (the compositor's
/// `Scaler` does the filtering), upscales are rendered at project size and enlarged by ffmpeg with the
/// chosen resize flag - rendering a 4K frame from a 1080p timeline gains nothing but time.
fn frame_render_size(project: (u32, u32), target: (u32, u32)) -> (u32, u32) {
    let (pw, ph) = (project.0.max(16), project.1.max(16));
    let (tw, th) = (target.0.max(16), target.1.max(16));
    if tw <= pw && th <= ph {
        (tw, th)
    } else {
        (pw, ph)
    }
}

// TODO(ui-panels-fx): call these from `effects_ui::set_thumbnail(kind, ...)` once that hook exists -
// the app renders each kind once on the GPU from this source and hands the result over.
#[allow(dead_code)]
/// Cache key of one effect thumbnail: kind, source image and size. Changing the stock image (or the
/// grid size) invalidates every thumbnail; two different effects never share a key.
fn timeline_is_empty(p: &Project) -> bool {
    p.is_empty() && p.main_stash.as_ref().is_none_or(|s| s.tracks.iter().all(|t| t.clips.is_empty()))
}

/// Compress… window state: what to shrink, how hard, and where the result goes.
struct Compress {
    src: PathBuf,
    /// false = quality (CRF), true = size target.
    by_size: bool,
    crf: u32,
    target_mb: f64,
    overwrite: bool,
    source_bytes: Option<u64>,
    duration: Option<f64>,
}

impl Compress {
    fn new(src: PathBuf, crf: u32) -> Self {
        let source_bytes = std::fs::metadata(&src).ok().map(|m| m.len());
        let duration = crate::engine::convert::probe_seconds(&src);
        // default target: half the current size, which is what "compress this" usually means
        let target_mb = source_bytes.map_or(10.0, |b| (b as f64 / 2e6).max(0.1));
        Self { src, by_size: false, crf: crf.max(23), target_mb, overwrite: false, source_bytes, duration }
    }
}

/// Where "Convert To…" writes: `<stem>_converted.<ext>` next to the source, uniquified so a convert can
/// never overwrite the source itself or a file already on disk (possibly one the timeline is using).
fn converted_path(src: &Path, ext: &str) -> PathBuf {
    let stem = src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "output".into());
    let mut out = src.with_file_name(format!("{stem}_converted.{ext}"));
    let mut n = 2;
    while out.exists() {
        out = src.with_file_name(format!("{stem}_converted_{n}.{ext}"));
        n += 1;
    }
    out
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, open: Option<PathBuf>, screenshot: Option<PathBuf>) -> Self {
        // eframe restores the window rect from the last session, which may be on another monitor
        crate::winpos::place_on_cursor_monitor(cc);
        let (settings, settings_bad) = Settings::load_reporting();
        media::ffpipe::set_dir(&settings.ffmpeg_dir);
        media::ytdlp::set_dir(&settings.ytdlp_dir);
        theme::apply(&cc.egui_ctx, &settings.theme, &settings.palette, &settings.ui_look);
        let backend = Backend::parse(&settings.decoder);
        let text = Arc::new(Mutex::new(TextRasterizer::new()));
        {
            // warm the font list off-thread so the first text clip / inspector doesn't hitch
            let t = text.clone();
            std::thread::spawn(move || {
                if let Ok(mut t) = t.lock() {
                    t.load_system_fonts();
                }
            });
        }
        // ---- ws:layout-modes-onboarding ----
        // The Explorer context-menu install that used to run here on every launch (guarded on
        // settings.context_menu / no --screenshot / release build / not yet installed) now lives in
        // `boot::run`: a first run arms the welcome wizard, whose opt-in checkbox is the consent that
        // was missing; later launches keep the same guard, gated on that recorded consent.
        let mut player = Player::new(cc.egui_ctx.clone(), backend, text.clone());
        player.set_cache_bytes(crate::playback::cache_budget_bytes(settings.cache_mb));
        let waveforms = WaveformCache::new(cc.egui_ctx.clone(), backend);
        let thumbs = ThumbCache::new(cc.egui_ctx.clone(), backend);
        let hotkeys = Hotkeys::from_settings(&settings);
        let palette = theme::palette_with(&cc.egui_ctx, &settings.palette);
        // GL belongs to this (UI) thread; the renderer itself is built on first use so a driver that
        // rejects the shaders only costs a toast.
        let gl = cc.gl.clone();
        let gpu_name = gl
            .as_ref()
            .map(|gl| {
                use eframe::glow::HasContext;
                unsafe { gl.get_parameter_string(eframe::glow::RENDERER) }
            })
            .unwrap_or_else(|| "no OpenGL context".into());
        // an old layout profile has no Tools / Nodes / Mixer / Markers pane: reset to the new default
        let layout = Layout::from_json(&settings.layout).unwrap_or_default();
        let layout_json = layout.to_json();
        let mut app = Self {
            project: Project::new(),
            project_path: None,
            dirty: false,
            undo: Vec::new(),
            redo: Vec::new(),
            settings,
            hotkeys,
            text,
            fonts: Vec::new(),
            player,
            waveforms,
            thumbs,
            layout,
            layout_json,
            layout_dirty: false,
            timeline: timeline::TimelineState::default(),
            preview: preview::PreviewState::default(),
            library: library::LibraryState::default(),
            settings_ui: settings_ui::SettingsUi::default(),
            transitions_ui: transitions_ui::TransitionsState::default(),
            curves: curves::CurvesState::default(),
            subtitles_ui: subtitles_ui::SubtitlesState::default(),
            planner: planner::PlannerState::default(),
            moodboard: moodboard_ui::MoodboardState::default(),
            history: history_ui::HistoryState::default(),
            autocut: autocut_ui::AutoCutState::default(),
            tracking: tracking_ui::TrackState::default(),
            retime: retime::RetimeUi::default(),
            export_ui: export_ui::ExportUi::default(),
            template_name: None,
            profile_name: None,
            fullscreen: false,
            selection: Vec::new(),
            sel_transitions: Vec::new(),
            playhead: 0.0,
            export: None,
            encoders: Vec::new(),
            toasts: Vec::new(),
            screenshot,
            started: Instant::now(),
            window_shown: false,
            first_frame_at: None,
            screenshot_requested: false,
            close_confirmed: false,
            close_after_export: false,
            was_playing: false,
            last_title: String::new(),
            palette,
            pending_frame: None,
            pending_actions: Vec::new(),
            mcp: None,
            mcp_port_running: 0,
            mcp_jobs: Vec::new(),
            convert_jobs: Vec::new(),
            convert_dialog: None,
            compress: None,
            ytdlp_available: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            url_dialog: None,
            downloads: Vec::new(),
            probes: Vec::new(),
            autocut_shown: false,
            autocut_drawing: false,
            tracking_shown: false,
            tracking_drawing: false,
            loaded_fonts: 0,
            gl,
            gpu_name,
            gpu: None,
            gpu_export: std::sync::mpsc::channel(),
            gpu_tex: None,
            gpu_tex_ids: std::collections::HashMap::new(),
            effect_thumbs: Vec::new(),
            effect_thumbs_key: None,
            bg_tex: None,
            gpu_failed: false,
            gpu_prev: None,
            tools: tools::ToolsState::default(),
            nodes: nodes::NodesState::default(),
            mixer: mixer_ui::MixerState::default(),
            markers: markers_ui::MarkersState::default(),
            buses: BusGraph::new(),
            capture_ui: capture_ui::CaptureUi::default(),
            frame_ui: frame_ui::FrameUi::default(),
            shader_ui: shader_ui::ShaderUi::default(),
            import_ui: import_ui::ImportUi::default(),
            paste_ui: paste_ui::PasteUi::default(),
            screen_rec: None,
            voice_rec: None,
            draw_rec: None,
            was_focused: true,
            attrs: None,
            clipboard: None,
            os_clipboard: None,
            // ---- ws:source-monitor ----
            source: None,
            source_live: None,
            source_tex: None,
            source_focus: false,
            source_pending: None,
            prerender: PreRender::new(),
            movie_stall: false,
            buffer_stall: false,
            run_script_path: None,
            proxy_job: None,
            proxy_map: std::collections::HashMap::new(),
            proxy_scan_at: None,
            canvas: (0, 0),
            audio_inputs: None,
            failed_panes: Vec::new(),
            alt_render: monitor::AltRenderState::default(),
            whatsnew_open: false,
            winpos_pending: None,
            // ---- ws:forgiveness ----
            settings_undo: None,
            confirm_active: Vec::new(),
            pending_close: false,
            autosave: autosave::AutosaveState::default(),
            restore_backup_open: false,
            play_stop_at: None,
            scrub_last_t: 0.0,
            cmd_palette: palette::PaletteState::default(),
            cheat_sheet_open: false,
            // already-expired so `palette_ctl::tick`'s 1 Hz refresh runs on the very first frame instead
            // of leaving the Scripts menu/palette empty of scripts for a whole second after startup
            script_meta_cache: (
                Instant::now().checked_sub(Duration::from_secs(2)).unwrap_or_else(Instant::now),
                Vec::new(),
            ),
            hook_running: false,
            disabled_hooks: Vec::new(),
            last_fired_selection: frame::SelSig::default(),
            // ---- ws:layout-modes-onboarding ----
            onboarding: None,
            home_dismissed: false,
            sel_sig: frame::SelSig::default(),
            // ---- ws:export-deliver ----
            export_queue: std::collections::VecDeque::new(),
            bake_jobs: Vec::new(),
            // ---- ws:inspector-gallery ----
            gallery_textures: Vec::new(),
            gallery: crate::ui::gallery::GalleryState::default(),
            // ---- ws:media-library ----
            media_jobs: Vec::new(),
            offline_scan_at: None,
            // ---- ws:transcript-captions ----
            transcript: transcript_ctl::TranscriptState::default(),
            // ---- ws:pro-monitor ----
            monitor: monitor::MonitorState::default(),
            // ---- ws:pro-timeline ----
            find: crate::ui::find_ui::FindState::default(),
        };
        if let Some(reason) = settings_bad {
            app.toast(format!("Settings file was corrupt (saved as settings.json.bad): {reason}"));
        }
        app.detect_ytdlp(&cc.egui_ctx);
        app.refresh_presets();
        app.player.set_project(&app.project);
        if let Some(p) = open {
            app.open_path(&p);
            // launched from Explorer ("Open with"): behave like a player - full screen, rolling
            if app.screenshot.is_none() && !app.project.tracks.iter().all(|t| t.clips.is_empty()) {
                app.fullscreen = true;
                app.player.play();
            }
        }
        app
    }

    // ---------------- helpers ----------------
    // toast()/toast_with_folder()/push_toast()/toast_undo() moved to feedback.rs (ws:forgiveness).

    fn push_undo(&mut self) {
        push_undo_json(&mut self.undo, &mut self.redo, self.project.to_json());
    }

    // ws:source-monitor: `insert_at` (chain each asset's clips end to end) is now
    // `place_assets(.., DropMode::Place)` in edit_ops.rs - every former caller names its DropMode.

    /// Empty project + one media file: open it as the project (returns empty); otherwise import into the library.
    /// ponytail: that single file is still probed on this thread - it settles the project format, size
    /// and zoom before anything is drawn; give it a placeholder too if opening ever feels slow.
    fn open_or_import(&mut self, paths: &[PathBuf]) -> Vec<Id> {
        // ---- ws:media-library ----: a frame of a numbered still run bakes as one clip instead
        let paths = media_sync::intercept_sequences(self, paths);
        if self.project.is_empty() && self.project.assets.is_empty() && paths.len() == 1 {
            self.open_media(&paths[0]);
            return Vec::new();
        }
        self.import_files(&paths)
    }

    /// After any project mutation.
    fn after_edit(&mut self) {
        self.dirty = true;
        let p = &self.project;
        self.selection.retain(|id| p.clip(*id).is_some());
        self.player.set_project(&self.project);
        if self.settings.movie_mode {
            // the picture changed: drop what was pre-rendered and render the range again
            let end = self.project.duration();
            let App { prerender, .. } = self;
            guarded(|| prerender.invalidate(0.0, end));
            self.request_prerender();
        }
    }

    fn set_project(&mut self, project: Project, path: Option<PathBuf>) {
        self.probes.clear(); // import probes belong to the project that started them
        self.project = project;
        self.project_path = path;
        self.dirty = false;
        self.undo.clear();
        self.redo.clear();
        self.selection.clear();
        self.playhead = 0.0;
        self.player.pause();
        self.player.set_project(&self.project);
        self.player.seek(0.0);
        self.timeline.zoom_to_fit(self.project.duration(), self.timeline.lanes_rect.width().max(800.0));
    }

    fn title(&self) -> String {
        let name = self
            .project_path
            .as_ref()
            .map(|p| p.file_name().unwrap_or_default().to_string_lossy().into_owned())
            .unwrap_or_else(|| self.project.name.clone());
        format!("{}{} - Simple Editor", if self.dirty { "*" } else { "" }, name)
    }

    fn seek(&mut self, t: f64) {
        self.playhead = t.clamp(0.0, self.project.duration().max(0.0));
        self.player.seek(self.playhead);
        // an explicit seek always brings the playhead back into view (a user pan only suspends the
        // follow while playing)
        self.timeline.follow_playhead(self.playhead);
    }

    fn backend(&self) -> Backend {
        Backend::parse(&self.settings.decoder)
    }

    fn timeline_is_empty(&self) -> bool {
        timeline_is_empty(&self.project)
    }

    /// The project with any open sequence closed - exports always render the MAIN timeline.
    fn export_project(&self) -> Project {
        let mut p = self.project.clone();
        if p.editing.is_some() {
            p.close_sequence();
        }
        p
    }

    // ---------------- file operations ----------------

    /// Import media files into the library. Probing spawns ffprobe per file (~100 ms), so each path
    /// lands as a placeholder asset now and `poll_probes` folds in the real metadata a few frames
    /// later - dropping ten files costs this thread nothing. Returns the asset ids.
    /// ponytail: an MCP `media.import` reply therefore quotes duration 0 until the probe lands;
    /// blocking the tool call on it is the fix if an agent ever needs the number in the same reply.
    fn import_files(&mut self, paths: &[PathBuf]) -> Vec<Id> {
        // ---- ws:media-library ----: every import path (Ctrl+I, drops, MCP media.import) funnels
        // through here, so this one gate covers them all - see media_sync::intercept_sequences
        let paths = media_sync::intercept_sequences(self, paths);
        let mut ids = Vec::new();
        let mut fresh: Vec<(Id, String)> = Vec::new();
        for path in &paths {
            let p = path.to_string_lossy().into_owned();
            // a re-import of a file already in the library must not re-probe it: adopting the result
            // would rebuild clips the user has since trimmed
            if let Some(a) = self.project.asset_by_path(&p) {
                ids.push(a.id);
                continue;
            }
            if fresh.is_empty() {
                self.push_undo();
            }
            let id = self.project.add_asset(crate::engine::import::placeholder(&p));
            ids.push(id);
            fresh.push((id, p));
        }
        if !fresh.is_empty() {
            self.probes.push(crate::engine::import::probe_async(fresh, self.backend()));
            self.after_edit();
        }
        ids
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if !self.window_shown {
            // viewport commands apply after this frame is painted, so no white flash
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            self.window_shown = true;
            boot::run(self); // ws:forgiveness: offers crash recovery, if any
        }
        // ws:forgiveness: confirm_discard_then's continuation sets this once the (non-blocking) discard
        // prompt resolves - the original close was cancelled below to let that prompt run, so re-send it.
        if std::mem::take(&mut self.pending_close) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.palette = theme::palette_with(ctx, &self.settings.palette);
        let cozy_look = self.settings.ui_look != "sharp";
        self.palette.rounding = if cozy_look { 6.0 } else { 2.0 };
        self.palette.clip_rounding = if cozy_look { 5.0 } else { 0.0 };
        if !self.settings.bg_image.is_empty() && self.settings.panel_opacity < 255 {
            let a = self.settings.panel_opacity;
            let al = |c: egui::Color32| egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a);
            self.palette.bg = al(self.palette.bg);
            self.palette.panel = al(self.palette.panel);
            self.palette.header = al(self.palette.header);
        }
        if self.fonts.is_empty() {
            if let Ok(t) = self.text.try_lock() {
                if t.is_loaded() {
                    self.fonts = t.families().to_vec();
                }
            }
        }
        // ---- ws:registries-schema-hooks ----
        // A future workstream's per-frame hook (autosave tick, playback tick, ...) is a FRAME_HOOKS
        // entry instead of a line added here. Nothing is registered yet.
        for f in FRAME_HOOKS {
            f(self, ctx);
        }
        self.poll_panels();
        self.poll_probes(ctx);
        self.build_effect_thumbnails(ctx);
        if self.serve_gpu_exports() || self.export.is_some() {
            // a GPU export needs this thread to keep coming back to serve its frames
            ctx.request_repaint();
        }
        // carry last frame's "the Auto-cut pane was on screen" into this frame's timeline drawing
        self.autocut_shown = self.autocut_drawing;
        self.autocut_drawing = false;
        self.tracking_shown = self.tracking_drawing;
        self.tracking_drawing = false;

        // close handling: confirm unsaved changes
        if ctx.input(|i| i.viewport().close_requested()) && !self.close_confirmed {
            if let Some((prog, _)) = &self.export {
                // let the export thread stop and clean up first; the close is re-requested once it has finished
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                prog.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
                self.close_after_export = true;
            } else if self.dirty && self.screenshot.is_none() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                // non-blocking now: the Close re-send happens at the top of `update` once this resolves
                // (see `pending_close` above), not synchronously here.
                self.confirm_discard_then(|app| {
                    app.close_confirmed = true;
                    app.pending_close = true;
                });
            } else {
                self.close_confirmed = true;
            }
        }

        // live links (paths / expressions) re-bake when their inputs changed; a hash check otherwise
        self.project.refresh_links();
        // playback clock (one extra read after it stops, so the playhead lands on the final time)
        let playing = self.player.is_playing();
        if playing || self.was_playing {
            self.playhead = self.player.time();
            self.timeline.ensure_visible(self.playhead);
            // a numeric field left focused before play would see its bound value move every frame and
            // report changed(), silently recording keyframes at the moving playhead - drop focus once
            // when playback starts (not every frame, so text can still be typed mid-playback)
            if playing && !self.was_playing {
                ctx.memory_mut(|m| m.stop_text_input());
            }
            ctx.request_repaint_after(Duration::from_millis(16));
        }
        // a Draw take runs until the video stops or the tool is put away - not one stroke at a time
        if self.draw_rec.is_some() && (self.tools.tool != Tool::Draw || (self.was_playing && !playing)) {
            self.tools.recording = false;
            self.toggle_draw_recording(false);
        }
        self.was_playing = playing;
        // GPU on: the player hands over decoded layers and we render them here (this thread owns GL);
        // GPU off / unavailable: the render thread already composited the frame on the CPU.
        self.sync_gpu();
        // leave the layers in place until the preview pane has reported its size (first frame), so the
        // very first decode is not thrown away
        if self.canvas.0 > 0 && self.canvas.1 > 0 {
            if let Some(layers) = self.player.take_layers() {
                let (w, h) = self.canvas;
                let t = self.player.time();
                // zero copy: render into a GL texture and let egui paint it directly. Only when nothing
                // else needs the pixels on the CPU (movie mode reads from its own cache).
                if let Some(tex) = self.gpu_preview_texture(&layers, t, w, h, _frame) {
                    self.gpu_tex = Some(tex);
                    self.pending_frame = None;
                } else if let Some(f) = self.gpu_frame(&layers, t, w, h) {
                    self.pending_frame = Some(f);
                }
            }
        }
        if let Some(f) = self.player.take_frame() {
            self.pending_frame = Some(f);
        }
        if self.pending_frame.is_some() && self.first_frame_at.is_none() {
            self.first_frame_at = Some(Instant::now());
            #[cfg(debug_assertions)]
            eprintln!("first frame after {} ms", self.started.elapsed().as_millis());
        }
        // movie mode: keep rendering the requested range in small slices and show what is ready
        if self.settings.movie_mode {
            let t = self.playhead;
            let gpu_tx = self.gpu.is_some().then(|| self.gpu_export.0.clone());
            let App { prerender, project, .. } = self;
            match guarded(|| (prerender.tick(project, 4.0, gpu_tx), prerender.frame(project, t))) {
                Some((busy, ready)) => {
                    // movie mode plays every frame at the project rate: rather than let the wall clock
                    // run past a second that is not rendered yet, hold it and resume when it lands.
                    match ready {
                        Some(f) => {
                            self.pending_frame = Some(f);
                            if self.movie_stall {
                                self.movie_stall = false;
                                self.player.play();
                            }
                        }
                        None if self.player.is_playing() => {
                            self.movie_stall = true;
                            self.player.pause();
                        }
                        None => {}
                    }
                    if busy || self.movie_stall {
                        ctx.request_repaint_after(Duration::from_millis(16));
                    }
                }
                None => {
                    self.settings.movie_mode = false;
                    self.toast("Movie mode is not available in this build");
                }
            }
        }
        // buffering: the render thread fell behind decode - hold the clock (the audio ring flushes
        // with the pause) and show a spinner until the read-ahead refills, instead of letting audio
        // play on over a frozen frame. Same shape as the movie-mode stall above.
        if self.player.is_buffering() {
            if !self.buffer_stall && self.player.is_playing() {
                self.buffer_stall = true;
                self.player.pause();
            }
            ctx.request_repaint_after(Duration::from_millis(50)); // keep polling for the refill
        } else if self.buffer_stall {
            self.buffer_stall = false;
            self.player.play();
        }
        // record-on-blur: start when the editor loses focus, stop (and import) when it comes back
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        if self.settings.capture_on_blur && self.capture_ui.screen_open {
            if self.was_focused && !focused && self.screen_rec.is_none() {
                let opts = self.blur_capture_options();
                self.start_screen_capture(opts);
            } else if !self.was_focused && focused && self.screen_rec.is_some() {
                self.stop_screen_capture();
            }
        }
        self.was_focused = focused;
        if self.screen_rec.is_some() || self.voice_rec.is_some() {
            let c = self.screen_rec.as_ref().map(|(c, _)| c).or(self.voice_rec.as_ref().map(|(c, _, _)| c));
            self.capture_ui.elapsed = c.and_then(|c| guarded(|| c.elapsed())).unwrap_or(0.0);
            ctx.request_repaint_after(Duration::from_millis(250));
        }

        // export progress
        if let Some((prog, _)) = &self.export {
            if prog.is_done() {
                self.finish_export();
                if std::mem::take(&mut self.close_after_export) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
        }

        if let Some(p) = self.run_script_path.take() {
            self.run_script(&p);
        }
        self.sync_proxies();
        // MCP server + queued tool calls (executed here, on the UI thread, against the live project)
        self.sync_mcp(ctx);
        self.poll_mcp(ctx);

        self.handle_drops(ctx);

        // the planner's timer ticks HERE, every frame, so a countdown keeps counting, banks time onto
        // its linked task, and notifies even while the Timer tab is hidden behind a sibling tab
        {
            let (banked, finished) = planner::tick(&mut self.planner, &mut self.project);
            if banked {
                // mark unsaved without the cost of a full after_edit() (undo entry, player refresh)
                self.dirty = true;
            }
            if finished {
                self.toast("Timer finished - time to stop");
            }
            if self.planner.timer.running {
                ctx.request_repaint_after(std::time::Duration::from_millis(200));
            }
        }
        // cleared so a frame where the Moodboard tab isn't the one actually drawn (a sibling tab in its
        // group is active instead) can't have next frame's handle_drops match a stale rect from the last
        // time it *was* drawn - `moodboard_ui::show` sets this back whenever it actually runs
        self.moodboard.content_rect = egui::Rect::NOTHING;
        self.screenshot_tick(ctx);

        // hotkeys
        // the tool strip claims the bare letters (V/T/D/M, Shift+S) before the action table is polled, so
        // a rebound action can never shadow a tool
        if let Some(t) = tools::handle_hotkeys(ctx, &self.hotkeys, &mut self.tools) {
            self.tools.tool = t;
            self.layout.reveal(Pane::Tools);
        }
        // bare S is snapping's own key, claimed the same way (see tools::handle_snap_hotkey)
        if tools::handle_snap_hotkey(ctx, &mut self.settings.snap) {
            self.settings.save();
        }
        let mut actions = self.hotkeys.poll(ctx);
        if !ctx.wants_keyboard_input() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::Y)) {
            actions.push(Action::Redo);
        }
        if self.fullscreen && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            actions.push(Action::Fullscreen);
        }

        let title = self.title();
        if title != self.last_title {
            self.last_title = title.clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }

        // ---- layout ----
        if self.fullscreen {
            // same pane as the docked preview (it reads self.fullscreen) - no second copy to drift
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE.fill(egui::Color32::BLACK))
                .show(ctx, |ui| self.draw_pane(ui, Pane::Preview));
        } else {
            egui::TopBottomPanel::top("menu").show(ctx, |ui| {
                actions.extend(self.menu_bar(ui));
            });
            self.ensure_bg_texture(ctx);
            let tab_bar = (self.bg_tex.is_some() && self.settings.panel_opacity < 255).then_some(self.palette.header);
            egui::CentralPanel::default().show(ctx, |ui| {
                if let Some((_, _, tex)) = &self.bg_tex {
                    let r = ui.max_rect();
                    ui.painter().image(
                        tex.id(),
                        r,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                    let t = self.settings.bg_tint;
                    ui.painter().rect_filled(r, 0.0, egui::Color32::from_rgba_unmultiplied(t[0], t[1], t[2], t[3]));
                }
                let mut l = std::mem::replace(&mut self.layout, Layout::new(egui_tiles::Tree::empty("layout")));
                // cloned: the draw closure needs self mutably while the tab renderer reads the icons
                let icons = self.settings.icon_overrides.clone();
                let cozy = self.settings.ui_look != "sharp";
                // ---- ws:layout-modes-onboarding ----
                // Both closures need `self` (draw: mutably; on_viewport: the hotkey table, then
                // pending_actions), and layout::show calls them strictly one after the other - never
                // nested - so a RefCell hands the borrow back and forth at runtime, the same shape
                // `App::fire_hook` already uses for its tool-call closure.
                let cell = std::cell::RefCell::new(&mut *self);
                let (changed, moved, set_icon) = layout::show(
                    ctx,
                    ui,
                    &mut l,
                    &icons,
                    tab_bar,
                    cozy,
                    &mut |ui, pane| cell.borrow_mut().draw_pane(ui, pane),
                    // ---- ws:registries-schema-hooks ----
                    // filled by ws:layout-modes-onboarding: poll the action table on the popped
                    // viewport's own ctx, so Space/J/K/L work in a torn-off Preview (each viewport
                    // has its own input state, so nothing double-fires with the root poll above)
                    &mut |vctx| {
                        let acts = layout_ctl::poll_popout(&cell.borrow().hotkeys, vctx);
                        cell.borrow_mut().pending_actions.extend(acts);
                    },
                );
                self.layout = l;
                self.layout_dirty |= changed;
                if moved {
                    push_undo_json(&mut self.undo, &mut self.redo, LAYOUT_STEP.to_owned());
                }
                if !set_icon.is_empty() {
                    for (pane, pick) in set_icon {
                        let key = format!("pane.{}", pane.title());
                        match pick {
                            Some(name) => drop(self.settings.icon_overrides.insert(key, name)),
                            None => drop(self.settings.icon_overrides.remove(&key)),
                        }
                    }
                    self.settings.save();
                }
            });
        }

        // clipboard and Delete last: the curve and node editors claim those while the pointer is over
        // them, and only what they leave behind should reach the timeline
        actions.extend(self.hotkeys.poll_late(ctx));
        if !ctx.wants_keyboard_input() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace))
        {
            actions.push(Action::Delete);
        }
        if let Some(text) = self.os_clipboard.take() {
            ctx.copy_text(text);
        }
        actions.append(&mut self.pending_actions);
        for a in actions {
            if a == Action::Fullscreen {
                // the viewport command needs the ctx; keep act() ctx-free
                self.act(a);
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
            } else {
                self.act(a);
            }
        }

        // also in fullscreen: an export's progress + Cancel must not disappear behind it
        self.windows(ctx);

        // persist the layout when it changed, debounced to the end of drag gestures
        if self.layout_dirty && !ctx.input(|i| i.pointer.any_down()) {
            let json = self.layout.to_json();
            if json != self.layout_json {
                self.layout_json = json.clone();
                self.settings.layout = json;
                self.settings.save();
            }
            self.layout_dirty = false;
        }

        // toasts: drawn by feedback::draw, a WINDOW_DRAWER (ws:forgiveness) - this used to be an inline
        // block here; see windows() -> WINDOW_DRAWERS.
    }
}

// =====================================================================================
// ---- ws:registries-schema-hooks ----
// The five append-only dispatch registries every later workstream plugs into: a `// ---- ws:<name>
// ----` marker line per workstream (wave-then-name order), so a PR that fills its own line never
// shares a hunk with another workstream's. See the "Shared-registry protocol" section of
// plans/ui-overhaul/README.md. Only TOOL_TABLES carries real content this wave (the existing tool
// groups, flattened for `mcp::tools::all()`); the other four start empty - nothing to migrate yet,
// since `act()`/`draw_pane_inner()`/`windows()`/`update()` keep every existing arm unchanged and only
// gained a small prelude loop (or, for `draw_pane_inner`, a trailing catch-all) that tries the
// registry first.
pub(crate) const TOOL_TABLES: &[&[mcp::tools::ToolDef]] = &[
    tools_timeline::TOOLS,
    tools_media::TOOLS,
    tools_clip::TOOLS,
    tools_playback::TOOLS,
    tools_subtitles::TOOLS,
    tools_ui::TOOLS,
    // ---- ws:registries-schema-hooks ----
    whatsnew::TOOLS,
    // ---- ws:split-god-files ----
    // ---- ws:audio-analysis ----
    tools_audio::TOOLS,
    // ---- ws:audio-dsp-automation ----
    tools_mixer::TOOLS,
    // ---- ws:color-engine ----
    tools_color::TOOLS,
    // ---- ws:command-palette ----
    tools_commands::TOOLS,
    // ---- ws:forgiveness ----
    tools_project::TOOLS,
    // ---- ws:player-rate-loop ----
    // already registered above (tools_playback::TOOLS predates the marker system; wave-0a wired it in
    // directly) - this workstream appends new rows into that same const, not a second registration.
    // ---- ws:snap-engine ----
    // ---- ws:trim-model ----
    tools_trim::TOOLS,
    // ---- ws:canvas-handles-monitor ----
    tools_preview::TOOLS,
    // ---- ws:export-deliver ----
    tools_export::TOOLS,
    // ---- ws:inspector-gallery ----
    tools_gallery::TOOLS,
    // ---- ws:layout-modes-onboarding ----
    tools_layout::TOOLS,
    // ---- ws:media-library ----
    // already registered above (tools_media::TOOLS predates the marker system; wave-0a wired it in
    // directly) - this workstream appends its rows into that same const, not a second registration.
    // ---- ws:source-monitor ----
    tools_source::TOOLS,
    // ---- ws:timeline-trim-gestures ----
    // ---- ws:transcript-captions ----
    tools_transcript::TOOLS,
    // ---- ws:pro-monitor ----
    tools_monitor::TOOLS,
    // ---- ws:pro-timeline ----
    tools_timeline_pro::TOOLS,
    // ---- ws:text-titles ----
    tools_titles::TOOLS,
    // ---- ws:docs-refresh ----
];

pub(crate) const ACT_HANDLERS: &[fn(&mut App, Action) -> bool] = &[
    // ---- ws:registries-schema-hooks ----
    whatsnew::act,
    // ---- ws:split-god-files ----
    // ---- ws:audio-analysis ----
    audio_actions::act,
    // ---- ws:audio-dsp-automation ----
    // ---- ws:color-engine ----
    tools_color::act,
    // ---- ws:command-palette ----
    palette_ctl::act,
    // ---- ws:forgiveness ----
    // ---- ws:player-rate-loop ----
    playback_ctl::act,
    // ---- ws:snap-engine ----
    // ---- ws:trim-model ----
    trim_actions::act,
    // ---- ws:canvas-handles-monitor ----
    monitor::act,
    // ---- ws:export-deliver ----
    tools_export::act,
    // ---- ws:inspector-gallery ----
    // ---- ws:layout-modes-onboarding ----
    layout_ctl::act,
    // ---- ws:media-library ----
    media_sync::act,
    // ---- ws:source-monitor ----
    source_ctl::act,
    // ---- ws:timeline-trim-gestures ----
    // ---- ws:transcript-captions ----
    transcript_ctl::act,
    // ---- ws:pro-monitor ----
    tools_monitor::act,
    // ---- ws:pro-timeline ----
    tools_timeline_pro::act,
    // ---- ws:text-titles ----
    // ---- ws:docs-refresh ----
];

pub(crate) const FRAME_HOOKS: &[fn(&mut App, &egui::Context)] = &[
    // ---- ws:registries-schema-hooks ----
    whatsnew::tick,
    // ---- ws:split-god-files ----
    // ---- ws:audio-analysis ----
    // ---- ws:audio-dsp-automation ----
    tools_mixer::sync_buses,
    // ---- ws:color-engine ----
    // ---- ws:command-palette ----
    palette_ctl::tick,
    // ---- ws:forgiveness ----
    autosave::autosave_tick,
    // ---- ws:player-rate-loop ----
    playback_ctl::tick,
    // ---- ws:snap-engine ----
    // ---- ws:trim-model ----
    // ---- ws:canvas-handles-monitor ----
    monitor::tick,
    // ---- ws:export-deliver ----
    tools_export::frame_tick,
    // ---- ws:inspector-gallery ----
    // ---- ws:layout-modes-onboarding ----
    frame::tick,
    // ---- ws:media-library ----
    media_sync::tick,
    // ---- ws:source-monitor ----
    source_pane::tick,
    // ---- ws:timeline-trim-gestures ----
    // ---- ws:transcript-captions ----
    transcript_ctl::tick,
    // ---- ws:pro-monitor ----
    monitor::monitor_tick,
    // ---- ws:pro-timeline ----
    // ---- ws:text-titles ----
    // ---- ws:docs-refresh ----
];

pub(crate) const WINDOW_DRAWERS: &[fn(&mut App, &egui::Context)] = &[
    // ---- ws:registries-schema-hooks ----
    whatsnew::window,
    // ---- ws:split-god-files ----
    // ---- ws:audio-analysis ----
    // ---- ws:audio-dsp-automation ----
    // ---- ws:color-engine ----
    // ---- ws:command-palette ----
    palette_ctl::windows,
    // ---- ws:forgiveness ----
    feedback::draw,
    confirm::draw,
    recovery::restore_window,
    // ---- ws:player-rate-loop ----
    // ---- ws:snap-engine ----
    // ---- ws:trim-model ----
    // ---- ws:canvas-handles-monitor ----
    // ---- ws:export-deliver ----
    // ---- ws:inspector-gallery ----
    // ---- ws:layout-modes-onboarding ----
    layout_ctl::windows,
    // ---- ws:media-library ----
    media_sync::windows,
    // ---- ws:source-monitor ----
    // ---- ws:timeline-trim-gestures ----
    // ---- ws:transcript-captions ----
    transcript_ctl::window,
    // ---- ws:pro-monitor ----
    tools_monitor::window_scopes,
    tools_monitor::window_multicam,
    // ---- ws:pro-timeline ----
    tools_timeline_pro::window,
    // ---- ws:text-titles ----
    // ---- ws:docs-refresh ----
];

pub(crate) const PANE_DRAWERS: &[fn(&mut App, &mut egui::Ui, Pane) -> bool] = &[
    // ---- ws:registries-schema-hooks ----
    // ---- ws:size-diet ----
    // ---- ws:split-god-files ----
    // ---- ws:audio-analysis ----
    // ---- ws:audio-dsp-automation ----
    // ---- ws:color-engine ----
    // ---- ws:command-palette ----
    // ---- ws:forgiveness ----
    // ---- ws:player-rate-loop ----
    // ---- ws:snap-engine ----
    // ---- ws:trim-model ----
    // ---- ws:canvas-handles-monitor ----
    // ---- ws:export-deliver ----
    // ---- ws:inspector-gallery ----
    gallery_ctl::draw,
    // ---- ws:layout-modes-onboarding ----
    // ---- ws:media-library ----
    // ---- ws:source-monitor ----
    source_pane::draw,
    // ---- ws:timeline-trim-gestures ----
    // ---- ws:transcript-captions ----
    // ---- ws:pro-monitor ----
    // ---- ws:pro-timeline ----
    // ---- ws:text-titles ----
    // ---- ws:docs-refresh ----
];

/// The dominant kind of the current selection, for a contextual inspector/palette to key off without
/// re-deriving it from `App.selection` itself (ws:inspector-gallery, wave 2). `Mixed` covers a
/// selection spanning more than one kind; `EditPoint`/`Cue` are placeholders for selections this
/// wave's App state doesn't track yet (edit-point set, subtitle cue).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // unused until ws:inspector-gallery (wave 2) reads it
pub(crate) enum SelectionKind {
    None,
    Video,
    Audio,
    Text,
    Shape,
    Sequence,
    Adjustment,
    Transition,
    Cue,
    EditPoint,
    Mixed,
}

impl App {
    /// The dominant kind of the current clip/transition selection.
    #[allow(dead_code)] // unused until ws:inspector-gallery (wave 2)
    pub(crate) fn selection_kind(&self) -> SelectionKind {
        if !self.sel_transitions.is_empty() && self.selection.is_empty() {
            return SelectionKind::Transition;
        }
        let kinds: Vec<ClipKind> =
            self.selection.iter().filter_map(|&id| self.project.clip(id)).map(|c| c.kind).collect();
        match &kinds[..] {
            [] => SelectionKind::None,
            [first, rest @ ..] if rest.iter().all(|k| k == first) => match first {
                ClipKind::Video | ClipKind::Image => SelectionKind::Video,
                ClipKind::Audio => SelectionKind::Audio,
                ClipKind::Text => SelectionKind::Text,
                ClipKind::Shape => SelectionKind::Shape,
                ClipKind::Sequence => SelectionKind::Sequence,
                ClipKind::Adjustment => SelectionKind::Adjustment,
            },
            _ => SelectionKind::Mixed,
        }
    }

    /// Bring `pane` to the front - today this is exactly `Layout::reveal` + marking the layout dirty
    /// so it persists; ws:layout-modes-onboarding (wave 2) makes it pin/mode-aware without touching
    /// call sites (a pinned pane stops auto-surfacing, a Granular-mode layout ignores it entirely).
    #[allow(dead_code)] // unused until ws:layout-modes-onboarding (wave 2)
    pub(crate) fn surface(&mut self, pane: Pane) {
        self.layout.reveal(pane);
        self.layout_dirty = true;
    }

    /// Real but partial: only the handful of guards worth centralising this wave (an export already
    /// running, exporting an empty timeline, pasting attributes with nothing copied yet). The ~80
    /// other `has_sel`/`has_clips` checks stay inline in `menu_bar` - command-palette (wave 1)
    /// migrates them here when the palette actually needs to grey out rows. `Err`'s text is the toast
    /// reason a caller (`ui.action`, and `act()`'s own prelude) shows the user.
    pub(crate) fn enabled(&self, a: Action) -> Result<(), &'static str> {
        Self::enabled_for(a, self.export.is_some(), self.timeline_is_empty(), self.attrs.is_none())?;
        // ---- ws:command-palette ----
        // A second small guard match, not a bigger `enabled_for` signature: `enabled_for` (and its
        // 3-bool call site) is pre-existing wave-0b code with its own test
        // (`tools_registry_tests::action_enabled_toasts_reason`) already pinned to that exact 3-arg
        // shape - growing it to 6 args would force an edit to a test outside this workstream's owned
        // files for guards only this ws's rows table needs. See `enabled_for2`.
        Self::enabled_for2(
            a,
            self.undo.is_empty(),
            self.redo.is_empty(),
            self.timeline_is_empty(),
            self.selection.is_empty(),
        )
    }

    /// ---- ws:command-palette ----
    /// The pure guards this workstream's palette/menu/hotkey dispatch needs beyond wave-0b's
    /// `enabled_for`: an empty undo/redo stack, and Delete/RippleDelete with nothing selected. Split
    /// into its own small match (see `enabled`'s doc comment) rather than growing `enabled_for`'s
    /// signature.
    pub(crate) fn enabled_for2(
        a: Action,
        undo_empty: bool,
        redo_empty: bool,
        timeline_empty: bool,
        no_selection: bool,
    ) -> Result<(), &'static str> {
        match a {
            Action::Undo if undo_empty => Err("Nothing to undo"),
            Action::Redo if redo_empty => Err("Nothing to redo"),
            Action::Split if timeline_empty => Err("Nothing to split - the timeline is empty"),
            Action::Delete | Action::RippleDelete if no_selection => Err("Select something to delete first"),
            _ => Ok(()),
        }
    }

    /// The pure match behind `enabled`, split out so `action_enabled_toasts_reason` can exercise every
    /// arm directly (plain bools in, no live `App` - see that test's doc comment for why one isn't
    /// buildable here today).
    pub(crate) fn enabled_for(
        a: Action,
        export_running: bool,
        timeline_empty: bool,
        no_attrs_copied: bool,
    ) -> Result<(), &'static str> {
        match a {
            Action::Save | Action::SaveProjectAs | Action::ExportVideo | Action::ExportLossless if export_running => {
                Err("An export is running - try again when it finishes")
            }
            Action::ExportVideo | Action::ExportLossless if timeline_empty => {
                Err("Nothing to export - the timeline is empty")
            }
            Action::PasteAttributes if no_attrs_copied => Err("Copy attributes from a clip first (Ctrl+Alt+C)"),
            _ => Ok(()),
        }
    }

    /// Set an undo entry's label directly (mirrors the existing `LAYOUT_STEP` sentinel path), so the
    /// History panel skips `describe_change`'s lazy diff for a labelled edit and shows `label` instead
    /// of "Project edited". Callers push the entry themselves (this only sets the label on the last
    /// one) - see `push_undo_json`. First real caller: ws:forgiveness (Delete/RippleDelete, History
    /// panel restore).
    pub(crate) fn push_undo_labeled(&mut self, before: String, label: &'static str) {
        push_undo_json(&mut self.undo, &mut self.redo, before);
        if let Some(e) = self.undo.last_mut() {
            e.label = label.to_string();
        }
    }

    // ---- ws:forgiveness ----
    /// Resolve a queued `confirm::ConfirmAction` on Yes - the two `Project`-touching variants push a
    /// labeled undo first (mirrors every other project edit); the two `Settings`-touching variants just
    /// save (Settings isn't part of the undo stack). The actual field mutation is the pure
    /// `confirm::apply_to_project`/`apply_to_settings` pair, so it's testable without a live `App`.
    pub(crate) fn resolve_confirm(&mut self, action: confirm::ConfirmAction) {
        match action {
            confirm::ConfirmAction::ClearSubtitles => {
                let before = self.project.to_json();
                if confirm::apply_to_project(&mut self.project, &action) {
                    self.push_undo_labeled(before, "Clear subtitles");
                    self.after_edit();
                }
            }
            confirm::ConfirmAction::ReplaceSubtitles(_) => {
                let before = self.project.to_json();
                if confirm::apply_to_project(&mut self.project, &action) {
                    self.push_undo_labeled(before, "Replace subtitles");
                    self.after_edit();
                }
            }
            confirm::ConfirmAction::ClearRecent | confirm::ConfirmAction::DeleteTemplate(_) => {
                if confirm::apply_to_settings(&mut self.settings, &action) {
                    self.settings.save();
                }
            }
            confirm::ConfirmAction::Custom(f) => f(self),
        }
    }

    /// Single-slot Settings snapshot for `Action::UndoSettings` - call before a destructive Settings
    /// mutation; `label` is reserved for a future toast/undo-entry description (see the
    /// `settings_undo` field's doc comment for why this is one slot, not a stack).
    /// ponytail: no caller yet this wave (see the PR body) - this workstream builds the primitive
    /// (field + Action::UndoSettings arm + this setter); the first destructive Settings op (e.g. a
    /// future "Reset all hotkeys") calls it.
    #[allow(dead_code, unused_variables)]
    pub(crate) fn settings_snapshot(&mut self, label: &'static str) {
        self.settings_undo = Some(self.settings.clone());
    }

    /// The one sanctioned funnel for a NEW timed-repaint request (`ctx.request_repaint_after` at a
    /// computed instant rather than a fixed duration). This is not a retroactive migration: 17
    /// pre-existing raw `ctx.request_repaint_after(...)` call sites (app.rs-descended files ~12,
    /// planner.rs:740, preview.rs:694/709, subtitles_ui.rs:616/741) stay as they are - narrowing the
    /// idle-CPU-0% principle to "the only path for new code", not an invariant already true of the
    /// whole crate today.
    /// ponytail: the 17 existing sites are an accepted, un-migrated ceiling - a follow-up cleanup
    /// (size-diet or its own pass) can fold them into this fn; not a wave-0b blocker.
    ///
    /// First caller: `whatsnew::tick` (size-diet), routing winpos's window-rect debounce through here.
    pub(crate) fn animate_until(&mut self, ctx: &egui::Context, at: Instant) {
        if let Some(dt) = at.checked_duration_since(Instant::now()) {
            ctx.request_repaint_after(dt);
        } else {
            ctx.request_repaint();
        }
    }

    // ---- ws:audio-analysis ----
    /// `marker_added` once per id in `ids` - the one call site every marker-creation path in this
    /// workstream (autocut_ui's silence "Mark instead", Detect Beats/Split at Beats, Scene cuts'
    /// "Mark instead", and their MCP-tool twins) funnels through, so the event fires exactly once
    /// regardless of entry point. See `fire_marker_added_for_each` for the plain, App-free half this
    /// delegates to (and that a test exercises directly).
    pub(crate) fn fire_markers_added(&mut self, ids: &[Id]) {
        fire_marker_added_for_each(ids, &mut |event, payload| self.fire_hook(event, payload));
    }
}

/// The pure half of `fire_markers_added`: call `hook("marker_added", {"marker_id": id})` once per id,
/// in order. Split out so a test can assert "exactly once per marker" without a live `App` - mirrors
/// `mcp_exec.rs`'s `snapshot_if_mutate`/`rollback_project` split for the identical reason.
pub(crate) fn fire_marker_added_for_each(ids: &[Id], hook: &mut dyn FnMut(&'static str, Value)) {
    for &id in ids {
        hook("marker_added", json!({"marker_id": id}));
    }
}

#[cfg(test)]
mod tests;
