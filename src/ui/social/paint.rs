//! The viewer side: `replay` draws a list of `Op`s with egui's painter, `GuideView` is what the preview is
//! told to show, and `State` runs the device preview (which sheet is open, how far its slide has got, the
//! avatar texture). Nothing here repaints on its own: an animation asks egui for frames only while it moves.

use super::device::{self, Hit, Scene, Sheet};
use super::profile::SocialProfile;
use super::scene::{self, Op};
use super::*;
use crate::settings::Settings;
use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Painter, Rect, Shape, Stroke, StrokeKind, TextureHandle, TextureId};
use std::borrow::Cow;

/// Everything the preview needs to draw the guide this frame (built by the app from `Settings` + the player).
pub struct GuideView<'a> {
    pub data: Arc<Data>,
    pub id: &'a str,
    pub mode: Mode,
    pub device: &'a str,
    pub worst: bool,
    pub keep: bool,
    pub profile: Cow<'a, SocialProfile>,
    pub title: String,
    pub progress: f32,
    pub clock: String,
    /// The editor is playing (the player buttons show pause). `new` leaves it false: set it from the player.
    pub playing: bool,
    /// Burnt into an export: the platform's UI only, none of the editor's marks (labels, crop dimming).
    pub export: bool,
    /// The project's aspect (w / h).
    pub aspect: f32,
    /// The desktop long-form player view.
    pub view: ViewMode,
}

/// A player's "elapsed / total" text: whole seconds, rounded down like the real players, with hours once
/// either side reaches one ("1:02:03"). The tiny bias keeps a frame time like 3.0 (stored as 2.9999...) on 0:03.
pub fn clock(t: f64, dur: f64) -> String {
    let f = |s: f64| {
        let s = (s.max(0.0) + 1e-6) as u64;
        if dur >= 3600.0 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
    };
    format!("{} / {}", f(t), f(dur))
}

/// How full a player's progress bar is at `t` of `dur` seconds (0..1; 0 for an empty timeline).
pub fn progress(t: f64, dur: f64) -> f32 {
    if dur > 0.0 { (t / dur).clamp(0.0, 1.0) as f32 } else { 0.0 }
}

impl<'a> GuideView<'a> {
    /// None when no guide is chosen (or its id is no longer in the data).
    pub fn new(s: &'a Settings, title: &str, aspect: f32, playhead: f64, duration: f64) -> Option<GuideView<'a>> {
        let id = s.guide.as_deref()?;
        let data = data();
        let net = data.platform(id)?.net.clone();
        let profile = match s.active_profile(&net) {
            Some(p) => Cow::Borrowed(p),
            None => Cow::Owned(SocialProfile::sample(&net)),
        };
        Some(GuideView {
            data,
            id,
            mode: Mode::from_id(&s.guide_mode),
            device: &s.guide_device,
            worst: s.guide_worst,
            keep: s.guides_keep_aspect,
            profile,
            title: title.to_string(),
            progress: progress(playhead, duration),
            clock: clock(playhead, duration),
            playing: false,
            export: false,
            aspect,
            view: ViewMode::from_id(&s.guide_view).unwrap_or_default(),
        })
    }
    pub fn platform(&self) -> &Platform {
        self.data.platform(self.id).expect("checked in new")
    }
    pub fn dev(&self) -> &Device {
        self.data.device(self.device)
    }
    pub fn ctx(&self) -> scene::Ctx<'_> {
        scene::Ctx { prof: &self.profile, title: &self.title, progress: self.progress, clock: &self.clock, playing: self.playing }
    }
    /// Where the platform's own aspect sits inside the video rectangle `lb`: the largest centred
    /// rectangle of that aspect, or `lb` itself when "Maintain Aspect Ratio" is off.
    pub fn frame_in(&self, lb: Rect) -> Rect {
        if self.keep { device::contain(lb, self.platform().aspect()) } else { lb }
    }
    /// The safe-zone / UI-preview ops over the video rectangle `lb`.
    pub fn overlay_ops(&self, lb: Rect) -> Vec<Op> {
        scene::overlay(&self.data, self.platform(), self.dev(), self.mode, self.worst, lb, self.frame_in(lb), &self.ctx(), !self.export)
    }
    pub fn device_view(&self, open: Option<Sheet>, e: f32) -> device::View<'_> {
        device::View {
            plat: self.platform(),
            dev: self.dev(),
            prof: &self.profile,
            title: &self.title,
            progress: self.progress,
            clock: &self.clock,
            playing: self.playing,
            pa: self.aspect,
            open,
            e,
            vm: self.view.idx(),
        }
    }
}

/// Per-viewer state of the device preview.
#[derive(Default)]
pub struct State {
    /// The sheet that is open or sliding closed.
    pub open: Option<Sheet>,
    /// What the user wants open now (None = closed).
    pub want: Option<Sheet>,
    avatar: Option<(String, TextureHandle)>,
}

impl State {
    /// The profile picture as a texture; None while it decodes (a repaint is asked for) or without one.
    pub fn avatar(&mut self, ctx: &egui::Context, p: &SocialProfile) -> Option<TextureId> {
        if p.avatar.is_empty() {
            return None;
        }
        if let Some((k, t)) = &self.avatar {
            if *k == p.avatar {
                return Some(t.id());
            }
        }
        match super::profile::avatar(p, false) {
            Some(f) => {
                let img = egui::ColorImage::from_rgba_unmultiplied([f.width as usize, f.height as usize], &f.rgba);
                let t = ctx.load_texture("social_avatar", img, egui::TextureOptions::LINEAR);
                let id = t.id();
                self.avatar = Some((p.avatar.clone(), t));
                Some(id)
            }
            None => {
                // only while the background decode runs: a picture that cannot be read asks for nothing
                if super::profile::avatar_pending(p) {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100));
                }
                None
            }
        }
    }
    /// The slide progress (0..1, eased) of the open sheet. Starts / ends the animation by `want`.
    pub fn slide(&mut self, ctx: &egui::Context, ms: u32) -> f32 {
        if self.want.is_some() {
            self.open = self.want;
        }
        let target = if self.want.is_some() { 1.0 } else { 0.0 };
        let e = ctx.animate_value_with_time(egui::Id::new("social_sheet"), target, ms.max(1) as f32 / 1000.0);
        if self.want.is_none() && e <= 0.001 {
            self.open = None;
        }
        1.0 - (1.0 - e).powi(3)
    }
    /// Apply a tap on the scene: open a sheet (a second tap on the open one closes it), close, or nothing.
    /// A tap on a player-view button asks for a setting change (a second tap on the active view goes back to Default).
    pub fn click(&mut self, scene: &Scene, p: egui::Pos2, cur: ViewMode) -> Option<Set> {
        match scene.hits.iter().rev().find(|(r, _)| r.contains(p)).map(|h| h.1) {
            Some(Hit::Open(s)) => self.want = if self.want == Some(s) { None } else { Some(s) },
            Some(Hit::Close) => self.want = None,
            Some(Hit::View(v)) => return Some(Set::View(if cur == v { ViewMode::Default } else { v })),
            _ => {}
        }
        None
    }
}

/// Build the device scene for this frame (advancing the slide) in `area`.
pub fn device_scene(ctx: &egui::Context, st: &mut State, g: &GuideView, area: Rect) -> Scene {
    let ms = st.want.or(st.open).map_or(300, |s| s.metrics(&g.platform().st).ms);
    let e = st.slide(ctx, ms);
    // the player view animates between its layouts (a linear glide over the layout index, eased by `build`)
    let vm = ctx.animate_value_with_time(egui::Id::new("social_view"), g.view.idx(), 0.28);
    let mut v = g.device_view(st.open, e);
    v.vm = vm;
    device::build(&v, area)
}

// ---------------------------------------------------------------- replay

fn cr(rad: f32, top: bool) -> CornerRadius {
    let r = rad.round().clamp(0.0, 255.0) as u8;
    if top { CornerRadius { nw: r, ne: r, sw: 0, se: 0 } } else { CornerRadius::same(r) }
}

pub fn replay(base: &Painter, ops: &[Op], avatar: Option<TextureId>) {
    let mut p = base.clone();
    for op in ops {
        match op {
            Op::Clip(r) => {
                p = match r {
                    Some(r) => base.with_clip_rect(r.intersect(base.clip_rect())),
                    None => base.clone(),
                }
            }
            Op::Rect { r, rad, top, fill, line, lc } => {
                let stroke = if *line > 0.0 { Stroke::new(*line, *lc) } else { Stroke::NONE };
                p.add(egui::epaint::RectShape::new(*r, cr(*rad, *top), *fill, stroke, StrokeKind::Middle));
            }
            Op::Circle { c, r, fill, line, lc } => {
                p.circle(*c, *r, *fill, if *line > 0.0 { Stroke::new(*line, *lc) } else { Stroke::NONE });
            }
            Op::Line { a, b, w, col } => {
                p.line_segment([*a, *b], Stroke::new(*w, *col));
            }
            Op::Poly { p: pts, fill } => {
                p.add(Shape::convex_polygon(pts.clone(), *fill, Stroke::NONE));
            }
            Op::Text { p: at, s, col, bold, t, al } => {
                let a = [Align2::LEFT_TOP, Align2::CENTER_TOP, Align2::RIGHT_TOP][(*al).min(2) as usize];
                let f = FontId::proportional(*s);
                p.text(*at, a, t, f.clone(), *col);
                if *bold {
                    p.text(*at + egui::vec2(s * 0.03 + 0.2, 0.0), a, t, f, *col);
                }
            }
            Op::Avatar { c, r } => match avatar {
                Some(tex) => {
                    let mut m = egui::Mesh::with_texture(tex);
                    let n = 40;
                    let v = |pos: egui::Pos2| egui::epaint::Vertex { pos, uv: egui::pos2((pos.x - c.x) / (2.0 * r) + 0.5, (pos.y - c.y) / (2.0 * r) + 0.5), color: Color32::WHITE };
                    m.vertices.push(v(*c));
                    for i in 0..=n {
                        let a = std::f32::consts::TAU * i as f32 / n as f32;
                        m.vertices.push(v(egui::pos2(c.x + r * a.cos(), c.y + r * a.sin())));
                        if i > 0 {
                            m.add_triangle(0, i as u32, i as u32 + 1);
                        }
                    }
                    p.add(Shape::mesh(m));
                }
                None => placeholder(&p, *c, *r),
            },
            Op::Grad { r, up, a } => {
                let (hi, lo) = (Color32::from_black_alpha(*a), Color32::TRANSPARENT);
                let (top, bot) = if *up { (lo, hi) } else { (hi, lo) };
                let mut m = egui::Mesh::default();
                for (pos, col) in [(r.left_top(), top), (r.right_top(), top), (r.right_bottom(), bot), (r.left_bottom(), bot)] {
                    m.colored_vertex(pos, col);
                }
                m.add_triangle(0, 1, 2);
                m.add_triangle(0, 2, 3);
                p.add(Shape::mesh(m));
            }
        }
    }
}

/// The circle shown without a picture: a head and shoulders.
fn placeholder(p: &Painter, c: egui::Pos2, r: f32) {
    p.circle_filled(c, r, Color32::from_rgb(92, 96, 108));
    p.circle_filled(c + egui::vec2(0.0, -r * 0.22), r * 0.32, Color32::from_rgb(190, 194, 205));
    let sh = p.with_clip_rect(Rect::from_center_size(c, egui::vec2(2.0 * r, 2.0 * r)));
    sh.circle_filled(c + egui::vec2(0.0, r * 0.95), r * 0.62, Color32::from_rgb(190, 194, 205));
}

/// Draw the safe-zone / UI overlay over the video rectangle `lb` (clipped to `clip`).
pub fn draw_overlay(ui: &egui::Ui, st: &mut State, g: &GuideView, lb: Rect, clip: Rect) {
    let ops = g.overlay_ops(lb);
    let av = st.avatar(ui.ctx(), &g.profile);
    replay(&ui.painter_at(clip), &ops, av);
}

