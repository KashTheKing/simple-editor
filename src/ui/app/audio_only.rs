//! ---- ws:preview-audio-only ----
//! Preview ▸ Audio Only (`Project::audio_only_preview`): playback draws no picture (`playback::played_view`
//! empties the picture tracks on the render thread, so nothing is decoded or composited), the sound plays
//! as usual, and the Preview shows a visualizer of the whole mix in the last-used style (Neon Orb by
//! default) with an "Audio Only" badge. The visualizer reads a whole-mix analysis made once in the
//! background after each edit (`SongAnalysis` of the mixed timeline, the same analysis a visualizer clip
//! is baked from) at the playhead; until that lands it follows the Main bus meter. Export is unaffected.

use super::tools_helpers::*;
use super::*;
use crate::engine::visualizer::{self as viz, SongAnalysis};
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// `visualizer::mix_key` of the project as last edited: an analysis made at another key is stale.
static KEY: AtomicU64 = AtomicU64::new(0);
/// The timeline-mix visualizers' (id, start, length), hashed: one added, moved or trimmed is baked again.
static VIS_KEY: AtomicU64 = AtomicU64::new(0);
/// The (KEY, VIS_KEY) the timeline-mix visualizers were last baked at.
static BAKED: Mutex<(u64, u64)> = Mutex::new((0, 0));
/// The project has a visualizer listening to the timeline mix (set by `edited`, read every frame).
static HAS_VIS: AtomicBool = AtomicBool::new(false);
/// A whole-mix analysis is being made.
static RUNNING: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// The visualizer drawn into the Preview, and the style it draws: (name, shader source).
    static ORB: std::cell::RefCell<(crate::ui::spiky_ball::Orb, String, String)> = Default::default();
    /// When the mix last changed (the re-analysis waits for edits to settle).
    static EDITED_AT: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) };
}

/// The project changed (an edit, undo, a project opened): note what the mix and the timeline-mix
/// visualizers depend on, so the frame hook analyses / bakes again when they changed.
pub(super) fn edited(p: &Project) {
    let key = viz::mix_key(p);
    if KEY.swap(key, Ordering::Relaxed) != key {
        EDITED_AT.with(|e| e.set(Some(Instant::now())));
    }
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let ids = viz::mix_visualizers(p);
    HAS_VIS.store(!ids.is_empty(), Ordering::Relaxed);
    for id in ids {
        if let Some(c) = p.clip(id) {
            (id, c.start.to_bits(), c.duration.to_bits()).hash(&mut h);
        }
    }
    VIS_KEY.store(h.finish(), Ordering::Relaxed);
}

/// Something needs the mix's analysis: Audio Only, or a visualizer listening to the timeline mix.
fn needed(app: &App) -> bool {
    app.project.audio_only_preview || HAS_VIS.load(Ordering::Relaxed)
}

/// Start a whole-mix analysis in the background when the cached one is stale and edits have settled for
/// half a second. Never blocks: the visualizers keep the last one (or the meters) meanwhile.
fn refresh(app: &App, ctx: &egui::Context) {
    let key = KEY.load(Ordering::Relaxed);
    if viz::mix_cached().is_some_and(|(k, _)| k == key) || app.project.duration() <= 0.0 {
        return;
    }
    if EDITED_AT.with(|e| e.get()).is_some_and(|t| t.elapsed() < Duration::from_millis(500)) {
        ctx.request_repaint_after(Duration::from_millis(500));
        return;
    }
    if RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }
    let (project, backend, ctx) = (app.project.clone(), app.backend(), ctx.clone());
    std::thread::spawn(move || {
        crate::media::bg::lower_thread();
        let mut src = viz::MixSource::new(project, backend);
        let a = guarded(|| SongAnalysis::compute(&mut src)).unwrap_or_default();
        viz::set_mix(key, Arc::new(a));
        RUNNING.store(false, Ordering::Release);
        ctx.request_repaint();
    });
}

/// Before an export: when a Timeline-mix visualizer's bake is behind the last edit, analyse the mix here
/// and bake now (the export is about to read every key; it cannot wait for the background run).
pub(super) fn bake_now(app: &mut App) {
    if !HAS_VIS.load(Ordering::Relaxed) {
        return;
    }
    let (key, vis_key) = (KEY.load(Ordering::Relaxed), VIS_KEY.load(Ordering::Relaxed));
    let mix = match viz::mix_cached() {
        Some((k, m)) if k == key => m,
        _ => {
            let mut src = viz::MixSource::new(app.project.clone(), app.backend());
            let m = Arc::new(guarded(|| SongAnalysis::compute(&mut src)).unwrap_or_default());
            viz::set_mix(key, m.clone());
            m
        }
    };
    for id in viz::mix_visualizers(&app.project) {
        let _ = viz::bake_mix(&mut app.project, id, &mix);
    }
    *BAKED.lock().unwrap_or_else(|e| e.into_inner()) = (key, vis_key);
    app.player.set_project(&app.project);
}

/// FRAME_HOOKS entry: keep the mix analysis fresh while something needs it, and bake the timeline-mix
/// visualizers from it once it lands (derived keys, not an undo step; the player gets the new project).
pub(super) fn tick(app: &mut App, ctx: &egui::Context) {
    if !needed(app) {
        return;
    }
    refresh(app, ctx);
    let (key, vis_key) = (KEY.load(Ordering::Relaxed), VIS_KEY.load(Ordering::Relaxed));
    let Some((k, mix)) = viz::mix_cached() else { return };
    let mut baked = BAKED.lock().unwrap_or_else(|e| e.into_inner());
    if k != key || *baked == (key, vis_key) {
        return;
    }
    *baked = (key, vis_key);
    drop(baked);
    let ids = viz::mix_visualizers(&app.project);
    for id in &ids {
        let _ = viz::bake_mix(&mut app.project, *id, &mix);
    }
    if !ids.is_empty() {
        app.dirty = true;
        app.player.set_project(&app.project);
    }
}

/// Paint the mix's visualizer over the Preview's canvas, and the badge. Called by the Preview pane while
/// Audio Only is on.
pub(super) fn paint(app: &mut App, ui: &mut egui::Ui) {
    let aspect = app.project.width.max(1) as f32 / app.project.height.max(1) as f32;
    let lb = crate::ui::preview::letterbox(app.preview.canvas_rect, aspect, ui.pixels_per_point());
    if !lb.is_positive() {
        return;
    }
    refresh(app, ui.ctx());
    let song = viz::mix_cached().map(|(_, a)| a);
    // fallback: the Main bus meter (peak of L/R) until the analysis lands
    let mut meter = || {
        crate::engine::mixer_fx::METER_FEED.drain_into(&mut app.buses);
        let main = app.project.buses.first().map_or(0, |b| b.id);
        let (l, r) = app.buses.meter(main);
        l.max(r)
    };
    let level = if song.is_none() { meter() } else { 0.0 };
    let t = app.playhead;
    let want = app.settings.visualizer_style.clone();
    ORB.with(|o| {
        let (orb, name, src) = &mut *o.borrow_mut();
        if src.is_empty() || *name != want {
            // (reads the library folder: only when the style changes)
            *src = crate::engine::visualizer::last_used(&want).src;
            *name = want;
        }
        match &song {
            Some(a) => orb.show(Some(a), t),
            None => orb.show_level(level, t),
        }
        orb.paint(ui.painter(), lb, src);
    });
    badge(ui, lb, &app.palette);
    if app.player.is_playing() {
        ui.ctx().request_repaint();
    }
}

/// The small "Audio Only" plate in the canvas's top-right corner.
fn badge(ui: &egui::Ui, lb: egui::Rect, palette: &crate::ui::Palette) {
    let font = egui::TextStyle::Small.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(BADGE.into(), font, palette.text);
    let size = galley.size() + egui::vec2(12.0, 6.0);
    let plate = egui::Rect::from_min_size(egui::pos2(lb.right() - size.x - 6.0, lb.top() + 6.0), size);
    ui.painter().rect_filled(plate, 3.0, palette.panel.gamma_multiply(0.8));
    ui.painter().galley(plate.min + egui::vec2(6.0, 3.0), galley, palette.text);
}

/// The badge's text.
pub(crate) const BADGE: &str = "Audio Only";

/// Turn Audio Only on or off for this project: instant (the render thread swaps its view), saved with
/// the project, not an undo step.
pub(super) fn set(app: &mut App, on: bool) {
    if app.project.audio_only_preview == on {
        return;
    }
    app.project.audio_only_preview = on;
    app.dirty = true;
    app.player.set_project(&app.project);
    app.gpu_tex = None;
    app.pending_frame = None;
    app.toast(if on { "Audio Only: on" } else { "Audio Only: off" });
}

/// ACT_HANDLERS entry: Audio Only toggles.
pub(super) fn act(app: &mut App, a: Action) -> bool {
    if a != Action::AudioOnlyPreview {
        return false;
    }
    let on = !app.project.audio_only_preview;
    set(app, on);
    true
}

pub const TOOLS: &[ToolDef] = &[ToolDef {
    name: "preview.audio_only",
    desc: "Audio Only preview: on = the Preview plays the timeline's sound and shows a visualizer of the mix \
           instead of the picture (no video is decoded); off = the picture again; omit `on` to toggle. Saved \
           with the project, not an undo step; export is unaffected. Returns the new state.",
    args: &["on:boolean:false:omit to toggle"],
    kind: ToolKind::Ui,
    run: |app, args| {
        let on = arg_bool(args, "on").unwrap_or(!app.project.audio_only_preview);
        set(app, on);
        Ok(ToolOutcome::Done(json!({"ok": true, "audio_only": app.project.audio_only_preview})))
    },
}];

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole-mix source reads what the mixer makes: a tone on the timeline is heard through it.
    #[test]
    fn the_mix_source_reads_the_timeline_mix() {
        use crate::media::AudioSource;
        let wav = crate::engine::visualizer::tests::song_wav("audio-only-mix");
        let mut p = Project::new();
        let asset = crate::engine::visualizer::tests::song_asset(&mut p, &wav);
        p.insert_asset_clips(asset, 0.0, None);
        let mut src = viz::MixSource::new(p, crate::media::Backend::Ffmpeg);
        assert!(src.duration() > 1.0);
        let mut buf = vec![0f32; 4800];
        let mut loud = 0.0f32;
        for i in 0..20 {
            src.read_at(0.5 + i as f64 * 0.05, &mut buf);
            loud = buf.iter().fold(loud, |m, v| m.max(v.abs()));
        }
        assert!(loud > 0.01, "the song is heard through the mix source");
        let a = SongAnalysis::compute(&mut src);
        assert!(a.frames.iter().any(|f| f[0] > 0.1), "the analysis follows the mix");
    }

    #[test]
    fn audio_only_is_a_tool_and_an_action() {
        assert_eq!(crate::mcp::tools::find("preview.audio_only").unwrap().kind, ToolKind::Ui);
        assert_eq!(Action::AudioOnlyPreview.label(), "Audio Only");
        assert_eq!(crate::hotkeys::group(Action::AudioOnlyPreview), "Playback");
    }
}
