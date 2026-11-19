//! The player's look: the media-player layout of 1.3.0 (the picture edge to edge, a thin title bar,
//! a full-width thin seek bar with elapsed / total at its ends, then one row: the file's name on the
//! left, small line icons in the middle - shuffle, previous, back 10 s, the ringed play button,
//! forward 10 s, next, loop, reverse - and speed, subtitles / audio, volume, full screen, mini player
//! and ⋯ on the right), coloured from Simple Editor's theme (Settings ▸ Appearance's theme, palette,
//! Look and accent, read by `PlayerApp::new`): the accent fills the progress, the knob, the play ring
//! and every toggle that is on; text and icons are the theme's text / dim colours; the bars fade into
//! its background. Hit areas are `HIT` bigger than the icons on every side, invisibly.
//!
//! Icons are Windows' own icon font (Segoe Fluent Icons / MDL2 Assets).

use super::{Flyout, PlayerApp, Zoom, SKIP_BACK, SKIP_FORWARD};
use crate::player::folder::Repeat;
use crate::theme::Palette;
use crate::ui::autohide;
use crate::ui::tools::on_accent;
use eframe::egui::{self, pos2, vec2, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Response, Sense, Stroke, Ui};
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
    /// SkipForward30, drawn with BACK10's "10" (`glyph`): the fonts have no forward-10 glyph.
    pub const FORWARD30: char = '\u{ED3D}';
    /// Not a font glyph: `glyph` draws FORWARD30's arrow over BACK10's "10".
    pub const FORWARD10: char = '\u{10FFFC}';
    pub const REPEAT_ALL: char = '\u{E8EE}';
    pub const LOOP: char = '\u{E8ED}'; // RepeatOne: this file over and over
    /// Not a font glyph: `glyph` draws it as PLAY mirrored (a play triangle pointing back).
    pub const REVERSE: char = '\u{10FFFD}';
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
    pub const CONVERT: char = '\u{E8AB}'; // Switch
    pub const UPSCALE: char = '\u{E740}'; // FullScreen arrows: bigger
    pub const TAG: char = '\u{E8EC}'; // Tag
    pub const ALL: [char; 32] = [
        EDIT, SHUFFLE, PREVIOUS, NEXT, PLAY, PAUSE, BACK10, FORWARD30, REPEAT_ALL, LOOP, CAPTIONS, VOLUME[0], VOLUME[1],
        VOLUME[2], VOLUME[3], MUTE, FULLSCREEN, EXIT_FULLSCREEN, MINI, UNMINI, MORE, MINIMIZE, MAXIMIZE, RESTORE, CLOSE, MUSIC,
        FOLDER, INFO, CHECK, CONVERT, UPSCALE, TAG,
    ];
}

const TOP_H: f32 = 40.0;
pub const BOTTOM_H: f32 = 116.0;
/// Every button's hit area reaches this far past its drawn box.
const HIT: f32 = 4.0;

/// "0:00:41": elapsed / total beside the seek bar.
pub fn clock(t: f64) -> String {
    let s = t.max(0.0).floor() as u64;
    format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

fn icon_font(px: f32) -> FontId {
    FontId::new(px, crate::theme::icons())
}

fn glyph(p: &egui::Painter, at: Pos2, c: char, px: f32, col: Color32) {
    if c == icon::FORWARD10 {
        // both glyphs put the arrow above ~0.44 of the size and the digits below (the ink has a gap
        // there), so the forward arrow and the back glyph's "10" join cleanly
        let g = p.layout_no_wrap(icon::FORWARD30.to_string(), icon_font(px), col);
        let split = at.y - g.size().y * 0.5 + px * 0.435;
        let r = p.clip_rect();
        p.with_clip_rect(Rect::from_min_max(r.min, pos2(r.max.x, split).min(r.max))).text(at, Align2::CENTER_CENTER, icon::FORWARD30, icon_font(px), col);
        p.with_clip_rect(Rect::from_min_max(pos2(r.min.x, split).max(r.min), r.max)).text(at, Align2::CENTER_CENTER, icon::BACK10, icon_font(px), col);
        return;
    }
    if c == icon::REVERSE {
        // PLAY turned half round about its centre: the (symmetric) triangle mirrored, pointing back
        let g = p.layout_no_wrap(icon::PLAY.to_string(), icon_font(px), col);
        let pos = at + g.size() * 0.5;
        p.add(egui::epaint::TextShape::new(pos, g, col).with_angle(std::f32::consts::PI));
        return;
    }
    p.text(at, Align2::CENTER_CENTER, c, icon_font(px), col);
}

fn radius(ui: &Ui) -> CornerRadius {
    ui.visuals().widgets.inactive.corner_radius
}

/// A flat icon button: a soft highlight under the pointer, the glyph in the theme's text colour (the
/// accent when `on`). The hit area is `rect` grown by `HIT`.
fn icon_button(ui: &mut Ui, pal: &Palette, rect: Rect, c: char, px: f32, on: bool, tip: &str) -> Response {
    let id = ui.id().with(("btn", c, rect.min.x as i32, rect.min.y as i32));
    let r = ui.interact(rect.expand(HIT), id, Sense::click()).on_hover_text(tip);
    let p = ui.painter();
    if r.is_pointer_button_down_on() {
        p.rect_filled(rect.shrink(2.0), 6.0, pal.text.gamma_multiply(0.06));
    } else if r.hovered() {
        p.rect_filled(rect.shrink(2.0), 6.0, pal.text.gamma_multiply(0.1));
    }
    glyph(p, rect.center(), c, px, if on { pal.accent } else { pal.text });
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

/// The play button: a disc in the theme's panel colour with a ring in the accent (shading to a lighter
/// accent and back round it).
fn play_button(ui: &mut Ui, pal: &Palette, center: Pos2, radius: f32, playing: bool) -> Response {
    let rect = Rect::from_center_size(center, vec2(radius * 2.0, radius * 2.0));
    let tip = if playing { "Pause (Space)" } else { "Play (Space)" };
    let id = ui.id().with(("play", center.x as i32, center.y as i32));
    let r = ui.interact(rect.expand(HIT), id, Sense::click()).on_hover_text(tip);
    let p = ui.painter();
    let fill = if r.hovered() { lerp(pal.panel, pal.text, 0.12) } else { pal.panel };
    p.circle_filled(center, radius, fill);
    let light = lerp(pal.accent, Color32::WHITE, 0.45);
    let n = 48;
    for i in 0..n {
        let (a0, a1) = (i as f32 / n as f32, (i + 1) as f32 / n as f32);
        let ang = |a: f32| -std::f32::consts::FRAC_PI_2 + a * std::f32::consts::TAU;
        let at = |a: f32| center + vec2(ang(a).cos(), ang(a).sin()) * (radius - 1.5);
        let k = ((a0 * std::f32::consts::TAU).sin() * 0.5 + 0.5).clamp(0.0, 1.0);
        p.line_segment([at(a0), at(a1)], Stroke::new(2.5, lerp(pal.accent, light, k)));
    }
    let c = if playing { icon::PAUSE } else { icon::PLAY };
    glyph(p, center + vec2(if playing { 0.0 } else { 1.5 }, 0.0), c, radius * 0.8, pal.text);
    r
}

/// The seek bar's tick steps for `dur` seconds over `px` pixels: (major, minor) in seconds. Majors are
/// whole "nice" steps (1 s, 2 s, 5 s ... 1 h) at least 8 px apart; minors divide a major evenly and are
/// at least 4 px apart (0 = none). So ticks never smear into a solid bar.
pub fn tick_steps(dur: f64, px: f32) -> (f64, f64) {
    const NICE: [f64; 18] = [0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 900.0, 1800.0, 3600.0, 7200.0, 18000.0];
    if dur <= 0.0 || px <= 0.0 {
        return (0.0, 0.0);
    }
    let pps = px as f64 / dur;
    let major = NICE.iter().copied().find(|&m| m >= 1.0 && m * pps >= 8.0).unwrap_or(36000.0);
    let even = |m: f64| ((major / m) - (major / m).round()).abs() < 1e-6;
    let minor = NICE.iter().copied().filter(|&m| m < major && m * pps >= 4.0 && even(m)).next().unwrap_or(0.0);
    (major, minor)
}

/// Ticks across the line: a longer one per major step, a short one per minor, just above and below.
fn ticks(p: &egui::Painter, pal: &Palette, x0: f32, x1: f32, y: f32, dur: f64) {
    let (major, minor) = tick_steps(dur, x1 - x0);
    let step = if minor > 0.0 { minor } else { major };
    if step <= 0.0 {
        return;
    }
    let (maj_c, min_c) = (pal.text_dim.gamma_multiply(0.7), pal.text_dim.gamma_multiply(0.35));
    for k in 0..=(dur / step + 1e-6) as u64 {
        let t = k as f64 * step;
        let is_major = ((t / major) - (t / major).round()).abs() < 1e-6;
        let x = (x0 + (x1 - x0) * (t / dur) as f32).round() + 0.5;
        let (half, c) = if is_major { (5.5, maj_c) } else { (3.5, min_c) };
        p.line_segment([pos2(x, y - half), pos2(x, y + half)], Stroke::new(1.0, c));
    }
}

/// The seek bar: a thin track in the theme's dim text colour with the played part in the accent, ticks
/// across it, a knob that grows under the pointer. All of `rect` grabs. Hovering shows the picture at that
/// moment (the strip's nearest thumbnail) with the time. Returns the time under the pointer while
/// dragged / clicked.
fn scrubber(app: &mut PlayerApp, ui: &mut Ui, pal: &Palette, rect: Rect, t: f64, dur: f64) -> (Response, Option<f64>) {
    let r = ui.interact(rect, ui.id().with("scrub"), Sense::click_and_drag());
    let p = ui.painter().clone();
    let (x0, x1) = (rect.left() + 8.0, rect.right() - 8.0);
    let y = rect.center().y;
    let k = if dur > 0.0 { (t / dur).clamp(0.0, 1.0) as f32 } else { 0.0 };
    let x = x0 + (x1 - x0) * k;
    let hot = r.hovered() || r.dragged();
    let th = if hot { 5.0 } else { 3.0 };
    ticks(&p, pal, x0, x1, y, dur);
    let line = |a: f32, b: f32, c| p.rect_filled(Rect::from_min_max(pos2(a, y - th / 2.0), pos2(b, y + th / 2.0)), th / 2.0, c);
    line(x0, x1, pal.text_dim.gamma_multiply(0.45));
    line(x0, x, pal.accent);
    let rad = if hot { 8.0 } else { 6.0 };
    p.circle_filled(pos2(x, y), rad, pal.accent);
    p.circle_stroke(pos2(x, y), rad, Stroke::new(1.5, pal.text));
    let at = |px: f32| (((px - x0) / (x1 - x0)).clamp(0.0, 1.0) as f64) * dur;
    let mut seek = None;
    if let Some(pos) = r.interact_pointer_pos().filter(|_| r.dragged() || r.clicked() || r.drag_started()) {
        seek = Some(at(pos.x));
    }
    let shot = app.shot_hover.map(|k| pos2(x0 + (x1 - x0) * k, y));
    if let Some(h) = r.hover_pos().or_else(|| r.interact_pointer_pos().filter(|_| r.dragged())).or(shot) {
        let ht = at(h.x);
        let g = p.layout_no_wrap(clock(ht), FontId::proportional(12.0), pal.text);
        let ctx = ui.ctx().clone();
        let pic = app.strip.as_mut().and_then(|s| {
            if !s.done() {
                ctx.request_repaint_after(std::time::Duration::from_millis(100)); // still filling
            }
            s.texture(&ctx, ht)
        });
        let screen = ctx.screen_rect();
        let place = |size: egui::Vec2| {
            let left = (h.x - size.x / 2.0).clamp(screen.left() + 4.0, (screen.right() - 4.0 - size.x).max(screen.left() + 4.0));
            Rect::from_min_size(pos2(left, rect.top() - 2.0 - size.y), size)
        };
        if let Some((id, px)) = pic {
            let img = vec2(176.0, 176.0 * px.y / px.x.max(1.0));
            let card = place(img + vec2(8.0, 8.0 + g.size().y + 6.0));
            p.rect(card, radius(ui), pal.panel, Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
            let img_r = Rect::from_min_size(card.min + vec2(4.0, 4.0), img);
            p.image(id, img_r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            p.galley(pos2(card.center().x - g.size().x / 2.0, img_r.bottom() + 4.0), g, pal.text);
        } else if app.strip.as_ref().is_some_and(|s| !s.done()) {
            // the thumbnail is not made yet: its card, loading
            let aspect = app.info.as_ref().filter(|i| i.size.0 > 0).map_or(9.0 / 16.0, |i| i.size.1 as f32 / i.size.0 as f32);
            let img = vec2(176.0, 176.0 * aspect);
            let card = place(img + vec2(8.0, 8.0 + g.size().y + 6.0));
            p.rect(card, radius(ui), pal.panel, Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
            let img_r = Rect::from_min_size(card.min + vec2(4.0, 4.0), img);
            shimmer(ui, img_r, pal);
            p.galley(pos2(card.center().x - g.size().x / 2.0, img_r.bottom() + 4.0), g, pal.text);
        } else {
            let box_ = place(g.size() + vec2(12.0, 6.0));
            p.rect(box_, radius(ui), pal.panel, Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
            p.galley(box_.center() - g.size() / 2.0, g, pal.text);
        }
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

    // ---- the picture (or the audio view; Audio Only: the visualizer, nothing decoded)
    let audio_only = app.engine.audio_only();
    let video_rect = match (&app.info, app.video.shown) {
        (Some(info), Some((tex, _, uv))) if info.has_video && !audio_only => {
            let r = app.zoom.place(full, info.size, ppp);
            ui.painter().with_clip_rect(full).image(tex, r, uv, Color32::WHITE);
            Some(r)
        }
        _ => None,
    };
    match app.info.as_ref().map(|i| (i.has_video, i.size)) {
        Some(_) if audio_only => visualizer_view(app, ui, full),
        Some((true, size)) => {
            // the video processor scales to exactly the pixels the picture covers
            let target = app.zoom.place(full, size, ppp);
            let px = |v: f32| ((v * ppp).round() as u32).clamp(16, 7680) & !1;
            // a picked resolution caps it: the decoder (or GPU video processor) scales down to it, GL
            // stretches that over the picture
            let cap = super::capped(size, app.res);
            if app.zoom == Zoom::Original {
                app.engine.set_output_size(cap.0, cap.1);
            } else {
                let (w, h) = (px(target.width()), px(target.height()));
                let (w, h) = if w > cap.0 || h > cap.1 { super::capped((w, h), Some(cap.1.min(h))) } else { (w, h) };
                app.engine.set_output_size(w.min(cap.0), h);
            }
        }
        // songs: the cover (loading: a placeholder), else - no cover, or Visualizer picked - the visualizer
        Some((false, _)) if app.song_shows_visualizer() => visualizer_view(app, ui, full),
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
    // right-click: the menu (the editor's rows)
    crate::ui::menu::context(&surface, |ui| super::panels::context_menu(app, ui, &ctx));
    if let Some(at) = app.shot_menu {
        // the same popup a right-click opens, held open for the picture
        egui::Popup::new(egui::Id::new("player-shot-menu"), ctx.clone(), egui::PopupAnchor::Position(at), ui.layer_id())
            .kind(egui::PopupKind::Menu)
            .layout(egui::Layout::top_down_justified(egui::Align::Min))
            .open(true)
            .show(|ui| {
                egui::containers::menu::menu_style(ui.style_mut());
                super::panels::context_menu(app, ui, &ctx)
            });
    }
    let menu_open = egui::Popup::is_any_open(&ctx) || app.shot_menu.is_some();
    if menu_open && !app.symbols {
        app.symbols = true; // a submenu's arrow: Segoe UI Symbol, read only now
        crate::theme::add_symbols(&ctx);
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
        // (`SE_PLAYER_SHOT` pictures the controls: they hold for it)
        hold: app.flyout.is_some()
            || app.scrub.is_some()
            || app.info_open
            || app.shot.is_some()
            || app.renaming.is_some()
            || app.convert.is_some()
            || app.upscale.is_some()
            || app.meta.is_some()
            || menu_open,
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
    convert_window(app, &ctx, pal);
    super::panels::upscale_window(app, &ctx, pal);
    super::panels::metadata_window(app, &ctx, pal);
    toast(app, &ctx, pal, full, out.opacity > 0.0 && !mini);
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

fn top_bar(app: &mut PlayerApp, ui: &mut Ui, pal: &Palette, top: Rect, maximized: bool) {
    let ctx = ui.ctx().clone();
    let shade = Rect::from_min_size(top.min, vec2(top.width(), TOP_H + 32.0));
    vgrad(ui.painter(), shade, pal.bg.gamma_multiply(0.6), Color32::TRANSPARENT);
    // the empty bar drags the window, a double-click maximizes
    let bar = ui.interact(top, ui.id().with("titlebar"), Sense::click_and_drag());
    if bar.drag_started() && !app.fullscreen {
        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    if bar.double_clicked() && !app.fullscreen {
        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
    }
    let b = Rect::from_min_size(top.min + vec2(4.0, 4.0), vec2(40.0, 32.0));
    if icon_button(ui, pal, b, icon::EDIT, 15.0, false, "Open in Editor (Esc)").clicked() {
        app.open_editor();
    }
    let icon_at = pos2(b.right() + 10.0, top.center().y);
    if app.icon.is_none() {
        app.icon = app_icon(&ctx);
    }
    if let Some(i) = &app.icon {
        let r = Rect::from_min_size(icon_at - vec2(0.0, 8.0), vec2(16.0, 16.0));
        ui.painter().image(i.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }
    ui.painter().text(icon_at + vec2(26.0, 0.0), Align2::LEFT_CENTER, app.title(), FontId::proportional(12.0), pal.text);
    if app.fullscreen {
        return;
    }
    let w = 46.0;
    let close = Rect::from_min_size(pos2(top.right() - w, top.top()), vec2(w, TOP_H - 8.0));
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
    let p = ui.painter().clone();
    let shade = Rect::from_min_max(pos2(bottom.left(), bottom.top() - 60.0), bottom.max);
    vgrad(&p, shade, Color32::TRANSPARENT, pal.bg.gamma_multiply(0.85));
    // the bar itself swallows clicks (no play/pause through it)
    ui.interact(bottom, ui.id().with("bottom"), Sense::click());
    let dur = app.engine.duration();
    let t = app.scrub.map_or_else(|| app.engine.time(), |(t, _)| t);
    // seek row: elapsed | bar | total
    let row_y = bottom.top() + 24.0;
    let font = FontId::proportional(12.0);
    let left = p.layout_no_wrap(clock(t), font.clone(), pal.text);
    let right = p.layout_no_wrap(clock(dur), font, pal.text);
    p.galley(pos2(bottom.left() + 14.0, row_y - left.size().y / 2.0), left.clone(), pal.text);
    p.galley(pos2(bottom.right() - 14.0 - right.size().x, row_y - right.size().y / 2.0), right.clone(), pal.text);
    // drawn thin, grabs 28 pt tall
    let bar = Rect::from_min_max(
        pos2(bottom.left() + 22.0 + left.size().x, row_y - 14.0),
        pos2(bottom.right() - 22.0 - right.size().x, row_y + 14.0),
    );
    let (r, at) = scrubber(app, ui, pal, bar, t, dur);
    scrub(app, &r, at);

    // controls row
    let cy = bottom.top() + 74.0;
    name_label(app, ui, pal, pos2(bottom.left() + 16.0, cy), (bottom.width() * 0.28).max(80.0));
    let cx = bottom.center().x;
    let step = 48.0;
    let sq = |x: f32| Rect::from_center_size(pos2(x, cy), vec2(40.0, 40.0));
    let playing = app.engine.is_playing();
    let shuffle = app.playlist.shuffle;
    let stip = if shuffle { "Shuffle: on" } else { "Shuffle: off" };
    if icon_button(ui, pal, sq(cx - step * 3.4), icon::SHUFFLE, 16.0, shuffle, stip).clicked() {
        app.set_shuffle(!shuffle);
    }
    if icon_button(ui, pal, sq(cx - step * 2.4), icon::PREVIOUS, 16.0, false, "Previous (Ctrl+Left)").clicked() {
        app.prev(&ctx);
    }
    if icon_button(ui, pal, sq(cx - step * 1.35), icon::BACK10, 20.0, false, "Skip back 10 seconds (Left)").clicked() {
        app.skip(-SKIP_BACK);
    }
    if play_button(ui, pal, pos2(cx, cy), 22.0, playing).clicked() {
        app.toggle_play();
    }
    if icon_button(ui, pal, sq(cx + step * 1.35), icon::FORWARD10, 20.0, false, "Skip forward 10 seconds (Right)").clicked() {
        app.skip(SKIP_FORWARD);
    }
    if icon_button(ui, pal, sq(cx + step * 2.4), icon::NEXT, 16.0, false, "Next (Ctrl+Right)").clicked() {
        app.next(&ctx);
    }
    let looping = app.looping();
    let ltip = if looping { "Loop: on" } else { "Loop: off" };
    if icon_button(ui, pal, sq(cx + step * 3.4), icon::LOOP, 16.0, looping, ltip).clicked() {
        app.set_loop(!looping);
    }
    let rev = app.engine.is_reverse();
    if icon_button(ui, pal, sq(cx + step * 4.4), icon::REVERSE, 16.0, rev, "Reverse").clicked() {
        app.toggle_reverse();
    }

    // right side: speed, subtitles / audio, volume, full screen, mini player, more
    let rsq = |i: f32| Rect::from_center_size(pos2(bottom.right() - 30.0 - i * 44.0, cy), vec2(40.0, 40.0));
    let more = rsq(0.0);
    if icon_button(ui, pal, more, icon::MORE, 16.0, app.flyout == Some(Flyout::More), "More options").clicked() {
        toggle(app, Flyout::More);
    }
    if icon_button(ui, pal, rsq(1.0), icon::MINI, 16.0, false, "Mini player").clicked() {
        app.toggle_mini(&ctx);
    }
    let (fc, ftip) = if app.fullscreen { (icon::EXIT_FULLSCREEN, "Exit full screen (F)") } else { (icon::FULLSCREEN, "Full screen (F)") };
    if icon_button(ui, pal, rsq(2.0), fc, 16.0, false, ftip).clicked() {
        app.toggle_fullscreen(&ctx);
    }
    let von = app.flyout == Some(Flyout::Volume);
    if icon_button(ui, pal, rsq(3.0), volume_icon(app), 16.0, von, "Volume (M mutes)").clicked() {
        toggle(app, Flyout::Volume);
    }
    let ton = app.flyout == Some(Flyout::Tracks);
    if icon_button(ui, pal, rsq(4.0), icon::CAPTIONS, 16.0, ton, "Subtitles and audio").clicked() {
        toggle(app, Flyout::Tracks);
        app.tracks();
    }
    let speed = Rect::from_center_size(pos2(rsq(4.0).left() - 24.0, cy), vec2(44.0, 28.0));
    if speed_button(ui, pal, speed, app.engine.rate(), app.flyout == Some(Flyout::Speed)).clicked() {
        toggle(app, Flyout::Speed);
    }
    ANCHORS.with(|a| a.set([speed, rsq(4.0), rsq(3.0), more]));
}

/// The file's name, bottom left: click it to rename the file (Enter renames, Esc cancels).
fn name_label(app: &mut PlayerApp, ui: &mut Ui, pal: &Palette, at: Pos2, width: f32) {
    let ctx = ui.ctx().clone();
    if let Some(mut text) = app.renaming.take() {
        let rect = Rect::from_min_size(pos2(at.x - 4.0, at.y - 14.0), vec2(width, 28.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        let edit = egui::TextEdit::singleline(&mut text).font(FontId::proportional(14.0)).desired_width(width).text_color(pal.text);
        let r = child.add(edit);
        let id = r.id;
        if !r.has_focus() && !r.lost_focus() {
            ctx.memory_mut(|m| m.request_focus(id));
        }
        let (enter, esc) = ctx.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
        if esc {
            ctx.memory_mut(|m| m.surrender_focus(id));
        } else if enter || (r.lost_focus() && !esc) {
            ctx.memory_mut(|m| m.surrender_focus(id));
            app.rename(&ctx, &text);
        } else {
            app.renaming = Some(text);
        }
        return;
    }
    let p = ui.painter();
    let g = p.layout(app.title(), FontId::proportional(14.0), pal.text, width);
    let rect = Rect::from_min_size(pos2(at.x, at.y - g.size().y / 2.0), g.size());
    let r = ui.interact(rect.expand(HIT), ui.id().with("name"), Sense::click()).on_hover_text("Rename");
    if r.hovered() {
        p.rect_filled(rect.expand(3.0), 4.0, pal.text.gamma_multiply(0.08));
        ctx.set_cursor_icon(egui::CursorIcon::Text);
    }
    p.galley(rect.min, g, pal.text);
    if r.clicked() {
        app.renaming = Some(app.title());
    }
}

/// Audio Only: the editor's visualizer (the style picked in ⋯ ▸ Visualizer, shared with the editor),
/// driven by the sound being played (the ticked tracks, mixed).
fn visualizer_view(app: &mut PlayerApp, ui: &mut Ui, full: Rect) {
    if app.viz.1.is_empty() {
        // (reads the library folder: only when the style changes)
        app.viz.1 = crate::engine::visualizer::last_used(&app.viz.0).src;
    }
    let tap = app.engine.tap();
    let n = crate::engine::visualizer::BANDS;
    let b = bands(&tap, n);
    let peak = tap.iter().fold(0f32, |m, v| m.max(v.abs())).min(1.0);
    let mut f = [0f32; 1 + crate::engine::visualizer::BANDS];
    f[0] = peak;
    f[1..].copy_from_slice(&b);
    // the look picked in ⋯ ▸ Visualizer: how far it moves, and the style's own params
    app.orb.show_frame(app.vis_look.shape(f), app.engine.time());
    let style = crate::engine::visualizer::last_used(&app.viz.0).name;
    app.orb.paint_with(ui.painter(), full, &app.viz.1, app.vis_look.params.get(&style));
    if app.engine.is_playing() {
        ui.ctx().request_repaint();
    }
}

/// The message at the bottom (rename, convert); a converted file gets Open / Open Folder / Add to
/// Project. A running conversion shows its progress and Cancel here too.
fn toast(app: &mut PlayerApp, ctx: &egui::Context, pal: &Palette, full: Rect, chrome_up: bool) {
    let running = app
        .convert_job
        .as_ref()
        .map(|j| ("Converting", j.progress.clone(), j.progress.fraction()))
        .or_else(|| app.upscale_job.as_ref().map(|j| ("Upscaling", j.progress.clone(), j.progress.fraction())));
    if app.toast.is_none() && running.is_none() {
        return;
    }
    let bottom = full.bottom() - if chrome_up { BOTTOM_H + 8.0 } else { 16.0 };
    egui::Area::new(egui::Id::new("player-toast"))
        .order(egui::Order::Foreground)
        .pivot(Align2::CENTER_BOTTOM)
        .fixed_pos(pos2(full.center().x, bottom))
        .show(ctx, |ui| {
            flyout_frame(pal, ctx).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
                if let Some((what, prog, k)) = &running {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(*what).color(pal.text));
                        ui.add(egui::ProgressBar::new(*k).desired_width(180.0).fill(pal.accent));
                        if ui.button("Cancel").clicked() {
                            prog.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                    });
                }
                let Some((text, _, about)) = app.toast.clone() else { return };
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&text).color(pal.text));
                    let Some(out) = about else { return };
                    let shown = if crate::engine::convert::is_sequence(&out) { out.parent().map(Path::to_path_buf).unwrap_or(out.clone()) } else { out.clone() };
                    if ui.button("Open").clicked() {
                        let _ = std::process::Command::new("explorer.exe").arg(&shown).spawn();
                    }
                    if ui.button("Open Folder").clicked() {
                        let _ = std::process::Command::new("explorer.exe").arg(format!("/select,{}", shown.display())).spawn();
                    }
                    if !crate::engine::convert::is_sequence(&out) && ui.button("Add to Project").clicked() {
                        app.toast = None;
                        app.handoff_file(out);
                    }
                });
            });
        });
}

/// Convert…: format, quality or size, resolution, frame rate and audio tracks; Convert starts it in the
/// background (progress and Cancel in the message at the bottom).
fn convert_window(app: &mut PlayerApp, ctx: &egui::Context, pal: &Palette) {
    use super::convert::FORMATS;
    let Some(mut spec) = app.convert.take() else { return };
    let (mut open, mut go) = (true, false);
    let has_video = app.info.as_ref().is_some_and(|i| i.has_video);
    egui::Window::new("Convert")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .frame(flyout_frame(pal, ctx).inner_margin(egui::Margin::same(14)))
        .show(ctx, |ui| {
            ui.set_width(340.0);
            heading(ui, pal, "Format");
            let labels: Vec<String> = FORMATS.iter().map(|f| f.0.to_string()).collect();
            if let Some(i) = chips(ui, pal, &labels, spec.format) {
                spec.format = i;
            }
            let ext = FORMATS[spec.format].1;
            let audio_out = matches!(ext, "mp3" | "wav" | "m4a");
            if !audio_out && ext != "png" && ext != "gif" {
                heading(ui, pal, "Quality");
                let q: Vec<String> = ["Smallest", "Small", "Good", "High", "Best", "Size"].map(String::from).to_vec();
                let on = if spec.size_mb.is_some() { 5 } else { spec.quality as usize };
                if let Some(i) = chips(ui, pal, &q, on) {
                    if i == 5 {
                        spec.size_mb.get_or_insert(25.0);
                    } else {
                        spec.size_mb = None;
                        spec.quality = i as u8;
                    }
                }
                if let Some(mb) = spec.size_mb.as_mut() {
                    ui.add(egui::Slider::new(mb, 1.0..=2000.0).logarithmic(true).suffix(" MB"));
                }
            }
            if has_video && !audio_out {
                heading(ui, pal, "Resolution");
                let hs = [None, Some(2160), Some(1440), Some(1080), Some(720), Some(480), Some(360)];
                let labels: Vec<String> = hs.iter().map(|h| h.map_or("Original".into(), |h| format!("{h}p"))).collect();
                let on = hs.iter().position(|h| *h == spec.height).unwrap_or(0);
                if let Some(i) = chips(ui, pal, &labels, on) {
                    spec.height = hs[i];
                }
                heading(ui, pal, "Frame rate");
                let fs = [None, Some(60.0), Some(30.0), Some(24.0), Some(15.0), Some(10.0)];
                let labels: Vec<String> = fs.iter().map(|f| f.map_or("Original".into(), |f| format!("{f}"))).collect();
                let on = fs.iter().position(|f| *f == spec.fps).unwrap_or(0);
                if let Some(i) = chips(ui, pal, &labels, on) {
                    spec.fps = fs[i];
                }
            }
            if ext != "gif" && ext != "png" && app.info.as_ref().is_some_and(|i| i.audio_streams.len() > 1) {
                heading(ui, pal, "Audio tracks");
                let labels = vec!["All".to_string(), "The ticked ones".to_string()];
                if let Some(i) = chips(ui, pal, &labels, spec.ticked_only as usize) {
                    spec.ticked_only = i == 1;
                }
            }
            ui.add_space(8.0);
            let busy = app.convert_job.is_some();
            ui.horizontal(|ui| {
                let b = egui::Button::new(egui::RichText::new("Convert").color(on_accent(pal.accent))).fill(pal.accent);
                if ui.add_enabled(!busy, b).on_disabled_hover_text("A conversion is running").clicked() {
                    go = true;
                }
                ui.label(egui::RichText::new("Saved next to the file").size(12.0).color(pal.text_dim));
            });
        });
    if go {
        app.start_convert(&spec);
    } else if open {
        app.convert = Some(spec);
    }
}

/// Volume: a mute button and a slider (the seek bar's look).
fn volume_flyout(app: &mut PlayerApp, ui: &mut Ui, pal: &Palette) {
    ui.horizontal(|ui| {
        let (rect, r) = ui.allocate_exact_size(vec2(36.0, 32.0), Sense::click());
        if r.hovered() {
            ui.painter().rect_filled(rect, 4.0, pal.text.gamma_multiply(0.08));
        }
        glyph(ui.painter(), rect.center(), volume_icon(app), 16.0, pal.text);
        if r.on_hover_text(if app.engine.muted() { "Unmute (M)" } else { "Mute (M)" }).clicked() {
            app.toggle_mute();
        }
        let (bar, r) = ui.allocate_exact_size(vec2(196.0, 32.0), Sense::click_and_drag());
        let v = if app.engine.muted() { 0.0 } else { app.engine.volume() };
        let y = bar.center().y;
        let (x0, x1) = (bar.left() + 8.0, bar.right() - 8.0);
        let x = x0 + (x1 - x0) * v;
        let p = ui.painter();
        p.rect_filled(Rect::from_min_max(pos2(x0, y - 2.0), pos2(x1, y + 2.0)), 2.0, pal.text_dim.gamma_multiply(0.45));
        p.rect_filled(Rect::from_min_max(pos2(x0, y - 2.0), pos2(x, y + 2.0)), 2.0, pal.accent);
        p.circle_filled(pos2(x, y), 7.0, pal.accent);
        p.circle_stroke(pos2(x, y), 7.0, Stroke::new(1.5, pal.text));
        if let Some(pos) = r.interact_pointer_pos().filter(|_| r.dragged() || r.clicked()) {
            app.set_volume(((pos.x - x0) / (x1 - x0)).clamp(0.0, 1.0));
        }
        ui.label(egui::RichText::new(format!("{}", (v * 100.0).round() as u32)).size(13.0).color(pal.text));
    });
}

thread_local! {
    /// Where the flyout buttons were drawn this frame: speed, tracks, volume, more.
    static ANCHORS: std::cell::Cell<[Rect; 4]> = const { std::cell::Cell::new([Rect::NOTHING; 4]) };
}

/// "1×", "0.25×" ...
pub fn speed_label(rate: f64) -> String {
    format!("{}×", (rate * 100.0).round() / 100.0)
}

/// The speed button: the current rate as text, in the accent when it is not 1×.
fn speed_button(ui: &mut Ui, pal: &Palette, rect: Rect, rate: f64, open: bool) -> Response {
    let r = ui.interact(rect.expand(HIT), ui.id().with("speed"), Sense::click()).on_hover_text("Playback speed");
    let p = ui.painter();
    let down = r.is_pointer_button_down_on();
    if down {
        p.rect_filled(rect.shrink(2.0), 6.0, pal.accent);
    } else if r.hovered() || open {
        p.rect_filled(rect.shrink(2.0), 6.0, pal.text.gamma_multiply(0.1));
    }
    let fg = if down {
        on_accent(pal.accent)
    } else if (rate - 1.0).abs() > 1e-6 {
        pal.accent
    } else {
        pal.text
    };
    p.text(rect.center(), Align2::CENTER_CENTER, speed_label(rate), FontId::proportional(13.0), fg);
    r
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
    p.rect_filled(full, 0.0, pal.bg.gamma_multiply(0.35));
    let close = Rect::from_min_size(pos2(full.right() - 36.0, full.top() + 4.0), vec2(32.0, 28.0));
    if icon_button(ui, pal, close, icon::CLOSE, 11.0, false, "Close").clicked() {
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
    if icon_button(ui, pal, close.translate(vec2(-34.0, 0.0)), icon::UNMINI, 14.0, false, "Back to full view").clicked() {
        app.toggle_mini(&ctx);
    }
    let c = full.center();
    let sq = |dx: f32| Rect::from_center_size(c + vec2(dx, 0.0), vec2(40.0, 40.0));
    if icon_button(ui, pal, sq(-56.0), icon::BACK10, 20.0, false, "Skip back 10 seconds").clicked() {
        app.skip(-SKIP_BACK);
    }
    if play_button(ui, pal, c, 22.0, app.engine.is_playing()).clicked() {
        app.toggle_play();
    }
    if icon_button(ui, pal, sq(56.0), icon::FORWARD10, 20.0, false, "Skip forward 10 seconds").clicked() {
        app.skip(SKIP_FORWARD);
    }
    let bar = Rect::from_min_max(pos2(full.left() + 4.0, full.bottom() - 26.0), pos2(full.right() - 4.0, full.bottom() - 2.0));
    let t = app.scrub.map_or_else(|| app.engine.time(), |(t, _)| t);
    let (r, at) = scrubber(app, ui, pal, bar, t, app.engine.duration());
    scrub(app, &r, at);
}

pub(super) fn flyout_frame(pal: &Palette, ctx: &egui::Context) -> egui::Frame {
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

pub(super) fn heading(ui: &mut Ui, pal: &Palette, s: &str) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new(s).size(12.0).color(pal.text_dim));
}

/// A row of choice chips (playback speed, zoom); returns the index clicked.
pub(super) fn chips(ui: &mut Ui, pal: &Palette, labels: &[String], on: usize) -> Option<usize> {
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

pub const SPEEDS: [f64; 8] = [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 4.0];

fn flyouts(app: &mut PlayerApp, ctx: &egui::Context, pal: &Palette, full: Rect) {
    let Some(f) = app.flyout else { return };
    let [speed_at, tracks_at, volume_at, more_at] = ANCHORS.with(|a| a.get());
    let (anchor, width) = match f {
        Flyout::Speed => (speed_at, 160.0),
        Flyout::Tracks => (tracks_at, 320.0),
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
            flyout_frame(pal, ctx).show(ui, |ui| {
                ui.set_width(width - 12.0);
                // taller than the room above the bar (More with the visualizer's controls): it scrolls
                egui::ScrollArea::vertical().max_height((anchor.top() - full.top() - 28.0).max(120.0)).show(ui, |ui| match f {
                    Flyout::Speed => speed_flyout(app, ui, pal),
                    Flyout::Tracks => tracks_flyout(app, ui, pal),
                    Flyout::Volume => volume_flyout(app, ui, pal),
                    Flyout::More => more_flyout(app, ui, pal),
                });
            });
        });
    // a press anywhere else closes it
    let rect = area.response.rect;
    if ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !rect.contains(p) && !anchor.contains(p))) {
        app.flyout = None;
    }
}

/// One row per speed, ticked at the current one.
fn speed_flyout(app: &mut PlayerApp, ui: &mut Ui, pal: &Palette) {
    let rate = app.engine.rate();
    for s in SPEEDS {
        let label = if s == 1.0 { "Normal".to_string() } else { speed_label(s) };
        if flyout_row(ui, pal, Some((s - rate).abs() < 1e-6), None, &label).clicked() {
            app.engine.set_rate(s);
            app.flyout = None;
        }
    }
}

/// "Stereo", "Mono", "5.1" ...
fn channels(n: u32) -> String {
    match n {
        0 => String::new(),
        1 => "Mono".into(),
        2 => "Stereo".into(),
        6 => "5.1".into(),
        8 => "7.1".into(),
        n => format!("{n} ch"),
    }
}

fn tracks_flyout(app: &mut PlayerApp, ui: &mut Ui, pal: &Palette) {
    let current_sub = app.subtitle.as_ref().map(|(n, _)| *n);
    let Some(t) = app.tracks().cloned() else {
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().size(14.0));
            ui.label(egui::RichText::new("Reading tracks…").color(pal.text));
        });
        return;
    };
    // audio: every stream the file has, each ticked one mixed in (all of them at first)
    heading(ui, pal, "Audio");
    let streams = app.info.as_ref().map(|i| i.audio_streams.clone()).unwrap_or_default();
    if streams.is_empty() {
        flyout_row(ui, pal, None, None, "No audio");
    }
    let on = app.engine.audio_streams();
    if !streams.is_empty() {
        // none ticked: no audio decoded or played at all
        let none = on.as_ref().is_some_and(|v| v.is_empty());
        if flyout_row(ui, pal, Some(none), None, "None").clicked() {
            app.set_audio_selection(if none { (0..streams.len()).collect() } else { vec![] }, streams.len());
        }
    }
    for (n, a) in streams.iter().enumerate() {
        let name = t.audio.get(n).map_or_else(|| a.label(), |tr| tr.label.clone());
        let ch = channels(a.channels);
        let label = if ch.is_empty() { name } else { format!("{name} · {ch}") };
        let ticked = on.as_ref().is_none_or(|v| v.contains(&n));
        if flyout_row(ui, pal, Some(ticked), None, &label).clicked() {
            app.set_audio_track(n, !ticked, streams.len());
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
    let has_video = app.info.as_ref().is_some_and(|i| i.has_video);
    if has_video && !app.engine.audio_only() {
        heading(ui, pal, "Zoom");
        let labels: Vec<String> = Zoom::ALL.iter().map(|z| z.label().to_string()).collect();
        let on = Zoom::ALL.iter().position(|z| *z == app.zoom).unwrap_or(0);
        if let Some(i) = chips(ui, pal, &labels, on) {
            app.zoom = Zoom::ALL[i];
        }
        heading(ui, pal, "Resolution");
        let opts = app.res_options();
        let mut labels = vec!["Auto".to_string()];
        labels.extend(opts.iter().map(|h| format!("{h}p")));
        let on = app.res.and_then(|r| opts.iter().position(|h| *h == r)).map_or(0, |i| i + 1);
        if let Some(i) = chips(ui, pal, &labels, on) {
            app.set_resolution(if i == 0 { None } else { Some(opts[i - 1]) });
        }
    }
    ui.add_space(6.0);
    ui.separator();
    // the whole folder, round and round (Loop on the bar repeats the one file)
    let all = app.playlist.repeat == Repeat::All;
    if flyout_row(ui, pal, Some(all), None, "Repeat folder").clicked() {
        app.set_repeat_folder(!all);
    }
    ui.separator();
    if app.info.as_ref().is_some_and(|i| !i.audio_streams.is_empty()) {
        let showing = if has_video {
            let ao = app.engine.audio_only();
            if flyout_row(ui, pal, Some(ao), None, "Audio Only").clicked() {
                app.set_audio_only(!ao);
            }
            ao
        } else {
            // songs show their cover; the visualizer when asked for (or when there is no cover)
            let viz = app.song_viz;
            if flyout_row(ui, pal, Some(viz), None, "Visualizer").clicked() {
                app.set_song_visualizer(!viz);
            }
            app.song_shows_visualizer()
        };
        if showing {
            super::panels::visualizer_controls(app, ui, pal);
        }
        ui.separator();
    }
    if flyout_row(ui, pal, None, Some(icon::CONVERT), "Convert…").clicked() {
        app.convert = Some(Default::default());
        app.flyout = None;
    }
    if flyout_row(ui, pal, None, Some(icon::UPSCALE), "Upscale…").clicked() {
        app.upscale = Some(Default::default());
        app.flyout = None;
    }
    if flyout_row(ui, pal, None, Some(icon::TAG), "Properties / Metadata…").clicked() {
        let ctx = ui.ctx().clone();
        app.open_metadata(&ctx);
        app.flyout = None;
    }
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

/// Songs: the cover large and centred over a dark, blurred copy of itself, the title under it; while it
/// loads, a shimmering placeholder (never a blank, then a pop).
fn audio_view(app: &mut PlayerApp, ui: &mut Ui, pal: &Palette, full: Rect, mini: bool) {
    let p = ui.painter().clone();
    p.rect_filled(full, 0.0, Color32::BLACK);
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    let room = full.shrink2(vec2(24.0, if mini { 12.0 } else { TOP_H + 8.0 }));
    let room = if mini { room } else { Rect::from_min_max(room.min, pos2(room.max.x, full.bottom() - BOTTOM_H - 40.0)) };
    // (room for the title under it)
    let side = (room.height() - if mini { 0.0 } else { 44.0 }).min(room.width()).max(48.0);
    let room = room.translate(vec2(0.0, if mini { 0.0 } else { -22.0 }));
    let art = Rect::from_center_size(room.center(), vec2(side, side));
    match &app.cover {
        Some(c) => {
            if let Some(bg) = &app.cover_bg {
                // fill the window (cropped to its aspect), darkened
                let (s, w) = (bg.size_vec2(), full.size());
                let k = (w.x / s.x).max(w.y / s.y);
                p.image(bg.id(), Rect::from_center_size(full.center(), s * k), uv, Color32::from_gray(90));
            }
            let s = c.size_vec2();
            let k = (side / s.x).min(side / s.y);
            let r = Rect::from_center_size(art.center(), s * k);
            p.rect_filled(r.expand(1.0).translate(vec2(0.0, 6.0)), 6.0, Color32::from_black_alpha(110));
            p.image(c.id(), r, uv, Color32::WHITE);
        }
        None => shimmer(ui, art, pal),
    }
    if mini {
        return;
    }
    let title = p.layout_no_wrap(app.title(), FontId::proportional(20.0), Color32::WHITE);
    let at = pos2(full.center().x - title.size().x / 2.0, art.bottom() + 14.0);
    if at.y + title.size().y < full.bottom() - BOTTOM_H {
        p.galley(at, title, Color32::WHITE);
    }
}

/// A loading placeholder: a soft tile with a light band sweeping across it, and a small spinner.
pub(super) fn shimmer(ui: &Ui, rect: Rect, pal: &Palette) {
    let p = ui.painter();
    let t = ui.input(|i| i.time) as f32;
    p.rect_filled(rect, 8.0, pal.text.gamma_multiply(0.07));
    let w = rect.width() * 0.35;
    let x = rect.left() - w + (rect.width() + w) * ((t * 0.6) % 1.0);
    let band = Rect::from_min_max(pos2(x.max(rect.left()), rect.top()), pos2((x + w).min(rect.right()), rect.bottom()));
    if band.width() > 0.0 {
        let mut m = egui::Mesh::default();
        let (clear, lit) = (Color32::TRANSPARENT, pal.text.gamma_multiply(0.08));
        let mid = band.center().x;
        for (a, b, ca, cb) in [(band.left(), mid, clear, lit), (mid, band.right(), lit, clear)] {
            let i = m.vertices.len() as u32;
            m.colored_vertex(pos2(a, band.top()), ca);
            m.colored_vertex(pos2(b, band.top()), cb);
            m.colored_vertex(pos2(a, band.bottom()), ca);
            m.colored_vertex(pos2(b, band.bottom()), cb);
            m.add_triangle(i, i + 1, i + 2);
            m.add_triangle(i + 1, i + 3, i + 2);
        }
        p.add(m);
    }
    let c = rect.center();
    let r = (rect.width().min(rect.height()) * 0.06).clamp(6.0, 14.0);
    let a0 = t * 4.0;
    let pts: Vec<Pos2> = (0..=24).map(|k| a0 + k as f32 / 24.0 * 4.5).map(|a| c + vec2(a.cos(), a.sin()) * r).collect();
    p.add(egui::Shape::line(pts, Stroke::new(2.0, pal.text_dim)));
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
}

/// `img` shrunk to `w` wide (box average) - the cover's blurred backdrop.
pub fn shrink(img: &egui::ColorImage, w: usize) -> egui::ColorImage {
    let [sw, sh] = img.size;
    let w = w.min(sw).max(1);
    let h = (sh * w / sw.max(1)).max(1);
    let mut px = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let (x0, x1, y0, y1) = (x * sw / w, ((x + 1) * sw / w).max(x * sw / w + 1), y * sh / h, ((y + 1) * sh / h).max(y * sh / h + 1));
            let mut acc = [0u32; 4];
            for yy in y0..y1.min(sh) {
                for xx in x0..x1.min(sw) {
                    let c = img.pixels[yy * sw + xx];
                    for (a, v) in acc.iter_mut().zip(c.to_array()) {
                        *a += v as u32;
                    }
                }
            }
            let n = ((y1.min(sh) - y0) * (x1.min(sw) - x0)).max(1) as u32;
            px.push(Color32::from_rgba_premultiplied((acc[0] / n) as u8, (acc[1] / n) as u8, (acc[2] / n) as u8, (acc[3] / n) as u8));
        }
    }
    egui::ColorImage { size: [w, h], pixels: px, source_size: vec2(w as f32, h as f32) }
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
    fn tick_steps_are_sane_for_short_medium_and_long_files() {
        // a 1800 px bar
        assert_eq!(tick_steps(10.0, 1800.0), (1.0, 0.1), "10 s: every second, tenths between");
        assert_eq!(tick_steps(180.0, 1800.0), (1.0, 0.5), "3 min: every second, halves");
        assert_eq!(tick_steps(7200.0, 1800.0), (60.0, 30.0), "2 h: every minute");
        // narrow (mini player): steps grow, never denser than 8 px / 4 px
        for (dur, px) in [(10.0, 300.0), (180.0, 300.0), (7200.0, 300.0), (36000.0, 300.0), (0.5, 1800.0)] {
            let (maj, min) = tick_steps(dur, px);
            let pps = px as f64 / dur;
            assert!(maj >= 1.0 && maj * pps >= 8.0, "{dur} s / {px} px: major {maj}");
            assert!(min == 0.0 || (min * pps >= 4.0 && min < maj), "{dur} s / {px} px: minor {min}");
        }
        assert_eq!(tick_steps(0.0, 100.0), (0.0, 0.0));
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
