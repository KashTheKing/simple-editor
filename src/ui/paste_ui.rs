//! "Paste Attributes" window (Ctrl+Alt+V): a checkbox per `AttrSet` field, All / None buttons, a line
//! saying what is on the clipboard ("from 'clip.mp4'") and how many clips will change. Non-modal, like
//! every other window. Returns Some(set) when the user confirms; the app then calls
//! `Project::paste_attributes` (one undo step).

use crate::model::AttrSet;
use eframe::egui;

#[derive(Default)]
pub struct PasteUi {
    pub open: bool,
    pub set: AttrSet,
}

/// Test-only: remember a widget rect so the headless test can click the real button.
#[cfg(test)]
fn mark(ui: &egui::Ui, name: &str, r: &egui::Response) {
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("paste_btn", name)), r.rect));
}
#[cfg(not(test))]
fn mark(_ui: &egui::Ui, _name: &str, _r: &egui::Response) {}

pub fn show(ctx: &egui::Context, state: &mut PasteUi, source_name: &str, targets: usize) -> Option<AttrSet> {
    if !state.open {
        return None;
    }
    let mut out = None;
    let mut close = false;
    let mut open = true;
    egui::Window::new("Paste Attributes").open(&mut open).resizable(false).show(ctx, |ui| {
        if source_name.is_empty() {
            ui.weak("Nothing copied — use Copy Attributes (Ctrl+Alt+C) first");
        } else {
            ui.weak(format!("from '{source_name}'"));
        }
        ui.horizontal(|ui| {
            if ui.small_button("All").clicked() {
                for (_, v) in state.set.fields() {
                    *v = true;
                }
            }
            if ui.small_button("None").clicked() {
                state.set = AttrSet::NONE;
            }
        });
        ui.separator();
        for (label, v) in state.set.fields() {
            ui.checkbox(v, label);
        }
        ui.separator();
        ui.weak(match targets {
            1 => "1 clip will change".to_string(),
            n => format!("{n} clips will change"),
        });
        let ready = targets > 0 && state.set.any() && !source_name.is_empty();
        ui.horizontal(|ui| {
            let r = ui.add_enabled(ready, egui::Button::new("Paste"));
            mark(ui, "paste", &r);
            if r.clicked() {
                out = Some(state.set);
            }
            if ui.button("Close").clicked() {
                close = true;
            }
        });
    });
    // confirming closes the window too (the app applies the set once)
    state.open = open && !close && out.is_none();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    struct H {
        ctx: egui::Context,
        state: PasteUi,
        time: f64,
    }

    impl H {
        fn new() -> Self {
            Self { ctx: egui::Context::default(), state: PasteUi { open: true, ..Default::default() }, time: 0.0 }
        }
        fn frame(&mut self, events: Vec<egui::Event>) -> Option<AttrSet> {
            self.time += 0.05;
            let input = egui::RawInput { time: Some(self.time), events, ..Default::default() };
            let mut out = None;
            let (state, _) = (&mut self.state, ());
            let _ = self.ctx.run(input, |ctx| out = show(ctx, state, "clip.mp4", 3));
            out
        }
        fn rect(&self, name: &str) -> egui::Rect {
            self.ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("paste_btn", name))))
                .expect("button rect recorded")
        }
        fn click(&mut self, pos: egui::Pos2) -> Option<AttrSet> {
            self.frame(vec![egui::Event::PointerMoved(pos)]);
            let press = egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            };
            self.frame(vec![press]);
            let release = egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            };
            self.frame(vec![release])
        }
    }

    #[test]
    fn closed_window_returns_nothing() {
        let ctx = egui::Context::default();
        let mut st = PasteUi::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            assert!(show(ctx, &mut st, "a.mp4", 1).is_none());
        });
    }

    #[test]
    fn paste_button_returns_the_set_and_closes() {
        let mut h = H::new();
        h.frame(vec![]);
        h.frame(vec![]);
        let r = h.rect("paste");
        let got = h.click(r.center()).expect("Paste returns the attribute set");
        assert_eq!(got, AttrSet::default());
        assert!(!h.state.open, "confirming closes the window");
    }

    #[test]
    fn all_and_none_flip_every_field() {
        let mut set = AttrSet::NONE;
        assert!(!set.any());
        for (_, v) in set.fields() {
            *v = true;
        }
        assert!(set.any());
        assert!(set.fields().iter().all(|(_, v)| **v));
    }
}
