//! Ctrl+K command palette: fuzzy search over every `Action`, `Pane` ("Show X"), arg-free `ToolDef`,
//! Luau script and workspace, in one list. A `:` prefix switches to "tool mode": fuzzy search over
//! EVERY tool (not just arg-free ones), and picking a row builds a tiny arg form from the tool's
//! `args` docs instead of running it immediately.
//!
//! `rows`/`tool_rows` take plain data (a `&Hotkeys`, an `enabled` closure, a script-metadata slice) -
//! not `&App` - so they're unit-testable directly: there is no headless `App` harness in this crate
//! (`App::new` needs a real `eframe::CreationContext`/GL context; see `tools_registry_tests.rs`'s
//! App-construction note, and `enabled`/`enabled_for`'s own split for the same reason). `ui::app::
//! palette_ctl` is the thin, App-owning glue that calls these from a live `App` and dispatches the
//! `Command` they return - matching how `retime::show`/`capture_ui::show`/every other non-blocking
//! window in this codebase already takes plain fields out of `App` rather than `&App` itself.
//! ---- ws:command-palette ----

use crate::hotkeys::{Action, Hotkeys};
use crate::scripting::ScriptMeta;
use crate::ui::layout::{Pane, WORKSPACES};
use crate::ui::tools::{self, Glyph};
use eframe::egui;
use std::path::PathBuf;

/// One thing the palette can run.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Action(Action),
    Pane(Pane),
    Tool(&'static str),
    Script(PathBuf),
    Workspace(&'static str),
}

pub struct Row {
    pub cmd: Command,
    pub label: String,
    pub shortcut: String,
    pub glyph: Option<Glyph>,
    pub enabled: bool,
    pub reason: Option<&'static str>,
}

/// Allocation-free, case-insensitive subsequence scorer: every character of `query` must appear in
/// `text`, in order (not necessarily contiguous). Higher is a better match; matching consecutively
/// scores a bonus, and the FIRST query character landing right at a word boundary scores a bigger one
/// (deliberately not every character - see `fuzzy_score_prefers_word_starts`'s regression case: without
/// this restriction a long label with several word starts, e.g. "Ripple Delete In / Out" against query
/// "redo", could out-score the near-exact match "Redo" purely by accumulating more word-start bonuses
/// than a short label has room for). `None` = `query` is not a subsequence of `text` at all (an empty
/// query matches everything with score 0).
pub fn fuzzy_score(query: &str, text: &str) -> Option<u32> {
    if query.is_empty() {
        return Some(0);
    }
    let mut score: u32 = 0;
    let mut chars = text.chars().enumerate();
    let mut prev_idx: Option<usize> = None;
    let mut at_word_start = true;
    for (qi, qc) in query.chars().enumerate() {
        let qc = qc.to_ascii_lowercase();
        loop {
            let (i, tc) = chars.next()?; // ran out of text before this query char matched anything
            let word_start_here = at_word_start;
            at_word_start = !tc.is_alphanumeric();
            if tc.to_ascii_lowercase() == qc {
                score += 1;
                if qi == 0 && word_start_here {
                    score += 8;
                }
                if prev_idx == Some(i.wrapping_sub(1)) {
                    score += 4; // consecutive with the previous match (e.g. "spl" inside "Split")
                }
                prev_idx = Some(i);
                break;
            }
        }
    }
    Some(score)
}

fn push_scored(rows: &mut Vec<(u32, Row)>, query: &str, label: &str, row: Row) {
    if let Some(s) = fuzzy_score(query, label) {
        rows.push((s, row));
    }
}

fn finish(mut rows: Vec<(u32, Row)>) -> Vec<Row> {
    rows.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.label.cmp(&b.1.label)));
    rows.into_iter().map(|(_, r)| r).collect()
}

/// The main row set: `Action::ALL` + `Pane::ALL` ("Show X") + arg-free `ToolDef`s + scripts +
/// `layout::WORKSPACES`, fuzzy-filtered by `query` (empty query = everything, most-recently-used
/// actions first - `recent`, capped to `Settings.palette_recent`). `enabled` is a closure rather than
/// `&App` so this stays unit-testable (see the module doc comment); `App::enabled` is what
/// `palette_ctl` actually passes.
pub fn rows(
    hotkeys: &Hotkeys,
    enabled: impl Fn(Action) -> Result<(), &'static str>,
    scripts: &[ScriptMeta],
    recent: &[String],
    query: &str,
) -> Vec<Row> {
    let mut rows: Vec<(u32, Row)> = Vec::new();
    let boost = |id: &str| -> u32 {
        if !query.is_empty() {
            return 0;
        }
        match recent.iter().position(|r| r == id) {
            Some(i) => 1000u32.saturating_sub(i as u32),
            None => 0,
        }
    };
    for &a in Action::ALL {
        let label = a.label();
        let Some(mut s) = fuzzy_score(query, label) else { continue };
        s += boost(a.id());
        let (ok, reason) = match enabled(a) {
            Ok(()) => (true, None),
            Err(r) => (false, Some(r)),
        };
        rows.push((
            s,
            Row {
                cmd: Command::Action(a),
                label: label.to_string(),
                shortcut: hotkeys.text(a),
                glyph: crate::ui::tools::action_glyph(a),
                enabled: ok,
                reason,
            },
        ));
    }
    for &p in Pane::ALL {
        let label = format!("Show {}", p.title());
        push_scored(
            &mut rows,
            query,
            &label,
            Row {
                cmd: Command::Pane(p),
                label: label.clone(),
                shortcut: String::new(),
                glyph: Some(p.glyph()),
                enabled: true,
                reason: None,
            },
        );
    }
    for t in crate::mcp::tools::all().filter(|t| t.args.is_empty()) {
        push_scored(
            &mut rows,
            query,
            t.name,
            Row {
                cmd: Command::Tool(t.name),
                label: t.name.to_string(),
                shortcut: String::new(),
                glyph: Some(Glyph::Terminal),
                enabled: true,
                reason: Some(t.desc),
            },
        );
    }
    for sm in scripts {
        push_scored(
            &mut rows,
            query,
            &sm.name,
            Row {
                cmd: Command::Script(sm.path.clone()),
                label: sm.name.clone(),
                shortcut: sm.hotkey.clone().unwrap_or_default(),
                glyph: sm.icon.and_then(Glyph::from_name).or(Some(Glyph::Terminal)),
                enabled: true,
                reason: None,
            },
        );
    }
    for &w in WORKSPACES {
        push_scored(
            &mut rows,
            query,
            w,
            Row {
                cmd: Command::Workspace(w),
                label: w.to_string(),
                shortcut: String::new(),
                glyph: None,
                enabled: true,
                reason: None,
            },
        );
    }
    finish(rows)
}

/// `:`-mode rows: every registered tool (not just arg-free ones - see `rows` above), fuzzy-filtered by
/// `filter` (the text after the `:`). Picking one builds an arg form (`show`, below) instead of running
/// it immediately - needs no `App` at all, since `mcp::tools::all()` is a free fn over the static
/// `TOOL_TABLES` registry.
pub fn tool_rows(filter: &str) -> Vec<Row> {
    let mut rows: Vec<(u32, Row)> = Vec::new();
    for t in crate::mcp::tools::all() {
        push_scored(
            &mut rows,
            filter,
            t.name,
            Row {
                cmd: Command::Tool(t.name),
                label: t.name.to_string(),
                shortcut: String::new(),
                glyph: Some(Glyph::Terminal),
                enabled: true,
                reason: Some(t.desc),
            },
        );
    }
    finish(rows)
}

/// Ctrl+K palette state.
#[derive(Default)]
pub struct PaletteState {
    pub open: bool,
    pub query: String,
    pub sel: usize,
    /// `Some((tool name, [(arg name, typed-in value), ...]))` while the `:` mode's arg form is showing.
    pub arg_form: Option<(&'static str, Vec<(String, String)>)>,
}

/// Draws the palette window if `state.open`. `rows` is the already-filtered/sorted list for the
/// current query (built by the caller via `rows`/`tool_rows` above, since which one to call depends on
/// whether `state.query` starts with `:` - see `palette_ctl::windows`). Returns the chosen `Command` on
/// Enter/click and closes the window - EXCEPT picking a tool that takes args, which switches to an arg
/// form instead of returning immediately; submitting that form (Run / Enter) is what finally returns
/// `Some(Command::Tool(name))`, and leaves `state.arg_form` populated with the typed values so the
/// caller (`palette_ctl::windows`) can read them (`state.arg_form.take()`) right after this call
/// returns, before anything else touches `state` - every other way of closing (Escape, the window's own
/// X, Cancel) clears `arg_form` itself. Disables the text-cursor blink while open so an idle palette
/// costs no repaints (`assert_no_idle_repaint_palette_closed_and_open`).
pub fn show(ctx: &egui::Context, state: &mut PaletteState, rows: &[Row]) -> Option<Command> {
    if !state.open {
        return None;
    }
    ctx.style_mut(|s| s.visuals.text_cursor.blink = false);
    let mut result = None;
    let mut open = true;
    let mut close_palette = false; // fully close (Escape at the top level, or the window's own X)
    let mut back_to_search = false; // leave arg-form mode, palette stays open on the search box
    egui::Window::new("Command Palette")
        .id(egui::Id::new("command_palette_window"))
        .order(egui::Order::Foreground)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 80.0))
        .default_width(520.0)
        .open(&mut open)
        .show(ctx, |ui| {
            if let Some((tool_name, fields)) = state.arg_form.clone() {
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    back_to_search = true;
                    return;
                }
                ui.strong(format!(": {tool_name}"));
                let mut fields = fields;
                for (k, v) in fields.iter_mut() {
                    ui.horizontal(|ui| {
                        ui.label(k.as_str());
                        ui.text_edit_singleline(v);
                    });
                }
                let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                let mut run = false;
                ui.horizontal(|ui| {
                    if ui.button("Run").clicked() || enter {
                        run = true;
                    }
                    if ui.button("Cancel").clicked() {
                        back_to_search = true;
                    }
                });
                state.arg_form = Some((tool_name, fields)); // persist this frame's edits either way
                if run {
                    result = Some(Command::Tool(tool_name));
                    close_palette = true;
                }
                return;
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                close_palette = true;
                return;
            }
            let id = ui.id().with("query");
            let first_frame = ctx.read_response(id).is_none();
            let r = ui.add(
                egui::TextEdit::singleline(&mut state.query)
                    .id(id)
                    .hint_text("Type a command…  (':' + a tool name for one that needs args)")
                    .desired_width(f32::INFINITY),
            );
            if first_frame {
                r.request_focus();
            }
            if r.changed() {
                state.sel = 0;
            }
            state.sel = state.sel.min(rows.len().saturating_sub(1));
            ui.input(|i| {
                if i.key_pressed(egui::Key::ArrowDown) {
                    state.sel = (state.sel + 1).min(rows.len().saturating_sub(1));
                }
                if i.key_pressed(egui::Key::ArrowUp) {
                    state.sel = state.sel.saturating_sub(1);
                }
            });
            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
            egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                for (i, row) in rows.iter().enumerate() {
                    let text = if row.shortcut.is_empty() {
                        row.label.clone()
                    } else {
                        format!("{}   {}", row.label, row.shortcut)
                    };
                    let picked = ui
                        .horizontal(|ui| {
                            let (icon_rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
                            if let Some(g) = row.glyph {
                                let fg = if row.enabled {
                                    ui.visuals().text_color()
                                } else {
                                    ui.visuals().weak_text_color()
                                };
                                tools::draw_glyph(ui.painter(), icon_rect, g, fg);
                            }
                            let resp = ui.add_enabled(row.enabled, egui::Button::selectable(i == state.sel, text));
                            let resp = match row.reason {
                                Some(r) => resp.on_hover_text(r),
                                None => resp,
                            };
                            resp.clicked()
                        })
                        .inner
                        || (enter && i == state.sel);
                    if picked && row.enabled {
                        if let Command::Tool(name) = &row.cmd {
                            if let Some(def) = crate::mcp::tools::find(name) {
                                if !def.args.is_empty() {
                                    let fields: Vec<(String, String)> = def
                                        .args
                                        .iter()
                                        .map(|a| (a.split(':').next().unwrap_or("").to_string(), String::new()))
                                        .collect();
                                    state.arg_form = Some((name, fields));
                                    return;
                                }
                            }
                        }
                        result = Some(row.cmd.clone());
                        close_palette = true;
                    }
                }
            });
        });
    if back_to_search {
        state.arg_form = None;
    }
    if close_palette || !open {
        state.open = false;
        state.query.clear();
        state.sel = 0;
        if result.is_none() {
            // a real close/cancel - a successful arg-form submit (`result` = Some(Tool(..))) leaves
            // `arg_form` for the caller to read once, right after this call returns (see doc comment)
            state.arg_form = None;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scripting::ScriptMeta;
    use std::time::Duration;

    #[test]
    fn fuzzy_score_prefers_word_starts() {
        // word-start bonus: "sp" as the start of "Split at Playhead" beats "sp" buried mid-word
        let word_start = fuzzy_score("sp", "Split at Playhead").unwrap();
        let mid_word = fuzzy_score("sp", "Grasp").unwrap();
        assert!(word_start > mid_word, "{word_start} should beat {mid_word}");
        // consecutive-char bonus: "spl" scores higher than the same 3 letters scattered apart
        let consecutive = fuzzy_score("spl", "Split").unwrap();
        let scattered = fuzzy_score("spl", "Sample Playhead List").unwrap();
        assert!(consecutive > scattered, "{consecutive} should beat {scattered}");
        // not a subsequence at all
        assert_eq!(fuzzy_score("xyz", "Split"), None);
        // empty query matches everything with score 0
        assert_eq!(fuzzy_score("", "anything"), Some(0));
        // case-insensitive
        assert!(fuzzy_score("SPLIT", "split at playhead").is_some());
    }

    fn dummy_script(name: &str) -> ScriptMeta {
        ScriptMeta {
            path: PathBuf::from(format!("{name}.luau")),
            name: name.to_string(),
            desc: String::new(),
            icon: None,
            hotkey: None,
            on: Vec::new(),
            budget: Duration::from_millis(250),
        }
    }

    #[test]
    fn palette_lists_every_action_pane_and_arg_free_tool() {
        let hk = Hotkeys::defaults();
        let scripts = vec![dummy_script("my_script")];
        let all = rows(&hk, |_| Ok(()), &scripts, &[], "");
        for &a in Action::ALL {
            assert!(all.iter().any(|r| matches!(&r.cmd, Command::Action(x) if *x == a)), "missing Action {a:?}");
        }
        for &p in Pane::ALL {
            assert!(all.iter().any(|r| matches!(&r.cmd, Command::Pane(x) if *x == p)), "missing Pane {p:?}");
        }
        for t in crate::mcp::tools::all().filter(|t| t.args.is_empty()) {
            assert!(
                all.iter().any(|r| matches!(&r.cmd, Command::Tool(n) if *n == t.name)),
                "missing arg-free tool {}",
                t.name
            );
        }
        // an arg-taking tool is NOT in the main list (it only shows up via tool_rows' ':' mode)
        let has_args = crate::mcp::tools::all().find(|t| !t.args.is_empty()).expect("at least one tool takes args");
        assert!(!all.iter().any(|r| matches!(&r.cmd, Command::Tool(n) if *n == has_args.name)));
        assert!(all.iter().any(|r| matches!(&r.cmd, Command::Script(p) if p.ends_with("my_script.luau"))));
        assert!(all.iter().any(|r| matches!(&r.cmd, Command::Workspace(_))));
    }

    #[test]
    fn tool_rows_covers_every_tool_including_ones_with_args() {
        let all = tool_rows("");
        assert_eq!(all.len(), crate::mcp::tools::all().count());
    }

    #[test]
    fn disabled_action_carries_its_reason() {
        let hk = Hotkeys::defaults();
        let rs = rows(&hk, |a| if a == Action::Undo { Err("Nothing to undo") } else { Ok(()) }, &[], &[], "undo");
        let row = rs.iter().find(|r| matches!(&r.cmd, Command::Action(Action::Undo))).unwrap();
        assert!(!row.enabled);
        assert_eq!(row.reason, Some("Nothing to undo"));
    }

    #[test]
    fn recent_boosts_ordering_only_when_query_empty() {
        let hk = Hotkeys::defaults();
        let recent = vec![Action::Undo.id().to_string()];
        let rs = rows(&hk, |_| Ok(()), &[], &recent, "");
        assert!(matches!(&rs[0].cmd, Command::Action(Action::Undo)), "recent action should sort first");
        // once there's a query, recency stops mattering - plain fuzzy order applies
        let rs2 = rows(&hk, |_| Ok(()), &[], &recent, "redo");
        assert!(matches!(&rs2[0].cmd, Command::Action(Action::Redo)));
    }

    /// Enter on the selected row returns exactly that command and closes the window.
    #[test]
    fn palette_enter_pushes_one_command_and_closes() {
        let ctx = egui::Context::default();
        let hk = Hotkeys::defaults();
        let rs = rows(&hk, |_| Ok(()), &[], &[], "undo");
        let undo_i = rs.iter().position(|r| matches!(&r.cmd, Command::Action(Action::Undo))).expect("Undo row present");
        let mut state = PaletteState { open: true, query: "undo".into(), sel: undo_i, ..Default::default() };
        let mut result = None;
        // settle the first-frame focus request, then drive the accept path with a synthetic Enter
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            result = show(ctx, &mut state, &rs);
        });
        let mut raw = egui::RawInput::default();
        raw.events.push(egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        let _ = ctx.run(raw, |ctx| {
            result = show(ctx, &mut state, &rs);
        });
        assert_eq!(result, Some(Command::Action(Action::Undo)));
        assert!(!state.open, "the window must close on accept");
        assert!(state.query.is_empty(), "the query resets on close");
    }

    /// 30 idle frames (no input), palette closed then open: neither state asks for a repaint (the
    /// text-cursor blink is disabled while open - see `show`'s doc comment).
    #[test]
    fn assert_no_idle_repaint_palette_closed_and_open() {
        let hk = Hotkeys::defaults();
        for open in [false, true] {
            let ctx = egui::Context::default();
            let mut state = PaletteState { open, ..Default::default() };
            for _ in 0..30 {
                let rs = rows(&hk, |_| Ok(()), &[], &[], &state.query);
                let _ = ctx.run(egui::RawInput::default(), |ctx| {
                    let _ = show(ctx, &mut state, &rs);
                });
            }
            assert!(!ctx.has_requested_repaint(), "idle palette (open={open}) requested a repaint");
        }
    }
}
