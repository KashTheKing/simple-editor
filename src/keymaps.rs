//! Keymap presets: named diff tables over the pre-existing `Action` ids (`hotkeys::Action::id()`),
//! applied on top of `Hotkeys::defaults()`. Only ids that already exist are listed (a later
//! workstream's new `Action` needing its own preset row is a follow-up, not this file's job - see
//! `keymap_presets_resolve_and_have_no_duplicate_chords`). Ctrl+K is Premiere's own "Add Edit" muscle
//! memory, so its preset rebinds `split` there and moves `command_palette` to Ctrl+Shift+P; the other
//! presets are milder relocations (real per-app accuracy is not the point - a small, safe, genuinely
//! differing diff table per app is).
//! ---- ws:command-palette ----

use crate::hotkeys::{Action, Hotkeys};

/// (preset name, [(action id, chord text)]). Chord text parses via `Hotkeys::parse` (the same format
/// the rebind UI writes back through `Hotkeys::format`). "Simple Editor" is the shipped defaults -
/// applying it is exactly `Hotkeys::reset_all()`.
pub const PRESETS: &[(&str, &[(&str, &str)])] = &[
    ("Simple Editor", &[]),
    ("Premiere", &[("split", "Ctrl+K"), ("command_palette", "Ctrl+Shift+P"), ("export", "Ctrl+M")]),
    ("Resolve", &[("split", "B"), ("command_palette", "Ctrl+Space"), ("toggle_transitions", "Ctrl+Shift+T")]),
    ("Avid", &[("command_palette", "Ctrl+Alt+K"), ("split", "Ctrl+Shift+B")]),
];

/// Apply preset `name`: reset to defaults, then rebind exactly the rows its diff table lists (`Hotkeys::
/// set` unbinds whichever other action, if any, held that chord - normal preset behaviour). `Err` when
/// `name` isn't in `PRESETS`, or a diff row names an id no `Action` has (should be unreachable given
/// `keymap_presets_resolve_and_have_no_duplicate_chords`, but the caller - the `hotkeys.preset` MCP
/// tool, the Hotkeys tab's preset combo - passes a name it did not itself validate).
pub fn apply(name: &str, hotkeys: &mut Hotkeys) -> Result<(), String> {
    let (_, diff) =
        PRESETS.iter().find(|(n, _)| *n == name).ok_or_else(|| format!("unknown keymap preset '{name}'"))?;
    hotkeys.reset_all();
    for &(id, chord) in *diff {
        let a = Action::from_id(id).ok_or_else(|| format!("preset '{name}' references unknown action '{id}'"))?;
        hotkeys.set(a, Hotkeys::parse(chord));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keymap_presets_resolve_and_have_no_duplicate_chords() {
        for (name, diff) in PRESETS {
            let mut seen = std::collections::HashSet::new();
            for &(id, chord) in *diff {
                assert!(Action::from_id(id).is_some(), "preset {name}: unknown action id '{id}'");
                assert!(seen.insert(chord), "preset {name}: duplicate chord '{chord}' in its own diff table");
            }
            let mut hk = Hotkeys::defaults();
            assert!(apply(name, &mut hk).is_ok(), "preset {name} failed to apply");
        }
    }

    #[test]
    fn premiere_moves_split_and_the_palette_off_each_other() {
        let mut hk = Hotkeys::defaults();
        apply("Premiere", &mut hk).unwrap();
        assert_eq!(hk.text(Action::Split), "Ctrl+K");
        assert_eq!(hk.text(Action::CommandPalette), "Ctrl+Shift+P");
    }

    #[test]
    fn unknown_preset_is_an_error_and_changes_nothing() {
        let mut hk = Hotkeys::defaults();
        let before = hk.text(Action::Split);
        assert!(apply("Not A Real Preset", &mut hk).is_err());
        assert_eq!(hk.text(Action::Split), before);
    }

    #[test]
    fn simple_editor_preset_is_the_defaults() {
        let mut hk = Hotkeys::defaults();
        hk.set(Action::Split, None);
        apply("Simple Editor", &mut hk).unwrap();
        assert_eq!(hk.text(Action::Split), Hotkeys::format(&Action::Split.default_shortcut().unwrap()));
    }
}
