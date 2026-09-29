//! F1 keyboard-shortcuts overlay: every `Action` grouped by `hotkeys::group` (bound first, unbound
//! last with a dim " - "), the hard-coded keys outside that table (`hotkeys::RESERVED`), and the
//! timeline's mouse gestures read straight from its frozen modifier table (`timeline::arm`), all
//! searchable. A plain, non-blocking `egui::Window` (same shape as every other overlay in this crate -
//! Retime, Export, Settings - never modal).
//! ---- ws:command-palette ----

use crate::hotkeys::{self, Action, Hotkeys, RESERVED};
use crate::ui::timeline::{arm, GestureKind, TrackFlags, Zone};
use crate::ui::tools::Tool;
use eframe::egui::{self, KeyboardShortcut, Modifiers};

/// One heading and its rows: (what it does, how - a chord, or a gesture like "Alt+drag clip edge").
pub(crate) struct Section {
    pub title: &'static str,
    pub rows: Vec<(String, String)>,
    /// The "how" column is a chord (monospace) rather than words.
    pub keys: bool,
}

/// Everything the sheet lists, filtered by `filter` (case-insensitive, either column); empty sections
/// drop out.
pub(crate) fn sections(hotkeys: &Hotkeys, filter: &str) -> Vec<Section> {
    let f = filter.trim().to_lowercase();
    let keep = |(what, how): &(String, String)| {
        f.is_empty() || what.to_lowercase().contains(&f) || how.to_lowercase().contains(&f)
    };
    let mut out: Vec<Section> = hotkeys::grouped()
        .into_iter()
        .map(|(title, actions)| {
            let mut rows: Vec<(String, String)> =
                actions.iter().map(|&a| (a.label().to_string(), hotkeys.text(a))).filter(keep).collect();
            rows.sort_by_key(|(_, how)| how.is_empty()); // unbound last, table order otherwise
            Section { title, rows, keys: true }
        })
        .collect();
    let fixed = RESERVED.iter().map(|&(name, m, k)| (name.to_string(), Hotkeys::format(&KeyboardShortcut::new(m, k))));
    out.push(Section { title: "Keys outside the table", rows: fixed.filter(keep).collect(), keys: true });
    out.push(Section { title: "Mouse", rows: mouse_rows().into_iter().filter(keep).collect(), keys: false });
    out.retain(|s| !s.rows.is_empty());
    out
}

/// What a gesture does, and the verb that starts it. `None` for the ones no Select-tool press arms.
fn gesture_text(g: GestureKind) -> Option<(&'static str, &'static str)> {
    use GestureKind::*;
    Some(match g {
        MoveNoOverlap => ("Move", "drag"),
        MagneticMove => ("Move, closing the gap", "drag"),
        Slip => ("Slip (the content slides, the clip stays)", "drag"),
        Slide => ("Slide (the neighbours trim to make room)", "drag"),
        Segment => ("Insert-move (close the gap, open one where it lands)", "drag"),
        Trim => ("Trim", "drag"),
        RippleTrim => ("Ripple trim (later clips follow)", "drag"),
        Roll => ("Roll the cut", "drag"),
        RateStretch => ("Rate stretch (change speed to fit)", "drag"),
        MultiRippleTrim => ("Ripple trim every selected clip", "drag"),
        SplitAt => ("Split here", "click"),
        SeamBoth => ("Select the edit point", "click"),
        SeamLeft => ("Select the outgoing side", "click"),
        SeamRight => ("Select the incoming side", "click"),
        SeamAddToSet => ("Add the cut to the trim set", "click"),
        GapSelect => ("Select the gap", "click"),
        RubberBandAdd => ("Add to the selection with a rubber band", "drag"),
        RulerInOutDrag => ("Move the In / Out mark", "drag"),
        DropDefault => ("Place on the free track under the pointer", "drop"),
        DropSplice => ("Insert, pushing later clips right", "drop"),
        DropOverwrite => ("Overwrite what's there", "drop"),
        DropPlaceOnTop => ("Place on a new track on top", "drop"),
        Pan | LegacyToolGesture => return None,
    })
}

/// The timeline's mouse rows, generated from `arm()` so they can't drift from what a press does: every
/// (zone, modifiers) the table defines for the Select tool, plus the magnetic-track variant where it
/// differs. A modifier that changes nothing about the drag (Ctrl/Shift on a clip only change the
/// selection) is listed once, with the click rows below it.
fn mouse_rows() -> Vec<(String, String)> {
    const ZONES: [(Zone, &str); 7] = [
        (Zone::Body, "clip"),
        (Zone::BodyBottom, "clip's lower half"),
        (Zone::EdgeEnd, "clip edge"),
        (Zone::Seam, "cut"),
        (Zone::Lane, "empty track"),
        (Zone::RulerInOut, "In / Out on the ruler"),
        (Zone::Drop, "from the Library"),
    ];
    let ctrl_alt = Modifiers { ctrl: true, alt: true, ..Modifiers::NONE };
    let ctrl_shift = Modifiers { ctrl: true, shift: true, ..Modifiers::NONE };
    let mods = [Modifiers::NONE, Modifiers::CTRL, Modifiers::ALT, Modifiers::SHIFT, ctrl_alt, ctrl_shift];
    let (flat, magnetic) = (TrackFlags::default(), TrackFlags { magnetic: true, ..TrackFlags::default() });
    let mut rows = Vec::new();
    for (zone, noun) in ZONES {
        let plain = arm(Modifiers::NONE, zone, flat, Tool::Select);
        for m in mods {
            let Some(g) = arm(m, zone, flat, Tool::Select) else { continue };
            let Some((what, verb)) = gesture_text(g).filter(|_| m == Modifiers::NONE || Some(g) != plain) else {
                continue;
            };
            let pre: String = [(m.ctrl, "Ctrl+"), (m.shift, "Shift+"), (m.alt, "Alt+")]
                .iter()
                .filter(|(on, _)| *on)
                .map(|(_, s)| *s)
                .collect();
            let how = format!("{pre}{verb} {noun}");
            rows.push((what.to_string(), how.clone()));
            let mag = arm(m, zone, magnetic, Tool::Select).filter(|&mg| mg != g).and_then(gesture_text);
            if let Some((what, _)) = mag {
                let short = what.split(" (").next().unwrap_or(what);
                rows.push((format!("{short} on a magnetic track"), how));
            }
        }
    }
    for (what, how) in [
        ("Add a clip to the selection or take it out", "Ctrl+click clip"),
        ("Add a clip to the selection", "Shift+click clip"),
        ("Select a clip without its linked clips", "Alt+click clip"),
        ("Pan the timeline", "middle-drag"),
        ("Zoom the timeline", "Ctrl+scroll"),
        ("Scroll the timeline sideways", "Shift+scroll"),
        ("Make a track taller or shorter", "Alt+scroll"),
    ] {
        rows.push((what.to_string(), how.to_string()));
    }
    rows
}

/// Draws the cheat-sheet window while `*open`.
pub fn show(ctx: &egui::Context, hotkeys: &Hotkeys, open: &mut bool) {
    if !*open {
        return;
    }
    let mut still_open = true;
    let search_id = egui::Id::new("cheatsheet-search");
    let mut filter: String = ctx.data_mut(|d| d.get_temp(search_id)).unwrap_or_default();
    egui::Window::new("Keyboard Shortcuts").open(&mut still_open).default_width(520.0).default_height(600.0).show(
        ctx,
        |ui| {
            let palette = hotkeys.text(Action::CommandPalette);
            if !palette.is_empty() {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&palette).monospace().strong());
                    ui.weak("searches every command by name");
                });
            }
            ui.add(egui::TextEdit::singleline(&mut filter).hint_text("Search shortcuts…").desired_width(f32::INFINITY));
            ui.separator();
            let sections = sections(hotkeys, &filter);
            egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
                // one grid, so both columns line up across every section
                egui::Grid::new("cheatsheet").num_columns(2).spacing([24.0, 3.0]).show(ui, |ui| {
                    for (i, s) in sections.iter().enumerate() {
                        ui.vertical(|ui| {
                            if i > 0 {
                                ui.add_space(8.0);
                            }
                            ui.strong(s.title);
                        });
                        ui.end_row();
                        for (what, how) in &s.rows {
                            ui.label(what);
                            if how.is_empty() {
                                ui.weak(" - ");
                            } else if s.keys {
                                ui.monospace(how);
                            } else {
                                ui.label(how);
                            }
                            ui.end_row();
                        }
                    }
                });
                if sections.is_empty() {
                    ui.weak("Nothing matches.");
                }
            });
        },
    );
    ctx.data_mut(|d| d.insert_temp(search_id, filter));
    *open = still_open;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shows_every_group_without_panicking() {
        let ctx = egui::Context::default();
        let hk = Hotkeys::defaults();
        let mut open = true;
        for _ in 0..2 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| show(ctx, &hk, &mut open));
        }
        assert!(open);
    }

    /// Every Action is listed exactly once, under its own group, bound ones ahead of unbound ones.
    #[test]
    fn lists_every_action_once_bound_first() {
        let mut hk = Hotkeys::defaults();
        hk.set(Action::Save, None); // an unbound File action goes below the bound ones
        let all = sections(&hk, "");
        for &a in Action::ALL {
            let hits: Vec<&str> = all
                .iter()
                .filter(|s| s.rows.iter().any(|(what, how)| what == a.label() && *how == hk.text(a)))
                .map(|s| s.title)
                .collect();
            assert_eq!(hits, vec![hotkeys::group(a)], "{a:?}");
        }
        let file = &all.iter().find(|s| s.title == "File").unwrap().rows;
        let first_unbound = file.iter().position(|(_, how)| how.is_empty()).unwrap();
        assert!(file[first_unbound..].iter().all(|(_, how)| how.is_empty()), "unbound actions come last");
        assert!(file[first_unbound..].iter().any(|(what, _)| what == Action::Save.label()));
    }

    /// The out-of-table keys and the timeline's modifier table are there, the latter as `arm()` says.
    #[test]
    fn lists_fixed_keys_and_mouse_gestures() {
        let all = sections(&Hotkeys::defaults(), "");
        let rows = |title: &str| all.iter().find(|s| s.title == title).map(|s| s.rows.clone()).unwrap_or_default();
        let fixed = rows("Keys outside the table");
        for want in [("Cycle shape tool", "Shift+S"), ("Redo (alias)", "Ctrl+Y"), ("Exit fullscreen", "Escape")] {
            assert!(fixed.iter().any(|(w, h)| w == want.0 && h == want.1), "{want:?} in {fixed:?}");
        }
        let mouse = rows("Mouse");
        for (what, how) in [
            ("Trim", "drag clip edge"),
            ("Ripple trim (later clips follow)", "Ctrl+drag clip edge"),
            ("Roll the cut", "Alt+drag clip edge"),
            ("Rate stretch (change speed to fit)", "Shift+drag clip edge"),
            ("Ripple trim on a magnetic track", "drag clip edge"),
            ("Move, closing the gap on a magnetic track", "drag clip"),
            ("Slip (the content slides, the clip stays)", "Alt+drag clip"),
            ("Slide (the neighbours trim to make room)", "Ctrl+Alt+drag clip"),
            ("Insert-move (close the gap, open one where it lands)", "Ctrl+Shift+drag clip"),
            ("Insert, pushing later clips right", "Ctrl+drop from the Library"),
            ("Select the outgoing side", "Ctrl+click cut"),
        ] {
            assert!(mouse.iter().any(|(w, h)| w == what && h == how), "missing {how} = {what}");
        }
        // Ctrl / Shift don't change a clip drag, so there's no second "Move" row for them
        assert_eq!(mouse.iter().filter(|(w, _)| w == "Move").count(), 1);
    }

    #[test]
    fn search_filters_either_column() {
        let hk = Hotkeys::defaults();
        let hits = sections(&hk, "ripple");
        assert!(hits.iter().all(|s| s.rows.iter().all(|(w, h)| (w.clone() + h).to_lowercase().contains("ripple"))));
        assert!(hits.iter().any(|s| s.title == "Mouse"), "gestures are searchable too");
        let by_key = sections(&hk, "ctrl+shift+z");
        assert_eq!(by_key.len(), 1);
        assert_eq!(by_key[0].rows, vec![(Action::Redo.label().to_string(), "Ctrl+Shift+Z".to_string())]);
        assert!(sections(&hk, "zzzz nothing").is_empty());
    }

    /// 30 idle frames, no input: the overlay must not cost a repaint while it sits open and unchanged.
    #[test]
    fn assert_no_idle_repaint_cheatsheet_open() {
        let ctx = egui::Context::default();
        let hk = Hotkeys::defaults();
        let mut open = true;
        for _ in 0..30 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| show(ctx, &hk, &mut open));
        }
        assert!(!ctx.has_requested_repaint(), "idle cheat-sheet requested a repaint");
    }
}
