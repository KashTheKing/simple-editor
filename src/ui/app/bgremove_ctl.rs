//! ---- ws:addon-bg-removal ----
//! The App side of background removal (`engine::bgremove`): the clip menu / palette actions, the jobs
//! that compute mattes (Jobs pane, cancel), the FRAME_HOOK that tells a paused preview about new matte
//! frames and toasts finished jobs, and the `clip.remove_background` MCP tool.
//!
//! Turning it on is one undo step that sets `Clip.bg_removal`; the mattes are a cache, never in the
//! project, so undo / redo never touch them. Until a frame's matte is computed the clip draws unkeyed.

use super::feedback::{Toast, ToastKind};
use super::tools_helpers::*;
use super::*;
use crate::engine::bgremove;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::model::{BgQuality, BgRemoval};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

/// Source seconds computed each side of what a clip shows (room for trims and transitions).
const HANDLE: f64 = 0.5;

/// One matte job: frames of one source at one quality.
pub(crate) struct Job {
    src: String,
    quality: BgQuality,
    frames: (i64, i64),
    prog: Arc<Progress>,
    /// New frames are on disk that the preview has not been told about.
    landed: Arc<AtomicBool>,
    report: Arc<Mutex<Option<bgremove::Report>>>,
}

// ponytail: one global list, not an App field - the add-on stays out of App's shared struct
static JOBS: Mutex<Vec<Job>> = Mutex::new(Vec::new());

fn jobs() -> std::sync::MutexGuard<'static, Vec<Job>> {
    JOBS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Rows for the Jobs pane.
pub(super) fn progress_jobs() -> Vec<jobs_pane::ProgJob> {
    let mut v: Vec<jobs_pane::ProgJob> = jobs()
        .iter()
        .map(|j| {
            let name = Path::new(&j.src).file_name().unwrap_or_default().to_string_lossy();
            (crate::ui::jobs_ui::JobKind::Bake, format!("Remove Background · {name}"), j.prog.clone())
        })
        .collect();
    if let Some(p) = bgremove::installing() {
        v.push((crate::ui::jobs_ui::JobKind::ModelDownload, "Install · Background removal".into(), p));
    }
    v
}

/// Why `clip`'s background cannot be removed, if it cannot.
pub(crate) fn refusal(project: &Project, clip: Id) -> Option<String> {
    let Some(c) = project.clip(clip) else { return Some(format!("no clip {clip}")) };
    if !matches!(c.kind, ClipKind::Video | ClipKind::Image) || c.is_empty_container() {
        return Some("Remove Background works on video and picture clips".into());
    }
    let Some(a) = project.asset(c.asset) else { return Some("the clip has no media".into()) };
    if a.html.is_some() || !matches!(a.kind, ClipKind::Video | ClipKind::Image) {
        return Some("Remove Background works on video and picture files".into());
    }
    if !Path::new(&a.path).is_file() {
        return Some(format!("{} is offline", a.path));
    }
    None
}

/// The request that covers what `clip` shows.
fn request(project: &Project, clip: Id, quality: BgQuality, ep: &str) -> Option<bgremove::Request> {
    let c = project.clip(clip)?;
    let a = project.asset(c.asset)?;
    let still = a.kind == ClipKind::Image || c.kind == ClipKind::Image && a.duration <= 0.0;
    let fps = if a.fps > 0.0 { a.fps } else { project.fps };
    Some(bgremove::Request {
        src: a.path.clone(),
        still,
        fps,
        size: (a.width, a.height),
        frames: bgremove::clip_frames(c, a.duration, fps, still, HANDLE),
        quality,
        ep: ep.to_string(),
    })
}

/// Set `clip`'s background removal to `bg` (None = off) as one undo step. True if it changed.
pub(crate) fn set(app: &mut App, clip: Id, bg: Option<BgRemoval>) -> bool {
    if app.project.clip(clip).is_none_or(|c| c.bg_removal == bg) {
        return false;
    }
    let before = app.project.to_json();
    if let Some(c) = app.project.clip_mut(clip) {
        c.bg_removal = bg;
    }
    app.push_undo_labeled(before, "Remove Background");
    app.after_edit();
    true
}

/// Turn background removal on for `clip` at `quality` (keeping its Edge / Show Matte) and start
/// computing the frames it shows that are not cached yet. Returns the job (None = every frame was
/// cached already). Err when the add-on (or that quality's model) is not installed: the clip still
/// gets the setting and draws unkeyed until it is.
pub(super) fn start(app: &mut App, clip: Id, quality: BgQuality, ep: &str) -> Result<Option<Arc<Progress>>, String> {
    if let Some(why) = refusal(&app.project, clip) {
        return Err(why);
    }
    let old = app.project.clip(clip).and_then(|c| c.bg_removal.clone()).unwrap_or_default();
    set(app, clip, Some(BgRemoval { quality, ..old }));
    if !bgremove::available(quality) {
        return Err(format!(
            "Background removal ({}) is not installed (Settings ▸ Add-ons ▸ Background removal, or clip.remove_background with install=true)",
            quality.label()
        ));
    }
    if media::ffpipe::ffmpeg_exe().is_none() {
        return Err("ffmpeg.exe not found".into());
    }
    let req = request(&app.project, clip, quality, ep).ok_or("no such clip")?;
    let (have, total) = bgremove::coverage(&req.src, quality, req.frames.0, req.frames.1);
    if have >= total {
        return Ok(None);
    }
    let mut list = jobs();
    if let Some(j) = list.iter().find(|j| j.src == req.src && j.quality == quality && !j.prog.is_done()) {
        if j.frames.0 <= req.frames.0 && j.frames.1 >= req.frames.1 {
            return Ok(Some(j.prog.clone()));
        }
    }
    // ponytail: one model on the GPU at a time - a second job waits for the first, so two never fight
    // over graphics memory
    let busy: Vec<Arc<Progress>> = list.iter().map(|j| j.prog.clone()).collect();
    let landed = Arc::new(AtomicBool::new(false));
    let report = Arc::new(Mutex::new(None));
    let (flag, slot, r) = (landed.clone(), report.clone(), req.clone());
    let prog = export::spawn_job("bgremove", move |p| {
        for b in &busy {
            while !b.is_done() {
                if p.is_cancelled() {
                    return Err(export::CANCELLED.into());
                }
                p.set(0.0, "Waiting for another background removal…");
                std::thread::sleep(Duration::from_millis(200));
            }
        }
        let rep = bgremove::run(&r, p, &|| flag.store(true, Ordering::Release))?;
        *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(rep);
        Ok(())
    });
    list.push(Job { src: req.src, quality, frames: req.frames, prog: prog.clone(), landed, report });
    Ok(Some(prog))
}

/// FRAME_HOOKS: tell the preview about new matte frames (only while it is not playing - an eviction
/// mid-play would make it re-decode under the playhead) and toast the jobs that end.
pub(super) fn tick(app: &mut App, ctx: &egui::Context) {
    if CTX.get().is_none() {
        let _ = CTX.set(ctx.clone());
    }
    let (finished, landed) = {
        let mut list = jobs();
        if list.is_empty() {
            return;
        }
        let playing = app.player.is_playing();
        let mut landed = Vec::new();
        for j in list.iter() {
            if (!playing || j.prog.is_done()) && j.landed.swap(false, Ordering::AcqRel) {
                landed.push(j.src.clone());
            }
        }
        let (done, pending): (Vec<Job>, Vec<Job>) =
            std::mem::take(&mut *list).into_iter().partition(|j| j.prog.is_done() && !j.landed.load(Ordering::Acquire));
        *list = pending;
        (done, landed)
    };
    for src in landed {
        app.player.mattes_landed(&src);
    }
    for j in finished {
        let name = Path::new(&j.src).file_name().unwrap_or_default().to_string_lossy().into_owned();
        match j.prog.error() {
            Some(_) if j.prog.is_cancelled() => app.toast("Remove Background cancelled"),
            Some(e) => app.push_toast(Toast::new(format!("Remove Background failed: {e}")).kind(ToastKind::Error)),
            None => {
                let rep = j.report.lock().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default();
                let speed = if rep.frames > 0 {
                    format!(" ({} frames, {:.0} ms a frame on the {})", rep.frames, rep.ms_per_frame, ep_name(&rep.ep))
                } else {
                    String::new()
                };
                app.push_toast(Toast::new(format!("Background removed · {name}{speed}")).kind(ToastKind::Success));
            }
        }
    }
    ctx.request_repaint_after(Duration::from_millis(250));
}

fn ep_name(ep: &str) -> &'static str {
    if ep == "dml" {
        "GPU"
    } else {
        "CPU"
    }
}

/// The running job for `clip`'s source and quality, if any.
pub(crate) fn job_for(project: &Project, clip: Id) -> Option<Arc<Progress>> {
    let c = project.clip(clip)?;
    let q = c.bg_removal.as_ref()?.quality;
    let path = &project.asset(c.asset)?.path;
    jobs().iter().find(|j| &j.src == path && j.quality == q && !j.prog.is_done()).map(|j| j.prog.clone())
}

/// (cached, total) matte frames of what `clip` shows, counted now (a folder listing: MCP, not per frame).
pub(crate) fn clip_coverage(project: &Project, clip: Id) -> Option<(usize, usize)> {
    let q = project.clip(clip)?.bg_removal.as_ref()?.quality;
    let r = request(project, clip, q, "auto")?;
    Some(bgremove::coverage(&r.src, q, r.frames.0, r.frames.1))
}

/// What a coverage count is of: (source, quality, first frame, end frame).
type CovKey = (String, BgQuality, i64, i64);

struct Cov {
    at: Instant,
    val: Option<(usize, usize)>,
    busy: bool,
}

static COVERAGE: Mutex<Option<HashMap<CovKey, Cov>>> = Mutex::new(None);
/// The UI's context, so a background count can ask for a repaint when it lands.
static CTX: std::sync::OnceLock<egui::Context> = std::sync::OnceLock::new();

/// How old a count may be before it is redone: often while a job writes frames, rarely otherwise.
fn cov_ttl(busy_job: bool) -> Duration {
    Duration::from_millis(if busy_job { 400 } else { 3000 })
}

/// `clip_coverage` for the Inspector, which draws every frame: the folder listing (thousands of files
/// for a long clip) runs on a short-lived thread and this returns the last count (None until the first
/// lands), so the UI thread never lists a folder.
pub(crate) fn clip_coverage_cached(project: &Project, clip: Id) -> Option<(usize, usize)> {
    let q = project.clip(clip)?.bg_removal.as_ref()?.quality;
    let r = request(project, clip, q, "auto")?;
    let busy_job = job_for(project, clip).is_some();
    let key: CovKey = (r.src.clone(), q, r.frames.0, r.frames.1);
    let mut g = COVERAGE.lock().unwrap_or_else(|e| e.into_inner());
    let map = g.get_or_insert_with(HashMap::new);
    if map.len() > 64 && !map.contains_key(&key) {
        map.retain(|_, c| c.busy);
    }
    let e = map.entry(key.clone()).or_insert(Cov { at: Instant::now(), val: None, busy: false });
    let stale = e.val.is_none() || e.at.elapsed() >= cov_ttl(busy_job);
    if stale && !e.busy {
        e.busy = true;
        std::thread::spawn(move || {
            let val = bgremove::coverage(&key.0, key.1, key.2, key.3);
            let mut g = COVERAGE.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(c) = g.get_or_insert_with(HashMap::new).get_mut(&key) {
                *c = Cov { at: Instant::now(), val: Some(val), busy: false };
            }
            drop(g);
            if let Some(ctx) = CTX.get() {
                ctx.request_repaint();
            }
        });
    }
    e.val
}

/// Install offer for a click before the add-on (or that quality's model) is there.
pub(crate) fn offer_install(q: BgQuality) {
    let size = match q {
        BgQuality::Fast => "about 45 MB",
        BgQuality::Best => "about 240 MB",
    };
    crate::ui::confirm::ask_app(
        "Install background removal?",
        &format!(
            "Remove Background ({}) uses an optional add-on ({size}: ONNX Runtime and the {} model; runs on the graphics card).\n\
             Download and install it now? Nothing else is sent anywhere. Uninstall any time in Settings ▸ Add-ons.",
            q.label(),
            if q == BgQuality::Fast { "MODNet" } else { "BiRefNet" }
        ),
        move |app| {
            bgremove::start_install(q);
            app.toast("Installing background removal - see Jobs; the clip is keyed once it is done and computed");
        },
    );
}

/// The clips an action works on: the selected video / picture clips.
fn targets(app: &App) -> Vec<Id> {
    app.selection.iter().copied().filter(|&id| refusal(&app.project, id).is_none()).collect()
}

/// ACT_HANDLERS: Remove Background Fast / Best / Off on the selected clips.
pub(super) fn act(app: &mut App, a: Action) -> bool {
    let q = match a {
        Action::RemoveBackgroundFast => Some(BgQuality::Fast),
        Action::RemoveBackgroundBest => Some(BgQuality::Best),
        Action::RemoveBackgroundOff => None,
        _ => return false,
    };
    let clips = targets(app);
    if clips.is_empty() {
        app.toast("Select a video or picture clip");
        return true;
    }
    let Some(q) = q else {
        for c in clips {
            set(app, c, None);
        }
        return true;
    };
    let mut started = 0;
    for &c in &clips {
        match start(app, c, q, "auto") {
            Ok(Some(_)) => started += 1,
            Ok(None) => {}
            Err(_) if !bgremove::available(q) => {
                offer_install(q);
                return true;
            }
            Err(e) => {
                app.toast(e);
                return true;
            }
        }
    }
    if started > 0 {
        app.toast("Removing the background - see Jobs; the clip plays unkeyed until its frames are ready");
    }
    true
}

/// When a job just finished and the install for its quality landed meanwhile, nothing restarts on its
/// own: the user clicks again (keeps a download from silently starting GPU work).
fn status_json(app: &App, clip: Option<Id>) -> Value {
    let i = bgremove::installed();
    let mut v = json!({
        "installed": {"helper": i.helper, "runtime": i.runtime, "fast": i.fast, "best": i.best},
        "path": bgremove::dir().display().to_string(),
        "version": bgremove::version(),
        "installing": bgremove::installing().map(|p| json!({"fraction": p.fraction(), "status": p.status()})),
        "install_error": bgremove::install_error(),
        "jobs": jobs().iter().map(|j| json!({
            "source": j.src, "quality": j.quality.id(), "frames": [j.frames.0, j.frames.1],
            "fraction": j.prog.fraction(), "status": j.prog.status(),
        })).collect::<Vec<_>>(),
        "models": {"fast": "MODNet (portrait matting, Apache-2.0)", "best": "BiRefNet lite (general subjects, MIT)"},
    });
    if let Some(id) = clip {
        let c = app.project.clip(id);
        let bg = c.and_then(|c| c.bg_removal.clone());
        let cov = clip_coverage(&app.project, id);
        v["clip"] = json!({
            "id": id,
            "on": bg.is_some(),
            "quality": bg.as_ref().map(|b| b.quality.id()),
            "edge": bg.as_ref().map(|b| b.edge),
            "show_matte": bg.as_ref().map(|b| b.show_matte),
            "frames_ready": cov.map(|c| c.0),
            "frames_total": cov.map(|c| c.1),
            "job": job_for(&app.project, id).map(|p| json!({"fraction": p.fraction(), "status": p.status()})),
            "refusal": refusal(&app.project, id),
        });
    }
    v
}

pub(crate) const TOOLS: &[ToolDef] = &[ToolDef {
    name: "clip.remove_background",
    desc: "Remove a video / picture clip's background without a green screen (optional add-on: MODNet = fast, people; BiRefNet = best, any subject; ONNX Runtime on the GPU via DirectML, else the CPU). Sets the clip's background removal (one undo step) and computes a cached matte per frame of what it shows as a job; the clip draws unkeyed until its frames are ready. status=true only reports (installed parts, jobs, this clip's ready frames). install=true installs the add-on for that quality first (a job). off=true turns it off. edge / show_matte only change how the matte is drawn.",
    args: &[
        "clip:integer:false:needed for anything but status / install",
        "quality:string:false:fast (MODNet, default) | best (BiRefNet)",
        "status:boolean:false:report only",
        "install:boolean:false:install the add-on (and this quality's model) instead",
        "uninstall:boolean:false:remove the add-on (computed mattes stay in the cache)",
        "off:boolean:false:turn background removal off for the clip",
        "edge:number:false:-1 shrink .. 0 .. 1 feather",
        "show_matte:boolean:false:draw the matte instead of the keyed picture",
        "ep:string:false:auto (default) | dml | cpu - where the model runs (benchmarks)",
    ],
    kind: ToolKind::Job,
    run: |app, args| {
        let clip = arg_u64(args, "clip");
        let quality = match arg_str(args, "quality") {
            Some(q) => BgQuality::parse(q).ok_or("quality: fast or best")?,
            None => clip
                .and_then(|c| app.project.clip(c))
                .and_then(|c| c.bg_removal.as_ref().map(|b| b.quality))
                .unwrap_or_default(),
        };
        if arg_bool(args, "status") == Some(true) {
            return Ok(ToolOutcome::Done(status_json(app, clip)));
        }
        if arg_bool(args, "uninstall") == Some(true) {
            bgremove::uninstall()?;
            return Ok(ToolOutcome::Done(status_json(app, clip)));
        }
        if arg_bool(args, "install") == Some(true) {
            if bgremove::available(quality) {
                return Ok(ToolOutcome::Done(status_json(app, clip)));
            }
            return Ok(ToolOutcome::Job(bgremove::start_install(quality), bgremove::dir()));
        }
        let clip = req(clip, "clip")?;
        if arg_bool(args, "off") == Some(true) {
            set(app, clip, None);
            return Ok(ToolOutcome::Done(status_json(app, Some(clip))));
        }
        let (edge, show) = (arg_f64(args, "edge"), arg_bool(args, "show_matte"));
        if edge.is_some() || show.is_some() {
            if let Some(why) = refusal(&app.project, clip) {
                return Err(why);
            }
            let mut bg = app.project.clip(clip).and_then(|c| c.bg_removal.clone()).unwrap_or_default();
            bg.quality = quality;
            if let Some(e) = edge {
                bg.edge = e.clamp(-1.0, 1.0) as f32;
            }
            if let Some(s) = show {
                bg.show_matte = s;
            }
            set(app, clip, Some(bg));
            if arg_str(args, "quality").is_none() {
                return Ok(ToolOutcome::Done(status_json(app, Some(clip))));
            }
        }
        let ep = arg_str(args, "ep").unwrap_or("auto").to_string();
        if !matches!(ep.as_str(), "auto" | "dml" | "cpu") {
            return Err("ep: auto, dml or cpu".into());
        }
        match start(app, clip, quality, &ep)? {
            Some(p) => {
                let dir = bgremove::cache_dir(&app.project.asset(app.project.clip(clip).map_or(0, |c| c.asset)).map(|a| a.path.clone()).unwrap_or_default(), quality);
                Ok(ToolOutcome::Job(p, dir))
            }
            None => Ok(ToolOutcome::Done(status_json(app, Some(clip)))),
        }
    },
}];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Asset, Clip};

    fn project(path: &str) -> Project {
        let mut p = Project::new();
        p.assets.push(Asset {
            id: 1,
            path: path.to_string(),
            kind: ClipKind::Video,
            duration: 10.0,
            width: 640,
            height: 360,
            fps: 25.0,
            audio_streams: vec![],
            codec: "h264".into(),
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
        let mut c = Clip::new(10, ClipKind::Video, "shot", 0.0, 2.0);
        c.asset = 1;
        c.src_in = 3.0;
        p.tracks[ti].clips.push(c);
        p
    }

    #[test]
    fn the_request_covers_what_the_clip_shows_plus_handles() {
        let p = project("C:/x/a.mp4");
        let r = request(&p, 10, BgQuality::Best, "auto").unwrap();
        // source 3..5 s at 25 fps, half a second each side
        assert_eq!(r.frames, (62, 138));
        assert!(!r.still);
        assert_eq!(r.quality, BgQuality::Best);
    }

    #[test]
    fn refusals_name_the_reason() {
        let p = project("Z:/nowhere/offline.mp4");
        assert!(refusal(&p, 10).unwrap().contains("offline"));
        assert!(refusal(&p, 99).unwrap().contains("no clip"));
    }

    #[test]
    fn the_inspector_count_lands_off_the_ui_thread() {
        let mut p = project("Z:/nowhere/coverage.mp4");
        p.clip_mut(10).unwrap().bg_removal = Some(BgRemoval::default());
        // the first call only starts the count; it lands on its own thread
        let t0 = Instant::now();
        let got = loop {
            if let Some(v) = clip_coverage_cached(&p, 10) {
                break v;
            }
            assert!(t0.elapsed() < Duration::from_secs(5), "the count never landed");
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(got, (0, 76));
        assert_eq!(clip_coverage(&p, 10), Some(got));
        // a clip without background removal has no count
        p.clip_mut(10).unwrap().bg_removal = None;
        assert_eq!(clip_coverage_cached(&p, 10), None);
    }

    #[test]
    fn the_tool_is_registered_as_a_job() {
        let t = TOOLS.iter().find(|t| t.name == "clip.remove_background").unwrap();
        assert_eq!(t.kind, ToolKind::Job);
    }
}
