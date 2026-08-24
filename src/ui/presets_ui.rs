//! Presets pane: everything reusable that lives on the machine instead of in the project — saved effect
//! chains, node graphs, adjustment layers and clip templates. All of it is in settings.json, so it
//! survives a new project. Clicking a row applies it to the selection (templates place at the playhead);
//! its context menu renames and deletes. "Save from selection" captures the selected clip's node graph
//! or effect stack, and falls back to a clip template for adjustment layers and multi-clip selections.
//!
//! Effects and node graphs share `Settings::effect_presets` (the JSON shape tells them apart —
//! `EffectPreset::is_graph`), adjustment layers and templates share `Settings::templates`.

use crate::settings::Settings;
use crate::ui::library::{confirm, inline_edit};
use eframe::egui;

#[cfg(test)]
use crate::ui::effects_ui::test_rects;

#[derive(Default)]
pub struct PresetsState {
    /// Row being renamed: (lives in `templates`, index into that Vec, edit buffer).
    rename: Option<(bool, usize, String)>,
    /// The "Save from selection" name field.
    name: String,
}

#[derive(Default)]
pub struct PresetsResponse {
    /// Index into `Settings::effect_presets` to apply to the selected clips.
    pub apply: Option<usize>,
    /// Template names to place at the playhead.
    pub place: Vec<String>,
    /// "Save from selection" was pressed with this (trimmed, non-empty) name.
    pub save: Option<String>,
    pub settings_changed: bool,
}

/// Rename / delete, deferred until the rows are drawn (they hold `settings` borrowed).
enum Act {
    Rename(bool, usize, String),
    Delete(bool, usize),
}

/// (index into `effect_presets`, name) for one of the two flavours.
fn effect_rows(settings: &Settings, graph: bool) -> Vec<(usize, String)> {
    settings
        .effect_presets
        .iter()
        .enumerate()
        .filter(|(_, p)| p.is_graph() == graph)
        .map(|(i, p)| (i, p.name.clone()))
        .collect()
}

pub fn show(
    ui: &mut egui::Ui,
    state: &mut PresetsState,
    settings: &mut Settings,
    has_selection: bool,
) -> PresetsResponse {
    #[cfg(test)]
    test_rects::clear();
    let mut resp = PresetsResponse::default();
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut state.name).desired_width(140.0).hint_text("Preset name"));
        let name = state.name.trim().to_string();
        let b = ui.add_enabled(has_selection && !name.is_empty(), egui::Button::new("Save from selection"));
        #[cfg(test)]
        test_rects::push("save".into(), b.rect);
        let tip = "The selected clip's node graph or effect stack — or the whole selection as a template";
        if b.on_hover_text(tip).clicked() {
            resp.save = Some(name);
            state.name.clear();
        }
    });
    ui.separator();

    // rows are read out before any of them can mutate `settings`
    let (fx, graphs) = (effect_rows(settings, false), effect_rows(settings, true));
    let (mut adj, mut cont, mut tpl) = (Vec::new(), Vec::new(), Vec::new());
    for (i, t) in settings.templates.iter().enumerate() {
        let dst = if crate::engine::presets::is_adjustment_template(t) {
            &mut adj
        } else if crate::engine::presets::is_container_template(t) {
            &mut cont
        } else {
            &mut tpl
        };
        dst.push((i, t.name.clone()));
    }

    let mut act = None;
    egui::ScrollArea::vertical().show(ui, |ui| {
        // names are user-typed and the pane is narrow: cut them off rather than widening every row
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        section(ui, state, false, "Effects", &fx, &mut act, &mut resp);
        section(ui, state, false, "Node graphs", &graphs, &mut act, &mut resp);
        section(ui, state, true, "Adjustment layers", &adj, &mut act, &mut resp);
        section(ui, state, true, "Container templates", &cont, &mut act, &mut resp);
        section(ui, state, true, "Templates", &tpl, &mut act, &mut resp);
    });

    if let Some(act) = act {
        resp.settings_changed = match act {
            Act::Rename(true, i, name) => {
                settings.templates[i].name = name;
                true
            }
            Act::Rename(false, i, name) => {
                settings.effect_presets[i].name = name;
                true
            }
            // presets live in settings.json, outside undo — ask before losing one
            Act::Delete(t, i) => {
                let name = if t { &settings.templates[i].name } else { &settings.effect_presets[i].name };
                let go = confirm("Delete preset", &format!("Delete the saved preset \"{name}\"?"));
                if go {
                    if t {
                        settings.templates.remove(i);
                    } else {
                        settings.effect_presets.remove(i);
                    }
                }
                go
            }
        };
    }
    resp
}

/// One titled list. `templates` picks both the backing Vec and what a click means (place vs apply).
fn section(
    ui: &mut egui::Ui,
    state: &mut PresetsState,
    templates: bool,
    title: &str,
    rows: &[(usize, String)],
    act: &mut Option<Act>,
    resp: &mut PresetsResponse,
) {
    ui.add_space(4.0);
    ui.strong(title);
    if rows.is_empty() {
        ui.weak("(none)");
        return;
    }
    let tip = if templates { "Place at the playhead" } else { "Apply to the selection" };
    for (i, name) in rows {
        if state.rename.as_ref().is_some_and(|(t, j, _)| *t == templates && j == i) {
            let buf = &mut state.rename.as_mut().unwrap().2;
            if let Some(new) = inline_edit(ui, buf) {
                if !new.is_empty() {
                    *act = Some(Act::Rename(templates, *i, new));
                }
                state.rename = None;
            }
            continue;
        }
        let w = ui.available_width();
        let r = ui.add(egui::Button::new(name.as_str()).min_size(egui::vec2(w, 0.0))).on_hover_text(tip);
        #[cfg(test)]
        test_rects::push(format!("{title}{i}"), r.rect);
        if r.clicked() {
            if templates {
                resp.place.push(name.clone());
            } else {
                resp.apply = Some(*i);
            }
        }
        r.context_menu(|ui| {
            if ui.button("Rename").clicked() {
                state.rename = Some((templates, *i, name.clone()));
                ui.close();
            }
            if ui.button("Delete").clicked() {
                *act = Some(Act::Delete(templates, *i));
                ui.close();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::EffectPreset;
    use eframe::egui::{vec2, Event, Modifiers, PointerButton, Pos2, RawInput, Rect};

    struct Harness {
        ctx: egui::Context,
        state: PresetsState,
        settings: Settings,
        time: f64,
    }

    impl Harness {
        /// One row in each of the four sections.
        fn new() -> Self {
            let mut settings = Settings::default();
            settings.effect_presets.push(EffectPreset { name: "Look".into(), json: "[]".into() });
            settings.effect_presets.push(EffectPreset { name: "Graph".into(), json: "{\"nodes\":[]}".into() });
            let mut p = crate::model::Project::new();
            let a = p.add_adjustment_clip(0.0, 2.0);
            let t = p.add_text_clip(0.0, 2.0);
            settings.templates.push(crate::engine::presets::capture_template("Grade", &p, &[a]));
            settings.templates.push(crate::engine::presets::capture_template("Title", &p, &[t]));
            Self { ctx: egui::Context::default(), state: PresetsState::default(), settings, time: 0.0 }
        }
        fn frame(&mut self, events: Vec<Event>) -> PresetsResponse {
            self.time += 0.05;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(300.0, 600.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let mut resp = PresetsResponse::default();
            let Harness { ctx, state, settings, .. } = self;
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| resp = show(ui, state, settings, true));
            });
            resp
        }
        /// The click lands on the release frame, which is the one returned.
        fn click(&mut self, name: &str) -> PresetsResponse {
            let pos = test_rects::get(name).unwrap_or_else(|| panic!("no widget rect for {name}")).center();
            let btn = |pressed| Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            self.frame(vec![Event::PointerMoved(pos)]);
            self.frame(vec![btn(true)]);
            self.frame(vec![btn(false)])
        }
    }

    /// Headless: the four sections lay out and a row hands back the right intent.
    #[test]
    fn rows_apply_and_place() {
        let mut h = Harness::new();
        h.frame(vec![]);
        // one row per section, split by JSON shape (effects vs graph) and clip kind (adjustment vs not)
        for name in ["Effects0", "Node graphs1", "Adjustment layers0", "Templates1"] {
            assert!(test_rects::get(name).is_some(), "no row for {name}");
        }
        // an effect preset applies to the selection, a template places at the playhead
        let r = h.click("Node graphs1");
        assert_eq!(r.apply, Some(1));
        assert!(r.place.is_empty());
        let r = h.click("Templates1");
        assert_eq!(r.place, vec!["Title".to_string()]);
        assert!(r.apply.is_none());
        // "Save from selection" stays disabled until a name is typed
        assert!(h.click("save").save.is_none());
        h.state.name = " Intro ".into();
        let r = h.click("save");
        assert_eq!(r.save.as_deref(), Some("Intro"));
        assert!(h.state.name.is_empty());
    }

    /// A machine with no presets yet still draws (every section empty, nothing requested).
    #[test]
    fn empty_settings_draw_nothing_to_apply() {
        let mut h = Harness::new();
        h.settings = Settings::default();
        let r = h.frame(vec![]);
        assert!(r.apply.is_none() && r.place.is_empty() && !r.settings_changed);
        assert!(test_rects::get("Effects0").is_none());
    }
}
