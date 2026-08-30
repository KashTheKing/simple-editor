//! Aspect-ratio / platform presets for the project inspector, and the "social guide" overlay drawn
//! over the preview: each platform's UI danger zones, safe-zone outline and mock UI silhouettes,
//! all as fractions of the video rect so they fit any project resolution.

use crate::theme::Palette;
use crate::ui::tools::Glyph;
use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Shape, Stroke, StrokeKind, pos2, vec2};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Guide {
    TikTok,
    IgReels,
    YtShorts,
    IgPost,
    YouTube,
}

impl Guide {
    pub const ALL: [Guide; 5] = [Guide::TikTok, Guide::IgReels, Guide::YtShorts, Guide::IgPost, Guide::YouTube];
    pub fn name(self) -> &'static str {
        match self {
            Guide::TikTok => "TikTok",
            Guide::IgReels => "Instagram Reels",
            Guide::YtShorts => "YouTube Shorts",
            Guide::IgPost => "Instagram Post",
            Guide::YouTube => "YouTube",
        }
    }
}

/// A one-click project format in the inspector. `guide` = overlay auto-enabled when picked.
/// Save and export follow the project's width/height/fps, so a preset carries through to the output.
pub struct Preset {
    pub name: &'static str,
    pub glyph: Glyph,
    pub w: u32,
    pub h: u32,
    pub fps: f64,
    pub guide: Option<Guide>,
}

pub const PRESETS: &[Preset] = &[
    Preset { name: "General Long Form", glyph: Glyph::Landscape, w: 1920, h: 1080, fps: 60.0, guide: None },
    Preset { name: "General Short Form", glyph: Glyph::Portrait, w: 1080, h: 1920, fps: 60.0, guide: None },
    Preset { name: "Square 1:1", glyph: Glyph::Square, w: 1080, h: 1080, fps: 60.0, guide: None },
    Preset { name: "Portrait 4:5", glyph: Glyph::Portrait, w: 1080, h: 1350, fps: 60.0, guide: None },
    Preset { name: "YouTube", glyph: Glyph::PlayRect, w: 1920, h: 1080, fps: 60.0, guide: Some(Guide::YouTube) },
    Preset { name: "YouTube Shorts", glyph: Glyph::PlayRect, w: 1080, h: 1920, fps: 60.0, guide: Some(Guide::YtShorts) },
    Preset { name: "TikTok", glyph: Glyph::MusicNote, w: 1080, h: 1920, fps: 60.0, guide: Some(Guide::TikTok) },
    Preset { name: "Instagram Reels", glyph: Glyph::Camera, w: 1080, h: 1920, fps: 60.0, guide: Some(Guide::IgReels) },
    Preset { name: "Instagram Post", glyph: Glyph::GridIcon, w: 1080, h: 1350, fps: 60.0, guide: Some(Guide::IgPost) },
    Preset { name: "4K UHD", glyph: Glyph::FilmStrip, w: 3840, h: 2160, fps: 60.0, guide: None },
];

/// Quality tiers applied to the current aspect: the shorter edge becomes this many pixels
/// (1080x1920 at "720p" -> 720x1280). See `scale_to_tier`.
pub const RES_TIERS: &[(&str, u32)] =
    &[("480p", 480), ("720p", 720), ("1080p", 1080), ("1440p", 1440), ("4K", 2160)];

pub const FPS_PRESETS: &[f64] = &[23.976, 24.0, 25.0, 30.0, 50.0, 60.0];

/// Keep the aspect of `(w, h)`, set the shorter edge to `tier`, both edges rounded to even
/// (encoders want even dimensions). An aspect within 2% of a common standard (16:9, 1:1, 4:5, 21:9,
/// and their portraits) snaps to it, so a project that adopted a 1924x1080 clip still gets a clean
/// 2560x1440 instead of 2566x1440.
pub fn scale_to_tier(w: u32, h: u32, tier: u32) -> (u32, u32) {
    let (w, h) = (w.max(16) as f64, h.max(16) as f64);
    let mut aspect = w.max(h) / w.min(h); // long/short edge, >= 1
    for std in [16.0 / 9.0, 21.0 / 9.0, 5.0 / 4.0, 4.0 / 3.0, 1.0] {
        if (aspect / std - 1.0).abs() < 0.02 {
            aspect = std;
            break;
        }
    }
    let long = ((tier as f64 * aspect / 2.0).round() as u32 * 2).max(16);
    if w >= h { (long, tier) } else { (tier, long) }
}

#[derive(Clone, Copy, PartialEq)]
enum ZoneKind {
    /// Platform UI covers this — translucent red fill.
    Danger,
    /// Keep critical content inside — solid green outline.
    Safe,
    /// Secondary crop hint (e.g. IG 4:5 grid thumbnail) — dashed outline.
    Crop,
}

/// One region in platform-spec pixels on `base`; converted to fractions of the video rect at draw time.
struct Zone {
    kind: ZoneKind,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}

const fn z(kind: ZoneKind, x0: f32, y0: f32, x1: f32, y1: f32) -> Zone {
    Zone { kind, x0, y0, x1, y1 }
}
use ZoneKind::{Crop, Danger, Safe};

static TIKTOK: [Zone; 4] = [
    z(Danger, 0.0, 0.0, 1080.0, 175.0),
    z(Danger, 0.0, 1650.0, 1080.0, 1920.0),
    z(Danger, 980.0, 700.0, 1080.0, 1650.0),
    z(Safe, 0.0, 285.0, 1080.0, 1635.0), // 1080x1350 centred
];
static IG_REELS: [Zone; 5] = [
    z(Danger, 0.0, 0.0, 1080.0, 250.0),
    z(Danger, 0.0, 1500.0, 1080.0, 1920.0),
    z(Danger, 960.0, 900.0, 1080.0, 1550.0),
    z(Safe, 0.0, 250.0, 1080.0, 1520.0),
    z(Crop, 0.0, 285.0, 1080.0, 1635.0), // 4:5 profile-grid thumbnail crop
];
static YT_SHORTS: [Zone; 4] = [
    z(Danger, 0.0, 0.0, 1080.0, 150.0),
    z(Danger, 0.0, 1470.0, 1080.0, 1920.0),
    z(Danger, 930.0, 750.0, 1080.0, 1470.0),
    z(Safe, 75.0, 300.0, 1005.0, 1620.0), // 930x1320 centred
];
static IG_POST: [Zone; 3] = [
    z(Danger, 0.0, 0.0, 1080.0, 135.0),
    z(Danger, 0.0, 1215.0, 1080.0, 1350.0),
    z(Safe, 0.0, 135.0, 1080.0, 1215.0), // 1:1 grid-thumbnail crop
];
static YOUTUBE: [Zone; 4] = [
    z(Danger, 0.0, 0.0, 1920.0, 100.0),
    z(Danger, 0.0, 960.0, 1920.0, 1080.0),
    z(Danger, 1650.0, 850.0, 1920.0, 960.0),
    z(Safe, 120.0, 70.0, 1800.0, 1010.0), // 1680x940 centred
];

/// (base canvas size the pixel specs were written against, zones)
fn spec(g: Guide) -> ((f32, f32), &'static [Zone]) {
    match g {
        Guide::TikTok => ((1080.0, 1920.0), &TIKTOK),
        Guide::IgReels => ((1080.0, 1920.0), &IG_REELS),
        Guide::YtShorts => ((1080.0, 1920.0), &YT_SHORTS),
        Guide::IgPost => ((1080.0, 1350.0), &IG_POST),
        Guide::YouTube => ((1920.0, 1080.0), &YOUTUBE),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn zones_fit_their_base_and_presets_match_guides() {
        for g in Guide::ALL {
            let ((w, h), zones) = spec(g);
            assert!(zones.iter().any(|z| z.kind == ZoneKind::Safe), "{} has no safe zone", g.name());
            for z in zones {
                assert!(z.x0 < z.x1 && z.y0 < z.y1, "{} zone is inverted", g.name());
                assert!(z.x0 >= 0.0 && z.y0 >= 0.0 && z.x1 <= w && z.y1 <= h, "{} zone leaves the canvas", g.name());
            }
        }
        for &(_, tier) in RES_TIERS {
            assert_eq!(scale_to_tier(1080, 1920, tier).0, tier, "portrait shorter edge = tier");
            assert_eq!(scale_to_tier(1920, 1080, tier).1, tier, "landscape shorter edge = tier");
            let (w, h) = scale_to_tier(1080, 1350, tier);
            assert!(w % 2 == 0 && h % 2 == 0, "even dimensions");
            assert!((w as f32 / h as f32 - 0.8).abs() < 0.01, "4:5 aspect kept at {tier}");
        }
        // a near-16:9 project (adopted from an odd clip) snaps to the clean standard size
        assert_eq!(scale_to_tier(1924, 1080, 1440), (2560, 1440));
        assert_eq!(scale_to_tier(1924, 1080, 2160), (3840, 2160));
        assert_eq!(scale_to_tier(1080, 1924, 720), (720, 1280));
        for p in PRESETS {
            assert!(p.w >= 16 && p.h >= 16);
            // a preset that carries a guide must match that guide's base aspect
            if let Some(g) = p.guide {
                let ((bw, bh), _) = spec(g);
                assert!((p.w as f32 / p.h as f32 - bw / bh).abs() < 0.01, "{} aspect mismatch", p.name);
            }
        }
    }
}

const DANGER_FILL: Color32 = Color32::from_rgba_premultiplied(46, 12, 12, 48);
const DANGER_EDGE: Color32 = Color32::from_rgba_premultiplied(90, 24, 24, 100);
const SAFE: Color32 = Color32::from_rgb(80, 220, 120);
const UI: Color32 = Color32::from_rgba_premultiplied(190, 190, 190, 200);

pub fn draw_guide(p: &egui::Painter, lb: Rect, g: Guide, _pal: &Palette) {
    let ((bw, bh), zones) = spec(g);
    let to = |x: f32, y: f32| pos2(lb.min.x + x / bw * lb.width(), lb.min.y + y / bh * lb.height());
    let rect = |zone: &Zone| Rect::from_min_max(to(zone.x0, zone.y0), to(zone.x1, zone.y1));
    for zone in zones {
        let r = rect(zone);
        match zone.kind {
            ZoneKind::Danger => {
                p.rect(r, 0.0, DANGER_FILL, Stroke::new(1.0, DANGER_EDGE), StrokeKind::Inside);
            }
            ZoneKind::Safe => {
                p.rect_stroke(r, 0.0, Stroke::new(1.5, SAFE), StrokeKind::Inside);
            }
            ZoneKind::Crop => {
                for [a, b] in [[r.left_top(), r.right_top()], [r.right_top(), r.right_bottom()], [r.right_bottom(), r.left_bottom()], [r.left_bottom(), r.left_top()]] {
                    p.add(Shape::dashed_line(&[a, b], Stroke::new(1.0, SAFE.gamma_multiply(0.7)), 6.0, 5.0));
                }
            }
        }
    }
    mock_ui(p, lb, g, to);
    // guide name chip, top-left of the video
    let chip = to(0.0, 0.0) + vec2(6.0, 6.0);
    p.text(chip, Align2::LEFT_TOP, g.name(), FontId::proportional(11.0), UI);
}

/// Silhouette-grade platform UI on top of the zones: action-column icons, caption lines, scrub bar.
/// Everything is sized from `s`, one screen point per 1/1080 of the shorter base edge.
fn mock_ui(p: &egui::Painter, lb: Rect, g: Guide, to: impl Fn(f32, f32) -> Pos2) {
    let s = (lb.width().min(lb.height())) / 1080.0; // px→points on the base's short edge
    let stroke = Stroke::new(1.4 * s.max(0.5).min(2.0), UI);
    let heart = |c: Pos2, r: f32| {
        p.circle_filled(c + vec2(-r * 0.5, -r * 0.35), r * 0.55, UI);
        p.circle_filled(c + vec2(r * 0.5, -r * 0.35), r * 0.55, UI);
        p.add(Shape::convex_polygon(
            vec![c + vec2(-r, -0.15 * r), c + vec2(r, -0.15 * r), c + vec2(0.0, r)],
            UI,
            Stroke::NONE,
        ));
    };
    let bubble = |c: Pos2, r: f32| {
        p.circle_stroke(c, r, stroke);
        p.add(Shape::convex_polygon(
            vec![c + vec2(-r * 0.3, r * 0.8), c + vec2(r * 0.35, r * 0.55), c + vec2(-r * 0.15, r * 1.35)],
            UI,
            Stroke::NONE,
        ));
    };
    let share = |c: Pos2, r: f32| {
        p.line_segment([c + vec2(-r, r * 0.6), c + vec2(r * 0.4, -r * 0.4)], stroke);
        p.add(Shape::convex_polygon(
            vec![c + vec2(r, -r), c + vec2(r * 0.1, -r * 0.9), c + vec2(r * 0.9, -r * 0.1)],
            UI,
            Stroke::NONE,
        ));
    };
    let avatar = |c: Pos2, r: f32| {
        p.circle_stroke(c, r, stroke);
        p.circle_stroke(c + vec2(0.0, -r * 0.25), r * 0.35, stroke);
        // shoulders
        p.line_segment([c + vec2(-r * 0.55, r * 0.55), c + vec2(r * 0.55, r * 0.55)], stroke);
    };
    let caption_lines = |x: f32, y: f32, widths: &[f32]| {
        for (i, w) in widths.iter().enumerate() {
            let a = to(x, y + i as f32 * 55.0);
            let b = to(x + w, y + i as f32 * 55.0);
            p.line_segment([a, b], Stroke::new(4.0 * s, UI.gamma_multiply(0.7)));
        }
    };
    let action_column = |x: f32, ys: &[f32]| {
        let r = 24.0 * s;
        for (i, &y) in ys.iter().enumerate() {
            let c = to(x, y);
            match i {
                0 => avatar(c, r),
                1 => heart(c, r * 0.9),
                2 => bubble(c, r * 0.8),
                3 => share(c, r * 0.8),
                _ => {
                    // bookmark: notched ribbon
                    p.add(Shape::closed_line(
                        vec![
                            c + vec2(-r * 0.6, -r * 0.8),
                            c + vec2(r * 0.6, -r * 0.8),
                            c + vec2(r * 0.6, r * 0.8),
                            c + vec2(0.0, r * 0.25),
                            c + vec2(-r * 0.6, r * 0.8),
                        ],
                        stroke,
                    ));
                }
            }
        }
    };
    match g {
        Guide::TikTok => {
            // top: Following | For You tabs
            caption_lines(340.0, 100.0, &[140.0]);
            caption_lines(600.0, 100.0, &[140.0]);
            action_column(1030.0, &[780.0, 950.0, 1110.0, 1270.0, 1430.0]);
            // spinning vinyl
            p.circle_stroke(to(1030.0, 1570.0), 26.0 * s, stroke);
            p.circle_filled(to(1030.0, 1570.0), 8.0 * s, UI);
            // bottom: @username + caption + sound marquee
            caption_lines(40.0, 1700.0, &[220.0, 620.0, 380.0]);
        }
        Guide::IgReels => {
            caption_lines(40.0, 130.0, &[160.0]); // "Reels" header
            action_column(1020.0, &[960.0, 1110.0, 1250.0, 1390.0, 1510.0]);
            avatar(to(80.0, 1580.0), 26.0 * s);
            caption_lines(140.0, 1570.0, &[200.0]);
            caption_lines(40.0, 1680.0, &[640.0, 420.0]);
        }
        Guide::YtShorts => {
            // top-right camera / search / menu dots
            p.circle_stroke(to(800.0, 80.0), 16.0 * s, stroke);
            p.circle_stroke(to(900.0, 80.0), 16.0 * s, stroke);
            for i in 0..3 {
                p.circle_filled(to(1000.0, 55.0 + i as f32 * 25.0), 4.0 * s, UI);
            }
            action_column(1005.0, &[830.0, 990.0, 1140.0, 1290.0, 1430.0]);
            avatar(to(80.0, 1560.0), 26.0 * s);
            caption_lines(140.0, 1550.0, &[200.0]);
            caption_lines(40.0, 1660.0, &[700.0]);
        }
        Guide::IgPost => {
            // pagination dots at the bottom-centre of the safe square
            for i in 0..4 {
                p.circle_filled(to(485.0 + i as f32 * 36.0, 1180.0), 5.0 * s, UI);
            }
            // multi-slide badge, top-right of the safe square
            p.rect_stroke(
                Rect::from_center_size(to(1000.0, 195.0), vec2(40.0, 40.0) * s),
                CornerRadius::same(4),
                stroke,
                StrokeKind::Middle,
            );
        }
        Guide::YouTube => {
            let sh = lb.height() / 1080.0; // this base is landscape: scale bar geometry by height
            // title line, top-left
            caption_lines(40.0, 50.0, &[500.0]);
            // scrub bar with red progress + playhead
            let bar_y = 970.0;
            p.line_segment([to(24.0, bar_y), to(1896.0, bar_y)], Stroke::new(3.0 * sh, UI.gamma_multiply(0.5)));
            p.line_segment([to(24.0, bar_y), to(700.0, bar_y)], Stroke::new(3.0 * sh, Color32::from_rgb(230, 40, 40)));
            p.circle_filled(to(700.0, bar_y), 8.0 * sh, Color32::from_rgb(230, 40, 40));
            // controls: play, volume, time · settings, fullscreen
            let cy = 1025.0;
            p.add(Shape::convex_polygon(
                vec![to(45.0, cy - 14.0), to(45.0, cy + 14.0), to(70.0, cy)],
                UI,
                Stroke::NONE,
            ));
            p.circle_stroke(to(120.0, cy), 12.0 * sh, stroke);
            caption_lines(170.0, cy, &[130.0]);
            p.circle_stroke(to(1790.0, cy), 12.0 * sh, stroke);
            p.rect_stroke(
                Rect::from_center_size(to(1860.0, cy), vec2(26.0, 22.0) * sh),
                CornerRadius::same(2),
                stroke,
                StrokeKind::Middle,
            );
            // channel watermark box, bottom-right hover area
            p.rect_stroke(
                Rect::from_min_max(to(1680.0, 870.0), to(1890.0, 945.0)),
                CornerRadius::same(3),
                Stroke::new(1.0, UI.gamma_multiply(0.6)),
                StrokeKind::Inside,
            );
        }
    }
}
