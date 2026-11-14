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
/// the raw candidate and no guides.
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
) -> ((f32, f32), Vec<CanvasGuide>) {
    if !enabled {
        return (cand, Vec::new());
    }
    use CanvasGuideKind::*;
    let (w, h) = (project.width.max(1) as f32, project.height.max(1) as f32);
    let mut xs = vec![(0.0, Centre), (-w / 2.0, Edge), (w / 2.0, Edge), (-w / 6.0, Third), (w / 6.0, Third)];
    let mut ys = vec![(0.0, Centre), (-h / 2.0, Edge), (h / 2.0, Edge), (-h / 6.0, Third), (h / 6.0, Third)];
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
        let snap = |cand: (f32, f32)| canvas_snap(true, &p, 1, 1.0, cand, half, 8.0);
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
        let ((x, y), g) = canvas_snap(false, &p, 1, 1.0, (4.0, -3.0), (50.0, 50.0), 8.0);
        assert_eq!((x, y), (4.0, -3.0), "raw candidate back");
        assert!(g.is_empty());
    }
}
