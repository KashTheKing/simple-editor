//! ---- ws:project-color-fps ----
//! "Match project to this video?": a small non-blocking card shown when an imported video's size,
//! frame rate or HDR-ness differs from the project's. Match sets width, height, the snapped frame rate
//! and, for an HDR source, the colour space as one undo step; Keep closes it; "Don't ask again" turns
//! `Settings.match_import` off (Settings ▸ General turns it back on).
//! ---- ws:mixed-footage ----
//! Every video of the import counts (batches that land over several polls add up while the card is open).
//! A batch with more than one format gets the mixed offer: the frame rate the others divide into evenly
//! (25 for 25 + 50; with no such rate the most common one and a warning), and 1080p or 4K as the delivery.

use super::*;
use crate::media::footage;
use crate::model::{snap_fps, Asset, ColorSpace};

/// A batch of several formats.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Mixed {
    /// Each distinct (snapped) rate and how many clips have it, in import order.
    pub(crate) rates: Vec<(f64, usize)>,
    /// Each distinct size and how many clips have it.
    pub(crate) sizes: Vec<(u32, u32, usize)>,
    /// Every rate is a whole multiple of the chosen one.
    pub(crate) even: bool,
    /// The delivery choice: 4K, else 1080p.
    pub(crate) four_k: bool,
    pub(crate) portrait: bool,
}

/// What Match would set.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MatchOffer {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) fps: f64,
    /// Some only for an HDR source.
    pub(crate) color: Option<ColorSpace>,
    /// Some for a batch of several formats.
    pub(crate) mixed: Option<Mixed>,
    /// The videos this offer was made from.
    pub(crate) batch: Vec<Asset>,
}

fn fps_label(f: f64) -> String {
    crate::ui::inspector::fps_label(f)
}

impl MatchOffer {
    /// "1920×1080 · 60 fps" (+ " · HDR10").
    pub(crate) fn summary(&self) -> String {
        let mut s = format!("{}×{} · {} fps", self.width, self.height, fps_label(self.fps));
        if let Some(c) = self.color {
            s.push_str(&format!(" · {}", c.short()));
        }
        s
    }

    /// The card's one line for a mixed batch: "Mixed footage: 1080p at 25 fps — your 50 fps clips can play
    /// as smooth half-speed slow motion".
    pub(crate) fn headline(&self) -> String {
        let Some(m) = &self.mixed else { return self.summary() };
        let res = match (m.four_k, m.portrait) {
            (true, false) => "4K",
            (true, true) => "4K vertical",
            (false, false) => "1080p",
            (false, true) => "1080p vertical",
        };
        let mut s = format!("Mixed footage: {res} at {} fps", fps_label(self.fps));
        if !m.even {
            s.push_str(" — some clips will drop or repeat frames");
            return s;
        }
        let fast: Vec<(f64, u32)> =
            m.rates.iter().filter_map(|&(r, _)| footage::rate_multiple(r, self.fps).map(|n| (r, n))).collect();
        if let [(r, n)] = fast[..] {
            let how = match n {
                2 => "half-speed".to_string(),
                4 => "quarter-speed".to_string(),
                n => format!("1/{n}-speed"),
            };
            s.push_str(&format!(" — your {} fps clips can play as smooth {how} slow motion", fps_label(r)));
        } else if !fast.is_empty() {
            let list: Vec<String> = fast.iter().map(|(r, _)| fps_label(*r)).collect();
            s.push_str(&format!(" — your {} fps clips can play as smooth slow motion", list.join(" and ")));
        }
        s
    }

    /// The tooltip: what the batch holds and why this choice.
    pub(crate) fn why(&self) -> String {
        let Some(m) = &self.mixed else { return String::new() };
        let mut s = String::from("Your clips:\n");
        for (r, n) in &m.rates {
            s.push_str(&format!("• {n} at {} fps\n", fps_label(*r)));
        }
        for (w, h, n) in &m.sizes {
            s.push_str(&format!("• {n} at {w}×{h}\n"));
        }
        s.push('\n');
        if m.even {
            s.push_str(&format!(
                "{} fps is the rate the others divide into evenly, so every clip plays whole frames: a faster clip shows every 2nd (or Nth) frame at normal speed, never a blend. \
                 Right-click a clip with a rate badge ▸ Speed ▸ Slow motion (every frame) to use all of its frames.\n",
                fps_label(self.fps)
            ));
        } else {
            s.push_str(&format!(
                "These rates don't divide evenly, so {} fps (the most common) is used and the other clips repeat or skip a frame now and then. \
                 For smoother motion pick Frame blending or Optical flow in the clip's Inspector ▸ Frame rate.\n",
                fps_label(self.fps)
            ));
        }
        if m.four_k {
            s.push_str("4K: every 4K clip is shown at full detail; smaller clips are upscaled.");
        } else {
            s.push_str("1080p: 4K clips fit the frame and can punch in up to 200 % (Inspector ▸ Zoom) with no loss. Pick 4K to deliver 4K.");
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
    differs.then(|| MatchOffer { width: a.width, height: a.height, fps, color, mixed: None, batch: vec![a.clone()] })
}

/// The offer for a whole import: one format = `offer_for`; several = the mixed offer, with the delivery
/// `four_k` (None = 4K only when every clip is 4K). None when the project already matches.
pub(crate) fn offer_for_batch(
    project: &Project,
    batch: &[Asset],
    trc: &dyn Fn(&str) -> String,
    four_k: Option<bool>,
) -> Option<MatchOffer> {
    let vids: Vec<&Asset> =
        batch.iter().filter(|a| a.kind == ClipKind::Video && a.width > 0 && a.height > 0).collect();
    let first = *vids.first()?;
    let rate = |a: &Asset| if a.fps > 1.0 { snap_fps(a.fps) } else { project.fps };
    let one_format = vids
        .iter()
        .all(|a| (a.width, a.height) == (first.width, first.height) && (rate(a) - rate(first)).abs() < 1e-3);
    if one_format {
        let mut o = offer_for(project, first, &trc(&first.path))?;
        o.batch = batch.to_vec();
        return Some(o);
    }
    let rates: Vec<f64> = vids.iter().map(|a| rate(a)).collect();
    let (fps, even) = footage::batch_fps(&rates)?;
    let mut m = Mixed { rates: Vec::new(), sizes: Vec::new(), even, four_k: false, portrait: false };
    for (a, r) in vids.iter().zip(&rates) {
        match m.rates.iter_mut().find(|(x, _)| (x - r).abs() < 1e-3) {
            Some(e) => e.1 += 1,
            None => m.rates.push((*r, 1)),
        }
        match m.sizes.iter_mut().find(|(w, h, _)| (*w, *h) == (a.width, a.height)) {
            Some(e) => e.2 += 1,
            None => m.sizes.push((a.width, a.height, 1)),
        }
    }
    m.portrait = vids.iter().filter(|a| a.height > a.width).count() * 2 > vids.len();
    m.four_k = four_k.unwrap_or_else(|| vids.iter().all(|a| a.width.min(a.height) >= 2160));
    let (long, short) = if m.four_k { (3840, 2160) } else { (1920, 1080) };
    let (width, height) = if m.portrait { (short, long) } else { (long, short) };
    let color = vids.iter().map(|a| ColorSpace::of_transfer(&trc(&a.path))).find(|c| c.is_hdr());
    let differs = (width, height) != (project.width, project.height)
        || (fps - project.fps).abs() > 1e-3
        || color.is_some_and(|c| c != project.color);
    differs.then(|| MatchOffer { width, height, fps, color, mixed: Some(m), batch: batch.to_vec() })
}

pub(crate) fn apply(project: &mut Project, o: &MatchOffer) {
    (project.width, project.height, project.fps) = (o.width, o.height, o.fps);
    if let Some(c) = o.color {
        project.color = c;
    }
}

impl App {
    /// One video landed on its own (opening a file).
    pub(super) fn offer_match(&mut self, a: &Asset) {
        self.offer_match_batch(std::slice::from_ref(a));
    }

    /// An import landed: ask about the whole batch (added to an open card's), or - asking turned off -
    /// adopt the format silently into an empty project, as imports always did.
    pub(super) fn offer_match_batch(&mut self, landed: &[Asset]) {
        let landed: Vec<Asset> = landed.iter().filter(|a| a.kind == ClipKind::Video).cloned().collect();
        if landed.is_empty() {
            return;
        }
        let open = self.match_offer.take();
        let four_k = open.as_ref().and_then(|o| o.mixed.as_ref()).map(|m| m.four_k);
        let mut batch = open.map(|o| o.batch).unwrap_or_default();
        batch.extend(landed);
        let o = offer_for_batch(&self.project, &batch, &|p| crate::media::ffpipe::transfer_of(p), four_k);
        if self.settings.match_import {
            self.match_offer = o;
        } else if let Some(o) = o.filter(|_| self.project.is_empty()) {
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

/// WINDOW_DRAWER: the card, bottom centre, until answered (and the footage cards above it).
pub(super) fn window(app: &mut App, ctx: &egui::Context) {
    super::footage_ctl::poll(app);
    let offer = app.match_offer.clone();
    if offer.is_none() && !super::footage_ctl::has_card(app) {
        return;
    }
    let mut answer = None;
    egui::Area::new(egui::Id::new("match_project_card"))
        .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -48.0])
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            super::footage_ctl::cards(app, ui);
            let Some(o) = &offer else { return };
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                if let Some(m) = &o.mixed {
                    ui.label(egui::RichText::new(o.headline()).strong()).on_hover_text(o.why());
                    ui.horizontal(|ui| {
                        if ui.button("Match").on_hover_text(format!("Set the project to {}", o.summary())).clicked() {
                            answer = Some(1);
                        }
                        for (k, label) in [(false, "1080p"), (true, "4K")] {
                            if ui.selectable_label(m.four_k == k, label).on_hover_text("Delivery size").clicked() && m.four_k != k {
                                answer = Some(if k { 4 } else { 3 });
                            }
                        }
                        if ui.button("Keep").on_hover_text("Leave the project as it is").clicked() {
                            answer = Some(0);
                        }
                        if ui.button("Don't ask again").on_hover_text("Turn back on in Settings ▸ General").clicked() {
                            answer = Some(2);
                        }
                    });
                    return;
                }
                ui.label(egui::RichText::new("Match project to this video?").strong());
                let now = format!(
                    "Project now: {}×{} · {} fps · {}",
                    app.project.width,
                    app.project.height,
                    fps_label(app.project.fps),
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
        Some(k @ (3 | 4)) => {
            if let Some(o) = app.match_offer.take() {
                app.match_offer = offer_for_batch(
                    &app.project,
                    &o.batch,
                    &|p| crate::media::ffpipe::transfer_of(p),
                    Some(k == 4),
                );
            }
        }
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

    fn batch_offer(p: &Project, vids: &[Asset], four_k: Option<bool>) -> MatchOffer {
        offer_for_batch(p, vids, &|_| String::new(), four_k).unwrap()
    }

    /// The Reddit case: A7 IV 4K 50p chorus shots and a6400 1080p 25p verse shots. The whole batch counts
    /// (not the first video): 1080p at 25 fps, and the 50p clips are offered as half-speed slow motion.
    #[test]
    fn mixed_batch_picks_the_even_rate_and_the_delivery_size() {
        let mut p = Project::new();
        (p.width, p.height, p.fps) = (1280, 720, 30.0);
        let vids = [video(3840, 2160, 50.0), video(1920, 1080, 25.0), video(3840, 2160, 50.0)];
        let o = batch_offer(&p, &vids, None);
        assert_eq!((o.width, o.height, o.fps), (1920, 1080, 25.0));
        assert_eq!(
            o.headline(),
            "Mixed footage: 1080p at 25 fps — your 50 fps clips can play as smooth half-speed slow motion"
        );
        assert!(o.why().contains("2 at 50 fps") && o.why().contains("200 %"), "{}", o.why());
        let o = batch_offer(&p, &vids, Some(true));
        assert_eq!((o.width, o.height), (3840, 2160), "4K delivery");
        // the other even mixes
        for (a, b, want) in [(24.0, 48.0, 24.0), (30.0, 60.0, 30.0), (59.94, 29.97, 29.97)] {
            let o = batch_offer(&p, &[video(1920, 1080, a), video(3840, 2160, b)], None);
            assert_eq!(o.fps, want);
            assert!(o.mixed.as_ref().unwrap().even);
        }
        // odd mixes: the most common rate and a plain warning
        for vids in [
            vec![video(1920, 1080, 24.0), video(3840, 2160, 30.0), video(3840, 2160, 30.0)],
            vec![video(1920, 1080, 25.0), video(3840, 2160, 29.97002997)],
        ] {
            let o = batch_offer(&p, &vids, None);
            assert!(!o.mixed.as_ref().unwrap().even);
            assert!(o.headline().ends_with("some clips will drop or repeat frames"), "{}", o.headline());
        }
        assert_eq!(batch_offer(&p, &[video(1920, 1080, 24.0), video(1920, 1080, 30.0), video(1920, 1080, 30.0)], None).fps, 30.0);
        assert_eq!(batch_offer(&p, &[video(1920, 1080, 25.0), video(1920, 1080, 29.97)], None).fps, 25.0);
        // every clip 4K: 4K by default; most clips vertical: a vertical frame
        let o = batch_offer(&p, &[video(3840, 2160, 50.0), video(3840, 2160, 25.0)], None);
        assert_eq!((o.width, o.height), (3840, 2160));
        let o = batch_offer(&p, &[video(1080, 1920, 60.0), video(1080, 1920, 30.0)], None);
        assert_eq!((o.width, o.height, o.fps), (1080, 1920, 30.0));
        // already that format: nothing to ask
        (p.width, p.height, p.fps) = (1920, 1080, 25.0);
        assert_eq!(offer_for_batch(&p, &vids, &|_| String::new(), None), None);
        // one format in the batch: the plain offer
        let o = batch_offer(&p, &[video(3840, 2160, 50.0), video(3840, 2160, 50.0)], None);
        assert!(o.mixed.is_none() && o.fps == 50.0);
    }
}
