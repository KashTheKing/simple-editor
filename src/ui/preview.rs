//! Preview panel: the rendered frame (letterboxed, black bars), a selection outline for the selected
//! visual clip (drag to move = edits clip.x / clip.y at the playhead time via `Animated::set_at`, calling
//! `undo` once at drag start), and the transport bar: |◀  ◀◀(prev cut)  ◀(step)  ▶/⏸  ▶(step)  ▶▶  ▶|
//! plus timecode / duration, In/Out buttons and the in/out times.
//! In `fullscreen` mode only the video is drawn (no transport, no overlay): Esc / F11 leave it (app).

use crate::engine::compose::placement;
use crate::hotkeys::Action;
use crate::media::Frame;
use crate::model::{ClipKind, Id, Project};
use crate::theme::Palette;
use crate::ui::timecode;
use eframe::egui::{self, pos2, vec2, Color32, Pos2, Rect, Sense, Shape, Stroke, TextureOptions, Vec2};
use std::sync::Arc;

#[derive(Default)]
pub struct PreviewState {
    pub texture: Option<egui::TextureHandle>,
    /// Active overlay drag: clip (x, y) at drag start and the accumulated pointer delta (points).
    drag: Option<(f64, f64, Vec2)>,
}

pub struct PreviewCtx<'a> {
    pub project: &'a mut Project,
    pub selection: &'a [Id],
    pub playhead: f64,
    pub playing: bool,
    /// Video only: no transport bar, no selection overlay / drag.
    pub fullscreen: bool,
    pub palette: &'a Palette,
    pub undo: &'a mut dyn FnMut(&Project),
    /// A newly rendered frame to upload this update (None = keep the current texture).
    pub frame: Option<Arc<Frame>>,
}

#[derive(Default)]
pub struct PreviewResponse {
    pub seek: Option<f64>,
    /// Transport buttons map straight to actions (PlayPause, Stop, StepBack, …, MarkIn, ClearInOut).
    pub actions: Vec<Action>,
    pub edited: bool,
    /// Pixel size available for the video image (for Player::set_canvas).
    pub canvas: (u32, u32),
}

pub fn show(ui: &mut egui::Ui, state: &mut PreviewState, mut c: PreviewCtx<'_>) -> PreviewResponse {
    let mut r = PreviewResponse::default();
    if c.fullscreen {
        video(ui, state, &mut c, &mut r);
        return r;
    }
    ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
        transport(ui, &c, &mut r);
        video(ui, state, &mut c, &mut r);
    });
    r
}

fn transport(ui: &mut egui::Ui, c: &PreviewCtx<'_>, r: &mut PreviewResponse) {
    let fps = c.project.fps;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let mut b = |ui: &mut egui::Ui, label: &str, a: Action| {
            if ui.button(label).clicked() {
                r.actions.push(a);
            }
        };
        b(ui, "|◀", Action::GoStart);
        b(ui, "◀◀", Action::PrevCut);
        b(ui, "◀", Action::StepBack);
        b(ui, if c.playing { "⏸" } else { "▶" }, Action::PlayPause);
        b(ui, "■", Action::Stop);
        b(ui, "▶", Action::StepForward);
        b(ui, "▶▶", Action::NextCut);
        b(ui, "▶|", Action::GoEnd);
        ui.add_space(8.0);
        ui.monospace(format!("{} / {}", timecode(c.playhead, fps), timecode(c.project.duration(), fps)));
        ui.add_space(8.0);
        b(ui, "In", Action::MarkIn);
        b(ui, "Out", Action::MarkOut);
        b(ui, "Clear", Action::ClearInOut);
        if c.project.in_point.is_some() || c.project.out_point.is_some() {
            ui.add_space(4.0);
            let tc = |t: Option<f64>| t.map(|t| timecode(t, fps)).unwrap_or_else(|| "-".into());
            ui.monospace(format!("{} → {}", tc(c.project.in_point), tc(c.project.out_point)));
        }
        ui.add_space(8.0);
        b(ui, "⛶", Action::Fullscreen);
    });
}

/// Largest `aspect` rect centred in `area`, edges snapped to whole physical pixels.
fn letterbox(area: Rect, aspect: f32, ppp: f32) -> Rect {
    let (aw, ah) = (area.width(), area.height());
    let (w, h) = if aw / ah > aspect { (ah * aspect, ah) } else { (aw, aw / aspect) };
    let snap = |v: f32| (v * ppp).round() / ppp;
    let min = pos2(snap(area.min.x + (aw - w) / 2.0), snap(area.min.y + (ah - h) / 2.0));
    Rect::from_min_size(min, vec2(snap(w), snap(h)))
}

fn video(ui: &mut egui::Ui, state: &mut PreviewState, c: &mut PreviewCtx<'_>, r: &mut PreviewResponse) {
    let (rect, resp) = ui.allocate_exact_size(ui.available_size_before_wrap(), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, Color32::BLACK);
    let ppp = ui.pixels_per_point();
    let aspect = c.project.width.max(1) as f32 / c.project.height.max(1) as f32;
    let lb = letterbox(rect, aspect, ppp);
    let (cw, ch) = ((lb.width() * ppp).round().max(16.0) as u32, (lb.height() * ppp).round().max(16.0) as u32);
    r.canvas = (cw, ch);

    if let Some(f) = &c.frame {
        let (w, h) = (f.width as usize, f.height as usize);
        if w > 0 && h > 0 && f.rgba.len() == w * h * 4 {
            let img = egui::ColorImage::from_rgba_premultiplied([w, h], &f.rgba);
            match &mut state.texture {
                // same size: sub-image upload (tex_sub_image_2d) instead of re-specifying the texture
                Some(t) if t.size() == [w, h] => t.set_partial([0, 0], img, TextureOptions::LINEAR),
                Some(t) => t.set(img, TextureOptions::LINEAR),
                None => state.texture = Some(ui.ctx().load_texture("preview", img, TextureOptions::LINEAR)),
            }
        }
    }
    if let Some(t) = &state.texture {
        painter.image(t.id(), lb, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
    }
    if c.fullscreen {
        // double-click toggles fullscreen, like every player
        if resp.double_clicked() {
            r.actions.push(Action::Fullscreen);
        }
        state.drag = None;
        return;
    }

    // selection overlay + drag-to-move
    let Some(clip) = c
        .selection
        .iter()
        .filter_map(|&id| c.project.clip(id))
        .find(|cl| cl.is_visual() && cl.enabled && cl.contains(c.playhead))
    else {
        state.drag = None;
        return;
    };
    let id = clip.id;
    let lt = clip.local(c.playhead);
    let to_screen =
        |x: f32, y: f32| pos2(lb.min.x + x / cw as f32 * lb.width(), lb.min.y + y / ch as f32 * lb.height());
    let stroke = Stroke::new(1.5, c.palette.selection);
    if clip.kind == ClipKind::Text {
        let p = placement(c.project, clip, c.playhead, (1, 1), cw, ch, false);
        let o = to_screen(p.cx, p.cy);
        painter.line_segment([o - vec2(6.0, 0.0), o + vec2(6.0, 0.0)], stroke);
        painter.line_segment([o - vec2(0.0, 6.0), o + vec2(0.0, 6.0)], stroke);
    } else if let Some(a) = c.project.asset(clip.asset) {
        let p = placement(c.project, clip, c.playhead, (a.width, a.height), cw, ch, true);
        let (s, co) = p.rot.to_radians().sin_cos();
        let (hw, hh) = (p.w / 2.0, p.h / 2.0);
        let pts = [(-hw, -hh), (hw, -hh), (hw, hh), (-hw, hh)]
            .iter()
            .map(|&(x, y)| to_screen(p.cx + x * co - y * s, p.cy + x * s + y * co))
            .collect();
        painter.add(Shape::closed_line(pts, stroke));
    }

    if resp.drag_started() {
        (c.undo)(c.project);
        state.drag = Some((clip.x.at(lt), clip.y.at(lt), Vec2::ZERO));
    }
    if resp.dragged() {
        if let Some((x0, y0, acc)) = &mut state.drag {
            *acc += resp.drag_delta();
            let k = c.project.width as f32 / lb.width(); // project px per point
            let (nx, ny) = (*x0 + (acc.x * k) as f64, *y0 + (acc.y * k) as f64);
            if let Some(cl) = c.project.clip_mut(id) {
                cl.x.set_at(lt, nx);
                cl.y.set_at(lt, ny);
                r.edited = true;
            }
        }
    }
    if resp.drag_stopped() {
        state.drag = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letterbox_fits_and_centres() {
        let area = Rect::from_min_size(pos2(10.0, 20.0), vec2(400.0, 300.0));
        // wide video in a 4:3 area → full width, bars top/bottom
        let lb = letterbox(area, 16.0 / 9.0, 1.0);
        assert_eq!(lb.width(), 400.0);
        assert_eq!(lb.height(), 225.0);
        assert!((lb.center() - area.center()).length() <= 0.5);
        // tall video → full height, bars left/right
        let lb = letterbox(area, 0.5, 1.0);
        assert_eq!(lb.height(), 300.0);
        assert_eq!(lb.width(), 150.0);
        assert!((lb.center() - area.center()).length() <= 0.5);
    }

    #[test]
    fn letterbox_snaps_to_pixels() {
        let area = Rect::from_min_size(pos2(0.3, 0.0), vec2(333.3, 200.0));
        let lb = letterbox(area, 16.0 / 9.0, 2.0);
        for v in [lb.min.x, lb.min.y, lb.width(), lb.height()] {
            assert!((v * 2.0 - (v * 2.0).round()).abs() < 1e-3, "{v} not pixel aligned");
        }
        assert!(lb.width() <= area.width() + 0.5 && lb.height() <= area.height() + 0.5);
    }
}
