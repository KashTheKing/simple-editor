//! The thin title bar of an undecorated window (the player's and the editor's): the empty bar drags
//! the window and a double-click maximizes / restores it, minimise / maximise / close sit at the top
//! right in the theme's colours (close red under the pointer, Windows' convention), and the window's
//! edges resize it. The OS move / size loops (`StartDrag`, `BeginResize`) keep Aero Snap and Win+arrows.

use crate::theme::Palette;
use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Rect, Response, Sense, Ui};

/// Segoe Fluent Icons / MDL2 Assets.
pub const MINIMIZE: char = '\u{E921}';
pub const MAXIMIZE: char = '\u{E922}';
pub const RESTORE: char = '\u{E923}';
pub const CLOSE: char = '\u{E8BB}';
/// The caption buttons' size.
pub const BUTTON: egui::Vec2 = vec2(46.0, 32.0);

/// The bar's empty area. Call it before the bar's own widgets, so they sit on top of it.
pub fn drag_area(ui: &mut Ui, rect: Rect, maximized: bool) {
    let bar = ui.interact(rect, ui.id().with("titlebar"), Sense::click_and_drag());
    if bar.drag_started() {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    if bar.double_clicked() {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
    }
}

/// Minimise, maximise / restore and close at `bar`'s top right. Returns the rect they cover.
pub fn caption_buttons(ui: &mut Ui, pal: &Palette, bar: Rect, maximized: bool) -> Rect {
    let ctx = ui.ctx().clone();
    let close = Rect::from_min_size(pos2(bar.right() - BUTTON.x, bar.top()), BUTTON);
    let max = close.translate(vec2(-BUTTON.x, 0.0));
    let min = max.translate(vec2(-BUTTON.x, 0.0));
    if caption_button(ui, pal, min, MINIMIZE, false, "Minimize").clicked() {
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
    }
    let (c, tip) = if maximized { (RESTORE, "Restore down") } else { (MAXIMIZE, "Maximize") };
    if caption_button(ui, pal, max, c, false, tip).clicked() {
        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
    }
    if caption_button(ui, pal, close, CLOSE, true, "Close").clicked() {
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
    min.union(close)
}

/// The bar's own buttons (File, Edit, the page tabs...) span its full height, like the window buttons:
/// square fills, a comfortable side padding, the text as before. A `MenuBar` style.
pub fn bar_style(style: &mut egui::Style) {
    egui::containers::menu::menu_style(style);
    style.spacing.interact_size.y = BUTTON.y;
    style.spacing.button_padding = vec2(10.0, 0.0);
    style.spacing.item_spacing.x = 0.0;
    let w = &mut style.visuals.widgets;
    for v in [&mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
        v.corner_radius = egui::CornerRadius::ZERO;
        v.expansion = 0.0;
    }
}

/// An icon button of the bar (Performance, Quick Export): full height, 40 px wide, the hover fill of the
/// window buttons; the glyph at its usual size.
pub fn icon_button(ui: &mut Ui, pal: &Palette, id: egui::Id, g: crate::ui::tools::Glyph, tip: &str) -> Response {
    let (rect, _) = ui.allocate_exact_size(vec2(ICON_W, BUTTON.y), Sense::hover());
    let r = ui.interact(rect, id, Sense::click());
    if r.hovered() {
        ui.painter().rect_filled(rect, 0.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    crate::ui::tools::draw_glyph(ui.painter(), rect, g, pal.text);
    r.on_hover_text(tip)
}

/// The width of a bar icon button.
pub const ICON_W: f32 = 40.0;

/// Window buttons: square, the close one red under the pointer.
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
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, c, FontId::new(10.0, crate::theme::icons()), fg);
    r
}

/// Grab an edge to resize (an undecorated window has no frame of its own). Not while maximized.
pub fn resize_edges(ctx: &egui::Context, full: Rect) {
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

/// Round window corners: not maximized or fullscreen (they meet the screen's edges there), and not in
/// the sharp look (corner radius 2, or a rounding of 0-2).
pub fn rounded(sharp: bool, maximized: bool, fullscreen: bool) -> bool {
    !(sharp || maximized || fullscreen)
}

/// The 1 px outline in the theme's border colour: only on a window that is neither maximized nor
/// fullscreen.
pub fn outlined(maximized: bool, fullscreen: bool) -> bool {
    !(maximized || fullscreen)
}

/// The palette every window's bar is drawn in: the editor / player set theirs each frame, a child
/// window (a popped-out pane, a screen window) reads it.
pub fn set_palette(ctx: &egui::Context, pal: Palette) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("titlebar-palette"), pal));
}

fn palette(ctx: &egui::Context) -> Palette {
    ctx.data(|d| d.get_temp(egui::Id::new("titlebar-palette"))).unwrap_or_else(|| crate::theme::palette(ctx))
}

/// The window's corners and outline from the theme and its state (call once a frame, in its viewport);
/// the OS is told only when they change (theme switched, maximized, restored, full screen). `hwnd` is
/// asked for only then.
pub fn frame_style(ctx: &egui::Context, hwnd: impl FnOnce() -> Option<windows::Win32::Foundation::HWND>) {
    let sharp = ctx.style().visuals.window_corner_radius.nw <= 2;
    let (max, full) = ctx.input(|i| (i.viewport().maximized.unwrap_or(false), i.viewport().fullscreen.unwrap_or(false)));
    let b = palette(ctx).border;
    let want = (rounded(sharp, max, full), outlined(max, full).then_some([b.r(), b.g(), b.b()]));
    let id = egui::Id::new(("titlebar-frame", ctx.viewport_id()));
    let mut native = ctx.data(|d| d.get_temp::<((bool, Option<[u8; 3]>), bool)>(id)).filter(|(w, _)| *w == want).map(|(_, n)| n);
    if native.is_none() {
        if let Some(h) = hwnd() {
            crate::winpos::smooth_moves(h);
            let n = crate::winpos::style_frame(h, want.0, want.1);
            ctx.data_mut(|d| d.insert_temp(id, (want, n)));
            native = Some(n);
        }
    }
    // no DWM outline (a square window, Windows 10): the 1 px stroke inside the client's edge instead
    if want.1.is_some() && native == Some(false) {
        let r = ctx.content_rect().shrink(0.5);
        let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("titlebar-outline")));
        p.rect_stroke(r, 0.0, egui::Stroke::new(1.0, b), egui::StrokeKind::Middle);
    }
}

/// `frame_style` for a main window (the editor's, the player's).
pub fn corners(ctx: &egui::Context, handle: &impl raw_window_handle::HasWindowHandle) {
    frame_style(ctx, || crate::winpos::hwnd_of(handle));
}

/// Every other Simple Editor window (a popped-out pane, a screen window, the HTML editor, the recorder):
/// undecorated, drawn by `window`.
pub fn viewport(title: impl Into<String>) -> egui::ViewportBuilder {
    egui::ViewportBuilder::default().with_title(title).with_decorations(false)
}

/// Such a window's chrome, called first inside its viewport, before its panels: the same thin bar as the
/// editor's (icon, `title` - the viewport's own title - and the window buttons; the empty bar drags and
/// double-click maximizes), its corners and outline, and edges that resize. Close asks that viewport to
/// close (`close_requested`), so each window keeps what its close did before (a pane docks back).
pub fn window(ctx: &egui::Context, title: &str) {
    let pal = palette(ctx);
    let (max, full) = ctx.input(|i| (i.viewport().maximized.unwrap_or(false), i.viewport().fullscreen.unwrap_or(false)));
    if !full {
        let frame = egui::Frame::side_top_panel(&ctx.style()).inner_margin(egui::Margin::ZERO);
        egui::TopBottomPanel::top("titlebar").frame(frame).exact_height(BUTTON.y).show(ctx, |ui| {
            let bar = ui.max_rect();
            drag_area(ui, bar, max);
            if let Some(i) = app_icon(ctx) {
                paint_icon(ui, &i, pos2(bar.left() + 10.0, bar.center().y));
            }
            let caption = caption_buttons(ui, &pal, bar, max);
            let text = Rect::from_min_max(pos2(bar.left() + 36.0, bar.top()), pos2(caption.left() - 8.0, bar.bottom()));
            let t = ui.painter().with_clip_rect(text).text(text.left_center(), Align2::LEFT_CENTER, title, FontId::proportional(12.0), pal.text);
            // a popped-out experimental pane (Screen Recording, the Assistant...) keeps its badge
            if crate::ui::layout::Pane::by_title(title).is_some_and(crate::ui::experimental::pane) && t.right() + 90.0 < caption.left() {
                let pill = crate::ui::experimental::paint(ui.painter(), pos2(t.right() + 8.0, bar.center().y));
                crate::ui::experimental::tip(ui, pill, ui.id());
            }
        });
    }
    if !(max || full) {
        resize_edges(ctx, ctx.content_rect());
    }
    frame_style(ctx, || crate::winpos::thread_window(title));
}

/// The 32 px picture of `assets/app.ico` (its entries are PNGs), decoded once per context.
pub fn app_icon(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let id = egui::Id::new("app-icon");
    if let Some(t) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return Some(t);
    }
    let t = decode_icon(ctx)?;
    ctx.data_mut(|d| d.insert_temp(id, t.clone()));
    Some(t)
}

fn decode_icon(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    static ICO: &[u8] = include_bytes!("../../assets/app.ico");
    let n = u16::from_le_bytes([ICO[4], ICO[5]]) as usize;
    let entry = (0..n).map(|i| &ICO[6 + 16 * i..22 + 16 * i]).find(|e| e[0] == 32)?;
    let size = u32::from_le_bytes([entry[8], entry[9], entry[10], entry[11]]) as usize;
    let off = u32::from_le_bytes([entry[12], entry[13], entry[14], entry[15]]) as usize;
    let f = crate::engine::html::decode_png(ICO.get(off..off + size)?).ok()?;
    let img = egui::ColorImage::from_rgba_unmultiplied([f.width as usize, f.height as usize], &f.rgba);
    Some(ctx.load_texture("app-icon", img, egui::TextureOptions::LINEAR))
}

/// Paint `icon` 16 px tall, left-centred at `at`.
pub fn paint_icon(ui: &Ui, icon: &egui::TextureHandle, at: egui::Pos2) {
    let r = Rect::from_min_size(at - vec2(0.0, 8.0), vec2(16.0, 16.0));
    ui.painter().image(icon.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, PointerButton, ViewportCommand as V, ViewportId};

    fn pal() -> Palette {
        crate::theme::palette_with(&egui::Context::default(), &Default::default())
    }

    /// One frame of a bar 800 x 32 with the buttons at its right; returns the commands it sent.
    fn frame(ctx: &egui::Context, t: f64, events: Vec<Event>, maximized: bool) -> Vec<V> {
        let raw = egui::RawInput {
            time: Some(t),
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))),
            events,
            ..Default::default()
        };
        let pal = pal();
        let out = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
                let bar = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, BUTTON.y));
                drag_area(ui, bar, maximized);
                caption_buttons(ui, &pal, bar, maximized);
            });
        });
        out.viewport_output.get(&ViewportId::ROOT).map(|v| v.commands.clone()).unwrap_or_default()
    }

    fn click(ctx: &egui::Context, t: f64, at: egui::Pos2, maximized: bool) -> Vec<V> {
        let press = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        let mut cmds = frame(ctx, t, vec![Event::PointerMoved(at)], maximized);
        cmds.extend(frame(ctx, t + 0.01, vec![press(true)], maximized));
        cmds.extend(frame(ctx, t + 0.02, vec![press(false)], maximized));
        cmds
    }

    /// File / Edit, a page tab and the icon buttons fill the bar from its top edge to its bottom edge,
    /// and a hovered one paints its fill over that whole height.
    #[test]
    fn bar_buttons_span_the_full_height() {
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx, "dark", &Default::default(), "cozy");
        let pal = pal();
        let run = |hover: Option<egui::Pos2>| {
            let mut rects = Vec::new();
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))),
                events: hover.map(Event::PointerMoved).into_iter().collect(),
                ..Default::default()
            };
            let out = ctx.run(raw, |ctx| {
                let frame = egui::Frame::side_top_panel(&ctx.style()).inner_margin(egui::Margin { left: 8, ..Default::default() });
                egui::TopBottomPanel::top("bar").frame(frame).exact_height(BUTTON.y).show(ctx, |ui| {
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.spacing_mut().interact_size.y = BUTTON.y; // as `App::menu_bar`
                        egui::MenuBar::new().style(bar_style).ui(ui, |ui| {
                            rects.push(ui.button("File").rect);
                            // a page tab: framed only while hovered (or the page on screen), drags to reorder
                            let page = egui::Button::new("Plan").min_size(vec2(90.0, 0.0)).frame_when_inactive(false);
                            rects.push(ui.add(page.sense(Sense::click_and_drag())).rect);
                            let id = egui::Id::new("perf");
                            rects.push(icon_button(ui, &pal, id, crate::ui::tools::Glyph::Perf, "Performance").rect);
                        });
                    });
                });
            });
            (rects, out)
        };
        let (rects, _) = run(None);
        for r in &rects {
            assert_eq!((r.top(), r.bottom()), (0.0, BUTTON.y), "{r:?}");
        }
        for r in rects {
            run(Some(r.center())); // the hover registers, the next frame paints it
            let (_, out) = run(Some(r.center()));
            let full = out.shapes.iter().any(|c| match &c.shape {
                egui::Shape::Rect(s) => s.fill != Color32::TRANSPARENT && s.rect.contains_rect(r.shrink(0.5)),
                _ => false,
            });
            assert!(full, "a hovered button fills its full-height rect {r:?}");
        }
    }

    #[test]
    fn the_outline_shows_only_on_a_restored_window() {
        assert!(outlined(false, false));
        assert!(!outlined(true, false), "maximized");
        assert!(!outlined(false, true), "fullscreen");
    }

    #[test]
    fn corners_round_only_restored_and_not_sharp() {
        assert!(rounded(false, false, false));
        assert!(!rounded(true, false, false), "sharp");
        assert!(!rounded(false, true, false), "maximized");
        assert!(!rounded(false, false, true), "fullscreen");
    }

    #[test]
    fn caption_buttons_minimize_maximize_restore_and_close() {
        let y = BUTTON.y / 2.0;
        let at = |n: f32| pos2(800.0 - BUTTON.x * n + BUTTON.x / 2.0, y);
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx, "dark", &Default::default(), "cozy"); // fonts: the icon font
        assert!(click(&ctx, 0.0, at(3.0), false).contains(&V::Minimized(true)));
        assert!(click(&ctx, 1.0, at(2.0), false).contains(&V::Maximized(true)));
        assert!(click(&ctx, 2.0, at(2.0), true).contains(&V::Maximized(false)));
        assert!(click(&ctx, 3.0, at(1.0), false).contains(&V::Close));
    }

    #[test]
    fn double_clicking_the_empty_bar_toggles_maximize_and_dragging_moves_the_window() {
        let at = pos2(300.0, 16.0);
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx, "dark", &Default::default(), "cozy"); // fonts: the icon font
        let mut cmds = click(&ctx, 0.0, at, false);
        assert!(!cmds.iter().any(|c| matches!(c, V::Maximized(_))), "one click maximizes nothing");
        cmds.extend(click(&ctx, 0.1, at, false));
        assert!(cmds.contains(&V::Maximized(true)), "{cmds:?}");
        let cmds = click(&ctx, 5.0, at, true);
        let cmds = [cmds, click(&ctx, 5.1, at, true)].concat();
        assert!(cmds.contains(&V::Maximized(false)), "{cmds:?}");

        let press = Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Default::default() };
        frame(&ctx, 10.0, vec![Event::PointerMoved(at)], false);
        frame(&ctx, 10.01, vec![press], false);
        let cmds = frame(&ctx, 10.3, vec![Event::PointerMoved(at + vec2(40.0, 0.0))], false);
        assert!(cmds.contains(&V::StartDrag), "{cmds:?}");
        // a click on a caption button is not a drag of the bar
        assert!(!click(&ctx, 20.0, pos2(790.0, 16.0), false).contains(&V::StartDrag));
    }
}
