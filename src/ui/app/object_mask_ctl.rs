//! ---- ws:addon-sam2 ----
//! The App side of object masks (`engine::sam2`, the optional SAM 2 add-on).
//!
//! **Object Mask** (Inspector ▸ Mask, the clip menu, the palette, `mask.object`) puts the Preview in
//! click mode for one clip: left-click adds a point on the object, Shift+left-click a point that is
//! not the object, right-click removes the nearest point. Every change re-masks the frame under the
//! playhead on a background thread and the result is tinted over the Preview. **Track** (`mask.track`)
//! sets the clip's mask to an Object mask on a fresh cache folder (one undo step) and propagates it
//! through the source range the clip shows as a job (Jobs pane, cancel). The mask renders from that
//! cache as frames land; until a frame is there, the last good one stands in.
//!
//! The click session lives in a static (`SESSION`), not on `App`: the add-on stays out of App's struct.

use super::feedback::{Toast, ToastKind};
use super::tools_helpers::*;
use super::*;
use crate::engine::sam2::{self, Point};
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::model::{Mask, MaskShape};

/// One click session.
struct Session {
    clip: Id,
    /// The source frame the points are on, and the timeline time it was clicked at.
    src_t: f64,
    at: f64,
    points: Vec<Point>,
    /// The latest mask of those points, and the GPU-side copy the overlay paints.
    preview: Option<Arc<sam2::Matte>>,
    tex: Option<egui::TextureHandle>,
    /// Points the preview shows (to know when it is stale).
    shown: Vec<Point>,
    /// The mask job in flight and what it will report.
    job: Option<(Arc<Progress>, Vec<Point>)>,
    result: Arc<Mutex<Option<Result<(sam2::Matte, Value), String>>>>,
    /// Last helper reply ("iou", "ms") or error, for the status line.
    note: String,
    /// The helper warm-up (the model load) started on entering click mode; None once it is not needed.
    warm: Option<Arc<Progress>>,
}

impl Session {
    /// Something will land without input (a mask job, the model loading), so the tick keeps waking.
    /// Idle click mode (points shown, nothing running, or a warm-up that failed) asks for no frames.
    fn busy(&self) -> bool {
        self.job.is_some() || self.warm.as_ref().is_some_and(|w| !w.is_done())
    }
}

/// The frame hook wants another frame soon: a session or a track still has work in flight.
fn wants_frames(session: Option<&Session>, tracking: bool) -> bool {
    tracking || session.is_some_and(Session::busy)
}

/// One track in flight.
struct Track {
    clip: Id,
    prog: Arc<Progress>,
    /// What an MCP caller waits on (ends with the toast).
    done: Arc<Progress>,
    req: sam2::TrackReq,
    reply: Arc<Mutex<Option<Value>>>,
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);
static TRACKS: Mutex<Vec<Track>> = Mutex::new(Vec::new());

fn session() -> std::sync::MutexGuard<'static, Option<Session>> {
    SESSION.lock().unwrap_or_else(|e| e.into_inner())
}

fn tracks() -> std::sync::MutexGuard<'static, Vec<Track>> {
    TRACKS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Rows for the Jobs pane.
pub(super) fn progress_jobs() -> Vec<jobs_pane::ProgJob> {
    let mut v: Vec<jobs_pane::ProgJob> = tracks()
        .iter()
        .map(|t| (crate::ui::jobs_ui::JobKind::Bake, format!("Object mask · clip {}", t.clip), t.prog.clone()))
        .collect();
    if let Some(p) = sam2::installing() {
        v.push((crate::ui::jobs_ui::JobKind::ModelDownload, "Install · Object masks".into(), p));
    }
    v
}

/// Why `clip` cannot get an object mask, if it cannot.
fn refusal(project: &Project, clip: Id) -> Option<String> {
    let Some(c) = project.clip(clip) else { return Some(format!("no clip {clip}")) };
    if !matches!(c.kind, ClipKind::Video | ClipKind::Image) {
        return Some("Object Mask works on video and picture clips".into());
    }
    let Some(a) = project.asset(c.asset) else { return Some("the clip has no media".into()) };
    if a.html.is_some() || !a.has_video() {
        return Some("Object Mask works on video and picture files".into());
    }
    if !Path::new(&a.path).is_file() {
        return Some(format!("{} is offline", a.path));
    }
    None
}

/// The clip in click mode, if any.
pub(crate) fn active_clip() -> Option<Id> {
    session().as_ref().map(|s| s.clip)
}

/// Offer the install when a click needs the add-on.
pub(crate) fn offer_install() {
    crate::ui::confirm::ask_app(
        "Install object masks?",
        "Object Mask uses SAM 2 (Meta's Segment Anything 2), an optional add-on: about 400 MB, downloaded once, \
         runs on the graphics card through DirectML or on the processor. Download and install it now? \
         Nothing else is sent anywhere. Uninstall any time in Settings ▸ Add-ons.",
        |app| {
            sam2::start_install();
            app.toast("Installing object masks - see Jobs; click Object Mask again when it is done");
        },
    );
}

/// Enter click mode on `clip` (the playhead moves onto it if it is elsewhere). The helper starts
/// loading at once, so the first click does not wait for the model.
pub(super) fn enter(app: &mut App, clip: Id) -> Result<(), String> {
    if !sam2::available() {
        return Err("The object-mask add-on is not installed (Settings ▸ Add-ons ▸ Object masks, or mask.install)".into());
    }
    if let Some(why) = refusal(&app.project, clip) {
        return Err(why);
    }
    let c = app.project.clip(clip).ok_or("no such clip")?;
    if !c.contains(app.playhead) {
        let t = c.start + c.duration / 2.0;
        app.seek(t);
    }
    let c = app.project.clip(clip).ok_or("no such clip")?;
    let at = app.playhead;
    let src_t = c.src_time(at);
    *session() = Some(Session {
        clip,
        src_t,
        at,
        points: Vec::new(),
        preview: None,
        tex: None,
        shown: Vec::new(),
        job: None,
        result: Arc::new(Mutex::new(None)),
        note: String::new(),
        warm: None,
    });
    // warm the helper up: the model load is the slow part (seconds; DirectML compiles on first use).
    // A failure shows on the first click (segment starts the helper again and reports why).
    if sam2::device().is_none() {
        let warm = export::spawn_job("sam2-warm", |p| sam2::warm(&|| p.is_cancelled()));
        if let Some(s) = session().as_mut() {
            s.warm = Some(warm);
        }
    }
    if !app.selection.contains(&clip) {
        app.selection = vec![clip];
    }
    Ok(())
}

pub(super) fn exit() {
    *session() = None;
}

/// Points changed: start a mask job unless one runs (the tick starts the next one when it lands).
fn kick(app: &App, s: &mut Session) {
    if s.job.is_some() || s.points == s.shown {
        return;
    }
    let Some(c) = app.project.clip(s.clip) else { return };
    let Some(a) = app.project.asset(c.asset) else { return };
    if !s.points.iter().any(|p| p.add) {
        // nothing on the object: no mask
        s.preview = None;
        s.tex = None;
        s.shown = s.points.clone();
        return;
    }
    let (video, t, pts, size) = (a.path.clone(), s.src_t, s.points.clone(), sam2::matte_size(a.width, a.height));
    let slot = s.result.clone();
    let prog = export::spawn_job("sam2-mask", move |p| {
        let r = sam2::segment(&video, t, &pts, size, &|| p.is_cancelled());
        let e = r.as_ref().err().cloned();
        *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
        e.map_or(Ok(()), Err)
    });
    s.job = Some((prog, s.points.clone()));
}

/// A click at (u, v) in 0..1 of the source frame: add (or a "not this" point), or remove the nearest.
pub(super) fn click(app: &mut App, u: f32, v: f32, add: bool, remove: bool) {
    let mut g = session();
    let Some(s) = g.as_mut() else { return };
    // the points belong to the frame they were put on: a click on another frame starts over there
    if let Some(c) = app.project.clip(s.clip) {
        let st = c.src_time(app.playhead);
        if (st - s.src_t).abs() > 1e-3 {
            s.src_t = st;
            s.at = app.playhead;
            s.points.clear();
            s.shown.clear();
            s.preview = None;
            s.tex = None;
        }
    }
    if remove {
        let near = s
            .points
            .iter()
            .enumerate()
            .min_by(|a, b| {
                let d = |p: &Point| (p.x - u).powi(2) + (p.y - v).powi(2);
                d(a.1).total_cmp(&d(b.1))
            })
            .map(|(i, _)| i);
        if let Some(i) = near {
            s.points.remove(i);
        }
    } else {
        s.points.push(Point { x: u.clamp(0.0, 1.0), y: v.clamp(0.0, 1.0), add });
    }
    kick(app, s);
}

/// Start tracking `clip` from the session's points (or `points` on the frame at timeline time `at`).
/// Sets the clip's mask to an Object mask on the new cache folder first (one undo step), so frames
/// show as they land. Returns (what an MCP caller waits on, the folder).
pub(super) fn start_track(
    app: &mut App,
    clip: Id,
    given: Option<(Vec<Point>, f64)>,
    effect: Option<usize>,
) -> Result<(Arc<Progress>, PathBuf), String> {
    if !sam2::available() {
        return Err("The object-mask add-on is not installed (Settings ▸ Add-ons ▸ Object masks, or mask.install)".into());
    }
    if let Some(why) = refusal(&app.project, clip) {
        return Err(why);
    }
    if tracks().iter().any(|t| t.clip == clip) {
        return Err("That clip is already being tracked".into());
    }
    let (points, at) = match given {
        Some(g) => g,
        None => {
            let g = session();
            let s = g.as_ref().filter(|s| s.clip == clip).ok_or("Click the object first (Object Mask)")?;
            (s.points.clone(), s.at)
        }
    };
    if !points.iter().any(|p| p.add) {
        return Err("Add a point on the object first".into());
    }
    let c = app.project.clip(clip).ok_or("no such clip")?;
    let a = app.project.asset(c.asset).ok_or("no media")?;
    let fps = if a.fps > 0.0 { a.fps } else { app.project.fps };
    // the source range the clip shows (a still shows one frame)
    let (lo, hi) = if a.kind == ClipKind::Image { (0.0, 1.0 / fps) } else { crate::engine::enhance::source_range(c, a.duration, fps, 0.0) };
    let count = (((hi - lo) * fps).round() as usize).max(1);
    let src_t = c.src_time(at.clamp(c.start, c.start + c.duration));
    let seed = (((src_t - lo) * fps + 1e-6).floor().max(0.0) as usize).min(count - 1);
    let mut req = sam2::TrackReq {
        video: a.path.clone(),
        start: lo,
        fps,
        count,
        seed,
        points,
        dir: PathBuf::new(),
        size: sam2::matte_size(a.width, a.height),
    };
    req.dir = sam2::matte_dir(&req);
    // the clip's (or the effect's) mask becomes the object's, keeping invert / opacity of an object mask
    // it already had
    let before = app.project.to_json();
    let slot = mask_slot(&mut app.project, clip, effect)?;
    let mut m = match slot.take() {
        Some(m) if m.shape == MaskShape::Object => m,
        _ => Mask::new(MaskShape::Object),
    };
    m.matte = req.dir.to_string_lossy().into_owned();
    m.enabled = true;
    *slot = Some(m);
    app.push_undo_labeled(before, "Object mask");
    app.after_edit();
    let r = req.clone();
    let reply = Arc::new(Mutex::new(None));
    let slot = reply.clone();
    let prog = export::spawn_job("sam2-track", move |p| {
        let v = sam2::track(&r, p)?;
        *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(v);
        Ok(())
    });
    let done = Progress::new();
    done.set(0.0, "Object mask");
    tracks().push(Track { clip, prog, done: done.clone(), req: req.clone(), reply });
    Ok((done, req.dir))
}

/// FRAME_HOOKS: land mask jobs (and start the next if the points moved on), finish tracks.
pub(super) fn tick(app: &mut App, ctx: &egui::Context) {
    let mut busy;
    {
        let mut g = session();
        if let Some(s) = g.as_mut() {
            if app.project.clip(s.clip).is_none() {
                *g = None;
            } else {
                if let Some(w) = s.warm.take_if(|w| w.is_done()) {
                    if let Some(e) = w.error().filter(|_| !w.is_cancelled()) {
                        s.note = format!("The object-mask helper did not start: {e}");
                    }
                }
                if let Some((p, pts)) = s.job.clone() {
                    if p.is_done() {
                        s.job = None;
                        match s.result.lock().unwrap_or_else(|e| e.into_inner()).take() {
                            Some(Ok((m, reply))) => {
                                s.note = format!(
                                    "{} point{} · {:.0} ms",
                                    pts.len(),
                                    if pts.len() == 1 { "" } else { "s" },
                                    reply["ms"].as_f64().unwrap_or(0.0)
                                );
                                let img = tint(&m);
                                s.tex = Some(ctx.load_texture("object-mask-preview", img, egui::TextureOptions::LINEAR));
                                s.preview = Some(Arc::new(m));
                                s.shown = pts;
                            }
                            Some(Err(e)) => {
                                s.note = e;
                                s.shown = pts;
                            }
                            None => s.shown = pts,
                        }
                        kick(app, s);
                    }
                }
            }
        }
        busy = wants_frames(g.as_ref(), false);
    }
    let finished: Vec<Track> = {
        let mut list = tracks();
        for t in list.iter() {
            t.done.set(t.prog.fraction(), t.prog.status());
        }
        busy |= wants_frames(None, !list.is_empty());
        let (done, pending): (Vec<Track>, Vec<Track>) = std::mem::take(&mut *list).into_iter().partition(|t| t.prog.is_done());
        *list = pending;
        done
    };
    for t in finished {
        let r = match t.prog.error() {
            Some(_) if t.prog.is_cancelled() => Err("Object mask tracking cancelled (the frames so far stay)".to_string()),
            Some(e) => Err(format!("Object mask tracking failed: {e}")),
            None => {
                let v = t.reply.lock().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default();
                let ms = v["per_frame_ms"]["total"].as_f64().unwrap_or(0.0);
                Ok(format!(
                    "Object mask: {} frames tracked ({:.2} s per frame on the {})",
                    t.req.count,
                    ms / 1000.0,
                    if v["device"] == "gpu" { "graphics card" } else { "processor" }
                ))
            }
        };
        match r {
            Ok(msg) => {
                app.push_toast(Toast::new(msg).kind(ToastKind::Success));
                t.done.set(1.0, "Done");
                t.done.finish(None);
            }
            Err(e) => {
                app.push_toast(Toast::new(e.clone()).kind(ToastKind::Error));
                t.done.finish(Some(e));
            }
        }
        app.after_edit(); // the cached frames changed under the clip: re-render
    }
    if busy {
        ctx.request_repaint_after(Duration::from_millis(150));
    }
}

/// The overlay picture: the accent-coloured mask at half strength (alpha = coverage).
fn tint(m: &sam2::Matte) -> egui::ColorImage {
    let rgba: Vec<u8> = m.px.iter().flat_map(|&k| [40, 200, 255, (k as u16 * 110 / 255) as u8]).collect();
    egui::ColorImage::from_rgba_unmultiplied([m.w as usize, m.h as usize], &rgba)
}

/// The clip's layer corners on screen (tl, tr, br, bl) in the Preview, and the screen → frame mapping.
fn layer_on_screen(app: &App, clip: Id, ppp: f32) -> Option<([egui::Pos2; 4], impl Fn(egui::Pos2) -> (f32, f32))> {
    let c = app.project.clip(clip)?;
    let a = app.project.asset(c.asset)?;
    let aspect = app.project.width.max(1) as f32 / app.project.height.max(1) as f32;
    let lb = crate::ui::preview::apply_view(crate::ui::preview::letterbox(app.preview.canvas_rect, aspect, ppp), app.preview.view);
    if !lb.is_positive() {
        return None;
    }
    let (cw, ch) = ((lb.width() * ppp).max(1.0) as u32, (lb.height() * ppp).max(1.0) as u32);
    let p = crate::engine::compose::placement(&app.project, c, app.playhead, (a.width, a.height), cw, ch, true);
    let k = lb.width() / cw as f32; // screen points per canvas px
    let (sin, cos) = p.rot.to_radians().sin_cos();
    let at = |u: f32, v: f32| {
        let (dx, dy) = ((u - 0.5) * p.w, (v - 0.5) * p.h);
        // layer → canvas: the clockwise rotation compose::draw_layer undoes
        let (x, y) = (p.cx + dx * cos - dy * sin, p.cy + dx * sin + dy * cos);
        egui::pos2(lb.min.x + x * k, lb.min.y + y * k)
    };
    let corners = [at(0.0, 0.0), at(1.0, 0.0), at(1.0, 1.0), at(0.0, 1.0)];
    let inv = move |s: egui::Pos2| {
        let (dx, dy) = ((s.x - lb.min.x) / k - p.cx, (s.y - lb.min.y) / k - p.cy);
        ((dx * cos + dy * sin) / p.w + 0.5, (-dx * sin + dy * cos) / p.h + 0.5)
    };
    Some((corners, inv))
}

/// Drawn over the Preview after it (preview_pane): the live mask, the points, the strip of controls,
/// and the clicks (an interact rect on top of the video takes them before the clip can be moved).
pub(super) fn overlay(app: &mut App, ui: &mut egui::Ui) {
    let Some(clip) = active_clip() else { return };
    let ppp = ui.pixels_per_point();
    let Some((corners, inv)) = layer_on_screen(app, clip, ppp) else { return };
    let rect = app.preview.canvas_rect;
    let painter = ui.painter_at(rect);
    let pal = app.palette.clone();
    let on_frame = {
        let g = session();
        let Some(s) = g.as_ref() else { return };
        let here = app.project.clip(clip).map(|c| c.src_time(app.playhead)).unwrap_or(-1.0);
        (here - s.src_t).abs() <= 1e-3
    };
    // the live mask (only on the frame the points are on)
    let (tex, points, note, busy, count) = {
        let g = session();
        let s = g.as_ref().expect("checked above");
        (s.tex.clone(), s.points.clone(), s.note.clone(), s.busy(), s.points.len())
    };
    if on_frame {
        if let Some(t) = &tex {
            let mut mesh = egui::Mesh::with_texture(t.id());
            let uv = [egui::pos2(0.0, 0.0), egui::pos2(1.0, 0.0), egui::pos2(1.0, 1.0), egui::pos2(0.0, 1.0)];
            for i in 0..4 {
                mesh.vertices.push(egui::epaint::Vertex { pos: corners[i], uv: uv[i], color: egui::Color32::WHITE });
            }
            mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
            painter.add(egui::Shape::mesh(mesh));
        }
        let to_screen = |u: f32, v: f32| {
            let top = corners[0] + (corners[1] - corners[0]) * u;
            let bot = corners[3] + (corners[2] - corners[3]) * u;
            top + (bot - top) * v
        };
        for p in &points {
            let c = to_screen(p.x, p.y);
            let fill = if p.add { egui::Color32::from_rgb(60, 220, 90) } else { egui::Color32::from_rgb(235, 70, 60) };
            painter.circle(c, 5.0, fill, egui::Stroke::new(1.5_f32, egui::Color32::WHITE));
        }
    }
    // the outline of the clip being masked
    painter.add(egui::Shape::closed_line(corners.to_vec(), egui::Stroke::new(1.0_f32, pal.accent)));

    // the clicks: on top of the video, so the clip underneath is not moved or selected
    let hit = ui.interact(rect, egui::Id::new("object-mask-clicks"), egui::Sense::click_and_drag());
    let shift = ui.input(|i| i.modifiers.shift);
    if let Some(pos) = hit.interact_pointer_pos() {
        let (u, v) = inv(pos);
        let inside = (0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v);
        if hit.clicked() && inside {
            click(app, u, v, !shift, false);
        } else if hit.secondary_clicked() {
            click(app, u, v, true, true);
        }
    }
    if hit.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    }

    // the strip: what it is doing, Track, Clear, Done
    let tracking = tracks().iter().find(|t| t.clip == clip).map(|t| t.prog.clone());
    let mut act: Option<&str> = None;
    egui::Area::new(egui::Id::new("object-mask-strip"))
        .order(egui::Order::Foreground)
        .fixed_pos(rect.min + egui::vec2(8.0, 8.0))
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.strong("Object Mask");
                    let status = match (&tracking, busy) {
                        (Some(p), _) => format!("{:.0}% · {}", p.fraction() * 100.0, p.status()),
                        (None, true) if sam2::device().is_none() => {
                            let n = sam2::helper_note();
                            if n.is_empty() { "Starting…".to_string() } else { n }
                        }
                        (None, true) => "Masking…".to_string(),
                        (None, false) if count == 0 => "Click the object".to_string(),
                        (None, false) if !on_frame => "Points are on another frame".to_string(),
                        (None, false) => note.clone(),
                    };
                    if busy || tracking.is_some() {
                        ui.add(egui::Spinner::new().size(12.0));
                    }
                    ui.weak(status).on_hover_text(
                        "Left-click adds a point on the object, Shift+click marks what is not the object, \
                         right-click removes the nearest point. Esc or Done leaves.",
                    );
                    if let Some(d) = sam2::device() {
                        ui.weak(if d == "gpu" { "GPU" } else { "CPU" })
                            .on_hover_text("Where SAM 2 runs: the graphics card through DirectML, or the processor");
                    }
                    let can_track = tracking.is_none() && points.iter().any(|p| p.add);
                    if ui
                        .add_enabled(can_track, egui::Button::new("Track"))
                        .on_hover_text("Follow the object's outline through the clip (a job; cancel it in Jobs)")
                        .clicked()
                    {
                        act = Some("track");
                    }
                    if ui.add_enabled(count > 0, egui::Button::new("Clear")).on_hover_text("Remove the points").clicked() {
                        act = Some("clear");
                    }
                    if ui.button("Done").on_hover_text("Leave Object Mask (Esc)").clicked() {
                        act = Some("done");
                    }
                });
            });
        });
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        act = Some("done");
    }
    match act {
        Some("track") => match start_track(app, clip, None, None) {
            Ok(_) => app.toast("Tracking the object - see Jobs"),
            Err(e) => app.toast(e),
        },
        Some("clear") => {
            if let Some(s) = session().as_mut() {
                s.points.clear();
                s.shown.clear();
                s.preview = None;
                s.tex = None;
            }
        }
        Some("done") => exit(),
        _ => {}
    }
}

/// ACT_HANDLERS: Object Mask enters click mode on the first selected clip that can take it; Object
/// Mask: Track tracks the session's points.
pub(super) fn act(app: &mut App, a: Action) -> bool {
    if !matches!(a, Action::ObjectMask | Action::ObjectMaskTrack) {
        return false;
    }
    if !sam2::available() {
        offer_install();
        return true;
    }
    if a == Action::ObjectMaskTrack {
        let clip = active_clip().or_else(|| app.selection.first().copied());
        match clip.ok_or("Select a clip".to_string()).and_then(|c| start_track(app, c, None, None)) {
            Ok(_) => app.toast("Tracking the object - see Jobs"),
            Err(e) => app.toast(e),
        }
        return true;
    }
    let clip = app.selection.iter().copied().find(|&id| refusal(&app.project, id).is_none());
    let Some(clip) = clip else {
        app.toast("Select a video or picture clip, then Object Mask");
        return true;
    };
    if active_clip() == Some(clip) {
        exit(); // the button toggles
        return true;
    }
    if let Err(e) = enter(app, clip) {
        app.toast(e);
    }
    true
}

fn status_json() -> Value {
    let list: Vec<Value> = tracks()
        .iter()
        .map(|t| json!({"clip": t.clip, "dir": t.req.dir, "frames": t.req.count, "fraction": t.prog.fraction(), "status": t.prog.status()}))
        .collect();
    let s = session();
    json!({
        "installed": sam2::available(),
        "path": sam2::exe().map(|p| p.display().to_string()),
        "version": sam2::version(),
        "device": sam2::device(),
        "installing": sam2::installing().map(|p| json!({"fraction": p.fraction(), "status": p.status()})),
        "click_mode": s.as_ref().map(|s| json!({"clip": s.clip, "points": sam2::points_json(&s.points)})),
        "tracks": list,
    })
}

/// A frame mask for MCP: the same work as a click, waited on as a job; the PNG lands at the returned path.
fn object_job(app: &mut App, clip: Id, points: Vec<Point>, at: Option<f64>) -> Result<(Arc<Progress>, PathBuf), String> {
    if !sam2::available() {
        return Err("The object-mask add-on is not installed (mask.install)".into());
    }
    if let Some(why) = refusal(&app.project, clip) {
        return Err(why);
    }
    if !points.iter().any(|p| p.add) {
        return Err("points: at least one with label 1 (on the object)".into());
    }
    let c = app.project.clip(clip).ok_or("no such clip")?;
    let at = at.unwrap_or(app.playhead).clamp(c.start, c.start + c.duration);
    let a = app.project.asset(c.asset).ok_or("no media")?;
    let (video, t, size) = (a.path.clone(), c.src_time(at), sam2::matte_size(a.width, a.height));
    let out = crate::settings::Settings::cache_dir().join("masks").join(format!("frame-{clip}.png"));
    // the UI shows the same points (click mode on that clip and frame)
    if active_clip() != Some(clip) {
        let _ = enter(app, clip);
    }
    if let Some(s) = session().as_mut() {
        (s.src_t, s.at, s.points, s.shown) = (t, at, points.clone(), points.clone());
    }
    let (slot, dest) = (session().as_ref().map(|s| s.result.clone()), out.clone());
    let prog = export::spawn_job("sam2-mask", move |p| {
        let r = sam2::segment(&video, t, &points, size, &|| p.is_cancelled());
        if let Ok((m, _)) = &r {
            sam2::write_png(&dest, m)?;
        }
        let e = r.as_ref().err().cloned();
        if let Some(slot) = slot {
            *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
        }
        e.map_or(Ok(()), Err)
    });
    if let Some(s) = session().as_mut() {
        s.job = Some((prog.clone(), s.points.clone()));
    }
    Ok((prog, out))
}

pub(crate) const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "mask.object",
        desc: "Mask an object on one frame of a clip (job): SAM 2 from click points, the same as Object Mask in the Preview (which shows the points and the mask). points = [[x, y, label], …] with x, y in 0..1 of the clip's source frame, label 1 = on the object, 0 = not the object. The 8-bit greyscale mask PNG (source aspect, long side ≤ 1024) lands at the returned path. Needs the object-mask add-on (mask.install).",
        args: &[
            "clip:integer:true:",
            "points:array:true:[[x, y, label], …] - x, y 0..1 of the frame, label 1 add / 0 remove",
            "at:number:false:timeline seconds of the frame (default the playhead)",
        ],
        kind: ToolKind::Job,
        run: |app, args| {
            let clip = req(arg_u64(args, "clip"), "clip")?;
            let points = sam2::parse_points(&args["points"])?;
            let (prog, out) = object_job(app, clip, points, arg_f64(args, "at"))?;
            Ok(ToolOutcome::Job(prog, out))
        },
    },
    ToolDef {
        name: "mask.track",
        desc: "Object mask through a clip (job): sets the clip's mask (or with effect, that effect's mask) to an Object mask (one undo step) and propagates SAM 2 from the prompted frame both ways over the source range the clip shows, into a cached mask sequence the mask renders from (invert it with clip.set_mask {invert:true}; effects take it through their own mask). points / at as for mask.object; omitted = the points of Object Mask's click mode on that clip. action=status reports the add-on, the device, click mode and running tracks instead.",
        args: &[
            "clip:integer:false:required unless action=status",
            "points:array:false:[[x, y, label], …] (default: the click-mode points)",
            "at:number:false:timeline seconds of the prompted frame (default the playhead)",
            "effect:integer:false:effect index: track into that effect's mask instead of the clip's",
            "action:string:false:run (default) | status",
        ],
        kind: ToolKind::Job,
        run: |app, args| {
            if arg_str(args, "action") == Some("status") {
                return Ok(ToolOutcome::Done(status_json()));
            }
            let clip = req(arg_u64(args, "clip"), "clip")?;
            let given = match args.get("points").filter(|v| !v.is_null()) {
                Some(v) => Some((sam2::parse_points(v)?, arg_f64(args, "at").unwrap_or(app.playhead))),
                None => None,
            };
            let effect = arg_u64(args, "effect").map(|e| e as usize);
            let (done, dir) = start_track(app, clip, given, effect)?;
            Ok(ToolOutcome::Job(done, dir))
        },
    },
    ToolDef {
        name: "mask.install",
        desc: "Install the object-mask add-on (job): SAM 2.1 tiny with ONNX Runtime + DirectML, about 400 MB from the library's bin/sam2.json, every file checked by SHA-256. Already installed = done at once. uninstall=true removes it instead (cached masks stay).",
        args: &["uninstall:boolean:false:remove the add-on"],
        kind: ToolKind::Job,
        run: |_, args| {
            if arg_bool(args, "uninstall") == Some(true) {
                sam2::uninstall()?;
                return Ok(ToolOutcome::Done(json!({"ok": true, "installed": sam2::available()})));
            }
            if sam2::available() {
                return Ok(ToolOutcome::Done(status_json()));
            }
            Ok(ToolOutcome::Job(sam2::start_install(), sam2::dir().join(sam2::EXE)))
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    fn idle_session() -> Session {
        Session {
            clip: 1,
            src_t: 0.0,
            at: 0.0,
            points: vec![Point { x: 0.5, y: 0.5, add: true }],
            preview: None,
            tex: None,
            shown: vec![Point { x: 0.5, y: 0.5, add: true }],
            job: None,
            result: Arc::new(Mutex::new(None)),
            note: String::new(),
            warm: None,
        }
    }

    /// The frame hook asks for frames only while work is in flight: no session, idle click mode with
    /// its mask shown, and a warm-up that ended (even one that failed) all stay asleep.
    #[test]
    fn assert_no_idle_repaint_object_mask() {
        assert!(!wants_frames(None, false), "no click mode and no track: the hook never wakes the UI");
        let mut s = idle_session();
        assert!(!wants_frames(Some(&s), false), "points shown, nothing running");
        let warm = Progress::new();
        s.warm = Some(warm.clone());
        assert!(wants_frames(Some(&s), false), "the model is loading: the strip shows it");
        warm.finish(Some("no DirectML".into()));
        assert!(!wants_frames(Some(&s), false), "a failed warm-up must not keep repainting");
        s.job = Some((Progress::new(), s.points.clone()));
        assert!(wants_frames(Some(&s), false), "a mask job lands without input");
        assert!(wants_frames(None, true), "a running track updates its Jobs row and the mask");
        for a in [Action::ObjectMask, Action::ObjectMaskTrack] {
            assert_eq!(Action::from_id(a.id()), Some(a));
        }
    }

    #[test]
    fn object_mask_tools_are_registered_once_with_the_right_kinds() {
        for (name, kind) in [("mask.object", ToolKind::Job), ("mask.track", ToolKind::Job), ("mask.install", ToolKind::Job)] {
            let found: Vec<_> = crate::mcp::tools::all().filter(|t| t.name == name).collect();
            assert_eq!(found.len(), 1, "{name}");
            assert_eq!(found[0].kind, kind, "{name}");
        }
    }

    #[test]
    fn only_visual_clips_on_disk_can_be_masked() {
        let mut p = Project::new();
        let f = std::env::temp_dir().join(format!("se-objmask-src-{}.mp4", std::process::id()));
        std::fs::write(&f, b"x").unwrap();
        p.assets.push(crate::model::Asset {
            id: 1,
            path: f.to_string_lossy().into_owned(),
            kind: ClipKind::Video,
            duration: 2.0,
            width: 64,
            height: 36,
            fps: 25.0,
            audio_streams: vec![],
            codec: String::new(),
            folder: String::new(),
            tags: vec![],
            label: 0,
            description: String::new(),
            rel_path: None,
            parent: None,
            range: None,
            proxy: None,
            effects: vec![],
            html: None,
        });
        let ti = p.video_tracks()[0];
        let mut c = crate::model::Clip::new(10, ClipKind::Video, "shot", 0.0, 2.0);
        c.asset = 1;
        p.tracks[ti].clips.push(c);
        assert_eq!(refusal(&p, 10), None);
        assert!(refusal(&p, 99).is_some());
        p.assets[0].path = "C:/nope/missing.mp4".into();
        assert!(refusal(&p, 10).unwrap().contains("offline"));
        p.assets[0].kind = ClipKind::Audio;
        assert!(refusal(&p, 10).is_some());
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn without_the_add_on_nothing_starts() {
        if sam2::available() {
            return; // a real install in this profile
        }
        let mut app = App::build(&egui::Context::default(), None, None);
        assert!(start_track(&mut app, 1, Some((vec![Point { x: 0.5, y: 0.5, add: true }], 0.0)), None).err().unwrap().contains("not installed"));
        assert!(object_job(&mut app, 1, vec![Point { x: 0.5, y: 0.5, add: true }], None).err().unwrap().contains("not installed"));
        assert!(enter(&mut app, 1).is_err());
        assert!(active_clip().is_none());
        assert!(tracks().is_empty());
    }
}
