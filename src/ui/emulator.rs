//! ---- ws:device-emulator ----
//! The Device Emulator pane: the project's video playing inside real devices (iPhone with its Dynamic Island
//! or notch, Pixel and Galaxy with the Android status bar and punch hole, iPad, a desktop browser, a TV) with
//! each platform's own UI round it (`ui::social`). A gallery of every device x platform pairing, each card
//! playing the video, and a large view of the one picked: turn it, open the comments, switch captions, play,
//! scrub, theatre / fullscreen on YouTube desktop. The choice is kept per project (`Settings::emulator`).
//! App-free: `show` reads an `EmuCtx` and returns what it was asked for (`Asked`), so it can be drawn headless.
//!
//! Playback never waits on it: the video is the preview's own texture (nothing is rendered for it) and each
//! card's device chrome is built once and kept until something it shows changes (the progress bar moves a
//! step, a sheet opens, the size changes), so a playing gallery rebuilds a card a few times a second at most.

use crate::settings::Settings;
use crate::ui::social::device::{self, Scene, Sheet};
use crate::ui::social::paint::{self, GuideView, State, Tap};
use crate::ui::social::{self, ViewMode};
use eframe::egui::{self, pos2, vec2, Color32, Rect, Sense, TextureId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// What the pane shows, kept per project.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Prefs {
    pub device: String,
    pub platform: String,
    /// Turned to landscape.
    pub land: bool,
    /// The gallery of every pairing (else the large view of the picked one).
    pub gallery: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs { device: "iphone".into(), platform: "tiktok".into(), land: false, gallery: false }
    }
}

/// The pane's own state (not saved).
#[derive(Default)]
pub struct EmuState {
    /// The large view's sheets, settings menu and speaker.
    pub social: State,
    /// The desktop player view in the large view (default / theatre / fullscreen).
    pub view: ViewMode,
    /// Each gallery card's last scene and the key it was built for.
    cards: HashMap<String, (u64, Scene)>,
    /// The cards' profile pictures, one decoder per network.
    avatars: HashMap<String, State>,
    /// The large view's last scene (for a test or a screenshot of what is on screen).
    pub last: Option<Scene>,
}

/// What the pane reads from the app this frame.
pub struct EmuCtx<'a> {
    pub settings: &'a Settings,
    pub prefs: &'a mut Prefs,
    pub title: &'a str,
    /// The project's aspect (w / h).
    pub aspect: f32,
    pub playhead: f64,
    pub duration: f64,
    pub playing: bool,
    pub buffering: bool,
    /// The project's subtitles are shown.
    pub cc: bool,
    /// The preview's picture (None = a stand-in colour).
    pub video: Option<TextureId>,
}

/// What the pane asks the app for.
#[derive(Clone, Debug, PartialEq)]
pub enum Asked {
    /// Play / pause, seek (seconds), captions on / off.
    Play,
    Seek(f64),
    Captions,
    /// The prefs changed: save them.
    Saved,
    /// Save a picture of the large view.
    Screenshot,
}

/// The pairings the gallery shows: every platform on an iPhone, a Pixel and a Galaxy; long form also on the
/// desktop browser and a TV.
pub fn gallery_pairs(d: &social::Data) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for p in &d.platforms {
        for dev in ["iphone", "android", "galaxy"] {
            out.push((p.id.clone(), dev.to_string()));
        }
        if p.pg == "watch" {
            out.push((p.id.clone(), "desktop".into()));
            out.push((p.id.clone(), "tv".into()));
        }
    }
    out
}

fn hash<T: std::hash::Hash>(t: &T) -> u64 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    t.hash(&mut h);
    h.finish()
}

/// Draw a scene: what is under the video, the video (or a stand-in), what is over it.
fn paint_scene(p: &egui::Painter, s: &Scene, video: Option<TextureId>, av: Option<TextureId>) {
    paint::replay(p, &s.under, av);
    match video {
        Some(t) => p.image(t, s.video, s.uv, Color32::WHITE),
        None => p.rect_filled(s.video, 0.0, Color32::from_rgb(38, 64, 98)),
    };
    paint::replay(p, &s.over, av);
}

pub fn show(ui: &mut egui::Ui, st: &mut EmuState, mut c: EmuCtx) -> Vec<Asked> {
    let mut asked = Vec::new();
    let d = social::data();
    if d.platform(&c.prefs.platform).is_none() {
        c.prefs.platform = d.platforms[0].id.clone();
    }
    let before = c.prefs.clone();
    toolbar(ui, st, &before, c.prefs, c.cc, &mut asked, &d);
    ui.add_space(4.0);
    if c.prefs.gallery {
        if let Some((p, dv)) = gallery(ui, st, &c, &d) {
            (c.prefs.platform, c.prefs.device, c.prefs.gallery) = (p, dv, false);
            asked.push(Asked::Saved);
        }
    } else {
        large(ui, st, &c, &mut asked);
    }
    asked
}

fn toolbar(ui: &mut egui::Ui, st: &mut EmuState, before: &Prefs, prefs: &mut Prefs, cc: bool, asked: &mut Vec<Asked>, d: &social::Data) {
    ui.horizontal_wrapped(|ui| {
        let devs: Vec<(&str, &str)> = d.devices.iter().map(|v| (v.id.as_str(), v.n.as_str())).collect();
        let plats: Vec<(&str, &str)> = d.platforms.iter().map(|p| (p.id.as_str(), p.n.as_str())).collect();
        crate::ui::combo(ui, "emu_device", &mut prefs.device, &devs, Some(170.0));
        crate::ui::combo(ui, "emu_platform", &mut prefs.platform, &plats, Some(170.0));
        let turns = d.device(&prefs.device).turns();
        let r = ui.add_enabled(turns, egui::Button::new(if prefs.land { "Portrait" } else { "Landscape" }));
        if r.on_hover_text("Turn the device").on_disabled_hover_text("A browser window and a TV do not turn").clicked() {
            prefs.land = !prefs.land;
        }
        let comments = st.social.want == Some(Sheet::Comments);
        if ui.selectable_label(comments, "Comments").on_hover_text("Open the platform's comments").clicked() {
            st.social.want = if comments { None } else { Some(Sheet::Comments) };
            prefs.gallery = false;
        }
        if ui.selectable_label(cc, "Captions").on_hover_text("Show or hide the project's subtitles (the player's CC button)").clicked() {
            asked.push(Asked::Captions);
        }
        if ui.selectable_label(prefs.gallery, "Gallery").on_hover_text("Every device and platform at once; click one to bring it up large").clicked() {
            prefs.gallery = !prefs.gallery;
        }
        if ui.add_enabled(!prefs.gallery, egui::Button::new("Screenshot")).on_hover_text("Save a picture of the device").clicked() {
            asked.push(Asked::Screenshot);
        }
    });
    if *prefs != *before {
        asked.push(Asked::Saved);
    }
}

/// The large view of the picked device: interactive (play, scrub, captions, comments and the other sheets,
/// the settings menu, the speaker, theatre / fullscreen).
fn large(ui: &mut egui::Ui, st: &mut EmuState, c: &EmuCtx, asked: &mut Vec<Asked>) {
    let (rect, resp) = ui.allocate_exact_size(ui.available_size_before_wrap().max(vec2(120.0, 120.0)), Sense::click());
    let Some(mut g) = GuideView::with(c.settings, &c.prefs.platform, &c.prefs.device, c.title, c.aspect, c.playhead, c.duration) else {
        return;
    };
    g.playing = c.playing;
    g.cc = c.cc;
    g.buffering = c.buffering;
    g.land = c.prefs.land;
    g.view = st.view;
    let scene = paint::device_scene_in(ui.ctx(), &mut st.social, &g, rect, "emulator");
    let av = st.social.avatar(ui.ctx(), &g.profile);
    paint_scene(&ui.painter_at(rect), &scene, c.video, av);
    if resp.clicked() {
        if let Some(p) = resp.interact_pointer_pos() {
            match st.social.click(&scene, p, st.view) {
                Some(Tap::Play) => asked.push(Asked::Play),
                Some(Tap::Seek(f)) => asked.push(Asked::Seek(f as f64 * c.duration)),
                Some(Tap::Captions) => asked.push(Asked::Captions),
                Some(Tap::Set(social::Set::View(v))) => st.view = v,
                _ => {}
            }
        }
    }
    if ui.input(|i| i.pointer.hover_pos()).is_some_and(|p| rect.contains(p) && scene.hits.iter().any(|(r, _)| r.contains(p))) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    st.last = Some(scene);
}

/// The gallery: a card per pairing, each the device playing the video, its name under it; a click opens it large.
fn gallery(ui: &mut egui::Ui, st: &mut EmuState, c: &EmuCtx, d: &social::Data) -> Option<(String, String)> {
    let pairs = gallery_pairs(d);
    let mut pick = None;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(12.0, 12.0);
            for (plat, dev) in &pairs {
                let device = d.device(dev);
                let (ow, oh) = social::frame::outer_dp(device);
                let h = 300.0f32;
                let w = (h * ow / oh).clamp(140.0, 520.0) + 16.0;
                let (cell, resp) = ui.allocate_exact_size(vec2(w, h + 36.0), Sense::click());
                if !ui.is_rect_visible(cell) {
                    continue;
                }
                let area = Rect::from_min_size(cell.min, vec2(w, h));
                let Some(mut g) = GuideView::with(c.settings, plat, dev, c.title, c.aspect, c.playhead, c.duration) else { continue };
                g.playing = c.playing;
                g.cc = c.cc;
                g.buffering = c.buffering;
                // the chrome is rebuilt only when what it shows changes: the bar moves a step, play / pause, ...
                let step = (g.progress * 200.0).round() as i32;
                let spin = if c.buffering { (ui.input(|i| i.time) * 20.0) as i64 } else { 0 };
                let key = hash(&(area.width() as i32, area.height() as i32, area.min.x as i32, area.min.y as i32, step, &g.clock, c.playing, c.cc, spin, &g.profile.id, &g.title));
                let name = format!("{plat}/{dev}");
                let fresh = st.cards.get(&name).is_none_or(|(k, _)| *k != key);
                if fresh {
                    let mut v = g.device_view(None, 0.0);
                    v.pl.t = ui.input(|i| i.time) as f32;
                    st.cards.insert(name.clone(), (key, device::build(&v, area)));
                }
                let av = st.avatars.entry(g.profile.net.clone()).or_default().avatar(ui.ctx(), &g.profile);
                if let Some((_, s)) = st.cards.get(&name) {
                    paint_scene(&ui.painter_at(area), s, c.video, av);
                }
                // platform over device, each cut to the cell's width (a narrow phone cell must not run into
                // its neighbour's caption)
                let hover = resp.hovered();
                let col = if hover { ui.visuals().strong_text_color() } else { ui.visuals().text_color() };
                for (i, line) in [g.platform().n.as_str(), device.n.as_str()].into_iter().enumerate() {
                    let mut job = egui::text::LayoutJob::single_section(
                        line.to_string(),
                        egui::TextFormat { font_id: egui::FontId::proportional(12.0), color: col, ..Default::default() },
                    );
                    job.wrap = egui::text::TextWrapping::truncate_at_width(cell.width());
                    let galley = ui.painter().layout_job(job);
                    let at = pos2(cell.center().x - galley.size().x / 2.0, area.max.y + 4.0 + i as f32 * 15.0);
                    ui.painter().galley(at, galley, col);
                }
                if hover {
                    ui.painter().rect_stroke(cell.expand(2.0), 6.0, egui::Stroke::new(1.5, ui.visuals().selection.stroke.color), egui::StrokeKind::Outside);
                }
                if resp.on_hover_text("Open it large").clicked() {
                    pick = Some((plat.clone(), dev.clone()));
                }
            }
        });
    });
    if c.buffering {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
    }
    pick
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::social::device::Hit;

    fn ctx_for<'a>(s: &'a Settings, prefs: &'a mut Prefs, cc: bool) -> EmuCtx<'a> {
        EmuCtx { settings: s, prefs, title: "My video", aspect: 9.0 / 16.0, playhead: 2.0, duration: 10.0, playing: false, buffering: false, cc, video: None }
    }

    fn run(ctx: &egui::Context, st: &mut EmuState, s: &Settings, prefs: &mut Prefs, events: Vec<egui::Event>) -> Vec<Asked> {
        let mut out = Vec::new();
        // a second between frames: the gallery's scroll bar fades in once, then everything is at rest
        let time = ctx.input(|i| i.time) + 1.0;
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1200.0, 900.0))), events, time: Some(time), ..Default::default() };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                out = show(ui, st, ctx_for(s, prefs, false));
            });
        });
        out
    }

    /// Idle, the pane (large view and gallery, every device, portrait and landscape) asks for nothing and
    /// requests no repaint: playback and buffering drive it, nothing else.
    #[test]
    fn assert_no_idle_repaint_emulator() {
        let s = Settings::default();
        for gallery in [false, true] {
            for (device, land) in [("iphone", false), ("android", true), ("desktop", false)] {
                let ctx = egui::Context::default();
                let mut st = EmuState::default();
                let mut prefs = Prefs { device: device.into(), platform: "yt_long".into(), land, gallery };
                for _ in 0..3 {
                    assert!(run(&ctx, &mut st, &s, &mut prefs, vec![]).is_empty());
                }
                assert!(!ctx.has_requested_repaint(), "Device Emulator repainted while idle ({device}, gallery {gallery})");
            }
        }
    }

    fn click(at: egui::Pos2) -> Vec<egui::Event> {
        let b = egui::PointerButton::Primary;
        vec![
            egui::Event::PointerMoved(at),
            egui::Event::PointerButton { pos: at, button: b, pressed: true, modifiers: Default::default() },
            egui::Event::PointerButton { pos: at, button: b, pressed: false, modifiers: Default::default() },
        ]
    }

    /// A click on the player's CC button asks the app to switch the project's subtitles; one on the play button
    /// to play; one on the progress bar to seek there.
    #[test]
    fn the_cc_button_switches_the_project_subtitles() {
        let s = Settings::default();
        let ctx = egui::Context::default();
        let mut st = EmuState::default();
        let mut prefs = Prefs { device: "desktop".into(), platform: "yt_long".into(), land: false, gallery: false };
        run(&ctx, &mut st, &s, &mut prefs, vec![]);
        let scene = st.last.take().expect("drawn");
        let find = |want: fn(&Hit) -> bool| scene.hits.iter().rev().find(|(_, h)| want(h)).map(|(r, _)| r.center()).expect("button");
        let cc = find(|h| *h == Hit::Captions);
        let play = find(|h| *h == Hit::Play);
        let bar = scene.hits.iter().find_map(|(r, h)| if let Hit::Seek(t) = h { Some((*r, *t)) } else { None }).unwrap();
        run(&ctx, &mut st, &s, &mut prefs, click(cc));
        assert_eq!(run(&ctx, &mut st, &s, &mut prefs, vec![]), vec![], "a click is one frame");
        let mut asked = run(&ctx, &mut st, &s, &mut prefs, click(cc));
        asked.extend(run(&ctx, &mut st, &s, &mut prefs, click(play)));
        let at = pos2(bar.1.min.x + bar.1.width() * 0.75, bar.0.center().y);
        asked.extend(run(&ctx, &mut st, &s, &mut prefs, click(at)));
        assert!(asked.contains(&Asked::Captions), "{asked:?}");
        assert!(asked.contains(&Asked::Play), "{asked:?}");
        assert!(asked.iter().any(|a| matches!(a, Asked::Seek(t) if (t - 7.5).abs() < 0.2)), "{asked:?}");
    }

    /// Every gallery pairing exists in the data; a click on a card brings it up large and is saved.
    #[test]
    fn a_gallery_card_opens_large() {
        let d = social::data();
        let pairs = gallery_pairs(&d);
        assert!(pairs.len() >= d.platforms.len() * 3);
        assert!(pairs.iter().all(|(p, v)| d.platform(p).is_some() && d.devices.iter().any(|x| &x.id == v)));
        let s = Settings::default();
        let ctx = egui::Context::default();
        let mut st = EmuState::default();
        let mut prefs = Prefs { gallery: true, ..Prefs::default() };
        run(&ctx, &mut st, &s, &mut prefs, vec![]);
        let asked = run(&ctx, &mut st, &s, &mut prefs, click(pos2(90.0, 200.0)));
        let asked2 = run(&ctx, &mut st, &s, &mut prefs, vec![]);
        assert!(asked.contains(&Asked::Saved) || asked2.contains(&Asked::Saved), "{asked:?}");
        assert!(!prefs.gallery, "the card opened large");
        assert_eq!((prefs.platform.as_str(), prefs.device.as_str()), (pairs[0].0.as_str(), pairs[0].1.as_str()));
    }
}
