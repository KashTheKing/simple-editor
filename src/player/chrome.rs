//! The player's look: Simple Editor's own theme (the palette, Look and accent picked in Settings ▸
//! Appearance, read by `PlayerApp::new`) on a media player's layout - the picture edge to edge, a
//! title strip on top (Editor button, title, window buttons) and a bottom bar over the video: a thick
//! scrub bar, the timecode, the editor's five play buttons with skip 10 / 30 s in place of frame steps,
//! then subtitles and audio, volume, full screen, mini player and more. Both fade with the pointer
//! (`ui::autohide`). Every control is a big target: 48-52 pt buttons, a 64 pt play button, a scrub
//! bar that grabs anywhere in its 28 pt height.
//!
//! Icons are the editor's glyphs (Windows' icon font, Segoe Fluent Icons / MDL2 Assets).

use super::{Flyout, PlayerApp, Zoom, SKIP_BACK, SKIP_FORWARD};
use crate::player::folder::Repeat;
use crate::theme::Palette;
use crate::ui::autohide;
use crate::ui::tools::on_accent;
use eframe::egui::{self, pos2, vec2, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Response, Sense, Stroke, Ui};
use std::path::Path;

/// Icon-font codepoints; the transport's are the editor's `ui::tools::Glyph` ones.
pub mod icon {
    pub const EDIT: char = '\u{E70F}'; // Glyph::Pencil
    pub const PREVIOUS: char = '\u{F8AC}'; // Glyph::Jump(Left)
    pub const NEXT: char = '\u{F8AD}'; // Glyph::Jump(Right)
    pub const PLAY: char = '\u{F5B0}'; // Glyph::Play
    pub const PAUSE: char = '\u{F8AE}'; // Glyph::Pause
    pub const BACK10: char = '\u{ED3C}';
    pub const FORWARD30: char = '\u{ED3D}';
    pub const CAPTIONS: char = '\u{E7F0}'; // Glyph::Subtitles
    pub const VOLUME: [char; 4] = ['\u{E992}', '\u{E993}', '\u{E994}', '\u{E995}'];
    pub const MUTE: char = '\u{E74F}'; // Glyph::SpeakerOff
    pub const FULLSCREEN: char = '\u{E740}'; // Glyph::Fullscreen
    pub const EXIT_FULLSCREEN: char = '\u{E73F}';
    pub const MINI: char = '\u{E92C}';
    pub const UNMINI: char = '\u{E92D}';
    pub const MORE: char = '\u{E712}';
    pub const MINIMIZE: char = '\u{E921}';
    pub const MAXIMIZE: char = '\u{E922}';
    pub const RESTORE: char = '\u{E923}';
    pub const CLOSE: char = '\u{E8BB}';
    pub const MUSIC: char = '\u{E8D6}';
    pub const FOLDER: char = '\u{E8B7}'; // Glyph::Folder
    pub const INFO: char = '\u{E946}';
    pub const CHECK: char = '\u{E73E}';
    pub const ALL: [char; 26] = [
        EDIT, PREVIOUS, NEXT, PLAY, PAUSE, BACK10, FORWARD30, CAPTIONS, VOLUME[0], VOLUME[1], VOLUME[2], VOLUME[3], MUTE,
        FULLSCREEN, EXIT_FULLSCREEN, MINI, UNMINI, MORE, MINIMIZE, MAXIMIZE, RESTORE, CLOSE, MUSIC, FOLDER, INFO, CHECK,
    ];
}

const TOP_H: f32 = 40.0;
/// The bottom bar and the margin around it.
const BAR_H: f32 = 104.0;
const MARGIN: f32 = 12.0;
pub const BOTTOM_H: f32 = BAR_H + MARGIN;
/// Button sides (points): the transport, the play button, the bar's other buttons.
const BTN: f32 = 52.0;
const PLAY: f32 = 64.0;
const SIDE: f32 = 48.0;
/// The scrub bar's grab height; the line drawn in it is `TRACK` thick (more under the pointer).
const SCRUB_H: f32 = 28.0;
const TRACK: f32 = 6.0;

/// The editor's timecode (hh:mm:ss:ff) at the file's frame rate.
fn tc(app: &PlayerApp, t: f64) -> String {
    crate::ui::timecode(t, app.info.as_ref().map_or(30.0, |i| i.fps))
}

fn icon_font(px: f32) -> FontId {
    FontId::new(px, crate::theme::icons())
}

fn glyph(p: &egui::Painter, at: Pos2, c: char, px: f32, col: Color32) {
    p.text(at, Align2::CENTER_CENTER, c, icon_font(px), col);
}

fn radius(ui: &Ui) -> CornerRadius {
    ui.visuals().widgets.inactive.corner_radius
}

/// A flat icon button, as the editor's: the hover fill under the pointer, accent while pressed, the
/// glyph in the accent when `on`. The whole `rect` is the hit area.
fn icon_button(ui: &mut Ui, pal: &Palette, rect: Rect, c: char, px: f32, on: bool, tip: &str) -> Response {
    let id = ui.id().with(("btn", c, rect.min.x as i32, rect.min.y as i32));
    let r = ui.interact(rect, id, Sense::click()).on_hover_text(tip);
    let p = ui.painter();
    let cr = radius(ui);
    let mut fg = if on { pal.accent } else { pal.text };
    if r.is_pointer_button_down_on() {
        p.rect_filled(rect.shrink(2.0), cr, pal.accent);
        fg = on_accent(pal.accent);
    } else if r.hovered() {
        p.rect_filled(rect.shrink(2.0), cr, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    glyph(p, rect.center(), c, px, fg);
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

/// The play button: an accent disc, lighter under the pointer, the glyph in the accent's text colour.
fn play_button(ui: &mut Ui, pal: &Palette, center: Pos2, side: f32, playing: bool) -> Response {
    let rect = Rect::from_center_size(center, vec2(side, side));
    let tip = if playing { "Pause (Space)" } else { "Play (Space)" };
    let r = ui.interact(rect, ui.id().with(("play", center.x as i32, center.y as i32)), Sense::click()).on_hover_text(tip);
    let fill = if r.is_pointer_button_down_on() {
        lerp(pal.accent, Color32::BLACK, 0.15)
    } else if r.hovered() {
        lerp(pal.accent, Color32::WHITE, 0.15)
    } else {
        pal.accent
    };
    let p = ui.painter();
    p.circle_filled(center, side / 2.0, fill);
    let c = if playing { icon::PAUSE } else { icon::PLAY };
    glyph(p, center + vec2(if playing { 0.0 } else { side * 0.03 }, 0.0), c, side * 0.38, on_accent(pal.accent));
    r
}

/// The scrub bar: the editor's border-coloured track with the played part in the accent, a thumb that
/// grows under the pointer. All of `rect` grabs. Returns the time under the pointer while dragged /
/// clicked.
fn scrubber(app: &PlayerApp, ui: &mut Ui, pal: &Palette, rect: Rect, t: f64, dur: f64) -> (Response, Option<f64>) {
    let r = ui.interact(rect, ui.id().with("scrub"), Sense::click_and_drag());
    let p = ui.painter();
    let (x0, x1) = (rect.left() + 8.0, rect.right() - 8.0);
    let y = rect.center().y;
    let k = if dur > 0.0 { (t / dur).clamp(0.0, 1.0) as f32 } else { 0.0 };
    let x = x0 + (x1 - x0) * k;
    let hot = r.hovered() || r.dragged();
    let th = if hot { TRACK + 2.0 } else { TRACK };
    let line = |a: f32, b: f32, c| p.rect_filled(Rect::from_min_max(pos2(a, y - th / 2.0), pos2(b, y + th / 2.0)), th / 2.0, c);
    line(x0, x1, pal.border);
    line(x0, x, pal.accent);
    let rad = if hot { 10.0 } else { 8.0 };
    p.circle_filled(pos2(x, y), rad, pal.accent);
    p.circle_stroke(pos2(x, y), rad, Stroke::new(2.0, pal.panel));
    let at = |px: f32| (((px - x0) / (x1 - x0)).clamp(0.0, 1.0) as f64) * dur;
    let mut seek = None;
    if let Some(pos) = r.interact_pointer_pos().filter(|_| r.dragged() || r.clicked() || r.drag_started()) {
        seek = Some(at(pos.x));
    }
    if let Some(h) = r.hover_pos() {
        let g = p.layout_no_wrap(tc(app, at(h.x)), FontId::proportional(13.0), pal.text);
        let box_ = Rect::from_center_size(pos2(h.x, rect.top() - 16.0), g.size() + vec2(14.0, 8.0));
        p.rect(box_, radius(ui), pal.panel, Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
        p.galley(box_.center() - g.size() / 2.0, g, pal.text);
    }
    (r, seek)
}

/// Draw the frame: picture, subtitles, chrome, flyouts. Returns what the auto-hide needs repainted.
pub fn draw(app: &mut PlayerApp, ctx: &egui::Context, now: f64) -> autohide::Output {
    let pal = crate::theme::palette_with(ctx, &app.look);
    let mut out = autohide::Output { opacity: 0.0, repaint_in: None, hide_cursor: false, dismissed: false };
    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(Color32::BLACK)).show(ctx, |ui| {
        out = body(app, ui, &pal, now);
    });
    out
}

fn body(app: &mut PlayerApp, ui: &mut Ui, pal: &Palette, now: f64) -> autohide::Output {
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
        Some((false, _)) => audio_view(app, ui, pal, full, mini),
        None => {}
    }
    if let Some(e) = &app.error {
        let msg = format!("Can't play this file\n{e}");
        ui.painter().text(full.center(), Align2::CENTER_CENTER, msg, FontId::proportional(15.0), Color32::from_gray(200));
    }
    subtitles(app, ui, video_rect.unwrap_or(full).intersect(full), app.hide.shown());

    // ---- the video surface: click = play/pause, double-click = full screen; in the mini player a
    // drag moves the window
    let surface = ui.interact(full, ui.id().with("surface"), Sense::click_and_drag());
    let clicked = surface.clicked() && !surface.double_clicked();
    if surface.double_clicked() {
        app.double_click_video(&ctx);
    } else if clicked {
        app.click_video(now);
    }
    if mini && surface.drag_started() {
        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }

    // ---- auto-hide
    let top = Rect::from_min_size(full.min, vec2(full.width(), TOP_H));
    let bottom = Rect::from_min_max(pos2(full.left(), full.bottom() - if mini { 72.0 } else { BOTTOM_H }), full.max);
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
        // (`SE_PLAYER_SHOT` pictures the controls: they hold for it)
        hold: app.flyout.is_some() || app.scrub.is_some() || app.info_open || app.shot.is_some(),
        // a click on the picture, not on a bar (the bars are drawn over `surface`)
        dismiss: clicked && !over_chrome,
    };
    let out = app.autohide(input);
    if out.dismissed {
        app.cancel_click(); // it only hid the controls
    }
    if out.hide_cursor {
        ctx.set_cursor_icon(egui::CursorIcon::None);
    }
    if out.opacity > 0.0 {
        let mut ui = ui.new_child(egui::UiBuilder::new().max_rect(full));
        ui.set_opacity(out.opacity);
        if mini {
            mini_chrome(app, &mut ui, pal, full);
        } else {
            top_bar(app, &mut ui, pal, top, maximized);
            bottom_bar(app, &mut ui, pal, bottom);
        }
    }
    if !(maximized || app.fullscreen || mini) {
        resize_edges(&ctx, full);
    }
    flyouts(app, &ctx, pal, full);
    if app.info_open {
        info_window(app, &ctx, pal);
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

/// A strip in the theme's panel colour, a hair see-through over the picture.
fn strip(p: &egui::Painter, pal: &Palette, rect: Rect, cr: CornerRadius) {
    p.rect(rect, cr, pal.panel.gamma_multiply(0.94), Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
}

fn top_bar(app: &mut PlayerApp, ui: &mut Ui, pal: &Palette, top: Rect, maximized: bool) {
    let ctx = ui.ctx().clone();
    // edge to edge: the side and top borders fall just outside the window
    strip(ui.painter(), pal, top.expand2(vec2(1.0, 0.0)).translate(vec2(0.0, -1.0)), CornerRadius::ZERO);
    // the empty bar drags the window, a double-click maximizes
    let bar = ui.interact(top, ui.id().with("titlebar"), Sense::click_and_drag());
    if bar.drag_started() && !app.fullscreen {
        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    if bar.double_clicked() && !app.fullscreen {
        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
    }
    // Editor: glyph + name, as the editor's toolbar buttons
    let b = Rect::from_min_size(top.min + vec2(4.0, 4.0), vec2(92.0, TOP_H - 8.0));
    let r = ui.interact(b, ui.id().with("editor"), Sense::click()).on_hover_text("Open in Editor (Esc)");
    let p = ui.painter();
    if r.hovered() {
        p.rect_filled(b, radius(ui), ui.visuals().widgets.hovered.weak_bg_fill);
    }
    glyph(p, pos2(b.left() + 18.0, b.center().y), icon::EDIT, 15.0, pal.text);
    p.text(pos2(b.left() + 34.0, b.center().y), Align2::LEFT_CENTER, "Editor", FontId::proportional(14.0), pal.text);
    if r.clicked() {
        app.open_editor();
    }
    let icon_at = pos2(b.right() + 12.0, top.center().y);
    if app.icon.is_none() {
        app.icon = app_icon(&ctx);
    }
    if let Some(i) = &app.icon {
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        ui.painter().image(i.id(), Rect::from_min_size(icon_at - vec2(0.0, 8.0), vec2(16.0, 16.0)), uv, Color32::WHITE);
    }
    let caption_w = if app.fullscreen { 0.0 } else { 3.0 * 46.0 };
    let title_w = (top.right() - caption_w - icon_at.x - 40.0).max(0.0);
    let mut job = egui::text::LayoutJob::simple_singleline(app.title(), FontId::proportional(14.0), pal.text);
    job.wrap = egui::text::TextWrapping::truncate_at_width(title_w);
    let g = ui.painter().layout_job(job);
    ui.painter().galley(pos2(icon_at.x + 26.0, top.center().y - g.size().y / 2.0), g, pal.text);
    if app.fullscreen {
        return;
    }
    let w = 46.0;
    let close = Rect::from_min_size(pos2(top.right() - w, top.top()), vec2(w, TOP_H));
    let max = close.translate(vec2(-w, 0.0));
    let min = max.translate(vec2(-w, 0.0));
    if caption_button(ui, pal, min, icon::MINIMIZE, false, "Minimize").clicked() {
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
    }
    let (c, tip) = if maximized { (icon::RESTORE, "Restore down") } else { (icon::MAXIMIZE, "Maximize") };
    if caption_button(ui, pal, max, c, false, tip).clicked() {
        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
    }
    if caption_button(ui, pal, close, icon::CLOSE, true, "Close").clicked() {
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

/// Window buttons: square, the close one red under the pointer (Windows' convention).
fn caption_button(ui: &mut Ui, pal: &Palette, rect: Rect, c: char, close: bool, tip: &str) -> Response {
    let r = ui.interact(rect, ui.id().with(("caption", c)), Sense::click()).on_hover_text(tip);
    let mut fg = pal.text;
    if r.hovered() {
        let fill = if close { Color32::from_rgb(0xC4, 0x2B, 0x1C) } else { ui.visuals().widgets.hovered.weak_bg_fill };
        ui.painter().rect_filled(rect, 0.0, fill);
        if close {
            fg = Color32::WHITE;
        }
    }
    glyph(ui.painter(), rect.center(), c, 10.0, fg);
    r
}

fn bottom_bar(app: &mut PlayerApp, ui: &mut Ui, pal: &Palette, bottom: Rect) {
    let ctx = ui.ctx().clone();
    let bar = Rect::from_min_max(pos2(bottom.left() + MARGIN, bottom.top()), pos2(bottom.right() - MARGIN, bottom.bottom() - MARGIN));
    strip(ui.painter(), pal, bar, ui.visuals().window_corner_radius);
    // the bar itself swallows clicks (no play/pause through it)
    ui.interact(bottom, ui.id().with("bottom"), Sense::click());
    let dur = app.engine.duration();
    let t = app.scrub.map_or_else(|| app.engine.time(), |(t, _)| t);
    let scrub_rect = Rect::from_min_size(pos2(bar.left() + 10.0, bar.top() + 6.0), vec2(bar.width() - 20.0, SCRUB_H));
    let (r, at) = scrubber(app, ui, pal, scrub_rect, t, dur);
    scrub(app, &r, at);

    // controls row: timecode | the five play buttons | tracks, volume, full screen, mini, more
    let cy = (scrub_rect.bottom() + bar.bottom()) / 2.0 + 2.0;
    let p = ui.painter().clone();
    let now_g = p.layout_no_wrap(tc(app, t), FontId::proportional(16.0), pal.text);
    let dur_g = p.layout_no_wrap(format!(" / {}", tc(app, dur)), FontId::proportional(16.0), pal.text_dim);
    let x = bar.left() + 18.0;
    let tc_right = x + now_g.size().x + dur_g.size().x;
    p.galley(pos2(x, cy - now_g.size().y / 2.0), now_g.clone(), pal.text);
    p.galley(pos2(x + now_g.size().x, cy - dur_g.size().y / 2.0), dur_g, pal.text_dim);

    let side = |i: f32| Rect::from_center_size(pos2(bar.right() - 10.0 - SIDE / 2.0 - i * SIDE, cy), vec2(SIDE, SIDE));
    let more = side(0.0);
    if icon_button(ui, pal, more, icon::MORE, 18.0, app.flyout == Some(Flyout::More), "More").clicked() {
        toggle(app, Flyout::More);
    }
    if icon_button(ui, pal, side(1.0), icon::MINI, 18.0, false, "Mini player").clicked() {
        app.toggle_mini(&ctx);
    }
    let (fc, ftip) = if app.fullscreen { (icon::EXIT_FULLSCREEN, "Exit full screen (F)") } else { (icon::FULLSCREEN, "Full screen (F)") };
    if icon_button(ui, pal, side(2.0), fc, 18.0, false, ftip).clicked() {
        app.toggle_fullscreen(&ctx);
    }
    let slider = Rect::from_min_max(pos2(side(2.0).left() - 8.0 - 120.0, cy - 20.0), pos2(side(2.0).left() - 8.0, cy + 20.0));
    volume_slider(app, ui, pal, slider);
    let speaker = Rect::from_center_size(pos2(slider.left() - SIDE / 2.0, cy), vec2(SIDE, SIDE));
    let mtip = if app.engine.muted() { "Unmute (M)" } else { "Mute (M)" };
    if icon_button(ui, pal, speaker, volume_icon(app), 18.0, false, mtip).clicked() {
        app.toggle_mute();
    }
    let tracks = speaker.translate(vec2(-SIDE, 0.0));
    let on = app.flyout == Some(Flyout::Tracks);
    if icon_button(ui, pal, tracks, icon::CAPTIONS, 18.0, on, "Subtitles and audio").clicked() {
        toggle(app, Flyout::Tracks);
        app.tracks();
    }
    ANCHORS.with(|a| a.set([tracks, more]));

    // the play buttons, centred - pushed right of the timecode / left of the right group when narrow
    let w = 4.0 * BTN + PLAY + 4.0 * 8.0;
    let cx = bar.center().x.min(tracks.left() - 12.0 - w / 2.0).max(tc_right + 12.0 + w / 2.0);
    let sq = |dx: f32| Rect::from_center_size(pos2(cx + dx, cy), vec2(BTN, BTN));
    let (near, far) = (PLAY / 2.0 + 8.0 + BTN / 2.0, PLAY / 2.0 + 16.0 + BTN * 1.5);
    if icon_button(ui, pal, sq(-far), icon::PREVIOUS, 20.0, false, "Previous (Ctrl+Left)").clicked() {
        app.prev(&ctx);
    }
    if icon_button(ui, pal, sq(-near), icon::BACK10, 24.0, false, "Back 10 seconds (Left)").clicked() {
        app.skip(-SKIP_BACK);
    }
    if play_button(ui, pal, pos2(cx, cy), PLAY, app.engine.is_playing()).clicked() {
        app.toggle_play();
    }
    if icon_button(ui, pal, sq(near), icon::FORWARD30, 24.0, false, "Forward 30 seconds (Right)").clicked() {
        app.skip(SKIP_FORWARD);
    }
    if icon_button(ui, pal, sq(far), icon::NEXT, 20.0, false, "Next (Ctrl+Right)").clicked() {
        app.next(&ctx);
    }
}

/// Volume, inline in the bar: the scrub bar's track and thumb; all of `rect` grabs.
fn volume_slider(app: &mut PlayerApp, ui: &mut Ui, pal: &Palette, rect: Rect) {
    let v = if app.engine.muted() { 0.0 } else { app.engine.volume() };
    let r = ui.interact(rect, ui.id().with("volume"), Sense::click_and_drag());
    let r = r.on_hover_text(format!("Volume {}", (v * 100.0).round() as u32));
    let (x0, x1, y) = (rect.left() + 8.0, rect.right() - 8.0, rect.center().y);
    let x = x0 + (x1 - x0) * v;
    let hot = r.hovered() || r.dragged();
    let th = if hot { TRACK } else { TRACK - 2.0 };
    let p = ui.painter();
    p.rect_filled(Rect::from_min_max(pos2(x0, y - th / 2.0), pos2(x1, y + th / 2.0)), th / 2.0, pal.border);
    p.rect_filled(Rect::from_min_max(pos2(x0, y - th / 2.0), pos2(x, y + th / 2.0)), th / 2.0, pal.accent);
    let rad = if hot { 9.0 } else { 7.0 };
    p.circle_filled(pos2(x, y), rad, pal.accent);
    p.circle_stroke(pos2(x, y), rad, Stroke::new(2.0, pal.panel));
    if let Some(pos) = r.interact_pointer_pos().filter(|_| r.dragged() || r.clicked() || r.drag_started()) {
        app.set_volume(((pos.x - x0) / (x1 - x0)).clamp(0.0, 1.0));
    }
}

thread_local! {
    /// Where the flyout buttons were drawn this frame: tracks, more.
    static ANCHORS: std::cell::Cell<[Rect; 2]> = const { std::cell::Cell::new([Rect::NOTHING; 2]) };
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

fn mini_chrome(app: &mut PlayerApp, ui: &mut Ui, pal: &Palette, full: Rect) {
    let ctx = ui.ctx().clone();
    let p = ui.painter().clone();
    p.rect_filled(full, 0.0, Color32::from_black_alpha(90));
    let close = Rect::from_min_size(pos2(full.right() - 44.0, full.top() + 4.0), vec2(40.0, 36.0));
    if icon_button(ui, pal, close, icon::CLOSE, 12.0, false, "Close").clicked() {
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
    if icon_button(ui, pal, close.translate(vec2(-42.0, 0.0)), icon::UNMINI, 15.0, false, "Back to full view").clicked() {
        app.toggle_mini(&ctx);
    }
    let c = full.center() - vec2(0.0, 8.0);
    let gap = PLAY / 2.0 + 8.0 + BTN / 2.0;
    let sq = |dx: f32| Rect::from_center_size(c + vec2(dx, 0.0), vec2(BTN, BTN));
    if icon_button(ui, pal, sq(-gap), icon::BACK10, 22.0, false, "Back 10 seconds").clicked() {
        app.skip(-SKIP_BACK);
    }
    if play_button(ui, pal, c, 56.0, app.engine.is_playing()).clicked() {
        app.toggle_play();
    }
    if icon_button(ui, pal, sq(gap), icon::FORWARD30, 22.0, false, "Forward 30 seconds").clicked() {
        app.skip(SKIP_FORWARD);
    }
    let bar = Rect::from_min_max(pos2(full.left() + 6.0, full.bottom() - SCRUB_H - 4.0), pos2(full.right() - 6.0, full.bottom() - 4.0));
    let t = app.scrub.map_or_else(|| app.engine.time(), |(t, _)| t);
    let (r, at) = scrubber(app, ui, pal, bar, t, app.engine.duration());
    scrub(app, &r, at);
}

fn flyout_frame(pal: &Palette, ctx: &egui::Context) -> egui::Frame {
    egui::Frame::NONE
        .fill(pal.panel)
        .corner_radius(ctx.style().visuals.window_corner_radius)
        .inner_margin(egui::Margin::same(6))
        .stroke(Stroke::new(1.0, pal.border))
        .shadow(egui::Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(120) })
}

/// A full-width row in a flyout; `check` draws a tick in the left column.
fn flyout_row(ui: &mut Ui, pal: &Palette, check: Option<bool>, c: Option<char>, label: &str) -> Response {
    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
    let p = ui.painter();
    if r.hovered() {
        p.rect_filled(rect, radius(ui), ui.visuals().widgets.hovered.weak_bg_fill);
    }
    if check == Some(true) {
        glyph(p, pos2(rect.left() + 18.0, rect.center().y), icon::CHECK, 13.0, pal.accent);
    }
    if let Some(c) = c {
        glyph(p, pos2(rect.left() + 18.0, rect.center().y), c, 15.0, pal.text);
    }
    p.text(pos2(rect.left() + 40.0, rect.center().y), Align2::LEFT_CENTER, label, FontId::proportional(14.0), pal.text);
    r
}

fn heading(ui: &mut Ui, pal: &Palette, s: &str) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new(s).size(12.0).color(pal.text_dim));
}

/// A row of choice chips (playback speed, zoom); returns the index clicked.
fn chips(ui: &mut Ui, pal: &Palette, labels: &[String], on: usize) -> Option<usize> {
    let mut hit = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        for (i, l) in labels.iter().enumerate() {
            let sel = i == on;
            let text = egui::RichText::new(l).size(13.0).color(if sel { on_accent(pal.accent) } else { pal.text });
            let fill = if sel { pal.accent } else { ui.visuals().widgets.inactive.weak_bg_fill };
            let b = egui::Button::new(text).fill(fill).min_size(vec2(44.0, 30.0));
            if ui.add(b).clicked() {
                hit = Some(i);
            }
        }
    });
    hit
}

pub const SPEEDS: [f64; 7] = [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 2.0];

fn flyouts(app: &mut PlayerApp, ctx: &egui::Context, pal: &Palette, full: Rect) {
    let Some(f) = app.flyout else { return };
    let [tracks_at, more_at] = ANCHORS.with(|a| a.get());
    let anchor = match f {
        Flyout::Tracks => tracks_at,
        Flyout::More => more_at,
    };
    let width = 300.0;
    if anchor == Rect::NOTHING {
        app.flyout = None;
        return;
    }
    let x = (anchor.center().x - width / 2.0).clamp(full.left() + 8.0, full.right() - width - 8.0);
    let area = egui::Area::new(egui::Id::new("player-flyout"))
        .order(egui::Order::Foreground)
        .pivot(Align2::LEFT_BOTTOM)
        .fixed_pos(pos2(x, anchor.top() - 14.0))
        .show(ctx, |ui| {
            flyout_frame(pal, ctx).show(ui, |ui| {
                ui.set_width(width - 12.0);
                match f {
                    Flyout::Tracks => tracks_flyout(app, ui, pal),
                    Flyout::More => more_flyout(app, ui, pal),
                }
            });
        });
    // a press anywhere else closes it
    let rect = area.response.rect;
    if ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !rect.contains(p) && !anchor.contains(p))) {
        app.flyout = None;
    }
}

fn tracks_flyout(app: &mut PlayerApp, ui: &mut Ui, pal: &Palette) {
    let current_audio = app.engine.audio_stream();
    let current_sub = app.subtitle.as_ref().map(|(n, _)| *n);
    let Some(t) = app.tracks().cloned() else {
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().size(14.0));
            ui.label(egui::RichText::new("Reading tracks…").color(pal.text));
        });
        return;
    };
    heading(ui, pal, "Audio");
    if t.audio.is_empty() {
        flyout_row(ui, pal, None, None, "No audio");
    }
    for a in &t.audio {
        if flyout_row(ui, pal, Some(a.index == current_audio), None, &a.label).clicked() && a.index != current_audio {
            app.engine.set_audio_stream(a.index);
        }
    }
    ui.add_space(4.0);
    heading(ui, pal, "Subtitles");
    if flyout_row(ui, pal, Some(current_sub.is_none()), None, "Off").clicked() {
        app.set_subtitle(None);
    }
    for (i, s) in t.subtitles.iter().enumerate() {
        let label = if s.bitmap { format!("{} (picture, not shown)", s.label) } else { s.label.clone() };
        if flyout_row(ui, pal, Some(current_sub == Some(i)), None, &label).clicked() && !s.bitmap {
            app.set_subtitle(Some(i));
        }
    }
}

fn more_flyout(app: &mut PlayerApp, ui: &mut Ui, pal: &Palette) {
    heading(ui, pal, "Playback speed");
    let rate = app.engine.rate();
    let labels: Vec<String> = SPEEDS.iter().map(|s| if *s == 1.0 { "Normal".into() } else { format!("{s}×") }).collect();
    let on = SPEEDS.iter().position(|s| (s - rate).abs() < 1e-6).unwrap_or(3);
    if let Some(i) = chips(ui, pal, &labels, on) {
        app.engine.set_rate(SPEEDS[i]);
    }
    if app.info.as_ref().is_some_and(|i| i.has_video) {
        heading(ui, pal, "Zoom");
        let labels: Vec<String> = Zoom::ALL.iter().map(|z| z.label().to_string()).collect();
        let on = Zoom::ALL.iter().position(|z| *z == app.zoom).unwrap_or(0);
        if let Some(i) = chips(ui, pal, &labels, on) {
            app.zoom = Zoom::ALL[i];
        }
    }
    ui.add_space(6.0);
    ui.separator();
    let shuffle = app.playlist.shuffle;
    if flyout_row(ui, pal, Some(shuffle), None, "Shuffle").clicked() {
        app.set_shuffle(!shuffle);
    }
    let repeat = match app.playlist.repeat {
        Repeat::Off => "Repeat",
        Repeat::All => "Repeat all",
        Repeat::One => "Repeat one",
    };
    if flyout_row(ui, pal, Some(app.playlist.repeat != Repeat::Off), None, repeat).clicked() {
        app.cycle_repeat();
    }
    ui.separator();
    if flyout_row(ui, pal, None, Some(icon::INFO), "Media info").clicked() {
        app.info_open = true;
        app.flyout = None;
    }
    if flyout_row(ui, pal, None, Some(icon::FOLDER), "Open file location").clicked() {
        app.open_folder();
        app.flyout = None;
    }
    if flyout_row(ui, pal, None, Some(icon::EDIT), "Open in Editor").clicked() {
        app.flyout = None;
        app.open_editor();
    }
}

fn info_window(app: &mut PlayerApp, ctx: &egui::Context, pal: &Palette) {
    let mut open = true;
    let path = app.path().to_path_buf();
    let size_on_disk = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    egui::Window::new("Media info")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .frame(flyout_frame(pal, ctx).inner_margin(egui::Margin::same(14)))
        .show(ctx, |ui| {
            egui::Grid::new("player-info").num_columns(2).spacing(vec2(18.0, 6.0)).show(ui, |ui| {
                let mut row = |k: &str, v: String| {
                    ui.label(egui::RichText::new(k).color(pal.text_dim));
                    ui.label(egui::RichText::new(v).color(pal.text));
                    ui.end_row();
                };
                row("File", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                row("Folder", path.parent().map(|p| p.display().to_string()).unwrap_or_default());
                row("Size", format!("{:.1} MB", size_on_disk as f64 / 1_048_576.0));
                if let Some(i) = &app.info {
                    row("Length", crate::ui::timecode(i.duration, i.fps.max(1.0)));
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

/// Audio files: the cover (or a note on an accent tile), the title, and a bar visualizer.
fn audio_view(app: &mut PlayerApp, ui: &mut Ui, pal: &Palette, full: Rect, mini: bool) {
    let p = ui.painter().clone();
    vgrad(&p, full, pal.bg, lerp(pal.bg, Color32::BLACK, 0.6));
    let side = (full.height() * if mini { 0.55 } else { 0.42 }).min(full.width() * 0.5).max(48.0);
    let art = Rect::from_center_size(pos2(full.center().x, full.top() + full.height() * 0.38), vec2(side, side));
    match &app.cover {
        Some(c) => {
            let s = c.size_vec2();
            let k = (side / s.x).min(side / s.y);
            p.image(c.id(), Rect::from_center_size(art.center(), s * k), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        None => {
            vgrad(&p, art, pal.accent, lerp(pal.accent, Color32::BLACK, 0.45));
            glyph(&p, art.center(), icon::MUSIC, side * 0.4, on_accent(pal.accent));
        }
    }
    if mini {
        return;
    }
    p.text(pos2(full.center().x, art.bottom() + 28.0), Align2::CENTER_CENTER, app.title(), FontId::proportional(22.0), pal.text);
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
        p.rect_filled(r, 1.5, pal.accent.gamma_multiply(0.85));
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
    let lift = if chrome_up { BOTTOM_H + 8.0 } else { video.height() * 0.06 };
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
        let fit = Zoom::Fit.place(area, (1920, 800), 1.0).size();
        assert!((fit - vec2(1600.0, 1600.0 * 800.0 / 1920.0)).length() < 1e-3, "{fit:?}");
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
