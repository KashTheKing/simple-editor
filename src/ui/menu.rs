//! ---- ws:ui-kit ----
//! One menu row for every `Action`: the menu bar and every right-click menu draw through here, so the
//! label, icon (Settings ▸ Appearance ▸ Icons overrides honoured), shortcut text and enabled state match
//! everywhere. No context plumbing: `App::update` publishes a `MenuSnapshot` once per frame and drains
//! the clicked Actions next frame into its normal `act()` dispatch - the same thread-local shape as
//! `inspector::PENDING_ACTION` / `confirm::STAGED`. Signatures frozen after ui-kit (wave 1 calls them).

use crate::hotkeys::{Action, Hotkeys};
use crate::ui::tools::{self, Glyph};
use eframe::egui;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};

/// What a menu row needs from `App`, captured once per frame.
#[derive(Default)]
pub struct MenuSnapshot {
    /// Current (rebindable) shortcut of every bound action.
    pub shortcuts: HashMap<Action, egui::KeyboardShortcut>,
    /// `Settings.icon_overrides`.
    pub icons: BTreeMap<String, String>,
    /// Actions `App::enabled` refuses right now, with its reason (shown on hover).
    pub disabled: HashMap<Action, &'static str>,
}

impl MenuSnapshot {
    pub fn new(
        hotkeys: &Hotkeys,
        icons: &BTreeMap<String, String>,
        enabled: impl Fn(Action) -> Result<(), &'static str>,
    ) -> Self {
        let mut s = Self { icons: icons.clone(), ..Default::default() };
        for &a in Action::ALL {
            if let Some(ks) = hotkeys.get(a) {
                s.shortcuts.insert(a, ks);
            }
            if let Err(why) = enabled(a) {
                s.disabled.insert(a, why);
            }
        }
        s
    }
}

thread_local! {
    static SNAPSHOT: RefCell<MenuSnapshot> = RefCell::new(MenuSnapshot::default());
    static QUEUE: RefCell<Vec<Action>> = const { RefCell::new(Vec::new()) };
    /// Icon picks from a row's right-click: (`icon_overrides` key, `None` = default / `Some(name)`).
    static ICON_PICKS: RefCell<Vec<(String, Option<String>)>> = const { RefCell::new(Vec::new()) };
}

/// `App::update`, once per frame, before anything draws a menu.
pub fn publish(s: MenuSnapshot) {
    SNAPSHOT.with(|c| *c.borrow_mut() = s);
}

/// Actions clicked through `action_item` / `action_menu` since the last call.
pub fn take_queued() -> Vec<Action> {
    QUEUE.with(|q| std::mem::take(&mut *q.borrow_mut()))
}

/// Icon picks made from a row's right-click since the last call.
pub fn take_icon_picks() -> Vec<(String, Option<String>)> {
    ICON_PICKS.with(|q| std::mem::take(&mut *q.borrow_mut()))
}

/// Icon for an action: the user's override first ("none" = no icon), then `tools::action_glyph`.
pub fn glyph_for(icons: &BTreeMap<String, String>, a: Action) -> Option<Glyph> {
    match icons.get(&format!("action.{}", a.id())) {
        Some(name) if name == "none" => None,
        Some(name) => Glyph::from_name(name),
        None => tools::action_glyph(a),
    }
}

/// One row: icon gutter, label, shortcut; greyed (with the reason on hover) while `App::enabled` says
/// no or `enabled` is false; right-click picks its icon. True when clicked (the menu closes). Does NOT
/// queue the action - `App::menu_item` pushes it straight into its own list.
pub fn item(ui: &mut egui::Ui, a: Action, enabled: bool) -> bool {
    let (glyph, shortcut, reason) = SNAPSHOT.with(|s| {
        let s = s.borrow();
        (glyph_for(&s.icons, a), s.shortcuts.get(&a).map(Hotkeys::format), s.disabled.get(&a).copied())
    });
    let icon_id = ui.id().with(("menu-icon", a.id()));
    // the gutter is there with or without an icon, so every label in a menu starts at the same x
    let gutter = egui::Atom::custom(icon_id, egui::vec2(18.0, 16.0));
    let button = egui::Button::new((gutter, a.label())).shortcut_text(shortcut.unwrap_or_default());
    let r = ui
        .add_enabled_ui(enabled && reason.is_none(), |ui| {
            let r = button.atom_ui(ui);
            if let (Some(g), Some((_, rect))) = (glyph, r.custom_rects().next()) {
                // this ui's painter already fades for a disabled row
                tools::draw_glyph(ui.painter(), rect, g, ui.style().interact(&r.response).text_color());
            }
            r.response
        })
        .inner;
    let r = match reason {
        Some(why) => r.on_disabled_hover_text(why),
        None => r,
    };
    r.context_menu(|ui| {
        if let Some(pick) = crate::ui::layout::icon_menu(ui) {
            ICON_PICKS.with(|q| q.borrow_mut().push((format!("action.{}", a.id()), pick)));
            ui.ctx().request_repaint();
        }
    });
    let clicked = r.clicked();
    if clicked {
        ui.close();
    }
    clicked
}

/// `item` for any right-click menu: a click queues `a` for `App::update` to run (next frame).
#[allow(dead_code)] // wave-1 right-click menus are the callers
pub fn action_item(ui: &mut egui::Ui, a: Action) -> bool {
    let clicked = item(ui, a, true);
    if clicked {
        QUEUE.with(|q| q.borrow_mut().push(a));
        ui.ctx().request_repaint(); // the queue drains at the top of the next frame
    }
    clicked
}

/// A whole menu body from Actions, `None` = separator. Long menus scroll instead of running off-screen.
#[allow(dead_code)] // wave-1 right-click menus are the callers
pub fn action_menu(ui: &mut egui::Ui, items: &[Option<Action>]) {
    scroll(ui, |ui| {
        for it in items {
            match *it {
                Some(a) => {
                    action_item(ui, a);
                }
                None => {
                    ui.separator();
                }
            }
        }
    });
}

/// Cap a menu at 80 % of the window height; the scroll bar only shows when it doesn't fit.
#[allow(dead_code)] // wave-1 right-click menus are the callers
pub fn scroll<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let max = ui.ctx().content_rect().height() * 0.8;
    egui::ScrollArea::vertical().max_height(max).show(ui, add).inner
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Modifiers, PointerButton, Pos2, Rect, Vec2};

    /// Draw `rows` in a bare panel for three frames with a click on the first row's centre; returns
    /// what was queued.
    fn click_first(rows: &[Action]) -> Vec<Action> {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::theme::test_fonts());
        let at = std::cell::Cell::new(Pos2::ZERO);
        let frame = |events: Vec<Event>, t: f64| {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 300.0))),
                time: Some(t),
                events,
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    for (i, &a) in rows.iter().enumerate() {
                        let top = ui.cursor().top();
                        action_item(ui, a);
                        if i == 0 {
                            at.set(Pos2::new(20.0, top + 8.0));
                        }
                    }
                });
            });
        };
        frame(vec![], 0.0);
        let press = |pressed| Event::PointerButton {
            pos: at.get(),
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        frame(vec![Event::PointerMoved(at.get())], 0.1);
        frame(vec![press(true)], 0.2);
        frame(vec![press(false)], 0.3);
        take_queued()
    }

    #[test]
    fn snapshot_and_queue_round_trip() {
        let hk = Hotkeys::defaults();
        let mut icons = BTreeMap::new();
        icons.insert("action.save".to_string(), "none".to_string());
        let snap = MenuSnapshot::new(&hk, &icons, |a| if a == Action::Undo { Err("Nothing to undo") } else { Ok(()) });
        assert_eq!(snap.disabled.get(&Action::Undo), Some(&"Nothing to undo"));
        assert!(snap.shortcuts.contains_key(&Action::Save), "Ctrl+S is bound by default");
        assert_eq!(glyph_for(&snap.icons, Action::Save), None, "'none' override hides the icon");
        assert_eq!(glyph_for(&snap.icons, Action::Undo), tools::action_glyph(Action::Undo));
        publish(snap);
        assert_eq!(click_first(&[Action::Save, Action::Undo]), vec![Action::Save], "a click queues its action");
        assert!(click_first(&[Action::Undo, Action::Save]).is_empty(), "a disabled row queues nothing");
        assert!(take_queued().is_empty(), "take drains");
    }
}
