use super::*;

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
                },
            )
        };
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
