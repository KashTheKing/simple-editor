use super::*;
// ---- ws:pro-monitor ----
use super::tools_monitor;

pub(super) fn draw(app: &mut App, ui: &mut egui::Ui) {
    // ws:source-monitor: the library-preview override that used to gate this block (`if
    // app.lib_preview.is_some() { draw_lib_preview } else { .. }`) is gone — Pane::Source owns the
    // source player now. The block itself is left un-dedented so canvas-handles-monitor's concurrent
    // edits to this file merge cleanly. A press on the program monitor hands transport focus
    // (Space/JKL/I/O) back to the timeline, exactly like a press on the timeline itself.
    if source_pane::pressed_in(ui) {
        app.source_focus = false;
    }
    {
        let frame = app.pending_frame.take();
        let proxy_busy = app.proxy_job.as_ref().map(|(_, _, p)| p.fraction());
        // ---- ws:pro-monitor ----
        // Computed against the whole `App` before the field-destructure below (which borrows `monitor`/
        // `gpu` disjointly) — both need methods (`monitor::trim_frames`, `GpuRenderer::stats`), not just
        // a field, so they can't live inside that destructure without re-borrowing all of `app`.
        let trim_frames = super::monitor::trim_frames(app);
        let trim_frames = match trim_frames {
            (Some(o), Some(i)) => Some((o, i)),
            _ => None,
        };
        let stats = app.gpu.as_ref().and_then(|g| g.stats());
        let pick_mode = app.monitor.pick_armed.map(|(_, t)| t);
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
                // ---- ws:canvas-handles-monitor ----
                alt_render,
                export,
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
                    playing: player.is_playing(),
                    fullscreen: *fullscreen,
                    palette,
                    undo: &mut push,
                    frame,
                    gpu_texture: *gpu_tex,
                    tool: tools.tool,
                    shape_style: match tools.tool {
                        Tool::Shape(k) => Some(tools::shape_style_from_tools(tools, k)),
                        _ => None,
                    },
                    quality: settings.preview_quality,
                    movie_mode: settings.movie_mode,
                    prerender: done,
                    buffering: player.is_buffering(),
                    proxy: proxy_busy,
                    tracker: tracking_shown.then(|| tracking.box_rect()),
                    guide: settings.guide,
                    // ---- ws:canvas-handles-monitor ----
                    canvas_snap: settings.canvas_snap,
                    // an export is served on this thread: the monitor keeps the live frame meanwhile
                    // (`monitor::tick` starts nothing either)
                    alt_texture: export.is_none().then(|| alt_render.texture()).flatten(),
                    use_proxies: settings.use_proxies,
                    dropped: player.dropped_frames(),
                    // ---- ws:pro-monitor ----
                    trim_frames: trim_frames.clone(),
                    pick_mode,
                    stats,
                },
            )
        };
        // ---- ws:pro-monitor ----
        // `pick_armed` is only consumed on an actual click (`resp.picked` is `Some`) — armed once by the
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
        let (cw, ch) = preview_canvas(resp.canvas, app.settings.preview_quality);
        app.player.set_canvas(cw, ch, app.settings.preview_max_width);
        // same clamp the player applies, so the GPU renders at the aspect the player decodes at
        app.canvas = clamp_canvas(cw, ch, app.settings.preview_max_width);
        if let Some(t) = resp.seek {
            app.seek(t);
        }
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
        if let Some(g) = resp.set_guide {
            app.settings.guide = g;
            app.settings.save();
        }
        // ---- ws:canvas-handles-monitor ----
        if let Some(on) = resp.set_canvas_snap {
            app.settings.canvas_snap = on;
            app.settings.save();
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
            app.add_stroke(s);
        }
        if resp.edited {
            app.after_edit();
        }
    }
}
