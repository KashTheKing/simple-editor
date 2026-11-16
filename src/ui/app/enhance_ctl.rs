//! ---- ws:ai-enhancer ----
//! The App side of the AI enhancer add-on (`engine::enhance`): the four "AI Upscale" actions (clip
//! menu ▸ AI Upscale, the palette), the FRAME_HOOK that swaps a clip onto its upscaled file once the
//! job lands (ONE labelled undo step), and the `enhance.*` MCP tools.
//!
//! The swap keeps the clip exactly as it was - effects, transform, keyframes, transitions, timing - and
//! only re-points it: the new file starts at source second `range.0`, so `src_in` (and a freeze time)
//! move back by that much. Placement is "contain" relative to the media's own size, so a ×2 file shows
//! at the same size. The original asset and file are untouched; the new one goes in the "AI enhanced" bin.

use super::feedback::{Toast, ToastKind};
use super::tools_helpers::*;
use super::*;
use crate::engine::enhance::{self, Model};
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};

pub(crate) const BIN: &str = "AI enhanced";
/// Source seconds kept each side of what the clip shows (room for transitions and small trims).
const HANDLE: f64 = 1.0;

/// One upscale in flight. `prog` is the worker (Jobs pane, cancel); `done` is what an MCP caller waits
/// on - it ends only once the swap has landed.
pub(crate) struct Job {
    clip: Id,
    old_asset: Id,
    /// Source second the new file's t=0 shows.
    t0: f64,
    out: PathBuf,
    prog: Arc<Progress>,
    done: Arc<Progress>,
    probed: Arc<Mutex<Option<Result<crate::model::Asset, String>>>>,
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
            let name = j.out.file_name().unwrap_or_default().to_string_lossy();
            (crate::ui::jobs_ui::JobKind::Bake, format!("AI upscale · {name}"), j.prog.clone())
        })
        .collect();
    if let Some(p) = enhance::installing() {
        v.push((crate::ui::jobs_ui::JobKind::ModelDownload, "Install · AI enhancer".into(), p));
    }
    v
}

/// Why `clip` cannot be upscaled, if it cannot.
fn refusal(project: &Project, clip: Id) -> Option<String> {
    let Some(c) = project.clip(clip) else { return Some(format!("no clip {clip}")) };
    if !matches!(c.kind, ClipKind::Video | ClipKind::Image) {
        return Some("AI upscale works on video and picture clips".into());
    }
    let Some(a) = project.asset(c.asset) else { return Some("the clip has no media".into()) };
    if a.html.is_some() || !matches!(a.kind, ClipKind::Video | ClipKind::Image) {
        return Some("AI upscale works on video and picture files".into());
    }
    if !Path::new(&a.path).is_file() {
        return Some(format!("{} is offline", a.path));
    }
    if project.track_of(clip).is_none_or(|ti| project.locked_of(ti)) {
        return Some("the clip's track is locked".into());
    }
    None
}

/// Start upscaling `clip`. Returns (the progress an MCP caller waits on, the output file).
pub(super) fn start(app: &mut App, clip: Id, scale: u32, model: Model) -> Result<(Arc<Progress>, PathBuf), String> {
    if !enhance::available() {
        return Err(
            "The AI enhancer add-on is not installed (Settings ▸ General ▸ AI enhancer, or enhance.install)".into()
        );
    }
    if media::ffpipe::ffmpeg_exe().is_none() {
        return Err("ffmpeg.exe not found".into());
    }
    if !matches!(scale, 2 | 4) {
        return Err("scale must be 2 or 4".into());
    }
    if let Some(why) = refusal(&app.project, clip) {
        return Err(why);
    }
    // ponytail: one at a time - two GPU upscales side by side only make both slower
    if !jobs().is_empty() {
        return Err("An AI upscale is already running - try again when it finishes".into());
    }
    let c = app.project.clip(clip).ok_or("no such clip")?;
    let a = app.project.asset(c.asset).ok_or("no media")?;
    let src = PathBuf::from(&a.path);
    let still = a.kind == ClipKind::Image;
    let fps = if a.fps > 0.0 { a.fps } else { app.project.fps };
    let range = if still { (0.0, 0.0) } else { enhance::source_range(c, a.duration, fps, HANDLE) };
    let name = enhance::output_name(&src, scale, model, still);
    let dir = src.parent().map(Path::to_path_buf).unwrap_or_else(std::env::temp_dir);
    let (stem, ext) = name.rsplit_once('.').unwrap_or((&name, "mp4"));
    let out = tools_export::unique_path(&dir, stem, ext);
    let req = enhance::Request { src, still, range, fps, scale, model, out: out.clone() };
    let (backend, probed) = (app.backend(), Arc::new(Mutex::new(None)));
    let slot = probed.clone();
    let prog = export::spawn_job("enhance", move |p| {
        enhance::run(&req, p)?;
        p.set(0.99, "Probing…");
        *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(media::probe(&req.out.to_string_lossy(), backend));
        Ok(())
    });
    let done = Progress::new();
    done.set(0.0, "AI upscale");
    jobs().push(Job { clip, old_asset: c.asset, t0: range.0, out: out.clone(), prog, done: done.clone(), probed });
    Ok((done, out))
}

/// FRAME_HOOKS: mirror the worker's progress for MCP waiters; swap the clip when a job lands.
pub(super) fn tick(app: &mut App, ctx: &egui::Context) {
    let finished: Vec<Job> = {
        let mut list = jobs();
        if list.is_empty() {
            return;
        }
        for j in list.iter() {
            j.done.set(j.prog.fraction(), j.prog.status());
        }
        let (done, pending): (Vec<Job>, Vec<Job>) =
            std::mem::take(&mut *list).into_iter().partition(|j| j.prog.is_done());
        *list = pending;
        done
    };
    for j in finished {
        let r = match j.prog.error() {
            Some(_) if j.prog.is_cancelled() => Err("AI upscale cancelled".to_string()),
            Some(e) => Err(format!("AI upscale failed: {e}")),
            None => finish(app, &j),
        };
        match r {
            Ok(msg) => {
                app.push_toast(Toast::with_folder(msg, j.out.clone()).kind(ToastKind::Success).undo(Action::Undo));
                j.done.set(1.0, "Done");
                j.done.finish(None);
            }
            Err(e) => {
                app.push_toast(Toast::new(e.clone()).kind(ToastKind::Error));
                j.done.finish(Some(e));
            }
        }
    }
    ctx.request_repaint_after(Duration::from_millis(250));
}

/// Import the upscaled file into the "AI enhanced" bin and re-point the clip - one labelled undo step.
fn finish(app: &mut App, j: &Job) -> Result<String, String> {
    let mut asset = tools_export::take_probed(&j.probed)?;
    let before = app.project.to_json();
    if let Some(old) = app.project.asset(j.old_asset) {
        // the library labels and master effects belong to the shot, not the file
        (asset.label, asset.tags, asset.effects) = (old.label, old.tags.clone(), old.effects.clone());
    }
    app.project.add_folder(BIN);
    asset.folder = BIN.to_string();
    let len = asset.duration;
    let id = app.project.add_asset(asset);
    let name = j.out.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let swapped = swap(&mut app.project, j.clip, j.old_asset, id, j.t0, len);
    app.push_undo_labeled(before, "AI upscale");
    app.after_edit();
    Ok(if swapped {
        format!("AI upscale: the clip now plays {name} (original kept in the library)")
    } else {
        format!("AI upscale: {name} is in the \"{BIN}\" bin (the clip changed while it ran, so it was not swapped)")
    })
}

/// Re-point `clip` from `old` to `new`, whose t=0 is source second `t0` and which runs `len` seconds
/// (0 = a still). False (nothing changed) when the clip is gone or now shows source outside the new file.
pub(crate) fn swap(p: &mut Project, clip: Id, old: Id, new: Id, t0: f64, len: f64) -> bool {
    let Some(c) = p.clip(clip) else { return false };
    if c.asset != old {
        return false;
    }
    let kind = p.asset(new).map(|a| a.kind).unwrap_or(ClipKind::Video);
    if kind == ClipKind::Video {
        let (a, b) = enhance::source_range(c, f64::INFINITY, 1000.0, 0.0);
        if a + 1e-3 < t0 || (len > 0.0 && b - t0 > len + 0.05) {
            return false;
        }
    }
    let (src_in, freeze) = (c.src_in, c.freeze);
    if !p.replace_clip(clip, new) {
        return false;
    }
    let Some(c) = p.clip_mut(clip) else { return false };
    if kind == ClipKind::Video {
        c.src_in = (src_in - t0).max(0.0);
        c.freeze = freeze.map(|f| (f - t0).max(0.0));
    }
    // an Image clip showing a video's frame swaps as a video (and a picture stays a picture)
    c.kind = kind;
    true
}

/// The four actions: scale and model.
fn action_args(a: Action) -> Option<(u32, Model)> {
    match a {
        Action::AiUpscale2Game => Some((2, Model::Game)),
        Action::AiUpscale2Real => Some((2, Model::Real)),
        Action::AiUpscale4Game => Some((4, Model::Game)),
        Action::AiUpscale4Real => Some((4, Model::Real)),
        _ => None,
    }
}

/// Install offer for a click on an upscale action before the add-on is there.
pub(crate) fn offer_install() {
    crate::ui::confirm::ask_app(
        "Install the AI enhancer?",
        "AI Upscale uses Real-ESRGAN, an optional add-on (about 45 MB, runs on the graphics card).\n\
         Download and install it now? Nothing else is sent anywhere. Uninstall any time in Settings ▸ General.",
        |app| {
            enhance::start_install();
            app.toast("Installing the AI enhancer - see Jobs; run AI Upscale again when it is done");
        },
    );
}

/// ACT_HANDLERS: the four AI Upscale actions run on the first selected video / picture clip.
pub(super) fn act(app: &mut App, a: Action) -> bool {
    let Some((scale, model)) = action_args(a) else { return false };
    if !enhance::available() {
        offer_install();
        return true;
    }
    let clip = app.selection.iter().copied().find(|&id| refusal(&app.project, id).is_none());
    let Some(clip) = clip else {
        app.toast("Select a video or picture clip to upscale");
        return true;
    };
    match start(app, clip, scale, model) {
        Ok((_, out)) => app.toast(format!(
            "AI upscale ×{scale} started → {} (see Jobs)",
            out.file_name().unwrap_or_default().to_string_lossy()
        )),
        Err(e) => app.toast(e),
    }
    true
}

fn status_json() -> Value {
    let list: Vec<Value> = jobs()
        .iter()
        .map(|j| json!({"clip": j.clip, "out": j.out.to_string_lossy(), "fraction": j.prog.fraction(), "status": j.prog.status()}))
        .collect();
    json!({
        "installed": enhance::available(),
        "path": enhance::exe().map(|p| p.display().to_string()),
        "version": enhance::version(),
        "installing": enhance::installing().map(|p| json!({"fraction": p.fraction(), "status": p.status()})),
        "jobs": list,
        "models": {"game": "realesr-animevideov3 (game / animation footage)", "real": "realesrgan-x4plus (real-world video)"},
    })
}

pub(crate) const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "enhance.status",
        desc: "The AI enhancer add-on (Real-ESRGAN upscaler, an optional download): installed, version, path, a running install, running upscales.",
        args: &[],
        kind: ToolKind::Read,
        run: |_, _| Ok(ToolOutcome::Done(status_json())),
    },
    ToolDef {
        name: "enhance.install",
        desc: "Install the AI enhancer add-on (job: downloads ~45 MB from the library's bin/enhancer.json, checks its SHA-256, unpacks it into the app's bin folder). Already installed = done at once. uninstall=true removes it instead.",
        args: &["uninstall:boolean:false:remove the add-on (upscaled files stay)"],
        kind: ToolKind::Job,
        run: |_, args| {
            if arg_bool(args, "uninstall") == Some(true) {
                enhance::uninstall()?;
                return Ok(ToolOutcome::Done(json!({"ok": true, "installed": enhance::available()})));
            }
            if enhance::available() {
                return Ok(ToolOutcome::Done(status_json()));
            }
            Ok(ToolOutcome::Job(enhance::start_install(), enhance::dir().join(enhance::EXE)))
        },
    },
    ToolDef {
        name: "enhance.upscale",
        desc: "AI-upscale a video or picture clip in place (job): Real-ESRGAN on the GPU over just the source range the clip shows plus 1 s handles, then the clip is re-pointed at the upscaled file keeping its effects, transform, keyframes, transitions and timing (one undo step). The new file goes in the \"AI enhanced\" bin beside the untouched original. Needs enhance.install first.",
        args: &[
            "clip:integer:true:",
            "scale:integer:false:2 or 4, default 2",
            "model:string:false:game (realesr-animevideov3, game / animation footage, default) | real (realesrgan-x4plus, real-world video)",
        ],
        kind: ToolKind::Job,
        run: |app, args| {
            let clip = req(arg_u64(args, "clip"), "clip")?;
            let scale = arg_u64(args, "scale").unwrap_or(2) as u32;
            let model = match arg_str(args, "model") {
                Some(m) => Model::parse(m).ok_or("model: game or real")?,
                None => Model::Game,
            };
            let (done, out) = start(app, clip, scale, model)?;
            Ok(ToolOutcome::Job(done, out))
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Asset, Clip};

    fn asset(id: Id, kind: ClipKind, duration: f64) -> Asset {
        Asset {
            id,
            path: format!("C:/x/{id}.mp4"),
            kind,
            duration,
            width: 640,
            height: 360,
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
        }
    }

    /// A project with one video clip (id 10) showing source 5..7 s of asset 1, with effects and keys.
    fn project() -> Project {
        let mut p = Project::new();
        p.assets.push(asset(1, ClipKind::Video, 60.0));
        p.assets.push(asset(2, ClipKind::Video, 4.0)); // the upscale of 4..8 s
        let ti = p.video_tracks()[0];
        let mut c = Clip::new(10, ClipKind::Video, "shot", 3.0, 2.0);
        c.asset = 1;
        c.src_in = 5.0;
        c.scale = crate::model::Animated::new(1.5);
        c.scale.keys.push(crate::model::Keyframe { t: 0.5, v: 2.0, ease: Default::default() });
        c.effects.push(crate::model::Effect::new(crate::model::EffectKind::Blur));
        p.tracks[ti].clips.push(c);
        p
    }

    #[test]
    fn swap_keeps_the_clip_and_moves_its_source_window() {
        let mut p = project();
        let before = p.clip(10).unwrap().clone();
        assert!(swap(&mut p, 10, 1, 2, 4.0, 4.0));
        let c = p.clip(10).unwrap();
        assert_eq!(c.asset, 2);
        assert!((c.src_in - 1.0).abs() < 1e-9, "src_in {}", c.src_in);
        // timing, transform + keys, effects untouched
        assert_eq!((c.start, c.duration), (before.start, before.duration));
        assert_eq!(c.scale.keys.len(), 1);
        assert_eq!(c.scale.value, 1.5);
        assert_eq!(c.effects.len(), 1);
        // the original asset is still in the library
        assert!(p.asset(1).is_some());
    }

    #[test]
    fn swap_refuses_a_clip_that_moved_outside_the_upscaled_range() {
        let mut p = project();
        p.clip_mut(10).unwrap().src_in = 2.0; // trimmed earlier than the file starts (4 s)
        assert!(!swap(&mut p, 10, 1, 2, 4.0, 4.0));
        assert_eq!(p.clip(10).unwrap().asset, 1);
        // re-pointed to another asset meanwhile
        let mut p = project();
        assert!(!swap(&mut p, 10, 99, 2, 4.0, 4.0));
        // gone
        assert!(!swap(&mut p, 77, 1, 2, 4.0, 4.0));
    }

    #[test]
    fn an_image_clip_of_a_video_swaps_as_a_video_and_keeps_its_freeze_frame() {
        let mut p = project();
        {
            let c = p.clip_mut(10).unwrap();
            c.kind = ClipKind::Image;
            c.freeze = Some(6.0);
        }
        assert!(swap(&mut p, 10, 1, 2, 4.0, 4.0));
        let c = p.clip(10).unwrap();
        assert_eq!(c.kind, ClipKind::Video);
        assert_eq!(c.freeze, Some(2.0));
    }

    #[test]
    fn only_video_and_pictures_can_be_upscaled() {
        let mut p = project();
        // offline (the test path does not exist)
        assert!(refusal(&p, 10).unwrap().contains("offline"));
        let f = std::env::temp_dir().join(format!("se-enh-src-{}.mp4", std::process::id()));
        std::fs::write(&f, b"x").unwrap();
        p.assets[0].path = f.to_string_lossy().into_owned();
        assert_eq!(refusal(&p, 10), None);
        p.assets[0].kind = ClipKind::Audio;
        assert!(refusal(&p, 10).is_some());
        assert!(refusal(&p, 999).is_some());
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn enhance_tools_are_registered_once_with_the_right_kinds() {
        for (name, kind) in
            [("enhance.status", ToolKind::Read), ("enhance.install", ToolKind::Job), ("enhance.upscale", ToolKind::Job)]
        {
            let found: Vec<_> = crate::mcp::tools::all().filter(|t| t.name == name).collect();
            assert_eq!(found.len(), 1, "{name}");
            assert_eq!(found[0].kind, kind, "{name}");
        }
        for a in [Action::AiUpscale2Game, Action::AiUpscale2Real, Action::AiUpscale4Game, Action::AiUpscale4Real] {
            assert!(action_args(a).is_some());
        }
    }
}
