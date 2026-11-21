use super::*;
// ---- ws:pro-monitor ----
use super::tools_monitor;

pub(super) fn draw(app: &mut App, ui: &mut egui::Ui) {
    // ws:source-monitor: the library-preview override that used to gate this block (`if
    // app.lib_preview.is_some() { draw_lib_preview } else { .. }`) is gone - Pane::Source owns the
    // source player now. The block itself is left un-dedented so canvas-handles-monitor's concurrent
    // edits to this file merge cleanly. A press on the program monitor hands transport focus
    // (Space/JKL/I/O) back to the timeline, exactly like a press on the timeline itself.
    if source_pane::pressed_in(ui) {
        app.source_focus = false;
        // a Source playing in a hidden tab stops when the Preview is clicked
        if !app.pane_drawn(Pane::Source) {
            if let Some(s) = app.source.as_mut() {
                s.player.pause();
            }
        }
    }
    // ---- ws:viewer-surface ----
    // the rail's options strip holds the Draw tool's Record button (#87's playback path)
    let recording = app.tools.recording;
    {
        let frame = app.pending_frame.take();
        let proxy_busy = app.proxy_job.as_ref().map(|(_, _, p)| p.fraction());
        // ---- ws:pro-monitor ----
        // Computed against the whole `App` before the field-destructure below (which borrows `monitor`/
        // `gpu` disjointly) - both need methods (`monitor::trim_frames`, `GpuRenderer::stats`), not just
        // a field, so they can't live inside that destructure without re-borrowing all of `app`.
        let trim_frames = super::monitor::trim_frames(app);
        let trim_frames = match trim_frames {
            (Some(o), Some(i)) => Some((o, i)),
            _ => None,
        };
        let stats = app.gpu.as_ref().and_then(|g| g.stats());
        let pick_mode = app.monitor.pick_armed.map(|(_, t)| t);
        // ws:animator: the onion skin ghosts the selected layer's own picture (a still's image, a video's frame
        // at the playhead), loaded off the UI thread by the thumbnail cache - nothing renders on the way
        let ghost = (crate::ui::animator::onion() > 0 && !app.player.is_playing())
            .then(|| {
                let c = app.selection.iter().find_map(|&id| {
                    app.project.clip(id).filter(|c| c.is_visual() && c.enabled && c.contains(app.playhead))
                })?;
                let a = app.project.asset(c.asset).filter(|a| a.has_video())?;
                app.thumbs.texture(ui.ctx(), &a.path, c.src_time(app.playhead), 360)
            })
            .flatten();
        crate::ui::animator::set_onion_tex(ghost);
        let tracker = app.tracking.preview_box().filter(|_| app.pane_drawn(Pane::Tracking));
        let resp = {
            let App {
                project,
                selection,
                playhead,
                undo,
                redo,
                preview: pv,
                player,
                play_gate,
                palette,
                fullscreen,
                tools,
                settings,
                prerender,
                gpu_tex,
                // ---- ws:canvas-handles-monitor ----
                alt_render,
                export,
                thumbs,
                ..
            } = app;
            let mut push = |p: &Project| push_undo_json(undo, redo, p.to_json());
            // only while movie mode is on: progress() walks the requested ranges
            let done =
                settings.movie_mode.then(|| guarded(|| prerender.progress()).unwrap_or(1.0)).filter(|&p| p < 1.0);
            preview::show(
                ui,
                pv,
                preview::PreviewCtx {
                    project,
                    selection,
                    playhead: *playhead,
                    playing: player.is_playing() || play_gate.buffering(),
                    fullscreen: *fullscreen,
                    palette,
                    undo: &mut push,
                    frame,
                    gpu_texture: *gpu_tex,
                    tools,
                    rail: !settings.tool_rail_hidden,
                    quality: settings.preview_quality,
                    movie_mode: settings.movie_mode,
                    prerender: done,
                    buffering: player.is_buffering() || play_gate.buffering(),
                    buffer_note: play_gate.note(), // ws:play-gate
                    proxy: proxy_busy,
                    tracker,
                    settings: &*settings,
                    // ---- ws:canvas-handles-monitor ----
                    canvas_snap: settings.canvas_snap,
                    // an export is served on this thread: the monitor keeps the live frame meanwhile
                    // (`monitor::tick` starts nothing either)
                    alt_texture: export.is_none().then(|| alt_render.texture()).flatten(),
                    use_proxies: settings.use_proxies,
                    dropped: player.dropped_frames(),
                    rate: if play_gate.buffering() { play_gate.rate() } else { player.rate() },
                    looping: player.loop_range().is_some(),
                    thumbs: Some(thumbs),
                    // ---- ws:pro-monitor ----
                    trim_frames: trim_frames.clone(),
                    pick_mode,
                    stats,
                },
            )
        };
        // ---- ws:addon-sam2 ---- Object Mask's click mode: the live mask, its points and controls on top
        super::object_mask_ctl::overlay(app, ui);
        // ---- ws:audio-visualizer ---- a stretch with only sound: the "now playing" card over the frame
        // (ws:preview-audio-only: Audio Only paints the mix's visualizer over the whole canvas instead)
        if app.project.audio_only_preview {
            super::audio_only::paint(app, ui);
        } else if let Some(aid) = crate::ui::audio_card::audio_only_at(&app.project, app.playhead) {
            let path = app.project.clip(aid).and_then(|c| app.project.asset(c.asset)).map(|a| a.path.clone());
            if let Some(path) = path {
                let aspect = app.project.width.max(1) as f32 / app.project.height.max(1) as f32;
                let lb = preview::letterbox(app.preview.canvas_rect, aspect, ui.pixels_per_point());
                if lb.is_positive() {
                    let cover = app
                        .thumbs
                        .texture(ui.ctx(), &path, 0.0, (lb.height() * ui.pixels_per_point()) as u32)
                        .or_else(|| crate::ui::audio_card::cover(ui.ctx(), &path));
                    let title = crate::ui::audio_card::title(ui.ctx(), &path);
                    crate::ui::audio_card::paint(ui, lb, cover, &title);
                }
            }
        }
        // ---- ws:pro-monitor ----
        // `pick_armed` is only consumed on an actual click (`resp.picked` is `Some`) - armed once by the
        // Color panel's Eyedropper button, it must survive every frame the user hasn't clicked yet
        // (moving the mouse from the panel to the canvas takes more than one frame). Checking
        // `resp.picked` first, THEN `.take()`-ing, keeps it armed across every frame nothing was clicked.
        if let Some(rgb) = resp.picked {
            if let Some((id, target)) = app.monitor.pick_armed.take() {
                let before = app.project.to_json();
                match tools_monitor::write_picked_color(app, id, target, rgb) {
                    Ok(()) => {
                        app.push_undo_labeled(before, "Eyedropper");
                        app.after_edit();
                    }
                    Err(e) => app.toast(e),
                }
            }
        }
        auto_res(app);
        crate::playback::set_audio_delay_ms(app.settings.audio_delay_ms); // an atomic store: cheap each frame
        let (cw, ch) = preview_canvas(resp.canvas, effective_quality(app));
        let cap = preview_cap(app);
        app.player.set_canvas(cw, ch, cap);
        // same clamp the player applies, so the GPU renders at the aspect the player decodes at
        app.canvas = clamp_canvas(cw, ch, cap);
        if let Some(t) = resp.seek {
            app.seek(t);
        }
        bar_transport(app, resp.set_rate, resp.reverse, resp.toggle_loop);
        if let Some(m) = resp.mark {
            drag_mark(app, m);
        }
        follow_inout_loop(app);
        app.pending_actions.extend(resp.actions);
        if let Some(q) = resp.set_quality {
            app.settings.preview_quality = q;
            app.settings.save();
        }
        if resp.set_movie_mode.is_some() {
            // the action toggles the setting and starts / clears the pre-render
            app.pending_actions.push(Action::MovieMode);
        }
        if let Some((x, y)) = resp.set_tracker {
            (app.tracking.cx, app.tracking.cy) = (x, y);
        }
        match resp.set_guide {
            Some(crate::ui::social::Set::Open) => super::emulator_ctl::open(app),
            Some(g) => g.apply(&mut app.settings),
            None => {}
        }
        if let Some(s) = resp.set_visualizer_style {
            app.settings.visualizer_style = s;
            app.settings.save();
        }
        // ---- ws:canvas-handles-monitor ----
        if let Some(on) = resp.set_canvas_snap {
            app.settings.canvas_snap = on;
            app.settings.save();
        }
        // ---- ws:parity-preview-tools ----
        if let Some(ov) = resp.set_overlay {
            app.settings.preview_overlay = ov;
            app.settings.save_async();
        }
        if let Some((kind, cx, cy, w, h)) = resp.new_shape {
            let id = app.add_shape(kind, Some((cx, cy, w, h)));
            if !resp.new_points.is_empty() {
                if let Some(s) = app.project.clip_mut(id).and_then(|c| c.shape.as_mut()) {
                    s.points = resp.new_points;
                }
                app.after_edit();
            }
        }
        if let Some((cx, cy, _, _)) = resp.new_text {
            app.add_text(cx, cy);
        }
        if let Some(s) = resp.stroke {
            if !super::animator_ctl::take_stroke(app, &s) {
                app.add_stroke(s);
            }
        }
        if resp.edited {
            app.after_edit();
        }
        // ---- ws:viewer-surface ----
        if let Some(id) = resp.select {
            app.selection = vec![id];
        }
    }
    if app.tools.recording != recording {
        app.toggle_draw_recording(app.tools.recording);
    }
    if app.settings.hud {
        crate::ui::hud::draw(ui, &app.hud, app.player.dropped_frames()); // ws:perf-baseline
    }
    super::health_ctl::chip(app, ui); // ws:forum-preview
}

/// The bar's speed menu, Reverse and Loop (`ui::media_bar`). A speed plays at it, keeping the direction;
/// Reverse turns playback round (starting it backwards when stopped); Loop loops In to Out when both are
/// marked, else the whole timeline.
fn bar_transport(app: &mut App, speed: Option<f64>, reverse: bool, toggle_loop: bool) {
    let rate = if app.transport_playing() { app.transport_rate() } else { 0.0 };
    if let Some(s) = speed {
        app.play_gated(if rate < 0.0 { -s } else { s });
    }
    if reverse {
        let mag = if rate == 0.0 { 1.0 } else { rate.abs() };
        app.play_gated(if rate < 0.0 { mag } else { -mag });
    }
    if toggle_loop {
        let range = loop_range(app.project.in_point, app.project.out_point, app.project.duration());
        let new = if app.player.loop_range().is_some() { None } else { range };
        app.player.set_loop(new);
        app.inout_loop = new;
    }
}

/// Each frame: a Loop the bar or Loop In/Out turned on follows In / Out when they move (a drag, Mark In,
/// Clear). Another loop (the Animator's) replaced it: leave that alone.
pub(super) fn follow_inout_loop(app: &mut App) {
    let Some(ours) = app.inout_loop else { return };
    if app.player.loop_range() != Some(ours) {
        app.inout_loop = None;
        return;
    }
    let want = loop_range(app.project.in_point, app.project.out_point, app.project.duration());
    if want.is_some() && want != Some(ours) {
        app.player.set_loop(want);
        app.inout_loop = want;
    }
}

/// What the bar's Loop loops: In to Out when both are marked (and in order), else the whole timeline.
pub(super) fn loop_range(a: Option<f64>, b: Option<f64>, dur: f64) -> Option<(f64, f64)> {
    match (a, b) {
        (Some(a), Some(b)) if b > a => Some((a, b)),
        _ => (dur > 0.0).then_some((0.0, dur)),
    }
}

// ---- ws:viewer-surface ----
/// ACT_HANDLERS entry: Show / Hide Tools shows or hides the viewer's tool rail (the Tools pane it used
/// to toggle is gone - its tools live on the rail).
pub(super) fn act(app: &mut App, a: Action) -> bool {
    if a != Action::ToggleTools {
        return false;
    }
    app.settings.tool_rail_hidden = !app.settings.tool_rail_hidden;
    app.settings.save();
    true
}

/// The playback bar's In / Out flag dragged: one undo step per drag, snapped to frames, In never past Out.
fn drag_mark(app: &mut App, m: crate::ui::media_bar::MarkDrag) {
    if m.start {
        app.push_undo();
    }
    let t = app.project.snap_frame(m.t.max(0.0));
    if m.out {
        app.project.out_point = Some(t.max(app.project.in_point.unwrap_or(0.0)));
    } else {
        app.project.in_point = Some(app.project.out_point.map_or(t, |o| t.min(o)));
    }
    if m.end {
        app.after_edit();
    }
}

/// Auto playback resolution (`Settings.preview_quality == 0`): the level a play runs at, and that play's
/// dropped-frame watch.
#[derive(Default)]
pub(crate) struct AutoRes {
    /// The resolution plays start at (100 until frames drop).
    level: u32,
    /// The current play: when the 1.5 s window began, the dropped count then, and whether it dropped.
    window: Option<(Instant, u64)>,
    dropped: bool,
    playing: bool,
    /// Has a play run smooth at full resolution (`preview_cap`)? None = not yet; Some(true) = plays run
    /// past `preview_max_width` too; Some(false) = one dropped frames there, so plays keep the cap.
    smooth: Option<bool>,
}

/// Widest preview a still frame is drawn at in fullscreen or with Playback Resolution on Auto.
pub(crate) const PRISTINE_MAX_W: u32 = 3840;

/// The width the preview canvas is capped at now. `preview_max_width` as set, except in fullscreen or
/// with Playback Resolution on Auto: a paused frame is drawn up to the pane's own pixel size (at most
/// `PRISTINE_MAX_W`) so it is sharp, and on Auto a play runs that big too once a play has been smooth.
pub(super) fn preview_cap(app: &App) -> u32 {
    let s = &app.settings;
    let smooth = app.auto_res.smooth == Some(true);
    preview_cap_for(s.preview_max_width, s.preview_quality, app.fullscreen, app.player.is_playing(), smooth)
}

/// `preview_cap` from its inputs: the set cap, the Playback Resolution (0 = Auto), fullscreen,
/// playing, and whether Auto has found a full-size play smooth. 0 = no cap, as `Player::set_canvas`.
pub(crate) fn preview_cap_for(max_w: u32, quality: u32, fullscreen: bool, playing: bool, smooth: bool) -> u32 {
    let auto = quality == 0;
    if max_w == 0 || !(fullscreen || auto) {
        return max_w;
    }
    let pristine = !playing || (auto && smooth);
    if pristine {
        max_w.max(PRISTINE_MAX_W)
    } else {
        max_w
    }
}

/// The render scale in use now: the setting, or with Auto the play's level while playing, full while paused.
pub(super) fn effective_quality(app: &App) -> u32 {
    match app.settings.preview_quality {
        0 if app.player.is_playing() => app.auto_res.level.max(25),
        0 => 100,
        q => q,
    }
}

/// Each frame: with Auto on, a play that keeps dropping frames (3+ in 1.5 s) steps one resolution down
/// and stays there; a play that ends with none dropped lets the next one start a step higher. Changing
/// the size only while it drops (or between plays) keeps a smooth play from ever being disturbed.
pub(super) fn auto_res(app: &mut App) {
    let a = &mut app.auto_res;
    if a.level == 0 {
        a.level = 100;
    }
    let playing = app.player.is_playing();
    if app.settings.preview_quality != 0 || !playing {
        if a.playing && !a.dropped && a.level < 100 {
            a.level = step(a.level, true);
        } else if a.playing && !a.dropped && a.smooth.is_none() {
            // a clean play at full resolution: the next ones may run past `preview_max_width`
            a.smooth = Some(true);
        }
        (a.playing, a.window, a.dropped) = (false, None, false);
        return;
    }
    a.playing = true;
    let (now, d) = (Instant::now(), app.player.dropped_frames());
    match a.window {
        None => a.window = Some((now, d)),
        Some((t0, d0)) if now.duration_since(t0).as_secs_f32() >= 1.5 => {
            if d.saturating_sub(d0) >= 3 {
                a.dropped = true;
                if a.smooth == Some(true) {
                    // too big uncapped: back under `preview_max_width` for good, at the same level
                    a.smooth = Some(false);
                } else {
                    a.level = step(a.level, false);
                }
            }
            a.window = Some((now, d));
        }
        _ => {}
    }
}

/// The next playback resolution up or down (`QUALITIES` is 100, 75, 50, 25).
fn step(q: u32, up: bool) -> u32 {
    let qs = crate::ui::preview::QUALITIES;
    let i = qs.iter().position(|&x| x == q).unwrap_or(0);
    if up { qs[i.saturating_sub(1)] } else { qs[(i + 1).min(qs.len() - 1)] }
}

#[cfg(test)]
mod auto_res_tests {
    #[test]
    fn steps_through_the_resolutions_and_stops_at_the_ends() {
        assert_eq!(super::step(100, false), 75);
        assert_eq!(super::step(25, false), 25);
        assert_eq!(super::step(50, true), 75);
        assert_eq!(super::step(100, true), 100);
    }

    #[test]
    fn paused_frames_are_drawn_at_the_pane_size_in_fullscreen_or_on_auto() {
        use super::{preview_cap_for as cap, PRISTINE_MAX_W as P};
        // (max_w, quality, fullscreen, playing, smooth)
        assert_eq!(cap(1280, 0, false, false, false), P, "Auto, paused: up to 4K");
        assert_eq!(cap(1280, 100, true, false, false), P, "fullscreen, paused: up to 4K");
        assert_eq!(cap(1280, 0, false, true, false), 1280, "playing keeps the cap");
        assert_eq!(cap(1280, 100, true, true, true), 1280, "a fixed resolution plays capped, fullscreen too");
        assert_eq!(cap(1280, 0, false, true, true), P, "Auto found a full play smooth: uncapped");
        assert_eq!(cap(1280, 50, false, false, false), 1280, "a fixed resolution, windowed: unchanged");
        assert_eq!(cap(0, 0, true, false, false), 0, "no cap stays no cap");
        assert_eq!(cap(3840, 0, false, false, false), 3840);
    }
}
