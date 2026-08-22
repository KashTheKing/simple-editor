//! The application: owns the project, undo stack, settings, player, dockable layout and wires the panels
//! together. Non-blocking windows (Settings, Retime, Export, Save Template, Save Profile, export/convert
//! progress) are plain `egui::Window`s — the editor stays usable while they are open. Also executes MCP
//! tool calls against the live project (one undo step per mutating call).

use crate::engine::export::{self, ExportOptions, Progress};
use crate::engine::text::TextRasterizer;
use crate::hotkeys::{Action, Hotkeys};
use crate::mcp;
use crate::media::thumbs::ThumbCache;
use crate::media::waveform::WaveformCache;
use crate::media::{self, Backend, Frame};
use crate::model::{BlendMode, Clip, ClipKind, EffectKind, Id, Project, Scaler, TrackKind, TransitionKind};
use crate::playback::Player;
use crate::settings::Settings;
use crate::theme::{self, Palette};
use crate::ui::layout::{self, Layout, Pane};
use crate::ui::{
    autocut_ui, curves, effects_ui, export_ui, inspector, library, planner, preview, retime, settings_ui, subtitles_ui,
    timeline, transitions_ui, DragPayload,
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
    autocut: autocut_ui::AutoCutState,
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
    /// Was the Auto-cut pane drawn last frame? (its keep-range shading is only valid while it is open).
    /// `autocut_drawing` accumulates this frame; the timeline reads `autocut_shown` so the shading does
    /// not depend on which pane the tile tree draws first.
    autocut_shown: bool,
    autocut_drawing: bool,
    /// Fonts already handed to the rasterizer (so we only reload when the list grows).
    loaded_fonts: usize,
}

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
];

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, open: Option<PathBuf>, screenshot: Option<PathBuf>) -> Self {
        // eframe restores the window rect from the last session, which may be on another monitor
        crate::winpos::place_on_cursor_monitor(cc);
        let settings = Settings::load();
        media::ffpipe::set_dir(&settings.ffmpeg_dir);
        theme::apply(&cc.egui_ctx, &settings.theme);
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
        let palette = theme::palette(&cc.egui_ctx);
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
            autocut: autocut_ui::AutoCutState::default(),
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
            autocut_shown: false,
            autocut_drawing: false,
            loaded_fonts: 0,
        };
        app.refresh_presets();
        app.player.set_project(&app.project);
        if let Some(p) = open {
            app.open_path(&p);
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
    }

    fn set_project(&mut self, project: Project, path: Option<PathBuf>) {
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

    /// Import media files into the library (probe each). Returns new asset ids.
    fn import_files(&mut self, paths: &[PathBuf]) -> Vec<Id> {
        let mut ids = Vec::new();
        let mut any = false;
        for path in paths {
            let p = path.to_string_lossy().into_owned();
            match media::probe(&p, self.backend()) {
                Ok(asset) => {
                    if !any {
                        self.push_undo();
                        any = true;
                    }
                    let id = self.project.add_asset(asset);
                    ids.push(id);
                    self.settings.touch_recent(&p);
                }
                Err(e) => self.toast(format!("Can't import {}: {e}", path.display())),
            }
        }
        if any {
            self.settings.save();
            if self.project.is_empty() && self.project.assets.len() == ids.len() {
                // first media into an empty project: adopt its format
                if let Some(a) = ids.first().and_then(|id| self.project.asset(*id)).cloned() {
                    if a.has_video() && a.width > 0 {
                        self.project.width = a.width;
                        self.project.height = a.height;
                        if a.fps > 1.0 {
                            self.project.fps = a.fps;
                        }
                    }
                }
            }
            self.after_edit();
        }
        ids
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
                self.toast("XML exported — import it in Premiere (File ▸ Import) or Resolve (File ▸ Import ▸ Timeline)")
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
                    self.redo.push(self.project.to_json());
                    if let Ok(p) = Project::from_json(&json) {
                        self.project = p;
                        self.after_edit();
                    }
                }
            }
            Redo => {
                if let Some(json) = self.redo.pop() {
                    self.undo.push(self.project.to_json());
                    if let Ok(p) = Project::from_json(&json) {
                        self.project = p;
                        self.after_edit();
                    }
                }
            }
            PlayPause => {
                if !self.player.is_playing() && self.playhead >= self.project.duration() - 1e-6 {
                    self.seek(0.0);
                }
                self.player.toggle();
            }
            Stop => self.player.pause(),
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
                if !self.project.split_at(self.playhead, only.as_deref()).is_empty() {
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
            // TODO(round 3): wired by the app agent (tools, nodes, mixer, capture, markers, paste,
            // export frame, timeline import, movie mode, shapes, masks, adjustment layers, last transition)
            AddLastTransition | CopyAttributes | PasteAttributes | AddMarker | ToggleMarkers | ToggleNodes
            | ToggleMixer | ToggleTools | AddShape | AddAdjustment | AddMask | ExportFrame | ScreenCapture
            | Voiceover | ImportTimeline | MovieMode => {}
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
                    let mut added = 0;
                    for id in ids {
                        // a transition always belongs to the clip on the RIGHT of the cut
                        let right = if at_end {
                            crate::ui::transitions_ui::right_neighbor(&self.project, id)
                        } else {
                            Some(id)
                        };
                        if let Some(right) = right {
                            if self.project.add_transition(right, TransitionKind::CrossFade, 1.0).is_some() {
                                added += 1;
                            }
                        }
                    }
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
        }
    }

    fn toggle_pane(&mut self, p: Pane) {
        self.layout.toggle(p);
        self.layout_dirty = true;
    }

    // ---------------- panes ----------------

    fn draw_pane(&mut self, ui: &mut egui::Ui, pane: Pane) {
        match pane {
            Pane::Preview => {
                let frame = self.pending_frame.take();
                let App { project, selection, playhead, undo, redo, preview: pv, player, palette, fullscreen, .. } =
                    self;
                let mut push = |p: &Project| push_undo_json(undo, redo, p.to_json());
                let resp = preview::show(
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
                    },
                );
                self.player.set_canvas(resp.canvas.0, resp.canvas.1, self.settings.preview_max_width);
                if let Some(t) = resp.seek {
                    self.seek(t);
                }
                self.pending_actions.extend(resp.actions);
                if resp.edited {
                    self.after_edit();
                }
            }
            Pane::Timeline => {
                // sequence breadcrumb: a thin strip above the timeline while editing a nested sequence
                if let Some(seq) = self.project.editing {
                    let name = self.project.sequence(seq).map(|s| s.name.clone()).unwrap_or_default();
                    ui.horizontal(|ui| {
                        if ui.button("⬑ Back").on_hover_text("Back to the main timeline (Alt+Up)").clicked() {
                            self.pending_actions.push(Action::OpenParentSequence);
                        }
                        ui.label(format!("Main ▸ {name}"));
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
                        ..
                    } = self;
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
                            self.push_undo();
                            if self.project.insert_sequence_clip(id, t, vt).is_none() {
                                self.undo.pop();
                                self.toast("A sequence can't contain itself");
                            } else {
                                self.after_edit();
                            }
                        }
                        DragPayload::Template(name) => self.place_template(&name, t),
                        _ => {}
                    }
                }
                if let Some(id) = resp.open_sequence {
                    self.enter_sequence(id);
                }
                self.pending_actions.extend(resp.actions);
            }
            Pane::Library => {
                let resp = {
                    let App { project, settings, library: lib, undo, redo, palette, .. } = self;
                    let mut push = |p: &Project| push_undo_json(undo, redo, p.to_json());
                    library::show(ui, lib, project, settings, palette, &mut push)
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
                if !resp.add_to_timeline.is_empty() {
                    self.push_undo();
                    self.insert_at(resp.add_to_timeline, self.playhead, None);
                    self.after_edit();
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
                if let Some(id) = resp.convert_dialog {
                    self.convert_dialog = Some((id, "mp4".into()));
                }
                if let Some(id) = resp.open_sequence {
                    self.enter_sequence(id);
                }
                for name in resp.place_template {
                    self.place_template(&name, self.playhead);
                }
            }
            Pane::Inspector => {
                let changed = {
                    let App { project, selection, playhead, undo, redo, fonts, palette, .. } = self;
                    let mut push = |p: &Project| push_undo_json(undo, redo, p.to_json());
                    let mut changed = false;
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        changed = inspector::show(ui, project, selection, *playhead, fonts, palette, &mut push);
                    });
                    changed
                };
                if changed {
                    self.after_edit();
                }
            }
            Pane::Effects => {
                let changed = {
                    let App { project, selection, playhead, undo, redo, palette, .. } = self;
                    let mut push = |p: &Project| push_undo_json(undo, redo, p.to_json());
                    effects_ui::show(ui, project, selection, *playhead, palette, &mut push)
                };
                if changed {
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
                    let App { project, selection, playhead, undo, redo, curves: st, palette, .. } = self;
                    let mut push = |p: &Project| push_undo_json(undo, redo, p.to_json());
                    curves::show(ui, st, project, selection, playhead, palette, &mut push)
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
                    let App { project, playhead, undo, redo, subtitles_ui: st, fonts, palette, .. } = self;
                    let mut push = |p: &Project| push_undo_json(undo, redo, p.to_json());
                    subtitles_ui::show(ui, st, project, playhead, fonts, palette, &mut push)
                };
                if resp.seeked {
                    self.player.pause();
                    self.player.seek(self.playhead);
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
        };
        self.toast(format!("Converting to {ext}…"));
        self.convert_jobs.push((crate::engine::convert::start_convert(opts), out));
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

    // ---------------- UI pieces ----------------

    fn menu_item(&mut self, ui: &mut egui::Ui, a: Action, enabled: bool, out: &mut Vec<Action>) {
        let text = self.hotkeys.text(a);
        let b = egui::Button::new(a.label()).shortcut_text(text);
        if ui.add_enabled(enabled, b).clicked() {
            out.push(a);
            ui.close();
        }
    }

    fn view_menu(&mut self, ui: &mut egui::Ui, out: &mut Vec<Action>) {
        use Action::*;
        const PANES: [(Pane, Option<Action>); 10] = [
            (Pane::Preview, None),
            (Pane::Timeline, None),
            (Pane::Library, Some(ToggleLibrary)),
            (Pane::Inspector, Some(ToggleInspector)),
            (Pane::Effects, Some(ToggleEffects)),
            (Pane::Transitions, Some(ToggleTransitions)),
            (Pane::Curves, Some(ToggleCurves)),
            (Pane::Subtitles, Some(ToggleSubtitles)),
            (Pane::Planner, Some(TogglePlanner)),
            (Pane::AutoCut, Some(Action::AutoCut)),
        ];
        for (pane, action) in PANES {
            let mut v = self.layout.is_visible(pane);
            let label = match action {
                Some(a) => format!("{}   {}", pane.title(), self.hotkeys.text(a)),
                None => pane.title().to_string(),
            };
            if ui.checkbox(&mut v, label).changed() {
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
                    if ui.button(&p.name).clicked() {
                        ui.close();
                        match Layout::from_json(&p.json) {
                            Some(l) => {
                                self.layout = l;
                                self.layout_dirty = true;
                            }
                            None => self.toast(format!("Profile '{}' is corrupted", p.name)),
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
                    match std::fs::read_to_string(&p).ok().and_then(|s| Layout::from_json(&s)) {
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
                if ui.button("Export Style Summary (.md)…").clicked() {
                    ui.close();
                    self.act_export_style();
                }
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
                self.menu_item(ui, AddText, true, &mut out);
                self.menu_item(ui, AddSubtitle, true, &mut out);
                self.menu_item(ui, AddTransition, has_sel, &mut out);
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
                    ui.label("▶");
                }
                if let Some(seq) = self.project.editing {
                    let name = self.project.sequence(seq).map(|s| s.name.as_str()).unwrap_or("?").to_string();
                    ui.label(egui::RichText::new(format!("editing: Main ▸ {name}")).weak());
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
        // shoot as soon as the first rendered frame is on screen (or after 2.5 s when nothing renders)
        if !self.screenshot_requested && (self.first_frame_at.is_some() || self.started.elapsed().as_secs_f32() > 2.5) {
            self.screenshot_requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
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
                let frame = self.player.render_once(t, w).ok_or("render timed out")?;
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
        // background conversions: progress + cancel
        if !self.convert_jobs.is_empty() {
            let jobs: Vec<(Arc<Progress>, String)> = self
                .convert_jobs
                .iter()
                .map(|(p, o)| (p.clone(), o.file_name().unwrap_or_default().to_string_lossy().into_owned()))
                .collect();
            egui::Window::new("Converting").resizable(false).default_width(300.0).show(ctx, |ui| {
                for (prog, name) in jobs {
                    ui.label(name);
                    ui.add(egui::ProgressBar::new(prog.fraction()).show_percentage());
                    if ui.button("Cancel").clicked() {
                        prog.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                }
            });
            ctx.request_repaint_after(Duration::from_millis(150));
        }
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
            let theme_before = self.settings.theme.clone();
            let ctxmenu_before = self.settings.context_menu;
            let ffdir_before = self.settings.ffmpeg_dir.clone();
            self.detect_encoders_once();
            let mcp_status = match (&self.mcp, self.settings.mcp_enabled) {
                (Some((s, _)), _) => format!("running at {}", s.url()),
                (None, true) => "starting…".into(),
                (None, false) => "stopped".into(),
            };
            let changed = settings_ui::show(
                ctx,
                &mut self.settings_ui,
                &mut self.settings,
                &mut self.hotkeys,
                &self.encoders,
                &self.palette,
                &mcp_status,
            );
            if changed {
                self.hotkeys.to_settings(&mut self.settings);
                self.settings.save();
                if self.settings.theme != theme_before {
                    theme::apply(ctx, &self.settings.theme);
                }
                if self.settings.ffmpeg_dir != ffdir_before {
                    media::ffpipe::set_dir(&self.settings.ffmpeg_dir);
                    self.encoders.clear();
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
        self.palette = theme::palette(ctx);
        if self.fonts.is_empty() {
            if let Ok(t) = self.text.try_lock() {
                if t.is_loaded() {
                    self.fonts = t.families().to_vec();
                }
            }
        }
        self.poll_panels();
        // carry last frame's "the Auto-cut pane was on screen" into this frame's timeline drawing
        self.autocut_shown = self.autocut_drawing;
        self.autocut_drawing = false;

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
        self.was_playing = playing;
        if let Some(f) = self.player.take_frame() {
            if self.first_frame_at.is_none() {
                self.first_frame_at = Some(Instant::now());
                #[cfg(debug_assertions)]
                eprintln!("first frame after {} ms", self.started.elapsed().as_millis());
            }
            self.pending_frame = Some(f);
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

        // MCP server + queued tool calls (executed here, on the UI thread, against the live project)
        self.sync_mcp(ctx);
        self.poll_mcp(ctx);

        self.handle_drops(ctx);
        self.screenshot_tick(ctx);

        // hotkeys
        let mut actions = self.hotkeys.poll(ctx);
        if !ctx.wants_keyboard_input() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::Y)) {
            actions.push(Action::Redo);
        }
        if !ctx.wants_keyboard_input() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace))
        {
            actions.push(Action::Delete);
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
            egui::CentralPanel::default().show(ctx, |ui| {
                let mut l = std::mem::replace(
                    &mut self.layout,
                    Layout { tree: egui_tiles::Tree::empty("layout"), popped: Vec::new() },
                );
                let changed = layout::show(ctx, ui, &mut l, &mut |ui, pane| self.draw_pane(ui, pane));
                self.layout = l;
                self.layout_dirty |= changed;
            });
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
