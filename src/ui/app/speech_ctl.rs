//! ---- ws:addon-voice ----
//! App glue for text to speech beyond the job itself (`transcript_ctl::speak` runs the queue): the
//! Speech section of the Voiceover pane (`ui::speech_ui`), voice lists listed off the UI thread, the Play
//! preview (the Gallery's sound preview: the default output device, refused while the Main bus is muted),
//! voice cloning into `%APPDATA%\SimpleEditor\voices`, Speak at playhead / Speak selection, the add-on
//! installs (Settings ▸ General and the Jobs pane), and the `tts.voices` / `tts.install` / `tts.status` tools.

use super::tools_helpers::*;
use super::*;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::engine::tts::{self, Engine, Speech, Voice};
use crate::engine::tts_addon::{self, Addon};
use crate::ui::speech_ui::{self, SpeechUi, SpeechView};

#[derive(Default)]
pub(super) struct SpeechState {
    pub(super) ui: SpeechUi,
    /// Voice lists by engine, filled off the UI thread (SAPI takes about a second of powershell).
    voices: std::collections::HashMap<&'static str, Vec<Voice>>,
    voices_rx: Option<(Engine, Receiver<Vec<Voice>>)>,
    /// The Play preview being synthesised: (job, WAV).
    preview: Option<(Arc<Progress>, tts::ProbedWav)>,
    /// A voice being cloned: (job, name).
    cloning: Option<(Arc<Progress>, String)>,
    /// Add-ons seen installing last frame: one that finishes becomes the engine when Windows was picked
    /// (Kokoro is the default natural voice once it is there).
    was_installing: [bool; 2],
}

/// Install rows for the Jobs pane.
pub(super) fn install_jobs() -> Vec<jobs_pane::ProgJob> {
    Addon::ALL
        .iter()
        .filter_map(|&a| {
            let name = if a == Addon::Kokoro { "Kokoro voices" } else { "Chatterbox voices" };
            tts_addon::installing(a).map(|p| (crate::ui::jobs_ui::JobKind::ModelDownload, format!("Install · {name}"), p))
        })
        .collect()
}

/// The speech settings as a request for `text` (engine, the voice picked for it, speed, pitch, intensity).
pub(super) fn speech_from_settings(s: &Settings, text: &str) -> Speech {
    let mut sp = Speech::new(text);
    sp.engine = s.tts_engine;
    sp.voice = s.tts_voices.get(s.tts_engine.id()).filter(|v| !v.is_empty()).cloned();
    sp.speed = s.tts_speed;
    sp.pitch = s.tts_pitch;
    sp.intensity = s.tts_intensity;
    sp
}

/// The voices of `engine`, starting a listing when there is none yet (never blocks).
fn voices_of(app: &mut App, engine: Engine) -> Option<Vec<Voice>> {
    if let Some(v) = app.speech.voices.get(engine.id()) {
        return Some(v.clone());
    }
    if app.speech.voices_rx.as_ref().is_none_or(|(e, _)| *e != engine) {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(tts::voice_list(engine));
        });
        app.speech.voices_rx = Some((engine, rx));
    }
    None
}

fn muted(p: &Project) -> bool {
    p.buses.first().is_some_and(|b| b.muted)
}

/// What a clone would copy: the selected Library item, else a selected clip with sound, else the last
/// voiceover take. (path, start seconds, shown name).
fn clone_source(app: &App) -> Option<(PathBuf, f64, String)> {
    let named = |path: &str| Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if let Some(a) = app.library.selected.and_then(|id| app.project.asset(id)).filter(|a| !a.audio_streams.is_empty()) {
        return Some((PathBuf::from(&a.path), a.range.map(|r| r.0).unwrap_or(0.0), named(&a.path)));
    }
    for &id in &app.selection {
        let Some(c) = app.project.clip(id) else { continue };
        if let Some(a) = app.project.asset(c.asset).filter(|a| !a.audio_streams.is_empty()) {
            return Some((PathBuf::from(&a.path), c.src_in, named(&a.path)));
        }
    }
    app.capture_ui.last_take.as_ref().map(|(_, p)| (p.clone(), 0.0, named(&p.to_string_lossy())))
}

/// What "Speak selection" speaks: the selected text clips (title cards, plan cards) at their start,
/// linked to them; else the selected transcript words where they are. (text, at, link_to)
pub(super) fn selection_items(app: &App) -> Vec<(String, f64, Option<Id>)> {
    let mut v: Vec<(String, f64, Option<Id>)> = app
        .selection
        .iter()
        .filter_map(|&id| app.project.clip(id))
        .filter_map(|c| {
            let t = c.text.as_ref()?.text.trim().to_string();
            (!t.is_empty()).then_some((t, c.start, Some(c.id)))
        })
        .collect();
    if v.is_empty() {
        let tr = &app.subtitles_ui.transcript;
        if let (Some(clip), Some((a, b))) = (tr.clip, tr.sel) {
            if let Some(w) = app.project.transcript(clip).and_then(|t| t.words.get(a.min(b)..=b.max(a))) {
                let text = w.iter().map(|w| w.2.as_str()).collect::<Vec<_>>().join(" ");
                if let Some(first) = w.first() {
                    v.push((text, first.0, None));
                }
            }
        }
    }
    v.sort_by(|a, b| a.1.total_cmp(&b.1));
    v
}

/// The Speech section, drawn under the Voiceover pane's takes.
pub(super) fn draw(app: &mut App, ui: &mut egui::Ui) {
    let engine = app.settings.tts_engine;
    let voices = voices_of(app, engine);
    let addon = engine.addon();
    let source = clone_source(app);
    let view = SpeechView {
        voices: voices.as_deref(),
        installed: [Addon::Kokoro.available(), Addon::Chatterbox.available()],
        installing: addon.and_then(tts_addon::installing).map(|p| (p.fraction(), p.status())),
        install_error: addon.and_then(tts_addon::install_error),
        download: addon.map(|a| a.manifest().download_size()).unwrap_or(0),
        previewing: app.speech.preview.is_some(),
        muted: muted(&app.project),
        selection: selection_items(app).len(),
        clone_source: source.as_ref().map(|s| s.2.clone()),
    };
    let r = speech_ui::show(ui, &mut app.speech.ui, &mut app.settings, &view);
    if r.changed {
        app.settings.save();
    }
    if r.install {
        if let Some(a) = addon {
            tts_addon::start_install(a);
        }
    }
    if r.preview {
        preview(app);
    }
    if let Some(name) = r.remove_clone {
        match tts::remove_clone(&name) {
            Ok(()) => {
                app.settings.tts_voices.insert(Engine::Chatterbox.id().into(), String::new());
                app.settings.save();
                app.speech.voices.remove(Engine::Chatterbox.id());
            }
            Err(e) => app.toast(e),
        }
    }
    if r.clone && app.speech.ui.consent {
        app.speech.ui.consent = false;
        if let Some((src, start, _)) = source {
            let name = tts::voice_file_name(&app.speech.ui.clone_name);
            let n = name.clone();
            let p = export::spawn_job("tts-clone", move |prog| {
                prog.set(0.3, "Saving the voice…");
                tts::save_clone_ref(&src, start, &n).map(|_| ())
            });
            app.speech.cloning = Some((p, name));
        }
    }
    if r.speak_script {
        speak_script(app);
    }
    if r.speak_selection {
        speak_selection(app);
    }
}

fn preview(app: &mut App) {
    if muted(&app.project) {
        return;
    }
    let sp = speech_from_settings(&app.settings, "Hello! This is how this voice sounds.");
    let out = Settings::cache_dir().join("tts").join(format!("preview-{}", sp.cache_name()));
    let (p, probed) = tts::speak_to_wav(&sp, &out, app.backend());
    app.speech.preview = Some((p, probed));
}

/// Speak at playhead: one clip per script line; timed lines at their time, the rest one after another.
fn speak_script(app: &mut App) {
    let lines = speech_ui::script_lines(&app.speech.ui.script);
    let group = app.transcript_new_group(app.playhead);
    for (t, text) in lines {
        let sp = speech_from_settings(&app.settings, &text);
        let place = match t {
            Some(t) => transcript_ctl::Place::At(t),
            None => transcript_ctl::Place::After(group),
        };
        app.queue_speech(sp, None, place);
    }
}

fn speak_selection(app: &mut App) {
    for (text, at, link) in selection_items(app) {
        let sp = speech_from_settings(&app.settings, &text);
        app.queue_speech(sp, link, transcript_ctl::Place::At(at));
    }
}

/// FRAME_HOOK: voice listings, the preview and clone jobs.
pub(super) fn tick(app: &mut App, ctx: &egui::Context) {
    let got = app.speech.voices_rx.as_ref().and_then(|(e, rx)| rx.try_recv().ok().map(|v| (*e, v)));
    if let Some((e, v)) = got {
        app.speech.voices.insert(e.id(), v);
        app.speech.voices_rx = None;
    }
    if app.speech.preview.as_ref().is_some_and(|(p, _)| p.is_done()) {
        if let Some((p, probed)) = app.speech.preview.take() {
            match (p.error(), super::tools_export::take_probed(&probed)) {
                (Some(e), _) | (None, Err(e)) => app.toast(format!("Preview failed: {e}")),
                (None, Ok(a)) => {
                    // the click asked for it; the Main bus may have been muted since
                    if !muted(&app.project) {
                        super::gallery_mine::preview::play(Path::new(&a.path));
                    }
                    let status = p.status();
                    if status.contains("not installed") {
                        app.toast(status);
                    }
                }
            }
        }
    }
    if app.speech.cloning.as_ref().is_some_and(|(p, _)| p.is_done()) {
        if let Some((p, name)) = app.speech.cloning.take() {
            match p.error() {
                Some(e) => app.toast(format!("Clone failed: {e}")),
                None => {
                    app.settings.tts_voices.insert(Engine::Chatterbox.id().into(), name.clone());
                    app.settings.save();
                    app.speech.voices.remove(Engine::Chatterbox.id());
                    app.speech.ui.clone_name.clear();
                    app.toast(format!("Voice \"{name}\" saved"));
                }
            }
        }
    }
    for (i, a) in Addon::ALL.into_iter().enumerate() {
        let now = tts_addon::installing(a).is_some();
        if std::mem::replace(&mut app.speech.was_installing[i], now) && !now && a.available() {
            if let Some(e) = adopt_engine(app.settings.tts_engine, a) {
                app.settings.tts_engine = e;
                app.settings.save();
            }
            app.toast(format!("{} voices installed", Engine::ALL[i + 1].label()));
        }
    }
    let busy = app.speech.voices_rx.is_some()
        || app.speech.preview.is_some()
        || app.speech.cloning.is_some()
        || Addon::ALL.iter().any(|&a| tts_addon::installing(a).is_some());
    if busy {
        ctx.request_repaint_after(Duration::from_millis(200));
    }
}

/// The engine to switch to once `installed` finishes installing: only away from Windows (a user who
/// already picked an add-on keeps it).
fn adopt_engine(current: Engine, installed: Addon) -> Option<Engine> {
    let e = Engine::ALL.into_iter().find(|e| e.addon() == Some(installed))?;
    (current == Engine::Windows).then_some(e)
}

/// Settings ▸ General: Install / Remove and the size of one add-on.
pub(crate) fn addon_row(ui: &mut egui::Ui, addon: Addon) {
    if let Some(p) = tts_addon::installing(addon) {
        ui.add(egui::Spinner::new());
        ui.weak(format!("{:.0}% · {}", p.fraction() * 100.0, p.status()));
        if ui.small_button("Cancel").clicked() {
            p.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        return;
    }
    let id = egui::Id::new(("tts_addon_row", addon.id()));
    if addon.available() {
        // the size walks the folder: once per Settings visit, kept in egui's temp data
        let size = ui.data_mut(|d| d.get_temp::<u64>(id.with("size"))).unwrap_or_else(|| {
            let s = addon.disk_size();
            ui.data_mut(|d| d.insert_temp(id.with("size"), s));
            s
        });
        ui.weak(format!("installed, {:.0} MB", size as f64 / 1048576.0));
        if ui.button("Remove").on_hover_text("Deletes the add-on; speech it already made stays").clicked() {
            let r = tts_addon::uninstall(addon).err();
            ui.data_mut(|d| {
                d.insert_temp(id, r);
                d.remove::<u64>(id.with("size"));
            });
        }
        if let Some(e) = ui.data(|d| d.get_temp::<Option<String>>(id)).flatten() {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
    } else {
        let m = addon.manifest();
        let mb = m.download_size() as f64 / 1048576.0;
        let tip = match addon {
            Addon::Kokoro => "Natural voices for Speak in the Voiceover pane, on the processor. Downloaded once.",
            Addon::Chatterbox => "Expressive voices and voice cloning for Speak in the Voiceover pane, fastest with a graphics card. Downloaded once.",
        };
        if ui.button("Install").on_hover_text(format!("{tip}\n{}", m.license)).clicked() {
            tts_addon::start_install(addon);
        }
        match tts_addon::install_error(addon) {
            Some(e) => ui.colored_label(ui.visuals().error_fg_color, format!("install failed: {e}")),
            None => ui.weak(format!("{mb:.0} MB")),
        };
    }
}

fn engine_arg(args: &Value) -> Result<Option<Engine>, String> {
    match arg_str(args, "engine") {
        None => Ok(None),
        Some(s) => Engine::parse(s).map(Some).ok_or_else(|| format!("unknown engine '{s}' (windows | kokoro | chatterbox)")),
    }
}

fn status_json(a: Addon) -> Value {
    json!({
        "engine": a.id(),
        "installed": a.available(),
        "installing": tts_addon::installing(a).map(|p| json!({"fraction": p.fraction(), "status": p.status()})),
        "error": tts_addon::install_error(a),
        "download_bytes": a.manifest().download_size(),
        "license": a.manifest().license,
    })
}

pub(super) const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "tts.voices",
        desc: "Voices for text to speech: engine windows | kokoro | chatterbox (default: the one picked in the Voiceover pane). Returns [{id, label}]; pass an id as tts.speak's voice. Chatterbox lists Default plus the voices the user cloned.",
        args: &["engine:string:false:windows|kokoro|chatterbox"],
        kind: ToolKind::Read,
        run: |app, args| {
            let engine = engine_arg(args)?.unwrap_or(app.settings.tts_engine);
            // Windows lists through powershell (about a second, cached after the first call)
            let voices = tts::voice_list(engine);
            Ok(ToolOutcome::Done(json!({"engine": engine.id(), "installed": engine.available(), "voices": voices})))
        },
    },
    ToolDef {
        name: "tts.status",
        desc: "Text to speech add-ons: installed, install progress, download size and licences of Kokoro and Chatterbox.",
        args: &[],
        kind: ToolKind::Read,
        run: |_, _| Ok(ToolOutcome::Done(json!({"addons": Addon::ALL.iter().map(|&a| status_json(a)).collect::<Vec<_>>()}))),
    },
    ToolDef {
        name: "tts.install",
        desc: "Install a voice add-on (job): engine kokoro (about 160 MB) or chatterbox (about 1.6 GB). One download at a time; say the size to the user before you start it.",
        args: &["engine:string:true:kokoro|chatterbox"],
        kind: ToolKind::Job,
        run: |_, args| {
            let addon = engine_arg(args)?.and_then(Engine::addon).ok_or("engine must be kokoro or chatterbox")?;
            Ok(ToolOutcome::Job(tts_addon::start_install(addon), PathBuf::new()))
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_become_the_request() {
        let mut s = Settings::default();
        s.tts_engine = Engine::Kokoro;
        s.tts_voices.insert("kokoro".into(), "bm_george".into());
        s.tts_voices.insert("windows".into(), "Zira".into());
        s.tts_speed = 1.2;
        let sp = speech_from_settings(&s, "Hi");
        assert_eq!((sp.engine, sp.voice.as_deref(), sp.speed, sp.text.as_str()), (Engine::Kokoro, Some("bm_george"), 1.2, "Hi"));
        s.tts_engine = Engine::Chatterbox;
        assert_eq!(speech_from_settings(&s, "Hi").voice, None, "no voice picked = the model's default");
        assert!(engine_arg(&json!({"engine": "nope"})).is_err());
        assert_eq!(engine_arg(&json!({"engine": "Kokoro"})), Ok(Some(Engine::Kokoro)));
        assert_eq!(engine_arg(&json!({})), Ok(None));
    }

    #[test]
    fn a_finished_install_becomes_the_engine_only_from_windows() {
        assert_eq!(adopt_engine(Engine::Windows, Addon::Kokoro), Some(Engine::Kokoro));
        assert_eq!(adopt_engine(Engine::Windows, Addon::Chatterbox), Some(Engine::Chatterbox));
        assert_eq!(adopt_engine(Engine::Chatterbox, Addon::Kokoro), None, "an add-on the user picked stays");
    }
}
