//! Aspect-ratio / platform presets for the project inspector and the canvas snap guides. The social
//! guides themselves (platform safe zones, UI silhouettes, device preview) are data-driven: see `ui::social`.

use crate::engine::compose::placement;
use crate::model::{Id, Project};
use crate::theme::Palette;
use crate::ui::tools::Glyph;
use eframe::egui::{self, Rect, Stroke, pos2};

// ---- ws:canvas-handles-monitor ----

/// What a canvas drag snapped onto (goals.md: "a visible snap-guide line *before* release").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanvasGuideKind {
    /// The canvas centre line.
    Centre,
    /// A canvas edge.
    Edge,
    /// A rule-of-thirds line.
    Third,
    /// Another visible clip's edge or centre.
    Clip,
    // ---- ws:parity-preview-tools ----
    /// A line of the Preview's overlay grid (`Overlay.cols` / `rows`).
    Grid,
    /// An action-safe or title-safe border (`Overlay.safe`).
    Safe,
}

// ---- ws:parity-preview-tools ----

/// The Preview's own overlay: a grid of `cols` x `rows` cells and the action-safe / title-safe
/// borders. Viewer-only (Settings.preview_overlay): never rendered, never exported. While it is shown,
/// a canvas drag also snaps onto its lines (`canvas_snap`).
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Overlay {
    /// Grid columns / rows (0 = no grid on that axis; 1 draws nothing either).
    pub cols: u32,
    pub rows: u32,
    /// Draw the action-safe and title-safe borders.
    pub safe: bool,
    /// Action-safe and title-safe areas, in % of the frame's width and height.
    pub action: f32,
    pub title: f32,
}

impl Default for Overlay {
    fn default() -> Self {
        // 90 / 80 %: the classic broadcast pair Shotcut and most editors draw
        Self { cols: 0, rows: 0, safe: false, action: 90.0, title: 80.0 }
    }
}

/// The grid presets of the Preview's Grid menu: (label, cols, rows). Square-ish cells on 16:9 are
/// the custom row's job; these are the counts editors ship.
pub const GRID_PRESETS: &[(&str, u32, u32)] =
    &[("Thirds", 3, 3), ("2 x 2", 2, 2), ("4 x 4", 4, 4), ("8 x 8", 8, 8), ("16 x 9", 16, 9), ("16 x 16", 16, 16)];

/// The largest grid count the custom fields accept (a 64-cell grid is already a texture, not a guide).
pub const GRID_MAX: u32 = 64;

impl Overlay {
    /// Is a grid drawn (on either axis)?
    pub fn grid_on(&self) -> bool {
        self.cols > 1 || self.rows > 1
    }
    /// Anything to draw or snap to?
    pub fn any(&self) -> bool {
        self.grid_on() || self.safe
    }
    /// Action- and title-safe fractions of the frame, each clamped to 10..100 %.
    fn safe_fracs(&self) -> [f32; 2] {
        [self.action, self.title].map(|p| (p / 100.0).clamp(0.1, 1.0))
    }
    /// The overlay's lines for a `w` x `h` project: (vertical x's, horizontal y's, kind) in project px
    /// relative to the canvas centre - the `Clip.x` / `Clip.y` frame `canvas_snap` works in. Grid lines
    /// exclude the frame edges (the canvas snap has those already).
    pub fn lines(&self, w: f32, h: f32) -> Vec<(bool, f32, CanvasGuideKind)> {
        let mut v = Vec::new();
        let mut grid = |n: u32, len: f32, vertical: bool| {
            for i in 1..n.min(GRID_MAX) {
                v.push((vertical, len * (i as f32 / n as f32 - 0.5), CanvasGuideKind::Grid));
            }
        };
        grid(self.cols, w, true);
        grid(self.rows, h, false);
        if self.safe {
            for f in self.safe_fracs() {
                let (hw, hh) = (w * f / 2.0, h * f / 2.0);
                v.extend([(true, -hw), (true, hw), (false, -hh), (false, hh)].map(|(vt, at)| (vt, at, CanvasGuideKind::Safe)));
            }
        }
        v
    }
}

/// Draw `ov` over the video rect `lb` (a `pw` x `ph` project): thin grid lines, the action-safe border
/// solid and the title-safe one dashed, in a light colour with a dark under-stroke so they read on
/// any picture. `clip` = the visible part of the canvas.
pub fn paint_overlay(p: &egui::Painter, ov: &Overlay, lb: Rect, (pw, ph): (u32, u32)) {
    if !ov.any() || !lb.is_positive() {
        return;
    }
    let (sx, sy) = (lb.width() / pw.max(1) as f32, lb.height() / ph.max(1) as f32);
    let c = lb.center();
    let shade = Stroke::new(2.0, egui::Color32::from_black_alpha(70));
    let line = Stroke::new(1.0, egui::Color32::from_white_alpha(110));
    let seg = |a: egui::Pos2, b: egui::Pos2| {
        p.line_segment([a, b], shade);
        p.line_segment([a, b], line);
    };
    for (vertical, at, kind) in ov.lines(pw as f32, ph as f32) {
        if kind != CanvasGuideKind::Grid {
            continue;
        }
        if vertical {
            let x = c.x + at * sx;
            seg(pos2(x, lb.top()), pos2(x, lb.bottom()));
        } else {
            let y = c.y + at * sy;
            seg(pos2(lb.left(), y), pos2(lb.right(), y));
        }
    }
    if ov.safe {
        for (i, f) in ov.safe_fracs().into_iter().enumerate() {
            let r = Rect::from_center_size(c, lb.size() * f);
            let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
            p.add(egui::Shape::line(pts.to_vec(), shade));
            if i == 0 {
                p.add(egui::Shape::line(pts.to_vec(), line));
            } else {
                p.extend(egui::Shape::dashed_line(&pts, line, 6.0, 4.0));
            }
        }
    }
}

/// One snap line the drag landed on: vertical (`at` = x) or horizontal (`at` = y), in project px
/// relative to the canvas centre - the same frame as `Clip.x` / `Clip.y`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasGuide {
    pub kind: CanvasGuideKind,
    pub vertical: bool,
    pub at: f32,
}

/// Snap a dragged clip's centre `cand` (project px relative to the canvas centre; `half` = the clip's
/// axis-aligned half size in the same units) onto the canvas centre, edges and thirds and every other
/// visible asset-backed clip's edges / centre at `playhead`. The clip's own left / centre / right (top /
/// centre / bottom) lines all count; per axis the closest target within `thr` px wins and returns its
/// guide, anything farther leaves that axis untouched. `enabled` false (Settings.canvas_snap off) =
/// the raw candidate and no guides. `overlay` = the Preview's shown grid / safe areas, whose lines
/// are snap targets too (ws:parity-preview-tools).
/// ponytail: only asset-backed clips (video / image) are snap targets - text and shape clips have no
/// native size for `placement`; add `ShapeStyle.w/h` bounds here if snapping to shapes is wanted.
pub fn canvas_snap(
    enabled: bool,
    project: &Project,
    moving: Id,
    playhead: f64,
    cand: (f32, f32),
    half: (f32, f32),
    thr: f32,
    overlay: &Overlay,
) -> ((f32, f32), Vec<CanvasGuide>) {
    if !enabled {
        return (cand, Vec::new());
    }
    use CanvasGuideKind::*;
    let (w, h) = (project.width.max(1) as f32, project.height.max(1) as f32);
    let mut xs = vec![(0.0, Centre), (-w / 2.0, Edge), (w / 2.0, Edge), (-w / 6.0, Third), (w / 6.0, Third)];
    let mut ys = vec![(0.0, Centre), (-h / 2.0, Edge), (h / 2.0, Edge), (-h / 6.0, Third), (h / 6.0, Third)];
    // ---- ws:parity-preview-tools ----
    for (vertical, at, kind) in overlay.lines(w, h) {
        if vertical { xs.push((at, kind)) } else { ys.push((at, kind)) }
    }
    for (_, c) in project.all_clips() {
        if c.id == moving || !c.is_visual() || !c.enabled || !c.contains(playhead) {
            continue;
        }
        let Some(a) = project.asset(c.asset) else { continue };
        let p = placement(project, c, playhead, (a.width, a.height), project.width, project.height, true);
        let (x0, y0, x1, y1) = p.bounds();
        xs.extend([(x0 - w / 2.0, Clip), (p.cx - w / 2.0, Clip), (x1 - w / 2.0, Clip)]);
        ys.extend([(y0 - h / 2.0, Clip), (p.cy - h / 2.0, Clip), (y1 - h / 2.0, Clip)]);
    }
    let axis = |c: f32, hs: f32, targets: &[(f32, CanvasGuideKind)]| -> (f32, Option<(f32, CanvasGuideKind)>) {
        let mut best: Option<(f32, f32, CanvasGuideKind)> = None; // (delta, target line, kind)
        for own in [c - hs, c, c + hs] {
            for &(t, k) in targets {
                let d = t - own;
                if d.abs() <= thr && best.is_none_or(|b| d.abs() < b.0.abs()) {
                    best = Some((d, t, k));
                }
            }
        }
        match best {
            Some((d, at, k)) => (c + d, Some((at, k))),
            None => (c, None),
        }
    };
    let (sx, gx) = axis(cand.0, half.0, &xs);
    let (sy, gy) = axis(cand.1, half.1, &ys);
    let mut guides = Vec::new();
    if let Some((at, kind)) = gx {
        guides.push(CanvasGuide { kind, vertical: true, at });
    }
    if let Some((at, kind)) = gy {
        guides.push(CanvasGuide { kind, vertical: false, at });
    }
    ((sx, sy), guides)
}

/// Accent guide lines across the video rect `lb` for a live canvas drag (`canvas_snap`'s output);
/// `(pw, ph)` = the project size the guides' px are relative to.
pub fn paint_canvas_guides(p: &egui::Painter, guides: &[CanvasGuide], lb: Rect, (pw, ph): (u32, u32), palette: &Palette) {
    let (sx, sy) = (lb.width() / pw.max(1) as f32, lb.height() / ph.max(1) as f32);
    let c = lb.center();
    for g in guides {
        let stroke = Stroke::new(1.0, palette.accent);
        if g.vertical {
            let x = c.x + g.at * sx;
            p.line_segment([pos2(x, lb.top()), pos2(x, lb.bottom())], stroke);
        } else {
            let y = c.y + g.at * sy;
            p.line_segment([pos2(lb.left(), y), pos2(lb.right(), y)], stroke);
        }
    }
}

/// A one-click project format in the inspector. `guide` = the social-guide platform id enabled when picked.
/// Save and export follow the project's width/height/fps, so a preset carries through to the output.
pub struct Preset {
    pub name: &'static str,
    pub glyph: Glyph,
    pub w: u32,
    pub h: u32,
    pub fps: f64,
    pub guide: Option<&'static str>,
}

pub const PRESETS: &[Preset] = &[
    Preset { name: "General Long Form", glyph: Glyph::Landscape, w: 1920, h: 1080, fps: 60.0, guide: None },
    Preset { name: "General Short Form", glyph: Glyph::Portrait, w: 1080, h: 1920, fps: 60.0, guide: None },
    Preset { name: "Square 1:1", glyph: Glyph::Square, w: 1080, h: 1080, fps: 60.0, guide: None },
    Preset { name: "Portrait 4:5", glyph: Glyph::Portrait, w: 1080, h: 1350, fps: 60.0, guide: None },
    Preset { name: "YouTube", glyph: Glyph::PlayRect, w: 1920, h: 1080, fps: 60.0, guide: Some("yt_long") },
    Preset { name: "YouTube Shorts", glyph: Glyph::PlayRect, w: 1080, h: 1920, fps: 60.0, guide: Some("yt_shorts") },
    Preset { name: "TikTok", glyph: Glyph::MusicNote, w: 1080, h: 1920, fps: 60.0, guide: Some("tiktok") },
    Preset { name: "Instagram Reels", glyph: Glyph::Camera, w: 1080, h: 1920, fps: 60.0, guide: Some("ig_reels") },
    Preset { name: "Instagram Post", glyph: Glyph::GridIcon, w: 1080, h: 1350, fps: 60.0, guide: Some("ig_feed") },
    Preset { name: "4K UHD", glyph: Glyph::FilmStrip, w: 3840, h: 2160, fps: 60.0, guide: None },
];

/// Short-form (9:16 portrait: Shorts, TikTok, Reels) projects start with this Max length.
pub const SHORT_FORM_MAX: f64 = 60.0;

/// Is `w`×`h` a 9:16 portrait (short-form) frame?
pub fn is_short_form(w: u32, h: u32) -> bool {
    h > w && (w as u64 * 16).abs_diff(h as u64 * 9) <= h as u64 / 100
}

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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tiers_scale_and_presets_match_their_guides() {
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
        let d = crate::ui::social::data();
        for p in PRESETS {
            assert!(p.w >= 16 && p.h >= 16);
            // a preset that carries a guide must name a real platform of that aspect
            if let Some(id) = p.guide {
                let g = d.platform(id).unwrap_or_else(|| panic!("{} names an unknown guide {id}", p.name));
                assert!((p.w as f32 / p.h as f32 - g.aspect()).abs() < 0.01, "{} aspect mismatch", p.name);
            }
        }
    }

    // ---- ws:canvas-handles-monitor ----

    /// A 1920x1080 project with two full-canvas video clips: 1 (the one being dragged) and 2, parked
    /// at x = 500 so its left edge sits at -460 and its right edge at 1460 (project px from centre).
    fn snap_project() -> Project {
        use crate::model::{Asset, Clip, ClipKind};
        let mut p = Project::new();
        let a = p.add_asset(Asset {
            id: 0,
            path: "C:/x.mp4".into(),
            kind: ClipKind::Video,
            duration: 4.0,
            width: 1920,
            height: 1080,
            fps: 30.0,
            audio_streams: Vec::new(),
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
        });
        for (id, x) in [(1u64, 0.0), (2, 500.0)] {
            let mut c = Clip::new(id, ClipKind::Video, "v", 0.0, 4.0);
            c.asset = a;
            c.x.value = x;
            p.tracks[0].clips.push(c);
        }
        p
    }

    #[test]
    fn canvas_snap_centre_edges_thirds_and_other_clip() {
        use CanvasGuideKind::*;
        let p = snap_project();
        let half = (50.0, 50.0);
        let snap = |cand: (f32, f32)| canvas_snap(true, &p, 1, 1.0, cand, half, 8.0, &Overlay::default());
        // centre: a candidate 4 px off the middle lands exactly on it
        let ((x, y), g) = snap((4.0, -3.0));
        assert_eq!((x, y), (0.0, 0.0));
        assert_eq!(g.len(), 2);
        assert!(g.iter().all(|g| g.kind == Centre && g.at == 0.0), "{g:?}");
        // left edge: the clip's own left side (-960 + 3) snaps onto the canvas edge at -960
        let ((x, _), g) = snap((-907.0, 0.0));
        assert_eq!(x, -910.0);
        assert!(g.iter().any(|g| g.vertical && g.kind == Edge && g.at == -960.0), "{g:?}");
        // thirds: the centre 2 px off the right third line (+320) snaps onto it
        let ((x, _), g) = snap((322.0, 300.0));
        assert_eq!(x, 320.0);
        assert!(g.iter().any(|g| g.vertical && g.kind == Third && g.at == 320.0), "{g:?}");
        // another clip: clip 2's right edge is at 1460; our left side 3 px past it snaps back onto it
        let ((x, _), g) = snap((1513.0, 300.0));
        assert_eq!(x, 1510.0);
        assert!(g.iter().any(|g| g.vertical && g.kind == Clip && g.at == 1460.0), "{g:?}");
        // far from everything: untouched, no guides
        let ((x, y), g) = snap((200.0, 200.0));
        assert_eq!((x, y), (200.0, 200.0));
        assert!(g.is_empty(), "{g:?}");
    }

    #[test]
    fn canvas_snap_off_returns_no_guides() {
        let p = snap_project();
        let ((x, y), g) = canvas_snap(false, &p, 1, 1.0, (4.0, -3.0), (50.0, 50.0), 8.0, &Overlay::default());
        assert_eq!((x, y), (4.0, -3.0), "raw candidate back");
        assert!(g.is_empty());
    }

    // ---- ws:parity-preview-tools ----

    #[test]
    fn overlay_lines_split_the_frame_and_safe_areas_are_centred() {
        let ov = Overlay { cols: 4, rows: 2, safe: true, action: 90.0, title: 80.0 };
        let l = ov.lines(1920.0, 1080.0);
        let at = |vert: bool, kind: CanvasGuideKind| -> Vec<f32> {
            l.iter().filter(|g| g.0 == vert && g.2 == kind).map(|g| g.1).collect()
        };
        // 4 columns: 3 inner lines at -480, 0, 480 (project px from centre); no frame edges
        assert_eq!(at(true, CanvasGuideKind::Grid), vec![-480.0, 0.0, 480.0]);
        assert_eq!(at(false, CanvasGuideKind::Grid), vec![0.0]);
        // action 90 %: +-864 x, +-486 y; title 80 %: +-768 x, +-432 y
        assert_eq!(at(true, CanvasGuideKind::Safe), vec![-864.0, 864.0, -768.0, 768.0]);
        assert_eq!(at(false, CanvasGuideKind::Safe), vec![-486.0, 486.0, -432.0, 432.0]);
        // off by default, 1 x 1 draws nothing, absurd counts are capped
        assert!(!Overlay::default().any());
        assert!(Overlay { cols: 1, rows: 1, ..Default::default() }.lines(100.0, 100.0).is_empty());
        let big = Overlay { cols: 10_000, ..Default::default() };
        assert_eq!(big.lines(100.0, 100.0).len(), GRID_MAX as usize - 1);
    }

    #[test]
    fn canvas_snap_lands_on_shown_grid_and_safe_lines_only() {
        use CanvasGuideKind::*;
        let p = snap_project();
        let half = (50.0, 50.0);
        let ov = Overlay { cols: 4, rows: 0, safe: true, action: 90.0, title: 80.0 };
        // the clip's centre 3 px off the 4-column line at -480 snaps onto it
        let ((x, _), g) = canvas_snap(true, &p, 1, 1.0, (-477.0, 300.0), half, 8.0, &ov);
        assert_eq!(x, -480.0);
        assert!(g.iter().any(|g| g.vertical && g.kind == Grid && g.at == -480.0), "{g:?}");
        // the clip's top edge (y - 50) 4 px below the title-safe top (-432) snaps up onto it
        let ((_, y), g) = canvas_snap(true, &p, 1, 1.0, (200.0, -378.0), half, 8.0, &ov);
        assert_eq!(y, -382.0);
        assert!(g.iter().any(|g| !g.vertical && g.kind == Safe && g.at == -432.0), "{g:?}");
        // the same candidates with the overlay hidden: no grid or safe target
        let ((x, _), g) = canvas_snap(true, &p, 1, 1.0, (-477.0, 300.0), half, 8.0, &Overlay::default());
        assert_eq!(x, -477.0);
        assert!(g.iter().all(|g| !matches!(g.kind, Grid | Safe)), "{g:?}");
    }

    #[test]
    fn overlay_settings_round_trip_and_old_files_get_the_default() {
        let mut s = crate::settings::Settings::default();
        assert_eq!(s.preview_overlay, Overlay::default());
        s.preview_overlay = Overlay { cols: 3, rows: 3, safe: true, action: 93.0, title: 90.0 };
        let back: crate::settings::Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.preview_overlay, s.preview_overlay);
        let old: crate::settings::Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(old.preview_overlay, Overlay::default());
        let partial: Overlay = serde_json::from_str(r#"{"safe": true}"#).unwrap();
        assert_eq!((partial.safe, partial.action, partial.title), (true, 90.0, 80.0));
    }
}
