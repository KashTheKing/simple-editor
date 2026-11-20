//! The media player's bottom bar for the editor's monitors (Preview and Source): a seek row (elapsed |
//! a thin bar with second ticks, hover thumbnails and the played part in the accent | total) over one
//! row of controls - previous, back 10 s, the ringed play button, forward 10 s, next, loop and reverse in
//! the middle, speed and full screen on the right. The pieces are the player's own (`player::chrome`:
//! `icon_button`, `play_button`, `ticks`, `speed_button`, the icon font), in the theme's colours.
//!
//! `Bar.edit` adds the editor's extras for a real edit (`is_single_clip` false): previous / next cut
//! instead of start / end, frame steps beside play, Mark In / Out and Add Marker on the left, a typed
//! timecode for the elapsed time, and the markers on the seek bar. A timeline that is one clip (an opened
//! file) gets the plain player controls. The in / out band is painted whenever marks are set.
//!
//! Pure UI: clicks come back as a `BarOut`; nothing is decoded here. Hover thumbnails come from the
//! caller's `thumb` (the editor's thumbnail cache, which never touches the playback decoders).

use crate::hotkeys::Action;
use crate::model::{Id, Project};
use crate::player::chrome::{self as pc, icon};
use crate::theme::Palette;
use crate::ui::{menu, preview, timecode};
use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Rect, Sense, Stroke, Ui};

/// The bar's height: the seek row and the controls row.
pub const BAR_H: f32 = 84.0;
const SEEK_Y: f32 = 20.0;
const ROW_Y: f32 = 58.0;
const BTN: f32 = 36.0;
const PLAY_R: f32 = 19.0;
/// Hit areas reach this far past a drawn button (the player's `HIT`).
const HIT: f32 = 4.0;
/// Segoe Fluent Icons / MDL2: one frame back / forward, and a marker.
const STEP_BACK: char = '\u{E76B}'; // ChevronLeft
const STEP_FWD: char = '\u{E76C}'; // ChevronRight
const FLAG: char = '\u{E7C1}';

/// A timeline that is just one clip (a video and its own linked sound count as one): the monitors show
/// the plain player controls. Anything more is an edit and gets the editor's extras.
pub fn is_single_clip(p: &Project) -> bool {
    let mut first: Option<(bool, Id)> = None;
    for (_, c) in p.all_clips() {
        // clips sharing a non-zero link move together (video + its audio): one unit
        let unit = if c.link != 0 { (true, c.link) } else { (false, c.id) };
        match first {
            None => first = Some(unit),
            Some(u) if u == unit => {}
            Some(_) => return false,
        }
    }
    true
}

/// The thumbnail to show for timeline time `t`: the uppermost enabled visual clip there with a file
/// behind it, and the source time in that file. None over a gap, a title or a shape.
pub fn thumb_source(p: &Project, t: f64) -> Option<(String, f64)> {
    p.all_clips()
        .filter(|(_, c)| c.enabled && c.is_visual() && c.contains(t))
        .filter_map(|(_, c)| {
            let a = p.asset(c.asset).filter(|a| a.has_video() || a.kind == crate::model::ClipKind::Image)?;
            Some((a.path.clone(), c.src_time(t)))
        })
        .next()
}

/// What the bar shows. `thumb` is asked for the picture under the pointer while the seek bar is hovered.
pub struct Bar<'a> {
    pub t: f64,
    pub dur: f64,
    pub fps: f64,
    pub playing: bool,
    /// The rate in effect (1 = normal, negative = reverse).
    pub rate: f64,
    pub looping: bool,
    /// The editor's extras (a real edit, not one clip).
    pub edit: bool,
    pub marks: (Option<f64>, Option<f64>),
    /// (time, length, colour) of each marker - drawn in `edit` only.
    pub markers: Vec<(f64, f64, Color32)>,
    /// Cut ticks across the seek bar (the Source Tape's clip boundaries).
    pub cuts: Vec<f64>,
    /// Full screen: the bar sits over the picture, shaded into it.
    pub fullscreen: bool,
    /// Offer the Full screen button (the program monitor's; the Source monitor has none).
    pub fullscreen_button: bool,
    /// `edit`: the elapsed time is the click-to-type timecode (`preview::timecode_label`).
    pub tc_edit: Option<&'a mut Option<String>>,
    pub thumb: &'a mut dyn FnMut(f64) -> Option<(egui::TextureId, [u32; 2])>,
}

#[derive(Default, Debug, PartialEq)]
pub struct BarOut {
    pub seek: Option<f64>,
    /// PlayPause, GoStart / GoEnd, PrevCut / NextCut, StepBack / StepForward, MarkIn / MarkOut,
    /// AddMarker, Fullscreen.
    pub actions: Vec<Action>,
    /// Back / forward 10 s: the signed seconds to move.
    pub skip: Option<f64>,
    /// A speed was picked from the speed menu (unsigned).
    pub rate: Option<f64>,
    pub reverse: bool,
    pub toggle_loop: bool,
    /// The seek bar is being dragged.
    pub scrubbing: bool,
    /// An In / Out flag is being dragged.
    pub mark: Option<MarkDrag>,
}

/// A drag of the seek bar's In (`out` false) or Out flag: the time under the pointer, and whether this
/// frame started / ended the drag (one undo step per drag).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MarkDrag {
    pub out: bool,
    pub t: f64,
    pub start: bool,
    pub end: bool,
}

/// One control in the middle group, in the order they sit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ctl {
    Prev,
    Back10,
    StepBack,
    Play,
    StepFwd,
    Fwd10,
    Next,
    Loop,
    Reverse,
}

/// The middle group for `edit`, dropping the least needed first while it is wider than `room`.
pub(crate) fn centre_controls(edit: bool, room: f32) -> Vec<Ctl> {
    use Ctl::*;
    let mut v = if edit {
        vec![Prev, Back10, StepBack, Play, StepFwd, Fwd10, Next, Loop, Reverse]
    } else {
        vec![Prev, Back10, Play, Fwd10, Next, Loop, Reverse]
    };
    for drop in [[Back10, Fwd10], [Loop, Reverse], [Prev, Next]] {
        if group_width(&v) <= room {
            break;
        }
        v.retain(|c| !drop.contains(c));
    }
    v
}

fn width_of(c: Ctl) -> f32 {
    match c {
        Ctl::Play => PLAY_R * 2.0 + 12.0,
        _ => BTN + 4.0,
    }
}

fn group_width(v: &[Ctl]) -> f32 {
    v.iter().map(|&c| width_of(c)).sum()
}

/// A text button in the icon buttons' look (Mark In "{" / Mark Out "}").
fn text_button(ui: &mut Ui, pal: &Palette, rect: Rect, s: &str, tip: &str) -> egui::Response {
    let r = ui.interact(rect.expand(HIT), ui.id().with(("tbtn", s)), Sense::click()).on_hover_text(tip);
    let p = ui.painter();
    if r.hovered() {
        p.rect_filled(rect.shrink(2.0), 6.0, pal.text.gamma_multiply(0.1));
    }
    p.text(rect.center(), Align2::CENTER_CENTER, s, FontId::monospace(17.0), pal.text);
    r
}

/// The time under the pointer on the hover card / beside the bar.
fn time_text(t: f64, fps: f64, edit: bool) -> String {
    if edit {
        timecode(t, fps)
    } else {
        pc::clock(t)
    }
}

/// Draw the bar into `rect` (`BAR_H` tall). `left` and `right` add the caller's own controls at the row's
/// ends (the right one laid out right to left, inside speed and full screen). Returns the clicks and the
/// middle group's rect.
pub fn show(
    ui: &mut Ui,
    pal: &Palette,
    rect: Rect,
    mut bar: Bar<'_>,
    left: impl FnOnce(&mut Ui),
    right: impl FnOnce(&mut Ui),
) -> (BarOut, Rect) {
    let mut out = BarOut::default();
    let p = ui.painter().clone();
    if bar.fullscreen {
        let shade = Rect::from_min_max(pos2(rect.left(), rect.top() - 60.0), rect.max);
        pc::vgrad(&p, shade, Color32::TRANSPARENT, pal.bg.gamma_multiply(0.85));
    } else {
        p.rect_filled(rect, 0.0, pal.bg);
    }
    // the bar swallows clicks: nothing reaches the picture through it
    ui.interact(rect, ui.id().with("media-bar"), Sense::click());

    // ---- seek row: elapsed | bar | total
    let sy = rect.top() + SEEK_Y;
    let font = FontId::proportional(12.0);
    let dur = bar.dur.max(0.0);
    let total = p.layout_no_wrap(time_text(dur, bar.fps, bar.edit), font.clone(), pal.text_dim);
    let left_x = rect.left() + 12.0;
    let elapsed_w = match bar.tc_edit.take() {
        Some(edit) if bar.edit => {
            let r = Rect::from_min_size(pos2(left_x, sy - 11.0), vec2(104.0, 22.0));
            let mut child = ui.new_child(
                egui::UiBuilder::new().id_salt("bar-tc").max_rect(r).layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            out.seek = preview::timecode_label(&mut child, edit, bar.t, dur, bar.fps);
            child.min_rect().width().max(60.0)
        }
        _ => {
            let g = p.layout_no_wrap(time_text(bar.t, bar.fps, bar.edit), font, pal.text);
            let w = g.size().x;
            p.galley(pos2(left_x, sy - g.size().y / 2.0), g, pal.text);
            w
        }
    };
    p.galley(pos2(rect.right() - 12.0 - total.size().x, sy - total.size().y / 2.0), total.clone(), pal.text_dim);
    let track = Rect::from_min_max(
        pos2(left_x + elapsed_w + 10.0, sy - 12.0),
        pos2(rect.right() - 22.0 - total.size().x, sy + 12.0),
    );
    if track.width() > 20.0 {
        let (seek, dragging, mark) = seek_bar(ui, pal, track, &bar);
        out.seek = seek.or(out.seek);
        out.scrubbing = dragging;
        out.mark = mark;
        if let Some(h) = ui.ctx().pointer_hover_pos().filter(|h| track.expand(2.0).contains(*h)) {
            hover_card(ui, pal, track, h.x, &mut bar);
        }
    }

    // ---- controls row: the caller's left / right, then the middle group between them
    let cy = rect.top() + ROW_Y;
    let row = Rect::from_min_max(pos2(rect.left() + 8.0, cy - BTN / 2.0), pos2(rect.right() - 8.0, cy + BTN / 2.0));
    // right edge: full screen, then speed
    let sq = |x: f32| Rect::from_center_size(pos2(x, cy), vec2(BTN, BTN));
    let fs = sq(row.right() - BTN / 2.0);
    let fs = if bar.fullscreen_button {
        let (fc, ftip) = if bar.fullscreen { (icon::EXIT_FULLSCREEN, "Exit full screen") } else { (icon::FULLSCREEN, "Full screen") };
        let ftip = format!("{ftip}{}", keys(Action::Fullscreen));
        if pc::icon_button(ui, pal, fs, fc, 16.0, false, &ftip).clicked() {
            out.actions.push(Action::Fullscreen);
        }
        fs
    } else {
        Rect::from_min_max(pos2(row.right() + 4.0, fs.top()), pos2(row.right() + 4.0, fs.bottom()))
    };
    let shown_rate = if bar.playing { bar.rate.abs() } else { 1.0 };
    let speed = Rect::from_center_size(pos2(fs.left() - 4.0 - 24.0, cy), vec2(44.0, 28.0));
    let sr = pc::speed_button(ui, pal, speed, shown_rate, false);
    egui::Popup::menu(&sr).align(egui::RectAlign::TOP_END).show(|ui| {
        for s in pc::SPEEDS {
            let label = if s == 1.0 { "Normal".to_string() } else { pc::speed_label(s) };
            if menu::check(ui, (s - shown_rate).abs() < 1e-6, &label, "").clicked() {
                out.rate = Some(s);
            }
        }
    });
    let side = |ui: &mut Ui, salt: &str, r: Rect, layout: egui::Layout, add: &mut dyn FnMut(&mut Ui)| {
        let mut ui = ui.new_child(egui::UiBuilder::new().id_salt(salt).max_rect(r).layout(layout));
        ui.spacing_mut().item_spacing.x = 4.0;
        add(&mut ui);
        ui.min_rect()
    };
    let right_room = Rect::from_min_max(row.min, pos2(speed.left() - 6.0, row.max.y));
    let mut right = Some(right);
    let rr = side(ui, "bar-right", right_room, egui::Layout::right_to_left(egui::Align::Center), &mut |ui| {
        if let Some(f) = right.take() {
            f(ui);
        }
    });
    let rr_left = if rr.width() > 0.0 { rr.left() } else { right_room.right() };
    // left edge: the editor's marks, then the caller's
    let mut lx = row.left();
    if bar.edit {
        for (s, a) in [("{", Action::MarkIn), ("}", Action::MarkOut)] {
            let b = Rect::from_min_size(pos2(lx, cy - BTN / 2.0), vec2(BTN - 4.0, BTN));
            if text_button(ui, pal, b, s, &preview::tip(a)).clicked() {
                out.actions.push(a);
            }
            lx = b.right() + 2.0;
        }
        let b = Rect::from_min_size(pos2(lx, cy - BTN / 2.0), vec2(BTN, BTN));
        if pc::icon_button(ui, pal, b, FLAG, 15.0, false, &preview::tip(Action::AddMarker)).clicked() {
            out.actions.push(Action::AddMarker);
        }
        lx = b.right() + 6.0;
    }
    let mut left = Some(left);
    let left_room = Rect::from_min_max(pos2(lx, row.top()), pos2(rr_left - 4.0, row.bottom()));
    let ll = side(ui, "bar-left", left_room, egui::Layout::left_to_right(egui::Align::Center), &mut |ui| {
        if let Some(f) = left.take() {
            f(ui);
        }
    });
    let l_right = if ll.width() > 0.0 { ll.right() } else { lx };

    // the middle group: play centred in the bar where it fits, else pushed between the two sides
    let (lo, hi) = (l_right + 8.0, rr_left - 8.0);
    let ctls = centre_controls(bar.edit, (hi - lo).max(0.0));
    let w = group_width(&ctls);
    let play_off: f32 = ctls.iter().take_while(|&&c| c != Ctl::Play).map(|&c| width_of(c)).sum::<f32>() + width_of(Ctl::Play) / 2.0;
    let mut x0 = rect.center().x - play_off;
    x0 = x0.min(hi - w).max(lo);
    let group = Rect::from_min_size(pos2(x0, cy - BTN / 2.0), vec2(w, BTN));
    let mut x = x0;
    let reverse = bar.playing && bar.rate < 0.0;
    for c in ctls {
        let cw = width_of(c);
        let at = pos2(x + cw / 2.0, cy);
        x += cw;
        let b = Rect::from_center_size(at, vec2(BTN, BTN));
        let (glyph, px, on, tip, act): (char, f32, bool, String, Option<Action>) = match c {
            Ctl::Play => {
                if pc::play_button(ui, pal, at, PLAY_R, bar.playing).clicked() {
                    out.actions.push(Action::PlayPause);
                }
                continue;
            }
            Ctl::Prev if bar.edit => (icon::PREVIOUS, 15.0, false, preview::tip(Action::PrevCut), Some(Action::PrevCut)),
            Ctl::Prev => (icon::PREVIOUS, 15.0, false, preview::tip(Action::GoStart), Some(Action::GoStart)),
            Ctl::Next if bar.edit => (icon::NEXT, 15.0, false, preview::tip(Action::NextCut), Some(Action::NextCut)),
            Ctl::Next => (icon::NEXT, 15.0, false, preview::tip(Action::GoEnd), Some(Action::GoEnd)),
            Ctl::StepBack => (STEP_BACK, 13.0, false, preview::tip(Action::StepBack), Some(Action::StepBack)),
            Ctl::StepFwd => (STEP_FWD, 13.0, false, preview::tip(Action::StepForward), Some(Action::StepForward)),
            Ctl::Back10 => (icon::BACK10, 19.0, false, "Back 10 seconds".into(), None),
            Ctl::Fwd10 => (icon::FORWARD10, 19.0, false, "Forward 10 seconds".into(), None),
            Ctl::Loop => {
                let t = if bar.looping { "Loop: on" } else { "Loop: off" };
                let t = if bar.marks.0.is_some() && bar.marks.1.is_some() { format!("{t} (In to Out)") } else { t.into() };
                (icon::LOOP, 15.0, bar.looping, t, None)
            }
            Ctl::Reverse => (icon::REVERSE, 15.0, reverse, "Reverse".into(), None),
        };
        if pc::icon_button(ui, pal, b, glyph, px, on, &tip).clicked() {
            match (c, act) {
                (_, Some(a)) => out.actions.push(a),
                (Ctl::Back10, _) => out.skip = Some(-10.0),
                (Ctl::Fwd10, _) => out.skip = Some(10.0),
                (Ctl::Loop, _) => out.toggle_loop = true,
                (Ctl::Reverse, _) => out.reverse = true,
                _ => {}
            }
        }
    }
    (out, group)
}

/// " (F11)": a tooltip's shortcut tail, empty when the action has no key.
fn keys(a: Action) -> String {
    match menu::shortcut(a) {
        k if k.is_empty() => String::new(),
        k => format!(" ({k})"),
    }
}

/// The seek bar: ticks, the track, the in / out band, markers and cut ticks, the played part, the knob.
/// All of `rect` grabs. Returns the time under the pointer while clicked / dragged, and whether it is.
fn seek_bar(ui: &mut Ui, pal: &Palette, rect: Rect, bar: &Bar<'_>) -> (Option<f64>, bool, Option<MarkDrag>) {
    let r = ui.interact(rect, ui.id().with("bar-seek"), Sense::click_and_drag());
    let p = ui.painter();
    let (x0, x1) = (rect.left() + 6.0, rect.right() - 6.0);
    let y = rect.center().y;
    let dur = bar.dur.max(f64::MIN_POSITIVE);
    let xt = |t: f64| x0 + (x1 - x0) * (t / dur).clamp(0.0, 1.0) as f32;
    let hot = r.hovered() || r.dragged();
    let th = if hot { 5.0 } else { 3.0 };
    pc::ticks(p, pal, x0, x1, y, bar.dur);
    let line = |a: f32, b: f32, c| p.rect_filled(Rect::from_min_max(pos2(a, y - th / 2.0), pos2(b, y + th / 2.0)), th / 2.0, c);
    line(x0, x1, pal.text_dim.gamma_multiply(0.45));
    if bar.marks.0.is_some() || bar.marks.1.is_some() {
        let (a, b) = (bar.marks.0.unwrap_or(0.0), bar.marks.1.unwrap_or(bar.dur));
        if b > a {
            let band = Rect::from_min_max(pos2(xt(a), y - 7.0), pos2(xt(b), y + 7.0));
            p.rect_filled(band, 2.0, pal.selection.gamma_multiply(0.35));
        }
        for (m, dir) in [(bar.marks.0, 1.0f32), (bar.marks.1, -1.0)] {
            let Some(m) = m else { continue };
            let x = xt(m);
            p.line_segment([pos2(x, y - 8.0), pos2(x, y + 8.0)], Stroke::new(1.5, pal.accent));
            let tri = vec![pos2(x, y - 9.0), pos2(x + dir * 5.0, y - 9.0), pos2(x, y - 4.0)];
            p.add(egui::Shape::convex_polygon(tri, pal.accent, Stroke::NONE));
        }
    }
    for &c in &bar.cuts {
        let x = xt(c);
        p.line_segment([pos2(x, y - 6.0), pos2(x, y + 6.0)], Stroke::new(1.0, pal.text));
    }
    line(x0, xt(bar.t), pal.accent);
    if bar.edit {
        for &(t, len, col) in &bar.markers {
            let x = xt(t);
            if len > 0.0 {
                p.rect_filled(Rect::from_min_max(pos2(x, y - 9.0), pos2(xt(t + len), y - 6.0)), 1.0, col);
            }
            let tri = vec![pos2(x - 4.0, y - 11.0), pos2(x + 4.0, y - 11.0), pos2(x, y - 5.0)];
            p.add(egui::Shape::convex_polygon(tri, col, Stroke::NONE));
        }
    }
    let x = xt(bar.t);
    let rad = if hot { 7.0 } else { 5.5 };
    p.circle_filled(pos2(x, y), rad, pal.accent);
    p.circle_stroke(pos2(x, y), rad, Stroke::new(1.5, pal.text));
    let dragging = r.dragged() || r.drag_started();
    let seek = r
        .interact_pointer_pos()
        .filter(|_| r.dragged() || r.clicked() || r.drag_started())
        .map(|pos| preview::scrub_time(((pos.x - x0) / (x1 - x0)) as f64, bar.dur));
    // the In / Out flags drag like markers (registered after the bar, so they win the pointer there)
    let mut mark = None;
    if bar.edit {
        for (out, m) in [(false, bar.marks.0), (true, bar.marks.1)] {
            let Some(m) = m else { continue };
            let hr = Rect::from_center_size(pos2(xt(m), y - 3.0), vec2(10.0, 16.0));
            let tip = if out { "Out: drag to move" } else { "In: drag to move" };
            let mr = ui
                .interact(hr, ui.id().with(("bar-mark", out)), Sense::drag())
                .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
                .on_hover_text(tip);
            if mr.dragged() || mr.drag_started() || mr.drag_stopped() {
                if let Some(pos) = mr.interact_pointer_pos().or(ui.ctx().pointer_latest_pos()) {
                    let t = preview::scrub_time(((pos.x - x0) / (x1 - x0)) as f64, bar.dur);
                    mark = Some(MarkDrag { out, t, start: mr.drag_started(), end: mr.drag_stopped() });
                }
            }
        }
    }
    (seek, dragging, mark)
}

/// Over the seek bar: the picture at that moment (when the cache has it) with the time; just the time
/// otherwise. Never waits for a thumbnail.
fn hover_card(ui: &mut Ui, pal: &Palette, track: Rect, hx: f32, bar: &mut Bar<'_>) {
    let (x0, x1) = (track.left() + 6.0, track.right() - 6.0);
    let ht = (((hx - x0) / (x1 - x0)).clamp(0.0, 1.0) as f64) * bar.dur.max(0.0);
    let ctx = ui.ctx().clone();
    // a tooltip layer: over the picture and any pane above the bar
    let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, ui.id().with("bar-hover")));
    let g = p.layout_no_wrap(time_text(ht, bar.fps, bar.edit), FontId::proportional(12.0), pal.text);
    let screen = ctx.content_rect();
    let place = |size: egui::Vec2| {
        let left = (hx - size.x / 2.0).clamp(screen.left() + 4.0, (screen.right() - 4.0 - size.x).max(screen.left() + 4.0));
        Rect::from_min_size(pos2(left, track.top() - 4.0 - size.y), size)
    };
    let radius = ui.visuals().widgets.inactive.corner_radius;
    // `thumb` only reads the cache (a miss queues it on the thumbnail worker): the time shows meanwhile
    let pic = (bar.thumb)(ht);
    if let Some((id, px)) = pic {
        let img = vec2(160.0, 160.0 * px[1] as f32 / px[0].max(1) as f32);
        let card = place(img + vec2(8.0, 8.0 + g.size().y + 6.0));
        p.rect(card, radius, pal.panel, Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
        let img_r = Rect::from_min_size(card.min + vec2(4.0, 4.0), img);
        p.image(id, img_r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        p.galley(pos2(card.center().x - g.size().x / 2.0, img_r.bottom() + 4.0), g, pal.text);
    } else {
        let b = place(g.size() + vec2(12.0, 6.0));
        p.rect(b, radius, pal.panel, Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
        p.galley(b.center() - g.size() / 2.0, g, pal.text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Asset, ClipKind};

    fn asset(id: Id, path: &str) -> Asset {
        Asset {
            id,
            path: path.into(),
            kind: ClipKind::Video,
            duration: 10.0,
            width: 1920,
            height: 1080,
            fps: 30.0,
            audio_streams: vec![crate::model::AudioStreamInfo { channels: 2, ..Default::default() }; 2],
            codec: String::new(),
            folder: String::new(),
            tags: Vec::new(),
            label: 0,
            description: String::new(),
            rel_path: None,
            parent: None,
            range: None,
            effects: Vec::new(),
            html: None,
            proxy: None,
        }
    }

    #[test]
    fn one_clip_is_single_and_an_edit_is_not() {
        let p = Project::from_media(asset(1, "a.mp4"));
        assert!(p.all_clips().count() >= 2, "a video and its audio tracks");
        assert!(is_single_clip(&p), "an opened file (video + its linked audio): one clip");
        assert!(is_single_clip(&Project::default()), "an empty timeline shows the plain controls");
        let mut two = p.clone();
        let mut c = two.all_clips().next().map(|(_, c)| c.clone()).expect("a clip");
        c.id = 9999;
        c.link = 0;
        c.start = 20.0;
        two.tracks[0].clips.push(c);
        assert!(!is_single_clip(&two), "a second clip makes it an edit");
    }

    #[test]
    fn the_edit_bar_adds_frame_steps_and_drops_skips_first_when_narrow() {
        let plain = centre_controls(false, 2000.0);
        assert!(!plain.contains(&Ctl::StepBack) && plain.contains(&Ctl::Back10), "{plain:?}");
        let edit = centre_controls(true, 2000.0);
        assert!(edit.contains(&Ctl::StepBack) && edit.contains(&Ctl::StepFwd) && edit.contains(&Ctl::Back10), "{edit:?}");
        let narrow = centre_controls(true, 260.0);
        assert!(!narrow.contains(&Ctl::Back10) && narrow.contains(&Ctl::StepBack) && narrow.contains(&Ctl::Play), "{narrow:?}");
        assert!(centre_controls(true, 0.0).contains(&Ctl::Play), "play never goes");
    }

    /// A click on the seek bar seeks there; the ringed play button plays; Loop / Reverse / skips report
    /// themselves. Idle, the bar asks for no repaint.
    #[test]
    fn assert_no_idle_repaint_media_bar_clicks_seek_and_play() {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let pal = Palette::new(true, Color32::WHITE);
        let screen = Rect::from_min_size(egui::Pos2::ZERO, vec2(900.0, 300.0));
        let rect = Rect::from_min_max(pos2(0.0, 300.0 - BAR_H), pos2(900.0, 300.0));
        let mut time = 0.0;
        let mut run = |events: Vec<egui::Event>| {
            time += 0.05;
            let mut out = BarOut::default();
            let input = egui::RawInput { screen_rect: Some(screen), time: Some(time), events, ..Default::default() };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut thumb = |_t: f64| None;
                    let bar = Bar {
                        t: 2.0,
                        dur: 10.0,
                        fps: 30.0,
                        playing: false,
                        rate: 1.0,
                        looping: false,
                        edit: false,
                        marks: (None, None),
                        markers: Vec::new(),
                        cuts: Vec::new(),
                        fullscreen: false,
                        fullscreen_button: true,
                        tc_edit: None,
                        thumb: &mut thumb,
                    };
                    out = show(ui, &pal, rect, bar, |_| {}, |_| {}).0;
                });
            });
            out
        };
        let click = |run: &mut dyn FnMut(Vec<egui::Event>) -> BarOut, at: egui::Pos2| {
            let b = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
            run(vec![egui::Event::PointerMoved(at)]);
            run(vec![b(true)]);
            run(vec![b(false)])
        };
        run(vec![]);
        // the seek bar runs between the elapsed and total times: its middle is about the file's middle
        let out = click(&mut run, pos2(450.0, rect.top() + SEEK_Y));
        let t = out.seek.expect("a click on the seek bar seeks");
        assert!((t - 5.0).abs() < 0.6, "the middle of the bar: {t}");
        let out = click(&mut run, pos2(450.0, rect.top() + ROW_Y));
        assert_eq!(out.actions, vec![Action::PlayPause], "the play button sits in the middle");
        let out = click(&mut run, pos2(450.0 + width_of(Ctl::Play) / 2.0 + width_of(Ctl::Fwd10) / 2.0, rect.top() + ROW_Y));
        assert_eq!(out.skip, Some(10.0), "forward 10 s sits right of play");
        // pointer gone, nothing open: no repaint asked for
        run(vec![egui::Event::PointerGone]);
        for _ in 0..4 {
            run(vec![]);
        }
        assert!(!ctx.has_requested_repaint(), "the bar idles");
    }
}
