//! Effects panel. Top: the catalogue (EffectKind::ALL as buttons / a list; click = add to every selected
//! visual clip, with undo). Below: the FIRST selected clip's effect stack, in order: for each effect a
//! header row (enabled checkbox, name, ▲ ▼ reorder, ✕ remove) and its parameters from
//! `effect.specs()`: label + DragValue (range from ParamSpec, speed ≈ (max-min)/200) + "◆" keyframe toggle
//! at clip-local playhead time (Animated::toggle_key / set_at, highlighted when a key exists) + "✕ keys".
//! Tint shows a colour button bound to the R/G/B params as well. Every change → undo once per gesture
//! (same `edit_start` rule as the inspector) then returns true.

use crate::model::{Animated, Effect, EffectKind, Id, Project};
use crate::settings::MotionPreset;
use crate::theme::Palette;
use eframe::egui::{self, Button, DragValue, Grid, Response};
use std::cell::RefCell;

thread_local! {
    // ponytail: thread_local hand-off because show() can't reach Settings — the app polls
    // take_pending_motion() each frame and stores the preset. Upgrade: pass an EffectsState if the
    // signature is ever allowed to grow.
    static PENDING_MOTION: RefCell<Option<MotionPreset>> = const { RefCell::new(None) };
}

/// A motion preset captured via "Save as motion preset…" waiting for the app to store it in Settings.
pub fn take_pending_motion() -> Option<MotionPreset> {
    PENDING_MOTION.with(|p| p.borrow_mut().take())
}

/// Test-only registry of widget rects so headless tests can click real widgets without pixel-guessing.
#[cfg(test)]
pub(crate) mod test_rects {
    use eframe::egui::Rect;
    use std::cell::RefCell;
    thread_local! {
        static RECTS: RefCell<Vec<(String, Rect)>> = const { RefCell::new(Vec::new()) };
    }
    pub fn clear() {
        RECTS.with(|r| r.borrow_mut().clear());
    }
    pub fn push(name: String, rect: Rect) {
        RECTS.with(|r| r.borrow_mut().push((name, rect)));
    }
    pub fn get(name: &str) -> Option<Rect> {
        RECTS.with(|r| r.borrow().iter().rev().find(|(n, _)| n == name).map(|(_, rect)| *rect))
    }
}

/// True on the first frame of an edit gesture (drag start, or a non-drag change such as typing/clicking).
fn edit_start(r: &Response) -> bool {
    r.drag_started() || (r.changed() && !r.dragged())
}

#[derive(Default)]
struct Gesture {
    start: bool,
    changed: bool,
}

impl Gesture {
    fn note(&mut self, r: &Response) {
        self.start |= edit_start(r);
        self.changed |= r.changed();
    }
    fn click(&mut self) {
        self.start = true;
        self.changed = true;
    }
}

fn key_buttons(ui: &mut egui::Ui, a: &mut Animated, lt: f64, palette: &Palette, g: &mut Gesture, _at: (usize, usize)) {
    let mut b = Button::new("◆").small();
    if a.has_key_at(lt) {
        b = b.fill(palette.keyframe);
    }
    let r = ui.add(b).on_hover_text("Toggle keyframe at playhead");
    #[cfg(test)]
    test_rects::push(format!("kf{}_{}", _at.0, _at.1), r.rect);
    if r.clicked() {
        a.toggle_key(lt);
        g.click();
    }
    if a.is_animated() && ui.small_button("✕ keys").on_hover_text("Remove all keyframes").clicked() {
        a.clear_keys(lt);
        g.click();
    }
}

/// Unit suffix for a wobble parameter.
fn wobble_suffix(name: &str) -> &'static str {
    match name {
        "Amplitude X" | "Amplitude Y" => " px",
        "Roll" | "Yaw" | "Pitch" => "°",
        "Frequency" => " Hz",
        _ => "",
    }
}

pub fn show(
    ui: &mut egui::Ui,
    project: &mut Project,
    selection: &[Id],
    playhead: f64,
    palette: &Palette,
    undo: &mut dyn FnMut(&Project),
) -> bool {
    let mut edited = false;
    #[cfg(test)]
    test_rects::clear();

    // ---- catalogue ----
    ui.strong("Effects");
    let mut add: Option<EffectKind> = None;
    ui.horizontal_wrapped(|ui| {
        for kind in EffectKind::ALL {
            let r = ui.add(Button::new(kind.name()).small());
            #[cfg(test)]
            test_rects::push(format!("cat_{}", kind.name()), r.rect);
            if r.clicked() {
                add = Some(kind);
            }
        }
    });
    if let Some(kind) = add {
        let targets: Vec<Id> =
            selection.iter().copied().filter(|&id| project.clip(id).is_some_and(|c| c.is_visual())).collect();
        if !targets.is_empty() {
            undo(project);
            for id in targets {
                if let Some(c) = project.clip_mut(id) {
                    c.effects.push(Effect::new(kind));
                }
            }
            edited = true;
        }
    }

    // ---- stack of the first selected clip ----
    let Some(&id) = selection.iter().find(|&&id| project.clip(id).is_some()) else {
        ui.separator();
        ui.label("Select a clip");
        return edited;
    };
    // ponytail: edit a per-frame clone and write back (inspector pattern) so `undo` can snapshot first.
    let orig = project.clip(id).cloned();
    let Some(mut clip) = orig else { return edited };
    // clamped: the playhead can sit off the clip, and keys written outside [0, duration] are invisible
    // in every editor yet still make the param "animated" forever (mixer.rs clamps the same way).
    let lt = clip.local(playhead).clamp(0.0, clip.duration);
    let mut g = Gesture::default();

    ui.separator();
    ui.strong(clip.name.clone());
    let n = clip.effects.len();
    let mut remove: Option<usize> = None;
    let mut swap: Option<(usize, usize)> = None;
    for (i, fx) in clip.effects.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            g.note(&ui.checkbox(&mut fx.enabled, ""));
            ui.label(fx.kind.name());
            let up = ui.add_enabled(i > 0, Button::new("▲").small());
            if up.clicked() {
                swap = Some((i, i - 1));
            }
            let down = ui.add_enabled(i + 1 < n, Button::new("▼").small());
            if down.clicked() {
                swap = Some((i, i + 1));
            }
            let del = ui.add(Button::new("✕").small());
            if del.clicked() {
                remove = Some(i);
            }
            #[cfg(test)]
            {
                test_rects::push(format!("up{i}"), up.rect);
                test_rects::push(format!("down{i}"), down.rect);
                test_rects::push(format!("del{i}"), del.rect);
            }
        });
        if fx.kind == EffectKind::Tint && fx.params.len() >= 3 {
            ui.horizontal(|ui| {
                ui.label("Colour");
                let mut rgb = [fx.params[0].at(lt) as u8, fx.params[1].at(lt) as u8, fx.params[2].at(lt) as u8];
                let r = ui.color_edit_button_srgb(&mut rgb);
                if r.changed() {
                    for (a, v) in fx.params.iter_mut().zip(rgb) {
                        a.set_at(lt, v as f64);
                    }
                }
                g.note(&r);
            });
        }
        Grid::new(("fx_params", i)).num_columns(2).show(ui, |ui| {
            for (j, spec) in fx.kind.params().iter().enumerate() {
                let Some(a) = fx.params.get_mut(j) else { continue };
                ui.label(spec.name);
                ui.horizontal(|ui| {
                    let mut v = a.at(lt);
                    // clamp_existing_to_range(false): clamping an out-of-range stored value reports
                    // `changed()`, which would fake an edit just by drawing the panel.
                    let mut dv = DragValue::new(&mut v)
                        .range(spec.min..=spec.max)
                        .clamp_existing_to_range(false)
                        .speed((spec.max - spec.min) / 200.0);
                    if fx.kind == EffectKind::Wobble {
                        dv = dv.suffix(wobble_suffix(spec.name));
                    }
                    let r = ui.add(dv);
                    #[cfg(test)]
                    test_rects::push(format!("param{i}_{j}"), r.rect);
                    if r.changed() {
                        a.set_at(lt, v);
                    }
                    g.note(&r);
                    key_buttons(ui, a, lt, palette, &mut g, (i, j));
                });
                ui.end_row();
            }
        });
    }
    if let Some((a, b)) = swap {
        clip.effects.swap(a, b);
        g.click();
    }
    if let Some(i) = remove {
        clip.effects.remove(i);
        g.click();
    }
    if clip.effects.is_empty() {
        ui.label("No effects — click one above to add it");
    }

    // ---- save the clip's animation as a motion preset ----
    if clip.all_animated().iter().any(|a| a.is_animated()) {
        ui.separator();
        ui.horizontal(|ui| {
            let name_id = ui.id().with("motion_name");
            let mut name: String = ui.ctx().data_mut(|d| d.get_temp(name_id).unwrap_or_default());
            ui.add(egui::TextEdit::singleline(&mut name).hint_text("Preset name").desired_width(120.0));
            if ui.add_enabled(!name.trim().is_empty(), Button::new("Save as motion preset")).clicked() {
                let p = crate::engine::presets::capture_motion(name.trim(), &clip);
                PENDING_MOTION.with(|s| *s.borrow_mut() = Some(p));
                name.clear();
            }
            ui.ctx().data_mut(|d| d.insert_temp(name_id, name));
        });
    }

    if g.start {
        undo(project);
    }
    if g.changed {
        if let Some(c) = project.clip_mut(id) {
            *c = clip;
        }
    }
    edited || g.changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Clip, ClipKind};
    use eframe::egui::{pos2, vec2, Color32, Event, Modifiers, PointerButton, Pos2, RawInput, Rect};

    struct Harness {
        ctx: egui::Context,
        project: Project,
        selection: Vec<Id>,
        undos: usize,
        time: f64,
        playhead: f64,
    }

    impl Harness {
        fn new() -> Self {
            let mut project = Project::new();
            project.tracks[0].clips.push(Clip::new(7, ClipKind::Video, "v", 0.0, 4.0));
            Self { ctx: egui::Context::default(), project, selection: vec![7], undos: 0, time: 0.0, playhead: 1.0 }
        }
        fn frame(&mut self, events: Vec<Event>) -> bool {
            self.time += 0.05;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(500.0, 700.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            };
            let pal = Palette::new(true, Color32::WHITE);
            let Harness { ctx, project, selection, undos, playhead, .. } = self;
            let playhead = *playhead;
            let mut edited = false;
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut undo = |_: &Project| *undos += 1;
                    edited |= show(ui, project, selection, playhead, &pal, &mut undo);
                });
            });
            edited
        }
        fn click(&mut self, pos: Pos2) -> bool {
            self.frame(vec![Event::PointerMoved(pos)]);
            let mut e = self.frame(vec![Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            }]);
            e |= self.frame(vec![Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            }]);
            e |= self.frame(vec![]);
            e
        }
        fn clip(&self) -> &Clip {
            &self.project.tracks[0].clips[0]
        }
    }

    #[test]
    fn catalogue_click_adds_effect_with_one_undo() {
        let mut h = Harness::new();
        h.frame(vec![]);
        let r = test_rects::get("cat_Blur").expect("catalogue button recorded");
        let edited = h.click(r.center());
        assert!(edited, "clicking the Blur catalogue button should add an effect");
        assert_eq!(h.clip().effects.len(), 1);
        assert_eq!(h.clip().effects[0].kind, EffectKind::Blur);
        assert_eq!(h.undos, 1);
    }

    #[test]
    fn stack_reorder_with_one_undo() {
        let mut h = Harness::new();
        {
            let c = &mut h.project.tracks[0].clips[0];
            c.effects.push(Effect::new(EffectKind::Blur));
            c.effects.push(Effect::new(EffectKind::Invert));
        }
        h.frame(vec![]);
        let r = test_rects::get("down0").expect("reorder button recorded");
        assert!(h.click(r.center()));
        assert_eq!(h.clip().effects[0].kind, EffectKind::Invert);
        assert_eq!(h.clip().effects[1].kind, EffectKind::Blur);
        assert_eq!(h.undos, 1);
    }

    #[test]
    fn stack_remove_with_one_undo() {
        let mut h = Harness::new();
        h.project.tracks[0].clips[0].effects.push(Effect::new(EffectKind::Blur));
        h.frame(vec![]);
        let r = test_rects::get("del0").expect("remove button recorded");
        assert!(h.click(r.center()));
        assert!(h.clip().effects.is_empty());
        assert_eq!(h.undos, 1);
    }

    #[test]
    fn param_drag_edits_with_one_undo() {
        let mut h = Harness::new();
        h.project.tracks[0].clips[0].effects.push(Effect::new(EffectKind::Blur));
        h.frame(vec![]);
        let r = test_rects::get("param0_0").expect("param drag value recorded");
        let from = r.center();
        h.frame(vec![Event::PointerMoved(from)]);
        h.frame(vec![Event::PointerButton {
            pos: from,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        }]);
        let mut edited = false;
        for i in 1..=4 {
            let p = from + vec2(10.0 * i as f32, 0.0);
            edited |= h.frame(vec![Event::PointerMoved(p)]);
        }
        edited |= h.frame(vec![Event::PointerButton {
            pos: from + vec2(40.0, 0.0),
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }]);
        edited |= h.frame(vec![]);
        assert!(edited, "dragging the Blur radius should edit the project");
        let radius = h.clip().effects[0].params[0].value;
        assert!(radius > 8.0, "radius should have grown from the default: {radius}");
        assert_eq!(h.undos, 1, "one drag gesture = one undo");
    }

    #[test]
    fn keyframe_button_off_clip_keys_inside_the_clip() {
        let mut h = Harness::new();
        h.project.tracks[0].clips[0].start = 10.0; // clip lives at 10..14
        h.project.tracks[0].clips[0].effects.push(Effect::new(EffectKind::Blur));
        h.playhead = 0.0; // playhead well before the clip → local time -10
        h.frame(vec![]);
        let r = test_rects::get("kf0_0").expect("keyframe button recorded");
        assert!(h.click(r.center()));
        let keys = &h.clip().effects[0].params[0].keys;
        assert_eq!(keys.len(), 1);
        assert!(keys[0].t >= 0.0 && keys[0].t <= 4.0, "key must land inside the clip, got t = {}", keys[0].t);
    }

    #[test]
    fn no_selection_is_harmless() {
        let mut h = Harness::new();
        h.selection.clear();
        assert!(!h.frame(vec![]));
        assert_eq!(h.undos, 0);
    }

    #[test]
    fn pending_motion_handoff() {
        assert!(take_pending_motion().is_none());
        PENDING_MOTION.with(|s| *s.borrow_mut() = Some(MotionPreset { name: "m".into(), props: Vec::new() }));
        assert_eq!(take_pending_motion().map(|m| m.name), Some("m".into()));
        assert!(take_pending_motion().is_none());
    }
}
