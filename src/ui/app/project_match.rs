//! ---- ws:project-color-fps ----
//! "Match project to this video?": a small non-blocking card shown when an imported video's size,
//! frame rate or HDR-ness differs from the project's (the first video of a batch decides). Match sets
//! width, height, the snapped frame rate and, for an HDR source, the colour space as one undo step;
//! Keep closes it; "Don't ask again" turns `Settings.match_import` off (Settings ▸ General turns it back on).

use super::*;
use crate::model::{snap_fps, Asset, ColorSpace};

/// What Match would set.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MatchOffer {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) fps: f64,
    /// Some only for an HDR source.
    pub(crate) color: Option<ColorSpace>,
}

impl MatchOffer {
    /// "1920×1080 · 60 fps" (+ " · HDR10").
    pub(crate) fn summary(&self) -> String {
        let mut s = format!("{}×{} · {} fps", self.width, self.height, crate::ui::inspector::fps_label(self.fps));
        if let Some(c) = self.color {
            s.push_str(&format!(" · {}", c.short()));
        }
        s
    }
}

/// The offer for `a` (probed `color_transfer` = `trc`), or None when it is not a video or already matches.
pub(crate) fn offer_for(project: &Project, a: &Asset, trc: &str) -> Option<MatchOffer> {
    if a.kind != ClipKind::Video || a.width == 0 || a.height == 0 {
        return None;
    }
    let fps = if a.fps > 1.0 { snap_fps(a.fps) } else { project.fps };
    let color = Some(ColorSpace::of_transfer(trc)).filter(|c| c.is_hdr());
    let differs = (a.width, a.height) != (project.width, project.height)
        || (fps - project.fps).abs() > 1e-3
        || color.is_some_and(|c| c != project.color);
    differs.then_some(MatchOffer { width: a.width, height: a.height, fps, color })
}

pub(crate) fn apply(project: &mut Project, o: &MatchOffer) {
    (project.width, project.height, project.fps) = (o.width, o.height, o.fps);
    if let Some(c) = o.color {
        project.color = c;
    }
}

impl App {
    /// An import landed with `a` as its first video: ask (once per batch), or - asking turned off -
    /// adopt the format silently into an empty project, as imports always did.
    pub(super) fn offer_match(&mut self, a: &Asset) {
        if self.match_offer.is_some() {
            return;
        }
        let trc = crate::media::ffpipe::transfer_of(&a.path);
        let Some(o) = offer_for(&self.project, a, &trc) else { return };
        if self.settings.match_import {
            self.match_offer = Some(o);
        } else if self.project.is_empty() {
            apply(&mut self.project, &o); // part of the import's own undo step
        }
    }

    /// Match: one undo step.
    pub(crate) fn accept_match(&mut self) {
        let Some(o) = self.match_offer.take() else { return };
        self.push_undo();
        apply(&mut self.project, &o);
        self.after_edit();
    }
}

/// WINDOW_DRAWER: the card, bottom centre, until answered.
pub(super) fn window(app: &mut App, ctx: &egui::Context) {
    let Some(o) = app.match_offer.clone() else { return };
    let mut answer = None;
    egui::Area::new(egui::Id::new("match_project_card"))
        .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -48.0])
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.label(egui::RichText::new("Match project to this video?").strong());
                let now = format!(
                    "Project now: {}×{} · {} fps · {}",
                    app.project.width,
                    app.project.height,
                    crate::ui::inspector::fps_label(app.project.fps),
                    app.project.color.short()
                );
                ui.weak(o.summary()).on_hover_text(now);
                ui.horizontal(|ui| {
                    if ui.button("Match").on_hover_text("Set the project's size, frame rate and colour to the video's").clicked() {
                        answer = Some(1);
                    }
                    if ui.button("Keep").on_hover_text("Leave the project as it is").clicked() {
                        answer = Some(0);
                    }
                    if ui.button("Don't ask again").on_hover_text("Turn back on in Settings ▸ General").clicked() {
                        answer = Some(2);
                    }
                });
            });
        });
    match answer {
        Some(1) => app.accept_match(),
        Some(0) => app.match_offer = None,
        Some(_) => {
            app.match_offer = None;
            app.settings.match_import = false;
            app.settings.save();
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(w: u32, h: u32, fps: f64) -> Asset {
        let mut a = crate::engine::import::placeholder("C:\\clips\\a.mp4");
        (a.kind, a.width, a.height, a.fps) = (ClipKind::Video, w, h, fps);
        a
    }

    #[test]
    fn offer_only_when_something_differs_and_fps_is_snapped() {
        let mut p = Project::new();
        (p.width, p.height, p.fps) = (1920, 1080, 30.0);
        assert_eq!(offer_for(&p, &video(1920, 1080, 30.0), "bt709"), None, "already matches");
        let o = offer_for(&p, &video(1080, 1920, 26.022134821183574), "").unwrap();
        assert_eq!((o.width, o.height, o.fps, o.color), (1080, 1920, 26.022, None));
        assert_eq!(o.summary(), "1080×1920 · 26.022 fps");
        let o = offer_for(&p, &video(1920, 1080, 30.0), "smpte2084").unwrap();
        assert_eq!(o.color, Some(ColorSpace::Hdr10), "an HDR source alone is a difference");
        assert_eq!(o.summary(), "1920×1080 · 30 fps · HDR10");
        let mut audio = video(0, 0, 0.0);
        audio.kind = ClipKind::Audio;
        assert_eq!(offer_for(&p, &audio, ""), None);
    }

    /// Match = `accept_match`: one snapshot pushed, then size + rate + colour applied together, so a
    /// single undo entry brings all of it back.
    #[test]
    fn match_applies_as_one_undo_step() {
        let mut p = Project::new();
        (p.width, p.height, p.fps) = (1920, 1080, 30.0);
        let before = p.to_json();
        let o = offer_for(&p, &video(3840, 2160, 59.94005994), "arib-std-b67").unwrap();
        let (mut undo, mut redo) = (Vec::new(), Vec::new());
        super::push_undo_json(&mut undo, &mut redo, p.to_json());
        apply(&mut p, &o);
        assert_eq!((p.width, p.height, p.fps, p.color), (3840, 2160, 59.94, ColorSpace::Hlg));
        assert_eq!(undo.len(), 1);
        let back = Project::from_json(&undo[0].json).unwrap();
        assert_eq!(back.to_json(), before);
    }
}
