//! ---- ws:addon-deepfilternet ----
//! The App side of Clean Audio (`engine::deepfilter`, `ui::denoise_ui`): the jobs (Jobs pane, cancel),
//! the FRAME_HOOK that re-points the clips at the cleaned file once a job lands (ONE labelled undo
//! step per file), and the `audio.clean` / `denoise.*` MCP tools.
//!
//! A Clean covers only the source range the clips play (plus `deepfilter::HANDLE` each side), one job
//! per source file. Re-cleaning a clip that already plays a cleaned file cleans its original again (at
//! the new strength). The cleaned WAV goes in the "Cleaned audio" bin; the original stays untouched.

use super::feedback::{Toast, ToastKind};
use super::tools_helpers::*;
use super::*;
use crate::engine::deepfilter::{self, Engine};
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::ui::denoise_ui;

pub(crate) const BIN: &str = "Cleaned audio";

/// One file being cleaned. `batch` is what an MCP caller waits on: shared by every job one request
/// started, finished once the last of them has swapped its clips.
pub(crate) struct Job {
    clips: Vec<Id>,
    /// The original asset the clips' windows were measured against.
    orig: Id,
    /// Source second the cleaned file's t=0 is.
    t0: f64,
    out: PathBuf,
    prog: Arc<Progress>,
    batch: Arc<Progress>,
    batch_err: Arc<Mutex<Option<String>>>,
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
            (crate::ui::jobs_ui::JobKind::Bake, format!("Clean audio · {name}"), j.prog.clone())
        })
        .collect();
    if let Some(p) = deepfilter::installing() {
        v.push((crate::ui::jobs_ui::JobKind::ModelDownload, "Install · Speech denoise".into(), p));
    }
    v
}

/// The original asset behind `asset` (itself, unless it is a cleaned file) and the shift from the
/// clip's source seconds to the original's.
fn original_of(project: &Project, asset: Id) -> Option<(Id, f64)> {
    let a = project.asset(asset)?;
    match deepfilter::sidecar_of(Path::new(&a.path)) {
        None => Some((asset, 0.0)),
        Some(side) => project
            .assets
            .iter()
            .find(|x| Path::new(&x.path) == Path::new(&side.src) || x.path.eq_ignore_ascii_case(&side.src))
            .map(|x| (x.id, side.t0)),
    }
}

/// Why `clip` cannot be cleaned, if it cannot.
fn refusal(project: &Project, clip: Id) -> Option<String> {
    let Some(c) = project.clip(clip) else { return Some(format!("no clip {clip}")) };
    let Some(a) = project.asset(c.asset) else { return Some("the clip has no media".into()) };
    if a.html.is_some() || a.kind == ClipKind::Image {
        return Some("Clean Audio works on clips with sound".into());
    }
    if project.track_of(clip).is_none_or(|ti| project.locked_of(ti)) {
        return Some("the clip's track is locked".into());
    }
    let Some((orig, _)) = original_of(project, c.asset) else {
        return Some("the cleaned file's original is no longer in the project".into());
    };
    let path = project.asset(orig).map(|a| a.path.clone()).unwrap_or_default();
    if !Path::new(&path).is_file() {
        return Some(format!("{path} is offline"));
    }
    None
}

/// The jobs one Clean needs: per original asset, the clips and the source range (with handles).
pub(crate) fn plan(project: &Project, clips: &[Id]) -> Result<Vec<(Id, Vec<Id>, (f64, f64))>, String> {
    let mut groups: Vec<(Id, Vec<Id>, (f64, f64))> = Vec::new();
    let mut why = None;
    for &id in clips {
        if let Some(w) = refusal(project, id) {
            why.get_or_insert(w);
            continue;
        }
        let c = project.clip(id).ok_or("no such clip")?;
        let (orig, shift) = original_of(project, c.asset).ok_or("no original")?;
        let (lo, hi) = denoise_ui::window(project, id).ok_or("no such clip")?;
        let (lo, hi) = (lo + shift, hi + shift);
        match groups.iter_mut().find(|g| g.0 == orig) {
            Some(g) => {
                g.1.push(id);
                g.2 = (g.2 .0.min(lo), g.2 .1.max(hi));
            }
            None => groups.push((orig, vec![id], (lo, hi))),
        }
    }
    for g in &mut groups {
        let dur = project.asset(g.0).map(|a| a.duration).unwrap_or(0.0);
        let hi = g.2 .1 + deepfilter::HANDLE;
        g.2 = ((g.2 .0 - deepfilter::HANDLE).max(0.0), if dur > 0.0 { hi.min(dur) } else { hi });
        // millisecond edges: the cache key and the file agree exactly
        g.2 = ((g.2 .0 * 1000.0).floor() / 1000.0, (g.2 .1 * 1000.0).ceil() / 1000.0);
    }
    if groups.is_empty() {
        return Err(why.unwrap_or_else(|| "Select a clip with sound on an audio track".into()));
    }
    Ok(groups)
}

/// Start cleaning `ids` (their audio-track clips). Returns (what an MCP caller waits on, the files).
pub(super) fn start(
    app: &mut App,
    ids: &[Id],
    asked: Option<Engine>,
    strength: f64,
) -> Result<(Arc<Progress>, Vec<PathBuf>), String> {
    let engine = deepfilter::resolve(asked, deepfilter::available())?;
    if media::ffpipe::ffmpeg_exe().is_none() {
        return Err("ffmpeg.exe not found".into());
    }
    let strength = strength.clamp(0.0, 1.0);
    let clips = denoise_ui::targets(&app.project, ids);
    let groups = plan(&app.project, &clips)?;
    let busy: Vec<PathBuf> = jobs().iter().map(|j| j.out.clone()).collect();
    let batch = Progress::new();
    batch.set(0.0, "Clean audio");
    let batch_err = Arc::new(Mutex::new(None));
    let mut outs = Vec::new();
    for (orig, clips, range) in groups {
        let src = PathBuf::from(app.project.asset(orig).map(|a| a.path.clone()).unwrap_or_default());
        let out = deepfilter::cached_path(&src, range, strength, engine);
        if busy.contains(&out) {
            return Err("That clip is already being cleaned".into());
        }
        let req = deepfilter::Request { src, range, strength, engine, out: out.clone() };
        let (backend, probed) = (app.backend(), Arc::new(Mutex::new(None)));
        let slot = probed.clone();
        let prog = export::spawn_job("denoise", move |p| {
            deepfilter::run(&req, p)?;
            *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(media::probe(&req.out.to_string_lossy(), backend));
            Ok(())
        });
        jobs().push(Job {
            clips,
            orig,
            t0: range.0,
            out: out.clone(),
            prog,
            batch: batch.clone(),
            batch_err: batch_err.clone(),
            probed,
        });
        outs.push(out);
    }
    Ok((batch, outs))
}

/// FRAME_HOOKS: start what the Inspector row asked for, mirror progress for MCP waiters, swap the clips
/// when a job lands.
pub(super) fn tick(app: &mut App, ctx: &egui::Context) {
    if let Some(r) = denoise_ui::take_request() {
        match start(app, &r.ids, None, r.strength) {
            Ok((_, outs)) => {
                let files = if outs.len() == 1 { "1 file".to_string() } else { format!("{} files", outs.len()) };
                app.push_toast(Toast::new(format!("Cleaning audio ({files})")).action("Jobs", Action::ToggleJobs));
            }
            Err(e) => app.toast(e),
        }
    }
    let finished: Vec<Job> = {
        let mut list = jobs();
        if list.is_empty() {
            return;
        }
        for j in list.iter() {
            let mine: Vec<f32> =
                list.iter().filter(|k| Arc::ptr_eq(&k.batch, &j.batch)).map(|k| k.prog.fraction()).collect();
            j.batch.set(mine.iter().sum::<f32>() / mine.len().max(1) as f32, j.prog.status());
        }
        let (done, pending): (Vec<Job>, Vec<Job>) =
            std::mem::take(&mut *list).into_iter().partition(|j| j.prog.is_done());
        *list = pending;
        done
    };
    for j in finished {
        let r = match j.prog.error() {
            Some(_) if j.prog.is_cancelled() => Err("Clean audio cancelled".to_string()),
            Some(e) => Err(format!("Clean audio failed: {e}")),
            None => finish(app, &j),
        };
        match r {
            Ok(msg) => app.push_toast(Toast::new(msg).kind(ToastKind::Success).undo(Action::Undo)),
            Err(e) => {
                app.push_toast(Toast::new(e.clone()).kind(ToastKind::Error));
                j.batch_err.lock().unwrap_or_else(|x| x.into_inner()).get_or_insert(e);
            }
        }
        let last = !jobs().iter().any(|k| Arc::ptr_eq(&k.batch, &j.batch));
        if last {
            let err = j.batch_err.lock().unwrap_or_else(|x| x.into_inner()).clone();
            if err.is_none() {
                j.batch.set(1.0, "Done");
            }
            j.batch.finish(err);
        }
    }
    ctx.request_repaint_after(Duration::from_millis(250));
}

/// Import the cleaned file into the "Cleaned audio" bin (once) and re-point the clips - one undo step.
fn finish(app: &mut App, j: &Job) -> Result<String, String> {
    let mut asset = tools_export::take_probed(&j.probed)?;
    let before = app.project.to_json();
    let path = j.out.to_string_lossy().into_owned();
    let id = match app.project.assets.iter().find(|a| Path::new(&a.path) == j.out) {
        Some(a) => a.id,
        None => {
            if let Some(old) = app.project.asset(j.orig) {
                (asset.label, asset.tags) = (old.label, old.tags.clone());
            }
            app.project.add_folder(BIN);
            asset.folder = BIN.to_string();
            asset.path = path;
            app.project.add_asset(asset)
        }
    };
    let len = app.project.asset(id).map(|a| a.duration).unwrap_or(0.0);
    let n = swap_all(&mut app.project, &j.clips, j.orig, id, j.t0, len);
    if n > 0 {
        app.push_undo_labeled(before, "Clean audio");
        app.after_edit();
    }
    let name = j.out.file_name().unwrap_or_default().to_string_lossy().into_owned();
    Ok(match n {
        0 => format!("Clean audio: {name} is in the \"{BIN}\" bin (the clips changed while it ran)"),
        1 => "Clean audio: the clip now plays the cleaned sound (A / B in the Inspector compares)".into(),
        n => format!("Clean audio: {n} clips now play the cleaned sound (A / B in the Inspector compares)"),
    })
}

/// Re-point each of `clips` that still plays `orig` (or an older Clean of it) at `new`, whose t=0 is
/// source second `t0` of `orig` and which runs `len` seconds. Clips now playing outside it are left.
pub(crate) fn swap_all(project: &mut Project, clips: &[Id], orig: Id, new: Id, t0: f64, len: f64) -> usize {
    let mut n = 0;
    for &id in clips {
        let Some(c) = project.clip(id) else { continue };
        if c.asset == new {
            continue;
        }
        let Some((o, shift)) = original_of(project, c.asset) else { continue };
        if o != orig {
            continue;
        }
        let Some((lo, hi)) = denoise_ui::window(project, id) else { continue };
        if lo + shift + 1e-3 < t0 || (len > 0.0 && hi + shift > t0 + len + 0.05) {
            continue;
        }
        n += denoise_ui::repoint(project, id, new, shift - t0) as usize;
    }
    n
}

fn engine_arg(args: &Value) -> Result<Option<Engine>, String> {
    match arg_str(args, "engine") {
        Some(e) => Engine::parse(e).map(Some).ok_or_else(|| "engine: deepfilter or ffmpeg".to_string()),
        None => Ok(None),
    }
}

fn strength_arg(args: &Value) -> f64 {
    arg_f64(args, "strength").unwrap_or(deepfilter::DEFAULT_STRENGTH).clamp(0.0, 1.0)
}

/// `audio.clean` (and `audio.repair` preset "clean"): a Clean, or with `ab` an A / B flip.
pub(super) fn clean_tool(app: &mut App, args: &Value) -> Result<ToolOutcome, String> {
    let ids = req(arg_ids(args, "clip_ids"), "clip_ids")?;
    if let Some(ab) = arg_str(args, "ab") {
        let b = match ab.trim().to_ascii_lowercase().as_str() {
            "a" | "original" => false,
            "b" | "cleaned" => true,
            _ => return Err("ab: a (original) or b (cleaned)".into()),
        };
        let before = app.project.to_json();
        let n = denoise_ui::set_ab(&mut app.project, &ids, b, &|p| deepfilter::sidecar_of(Path::new(p)));
        if n > 0 {
            app.push_undo_labeled(before, if b { "Cleaned audio (B)" } else { "Original audio (A)" });
            app.after_edit();
        }
        return Ok(ToolOutcome::Done(json!({"ok": true, "changed": n})));
    }
    let (done, outs) = start(app, &ids, engine_arg(args)?, strength_arg(args))?;
    Ok(ToolOutcome::Job(done, outs.into_iter().next().unwrap_or_default()))
}

/// `export.denoise`'s engine: Some(strength) = DeepFilterNet, None = ffmpeg afftdn.
pub(super) fn bake_engine(args: &Value) -> Result<Option<f64>, String> {
    Ok(match deepfilter::resolve(engine_arg(args)?, deepfilter::available())? {
        Engine::DeepFilter => Some(strength_arg(args)),
        Engine::Ffmpeg => None,
    })
}

fn status_json() -> Value {
    let list: Vec<Value> = jobs()
        .iter()
        .map(|j| json!({"clips": j.clips, "out": j.out.to_string_lossy(), "fraction": j.prog.fraction(), "status": j.prog.status()}))
        .collect();
    let installed = deepfilter::available();
    json!({
        "installed": installed,
        "default_engine": if installed { "deepfilter" } else { "ffmpeg" },
        "path": deepfilter::exe().map(|p| p.display().to_string()),
        "version": deepfilter::version(),
        "installing": deepfilter::installing().map(|p| json!({"fraction": p.fraction(), "status": p.status()})),
        "cache": deepfilter::cache_dir().display().to_string(),
        "jobs": list,
    })
}

pub(crate) const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "denoise.status",
        desc: "The speech-denoise add-on (DeepFilterNet, an optional download): installed, version, path, the default engine, a running install, running Clean Audio jobs.",
        args: &[],
        kind: ToolKind::Read,
        run: |_, _| Ok(ToolOutcome::Done(status_json())),
    },
    ToolDef {
        name: "denoise.install",
        desc: "Install the speech-denoise add-on (job: downloads the ~27 MB DeepFilterNet CLI named by the library's bin/deepfilter.json, checks its SHA-256, puts it in the app's bin folder). Already installed = done at once. uninstall=true removes it instead.",
        args: &["uninstall:boolean:false:remove the add-on (cleaned audio stays)"],
        kind: ToolKind::Job,
        run: |_, args| {
            if arg_bool(args, "uninstall") == Some(true) {
                deepfilter::uninstall()?;
                return Ok(ToolOutcome::Done(json!({"ok": true, "installed": deepfilter::available()})));
            }
            if deepfilter::available() {
                return Ok(ToolOutcome::Done(status_json()));
            }
            Ok(ToolOutcome::Job(deepfilter::start_install(), deepfilter::dir().join(deepfilter::EXE)))
        },
    },
    ToolDef {
        name: "audio.clean",
        desc: "Clean Audio (job): remove background noise from the clips' sound - only the source range they play plus 1 s handles, cached per source, range and strength - then re-point the clips at the cleaned WAV (one undo step; the original file is untouched and stays in the library). ab=a|b instead flips the clips between the original and the cleaned file at once.",
        args: &[
            "clip_ids:array:true:audio clips (a video clip's linked audio is included)",
            "engine:string:false:deepfilter | ffmpeg (default deepfilter when installed, else ffmpeg afftdn)",
            "strength:number:false:0..1, default 0.8 (1 = remove all the noise)",
            "ab:string:false:a (original) | b (cleaned): flip instead of cleaning",
        ],
        kind: ToolKind::Job,
        run: clean_tool,
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Asset, Clip};

    fn asset(id: Id, path: &str, duration: f64) -> Asset {
        Asset {
            id,
            path: path.into(),
            kind: ClipKind::Audio,
            duration,
            width: 0,
            height: 0,
            fps: 0.0,
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

    /// Two audio clips of one real file (5..7 s and 20..21 s) plus one of another.
    fn project(file: &str) -> Project {
        let mut p = Project::new();
        p.assets.push(asset(1, file, 30.0));
        p.assets.push(asset(2, file, 30.0));
        let ti = p.audio_tracks()[0];
        for (id, start, src_in, dur, a) in [(10, 0.0, 5.0, 2.0, 1), (11, 4.0, 20.0, 1.0, 1), (12, 8.0, 0.0, 1.0, 2)] {
            let mut c = Clip::new(id, ClipKind::Audio, "v", start, dur);
            c.asset = a;
            c.src_in = src_in;
            p.tracks[ti].clips.push(c);
        }
        p
    }

    fn real_file() -> PathBuf {
        let f = std::env::temp_dir().join(format!("se-dfn-plan-{}.wav", std::process::id()));
        std::fs::write(&f, b"x").unwrap();
        f
    }

    #[test]
    fn one_job_per_source_covering_every_clip_plus_handles() {
        let f = real_file();
        let p = project(&f.to_string_lossy());
        let g = plan(&p, &[10, 11, 12]).unwrap();
        assert_eq!(g.len(), 2);
        assert_eq!((g[0].0, g[0].1.clone(), g[0].2), (1, vec![10, 11], (4.0, 22.0)));
        assert_eq!((g[1].0, g[1].2), (2, (0.0, 2.0)));
        let _ = std::fs::remove_file(&f);
    }

    /// A speed curve plays from src_in to the end of the file: the Clean covers that, and the swap lands.
    #[test]
    fn a_speed_curve_clip_is_cleaned_to_the_end_of_its_file_and_swapped() {
        let f = real_file();
        let mut p = project(&f.to_string_lossy());
        p.clip_mut(10).unwrap().speed_curve.keys.push(crate::model::Keyframe { t: 0.0, v: 2.0, ease: Default::default() });
        let g = plan(&p, &[10]).unwrap();
        assert_eq!(g[0].2, (4.0, 30.0));
        p.assets.push(asset(3, "C:/cache/denoise/v.2.wav", 26.0)); // a Clean of 4..30 s
        assert_eq!(swap_all(&mut p, &[10], 1, 3, 4.0, 26.0), 1);
        assert_eq!((p.clip(10).unwrap().asset, p.clip(10).unwrap().src_in), (3, 1.0));
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn offline_or_locked_clips_are_refused_with_a_reason() {
        let mut p = project(r"C:\nope\gone.wav");
        assert!(plan(&p, &[10]).unwrap_err().contains("offline"));
        let f = real_file();
        p.assets[0].path = f.to_string_lossy().into_owned();
        let ti = p.track_of(10).unwrap();
        p.tracks[ti].locked = true;
        assert!(plan(&p, &[10]).unwrap_err().contains("locked"));
        assert!(plan(&p, &[]).is_err());
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn the_swap_moves_the_source_window_and_skips_clips_outside_the_file() {
        let f = real_file();
        let mut p = project(&f.to_string_lossy());
        p.assets.push(asset(3, "C:/cache/denoise/v.1.wav", 4.0)); // a Clean of 4..8 s
        // clip 11 plays 20..21 s: outside the file, left alone; clip 12 plays another asset
        assert_eq!(swap_all(&mut p, &[10, 11, 12], 1, 3, 4.0, 4.0), 1);
        let c = p.clip(10).unwrap();
        assert_eq!((c.asset, c.src_in, c.start, c.duration), (3, 1.0, 0.0, 2.0));
        assert_eq!(p.clip(11).unwrap().asset, 1);
        assert_eq!(p.clip(12).unwrap().asset, 2);
        // the original asset is still in the library
        assert!(p.asset(1).is_some());
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn requesting_deepfilter_without_it_is_an_error_and_the_default_falls_back() {
        if deepfilter::available() {
            return;
        }
        assert_eq!(bake_engine(&json!({})), Ok(None), "ffmpeg afftdn when not installed");
        assert_eq!(bake_engine(&json!({"engine": "ffmpeg"})), Ok(None));
        assert_eq!(bake_engine(&json!({"engine": "deepfilter"})), Err(deepfilter::NOT_INSTALLED.to_string()));
        assert!(bake_engine(&json!({"engine": "nope"})).is_err());
    }

    #[test]
    fn denoise_tools_are_registered_once_with_the_right_kinds() {
        for (name, kind) in
            [("denoise.status", ToolKind::Read), ("denoise.install", ToolKind::Job), ("audio.clean", ToolKind::Job)]
        {
            let found: Vec<_> = crate::mcp::tools::all().filter(|t| t.name == name).collect();
            assert_eq!(found.len(), 1, "{name}");
            assert_eq!(found[0].kind, kind, "{name}");
        }
    }
}
