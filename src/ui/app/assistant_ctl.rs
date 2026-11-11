//! ---- ws:assistant-pane ----
//! `Pane::Assistant`'s App side: sending a request (MCP server switched on, context attached, one undo
//! group per request), draining the agent's events from a FRAME_HOOKS tick (wakes through
//! `App::animate_until` only while a request is in flight), Stop, "Undo all", the `>` palette prefix,
//! the guard that holds an agent's MCP calls which leave the project until the user allows them, and the
//! `assistant.*` tools. The process side is `engine::agent`; the widget is `ui::assistant_ui`.

use super::tools_helpers::*;
use super::*;
use crate::engine::agent::{self, Event};
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::ui::assistant_ui::{self, Line, Out, View};
use std::sync::mpsc::TryRecvError;

/// While a request is in flight the tick drains the agent's events at this cadence.
const POLL: Duration = Duration::from_millis(150);

/// MCP tools an agent may only call after the user clicks Allow (they leave the project).
pub(super) fn leaves_project(tool: &str, args: &Value) -> bool {
    mcp::tools::leaves_project(tool, args)
}

/// What "Always allow this session" remembers: the tool, or `ui.action:<id>` for one Action.
fn allow_key(tool: &str, args: &Value) -> String {
    match args.get("id").and_then(Value::as_str) {
        Some(id) if tool == "ui.action" => format!("ui.action:{id}"),
        _ => tool.to_string(),
    }
}

/// Whether `guard` would hold a call to `tool` (also what `edit.batch` refuses to run inside a batch).
pub(super) fn needs_approval(app: &App, tool: &str, args: &Value) -> bool {
    let rt = &app.assistant_rt;
    !rt.bypass
        && app.settings.assistant_guard
        && leaves_project(tool, args)
        && !rt.always.iter().any(|t| *t == allow_key(tool, args))
}

#[derive(Default)]
pub(super) struct AssistantRt {
    run: Option<agent::Run>,
    /// A request waiting for the MCP server to come up: (what the user typed, the full prompt).
    queued: Option<(String, String)>,
    /// The request in flight: its undo label and the undo-stack length when it began.
    active: Option<(String, usize)>,
    /// The request's group was closed under it (a hand edit, Undo) and re-opened: it is more than one
    /// undo step, so "Undo all" only reaches its last part.
    split: bool,
    /// MCP calls held until the user answers the approval window, with when each was held (the
    /// client gives up after `mcp::HELD_WAIT`; an Allow after that must not run it).
    held: Vec<(mcp::ToolCall, Instant)>,
    /// Tools the user allowed for the rest of the session.
    always: Vec<String>,
    /// Set while a released call runs, so the guard lets it through.
    bypass: bool,
}

fn busy(app: &App) -> bool {
    app.assistant_rt.run.is_some() || app.assistant_rt.queued.is_some()
}

// ---------------------------------------------------------------------------------------------------
// context chips

/// One chip per piece of `editor.context` worth showing: (key, label). Keys: selection, range,
/// playhead, comment.
pub(super) fn chips_of(c: &Value) -> Vec<(&'static str, String)> {
    let n = |k: &str| c[k].as_array().map_or(0, Vec::len);
    let mut out = Vec::new();
    let words = c["selected_words"]["text"].as_str().filter(|t| !t.is_empty());
    let sel = match (words, n("selected_clips"), n("selected_cues")) {
        (Some(w), _, _) => Some(format!("Words: \u{201c}{}\u{201d}", short(w, 30))),
        (_, 1, _) => Some(format!("Clip: {}", c["selected_clips"][0]["name"].as_str().unwrap_or("1 clip"))),
        (_, k, _) if k > 1 => Some(format!("{k} clips")),
        (_, _, k) if k > 0 => Some(format!("{k} subtitle{}", if k == 1 { "" } else { "s" })),
        _ => None,
    };
    if let Some(s) = sel {
        out.push(("selection", s));
    }
    if let (Some(a), Some(b)) = (c["in"].as_f64(), c["out"].as_f64()) {
        out.push(("range", format!("In/Out {a:.1}s–{b:.1}s")));
    }
    if let Some(t) = c["playhead"].as_f64() {
        out.push(("playhead", format!("Playhead {t:.1}s")));
    }
    let note = c["selected_markers"].as_array().into_iter().flatten().find_map(|m| m["note"].as_str().filter(|s| !s.trim().is_empty()));
    if let Some(note) = note {
        out.push(("comment", format!("Comment: \u{201c}{}\u{201d}", short(note, 30))));
    }
    out
}

/// `editor.context` minus what the user clicked off.
pub(super) fn filter_context(mut c: Value, off: &[&str]) -> Value {
    if let Some(o) = c.as_object_mut() {
        for key in off {
            let fields: &[&str] = match *key {
                "selection" => &["selected_clips", "selected_transitions", "selected_cues", "selected_words", "edit_point"],
                "range" => &["in", "out"],
                "playhead" => &["playhead"],
                "comment" => &["selected_markers"],
                _ => &[],
            };
            for f in fields {
                o.remove(*f);
            }
        }
    }
    c
}

fn short(s: &str, n: usize) -> String {
    let s = s.replace(['\n', '\r'], " ");
    match s.char_indices().nth(n) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s,
    }
}

// ---------------------------------------------------------------------------------------------------
// sending

/// Send `text` as a request (the palette's `>` prefix, `assistant.send`).
pub(super) fn ask(app: &mut App, text: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("empty request".into());
    }
    if busy(app) {
        return Err("the assistant is still working on the last request".into());
    }
    app.assistant.prompt = text.to_string();
    app.surface(Pane::Assistant);
    send(app);
    Ok(())
}

/// Send the prompt box: switch the MCP server on, attach the context, queue the run (`start_queued`
/// starts it once the server is bound).
fn send(app: &mut App) {
    if busy(app) {
        app.toast("The assistant is still working on the last request");
        return;
    }
    let prompt = std::mem::take(&mut app.assistant.prompt).trim().to_string();
    if prompt.is_empty() {
        return;
    }
    app.assistant.log.push(Line::User(prompt.clone()));
    if !app.settings.mcp_enabled {
        app.settings.mcp_enabled = true; // sync_mcp starts it this frame
        app.settings.save();
    }
    let off = std::mem::take(&mut app.assistant.chips_off);
    if app.settings.assistant_backend == "external" {
        // assistant.wait attaches the context itself
        app.assistant.pending.push_back(prompt);
        app.assistant.log.push(Line::Info("Waiting for a connected agent (assistant.wait)…".into()));
        return;
    }
    let ctx = filter_context(tools_context::context_json(app), &off);
    let full = format!(
        "You are editing the video project open in Simple Editor, live, through its MCP tools. This whole \
         request is already one undo step: do not call edit.begin or edit.end. The editor's context when the \
         user sent this (editor.context JSON):\n{ctx}\n\nRequest: {prompt}"
    );
    app.assistant_rt.queued = Some((prompt, full));
    start_queued(app);
}

fn start_queued(app: &mut App) {
    let Some(url) = app.mcp.as_ref().map(|(s, _)| s.url()) else {
        if !app.settings.mcp_enabled {
            // sync_mcp tried every port and switched it back off
            app.assistant_rt.queued = None;
            app.assistant.log.push(Line::Error("The MCP server couldn't start (see Settings ▸ General ▸ MCP).".into()));
            app.assistant.log.push(Line::Done { undo: None });
        }
        return;
    };
    let Some((prompt, full)) = app.assistant_rt.queued.take() else { return };
    let s = &app.settings;
    let claude = s.assistant_backend != "custom";
    let key_env = if claude { "ANTHROPIC_API_KEY".to_string() } else { s.assistant_key_env.clone() };
    let session_key = session_key(app);
    let spec = agent::Spec {
        backend: if claude { agent::Backend::Claude } else { agent::Backend::Custom },
        prompt: full,
        mcp_url: url,
        model: s.assistant_model.clone(),
        resume: session_key.and_then(|k| s.assistant_sessions.get(&k).cloned()),
        command: s.assistant_command.clone(),
        key: agent::key_load().map(|k| (key_env, k)),
    };
    let label: String = prompt.replace(['\n', '\r'], " ").chars().take(60).collect();
    app.begin_group(&label, if claude { "Claude" } else { "Assistant" });
    let group_label = app.mcp_ctx.group.as_ref().map(|g| g.label.clone()).unwrap_or_default();
    app.assistant_rt.active = Some((group_label, app.undo.len()));
    app.assistant_rt.split = false;
    app.assistant_rt.run = Some(agent::start(spec));
}

/// Conversations are resumed per saved project file; an unsaved project (incl. after File ▸ New) has
/// none, so it always starts a fresh session instead of sharing one with every other unsaved project.
fn session_key(app: &App) -> Option<String> {
    app.project_path.as_ref().map(|p| p.to_string_lossy().into_owned())
}

/// The request ended (finished, failed or stopped): close its undo group, log "Finished" with its
/// "Undo all" when its entry is on the stack.
fn finish(app: &mut App) {
    app.assistant_rt.run = None; // drops the Run: kills the child if it's still alive
    app.end_group();
    let undo = app.assistant_rt.active.take().and_then(|(label, start)| {
        let top = app.undo.last().is_some_and(|e| e.label == label);
        (top && app.undo.len() > start).then(|| (app.undo.len(), label))
    });
    if std::mem::take(&mut app.assistant_rt.split) && undo.is_some() {
        app.assistant.log.push(Line::Info(
            "You edited while this ran, so it is split around your edit: Undo all undoes only its last part \
             (the earlier part is further down in History)."
                .into(),
        ));
    }
    app.assistant.log.push(Line::Done { undo });
}

fn stop(app: &mut App) {
    if let Some(run) = &app.assistant_rt.run {
        run.stop();
    }
    if app.assistant_rt.queued.take().is_some() || app.assistant_rt.run.is_some() {
        // the stopped agent's held calls must not run on a later Allow
        deny_held(app, "the user stopped the request");
        app.assistant.log.push(Line::Info("Stopped.".into()));
        finish(app);
    }
}

// ---------------------------------------------------------------------------------------------------
// MCP guard

/// `handle_tool`'s first step: None = the call is held for the user's approval.
pub(super) fn guard(app: &mut App, call: mcp::ToolCall) -> Option<mcp::ToolCall> {
    if !needs_approval(app, &call.name, &call.args) {
        return Some(call);
    }
    app.assistant_rt.held.push((call, Instant::now()));
    None
}

/// Answer every held call with an error (the user stopped the request, the app closed).
pub(super) fn deny_held(app: &mut App, why: &str) {
    for (call, _) in app.assistant_rt.held.drain(..) {
        let _ = call.reply.send(Err(format!("'{}': {why}", call.name)));
    }
}

/// WINDOW_DRAWERS entry: one row per held call - Allow / Always allow this session / Deny. Non-modal;
/// the agent's call simply waits (or times out on its side).
pub(super) fn approvals(app: &mut App, ctx: &egui::Context) {
    if app.assistant_rt.held.is_empty() {
        return;
    }
    let mut pick: Option<(usize, u8)> = None;
    // ponytail: the MCP server doesn't track which client sent a call; name it by what is running
    let who = if app.assistant_rt.run.is_some() {
        "The Assistant (or another connected MCP client) asks"
    } else {
        "A connected MCP client asks"
    };
    egui::Window::new("An agent wants to leave the project")
        .id(egui::Id::new("assistant_approvals"))
        .collapsible(false)
        .resizable(false)
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::RIGHT_TOP, [-12.0, 60.0])
        .show(ctx, |ui| {
            ui.label(who);
            for (i, (c, _)) in app.assistant_rt.held.iter().enumerate() {
                let what = match c.args.get("id").and_then(Value::as_str).and_then(Action::from_id) {
                    Some(a) if c.name == "ui.action" => format!("ui.action: {}", a.label()),
                    _ => c.name.clone(),
                };
                ui.label(egui::RichText::new(what).strong());
                ui.weak(short(&c.args.to_string(), 120));
                ui.horizontal(|ui| {
                    if ui.button("Allow").clicked() {
                        pick = Some((i, 0));
                    }
                    if ui.button("Always allow this session").clicked() {
                        pick = Some((i, 1));
                    }
                    if ui.button("Deny").clicked() {
                        pick = Some((i, 2));
                    }
                });
                ui.separator();
            }
            ui.weak("Turn this check off in Settings ▸ Assistant.");
        });
    let Some((i, choice)) = pick else { return };
    let (call, held_at) = app.assistant_rt.held.remove(i);
    if choice == 2 {
        let _ = call.reply.send(Err(format!("the user denied '{}'", call.name)));
        return;
    }
    if held_at.elapsed() >= mcp::HELD_WAIT {
        // the agent was already told it timed out: running it now would surprise everyone
        app.toast(format!("'{}' waited too long - the agent gave up on it; nothing was run", call.name));
        return;
    }
    if choice == 1 {
        app.assistant_rt.always.push(allow_key(&call.name, &call.args));
    }
    app.assistant_rt.bypass = true;
    app.handle_tool(call);
    app.assistant_rt.bypass = false;
    ctx.request_repaint();
}

// ---------------------------------------------------------------------------------------------------
// registries

/// PANE_DRAWERS entry.
pub(super) fn draw(app: &mut App, ui: &mut egui::Ui, pane: Pane) -> bool {
    if pane != Pane::Assistant {
        return false;
    }
    let chips = chips_of(&tools_context::context_json(app));
    let running = busy(app);
    let out = {
        let v = View {
            running,
            chips: &chips,
            quick: &app.settings.assistant_quick,
            undo_top: (app.undo.len(), app.undo.last().map(|e| e.label.as_str())),
            backend: &app.settings.assistant_backend,
        };
        assistant_ui::draw(ui, &mut app.assistant, &v)
    };
    match out {
        Out::Send => send(app),
        Out::Stop => stop(app),
        Out::Undo => app.pending_actions.push(Action::Undo),
        Out::None => {}
    }
    true
}

/// FRAME_HOOKS entry: a dictated / quick prompt waiting to go, the server coming up for a queued
/// request, and the running agent's events. Schedules nothing when idle.
pub(super) fn tick(app: &mut App, ctx: &egui::Context) {
    if std::mem::take(&mut app.assistant.send_now) {
        send(app);
    }
    if app.assistant_rt.queued.is_some() {
        start_queued(app);
        if app.assistant_rt.queued.is_some() {
            app.animate_until(ctx, Instant::now() + POLL);
        }
    }
    if app.assistant_rt.run.is_none() {
        return;
    }
    // the agent may think for longer than the group's 30 s idle timeout between tool calls: while it
    // runs, its request stays one undo group
    if let Some(g) = app.mcp_ctx.group.as_mut() {
        g.last_call = Instant::now();
    } else if let Some((label, _)) = &app.assistant_rt.active {
        // a hand edit / Undo closed it (tools_context::tick, close_group_for_user_edit): the rest of
        // the request is a fresh group under the same label, above the user's own entry
        app.mcp_ctx.group = Some(super::tools_context::UndoGroup {
            label: label.clone(),
            before: app.project.to_json(),
            undo_len: app.undo.len(),
            top_at: app.undo.last().map(|e| e.at),
            last_call: Instant::now(),
        });
        app.assistant_rt.split = true;
    }
    let Some(run) = app.assistant_rt.run.as_ref() else { return };
    let mut events = Vec::new();
    let mut ended = false;
    loop {
        match run.rx.try_recv() {
            Ok(e) => events.push(e),
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                ended = true;
                break;
            }
        }
    }
    let had = !events.is_empty();
    for e in events {
        match e {
            Event::Session(s) => remember_session(app, s),
            Event::Text(t) | Event::Line(t) => app.assistant.log.push(Line::Text(t)),
            Event::Tool(t) => app.assistant.log.push(Line::Tool(t)),
            Event::Info(t) => app.assistant.log.push(Line::Info(t)),
            Event::Error(t) => app.assistant.log.push(Line::Error(t)),
            Event::Done { session, .. } => {
                if let Some(s) = session {
                    remember_session(app, s);
                }
                ended = true;
            }
        }
    }
    if ended {
        finish(app);
        ctx.request_repaint();
    } else {
        if had {
            ctx.request_repaint();
        }
        app.animate_until(ctx, Instant::now() + POLL);
    }
}

fn remember_session(app: &mut App, s: String) {
    let Some(key) = session_key(app) else { return };
    if app.settings.assistant_sessions.get(&key) != Some(&s) {
        app.settings.assistant_sessions.insert(key, s);
        app.settings.save();
    }
}

/// ACT_HANDLERS entry.
pub(super) fn act(app: &mut App, a: Action) -> bool {
    match a {
        Action::ToggleAssistant => {
            app.toggle_pane(Pane::Assistant);
            true
        }
        _ => false,
    }
}

/// The palette's `>` row: "Ask the assistant: …".
pub(super) fn palette_row(text: &str) -> crate::ui::palette::Row {
    let text = text.trim();
    crate::ui::palette::Row {
        cmd: crate::ui::palette::Command::Action(Action::ToggleAssistant),
        label: if text.is_empty() { "Ask the assistant: type a request".into() } else { format!("Ask the assistant: {text}") },
        shortcut: String::new(),
        glyph: Some(crate::ui::tools::Glyph::Assistant),
        enabled: !text.is_empty(),
        reason: None,
    }
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "assistant.send",
        desc: "Send a request to the Assistant pane's agent, as if the user typed it (one request at a time).",
        args: &["prompt:string:true:the request"],
        kind: ToolKind::Ui,
        run: |app, args| {
            ask(app, req(arg_str(args, "prompt"), "prompt")?)?;
            Ok(ToolOutcome::Done(json!({"ok": true, "backend": app.settings.assistant_backend})))
        },
    },
    ToolDef {
        name: "assistant.status",
        desc: "The Assistant pane: whether a request is running, the backend, requests waiting for \
               assistant.wait, and the last 20 log lines.",
        args: &[],
        kind: ToolKind::Read,
        run: |app, _| {
            let log: Vec<String> = app.assistant.log.iter().rev().take(20).rev().map(|l| format!("{l:?}")).collect();
            Ok(ToolOutcome::Done(json!({
                "running": busy(app),
                "backend": app.settings.assistant_backend,
                "pending": app.assistant.pending.len(),
                "held_calls": app.assistant_rt.held.iter().map(|(c, _)| c.name.clone()).collect::<Vec<_>>(),
                "log": log,
            })))
        },
    },
    ToolDef {
        name: "assistant.stop",
        desc: "Stop the Assistant pane's running request (its edits so far stay, as one undo step).",
        args: &[],
        kind: ToolKind::Ui,
        run: |app, _| {
            let was = busy(app);
            stop(app);
            Ok(ToolOutcome::Done(json!({"ok": true, "stopped": was})))
        },
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chips_and_filtering() {
        let c = json!({
            "playhead": 12.0, "in": 1.0, "out": 4.0,
            "selected_clips": [{"id": 3, "name": "intro.mp4"}],
            "selected_transitions": [], "selected_cues": [], "edit_point": null, "selected_words": null,
            "selected_markers": [{"id": 9, "note": "too loud here"}],
        });
        let chips = chips_of(&c);
        let keys: Vec<&str> = chips.iter().map(|(k, _)| *k).collect();
        assert_eq!(keys, ["selection", "range", "playhead", "comment"]);
        assert_eq!(chips[0].1, "Clip: intro.mp4");
        let f = filter_context(c.clone(), &["selection", "comment"]);
        assert!(f.get("selected_clips").is_none() && f.get("selected_markers").is_none());
        assert_eq!(f["in"], 1.0, "chips left on stay");
        assert_eq!(filter_context(c.clone(), &[]), c);
        // nothing selected, no range, no comment: just the playhead
        let bare = json!({"playhead": 0.0, "in": null, "out": null, "selected_clips": [], "selected_markers": []});
        assert_eq!(chips_of(&bare).len(), 1);
    }

    #[test]
    fn guarded_tools() {
        let none = json!({});
        for t in
            ["export.video", "export.edl", "export.xml", "project.open", "project.new", "settings.set", "scripts.run", "media.convert"]
        {
            assert!(leaves_project(t, &none), "{t}");
        }
        for t in ["timeline.cut", "editor.context", "markers.add", "project.summary", "ui.action"] {
            assert!(!leaves_project(t, &none), "{t}");
        }
        // ui.action: only the Actions that leave the project, and every listed id is a real Action
        for id in ["new_project", "open_project", "export", "quick_export", "export_xml", "export_edl", "save", "settings"] {
            assert!(Action::from_id(id).is_some(), "{id}");
            assert!(leaves_project("ui.action", &json!({"id": id})), "{id}");
        }
        assert!(!leaves_project("ui.action", &json!({"id": "undo"})));
        assert_eq!(allow_key("ui.action", &json!({"id": "export"})), "ui.action:export");
        assert_eq!(allow_key("export.xml", &json!({"id": "x"})), "export.xml");
    }

    #[test]
    fn palette_row_needs_text() {
        assert!(!palette_row("  ").enabled);
        let r = palette_row(" cut the ums ");
        assert!(r.enabled && r.label == "Ask the assistant: cut the ums");
    }
}
