//! ---- ws:mcp-permissions ----
//! The Permissions tab: every MCP tool, grouped by area (`mcp::perms::group`), one aligned row each
//! (`ui::list_rows`) with its state and Allow / Ask / Block buttons; a group's row sets all of its tools.
//! Search, Reset to defaults, and the permission mode for MCP clients other than an Assistant chat.
//! Only an override that differs from the tool's default is stored (`Settings.mcp_perms`).

use crate::mcp::perms::{self, Mode, Perm};
use crate::settings::Settings;
use crate::ui::list_rows::{Ctrl, RowSpec, Rows};
use eframe::egui;
use std::collections::BTreeMap;

const CTRLS: &[Ctrl] = &[
    Ctrl { glyph: "✓", hint: "Allow: runs without asking" },
    Ctrl { glyph: "?", hint: "Ask: waits for your Allow" },
    Ctrl { glyph: "✕", hint: "Block: never runs" },
];

/// Set `tool` to `p`, storing it only when it differs from the default.
pub(crate) fn set(s: &mut Settings, tool: &str, p: Perm) {
    if p == perms::default_for(tool) {
        s.mcp_perms.remove(tool);
    } else {
        s.mcp_perms.insert(tool.to_string(), p);
    }
}

fn state(s: &Settings, tool: &str) -> Perm {
    s.mcp_perms.get(tool).copied().unwrap_or_else(|| perms::default_for(tool))
}

pub(super) fn permissions_tab(ui: &mut egui::Ui, search: &mut String, open: &mut Vec<String>, s: &mut Settings) -> bool {
    let mut changed = false;
    if perms::full_access() {
        ui.horizontal(|ui| {
            crate::ui::assistant_ui::full_access_badge(ui);
            ui.weak("Every MCP call runs now, whatever is set here.");
        });
        ui.add_space(4.0);
    }
    egui::Grid::new("perm_settings").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
        ui.label("Other MCP clients");
        ui.horizontal(|ui| {
            for m in Mode::ALL {
                changed |= ui.radio_value(&mut s.mcp_client_mode, m, m.label()).on_hover_text(m.hint()).changed();
            }
        })
        .response
        .on_hover_text("Agents connected to this editor's MCP server from outside the Assistant. Each chat picks its own under its message box.");
        ui.end_row();
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(search).hint_text("Search tools").desired_width(220.0));
        let reset = ui.add_enabled(!s.mcp_perms.is_empty(), egui::Button::new("Reset to defaults"));
        if reset.on_hover_text("Reads allow; edits, exports and anything that leaves the project ask").clicked() {
            s.mcp_perms.clear();
            changed = true;
        }
    });
    ui.add_space(4.0);

    let q = search.trim().to_ascii_lowercase();
    let mut groups: BTreeMap<&str, Vec<&'static crate::mcp::tools::ToolDef>> = BTreeMap::new();
    for t in crate::mcp::tools::all().filter(|t| q.is_empty() || t.name.contains(q.as_str())) {
        groups.entry(perms::group(t.name)).or_default().push(t);
    }
    let mut rows = Rows::new(ui, "perm_rows", CTRLS, &[60.0, 60.0], true).fixed_order();
    let mut n = 0;
    for (g, tools) in &groups {
        let is_open = !q.is_empty() || open.iter().any(|o| o == g);
        let count = |p| tools.iter().filter(|t| state(s, t.name) == p).count();
        let summary = match Perm::ALL.map(count) {
            [a, 0, 0] if a > 0 => "Allow".to_string(),
            [0, a, 0] if a > 0 => "Ask".to_string(),
            [0, 0, a] if a > 0 => "Block".to_string(),
            _ => "Mixed".to_string(),
        };
        let size = tools.len().to_string();
        let r = rows.row(ui, n, &RowSpec { key: n as u64, label: g, open: Some(is_open), values: &[size.as_str(), summary.as_str()], ..Default::default() });
        n += 1;
        if r.toggled || r.body.clicked() {
            match open.iter().position(|o| o == g) {
                Some(i) => drop(open.remove(i)),
                None => open.push(g.to_string()),
            }
        }
        if let Some(k) = r.ctrl {
            for t in tools {
                set(s, t.name, Perm::ALL[k]);
            }
            changed = true;
        }
        if !is_open {
            continue;
        }
        for t in tools {
            let p = state(s, t.name);
            let d = perms::default_for(t.name);
            let note = if p == d { "" } else { "changed" };
            let r = rows.row(ui, n, &RowSpec { key: n as u64, label: t.name, depth: 1, values: &[note, p.label()], ..Default::default() });
            n += 1;
            let first = t.desc.split(". ").next().unwrap_or(t.desc);
            r.body.on_hover_text(format!("{first}\nDefault: {}", d.label()));
            if let Some(k) = r.ctrl {
                set(s, t.name, Perm::ALL[k]);
                changed = true;
            }
        }
    }
    rows.finish(ui);
    if n == 0 {
        ui.weak("No tool matches.");
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_stores_only_what_differs_from_the_default() {
        let mut s = Settings::default();
        set(&mut s, "timeline.list", Perm::Allow); // its default
        assert!(s.mcp_perms.is_empty());
        set(&mut s, "timeline.list", Perm::Block);
        assert_eq!(s.mcp_perms.get("timeline.list"), Some(&Perm::Block));
        set(&mut s, "timeline.list", Perm::Allow);
        assert!(s.mcp_perms.is_empty());
    }

    /// Group and tool rows put their controls at one x, whatever the name's length.
    #[test]
    fn assert_no_idle_repaint_permissions_tab() {
        let ctx = egui::Context::default();
        let mut s = Settings::default();
        let (mut search, mut open) = (String::new(), vec!["timeline".to_string(), "media".to_string()]);
        for _ in 0..3 {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| permissions_tab(ui, &mut search, &mut open, &mut s));
                });
            });
        }
        assert!(!ctx.has_requested_repaint(), "an idle Permissions tab must request no repaint");
    }
}
