//! Clip properties shared by audio and video clips — the generic per-property loop over
//! `Clip::props_mut()` (Position X/Y, Scale, Rotation, Opacity for visual clips; Volume, Pan for audio
//! clips — same widgets, same keyframe/link controls), fade in/out, blend mode, and the audio-bus
//! override. Extracted verbatim from `clip_section` in inspector.rs (see its module doc for the whole
//! inspector); zero behaviour change from the original inline code, split out only to keep
//! clip_section from growing further.
//!
//! Unlike the rest of clip_section (which edits a clip clone the caller writes back), this section
//! manages its own clone-edit-writeback + diff-propagate cycle directly against `project`, so it can
//! be dropped into inspector.rs at a single call site with a stable signature.

use crate::hotkeys::Action;
use crate::model::{AnimLink, Animated, AudioRole, BlendMode, ClipKind, Id, Project};
use crate::theme::Palette;
use crate::ui::inspector::{link_menu, luau_highlight, mark, set_pending_action};
use crate::ui::{key_buttons, Gesture};
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

/// Properties + fades + blend (bulk-editable across `ids`, diff-against-original / absolute-overwrite
/// like `transition_section`) followed by the audio-bus override (single-clip only, like the rest of
/// clip_section's "zone 2"). Returns true if anything changed.
#[allow(clippy::too_many_arguments)]
pub(super) fn section(
    ui: &mut egui::Ui,
    project: &mut Project,
    ids: &[Id],
    playhead: f64,
    palette: &Palette,
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
    let lt = clip.local(playhead);
    let mut g = Gesture::default();
    let path_list: Vec<(Id, String)> = project.paths.iter().map(|p| (p.id, p.name.clone())).collect();

    egui::Grid::new("inspector_clip_props").num_columns(2).show(ui, |ui| {
        for (label, a) in clip.props_mut() {
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
                        "Scale" | "Scale X" | "Scale Y" => {
                            ui.add(DragValue::new(&mut v).speed(0.01).range(0.01..=20.0))
                        }
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
                key_buttons(ui, a, lt, palette, &mut g);
                link_menu(ui, label, a, &path_list, &mut g);
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
        if clip.kind != ClipKind::Adjustment {
            let dur = clip.duration;
            ui.label("Fade in");
            g.note(&ui.add(DragValue::new(&mut clip.fade_in).range(0.0..=dur).speed(0.05).suffix(" s")));
            ui.end_row();
            ui.label("Fade out");
            g.note(&ui.add(DragValue::new(&mut clip.fade_out).range(0.0..=dur).speed(0.05).suffix(" s")));
            ui.end_row();
        }
        if clip.is_visual() {
            ui.label("Blend");
            egui::ComboBox::from_id_salt("blend").selected_text(clip.blend.name()).show_ui(ui, |ui| {
                for b in BlendMode::ALL {
                    g.note(&ui.selectable_value(&mut clip.blend, b, b.name()));
                }
            });
            ui.end_row();
        }
    });

    if g.start {
        undo(project);
    }
    let mut changed = false;
    if g.changed {
        changed = true;
        // props: copy each Animated back onto the project's clip by label (same set, same order)
        let edited_props: Vec<(&'static str, Animated)> =
            clip.props_mut().into_iter().map(|(l, a)| (l, a.clone())).collect();
        if let Some(c) = project.clip_mut(id) {
            c.fade_in = clip.fade_in;
            c.fade_out = clip.fade_out;
            c.blend = clip.blend;
            for (c_label, c_a) in c.props_mut() {
                if let Some((_, src)) = edited_props.iter().find(|(l, _)| *l == c_label) {
                    *c_a = src.clone();
                }
            }
        }
        // Bulk propagation: only the fields that actually changed from `orig`, absolute-overwrite of
        // the sibling's own value — mirrors clip_section's own enabled/label propagation.
        if multi {
            let mut orig_probe = orig.clone();
            let mut changed_props: Vec<(&'static str, f64)> = Vec::new();
            for ((label, a), (_, oa)) in clip.props_mut().into_iter().zip(orig_probe.props_mut()) {
                let v = a.at(lt);
                if v != oa.at(lt) {
                    changed_props.push((label, v));
                }
            }
            for &sid in ids {
                if sid == id {
                    continue;
                }
                let Some(s) = project.clip_mut(sid) else { continue };
                if clip.blend != orig.blend && s.is_visual() {
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
                        for (slabel, sa) in s.props_mut() {
                            if slabel == *label && sa.link.is_none() {
                                sa.set_at(slt, *v);
                            }
                        }
                    }
                }
            }
        }
    }

    // ---- ws:audio-dsp-automation ----
    // Essential-Sound block, right under the primary Volume/Pan/Fades and before the generic effects
    // list: Role tag, one-click Repair/Clarity chains (a visible, editable Mixer bus), a jump to that
    // bus, and the Duck/Normalize actions audio-analysis owns.
    if orig.kind == ClipKind::Audio {
        changed |= essential_sound(ui, project, ids, &orig, undo);
    }

    changed
}

// ---- ws:audio-dsp-automation ----
/// Role combo + Repair / Clarity / Open in Mixer + Duck / Normalize for the selected audio clips.
/// Repair/Clarity mutate `project` directly (bus + routing), so they snapshot `undo` themselves —
/// exactly once per click. Returns true when the project changed.
fn essential_sound(
    ui: &mut egui::Ui,
    project: &mut Project,
    ids: &[Id],
    orig: &crate::model::Clip,
    undo: &mut dyn FnMut(&Project),
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label("Role");
        let mut role = orig.audio_role;
        let mut rg = Gesture::default();
        let r = egui::ComboBox::from_id_salt("audio_role").selected_text(role.name()).width(110.0).show_ui(ui, |ui| {
            for r in AudioRole::ALL {
                rg.note(&ui.selectable_value(&mut role, r, r.name()));
            }
        });
        mark(ui, "audio_role", &r.response);
        if rg.changed {
            undo(project);
            for &id in ids {
                if let Some(c) = project.clip_mut(id) {
                    c.audio_role = role;
                }
            }
            changed = true;
        }
    });
    ui.horizontal(|ui| {
        for (label, preset, tip) in [
            ("Repair", "repair", "High-pass · De-hum · Gate · Compressor · Limiter on a new Mixer bus"),
            ("Clarity", "clarity", "Presence EQ · gentle Compressor on a new Mixer bus"),
        ] {
            let r = ui.small_button(label).on_hover_text(tip);
            mark(ui, &format!("audio_{preset}"), &r);
            if r.clicked() {
                undo(project);
                let bus = project.apply_repair(ids, preset);
                crate::ui::mixer_ui::request_focus_bus(bus);
                changed = true;
            }
        }
        let r = ui.small_button("Open in Mixer").on_hover_text("Select this clip's bus in the Mixer pane");
        mark(ui, "audio_open_mixer", &r);
        if r.clicked() {
            let track = project.track_of(orig.id).unwrap_or(0);
            let bus = project.bus_of(track, orig);
            crate::ui::mixer_ui::request_focus_bus(bus);
        }
    });
    ui.horizontal(|ui| {
        for (label, action, tip) in [
            ("Duck", Action::AutoDuck, "Duck music under dialogue (Auto-cut pane's Duck picks)"),
            ("Normalize", Action::Normalize, "Normalize the selected clips' gain"),
        ] {
            let r = ui.small_button(label).on_hover_text(tip);
            mark(ui, &format!("audio_{}", label.to_lowercase()), &r);
            if r.clicked() {
                set_pending_action(action);
            }
        }
    });
    changed
}

/// Audio bus override — per-clip data. Rendered by `clip_section` at the original position (after the
/// Path section, before Markers, inside its own `add_enabled_ui(!multi, ...)` zone 2) so pulling this
/// out of `section` above does not visibly reorder the panel.
pub(super) fn bus_section(
    ui: &mut egui::Ui,
    project: &mut Project,
    id: Id,
    kind: ClipKind,
    undo: &mut dyn FnMut(&Project),
) -> bool {
    if kind != ClipKind::Audio && kind != ClipKind::Video {
        return false;
    }
    let Some(clip) = project.clip(id) else { return false };
    ui.separator();
    let buses: Vec<(Id, String)> = project.buses.iter().map(|b| (b.id, b.name.clone())).collect();
    let mut bus = clip.bus;
    let mut gb = Gesture::default();
    ui.horizontal(|ui| {
        ui.strong("Bus");
        let name =
            buses.iter().find(|(bid, _)| *bid == bus).map(|(_, n)| n.clone()).unwrap_or_else(|| "Track default".into());
        egui::ComboBox::from_id_salt("clip_bus").selected_text(name).width(140.0).show_ui(ui, |ui| {
            gb.note(&ui.selectable_value(&mut bus, 0, "Track default"));
            for (bid, n) in &buses {
                gb.note(&ui.selectable_value(&mut bus, *bid, n));
            }
        });
    });
    if gb.changed {
        if gb.start {
            undo(project);
        }
        if let Some(c) = project.clip_mut(id) {
            c.bus = bus;
        }
        return true;
    }
    false
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

    /// Headless `section()` over one audio clip: draw frames, click a marked widget by its recorded rect.
    struct Harness {
        ctx: egui::Context,
        project: Project,
        ids: Vec<Id>,
        undos: usize,
        time: f64,
    }

    impl Harness {
        fn new() -> Self {
            let mut project = Project::new();
            let ai = project.audio_tracks()[0];
            project.tracks[ai].clips.push(crate::model::Clip::new(7, ClipKind::Audio, "a", 0.0, 4.0));
            Self { ctx: egui::Context::default(), project, ids: vec![7], undos: 0, time: 0.0 }
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
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut undo = |_: &Project| *undos += 1;
                    changed = section(ui, project, ids, 0.0, &pal, &mut undo);
                });
            });
            changed
        }
        fn click(&mut self, name: &str) -> bool {
            self.frame(vec![]);
            let r = self
                .ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("insp", name.to_string()))))
                .unwrap_or_else(|| panic!("no widget rect for {name}"));
            let pos = r.center();
            let press = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            let mut e = self.frame(vec![egui::Event::PointerMoved(pos)]);
            e |= self.frame(vec![press(true)]);
            e |= self.frame(vec![press(false)]);
            e
        }
    }

    #[test]
    fn repair_button_pushes_exactly_one_undo() {
        let mut h = Harness::new();
        assert!(!h.frame(vec![]), "drawing is not an edit");
        assert_eq!(h.undos, 0);
        assert!(h.click("audio_repair"), "Repair reports a project change");
        assert_eq!(h.undos, 1, "exactly one undo entry per Repair click");
        assert_eq!(h.project.buses.len(), 2, "Main + Repair");
        let bus = h.project.buses[1].id;
        assert_eq!(h.project.buses[1].name, "Repair");
        assert_eq!(h.project.clip(7).unwrap().bus, bus, "the clip is routed through it");
        assert_eq!(crate::ui::mixer_ui::take_focus_bus(), Some(bus), "and the Mixer is asked to select it");
        // a second click reuses the bus and still costs exactly one more undo
        assert!(h.click("audio_repair"));
        assert_eq!(h.undos, 2);
        assert_eq!(h.project.buses.len(), 2);
        // Open in Mixer only hands the bus over — no project change, no undo
        assert!(!h.click("audio_open_mixer"));
        assert_eq!(h.undos, 2);
        assert_eq!(crate::ui::mixer_ui::take_focus_bus(), Some(bus));
        // Clarity is its own bus
        assert!(h.click("audio_clarity"));
        assert_eq!(h.undos, 3);
        assert_eq!(h.project.buses.len(), 3);
        assert_eq!(h.project.clip(7).unwrap().bus, h.project.buses[2].id);
    }

    /// Duck / Normalize dispatch audio-analysis's Actions through the inspector's pending-action
    /// hand-off, and never touch the project themselves.
    #[test]
    fn duck_and_normalize_dispatch_actions() {
        let mut h = Harness::new();
        assert!(!h.click("audio_duck"));
        assert_eq!(crate::ui::inspector::take_pending_action(), Some(Action::AutoDuck));
        assert!(!h.click("audio_normalize"));
        assert_eq!(crate::ui::inspector::take_pending_action(), Some(Action::Normalize));
        assert_eq!(h.undos, 0);
        assert!(h.project.buses.is_empty());
        // the Role combo is drawn for an audio clip (its popup is exercised via the audio.role tool)
        h.frame(vec![]);
        assert!(h.ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("insp", "audio_role".to_string())))).is_some());
    }
}
