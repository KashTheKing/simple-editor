//! ---- ws:addon-demucs ----
//! The App side of Stem Separation (`engine::demucs`): the clip menu / Inspector actions (Separate Stems,
//! Separate Stems to Tracks, Remove Vocals, Keep Only Vocals), the FRAME_HOOK that lands a finished job
//! (ONE labelled undo step), and the `audio.stems` MCP tool. Ducking the vocals stem (`audio.duck stem`)
//! and beats from the drums stem (`beats.detect source`) look the stems up through `placed_stem` /
//! `engine::demucs::cached`.
//!
//! A job separates the clip's whole audio stream once into the cache; the result is linked into the
//! Library as a bin pointing at the cache folder ("Stems · <name>"). The placing modes add the four stems
//! on four new audio tracks, each a copy of the clip (timing, trims, speed, volume, fades) re-pointed at
//! its stem, the four linked together, and switch the original audio clip off (Clip ▸ Enable brings it back).

use super::feedback::{Toast, ToastKind};
use super::tools_helpers::*;
use super::*;
use crate::engine::demucs::{self, Stem};
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    /// Into the Library only.
    Bin,
    /// Four new tracks, all on.
    Tracks,
    /// Four new tracks, the vocals off.
    NoVocals,
    /// Four new tracks, only the vocals on.
    OnlyVocals,
}

impl Mode {
    fn parse(s: &str) -> Option<Mode> {
        match s.trim().to_ascii_lowercase().as_str() {
            "bin" | "library" => Some(Mode::Bin),
            "tracks" | "place" => Some(Mode::Tracks),
            "remove_vocals" | "instrumental" => Some(Mode::NoVocals),
            "keep_vocals" | "vocals" | "acapella" => Some(Mode::OnlyVocals),
            _ => None,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Mode::Bin => "Separate Stems",
            Mode::Tracks => "Separate Stems to Tracks",
            Mode::NoVocals => "Remove Vocals",
            Mode::OnlyVocals => "Keep Only Vocals",
        }
    }
    /// Which placed stems play.
    fn plays(self, s: Stem) -> bool {
        match self {
            Mode::Bin | Mode::Tracks => true,
            Mode::NoVocals => s != Stem::Vocals,
            Mode::OnlyVocals => s == Stem::Vocals,
        }
    }
}

type Probed = Arc<Mutex<Option<Result<(PathBuf, Vec<(Stem, crate::model::Asset)>), String>>>>;

/// One separation in flight. `prog` is the worker; `done` is what an MCP caller waits on (it ends once
/// the result has landed in the project).
pub(crate) struct Job {
    /// The audio clip separated.
    clip: Id,
    mode: Mode,
    name: String,
    prog: Arc<Progress>,
    done: Arc<Progress>,
    probed: Probed,
}

// ponytail: one global list, not an App field - the add-on stays out of App's shared struct
static JOBS: Mutex<Vec<Job>> = Mutex::new(Vec::new());

fn jobs() -> std::sync::MutexGuard<'static, Vec<Job>> {
    JOBS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Rows for the Jobs pane (cancel works from there).
pub(super) fn progress_jobs() -> Vec<jobs_pane::ProgJob> {
    let mut v: Vec<jobs_pane::ProgJob> = jobs()
        .iter()
        .map(|j| (crate::ui::jobs_ui::JobKind::Bake, format!("Stems · {}", j.name), j.prog.clone()))
        .collect();
    if let Some(p) = demucs::installing() {
        v.push((crate::ui::jobs_ui::JobKind::ModelDownload, "Install · Stem Separation".into(), p));
    }
    v
}

/// The audio clip a separation of `clip` works on: itself, or a video clip's linked audio.
fn target(project: &Project, clip: Id) -> Result<Id, String> {
    crate::ui::autocut_ui::audio_targets(project, &[clip])
        .first()
        .copied()
        .ok_or_else(|| "Stem separation needs an audio clip or a video clip with its audio".into())
}

/// Why `clip` (an audio clip) cannot be separated with `mode`, if it cannot.
fn refusal(project: &Project, clip: Id, mode: Mode) -> Option<String> {
    let Some(c) = project.clip(clip) else { return Some(format!("no clip {clip}")) };
    let Some(a) = project.asset(c.asset) else { return Some("the clip has no media".into()) };
    if a.html.is_some() {
        return Some("an HTML layer has no audio to separate".into());
    }
    if !Path::new(&a.path).is_file() {
        return Some(format!("{} is offline", a.path));
    }
    if mode != Mode::Bin && project.track_of(clip).is_none_or(|ti| project.locked_of(ti)) {
        return Some("the clip's track is locked".into());
    }
    None
}

/// Start separating `clip` (an audio clip, or a video clip's audio). Returns (what an MCP caller waits
/// on, the stems' folder). Cached stems land on the next frame without running anything.
pub(super) fn start(app: &mut App, clip: Id, mode: Mode) -> Result<(Arc<Progress>, PathBuf), String> {
    let clip = target(&app.project, clip)?;
    if let Some(why) = refusal(&app.project, clip, mode) {
        return Err(why);
    }
    let c = app.project.clip(clip).ok_or("no such clip")?;
    let a = app.project.asset(c.asset).ok_or("no media")?;
    let (src, stream) = (PathBuf::from(&a.path), c.audio_stream);
    if demucs::cached(&src, stream).is_none() {
        if !demucs::available() {
            return Err("Stem Separation is not installed (Settings ▸ General ▸ Stem Separation, or audio.stems action=install)".into());
        }
        if media::ffpipe::ffmpeg_exe().is_none() {
            return Err("ffmpeg.exe not found".into());
        }
        // ponytail: one at a time - two separations side by side each take half the cores anyway
        if !jobs().is_empty() {
            return Err("A stem separation is already running - try again when it finishes".into());
        }
    }
    let name = Path::new(&a.path).file_stem().unwrap_or_default().to_string_lossy().into_owned();
    let dir = demucs::stems_dir(&src, stream);
    let (backend, probed): (_, Probed) = (app.backend(), Arc::new(Mutex::new(None)));
    let slot = probed.clone();
    let prog = export::spawn_job("stems", move |p| {
        let r = demucs::run(&src, stream, p).and_then(|d| {
            let mut v = Vec::new();
            for s in Stem::ALL {
                v.push((s, media::probe(&demucs::stem_file(&d, s).to_string_lossy(), backend)?));
            }
            Ok((d, v))
        });
        let err = r.as_ref().err().cloned();
        *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
        err.map_or(Ok(()), Err)
    });
    let done = Progress::new();
    done.set(0.0, mode.label());
    jobs().push(Job { clip, mode, name, prog, done: done.clone(), probed });
    Ok((done, dir))
}

/// FRAME_HOOKS: mirror progress for MCP waiters; land finished jobs.
pub(super) fn tick(app: &mut App, ctx: &egui::Context) {
    let finished: Vec<Job> = {
        let mut list = jobs();
        if list.is_empty() {
            return;
        }
        for j in list.iter() {
            j.done.set(j.prog.fraction(), j.prog.status());
        }
        let (done, pending): (Vec<Job>, Vec<Job>) = std::mem::take(&mut *list).into_iter().partition(|j| j.prog.is_done());
        *list = pending;
        done
    };
    for j in finished {
        let r = match j.prog.error() {
            Some(_) if j.prog.is_cancelled() => Err("Stem separation cancelled".to_string()),
            Some(e) => Err(format!("Stem separation failed: {e}")),
            None => finish(app, &j),
        };
        match r {
            Ok((msg, dir)) => {
                app.push_toast(Toast::with_folder(msg, dir).kind(ToastKind::Success).undo(Action::Undo));
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

/// Link the stems' folder as a bin; the placing modes add them under the clip. One labelled undo step.
fn finish(app: &mut App, j: &Job) -> Result<(String, PathBuf), String> {
    let (dir, assets) = j.probed.lock().unwrap_or_else(|e| e.into_inner()).take().unwrap_or_else(|| Err("probe crashed".into()))?;
    let before = app.project.to_json();
    let bin = app.project.link_bin(&dir.to_string_lossy(), Some(&format!("Stems · {}", j.name)));
    let mut msg = format!("Stems of {} are in the \"{bin}\" bin", j.name);
    if j.mode != Mode::Bin {
        let ids: Vec<(Stem, Id)> = assets.into_iter().map(|(s, a)| (s, app.project.add_asset(a))).collect();
        if place(&mut app.project, j.clip, &ids, j.mode).is_empty() {
            msg.push_str(" (the clip is gone, so nothing was placed)");
        } else {
            msg = format!("{}: {} on 4 new tracks, the original audio is off", j.mode.label(), j.name);
        }
    }
    app.push_undo_labeled(before, j.mode.label());
    app.after_edit();
    Ok((msg, dir))
}

/// Put each stem on a new audio track as a copy of `clip` re-pointed at it (the four linked together, the
/// ones `mode` mutes switched off) and switch `clip` off. Returns the new clips (empty: `clip` is gone).
pub(crate) fn place(p: &mut Project, clip: Id, stems: &[(Stem, Id)], mode: Mode) -> Vec<Id> {
    let Some(orig) = p.clip(clip).cloned() else { return Vec::new() };
    let link = p.new_id();
    let mut out = Vec::new();
    // right under the clip's own track, in stem order (add_track appends; nothing before `at` moves)
    let mut at = p.track_of(clip).map_or(usize::MAX, |ti| ti + 1);
    for &(s, asset) in stems {
        let added = p.add_track(TrackKind::Audio);
        let mut track = p.tracks.remove(added);
        track.name = s.label().into();
        let ti = at.min(p.tracks.len());
        p.tracks.insert(ti, track);
        at = ti + 1;
        let mut c = orig.clone();
        c.id = p.new_id();
        c.asset = asset;
        c.audio_stream = 0;
        c.link = link;
        c.name = format!("{} · {}", orig.name, s.label());
        c.enabled = mode.plays(s);
        p.tracks[ti].clips.push(c.clone());
        out.push(c.id);
    }
    if let Some(c) = p.clip_mut(clip) {
        c.enabled = false;
    }
    out
}

/// The placed `stem` of `clip`'s audio: a clip on the timeline playing that stem file, starting where
/// `clip` does. `clip` may be the original (or a video with it linked) or any of its stems.
pub(crate) fn placed_stem(p: &Project, clip: Id, stem: Stem) -> Option<Id> {
    let c = p.clip(target(p, clip).ok()?)?;
    let start = c.start;
    let path = p.asset(c.asset)?.path.clone();
    // the clip may itself be a stem: its folder is the stems' folder
    let dir = Path::new(&path)
        .parent()
        .filter(|d| d.parent() == Some(demucs::cache_root().as_path()))
        .map(Path::to_path_buf)
        .unwrap_or_else(|| demucs::stems_dir(Path::new(&path), c.audio_stream));
    let want = demucs::stem_file(&dir, stem).to_string_lossy().to_lowercase();
    p.all_clips()
        .filter(|(_, x)| (x.start - start).abs() < 1e-3)
        .find(|(_, x)| p.asset(x.asset).is_some_and(|a| a.path.to_lowercase() == want))
        .map(|(_, x)| x.id)
}

/// What AutoDuck lowers for the music pick `clip`: its placed vocals stem when the song was split onto tracks
/// (the instrumental stems keep playing), else None (duck the clip itself, as before).
pub(crate) fn duck_target(p: &Project, clip: Id) -> Option<Id> {
    placed_stem(p, clip, Stem::Vocals).filter(|&v| v != clip)
}

/// The file the Beat Aligner reads for the song `path`: its cached drums stem once separated (a cleaner
/// beat grid on a busy mix), else the song itself.
pub(crate) fn beat_source(path: &str) -> String {
    cached_stem(path, Stem::Drums).map_or_else(|| path.to_string(), |f| f.to_string_lossy().into_owned())
}

/// The cached stem file of the song asset `path` (stream 0), if it has been separated.
pub(crate) fn cached_stem(path: &str, stem: Stem) -> Option<PathBuf> {
    demucs::cached(Path::new(path), 0).map(|d| demucs::stem_file(&d, stem))
}

fn action_mode(a: Action) -> Option<Mode> {
    match a {
        Action::SeparateStems => Some(Mode::Bin),
        Action::SeparateStemsToTracks => Some(Mode::Tracks),
        Action::RemoveVocals => Some(Mode::NoVocals),
        Action::KeepOnlyVocals => Some(Mode::OnlyVocals),
        _ => None,
    }
}

/// Install offer for a click before the add-on is there.
fn offer_install() {
    crate::ui::confirm::ask_app(
        "Install Stem Separation?",
        "Separate Stems uses Demucs (htdemucs), an optional add-on (about 85 MB, runs on the processor).\n\
         Download and install it now? Nothing else is sent anywhere. Uninstall any time in Settings ▸ General.",
        |app| {
            demucs::start_install();
            app.toast("Installing Stem Separation - see Jobs; run it again when it is done");
        },
    );
}

/// ACT_HANDLERS: the four stem actions run on the first selected clip with audio.
pub(super) fn act(app: &mut App, a: Action) -> bool {
    let Some(mode) = action_mode(a) else { return false };
    let clip = app.selection.iter().copied().find(|&id| target(&app.project, id).is_ok());
    let Some(clip) = clip else {
        app.toast("Select an audio clip or a video clip with its audio");
        return true;
    };
    let cached = target(&app.project, clip)
        .ok()
        .and_then(|t| app.project.clip(t))
        .and_then(|c| Some((app.project.asset(c.asset)?.path.clone(), c.audio_stream)))
        .is_some_and(|(p, s)| demucs::cached(Path::new(&p), s).is_some());
    if !cached && !demucs::available() {
        offer_install();
        return true;
    }
    match start(app, clip, mode) {
        Ok(_) if cached => {}
        Ok(_) => app.toast(format!("{} started (see Jobs)", mode.label())),
        Err(e) => app.toast(e),
    }
    true
}

fn status_json() -> Value {
    let list: Vec<Value> = jobs()
        .iter()
        .map(|j| json!({"clip": j.clip, "mode": j.mode.label(), "fraction": j.prog.fraction(), "status": j.prog.status()}))
        .collect();
    json!({
        "installed": demucs::available(),
        "path": demucs::exe().map(|p| p.display().to_string()),
        "version": demucs::version(),
        "threads": demucs::threads(),
        "installing": demucs::installing().map(|p| json!({"fraction": p.fraction(), "status": p.status()})),
        "jobs": list,
        "model": "htdemucs (Demucs v4, 4 sources: drums, bass, other, vocals)",
    })
}

/// The stems of `clip` (cached files, and the placed clips).
fn list_json(p: &Project, clip: Id) -> Result<Value, String> {
    let t = target(p, clip)?;
    let c = p.clip(t).ok_or("no such clip")?;
    let a = p.asset(c.asset).ok_or("no media")?;
    let dir = demucs::cached(Path::new(&a.path), c.audio_stream);
    let stems: Vec<Value> = Stem::ALL
        .iter()
        .map(|&s| {
            json!({
                "stem": s.id(),
                "file": dir.as_ref().map(|d| demucs::stem_file(d, s).to_string_lossy().into_owned()),
                "clip": placed_stem(p, t, s),
            })
        })
        .collect();
    Ok(json!({"clip": t, "separated": dir.is_some(), "folder": dir.map(|d| d.to_string_lossy().into_owned()), "stems": stems}))
}

pub(crate) const TOOLS: &[ToolDef] = &[ToolDef {
    name: "audio.stems",
    desc: "Stem separation (Demucs htdemucs, an optional add-on): action=separate splits a clip's audio (a video clip's linked audio) into drums, bass, other and vocals (job; cached in the app's cache, linked into the Library as a \"Stems · <name>\" bin); mode=tracks also places them on 4 new audio tracks aligned under the clip and switches the original audio off, remove_vocals / keep_vocals do the same with the vocals / the rest off (one undo step). action=list: a clip's stem files and placed stem clips. action=status: installed, version, running jobs. action=install / uninstall: the add-on (~85 MB download, SHA-256 checked).",
    args: &[
        "action:string:true:separate | list | status | install | uninstall",
        "clip:integer:false:separate / list: an audio clip or a video clip with linked audio",
        "mode:string:false:separate: bin (default, Library only) | tracks | remove_vocals | keep_vocals",
    ],
    kind: ToolKind::Job,
    run: |app, args| match arg_str(args, "action").unwrap_or("") {
        "status" => Ok(ToolOutcome::Done(status_json())),
        "list" => Ok(ToolOutcome::Done(list_json(&app.project, req(arg_u64(args, "clip"), "clip")?)?)),
        "install" => {
            if demucs::available() {
                return Ok(ToolOutcome::Done(status_json()));
            }
            Ok(ToolOutcome::Job(demucs::start_install(), demucs::dir().join(demucs::EXE)))
        }
        "uninstall" => {
            demucs::uninstall()?;
            Ok(ToolOutcome::Done(json!({"ok": true, "installed": demucs::available()})))
        }
        "separate" => {
            let clip = req(arg_u64(args, "clip"), "clip")?;
            let mode = match arg_str(args, "mode") {
                Some(m) => Mode::parse(m).ok_or("mode: bin, tracks, remove_vocals or keep_vocals")?,
                None => Mode::Bin,
            };
            let (done, dir) = start(app, clip, mode)?;
            Ok(ToolOutcome::Job(done, dir))
        }
        _ => Err("action: separate, list, status, install or uninstall".into()),
    },
}];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Asset, Clip};

    fn asset(id: Id, path: &str) -> Asset {
        Asset {
            id,
            path: path.into(),
            kind: ClipKind::Audio,
            duration: 180.0,
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

    /// A song clip (id 10, asset 1, 3..9 s of source at timeline 2 s) and four stem assets 11..14.
    fn project() -> (Project, Vec<(Stem, Id)>) {
        let mut p = Project::new();
        let song = std::env::temp_dir().join("se-stems-song.wav").to_string_lossy().into_owned();
        p.assets.push(asset(1, &song));
        let dir = demucs::stems_dir(Path::new(&song), 0);
        let ids: Vec<(Stem, Id)> = Stem::ALL
            .iter()
            .enumerate()
            .map(|(i, &s)| {
                let id = 11 + i as Id;
                p.assets.push(asset(id, &demucs::stem_file(&dir, s).to_string_lossy()));
                (s, id)
            })
            .collect();
        let ti = p.add_track(TrackKind::Audio);
        let mut c = Clip::new(10, ClipKind::Audio, "song", 2.0, 6.0);
        c.asset = 1;
        c.src_in = 3.0;
        c.speed = 1.0;
        p.tracks[ti].clips.push(c);
        (p, ids)
    }

    #[test]
    fn placing_aligns_four_linked_copies_and_mutes_the_original() {
        let (mut p, ids) = project();
        let below = p.add_track(TrackKind::Audio); // a track the stems must go above
        p.tracks[below].name = "Music".into();
        let tracks = p.tracks.len();
        let song_track = p.track_of(10).unwrap();
        let new = place(&mut p, 10, &ids, Mode::Tracks);
        assert_eq!((new.len(), p.tracks.len()), (4, tracks + 4));
        // the four stem tracks sit right under the song's track, in stem order, named after the stem
        let names: Vec<&str> = p.tracks[song_track + 1..song_track + 5].iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["Drums", "Bass", "Other", "Vocals"]);
        assert_eq!(p.tracks.last().unwrap().name, "Music");
        for (k, &id) in new.iter().enumerate() {
            assert_eq!(p.track_of(id), Some(song_track + 1 + k));
        }
        let link = p.clip(new[0]).unwrap().link;
        assert_ne!(link, 0);
        for (&id, &(s, asset)) in new.iter().zip(&ids) {
            let c = p.clip(id).unwrap();
            assert_eq!((c.start, c.duration, c.src_in, c.asset, c.link), (2.0, 6.0, 3.0, asset, link));
            assert!(c.enabled && c.name.ends_with(s.label()));
        }
        assert!(!p.clip(10).unwrap().enabled);
        assert_eq!(placed_stem(&p, 10, Stem::Drums), Some(new[0]));
        // from a stem clip too
        assert_eq!(placed_stem(&p, new[0], Stem::Vocals), Some(new[3]));
        assert_eq!(place(&mut p, 999, &ids, Mode::Tracks), Vec::<Id>::new());
    }

    #[test]
    fn remove_and_keep_vocals_switch_the_right_stems() {
        let (mut p, ids) = project();
        let new = place(&mut p, 10, &ids, Mode::NoVocals);
        let on: Vec<bool> = new.iter().map(|&id| p.clip(id).unwrap().enabled).collect();
        assert_eq!(on, [true, true, true, false]);
        let (mut p, ids) = project();
        let new = place(&mut p, 10, &ids, Mode::OnlyVocals);
        let on: Vec<bool> = new.iter().map(|&id| p.clip(id).unwrap().enabled).collect();
        assert_eq!(on, [false, false, false, true]);
    }

    #[test]
    fn autoduck_lowers_only_the_placed_vocals() {
        let (mut p, ids) = project();
        assert_eq!(duck_target(&p, 10), None); // not split: duck the song itself
        let new = place(&mut p, 10, &ids, Mode::Tracks);
        assert_eq!(duck_target(&p, 10), Some(new[3]));
        assert_eq!(duck_target(&p, new[0]), Some(new[3])); // picked the drums stem: still the vocals
        assert_eq!(duck_target(&p, new[3]), None); // picked the vocals stem: that clip itself
    }

    #[test]
    fn beats_read_the_drums_stem_once_separated() {
        let song = std::env::temp_dir().join(format!("se-stems-beats-{}.wav", std::process::id()));
        std::fs::write(&song, b"song").unwrap();
        let path = song.to_string_lossy().into_owned();
        assert_eq!(beat_source(&path), path);
        let dir = demucs::stems_dir(&song, 0);
        std::fs::create_dir_all(&dir).unwrap();
        for s in Stem::ALL {
            std::fs::write(demucs::stem_file(&dir, s), b"x").unwrap();
        }
        assert_eq!(beat_source(&path), demucs::stem_file(&dir, Stem::Drums).to_string_lossy());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(&song);
    }

    #[test]
    fn nothing_placed_means_no_stem_found() {
        let (p, _) = project();
        assert_eq!(placed_stem(&p, 10, Stem::Vocals), None);
        assert!(target(&p, 999).is_err());
    }

    #[test]
    fn offline_and_locked_are_refused_and_modes_parse() {
        let (mut p, _) = project();
        assert!(refusal(&p, 10, Mode::Bin).unwrap().contains("offline"));
        let f = std::env::temp_dir().join(format!("se-stems-src-{}.wav", std::process::id()));
        std::fs::write(&f, b"x").unwrap();
        p.assets[0].path = f.to_string_lossy().into_owned();
        assert_eq!(refusal(&p, 10, Mode::Tracks), None);
        let ti = p.track_of(10).unwrap();
        p.tracks[ti].locked = true;
        assert!(refusal(&p, 10, Mode::Tracks).unwrap().contains("locked"));
        assert_eq!(refusal(&p, 10, Mode::Bin), None);
        let _ = std::fs::remove_file(&f);
        assert_eq!(Mode::parse("remove_vocals"), Some(Mode::NoVocals));
        assert_eq!(Mode::parse("keep_vocals"), Some(Mode::OnlyVocals));
        assert_eq!(Mode::parse("x"), None);
        for a in [Action::SeparateStems, Action::SeparateStemsToTracks, Action::RemoveVocals, Action::KeepOnlyVocals] {
            assert!(action_mode(a).is_some());
        }
    }

    #[test]
    fn stems_tool_is_registered_once_and_refuses_when_not_installed() {
        let found: Vec<_> = crate::mcp::tools::all().filter(|t| t.name == "audio.stems").collect();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, ToolKind::Job);
        let s = status_json();
        assert_eq!(s["installed"], demucs::available());
    }
}
