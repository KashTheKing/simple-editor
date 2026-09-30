//! The clip's property sections - `Clip::props_mut()` drawn as Transform (Position X/Y, Scale with a
//! chain toggle revealing Scale X/Y, Rotation) + "Opacity & blend" (Opacity, Blend, fades) for visual
//! clips, or one Audio section (Volume, Pan, fades, Role, Bus; right-click for Repair / Clarity / Open in
//! Mixer / Auto Duck / Normalize) for anything on an audio track. Every property row is label · value ·
//! ◆ (`ui::key_buttons`; right-click the ◆ or the value for keys and links).
//!
//! Unlike the rest of clip_section (which edits a clip clone the caller writes back), this manages its
//! own clone-edit-writeback + diff-propagate cycle directly against `project`: the property / fade /
//! blend edits bulk-apply to every selected clip (diff-against-original, absolute overwrite, like
//! `transition_section`).

use crate::hotkeys::Action;
use crate::model::{AnimLink, Animated, AudioRole, BlendMode, Clip, ClipKind, Id, Project, TrackKind};
use crate::theme::Palette;
use crate::ui::inspector::{fold, luau_highlight, mark, set_pending_action, Folds};
use crate::ui::{key_buttons, key_menu, menu, Gesture};
use eframe::egui::{self, DragValue, Slider};

pub(super) fn gain_to_db(g: f64) -> f64 {
    if g <= 0.0 {
        -60.0
    } else {
        (20.0 * g.log10()).max(-60.0)
    }
}

pub(super) fn db_to_gain(db: f64) -> f64 {
    if db <= -60.0 {
        0.0
    } else {
        10f64.powf(db / 20.0)
    }
}

/// True for a clip on an audio track - which also covers a nested sequence's audio twin, a Sequence
/// clip that `Clip::is_visual()` (by kind) would call visual. Only audio tracks sound (`engine::mixer`)
/// and only video tracks draw, so the inspector sorts sections by this, not by kind.
pub(super) fn on_audio_track(project: &Project, id: Id) -> bool {
    project.track_of(id).is_some_and(|t| project.tracks[t].kind == TrackKind::Audio)
}

/// `Clip::props_mut`, except anything on an audio track gets Volume/Pan (see `on_audio_track`).
fn props(c: &mut Clip, audio: bool) -> Vec<(&'static str, &mut Animated)> {
    if audio {
        vec![("Volume", &mut c.volume), ("Pan", &mut c.pan)]
    } else {
        c.props_mut()
    }
}

/// One grid row: label · value · ◆ (+ the Scale row's chain toggle), and the expression editor under it
/// while an expression drives the property. Right-clicking the value opens the ◆'s menu too.
#[allow(clippy::too_many_arguments)]
fn prop_row(
    ui: &mut egui::Ui,
    label: &'static str,
    a: &mut Animated,
    lt: f64,
    palette: &Palette,
    g: &mut Gesture,
    paths: &[(Id, String)],
    chain: Option<&mut bool>,
) {
    ui.label(label);
    let linked = !a.link.is_none();
    ui.horizontal(|ui| {
        let mut v = a.at(lt);
        let r = ui
            .add_enabled_ui(!linked, |ui| match label {
                "Volume" => {
                    let mut db = gain_to_db(v);
                    let r = ui.add(Slider::new(&mut db, -60.0..=12.0).suffix(" dB").fixed_decimals(1));
                    if r.changed() {
                        v = db_to_gain(db);
                    }
                    r
                }
                "Pan" => {
                    let r = ui.add(Slider::new(&mut v, -1.0..=1.0).fixed_decimals(2));
                    #[cfg(test)]
                    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("test_pan_slider"), r.id));
                    r
                }
                "Scale" | "Scale X" | "Scale Y" => ui.add(DragValue::new(&mut v).speed(0.01).range(0.01..=20.0)),
                "Opacity" => {
                    let mut pct = v * 100.0;
                    let r = ui.add(Slider::new(&mut pct, 0.0..=100.0).suffix(" %").fixed_decimals(0));
                    if r.changed() {
                        v = pct / 100.0;
                    }
                    #[cfg(test)]
                    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("test_opacity_slider"), r.id));
                    r
                }
                _ => ui.add(DragValue::new(&mut v).speed(1.0)),
            })
            .inner;
        if r.changed() {
            a.set_at(lt, v);
        }
        g.note(&r);
        #[cfg(test)]
        mark(ui, &format!("val_{label}"), &r);
        r.context_menu(|ui| key_menu(ui, a, lt, g, label, paths));
        let _kf = key_buttons(ui, a, lt, palette, g, label, paths);
        #[cfg(test)]
        mark(ui, &format!("kf_{label}"), &_kf);
        if let Some(on) = chain {
            let tip = if *on { "Hide Scale X / Y" } else { "Scale X and Y separately" };
            let id = ui.id().with("scale_xy");
            let r = crate::ui::tools::icon_button(ui, palette, id, crate::ui::tools::Glyph::Chain, tip, *on);
            mark(ui, "scale_xy", &r);
            if r.clicked() {
                *on = !*on;
            }
        }
    });
    ui.end_row();
    // a linked expression is edited right under its property
    if let Animated { link: AnimLink::Expr(src), link_err, .. } = a {
        ui.label("");
        ui.horizontal(|ui| {
            let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
                let mut job = luau_highlight(ui, buf.as_str());
                job.wrap.max_width = wrap_width;
                ui.fonts_mut(|f| f.layout_job(job))
            };
            g.note(
                &ui.add(
                    egui::TextEdit::singleline(src)
                        .desired_width(150.0)
                        .hint_text("return value + math.sin(t*4) * 20")
                        .layouter(&mut layouter),
                ),
            );
            if let Some(e) = link_err {
                ui.colored_label(ui.visuals().warn_fg_color, "!").on_hover_text(e.clone());
            }
        });
        ui.end_row();
    }
}

/// The Audio section's right-click verbs that act on the project (run after the section has drawn).
#[derive(Clone, Copy)]
enum AudioCmd {
    Repair(&'static str),
    OpenMixer,
}

/// Transform + "Opacity & blend" (visual clips) or Audio (anything on an audio track), bulk-editable
/// across `ids`. Returns true if anything changed.
pub(super) fn section(
    ui: &mut egui::Ui,
    project: &mut Project,
    ids: &[Id],
    playhead: f64,
    palette: &Palette,
    folds: &mut Folds,
    undo: &mut dyn FnMut(&Project),
) -> bool {
    let Some(&id) = ids.first() else {
        return false;
    };
    let Some(orig) = project.clip(id).cloned() else {
        return false;
    };
    let mut clip = orig.clone();
    let multi = ids.len() > 1;
    let audio = on_audio_track(project, id);
    let lt = clip.local(playhead);
    let mut g = Gesture::default();
    let path_list: Vec<(Id, String)> = project.paths.iter().map(|p| (p.id, p.name.clone())).collect();
    let mut changed = false;

    if audio {
        // ---- ws:audio-dsp-automation: Essential Sound verbs, now on the section's right-click ----
        let mut cmd: Option<AudioCmd> = None;
        let mut act: Option<Action> = None;
        let mut menu_fn = |ui: &mut egui::Ui| {
            for (label, preset, tip) in [
                ("Repair", "repair", "High-pass · De-hum · Gate · Compressor · Limiter on a new Mixer bus"),
                ("Clarity", "clarity", "Presence EQ · gentle Compressor on a new Mixer bus"),
            ] {
                if menu::row(ui, Some(crate::ui::tools::Glyph::Wrench), label, "").on_hover_text(tip).clicked() {
                    cmd = Some(AudioCmd::Repair(preset));
                }
            }
            if menu::row(ui, Some(crate::ui::tools::Glyph::Sliders), "Open in Mixer", "").clicked() {
                cmd = Some(AudioCmd::OpenMixer);
            }
            ui.separator();
            for a in [Action::AutoDuck, Action::Normalize] {
                if menu::item(ui, a, true) {
                    act = Some(a);
                }
            }
        };
        fold(ui, folds, "audio", "Audio", Some(&mut menu_fn), |ui| {
            egui::Grid::new("inspector_clip_audio").num_columns(2).show(ui, |ui| {
                for (label, a) in props(&mut clip, true) {
                    prop_row(ui, label, a, lt, palette, &mut g, &path_list, None);
                }
                fades(ui, &mut clip, &mut g);
                // Role + Bus write straight to the project, with their own undo
                changed |= role_and_bus(ui, project, ids, &orig, undo);
            });
        });
        match cmd {
            Some(AudioCmd::Repair(preset)) => {
                undo(project);
                let bus = project.apply_repair(ids, preset);
                crate::ui::mixer_ui::request_focus_bus(bus);
                changed = true;
            }
            Some(AudioCmd::OpenMixer) => {
                let track = project.track_of(orig.id).unwrap_or(0);
                crate::ui::mixer_ui::request_focus_bus(project.bus_of(track, &orig));
            }
            None => {}
        }
        if let Some(a) = act {
            set_pending_action(a); // audio-analysis's Actions, through the inspector's hand-off
        }
    } else {
        let xy_id = egui::Id::new("insp_scale_xy");
        let mut show_xy: bool = ui.ctx().data(|d| d.get_temp(xy_id).unwrap_or(false));
        fold(ui, folds, "transform", "Transform", None, |ui| {
            egui::Grid::new("inspector_clip_transform").num_columns(2).show(ui, |ui| {
                for (label, a) in props(&mut clip, false) {
                    match label {
                        "Opacity" => {}
                        "Scale X" | "Scale Y" if !show_xy => {}
                        "Scale" => prop_row(ui, label, a, lt, palette, &mut g, &path_list, Some(&mut show_xy)),
                        _ => prop_row(ui, label, a, lt, palette, &mut g, &path_list, None),
                    }
                }
            });
        });
        ui.ctx().data_mut(|d| d.insert_temp(xy_id, show_xy));
        fold(ui, folds, "opacity", "Opacity & blend", None, |ui| {
            egui::Grid::new("inspector_clip_opacity").num_columns(2).show(ui, |ui| {
                if let Some((label, a)) = props(&mut clip, false).into_iter().find(|(l, _)| *l == "Opacity") {
                    prop_row(ui, label, a, lt, palette, &mut g, &path_list, None);
                }
                ui.label("Blend");
                egui::ComboBox::from_id_salt("blend").selected_text(clip.blend.name()).show_ui(ui, |ui| {
                    for b in BlendMode::ALL {
                        g.note(&ui.selectable_value(&mut clip.blend, b, b.name()));
                    }
                });
                ui.end_row();
                fades(ui, &mut clip, &mut g);
            });
        });
    }

    if g.start {
        undo(project);
    }
    if g.changed {
        changed = true;
        // props: copy each Animated back onto the project's clip by label (same set, same order)
        let edited_props: Vec<(&'static str, Animated)> =
            props(&mut clip, audio).into_iter().map(|(l, a)| (l, a.clone())).collect();
        if let Some(c) = project.clip_mut(id) {
            c.fade_in = clip.fade_in;
            c.fade_out = clip.fade_out;
            c.blend = clip.blend;
            for (c_label, c_a) in props(c, audio) {
                if let Some((_, src)) = edited_props.iter().find(|(l, _)| *l == c_label) {
                    *c_a = src.clone();
                }
            }
        }
        // Bulk propagation: only the fields that actually changed from `orig`, absolute-overwrite of
        // the sibling's own value - mirrors clip_section's own enabled/label propagation.
        if multi {
            let mut orig_probe = orig.clone();
            let mut changed_props: Vec<(&'static str, f64)> = Vec::new();
            for ((label, a), (_, oa)) in props(&mut clip, audio).into_iter().zip(props(&mut orig_probe, audio)) {
                let v = a.at(lt);
                if v != oa.at(lt) {
                    changed_props.push((label, v));
                }
            }
            for &sid in ids {
                if sid == id {
                    continue;
                }
                let s_audio = on_audio_track(project, sid);
                let Some(s) = project.clip_mut(sid) else { continue };
                if clip.blend != orig.blend && s.is_visual() && !s_audio {
                    s.blend = clip.blend;
                }
                if clip.fade_in != orig.fade_in {
                    s.fade_in = clip.fade_in.clamp(0.0, s.duration);
                }
                if clip.fade_out != orig.fade_out {
                    s.fade_out = clip.fade_out.clamp(0.0, s.duration);
                }
                if !changed_props.is_empty() {
                    let slt = s.local(playhead).clamp(0.0, s.duration);
                    for (label, v) in &changed_props {
                        for (slabel, sa) in props(s, s_audio) {
                            if slabel == *label && sa.link.is_none() {
                                sa.set_at(slt, *v);
                            }
                        }
                    }
                }
            }
        }
    }
    changed
}

/// Fade in / out rows (none on an Adjustment layer, which has nothing of its own to fade).
fn fades(ui: &mut egui::Ui, clip: &mut Clip, g: &mut Gesture) {
    if clip.kind == ClipKind::Adjustment {
        return;
    }
    let dur = clip.duration;
    ui.label("Fade in");
    g.note(&ui.add(DragValue::new(&mut clip.fade_in).range(0.0..=dur).speed(0.05).suffix(" s")));
    ui.end_row();
    ui.label("Fade out");
    g.note(&ui.add(DragValue::new(&mut clip.fade_out).range(0.0..=dur).speed(0.05).suffix(" s")));
    ui.end_row();
}

// ---- ws:audio-dsp-automation ----
/// Role (every selected clip) + the per-clip Bus override, as two rows of the Audio grid. Both write
/// straight to `project` and snapshot `undo` themselves, exactly once per pick. True when it changed.
fn role_and_bus(
    ui: &mut egui::Ui,
    project: &mut Project,
    ids: &[Id],
    orig: &Clip,
    undo: &mut dyn FnMut(&Project),
) -> bool {
    let buses: Vec<(Id, String)> = project.buses.iter().map(|b| (b.id, b.name.clone())).collect();
    let (mut role, mut bus) = (orig.audio_role, orig.bus);
    let (mut rg, mut gb) = (Gesture::default(), Gesture::default());
    {
        ui.label("Role");
        let r = egui::ComboBox::from_id_salt("audio_role").selected_text(role.name()).width(120.0).show_ui(ui, |ui| {
            for r in AudioRole::ALL {
                rg.note(&ui.selectable_value(&mut role, r, r.name()));
            }
        });
        mark(ui, "audio_role", &r.response);
        ui.end_row();
        // the bus override is per-clip data: greyed like the rest of "zone 2" for a multi-selection
        ui.label("Bus");
        ui.add_enabled_ui(ids.len() == 1, |ui| {
            let name =
                buses.iter().find(|(b, _)| *b == bus).map(|(_, n)| n.clone()).unwrap_or_else(|| "Track default".into());
            egui::ComboBox::from_id_salt("clip_bus").selected_text(name).width(120.0).show_ui(ui, |ui| {
                gb.note(&ui.selectable_value(&mut bus, 0, "Track default"));
                for (b, n) in &buses {
                    gb.note(&ui.selectable_value(&mut bus, *b, n));
                }
            });
        });
        ui.end_row();
    }
    if rg.changed {
        undo(project);
        for &id in ids {
            if let Some(c) = project.clip_mut(id) {
                c.audio_role = role;
            }
        }
    }
    if gb.changed {
        undo(project);
        if let Some(c) = project.clip_mut(orig.id) {
            c.bus = bus;
        }
    }
    rg.changed || gb.changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_gain_roundtrip() {
        assert_eq!(gain_to_db(1.0), 0.0);
        assert!((db_to_gain(6.0) - 1.9953).abs() < 1e-3);
        assert_eq!(db_to_gain(-60.0), 0.0);
        assert_eq!(gain_to_db(0.0), -60.0);
        for db in [-59.0, -20.0, -3.0, 0.0, 6.0, 12.0] {
            assert!((gain_to_db(db_to_gain(db)) - db).abs() < 1e-9, "{db}");
        }
        for g in [0.002, 0.1, 0.5, 1.0, 2.0, 3.98] {
            assert!((db_to_gain(gain_to_db(g)) - g).abs() < 1e-9, "{g}");
        }
    }

    // ---- ws:audio-dsp-automation ----

    /// Headless `section()` over one audio clip: draw frames, click a marked widget by its recorded rect,
    /// or pick a row of the Audio section's right-click menu.
    struct Harness {
        ctx: egui::Context,
        project: Project,
        ids: Vec<Id>,
        undos: usize,
        time: f64,
        shapes: Vec<egui::epaint::ClippedShape>,
    }

    impl Harness {
        fn new() -> Self {
            let mut project = Project::new();
            let ai = project.audio_tracks()[0];
            project.tracks[ai].clips.push(crate::model::Clip::new(7, ClipKind::Audio, "a", 0.0, 4.0));
            let ctx = egui::Context::default();
            ctx.set_fonts(crate::theme::test_fonts());
            Self { ctx, project, ids: vec![7], undos: 0, time: 0.0, shapes: Vec::new() }
        }
        fn frame(&mut self, events: Vec<egui::Event>) -> bool {
            self.time += 0.05;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 800.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let pal = Palette::new(true, egui::Color32::WHITE);
            let Harness { ctx, project, ids, undos, .. } = self;
            let mut changed = false;
            let mut map = std::collections::BTreeMap::new();
            let out = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut undo = |_: &Project| *undos += 1;
                    let mut folds = Folds { primary: "audio", map: &mut map };
                    changed = section(ui, project, ids, 0.0, &pal, &mut folds, &mut undo);
                });
            });
            self.shapes = out.shapes;
            changed
        }
        fn rect(&self, name: &str) -> egui::Rect {
            self.ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("insp", name.to_string()))))
                .unwrap_or_else(|| panic!("no widget rect for {name}"))
        }
        fn press(&mut self, pos: egui::Pos2, button: egui::PointerButton) -> bool {
            let mut e = self.frame(vec![egui::Event::PointerMoved(pos)]);
            for pressed in [true, false] {
                e |= self.frame(vec![egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }]);
            }
            e
        }
        /// Right-click the Audio section's title and click its `row`.
        fn menu(&mut self, row: &str) -> bool {
            self.frame(vec![]);
            let title = self.rect("sec_audio*").left_center() + egui::vec2(10.0, 0.0);
            let mut e = self.press(title, egui::PointerButton::Secondary);
            e |= self.frame(vec![]);
            let at = self
                .shapes
                .iter()
                .rev()
                .find_map(|c| match &c.shape {
                    egui::epaint::Shape::Text(t) if t.galley.text() == row => Some(t.visual_bounding_rect().center()),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("no menu row '{row}'"));
            self.time += 1.0;
            e | self.press(at, egui::PointerButton::Primary)
        }
    }

    #[test]
    fn repair_button_pushes_exactly_one_undo() {
        let mut h = Harness::new();
        assert!(!h.frame(vec![]), "drawing is not an edit");
        assert_eq!(h.undos, 0);
        assert!(h.menu("Repair"), "Repair reports a project change");
        assert_eq!(h.undos, 1, "exactly one undo entry per Repair click");
        assert_eq!(h.project.buses.len(), 2, "Main + Repair");
        let bus = h.project.buses[1].id;
        assert_eq!(h.project.buses[1].name, "Repair");
        assert_eq!(h.project.clip(7).unwrap().bus, bus, "the clip is routed through it");
        assert_eq!(crate::ui::mixer_ui::take_focus_bus(), Some(bus), "and the Mixer is asked to select it");
        // a second click reuses the bus and still costs exactly one more undo
        assert!(h.menu("Repair"));
        assert_eq!(h.undos, 2);
        assert_eq!(h.project.buses.len(), 2);
        // Open in Mixer only hands the bus over - no project change, no undo
        assert!(!h.menu("Open in Mixer"));
        assert_eq!(h.undos, 2);
        assert_eq!(crate::ui::mixer_ui::take_focus_bus(), Some(bus));
        // Clarity is its own bus
        assert!(h.menu("Clarity"));
        assert_eq!(h.undos, 3);
        assert_eq!(h.project.buses.len(), 3);
        assert_eq!(h.project.clip(7).unwrap().bus, h.project.buses[2].id);
    }

    /// A nested sequence's audio twin is a Sequence clip (visual by kind) on an audio track: it gets the
    /// Volume/Pan controls, not Position/Scale/Opacity.
    #[test]
    fn sequence_audio_twin_gets_volume_and_pan() {
        let mut h = Harness::new();
        let ai = h.project.audio_tracks()[0];
        h.project.tracks[ai].clips[0].kind = ClipKind::Sequence;
        h.frame(vec![]);
        assert!(h.ctx.data(|d| d.get_temp::<egui::Id>(egui::Id::new("test_pan_slider"))).is_some());
        assert!(h.ctx.data(|d| d.get_temp::<egui::Id>(egui::Id::new("test_opacity_slider"))).is_none());
    }

    /// Duck / Normalize (the Audio section's right-click) dispatch audio-analysis's Actions through the
    /// inspector's pending-action hand-off, and never touch the project themselves.
    #[test]
    fn duck_and_normalize_dispatch_actions() {
        let mut h = Harness::new();
        assert!(!h.menu(Action::AutoDuck.label()));
        assert_eq!(crate::ui::inspector::take_pending_action(), Some(Action::AutoDuck));
        assert!(!h.menu(Action::Normalize.label()));
        assert_eq!(crate::ui::inspector::take_pending_action(), Some(Action::Normalize));
        assert_eq!(h.undos, 0);
        assert!(h.project.buses.is_empty());
        // the Role combo is drawn for an audio clip (its popup is exercised via the audio.role tool)
        h.frame(vec![]);
        assert!(h.ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("insp", "audio_role".to_string())))).is_some());
    }
}
