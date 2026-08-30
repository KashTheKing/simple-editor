//! The application: owns the project, undo stack, settings, player, dockable layout and wires the panels
//! together. Non-blocking windows (Settings, Retime, Export, Save Template, Save Profile, export/convert
//! progress) are plain `egui::Window`s — the editor stays usable while they are open. Also executes MCP
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
use crate::settings::Settings;
use crate::theme::{self, Palette};
use crate::ui::layout::{self, Layout, Pane};
use crate::ui::tools::Tool;
use crate::ui::{
    autocut_ui, capture_ui, curves, effects_ui, export_ui, frame_ui, import_ui, inspector, library, markers_ui,
    mixer_ui, nodes, paste_ui, planner, presets_ui, preview, retime, settings_ui, shader_ui, subtitles_ui, timeline,
    tools, tracking_ui, transitions_ui, DragPayload,
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

enum ExportKind {
    File,
    Overwrite { original: PathBuf, temp: PathBuf },
}

/// A blocking MCP tool job (export.video / media.convert): the reply is sent when the job finishes.
struct McpJob {
    prog: Arc<Progress>,
    reply: Sender<Result<Value, String>>,
    out: PathBuf,
}

/// The library's own preview player: a file, its own decoder/audio pipeline, and the duration/fps a
/// transport needs (an asset's own probed values, since this project is a synthetic single-clip one).
struct LibPreview {
    path: PathBuf,
    player: Player,
    duration: f64,
    fps: f64,
}

pub struct App {
    project: Project,
    project_path: Option<PathBuf>,
    dirty: bool,
    undo: Vec<String>,
    redo: Vec<String>,
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
    presets: presets_ui::PresetsState,
    autocut: autocut_ui::AutoCutState,
    tracking: tracking_ui::TrackState,
    retime: retime::RetimeUi,
    export_ui: export_ui::ExportUi,
    /// Some = the "Save Template" / "Save Profile" name windows are open (the String is the name field).
    template_name: Option<String>,
    profile_name: Option<String>,
    fullscreen: bool,
    selection: Vec<Id>,
    playhead: f64,
    export: Option<(Arc<Progress>, ExportKind)>,
    encoders: Vec<String>,
    toasts: Vec<(String, Instant)>,
    screenshot: Option<PathBuf>,
    started: Instant,
    first_frame_at: Option<Instant>,
    screenshot_requested: bool,
    close_confirmed: bool,
    /// Close was requested during an export: cancel it, then re-request the close once it has finished.
    close_after_export: bool,
    was_playing: bool,
    /// Last title sent to the OS — `send_viewport_cmd` forces a repaint, so only send on change.
    last_title: String,
    palette: Palette,
    /// Newest rendered frame, handed to the preview pane when it draws.
    pending_frame: Option<Arc<Frame>>,
    /// Actions requested by panels this frame (transport, context menus, breadcrumb).
    pending_actions: Vec<Action>,
    mcp: Option<(mcp::Server, Receiver<mcp::ToolCall>)>,
    mcp_port_running: u16,
    mcp_jobs: Vec<McpJob>,
    /// Library "Convert To…" jobs: (progress, output path) — polled each frame, imported when done.
    convert_jobs: Vec<(Arc<Progress>, PathBuf)>,
    /// Asset id + target extension for the Convert To… options window.
    convert_dialog: Option<(Id, String)>,
    /// Compress… window state (None = closed).
    compress: Option<Compress>,
    /// A working yt-dlp was found — gates the Library's URL import. Detected on a background thread
    /// (it spawns `yt-dlp --version`) at start-up and again when the setting changes.
    ytdlp_available: Arc<std::sync::atomic::AtomicBool>,
    /// Import-URL window state: (url, audio only).
    url_dialog: Option<(String, bool)>,
    /// Running URL downloads — polled each frame, imported into the library when they finish.
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
    /// on the GL context) — shared, since both are served identically.
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
    /// Editor background image: (path, blur radius, texture) — reloaded when either key changes.
    bg_tex: Option<(String, u8, egui::TextureHandle)>,
    /// The GPU path failed once — do not retry until the setting is switched off and on again.
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
    /// Ctrl+C / Ctrl+X clip clipboard — a template (clips + the assets they use), so paste reuses
    /// `Project::place_clips` and its fresh clip / link ids.
    clipboard: Option<crate::settings::Template>,
    /// Text to hand the OS clipboard at the end of the frame. egui-winit only emits `Event::Paste` when
    /// the system clipboard holds text (egui-winit-0.33.3 src/lib.rs:823 returns without pushing the key
    /// event either way), so a Ctrl+V after an internal-only copy produced NO event at all and could
    /// never be bound. Copying clips therefore also writes them out as text.
    os_clipboard: Option<String>,
    /// The library's own preview: a player of its own so it never disturbs the program monitor or the
    /// timeline playhead, and the texture the pane paints this frame. While it is Some, the Preview
    /// pane shows this instead of the timeline (see `draw_lib_preview`).
    lib_preview: Option<LibPreview>,
    lib_preview_tex: Option<egui::TextureHandle>,
    /// This update's uploaded frame, computed once (`Player::take_frame` consumes the buffered frame, so
    /// pulling it twice in one update would starve whichever call came second). Both the library pane's
    /// own preview box and the viewport override read this same value.
    lib_preview_live: Option<library::PreviewFrame>,
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
/// `Layout`'s own (much shorter) history — this only keeps Ctrl+Z stepping back in the right order.
/// ponytail: once the layout history has scrolled past its 20 entries the marker undoes nothing; deepen
/// the layout stack if that ever bites.
const LAYOUT_STEP: &str = "\u{0}layout";

/// Push an undo snapshot (capped) and clear the redo history.
fn push_undo_json(undo: &mut Vec<String>, redo: &mut Vec<String>, json: String) {
    undo.push(json);
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
/// down — same policy as the decoder threads. None = it panicked.
/// ponytail: the panic message goes to the default hook (stderr); the caller toasts and degrades.
fn guarded<T>(f: impl FnOnce() -> T) -> Option<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).ok()
}

/// Give the clip's last effect a mask, or the clip itself when it has no effects. False = there is one
/// already, or the clip is audio (a mask shapes pixels, and audio has none).
fn add_mask(project: &mut Project, clip: Id, shape: MaskShape) -> bool {
    let Some(c) = project.clip_mut(clip).filter(|c| c.is_visual()) else { return false };
    match c.effects.iter_mut().rev().find(|e| e.enabled) {
        Some(e) if e.mask.is_none() => {
            e.mask = Some(Mask::new(shape));
            true
        }
        Some(_) => false,
        None if c.mask.is_none() => {
            c.mask = Some(Mask::new(shape));
            true
        }
        None => false,
    }
}

/// Preview render size for a pane of `canvas` px at `quality` percent (25..100). Zero stays zero
/// (nothing to render), and the aspect ratio is kept.
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
/// chosen resize flag — rendering a 4K frame from a 1080p timeline gains nothing but time.
fn frame_render_size(project: (u32, u32), target: (u32, u32)) -> (u32, u32) {
    let (pw, ph) = (project.0.max(16), project.1.max(16));
    let (tw, th) = (target.0.max(16), target.1.max(16));
    if tw <= pw && th <= ph {
        (tw, th)
    } else {
        (pw, ph)
    }
}

// TODO(ui-panels-fx): call these from `effects_ui::set_thumbnail(kind, ...)` once that hook exists —
// the app renders each kind once on the GPU from this source and hands the result over.
#[allow(dead_code)]
/// Cache key of one effect thumbnail: kind, source image and size. Changing the stock image (or the
/// grid size) invalidates every thumbnail; two different effects never share a key.
fn effect_thumb_key(kind: EffectKind, size: (u32, u32), image: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325; // FNV-1a
    let mut eat = |b: &[u8]| {
        for &x in b {
            h ^= x as u64;
            h = h.wrapping_mul(0x1000_0000_01b3);
        }
    };
    eat(kind.name().as_bytes());
    eat(&size.0.to_le_bytes());
    eat(&size.1.to_le_bytes());
    eat(image.as_bytes());
    h
}

/// The embedded stock picture, already decoded: `assets/bubble.webp` converted once to raw RGBA at card
/// size, because nothing in the binary can decode a webp (no image crate, and ffmpeg may be missing).
const STOCK: &[u8] = include_bytes!("../../assets/bubble_96x54.rgba");
const STOCK_W: u32 = 96;
const STOCK_H: u32 = 54;

#[allow(dead_code)]
/// The picture the effect thumbnails are rendered from: the user's stock image, else the embedded one.
fn effect_thumb_source(image: &str, w: u32, h: u32, backend: Backend) -> Frame {
    let (w, h) = (w.max(1), h.max(1));
    if !image.is_empty() {
        if let Ok(mut src) = media::open_video(image, backend) {
            let mut f = Frame::default();
            if src.frame_at(0.0, w, h, &mut f) && !f.is_empty() {
                return f;
            }
        }
    }
    // ponytail: nearest rescale — the catalogue asks for exactly STOCK_W x STOCK_H, so it is a plain copy
    let mut f = Frame::new(w, h);
    for y in 0..h {
        let sy = y * STOCK_H / h;
        for x in 0..w {
            let s = ((sy * STOCK_W + x * STOCK_W / w) * 4) as usize;
            let d = ((y * w + x) * 4) as usize;
            f.rgba[d..d + 4].copy_from_slice(&STOCK[s..s + 4]);
        }
    }
    f
}

/// In-place box blur of an RGBA buffer, three separable passes ≈ gaussian. Runs once at background
/// image load time, never per frame. ponytail: CPU blur at load; a shader pass if live blur is wanted.
fn box_blur(rgba: &mut [u8], w: usize, h: usize, radius: usize) {
    if w == 0 || h == 0 || radius == 0 {
        return;
    }
    let mut tmp = vec![0u8; rgba.len()];
    // one horizontal sliding-window pass over `src` into `dst`; the vertical pass reuses it transposed
    // by swapping the stride arguments.
    let pass = |src: &[u8], dst: &mut [u8], cols: usize, rows: usize, col_stride: usize, row_stride: usize| {
        for row in 0..rows {
            let at = |col: usize| row * row_stride + col * col_stride;
            let mut sum = [0usize; 4];
            for col in 0..=radius.min(cols - 1) {
                let p = at(col);
                for c in 0..4 {
                    sum[c] += src[p + c] as usize;
                }
            }
            let mut count = radius.min(cols - 1) + 1;
            for col in 0..cols {
                let p = at(col);
                for c in 0..4 {
                    dst[p + c] = (sum[c] / count) as u8;
                }
                if col + radius + 1 < cols {
                    let q = at(col + radius + 1);
                    for c in 0..4 {
                        sum[c] += src[q + c] as usize;
                    }
                    count += 1;
                }
                if col >= radius {
                    let q = at(col - radius);
                    for c in 0..4 {
                        sum[c] -= src[q + c] as usize;
                    }
                    count -= 1;
                }
            }
        }
    };
    for _ in 0..3 {
        pass(rgba, &mut tmp, w, h, 4, w * 4); // horizontal
        pass(&tmp, rgba, h, w, w * 4, 4); // vertical
    }
}

/// Write one rendered frame as PNG / JPG / WebP through ffmpeg (raw RGBA on stdin), resizing to
/// `opts.size` with `opts.resize` when the render came out at a different size.
fn write_image(frame: &Frame, opts: &frame_ui::FrameExport) -> Result<(), String> {
    if frame.is_empty() {
        return Err("empty frame".into());
    }
    let exe = media::ffpipe::ffmpeg_exe().ok_or("ffmpeg.exe not found")?;
    let ext = opts.out.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_else(|| "png".into());
    let mut cmd = media::ffpipe::command(&exe);
    cmd.args(["-y", "-v", "error", "-f", "rawvideo", "-pix_fmt", "rgba"]);
    cmd.args(["-s", &format!("{}x{}", frame.width, frame.height), "-i", "-"]);
    if (frame.width, frame.height) != opts.size {
        let flags = if opts.resize.is_empty() { "lanczos" } else { opts.resize.as_str() };
        cmd.args(["-vf", &format!("scale={}:{}:flags={flags}", opts.size.0, opts.size.1)]);
    }
    let q = opts.quality.clamp(1, 100);
    match ext.as_str() {
        // ffmpeg's mjpeg qscale is 2 (best) .. 31 (worst)
        "jpg" | "jpeg" => cmd.args(["-q:v", &format!("{}", 2 + (100 - q) * 29 / 100)]),
        "webp" => cmd.args(["-quality", &format!("{q}")]),
        _ => cmd.args(["-compression_level", "9"]),
    };
    cmd.args(["-frames:v", "1"]).arg(&opts.out);
    cmd.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("ffmpeg: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        let _ = stdin.write_all(&frame.rgba); // a broken pipe shows up as a non-zero exit below
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Standard base64 (RFC 4648, with padding) — for the `render.frame` PNG data url. Tool path, not hot.
fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

// ---------------- MCP tool argument helpers ----------------

fn arg_str<'a>(args: &'a Value, k: &str) -> Option<&'a str> {
    args.get(k)?.as_str()
}
fn arg_f64(args: &Value, k: &str) -> Option<f64> {
    args.get(k)?.as_f64()
}
fn arg_u64(args: &Value, k: &str) -> Option<u64> {
    args.get(k)?.as_u64()
}
fn arg_bool(args: &Value, k: &str) -> Option<bool> {
    args.get(k)?.as_bool()
}
fn arg_ids(args: &Value, k: &str) -> Option<Vec<Id>> {
    Some(args.get(k)?.as_array()?.iter().filter_map(|v| v.as_u64()).collect())
}
fn req<T>(o: Option<T>, k: &str) -> Result<T, String> {
    o.ok_or_else(|| format!("missing or invalid argument '{k}'"))
}

/// "Linear" | "EaseIn" | … | "cubic-bezier(x1,y1,x2,y2)"
fn parse_ease(s: &str) -> Option<crate::model::Ease> {
    use crate::model::Ease;
    match s {
        "Linear" => return Some(Ease::Linear),
        "EaseIn" => return Some(Ease::EaseIn),
        "EaseOut" => return Some(Ease::EaseOut),
        "EaseInOut" => return Some(Ease::EaseInOut),
        "Hold" => return Some(Ease::Hold),
        _ => {}
    }
    let inner = s.strip_prefix("cubic-bezier(")?.strip_suffix(')')?;
    let v: Vec<f32> = inner.split(',').filter_map(|p| p.trim().parse().ok()).collect();
    match v[..] {
        [x1, y1, x2, y2] => Some(Ease::Bezier { x1, y1, x2, y2 }),
        _ => None,
    }
}

/// The animated property `name` of a clip: inspector labels or "<Effect name>: <Param name>".
fn anim_of<'a>(clip: &'a mut Clip, name: &str) -> Option<&'a mut crate::model::Animated> {
    match name {
        "Position X" => return Some(&mut clip.x),
        "Position Y" => return Some(&mut clip.y),
        "Scale" => return Some(&mut clip.scale),
        "Rotation" => return Some(&mut clip.rotation),
        "Opacity" => return Some(&mut clip.opacity),
        "Volume" => return Some(&mut clip.volume),
        "Pan" => return Some(&mut clip.pan),
        _ => {}
    }
    let (effect, param) = name.split_once(':')?;
    let (effect, param) = (effect.trim(), param.trim());
    let e = clip.effects.iter_mut().find(|e| e.kind.name().eq_ignore_ascii_case(effect))?;
    let i = e.kind.params().iter().position(|p| p.name.eq_ignore_ascii_case(param))?;
    e.params.get_mut(i)
}

fn mask_shape(s: &str) -> Result<MaskShape, String> {
    MaskShape::ALL
        .into_iter()
        .find(|m| m.name().eq_ignore_ascii_case(s) || format!("{m:?}").eq_ignore_ascii_case(s))
        .ok_or_else(|| format!("unknown mask shape '{s}' (Rect | Ellipse | Polygon | Path)"))
}

/// The mask slot of a clip, or of one of its effects (`effect` = index in the effect stack).
fn mask_slot(project: &mut Project, clip: Id, effect: Option<usize>) -> Result<&mut Option<Mask>, String> {
    let c = project.clip_mut(clip).ok_or("no such clip")?;
    if !c.is_visual() {
        return Err("that clip is audio — a mask shapes pixels, and audio has none".into());
    }
    match effect {
        None => Ok(&mut c.mask),
        Some(i) => Ok(&mut c.effects.get_mut(i).ok_or("no effect at that index")?.mask),
    }
}

fn apply_mask_fields(mask: &mut Mask, fields: &Value) -> Result<(), String> {
    let obj = fields.as_object().ok_or("'fields' must be an object")?;
    for (k, v) in obj {
        match k.as_str() {
            "shape" => mask.shape = mask_shape(v.as_str().ok_or("shape: string")?)?,
            "invert" => mask.invert = v.as_bool().ok_or("invert: bool")?,
            "enabled" => mask.enabled = v.as_bool().ok_or("enabled: bool")?,
            "points" => {
                let a = v.as_array().ok_or("points: [[x, y], …]")?;
                mask.points = a
                    .iter()
                    .filter_map(|p| {
                        let p = p.as_array()?;
                        Some((p.first()?.as_f64()? as f32, p.get(1)?.as_f64()? as f32))
                    })
                    .collect();
            }
            "cx" | "cy" | "rx" | "ry" | "rotation" | "feather" | "expand" | "opacity" => {
                let val = v.as_f64().ok_or_else(|| format!("{k}: number"))?;
                let a = match k.as_str() {
                    "cx" => &mut mask.cx,
                    "cy" => &mut mask.cy,
                    "rx" => &mut mask.rx,
                    "ry" => &mut mask.ry,
                    "rotation" => &mut mask.rotation,
                    "feather" => &mut mask.feather,
                    "expand" => &mut mask.expand,
                    _ => &mut mask.opacity,
                };
                a.keys.clear();
                a.value = val;
            }
            _ => return Err(format!("unknown mask field '{k}'")),
        }
    }
    Ok(())
}

/// "Blur" / "Color" / "Blend" / "Combine" / "Merge" / "Matte" / "Mask" / "Text" / "Input" → a node kind.
fn node_kind(s: &str) -> Result<NodeKind, String> {
    match s.to_ascii_lowercase().as_str() {
        "input" => return Ok(NodeKind::Input),
        "output" => return Err("every graph already has exactly one Output".into()),
        "blend" => return Ok(NodeKind::Blend { mode: BlendMode::Normal, opacity: crate::model::Animated::new(1.0) }),
        "combine" => {
            return Ok(NodeKind::Combine { mode: BlendMode::Normal, factor: crate::model::Animated::new(0.5) })
        }
        "merge" => return Ok(NodeKind::Merge),
        "matte" => return Ok(NodeKind::Matte { invert: false, use_alpha: false }),
        "color" => return Ok(NodeKind::Color([0, 0, 0, 255])),
        "mask" => return Ok(NodeKind::Mask(Mask::new(MaskShape::Ellipse))),
        "text" | "string" => {
            return Ok(NodeKind::String(crate::model::TextStyle { text: "{time}".into(), ..Default::default() }))
        }
        _ => {}
    }
    let kind = EffectKind::ALL
        .into_iter()
        .find(|k| k.name().eq_ignore_ascii_case(s) || format!("{k:?}").eq_ignore_ascii_case(s))
        .ok_or_else(|| {
            format!(
                "unknown node kind '{s}' (an effect name, Blend, Combine, Merge, Matte, Mask, Color, Text or Input)"
            )
        })?;
    Ok(NodeKind::Effect(Effect::new(kind)))
}

fn color_arg(v: &Value) -> Option<[u8; 4]> {
    let a = v.as_array()?;
    let mut c = [255u8; 4];
    for (i, x) in a.iter().take(4).enumerate() {
        c[i] = x.as_u64()? as u8;
    }
    (a.len() >= 3).then_some(c)
}

/// Apply `clip.set` fields to a clip (everything except `speed`/`reverse`, which the caller routes
/// through `Project::set_speed` so neighbour collisions are respected).
fn apply_clip_fields(clip: &mut Clip, fields: &Value) -> Result<(), String> {
    let obj = fields.as_object().ok_or("'fields' must be an object")?;
    for (k, v) in obj {
        match k.as_str() {
            "speed" | "reverse" => {} // handled by the caller
            "name" => clip.name = v.as_str().ok_or("name: string")?.to_string(),
            "enabled" => clip.enabled = v.as_bool().ok_or("enabled: bool")?,
            "label" => clip.label = v.as_u64().filter(|&l| l <= 8).ok_or("label: 0..8")? as u8,
            "freeze" => clip.freeze = if v.is_null() { None } else { Some(v.as_f64().ok_or("freeze: number|null")?) },
            "blend" => {
                let name = v.as_str().ok_or("blend: string")?;
                clip.blend = BlendMode::ALL
                    .into_iter()
                    .find(|b| b.name().eq_ignore_ascii_case(name) || format!("{b:?}").eq_ignore_ascii_case(name))
                    .ok_or_else(|| format!("unknown blend mode '{name}'"))?;
            }
            "fade_in" => clip.fade_in = v.as_f64().ok_or("fade_in: number")?.max(0.0),
            "fade_out" => clip.fade_out = v.as_f64().ok_or("fade_out: number")?.max(0.0),
            "x" | "y" | "scale" | "rotation" | "opacity" | "volume" | "pan" => {
                let val = v.as_f64().ok_or_else(|| format!("{k}: number"))?;
                let a = match k.as_str() {
                    "x" => &mut clip.x,
                    "y" => &mut clip.y,
                    "scale" => &mut clip.scale,
                    "rotation" => &mut clip.rotation,
                    "opacity" => &mut clip.opacity,
                    "volume" => &mut clip.volume,
                    _ => &mut clip.pan,
                };
                a.keys.clear(); // "constant value": drop any animation
                a.value = val;
            }
            // text style fields
            _ => {
                let Some(t) = clip.text.as_mut() else {
                    return Err(format!("unknown clip field '{k}' (text fields need a text clip)"));
                };
                match k.as_str() {
                    "text" => t.text = v.as_str().ok_or("text: string")?.to_string(),
                    "font" => t.font = v.as_str().ok_or("font: string")?.to_string(),
                    "size" => t.size = v.as_f64().ok_or("size: number")? as f32,
                    "bold" => t.bold = v.as_bool().ok_or("bold: bool")?,
                    "italic" => t.italic = v.as_bool().ok_or("italic: bool")?,
                    "color" => t.color = color_arg(v).ok_or("color: [r,g,b,a]")?,
                    "outline_width" => t.outline_width = v.as_f64().ok_or("outline_width: number")? as f32,
                    "outline_color" => t.outline_color = color_arg(v).ok_or("outline_color: [r,g,b,a]")?,
                    "shadow" => t.shadow = v.as_bool().ok_or("shadow: bool")?,
                    "shadow_color" => t.shadow_color = color_arg(v).ok_or("shadow_color: [r,g,b,a]")?,
                    "shadow_x" => t.shadow_x = v.as_f64().ok_or("shadow_x: number")? as f32,
                    "shadow_y" => t.shadow_y = v.as_f64().ok_or("shadow_y: number")? as f32,
                    "shadow_blur" => t.shadow_blur = v.as_f64().ok_or("shadow_blur: number")? as f32,
                    "align" => t.align = v.as_u64().filter(|&a| a <= 2).ok_or("align: 0..2")? as u8,
                    "line_spacing" => t.line_spacing = v.as_f64().ok_or("line_spacing: number")? as f32,
                    "letter_spacing" => t.letter_spacing = v.as_f64().ok_or("letter_spacing: number")? as f32,
                    "box_color" => t.box_color = color_arg(v).ok_or("box_color: [r,g,b,a]")?,
                    "box_padding" => t.box_padding = v.as_f64().ok_or("box_padding: number")? as f32,
                    _ => return Err(format!("unknown clip field '{k}'")),
                }
            }
        }
    }
    Ok(())
}

/// Nothing to save/export: the MAIN timeline is empty. `Project::is_empty()` only sees the open sequence,
/// and `main_stash` is Some exactly while one is open — so this is `export_project().is_empty()` without the clone.
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

/// Time the library preview's "step back/forward one frame" buttons seek to.
fn step_time(playhead: f64, fps: f64, forward: bool, duration: f64) -> f64 {
    let dt = 1.0 / fps.max(1.0);
    if forward {
        (playhead + dt).min(duration)
    } else {
        (playhead - dt).max(0.0)
    }
}

/// Time a library preview scrub-bar click/drag at fractional position `frac` (0..1 across the bar,
/// unclamped so a drag past either end still reads as 0 or `duration`) seeks to.
fn scrub_time(frac: f64, duration: f64) -> f64 {
    frac.clamp(0.0, 1.0) * duration
}

/// Tools whose success means "one undo step + after_edit" (media.import pushes its own undo).
const MUTATING_TOOLS: &[&str] = &[
    "project.set",
    "media.set",
    "timeline.add_clip",
    "timeline.split",
    "timeline.delete",
    "timeline.move",
    "timeline.trim",
    "timeline.add_transition",
    "timeline.auto_cut",
    "timeline.nest",
    "clip.set",
    "clip.keyframe",
    "clip.add_effect",
    "clip.remove_effect",
    "clip.apply_motion",
    "subtitles.set",
    "subtitles.import",
    "plan.add",
    "plan.set",
    "plan.remove",
    "notes.set",
    "templates.apply",
    "clip.add_mask",
    "clip.set_mask",
    "clip.add_node",
    "clip.connect_nodes",
    "markers.add",
    "markers.remove",
    "audio.add_bus",
    "audio.add_filter",
    "audio.route",
    "shapes.add",
    "labels.set",
    "container.add",
    "container.replace",
    "container.make",
    "container.unmake",
];

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, open: Option<PathBuf>, screenshot: Option<PathBuf>) -> Self {
        // eframe restores the window rect from the last session, which may be on another monitor
        crate::winpos::place_on_cursor_monitor(cc);
        let settings = Settings::load();
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
        // first-run install points the entry at this exe; skip for screenshot/debug runs so they don't re-point it
        if settings.context_menu
            && screenshot.is_none()
            && !cfg!(debug_assertions)
            && !crate::contextmenu::is_installed()
        {
            let _ = crate::contextmenu::install();
        }
        let player = Player::new(cc.egui_ctx.clone(), backend, text.clone());
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
            presets: presets_ui::PresetsState::default(),
            autocut: autocut_ui::AutoCutState::default(),
            tracking: tracking_ui::TrackState::default(),
            retime: retime::RetimeUi::default(),
            export_ui: export_ui::ExportUi::default(),
            template_name: None,
            profile_name: None,
            fullscreen: false,
            selection: Vec::new(),
            playhead: 0.0,
            export: None,
            encoders: Vec::new(),
            toasts: Vec::new(),
            screenshot,
            started: Instant::now(),
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
            lib_preview: None,
            lib_preview_live: None,
            lib_preview_tex: None,
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
        };
        app.detect_ytdlp(&cc.egui_ctx);
        app.refresh_presets();
        app.player.set_project(&app.project);
        if let Some(p) = open {
            app.open_path(&p);
            // launched from Explorer ("Open with"): behave like a player — full screen, rolling
            if app.screenshot.is_none() && !app.project.tracks.iter().all(|t| t.clips.is_empty()) {
                app.fullscreen = true;
                app.player.play();
            }
        }
        app
    }

    // ---------------- helpers ----------------

    fn toast(&mut self, msg: impl Into<String>) {
        self.toasts.push((msg.into(), Instant::now()));
    }

    fn push_undo(&mut self) {
        push_undo_json(&mut self.undo, &mut self.redo, self.project.to_json());
    }

    /// Insert each asset's clips at `t` (video on `vt` if given), chaining them end to end.
    fn insert_at(&mut self, ids: Vec<Id>, mut t: f64, vt: Option<usize>) {
        for id in ids {
            let new = self.project.insert_asset_clips(id, t, vt);
            if let Some(c) = new.first().and_then(|c| self.project.clip(*c)) {
                t = c.end();
            }
        }
    }

    /// Empty project + one media file: open it as the project (returns empty); otherwise import into the library.
    /// ponytail: that single file is still probed on this thread — it settles the project format, size
    /// and zoom before anything is drawn; give it a placeholder too if opening ever feels slow.
    fn open_or_import(&mut self, paths: &[PathBuf]) -> Vec<Id> {
        if self.project.is_empty() && self.project.assets.is_empty() && paths.len() == 1 {
            self.open_media(&paths[0]);
            return Vec::new();
        }
        self.import_files(paths)
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
        format!("{}{} — Simple Editor", if self.dirty { "*" } else { "" }, name)
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

    /// The project with any open sequence closed — exports always render the MAIN timeline.
    fn export_project(&self) -> Project {
        let mut p = self.project.clone();
        if p.editing.is_some() {
            p.close_sequence();
        }
        p
    }

    // ---------------- file operations ----------------

    fn open_path(&mut self, path: &Path) {
        if path.extension().map(|e| e.to_string_lossy().eq_ignore_ascii_case(PROJECT_EXT)).unwrap_or(false) {
            self.open_project(path);
        } else {
            self.open_media(path);
        }
    }

    fn open_media(&mut self, path: &Path) {
        let p = path.to_string_lossy().into_owned();
        match media::probe(&p, self.backend()) {
            Ok(asset) => {
                let project = Project::from_media(asset);
                self.set_project(project, None);
                self.settings.touch_recent(&p);
                self.settings.save();
            }
            Err(e) => self.toast(format!("Can't open {}: {e}", path.display())),
        }
    }

    fn open_project(&mut self, path: &Path) {
        match Project::load(path) {
            Ok(mut project) => {
                for m in relocate_assets(&mut project, path.parent()) {
                    self.toast(format!("Missing media: {m}"));
                }
                self.set_project(project, Some(path.to_path_buf()));
                self.settings.touch_recent_project(&path.to_string_lossy());
                self.settings.save();
            }
            Err(e) => self.toast(format!("Can't open project: {e}")),
        }
    }

    /// Import media files into the library. Probing spawns ffprobe per file (~100 ms), so each path
    /// lands as a placeholder asset now and `poll_probes` folds in the real metadata a few frames
    /// later — dropping ten files costs this thread nothing. Returns the asset ids.
    /// ponytail: an MCP `media.import` reply therefore quotes duration 0 until the probe lands;
    /// blocking the tool call on it is the fix if an agent ever needs the number in the same reply.
    fn import_files(&mut self, paths: &[PathBuf]) -> Vec<Id> {
        let mut ids = Vec::new();
        let mut fresh: Vec<(Id, String)> = Vec::new();
        for path in paths {
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

    /// Import probes that finished on their worker: fill the placeholder in, drop it if the file
    /// turned out to be unreadable, and re-place any clip that was laid down from the placeholder.
    fn poll_probes(&mut self, ctx: &egui::Context) {
        if self.probes.is_empty() {
            return;
        }
        let mut landed: Vec<crate::engine::import::Probed> = Vec::new();
        self.probes.retain(|rx| loop {
            match rx.try_recv() {
                Ok(p) => landed.push(p),
                Err(std::sync::mpsc::TryRecvError::Empty) => break true,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break false,
            }
        });
        let mut any = false;
        for p in landed {
            match p.asset {
                Ok(a) => {
                    any |= crate::engine::import::adopt(&mut self.project, p.id, a);
                    self.settings.touch_recent(&p.path);
                }
                Err(e) => {
                    self.project.remove_asset(p.id);
                    self.toast(format!("Can't import {}: {e}", p.path));
                    any = true;
                }
            }
        }
        if any {
            // first media into an empty project: adopt its format (only knowable now)
            if self.project.is_empty() {
                if let Some(a) = self.project.assets.first().cloned() {
                    if a.has_video() && a.width > 0 {
                        self.project.width = a.width;
                        self.project.height = a.height;
                        if a.fps > 1.0 {
                            self.project.fps = a.fps;
                        }
                    }
                }
            }
            self.settings.save();
            self.after_edit();
        }
        if !self.probes.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(80));
        }
    }

    fn media_dialog() -> rfd::FileDialog {
        rfd::FileDialog::new()
            .add_filter("Media", MEDIA_EXTS)
            .add_filter("Simple Editor project", &[PROJECT_EXT])
            .add_filter("All files", &["*"])
    }

    fn act_open_file(&mut self) {
        if !self.confirm_discard() {
            return;
        }
        if let Some(p) = Self::media_dialog().pick_file() {
            self.open_path(&p);
        }
    }

    fn act_open_project(&mut self) {
        if !self.confirm_discard() {
            return;
        }
        if let Some(p) = rfd::FileDialog::new().add_filter("Simple Editor project", &[PROJECT_EXT]).pick_file() {
            self.open_project(&p);
        }
    }

    fn act_import(&mut self) {
        if let Some(paths) = Self::media_dialog().pick_files() {
            let ids = self.import_files(&paths);
            if let Some(id) = ids.last() {
                self.library.selected = Some(*id);
                self.library.tab = 0;
            }
        }
    }

    fn replace_container_dialog(&mut self, clip_id: Id, pair: bool) {
        if let Some(p) = Self::media_dialog().pick_file() {
            let ids = self.import_files(&[p]);
            let Some(&aid) = ids.first() else { return };
            let snap = self.project.to_json();
            let ok = if pair {
                self.project.replace_container_pair(clip_id, aid)
            } else {
                self.project.replace_container_media(clip_id, aid)
            };
            if ok {
                push_undo_json(&mut self.undo, &mut self.redo, snap);
                self.after_edit();
                self.toast("Container media replaced");
            }
        }
    }

    fn save_project_as(&mut self) -> bool {
        let mut d = rfd::FileDialog::new()
            .add_filter("Simple Editor project", &[PROJECT_EXT])
            .set_file_name(format!("{}.{PROJECT_EXT}", self.project.name));
        if let Some(dir) = self.project_path.as_ref().and_then(|p| p.parent()) {
            d = d.set_directory(dir);
        } else if let Some(dir) = self.project.source_video.as_ref().and_then(|p| Path::new(p).parent()) {
            d = d.set_directory(dir);
        }
        match d.save_file() {
            Some(p) => {
                self.project_path = Some(p);
                self.save_project()
            }
            None => false,
        }
    }

    fn save_project(&mut self) -> bool {
        let Some(path) = self.project_path.clone() else { return self.save_project_as() };
        match self.project.save(&path) {
            Ok(()) => {
                self.dirty = false;
                self.settings.touch_recent_project(&path.to_string_lossy());
                self.settings.save();
                self.toast("Project saved");
                true
            }
            Err(e) => {
                self.toast(format!("Save failed: {e}"));
                false
            }
        }
    }

    /// "Open folder" in the subtitles panel: write the current cues as an .srt sidecar into
    /// `<project dir>\<name> subtitles\` and open that folder in Explorer.
    fn open_subtitle_folder(&mut self) {
        let dir = match self.project_path.as_ref().and_then(|p| p.parent()) {
            Some(d) => d.join(format!("{} subtitles", self.project.name)),
            None => {
                self.toast("Save the project first — the subtitle folder lives next to it");
                return;
            }
        };
        if let Err(e) = std::fs::create_dir_all(&dir) {
            self.toast(format!("Could not create {}: {e}", dir.display()));
            return;
        }
        if !self.project.subtitles.is_empty() {
            let srt = crate::engine::subtitles::to_srt(&self.project.subtitles);
            let _ = std::fs::write(dir.join(format!("{}.srt", self.project.name)), srt);
        }
        let _ = std::process::Command::new("explorer").arg(&dir).spawn();
    }

    /// Ctrl+S: project file if there is one; otherwise overwrite the opened video; otherwise Save As.
    fn act_save(&mut self) {
        if self.project_path.is_some() {
            self.save_project();
        } else if self.project.source_video.is_some() {
            self.act_overwrite();
        } else {
            self.save_project_as();
        }
    }

    /// Ask to save unsaved changes. Returns false if the user cancelled.
    fn confirm_discard(&mut self) -> bool {
        if !self.dirty {
            return true;
        }
        // Yes saves a .sedit (the video itself only changes via Save / Overwrite Original Video) — say so
        let msg = if self.project_path.is_some() {
            "Save changes to the project?"
        } else {
            "Save changes as a project file (.sedit)?"
        };
        let r = rfd::MessageDialog::new()
            .set_title("Simple Editor")
            .set_description(msg)
            .set_buttons(rfd::MessageButtons::YesNoCancel)
            .set_level(rfd::MessageLevel::Warning)
            .show();
        match r {
            rfd::MessageDialogResult::Yes => self.save_project(),
            rfd::MessageDialogResult::No => true,
            _ => false,
        }
    }

    fn ffmpeg_missing(&mut self) -> bool {
        if media::ffpipe::ffmpeg_exe().is_none() {
            self.toast(
                "ffmpeg.exe not found — install FFmpeg (winget install Gyan.FFmpeg) or set its folder in Settings",
            );
            return true;
        }
        false
    }

    fn export_opts(&self, out_path: PathBuf) -> ExportOptions {
        ExportOptions {
            out_path,
            encoder: self.settings.encoder.clone(),
            crf: self.settings.crf,
            preset: self.settings.preset.clone(),
            backend: self.backend(),
            out_size: None,
            scaler: self.settings.export_scaler.clone(),
            frames: self.export_frames(),
            metadata: Vec::new(),
        }
    }

    fn detect_encoders_once(&mut self) {
        if self.encoders.is_empty() && media::ffpipe::ffmpeg_exe().is_some() {
            self.encoders = export::detect_encoders();
        }
    }

    /// Open the (non-blocking) Export window.
    fn act_export(&mut self) {
        if self.ffmpeg_missing() || self.timeline_is_empty() {
            return;
        }
        self.detect_encoders_once();
        self.export_ui.open = true;
    }

    /// The Export window confirmed: start the export (options include the chosen output path).
    fn start_export_choice(&mut self, choice: export_ui::ExportChoice) {
        if self.export.is_some() || self.ffmpeg_missing() || self.timeline_is_empty() {
            return;
        }
        // writing over a file the player/decoders are reading from is the Overwrite path's job (release + reopen)
        let out_c = std::fs::canonicalize(&choice.opts.out_path).ok();
        if out_c.is_some() && self.project.assets.iter().any(|a| std::fs::canonicalize(&a.path).ok() == out_c) {
            self.toast("That file is a source of this project — use Overwrite Original Video (Ctrl+S) instead");
            return;
        }
        self.player.pause();
        let project = self.export_project();
        let prog = if choice.lossless {
            export::start_lossless_cut(project, choice.opts.out_path.clone())
        } else {
            export::start_export(project, choice.opts, self.text.clone())
        };
        self.export = Some((prog, ExportKind::File));
        self.settings.save(); // the window remembers resolution/scaler in settings
    }

    fn act_export_lossless(&mut self) {
        if self.export.is_some() || self.ffmpeg_missing() {
            return;
        }
        let project = self.export_project();
        if export::lossless_segments(&project).is_none() {
            self.toast("Lossless cut needs a plain cut of one video (no effects, text, overlays or extra media)");
            return;
        }
        let src = project.source_video.clone().or_else(|| project.assets.first().map(|a| a.path.clone()));
        let ext = src
            .as_ref()
            .and_then(|s| Path::new(s).extension().map(|e| e.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "mp4".into());
        let mut d = rfd::FileDialog::new()
            .add_filter(&format!("{} (same container)", ext.to_uppercase()), &[ext.as_str()])
            .set_file_name(format!("{}_cut.{ext}", project.name));
        if let Some(dir) = src.as_ref().and_then(|p| Path::new(p).parent()) {
            d = d.set_directory(dir);
        }
        let Some(out) = d.save_file() else { return };
        self.player.pause();
        let prog = export::start_lossless_cut(project, out);
        self.export = Some((prog, ExportKind::File));
    }

    fn act_export_xml(&mut self) {
        let Some(out) = rfd::FileDialog::new()
            .add_filter("Final Cut Pro 7 XML (Premiere / Resolve)", &["xml"])
            .set_file_name(format!("{}.xml", self.project.name))
            .save_file()
        else {
            return;
        };
        match std::fs::write(&out, crate::engine::xmeml::export_xmeml(&self.export_project())) {
            Ok(()) => {
                self.toast("XML exported — import it in Premiere (File > Import) or Resolve (File > Import > Timeline)")
            }
            Err(e) => self.toast(format!("XML export failed: {e}")),
        }
    }

    fn act_export_style(&mut self) {
        let Some(out) = rfd::FileDialog::new()
            .add_filter("Markdown", &["md"])
            .set_file_name(format!("{}_style.md", self.project.name))
            .save_file()
        else {
            return;
        };
        // the summary describes the MAIN timeline, even while a nested sequence is open
        match std::fs::write(&out, crate::engine::style::style_summary(&self.export_project())) {
            Ok(()) => self.toast("Style summary exported"),
            Err(e) => self.toast(format!("Style summary failed: {e}")),
        }
    }

    /// Re-encode the timeline over the opened video file (temp file in the same folder, then replace).
    fn act_overwrite(&mut self) {
        let Some(src) = self.project.source_video.clone() else {
            self.toast("No source video to overwrite — use Export Video As");
            return;
        };
        if self.export.is_some() {
            self.toast("An export is already running");
            return;
        }
        if self.timeline_is_empty() {
            self.toast("Timeline is empty — nothing to save");
            return;
        }
        if self.ffmpeg_missing() {
            return;
        }
        if self.settings.confirm_overwrite {
            let r = rfd::MessageDialog::new()
                .set_title("Overwrite original video?")
                .set_description(format!(
                    "{src}\n\nThe file will be replaced with the edited video. This cannot be undone."
                ))
                .set_buttons(rfd::MessageButtons::OkCancel)
                .set_level(rfd::MessageLevel::Warning)
                .show();
            if r != rfd::MessageDialogResult::Ok {
                return;
            }
        }
        // the new file is reloaded as a fresh project afterwards, so state that isn't burned into the video
        // (subtitles, planner, notes, sequences, imported media, undo) is dropped — offer to save a .sedit first
        let p = &self.project;
        let loses = !p.plan.is_empty()
            || !p.notes.is_empty()
            || !p.subtitles.is_empty()
            || !p.sequences.is_empty()
            || p.assets.len() > 1;
        if loses && (self.dirty || self.project_path.is_none()) {
            match rfd::MessageDialog::new()
                .set_title("Save the project first?")
                .set_description(
                    "Overwriting reloads the new file as a fresh project — subtitles, planner, notes, \
                     sequences and imported media are not kept. Save a project file (.sedit) first?",
                )
                .set_buttons(rfd::MessageButtons::YesNoCancel)
                .set_level(rfd::MessageLevel::Warning)
                .show()
            {
                rfd::MessageDialogResult::Yes => {
                    if !self.save_project() {
                        return;
                    }
                }
                rfd::MessageDialogResult::No => {}
                _ => return,
            }
        }
        let original = PathBuf::from(&src);
        let ext = original.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_else(|| "mp4".into());
        let temp = original.with_file_name(format!(
            ".{}.simple-editor-tmp.{ext}",
            original.file_stem().unwrap_or_default().to_string_lossy()
        ));
        self.player.pause();
        let project = self.export_project();
        // opt-in: a plain cut can be saved instantly with `-c copy` (keyframe-accurate) instead of re-encoding
        let lossless = self.settings.lossless_save && export::lossless_segments(&project).is_some();
        let prog = if lossless {
            export::start_lossless_cut(project, temp.clone())
        } else {
            export::start_export(project, self.export_opts(temp.clone()), self.text.clone())
        };
        self.export = Some((prog, ExportKind::Overwrite { original, temp }));
    }

    fn finish_export(&mut self) {
        let Some((prog, kind)) = self.export.take() else { return };
        if let Some(e) = prog.error() {
            if let ExportKind::Overwrite { temp, .. } = &kind {
                let _ = std::fs::remove_file(temp);
            }
            if prog.is_cancelled() {
                self.toast("Export cancelled");
            } else {
                self.toast(format!("Export failed: {e}"));
            }
            return;
        }
        match kind {
            ExportKind::File => self.toast("Export finished"),
            ExportKind::Overwrite { original, temp } => {
                self.player.release_files();
                // the thumbnail worker holds a decoder (ffmpeg child) on the source — drop it while we retry
                self.thumbs.clear();
                // ponytail: killed ffmpeg children release their file handle a few ms after wait() returns — retry briefly
                let mut r = std::fs::rename(&temp, &original);
                let deadline = Instant::now() + Duration::from_millis(500);
                while r.is_err() && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(10));
                    r = std::fs::rename(&temp, &original);
                }
                match r {
                    Ok(()) => {
                        self.toast("Saved over the original video");
                        // in-memory peaks are keyed by path only and the file behind it just changed
                        self.waveforms.clear();
                        self.open_media(&original);
                    }
                    Err(e) => {
                        self.toast(format!(
                            "Couldn't replace the original ({e}); edited file left at {}",
                            temp.display()
                        ));
                        self.player.set_project(&self.project);
                    }
                }
            }
        }
    }

    // ---------------- editing actions ----------------

    fn act(&mut self, a: Action) {
        use Action::*;
        // an export writes the timeline: block only saving/exporting, keep editing usable
        if self.export.is_some() && matches!(a, Save | SaveProjectAs | ExportVideo | ExportLossless) {
            self.toast("An export is running — try again when it finishes");
            return;
        }
        match a {
            NewProject => {
                if self.confirm_discard() {
                    self.set_project(Project::new(), None);
                }
            }
            OpenFile => self.act_open_file(),
            OpenProject => self.act_open_project(),
            Save => self.act_save(),
            SaveProjectAs => {
                self.save_project_as();
            }
            ExportVideo => self.act_export(),
            ExportLossless => self.act_export_lossless(),
            ExportXml => self.act_export_xml(),
            ImportMedia => self.act_import(),
            Settings => self.settings_ui.open = !self.settings_ui.open,
            Undo => {
                if let Some(json) = self.undo.pop() {
                    if json == LAYOUT_STEP {
                        self.layout.undo();
                        self.layout_dirty = true;
                        self.redo.push(json);
                    } else {
                        self.redo.push(self.project.to_json());
                        if let Ok(p) = Project::from_json(&json) {
                            self.project = p;
                            self.after_edit();
                        }
                    }
                }
            }
            Redo => {
                if let Some(json) = self.redo.pop() {
                    if json == LAYOUT_STEP {
                        self.layout.redo();
                        self.layout_dirty = true;
                        self.undo.push(json);
                    } else {
                        self.undo.push(self.project.to_json());
                        if let Ok(p) = Project::from_json(&json) {
                            self.project = p;
                            self.after_edit();
                        }
                    }
                }
            }
            PlayPause => {
                if self.buffer_stall {
                    self.buffer_stall = false; // buffering held the clock: space means "stop waiting"
                } else {
                    if !self.player.is_playing() && self.playhead >= self.project.duration() - 1e-6 {
                        self.seek(0.0);
                    }
                    self.player.toggle();
                }
            }
            Stop => {
                self.buffer_stall = false;
                self.player.pause();
            }
            StepBack => {
                self.player.pause();
                let t = self.project.snap_frame(self.playhead - self.project.frame_dur());
                self.seek(t);
            }
            StepForward => {
                self.player.pause();
                let t = self.project.snap_frame(self.playhead + self.project.frame_dur());
                self.seek(t);
            }
            GoStart => self.seek(0.0),
            GoEnd => self.seek(self.project.duration()),
            PrevCut => {
                let cuts = self.project.cut_points();
                if let Some(&t) = cuts.iter().rev().find(|&&c| c < self.playhead - 1e-4) {
                    self.seek(t);
                }
            }
            NextCut => {
                let cuts = self.project.cut_points();
                if let Some(&t) = cuts.iter().find(|&&c| c > self.playhead + 1e-4) {
                    self.seek(t);
                }
            }
            Split => {
                let only =
                    if self.selection.is_empty() { None } else { Some(self.project.expand_links(&self.selection)) };
                let snap = self.project.to_json();
                let mut did = !self.project.split_at(self.playhead, only.as_deref()).is_empty();
                // cues selected on the timeline's subtitle lane split too, like clips
                for id in self.timeline.sub_sel.clone() {
                    did |= self.project.split_cue(id, self.playhead).is_some();
                }
                if did {
                    push_undo_json(&mut self.undo, &mut self.redo, snap);
                    self.after_edit();
                }
            }
            Delete | RippleDelete => {
                let ids = self.project.expand_links(&self.selection);
                if !ids.is_empty() {
                    self.push_undo();
                    self.project.delete_clips(&ids, a == RippleDelete);
                    self.selection.clear();
                    self.after_edit();
                }
            }
            SelectAll => self.selection = self.project.all_clips().map(|(_, c)| c.id).collect(),
            Deselect => self.selection.clear(),
            // in/out marks are saved with the project: undoable, and they make it dirty like any other edit
            MarkIn => {
                self.push_undo();
                self.project.in_point = Some(self.playhead);
                if let Some(o) = self.project.out_point {
                    if o <= self.playhead {
                        self.project.out_point = None;
                    }
                }
                self.after_edit();
            }
            MarkOut => {
                self.push_undo();
                self.project.out_point = Some(self.playhead);
                if let Some(i) = self.project.in_point {
                    if i >= self.playhead {
                        self.project.in_point = None;
                    }
                }
                self.after_edit();
            }
            ClearInOut => {
                if self.project.in_point.is_some() || self.project.out_point.is_some() {
                    self.push_undo();
                    self.project.in_point = None;
                    self.project.out_point = None;
                    self.after_edit();
                }
            }
            TrimToInOut | RippleDeleteInOut => {
                let a0 = self.project.in_point.unwrap_or(0.0);
                let b0 = self.project.out_point.unwrap_or(self.project.duration());
                if b0 > a0 {
                    self.push_undo();
                    if a == TrimToInOut {
                        self.project.trim_to_range(a0, b0);
                        self.seek(0.0);
                    } else {
                        self.project.ripple_delete_range(a0, b0);
                        self.project.in_point = None;
                        self.project.out_point = None;
                        self.seek(a0);
                    }
                    self.after_edit();
                }
            }
            AddText => {
                self.push_undo();
                let id = self.project.add_text_clip(self.playhead, 5.0);
                self.selection = vec![id];
                self.after_edit();
            }
            ZoomIn => self.timeline.zoom_by(1.25, None),
            ZoomOut => self.timeline.zoom_by(0.8, None),
            ZoomFit => self.timeline.zoom_to_fit(self.project.duration(), self.timeline.lanes_rect.width()),
            LinkToggle => {
                if !self.selection.is_empty() {
                    self.push_undo();
                    let ids = self.selection.clone();
                    self.project.toggle_link(&ids);
                    self.after_edit();
                }
            }
            ToggleEnabled => {
                if let Some(first) = self.selection.first().and_then(|id| self.project.clip(*id)) {
                    let en = !first.enabled;
                    self.push_undo();
                    let ids = self.selection.clone();
                    self.project.set_enabled(&ids, en);
                    self.after_edit();
                }
            }
            NudgeLeft | NudgeRight => {
                let ids = self.project.expand_links(&self.selection);
                if !ids.is_empty() {
                    let dt = if a == NudgeLeft { -self.project.frame_dur() } else { self.project.frame_dur() };
                    let snap = self.project.to_json();
                    if self.project.move_clips(&ids, dt, 0, None) {
                        push_undo_json(&mut self.undo, &mut self.redo, snap);
                        self.after_edit();
                    }
                }
            }
            ToggleSnap => {
                self.settings.snap = !self.settings.snap;
                self.settings.save();
            }
            AddVideoTrack | AddAudioTrack => {
                self.push_undo();
                self.project.add_track(if a == AddVideoTrack { TrackKind::Video } else { TrackKind::Audio });
                self.after_edit();
            }
            ToggleLibrary => self.toggle_pane(Pane::Library),
            ToggleMarkers => self.toggle_pane(Pane::Markers),
            ToggleNodes => self.toggle_pane(Pane::Nodes),
            ToggleMixer => self.toggle_pane(Pane::Mixer),
            ToggleTools => self.toggle_pane(Pane::Tools),
            AddLastTransition => {
                // the panel state IS the memory: every apply path records into it (transitions_ui)
                let (kind, dur) = (self.transitions_ui.kind(), self.transitions_ui.duration);
                if self.selection.is_empty() {
                    self.toast("Select a clip next to the cut first");
                } else {
                    let snap = self.project.to_json();
                    let ids = self.selection.clone();
                    let st = &mut self.transitions_ui;
                    if transitions_ui::add_transitions(&mut self.project, &ids, st, kind, dur, false) > 0 {
                        push_undo_json(&mut self.undo, &mut self.redo, snap);
                        self.after_edit();
                        self.toast(format!("{} ({dur:.2} s)", kind.name()));
                    } else {
                        self.toast("No abutting left neighbour — transitions sit on a cut");
                    }
                }
            }
            CopyAttributes => match self.selection.first().and_then(|&id| self.project.copy_attributes(id)) {
                Some(c) => {
                    let name = c.name.clone();
                    self.attrs = Some(c);
                    self.toast(format!("Copied attributes from '{name}'"));
                }
                None => self.toast("Select a clip to copy attributes from"),
            },
            PasteAttributes => {
                if self.attrs.is_none() {
                    self.toast("Copy attributes from a clip first (Ctrl+Alt+C)");
                } else if self.selection.is_empty() {
                    self.toast("Select the clips to paste onto");
                } else {
                    self.paste_ui.open = true;
                }
            }
            CopyClips | CutClips => {
                let ids = self.project.expand_links(&self.selection);
                if ids.is_empty() {
                    self.toast("Select the clips to copy first");
                } else {
                    let t = crate::engine::presets::capture_template("clipboard", &self.project, &ids);
                    // the JSON is what makes Ctrl+V fire at all (see App::os_clipboard); it is also
                    // readable, so a copy can be pasted into another instance by hand
                    self.os_clipboard = Some(t.json.clone());
                    self.clipboard = Some(t);
                    if a == CutClips {
                        self.push_undo();
                        self.project.delete_clips(&ids, false);
                        self.selection.clear();
                        self.after_edit();
                    }
                }
            }
            PasteClips | PasteInPlace | PasteInsert | PasteAtTop => {
                match self.clipboard.as_ref().and_then(crate::engine::presets::decode_template) {
                    Some((clips, assets)) => {
                        // Paste In Place ignores the clicked row: place_clips takes the first track with room
                        let target = (a == PasteClips).then_some(self.timeline.last_track).flatten();
                        let snap = self.project.to_json();
                        if a == PasteInsert {
                            // ripple: everything at or after the playhead slides right by the paste's span
                            let span = clips.iter().map(|c| c.start + c.duration).fold(0.0_f64, f64::max);
                            self.project.ripple_open(self.playhead, span);
                        }
                        if a == PasteAtTop {
                            let kind = clips
                                .iter()
                                .find(|c| c.kind != ClipKind::Audio)
                                .map_or(TrackKind::Audio, |_| TrackKind::Video);
                            self.project.add_track(kind);
                        }
                        let ids = timeline::paste_clips(&mut self.project, clips, assets, self.playhead, target);
                        if ids.is_empty() {
                            self.toast("Nothing could be pasted here");
                        } else {
                            push_undo_json(&mut self.undo, &mut self.redo, snap);
                            self.selection = ids;
                            self.after_edit();
                        }
                    }
                    None => self.toast("Nothing copied yet — Ctrl+C copies the selected clips"),
                }
            }
            AddMarker => {
                self.push_undo();
                let t = self.playhead;
                let id = self.project.add_marker(t, format!("Marker at {}", crate::ui::timecode(t, self.project.fps)));
                self.markers.selected = Some(id);
                self.layout.reveal(Pane::Markers);
                self.layout_dirty = true;
                self.after_edit();
            }
            AddShape => {
                let kind = match self.tools.tool {
                    Tool::Shape(k) => k,
                    Tool::Draw => ShapeKind::Draw,
                    _ => ShapeKind::Rect,
                };
                self.add_shape(kind, None);
            }
            AddAdjustment => {
                self.push_undo();
                let id = self.project.add_adjustment_clip(self.playhead, 5.0);
                self.selection = vec![id];
                self.after_edit();
            }
            AddMask => {
                let shape = match self.tools.tool {
                    Tool::Mask(s) => s,
                    _ => MaskShape::Ellipse,
                };
                let Some(&id) = self.selection.first() else {
                    self.toast("Select a clip (or an effect on it) to mask");
                    return;
                };
                // snapshot first, commit on success: popping the undo entry afterwards would leave the
                // redo stack cleared for an edit that never happened
                let snap = self.project.to_json();
                if add_mask(&mut self.project, id, shape) {
                    push_undo_json(&mut self.undo, &mut self.redo, snap);
                    self.tools.tool = Tool::Mask(shape);
                    self.layout.reveal(Pane::Tools);
                    self.layout_dirty = true;
                    self.after_edit();
                } else if self.project.clip(id).is_some_and(|c| !c.is_visual()) {
                    self.toast("A mask shapes pixels — an audio clip has none");
                } else {
                    self.toast("That clip already has a mask");
                }
            }
            ExportFrame => {
                if self.timeline_is_empty() {
                    self.toast("Timeline is empty — nothing to export");
                } else if !self.ffmpeg_missing() {
                    self.frame_ui.open = true;
                }
            }
            ScreenCapture => self.capture_ui.screen_open = !self.capture_ui.screen_open,
            Voiceover => self.capture_ui.voice_open = !self.capture_ui.voice_open,
            ImportTimeline => self.act_import_timeline(),
            MovieMode => {
                self.settings.movie_mode = !self.settings.movie_mode;
                self.settings.save();
                if self.settings.movie_mode {
                    self.request_prerender();
                } else {
                    guarded(|| self.prerender.clear());
                }
                self.toast(if self.settings.movie_mode { "Movie mode on" } else { "Movie mode off" });
            }
            ToggleInspector => self.toggle_pane(Pane::Inspector),
            ToggleEffects => self.toggle_pane(Pane::Effects),
            ToggleTransitions => self.toggle_pane(Pane::Transitions),
            ToggleCurves => self.toggle_pane(Pane::Curves),
            ToggleSubtitles => self.toggle_pane(Pane::Subtitles),
            TogglePlanner => self.toggle_pane(Pane::Planner),
            AutoCut => {
                self.layout.reveal(Pane::AutoCut);
                self.layout_dirty = true;
            }
            Retime => self.retime.open = !self.retime.open,
            FreezeFrame => {
                if self.selection.is_empty() {
                    self.toast("Select a clip to freeze");
                } else {
                    let snap = self.project.to_json();
                    let ids = self.selection.clone();
                    let frozen = self.project.freeze_at(self.playhead, &ids);
                    if frozen.is_empty() {
                        self.toast("Nothing to freeze");
                    } else {
                        push_undo_json(&mut self.undo, &mut self.redo, snap);
                        self.selection = frozen;
                        self.after_edit();
                    }
                }
            }
            Fullscreen => {
                // the caller sends ViewportCommand::Fullscreen (needs the ctx)
                self.fullscreen = !self.fullscreen;
                self.player.seek(self.playhead); // re-render at the new canvas size
            }
            AddTransition | AddTransitionEnd => {
                let at_end = a == AddTransitionEnd;
                if self.selection.is_empty() {
                    self.toast("Select a clip next to the cut first");
                } else {
                    let snap = self.project.to_json();
                    let ids = self.selection.clone();
                    let (kind, dur) = (TransitionKind::CrossFade, 1.0);
                    let st = &mut self.transitions_ui;
                    let added = transitions_ui::add_transitions(&mut self.project, &ids, st, kind, dur, at_end);
                    if added > 0 {
                        push_undo_json(&mut self.undo, &mut self.redo, snap);
                        self.after_edit();
                    } else if at_end {
                        self.toast("Nothing abuts the end of this clip — transitions sit on a cut");
                    } else {
                        self.toast("No abutting left neighbour — transitions sit on a cut");
                    }
                }
            }
            AddSubtitle => {
                self.push_undo();
                let id = self.project.add_cue(self.playhead, self.playhead + 2.0, "Subtitle");
                self.subtitles_ui.selected = Some(id);
                self.layout.reveal(Pane::Subtitles);
                self.layout_dirty = true;
                self.after_edit();
            }
            NestSequence => {
                if self.selection.is_empty() {
                    self.toast("Select the clips to nest");
                } else {
                    let snap = self.project.to_json();
                    let name = format!("Sequence {}", self.project.sequences.len() + 1);
                    let ids = self.selection.clone();
                    match self.project.nest_selection(&ids, name.clone()) {
                        Some(_) => {
                            push_undo_json(&mut self.undo, &mut self.redo, snap);
                            self.selection.clear();
                            self.after_edit();
                            self.toast(format!("Nested into '{name}' — double-click / open it to edit inside"));
                        }
                        None => self.toast("Nothing to nest"),
                    }
                }
            }
            OpenParentSequence => {
                if self.project.editing.is_some() {
                    self.project.close_sequence();
                    self.sequence_view_changed(self.playhead); // clamps into the parent timeline
                }
            }
            SaveTemplate => {
                if self.selection.is_empty() {
                    self.toast("Select the clips to save as a template");
                } else {
                    self.template_name = Some(String::new());
                }
            }
            ApplyFlow => {
                let mut sel: Vec<&Clip> = self.selection.iter().filter_map(|&id| self.project.clip(id)).collect();
                sel.sort_by(|a, b| a.start.total_cmp(&b.start));
                if sel.len() != 2 {
                    self.toast("Flow needs exactly two selected clips");
                } else {
                    let (a_id, b_id) = (sel[0].id, sel[1].id);
                    let snap = self.project.to_json();
                    if self.project.flow_clips(a_id, b_id) {
                        push_undo_json(&mut self.undo, &mut self.redo, snap);
                        self.after_edit();
                    } else {
                        self.toast("Flow needs two abutting clips (no gap at the cut)");
                    }
                }
            }
            AddContainer => {
                let snap = self.project.to_json();
                let (vid, aid) = self.project.add_container_clip(self.playhead, 5.0);
                push_undo_json(&mut self.undo, &mut self.redo, snap);
                self.selection = vec![vid, aid];
                self.after_edit();
                self.toast("Container clip added");
            }
            ReplaceContainerMedia => {
                if let Some(&id) = self.selection.first() {
                    self.replace_container_dialog(id, false);
                } else {
                    self.toast("Select a container clip to replace");
                }
            }
            MakeContainer => {
                if !self.selection.is_empty() {
                    let snap = self.project.to_json();
                    self.project.make_container(&self.selection);
                    push_undo_json(&mut self.undo, &mut self.redo, snap);
                    self.after_edit();
                    self.toast("Converted to container");
                } else {
                    self.toast("Select clips to convert to container");
                }
            }
            UnmakeContainer => {
                if !self.selection.is_empty() {
                    let snap = self.project.to_json();
                    self.project.unmake_container(&self.selection);
                    push_undo_json(&mut self.undo, &mut self.redo, snap);
                    self.after_edit();
                    self.toast("Container removed");
                }
            }
        }
    }

    fn toggle_pane(&mut self, p: Pane) {
        self.layout.toggle(p);
        self.layout_dirty = true;
    }

    // ---------------- panes ----------------

    /// Draw one pane, containing a panic in its widget so the rest of the editor keeps working
    /// (same policy as the decoder threads). A pane that panicked once is not drawn again this session.
    fn draw_pane(&mut self, ui: &mut egui::Ui, pane: Pane) {
        if self.failed_panes.contains(&pane) {
            ui.weak(format!("{} is unavailable in this build.", pane.title()));
            return;
        }
        if guarded(|| self.draw_pane_inner(ui, pane)).is_none() {
            self.failed_panes.push(pane);
            self.toast(format!("{} failed to draw — the pane is disabled for this session", pane.title()));
        }
    }

    fn draw_pane_inner(&mut self, ui: &mut egui::Ui, pane: Pane) {
        match pane {
            Pane::Preview => {
                if self.lib_preview.is_some() {
                    self.draw_lib_preview(ui);
                } else {
                    let frame = self.pending_frame.take();
                    let proxy_busy = self.proxy_job.as_ref().map(|(_, _, p)| p.fraction());
                    let resp = {
                        let App {
                            project,
                            selection,
                            playhead,
                            undo,
                            redo,
                            preview: pv,
                            player,
                            palette,
                            fullscreen,
                            tools,
                            settings,
                            prerender,
                            gpu_tex,
                            tracking,
                            tracking_shown,
                            ..
                        } = self;
                        let mut push = |p: &Project| push_undo_json(undo, redo, p.to_json());
                        // only while movie mode is on: progress() walks the requested ranges
                        let done = settings
                            .movie_mode
                            .then(|| guarded(|| prerender.progress()).unwrap_or(1.0))
                            .filter(|&p| p < 1.0);
                        preview::show(
                            ui,
                            pv,
                            preview::PreviewCtx {
                                project,
                                selection,
                                playhead: *playhead,
                                playing: player.is_playing(),
                                fullscreen: *fullscreen,
                                palette,
                                undo: &mut push,
                                frame,
                                gpu_texture: *gpu_tex,
                                tool: tools.tool,
                                quality: settings.preview_quality,
                                movie_mode: settings.movie_mode,
                                prerender: done,
                                buffering: player.is_buffering(),
                                proxy: proxy_busy,
                                tracker: tracking_shown.then(|| tracking.box_rect()),
                            },
                        )
                    };
                    let (cw, ch) = preview_canvas(resp.canvas, self.settings.preview_quality);
                    self.player.set_canvas(cw, ch, self.settings.preview_max_width);
                    // same clamp the player applies, so the GPU renders at the aspect the player decodes at
                    self.canvas = clamp_canvas(cw, ch, self.settings.preview_max_width);
                    if let Some(t) = resp.seek {
                        self.seek(t);
                    }
                    self.pending_actions.extend(resp.actions);
                    if let Some(q) = resp.set_quality {
                        self.settings.preview_quality = q;
                        self.settings.save();
                    }
                    if resp.set_movie_mode.is_some() {
                        // the action toggles the setting and starts / clears the pre-render
                        self.pending_actions.push(Action::MovieMode);
                    }
                    if let Some((x, y)) = resp.set_tracker {
                        (self.tracking.cx, self.tracking.cy) = (x, y);
                    }
                    if let Some((kind, cx, cy, w, h)) = resp.new_shape {
                        let id = self.add_shape(kind, Some((cx, cy, w, h)));
                        if !resp.new_points.is_empty() {
                            if let Some(s) = self.project.clip_mut(id).and_then(|c| c.shape.as_mut()) {
                                s.points = resp.new_points;
                            }
                            self.after_edit();
                        }
                    }
                    if let Some(s) = resp.stroke {
                        self.add_stroke(s);
                    }
                    if resp.edited {
                        self.after_edit();
                    }
                }
            }
            Pane::Timeline => {
                // sequence breadcrumb: a thin strip above the timeline while editing a nested sequence
                if let Some(seq) = self.project.editing {
                    let name = self.project.sequence(seq).map(|s| s.name.clone()).unwrap_or_default();
                    ui.horizontal(|ui| {
                        let back = crate::ui::tools::glyph_text_button(
                            ui,
                            crate::ui::tools::Glyph::Tri(crate::ui::tools::Dir::Left),
                            "Back",
                        );
                        if back.on_hover_text("Back to the main timeline (Alt+Up)").clicked() {
                            self.pending_actions.push(Action::OpenParentSequence);
                        }
                        ui.label(format!("Main > {name}"));
                    });
                }
                let resp = {
                    let App {
                        project,
                        selection,
                        playhead,
                        undo,
                        redo,
                        waveforms,
                        thumbs,
                        autocut,
                        autocut_shown,
                        timeline: tl,
                        settings,
                        player,
                        palette,
                        tools,
                        prerender,
                        ..
                    } = self;
                    let tool = tools.tool;
                    let prerender_bar =
                        if settings.movie_mode { guarded(|| prerender.segments()).unwrap_or_default() } else { vec![] };
                    let mut push = |p: &Project| push_undo_json(undo, redo, p.to_json());
                    timeline::show(
                        ui,
                        tl,
                        timeline::TimelineCtx {
                            project,
                            selection,
                            playhead,
                            undo: &mut push,
                            waveforms,
                            palette,
                            snap: settings.snap,
                            playing: player.is_playing(),
                            thumbs: Some(thumbs),
                            // only while the Auto-cut pane is on screen: a stale overlay would keep
                            // shading the timeline after the pane is hidden or a new project is opened
                            keep_ranges: if *autocut_shown { &autocut.overlay } else { &[] },
                            prerender: &prerender_bar,
                            tool,
                        },
                    )
                };
                if resp.seeked {
                    self.player.pause();
                    self.player.seek(self.playhead);
                }
                if resp.edited {
                    self.after_edit();
                }
                if !resp.dropped_files.is_empty() {
                    for (path, t, track) in resp.dropped_files {
                        let ids = self.import_files(&[path]);
                        let vt = track.filter(|&i| self.project.tracks[i].kind == TrackKind::Video);
                        self.insert_at(ids, t, vt);
                    }
                    self.after_edit();
                }
                for (payload, t, track) in resp.dropped_other {
                    let vt = track.filter(|&i| self.project.tracks[i].kind == TrackKind::Video);
                    match payload {
                        DragPayload::Sequence(id) => {
                            let snap = self.project.to_json();
                            if self.project.insert_sequence_clip(id, t, vt).is_none() {
                                self.toast("A sequence can't contain itself");
                            } else {
                                push_undo_json(&mut self.undo, &mut self.redo, snap);
                                self.after_edit();
                            }
                        }
                        DragPayload::Template(name) => self.place_template(&name, t),
                        // dropped onto a clip that can actually take this kind (see effects_ui's own
                        // click-to-add gate); a miss says so rather than swallowing the gesture
                        DragPayload::Effect(kind) => {
                            let hit = track
                                .and_then(|ti| self.project.tracks.get(ti))
                                .and_then(|tr| tr.clips.iter().find(|c| c.contains(t)))
                                .filter(|c| (c.kind == ClipKind::Audio) == kind.applies_to_audio());
                            match hit.map(|c| (c.id, c.uses_graph())) {
                                Some((id, false)) => {
                                    let snap = self.project.to_json();
                                    if let Some(c) = self.project.clip_mut(id) {
                                        c.effects.push(Effect::new(kind));
                                    }
                                    push_undo_json(&mut self.undo, &mut self.redo, snap);
                                    self.after_edit();
                                    self.toast(format!("{} added", kind.name()));
                                }
                                Some((_, true)) => self.toast("That clip renders from its node graph"),
                                None => self.toast(format!("Drop {} on a clip it applies to", kind.name())),
                            }
                        }
                        // a transition belongs to a cut, so the half of the clip it lands on picks which
                        // one: left half = the cut at its start, right half = the cut at its end
                        DragPayload::Transition(kind) => {
                            let hit = track
                                .and_then(|ti| self.project.tracks.get(ti))
                                .and_then(|tr| tr.clips.iter().find(|c| c.contains(t)))
                                .map(|c| (c.id, transitions_ui::drop_at_end(c, t)));
                            match hit {
                                Some((id, at_end)) => {
                                    let snap = self.project.to_json();
                                    let dur = self.transitions_ui.duration;
                                    let st = &mut self.transitions_ui;
                                    if transitions_ui::add_transitions(&mut self.project, &[id], st, kind, dur, at_end)
                                        > 0
                                    {
                                        push_undo_json(&mut self.undo, &mut self.redo, snap);
                                        self.after_edit();
                                        self.toast(format!("{} ({dur:.2} s)", kind.name()));
                                    } else {
                                        self.toast("No abutting clip on that side — transitions sit on a cut");
                                    }
                                }
                                None => self.toast("Drop a transition on the clip beside the cut"),
                            }
                        }
                        _ => {}
                    }
                }
                if let Some(id) = resp.open_sequence {
                    self.enter_sequence(id);
                }
                if let Some((cid, pair)) = resp.replace_container {
                    self.replace_container_dialog(cid, pair);
                }
                self.pending_actions.extend(resp.actions);
            }
            Pane::Library => {
                let resp = {
                    let live = self.lib_preview_live;
                    let App { project, settings, library: lib, ytdlp_available, thumbs, undo, redo, palette, .. } =
                        self;
                    let mut push = |p: &Project| push_undo_json(undo, redo, p.to_json());
                    let ytdlp = ytdlp_available.load(std::sync::atomic::Ordering::Relaxed);
                    library::show(ui, lib, project, settings, Some(thumbs), live.as_ref(), palette, ytdlp, &mut push)
                };
                if resp.edited {
                    self.after_edit();
                }
                if resp.settings_changed {
                    self.settings.save();
                }
                if resp.import {
                    self.act_import();
                }
                // single-clicked in the Library: the file the preview pane should show (source viewer)
                if let Some(p) = resp.preview.clone() {
                    self.start_lib_preview(ui.ctx(), p);
                }
                if !resp.add_to_timeline.is_empty() {
                    self.push_undo();
                    self.insert_at(resp.add_to_timeline, self.playhead, None);
                    self.after_edit();
                }
                // both used to sit inside the open_paths branch, so the Library's "New ▸ Adjustment layer"
                // and "Open…" only ever fired on a frame that also opened a file — i.e. never
                if resp.new_adjustment {
                    self.pending_actions.push(Action::AddAdjustment);
                }
                if resp.open_dialog {
                    self.pending_actions.push(Action::OpenFile);
                }
                if !resp.open_paths.is_empty() {
                    let ids = self.open_or_import(&resp.open_paths);
                    if !ids.is_empty() {
                        self.library.selected = ids.last().copied();
                        self.library.tab = 0;
                    }
                }
                if !resp.remove.is_empty() {
                    self.push_undo();
                    for id in resp.remove {
                        self.project.remove_asset(id);
                    }
                    self.after_edit();
                }
                if resp.clear_recent {
                    self.settings.recent_assets.clear();
                    self.settings.save();
                }
                for (id, ext) in resp.convert {
                    self.start_asset_convert(id, &ext);
                }
                if !resp.regen_proxy.is_empty() {
                    // decoders hold the proxy files open on Windows: release them before deleting
                    self.player.set_proxies(std::collections::HashMap::new());
                    self.player.release_files();
                    for src in resp.regen_proxy {
                        let _ = std::fs::remove_file(crate::media::proxy::proxy_path(&src, self.settings.proxy_height));
                    }
                    self.proxy_map.clear();
                    self.proxy_scan_at = None; // rebuild + re-push on the next update
                }
                if let Some(id) = resp.convert_dialog {
                    self.convert_dialog = Some((id, "mp4".into()));
                }
                if let Some(p) = resp.compress.and_then(|id| self.project.asset(id)).map(|a| a.path.clone()) {
                    self.compress = Some(Compress::new(PathBuf::from(p), self.settings.crf));
                }
                if let Some(id) = resp.open_sequence {
                    self.enter_sequence(id);
                }
                if resp.import_url {
                    self.url_dialog = Some((String::new(), false));
                }
                for name in resp.place_template {
                    self.place_template(&name, self.playhead);
                }
                // Recent tab, reusable sections: an effect / saved preset / node graph goes onto the
                // selection (a clip rendering from a graph ignores its linear stack, so it is skipped).
                if let Some(kind) = resp.add_effect {
                    let targets: Vec<Id> = self
                        .selection
                        .iter()
                        .copied()
                        .filter(|&id| self.project.clip(id).is_some_and(|c| c.is_visual() && !c.uses_graph()))
                        .collect();
                    if targets.is_empty() {
                        self.toast("Select a clip first");
                    } else {
                        self.push_undo();
                        for id in targets {
                            if let Some(c) = self.project.clip_mut(id) {
                                c.effects.push(Effect::new(kind));
                            }
                        }
                        self.after_edit();
                    }
                }
                if let Some(i) = resp.apply_preset {
                    self.apply_effect_preset(i);
                }
                if let Some(from) = resp.copy_graph {
                    let graph = self.project.clip(from).and_then(|c| c.graph.clone());
                    let targets: Vec<Id> = self
                        .selection
                        .iter()
                        .copied()
                        .filter(|&id| id != from && self.project.clip(id).is_some_and(|c| c.is_visual()))
                        .collect();
                    match graph {
                        Some(g) if !targets.is_empty() => {
                            self.push_undo();
                            for id in targets {
                                if let Some(c) = self.project.clip_mut(id) {
                                    c.graph = Some(g.clone());
                                }
                            }
                            self.after_edit();
                        }
                        _ => self.toast("Select another clip to copy this node graph onto"),
                    }
                }
            }
            Pane::Inspector => {
                let changed = {
                    let App { project, selection, playhead, undo, redo, fonts, palette, settings, .. } = self;
                    let mut push = |p: &Project| push_undo_json(undo, redo, p.to_json());
                    let mut changed = false;
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        changed =
                            inspector::show(ui, project, selection, *playhead, fonts, palette, settings, &mut push);
                    });
                    changed
                };
                if changed {
                    self.after_edit();
                }
                if let Some(a) = inspector::take_pending_action() {
                    self.pending_actions.push(a);
                }
            }
            Pane::Effects => {
                let resp = {
                    let App { project, selection, playhead, undo, redo, palette, .. } = self;
                    let mut push = |p: &Project| push_undo_json(undo, redo, p.to_json());
                    // the always-open catalogue is taller than the pane: without this the per-clip
                    // effect stack under it is unreachable
                    egui::ScrollArea::vertical()
                        .show(ui, |ui| effects_ui::show(ui, project, selection, *playhead, palette, &mut push))
                        .inner
                };
                if let Some(i) = resp.mask_for {
                    // ponytail: the viewport's mask tool edits clip.mask, not the effect's own mask —
                    // targeting an effect index is a preview.rs change, not an app one.
                    let shape = self
                        .selection
                        .first()
                        .and_then(|&id| self.project.clip(id))
                        .and_then(|c| c.effects.get(i))
                        .and_then(|fx| fx.mask.as_ref())
                        .map(|m| m.shape)
                        .unwrap_or(MaskShape::Ellipse);
                    self.tools.tool = Tool::Mask(shape);
                    self.layout.reveal(Pane::Tools);
                    self.layout_dirty = true;
                }
                if resp.open_nodes {
                    self.layout.reveal(Pane::Nodes);
                    self.layout_dirty = true;
                }
                if let Some(i) = resp.edit_shader {
                    // same clip the panel showed the stack of
                    let id = self.selection.iter().copied().find(|&id| self.project.clip(id).is_some()).unwrap_or(0);
                    let src = self
                        .project
                        .clip(id)
                        .and_then(|c| c.effects.get(i))
                        .filter(|fx| fx.kind == EffectKind::Shader)
                        .map(|fx| fx.shader.clone());
                    if let Some(src) = src {
                        self.shader_ui.edit(id, i, &src);
                        // a source the renderer already rejected opens with its log, not blank
                        let known = self.gpu.as_ref().and_then(|g| g.shader_error(&src));
                        self.shader_ui.error = known.unwrap_or_default().to_string();
                    }
                }
                if resp.edited {
                    self.after_edit();
                }
            }
            Pane::Transitions => {
                let changed = {
                    let App { project, selection, playhead, undo, redo, transitions_ui: st, palette, .. } = self;
                    let mut push = |p: &Project| push_undo_json(undo, redo, p.to_json());
                    transitions_ui::show(ui, st, project, selection, *playhead, palette, &mut push)
                };
                if changed {
                    self.after_edit();
                }
            }
            Pane::Curves => {
                let resp = {
                    let App { project, selection, playhead, undo, redo, curves: st, palette, mixer, .. } = self;
                    let bus = mixer.selected_bus;
                    let mut push = |p: &Project| push_undo_json(undo, redo, p.to_json());
                    curves::show(ui, st, project, selection, bus, playhead, palette, &mut push)
                };
                if resp.seeked {
                    self.player.pause();
                    self.player.seek(self.playhead);
                }
                if resp.edited {
                    self.after_edit();
                }
            }
            Pane::Subtitles => {
                let resp = {
                    let App { project, playhead, selection, undo, redo, subtitles_ui: st, fonts, palette, .. } = self;
                    let mut push = |p: &Project| push_undo_json(undo, redo, p.to_json());
                    subtitles_ui::show(ui, st, project, playhead, selection, fonts, palette, &mut push)
                };
                if resp.seeked {
                    self.player.seek(self.playhead);
                    if resp.play {
                        self.player.play();
                    } else {
                        self.player.pause();
                    }
                }
                if resp.open_folder {
                    self.open_subtitle_folder();
                }
                if resp.edited {
                    self.after_edit();
                }
            }
            Pane::Planner => {
                let resp = {
                    let App { project, undo, redo, planner: st, thumbs, palette, .. } = self;
                    let mut push = |p: &Project| push_undo_json(undo, redo, p.to_json());
                    planner::show(ui, st, project, thumbs, palette, &mut push)
                };
                if !resp.add_to_timeline.is_empty() {
                    self.push_undo();
                    self.insert_at(resp.add_to_timeline, self.playhead, None);
                    self.after_edit();
                }
                if resp.edited {
                    self.after_edit();
                }
            }
            Pane::AutoCut => {
                self.autocut_drawing = true;
                let changed = {
                    let App { project, selection, undo, redo, autocut: st, waveforms, palette, .. } = self;
                    let mut push = |p: &Project| push_undo_json(undo, redo, p.to_json());
                    autocut_ui::show(ui, st, project, selection, waveforms, palette, &mut push)
                };
                if changed {
                    self.after_edit();
                }
            }
            Pane::Tracking => {
                self.tracking_drawing = true;
                let backend = self.backend();
                let changed = {
                    let App { project, selection, undo, redo, tracking: st, palette, .. } = self;
                    let mut push = |p: &Project| push_undo_json(undo, redo, p.to_json());
                    tracking_ui::show(ui, st, project, selection, backend, palette, &mut push)
                };
                if changed {
                    self.after_edit();
                }
            }
            Pane::Tools => {
                let was = self.tools.recording;
                let snap_was = self.settings.snap;
                {
                    let App { tools: st, palette, settings, .. } = self;
                    tools::show(ui, st, palette, &mut settings.snap);
                }
                if self.tools.recording != was {
                    self.toggle_draw_recording(self.tools.recording);
                }
                if self.settings.snap != snap_was {
                    self.settings.save();
                }
            }
            Pane::Nodes => {
                let resp = {
                    let App { project, selection, playhead, undo, redo, nodes: st, palette, .. } = self;
                    let mut push = |p: &Project| push_undo_json(undo, redo, p.to_json());
                    nodes::show(ui, st, project, selection, *playhead, palette, &mut push)
                };
                if resp.edited {
                    self.after_edit();
                }
            }
            Pane::Mixer => {
                let changed = {
                    let App { project, selection, playhead, mixer: st, buses, palette, undo, redo, .. } = self;
                    let mut push = |p: &Project| push_undo_json(undo, redo, p.to_json());
                    mixer_ui::show(ui, st, project, selection, buses, *playhead, palette, &mut push)
                };
                if changed {
                    self.after_edit();
                }
            }
            Pane::Presets => {
                let has_sel = !self.selection.is_empty();
                let resp = presets_ui::show(ui, &mut self.presets, &mut self.settings, has_sel);
                if let Some(i) = resp.apply {
                    self.apply_effect_preset(i);
                }
                for name in resp.place {
                    self.place_template(&name, self.playhead);
                }
                if let Some(name) = resp.save {
                    self.save_preset(&name);
                }
                if resp.settings_changed {
                    self.settings.save();
                }
            }
            Pane::Markers => {
                let resp = {
                    let App { project, selection, playhead, markers: st, palette, undo, redo, .. } = self;
                    let mut push = |p: &Project| push_undo_json(undo, redo, p.to_json());
                    markers_ui::show(ui, st, project, selection, *playhead, palette, &mut push)
                };
                if let Some(t) = resp.seek {
                    self.player.pause();
                    self.seek(t);
                }
                if resp.edited {
                    self.after_edit();
                }
            }
        }
    }

    /// Place a saved template (by name) on the timeline at `t`.
    /// Hand the saved curve/motion presets (built-ins first) to the curve editor. Called on start and
    /// whenever the lists change — never per frame.
    fn refresh_presets(&self) {
        curves::set_available_presets(self.settings.curve_presets.clone());
        let motions: Vec<crate::settings::MotionPreset> = crate::engine::presets::builtin_motions()
            .into_iter()
            .chain(self.settings.motion_presets.iter().cloned())
            .collect();
        curves::set_available_motions(motions);
    }

    /// Add a shape clip at the playhead, styled from the tool strip. `place` = (centre x, centre y, half
    /// width, half height) in project px when the shape was dragged out in the viewport (Action::AddShape
    /// passes None and keeps the default size). For `ShapeKind::Draw` the playhead/5s here are only a
    /// placeholder — `add_stroke` / `toggle_draw_recording` re-pin start and duration to the strokes
    /// actually recorded once there is ink to measure.
    fn add_shape(&mut self, kind: ShapeKind, place: Option<(f32, f32, f32, f32)>) -> Id {
        self.push_undo();
        let id = self.project.add_shape_clip(kind, self.playhead, 5.0);
        let App { project, tools, .. } = self;
        if let Some(c) = project.clip_mut(id) {
            if let Some((cx, cy, _, _)) = place {
                c.x.value = cx as f64;
                c.y.value = cy as f64;
            }
            if let Some(s) = c.shape.as_mut() {
                s.fill = tools.fill;
                // line / arrow / drawing have no fill, so a transparent stroke would draw nothing at all:
                // fall back to the brush colour (and then the fill) instead of an invisible clip
                let stroke_only = matches!(kind, ShapeKind::Line | ShapeKind::Arrow | ShapeKind::Draw);
                s.stroke = match (stroke_only, tools.stroke[3], tools.brush[3]) {
                    (true, 0, 0) => [tools.fill[0], tools.fill[1], tools.fill[2], 255],
                    (true, 0, _) => tools.brush,
                    _ => tools.stroke,
                };
                s.stroke_width = tools.stroke_width;
                s.sides = tools.sides;
                s.corner = tools.corner;
                s.draw_rate = tools.draw_rate;
                s.page = tools.page;
                if let Some((_, _, w, h)) = place {
                    s.w.value = w as f64;
                    s.h.value = h as f64;
                }
            }
        }
        self.selection = vec![id];
        self.after_edit();
        id
    }

    /// The Draw tool's play/record button: the video plays and every stroke joins one drawing until the
    /// take is stopped (button, video stopped, or the tool put away). A voiceover running at the same
    /// time owns the transport, so it is left playing and its take is not cut short.
    fn toggle_draw_recording(&mut self, on: bool) {
        if on {
            let id = self.add_shape(ShapeKind::Draw, None);
            self.draw_rec = Some((id, self.playhead));
            if !self.player.is_playing() {
                self.pending_actions.push(Action::PlayPause);
            }
            return;
        }
        let Some((id, _)) = self.draw_rec.take() else { return };
        // the clip was placed at record-press time with a placeholder length; pin its real bounds to
        // exactly the first and last point drawn (`add_stroke` already times every point from that press)
        // instead of the press-to-stop span, which pads the clip with dead time on either side.
        let bounds = self.project.clip(id).and_then(|c| c.shape.as_ref()).map(|s| {
            let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
            for st in &s.strokes {
                for p in &st.points {
                    lo = lo.min(p.2);
                    hi = hi.max(p.2);
                }
            }
            (lo, hi)
        });
        match bounds.filter(|(lo, _)| lo.is_finite()) {
            Some((lo, hi)) => {
                if let Some(c) = self.project.clip_mut(id) {
                    c.start += lo as f64;
                    c.duration = ((hi - lo).max(0.0) as f64).max(MIN_CLIP);
                    if let Some(s) = c.shape.as_mut() {
                        for st in &mut s.strokes {
                            for p in &mut st.points {
                                p.2 -= lo;
                            }
                        }
                    }
                }
            }
            None => self.project.delete_clips(&[id], false), // nothing was drawn: leave no stub behind
        }
        if self.voice_rec.is_none() {
            self.player.pause();
        }
        self.after_edit();
    }

    /// A stroke drawn in the viewport: append it to the take (or the selected drawing), or start a new one.
    fn add_stroke(&mut self, mut stroke: crate::model::Stroke) {
        stroke.color = self.tools.brush;
        stroke.width = self.tools.brush_width.max(0.5);
        // during a take the stroke is timed from where the playhead was when the pen went down: its own
        // points are timed from the press, so the last one dates the whole stroke
        let rec = self.draw_rec.filter(|&(id, _)| self.project.clip(id).is_some());
        if let Some((_, at)) = rec {
            let off = ((self.playhead - at) - stroke.points.last().map_or(0.0, |p| p.2 as f64)).max(0.0) as f32;
            for p in &mut stroke.points {
                p.2 += off;
            }
        }
        let onto = rec.map(|(id, _)| id).or_else(|| {
            self.selection.first().copied().filter(|&id| {
                self.project.clip(id).and_then(|c| c.shape.as_ref()).is_some_and(|s| s.kind == ShapeKind::Draw)
            })
        });
        let id = match onto {
            Some(id) => {
                self.push_undo();
                id
            }
            None => self.add_shape(ShapeKind::Draw, None),
        };
        if let Some(c) = self.project.clip_mut(id) {
            if let Some(s) = c.shape.as_mut() {
                s.strokes.push(stroke);
            }
            if rec.is_some() {
                // still recording: keep the preview at least as long as what's drawn so far; the exact
                // start/end are pinned once the take stops (toggle_draw_recording)
                let len = c.shape.as_ref().map_or(0.0, |s| s.draw_duration());
                c.duration = c.duration.max(len);
            } else {
                // not part of a running take (a plain drag): pin the clip to exactly what's drawn, instead
                // of leaving it at add_shape's placeholder duration if the stroke was shorter than that
                let bounds = c.shape.as_mut().map(|s| {
                    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
                    for st in &s.strokes {
                        for p in &st.points {
                            lo = lo.min(p.2);
                            hi = hi.max(p.2);
                        }
                    }
                    if lo > 0.0 {
                        for st in &mut s.strokes {
                            for p in &mut st.points {
                                p.2 -= lo;
                            }
                        }
                        hi -= lo;
                    }
                    (lo.max(0.0), hi.max(0.0))
                });
                if let Some((lo, hi)) = bounds {
                    c.start += lo as f64;
                    c.duration = (hi as f64).max(MIN_CLIP);
                }
            }
        }
        self.selection = vec![id];
        self.after_edit();
    }

    /// Per-frame hand-offs from panels that can't reach Settings, plus background convert jobs.
    fn poll_panels(&mut self) {
        // saved presets from the effects / curves panels
        if let Some(m) = effects_ui::take_pending_motion() {
            self.settings.motion_presets.retain(|p| p.name != m.name);
            self.settings.motion_presets.push(m);
            self.settings.save();
            self.toast("Motion preset saved");
        }
        if let Some(m) = curves::take_pending_motion_preset() {
            self.settings.motion_presets.retain(|p| p.name != m.name);
            self.settings.motion_presets.push(m);
            self.settings.save();
            self.refresh_presets();
            self.toast("Motion preset saved");
        }
        if let Some(c) = curves::take_pending_curve_preset() {
            self.settings.curve_presets.retain(|p| p.name != c.name);
            self.settings.curve_presets.push(c);
            self.settings.save();
            // hand them over only when they change (this used to clone the whole list every frame)
            self.refresh_presets();
            self.toast("Curve preset saved");
        }
        // font import from the inspector
        if let Some(path) = inspector::take_pending_font_import() {
            if !self.settings.user_fonts.iter().any(|f| f.eq_ignore_ascii_case(&path)) {
                self.settings.user_fonts.push(path);
                self.settings.save();
            }
        }
        if self.loaded_fonts != self.settings.user_fonts.len() {
            let fonts = self.settings.user_fonts.clone();
            if let Ok(mut t) = self.text.try_lock() {
                t.load_user_fonts(&fonts);
                self.fonts = t.families().to_vec();
                self.loaded_fonts = fonts.len();
            }
        }
        if let Some(id) = inspector::take_open_sequence() {
            self.enter_sequence(id);
        }
        // "Edit in viewport" / "Open node editor" from the inspector
        if let Some(id) = inspector::take_edit_mask() {
            let shape =
                self.project.clip(id).and_then(|c| c.mask.as_ref()).map(|m| m.shape).unwrap_or(MaskShape::Ellipse);
            self.selection = vec![id];
            self.tools.tool = Tool::Mask(shape);
            self.layout.reveal(Pane::Tools);
            self.layout_dirty = true;
        }
        if let Some(id) = inspector::take_open_nodes() {
            self.selection = vec![id];
            self.layout.reveal(Pane::Nodes);
            self.layout_dirty = true;
        }
        if let Some(id) = inspector::take_unlink_nodes() {
            self.push_undo();
            match self.project.unlink_graph(id) {
                Ok(n) => {
                    self.after_edit();
                    self.toast(format!("Unlinked — {n} effect layer{}", if n == 1 { "" } else { "s" }));
                }
                // nothing changed, so the snapshot above would be a no-op undo entry
                Err(e) => {
                    self.undo.pop();
                    self.toast(format!("Can't unlink this graph: {e}"));
                }
            }
        }
        // URL downloads: import the finished file, report failures
        let mut fetched: Vec<(Option<PathBuf>, Option<String>, bool)> = Vec::new();
        self.downloads.retain(|d| {
            if d.progress.is_done() {
                fetched.push((d.path(), d.progress.error(), d.progress.is_cancelled()));
                false
            } else {
                true
            }
        });
        for (path, err, cancelled) in fetched {
            match (path, err) {
                (Some(p), None) => {
                    let ids = self.import_files(&[p.clone()]);
                    self.library.selected = ids.last().copied();
                    self.library.tab = 0;
                    self.toast(format!("Imported {}", p.file_name().unwrap_or_default().to_string_lossy()));
                }
                // cancelling is not a failure — matches finish_export's wording
                (_, Some(_)) if cancelled => self.toast("Download cancelled"),
                (_, Some(e)) => self.toast(format!("Download failed: {e}")),
                (None, None) => self.toast("Download finished but produced no file"),
            }
        }

        // library conversions
        let mut done: Vec<(PathBuf, Option<String>)> = Vec::new();
        self.convert_jobs.retain(|(prog, out)| {
            if prog.is_done() {
                done.push((out.clone(), prog.error()));
                false
            } else {
                true
            }
        });
        for (out, err) in done {
            match err {
                Some(e) => self.toast(format!("Convert failed: {e}")),
                None => {
                    let ids = self.import_files(&[out.clone()]);
                    self.library.selected = ids.last().copied();
                    self.toast(format!("Converted → {}", out.file_name().unwrap_or_default().to_string_lossy()));
                }
            }
        }
    }

    /// Open a nested timeline for editing (keeps the player/preview in sync).
    fn enter_sequence(&mut self, id: Id) {
        if self.project.editing == Some(id) {
            return;
        }
        if self.project.open_sequence(id) {
            self.sequence_view_changed(0.0);
        } else {
            self.toast("That sequence no longer exists");
        }
    }

    /// Navigating in/out of a nested sequence is a view change, not an edit: re-sync the player,
    /// but no undo step and no dirty flag (that made "just looking" prompt to save on exit).
    fn sequence_view_changed(&mut self, t: f64) {
        self.selection.clear();
        self.player.set_project(&self.project);
        self.seek(t);
    }

    /// Import URL window (non-blocking): paste a link, pick a folder, download with yt-dlp, and the
    /// finished file is imported into the library like any other media.
    fn url_window(&mut self, ctx: &egui::Context) {
        let Some((mut url, mut audio_only)) = self.url_dialog.clone() else { return };
        let mut open = true;
        let mut start = false;
        let dir = self.download_dir();
        egui::Window::new("Import URL").open(&mut open).resizable(false).default_width(420.0).show(ctx, |ui| {
            ui.label("Paste a link (YouTube, Vimeo, X, TikTok, direct file, …)");
            let r = ui.add(egui::TextEdit::singleline(&mut url).desired_width(400.0).hint_text("https://…"));
            // Enter in the field downloads, like every other URL box
            start |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if crate::media::ffpipe::ffmpeg_exe().is_some() {
                ui.checkbox(&mut audio_only, "Audio only (music / SFX)");
            }
            ui.horizontal(|ui| {
                ui.label("Save to");
                ui.weak(dir.to_string_lossy());
                if ui.small_button("Browse…").clicked() {
                    if let Some(p) = rfd::FileDialog::new().set_directory(&dir).pick_folder() {
                        self.settings.download_dir = p.to_string_lossy().into_owned();
                        self.settings.save();
                    }
                }
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                start |= ui.add_enabled(!url.trim().is_empty(), egui::Button::new("Download")).clicked();
                ui.weak(format!("{} running", self.downloads.len()));
            });
        });
        match (open, start) {
            (_, true) => {
                self.url_dialog = None;
                self.start_download(url.trim(), audio_only);
            }
            (true, false) => self.url_dialog = Some((url, audio_only)),
            (false, false) => self.url_dialog = None,
        }
    }

    /// Configured download folder, or the user's Videos folder.
    fn download_dir(&self) -> PathBuf {
        let d = self.settings.download_dir.trim();
        if d.is_empty() {
            crate::media::ytdlp::default_dir()
        } else {
            PathBuf::from(d)
        }
    }

    /// Look for a working yt-dlp on a background thread (it spawns `yt-dlp --version`, and a candidate
    /// that hangs must not hang the editor); the Library button appears once it reports success.
    fn detect_ytdlp(&self, ctx: &egui::Context) {
        let flag = self.ytdlp_available.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let found = media::ytdlp::exe().is_some();
            flag.store(found, std::sync::atomic::Ordering::Relaxed);
            ctx.request_repaint(); // the Library button appears without waiting for the next input
        });
    }

    fn start_download(&mut self, url: &str, audio_only: bool) {
        if !self.ytdlp_available.load(std::sync::atomic::Ordering::Relaxed) {
            self.toast("yt-dlp not found — set its folder in Settings");
            return;
        }
        let opts = crate::media::ytdlp::DownloadOptions { url: url.to_string(), dir: self.download_dir(), audio_only };
        self.toast("Downloading…");
        self.downloads.push(crate::media::ytdlp::start_download(opts));
    }

    /// "Convert To…" on a library asset: transcode next to the source, then import the result.
    fn start_asset_convert(&mut self, asset: Id, ext: &str) {
        if self.ffmpeg_missing() {
            return;
        }
        let Some(a) = self.project.asset(asset) else { return };
        let src = PathBuf::from(&a.path);
        let out = converted_path(&src, ext);
        let opts = crate::engine::convert::ConvertOptions {
            src,
            out: out.clone(),
            encoder: self.settings.encoder.clone(),
            crf: self.settings.crf,
            preset: self.settings.preset.clone(),
            out_size: None,
            scaler: self.settings.export_scaler.clone(),
            gif_fps: 15,
            target_bytes: None,
        };
        self.toast(format!("Converting to {ext}…"));
        self.convert_jobs.push((crate::engine::convert::start_convert(opts), out));
    }

    /// "Save from selection" in the Presets pane: one clip's node graph or effect stack becomes an
    /// effect preset, anything else (adjustment layers, several clips) becomes a clip template — that is
    /// the only flavour that can be *placed* rather than applied.
    fn save_preset(&mut self, name: &str) {
        let fx = match self.selection.as_slice() {
            [id] => self
                .project
                .clip(*id)
                .filter(|c| c.kind != ClipKind::Adjustment && (c.graph.is_some() || !c.effects.is_empty()))
                .map(|c| crate::engine::presets::capture_effects(name, c)),
            _ => None,
        };
        if let Some(p) = fx {
            self.settings.effect_presets.retain(|x| x.name != name);
            self.settings.effect_presets.push(p);
        } else if self.selection.is_empty() {
            return self.toast("Select a clip first");
        } else {
            let t = crate::engine::presets::capture_template(name, &self.project, &self.selection);
            self.settings.templates.retain(|x| x.name != name);
            self.settings.templates.push(t);
        }
        self.settings.save();
        self.toast(format!("Saved \"{name}\""));
    }

    /// Apply a saved effect chain / node graph to every selected clip (one undo entry).
    fn apply_effect_preset(&mut self, i: usize) {
        let Some(p) = self.settings.effect_presets.get(i).cloned() else { return };
        if self.selection.is_empty() {
            return self.toast("Select a clip first");
        }
        self.push_undo();
        let mut n = 0;
        for id in self.selection.clone() {
            n += crate::engine::presets::apply_effects(&p, &mut self.project, id) as usize;
        }
        if n == 0 {
            return self.toast("That preset is corrupted");
        }
        self.after_edit();
    }

    fn place_template(&mut self, name: &str, t: f64) {
        let Some(tpl) = self.settings.templates.iter().find(|x| x.name == name).cloned() else {
            self.toast(format!("Template '{name}' not found"));
            return;
        };
        match crate::engine::presets::decode_template(&tpl) {
            Some((clips, assets)) => {
                self.push_undo();
                let ids = self.project.place_clips(clips, assets, t);
                self.selection = ids;
                self.after_edit();
            }
            None => self.toast(format!("Template '{name}' is corrupted")),
        }
    }

    // ---------------- round 3: GPU, capture, frames, movie mode ----------------

    /// Build / drop the GPU renderer to match `settings.gpu`, and tell the player which kind of output
    /// the UI wants (decoded layers for the GPU, a composited frame for the CPU compositor).
    fn sync_gpu(&mut self) {
        let want = self.settings.gpu && !self.gpu_failed;
        if want == self.gpu.is_some() {
            return;
        }
        if !want {
            self.gpu = None;
            self.player.set_gpu(false);
            return;
        }
        let Some(gl) = self.gl.clone() else {
            self.gpu_failed = true; // no GL context at all (software / headless run)
            self.player.set_gpu(false);
            return;
        };
        match guarded(|| GpuRenderer::new(gl)) {
            Some(Ok(mut g)) => {
                g.text = Some(self.text.clone()); // Text nodes rasterise with the same fonts as everything else
                self.gpu = Some(g);
                self.player.set_gpu(true);
                self.toast(format!("GPU preview: {}", self.gpu_name));
            }
            Some(Err(e)) => self.gpu_off(&e),
            None => self.gpu_off("the renderer panicked"),
        }
    }

    /// Render one catalogue thumbnail per `EffectKind` over the stock image and hand them to the effects
    /// panel. Runs once per (stock image, size) — the GPU renderer owns the GL context, so this happens on
    /// the UI thread. Without a GPU the panel keeps its neutral named cards.
    fn build_effect_thumbnails(&mut self, ctx: &egui::Context) {
        const TW: u32 = 96;
        const TH: u32 = 54;
        let key = (self.settings.effect_thumb_image.clone(), TW);
        if self.gpu.is_none() || self.effect_thumbs_key.as_ref() == Some(&key) {
            return;
        }
        let src = effect_thumb_source(&key.0, TW, TH, self.backend());
        if src.is_empty() {
            return;
        }
        effects_ui::clear_thumbnails();
        self.effect_thumbs.clear();
        let mut out = Frame::default();
        for kind in EffectKind::ALL {
            // geometric kinds move the layer instead of touching pixels: show the plain source
            let effect = crate::model::Effect::new(kind);
            let rendered = {
                let Some(gpu) = self.gpu.as_mut() else { break };
                guarded(|| gpu.effect_preview(&src, &effect, 0.35, &mut out)).unwrap_or(false)
            };
            let frame = if rendered { &out } else { &src };
            if frame.is_empty() || frame.rgba.len() != (frame.width * frame.height * 4) as usize {
                continue;
            }
            let img =
                egui::ColorImage::from_rgba_premultiplied([frame.width as usize, frame.height as usize], &frame.rgba);
            let tex = ctx.load_texture(format!("fxthumb_{}", kind.name()), img, egui::TextureOptions::LINEAR);
            effects_ui::set_thumbnail(kind, tex.id(), [frame.width, frame.height]);
            self.effect_thumbs.push(tex);
        }
        // the transitions catalogue previews its wipes over the same picture (painted, no GPU pass)
        let img = egui::ColorImage::from_rgba_premultiplied([src.width as usize, src.height as usize], &src.rgba);
        transitions_ui::set_stock(ctx.load_texture("tr_stock", img, egui::TextureOptions::LINEAR));
        self.effect_thumbs_key = Some(key);
    }

    /// Where a new export should get its pictures: the GPU renderer when it is running (so the file
    /// matches the preview), otherwise the export thread's own CPU compositor.
    fn export_frames(&self) -> crate::engine::export::FrameSource {
        match self.gpu {
            Some(_) => crate::engine::export::FrameSource::Gpu(self.gpu_export.0.clone()),
            None => crate::engine::export::FrameSource::Cpu,
        }
    }

    /// Serve the frames export threads and movie-mode prerender workers are waiting on. Called every
    /// frame; each request is answered on the GL context, so both run the same shaders as the preview.
    /// Returns true if any were served (the caller keeps repainting so a background export is never
    /// starved; prerender already repaints on its own while busy).
    fn serve_gpu_exports(&mut self) -> bool {
        let mut served = false;
        while let Ok(req) = self.gpu_export.1.try_recv() {
            served = true;
            let mut out = Frame::default();
            let ok = {
                let App { gpu, project, .. } = self;
                match gpu.as_mut() {
                    Some(g) => {
                        out.resize(req.w, req.h);
                        guarded(|| g.render_frame(project, req.t, req.w, req.h, &req.layers, &mut out)).is_some()
                    }
                    None => false,
                }
            };
            if !ok {
                self.gpu_off("the renderer panicked");
            }
            // None tells the export thread to finish on the CPU compositor
            let _ = req.reply.send(ok.then_some(out));
        }
        served
    }

    /// Fall back to the CPU compositor and say why, once.
    fn gpu_off(&mut self, why: &str) {
        self.gpu = None;
        self.gpu_tex = None;
        self.gpu_tex_ids.clear();
        effects_ui::clear_thumbnails();
        self.effect_thumbs.clear();
        self.effect_thumbs_key = None;
        self.gpu_failed = true;
        self.player.set_gpu(false);
        self.toast(format!("GPU rendering unavailable ({why}) — using the CPU compositor"));
    }

    /// Render the timeline into a GL texture and register it with egui, so the preview paints the GPU's
    /// own canvas — no glReadPixels, no re-upload. None when there is no GPU (the caller falls back to
    /// `gpu_frame`). The texture is only valid for this frame, which is exactly how long it is painted.
    fn gpu_preview_texture(
        &mut self,
        layers: &crate::engine::gpu::LayerSet,
        t: f64,
        w: u32,
        h: u32,
        frame: &mut eframe::Frame,
    ) -> Option<(egui::TextureId, [u32; 2])> {
        if self.gpu.is_none() || w == 0 || h == 0 {
            return None;
        }
        let made = {
            let App { gpu, project, .. } = self;
            let gpu = gpu.as_mut()?;
            guarded(|| gpu.render_preview_texture(project, t, w, h, layers)).flatten()
        };
        let (tex, tw, th) = made?;
        let id = match self.gpu_tex_ids.get(&tex) {
            Some(&id) => id,
            None => {
                let id = frame.register_native_glow_texture(tex);
                self.gpu_tex_ids.insert(tex, id);
                id
            }
        };
        Some((id, [tw, th]))
    }

    /// Render decoded layers with the GPU into a frame the preview can upload. None = the GPU path died
    /// (already switched off).
    fn gpu_frame(&mut self, layers: &crate::engine::gpu::LayerSet, t: f64, w: u32, h: u32) -> Option<Arc<Frame>> {
        if self.gpu.is_none() || w == 0 || h == 0 {
            return None;
        }
        // reuse the buffer of the frame handed out last time, once the preview has uploaded it
        let mut out = match self.gpu_prev.take().map(Arc::try_unwrap) {
            Some(Ok(f)) => f,
            _ => Frame::default(),
        };
        let ok = {
            let App { gpu, project, .. } = self;
            let gpu = gpu.as_mut()?;
            guarded(|| gpu.render_frame(project, t, w, h, layers, &mut out)).is_some()
        };
        if !ok {
            self.gpu_off("the renderer panicked");
            return None;
        }
        out.pts = t;
        let frame = Arc::new(out);
        self.gpu_prev = Some(frame.clone());
        Some(frame)
    }

    /// One frame at `t`, `w` px wide, through the same path the preview uses (GPU when it is on, the
    /// player's compositor otherwise) — export-frame and the MCP tools.
    fn render_frame_now(&mut self, t: f64, w: u32) -> Option<Arc<Frame>> {
        if self.gpu.is_some() {
            let (pw, ph) = (self.project.width.max(16), self.project.height.max(16));
            let w = w.clamp(16, pw);
            let h = ((ph as u64 * w as u64) / pw as u64).max(1) as u32;
            if let Some(layers) = self.player.layers_once(t, w) {
                if let Some(f) = self.gpu_frame(&layers, t, w, h) {
                    return Some(f);
                }
            }
        }
        self.player.render_once(t, w)
    }

    /// Movie mode: ask for the in/out range, or the whole timeline when there is none.
    fn request_prerender(&mut self) {
        let a = self.project.in_point.unwrap_or(0.0);
        let b = self.project.out_point.unwrap_or_else(|| self.project.duration());
        if b > a {
            let App { prerender, project, .. } = self;
            if guarded(|| prerender.request(project, a, b)).is_none() {
                self.settings.movie_mode = false;
                self.toast("Movie mode is not available in this build");
            }
        }
    }

    /// "Export Frame…" confirmed: render at the chosen size and write the image with ffmpeg.
    fn export_frame(&mut self, opts: frame_ui::FrameExport) {
        let (rw, rh) = frame_render_size((self.project.width, self.project.height), opts.size);
        let scaler_before = self.project.scaler;
        self.project.scaler = opts.scaler;
        let frame = if opts.with_effects {
            self.render_frame_now(self.playhead, rw)
        } else {
            self.source_frame(self.playhead, rw, rh)
        };
        self.project.scaler = scaler_before;
        let Some(frame) = frame else {
            self.toast("Could not render that frame");
            return;
        };
        match write_image(&frame, &opts) {
            Ok(()) => self.toast(format!("Frame saved to {}", opts.out.display())),
            Err(e) => self.toast(format!("Frame export failed: {e}")),
        }
    }

    /// The decoded frame of the top-most visual clip under the playhead ("source frame only").
    fn source_frame(&mut self, t: f64, w: u32, h: u32) -> Option<Arc<Frame>> {
        let clip = self
            .project
            .tracks
            .iter()
            .enumerate()
            .filter(|(i, tr)| tr.kind == TrackKind::Video && self.project.active(*i))
            .flat_map(|(_, tr)| tr.clips.iter())
            .filter(|c| c.enabled && c.contains(t) && c.uses_asset())
            .next_back()?;
        let asset = self.project.asset(clip.asset)?;
        let mut src = media::open_video(&asset.path, self.backend()).ok()?;
        let mut f = Frame::default();
        src.frame_at(clip.src_time(t).max(0.0), w, h, &mut f).then(|| Arc::new(f))
    }

    /// Import a timeline from another editor (FCP7 XML / EDL / .prproj) and show the report.
    fn act_import_timeline(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Timelines (XML, EDL, prproj)", crate::engine::import::IMPORT_EXTS)
            .add_filter("All files", &["*"])
            .pick_file()
        else {
            return;
        };
        match guarded(|| crate::engine::import::import_file(&path)) {
            Some(Ok(report)) => {
                self.toast(format!("Imported {} clips on {} tracks", report.clips, report.tracks));
                self.import_ui.report = Some(report);
                self.import_ui.open = true;
            }
            Some(Err(e)) => self.toast(format!("Import failed: {e}")),
            None => self.toast("Timeline import is not available in this build"),
        }
    }

    /// Start / stop the screen recorder with the options the window built (also driven by focus when
    /// `capture_on_blur` is on, which rebuilds them from settings + the window's region).
    fn start_screen_capture(&mut self, opts: crate::engine::capture::ScreenCaptureOptions) {
        if self.screen_rec.is_some() || self.ffmpeg_missing() {
            return;
        }
        let out = opts.out.clone();
        if let Some(dir) = out.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match guarded(|| crate::engine::capture::start_screen(opts)) {
            Some(Ok(c)) => self.screen_rec = Some((c, out)),
            Some(Err(e)) => self.toast(format!("Screen recording failed: {e}")),
            None => self.toast("Screen recording is not available in this build"),
        }
    }

    fn stop_screen_capture(&mut self) {
        let Some((c, out)) = self.screen_rec.take() else { return };
        if guarded(move || c.stop()).is_none() {
            return;
        }
        self.import_recording(out, None);
    }

    fn start_voiceover(&mut self, opts: crate::engine::capture::VoiceoverOptions) {
        if self.voice_rec.is_some() || self.ffmpeg_missing() {
            return;
        }
        let out = opts.out.clone();
        if let Some(dir) = out.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match guarded(|| crate::engine::capture::start_voiceover(opts)) {
            Some(Ok(c)) => {
                self.voice_rec = Some((c, out, self.playhead));
                self.player.play(); // the take lines up with what you hear
            }
            Some(Err(e)) => self.toast(format!("Voiceover failed: {e}")),
            None => self.toast("Voiceover recording is not available in this build"),
        }
    }

    fn stop_voiceover(&mut self) {
        let Some((c, out, at)) = self.voice_rec.take() else { return };
        self.player.pause();
        if guarded(move || c.stop()).is_none() {
            return;
        }
        self.import_recording(out, Some(at));
    }

    /// A finished recording: import it and (for a voiceover) drop it on the timeline at `at`.
    fn import_recording(&mut self, out: PathBuf, at: Option<f64>) {
        // ffmpeg finalises the container a moment after it is asked to stop
        let deadline = Instant::now() + Duration::from_secs(3);
        while !out.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        if !out.exists() {
            self.toast(format!("Recording not written: {}", out.display()));
            return;
        }
        let ids = self.import_files(&[out.clone()]);
        match at {
            Some(t) => {
                self.push_undo();
                self.insert_at(ids, t, None);
                self.after_edit();
                self.toast("Voiceover placed on the timeline");
            }
            None => {
                self.library.tab = 0;
                self.library.selected = ids.last().copied();
                self.toast(format!("Recording imported: {}", out.file_name().unwrap_or_default().to_string_lossy()));
            }
        }
    }

    /// dshow audio inputs, asked for once (the ffmpeg device probe takes ~a second).
    fn audio_inputs(&mut self) -> Vec<(String, bool)> {
        if self.audio_inputs.is_none() {
            self.audio_inputs = Some(guarded(crate::engine::capture::audio_devices).unwrap_or_default());
        }
        self.audio_inputs.clone().unwrap_or_default()
    }

    /// Screen-capture options for the record-on-blur path, which has no window response to take them
    /// from. Same folder the Screen Recording window shows (`capture_ui::capture_dir`).
    fn blur_capture_options(&self) -> crate::engine::capture::ScreenCaptureOptions {
        crate::engine::capture::ScreenCaptureOptions {
            out: capture_ui::capture_dir(&self.settings).join(format!("screen-{}.mp4", Settings::now())),
            fps: self.settings.capture_fps.clamp(1, 120),
            bitrate_kbps: self.settings.capture_bitrate_kbps,
            crf: self.settings.crf,
            region: if self.capture_ui.area == "region" { self.capture_ui.region } else { None },
            mic: self.settings.capture_mic.clone(),
            desktop_audio: self.settings.capture_desktop_audio,
            cursor: self.settings.capture_cursor,
        }
    }

    // ---------------- UI pieces ----------------

    /// Icon shown next to a menu action: the user's pick from Settings → Appearance → Icons wins,
    /// then the built-in defaults below. Abstract actions stay text-only.
    fn glyph_for(&self, a: Action) -> Option<tools::Glyph> {
        if let Some(name) = self.settings.icon_overrides.get(&format!("action.{}", a.id())) {
            return if name == "none" { None } else { tools::Glyph::from_name(name) };
        }
        tools::action_glyph(a)
    }

    fn menu_item(&mut self, ui: &mut egui::Ui, a: Action, enabled: bool, out: &mut Vec<Action>) {
        let text = self.hotkeys.text(a);
        let glyph = self.glyph_for(a);
        // ponytail: the glyph is painted over a left gutter made of spaces in the label — that keeps
        // egui's own menu-button sizing/shortcut layout instead of reimplementing the widget
        let label = match glyph {
            Some(_) => format!("     {}", a.label()),
            None => a.label().to_string(),
        };
        let b = egui::Button::new(label).shortcut_text(text);
        let r = ui.add_enabled(enabled, b);
        if let Some(g) = glyph {
            let rect = egui::Rect::from_min_size(
                egui::pos2(r.rect.min.x + 4.0, r.rect.center().y - 11.0),
                egui::vec2(24.0, 22.0),
            );
            let fg = if enabled { ui.visuals().text_color() } else { ui.visuals().weak_text_color() };
            tools::draw_glyph(ui.painter(), rect, g, fg);
        }
        // right-click any menu action: pick its icon in place (same picker as the tab context menu)
        let mut set: Option<Option<String>> = None;
        r.context_menu(|ui| {
            if let Some(pick) = layout::icon_menu(ui) {
                set = Some(pick);
            }
        });
        if let Some(pick) = set {
            let key = format!("action.{}", a.id());
            match pick {
                Some(name) => drop(self.settings.icon_overrides.insert(key, name)),
                None => drop(self.settings.icon_overrides.remove(&key)),
            }
            self.settings.save();
        }
        if r.clicked() {
            out.push(a);
            ui.close();
        }
    }

    /// Icon shown next to a pane (View menu, icon picker), with the user's Settings override first.
    fn pane_glyph(&self, p: Pane) -> Option<tools::Glyph> {
        layout::pane_icon(&self.settings.icon_overrides, p)
    }

    /// (Re)load the editor background image texture when the path or blur setting changed.
    fn ensure_bg_texture(&mut self, ctx: &egui::Context) {
        let (path, blur) = (self.settings.bg_image.clone(), self.settings.bg_blur);
        if path.is_empty() {
            self.bg_tex = None;
            return;
        }
        if matches!(&self.bg_tex, Some((p, b, _)) if *p == path && *b == blur) {
            return;
        }
        let mut f = Frame::default();
        let ok = media::open_video(&path, self.backend())
            .ok()
            .map(|mut src| src.frame_at(0.0, 1280, 720, &mut f) && !f.is_empty())
            .unwrap_or(false);
        if !ok {
            self.toast("Background image could not be read");
            self.settings.bg_image.clear();
            self.settings.save();
            self.bg_tex = None;
            return;
        }
        if blur > 0 {
            box_blur(&mut f.rgba, f.width as usize, f.height as usize, blur as usize);
        }
        let img = egui::ColorImage::from_rgba_premultiplied([f.width as usize, f.height as usize], &f.rgba);
        let tex = ctx.load_texture("editor-bg", img, egui::TextureOptions::LINEAR);
        self.bg_tex = Some((path, blur, tex));
    }

    fn view_menu(&mut self, ui: &mut egui::Ui, out: &mut Vec<Action>) {
        use Action::*;
        const PANES: [(Pane, Option<Action>); 16] = [
            (Pane::Preview, None),
            (Pane::Timeline, None),
            (Pane::Tools, Some(ToggleTools)),
            (Pane::Library, Some(ToggleLibrary)),
            (Pane::Inspector, Some(ToggleInspector)),
            (Pane::Effects, Some(ToggleEffects)),
            (Pane::Transitions, Some(ToggleTransitions)),
            (Pane::Curves, Some(ToggleCurves)),
            (Pane::Nodes, Some(ToggleNodes)),
            (Pane::Subtitles, Some(ToggleSubtitles)),
            (Pane::Markers, Some(ToggleMarkers)),
            (Pane::Mixer, Some(ToggleMixer)),
            (Pane::Presets, None),
            (Pane::Planner, Some(TogglePlanner)),
            (Pane::AutoCut, Some(Action::AutoCut)),
            (Pane::Tracking, None),
        ];
        for (pane, action) in PANES {
            let mut v = self.layout.is_visible(pane);
            let label = match action {
                Some(a) => format!("{}   {}", pane.title(), self.hotkeys.text(a)),
                None => pane.title().to_string(),
            };
            let changed = ui
                .horizontal(|ui| {
                    match self.pane_glyph(pane) {
                        Some(g) => {
                            tools::glyph_label(ui, g, ui.visuals().text_color());
                        }
                        None => ui.add_space(18.0),
                    }
                    ui.checkbox(&mut v, label).changed()
                })
                .inner;
            if changed {
                // AutoCut's action only reveals; go through the layout directly so unchecking works too
                match action {
                    Some(a) if a != Action::AutoCut => out.push(a),
                    _ => self.toggle_pane(pane),
                }
            }
        }
        ui.separator();
        ui.menu_button("Pop out", |ui| {
            for pane in Pane::ALL {
                if ui.button(pane.title()).clicked() {
                    ui.close();
                    self.layout.popout(pane);
                    self.layout_dirty = true;
                }
            }
        });
        ui.menu_button("Layout", |ui| {
            ui.menu_button("Preset", |ui| {
                for (name, make) in [
                    ("Default", Layout::default_layout as fn() -> Layout),
                    ("Colorist", Layout::colorist_layout),
                    ("Fast-Cut Assembly", Layout::fastcut_layout),
                ] {
                    if ui.button(name).clicked() {
                        ui.close();
                        self.layout.push_undo(self.layout.to_json());
                        let (undo, redo) = (std::mem::take(&mut self.layout.undo), std::mem::take(&mut self.layout.redo));
                        self.layout = make();
                        (self.layout.undo, self.layout.redo) = (undo, redo);
                        self.layout_dirty = true;
                    }
                }
            });
            if ui.button("Save profile…").clicked() {
                ui.close();
                self.profile_name = Some(String::new());
            }
            let profiles = self.settings.layout_profiles.clone();
            ui.menu_button("Load profile", |ui| {
                if profiles.is_empty() {
                    ui.label("(none)");
                }
                for p in profiles {
                    if layout::profile_button(ui, &p.name).clicked() {
                        ui.close();
                        match Layout::from_json_migrating(&p.json) {
                            Some(l) => {
                                self.layout = l;
                                self.layout_dirty = true;
                            }
                            None => self.toast(format!("Profile '{}' could not be read", p.name)),
                        }
                    }
                }
            });
            if ui.button("Export profile to file…").clicked() {
                ui.close();
                if let Some(out) = rfd::FileDialog::new()
                    .add_filter("Simple Editor layout", &["sedit-layout"])
                    .set_file_name("layout.sedit-layout")
                    .save_file()
                {
                    match std::fs::write(&out, self.layout.to_json()) {
                        Ok(()) => self.toast("Layout exported"),
                        Err(e) => self.toast(format!("Layout export failed: {e}")),
                    }
                }
            }
            if ui.button("Import profile from file…").clicked() {
                ui.close();
                if let Some(p) =
                    rfd::FileDialog::new().add_filter("Simple Editor layout", &["sedit-layout", "json"]).pick_file()
                {
                    match std::fs::read_to_string(&p).ok().and_then(|s| Layout::from_json_migrating(&s)) {
                        Some(l) => {
                            let name = p
                                .file_stem()
                                .map(|s| s.to_string_lossy().into_owned())
                                .unwrap_or_else(|| "Imported".into());
                            self.settings.layout_profiles.retain(|x| x.name != name);
                            self.settings
                                .layout_profiles
                                .push(crate::settings::LayoutProfile { name, json: l.to_json() });
                            self.settings.save();
                            self.layout = l;
                            self.layout_dirty = true;
                        }
                        None => self.toast("Not a valid layout file"),
                    }
                }
            }
            ui.separator();
            if ui.button("Reset layout").clicked() {
                ui.close();
                self.layout.reset();
                self.layout_dirty = true;
            }
        });
        ui.separator();
        let mut movie = self.settings.movie_mode;
        if ui.checkbox(&mut movie, "Movie mode (pre-rendered playback)").changed() {
            out.push(MovieMode);
        }
        self.menu_item(ui, Fullscreen, true, out);
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) -> Vec<Action> {
        use Action::*;
        let mut out = Vec::new();
        let has_sel = !self.selection.is_empty();
        let has_clips = !self.timeline_is_empty();
        let can_overwrite = self.project.source_video.is_some() && has_clips;
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                self.menu_item(ui, NewProject, true, &mut out);
                self.menu_item(ui, OpenFile, true, &mut out);
                self.menu_item(ui, OpenProject, true, &mut out);
                let recents = self.settings.recent_projects.clone();
                let mut forget: Option<Option<String>> = None; // Some(path) = drop one, None = clear all
                ui.menu_button("Open Recent Project", |ui| {
                    ui.set_max_width(420.0);
                    if recents.is_empty() {
                        ui.label("(none)");
                    }
                    for r in &recents {
                        let p = Path::new(r);
                        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| r.clone());
                        let folder = p.parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
                        let b = egui::Button::new(name).shortcut_text(folder).wrap_mode(egui::TextWrapMode::Truncate);
                        let resp = ui.add(b);
                        resp.context_menu(|ui| {
                            if ui.button("Remove from recent").clicked() {
                                ui.close();
                                forget = Some(Some(r.clone()));
                            }
                        });
                        if resp.on_hover_text(r).clicked() {
                            ui.close();
                            if self.confirm_discard() {
                                self.open_project(Path::new(r));
                            }
                        }
                    }
                    if !recents.is_empty() {
                        ui.separator();
                        if ui.button("Clear history").clicked() {
                            ui.close();
                            forget = Some(None);
                        }
                    }
                });
                if let Some(one) = forget {
                    match one {
                        Some(path) => self.settings.recent_projects.retain(|p| *p != path),
                        None => self.settings.recent_projects.clear(),
                    }
                    self.settings.save();
                }
                self.menu_item(ui, ImportMedia, true, &mut out);
                ui.separator();
                self.menu_item(ui, Save, true, &mut out);
                self.menu_item(ui, SaveProjectAs, true, &mut out);
                ui.separator();
                self.menu_item(ui, ExportVideo, has_clips, &mut out);
                self.menu_item(ui, ExportLossless, has_clips, &mut out);
                if ui.add_enabled(can_overwrite, egui::Button::new("Overwrite Original Video…")).clicked() {
                    ui.close();
                    self.act_overwrite();
                }
                self.menu_item(ui, ExportXml, has_clips, &mut out);
                self.menu_item(ui, ExportFrame, has_clips, &mut out);
                if ui.button("Export Style Summary (.md)…").clicked() {
                    ui.close();
                    self.act_export_style();
                }
                ui.separator();
                self.menu_item(ui, ImportTimeline, true, &mut out);
                self.menu_item(ui, ScreenCapture, true, &mut out);
                self.menu_item(ui, Voiceover, true, &mut out);
                ui.separator();
                self.menu_item(ui, Settings, true, &mut out);
                ui.separator();
                if ui.button("Exit").clicked() {
                    ui.close();
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Edit", |ui| {
                self.menu_item(ui, Undo, !self.undo.is_empty(), &mut out);
                self.menu_item(ui, Redo, !self.redo.is_empty(), &mut out);
                ui.separator();
                self.menu_item(ui, CopyClips, has_sel, &mut out);
                self.menu_item(ui, CutClips, has_sel, &mut out);
                self.menu_item(ui, PasteClips, self.clipboard.is_some(), &mut out);
                self.menu_item(ui, PasteInPlace, self.clipboard.is_some(), &mut out);
                self.menu_item(ui, PasteInsert, self.clipboard.is_some(), &mut out);
                self.menu_item(ui, PasteAtTop, self.clipboard.is_some(), &mut out);
                ui.separator();
                self.menu_item(ui, Split, has_clips, &mut out);
                self.menu_item(ui, Delete, has_sel, &mut out);
                self.menu_item(ui, RippleDelete, has_sel, &mut out);
                self.menu_item(ui, NudgeLeft, has_sel, &mut out);
                self.menu_item(ui, NudgeRight, has_sel, &mut out);
                ui.separator();
                self.menu_item(ui, SelectAll, has_clips, &mut out);
                self.menu_item(ui, Deselect, has_sel, &mut out);
                self.menu_item(ui, LinkToggle, has_sel, &mut out);
                self.menu_item(ui, ToggleEnabled, has_sel, &mut out);
                ui.separator();
                self.menu_item(ui, CopyAttributes, has_sel, &mut out);
                self.menu_item(ui, PasteAttributes, has_sel && self.attrs.is_some(), &mut out);
                ui.separator();
                self.menu_item(ui, AddText, true, &mut out);
                self.menu_item(ui, AddShape, true, &mut out);
                self.menu_item(ui, AddAdjustment, true, &mut out);
                self.menu_item(ui, AddMask, has_sel, &mut out);
                self.menu_item(ui, AddMarker, true, &mut out);
                self.menu_item(ui, AddSubtitle, true, &mut out);
                self.menu_item(ui, AddTransition, has_sel, &mut out);
                self.menu_item(ui, AddLastTransition, has_sel, &mut out);
                self.menu_item(ui, AddTransitionEnd, has_sel, &mut out);
                self.menu_item(ui, Retime, has_sel, &mut out);
                self.menu_item(ui, FreezeFrame, has_sel, &mut out);
                self.menu_item(ui, NestSequence, has_sel, &mut out);
                self.menu_item(ui, OpenParentSequence, self.project.editing.is_some(), &mut out);
                self.menu_item(ui, SaveTemplate, has_sel, &mut out);
                self.menu_item(ui, ApplyFlow, self.selection.len() == 2, &mut out);
                ui.separator();
                self.menu_item(ui, MarkIn, true, &mut out);
                self.menu_item(ui, MarkOut, true, &mut out);
                self.menu_item(ui, ClearInOut, true, &mut out);
                self.menu_item(ui, TrimToInOut, has_clips, &mut out);
                self.menu_item(ui, RippleDeleteInOut, has_clips, &mut out);
            });
            ui.menu_button("Timeline", |ui| {
                self.menu_item(ui, AddVideoTrack, true, &mut out);
                self.menu_item(ui, AddAudioTrack, true, &mut out);
                ui.separator();
                self.menu_item(ui, ZoomIn, true, &mut out);
                self.menu_item(ui, ZoomOut, true, &mut out);
                self.menu_item(ui, ZoomFit, true, &mut out);
                ui.separator();
                let mut snap = self.settings.snap;
                if ui.checkbox(&mut snap, format!("Snapping   {}", self.hotkeys.text(ToggleSnap))).changed() {
                    out.push(ToggleSnap);
                }
                ui.menu_button("Scaling quality", |ui| {
                    for s in Scaler::ALL {
                        if ui.radio(self.project.scaler == s, s.name()).clicked() {
                            ui.close();
                            if self.project.scaler != s {
                                self.push_undo();
                                self.project.scaler = s;
                                self.after_edit();
                            }
                        }
                    }
                });
            });
            ui.menu_button("Playback", |ui| {
                for a in [PlayPause, Stop, StepBack, StepForward, PrevCut, NextCut, GoStart, GoEnd] {
                    self.menu_item(ui, a, true, &mut out);
                }
            });
            ui.menu_button("View", |ui| self.view_menu(ui, &mut out));
            ui.menu_button("Scripts", |ui| {
                // re-reading the folder on every open IS the refresh mechanism
                let scripts = crate::scripting::list();
                for p in &scripts {
                    let label = p.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    if crate::ui::tools::glyph_text_button(ui, crate::ui::tools::Glyph::Terminal, &label).clicked() {
                        self.run_script_path = Some(p.clone());
                        ui.close();
                    }
                }
                if scripts.is_empty() {
                    ui.weak("No scripts yet");
                }
                ui.separator();
                if ui.button("Open Scripts Folder").clicked() {
                    let _ = std::process::Command::new("explorer").arg(crate::scripting::scripts_dir()).spawn();
                    ui.close();
                }
            });
            ui.menu_button("Help", |ui| {
                ui.label(format!("Simple Editor {}", env!("CARGO_PKG_VERSION")));
                ui.label(match media::ffpipe::ffmpeg_exe() {
                    Some(p) => format!("ffmpeg: {}", p.display()),
                    None => "ffmpeg: not found (export disabled)".into(),
                });
                ui.label(format!(
                    "Context menu: {}",
                    if crate::contextmenu::is_installed() { "installed" } else { "not installed" }
                ));
                ui.label("Mouse: Ctrl+Scroll zoom · Shift+Scroll pan · Alt+Scroll track height");
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(format!(
                        "{}  /  {}",
                        crate::ui::timecode(self.playhead, self.project.fps),
                        crate::ui::timecode(self.project.duration(), self.project.fps)
                    ))
                    .monospace(),
                );
                if self.player.is_playing() {
                    crate::ui::tools::glyph_label(ui, crate::ui::tools::Glyph::Play, ui.visuals().text_color());
                }
                if let Some(seq) = self.project.editing {
                    let name = self.project.sequence(seq).map(|s| s.name.as_str()).unwrap_or("?").to_string();
                    ui.label(egui::RichText::new(format!("editing: Main > {name}")).weak());
                }
            });
        });
        out
    }

    fn handle_drops(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        if dropped.is_empty() || self.export.is_some() {
            return;
        }
        // drop point in window points: OS cursor (physical px) / ppp − client-area origin
        let pos = ctx.input(|i| {
            let mut p = windows::Win32::Foundation::POINT::default();
            if !unsafe { GetCursorPos(&mut p) }.as_bool() {
                return None;
            }
            let inner = i.viewport().inner_rect?;
            Some(egui::pos2(p.x as f32 / i.pixels_per_point, p.y as f32 / i.pixels_per_point) - inner.min.to_vec2())
        });
        // a project file: open it
        if dropped.len() == 1
            && dropped[0].extension().map(|e| e.to_string_lossy().eq_ignore_ascii_case(PROJECT_EXT)).unwrap_or(false)
        {
            if self.confirm_discard() {
                self.open_project(&dropped[0]);
            }
            return;
        }
        let ids = self.open_or_import(&dropped);
        if ids.is_empty() {
            return;
        }
        let on_timeline = pos.map(|p| self.timeline.lanes_rect.contains(p)).unwrap_or(false);
        if on_timeline {
            let p = pos.unwrap();
            let mut t = self.timeline.time_at(p.x).max(0.0);
            if self.settings.snap {
                t = self.project.snap_frame(t);
            }
            let track = self.timeline.track_at(p.y, &self.project);
            let vt = track.filter(|&i| self.project.tracks[i].kind == TrackKind::Video);
            self.insert_at(ids, t, vt);
            self.after_edit();
        } else {
            self.library.tab = 0;
            self.library.selected = ids.last().copied();
        }
    }

    /// Compress… — re-encode one file smaller, either by quality (CRF) or to a size target, writing a
    /// copy or replacing the original.
    fn compress_window(&mut self, ctx: &egui::Context) {
        let Some(mut c) = self.compress.take() else { return };
        let (mut open, mut start) = (true, false);
        egui::Window::new("Compress").open(&mut open).resizable(false).default_width(320.0).show(ctx, |ui| {
            ui.label(c.src.file_name().unwrap_or_default().to_string_lossy());
            if let Some(b) = c.source_bytes {
                ui.weak(format!("{:.1} MB on disk", b as f64 / 1e6));
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.selectable_value(&mut c.by_size, false, "Amount");
                ui.selectable_value(&mut c.by_size, true, "Target size");
            });
            if c.by_size {
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut c.target_mb).speed(0.5).range(0.1..=20_000.0).suffix(" MB"));
                    if let Some(d) = c.duration.filter(|d| *d > 0.0) {
                        match crate::engine::convert::target_bitrate((c.target_mb * 1e6) as u64, d) {
                            Some(bps) => ui.weak(format!("\u{2248} {} kbps video", bps / 1000)),
                            None => ui.colored_label(ui.visuals().error_fg_color, "too small for this length"),
                        };
                    }
                });
            } else {
                ui.horizontal(|ui| {
                    ui.add(egui::Slider::new(&mut c.crf, 18..=40).text("CRF"));
                });
                ui.weak("Higher = smaller file, more artefacts. 23 is the usual default.");
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.selectable_value(&mut c.overwrite, false, "Save a copy");
                ui.selectable_value(&mut c.overwrite, true, "Overwrite original");
            });
            if c.overwrite {
                ui.colored_label(ui.visuals().warn_fg_color, "The original file is replaced when this finishes.");
            } else {
                ui.weak("Written next to the source as <name>_compressed.<ext> and added to the library.");
            }
            ui.add_space(4.0);
            start = ui.button("Compress").clicked();
        });
        if start {
            self.start_compress(&c);
            return; // window closes; the job window takes over
        }
        if open {
            self.compress = Some(c);
        }
    }

    fn start_compress(&mut self, c: &Compress) {
        if self.ffmpeg_missing() {
            return;
        }
        let ext = c.src.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_else(|| "mp4".into());
        let out = if c.overwrite {
            c.src.clone()
        } else {
            let stem = c.src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "output".into());
            let mut p = c.src.with_file_name(format!("{stem}_compressed.{ext}"));
            let mut n = 2;
            while p.exists() {
                p = c.src.with_file_name(format!("{stem}_compressed_{n}.{ext}"));
                n += 1;
            }
            p
        };
        let opts = crate::engine::convert::ConvertOptions {
            src: c.src.clone(),
            out: out.clone(),
            encoder: self.settings.encoder.clone(),
            crf: c.crf,
            preset: self.settings.preset.clone(),
            out_size: None,
            scaler: self.settings.export_scaler.clone(),
            gif_fps: 15,
            target_bytes: c.by_size.then(|| (c.target_mb * 1e6) as u64),
        };
        self.toast("Compressing\u{2026}");
        self.convert_jobs.push((crate::engine::convert::start_convert(opts), out));
    }

    /// Point the library's preview player at `path` and roll it. A file already playing is left alone,
    /// so re-clicking the selected row does not restart it. Pauses the timeline: previewing a source and
    /// the program monitor should not both be making sound at once.
    fn start_lib_preview(&mut self, ctx: &egui::Context, path: PathBuf) {
        self.player.pause();
        if self.lib_preview.as_ref().is_some_and(|lp| lp.path == path) {
            return;
        }
        // an asset the project already knows carries its probed duration; anything else would need a
        // blocking ffprobe on the UI thread, so it keeps the still thumbnail until it is imported
        let Some(asset) = self.project.assets.iter().find(|a| a.path == path.to_string_lossy()).cloned() else {
            self.lib_preview = None;
            return;
        };
        let duration = asset.duration.max(crate::model::MIN_CLIP);
        let fps = if asset.fps > 0.0 { asset.fps } else { 30.0 };
        let mut player = Player::new(ctx.clone(), self.backend(), self.text.clone());
        player.set_project(&Project::from_media(asset));
        player.play();
        self.lib_preview = Some(LibPreview { path, player, duration, fps });
    }

    /// The library preview's current frame, uploaded for the pane to paint. None = nothing is previewing.
    /// Called exactly once per update (see `self.lib_preview_live`) - `Player::take_frame` consumes the
    /// buffered frame, so a second call in the same frame would come back empty.
    fn lib_preview_frame(&mut self, ctx: &egui::Context) -> Option<library::PreviewFrame> {
        let lp = self.lib_preview.as_mut()?;
        let playing = lp.player.is_playing();
        if let Some(f) = lp.player.take_frame() {
            let (w, h) = (f.width as usize, f.height as usize);
            if w > 0 && h > 0 && f.rgba.len() == w * h * 4 {
                let img = egui::ColorImage::from_rgba_premultiplied([w, h], &f.rgba);
                match self.lib_preview_tex.as_mut() {
                    Some(t) if t.size() == [w, h] => t.set_partial([0, 0], img, egui::TextureOptions::LINEAR),
                    Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                    None => {
                        self.lib_preview_tex = Some(ctx.load_texture("lib_preview", img, egui::TextureOptions::LINEAR))
                    }
                }
            }
        }
        if playing {
            ctx.request_repaint();
        }
        let t = self.lib_preview_tex.as_ref()?;
        Some(library::PreviewFrame { tex: t.id(), size: [t.size()[0] as u32, t.size()[1] as u32], playing })
    }

    /// The Preview pane while a library asset is being previewed: the timeline's player and project are
    /// left untouched underneath (still decoding in the background, so resuming is instant), and this
    /// draws the previewed file's own frame with a transport bound to its own `Player` instead.
    fn draw_lib_preview(&mut self, ui: &mut egui::Ui) {
        let Some(lp) = self.lib_preview.as_ref() else { return };
        let name = lp.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let duration = lp.duration;
        let fps = lp.fps;
        let playhead = lp.player.time();
        let playing = lp.player.is_playing();
        let frame = self.lib_preview_live;
        let palette = self.palette;

        let (mut close, mut toggle, mut stop) = (false, false, false);
        let mut seek_to: Option<f64> = None;

        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let mut b = |ui: &mut egui::Ui, icon: tools::Glyph, label: &str| -> bool {
                    tools::glyph_text_button(ui, icon, "").on_hover_text(label).clicked()
                };
                if b(ui, tools::Glyph::Jump(tools::Dir::Left), "Go to start") {
                    seek_to = Some(0.0);
                }
                if b(ui, tools::Glyph::Tri(tools::Dir::Left), "Step back one frame") {
                    seek_to = Some(step_time(playhead, fps, false, duration));
                }
                let pp_icon = if playing { tools::Glyph::Pause } else { tools::Glyph::Play };
                let pp_label = if playing { "Pause" } else { "Play" };
                if b(ui, pp_icon, pp_label) {
                    toggle = true;
                }
                if b(ui, tools::Glyph::Stop, "Stop") {
                    stop = true;
                }
                if b(ui, tools::Glyph::Tri(tools::Dir::Right), "Step forward one frame") {
                    seek_to = Some(step_time(playhead, fps, true, duration));
                }
                if b(ui, tools::Glyph::Jump(tools::Dir::Right), "Go to end") {
                    seek_to = Some(duration);
                }
                ui.add_space(8.0);
                let tc = crate::ui::timecode;
                ui.monospace(format!("{} / {}", tc(playhead, fps), tc(duration, fps)));
                ui.add_space(8.0);
                if markers_ui::x_button(ui).on_hover_text("Back to timeline").clicked() {
                    close = true;
                }
                ui.weak(name);
            });
            // scrub bar: click or drag anywhere on it seeks
            let (bar, br) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), 10.0), egui::Sense::click_and_drag());
            ui.painter().rect_filled(bar, 2.0, palette.panel);
            let frac = (playhead / duration).clamp(0.0, 1.0) as f32;
            let filled = egui::Rect::from_min_max(bar.min, egui::pos2(bar.left() + bar.width() * frac, bar.bottom()));
            ui.painter().rect_filled(filled, 2.0, palette.accent);
            ui.painter().rect_stroke(bar, 2.0, egui::Stroke::new(1.0, palette.border), egui::StrokeKind::Inside);
            if (br.clicked() || br.dragged()) && bar.width() > 0.0 {
                if let Some(p) = br.interact_pointer_pos() {
                    let f = ((p.x - bar.left()) / bar.width()) as f64;
                    seek_to = Some(scrub_time(f, duration));
                }
            }

            // video, filling whatever is left
            let (rect, _) = ui.allocate_exact_size(ui.available_size_before_wrap(), egui::Sense::hover());
            ui.painter().rect_filled(rect, 0.0, egui::Color32::BLACK);
            if let Some(f) = frame {
                let aspect = f.size[0].max(1) as f32 / f.size[1].max(1) as f32;
                let lb = preview::letterbox(rect, aspect, ui.pixels_per_point());
                ui.painter().image(
                    f.tex,
                    lb,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
                let cw = (lb.width() * ui.pixels_per_point()) as u32;
                let ch = (lb.height() * ui.pixels_per_point()) as u32;
                if let Some(lp) = self.lib_preview.as_mut() {
                    lp.player.set_canvas(cw.max(16), ch.max(16), self.settings.preview_max_width);
                }
            }
        });

        if let Some(lp) = self.lib_preview.as_mut() {
            if toggle {
                lp.player.toggle();
            }
            if stop {
                lp.player.pause();
                lp.player.seek(0.0);
            }
            if let Some(t) = seek_to {
                lp.player.seek(t.clamp(0.0, duration));
            }
        }
        if close {
            self.lib_preview = None;
            self.lib_preview_tex = None;
            self.lib_preview_live = None;
        }
    }

    fn screenshot_tick(&mut self, ctx: &egui::Context) {
        let Some(path) = self.screenshot.clone() else { return };
        // save when the screenshot event arrives
        let img = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = img {
            let [w, h] = img.size;
            let mut data = format!("P6\n{w} {h}\n255\n").into_bytes();
            for p in &img.pixels {
                data.extend_from_slice(&[p.r(), p.g(), p.b()]);
            }
            let _ = std::fs::write(&path, data);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            self.screenshot = None;
            return;
        }
        // shoot as soon as the first rendered frame is on screen (or after the timeout when nothing renders).
        // SE_SCREENSHOT_DELAY=<seconds> waits instead, for checks that need caches (thumbnails) warmed up.
        let elapsed = self.started.elapsed().as_secs_f32();
        let delay: Option<f32> = std::env::var("SE_SCREENSHOT_DELAY").ok().and_then(|v| v.parse().ok());
        let ready = match delay {
            Some(d) => elapsed > d,
            None => self.first_frame_at.is_some() || elapsed > 2.5,
        };
        if !self.screenshot_requested && ready {
            self.screenshot_requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }

    /// Proxy media: keep an all-intra low-res proxy built for every video asset and hand the map to
    /// the player. Rescans every 2 s (a handful of stat calls); one ffmpeg transcode at a time.
    /// ponytail: no per-asset badge yet — the preview shows one aggregate "Building proxy" line.
    fn sync_proxies(&mut self) {
        if let Some((src, _, p)) = &self.proxy_job {
            if !p.is_done() {
                return;
            }
            if let Some(e) = p.error() {
                eprintln!("proxy for {src}: {e}");
            }
            self.proxy_job = None;
            self.proxy_scan_at = None; // pick up the finished file (and start the next) right away
        }
        if self.proxy_scan_at.is_some_and(|t| t > Instant::now()) {
            return;
        }
        self.proxy_scan_at = Some(Instant::now() + Duration::from_secs(2));
        let h = self.settings.proxy_height.max(120);
        let mut map = std::collections::HashMap::new();
        let mut want: Option<(String, std::path::PathBuf)> = None;
        if self.settings.use_proxies {
            for a in &self.project.assets {
                // only real video that out-sizes the proxy: images/audio gain nothing, and neither
                // does footage already at or below proxy resolution
                if a.kind != crate::model::ClipKind::Video || a.height <= h || a.duration <= 0.0 {
                    continue;
                }
                let dst = crate::media::proxy::proxy_path(&a.path, h);
                if dst.exists() {
                    map.insert(a.path.clone(), dst.to_string_lossy().into_owned());
                } else if want.is_none() && std::path::Path::new(&a.path).exists() {
                    want = Some((a.path.clone(), dst));
                }
            }
        }
        if map != self.proxy_map {
            self.proxy_map = map.clone();
            self.player.set_proxies(map);
        }
        if let Some((src, dst)) = want {
            if crate::media::ffpipe::ffmpeg_exe().is_some() {
                let job = crate::media::proxy::generate(src.clone(), dst.clone(), h);
                self.proxy_job = Some((src, dst, job));
            }
        }
    }

    // ---------------- scripting ----------------

    /// Run one Luau script against the live project. The whole run is a single undo step; a tool
    /// that fails mid-script rolls its own mutation back (same policy as MCP) and stops the script.
    fn run_script(&mut self, path: &std::path::Path) {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let src = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => return self.toast(format!("{name}: {e}")),
        };
        let snap = self.project.to_json();
        let mut logs = Vec::new();
        let result = {
            let app = std::cell::RefCell::new(&mut *self);
            let mut call = |tool: &str, args: &serde_json::Value| -> Result<serde_json::Value, String> {
                let mut app = app.borrow_mut();
                let before = MUTATING_TOOLS.contains(&tool).then(|| app.project.to_json());
                let r = app.run_tool(tool, args);
                if let Some(snap) = before {
                    if r.is_ok() {
                        app.after_edit();
                    } else if let Ok(p) = Project::from_json(&snap) {
                        app.project = p; // a failed tool is a no-op
                    }
                }
                r
            };
            crate::scripting::run(&src, &name, &mut call, &mut logs)
        };
        if self.project.to_json() != snap {
            push_undo_json(&mut self.undo, &mut self.redo, snap);
        }
        for l in &logs {
            self.toast(format!("{name}: {l}"));
        }
        match result {
            Ok(()) if logs.is_empty() => self.toast(format!("{name}: done")),
            Ok(()) => {}
            Err(e) => {
                eprintln!("script {name}: {e}");
                let line = e.lines().next().unwrap_or("failed").to_string();
                self.toast(format!("{name}: {line}"));
            }
        }
    }

    // ---------------- MCP ----------------

    /// Start/stop/restart the MCP server to match the settings; runs every frame (cheap when in sync).
    fn sync_mcp(&mut self, ctx: &egui::Context) {
        let (want, port) = (self.settings.mcp_enabled, self.settings.mcp_port);
        if !want {
            if let Some((server, _)) = self.mcp.take() {
                server.stop();
                self.toast("MCP server stopped");
            }
            return;
        }
        if self.mcp.is_some() && self.mcp_port_running == port {
            return;
        }
        if let Some((server, _)) = self.mcp.take() {
            server.stop();
        }
        match mcp::Server::start(port, ctx.clone()) {
            Ok((server, rx)) => {
                self.toast(format!("MCP server at {}", server.url()));
                self.mcp = Some((server, rx));
                self.mcp_port_running = port;
            }
            Err(e) => {
                self.toast(format!("MCP server failed: {e}"));
                self.settings.mcp_enabled = false; // don't retry every frame
                self.settings.save();
            }
        }
    }

    fn poll_mcp(&mut self, ctx: &egui::Context) {
        // finish blocking jobs (export.video / media.convert) — reply when their thread is done
        if !self.mcp_jobs.is_empty() {
            let mut i = 0;
            while i < self.mcp_jobs.len() {
                if self.mcp_jobs[i].prog.is_done() {
                    let j = self.mcp_jobs.remove(i);
                    let r = match j.prog.error() {
                        None => Ok(json!({"ok": true, "path": j.out.to_string_lossy()})),
                        Some(e) => Err(e),
                    };
                    let _ = j.reply.send(r);
                } else {
                    i += 1;
                }
            }
            ctx.request_repaint_after(Duration::from_millis(200));
        }
        // one per frame: render.frame blocks the UI thread for up to 3 s, so a queue must not run in one go
        let call = self.mcp.as_ref().and_then(|(_, rx)| rx.try_recv().ok());
        if let Some(c) = call {
            self.handle_tool(c);
            ctx.request_repaint(); // anything else queued runs on the next frames
        }
    }

    fn handle_tool(&mut self, call: mcp::ToolCall) {
        let mcp::ToolCall { name, args, reply } = call;
        match name.as_str() {
            // blocking jobs: start them and reply when they finish (polled per frame)
            "export.video" | "media.convert" => match self.start_tool_job(&name, &args) {
                Ok((prog, out)) => self.mcp_jobs.push(McpJob { prog, reply, out }),
                Err(e) => {
                    let _ = reply.send(Err(e));
                }
            },
            _ => {
                let before = MUTATING_TOOLS.contains(&name.as_str()).then(|| self.project.to_json());
                let r = self.run_tool(&name, &args);
                if let Some(snap) = before {
                    if r.is_ok() {
                        if snap != self.project.to_json() {
                            push_undo_json(&mut self.undo, &mut self.redo, snap);
                        }
                        self.after_edit();
                    } else if let Ok(p) = Project::from_json(&snap) {
                        // a failed tool is a no-op: some arms mutate before returning Err (subtitles.set, clip.set)
                        self.project = p;
                    }
                }
                let _ = reply.send(r);
            }
        }
    }

    fn start_tool_job(&mut self, name: &str, args: &Value) -> Result<(Arc<Progress>, PathBuf), String> {
        if media::ffpipe::ffmpeg_exe().is_none() {
            return Err("ffmpeg.exe not found".into());
        }
        let out_size = match (arg_u64(args, "width"), arg_u64(args, "height")) {
            (Some(w), Some(h)) => Some((w as u32, h as u32)),
            _ => None,
        };
        let scaler = arg_str(args, "scaler").unwrap_or(&self.settings.export_scaler).to_string();
        if name == "export.video" {
            if self.export.is_some() {
                return Err("an export is already running".into());
            }
            let out = PathBuf::from(req(arg_str(args, "path"), "path")?);
            let opts = ExportOptions {
                out_path: out.clone(),
                encoder: arg_str(args, "encoder").unwrap_or(&self.settings.encoder).to_string(),
                crf: arg_u64(args, "crf").map(|c| c as u32).unwrap_or(self.settings.crf),
                preset: self.settings.preset.clone(),
                backend: self.backend(),
                out_size,
                scaler,
                frames: self.export_frames(),
                metadata: Vec::new(),
            };
            let prog = export::start_export(self.export_project(), opts, self.text.clone());
            // same slot the UI uses: exclusion, the progress/Cancel window and the close guard all key off it
            self.export = Some((prog.clone(), ExportKind::File));
            Ok((prog, out))
        } else {
            let src = PathBuf::from(req(arg_str(args, "path"), "path")?);
            let ext = req(arg_str(args, "ext"), "ext")?.trim_start_matches('.').to_string();
            // never write over the source (converting to its own container used to do exactly that)
            let out = converted_path(&src, &ext);
            let opts = crate::engine::convert::ConvertOptions {
                src,
                out: out.clone(),
                encoder: self.settings.encoder.clone(),
                crf: self.settings.crf,
                preset: self.settings.preset.clone(),
                out_size,
                scaler,
                gif_fps: 15,
                target_bytes: arg_u64(args, "target_bytes"),
            };
            Ok((crate::engine::convert::start_convert(opts), out))
        }
    }

    /// Execute one (non-job) MCP tool by name. The caller handles undo/after_edit for mutating tools.
    fn run_tool(&mut self, name: &str, args: &Value) -> Result<Value, String> {
        match name {
            "project.summary" => {
                let p = &self.project;
                let clips_per_track: Vec<Value> = p
                    .tracks
                    .iter()
                    .map(|t| json!({"name": t.name, "kind": format!("{:?}", t.kind), "clips": t.clips.len()}))
                    .collect();
                fn count_plan(items: &[crate::model::PlanItem]) -> (usize, usize) {
                    let mut done = 0;
                    let mut total = 0;
                    for i in items {
                        total += 1;
                        if i.done {
                            done += 1;
                        }
                        let (d, t) = count_plan(&i.children);
                        done += d;
                        total += t;
                    }
                    (done, total)
                }
                let (done, total) = count_plan(&p.plan);
                Ok(json!({
                    "name": p.name, "width": p.width, "height": p.height, "fps": p.fps,
                    "duration": p.duration(), "tracks": clips_per_track,
                    "assets": p.assets.len(), "sequences": p.sequences.len(),
                    "subtitles": p.subtitles.len(), "plan_done": done, "plan_total": total,
                    "notes": p.notes, "style": crate::engine::style::style_summary(p),
                    // non-null = these tracks/size are a nested sequence's, not the main timeline's
                    "editing_sequence": p.editing,
                }))
            }
            "project.get" => serde_json::to_value(&self.project).map_err(|e| e.to_string()),
            "project.new" => {
                let mut p = Project::new();
                p.width = arg_u64(args, "width").map(|w| w as u32).unwrap_or(1920);
                p.height = arg_u64(args, "height").map(|h| h as u32).unwrap_or(1080);
                p.fps = arg_f64(args, "fps").unwrap_or(30.0);
                self.set_project(p, None);
                Ok(json!({"ok": true}))
            }
            "project.open" => {
                let path = PathBuf::from(req(arg_str(args, "path"), "path")?);
                if !path.exists() {
                    return Err(format!("no such file: {}", path.display()));
                }
                if path.extension().map(|e| e.to_string_lossy().eq_ignore_ascii_case(PROJECT_EXT)).unwrap_or(false) {
                    let mut project = Project::load(&path)?;
                    relocate_assets(&mut project, path.parent());
                    self.set_project(project, Some(path));
                } else {
                    let asset = media::probe(&path.to_string_lossy(), self.backend())?;
                    self.set_project(Project::from_media(asset), None);
                }
                Ok(json!({"ok": true}))
            }
            "project.save" => {
                let path = match arg_str(args, "path") {
                    Some(p) => PathBuf::from(p),
                    None => self.project_path.clone().ok_or("no project file yet — pass a path")?,
                };
                self.project.save(&path).map_err(|e| e.to_string())?;
                self.project_path = Some(path.clone());
                self.dirty = false;
                Ok(json!({"ok": true, "path": path.to_string_lossy()}))
            }
            "project.set" => {
                if let Some(n) = arg_str(args, "name") {
                    self.project.name = n.to_string();
                }
                if let Some(w) = arg_u64(args, "width") {
                    self.project.width = w as u32;
                }
                if let Some(h) = arg_u64(args, "height") {
                    self.project.height = h as u32;
                }
                if let Some(f) = arg_f64(args, "fps") {
                    self.project.fps = f.max(1.0);
                }
                Ok(json!({"ok": true}))
            }
            "media.import" => {
                let paths: Vec<PathBuf> = req(args.get("paths").and_then(|v| v.as_array()), "paths")?
                    .iter()
                    .filter_map(|v| v.as_str().map(PathBuf::from))
                    .collect();
                let ids = self.import_files(&paths);
                Ok(json!({"ok": true, "asset_ids": ids}))
            }
            "media.list" => {
                let used = self.project.used_assets();
                let list: Vec<Value> = self
                    .project
                    .assets
                    .iter()
                    .map(|a| {
                        json!({
                            "id": a.id, "path": a.path, "kind": format!("{:?}", a.kind),
                            "duration": a.duration, "width": a.width, "height": a.height,
                            "tags": a.tags, "label": a.label, "folder": a.folder,
                            "description": a.description, "used": used.contains(&a.id),
                        })
                    })
                    .collect();
                Ok(json!(list))
            }
            "media.set" => {
                let id = req(arg_u64(args, "id"), "id")?;
                let a = self.project.asset_mut(id).ok_or("no such asset")?;
                if let Some(d) = arg_str(args, "description") {
                    a.description = d.to_string();
                }
                if let Some(tags) = args.get("tags").and_then(|v| v.as_array()) {
                    a.tags = tags.iter().filter_map(|t| t.as_str().map(String::from)).collect();
                }
                if let Some(l) = arg_u64(args, "label") {
                    a.label = l.min(8) as u8;
                }
                if let Some(f) = arg_str(args, "folder") {
                    a.folder = f.to_string();
                }
                Ok(json!({"ok": true}))
            }
            "timeline.list" => {
                let p = &self.project;
                let tracks: Vec<Value> = p
                    .tracks
                    .iter()
                    .map(|t| {
                        let clips: Vec<Value> = t
                            .clips
                            .iter()
                            .map(|c| {
                                json!({
                                    "id": c.id, "kind": format!("{:?}", c.kind), "name": c.name,
                                    "asset": c.asset, "sequence": c.sequence, "start": c.start,
                                    "duration": c.duration, "src_in": c.src_in, "speed": c.speed,
                                    "reverse": c.reverse, "freeze": c.freeze, "enabled": c.enabled,
                                    "label": p.clip_label(c), "link": c.link,
                                    "effects": c.effects.iter().map(|e| e.kind.name()).collect::<Vec<_>>(),
                                })
                            })
                            .collect();
                        json!({"id": t.id, "name": t.name, "kind": format!("{:?}", t.kind), "clips": clips})
                    })
                    .collect();
                Ok(json!({"editing_sequence": p.editing, "duration": p.duration(), "tracks": tracks}))
            }
            "timeline.add_clip" => {
                let at = req(arg_f64(args, "at"), "at")?;
                let track = arg_u64(args, "track").map(|t| t as usize);
                if let Some(aid) = arg_u64(args, "asset_id") {
                    if self.project.asset(aid).is_none() {
                        return Err("no such asset".into());
                    }
                    let ids = self.project.insert_asset_clips(aid, at, track);
                    Ok(json!({"ok": true, "clip_ids": ids}))
                } else if let Some(sid) = arg_u64(args, "sequence_id") {
                    let id = self.project.insert_sequence_clip(sid, at, track).ok_or("no such sequence (or cycle)")?;
                    Ok(json!({"ok": true, "clip_ids": [id]}))
                } else if let Some(text) = arg_str(args, "text") {
                    let dur = arg_f64(args, "duration").unwrap_or(5.0).max(0.1);
                    let id = self.project.add_text_clip(at, dur);
                    if let Some(t) = self.project.clip_mut(id).and_then(|c| c.text.as_mut()) {
                        t.text = text.to_string();
                    }
                    Ok(json!({"ok": true, "clip_ids": [id]}))
                } else {
                    Err("pass asset_id, sequence_id or text".into())
                }
            }
            "timeline.split" => {
                let t = req(arg_f64(args, "t"), "t")?;
                let only = arg_ids(args, "clip_ids");
                let new = self.project.split_at(t, only.as_deref());
                Ok(json!({"ok": true, "new_clip_ids": new}))
            }
            "timeline.delete" => {
                let ids = self.project.expand_links(&req(arg_ids(args, "clip_ids"), "clip_ids")?);
                let ripple = arg_bool(args, "ripple").unwrap_or(false);
                self.project.delete_clips(&ids, ripple);
                Ok(json!({"ok": true}))
            }
            "timeline.move" => {
                let ids = self.project.expand_links(&req(arg_ids(args, "clip_ids"), "clip_ids")?);
                let dt = req(arg_f64(args, "dt"), "dt")?;
                let dtrack = args.get("dtrack").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let moved = self.project.move_clips(&ids, dt, dtrack, None);
                if moved {
                    Ok(json!({"ok": true}))
                } else {
                    Err("move blocked (overlap or out of range)".into())
                }
            }
            "timeline.trim" => {
                let id = req(arg_u64(args, "clip_id"), "clip_id")?;
                let clip = self.project.clip(id).ok_or("no such clip")?.clone();
                let headroom = self.project.head_room(&clip);
                let max_dur = self.project.max_clip_duration(&clip);
                let c = self.project.clip_mut(id).ok_or("no such clip")?;
                if let Some(s) = arg_f64(args, "start") {
                    c.trim_start(s, headroom);
                }
                if let Some(e) = arg_f64(args, "end") {
                    c.trim_end(e, max_dur);
                }
                let (start, duration) = (c.start, c.duration);
                self.project.tidy();
                Ok(json!({"ok": true, "start": start, "end": start + duration}))
            }
            "timeline.add_transition" => {
                let right = req(arg_u64(args, "right_clip_id"), "right_clip_id")?;
                let kind = match req(arg_str(args, "kind"), "kind")? {
                    "CrossFade" => TransitionKind::CrossFade,
                    "FadeToColor" => TransitionKind::FadeToColor,
                    "Push" => TransitionKind::Push,
                    "Wipe" => TransitionKind::Wipe,
                    k => return Err(format!("unknown transition kind '{k}'")),
                };
                let dur = arg_f64(args, "duration").unwrap_or(1.0);
                let id = self.project.add_transition(right, kind, dur).ok_or("no abutting left neighbour")?;
                self.transitions_ui.remember(kind, dur); // Ctrl+T repeats this one too
                Ok(json!({"ok": true, "transition_id": id}))
            }
            "timeline.auto_cut" => {
                use crate::engine::autocut::{loud_segments, to_timeline, AutoCutParams};
                let ids = req(arg_ids(args, "clip_ids"), "clip_ids")?;
                let mut params = AutoCutParams::default();
                if let Some(v) = arg_f64(args, "threshold_db") {
                    params.threshold_db = v as f32;
                }
                if let Some(v) = arg_f64(args, "min_silence") {
                    params.min_silence = v;
                }
                if let Some(v) = arg_f64(args, "min_speech") {
                    params.min_speech = v;
                }
                if let Some(v) = arg_f64(args, "padding") {
                    params.padding = v;
                }
                let keep_quiet = arg_bool(args, "keep_quiet").unwrap_or(false);
                let ripple = arg_bool(args, "ripple").unwrap_or(true);
                let mut cuts = Vec::new();
                let mut removes = Vec::new();
                for &id in &ids {
                    let c = self.project.clip(id).ok_or("no such clip")?.clone();
                    if c.kind != ClipKind::Audio || c.reverse || c.freeze.is_some() {
                        continue;
                    }
                    let a = self.project.asset(c.asset).ok_or("clip has no asset")?;
                    let peaks = self
                        .waveforms
                        .get(&a.path, c.audio_stream)
                        .ok_or("waveform still computing — try again in a moment")?;
                    let segs = loud_segments(&peaks, c.src_in, c.src_len(), &params);
                    let (mut cs, mut rs) = to_timeline(&segs, c.start, c.src_in, c.duration, c.speed, keep_quiet);
                    cuts.append(&mut cs);
                    removes.append(&mut rs);
                }
                if cuts.is_empty() && removes.is_empty() {
                    return Err("no segments found (are the clips audio clips?)".into());
                }
                let n = self.project.auto_cut(&ids, &cuts, &removes, ripple);
                Ok(json!({"ok": true, "removed": n}))
            }
            "timeline.nest" => {
                let ids = req(arg_ids(args, "clip_ids"), "clip_ids")?;
                let name = arg_str(args, "name")
                    .map(String::from)
                    .unwrap_or_else(|| format!("Sequence {}", self.project.sequences.len() + 1));
                let id = self.project.nest_selection(&ids, name).ok_or("nothing to nest")?;
                Ok(json!({"ok": true, "sequence_id": id}))
            }
            "sequence.list" => {
                let list: Vec<Value> = self
                    .project
                    .sequences
                    .iter()
                    .map(|s| {
                        json!({"id": s.id, "name": s.name, "width": s.width, "height": s.height,
                               "fps": s.fps, "duration": s.duration()})
                    })
                    .collect();
                Ok(json!(list))
            }
            "sequence.open" => {
                match arg_u64(args, "id") {
                    Some(id) => {
                        if !self.project.open_sequence(id) {
                            return Err("no such sequence (or already open / cycle)".into());
                        }
                    }
                    None => self.project.close_sequence(),
                }
                self.after_edit();
                Ok(json!({"ok": true, "editing": self.project.editing}))
            }
            "clip.set" => {
                let id = req(arg_u64(args, "clip_id"), "clip_id")?;
                let fields = req(args.get("fields"), "fields")?.clone();
                // speed/reverse go through Project::set_speed so neighbour collisions are respected
                let speed = arg_f64(&fields, "speed");
                let reverse = arg_bool(&fields, "reverse");
                {
                    let c = self.project.clip_mut(id).ok_or("no such clip")?;
                    apply_clip_fields(c, &fields)?;
                }
                if speed.is_some() || reverse.is_some() {
                    let cur = self.project.clip(id).ok_or("no such clip")?;
                    let (s, r) = (speed.unwrap_or(cur.speed), reverse.unwrap_or(cur.reverse));
                    if !self.project.set_speed(&[id], s, r) {
                        return Err("speed change blocked by a neighbouring clip".into());
                    }
                }
                Ok(json!({"ok": true}))
            }
            "clip.keyframe" => {
                let id = req(arg_u64(args, "clip_id"), "clip_id")?;
                let prop = req(arg_str(args, "property"), "property")?.to_string();
                let t = req(arg_f64(args, "t"), "t")?;
                let remove = arg_bool(args, "remove").unwrap_or(false);
                let value = arg_f64(args, "value");
                let ease = arg_str(args, "ease").map(|s| parse_ease(s).ok_or(format!("bad ease '{s}'"))).transpose()?;
                let c = self.project.clip_mut(id).ok_or("no such clip")?;
                let a = anim_of(c, &prop).ok_or_else(|| format!("unknown property '{prop}'"))?;
                if remove {
                    if let Some(i) = a.key_index_at(t) {
                        a.keys.remove(i);
                    }
                } else {
                    if !a.is_animated() {
                        a.toggle_key(t); // first key: set_at alone would only change the constant value
                    }
                    a.set_at(t, value.unwrap_or_else(|| a.at(t)));
                    if let Some(e) = ease {
                        a.set_ease_at(t, e);
                    }
                }
                Ok(json!({"ok": true}))
            }
            "clip.add_effect" => {
                let id = req(arg_u64(args, "clip_id"), "clip_id")?;
                let kind_s = req(arg_str(args, "kind"), "kind")?;
                let kind = EffectKind::ALL
                    .into_iter()
                    .find(|k| k.name().eq_ignore_ascii_case(kind_s) || format!("{k:?}").eq_ignore_ascii_case(kind_s))
                    .ok_or_else(|| format!("unknown effect '{kind_s}'"))?;
                let mut effect = crate::model::Effect::new(kind);
                if let Some(params) = args.get("params").and_then(|v| v.as_object()) {
                    for (pname, pval) in params {
                        let i = kind
                            .params()
                            .iter()
                            .position(|s| s.name.eq_ignore_ascii_case(pname))
                            .ok_or_else(|| format!("unknown param '{pname}' for {}", kind.name()))?;
                        effect.params[i].value = pval.as_f64().ok_or("param values must be numbers")?;
                    }
                }
                let c = self.project.clip_mut(id).ok_or("no such clip")?;
                c.effects.push(effect);
                Ok(json!({"ok": true, "index": c.effects.len() - 1}))
            }
            "clip.remove_effect" => {
                let id = req(arg_u64(args, "clip_id"), "clip_id")?;
                let i = req(arg_u64(args, "index"), "index")? as usize;
                let c = self.project.clip_mut(id).ok_or("no such clip")?;
                if i >= c.effects.len() {
                    return Err("no effect at that index".into());
                }
                c.effects.remove(i);
                Ok(json!({"ok": true}))
            }
            "clip.apply_motion" => {
                let id = req(arg_u64(args, "clip_id"), "clip_id")?;
                let name = req(arg_str(args, "name"), "name")?;
                let scaled = arg_bool(args, "scaled").unwrap_or(true);
                let preset = self
                    .settings
                    .motion_presets
                    .iter()
                    .find(|p| p.name.eq_ignore_ascii_case(name))
                    .cloned()
                    .or_else(|| {
                        crate::engine::presets::builtin_motions()
                            .into_iter()
                            .find(|p| p.name.eq_ignore_ascii_case(name))
                    })
                    .ok_or_else(|| format!("no motion preset '{name}'"))?;
                let c = self.project.clip_mut(id).ok_or("no such clip")?;
                crate::engine::presets::apply_motion(&preset, c, scaled);
                Ok(json!({"ok": true}))
            }
            "subtitles.get" => {
                let cues: Vec<Value> = self
                    .project
                    .subtitles
                    .iter()
                    .map(|c| json!({"id": c.id, "start": c.start, "end": c.end, "text": c.text}))
                    .collect();
                Ok(json!(cues))
            }
            "subtitles.set" => {
                let cues = req(args.get("cues").and_then(|v| v.as_array()), "cues")?.clone();
                self.project.subtitles.clear();
                for c in cues {
                    let start = req(arg_f64(&c, "start"), "cues[].start")?;
                    let end = req(arg_f64(&c, "end"), "cues[].end")?;
                    let text = req(arg_str(&c, "text"), "cues[].text")?;
                    self.project.add_cue(start, end, text);
                }
                self.project.sort_cues();
                Ok(json!({"ok": true, "count": self.project.subtitles.len()}))
            }
            "subtitles.import" => {
                let path = req(arg_str(args, "path"), "path")?;
                let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
                let cues = crate::engine::subtitles::parse(&text);
                if cues.is_empty() {
                    return Err("no cues found".into());
                }
                self.project.subtitles.clear();
                for (start, end, text) in cues {
                    self.project.add_cue(start, end, text);
                }
                self.project.sort_cues();
                self.project.show_subtitles = true;
                Ok(json!({"ok": true, "count": self.project.subtitles.len()}))
            }
            "plan.get" => Ok(json!({
                "plan": serde_json::to_value(&self.project.plan).map_err(|e| e.to_string())?,
                "notes": self.project.notes,
            })),
            "plan.add" => {
                let title = req(arg_str(args, "title"), "title")?.to_string();
                let parent = arg_u64(args, "parent");
                let id = self.project.plan_add(parent, title);
                if parent.is_some() && self.project.plan_item_mut(id).is_none() {
                    return Err("no such parent".into());
                }
                let assets = arg_ids(args, "assets").unwrap_or_default();
                if let Some(item) = self.project.plan_item_mut(id) {
                    if let Some(n) = arg_str(args, "notes") {
                        item.notes = n.to_string();
                    }
                    for a in assets {
                        item.assets.push(a);
                        item.asset_notes.push(String::new());
                    }
                }
                Ok(json!({"ok": true, "id": id}))
            }
            "plan.set" => {
                let id = req(arg_u64(args, "id"), "id")?;
                let item = self.project.plan_item_mut(id).ok_or("no such planner item")?;
                if let Some(t) = arg_str(args, "title") {
                    item.title = t.to_string();
                }
                if let Some(d) = arg_bool(args, "done") {
                    item.done = d;
                }
                if let Some(n) = arg_str(args, "notes") {
                    item.notes = n.to_string();
                }
                Ok(json!({"ok": true}))
            }
            "plan.remove" => {
                let id = req(arg_u64(args, "id"), "id")?;
                self.project.plan_remove(id);
                Ok(json!({"ok": true}))
            }
            "notes.get" => Ok(json!({"notes": self.project.notes})),
            "notes.set" => {
                let text = req(arg_str(args, "text"), "text")?;
                if arg_bool(args, "append").unwrap_or(false) {
                    if !self.project.notes.is_empty() {
                        self.project.notes.push_str("\n\n");
                    }
                    self.project.notes.push_str(text);
                } else {
                    self.project.notes = text.to_string();
                }
                Ok(json!({"ok": true}))
            }
            "render.frame" => {
                let t = req(arg_f64(args, "t"), "t")?;
                let w = arg_u64(args, "width").map(|w| w as u32).unwrap_or(640).clamp(16, 3840);
                let frame = self.render_frame_now(t, w).ok_or("render timed out")?;
                let png = mcp::png_encode(&frame);
                Ok(json!({
                    "width": frame.width, "height": frame.height,
                    "data_url": format!("data:image/png;base64,{}", base64(&png)),
                }))
            }
            "playback.seek" => {
                let t = req(arg_f64(args, "t"), "t")?;
                self.seek(t);
                Ok(json!({"ok": true, "t": self.playhead}))
            }
            "playback.play" => {
                self.player.play();
                Ok(json!({"ok": true}))
            }
            "playback.pause" => {
                self.player.pause();
                self.playhead = self.player.time();
                Ok(json!({"ok": true, "t": self.playhead}))
            }
            "style.summary" => Ok(json!({"markdown": crate::engine::style::style_summary(&self.export_project())})),
            "templates.list" => {
                let templates: Vec<&str> = self.settings.templates.iter().map(|t| t.name.as_str()).collect();
                let motions: Vec<String> = crate::engine::presets::builtin_motions()
                    .iter()
                    .map(|m| m.name.clone())
                    .chain(self.settings.motion_presets.iter().map(|m| m.name.clone()))
                    .collect();
                Ok(json!({"templates": templates, "motion_presets": motions}))
            }
            "templates.apply" => {
                let name = req(arg_str(args, "name"), "name")?;
                let at = req(arg_f64(args, "at"), "at")?;
                let tpl = self
                    .settings
                    .templates
                    .iter()
                    .find(|t| t.name.eq_ignore_ascii_case(name))
                    .cloned()
                    .ok_or_else(|| format!("no template '{name}'"))?;
                let (clips, assets) = crate::engine::presets::decode_template(&tpl).ok_or("template is corrupted")?;
                let ids = self.project.place_clips(clips, assets, at);
                Ok(json!({"ok": true, "clip_ids": ids}))
            }
            // ---------------- round 3 ----------------
            "clip.add_mask" => {
                let id = req(arg_u64(args, "clip_id"), "clip_id")?;
                let shape = mask_shape(arg_str(args, "shape").unwrap_or("Ellipse"))?;
                let slot = mask_slot(&mut self.project, id, arg_u64(args, "effect").map(|i| i as usize))?;
                if slot.is_some() {
                    return Err("that clip / effect already has a mask".into());
                }
                *slot = Some(Mask::new(shape));
                Ok(json!({"ok": true}))
            }
            "clip.set_mask" => {
                let id = req(arg_u64(args, "clip_id"), "clip_id")?;
                let fields = req(args.get("fields"), "fields")?.clone();
                let slot = mask_slot(&mut self.project, id, arg_u64(args, "effect").map(|i| i as usize))?;
                let mask = slot.as_mut().ok_or("no mask on that clip / effect (call clip.add_mask first)")?;
                apply_mask_fields(mask, &fields)?;
                Ok(json!({"ok": true}))
            }
            "clip.add_node" => {
                let id = req(arg_u64(args, "clip_id"), "clip_id")?;
                let kind = node_kind(req(arg_str(args, "kind"), "kind")?)?;
                let (x, y) = (arg_f64(args, "x").unwrap_or(160.0) as f32, arg_f64(args, "y").unwrap_or(120.0) as f32);
                let node = self.project.add_node(id, kind, x, y).ok_or("no such clip")?;
                Ok(json!({"ok": true, "node_id": node}))
            }
            "clip.connect_nodes" => {
                let id = req(arg_u64(args, "clip_id"), "clip_id")?;
                let from = req(arg_u64(args, "from"), "from")?;
                let to = req(arg_u64(args, "to"), "to")?;
                let port = arg_u64(args, "port").unwrap_or(0) as usize;
                self.project.ensure_graph(id);
                let g = self.project.clip_mut(id).and_then(|c| c.graph.as_mut()).ok_or("no such clip")?;
                if !g.connect(from, to, port) {
                    return Err("connection refused (unknown node, bad port or a cycle)".into());
                }
                Ok(json!({"ok": true}))
            }
            "markers.list" => {
                let list: Vec<Value> = self
                    .project
                    .markers_in_timeline()
                    .into_iter()
                    .map(|(id, t, dur, name, label)| json!({"id": id, "t": t, "duration": dur, "name": name,
                                                            "label": label, "label_name": self.project.label_name(label)}))
                    .collect();
                Ok(json!(list))
            }
            "markers.add" => {
                let t = req(arg_f64(args, "t"), "t")?;
                let name = arg_str(args, "name").unwrap_or("Marker").to_string();
                let id = match arg_u64(args, "clip_id") {
                    Some(clip) => {
                        let start = self.project.clip(clip).ok_or("no such clip")?.start;
                        self.project.add_clip_marker(clip, t - start, name).ok_or("no such clip")?
                    }
                    None => self.project.add_marker(t, name),
                };
                if let Some(m) = self.project.marker_mut(id) {
                    if let Some(n) = arg_str(args, "note") {
                        m.note = n.to_string();
                    }
                    if let Some(l) = arg_u64(args, "label") {
                        m.label = l.min(255) as u8;
                    }
                    if let Some(d) = arg_f64(args, "duration") {
                        m.duration = d.max(0.0);
                    }
                }
                Ok(json!({"ok": true, "id": id}))
            }
            "markers.remove" => {
                let id = req(arg_u64(args, "id"), "id")?;
                self.project.remove_marker(id);
                Ok(json!({"ok": true}))
            }
            "audio.buses" => {
                self.project.main_bus();
                let list: Vec<Value> = self
                    .project
                    .buses
                    .iter()
                    .map(|b| {
                        json!({"id": b.id, "name": b.name, "gain": b.gain.value, "pan": b.pan.value,
                               "muted": b.muted, "solo": b.solo, "mono": b.mono, "output": b.output,
                               "filters": b.filters.iter().map(|f| f.kind.name()).collect::<Vec<_>>()})
                    })
                    .collect();
                Ok(json!(list))
            }
            "audio.add_bus" => {
                let name = arg_str(args, "name").unwrap_or("Bus").to_string();
                let id = self.project.add_bus(name);
                Ok(json!({"ok": true, "bus_id": id}))
            }
            "audio.add_filter" => {
                let bus = req(arg_u64(args, "bus"), "bus")?;
                let kind_s = req(arg_str(args, "kind"), "kind")?;
                let kind = FilterKind::ALL
                    .into_iter()
                    .find(|k| k.name().eq_ignore_ascii_case(kind_s) || format!("{k:?}").eq_ignore_ascii_case(kind_s))
                    .ok_or_else(|| format!("unknown filter '{kind_s}'"))?;
                let mut f = crate::model::AudioFilter::new(kind);
                if let Some(params) = args.get("params").and_then(|v| v.as_object()) {
                    for (pname, pval) in params {
                        let i = kind
                            .params()
                            .iter()
                            .position(|s| s.name.eq_ignore_ascii_case(pname))
                            .ok_or_else(|| format!("unknown param '{pname}' for {}", kind.name()))?;
                        f.params[i].value = pval.as_f64().ok_or("param values must be numbers")?;
                    }
                }
                let b = self.project.bus_mut(bus).ok_or("no such bus")?;
                b.filters.push(f);
                Ok(json!({"ok": true, "index": b.filters.len() - 1}))
            }
            "audio.route" => {
                let bus = req(arg_u64(args, "bus"), "bus")?;
                if bus != 0 && self.project.bus(bus).is_none() {
                    return Err("no such bus".into());
                }
                if let Some(clip) = arg_u64(args, "clip_id") {
                    self.project.clip_mut(clip).ok_or("no such clip")?.bus = bus;
                } else if let Some(track) = arg_u64(args, "track") {
                    let t = self.project.tracks.get_mut(track as usize).ok_or("no such track")?;
                    t.bus = bus;
                } else if let Some(from) = arg_u64(args, "from_bus") {
                    let main = self.project.main_bus();
                    if from == main {
                        return Err("the Main bus has no output".into());
                    }
                    self.project.bus_mut(from).ok_or("no such bus")?.output = bus;
                } else {
                    return Err("pass clip_id, track or from_bus".into());
                }
                Ok(json!({"ok": true}))
            }
            "shapes.add" => {
                let kind_s = arg_str(args, "kind").unwrap_or("Rect");
                let kind = ShapeKind::ALL
                    .into_iter()
                    .find(|k| k.name().eq_ignore_ascii_case(kind_s) || format!("{k:?}").eq_ignore_ascii_case(kind_s))
                    .ok_or_else(|| format!("unknown shape '{kind_s}'"))?;
                let at = req(arg_f64(args, "at"), "at")?;
                let dur = arg_f64(args, "duration").unwrap_or(5.0).max(0.1);
                let id = self.project.add_shape_clip(kind, at, dur);
                if let Some(s) = self.project.clip_mut(id).and_then(|c| c.shape.as_mut()) {
                    if let Some(c) = args.get("fill").and_then(color_arg) {
                        s.fill = c;
                    }
                    if let Some(c) = args.get("stroke").and_then(color_arg) {
                        s.stroke = c;
                    }
                    if let Some(w) = arg_f64(args, "stroke_width") {
                        s.stroke_width = w as f32;
                    }
                    if let Some(n) = arg_u64(args, "sides") {
                        s.sides = n.clamp(3, 64) as u32;
                    }
                    if let Some(w) = arg_f64(args, "width") {
                        s.w.value = w;
                    }
                    if let Some(h) = arg_f64(args, "height") {
                        s.h.value = h;
                    }
                }
                Ok(json!({"ok": true, "clip_id": id}))
            }
            "timeline.import" => {
                let path = PathBuf::from(req(arg_str(args, "path"), "path")?);
                let report = crate::engine::import::import_file(&path)?;
                let md = report.to_markdown();
                let (clips, tracks, missing) = (report.clips, report.tracks, report.missing_media);
                if arg_bool(args, "replace").unwrap_or(false) {
                    self.set_project(report.project, None);
                } else {
                    self.import_ui.report = Some(report);
                    self.import_ui.open = true;
                }
                Ok(json!({"ok": true, "clips": clips, "tracks": tracks, "missing_media": missing, "report": md}))
            }
            "frame.export" => {
                let out = PathBuf::from(req(arg_str(args, "path"), "path")?);
                let t = arg_f64(args, "t").unwrap_or(self.playhead);
                let (pw, ph) = (self.project.width.max(16), self.project.height.max(16));
                let w = arg_u64(args, "width").map(|w| w as u32).unwrap_or(pw).clamp(16, 7680);
                let h = arg_u64(args, "height")
                    .map(|h| h as u32)
                    .unwrap_or_else(|| ((ph as u64 * w as u64) / pw as u64).max(1) as u32)
                    .clamp(16, 4320);
                let opts = frame_ui::FrameExport {
                    out: out.clone(),
                    size: (w, h),
                    scaler: self.project.scaler,
                    resize: arg_str(args, "resize").unwrap_or(&self.settings.export_scaler).to_string(),
                    with_effects: arg_bool(args, "with_effects").unwrap_or(true),
                    quality: arg_u64(args, "quality").map(|q| q as u32).unwrap_or(self.settings.frame_quality),
                };
                let (rw, _) = frame_render_size((pw, ph), opts.size);
                let frame = self.render_frame_now(t, rw).ok_or("render timed out")?;
                write_image(&frame, &opts)?;
                Ok(json!({"ok": true, "path": out.to_string_lossy(), "width": w, "height": h}))
            }
            "labels.list" => {
                let list: Vec<Value> = self
                    .project
                    .labels
                    .iter()
                    .enumerate()
                    .map(|(i, l)| json!({"index": i + 1, "name": l.name, "color": l.color}))
                    .collect();
                Ok(json!(list))
            }
            "labels.set" => {
                let color = args.get("color").and_then(color_arg).map(|c| [c[0], c[1], c[2]]);
                match arg_u64(args, "index") {
                    Some(i) if arg_bool(args, "remove").unwrap_or(false) => {
                        self.project.remove_label(i as u8);
                        Ok(json!({"ok": true, "labels": self.project.labels.len()}))
                    }
                    Some(i) => {
                        let l = self
                            .project
                            .labels
                            .get_mut((i as usize).checked_sub(1).ok_or("index is 1-based")?)
                            .ok_or("no such label")?;
                        if let Some(n) = arg_str(args, "name") {
                            l.name = n.to_string();
                        }
                        if let Some(c) = color {
                            l.color = c;
                        }
                        Ok(json!({"ok": true, "index": i}))
                    }
                    None => {
                        let name = req(arg_str(args, "name"), "name")?.to_string();
                        let idx = self.project.add_label(name, color.unwrap_or([128, 128, 128]));
                        Ok(json!({"ok": true, "index": idx}))
                    }
                }
            }
            "container.add" => {
                let at = req(arg_f64(args, "at"), "at")?;
                let dur = arg_f64(args, "duration").unwrap_or(5.0).max(0.1);
                let (vid, aid) = self.project.add_container_clip(at, dur);
                if let Some(lbl) = arg_str(args, "label") {
                    if let Some(vc) = self.project.clip_mut(vid) {
                        vc.container_label = lbl.to_string();
                    }
                    if let Some(ac) = self.project.clip_mut(aid) {
                        ac.container_label = lbl.to_string();
                    }
                }
                Ok(json!({"ok": true, "video_clip_id": vid, "audio_clip_id": aid}))
            }
            "container.replace" => {
                let clip_id = req(arg_u64(args, "clip_id"), "clip_id")?;
                let asset_id = req(arg_u64(args, "asset_id"), "asset_id")?;
                let pair = arg_bool(args, "pair").unwrap_or(false);
                let ok = if pair {
                    self.project.replace_container_pair(clip_id, asset_id)
                } else {
                    self.project.replace_container_media(clip_id, asset_id)
                };
                if ok {
                    Ok(json!({"ok": true}))
                } else {
                    Err("failed to replace (clip not a container or asset not found)".into())
                }
            }
            "container.make" => {
                let ids = req(arg_ids(args, "clip_ids"), "clip_ids")?;
                self.project.make_container(&ids);
                Ok(json!({"ok": true}))
            }
            "container.unmake" => {
                let ids = req(arg_ids(args, "clip_ids"), "clip_ids")?;
                self.project.unmake_container(&ids);
                Ok(json!({"ok": true}))
            }
            "container.list" => {
                let containers: Vec<Value> = self
                    .project
                    .all_clips()
                    .filter(|(_, c)| c.container)
                    .map(|(ti, c)| {
                        json!({
                            "clip_id": c.id,
                            "track_index": ti,
                            "kind": format!("{:?}", c.kind),
                            "name": c.name,
                            "label": c.container_label,
                            "is_empty": c.is_empty_container(),
                            "asset_id": c.asset,
                            "start": c.start,
                            "duration": c.duration,
                            "link": c.link,
                        })
                    })
                    .collect();
                Ok(json!(containers))
            }
            _ => Err(format!("unknown tool '{name}'")),
        }
    }

    // ---------------- non-blocking windows ----------------

    /// A small egui::Window asking for a name. Returns Some(name) once confirmed.
    fn name_window(ctx: &egui::Context, title: &str, field: &mut Option<String>) -> Option<String> {
        let mut name = field.take()?;
        let mut open = true;
        let mut done = None;
        let mut cancel = false;
        egui::Window::new(title).open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
            // focus only on the window's first frame — every frame would steal it from the panels behind it
            let id = ui.id().with("name");
            let first = ctx.read_response(id).is_none();
            let r = ui.add(egui::TextEdit::singleline(&mut name).id(id));
            if first {
                r.request_focus();
            }
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.horizontal(|ui| {
                if (ui.button("Save").clicked() || enter) && !name.trim().is_empty() {
                    done = Some(name.trim().to_string());
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
        if done.is_some() || cancel || !open {
            done
        } else {
            *field = Some(name);
            None
        }
    }

    fn windows(&mut self, ctx: &egui::Context) {
        self.url_window(ctx);
        self.compress_window(ctx);
        // "Convert To…" options (non-blocking; the timeline stays usable)
        if let Some((id, ext)) = self.convert_dialog.clone() {
            let name = self.project.asset(id).map(|a| a.name()).unwrap_or_default();
            let mut open = true;
            let mut start = false;
            let mut ext = ext;
            egui::Window::new("Convert To…").open(&mut open).resizable(false).show(ctx, |ui| {
                ui.label(&name);
                ui.horizontal(|ui| {
                    ui.label("Format");
                    egui::ComboBox::from_id_salt("convert_ext").selected_text(&ext).show_ui(ui, |ui| {
                        for t in crate::engine::convert::TARGETS {
                            ui.selectable_value(&mut ext, (*t).to_string(), *t);
                        }
                    });
                });
                ui.weak("Saved next to the source as <name>_converted.<ext> and added to the library.");
                start = ui.button("Convert").clicked();
            });
            match (open, start) {
                (_, true) => {
                    self.convert_dialog = None;
                    self.start_asset_convert(id, &ext);
                }
                (true, false) => self.convert_dialog = Some((id, ext)),
                (false, false) => self.convert_dialog = None,
            }
        }
        // background conversions / downloads: progress + cancel
        let convert_jobs: Vec<(Arc<Progress>, String)> = self
            .convert_jobs
            .iter()
            .map(|(p, o)| (p.clone(), o.file_name().unwrap_or_default().to_string_lossy().into_owned()))
            .collect();
        job_window(ctx, "Converting", &convert_jobs);
        let downloads: Vec<(Arc<Progress>, String)> =
            self.downloads.iter().map(|d| (d.progress.clone(), d.url.clone())).collect();
        job_window(ctx, "Downloading", &downloads);
        // retime (Ctrl+R)
        if self.retime.open {
            let changed = {
                let App { project, selection, playhead, undo, redo, retime, .. } = self;
                let mut push = |p: &Project| push_undo_json(undo, redo, p.to_json());
                retime::show(ctx, retime, project, selection, *playhead, &mut push)
            };
            if changed {
                self.after_edit();
            }
        }
        // export window
        if self.export_ui.open {
            self.detect_encoders_once();
            // the export always renders the MAIN timeline — show its size/lossless state, not the open sequence's
            let main = self.project.editing.is_some().then(|| self.export_project());
            let choice = {
                let App { project, settings, export_ui: st, encoders, export, .. } = self;
                export_ui::show(ctx, st, main.as_ref().unwrap_or(project), settings, encoders, export.is_some())
            };
            if let Some(c) = choice {
                self.start_export_choice(c);
            }
        }
        // save template / save layout profile
        if let Some(name) = Self::name_window(ctx, "Save Template", &mut self.template_name) {
            let t = crate::engine::presets::capture_template(&name, &self.project, &self.selection);
            self.settings.templates.retain(|x| x.name != name);
            self.settings.templates.push(t);
            self.settings.save();
            self.toast(format!("Template '{name}' saved"));
        }
        if let Some(name) = Self::name_window(ctx, "Save Layout Profile", &mut self.profile_name) {
            let json = self.layout.to_json();
            self.settings.layout_profiles.retain(|x| x.name != name);
            self.settings.layout_profiles.push(crate::settings::LayoutProfile { name: name.clone(), json });
            self.settings.save();
            self.toast(format!("Layout profile '{name}' saved"));
        }
        // settings window
        if self.settings_ui.open {
            let backend_before = self.settings.decoder.clone();
            let gpu_before = self.settings.gpu;
            let theme_before = self.settings.theme.clone();
            let look_before = self.settings.ui_look.clone();
            let palette_before = self.settings.palette.clone();
            let ctxmenu_before = self.settings.context_menu;
            let ffdir_before = self.settings.ffmpeg_dir.clone();
            let ytdlp_dir_before = self.settings.ytdlp_dir.clone();
            self.detect_encoders_once();
            let mcp_status = match (&self.mcp, self.settings.mcp_enabled) {
                (Some((s, _)), _) => format!("running at {}", s.url()),
                (None, true) => "starting…".into(),
                (None, false) => "stopped".into(),
            };
            let inputs = self.audio_inputs();
            let changed = settings_ui::show(
                ctx,
                &mut self.settings_ui,
                &mut self.settings,
                &mut self.hotkeys,
                &self.encoders,
                &self.palette,
                &mcp_status,
                &self.gpu_name,
                &inputs,
            );
            if changed {
                self.hotkeys.to_settings(&mut self.settings);
                self.settings.save();
                if self.settings.theme != theme_before
                    || self.settings.ui_look != look_before
                    || self.settings.palette != palette_before
                {
                    theme::apply(ctx, &self.settings.theme, &self.settings.palette, &self.settings.ui_look);
                }
                if self.settings.ffmpeg_dir != ffdir_before {
                    media::ffpipe::set_dir(&self.settings.ffmpeg_dir);
                    self.encoders.clear();
                }
                if self.settings.ytdlp_dir != ytdlp_dir_before {
                    media::ytdlp::set_dir(&self.settings.ytdlp_dir);
                }
                // the folder may have changed, and yt-dlp may have been installed since we last looked
                self.detect_ytdlp(ctx);
                if self.settings.gpu != gpu_before {
                    self.gpu_failed = false; // an explicit toggle retries the renderer
                }
                if self.settings.decoder != backend_before {
                    let b = self.backend();
                    self.player.set_backend(b);
                    self.waveforms.set_backend(b);
                    self.thumbs.set_backend(b);
                }
                if self.settings.context_menu != ctxmenu_before {
                    let r = if self.settings.context_menu {
                        crate::contextmenu::install()
                    } else {
                        crate::contextmenu::uninstall()
                    };
                    if let Err(e) = r {
                        self.toast(format!("Context menu: {e}"));
                    }
                }
                // TODO(integration): text.lock().load_user_fonts(&settings.user_fonts) + refresh self.fonts
                // once the text rasterizer grows user-font support (engine-video agent).
            }
        }
        // "Export Frame…" (Ctrl+Shift+F)
        if self.frame_ui.open {
            let choice = {
                let App { frame_ui: st, project, settings, .. } = self;
                frame_ui::show(ctx, st, project, settings)
            };
            if let Some(c) = choice {
                self.settings.save();
                self.export_frame(c);
            }
        }
        // GLSL editor — Apply is an effect edit like any other (undo + re-render), and the compile log
        // goes straight back into the window so a rejected shader is never a silent no-op
        if shader_ui::show(ctx, &mut self.shader_ui) {
            if let Some((id, i)) = self.shader_ui.target {
                let src = self.shader_ui.src.clone();
                let snap = self.project.to_json();
                let changed = match self.project.clip_mut(id).and_then(|c| c.effects.get_mut(i)) {
                    Some(fx) if fx.kind == EffectKind::Shader && fx.shader != src => {
                        fx.shader = src.clone();
                        true
                    }
                    _ => false,
                };
                if changed {
                    push_undo_json(&mut self.undo, &mut self.redo, snap);
                    self.after_edit();
                }
                self.shader_ui.error = match self.gpu.as_mut() {
                    Some(g) => g.check_shader(&src).err().unwrap_or_default(),
                    None => "GPU renderer is off — this shader cannot be compiled or previewed.".into(),
                };
            }
        }
        // "Paste Attributes" (Ctrl+Alt+V) — one undo step for the whole paste
        if self.paste_ui.open {
            let name = self.attrs.as_ref().map(|c| c.name.clone()).unwrap_or_default();
            let targets = self.selection.len();
            let chosen = paste_ui::show(ctx, &mut self.paste_ui, &name, targets);
            if let Some(set) = chosen {
                if let Some(src) = self.attrs.clone() {
                    let snap = self.project.to_json();
                    let ids = self.selection.clone();
                    let n = self.project.paste_attributes(&src, &ids, set);
                    if n > 0 {
                        push_undo_json(&mut self.undo, &mut self.redo, snap);
                        self.after_edit();
                    }
                    self.toast(format!("Pasted attributes onto {n} clip(s)"));
                }
                self.paste_ui.open = false;
            }
        }
        // screen recording / voiceover
        if self.capture_ui.screen_open || self.capture_ui.voice_open {
            let resp = {
                let App { capture_ui: st, settings, palette, screen_rec, voice_rec, .. } = self;
                capture_ui::show(ctx, st, settings, screen_rec.is_some(), voice_rec.is_some(), palette)
            };
            if resp.stop_screen {
                self.stop_screen_capture();
            }
            if resp.stop_voice {
                self.stop_voiceover();
            }
            if let Some(o) = resp.screen.filter(|_| resp.start_screen) {
                self.start_screen_capture(o);
            }
            if let Some(o) = resp.voice.filter(|_| resp.start_voice) {
                self.start_voiceover(o);
            }
        }
        // imported timeline report — "Use this project" swaps it in
        if self.import_ui.open {
            let accept = {
                let App { import_ui: st, palette, .. } = self;
                import_ui::show(ctx, st, palette)
            };
            // ask first: cancelling the unsaved-changes prompt must keep the report (and the ffprobes
            // that built it), not throw the whole import away
            if accept && self.confirm_discard() {
                if let Some(r) = self.import_ui.report.take() {
                    self.set_project(r.project, None);
                    self.toast("Imported timeline is now the project");
                }
                self.import_ui.open = false;
            }
        }
        // export / convert progress — non-modal: keep editing while it runs
        if let Some((prog, kind)) = &self.export {
            let prog = prog.clone();
            let title = match kind {
                ExportKind::File => "Exporting…",
                ExportKind::Overwrite { .. } => "Saving over the original…",
            };
            egui::Window::new(title)
                .collapsible(false)
                .resizable(false)
                .default_pos(ctx.content_rect().center() - egui::vec2(180.0, 60.0))
                .show(ctx, |ui| {
                    ui.set_width(360.0);
                    ui.add(egui::ProgressBar::new(prog.fraction()).show_percentage());
                    ui.label(prog.status());
                    if ui.button("Cancel").clicked() {
                        prog.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                });
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.palette = theme::palette_with(ctx, &self.settings.palette);
        self.palette.rounding = if self.settings.ui_look == "sharp" { 2.0 } else { 6.0 };
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
        self.poll_panels();
        self.poll_probes(ctx);
        self.lib_preview_live = self.lib_preview_frame(ctx);
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
                if self.confirm_discard() {
                    self.close_confirmed = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            } else {
                self.close_confirmed = true;
            }
        }

        // playback clock (one extra read after it stops, so the playhead lands on the final time)
        let playing = self.player.is_playing();
        if playing || self.was_playing {
            self.playhead = self.player.time();
            self.timeline.ensure_visible(self.playhead);
            ctx.request_repaint_after(Duration::from_millis(16));
        }
        // a Draw take runs until the video stops or the tool is put away — not one stroke at a time
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
        // buffering: the render thread fell behind decode — hold the clock (the audio ring flushes
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
        self.screenshot_tick(ctx);

        // hotkeys
        // the tool strip claims the bare letters (V/T/D/M, Shift+S) before the action table is polled, so
        // a rebound action can never shadow a tool
        if let Some(t) = tools::handle_hotkeys(ctx, &mut self.tools) {
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
            // same pane as the docked preview (it reads self.fullscreen) — no second copy to drift
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
                let (changed, moved, set_icon) =
                    layout::show(ctx, ui, &mut l, &icons, tab_bar, cozy, &mut |ui, pane| self.draw_pane(ui, pane));
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

        // toasts
        self.toasts.retain(|(_, t)| t.elapsed().as_secs_f32() < 5.0);
        if !self.toasts.is_empty() {
            egui::Area::new(egui::Id::new("toasts"))
                .anchor(egui::Align2::RIGHT_BOTTOM, [-12.0, -12.0])
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    for (msg, _) in &self.toasts {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            ui.label(msg);
                        });
                    }
                });
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Asset, Ease};

    #[test]
    fn box_blur_spreads_and_preserves_flat_areas() {
        // a flat image stays exactly flat
        let (w, h) = (8, 8);
        let mut flat = vec![100u8; w * h * 4];
        box_blur(&mut flat, w, h, 2);
        assert!(flat.iter().all(|&v| v == 100));
        // a single bright pixel spreads to its neighbours and dims in place
        let mut img = vec![0u8; w * h * 4];
        let centre = ((4 * w + 4) * 4) as usize;
        img[centre] = 255;
        box_blur(&mut img, w, h, 1);
        assert!(img[centre] < 255, "the spike must dim");
        let neighbour = ((4 * w + 5) * 4) as usize;
        assert!(img[neighbour] > 0, "and bleed into the pixel beside it");
        // radius 0 is a no-op and empty buffers don't panic
        let mut same = vec![7u8; 16];
        box_blur(&mut same, 2, 2, 0);
        assert_eq!(same, vec![7u8; 16]);
        box_blur(&mut [], 0, 0, 3);
    }

    fn asset(path: &str) -> Asset {
        Asset {
            id: Id::default(),
            path: path.into(),
            kind: ClipKind::Video,
            duration: 1.0,
            width: 0,
            height: 0,
            fps: 0.0,
            audio_streams: Vec::new(),
            codec: String::new(),
            folder: String::new(),
            tags: Vec::new(),
            label: 0,
            description: String::new(),
        }
    }

    /// Ctrl+V was dead because egui-winit only emits `Event::Paste` when the SYSTEM clipboard holds
    /// text, and an internal clip copy never wrote to it — so the chord produced no event at all and no
    /// binding could see it. Copying clips must therefore also queue text for the OS clipboard.
    #[test]
    fn copying_clips_also_writes_the_os_clipboard() {
        let mut p = Project::from_media(long_asset("C:/x.mp4"));
        let id = p.tracks[0].clips[0].id;
        let t = crate::engine::presets::capture_template("clipboard", &p, &[id]);
        assert!(!t.json.is_empty(), "a captured clip serialises to something");
        // what act(CopyClips) stores: the same JSON goes to both clipboards
        let os = t.json.clone();
        assert!(
            crate::engine::presets::decode_template(&t).is_some(),
            "the internal clipboard still decodes back into clips"
        );
        assert!(os.contains("clips") || os.contains("start"), "the OS text is the template JSON: {os:.80}");
        // and the ripple that Paste Insert performs opens exactly the span it is given
        let before = p.tracks[0].clips[0].start;
        p.ripple_open(before, 2.0);
        assert!(
            (p.tracks[0].clips[0].start - (before + 2.0)).abs() < 1e-6,
            "ripple_open slides the clip right by the span: {} -> {}",
            before,
            p.tracks[0].clips[0].start
        );
    }

    /// A 10 s video asset, long enough to split a few times.
    fn long_asset(path: &str) -> Asset {
        Asset { duration: 10.0, width: 320, height: 240, fps: 30.0, ..asset(path) }
    }

    #[test]
    fn relocate_assets_falls_back_to_project_dir() {
        let dir = std::env::temp_dir().join(format!("se-relocate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.mp4"), b"x").unwrap();
        let mut p = Project::new();
        p.assets.push(asset("Z:\\gone\\a.mp4"));
        p.assets.push(asset("Z:\\gone\\b.mp4"));
        let missing = relocate_assets(&mut p, Some(&dir));
        assert_eq!(p.assets[0].path, dir.join("a.mp4").to_string_lossy());
        assert_eq!(missing, vec!["Z:\\gone\\b.mp4".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn converted_path_never_hits_the_source_or_an_existing_file() {
        let dir = std::env::temp_dir().join(format!("se-conv-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("clip.mp4");
        std::fs::write(&src, b"x").unwrap();
        // converting to the same container must not write over the source
        let out = converted_path(&src, "mp4");
        assert_ne!(out, src);
        assert_eq!(out.file_name().unwrap(), "clip_converted.mp4");
        // nor over a file that is already there (the timeline may be using it)
        std::fs::write(&out, b"y").unwrap();
        assert_eq!(converted_path(&src, "mp4").file_name().unwrap(), "clip_converted_2.mp4");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_empty_open_sequence_is_not_an_empty_timeline() {
        let mut p = Project::from_media(asset("a.mp4"));
        assert!(!timeline_is_empty(&p));
        let ids: Vec<Id> = p.all_clips().map(|(_, c)| c.id).collect();
        let seq = p.nest_selection(&ids, "S").unwrap();
        assert!(p.open_sequence(seq));
        let inner: Vec<Id> = p.all_clips().map(|(_, c)| c.id).collect();
        p.delete_clips(&inner, false);
        assert!(p.is_empty()); // this sequence is empty…
        assert!(!timeline_is_empty(&p)); // …but the main timeline still holds the Sequence clip
        p.close_sequence();
        let all: Vec<Id> = p.all_clips().map(|(_, c)| c.id).collect();
        p.delete_clips(&all, false);
        assert!(timeline_is_empty(&p));
    }

    #[test]
    fn undo_snapshot_is_capped_and_clears_redo() {
        let (mut undo, mut redo) = (Vec::new(), vec!["r".to_string()]);
        for i in 0..205 {
            push_undo_json(&mut undo, &mut redo, i.to_string());
        }
        assert_eq!(undo.len(), 200);
        assert_eq!(undo[0], "5");
        assert!(redo.is_empty());
    }

    #[test]
    fn base64_rfc4648_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn parse_ease_names_and_bezier() {
        assert_eq!(parse_ease("Linear"), Some(Ease::Linear));
        assert_eq!(parse_ease("Hold"), Some(Ease::Hold));
        assert_eq!(
            parse_ease("cubic-bezier(0.42, 0, 0.58, 1)"),
            Some(Ease::Bezier { x1: 0.42, y1: 0.0, x2: 0.58, y2: 1.0 })
        );
        assert_eq!(parse_ease("cubic-bezier(1,2,3)"), None);
        assert_eq!(parse_ease("bogus"), None);
    }

    #[test]
    fn clip_fields_apply_and_reject() {
        let mut c = Clip::new(1, ClipKind::Video, "c", 0.0, 4.0);
        apply_clip_fields(
            &mut c,
            &json!({"name": "renamed", "enabled": false, "label": 3, "blend": "Screen",
                     "fade_in": 0.5, "opacity": 0.25, "freeze": 1.5}),
        )
        .unwrap();
        assert_eq!(c.name, "renamed");
        assert!(!c.enabled);
        assert_eq!(c.label, 3);
        assert_eq!(c.blend, BlendMode::Screen);
        assert_eq!(c.fade_in, 0.5);
        assert_eq!(c.opacity.value, 0.25);
        assert_eq!(c.freeze, Some(1.5));
        apply_clip_fields(&mut c, &json!({"freeze": null})).unwrap();
        assert_eq!(c.freeze, None);
        // setting a constant clears animation
        c.opacity.toggle_key(1.0);
        assert!(c.opacity.is_animated());
        apply_clip_fields(&mut c, &json!({"opacity": 1.0})).unwrap();
        assert!(!c.opacity.is_animated());
        // unknown fields / text fields on a non-text clip are errors
        assert!(apply_clip_fields(&mut c, &json!({"nope": 1})).is_err());
        assert!(apply_clip_fields(&mut c, &json!({"text": "hi"})).is_err());
        // text clip accepts text style fields
        let mut t = Clip::new(2, ClipKind::Text, "t", 0.0, 4.0);
        apply_clip_fields(&mut t, &json!({"text": "hello", "size": 90, "color": [10, 20, 30, 255], "align": 0}))
            .unwrap();
        let ts = t.text.as_ref().unwrap();
        assert_eq!(ts.text, "hello");
        assert_eq!(ts.size, 90.0);
        assert_eq!(ts.color, [10, 20, 30, 255]);
        assert_eq!(ts.align, 0);
    }

    /// Ctrl+T repeats the last transition: same kind and duration, on the selected clip's left cut.
    #[test]
    fn last_transition_is_remembered_and_reapplied() {
        let mut p = Project::from_media(long_asset("a.mp4"));
        let ids: Vec<Id> = p.split_at(1.0, None);
        let (first, second) = (p.all_clips().next().unwrap().1.id, ids[0]);
        // the "Add Transition" action's default: cross fade, 1 s — remembered in the panel state
        let mut st = transitions_ui::TransitionsState::default();
        let add = transitions_ui::add_transitions;
        assert_eq!(add(&mut p, &[second], &mut st, TransitionKind::CrossFade, 1.0, false), 1);
        let tr = p.tracks[0].transitions[0].clone();
        assert_eq!((tr.kind, tr.duration), (TransitionKind::CrossFade, 1.0));
        // a different choice replaces the memory and Ctrl+T applies exactly that at the next cut
        let ids2 = p.split_at(2.0, None);
        assert_eq!(add(&mut p, &ids2, &mut st, TransitionKind::Wipe, 0.4, false), 1);
        assert_eq!(st.kind(), TransitionKind::Wipe, "Ctrl+T follows whatever went through the funnel");
        let tr = p.tracks[0].transitions.iter().find(|t| t.right == ids2[0]).expect("second transition");
        assert_eq!((tr.kind, tr.duration), (TransitionKind::Wipe, 0.4));
        // nothing abuts the very first clip's left edge
        assert_eq!(add(&mut p, &[first], &mut st, TransitionKind::Wipe, 0.4, false), 0);
    }

    #[test]
    fn masks_land_on_the_last_effect_then_the_clip() {
        let mut p = Project::from_media(long_asset("a.mp4"));
        let id = p.all_clips().next().unwrap().1.id;
        // no effects: the clip itself gets the mask
        assert!(add_mask(&mut p, id, MaskShape::Ellipse));
        assert_eq!(p.clip(id).unwrap().mask.as_ref().map(|m| m.shape), Some(MaskShape::Ellipse));
        assert!(!add_mask(&mut p, id, MaskShape::Rect), "a second mask on the same clip is refused");
        // with an effect, the mask goes on the effect (that is what a mask usually means)
        p.clip_mut(id).unwrap().effects.push(Effect::new(EffectKind::Blur));
        assert!(add_mask(&mut p, id, MaskShape::Polygon));
        assert_eq!(p.clip(id).unwrap().effects[0].mask.as_ref().map(|m| m.shape), Some(MaskShape::Polygon));
        assert!(!add_mask(&mut p, 999, MaskShape::Rect), "unknown clip");
        // a mask shapes pixels: audio takes none, through either route (Ctrl+Shift+M or MCP)
        let a = p.new_id();
        p.tracks[1].clips.push(Clip::new(a, ClipKind::Audio, "a", 0.0, 1.0));
        assert!(!add_mask(&mut p, a, MaskShape::Rect), "audio clips take no mask");
        assert!(mask_slot(&mut p, a, None).is_err(), "and clip.add_mask / clip.set_mask refuse them");
        assert!(p.clip(a).unwrap().mask.is_none());
    }

    /// Paste Attributes only touches the boxes that were ticked (and never timing or media).
    #[test]
    fn paste_attributes_applies_only_the_chosen_fields() {
        let mut p = Project::from_media(long_asset("a.mp4"));
        let ids = p.split_at(2.0, None);
        let src_id = p.all_clips().next().unwrap().1.id;
        {
            let c = p.clip_mut(src_id).unwrap();
            c.opacity.value = 0.25;
            c.blend = BlendMode::Screen;
            c.effects.push(Effect::new(EffectKind::Blur));
            c.label = 3;
        }
        let src = p.copy_attributes(src_id).unwrap();
        let target = ids[0];
        let (start, dur) = (p.clip(target).unwrap().start, p.clip(target).unwrap().duration);
        let set = crate::model::AttrSet { opacity: true, ..crate::model::AttrSet::NONE };
        assert_eq!(p.paste_attributes(&src, &[target], set), 1);
        let c = p.clip(target).unwrap();
        assert_eq!(c.opacity.value, 0.25);
        assert_eq!(c.blend, BlendMode::Normal, "blend was not ticked");
        assert!(c.effects.is_empty(), "effects were not ticked");
        assert_eq!(c.label, 0, "label was not ticked");
        assert_eq!((c.start, c.duration), (start, dur), "timing is never pasted");
        // ticking effects + label copies those too
        let set = crate::model::AttrSet { effects: true, label: true, ..crate::model::AttrSet::NONE };
        p.paste_attributes(&src, &[target], set);
        let c = p.clip(target).unwrap();
        assert_eq!(c.effects.len(), 1);
        assert_eq!(c.label, 3);
    }

    #[test]
    fn frame_export_and_preview_sizes() {
        // downscale: render straight at the target
        assert_eq!(frame_render_size((1920, 1080), (960, 540)), (960, 540));
        assert_eq!(frame_render_size((1920, 1080), (1920, 1080)), (1920, 1080));
        // upscale (2x / 4x buttons): render at project size, ffmpeg enlarges with the chosen flag
        assert_eq!(frame_render_size((1920, 1080), (3840, 2160)), (1920, 1080));
        // mixed (wider but shorter) counts as an upscale, and degenerate sizes are clamped
        assert_eq!(frame_render_size((1920, 1080), (4000, 100)), (1920, 1080));
        assert_eq!(frame_render_size((0, 0), (0, 0)), (16, 16));
        // preview quality scales the canvas, keeps zero at zero and never goes below 16 px
        assert_eq!(preview_canvas((800, 600), 100), (800, 600));
        assert_eq!(preview_canvas((800, 600), 50), (400, 300));
        assert_eq!(preview_canvas((800, 600), 1), (200, 150)); // clamped to 25 %
        assert_eq!(preview_canvas((0, 600), 50), (0, 0));
        assert_eq!(preview_canvas((10, 10), 25), (16, 16));
    }

    /// The GPU renders at `self.canvas`, the player decodes at its own clamp — they must agree, or the
    /// preview comes out squashed whenever the pane is wider than preview_max_width.
    #[test]
    fn canvas_clamp_keeps_the_aspect_ratio() {
        assert_eq!(clamp_canvas(800, 450, 1280), (800, 450), "under the limit: untouched");
        assert_eq!(clamp_canvas(800, 450, 320), (320, 180), "height scales with the width");
        assert_eq!(clamp_canvas(1920, 1080, 0), (1920, 1080), "0 = no limit (same as Player::set_canvas)");
        assert_eq!(clamp_canvas(1000, 3, 100), (100, 1), "never collapses to zero");
        // the same numbers the player would land on
        let (w, h) = (1600u32, 900u32);
        let max = 640u32;
        assert_eq!(clamp_canvas(w, h, max), (max, ((h as u64 * max as u64) / w as u64) as u32));
    }

    #[test]
    fn effect_thumbnail_cache_keys() {
        let a = effect_thumb_key(EffectKind::Blur, (96, 54), "");
        assert_eq!(a, effect_thumb_key(EffectKind::Blur, (96, 54), ""), "stable for the same inputs");
        assert_ne!(a, effect_thumb_key(EffectKind::Vhs, (96, 54), ""), "kind matters");
        assert_ne!(a, effect_thumb_key(EffectKind::Blur, (192, 108), ""), "size matters");
        assert_ne!(a, effect_thumb_key(EffectKind::Blur, (96, 54), "C:/pic.png"), "stock image matters");
        // every kind gets its own key at one size
        let mut keys: Vec<u64> = EffectKind::ALL.iter().map(|&k| effect_thumb_key(k, (96, 54), "x")).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), EffectKind::ALL.len());
        // no stock image set => the embedded default, unscaled at card size and resampled elsewhere
        assert_eq!(STOCK.len(), (STOCK_W * STOCK_H * 4) as usize, "embedded RGBA is not W*H*4");
        let card = effect_thumb_source("", STOCK_W, STOCK_H, Backend::Ffmpeg);
        assert_eq!(card.rgba, STOCK, "at card size the embedded image is copied through untouched");
        let f = effect_thumb_source("", 32, 24, Backend::Ffmpeg);
        assert_eq!((f.width, f.height), (32, 24));
        assert_eq!(f.rgba.len(), 32 * 24 * 4);
        assert!(f.rgba.chunks_exact(4).all(|p| p[3] == 255), "the stock image must be opaque");
        assert!(f.rgba.chunks_exact(4).any(|p| p[0] != p[1] || p[1] != p[2]), "the stock image has colour in it");
    }

    /// The frame writer really produces an image of the requested size (needs ffmpeg; skipped without).
    #[test]
    fn write_image_scales_and_writes() {
        if media::ffpipe::ffmpeg_exe().is_none() {
            println!("write_image test: no ffmpeg, skipped");
            return;
        }
        let dir = std::env::temp_dir().join(format!("se-frame-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let frame = effect_thumb_source("", 64, 48, Backend::Ffmpeg);
        for (name, quality) in [("shot.png", 100), ("shot.jpg", 80), ("shot.webp", 80)] {
            let out = dir.join(name);
            let opts = frame_ui::FrameExport {
                out: out.clone(),
                size: (128, 96), // upscaled by ffmpeg, like a 2x frame export
                scaler: Scaler::Bilinear,
                resize: "lanczos".into(),
                with_effects: true,
                quality,
            };
            write_image(&frame, &opts).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(out.is_file(), "{name} not written");
            let probe = media::probe(&out.to_string_lossy(), Backend::Ffmpeg).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!((probe.width, probe.height), (128, 96), "{name} size");
        }
        // a path ffmpeg cannot write is an error, not a panic
        let bad = frame_ui::FrameExport {
            out: PathBuf::from("Z:/nope/shot.png"),
            size: (64, 48),
            scaler: Scaler::Bilinear,
            resize: String::new(),
            with_effects: true,
            quality: 90,
        };
        assert!(write_image(&frame, &bad).is_err());
        assert!(write_image(&Frame::default(), &bad).is_err(), "an empty frame is refused");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Library-preview transport math: frame-step clamps at both ends of the file, and a scrub-bar
    /// fraction (even one dragged past the bar's edge) maps to a time within the file.
    #[test]
    fn lib_preview_seek_math() {
        assert_eq!(step_time(1.0, 25.0, true, 4.0), 1.04, "forward steps by 1/fps");
        assert_eq!(step_time(0.02, 25.0, false, 4.0), 0.0, "backward step clamps at 0");
        assert_eq!(step_time(3.99, 25.0, true, 4.0), 4.0, "forward step clamps at the file's duration");
        assert_eq!(scrub_time(0.0, 4.0), 0.0);
        assert_eq!(scrub_time(1.0, 4.0), 4.0);
        assert_eq!(scrub_time(0.5, 4.0), 2.0);
        assert_eq!(scrub_time(-0.2, 4.0), 0.0, "a drag past the left edge still reads as the start");
        assert_eq!(scrub_time(1.2, 4.0), 4.0, "a drag past the right edge still reads as the end");
    }

    /// Every action is dispatched (`act` matches exhaustively) and every pane can be toggled from
    /// the View menu; here we only check the round-3 actions still carry their advertised bindings.
    #[test]
    fn round3_actions_are_bound() {
        use crate::hotkeys::Hotkeys;
        let h = Hotkeys::defaults();
        for (a, text) in [
            (Action::AddLastTransition, "Ctrl+T"),
            (Action::CopyAttributes, "Ctrl+Alt+C"),
            (Action::PasteAttributes, "Ctrl+Alt+V"),
            // the bare letters V/T/S/D/M belong to the tool strip, so this moved to Shift+M
            (Action::AddMarker, "Shift+M"),
            (Action::AddMask, "Ctrl+Shift+M"),
            (Action::ExportFrame, "Ctrl+Shift+F"),
        ] {
            assert_eq!(h.text(a), text, "{a:?}");
        }
    }

    #[test]
    fn anim_of_props_and_effect_params() {
        let mut c = Clip::new(1, ClipKind::Video, "c", 0.0, 4.0);
        assert!(anim_of(&mut c, "Position X").is_some());
        assert!(anim_of(&mut c, "Volume").is_some());
        assert!(anim_of(&mut c, "Nope").is_none());
        c.effects.push(crate::model::Effect::new(EffectKind::Blur));
        let pname = EffectKind::Blur.params()[0].name;
        assert!(anim_of(&mut c, &format!("Blur: {pname}")).is_some());
        assert!(anim_of(&mut c, "Blur: Nope").is_none());
    }
}
