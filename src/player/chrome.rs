//! The player's look, after Windows Media Player: the picture edge to edge, and over it a title bar
//! (Editor button, app icon and name, window buttons) and a bottom bar (scrubber with elapsed / total,
//! the file's title, the transport in the middle, tracks / volume / full screen / mini player / ⋯ on
//! the right), both on soft dark gradients that fade away with the pointer (`ui::autohide`).
//!
//! Icons are Windows' own icon font (Segoe Fluent Icons / MDL2 Assets), the glyphs Media Player draws.

use super::{Flyout, PlayerApp, Zoom, SKIP_BACK, SKIP_FORWARD};
use crate::player::folder::Repeat;
use crate::ui::autohide;
use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Pos2, Rect, Response, Sense, Stroke, Ui};
use std::path::Path;

/// Segoe Fluent Icons / MDL2 Assets codepoints.
pub mod icon {
    pub const EDIT: char = '\u{E70F}';
    pub const SHUFFLE: char = '\u{E8B1}';
    pub const PREVIOUS: char = '\u{E892}';
    pub const NEXT: char = '\u{E893}';
    pub const PLAY: char = '\u{E768}';
    pub const PAUSE: char = '\u{E769}';
    pub const BACK10: char = '\u{ED3C}';
    pub const FORWARD30: char = '\u{ED3D}';
    pub const REPEAT_ALL: char = '\u{E8EE}';
    pub const REPEAT_ONE: char = '\u{E8ED}';
    pub const CAPTIONS: char = '\u{E7F0}';
    pub const VOLUME: [char; 4] = ['\u{E992}', '\u{E993}', '\u{E994}', '\u{E995}'];
    pub const MUTE: char = '\u{E74F}';
    pub const FULLSCREEN: char = '\u{E740}';
    pub const EXIT_FULLSCREEN: char = '\u{E73F}';
    pub const MINI: char = '\u{E92C}';
    pub const UNMINI: char = '\u{E92D}';
    pub const MORE: char = '\u{E712}';
    pub const MINIMIZE: char = '\u{E921}';
    pub const MAXIMIZE: char = '\u{E922}';
    pub const RESTORE: char = '\u{E923}';
    pub const CLOSE: char = '\u{E8BB}';
    pub const MUSIC: char = '\u{E8D6}';
    pub const FOLDER: char = '\u{E838}';
    pub const INFO: char = '\u{E946}';
    pub const CHECK: char = '\u{E73E}';
    pub const ALL: [char; 29] = [
        EDIT, SHUFFLE, PREVIOUS, NEXT, PLAY, PAUSE, BACK10, FORWARD30, REPEAT_ALL, REPEAT_ONE, CAPTIONS, VOLUME[0],
        VOLUME[1], VOLUME[2], VOLUME[3], MUTE, FULLSCREEN, EXIT_FULLSCREEN, MINI, UNMINI, MORE, MINIMIZE, MAXIMIZE,
        RESTORE, CLOSE, MUSIC, FOLDER, INFO, CHECK,
    ];
}

/// Media Player's progress orange and the pink its play ring blends into.
const ORANGE: Color32 = Color32::from_rgb(0xF7, 0x7A, 0x1E);
const PINK: Color32 = Color32::from_rgb(0xE0, 0x2E, 0x8C);
const FLYOUT_BG: Color32 = Color32::from_rgb(0x2C, 0x2C, 0x2C);

const TOP_H: f32 = 40.0;
const BOTTOM_H: f32 = 116.0;

/// "0:00:41", as Media Player writes it.
pub fn clock(t: f64) -> String {
    let s = t.max(0.0).floor() as u64;
    format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

fn icon_font(px: f32) -> FontId {
    FontId::new(px, crate::theme::icons())
}

fn glyph(p: &egui::Painter, at: Pos2, c: char, px: f32, col: Color32) {
    p.text(at, Align2::CENTER_CENTER, c, icon_font(px), col);
}

/// A flat icon button: a soft highlight under the pointer, the glyph in white (accent when `on`).
fn icon_button(ui: &mut Ui, rect: Rect, c: char, px: f32, on: bool, tip: &str) -> Response {
    let r = ui.interact(rect, ui.id().with(("btn", c, rect.min.x as i32)), Sense::click()).on_hover_text(tip);
    let p = ui.painter();
    if r.is_pointer_button_down_on() {
        p.rect_filled(rect.shrink(2.0), 6.0, Color32::from_white_alpha(14));
    } else if r.hovered() {
        p.rect_filled(rect.shrink(2.0), 6.0, Color32::from_white_alpha(24));
    }
    glyph(p, rect.center(), c, px, if on { ORANGE } else { Color32::WHITE });
    r
}

/// A vertical gradient (`top` colour to `bottom` colour).
fn vgrad(p: &egui::Painter, rect: Rect, top: Color32, bottom: Color32) {
    let mut m = egui::Mesh::default();
    m.colored_vertex(rect.left_top(), top);
    m.colored_vertex(rect.right_top(), top);
    m.colored_vertex(rect.left_bottom(), bottom);
    m.colored_vertex(rect.right_bottom(), bottom);
    m.add_triangle(0, 1, 2);
    m.add_triangle(1, 3, 2);
    p.add(m);
}

fn lerp(a: Color32, b: Color32, k: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * k).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// The big play button: dark disc, orange-to-pink ring.
fn play_button(ui: &mut Ui, center: Pos2, radius: f32, playing: bool) -> Response {
    let rect = Rect::from_center_size(center, vec2(radius * 2.0, radius * 2.0));
    let tip = if playing { "Pause (Space)" } else { "Play (Space)" };
    let r = ui.interact(rect, ui.id().with("play"), Sense::click()).on_hover_text(tip);
    let p = ui.painter();
    let fill = if r.hovered() { Color32::from_rgb(0x3A, 0x3A, 0x3A) } else { Color32::from_rgb(0x26, 0x26, 0x26) };
    p.circle_filled(center, radius, fill);
    let n = 48;
    for i in 0..n {
        let (a0, a1) = (i as f32 / n as f32, (i + 1) as f32 / n as f32);
        let ang = |a: f32| -std::f32::consts::FRAC_PI_2 + a * std::f32::consts::TAU;
        let at = |a: f32| center + vec2(ang(a).cos(), ang(a).sin()) * (radius - 1.5);
        // orange at the top-left, pink at the bottom-right, back again
        let k = ((a0 * std::f32::consts::TAU).sin() * 0.5 + 0.5).clamp(0.0, 1.0);
        p.line_segment([at(a0), at(a1)], Stroke::new(2.5, lerp(ORANGE, PINK, k)));
    }
    glyph(p, center + vec2(if playing { 0.0 } else { 1.5 }, 0.0), if playing { icon::PAUSE } else { icon::PLAY }, radius * 0.8, Color32::WHITE);
    r
}

/// The scrubber: grey track, orange played part, a thumb that grows under the pointer. Returns the
/// time under the pointer while dragged / clicked.
fn scrubber(ui: &mut Ui, rect: Rect, t: f64, dur: f64) -> (Response, Option<f64>) {
    let r = ui.interact(rect, ui.id().with("scrub"), Sense::click_and_drag());
    let p = ui.painter();
    let x0 = rect.left() + 8.0;
    let x1 = rect.right() - 8.0;
    let y = rect.center().y;
    let k = if dur > 0.0 { (t / dur).clamp(0.0, 1.0) as f32 } else { 0.0 };
    let x = x0 + (x1 - x0) * k;
    let hot = r.hovered() || r.dragged();
    let th = if hot { 5.0 } else { 3.0 };
    p.rect_filled(Rect::from_min_max(pos2(x0, y - th / 2.0), pos2(x1, y + th / 2.0)), th / 2.0, Color32::from_white_alpha(70));
    p.rect_filled(Rect::from_min_max(pos2(x0, y - th / 2.0), pos2(x, y + th / 2.0)), th / 2.0, ORANGE);
    let rad = if hot { 8.0 } else { 6.0 };
    p.circle_filled(pos2(x, y), rad, ORANGE);
    p.circle_stroke(pos2(x, y), rad, Stroke::new(1.5, Color32::WHITE));
    let at = |px: f32| (((px - x0) / (x1 - x0)).clamp(0.0, 1.0) as f64) * dur;
    let mut seek = None;
    if let Some(pos) = r.interact_pointer_pos().filter(|_| r.dragged() || r.clicked() || r.drag_started()) {
        seek = Some(at(pos.x));
    }
    if let Some(h) = r.hover_pos() {
        if !r.dragged() {
            let tip = clock(at(h.x));
            let g = p.layout_no_wrap(tip, FontId::proportional(12.0), Color32::WHITE);
            let box_ = Rect::from_center_size(pos2(h.x, rect.top() - 14.0), g.size() + vec2(12.0, 6.0));
            p.rect_filled(box_, 4.0, FLYOUT_BG);
            p.galley(box_.center() - g.size() / 2.0, g, Color32::WHITE);
        }
    }
    (r, seek)
}

/// Draw the frame: picture, subtitles, chrome, flyouts. Returns what the auto-hide needs repainted.
pub fn draw(app: &mut PlayerApp, ctx: &egui::Context, now: f64) -> autohide::Output {
    let mut out = autohide::Output { opacity: 0.0, repaint_in: None, hide_cursor: false };
    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(Color32::BLACK)).show(ctx, |ui| {
        out = body(app, ui, now);
    });
    out
}

fn body(app: &mut PlayerApp, ui: &mut Ui, now: f64) -> autohide::Output {
    let ctx = ui.ctx().clone();
    let full = ui.max_rect();
    let ppp = ctx.pixels_per_point();
    let mini = app.mini.is_some();
    let vp = ctx.input(|i| i.viewport().clone());
    let maximized = vp.maximized.unwrap_or(false);

    // ---- the picture (or the audio view)
    let video_rect = match (&app.info, app.video.shown) {
        (Some(info), Some((tex, _, uv))) if info.has_video => {
            let r = app.zoom.place(full, info.size, ppp);
            ui.painter().with_clip_rect(full).image(tex, r, uv, Color32::WHITE);
            Some(r)
        }
        _ => None,
    };
    match app.info.as_ref().map(|i| (i.has_video, i.size)) {
        Some((true, size)) => {
            // the video processor scales to exactly the pixels the picture covers
            let target = app.zoom.place(full, size, ppp);
            let px = |v: f32| ((v * ppp).round() as u32).clamp(16, 7680) & !1;
            if app.zoom == Zoom::Original {
                app.engine.set_output_size(size.0, size.1);
            } else {
                app.engine.set_output_size(px(target.width()), px(target.height()));
            }
        }
        Some((false, _)) => audio_view(app, ui, full, mini),
        None => {}
    }
    if let Some(e) = &app.error {
        ui.painter().text(full.center(), Align2::CENTER_CENTER, format!("Can't play this file\n{e}"), FontId::proportional(15.0), Color32::from_gray(200));
    }
    subtitles(app, ui, video_rect.unwrap_or(full).intersect(full), app.hide.shown());

    // ---- the video surface: click = play/pause, double-click = full screen; in the mini player a
    // drag moves the window
    let surface = ui.interact(full, ui.id().with("surface"), Sense::click_and_drag());
    if surface.double_clicked() {
        app.double_click_video(&ctx);
    } else if surface.clicked() {
        app.click_video(now);
    }
    if mini && surface.drag_started() {
        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }

    // ---- auto-hide
    let top = Rect::from_min_size(full.min, vec2(full.width(), TOP_H));
    let bottom = Rect::from_min_max(pos2(full.left(), full.bottom() - if mini { 64.0 } else { BOTTOM_H }), full.max);
    let (pointer, moved) = ctx.input(|i| {
        let moved = i.events.iter().any(|e| matches!(e, egui::Event::PointerMoved(_) | egui::Event::PointerButton { .. }));
        (i.pointer.hover_pos(), moved)
    });
    let pointer_in = pointer.is_some_and(|p| full.contains(p));
    let over_chrome = pointer.is_some_and(|p| top.contains(p) || bottom.contains(p));
    let input = autohide::Input {
        now,
        pointer_in,
        moved,
        // paused, over the controls, or a sound file (nothing to uncover): the controls stay up
        keep: over_chrome || !app.engine.is_playing() || app.info.as_ref().is_some_and(|i| !i.has_video),
        hold: app.flyout.is_some() || app.scrub.is_some() || app.info_open,
    };
    let out = app.autohide(input);
    if out.hide_cursor {
        ctx.set_cursor_icon(egui::CursorIcon::None);
    }
    if out.opacity > 0.0 {
        let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(full));
        ui.set_opacity(out.opacity);
        if mini {
            mini_chrome(app, &mut ui, full);
        } else {
            top_bar(app, &mut ui, top, maximized);
            bottom_bar(app, &mut ui, bottom);
        }
    }
    if !(maximized || app.fullscreen || mini) {
        resize_edges(&ctx, full);
    }
    flyouts(app, &ctx, full);
    if app.info_open {
        info_window(app, &ctx);
    }
    out
}

/// Undecorated window: grab an edge to resize.
fn resize_edges(ctx: &egui::Context, full: Rect) {
    use egui::{CursorIcon as C, ResizeDirection as D};
    let Some(p) = ctx.input(|i| i.pointer.hover_pos()) else { return };
    let e = 5.0;
    let (l, r, t, b) = (p.x < full.left() + e, p.x > full.right() - e, p.y < full.top() + e, p.y > full.bottom() - e);
    let dir = match (l, r, t, b) {
        (true, _, true, _) => (D::NorthWest, C::ResizeNwSe),
        (_, true, true, _) => (D::NorthEast, C::ResizeNeSw),
        (true, _, _, true) => (D::SouthWest, C::ResizeNeSw),
        (_, true, _, true) => (D::SouthEast, C::ResizeNwSe),
        (true, ..) => (D::West, C::ResizeHorizontal),
        (_, true, ..) => (D::East, C::ResizeHorizontal),
        (_, _, true, _) => (D::North, C::ResizeVertical),
        (_, _, _, true) => (D::South, C::ResizeVertical),
        _ => return,
    };
    ctx.set_cursor_icon(dir.1);
    if ctx.input(|i| i.pointer.primary_pressed()) {
        ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(dir.0));
    }
}

fn top_bar(app: &mut PlayerApp, ui: &mut Ui, top: Rect, maximized: bool) {
    let ctx = ui.ctx().clone();
    vgrad(ui.painter(), Rect::from_min_size(top.min, vec2(top.width(), TOP_H + 32.0)), Color32::from_black_alpha(150), Color32::TRANSPARENT);
    // the empty bar drags the window, a double-click maximizes
    let bar = ui.interact(top, ui.id().with("titlebar"), Sense::click_and_drag());
    if bar.drag_started() && !app.fullscreen {
        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    if bar.double_clicked() && !app.fullscreen {
        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
    }
    let b = Rect::from_min_size(top.min + vec2(4.0, 4.0), vec2(40.0, 32.0));
    if icon_button(ui, b, icon::EDIT, 15.0, false, "Open in Editor (Esc)").clicked() {
        app.open_editor();
    }
    let icon_at = pos2(b.right() + 10.0, top.center().y);
    if app.icon.is_none() {
        app.icon = app_icon(&ctx);
    }
    if let Some(i) = &app.icon {
        ui.painter().image(i.id(), Rect::from_min_size(icon_at - vec2(0.0, 8.0), vec2(16.0, 16.0)), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }
    ui.painter().text(icon_at + vec2(26.0, 0.0), Align2::LEFT_CENTER, "Simple Editor", FontId::proportional(12.0), Color32::WHITE);
    if app.fullscreen {
        return;
    }
    let w = 46.0;
    let close = Rect::from_min_size(pos2(top.right() - w, top.top()), vec2(w, TOP_H - 8.0));
    let max = close.translate(vec2(-w, 0.0));
    let min = max.translate(vec2(-w, 0.0));
    if caption_button(ui, min, icon::MINIMIZE, false, "Minimize").clicked() {
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
    }
    let (c, tip) = if maximized { (icon::RESTORE, "Restore down") } else { (icon::MAXIMIZE, "Maximize") };
    if caption_button(ui, max, c, false, tip).clicked() {
        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
    }
    if caption_button(ui, close, icon::CLOSE, true, "Close").clicked() {
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

/// Windows' caption buttons: square, the close one red under the pointer.
fn caption_button(ui: &mut Ui, rect: Rect, c: char, close: bool, tip: &str) -> Response {
    let r = ui.interact(rect, ui.id().with(("caption", c)), Sense::click()).on_hover_text(tip);
    if r.hovered() {
        let fill = if close { Color32::from_rgb(0xC4, 0x2B, 0x1C) } else { Color32::from_white_alpha(22) };
        ui.painter().rect_filled(rect, 0.0, fill);
    }
    glyph(ui.painter(), rect.center(), c, 10.0, Color32::WHITE);
    r
}

fn bottom_bar(app: &mut PlayerApp, ui: &mut Ui, bottom: Rect) {
    let ctx = ui.ctx().clone();
    let p = ui.painter().clone();
    vgrad(&p, Rect::from_min_max(pos2(bottom.left(), bottom.top() - 60.0), bottom.max), Color32::TRANSPARENT, Color32::from_black_alpha(200));
    // the bar itself swallows clicks (no play/pause through it)
    ui.interact(bottom, ui.id().with("bottom"), Sense::click());
    let dur = app.engine.duration();
    let t = app.scrub.map_or_else(|| app.engine.time(), |(t, _)| t);
    // scrubber row: elapsed | bar | total
    let row_y = bottom.top() + 24.0;
    let font = FontId::proportional(12.0);
    let left = p.layout_no_wrap(clock(t), font.clone(), Color32::WHITE);
    let right = p.layout_no_wrap(clock(dur), font.clone(), Color32::WHITE);
    p.galley(pos2(bottom.left() + 14.0, row_y - left.size().y / 2.0), left.clone(), Color32::WHITE);
    p.galley(pos2(bottom.right() - 14.0 - right.size().x, row_y - right.size().y / 2.0), right.clone(), Color32::WHITE);
    let bar = Rect::from_min_max(pos2(bottom.left() + 22.0 + left.size().x, row_y - 10.0), pos2(bottom.right() - 22.0 - right.size().x, row_y + 10.0));
    let (r, at) = scrubber(ui, bar, t, dur);
    scrub(app, &r, at);

    // controls row
    let cy = bottom.top() + 74.0;
    let title = p.layout(app.title(), FontId::proportional(14.0), Color32::WHITE, (bottom.width() * 0.28).max(80.0));
    p.galley(pos2(bottom.left() + 16.0, cy - title.size().y / 2.0), title, Color32::WHITE);
    let cx = bottom.center().x;
    let step = 48.0;
    let sq = |x: f32| Rect::from_center_size(pos2(x, cy), vec2(40.0, 40.0));
    let playing = app.engine.is_playing();
    if icon_button(ui, sq(cx - step * 3.4), icon::SHUFFLE, 16.0, app.playlist.shuffle, if app.playlist.shuffle { "Shuffle: on" } else { "Shuffle: off" }).clicked() {
        app.set_shuffle(!app.playlist.shuffle);
    }
    if icon_button(ui, sq(cx - step * 2.4), icon::PREVIOUS, 16.0, false, "Previous (Ctrl+Left)").clicked() {
        app.prev(&ctx);
    }
    if icon_button(ui, sq(cx - step * 1.35), icon::BACK10, 20.0, false, "Skip back 10 seconds (Left)").clicked() {
        app.skip(-SKIP_BACK);
    }
    if play_button(ui, pos2(cx, cy), 22.0, playing).clicked() {
        app.toggle_play();
    }
    if icon_button(ui, sq(cx + step * 1.35), icon::FORWARD30, 20.0, false, "Skip forward 30 seconds (Right)").clicked() {
        app.skip(SKIP_FORWARD);
    }
    if icon_button(ui, sq(cx + step * 2.4), icon::NEXT, 16.0, false, "Next (Ctrl+Right)").clicked() {
        app.next(&ctx);
    }
    let (rc, rtip) = match app.playlist.repeat {
        Repeat::Off => (icon::REPEAT_ALL, "Repeat: off"),
        Repeat::All => (icon::REPEAT_ALL, "Repeat: all"),
        Repeat::One => (icon::REPEAT_ONE, "Repeat: one"),
    };
    if icon_button(ui, sq(cx + step * 3.4), rc, 16.0, app.playlist.repeat != Repeat::Off, rtip).clicked() {
        app.cycle_repeat();
    }

    // right side: tracks, volume, full screen, mini player, more
    let rsq = |i: f32| Rect::from_center_size(pos2(bottom.right() - 30.0 - i * 44.0, cy), vec2(40.0, 40.0));
    let more = rsq(0.0);
    if icon_button(ui, more, icon::MORE, 16.0, app.flyout == Some(Flyout::More), "More options").clicked() {
        toggle(app, Flyout::More);
    }
    if icon_button(ui, rsq(1.0), icon::MINI, 16.0, false, "Mini player").clicked() {
        app.toggle_mini(&ctx);
    }
    let (fc, ftip) = if app.fullscreen { (icon::EXIT_FULLSCREEN, "Exit full screen (F)") } else { (icon::FULLSCREEN, "Full screen (F)") };
    if icon_button(ui, rsq(2.0), fc, 16.0, false, ftip).clicked() {
        app.toggle_fullscreen(&ctx);
    }
    if icon_button(ui, rsq(3.0), volume_icon(app), 16.0, app.flyout == Some(Flyout::Volume), "Volume (M mutes)").clicked() {
        toggle(app, Flyout::Volume);
    }
    if icon_button(ui, rsq(4.0), icon::CAPTIONS, 16.0, app.flyout == Some(Flyout::Tracks), "Subtitles and audio").clicked() {
        toggle(app, Flyout::Tracks);
        app.tracks();
    }
    ANCHORS.with(|a| a.set([rsq(4.0), rsq(3.0), more]));
}

thread_local! {
    /// Where the flyout buttons were drawn this frame: tracks, volume, more.
    static ANCHORS: std::cell::Cell<[Rect; 3]> = const { std::cell::Cell::new([Rect::NOTHING; 3]) };
}

fn toggle(app: &mut PlayerApp, f: Flyout) {
    app.flyout = if app.flyout == Some(f) { None } else { Some(f) };
}

fn volume_icon(app: &PlayerApp) -> char {
    let v = app.engine.volume();
    if app.engine.muted() || v <= 0.0 {
        icon::MUTE
    } else {
        icon::VOLUME[((v * 3.0).ceil() as usize).min(3)]
    }
}

/// A scrubber drag seeks as it goes (at most every 60 ms, so a fast drag does not queue up seeks) and
/// once more where it is let go.
fn scrub(app: &mut PlayerApp, r: &Response, at: Option<f64>) {
    let now = std::time::Instant::now();
    if let Some(t) = at {
        let due = app.scrub.is_none_or(|(_, last)| now.duration_since(last).as_millis() >= 60);
        if due {
            app.seek(t);
            app.scrub = Some((t, now));
        } else if let Some(s) = app.scrub.as_mut() {
            s.0 = t;
            r.ctx.request_repaint_after(std::time::Duration::from_millis(60));
        }
    }
    if r.drag_stopped() || (r.clicked() && !r.dragged()) {
        if let Some((t, _)) = app.scrub.take() {
            app.seek(t);
        }
    }
    if !r.dragged() && !r.drag_started() && !r.is_pointer_button_down_on() {
        app.scrub = None;
    }
}

fn mini_chrome(app: &mut PlayerApp, ui: &mut Ui, full: Rect) {
    let ctx = ui.ctx().clone();
    let p = ui.painter().clone();
    p.rect_filled(full, 0.0, Color32::from_black_alpha(90));
    let close = Rect::from_min_size(pos2(full.right() - 36.0, full.top() + 4.0), vec2(32.0, 28.0));
    if icon_button(ui, close, icon::CLOSE, 11.0, false, "Close").clicked() {
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
    if icon_button(ui, close.translate(vec2(-34.0, 0.0)), icon::UNMINI, 14.0, false, "Back to full view").clicked() {
        app.toggle_mini(&ctx);
    }
    let c = full.center();
    if icon_button(ui, Rect::from_center_size(c - vec2(56.0, 0.0), vec2(40.0, 40.0)), icon::BACK10, 20.0, false, "Skip back 10 seconds").clicked() {
        app.skip(-SKIP_BACK);
    }
    if play_button(ui, c, 22.0, app.engine.is_playing()).clicked() {
        app.toggle_play();
    }
    if icon_button(ui, Rect::from_center_size(c + vec2(56.0, 0.0), vec2(40.0, 40.0)), icon::FORWARD30, 20.0, false, "Skip forward 30 seconds").clicked() {
        app.skip(SKIP_FORWARD);
    }
    let bar = Rect::from_min_max(pos2(full.left() + 4.0, full.bottom() - 22.0), pos2(full.right() - 4.0, full.bottom() - 2.0));
    let t = app.scrub.map_or_else(|| app.engine.time(), |(t, _)| t);
    let (r, at) = scrubber(ui, bar, t, app.engine.duration());
    scrub(app, &r, at);
}

fn flyout_frame() -> egui::Frame {
    egui::Frame::NONE
        .fill(FLYOUT_BG)
        .corner_radius(8.0)
        .inner_margin(egui::Margin::same(6))
        .stroke(Stroke::new(1.0, Color32::from_white_alpha(20)))
        .shadow(egui::Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(120) })
}

/// A full-width row in a flyout; `check` draws a tick in the left column.
fn flyout_row(ui: &mut Ui, check: Option<bool>, c: Option<char>, label: &str) -> Response {
    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
    let p = ui.painter();
    if r.hovered() {
        p.rect_filled(rect, 4.0, Color32::from_white_alpha(18));
    }
    if check == Some(true) {
        glyph(p, pos2(rect.left() + 16.0, rect.center().y), icon::CHECK, 12.0, ORANGE);
    }
    if let Some(c) = c {
        glyph(p, pos2(rect.left() + 16.0, rect.center().y), c, 14.0, Color32::WHITE);
    }
    p.text(pos2(rect.left() + 36.0, rect.center().y), Align2::LEFT_CENTER, label, FontId::proportional(13.0), Color32::WHITE);
    r
}

fn heading(ui: &mut Ui, s: &str) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new(s).size(12.0).color(Color32::from_gray(170)));
}

/// A row of choice chips (playback speed, zoom); returns the index clicked.
fn chips(ui: &mut Ui, labels: &[String], on: usize) -> Option<usize> {
    let mut hit = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        for (i, l) in labels.iter().enumerate() {
            let text = egui::RichText::new(l).size(12.0).color(if i == on { Color32::BLACK } else { Color32::WHITE });
            let b = egui::Button::new(text).fill(if i == on { ORANGE } else { Color32::from_white_alpha(16) }).corner_radius(12.0);
            if ui.add(b).clicked() {
                hit = Some(i);
            }
        }
    });
    hit
}

pub const SPEEDS: [f64; 7] = [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 2.0];

fn flyouts(app: &mut PlayerApp, ctx: &egui::Context, full: Rect) {
    let Some(f) = app.flyout else { return };
    let [tracks_at, volume_at, more_at] = ANCHORS.with(|a| a.get());
    let (anchor, width) = match f {
        Flyout::Tracks => (tracks_at, 300.0),
        Flyout::Volume => (volume_at, 300.0),
        Flyout::More => (more_at, 300.0),
    };
    if anchor == Rect::NOTHING {
        app.flyout = None;
        return;
    }
    let x = (anchor.center().x - width / 2.0).clamp(full.left() + 8.0, full.right() - width - 8.0);
    let area = egui::Area::new(egui::Id::new("player-flyout"))
        .order(egui::Order::Foreground)
        .pivot(Align2::LEFT_BOTTOM)
        .fixed_pos(pos2(x, anchor.top() - 8.0))
        .show(ctx, |ui| {
            flyout_frame().show(ui, |ui| {
                ui.set_width(width - 12.0);
                match f {
                    Flyout::Tracks => tracks_flyout(app, ui),
                    Flyout::Volume => volume_flyout(app, ui),
                    Flyout::More => more_flyout(app, ui),
                }
            });
        });
    // a press anywhere else closes it
    let rect = area.response.rect;
    if ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !rect.contains(p) && !anchor.contains(p))) {
        app.flyout = None;
    }
}

fn tracks_flyout(app: &mut PlayerApp, ui: &mut Ui) {
    let current_audio = app.engine.audio_stream();
    let current_sub = app.subtitle.as_ref().map(|(n, _)| *n);
    let Some(t) = app.tracks().cloned() else {
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().size(14.0));
            ui.label("Reading tracks…");
        });
        return;
    };
    heading(ui, "Audio");
    if t.audio.is_empty() {
        flyout_row(ui, None, None, "No audio");
    }
    for a in &t.audio {
        if flyout_row(ui, Some(a.index == current_audio), None, &a.label).clicked() && a.index != current_audio {
            app.engine.set_audio_stream(a.index);
        }
    }
    ui.add_space(4.0);
    heading(ui, "Subtitles");
    if flyout_row(ui, Some(current_sub.is_none()), None, "Off").clicked() {
        app.set_subtitle(None);
    }
    for (i, s) in t.subtitles.iter().enumerate() {
        let label = if s.bitmap { format!("{} (picture, not shown)", s.label) } else { s.label.clone() };
        if flyout_row(ui, Some(current_sub == Some(i)), None, &label).clicked() && !s.bitmap {
            app.set_subtitle(Some(i));
        }
    }
}

fn volume_flyout(app: &mut PlayerApp, ui: &mut Ui) {
    ui.horizontal(|ui| {
        let (rect, r) = ui.allocate_exact_size(vec2(36.0, 32.0), Sense::click());
        if r.hovered() {
            ui.painter().rect_filled(rect, 4.0, Color32::from_white_alpha(18));
        }
        glyph(ui.painter(), rect.center(), volume_icon(app), 16.0, Color32::WHITE);
        if r.on_hover_text(if app.engine.muted() { "Unmute (M)" } else { "Mute (M)" }).clicked() {
            app.toggle_mute();
        }
        let (bar, r) = ui.allocate_exact_size(vec2(196.0, 32.0), Sense::click_and_drag());
        let v = if app.engine.muted() { 0.0 } else { app.engine.volume() };
        let y = bar.center().y;
        let (x0, x1) = (bar.left() + 8.0, bar.right() - 8.0);
        let x = x0 + (x1 - x0) * v;
        let p = ui.painter();
        p.rect_filled(Rect::from_min_max(pos2(x0, y - 2.0), pos2(x1, y + 2.0)), 2.0, Color32::from_white_alpha(70));
        p.rect_filled(Rect::from_min_max(pos2(x0, y - 2.0), pos2(x, y + 2.0)), 2.0, ORANGE);
        p.circle_filled(pos2(x, y), 7.0, ORANGE);
        p.circle_stroke(pos2(x, y), 7.0, Stroke::new(1.5, Color32::WHITE));
        if let Some(pos) = r.interact_pointer_pos().filter(|_| r.dragged() || r.clicked()) {
            app.set_volume(((pos.x - x0) / (x1 - x0)).clamp(0.0, 1.0));
        }
        ui.label(egui::RichText::new(format!("{}", (v * 100.0).round() as u32)).size(13.0).color(Color32::WHITE));
    });
}

fn more_flyout(app: &mut PlayerApp, ui: &mut Ui) {
    heading(ui, "Playback speed");
    let rate = app.engine.rate();
    let labels: Vec<String> = SPEEDS.iter().map(|s| if *s == 1.0 { "Normal".into() } else { format!("{s}×") }).collect();
    let on = SPEEDS.iter().position(|s| (s - rate).abs() < 1e-6).unwrap_or(3);
    if let Some(i) = chips(ui, &labels, on) {
        app.engine.set_rate(SPEEDS[i]);
    }
    if app.info.as_ref().is_some_and(|i| i.has_video) {
        heading(ui, "Zoom");
        let labels: Vec<String> = Zoom::ALL.iter().map(|z| z.label().to_string()).collect();
        let on = Zoom::ALL.iter().position(|z| *z == app.zoom).unwrap_or(0);
        if let Some(i) = chips(ui, &labels, on) {
            app.zoom = Zoom::ALL[i];
        }
    }
    ui.add_space(6.0);
    ui.separator();
    if flyout_row(ui, None, Some(icon::INFO), "Media info").clicked() {
        app.info_open = true;
        app.flyout = None;
    }
    if flyout_row(ui, None, Some(icon::FOLDER), "Open file location").clicked() {
        app.open_folder();
        app.flyout = None;
    }
    if flyout_row(ui, None, Some(icon::EDIT), "Open in Editor").clicked() {
        app.flyout = None;
        app.open_editor();
    }
}

fn info_window(app: &mut PlayerApp, ctx: &egui::Context) {
    let mut open = true;
    let path = app.path().to_path_buf();
    let size_on_disk = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    egui::Window::new("Media info")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .frame(flyout_frame().inner_margin(egui::Margin::same(14)))
        .show(ctx, |ui| {
            egui::Grid::new("player-info").num_columns(2).spacing(vec2(18.0, 6.0)).show(ui, |ui| {
                let mut row = |k: &str, v: String| {
                    ui.label(egui::RichText::new(k).color(Color32::from_gray(170)));
                    ui.label(egui::RichText::new(v).color(Color32::WHITE));
                    ui.end_row();
                };
                row("File", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                row("Folder", path.parent().map(|p| p.display().to_string()).unwrap_or_default());
                row("Size", format!("{:.1} MB", size_on_disk as f64 / 1_048_576.0));
                if let Some(i) = &app.info {
                    row("Length", clock(i.duration));
                    if i.has_video {
                        row("Resolution", format!("{} × {}", i.size.0, i.size.1));
                        let fps = format!("{:.3}", i.fps);
                        row("Frame rate", format!("{} fps", fps.trim_end_matches('0').trim_end_matches('.')));
                        row("Video codec", i.codec.to_uppercase());
                        row("Decoder", i.decoder.to_string());
                        row("On screen", if app.video.zero_copy { "GPU, zero-copy".into() } else { "Uploaded".into() });
                        row("Dropped frames", app.engine.dropped().to_string());
                    }
                    let audio: Vec<String> = i.audio_streams.iter().map(|a| a.label()).collect();
                    row("Audio", if audio.is_empty() { "None".into() } else { audio.join(", ") });
                }
            });
        });
    if !open {
        app.info_open = false;
    }
}

/// Audio files: the cover (or a note on a gradient tile), the title, and a bar visualizer.
fn audio_view(app: &mut PlayerApp, ui: &mut Ui, full: Rect, mini: bool) {
    let p = ui.painter().clone();
    vgrad(&p, full, Color32::from_rgb(0x1E, 0x1B, 0x2A), Color32::from_rgb(0x0B, 0x0B, 0x10));
    let side = (full.height() * if mini { 0.55 } else { 0.42 }).min(full.width() * 0.5).max(48.0);
    let art = Rect::from_center_size(pos2(full.center().x, full.top() + full.height() * 0.38), vec2(side, side));
    match &app.cover {
        Some(c) => {
            let s = c.size_vec2();
            let k = (side / s.x).min(side / s.y);
            p.image(c.id(), Rect::from_center_size(art.center(), s * k), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        None => {
            let mut m = egui::Mesh::default();
            m.colored_vertex(art.left_top(), ORANGE);
            m.colored_vertex(art.right_top(), lerp(ORANGE, PINK, 0.5));
            m.colored_vertex(art.left_bottom(), lerp(ORANGE, PINK, 0.5));
            m.colored_vertex(art.right_bottom(), PINK);
            m.add_triangle(0, 1, 2);
            m.add_triangle(1, 3, 2);
            p.add(m);
            glyph(&p, art.center(), icon::MUSIC, side * 0.4, Color32::WHITE);
        }
    }
    if mini {
        return;
    }
    p.text(pos2(full.center().x, art.bottom() + 28.0), Align2::CENTER_CENTER, app.title(), FontId::proportional(22.0), Color32::WHITE);
    // visualizer: log-spaced bands of the last output samples, falling back slowly
    let tap = app.engine.tap();
    let n = 48;
    if app.bars.len() != n {
        app.bars = vec![0.0; n];
    }
    let fresh = bands(&tap, n);
    for (b, f) in app.bars.iter_mut().zip(fresh) {
        *b = if app.engine.is_playing() { f.max(*b * 0.82) } else { *b * 0.82 };
    }
    let base = full.bottom() - BOTTOM_H - 12.0;
    let w = (full.width() * 0.6).min(720.0);
    let bw = w / n as f32;
    let x0 = full.center().x - w / 2.0;
    for (i, b) in app.bars.iter().enumerate() {
        let h = (b * 90.0).clamp(2.0, 90.0);
        let r = Rect::from_min_max(pos2(x0 + i as f32 * bw + 1.0, base - h), pos2(x0 + (i + 1) as f32 * bw - 1.0, base));
        p.rect_filled(r, 1.5, lerp(ORANGE, PINK, i as f32 / n as f32).gamma_multiply(0.85));
    }
}

/// Magnitudes (0..1) of `n` log-spaced frequency bands of `x` (a small DFT: 512 samples x n bins).
pub fn bands(x: &[f32], n: usize) -> Vec<f32> {
    let len = x.len().min(512);
    if len < 64 {
        return vec![0.0; n];
    }
    let x = &x[x.len() - len..];
    (0..n)
        .map(|i| {
            // 60 Hz .. 12 kHz at 48 kHz
            let f = 60.0 * (200f32).powf(i as f32 / (n - 1) as f32) / crate::media::SAMPLE_RATE as f32;
            let (mut re, mut im) = (0.0f32, 0.0f32);
            for (k, s) in x.iter().enumerate() {
                let w = 0.5 - 0.5 * (std::f32::consts::TAU * k as f32 / len as f32).cos(); // Hann
                let a = std::f32::consts::TAU * f * k as f32;
                re += s * w * a.cos();
                im -= s * w * a.sin();
            }
            let mag = (re * re + im * im).sqrt() / len as f32 * 4.0;
            (1.0 + mag * 30.0).ln() / (31f32).ln()
        })
        .map(|v| v.clamp(0.0, 1.0))
        .collect()
}

/// The cue at the playhead, white with a dark edge, above the controls while they show.
fn subtitles(app: &PlayerApp, ui: &mut Ui, video: Rect, chrome_up: bool) {
    let Some((_, Some(cues))) = &app.subtitle else { return };
    let Some(text) = super::tracks::cue_at(cues, app.engine.time()) else { return };
    let px = (video.height() * 0.05).clamp(16.0, 44.0);
    let p = ui.painter();
    let g = p.layout(text.to_string(), FontId::proportional(px), Color32::WHITE, video.width() * 0.8);
    let lift = if chrome_up { BOTTOM_H } else { video.height() * 0.06 };
    let at = pos2(video.center().x - g.size().x / 2.0, (video.bottom() - lift - g.size().y).min(ui.max_rect().bottom() - lift - g.size().y));
    for (dx, dy) in [(-1.5, 0.0), (1.5, 0.0), (0.0, -1.5), (0.0, 1.5), (-1.0, -1.0), (1.0, 1.0), (-1.0, 1.0), (1.0, -1.0)] {
        p.galley(at + vec2(dx, dy), g.clone(), Color32::from_black_alpha(200));
    }
    p.galley(at, g, Color32::WHITE);
}

/// The 32 px picture of `assets/app.ico` (its entries are PNGs).
fn app_icon(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    static ICO: &[u8] = include_bytes!("../../assets/app.ico");
    let n = u16::from_le_bytes([ICO[4], ICO[5]]) as usize;
    let entry = (0..n).map(|i| &ICO[6 + 16 * i..22 + 16 * i]).find(|e| e[0] == 32)?;
    let size = u32::from_le_bytes([entry[8], entry[9], entry[10], entry[11]]) as usize;
    let off = u32::from_le_bytes([entry[12], entry[13], entry[14], entry[15]]) as usize;
    let f = crate::engine::html::decode_png(ICO.get(off..off + size)?).ok()?;
    let img = egui::ColorImage::from_rgba_unmultiplied([f.width as usize, f.height as usize], &f.rgba);
    Some(ctx.load_texture("player-app-icon", img, egui::TextureOptions::LINEAR))
}

/// An audio file's cover: the embedded picture, else cover / folder / front .jpg / .png beside it.
pub fn cover_art(path: &Path) -> Option<egui::ColorImage> {
    let ff = crate::media::ffpipe::ffmpeg_exe()?;
    let grab = |src: &Path| -> Option<egui::ColorImage> {
        let out = crate::media::ffpipe::command(&ff)
            .args(["-v", "error", "-i"])
            .arg(src)
            .args(["-an", "-frames:v", "1", "-vf", "scale='min(800,iw)':-2", "-f", "image2pipe", "-c:v", "png", "pipe:1"])
            .output()
            .ok()?;
        let f = crate::engine::html::decode_png(&out.stdout).ok()?;
        Some(egui::ColorImage::from_rgba_unmultiplied([f.width as usize, f.height as usize], &f.rgba))
    };
    grab(path).or_else(|| {
        let dir = path.parent()?;
        ["cover", "folder", "front", "Cover", "Folder", "Front"]
            .iter()
            .flat_map(|n| ["jpg", "png", "jpeg"].map(|e| dir.join(format!("{n}.{e}"))))
            .find(|p| p.is_file())
            .and_then(|p| grab(&p))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_reads_like_media_player() {
        assert_eq!(clock(41.7), "0:00:41");
        assert_eq!(clock(190.0), "0:03:10");
        assert_eq!(clock(3725.0), "1:02:05");
    }

    /// Every icon the player draws exists in the installed Windows icon font(s).
    #[test]
    fn player_icons_exist_in_the_icon_fonts() {
        let dir = std::path::PathBuf::from(std::env::var_os("WINDIR").unwrap_or_else(|| r"C:\Windows".into()));
        let mut checked = 0;
        for file in ["SegoeIcons.ttf", "segmdl2.ttf"] {
            let Ok(bytes) = std::fs::read(dir.join("Fonts").join(file)) else { continue };
            let mut defs = egui::FontDefinitions::empty();
            defs.font_data.insert(file.into(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
            defs.families.insert(egui::FontFamily::Proportional, vec![file.into()]);
            let ctx = egui::Context::default();
            ctx.set_fonts(defs);
            let _ = ctx.run(egui::RawInput::default(), |_| {});
            for c in icon::ALL {
                let ok = ctx.fonts_mut(|f| f.has_glyph(&FontId::proportional(16.0), c));
                assert!(ok, "U+{:04X} is missing from {file}", c as u32);
                checked += 1;
            }
        }
        assert!(checked > 0, "no icon font installed");
    }

    #[test]
    fn zoom_places_the_picture() {
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(1600.0, 900.0));
        assert_eq!(Zoom::Fit.place(area, (1920, 800), 1.0).size(), vec2(1600.0, 1600.0 * 800.0 / 1920.0));
        assert_eq!(Zoom::Fill.place(area, (1920, 800), 1.0).height(), 900.0);
        assert_eq!(Zoom::Stretch.place(area, (640, 480), 1.0), area);
        assert_eq!(Zoom::Original.place(area, (640, 480), 2.0).size(), vec2(320.0, 240.0));
    }

    #[test]
    fn bands_hear_a_tone_in_the_right_band() {
        let tone: Vec<f32> = (0..512).map(|i| (std::f32::consts::TAU * 1000.0 * i as f32 / 48000.0).sin() * 0.5).collect();
        let b = bands(&tone, 48);
        let peak = b.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0;
        // 1 kHz on a 60 Hz..12 kHz log scale of 48 bands: about band 25
        assert!((22..=28).contains(&peak), "peak in band {peak}: {b:?}");
        assert!(bands(&[0.0; 512], 8).iter().all(|v| *v == 0.0));
    }
}
