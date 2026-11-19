//! ---- ws:player-metadata ----
//! Settings ▸ Metadata: saved tag presets (artist / author, genre, copyright, comment, Explorer tags ...),
//! applied in one click from the media player's Metadata panel, the Library's right-click on selected
//! media and the Export panel. A table of presets (drag or Alt+Up / Down to reorder), the selected one's
//! fields under it.

use crate::engine::tags::{MetaPreset, FIELDS};
use crate::settings::Settings;
use crate::ui::list_rows::{Ctrl, Moved, RowSpec, Rows};
use eframe::egui;

const CTRLS: &[Ctrl] = &[Ctrl { glyph: "⧉", hint: "Duplicate" }, Ctrl { glyph: "✕", hint: "Delete" }];

/// Move `v[from]` as `m` asks (a drop before slot `slot`, or a step).
pub fn apply_move<T>(v: &mut Vec<T>, m: &Moved) -> Option<usize> {
    let (from, to) = match *m {
        Moved::Slot { from, slot, .. } => (from, if slot > from { slot - 1 } else { slot }),
        Moved::Step { from, delta } => (from, (from as i64 + delta as i64).clamp(0, v.len() as i64 - 1) as usize),
        Moved::Indent { .. } => return None,
    };
    if from >= v.len() || to >= v.len() || from == to {
        return None;
    }
    let it = v.remove(from);
    v.insert(to, it);
    Some(to)
}

/// How many fields a preset fills (its Explorer tags count as one).
pub fn filled(p: &MetaPreset) -> usize {
    p.values.values().filter(|v| !v.trim().is_empty()).count() + usize::from(!p.keywords.trim().is_empty())
}

fn key_of(i: usize, p: &MetaPreset) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (i, &p.name).hash(&mut h);
    h.finish()
}

/// The tab. True when a preset changed (the caller saves).
pub fn metadata_tab(ui: &mut egui::Ui, sel: &mut Option<usize>, settings: &mut Settings) -> bool {
    let mut changed = false;
    ui.strong("Metadata presets");
    ui.weak("Applied in one click from the player's Metadata panel, the Library's right-click and Export.");
    ui.add_space(6.0);
    let list = &mut settings.metadata_presets;
    let mut rows = Rows::new(ui, "metadata_presets", CTRLS, &[80.0], false);
    let mut act = None;
    for (i, p) in list.iter().enumerate() {
        let n = format!("{} field{}", filled(p), if filled(p) == 1 { "" } else { "s" });
        let name = if p.name.trim().is_empty() { "(unnamed)" } else { p.name.as_str() };
        let r = rows.row(ui, i, &RowSpec { key: key_of(i, p), label: name, values: &[n.as_str()], ..Default::default() });
        if r.body.clicked() {
            *sel = Some(i);
        }
        if let Some(k) = r.ctrl {
            act = Some((i, k));
        }
    }
    if let Some(m) = rows.finish(ui) {
        if let Some(to) = apply_move(list, &m) {
            *sel = Some(to);
            changed = true;
        }
    }
    match act {
        Some((i, 0)) => {
            let mut c = list[i].clone();
            c.name = format!("{} copy", c.name);
            list.insert(i + 1, c);
            *sel = Some(i + 1);
            changed = true;
        }
        Some((i, _)) => {
            list.remove(i);
            *sel = None;
            changed = true;
        }
        None => {}
    }
    if list.is_empty() {
        ui.weak("No presets yet.");
    }
    if ui.button("+ New preset").clicked() {
        list.push(MetaPreset { name: format!("Preset {}", list.len() + 1), ..Default::default() });
        *sel = Some(list.len() - 1);
        changed = true;
    }
    let Some(p) = sel.and_then(|i| list.get_mut(i)) else { return changed };
    ui.add_space(10.0);
    ui.separator();
    changed |= preset_fields(ui, p);
    changed
}

/// One preset's name and fields (empty = not part of it). True when something changed.
pub fn preset_fields(ui: &mut egui::Ui, p: &mut MetaPreset) -> bool {
    let mut changed = false;
    egui::Grid::new("metadata_preset_fields").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
        ui.label("Name");
        changed |= ui.add(egui::TextEdit::singleline(&mut p.name).desired_width(260.0)).changed();
        ui.end_row();
        for f in FIELDS.iter().filter(|f| f.key != "title" && f.key != "track" && f.key != "disc" && f.key != "lyrics") {
            ui.label(if f.key == "artist" { "Artist / Author" } else { f.label });
            let mut v = p.values.get(f.key).cloned().unwrap_or_default();
            let edit = if f.multiline { egui::TextEdit::multiline(&mut v).desired_rows(2) } else { egui::TextEdit::singleline(&mut v) };
            if ui.add(edit.desired_width(260.0)).changed() {
                changed = true;
                if v.is_empty() {
                    p.values.remove(f.key);
                } else {
                    p.values.insert(f.key.to_string(), v);
                }
            }
            ui.end_row();
        }
        ui.label("Explorer tags").on_hover_text("Windows' Tags (Details ▸ Tags), separated by ; - M4A, MP4, MOV, WMA and WMV files hold them");
        changed |= ui.add(egui::TextEdit::singleline(&mut p.keywords).desired_width(260.0).hint_text("travel; 2026")).changed();
        ui.end_row();
    });
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves_reorder_the_presets() {
        let mut v = vec!['a', 'b', 'c', 'd'];
        assert_eq!(apply_move(&mut v, &Moved::Slot { from: 0, slot: 3, depth: None }), Some(2));
        assert_eq!(v, ['b', 'c', 'a', 'd']);
        assert_eq!(apply_move(&mut v, &Moved::Step { from: 3, delta: -1 }), Some(2));
        assert_eq!(v, ['b', 'c', 'd', 'a']);
        assert_eq!(apply_move(&mut v, &Moved::Step { from: 0, delta: -1 }), None, "already first");
    }

    #[test]
    fn a_preset_writes_only_what_it_fills() {
        let mut p = MetaPreset { name: "Channel".into(), keywords: "a; b".into(), ..Default::default() };
        p.values.insert("artist".into(), "Kash".into());
        p.values.insert("genre".into(), "  ".into());
        p.values.insert("composer".into(), "Me".into());
        let e = p.edit();
        assert_eq!(e.set.keys().collect::<Vec<_>>(), ["artist", "composer"]);
        assert!(!e.clear && e.cover == crate::engine::tags::CoverEdit::Keep);
        assert!(!p.edit_for(crate::engine::tags::Container::Wav).set.contains_key("composer"), "WAV keeps no composer");
        assert_eq!(p.keyword_list(), ["a", "b"]);
        assert_eq!(filled(&p), 3);
    }
}
