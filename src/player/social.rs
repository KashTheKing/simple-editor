//! The Platform Emulator in the player (social guides over the picture, or the picture in a device): the editor's own (`ui::social`, `ui::emulator`)
//! over the player's picture. The editor's settings (platform, mode, device, profiles) are read the first
//! time either is asked for, never at startup; a guide shows once it is picked here.

use super::PlayerApp;
use crate::ui::social::{self, paint, Mode};
use eframe::egui::{self, Color32, Rect};

/// The player's guide / emulator state (not saved; the emulator's picks are in the player prefs).
#[derive(Default)]
pub struct Social {
    settings: Option<crate::settings::Settings>,
    /// A guide was picked in this player: draw the editor's current one.
    pub on: bool,
    /// The Device Emulator is up instead of the plain picture.
    pub emulator: bool,
    state: paint::State,
    emu: crate::ui::emulator::EmuState,
}

impl Social {
    fn settings(&mut self) -> &mut crate::settings::Settings {
        self.settings.get_or_insert_with(crate::settings::Settings::load)
    }

    /// Show platform `id`'s guide (`SE_PLAYER_SHOT_SETUP=guide=tiktok`), in the editor's guide mode.
    pub fn show_guide(&mut self, id: &str) {
        let s = self.settings();
        s.guide = Some(id.to_string());
        self.on = true;
    }
}

/// The right-click menu's Platform Emulator: the platform, its guides over the picture, the picture inside
/// the emulator, or off. The platform is the editor's guide platform and the emulator's.
pub fn menu(app: &mut PlayerApp, ui: &mut egui::Ui) {
    use crate::ui::menu;
    menu::sub(ui, None, "Platform Emulator", |ui| {
        let d = social::data();
        let cur = app.social.settings().guide.clone().or_else(|| Some(app.prefs.emu.platform.clone()));
        for p in &d.platforms {
            if menu::check(ui, cur.as_deref() == Some(p.id.as_str()), &p.n, "").clicked() {
                let s = app.social.settings();
                if s.guide.is_some() {
                    social::Set::Platform(Some(p.id.clone())).apply(s);
                }
                app.prefs.emu.platform = p.id.clone();
                app.prefs_dirty = true;
            }
        }
        ui.separator();
        let shown = app.social.on;
        if menu::check(ui, shown, "Show guides", "").on_hover_text("The platform's safe zones or UI over the picture (the editor's Social Guides)").clicked() {
            let id = app.prefs.emu.platform.clone();
            let s = app.social.settings();
            if !shown && s.guide.is_none() {
                social::Set::Platform(Some(id)).apply(s);
            }
            app.social.on = !shown;
            app.social.emulator = false;
        }
        let emu = app.social.emulator;
        if menu::check(ui, emu, "Open in emulator", "").on_hover_text("The picture inside a phone, tablet, browser or TV, with the platform's UI").clicked() {
            app.social.settings();
            app.social.emulator = !emu;
        }
        if menu::check(ui, !shown && !emu, "Off", "").clicked() {
            app.social.on = false;
            app.social.emulator = false;
        }
    });
}

/// The picture with a guide or inside the emulator: true when it was drawn here (the player draws nothing
/// more), false when the player draws it (a safe-zone / UI overlay then goes on with `overlay`).
pub fn picture(app: &mut PlayerApp, ui: &mut egui::Ui, full: Rect, top: f32, bottom: f32) -> bool {
    let tex = app.video.shown.map(|(t, _, uv)| (t, uv)).filter(|_| !app.engine.audio_only()).or_else(|| app.cover.as_ref().map(|c| (c.id(), Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)))));
    let aspect = app.info.as_ref().filter(|i| i.size.1 > 0).map_or(16.0 / 9.0, |i| i.size.0 as f32 / i.size.1 as f32);
    let (t, dur, playing) = (app.engine.time(), app.engine.duration(), app.engine.is_playing());
    let title = app.title();
    let area = Rect::from_min_max(full.min + egui::vec2(0.0, top), full.max - egui::vec2(0.0, bottom));
    if app.social.emulator {
        let st = &mut app.social;
        let settings = st.settings.get_or_insert_with(crate::settings::Settings::load);
        let c = crate::ui::emulator::EmuCtx {
            settings,
            prefs: &mut app.prefs.emu,
            title: &title,
            aspect,
            playhead: t,
            duration: dur,
            playing,
            buffering: false,
            cc: app.subtitle.is_some(),
            video: tex.map(|(t, _)| t),
        };
        ui.painter().rect_filled(full, 0.0, Color32::BLACK);
        // its own layer, over the player's click-to-play surface, so its buttons and taps get the clicks
        let inner = area.shrink(12.0);
        let asked = egui::Area::new(egui::Id::new("player-emulator"))
            .fixed_pos(inner.min)
            .order(egui::Order::Middle)
            .show(ui.ctx(), |ui| {
                ui.set_width(inner.width());
                ui.set_height(inner.height());
                crate::ui::emulator::show(ui, &mut st.emu, c)
            })
            .inner;
        for a in asked {
            use crate::ui::emulator::Asked;
            match a {
                Asked::Play => app.toggle_play(),
                Asked::Seek(s) => app.seek(s),
                Asked::Saved => app.prefs_dirty = true,
                // the editor's (captions of the project, a device picture, an export of the timeline, guides on its Preview)
                Asked::Captions | Asked::Screenshot | Asked::Export | Asked::PreviewGuides => {}
            }
        }
        return true;
    }
    if !app.social.on {
        return false;
    }
    let st = &mut app.social;
    let Some(settings) = st.settings.as_ref() else { return false };
    let Some(mut g) = paint::GuideView::new(settings, &title, aspect, t, dur) else { return false };
    if g.mode != Mode::Device {
        return false;
    }
    g.playing = playing;
    let s = paint::device_scene(ui.ctx(), &mut st.state, &g, area);
    let p = ui.painter_at(full);
    p.rect_filled(full, 0.0, Color32::BLACK);
    let av = st.state.avatar(ui.ctx(), &g.profile);
    paint::replay(&p, &s.under, av);
    match tex {
        Some((id, uv)) => p.image(id, s.video, uv, Color32::WHITE),
        None => p.rect_filled(s.video, 0.0, Color32::from_rgb(38, 64, 98)),
    };
    paint::replay(&p, &s.over, av);
    let tap = ui.input(|i| i.pointer.primary_clicked().then(|| i.pointer.interact_pos()).flatten()).filter(|p| area.contains(*p)).and_then(|at| st.state.click(&s, at, g.view));
    drop(g);
    match tap {
        Some(paint::Tap::Play) => app.toggle_play(),
        Some(paint::Tap::Seek(f)) => app.seek(f as f64 * dur),
        Some(paint::Tap::Set(set)) => {
            if let Some(s) = app.social.settings.as_mut() {
                set.apply(s);
            }
        }
        Some(paint::Tap::Captions) | None => {}
    }
    true
}

/// The safe-zone / UI-preview guide over the picture at `video` (the player drew it there).
pub fn overlay(app: &mut PlayerApp, ui: &egui::Ui, video: Rect, full: Rect) {
    if !app.social.on || app.social.emulator {
        return;
    }
    let aspect = video.width() / video.height().max(1.0);
    let (t, dur, title) = (app.engine.time(), app.engine.duration(), app.title());
    let st = &mut app.social;
    let Some(settings) = st.settings.as_ref() else { return };
    if let Some(mut g) = paint::GuideView::new(settings, &title, aspect, t, dur).filter(|g| g.mode != Mode::Device) {
        g.playing = app.engine.is_playing();
        paint::draw_overlay(ui, &mut st.state, &g, video, video.intersect(full));
    }
}
